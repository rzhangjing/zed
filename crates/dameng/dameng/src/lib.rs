//! # dameng —— 达梦数据库 Rust 中间件
//!
//! 面向 Rust 程序的达梦（DM8）数据库中间件。默认后端 **`dm-dpi`**（FFI 直连达梦官方
//! `dmdpi.dll` / `libdmdpi.so`），也可切回 **`dm-odbc`**（达梦官方 ODBC 驱动
//! `dodbc.dll` / `libdodbc.so`，驱动名 `DM8 ODBC DRIVER`），上层可选挂
//! [Toasty](https://crates.io/crates/toasty) ORM。
//!
//! ## 分层
//!
//! ```text
//! L6 门面      dameng                     ← 本 crate，只做 re-export
//! L5 查询      dm-query  FromRow 事务 批处理
//! L4 池        dm-pool   借还 探活 回收 异步桥接
//! L3 后端      dm-dpi（默认）/ dm-odbc   连接 语句 绑定 取值 诊断 LOB
//! L2 核心      dm-core   配置 连接串 值模型 能力表
//! L1 错误      dm-error  错误分类 诊断记录 重试判定
//! ```
//!
//! 依赖方向严格单向：上层依赖下层，**下层绝不依赖上层**。两个后端实现同一个
//! `dm_core::Backend` trait，切换只改 feature，不改业务代码。
//!
//! ## 里程碑
//!
//! | 里程碑 | 状态 |
//! |---|---|
//! | M0 骨架 / M1 驱动探针 / M2 错误与核心 / M3 ODBC 后端 / M4 连接池 / M5 查询层 / M6 二进制、LOB、批处理、错误码目录 | ✅ 已过门禁 |
//! | M7 `dm-migrate` + 本门面 + 示例 + 文档 | ✅ 已过 G7 |
//! | M8 Toasty 集成（`toasty-driver-dameng` + vendored `toasty-sql`） | ✅ 官方一致性套件 851/851 |
//! | M9 DPI 直连后端（`dm-dpi`，默认后端） | ✅ 官方一致性套件 851/851，`dm-odbc` 保留可回退 |
//!
//! 使用手册见 [`docs/达梦中间件使用手册.md`](../../docs/达梦中间件使用手册.md)，
//! 如实登记的缺口见 `docs/acceptance-report.md` §4。
//!
//! 一条**由达梦驱动行为决定的实现细节**：`Transaction::rollback_to` 在
//! `ROLLBACK TO SAVEPOINT` 之后会补发一条无害语句，否则紧接的 `commit()` 会被驱动
//! 当成「事务已结束」而变成空操作（机制见 `docs/设计方案.md` §12.2）；代价是保存点
//! 回滚多一次往返。
//!
//! ## 用法
//!
//! ```no_run
//! use dameng::prelude::*;
//!
//! # async fn demo() -> Result<(), DmError> {
//! // 1) 用 ODBC 连接串（两个后端都吃这一种写法；DPI 只用其中的主机/端口/用户/口令）
//! let cfg = DmConfig::parse(
//!     "DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA",
//! )?;
//! let _cfg2 = DmConfig::parse("dm://SYSDBA:SYSDBA@localhost:5236/SYSDBA")?;
//!
//! let pool = Pool::connect_with(cfg, ExecutorConfig::for_pool(4)).await?;
//! let mut conn = PooledConn::acquire(&pool).await?;
//! let row = conn
//!     .query_one("select 1 as N from dual", ())
//!     .await?
//!     .expect("select 1 必定返回一行");
//! assert_eq!(row.get::<i32>("N")?, 1);
//!
//! drop(conn);
//! pool.close().await?;
//! pool.wait_closed().await;
//! # Ok(())
//! # }
//! ```
//!
//! 四个可运行示例（`cargo run --example <名字>`，没配 `DM_TEST_DSN`/`DM_SERVER`
//! 时会打印一行原因后退出）：`basic_query`、`pool`、`txn`、`error_handling`。
//!
//! > **`#[derive(FromRow)]` 要求你自己的 crate 也依赖 `dm-query`**：派生宏生成的代码
//! > 引用全限定路径 `::dm_query::…`（与 `serde` 同款做法：不往你的命名空间里塞名字），
//! > 而本门面依赖 `dm-query` 并不会把它放进**你的** extern prelude。不用派生宏则不需要。
//!
//! ## 三条实测约束（用之前务必知道）
//!
//! 1. **达梦不支持 ODBC 异步**（`SQL_ATTR_ASYNC_ENABLE` 报 `HYC00/-70054`），
//!    两个后端都是同步调用 ⇒ 每条连接占住一个阻塞线程，**连接池大小就是并发上限**。
//! 2. **字符集**：DPI 在登录前把会话字符集设成 `PG_UTF8`；ODBC 后端文本走宽字符（UTF-16）。
//!    两者都不该启用 `odbc-api` 的 `narrow` feature。
//! 3. **`varchar(n)` 按字节计量**（`LENGTH_IN_CHAR=0`），`varchar(100)` 只能存 50 个汉字。

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

// ---------------------------------------------------------------------------
// 常用类型的一次性导入
// ---------------------------------------------------------------------------

/// 常用类型的一次性导入：`use dameng::prelude::*;`
///
/// 这里只放**绝大多数程序都要用**的项。需要更细的能力（例如拿 `Fingerprint`
/// 自己组装日志、或直接操作 ODBC 属性）时从 crate 根或 [`raw`] 取。
/// 慢查询告警已内建：`DmConfig::slow_query_threshold`（默认 1 s，`DM_SLOW_QUERY_MS`
/// 可覆盖，`0` = 关闭）。语句缓存也已内建：`DmConfigBuilder::pool_stmt_cache_size(n)`
/// （默认 16，`DM_STMT_CACHE_SIZE` 可覆盖，`without_stmt_cache()` = 关闭），
/// 计数器从 [`PoolGuard::stmt_cache_stats`] 取。
pub mod prelude {
    // 本门面自己的两个短别名：绝大多数程序直接用它们（`DmPool<OdbcBackend>` 写起来太长）
    pub use crate::{DmResult, Pool};
    // L2 核心：配置、值、能力表
    pub use dm_core::{
        Backend, CharsetCheck, ColumnMeta, CompatMode, DataType, DmConfig, DmConfigBuilder,
        DmValue, PoolConfig, RowBuf, SecretString,
    };
    // L1 错误
    pub use dm_error::{DiagRecord, DmError, DmErrorKind, Stage};
    // L3 后端（默认后端就是它）
    pub use dm_odbc::{
        ChunkState, LOB_CHUNK_BYTES, LobRead, LobState, LobWriter, OdbcBackend, OdbcConnection,
        OdbcStatement,
    };
    // L4 池
    pub use dm_pool::{
        DefaultBackend, DmPool, ExecutorConfig, PoolGuard, PoolState, PoolStats, StmtCacheStats,
    };
    // L5 查询：RowSet/Row/RowStream、参数、事务、批处理、派生宏
    pub use dm_macros::FromRow;
    pub use dm_query::{
        Args, BatchOutcome, ColumnIndex, Decode, FromRow, IntoArgs, IsolationLevel, Level,
        PooledConn, Row, RowSet, RowStream, Transaction,
    };
    // 迁移入口（不用 Toasty 的用户用它）
    pub use dm_migrate::{AppliedMigration, MigrationFile, Migrator};
}

// ---------------------------------------------------------------------------
// 冻结面（设计方案 §6.12）：prelude + DmPool/DmConfig/DmError/Row/FromRow
// ---------------------------------------------------------------------------

pub use dm_core::{
    Backend, CharsetCheck, ColumnMeta, CompatMode, DEFAULT_DRIVER, DEFAULT_PORT, DataType,
    DmConfig, DmConfigBuilder, DmValue, PoolConfig, RowBuf, SecretString, parse_odbc_string,
    render_odbc_string,
};
pub use dm_error::{DiagRecord, DmError, DmErrorKind, Stage};
pub use dm_macros::FromRow;
pub use dm_migrate::{AppliedMigration, MigrationFile, Migrator};
pub use dm_odbc::{
    ChunkState, LOB_CHUNK_BYTES, LobRead, LobState, LobWriter, OdbcBackend, OdbcConnection,
    OdbcStatement,
};
pub use dm_pool::{
    DefaultBackend, DmPool, ExecutorConfig, PoolGuard, PoolState, PoolStats, StmtCacheStats,
};
#[cfg(feature = "rw-split")]
pub use dm_pool::{Router, RouterStats, RwMode, RwStrategy};
pub use dm_query::{
    Args, BatchOutcome, ColumnIndex, Decode, Fingerprint, FromRow, IntoArgs, IsolationLevel, Level,
    PooledConn, Row, RowSet, RowStream, Transaction, normalize,
};

/// 默认后端下的池：`DmPool<DefaultBackend>` 的短名字。
///
/// [`DmPool`] 对后端泛型（离线的假后端因此可以复用整个池）。默认后端由 feature 决定：
/// 开着 `dpi`（默认）就是 `dm_dpi::DpiBackend`（直连 `dmdpi.dll`），
/// `--no-default-features --features "chrono decimal"` 则退回 `dm_odbc::OdbcBackend`。
/// 两种情况下本别名都指向 [`DefaultBackend`]，所以业务代码不用改。
pub type Pool = DmPool<DefaultBackend>;

/// 本 crate 的 `Result` 别名：错误恒为 [`DmError`]。
pub type DmResult<T> = std::result::Result<T, DmError>;

/// 门面把 `tracing` 原样透出：配置日志时不必自己再挂一个依赖。
pub use tracing;

// ---------------------------------------------------------------------------
// 底层 crate 的直接入口
// ---------------------------------------------------------------------------

/// 各层的直接入口。
///
/// 门面只做 re-export，**不隐藏**下层：需要用到较高层能力（例如自己构造 ODBC
/// 属性、或直接用 `dm_pool` 的统计埋点）时走这里。这里刻意把 crate 原名透出来
/// （`raw::odbc` 而不是 `raw::dm_odbc`），因为 `dm_` 前缀在本 crate 的语境里是冗余的。
pub mod raw {
    /// L2 核心：配置、值模型、能力表、`Backend` trait。
    pub use dm_core as core;
    /// L1 错误：`DmError` 家族的分类与诊断。
    pub use dm_error as error;
    /// 派生宏（`#[derive(FromRow)]`）。
    pub use dm_macros as macros;
    /// L5 迁移：脚本发现与执行、版本记录表。
    pub use dm_migrate as migrate;
    /// L3 后端：ODBC 连接/语句/绑定/取值/LOB。
    pub use dm_odbc as odbc;
    /// L4 池：状态机、线程池、统计、读写分离路由。
    pub use dm_pool as pool;
    /// L5 查询：`RowSet`/`RowStream`/事务/批处理。
    pub use dm_query as query;
}
