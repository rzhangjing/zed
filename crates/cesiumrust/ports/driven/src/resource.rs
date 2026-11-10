//! M8 / P2.3 — `ResourceBackend` 端口：经由管线托管的缓存层级
//! 进行批量资源流式加载。
//!
//! 由 M1.1 占位实现（仅 `name()` / `is_available()`）具体化
//! 为下方的真实流式加载契约，遵循 M1.1 的 dyn 兼容性
//! 裁决（PIPELINE_PROMOTION_PLAN.md L95）：异步方法使用
//! `Pin<Box<dyn Future + Send>>`，无泛型方法，无 `Self: Sized` 约束。
//! 该 trait 对资产 key `K` 泛型，与 `TilePipeline<K, Payload>` 完全一致 ——
//! 一旦 `K` 具体化即为 dyn 兼容。
//!
//! ports 层保持**运行时无关**：无 tokio，无 executor，无 glam。
//! 缓存层级语义（Hot/Warm/Cold）在此定义；对 `GpuCache` / `HiddenLru` /
//! `Dedup` 的具体复用则位于 `adapters/pipeline`。

use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;

use crate::PortResult;

/// 批量资产在管线托管的缓存层级中的位置。
///
/// 这三个层级镜像 M1.2 `cesium-pipeline` 的缓存结构，adapter
/// **必须复用**它们（不得另建平行的缓存系统 —— 那会破坏
/// 保护 `dynamic_globe` 黄金路径免于花屏 / 空帧的三条驱逐不变量）：
///
/// - [`CacheTier::Hot`] —— 常驻 GPU 句柄缓存（`GpuCache`）且
///   正被活跃引用；立即可用，无需往返。
/// - [`CacheTier::Warm`] —— 常驻 `GpuCache` 但由隐藏 LRU
///   （`HiddenLru`）追踪：一种跨可见性变化保留的暖回退，
///   无需网络抓取即可重新激活，超预算时按 LRU 优先驱逐。
/// - [`CacheTier::Cold`] —— 任何地方都未缓存；使用前必须
///   从网络后端流式加载。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheTier {
    /// 常驻热 GPU 缓存且正被活跃引用。
    Hot,
    /// 常驻缓存但处于隐藏状态（暖 LRU 回退）。
    Warm,
    /// 未缓存；必须从网络流式加载。
    Cold,
}

/// [`ResourceBackend`] 流式缓存层级的统计快照。
///
/// 字段语义有意与 [`crate::PipelineStats`]（瓦片管线快照）对齐，
/// 使宿主可以将资源流式计数器折叠进同一套 M0.4 `PerfCounters`
/// 观测路径。
#[derive(Debug, Clone, Default)]
pub struct ResourceStats {
    /// 常驻热层的资产（`GpuCache`，正被活跃引用）。
    pub hot_entries: u32,
    /// 常驻暖层的资产（`GpuCache` + `HiddenLru`）。
    pub warm_entries: u32,
    /// 有传输中流式请求的资产（`Dedup` 集合大小）。
    pub in_flight: u32,
    /// 累计成功流式加载进缓存的资产。
    pub streamed: u32,
    /// 累计被去重的冷请求（已在传输中或已缓存）。
    pub deduped: u32,
    /// 累计热层驱逐（FIFO，遵守三条不变量）。
    pub evicted: u32,
}

/// 批量资产流式加载后端（纹理 / mesh / 3D Tiles 内容），经由
/// 管线托管的缓存层级。
///
/// # 命名澄清（M8 vs M12 —— 切勿混淆）
///
/// 该 trait 与 `feature_flags.rs:ENV_ENABLE_RESOURCE_BACKEND`
/// 共享一个名称前缀（这是针对 CesiumJS 风格 `Resource` 对象的
/// **M12** 插件门），但两者**语义上毫不相关**，绝不可互相接线：
///
/// - **本 `ResourceBackend` trait（M8 / P2.3）**：批量资产流式加载
///   （纹理、mesh、3D Tiles 内容）通过管线托管的缓存层级
///   （`GpuCache` 热层 + `HiddenLru` 暖层 + `Dedup` 传输中），
///   复用 M1.2 `cesium-pipeline` 的驱逐/去重语义。
///   它是一个 **IO/缓存层**契约 —— 无坐标运算，无 glam。
/// - **`ENV_ENABLE_RESOURCE_BACKEND`（M12）**：切换 `Resource` 对象
///   抽象（URL 模板、查询参数、重试 header），用于
///   tileset/imagery provider 的*配置*（`domain/resource`）。
///
/// M8 **不得**复用 `ENV_ENABLE_RESOURCE_BACKEND` flag，且 M12 的
/// `Resource` 抽象也不得通过本 trait 实现。
///
/// # Dyn 兼容性
///
/// 对资产 key `K` 泛型（镜像 `TilePipeline<K, Payload>`）；
/// 一旦 `K` 具体化即为 dyn 兼容。异步方法返回
/// `Pin<Box<dyn Future + Send>>`（与 [`crate::TileFetcher`] 风格一致）。
/// 无泛型方法，无 `Self: Sized` 约束。
pub trait ResourceBackend<K>: Send + Sync
where
    K: Hash + Eq + Copy + Send + 'static,
{
    /// 为 `key` 标识的批量资产发起一个流式加载请求。
    ///
    /// 当后端队列很深时 `priority` 决定抓取顺序（越大 = 越早），
    /// 与 [`crate::TilePipeline::submit`] 一致。返回的 future
    /// 携流式加载的资产字节 resolve：
    ///
    /// - **缓存命中**（[`CacheTier::Hot`] / [`CacheTier::Warm`]）：立即
    ///   以缓存字节 resolve（暖命中会被提升为热）。
    /// - **缓存未命中**（[`CacheTier::Cold`]）：请求通过传输中集合
    ///   去重，并经阻塞 worker 池流式加载；资产落入缓存后 future
    ///   才 resolve。对同一冷 key 的并发请求共享一次网络抓取。
    ///
    /// 错误映射到 [`PortError`]：`Cancelled`（传输途中中止）、
    /// `Network`（重试耗尽）、`NotFound`（无可用资产）。
    fn request_stream<'a>(
        &'a self,
        key: K,
        priority: f64,
    ) -> Pin<Box<dyn Future<Output = PortResult<Vec<u8>>> + Send + 'a>>;

    /// 取消 `key` 的挂起 / 传输中流。
    ///
    /// 将 key 从 wanted 集合移除，使 worker 门控产生一个中止
    /// 结果，并清除传输中去重条目，使该资产稍后可被
    /// 重新请求。镜像 [`crate::TilePipeline::cancel`]。
    fn cancel(&self, key: &K);

    /// 报告缓存层级当前将 `key` 置于哪一层。
    fn cache_tier(&self, key: &K) -> CacheTier;

    /// 快照流式加载 / 缓存层级统计。
    fn stats(&self) -> ResourceStats;

    /// 返回此后端的人类可读名称（诊断用）。
    fn name(&self) -> &str;

    /// 若此后端当前可运行则返回 true。
    fn is_available(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仅用于证明该 trait 是对象安全的最小具体实现。
    struct DummyResourceBackend;

    type DummyKey = (u32, u32, u32);

    impl ResourceBackend<DummyKey> for DummyResourceBackend {
        fn request_stream<'a>(
            &'a self,
            _key: DummyKey,
            _priority: f64,
        ) -> Pin<Box<dyn Future<Output = PortResult<Vec<u8>>> + Send + 'a>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn cancel(&self, _key: &DummyKey) {}
        fn cache_tier(&self, _key: &DummyKey) -> CacheTier {
            CacheTier::Cold
        }
        fn stats(&self) -> ResourceStats {
            ResourceStats::default()
        }
        fn name(&self) -> &str {
            "dummy-resource-backend"
        }
        fn is_available(&self) -> bool {
            true
        }
    }

    /// 编译期 + 运行期验证：一旦 `K` 具体化，`ResourceBackend<K>` 即
    /// dyn 兼容（对象安全）。镜像 M1.1 裁决以及 `cesium-pipeline`
    /// （`retry.rs` / `staleness.rs`）中的 `*_is_dyn_compatible` 测试。
    /// 若该 trait 一旦获得泛型方法或 `Self: Sized` 约束，下方的
    /// `Box<dyn ...>` 强制转换就会停止编译 —— 这正是我们想要的守卫。
    #[test]
    fn resource_backend_is_dyn_compatible() {
        let boxed: Box<dyn ResourceBackend<DummyKey>> = Box::new(DummyResourceBackend);
        assert_eq!(boxed.name(), "dummy-resource-backend");
        assert!(boxed.is_available());
        assert_eq!(boxed.cache_tier(&(0, 0, 0)), CacheTier::Cold);
        assert_eq!(boxed.stats().hot_entries, 0);
    }

    /// 三个缓存层级两两不同（Hot/Warm/Cold 绝不可合并 ——
    /// 它们驱动不同的驱逐/提升行为）。
    #[test]
    fn cache_tiers_are_distinct() {
        let tiers = [CacheTier::Hot, CacheTier::Warm, CacheTier::Cold];
        for (i, a) in tiers.iter().enumerate() {
            for (j, b) in tiers.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "tiers {i} and {j} must differ");
                }
            }
        }
    }
}
