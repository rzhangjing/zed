//! `dm-query` 集成测试的公共夹具：一个**离线假后端** + 池装配。
//!
//! 为什么不去 `#[path]` 引 `dm-pool/src/testing.rs`：那个假后端是为**池状态机**造的
//! （`execute` 恒返回 `Affected`、`fetch` 恒 `Exhausted`），根本产不出结果集，
//! 而这一层要测的正是「结果集怎么变成 `Row`/`RowSet`/`RowStream`」。所以这里自己造一个
//! **能产结果集、能分批、能注入失败**的假后端，把 `dm-query` 的逻辑完全离线钉住。
//!
//! 真机行为（`-70027`、达梦的 `LENGTH` 口径、`v$sessions`）不在这一层：那些属于
//! `crates/dm-odbc/tests/` 与 `crates/dm-pool/tests/g4_real.rs`，以及本 crate 的
//! `tests/query_real.rs`（`#[ignore]` + `DM_TEST_DSN`）。

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use dm_core::{
    Backend, ColumnMeta, DataType, DmConfig, DmValue, ExecOutcome, FetchState, LobChunk, LobKind,
    LobParam, LobSource, ParamArrayOutcome, RowBuf,
};
use dm_error::{DiagRecord, DmError, DmErrorKind, Stage};
use dm_pool::{DmPool, ExecutorConfig};

/// 测试配置里放登记号的键（跨线程把 `Arc<FakeShared>` 递给 `Backend::connect`）。
pub const FAKE_TOKEN_KEY: &str = "fake.token";

static REGISTRY: OnceLock<Mutex<HashMap<u64, Arc<FakeShared>>>> = OnceLock::new();
static NEXT_TOKEN: AtomicUsize = AtomicUsize::new(1);

fn registry() -> &'static Mutex<HashMap<u64, Arc<FakeShared>>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 登记一份共享状态，返回代号（放进 `DmConfig::extra`）。
pub fn register(shared: Arc<FakeShared>) -> u64 {
    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed) as u64;
    if let Ok(mut map) = registry().lock() {
        map.insert(token, shared);
    }
    token
}

/// 按代号取回共享状态。
pub fn shared_of(token: u64) -> Option<Arc<FakeShared>> {
    registry().lock().ok()?.get(&token).cloned()
}

/// 从配置里取回共享状态（`Backend::connect` 只能拿到 `&DmConfig`）。
///
/// 之所以要绕这一圈：`connect` 跑在池的**阻塞线程**上，调用方的 thread-local 在那边
/// 看不见（这个坑在 `dm-pool` 的假后端里踩过一次，这里沿用登记表方案）。
pub fn shared_for_config(cfg: &DmConfig) -> Result<Arc<FakeShared>, DmError> {
    cfg.extra
        .get(FAKE_TOKEN_KEY)
        .and_then(|raw| raw.parse::<u64>().ok())
        .and_then(shared_of)
        .ok_or_else(|| {
            DmError::at(
                DmErrorKind::Config("测试配置里缺少（或无效的）fake.token".to_owned()),
                Stage::Connect,
            )
        })
}

/// 一列元数据。
pub fn column(name: &str, data_type: DataType) -> ColumnMeta {
    ColumnMeta {
        name: name.to_owned(),
        data_type,
        nullable: true,
        precision: None,
        scale: None,
        column_size: None,
        length_in_char: None,
    }
}

/// 若干列（`(列名, 类型)`）。
pub fn columns(spec: &[(&str, DataType)]) -> Arc<[ColumnMeta]> {
    spec.iter()
        .map(|(name, ty)| column(name, *ty))
        .collect::<Vec<_>>()
        .into()
}

/// 假后端的一次「执行计划」。
#[derive(Debug)]
pub struct Plan {
    /// 结果集列（`select` 用）。
    pub columns: Arc<[ColumnMeta]>,
    /// 结果集数据。
    pub rows: Vec<Vec<DmValue>>,
    /// 每次 `fetch` 返回多少行（模拟驱动行集大小）。
    pub batch_rows: usize,
    /// 非查询语句的影响行数。
    pub affected: Option<u64>,
    /// 前几次 `execute` 失败——**断连类**（`Io`，`is_connection_lost() == true`）。
    pub fail_execute_times: usize,
    /// 前几次 `execute` 失败——**普通错**（`Syntax`，连接没坏，只是这句话不对）。
    pub fail_execute_plain_times: usize,
    /// 让**第几次** `fetch` 失败（0 起算）；`None` = 从不失败。
    pub fail_fetch_on: Option<usize>,
    /// 语句的 `?` 个数（`Backend::param_count` 的返回值）。
    ///
    /// 默认 `None`＝「后端答不上来」，这正是 `execute_param_array` **默认实现**那类
    /// 后端的行为；要测数组绑定路径就把它设成与每组参数个数一致的值。
    pub param_count: Option<u16>,
    /// 覆盖 `execute_param_array`（列式数组绑定）。
    ///
    /// 默认 `false`：本假后端**不**覆盖那个方法，于是 `dm-query` 会走 trait 的默认
    /// 实现（逐行 `bind_params` + `execute`）——那是「不支持数组绑定的后端」的等价物。
    /// 设成 `true` 就变成「能数组绑定」的后端，用来断言走了哪条路、以及
    /// `ParamArrayOutcome` 的三个字段怎么进 `BatchOutcome`。
    pub array_binding: bool,
    /// 数组绑定一次 `SQLExecute` 的影响行数。
    pub array_affected: u64,
    /// 数组绑定里成功执行的组数（`ParamArrayOutcome::executed`）。
    pub array_executed: usize,
    /// 数组绑定里失败组的下标（`ParamArrayOutcome::failed_at`）。
    pub array_failed_at: Option<usize>,
    /// 分块读（`Backend::lob_start`/`lob_next_row`/`lob_chunk`）的**逐行取值**。
    ///
    /// 下标与 `rows` 对齐；`None` 表示这一行的 LOB 列是 SQL NULL（不是空值）。
    /// 取值与 `rows` 分开摆，是因为分块读**根本不经过行缓冲**——这正是它存在的理由。
    pub lob_rows: Vec<Option<Vec<u8>>>,
    /// 每次 `lob_chunk` 最多吐出多少字节（模拟驱动的取数窗口）。
    pub lob_chunk_bytes: usize,
    /// 前几次 `lob_chunk` 失败——**断连类**（`Io`，`is_connection_lost() == true`）。
    pub lob_fail_times: usize,
    /// 让 `lob_start` 失败（`Syntax`：连接没坏，只是这一列读不了）。
    pub lob_start_fails: bool,
    /// 前几次分块**写**（`Backend::execute_lob`）在**喂到一半**失败——**断连类**
    /// （`Io`，`is_connection_lost() == true`）：真驱动上这意味着语句停在
    /// `SQL_NEED_DATA` 中间，池必须丢弃连接。
    ///
    /// 「喂到一半」是重点：假后端会先把已收到的字节记进 `FakeShared::lob_written`，
    /// 再报错——这样「失败后语句不能进缓存」与「连接被丢弃」才有东西可断言。
    pub lob_write_fail_times: usize,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            columns: columns(&[("ID", DataType::Integer)]),
            rows: Vec::new(),
            batch_rows: 4,
            affected: Some(0),
            fail_execute_times: 0,
            fail_execute_plain_times: 0,
            fail_fetch_on: None,
            param_count: None,
            array_binding: false,
            array_affected: 0,
            array_executed: 0,
            array_failed_at: None,
            lob_rows: Vec::new(),
            lob_chunk_bytes: 4,
            lob_fail_times: 0,
            lob_start_fails: false,
            lob_write_fail_times: 0,
        }
    }
}

/// 假后端的共享状态（所有测试断言都读它）。
#[derive(Debug)]
pub struct FakeShared {
    /// 调用流水：`prepare#<sql>` / `bind#<n>` / `execute#<sql>` / `fetch#<n>` /
    /// `close_cursor#<sql>` / `reset#rollback` / `reset` / `begin` / `commit` /
    /// `rollback` / `ping` / `connect#<id>` / `close#<id>`。
    pub ops: Mutex<Vec<String>>,
    pub connects: AtomicUsize,
    pub closes: AtomicUsize,
    pub resets: AtomicUsize,
    pub begins: AtomicUsize,
    pub commits: AtomicUsize,
    pub rollbacks: AtomicUsize,
    /// 复位（`Backend::reset`）内部做的**服务端回滚**次数。
    ///
    /// 与 `rollbacks` 分开记，而不是混在一起：真的 `OdbcBackend::reset()` 就是
    /// `conn.rollback()? + set_autocommit(true)`（`crates/dm-odbc/src/lib.rs:202`），
    /// 所以「池复位一次」在服务端**确实**留下一次回滚；但把它算进 `rollbacks` 会
    /// 让「`commit` 之后不该有回滚」这类断言失去意义（提交后连接也要复位）。
    pub reset_rollbacks: AtomicUsize,
    pub fetch_calls: AtomicUsize,
    pub closes_cursor: AtomicUsize,
    /// `execute_param_array`（列式数组绑定）**真正被调过**几次。
    ///
    /// 这是 T6.4 的核心探针：不数它就只能断言「结果对」，而「结果对」在逐行路径上
    /// 也一样对——走过没走过数组绑定根本无从观察。
    pub param_array_calls: AtomicUsize,
    /// 数组绑定收到的**列数**流水（每次调用一笔，断言转置形状用）。
    pub param_array_widths: Mutex<Vec<usize>>,
    /// 数组绑定收到的**行数**流水。
    pub param_array_rows: Mutex<Vec<usize>>,
    /// `Backend::lob_start` 被调过几次。
    pub lob_starts: AtomicUsize,
    /// `Backend::lob_chunk` 被调过几次（含失败的那些，`lob_fail_times` 按这个序号算）。
    pub lob_chunks: AtomicUsize,
    /// `Backend::execute_lob` 被调过几次。
    pub lob_writes: AtomicUsize,
    /// 每次分块写**实际收到的字节**（按调用顺序）。
    ///
    /// 文本源这里记的是源给的 **UTF-8** 字节——假后端不做 UTF-16 转换，那是驱动层的事
    /// （`crates/dm-odbc/src/lob.rs` 的 `LobFeeder` 有自己的单测钉它）。
    pub lob_written: Mutex<Vec<Vec<u8>>>,
    /// 每次分块写的**值类型**（按调用顺序）。
    pub lob_write_kinds: Mutex<Vec<LobKind>>,
    /// 每次分块写的 **`?` 下标**（按调用顺序）。
    pub lob_write_columns: Mutex<Vec<usize>>,
    pub ping_alive: AtomicBool,
    plan: Mutex<Plan>,
}

impl FakeShared {
    /// 建一份共享状态，并把登记号写进返回的配置里。
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            ops: Mutex::new(Vec::new()),
            connects: AtomicUsize::new(0),
            closes: AtomicUsize::new(0),
            resets: AtomicUsize::new(0),
            begins: AtomicUsize::new(0),
            commits: AtomicUsize::new(0),
            rollbacks: AtomicUsize::new(0),
            reset_rollbacks: AtomicUsize::new(0),
            fetch_calls: AtomicUsize::new(0),
            closes_cursor: AtomicUsize::new(0),
            param_array_calls: AtomicUsize::new(0),
            param_array_widths: Mutex::new(Vec::new()),
            param_array_rows: Mutex::new(Vec::new()),
            lob_starts: AtomicUsize::new(0),
            lob_chunks: AtomicUsize::new(0),
            lob_writes: AtomicUsize::new(0),
            lob_written: Mutex::new(Vec::new()),
            lob_write_kinds: Mutex::new(Vec::new()),
            lob_write_columns: Mutex::new(Vec::new()),
            ping_alive: AtomicBool::new(true),
            plan: Mutex::new(Plan::default()),
        })
    }

    /// 数组绑定被调用了几次。
    #[must_use]
    pub fn param_array_calls(&self) -> usize {
        self.param_array_calls.load(Ordering::Relaxed)
    }

    /// 每次数组绑定的（列数, 行数）。
    #[must_use]
    pub fn param_array_shape(&self) -> Vec<(usize, usize)> {
        let widths = self
            .param_array_widths
            .lock()
            .map(|w| w.clone())
            .unwrap_or_default();
        let rows = self
            .param_array_rows
            .lock()
            .map(|r| r.clone())
            .unwrap_or_default();
        widths.into_iter().zip(rows).collect()
    }

    /// `Backend::lob_start` 被调过几次。
    #[must_use]
    pub fn lob_starts(&self) -> usize {
        self.lob_starts.load(Ordering::Relaxed)
    }

    /// `Backend::lob_chunk` 被调过几次（含失败的）。
    #[must_use]
    pub fn lob_chunks(&self) -> usize {
        self.lob_chunks.load(Ordering::Relaxed)
    }

    /// `Backend::execute_lob` 被调过几次。
    #[must_use]
    pub fn lob_writes(&self) -> usize {
        self.lob_writes.load(Ordering::Relaxed)
    }

    /// 每次分块写**实际收到的字节**（按调用顺序）。
    #[must_use]
    pub fn lob_written(&self) -> Vec<Vec<u8>> {
        self.lob_written
            .lock()
            .map(|w| w.clone())
            .unwrap_or_default()
    }

    /// 每次分块写的值类型（按调用顺序）。
    #[must_use]
    pub fn lob_write_kinds(&self) -> Vec<LobKind> {
        self.lob_write_kinds
            .lock()
            .map(|k| k.clone())
            .unwrap_or_default()
    }

    /// 每次分块写的 `?` 下标（按调用顺序）。
    #[must_use]
    pub fn lob_write_columns(&self) -> Vec<usize> {
        self.lob_write_columns
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default()
    }

    /// 改执行计划。
    pub fn configure(&self, f: impl FnOnce(&mut Plan)) {
        if let Ok(mut plan) = self.plan.lock() {
            f(&mut plan);
        }
    }

    /// 取一份计划快照（断言用）。
    #[must_use]
    pub fn plan(&self) -> Plan {
        self.plan.lock().map(|p| p.clone_plan()).unwrap_or_default()
    }

    /// 记一笔流水。
    pub fn note(&self, entry: impl Into<String>) {
        if let Ok(mut ops) = self.ops.lock() {
            ops.push(entry.into());
        }
    }

    /// 流水快照。
    #[must_use]
    pub fn op_log(&self) -> Vec<String> {
        self.ops.lock().map(|ops| ops.clone()).unwrap_or_default()
    }
}

impl Plan {
    fn clone_plan(&self) -> Self {
        Self {
            columns: Arc::clone(&self.columns),
            rows: self.rows.clone(),
            batch_rows: self.batch_rows,
            affected: self.affected,
            fail_execute_times: self.fail_execute_times,
            fail_execute_plain_times: self.fail_execute_plain_times,
            fail_fetch_on: self.fail_fetch_on,
            param_count: self.param_count,
            array_binding: self.array_binding,
            array_affected: self.array_affected,
            array_executed: self.array_executed,
            array_failed_at: self.array_failed_at,
            lob_rows: self.lob_rows.clone(),
            lob_chunk_bytes: self.lob_chunk_bytes,
            lob_fail_times: self.lob_fail_times,
            lob_start_fails: self.lob_start_fails,
            lob_write_fail_times: self.lob_write_fail_times,
        }
    }

    fn take_execute_failure(&mut self) -> bool {
        if self.fail_execute_times > 0 {
            self.fail_execute_times -= 1;
            true
        } else {
            false
        }
    }

    fn take_execute_plain_failure(&mut self) -> bool {
        if self.fail_execute_plain_times > 0 {
            self.fail_execute_plain_times -= 1;
            true
        } else {
            false
        }
    }
}

/// 假语句句柄：记住 SQL 与游标位置。
#[derive(Debug)]
pub struct FakeStmt {
    pub sql: String,
    pub cursor: usize,
    pub columns: Arc<[ColumnMeta]>,
    pub closed: bool,
    /// 分块读的游标（`lob_start` 之后才有）。
    pub lob: Option<LobCursor>,
}

/// 分块读游标：正在读第几行、读到这一行的第几个字节。
///
/// 刻意**挂在语句上**而不是 `dm-query` 侧：真实的 `OdbcStatement` 就是这么放的
/// （`crates/dm-odbc/src/stmt.rs` 的 `lob: Option<LobState>`），所以假后端也照这个形状
/// 摆——否则「换一条语句」与「换一个行」在测试里分不出来。
#[derive(Debug, Default)]
pub struct LobCursor {
    /// 当前行（`0` 起算）。
    pub row: usize,
    /// 这一行已经吐出的字节数。
    pub offset: usize,
}

/// 假后端。
#[derive(Debug)]
pub struct FakeBackend {
    shared: Arc<FakeShared>,
    id: usize,
}

impl FakeBackend {
    /// 共享状态（断言用）。
    #[must_use]
    pub fn shared(&self) -> &Arc<FakeShared> {
        &self.shared
    }
}

fn is_select(sql: &str) -> bool {
    sql.trim_start().to_ascii_lowercase().starts_with("select")
}

/// `dm_core::Backend::execute_param_array` 默认体（逐行 `bind_params` + `execute`）的
/// 等价物，供假后端在「假装没覆盖这个方法」时使用。
///
/// 口径与默认体逐条对齐：**行级**失败（某行的 `execute` 报错，**含第 0 行**——
/// 违约就是违约，与服务端在第几行挡下无关）记进 `failed_at` 后停下，返回 `Ok`，
/// 前面已生效的照实计数；**语句级**失败（`bind_params` 报错）才立即 `Err`；
/// 行内出现结果集就关游标并算这一条成功。
fn row_loop_like_the_default_impl(
    backend: &mut FakeBackend,
    stmt: &mut FakeStmt,
    columns: &[Vec<DmValue>],
) -> Result<ParamArrayOutcome, DmError> {
    let rows = columns.iter().map(Vec::len).min().unwrap_or(0);
    let mut outcome = ParamArrayOutcome::default();
    let mut args: Vec<DmValue> = Vec::with_capacity(columns.len());
    for index in 0..rows {
        args.clear();
        args.extend(columns.iter().map(|column| column[index].clone()));
        // 绑不上是**语句级**问题（参数形状/类型与语句不符），没有账可记。
        backend.bind_params(stmt, &args)?;

        let executed = match backend.execute(stmt, "") {
            Ok(ExecOutcome::Affected { rows }) => {
                outcome.affected += rows.unwrap_or(0);
                true
            }
            Ok(ExecOutcome::ResultSet { .. }) => {
                backend.close_cursor(stmt)?;
                true
            }
            Ok(_) => false,
            Err(_) => {
                outcome.failed_at = Some(index);
                false
            }
        };
        if executed {
            outcome.executed += 1;
        } else {
            break;
        }
    }
    Ok(outcome)
}

impl Backend for FakeBackend {
    type Stmt = FakeStmt;

    fn name(&self) -> &'static str {
        "fake-query"
    }

    fn connect(cfg: &DmConfig) -> Result<Self, DmError> {
        let shared = shared_for_config(cfg)?;
        let id = shared.connects.fetch_add(1, Ordering::Relaxed) + 1;
        shared.note(format!("connect#{id}"));
        Ok(Self { shared, id })
    }

    fn ping(&mut self) -> Result<bool, DmError> {
        self.shared.note("ping");
        Ok(self.shared.ping_alive.load(Ordering::Relaxed))
    }

    fn prepare(&mut self, sql: &str) -> Result<Self::Stmt, DmError> {
        self.shared.note(format!("prepare#{sql}"));
        // 只借计划的 `columns`（`Arc` 一拷即得）。**别**用 `self.shared.plan()`：那是整份
        // `Plan` 的深拷贝（`rows` / `lob_rows` 可能是 MiB 级），
        // `crates/dm-query/tests/alloc_budget.rs` 量内存预算时会被它算歪。
        let plan = self.shared.plan.lock().map_err(|_| poisoned())?;
        Ok(FakeStmt {
            sql: sql.to_owned(),
            cursor: 0,
            columns: if is_select(sql) {
                Arc::clone(&plan.columns)
            } else {
                Arc::from(Vec::<ColumnMeta>::new())
            },
            closed: false,
            lob: None,
        })
    }

    fn bind_params(&mut self, _stmt: &mut Self::Stmt, args: &[DmValue]) -> Result<(), DmError> {
        self.shared.note(format!("bind#{}", args.len()));
        Ok(())
    }

    /// 只在 `Plan::param_count` 配了值时才答得上——默认 `None` 就是「后端答不上来」，
    /// 与 `dm-core` 的默认实现一致。
    fn param_count(&mut self, _stmt: &Self::Stmt) -> Option<u16> {
        // 同上：要的是一个 `Copy` 的标量，不值得为此深拷贝整份计划。
        self.shared
            .plan
            .lock()
            .map(|plan| plan.param_count)
            .unwrap_or_default()
    }

    /// 覆盖（或不覆盖）数组绑定，取决于 `Plan::array_binding`。
    ///
    /// # 为什么「不覆盖」这一支要自己写循环，而不是转调 trait 的默认体
    ///
    /// 第一版这里写的是 `Backend::execute_param_array(self, stmt, columns)`，赌
    /// 「完全限定语法能拿到默认体」。**赌错了**：默认体不是独立函数，它一旦被
    /// `FakeBackend` override，`Backend::execute_param_array` 解析到的还是这个
    /// override——于是它自己调自己，测试进程直接
    /// `STATUS_STACK_OVERFLOW`（0xC00000FD，崩在 `dm-pool-blocking-1`，那条
    /// 512 KiB 栈的池线程上）。trait 默认体在 Rust 里是**够不着**的。
    ///
    /// 所以这里改为调 [`row_loop_like_the_default_impl`]：把默认体那几行原样照抄成
    /// 独立函数。语义与「真的没 override」一致，而且这个假后端能同时表达两条路。
    fn execute_param_array(
        &mut self,
        stmt: &mut Self::Stmt,
        columns: &[Vec<DmValue>],
    ) -> Result<ParamArrayOutcome, DmError> {
        // 只取几个 `Copy` 的标量就把锁放掉：下面要调 `self` 的 `&mut` 方法，拿着 guard 借不动。
        let (array_binding, array_affected, array_executed, array_failed_at) = {
            let plan = self.shared.plan.lock().map_err(|_| poisoned())?;
            (
                plan.array_binding,
                plan.array_affected,
                plan.array_executed,
                plan.array_failed_at,
            )
        };
        if !array_binding {
            // 「没覆盖这个方法的后端」——逐行 bind + execute。
            return row_loop_like_the_default_impl(self, stmt, columns);
        }

        let rows = columns.first().map(Vec::len).unwrap_or(0);
        self.shared
            .param_array_calls
            .fetch_add(1, Ordering::Relaxed);
        if let Ok(mut widths) = self.shared.param_array_widths.lock() {
            widths.push(columns.len());
        }
        if let Ok(mut recorded) = self.shared.param_array_rows.lock() {
            recorded.push(rows);
        }
        self.shared
            .note(format!("execute_param_array#{}x{}", columns.len(), rows));

        Ok(ParamArrayOutcome {
            affected: array_affected,
            executed: array_executed,
            failed_at: array_failed_at,
        })
    }

    fn execute(&mut self, stmt: &mut Self::Stmt, sql: &str) -> Result<ExecOutcome, DmError> {
        let effective = if sql.is_empty() {
            stmt.sql.clone()
        } else {
            sql.to_owned()
        };
        self.shared.note(format!("execute#{effective}"));
        let mut plan = self.shared.plan.lock().map_err(|_| poisoned())?;
        if plan.take_execute_plain_failure() {
            let mut err = DmError::at(DmErrorKind::Syntax, Stage::Execute);
            err.push_diag(DiagRecord::native(
                0,
                "假后端：按配置让这次执行失败（语法错，连接没坏）",
            ));
            return Err(err);
        }
        if plan.take_execute_failure() {
            let mut err = DmError::at(DmErrorKind::Io, Stage::Execute);
            err.push_diag(DiagRecord::native(
                0,
                "假后端：按配置让这次执行失败（按连接丢失处理）",
            ));
            return Err(err);
        }
        if is_select(&effective) {
            stmt.cursor = 0;
            stmt.closed = false;
            // 与真驱动一致：重新执行会让上一轮的分块读状态作废
            // （`crates/dm-odbc/src/stmt.rs` 的 `exec`/`exec_direct` 都清 `lob`）。
            stmt.lob = None;
            Ok(ExecOutcome::ResultSet {
                columns: Arc::clone(&stmt.columns),
            })
        } else {
            Ok(ExecOutcome::Affected {
                rows: plan.affected,
            })
        }
    }

    fn columns(&mut self, stmt: &Self::Stmt) -> Result<Arc<[ColumnMeta]>, DmError> {
        Ok(Arc::clone(&stmt.columns))
    }

    fn fetch(&mut self, stmt: &mut Self::Stmt, into: &mut RowBuf) -> Result<FetchState, DmError> {
        // 0 起算的调用序号：`fetch_stream` 的首批也是从这里过的（`start` 里等的那次）。
        let call = self.shared.fetch_calls.fetch_add(1, Ordering::Relaxed);
        let plan = self.shared.plan.lock().map_err(|_| poisoned())?;
        if plan.fail_fetch_on == Some(call) {
            let mut err = DmError::at(DmErrorKind::Io, Stage::Fetch);
            err.push_diag(DiagRecord::native(
                0,
                "假后端：按配置让这次取数失败（按连接丢失处理）",
            ));
            return Err(err);
        }
        let remaining = plan.rows.len().saturating_sub(stmt.cursor);
        if remaining == 0 {
            self.shared.note("fetch#0");
            return Ok(FetchState::Exhausted);
        }
        let take = remaining.min(plan.batch_rows.max(1));
        into.clear();
        for row in &plan.rows[stmt.cursor..stmt.cursor + take] {
            into.push_row(row)?;
        }
        stmt.cursor += take;
        self.shared.note(format!("fetch#{take}"));
        Ok(FetchState::Rows { rows: take })
    }

    fn close_cursor(&mut self, stmt: &mut Self::Stmt) -> Result<(), DmError> {
        self.shared.closes_cursor.fetch_add(1, Ordering::Relaxed);
        self.shared.note(format!("close_cursor#{}", stmt.sql));
        stmt.closed = true;
        stmt.lob = None;
        Ok(())
    }

    fn lob_start(&mut self, stmt: &mut Self::Stmt, column: usize) -> Result<(), DmError> {
        self.shared.note(format!("lob_start#{column}"));
        // 先记账再判失败：这个计数器数的是**尝试**次数（`lob_fail_times`/`lob_start_fails`
        // 这类注入都发生在被调用之后），否则「失败之后有没有重来一次」就观察不到了。
        self.shared.lob_starts.fetch_add(1, Ordering::Relaxed);
        // 借锁直读计划，不走 `FakeShared::plan()` 的快照：快照是**深拷贝**，而
        // `lob_rows` 里的载荷可能是 MiB 级。假后端的热路径必须像真驱动那样「只借一块
        // 窗口」，否则测内存预算的用例（`tests/alloc_budget.rs`）量到的是夹具自己。
        // `fetch` 早就是这个写法（`self.shared.plan.lock()`）。
        let plan = self.shared.plan.lock().map_err(|_| poisoned())?;
        if plan.lob_start_fails {
            let mut err = DmError::at(DmErrorKind::Syntax, Stage::Fetch);
            err.push_diag(DiagRecord::native(
                0,
                "假后端：按配置让 lob_start 失败（连接没坏）",
            ));
            return Err(err);
        }
        if column >= stmt.columns.len() {
            return Err(DmError::at(
                DmErrorKind::Config(format!(
                    "假后端：分块读的列号 {column} 越界（这条语句有 {} 列）",
                    stmt.columns.len()
                )),
                Stage::Fetch,
            ));
        }
        // `lob_next_row` 已经把游标推过了，所以「当前行」是 `cursor - 1`：
        // 真驱动这边是 `SQLGetData` 直接读定位中的行，没有第二套行号。
        let row = stmt.cursor.checked_sub(1).ok_or_else(|| {
            DmError::at(
                DmErrorKind::Config(
                    "假后端：lob_start 之前没有定位到行（要先 lob_next_row）".to_owned(),
                ),
                Stage::Fetch,
            )
        })?;
        stmt.lob = Some(LobCursor { row, offset: 0 });
        Ok(())
    }

    fn lob_next_row(&mut self, stmt: &mut Self::Stmt) -> Result<bool, DmError> {
        // 同 `lob_start`：借锁直读，不深拷贝计划。
        let plan = self.shared.plan.lock().map_err(|_| poisoned())?;
        if stmt.cursor >= plan.rows.len() {
            self.shared.note("lob_next_row#0");
            return Ok(false);
        }
        stmt.cursor += 1;
        // 换行 ⇒ 上一行的分块读游标作废（真驱动里 `SQLFetch` 会清掉 `SQLGetData` 状态）。
        stmt.lob = None;
        self.shared.note(format!("lob_next_row#{}", stmt.cursor));
        Ok(true)
    }

    fn lob_chunk(&mut self, stmt: &mut Self::Stmt, out: &mut Vec<u8>) -> Result<LobChunk, DmError> {
        let call = self.shared.lob_chunks.fetch_add(1, Ordering::Relaxed);
        // 同 `lob_start`：借锁直读，不深拷贝计划（`payload` 就直接借在这把锁上）。
        let plan = self.shared.plan.lock().map_err(|_| poisoned())?;
        if call < plan.lob_fail_times {
            let mut err = DmError::at(DmErrorKind::Io, Stage::Fetch);
            err.push_diag(DiagRecord::native(
                0,
                "假后端：按配置让这次分块读失败（按连接丢失处理）",
            ));
            return Err(err);
        }
        let cursor = stmt.lob.as_mut().ok_or_else(|| {
            DmError::at(
                DmErrorKind::Config(
                    "假后端：分块读没有从 lob_start 开始（同一列混用了绑定与自定义取数？）"
                        .to_owned(),
                ),
                Stage::Fetch,
            )
        })?;
        let Some(payload) = plan.lob_rows.get(cursor.row).and_then(Option::as_ref) else {
            self.shared.note("lob_chunk#null");
            return Ok(LobChunk::Null);
        };
        if cursor.offset >= payload.len() {
            self.shared.note("lob_chunk#done");
            return Ok(LobChunk::Done);
        }
        let end = (cursor.offset + plan.lob_chunk_bytes.max(1)).min(payload.len());
        out.extend_from_slice(&payload[cursor.offset..end]);
        let taken = end - cursor.offset;
        cursor.offset = end;
        self.shared.note(format!("lob_chunk#{taken}"));
        Ok(LobChunk::More)
    }

    /// 分块写：把源一趟一趟**抽干**，收到的字节记进 `FakeShared::lob_written`。
    ///
    /// 与真后端对齐的三点：
    ///
    /// 1. 参数是「借一处 LOB」——`args` 里**没有**它，`?` 总个数 = `args.len() + 1`；
    ///    `lob.column` 是它在 `?` 里的下标（0 起算），它可以落在任意位置；
    /// 2. 源是**追加**语义、一趟一趟给（不要求一次给完），`false` = 取尽；
    /// 3. `Plan::lob_write_fail_times` 会让「喂到一半」失败：已收到的字节照记
    ///    （`lob_written` 里留半份），错误按**断连类**（`Io`）报——真驱动上此时语句
    ///    停在 `SQL_NEED_DATA`，池必须丢弃这条连接。
    ///
    /// 这里**不**做 UTF-16 转换：`LobKind::Text` 的源按契约给 UTF-8，转换是驱动层的事
    /// （`crates/dm-odbc/src/lob.rs` 的 `LobFeeder` 有自己的单测钉转换与切块）。
    fn execute_lob(
        &mut self,
        stmt: &mut Self::Stmt,
        sql: &str,
        args: &[DmValue],
        lob: &mut LobParam<&mut dyn LobSource>,
    ) -> Result<ExecOutcome, DmError> {
        let effective = if sql.is_empty() {
            stmt.sql.clone()
        } else {
            sql.to_owned()
        };
        self.shared.note(format!("execute_lob#{effective}"));
        self.shared.lob_writes.fetch_add(1, Ordering::Relaxed);
        // 只借需要的三个标量。**别** `self.shared.plan()`：那是整份 `Plan` 的深拷贝
        // （写侧计划的 `rows` / `lob_rows` 可能是 MiB 级，内存预算用例会被它算歪），
        // 而且下面还要 `self.shared.configure(..)`——同一把锁，拿着 guard 就是自锁。
        let (param_count, affected, fail) = {
            let plan = self.shared.plan.lock().map_err(|_| poisoned())?;
            (
                plan.param_count,
                plan.affected,
                plan.lob_write_fail_times > 0,
            )
        };

        // 形状校验：口径与 `crates/dm-odbc/src/param.rs` 的 `bind_params_with_lob` 一致。
        // 假后端的 `param_count` 默认 `None`（答不上来），那时只校验「至少一处 LOB」。
        let expected = param_count.map(usize::from).unwrap_or(args.len() + 1);
        if args.len() + 1 != expected {
            return Err(DmError::at(
                DmErrorKind::Config(format!(
                    "假后端：语句有 {expected} 个 `?`，但收到 {} 个立即参数 + 1 处分块 LOB",
                    args.len()
                )),
                Stage::Bind,
            ));
        }
        if lob.column >= expected {
            return Err(DmError::at(
                DmErrorKind::Config(format!(
                    "假后端：分块 LOB 的列下标 {} 越界（合法下标 0..{expected}）",
                    lob.column
                )),
                Stage::Bind,
            ));
        }

        if fail {
            self.shared.configure(|plan| {
                plan.lob_write_fail_times = plan.lob_write_fail_times.saturating_sub(1);
            });
        }

        let mut collected = Vec::new();
        let mut chunks = 0usize;
        let mut failure: Option<DmError> = None;
        loop {
            // 「喂到一半失败」：收下第一批就先不收了。
            if fail && chunks == 1 {
                break;
            }
            match lob.source.next_chunk(&mut collected) {
                Ok(true) => chunks += 1,
                Ok(false) => break,
                Err(err) => {
                    // 源自己读不出来：已收到的半份照样记账（诊断用），错误原样上报。
                    failure = Some(err);
                    break;
                }
            }
        }
        self.shared.note(format!(
            "lob_write#{}#col{}#{}",
            collected.len(),
            lob.column,
            if fail { "failed" } else { "ok" }
        ));
        if let Ok(mut written) = self.shared.lob_written.lock() {
            written.push(collected);
        }
        if let Ok(mut kinds) = self.shared.lob_write_kinds.lock() {
            kinds.push(lob.kind);
        }
        if let Ok(mut columns) = self.shared.lob_write_columns.lock() {
            columns.push(lob.column);
        }

        if let Some(err) = failure {
            return Err(err);
        }
        if fail {
            let mut err = DmError::at(DmErrorKind::Io, Stage::Execute);
            err.push_diag(DiagRecord::native(
                0,
                "假后端：按配置让这次分块写在喂数据途中失败（按连接丢失处理）",
            ));
            return Err(err);
        }
        // 与真驱动一致：重新执行会让上一轮的分块读状态作废。
        stmt.lob = None;
        Ok(ExecOutcome::Affected { rows: affected })
    }

    fn begin(&mut self) -> Result<(), DmError> {
        self.shared.begins.fetch_add(1, Ordering::Relaxed);
        self.shared.note("begin");
        Ok(())
    }
    fn commit(&mut self) -> Result<(), DmError> {
        self.shared.commits.fetch_add(1, Ordering::Relaxed);
        self.shared.note("commit");
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), DmError> {
        self.shared.rollbacks.fetch_add(1, Ordering::Relaxed);
        self.shared.note("rollback");
        Ok(())
    }

    fn reset(&mut self) -> Result<(), DmError> {
        // 照实模拟 `OdbcBackend::reset()`：它做的第一件事就是回滚
        // （`crates/dm-odbc/src/lib.rs:202` 的 `conn.rollback()?`），然后才还原
        // autocommit。所以这里也要记下一次回滚，否则「未提交的事务对象被丢弃后，
        // 服务端确实回了滚」这条判据在离线夹具上根本无从观察。
        self.shared.reset_rollbacks.fetch_add(1, Ordering::Relaxed);
        self.shared.note("reset#rollback");
        self.shared.resets.fetch_add(1, Ordering::Relaxed);
        self.shared.note("reset");
        Ok(())
    }

    fn close(self) {
        self.shared.closes.fetch_add(1, Ordering::Relaxed);
        self.shared.note(format!("close#{}", self.id));
    }
}

fn poisoned() -> DmError {
    DmError::at(
        DmErrorKind::Config("假后端的共享状态被 poison".to_owned()),
        Stage::Other,
    )
}

/// 造一份指向 `shared` 的配置。
#[must_use]
pub fn config(shared: &Arc<FakeShared>, max_size: usize) -> DmConfig {
    let token = register(Arc::clone(shared));
    let mut cfg = DmConfig::from_connection_string(
        "DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=1;UID=FAKE;PWD=FAKE;DBNAME=FAKE",
    )
    .expect("内置的假 DSN 必须能解析");
    cfg.extra
        .insert(FAKE_TOKEN_KEY.to_owned(), token.to_string());
    cfg.pool.max_size = max_size;
    cfg.pool.min_idle = 0;
    cfg.pool.recycle_interval = std::time::Duration::from_secs(3600);
    cfg
}

/// 测试夹具：池 + 共享状态。
pub struct Fixture {
    pub pool: DmPool<FakeBackend>,
    pub shared: Arc<FakeShared>,
}

impl Fixture {
    /// 建池（`connect_with` 会先建第一条连接）。
    pub async fn open(max_size: usize) -> Self {
        let shared = FakeShared::new();
        let cfg = config(&shared, max_size);
        let pool = DmPool::connect_with(cfg, ExecutorConfig::for_pool(max_size.max(2)))
            .await
            .expect("假后端建池必须成功");
        Self { pool, shared }
    }

    /// 借一条连接。
    pub async fn conn(&self) -> dm_query::PooledConn<FakeBackend> {
        dm_query::PooledConn::acquire(&self.pool)
            .await
            .expect("假后端借连接必须成功")
    }

    /// 关池并等在飞操作结束（超时即 panic，把「忘了 drop 连接」暴露成响亮的失败）。
    pub async fn shutdown(self) {
        let stats = self.pool.stats();
        self.pool.close().await.expect("关池必须成功");
        if tokio::time::timeout(std::time::Duration::from_secs(5), self.pool.wait_closed())
            .await
            .is_err()
        {
            panic!("关池后 5 秒仍未收敛：{stats:?}");
        }
    }
}
