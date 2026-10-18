//! 语句缓存（`DmConfig::pool.stmt_cache_size`，CAP-C-11 / 设计方案 §5.4）的离线用例。
//!
//! 判据是「同一条 SQL 只 `prepare` 一次」：假后端把 `Backend::prepare` 的调用次数
//! 记在 `FakeShared::prepares` 上，于是「缓存真的命中了」这件事可以在**离线**断言，
//! 不必真机、也不必拿 SQL 往返耗时当证据（计时断言在共享 CI 上不可靠）。
//!
//! 覆盖六件事：命中/未命中计数、容量按 LRU 淘汰、执行失败丢条目、缓存是
//! **每条连接**一份、「干净归还保留缓存 / 脏归还复位清空缓存」，以及
//! **后端声明「不可复用」的语句一律不进缓存**（达梦的结果集语句，见文末用例）。

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::{FakeBackend, FakeShared, PoolOpts, shutdown, wait_for};
use dm_core::{Backend, ExecOutcome};
use dm_error::{DmError, DmErrorKind, Stage};
use dm_pool::{DmPool, ExecutorConfig, PoolGuard};

/// 起一个假后端池，并指定语句缓存容量（`0` = 关闭）。
async fn open_with_cache(shared: &Arc<FakeShared>, size: usize) -> DmPool<FakeBackend> {
    let mut cfg = common::config(shared, &PoolOpts::default().max_size(1));
    cfg.pool.stmt_cache_size = size;
    DmPool::connect_with(cfg, ExecutorConfig::for_pool(2))
        .await
        .expect("假后端建池不该失败")
}

/// 走一次缓存路径：把语句交给假后端执行，返回影响行数。
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

/// 走一次缓存路径，但闭包**故意失败**（用来验证「失败的语句不进缓存」）。
async fn run_failing(guard: &mut PoolGuard<FakeBackend>, sql: &str) -> Result<u64, DmError> {
    guard
        .with_cached_stmt(sql.to_owned(), |_raw, _stmt, _sql| {
            Err(DmError::at(DmErrorKind::Io, Stage::Execute))
        })
        .await
}

// ---------------------------------------------------------------------------
// 命中 / 未命中
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_same_sql_prepares_once_but_is_executed_every_time() {
    let shared = FakeShared::new();
    let pool = open_with_cache(&shared, 8).await;
    let mut guard = pool.acquire().await.expect("借出");

    assert_eq!(
        run_cached(&mut guard, "insert into t (id) values (1)")
            .await
            .unwrap(),
        1
    );
    assert_eq!(shared.prepares(), 1, "第一次必然要 prepare");

    assert_eq!(
        run_cached(&mut guard, "insert into t (id) values (1)")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        shared.prepares(),
        1,
        "同一条 SQL 第二次不该再 prepare（缓存命中）"
    );

    let stats = guard.stmt_cache_stats();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.size, 1);
    assert_eq!(stats.capacity, 8);

    // 执行次数与缓存命中无关：两条语句都真的执行了。
    assert_eq!(guard.conn_stats().ops, 2);

    drop(guard);
    shutdown(&pool).await;
}

#[tokio::test]
async fn different_sql_texts_are_different_entries() {
    let shared = FakeShared::new();
    let pool = open_with_cache(&shared, 8).await;
    let mut guard = pool.acquire().await.expect("借出");

    run_cached(&mut guard, "insert into t (id) values (1)")
        .await
        .unwrap();
    run_cached(&mut guard, "insert into t (id) values (2)")
        .await
        .unwrap();
    assert_eq!(shared.prepares(), 2, "文本不同就是两条语句");

    let stats = guard.stmt_cache_stats();
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.size, 2);

    drop(guard);
    shutdown(&pool).await;
}

#[tokio::test]
async fn a_disabled_cache_prepares_every_time_and_stores_nothing() {
    let shared = FakeShared::new();
    let pool = open_with_cache(&shared, 0).await;
    let mut guard = pool.acquire().await.expect("借出");

    run_cached(&mut guard, "insert into t (id) values (1)")
        .await
        .unwrap();
    run_cached(&mut guard, "insert into t (id) values (1)")
        .await
        .unwrap();
    assert_eq!(shared.prepares(), 2, "缓存关闭时每次都新建语句");

    let stats = guard.stmt_cache_stats();
    assert_eq!(stats.capacity, 0);
    assert_eq!(stats.size, 0, "关闭的缓存不许攒句柄");
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.misses, 0, "压根没查表就不该记未命中");

    drop(guard);
    shutdown(&pool).await;
}

// ---------------------------------------------------------------------------
// 容量与淘汰
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_least_recently_used_entry_is_evicted_first() {
    let shared = FakeShared::new();
    let pool = open_with_cache(&shared, 2).await;
    let mut guard = pool.acquire().await.expect("借出");

    for sql in ["a", "b", "c"] {
        run_cached(&mut guard, sql).await.unwrap();
    }
    // 容量 2：放进 c 时把最久未用的 a 挤掉。
    assert_eq!(shared.prepares(), 3);
    assert_eq!(guard.stmt_cache_stats().evictions, 1);

    // b 还在（用一下它，让它变成最近使用）。
    run_cached(&mut guard, "b").await.unwrap();
    assert_eq!(shared.prepares(), 3, "b 还在缓存里");

    // a 已经被淘汰：再跑一次必须重建。
    run_cached(&mut guard, "a").await.unwrap();
    assert_eq!(shared.prepares(), 4, "被淘汰的 a 必须重新 prepare");

    let stats = guard.stmt_cache_stats();
    assert_eq!(stats.size, 2);
    assert_eq!(stats.capacity, 2);
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.evictions, 2, "放进 a 又挤掉了 c");

    drop(guard);
    shutdown(&pool).await;
}

// ---------------------------------------------------------------------------
// 失败路径
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_failed_statement_is_not_kept_in_the_cache() {
    let shared = FakeShared::new();
    let pool = open_with_cache(&shared, 4).await;
    let mut guard = pool.acquire().await.expect("借出");

    assert!(
        run_failing(&mut guard, "insert into t (id) values (1)")
            .await
            .is_err()
    );
    assert_eq!(shared.prepares(), 1);
    assert_eq!(guard.stmt_cache_stats().size, 0, "失败的语句不进缓存");

    // 再来一次成功的：因为上一条被丢掉了，这里必须重新 prepare。
    run_cached(&mut guard, "insert into t (id) values (1)")
        .await
        .unwrap();
    assert_eq!(shared.prepares(), 2, "失败之后不复用半坏的语句");

    let stats = guard.stmt_cache_stats();
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.size, 1);

    drop(guard);
    shutdown(&pool).await;
}

// ---------------------------------------------------------------------------
// 缓存归连接所有
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_cache_is_per_connection() {
    let shared = FakeShared::new();
    let mut cfg = common::config(&shared, &PoolOpts::default().max_size(2));
    cfg.pool.stmt_cache_size = 4;
    let pool = DmPool::<FakeBackend>::connect_with(cfg, ExecutorConfig::for_pool(2))
        .await
        .expect("假后端建池不该失败");

    let mut first = pool.acquire().await.expect("借第一条");
    let mut second = pool.acquire().await.expect("借第二条");
    assert_ne!(first.id(), second.id(), "要的是两条不同的连接");

    run_cached(&mut first, "insert into t (id) values (1)")
        .await
        .unwrap();
    run_cached(&mut second, "insert into t (id) values (1)")
        .await
        .unwrap();

    assert_eq!(
        shared.prepares(),
        2,
        "缓存不跨连接共享：两条连接各 prepare 一次"
    );
    assert_eq!(first.stmt_cache_stats().hits, 0);
    assert_eq!(second.stmt_cache_stats().hits, 0);
    assert_eq!(first.stmt_cache_stats().size, 1);

    drop(first);
    drop(second);
    shutdown(&pool).await;
}

// ---------------------------------------------------------------------------
// 归还路径：干净保留、脏清空
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_clean_return_keeps_the_cache_and_a_dirty_return_clears_it() {
    let shared = FakeShared::new();
    let pool = open_with_cache(&shared, 4).await;

    let first_id = {
        let mut guard = pool.acquire().await.expect("借出");
        run_cached(&mut guard, "insert into t (id) values (1)")
            .await
            .unwrap();
        assert_eq!(shared.prepares(), 1);
        // 干净归还：连接直接回空闲栈，缓存留着。
        guard.id()
    };

    let hits_before_dirty = {
        let mut guard = pool.acquire().await.expect("再借");
        assert_eq!(guard.id(), first_id, "应当借到同一条物理连接");
        run_cached(&mut guard, "insert into t (id) values (1)")
            .await
            .unwrap();
        assert_eq!(shared.prepares(), 1, "干净归还后缓存还在");
        let hits = guard.stmt_cache_stats().hits;
        assert_eq!(hits, 1, "第二次执行命中了缓存");
        // 标脏归还：连接要过复位闸门，缓存条目必须被清掉。
        guard.mark_dirty();
        hits
    };

    wait_for(
        "脏连接复位后回到空闲栈",
        Duration::from_secs(2),
        || pool.stats().idle == 1,
    )
    .await;

    {
        let mut guard = pool.acquire().await.expect("复位之后再借");
        run_cached(&mut guard, "insert into t (id) values (1)")
            .await
            .unwrap();
        assert_eq!(
            shared.prepares(),
            2,
            "复位清空了缓存，这条 SQL 必须重新 prepare"
        );
        let stats = guard.stmt_cache_stats();
        // 复位清掉的是**条目**，累计计数是历史账（与 `clear()` 的单测同一口径）。
        assert_eq!(stats.hits, hits_before_dirty);
        assert_eq!(stats.size, 1);
    }

    assert_eq!(
        shared.connects.load(Ordering::Relaxed),
        1,
        "全程只有一条物理连接"
    );
    shutdown(&pool).await;
}

// ---------------------------------------------------------------------------
// 后端说「不可复用」的语句
// ---------------------------------------------------------------------------

/// 达梦上「结果集被取尽之后的语句句柄」就再也执行不了了（真机证据：
/// `crates/dm-odbc/examples/cursor_reuse_probe.rs`），所以 `dm-odbc` 的
/// `Backend::statement_reusable` 对结果集语句回 `false`。假后端照抄这条判据
/// （`select` 前缀 ⇒ 不可复用），于是「查询不跨调用复用」在这里可以离线钉住。
#[tokio::test]
async fn a_statement_the_backend_calls_unreusable_is_never_cached() {
    let shared = FakeShared::new();
    let pool = open_with_cache(&shared, 8).await;
    let mut guard = pool.acquire().await.expect("借出");

    run_cached(&mut guard, "select 1 from dual").await.unwrap();
    assert_eq!(shared.prepares(), 1);
    assert_eq!(guard.stmt_cache_stats().size, 0, "不可复用的语句不许进缓存");

    // 第二次：缓存里没有，必须重新 prepare（这正是达梦上唯一可行的做法）。
    run_cached(&mut guard, "select 1 from dual").await.unwrap();
    assert_eq!(shared.prepares(), 2, "不可复用的语句每次都重建");

    let stats = guard.stmt_cache_stats();
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.size, 0);

    // 与它同连接的可复用语句不受影响。
    run_cached(&mut guard, "insert into t (id) values (9)")
        .await
        .unwrap();
    run_cached(&mut guard, "insert into t (id) values (9)")
        .await
        .unwrap();
    assert_eq!(shared.prepares(), 3, "DML 那条只 prepare 一次");
    assert_eq!(guard.stmt_cache_stats().size, 1);

    drop(guard);
    shutdown(&pool).await;
}
