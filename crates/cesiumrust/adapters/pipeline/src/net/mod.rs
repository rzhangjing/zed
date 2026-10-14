//! 网络后端抽象 + 实现。
//!
//! 流水线使用一个**同步阻塞池**（ureq，16 个带 keep-alive 的工作线程）
//! 作为默认网络路径，对应 `dynamic_globe.rs:2148-2151`：
//!
//! ```text
//! let agent = ureq::AgentBuilder::new()
//!     .user_agent("Mozilla/5.0 CesiumRust/0.1")
//!     .timeout(Duration::from_secs(10))
//!     .build();
//! ```
//!
//! 一个可选的、基于 `reqwest` 的 async 后端在 `reqwest-backend` feature flag
//! 后可用，供未来集成（M1.4+），但 **tokio 并非流水线的主运行时** ——
//! 它至多是可选 reqwest 后端的一个传递依赖。

pub mod ureq_backend;

#[cfg(feature = "reqwest-backend")]
pub mod reqwest_backend;

use std::time::Duration;

/// 一次网络获取尝试的结果。
#[derive(Debug, Clone)]
pub enum FetchResult {
    /// 带原始字节的成功获取。
    Ok(Vec<u8>),
    /// 瞬时失败（超时、403、429、网络错误）。
    /// 工作线程会根据 `RetryPolicy` 重试。
    Transient(String),
    /// 永久失败（404、无效 URL）。不重试。
    Permanent(String),
}

/// 供工作池使用的网络后端 trait。
///
/// 实现必须是 `Send + Sync`（跨工作线程共享）。默认实现是
/// `UreqBackend`（阻塞、keep-alive 池）。
///
/// 对应 `dynamic_globe.rs:2148-2151` 的 ureq agent 以及 L2192-2203 的
/// 获取循环。
pub trait NetworkBackend: Send + Sync {
    /// 从 URL 获取字节。阻塞直到完成或超时。
    ///
    /// 此函数从工作线程（而非帧线程）调用，因此阻塞是可接受且预期的。
    fn fetch(&self, url: &str) -> FetchResult;

    /// 用于诊断的后端名称。
    fn name(&self) -> &str;

    /// 连接超时（L2150：10 s）。
    fn timeout(&self) -> Duration;
}

/// 由后端测试使用的极简 keep-alive HTTP 服务器（离线、临时端口）。它
/// 追踪接受的 TCP 连接数与已服务请求数，以便测试可以断言连接复用
/// （keep-alive 池化，L2148-2151）。
#[cfg(test)]
pub(crate) mod test_server {
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    /// 由测试观察的计数器。
    pub(crate) struct ServerStats {
        /// 接受的 TCP 连接数（keep-alive ⇒ 少于请求数）。
        pub(crate) connections: AtomicUsize,
        /// 已服务的 HTTP 请求数。
        pub(crate) requests: AtomicUsize,
    }

    /// 一个运行中的测试服务器的句柄。
    pub(crate) struct TestServer {
        /// 服务器基地址（如 `http://127.0.0.1:{port}`），用于拼接请求 URL。
        base: String,
        /// 共享的统计。
        pub(crate) stats: Arc<ServerStats>,
    }

    impl TestServer {
        /// 为 `path`（例如 `/tile`）构造一个完整 URL。
        pub(crate) fn url(&self, path: &str) -> String {
            format!("{}{}", self.base, path)
        }
    }

    /// 启动一个服务器，对每个 GET 都返回 `body`（HTTP 200，keep-alive）。
    pub(crate) fn spawn(body: Vec<u8>) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("local_addr").port();
        let stats = Arc::new(ServerStats {
            connections: AtomicUsize::new(0),
            requests: AtomicUsize::new(0),
        });
        let shared = Arc::clone(&stats);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                shared.connections.fetch_add(1, Ordering::SeqCst);
                let b = body.clone();
                let s = Arc::clone(&shared);
                thread::spawn(move || handle_conn(stream, &b, &s));
            }
        });
        TestServer {
            base: format!("http://127.0.0.1:{port}"),
            stats,
        }
    }

    /// 处理一个 keep-alive 连接：循环读取请求行并每次都返回 `body`（200），
    /// 直至客户端关闭连接（读到 EOF）。
    fn handle_conn(mut stream: TcpStream, body: &[u8], stats: &ServerStats) {
        let reader_stream = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut reader = BufReader::new(reader_stream);
        loop {
            // 读取请求行；EOF（0 字节）意味着客户端已关闭。
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => return,
                Ok(_) => {}
                Err(_) => return,
            }
            // 排空剩余的头部，直到遇到空行终止符。
            loop {
                let mut h = String::new();
                match reader.read_line(&mut h) {
                    Ok(0) => return,
                    Ok(_) => {}
                    Err(_) => return,
                }
                if h == "\r\n" || h == "\n" || h.is_empty() {
                    break;
                }
            }
            stats.requests.fetch_add(1, Ordering::SeqCst);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                body.len()
            );
            if stream.write_all(head.as_bytes()).is_err() {
                return;
            }
            if stream.write_all(body).is_err() {
                return;
            }
            if stream.flush().is_err() {
                return;
            }
        }
    }
}
