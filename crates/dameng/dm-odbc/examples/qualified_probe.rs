//! 「限定名到底怎么写」的定位探针：`S.T`（不带引号）vs `"S"."T"`（带引号）。
//!
//! 起因：用户反馈（Round 36）——达梦支持的是 `S.T`，不是 `"S"."T"`。中间件当前把
//! 带模式前缀的表名渲染成 `"DBTESTA"."T_ORDER"`，如果这个写法在真库上不被接受（或
//! 语义与 `DBTESTA.T_ORDER` 不同），那就得改成不带引号的形态。
//!
//! 这个探针只打印、不断言（定位工具，不是门禁），要回答五件事：
//!
//! 1. 建表用 `"S"."T"`，读用 `S.T` / `"S"."T"` / `"S.T"` 各是什么结果。
//! 2. 建表用 `S.T`，读用 `"S"."T"` / `S.T` 各是什么结果。
//! 3. 两种写法能不能互相引用（同一条语句里混用）。
//! 4. `ALL_TABLES` 里对象究竟落在哪个 owner（确认不是「落到了别的模式」）。
//! 5. 小写模式、混合大小写表名在「不带引号」时被折成大写的行为（决定折大小写策略）。
//!
//! 跑法（在仓库根）：
//!
//! ```text
//! $env:DM_TEST_DSN='DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc run -p dm-odbc --example qualified_probe
//! ```
use odbc_api::{Connection, ConnectionOptions, Cursor, Environment};

type R<T> = Result<T, String>;

fn base_dsn() -> R<String> {
    match std::env::var("DM_TEST_DSN") {
        Ok(dsn) if !dsn.trim().is_empty() => Ok(dsn),
        _ => Err(
            "需要 DM_TEST_DSN（例如 DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA）"
                .into(),
        ),
    }
}

fn open<'e>(env: &'e Environment, connection_string: &str) -> R<Connection<'e>> {
    env.connect_with_connection_string(connection_string, ConnectionOptions::default())
        .map_err(|err| format!("SQLDriverConnect: {err}"))
}

/// 单列单行的宽字符取值（中文/标识符一律走宽字符路径）。
fn q_text(conn: &Connection<'_>, sql: &str) -> R<Option<String>> {
    let Some(mut cur) = conn
        .execute(sql, (), None)
        .map_err(|e| format!("{sql} → {e}"))?
    else {
        return Ok(None);
    };
    let Some(mut row) = cur.next_row().map_err(|e| format!("next_row: {e}"))? else {
        return Ok(None);
    };
    let mut buf: Vec<u16> = Vec::new();
    match row.get_wide_text(1, &mut buf) {
        Ok(true) => Ok(Some(String::from_utf16_lossy(&buf))),
        Ok(false) => Ok(None),
        Err(e) => Err(format!("get_wide_text: {e}")),
    }
}

/// 跑一条语句：成功打成 `OK  <label>  <sql>`，失败打成 `FAIL <label>  <sql> ⇒ 错误`。
/// 返回是否成功。
fn try_sql(conn: &Connection<'_>, label: &str, sql: &str) -> bool {
    match conn.execute(sql, (), None) {
        Ok(_) => {
            println!("  OK   {label:<38} {sql}");
            true
        }
        Err(e) => {
            println!("  FAIL {label:<38} {sql}\n         ⇒ {e}");
            false
        }
    }
}

/// 查一列并打印（`?` 表示 NULL / 查询失败）。
fn show(conn: &Connection<'_>, sql: &str) {
    match q_text(conn, sql) {
        Ok(Some(v)) => println!("  {sql}\n       ⇒ {v}"),
        Ok(None) => println!("  {sql}\n       ⇒ <无行/NULL>"),
        Err(e) => println!("  {sql}\n       ⇒ 查询失败：{e}"),
    }
}

fn main() -> R<()> {
    let dsn = base_dsn()?;
    let env = Environment::new().map_err(|e| format!("Environment: {e}"))?;
    let conn = open(&env, &dsn)?;
    println!("DSN = {dsn}");

    // 模式先备好（对已存在的模式，CREATE 会失败，忽略即可）。
    println!("\n=== 0. 清场 ===");
    let _ = conn.execute(r#"create schema "DBTESTA" authorization SYSDBA"#, (), None);
    let _ = conn.execute(
        r#"create schema "dbtest_lower" authorization SYSDBA"#,
        (),
        None,
    );
    for (label, name) in [
        ("P_QUOTED", r#""DBTESTA"."P_QUOTED""#),
        ("P_PLAIN", r#""DBTESTA"."P_PLAIN""#),
        ("P_MixedCase", r#""DBTESTA"."P_MixedCase""#),
        ("P_LOWER", r#""dbtest_lower"."P_LOWER""#),
    ] {
        try_sql(
            &conn,
            &format!("drop if exists {label}"),
            &format!("drop table if exists {name} cascade constraints"),
        );
    }

    println!("\n=== 1. 建表用带引号，读用三种写法 ===");
    try_sql(
        &conn,
        "create \"S\".\"T\"",
        r#"create table "DBTESTA"."P_QUOTED" (id int, note varchar(32))"#,
    );
    try_sql(
        &conn,
        "select  S.T      （不带引号）",
        "select count(*) from DBTESTA.P_QUOTED",
    );
    try_sql(
        &conn,
        "select  \"S\".\"T\"  （带引号）",
        r#"select count(*) from "DBTESTA"."P_QUOTED""#,
    );
    try_sql(
        &conn,
        "select  \"S.T\"      （整串）",
        r#"select count(*) from "DBTESTA.P_QUOTED""#,
    );

    println!("\n=== 2. 建表用不带引号，读用两种写法 ===");
    try_sql(
        &conn,
        "create S.T",
        "create table DBTESTA.P_PLAIN (id int, note varchar(32))",
    );
    try_sql(
        &conn,
        "select  \"S\".\"T\"  （带引号）",
        r#"select count(*) from "DBTESTA"."P_PLAIN""#,
    );
    try_sql(
        &conn,
        "select  S.T      （不带引号）",
        "select count(*) from DBTESTA.P_PLAIN",
    );

    println!("\n=== 3. 两种写法混用（写入 + 同语句引用） ===");
    try_sql(
        &conn,
        "insert 不带引号",
        "insert into DBTESTA.P_PLAIN (id, note) values (1, 'a')",
    );
    try_sql(
        &conn,
        "insert 带引号",
        r#"insert into "DBTESTA"."P_QUOTED" (id, note) values (2, 'b')"#,
    );
    try_sql(
        &conn,
        "select 一条语句混用",
        r#"select count(*) from DBTESTA.P_PLAIN p, "DBTESTA"."P_QUOTED" q where p.id = 1"#,
    );

    println!("\n=== 4. 目录表：对象落在哪个 owner ===");
    show(
        &conn,
        "select owner || '.' || table_name from all_tables where table_name in ('P_QUOTED','P_PLAIN') order by 1",
    );
    show(
        &conn,
        "select cast(count(*) as varchar(20)) from all_tables where table_name in ('P_QUOTED','P_PLAIN')",
    );

    println!("\n=== 5. 小写模式：不带引号会被折成大写 ===");
    try_sql(
        &conn,
        "create \"dbtest_lower\".\"P_LOWER\"",
        r#"create table "dbtest_lower"."P_LOWER" (id int)"#,
    );
    try_sql(
        &conn,
        "select  \"s\".\"t\"  （带引号）",
        r#"select count(*) from "dbtest_lower"."P_LOWER""#,
    );
    try_sql(
        &conn,
        "select  s.t        （不带引号）",
        "select count(*) from dbtest_lower.P_LOWER",
    );

    println!("\n=== 6. 混合大小写表名：不带引号被折成大写 ===");
    try_sql(
        &conn,
        "create \"S\".\"P_MixedCase\"",
        r#"create table "DBTESTA"."P_MixedCase" (id int)"#,
    );
    try_sql(
        &conn,
        "select  S.T        （不带引号，折大写）",
        "select count(*) from DBTESTA.P_MixedCase",
    );
    try_sql(
        &conn,
        "select  \"S\".\"T\"  （带引号，逐字）",
        r#"select count(*) from "DBTESTA"."P_MixedCase""#,
    );

    println!("\n=== 7. 清场 ===");
    for (label, name) in [
        ("P_QUOTED", r#""DBTESTA"."P_QUOTED""#),
        ("P_PLAIN", r#""DBTESTA"."P_PLAIN""#),
        ("P_MixedCase", r#""DBTESTA"."P_MixedCase""#),
        ("P_LOWER", r#""dbtest_lower"."P_LOWER""#),
    ] {
        try_sql(
            &conn,
            &format!("drop {label}"),
            &format!("drop table if exists {name} cascade constraints"),
        );
    }
    try_sql(
        &conn,
        "drop schema dbtest_lower",
        r#"drop schema "dbtest_lower" restrict"#,
    );

    Ok(())
}
