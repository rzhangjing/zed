//! `dm-pool` 集成测试的公共夹具（L4 层，离线假后端，不碰 ODBC）。
//!
//! 这里做的事只有三件：
//!
//! 1. 用 `#[path]` 把 `src/testing.rs` 的假后端编进来——集成测试是**独立 crate**，
//!    看不见 `#[cfg(test)]` 的模块；
//! 2. 给出确定性的池参数（默认 `recycle_interval` 一小时，免得后台维护在用例
//!    中途插一脚把计数搅乱；要测维护的用例自己调小它）；
//! 3. 提供「等条件成立」的异步助手——并发用例里等固定时长是不稳的。

#![allow(dead_code)]

#[path = "../../src/testing.rs"]
pub mod fake;

use std::sync::Arc;
use std::time::{Duration, Instant};

use dm_core::DmConfig;
use dm_pool::{DmPool, ExecutorConfig, PoolState, PoolStats};

pub use fake::{FakeBackend, FakeShared};

/// 集成测试用的池参数。默认值刻意「安静」：不探活、不回收、不保活。
#[derive(Debug, Clone)]
pub struct PoolOpts {
    pub max_size: usize,
    pub min_idle: usize,
    pub acquire_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_lifetime: Duration,
    pub health_check_interval: Duration,
    pub test_on_acquire: bool,
    pub recycle_interval: Duration,
    pub reuse_after_cancel: bool,
}

impl Default for PoolOpts {
    fn default() -> Self {
        Self {
            max_size: 4,
            min_idle: 0,
            acquire_timeout: Duration::from_secs(1),
            idle_timeout: Duration::from_secs(60),
            max_lifetime: Duration::from_secs(300),
            health_check_interval: Duration::from_secs(60),
            test_on_acquire: false,
            // 一小时：测试跑不到，后台维护相当于不存在。
            recycle_interval: Duration::from_secs(3600),
            reuse_after_cancel: false,
        }
    }
}

impl PoolOpts {
    pub fn max_size(mut self, n: usize) -> Self {
        self.max_size = n;
        self
    }

    pub fn min_idle(mut self, n: usize) -> Self {
        self.min_idle = n;
        self
    }

    pub fn acquire_timeout(mut self, d: Duration) -> Self {
        self.acquire_timeout = d;
        self
    }

    pub fn idle_timeout(mut self, d: Duration) -> Self {
        self.idle_timeout = d;
        self
    }

    pub fn max_lifetime(mut self, d: Duration) -> Self {
        self.max_lifetime = d;
        self
    }

    pub fn health_check_interval(mut self, d: Duration) -> Self {
        self.health_check_interval = d;
        self
    }

    pub fn test_on_acquire(mut self, on: bool) -> Self {
        self.test_on_acquire = on;
        self
    }

    pub fn recycle_interval(mut self, d: Duration) -> Self {
        self.recycle_interval = d;
        self
    }

    pub fn reuse_after_cancel(mut self, on: bool) -> Self {
        self.reuse_after_cancel = on;
        self
    }
}

/// 一份指向假后端的配置。
pub fn config(shared: &Arc<FakeShared>, opts: &PoolOpts) -> DmConfig {
    let mut cfg = fake::fake_config(shared);
    cfg.pool.max_size = opts.max_size;
    cfg.pool.min_idle = opts.min_idle;
    cfg.pool.acquire_timeout = opts.acquire_timeout;
    cfg.pool.idle_timeout = opts.idle_timeout;
    cfg.pool.max_lifetime = opts.max_lifetime;
    cfg.pool.health_check_interval = opts.health_check_interval;
    cfg.pool.test_on_acquire = opts.test_on_acquire;
    cfg.pool.recycle_interval = opts.recycle_interval;
    cfg.pool.reuse_after_cancel = opts.reuse_after_cancel;
    cfg
}

/// 起一个假后端连接池。
pub async fn open(shared: &Arc<FakeShared>, opts: PoolOpts) -> DmPool<FakeBackend> {
    let cfg = config(shared, &opts);
    // 阻塞线程池按 max_size 配，但至少 2 条线程：否则「一条连接卡住」的
    // 用例会连归还动作都排不上队。
    let exec = ExecutorConfig::for_pool(opts.max_size.max(2));
    DmPool::connect_with(cfg, exec)
        .await
        .expect("假后端建池不该失败")
}

/// 走「显式关闭 + 等干净」的标准收尾。
///
/// 等待加了 5 秒上限：`wait_closed` 只要还有借出未还的句柄就会一直等下去，
/// 而「忘了 drop 句柄」是写测试时最容易犯的错。让它**响亮地失败**，
/// 而不是把整个测试进程挂死。
pub async fn shutdown(pool: &DmPool<FakeBackend>) {
    pool.close().await.expect("关池不该失败");
    if tokio::time::timeout(Duration::from_secs(5), pool.wait_closed())
        .await
        .is_err()
    {
        panic!(
            "wait_closed 超时：还有借出未还的句柄没 drop（当前指标 {:?}）",
            pool.stats()
        );
    }
}

// ---------------------------------------------------------------------------
// 异步等待助手
// ---------------------------------------------------------------------------

/// 每 1 ms 检查一次，最多等 `budget`；超时就把「在等什么」打进 panic 消息。
///
/// 用它而不是 `sleep(固定时长)`：并发用例里 J 固定 sleep 不是太短（偶发失败）
/// 就是太长（测试变慢），等待条件成立才既快又稳。
pub async fn wait_for(what: &str, budget: Duration, mut pred: impl FnMut() -> bool) {
    let deadline = Instant::now() + budget;
    while !pred() {
        if Instant::now() >= deadline {
            panic!("等「{what}」超时（{:?}）", budget);
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// 断言一段时间内条件**始终**不成立（用来验证「不该发生的事没发生」）。
pub async fn stays_false(what: &str, window: Duration, mut pred: impl FnMut() -> bool) {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        assert!(!pred(), "「{what}」本不该发生");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// 池指标的一致性断言，几乎每个用例结尾都值得跑一次。
pub fn assert_stats_consistent(pool: &DmPool<FakeBackend>, stats: &PoolStats) {
    assert_eq!(
        stats.size,
        stats.idle + stats.to_recycle + stats.in_use + stats.creating,
        "size 必须等于四部分之和：{stats:?}"
    );
    assert!(
        stats.size <= stats.max_size,
        "存活连接数 {} 超过 max_size {}：{stats:?}",
        stats.size,
        stats.max_size
    );
    assert!(stats.in_use <= stats.max_size, "在飞数超过上限：{stats:?}");
    let _ = pool;
}

/// 池是否已经进入终态。
pub fn is_closed(pool: &DmPool<FakeBackend>) -> bool {
    pool.state() == PoolState::Closed
}
