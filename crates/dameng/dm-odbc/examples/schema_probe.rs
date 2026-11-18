//! 模式（schema）探针：达梦的「连接后默认模式」到底由什么决定。
//!
//! 起因：Round 34 的 `DBTESTA` / `DBTESTB` 验收里发现——连接串写 `SCHEMA=DBTESTA`
//! 连上后 `SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID)` 仍然是 `SYSDBA`，而未限定的 DDL
//! 也落在 `SYSDBA` 里。也就是说 `DmConfig::schema` 目前只是「把关键字塞进连接串」，
//! 对达梦 ODBC **不起作用**。
//!
//! 这个探针只打印、不断言（定位工具，不是门禁），要回答四件事：
//!
//! 1. 连接串里到底有没有**任何**关键字能让默认模式生效（`SCHEMA` / `SCHEMA_NAME` /
//!    `CURRENT_SCHEMA`，以及组合）。
//! 2. SQL 侧哪些写法真的能改会话默认模式（`SET SCHEMA` 带不带 `=`、引号、分号，
//!    `ALTER SESSION SET CURRENT_SCHEMA`），以及不存在模式时报什么。
//! 3. 能不能把「字符集自检」和「当前模式」并成**一条**查询（决定我们是否需要多一次
//!    往返：`select unicode()` 现在每次建连都要跑）。
//! 4. 模式名大小写敏感度：小写模式必须加引号吗（决定我们拼 `SET SCHEMA` 时是否总加引号）。
//!
//! 跑法：
//! ```text
//! $env:DM_TEST_DSN='DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc run -p dm-odbc --example schema_probe
//! ```

use odbc_api::{Connection, ConnectionOptions, Cursor, Environment};

type R<T> = Result<T, String>;

fn base_dsn() -> R<String> {
    match std::env::var("DM_TEST_DSN") {
        Ok(dsn) if !dsn.trim().is_empty() => Ok(dsn),
        _ => Err("需要 DM_TEST_DSN（例如 DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA）".into()),
    }
}

/// 在基础连接串后面追加若干 `KEY=VALUE`（基础串本来以 `;` 结尾与否都兼容）。
fn with_pairs(base: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = base.trim_end_matches(';').to_owned();
    for (k, v) in pairs {
        out.push(';');
        out.push_str(k);
        out.push('=');
        out.push_str(v);
    }
    out
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

fn ddl(conn: &Connection<'_>, sql: &str) -> R<()> {
    conn.execute(sql, (), None)
        .map(|_| ())
        .map_err(|e| format!("{sql} → {e}"))
}

fn current_schema(conn: &Connection<'_>) -> R<String> {
    q_text(
        conn,
        "select SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from dual",
    )
    .map(|v| v.unwrap_or_else(|| "<NULL>".into()))
}

/// 取首行的前两列宽文本（`None` = 该列 NULL / 没有行）。
fn q_two(conn: &Connection<'_>, sql: &str) -> R<Option<(Option<String>, Option<String>)>> {
    let Some(mut cur) = conn
        .execute(sql, (), None)
        .map_err(|e| format!("{sql} → {e}"))?
    else {
        return Ok(None);
    };
    let Some(mut row) = cur.next_row().map_err(|e| format!("next_row: {e}"))? else {
        return Ok(None);
    };
    let mut one: Vec<u16> = Vec::new();
    let mut two: Vec<u16> = Vec::new();
    let has_one = row.get_wide_text(1, &mut one).map_err(|e| e.to_string())?;
    let has_two = row.get_wide_text(2, &mut two).map_err(|e| e.to_string())?;
    Ok(Some((
        has_one.then(|| String::from_utf16_lossy(&one)),
        has_two.then(|| String::from_utf16_lossy(&two)),
    )))
}

fn main() -> Result<(), String> {
    let base = base_dsn()?;
    println!("基础连接串：{base}\n");
    let env = Environment::new().map_err(|e| format!("Environment: {e}"))?;
    let mut failures = 0usize;

    // ---------------------------------------------------------------
    // A. 连接串关键字
    // ---------------------------------------------------------------
    println!("=== A. 连接串关键字能否决定默认模式（期望 DBTESTA 才算生效）===");
    let variants: Vec<(&str, Vec<(&str, &str)>)> = vec![
        ("基线（不带任何模式关键字）", vec![]),
        ("SCHEMA=DBTESTA", vec![("SCHEMA", "DBTESTA")]),
        ("SCHEMA_NAME=DBTESTA", vec![("SCHEMA_NAME", "DBTESTA")]),
        (
            "CURRENT_SCHEMA=DBTESTA",
            vec![("CURRENT_SCHEMA", "DBTESTA")],
        ),
        (
            "SCHEMA=DBTESTA;CURRENT_SCHEMA=DBTESTA",
            vec![("SCHEMA", "DBTESTA"), ("CURRENT_SCHEMA", "DBTESTA")],
        ),
    ];
    for (name, pairs) in &variants {
        let cs = with_pairs(&base, pairs);
        match open(&env, &cs).and_then(|conn| current_schema(&conn)) {
            Ok(schema) => {
                let ok = schema.eq_ignore_ascii_case("DBTESTA");
                println!(
                    "  {:<45} → 当前模式 = {schema:<12} {}",
                    name,
                    if ok { "[生效]" } else { "[忽略]" }
                );
            }
            Err(err) => {
                failures += 1;
                println!("  {:<45} → 失败：{err}", name);
            }
        }
    }

    // ---------------------------------------------------------------
    // B. SQL 侧：哪些写法真的改默认模式
    // ---------------------------------------------------------------
    println!("\n=== B. SQL 写法（同一条连接上依次改）===");
    let conn = open(&env, &base)?;
    println!("  建连时默认模式 = {}", current_schema(&conn)?);
    let statements: Vec<&str> = vec![
        "SET SCHEMA DBTESTA",
        "SET SCHEMA DBTESTB",
        "SET SCHEMA = DBTESTA",
        "SET SCHEMA \"DBTESTB\"",
        "SET SCHEMA DBTESTA;",
        "ALTER SESSION SET CURRENT_SCHEMA = DBTESTB",
    ];
    for sql in statements {
        match ddl(&conn, sql) {
            Ok(()) => match current_schema(&conn) {
                Ok(schema) => println!("  {sql:<45} → OK，当前模式 = {schema}"),
                Err(err) => {
                    failures += 1;
                    println!("  {sql:<45} → 执行成功但读不回模式：{err}");
                }
            },
            Err(err) => println!("  {sql:<45} → 拒绝：{err}"),
        }
    }

    println!("\n=== B2. 改到不存在的模式 ===");
    match ddl(&conn, "SET SCHEMA NO_SUCH_SCHEMA_X") {
        Ok(()) => println!("  SET SCHEMA NO_SUCH_SCHEMA_X → 竟然成功（需复查）"),
        Err(err) => println!("  SET SCHEMA NO_SUCH_SCHEMA_X → 拒绝：{err}"),
    }

    // ---------------------------------------------------------------
    // C. 自检 + 当前模式能否并成一条
    // ---------------------------------------------------------------
    println!("\n=== C. 自检与模式能否一条查回 ===");
    match q_two(
        &conn,
        "select unicode(), SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from dual",
    ) {
        Ok(Some((charset, schema))) => {
            println!("  OK  一条查询同时取到：unicode={charset:?}、当前模式={schema:?}")
        }
        Ok(None) => println!("  查询没有返回行"),
        Err(err) => {
            failures += 1;
            println!("  失败：{err}");
        }
    }

    // ---------------------------------------------------------------
    // D. 大小写：小写模式名是否必须加引号
    // ---------------------------------------------------------------
    println!("\n=== D. 小写模式名与引号 ===");
    match ddl(&conn, "CREATE SCHEMA \"dbtest_lower\" AUTHORIZATION SYSDBA") {
        Ok(()) => println!("  CREATE SCHEMA \"dbtest_lower\" → OK"),
        Err(err) => println!("  CREATE SCHEMA \"dbtest_lower\" → {err}"),
    }
    for sql in ["SET SCHEMA dbtest_lower", "SET SCHEMA \"dbtest_lower\""] {
        match ddl(&conn, sql) {
            Ok(()) => match current_schema(&conn) {
                Ok(schema) => println!("  {sql:<32} → OK，当前模式 = {schema}"),
                Err(err) => println!("  {sql:<32} → {err}"),
            },
            Err(err) => println!("  {sql:<32} → 拒绝：{err}"),
        }
    }
    if let Err(err) = ddl(&conn, "DROP SCHEMA \"dbtest_lower\" RESTRICT") {
        println!("  DROP SCHEMA \"dbtest_lower\" RESTRICT → {err}");
    }

    // ---------------------------------------------------------------
    // E. 会话级还是连接级：新连接应回到登录用户的默认模式
    // ---------------------------------------------------------------
    println!("\n=== E. 新连接是否回到登录用户默认模式 ===");
    match open(&env, &base).and_then(|c| current_schema(&c)) {
        Ok(schema) => {
            println!("  新连接默认模式 = {schema}（期望 SYSDBA：SET SCHEMA 只影响当前会话）")
        }
        Err(err) => {
            failures += 1;
            println!("  失败：{err}");
        }
    }

    // ---------------------------------------------------------------
    // F. 清理用语法：`DROP SCHEMA IF EXISTS` 与模式目录
    // ---------------------------------------------------------------
    println!("\n=== F. DROP SCHEMA IF EXISTS / 模式目录 ===");
    for sql in [
        "drop schema if exists \"no_such_zzz\" restrict",
        "select count(*) from SYS.ALL_USERS where lower(username) = 'dbtesta'",
    ] {
        match q_text(&conn, sql) {
            Ok(Some(v)) => println!("  {sql:<62} → OK（值 {v}）"),
            Ok(None) => println!("  {sql:<62} → OK（无结果集）"),
            Err(err) => println!("  {sql:<62} → 拒绝：{err}"),
        }
    }

    println!("\n探针结束：{failures} 处异常需要复查");
    Ok(())
}
