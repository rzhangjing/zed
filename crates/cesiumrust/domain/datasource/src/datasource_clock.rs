//! DataSourceClock - 与 DataSource 关联的时钟设置。
//!
//! 它记录数据源自带的时间轴：起始/停止时刻、当前时刻、倍率与步进方式。

use cesium_time::{Clock, ClockOptions, ClockRange, ClockStep, JulianDate};

/// 与 DataSource 关联的时钟设置，提供合并、克隆与逐字段取值的语义。
///
/// 数据源在加载时用它覆盖全局时钟的默认区间，使动画从自带时间轴起播。
#[derive(Debug, Clone, Default)]
pub struct DataSourceClock {
    /// 时钟的起始时间。
    pub start_time: Option<JulianDate>,
    /// 时钟的停止时间。
    pub stop_time: Option<JulianDate>,
    /// 当前时间。
    pub current_time: Option<JulianDate>,
    /// 决定时钟在起始/停止边界处的行为。
    pub clock_range: Option<ClockRange>,
    /// 决定时间每次推进的方式。
    pub clock_step: Option<ClockStep>,
    /// 时间推进的倍率。
    pub multiplier: Option<f64>,
}

impl DataSourceClock {
    /// 创建一个所有字段均未设置的新 DataSourceClock。
    pub fn new() -> Self {
        Self::default()
    }

    /// 将 `source` 中未赋值的属性合并到此时钟。
    /// 已赋值的属性不会被覆盖。
    ///
    /// 映射到 `DataSourceClock.prototype.merge`
    pub fn merge(&mut self, source: &DataSourceClock) {
        if self.start_time.is_none() {
            self.start_time = source.start_time;
        }
        if self.stop_time.is_none() {
            self.stop_time = source.stop_time;
        }
        if self.current_time.is_none() {
            self.current_time = source.current_time;
        }
        if self.clock_range.is_none() {
            self.clock_range = source.clock_range;
        }
        if self.clock_step.is_none() {
            self.clock_step = source.clock_step;
        }
        if self.multiplier.is_none() {
            self.multiplier = source.multiplier;
        }
    }

    /// 以 Clock 实例的形式获取值。未设置的字段使用默认值：
    /// clock_range=UNBOUNDED，clock_step=SYSTEM_CLOCK_MULTIPLIER，multiplier=1.0。
    ///
    /// 映射到 `DataSourceClock.prototype.getValue`
    pub fn get_value(&self) -> Clock {
        let options = ClockOptions {
            start_time: self.start_time,
            stop_time: self.stop_time,
            current_time: self.current_time,
            multiplier: self.multiplier,
            clock_step: self.clock_step,
            clock_range: self.clock_range,
            can_animate: Some(true),
            should_animate: Some(true),
        };
        Clock::from_options(&options)
    }
}
