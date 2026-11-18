//! 真机：**模型表名带模式前缀**（`#[table = "DBTESTA.T_QUALIFIED"]`）。
//!
//! Round 36 的支持点：Toasty 侧只有 `db::Table::name` 一个字符串，模型写
//! `#[table = "DBTESTA.T_QUALIFIED"]` 会把整串塞进去。达梦方言必须把它渲染成
//! `"DBTESTA"."T_QUALIFIED"`（两个标识符），否则 `"DBTESTA.T_QUALIFIED"` 会被当成
//! 一个「名字里带点」的表 ⇒ `[42S02/-2106] 无效的表或视图名`。
//!
//! 这个用例覆盖整条链路：`push_schema` 建表（DDL）→ 插入（DML）→ 唯一键/主键查询 →
//! 更新 → 删除（含 `SET IDENTITY_INSERT` 不参与，因为这里不走 upsert），并从
//! `ALL_TABLES` 直接确认表**落在 DBTESTA 而不是当前默认模式**。
//!
//! 跑法（与 `toasty_real.rs` 同一约定：没配 DSN 就跳过）：
//!
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc test -p toasty-driver-dameng --test qualified_model_real -- --include-ignored --test-threads=1 --nocapture
//! ```
use dm_core::{DmConfig, PoolConfig};
use dm_query::PooledConn;
use toasty_driver_dameng::DamengDriver;

/// 唯一一张带模式前缀的表。
#[derive(Debug, toasty::Model)]
#[table = "DBTESTA.T_QUALIFIED"]
struct QualifiedNote {
    #[key]
    #[auto]
    id: i64,

    title: String,

    note: Option<String>,
}

/// 故意写**小写**的模式与表名：达梦把不加引号的标识符折成大写，所以这里应当
/// 落到 `DBTEST_LOWER.T_LOWER` 上（用户手写 `dbtest_lower.t_lower` 的习惯）。
#[derive(Debug, toasty::Model)]
#[table = "dbtest_lower.T_LOWER"]
struct LowerNote {
    #[key]
    #[auto]
    id: i64,

    label: String,
}

/// 同一条连接里查一个标量（诊断/断言都用它）。
async fn scalar_i64(conn: &mut PooledConn, sql: &str) -> i64 {
    let rs = conn.query(sql, ()).await.expect("查询失败");
    rs.first()
        .expect("查询没有返回行")
        .get::<i64>(0)
        .expect("查询结果不是整数")
}

fn real_config() -> Option<DmConfig> {
    let dsn = std::env::var("DM_TEST_DSN")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let mut cfg = if let Some(dsn) = dsn {
        DmConfig::from_connection_string(&dsn).expect("DM_TEST_DSN 不是合法的连接串")
    } else if std::env::var("DM_SERVER").is_ok() {
        DmConfig::from_env().expect("DM_SERVER 环境下无法构造 DmConfig")
    } else {
        eprintln!("跳过：未配置 DM_TEST_DSN / DM_SERVER");
        return None;
    };
    cfg.pool = PoolConfig {
        max_size: 2,
        min_idle: 0,
        ..PoolConfig::default()
    };
    Some(cfg)
}

/// 清场：模式不存在就先建（`CREATE SCHEMA` 是幂等失败也无所谓），再删残留表。
async fn prepare(cfg: &DmConfig) {
    let pool: dm_pool::DmPool = dm_pool::DmPool::connect(cfg.clone())
        .await
        .expect("连不上达梦（清场）");
    let mut conn = PooledConn::acquire(&pool).await.expect("借连接失败");

    // 注意：模式名**不加引号**创建（达梦折成大写 `DBTEST_LOWER`）。加了引号会建出
    // 逐字小写的 `dbtest_lower`，那就只有 `"dbtest_lower"."T"` 能访问到——裸名
    // `dbtest_lower.T` 反而报 `-2103`（这正是 `qualified_probe.rs` D 段实测到的）。
    for schema in ["DBTESTA", "DBTEST_LOWER"] {
        let ddl = format!(r#"CREATE SCHEMA {schema} AUTHORIZATION SYSDBA"#);
        match conn.execute(ddl.as_str(), ()).await {
            Ok(_) => println!("      清场：已建模式 {schema}"),
            Err(e) => println!("      清场：模式 {schema} 未建（{e}）"),
        }
    }

    for table in [r#""DBTESTA"."T_QUALIFIED""#, r#""DBTEST_LOWER"."T_LOWER""#] {
        let ddl = format!("DROP TABLE IF EXISTS {table} CASCADE CONSTRAINTS");
        match conn.execute(ddl.as_str(), ()).await {
            Ok(_) => println!("      清场：已删残留表 {table}"),
            Err(e) => println!("      清场：删残留表 {table} 失败（{e}）"),
        }
    }

    // 必须先归还连接：`close()` 会等「已借出的连接全部回池」，握着连接调它会等很久。
    drop(conn);
    let _ = pool.close().await;
    pool.wait_closed().await;
}

#[tokio::test]
#[ignore = "真机：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn qualified_model_table_lands_in_its_schema() {
    let Some(cfg) = real_config() else {
        return;
    };

    prepare(&cfg).await;

    let driver = DamengDriver::from_config(cfg.clone())
        .await
        .expect("构造 DamengDriver 失败（连不上达梦？）");
    let mut db = toasty::Db::builder()
        .models(toasty::models!(crate::*))
        .build(driver)
        .await
        .expect("Db::builder().build(driver) 失败");

    println!("[1/6] push_schema（应建出 DBTESTA.T_QUALIFIED 与 DBTEST_LOWER.T_LOWER）");
    db.push_schema().await.expect("push_schema 失败");

    // 目录表确认：表在 DBTESTA 里，且**不在**默认模式（SYSDBA）里。
    let pool: dm_pool::DmPool = dm_pool::DmPool::connect(cfg.clone())
        .await
        .expect("连不上达梦（目录校验）");
    let mut conn = PooledConn::acquire(&pool).await.expect("借连接失败");

    let in_dbtesta = scalar_i64(
        &mut conn,
        "SELECT COUNT(*) FROM ALL_TABLES WHERE OWNER = 'DBTESTA' AND TABLE_NAME = 'T_QUALIFIED'",
    )
    .await;
    let in_sysdba = scalar_i64(
        &mut conn,
        "SELECT COUNT(*) FROM ALL_TABLES WHERE OWNER = 'SYSDBA' AND TABLE_NAME = 'T_QUALIFIED'",
    )
    .await;
    // 小写写法：模型里写的是 `dbtest_lower.T_LOWER`，折大写后应落到 `DBTEST_LOWER`。
    let in_lower = scalar_i64(
        &mut conn,
        "SELECT COUNT(*) FROM ALL_TABLES WHERE OWNER = 'DBTEST_LOWER' AND TABLE_NAME = 'T_LOWER'",
    )
    .await;
    println!(
        "      DBTESTA 里 = {in_dbtesta} 张，SYSDBA 里 = {in_sysdba} 张，DBTEST_LOWER 里 = {in_lower} 张"
    );
    assert_eq!(in_dbtesta, 1, "表没有建在 DBTESTA 里（限定名没有生效？）");
    assert_eq!(
        in_sysdba, 0,
        "表被建到了默认模式 SYSDBA 里（限定名没有被渲染成两段）"
    );
    assert_eq!(in_lower, 1, "小写限定名没有折成大写落到 DBTEST_LOWER 里");

    // 当前会话的默认模式无关紧要：所有 DML 都必须带模式前缀。
    println!("[2/6] create! / get / all");
    let created = toasty::create!(QualifiedNote {
        title: "限定名",
        note: None,
    })
    .exec(&mut db)
    .await
    .expect("插入失败");
    assert!(created.id > 0, "自增主键没有被回填：id = {}", created.id);

    let reloaded = QualifiedNote::get_by_id(&mut db, &created.id)
        .await
        .expect("按主键查询失败");
    assert_eq!(reloaded.title, "限定名");

    let all: Vec<QualifiedNote> = QualifiedNote::all()
        .exec(&mut db)
        .await
        .expect("全表查询失败");
    assert_eq!(all.len(), 1, "刚插入一行，all() 应返回 1 行");

    // 反证：不带前缀的表名在默认模式里查不到（说明 DML 真的走的是限定名）。
    println!("[3/6] 反证：SYSDBA.T_QUALIFIED 不存在");
    let bare = conn
        .query("SELECT COUNT(*) FROM \"SYSDBA\".\"T_QUALIFIED\"", ())
        .await;
    match bare {
        Ok(_) => panic!("SYSDBA.T_QUALIFIED 竟然存在：限定名没生效"),
        Err(e) => println!("      预期中的错误：{e}"),
    }

    println!("[4/6] update!");
    let mut row = reloaded;
    toasty::update!(row {
        title: "限定名-改"
    })
    .exec(&mut db)
    .await
    .expect("更新失败");
    let again = QualifiedNote::get_by_id(&mut db, &created.id)
        .await
        .expect("更新后重查失败");
    assert_eq!(again.title, "限定名-改");

    println!("[5/6] delete!");
    toasty::stmt::Query::<toasty::stmt::List<QualifiedNote>>::all()
        .filter(QualifiedNote::fields().id().eq(created.id))
        .delete()
        .exec(&mut db)
        .await
        .expect("删除失败");
    let left: Vec<QualifiedNote> = QualifiedNote::all()
        .exec(&mut db)
        .await
        .expect("删除后全表查询失败");
    assert!(
        left.is_empty(),
        "删除后 all() 应为空，实际 {} 行",
        left.len()
    );

    // 小写限定名走同一条链路：`dbtest_lower.T_LOWER` ⇒ `DBTEST_LOWER.T_LOWER`。
    println!("[6/6] 小写限定名 create! / get");
    let lower = toasty::create!(LowerNote {
        label: "小写模式"
    })
    .exec(&mut db)
    .await
    .expect("小写限定名插入失败");
    let lower_back = LowerNote::get_by_id(&mut db, &lower.id)
        .await
        .expect("小写限定名按主键查询失败");
    assert_eq!(lower_back.label, "小写模式");

    // 收尾：把表与临时模式删掉，避免下次跑撞 `-2124 对象已存在`。
    for sql in [
        r#"DROP TABLE "DBTESTA"."T_QUALIFIED""#,
        r#"DROP TABLE "DBTEST_LOWER"."T_LOWER""#,
    ] {
        match conn.execute(sql, ()).await {
            Ok(_) => println!("      收尾：已执行 {sql}"),
            Err(e) => println!("      收尾：{sql} 失败（{e}）"),
        }
    }
    for sql in [
        r#"DROP SCHEMA DBTEST_LOWER RESTRICT"#,
        r#"DROP SCHEMA "dbtest_lower" RESTRICT"#,
    ] {
        match conn.execute(sql, ()).await {
            Ok(_) => println!("      收尾：已执行 {sql}"),
            Err(e) => println!("      收尾：{sql} 失败（{e}）"),
        }
    }

    // `db` 自己持有一个池；`conn` 也得先还回我们的池，否则 `close()` 会一直等。
    drop(conn);
    drop(db);
    let _ = pool.close().await;
    pool.wait_closed().await;
}
