//! M8 — Property-based contract tests for the `ResourceBackend` adapter
//! (`cesium_pipeline::resource_backend::PipelineResourceBackend`).
//!
//! Scope: **pure `std` core only** (no Bevy, no GPU, no real network — a mock
//! `NetworkBackend` stands in). These properties prove that the M8 bulk-asset
//! streaming backend's **eviction / dedup invariants are identical to the
//! reused `GpuCache` / `HiddenLru` / `Dedup` primitives** — i.e. M8 did not
//! smuggle in a parallel cache system that could break the three M1.2
//! invariants protecting the `dynamic_globe` golden path from 花屏.
//!
//! Invariants covered (proptest), mirroring `pipeline_invariants.rs`:
//! 1. FIFO eviction keeps newest          — `evict` + `cache_tier`
//! 2. live + base never evicted, len bound — three invariants via the backend
//! 3. BASE_LAYER permanently exempt        — under extreme pressure
//! 4. cache-tier classification (Hot/Warm/Cold) is consistent with the reused
//!    `GpuCache` (membership) + `HiddenLru` (hidden) state
//! 5. dedup collapses concurrent same-key cold requests to ONE fetch
//!
//! DEFERRED (registered, NOT implemented here): pixel-level 花屏 detection
//! (adjacent-frame diff) needs a real GPU renderer + framebuffer readback —
//! out of scope for the pure-`std` core suite (see `pipeline_invariants.rs`
//! and `camera_jump_stress.rs` for the matching deferral, `deferred.md #23`).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;

use proptest::prelude::*;

use cesium_pipeline::base_layer::BaseLayerGuard;
use cesium_pipeline::net::{FetchResult, NetworkBackend};
use cesium_pipeline::pool::PoolConfig;
use cesium_pipeline::resource_backend::PipelineResourceBackend;

use cesium_ports_driven::{CacheTier, ResourceBackend};

/// Canonical asset key `(x, y, zoom)` — the same shape the tile pipeline uses.
type TileKey = (u32, u32, u32);

fn zoom_of(k: &TileKey) -> u32 {
    k.2
}

fn url_of(k: &TileKey) -> String {
    format!("http://assets/{}/{}/{}", k.2, k.0, k.1)
}

/// Drive a boxed future to completion with a single `Waker::noop` poll. The
/// `request_stream` future blocks internally on an `mpsc` receiver, so one poll
/// always yields `Ready` (no executor, no tokio — matches `offline_check.rs`).
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    let mut fut = Box::pin(fut);
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(out) => out,
        Poll::Pending => panic!("resource future returned Pending (must block to Ready)"),
    }
}

/// Network backend that is never called (cache-hierarchy properties only use
/// the inherent `insert` / `evict` / `hide` / `show` / `cache_tier` surface).
struct NullNet;

impl NetworkBackend for NullNet {
    fn fetch(&self, _url: &str) -> FetchResult {
        FetchResult::Transient("NullNet must not be called".into())
    }
    fn name(&self) -> &str {
        "null"
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(1)
    }
}

/// Build a cache-only backend (1 idle worker, no network) with the given cap.
fn cache_backend(max_entries: usize) -> PipelineResourceBackend<TileKey> {
    cache_backend_with(max_entries, BaseLayerGuard::new())
}

fn cache_backend_with(
    max_entries: usize,
    guard: BaseLayerGuard,
) -> PipelineResourceBackend<TileKey> {
    PipelineResourceBackend::with_config(
        "prop-cache-backend",
        Arc::new(NullNet),
        Arc::new(url_of),
        zoom_of,
        guard,
        max_entries,
        PoolConfig {
            threads: 1,
            max_attempts: 1,
            backoff_base: Duration::from_millis(1),
        },
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Cache-hierarchy invariants (synchronous; each case builds a fresh backend).
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::with_cases(64)
    })]

    /// Invariant 1: FIFO eviction through the backend keeps the newest `max`
    /// assets and drops the oldest — byte-for-byte the same property the
    /// reused `GpuCache` satisfies (`fifo_eviction_keeps_newest`). Evicted keys
    /// report `CacheTier::Cold`; survivors report `CacheTier::Hot`.
    #[test]
    fn resource_fifo_eviction_keeps_newest(n in 1usize..64, max in 1usize..32) {
        let be = cache_backend(max);
        let keys: Vec<TileKey> = (0..n as u32).map(|i| (i, 0, 5)).collect();
        for (i, k) in keys.iter().enumerate() {
            be.insert(*k, vec![i as u8]);
        }

        let res = be.evict(|_| false);

        let keep = n.min(max);
        let dropped = n - keep;
        prop_assert_eq!(be.cache_len(), keep);
        prop_assert_eq!(res.evicted as usize, dropped);
        prop_assert_eq!(res.deferred, 0);
        for k in keys.iter().take(dropped) {
            prop_assert_eq!(be.cache_tier(k), CacheTier::Cold, "oldest {:?} must evict", k);
        }
        for k in keys.iter().skip(dropped) {
            prop_assert_eq!(be.cache_tier(k), CacheTier::Hot, "newest {:?} must survive", k);
        }
        prop_assert!(be.evict_order_len() <= max, "FIFO order must be bounded by cap");
    }

    /// Invariants 2 + 3 (the 花屏防护 core): live assets are never evicted,
    /// base-layer assets (zoom <= 3) are permanently exempt, and the cache size
    /// stays bounded by `max + live`. Mirrors `live_and_base_never_evicted_
    /// len_bounded`, asserted through the backend's `cache_tier` / `cache_len`.
    #[test]
    fn resource_live_and_base_never_evicted_len_bounded(
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

        // Termination guard: non-base live entries recycle in the FIFO, so they
        // must be strictly fewer than `max` (same relationship the golden path
        // relies on: MAX_TILE_ENTITIES(1800) << MAX_GPU_CACHE_ENTRIES(3000)).
        let non_base_live = live_set.iter().filter(|k| !base_set.contains(k)).count();
        let max = non_base_live + extra;

        let be = cache_backend_with(max, base);
        for (i, k) in all.iter().enumerate() {
            be.insert(*k, vec![i as u8]);
        }
        for k in &live_set {
            be.set_live(*k, true);
        }

        let res = be.evict(|k| live_set.contains(k));

        // Invariant 2: no live key was evicted.
        for k in &live_set {
            prop_assert_ne!(be.cache_tier(k), CacheTier::Cold, "live {:?} was evicted", k);
        }
        // Invariant 3: no base-layer key was evicted (permanent exemption).
        for k in &base_set {
            prop_assert_ne!(be.cache_tier(k), CacheTier::Cold, "base {:?} was evicted", k);
        }
        // Bounded growth: len <= max + live.
        prop_assert!(
            be.cache_len() <= max + live_set.len(),
            "len {} exceeds max {} + live {}",
            be.cache_len(),
            max,
            live_set.len()
        );
        prop_assert!(be.evict_order_len() <= max, "order len exceeds cap");
        // Evictions only ever remove dead, non-base keys.
        let dead_normal = all
            .iter()
            .filter(|k| !live_set.contains(k) && !base_set.contains(k))
            .count();
        prop_assert!((res.evicted as usize) <= dead_normal);
    }

    /// Invariant 3 (focused): under extreme eviction pressure (tiny cap), the
    /// base layer survives intact no matter how many normal assets churn.
    /// Mirrors `base_layer_permanently_exempt`.
    #[test]
    fn resource_base_layer_permanently_exempt(n_normal in 1usize..64, n_base in 1usize..16) {
        let be = cache_backend(2); // deliberately tiny → maximum pressure
        let mut live: HashSet<TileKey> = HashSet::new();
        for b in 0..n_base as u32 {
            let k: TileKey = (b, 1, 2); // zoom 2 <= BASE_LAYER_ZOOM
            be.insert(k, vec![b as u8]);
            be.set_live(k, true);
            live.insert(k);
        }
        for i in 0..n_normal as u32 {
            let k: TileKey = (i, 2, 9); // zoom 9 → normal, dead
            be.insert(k, vec![0]);
        }

        let _res = be.evict(|k| live.contains(k));

        for b in 0..n_base as u32 {
            let k: TileKey = (b, 1, 2);
            prop_assert_ne!(
                be.cache_tier(&k),
                CacheTier::Cold,
                "base-layer {:?} evicted under pressure",
                k
            );
        }
    }

    /// Invariant 4: the Hot/Warm/Cold classification reported by `cache_tier`
    /// is exactly consistent with the reused primitives — `Cold` iff absent
    /// from `GpuCache`, `Warm` iff present AND tracked by `HiddenLru`, `Hot`
    /// iff present and not hidden. No tier is ever mis-reported.
    #[test]
    fn resource_cache_tier_consistency(
        ops in proptest::collection::vec((0u32..32, 0u32..3), 0..128),
    ) {
        let be = cache_backend(4096); // large cap → no eviction interference
        let mut cached: HashSet<TileKey> = HashSet::new();
        let mut hidden: HashSet<TileKey> = HashSet::new();
        let mut all_keys: HashSet<TileKey> = HashSet::new();

        for (id, op) in &ops {
            let k: TileKey = (*id, 0, 5);
            all_keys.insert(k);
            match op {
                0 => {
                    be.insert(k, vec![1]);
                    cached.insert(k);
                }
                1 => {
                    be.hide(k);
                    hidden.insert(k);
                }
                _ => {
                    be.show(&k);
                    hidden.remove(&k);
                }
            }
        }

        for k in &all_keys {
            let expected = if !cached.contains(k) {
                CacheTier::Cold
            } else if hidden.contains(k) {
                CacheTier::Warm
            } else {
                CacheTier::Hot
            };
            prop_assert_eq!(be.cache_tier(k), expected, "tier mismatch for {:?}", k);
            // `cached_bytes` presence must agree with non-Cold membership.
            prop_assert_eq!(be.cached_bytes(k).is_some(), cached.contains(k));
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Threaded dedup invariant (gated network → deterministic, no flakiness).
// ─────────────────────────────────────────────────────────────────────────────

/// Counting backend whose single fetch blocks until the test releases the gate,
/// so every concurrent cold caller reaches the intake decision (and the dedup
/// set) before any result can complete. Makes the dedup invariant deterministic.
struct GatedNet {
    count: AtomicUsize,
    release: AtomicBool,
}

impl NetworkBackend for GatedNet {
    fn fetch(&self, _url: &str) -> FetchResult {
        self.count.fetch_add(1, Ordering::SeqCst);
        while !self.release.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
        FetchResult::Ok(vec![42])
    }
    fn name(&self) -> &str {
        "gated"
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::with_cases(16)
    })]

    /// Invariant 5: `n` concurrent cold requests for the SAME key collapse to
    /// exactly ONE network fetch (the reused `Dedup` set), with `n - 1` callers
    /// recorded as deduped waiters — and all `n` receive the same payload. This
    /// is the resource-streaming analogue of `pipeline_inflight_unique_under_
    /// duplicate_submits`.
    #[test]
    fn resource_dedup_collapses_concurrent_same_key(n in 2usize..16) {
        let net = Arc::new(GatedNet {
            count: AtomicUsize::new(0),
            release: AtomicBool::new(false),
        });
        let be = Arc::new(PipelineResourceBackend::with_config(
            "dedup-prop",
            Arc::clone(&net) as Arc<dyn NetworkBackend>,
            Arc::new(url_of),
            zoom_of,
            BaseLayerGuard::new(),
            100,
            PoolConfig {
                threads: 4,
                max_attempts: 1,
                backoff_base: Duration::from_millis(1),
            },
        ));

        let key: TileKey = (1, 1, 5);
        let mut handles = Vec::with_capacity(n);
        for _ in 0..n {
            let be = Arc::clone(&be);
            handles.push(thread::spawn(move || block_on(be.request_stream(key, 1.0))));
        }

        // All callers reach intake while the single fetch is gated open.
        thread::sleep(Duration::from_millis(120));
        prop_assert_eq!(net.count.load(Ordering::SeqCst), 1, "exactly one fetch in-flight");
        prop_assert_eq!(be.stats().deduped as usize, n - 1, "n-1 callers deduped");
        prop_assert_eq!(be.stats().in_flight, 1, "one distinct key in-flight");

        net.release.store(true, Ordering::SeqCst);
        for h in handles {
            let r = h.join().unwrap();
            prop_assert_eq!(r.unwrap(), vec![42]);
        }
        prop_assert_eq!(net.count.load(Ordering::SeqCst), 1, "dedup collapsed to one fetch");
        prop_assert_eq!(be.cache_tier(&key), CacheTier::Hot);
        prop_assert_eq!(be.stats().in_flight, 0, "in-flight cleared after completion");
    }
}
