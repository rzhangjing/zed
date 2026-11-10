//! 默认过期策略 —— 三态结果分类。
//!
//! 照搬 `dynamic_globe.rs:1052-1098` 的分派。三个失败态
//! **语义上互不相同，绝不能合并**：
//!
//! - `Aborted`（L1052-1060）：瓦片在在途期间离开了 wanted 集。仅清除
//!   `in_flight` + 重上传守卫，并计入一次 `stale_skip`。无永久
//!   状态变更 —— 该瓦片可能在下一帧被重新请求。
//! - `Failed`（L1062-1072）：所有工作线程重试均已耗尽（瞬时的
//!   限速/超时）。进入 `retry_after` 冷却（10 s）。并非
//!   永久性的无数据。
//! - `Placeholder`（L1074-1098）：无可用影像（例如 Bing 渐变 JPEG）。
//!   会戳上**永久性**的无数据标记，并通过 UV 上采样继承
//!   祖先覆盖。绝不重试。
//!
//! 第四个 `Fresh` 判决代表一次带载荷的成功下载。

use std::time::Duration;

use cesium_ports_driven::{StalenessPolicy, StalenessVerdict};

/// 与 `dynamic_globe.rs:1052-1098` 一致的默认过期策略。
#[derive(Debug, Clone, Copy)]
pub struct DefaultStaleness;

impl DefaultStaleness {
    /// `dynamic_globe.rs:1070` —— 一个 `Failed` 判决后的重试冷却。
    pub const RETRY_COOLDOWN: Duration = Duration::from_secs(10);
}

impl Default for DefaultStaleness {
    /// `DefaultStaleness` 为无字段单元结构，默认值即自身。
    fn default() -> Self {
        Self
    }
}

impl StalenessPolicy for DefaultStaleness {
    /// 将一个下载结果分类为四个判决之一。
    ///
    /// 优先级与 `dynamic_globe.rs` 的分派顺序一致（L1052 → L1098）：
    /// 首先检查 `aborted`（L2161 的 wanted-集门控在任何解码之前触发），
    /// 然后是 `failed`（重试耗尽），再是 `placeholder`（解码未返回可用
    /// 影像）。若都不适用，则该瓦片为 `Fresh`。
    fn classify(&self, aborted: bool, failed: bool, placeholder: bool) -> StalenessVerdict {
        if aborted {
            StalenessVerdict::Aborted
        } else if failed {
            StalenessVerdict::Failed
        } else if placeholder {
            StalenessVerdict::Placeholder
        } else {
            StalenessVerdict::Fresh
        }
    }

    /// 失败瓦片重试前的冷却时长。
    #[inline]
    fn retry_cooldown(&self) -> Duration {
        Self::RETRY_COOLDOWN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_states_are_not_merged() {
        let s = DefaultStaleness;
        // 每个单-flag 输入都映射到一个不同的判决。
        assert_eq!(s.classify(true, false, false), StalenessVerdict::Aborted);
        assert_eq!(s.classify(false, true, false), StalenessVerdict::Failed);
        assert_eq!(s.classify(false, false, true), StalenessVerdict::Placeholder);
        assert_eq!(s.classify(false, false, false), StalenessVerdict::Fresh);

        // 四个判决彼此两两不同。
        let verdicts = [
            s.classify(true, false, false),
            s.classify(false, true, false),
            s.classify(false, false, true),
            s.classify(false, false, false),
        ];
        for (i, a) in verdicts.iter().enumerate() {
            for (j, b) in verdicts.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "verdicts {i} and {j} must differ");
                }
            }
        }
    }

    #[test]
    fn aborted_takes_precedence() {
        // wanted-集门控（L2161）在解码之前触发，因此即使其他 flag
        // 碰巧也被置位，`aborted` 仍然胜出。
        let s = DefaultStaleness;
        assert_eq!(s.classify(true, true, true), StalenessVerdict::Aborted);
        assert_eq!(s.classify(false, true, true), StalenessVerdict::Failed);
    }

    #[test]
    fn retry_cooldown_is_10s() {
        // L1070：`retry_after.insert(key, now + 10s)`
        let s = DefaultStaleness;
        assert_eq!(s.retry_cooldown(), Duration::from_secs(10));
    }

    #[test]
    fn staleness_policy_is_dyn_compatible() {
        let boxed: Box<dyn StalenessPolicy> = Box::new(DefaultStaleness);
        assert_eq!(boxed.classify(false, false, true), StalenessVerdict::Placeholder);
    }
}
