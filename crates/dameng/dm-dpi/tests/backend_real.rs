//! `dm-dpi` 后端的真机测试。
//!
//! 需要环境变量 `DM_TEST_DSN`（与 `dm-odbc` 的真机测试同一口径），没有就直接
//! 返回——这些用例全部标了 `#[ignore]`，要在真机上跑：
//!
//! ```text
//! $env:DM_TEST_DSN='DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo test -p dm-dpi --test backend_real -- --include-ignored --test-threads=1
//! ```
//!
//! 注意 DSN 只用来取「服务器/端口/用户/密码」四项；`dm-dpi` 自己解析它，
//! 不经过 ODBC 驱动（`DRIVER=` 段被忽略）。

use std::str::FromStr;

use dm_core::backend::{
    Backend, ExecOutcome, FetchState, LobBytes, LobKind, LobParam, LobSource, RowBuf,
};
use dm_core::{DmConfig, DmValue};
use dm_dpi::DpiBackend;
use dm_error::DmErrorKind;
use rust_decimal::Decimal;

const TYPES_TABLE: &str = "DM_DPI_T1";
const NULL_TABLE: &str = "DM_DPI_T2";
const TXN_TABLE: &str = "DM_DPI_T3";
const LOB_TABLE: &str = "DM_DPI_T4";
const BULK_TABLE: &str = "DM_DPI_T5";

fn dsn() -> Option<String> {
    std::env::var("DM_TEST_DSN")
        .ok()
        .filter(|dsn| !dsn.trim().is_empty())
}

fn config(dsn: &str, schema: Option<&str>) -> DmConfig {
    let mut config = DmConfig::from_connection_string(dsn).expect("DSN 能解析");
    config.schema = schema.map(str::to_owned);
    config.pool.min_idle = 0;
    config.pool.max_size = 2;
    config
}

/// 连一条后端；没有 `DM_TEST_DSN` 时返回 `None`（用例直接过）。
fn connect(schema: Option<&str>) -> Option<DpiBackend> {
    let dsn = dsn()?;
    let config = config(&dsn, schema);
    Some(DpiBackend::connect(&config).expect("连接达梦"))
}

/// 执行一条不取数的语句。
fn exec(backend: &mut DpiBackend, sql: &str, args: &[DmValue]) -> ExecOutcome {
    let mut stmt = backend.prepare(sql).expect("prepare");
    backend.bind_params(&mut stmt, args).expect("bind");
    backend.execute(&mut stmt, sql).expect("execute")
}

/// 执行一条查询并取回全部行。
/// 先删表：`prepare` 不执行等于没删，所以真发一次。
fn drop_if_exists(backend: &mut DpiBackend, table: &str) {
    let sql = format!("drop table if exists {table} cascade constraints");
    // 表不存在/正被占用都不影响用例，失败就算了。
    if let Ok(mut stmt) = backend.prepare(&sql) {
        let _ = backend.execute(&mut stmt, &sql);
    }
}
fn query(backend: &mut DpiBackend, sql: &str, args: &[DmValue]) -> Vec<Vec<DmValue>> {
    let mut stmt = backend.prepare(sql).expect("prepare");
    backend.bind_params(&mut stmt, args).expect("bind");
    let outcome = backend.execute(&mut stmt, sql).expect("execute");
    let columns = match outcome {
        ExecOutcome::ResultSet { columns } => columns,
        _ => panic!("这条语句没有结果集：{sql}"),
    };
    let mut rows = RowBuf::new(columns.len());
    loop {
        match backend.fetch(&mut stmt, &mut rows).expect("fetch") {
            FetchState::Rows { rows } if rows > 0 => {}
            _ => break,
        }
    }
    rows.rows().map(<[DmValue]>::to_vec).collect()
}

fn scalar(backend: &mut DpiBackend, sql: &str, args: &[DmValue]) -> DmValue {
    let rows = query(backend, sql, args);
    assert_eq!(rows.len(), 1, "期望恰好一行：{sql}");
    rows[0][0].clone()
}

fn text(backend: &mut DpiBackend, sql: &str, args: &[DmValue]) -> String {
    match scalar(backend, sql, args) {
        DmValue::Text(value) => value,
        other => panic!("期望文本，拿到 {other:?}"),
    }
}

fn count(backend: &mut DpiBackend, table: &str) -> i64 {
    match scalar(backend, &format!("select count(*) from {table}"), &[]) {
        DmValue::BigInt(value) => value,
        DmValue::Int(value) => i64::from(value),
        other => panic!("count 返回 {other:?}"),
    }
}

#[tokio::test]
#[ignore = "真机用例：需要 DM_TEST_DSN"]
async fn connect_ping_and_default_schema() {
    let Some(mut backend) = connect(Some("DBTESTA")) else {
        return;
    };
    assert_eq!(backend.name(), "dm-dpi");
    assert!(backend.ping().expect("探活"));

    assert_eq!(
        text(
            &mut backend,
            "select SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from dual",
            &[]
        ),
        "DBTESTA",
        "cfg.schema 要真的生效（走 SET SCHEMA，不是连接串 SCHEMA=）"
    );

    // `init_sql` 仍然在 schema 之后逐条执行。
    let dsn = dsn().expect("DSN");
    let mut config = config(&dsn, Some("DBTESTA"));
    config.init_sql = vec!["SET SCHEMA \"DBTESTB\"".to_string()];
    let mut backend = DpiBackend::connect(&config).expect("连接达梦");
    assert_eq!(
        text(
            &mut backend,
            "select SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from dual",
            &[]
        ),
        "DBTESTB",
        "init_sql 后执行，能覆盖 schema"
    );
}

#[tokio::test]
#[ignore = "真机用例：需要 DM_TEST_DSN"]
async fn types_round_trip() {
    let Some(mut backend) = connect(Some("DBTESTA")) else {
        return;
    };
    drop_if_exists(&mut backend, TYPES_TABLE);
    exec(
        &mut backend,
        &format!(
            "create table {TYPES_TABLE} (
                id int not null primary key,
                c1 varchar(64),
                c2 decimal(18, 4),
                c3 double,
                c4 timestamp,
                c5 date,
                c6 time,
                c7 varbinary(32)
            )"
        ),
        &[],
    );

    let timestamp = chrono::NaiveDate::from_ymd_opt(2024, 5, 27)
        .unwrap()
        .and_hms_micro_opt(10, 11, 12, 123_456)
        .unwrap();
    let args = [
        DmValue::Int(7),
        DmValue::Text("达梦 DPI".to_string()),
        DmValue::Decimal(Decimal::from_str("123.4500").unwrap()),
        DmValue::Double(1.5),
        DmValue::Timestamp(timestamp),
        DmValue::Date(chrono::NaiveDate::from_ymd_opt(2024, 5, 27).unwrap()),
        DmValue::Time(chrono::NaiveTime::from_hms_opt(10, 11, 12).unwrap()),
        DmValue::Bytes(vec![1, 2, 3, 4, 5]),
    ];
    let outcome = exec(
        &mut backend,
        &format!(
            "insert into {TYPES_TABLE} (id, c1, c2, c3, c4, c5, c6, c7)
             values (?, ?, ?, ?, ?, ?, ?, ?)"
        ),
        &args,
    );
    assert!(matches!(outcome, ExecOutcome::Affected { rows: Some(1) }));

    let rows = query(
        &mut backend,
        &format!("select id, c1, c2, c3, c4, c5, c6, c7 from {TYPES_TABLE}"),
        &[],
    );
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row[0], DmValue::Int(7));
    assert_eq!(row[1], DmValue::Text("达梦 DPI".to_string()));
    assert_eq!(row[2], args[2]);
    assert_eq!(row[3], DmValue::Double(1.5));
    assert_eq!(row[4], DmValue::Timestamp(timestamp));
    assert_eq!(row[5], args[5]);
    assert_eq!(row[6], args[6]);
    assert_eq!(row[7], DmValue::Bytes(vec![1, 2, 3, 4, 5]));

    exec(&mut backend, &format!("drop table {TYPES_TABLE}"), &[]);
}

#[tokio::test]
#[ignore = "真机用例：需要 DM_TEST_DSN"]
async fn nulls_and_affected_rows() {
    let Some(mut backend) = connect(Some("DBTESTA")) else {
        return;
    };
    drop_if_exists(&mut backend, NULL_TABLE);
    exec(
        &mut backend,
        &format!("create table {NULL_TABLE} (id int not null primary key, c1 varchar(32), c2 int)"),
        &[],
    );

    exec(
        &mut backend,
        &format!("insert into {NULL_TABLE} (id, c1, c2) values (?, ?, ?)"),
        &[DmValue::Int(1), DmValue::Null, DmValue::Null],
    );
    let rows = query(
        &mut backend,
        &format!("select id, c1, c2 from {NULL_TABLE}"),
        &[],
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], DmValue::Int(1));
    assert!(rows[0][1].is_null(), "NULL 文本要读回 Null");
    assert!(rows[0][2].is_null(), "NULL 整数要读回 Null");

    let outcome = exec(
        &mut backend,
        &format!("update {NULL_TABLE} set c2 = ? where id = ?"),
        &[DmValue::Int(9), DmValue::Int(1)],
    );
    assert!(matches!(outcome, ExecOutcome::Affected { rows: Some(1) }));

    let outcome = exec(
        &mut backend,
        &format!("delete from {NULL_TABLE} where id = ?"),
        &[DmValue::Int(1)],
    );
    assert!(matches!(outcome, ExecOutcome::Affected { rows: Some(1) }));
    assert_eq!(count(&mut backend, NULL_TABLE), 0);

    exec(&mut backend, &format!("drop table {NULL_TABLE}"), &[]);
}

#[tokio::test]
#[ignore = "真机用例：需要 DM_TEST_DSN"]
async fn transaction_rollback_and_reset() {
    let Some(mut backend) = connect(Some("DBTESTA")) else {
        return;
    };
    drop_if_exists(&mut backend, TXN_TABLE);
    exec(
        &mut backend,
        &format!("create table {TXN_TABLE} (id int not null primary key)"),
        &[],
    );

    backend.begin().expect("begin");
    exec(
        &mut backend,
        &format!("insert into {TXN_TABLE} (id) values (?)"),
        &[DmValue::Int(1)],
    );
    backend.rollback().expect("rollback");
    assert_eq!(count(&mut backend, TXN_TABLE), 0, "回滚后不该有行");

    backend.begin().expect("begin");
    exec(
        &mut backend,
        &format!("insert into {TXN_TABLE} (id) values (?)"),
        &[DmValue::Int(2)],
    );
    backend.commit().expect("commit");
    assert_eq!(count(&mut backend, TXN_TABLE), 1, "提交后要有行");

    // `reset` 要回到自动提交：不提交也不回滚的插入会立刻可见。
    backend.reset().expect("reset");
    exec(
        &mut backend,
        &format!("insert into {TXN_TABLE} (id) values (?)"),
        &[DmValue::Int(3)],
    );
    assert_eq!(
        count(&mut backend, TXN_TABLE),
        2,
        "reset 之后应恢复自动提交"
    );

    exec(&mut backend, &format!("drop table {TXN_TABLE}"), &[]);
}

#[tokio::test]
#[ignore = "真机用例：需要 DM_TEST_DSN"]
async fn lob_write_and_read() {
    let Some(mut backend) = connect(Some("DBTESTA")) else {
        return;
    };
    drop_if_exists(&mut backend, LOB_TABLE);
    exec(
        &mut backend,
        &format!("create table {LOB_TABLE} (id int not null primary key, b blob, c clob)"),
        &[],
    );

    // 1 MiB 二进制：内容可校验（每字节 = 下标低位）。
    let mut blob = Vec::with_capacity(1024 * 1024);
    for index in 0..1024 * 1024u32 {
        blob.push((index % 251) as u8);
    }
    let clob = "达梦 DPI 大对象。".repeat(10_000);

    let mut stmt = backend
        .prepare(&format!(
            "insert into {LOB_TABLE} (id, b, c) values (?, ?, ?)"
        ))
        .expect("prepare insert");
    backend
        .bind_params(
            &mut stmt,
            &[
                DmValue::Int(1),
                DmValue::Bytes(blob.clone()),
                DmValue::Text(clob.clone()),
            ],
        )
        .expect("bind");
    backend.execute(&mut stmt, "").expect("execute");

    // 走 fetch：后端会把整值读进 `RowBuf`。
    let rows = query(&mut backend, &format!("select b, c from {LOB_TABLE}"), &[]);
    assert_eq!(rows.len(), 1);
    match &rows[0][0] {
        DmValue::Bytes(bytes) => assert_eq!(bytes, &blob, "BLOB 逐字节一致"),
        other => panic!("BLOB 读成 {other:?}"),
    }
    match &rows[0][1] {
        DmValue::Text(value) => assert_eq!(value, &clob, "CLOB 逐字节一致"),
        other => panic!("CLOB 读成 {other:?}"),
    }

    // 走流式接口：lob_start / lob_next_row / lob_chunk。
    let mut stmt = backend
        .prepare(&format!("select b from {LOB_TABLE} where id = ?"))
        .expect("prepare select");
    backend
        .bind_params(&mut stmt, &[DmValue::Int(1)])
        .expect("bind");
    backend.execute(&mut stmt, "").expect("execute");
    backend.lob_start(&mut stmt, 0).expect("lob_start");
    assert!(backend.lob_next_row(&mut stmt).expect("lob_next_row"));
    let mut streamed = Vec::new();
    loop {
        let chunk = backend
            .lob_chunk(&mut stmt, &mut streamed)
            .expect("lob_chunk");
        if chunk.is_done() {
            break;
        }
    }
    drop(stmt);
    assert_eq!(streamed, blob, "流式读也要逐字节一致");

    exec(&mut backend, &format!("drop table {LOB_TABLE}"), &[]);
}

#[tokio::test]
#[ignore = "真机用例：需要 DM_TEST_DSN"]
async fn bulk_param_array() {
    let Some(mut backend) = connect(Some("DBTESTA")) else {
        return;
    };
    drop_if_exists(&mut backend, BULK_TABLE);
    exec(
        &mut backend,
        &format!("create table {BULK_TABLE} (id int not null primary key, c1 varchar(32))"),
        &[],
    );

    // 列式：外层是列，内层是每一行的该列值（与 `Backend::execute_param_array` 的契约一致）。
    let columns: Vec<Vec<DmValue>> = vec![
        (0..500).map(DmValue::Int).collect(),
        (0..500)
            .map(|index| DmValue::Text(format!("row-{index}")))
            .collect(),
    ];
    let mut stmt = backend
        .prepare(&format!("insert into {BULK_TABLE} (id, c1) values (?, ?)"))
        .expect("prepare");
    let outcome = backend
        .execute_param_array(&mut stmt, &columns)
        .expect("execute_param_array");
    assert_eq!(outcome.executed, 500);
    assert_eq!(outcome.affected, 500);
    assert!(outcome.failed_at.is_none());
    assert_eq!(count(&mut backend, BULK_TABLE), 500);

    exec(&mut backend, &format!("drop table {BULK_TABLE}"), &[]);
}

#[tokio::test]
#[ignore = "真机用例：需要 DM_TEST_DSN"]
async fn server_errors_keep_code_and_sqlstate() {
    let Some(mut backend) = connect(Some("DBTESTA")) else {
        return;
    };
    let error = backend
        .prepare("select * from DM_DPI_NO_SUCH_TABLE")
        .and_then(|mut stmt| backend.execute(&mut stmt, ""))
        .expect_err("不存在的表必须报错");
    // 后端与 `dm-odbc` 对齐：`-2106` 先经 `classify_server` 归类成 `ObjectNotFound`，
    // 原始码与消息仍保留在诊断记录里（Round 41 起）。两种形态都接受，判定以诊断为准。
    let diag = error
        .diags
        .iter()
        .find(|d| d.native_code == -2106)
        .expect("诊断记录里要有 -2106 原始码");
    assert_eq!(diag.sqlstate, *b"HY000", "DPI 不给 SQLSTATE，统一按 HY000");
    assert!(
        diag.message.contains("DM_DPI_NO_SUCH_TABLE"),
        "错误消息要带对象名：{}",
        diag.message
    );
    assert!(
        matches!(
            error.kind,
            DmErrorKind::ObjectNotFound | DmErrorKind::Server { .. }
        ),
        "期望 ObjectNotFound 或 Server，拿到 {:?}",
        error.kind
    );
}

#[tokio::test]
#[ignore = "真机用例：需要 DM_TEST_DSN"]
async fn execute_lob_writes_bytes() {
    let Some(mut backend) = connect(Some("DBTESTA")) else {
        return;
    };
    drop_if_exists(&mut backend, LOB_TABLE);
    exec(
        &mut backend,
        &format!("create table {LOB_TABLE} (id int not null primary key, b blob)"),
        &[],
    );

    let payload: Vec<u8> = (0..300_000u32).map(|index| (index % 253) as u8).collect();
    let mut source = LobBytes::new(payload.clone());
    let mut lob = LobParam::new(1, LobKind::Binary, &mut source as &mut dyn LobSource);
    let mut stmt = backend
        .prepare(&format!("insert into {LOB_TABLE} (id, b) values (?, ?)"))
        .expect("prepare");
    let outcome = backend
        .execute_lob(&mut stmt, "", &[DmValue::Int(2)], &mut lob)
        .expect("execute_lob");
    assert!(matches!(outcome, ExecOutcome::Affected { rows: Some(1) }));

    let rows = query(&mut backend, &format!("select b from {LOB_TABLE}"), &[]);
    match &rows[0][0] {
        DmValue::Bytes(bytes) => assert_eq!(bytes, &payload),
        other => panic!("BLOB 读成 {other:?}"),
    }

    exec(&mut backend, &format!("drop table {LOB_TABLE}"), &[]);
}
