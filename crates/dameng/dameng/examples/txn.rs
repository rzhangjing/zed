//! 事务示例：`begin`/`commit`/`rollback`、保存点、以及**未提交就被丢弃的事务自动回滚**。
//!
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc run -p dameng --example txn
//! ```
//!
//! 四条值得注意的语义（都由真机用例钉住，见 `docs/设计方案.md` §6.6、§12.2「G5 出口记录」）：
//!
//! 1. `Transaction` **吃掉连接**：`conn.begin()` 按值拿走 `PooledConn`，返回的 `Transaction`
//!    持有连接所有权——于是「事务之外还用这条连接跑语句」在类型层面就不成立。想跑多条
//!    互相独立的事务，就每条各自 `PooledConn::acquire(&pool)` 一次；
//! 2. 未提交的 `Transaction` 被 `Drop` → 连接标脏并由池**回滚**（不会带着半开事务回池）；
//! 3. 达梦**没有** `RELEASE SAVEPOINT` 语法：本中间件照发该语句，但把语法拒绝翻成
//!    `DmErrorKind::Unsupported`（原始诊断保留），所以调用方不必去猜 `-2007` 是不是自己写错了；
//! 4. 池只有 1 条连接时，**事务期间**不能再借连接（会一直排队）——本示例因此只在事务
//!    之外调 [`count`]，用完即还。

use dameng::prelude::*;

/// 示例表：名字带前缀，避免与别的示例/测试撞名。
const T_TXN: &str = "DM_EXAMPLE_TXN";

fn config() -> Option<DmConfig> {
    if let Ok(dsn) = std::env::var("DM_TEST_DSN")
        && !dsn.trim().is_empty()
    {
        return Some(DmConfig::from_connection_string(&dsn).expect("DM_TEST_DSN 不是合法的连接串"));
    }
    if std::env::var("DM_SERVER").is_ok() {
        return Some(DmConfig::from_env().expect("DM_SERVER 环境下无法构造 DmConfig"));
    }
    None
}

/// `select count(*)`：借一条连接数一遍行数，数完立刻还。
///
/// 只能在**没有事务开着**的时候调用：池只有 1 条连接，事务期间它被别人按着，
/// 这个函数会一直排队（而事务正等着本函数返回），所以那是自死锁而不是等待。
async fn count(pool: &Pool) -> Result<i64, DmError> {
    let mut conn = PooledConn::acquire(pool).await?;
    let row = conn
        .query_one(&format!("select count(*) as N from {T_TXN}"), ())
        .await?
        .expect("count(*) 必定返回一行");
    row.get::<i64>("N")
}

// 当前线程运行时足够（阻塞工作都在 `dm-pool` 的阻塞线程池上）。
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), DmError> {
    let Some(cfg) = config() else {
        println!("跳过：未配置 DM_TEST_DSN / DM_SERVER（本示例需要一台可达的达梦 DM8）");
        return Ok(());
    };

    // 一条连接的池就够：事务语义全都发生在同一条物理连接上。
    //
    // ⚠️ 必须同时设 `cfg.pool.max_size`：`ExecutorConfig::for_pool(1)` 只配**阻塞线程池**的
    // 线程数，池的连接上限在 `DmConfig::pool.max_size`（默认 10）。只传后者的话
    // `count()` 会借到**另一条**连接，于是「事务期间池里没有第二条连接」这个前提不成立。
    let mut cfg = cfg;
    cfg.pool.max_size = 1;
    cfg.pool.min_idle = 0;
    let pool = Pool::connect_with(cfg, ExecutorConfig::for_pool(1)).await?;

    // 建表用一次短借用，建完就还（`conn` 在块结束时 drop）。
    {
        let mut conn = PooledConn::acquire(&pool).await?;
        conn.execute(&format!("drop table if exists {T_TXN}"), ())
            .await?;
        conn.execute(
            &format!("create table {T_TXN} (ID INT PRIMARY KEY, NOTE VARCHAR(50))"),
            (),
        )
        .await?;
    }
    println!("已建表 {T_TXN}");

    // ---- 1) 回滚：写进去的行不该存在 --------------------------------------
    {
        // `begin()` 按值吃掉连接；块结束时 `txn` 已消费掉，连接自动回池。
        let mut txn = PooledConn::acquire(&pool).await?.begin().await?;
        txn.execute(
            &format!("insert into {T_TXN} (ID, NOTE) values (?, ?)"),
            (1_i32, "会被回滚"),
        )
        .await?;
        txn.rollback().await?;
    }
    println!("回滚后行数 = {}（应为 0）", count(&pool).await?);

    // ---- 2) 提交：写进去的行应当存在 --------------------------------------
    {
        let mut txn = PooledConn::acquire(&pool).await?.begin().await?;
        txn.execute(
            &format!("insert into {T_TXN} (ID, NOTE) values (?, ?)"),
            (2_i32, "会被提交"),
        )
        .await?;
        txn.commit().await?;
    }
    println!("提交后行数 = {}（应为 1）", count(&pool).await?);

    // ---- 3) 保存点：只回滚保存点**之后**的影响 ---------------------------
    {
        let mut txn = PooledConn::acquire(&pool).await?.begin().await?;
        txn.execute(
            &format!("insert into {T_TXN} (ID, NOTE) values (?, ?)"),
            (3_i32, "保存点之前"),
        )
        .await?;
        txn.savepoint("sp1").await?;
        txn.execute(
            &format!("insert into {T_TXN} (ID, NOTE) values (?, ?)"),
            (4_i32, "保存点之后"),
        )
        .await?;
        txn.rollback_to("sp1").await?;
        txn.commit().await?;
    }
    println!(
        "回滚到保存点后行数 = {}（应为 2：ID 2 与 ID 3，ID 4 被撤销）",
        count(&pool).await?
    );

    // ---- 4) `RELEASE SAVEPOINT`：达梦不支持，中间件如实翻成 Unsupported ----
    {
        let mut txn = PooledConn::acquire(&pool).await?.begin().await?;
        txn.savepoint("sp2").await?;
        match txn.release_savepoint("sp2").await {
            Ok(()) => println!("`RELEASE SAVEPOINT` 成功（本机达梦支持该语法）"),
            Err(err) => match &err.kind {
                DmErrorKind::Unsupported { feature } => {
                    println!("`RELEASE SAVEPOINT` → Unsupported：{feature}");
                    println!(
                        "  原始诊断仍保留：{}",
                        err.diags().first().map_or_else(
                            || "（无）".to_owned(),
                            |diag| format!(
                                "SQLSTATE {} / 原生码 {} / {}",
                                diag.sqlstate_str(),
                                diag.native_code,
                                diag.message
                            )
                        )
                    );
                }
                other => println!("意外错误：{other:?}"),
            },
        }
        txn.commit().await?;
    }
    println!("RELEASE SAVEPOINT 一轮结束，行数 = {}", count(&pool).await?);

    // ---- 5) 未提交就丢弃：池会把这条连接回滚掉 ---------------------------
    {
        let mut txn = PooledConn::acquire(&pool).await?.begin().await?;
        txn.execute(
            &format!("insert into {T_TXN} (ID, NOTE) values (?, ?)"),
            (99_i32, "未提交就被丢弃"),
        )
        .await?;
        // 不 commit、不 rollback：`txn` 在这里 drop，连接被标脏，池复位时回滚。
    }
    // 再借一次连接（池只有 1 条，只能借到同一条），逼池先把脏连接复位完。
    println!(
        "丢弃未提交事务后行数 = {}（应为 2，没有 ID 99）",
        count(&pool).await?
    );

    // 收尾：清表（示例不该在库里留东西）→ 关池 → 等存活连接归零。
    {
        let mut conn = PooledConn::acquire(&pool).await?;
        conn.execute(&format!("drop table if exists {T_TXN}"), ())
            .await?;
    }
    pool.close().await?;
    pool.wait_closed().await;
    println!("完成。");
    Ok(())
}
