//! 请求统计聚合。
//!
//! 聚合调度器的活动/尝试/成功/失败/取消计数，以及用于
//! 诊断与限流决策的逐服务器、逐类型分解计数器。
//!
//! 这些统计是 **纯计数器** —— 无 IO，无框架依赖。
//! `lib.rs` 中的 [`RequestScheduler`] 驱动更新这些计数器的状态转移。

use std::collections::HashMap;

use crate::RequestType;

/// 聚合的请求统计。
///
/// 记录一个请求从“被调度”到“激活/完成/失败/取消”各阶段的
/// 计数变迁，是调度器做限流决策与对外暴露诊断的核心状态：
/// - `attempted`：累计被调度的请求数；
/// - `active`：当前正在传输的请求数（可增可减）；
/// - `succeeded`/`failed`：已结束请求的成败计数；
/// - `cancelled_pending`/`cancelled_active`：待定态与活动态下的取消数；
/// - `active_ever`：曾被激活过的请求总数（单调递增）。
///
/// 在此之上额外维护逐服务器与逐类型的分解映射，以获得更
/// 丰富的诊断视角（例如定位某个服务器或某类请求的堆积）。
#[derive(Debug, Clone, Default)]
pub struct RequestStatistics {
    /// 已被尝试（已调度）的请求总数。
    pub attempted: u64,

    /// 当前活动请求的数量。
    pub active: u64,

    /// 在待定状态（从未激活）时被取消的请求数量。
    pub cancelled_pending: u64,

    /// 在活动状态时被取消的请求数量。
    pub cancelled_active: u64,

    /// 失败的请求数量（重试耗尽或出错）。
    pub failed: u64,

    /// 成功完成的请求数量。
    pub succeeded: u64,

    /// 曾被激活的请求总数（单调递增）。
    pub active_ever: u64,

    /// 上一次 `update()` 调用时快照的活动请求数量。
    /// 用于比较相邻两帧的活动数变化（增量诊断）。
    pub last_active: u64,

    /// 逐服务器的活动请求计数。
    ///
    /// 以 server_key 为键，跟踪每个服务器当前占用槽位的请求数。
    pub active_by_server: HashMap<String, u64>,

    /// 逐服务器的已完成总数。
    pub completed_by_server: HashMap<String, u64>,

    /// 逐服务器的总失败数。
    pub failed_by_server: HashMap<String, u64>,

    /// 逐类型的活动请求计数。
    pub active_by_type: HashMap<RequestType, u64>,

    /// 逐类型的已完成总数。
    pub completed_by_type: HashMap<RequestType, u64>,

    /// 逐类型的总失败数。
    pub failed_by_type: HashMap<RequestType, u64>,
}

impl RequestStatistics {
    /// 创建一个新初始化为零的统计实例。
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一个请求已被调度（已尝试）。
    pub fn on_scheduled(&mut self) {
        self.attempted += 1;
    }

    /// 记录一个请求已被激活。
    pub fn on_activated(&mut self, server_key: &str, request_type: RequestType) {
        // 全局活动数与“曾激活”单调计数同时加一
        self.active += 1;
        self.active_ever += 1;
        // 同步累加该服务器与该类型的活动分解计数
        *self.active_by_server.entry(server_key.to_string()).or_insert(0) += 1;
        *self.active_by_type.entry(request_type).or_insert(0) += 1;
    }

    /// 记录一个请求成功完成。
    pub fn on_completed(&mut self, server_key: &str, request_type: RequestType) {
        // 请求成功结束：活动数饱和减一避免下溢，成功数加一
        self.active = self.active.saturating_sub(1);
        self.succeeded += 1;
        // 同步扣减该服务器与该类型的活动分解计数
        if let Some(count) = self.active_by_server.get_mut(server_key) {
            *count = count.saturating_sub(1);
        }
        if let Some(count) = self.active_by_type.get_mut(&request_type) {
            *count = count.saturating_sub(1);
        }
        // 累加该服务器与该类型的完成总数
        *self.completed_by_server.entry(server_key.to_string()).or_insert(0) += 1;
        *self.completed_by_type.entry(request_type).or_insert(0) += 1;
    }

    /// 记录一个请求失败。
    pub fn on_failed(&mut self, server_key: &str, request_type: RequestType) {
        // 请求失败结束：活动数饱和减一，失败数加一
        self.active = self.active.saturating_sub(1);
        self.failed += 1;
        // 同步扣减该服务器与该类型的活动分解计数
        if let Some(count) = self.active_by_server.get_mut(server_key) {
            *count = count.saturating_sub(1);
        }
        if let Some(count) = self.active_by_type.get_mut(&request_type) {
            *count = count.saturating_sub(1);
        }
        // 累加该服务器与该类型的失败总数
        *self.failed_by_server.entry(server_key.to_string()).or_insert(0) += 1;
        *self.failed_by_type.entry(request_type).or_insert(0) += 1;
    }

    /// 记录一个待定（从未激活）的请求被取消。
    pub fn on_cancelled_pending(&mut self) {
        self.cancelled_pending += 1;
    }

    /// 记录一个活动请求被取消。
    pub fn on_cancelled_active(&mut self, server_key: &str, request_type: RequestType) {
        // 活动态取消：活动数饱和减一，取消（活动）计数加一
        self.active = self.active.saturating_sub(1);
        self.cancelled_active += 1;
        if let Some(count) = self.active_by_server.get_mut(server_key) {
            *count = count.saturating_sub(1);
        }
        if let Some(count) = self.active_by_type.get_mut(&request_type) {
            *count = count.saturating_sub(1);
        }
    }

    /// 在每个调度器 `update()` 周期开始时调用，把当前的
    /// 活动计数快照到 `last_active`，供下一帧做增量对比。
    ///
    /// 语义即 `last_active = active` 的一次赋值快照。
    pub fn snapshot_last_active(&mut self) {
        self.last_active = self.active;
    }

    /// 返回被取消的请求总数（待定 + 活动）。
    pub fn total_cancelled(&self) -> u64 {
        self.cancelled_pending + self.cancelled_active
    }

    /// 返回已结束的请求总数（成功 + 失败 + 取消）。
    pub fn total_finished(&self) -> u64 {
        self.succeeded + self.failed + self.total_cancelled()
    }

    /// 返回某个特定服务器的活动请求数量。
    pub fn active_for_server(&self, server_key: &str) -> u64 {
        self.active_by_server.get(server_key).copied().unwrap_or(0)
    }

    /// 返回某个特定类型的活动请求数量。
    pub fn active_for_type(&self, request_type: RequestType) -> u64 {
        self.active_by_type.get(&request_type).copied().unwrap_or(0)
    }

    /// 将所有计数器重置为零（用于测试或调度器重新初始化）。
    ///
    /// 通过重建一个默认实例覆盖 `self` 达成。
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// 返回用于诊断的人类可读摘要字符串。
    pub fn summary(&self) -> String {
        format!(
            "attempted={} active={} succeeded={} failed={} cancelled={}",
            self.attempted,
            self.active,
            self.succeeded,
            self.failed,
            self.total_cancelled(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_statistics_are_zeroed() {
        let stats = RequestStatistics::new();
        assert_eq!(stats.attempted, 0);
        assert_eq!(stats.active, 0);
        assert_eq!(stats.succeeded, 0);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.total_cancelled(), 0);
        assert!(stats.active_by_server.is_empty());
    }

    #[test]
    fn schedule_and_activate() {
        let mut stats = RequestStatistics::new();
        stats.on_scheduled();
        stats.on_activated("example.com:443", RequestType::Imagery);
        assert_eq!(stats.attempted, 1);
        assert_eq!(stats.active, 1);
        assert_eq!(stats.active_ever, 1);
        assert_eq!(stats.active_for_server("example.com:443"), 1);
        assert_eq!(stats.active_for_type(RequestType::Imagery), 1);
    }

    #[test]
    fn complete_decrements_active() {
        let mut stats = RequestStatistics::new();
        stats.on_scheduled();
        stats.on_activated("a.com:80", RequestType::Terrain);
        stats.on_completed("a.com:80", RequestType::Terrain);
        assert_eq!(stats.active, 0);
        assert_eq!(stats.succeeded, 1);
        assert_eq!(stats.active_for_server("a.com:80"), 0);
        assert_eq!(*stats.completed_by_server.get("a.com:80").unwrap(), 1);
    }

    #[test]
    fn fail_decrements_active() {
        let mut stats = RequestStatistics::new();
        stats.on_scheduled();
        stats.on_activated("b.com:443", RequestType::Tiles3D);
        stats.on_failed("b.com:443", RequestType::Tiles3D);
        assert_eq!(stats.active, 0);
        assert_eq!(stats.failed, 1);
        assert_eq!(*stats.failed_by_type.get(&RequestType::Tiles3D).unwrap(), 1);
    }

    #[test]
    fn cancel_pending_does_not_affect_active() {
        let mut stats = RequestStatistics::new();
        stats.on_scheduled();
        stats.on_cancelled_pending();
        assert_eq!(stats.active, 0);
        assert_eq!(stats.cancelled_pending, 1);
        assert_eq!(stats.total_cancelled(), 1);
    }

    #[test]
    fn cancel_active_decrements() {
        let mut stats = RequestStatistics::new();
        stats.on_scheduled();
        stats.on_activated("c.com:80", RequestType::Other);
        stats.on_cancelled_active("c.com:80", RequestType::Other);
        assert_eq!(stats.active, 0);
        assert_eq!(stats.cancelled_active, 1);
        assert_eq!(stats.active_for_server("c.com:80"), 0);
    }

    #[test]
    fn multiple_servers_tracked_independently() {
        let mut stats = RequestStatistics::new();
        stats.on_activated("a.com:443", RequestType::Imagery);
        stats.on_activated("a.com:443", RequestType::Imagery);
        stats.on_activated("b.com:443", RequestType::Terrain);
        assert_eq!(stats.active, 3);
        assert_eq!(stats.active_for_server("a.com:443"), 2);
        assert_eq!(stats.active_for_server("b.com:443"), 1);
        assert_eq!(stats.active_for_type(RequestType::Imagery), 2);
        assert_eq!(stats.active_for_type(RequestType::Terrain), 1);
    }

    #[test]
    fn snapshot_last_active() {
        let mut stats = RequestStatistics::new();
        stats.on_activated("x.com:80", RequestType::Other);
        stats.on_activated("x.com:80", RequestType::Other);
        stats.snapshot_last_active();
        assert_eq!(stats.last_active, 2);
        stats.on_completed("x.com:80", RequestType::Other);
        assert_eq!(stats.active, 1);
        assert_eq!(stats.last_active, 2); // 直到下次快照保持不变
    }

    #[test]
    fn reset_clears_everything() {
        let mut stats = RequestStatistics::new();
        stats.on_scheduled();
        stats.on_activated("d.com:80", RequestType::Imagery);
        stats.on_completed("d.com:80", RequestType::Imagery);
        stats.reset();
        assert_eq!(stats.attempted, 0);
        assert_eq!(stats.succeeded, 0);
        assert!(stats.active_by_server.is_empty());
    }

    #[test]
    fn total_finished_accounts_for_all_outcomes() {
        let mut stats = RequestStatistics::new();
        // 1 个成功、1 个失败、1 个 cancelled_pending、1 个 cancelled_active
        stats.on_activated("e.com:80", RequestType::Other);
        stats.on_completed("e.com:80", RequestType::Other);
        stats.on_activated("e.com:80", RequestType::Other);
        stats.on_failed("e.com:80", RequestType::Other);
        stats.on_cancelled_pending();
        stats.on_activated("e.com:80", RequestType::Other);
        stats.on_cancelled_active("e.com:80", RequestType::Other);
        assert_eq!(stats.total_finished(), 4);
    }

    #[test]
    fn summary_format() {
        let mut stats = RequestStatistics::new();
        stats.on_scheduled();
        stats.on_activated("f.com:80", RequestType::Other);
        let s = stats.summary();
        assert!(s.contains("attempted=1"));
        assert!(s.contains("active=1"));
    }
}
