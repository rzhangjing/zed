//! M8 端到端网络骨架的顶层测试框架。
//!
//! Cargo 会将每个 `tests/*.rs` 文件自动发现为一个独立的集成测试
//! crate。本文件是 crate 根，引入 `e2e_network` 模块
//! 目录（仿照现有的 `integration_tests.rs` → `mod integration;`
//! 在本套件其他地方使用的约定）。
//!
//! 内部所有测试均被 `#[ignore]` —— 参见 `docs/deferred.md`（M8-wiremock
//! 骨架）以了解理由与解锁条件。在 M11.1 将真正的
//! `HttpTileFetcher` 接入 `wiremock::MockServer` 之前，
//! 它们仅是骨架。

mod e2e_network;
