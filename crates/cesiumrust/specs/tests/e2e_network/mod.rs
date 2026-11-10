//! M8 端到端网络测试骨架。
//!
//! **状态：仅骨架。** 本模块中的每个测试均依据计划 L206
//!（"先建骨架标 #[ignore]"）被 `#[ignore]`。它们的存在是为了在适配器
//! 接线落地前锁定网络路径的预期端到端形态——针对
//! `wiremock::MockServer` 驱动 `HttpTileFetcher`，演练域的 `retry_callback` 与
//! `priority_function` 语义。
//!
//! # 为何忽略（参见 docs/deferred.md："M8-wiremock skeleton"）
//!
//! 1. `wiremock::MockServer` 需要异步运行时（`#[tokio::test]`）。
//!    cesium-specs 的 dev-profile 尚未为本次套件启用 tokio 的 `macros`/`rt`
//!    特性，且生产路径刻意
//!    不依赖 tokio（ureq 阻塞池）。接入异步测试框架是
//!    M11.1 的事项。
//! 2. `adapters/network` 的 `HttpTileFetcher::fetch(url, _priority)` 目前
//!    会*忽略* `_priority`（参见 `adapters/network/src/lib.rs:186`）。优先级端到端
//!    断言只有在 M8.3 (#66) 让获取器消费
//!    它、且 M11.1 接入调度器后才成立。
//!
//! # 今天仍能运行的部分
//!
//! 每个测试中的**域侧**断言（RetryPolicy 决策、
//! PriorityFunction 排序、FetchDescriptor 构造）是纯函数，即使没有
//! 服务器也能通过；它们是在 `cesium-resource` 中已实现的
//! 部分。wiremock 的 `Mock`/`ResponseTemplate` 构造是
//! 同步的且现在即可编译，证明 dev-dependency 可解析；只有
//! 异步的 `.mount(&server)` + 真正的获取被推迟。

pub mod test_http_tile_fetcher;
pub mod test_priority;
pub mod test_retry;
