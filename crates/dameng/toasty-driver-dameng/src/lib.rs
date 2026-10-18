//! # toasty-driver-dameng —— Toasty 的达梦驱动（M8）
//!
//! 目标是把 `toasty_core::driver::Driver` / `Connection` 实现到达梦上，
//! **搭在自研中间件之上**（设计方案 §9.6 已定案：池不外包给 Toasty）。
//!
//! ## 关键约束（全部有实测或源码依据，见设计方案 §4.6、§5.2、§9）
//!
//! 1. **`Dialect` 是闭枚举**（`Sqlite | Postgresql | Mysql | MariaDb`），且
//!    `toasty_sql::Serializer` 的 `dialect` 字段私有、构造函数只有 5 个，
//!    没有泛型的 `Serializer::new(schema, dialect)`。
//!    ⇒ **外部 crate 无法让渲染器说 DM 方言**，必须 fork `toasty-sql` 加一个
//!    `pub fn dameng(schema) -> Serializer`。这正是路线「乙」。
//! 2. **fork 面压到 1 个 crate（T8.0 已结案，见 `docs/设计方案.md` §9.1.1）**：
//!    引擎虽然重度读 `Capability`，但按**方言身份**（`Dialect` 变体值）分支的只有
//!    `engine/plan/statement.rs:1036` 一处 —— 那一处恰好是达梦必须走的路
//!    （`returning_from_insert = false` 时，只有 `Dialect::Mysql` 身份能让规划器接受
//!    「单行 insert + 靠回查拿自增主键」）。所以 `Capability.sql` 填
//!    `Some(Dialect::Mysql)`，**不是** `Sqlite`；`toasty-core` / `toasty` 都用上游版本。
//!    渲染侧的「达梦方言」由 fork 内部的本地枚举承担（`Dialect` 加不了变体）。
//! 3. **驱动侧要做两件事**：`Operation` 分发（11 个变体 + `Transaction` 的 **6 个变体**，
//!    `QuerySql` 装的是 `stmt::Statement` AST 而不是渲染好的 SQL），
//!    以及 `stmt::Value ↔ db::Type` 编解码（二进制列模板见
//!    `toasty-driver-mysql` 的 `value.rs`，**原始字节直进参数缓冲，无 base64/hex**）。
//! 4. **`Connection` 不含 `Sync`**，且 `is_valid()` 明确禁止阻塞/IO。
//! 5. **错误契约**：连接类故障必须归入 `Error::connection_lost`，否则会
//!    「把死连接漏回连接池」。
//! 6. **`dameng://` URL 走不通**：`toasty` 的 URL→驱动分派是闭 `match`，
//!    第三方驱动无法注册 scheme，必须用 `Db::builder()` 喂 `&dyn Driver`。
//! 7. **加 `Dialect::Dameng` 变体后要查两类 match**：穷举 4 方言的会编译报错
//!    （编译器会替我们列清单），带 `_ => todo!()` 的会**静默 panic**——
//!    必须执行 `grep -n "_ => todo!" toasty-sql/src/`。
//!
//! MSRV 由 Toasty 决定为 **1.95**（设计方案 §2）。

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod capability;
pub mod conn;
pub mod driver;
pub mod error;
pub mod value;

/// `Vec<scalar>` 的 JSON 文本编解码（T8.12）：只在 `value.rs` 内部用，不进公开 API。
mod list_json;

pub use crate::conn::DamengConnection;
pub use crate::driver::DamengDriver;
