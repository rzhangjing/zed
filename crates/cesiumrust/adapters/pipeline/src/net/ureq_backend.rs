//! 基于 ureq 的同步网络后端（默认）。
//!
//! 照搬 `dynamic_globe.rs:2148-2151`：
//! ```text
//! let agent = ureq::AgentBuilder::new()
//!     .user_agent("Mozilla/5.0 CesiumRust/0.1")
//!     .timeout(Duration::from_secs(10))
//!     .build();
//! ```
//!
//! ureq agent 维护一个带 keep-alive 的连接池，因此瓦片服务器的连接会在
//! 多次获取之间保持热度（L2140-2142 文档注释）。这是流水线工作池的
//! **默认且推荐**后端。

use std::io::Read;
use std::time::Duration;

use super::{FetchResult, NetworkBackend};

/// 使用 ureq 并带 keep-alive 连接池化的阻塞式 HTTP 后端。
///
/// 线程安全：ureq 的 `Agent` 内部就是 `Send + Sync`，并跨线程管理
/// 自己的连接池。
pub struct UreqBackend {
    /// ureq 客户端 Agent（自带 `Send + Sync` 与跨线程 keep-alive 连接池）。
    agent: ureq::Agent,
    /// 单次请求的超时时长。
    timeout: Duration,
}

impl UreqBackend {
    /// 创建一个使用与 dynamic_globe.rs 一致的默认设置的后端：
    /// - User-Agent："Mozilla/5.0 CesiumRust/0.1"
    /// - 超时：10 秒
    /// - Keep-alive：启用（ureq 默认）
    pub fn new() -> Self {
        let timeout = Duration::from_secs(10);
        let agent = ureq::AgentBuilder::new()
            .user_agent("Mozilla/5.0 CesiumRust/0.1")
            .timeout(timeout)
            .build();
        Self { agent, timeout }
    }

    /// 创建一个使用自定义超时的后端。
    pub fn with_timeout(timeout: Duration) -> Self {
        let agent = ureq::AgentBuilder::new()
            .user_agent("Mozilla/5.0 CesiumRust/0.1")
            .timeout(timeout)
            .build();
        Self { agent, timeout }
    }
}

impl Default for UreqBackend {
    /// 默认构造（等价于 [`UreqBackend::new`]，10 s 超时）。
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkBackend for UreqBackend {
    /// 同步 GET 一个 URL，按结果分类为 [`FetchResult`]：成功读取为 `Ok`，
    /// 404 为 `Permanent`，其余状态码/传输错误为 `Transient`。
    fn fetch(&self, url: &str) -> FetchResult {
        match self.agent.get(url).call() {
            Ok(resp) => {
                let status = resp.status();
                let mut reader = resp.into_reader();
                let mut data = Vec::new();
                match reader.read_to_end(&mut data) {
                    Ok(_) => FetchResult::Ok(data),
                    Err(e) => FetchResult::Transient(format!(
                        "read error (status {status}): {e}"
                    )),
                }
            }
            Err(ureq::Error::Status(code, _)) => {
                // 403/429 = 限速（瞬时的，带退避重试）
                // 404 = 永久（瓦片不存在）
                if code == 404 {
                    FetchResult::Permanent(format!("HTTP 404: {url}"))
                } else {
                    FetchResult::Transient(format!("HTTP {code}: {url}"))
                }
            }
            Err(ureq::Error::Transport(e)) => {
                // 超时、连接重置、DNS 失败 —— 都是瞬时的
                FetchResult::Transient(format!("transport: {e}"))
            }
        }
    }

    /// 后端名称标识（固定为 `"ureq"`）。
    fn name(&self) -> &str {
        "ureq"
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
    fn backend_name_is_ureq() {
        let b = UreqBackend::new();
        assert_eq!(b.name(), "ureq");
    }

    #[test]
    fn default_timeout_is_10s() {
        let b = UreqBackend::new();
        assert_eq!(b.timeout(), Duration::from_secs(10));
    }

    #[test]
    fn custom_timeout() {
        let b = UreqBackend::with_timeout(Duration::from_secs(5));
        assert_eq!(b.timeout(), Duration::from_secs(5));
    }

    /// 工作线程连接复用（L2148-2151 keep-alive）：单个 ureq `Agent` 必须
    /// 在连续的获取之间池化 TCP 连接，因此服务器观察到的连接数会
    /// 少于请求数。
    #[test]
    fn keepalive_reuses_connection() {
        use crate::net::test_server;
        use std::sync::atomic::Ordering;

        let server = test_server::spawn(b"tiledata".to_vec());
        let backend = UreqBackend::new();
        let url = server.url("/tile");

        for _ in 0..5 {
            match backend.fetch(&url) {
                FetchResult::Ok(bytes) => assert_eq!(bytes, b"tiledata"),
                other => panic!("expected Ok, got {other:?}"),
            }
        }

        let reqs = server.stats.requests.load(Ordering::SeqCst);
        let conns = server.stats.connections.load(Ordering::SeqCst);
        assert_eq!(reqs, 5, "all 5 requests must reach the server");
        assert!(
            conns < reqs,
            "keep-alive must reuse connections (conns={conns}, reqs={reqs})"
        );
    }
}
