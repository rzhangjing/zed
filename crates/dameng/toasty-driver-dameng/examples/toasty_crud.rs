//! T8.10：Toasty × 达梦的**最小可用示例**（真机跑）。
//!
//! 演示 §9.7「挂载方式与限制」的全部要点：
//!
//! - **挂载**：必须 `Db::builder().build(driver)` 直接喂 `&dyn Driver`——
//!   `dameng://…` URL **走不通**（`toasty/src/db/connect.rs` 的 URL→驱动分派是闭 match，
//!   第三方驱动无法按 scheme 注册自己）；
//! - **两层池的上限预算**：`dm-pool` 侧的 `PoolConfig::max_size` 是**物理连接**上限，
//!   Toasty 自带的 `deadpool` 是应用侧上限；两者相乘要落在服务端预算内
//!   （`MAX_SESSIONS=1500`，§9.6 的风险 R-06）。驱动**不**设 `Driver::max_connections()`
//!   ——两处都设会互相打架（`driver.rs` 模块文档）；
//! - **MSRV**：Toasty 硬要求 Rust **≥ 1.95** + edition 2024；
//! - **日志安全**：Toasty 对 `Value::Bytes` 只渲染 `<N bytes>`，且**默认不打印参数值**，
//!   需要时显式 `log_statement_params(true)`；
//! - **二进制列的边界**：`stmt::Type::Bytes` 没有 JSON 表示 ⇒ 二进制列不能进
//!   `#[document]` / JSON 嵌入。
//!
//! 跑法（没配 `DM_TEST_DSN` / `DM_SERVER` 时打印一行「跳过」并以退出码 0 结束，
//! 与 `dm-query` 真机用例同一约定）：
//!
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc run -p toasty-driver-dameng --example toasty_crud
//! ```
//!
//! 示例会自己建表、跑完再清表（`push_schema` 发的是裸 `CREATE TABLE`，没有
//! `IF NOT EXISTS`，残留表会让下一次直接死在 `-2124 对象已存在` 上）。

use dm_core::{DmConfig, PoolConfig};
use dm_query::PooledConn;
use toasty_driver_dameng::DamengDriver;

/// 示例模型：自增整数主键 + 唯一键 + 普通列 + 可空列 + 布尔列。
///
/// 五列正好覆盖达梦侧四条渲染规则：`IDENTITY(1,1)`（自增）、唯一约束、
/// `VARCHAR(8188)`、可空、`BIT`（布尔）。
#[derive(Debug, toasty::Model)]
struct Note {
    #[key]
    #[auto]
    id: i64,

    title: String,

    #[unique]
    slug: String,

    body: Option<String>,

    pinned: bool,
}

/// 从环境读真机配置；`None` = 本机没配达梦，示例直接跳过（退出码 0）。
fn real_config() -> Option<DmConfig> {
    let dsn = std::env::var("DM_TEST_DSN")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let mut cfg = if let Some(dsn) = dsn {
        DmConfig::from_connection_string(&dsn).expect("DM_TEST_DSN 不是合法的连接串")
    } else if std::env::var("DM_SERVER").is_ok() {
        DmConfig::from_env().expect("DM_SERVER 环境下无法构造 DmConfig")
    } else {
        eprintln!("跳过：未配置 DM_TEST_DSN / DM_SERVER（示例只在真库上跑）");
        return None;
    };
    // 物理连接上限：**唯一**的上限来源（Toasty 侧请用 `Db::builder().max_pool_size()`）。
    cfg.pool = PoolConfig {
        max_size: 4,
        min_idle: 0,
        ..PoolConfig::default()
    };
    Some(cfg)
}

/// 列出当前 schema 的表名（看清 Toasty 到底建了哪张表）。
async fn user_tables(cfg: &DmConfig) -> Vec<String> {
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

/// 清掉上次跑留下的示例表（`push_schema` 没有 `IF NOT EXISTS`）。
///
/// 表名大小写各试一遍：fork 把标识符折成大写（`toasty-sql/src/serializer/ident.rs`），
/// 所以现在库里是 `NOTES`；改动之前跑出来的残留是小写引号名。
async fn drop_stray_tables(cfg: &DmConfig) {
    let pool: dm_pool::DmPool = dm_pool::DmPool::connect(cfg.clone())
        .await
        .expect("连不上达梦（清理残留表）");
    let mut conn = PooledConn::acquire(&pool).await.expect("借连接失败");
    for name in [
        "NOTES",
        "__TOASTY_MIGRATIONS",
        "notes",
        "__toasty_migrations",
    ] {
        match conn
            .execute(&format!("DROP TABLE \"{name}\" CASCADE CONSTRAINTS"), ())
            .await
        {
            Ok(_) => println!("      清理：已删残留表 {name}"),
            // 表不存在是正常情况，其余错误也只提示（不能因此让示例失败）。
            Err(e) => println!("      清理：{name} 未删（{e}）"),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(cfg) = real_config() else {
        return Ok(());
    };

    // 语句日志：DDL 与 `toasty::query` target 上的 debug 日志。**默认不含参数值**。
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .try_init();

    // ---- ① 挂载：`Db::builder()` 是唯一的入口 ------------------------------------
    let driver = DamengDriver::from_config(cfg.clone()).await?;
    let mut db = toasty::Db::builder()
        .models(toasty::models!(crate::*))
        // 应用侧池上限；与 `PoolConfig::max_size`（=4）相乘 ≤ 服务端预算（§9.6 / R-06）。
        .max_pool_size(4)
        // 需要连参数值一起打日志时打开（§9.7 的日志安全条目）：
        // .log_statement_params(true)
        .build(driver)
        .await?;

    // ---- ② 建表 ------------------------------------------------------------------
    drop_stray_tables(&cfg).await;
    db.push_schema().await?;
    println!("建表完成；USER_TABLES = {:?}", user_tables(&cfg).await);

    // ---- ③ 插入：`#[auto]` 自增主键被回填（达梦没有 RETURNING，靠 `@@IDENTITY`） -----
    let first = toasty::create!(Note {
        title: "达梦上手",
        slug: "dm-getting-started",
        body: None,
        pinned: true,
    })
    .exec(&mut db)
    .await?;
    println!("插入并回填主键：{first:?}");
    assert!(first.id > 0, "自增主键没有被回填：id = {}", first.id);

    toasty::create!(Note {
        title: "第二篇",
        slug: "dm-second",
        body: Some("由 toasty 写入".to_string()),
        pinned: false,
    })
    .exec(&mut db)
    .await?;

    // ---- ④ 唯一键查询 -------------------------------------------------------------
    let same = Note::get_by_slug(&mut db, "dm-getting-started").await?;
    assert_eq!(same.id, first.id);
    println!("唯一键查询命中 id={}", same.id);

    // ---- ⑤ 全表查询 ---------------------------------------------------------------
    let all: Vec<Note> = Note::all().exec(&mut db).await?;
    println!("全表 {} 行", all.len());
    assert_eq!(all.len(), 2);

    // ---- ⑥ 更新 -------------------------------------------------------------------
    let mut row = same;
    toasty::update!(row {
        title: "达梦上手（已改写）"
    })
    .exec(&mut db)
    .await?;
    let reloaded = Note::get_by_slug(&mut db, "dm-getting-started").await?;
    assert_eq!(reloaded.title, "达梦上手（已改写）");
    println!("更新后重查：title = {}", reloaded.title);

    // ---- ⑦ 事务：提交可见、回滚不可见 ---------------------------------------------
    let mut tx = db.transaction().await?;
    toasty::create!(Note {
        title: "事务内",
        slug: "dm-in-tx",
        body: None,
        pinned: false,
    })
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    assert!(Note::get_by_slug(&mut db, "dm-in-tx").await.is_ok());

    let mut tx = db.transaction().await?;
    toasty::create!(Note {
        title: "会被回滚",
        slug: "dm-rolled-back",
        body: None,
        pinned: false,
    })
    .exec(&mut tx)
    .await?;
    tx.rollback().await?;
    assert!(Note::get_by_slug(&mut db, "dm-rolled-back").await.is_err());
    println!("事务：提交可见、回滚不可见 ✓");

    // ---- ⑧ 删除与收尾 -------------------------------------------------------------
    Note::filter_by_slug("dm-second")
        .delete()
        .exec(&mut db)
        .await?;
    let left: Vec<Note> = Note::all().exec(&mut db).await?;
    println!("删除后全表 {} 行", left.len());

    drop_stray_tables(&cfg).await;
    println!("示例跑通：Toasty × 达梦（T8.10）");
    Ok(())
}
