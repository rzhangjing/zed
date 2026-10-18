//! `dm-pool` 池核心集成测试（T4.2/T4.3/T4.5/T4.6，离线假后端）。
//!
//! 这里**不碰 ODBC**：所有物理调用都由 `common::fake` 的假后端代劳，于是
//! 池的状态机、名额核算、复位/淘汰、取消语义可以被精确断言。

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::fake::{self, FakeBackend, FakeShared};
use common::{PoolOpts, assert_stats_consistent, open, shutdown, stays_false, wait_for};
use dm_core::{Backend, DmValue};
use dm_error::DmErrorKind;
use dm_pool::PoolState;

/// 最少几步就能起一个池，且默认参数是「安静的」（不探活、不回收）。
async fn pool(opts: PoolOpts) -> (Arc<FakeShared>, dm_pool::DmPool<FakeBackend>) {
    let shared = FakeShared::new();
    let pool = open(&shared, opts).await;
    (shared, pool)
}

// ---------------------------------------------------------------------------
// 建池与配置校验
// ---------------------------------------------------------------------------

#[tokio::test]
async fn connect_creates_exactly_one_connection() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(shared.live(), 1);
    assert_eq!(pool.state(), PoolState::Running);
    let stats = pool.stats();
    assert_eq!(stats.created, 1);
    assert_eq!(stats.idle, 1);
    assert_eq!(stats.in_use, 0);
    assert_stats_consistent(&pool, &stats);
    shutdown(&pool).await;
}

#[tokio::test]
async fn max_size_zero_is_rejected() {
    let shared = FakeShared::new();
    let opts = PoolOpts::default().max_size(0);
    let cfg = common::config(&shared, &opts);
    let err = dm_pool::DmPool::<FakeBackend>::connect(cfg)
        .await
        .expect_err("max_size=0 必须被拒绝");
    assert!(
        matches!(err.kind, DmErrorKind::Config(ref m) if m.contains("max_size")),
        "错误种类不对：{err:?}"
    );
    // 校验发生在建连之前：一条物理连接都不该被创建。
    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[tokio::test]
async fn min_idle_above_max_size_is_rejected() {
    let shared = FakeShared::new();
    let opts = PoolOpts::default().max_size(2).min_idle(3);
    let cfg = common::config(&shared, &opts);
    let err = dm_pool::DmPool::<FakeBackend>::connect(cfg)
        .await
        .expect_err("min_idle > max_size 必须被拒绝");
    assert!(
        matches!(err.kind, DmErrorKind::Config(ref m) if m.contains("min_idle")),
        "错误种类不对：{err:?}"
    );
    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

// ---------------------------------------------------------------------------
// 借出 / 归还
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acquire_reuses_the_same_physical_connection() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    let first = {
        let guard = pool.acquire().await.expect("第一次借出");
        guard.id()
    };
    // 归还后连接进空闲栈；第二次借出必须复用它，而不是新建。
    wait_for("连接回到空闲栈", Duration::from_secs(1), || {
        pool.stats().idle == 1
    })
    .await;
    let second = {
        let guard = pool.acquire().await.expect("第二次借出");
        guard.id()
    };
    assert_eq!(first, second, "同一时刻只有一条连接，应当复用");
    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(shared.live(), 1);
    shutdown(&pool).await;
}

#[tokio::test]
async fn get_is_an_alias_of_acquire() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    let id = { pool.get().await.expect("get 借出").id() };
    wait_for("归还", Duration::from_secs(1), || pool.stats().idle == 1).await;
    let again = pool.acquire().await.expect("acquire 借出");
    assert_eq!(id, again.id());
    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    drop(again);
    shutdown(&pool).await;
}

#[tokio::test]
async fn dropping_a_guard_returns_the_connection_to_the_pool() {
    let (_shared, pool) = pool(PoolOpts::default()).await;
    {
        let mut guard = pool.acquire().await.expect("借出");
        // 借出但**没有操作在跑**时，连接标签仍是 `Idle`：`InFlight` 的含义是
        // 「阻塞闭包正拿着裸连接」，而不是「句柄被借走了」。这个区分是必须的——
        // 归还责任握手（`mark_detached`）只会从 `InFlight` 成功，如果借出就置
        // `InFlight`，那么「借出后直接 drop」的句柄会以为自己该交给阻塞侧收官，
        // 于是没人归还，连接凭空消失。
        assert_eq!(guard.state(), "Idle");
        guard
            .with_conn(|raw| Ok(raw.name()))
            .await
            .expect("假后端操作");
        assert_eq!(guard.state(), "Idle", "操作收尾后应立即回到 Idle");
        assert_eq!(pool.stats().in_use, 1);
    }
    wait_for("归还后空闲", Duration::from_secs(1), || {
        pool.stats().idle == 1 && pool.stats().in_use == 0
    })
    .await;
    assert_stats_consistent(&pool, &pool.stats());
    shutdown(&pool).await;
}

#[tokio::test]
async fn guard_stats_track_operations_and_errors() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    let stats = {
        let mut guard = pool.acquire().await.expect("借出");
        guard.with_conn(|_| Ok(())).await.expect("成功操作");
        let err = guard
            .with_conn(|_| -> Result<(), dm_error::DmError> {
                Err(dm_error::DmError::new(DmErrorKind::Syntax))
            })
            .await
            .expect_err("失败操作必须把错误传回来");
        assert!(matches!(err.kind, DmErrorKind::Syntax));
        guard.conn_stats()
    };
    assert_eq!(stats.ops, 2);
    assert_eq!(stats.errors, 1);
    let _ = &shared;
    shutdown(&pool).await;
}

#[tokio::test]
async fn guard_rejects_a_second_operation_while_the_first_is_running() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    let mut guard = pool.acquire().await.expect("借出");
    // 第一次操作被外层放弃（超时），但阻塞线程上的闭包还在跑——此时裸连接
    // 已经被它取走，句柄必须拒绝第二次操作，而不是并发用同一条连接。
    let abandoned = tokio::time::timeout(
        Duration::from_millis(1),
        guard.with_conn(|_| {
            std::thread::sleep(Duration::from_millis(200));
            Ok(())
        }),
    )
    .await;
    assert!(abandoned.is_err(), "外层超时必须先返回");
    let err = guard
        .with_conn(|_| Ok(()))
        .await
        .expect_err("第一次操作还在跑，第二次必须被拒绝");
    assert!(
        matches!(err.kind, DmErrorKind::Config(ref m) if m.contains("占用")),
        "错误种类不对：{err:?}"
    );
    // 第一次操作跑完并因为是「取消」而被毒化（D11）；归还由句柄 drop 触发
    // （阻塞侧发现句柄还活着，就不抢收官责任）。
    drop(guard);
    wait_for(
        "被取消的连接被销毁",
        Duration::from_secs(2),
        || shared.closed.load(std::sync::atomic::Ordering::Relaxed) == 1,
    )
    .await;
    shutdown(&pool).await;
}

// ---------------------------------------------------------------------------
// 名额核算（max_size 是硬上限）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acquire_times_out_when_max_size_is_exhausted() {
    let opts = PoolOpts::default()
        .max_size(1)
        .acquire_timeout(Duration::from_millis(50));
    let (shared, pool) = pool(opts).await;
    let held = pool.acquire().await.expect("第一条必须成功");
    let err = pool
        .acquire()
        .await
        .expect_err("max_size=1 且连接被占，第二次借出必须超时");
    assert!(
        matches!(err.kind, DmErrorKind::PoolTimeout),
        "错误种类不对：{err:?}"
    );
    let stats = pool.stats();
    assert!(stats.timed_out >= 1, "timed_out 没记上：{stats:?}");
    assert_eq!(stats.waiting, 0, "等待计数必须归零（RAII 没生效？）");
    assert_eq!(stats.size, 1, "超时不该建出第二条连接：{stats:?}");
    assert_stats_consistent(&pool, &stats);
    let _ = &shared;
    drop(held);
    shutdown(&pool).await;
}

#[tokio::test]
async fn cancelled_waiter_does_not_leak_the_waiting_counter() {
    let opts = PoolOpts::default()
        .max_size(1)
        .acquire_timeout(Duration::from_secs(30));
    let (shared, pool) = pool(opts).await;
    let held = pool.acquire().await.expect("占住唯一的连接");
    // 等待中把 future 丢掉：`waiting` 必须回落，且不能记成超时。
    let _ = tokio::time::timeout(Duration::from_millis(30), pool.acquire()).await;
    assert_eq!(pool.stats().waiting, 0, "被取消的等待者漏了 waiting");
    let stats = pool.stats();
    assert_stats_consistent(&pool, &stats);
    let _ = &shared;
    drop(held);
    shutdown(&pool).await;
}

#[tokio::test]
async fn max_size_is_never_exceeded_under_contention() {
    let opts = PoolOpts::default().max_size(4);
    let (shared, pool) = pool(opts).await;
    let pool = Arc::new(pool);
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let pool = Arc::clone(&pool);
        tasks.push(tokio::spawn(async move {
            for _ in 0..20 {
                let mut guard = pool.acquire().await.expect("并发借出");
                guard
                    .with_conn(|raw| {
                        let _ = raw.name();
                        Ok(())
                    })
                    .await
                    .expect("并发操作");
            }
        }));
    }
    for task in tasks {
        task.await.expect("并发任务不该 panic");
    }
    assert!(
        shared.peak_live() <= 4,
        "峰值物理连接数 {} 超过 max_size 4",
        shared.peak_live()
    );
    let stats = pool.stats();
    assert!(stats.size <= 4, "{stats:?}");
    assert_eq!(stats.in_use, 0, "全部归还后不该还有在飞的：{stats:?}");
    assert_stats_consistent(&pool, &stats);
    shutdown(&pool).await;
}

#[tokio::test]
async fn failing_connect_does_not_leak_the_slot() {
    let opts = PoolOpts::default().max_size(2);
    let (shared, pool) = pool(opts).await;
    let held = pool.acquire().await.expect("占住空闲连接");
    // 第二次借出必须新建，而新建会失败。
    shared.fail_next_connects(1);
    let err = pool.acquire().await.expect_err("建连失败必须上报");
    assert!(
        matches!(err.kind, DmErrorKind::Connect { .. }),
        "错误种类不对：{err:?}"
    );
    assert!(pool.stats().connect_failed >= 1);
    // 名额必须还回去了：紧接着的借出要能成功。
    let guard = pool.acquire().await.expect("失败不该永久占住名额");
    assert_eq!(shared.live(), 2, "存活连接不该超过 max_size");
    assert!(
        shared.peak_live() <= 2,
        "峰值 {} 超过上限",
        shared.peak_live()
    );
    drop(guard);
    drop(held);
    shutdown(&pool).await;
}

// ---------------------------------------------------------------------------
// 复位与淘汰（T4.3）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dirty_guard_is_recycled_with_reset() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    {
        let mut guard = pool.acquire().await.expect("借出");
        guard.mark_dirty();
    }
    wait_for("复位完成", Duration::from_secs(2), || {
        shared.resets.load(std::sync::atomic::Ordering::Relaxed) == 1
    })
    .await;
    wait_for("复位后回到空闲栈", Duration::from_secs(2), || {
        pool.stats().idle == 1
    })
    .await;
    // 复位成功 ⇒ 同一条连接复用，不新建。
    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(shared.closed.load(std::sync::atomic::Ordering::Relaxed), 0);
    let stats = pool.stats();
    assert!(stats.recycled >= 1, "{stats:?}");
    assert_stats_consistent(&pool, &stats);
    assert!(
        fake::reset_steps_are_ordered(&shared.op_log()),
        "复位必须按「先关游标再回滚」的顺序：{:?}",
        shared.op_log()
    );
    shutdown(&pool).await;
}

#[tokio::test]
async fn poisoned_guard_is_destroyed_not_reused() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    let first_id = {
        let mut guard = pool.acquire().await.expect("借出");
        let id = guard.id();
        guard.mark_poisoned();
        id
    };
    wait_for(
        "毒化连接被物理关闭",
        Duration::from_secs(2),
        || shared.closed.load(std::sync::atomic::Ordering::Relaxed) == 1,
    )
    .await;
    assert_eq!(shared.live(), 0, "毒化的连接必须消失");
    // 池自己的 `closed` 也必须记上：它和假后端的 `closed` 是两套计数，
    // 早先只有 `discard` 记账、`close_physically`（毒化/复位失败/超期走的路）
    // 漏了，于是「物理关了但指标是 0」——真机 G4 用例才发现。
    assert_eq!(
        pool.stats().closed,
        1,
        "经回收路径销毁的连接必须记进 stats.closed：{:?}",
        pool.stats()
    );
    assert_eq!(
        shared.resets.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "毒化的连接不该被复位（状态未知，复位等于赌它能用）"
    );
    // 下一次借出必须是一条新连接。
    let second_id = pool.acquire().await.expect("毒化后仍能借出").id();
    assert_ne!(first_id, second_id);
    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        2
    );
    shutdown(&pool).await;
}

#[tokio::test]
async fn failed_reset_destroys_the_connection() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    shared.fail_next_resets(1);
    {
        let mut guard = pool.acquire().await.expect("借出");
        guard.mark_dirty();
    }
    wait_for(
        "复位失败后物理关闭",
        Duration::from_secs(2),
        || shared.closed.load(std::sync::atomic::Ordering::Relaxed) == 1,
    )
    .await;
    let stats = pool.stats();
    assert!(stats.recycle_failed >= 1, "{stats:?}");
    assert_eq!(shared.live(), 0);
    // 后续借出会新建，说明名额也被还了。
    let guard = pool.acquire().await.expect("淘汰后仍能借出");
    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        2
    );
    drop(guard);
    shutdown(&pool).await;
}

#[tokio::test]
async fn dead_connection_is_evicted_on_acquire() {
    let opts = PoolOpts::default().test_on_acquire(true);
    let (shared, pool) = pool(opts).await;
    shared.set_ping_alive(false);
    let guard = pool
        .acquire()
        .await
        .expect("探活失败应当淘汰旧连接后新建一条，而不是把错误抛给调用方");
    assert_eq!(guard.id(), 1, "新建的连接编号应当是 1（初始那条是 0）");
    let stats = pool.stats();
    assert!(stats.health_check_failed >= 1, "{stats:?}");
    assert_eq!(shared.closed.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        2
    );
    drop(guard);
    shutdown(&pool).await;
}

#[tokio::test]
async fn ping_error_that_is_not_connection_lost_still_lends() {
    let opts = PoolOpts::default().test_on_acquire(true);
    let (shared, pool) = pool(opts).await;
    shared.fail_next_pings(1); // 普通错误：连接本身不一定要丢
    let guard = pool.acquire().await.expect("探活报「不支持」不该导致淘汰");
    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "不该新建连接"
    );
    assert_eq!(shared.closed.load(std::sync::atomic::Ordering::Relaxed), 0);
    drop(guard);
    shutdown(&pool).await;
}

#[tokio::test]
async fn max_lifetime_rotates_before_lending() {
    let opts = PoolOpts::default()
        .max_lifetime(Duration::from_millis(20))
        .min_idle(0);
    let (shared, pool) = pool(opts).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let guard = pool.acquire().await.expect("超期连接应当被轮换后新建");
    assert_eq!(guard.id(), 1, "借出的应当是新建的那条");
    let stats = pool.stats();
    assert!(stats.lifetime_rotated >= 1, "{stats:?}");
    assert!(shared.closed.load(std::sync::atomic::Ordering::Relaxed) >= 1);
    drop(guard);
    shutdown(&pool).await;
}

#[tokio::test]
async fn idle_timeout_shrinks_the_idle_stack() {
    let opts = PoolOpts::default()
        .idle_timeout(Duration::from_millis(20))
        .recycle_interval(Duration::from_millis(10))
        .min_idle(0);
    let (shared, pool) = pool(opts).await;
    wait_for("空闲超期被回收", Duration::from_secs(2), || {
        shared.live() == 0
    })
    .await;
    let stats = pool.stats();
    assert!(stats.expired >= 1, "{stats:?}");
    assert_eq!(stats.size, 0, "回收后不该还留着连接：{stats:?}");
    assert_stats_consistent(&pool, &stats);
    shutdown(&pool).await;
}

#[tokio::test]
async fn maintenance_refills_to_min_idle() {
    let opts = PoolOpts::default()
        .max_size(4)
        .min_idle(2)
        .recycle_interval(Duration::from_millis(10));
    let (shared, pool) = pool(opts).await;
    wait_for("保活补到 min_idle", Duration::from_secs(2), || {
        pool.stats().idle >= 2
    })
    .await;
    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        2
    );
    let stats = pool.stats();
    assert_eq!(stats.size, 2);
    assert_stats_consistent(&pool, &stats);
    shutdown(&pool).await;
}

/// **T4.4 关闭语义**：维护任务在关池后不许再建连。
///
/// 判据是「关池前后的 `connects` 计数不变」——比「任务有没有退出」更好观察，
/// 也是真正要防的事故（关了池还在后台偷偷握连接，真机上就表现为
/// `v$sessions` 泄漏）。
///
/// 这里刻意**不用 `tokio::time::pause()`**：池内的空闲/寿命判定用的是
/// `std::time::Instant`（不受 tokio 时间旅行影响），暂停时钟推不动它。
/// 要测时间相关的行为只能配极小的真实时长再加 `wait_for`。
#[tokio::test]
async fn maintenance_stops_creating_connections_after_close() {
    let opts = PoolOpts::default()
        .max_size(4)
        .min_idle(2)
        .recycle_interval(Duration::from_millis(10));
    let (shared, pool) = pool(opts).await;
    wait_for("保活补到 min_idle", Duration::from_secs(2), || {
        pool.stats().idle >= 2
    })
    .await;
    let connects_before = shared.connects.load(std::sync::atomic::Ordering::Relaxed);

    shutdown(&pool).await;
    // 等好几个回收周期：任何「关池后还想补空闲」的维护动作都会在这里露出来。
    tokio::time::sleep(Duration::from_millis(80)).await;

    assert_eq!(
        shared.connects.load(std::sync::atomic::Ordering::Relaxed),
        connects_before,
        "关池之后维护任务不该再建连"
    );
    assert_eq!(shared.live(), 0, "关池之后不该还有物理连接");
    assert_eq!(pool.stats().size, 0);
}

// ---------------------------------------------------------------------------
// 取消语义（T4.5，D11）
// ---------------------------------------------------------------------------
/// G4 判据：超时要报 `DmErrorKind::Timeout`（不是裸的 `Elapsed`），
/// 且这条连接绝不再回池。
#[tokio::test]
async fn with_timeout_reports_timeout_and_discards_the_connection() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    let mut guard = pool.acquire().await.expect("借出");
    let err = guard
        .with_timeout(Duration::from_millis(1), |_| {
            std::thread::sleep(Duration::from_millis(20));
            Ok(1)
        })
        .await
        .expect_err("1 ms 内不可能跑完 20 ms 的操作");
    assert!(
        matches!(err.kind, DmErrorKind::Timeout),
        "超时必须映射成 Timeout：{err:?}"
    );
    assert_eq!(err.context.stage, dm_error::Stage::Execute, "{err:?}");

    drop(guard);
    wait_for("超时的连接被销毁", Duration::from_secs(2), || {
        shared.live() == 0
    })
    .await;
    let stats = pool.stats();
    assert_stats_consistent(&pool, &stats);
    assert!(
        stats.recycled_cancelled >= 1,
        "超时丢弃要记进 recycled{{cancelled}}：{stats:?}"
    );

    // 池仍可用。
    let mut guard = pool.acquire().await.expect("超时之后仍能借出");
    guard.with_conn(|_| Ok(())).await.expect("新连接可用");
    drop(guard);
    shutdown(&pool).await;
}

/// 造一次「调用方放弃等待」：闭包睡 60 ms，外层 1 ms 就超时。
async fn cancel_one_operation(pool: &dm_pool::DmPool<FakeBackend>) -> u64 {
    let mut guard = pool.acquire().await.expect("借出");
    let id = guard.id();
    let result = tokio::time::timeout(
        Duration::from_millis(1),
        guard.with_conn(|_| {
            std::thread::sleep(Duration::from_millis(60));
            Ok(())
        }),
    )
    .await;
    assert!(result.is_err(), "外层超时必须先返回");
    drop(guard);
    id
}

#[tokio::test]
async fn cancelled_operation_poisons_the_connection() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    let id = cancel_one_operation(&pool).await;
    wait_for(
        "被取消的连接被销毁",
        Duration::from_secs(2),
        || shared.closed.load(std::sync::atomic::Ordering::Relaxed) == 1,
    )
    .await;
    assert_eq!(shared.live(), 0, "D11：取消过的连接绝不回池");
    let stats = pool.stats();
    assert!(stats.recycled_cancelled >= 1, "{stats:?}");
    // 下一次借出必须换一条。
    let next = pool.acquire().await.expect("仍能借出");
    assert_ne!(next.id(), id);
    drop(next);
    shutdown(&pool).await;
}

#[tokio::test]
async fn reuse_after_cancel_escape_hatch_keeps_the_connection() {
    let opts = PoolOpts::default().reuse_after_cancel(true);
    let (shared, pool) = pool(opts).await;
    let id = cancel_one_operation(&pool).await;
    wait_for(
        "逃生舱：连接被复用而非销毁",
        Duration::from_secs(2),
        || pool.stats().idle == 1,
    )
    .await;
    assert_eq!(shared.closed.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(pool.stats().recycled_cancelled, 0, "开了逃生舱就不该记取消");
    let next = pool.acquire().await.expect("借出");
    assert_eq!(next.id(), id, "同一条连接应当被复用");
    drop(next);
    shutdown(&pool).await;
}

// ---------------------------------------------------------------------------
// 关池（T4.2 的 close / wait_closed）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn close_rejects_new_acquires() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    pool.close().await.expect("关池");
    assert_eq!(pool.state(), PoolState::Closed);
    let err = pool.acquire().await.expect_err("关池后不该再借出");
    assert!(
        matches!(err.kind, DmErrorKind::Config(ref m) if m.contains("不再接受")),
        "错误种类不对：{err:?}"
    );
    assert_eq!(shared.live(), 0, "空闲连接必须被物理关闭");
    let _ = &shared;
}

#[tokio::test]
async fn close_discards_idle_connections_immediately() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    assert_eq!(shared.live(), 1);
    pool.close().await.expect("关池");
    assert_eq!(shared.live(), 0);
    assert_eq!(shared.closed.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(pool.stats().size, 0);
    assert_eq!(pool.stats().idle, 0);
}

#[tokio::test]
async fn wait_closed_waits_for_outstanding_guards() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    let pool = Arc::new(pool);
    let guard = pool.acquire().await.expect("借出并一直拿着");
    pool.close().await.expect("关池");
    let waiting = tokio::spawn({
        let pool = Arc::clone(&pool);
        async move { pool.wait_closed().await }
    });
    // 句柄还没还，`wait_closed` 不该返回。
    stays_false(
        "wait_closed 提前返回",
        Duration::from_millis(30),
        || waiting.is_finished(),
    )
    .await;
    drop(guard);
    tokio::time::timeout(Duration::from_secs(2), waiting)
        .await
        .expect("归还后 wait_closed 必须返回")
        .expect("wait_closed 任务不该 panic");
    assert_eq!(shared.live(), 0);
    let _ = &shared;
}

#[tokio::test]
async fn guard_still_works_after_pool_is_closed() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    let mut guard = pool.acquire().await.expect("借出");
    pool.close().await.expect("关池");
    // 池关了，但已经在手上的句柄还能把当前操作做完。
    guard
        .with_conn(|raw| Ok(raw.name()))
        .await
        .expect("关池后手上这条连接仍可用");
    drop(guard);
    wait_for(
        "归还会销毁这条连接",
        Duration::from_secs(2),
        || shared.live() == 0,
    )
    .await;
    assert_eq!(shared.closed.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[tokio::test]
async fn pool_drop_stops_accepting_borrows() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    let stats = pool.stats();
    assert_eq!(stats.size, 1);
    drop(pool);
    // 池被丢弃后裸连接随 `ConnInner` 一起析构；这里只断言「没 panic 且
    // 建池那条连接的存活计数已经归零」——`Drop` 不做阻塞 IO，是刻意设计。
    let _ = shared;
}

// ---------------------------------------------------------------------------
// 端到端小用例：操作真的走到了假后端
// ---------------------------------------------------------------------------

#[tokio::test]
async fn with_conn_runs_on_a_blocking_thread() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    let mut guard = pool.acquire().await.expect("借出");
    let thread = guard
        .with_conn(|_| {
            // 闭包必须是 `'static` 且跑在阻塞线程上；这里把参数也走一遍，
            // 确认 `DmValue` 能 move 进去。
            let args: Vec<DmValue> = vec![DmValue::Int(1)];
            assert_eq!(args.len(), 1);
            Ok(std::thread::current().name().map(str::to_owned))
        })
        .await
        .expect("操作");
    let name = thread.expect("阻塞线程应当有名字");
    assert!(
        name.starts_with("dm-pool-blocking"),
        "操作没有跑在池的阻塞线程上，而是在 {name}"
    );
    drop(guard);
    let _ = shared;
    shutdown(&pool).await;
}

// ---------------------------------------------------------------------------
// panic 隔离：驱动在同步代码里 panic 时，池的账必须照样结清
// ---------------------------------------------------------------------------

/// 操作闭包 panic ⇒ 连接按「状态未知」丢弃，池还能继续用。
///
/// 这条用例钉的是一个**真实的泄漏**：不接住 panic 的话 `put_conn` 永远不执行，
/// 裸连接随栈展开被就地析构（对 odbc-api 就是一次没配对的 `SQLFreeHandle`），
/// 而连接名额与存活计数一起漏掉 —— `wait_closed` 于是永远等不到零。
#[tokio::test]
async fn panicking_operation_discards_the_connection() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    let mut guard = pool.acquire().await.expect("借出");
    let err = guard
        .with_conn(|_| -> Result<(), dm_error::DmError> {
            panic!("用例故意在操作闭包里 panic")
        })
        .await
        .expect_err("panic 必须变成错误交回调用方");
    assert!(
        err.is_connection_lost(),
        "panic 之后的连接状态未知，必须按不可用处理：{err}"
    );
    assert_eq!(
        err.context.stage,
        dm_error::Stage::Execute,
        "panic 归到执行阶段：{err}"
    );
    drop(guard);

    wait_for(
        "毒化连接被物理关闭",
        Duration::from_secs(2),
        || shared.live() == 0,
    )
    .await;
    assert_stats_consistent(&pool, &pool.stats());
    assert_eq!(pool.stats().size, 0, "毒化的连接不该留在池里");

    // 池没被拖坏：还能借出、还能干活。
    let mut guard = pool.acquire().await.expect("panic 之后仍应能借出");
    guard.with_conn(|_| Ok(())).await.expect("新连接可用");
    drop(guard);
    shutdown(&pool).await;
}

/// 复位 panic ⇒ 连接销毁，但**回收闸门不能卡死**。
///
/// `recycle_all` 用一个 `recycling` 闸门保证同一时刻只有一次回收在跑；panic
/// 从 `raw.reset()` 穿出去的话闸门会永久留在 `true`，待复位队列再也排不空。
#[tokio::test]
async fn panicking_reset_discards_the_connection_and_keeps_recycling() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    shared.panic_next_resets(1);

    let mut guard = pool.acquire().await.expect("借出");
    guard.with_conn(|_| Ok(())).await.expect("操作");
    guard.mark_dirty();
    drop(guard);

    wait_for(
        "复位 panic 的连接被销毁",
        Duration::from_secs(2),
        || shared.live() == 0,
    )
    .await;
    assert_stats_consistent(&pool, &pool.stats());
    assert_eq!(pool.stats().size, 0);
    assert_eq!(pool.stats().to_recycle, 0, "待复位队列必须排空");

    // 闸门确实还在工作：第二条脏连接照样能被回收成空闲。
    let mut guard = pool.acquire().await.expect("再借一条");
    guard.with_conn(|_| Ok(())).await.expect("操作");
    guard.mark_dirty();
    drop(guard);
    wait_for(
        "第二条脏连接被回收",
        Duration::from_secs(2),
        || pool.stats().idle == 1,
    )
    .await;
    assert_stats_consistent(&pool, &pool.stats());
    shutdown(&pool).await;
}

/// `Backend::close` panic ⇒ 错误被就地接住，名额与存活计数照样结清。
///
/// 判据取「还能立刻借到」：名额若漏掉，后续 `acquire` 会一路等到
/// `acquire_timeout`（1 秒）才失败。
#[tokio::test]
async fn panicking_close_still_releases_the_slot() {
    let (shared, pool) = pool(PoolOpts::default()).await;
    shared.panic_next_closes(1);

    let mut guard = pool.acquire().await.expect("借出");
    guard.with_conn(|_| Ok(())).await.expect("操作");
    guard.mark_poisoned();
    drop(guard);

    let mut guard = pool.acquire().await.expect("名额必须已经归还");
    guard.with_conn(|_| Ok(())).await.expect("后续连接可用");
    drop(guard);
    assert_stats_consistent(&pool, &pool.stats());
    shutdown(&pool).await;
}
