//! 错误码采集探针（§10.3 那张采集表的现场入口，M6/T6.5 配套件）。
//!
//! §10.3 要求 `errors.rs` 故意制造 **11 类**错误，并把 `(SQLSTATE, 原生码, 消息模板, 分类,
//! 可重试)` 落表 → `docs/errors-catalog.json`。集成用例负责**钉住**已经测出来的码；
//! 这个探针负责**发现**还没测出来的码——它只打印、不断言，所以可以在达梦版本升级后
//! 直接重跑，对比 §6.8 的表有没有漂移（「新错误码出现即告警」）。
//!
//! 跑法：
//! ```text
//! $env:DM_SERVER='localhost'; $env:DM_USER='SYSDBA'; $env:DM_PASSWORD='SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc run -p dm-odbc --example errors_probe
//! ```
//!
//! # 为什么每条都要「执行 + 取一行」
//!
//! 达梦有的错误在 `SQLExecute`（`执行`）就报，有的要等 `SQLFetch`（`取数`）才报——
//! 所以表里那列写的是**触发阶段**。只 `exec_direct` 不看结果集，会把「取数时才炸」
//! 误记成「服务端接受了这条语句」。
//!
//! # 本探针不采集的（原因写在各自的打印里，不是遗漏）
//!
//! * 权限不足——需要一个受限用户，属于 DBA 侧的准备工作，不适合塞进只读探针；
//! * 死锁——要靠两个会话互相等待，达梦的死锁检测有自己的间隔，探针有卡住的风险；
//!   真要做，得用带超时的两个连接在**独立用例**里做，见 §10.3 的备注。

use dm_core::{DmConfig, RowBuf};
use dm_error::DmError;
use dm_odbc::{FetchScratch, OdbcConnection, OdbcStatement, StmtOutcome, describe_columns, fetch};

fn connect() -> OdbcConnection {
    let cfg = DmConfig::from_env().expect("需要 DM_SERVER / DM_USER / DM_PASSWORD");
    OdbcConnection::connect(&cfg).expect("连接失败")
}

/// 打印采集表的一行，列与 §6.8 的表一致。
fn report(label: &str, stage: &str, err: &DmError) {
    let sqlstate = err
        .diags()
        .first()
        .map(|rec| rec.sqlstate_str())
        .unwrap_or_default();
    let (code, message) = err.server_code_and_message().unwrap_or((0, ""));
    // 消息里可能带换行（达梦的「第1 行附近出现错误:\n…」），压成一行好入表。
    let message = message.split_whitespace().collect::<Vec<_>>().join(" ");
    println!(
        "| {label} | {stage} | {sqlstate} | {code} | {message} | {} | {} |",
        err.kind,
        if err.is_retryable() { "是" } else { "否" }
    );
    for rec in err.diags().iter().skip(1) {
        println!(
            "|   ↳ 附加诊断 | {stage} | {} | {} | {} | — | — |",
            rec.sqlstate_str(),
            rec.native_code,
            rec.message.split_whitespace().collect::<Vec<_>>().join(" ")
        );
    }
}

/// 取一行；返回「是否还有行」。
fn fetch_one(stmt: &mut OdbcStatement<'_>) -> Result<bool, DmError> {
    let columns = describe_columns(stmt, None)?;
    let mut buf = RowBuf::with_capacity(1, 256);
    let mut scratch = FetchScratch::new();
    Ok(fetch(stmt, &columns, &mut buf, &mut scratch)?.rows() > 0)
}

/// 跑一条**注定失败**的语句：执行失败就报执行阶段，执行成功则再取一行。
fn probe(label: &str, sql: &str) {
    let mut conn = connect();
    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            report(label, "申请语句", &err);
            return;
        }
    };
    match stmt.exec_direct(sql) {
        Err(err) => report(label, "执行", &err),
        Ok(StmtOutcome::Affected { .. }) => {
            println!("| {label} | 执行 | — | — | **没有报错**（也不是结果集） | — | — |");
        }
        Ok(StmtOutcome::ResultSet { .. }) => match fetch_one(&mut stmt) {
            Ok(true) => {
                println!("| {label} | 取数 | — | — | **没有报错**（服务端真的算出来了） | — | — |");
            }
            Ok(false) => {
                println!("| {label} | 取数 | — | — | **没有报错**（结果集是空的） | — | — |");
            }
            Err(err) => report(label, "取数", &err),
        },
    }
}

/// 无效游标：把结果集取到耗尽，然后再取一次（期望 `-70027`）。
///
/// 这条同时验证 §6.4 的纪律——`dm-odbc` 应当把 `SQL_NO_DATA` 之后的再次取数
/// 认成「游标关了」，而不是把驱动的坏返回码原样漏出去。
fn probe_invalid_cursor() {
    let label = "无效游标（取尽后再取）";
    let mut conn = connect();
    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => return report(label, "申请语句", &err),
    };
    if let Err(err) = stmt.exec_direct("select ID from DM_T_STR") {
        return report(label, "执行", &err);
    }
    let columns = match describe_columns(&mut stmt, None) {
        Ok(columns) => columns,
        Err(err) => return report(label, "描述列", &err),
    };
    let mut buf = RowBuf::with_capacity(8, 128);
    let mut scratch = FetchScratch::new();
    loop {
        match fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
            Ok(state) if state.is_exhausted() => break,
            Ok(_) => continue,
            Err(err) => return report(label, "取数", &err),
        }
    }
    match fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
        Ok(_) => println!("| {label} | 取数 | — | — | **没有报错**：耗尽后还能再取 | — | — |"),
        Err(err) => report(label, "取数", &err),
    }
}

/// 行锁冲突：持锁连接开着事务不放，另一个连接用 `FOR UPDATE NOWAIT` 立刻要锁。
///
/// 用 `NOWAIT` 而不是等超时：`NOWAIT` 是**确定性**的，等超时会让探针挂住几分钟，
/// 而「锁冲突报什么码」这两条路径在达梦上大概率是同一个码。
fn probe_lock_conflict() {
    let label = "行锁冲突（NOWAIT 侧）";
    let mut holder = connect();
    let mut waiter = connect();

    for sql in [
        "delete from DM_T_TXN where ID = 1",
        "insert into DM_T_TXN (ID, V) values (1, 0)",
    ] {
        let outcome = holder
            .allocate_statement()
            .and_then(|mut stmt| stmt.exec_direct(sql));
        if let Err(err) = outcome {
            return report(label, "准备数据", &err);
        }
    }

    if let Err(err) = holder.begin() {
        return report(label, "begin", &err);
    }
    // 持锁：语句对象必须活到 waiter 跑完——`Drop` 会关游标，锁也就跟着放了。
    let held = holder
        .allocate_statement()
        .and_then(|mut stmt| stmt.exec_direct("select * from DM_T_TXN where ID = 1 for update"));
    match held {
        Err(err) => {
            report("行锁冲突：持锁语句本身失败", "执行", &err);
            let _ = holder.rollback();
            return;
        }
        Ok(_) => println!(
            "| 行锁冲突 | — | — | — | 持锁连接已 `select … for update`（事务未提交） | — | — |"
        ),
    }

    let outcome = waiter.allocate_statement().and_then(|mut stmt| {
        stmt.exec_direct("select * from DM_T_TXN where ID = 1 for update nowait")
    });
    match outcome {
        Ok(_) => println!("| {label} | 执行 | — | — | **没有报错**：`NOWAIT` 没起作用 | — | — |"),
        Err(err) => report(label, "执行", &err),
    }

    if let Err(err) = holder.rollback() {
        report(label, "收尾 rollback", &err);
    }
}

fn main() {
    println!("错误码采集（§10.3）：`| 触发 | 阶段 | SQLSTATE | 原生码 | 消息 | 分类 | 可重试 |`\n");

    // 已登记的三条（对照 §6.8 的表有没有漂移）。
    probe("表不存在", "select 1 from DM_T_NOT_THERE");
    probe("列不存在", "select NO_SUCH_COLUMN from DM_T_STR");
    probe("语法错（反引号标识符）", "select `ID` from DM_T_STR");

    // 类型不匹配 / 除零：`dual` 上的表达式不需要任何表，探针可以单独跑。
    probe("类型不匹配（字符串参与算术）", "select 1 + 'abc' from dual");
    probe(
        "类型不匹配（字符串塞进 INT 列）",
        "insert into DM_T_STR (ID, N) values (9001, 'abc')",
    );
    probe(
        "类型不匹配（非日期塞进 DATE 列）",
        "insert into DM_T_TYPES (ID, C_DATE) values (9002, 'not-a-date')",
    );
    probe("除零（常量折叠）", "select 1/0 from dual");
    probe(
        "除零（子查询包一层）",
        "select (select 1 from dual) / (select 1 - 1 from dual) from dual",
    );
    probe("参数个数不符（exec_direct 带 ?）", "select ? from dual");

    probe_invalid_cursor();
    probe_lock_conflict();

    println!(
        "\n| 权限不足 | 未采集 | — | — | 需要一个受限用户（DBA 侧准备），见本文件模块文档 | — | — |"
    );
    println!(
        "| 死锁 | 未采集 | — | — | 需要两个会话互相等待 + 死锁检测间隔，风险是挂住探针 | — | — |"
    );
}
