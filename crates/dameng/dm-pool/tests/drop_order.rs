//! **CT-09** 的离线判据：释放顺序（`stmt → conn → env`）。
//!
//! 契约原文（设计方案 §10.2）：句柄 Drop 顺序正确（stmt → conn → env），Drop 不 panic。
//! 其中 `env` 是 `odbc-api` 进程级环境句柄（由库自己管），本项目要负责的是
//! **语句先于它所属的连接**被释放——真机上顺序反了不是「不优雅」，是直接坏：
//! 连接/env 先走之后 `SQLFreeHandle` 会返回 `-2`（`SQL_INVALID_HANDLE`），
//! 甚至把进程打成 `0xc0000005`（证据：`crates/dm-odbc/examples/cursor_reuse_probe.rs`，
//! 以及 Round 11 引入语句缓存时真实踩到的那次回归）。
//!
//! 假后端把每次释放都记进 `FakeShared::ops`：语句记 `drop_stmt#N@connM`，
//! 连接记 `reset#M` / `close#M` / `drop_conn#M`。于是「顺序」这件事在**离线**
//! 可以逐条断言，不必依赖真机的 `SQLFreeHandle` 是否恰好报错。
//!
//! 覆盖三条真实路径：显式关池（`close#`）、脏连接复位（`reset#`）、
//! 池被直接 drop（`drop_conn#`）；外加「失败的语句不留句柄」。

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{FakeBackend, FakeShared, PoolOpts, fake, shutdown, wait_for};
use dm_core::{Backend, ExecOutcome};
use dm_error::{DmError, DmErrorKind, Stage};
use dm_pool::{DmPool, ExecutorConfig, PoolGuard};

/// 非结果集语句（假后端判定为「可复用」），用来把语句放进缓存。
const INSERT: &str = "insert into t (id) values (1)";

async fn open_with_cache(shared: &Arc<FakeShared>, size: usize) -> DmPool<FakeBackend> {
    let mut cfg = common::config(shared, &PoolOpts::default().max_size(1));
    cfg.pool.stmt_cache_size = size;
    DmPool::connect_with(cfg, ExecutorConfig::for_pool(2))
        .await
        .expect("假后端建池不该失败")
}

/// 走一次缓存路径（成功后语句留在缓存里）。
async fn run_cached(guard: &mut PoolGuard<FakeBackend>, sql: &str) -> Result<u64, DmError> {
    guard
        .with_cached_stmt(sql.to_owned(), |raw, stmt, _sql| {
            match raw.execute(stmt, "")? {
                ExecOutcome::Affected { rows } => Ok(rows.unwrap_or(0)),
                _ => Ok(0),
            }
        })
        .await
}

/// 走一次缓存路径，但闭包故意失败（语句不该留在缓存里）。
async fn run_failing(guard: &mut PoolGuard<FakeBackend>, sql: &str) -> Result<u64, DmError> {
    guard
        .with_cached_stmt(sql.to_owned(), |_raw, _stmt, _sql| {
            Err(DmError::at(DmErrorKind::Io, Stage::Execute))
        })
        .await
}

/// 每个物理连接的语句都必须在 `marker` 之前释放。
fn assert_dropped_before(log: &[String], marker: &str) {
    let ids = fake::conn_ids(log);
    assert!(!ids.is_empty(), "日志里应能看到物理连接：{log:?}");
    for id in ids {
        assert!(
            fake::statements_are_dropped_before(log, id, marker),
            "连接 {id} 的语句必须在 {marker} 之前释放：{log:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 显式关池
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cached_statements_are_dropped_before_the_connection_is_closed() {
    let shared = FakeShared::new();
    let pool = open_with_cache(&shared, 4).await;
    let mut guard = pool.acquire().await.expect("借出");

    run_cached(&mut guard, INSERT).await.expect("执行");
    assert_eq!(guard.stmt_cache_stats().size, 1, "语句应当留在缓存里");
    drop(guard);
    wait_for("连接回到空闲栈", Duration::from_secs(2), || {
        pool.stats().idle == 1
    })
    .await;

    shutdown(&pool).await;

    let log = shared.op_log();
    assert!(
        shared.stmt_drops() >= 1,
        "缓存里的语句必须被释放（否则真机上就是 SQLFreeHandle -2）：{log:?}"
    );
    assert!(
        log.iter().any(|e| e.starts_with("close#")),
        "显式关池应当走 Backend::close：{log:?}"
    );
    assert_dropped_before(&log, "close#");
}

// ---------------------------------------------------------------------------
// 脏连接复位
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cached_statements_are_dropped_before_the_connection_is_reset() {
    let shared = FakeShared::new();
    let pool = open_with_cache(&shared, 4).await;
    let mut guard = pool.acquire().await.expect("借出");

    run_cached(&mut guard, INSERT).await.expect("执行");
    assert_eq!(guard.stmt_cache_stats().size, 1);
    guard.mark_dirty();
    drop(guard);
    wait_for(
        "脏连接复位后回到空闲栈",
        Duration::from_secs(2),
        || pool.stats().idle == 1,
    )
    .await;

    let log = shared.op_log();
    assert!(
        log.iter().any(|e| e.starts_with("reset#")),
        "脏连接应当走复位：{log:?}"
    );
    assert_dropped_before(&log, "reset#");
    shutdown(&pool).await;
}

// ---------------------------------------------------------------------------
// 池被直接 drop（没有 close()）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dropping_the_pool_still_drops_statements_before_the_connection() {
    let shared = FakeShared::new();
    let pool = open_with_cache(&shared, 4).await;
    {
        let mut guard = pool.acquire().await.expect("借出");
        run_cached(&mut guard, INSERT).await.expect("执行");
        assert_eq!(guard.stmt_cache_stats().size, 1);
    }

    // 直接 drop 池（不调 close）：连接对象只能靠 `Drop` 释放。
    drop(pool);
    wait_for(
        "池被 drop 后连接析构",
        Duration::from_secs(2),
        || shared.op_log().iter().any(|e| e.starts_with("drop_conn#")),
    )
    .await;

    let log = shared.op_log();
    assert!(
        !log.iter().any(|e| e.starts_with("close#")),
        "这条路径没有显式 close，不该出现 close#：{log:?}"
    );
    assert_dropped_before(&log, "drop_conn#");
}

// ---------------------------------------------------------------------------
// 失败的语句不留句柄
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_failed_statement_is_dropped_instead_of_being_cached() {
    let shared = FakeShared::new();
    let pool = open_with_cache(&shared, 4).await;
    let mut guard = pool.acquire().await.expect("借出");

    assert!(run_failing(&mut guard, INSERT).await.is_err());
    assert_eq!(
        guard.stmt_cache_stats().size,
        0,
        "执行失败的语句不该留在缓存里"
    );
    assert_eq!(shared.prepares(), 1, "失败路径也要 prepare 一次");
    assert!(
        shared.stmt_drops() >= 1,
        "失败的语句句柄必须立刻释放：{:?}",
        shared.op_log()
    );

    drop(guard);
    shutdown(&pool).await;
}
