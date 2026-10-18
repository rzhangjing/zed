//! 语句复用（CAP-C-11 语句缓存）在达梦上的游标语义定位。
//!
//! 背景：语句缓存命中时走 `OdbcStatement::exec()`（`SQLExecute`），而旧路径每次都
//! `SQLExecDirectW`。真机回归里 `query_real` 出现 `52000 / -70027 无效的游标状态`
//! （stage = Execute），怀疑「结果集取尽后 `SQLExecute` 仍要求先显式关游标」。
//!
//! 每个场景一条新连接，逐个排除：
//!   A 预处理 + 执行 + **取尽** + 再执行（空 SQL ⇒ `SQLExecute`）——复现目标
//!   B 同 A，但两次之间显式 `close_cursor()`
//!   C 同 A，但第二次改用 `exec_direct`（旧路径的形状）
//!   D 预处理 + 执行 + **不取**（留活游标）+ 再执行
//!   E DML（无结果集）复用同一条预处理语句
//!   F 取尽后先 `close_cursor()` 再执行（关得掉吗？关完还能执行吗？）
//!   G DDL 复用（第二次报的是服务端「对象已存在」，还是又变成 `-70027`）
//!
//! # 结论（本机 DM8 实测）
//!
//! **结果集被取尽之后，这条语句句柄就再也执行不了了**：
//!
//! | 场景 | 结果 |
//! |---|---|
//! | A `exec` + 取尽 + `exec` | `24000/-70027 无效的游标状态` |
//! | B 同上 + 显式 `close_cursor()`（它返回 `Ok`） | 仍 `-70027` |
//! | C 同上 + `exec_direct`（旧路径形状） | 仍 `-70027` |
//! | D `exec` + **不取** + `exec` | 通过（两次都拿到结果集） |
//! | E DML `exec` + `exec` | 通过 |
//! | G DDL `exec` + `exec` | 报服务端 `HY000/-7041 对象已存在`（句柄是好的） |
//!
//! 也就是说「取尽」把句柄弄死了，`SQLFreeStmt(SQL_CLOSE)` 与 `SQLExecDirect` 都救不回来
//! （与 `-70027` 的既有教训同族，见 `examples/t37_bisect.rs`）。设计后果：
//! [`dm_odbc::OdbcBackend`] 的 `Backend::statement_reusable` 对**结果集语句**回 `false`，
//! 语句缓存（CAP-C-11）只复用 DML/DDL 形态的语句；查询语句每次重新 `prepare`，
//! 但执行仍走已预处理的计划（`SQLExecute`），不再退回 `SQLExecDirect` 的双份解析。
//!
//! 跑法：`cargo run -p dm-odbc --example cursor_reuse_probe`（在本机达梦上，凭据写死
//! 在 `connect()` 里，故意如此：这个探针只服务于本机真机定位）。

use dm_core::{DmConfig, RowBuf};
use dm_error::DmError;
use dm_odbc::{FetchScratch, OdbcConnection, describe_columns, fetch};

fn tag<T: std::fmt::Debug>(result: &Result<T, DmError>) -> String {
    match result {
        Ok(what) => format!("OK {what:?}"),
        Err(err) => {
            let codes: Vec<String> = err
                .diags()
                .iter()
                .map(|d| format!("{}/{}", d.sqlstate_str(), d.native_code))
                .collect();
            format!("ERR {} [{codes:?}] {}", err.kind, err)
        }
    }
}

fn connect() -> OdbcConnection {
    let cfg = DmConfig::builder()
        .driver("DM8 ODBC DRIVER")
        .server("localhost")
        .port(5236)
        .user("SYSDBA")
        .password("SYSDBA")
        .build()
        .expect("配置构建失败");
    OdbcConnection::connect(&cfg).expect("连接失败")
}

/// 把结果集取尽（`SQLFetch` 走到 `SQL_NO_DATA`，取值层会 `mark_cursor_closed`）。
fn drain(length_in_char: Option<bool>, stmt: &mut dm_odbc::OdbcStatement<'_>) {
    let columns = match describe_columns(stmt, length_in_char) {
        Ok(columns) => columns,
        Err(err) => {
            println!("      (drain: 描述列失败 {})", tag(&Err::<(), _>(err)));
            return;
        }
    };
    let mut buf = RowBuf::new(columns.len());
    let mut scratch = FetchScratch::new();
    loop {
        match fetch(stmt, &columns, &mut buf, &mut scratch) {
            Ok(dm_core::backend::FetchState::Rows { .. }) => {}
            Ok(_) => break,
            Err(err) => {
                println!("      (drain: fetch 失败 {})", tag(&Err::<(), _>(err)));
                break;
            }
        }
    }
}

fn scenario(title: &str, body: impl FnOnce(&mut OdbcConnection)) {
    println!("\n== {title} ==");
    let mut conn = connect();
    body(&mut conn);
    if let Err(err) = conn.close() {
        println!("  (关闭连接失败：{})", tag(&Err::<(), _>(err)));
    }
}

const SELECT: &str = "select count(*) from dual";
const DML: &str = "insert into CURSOR_PROBE_T (ID) values (1)";

fn main() {
    // A：复现目标——同一预处理语句、结果集取尽后再执行。
    scenario("A prepare + exec + 取尽 + exec(空 SQL)", |conn| {
        let length_in_char = conn.config().length_in_char;
        let mut s = conn.allocate_statement().expect("alloc");
        println!("  prepare   -> {}", tag(&s.prepare(SELECT)));
        println!("  exec #1   -> {}", tag(&s.exec()));
        drain(length_in_char, &mut s);
        println!("  取尽后 cursor_open = {}", s.is_cursor_open());
        println!("  exec #2   -> {}", tag(&s.exec()));
    });

    // B：同 A，中间显式 close_cursor()。
    scenario("B prepare + exec + 取尽 + close_cursor + exec", |conn| {
        let length_in_char = conn.config().length_in_char;
        let mut s = conn.allocate_statement().expect("alloc");
        println!("  prepare   -> {}", tag(&s.prepare(SELECT)));
        println!("  exec #1   -> {}", tag(&s.exec()));
        drain(length_in_char, &mut s);
        println!("  close     -> {}", tag(&s.close_cursor()));
        println!("  exec #2   -> {}", tag(&s.exec()));
    });

    // C：旧路径的形状——第二次用 exec_direct。
    scenario("C prepare + exec + 取尽 + exec_direct(SQL)", |conn| {
        let length_in_char = conn.config().length_in_char;
        let mut s = conn.allocate_statement().expect("alloc");
        println!("  prepare   -> {}", tag(&s.prepare(SELECT)));
        println!("  exec #1   -> {}", tag(&s.exec()));
        drain(length_in_char, &mut s);
        println!("  direct #2 -> {}", tag(&s.exec_direct(SELECT)));
    });

    // D：不取行（游标真的开着）再执行。
    scenario("D prepare + exec + 不取 + exec", |conn| {
        let mut s = conn.allocate_statement().expect("alloc");
        println!("  prepare   -> {}", tag(&s.prepare(SELECT)));
        println!("  exec #1   -> {}", tag(&s.exec()));
        println!("  cursor_open = {}", s.is_cursor_open());
        println!("  exec #2   -> {}", tag(&s.exec()));
    });

    // E：DML 复用（无结果集）。
    scenario("E DML prepare + exec + exec", |conn| {
        {
            let mut s0 = conn.allocate_statement().expect("alloc");
            let _ = s0.exec_direct("drop table CURSOR_PROBE_T");
            println!(
                "  create    -> {}",
                tag(&s0.exec_direct("create table CURSOR_PROBE_T (ID INT)"))
            );
        }
        let mut s = conn.allocate_statement().expect("alloc");
        println!("  prepare   -> {}", tag(&s.prepare(DML)));
        println!("  exec #1   -> {}", tag(&s.exec()));
        println!("  exec #2   -> {}", tag(&s.exec()));
        drop(s);
        let mut s2 = conn.allocate_statement().expect("alloc");
        let _ = s2.exec_direct("drop table CURSOR_PROBE_T");
    });

    // F：取尽后 close_cursor 是否可用（关不关得掉）以及之后能否再执行。
    scenario(
        "F prepare + exec + 取尽 + close_cursor + exec_direct",
        |conn| {
            let length_in_char = conn.config().length_in_char;
            let mut s = conn.allocate_statement().expect("alloc");
            println!("  prepare   -> {}", tag(&s.prepare(SELECT)));
            println!("  exec #1   -> {}", tag(&s.exec()));
            drain(length_in_char, &mut s);
            println!("  close     -> {}", tag(&s.close_cursor()));
            println!("  direct #2 -> {}", tag(&s.exec_direct(SELECT)));
        },
    );

    // G：DDL 复用（语句缓存的准入面之一）。第二次必然报「表已存在」——要看的是它
    // 报的是**服务端的那条**错误，还是又变成 -70027。
    scenario("G DDL prepare + exec + exec", |conn| {
        let ddl = "create table CURSOR_PROBE_T2 (ID INT)";
        {
            let mut s0 = conn.allocate_statement().expect("alloc");
            let _ = s0.exec_direct("drop table CURSOR_PROBE_T2");
        }
        let mut s = conn.allocate_statement().expect("alloc");
        println!("  prepare   -> {}", tag(&s.prepare(ddl)));
        println!("  exec #1   -> {}", tag(&s.exec()));
        println!("  exec #2   -> {}", tag(&s.exec()));
        drop(s);
        let mut s2 = conn.allocate_statement().expect("alloc");
        let _ = s2.exec_direct("drop table CURSOR_PROBE_T2");
    });

    println!("\n完成。");
}
