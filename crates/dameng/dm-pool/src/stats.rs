//! 池状态与指标（设计方案 §6.9、§5.7）。
//!
//! 对外只有两个类型：[`PoolState`] 与 [`PoolStats`]——两者都在 §6.12 的冻结清单里，
//! 因此字段**只增不改**（`#[non_exhaustive]` 让下游无法用字面量构造，只能读）。
//!
//! 内部用一组原子计数器（[`PoolCounters`]）累加，快照时一次性读出来。
//! 所有计数都是 `Relaxed`：指标只求最终一致，不需要参与同步——
//! 用更强的序换来的只是「指标更整齐」，却给每条热路径加上栅栏。

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// 池的对外状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PoolState {
    /// 正常运行，可借出连接。
    Running,
    /// 正在关闭：**不再接受**新的借出请求，已借出的连接仍可用。
    Closing,
    /// 已关闭：空闲连接已全部物理关闭，`stats().size` 已归零。
    Closed,
}

impl PoolState {
    pub(crate) fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::Closing,
            2 => Self::Closed,
            _ => Self::Running,
        }
    }

    pub(crate) fn as_u8(self) -> u8 {
        match self {
            Self::Running => 0,
            Self::Closing => 1,
            Self::Closed => 2,
        }
    }

    /// 稳定的中文标签，用于日志与快照。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Closing => "Closing",
            Self::Closed => "Closed",
        }
    }
}

impl std::fmt::Display for PoolState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 池指标快照。
///
/// `size`/`idle`/`in_use`/`waiting` 是**瞬时量**，其余是**累计量**（自池创建起）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PoolStats {
    /// 当前物理连接总数 = `idle + to_recycle + in_use + creating`。
    pub size: usize,
    /// 空闲且**可直接借出**的连接数。
    pub idle: usize,
    /// 已归还但**等待复位**的连接数（复位成功才会进 `idle`）。
    pub to_recycle: usize,
    /// 已借出（含正在归还途中）的连接数。
    pub in_use: usize,
    /// 正在建连的连接数（单飞期间最多 1）。
    pub creating: usize,
    /// 正在等待借出结果的调用方数量。
    pub waiting: usize,
    /// 配置上限（快照里带上，省得调用方再去问 `config()`）。
    pub max_size: usize,
    /// 保活下限。
    pub min_idle: usize,

    /// 累计建连成功次数。
    pub created: u64,
    /// 累计建连失败次数。
    pub connect_failed: u64,
    /// 累计物理关闭次数（超期、探活失败、毒化、池关闭）。
    pub closed: u64,
    /// 累计成功借出次数。
    pub acquired: u64,
    /// 累计归还次数（无论干净还是脏）。
    pub released: u64,
    /// 累计因 `acquire_timeout` 失败的次数。
    pub timed_out: u64,

    /// 累计复位后成功复用的次数。
    pub recycled: u64,
    /// 累计因 `idle_timeout` 被回收关闭的次数。
    pub expired: u64,
    /// 累计因 `max_lifetime` 到期被轮换关闭的次数。
    pub lifetime_rotated: u64,
    /// 累计探活判死而淘汰的次数。
    pub health_check_failed: u64,
    /// 累计复位失败的次数（失败即物理关闭）。
    pub recycle_failed: u64,
    /// 累计因取消/超时被判为不可复用（**D11**）而销毁的次数。
    pub recycled_cancelled: u64,
}

impl PoolStats {
    /// 供 `tracing` span 用的一行摘要。
    pub(crate) fn summary(&self) -> String {
        format!(
            "size={} idle={} recycle={} in_use={} waiting={}",
            self.size, self.idle, self.to_recycle, self.in_use, self.waiting
        )
    }
}

/// 内部累计计数器。字段全部 `pub(crate)`，由池的各条路径直接 `fetch_add`。
#[derive(Debug, Default)]
pub(crate) struct PoolCounters {
    pub(crate) created: AtomicU64,
    pub(crate) connect_failed: AtomicU64,
    pub(crate) closed: AtomicU64,
    pub(crate) acquired: AtomicU64,
    pub(crate) released: AtomicU64,
    pub(crate) timed_out: AtomicU64,
    pub(crate) recycled: AtomicU64,
    pub(crate) expired: AtomicU64,
    pub(crate) lifetime_rotated: AtomicU64,
    pub(crate) health_check_failed: AtomicU64,
    pub(crate) recycle_failed: AtomicU64,
    pub(crate) recycled_cancelled: AtomicU64,
    pub(crate) waiting: AtomicUsize,
}

impl PoolCounters {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn add(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// 把瞬时量（由调用方从各自的锁下读出）与累计量合并成快照。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn snapshot(
        &self,
        idle: usize,
        to_recycle: usize,
        in_use: usize,
        creating: usize,
        max_size: usize,
        min_idle: usize,
    ) -> PoolStats {
        let size = idle + to_recycle + in_use + creating;
        PoolStats {
            size,
            idle,
            to_recycle,
            in_use,
            creating,
            waiting: self.waiting.load(Ordering::Relaxed),
            max_size,
            min_idle,
            created: self.created.load(Ordering::Relaxed),
            connect_failed: self.connect_failed.load(Ordering::Relaxed),
            closed: self.closed.load(Ordering::Relaxed),
            acquired: self.acquired.load(Ordering::Relaxed),
            released: self.released.load(Ordering::Relaxed),
            timed_out: self.timed_out.load(Ordering::Relaxed),
            recycled: self.recycled.load(Ordering::Relaxed),
            expired: self.expired.load(Ordering::Relaxed),
            lifetime_rotated: self.lifetime_rotated.load(Ordering::Relaxed),
            health_check_failed: self.health_check_failed.load(Ordering::Relaxed),
            recycle_failed: self.recycle_failed.load(Ordering::Relaxed),
            recycled_cancelled: self.recycled_cancelled.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_through_u8() {
        for state in [PoolState::Running, PoolState::Closing, PoolState::Closed] {
            assert_eq!(PoolState::from_u8(state.as_u8()), state);
            assert!(!state.as_str().is_empty());
        }
        // 未知字节按 Running 处理：状态是「尽力而为」的观测值，不该让池 panic。
        assert_eq!(PoolState::from_u8(200), PoolState::Running);
    }

    #[test]
    fn snapshot_sums_size_from_parts() {
        let counters = PoolCounters::new();
        PoolCounters::add(&counters.created);
        PoolCounters::add(&counters.created);
        PoolCounters::add(&counters.timed_out);
        counters.waiting.store(3, Ordering::Relaxed);

        let stats = counters.snapshot(2, 1, 4, 1, 10, 2);
        assert_eq!(stats.size, 8, "size 必须等于四部分之和");
        assert_eq!(stats.idle, 2);
        assert_eq!(stats.to_recycle, 1);
        assert_eq!(stats.in_use, 4);
        assert_eq!(stats.creating, 1);
        assert_eq!(stats.waiting, 3);
        assert_eq!(stats.max_size, 10);
        assert_eq!(stats.min_idle, 2);
        assert_eq!(stats.created, 2);
        assert_eq!(stats.timed_out, 1);
        assert_eq!(stats.recycled, 0);
        assert!(stats.summary().contains("in_use=4"));
    }

    #[test]
    fn default_stats_are_zeroed() {
        let stats = PoolStats::default();
        assert_eq!(stats.size, 0);
        assert_eq!(stats.acquired, 0);
        assert_eq!(
            stats.summary(),
            "size=0 idle=0 recycle=0 in_use=0 waiting=0"
        );
    }
}
