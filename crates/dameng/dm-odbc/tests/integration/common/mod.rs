//! L3 真机集成测试的公共夹具（设计方案 §10.3）。
//!
//! # 门禁
//!
//! 整组用例都带 `#[ignore]`，理由写在属性里。要跑真机：
//!
//! ```text
//! $env:DM_TEST_DSN="DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA"
//! cargo test -p dm-odbc --test integration -- --include-ignored
//! ```
//!
//! 也可以不设 `DM_TEST_DSN`，改用 `DM_SERVER` / `DM_USER` / `DM_PASSWORD` / `DM_DRIVER`
//! （即 `DmConfig::from_env` 的那套具名变量）。两个都没设时每个用例都会打印跳过原因并返回，
//! **不会**因为「忘了配环境」而变成一片红。
//!
//! # `-70027` 零出现断言（AC-05）
//!
//! [`note_error`] 在**每一个**错误对象上检查原生码，命中 `-70027 无效的游标状态` 就累加一个
//! **线程局部**计数。每个用例结束时 [`Harness::finish`] 比较「本用例开始时的计数」，差不为 0
//! 即判失败。
//!
//! 用线程局部而不是全局原子量：`cargo test` 默认**并行**跑用例，每个用例一个线程，全局计数
//! 会被邻居的错误带偏——实测过（`smoke_select_one_from_dual` 自己一条错都没出，却因为别的
//! 用例的 `-70027` 被判失败）。线程局部量让「差值」精确等于「本用例自己产生的次数」，
//! 既保住了「失败落在真正制造它的用例上」，也没有放弃「全跑一遍必须为零」这条防线。
//!
//! 它一旦出现就说明**驱动有 bug**，不是用户有 bug——这是本项目最有价值的自动化防线之一。

// 夹具里有若干「按需使用」的辅助函数（不同用例用得不一样），统一在这里放行 dead_code，
// 免得 `clippy --all-targets -- -D warnings` 因为某个用例没用 `cell_text` 就判失败。
#![allow(dead_code)]

use std::cell::Cell;
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};

use dm_core::backend::FetchState;
use dm_core::{ColumnMeta, DmConfig, DmValue, RowBuf};
use dm_error::DmError;
use dm_odbc::{
    FetchScratch, OdbcConnection, OdbcStatement, ParamArena, StmtOutcome, bind_params,
    describe_columns, fetch,
};

// ---------------------------------------------------------------------------
// 表名
// ---------------------------------------------------------------------------

/// §7 类型往返用的宽表（19 列，见 `fixtures/schema.sql`）。
pub const T_TYPES: &str = "DM_T_TYPES";
/// 字符集与参数绑定用的小表。
pub const T_STR: &str = "DM_T_STR";
/// 大对象用例。
pub const T_LOB: &str = "DM_T_LOB";
/// 事务用例。
pub const T_TXN: &str = "DM_T_TXN";
/// 错误码采集。
pub const T_ERR: &str = "DM_T_ERR";

// ---------------------------------------------------------------------------
// `-70027` 计数器
// ---------------------------------------------------------------------------

static INVALID_CURSOR: AtomicUsize = AtomicUsize::new(0);

/// 真机用例之间的**全局互斥**。
///
/// `apply_schema()` 会对共享的表做 `DROP` + `CREATE`；`cargo test` 默认多线程跑用例，
/// 两个用例同时改同一张表的定义，达梦会直接回 `HY000 / -7009 对象定义被修改`。
/// 与其让每个用例去抢表名，不如让它们排队——56 个用例串行也就几秒钟。
pub fn db_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // 前面的用例 panic 会毒化锁，但锁保护的只是「别同时动库」，被毒化不影响继续用。
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

thread_local! {
    /// 本线程（= 本用例）观察到的 `-70027` 次数。见模块文档「`-70027` 零出现断言」。
    static THREAD_INVALID_CURSOR: Cell<usize> = const { Cell::new(0) };
}

/// 截至此刻、**本线程**观察到的 `-70027` 次数。
pub fn invalid_cursor_count() -> usize {
    THREAD_INVALID_CURSOR.with(Cell::get)
}

/// 错误的**原生码**（从诊断记录里读，与分类结果无关）。
pub fn native_code(err: &DmError) -> Option<i32> {
    err.server_code_and_message().map(|(code, _)| code)
}

/// 记一次错误：命中 `-70027` 就累加本线程计数（同时累加全局计数供收尾汇总用）。
///
/// 每个拿到 `DmError` 的地方都该过一遍这里——夹具里的执行/取值辅助函数已经这么做了，
/// 测试自己捕获错误时也请调用它（别直接判 `is_err()` 就完事）。
pub fn note_error(err: &DmError) {
    if native_code(err) == Some(dm_error::codes::INVALID_CURSOR) {
        INVALID_CURSOR.fetch_add(1, Ordering::SeqCst);
        THREAD_INVALID_CURSOR.with(|count| count.set(count.get() + 1));
    }
}

/// 把错误渲染成多行文本（分类 + 阶段 + 每条诊断）。
pub fn describe(err: &DmError) -> String {
    let mut out = format!("{} @ {:?}", err.kind, err.context.stage);
    for rec in err.diags() {
        out.push_str(&format!(
            "\n    [{} / {}] {}",
            rec.sqlstate_str(),
            rec.native_code,
            rec.message.trim()
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// 环境与配置
// ---------------------------------------------------------------------------

/// 从环境变量构造测试配置；一个都没配时返回 `None`（调用方据此跳过）。
///
/// 认两套变量，优先级从高到低：
/// 1. `DM_TEST_DSN` —— §10.3 规定的门禁变量，一条完整的 ODBC 连接串；
/// 2. `DM_DRIVER` / `DM_SERVER` / `DM_PORT` / `DM_USER` / `DM_PASSWORD` —— `DmConfig::from_env`
///    那套具名变量。
pub fn test_config() -> Option<DmConfig> {
    if let Ok(dsn) = std::env::var("DM_TEST_DSN") {
        let dsn = dsn.trim();
        if !dsn.is_empty() {
            return Some(
                DmConfig::from_connection_string(dsn).expect("DM_TEST_DSN 不是合法的连接串"),
            );
        }
    }
    if std::env::var("DM_SERVER").is_err() {
        return None;
    }
    Some(DmConfig::from_env().expect("DM_* 具名环境变量不合法"))
}

/// 连上达梦并包好夹具；未配置环境时返回 `None` 并打印跳过原因。
///
/// 环境**配了却连不上**时会 panic：那是真失败，不该被当成跳过。
#[must_use]
pub fn harness() -> Option<Harness> {
    let Some(cfg) = test_config() else {
        eprintln!(
            "跳过：真机集成测试需要达梦实例——请设置 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD），\
             然后用 `cargo test -p dm-odbc --test integration -- --include-ignored` 运行"
        );
        return None;
    };

    match OdbcConnection::connect(&cfg) {
        Ok(conn) => {
            let length_in_char = conn.config().length_in_char;
            Some(Harness {
                conn,
                arena: ParamArena::new(),
                scratch: FetchScratch::new(),
                length_in_char,
                checks: 0,
                failures: Vec::new(),
                invalid_at_start: invalid_cursor_count(),
            })
        }
        Err(err) => panic!("连接达梦失败：{}", describe(&err)),
    }
}

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 一个用例的上下文：连接 + 复用缓冲 + 记账。
pub struct Harness {
    /// 被测连接。
    pub conn: OdbcConnection,
    /// 参数缓冲（跨语句复用，正是 `Backend` 的用法）。
    pub arena: ParamArena,
    /// 取值暂存。
    pub scratch: FetchScratch,
    /// 建连时自检出来的列长单位（`SQL_DESC_OCTET_LENGTH` 的解释）。
    pub length_in_char: Option<bool>,
    checks: usize,
    failures: Vec<String>,
    invalid_at_start: usize,
}

impl Harness {
    /// 记一条断言。
    pub fn check(&mut self, name: &str, ok: bool) {
        self.checks += 1;
        if !ok {
            self.failures.push(format!("✗ {name}"));
        }
    }

    /// 记一条带上下文的断言（`detail` 只在失败时求值）。
    pub fn check_that(&mut self, ok: bool, detail: impl FnOnce() -> String) {
        self.checks += 1;
        if !ok {
            self.failures.push(format!("✗ {}", detail()));
        }
    }

    /// 相等断言。
    pub fn eq<T: PartialEq + Debug>(&mut self, name: &str, got: T, want: T) {
        self.checks += 1;
        if got != want {
            self.failures
                .push(format!("✗ {name}: got {got:?}, want {want:?}"));
        }
    }

    /// 直接记一条失败（用于辅助函数内部与「不该发生的错误」）。
    pub fn fail(&mut self, message: impl Into<String>) {
        self.checks += 1;
        self.failures.push(format!("✗ {}", message.into()));
    }

    /// 记一个不该出现的错误。
    pub fn record(&mut self, what: &str, err: &DmError) {
        note_error(err);
        self.fail(format!("{what} 不该失败：{}", describe(err)));
    }

    /// 已经记了多少条断言（用于确认用例真的跑了东西）。
    #[must_use]
    pub fn checks(&self) -> usize {
        self.checks
    }

    /// 用例收尾：核对 `-70027` 差值、打印小结、按需要 panic。
    pub fn finish(mut self) {
        let invalid_delta = invalid_cursor_count().saturating_sub(self.invalid_at_start);
        if invalid_delta > 0 {
            self.failures.push(format!(
                "✗ `-70027 无效的游标状态` 在本用例中出现了 {invalid_delta} 次（必须为 0）——\
                 这说明驱动有 bug，不是用户有 bug"
            ));
        }
        if self.checks == 0 {
            self.failures
                .push("✗ 本用例一条断言都没执行（夹具没跑起来？）".to_owned());
        }

        let checks = self.checks;
        let failures = std::mem::take(&mut self.failures);
        if let Err(err) = self.conn.close() {
            eprintln!("（收尾）关闭连接失败：{}", describe(&err));
        }

        assert!(
            failures.is_empty(),
            "\n集成用例失败：{}/{} 项断言未通过\n{}\n",
            failures.len(),
            checks,
            failures.join("\n")
        );
        eprintln!("✓ 集成用例通过：{checks} 项断言");
    }

    // ---------------- 执行 ----------------

    /// 一次性下发（`SQLExecDirect`）。
    pub fn exec(&mut self, sql: &str) -> Result<StmtOutcome, DmError> {
        let mut stmt = self.conn.allocate_statement()?;
        let outcome = stmt.exec_direct(sql)?;
        stmt.close_cursor()?;
        Ok(outcome)
    }

    /// 一次性下发，失败即记账（返回 `None`）。
    pub fn try_exec(&mut self, sql: &str) -> Option<StmtOutcome> {
        match self.exec(sql) {
            Ok(outcome) => Some(outcome),
            Err(err) => {
                self.record(&format!("执行 `{sql}`"), &err);
                None
            }
        }
    }

    /// 绑定参数执行（`prepare` → `bind_params` → `exec`）。
    pub fn exec_params(&mut self, sql: &str, args: &[DmValue]) -> Result<StmtOutcome, DmError> {
        let mut stmt = self.conn.allocate_statement()?;
        stmt.prepare(sql)?;
        bind_params(&mut stmt, args, &mut self.arena)?;
        let outcome = stmt.exec()?;
        stmt.close_cursor()?;
        Ok(outcome)
    }

    /// 绑定参数执行，失败即记账。
    pub fn try_exec_params(&mut self, sql: &str, args: &[DmValue]) -> Option<StmtOutcome> {
        match self.exec_params(sql, args) {
            Ok(outcome) => Some(outcome),
            Err(err) => {
                self.record(&format!("带参数执行 `{sql}`"), &err);
                None
            }
        }
    }

    /// 取回整个结果集（行优先，裸 `DmValue`）。
    pub fn rows(&mut self, sql: &str) -> Result<Vec<Vec<DmValue>>, DmError> {
        let length_in_char = self.length_in_char;
        let mut stmt = self.conn.allocate_statement()?;
        let outcome = stmt.exec_direct(sql)?;

        let rows = match outcome {
            StmtOutcome::ResultSet { .. } => {
                let columns = describe_columns(&mut stmt, length_in_char)?;
                let mut buf = RowBuf::new(columns.len());
                let mut out = Vec::new();
                loop {
                    match fetch(&mut stmt, &columns, &mut buf, &mut self.scratch)? {
                        FetchState::Rows { .. } => {
                            if let Some(row) = buf.row(0) {
                                out.push(row.to_vec());
                            }
                        }
                        FetchState::Exhausted => break,
                        // `FetchState` 是 `#[non_exhaustive]`：真冒出第三个变体说明
                        // `dm-core` 加了新状态。这里当「取完了」会让用例**悄悄少读数据**，
                        // 所以宁可炸掉也不要静默。
                        other => panic!("未知的 FetchState 变体：{other:?}"),
                    }
                }
                out
            }
            // 非结果集语句没有行可取；调用方自己判 `outcome`。
            StmtOutcome::Affected { .. } => Vec::new(),
        };

        stmt.close_cursor()?;
        Ok(rows)
    }

    /// 取回整个结果集，失败即记账。
    pub fn try_rows(&mut self, sql: &str) -> Option<Vec<Vec<DmValue>>> {
        match self.rows(sql) {
            Ok(rows) => Some(rows),
            Err(err) => {
                self.record(&format!("查询 `{sql}`"), &err);
                None
            }
        }
    }

    /// 服务端表达式的单个标量（`NULL` → `None`）。用于「与服务端侧表达比对」。
    pub fn scalar(&mut self, sql: &str) -> Result<Option<String>, DmError> {
        let rows = self.rows(sql)?;
        match rows.first().and_then(|row| row.first()) {
            None => Ok(None),
            Some(value) => Ok(value_text(value)),
        }
    }

    /// 断言服务端表达式等于期望文本。
    pub fn expect_scalar(&mut self, name: &str, sql: &str, want: &str) {
        match self.scalar(sql) {
            Ok(got) => self.eq(name, got.as_deref(), Some(want)),
            Err(err) => self.record(&format!("{name}（`{sql}`）"), &err),
        }
    }

    // ---------------- 建表 ----------------

    /// 应用 `fixtures/schema.sql`（幂等：脚本自己 `DROP TABLE IF EXISTS`）。
    ///
    /// 用 `try_exec` 是有意的：建表失败在这里吞掉，会在用例主体里以「表不存在 / 列不存在」
    /// 的形式暴露出来，定位反而更直接。**幂等门禁不能用这一版**——见 `apply_schema_strict`。
    pub fn apply_schema(&mut self) {
        for statement in schema_statements() {
            self.try_exec(&statement);
        }
    }

    /// 严格版建表：任何一条 DDL 失败都作为用例失败上报，不吞。
    ///
    /// `smoke_schema_fixture_is_idempotent` 必须走这一版。理由：幂等门禁的弱读法
    /// （「连跑两次都不报错」）**可以被静默 no-op 骗过去**——DROP 失败被吞掉、CREATE
    /// 又因为表已经存在而失败也被吞掉，于是一个根本没重建过的库看起来完全「幂等」。
    pub fn apply_schema_strict(&mut self) {
        for statement in schema_statements() {
            if let Err(err) = self.exec(&statement) {
                self.record(&format!("执行建表语句：{statement}"), &err);
            }
        }
    }
}

/// 把 `fixtures/schema.sql` 切成逐条语句（丢掉 `--` 整行注释）。
#[must_use]
pub fn schema_statements() -> Vec<String> {
    include_str!("../fixtures/schema.sql")
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

// ---------------------------------------------------------------------------
// 值的小工具
// ---------------------------------------------------------------------------

/// 把一个值渲染成文本（`NULL` → `None`）。只用于断言消息与服务端侧比对。
#[must_use]
pub fn value_text(value: &DmValue) -> Option<String> {
    match value {
        DmValue::Null => None,
        DmValue::Bool(b) => Some(if *b { "1" } else { "0" }.to_owned()),
        DmValue::TinyInt(v) => Some(v.to_string()),
        DmValue::SmallInt(v) => Some(v.to_string()),
        DmValue::Int(v) => Some(v.to_string()),
        DmValue::BigInt(v) => Some(v.to_string()),
        DmValue::Real(v) => Some(v.to_string()),
        DmValue::Double(v) => Some(v.to_string()),
        DmValue::Text(s) => Some(s.clone()),
        // 其余（Decimal / 日期时间 / 二进制 / 未知码）用 `Debug`——这里只服务于诊断文本，
        // 不参与「值是否正确」的判定。
        other => Some(format!("{other:?}")),
    }
}

/// 取一行里某一列的文本视图。
#[must_use]
pub fn cell_text(row: &[DmValue], index: usize) -> Option<String> {
    row.get(index).and_then(value_text)
}

/// 小写十六进制（用于二进制列的规范化比较）。
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // 写进 `String` 不会失败。
        let _ = write!(out, "{byte:02X}");
    }
    out
}

/// 把值渲染成**规范化文本**，供「全类型往返」做表驱动比较。
///
/// 规范化规则（两边都按同一套写，比较才有意义）：
/// * `NULL` → `"<NULL>"`；布尔 → `"1"` / `"0"`（达梦的 `BIT` 取回来就是 0/1）；
/// * 文本 → 去掉尾部空格（`CHAR(n)` 由服务端补空格，那是存储形态不是值）；
/// * 二进制 → 大写十六进制；
/// * 定点数 → `normalize()` 后的最简写法（`-12345678.90` 与 `-12345678.9` 是同一个值，
///   达梦对不同 scale 的列回不同的字符串，直接比字符串会把等价表示判成失败）；
/// * 日期时间 → `chrono` 的 `Display`（`1999-12-31` / `23:59:59` / `2024-03-04 05:06:07.123456`）。
#[must_use]
pub fn canonical(value: &DmValue) -> String {
    match value {
        DmValue::Null => "<NULL>".to_owned(),
        DmValue::Bool(b) => u8::from(*b).to_string(),
        DmValue::TinyInt(v) => v.to_string(),
        DmValue::SmallInt(v) => v.to_string(),
        DmValue::Int(v) => v.to_string(),
        DmValue::BigInt(v) => v.to_string(),
        DmValue::Real(v) => f64::from(*v).to_string(),
        DmValue::Double(v) => v.to_string(),
        DmValue::Decimal(d) => d.normalize().to_string(),
        DmValue::Text(s) => s.trim_end().to_owned(),
        DmValue::Bytes(b) => hex(b),
        DmValue::Date(d) => d.to_string(),
        DmValue::Time(t) => t.to_string(),
        DmValue::Timestamp(t) => t.to_string(),
        DmValue::Interval(i) => i.to_string(),
        DmValue::Unknown(b) => format!("<unknown:{}>", hex(b)),
        other => format!("<{other:?}>"),
    }
}

/// 按列号取规范化文本（越界/缺列时给出可读的占位）。
#[must_use]
pub fn cell_canonical(row: &[DmValue], index: usize) -> String {
    match row.get(index) {
        Some(value) => canonical(value),
        None => format!("<缺列 {index}>"),
    }
}

/// 取一列的元数据类型名（`DataType` 的 `Display`）。
#[must_use]
pub fn column_type_names(columns: &[ColumnMeta]) -> Vec<String> {
    columns.iter().map(|c| c.data_type.to_string()).collect()
}

// ---------------------------------------------------------------------------
// 用例宏
// ---------------------------------------------------------------------------

/// 声明一个真机集成用例：自动带 `#[ignore]`、自动取夹具、自动收尾。
///
/// 夹具变量名由调用方写成参数（这里是 `h`）——`macro_rules!` 的**局部变量是有卫生性的**：
/// 宏自己 `let` 出来的名字，在被传进来的 `$body` 里是看不见的，所以必须先由调用方把名字交进来。
///
/// ```ignore
/// dm_test!(fn smoke_select_one(h) {
///     let rows = h.try_rows("select 1 from dual").unwrap();
///     h.eq("一行", rows.len(), 1);
/// });
/// ```
macro_rules! dm_test {
    ($(#[$meta:meta])* fn $name:ident($h:ident) $body:block) => {
        #[test]
        #[ignore = "真机集成测试：设置 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）后加 --include-ignored"]
        $(#[$meta])*
        fn $name() {
            // 整个用例期间独占数据库：`apply_schema` 会 `DROP`/`CREATE` 共享的表，
            // 并行跑就会互相踩（真机上实测到 `-7009 对象定义被修改`）。
            let _serial = crate::common::db_lock();
            let Some(mut $h) = crate::common::harness() else {
                return;
            };
            // 用例体放进闭包：体内的 `return` 只结束用例，**收尾永远会跑**——
            // 否则早退的用例会把已经记下的失败一起吞掉。
            // `&mut` 形参必须是 `mut` 才能重借出去，但不需要改的用例会嫌它多余。
            #[allow(unused_mut)]
            let body = |mut $h: &mut crate::common::Harness| $body;
            body(&mut $h);
            $h.finish();
        }
    };
}

pub(crate) use dm_test;

/// 供测试断言「语句上没有留活游标」用的小工具。
pub fn cursor_closed(stmt: &OdbcStatement<'_>) -> bool {
    !stmt.is_cursor_open()
}
