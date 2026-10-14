//! M1.6 — Camera-jump churn stress test for the `cesium-pipeline` CORE crate.
//!
//! Scope: **pure `std` core only** (no Bevy, no GPU, no real network). A
//! deterministic pseudo-random "camera" teleports 200 times; each jump rewrites
//! the wanted set (commit / cancel / refresh), drives `GenericPipeline` submit +
//! poll, and feeds a `GpuCache` that is evicted under churn. The test asserts
//! **0 panics** and that the core invariants hold on *every* jump.
//!
//! Two entry points share one engine (`run_camera_churn`):
//! - `camera_jump_200_scaled` — default suite: 200 jumps, ~1 ms pacing (fast).
//! - `camera_jump_200_full_60s` — `#[ignore]` long soak: 200 jumps paced to
//!   ~60 s wall-clock so real worker completion / retry / abort flows interleave
//!   with the churn. Run explicitly via `cargo test -p cesium-pipeline --
//!   --ignored --nocapture --test-threads=1`.
//!
//! Invariants asserted per jump:
//! - live (wanted ∩ cached) keys are NEVER evicted          (花屏防护核心)
//! - base-layer (zoom <= 3) keys are permanently exempt
//! - `gpu_cache.len() <= cache_max + live`
//! - `dl_in_flight <= total_submits` (guards against u32 underflow-wrap)
//!
//! After a full settle-drain:
//! - `dl_in_flight == 0` and `load_n == 0` (every submit's result was polled;
//!   in-flight accounting is exactly balanced — no leak, no double-count).
//!
//! DEFERRED (registered, NOT implemented here):
//! - **Screen-tearing detection (adjacent-frame pixel diff > 60%)**. Detecting a
//!   torn/blank frame requires a real GPU renderer + framebuffer readback and a
//!   per-frame image to diff. That is the domain of M1.5 (PSNR >= 45 dB
//!   pixel-neutrality vs the golden path) and M11 (end-to-end visual harness).
//!   Pulling `bevy`/wgpu into the pure-`std` core test suite is explicitly out
//!   of scope for M1.6, so this stress test validates the *pipeline-level*
//!   anti-tearing guarantee (live tiles are never evicted mid-flight) instead of
//!   the pixel-level symptom. Cross-reference: `pipeline_invariants.rs` header.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cesium_pipeline::base_layer::BaseLayerGuard;
use cesium_pipeline::gpu_cache::GpuCache;
use cesium_pipeline::net::{FetchResult, NetworkBackend};
use cesium_pipeline::pool::{Decoder, PoolConfig};
use cesium_pipeline::runtime::GenericPipeline;

use cesium_ports_driven::{PollOutcome, TilePipeline};

/// Canonical tile key `(x, y, zoom)`.
type TileKey = (u32, u32, u32);

/// Zoom extractor for `TileKey`.
fn zoom_of(k: &TileKey) -> u32 {
    k.2
}

/// Deterministic URL builder (same shape as the runtime test harness).
fn tile_url(k: &TileKey) -> String {
    format!("http://tiles/{}/{}/{}", k.2, k.0, k.1)
}

// ── Deterministic PRNG (avoids adding a `rand` dependency) ───────────────────

/// Knuth MMIX LCG — reproducible churn without an external RNG crate.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
}

/// FNV-1a over a string → stable per-URL bucket (reproducible outcomes).
fn fnv1a(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// ── Deterministic churn backend (exercises all four outcomes) ────────────────

/// Backend whose per-URL outcome is a deterministic function of the URL hash, so
/// the whole soak is reproducible from the seed alone. Distribution (~):
/// 70% Success, 10% empty→Placeholder, 20% Permanent→Placeholder, 10%
/// Transient→Failed (after retries). Aborted arises from wanted-set churn.
struct ChurnBackend;

impl NetworkBackend for ChurnBackend {
    fn fetch(&self, url: &str) -> FetchResult {
        match fnv1a(url) % 10 {
            0 => FetchResult::Ok(Vec::new()),                // decode → None → Placeholder
            1 => FetchResult::Transient("throttle".into()),  // retries exhausted → Failed
            2 => FetchResult::Permanent("404".into()),       // → Placeholder
            n => FetchResult::Ok(vec![(n as u8) | 0x80, 7, 7, 7]), // → Success
        }
    }
    fn name(&self) -> &str {
        "churn-mock"
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(1)
    }
}

// ── Outcome tally (reported for the three-state-dispatch evidence) ───────────

#[derive(Debug, Default, Clone, Copy)]
struct Tally {
    ready: u64,
    aborted: u64,
    failed: u64,
    placeholder: u64,
    cancels: u64,
    evicted: u64,
    deferred: u64,
    jumps: u64,
}

/// Shared soak engine. `per_jump_delay` paces the loop (0 for the scaled run,
/// ~300 ms for the 60 s run); `cache_max` is the GPU-cache cap that eviction
/// fights against (small, so eviction actually fires during the soak).
fn run_camera_churn(
    jumps: usize,
    per_jump_delay: Duration,
    seed: u64,
    cache_max: usize,
    label: &str,
) -> Tally {
    let decode: Decoder<Vec<u8>> = Arc::new(|d: &[u8]| {
        if d.is_empty() {
            None
        } else {
            Some(d.to_vec())
        }
    });
    let cfg = PoolConfig {
        threads: 8,
        max_attempts: 2,
        backoff_base: Duration::from_millis(2),
    };
    let pipe: GenericPipeline<TileKey, Vec<u8>> =
        GenericPipeline::with_config(Arc::new(ChurnBackend), Arc::new(tile_url), decode, cfg);

    let mut cache: GpuCache<TileKey, u64> = GpuCache::new(cache_max, BaseLayerGuard::new(), zoom_of);
    let mut rng = Lcg(seed);
    let mut inserted: HashSet<TileKey> = HashSet::new();
    let mut handle_id: u64 = 0;
    let mut prev_wanted: Vec<TileKey> = Vec::new();
    let mut tally = Tally::default();
    let mut total_submits: u64 = 0;

    let started = Instant::now();

    for jump in 0..jumps {
        pipe.begin_frame();

        // ── Camera teleport → build a fresh wanted window ──
        // Zoom is kept >= 4 so the ONLY base-layer (zoom <= 3) keys in play are
        // the fixed resident set below — faithful to `dynamic_globe.rs`, where
        // the base layer is a small permanently-resident, always-live set. This
        // keeps the invariant `len <= cache_max + live` exact (every exempt
        // base key is also live).
        let z = 4 + rng.below(10) as u32; // zoom 4..=13
        let dim = 1u64 << z.min(20);
        let cx = rng.below(dim);
        let cy = rng.below(dim);

        let mut wanted: Vec<TileKey> = Vec::with_capacity(29);
        for b in 0..4u32 {
            wanted.push((b, 0, 3)); // resident base layer (always wanted → always live)
        }
        let span = 5u32;
        for dx in 0..span {
            for dy in 0..span {
                let x = ((cx + u64::from(dx)) % dim) as u32;
                let y = ((cy + u64::from(dy)) % dim) as u32;
                wanted.push((x, y, z));
            }
        }

        // ── Churn: cancel some tiles that dropped out of the view ──
        for k in &prev_wanted {
            if !wanted.contains(k) && rng.below(100) < 40 {
                pipe.cancel(k);
                tally.cancels += 1;
            }
        }

        // ── Commit the new wanted set and submit it ──
        pipe.refresh_wanted(&wanted);
        for k in &wanted {
            let prio = rng.below(1000) as f64;
            pipe.submit(*k, prio);
            total_submits += 1;
        }

        // ── Non-blocking poll → feed the GPU cache ──
        let outcomes = pipe.poll_ready(16);
        for o in &outcomes {
            match o {
                PollOutcome::Ready(k, _) => {
                    tally.ready += 1;
                    inserted.insert(*k);
                    cache.insert(*k, handle_id);
                    handle_id += 1;
                }
                PollOutcome::Aborted(_) => tally.aborted += 1,
                PollOutcome::Failed(_) => tally.failed += 1,
                PollOutcome::Placeholder(_) => tally.placeholder += 1,
            }
        }

        // ── Live set = (currently cached) ∩ (currently wanted) ──
        // Base-layer tiles are always in `wanted`, so cached base tiles are live.
        let wanted_set: HashSet<TileKey> = wanted.iter().copied().collect();
        let mut live_now: HashSet<TileKey> = HashSet::new();
        for k in &inserted {
            let is_live = cache.contains_key(k) && wanted_set.contains(k);
            cache.set_live(*k, is_live);
            if is_live {
                live_now.insert(*k);
            }
        }

        // ── Evict under churn ──
        let res = cache.evict(|k| live_now.contains(k));
        tally.evicted += u64::from(res.evicted);
        tally.deferred += u64::from(res.deferred);

        // ── Invariants (asserted on EVERY jump) ──
        for k in &live_now {
            assert!(cache.contains_key(k), "[{label}] jump {jump}: live {k:?} evicted");
        }
        for k in &inserted {
            if k.2 <= 3 {
                assert!(
                    cache.contains_key(k),
                    "[{label}] jump {jump}: base-layer {k:?} evicted"
                );
            }
        }
        assert!(
            cache.len() <= cache_max + live_now.len(),
            "[{label}] jump {jump}: len {} > max {} + live {}",
            cache.len(),
            cache_max,
            live_now.len()
        );
        let s = pipe.stats();
        assert!(
            u64::from(s.dl_in_flight) <= total_submits,
            "[{label}] jump {jump}: dl_in_flight {} > total_submits {} (u32 underflow-wrap?)",
            s.dl_in_flight,
            total_submits
        );

        prev_wanted = wanted;
        tally.jumps += 1;

        if per_jump_delay > Duration::ZERO {
            std::thread::sleep(per_jump_delay);
        }
    }

    // ── Settle: drain every outstanding result, then check exact balance ──
    let mut empty_streak = 0u32;
    let drain_start = Instant::now();
    while empty_streak < 10 && drain_start.elapsed() < Duration::from_secs(15) {
        let r = pipe.poll_ready(256);
        for o in &r {
            match o {
                PollOutcome::Ready(_, _) => tally.ready += 1,
                PollOutcome::Aborted(_) => tally.aborted += 1,
                PollOutcome::Failed(_) => tally.failed += 1,
                PollOutcome::Placeholder(_) => tally.placeholder += 1,
            }
        }
        if r.is_empty() {
            empty_streak += 1;
            std::thread::sleep(Duration::from_millis(30));
        } else {
            empty_streak = 0;
        }
    }

    let s = pipe.stats();
    assert_eq!(
        s.dl_in_flight, 0,
        "[{label}] in-flight gauge must settle to 0 after full drain"
    );
    assert_eq!(
        s.load_n, 0,
        "[{label}] dedup set must be empty after full drain"
    );

    let elapsed = started.elapsed();
    eprintln!(
        "[{label}] jumps={} elapsed={:.2?} tally={:?}",
        tally.jumps, elapsed, tally
    );

    pipe.shutdown();
    tally
}

/// Default-suite scaled soak: 200 camera jumps, minimal pacing (~fast).
/// Proves 0 panics + all invariants + balanced in-flight accounting under churn.
#[test]
fn camera_jump_200_scaled() {
    let tally = run_camera_churn(
        200,
        Duration::from_millis(1),
        0x5eed_2024_0617,
        64,
        "scaled-200",
    );

    // The three-state dispatch must actually be exercised by the churn.
    assert!(tally.ready > 0, "expected some Ready outcomes");
    assert!(tally.placeholder > 0, "expected some Placeholder outcomes");
    assert!(
        tally.aborted + tally.failed > 0,
        "expected the staleness paths (Aborted/Failed) to be exercised"
    );
    assert_eq!(tally.jumps, 200);
}

/// Full 60 s × 200-jump soak (ignored by default to keep `cargo test` fast).
///
/// Run once explicitly to prove 0 panics over a realistic frame cadence:
/// ```text
/// cargo test -p cesium-pipeline --release -- --ignored --nocapture --test-threads=1
/// ```
#[test]
#[ignore = "long soak: ~60s wall-clock (200 jumps × ~300ms). Run with --ignored."]
fn camera_jump_200_full_60s() {
    let tally = run_camera_churn(
        200,
        Duration::from_millis(300),
        0x5eed_2024_0617,
        64,
        "full-60s",
    );

    assert!(tally.ready > 0, "expected some Ready outcomes");
    assert!(tally.placeholder > 0, "expected some Placeholder outcomes");
    assert!(
        tally.aborted + tally.failed > 0,
        "expected the staleness paths (Aborted/Failed) to be exercised"
    );
    assert_eq!(tally.jumps, 200);
}
