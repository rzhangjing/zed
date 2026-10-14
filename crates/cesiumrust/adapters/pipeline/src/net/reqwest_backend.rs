//! 可选的、基于 reqwest 的 async 网络后端。
//!
//! 在 `reqwest-backend` feature flag 后可用。仅将 tokio 作为
//! **传递依赖**使用 —— 流水线的主运行时仍是同步的（std 线程 + mpsc
//! 通道）。reqwest 后端内部在每个工作线程上针对一个专用的 tokio
//! 运行时句柄阻塞。
//!
//! 该后端为未来 M1.4+ 集成而存在，届时 async 提供者（例如流式 3D Tiles
//! 内容）可能从 HTTP/2 多路复用中受益。对于默认的瓦片影像路径，
//! 更推荐 `UreqBackend`（更简单、无 tokio 依赖、久经验证的 keep-alive 池化）。

use std::time::Duration;

use super::{FetchResult, NetworkBackend};

/// 使用 reqwest 并带 rustls-tls 的 async HTTP 后端。
///
/// 内部为从工作线程发起的阻塞调用创建一个单线程的 tokio 运行时。这
/// 并非流水线的主运行时。
pub struct ReqwestBackend {
    /// reqwest 阻塞式 HTTP 客户端（内部包裹一个单线程 tokio 运行时）。
    client: reqwest::blocking::Client,
    /// 单次请求的超时时长。
    timeout: Duration,
}

impl ReqwestBackend {
    /// 创建一个使用默认设置的后端（10 s 超时、rustls-tls）。
    pub fn new() -> Self {
        let timeout = Duration::from_secs(10);
        let client = reqwest::blocking::Client::builder()
            .user_agent("Mozilla/5.0 CesiumRust/0.1")
            .timeout(timeout)
            .build()
            .expect("reqwest client build failed");
        Self { client, timeout }
    }

    /// 创建一个使用自定义超时的后端。
    pub fn with_timeout(timeout: Duration) -> Self {
        let client = reqwest::blocking::Client::builder()
            .user_agent("Mozilla/5.0 CesiumRust/0.1")
            .timeout(timeout)
            .build()
            .expect("reqwest client build failed");
        Self { client, timeout }
    }
}

impl Default for ReqwestBackend {
    /// 默认构造（等价于 [`ReqwestBackend::new`]，10 s 超时）。
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkBackend for ReqwestBackend {
    /// 同步 GET 一个 URL，按结果分类为 [`FetchResult`]：404 为
    /// `Permanent`，非2xx/传输错误为 `Transient`，成功读取为 `Ok`。
    fn fetch(&self, url: &str) -> FetchResult {
        match self.client.get(url).send() {
            Ok(resp) => {
                let status = resp.status();
                if status == reqwest::StatusCode::NOT_FOUND {
                    return FetchResult::Permanent(format!("HTTP 404: {url}"));
                }
                if !status.is_success() {
                    return FetchResult::Transient(format!("HTTP {}: {url}", status.as_u16()));
                }
                match resp.bytes() {
                    Ok(body) => FetchResult::Ok(body.to_vec()),
                    Err(e) => FetchResult::Transient(format!("read error: {e}")),
                }
            }
            Err(e) => {
                // 超时/连接失败归为 `Transient`（可重试）；其余请求级错误同样
                // 保守地归为瞬时，避免因一次抖动就永久丢弃瓦片。
                if e.is_timeout() || e.is_connect() {
                    FetchResult::Transient(format!("transport: {e}"))
                } else {
                    FetchResult::Transient(format!("request: {e}"))
                }
            }
        }
    }

    /// 后端名称标识（固定为 `"reqwest"`）。
    fn name(&self) -> &str {
        "reqwest"
    }

    /// 当前生效的请求超时时长。
    fn timeout(&self) -> Duration {
        self.timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_name_is_reqwest() {
        let b = ReqwestBackend::new();
        assert_eq!(b.name(), "reqwest");
    }

    #[test]
    fn default_timeout_is_10s() {
        let b = ReqwestBackend::new();
        assert_eq!(b.timeout(), Duration::from_secs(10));
    }

    /// 同-K 等价性：对于相同的 URL（瓦片键），ureq 和 reqwest 后端必须返回
    /// 逐字节相同的 `FetchResult::Ok` 载荷。这证明两个后端在
    /// `NetworkBackend` 之后可互换（宿主可以在不改变流水线行为的前提下
    /// 切换后端）。
    #[test]
    fn same_k_ureq_reqwest_equivalent() {
        use crate::net::test_server;
        use crate::net::ureq_backend::UreqBackend;

        let server = test_server::spawn(b"equivalent-tile-payload".to_vec());
        let url = server.url("/tile");

        let ureq_b = UreqBackend::new();
        let rw_b = ReqwestBackend::new();

        let a = ureq_b.fetch(&url);
        let b = rw_b.fetch(&url);

        match (a, b) {
            (FetchResult::Ok(x), FetchResult::Ok(y)) => {
                assert_eq!(
                    x, y,
                    "same-K results must be byte-identical across backends"
                );
                assert_eq!(x, b"equivalent-tile-payload");
            }
            (x, y) => panic!("both backends should succeed: {x:?} / {y:?}"),
        }
    }
}
