//! L3 真机集成测试（设计方案 §10.3）。
//!
//! 入口在这里是因为 Cargo 把 `tests/integration/` 目录当成**一个**测试目标
//! （`cargo test -p dm-odbc --test integration`）——所有子模块共用一个二进制，
//! 于是 `common` 里的 `-70027` 计数器和夹具也就能跨文件共享。
//!
//! **全部用例都带 `#[ignore]`**：它们需要一台真的达梦。跑法见 [`common`] 的模块文档。

mod common;

mod backend;
mod binary;
mod catalog;
mod charset;
mod ct;
mod errors;
mod lob;
mod params;
mod result;
mod smoke;
mod stmt_cache;
mod txn;
mod types_roundtrip;
