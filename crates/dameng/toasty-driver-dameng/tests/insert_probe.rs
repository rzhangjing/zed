//! 临时探针（T8.1b）：达梦 8 对 `INSERT` 里 `DEFAULT` / `IDENTITY` 列的接受面。
//!
//! 背景：T8.1a 真机首轮失败在
//! `INSERT INTO "probe_widgets" ("id", …) VALUES (DEFAULT, …)`
//! ⇒ 达梦报 `-2871 DEFAULT不允许作为显式标识值`。fork 的达梦分支要决定
//! 「自增列该整列省略、还是写 NULL」，这里用真机把候选写法逐个打一遍，**不猜**。
//!
//! 跑法：
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc test -p toasty-driver-dameng --test insert_probe \
//!     -- --include-ignored --test-threads=1 --nocapture
//! ```
//!
//! 探针只建/删自己的表（`dm_insert_probe`），失败不 panic（除了连不上库），
//! 结论看打印的 OK/FAIL 行。

use dm_core::DmConfig;
use dm_query::PooledConn;

const TABLE: &str = "\"dm_insert_probe\"";

async fn conn() -> (dm_pool::DmPool, PooledConn) {
    let dsn = std::env::var("DM_TEST_DSN").expect("需要 DM_TEST_DSN");
    let cfg = DmConfig::from_connection_string(&dsn).expect("DSN 解析失败");
    let pool: dm_pool::DmPool = dm_pool::DmPool::connect(cfg).await.expect("连不上达梦");
    let conn = PooledConn::acquire(&pool).await.expect("借连接失败");
    (pool, conn)
}

async fn probe(conn: &mut PooledConn, label: &str, sql: &str, args: impl dm_query::IntoArgs) {
    match conn.execute(sql, args).await {
        Ok(n) => println!("OK   {label:<28} 影响 {n} 行 :: {sql}"),
        Err(e) => println!("FAIL {label:<28} {e} :: {sql}"),
    }
}

#[tokio::test]
#[ignore = "真机探针：需要 DM_TEST_DSN"]
async fn probe_insert_default_and_identity() {
    let (_pool, mut conn) = conn().await;

    let _ = conn
        .execute(&format!("DROP TABLE {TABLE} CASCADE CONSTRAINTS"), ())
        .await;

    // id 自增主键；name 必填；flag 有列默认值。
    conn.execute(
        &format!(
            "CREATE TABLE {TABLE} (\"id\" BIGINT IDENTITY(1,1) NOT NULL, \
             \"name\" VARCHAR(20) NOT NULL, \"flag\" INT DEFAULT 7, PRIMARY KEY(\"id\"))"
        ),
        (),
    )
    .await
    .expect("建探针表失败");
    println!("建表 OK：{TABLE}");

    // (a) 自增列写 DEFAULT —— T8.1a 撞的那句
    probe(
        &mut conn,
        "a. id=DEFAULT",
        &format!("INSERT INTO {TABLE} (\"id\", \"name\") VALUES (DEFAULT, ?)"),
        vec![dm_core::DmValue::Text("a".into())],
    )
    .await;

    // (b) 整列省略
    probe(
        &mut conn,
        "b. 省略 id 列",
        &format!("INSERT INTO {TABLE} (\"name\") VALUES (?)"),
        vec![dm_core::DmValue::Text("b".into())],
    )
    .await;

    // (c) 自增列写 NULL
    probe(
        &mut conn,
        "c. id=NULL",
        &format!("INSERT INTO {TABLE} (\"id\", \"name\") VALUES (NULL, ?)"),
        vec![dm_core::DmValue::Text("c".into())],
    )
    .await;

    // (d) 普通列的列默认值（`DEFAULT` 在**非**自增列上是否被接受）
    probe(
        &mut conn,
        "d. flag=DEFAULT",
        &format!("INSERT INTO {TABLE} (\"name\", \"flag\") VALUES (?, DEFAULT)"),
        vec![dm_core::DmValue::Text("d".into())],
    )
    .await;

    // (e) 只有自增列时：DEFAULT VALUES
    let _ = conn.execute(&format!("DROP TABLE {TABLE}"), ()).await;
    conn.execute(
        &format!(
            "CREATE TABLE {TABLE} (\"id\" BIGINT IDENTITY(1,1) NOT NULL, PRIMARY KEY(\"id\"))"
        ),
        (),
    )
    .await
    .expect("建只含自增列的表失败");
    probe(
        &mut conn,
        "e. DEFAULT VALUES",
        &format!("INSERT INTO {TABLE} DEFAULT VALUES"),
        (),
    )
    .await;
    probe(
        &mut conn,
        "e2. (id) VALUES(DEFAULT)",
        &format!("INSERT INTO {TABLE} (\"id\") VALUES (DEFAULT)"),
        (),
    )
    .await;

    // 收尾 + 看自增回填
    let rs = conn
        .query(
            &format!("SELECT \"id\", \"name\" FROM {TABLE} ORDER BY \"id\""),
            (),
        )
        .await
        .expect("回读失败");
    for row in rs.rows() {
        let id = row.get::<i64>(0).unwrap_or_default();
        let name = row.get::<String>(1).unwrap_or_else(|_| "<非文本>".into());
        println!("   行：id={id} name={name}");
    }

    let _ = conn
        .execute(&format!("DROP TABLE {TABLE} CASCADE CONSTRAINTS"), ())
        .await;
    println!("探针表已删");
}
