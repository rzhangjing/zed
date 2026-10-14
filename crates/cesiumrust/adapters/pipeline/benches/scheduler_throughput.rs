//! M8.5 — scheduler end-to-end throughput bench (gate 门⑤).
//!
//! Plan authority: `cesiumrust_P1P2P3_执行计划` L206 — M8 validation gate
//! "scheduler 端到端吞吐 ≥ M1 baseline"; L329 (Nora perf gate) — every
//! perf sub-item ships a `before.csv` / `after.csv` / `delta-report.md`
//! 三件套.
//!
//! # What is measured
//!
//! Both arms push `N` **distinct cold** tile fetches through the *same*
//! 16-worker keep-alive [`WorkerPool`] with the *same* deterministic instant
//! mock backend, then measure wall-clock throughput (jobs/sec):
//!
//! * **before — M1 baseline**: [`GenericPipeline`] (the M1.2 `TilePipeline`
//!   impl exercised by `tests/camera_jump_stress.rs`). One driver thread
//!   `submit`s all keys and `poll_ready`-drains the results — the push model
//!   the golden path uses.
//! * **after — M8 path**: [`PipelineResourceBackend`] (the M8 `ResourceBackend`
//!   impl). `DRIVERS` concurrent callers each `block_on(request_stream(key))`
//!   — the pull model `NetworkResourceBackend::fetch_url_blocking` uses. This
//!   arm additionally rides the M8 cache hierarchy (hot `GpuCache` + warm
//!   `HiddenLru`), in-flight `Dedup`, the dispatcher thread, and the
//!   waiter-channel bridge.
//!
//! The **delta** therefore isolates the overhead the M8 resource-backend
//! abstraction layer adds on top of the M1 scheduler engine. 门⑤ passes when
//! `after ≥ before` (no throughput regression).
//!
//! DEVIATION: 门⑤ 字面判定 = REGRESSION (after ≈194K < before ≈1.59M jobs/s,
//! ratio 0.122). Leader 裁定重诠释门⑤ 约束对象为 SCHEDULER ENGINE 层吞吐保持
//! (两臂共用同一 16-worker `WorkerPool` + 同一 `InstantMock`, before 臂 1.59M
//! 即引擎上限, 实测未回归); M8 `ResourceBackend` wrapper 层 8x 差距 =
//! per-request blocking-RPC 模型 (cache tiers + dedup + streaming + 全局 intake
//! 锁) 固有构造性开销, 非引擎回归; wrapper opt-in 未接黄金路径.
//! see docs/deviations.md#dev-020
//!
//! # Determinism / offline reproducibility
//!
//! * No real network: [`InstantMock`] returns canned bytes for every URL
//!   (offline, CI-safe, no GPU, no external HTTP → not flaky).
//! * Small fixed payload (`PAYLOAD` bytes) so the measurement isolates
//!   *scheduler dispatch throughput*, not memory bandwidth.
//! * Pool/backend spawn + driver-thread spawn are **excluded** from the timed
//!   region (criterion `iter_custom`; the artifact harness times only
//!   barrier-release → completion).
//! * `median`/`mean`/`stddev` over `RUNS` passes bound run-to-run variance.
//!
//! Run with:
//!
//! ```text
//! cargo bench -p cesium-pipeline --offline
//! ```
//!
//! The binary first writes the 三件套 to
//! `adapters/pipeline/perf/m8_scheduler/{before.csv,after.csv,delta-report.md}`
//! (path resolved via `CARGO_MANIFEST_DIR`, CWD-independent), then runs the
//! criterion benches.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use criterion::{criterion_group, Criterion};

use cesium_pipeline::base_layer::BaseLayerGuard;
use cesium_pipeline::budget::DefaultBudget;
use cesium_pipeline::net::{FetchResult, NetworkBackend};
use cesium_pipeline::pool::{Decoder, PoolConfig};
use cesium_pipeline::resource_backend::PipelineResourceBackend;
use cesium_pipeline::runtime::{GenericPipeline, UrlBuilder};
use cesium_ports_driven::{ResourceBackend, TilePipeline};

/// Canonical tile key `(x, y, zoom)` — matches `dynamic_globe.rs:77`.
type TileKey = (u32, u32, u32);

/// Worker-pool size — the golden-path constant (`dynamic_globe.rs:48`).
const THREADS: usize = DefaultBudget::DOWNLOAD_THREADS; // 16
/// Concurrent callers saturating the M8 pull path (> THREADS keeps the pool busy).
const DRIVERS: usize = 32;
/// Canned payload size (bytes). Deliberately small: the bench isolates
/// scheduler dispatch throughput, not payload copy / bandwidth.
const PAYLOAD: usize = 64;
/// Jobs per criterion iteration (bounded so `cargo bench` stays quick).
const N_CRITERION: usize = 1_000;
/// Jobs per artifact-harness pass (larger → smoother throughput estimate).
const N_ARTIFACT: usize = 2_000;
/// Deterministic passes for the 三件套 (odd → clean median).
const RUNS: usize = 9;
/// Idle backoff between empty `poll_ready` drains (before arm).
const POLL_IDLE: Duration = Duration::from_micros(100);

// ── Deterministic instant mock backend (offline, no network) ────────────────

/// Returns canned bytes for **every** URL instantly. Shared by both arms so
/// the only variable is the pipeline abstraction layer, never the backend.
struct InstantMock {
    payload: usize,
}

impl InstantMock {
    fn new(payload: usize) -> Self {
        Self { payload }
    }
}

impl NetworkBackend for InstantMock {
    fn fetch(&self, _url: &str) -> FetchResult {
        FetchResult::Ok(vec![0xA5; self.payload])
    }
    fn name(&self) -> &str {
        "instant-mock"
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(1)
    }
}

/// Deterministic distinct key generator (zoom 8 → above the base layer, so no
/// base-layer exemption skews the M8 hot cache).
fn key_gen(i: usize) -> TileKey {
    ((i & 0xFFFF) as u32, ((i >> 16) & 0xFFFF) as u32, 8)
}

fn url_of(k: &TileKey) -> String {
    format!("http://bench-tiles/{}/{}/{}", k.2, k.0, k.1)
}

fn zoom_of(k: &TileKey) -> u32 {
    k.2
}

/// Deterministic f64 priority spread (exercises the priority field; the pool
/// itself is FIFO, so this does not reorder — it only proves f64 fidelity).
fn priority_of(i: usize) -> f64 {
    1.0 + (i % 64) as f64 * 0.01
}

fn decode_identity() -> Decoder<Vec<u8>> {
    Arc::new(|d: &[u8]| if d.is_empty() { None } else { Some(d.to_vec()) })
}

/// Production-shaped pool config (3 attempts) with a tiny backoff so a
/// spurious transient never stalls the bench (the instant mock never fails).
fn fast_config(threads: usize) -> PoolConfig {
    PoolConfig {
        threads,
        max_attempts: 3,
        backoff_base: Duration::from_millis(1),
    }
}

/// Drive a boxed future to completion with a single `Waker::noop` poll. The M8
/// `request_stream` future blocks internally on an `mpsc` receiver, so one poll
/// always yields `Ready` (mirrors `resource_backend.rs` test harness + the
/// `NetworkResourceBackend::block_on_noop` bridge — no tokio, no executor).
fn block_on<F: Future>(fut: F) -> F::Output {
    use std::task::{Context, Poll, Waker};
    let mut fut = Box::pin(fut);
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(out) => out,
        Poll::Pending => panic!("M8 request_stream must block to Ready in one noop-waker poll"),
    }
}

// ── Measurement kernels (shared by criterion + the 三件套 harness) ───────────

/// M1 baseline: [`GenericPipeline`] submit-all + poll-drain throughput.
/// Returns `(timed_elapsed, completed_outcomes)`. Pool spawn is untimed.
fn measure_m1_baseline(n: usize) -> (Duration, usize) {
    let mock: Arc<dyn NetworkBackend> = Arc::new(InstantMock::new(PAYLOAD));
    let pipe: GenericPipeline<TileKey, Vec<u8>> = GenericPipeline::with_config(
        mock,
        Arc::new(url_of) as UrlBuilder<TileKey>,
        decode_identity(),
        fast_config(THREADS),
    );

    let start = Instant::now();
    for i in 0..n {
        pipe.submit(key_gen(i), priority_of(i));
    }
    let mut completed = 0usize;
    while completed < n {
        let batch = pipe.poll_ready(64);
        if batch.is_empty() {
            std::thread::sleep(POLL_IDLE);
        }
        completed += batch.len();
    }
    let elapsed = start.elapsed();

    pipe.shutdown();
    (elapsed, completed)
}

/// M8 path: [`PipelineResourceBackend`] concurrent `request_stream` throughput.
/// Returns `(timed_elapsed, completed_ok)`. Backend + driver-thread spawn are
/// untimed (drivers block on a barrier; the clock starts at release).
fn measure_m8_backend(n: usize, drivers: usize) -> (Duration, usize) {
    let mock: Arc<dyn NetworkBackend> = Arc::new(InstantMock::new(PAYLOAD));
    let backend = Arc::new(PipelineResourceBackend::with_config(
        "bench-m8-backend",
        mock,
        Arc::new(url_of) as UrlBuilder<TileKey>,
        zoom_of,
        BaseLayerGuard::new(),
        n + 16, // head-room so no FIFO eviction contaminates the cold path
        fast_config(THREADS),
    ));

    let keys: Arc<Vec<TileKey>> = Arc::new((0..n).map(key_gen).collect());
    let completed = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(drivers + 1));
    let chunk = n.div_ceil(drivers);

    let mut handles = Vec::with_capacity(drivers);
    for d in 0..drivers {
        let be = Arc::clone(&backend);
        let keys = Arc::clone(&keys);
        let comp = Arc::clone(&completed);
        let bar = Arc::clone(&barrier);
        let lo = d * chunk;
        let hi = lo.saturating_add(chunk).min(n);
        handles.push(std::thread::spawn(move || {
            bar.wait(); // released by the main thread inside the timed region
            for i in lo..hi {
                if block_on(be.request_stream(keys[i], priority_of(i))).is_ok() {
                    comp.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }

    let start = Instant::now();
    barrier.wait(); // release all drivers at once
    for h in handles {
        let _ = h.join();
    }
    let elapsed = start.elapsed();

    let done = completed.load(Ordering::Relaxed);
    drop(backend); // stop the dispatcher thread
    (elapsed, done)
}

// ── Statistics helpers ──────────────────────────────────────────────────────

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = s.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

fn stddev(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
}

fn throughput(completed: usize, elapsed: Duration) -> f64 {
    let secs = elapsed.as_secs_f64();
    if secs <= 0.0 {
        0.0
    } else {
        completed as f64 / secs
    }
}

struct Arm {
    elapsed_ms: Vec<f64>,
    throughput: Vec<f64>,
    completed: Vec<usize>,
}

impl Arm {
    fn new() -> Self {
        Self {
            elapsed_ms: Vec::new(),
            throughput: Vec::new(),
            completed: Vec::new(),
        }
    }
    fn push(&mut self, completed: usize, elapsed: Duration) {
        self.elapsed_ms.push(elapsed.as_secs_f64() * 1000.0);
        self.throughput.push(throughput(completed, elapsed));
        self.completed.push(completed);
    }
}

// ── 三件套 artifact emitter (before.csv / after.csv / delta-report.md) ───────

fn artifact_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("perf/m8_scheduler")
}

fn write_csv(path: &std::path::Path, arm: &Arm, n: usize) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str("run,jobs,completed,elapsed_ms,throughput_jobs_per_sec\n");
    for (i, (c, (ms, tp))) in arm
        .completed
        .iter()
        .zip(arm.elapsed_ms.iter().zip(arm.throughput.iter()))
        .enumerate()
    {
        out.push_str(&format!(
            "{},{},{},{:.3},{:.1}\n",
            i + 1,
            n,
            c,
            ms,
            tp
        ));
    }
    std::fs::write(path, out)
}

fn emit_three_piece_artifacts() {
    let dir = artifact_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[m8.5] cannot create artifact dir {}: {e}", dir.display());
        return;
    }

    // Warm-up (untimed): stabilise allocator / thread pools before sampling.
    let _ = measure_m1_baseline(N_ARTIFACT);
    let _ = measure_m8_backend(N_ARTIFACT, DRIVERS);

    let mut before = Arm::new();
    let mut after = Arm::new();
    for _ in 0..RUNS {
        let (d, c) = measure_m1_baseline(N_ARTIFACT);
        before.push(c, d);
        let (d, c) = measure_m8_backend(N_ARTIFACT, DRIVERS);
        after.push(c, d);
    }

    let _ = write_csv(&dir.join("before.csv"), &before, N_ARTIFACT);
    let _ = write_csv(&dir.join("after.csv"), &after, N_ARTIFACT);

    let b_med = median(&before.throughput);
    let a_med = median(&after.throughput);
    let ratio = if b_med > 0.0 { a_med / b_med } else { 0.0 };
    let verdict = if ratio >= 1.0 {
        "PASS — after ≥ M1 baseline (no regression)"
    } else if ratio >= 0.95 {
        "PASS — within 5% of M1 baseline (Nora G5 tolerance)"
    } else {
        "REGRESSION — after < 95% of M1 baseline"
    };

    let report = format!(
        "# M8.5 scheduler 端到端吞吐 — delta report (门⑤)\n\n\
         > 权威判据：计划 L206「scheduler 端到端吞吐 ≥ M1 baseline」；三件套口径 L329（Nora 性能门）。\n\
         > 生成者：`adapters/pipeline/benches/scheduler_throughput.rs`（`cargo bench -p cesium-pipeline --offline`）。\n\
         > 离线确定性：`InstantMock` 对每个 URL 返回固定 {payload}B，无真实网络 / 无 GPU / 无外网。\n\n\
         ## 度量方法\n\n\
         | 臂 | 路径 | 驱动模型 | 计时区 |\n|---|---|---|---|\n\
         | before (M1 baseline) | `GenericPipeline` (M1.2 `TilePipeline`) | 单驱动 `submit`×N + `poll_ready` drain | submit→drain 完成 |\n\
         | after (M8 path) | `PipelineResourceBackend` (M8 `ResourceBackend`) | {drivers} 并发 caller `block_on(request_stream)` | barrier release→全 join |\n\n\
         两臂共用同一 {threads}-worker keep-alive `WorkerPool` + 同一 `InstantMock` + N 个互异冷 key；\n\
         pool/backend/driver 线程 spawn 均排除在计时区外。delta = M8 cache 层级(hot `GpuCache`+warm `HiddenLru`)+`Dedup`+dispatcher+waiter 桥的净开销。\n\n\
         ## 实测（N={n} jobs/pass，RUNS={runs}，单位 jobs/sec）\n\n\
         | 统计 | before (M1) | after (M8) |\n|---|---|---|\n\
         | median | {b_med:.1} | {a_med:.1} |\n\
         | mean | {b_mean:.1} | {a_mean:.1} |\n\
         | stddev | {b_sd:.1} | {a_sd:.1} |\n\
         | min | {b_min:.1} | {a_min:.1} |\n\
         | max | {b_max:.1} | {a_max:.1} |\n\
         | completed (median) | {b_done} / {n} | {a_done} / {n} |\n\n\
         ## 门⑤ 判定\n\n\
         - **ratio = after_median / before_median = {ratio:.4}**\n\
         - **结论：{verdict}**\n\n\
         ## 复现\n\n\
         ```text\n cargo bench -p cesium-pipeline --offline\n ```\n\n\
         逐 pass 原始样本见同目录 `before.csv` / `after.csv`（列：run,jobs,completed,elapsed_ms,throughput_jobs_per_sec）。\n",
        payload = PAYLOAD,
        drivers = DRIVERS,
        threads = THREADS,
        n = N_ARTIFACT,
        runs = RUNS,
        b_med = b_med,
        a_med = a_med,
        b_mean = mean(&before.throughput),
        a_mean = mean(&after.throughput),
        b_sd = stddev(&before.throughput),
        a_sd = stddev(&after.throughput),
        b_min = before.throughput.iter().cloned().fold(f64::INFINITY, f64::min),
        a_min = after.throughput.iter().cloned().fold(f64::INFINITY, f64::min),
        b_max = before.throughput.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        a_max = after.throughput.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        b_done = median(&before.completed.iter().map(|&c| c as f64).collect::<Vec<_>>()) as usize,
        a_done = median(&after.completed.iter().map(|&c| c as f64).collect::<Vec<_>>()) as usize,
        ratio = ratio,
        verdict = verdict,
    );

    if let Err(e) = std::fs::write(dir.join("delta-report.md"), &report) {
        eprintln!("[m8.5] cannot write delta-report.md: {e}");
    }

    // Console summary so `cargo bench` surfaces the verdict even without the file.
    println!("[m8.5] before(M1) median = {b_med:.1} jobs/s");
    println!("[m8.5] after (M8) median = {a_med:.1} jobs/s");
    println!("[m8.5] ratio = {ratio:.4} → {verdict}");
    println!("[m8.5] 三件套 written to {}", dir.display());
}

// ── criterion benches (mirror the M7-C `styling_eval` convention) ───────────

fn bench_m1_baseline(c: &mut Criterion) {
    c.bench_function("scheduler/m1_baseline_generic_pipeline", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let (d, _) = measure_m1_baseline(N_CRITERION);
                total += d;
            }
            total
        })
    });
}

fn bench_m8_backend(c: &mut Criterion) {
    c.bench_function("scheduler/m8_resource_backend", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let (d, _) = measure_m8_backend(N_CRITERION, DRIVERS);
                total += d;
            }
            total
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .measurement_time(Duration::from_secs(2))
        .warm_up_time(Duration::from_millis(500));
    targets = bench_m1_baseline, bench_m8_backend
}

fn main() {
    // Emit the 三件套 first (bounded, ~1 s), then run the criterion benches.
    emit_three_piece_artifacts();
    benches();
}
