//! cesium-ports-driven: 被驱动端口（领域 → 外部）
//! 领域所依赖的外部服务的 trait 契约。
//!
//! 在六边形架构中，被驱动端口定义领域
//! 如何与外部系统通信（adapter 实现这些 trait）。

use cesium_geospatial::{GeometryData, Rectangle};
use std::future::Future;
use std::pin::Pin;

/// 端口操作的错误类型。
#[derive(Debug, thiserror::Error)]
pub enum PortError {
    #[error("Network error: {0}")]
    Network(String),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Cache error: {0}")]
    Cache(String),
    #[error("GPU error: {0}")]
    Gpu(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Cancelled")]
    Cancelled,
}

/// 端口操作的结果类型。
pub type PortResult<T> = Result<T, PortError>;

// ============================================================================
// 数据抓取端口
// ============================================================================

/// 从 URL 抓取原始字节。
/// 由 HTTP adapter 实现（reqwest、浏览器 fetch 等）
pub trait TileFetcher: Send + Sync {
    /// 从给定 URL 抓取字节。
    fn fetch<'a>(
        &'a self,
        url: &'a str,
        priority: f64,
    ) -> Pin<Box<dyn Future<Output = PortResult<Vec<u8>>> + Send + 'a>>;

    /// 取消一个挂起的抓取。
    fn cancel(&self, url: &str);
}

/// 抓取影像瓦片。
pub trait ImageryProvider: Send + Sync {
    /// 获取此影像 provider 覆盖的矩形区域。
    fn rectangle(&self) -> Rectangle;

    /// 获取最小缩放级别。
    fn minimum_level(&self) -> u32;

    /// 获取最大缩放级别。
    fn maximum_level(&self) -> u32;

    /// 获取瓦片宽度（像素）。
    fn tile_width(&self) -> u32;

    /// 获取瓦片高度（像素）。
    fn tile_height(&self) -> u32;

    /// 请求一张影像瓦片。
    fn request_image<'a>(
        &'a self,
        x: u32,
        y: u32,
        level: u32,
    ) -> Pin<Box<dyn Future<Output = PortResult<Vec<u8>>> + Send + 'a>>;
}

/// 抓取地形瓦片。
pub trait TerrainProvider: Send + Sync {
    /// 获取此地形 provider 覆盖的矩形区域。
    fn rectangle(&self) -> Rectangle;

    /// 获取最大缩放级别。
    fn maximum_level(&self) -> u32;

    /// 请求一块地形瓦片。
    fn request_tile_geometry<'a>(
        &'a self,
        x: u32,
        y: u32,
        level: u32,
    ) -> Pin<Box<dyn Future<Output = PortResult<GeometryData>> + Send + 'a>>;

    /// 获取某位置处地形数据的可用性。
    fn get_availability(&self, x: u32, y: u32, level: u32) -> bool;
}

// ============================================================================
// GPU/渲染端口
// ============================================================================

/// 一个 GPU 资源句柄（纹理、buffer 等）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpuHandle(pub u64);

/// 将几何与纹理发送到 GPU。
/// 由渲染 adapter 实现（Bevy、wgpu 等）
pub trait GpuSink: Send + Sync {
    /// 上传几何数据到 GPU，返回一个句柄。
    fn upload_geometry(&mut self, geometry: &GeometryData) -> PortResult<GpuHandle>;

    /// 上传纹理数据到 GPU，返回一个句柄。
    fn upload_texture(
        &mut self,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> PortResult<GpuHandle>;

    /// 更新一张已存在的纹理。
    fn update_texture(
        &mut self,
        handle: GpuHandle,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> PortResult<()>;

    /// 删除一个 GPU 资源。
    fn delete(&mut self, handle: GpuHandle);
}

// ============================================================================
// 解码端口
// ============================================================================

/// 解码压缩/编码的数据格式。
pub trait Decoder: Send + Sync {
    /// 解码 Draco 压缩的几何。
    fn decode_draco(&self, data: &[u8]) -> PortResult<GeometryData>;

    /// 解码一张图像（PNG、JPEG、WebP 等）
    fn decode_image(&self, data: &[u8]) -> PortResult<DecodedImage>;

    /// 解码 gzip 压缩的数据。
    fn decode_gzip(&self, data: &[u8]) -> PortResult<Vec<u8>>;
}

/// 一张带原始像素数据的已解码图像。
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub channels: u32,
    pub data: Vec<u8>,
}

// ============================================================================
// 缓存端口
// ============================================================================

/// 用于存储已抓取/已解码数据的缓存。
pub trait Cache: Send + Sync {
    /// 按 key 获取一个缓存值。
    fn get(&self, key: &str) -> Option<Vec<u8>>;

    /// 在缓存中存储一个值。
    fn set(&self, key: &str, value: Vec<u8>);

    /// 从缓存中移除一个值。
    fn remove(&self, key: &str) -> bool;

    /// 清空所有缓存数据。
    fn clear(&self);

    /// 获取当前缓存大小（字节）。
    fn size(&self) -> usize;

    /// 获取最大缓存大小（字节）。
    fn max_size(&self) -> usize;
}

// ============================================================================
// 时间/时钟端口
// ============================================================================

/// 提供当前系统时间。
pub trait SystemClock: Send + Sync {
    /// 获取当前时间（自 Unix 纪元起的秒数）。
    fn now_secs(&self) -> f64;

    /// 获取距上次调用的经过时间（用于帧计时）。
    fn delta_secs(&mut self) -> f64;
}

// ============================================================================
// 场景/渲染端口
// ============================================================================

/// 提供对渲染上下文的访问。
pub trait RenderContext: Send + Sync {
    /// 获取绘制缓冲区宽度。
    fn drawing_buffer_width(&self) -> u32;

    /// 获取绘制缓冲区高度。
    fn drawing_buffer_height(&self) -> u32;

    /// 获取设备像素比。
    fn device_pixel_ratio(&self) -> f64;
}

// ============================================================================
// M1.1 — 瓦片管线契约
// ============================================================================
//
// 定义通用瓦片管线抽象的七个 trait，抽取自
// `application/cesium-app/src/dynamic_globe.rs`（"黄金路径"参考
// 实现）。这些契约是**增量式**的——上方现有的 8 个 trait
// 保持不变，P0 三 loader 的编译也不受影响。
//
// 设计决策（由 leader 裁决）：
// - Q1：在将 K/Payload 具体化后，所有 trait 均为 dyn 兼容。
//   异步方法使用 `Pin<Box<dyn Future + Send>>`（与 TileFetcher 风格一致）。
//   无泛型方法，无 `Self: Sized` 约束。
// - Q2：这里没有 `BlockingTileFetcher`；同步网络由
//   `adapters/pipeline::NetworkBackend`（ureq 池）提供，而非 ports/driven 的 trait。
//
// 类型参数约束：
// - `K: Hash + Eq + Copy + 'static` — 瓦片 key（例如 `(u32, u32, u32)` = TileKey）
// - `Payload: Send + 'static` — 下载结果（例如 TileDownloadResult）

use std::collections::VecDeque;
use std::hash::Hash;
use std::time::Duration;

// ── TilePipeline ────────────────────────────────────────────────────────────

/// 从管线轮询一个已提交瓦片的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome<K, Payload> {
    /// 瓦片下载成功完成，带 payload。
    Ready(K, Payload),
    /// 瓦片被中止（在传输途中离开了 wanted 集合）。
    /// 对应 `dynamic_globe.rs:1052-1060` —— 仅清除 in_flight，
    /// 计入 stale_skip，并不打上 no-data 标记。
    Aborted(K),
    /// 瓦片在所有重试后抓取失败（临时性限流/超时）。
    /// 对应 `dynamic_globe.rs:1062-1072` —— 进入 retry_after
    /// 冷却期（10 秒），并不打上永久性 no-data 标记。
    Failed(K),
    /// 瓦片是一个占位图（无可用影像，例如 Bing 渐变 JPEG）。
    /// 对应 `dynamic_globe.rs:1074-1098` —— 打上永久性 no-data 标记，
    /// 继承祖先覆盖。
    Placeholder(K),
}

/// 瓦片管线核心 trait：提交工作、取消过期请求、轮询结果。
///
/// 镜像 `dynamic_globe.rs::process_pipeline`（L662-1429）
/// 和 `download_worker`（L2143-2285）中的编排逻辑。管线拥有下载线程
/// 池、去重、wanted 集合门控，以及三态结果分发。
///
/// 当 `K` 与 `Payload` 具体化时为 dyn 兼容。
pub trait TilePipeline<K, Payload>: Send + Sync
where
    K: Hash + Eq + Copy + Send + 'static,
    Payload: Send + 'static,
{
    /// 提交一个瓦片用于下载。Priority 决定抓取顺序
    /// （越大 = 越早）。对应 `enqueue_tiles`（L371-459）：
    /// 通过 in_flight 集合去重、优先级排序、wanted 集合注入。
    fn submit(&self, key: K, priority: f64);

    /// 取消一个挂起/传输中的瓦片。从 wanted 集合移除，使
    /// 下载 worker 的门控（L2161）产生一个 Aborted 结果。
    fn cancel(&self, key: &K);

    /// 轮询已完成的瓦片（非阻塞排空）。最多返回 `budget`
    /// 个结果。对应 tex_rx 排空循环（L1046-1048），受
    /// `MAX_TEXTURE_UPLOADS_PER_FRAME` 约束。
    fn poll_ready(&self, budget: usize) -> Vec<PollOutcome<K, Payload>>;

    /// 替换 wanted 集合（每帧调用一次，见 L1402-1409）。
    /// 不在新集合中的瓦片在 worker 门控处成为中止候选。
    fn refresh_wanted(&self, wanted: &[K]);

    /// 快照当前管线统计。
    fn stats(&self) -> PipelineStats;
}

// ── BudgetPolicy ────────────────────────────────────────────────────────────

/// 瓦片管线的每帧预算配置。
///
/// 默认值**逐字取自** `dynamic_globe.rs` 常量（L48-73）：
/// - `DOWNLOAD_THREADS = 16` (L48)
/// - `MAX_MESH_UPLOADS_PER_FRAME = 12` (L53)
/// - `MAX_SPAWNS_PER_FRAME = 16` (L54)
/// - `MAX_TEXTURE_UPLOADS_PER_FRAME = 16` (L55)
/// - `MAX_DESPAWNS_PER_FRAME = 24` (L58)
/// - `MAX_TILE_ENTITIES = 1800` (L65)
/// - `BASE_LAYER_ZOOM = 3` (L70)
/// - `MAX_GPU_CACHE_ENTRIES = 3000` (L73)
///
/// 这些预算将 GPU 工作切分到各帧，使一次缩放/平移不会造成
/// 数百毫秒的卡顿（L50-52 doc 注释）。
pub trait BudgetPolicy: Send + Sync {
    /// 并行下载 worker 线程数（L48：16）。
    fn download_threads(&self) -> usize;

    /// 每帧最大 mesh GPU 上传数（L53：12）。
    fn max_mesh_uploads_per_frame(&self) -> usize;

    /// 每帧最大实体 spawn 数（L54：16）。
    fn max_spawns_per_frame(&self) -> usize;

    /// 每帧最大纹理 GPU 上传数（L55：16）。
    fn max_texture_uploads_per_frame(&self) -> usize;

    /// 每帧最大实体 despawn 数（L58：24）。
    fn max_despawns_per_frame(&self) -> usize;

    /// 存活瓦片实体的硬上限（L65：1800）。隐藏瓦片也计入
    /// 此上限；它约束的是实体/句柄存储，而非绘制调用。
    fn max_tile_entities(&self) -> usize;

    /// 永久常驻的最粗缩放级别（L70：3）。z <= base_layer_zoom 的
    /// 瓦片永不被驱逐或 despawn。
    fn base_layer_zoom(&self) -> u32;

    /// GPU 句柄缓存的上界（L73：3000）。超限时最旧条目
    /// 按 FIFO 驱逐。
    fn max_gpu_cache_entries(&self) -> usize;
}

// ── EvictionPolicy ──────────────────────────────────────────────────────────

/// GPU 缓存驱逐策略，表达 `dynamic_globe.rs::evict_gpu_cache`
/// （L1476-1502）中的三条不变量：
///
/// 1. **BASE_LAYER 豁免**（L1483）：z <= base_layer_zoom 的瓦片
///    永不驱逐 —— 永久的全局回退层。
/// 2. **存活实体延后**（L1487-1491）：拥有活跃实体的瓦片被
///    推回队列尾部（花屏防护核心 —— 避免出现空帧）。
/// 3. **终止性保证**（L1471-1472）：MAX_TILE_ENTITIES(1800) <<
///    MAX_GPU_CACHE_ENTRIES(3000)，因此总是存在可驱逐的（死亡）条目。
pub trait EvictionPolicy<K>: Send + Sync
where
    K: Hash + Eq + Copy + Send + 'static,
{
    /// 返回 FIFO 驱逐顺序（最旧优先）。
    /// 对应 `mgr.gpu_tex_order: VecDeque<TileKey>`（L1479）。
    fn evict_order(&self) -> &VecDeque<K>;

    /// 若此 key 绝不可驱逐（基础层锁定）则返回 true。
    /// 对应 L1483：`if old.2 <= BASE_LAYER_ZOOM { continue }`。
    fn never_evict(&self, key: &K) -> bool;

    /// 若驱逐应被延后（瓦片仍有存活实体）则返回 true。
    /// 对应 L1487：`if mgr.tile_entities.contains_key(&old)`。
    /// 被延后的条目推回队列尾部（L1489）。
    fn defer_if_live(&self, key: &K) -> bool;
}

// ── StalenessPolicy ─────────────────────────────────────────────────────────

/// 对已下载瓦片的三态结果分类。
///
/// 镜像 `dynamic_globe.rs:1052-1098` 处的分发。这三态
/// **语义上互不相同，不可合并**：
///
/// - `Aborted`（L1052-1060）：瓦片在传输途中离开 wanted 集合。仅清除
///   in_flight + 重传守卫，计入 stale_skip。无永久状态变更。
/// - `Failed`（L1062-1072）：所有重试耗尽（临时性）。进入
///   retry_after 冷却期（10 秒）。不是永久性 no-data。
/// - `Placeholder`（L1074-1098）：无可用影像（Bing 渐变 JPEG）。
///   打上永久性 no-data 标记，通过 UV 上采样继承祖先覆盖。
pub trait StalenessPolicy: Send + Sync {
    /// 将一次下载结果分类为三态之一。
    /// 返回相应的 `PollOutcome` 变体标签。
    fn classify(&self, aborted: bool, failed: bool, placeholder: bool) -> StalenessVerdict;

    /// Failed 裁决后重试冷却期的时长（L1070：10 秒）。
    fn retry_cooldown(&self) -> Duration;
}

/// 由过期分类得出的裁决。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StalenessVerdict {
    /// 瓦片离开 wanted 集合；清除 in_flight，计入 stale_skip（L1052-1060）。
    Aborted,
    /// 临时性失败；进入重试冷却期（L1062-1072）。
    Failed,
    /// 永久性 no-data；继承祖先覆盖（L1074-1098）。
    Placeholder,
    /// 下载成功，payload 可用。
    Fresh,
}

// ── RetryPolicy ─────────────────────────────────────────────────────────────

/// 镜像 `dynamic_globe.rs` 的双时间尺度重试配置：
///
/// 1. **Worker 级重试**（L2186-2191）：3 次尝试，指数退避
///    `250ms << attempt`（250 毫秒、500 毫秒、1000 毫秒）。处理
///    来自瓦片服务器的临时性 403/429/超时。
/// 2. **管线级冷却**（L1068-1071）：在所有 worker 重试
///    失败后，瓦片进入 `retry_after` map，带 10 秒冷却，之后
///    入队路径（L398-400、L419-421）才会重新发起抓取。
///
/// 两个时间尺度都必不可少：worker 重试处理瞬时抖动；
/// 管线冷却防止每帧都猛击一个限流的服务器。
pub trait RetryPolicy: Send + Sync {
    /// Worker 级重试次数（L2186：3）。
    fn max_attempts(&self) -> u32;

    /// Worker 重试的基准退避时长（L2189：250 毫秒）。
    /// 实际 sleep = `base << attempt`（指数）。
    fn backoff_base(&self) -> Duration;

    /// 所有尝试耗尽后的管线级冷却（L1070：10 秒）。
    fn cooldown(&self) -> Duration;
}

// ── PipelineStats ───────────────────────────────────────────────────────────

/// 管线统计快照，与 M0.4 的 `PerfCounters`
/// （`application/cesium-app/src/perf_counters.rs`）以及 17 列 CSV
/// 追踪格式（`perf_trace.rs::CSV_HEADER`）对齐。
///
/// 字段到 PerfCounters / CSV 列的映射：
/// | PipelineStats 字段     | PerfCounters 字段  | CSV 列（从 1 开始）      |
/// |-----------------------|--------------------|------------------------|
/// | `frame_idx`           | `frame_idx`        | 1                      |
/// | `dt_ms`               | `dt_ms`            | 2                      |
/// | `visible_n`           | `tile_entities`    | 4                      |
/// | `partition_n`         | `spawn_queue`      | 5                      |
/// | `load_n`              | `load_set`         | 6                      |
/// | `spawn_n`             | `frame_spawn`      | 7                      |
/// | `tex_upload_n`        | `frame_tex`        | 8                      |
/// | `evict_n`             | `evict_total`      | 9                      |
/// | `gpu_tex_cache`       | `gpu_tex_order`    | 10                     |
/// | `mesh_backlog`        | `backlog`          | 11                     |
/// | `dl_in_flight`        | `in_flight`        | 12                     |
/// | `stale_skips`         | `stale_skips`      | 13                     |
/// | `retry_after`         | `retry_after`      | 14                     |
/// | `frame_mesh`          | `frame_mesh`       | 15                     |
/// | `frame_despawn`       | `frame_despawn`    | 16                     |
/// | `evict_deferred`      | `evict_deferred`   | 17                     |
///
/// 第 3 列（`view_sse`）是四叉树的每瓦片值，不在管线层级
/// 追踪；保留为 0 占位（见 deferred DEFER-M0-VIEWSSE）。
#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    /// 单调递增的帧索引（CSV col 1）。
    pub frame_idx: u32,
    /// 帧 wall-clock 时间差（毫秒）（CSV col 2）。
    pub dt_ms: f64,
    /// 存活瓦片实体（CSV col 4 = PerfCounters::tile_entities）。
    pub visible_n: u32,
    /// spawn 队列深度（CSV col 5 = PerfCounters::spawn_queue）。
    pub partition_n: u32,
    /// load 集合大小（CSV col 6 = PerfCounters::load_set）。
    pub load_n: u32,
    /// 本帧 spawn 的实体数（CSV col 7）。
    pub spawn_n: u32,
    /// 本帧上传的纹理数（CSV col 8）。
    pub tex_upload_n: u32,
    /// 累计驱逐数（CSV col 9）。
    pub evict_n: u32,
    /// GPU 纹理缓存条目数（CSV col 10）。
    pub gpu_tex_cache: u32,
    /// mesh 积压深度（CSV col 11）。
    pub mesh_backlog: u32,
    /// 传输中的下载数（CSV col 12）。
    pub dl_in_flight: u32,
    /// 累计过期跳过数（CSV col 13）。
    pub stale_skips: u32,
    /// 处于重试冷却期的瓦片数（CSV col 14）。
    pub retry_after: u32,
    /// 本帧 mesh 上传数（CSV col 15）。
    pub frame_mesh: u32,
    /// 本帧 despawn 数（CSV col 16）。
    pub frame_despawn: u32,
    /// 被延后的驱逐数（CSV col 17）。
    pub evict_deferred: u32,
}

// ── ResourceBackend (M8 / P2.3) ─────────────────────────────────────────────
// 已移至 `ports/driven/src/resource.rs`（M8.2 位置修正）。在下方
// 重新导出，使所有现有的 `use cesium_ports_driven::{CacheTier, ResourceBackend,
// ResourceStats}` 路径保持有效。
pub mod resource;
pub use resource::{CacheTier, ResourceBackend, ResourceStats};
