//! 语句对象（T3.3，设计方案 §6.3；`-70027` 纪律见 §6.8 表末行）。
//!
//! 这一层只负责「让一条语句能被反复执行而不出错」：预处理、一次性下发、执行、关游标、
//! 问出参数个数与本次产出的形态。取值在 T3.5（`result.rs`），绑定在 T3.4（`param.rs`）。
//!
//! # 三条纪律
//!
//! 1. **执行前先关游标**。达梦在游标未关时对同一个语句句柄再次执行会报
//!    `-70027 无效的游标状态`（§6.8）。ODBC 的规矩是：执行下一条语句前，上一条语句的结果集
//!    要消费完，或者显式 `SQLFreeStmt(SQL_CLOSE)`。本类型把这条规矩编进了
//!    [`OdbcStatement::exec`] 与 [`OdbcStatement::exec_direct`]：进函数先关。
//! 2. **语句类别问驱动，不看 SQL 文本**。执行后用 `SQLNumResultCols` 判定这次产出的是结果集
//!    还是影响行数（[`StmtOutcome`]）。DPI 时代靠数 `number_columns > 0` 漏判过一次，于是对
//!    `create table` 去 `fetch`，换来一个 `-70027`。**照 SQL 文本猜 `SELECT`/`DDL` 是同一类
//!    错误的另一种写法**：文本里可以有注释、`with`、`returning`、存储过程调用，猜不完。
//! 3. **不该出现的返回码要给可行动的错**。[`OdbcStatement::exec`] 遇到 `NeedData`（有 `?` 却没
//!    绑定）或 `StillExecuting`（异步执行）时给明确的 `Unsupported`，而不是把驱动那句
//!    「状态不对」原样抛给用户（§4.5 实测达梦不支持 ODBC 异步）。反过来，**该出现的
//!    `NeedData` 要接住**：延迟输入参数（[`OdbcStatement::exec_delayed`]，§6.5 的大 LOB 写）
//!    就靠它握手，那条路径上它是正常流程而不是错误。
//!
//! # T3.3 真机实测记下的四条
//!
//! 1. `create table` / `insert` / `update` 之后问 `SQLNumResultCols` 一律得 **0**，问
//!    `SQLRowCount` 得真数字（`insert` 一行 ⇒ `1`，`update` 三行 ⇒ `3`）。DPI 时代靠
//!    `number_columns > 0` 判类别会在这里翻车。
//! 2. **同一语句句柄连续执行多次不会自己变成 `-70027`**——只要每次执行前关掉游标。
//!    真正触发 `-70027` 的是「结果集一行没取就再执行」，而 `close_cursor_if_open()` 挡住了它：
//!    探针里「未消费结果集就第二次 `exec()`」「第三次 `exec()`」「`exec_direct` 与 `exec` 交替」
//!    全部成功，全程 `-70027` 计数为 **0**。
//! 3. `SQLExecute` 时绑定个数少于 `?` 个数 ⇒ `07002 / -70039 绑定的参数个数不正确`；
//!    `SQLExecDirect` 下发带 `?` 的语句 ⇒ `HY000 / -6804 需要更多的参数`。**这两条都不是**
//!    `SQL_NEED_DATA`。但 `SQL_NEED_DATA` 本身**可达**：把某处参数按延迟输入绑定
//!    （`SQL_DATA_AT_EXEC` / `SQL_LEN_DATA_AT_EXEC`）之后 `SQLExecute` 就会回它——Round 14
//!    用 `examples/putdata_probe.rs` 在真机上证实，接住它的是
//!    [`OdbcStatement::exec_delayed`]。
//! 4. 表不存在 ⇒ `42S02 / -2106 无效的表或视图名`（**不是** `-2111`，那是列不存在）。`-2106`
//!    是 T3.3 才登记进 `dm-error::codes` 的，分类为 `ObjectNotFound`。
//!
//! # 生命周期
//!
//! 语句句柄是连接句柄的子句柄，必须在父句柄之前释放（契约 CT-09：`stmt → conn → env`）。
//! 有两条分配路径，它们对这条契约的保证方式不同：
//!
//! * [`OdbcConnection::allocate_statement`] 返回的语句**借用**连接，顺序由借用检查器保证；
//! * [`OdbcConnection::allocate_statement_owned`] 返回 `OdbcStatement<'static>`——它不借用
//!   任何东西（`Backend::Stmt` 要求 `Send + 'static`），所以顺序只剩约定：**语句必须先于
//!   连接析构**。连接侧用一个活语句计数把这条约定变成可观测事实（断连时若仍有活语句会
//!   `warn!`），`OdbcBackend` 则只在整条连接生命周期内交出语句、从不自己留着。
//!
//! T3.6 的 `Backend` 走第二条：`OdbcStatement<'static>` 由调用方（`dm-pool` 的语句缓存）
//! 持有并负责在连接之前丢弃。

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dm_core::ColumnMeta;
use dm_error::{DmError, DmErrorKind, Stage};
use odbc_api::handles::{SqlResult, SqlText, Statement, StatementImpl};
use odbc_sys::{Len, Pointer, SQLPutData};

use crate::conn::OdbcConnection;
use crate::diag;
use crate::lob::{LobFeeder, LobState};
use crate::param::{ParamArrayArena, ParamArraySummary, reset_param_array_attrs};

/// `exec_direct` 收到带 `?` 的语句时的 feature 文本。
///
/// `DmErrorKind` 里没有专门的「调用方用法错误」变体，所以把**可行动的建议**放进 `feature`：
/// 用户看到的是「不支持的特性：带参数的 EXEC_DIRECT …：请改用 prepare() + bind_params()」。
/// 把建议藏进日志是不可接受的——错误消息是唯一保证会被看到的地方（§6.2 `DriverNotFound{tried,hint}`
/// 立的就是这个规矩）。
///
/// **真机实测（T3.3）**：达梦对「`SQLExecDirect` 下发带 `?` 的语句」并不回 `SQL_NEED_DATA`，
/// 而是回 `HY000 / -6804 需要更多的参数`（走 `SQL_ERROR` 路径，落回 `Server`）。对「`SQLExecute`
/// 时绑定个数不符」则回 `07002 / -70039 绑定的参数个数不正确`。**这两条都与
/// [`OdbcStatement::exec_delayed`] 无关**：那条路径是「已经绑了延迟输入参数」，达梦会规规矩矩
/// 回 `SQL_NEED_DATA`（Round 14 实测），由 `exec_delayed` 自己消费掉，不走这里。
/// 所以本常量只在「语句里有 `?`、却一处都没绑定」时才用得上。
/// 两条实测码**故意不登记**进 `dm-error::codes`：它们是「调用方用法错误」，而这里的规矩是
/// 未登记的码一律原样落回 `Server`（见 `DmErrorKind::classify_server`）。
const EXEC_DIRECT_WITH_PARAMS: &str =
    "带参数的 EXEC_DIRECT（`?` 未绑定）：请改用 prepare() + bind_params()";

/// `exec` 遇到 `SQL_STILL_EXECUTING` 时的 feature 文本（§4.5）。
const ASYNC_EXECUTION: &str = "ODBC 异步执行（SQL_ATTR_ASYNC_ENABLE）";

/// [`OdbcStatement::exec_delayed`] 的 `SQLParamData` 趟数上限。
///
/// 一趟 = 驱动索要一处参数的数据。`bind_params_with_lob` 只绑一处 LOB，正常一两趟就结束；
/// 给上限是防坏驱动在 `SQLParamData` 上永远要数据，把执行线程粘在一条语句上。
/// 宁可报一个能行动的错，也不要静默挂住（与 `dm-query` 的 `MAX_EMPTY_BATCHES` 同一条纪律）。
const MAX_DELAYED_ROUNDS: usize = 1_000_000;

// ---------------------------------------------------------------------------
// 语句的两种分类：来源与产出
// ---------------------------------------------------------------------------

/// 语句句柄当前装的是什么，决定 [`OdbcStatement::exec`] 能不能调用。
///
/// 这是**来源**分类；本次执行**产出了什么**是 [`StmtOutcome`]，两者不要混。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StmtKind {
    /// 既没 `prepare` 也没 `exec_direct` 过。
    Fresh,
    /// 已经 [`OdbcStatement::prepare`]：驱动编译过这条 SQL，`?` 占位符个数已确定。
    Prepared,
    /// 已经 [`OdbcStatement::exec_direct`] 一次性下发：句柄上只有「执行过的直接语句」，
    /// 没有可重复执行的预处理计划，因此 [`OdbcStatement::exec`] 会拒绝。
    Direct,
}

impl StmtKind {
    /// 是否处于可重复执行的预处理状态。
    pub fn is_prepared(self) -> bool {
        matches!(self, StmtKind::Prepared)
    }
}

/// 一次执行**驱动实际报告**的产出形态。
///
/// 只有一个来源：执行成功后问 `SQLNumResultCols`。`columns > 0` ⇒ 结果集；否则 ⇒ 影响行数。
/// 这是「先判语句类别」的唯一正确做法（见模块文档纪律 2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StmtOutcome {
    /// 有结果集，列数为 `columns`（≥ 1）。
    ResultSet {
        /// 结果集列数。
        columns: u16,
    },
    /// 没有结果集。`rows` 是 `SQLRowCount` 给出（或没给出）的影响行数。
    Affected {
        /// 影响行数；驱动报 `-1`（不可用）时为 `None`。
        ///
        /// **`None` 不等于 0**：0 是「确实一行都没影响」，`None` 是「驱动没说」。
        rows: Option<u64>,
    },
}

impl StmtOutcome {
    /// 是否有结果集。为 `false` 时**不得** `fetch`（否则是 `-70027`）。
    pub fn has_result_set(self) -> bool {
        matches!(self, StmtOutcome::ResultSet { .. })
    }

    /// 结果集列数；不是结果集时为 `None`。
    pub fn columns(self) -> Option<u16> {
        match self {
            StmtOutcome::ResultSet { columns } => Some(columns),
            StmtOutcome::Affected { .. } => None,
        }
    }

    /// 影响行数；结果集、或驱动没给数字时为 `None`。
    pub fn affected_rows(self) -> Option<u64> {
        match self {
            StmtOutcome::Affected { rows } => rows,
            StmtOutcome::ResultSet { .. } => None,
        }
    }

    /// 是否是「无结果集」的语句（`create table`/`insert`/`update`/`delete` 等）。
    pub fn is_affected(self) -> bool {
        matches!(self, StmtOutcome::Affected { .. })
    }
}

// ---------------------------------------------------------------------------
// 语句对象
// ---------------------------------------------------------------------------

/// 一条达梦语句句柄。
///
/// 由 [`OdbcConnection::allocate_statement`] 创建；`Drop` 时 `odbc-api` 会
/// `SQLFreeHandle(SQL_HANDLE_STMT)`（句柄所有权在 `StatementImpl` 里，我们不自建 `HStmt`，
/// 也就没有「谁负责释放」的第二份约定）。
///
/// 所有会与服务端交互的方法都要 `&mut self`：用完必须先关游标再执行下一条（纪律 1）。
pub struct OdbcStatement<'conn> {
    /// 拥有的语句句柄。`'conn` 是它借用的连接句柄的存活期（模块文档「生命周期」）。
    stmt: StatementImpl<'conn>,
    /// 最近一次 `prepare`/`exec_direct` 的 SQL 原文，供错误上报与 `Debug`。
    ///
    /// 存一份 `String` 是有意的：出错时原始 SQL 是排查的第一现场，而 `Statement` 不回吐它。
    sql: Option<String>,
    /// 句柄上装的是什么（来源分类）。
    kind: StmtKind,
    /// 最近一次执行的产出形态。
    outcome: Option<StmtOutcome>,
    /// `prepare` 之后由驱动给出的 `?` 占位符个数。
    param_count: u16,
    /// **本层认为**游标是否开着。
    ///
    /// 只用于「执行前要不要先关」这一个决定，而且**估计偏保守**：T3.5 把结果集取空时游标
    /// 其实已经关了，这里仍记为开着，于是执行前会多关一次——`SQLFreeStmt(SQL_CLOSE)` 对没有
    /// 游标的句柄是空操作，代价可以忽略。反方向的错（该关却没关）才会换来 `-70027`，
    /// 所以宁可多关。
    cursor_open: bool,
    /// 最近一次描述出来的列元数据。
    ///
    /// 缓存它是为了让 `Backend::fetch` 不必每次取行都重新问一遍列属性：
    /// [`crate::result::describe_columns`] 成功后会写进来，执行新语句时清空。
    /// 用 `Arc<[ColumnMeta]>` 是因为它同时要交给 `ExecOutcome::ResultSet`，
    /// 两边共享同一份、克隆只加一个引用计数。
    columns: Option<Arc<[ColumnMeta]>>,
    /// 分块 LOB 读的进度（§6.5，[`crate::lob`]）。
    ///
    /// 状态挂在语句上而不是挂在调用方那边：`dm-pool` 的 `with_conn` 只搬
    /// `Self` 与 `Self::Stmt`，分块读的进度必须跟着语句一起回执行线程，否则会出现
    /// 「语句回了执行线程、进度还留在调用线程」的错位（`dm-query` 的 `LobStream`
    /// 就是靠这条把状态跨 `await` 搬运的）。
    ///
    /// 任何让**当前行失效**的操作都会把它清成 `None`（预处理/执行/关游标/取尽）。
    lob: Option<LobState>,
    /// 连接侧的活语句计数，`Drop` 时减一（CT-09 的可观测面，见模块文档「生命周期」）。
    live: Arc<AtomicUsize>,
}

impl Drop for OdbcStatement<'_> {
    fn drop(&mut self) {
        // 先减计数，再让字段里的 `StatementImpl` 去 `SQLFreeHandle`（字段在函数体之后析构）。
        self.live.fetch_sub(1, Ordering::Relaxed);
    }
}

impl<'conn> OdbcStatement<'conn> {
    /// 包一个刚分配好的语句句柄，并在连接的在册计数上加一。
    pub(crate) fn new(stmt: StatementImpl<'conn>, live: Arc<AtomicUsize>) -> Self {
        live.fetch_add(1, Ordering::Relaxed);
        Self {
            stmt,
            sql: None,
            kind: StmtKind::Fresh,
            outcome: None,
            param_count: 0,
            cursor_open: false,
            columns: None,
            lob: None,
            live,
        }
    }

    /// 预处理一条 SQL。对应 `SQLPrepare`。
    ///
    /// 成功后 [`OdbcStatement::param_count`] 给出 `?` 的个数（由驱动算，不是我们数文本里的
    /// `?`——文本里的 `?` 可能出现在字符串字面量或注释里）。
    pub fn prepare(&mut self, sql: &str) -> Result<(), DmError> {
        // 重新编译前先关掉上一条语句留下的游标：DM 在游标未关时会对预处理/执行报状态错。
        self.close_cursor_if_open()?;
        // 预处理换掉的是「当前行」本身，挂在旧行上的分块读进度随之失效。
        self.lob = None;
        let text = SqlText::new(sql);
        let res = self.stmt.prepare(&text);
        diag::sql_ok(res, &self.stmt, Stage::Prepare, "SQLPrepareW")?;

        self.sql = Some(sql.to_owned());
        self.kind = StmtKind::Prepared;
        self.outcome = None;
        // `SQLNumParams` 只对预处理过的语句有效，所以只能放在 `prepare` 成功之后。
        let params = self.stmt.num_params();
        self.param_count = diag::sql_ok(params, &self.stmt, Stage::Prepare, "SQLNumParams")?;
        Ok(())
    }

    /// 一次性下发一条 SQL。对应 `SQLExecDirect`。
    ///
    /// **不接受参数**：本层只有立即绑定路径（§6.3），`?` 占位符必须在执行前由
    /// [`crate::param`]（T3.4）绑定，而一次性下发没有「执行前」这个时机。语句里带 `?` 时
    /// 驱动会回 `SQL_NEED_DATA`，这里翻成可行动的 `Unsupported`（见 [`EXEC_DIRECT_WITH_PARAMS`]）。
    pub fn exec_direct(&mut self, sql: &str) -> Result<StmtOutcome, DmError> {
        self.close_cursor_if_open()?;
        self.lob = None;
        let text = SqlText::new(sql);
        // SAFETY: `SQLExecDirect` 可能解引用已绑定的参数缓冲。本层从不绑定 delayed input 参数
        // （未使用 `bind_delayed_input_parameter`），因此不存在会被解引用的参数缓冲；
        // `self.stmt` 是 `StatementImpl` 持有的有效语句句柄（`allocate_statement` 成功才构造本类型）。
        let res = unsafe { self.stmt.exec_direct(&text) };

        self.sql = Some(sql.to_owned());
        self.kind = StmtKind::Direct;
        self.param_count = 0;
        self.outcome = None;
        self.finish_exec(res, "SQLExecDirectW")
    }

    /// 执行已预处理的语句。对应 `SQLExecute`，即 §6.3 的 `exec()`。
    ///
    /// `kind != Prepared` 时直接拒绝：`SQLExecute` 打在只执行过 `SQLExecDirect` 的句柄上，
    /// 驱动的反应是「无效的游标状态」之类与真实原因无关的诊断。**该拒绝的必须拒绝**，
    /// 免得把一个用法错误伪装成服务端错误。
    pub fn exec(&mut self) -> Result<StmtOutcome, DmError> {
        if !self.kind.is_prepared() {
            return Err(DmError::at(
                DmErrorKind::Config(format!(
                    "exec() 只能执行预处理过的语句，当前状态是 {:?}：请先 prepare(sql)，\
                     或者用 exec_direct(sql) 一次性下发",
                    self.kind
                )),
                Stage::Execute,
            ));
        }
        self.close_cursor_if_open()?;
        // 同上：执行把结果集换成新的，旧的分块读进度不能跨执行存活。
        self.lob = None;
        // SAFETY: 同 `exec_direct` —— 本层不绑定 delayed input 参数，没有被解引用的参数缓冲；
        // `self.stmt` 是有效语句句柄。
        let res = unsafe { self.stmt.execute() };
        self.outcome = None;
        self.finish_exec(res, "SQLExecute")
    }

    /// 执行一条**已绑好延迟输入参数**的语句，并按驱动要的节奏把数据喂进去
    /// （§6.5 的大 LOB 写，Round 14；绑定见 [`crate::param::bind_params_with_lob`]）。
    ///
    /// # 三件事
    ///
    /// 1. `SQLExecute`：对延迟参数驱动回 `SQL_NEED_DATA`（`examples/putdata_probe.rs`
    ///    在真机上确认达梦会回）；
    /// 2. `SQLParamData` 循环：驱动每给出一个 token，就用 `SQLPutData` 把那一处的数据推完；
    /// 3. 收尾：`SQLParamData` 回 `Success(None)` 之后按普通路径问产出（[`StmtOutcome`]）。
    ///
    /// # token 对不上必须报错
    ///
    /// 一条语句上只允许有一处延迟参数（`bind_params_with_lob` 只绑一处）。token 对不上
    /// 就意味着驱动还回来的不是我们绑进去的那处——此时**没有任何办法**知道该往哪里取数据，
    /// 只能报错。猜一个继续喂，等于把另一处参数的内容写进这个 LOB。
    ///
    /// # 喂到一半失败会怎样
    ///
    /// 失败时语句停在 `SQL_NEED_DATA` 状态：既不能再执行一次，也不能留在池里给下一个
    /// 借用者。调用方（`OdbcBackend::execute_lob`）把它当普通 `Err` 返回，`dm-pool` 会
    /// 因为「不可复用」而丢掉这条语句。源侧的错误**故意**归 `Encode`（不是 `Io`）——
    /// 否则池子会顺手把一条好连接也扔了（`DmErrorKind::is_connection_lost`）。
    pub fn exec_delayed(&mut self, feeder: &mut LobFeeder<'_>) -> Result<StmtOutcome, DmError> {
        if !self.kind.is_prepared() {
            return Err(DmError::at(
                DmErrorKind::Config(format!(
                    "exec_delayed() 只能执行预处理过的语句，当前状态是 {:?}：请先 prepare(sql)",
                    self.kind
                )),
                Stage::Execute,
            ));
        }
        self.close_cursor_if_open()?;
        // 执行换掉的是「当前行」本身，挂在旧行上的分块读进度随之失效（同 `exec`）。
        self.lob = None;

        // SAFETY: 延迟输入参数的 `ParameterValuePtr` 是 token 而不是数据地址，驱动在
        // `SQLExecute` 上不会解引用它（要数据时回 `SQL_NEED_DATA`，数据走下面的
        // `SQLPutData`）；`self.stmt` 是有效语句句柄。
        let executed = unsafe { self.stmt.execute() };
        self.outcome = None;
        match executed {
            SqlResult::NeedData => {}
            SqlResult::Error { function } => {
                return Err(diag::collect_and_classify(
                    &self.stmt,
                    Stage::Execute,
                    function,
                ));
            }
            // 绑了延迟参数却没被要数据：这条路径上不该发生，绝不静默当成成功
            // （那会把「参数根本没写进去」伪装成一次成功的写入）。
            other => return Err(delayed_unexpected("SQLExecute", &other)),
        }

        let token = feeder.token();
        let mut rounds = 0usize;
        loop {
            rounds += 1;
            if rounds > MAX_DELAYED_ROUNDS {
                return Err(delayed_too_many_rounds(rounds));
            }

            match self.statement().param_data() {
                SqlResult::Success(Some(got)) => {
                    if got != token {
                        return Err(delayed_token_mismatch(got, token));
                    }
                    let mut fed = 0usize;
                    while let Some(batch) = feeder.next_batch()? {
                        fed += batch.len();
                        self.put_data(batch)?;
                    }
                    if fed == 0 {
                        return Err(delayed_source_empty());
                    }
                }
                SqlResult::Success(None) => break,
                SqlResult::Error { function } => {
                    return Err(diag::collect_and_classify(
                        &self.stmt,
                        Stage::Execute,
                        function,
                    ));
                }
                other => return Err(delayed_unexpected("SQLParamData", &other)),
            }
        }

        // `SQLParamData` 的 `Success(None)` 已经等价于「执行成功」：这里按普通路径收尾。
        self.finish_exec(SqlResult::Success(()), "SQLExecute")
    }

    /// 把一批数据推给驱动（`SQLPutData`）：延迟参数的一次「喂」。
    ///
    /// 空批在这里是**错误**（调用方判 `fed == 0`），而不是无害调用：ODBC 允许用
    /// 「空批」表达「长度未知时的收尾」，而我们的源已经用 `None` 表达「取尽」了——
    /// 两套「结束」语义混在一起会让「到底写完没有」变得不可判定。
    fn put_data(&mut self, batch: &[u8]) -> Result<(), DmError> {
        let len = Len::try_from(batch.len()).map_err(|_| {
            DmError::at(
                DmErrorKind::Config(format!(
                    "待写 LOB 的一批数据 {} 字节超过 ODBC 的 isize 长度上限",
                    batch.len()
                )),
                Stage::Execute,
            )
        })?;

        // SAFETY: `SQLPutData` 从 `batch` 取走这一批（契约是「取一份拷贝」，驱动不留指针），
        // `batch` 在本次调用期间有效；句柄来自活着的语句，本方法持有 `&mut self`。
        let ret = unsafe { SQLPutData(self.statement().as_sys(), batch.as_ptr() as Pointer, len) };
        diag::sql_ok(
            diag::fold_return(ret, "SQLPutData"),
            &self.stmt,
            Stage::Execute,
            "SQLPutData",
        )
    }

    /// 执行已预处理、且已绑好**参数数组**的语句（T6.4，§4.7）。
    ///
    /// 与 [`OdbcStatement::exec`] 的差别是「一次下发 `N` 组参数」，所以结果的形态也不同：
    /// 这里不返回 [`StmtOutcome`]，而是给出一份逐行的账单
    /// （[`ParamArraySummary`]）——数组执行时「第几行坏了」只能从参数状态数组里读出来。
    ///
    /// # 为什么复位必须写在这里
    ///
    /// `SQL_ATTR_PARAMSET_SIZE` 与两个指针是**语句句柄级**状态。语句在池里复用，
    /// 所以「执行完把数组设置收回」是执行语义的一部分，不是收尾的客套。这里用
    /// 一个**独立的作用域**把复位与执行绑在一起：`exec_param_array` 返回之后就复位，
    /// 成功、行级失败、甚至提前 `?` 返回（它在内层作用域里调用，由
    /// [`crate::param::reset_param_array_attrs`] 的调用点收尾）都不会漏。
    ///
    /// 顺序上先 `SQLExecute`（此时驱动还在读我们的缓冲）再复位，中间不能插别的事。
    ///
    /// # 行级失败为什么返回 `Ok`
    ///
    /// ODBC 说「某一组参数违反约束」时常常整体 `SQL_SUCCESS_WITH_INFO`，
    /// 而 [`diag::sql_ok`] 把 `SUCCESS_WITH_INFO` 当成功（只留一条 `warn!`）。
    /// 只靠返回码判断会把「第 500 行没插进去」读成「1000 行全成功」——
    /// 所以判决必须来自参数状态数组，而那个判决的结果是一种**正常的业务结果**，
    /// 由 `dm-query` 记进 `BatchOutcome`（§6.7），不在这里变成 `Err`。
    pub fn exec_param_array(
        &mut self,
        arena: &mut ParamArrayArena,
    ) -> Result<ParamArraySummary, DmError> {
        if !self.kind.is_prepared() {
            return Err(DmError::at(
                DmErrorKind::Config(format!(
                    "exec_param_array() 只能执行预处理过的语句，当前状态是 {:?}：请先 prepare(sql)",
                    self.kind
                )),
                Stage::Execute,
            ));
        }
        self.close_cursor_if_open()?;
        self.outcome = None;

        // `arena` 的裸指针已经交给驱动，所以从绑定到这里的这条路径上不能再动它。
        // 执行与「读回逐行账单」放在同一个作用域，复位紧跟其后：见函数文档。
        let summary = {
            // SAFETY: 与 `exec` 同理——本层不绑定 delayed input 参数；参数缓冲由
            // `arena` 持有且活过整次 `SQLExecute`（调用方借出 `&mut arena`，本函数
            // 期间不可能有人改它）；`self.stmt` 是有效语句句柄。
            let res = unsafe { self.stmt.execute() };
            if let Some(err) = misuse_error(&res) {
                return Err(err);
            }
            diag::sql_ok(res, &self.stmt, Stage::Execute, "SQLExecute")?;

            // 行级判决：`SQL_PARAM_ERROR` 是唯一确定的坏行（`SUCCESS_WITH_INFO`
            // 与 `DIAG_UNAVAILABLE` 都可能是「这行成功了但另有告警」）。取第一个坏行
            // 是因为已经执行过的行**不会**被驱动回滚——`execute_many` 的契约是
            // 「不抹掉已生效的前几组」，所以这里也按「首次违约即停下记账」处理。
            //
            // 成功行数按状态数组数，不按 `failed_at` 减：驱动完全可以把坏行之后的
            // 槽位也填上成功（那些行其实没执行），减出来的数会凭空多记。
            let rows = arena.rows();
            let failed_at = arena.first_param_error();
            let succeeded = arena.succeeded();
            let processed = arena.processed();
            let affected = self.affected_rows()?;
            ParamArraySummary {
                rows,
                processed,
                succeeded,
                failed_at,
                affected,
            }
        };

        // 语句句柄上的数组设置必须收回，否则下一次复用这条语句的单行执行会按
        // 「一次 N 行」跑（本项目最隐蔽的一类错，见 param.rs 的同名注释）。
        reset_param_array_attrs(self)?;

        // 产出形态仍按单行那套记：数组执行也只会是「影响行数」或「结果集」之一。
        // 这里有意**不**用 `finish_exec`——它会重新问一次 `SQLNumResultCols` 与
        // `SQLRowCount`，而 `ParamArraySummary` 已经带着 `affected` 了。
        let columns = self.stmt.num_result_cols();
        let columns = diag::sql_ok(columns, &self.stmt, Stage::Execute, "SQLNumResultCols")?;
        self.outcome = Some(if columns > 0 {
            StmtOutcome::ResultSet {
                columns: u16::try_from(columns).unwrap_or(u16::MAX),
            }
        } else {
            StmtOutcome::Affected {
                rows: summary.affected,
            }
        });
        self.cursor_open = self.outcome.is_some_and(StmtOutcome::has_result_set);
        self.columns = None;
        Ok(summary)
    }

    /// 执行成功后问驱动：这次产出的是结果集还是影响行数，并记下来。
    fn finish_exec(
        &mut self,
        res: SqlResult<()>,
        function: &'static str,
    ) -> Result<StmtOutcome, DmError> {
        // 先挑出「本层不该出现」的返回码，剩下的交给统一口径（`Success`/`SuccessWithInfo`
        // 之外一律收集诊断并分类）。
        if let Some(err) = misuse_error(&res) {
            return Err(err);
        }
        diag::sql_ok(res, &self.stmt, Stage::Execute, function)?;

        let columns = self.stmt.num_result_cols();
        let columns = diag::sql_ok(columns, &self.stmt, Stage::Execute, "SQLNumResultCols")?;
        let outcome = if columns > 0 {
            StmtOutcome::ResultSet {
                columns: u16::try_from(columns).unwrap_or(u16::MAX),
            }
        } else {
            StmtOutcome::Affected {
                rows: affected_rows_from(self.raw_row_count()?),
            }
        };

        self.outcome = Some(outcome);
        self.cursor_open = outcome.has_result_set();
        // 换了产出就作废上一轮的列元数据：留着它会让 `columns()` 报出上一条语句的列。
        self.columns = None;
        Ok(outcome)
    }

    /// `SQLRowCount` 的原始值（负数表示驱动没给数字）。
    fn raw_row_count(&mut self) -> Result<isize, DmError> {
        let res = self.stmt.row_count();
        diag::sql_ok(res, &self.stmt, Stage::Execute, "SQLRowCount")
    }

    /// 影响行数。结果集语句、或驱动没给数字时为 `None`。
    ///
    /// 与 [`OdbcStatement::outcome`] 的区别：这个**每次都重新问驱动**，用于「执行完很久之后
    /// 再确认一次」的场景（T3.6 的 `Backend::execute` 直接用 `outcome`，不必重问）。
    pub fn affected_rows(&mut self) -> Result<Option<u64>, DmError> {
        Ok(affected_rows_from(self.raw_row_count()?))
    }

    /// 关闭游标（`SQLFreeStmt(SQL_CLOSE)`），丢弃尚未取完的结果。
    ///
    /// **没有游标时直接返回 `Ok(())`，不下发 ODBC 调用**——达梦对「没有游标时的
    /// `SQLFreeStmt(SQL_CLOSE)`」回 `24000/-70027 无效的游标状态`（M3/T3.7 真机实测，
    /// 见 `examples/t37_bisect.rs`：fresh 句柄上 `close_cursor` ⇒ `-70027`；刚跑完 DDL 的句柄
    /// 上 `close_cursor` ⇒ `-70027`；只有「`SELECT` 之后、结果集还没取完」时才成功）。
    /// ODBC 规范说这种情况该是空操作，驱动不是；**照规范写会让每一次 DDL 都报一条假错误**，
    /// 而 `-70027` 正是本项目最重要的自动化防线（AC-05），不能被假阳性淹掉。
    ///
    /// 因此调用前先看 [`OdbcStatement::is_cursor_open`]。那个标志是**保守的过估计**
    /// （取值层取尽时会 [`OdbcStatement::mark_cursor_closed`] 收窄），方向是对的：
    /// 宁可少关一次（多一次空操作）也不要多关一次（换来一条假 `-70027`）。
    ///
    /// 重复调用是安全的。**事务里必须在提交/回滚前关掉**（§7「事务内不允许留活游标」）。
    pub fn close_cursor(&mut self) -> Result<(), DmError> {
        // 关游标就是作废「当前行」，挂在那一行上的分块读进度一并丢掉（跨行/跨结果集都不该存活）。
        self.lob = None;
        if !self.cursor_open {
            return Ok(());
        }
        let res = self.stmt.close_cursor();
        diag::sql_ok(res, &self.stmt, Stage::Execute, "SQLFreeStmt(SQL_CLOSE)")?;
        self.cursor_open = false;
        Ok(())
    }

    /// 复位已绑定的参数（`SQLFreeStmt(SQL_RESET_PARAMS)`）。T3.4 换参数集前会用到。
    pub fn reset_parameters(&mut self) -> Result<(), DmError> {
        let res = self.stmt.reset_parameters();
        diag::sql_ok(
            res,
            &self.stmt,
            Stage::Bind,
            "SQLFreeStmt(SQL_RESET_PARAMS)",
        )
    }

    /// 执行前先关掉上一条语句留下的游标（纪律 1）。
    fn close_cursor_if_open(&mut self) -> Result<(), DmError> {
        if self.cursor_open {
            self.close_cursor()?;
        }
        Ok(())
    }

    /// 句柄上装的是什么。
    pub fn kind(&self) -> StmtKind {
        self.kind
    }

    /// 最近一次执行的产出形态；还没执行过时为 `None`。
    pub fn outcome(&self) -> Option<StmtOutcome> {
        self.outcome
    }

    /// 最近一次交给驱动的 SQL 原文。
    pub fn sql(&self) -> Option<&str> {
        self.sql.as_deref()
    }

    /// 预处理后由驱动给出的 `?` 占位符个数（未预处理时为 0）。
    pub fn param_count(&self) -> u16 {
        self.param_count
    }

    /// 本层认为游标是否开着（估计偏保守，见字段文档）。
    pub fn is_cursor_open(&self) -> bool {
        self.cursor_open
    }

    /// 由取值层（T3.5）告知「驱动已经把游标走到头了」。
    ///
    /// `SQLFetch` 返回 `SQL_NO_DATA` 时驱动自己会关掉游标；取值层知道这件事，
    /// 本层不知道（它只看得到 `SQLNumResultCols`）。同步这个状态只为少发一次
    /// 无意义的 `SQLFreeStmt(SQL_CLOSE)`——**不发也没错**，所以这个方法是保守估计
    /// 的收窄方向，不会引入 `-70027`。
    pub fn mark_cursor_closed(&mut self) {
        self.cursor_open = false;
        // 取尽意味着没有「当前行」了：分块读进度不再有任何意义（连同 carry 一起丢掉）。
        self.lob = None;
    }

    /// 取下一行（`SQLFetch`）：取到返回 `true`，取尽返回 `false`。
    ///
    /// 分块 LOB 读需要**自己**把游标定位到目标行上（`SQLGetData` 只对当前行有效），
    /// 所以 `Backend::lob_start` 用它推进游标。取尽时会调用
    /// [`mark_cursor_closed`](Self::mark_cursor_closed)（`SQL_NO_DATA` 时驱动已经自己关了游标）。
    ///
    /// 与 [`crate::result::fetch`] 的区别是「谁来读这一行」：那边取整行并转成 `DmValue`，
    /// 这边只推进游标，把值留给 [`crate::lob`] 按块取。**同一行内 `SQLGetData` 的列号必须
    /// 递增**（ODBC 规范），所以两者不要在同一行上混用。
    pub fn fetch_row(&mut self) -> Result<bool, DmError> {
        // SAFETY: 本层不绑定 delayed input 参数，`SQLFetch` 不会解引用调用方的参数缓冲；
        // `self.stmt` 是有效语句句柄。
        let fetched = unsafe { self.stmt.fetch() };
        match fetched {
            SqlResult::Success(()) | SqlResult::SuccessWithInfo(()) => Ok(true),
            SqlResult::NoData => {
                self.mark_cursor_closed();
                Ok(false)
            }
            other => {
                let _ = other;
                Err(diag::collect_and_classify(
                    &self.stmt,
                    Stage::Fetch,
                    "SQLFetch",
                ))
            }
        }
    }

    /// 在当前行上开始分块读某一列（§6.5，[`crate::lob`]）。
    ///
    /// 只是**备好状态**，不动驱动：真正的第一趟发生在
    /// [`crate::lob::LobState::next_chunk`]。状态挂在语句上（见 [`crate::lob`] 模块头），
    /// 所以这一步是幂等的覆盖：再调一次就从新的列重新开始，旧的进度被丢掉。
    pub fn start_lob(&mut self, column_index: u16, meta: &ColumnMeta) -> Result<(), DmError> {
        self.lob = Some(LobState::new(column_index, meta)?);
        Ok(())
    }

    /// 把分块读状态**借出去**（拿走），处理完再用 [`put_lob`](Self::put_lob) 放回。
    ///
    /// 为什么要借：状态与语句句柄需要同时可变借用（一个要改状态，一个要发
    /// `SQLGetData`）。`Self::Stmt` 是 `Send + 'static` 的，所以状态不能挂在调用方的
    /// 栈上而必须挂在语句里，而语句本身又必须能借给取数函数——借出/放回是这两条约束的
    /// 交点。返回 `None` 表示「没有进行中的分块读」。
    pub fn take_lob(&mut self) -> Option<LobState> {
        self.lob.take()
    }

    /// 把借出去的状态放回来（[`take_lob`](Self::take_lob) 的逆操作）。
    pub fn put_lob(&mut self, state: LobState) {
        self.lob = Some(state);
    }

    /// 进行中的分块读状态（只读视图）。
    pub fn lob(&self) -> Option<&LobState> {
        self.lob.as_ref()
    }

    /// 取底层语句句柄，供 T3.4 绑定参数、T3.5 绑定列与取值使用。
    ///
    /// 暴露它是有意的：把「绑定」和「取值」拆到另外两个模块，但不想为它们各自再包一层
    /// 转发方法（那会变成 30 个 `pub fn` 的透传层）。调用方拿到的仍是 `odbc-api` 的 `Statement`
    /// trait，不涉及裸 FFI。
    pub fn statement(&mut self) -> &mut StatementImpl<'conn> {
        &mut self.stmt
    }

    /// 记下本轮结果集的列元数据（[`crate::result::describe_columns`] 调用）。
    pub fn set_columns(&mut self, columns: Arc<[ColumnMeta]>) {
        self.columns = Some(columns);
    }

    /// 本轮结果集的列元数据；还没描述过时为 `None`。
    ///
    /// 返回 `Arc` 而不是切片，是为了让调用方在**不借用**语句的前提下继续用它
    /// （`Backend::fetch` 要在持有列元数据的同时可变借用语句去取行）。
    pub fn columns(&self) -> Option<&Arc<[ColumnMeta]>> {
        self.columns.as_ref()
    }
}

impl fmt::Debug for OdbcStatement<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 不打印句柄：`HStmt` 是个裸指针，打出来对排查没帮助，还会让日志看起来像有地址信息。
        f.debug_struct("OdbcStatement")
            .field("kind", &self.kind)
            .field("sql", &self.sql)
            .field("param_count", &self.param_count)
            .field("outcome", &self.outcome)
            .field("cursor_open", &self.cursor_open)
            .field("columns", &self.columns.as_ref().map(|c| c.len()))
            // 只报「有没有进行中的分块读」：进度细节对排查没帮助，还会把 Debug 输出拉长。
            .field("lob", &self.lob.is_some())
            .finish_non_exhaustive()
    }
}

impl OdbcConnection {
    /// 在本连接上分配一条语句句柄。
    ///
    /// 返回的语句**借用**本连接（生命周期由借用检查器管，见 [`OdbcStatement`] 的「生命周期」）。
    /// 需要 `'static` 语句（`Backend::Stmt`）时用
    /// [`OdbcConnection::allocate_statement_owned`]。
    ///
    /// 分配失败时会带上语句句柄上的诊断——注意 ODBC 的规矩：`SQLAllocHandle` 失败时诊断在
    /// **连接**句柄上（新语句句柄根本没建起来），所以这里收集的是连接句柄的诊断。
    pub fn allocate_statement(&mut self) -> Result<OdbcStatement<'_>, DmError> {
        let counter = self.statement_counter();
        Self::allocate_raw(self, |stmt| OdbcStatement::new(stmt, counter))
    }

    /// 分配一条**不借用**连接的语句（`Backend::Stmt` 要求 `Send + 'static`）。
    ///
    /// ⚠️ 拿到的语句必须在连接**之前**析构（CT-09）。借用检查器在这里帮不上忙，
    /// 所以连接侧维护了一个活语句计数：[`OdbcConnection::live_statement_count`]，
    /// 断开时若不为零会留下 `warn!`。
    pub fn allocate_statement_owned(&mut self) -> Result<OdbcStatement<'static>, DmError> {
        let counter = self.statement_counter();
        Self::allocate_raw(self, move |stmt| {
            // `into_sys` 把句柄的所有权从 `StatementImpl<'conn>` 里搬出来（不触发 Drop、
            // 不会释放句柄），再交给一个 `'static` 的包装。句柄本身与生命周期无关——
            // 它就是一个 `HStmt` 裸指针；`'static` 在这里表达的是「不借用连接」，
            // 而不是「保证活得比连接久」，后者由调用方按 CT-09 负责。
            let handle = stmt.into_sys();
            // SAFETY: `handle` 是一个有效的语句句柄，来自刚刚成功的
            // `SQLAllocHandle(SQL_HANDLE_STMT)`；它的所有权已经通过 `into_sys` 转移出来，
            // 所以由新建的 `StatementImpl` 独占并在 `Drop` 时释放恰好一次，不会双重释放。
            let owned = unsafe { StatementImpl::<'static>::new(handle) };
            OdbcStatement::new(owned, counter)
        })
    }

    /// 分配 + 包装的公共部分：`wrap` 决定包成借用版还是 `'static` 版
    /// （因此返回类型对 `wrap` 的输出泛型，两种包装各有各的生命周期）。
    fn allocate_raw<'a, R>(
        conn: &'a mut OdbcConnection,
        wrap: impl FnOnce(StatementImpl<'a>) -> R,
    ) -> Result<R, DmError> {
        let handle = conn.handle();
        match handle.allocate_statement() {
            SqlResult::Success(stmt) | SqlResult::SuccessWithInfo(stmt) => Ok(wrap(stmt)),
            _ => Err(diag::collect_and_classify(
                handle,
                Stage::Execute,
                "SQLAllocHandle(SQL_HANDLE_STMT)",
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// 纯逻辑（可脱机测试的部分）
// ---------------------------------------------------------------------------

/// `SQLRowCount` 的原始值 → 影响行数。
///
/// ODBC 允许驱动在「说不清影响多少行」时把输出设成负数（`-1`，部分驱动用 `SQL_NO_TOTAL` 的
/// `-4`）。**不能夹到 0**：`0` 是「确实一行都没影响」，把它们混起来会让调用方以为
/// `update` 命中了零行。
fn affected_rows_from(count: isize) -> Option<u64> {
    u64::try_from(count).ok()
}

/// 延迟输入路径上出现了非预期形状的返回码。
///
/// 归 `Unsupported` 而不是 `Server`：这不是「服务端说你的 SQL 错了」，而是这条调用路径
/// 在这台驱动上不按 ODBC 规范走。**绝不静默降级成成功**——那会把「参数根本没写进去」
/// 伪装成一次成功的写入。
///
/// 对 `T` 泛型是必要的：延迟输入的两个调用点一个看 `SqlResult<()>`（`SQLExecute`），
/// 另一个看 `SqlResult<Option<Pointer>>`（`SQLParamData`），形状不同但归处相同。
fn delayed_unexpected<T: std::fmt::Debug>(function: &str, result: &SqlResult<T>) -> DmError {
    DmError::at(
        DmErrorKind::Unsupported {
            feature: format!("{function} 在延迟输入路径上返回了非预期结果 {result:?}"),
        },
        Stage::Execute,
    )
}

/// 驱动还回来的 token 不是我们绑进去的那个。
fn delayed_token_mismatch(got: Pointer, expected: Pointer) -> DmError {
    DmError::at(
        DmErrorKind::Unsupported {
            feature: format!(
                "延迟输入参数的 token 对不上（驱动给 {got:p}，绑定时是 {expected:p}）：\
                 无法确定该往哪个参数喂数据，已中止"
            ),
        },
        Stage::Execute,
    )
}

/// 驱动还在要数据，但源已经取尽：再喂下去写进去的就是残缺值。
fn delayed_source_empty() -> DmError {
    DmError::at(
        DmErrorKind::Unsupported {
            feature: "延迟输入参数还在要数据，但数据源已经取尽：拒绝写出残缺的 LOB".to_owned(),
        },
        Stage::Execute,
    )
}

/// `SQLParamData` 要数据的趟数超过上限：驱动不收敛。
fn delayed_too_many_rounds(rounds: usize) -> DmError {
    DmError::at(
        DmErrorKind::Unsupported {
            feature: format!(
                "延迟输入在 {rounds} 趟之后仍在索要数据（上限 MAX_DELAYED_ROUNDS）：驱动不收敛"
            ),
        },
        Stage::Execute,
    )
}

/// 本层调用点上**不该出现**的返回码 → 可行动的 `DmError`；`None` 表示「成功，或该走统一分类」。
///
/// 单拎出这两个的理由：它们不是驱动故障，而是**我们的用法与驱动状态不匹配**，
/// 落回 `Server { sqlstate: "", code: 0, message: "状态不对" }` 对用户毫无用处。
fn misuse_error(res: &SqlResult<()>) -> Option<DmError> {
    match res {
        // `SQL_NEED_DATA`：语句里有等待数据的 delayed 参数。走到这里只有一种原因——
        // 「有 `?` 却一处都没绑定」（比如 `exec_direct` 发出了带 `?` 的 SQL）。真正绑了
        // 延迟参数的那条路径由 `exec_delayed` 自己消费 `NeedData`，不会把它交给本函数。
        SqlResult::NeedData => Some(DmError::at(
            DmErrorKind::Unsupported {
                feature: EXEC_DIRECT_WITH_PARAMS.to_owned(),
            },
            Stage::Execute,
        )),
        // 异步执行中。达梦**不支持** ODBC 异步（§4.5：设 `SQL_ATTR_ASYNC_ENABLE` 返
        // `HYC00/-70054`，`SQL_ASYNC_MODE = SQL_AM_NONE`），所以本层永远不该看到它。
        SqlResult::StillExecuting => Some(DmError::at(
            DmErrorKind::Unsupported {
                feature: ASYNC_EXECUTION.to_owned(),
            },
            Stage::Execute,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 影响行数：负数必须变成 `None`，不能夹成 0 ----

    #[test]
    fn non_negative_row_counts_are_reported() {
        assert_eq!(affected_rows_from(0), Some(0));
        assert_eq!(affected_rows_from(1), Some(1));
        assert_eq!(affected_rows_from(1_000_000), Some(1_000_000));
    }

    #[test]
    fn negative_row_counts_become_none_not_zero() {
        // -1 是「驱动没说」，0 是「确实零行」——两者混起来会让 update 的调用方误判。
        assert_eq!(affected_rows_from(-1), None);
        // SQL_NO_TOTAL
        assert_eq!(affected_rows_from(-4), None);
        assert_eq!(affected_rows_from(isize::MIN), None);
    }

    // ---- 产出形态 ----

    #[test]
    fn result_set_outcome_reports_columns() {
        let outcome = StmtOutcome::ResultSet { columns: 3 };
        assert!(outcome.has_result_set());
        assert!(!outcome.is_affected());
        assert_eq!(outcome.columns(), Some(3));
        assert_eq!(outcome.affected_rows(), None);
    }

    #[test]
    fn affected_outcome_never_looks_like_a_result_set() {
        let with_rows = StmtOutcome::Affected { rows: Some(7) };
        assert!(!with_rows.has_result_set());
        assert!(with_rows.is_affected());
        assert_eq!(with_rows.columns(), None);
        assert_eq!(with_rows.affected_rows(), Some(7));

        // 「驱动没说」与「零行」是两回事，都要能表达。
        let unknown = StmtOutcome::Affected { rows: None };
        assert_eq!(unknown.affected_rows(), None);
        assert_eq!(
            StmtOutcome::Affected { rows: Some(0) }.affected_rows(),
            Some(0)
        );
    }

    // ---- 不该出现的返回码 ----

    #[test]
    fn need_data_is_translated_into_actionable_unsupported() {
        let err = misuse_error(&SqlResult::NeedData).expect("必须被判为用法错误");
        assert_eq!(err.context.stage, Stage::Execute);
        match &err.kind {
            DmErrorKind::Unsupported { feature } => {
                // 建议必须出现在错误里：只说「不支持」等于没说。
                assert!(feature.contains("EXEC_DIRECT"), "feature={feature}");
                assert!(feature.contains("prepare()"), "feature={feature}");
                assert!(feature.contains("bind_params()"), "feature={feature}");
            }
            other => panic!("应是 Unsupported，实际 {other:?}"),
        }
    }

    #[test]
    fn still_executing_is_reported_as_async_unsupported() {
        // §4.5：达梦的 SQL_ASYNC_MODE 是 SQL_AM_NONE，异步不可达。
        let err = misuse_error(&SqlResult::StillExecuting).expect("必须被判为异步");
        match &err.kind {
            DmErrorKind::Unsupported { feature } => assert!(feature.contains("ASYNC_ENABLE")),
            other => panic!("应是 Unsupported，实际 {other:?}"),
        }
    }

    #[test]
    fn success_and_real_errors_are_left_to_the_shared_classifier() {
        // 成功不该被这里拦下（否则 `finish_exec` 后面的 NumResultCols 流程就跑不到）。
        assert!(misuse_error(&SqlResult::Success(())).is_none());
        assert!(misuse_error(&SqlResult::SuccessWithInfo(())).is_none());
        // 真正的服务端错误要留给 `diag::collect_and_classify`：只有它手上有句柄、能取全部诊断。
        assert!(
            misuse_error(&SqlResult::Error {
                function: "SQLExecute"
            })
            .is_none()
        );
        assert!(misuse_error(&SqlResult::NoData).is_none());
    }

    // ---- 来源分类 ----

    #[test]
    fn only_prepared_statements_are_repeatable() {
        assert!(StmtKind::Prepared.is_prepared());
        // `exec()` 在这两种状态下必须拒绝：句柄上没有预处理计划。
        assert!(!StmtKind::Fresh.is_prepared());
        assert!(!StmtKind::Direct.is_prepared());
    }
}
