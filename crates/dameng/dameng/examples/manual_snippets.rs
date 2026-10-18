#![allow(dead_code)] // 示例只做编译验证：这些函数不被调用，`main` 也不连库。
//! `docs/达梦中间件使用手册.md` 里的关键片段，**编译可验证**版本。
//!
//! 这个示例不做任何断言、也不会被门禁当作功能测试：它存在的唯一目的是保证手册里的
//! 代码不是「看起来能跑」。改 API 而忘了改手册时，`cargo check --example manual_snippets`
//! 会先红。
//!
//! ```text
//! cargo +stable-x86_64-pc-windows-msvc check -p dameng --example manual_snippets --all-features
//! ```

use std::future::poll_fn;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use dameng::prelude::*;
use dm_query::{LobBytes, LobKind, LobParam, LobReader, LobRef};
use futures_core::Stream;

type R<T> = Result<T, DmError>;

/// 手册 §6.1：三种配置来源 + builder。
fn config_forms() -> R<()> {
    let _cfg1 = DmConfig::from_connection_string(
        "DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA",
    )?;
    let _cfg2 = DmConfig::parse("dm://SYSDBA:SYSDBA@127.0.0.1:5236/DBTESTA")?;

    let mut cfg = DmConfig::builder()
        .server("127.0.0.1")
        .port(5236)
        .user("SYSDBA")
        .password("SYSDBA")
        .schema("DBTESTA")
        .pool(PoolConfig {
            max_size: 8,
            min_idle: 2,
            ..PoolConfig::default()
        })
        .init_sql(["ALTER SESSION SET CURRENT_SCHEMA = DBTESTA"])
        .build()?;

    // 逃生舱
    cfg.extra.insert("KEEPALIVE".into(), "1".into());

    let _ = cfg.to_connection_string();
    let _ = cfg.to_url();
    cfg.validate()?;
    Ok(())
}

/// 手册 §2.3 / §7：查询、写入、取值。
async fn basic_usage(pool: &Pool) -> R<()> {
    let mut conn = PooledConn::acquire(pool).await?;

    // query_one：没有行是 Ok(None)，不是错误
    let row = conn.query_one("select 1 as N from dual", ()).await?;
    if let Some(row) = row {
        let _n: i32 = row.get("N")?;
        let _also: i32 = row.get(0)?;
        let _raw: &DmValue = row.value("N")?;
    }

    // query：材料化结果集
    let rs: RowSet = conn
        .query("select ID, NAME from T_DEMO where ID >= ?", (0i32,))
        .await?;
    let _cols = rs.columns().len();
    for r in rs.rows() {
        let _id: i64 = r.get("ID")?;
        let _name: Option<String> = r.get("NAME")?;
    }

    // execute：DML / DDL 都走它
    let _affected: u64 = conn
        .execute(
            "insert into T_DEMO (ID, NAME) values (?, ?)",
            (1i32, "张三"),
        )
        .await?;
    let _ = conn
        .execute(
            "create table T_DEMO (ID int primary key, NAME varchar(64))",
            (),
        )
        .await;
    let _ = conn.execute("drop table T_DEMO", ()).await;

    Ok(())
}

/// 手册 §8.2：`IntoArgs` 的几种写法，含动态条件。
async fn dynamic_args(conn: &mut PooledConn, min_id: Option<i64>, keyword: Option<&str>) -> R<()> {
    let mut sql = String::from("select ID, NAME from T_USER where 1 = 1");
    let mut args: Vec<DmValue> = Vec::new();
    if let Some(min) = min_id {
        sql.push_str(" and ID >= ?");
        args.push(DmValue::BigInt(min));
    }
    if let Some(kw) = keyword {
        sql.push_str(" and NAME like ?");
        args.push(DmValue::Text(format!("%{kw}%")));
    }
    let _ = conn.query(&sql, args).await?;
    Ok(())
}

/// 手册 §9：事务、隔离级别、保存点。
async fn transactions(pool: &Pool) -> R<()> {
    let conn = PooledConn::acquire(pool).await?;
    let mut tx = conn.begin().await?;
    tx.execute("insert into T (ID) values (?)", (1i32,)).await?;
    tx.savepoint("sp1").await?;
    tx.execute("insert into T (ID) values (?)", (2i32,)).await?;
    tx.rollback_to("sp1").await?;
    tx.release_savepoint("sp1").await?;
    tx.commit().await?;

    let conn = PooledConn::acquire(pool).await?;
    let tx = conn.begin_with(IsolationLevel::Serializable).await?;
    let mut tx = tx;
    tx.execute("insert into T (ID) values (?)", (3i32,)).await?;
    tx.rollback().await?;

    let _all = IsolationLevel::ALL;
    Ok(())
}

/// 手册 §10：批处理。
async fn batch(conn: &mut PooledConn) -> R<()> {
    let rows: Vec<Vec<DmValue>> = (0..3)
        .map(|i| vec![DmValue::Int(i), DmValue::Text(format!("n{i}"))])
        .collect();
    let outcome: BatchOutcome = conn
        .execute_many("insert into T (ID, NAME) values (?, ?)", rows)
        .await?;
    let _ = (outcome.affected, outcome.executed, outcome.failed_at);
    Ok(())
}

/// 手册 §11：流式读取。
async fn streaming(pool: &Pool) -> R<()> {
    let conn = PooledConn::acquire(pool).await?;
    let mut stream = conn.fetch_stream("select ID from T_BIG", ()).await?;
    let mut n = 0u64;
    while let Some(row) = poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await {
        let row = row?;
        let _id: i64 = row.get("ID")?;
        n += 1;
    }
    let _conn_back = stream.into_conn().await;
    let _ = n;
    Ok(())
}

/// 手册 §12：大字段读写。
async fn lobs(pool: &Pool) -> R<()> {
    let mut conn = PooledConn::acquire(pool).await?;

    // 写：普通参数 + LobParam（0 基列下标）
    let _ = conn
        .execute(
            "create table T_DOC (ID int primary key, BODY blob, TXT clob)",
            (),
        )
        .await;
    let _affected = conn
        .execute_lob(
            "insert into T_DOC (ID, BODY) values (?, ?)",
            (1i32,),
            LobParam::new(1, LobKind::Binary, LobBytes::new(vec![0u8; 1024])),
        )
        .await?;

    // 从 reader 写
    let reader = std::io::Cursor::new(vec![1u8; 4096]);
    let _ = conn
        .execute_lob(
            "insert into T_DOC (ID, BODY) values (?, ?)",
            (2i32,),
            LobParam::new(1, LobKind::Binary, LobReader::with_size_hint(reader, 4096)),
        )
        .await;

    // 读：LOB 通道
    let mut stream = conn
        .lob_stream(
            "select ID, BODY from T_DOC where ID = ?",
            (1i32,),
            LobRef::name("BODY"),
        )
        .await?;
    let _name = stream.column().name.clone();
    if stream.next_row().await? {
        let mut buf = Vec::new();
        loop {
            // `is_done()` 对 `Done` 与 `Null` 都为真：空二进制与 SQL NULL 都收尾。
            if stream.read_chunk(&mut buf).await?.is_done() {
                break;
            }
        }
        let _bytes = stream.bytes_read();
    }
    let mut conn = stream.into_conn().await;
    let _ = conn.execute("drop table T_DOC", ()).await;
    Ok(())
}

/// 手册 §13：池统计与关闭语义。
async fn pool_ops(pool: &Pool) -> R<()> {
    let conn = PooledConn::acquire(pool).await?;
    let _stats: PoolStats = pool.stats();
    let _state: PoolState = pool.state();
    let _cache = conn.guard().stmt_cache_stats();
    let _conn_stats = conn.conn_stats();
    drop(conn);
    pool.close().await?;
    pool.wait_closed().await;
    Ok(())
}

/// 手册 §15：迁移。
async fn migrations(pool: Arc<Pool>) -> R<()> {
    let migrator = Migrator::new(pool.clone()).with_history_table("DM_SCHEMA_HISTORY")?;
    migrator.ensure_table().await?;
    let dir = Path::new("migrations");
    let pending = migrator.pending(dir).await?;
    if !pending.is_empty() {
        let _applied = migrator.apply_pending(dir).await?;
    }
    let _applied = migrator.applied().await?;
    Ok(())
}

/// 手册 §18：错误分类与重试判定。
fn error_switch(err: &DmError) -> bool {
    match err.kind {
        DmErrorKind::Io | DmErrorKind::ConnectionLost => true,
        _ => err.kind.is_retryable_kind(),
    }
}

/// 手册 §14：读写分离路由（需要 `rw-split` feature）。
#[cfg(feature = "rw-split")]
async fn router(primary: Arc<Pool>, replicas: Vec<Arc<Pool>>) -> R<()> {
    use dameng::{Router, RwMode, RwStrategy};

    let router = Router::new(primary, replicas, RwStrategy::RoundRobin)?;
    let mut conn = PooledConn::new(router.acquire(RwMode::ReadOnly).await?);
    let _ = conn.query("select 1 from dual", ()).await?;
    let conn = PooledConn::new(router.acquire_primary().await?);
    let _tx = conn.begin().await?;
    let _loads = router.replica_loads();
    Ok(())
}

fn main() {
    // 本示例只为编译检查，不连库。
    let _ = config_forms();
    let _: fn(&DmError) -> bool = error_switch;
    println!("manual_snippets：手册片段全部通过编译");
}
