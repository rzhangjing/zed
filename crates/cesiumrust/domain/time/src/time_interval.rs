//! TimeInterval - 带开始/停止包含标志的时间区间。
//! 区间由起止儒略日与"边界是否闭合"两个标志确定，
//! 支持包含判定、交集运算、ISO 8601 解析与格式化。

use crate::julian_date::JulianDate;
use serde::{Deserialize, Serialize};

/// 由开始和停止时间定义的区间，可选择包含这些时间。
/// 空区间、单点区间与左右开区间均可由标志组合表达。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeInterval {
    /// 区间的开始时间。
    pub start: JulianDate,
    /// 区间的停止时间。
    pub stop: JulianDate,
    /// 开始时间是否包含在区间内。
    pub is_start_included: bool,
    /// 停止时间是否包含在区间内。
    pub is_stop_included: bool,
}

impl TimeInterval {
    /// 创建一个新的 TimeInterval。
    pub fn new(
        start: JulianDate,
        stop: JulianDate,
        is_start_included: bool,
        is_stop_included: bool,
    ) -> Self {
        Self {
            start,
            stop,
            is_start_included,
            is_stop_included,
        }
    }

    /// 若此区间为空则返回 true。
    /// 空当且仅当停止早于开始，或二者相等且任一边界不闭合。
    pub fn is_empty(&self) -> bool {
        // 比较停止与开始时间的先后
        let cmp = self.stop.cmp(&self.start);
        // 停止更早即空；相等时任一边界开区间亦为空
        cmp == std::cmp::Ordering::Less
            || (cmp == std::cmp::Ordering::Equal
                && (!self.is_start_included || !self.is_stop_included))
    }

    /// 若区间包含给定时间则返回 true。
    /// 空区间恒不包含；否则要求不早于起点且不晚于止点，
    /// 并依开闭标志决定边界等值时是否计入。
    pub fn contains(&self, time: &JulianDate) -> bool {
        if self.is_empty() {
            return false;
        }

        // 时间相对起点、止点各自的序关系
        let start_cmp = time.cmp(&self.start);
        let stop_cmp = time.cmp(&self.stop);

        // 起点闭合时等于起点即算在内；起点开放时需严格大于
        let after_start = if self.is_start_included {
            start_cmp != std::cmp::Ordering::Less
        } else {
            start_cmp == std::cmp::Ordering::Greater
        };

        // 止点闭合时等于止点即算在内；止点开放时需严格小于
        let before_stop = if self.is_stop_included {
            stop_cmp != std::cmp::Ordering::Greater
        } else {
            stop_cmp == std::cmp::Ordering::Less
        };

        // 同时满足下界与上界方为包含
        after_start && before_stop
    }

    /// 计算两个区间的交集。
    /// 取较晚的起点与较早的止点，边界闭合标志按与运算合并；
    /// 若结果区间为空则返回标准空区间常量。
    pub fn intersect(&self, other: &Self) -> Self {
        // 确定较晚的开始时间
        let (start, is_start_included) = if self.start > other.start {
            (self.start, self.is_start_included)
        } else if other.start > self.start {
            (other.start, other.is_start_included)
        } else {
            (self.start, self.is_start_included && other.is_start_included)
        };

        // 确定较早的停止时间
        let (stop, is_stop_included) = if self.stop < other.stop {
            (self.stop, self.is_stop_included)
        } else if other.stop < self.stop {
            (other.stop, other.is_stop_included)
        } else {
            (self.stop, self.is_stop_included && other.is_stop_included)
        };

        let result = Self::new(start, stop, is_start_included, is_stop_included);
        // 交集为空时归一为标准空区间
        if result.is_empty() {
            Self::EMPTY
        } else {
            result
        }
    }

    /// 一个空区间。
    pub const EMPTY: Self = Self {
        start: JulianDate { day_number: 0, seconds_of_day: 0.0 },
        stop: JulianDate { day_number: 0, seconds_of_day: 0.0 },
        is_start_included: false,
        is_stop_included: false,
    };

    /// 从 ISO 8601 区间字符串（"start/stop"）创建 TimeInterval。
    /// 以 '/' 分割两段并各自解析为儒略日；任一段无法解析则返回 None。
    pub fn from_iso8601(
        iso8601: &str,
        is_start_included: bool,
        is_stop_included: bool,
    ) -> Option<Self> {
        // 按 '/' 拆分为起止两段
        let parts: Vec<&str> = iso8601.split('/').collect();
        // 必须恰好两段，否则不是合法的区间字符串
        if parts.len() != 2 {
            return None;
        }
        // 两段分别解析为儒略日，任一失败则整体失败
        let start = JulianDate::from_iso8601(parts[0])?;
        let stop = JulianDate::from_iso8601(parts[1])?;
        Some(Self::new(start, stop, is_start_included, is_stop_included))
    }

    /// 将此区间格式化为 ISO 8601 区间字符串。
    /// 形如 "起始/停止"，两段各自用默认精度的 ISO 8601 表示。
    pub fn to_iso8601(&self) -> String {
        format!("{}/{}", self.start.to_iso8601(), self.stop.to_iso8601())
    }

    /// 以指定精度将此区间格式化为 ISO 8601 区间字符串。
    /// precision 为 None 时各段采用默认精度。
    pub fn to_iso8601_with_precision(&self, precision: Option<usize>) -> String {
        format!(
            "{}/{}",
            self.start.to_iso8601_with_precision(precision),
            self.stop.to_iso8601_with_precision(precision)
        )
    }

    /// 在 epsilon（秒）范围内比较两个区间是否相等。
    /// 起止时间各自满足容差比较，且两个边界闭合标志完全一致。
    pub fn equals_epsilon(&self, other: &Self, epsilon: f64) -> bool {
        // 起止时间容差比较 + 边界标志严格相等
        self.start.equals_epsilon(&other.start, epsilon)
            && self.stop.equals_epsilon(&other.stop, epsilon)
            && self.is_start_included == other.is_start_included
            && self.is_stop_included == other.is_stop_included
    }

    /// 区间的时长（秒）。
    pub fn duration_seconds(&self) -> f64 {
        // 空区间时长为 0；否则为止点与起点之差
        if self.is_empty() {
            0.0
        } else {
            self.stop.seconds_difference(&self.start)
        }
    }
}

impl Default for TimeInterval {
    /// 默认区间：起止均为默认儒略日且两侧边界闭合。
    fn default() -> Self {
        Self {
            start: JulianDate::default(),
            stop: JulianDate::default(),
            is_start_included: true,
            is_stop_included: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 闭区间应包含内部点及两端边界点，区间外点不包含
    #[test]
    fn test_contains() {
        let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
        let stop = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
        let interval = TimeInterval::new(start, stop, true, true);

        let inside = JulianDate::from_date_components(2000, 1, 1, 12, 0, 0, 0.0);
        assert!(interval.contains(&inside));
        assert!(interval.contains(&start));
        assert!(interval.contains(&stop));

        let outside = JulianDate::from_date_components(2000, 1, 3, 0, 0, 0, 0.0);
        assert!(!interval.contains(&outside));
    }

    // 左右开区间不包含两端边界点，但包含内部点
    #[test]
    fn test_exclusive_bounds() {
        let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
        let stop = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
        let interval = TimeInterval::new(start, stop, false, false);

        assert!(!interval.contains(&start));
        assert!(!interval.contains(&stop));

        let inside = JulianDate::from_date_components(2000, 1, 1, 12, 0, 0, 0.0);
        assert!(interval.contains(&inside));
    }

    // 反向区间与单点开区间均判定为空
    #[test]
    fn test_is_empty() {
        let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
        let stop = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);

        let normal = TimeInterval::new(start, stop, true, true);
        assert!(!normal.is_empty());

        let inverted = TimeInterval::new(stop, start, true, true);
        assert!(inverted.is_empty());

        let point_exclusive = TimeInterval::new(start, start, false, true);
        assert!(point_exclusive.is_empty());
    }

    // 两个重叠区间的交集取较晚起点与较早止点
    #[test]
    fn test_intersect() {
        let start1 = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
        let stop1 = JulianDate::from_date_components(2000, 1, 10, 0, 0, 0, 0.0);
        let interval1 = TimeInterval::new(start1, stop1, true, true);

        let start2 = JulianDate::from_date_components(2000, 1, 5, 0, 0, 0, 0.0);
        let stop2 = JulianDate::from_date_components(2000, 1, 15, 0, 0, 0, 0.0);
        let interval2 = TimeInterval::new(start2, stop2, true, true);

        let intersection = interval1.intersect(&interval2);
        assert_eq!(intersection.start, start2);
        assert_eq!(intersection.stop, stop1);
    }

    // 不相交区间的交集为空区间
    #[test]
    fn test_no_intersection() {
        let start1 = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
        let stop1 = JulianDate::from_date_components(2000, 1, 5, 0, 0, 0, 0.0);
        let interval1 = TimeInterval::new(start1, stop1, true, true);

        let start2 = JulianDate::from_date_components(2000, 1, 10, 0, 0, 0, 0.0);
        let stop2 = JulianDate::from_date_components(2000, 1, 15, 0, 0, 0, 0.0);
        let interval2 = TimeInterval::new(start2, stop2, true, true);

        assert!(interval1.intersect(&interval2).is_empty());
    }
}
