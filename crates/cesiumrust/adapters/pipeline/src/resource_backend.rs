//! M8 / P2.3 — `ResourceBackend` 适配器：通过流水线管理的缓存层级
//! 实现批量资产流式传输。
//!
//! 本模块为批量资产流式传输（纹理 / 网格 / 3D Tiles 内容）实现了
//! [`cesium_ports_driven::ResourceBackend`] 契约。它是一个**纯 `std`、无 tokio**
//! 的 IO/缓存层适配器，与 M1.2 瓦片流水线并列存在，并*复用*其缓存
//! 原语，而非另建一套平行的缓存系统：
//!
//! | 层级 | 复用的原语 | 角色 |
//! |------|------------------|------|
//! | 热  | [`GpuCache`]     | FIFO 驱逐 + 基础层锁定 + 活实体延迟（M1.2 的三个不变式） |
//! | 温 | [`HiddenLru`]    | 隐藏的温回退追踪，在预算内按 LRU 优先 despawn |
//! | —    | [`Dedup`]        | 在途去重（每个冷键一次网络抓取） |
//! | —    | [`WorkerPool`]   | 现有的 16-worker 阻塞池 + keep-alive（ureq） |
//!
//! # 此处遵守的硬约束
//!
//! - **无平行缓存系统。** 驱逐/去重语义逐字委托给
//!   [`GpuCache`] / [`HiddenLru`] / [`Dedup`]，因此保护
//!   `dynamic_globe` 黄金路径免于花屏的三个不变式（BASE_LAYER 豁免 / 活实体延迟 / 终止性）
//!   得以保留。
//! - **无 tokio 主 runtime。** 网络流式传输依托现有的阻塞
//!   [`WorkerPool`]（std 线程 + `mpsc`，ureq keep-alive）。[`ResourceBackend::request_stream`]
//!   返回的装箱 future 会在一个 `mpsc` receiver 上阻塞*调用*
//!   线程，直到内部 dispatcher 交付——因此它
//!   必须从 IO/worker 上下文驱动，绝不能从帧线程驱动。这与 `pool.rs` 的阻塞池哲学
//!   以及其他地方使用的 `Waker::noop` `block_on` 模式一致（无 executor，无 tokio）。
//! - **无 glam / 无坐标数学。** M8 是一个 IO/缓存层；键是不透明的
//!   （`Hash + Eq + Copy`），唯一的数值类型是 `f64` 优先级。
//! - **选择性启用，未接线。** 此 backend *不会*被注册到任何默认的
//!   `cesium-app` plugin（M1.3 推广守卫）：在 M8 被显式提升之前，黄金路径
//!   保持像素中性。
//!
//! # 命名澄清（M8 vs M12）
//!
//! 参见 [`cesium_ports_driven::ResourceBackend`] 上的 trait 级文档。简单
//! 来说：此流式传输/缓存契约与 M12 的 `ENV_ENABLE_RESOURCE_BACKEND` 标志
//! （`Resource` 对象抽象：URL 模板 / 查询参数 / 重试头）
//! **在语义上无关**。M8 **不会**读取或复用那个标志。

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cesium_ports_driven::{
    CacheTier, PortError, PortResult, ResourceBackend, ResourceStats,
};

use crate::base_layer::BaseLayerGuard;
use crate::budget::DefaultBudget;
use crate::dedup::Dedup;
use crate::gpu_cache::{EvictionResult, GpuCache};
use crate::hidden_lru::HiddenLru;
use crate::net::NetworkBackend;
use crate::pool::{Decoder, Job, JobOutcome, PoolConfig, WorkerPool};
use crate::runtime::UrlBuilder;

/// 每键的完成发送器。一次冷的 [`ResourceBackend::request_stream`] 调用
/// 会在此注册一个发送器；dispatcher 线程将流式结果
/// 交付给该键的每一个等待者（因此同一键的并发冷请求
/// 共享单次网络抓取）。
type Waiters<K> = HashMap<K, Vec<mpsc::Sender<PortResult<Vec<u8>>>>>;

/// 累计流式传输计数器（原子量——由 `stats()` 跨线程读取）。
struct RbStats {
    /// 成功流式传输进入缓存的资产。
    streamed: AtomicU32,
    /// 被去重的冷请求（已在途中）。
    deduped: AtomicU32,
    /// 热层的 FIFO 驱逐。
    evicted: AtomicU32,
}

impl RbStats {
    /// 构造一个所有计数器归零的后端统计块。
    fn new() -> Self {
        Self {
            streamed: AtomicU32::new(0),
            deduped: AtomicU32::new(0),
            evicted: AtomicU32::new(0),
        }
    }
}

/// 内部共享状态，由 backend 句柄与 dispatcher
/// 线程共同拥有（通过 `Arc`）。
struct Inner<K>
where
    K: Hash + Eq + Copy + Send + 'static,
{
    /// 诊断用名称。
    name: String,
    /// 热/温数据存储（复用的 `GpuCache`，FIFO + 三个不变式）。
    cache: Mutex<GpuCache<K, Vec<u8>>>,
    /// 温层隐藏-LRU 追踪（复用的 `HiddenLru`）。
    hidden: Mutex<HiddenLru<K>>,
    /// 在途去重（复用的 `Dedup`）。
    dedup: Dedup<K>,
    /// 现有的 16-worker 阻塞池 + ureq keep-alive（复用的 `WorkerPool`）。
    pool: WorkerPool<K, Vec<u8>>,
    /// 每键的完成等待者（push 池 → pull future 的桥接）。
    waiters: Mutex<Waiters<K>>,
    /// 键 → 抓取 URL。
    url_builder: UrlBuilder<K>,
    /// 将冷承受判定（缓存未命中检查 + 去重标记 +
    /// 提交）与 dispatcher 的完成（去重清除 + 缓存
    /// 插入）串行化。没有它，一次完成可能插入到调用者的
    /// 缓存未命中检查与其 `dedup.insert` 之间，为同一个冷键产生一个冗余的第二次
    /// 抓取。锁顺序总是先 `intake`，然后
    /// 短命的 `cache` / `dedup` / `waiters` 锁（从不同时持有其中两个），
    /// 因此不存在反转。
    intake: Mutex<()>,
    /// 累计计数器。
    stats: RbStats,
    /// 在 `Drop` 时置位以停止 dispatcher 线程。
    shutdown: AtomicBool,
    /// 仅测试的接缝（M2）：置位时，dispatcher 会在其下一批
    /// 非空结果上 panic，以便 [`run_dispatcher_guarded`]
    /// 中的 `catch_unwind` + 等待者排空恢复可以被确定性地演练。
    #[cfg(test)]
    test_panic: AtomicBool,
}

/// 面向批量资产流式传输的流水线支撑 [`ResourceBackend`]。
///
/// 对资产键 `K` 泛型（例如 `TileKey = (u32, u32, u32)`，或一个
/// `u64` 内容哈希）。一旦 `K` 具体化即为 dyn 兼容：
/// `Box<dyn ResourceBackend<TileKey>>`。
///
/// 通过 [`PipelineResourceBackend::new`]（黄金路径默认值）或
/// [`PipelineResourceBackend::with_config`]（显式的池/缓存配置，由测试使用）
/// 构造。丢弃该 backend 会停止内部 dispatcher 线程并
/// 关闭工作线程池。
pub struct PipelineResourceBackend<K>
where
    K: Hash + Eq + Copy + Send + 'static,
{
    inner: Arc<Inner<K>>,
    /// 内部 dispatcher：排空池结果 → 缓存 + 等待者。
    dispatcher: Option<thread::JoinHandle<()>>,
}

impl<K> PipelineResourceBackend<K>
where
    K: Hash + Eq + Copy + Send + 'static,
{
    /// 以黄金路径默认值创建一个 backend：16 个下载线程
    /// （`DefaultBudget::DOWNLOAD_THREADS`）、一个 3000 条目的热缓存
    /// （`MAX_GPU_CACHE_ENTRIES`）、基础层 zoom 3，以及标准的
    /// 双时间尺度重试（3 次尝试，250 ms 基准退避）。
    pub fn new(
        name: impl Into<String>,
        backend: Arc<dyn NetworkBackend>,
        url_builder: UrlBuilder<K>,
        zoom_of: fn(&K) -> u32,
    ) -> Self {
        Self::with_config(
            name,
            backend,
            url_builder,
            zoom_of,
            BaseLayerGuard::new(),
            DefaultBudget::MAX_GPU_CACHE_ENTRIES,
            PoolConfig {
                threads: DefaultBudget::DOWNLOAD_THREADS,
                max_attempts: 3,
                backoff_base: Duration::from_millis(250),
            },
        )
    }

    /// 以显式配置创建一个 backend（由测试使用）。
    ///
    /// - `base_guard`：基础层 zoom 豁免（不变式 1）。
    /// - `max_cache_entries`：热缓存 FIFO 上限（不变式 3 终止性）。
    /// - `config`：工作线程池线程数 + 重试策略。
    pub fn with_config(
        name: impl Into<String>,
        backend: Arc<dyn NetworkBackend>,
        url_builder: UrlBuilder<K>,
        zoom_of: fn(&K) -> u32,
        base_guard: BaseLayerGuard,
        max_cache_entries: usize,
        config: PoolConfig,
    ) -> Self {
        // 恒等解码器：非空字节原样流式传输；空
        // 响应被归类为占位符（无可用的资产），匹配
        // 瓦片流水线的 `is_placeholder_tile` 约定。
        let decode: Decoder<Vec<u8>> = Arc::new(|d: &[u8]| {
            if d.is_empty() {
                None
            } else {
                Some(d.to_vec())
            }
        });

        let pool = WorkerPool::spawn(backend, decode, config);
        let inner = Arc::new(Inner {
            name: name.into(),
            cache: Mutex::new(GpuCache::new(max_cache_entries, base_guard, zoom_of)),
            hidden: Mutex::new(HiddenLru::new()),
            dedup: Dedup::new(),
            pool,
            waiters: Mutex::new(HashMap::new()),
            url_builder,
            intake: Mutex::new(()),
            stats: RbStats::new(),
            shutdown: AtomicBool::new(false),
            #[cfg(test)]
            test_panic: AtomicBool::new(false),
        });

        let disp = Arc::clone(&inner);
        let dispatcher = thread::spawn(move || run_dispatcher_guarded(disp));

        Self {
            inner,
            dispatcher: Some(dispatcher),
        }
    }

    // ── 固有的缓存层级管理（宿主 + 测试接口）──────────
    //
    // 这些逐字委托给复用的 `GpuCache` / `HiddenLru` / `Dedup`，
    // 以便驱逐/去重不变式与瓦片流水线保持完全一致。

    /// 用一个资产直接填充热缓存（例如本地解码或从祖先
    /// 重新挂接）。绕过网络路径。
    pub fn insert(&self, key: K, bytes: Vec<u8>) {
        self.inner.cache.lock().unwrap().insert(key, bytes);
    }

    /// 在热缓存上运行 FIFO 驱逐。三个不变式（基础层
    /// 豁免 / 活实体延迟 / 终止性）由 [`GpuCache`] 强制实施。
    /// `is_live` 对应 `mgr.tile_entities.contains_key(&old)`（L1487）。
    pub fn evict<F>(&self, is_live: F) -> EvictionResult
    where
        F: Fn(&K) -> bool,
    {
        let res = self.inner.cache.lock().unwrap().evict(is_live);
        self.inner.stats.evicted.fetch_add(res.evicted, Ordering::Relaxed);
        res
    }

    /// 将一个键标记为被活引用支撑（或不再支撑）（设置不变式 2）。
    pub fn set_live(&self, key: K, is_live: bool) {
        self.inner.cache.lock().unwrap().set_live(key, is_live);
    }

    /// 将一个已缓存的资产降级到温层（隐藏 LRU 回退）。
    pub fn hide(&self, key: K) {
        self.inner.hidden.lock().unwrap().hide(key);
    }

    /// 将一个温资产提升回热层。若它之前被隐藏则返回 true。
    pub fn show(&self, key: &K) -> bool {
        self.inner.hidden.lock().unwrap().show(key)
    }

    /// 推进温层 LRU 的帧 tick（每帧调用一次）。
    pub fn begin_frame(&self) {
        self.inner.hidden.lock().unwrap().advance_frame();
    }

    /// 读取已缓存的字节（热或温），若存在。
    pub fn cached_bytes(&self, key: &K) -> Option<Vec<u8>> {
        self.inner.cache.lock().unwrap().get(key).cloned()
    }

    /// 热/温缓存存储中的条目数。
    pub fn cache_len(&self) -> usize {
        self.inner.cache.lock().unwrap().len()
    }

    /// 若对 `key` 的流请求当前在途（去重集）则返回 true。
    pub fn is_in_flight(&self, key: &K) -> bool {
        self.inner.dedup.contains(key)
    }

    /// 热缓存 FIFO 顺序的只读视图（最旧在前）。照搬
    /// `GpuCache::order()`，用于不变式断言。
    pub fn evict_order_len(&self) -> usize {
        self.inner.cache.lock().unwrap().order().len()
    }

    /// 测试接缝（M2）：装备 dispatcher 在其下一批非空结果上 panic，
    /// 以演练 `catch_unwind` + 等待者排空恢复。
    #[cfg(test)]
    fn arm_dispatcher_panic(&self) {
        self.inner.test_panic.store(true, Ordering::Relaxed);
    }
}

impl<K> ResourceBackend<K> for PipelineResourceBackend<K>
where
    K: Hash + Eq + Copy + Send + 'static,
{
    /// 流式请求一个资源键的字节。
    ///
    /// 在 `intake` 锁下原子地依次：查热/温缓存命中→冷未命中时注册等待者并
    /// 经去重后提交到 worker 池→阻塞至 dispatcher 交付（仅 std mpsc）。
    fn request_stream<'a>(
        &'a self,
        key: K,
        priority: f64,
    ) -> Pin<Box<dyn Future<Output = PortResult<Vec<u8>>> + Send + 'a>> {
        let inner = Arc::clone(&self.inner);
        // 返回的 future 实际上是 `'static`（它拥有一个 `Arc<Inner>`），
        // 这平凡地满足了来自 `&'a self` 的 `'a` 约束。
        Box::pin(async move {
            // 原子冷承受：跨缓存未命中检查 AND
            // 去重标记 + 提交都持有 `intake`，因此 dispatcher 的完成（同样
            // 会取 `intake`）绝不会交错，并为同一个冷键造成冗余的
            // 第二次抓取。
            let intake = inner.intake.lock().unwrap();

            // 1. 缓存命中（热或温）→ 立即解析，提升温层。
            let cached = inner.cache.lock().unwrap().get(&key).cloned();
            if let Some(bytes) = cached {
                drop(intake);
                // 温 → 热提升（温资产仍被缓存）。
                inner.hidden.lock().unwrap().show(&key);
                return Ok(bytes);
            }

            // 2. 冷未命中 → 注册一个等待者、去重、提交到池。
            let (tx, rx) = mpsc::channel();
            let is_new = inner.dedup.insert(key);
            inner
                .waiters
                .lock()
                .unwrap()
                .entry(key)
                .or_default()
                .push(tx);
            if is_new {
                let url = (inner.url_builder)(&key);
                inner.pool.extend_wanted(&[key]);
                inner.pool.submit(Job { key, url, priority });
            } else {
                // 已在途：共享单次网络抓取。
                inner.stats.deduped.fetch_add(1, Ordering::Relaxed);
            }
            drop(intake);

            // 3. 阻塞调用线程直到 dispatcher 交付。
            //    仅 std（mpsc），无 tokio，无 executor。参见模块文档：
            //    从 IO/worker 上下文驱动，而非帧线程。
            match rx.recv() {
                Ok(res) => res,
                Err(_) => Err(PortError::Cancelled),
            }
        })
    }

    /// 取消一个在途键：清除去重槽、从 wanted 集摘除并以 `Cancelled`
    /// 立即唤醒所有等待者。
    fn cancel(&self, key: &K) {
        // 清除在途槽位，以便该资产可被重新请求。
        self.inner.dedup.remove(key);
        // 从池的 wanted 集中丢下该键，以便 worker 门控（L2161）
        // 产生一个 Aborted 结果而非抓取。
        self.inner.pool.remove_wanted(key);
        // 立即用 Cancelled 解除任何等待者的阻塞。
        if let Some(senders) = self.inner.waiters.lock().unwrap().remove(key) {
            for s in senders {
                let _ = s.send(Err(PortError::Cancelled));
            }
        }
    }

    /// 查询一个键当前所在的缓存层级（冷/温/热）。
    fn cache_tier(&self, key: &K) -> CacheTier {
        // 冷：完全不在缓存存储中。
        if !self.inner.cache.lock().unwrap().contains_key(key) {
            return CacheTier::Cold;
        }
        // 温：已缓存但被追踪为隐藏（LRU 回退）。
        if self.inner.hidden.lock().unwrap().is_hidden(key) {
            CacheTier::Warm
        } else {
            CacheTier::Hot
        }
    }

    /// 返回后端的运行时统计快照（热/温条目数、在途、streamed/deduped/evicted）。
    fn stats(&self) -> ResourceStats {
        let cache_len = self.inner.cache.lock().unwrap().len() as u32;
        let hidden_len = self.inner.hidden.lock().unwrap().len() as u32;
        // 温 = 仍被缓存的隐藏条目；热 = 其余部分。
        let warm_entries = hidden_len.min(cache_len);
        let hot_entries = cache_len.saturating_sub(warm_entries);
        ResourceStats {
            hot_entries,
            warm_entries,
            in_flight: self.inner.dedup.len() as u32,
            streamed: self.inner.stats.streamed.load(Ordering::Relaxed),
            deduped: self.inner.stats.deduped.load(Ordering::Relaxed),
            evicted: self.inner.stats.evicted.load(Ordering::Relaxed),
        }
    }

    /// 后端的可读名称标识。
    fn name(&self) -> &str {
        &self.inner.name
    }

    /// 后端是否可用：未收到关闭请求时即为可用。
    fn is_available(&self) -> bool {
        !self.inner.shutdown.load(Ordering::Relaxed)
    }
}

impl<K> Drop for PipelineResourceBackend<K>
where
    K: Hash + Eq + Copy + Send + 'static,
{
    /// 停止 dispatcher 线程并释放内部 `Arc`；丢弃 `WorkerPool` 会关闭其
    /// 任务通道，使 worker 自然退出。
    fn drop(&mut self) {
        // 停止 dispatcher，然后让 `Arc<Inner>` 解开：丢弃
        // `WorkerPool` 会关闭其任务通道，（分离的）worker 随之退出。
        self.inner.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.dispatcher.take() {
            let _ = handle.join();
        }
    }
}

/// 在 `catch_unwind` 下运行 [`dispatcher_loop`]，并在任意退出（panic 或
/// 正常 shutdown）时，排空 + 失败每一个仍注册的等待者。
///
/// M2 review fix：如果 dispatcher panic 了（例如一个被污染的锁或一个
/// 内部 bug），否则等待者的 `Sender` 将永远存活于
/// `Arc<Inner>` 内，使每一个在途的 `request_stream` 调用者
/// 阻塞在 `rx.recv()` 上，既无唤醒也无监督。捕获该
/// unwind 并显式地向每个等待者发送一个错误，可确定性地解除所有调用者的
/// 阻塞（`recv` 会产生错误，绝不会挂死）。panic 时
/// shutdown 标志也会被置位，以便 `is_available()` 报告 backend 不可用。
fn run_dispatcher_guarded<K>(inner: Arc<Inner<K>>)
where
    K: Hash + Eq + Copy + Send + 'static,
{
    let cleanup = Arc::clone(&inner);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        dispatcher_loop(inner)
    }))
    .is_err();

    if panicked {
        // 将 backend 标记为不可用，以便调用者检测到已死的 dispatcher
        // 而不是向它队列更多工作。
        cleanup.shutdown.store(true, Ordering::Relaxed);
    }

    // 排空每一个仍注册的等待者并使其失败，以便没有调用者在
    // dispatcher 退出后仍阻塞在 `rx.recv()` 上。通过 `into_inner`
    // 从一个可能被污染的 `waiters` 锁中恢复（panic 可能在 dispatcher
    // 持有它时发生）。
    let mut waiters = cleanup
        .waiters
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for (_key, senders) in waiters.drain() {
        for s in senders {
            let _ = s.send(Err(PortError::Network(
                "resource dispatcher exited before delivering this stream".into(),
            )));
        }
    }
}

/// 后台 dispatcher：排空工作线程池的结果、填充热缓存，
/// 并将每个结果路由到其注册的等待者。
///
/// 这是池结果队列的唯一消费者（无排空竞态）。
/// 空闲时每 1 ms 轮询一次，并在 `shutdown` 置位后立即退出。
fn dispatcher_loop<K>(inner: Arc<Inner<K>>)
where
    K: Hash + Eq + Copy + Send + 'static,
{
    loop {
        if inner.shutdown.load(Ordering::Relaxed) {
            break;
        }
        let results = inner.pool.drain_results(64);
        if results.is_empty() {
            thread::sleep(Duration::from_millis(1));
            continue;
        }
        // M2 测试接缝（仅 cfg(test)）：一旦拿到一个结果（因此已为其
        // 注册了一个等待者）就确定性地 panic dispatcher，以
        // 演练 `run_dispatcher_guarded` 中的 catch_unwind + 等待者排空
        // 恢复。
        #[cfg(test)]
        if inner.test_panic.load(Ordering::Relaxed) {
            panic!("test-injected dispatcher panic (M2 recovery path)");
        }
        for r in results {
            // 完成相对于冷承受是原子的：跨在途清除 + 缓存插入 + 等待者
            // 路由都持有 `intake`，因此同一键的并发
            // `request_stream` 绝不会在缓存写入与等待者排空之间的窗口内
            // 注册一个 NEW 等待者。
            //
            // H3 review fix：修复前 `intake` 守卫在 `waiters.remove` 之前就被
            // 释放，因此对于一个 Failed/Aborted 结果（缓存未被
            // 写入），在该窗口内进入的调用者会注册一个新鲜等待者，而此陈旧
            // 结果又会把它扫走——新调用者拿到了旧失败，而其自己提交任务的结果
            // 则被孤立。`mpsc::Sender::send` 是非阻塞且不获取任何锁，因此
            // 跨扇出持有 `intake` 是安全的。锁顺序保持
            // intake → {dedup, cache, waiters}；每次只持有一个内部锁
            // （每个都是语句结束时即释放的临时量）。
            let _intake = inner.intake.lock().unwrap();
            // 流已完成（或已中止）——释放 in-flight 槽位。
            inner.dedup.remove(&r.key);
            // 成功的流落入热缓存（复用的 GpuCache，因此
            // FIFO 顺序 / 基础层锁定 / 活实体延迟全部适用）。
            if let JobOutcome::Success(ref bytes) = r.outcome {
                inner.cache.lock().unwrap().insert(r.key, bytes.clone());
                inner.stats.streamed.fetch_add(1, Ordering::Relaxed);
            }
            // 将该结果路由到该键的每一个等待者——仍在
            // `intake` 之下，使 {dedup.remove, cache.insert, 等待者路由}
            // 对该键成为原子操作。
            if let Some(senders) = inner.waiters.lock().unwrap().remove(&r.key) {
                for s in senders {
                    let msg = match &r.outcome {
                        JobOutcome::Success(b) => Ok(b.clone()),
                        JobOutcome::Aborted => Err(PortError::Cancelled),
                        JobOutcome::Failed => {
                            Err(PortError::Network("resource stream retries exhausted".into()))
                        }
                        JobOutcome::Placeholder => {
                            Err(PortError::NotFound("no usable asset content".into()))
                        }
                    };
                    let _ = s.send(msg);
                }
            }
        }
    }
}

// `Pin` / `Future` 在模块顶部被导入。

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FetchResult;
    use std::sync::atomic::AtomicUsize;

    type TileKey = (u32, u32, u32);

    fn zoom_of(k: &TileKey) -> u32 {
        k.2
    }

    fn url_of(k: &TileKey) -> String {
        format!("http://assets/{}/{}/{}", k.2, k.0, k.1)
    }

    /// 即时 mock backend，按 URL 返回固定字节。
    struct MockNet {
        responses: Mutex<HashMap<String, FetchResult>>,
    }

    impl MockNet {
        fn new() -> Self {
            Self {
                responses: Mutex::new(HashMap::new()),
            }
        }
        fn add(&self, url: &str, r: FetchResult) {
            self.responses.lock().unwrap().insert(url.to_string(), r);
        }
    }

    impl NetworkBackend for MockNet {
        fn fetch(&self, url: &str) -> FetchResult {
            self.responses
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .unwrap_or(FetchResult::Transient("not mocked".into()))
        }
        fn name(&self) -> &str {
            "mock-resource"
        }
        fn timeout(&self) -> Duration {
            Duration::from_secs(1)
        }
    }

    fn fast_config(threads: usize) -> PoolConfig {
        PoolConfig {
            threads,
            max_attempts: 3,
            backoff_base: Duration::from_millis(5),
        }
    }

    fn make_backend(net: Arc<MockNet>, max_cache: usize) -> PipelineResourceBackend<TileKey> {
        PipelineResourceBackend::with_config(
            "test-resource-backend",
            net,
            Arc::new(url_of),
            zoom_of,
            BaseLayerGuard::new(),
            max_cache,
            fast_config(2),
        )
    }

    /// 以一个 `Waker::noop` 风格的单次轮询将装箱 future 驱动到完成
    /// （future 内部会阻塞，因此一次轮询总是产出 Ready）。
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        use std::task::{Context, Poll, Waker};
        let mut fut = Box::pin(fut);
        let mut cx = Context::from_waker(Waker::noop());
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(out) => out,
            Poll::Pending => panic!("resource future returned Pending (must block to Ready)"),
        }
    }

    #[test]
    fn cold_miss_streams_and_populates_hot_cache() {
        let net = Arc::new(MockNet::new());
        net.add("http://assets/5/1/2", FetchResult::Ok(vec![9, 8, 7]));
        let be = make_backend(net, 100);

        assert_eq!(be.cache_tier(&(1, 2, 5)), CacheTier::Cold);
        let bytes = block_on(be.request_stream((1, 2, 5), 1.0)).unwrap();
        assert_eq!(bytes, vec![9, 8, 7]);
        // dispatcher 将流式资产插入到了热缓存中。
        assert_eq!(be.cache_tier(&(1, 2, 5)), CacheTier::Hot);
        assert_eq!(be.cached_bytes(&(1, 2, 5)), Some(vec![9, 8, 7]));
        assert_eq!(be.stats().streamed, 1);
    }

    #[test]
    fn cache_hit_resolves_without_network() {
        let net = Arc::new(MockNet::new()); // 无响应→若命中会失败
        let be = make_backend(net, 100);
        be.insert((3, 3, 6), vec![1, 2, 3]);

        let bytes = block_on(be.request_stream((3, 3, 6), 1.0)).unwrap();
        assert_eq!(bytes, vec![1, 2, 3]);
        assert_eq!(be.cache_tier(&(3, 3, 6)), CacheTier::Hot);
    }

    #[test]
    fn warm_hit_promotes_to_hot() {
        let net = Arc::new(MockNet::new());
        let be = make_backend(net, 100);
        be.insert((4, 4, 7), vec![5, 5]);
        be.hide((4, 4, 7));
        assert_eq!(be.cache_tier(&(4, 4, 7)), CacheTier::Warm);

        let bytes = block_on(be.request_stream((4, 4, 7), 1.0)).unwrap();
        assert_eq!(bytes, vec![5, 5]);
        // 访问时温 → 热提升。
        assert_eq!(be.cache_tier(&(4, 4, 7)), CacheTier::Hot);
    }

    #[test]
    fn concurrent_cold_requests_dedup_to_one_fetch() {
        // 一个门控、计数的 backend 确定性地证明 `Dedup` 复用：
        // 抓取会阻塞直到被释放，因此所有 N 个并发冷请求都在单个
        // 任务仍在途时到达承受判定。
        // 恰好一个调用者赢得 `dedup.insert` 并提交；其余 N-1
        // 个作为等待者加入（由 `deduped` 计数）。因此恰好一次抓取。
        struct GatedNet {
            count: AtomicUsize,
            release: AtomicBool,
        }
        impl NetworkBackend for GatedNet {
            fn fetch(&self, _url: &str) -> FetchResult {
                self.count.fetch_add(1, Ordering::SeqCst);
                // 保持单个在途任务开着直到测试将其释放。
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

        let net = Arc::new(GatedNet {
            count: AtomicUsize::new(0),
            release: AtomicBool::new(false),
        });
        let be = Arc::new(PipelineResourceBackend::with_config(
            "dedup-backend",
            Arc::clone(&net) as Arc<dyn NetworkBackend>,
            Arc::new(url_of),
            zoom_of,
            BaseLayerGuard::new(),
            100,
            fast_config(4),
        ));

        let key: TileKey = (1, 1, 5);
        let mut handles = Vec::new();
        for _ in 0..8 {
            let be = Arc::clone(&be);
            handles.push(thread::spawn(move || block_on(be.request_stream(key, 1.0))));
        }
        // 让 8 个调用者都到达承受判定：一个提交 + 阻塞在
        // 门控抓取中，其余 7 个注册为去重的等待者。
        thread::sleep(Duration::from_millis(150));
        assert_eq!(net.count.load(Ordering::SeqCst), 1, "exactly one fetch in-flight");
        assert_eq!(be.stats().deduped, 7, "7 concurrent callers deduped onto the one fetch");

        // 释放门控；单个结果向所有 8 个等待者扇出。
        net.release.store(true, Ordering::SeqCst);
        for h in handles {
            let res = h.join().unwrap();
            assert_eq!(res.unwrap(), vec![42]);
        }
        assert_eq!(net.count.load(Ordering::SeqCst), 1, "dedup must collapse to one fetch");
        assert_eq!(be.cache_tier(&key), CacheTier::Hot);
    }

    #[test]
    fn cancel_unblocks_waiter_with_cancelled() {
        // 一个抓取总是瞬时失败的 backend 会让任务保持在途足够长，
        // 以便确定性地取消。
        struct SlowNet;
        impl NetworkBackend for SlowNet {
            fn fetch(&self, _url: &str) -> FetchResult {
                FetchResult::Transient("slow".into())
            }
            fn name(&self) -> &str {
                "slow"
            }
            fn timeout(&self) -> Duration {
                Duration::from_secs(1)
            }
        }
        let be = Arc::new(PipelineResourceBackend::with_config(
            "cancel-backend",
            Arc::new(SlowNet),
            Arc::new(url_of),
            zoom_of,
            BaseLayerGuard::new(),
            100,
            PoolConfig {
                threads: 1,
                max_attempts: 50,
                backoff_base: Duration::from_millis(20),
            },
        ));

        let key: TileKey = (2, 2, 8);
        let be2 = Arc::clone(&be);
        let handle = thread::spawn(move || block_on(be2.request_stream(key, 1.0)));
        // 给 worker 一些时间接手任务并开始重试。
        thread::sleep(Duration::from_millis(60));
        assert!(be.is_in_flight(&key));
        be.cancel(&key);
        let res = handle.join().unwrap();
        assert!(matches!(res, Err(PortError::Cancelled)));
        assert!(!be.is_in_flight(&key));
    }

    #[test]
    fn placeholder_maps_to_not_found() {
        let net = Arc::new(MockNet::new());
        net.add("http://assets/6/0/0", FetchResult::Ok(vec![])); // 空 → 占位符
        let be = make_backend(net, 100);
        let res = block_on(be.request_stream((0, 0, 6), 1.0));
        assert!(matches!(res, Err(PortError::NotFound(_))));
        // 占位符资产不会被缓存。
        assert_eq!(be.cache_tier(&(0, 0, 6)), CacheTier::Cold);
    }

    #[test]
    fn backend_is_dyn_compatible() {
        let net = Arc::new(MockNet::new());
        let be = make_backend(net, 100);
        let boxed: Box<dyn ResourceBackend<TileKey>> = Box::new(be);
        assert_eq!(boxed.name(), "test-resource-backend");
        assert!(boxed.is_available());
        assert_eq!(boxed.cache_tier(&(0, 0, 0)), CacheTier::Cold);
    }

    /// H3 review fix：dispatcher 在 intake 临界区内部路由等待者，
    /// 因此对一个先前流已 FAILED 的键的重新请求，绝不会
    /// 被陈旧的 Failed 结果扫走（修复前等待者路由
    /// 发生在 `intake` 被释放之后，打开了一个窗口：一个新注册的
    /// 等待者被递上旧失败，而其自己任务的结果则被孤立）。逐键：(1) 首次流确定性地
    /// 失败（transient + `max_attempts=1` → Failed）；(2) 翻转 backend 使其成功；
    /// (3) 重新请求同一个键并断言它收到自己的新鲜 Ok
    /// 字节，而非陈旧的 Failed。一个多键循环可抖出残余的竞态。
    #[test]
    fn failed_stream_then_rerequest_gets_fresh_result_not_stale_failure() {
        struct FlipNet {
            ok: AtomicBool,
        }
        impl NetworkBackend for FlipNet {
            fn fetch(&self, _url: &str) -> FetchResult {
                if self.ok.load(Ordering::SeqCst) {
                    FetchResult::Ok(vec![7, 7, 7])
                } else {
                    FetchResult::Transient("fail-first".into())
                }
            }
            fn name(&self) -> &str {
                "flip"
            }
            fn timeout(&self) -> Duration {
                Duration::from_secs(1)
            }
        }

        let net = Arc::new(FlipNet { ok: AtomicBool::new(false) });
        // max_attempts=1 → 单个 Transient 会变为 Failed 且无重试。
        let be = PipelineResourceBackend::with_config(
            "h3-backend",
            Arc::clone(&net) as Arc<dyn NetworkBackend>,
            Arc::new(url_of),
            zoom_of,
            BaseLayerGuard::new(),
            100,
            PoolConfig {
                threads: 2,
                max_attempts: 1,
                backoff_base: Duration::from_millis(1),
            },
        );

        for i in 0..20u32 {
            let key: TileKey = (i, i, 5);
            // 失败模式：对该键的首次请求必须暴露 Failed → Network。
            net.ok.store(false, Ordering::SeqCst);
            let r1 = block_on(be.request_stream(key, 1.0));
            assert!(
                matches!(r1, Err(PortError::Network(_))),
                "first stream must fail (transient, max_attempts=1); got {r1:?}"
            );
            // 成功模式：重新请求同一个键 → 必须得到自己的新鲜
            // 结果，绝非来自上一个任务的陈旧 Failed。
            net.ok.store(true, Ordering::SeqCst);
            let r2 = block_on(be.request_stream(key, 1.0));
            assert_eq!(
                r2.unwrap(),
                vec![7, 7, 7],
                "re-request after a failure must get its own fresh result, not the stale Failed"
            );
        }
    }

    /// M2 review fix：如果 dispatcher 线程死亡（panic），每一个在途的
    /// `request_stream` 调用者都必须用一个 Err 解除阻塞——绝不会
    /// 永远阻塞在 `rx.recv()` 上。我们装备仅测试的 panic 接缝，然后
    /// 发出一个冷请求；一旦它的结果到达 dispatcher 就会 panic，
    /// `run_dispatcher_guarded` 捕获该 unwind 并用一个 Network 错误排空
    /// 等待者，并将 backend 标记为不可用。
    #[test]
    fn dispatcher_panic_unblocks_waiters_with_error() {
        let net = Arc::new(MockNet::new());
        net.add("http://assets/7/3/3", FetchResult::Ok(vec![1, 2, 3]));
        let be = Arc::new(make_backend(net, 100));
        be.arm_dispatcher_panic();

        let key: TileKey = (3, 3, 7);
        let be2 = Arc::clone(&be);
        // 在一个线程上运行：有修复时调用者会被及时解除阻塞；
        // 一个回归会在此挂死（表现为卡住的测试）。
        let handle = thread::spawn(move || block_on(be2.request_stream(key, 1.0)));
        let res = handle.join().unwrap();
        assert!(
            matches!(res, Err(PortError::Network(_))),
            "dispatcher death must surface as an Err, not a permanent block; got {res:?}"
        );
        // backend 现在报告不可用（dispatcher panic 时 shutdown 被置位）。
        assert!(!be.is_available(), "dead dispatcher must mark the backend unavailable");
    }
}
