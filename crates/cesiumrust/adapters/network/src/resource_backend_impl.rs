//! M8.3 —— 网络 `ResourceBackend` 适配器（adapters/network）。
//!
//! 本模块承担网络适配器的 M8.3 结构性重构：
//!
//! 1. [`resource_fetch_backend_enabled`] —— 针对
//!    `CESIUM_ENABLE_RESOURCE_FETCH_BACKEND` 的一个**本地**环境门控读取器。它
//!    照搬了 M9.2 `ENV_ENABLE_GLTF_UPGRADE` 的先例（
//!    `adapters/bevy-render/src/tileset/content_loader.rs:489-496`：适配器
//!    局部读取；**不**依赖 `cesium-app` 的 `feature_flags` 注册表，
//!    后者正由并行里程碑共同编辑）。该名字**有意**区别于
//!    `ENV_ENABLE_RESOURCE_BACKEND`（M12）—— 参见
//!    `ports/driven/src/resource.rs:69-85` 中的 M8-against-M12 命名隔离裁定。
//! 2. [`spawn_blocking_fetch`] / [`block_on_noop`] —— M8.3 清除 tokio 后，
//!    [`crate::HttpTileFetcher::fetch`] 使用的**唯一**两个无 tokio 的
//!    async 边界辅助函数。两者都仅用 `std`（`std::thread` +
//!    `std::sync::mpsc` + `Waker::noop`），与
//!    `PipelineResourceBackend::request_stream` 的“同步嵌异步”模式相配
//!    （`adapters/pipeline/src/resource_backend.rs:296-343`）。
//! 3. [`NetworkResourceBackend`] —— 面向网络流式 bulk 资产的
//!    [`ResourceBackend<u64>`] 实现。它是 `cesium-pipeline` 中
//!    [`PipelineResourceBackend<u64>`] 的一个**薄包裹**，因此 16 工作线程
//!    keep-alive [`WorkerPool`](cesium_pipeline::pool::WorkerPool) +
//!    [`UreqBackend`](cesium_pipeline::net::ureq_backend::UreqBackend) +
//!    热/温缓存层级 + 在途去重都被**逐字复用** —— 无并行池，无重复缓存，
//!    无新 HTTP 客户端。该包裹只额外添加一个 URL 注册表，以便调用方按 URL
//!    字符串为请求键控（经 [`url_hash`] 哈希为 `u64`），同时满足 pipeline 的
//!    `K: Copy` 约束。
//!
//! # 已遵守的硬约束
//!
//! * **无 tokio Runtime，无来自执行器的 `block_on`。** 所有阻塞都在
//!   `std::thread` + `std::sync::mpsc` 上进行；返回的 future 在首次 poll 时
//!   即 `Ready`（`Waker::noop()` 足以驱动它们）。
//! * **无并行池。** 重活（重试 + keep-alive + 去重 + 热/温缓存）逐字
//!   委派给 `PipelineResourceBackend`，因此保护 `dynamic_globe` 黄金路径
//!   免遭花屏的三个驱逐不变式继续生效。
//! * **门控 OFF = 与 M8.3 前逐字节一致。** 除非 `CESIUM_ENABLE_RESOURCE_FETCH_BACKEND`
//!   为真值，`resource_fetch_backend_enabled` 返回 `false`，而
//!   `HttpTileFetcher::fetch` 仅当其为真时才走 new-backend 分支。
//!   默认（未设置）⇒ 与 M8.3 前的 `HttpTileFetcher` 一致的限速 + 重试 +
//!   取消语义，因此 v0 的 8 图基线保持零差异。
//! * **domain/resource 保持无网络。** 本适配器消费 M8.1 的领域类型
//!   （`FetchDescriptor`、`RetryPolicy`、`classify_status`），但绝不将
//!   ureq / reqwest / tokio 再导出回领域层。

use std::collections::HashMap;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread;

use cesium_pipeline::net::ureq_backend::UreqBackend;
use cesium_pipeline::net::NetworkBackend;
use cesium_pipeline::resource_backend::PipelineResourceBackend;
use cesium_pipeline::runtime::UrlBuilder;
use cesium_ports_driven::{CacheTier, PortResult, ResourceBackend, ResourceStats};

// ─────────────────────────────────────────────────────────────────────────────
// 环境门控（M8.3）—— 本地读取，不依赖 `feature_flags.rs`
// ─────────────────────────────────────────────────────────────────────────────

/// M8.3 网络-resource-backend 门控的环境变量名。
///
/// **有意**区别于 `ENV_ENABLE_RESOURCE_BACKEND`（M12 —— CesiumJS 的
/// `Resource` 对象抽象：URL 模板、查询参数、重试头）。两个标志语义上不相关，
/// 绝不能相互接线；参见 `ports/driven/src/resource.rs:69-85` 中的
/// M8-against-M12 命名隔离裁定。
pub const ENV_ENABLE_RESOURCE_FETCH_BACKEND: &str = "CESIUM_ENABLE_RESOURCE_FETCH_BACKEND";

/// 真值-token 谓词 —— 与 `pipeline::fetch::truthy`
/// （`adapters/bevy-render/src/pipeline/fetch.rs:75`）以及 `cesium-app` 中的
/// `feature_flags::truthy` **逐字节一致**。接受（大小写不敏感，去除首尾
/// 空白）：`1`、`true`、`yes`、`on`。其余一切（包括未设置）都是 `false`。
fn truthy(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// 从原始环境变量值（`None` = 未设置）进行纯门控计算。从
/// [`resource_fetch_backend_enabled`] 拆出，以便真值表可在**不**变更进程
/// 全局环境（那会与并行测试产生竞态）的情况下进行单元测试。
/// 照搬 bevy-render 中的 `pipeline::fetch::gate_from_env_value`。
pub fn gate_from_env_value(raw: Option<String>) -> bool {
    match raw {
        Some(v) => truthy(&v),
        None => false,
    }
}

/// 本地读取 M8.3 门控，照搬 M9.2 `ENV_ENABLE_GLTF_UPGRADE` 的先例
/// （`adapters/bevy-render/src/tileset/content_loader.rs:489-496`）。
///
/// 默认 OFF，因此除非 `CESIUM_ENABLE_RESOURCE_FETCH_BACKEND` 显式为真，
/// M8.3 前的 [`crate::HttpTileFetcher`] 路径保持逐字节一致。**不**查询
/// `cesium-app::feature_flags`（那个注册表由并行里程碑共同编辑；适配器
/// 局部读取可避免冲突）。
#[inline]
pub fn resource_fetch_backend_enabled() -> bool {
    gate_from_env_value(std::env::var(ENV_ENABLE_RESOURCE_FETCH_BACKEND).ok())
}

// ─────────────────────────────────────────────────────────────────────────────
// URL 哈希 —— 将 `String` URL 桥接到 pipeline 的 `K: Copy` 契约
// ─────────────────────────────────────────────────────────────────────────────

/// 使用 [`std::collections::hash_map::DefaultHasher`] 将一个 URL 哈希为稳定的
/// `u64` 键（SipHash-1-3，带每进程种子 —— 在进程内稳定，这正是 pipeline
/// 缓存所需的作域）。
///
/// 由 [`NetworkResourceBackend`] 用于按 URL 为 pipeline 缓存层级键控，而无须
/// 持有 `String` 键（pipeline 的 `K: Copy` 约束排除了 `String`）。哈希碰撞由
/// URL 注册表处理：一个碰撞的键会解析到最后被插入的那个 URL，因此依赖长期
/// 存活缓存的调用方应优先使用内容哈希键（例如将瓦片 quadkey 打包进 `u64`）
/// 而非 URL 哈希。
pub fn url_hash(url: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    h.finish()
}

// ─────────────────────────────────────────────────────────────────────────────
// spawn_blocking_fetch / block_on_noop —— 无 tokio 的 Future 桥接
// ─────────────────────────────────────────────────────────────────────────────

/// 在一个专用的 `std::thread` 上运行 `work`，并返回一个 `Send` future，
/// 它在首次 poll 时以该闭包的返回值 `Ready`。
///
/// 这是 M8.3 清除 tokio 后 [`crate::HttpTileFetcher::fetch`] 使用的**唯一**
/// async 边界辅助函数。它匹配 `PipelineResourceBackend::request_stream`
/// 使用的“同步嵌异步”模式（`adapters/pipeline/src/resource_backend.rs:296-343`）：
/// 返回的 future 在 `mpsc::recv()` 上阻塞其轮询线程且从不产出 `Pending`，
/// 因此一个 `Waker::noop()` 的单 poll 驱动器（见 [`block_on_noop`]）就足够
/// —— 无执行器，无 tokio Runtime。
///
/// 调用方**必须**从 IO/工作线程上下文驱动该 future（绝不能是帧线程），
/// 与 pipeline 的阻塞池理念一致。
pub fn spawn_blocking_fetch<F, T>(work: F) -> Pin<Box<dyn Future<Output = T> + Send>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = mpsc::channel::<T>();
    // 分离的 std::thread —— 无 tokio，无 JoinHandle 追踪。若调用方在完成前
    // 丢弃了 future，发送端的 `send` 会返回 Err 并被静默忽略（工作线程仍
    // 会运行到完成，与 `tokio::task::spawn_blocking` 的丢弃语义保持一致）。
    thread::spawn(move || {
        let out = work();
        let _ = tx.send(out);
    });
    Box::pin(BlockedOnRecv { rx: Some(rx) })
}

/// 在 `mpsc::recv()` 上阻塞轮询线程、并在首次 poll 返回 `Ready` 的
/// future。仅当工作线程未发送就丢弃了发送端时才会 panic（这需要闭包
/// 内部出现 `std::mem::forget` 类的误用 —— 视为 bug，而非可恢复状态）。
struct BlockedOnRecv<T> {
    /// 待接收的工作线程回传值接收端；首次 poll 时被 `take` 消费。
    rx: Option<mpsc::Receiver<T>>,
}

impl<T: Send + 'static> Future for BlockedOnRecv<T> {
    type Output = T;
    /// 阻塞式单次 poll：`take` 出接收端并在 `recv()` 上同步阻塞直至
    /// 工作线程回传结果，因此首次 poll 即返回 `Ready`。
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<T> {
        let rx = self
            .rx
            .take()
            .expect("BlockedOnRecv polled after completion (single-poll future)");
        match rx.recv() {
            Ok(v) => Poll::Ready(v),
            Err(_) => panic!("spawn_blocking_fetch worker dropped sender without a result"),
        }
    }
}

/// 使用 [`Waker::noop`] + 单次 poll 将一个 boxed future 驱动到完成。
///
/// 仅适用于那些内部阻塞、从不产出 `Pending` 的 future
/// （[`spawn_blocking_fetch`]、`PipelineResourceBackend::request_stream`）。这
/// **不是**一个通用执行器 —— 对于会产出 Pending 的 future，请使用真实
/// 运行时（本代码库有意在帧线程上避免运行时）。
pub fn block_on_noop<F>(fut: F) -> F::Output
where
    F: Future,
{
    let mut fut = Box::pin(fut);
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => {
            panic!("block_on_noop: future returned Pending (must block internally to Ready)")
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NetworkResourceBackend —— M8.3 的网络化 ResourceBackend
// ─────────────────────────────────────────────────────────────────────────────

/// 在后端句柄与交给 [`PipelineResourceBackend`] 的 `url_builder` 闭包之间
/// 共享的 URL 注册表。每次 [`NetworkResourceBackend::fetch_url_blocking`]
/// 调用时都会插入，以便 pipeline 的 `url_builder: fn(&u64) -> String` 能
/// 将键解析回 URL。
type UrlRegistry = Arc<Mutex<HashMap<u64, String>>>;

/// 面向 bulk 资产流式传输的网络化 [`ResourceBackend`]（M8.3）。
///
/// [`PipelineResourceBackend<u64>`] 的薄包裹，它用一个 `u64` URL 哈希
/// （[`url_hash`]）为请求键控，并管理一个 URL 注册表，以便 pipeline 的
/// `url_builder: fn(&K) -> String` 闭包能将键解析回 URL。所有重活 —— 16
/// 工作线程 keep-alive 池、重试、热/温缓存层级、在途去重，以及三个驱逐
/// 不变式 —— 都逐字委派给 pipeline 后端。无并行池，无重复缓存。
///
/// # 具体实例化
///
/// 关于 `K = u64`（URL 哈希）单态化。需要不同键类型的调用方应（a）在调用
/// 点哈希为 `u64`，或（b）用自己的 `url_builder` 直接构造
/// [`PipelineResourceBackend<K>`] —— 该包裹只为将 URL 字符串获取桥接到
/// `K: Copy` 的 pipeline 契约而存在。
///
/// # Dyn 兼容性
///
/// 实现 [`ResourceBackend<u64>`]，因此可作为 `Box<dyn ResourceBackend<u64>>`
/// 持有，与 pipeline 自己的 `PipelineResourceBackend<TileKey>`（使用
/// `(u32, u32, u32)` 键）并存。
pub struct NetworkResourceBackend {
    /// 包装的 pipeline 资源后端，提供热/冷缓存层级与在途去重。
    inner: PipelineResourceBackend<u64>,
    /// 数值 `u64` 资源键 ↔ 实际 URL 的双向注册表。
    registry: UrlRegistry,
    /// 累计发起的获取次数（单调递增，供统计）。
    fetch_count: AtomicU64,
    /// 后端可用性标志，供 [`ResourceBackend::is_available`] 读取。
    available: AtomicBool,
}

impl NetworkResourceBackend {
    /// 使用默认的 ureq 网络层（16 工作线程、10 s 超时、keep-alive 池化 ——
    /// 见 [`UreqBackend::new`]）以及 pipeline 的黄金路径缓存/池配置
    /// （3000 条目热缓存、基础层 zoom 3、双时间尺度重试：3 次尝试、
    /// 250 ms 基础退避）创建一个后端。
    pub fn new() -> Self {
        Self::with_name("cesium-network-resource")
    }

    /// 使用自定义的诊断名（通过 [`ResourceBackend::name`] 展现）创建一个后端。
    pub fn with_name(name: &str) -> Self {
        let registry: UrlRegistry = Arc::new(Mutex::new(HashMap::new()));
        let reg_builder = Arc::clone(&registry);
        let url_builder: UrlBuilder<u64> = Arc::new(move |k: &u64| {
            reg_builder
                .lock()
                .unwrap()
                .get(k)
                .cloned()
                .unwrap_or_default()
        });
        let net: Arc<dyn NetworkBackend> = Arc::new(UreqBackend::new());
        // zoom-of 对 URL-哈希键并不使用（无 LOD 金字塔）；返回 0 以保持
        // pipeline 的 `BaseLayerGuard` 不变式被平凡地满足 —— 没有任何键会
        // 被视为受保护的基础层瓦片，这对通用 bulk 资产流式传输是正确的
        // （基础层豁免适用于瓦片金字塔，而非 URL-键控资产）。
        let inner = PipelineResourceBackend::new(name, net, url_builder, |_k: &u64| 0);
        Self {
            inner,
            registry,
            fetch_count: AtomicU64::new(0),
            available: AtomicBool::new(true),
        }
    }

    /// 在稳定的哈希键下注册 `url`，并通过 pipeline 管理的缓存层级流式传输
    /// 资产字节。**阻塞** —— 从 IO/工作线程调用，绝不能是帧线程。
    ///
    /// 缓存命中时立即完成（无网络往返）。冷 miss 时提交到 16 工作线程
    /// keep-alive 池，并在调度器的完成信号上阻塞（经
    /// [`PipelineResourceBackend::request_stream`] → `mpsc::recv`）。
    ///
    /// 使用相同 URL 的并发调用会经由 pipeline 的 `Dedup` 集合收敛为单次
    /// 网络获取（不变式测试参见
    /// `adapters/pipeline/src/resource_backend.rs:592-654`）。
    pub fn fetch_url_blocking(&self, url: &str, priority: f64) -> PortResult<Vec<u8>> {
        let key = url_hash(url);
        // 在哈希键下注册 URL（对重复获取是幂等的；哈希碰撞会覆写，
        // 但 SipHash-1-3 在真实 URL 集上的碰撞几乎不可能发生）。
        self.registry.lock().unwrap().insert(key, url.to_string());
        self.fetch_count.fetch_add(1, Ordering::Relaxed);
        // `request_stream` 内部阻塞（在调度器上 mpsc::recv）；
        // 一个 `Waker::noop()` 的单 poll 驱动器就足够。
        block_on_noop(self.inner.request_stream(key, priority))
    }

    /// [`Self::fetch_url_blocking`] 调用的累计次数
    /// （诊断用 —— 同等统计冷 miss + 缓存命中 + 去重共享）。
    pub fn fetch_count(&self) -> u64 {
        self.fetch_count.load(Ordering::Relaxed)
    }

    /// 直接访问底层 pipeline 后端（用于缓存层级管理 —— `insert` /
    /// `evict` / `hide` / `show` / `cached_bytes`）。
    pub fn inner(&self) -> &PipelineResourceBackend<u64> {
        &self.inner
    }

    /// 将后端标记为不可用（后续的 [`ResourceBackend::is_available`]
    /// 返回 `false`）。由优雅关闭路径使用；底层 pipeline 后端会继续排空
    /// 在途工作。
    pub fn mark_unavailable(&self) {
        self.available.store(false, Ordering::Relaxed);
    }
}

impl Default for NetworkResourceBackend {
    /// 默认构造（等价于 [`NetworkResourceBackend::new`]）。
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceBackend<u64> for NetworkResourceBackend {
    /// 请求一个资源键的字节流，委托给内部 pipeline 后端（含缓存与去重）。
    fn request_stream<'a>(
        &'a self,
        key: u64,
        priority: f64,
    ) -> Pin<Box<dyn Future<Output = PortResult<Vec<u8>>> + Send + 'a>> {
        self.inner.request_stream(key, priority)
    }

    /// 取消一个在途请求，按数值键委托给内部后端。
    fn cancel(&self, key: &u64) {
        self.inner.cancel(key);
    }

    /// 查询一个键当前所在的缓存层级（热/冷/未缓存）。
    fn cache_tier(&self, key: &u64) -> CacheTier {
        self.inner.cache_tier(key)
    }

    /// 返回后端的运行时统计快照。
    fn stats(&self) -> ResourceStats {
        self.inner.stats()
    }

    /// 返回后端的可读名称标识。
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// 后端是否可用：本地 `available` 标志为真且内部后端亦可用。
    fn is_available(&self) -> bool {
        self.available.load(Ordering::Relaxed) && self.inner.is_available()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 测试
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // --- gate_from_env_value 真值表（纯函数，无环境变更）-------------

    #[test]
    fn gate_unset_is_off() {
        assert!(!gate_from_env_value(None));
    }

    #[test]
    fn gate_truthy_tokens_are_on() {
        for t in ["1", "true", "TRUE", "True", "yes", "YES", "on", "ON", " 1 ", "\ttrue\n"] {
            assert!(
                gate_from_env_value(Some(t.to_string())),
                "{t:?} must be truthy"
            );
        }
    }

    #[test]
    fn gate_falsy_tokens_are_off() {
        for t in ["0", "false", "no", "off", "", "  ", "maybe", "2", "-1"] {
            assert!(
                !gate_from_env_value(Some(t.to_string())),
                "{t:?} must be falsy"
            );
        }
    }

    // --- url_hash 稳定性 ---------------------------------------------------

    #[test]
    fn url_hash_is_stable_within_process() {
        let a = url_hash("https://tiles.example.com/1/2/3.b3dm");
        let b = url_hash("https://tiles.example.com/1/2/3.b3dm");
        assert_eq!(a, b);
    }

    #[test]
    fn url_hash_distinguishes_distinct_urls() {
        let a = url_hash("https://tiles.example.com/1/2/3.b3dm");
        let b = url_hash("https://tiles.example.com/1/2/4.b3dm");
        assert_ne!(a, b);
    }

    // --- block_on_noop / spawn_blocking_fetch ---------------------------------

    #[test]
    fn spawn_blocking_fetch_returns_ready_on_first_poll() {
        let fut = spawn_blocking_fetch(|| 42u32);
        let v = block_on_noop(fut);
        assert_eq!(v, 42);
    }

    #[test]
    fn spawn_blocking_fetch_propagates_closure_result() {
        let fut = spawn_blocking_fetch(|| Ok::<_, ()>(vec![1u8, 2, 3]));
        let v = block_on_noop(fut).unwrap();
        assert_eq!(v, vec![1, 2, 3]);
    }

    #[test]
    fn spawn_blocking_fetch_runs_on_separate_thread() {
        let main_id = thread::current().id();
        let fut = spawn_blocking_fetch(move || {
            let worker_id = thread::current().id();
            (worker_id, worker_id != main_id)
        });
        let (_id, differs) = block_on_noop(fut);
        assert!(differs, "worker must run on a distinct std::thread");
    }

    // --- NetworkResourceBackend 缓存层级 -------------------------------
    //
    // 这些测试练习包裹器的缓存命中路径（它无需任何网络即同步
    // 完成），因此保持封闭且离线安全。冷 miss + wiremock 驱动的网络
    // 路径由 `specs/tests/e2e_network/*` 中的 e2e_network 骨架覆盖（deferred #37，
    // 在 M11.1 接入 async harness 之前都挂在 `#[ignore]` 后）。

    #[test]
    fn backend_reports_name_and_availability() {
        let be = NetworkResourceBackend::with_name("unit-test-backend");
        assert_eq!(be.name(), "unit-test-backend");
        assert!(be.is_available());
        be.mark_unavailable();
        assert!(!be.is_available());
    }

    #[test]
    fn backend_starts_with_empty_cache() {
        let be = NetworkResourceBackend::new();
        let stats = be.stats();
        assert_eq!(stats.hot_entries, 0);
        assert_eq!(stats.warm_entries, 0);
        assert_eq!(stats.in_flight, 0);
        assert_eq!(stats.streamed, 0);
        assert_eq!(be.fetch_count(), 0);
    }

    #[test]
    fn backend_cache_hit_resolves_without_network() {
        let be = NetworkResourceBackend::new();
        let key = url_hash("https://cached.example/asset.bin");
        // 直接经内部 pipeline 后端预热热缓存，然后断言
        // `request_stream` 从缓存完成（无需 wiremock）。
        be.inner().insert(key, vec![9, 8, 7]);
        assert_eq!(be.cache_tier(&key), CacheTier::Hot);

        let bytes = block_on_noop(be.request_stream(key, 1.0)).unwrap();
        assert_eq!(bytes, vec![9, 8, 7]);
    }

    #[test]
    fn backend_is_dyn_compatible() {
        let be = NetworkResourceBackend::new();
        let boxed: Box<dyn ResourceBackend<u64>> = Box::new(be);
        assert_eq!(boxed.name(), "cesium-network-resource");
        assert!(boxed.is_available());
        let key = url_hash("https://dyn.example/x");
        assert_eq!(boxed.cache_tier(&key), CacheTier::Cold);
    }

    #[test]
    fn backend_cancel_on_unknown_key_is_noop() {
        let be = NetworkResourceBackend::new();
        let key = url_hash("https://never-requested.example/x");
        // 绝不能 panic；底层 pipeline 后端将未知键的 cancel 视为空操作
        // （dedup.remove + pool.remove_wanted 都是幂等的）。
        be.cancel(&key);
        assert_eq!(be.cache_tier(&key), CacheTier::Cold);
    }

    // --- 环境门控默认值（未设置）---------------------------------------------

    #[test]
    fn resource_fetch_backend_enabled_defaults_off_when_unset() {
        // 健全性检查：测试环境本身必须没有设置
        // CESIUM_ENABLE_RESOURCE_FETCH_BACKEND（否则工作区中其他所有触及
        // 该门控的测试都会改变行为）。
        //
        // 我们这里**不**取消它（那会与并行测试产生竞态）；只断言观察到的
        // 默认值。CI / 本地运行必须为该黄金路径保持此变量未设置。
        if std::env::var(ENV_ENABLE_RESOURCE_FETCH_BACKEND).is_err() {
            assert!(!resource_fetch_backend_enabled());
        }
    }
}
