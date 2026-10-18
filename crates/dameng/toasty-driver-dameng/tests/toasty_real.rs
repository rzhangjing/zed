//! T8.5–T8.1b 真机联调：**Toasty 引擎 → 达梦驱动 → 真库**端到端。
//!
//! 首轮（T8.1a）刻意先用上游 `toasty_sql::Serializer::mysql` 渲染，把「达梦与
//! MySQL 方言的真实差异」用真机测出来（结论：反引号标识符、`AUTO_INCREMENT`、
//! `INSERT … VALUES (DEFAULT, …)` 三处硬差异，见 `docs/toasty-fork.md`）。
//! 现在驱动已切到 vendored fork 的 `Serializer::dameng`，本用例转而作为
//! **T8.1b + T8.5 的回归**：建表 → 插入（自增主键回填）→ 唯一键查询 → 全表查询 →
//! 更新 → 删除，全部在真达梦上跑通。
//!
//! 判据（§12.2 的 M8 段）：六步全绿，且自增主键被回填。
//!
//! 跑法（host 是 gnu，`dlltool.exe` 不在 PATH，必须显式 msvc）：
//!
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc test -p toasty-driver-dameng --test toasty_real -- --include-ignored --test-threads=1 --nocapture
//! ```
//!
//! 没配 `DM_TEST_DSN` / `DM_SERVER` 时打印一行原因并直接返回（与 `dm-query` 的真机组
//! 同一约定）。

use dm_core::{DmConfig, PoolConfig};
use dm_query::PooledConn;
use toasty_driver_dameng::DamengDriver;

/// 探针模型：自增整数主键 + 唯一键 + 普通列 + 可空列。
///
/// 这几列正好覆盖首轮要测的四条渲染路径：`IDENTITY`/`AUTO_INCREMENT`、唯一约束、
/// 普通 `VARCHAR`、`NULL`。
#[derive(Debug, toasty::Model)]
struct ProbeWidget {
    #[key]
    #[auto]
    id: i64,

    name: String,

    #[unique]
    code: String,

    note: Option<String>,
}

/// 从环境读真机配置；`None` = 本机没配达梦，用例跳过。
fn real_config() -> Option<DmConfig> {
    let dsn = std::env::var("DM_TEST_DSN")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let mut cfg = if let Some(dsn) = dsn {
        DmConfig::from_connection_string(&dsn).expect("DM_TEST_DSN 不是合法的连接串")
    } else if std::env::var("DM_SERVER").is_ok() {
        DmConfig::from_env().expect("DM_SERVER 环境下无法构造 DmConfig")
    } else {
        eprintln!("跳过：未配置 DM_TEST_DSN / DM_SERVER（真机用例只在真库上跑，见文件头注释）");
        return None;
    };
    cfg.pool = PoolConfig {
        max_size: 4,
        min_idle: 0,
        ..PoolConfig::default()
    };
    Some(cfg)
}

/// 列出当前 schema 的表名（探针用：看清 Toasty 到底建了哪张表、有没有残留）。
async fn user_tables(cfg: &DmConfig) -> Vec<String> {
    // 显式标注类型：`DmPool<B>` 的 `B` 有默认值但推断不出来（E0283）。
    let pool: dm_pool::DmPool = dm_pool::DmPool::connect(cfg.clone())
        .await
        .expect("连不上达梦（USER_TABLES 探针）");
    let mut conn = PooledConn::acquire(&pool).await.expect("借连接失败");
    let rs = conn
        .query("SELECT TABLE_NAME FROM USER_TABLES ORDER BY TABLE_NAME", ())
        .await
        .expect("查询 USER_TABLES 失败");
    rs.rows()
        .iter()
        .map(|row| row.get::<String>(0).unwrap_or_else(|_| "<非文本>".into()))
        .collect()
}

/// 清掉上次跑失败留下的探针表。
///
/// `push_schema` 发的是裸 `CREATE TABLE`（没有 `IF NOT EXISTS`），上一次用例在
/// 中途 panic 留下的表会让这次直接死在 `-2124 对象已存在` 上——那是**环境残留**，
/// 不是驱动缺陷，所以先在入口清理干净。
async fn drop_probe_tables(cfg: &DmConfig) {
    let pool: dm_pool::DmPool = dm_pool::DmPool::connect(cfg.clone())
        .await
        .expect("连不上达梦（清理残留表）");
    let mut conn = PooledConn::acquire(&pool).await.expect("借连接失败");

    // 两套大小写都试一遍：fork 把标识符折成大写（`serializer/ident.rs`），所以**现在**库里
    // 是 `PROBE_WIDGETS`；而改动之前跑出来的残留是小写引号名。
    for name in [
        "PROBE_WIDGETS",
        "__TOASTY_MIGRATIONS",
        "probe_widgets",
        "__toasty_migrations",
    ] {
        match conn
            .execute(&format!("DROP TABLE \"{name}\" CASCADE CONSTRAINTS"), ())
            .await
        {
            Ok(_) => println!("      清理：已删残留表 {name}"),
            // 表不存在是正常情况，其余错误也只是提示（不能因此让用例失败）。
            Err(e) => println!("      清理：{name} 未删（{e}）"),
        }
    }
}

/// 打开语句日志（`toasty::query` target 上的 debug 日志 + 我们的 DDL 日志）。
///
/// 多个用例共用一个进程时会重复调用，所以用 `try_init` 并忽略「已经装过」的错误。
fn init_log() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .with_test_writer()
        .try_init();
}

#[tokio::test]
#[ignore = "真机 T8.1a：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_toasty_end_to_end_round_trip() {
    let Some(cfg) = real_config() else {
        return;
    };
    init_log();

    let driver = DamengDriver::from_config(cfg.clone())
        .await
        .expect("构造 DamengDriver 失败（连不上达梦？）");
    let mut db = toasty::Db::builder()
        .models(toasty::models!(crate::*))
        .build(driver)
        .await
        .expect("Db::builder().build(driver) 失败");

    // --- 建表：本轮已验证「双引号标识符 + IDENTITY(1, 1) + DM 类型名」达梦接受 -------
    println!("[1/6] push_schema …");
    drop_probe_tables(&cfg).await;
    match db.push_schema().await {
        Ok(()) => println!(
            "      push_schema OK；USER_TABLES = {:?}",
            user_tables(&cfg).await
        ),
        Err(e) => {
            println!("      push_schema 失败：{e}");
            println!("      USER_TABLES = {:?}", user_tables(&cfg).await);
            panic!("T8.1b 真机失败：push_schema 的 DDL 达梦不接受：{e}");
        }
    }

    // --- 插入：自增主键回填（Insert 的 ret ⇒ @@IDENTITY 路径） -----------------------------
    println!("[2/6] create!（自增主键应被回填）");
    let created = toasty::create!(ProbeWidget {
        name: "widget-a",
        code: "A-1",
        note: None,
    })
    .exec(&mut db)
    .await
    .expect("插入失败");
    println!("      created = {created:?}");
    assert!(created.id > 0, "自增主键没有被回填：id = {}", created.id);

    // --- 唯一键查询 --------------------------------------------------------------------
    println!("[3/6] get_by_code（唯一键）");
    let by_code = ProbeWidget::get_by_code(&mut db, "A-1")
        .await
        .expect("按唯一键查询失败");
    assert_eq!(by_code.id, created.id);

    // --- 全表查询 ----------------------------------------------------------------------
    println!("[4/6] all()");
    let all: Vec<ProbeWidget> = ProbeWidget::all()
        .exec(&mut db)
        .await
        .expect("全表查询失败");
    assert_eq!(all.len(), 1, "刚插入一行，all() 应返回 1 行");

    // --- 更新 -------------------------------------------------------------------------
    println!("[5/6] update!");
    let mut row = created;
    toasty::update!(row { name: "widget-a2" })
        .exec(&mut db)
        .await
        .expect("更新失败");
    let reloaded = ProbeWidget::get_by_code(&mut db, "A-1")
        .await
        .expect("更新后重查失败");
    assert_eq!(reloaded.name, "widget-a2");

    // --- 删除 -------------------------------------------------------------------------
    println!("[6/6] delete");
    ProbeWidget::filter_by_code("A-1")
        .delete()
        .exec(&mut db)
        .await
        .expect("删除失败");
    let after: Vec<ProbeWidget> = ProbeWidget::all()
        .exec(&mut db)
        .await
        .expect("删除后全表查询失败");
    assert!(after.is_empty(), "删除后应为空，实际 {} 行", after.len());

    println!(
        "T8.1b 端到端 OK；USER_TABLES = {:?}",
        user_tables(&cfg).await
    );
    drop_probe_tables(&cfg).await;
}
