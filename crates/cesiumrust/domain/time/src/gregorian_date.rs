//! GregorianDate - 日历日期表示。

use serde::{Deserialize, Serialize};

/// 每个月的天数（非闰年）。索引 0 = 一月。
const DAYS_IN_MONTH: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// 若给定年份是闰年则返回 true。
/// 遵循“四年一闰、百年不闰、四百年再闰”规则。
pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// 返回给定年份中给定月份（从 1 开始）的天数。
pub fn days_in_month(year: i32, month: u32) -> u32 {
    if month == 2 && is_leap_year(year) {
        29
    } else {
        DAYS_IN_MONTH[(month - 1) as usize]
    }
}

/// 格里高利历（公历）中的一个日历日期。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GregorianDate {
    /// 年（1-9999）。
    pub year: i32,
    /// 月（1-12）。
    pub month: u32,
    /// 该月的日（1-31）。
    pub day: u32,
    /// 时（0-23）。
    pub hour: u32,
    /// 分（0-59）。
    pub minute: u32,
    /// 秒（0-60，闰秒时为 60）。
    pub second: u32,
    /// 毫秒（0-999.999...）。
    pub millisecond: f64,
    /// 该日期是否处于闰秒期间。
    pub is_leap_second: bool,
}

impl GregorianDate {
    /// 创建一个经过校验的新 GregorianDate。
    ///
    /// 校验仅在 debug 下生效（越界会触发 debug_assert 失败）。
    // deferred.md #13: debug_assert 范围校验 (year/month/day/millisecond) 触发 manual_range_contains，风格问题。
    #[allow(clippy::too_many_arguments, clippy::manual_range_contains)]
    pub fn new(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        millisecond: f64,
        is_leap_second: bool,
    ) -> Self {
        // 仅 debug 下生效的范围校验
        debug_assert!(year >= 1 && year <= 9999, "Year must be in range [1, 9999], got {year}");
        debug_assert!(month >= 1 && month <= 12, "Month must be in range [1, 12], got {month}");
        debug_assert!(day >= 1 && day <= 31, "Day must be in range [1, 31], got {day}");
        debug_assert!(hour <= 23, "Hour must be in range [0, 23], got {hour}");
        debug_assert!(minute <= 59, "Minute must be in range [0, 59], got {minute}");
        let max_second = if is_leap_second { 60 } else { 59 };
        debug_assert!(second <= max_second, "Second must be in range [0, {max_second}], got {second}");
        debug_assert!(millisecond >= 0.0 && millisecond < 1000.0,
            "Millisecond must be in range [0, 1000), got {millisecond}");
        // 校验该日对给定的月/年是否有效
        if month >= 1 && month <= 12 {
            let max_day = days_in_month(year, month);
            debug_assert!(day <= max_day,
                "Day {day} is invalid for year {year} month {month} (max {max_day})");
        }

        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            millisecond,
            is_leap_second,
        }
    }
}

impl Default for GregorianDate {
    /// 构造最小日期（1 年 1 月 1 日，午夜）。
    /// 各分量取类型默认值：年月日为 1，时间部分归零。
    fn default() -> Self {
        Self {
            year: 1,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            millisecond: 0.0,
            is_leap_second: false,
        }
    }
}
