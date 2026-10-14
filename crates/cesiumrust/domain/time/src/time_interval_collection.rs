//! TimeIntervalCollection - 按开始时间排序的、互不重叠的 `TimeInterval`
//! 实例集合。
//!
//! 集合内部维持有序与互不重叠不变式：新增或移除区间时会相应地
//! 合并、拆分、截断相邻区间，从而保持时间轴上的无冲突覆盖。
//!
//! 该集合按开始时间排序保存各区间，并保证任意两个区间互不重叠。添加一个
//! 区间时，若其携带相同数据则与相邻区间合并；若数据不同则拆分/截断已有
//! 区间（新添加区间的数据优先）。

// deferred.md #13: `is_leap_year` 目前仅由 julian_date.rs 经全路径调用，本文件暂未使用。
#[allow(unused_imports)]
use crate::gregorian_date::{days_in_month, is_leap_year, GregorianDate};
use crate::julian_date::JulianDate;
use crate::time_interval::TimeInterval;
use std::cmp::Ordering;

/// 一个 `TimeInterval` 及其可选的数据负载。
///
/// 集合中的每个条目都携带一段区间与一个可选数据，合并/拆分时
/// 数据随之流动；`data` 为 `None` 表示该区间不承载业务数据。
#[derive(Debug, Clone, PartialEq)]
pub struct TimeIntervalData<T> {
    /// 底层区间（开始/停止/包含标志）。
    pub interval: TimeInterval,
    /// 与该区间关联的数据。
    pub data: Option<T>,
}

impl<T> TimeIntervalData<T> {
    /// 创建一个带数据的新区间。
    pub fn new(interval: TimeInterval, data: Option<T>) -> Self {
        Self { interval, data }
    }

    /// 若区间为空则返回 true。
    pub fn is_empty(&self) -> bool {
        self.interval.is_empty()
    }

    /// 若区间包含给定时间则返回 true。
    pub fn contains(&self, time: &JulianDate) -> bool {
        self.interval.contains(time)
    }
}

/// 按开始时间排序的、互不重叠的 `TimeInterval` 实例集合。
///
/// 集合持有泛型数据负载 `T`，仅在需要合并/拆分相邻区间时被比较，
/// 因此相同数据的相邻区间会被折叠为一个连续区间。
#[derive(Debug, Clone)]
pub struct TimeIntervalCollection<T> {
    /// 按开始时间升序排列且互不重叠的区间条目。
    intervals: Vec<TimeIntervalData<T>>,
}

impl<T> Default for TimeIntervalCollection<T> {
    /// 默认构造等价于空集合。
    fn default() -> Self {
        Self::new()
    }
}

impl<T> TimeIntervalCollection<T> {
    /// 创建一个空集合。
    pub fn new() -> Self {
        // 初始不含任何区间
        Self {
            intervals: Vec::new(),
        }
    }

    /// 创建一个以给定区间预填充的集合。
    /// 逐个调用 `add_interval`，以保证排序、合并与去重叠不变式。
    pub fn from_intervals<F>(intervals: Vec<TimeIntervalData<T>>, same_data: &F) -> Self
    where
        T: Clone,
        F: Fn(&T, &T) -> bool,
    {
        let mut collection = Self::new();
        // 逐个插入以复用 add_interval 的排序/合并/去重叠不变式
        for interval in intervals {
            collection.add_interval(interval, same_data);
        }
        collection
    }

    /// 集合中区间数量。
    /// 直接返回内部有序列表的长度。
    pub fn len(&self) -> usize {
        // 条目数即集合长度
        self.intervals.len()
    }

    /// 若集合为空则返回 true。
    /// 内部列表无元素即视为空集合。
    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty()
    }

    /// 集合的开始时间（第一个区间的开始）。
    /// 空集合返回 None；否则取有序列表首元素的起点。
    pub fn start(&self) -> Option<JulianDate> {
        // 首个区间的 start 即整体起点
        self.intervals.first().map(|i| i.interval.start)
    }

    /// 开始时间是否包含在集合内。
    /// 取决于首区间的左闭标志；空集合按不包含处理。
    pub fn is_start_included(&self) -> bool {
        // 首区间缺失时默认开区间
        self.intervals
            .first()
            .map(|i| i.interval.is_start_included)
            .unwrap_or(false)
    }

    /// 集合的停止时间（最后一个区间的停止）。
    /// 空集合返回 None；否则取有序列表末元素的止点。
    pub fn stop(&self) -> Option<JulianDate> {
        // 末区间的 stop 即整体止点
        self.intervals.last().map(|i| i.interval.stop)
    }

    /// 停止时间是否包含在集合内。
    /// 取决于末区间的右闭标志；空集合按不包含处理。
    pub fn is_stop_included(&self) -> bool {
        // 末区间缺失时默认开区间
        self.intervals
            .last()
            .map(|i| i.interval.is_stop_included)
            .unwrap_or(false)
    }

    /// 获取指定索引处的区间。
    /// 越界返回 None，按有序列表的物理下标直接访问。
    pub fn get(&self, index: usize) -> Option<&TimeIntervalData<T>> {
        self.intervals.get(index)
    }

    /// 返回遍历各区间的迭代器。
    pub fn iter(&self) -> std::slice::Iter<'_, TimeIntervalData<T>> {
        self.intervals.iter()
    }

    /// 从集合中移除所有区间。
    /// 清空内部列表，集合回到空状态。
    pub fn remove_all(&mut self) {
        // 直接清空有序区间列表
        self.intervals.clear();
    }

    /// 查找并返回包含指定日期的区间的索引。当没有区间包含该日期时，
    /// 返回一个负数（插入索引的按位取反），以便调用方据此定位插入位置。
    ///
    /// 先按开始时间二分，再结合左右边界闭开标志判断日期究竟落在
    /// 命中区间、前一个区间，还是二者之间的空隙。
    pub fn index_of(&self, date: &JulianDate) -> isize {
        let intervals = &self.intervals;

        // 对开始时间进行二分查找，寻找 start == date 的区间。
        let mut index = binary_search_start(intervals, date);

        if index >= 0 {
            let idx = index as usize;
            if intervals[idx].interval.is_start_included {
                return index;
            }
            if idx > 0
                && intervals[idx - 1].interval.stop == *date
                && intervals[idx - 1].interval.is_stop_included
            {
                return (idx - 1) as isize;
            }
            return !(index);
        }

        index = !index;
        let idx = index as usize;
        if idx > 0
            && idx - 1 < intervals.len()
            && intervals[idx - 1].contains(date)
        {
            return (idx - 1) as isize;
        }
        !index
    }

    /// 若集合包含指定日期则返回 true。
    /// 委托 index_of：返回非负索引即表示存在包含该区间的条目。
    pub fn contains(&self, date: &JulianDate) -> bool {
        // 索引非负说明命中某个区间
        self.index_of(date) >= 0
    }

    /// 查找并返回包含指定日期的区间。
    /// 先经 index_of 定位，命中则返回该区间引用，否则返回 None。
    pub fn find_interval_containing_date(&self, date: &JulianDate) -> Option<&TimeIntervalData<T>> {
        // 非负索引代表命中
        let index = self.index_of(date);
        if index >= 0 {
            self.intervals.get(index as usize)
        } else {
            None
        }
    }

    /// 查找并返回包含指定日期的区间的数据。
    /// 定位区间后取其 data 字段引用；区间缺失或无数据均返回 None。
    pub fn find_data_for_interval_containing_date(&self, date: &JulianDate) -> Option<&T> {
        // 先取区间再取其数据负载
        self.find_interval_containing_date(date)
            .and_then(|i| i.data.as_ref())
    }

    /// 返回匹配可选 start/stop/inclusion 参数的第一个区间。
    /// 为 `None` 的参数视为不关心。
    ///
    /// 未给出的参数自动放行，从而支持按任意子集匹配。
    pub fn find_interval(
        &self,
        start: Option<&JulianDate>,
        stop: Option<&JulianDate>,
        is_start_included: Option<bool>,
        is_stop_included: Option<bool>,
    ) -> Option<&TimeIntervalData<T>> {
        self.intervals.iter().find(|interval| {
            let iv = &interval.interval;
            let start_ok = start.map(|s| iv.start == *s).unwrap_or(true);
            let stop_ok = stop.map(|s| iv.stop == *s).unwrap_or(true);
            let isi_ok = is_start_included
                .map(|v| iv.is_start_included == v)
                .unwrap_or(true);
            let ist_ok = is_stop_included
                .map(|v| iv.is_stop_included == v)
                .unwrap_or(true);
            start_ok && stop_ok && isi_ok && ist_ok
        })
    }

    /// 向集合添加一个区间，合并包含相同数据的区间，并在需要时拆分数据
    /// 不同的区间，以维持一个互不重叠的集合。新区间中的数据优先于任何
    /// 已有区间。
    ///
    /// `same_data` 比较两个数据负载，以决定相邻区间能否合并。
    ///
    /// 流程：空区间直接忽略；晚于全部已有区间则快速追加；否则二分定位
    /// 插入点，向前处理重叠、向后循环吞并/截断重叠区间，最后插入合并结果。
    pub fn add_interval<F>(&mut self, mut interval: TimeIntervalData<T>, same_data: &F)
    where
        T: Clone,
        F: Fn(&T, &T) -> bool,
    {
        if interval.is_empty() {
            return;
        }

        // 快速路径：追加在所有已有内容之后。
        if self.intervals.is_empty()
            || interval.interval.start > self.intervals[self.intervals.len() - 1].interval.stop
        {
            self.intervals.push(interval);
            return;
        }

        // 保持列表按开始日期排序。
        let mut index = binary_search_start(&self.intervals, &interval.interval.start);
        if index < 0 {
            index = !index;
        } else {
            let mut idx = index as usize;
            // 起点相同且都左闭时，插入到前一个区间之前以保持合并顺序
            if idx > 0
                && interval.interval.is_start_included
                && self.intervals[idx - 1].interval.is_start_included
                && self.intervals[idx - 1].interval.start == interval.interval.start
            {
                idx -= 1;
            // 新区间左开而已有区间左闭且起点相同时，排到其之后
            } else if idx < self.intervals.len()
                && !interval.interval.is_start_included
                && self.intervals[idx].interval.is_start_included
                && self.intervals[idx].interval.start == interval.interval.start
            {
                idx += 1;
            }
            index = idx as isize;
        }

        // 将定位索引转为 usize 供后续前向/后向处理
        let mut idx = index as usize;

        if idx > 0 {
            // 查看前一个区间是否与此区间重叠。
            // 比较前区间的止点与新区间的起点
            let cmp = compare(
                &self.intervals[idx - 1].interval.stop,
                &interval.interval.start,
            );
            // 止点更晚，或相等且任一边界闭合，即判定为重叠
            if cmp == Ordering::Greater
                || (cmp == Ordering::Equal
                    && (self.intervals[idx - 1].interval.is_stop_included
                        || interval.interval.is_start_included))
            {
                // 判断相邻区间是否与新区间携带相同数据
                let same = data_equals(
                    self.intervals[idx - 1].data.as_ref(),
                    interval.data.as_ref(),
                    same_data,
                );
                if same {
                    // 重叠的区间具有相同数据，因此将它们合并。
                    // 新区间延伸更晚时，把止点更新为新区间止点
                    if interval.interval.stop > self.intervals[idx - 1].interval.stop {
                        interval = TimeIntervalData {
                            interval: TimeInterval::new(
                                self.intervals[idx - 1].interval.start,
                                interval.interval.stop,
                                self.intervals[idx - 1].interval.is_start_included,
                                interval.interval.is_stop_included,
                            ),
                            data: interval.data,
                        };
                    } else {
                        let stop_included = self.intervals[idx - 1].interval.is_stop_included
                            || (interval.interval.stop == self.intervals[idx - 1].interval.stop
                                && interval.interval.is_stop_included);
                        interval = TimeIntervalData {
                            interval: TimeInterval::new(
                                self.intervals[idx - 1].interval.start,
                                self.intervals[idx - 1].interval.stop,
                                self.intervals[idx - 1].interval.is_start_included,
                                stop_included,
                            ),
                            data: interval.data,
                        };
                    }
                    self.intervals.remove(idx - 1);
                    idx -= 1;
                } else {
                    // 数据不同：新区间胜出；截断前一个区间，
                    // 若其延伸超过新区间则拆分它。
                    // 比较前区间止点与新区间止点，判断前区间是否延伸更远
                    let cmp2 = compare(
                        &self.intervals[idx - 1].interval.stop,
                        &interval.interval.stop,
                    );
                    // 前区间尾部超出新区间，需拆出一段保留其后半
                    if cmp2 == Ordering::Greater
                        || (cmp2 == Ordering::Equal
                            && self.intervals[idx - 1].interval.is_stop_included
                            && !interval.interval.is_stop_included)
                    {
                        let tail = TimeIntervalData {
                            interval: TimeInterval::new(
                                interval.interval.stop,
                                self.intervals[idx - 1].interval.stop,
                                !interval.interval.is_stop_included,
                                self.intervals[idx - 1].interval.is_stop_included,
                            ),
                            data: self.intervals[idx - 1].data.clone(),
                        };
                        self.intervals.insert(idx, tail);
                    }
                    // 将前区间截断到新区间起点之前
                    let prev = &self.intervals[idx - 1];
                    let truncated = TimeIntervalData {
                        interval: TimeInterval::new(
                            prev.interval.start,
                            interval.interval.start,
                            prev.interval.is_start_included,
                            !interval.interval.is_start_included,
                        ),
                        data: prev.data.clone(),
                    };
                    self.intervals[idx - 1] = truncated;
                }
            }
        }

        // 向后遍历，处理所有与新区间重叠的后续区间
        while idx < self.intervals.len() {
            // 查看此区间之后的区间是否与此区间重叠。
            let cmp = compare(&interval.interval.stop, &self.intervals[idx].interval.start);
            if cmp == Ordering::Greater
                || (cmp == Ordering::Equal
                    && (interval.interval.is_stop_included
                        || self.intervals[idx].interval.is_start_included))
            {
                let same = data_equals(
                    self.intervals[idx].data.as_ref(),
                    interval.data.as_ref(),
                    same_data,
                );
                if same {
                    // 相同数据：将它们合并。
                    // 取更晚的止点及其闭合标志
                    let next_stop = self.intervals[idx].interval.stop;
                    let (new_stop, new_stop_included) = if next_stop > interval.interval.stop {
                        (next_stop, self.intervals[idx].interval.is_stop_included)
                    } else {
                        (interval.interval.stop, interval.interval.is_stop_included)
                    };
                    interval = TimeIntervalData {
                        interval: TimeInterval::new(
                            interval.interval.start,
                            new_stop,
                            interval.interval.is_start_included,
                            new_stop_included,
                        ),
                        data: interval.data,
                    };
                    self.intervals.remove(idx);
                } else {
                    // 数据不同：新区间胜出；截断下一个区间。
                    let next = &self.intervals[idx];
                    let truncated = TimeIntervalData {
                        interval: TimeInterval::new(
                            interval.interval.stop,
                            next.interval.stop,
                            !interval.interval.is_stop_included,
                            next.interval.is_stop_included,
                        ),
                        data: next.data.clone(),
                    };
                    if truncated.is_empty() {
                        self.intervals.remove(idx);
                    } else {
                        self.intervals[idx] = truncated;
                        // 找到部分跨越；下一个区间无法被跨越。
                        break;
                    }
                }
            } else {
                // 找到我们跨越的最后一个区间；停止查找。
                break;
            }
        }

        // 插入最终（可能已合并扩展的）区间
        self.intervals.insert(idx, interval);
    }

    /// 从本集合移除指定区间，在指定区间处创建一个空洞。输入的区间数据被忽略。
    /// 若区间的任何部分原本在集合中则返回 true。
    ///
    /// 二分定位后，依次处理左侧截断、起点保留、完全覆盖的删除、止点
    /// 边界保留以及右侧部分重叠的截断，逐步把输入区间从集合中挖出。
    pub fn remove_interval(&mut self, interval: &TimeInterval) -> bool
    where
        T: Clone,
    {
        // 空区间无需处理
        if interval.is_empty() {
            return false;
        }

        // 二分定位到待移除起点对应的插入位置
        let mut index = binary_search_start(&self.intervals, &interval.start);
        if index < 0 {
            index = !index;
        }
        let mut idx = index as usize;

        // result 记录是否确有内容与本次移除相交
        let mut result = false;

        // 检查前一个区间末尾的截断。
        if idx > 0
            && (self.intervals[idx - 1].interval.stop > interval.start
                || (self.intervals[idx - 1].interval.stop == interval.start
                    && self.intervals[idx - 1].interval.is_stop_included
                    && interval.is_start_included))
        {
            result = true;

            if self.intervals[idx - 1].interval.stop > interval.stop
                || (self.intervals[idx - 1].interval.is_stop_included
                    && !interval.is_stop_included
                    && self.intervals[idx - 1].interval.stop == interval.stop)
            {
                // 将已有区间拆成两段。
                let tail = TimeIntervalData {
                    interval: TimeInterval::new(
                        interval.stop,
                        self.intervals[idx - 1].interval.stop,
                        !interval.is_stop_included,
                        self.intervals[idx - 1].interval.is_stop_included,
                    ),
                    data: self.intervals[idx - 1].data.clone(),
                };
                self.intervals.insert(idx, tail);
            }
            // 把前区间截断到待移除起点之前
            let prev = &self.intervals[idx - 1];
            let truncated = TimeIntervalData {
                interval: TimeInterval::new(
                    prev.interval.start,
                    interval.start,
                    prev.interval.is_start_included,
                    !interval.is_start_included,
                ),
                data: prev.data.clone(),
            };
            self.intervals[idx - 1] = truncated;
        }

        // 若 interval.start 匹配但不包含，则保留该起始点。
        if idx < self.intervals.len()
            && !interval.is_start_included
            && self.intervals[idx].interval.is_start_included
            && interval.start == self.intervals[idx].interval.start
        {
            result = true;
            let point = TimeIntervalData {
                interval: TimeInterval::new(
                    self.intervals[idx].interval.start,
                    self.intervals[idx].interval.start,
                    true,
                    true,
                ),
                data: self.intervals[idx].data.clone(),
            };
            self.intervals.insert(idx, point);
            idx += 1;
        }

        // 移除被输入区间完全覆盖的所有区间。
        while idx < self.intervals.len() && interval.stop > self.intervals[idx].interval.stop {
            result = true;
            self.intervals.remove(idx);
        }

        // 处理输入区间与某个已有区间
        // 结束于同一日期的情况。
        if idx < self.intervals.len() && interval.stop == self.intervals[idx].interval.stop {
            result = true;
            if !interval.is_stop_included && self.intervals[idx].interval.is_stop_included {
                // 最后一个点应当保留。
                let stop_time = interval.stop;
                let cur = &self.intervals[idx];
                self.intervals[idx] = TimeIntervalData {
                    interval: TimeInterval::new(stop_time, stop_time, true, true),
                    data: cur.data.clone(),
                };
            } else {
                self.intervals.remove(idx);
            }
        }

        // 截断任何部分重叠的区间。
        if idx < self.intervals.len()
            && (interval.stop > self.intervals[idx].interval.start
                || (interval.stop == self.intervals[idx].interval.start
                    && interval.is_stop_included
                    && self.intervals[idx].interval.is_start_included))
        {
            result = true;
            let cur = &self.intervals[idx];
            let truncated = TimeIntervalData {
                interval: TimeInterval::new(
                    interval.stop,
                    cur.interval.stop,
                    !interval.is_stop_included,
                    cur.interval.is_stop_included,
                ),
                data: cur.data.clone(),
            };
            self.intervals[idx] = truncated;
        }

        result
    }

    /// 创建一个新集合，为本集合与所提供集合的交集。
    ///
    /// 用双指针在两个有序列表上同步推进：止点较早的一侧先前进，
    /// 仅当两侧数据相同才产出一段交集。
    pub fn intersect<F>(&self, other: &TimeIntervalCollection<T>, same_data: &F) -> TimeIntervalCollection<T>
    where
        T: Clone,
        F: Fn(&T, &T) -> bool,
    {
        let mut result = TimeIntervalCollection::new();
        // 双指针分别在本集合与 other 上推进
        let mut left = 0usize;
        let mut right = 0usize;

        while left < self.intervals.len() && right < other.intervals.len() {
            let left_interval = &self.intervals[left];
            let right_interval = &other.intervals[right];

            // 左侧完全早于右侧则左指针前进，反之右指针前进
            if left_interval.interval.stop < right_interval.interval.start {
                left += 1;
            } else if right_interval.interval.stop < left_interval.interval.start {
                right += 1;
            } else {
                // 两区间相交：仅当数据相同才保留交集
                let same = data_equals(
                    left_interval.data.as_ref(),
                    right_interval.data.as_ref(),
                    same_data,
                );
                if same {
                    let intersection =
                        left_interval.interval.intersect(&right_interval.interval);
                    if !intersection.is_empty() {
                        result.add_interval(
                            TimeIntervalData {
                                interval: intersection,
                                data: left_interval.data.clone(),
                            },
                            same_data,
                        );
                    }
                }

                // 止点较早（或相等但左开右闭）的一侧先前进
                if left_interval.interval.stop < right_interval.interval.stop
                    || (left_interval.interval.stop == right_interval.interval.stop
                        && !left_interval.interval.is_stop_included
                        && right_interval.interval.is_stop_included)
                {
                    left += 1;
                } else {
                    right += 1;
                }
            }
        }

        result
    }

    /// 将本集合与另一个集合比较是否相等，使用 `same_data` 比较区间数据。
    ///
    /// 要求区间数量、逐段区间以及对应数据负载都完全一致。
    pub fn equals<F>(&self, other: &TimeIntervalCollection<T>, same_data: &F) -> bool
    where
        F: Fn(&T, &T) -> bool,
    {
        // 数量不同直接不等
        if self.intervals.len() != other.intervals.len() {
            return false;
        }
        // 数量相同则逐段比对，全部一致才为相等
        // 逐段比较区间几何与数据负载
        for (a, b) in self.intervals.iter().zip(other.intervals.iter()) {
            if a.interval != b.interval {
                return false;
            }
            if !data_equals(a.data.as_ref(), b.data.as_ref(), same_data) {
                return false;
            }
        }
        true
    }
}

/// Iso8601.MINIMUM_VALUE 的等价：0001-01-01T00:00:00Z
fn iso8601_minimum_value() -> JulianDate {
    JulianDate::from_iso8601("0001-01-01T00:00:00Z").unwrap()
}

/// Iso8601.MAXIMUM_VALUE 的等价：9999-12-31T24:00:00Z
fn iso8601_maximum_value() -> JulianDate {
    JulianDate::from_iso8601("9999-12-31T24:00:00Z").unwrap()
}

/// 以 GregorianDate 各分量表示的时长中间值。
/// 由 ISO8601 时长解析得到，并可按分量逐段加到一个日期上。
#[derive(Debug, Clone, Copy, Default)]
struct Duration {
    /// 年数分量。
    year: f64,
    /// 月数分量。
    month: f64,
    /// 天数分量（周会被折算成天数叠加到此）。
    day: f64,
    /// 小时分量。
    hour: f64,
    /// 分钟分量。
    minute: f64,
    /// 秒分量。
    second: f64,
    /// 毫秒分量。
    millisecond: f64,
}

impl Duration {
    /// 当所有分量均为零时视为零时长。
    fn is_zero(&self) -> bool {
        // 逐分量与 0 比较，全部为零才返回 true
        self.year == 0.0
            && self.month == 0.0
            && self.day == 0.0
            && self.hour == 0.0
            && self.minute == 0.0
            && self.second == 0.0
            && self.millisecond == 0.0
    }
}

/// 解析 ISO8601 时长字符串（如 "P1Y2M3DT1H2M3.5S"）或基于日期的
/// 时长（如 "0001-02-03T01:02:03.5"）。
/// 前者按 'P' 前缀走分量解析，后者补 'Z' 后按日期解析并提取各分量。
/// 解析失败或结果为零时长时均返回 None。
fn parse_duration(iso8601: Option<&str>) -> Option<Duration> {
    // 无输入直接返回 None
    let iso8601 = iso8601?;
    // 空串不是合法时长
    if iso8601.is_empty() {
        return None;
    }

    // 累加各分量的结果时长
    let mut result = Duration::default();

    // deferred.md #13: 手动 strip 'P' 前缀，等价 strip_prefix('P')；风格问题非逻辑错误。
    #[allow(clippy::manual_strip)]
    if iso8601.starts_with('P') {
        // ISO8601 时长格式：P[n]Y[n]M[n]W[n]DT[n]H[n]M[n]S
        let s = &iso8601[1..]; // 去掉 'P'
        // 以 'T' 为界拆分为日期部分与时间部分（时间部分可缺省）
        let (date_part, time_part) = if let Some(idx) = s.find('T') {
            (&s[..idx], Some(&s[idx + 1..]))
        } else {
            (s, None)
        };

        // 解析日期部分：[n]Y[n]M[n]W[n]D
        let mut remaining = date_part;
        // 循环读取"数字+标识符"对，直至日期部分耗尽
        while !remaining.is_empty() {
            // 定位数字段末尾（首个非数字且非小数点/逗号的字符）
            let num_end = remaining
                .find(|c: char| !c.is_ascii_digit() && c != '.' && c != ',')
                .unwrap_or(remaining.len());
            if num_end == 0 {
                break;
            }
            // 逗号视作小数点，解析为浮点数量；非法数字退为 0
            let num_str = remaining[..num_end].replace(',', ".");
            let num: f64 = num_str.parse().unwrap_or(0.0);
            let designator = remaining.as_bytes()[num_end] as char;
            remaining = &remaining[num_end + 1..];
            match designator {
                'Y' => result.year = num,
                'M' => result.month = num,
                'W' => result.day += num * 7.0,
                'D' => result.day += num,
                _ => {}
            }
        }

        // 解析时间部分：[n]H[n]M[n]S
        if let Some(tp) = time_part {
            let mut remaining = tp;
            // 与日期部分同构：循环读取"数字+标识符"对直至耗尽
            while !remaining.is_empty() {
                let num_end = remaining
                    .find(|c: char| !c.is_ascii_digit() && c != '.' && c != ',')
                    .unwrap_or(remaining.len());
                if num_end == 0 {
                    break;
                }
                let num_str = remaining[..num_end].replace(',', ".");
                let num: f64 = num_str.parse().unwrap_or(0.0);
                let designator = remaining.as_bytes()[num_end] as char;
                remaining = &remaining[num_end + 1..];
                match designator {
                    'H' => result.hour = num,
                    'M' => result.minute = num,
                    'S' => {
                        // 秒分量取整，小数部分折算为毫秒
                        result.second = num.floor();
                        result.millisecond = (num % 1.0) * 1000.0;
                    }
                    _ => {}
                }
            }
        }
    } else {
        // 基于日期的时长：解析为日期并提取 GregorianDate 各分量
        let s = if iso8601.ends_with('Z') {
            iso8601.to_string()
        } else {
            format!("{}Z", iso8601)
        };
        let jd = JulianDate::from_iso8601(&s)?;
        let g = jd.to_gregorian_date();
        result.year = g.year as f64;
        result.month = g.month as f64;
        result.day = g.day as f64;
        result.hour = g.hour as f64;
        result.minute = g.minute as f64;
        result.second = g.second as f64;
        result.millisecond = g.millisecond;
    }

    // 零时长等价于"无有效时长"，返回 None
    if result.is_zero() {
        None
    } else {
        Some(result)
    }
}

/// 将一个时长（以 GregorianDate 各分量表示）加到一个 JulianDate 上。
/// 先把各分量与日期分量相加，再自毫秒向上逐级进位（秒/分/时）。
/// 最后循环处理日对月、月对年的溢出，重新组装为有效的格里高利日期。
fn add_to_date(julian_date: &JulianDate, duration: &Duration) -> JulianDate {
    let g = julian_date.to_gregorian_date();

    // 先把时长的各分量直接累加到对应日期分量上（未进位）
    let mut millisecond = g.millisecond + duration.millisecond;
    let mut second = g.second as f64 + duration.second;
    let mut minute = g.minute as f64 + duration.minute;
    let mut hour = g.hour as f64 + duration.hour;
    let mut day = g.day as f64 + duration.day;
    // deferred.md #13: month/year 之后未被重新赋值（改用 month_i/year_i），mut 冗余。
    #[allow(unused_mut)]
    let mut month = g.month as f64 + duration.month;
    #[allow(unused_mut)]
    let mut year = g.year as f64 + duration.year;

    // 毫秒向秒进位
    if millisecond >= 1000.0 {
        second += (millisecond / 1000.0).floor();
        millisecond %= 1000.0;
    }
    // 秒向分进位
    if second >= 60.0 {
        minute += (second / 60.0).floor();
        second %= 60.0;
    }
    // 分向时进位
    if minute >= 60.0 {
        hour += (minute / 60.0).floor();
        minute %= 60.0;
    }
    // 时向日进位
    if hour >= 24.0 {
        day += (hour / 24.0).floor();
        hour %= 24.0;
    }

    // 调整日/月/年
    let mut year_i = year as i32;
    let mut month_i = month as u32;
    let mut day_i = day as u32;

    // deferred.md #13: 手动范围判断 (m >= 1 && m <= 12)，等价 (1..=12).contains(&m)。
    #[allow(clippy::manual_range_contains)]
    let month_len = |y: i32, m: u32| -> u32 {
        if m >= 1 && m <= 12 {
            days_in_month(y, m)
        } else {
            31
        }
    };

    // 归一化：月超过 12 则进年，日超过当月天数则进月
    while day_i > month_len(year_i, month_i) || month_i >= 13 {
        // deferred.md #13: month_i = month_i % 12 可用 %= 简写（属性置于 if 块，赋值语句不支持属性）。
        #[allow(clippy::assign_op_pattern)]
        if month_i >= 13 {
            month_i -= 1;
            year_i += (month_i / 12) as i32;
            month_i = month_i % 12;
            month_i += 1;
        }
        // 日超出当月天数则借位进月
        if day_i > month_len(year_i, month_i) {
            day_i -= month_len(year_i, month_i);
            month_i += 1;
        }
    }

    // 用归一后的分量组装结果日期，再回到 JulianDate
    let result_g = GregorianDate {
        year: year_i,
        month: month_i,
        day: day_i,
        hour: hour as u32,
        minute: minute as u32,
        second: second as u32,
        millisecond,
        is_leap_second: false,
    };
    JulianDate::from_gregorian_date(&result_g)
}

/// `from_iso8601` 及相关构造函数的选项。
pub struct FromIso8601Options {
    /// ISO8601 区间字符串（"start/stop" 或 "start/stop/duration"）。
    pub iso8601: String,
    /// 是否包含开始时间（默认 true）。
    pub is_start_included: Option<bool>,
    /// 是否包含停止时间（默认 true）。
    pub is_stop_included: Option<bool>,
    /// 添加一个从 MINIMUM_VALUE 到 start 的前置区间。
    pub leading_interval: bool,
    /// 添加一个从 stop 到 MAXIMUM_VALUE 的后置区间。
    pub trailing_interval: bool,
}

impl<T> TimeIntervalCollection<T> {
    /// 从 ISO8601 区间字符串创建集合。
    /// 解析 "start/stop" 或 "start/stop/duration"：给出时长时按步长
    /// 切分 [start, stop]，否则仅端点两段；再委托数组构造函数。
    pub fn from_iso8601<F>(options: &FromIso8601Options, same_data: &F) -> Self
    where
        T: Clone + From<usize>,
        F: Fn(&T, &T) -> bool,
    {
        // 以 '/' 拆分出起止（及可选时长）段
        let parts: Vec<&str> = options.iso8601.split('/').collect();
        // 首两段分别解析为区间的起止端点
        let start = JulianDate::from_iso8601(parts[0]).unwrap();
        let stop = JulianDate::from_iso8601(parts[1]).unwrap();

        // 收集所有切分得到的边界时刻
        let mut julian_dates: Vec<JulianDate> = Vec::new();

        // 第三段（若存在）是时长，决定是否需要按步长切分
        let duration = if parts.len() > 2 {
            parse_duration(Some(parts[2]))
        } else {
            None
        };

        match duration {
            // 无时长：只保留两个端点
            None => {
                julian_dates.push(start);
                julian_dates.push(stop);
            }
            // 有时长：从 start 起反复叠加，越界则钳制到 stop
            Some(dur) => {
                let mut date = start;
                julian_dates.push(date);
                while date < stop {
                    date = add_to_date(&date, &dur);
                    // 末段超出 stop 时钳制，避免越界
                    if stop <= date {
                        date = stop;
                    }
                    julian_dates.push(date);
                }
            }
        }

        Self::from_julian_date_array(
            &julian_dates,
            options.is_start_included.unwrap_or(true),
            options.is_stop_included.unwrap_or(true),
            options.leading_interval,
            options.trailing_interval,
            same_data,
        )
    }

    /// 从 JulianDate 数组创建集合。
    /// 相邻边界两两成段；可选地在首尾附加延伸到极值的 leading/trailing 区间。
    pub fn from_julian_date_array<F>(
        julian_dates: &[JulianDate],
        is_start_included: bool,
        is_stop_included: bool,
        leading_interval: bool,
        trailing_interval: bool,
        same_data: &F,
    ) -> Self
    where
        T: Clone + From<usize>,
        F: Fn(&T, &T) -> bool,
    {
        let mut result = Self::new();
        let length = julian_dates.len();
        // 少于两个端点无法构成任何区间
        if length < 2 {
            return result;
        }

        // 首段的"起点闭合"仅在无前置区间时生效
        let start_index: usize = if leading_interval { 1 } else { 0 };

        // 前置区间：从 ISO8601 最小值延伸到第一个端点
        if leading_interval {
            let interval = TimeIntervalData {
                interval: TimeInterval::new(
                    iso8601_minimum_value(),
                    julian_dates[0],
                    true,
                    !is_start_included,
                ),
                data: Some(T::from(result.len())),
            };
            result.add_interval(interval, same_data);
        }

        // 相邻端点两两成段
        for i in 0..length - 1 {
            let start_date = julian_dates[i];
            let end_date = julian_dates[i + 1];
            // 仅首段按调用方要求决定左闭；其余内部段一律左闭
            let isi = if result.len() == start_index {
                is_start_included
            } else {
                true
            };
            // 仅末段按调用方要求决定右闭；其余内部段右开
            let isti = if i == length - 2 {
                is_stop_included
            } else {
                false
            };
            let interval = TimeIntervalData {
                interval: TimeInterval::new(start_date, end_date, isi, isti),
                data: Some(T::from(result.len())),
            };
            result.add_interval(interval, same_data);
        }

        // 后置区间：从最后一个端点延伸到 ISO8601 最大值
        if trailing_interval {
            let interval = TimeIntervalData {
                interval: TimeInterval::new(
                    julian_dates[length - 1],
                    iso8601_maximum_value(),
                    !is_stop_included,
                    true,
                ),
                data: Some(T::from(result.len())),
            };
            result.add_interval(interval, same_data);
        }

        result
    }

    /// 从一个相对于纪元的 ISO8601 时长字符串数组创建集合。
    /// 每个时长可相对纪元或前一个日期累加（由 relative_to_previous 决定），
    /// 得到一组边界时刻后委托数组构造函数生成集合。
    // deferred.md #13: 参数较多（8/7），保持与时长数组构造签名一一对应。
    #[allow(clippy::too_many_arguments)]
    pub fn from_iso8601_duration_array<F>(
        epoch: &JulianDate,
        iso8601_durations: &[&str],
        relative_to_previous: bool,
        is_start_included: bool,
        is_stop_included: bool,
        leading_interval: bool,
        trailing_interval: bool,
        same_data: &F,
    ) -> Self
    where
        T: Clone + From<usize>,
        F: Fn(&T, &T) -> bool,
    {
        // 收集由时长累加得到的边界时刻
        let mut julian_dates: Vec<JulianDate> = Vec::new();
        // 记录上一个边界，供相对累加模式作为基准
        let mut previous_date: Option<JulianDate> = None;

        // 逐个解析时长字符串并叠加到相应基准日期上
        for (i, dur_str) in iso8601_durations.iter().enumerate() {
            let dur = parse_duration(Some(dur_str));
            // 允许首次迭代时时长为 0（它仅是纪元）
            if dur.is_some() || i == 0 {
                let effective_dur = dur.unwrap_or_default();
                // 相对模式基于前一个日期累加，否则一律基于纪元累加
                let date = if relative_to_previous {
                    if let Some(prev) = previous_date {
                        add_to_date(&prev, &effective_dur)
                    } else {
                        add_to_date(epoch, &effective_dur)
                    }
                } else {
                    add_to_date(epoch, &effective_dur)
                };
                julian_dates.push(date);
                previous_date = Some(date);
            }
        }

        Self::from_julian_date_array(
            &julian_dates,
            is_start_included,
            is_stop_included,
            leading_interval,
            trailing_interval,
            same_data,
        )
    }
}

/// 比较两个可选数据负载。两个 `None` 视为相等；一个 `None` 与一个
/// `Some` 视为不等；两个 `Some` 用 `same_data` 比较。
fn data_equals<T, F>(a: Option<&T>, b: Option<&T>, same_data: &F) -> bool
where
    F: Fn(&T, &T) -> bool,
{
    // 两侧皆无数据视为相等；皆有则用 same_data；一方有一方无则不等
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => same_data(x, y),
        _ => false,
    }
}

/// 比较两个儒略日的先后，作为区间边界重叠判定的辅助。
fn compare(a: &JulianDate, b: &JulianDate) -> Ordering {
    a.cmp(b)
}

/// 对各区间的开始时间进行二分查找。返回 start 等于 `time` 的区间索引，
/// 或插入索引的按位取反。
fn binary_search_start<T>(intervals: &[TimeIntervalData<T>], time: &JulianDate) -> isize {
    // 在 [low, high] 闭区间上二分
    let mut low: isize = 0;
    let mut high: isize = intervals.len() as isize - 1;

    while low <= high {
        let mid = (low + high) / 2;
        let mid_start = &intervals[mid as usize].interval.start;
        // 命中返回索引，否则收缩到左半或右半
        match mid_start.cmp(time) {
            Ordering::Equal => return mid,
            Ordering::Less => low = mid + 1,
            Ordering::Greater => high = mid - 1,
        }
    }
    // 未命中：返回插入点 low 的按位取反
    !low
}

#[cfg(test)]
mod tests {
    use super::*;

    // 构造一个仅指定年月日时的儒略日，时分秒归零，便于测试书写边界时刻。
    fn jd(year: i32, month: u32, day: u32, hour: u32) -> JulianDate {
        JulianDate::from_date_components(year, month, day, hour, 0, 0, 0.0)
    }

    // 组装一个左闭右开、携带整数数据的区间条目，作为测试的统一输入形态。
    fn iv(start: JulianDate, stop: JulianDate, data: i32) -> TimeIntervalData<i32> {
        TimeIntervalData::new(TimeInterval::new(start, stop, true, false), Some(data))
    }

    // 数据相等判定：整数负载直接按值比较，决定相邻区间能否合并。
    fn same(a: &i32, b: &i32) -> bool {
        a == b
    }

    // 连续添加两个数据不同的相邻区间：二者各自独立保留，长度应为 2；
    // 整体起点取首区间、止点取末区间，左闭右开标志也分别继承自首/末条目。
    #[test]
    fn test_add_and_length() {
        let mut c = TimeIntervalCollection::new();
        assert!(c.is_empty());
        c.add_interval(iv(jd(2012, 8, 1, 0), jd(2012, 8, 1, 6), 1), &same);
        c.add_interval(iv(jd(2012, 8, 1, 6), jd(2012, 8, 1, 12), 2), &same);
        assert_eq!(c.len(), 2);
        assert_eq!(c.start(), Some(jd(2012, 8, 1, 0)));
        assert_eq!(c.stop(), Some(jd(2012, 8, 1, 12)));
        assert!(c.is_start_included());
        assert!(!c.is_stop_included());
    }

    // 携带相同数据的相邻区间在添加时会被折叠：0-6 与 6-12 合并为单一
    // 0-12 区间，集合长度降为 1，起止边界随之扩展到合并后的两端。
    #[test]
    fn test_merge_same_data() {
        let mut c = TimeIntervalCollection::new();
        c.add_interval(iv(jd(2012, 8, 1, 0), jd(2012, 8, 1, 6), 1), &same);
        // 携带相同数据的相邻区间应当合并。
        c.add_interval(iv(jd(2012, 8, 1, 6), jd(2012, 8, 1, 12), 1), &same);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get(0).unwrap().interval.start, jd(2012, 8, 1, 0));
        assert_eq!(c.get(0).unwrap().interval.stop, jd(2012, 8, 1, 12));
    }

    // 相邻但数据不同的区间不应合并：0-6 数据 1 与 6-12 数据 2 保持独立，
    // 长度维持为 2，以验证合并且仅发生在数据相等的情形。
    #[test]
    fn test_no_merge_different_data() {
        let mut c = TimeIntervalCollection::new();
        c.add_interval(iv(jd(2012, 8, 1, 0), jd(2012, 8, 1, 6), 1), &same);
        c.add_interval(iv(jd(2012, 8, 1, 6), jd(2012, 8, 1, 12), 2), &same);
        assert_eq!(c.len(), 2);
    }

    // 在已有 0-12 数据 1 的区间中部插入数据不同的 4-8 新区间：旧区间被
    // 截断为 0-4 与 8-12 两段，新段居中且数据优先，最终顺序为 1、2、1，
    // 中间段边界恰为插入区间的 4 与 8。
    #[test]
    fn test_overlapping_new_wins() {
        let mut c = TimeIntervalCollection::new();
        c.add_interval(iv(jd(2012, 8, 1, 0), jd(2012, 8, 1, 12), 1), &same);
        // 中间插入的数据不同的新区间会截断旧区间。
        c.add_interval(iv(jd(2012, 8, 1, 4), jd(2012, 8, 1, 8), 2), &same);
        assert_eq!(c.len(), 3);
        assert_eq!(c.get(0).unwrap().data, Some(1));
        assert_eq!(c.get(1).unwrap().data, Some(2));
        assert_eq!(c.get(2).unwrap().data, Some(1));
        assert_eq!(c.get(1).unwrap().interval.start, jd(2012, 8, 1, 4));
        assert_eq!(c.get(1).unwrap().interval.stop, jd(2012, 8, 1, 8));
    }

    // index_of 依据二分定位日期所属区间：3 时落在索引 0、9 时落在索引 1；
    // contains 对内部点为真，而对恰等于开区间止点的 12 时及范围外日期为假。
    #[test]
    fn test_index_of_and_contains() {
        let mut c = TimeIntervalCollection::new();
        c.add_interval(iv(jd(2012, 8, 1, 0), jd(2012, 8, 1, 6), 1), &same);
        c.add_interval(iv(jd(2012, 8, 1, 6), jd(2012, 8, 1, 12), 2), &same);

        assert_eq!(c.index_of(&jd(2012, 8, 1, 3)), 0);
        assert_eq!(c.index_of(&jd(2012, 8, 1, 9)), 1);
        assert!(c.contains(&jd(2012, 8, 1, 3)));
        // 停止时间为开区间，因此 12:00 不被包含。
        assert!(!c.contains(&jd(2012, 8, 1, 12)));
        assert!(!c.contains(&jd(2012, 8, 2, 0)));
    }

    // 按日期检索其所属区间的数据负载：区间内分别取回 10、20，
    // 范围外日期无匹配区间则返回 None，验证查找与数据访问的联动。
    #[test]
    fn test_find_data_for_interval_containing_date() {
        let mut c = TimeIntervalCollection::new();
        c.add_interval(iv(jd(2012, 8, 1, 0), jd(2012, 8, 1, 6), 10), &same);
        c.add_interval(iv(jd(2012, 8, 1, 6), jd(2012, 8, 1, 12), 20), &same);

        assert_eq!(
            c.find_data_for_interval_containing_date(&jd(2012, 8, 1, 3)),
            Some(&10)
        );
        assert_eq!(
            c.find_data_for_interval_containing_date(&jd(2012, 8, 1, 9)),
            Some(&20)
        );
        assert_eq!(
            c.find_data_for_interval_containing_date(&jd(2012, 8, 2, 0)),
            None
        );
    }

    // 从 0-12 的单一区间中挖除中部 4-8：返回移除成功，集合裂为两段，
    // 空洞处 6 时不再被包含，而两侧的 2 时与 10 时仍在覆盖范围内。
    #[test]
    fn test_remove_interval_hole() {
        let mut c = TimeIntervalCollection::new();
        c.add_interval(iv(jd(2012, 8, 1, 0), jd(2012, 8, 1, 12), 1), &same);
        let removed = c.remove_interval(&TimeInterval::new(
            jd(2012, 8, 1, 4),
            jd(2012, 8, 1, 8),
            true,
            true,
        ));
        assert!(removed);
        assert_eq!(c.len(), 2);
        assert!(!c.contains(&jd(2012, 8, 1, 6)));
        assert!(c.contains(&jd(2012, 8, 1, 2)));
        assert!(c.contains(&jd(2012, 8, 1, 10)));
    }

    // remove_all 清空全部区间：添加一条后调用应使集合回到空状态。
    #[test]
    fn test_remove_all() {
        let mut c = TimeIntervalCollection::new();
        c.add_interval(iv(jd(2012, 8, 1, 0), jd(2012, 8, 1, 6), 1), &same);
        c.remove_all();
        assert!(c.is_empty());
    }

    // 两集合求交：a 覆盖 0-12、b 覆盖 6-次日 0，交集为重叠段 6-12，
    // 结果只含一个区间，起止分别取较晚起点与较早止点。
    #[test]
    fn test_intersect() {
        let mut a = TimeIntervalCollection::new();
        a.add_interval(iv(jd(2012, 8, 1, 0), jd(2012, 8, 1, 12), 1), &same);

        let mut b = TimeIntervalCollection::new();
        b.add_interval(iv(jd(2012, 8, 1, 6), jd(2012, 8, 2, 0), 1), &same);

        let inter = a.intersect(&b, &same);
        assert_eq!(inter.len(), 1);
        assert_eq!(inter.get(0).unwrap().interval.start, jd(2012, 8, 1, 6));
        assert_eq!(inter.get(0).unwrap().interval.stop, jd(2012, 8, 1, 12));
    }

    // 相等比较：条目与数据完全一致时 equals 为真；给一侧追加新区间后
    // 结构不再对称，比较随即转为假。
    #[test]
    fn test_equals() {
        let mut a = TimeIntervalCollection::new();
        a.add_interval(iv(jd(2012, 8, 1, 0), jd(2012, 8, 1, 6), 1), &same);
        let mut b = TimeIntervalCollection::new();
        b.add_interval(iv(jd(2012, 8, 1, 0), jd(2012, 8, 1, 6), 1), &same);
        assert!(a.equals(&b, &same));

        b.add_interval(iv(jd(2012, 8, 1, 6), jd(2012, 8, 1, 12), 2), &same);
        assert!(!a.equals(&b, &same));
    }

    // 反向（止点早于起点）的空区间在添加时被直接忽略：集合保持为空，
    // 验证空区间不进入有序列表的入口守卫。
    #[test]
    fn test_empty_interval_ignored() {
        let mut c = TimeIntervalCollection::new();
        let empty = TimeIntervalData::new(
            TimeInterval::new(jd(2012, 8, 1, 6), jd(2012, 8, 1, 0), true, true),
            Some(1),
        );
        c.add_interval(empty, &same);
        assert!(c.is_empty());
    }
}
