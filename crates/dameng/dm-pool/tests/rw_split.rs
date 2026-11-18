//! 读写分离路由器（`feature = "rw-split"`，设计方案 §6.10）的集成用例。
//!
//! `router.rs` 里已有的 18 条单测只覆盖**纯决策函数** `pick` 与驱动侧冲突检测
//! （`check_no_driver_splitting`）。本文件补上 [`Router`] 自己那层胶水：
//! 构造期的越界/冲突校验、策略切换、挑池与计数、`acquire_primary` 的
//! 「事务粘住主库」，以及 `Debug`/指标访问器。全部走 L4 假后端，离线可跑。
//!
//! 覆盖率背景：`cargo llvm-cov -p dm-pool --all-features` 曾测得 `router.rs`
//! 行覆盖率 63.96%——漏掉的 114 行几乎正好是 `Router` 的整个公开面
//! （因为此前没有任何测试构造过 `Router`）。本文件就是为这一段写的。
#![cfg(feature = "rw-split")]

mod common;

use std::sync::Arc;

use common::{FakeBackend, FakeShared, PoolOpts, shutdown};
use dm_core::DmConfig;
use dm_error::DmErrorKind;
use dm_pool::{DmPool, ExecutorConfig, Router, RwMode, RwStrategy};

/// 起一个假后端池（`Arc` 包一层，`Router` 要的是 `Arc<DmPool<B>>`）。
async fn open_pool(shared: &Arc<FakeShared>) -> Arc<DmPool<FakeBackend>> {
    Arc::new(common::open(shared, PoolOpts::default().max_size(2)).await)
}

/// 用一份**自定义配置**起池（冲突检测用例要往配置里塞 `RW_SEPARATE`）。
async fn open_cfg(cfg: DmConfig) -> Arc<DmPool<FakeBackend>> {
    let exec = ExecutorConfig::for_pool(2);
    Arc::new(
        DmPool::connect_with(cfg, exec)
            .await
            .expect("假后端建池不该失败"),
    )
}

/// 收尾：先 drop 掉 `Router` 与句柄，再逐个关池。
async fn close_all(pools: &[&Arc<DmPool<FakeBackend>>]) {
    for pool in pools {
        shutdown(pool).await;
    }
}

#[tokio::test]
async fn read_only_goes_to_a_replica_and_read_write_to_the_primary() {
    let (sh_p, sh_r) = (FakeShared::new(), FakeShared::new());
    let primary = open_pool(&sh_p).await;
    let replica = open_pool(&sh_r).await;
    let router = Router::new(
        Arc::clone(&primary),
        vec![Arc::clone(&replica)],
        RwStrategy::RoundRobin,
    )
    .expect("干净配置 + Percent 未越界，构造必须成功");

    assert!(
        Arc::ptr_eq(router.route(RwMode::ReadOnly), &replica),
        "只读请求应当命中备库"
    );
    assert!(
        Arc::ptr_eq(router.route(RwMode::ReadWrite), &primary),
        "写请求必须留在主库（规则 1）"
    );
    assert_eq!(router.stats().routed_replica, 1);
    assert_eq!(router.stats().routed_primary, 1);
    assert_eq!(router.stats().fallbacks, 0, "有备库时不算回退");

    drop(router);
    close_all(&[&primary, &replica]).await;
}

#[tokio::test]
async fn without_replicas_reads_fall_back_to_the_primary_and_are_counted() {
    let sh_p = FakeShared::new();
    let primary = open_pool(&sh_p).await;
    let router = Router::new(Arc::clone(&primary), Vec::new(), RwStrategy::RoundRobin)
        .expect("没有备库不是错误");

    for _ in 0..3 {
        assert!(
            Arc::ptr_eq(router.route(RwMode::ReadOnly), &primary),
            "规则 3：无备库时回退主库"
        );
    }
    let stats = router.stats();
    assert_eq!(stats.fallbacks, 3, "每次回退都要计数");
    assert_eq!(stats.routed_primary, 3);
    assert_eq!(stats.routed_replica, 0);

    drop(router);
    close_all(&[&primary]).await;
}

#[tokio::test]
async fn round_robin_rotates_over_replicas_through_the_router() {
    let (sh_p, sh_a, sh_b, sh_c) = (
        FakeShared::new(),
        FakeShared::new(),
        FakeShared::new(),
        FakeShared::new(),
    );
    let primary = open_pool(&sh_p).await;
    let (a, b, c) = (
        open_pool(&sh_a).await,
        open_pool(&sh_b).await,
        open_pool(&sh_c).await,
    );
    let router = Router::new(
        Arc::clone(&primary),
        vec![Arc::clone(&a), Arc::clone(&b), Arc::clone(&c)],
        RwStrategy::RoundRobin,
    )
    .unwrap();

    let picked: Vec<bool> = (0..6)
        .map(|i| {
            let expected = [&a, &b, &c][i % 3];
            Arc::ptr_eq(router.route(RwMode::ReadOnly), expected)
        })
        .collect();
    assert_eq!(
        picked,
        vec![true; 6],
        "轮转必须按 0/1/2 循环（序号从 0 开始）"
    );
    assert_eq!(router.stats().routed_replica, 6);
    assert_eq!(router.stats().routed_primary, 0);

    drop(router);
    close_all(&[&primary, &a, &b, &c]).await;
}

#[tokio::test]
async fn least_in_flight_uses_the_live_replica_loads() {
    let (sh_p, sh_a, sh_b) = (FakeShared::new(), FakeShared::new(), FakeShared::new());
    let primary = open_pool(&sh_p).await;
    let (a, b) = (open_pool(&sh_a).await, open_pool(&sh_b).await);
    let router = Router::new(
        Arc::clone(&primary),
        vec![Arc::clone(&a), Arc::clone(&b)],
        RwStrategy::LeastInFlight,
    )
    .unwrap();

    // 先给备库 0 压一条在飞连接，再让路由器挑：必须挑到备库 1。
    let held = a.acquire().await.expect("假后端借出不该失败");
    assert_eq!(router.replica_loads(), vec![1, 0], "loads 要读实时在飞数");
    assert!(
        Arc::ptr_eq(router.route(RwMode::ReadOnly), &b),
        "负载感知策略必须避开忙的备库"
    );
    drop(held);

    drop(router);
    close_all(&[&primary, &a, &b]).await;
}

#[tokio::test]
async fn random_never_leaves_a_replica_when_replicas_exist() {
    let (sh_p, sh_a, sh_b) = (FakeShared::new(), FakeShared::new(), FakeShared::new());
    let primary = open_pool(&sh_p).await;
    let (a, b) = (open_pool(&sh_a).await, open_pool(&sh_b).await);
    let router = Router::new(
        Arc::clone(&primary),
        vec![Arc::clone(&a), Arc::clone(&b)],
        RwStrategy::Random,
    )
    .unwrap()
    .with_seed(7);

    for _ in 0..16 {
        let target = router.route(RwMode::ReadOnly);
        assert!(
            Arc::ptr_eq(target, &a) || Arc::ptr_eq(target, &b),
            "Random 有备库时绝不回主库"
        );
    }
    // Random 有备库时绝不回主库，所以 16 次全部计入 routed_replica。
    let stats = router.stats();
    assert_eq!(stats.routed_replica, 16);
    assert_eq!(stats.routed_primary, 0);
    assert_eq!(stats.fallbacks, 0);

    drop(router);
    close_all(&[&primary, &a, &b]).await;
}

#[tokio::test]
async fn percent_out_of_range_is_rejected_at_construction_and_on_switch() {
    let sh_p = FakeShared::new();
    let primary = open_pool(&sh_p).await;

    let err = Router::new(Arc::clone(&primary), Vec::new(), RwStrategy::Percent(101))
        .expect_err("Percent(101) 必须被拒绝");
    match err.kind {
        DmErrorKind::Config(ref message) => {
            assert!(message.contains("越界"), "错误消息要说清越界：{message}");
        }
        other => panic!("越界应当是 Config，实际 {other:?}"),
    }

    let mut router = Router::new(Arc::clone(&primary), Vec::new(), RwStrategy::RoundRobin).unwrap();
    assert!(
        router.set_strategy(RwStrategy::Percent(101)).is_err(),
        "运行期换策略同样要校验"
    );
    assert_eq!(
        router.strategy(),
        RwStrategy::RoundRobin,
        "被拒绝之后策略不能变"
    );
    router
        .set_strategy(RwStrategy::Percent(50))
        .expect("合法比例应当接受");
    assert_eq!(router.strategy(), RwStrategy::Percent(50));
    assert_eq!(router.strategy().as_str(), "percent(50)");

    drop(router);
    close_all(&[&primary]).await;
}

#[tokio::test]
async fn driver_side_rw_separate_on_the_primary_rejects_the_router() {
    let sh = FakeShared::new();
    let mut cfg = common::fake::fake_config(&sh);
    cfg.rw_separate = Some(true);
    let primary = open_cfg(cfg).await;

    let err = Router::new(Arc::clone(&primary), Vec::new(), RwStrategy::RoundRobin)
        .expect_err("驱动侧已开读写分离时构造 Router 必须失败（R-12）");
    match err.kind {
        DmErrorKind::Config(ref message) => {
            assert!(
                message.contains("RW_SEPARATE"),
                "消息要点名关键字：{message}"
            );
        }
        other => panic!("应当是 Config，实际 {other:?}"),
    }
    assert!(
        err.diags.iter().any(|d| d.message.contains("R-12")),
        "诊断里要点出规则编号：{:?}",
        err.diags
    );

    drop(primary);
}

#[tokio::test]
async fn driver_side_rw_separate_on_a_replica_reports_which_replica() {
    let (sh_p, sh_r) = (FakeShared::new(), FakeShared::new());
    let primary = open_pool(&sh_p).await;
    let mut cfg = common::fake::fake_config(&sh_r);
    cfg.extra.insert("RW_SEPARATE".to_owned(), "ON".to_owned());
    let replica = open_cfg(cfg).await;

    let err = Router::new(
        Arc::clone(&primary),
        vec![Arc::clone(&replica)],
        RwStrategy::RoundRobin,
    )
    .expect_err("备库配置冲突也要拒绝");
    assert!(
        err.diags
            .iter()
            .any(|d| d.message.contains("第 0 个备库池")),
        "诊断要指出是哪个备库池冲突：{:?}",
        err.diags
    );

    drop(primary);
    drop(replica);
}

#[tokio::test]
async fn acquire_primary_pins_the_transaction_to_the_primary() {
    let (sh_p, sh_r) = (FakeShared::new(), FakeShared::new());
    let primary = open_pool(&sh_p).await;
    let replica = open_pool(&sh_r).await;
    let router = Router::new(
        Arc::clone(&primary),
        vec![Arc::clone(&replica)],
        RwStrategy::RoundRobin,
    )
    .unwrap();

    let guard = router.acquire_primary().await.expect("借出主库连接");
    assert_eq!(primary.stats().in_use, 1, "连接必须来自主库");
    assert_eq!(replica.stats().in_use, 0, "事务不能借备库");
    assert_eq!(router.stats().routed_primary, 1);
    drop(guard);

    // `acquire` 走的是同一个决策：只读借备库。
    let guard = router
        .acquire(RwMode::ReadOnly)
        .await
        .expect("借出备库连接");
    assert_eq!(replica.stats().in_use, 1);
    assert_eq!(primary.stats().in_use, 0);
    drop(guard);

    drop(router);
    close_all(&[&primary, &replica]).await;
}

#[tokio::test]
async fn accessors_and_debug_report_the_configuration() {
    let (sh_p, sh_a, sh_b) = (FakeShared::new(), FakeShared::new(), FakeShared::new());
    let primary = open_pool(&sh_p).await;
    let (a, b) = (open_pool(&sh_a).await, open_pool(&sh_b).await);
    let mut router = Router::new(
        Arc::clone(&primary),
        vec![Arc::clone(&a), Arc::clone(&b)],
        RwStrategy::LeastInFlight,
    )
    .unwrap();

    assert!(Arc::ptr_eq(router.primary(), &primary));
    assert_eq!(router.replicas().len(), 2);
    assert!(Arc::ptr_eq(&router.replicas()[0], &a));
    assert_eq!(router.strategy(), RwStrategy::LeastInFlight);
    assert_eq!(router.replica_loads(), vec![0, 0]);

    let stats = router.replica_stats();
    assert_eq!(stats.len(), 2, "每个备库一份快照");
    for snapshot in &stats {
        assert_eq!(snapshot.max_size, 2);
        assert_eq!(snapshot.in_use, 0);
    }

    let debug = format!("{router:?}");
    assert!(
        debug.contains("replicas: 2"),
        "Debug 要能看出几个备库：{debug}"
    );
    assert!(
        debug.contains("least-in-flight"),
        "Debug 要能看出策略：{debug}"
    );

    // 换策略到 `RoundRobin` 之后，决策路径也跟着换。
    router.set_strategy(RwStrategy::RoundRobin).unwrap();
    assert!(Arc::ptr_eq(router.route(RwMode::ReadOnly), &a));
    assert!(Arc::ptr_eq(router.route(RwMode::ReadOnly), &b));
    assert!(Arc::ptr_eq(router.route(RwMode::ReadWrite), &primary));

    drop(router);
    close_all(&[&primary, &a, &b]).await;
}
