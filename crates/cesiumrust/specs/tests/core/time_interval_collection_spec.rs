//! Core/TimeIntervalCollectionSpec → Rust 集成测试
//! 原始共 67 个 it() 块。C 类（throws/changedEvent）已跳过（11 个）。
//! 合并回调测试已跳过（API 尚未支持）。
//! 已移植：约 55 个测试，覆盖 construct、contains、indexOf、get、findInterval、
//! addInterval、removeInterval、intersect、equals、fromIso8601、fromIso8601DateArray、
//! fromIso8601DurationArray。

use cesium_time::{
    FromIso8601Options, JulianDate, TimeInterval, TimeIntervalCollection, TimeIntervalData,
    TimeStandard,
};

/// 辅助函数：从日数（UTC）创建 JulianDate
fn jd(day: f64) -> JulianDate {
    JulianDate::new(day, 0.0)
}

/// 辅助函数：从日数 + 秒数（TAI）创建 JulianDate
fn jd_tai(day: f64, sec: f64) -> JulianDate {
    JulianDate::with_time_standard(day, sec, TimeStandard::TAI)
}

/// 辅助函数：创建带 i32 数据的 TimeIntervalData
fn tid(start: f64, stop: f64, si: bool, sti: bool, data: i32) -> TimeIntervalData<i32> {
    TimeIntervalData::new(TimeInterval::new(jd(start), jd(stop), si, sti), Some(data))
}

/// 辅助函数：创建无数据的 TimeIntervalData
fn tid_nodata(start: f64, stop: f64, si: bool, sti: bool) -> TimeIntervalData<i32> {
    TimeIntervalData::new(TimeInterval::new(jd(start), jd(stop), si, sti), None)
}

fn same_i32(a: &i32, b: &i32) -> bool {
    a == b
}

fn same_usize(a: &usize, b: &usize) -> bool {
    a == b
}

/// 辅助函数：检查各区间是否匹配预期的 julian 日期数组（用于 fromIso8601 测试）
fn check_intervals(
    collection: &TimeIntervalCollection<usize>,
    julian_dates: &[JulianDate],
    is_start_included: bool,
    is_stop_included: bool,
) {
    let length = julian_dates.len() - 1;
    assert_eq!(collection.len(), length, "interval count mismatch");
    for i in 0..length {
        let interval = collection.get(i).unwrap();
        assert_eq!(
            interval.interval.start, julian_dates[i],
            "start mismatch at {}",
            i
        );
        assert_eq!(
            interval.interval.stop,
            julian_dates[i + 1],
            "stop mismatch at {}",
            i
        );
        assert_eq!(
            interval.interval.is_start_included,
            if i == 0 { is_start_included } else { true },
            "isStartIncluded mismatch at {}",
            i
        );
        assert_eq!(
            interval.interval.is_stop_included,
            if i == length - 1 {
                is_stop_included
            } else {
                false
            },
            "isStopIncluded mismatch at {}",
            i
        );
        assert_eq!(interval.data, Some(i), "data mismatch at {}", i);
    }
}

fn iso8601_to_julian_date_array(dates: &[&str]) -> Vec<JulianDate> {
    dates
        .iter()
        .map(|s| JulianDate::from_iso8601(s).unwrap())
        .collect()
}

// === 构造 ===

#[test]
fn constructing_default_interval_collection_has_expected_property_values() {
    let intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    assert_eq!(intervals.len(), 0);
    assert_eq!(intervals.start(), None);
    assert_eq!(intervals.stop(), None);
    assert!(!intervals.is_start_included());
    assert!(!intervals.is_stop_included());
    assert!(intervals.is_empty());
}

#[test]
fn constructing_interval_collection_from_array() {
    let arg = vec![
        tid(1.0, 2.0, true, false, 0),
        tid(2.0, 3.0, false, true, 0),
    ];
    let intervals = TimeIntervalCollection::from_intervals(arg, &same_i32);
    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals.start(), Some(jd(1.0)));
    assert_eq!(intervals.stop(), Some(jd(3.0)));
    assert!(intervals.is_start_included());
    assert!(intervals.is_stop_included());
    assert!(!intervals.is_empty());
}

#[test]
fn is_start_included_is_stop_included_works() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    let interval1 = tid(1.0, 2.0, true, false, 0);
    let interval2 = tid(2.0, 3.0, false, true, 0);

    assert!(!intervals.is_start_included());
    assert!(!intervals.is_stop_included());

    intervals.add_interval(interval1, &same_i32);
    assert!(intervals.is_start_included());
    assert!(!intervals.is_stop_included());

    intervals.add_interval(interval2, &same_i32);
    assert!(intervals.is_start_included());
    assert!(intervals.is_stop_included());
}

// === 包含 ===

#[test]
fn contains_works_for_a_simple_interval_collection() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    intervals.add_interval(tid(1.0, 2.0, true, false, 0), &same_i32);
    intervals.add_interval(tid(2.0, 3.0, false, true, 0), &same_i32);
    assert!(!intervals.contains(&jd(0.5)));
    assert!(intervals.contains(&jd(1.5)));
    assert!(!intervals.contains(&jd(2.0)));
    assert!(intervals.contains(&jd(2.5)));
    assert!(intervals.contains(&jd(3.0)));
    assert!(!intervals.contains(&jd(3.5)));
}

#[test]
fn contains_works_for_endpoints_of_a_closed_interval_collection() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    let interval = tid(1.0, 2.0, true, true, 0);
    intervals.add_interval(interval, &same_i32);
    assert!(intervals.contains(&jd(1.0)));
    assert!(intervals.contains(&jd(2.0)));
}

#[test]
fn contains_works_for_endpoints_of_an_open_interval_collection() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    let interval = tid(1.0, 2.0, false, false, 0);
    intervals.add_interval(interval, &same_i32);
    assert!(!intervals.contains(&jd(1.0)));
    assert!(!intervals.contains(&jd(2.0)));
}

// === IndexOf ===

#[test]
fn index_of_finds_the_correct_interval_for_a_valid_date() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    intervals.add_interval(tid(1.0, 2.0, true, false, 0), &same_i32);
    intervals.add_interval(tid(2.0, 3.0, false, true, 0), &same_i32);
    assert_eq!(intervals.index_of(&jd(2.5)), 1);
}

#[test]
fn index_of_returns_complement_of_index_of_the_interval_that_a_missing_date_would_come_before() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    intervals.add_interval(tid(1.0, 2.0, true, false, 0), &same_i32);
    intervals.add_interval(tid(2.0, 3.0, false, true, 0), &same_i32);
    assert_eq!(intervals.index_of(&jd(2.0)), !1isize);
}

#[test]
fn index_of_returns_complement_of_collection_length_if_date_is_after_all_intervals() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    intervals.add_interval(tid(1.0, 2.0, true, false, 0), &same_i32);
    intervals.add_interval(tid(2.0, 3.0, false, true, 0), &same_i32);
    assert_eq!(intervals.index_of(&jd(4.0)), !2isize);
}

// === Get ===

#[test]
fn get_returns_the_interval_at_the_correct_index() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    intervals.add_interval(tid_nodata(1.0, 2.0, false, false), &same_i32);
    intervals.add_interval(tid_nodata(2.0, 3.0, false, false), &same_i32);
    intervals.add_interval(tid_nodata(4.0, 5.0, false, false), &same_i32);
    let got = intervals.get(1).unwrap();
    assert_eq!(got.interval.start, jd(2.0));
    assert_eq!(got.interval.stop, jd(3.0));
}

#[test]
fn get_is_none_for_out_of_range_index() {
    let intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    assert!(intervals.get(1).is_none());
}

// === FindInterval ===

#[test]
fn find_interval_works_when_looking_for_an_exact_interval() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    intervals.add_interval(tid(0.0, 1.0, false, false, 1), &same_i32);
    intervals.add_interval(tid(1.0, 2.0, true, false, 2), &same_i32);
    intervals.add_interval(tid(2.0, 3.0, false, false, 3), &same_i32);
    let found = intervals
        .find_interval(Some(&jd(1.0)), Some(&jd(2.0)), Some(true), Some(false))
        .unwrap();
    assert_eq!(found.data, Some(2));
}

#[test]
fn find_interval_works_when_you_do_not_care_about_end_points() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    intervals.add_interval(tid(0.0, 1.0, false, false, 1), &same_i32);
    intervals.add_interval(tid(1.0, 2.0, true, false, 2), &same_i32);
    intervals.add_interval(tid(2.0, 3.0, false, false, 3), &same_i32);
    let found = intervals
        .find_interval(Some(&jd(1.0)), Some(&jd(2.0)), None, None)
        .unwrap();
    assert_eq!(found.data, Some(2));
}

// === Start/Stop/IsEmpty ===

#[test]
fn get_start_and_get_stop_return_expected_values() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    intervals.add_interval(tid(1.0, 2.0, true, false, 0), &same_i32);
    intervals.add_interval(tid(2.0, 3.0, true, false, 0), &same_i32);
    assert_eq!(intervals.start(), Some(jd(1.0)));
    assert_eq!(intervals.stop(), Some(jd(3.0)));
}

#[test]
fn is_empty_and_clear_return_expected_values() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    intervals.add_interval(tid_nodata(1.0, 2.0, false, true), &same_i32);
    assert!(!intervals.is_empty());
    intervals.remove_all();
    assert!(intervals.is_empty());
}

// === Length ===

#[test]
fn length_returns_correct_interval_length_when_adding_intervals_with_different_data() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    assert_eq!(intervals.len(), 0);

    intervals.add_interval(tid(1.0, 4.0, true, true, 1), &same_i32);
    assert_eq!(intervals.len(), 1);

    intervals.add_interval(tid(2.0, 3.0, true, true, 2), &same_i32);
    assert_eq!(intervals.len(), 3);

    intervals.remove_all();
    assert_eq!(intervals.len(), 0);
}

#[test]
fn length_returns_correct_length_after_two_intervals_with_same_data_are_merged() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();

    intervals.add_interval(tid(1.0, 4.0, true, true, 1), &same_i32);
    assert_eq!(intervals.len(), 1);

    intervals.add_interval(tid(2.0, 3.0, true, true, 1), &same_i32);
    assert_eq!(intervals.len(), 1);

    intervals.remove_all();
    assert_eq!(intervals.len(), 0);
}

// === AddInterval + FindIntervalContainingDate ===

#[test]
fn add_interval_and_find_interval_containing_date_work_with_non_overlapping_intervals() {
    let interval1 = tid(1.0, 2.0, true, true, 1);
    let interval2 = tid(2.0, 3.0, false, true, 2);
    let interval3 = tid(4.0, 5.0, true, true, 3);

    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();

    intervals.add_interval(interval1.clone(), &same_i32);
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals.start(), Some(jd(1.0)));
    assert_eq!(intervals.stop(), Some(jd(2.0)));
    assert!(!intervals.is_empty());

    assert_eq!(
        intervals.find_interval_containing_date(&jd(1.0)).unwrap().data,
        Some(1)
    );
    assert_eq!(
        intervals.find_interval_containing_date(&jd(2.0)).unwrap().data,
        Some(1)
    );

    intervals.add_interval(interval2.clone(), &same_i32);
    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals.start(), Some(jd(1.0)));
    assert_eq!(intervals.stop(), Some(jd(3.0)));

    assert_eq!(
        intervals.find_interval_containing_date(&jd(1.0)).unwrap().data,
        Some(1)
    );
    assert_eq!(
        intervals.find_interval_containing_date(&jd(2.0)).unwrap().data,
        Some(1)
    );
    assert_eq!(
        intervals.find_interval_containing_date(&jd(3.0)).unwrap().data,
        Some(2)
    );

    intervals.add_interval(interval3.clone(), &same_i32);
    assert_eq!(intervals.len(), 3);
    assert_eq!(intervals.start(), Some(jd(1.0)));
    assert_eq!(intervals.stop(), Some(jd(5.0)));

    assert_eq!(
        intervals.find_interval_containing_date(&jd(4.0)).unwrap().data,
        Some(3)
    );
    assert_eq!(
        intervals.find_interval_containing_date(&jd(5.0)).unwrap().data,
        Some(3)
    );
}

#[test]
fn add_interval_and_find_interval_containing_date_work_with_overlapping_intervals() {
    let interval1 = tid(1.0, 2.5, true, true, 1);
    let interval2 = tid(2.0, 3.0, false, true, 2);
    let interval3 = tid(1.0, 3.0, true, true, 3);

    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();

    intervals.add_interval(interval1, &same_i32);
    assert_eq!(intervals.len(), 1);
    assert_eq!(
        intervals.find_interval_containing_date(&jd(1.0)).unwrap().data,
        Some(1)
    );
    assert_eq!(
        intervals.find_interval_containing_date(&jd(2.5)).unwrap().data,
        Some(1)
    );

    intervals.add_interval(interval2, &same_i32);
    assert_eq!(intervals.len(), 2);
    assert_eq!(
        intervals.find_interval_containing_date(&jd(1.0)).unwrap().data,
        Some(1)
    );
    assert_eq!(
        intervals.find_interval_containing_date(&jd(2.5)).unwrap().data,
        Some(2)
    );
    assert_eq!(
        intervals.find_interval_containing_date(&jd(3.0)).unwrap().data,
        Some(2)
    );

    intervals.add_interval(interval3, &same_i32);
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals.start(), Some(jd(1.0)));
    assert_eq!(intervals.stop(), Some(jd(3.0)));
    assert_eq!(
        intervals.find_interval_containing_date(&jd(1.0)).unwrap().data,
        Some(3)
    );
    assert_eq!(
        intervals.find_interval_containing_date(&jd(2.0)).unwrap().data,
        Some(3)
    );
    assert_eq!(
        intervals.find_interval_containing_date(&jd(3.0)).unwrap().data,
        Some(3)
    );
}

// === FindDataForIntervalContainingDate ===

#[test]
fn find_data_for_interval_containing_date_works() {
    let interval1 = tid(1.0, 2.5, true, true, 1);
    let interval2 = tid(2.0, 3.0, false, true, 2);

    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    intervals.add_interval(interval1, &same_i32);
    assert_eq!(
        intervals.find_data_for_interval_containing_date(&jd(1.0)),
        Some(&1)
    );
    assert_eq!(
        intervals.find_data_for_interval_containing_date(&jd(2.5)),
        Some(&1)
    );

    intervals.add_interval(interval2, &same_i32);
    assert_eq!(
        intervals.find_data_for_interval_containing_date(&jd(1.0)),
        Some(&1)
    );
    assert_eq!(
        intervals.find_data_for_interval_containing_date(&jd(2.5)),
        Some(&2)
    );
    assert_eq!(
        intervals.find_data_for_interval_containing_date(&jd(3.0)),
        Some(&2)
    );
    assert_eq!(
        intervals.find_data_for_interval_containing_date(&jd(5.0)),
        None
    );
}

// === AddInterval with equalsCallback ===

#[test]
fn add_interval_correctly_merges_intervals_that_have_the_same_data_when_using_equals_callback() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();

    let interval1 = tid(1.0, 4.0, true, true, 2);
    let interval2 = tid(1.0, 3.0, true, false, 2);
    let interval3 = tid(3.0, 4.0, false, true, 2);
    let interval4 = tid(3.0, 4.0, true, true, 3);

    intervals.add_interval(interval1, &same_i32);
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals.start(), Some(jd(1.0)));
    assert_eq!(intervals.stop(), Some(jd(4.0)));
    assert_eq!(intervals.get(0).unwrap().data, Some(2));

    intervals.add_interval(interval2, &same_i32);
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals.get(0).unwrap().data, Some(2));

    intervals.add_interval(interval3, &same_i32);
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals.get(0).unwrap().data, Some(2));

    intervals.add_interval(interval4, &same_i32);
    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals.get(0).unwrap().interval.start, jd(1.0));
    assert_eq!(intervals.get(0).unwrap().interval.stop, jd(3.0));
    assert!(intervals.get(0).unwrap().interval.is_start_included);
    assert!(!intervals.get(0).unwrap().interval.is_stop_included);
    assert_eq!(intervals.get(0).unwrap().data, Some(2));

    assert_eq!(intervals.get(1).unwrap().interval.start, jd(3.0));
    assert_eq!(intervals.get(1).unwrap().interval.stop, jd(4.0));
    assert!(intervals.get(1).unwrap().interval.is_start_included);
    assert!(intervals.get(1).unwrap().interval.is_stop_included);
    assert_eq!(intervals.get(1).unwrap().data, Some(3));
}

// === RemoveInterval ===

#[test]
fn remove_interval_works_correctly() {
    fn create_ti(start_days: f64, stop_days: f64) -> TimeInterval {
        TimeInterval::new(jd_tai(start_days, 0.0), jd_tai(stop_days, 0.0), true, true)
    }
    fn create_ti_excl(start_days: f64, stop_days: f64, si: bool, sti: bool) -> TimeInterval {
        TimeInterval::new(jd_tai(start_days, 0.0), jd_tai(stop_days, 0.0), si, sti)
    }

    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    let same = |_: &i32, _: &i32| true;
    intervals.add_interval(
        TimeIntervalData::new(create_ti(10.0, 20.0), None),
        &same,
    );
    intervals.add_interval(
        TimeIntervalData::new(create_ti(30.0, 40.0), None),
        &same,
    );

    // 空
    assert!(!intervals.remove_interval(&TimeInterval::EMPTY));
    assert_eq!(intervals.len(), 2);

    // 首个之前
    assert!(!intervals.remove_interval(&create_ti(1.0, 5.0)));
    assert_eq!(intervals.len(), 2);

    // 最后一个之后
    assert!(!intervals.remove_interval(&create_ti(50.0, 60.0)));
    assert_eq!(intervals.len(), 2);

    // 位于空洞内
    assert!(!intervals.remove_interval(&create_ti(22.0, 28.0)));
    assert_eq!(intervals.len(), 2);

    // 从开头
    assert!(intervals.remove_interval(&create_ti(5.0, 15.0)));
    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals.get(0).unwrap().interval.start.total_days(), 15.0);
    assert_eq!(intervals.get(0).unwrap().interval.stop.total_days(), 20.0);

    // 从结尾
    assert!(intervals.remove_interval(&create_ti(35.0, 45.0)));
    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals.get(1).unwrap().interval.start.total_days(), 30.0);
    assert_eq!(intervals.get(1).unwrap().interval.stop.total_days(), 35.0);

    // 从单个区间的中部
    intervals.remove_all();
    intervals.add_interval(TimeIntervalData::new(create_ti(10.0, 20.0), None), &same);
    intervals.add_interval(TimeIntervalData::new(create_ti(30.0, 40.0), None), &same);
    assert!(intervals.remove_interval(&create_ti(12.0, 18.0)));
    assert_eq!(intervals.len(), 3);
    assert_eq!(intervals.get(0).unwrap().interval.stop.total_days(), 12.0);
    assert!(!intervals.get(0).unwrap().interval.is_stop_included);
    assert_eq!(intervals.get(1).unwrap().interval.start.total_days(), 18.0);
    assert!(!intervals.get(1).unwrap().interval.is_start_included);

    // 跨越整个区间并延伸进下一个区间的一部分
    intervals.remove_all();
    intervals.add_interval(TimeIntervalData::new(create_ti(10.0, 20.0), None), &same);
    intervals.add_interval(TimeIntervalData::new(create_ti(30.0, 40.0), None), &same);
    intervals.add_interval(TimeIntervalData::new(create_ti(45.0, 50.0), None), &same);
    assert!(intervals.remove_interval(&create_ti(25.0, 46.0)));
    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals.get(1).unwrap().interval.start.total_days(), 46.0);
    assert!(!intervals.get(1).unwrap().interval.is_start_included);

    // 区间结束于与现有区间相同的日期
    intervals.remove_all();
    intervals.add_interval(TimeIntervalData::new(create_ti(10.0, 20.0), None), &same);
    intervals.add_interval(TimeIntervalData::new(create_ti(30.0, 40.0), None), &same);
    intervals.add_interval(TimeIntervalData::new(create_ti(45.0, 50.0), None), &same);
    assert!(intervals.remove_interval(&create_ti(25.0, 40.0)));
    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals.get(0).unwrap().interval.stop.total_days(), 20.0);
    assert_eq!(intervals.get(1).unwrap().interval.start.total_days(), 45.0);

    // 区间结束于同一日期，单点存活
    intervals.remove_all();
    intervals.add_interval(TimeIntervalData::new(create_ti(10.0, 20.0), None), &same);
    intervals.add_interval(TimeIntervalData::new(create_ti(30.0, 40.0), None), &same);
    intervals.add_interval(TimeIntervalData::new(create_ti(45.0, 50.0), None), &same);
    assert!(intervals.remove_interval(&create_ti_excl(25.0, 40.0, true, false)));
    assert_eq!(intervals.len(), 3);
    assert_eq!(intervals.get(0).unwrap().interval.stop.total_days(), 20.0);
    assert_eq!(intervals.get(1).unwrap().interval.start.total_days(), 40.0);
    assert_eq!(intervals.get(1).unwrap().interval.stop.total_days(), 40.0);
    assert!(intervals.get(1).unwrap().interval.is_start_included);
    assert!(intervals.get(1).unwrap().interval.is_stop_included);
    assert_eq!(intervals.get(2).unwrap().interval.start.total_days(), 45.0);

    // 单点存活且可与下一个区间合并
    intervals.remove_all();
    intervals.add_interval(TimeIntervalData::new(create_ti(10.0, 20.0), None), &same);
    intervals.add_interval(TimeIntervalData::new(create_ti(30.0, 40.0), None), &same);
    intervals.add_interval(
        TimeIntervalData::new(create_ti_excl(40.0, 50.0, false, true), None),
        &same,
    );
    assert!(intervals.remove_interval(&create_ti_excl(25.0, 40.0, true, false)));
    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals.get(0).unwrap().interval.stop.total_days(), 20.0);
    assert_eq!(intervals.get(1).unwrap().interval.start.total_days(), 40.0);
    assert!(intervals.get(1).unwrap().interval.is_start_included);

    // 被移除区间的端点与现有区间的第一个点重叠
    intervals.remove_all();
    intervals.add_interval(TimeIntervalData::new(create_ti(10.0, 20.0), None), &same);
    assert!(intervals.remove_interval(&create_ti(0.0, 10.0)));
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals.get(0).unwrap().interval.start.total_days(), 10.0);
    assert_eq!(intervals.get(0).unwrap().interval.stop.total_days(), 20.0);
    assert!(!intervals.get(0).unwrap().interval.is_start_included);
    assert!(intervals.get(0).unwrap().interval.is_stop_included);

    // 被移除区间的起点不（NOT）与最后一个点重叠
    intervals.remove_all();
    intervals.add_interval(TimeIntervalData::new(create_ti(10.0, 20.0), None), &same);
    assert!(!intervals.remove_interval(&create_ti_excl(20.0, 30.0, false, true)));
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals.get(0).unwrap().interval.start.total_days(), 10.0);
    assert_eq!(intervals.get(0).unwrap().interval.stop.total_days(), 20.0);
    assert!(intervals.get(0).unwrap().interval.is_start_included);
    assert!(intervals.get(0).unwrap().interval.is_stop_included);

    // 从一个其余部分相同的闭区间中移除一个开区间
    intervals.remove_all();
    intervals.add_interval(TimeIntervalData::new(create_ti(0.0, 20.0), None), &same);
    assert!(intervals.remove_interval(&create_ti_excl(0.0, 20.0, false, false)));
    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals.get(0).unwrap().interval.start.total_days(), 0.0);
    assert_eq!(intervals.get(0).unwrap().interval.stop.total_days(), 0.0);
    assert!(intervals.get(0).unwrap().interval.is_start_included);
    assert!(intervals.get(0).unwrap().interval.is_stop_included);
    assert_eq!(intervals.get(1).unwrap().interval.start.total_days(), 20.0);
    assert_eq!(intervals.get(1).unwrap().interval.stop.total_days(), 20.0);
    assert!(intervals.get(1).unwrap().interval.is_start_included);
    assert!(intervals.get(1).unwrap().interval.is_stop_included);
}

#[test]
fn remove_interval_removes_the_first_interval_correctly() {
    let mut intervals: TimeIntervalCollection<&str> = TimeIntervalCollection::new();
    let same = |_: &&str, _: &&str| false; // 从不合并（数据不同）

    let from1to3 = TimeIntervalData::new(
        TimeInterval::new(jd(1.0), jd(3.0), true, true),
        Some("1-to-3"),
    );
    let from3to6 = TimeIntervalData::new(
        TimeInterval::new(jd(3.0), jd(6.0), true, true),
        Some("3-to-6"),
    );

    intervals.add_interval(from1to3, &same);
    intervals.add_interval(from3to6, &same);

    assert_eq!(intervals.len(), 2);
    assert!(intervals.get(0).unwrap().interval.is_start_included);
    assert!(!intervals.get(0).unwrap().interval.is_stop_included);
    assert_eq!(intervals.get(0).unwrap().interval.start.day_number, 1);
    assert_eq!(intervals.get(0).unwrap().interval.stop.day_number, 3);
    assert_eq!(intervals.get(0).unwrap().data, Some("1-to-3"));
    assert!(intervals.get(1).unwrap().interval.is_start_included);
    assert!(intervals.get(1).unwrap().interval.is_stop_included);
    assert_eq!(intervals.get(1).unwrap().interval.start.day_number, 3);
    assert_eq!(intervals.get(1).unwrap().interval.stop.day_number, 6);
    assert_eq!(intervals.get(1).unwrap().data, Some("3-to-6"));

    let to_remove = TimeInterval::new(jd(1.0), jd(3.0), true, true);
    assert!(intervals.remove_interval(&to_remove));
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals.start().unwrap().day_number, 3);
    assert_eq!(intervals.stop().unwrap().day_number, 6);
    assert_eq!(intervals.get(0).unwrap().interval.start.day_number, 3);
    assert_eq!(intervals.get(0).unwrap().interval.stop.day_number, 6);
    assert!(!intervals.get(0).unwrap().interval.is_start_included);
    assert!(intervals.get(0).unwrap().interval.is_stop_included);
    assert_eq!(intervals.get(0).unwrap().data, Some("3-to-6"));
}

#[test]
fn should_add_and_remove_intervals_correctly_integration_test() {
    const CONST_DAY_NUM: f64 = 3000000.0;

    fn interval_from_seconds(seconds: f64, data: i32) -> TimeIntervalData<i32> {
        TimeIntervalData::new(
            TimeInterval::new(
                JulianDate::new(CONST_DAY_NUM, seconds),
                JulianDate::new(CONST_DAY_NUM, seconds + 4.0),
                true,
                true,
            ),
            Some(data),
        )
    }

    fn remove_from_to(collection: &mut TimeIntervalCollection<i32>, from_sec: f64, to_sec: f64) {
        collection.remove_interval(&TimeInterval::new(
            JulianDate::new(CONST_DAY_NUM, from_sec),
            JulianDate::new(CONST_DAY_NUM, to_sec),
            true,
            true,
        ));
    }

    let same = |_: &i32, _: &i32| false; // 从不合并
    let mut collection: TimeIntervalCollection<i32> = TimeIntervalCollection::new();

    // 添加初始区间
    collection.add_interval(interval_from_seconds(0.0, 0), &same);
    collection.add_interval(interval_from_seconds(2.0, 2), &same);
    collection.add_interval(interval_from_seconds(4.0, 4), &same);
    collection.add_interval(interval_from_seconds(6.0, 6), &same);
    assert_eq!(collection.len(), 4);

    // 验证各个秒数处的数据
    assert_eq!(
        collection.find_data_for_interval_containing_date(&JulianDate::new(CONST_DAY_NUM, 0.0)),
        Some(&0)
    );
    assert_eq!(
        collection.find_data_for_interval_containing_date(&JulianDate::new(CONST_DAY_NUM, 2.0)),
        Some(&2)
    );
    assert_eq!(
        collection.find_data_for_interval_containing_date(&JulianDate::new(CONST_DAY_NUM, 6.0)),
        Some(&6)
    );
    assert_eq!(
        collection.find_data_for_interval_containing_date(&JulianDate::new(CONST_DAY_NUM, 10.0)),
        Some(&6)
    );
    assert_eq!(
        collection.find_data_for_interval_containing_date(&JulianDate::new(CONST_DAY_NUM, 11.0)),
        None
    );

    // 添加重叠区间
    collection.add_interval(interval_from_seconds(1.0, 1), &same);
    collection.add_interval(interval_from_seconds(3.0, 3), &same);
    assert_eq!(collection.len(), 4);
    assert_eq!(
        collection.find_data_for_interval_containing_date(&JulianDate::new(CONST_DAY_NUM, 0.0)),
        Some(&0)
    );
    assert_eq!(
        collection.find_data_for_interval_containing_date(&JulianDate::new(CONST_DAY_NUM, 1.0)),
        Some(&1)
    );
    assert_eq!(
        collection.find_data_for_interval_containing_date(&JulianDate::new(CONST_DAY_NUM, 8.0)),
        Some(&6)
    );

    // 移除区间
    remove_from_to(&mut collection, 3.0, 8.0);
    assert_eq!(collection.len(), 3);
    assert_eq!(
        collection.find_data_for_interval_containing_date(&JulianDate::new(CONST_DAY_NUM, 3.0)),
        None
    );
    assert_eq!(
        collection.find_data_for_interval_containing_date(&JulianDate::new(CONST_DAY_NUM, 9.0)),
        Some(&6)
    );

    // 移除全部
    remove_from_to(&mut collection, 0.0, 11.0);
    assert_eq!(collection.len(), 0);
}

#[test]
fn remove_interval_leaves_a_hole() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    let interval = tid_nodata(1.0, 4.0, true, true);
    let removed_interval = TimeInterval::new(jd(2.0), jd(3.0), true, false);
    intervals.add_interval(interval, &same_i32);
    assert!(intervals.remove_interval(&removed_interval));

    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals.get(0).unwrap().interval.start, jd(1.0));
    assert_eq!(intervals.get(0).unwrap().interval.stop, jd(2.0));
    assert!(intervals.get(0).unwrap().interval.is_start_included);
    assert!(!intervals.get(0).unwrap().interval.is_stop_included);

    assert_eq!(intervals.get(1).unwrap().interval.start, jd(3.0));
    assert_eq!(intervals.get(1).unwrap().interval.stop, jd(4.0));
    assert!(intervals.get(1).unwrap().interval.is_start_included);
    assert!(intervals.get(1).unwrap().interval.is_stop_included);
}

#[test]
fn remove_interval_with_an_interval_of_the_exact_same_size_works() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    let interval = TimeInterval::new(jd(1.0), jd(4.0), true, false);
    intervals.add_interval(TimeIntervalData::new(interval.clone(), None), &same_i32);
    assert_eq!(intervals.len(), 1);

    intervals.remove_interval(&interval);
    assert_eq!(intervals.len(), 0);
}

#[test]
fn remove_interval_with_an_empty_interval_has_no_affect() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    let interval = tid_nodata(1.0, 4.0, true, true);
    intervals.add_interval(interval, &same_i32);
    assert_eq!(intervals.len(), 1);

    assert!(!intervals.remove_interval(&TimeInterval::EMPTY));
    assert_eq!(intervals.len(), 1);
}

#[test]
fn remove_interval_takes_is_start_included_and_is_stop_included_into_account() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    let interval = tid_nodata(1.0, 4.0, true, true);
    let removed_interval = TimeInterval::new(jd(1.0), jd(4.0), false, false);
    intervals.add_interval(interval, &same_i32);
    assert!(intervals.remove_interval(&removed_interval));

    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals.get(0).unwrap().interval.start, jd(1.0));
    assert_eq!(intervals.get(0).unwrap().interval.stop, jd(1.0));
    assert!(intervals.get(0).unwrap().interval.is_start_included);
    assert!(intervals.get(0).unwrap().interval.is_stop_included);

    assert_eq!(intervals.get(1).unwrap().interval.start, jd(4.0));
    assert_eq!(intervals.get(1).unwrap().interval.stop, jd(4.0));
    assert!(intervals.get(1).unwrap().interval.is_start_included);
    assert!(intervals.get(1).unwrap().interval.is_stop_included);
}

#[test]
fn remove_interval_removes_overlapped_intervals() {
    let mut intervals: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    intervals.add_interval(tid_nodata(1.0, 2.0, true, false), &same_i32);
    intervals.add_interval(tid_nodata(2.0, 3.0, false, false), &same_i32);
    intervals.add_interval(tid_nodata(3.0, 4.0, false, false), &same_i32);
    intervals.add_interval(tid_nodata(4.0, 5.0, false, true), &same_i32);

    let removed_interval = TimeInterval::new(jd(2.0), jd(4.0), false, false);
    assert_eq!(intervals.len(), 4);
    assert!(intervals.remove_interval(&removed_interval));
    assert_eq!(intervals.len(), 2);
}

// === Intersect ===

#[test]
fn intersect_works_with_an_empty_collection() {
    let mut left: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    left.add_interval(tid_nodata(1.0, 4.0, true, true), &same_i32);
    let empty: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    assert_eq!(left.intersect(&empty, &same_i32).len(), 0);
}

#[test]
fn intersect_works_with_non_overlapping_intervals() {
    let mut left: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    left.add_interval(tid_nodata(1.0, 2.0, true, false), &same_i32);

    let mut right: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    right.add_interval(tid_nodata(2.0, 3.0, true, true), &same_i32);

    assert_eq!(left.intersect(&right, &same_i32).len(), 0);
}

#[test]
fn intersect_works_with_intersecting_intervals_and_no_merge_callback() {
    let mut left: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    left.add_interval(tid_nodata(1.0, 4.0, true, true), &same_i32);

    let mut right: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    right.add_interval(tid_nodata(2.0, 3.0, false, false), &same_i32);

    let intersected = left.intersect(&right, &same_i32);
    assert_eq!(intersected.len(), 1);
    assert_eq!(intersected.get(0).unwrap().interval.start, jd(2.0));
    assert_eq!(intersected.get(0).unwrap().interval.stop, jd(3.0));
    assert!(!intersected.get(0).unwrap().interval.is_start_included);
    assert!(!intersected.get(0).unwrap().interval.is_stop_included);
}

// === Equals ===

#[test]
fn equals_works_without_data() {
    let interval1 = tid_nodata(1.0, 2.0, true, true);
    let interval2 = tid_nodata(2.0, 3.0, false, true);
    let interval3 = tid_nodata(4.0, 5.0, true, true);

    let mut left: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    left.add_interval(interval1.clone(), &same_i32);
    left.add_interval(interval2.clone(), &same_i32);
    left.add_interval(interval3.clone(), &same_i32);

    let mut right: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    right.add_interval(interval1, &same_i32);
    right.add_interval(interval2, &same_i32);
    right.add_interval(interval3, &same_i32);

    assert!(left.equals(&right, &same_i32));
}

#[test]
fn equals_works_with_data() {
    // 在 CesiumJS 中，{} !== {}，因此没有回调时，不同的对象不相等。
    // 在 Rust 中，我们通过让 same_data 始终返回 false 来模拟这一行为。
    let always_false = |_: &i32, _: &i32| false;
    let always_true = |_: &i32, _: &i32| true;

    let mut left: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    left.add_interval(tid(1.0, 2.0, true, true, 100), &same_i32);
    left.add_interval(tid(2.0, 3.0, false, true, 200), &same_i32);
    left.add_interval(tid(4.0, 5.0, true, true, 300), &same_i32);

    let mut right: TimeIntervalCollection<i32> = TimeIntervalCollection::new();
    right.add_interval(tid(1.0, 2.0, true, true, 100), &same_i32);
    right.add_interval(tid(2.0, 3.0, false, true, 200), &same_i32);
    right.add_interval(tid(4.0, 5.0, true, true, 300), &same_i32);

    // 当 same_data = same_i32 时，它们相等（值相同）
    assert!(left.equals(&right, &same_i32));

    // 使用 always_true 时，它们相等
    assert!(left.equals(&right, &always_true));

    // 使用 always_false 时，它们不相等（模拟 {} !== {}）
    assert!(!left.equals(&right, &always_false));
}

// === fromIso8601 ===

#[test]
fn from_iso8601_returns_single_interval_if_no_duration() {
    let start = "2017-01-01T00:00:00Z";
    let stop = "2017-01-02T00:00:00Z";
    let julian_dates = iso8601_to_julian_date_array(&[start, stop]);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}", start, stop),
            is_start_included: Some(false),
            is_stop_included: Some(false),
            leading_interval: false,
            trailing_interval: false,
        },
        &same_usize,
    );

    check_intervals(&intervals, &julian_dates, false, false);
}

#[test]
fn from_iso8601_works_with_just_year() {
    let iso8601_dates = [
        "2017-01-01T00:00:00Z",
        "2018-01-01T00:00:00Z",
        "2019-01-01T00:00:00Z",
        "2020-01-01T00:00:00Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}/P1Y", iso8601_dates[0], iso8601_dates[3]),
            is_start_included: None,
            is_stop_included: None,
            leading_interval: false,
            trailing_interval: false,
        },
        &same_usize,
    );

    check_intervals(&intervals, &julian_dates, true, true);
}

#[test]
fn from_iso8601_works_with_just_month() {
    let iso8601_dates = [
        "2016-12-02T10:00:01.5Z",
        "2017-01-02T10:00:01.5Z",
        "2017-02-02T10:00:01.5Z",
        "2017-03-02T10:00:01.5Z",
        "2017-04-02T10:00:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}/P1M", iso8601_dates[0], iso8601_dates[4]),
            is_start_included: None,
            is_stop_included: None,
            leading_interval: false,
            trailing_interval: false,
        },
        &same_usize,
    );

    check_intervals(&intervals, &julian_dates, true, true);
}

#[test]
fn from_iso8601_works_with_just_day() {
    let iso8601_dates = [
        "2016-12-31T10:01:01.5Z",
        "2017-01-01T10:01:01.5Z",
        "2017-01-02T10:01:01.5Z",
        "2017-01-03T10:01:01.5Z",
        "2017-01-04T10:01:01.5Z",
        "2017-01-05T10:01:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}/P1D", iso8601_dates[0], iso8601_dates[5]),
            is_start_included: Some(false),
            is_stop_included: None,
            leading_interval: false,
            trailing_interval: false,
        },
        &same_usize,
    );

    check_intervals(&intervals, &julian_dates, false, true);
}

#[test]
fn from_iso8601_works_with_all_date_components() {
    let iso8601_dates = [
        "2017-01-01T10:01:01.5Z",
        "2018-03-04T10:01:01.5Z",
        "2019-05-07T10:01:01.5Z",
        "2020-07-10T10:01:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}/P1Y2M3D", iso8601_dates[0], iso8601_dates[3]),
            is_start_included: None,
            is_stop_included: Some(false),
            leading_interval: false,
            trailing_interval: false,
        },
        &same_usize,
    );

    check_intervals(&intervals, &julian_dates, true, false);
}

#[test]
fn from_iso8601_works_with_just_hour() {
    let iso8601_dates = [
        "2017-01-01T22:01:01.5Z",
        "2017-01-01T23:01:01.5Z",
        "2017-01-02T00:01:01.5Z",
        "2017-01-02T01:01:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}/PT1H", iso8601_dates[0], iso8601_dates[3]),
            is_start_included: Some(false),
            is_stop_included: None,
            leading_interval: false,
            trailing_interval: false,
        },
        &same_usize,
    );

    check_intervals(&intervals, &julian_dates, false, true);
}

#[test]
fn from_iso8601_works_with_just_minute() {
    let iso8601_dates = [
        "2016-12-31T23:58:01.5Z",
        "2016-12-31T23:59:01.5Z",
        "2017-01-01T00:00:01.5Z",
        "2017-01-01T00:01:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}/PT1M", iso8601_dates[0], iso8601_dates[3]),
            is_start_included: None,
            is_stop_included: Some(false),
            leading_interval: false,
            trailing_interval: false,
        },
        &same_usize,
    );

    check_intervals(&intervals, &julian_dates, true, false);
}

#[test]
fn from_iso8601_works_with_just_second() {
    let iso8601_dates = [
        "2016-12-31T23:59:58.5Z",
        "2016-12-31T23:59:59.5Z",
        "2017-01-01T00:00:00.5Z",
        "2017-01-01T00:00:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}/PT1S", iso8601_dates[0], iso8601_dates[3]),
            is_start_included: Some(false),
            is_stop_included: Some(false),
            leading_interval: false,
            trailing_interval: false,
        },
        &same_usize,
    );

    check_intervals(&intervals, &julian_dates, false, false);
}

#[test]
fn from_iso8601_works_with_just_millisecond() {
    let iso8601_dates = [
        "2016-12-31T23:59:58.5Z",
        "2016-12-31T23:59:59Z",
        "2016-12-31T23:59:59.5Z",
        "2017-01-01T00:00:00Z",
        "2017-01-01T00:00:00.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}/PT0.5S", iso8601_dates[0], iso8601_dates[4]),
            is_start_included: None,
            is_stop_included: None,
            leading_interval: false,
            trailing_interval: false,
        },
        &same_usize,
    );

    check_intervals(&intervals, &julian_dates, true, true);
}

#[test]
fn from_iso8601_works_with_all_time_components() {
    let iso8601_dates = [
        "2017-01-01T10:01:01.5Z",
        "2017-01-01T11:03:05Z",
        "2017-01-01T12:05:08.5Z",
        "2017-01-01T13:07:12Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}/PT1H2M3.5S", iso8601_dates[0], iso8601_dates[3]),
            is_start_included: None,
            is_stop_included: None,
            leading_interval: false,
            trailing_interval: false,
        },
        &same_usize,
    );

    check_intervals(&intervals, &julian_dates, true, true);
}

#[test]
fn from_iso8601_works_with_all_date_and_time_components() {
    let iso8601_dates = [
        "2017-01-01T10:01:01.5Z",
        "2018-03-04T11:03:05Z",
        "2019-05-07T12:05:08.5Z",
        "2020-07-10T13:07:12Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!(
                "{}/{}/P1Y2M3DT1H2M3.5S",
                iso8601_dates[0], iso8601_dates[3]
            ),
            is_start_included: None,
            is_stop_included: None,
            leading_interval: false,
            trailing_interval: false,
        },
        &same_usize,
    );

    check_intervals(&intervals, &julian_dates, true, true);
}

#[test]
fn from_iso8601_works_with_date_string_for_duration() {
    let iso8601_dates = [
        "2017-01-01T10:01:01.5Z",
        "2018-03-04T11:03:05Z",
        "2019-05-07T12:05:08.5Z",
        "2020-07-10T13:07:12Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!(
                "{}/{}/0001-02-03T01:02:03.5",
                iso8601_dates[0], iso8601_dates[3]
            ),
            is_start_included: None,
            is_stop_included: None,
            leading_interval: false,
            trailing_interval: false,
        },
        &same_usize,
    );

    check_intervals(&intervals, &julian_dates, true, true);
}

// === fromIso8601 with leading/trailing ===

#[test]
fn from_iso8601_handles_leading_interval_option() {
    let iso8601_dates = [
        "2016-12-31T23:58:01.5Z",
        "2016-12-31T23:59:01.5Z",
        "2017-01-01T00:00:01.5Z",
        "2017-01-01T00:01:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}/PT1M", iso8601_dates[0], iso8601_dates[3]),
            is_start_included: Some(true),
            is_stop_included: Some(false),
            leading_interval: true,
            trailing_interval: false,
        },
        &same_usize,
    );

    // 总计：1 个前导 + 3 个主要 = 4
    assert_eq!(intervals.len(), 4);

    // 检查前导区间
    let leading = intervals.get(0).unwrap();
    let min_value = JulianDate::from_iso8601("0001-01-01T00:00:00Z").unwrap();
    assert_eq!(leading.interval.start, min_value);
    assert_eq!(leading.interval.stop, julian_dates[0]);
    assert!(leading.interval.is_start_included);
    assert!(!leading.interval.is_stop_included); // !isStartIncluded = false

    // 检查主要区间（索引 1..4）
    for i in 0..3 {
        let interval = intervals.get(i + 1).unwrap();
        assert_eq!(interval.interval.start, julian_dates[i]);
        assert_eq!(interval.interval.stop, julian_dates[i + 1]);
        // 对于此选项集，每个主要区间都包含其起点并
        // 排除其终点，与 `i` 无关。
        assert!(interval.interval.is_start_included);
        assert!(!interval.interval.is_stop_included);
    }
}

#[test]
fn from_iso8601_handles_trailing_interval_option() {
    let iso8601_dates = [
        "2016-12-31T23:58:01.5Z",
        "2016-12-31T23:59:01.5Z",
        "2017-01-01T00:00:01.5Z",
        "2017-01-01T00:01:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}/PT1M", iso8601_dates[0], iso8601_dates[3]),
            is_start_included: Some(false),
            is_stop_included: Some(true),
            leading_interval: false,
            trailing_interval: true,
        },
        &same_usize,
    );

    // 总计：3 个主要 + 1 个尾随 = 4
    assert_eq!(intervals.len(), 4);

    // 检查尾随区间
    let trailing = intervals.get(3).unwrap();
    let max_value = JulianDate::from_iso8601("9999-12-31T24:00:00Z").unwrap();
    assert_eq!(trailing.interval.start, julian_dates[3]);
    assert_eq!(trailing.interval.stop, max_value);
    assert!(!trailing.interval.is_start_included); // !isStopIncluded = false
    assert!(trailing.interval.is_stop_included);

    // 检查主要区间
    for i in 0..3 {
        let interval = intervals.get(i).unwrap();
        assert_eq!(interval.interval.start, julian_dates[i]);
        assert_eq!(interval.interval.stop, julian_dates[i + 1]);
        assert_eq!(
            interval.interval.is_start_included,
            if i == 0 { false } else { true }
        );
        assert_eq!(
            interval.interval.is_stop_included,
            if i == 2 { true } else { false }
        );
    }
}

#[test]
fn from_iso8601_handles_leading_and_trailing_interval_options() {
    let iso8601_dates = [
        "2016-12-31T23:58:01.5Z",
        "2016-12-31T23:59:01.5Z",
        "2017-01-01T00:00:01.5Z",
        "2017-01-01T00:01:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_iso8601(
        &FromIso8601Options {
            iso8601: format!("{}/{}/PT1M", iso8601_dates[0], iso8601_dates[3]),
            is_start_included: Some(false),
            is_stop_included: Some(false),
            leading_interval: true,
            trailing_interval: true,
        },
        &same_usize,
    );

    // 总计：1 个前导 + 3 个主要 + 1 个尾随 = 5
    assert_eq!(intervals.len(), 5);

    // 检查前导区间
    let leading = intervals.get(0).unwrap();
    let min_value = JulianDate::from_iso8601("0001-01-01T00:00:00Z").unwrap();
    assert_eq!(leading.interval.start, min_value);
    assert_eq!(leading.interval.stop, julian_dates[0]);
    assert!(leading.interval.is_start_included);
    assert!(leading.interval.is_stop_included); // !isStartIncluded = !false = true

    // 检查尾随区间
    let trailing = intervals.get(4).unwrap();
    let max_value = JulianDate::from_iso8601("9999-12-31T24:00:00Z").unwrap();
    assert_eq!(trailing.interval.start, julian_dates[3]);
    assert_eq!(trailing.interval.stop, max_value);
    assert!(trailing.interval.is_start_included); // !isStopIncluded = !false = true
    assert!(trailing.interval.is_stop_included);

    // 检查主要区间（索引 1..4）
    for i in 0..3 {
        let interval = intervals.get(i + 1).unwrap();
        assert_eq!(interval.interval.start, julian_dates[i]);
        assert_eq!(interval.interval.stop, julian_dates[i + 1]);
        assert_eq!(
            interval.interval.is_start_included,
            if i == 0 { false } else { true }
        );
        assert!(!interval.interval.is_stop_included);
    }
}

// === fromIso8601DateArray ===

#[test]
fn from_iso8601_date_array_handles_leading_interval_option() {
    let iso8601_dates = [
        "2016-12-31T23:58:01.5Z",
        "2016-12-31T23:59:01.5Z",
        "2017-01-01T00:00:01.5Z",
        "2017-01-01T00:01:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_julian_date_array(
        &julian_dates,
        true,  // is_start_included
        false, // is_stop_included
        true,  // leading_interval
        false, // trailing_interval
        &same_usize,
    );

    assert_eq!(intervals.len(), 4);

    // 检查前导区间
    let leading = intervals.get(0).unwrap();
    let min_value = JulianDate::from_iso8601("0001-01-01T00:00:00Z").unwrap();
    assert_eq!(leading.interval.start, min_value);
    assert_eq!(leading.interval.stop, julian_dates[0]);
    assert!(leading.interval.is_start_included);
    assert!(!leading.interval.is_stop_included); // !isStartIncluded = false
}

#[test]
fn from_iso8601_date_array_handles_trailing_interval_option() {
    let iso8601_dates = [
        "2016-12-31T23:58:01.5Z",
        "2016-12-31T23:59:01.5Z",
        "2017-01-01T00:00:01.5Z",
        "2017-01-01T00:01:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_julian_date_array(
        &julian_dates,
        false, // is_start_included
        true,  // is_stop_included
        false, // leading_interval
        true,  // trailing_interval
        &same_usize,
    );

    assert_eq!(intervals.len(), 4);

    // 检查尾随区间
    let trailing = intervals.get(3).unwrap();
    let max_value = JulianDate::from_iso8601("9999-12-31T24:00:00Z").unwrap();
    assert_eq!(trailing.interval.start, julian_dates[3]);
    assert_eq!(trailing.interval.stop, max_value);
    assert!(!trailing.interval.is_start_included); // !isStopIncluded = false
    assert!(trailing.interval.is_stop_included);
}

#[test]
fn from_iso8601_date_array_handles_leading_and_trailing_interval_options() {
    let iso8601_dates = [
        "2016-12-31T23:58:01.5Z",
        "2016-12-31T23:59:01.5Z",
        "2017-01-01T00:00:01.5Z",
        "2017-01-01T00:01:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);

    let intervals = TimeIntervalCollection::<usize>::from_julian_date_array(
        &julian_dates,
        false, // is_start_included
        false, // is_stop_included
        true,  // leading_interval
        true,  // trailing_interval
        &same_usize,
    );

    assert_eq!(intervals.len(), 5);

    // 检查前导区间
    let leading = intervals.get(0).unwrap();
    let min_value = JulianDate::from_iso8601("0001-01-01T00:00:00Z").unwrap();
    assert_eq!(leading.interval.start, min_value);
    assert_eq!(leading.interval.stop, julian_dates[0]);
    assert!(leading.interval.is_start_included);
    assert!(leading.interval.is_stop_included); // !isStartIncluded = !false = true

    // 检查尾随区间
    let trailing = intervals.get(4).unwrap();
    let max_value = JulianDate::from_iso8601("9999-12-31T24:00:00Z").unwrap();
    assert_eq!(trailing.interval.start, julian_dates[3]);
    assert_eq!(trailing.interval.stop, max_value);
    assert!(trailing.interval.is_start_included); // !isStopIncluded = !false = true
    assert!(trailing.interval.is_stop_included);
}

// === fromIso8601DurationArray ===

#[test]
fn from_iso8601_duration_array_handles_relative_to_previous_set_to_false() {
    let iso8601_dates = [
        "2016-12-31T23:58:01.5Z",
        "2016-12-31T23:59:01.5Z",
        "2017-01-01T00:00:01.5Z",
        "2017-01-01T00:01:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);
    let iso8601_durations = ["PT0M", "PT1M", "PT2M", "PT3M"];

    let intervals = TimeIntervalCollection::<usize>::from_iso8601_duration_array(
        &julian_dates[0],
        &iso8601_durations,
        false, // relative_to_previous
        false, // is_start_included
        false, // is_stop_included
        true,  // leading_interval
        true,  // trailing_interval
        &same_usize,
    );

    // 总计：1 个前导 + 3 个主要 + 1 个尾随 = 5
    assert_eq!(intervals.len(), 5);

    // 检查前导区间
    let leading = intervals.get(0).unwrap();
    let min_value = JulianDate::from_iso8601("0001-01-01T00:00:00Z").unwrap();
    assert_eq!(leading.interval.start, min_value);
    assert_eq!(leading.interval.stop, julian_dates[0]);
    assert!(leading.interval.is_start_included);
    assert!(leading.interval.is_stop_included); // !isStartIncluded = !false = true

    // 检查尾随区间
    let trailing = intervals.get(4).unwrap();
    let max_value = JulianDate::from_iso8601("9999-12-31T24:00:00Z").unwrap();
    assert_eq!(trailing.interval.start, julian_dates[3]);
    assert_eq!(trailing.interval.stop, max_value);
    assert!(trailing.interval.is_start_included); // !isStopIncluded = !false = true
    assert!(trailing.interval.is_stop_included);

    // 检查主要区间
    for i in 0..3 {
        let interval = intervals.get(i + 1).unwrap();
        assert_eq!(interval.interval.start, julian_dates[i]);
        assert_eq!(interval.interval.stop, julian_dates[i + 1]);
    }
}

#[test]
fn from_iso8601_duration_array_handles_relative_to_previous_set_to_true() {
    let iso8601_dates = [
        "2016-12-31T23:58:01.5Z",
        "2016-12-31T23:59:01.5Z",
        "2017-01-01T00:00:01.5Z",
        "2017-01-01T00:01:01.5Z",
    ];
    let julian_dates = iso8601_to_julian_date_array(&iso8601_dates);
    let iso8601_durations = ["PT0M", "PT1M", "PT1M", "PT1M"];

    let intervals = TimeIntervalCollection::<usize>::from_iso8601_duration_array(
        &julian_dates[0],
        &iso8601_durations,
        true,  // relative_to_previous
        false, // is_start_included
        false, // is_stop_included
        true,  // leading_interval
        true,  // trailing_interval
        &same_usize,
    );

    // 总计：1 个前导 + 3 个主要 + 1 个尾随 = 5
    assert_eq!(intervals.len(), 5);

    // 检查前导区间
    let leading = intervals.get(0).unwrap();
    let min_value = JulianDate::from_iso8601("0001-01-01T00:00:00Z").unwrap();
    assert_eq!(leading.interval.start, min_value);
    assert_eq!(leading.interval.stop, julian_dates[0]);

    // 检查尾随区间
    let trailing = intervals.get(4).unwrap();
    let max_value = JulianDate::from_iso8601("9999-12-31T24:00:00Z").unwrap();
    assert_eq!(trailing.interval.start, julian_dates[3]);
    assert_eq!(trailing.interval.stop, max_value);

    // 检查主要区间
    for i in 0..3 {
        let interval = intervals.get(i + 1).unwrap();
        assert_eq!(interval.interval.start, julian_dates[i]);
        assert_eq!(interval.interval.stop, julian_dates[i + 1]);
    }
}
