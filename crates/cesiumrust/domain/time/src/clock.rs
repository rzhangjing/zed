//! Clock - 用于时间管理的模拟时钟。
//! 覆盖时钟范围策略、步进方式与推进 tick 的纯领域语义。

use crate::julian_date::JulianDate;
use serde::{Deserialize, Serialize};

/// 决定时钟在到达开始/停止时间时的行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ClockRange {
    /// 时钟始终沿当前方向推进。
    #[default]
    Unbounded,
    /// 时钟不会越过开始/停止时间推进。
    Clamped,
    /// 到达停止时间时时钟循环回开始时间。
    LoopStop,
}

/// 决定每次 tick 推进多少时间。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ClockStep {
    /// 按固定的秒数（倍率）推进。
    TickDependent,
    /// 按流逝的系统时间 * 倍率 推进。
    #[default]
    SystemClockMultiplier,
    /// 将时钟设为当前系统时间。
    SystemClock,
}

/// 用于跟踪模拟时间的简单时钟。
#[derive(Debug, Clone)]
pub struct Clock {
    /// 时钟的开始时间。
    pub start_time: JulianDate,
    /// 时钟的停止时间。
    pub stop_time: JulianDate,
    /// 当前时间。
    pub current_time: JulianDate,
    /// 每次 tick 推进的时间量（秒或倍率）。
    pub multiplier: f64,
    /// 决定 tick 行为（依赖帧或依赖系统时钟）。
    pub clock_step: ClockStep,
    /// 决定在开始/停止边界处的行为。
    pub clock_range: ClockRange,
    /// tick 是否可以推进时间。
    pub can_animate: bool,
    /// tick 是否应尝试推进时间。
    pub should_animate: bool,
    /// 上次系统时间（秒），用于 SystemClockMultiplier。
    #[allow(dead_code)]
    last_system_time_secs: f64,
}

/// 构造 Clock 的选项。
#[derive(Debug, Clone, Default)]
pub struct ClockOptions {
    /// 时钟的开始时间。
    pub start_time: Option<JulianDate>,
    /// 时钟的停止时间。
    pub stop_time: Option<JulianDate>,
    /// 当前时间。
    pub current_time: Option<JulianDate>,
    /// 决定每次 tick 推进的时间量。
    pub multiplier: Option<f64>,
    /// 决定 tick 行为。
    pub clock_step: Option<ClockStep>,
    /// 决定在开始/停止边界处的行为。
    pub clock_range: Option<ClockRange>,
    /// tick 是否可以推进时间。
    pub can_animate: Option<bool>,
    /// tick 是否应尝试推进时间。
    pub should_animate: Option<bool>,
}

impl Clock {
    /// 使用给定参数创建一个新 Clock。
    pub fn new(
        start_time: JulianDate,
        stop_time: JulianDate,
        current_time: JulianDate,
    ) -> Self {
        Self {
            start_time,
            stop_time,
            current_time,
            multiplier: 1.0,
            clock_step: ClockStep::SystemClockMultiplier,
            clock_range: ClockRange::Unbounded,
            can_animate: true,
            should_animate: false,
            last_system_time_secs: Self::get_system_time_secs(),
        }
    }

    /// 从选项创建 Clock，按下列推导规则补全缺省字段：
    /// - currentTime：若未指定 → 若设置了 startTime 则用它，否则 stopTime - 1 天，否则当前时间
    /// - startTime：若未指定 → currentTime（如上推导）
    /// - stopTime：若未指定 → startTime + 1 天
    pub fn from_options(options: &ClockOptions) -> Self {
        // 推导 currentTime
        let current_time = if let Some(ct) = options.current_time {
            ct
        } else if let Some(st) = options.start_time {
            st
        } else if let Some(stop) = options.stop_time {
            stop.add_days(-1.0)
        } else {
            JulianDate::now()
        };

        // 推导 startTime
        let start_time = options.start_time.unwrap_or(current_time);

        // 推导 stopTime
        let stop_time = options.stop_time.unwrap_or_else(|| start_time.add_days(1.0));

        Self {
            start_time,
            stop_time,
            current_time,
            multiplier: options.multiplier.unwrap_or(1.0),
            clock_step: options.clock_step.unwrap_or(ClockStep::SystemClockMultiplier),
            clock_range: options.clock_range.unwrap_or(ClockRange::Unbounded),
            can_animate: options.can_animate.unwrap_or(true),
            should_animate: options.should_animate.unwrap_or(false),
            last_system_time_secs: Self::get_system_time_secs(),
        }
    }

    /// 创建一个使用默认设置的时钟（当前时间 = now）。
    pub fn default_now() -> Self {
        Self::from_options(&ClockOptions::default())
    }

    /// 从当前时间推进时钟。
    ///
    /// `delta_secs` 是自上次 tick 以来流逝的系统时间（秒）
    /// （由调用方提供以保持框架无关）。
    pub fn tick(&mut self, delta_secs: f64) -> JulianDate {
        let mut current_time = self.current_time;

        if self.can_animate && self.should_animate {
            match self.clock_step {
                ClockStep::SystemClock => {
                    current_time = JulianDate::now();
                }
                ClockStep::TickDependent => {
                    current_time = current_time.add_seconds(self.multiplier);
                }
                ClockStep::SystemClockMultiplier => {
                    current_time = current_time.add_seconds(self.multiplier * delta_secs);
                }
            }

            // 应用时钟范围约束
            match self.clock_range {
                ClockRange::Clamped => {
                    if current_time.less_than(&self.start_time) {
                        current_time = self.start_time;
                    } else if current_time.greater_than(&self.stop_time) {
                        current_time = self.stop_time;
                    }
                }
                ClockRange::LoopStop => {
                    if current_time.less_than(&self.start_time) {
                        current_time = self.start_time;
                    }
                    while current_time.greater_than(&self.stop_time) {
                        let overshoot = current_time.seconds_difference(&self.stop_time);
                        current_time = self.start_time.add_seconds(overshoot);
                    }
                }
                ClockRange::Unbounded => {}
            }
        }

        self.current_time = current_time;
        current_time
    }

    /// 获取当前系统时间（秒，单调）。
    fn get_system_time_secs() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }
}

impl Default for Clock {
    /// 以当前时间为基准的默认时钟。
    fn default() -> Self {
        Self::default_now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_unbounded() {
        let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
        let stop = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
        let mut clock = Clock::new(start, stop, start);
        clock.should_animate = true;
        clock.clock_step = ClockStep::TickDependent;
        clock.multiplier = 60.0; // 每 tick 60 秒

        let result = clock.tick(0.016); // TickDependent 下 delta 无关紧要
        let expected = start.add_seconds(60.0);
        assert!(result.equals_epsilon(&expected, 1e-10));
    }

    #[test]
    fn test_tick_clamped() {
        let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
        let stop = JulianDate::from_date_components(2000, 1, 1, 0, 1, 0, 0.0); // 1 分钟
        let mut clock = Clock::new(start, stop, start);
        clock.should_animate = true;
        clock.clock_step = ClockStep::TickDependent;
        clock.clock_range = ClockRange::Clamped;
        clock.multiplier = 120.0; // 每 tick 2 分钟（超过 stop）

        let result = clock.tick(0.016);
        assert_eq!(result, stop); // 被钳制到 stop
    }

    #[test]
    fn test_tick_loop_stop() {
        let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
        let stop = JulianDate::from_date_components(2000, 1, 1, 1, 0, 0, 0.0); // 1 小时
        let current = JulianDate::from_date_components(2000, 1, 1, 0, 59, 0, 0.0);
        let mut clock = Clock::new(start, stop, current);
        clock.should_animate = true;
        clock.clock_step = ClockStep::TickDependent;
        clock.clock_range = ClockRange::LoopStop;
        clock.multiplier = 120.0; // 每 tick 2 分钟

        let result = clock.tick(0.016);
        // 59:00 + 2:00 = 61:00，即超过 stop（60:00）1:00
        // 循环回 start + 60 秒 = 00:01:00
        let expected = start.add_seconds(60.0);
        assert!(result.equals_epsilon(&expected, 1e-10));
    }

    #[test]
    fn test_tick_no_animate() {
        let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
        let stop = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
        let mut clock = Clock::new(start, stop, start);
        clock.should_animate = false;
        clock.clock_step = ClockStep::TickDependent;
        clock.multiplier = 60.0;

        let result = clock.tick(0.016);
        assert_eq!(result, start); // 不应推进
    }

    #[test]
    fn test_system_clock_multiplier() {
        let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
        let stop = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
        let mut clock = Clock::new(start, stop, start);
        clock.should_animate = true;
        clock.clock_step = ClockStep::SystemClockMultiplier;
        clock.multiplier = 2.0; // 2x 速度

        let result = clock.tick(0.5); // 流逝 0.5 秒
        let expected = start.add_seconds(1.0); // 0.5 * 2.0 = 1.0 秒
        assert!(result.equals_epsilon(&expected, 1e-10));
    }
}
