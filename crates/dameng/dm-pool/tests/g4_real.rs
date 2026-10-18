//! G4 真机门禁：并发、取消、会话泄漏、句柄配对（需要达梦 DM8 实例）。
//!
//! 与 `pool.rs` / `concurrency.rs` 的分工：那两组用离线假后端钉「池的状态机
//! 与核算」，本组用**真达梦**钉「池没在服务端留下垃圾、没把 `max_size` 顶破」。
//! 判据来自设计方案 §10.4（G4 出口）。
//!
//! 跑法（host 是 gnu，`dlltool.exe` 不在 PATH，必须显式 msvc）：
//!
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc test -p dm-pool --test g4_real -- --include-ignored --test-threads=1 --nocapture
//! ```
//!
//! ## 本组**没有**覆盖的 G4 判据（如实记录，别当已验）
//!
//! - `dhat` RSS 平坦：需要在测试进程里挂 `dhat` 全局分配器（要加依赖），本轮未做。
//! - 30 分钟长稳：本组用 `g4_soak`（默认 60 s，可用 `G4_SOAK_SECS` 调长）替代，
//!   它检查的是同一类不变量（`size ≤ max_size`、`stats` 一致、无错误累积）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use dm_core::backend::{ExecOutcome, FetchState};
use dm_core::{Backend, DmConfig, DmValue, PoolConfig, RowBuf};
use dm_error::{DmError, DmErrorKind};
// 默认后端（dpi feature 打开时是 `dm_dpi::DpiBackend`）。
use dm_pool::DefaultBackend as OdbcBackend;
use dm_pool::DmPool;

/// 真机默认池参数：小而紧，好把「顶破上限」暴露成失败而不是掩盖成排队。
const MAX_SIZE: usize = 8;

// ---------------------------------------------------------------------------
// 配置：没配环境就跳过
// ---------------------------------------------------------------------------

/// 从环境读真机配置：`DM_TEST_DSN` 优先，否则 `DM_SERVER` 系。
fn base_config() -> Option<DmConfig> {
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

/// 配好池参数的真机配置；返回 `None` 表示本机没配达梦（用例跳过）。
fn g4_config(max_size: usize, tune: impl FnOnce(&mut PoolConfig)) -> Option<DmConfig> {
    let Some(mut cfg) = base_config() else {
        eprintln!("跳过：未配置 DM_TEST_DSN / DM_SERVER（G4 真机用例只在真库上跑，见文件头注释）");
        return None;
    };
    cfg.pool = PoolConfig {
        max_size,
        min_idle: 0,
        ..PoolConfig::default()
    };
    tune(&mut cfg.pool);
    Some(cfg)
}

// ---------------------------------------------------------------------------
// 跑在阻塞线程上的一条查询
// ---------------------------------------------------------------------------

/// `select 1 from dual`，走 `Backend` 的完整路径（prepare → execute → fetch → 关游标）。
///
/// 刻意**不用** `dm-odbc` 的高层便捷方法：`dm-query` 以后走的就是这条 `Backend`
/// 路径，G4 要验的正是它。
fn select_one(raw: &mut OdbcBackend) -> Result<(), DmError> {
    let mut stmt = raw.prepare("select 1 from dual")?;
    let outcome = raw.execute(&mut stmt, "")?;
    match outcome {
        ExecOutcome::ResultSet { columns } => {
            let mut buf = RowBuf::new(columns.len());
            loop {
                match raw.fetch(&mut stmt, &mut buf)? {
                    FetchState::Rows { .. } => {}
                    FetchState::Exhausted => break,
                    other => {
                        return Err(DmError::new(DmErrorKind::Config(format!(
                            "未知的 FetchState 变体：{other:?}"
                        ))));
                    }
                }
            }
        }
        other => {
            return Err(DmError::new(DmErrorKind::Config(format!(
                "`select 1 from dual` 应当回结果集，实得：{other:?}"
            ))));
        }
    }
    raw.close_cursor(&mut stmt)
}

fn value_as_i64(value: &DmValue) -> Result<i64, DmError> {
    let bad = || {
        DmError::new(DmErrorKind::Config(format!(
            "无法把会话数解释成整数：{value:?}"
        )))
    };
    match value {
        DmValue::TinyInt(v) => Ok(i64::from(*v)),
        DmValue::SmallInt(v) => Ok(i64::from(*v)),
        DmValue::Int(v) => Ok(i64::from(*v)),
        DmValue::BigInt(v) => Ok(*v),
        DmValue::Text(s) => s.trim().parse::<i64>().map_err(|_| bad()),
        _ => Err(bad()),
    }
}

/// 用一条**独立的临时连接**数 `v$sessions`（自己开、自己关，不污染前后比较）。
async fn session_count(cfg: &DmConfig) -> Result<i64, DmError> {
    let cfg = cfg.clone();
    tokio::task::spawn_blocking(move || {
        let mut raw = OdbcBackend::connect(&cfg)?;
        let queried = (|| -> Result<i64, DmError> {
            let mut stmt = raw.prepare("select to_char(count(*)) from v$sessions")?;
            let outcome = raw.execute(&mut stmt, "")?;
            let mut first = None;
            if let ExecOutcome::ResultSet { columns } = outcome {
                let mut buf = RowBuf::new(columns.len());
                loop {
                    match raw.fetch(&mut stmt, &mut buf)? {
                        FetchState::Rows { .. } => {
                            if first.is_none() {
                                first = buf.row(0).and_then(|row| row.first()).cloned();
                            }
                        }
                        FetchState::Exhausted => break,
                        other => {
                            return Err(DmError::new(DmErrorKind::Config(format!(
                                "未知的 FetchState 变体：{other:?}"
                            ))));
                        }
                    }
                }
            }
            raw.close_cursor(&mut stmt)?;
            let value = first.ok_or_else(|| {
                DmError::new(DmErrorKind::Config("v$sessions 数不出行来".to_owned()))
            })?;
            value_as_i64(&value)
        })();
        raw.close();
        queried
    })
    .await
    .expect("spawn_blocking 不该 panic")
}

/// 等条件成立；超时 panic 并打印在等什么。
async fn wait_for(what: &str, budget: Duration, mut pred: impl FnMut() -> bool) {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if pred() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("等不到「{what}」（预算 {budget:?}）");
}

/// `size == idle + to_recycle + in_use + creating` 且 `size <= max_size`。
fn assert_stats_consistent(pool: &DmPool<OdbcBackend>) {
    let s = pool.stats();
    assert_eq!(
        s.size,
        s.idle + s.to_recycle + s.in_use + s.creating,
        "池核算不自洽：{s:?}"
    );
    assert!(s.size <= s.max_size, "池超出上限：{s:?}");
}

// ---------------------------------------------------------------------------
// G4-1：256 并发 × 1000 次 select 1（max_size = 8）
// ---------------------------------------------------------------------------

/// G4 判据：峰值 `in_use ≤ max_size`、无 panic、无 `PoolTimeout`。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "真机 G4 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn g4_256_concurrent_tasks_1000_ops_each() {
    const TASKS: usize = 256;
    const OPS: usize = 1000;

    let Some(cfg) = g4_config(MAX_SIZE, |_| {}) else {
        return;
    };
    let before = session_count(&cfg).await.expect("数会话数（基线）");
    let pool = Arc::new(DmPool::connect(cfg.clone()).await.expect("建池失败"));

    let start = Instant::now();
    let mut tasks = Vec::with_capacity(TASKS);
    for _ in 0..TASKS {
        let pool = Arc::clone(&pool);
        tasks.push(tokio::spawn(async move {
            for _ in 0..OPS {
                let mut guard = pool.acquire().await?;
                guard.with_conn(select_one).await?;
            }
            Ok::<_, DmError>(())
        }));
    }
    for task in tasks {
        task.await
            .expect("并发任务不该 panic")
            .unwrap_or_else(|err| {
                panic!("并发任务失败（不该出现 PoolTimeout）: {err}");
            });
    }
    let elapsed = start.elapsed();

    let stats = pool.stats();
    assert_eq!(stats.acquired, (TASKS * OPS) as u64, "{stats:?}");
    assert_eq!(stats.timed_out, 0, "不该有 PoolTimeout：{stats:?}");
    assert_eq!(stats.in_use, 0, "全部归还后 in_use 应当归零：{stats:?}");
    assert_eq!(stats.waiting, 0, "不该还有等待者：{stats:?}");
    assert_stats_consistent(&pool);
    assert!(
        stats.created <= MAX_SIZE as u64 + stats.closed,
        "句柄申请/释放不配对（建连数异常）：{stats:?}"
    );

    let during = session_count(&cfg).await.expect("数会话数（满载）");
    assert!(
        during - before <= MAX_SIZE as i64,
        "会话数比基线多了 {}（max_size = {MAX_SIZE}）",
        during - before
    );

    pool.close().await.expect("关池");
    pool.wait_closed().await;
    let after = session_count(&cfg).await.expect("数会话数（关池后）");
    assert!(
        after <= before,
        "关池后会话数没回到基线：before = {before}, after = {after}"
    );

    eprintln!(
        "G4-1：{TASKS} 任务 × {OPS} 次 = {} 次 select 1，用时 {:.2}s（{:.1} op/s），\
         建连 {} 次、会话数 {before} → {during} → {after}",
        TASKS * OPS,
        elapsed.as_secs_f64(),
        (TASKS * OPS) as f64 / elapsed.as_secs_f64(),
        stats.created
    );
}

// ---------------------------------------------------------------------------
// G4-2：future 反复超时被 drop，名额与会话都不许泄漏
// ---------------------------------------------------------------------------

/// G4 判据：`timeout(1ms)` 反复 drop 之后 `in_use` 归零、等待者不残留。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "真机 G4 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn g4_dropped_timeout_futures_do_not_leak_slots() {
    const ROUNDS: usize = 200;

    let Some(cfg) = g4_config(2, |pool| {
        pool.acquire_timeout = Duration::from_millis(50);
    }) else {
        return;
    };
    let before = session_count(&cfg).await.expect("数会话数（基线）");
    let pool = DmPool::connect(cfg.clone()).await.expect("建池失败");

    // 把两个名额都占住，后续 acquire 必然要等。
    let mut first = pool.acquire().await.expect("借第一条");
    let mut second = pool.acquire().await.expect("借第二条");
    first.with_conn(select_one).await.expect("第一条可用");
    second.with_conn(select_one).await.expect("第二条可用");

    let mut timeouts = 0;
    for _ in 0..ROUNDS {
        match tokio::time::timeout(Duration::from_millis(1), pool.acquire()).await {
            Err(_elapsed) => timeouts += 1,
            Ok(Ok(_guard)) => panic!("名额没占住：不该借到连接"),
            Ok(Err(err)) => panic!("借出应当被超时打断，实得错误：{err}"),
        }
    }
    assert_eq!(timeouts, ROUNDS);
    let stats = pool.stats();
    assert_eq!(stats.waiting, 0, "被 drop 的等待者必须清干净：{stats:?}");
    assert_eq!(stats.in_use, 2, "{stats:?}");
    assert_eq!(stats.timed_out, 0, "客户端超时不该记成池超时：{stats:?}");

    drop(first);
    drop(second);
    wait_for("名额全部归还", Duration::from_secs(5), || {
        pool.stats().in_use == 0
    })
    .await;
    assert_stats_consistent(&pool);

    let during = session_count(&cfg).await.expect("数会话数（收尾）");
    assert!(during - before <= 2, "会话数泄漏：+{}", during - before);

    pool.close().await.expect("关池");
    pool.wait_closed().await;
}

// ---------------------------------------------------------------------------
// G4-3：长操作超时 → `DmErrorKind::Timeout` 且连接被销毁
// ---------------------------------------------------------------------------

/// G4 判据：`timeout(200ms)` 打断长操作 → 报 `DmErrorKind::Timeout`，
/// 并且那条连接**被物理销毁**（不能带着未知状态回到池里）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "真机 G4 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn g4_long_operation_timeout_destroys_the_connection() {
    let Some(cfg) = g4_config(2, |_| {}) else {
        return;
    };
    let pool = DmPool::connect(cfg).await.expect("建池失败");

    let mut guard = pool.acquire().await.expect("借连接");
    let id = guard.id();
    let err = guard
        .with_timeout(Duration::from_millis(200), |raw: &mut OdbcBackend| {
            // 用「阻塞线程上睡 800 ms」模拟一次长 ODBC 调用：真实的长查询
            // 时长不可控（会随数据量变化），而 G4 要钉的是**池侧的处置**。
            std::thread::sleep(Duration::from_millis(800));
            select_one(raw)
        })
        .await
        .expect_err("200 ms 超时必须报错");
    assert!(
        matches!(err.kind, DmErrorKind::Timeout),
        "应当是 Timeout，实得：{err:?}"
    );
    drop(guard);

    // 闭包要睡满 800 ms 才返回，所以这里等一段确定的时间再断言——比轮询超时
    // 更好用：失败时能把整份 `stats` 打出来（第一版轮询断在「等不到」上，
    // 看不出是「没销毁」还是「销毁了没记账」）。
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let stats = pool.stats();
    assert!(stats.closed >= 1, "超时后的连接应当被物理销毁：{stats:?}");
    assert!(stats.recycled <= 1, "被取消的连接不该被回收复用：{stats:?}");
    assert_eq!(stats.recycled_cancelled, 1, "取消销毁要记进指标：{stats:?}");
    assert_eq!(stats.in_use, 0, "{stats:?}");

    // 池必须还能正常干活（销毁一条不等于池坏了）。
    let mut guard = pool.acquire().await.expect("销毁后仍应能借出");
    assert_ne!(guard.id(), id, "不该把被销毁的连接再借出来");
    guard.with_conn(select_one).await.expect("新连接可用");
    drop(guard);
    assert_stats_consistent(&pool);

    pool.close().await.expect("关池");
    pool.wait_closed().await;
}

// ---------------------------------------------------------------------------
// G4-4：连接复用（句柄不随操作数增长）
// ---------------------------------------------------------------------------

/// G4 判据：句柄申请/释放配对、无增长——500 次串行操作只该建 `1` 条连接。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "真机 G4 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn g4_sequential_operations_reuse_one_connection() {
    const OPS: usize = 500;

    let Some(cfg) = g4_config(4, |_| {}) else {
        return;
    };
    let pool = DmPool::connect(cfg).await.expect("建池失败");
    for _ in 0..OPS {
        let mut guard = pool.acquire().await.expect("借连接");
        guard.with_conn(select_one).await.expect("select 1");
    }
    let stats = pool.stats();
    assert_eq!(stats.created, 1, "串行操作不该重复建连：{stats:?}");
    assert_eq!(stats.closed, 0, "{stats:?}");
    assert_eq!(stats.acquired, OPS as u64, "{stats:?}");
    assert_eq!(stats.in_use, 0, "{stats:?}");
    assert_stats_consistent(&pool);

    pool.close().await.expect("关池");
    pool.wait_closed().await;
    assert_eq!(pool.stats().size, 0, "关池后不该还留着连接");
}

// ---------------------------------------------------------------------------
// G4-5：限时长稳（30 分钟判据的等比例替代）
// ---------------------------------------------------------------------------

/// G4 判据：长稳期间 `size ≤ max_size`、`stats` 自洽、无错误累积、无 panic。
///
/// 时长取 `G4_SOAK_SECS`（默认 60 s）；真正的 30 分钟门禁请把它调成 1800。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "真机 G4 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn g4_soak_keeps_the_pool_bounded() {
    let secs: u64 = std::env::var("G4_SOAK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);

    let Some(cfg) = g4_config(MAX_SIZE, |pool| {
        pool.recycle_interval = Duration::from_millis(200);
    }) else {
        return;
    };
    let before = session_count(&cfg).await.expect("数会话数（基线）");
    let pool = Arc::new(DmPool::connect(cfg.clone()).await.expect("建池失败"));

    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut rounds = 0u64;
    while Instant::now() < deadline {
        let mut tasks = Vec::with_capacity(16);
        for _ in 0..16 {
            let pool = Arc::clone(&pool);
            tasks.push(tokio::spawn(async move {
                let mut guard = pool.acquire().await?;
                guard.with_conn(select_one).await
            }));
        }
        for task in tasks {
            task.await.expect("长稳任务不该 panic").expect("select 1");
        }
        rounds += 1;

        let stats = pool.stats();
        assert_stats_consistent(&pool);
        assert_eq!(
            stats.in_use, 0,
            "第 {rounds} 轮收尾后有连接没归还：{stats:?}"
        );
        assert_eq!(stats.timed_out, 0, "{stats:?}");
        assert_eq!(stats.connect_failed, 0, "{stats:?}");
        assert_eq!(stats.recycle_failed, 0, "{stats:?}");
        assert_eq!(stats.health_check_failed, 0, "{stats:?}");
    }

    let during = session_count(&cfg).await.expect("数会话数（长稳后）");
    assert!(
        during - before <= MAX_SIZE as i64,
        "长稳期间会话数漂移：+{}",
        during - before
    );

    pool.close().await.expect("关池");
    pool.wait_closed().await;
    let after = session_count(&cfg).await.expect("数会话数（关池后）");
    assert!(
        after <= before,
        "关池后会话数没回到基线：{after} > {before}"
    );

    eprintln!(
        "G4-5：长稳 {secs}s / {rounds} 轮（每轮 16 次 select 1），建连 {} 次、\
         会话数 {before} → {during} → {after}",
        pool.stats().created
    );
}
