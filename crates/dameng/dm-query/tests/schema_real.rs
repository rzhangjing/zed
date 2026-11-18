//! `DBTESTA` / `DBTESTB` 两个模式上的中间件功能测试。
//!
//! 与其它真机测试的区别：本文件的关注点是**模式（schema）语义**——连接默认模式怎么切、
//! 跨模式对象如何限定、批处理 / LOB / 事务 / 并发在两个独立模式里是否行为一致。
//!
//! 运行方式（模式需事先存在，见 `docs/验收报告` 的「测试环境」一节）：
//!
//! ```text
//! $env:DM_TEST_DSN='DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc test -p dm-query --test schema_real \
//!     -- --include-ignored --test-threads=1 --nocapture
//! ```
//!
//! 没有 `DM_TEST_DSN` 时每个用例直接 `return`（与其它真机测试同约定）。
//!
//! **重要事实**：达梦的 ODBC 连接串里 `SCHEMA=` 不生效（实测连接后仍是登录用户的默认模式），
//! 切换默认模式必须靠 `SET SCHEMA`。中间件已经把这件事**收进 `DmConfig::schema`**：
//! 建连时复用字符集自检那条查询取回当前模式，与目标不一致才补一条
//! `SET SCHEMA "<模式名>"`（已就位则一条 SQL 都不发，`crates/dm-odbc/src/conn.rs` 的
//! `inspect_session` / `apply_schema`）。本文件的绝大部分用例走 `DmConfig::schema` 这条路，
//! 另有 `dbtesta_config_schema_matches_init_sql` 锁住「和 `init_sql` 写法等价」。
//!
//! `chrono` / `decimal` 两个特性关掉时整个文件不参与编译（feature 矩阵门禁会
//! `cargo check --all-targets --no-default-features`）。

#![cfg(all(feature = "chrono", feature = "decimal"))]

use std::str::FromStr;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveTime};
use dm_core::{CharsetCheck, DmConfig, PoolConfig};
// 默认后端（dpi feature 打开时是 `dm_dpi::DpiBackend`），这样同一份真机用例能跑两种后端。
use dm_pool::{DmPool, ExecutorConfig};
use dm_query::DefaultBackend as OdbcBackend;
use dm_query::{LobBytes, LobKind, LobParam, LobRef, PooledConn};
use rust_decimal::Decimal;

const A: &str = "DBTESTA";
const B: &str = "DBTESTB";

fn dsn() -> Option<String> {
    std::env::var("DM_TEST_DSN")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// 建一个「默认模式 = `schema`」的配置。
///
/// 用 [`DmConfig::schema`]：中间件在建连时会自动 `SET SCHEMA "<模式名>"`（达梦忽略连接串里
/// 的 `SCHEMA=` 关键字，见文件头）。池里每条新连接各自设一次。
fn config_at(schema: &str, max_size: usize) -> Option<DmConfig> {
    Some(cfg_from_dsn(&dsn()?, schema, max_size))
}

/// 同一个连接串 + 目标模式 → 配置（`max_size` 至少 1）。
fn cfg_from_dsn(dsn: &str, schema: &str, max_size: usize) -> DmConfig {
    let mut cfg = DmConfig::from_connection_string(dsn).expect("DM_TEST_DSN 不是合法的连接串");
    cfg.schema = Some(schema.to_owned());
    cfg.pool = PoolConfig {
        max_size: max_size.max(1),
        min_idle: 0,
        ..PoolConfig::default()
    };
    cfg
}

/// 老写法：`init_sql` 里 `SET SCHEMA`。只用来证明两条路等价。
fn config_via_init_sql(schema: &str, max_size: usize) -> Option<DmConfig> {
    let mut cfg = config_at(schema, max_size)?;
    cfg.schema = None;
    cfg.init_sql = vec![format!("SET SCHEMA \"{schema}\"")];
    Some(cfg)
}
async fn pool_at(schema: &str, max_size: usize) -> Option<Arc<DmPool<OdbcBackend>>> {
    let cfg = config_at(schema, max_size)?;
    let pool = DmPool::connect_with(cfg, ExecutorConfig::for_pool(max_size.max(1)))
        .await
        .expect("建池失败（达梦是否可达？）");
    Some(Arc::new(pool))
}

async fn conn_of(pool: &DmPool<OdbcBackend>) -> PooledConn<OdbcBackend> {
    PooledConn::acquire(pool).await.expect("借连接失败")
}

async fn exec(conn: &mut PooledConn<OdbcBackend>, sql: &str) {
    conn.execute(sql, ())
        .await
        .unwrap_or_else(|err| panic!("执行失败：{sql}\n{err:?}"));
}

/// 执行并忽略错误：只用于幂等清场——达梦的 `drop table if exists` 在前面挂了不存在
/// 的模式名时并不兜住缺失的模式（会直接报 `无效的表或视图名`），所以清场不能假设顺序。
async fn try_exec(conn: &mut PooledConn<OdbcBackend>, sql: &str) {
    if let Err(err) = conn.execute(sql, ()).await {
        println!("[清场忽略] {sql} → {err:?}");
    }
}

async fn count(conn: &mut PooledConn<OdbcBackend>, sql: &str) -> i64 {
    conn.query_one(sql, ())
        .await
        .unwrap_or_else(|err| panic!("查询失败：{sql}\n{err:?}"))
        .expect("聚合查询必须返回一行")
        .get::<i64>(0)
        .expect("count 应当是整数")
}

async fn text_of(conn: &mut PooledConn<OdbcBackend>, sql: &str) -> String {
    conn.query_one(sql, ())
        .await
        .unwrap_or_else(|err| panic!("查询失败：{sql}\n{err:?}"))
        .expect("查询必须返回一行")
        .get::<String>(0)
        .expect("应当是文本")
}

async fn close(pool: Arc<DmPool<OdbcBackend>>) {
    match Arc::try_unwrap(pool) {
        Ok(pool) => {
            pool.close().await.expect("关池失败");
            pool.wait_closed().await;
        }
        Err(_) => panic!("还有连接没释放"),
    }
}

/// `DmConfig::schema` 真的生效：会话默认模式、未限定 DDL 的落点、以及
/// 未限定名的可见范围（跨模式对象在本模式下不可见）。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtesta_default_schema_comes_from_config() {
    let Some(pool) = pool_at(A, 2).await else {
        return;
    };

    {
        let mut conn = conn_of(&pool).await;

        let current = text_of(
            &mut conn,
            "select SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from DUAL",
        )
        .await;
        assert_eq!(current, A, "会话默认模式应当是 {A}（SET SCHEMA 生效）");

        exec(&mut conn, "drop table if exists T_SCHEMA_PROBE").await;
        exec(
            &mut conn,
            "create table T_SCHEMA_PROBE (ID INT PRIMARY KEY)",
        )
        .await;

        let owner = text_of(
            &mut conn,
            "select owner from SYS.ALL_TABLES where table_name = 'T_SCHEMA_PROBE'",
        )
        .await;
        assert_eq!(owner, A, "未限定的 CREATE TABLE 必须落在 {A} 里");

        // 未限定名只在本模式内查找：DBTESTB 的表在 DBTESTA 里应当不可见。
        let err = conn
            .query_one("select count(*) from T_JOIN_B", ())
            .await
            .expect_err("{B} 的表不应在 {A} 的默认模式里解析到");
        println!("\n[模式隔离] 未限定跨模式表名的错误：{err:?}");

        exec(&mut conn, "drop table T_SCHEMA_PROBE").await;
    }

    close(pool).await;
}

/// 单表 CRUD + 约束：默认值、参数化写、更新、删除、`NOT NULL` / 主键唯一性都由服务端拦下。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtesta_crud_and_constraints() {
    let Some(pool) = pool_at(A, 2).await else {
        return;
    };

    let mut conn = conn_of(&pool).await;
    exec(&mut conn, "drop table if exists T_CRUD").await;
    exec(
        &mut conn,
        "create table T_CRUD (ID INT PRIMARY KEY, NAME VARCHAR(64) NOT NULL, N INT DEFAULT 7, TS TIMESTAMP(6))",
    )
    .await;

    // 默认值：不给 N / TS。
    let affected = conn
        .execute("insert into T_CRUD (ID, NAME) values (?, ?)", (1i32, "甲"))
        .await
        .expect("插入默认值行失败");
    assert_eq!(affected, 1);

    let ts = NaiveDate::from_ymd_opt(2024, 1, 2)
        .unwrap()
        .and_hms_micro_opt(3, 4, 5, 123_456)
        .unwrap();
    conn.execute(
        "insert into T_CRUD (ID, NAME, N, TS) values (?, ?, ?, ?)",
        (2i32, "乙", 42i32, ts),
    )
    .await
    .expect("插入显式值行失败");

    let row = conn
        .query_one("select NAME, N, TS from T_CRUD where ID = ?", (1i32,))
        .await
        .expect("查询失败")
        .expect("必须有一行");
    assert_eq!(row.get::<String>(0).unwrap(), "甲");
    assert_eq!(row.get::<i32>(1).unwrap(), 7, "N 应当取默认值 7");
    // `NULL` 只能走 `Option<T>` 这条通道（`Row::get` 对非 `Option` 返回
    // `NullViolation` 并带列名，见 `crates/dm-query/src/row.rs` 的模块文档）。
    assert_eq!(
        row.get::<Option<chrono::NaiveDateTime>>(2).unwrap(),
        None,
        "TS 未赋值应为 NULL"
    );

    let row = conn
        .query_one("select TS from T_CRUD where ID = ?", (2i32,))
        .await
        .expect("查询失败")
        .expect("必须有一行");
    assert_eq!(
        row.get::<chrono::NaiveDateTime>(0).unwrap(),
        ts,
        "TIMESTAMP(6) 必须保留微秒"
    );

    // UPDATE
    let affected = conn
        .execute("update T_CRUD set N = N + 10 where ID = ?", (2i32,))
        .await
        .expect("更新失败");
    assert_eq!(affected, 1);
    assert_eq!(
        count(&mut conn, "select N from T_CRUD where ID = 2").await,
        52
    );

    // 约束：NOT NULL
    let err = conn
        .execute(
            "insert into T_CRUD (ID, NAME) values (?, ?)",
            (3i32, None::<String>),
        )
        .await
        .expect_err("NAME 是 NOT NULL，必须被拒绝");
    println!("\n[约束] NOT NULL 违反：{err:?}");

    // 约束：主键唯一
    let err = conn
        .execute("insert into T_CRUD (ID, NAME) values (?, ?)", (2i32, "丁"))
        .await
        .expect_err("主键重复必须被拒绝");
    println!("[约束] 主键唯一性违反：{err:?}");

    // DELETE
    let affected = conn
        .execute("delete from T_CRUD where ID = ?", (1i32,))
        .await
        .expect("删除失败");
    assert_eq!(affected, 1);
    assert_eq!(count(&mut conn, "select count(*) from T_CRUD").await, 1);

    exec(&mut conn, "drop table T_CRUD").await;
    drop(conn);
    close(pool).await;
}

/// 达梦类型映射的往返一致性：整数 / 浮点 / `DECIMAL` / 文本 / 日期时间 / 布尔 / 二进制。
///
/// 列类型都与 fork 的 `type.rs` 映射一致（`BIT` 代布尔、`DECIMAL(p,s)` 代 `Numeric(Some)`、
/// 时间类型带 `(6)` 精度）。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtestb_type_round_trip() {
    let Some(pool) = pool_at(B, 2).await else {
        return;
    };

    let mut conn = conn_of(&pool).await;
    exec(&mut conn, "drop table if exists T_TYPES").await;
    exec(
        &mut conn,
        "create table T_TYPES (\
             ID INT PRIMARY KEY, I INT, BI BIGINT, D DOUBLE, DECV DECIMAL(18,4), \
             S VARCHAR(64), TXT VARCHAR(8188), DT DATE, TM TIME(6), TS TIMESTAMP(6), \
             FLAG BIT, BIN VARBINARY(32)\
         )",
    )
    .await;

    let date = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
    let time = NaiveTime::from_hms_micro_opt(3, 4, 5, 123_456).unwrap();
    let ts = date.and_hms_micro_opt(3, 4, 5, 123_456).unwrap();
    let decimal = Decimal::from_str("12345.6789").unwrap();
    let long_text = "达梦".repeat(1_000); // 6000 字节，超过 `VARCHAR(64)`，在 `VARCHAR(8188)` 内
    let bytes = vec![0x00u8, 0xFF, 0x7F, 0x01];

    conn.execute(
        "insert into T_TYPES (ID, I, BI, D, DECV, S, TXT, DT, TM, TS, FLAG, BIN) \
         values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        (
            1i32,
            -7i32,
            9_000_000_000i64,
            2.5f64,
            decimal,
            "汉字abc",
            long_text.clone(),
            date,
            time,
            ts,
            true,
            bytes.clone(),
        ),
    )
    .await
    .expect("类型往返插入失败");

    let row = conn
        .query_one(
            "select I, BI, D, DECV, S, TXT, DT, TM, TS, FLAG, BIN from T_TYPES where ID = ?",
            (1i32,),
        )
        .await
        .expect("查询失败")
        .expect("必须有一行");

    assert_eq!(row.get::<i32>(0).unwrap(), -7);
    assert_eq!(row.get::<i64>(1).unwrap(), 9_000_000_000);
    assert_eq!(row.get::<f64>(2).unwrap(), 2.5);
    assert_eq!(
        row.get::<Decimal>(3).unwrap().normalize().to_string(),
        "12345.6789",
        "DECIMAL(18,4) 必须逐位一致"
    );
    assert_eq!(row.get::<String>(4).unwrap(), "汉字abc");
    assert_eq!(
        row.get::<String>(5).unwrap(),
        long_text,
        "长文本必须逐字符一致"
    );
    assert_eq!(row.get::<NaiveDate>(6).unwrap(), date);
    assert_eq!(row.get::<NaiveTime>(7).unwrap(), time);
    assert_eq!(row.get::<chrono::NaiveDateTime>(8).unwrap(), ts);
    assert!(row.get::<bool>(9).unwrap(), "BIT 列回读为 true");
    assert_eq!(
        row.get::<Vec<u8>>(10).unwrap(),
        bytes,
        "VARBINARY 必须逐字节一致"
    );

    // 布尔 false 也要能被正确区分（BIT 0/1）。
    conn.execute(
        "insert into T_TYPES (ID, FLAG) values (?, ?)",
        (2i32, false),
    )
    .await
    .expect("插入布尔 false 失败");
    let row = conn
        .query_one("select FLAG from T_TYPES where ID = ?", (2i32,))
        .await
        .expect("查询失败")
        .expect("必须有一行");
    assert!(!row.get::<bool>(0).unwrap(), "BIT 0 应当回读为 false");

    exec(&mut conn, "drop table T_TYPES").await;
    drop(conn);
    close(pool).await;
}

/// 跨模式 JOIN 与跨模式写：`DBTESTA` 的连接可以直接读 / 写 `DBTESTB` 的对象（加模式限定）。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtesta_cross_schema_join_and_write() {
    let Some(pool) = pool_at(A, 2).await else {
        return;
    };

    let mut conn = conn_of(&pool).await;
    for sql in [
        format!("drop table if exists {A}.T_JOIN_A"),
        format!("drop table if exists {B}.T_JOIN_B"),
        format!("create table {A}.T_JOIN_A (ID INT PRIMARY KEY, V VARCHAR(16) NOT NULL)"),
        format!("create table {B}.T_JOIN_B (ID INT PRIMARY KEY, W VARCHAR(16) NOT NULL)"),
    ] {
        exec(&mut conn, &sql).await;
    }
    for (id, v) in [(1i32, "甲"), (2i32, "乙")] {
        conn.execute(
            &format!("insert into {A}.T_JOIN_A (ID, V) values (?, ?)"),
            (id, v),
        )
        .await
        .expect("写 DBTESTA 失败");
    }
    for (id, w) in [(1i32, "一"), (3i32, "三")] {
        conn.execute(
            &format!("insert into {B}.T_JOIN_B (ID, W) values (?, ?)"),
            (id, w),
        )
        .await
        .expect("跨模式写 DBTESTB 失败");
    }

    // 内连接：只有 ID = 1 两边都有。
    let rs = conn
        .query(
            &format!(
                "select a.ID, a.V, b.W from {A}.T_JOIN_A a join {B}.T_JOIN_B b on a.ID = b.ID order by a.ID"
            ),
            (),
        )
        .await
        .expect("跨模式 JOIN 失败");
    assert_eq!(rs.len(), 1, "只有 ID=1 两边都存在");
    let row = &rs.rows()[0];
    assert_eq!(row.get::<i32>(0).unwrap(), 1);
    assert_eq!(row.get::<String>(1).unwrap(), "甲");
    assert_eq!(row.get::<String>(2).unwrap(), "一");

    // 左连接：DBTESTA 侧的两行都要在，缺失的 W 为 NULL。
    let rs = conn
        .query(
            &format!(
                "select a.ID, b.W from {A}.T_JOIN_A a left join {B}.T_JOIN_B b on a.ID = b.ID order by a.ID"
            ),
            (),
        )
        .await
        .expect("跨模式左连接失败");
    assert_eq!(rs.len(), 2);
    assert_eq!(
        rs.rows()[1].get::<Option<String>>(1).unwrap(),
        None,
        "左连接缺失的一侧应当是 NULL"
    );

    // 跨模式写：在 DBTESTA 的会话里更新 DBTESTB 的行。
    let affected = conn
        .execute(
            &format!("update {B}.T_JOIN_B set W = ? where ID = ?"),
            ("壹", 1i32),
        )
        .await
        .expect("跨模式更新失败");
    assert_eq!(affected, 1);
    let w = text_of(
        &mut conn,
        &format!("select W from {B}.T_JOIN_B where ID = 1"),
    )
    .await;
    assert_eq!(w, "壹");

    for sql in [
        format!("drop table {A}.T_JOIN_A"),
        format!("drop table {B}.T_JOIN_B"),
    ] {
        exec(&mut conn, &sql).await;
    }
    drop(conn);
    close(pool).await;
}

/// 事务与保存点：回滚到保存点只撤销其后的语句，提交后另一个连接能看到结果。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtesta_transaction_and_savepoint() {
    let Some(pool) = pool_at(A, 2).await else {
        return;
    };

    {
        let mut conn = conn_of(&pool).await;
        exec(&mut conn, "drop table if exists T_TXN").await;
        exec(
            &mut conn,
            "create table T_TXN (ID INT PRIMARY KEY, NAME VARCHAR(16))",
        )
        .await;
    }

    {
        // `PooledConn::begin` 是 `self` 收尾（事务持有这条连接），所以这里不需要 `mut`。
        let conn = conn_of(&pool).await;
        let mut txn = conn.begin().await.expect("开事务失败");
        txn.execute("insert into T_TXN (ID, NAME) values (?, ?)", (1i32, "一"))
            .await
            .expect("事务内插入 1 失败");
        txn.savepoint("sp1").await.expect("建保存点失败");
        txn.execute("insert into T_TXN (ID, NAME) values (?, ?)", (2i32, "二"))
            .await
            .expect("事务内插入 2 失败");
        txn.rollback_to("sp1").await.expect("回滚到保存点失败");
        txn.execute("insert into T_TXN (ID, NAME) values (?, ?)", (3i32, "三"))
            .await
            .expect("事务内插入 3 失败");
        txn.commit().await.expect("提交失败");
    }

    // 另开一个连接验证：1 和 3 在、2 不在。
    {
        let mut conn = conn_of(&pool).await;
        assert_eq!(count(&mut conn, "select count(*) from T_TXN").await, 2);
        assert_eq!(
            count(&mut conn, "select count(*) from T_TXN where ID = 2").await,
            0,
            "回滚到保存点后，保存点之后的插入必须消失"
        );
        assert_eq!(
            count(&mut conn, "select count(*) from T_TXN where ID in (1, 3)").await,
            2
        );
    }

    // 整体回滚：一条都不该留下。`begin` 收走连接，所以单独借一条。
    {
        let conn = conn_of(&pool).await;
        let mut txn = conn.begin().await.expect("开事务失败");
        txn.execute("insert into T_TXN (ID, NAME) values (?, ?)", (4i32, "四"))
            .await
            .expect("插入失败");
        txn.rollback().await.expect("回滚失败");
    }

    {
        let mut conn = conn_of(&pool).await;
        assert_eq!(
            count(&mut conn, "select count(*) from T_TXN").await,
            2,
            "整体回滚后行数必须回到 2"
        );
        exec(&mut conn, "drop table T_TXN").await;
    }

    close(pool).await;
}

/// 批量写：500 行一次提交，影响行数、行数、求和都对得上（批处理路径不分模式）。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtestb_batch_insert() {
    let Some(pool) = pool_at(B, 2).await else {
        return;
    };

    let mut conn = conn_of(&pool).await;
    exec(&mut conn, "drop table if exists T_BATCH").await;
    exec(
        &mut conn,
        "create table T_BATCH (ID INT PRIMARY KEY, NAME VARCHAR(32), N BIGINT)",
    )
    .await;

    let rows: Vec<(i32, String, i64)> = (0..500).map(|i| (i, format!("行{i}"), i as i64)).collect();
    let outcome = conn
        .execute_many(
            "insert into T_BATCH (ID, NAME, N) values (?, ?, ?)",
            rows.into_iter(),
        )
        .await
        .expect("批量插入失败");
    assert_eq!(outcome.affected, 500, "批量写的影响行数应当累计 500");
    assert_eq!(outcome.failed_at, None, "不应有失败的分组");

    assert_eq!(count(&mut conn, "select count(*) from T_BATCH").await, 500);
    assert_eq!(
        count(&mut conn, "select sum(N) from T_BATCH").await,
        124_750,
        "0..499 的和必须是 124750"
    );

    exec(&mut conn, "drop table T_BATCH").await;
    drop(conn);
    close(pool).await;
}

/// LOB：1.2 MB 的 CLOB 分块写入 + 分块读回，256 KiB 的 BLOB 整值往返。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtesta_lob_write_and_stream_read() {
    let Some(pool) = pool_at(A, 2).await else {
        return;
    };

    let text = "达梦".repeat(200_000); // 1.2 MB（UTF-8）
    let mut state = 0x1234_5678u32;
    let blob: Vec<u8> = (0..256 * 1024)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect();

    {
        let mut conn = conn_of(&pool).await;
        exec(&mut conn, "drop table if exists T_LOB").await;
        exec(
            &mut conn,
            "create table T_LOB (ID INT PRIMARY KEY, BODY CLOB, BIN BLOB)",
        )
        .await;

        let affected = conn
            .execute_lob(
                "insert into T_LOB (ID, BODY) values (?, ?)",
                (1i32,),
                LobParam::new(1, LobKind::Text, LobBytes::new(text.as_bytes().to_vec())),
            )
            .await
            .expect("分块写 CLOB 失败");
        assert_eq!(affected, 1);

        let affected = conn
            .execute_lob(
                "insert into T_LOB (ID, BIN) values (?, ?)",
                (2i32,),
                LobParam::new(1, LobKind::Binary, LobBytes::new(blob.clone())),
            )
            .await
            .expect("分块写 BLOB 失败");
        assert_eq!(affected, 1);
    }

    // CLOB：分块读（不会把 1.2 MB 一次拿到内存里）。
    {
        let conn = conn_of(&pool).await;
        let mut stream = conn
            .lob_stream(
                "select BODY from T_LOB where ID = ?",
                (1i32,),
                LobRef::name("BODY"),
            )
            .await
            .expect("开分块读失败");
        assert_eq!(stream.column().name, "BODY");
        assert!(stream.next_row().await.expect("定位第一行失败"));
        let mut value = Vec::new();
        let mut chunks = 0usize;
        loop {
            let state = stream.read_chunk(&mut value).await.expect("分块读失败");
            chunks += 1;
            if state.is_done() {
                break;
            }
        }
        assert!(
            value == text.as_bytes(),
            "CLOB 分块读的内容必须逐字节一致（原文 {} 字节，回读 {} 字节）",
            text.len(),
            value.len()
        );
        assert_eq!(stream.bytes_read(), text.len());
        println!("\n[LOB] CLOB {} 字节分 {} 趟读回一致", value.len(), chunks);
    }

    // BLOB：整值往返。
    {
        let mut conn = conn_of(&pool).await;
        let row = conn
            .query_one("select BIN from T_LOB where ID = ?", (2i32,))
            .await
            .expect("查询失败")
            .expect("必须有一行");
        let back = row.get::<Vec<u8>>(0).expect("BIN 应当是二进制");
        assert_eq!(back.len(), blob.len());
        assert!(back == blob, "BLOB 内容必须逐字节一致");
        println!("[LOB] BLOB {} 字节往返一致", back.len());

        exec(&mut conn, "drop table T_LOB").await;
    }

    close(pool).await;
}

/// 并发：4 个连接上限、8 个任务各写 25 行，最终行数与任务数都对。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtestb_pool_concurrency() {
    const TASKS: i32 = 8;
    const PER_TASK: i32 = 25;

    let Some(pool) = pool_at(B, 4).await else {
        return;
    };

    {
        let mut conn = conn_of(&pool).await;
        exec(&mut conn, "drop table if exists T_CONC").await;
        exec(
            &mut conn,
            "create table T_CONC (ID INT PRIMARY KEY, WORKER INT NOT NULL, N INT NOT NULL)",
        )
        .await;
    }

    let mut tasks = Vec::new();
    for worker in 0..TASKS {
        let pool = Arc::clone(&pool);
        tasks.push(tokio::spawn(async move {
            let conn = conn_of(&pool).await;
            let mut txn = conn.begin().await.expect("开事务失败");
            for i in 0..PER_TASK {
                txn.execute(
                    "insert into T_CONC (ID, WORKER, N) values (?, ?, ?)",
                    (worker * PER_TASK + i, worker, i),
                )
                .await
                .unwrap_or_else(|err| panic!("worker {worker} 第 {i} 行插入失败：{err:?}"));
            }
            txn.commit().await.expect("提交失败");
        }));
    }
    for (i, task) in tasks.into_iter().enumerate() {
        task.await
            .unwrap_or_else(|err| panic!("任务 {i} 崩了：{err:?}"));
    }

    {
        let mut conn = conn_of(&pool).await;
        assert_eq!(
            count(&mut conn, "select count(*) from T_CONC").await,
            i64::from(TASKS * PER_TASK),
            "并发写必须一行不少"
        );
        assert_eq!(
            count(&mut conn, "select count(distinct WORKER) from T_CONC").await,
            i64::from(TASKS),
            "每个任务都要有落库的行"
        );
        exec(&mut conn, "drop table T_CONC").await;
    }

    close(pool).await;
}

/// 错误映射：语法错、未知表、未知列、主键冲突都要以 `Err` 返回并带上服务端错误码。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtesta_error_mapping() {
    let Some(pool) = pool_at(A, 2).await else {
        return;
    };

    let mut conn = conn_of(&pool).await;
    exec(&mut conn, "drop table if exists T_ERR").await;
    exec(&mut conn, "create table T_ERR (ID INT PRIMARY KEY)").await;
    conn.execute("insert into T_ERR (ID) values (?)", (1i32,))
        .await
        .expect("插入失败");

    let err = conn
        .query("select * from", ())
        .await
        .expect_err("语法错误必须返回 Err");
    println!("\n[错误映射] 语法错误：{err:?}");

    let err = conn
        .query_one("select count(*) from T_ERR_NO_SUCH_TABLE", ())
        .await
        .expect_err("未知表必须返回 Err");
    println!("[错误映射] 未知表：{err:?}");

    let err = conn
        .query_one("select NO_SUCH_COLUMN from T_ERR", ())
        .await
        .expect_err("未知列必须返回 Err");
    println!("[错误映射] 未知列：{err:?}");

    let err = conn
        .execute("insert into T_ERR (ID) values (?)", (1i32,))
        .await
        .expect_err("主键冲突必须返回 Err");
    println!("[错误映射] 主键冲突：{err:?}");

    // 出错之后连接仍可继续使用（语句级错误不污染连接）。
    assert_eq!(count(&mut conn, "select count(*) from T_ERR").await, 1);

    exec(&mut conn, "drop table T_ERR").await;
    drop(conn);
    close(pool).await;
}

/// DDL 生命周期：建表 / 加列 / 建索引 / 删索引 / 清空 / 删表，结束时对象确实不存在。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtestb_ddl_lifecycle() {
    let Some(pool) = pool_at(B, 2).await else {
        return;
    };

    let mut conn = conn_of(&pool).await;
    exec(&mut conn, "drop table if exists T_DDL").await;
    exec(
        &mut conn,
        "create table T_DDL (ID INT PRIMARY KEY, NAME VARCHAR(32))",
    )
    .await;
    exec(&mut conn, "alter table T_DDL add COL2 INT DEFAULT 0").await;
    exec(&mut conn, "create index IDX_DDL_NAME on T_DDL (NAME)").await;

    conn.execute("insert into T_DDL (ID, NAME) values (?, ?)", (1i32, "甲"))
        .await
        .expect("插入失败");
    let row = conn
        .query_one("select COL2 from T_DDL where ID = ?", (1i32,))
        .await
        .expect("查询失败")
        .expect("必须有一行");
    assert_eq!(row.get::<i32>(0).unwrap(), 0, "ALTER ADD 的默认值要生效");

    exec(&mut conn, "drop index IDX_DDL_NAME").await;
    exec(&mut conn, "truncate table T_DDL").await;
    assert_eq!(count(&mut conn, "select count(*) from T_DDL").await, 0);
    exec(&mut conn, "drop table T_DDL").await;

    let left = count(
        &mut conn,
        "select count(*) from SYS.ALL_TABLES where table_name = 'T_DDL'",
    )
    .await;
    assert_eq!(left, 0, "DROP TABLE 之后对象必须消失");

    drop(conn);
    close(pool).await;
}

// ---------------------------------------------------------------------------
// 默认模式的几条「路径」：`DmConfig::schema`、老写法 `init_sql`、`charset_check=off`、小写模式名
// ---------------------------------------------------------------------------

/// `DmConfig::schema` 与老写法 `init_sql = ["SET SCHEMA …"]` 必须等价——
/// 这条锁住「重写建连逻辑没改变语义」，也说明两种写法可以互换。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtesta_config_schema_matches_init_sql() {
    let Some(from_config) = config_at(A, 1) else {
        return;
    };
    let Some(from_init) = config_via_init_sql(A, 1) else {
        return;
    };

    let mut seen = Vec::new();
    for (label, cfg) in [("schema", from_config), ("init_sql", from_init)] {
        let pool = Arc::new(
            DmPool::connect_with(cfg, ExecutorConfig::for_pool(1))
                .await
                .unwrap_or_else(|err| panic!("{label} 建池失败：{err:?}")),
        );
        let mut conn = conn_of(&pool).await;
        let current = text_of(
            &mut conn,
            "select SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from DUAL",
        )
        .await;
        seen.push((label, current));
        drop(conn);
        close(pool).await;
    }

    assert_eq!(seen[0].1, A, "DmConfig::schema 应当让默认模式变成 {A}");
    assert_eq!(seen[1].1, A, "init_sql 写法的默认模式也应当是 {A}（对照）");
}

/// `charset_check = off` 时仍然要能切默认模式：关掉字符集自检不等于忽略模式配置。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtestb_schema_survives_charset_check_off() {
    let Some(mut cfg) = dsn().map(|d| cfg_from_dsn(&d, B, 1)) else {
        return;
    };
    cfg.charset_check = CharsetCheck::Off;

    let pool = Arc::new(
        DmPool::connect_with(cfg, ExecutorConfig::for_pool(1))
            .await
            .expect("charset_check=off 建池失败"),
    );
    let mut conn = conn_of(&pool).await;
    let current = text_of(
        &mut conn,
        "select SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from DUAL",
    )
    .await;
    assert_eq!(current, B, "charset_check=off 时 {B} 仍须生效");

    // 未限定 DDL 落到 DBTESTB。
    exec(&mut conn, "drop table if exists T_OFF_PROBE").await;
    exec(&mut conn, "create table T_OFF_PROBE (ID INT PRIMARY KEY)").await;
    let owner = text_of(
        &mut conn,
        "select owner from SYS.ALL_TABLES where table_name = 'T_OFF_PROBE'",
    )
    .await;
    assert_eq!(owner, B, "未限定 CREATE TABLE 必须落在 {B}");
    exec(&mut conn, "drop table T_OFF_PROBE").await;

    drop(conn);
    close(pool).await;
}

/// 小写模式名：达梦把未加引号的标识符折成大写，`SET SCHEMA dbtest_lower` 会报
/// `-2103 无效的模式名`；中间件生成的语句一律带双引号，所以这条能连上且模式正确。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn dbtest_lower_schema_requires_quoting() {
    const LOWER: &str = "dbtest_lower";

    let Some(pool_admin) = pool_at(A, 1).await else {
        return;
    };
    {
        let mut conn = conn_of(&pool_admin).await;
        // 幂等：清掉上次可能的残留。**先删模式**——模式不存在时
        // `drop table if exists "模式"."表"` 仍会报 `无效的表或视图名`。
        try_exec(&mut conn, "drop schema if exists \"dbtest_lower\" restrict").await;
        try_exec(&mut conn, "drop table if exists \"dbtest_lower\".T_LOWER").await;
        exec(
            &mut conn,
            "create schema \"dbtest_lower\" authorization SYSDBA",
        )
        .await;
    }

    let Some(cfg) = dsn().map(|d| cfg_from_dsn(&d, LOWER, 1)) else {
        return;
    };
    let pool_lower = Arc::new(
        DmPool::connect_with(cfg, ExecutorConfig::for_pool(1))
            .await
            .expect("小写模式建池失败：SET SCHEMA 是否没加引号？"),
    );
    {
        let mut conn = conn_of(&pool_lower).await;
        let current = text_of(
            &mut conn,
            "select SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from DUAL",
        )
        .await;
        assert_eq!(current, LOWER, "当前模式应当逐字等于 {LOWER}");

        exec(&mut conn, "drop table if exists T_LOWER").await;
        exec(&mut conn, "create table T_LOWER (ID INT PRIMARY KEY)").await;
        exec(&mut conn, "insert into T_LOWER (ID) values (1)").await;
        assert_eq!(count(&mut conn, "select count(*) from T_LOWER").await, 1);

        let owner = text_of(
            &mut conn,
            "select owner from SYS.ALL_TABLES where table_name = 'T_LOWER'",
        )
        .await;
        assert_eq!(
            owner.to_lowercase(),
            LOWER,
            "未限定建表应落在小写模式里（owner 实际为 {owner}）"
        );
        exec(&mut conn, "drop table T_LOWER").await;
    }
    close(pool_lower).await;

    // 收尾：删掉小写模式（此时没有任何会话以它为默认模式）。
    {
        let mut conn = conn_of(&pool_admin).await;
        exec(&mut conn, "drop schema \"dbtest_lower\" restrict").await;
    }
    close(pool_admin).await;
}

// ---------------------------------------------------------------------------
// 完全限定名（`"模式名"."表名"`）：用户既有的写法在中间件里能不能直接用
// ---------------------------------------------------------------------------

const CURRENT_SCHEMA_SQL: &str = "select SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from DUAL";

/// 完全限定名（`"模式名"."表名"`）不依赖会话默认模式：一条连接同时读写两个模式。
///
/// 这是「同一连接里频繁跨模式读写」的推荐姿势——限定名是**语句级**信息，不改会话状态；
/// 而 `SET SCHEMA` 是**会话级**的，会留在池里的物理连接上
/// （见 `set_schema_on_a_pooled_conn_leaks_into_the_pool`）。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn qualified_names_work_without_any_default_schema() {
    let Some(cfg) = dsn().map(|d| {
        let mut cfg = DmConfig::from_connection_string(&d).expect("DM_TEST_DSN 不是合法的连接串");
        cfg.pool = PoolConfig {
            max_size: 1,
            min_idle: 0,
            ..PoolConfig::default()
        };
        cfg
    }) else {
        return;
    };
    assert!(cfg.schema.is_none(), "本用例刻意不配 schema");

    let pool = Arc::new(
        DmPool::connect_with(cfg, ExecutorConfig::for_pool(1))
            .await
            .expect("建池失败（达梦是否可达？）"),
    );
    let mut conn = conn_of(&pool).await;

    let current = text_of(&mut conn, CURRENT_SCHEMA_SQL).await;
    println!("\n[限定名] 没配 schema 时的会话默认模式：{current}");
    assert_eq!(current, "SYSDBA", "不配 schema 就不该动会话默认模式");

    // 两个模式里建同名表，DDL 全走限定名。
    for s in [A, B] {
        exec(&mut conn, &format!("drop table if exists \"{s}\".T_Q_A")).await;
        exec(
            &mut conn,
            &format!("create table \"{s}\".T_Q_A (ID INT PRIMARY KEY, TAG VARCHAR(32))"),
        )
        .await;
    }

    // 交替写：同一连接、未切换任何会话状态。
    conn.execute(
        "insert into \"DBTESTA\".T_Q_A (ID, TAG) values (?, ?)",
        (1i32, "a1"),
    )
    .await
    .expect("写 DBTESTA 失败");
    conn.execute(
        "insert into \"DBTESTB\".T_Q_A (ID, TAG) values (?, ?)",
        (1i32, "b1"),
    )
    .await
    .expect("写 DBTESTB 失败");
    conn.execute(
        "insert into \"DBTESTA\".T_Q_A (ID, TAG) values (?, ?)",
        (2i32, "a2"),
    )
    .await
    .expect("写 DBTESTA 失败");

    // 跨模式 JOIN（两个模式各一行同 ID）→ 命中 1 行。
    let joined = count(
        &mut conn,
        "select count(*) from \"DBTESTA\".T_Q_A a join \"DBTESTB\".T_Q_A b on a.ID = b.ID",
    )
    .await;
    assert_eq!(joined, 1, "跨模式 JOIN 应当命中 1 行（ID=1）");

    // 限定名 UPDATE / SELECT 各打一个模式。
    let affected = conn
        .execute(
            "update \"DBTESTB\".T_Q_A set TAG = ? where ID = ?",
            ("b1-updated", 1i32),
        )
        .await
        .expect("限定名 UPDATE 失败");
    assert_eq!(affected, 1);
    let tag = text_of(&mut conn, "select TAG from \"DBTESTB\".T_Q_A where ID = 1").await;
    assert_eq!(tag, "b1-updated");
    assert_eq!(
        count(&mut conn, "select count(*) from \"DBTESTA\".T_Q_A").await,
        2
    );
    assert_eq!(
        count(&mut conn, "select count(*) from \"DBTESTB\".T_Q_A").await,
        1
    );

    // 对照：不加限定名就找不到表（证明上面的成功靠的是限定名本身）。
    let err = conn
        .query_one("select count(*) from T_Q_A", ())
        .await
        .expect_err("未限定名不应解析到任何表");
    println!("[限定名] 未限定名 T_Q_A 的错误：{err:?}");

    for s in [A, B] {
        exec(&mut conn, &format!("drop table \"{s}\".T_Q_A")).await;
    }
    drop(conn);
    close(pool).await;
}

/// 一条池化连接上反复交替读写两个模式：不切会话模式、不借第二条连接。
///
/// 顺带打印吞吐，用来回答「频繁跨模式读写会不会因为切模式而变慢」。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn qualified_names_alternate_freely_on_one_pooled_conn() {
    const ROUNDS: i32 = 100;

    let Some(pool) = pool_at(A, 1).await else {
        return;
    };
    let mut conn = conn_of(&pool).await;

    for s in [A, B] {
        exec(&mut conn, &format!("drop table if exists \"{s}\".T_Q_ALT")).await;
        exec(
            &mut conn,
            &format!("create table \"{s}\".T_Q_ALT (ID INT PRIMARY KEY, SIDE CHAR(1))"),
        )
        .await;
    }

    let started = std::time::Instant::now();
    let mut statements = 0usize;
    for i in 1..=ROUNDS {
        conn.execute(
            "insert into \"DBTESTA\".T_Q_ALT (ID, SIDE) values (?, ?)",
            (i, "A"),
        )
        .await
        .unwrap_or_else(|err| panic!("第 {i} 轮写 {A} 失败：{err:?}"));
        conn.execute(
            "insert into \"DBTESTB\".T_Q_ALT (ID, SIDE) values (?, ?)",
            (i, "B"),
        )
        .await
        .unwrap_or_else(|err| panic!("第 {i} 轮写 {B} 失败：{err:?}"));
        statements += 2;
        if i % 10 == 0 {
            let n = count(
                &mut conn,
                "select count(*) from \"DBTESTA\".T_Q_ALT a join \"DBTESTB\".T_Q_ALT b on a.ID = b.ID",
            )
            .await;
            assert_eq!(n, i as i64, "第 {i} 轮 JOIN 行数不对");
            statements += 1;
        }
    }
    let elapsed = started.elapsed();

    let a_rows = count(&mut conn, "select count(*) from \"DBTESTA\".T_Q_ALT").await;
    let b_rows = count(&mut conn, "select count(*) from \"DBTESTB\".T_Q_ALT").await;
    assert_eq!(a_rows, ROUNDS as i64);
    assert_eq!(b_rows, ROUNDS as i64);

    // 全程没有动过会话默认模式：仍然停在 pool 配的 DBTESTA。
    let current = text_of(&mut conn, CURRENT_SCHEMA_SQL).await;
    assert_eq!(current, A, "限定名读写不该改变会话默认模式");

    println!(
        "\n[限定名交替] {statements} 条语句 / {elapsed:?}，平均 {:.0} µs/条（会话模式全程 = {current}）",
        elapsed.as_micros() as f64 / statements as f64
    );

    for s in [A, B] {
        exec(&mut conn, &format!("drop table \"{s}\".T_Q_ALT")).await;
    }
    drop(conn);
    close(pool).await;
}

/// 记录一个坑：在池化连接上手动 `SET SCHEMA`，归还后**不会**被复位。
///
/// 池的 `reset()` 契约（`crates/dm-core/src/backend.rs:748`）只做「关游标 → 回滚 →
/// 还原 autocommit → 清缓存」，达梦侧实现（`crates/dm-odbc/src/lib.rs:353`）就是
/// `rollback()` + `set_autocommit(true)`，**没有** `SET SCHEMA` 复位。所以会话模式会跟着
/// 物理连接留在池里，下一位借用者悄悄拿到别的默认模式。
///
/// 本用例只做观测：若池没有复用同一条物理连接，则本条观测不成立，不算失败。
#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN"]
async fn set_schema_on_a_pooled_conn_leaks_into_the_pool() {
    let Some(pool) = pool_at(A, 1).await else {
        return;
    };

    {
        let mut conn = conn_of(&pool).await;
        let before = text_of(&mut conn, CURRENT_SCHEMA_SQL).await;
        assert_eq!(before, A, "池里的连接应当以 {A} 为默认模式");
        exec(&mut conn, "SET SCHEMA \"DBTESTB\"").await;
        let inside = text_of(&mut conn, CURRENT_SCHEMA_SQL).await;
        assert_eq!(inside, B, "SET SCHEMA 应当立即生效");
        // 连接在这里回池（PooledConn 的 Drop），期间只走 reset()。
    }

    let mut again = conn_of(&pool).await;
    let after = text_of(&mut again, CURRENT_SCHEMA_SQL).await;
    println!("\n[模式残留] 归还前 = {B}，重借后 = {after}");
    if after == B {
        println!("=> 确认：手动 SET SCHEMA 会随物理连接留在池里，下一位借用者拿到的是 {B}");
        exec(&mut again, &format!("SET SCHEMA \"{A}\"")).await;
    } else {
        println!("=> 池没有复用同一条物理连接（现在是 {after}），本条观测不成立（不算失败）");
    }
    drop(again);
    close(pool).await;
}
