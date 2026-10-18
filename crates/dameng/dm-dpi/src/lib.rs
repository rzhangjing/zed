//! 达梦 DPI 后端：直连 `dmdpi.dll` / `libdmdpi.so` 的 FFI 绑定。
//!
//! # 为什么会有这个 crate
//!
//! 达梦的 ODBC 驱动 `dodbc.dll` 本身就是 DPI 之上的一层壳（二进制里能搜到
//! `dmdpi.dll` 的导入）。走一层 ODBC 要多付：每列的 `SQLGetData` 一次往返、
//! 每次取值的驱动内部状态机、以及一层把诊断码转成 SQLSTATE 的映射。
//! 实测（见 `docs/性能测试报告.md`）宽表随机读 250 µs 里绝大部分是
//! 100 列逐列取值的固定开销，这层壳正是开销所在。
//!
//! # 分层
//!
//! | 模块 | 职责 |
//! | --- | --- |
//! | `sys` | 运行时加载共享库（`LoadLibraryW` / `dlopen`），不产生链接期依赖 |
//! | [`ffi`] | 常量与导出函数原型，逐条对自 `DPI.h` |
//! | [`handle`] | 句柄 RAII + 诊断，安全接口 |
//! | `backend` | 实现 `dm_core::backend::Backend`，与 `dm-odbc` 等价（后续） |
//!
//! # 现在能做什么
//!
//! 连接、执行 SQL、预处理、绑定参数、取列描述、逐行取值（含 UTF-8 / UTF-16
//! 文本、`i64` / `f64`、时间戳）、切换当前模式、读连接属性、诊断消息。
//! LOB、批量、事务对象等按阶段补齐（见 `docs/设计方案.md` §15）。
//!
//! # 示例
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let dpi = dm_dpi::Dpi::open()?;
//! let env = dpi.env()?;
//! env.set_local_code(dm_dpi::UTF8)?;
//! let conn = env.connect()?;
//! conn.login("127.0.0.1:5236", "SYSDBA", "SYSDBA")?;
//! let stmt = conn.stmt()?;
//! stmt.exec_direct("select 1 from dual")?;
//! while stmt.fetch()? {
//!     println!("{:?}", stmt.column_i64(1)?);
//! }
//! # Ok(())
//! # }
//! ```

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(rustdoc::broken_intra_doc_links)]

mod sys;

pub mod backend;
pub mod error;
pub mod ffi;
pub mod handle;

pub use backend::{DpiBackend, DpiStmt};
pub use error::{DpiError, LoadError};
pub use ffi::Api;
pub use handle::{ColumnDesc, Conn, Dpi, Env, ParamDesc, ReadChunk, Stmt, Timestamp, UTF8};
