//! 默认重试策略 —— 双时间尺度，逐字来自 `dynamic_globe.rs`。
//!
//! 照搬两个相互独立、绝不能混淆的重试机制：
//!
//! 1. **工作线程级重试**（`download_worker`，L2186-2191）：3 次尝试，
//!    采用指数退避 `250ms << attempt` → 250 ms、500 ms、1000 ms。
//!    处理瓦片服务器瞬时的 403/429/超时。
//! 2. **流水线级冷却**（L1068-1071）：当所有工作线程重试耗尽后，瓦片
//!    进入 `retry_after` 映射，带 **10 s** 冷却，之后 `enqueue_tiles`
//!    （L398-400、L419-421）才会重新发起获取。
//!
//! 两个时间尺度都必需：工作线程重试平滑过渡瞬时的抖动；
//! 流水线冷却防止每帧都猛砸一个正在限速的服务器。

use std::time::Duration;

use cesium_ports_driven::RetryPolicy;

/// 与 `dynamic_globe.rs` 完全一致的默认重试策略。
///
/// | 参数 | 值 | 来源行 |
/// |-----------|-------|-------------|
/// | `max_attempts` | 3 | L2186 |
/// | `backoff_base` | 250 ms | L2189 |
/// | `cooldown` | 10 s | L1070 |
#[derive(Debug, Clone, Copy)]
pub struct DefaultRetry;

impl DefaultRetry {
    /// `dynamic_globe.rs:2186` —— 工作线程级重试次数（`0..3u32`）。
    pub const MAX_ATTEMPTS: u32 = 3;
    /// `dynamic_globe.rs:2189` —— 基础退避；实际 sleep = `base << attempt`。
    pub const BACKOFF_BASE: Duration = Duration::from_millis(250);
    /// `dynamic_globe.rs:1070` —— 流水线级重试冷却（10 s）。
    pub const COOLDOWN: Duration = Duration::from_secs(10);

    /// 为一个从零基起的尝试索引计算退避 sleep。
    ///
    /// 对应 L2188-2190：`sleep(250ms << attempt)`。
    /// attempt 0 → 250 ms，1 → 500 ms，2 → 1000 ms。
    #[inline]
    pub fn backoff_for(attempt: u32) -> Duration {
        Self::BACKOFF_BASE * (1u32 << attempt)
    }
}

impl Default for DefaultRetry {
    /// `DefaultRetry` 为无字段单元结构，默认值即自身。
    fn default() -> Self {
        Self
    }
}

impl RetryPolicy for DefaultRetry {
    /// 单次请求的最大尝试次数（含首次）。
    #[inline]
    fn max_attempts(&self) -> u32 {
        Self::MAX_ATTEMPTS
    }

    /// 指数退避的基础时长。
    #[inline]
    fn backoff_base(&self) -> Duration {
        Self::BACKOFF_BASE
    }

    /// 失败后进入冷却、暂停重试的时长。
    #[inline]
    fn cooldown(&self) -> Duration {
        Self::COOLDOWN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_retry_matches_dynamic_globe() {
        let r = DefaultRetry;
        assert_eq!(r.max_attempts(), 3); // L2186
        assert_eq!(r.backoff_base(), Duration::from_millis(250)); // L2189
        assert_eq!(r.cooldown(), Duration::from_secs(10)); // L1070
    }

    #[test]
    fn worker_backoff_is_exponential() {
        // L2188-2190：250ms << attempt
        assert_eq!(DefaultRetry::backoff_for(0), Duration::from_millis(250));
        assert_eq!(DefaultRetry::backoff_for(1), Duration::from_millis(500));
        assert_eq!(DefaultRetry::backoff_for(2), Duration::from_millis(1000));
    }

    #[test]
    fn dual_timescale_is_distinct() {
        // 流水线冷却（10 s）必须严格大于工作线程重试总窗口
        // （250 + 500 + 1000 = 1750 ms）。这证明两个时间尺度
        // 并未被合并为一个。
        let r = DefaultRetry;
        let worker_window: Duration = (0..r.max_attempts())
            .map(DefaultRetry::backoff_for)
            .sum();
        assert!(
            r.cooldown() > worker_window,
            "cooldown ({:?}) must exceed worker retry window ({:?})",
            r.cooldown(),
            worker_window
        );
    }

    #[test]
    fn retry_policy_is_dyn_compatible() {
        // Q1：该 trait 在具体化后必须可作为 trait 对象使用。
        let boxed: Box<dyn RetryPolicy> = Box::new(DefaultRetry);
        assert_eq!(boxed.max_attempts(), 3);
    }
}
