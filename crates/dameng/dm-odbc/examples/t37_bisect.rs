//! T3.7 `-70027` 的机制定位：六个场景逐个排除。
//!
//! 每个场景都**新建一条连接**，避免上一条连接的残留状态污染判断。
//! 跑法：设 `DM_DRIVER`/`DM_SERVER`/`DM_PORT`/`DM_USER`/`DM_PASSWORD` 后
//! `cargo run -p dm-odbc --example t37_bisect`
//!
//! **结论**（已写进设计文档 §7「真机实测补注（M3/T3.7）」第 14 条）：
//! `SQLFreeStmt(SQL_HANDLE_STMT, SQL_CLOSE)` 在「句柄上没有开着的游标」时会失败，
//! 并且**污染该连接的后续语句**（下一条 DDL/查询报 `HY000 / -70027`，错误信息里
//! 完全没有指向 close 的字样）。基线场景 0「新句柄 + 不调 `close_cursor`」一切正常，
//! 场景 2（= `Harness::exec` 的形状）/ 3 / 5 会炸——这就是根因。
//! 修法：`OdbcStatement::close_cursor` 先问状态再关，不无条件发 `SQL_CLOSE`。
//!
//! **保留**（不删）：这是「驱动有这个怪癖」的唯一现场证据。达梦换版本后重跑一遍，
//! 如果场景 3 不再报错，就说明驱动修了，`close_cursor` 里的状态判断可以考虑简化。
//! 永久回归防线另在 `tests/integration/common/mod.rs` 的 `-70027` 全局计数断言。

use dm_core::{DmConfig, DmValue, RowBuf};
use dm_error::DmError;
use dm_odbc::{FetchScratch, OdbcConnection, describe_columns, fetch};

fn tag(result: &Result<impl std::fmt::Debug, DmError>) -> String {
    match result {
        Ok(what) => format!("OK {what:?}"),
        Err(err) => {
            let codes: Vec<String> = err
                .diags()
                .iter()
                .map(|d| format!("{}/{}", d.sqlstate_str(), d.native_code))
                .collect();
            format!("ERR {} [{codes:?}]", err.kind)
        }
    }
}

fn connect() -> OdbcConnection {
    let cfg = DmConfig::from_env().expect("需要 DM_* 环境变量");
    OdbcConnection::connect(&cfg).expect("连接失败")
}

/// `SELECT` 的结果集必须取完（或显式关掉）才能再下一条语句。
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
    let _ = DmValue::Null;
}

fn scenario(title: &str, body: impl FnOnce(&mut OdbcConnection)) {
    println!("\n== {title} ==");
    let mut conn = connect();
    body(&mut conn);
    if let Err(err) = conn.close() {
        println!("  (关闭连接失败：{})", tag(&Err::<(), _>(err)));
    }
}

fn main() {
    // 0. 基线：连续两条 DDL，各用新句柄，不碰游标。
    scenario("0 基线：新句柄 + DDL，不调 close_cursor", |conn| {
        let mut s1 = conn.allocate_statement().expect("alloc");
        println!(
            "  create A  -> {}",
            tag(&s1.exec_direct("create table P_A (ID INT)"))
        );
        drop(s1);
        let mut s2 = conn.allocate_statement().expect("alloc");
        println!(
            "  create B  -> {}",
            tag(&s2.exec_direct("create table P_B (ID INT)"))
        );
        drop(s2);
        let mut s3 = conn.allocate_statement().expect("alloc");
        println!("  drop A    -> {}", tag(&s3.exec_direct("drop table P_A")));
        let _ = s3.exec_direct("drop table P_B");
    });

    // 1. SELECT（不取行、不关游标）之后直接丢句柄，再执行 DDL。
    scenario("1 SELECT → 丢句柄（不关游标）→ DDL", |conn| {
        let mut s1 = conn.allocate_statement().expect("alloc");
        println!(
            "  select    -> {}",
            tag(&s1.exec_direct("select 1 from dual"))
        );
        drop(s1);
        let mut s2 = conn.allocate_statement().expect("alloc");
        println!(
            "  create C  -> {}",
            tag(&s2.exec_direct("create table P_C (ID INT)"))
        );
        let _ = s2.exec_direct("drop table P_C");
    });

    // 2. SELECT → close_cursor → 丢句柄 → DDL（= `Harness::exec` 的形状）。
    scenario("2 SELECT → close_cursor → 丢句柄 → DDL", |conn| {
        let mut s1 = conn.allocate_statement().expect("alloc");
        println!(
            "  select    -> {}",
            tag(&s1.exec_direct("select 1 from dual"))
        );
        println!("  close     -> {}", tag(&s1.close_cursor()));
        drop(s1);
        let mut s2 = conn.allocate_statement().expect("alloc");
        println!(
            "  create D  -> {}",
            tag(&s2.exec_direct("create table P_D (ID INT)"))
        );
        let _ = s2.exec_direct("drop table P_D");
    });

    // 3. 没有游标时 close_cursor（同一句柄上紧接着执行）。
    scenario("3 无游标 close_cursor → 同一句柄执行", |conn| {
        let mut s1 = conn.allocate_statement().expect("alloc");
        println!("  close     -> {}", tag(&s1.close_cursor()));
        println!(
            "  create E  -> {}",
            tag(&s1.exec_direct("create table P_E (ID INT)"))
        );
        println!("  close     -> {}", tag(&s1.close_cursor()));
        println!(
            "  create E2 -> {}",
            tag(&s1.exec_direct("create table P_E2 (ID INT)"))
        );
        let _ = s1.exec_direct("drop table P_E");
        let _ = s1.exec_direct("drop table P_E2");
    });

    // 4. SELECT → 取完 → 丢句柄 → DDL。
    scenario("4 SELECT → 取完 → 丢句柄 → DDL", |conn| {
        let length_in_char = conn.config().length_in_char;
        let mut s1 = conn.allocate_statement().expect("alloc");
        println!(
            "  select    -> {}",
            tag(&s1.exec_direct("select 1 from dual"))
        );
        drain(length_in_char, &mut s1);
        println!("  取完后 cursor_open = {}", s1.is_cursor_open());
        drop(s1);
        let mut s2 = conn.allocate_statement().expect("alloc");
        println!(
            "  create F  -> {}",
            tag(&s2.exec_direct("create table P_F (ID INT)"))
        );
        let _ = s2.exec_direct("drop table P_F");
    });

    // 5. DDL → close_cursor（无游标）→ 丢句柄 → DDL。
    scenario("5 DDL → close_cursor → 丢句柄 → DDL", |conn| {
        let mut s1 = conn.allocate_statement().expect("alloc");
        println!(
            "  create G  -> {}",
            tag(&s1.exec_direct("create table P_G (ID INT)"))
        );
        println!("  close     -> {}", tag(&s1.close_cursor()));
        drop(s1);
        let mut s2 = conn.allocate_statement().expect("alloc");
        println!(
            "  create H  -> {}",
            tag(&s2.exec_direct("create table P_H (ID INT)"))
        );
        let _ = s2.exec_direct("drop table P_G");
        let _ = s2.exec_direct("drop table P_H");
    });

    // 6. SELECT 后用裸 `SQLCloseCursor`（ODBC 3.x 的专用函数）替代 `SQLFreeStmt(SQL_CLOSE)`。
    scenario(
        "6 SELECT → 裸 SQLCloseCursor → 丢句柄 → DDL",
        |conn| {
            use odbc_api::handles::Statement as _;
            let mut s1 = conn.allocate_statement().expect("alloc");
            println!(
                "  select    -> {}",
                tag(&s1.exec_direct("select 1 from dual"))
            );
            let raw = unsafe { odbc_sys::SQLCloseCursor(s1.statement().as_sys()) };
            println!("  SQLCloseCursor 返回 {raw:?}");
            drop(s1);
            let mut s2 = conn.allocate_statement().expect("alloc");
            println!(
                "  create I  -> {}",
                tag(&s2.exec_direct("create table P_I (ID INT)"))
            );
            let _ = s2.exec_direct("drop table P_I");
        },
    );

    println!("\n完成。");
}
