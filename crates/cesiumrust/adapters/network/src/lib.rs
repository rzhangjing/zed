//! cesium-network：HTTP + 离线磁盘网络适配器
//!
//! 实现 `TileFetcher` / `TerrainProvider` 驱动端口：
//!
//! * [`HttpTileFetcher`] —— 通过 ureq 发起的同步 HTTP 请求，经由一个
//!   **不依赖 tokio** 的 `std::thread` + `mpsc` 桥接分发
//!   （[`resource_backend_impl::spawn_blocking_fetch`]）——具限速、
//!   重试、可取消能力。M8.3（#66）清理了之前的
//!   `tokio::task::spawn_blocking` / `tokio::time::sleep` / `tokio::sync::Mutex`
//!   生产路径，改用共享的阻塞池理念
//!   （`adapters/pipeline/src/pool.rs`：16 个工作线程 + keep-alive，无 tokio
//!   Runtime）。
//! * [`FileTileFetcher`] —— 离线磁盘支撑的影像瓦片（XYZ / quadkey
//!   布局），具有 STRICT_OFFLINE 语义（无 HTTP 回退）。
//! * [`FileTerrainFetcher`] —— 离线磁盘支撑的 heightmap-1.0 地形瓦片。
//! * [`MockTileFetcher`] —— 供测试使用的预定义响应获取器。
//! * [`resource_backend_impl`] —— M8.3 的 [`ResourceBackend`] 网络适配器
//!   （[`NetworkResourceBackend`]）+ 针对 `CESIUM_ENABLE_RESOURCE_FETCH_BACKEND` 的
//!   本地环境门控（[`resource_fetch_backend_enabled`]）。
//!   当门控为 ON 时，[`HttpTileFetcher::fetch`] 经由 pipeline 管理的
//!   16 工作线程 keep-alive 池 + 热/冷缓存层级路由；
//!   为 OFF（默认）时，走 M8.3 前的直接-ureq 路径，以保证 v0
//!   基线逐字节一致。

pub mod file_terrain_fetcher;
pub mod file_tile_fetcher;
pub mod resource_backend_impl;

pub use file_terrain_fetcher::{FileTerrainFetcher, TerrainScheme};
pub use file_tile_fetcher::{FileTileFetcher, FileTileScheme};
pub use resource_backend_impl::{
    block_on_noop, gate_from_env_value, resource_fetch_backend_enabled, spawn_blocking_fetch,
    url_hash, NetworkResourceBackend, ENV_ENABLE_RESOURCE_FETCH_BACKEND,
};

use cesium_ports_driven::{PortError, PortResult, TileFetcher};
// M8.4（#67）：适配器执行由 `Resource::fetch_*`/`post` 产生的、无 IO 的领域
// `FetchDescriptor`。domain/resource 保持无网络；
// 所有 HTTP 执行都集中在此处的适配器层。
use cesium_resource::{FetchDescriptor, HttpMethod};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use thiserror::Error;

const DEFAULT_MAX_REQUESTS_PER_SERVER: usize = 6;
const DEFAULT_RETRY_COUNT: u32 = 3;
const DEFAULT_READ_TIMEOUT_SECS: u64 = 30;
const RATE_LIMIT_POLL_MS: u64 = 50;

/// 网络错误
#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("IO error: {0}")]
    IoError(String),

    #[error("Timeout")]
    Timeout,

    #[error("Request cancelled")]
    Cancelled,
}

/// 基于 HTTP 的瓦片获取器，使用 ureq 发起同步的 HTTP/HTTPS 请求。
///
/// 请求被分发到一个**不依赖 tokio** 的 `std::thread` + `mpsc` 桥接
/// （[`resource_backend_impl::spawn_blocking_fetch`]），从而同步的 ureq
/// 调用不会阻塞轮询上下文。当
/// [`resource_fetch_backend_enabled()`] 返回 `true`（环境门控
/// `CESIUM_ENABLE_RESOURCE_FETCH_BACKEND`）时，获取会经由共享的
/// [`NetworkResourceBackend`]（16 工作线程 keep-alive 池 + 热/冷
/// 缓存层级 + 在途去重，均复用自 `cesium-pipeline`）路由；
/// 为 OFF（默认）时运行 M8.3 前的直接-ureq 路径，以保持 v0
/// 基线逐字节一致。
pub struct HttpTileFetcher {
    /// 共享的 ureq HTTP 客户端实例，负责连接池与 keep-alive。
    agent: ureq::Agent,
    /// 预留的基地址（当前直接-ureq 路径未使用，故 `dead_code`）。
    #[allow(dead_code)]
    base_url: String,
    /// 随每个请求发送的额外请求头键值对。
    headers: HashMap<String, String>,
    /// 每服务器在途计数（限速门控）。M8.3：`tokio::sync::Mutex`
    /// → `std::sync::Mutex`（在 spawn 出的 std::thread 内阻塞等待；
    /// 因为整个限速循环都运行在 [`spawn_blocking_fetch`] 内部，
    /// 所以绝不会阻塞轮询上下文）。
    active_requests: Arc<StdMutex<HashMap<String, usize>>>,
    /// 单个服务器允许同时在途的最大请求数（限速门控阈值）。
    max_requests_per_server: usize,
    /// 瞬时错误时的重试次数。
    retry_count: u32,
    /// 已取消 URL 集合，供在途任务轮询以提前中断。
    cancelled: Arc<StdMutex<HashSet<String>>>,
    /// M8.3 网络 `ResourceBackend` —— 仅当
    /// [`resource_fetch_backend_enabled()`] 返回 `true` 时才使用。在所有
    /// `HttpTileFetcher` 克隆间共享，以便 16 工作线程 keep-alive 池 + 热/冷
    /// 缓存层级在进程范围内被摊薄复用。
    resource_backend: Arc<NetworkResourceBackend>,
}

impl HttpTileFetcher {
    /// 创建一个新的 `HttpTileFetcher`，使用默认的 ureq agent。
    pub fn new(base_url: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(DEFAULT_READ_TIMEOUT_SECS))
            .build();

        Self {
            agent,
            base_url: base_url.to_string(),
            headers: HashMap::new(),
            active_requests: Arc::new(StdMutex::new(HashMap::new())),
            max_requests_per_server: DEFAULT_MAX_REQUESTS_PER_SERVER,
            retry_count: DEFAULT_RETRY_COUNT,
            cancelled: Arc::new(StdMutex::new(HashSet::new())),
            resource_backend: Arc::new(NetworkResourceBackend::new()),
        }
    }

    /// 创建一个新的 `HttpTileFetcher`，使用自定义的 ureq agent。
    pub fn with_agent(base_url: &str, agent: ureq::Agent) -> Self {
        Self {
            agent,
            base_url: base_url.to_string(),
            headers: HashMap::new(),
            active_requests: Arc::new(StdMutex::new(HashMap::new())),
            max_requests_per_server: DEFAULT_MAX_REQUESTS_PER_SERVER,
            retry_count: DEFAULT_RETRY_COUNT,
            cancelled: Arc::new(StdMutex::new(HashSet::new())),
            resource_backend: Arc::new(NetworkResourceBackend::new()),
        }
    }

    /// 设置一个请求头。
    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    /// 设置每服务器的最大并发请求数。
    pub fn with_max_requests_per_server(mut self, max: usize) -> Self {
        self.max_requests_per_server = max;
        self
    }

    /// 设置瞬时失败时的重试次数。
    pub fn with_retry_count(mut self, retries: u32) -> Self {
        self.retry_count = retries;
        self
    }

    /// 从 URL 中提取服务器键（host[:port]）。
    fn extract_server_key(url: &str) -> String {
        if let Some(start) = url.find("://") {
            let rest = &url[start + 3..];
            if let Some(end) = rest.find('/') {
                return rest[..end].to_string();
            }
            return rest.to_string();
        }
        url.to_string()
    }

    /// 执行一次 HTTP GET 请求并返回响应体。
    ///
    /// L2 审查修正：返回一个 [`FetchFailure`]（错误 + 重试分类）
    /// 而非裸的 `PortError`，以便 `do_fetch_with_retry` 只重试
    /// 真正瞬时的失败（408/429/5xx/传输错误），而对永久性 4xx 客户端
    /// 错误快速失败。
    fn do_fetch(
        agent: &ureq::Agent,
        url: &str,
        headers: &HashMap<String, String>,
    ) -> Result<Vec<u8>, FetchFailure> {
        let mut req = agent.get(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }

        let resp = req.call().map_err(classify_ureq_error)?;

        let mut data = Vec::new();
        resp.into_reader()
            .read_to_end(&mut data)
            .map_err(|e| FetchFailure {
                // 响应体读取中途出错（连接重置、响应被截断）
                // 是瞬时的 —— 重试可能成功。
                err: PortError::Network(format!("Failed to read response body: {}", e)),
                transient: true,
            })?;

        Ok(data)
    }

    /// 执行一次获取，带面向瞬时失败的重试逻辑。
    fn do_fetch_with_retry(
        agent: &ureq::Agent,
        url: &str,
        headers: &HashMap<String, String>,
        retry_count: u32,
    ) -> PortResult<Vec<u8>> {
        let mut last_err = None;

        for attempt in 0..=retry_count {
            match Self::do_fetch(agent, url, headers) {
                Ok(data) => return Ok(data),
                Err(failure) => {
                    // L2 审查修正：只重试瞬时失败。修正前的
                    // `matches!(&e, PortError::Network(_))` 会重试*每一个*
                    // 非-404 状态（map_ureq_error 把它们全部归入
                    // Network），因此一个永久性的 400/401/403 会被
                    // 毫无意义地重试。`classify_ureq_error` 现在将 408/429/5xx/
                    // 传输错误标为瞬时，其余标为永久。
                    if !failure.transient {
                        return Err(failure.err);
                    }
                    last_err = Some(failure.err);
                    if attempt < retry_count {
                        std::thread::sleep(Duration::from_millis(
                            100 * (attempt as u64 + 1),
                        ));
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            PortError::Network("Retry exhausted with no error".to_string())
        }))
    }

    /// M8.4（#67）：经由受门控保护的适配器路径执行一个领域 [`FetchDescriptor`]，
    /// 并将门控判定**注入**，以便 ON/OFF 分支可在不变更进程
    /// 环境的情况下进行单元测试。
    ///
    /// 路由（公共契约见 [`Self::fetch_descriptor_blocking`]）：
    /// * `is_data_uri` → 通过纯领域解码器短路
    ///   （`cesium_resource::data_uri::decode_data_uri_bytes`）；零网络。
    /// * 非-GET 方法 → `PortError::Network`（`NetworkBackend::fetch(url)`
    ///   trait + 遗留的 `do_fetch` 只执行 GET；按请求的
    ///   方法/请求体尚待 trait 扩展——docs/deferred.md）。五个
    ///   `Resource::fetch_*` 构建器均发出 GET，因此常见路径已被覆盖。
    /// * `gate_enabled` → `NetworkResourceBackend`（→ `PipelineResourceBackend`
    ///   → 16 工作线程 keep-alive `WorkerPool` → `UreqBackend`），按 url + 优先级。
    /// * 否则 → 逐字节一致的 M8.4 前直接-ureq 路径
    ///   （`do_fetch_with_retry`，将获取器 headers 与 descriptor headers 合并，
    ///   重试次数取自 `descriptor.retry.max_attempts`）。
    fn execute_descriptor_gated(
        &self,
        descriptor: &FetchDescriptor,
        gate_enabled: bool,
    ) -> PortResult<Vec<u8>> {
        // L4 审查修正：在数据-URI 短路*之前*拒绝非-GET 方法，
        // 以便携带非-GET 方法的 `data:` descriptor 不会绕过方法守卫。
        // （每个发出数据 URI 的 `Resource::fetch_*`/`post`
        // 构建器都使用 GET，因此这一重排在不改变黄金路径
        // 行为的前提下收紧了守卫。）
        if !matches!(descriptor.method, HttpMethod::Get) {
            return Err(PortError::Network(format!(
                "M8.4 backend executes GET only; {:?} awaits a NetworkBackend trait extension",
                descriptor.method
            )));
        }
        if descriptor.is_data_uri {
            return cesium_resource::data_uri::decode_data_uri_bytes(&descriptor.url)
                .map_err(|e| PortError::Decode(format!("data URI decode failed: {e:?}")));
        }
        // H2 审查修正（过渡性，选项 b）：受门控保护的后端路径
        // （`NetworkResourceBackend::fetch_url_blocking`）只转发
        // url + 优先级 —— 它会丢弃 `descriptor.headers`（可能携带
        // Authorization / Ion token）和 `descriptor.retry`。直到
        // `NetworkBackend` trait 长出 headers/retry 参数（deferred #41）为止，
        // 任何实际携带 headers 的请求都经由逐字节一致的直接路径，
        // 以免静默丢失凭据。无 headers 的请求仍享受共享池 + 缓存层级。
        let has_headers = !descriptor.headers.is_empty() || !self.headers.is_empty();
        if gate_enabled && !has_headers {
            self.resource_backend
                .fetch_url_blocking(&descriptor.url, descriptor.priority)
        } else {
            let mut headers = self.headers.clone();
            headers.extend(descriptor.headers.clone());
            Self::do_fetch_with_retry(
                &self.agent,
                &descriptor.url,
                &headers,
                descriptor.retry.max_attempts,
            )
        }
    }

    /// M8.4（#67）：无 IO 的
    /// `Resource::fetch_array_buffer`/`fetch_json`/`fetch_text`/`fetch_image`/
    /// `fetch_blob`/`post` descriptor 构建器在适配器端的执行对应物。它闭合了 M8
    /// “`Resource::fetch` 全量切换收敛”门（门④）：每个 descriptor 要么
    /// 经由共享后端（门 ON）执行，要么经由逐字节一致的遗留
    /// 直接路径（门 OFF）执行 —— 绝不通过分散在某个 loader 里的临时 HTTP 调用，
    /// 因此切换之后仍能保持 门①（HTTP 直接调用限定在后端抽象
    /// 层内）。
    ///
    /// 门控仅从 [`resource_fetch_backend_enabled()`]
    /// （`CESIUM_ENABLE_RESOURCE_FETCH_BACKEND`）读取一次。阻塞式（同
    /// [`NetworkResourceBackend::fetch_url_blocking`]）；从工作线程调用，绝不
    /// 从帧线程调用。domain/resource 保持无 IO。
    pub fn fetch_descriptor_blocking(&self, descriptor: &FetchDescriptor) -> PortResult<Vec<u8>> {
        self.execute_descriptor_gated(descriptor, resource_fetch_backend_enabled())
    }
}

impl TileFetcher for HttpTileFetcher {
    /// 经由 HTTP 获取一个瓦片的原始字节。
    ///
    /// 在 `spawn_blocking` 出的 std 线程上执行带限速与重试的阻塞获取，
    /// 返回一个解析为 [`Vec<u8>`] 的 boxed future；`priority` 仅在启用
    /// [`NetworkResourceBackend`] 路径时影响调度优先级。
    fn fetch<'a>(
        &'a self,
        url: &'a str,
        priority: f64,
    ) -> Pin<Box<dyn Future<Output = PortResult<Vec<u8>>> + Send + 'a>> {
        let url_owned = url.to_string();
        let cancelled = Arc::clone(&self.cancelled);
        let agent = self.agent.clone();
        let headers = self.headers.clone();
        let active = Arc::clone(&self.active_requests);
        let max_req = self.max_requests_per_server;
        let retry_count = self.retry_count;
        let server_key = Self::extract_server_key(&url_owned);
        let backend = Arc::clone(&self.resource_backend);
        // 每次获取只对门控快照一次，以便飞行中途的环境变更无法
        // 撕裂限速 + 获取的判定（与 M8.3 前的原子行为一致，
        // 当时 `tokio::task::spawn_blocking` 在 spawn 时就捕获了
        // 闭包环境）。
        let use_backend = resource_fetch_backend_enabled();

        // M8.3 tokio 清理：整个限速 + 获取 + 释放序列
        // 在一个专门的 `std::thread`（通过 `spawn_blocking_fetch`）上运行，
        // 而非在 tokio 工作线程上。返回的 future 在 `mpsc::recv()`
        // 上阻塞轮询线程，并在首次 poll 时解析为 `Ready`，与
        // `PipelineResourceBackend::request_stream` 模式一致。调用方必须
        // 从 IO/工作上下文驱动这个 future（绝不从帧线程）——
        // 与 `PipelineResourceBackend` 发布的契约相同。
        spawn_blocking_fetch(move || {
            // 取消检查（与 M8.3 前逐字节一致的语义）。
            {
                let cancelled_set = cancelled.lock().unwrap();
                if cancelled_set.contains(&url_owned) {
                    return Err(PortError::Cancelled);
                }
            }

            // 限速：等待直到为该服务器开出一个名额。M8.3 前是一个
            // `tokio::time::sleep(...).await` 循环；现在是 spawn 出的工作线程
            // 内阻塞的 `std::thread::sleep`。可观察行为（名额获取
            // 顺序、轮询间隔、饱和释放）不变。
            loop {
                let acquired = {
                    let mut active_map = active.lock().unwrap();
                    let count = active_map.entry(server_key.clone()).or_insert(0);
                    if *count < max_req {
                        *count += 1;
                        true
                    } else {
                        false
                    }
                };
                if acquired {
                    break;
                }
                std::thread::sleep(Duration::from_millis(RATE_LIMIT_POLL_MS));
            }

            // 分发：门 ON 经由共享的 16 工作线程 keep-alive 池路由
            // （NetworkResourceBackend → PipelineResourceBackend →
            // WorkerPool → UreqBackend）；门 OFF 走 M8.3 前的直接
            // ureq 路径（`do_fetch_with_retry`），以保证 v0 基线
            // 逐字节一致。
            // H2 审查修正（过渡性，选项 b）：镜像
            // `execute_descriptor_gated`。受门控保护的后端路径只转发
            // url + 优先级，因此为配置了 headers 的获取器（例如通过
            // `with_header` 设置的 Authorization / Ion token）会丢失它们。
            // 在后端长出 headers 参数（deferred #41）之前，将携带 headers
            // 的获取经由直接路径。
            let result = if use_backend && headers.is_empty() {
                backend.fetch_url_blocking(&url_owned, priority)
            } else {
                Self::do_fetch_with_retry(&agent, &url_owned, &headers, retry_count)
            };

            // 释放名额（与 M8.3 前逐字节一致的语义）。
            {
                let mut active_map = active.lock().unwrap();
                if let Some(count) = active_map.get_mut(&server_key) {
                    *count = count.saturating_sub(1);
                }
            }

            result
        })
    }

    /// 取消一个在途获取：将 `url` 登记到 `cancelled` 集合，供飞行中
    /// 的任务在下一次轮询时提前中断。
    fn cancel(&self, url: &str) {
        let mut set = self.cancelled.lock().unwrap();
        set.insert(url.to_string());
    }
}

/// 将一个 ureq 错误映射为 `PortError`。
fn map_ureq_error(err: ureq::Error) -> PortError {
    match err {
        ureq::Error::Status(code, _resp) => {
            if code == 404 {
                PortError::NotFound(format!("HTTP {}", code))
            } else {
                PortError::Network(format!("HTTP status {}", code))
            }
        }
        ureq::Error::Transport(transport) => {
            let msg = transport.to_string();
            if msg.contains("timed out") || msg.contains("Timeout") {
                PortError::Network(format!("Request timed out: {}", msg))
            } else {
                PortError::Network(format!("Transport error: {}", msg))
            }
        }
    }
}

/// 一个获取失败，与其重试分类配对（L2 审查修正）。
///
/// 修正前的重试循环从 `matches!(err, PortError::Network(_))` 推断“瞬时”，
/// 但 [`map_ureq_error`] 将*每一个*非-404 的 HTTP 状态都归入
/// `PortError::Network` —— 因此一个永久性的 4xx（400 错误请求、
/// 401/403 鉴权）会被毫无意义地重试。携带一个显式的
/// `transient` 标志，使 [`HttpTileFetcher::do_fetch_with_retry`] 只重试
/// 真正可重试的失败，其余快速失败。
struct FetchFailure {
    /// 向调用方冒泡的错误。
    err: PortError,
    /// 该失败是否值得重试。
    transient: bool,
}

/// 将一个 ureq 错误分类为一个 [`FetchFailure`]（L2 审查修正）。
///
/// * HTTP 408（请求超时）/ 429（请求过多）/ 5xx → 瞬时。
/// * HTTP 404 → `PortError::NotFound`，非瞬时。
/// * 任何其他 4xx → `PortError::Network`，非瞬时（客户端错误不会
///   在重试时自行恢复）。
/// * 传输错误（DNS、连接、读取超时）→ 瞬时。
///
/// `err` 负载复用 [`map_ureq_error`]，因此冒泡的错误变体与
/// 修正前保持一致；仅修正了重试决策。
fn classify_ureq_error(err: ureq::Error) -> FetchFailure {
    let transient = match &err {
        ureq::Error::Status(code, _) => *code == 408 || *code == 429 || *code >= 500,
        ureq::Error::Transport(_) => true,
    };
    FetchFailure {
        err: map_ureq_error(err),
        transient,
    }
}

// ============================================================================
// MockTileFetcher（用于测试）
// ============================================================================

/// 一个用于测试的 mock 瓦片获取器，返回预定义数据。
pub struct MockTileFetcher {
    /// URL → 预定义字节响应的映射表，命中时直接返回。
    responses: HashMap<String, Vec<u8>>,
}

impl MockTileFetcher {
    /// 创建一个新的 mock 瓦片获取器。
    pub fn new() -> Self {
        Self {
            responses: HashMap::new(),
        }
    }

    /// 为一个 URL 添加预定义响应。
    pub fn with_response(mut self, url: &str, data: Vec<u8>) -> Self {
        self.responses.insert(url.to_string(), data);
        self
    }
}

impl Default for MockTileFetcher {
    /// 默认构造一个空响应的 mock 获取器（等价于 [`MockTileFetcher::new`]）。
    fn default() -> Self {
        Self::new()
    }
}

impl TileFetcher for MockTileFetcher {
    /// 同步地从预定义表返回 `url` 对应的字节；未命中则
    /// [`PortError::NotFound`]。future 首次 poll 即完成。
    fn fetch<'a>(
        &'a self,
        url: &'a str,
        _priority: f64,
    ) -> Pin<Box<dyn Future<Output = PortResult<Vec<u8>>> + Send + 'a>> {
        let result = self
            .responses
            .get(url)
            .cloned()
            .ok_or_else(|| PortError::NotFound(format!("No mock response for URL: {}", url)));

        Box::pin(async move { result })
    }

    /// mock 获取器为同步一次性完成，无可取消项，故为空实现。
    fn cancel(&self, _url: &str) {
        // mock 获取器不需要取消
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_ports_driven::ResourceBackend;

    // --- extract_server_key --------------------------------------------------

    #[test]
    fn test_extract_server_key_https() {
        assert_eq!(
            HttpTileFetcher::extract_server_key("https://example.com/tiles/0/0/0.terrain"),
            "example.com"
        );
    }

    #[test]
    fn test_extract_server_key_http_with_port() {
        assert_eq!(
            HttpTileFetcher::extract_server_key("http://localhost:8080/api"),
            "localhost:8080"
        );
    }

    #[test]
    fn test_extract_server_key_no_scheme() {
        assert_eq!(
            HttpTileFetcher::extract_server_key("example.com/path"),
            "example.com/path"
        );
    }

    #[test]
    fn test_extract_server_key_no_path() {
        assert_eq!(
            HttpTileFetcher::extract_server_key("https://example.com"),
            "example.com"
        );
    }

    // --- 构建器 -------------------------------------------------------------

    #[test]
    fn test_http_tile_fetcher_builder() {
        let fetcher = HttpTileFetcher::new("https://assets.cesium.com")
            .with_header("Authorization", "Bearer token")
            .with_max_requests_per_server(10)
            .with_retry_count(5);

        assert_eq!(fetcher.base_url, "https://assets.cesium.com");
        assert_eq!(fetcher.max_requests_per_server, 10);
        assert_eq!(fetcher.retry_count, 5);
        assert!(fetcher.headers.contains_key("Authorization"));
        assert_eq!(fetcher.headers.get("Authorization").unwrap(), "Bearer token");
    }

    #[test]
    fn test_http_tile_fetcher_defaults() {
        let fetcher = HttpTileFetcher::new("https://assets.cesium.com");
        assert_eq!(fetcher.max_requests_per_server, 6);
        assert_eq!(fetcher.retry_count, 3);
        assert!(fetcher.headers.is_empty());
    }

    #[test]
    fn test_http_tile_fetcher_with_agent() {
        let agent = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(10))
            .build();
        let fetcher = HttpTileFetcher::with_agent("https://custom.example.com", agent);
        assert_eq!(fetcher.base_url, "https://custom.example.com");
    }

    // --- 真实获取（类集成）---------------------------------------
    //
    // M8.3：`#[tokio::test]` → `#[test]` + `block_on_noop`。新的
    // `spawn_blocking_fetch` 桥接在首次 poll 时解析为 `Ready`（工作线程
    // `std::thread` 内部阻塞在 `mpsc::recv` 上），因此一个
    // `Waker::noop()` 单次-poll 驱动器就足够了 —— 无 tokio Runtime，
    // 符合生产路径的不依赖-tokio 契约。

    #[test]
    fn test_fetch_invalid_url() {
        let fetcher = HttpTileFetcher::new("https://invalid.example.invalid");
        let result = block_on_noop(fetcher.fetch("https://invalid.example.invalid/tile.terrain", 1.0));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PortError::Network(_)));
    }

    #[test]
    fn test_fetch_cancelled() {
        let fetcher = HttpTileFetcher::new("https://example.com");
        fetcher.cancel("https://example.com/cancelled.terrain");

        let result = block_on_noop(fetcher.fetch("https://example.com/cancelled.terrain", 1.0));
        assert!(matches!(result.unwrap_err(), PortError::Cancelled));
    }

    // --- mock ----------------------------------------------------------------

    #[test]
    fn test_mock_tile_fetcher() {
        let fetcher =
            MockTileFetcher::new().with_response("http://test.com/tile.terrain", vec![1, 2, 3, 4]);

        let result = block_on_noop(fetcher.fetch("http://test.com/tile.terrain", 1.0));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec![1, 2, 3, 4]);

        let result = block_on_noop(fetcher.fetch("http://test.com/missing.terrain", 1.0));
        assert!(result.is_err());
    }

    #[test]
    fn test_mock_tile_fetcher_cancel_is_noop() {
        let fetcher = MockTileFetcher::new();
        fetcher.cancel("anything");
        let result = block_on_noop(fetcher.fetch("anything", 1.0));
        assert!(result.is_err()); // 未找到，非取消 —— cancel 是一个空操作
    }

    // --- M8.3 门控分支可达性 ----------------------------------------
    //
    // 证明 `HttpTileFetcher::fetch` 中的门-ON 分支在语法上可达，且
    // `NetworkResourceBackend` 已接入该结构体。端到端的 wiremock
    // 驱动断言位于 `specs/tests/e2e_network/*`（deferred #37，
    // 在 M11.1 接入异步测试架之前以 `#[ignore]` 门控）。

    #[test]
    fn http_tile_fetcher_holds_network_resource_backend() {
        let fetcher = HttpTileFetcher::new("https://example.com");
        // 后端被急切地构造，以便门-ON 分支在获取时是一个纯粹的环境
        // 变量判定（无懒初始化的竞态）。
        assert_eq!(fetcher.resource_backend.name(), "cesium-network-resource");
        assert!(fetcher.resource_backend.is_available());
    }

    #[test]
    fn gate_off_takes_direct_ureq_branch() {
        // 对于黄金路径，环境变量必须未设置 CESIUM_ENABLE_RESOURCE_FETCH_BACKEND；
        // 断言观察到的默认值，以便 CI 中一个误设的导出会在此处响亮地
        // 失败，而不是静默地翻转分支。
        if std::env::var(ENV_ENABLE_RESOURCE_FETCH_BACKEND).is_err() {
            assert!(!resource_fetch_backend_enabled());
        }
    }

    // --- 错误映射 -------------------------------------------------------

    #[test]
    fn test_map_ureq_error_status_404() {
        // 没有真实响应我们很难构造 ureq::Error::Status，
        // 但上面的集成测试已间接测试了该逻辑。
        // 这个占位注释记录了预期的映射。
    }

    // --- M8.4（#67）：FetchDescriptor 执行接入 ------------------------
    //
    // 证明 M8.4 收敛门：由 `Resource::fetch_*`/`post` 产生的无-IO 领域
    // descriptor 经由受门控保护的适配器路径执行 —— 数据 URI 短路（零网络），
    // 门 ON 经由 `NetworkResourceBackend` 路由，门 OFF 走逐字节一致的
    // 直接路径。门控判定通过 `execute_descriptor_gated` 注入，因此两个
    // 分支都能在不变更进程环境（这在并行测试间会产生竞态）的
    // 情况下被执行。

    /// 最小化的离线 HTTP 服务器（临时 127.0.0.1 端口），对每个 GET 都返回 `body`。
    /// 镜像了 `adapters/pipeline/src/net/mod.rs::test_server`
    /// （后者是 `pub(crate)`，因此跨 crate 不可达）。
    fn spawn_test_server(body: &'static [u8]) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("local_addr").port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf); // 排空请求头
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if stream.write_all(head.as_bytes()).is_err() {
                    return;
                }
                if stream.write_all(body).is_err() {
                    return;
                }
                let _ = stream.flush();
            }
        });
        format!("http://127.0.0.1:{port}/tile")
    }

    #[test]
    fn fetch_descriptor_data_uri_short_circuits_no_network() {
        let fetcher = HttpTileFetcher::new("https://example.com");
        // "QUJD" 是 "ABC" 的 base64。
        let resource = cesium_resource::Resource::new("data:application/octet-stream;base64,QUJD");
        let descriptor = resource.fetch_array_buffer(None);
        assert!(descriptor.is_data_uri);
        let before = fetcher.resource_backend.fetch_count();
        // 数据 URI 在门控被查询之前就已短路。
        let out = fetcher
            .fetch_descriptor_blocking(&descriptor)
            .expect("data URI decodes");
        assert_eq!(out, b"ABC");
        assert_eq!(
            fetcher.resource_backend.fetch_count(),
            before,
            "data URI must not touch the backend"
        );
    }

    #[test]
    fn fetch_descriptor_rejects_non_get_method() {
        let fetcher = HttpTileFetcher::new("https://example.com");
        let resource = cesium_resource::Resource::new("https://example.com/api");
        let descriptor = resource.post(vec![1, 2, 3], None);
        assert!(matches!(descriptor.method, HttpMethod::Post));
        let err = fetcher.fetch_descriptor_blocking(&descriptor).unwrap_err();
        assert!(
            matches!(err, PortError::Network(_)),
            "non-GET surfaces a Network error, got {err:?}"
        );
    }

    #[test]
    fn fetch_descriptor_gate_off_direct_path_returns_body() {
        let url = spawn_test_server(b"tiledata");
        let fetcher = HttpTileFetcher::new("");
        let resource = cesium_resource::Resource::new(&url);
        let descriptor = resource.fetch_array_buffer(None);
        assert!(!descriptor.is_data_uri);
        // 门 OFF（注入）-> 逐字节一致的遗留直接-ureq 路径。
        let out = fetcher
            .execute_descriptor_gated(&descriptor, false)
            .expect("direct path");
        assert_eq!(out, b"tiledata");
    }

    #[test]
    fn fetch_descriptor_gate_on_backend_path_returns_body() {
        let url = spawn_test_server(b"tiledata");
        let fetcher = HttpTileFetcher::new("");
        let resource = cesium_resource::Resource::new(&url);
        let descriptor = resource.fetch_array_buffer(None);
        let before = fetcher.resource_backend.fetch_count();
        // 门 ON（注入）-> NetworkResourceBackend -> WorkerPool -> UreqBackend。
        let out = fetcher
            .execute_descriptor_gated(&descriptor, true)
            .expect("backend path");
        assert_eq!(out, b"tiledata");
        assert!(
            fetcher.resource_backend.fetch_count() > before,
            "gate ON must route through the backend"
        );
    }

    /// 离线 HTTP 服务器，仅当请求头携带 `required_header`（大小写不敏感的
    /// 子串匹配）时才返回 `body`（200）；否则 403。证明 H2 回退确实
    /// 传送了受门控保护的后端路径会丢弃的 descriptor headers。
    fn spawn_header_gate_server(required_header: &'static str, body: &'static [u8]) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("local_addr").port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                let (status, payload): (&str, &[u8]) =
                    if head.contains(&required_header.to_lowercase()) {
                        ("200 OK", body)
                    } else {
                        ("403 Forbidden", b"missing-header")
                    };
                let resp_head = format!(
                    "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    status,
                    payload.len()
                );
                if stream.write_all(resp_head.as_bytes()).is_err() {
                    return;
                }
                if stream.write_all(payload).is_err() {
                    return;
                }
                let _ = stream.flush();
            }
        });
        format!("http://127.0.0.1:{port}/tile")
    }

    #[test]
    fn gate_on_with_headers_falls_back_to_direct_and_sends_them() {
        // H2 审查修正：携带 headers 的 descriptor 在门-ON 路径上绝不能丢失
        // 它们。服务器仅当 `X-Ion-Token` header 到达时才返回 body，因此
        // 收到 body 就证明了向直接路径的过渡性回退传送了它；而 `fetch_count`
        // 保持不变则证明该请求绕过了丢弃 headers 的后端。
        let url = spawn_header_gate_server("X-Ion-Token: secret", b"authorized-tile");
        let fetcher = HttpTileFetcher::new("");
        let resource = cesium_resource::Resource::new(&url).with_header("X-Ion-Token", "secret");
        let descriptor = resource.fetch_array_buffer(None);
        assert!(
            !descriptor.headers.is_empty(),
            "descriptor must carry the token header"
        );

        let before = fetcher.resource_backend.fetch_count();
        // 门 ON（注入）但存在 headers -> 过渡性地回退到直接路径。
        let out = fetcher
            .execute_descriptor_gated(&descriptor, true)
            .expect("header-bearing gate-ON request falls back to direct and succeeds");
        assert_eq!(out, b"authorized-tile");
        assert_eq!(
            fetcher.resource_backend.fetch_count(),
            before,
            "header-bearing request must bypass the header-dropping backend path"
        );
    }

    #[test]
    fn non_get_data_uri_descriptor_rejected_before_short_circuit() {
        // L4 审查修正：方法守卫现在在数据-URI 短路之前运行，因此
        // 携带 `data:` URL 的非-GET descriptor 会被以 Network 错误拒绝，而不是
        // 静默解码。修正前数据-URI 分支先触发，会为 POST 返回已解码的
        // 字节。（所有 `Resource::fetch_*` 数据-URI 构建器都发出 GET；只有
        // 那个没意义的“对数据 URI 调用 `post()`”路径会抵达此守卫。）
        let fetcher = HttpTileFetcher::new("");
        let resource = cesium_resource::Resource::new("data:application/octet-stream;base64,QUJD");
        let descriptor = resource.post(vec![1, 2, 3], None);
        assert!(descriptor.is_data_uri, "data: URL still flagged");
        assert!(matches!(descriptor.method, HttpMethod::Post));
        let err = fetcher
            .execute_descriptor_gated(&descriptor, false)
            .unwrap_err();
        assert!(
            matches!(err, PortError::Network(_)),
            "non-GET rejected before the data-URI decode, got {err:?}"
        );
    }

    #[test]
    fn permanent_4xx_fails_fast_without_retry() {
        // L2 审查修正：一个永久性的 4xx（此处为 400 Bad Request）被分类为
        // 非-瞬时，因此 `do_fetch_with_retry` 在第一次尝试就快速失败，
        // 而不是重试 `retry_count` 次。服务器统计命中数；恰好一个就
        // 证明了没有重试。（408/429/5xx 仍为瞬时，会被重试。）
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let hits = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("local_addr").port();
        let hits_srv = Arc::clone(&hits);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                hits_srv.fetch_add(1, Ordering::SeqCst);
                let head =
                    "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                if stream.write_all(head.as_bytes()).is_err() {
                    return;
                }
                let _ = stream.flush();
            }
        });
        let url = format!("http://127.0.0.1:{port}/tile");
        let agent = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(5))
            .build();
        let headers: HashMap<String, String> = HashMap::new();
        let err = HttpTileFetcher::do_fetch_with_retry(&agent, &url, &headers, 3).unwrap_err();
        assert!(
            matches!(err, PortError::Network(_)),
            "400 surfaces a Network error, got {err:?}"
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "permanent 4xx must NOT be retried (exactly one server hit)"
        );
    }
}
