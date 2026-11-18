//! 连接池示例：池大小就是并发上限，多任务排队借连接，最后看指标并关池。
//!
//! 达梦**不支持 ODBC 异步**（`SQL_ATTR_ASYNC_ENABLE` 报 `HYC00/-70054`），所以每条
//! 物理连接都占住一个线程：`max_size` 既是池上限，也是**并发上限**。超过它的调用方
//! 会在池里排队（`stats().waiting`），而不是把线程数顶爆。
//!
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc run -p dameng --example pool
//! ```

use std::sync::Arc;
use std::time::Instant;

use dameng::prelude::*;

/// 池上限（也是并发上限）。
const MAX_SIZE: usize = 4;
/// 提交的任务数：**故意大于** `MAX_SIZE`，好让排队可见。
const TASKS: i32 = 12;

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

// 当前线程运行时：并发上限由池（以及它背后的阻塞线程池）决定，不由 tokio 决定。
// `tokio::spawn` 的任务数远超线程数也没关系——它们排在池上等连接。
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), DmError> {
    let Some(mut cfg) = config() else {
        println!("跳过：未配置 DM_TEST_DSN / DM_SERVER（本示例需要一台可达的达梦 DM8）");
        return Ok(());
    };

    // 池参数挂在 `DmConfig` 上（`dm-core` 定义、`dm-pool` 原样转出）：
    // 调用方只依赖 `dameng` 就能把池配好。逐字段赋值而不是结构体字面量：
    // `PoolConfig` 是 `#[non_exhaustive]`，外部 crate 写不出 `PoolConfig { .. }`。
    cfg.pool.max_size = MAX_SIZE;
    cfg.pool.min_idle = 0;

    let pool = Arc::new(Pool::connect_with(cfg, ExecutorConfig::for_pool(MAX_SIZE)).await?);
    println!("池已建：max_size = {MAX_SIZE}，并发提交 {TASKS} 个任务");

    let started = Instant::now();
    let mut tasks = Vec::with_capacity(TASKS as usize);
    for i in 0..TASKS {
        let pool = Arc::clone(&pool);
        tasks.push(tokio::spawn(async move {
            // `acquire` 在池满时会排队；`PooledConn` 一旦离开作用域就归还连接。
            let mut conn = PooledConn::acquire(&pool).await?;
            let row = conn
                .query_one("select cast(? as int) as N from dual", (i,))
                .await?
                .expect("这个查询必定返回一行");
            row.get::<i32>("N")
        }));
    }

    let mut sum = 0_i32;
    for task in tasks {
        sum += task.await.expect("任务 panic")?;
    }
    let elapsed = started.elapsed();
    println!("12 个任务全部完成：sum = {sum}（0..12 之和 = 66），耗时 {elapsed:?}");

    if sum != (0..TASKS).sum::<i32>() {
        println!("⚠️ 结果与预期不符——请检查查询与参数绑定");
    }

    // 指标快照：瞬时量（size/idle/in_use/waiting）+ 累计量（created/acquired/released）。
    let stats = pool.stats();
    println!(
        "池指标：state = {}, size = {}, idle = {}, to_recycle = {}, in_use = {}, waiting = {}",
        pool.state(),
        stats.size,
        stats.idle,
        stats.to_recycle,
        stats.in_use,
        stats.waiting
    );
    println!(
        "累计：created = {}, acquired = {}, released = {}, closed = {}",
        stats.created, stats.acquired, stats.released, stats.closed
    );

    // 关池：先不再接受新借出，再等存活连接归零（`stats().size` 归零）。
    pool.close().await?;
    pool.wait_closed().await;
    println!("池已关闭：size = {}", pool.stats().size);
    Ok(())
}
