//! `dm-pool` 并发与泄漏测试（T4.7，离线假后端）。
//!
//! G4 的四条硬判据里有三条在这一层就能确定性验证（真机版在
//! `tests/g4_real.rs`，需要 `DM_TEST_DSN`）：
//!
//! 1. 高并发下**峰值在飞数绝不越过 `max_size`**、名额账目始终配平；
//! 2. future 被反复 `timeout(…).await` 丢弃后**不泄漏名额**：`in_use` 归零、
//!    等待计数归零、连接数不涨；
//! 3. 被丢弃/被取消的连接**不可回收复用**（D11）。
//!
//! ⚠️ 改这个文件时别用 `Get-Content -Raw` + `Set-Content` 回写：那样会把三字节
//! UTF-8 字符（。、、「」⇒ 之类）打成 `?`，文件随即变成非法 UTF-8，
//! cargo 直接报 `stream did not contain valid UTF-8`。要改就用能指定编码的写入。
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::fake::FakeShared;
use common::{PoolOpts, assert_stats_consistent, open, shutdown, wait_for};
use dm_pool::PoolState;

/// 假后端上跑一轮「借出 → 干活 → 归还」。
async fn one_round(pool: &dm_pool::DmPool<common::FakeBackend>, ops: usize) {
    for _ in 0..ops {
        let mut guard = pool.acquire().await.expect("借出不该失败");
        guard.with_conn(|_| Ok(())).await.expect("操作不该失败");
    }
}

// ---------------------------------------------------------------------------
// 1. 高并发：峰值不越界、账目配平
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn heavy_concurrency_never_exceeds_max_size() {
    const MAX_SIZE: usize = 8;
    const TASKS: usize = 64;
    const OPS: usize = 64;

    let shared = FakeShared::new();
    let pool = open(
        &shared,
        PoolOpts::default()
            .max_size(MAX_SIZE)
            .acquire_timeout(Duration::from_secs(10)),
    )
    .await;
    let pool = Arc::new(pool);

    let mut tasks = Vec::with_capacity(TASKS);
    for _ in 0..TASKS {
        let pool = Arc::clone(&pool);
        tasks.push(tokio::spawn(async move { one_round(&pool, OPS).await }));
    }
    for task in tasks {
        task.await.expect("任务不该 panic");
    }

    wait_for("所有句柄归还", Duration::from_secs(5), || {
        pool.stats().in_use == 0
    })
    .await;

    let stats = pool.stats();
    assert_stats_consistent(&pool, &stats);
    assert_eq!(stats.max_size, MAX_SIZE);
    assert_eq!(stats.waiting, 0, "不该有残留的等待者：{stats:?}");
    assert_eq!(
        stats.acquired, stats.released,
        "借出与归还必须配平：{stats:?}"
    );
    assert_eq!(stats.acquired, (TASKS * OPS) as u64, "{stats:?}");
    assert!(
        shared.peak_live() <= MAX_SIZE,
        "峰值物理连接数 {} 越过 max_size {MAX_SIZE}",
        shared.peak_live()
    );

    shutdown(&pool).await;
    assert_eq!(shared.live(), 0, "关池后不该还有存活连接");
    assert_stats_consistent(&pool, &pool.stats());
    assert_eq!(pool.stats().size, 0);
}

// ---------------------------------------------------------------------------
// 2. future 反复被丢弃：不泄漏名额、不泄漏等待计数
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn dropped_acquire_futures_do_not_leak_slots() {
    // `max_size = 1`：谁拿到唯一那条连接，其他人就只能在等待队列里排队。
    let shared = FakeShared::new();
    let pool = open(
        &shared,
        PoolOpts::default()
            .max_size(1)
            .acquire_timeout(Duration::from_millis(1)),
    )
    .await;

    // 占住唯一的名额，让所有 `acquire` 都在排队。
    let holder = pool.acquire().await.expect("占位借出");

    for _ in 0..200 {
        // 超时 ⇒ future 被 drop ⇒ 它在信号量队列里的节点也必须一起撤掉。
        let err = pool
            .acquire()
            .await
            .expect_err("名额被占满时 acquire 必须超时");
        assert!(
            matches!(err.kind, dm_error::DmErrorKind::PoolTimeout),
            "{err:?}"
        );
    }

    let stats = pool.stats();
    assert_eq!(stats.waiting, 0, "被丢弃的等待者必须撤出队列：{stats:?}");
    assert_eq!(stats.in_use, 1);
    assert_stats_consistent(&pool, &stats);

    // 归还之后账目仍然配平，且没有因为刚才的 200 次超时凭空多出连接。
    drop(holder);
    wait_for("占位连接归还", Duration::from_secs(2), || {
        pool.stats().idle == 1
    })
    .await;
    assert_stats_consistent(&pool, &pool.stats());
    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        1
    );

    // 现在能借到了：名额没有被丢掉的 future 吃掉。
    let mut guard = pool.acquire().await.expect("名额必须仍然可用");
    guard.with_conn(|_| Ok(())).await.expect("操作");
    drop(guard);

    shutdown(&pool).await;
    assert_eq!(shared.live(), 0);
}

// ---------------------------------------------------------------------------
// 3. 混合负载下的取消语义（D11）与终态配平
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn cancelled_operations_under_load_are_never_reused() {
    let shared = FakeShared::new();
    let pool = open(
        &shared,
        PoolOpts::default()
            .max_size(4)
            .acquire_timeout(Duration::from_secs(5)),
    )
    .await;
    let pool = Arc::new(pool);

    // 每个任务：借一条连接，跑一个 20 ms 的阻塞操作，但只等 1 ms —— 必然超时。
    // 于是操作在阻塞线程上继续跑完，而归还被判成「取消」⇒ 连接销毁。
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let pool = Arc::clone(&pool);
        tasks.push(tokio::spawn(async move {
            let mut guard = pool.acquire().await.expect("借出");
            let outcome = tokio::time::timeout(Duration::from_millis(1), async {
                guard
                    .with_conn(|_| {
                        std::thread::sleep(Duration::from_millis(20));
                        Ok(())
                    })
                    .await
            })
            .await;
            assert!(outcome.is_err(), "1 ms 内不可能跑完 20 ms 的操作");
            // future 被 drop 掉，句柄也随之归还。
        }));
    }
    for task in tasks {
        task.await.expect("任务不该 panic");
    }

    wait_for(
        "取消的连接全部被销毁",
        Duration::from_secs(10),
        || {
            let stats = pool.stats();
            stats.in_use == 0 && stats.to_recycle == 0
        },
    )
    .await;

    let stats = pool.stats();
    assert_stats_consistent(&pool, &stats);
    assert!(
        stats.recycled_cancelled >= 1,
        "至少应记下一次取消回收：{stats:?}"
    );
    assert_eq!(
        stats.size, stats.idle,
        "除了空闲的，不该有别的连接留在池里：{stats:?}"
    );
    assert!(
        shared.live() <= 4,
        "存活连接数 {} 越过 max_size",
        shared.live()
    );

    // 池还能继续干活。
    let mut guard = pool.acquire().await.expect("负载之后仍能借出");
    guard.with_conn(|_| Ok(())).await.expect("操作");
    drop(guard);

    shutdown(&pool).await;
    assert_eq!(shared.live(), 0);
    assert_eq!(pool.state(), PoolState::Closed);
    let _ = Arc::clone(&shared);
}
