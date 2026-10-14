//! M1.6 — Property-based contract tests for the `cesium-pipeline` CORE crate.
//!
//! Scope: **pure `std` core only** (no Bevy, no GPU, no real network). Every
//! property below is expressed against the crate's public API and the
//! `cesium-ports-driven` contracts it implements. The invariants mirror the
//! golden-path reference `dynamic_globe.rs` and the M1 verification gate.
//!
//! Invariants covered (proptest):
//! 1. in-flight uniqueness          — `Dedup` + `GenericPipeline::submit`
//! 2. FIFO eviction order           — `GpuCache::evict`
//! 3. live entities never evicted   — `GpuCache::evict` (花屏防护核心)
//! 4. `len() <= MAX_GPU_CACHE + live`— `GpuCache`
//! 5. BASE_LAYER permanent exemption— `GpuCache` + `BaseLayerGuard`
//! 6. staleness three-state dispatch— `DefaultStaleness::classify`
//! 7. retry cooldown dual-timescale — `DefaultRetry`
//! 8. hidden-LRU oldest-first        — `HiddenLru::pop_lru`
//!
//! DEFERRED (registered, NOT implemented here):
//! - Screen-tearing detection (adjacent-frame pixel diff > 60%) requires a real
//!   GPU renderer + framebuffer readback. That belongs to M1.5 (PSNR >= 45 dB
//!   pixel-neutrality) and M11 (e2e visual harness). Implementing it here would
//!   force a `bevy` dependency into the pure-`std` core test suite, which is
//!   explicitly out of scope for M1.6. See `camera_jump_stress.rs` for the
//!   matching deferral note on the runtime side.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use proptest::prelude::*;

use cesium_pipeline::base_layer::BaseLayerGuard;
use cesium_pipeline::budget::DefaultBudget;
use cesium_pipeline::dedup::Dedup;
use cesium_pipeline::gpu_cache::GpuCache;
use cesium_pipeline::hidden_lru::HiddenLru;
use cesium_pipeline::net::{FetchResult, NetworkBackend};
use cesium_pipeline::pool::{Decoder, PoolConfig};
use cesium_pipeline::retry::DefaultRetry;
use cesium_pipeline::runtime::GenericPipeline;
use cesium_pipeline::staleness::DefaultStaleness;

use cesium_ports_driven::{
    BudgetPolicy, RetryPolicy, StalenessPolicy, StalenessVerdict, TilePipeline,
};

/// Canonical tile key `(x, y, zoom)` used across the pipeline.
type TileKey = (u32, u32, u32);

/// Zoom extractor for `TileKey` (`.2` is the zoom component).
fn zoom_of(k: &TileKey) -> u32 {
    k.2
}

// ─────────────────────────────────────────────────────────────────────────────
// Pure-structure invariants (no threads): 256 cases each.
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    // Integration tests live in `tests/` (no lib.rs sibling), so disable
    // proptest's source-parallel failure persistence to keep output clean.
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::with_cases(256)
    })]

    /// Invariant 1 (data-structure level): the in-flight dedup set never
    /// double-counts a key. `insert` returns `true` iff the key was absent,
    /// mirroring `dynamic_globe.rs:406/417` (`in_flight`/`queued` guards).
    #[test]
    fn dedup_inflight_uniqueness(
        ops in proptest::collection::vec((0u32..32, any::<bool>()), 0..256),
    ) {
        let d = Dedup::<u32>::new();
        let mut model: HashSet<u32> = HashSet::new();
        for (k, do_insert) in ops {
            if do_insert {
                // `insert` must agree with the model on novelty (uniqueness).
                prop_assert_eq!(d.insert(k), model.insert(k));
            } else {
                prop_assert_eq!(d.remove(&k), model.remove(&k));
            }
            prop_assert_eq!(d.len(), model.len());
            prop_assert_eq!(d.contains(&k), model.contains(&k));
            prop_assert_eq!(d.is_empty(), model.is_empty());
        }
    }

    /// Invariant 2: FIFO eviction keeps the newest `max` keys and drops the
    /// oldest, preserving insertion order among survivors. All keys here are
    /// normal (zoom 5 > BASE_LAYER_ZOOM) and dead, so eviction is unimpeded.
    #[test]
    fn fifo_eviction_keeps_newest(n in 1usize..64, max in 1usize..32) {
        let mut cache: GpuCache<TileKey, u64> =
            GpuCache::new(max, BaseLayerGuard::new(), zoom_of);
        let keys: Vec<TileKey> = (0..n as u32).map(|i| (i, 0, 5)).collect();
        for (i, k) in keys.iter().enumerate() {
            cache.insert(*k, i as u64);
        }

        let res = cache.evict(|_| false);

        let keep = n.min(max);
        let dropped = n - keep;
        prop_assert_eq!(cache.len(), keep);
        prop_assert_eq!(res.evicted as usize, dropped);
        prop_assert_eq!(res.deferred, 0);
        for k in keys.iter().take(dropped) {
            prop_assert!(!cache.contains_key(k), "oldest {:?} must be evicted first", k);
        }
        for k in keys.iter().skip(dropped) {
            prop_assert!(cache.contains_key(k), "newest {:?} must survive", k);
        }
        // Survivor FIFO order is exactly the tail of the insertion order.
        prop_assert!(cache
            .order()
            .iter()
            .copied()
            .eq(keys[dropped..].iter().copied()));
    }

    /// Invariants 3 + 4 + 5 (combined, the核心 花屏防护 guarantees):
    /// - live entities are NEVER evicted (deferred / pushed back),
    /// - base-layer tiles (zoom <= 3) are permanently exempt,
    /// - `len() <= max_entries + live_count` always holds after eviction.
    ///
    /// Base-layer keys are forced into the live set (semantically faithful: the
    /// base layer always has live entities). `max` is computed from the non-base
    /// live count so the FIFO recycle loop is guaranteed to terminate — this is
    /// the same relationship the golden path relies on
    /// (`MAX_TILE_ENTITIES(1800) << MAX_GPU_CACHE_ENTRIES(3000)`).
    #[test]
    fn live_and_base_never_evicted_len_bounded(
        raw in proptest::collection::vec((0u32..48, 0u32..8), 1..96),
        live_bits in proptest::collection::vec(any::<bool>(), 0..96),
        extra in 1usize..16,
    ) {
        let base = BaseLayerGuard::new(); // max_zoom = 3
        let mut all: Vec<TileKey> = Vec::new();
        let mut seen: HashSet<TileKey> = HashSet::new();
        let mut live_set: HashSet<TileKey> = HashSet::new();
        let mut base_set: HashSet<TileKey> = HashSet::new();

        for (i, (id, z)) in raw.iter().enumerate() {
            let k: TileKey = (*id, 0, *z);
            if !seen.insert(k) {
                continue; // distinct keys only
            }
            all.push(k);
            let is_base = base.is_base_layer(*z);
            let is_live = live_bits.get(i).copied().unwrap_or(false) || is_base;
            if is_base {
                base_set.insert(k);
            }
            if is_live {
                live_set.insert(k);
            }
        }

        // Termination guard: non-base live entries are recycled in the FIFO, so
        // they must be strictly fewer than `max`.
        let non_base_live = live_set
            .iter()
            .filter(|k| !base_set.contains(k))
            .count();
        let max = non_base_live + extra;

        let mut cache: GpuCache<TileKey, u64> = GpuCache::new(max, base, zoom_of);
        for (i, k) in all.iter().enumerate() {
            cache.insert(*k, i as u64);
        }
        for k in &live_set {
            cache.set_live(*k, true);
        }

        let res = cache.evict(|k| live_set.contains(k));

        // Invariant 3: no live key was evicted.
        for k in &live_set {
            prop_assert!(cache.contains_key(k), "live {:?} was evicted", k);
        }
        // Invariant 5: no base-layer key was evicted (permanent exemption).
        for k in &base_set {
            prop_assert!(cache.contains_key(k), "base-layer {:?} was evicted", k);
        }
        // Invariant 4: bounded growth — len <= max + live.
        prop_assert!(
            cache.len() <= max + live_set.len(),
            "len {} exceeds max {} + live {}",
            cache.len(),
            max,
            live_set.len()
        );
        // FIFO order is bounded by max after eviction.
        prop_assert!(cache.order().len() <= max, "order len {} > max {}", cache.order().len(), max);
        // Evictions only ever remove dead, non-base keys.
        let dead_normal = all
            .iter()
            .filter(|k| !live_set.contains(k) && !base_set.contains(k))
            .count();
        prop_assert!((res.evicted as usize) <= dead_normal);
    }

    /// Invariant 5 (focused): under extreme eviction pressure (tiny cap), the
    /// base layer survives intact no matter how many normal tiles churn.
    #[test]
    fn base_layer_permanently_exempt(n_normal in 1usize..64, n_base in 1usize..16) {
        let max = 2usize; // deliberately tiny → maximum pressure
        let mut cache: GpuCache<TileKey, u64> =
            GpuCache::new(max, BaseLayerGuard::new(), zoom_of);
        let mut live: HashSet<TileKey> = HashSet::new();
        for b in 0..n_base as u32 {
            let k: TileKey = (b, 1, 2); // zoom 2 <= BASE_LAYER_ZOOM
            cache.insert(k, b as u64);
            cache.set_live(k, true);
            live.insert(k);
        }
        for i in 0..n_normal as u32 {
            let k: TileKey = (i, 2, 9); // zoom 9 → normal, dead
            cache.insert(k, i as u64);
        }

        let _res = cache.evict(|k| live.contains(k));

        for b in 0..n_base as u32 {
            let k: TileKey = (b, 1, 2);
            prop_assert!(cache.contains_key(&k), "base-layer {:?} evicted under pressure", k);
        }
    }

    /// Invariant 6: staleness classification is a strict precedence chain
    /// (aborted > failed > placeholder > fresh) and the states are never merged.
    #[test]
    fn staleness_three_state_precedence(
        a in any::<bool>(),
        f in any::<bool>(),
        p in any::<bool>(),
    ) {
        let s = DefaultStaleness;
        let got = s.classify(a, f, p);
        let expected = if a {
            StalenessVerdict::Aborted
        } else if f {
            StalenessVerdict::Failed
        } else if p {
            StalenessVerdict::Placeholder
        } else {
            StalenessVerdict::Fresh
        };
        prop_assert_eq!(got, expected);
        // Each single-flag input maps to its own distinct verdict.
        if a && !f && !p {
            prop_assert_eq!(got, StalenessVerdict::Aborted);
        }
        if f && !a && !p {
            prop_assert_eq!(got, StalenessVerdict::Failed);
        }
        if p && !a && !f {
            prop_assert_eq!(got, StalenessVerdict::Placeholder);
        }
    }

    /// Invariant 7: worker backoff is exactly `base << attempt` (exponential)
    /// and the pipeline cooldown strictly dominates the whole worker window —
    /// the two timescales are never collapsed into one.
    #[test]
    fn retry_dual_timescale(attempt in 0u32..3) {
        let r = DefaultRetry;
        prop_assert_eq!(r.max_attempts(), 3);
        prop_assert_eq!(r.backoff_base(), Duration::from_millis(250));
        prop_assert_eq!(r.cooldown(), Duration::from_secs(10));
        prop_assert_eq!(
            DefaultRetry::backoff_for(attempt),
            r.backoff_base() * (1u32 << attempt)
        );
        prop_assert_eq!(
            DefaultRetry::backoff_for(attempt),
            Duration::from_millis(250u64 << attempt)
        );
        let window: Duration = (0..r.max_attempts()).map(DefaultRetry::backoff_for).sum();
        prop_assert!(
            r.cooldown() > window,
            "cooldown {:?} must dominate worker window {:?}",
            r.cooldown(),
            window
        );
        prop_assert!(r.cooldown() >= DefaultRetry::backoff_for(attempt));
    }

    /// Invariant 8: the hidden-entity LRU despawns the least-recently-hidden
    /// tiles first, exactly up to the requested budget.
    #[test]
    fn hidden_lru_evicts_oldest_first(
        ids in proptest::collection::vec(0u32..64, 1..64),
        budget in 1usize..16,
    ) {
        let mut lru = HiddenLru::<TileKey>::new();
        let mut order: Vec<TileKey> = Vec::new();
        let mut seen: HashSet<u32> = HashSet::new();
        for id in ids {
            if !seen.insert(id) {
                continue; // distinct keys → deterministic tick ordering
            }
            lru.advance_frame();
            let k: TileKey = (id, 0, 5);
            lru.hide(k);
            order.push(k);
        }

        let evicted = lru.pop_lru(budget);
        let take = budget.min(order.len());
        prop_assert!(evicted.iter().copied().eq(order[..take].iter().copied()));
        prop_assert_eq!(lru.len(), order.len() - take);
        // The survivors are the most-recently-hidden tail.
        for k in &order[take..] {
            prop_assert!(lru.is_hidden(k));
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Threaded pipeline invariant (in-flight uniqueness through `submit`).
// Fewer cases: each spawns a real worker pool.
// ─────────────────────────────────────────────────────────────────────────────

/// Backend that always succeeds instantly — the in-flight gauge is asserted
/// *before* any `poll_ready`, so worker timing is irrelevant to the invariant.
struct OkBackend;

impl NetworkBackend for OkBackend {
    fn fetch(&self, _url: &str) -> FetchResult {
        FetchResult::Ok(vec![1, 2, 3])
    }
    fn name(&self) -> &str {
        "ok-mock"
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(1)
    }
}

fn tile_url(k: &TileKey) -> String {
    format!("http://tiles/{}/{}/{}", k.2, k.0, k.1)
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::with_cases(64)
    })]

    /// Invariant 1 (pipeline level): `submit` dedups — after a batch of
    /// (possibly duplicated) submits and *no* `poll_ready`, both the dedup set
    /// (`load_n`) and the in-flight gauge (`dl_in_flight`) equal the number of
    /// DISTINCT keys submitted. This is deterministic because dedup removal
    /// only happens inside `poll_ready`/`cancel`, neither of which is called.
    #[test]
    fn pipeline_inflight_unique_under_duplicate_submits(
        raw in proptest::collection::vec((0u32..32, 0u32..4), 1..64),
    ) {
        let decode: Decoder<Vec<u8>> = Arc::new(|d: &[u8]| {
            if d.is_empty() {
                None
            } else {
                Some(d.to_vec())
            }
        });
        let cfg = PoolConfig {
            threads: 2,
            max_attempts: 1,
            backoff_base: Duration::from_millis(1),
        };
        let pipe: GenericPipeline<TileKey, Vec<u8>> = GenericPipeline::with_config(
            Arc::new(OkBackend),
            Arc::new(tile_url),
            decode,
            cfg,
        );

        let mut distinct: HashSet<TileKey> = HashSet::new();
        for (id, z) in &raw {
            let k: TileKey = (*id, 0, *z + 5); // zoom 5..9 (never base layer)
            pipe.submit(k, 1.0);
            distinct.insert(k);
        }

        let s = pipe.stats();
        prop_assert_eq!(s.load_n as usize, distinct.len());
        prop_assert_eq!(s.dl_in_flight as usize, distinct.len());

        pipe.shutdown();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Plain (non-property) golden-path constant confirmation via the public API.
// ─────────────────────────────────────────────────────────────────────────────

/// The budget constants exposed through `BudgetPolicy` must match the
/// golden-path reference exactly, and the eviction termination invariant
/// (`MAX_TILE_ENTITIES < MAX_GPU_CACHE_ENTRIES`) must hold.
#[test]
fn budget_golden_path_constants_and_termination() {
    let b = DefaultBudget;
    assert_eq!(b.download_threads(), 16);
    assert_eq!(b.max_mesh_uploads_per_frame(), 12);
    assert_eq!(b.max_spawns_per_frame(), 16);
    assert_eq!(b.max_texture_uploads_per_frame(), 16);
    assert_eq!(b.max_despawns_per_frame(), 24);
    assert_eq!(b.max_tile_entities(), 1800);
    assert_eq!(b.base_layer_zoom(), 3);
    assert_eq!(b.max_gpu_cache_entries(), 3000);
    assert!(
        b.max_tile_entities() < b.max_gpu_cache_entries(),
        "termination invariant: 1800 must be < 3000"
    );
}
