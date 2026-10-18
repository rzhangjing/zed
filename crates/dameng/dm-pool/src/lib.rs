//! # dm-pool —— 连接池与异步↔阻塞桥接（L4）
//!
//! 计划中的模块（设计方案 §5.4、§5.7、§6.9、§6.10、§12 M4）：
//!
//! | 文件 | 职责 |
//! |---|---|
//! | `exec.rs` | **`BlockingExecutor`**：专用线程池 + 有界队列 + `oneshot` 回传 + 队列深度指标 |
//! | `inner.rs` | 池核心状态机：信号量门控、空闲栈（LIFO）、单飞建连、超时、待复位队列、`close`/`wait_closed` |
//! | `conn.rs` | `ConnInner`（共享核心）、`PoolGuard`（drop 自动归还）、取消语义（**D11**） |
//! | `health.rs` | 探活（`SQL_ATTR_CONNECTION_DEAD`）与节流 |
//! | `stats.rs` | `PoolStats`/`PoolState` + `tracing` 埋点 |
//! | `stmt_cache.rs` | 每条连接一份的 LRU 预处理语句缓存（CAP-C-11） |
//! | `router.rs` | 读写分离路由（feature `rw-split`），与驱动内建 `RW_SEPARATE` 二选一 |
//!
//! 复位与后台维护（原计划的 `recycle.rs`）落在 `inner.rs`：它们都是
//! `PoolInner` 的私有状态机动作，拆出去要把字段全开成 `pub(crate)`，
//! 换来的是更松的封装而不是更清楚的边界。
//!
//! ## 为什么必须有专用线程池（而不是 `spawn_blocking`）
//!
//! ODBC 是同步 API，达梦又不支持异步（§4.5），所以每个在做事的连接都占住一个线程。
//! 用 `tokio::task::spawn_blocking` 有两个致命问题：
//!
//! - 阻塞线程数不可控，池上限形同虚设（`max_size` 必须 ≤ 阻塞线程数，§5.7）；
//! - `spawn_blocking` 的任务**无法被取消**，`abort` 对已开始执行的任务无效。
//!
//! 因此这里自建线程池，并用有界队列做背压。**池大小就是 ODBC 的并发上限。**

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]
// 持有锁跨 await ⇒ 直接编译失败（设计方案 §5.8）。本 crate 是唯一有锁又有 await 的地方。
#![deny(clippy::await_holding_lock)]
#![deny(clippy::await_holding_invalid_type)]

pub mod exec;

mod conn;
mod health;
mod inner;
#[cfg(feature = "rw-split")]
pub mod router;
pub mod stats;
mod stmt_cache;

/// 默认后端。
///
/// 默认是 DPI（`dm_dpi::DpiBackend`，FFI 直连 `dmdpi.dll` / `libdmdpi.so`，不经过
/// ODBC 驱动）；关掉 `dpi` feature（`--no-default-features`）退回
/// `dm_odbc::OdbcBackend`。
///
/// 用它而不是写死 `OdbcBackend`：`DmPool` / `PoolGuard` / `Router` /
/// `dm_query::PooledConn` 的默认类型参数**全部**指到这里，切后端只改一个 feature。
#[cfg(not(feature = "dpi"))]
pub type DefaultBackend = dm_odbc::OdbcBackend;
/// 默认后端（`dpi` 是默认 feature，因此这就是 `dm_dpi::DpiBackend`）。
#[cfg(feature = "dpi")]
pub type DefaultBackend = dm_dpi::DpiBackend;

pub use conn::{ConnStatsSnapshot, PoolGuard};
pub use exec::{BlockingExecutor, ExecutorConfig, JobReply};
pub use inner::DmPool;
#[cfg(feature = "rw-split")]
pub use router::{Router, RouterStats, RwMode, RwStrategy};
pub use stats::{PoolState, PoolStats};
pub use stmt_cache::StmtCacheStats;

// 池参数定义在 `dm-core`（`DmConfig` 要内嵌它，依赖方向是 dm-core ← dm-pool），
// 这里原样转出去，让调用方只依赖 `dm-pool` 就能配池。
pub use dm_core::PoolConfig;
