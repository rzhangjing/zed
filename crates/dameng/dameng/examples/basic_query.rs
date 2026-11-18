//! 最小可用示例：连上达梦 → 查一行 → 建表 → 参数化写入 → 用 `#[derive(FromRow)]` 读回。
//!
//! 跑法（没配 `DM_TEST_DSN` / `DM_SERVER` 时会打印一行原因后正常退出）：
//!
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc run -p dameng --example basic_query
//! ```
//!
//! 注意：宿主默认工具链是 gnu 而 `dlltool.exe` 不在 PATH，所以**必须**显式
//! `+stable-x86_64-pc-windows-msvc`（例子都要连着达梦实例跑，见 `README.md`）。

use dameng::prelude::*;

/// 示例表：名字带前缀，避免与别的示例/测试撞名。
const T_USER: &str = "DM_EXAMPLE_USER";

/// 从环境读真机配置：`DM_TEST_DSN`（ODBC 连接串）优先，否则 `DM_SERVER` 系。
///
/// 返回 `None` 表示本机没配达梦——示例会打印原因后正常退出，而不是 panic。
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

/// `FromRow` 由 `dameng::prelude` 里的派生宏生成；列名查找**大小写不敏感**
/// （达梦默认把标识符折成大写），所以这里写小写字段名也能对上 `ID`/`NAME`。
#[derive(Debug, FromRow)]
struct User {
    id: i32,
    name: String,
}

// 当前线程运行时足够：真正的阻塞工作（ODBC 调用）跑在 `dm-pool` 自建的阻塞线程池上，
// 而不是 tokio 的工作线程上。这样也不必为本 crate 打开 `rt-multi-thread` feature。
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), DmError> {
    let Some(cfg) = config() else {
        println!("跳过：未配置 DM_TEST_DSN / DM_SERVER（本示例需要一台可达的达梦 DM8）");
        return Ok(());
    };

    // 池大小就是并发上限（达梦不支持 ODBC 异步，每条连接占一个线程）。
    let pool = Pool::connect_with(cfg, ExecutorConfig::for_pool(2)).await?;
    let mut conn = PooledConn::acquire(&pool).await?;

    // 1) 不落表的查询：值从 `Row` 里按列名取，取错类型会报 `Decode` 而不是给默认值。
    let rs = conn
        .query("select 1 as N, '达梦' as S from dual", ())
        .await?;
    for row in rs.rows() {
        println!(
            "select 1 → N = {}, S = {}",
            row.get::<i32>("N")?,
            row.get::<String>("S")?
        );
    }

    // 2) 建表（达梦没有 `create table if not exists`，所以先 `drop … if exists`）。
    conn.execute(&format!("drop table if exists {T_USER}"), ())
        .await?;
    conn.execute(
        &format!("create table {T_USER} (ID INT PRIMARY KEY, NAME VARCHAR(50) NOT NULL)"),
        (),
    )
    .await?;

    // 3) 参数化写入走**数组绑定**（不是逐行循环）：1000 行量级实测比逐行快一个数量级。
    let outcome = conn
        .execute_many(
            &format!("insert into {T_USER} (ID, NAME) values (?, ?)"),
            vec![(1_i32, "阿一"), (2, "阿二"), (3, "阿三")],
        )
        .await?;
    println!(
        "批量写入：executed = {}, affected = {}, 全部成功 = {}",
        outcome.executed,
        outcome.affected,
        outcome.is_complete()
    );

    // 4) 读回并用派生宏反序列化。
    let rs = conn
        .query(&format!("select ID, NAME from {T_USER} order by ID"), ())
        .await?;
    for row in rs.rows() {
        let user: User = User::from_row(row)?;
        // 逐字段打印（而不是 `{user:?}`）：这样两个字段都算「被读过」，
        // 也顺带演示派生出来的结构体字段就是普通字段。
        println!("读到一行：id = {}, name = {}", user.id, user.name);
    }

    // 5) 收尾：显式清表（示例不该在库里留东西），然后关池等存活连接归零。
    conn.execute(&format!("drop table if exists {T_USER}"), ())
        .await?;
    drop(conn);
    pool.close().await?;
    pool.wait_closed().await;
    println!("完成。");
    Ok(())
}
