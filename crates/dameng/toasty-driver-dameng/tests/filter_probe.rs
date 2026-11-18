//! 临时探针（T8.1b）：达梦 8 里「裸布尔参数当过滤条件」怎么写才被接受。
//!
//! 背景：Toasty 引擎对「无过滤」的全表查询会发
//! `SELECT … FROM "t" WHERE ?`（绑一个布尔参数）。MySQL/PG/SQLite 接受这种写法，
//! 达梦报 `[HY000/-4104] 查询使用值表达式作为过滤条件`。
//! 这里把候选改写逐个打到真机上，**不猜**。
//!
//! 跑法：
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc test -p toasty-driver-dameng --test filter_probe \
//!     -- --include-ignored --test-threads=1 --nocapture
//! ```

use dm_core::{DmConfig, DmValue};
use dm_query::PooledConn;

async fn conn() -> (dm_pool::DmPool, PooledConn) {
    let dsn = std::env::var("DM_TEST_DSN").expect("需要 DM_TEST_DSN");
    let cfg = DmConfig::from_connection_string(&dsn).expect("DSN 解析失败");
    let pool: dm_pool::DmPool = dm_pool::DmPool::connect(cfg).await.expect("连不上达梦");
    let conn = PooledConn::acquire(&pool).await.expect("借连接失败");
    (pool, conn)
}

async fn probe(conn: &mut PooledConn, label: &str, sql: &str, args: impl dm_query::IntoArgs) {
    match conn.query(sql, args).await {
        Ok(rs) => println!("OK   {label:<34} 返回 {} 行 :: {sql}", rs.len()),
        Err(e) => println!("FAIL {label:<34} {e} :: {sql}"),
    }
}

#[tokio::test]
#[ignore = "真机探针：需要 DM_TEST_DSN"]
async fn probe_bare_boolean_parameter_as_filter() {
    let (_pool, mut conn) = conn().await;

    // 基线：不带参数
    probe(&mut conn, "0. 无 WHERE", "SELECT 1 FROM DUAL", ()).await;

    // 现状：裸参数（引擎现在发的形状）
    probe(
        &mut conn,
        "1. WHERE ?（Bool 参数）",
        "SELECT 1 FROM DUAL WHERE ?",
        vec![DmValue::Bool(true)],
    )
    .await;

    // 候选 A：与 TRUE 比较
    probe(
        &mut conn,
        "2. WHERE ? = TRUE",
        "SELECT 1 FROM DUAL WHERE ? = TRUE",
        vec![DmValue::Bool(true)],
    )
    .await;

    // 候选 B：与 1 比较（把布尔参数当数值）
    probe(
        &mut conn,
        "3. WHERE ? = 1",
        "SELECT 1 FROM DUAL WHERE ? = 1",
        vec![DmValue::Bool(true)],
    )
    .await;

    // 候选 C：显式 CAST 成 BOOLEAN
    probe(
        &mut conn,
        "4. WHERE CAST(? AS BOOLEAN) = TRUE",
        "SELECT 1 FROM DUAL WHERE CAST(? AS BOOLEAN) = TRUE",
        vec![DmValue::Bool(true)],
    )
    .await;

    // 候选 D：把布尔当整数传（TinyInt 1）
    probe(
        &mut conn,
        "5. WHERE ?（TinyInt 参数）",
        "SELECT 1 FROM DUAL WHERE ?",
        vec![DmValue::TinyInt(1)],
    )
    .await;

    // 候选 E：整数参数与 1 比较
    probe(
        &mut conn,
        "6. WHERE ? = 1（TinyInt 参数）",
        "SELECT 1 FROM DUAL WHERE ? = 1",
        vec![DmValue::TinyInt(1)],
    )
    .await;

    // 候选 F：CAST(? AS INT) = 1
    probe(
        &mut conn,
        "7. WHERE CAST(? AS INT) = 1",
        "SELECT 1 FROM DUAL WHERE CAST(? AS INT) = 1",
        vec![DmValue::Bool(true)],
    )
    .await;

    // 候选 G：`= 1` 里的 1 换成 `CAST('1' AS BOOLEAN)`？先看 TRUE 字面量本身
    probe(
        &mut conn,
        "8. WHERE TRUE",
        "SELECT 1 FROM DUAL WHERE TRUE",
        (),
    )
    .await;
    probe(
        &mut conn,
        "9. WHERE 1 = 1",
        "SELECT 1 FROM DUAL WHERE 1 = 1",
        (),
    )
    .await;
}
