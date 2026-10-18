//! # dm-query —— 查询层（L5，设计方案 §6.7 / §6.12 / §12 M5）
//!
//! | 文件 | 职责 |
//! |---|---|
//! | [`row`] | `Row`：按名字/序号取值，`get::<T>()` 类型不匹配**报错而非静默转换** |
//! | [`rowset`] | `RowSet`：`rows` + `affected`（区分「无匹配行」与「非查询语句」） |
//! | [`stream`] | `RowStream`：`futures_core::Stream`，分批取数 |
//! | [`lob`] | `LobRef`/`LobStream`：大 LOB **按块读**（`get::<Vec<u8>>` 是全量载入） |
//! | [`args`] | `Args`/`IntoArgs`：参数化入口，**一次 query 只跨线程桥接一次** |
//! | [`from_row`] | `FromRow` trait（配合 `dm-macros` 的 `#[derive(FromRow)]`） |
//! | [`conn`] | `PooledConn`：`query`/`execute`/`execute_many`/`execute_lob`/`fetch_stream`/`lob_stream`/`begin` |
//! | `batch`（私有） | 批量写的参数转置与形状校验：行优先 → 列式（§4.7 数组绑定的输入） |
//! | [`txn`] | `Transaction`：**`Drop` 时未提交自动回滚** |
//! | [`fingerprint`] | SQL 指纹：进日志的 SQL 去掉字面量，**不含参数值** |
//!
//! ## 冻结的语义（接口冻结清单，设计方案 §6.12）
//!
//! 1. `query` 对 DDL/DML **不报错**，返回 `rows: []` + `affected: Some(n)`；
//! 2. `ColumnIndex for &str` 大小写不敏感；
//! 3. `get::<T>()` 类型不匹配报 `Decode { column }`，**绝不静默返回默认值**；
//! 4. `NULL` → 非 `Option` 报 `NullViolation`，并带上列名；
//! 5. 事务 `Drop` 未提交**自动回滚**；
//! 6. 取消/超时后的连接**销毁不复用**（逃生舱 `reuse_after_cancel`）。
//!
//! ## 本层的两条实现纪律
//!
//! - **一次语句执行的 ODBC 调用全在同一个阻塞闭包内**（§5.4 纪律 1）：
//!   `prepare → bind → execute → fetch… → close_cursor` 不许拆成多次跨线程往返，
//!   否则每步都要重新取锁、重新进入阻塞线程，代价随语句数线性放大。
//! - **参数与结果只在两个边界上复制一次**：参数进闭包前收集成 `Vec<DmValue>`，
//!   结果出闭包后搬成 `Row`；中间不许再出现第三份。

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod args;
mod batch;
pub mod conn;
pub mod fingerprint;
pub mod from_row;
pub mod lob;
pub mod row;
pub mod rowset;
pub mod stream;
pub mod txn;

pub use args::{Args, IntoArgs};
pub use conn::PooledConn;
pub use fingerprint::{Fingerprint, normalize};
pub use from_row::FromRow;
pub use lob::{LobRef, LobStream};
pub use row::{ColumnIndex, Decode, Row};
pub use rowset::{BatchOutcome, RowSet};
pub use stream::RowStream;
pub use txn::{IsolationLevel, Level, Transaction};

/// 缓冲式 LOB 写入器（冻结清单 §6.12 把它记在 `dm-query` 上）。
///
/// 实现留在 `dm-odbc`（它本来就是驱动层的「缓冲式写」），这里只是把它摆到冻结清单说的
/// 位置上：`dm-query` 的调用方不必为了写一个大值去认识 `dm-odbc`。
pub use dm_odbc::lob::LobWriter;

/// 分块读的状态码（[`LobStream::read_chunk`] 的返回值）。
///
/// 由 `dm-core` 定义（它是 `Backend::lob_chunk` 的返回类型），这里转出来是为了让调用方
/// 用 `dm_query::LobChunk` 就能配平 `match`，不必再依赖 `dm-core`。
pub use dm_core::LobChunk;

/// 分块**写**（[`PooledConn::execute_lob`]）的公开面：值类型、数据源 trait、参数描述
/// 与两个现成源。
///
/// 它们由 `dm-core` 定义（`Backend::execute_lob` 的入参形状就在那里），这里转出来是为了
/// 让写一个大值的调用方只需要认识 `dm-query`——同 [`LobChunk`] 的理由。
///
/// - [`LobSource`]：自己的数据从哪来（`next_chunk` 追加语义）；
/// - [`LobBytes`]：值已经在内存里（`Vec<u8>`/`&[u8]`/`String` 都收）；
/// - [`LobReader`]：从 `std::io::Read` 流式读（真正的流式写用它）；
/// - [`LOB_WRITE_CHUNK_BYTES`]：一趟交付的上限，也给出「实现一个源时一趟最多追加多少」。
pub use dm_core::{LOB_WRITE_CHUNK_BYTES, LobBytes, LobKind, LobParam, LobReader, LobSource};

/// 把 `PoolConfig` 转成 `dm-query` 侧常用的借用形态（便利重导出，避免调用方再多一个依赖）。
pub use dm_core::PoolConfig;
/// 默认后端：默认就是 `dm_dpi::DpiBackend`（DPI+FFI 直连）；关掉 `dpi` feature
/// 退回 `dm_odbc::OdbcBackend`。
pub use dm_pool::DefaultBackend;

/// 错误类型与诊断记录的便利重导出。
///
/// `dm-macros` 的 `#[derive(FromRow)]` 生成代码里要用到它们，而生成代码引用的是
/// `::dm_query::…`——所以这里必须导出，否则用户除了 `dm-query` 还得手工把 `dm-error`
/// 挂到依赖里，派生宏才能编译。
pub use dm_error::{DiagRecord, DmError, DmErrorKind, Stage};
