//! # dm-odbc —— 达梦 ODBC 后端（L3）
//!
//! 计划中的模块（设计方案 §6.2–§6.6）：
//!
//! | 文件 | 职责 |
//! |---|---|
//! | `conn.rs` | `OdbcConnection::connect`（8 步：驱动注册校验 → `Environment` → `SQLDriverConnect` → 属性 → **`unicode()` 自检** → `init_sql`） |
//! | `diag.rs` | 多记录诊断收集，`SQLSTATE` + 原生码 + 列/行号 + **宽字符消息** |
//! | `stmt.rs` | `prepare`/`exec_direct`/`exec`/`StmtKind`/游标关闭/`number_params` |
//! | `param.rs` | 参数绑定（**D8 纪律**）：`DmValue` → `(ctype, dtype, column_size, buf, ind)` |
//! | `result.rs` | 列描述、两段式取值、**截断重取**、`FetchState`、多结果集、`ColumnarBuffer` |
//! | `lob.rs` | 二进制与 LOB；大 BLOB 走延迟输入（`SQLPutData` / `DelayedInput`） |
//! | `txn.rs` | 事务与保存点 |
//!
//! ## 关于 `unsafe`
//!
//! 本 crate 是全项目**唯一**允许出现 `unsafe` 的地方（大 BLOB 分块读若需下探
//! `odbc-sys` 裸 API），因此这里刻意不写 `#![forbid(unsafe_code)]`；
//! 每个 `unsafe` 块都必须有 SAFETY 注释说明前置条件。
//!
//! ## 两条实测约束（务必遵守，见设计方案 §4）
//!
//! 1. **绝不启用 `odbc-api` 的 `narrow` feature**：Windows 上会把 `SqlChar` 变成 `u8`，
//!    中文写入即乱码、`SQL_DBMS_NAME` 直接 panic。
//! 2. **达梦不支持 ODBC 异步**：`SQL_ATTR_ASYNC_ENABLE` 一律返回 `HYC00/-70054`
//!    （`SQL_ASYNC_MODE = SQL_AM_NONE`）。因此本层是纯同步的，异步桥接在 `dm-pool`。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations)]

pub mod conn;
pub mod diag;
pub mod lob;
pub mod param;
pub mod result;
pub mod stmt;

pub use conn::{ConnStats, OdbcConnection, SUPPORTED_SERVER_UNICODE, UNICODE_GB18030};
pub use lob::{ChunkState, LOB_CHUNK_BYTES, LobFeeder, LobRead, LobState, LobWriter};
pub use param::{
    ParamArena, ParamArrayArena, ParamArrayColumnPlan, ParamArraySummary, ParamMeta,
    bind_param_array, bind_params, bind_params_with_lob, reset_param_array_attrs,
};
pub use result::{FetchScratch, describe_columns, fetch};
pub use stmt::{OdbcStatement, StmtKind, StmtOutcome};

use std::sync::Arc;

use dm_core::backend::{
    Backend, ExecOutcome, FetchState, LobChunk, LobParam, LobSource, ParamArrayOutcome, RowBuf,
};
use dm_core::{ColumnMeta, DmConfig, DmValue};
use dm_error::{DmError, DmErrorKind, Stage};

// ---------------------------------------------------------------------------
// T3.6：Backend 实现
// ---------------------------------------------------------------------------

/// 达梦 ODBC 后端：把 [`OdbcConnection`] 与语句/参数/结果集三层粘成
/// [`dm_core::backend::Backend`]。
///
/// # 为什么这里只有连接，没有语句表
///
/// `Backend::prepare` 把语句**交出去**（`type Stmt = OdbcStatement<'static>`），
/// 由调用方（`dm-pool` 的语句缓存）持有。所以本类型不需要、也不应该同时拥有连接与语句：
/// 交出去之后它管不了别人的析构顺序，那条「语句先于连接释放」（CT-09）就落在调用方头上。
/// 能做的两件事都做了：
///
/// 1. 抽出来的句柄是 `'static` 的，但**连接侧记着活语句数**
///    （[`OdbcConnection::live_statement_count`]），断连时若不为零会 `warn!`——
///    违规从「某天在已释放的父句柄上 `SQLFreeStmt`」变成一条当场可见的日志；
/// 2. 本类型的 `close` 在确认式关闭前先提示这一条。
///
/// # 复用
///
/// [`ParamArena`] 与 [`FetchScratch`] 挂在后端上**跨语句复用**：这是 §6.3 纪律 5
/// 「稳态零分配」的落点——它们不是每次绑定/取行都新建的临时缓冲。
#[derive(Debug)]
pub struct OdbcBackend {
    conn: OdbcConnection,
    /// 参数缓冲。上一趟的指针已经交给驱动，所以这里**只增不减**地复用容量
    /// （见 [`ParamArena::clear`] 的文档）。
    arena: ParamArena,
    /// 数组绑定的列式缓冲（T6.4）。与 [`OdbcBackend::arena`] 一样跨语句复用：
    /// 它按 2 的幂增长，所以同一条连接上反复批插不会再分配。
    param_array: ParamArrayArena,
    /// 取值暂存（CLOB/二进制试探块）。
    scratch: FetchScratch,
}

impl OdbcBackend {
    /// 底层连接（事务、统计、字符集自检结果都在这上面）。
    pub fn connection(&self) -> &OdbcConnection {
        &self.conn
    }

    /// 可变连接。
    pub fn connection_mut(&mut self) -> &mut OdbcConnection {
        &mut self.conn
    }

    /// 保证语句上的列元数据已经描述过，并返回它。
    ///
    /// [`Backend::execute`] 在产出结果集时已经描述过一次；这里的缓存路径让
    /// [`Backend::fetch`] 每取一行都不必重新问驱动一遍列属性。
    fn ensure_columns(
        &mut self,
        stmt: &mut OdbcStatement<'static>,
    ) -> Result<Arc<[ColumnMeta]>, DmError> {
        if let Some(columns) = stmt.columns() {
            return Ok(Arc::clone(columns));
        }
        // `Some(false)`：`SQL_DESC_OCTET_LENGTH` 的单位是字节（见 `result.rs` 模块文档）。
        let length_in_char = self.conn.config().length_in_char.unwrap_or(false);
        describe_columns(stmt, Some(length_in_char))
    }
}

impl Backend for OdbcBackend {
    type Stmt = OdbcStatement<'static>;

    fn name(&self) -> &'static str {
        "dm-odbc"
    }

    fn connect(cfg: &DmConfig) -> Result<Self, DmError> {
        Ok(Self {
            conn: OdbcConnection::connect(cfg)?,
            arena: ParamArena::new(),
            param_array: ParamArrayArena::new(),
            scratch: FetchScratch::new(),
        })
    }

    fn ping(&mut self) -> Result<bool, DmError> {
        self.conn.ping()
    }

    fn prepare(&mut self, sql: &str) -> Result<Self::Stmt, DmError> {
        let mut stmt = self.conn.allocate_statement_owned()?;
        stmt.prepare(sql)?;
        Ok(stmt)
    }

    fn bind_params(&mut self, stmt: &mut Self::Stmt, args: &[DmValue]) -> Result<(), DmError> {
        bind_params(stmt, args, &mut self.arena)
    }

    /// `SQLNumParams` 在 `prepare` 时已经问过一次并记在语句上，这里直接取。
    ///
    /// 有它之后，「某组参数少了一个」的报错会带上「语句有几个 `?`」这一半信息，
    /// 调用方拿到的就不是驱动那句离定位很远的话术（见 [`Backend::param_count`]）。
    fn param_count(&mut self, stmt: &Self::Stmt) -> Option<u16> {
        Some(stmt.param_count())
    }

    /// 列式参数数组绑定（T6.4，§4.7）：一次 `SQLExecute` 下发 `N` 组参数。
    ///
    /// # 这里的「列」就是「参数位」
    ///
    /// `columns[p][r]` 是第 `r` 组参数的第 `p` 个值，所以 `columns.len()` 必须等于语句的
    /// `?` 个数、每个 `columns[p]` 的长度相同（行数）。形状校验在 [`bind_param_array`]
    /// 里做，这里不重复。
    ///
    /// # 失败的分层（§6.7 的记账口径靠这一层区分）
    ///
    /// * **语句级**失败（prepare 没做、形状不对、`SQLExecute` 返回错误码、绑定失败）→ `Err`；
    /// * **行级**失败（参数状态数组里第 `i` 行是 `SQL_PARAM_ERROR`）→ `Ok`，但
    ///   `ParamArrayOutcome::failed_at = Some(i)`。
    ///
    /// 行级失败不是错误而是结果：`execute_many` 的契约要求把它记进 `BatchOutcome`
    /// （「不抹掉已生效的前几组」），在中间变成 `Err` 就会丢掉「前几组已经生效」这个事实。
    fn execute_param_array(
        &mut self,
        stmt: &mut Self::Stmt,
        columns: &[Vec<DmValue>],
    ) -> Result<ParamArrayOutcome, DmError> {
        bind_param_array(stmt, columns, &mut self.param_array)?;
        let summary = stmt.exec_param_array(&mut self.param_array)?;
        // `affected` 为 `None`（驱动没报）时按 0 记：`ParamArrayOutcome::affected` 是
        // 累加口径，没有「未知」这个状态。真机实测 `SQLRowCount` 在批插后给的是
        // 实际插入行数，所以这里通常拿得到真数字。
        Ok(ParamArrayOutcome {
            affected: summary.affected.unwrap_or(0),
            executed: summary.executed(),
            failed_at: summary.failed_at,
        })
    }

    fn execute(&mut self, stmt: &mut Self::Stmt, sql: &str) -> Result<ExecOutcome, DmError> {
        let outcome = if sql.is_empty() {
            stmt.exec()?
        } else {
            stmt.exec_direct(sql)?
        };

        match outcome {
            StmtOutcome::ResultSet { .. } => {
                // `Backend::columns` 只拿到 `&Self::Stmt`，问不了驱动，所以列元数据
                // 必须在这里就描述好并记在语句上。
                let columns = self.ensure_columns(stmt)?;
                Ok(ExecOutcome::ResultSet { columns })
            }
            // `rows: None` 是「驱动没说影响多少行」，不是「零行」——`ExecOutcome` 本来就
            // 用 `Option` 表达这个区别，照搬即可，不要在这里把它当成结果集。
            StmtOutcome::Affected { rows } => Ok(ExecOutcome::Affected { rows }),
        }
    }

    /// 达梦的语句句柄**取尽结果集之后就不能再执行了**（真机实测见
    /// `crates/dm-odbc/examples/cursor_reuse_probe.rs`：`SQLExecute` 与
    /// `SQLExecDirect` 都回 `24000/-70027 无效的游标状态`，`SQLFreeStmt(SQL_CLOSE)`
    /// 返回 `Ok` 也没用）。所以「产出结果集」的语句不给语句缓存复用，
    /// 只有影响行数形态（DML/DDL）的语句才放回缓存。
    fn statement_reusable(&self, stmt: &Self::Stmt) -> bool {
        !stmt.outcome().is_some_and(StmtOutcome::has_result_set)
    }

    fn columns(&mut self, stmt: &Self::Stmt) -> Result<Arc<[ColumnMeta]>, DmError> {
        match stmt.columns() {
            Some(columns) => Ok(Arc::clone(columns)),
            // 不在这里临时问一遍驱动：`&Self::Stmt` 拿不到可变句柄。真正的调用顺序是
            // 「execute 拿到 ResultSet → 取列」，走到这里说明语句还没产出过结果集。
            None => Err(DmError::at(
                DmErrorKind::Config(
                    "列元数据尚未描述：请先 execute() 让语句产出结果集，或直接读 \
                     ExecOutcome::ResultSet 里带的那份"
                        .to_owned(),
                ),
                Stage::Fetch,
            )),
        }
    }

    fn fetch(&mut self, stmt: &mut Self::Stmt, into: &mut RowBuf) -> Result<FetchState, DmError> {
        let columns = self.ensure_columns(stmt)?;
        // `result::fetch` 自己会核对列数（不一致就报 `Decode`）并清空 `into`，
        // 这里不重复检查——判断只留一处。
        fetch(stmt, &columns, into, &mut self.scratch)
    }

    fn close_cursor(&mut self, stmt: &mut Self::Stmt) -> Result<(), DmError> {
        stmt.close_cursor()
    }

    /// 分块 LOB 读：在**当前行**上开始读第 `column` 列（`column` 从 0 起，是结果集里的下标）。
    ///
    /// 列元数据在这里用 [`OdbcBackend::ensure_columns`] 补齐（和 [`Backend::fetch`] 同一份缓存，
    /// 不重复问驱动），再由语句把它换成 ODBC 的 1 起列号并挂上读取状态。
    ///
    /// 「当前行」由调用方用 [`Backend::lob_next_row`] 定位（或先 [`Backend::fetch`] 取过一批行）；
    /// 本方法**不让游标动**，只备状态，所以调用方还有机会先看一眼列元数据再决定读不读。
    fn lob_start(&mut self, stmt: &mut Self::Stmt, column: usize) -> Result<(), DmError> {
        let columns = self.ensure_columns(stmt)?;
        let meta = columns.get(column).ok_or_else(|| {
            DmError::at(
                DmErrorKind::Config(format!(
                    "列下标 {column} 越界：这一轮结果集只有 {} 列",
                    columns.len()
                )),
                Stage::Fetch,
            )
        })?;
        // ODBC 的列号从 1 起。`usize -> u16` 溢出在这里只可能是调用方给了个荒谬的列号，
        // 而上面已经用真实列数挡过一次，所以 `unwrap_or` 取饱和值不会把错的说成对的。
        let index = u16::try_from(column + 1).unwrap_or(u16::MAX);
        stmt.start_lob(index, meta)
    }

    /// 分块 LOB 读：把游标推进到下一行，不物化任何列（`false` = 取尽）。
    fn lob_next_row(&mut self, stmt: &mut Self::Stmt) -> Result<bool, DmError> {
        stmt.fetch_row()
    }

    /// 分块 LOB 读：再取一趟，把新增内容**追加**到 `out`。
    ///
    /// 状态在语句上，所以要「借出来 → 用 → 放回去」（见 [`crate::stmt`] 的 `take_lob`）。
    /// 这一层只做 `ChunkState -> LobChunk` 的翻译：`NULL` 与「空值」分得开
    /// （[`LobChunk::Null`] 只在这一列确实为 NULL 时给）。
    fn lob_chunk(&mut self, stmt: &mut Self::Stmt, out: &mut Vec<u8>) -> Result<LobChunk, DmError> {
        let mut state = stmt.take_lob().ok_or_else(|| {
            DmError::at(
                DmErrorKind::Config(
                    "分块读还没开始：先 lob_next_row 定位到行，再 lob_start 选定列".to_owned(),
                ),
                Stage::Fetch,
            )
        })?;
        let result = state.next_chunk(stmt, out);
        let null = state.is_null();
        // 出错时状态也要放回去：`out` 里可能已经有半趟内容，丢掉状态会让调用方
        // 既拿不到后续内容、也读不出错因（错误里本来带着列名，但那已在 Err 里）。
        stmt.put_lob(state);
        match result? {
            ChunkState::More => Ok(LobChunk::More),
            ChunkState::Done if null => Ok(LobChunk::Null),
            ChunkState::Done => Ok(LobChunk::Done),
        }
    }

    /// 分块 LOB 写：一处 `?` 的值由 [`LobSource`] 分块交付（`SQL_DATA_AT_EXEC` + `SQLPutData`）。
    ///
    /// # 为什么 feeder 是这里的局部变量
    ///
    /// [`bind_params_with_lob`] 会把 [`LobFeeder`] 的**地址**当 token 交给驱动，紧接着
    /// [`OdbcStatement::exec_delayed`] 用这个 token 接受驱动的索要（`SQLParamData` 原样
    /// 还回来）。feeder 一被搬走，还回来的 token 就指向别处——所以 `bind_params_with_lob`
    /// 返回的是 **`Box<LobFeeder>`**（装箱保证地址不随返回值搬家），而绑定与执行之间
    /// **不能插别的事**（对语句句柄做任何操作都可能重置参数状态）。
    ///
    /// # 与 [`Backend::execute`] 同一条规矩
    ///
    /// `sql` 非空表示「还没 prepare」，先编译；已 prepare 的传空串。产出形态也一样：
    /// 结果集就描述列，否则给影响行数（LOB 写通常是 `insert`/`update`，走后者）。
    fn execute_lob(
        &mut self,
        stmt: &mut Self::Stmt,
        sql: &str,
        args: &[DmValue],
        lob: &mut LobParam<&mut dyn LobSource>,
    ) -> Result<ExecOutcome, DmError> {
        if !sql.is_empty() {
            stmt.prepare(sql)?;
        }
        let mut feeder = bind_params_with_lob(stmt, args, lob, &mut self.arena)?;
        let outcome = stmt.exec_delayed(&mut feeder)?;
        match outcome {
            StmtOutcome::ResultSet { .. } => {
                let columns = self.ensure_columns(stmt)?;
                Ok(ExecOutcome::ResultSet { columns })
            }
            StmtOutcome::Affected { rows } => Ok(ExecOutcome::Affected { rows }),
        }
    }

    fn begin(&mut self) -> Result<(), DmError> {
        self.conn.begin()
    }

    fn commit(&mut self) -> Result<(), DmError> {
        self.conn.commit()
    }

    fn rollback(&mut self) -> Result<(), DmError> {
        self.conn.rollback()
    }

    /// 回池复位：回滚未提交事务 → 还原 `autocommit = ON`。
    ///
    /// §5.7 的四步里另外两步**不在这里**：关游标与清理语句缓存属于语句的主人
    /// （`dm-pool`），本类型拿不到那些语句——它们在 `prepare` 时就交出去了。
    /// 这两步由池在调用 `reset` 之前完成。
    fn reset(&mut self) -> Result<(), DmError> {
        // 回滚失败照常上报：多半意味着连接已经断了，那时池应当丢弃这条连接
        // （`DmErrorKind::Connect`/`ConnectionLost` 会被 `is_connection_lost` 认出来）。
        self.conn.rollback()?;
        self.conn.set_autocommit(true)
    }

    fn close(self) {
        let Self { conn, .. } = self;
        let live = conn.live_statement_count();
        if live > 0 {
            tracing::warn!(
                live_statements = live,
                "关闭后端时仍有语句句柄未释放：语句必须在连接之前析构（CT-09）"
            );
        }
        // `Backend::close` 没有返回值（`Drop` 那一套），所以这里把错误降级成日志——
        // 关闭失败通常只意味着连接已经断了，重连即可。要拿到错误请直接用
        // `OdbcConnection::close`。
        if let Err(err) = conn.close() {
            tracing::warn!(error = %err, "确认式关闭 ODBC 连接失败（已忽略）");
        }
    }
}
