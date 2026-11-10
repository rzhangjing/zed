//! Core/TimeIntervalSpec → Rust integration tests
//! 35 original it() blocks. JS-specific tests (undefined params, data field, result param) skipped.
//! Ported: constructor, fromIso8601, toIso8601, isEmpty, contains, equals, clone, intersect

use cesium_time::{JulianDate, TimeInterval, TimeStandard};

/// Helper: create JulianDate from day number (TAI) - equivalent to new JulianDate(n)
fn jd(day: f64) -> JulianDate {
    JulianDate::with_time_standard(day, 0.0, TimeStandard::UTC)
}

/// Helper: create TimeInterval
fn ti(start: f64, stop: f64, start_incl: bool, stop_incl: bool) -> TimeInterval {
    TimeInterval::new(jd(start), jd(stop), start_incl, stop_incl)
}

// === Constructor ===

#[test]
fn constructor_sets_expected_defaults() {
    let interval = TimeInterval::default();
    let default_date = JulianDate::default();
    assert_eq!(interval.start, default_date);
    assert_eq!(interval.stop, default_date);
    assert!(interval.is_start_included);
    assert!(interval.is_stop_included);
}

#[test]
fn constructor_assigns_all_options() {
    let start = jd(1.0);
    let stop = jd(2.0);
    let interval = TimeInterval::new(start, stop, false, false);
    assert_eq!(interval.start, start);
    assert_eq!(interval.stop, stop);
    assert!(!interval.is_start_included);
    assert!(!interval.is_stop_included);
}

// === fromIso8601 ===

#[test]
fn from_iso8601_assigns_expected_defaults() {
    let start = JulianDate::from_iso8601("2013").unwrap();
    let stop = JulianDate::from_iso8601("2014").unwrap();
    let interval = TimeInterval::from_iso8601("2013/2014", true, true).unwrap();
    assert_eq!(interval.start, start);
    assert_eq!(interval.stop, stop);
    assert!(interval.is_start_included);
    assert!(interval.is_stop_included);
}

#[test]
fn from_iso8601_assigns_all_options() {
    let start = JulianDate::from_iso8601("2013").unwrap();
    let stop = JulianDate::from_iso8601("2014").unwrap();
    let interval = TimeInterval::from_iso8601("2013/2014", false, false).unwrap();
    assert_eq!(interval.start, start);
    assert_eq!(interval.stop, stop);
    assert!(!interval.is_start_included);
    assert!(!interval.is_stop_included);
}

#[test]
fn from_iso8601_invalid_date_returns_none() {
    // 单个日期（无 '/'）对区间而言无效
    assert!(TimeInterval::from_iso8601("2020-08-29T00:00:00+00:00", true, true).is_none());
}

// === toIso8601 ===

#[test]
fn to_iso8601_works() {
    let iso_date1 = "0950-01-02T03:04:05Z";
    let iso_date2 = "0950-01-03T03:04:05Z";
    let interval = TimeInterval::new(
        JulianDate::from_iso8601(iso_date1).unwrap(),
        JulianDate::from_iso8601(iso_date2).unwrap(),
        true,
        true,
    );
    assert_eq!(interval.to_iso8601(), "0950-01-02T03:04:05Z/0950-01-03T03:04:05Z");
}

#[test]
fn can_round_trip_with_iso8601() {
    let start = jd(2455000.0);
    let stop = jd(2455001.0);
    let interval = TimeInterval::new(start, stop, true, true);
    let iso = interval.to_iso8601();
    let roundtrip = TimeInterval::from_iso8601(&iso, true, true).unwrap();
    assert_eq!(roundtrip, interval);
}

#[test]
fn to_iso8601_works_with_specified_precision() {
    let iso_date1 = "0950-01-02T03:04:05.012345Z";
    let iso_date2 = "0950-01-03T03:04:05.012345Z";
    let interval = TimeInterval::new(
        JulianDate::from_iso8601(iso_date1).unwrap(),
        JulianDate::from_iso8601(iso_date2).unwrap(),
        true,
        true,
    );
    assert_eq!(
        interval.to_iso8601_with_precision(Some(0)),
        "0950-01-02T03:04:05Z/0950-01-03T03:04:05Z"
    );
    assert_eq!(
        interval.to_iso8601_with_precision(Some(7)),
        "0950-01-02T03:04:05.0123450Z/0950-01-03T03:04:05.0123450Z"
    );
}

// === isEmpty ===

#[test]
fn is_empty_false_for_non_empty_interval() {
    let interval = ti(1.0, 2.0, true, true);
    assert!(!interval.is_empty());
}

#[test]
fn is_empty_false_for_instantaneous_closed_both_ends() {
    let interval = ti(1.0, 1.0, true, true);
    assert!(!interval.is_empty());
}

#[test]
fn is_empty_true_for_instantaneous_open_both_ends() {
    let interval = ti(1.0, 1.0, false, false);
    assert!(interval.is_empty());
}

#[test]
fn is_empty_true_for_instantaneous_open_start() {
    let interval = ti(1.0, 1.0, false, true);
    assert!(interval.is_empty());
}

#[test]
fn is_empty_true_for_instantaneous_open_stop() {
    let interval = ti(1.0, 1.0, true, false);
    assert!(interval.is_empty());
}

#[test]
fn is_empty_true_for_stop_before_start() {
    let interval = ti(5.0, 4.0, true, true);
    assert!(interval.is_empty());
}

#[test]
fn is_empty_true_for_instantaneous_only_closed_stop() {
    let interval = ti(5.0, 5.0, false, true);
    assert!(interval.is_empty());
}

#[test]
fn is_empty_true_for_instantaneous_only_closed_start() {
    let interval = ti(5.0, 5.0, true, false);
    assert!(interval.is_empty());
}

// === contains ===

#[test]
fn contains_works_for_non_empty_interval() {
    let interval = TimeInterval::new(
        JulianDate::with_time_standard(2451545.0, 0.0, TimeStandard::UTC),
        JulianDate::with_time_standard(2451546.0, 0.0, TimeStandard::UTC),
        true,
        true,
    );
    let inside = JulianDate::with_time_standard(2451545.5, 0.0, TimeStandard::UTC);
    let outside = JulianDate::with_time_standard(2451546.5, 0.0, TimeStandard::UTC);
    assert!(interval.contains(&inside));
    assert!(!interval.contains(&outside));
}

#[test]
fn contains_works_for_empty_interval() {
    let empty = TimeInterval::EMPTY;
    let date = JulianDate::default();
    assert!(!empty.contains(&date));
}

#[test]
fn contains_returns_true_at_start_stop_of_closed_interval() {
    let interval = TimeInterval::new(
        JulianDate::with_time_standard(2451545.0, 0.0, TimeStandard::UTC),
        JulianDate::with_time_standard(2451546.0, 0.0, TimeStandard::UTC),
        true,
        true,
    );
    let start = JulianDate::with_time_standard(2451545.0, 0.0, TimeStandard::UTC);
    let stop = JulianDate::with_time_standard(2451546.0, 0.0, TimeStandard::UTC);
    assert!(interval.contains(&start));
    assert!(interval.contains(&stop));
}

#[test]
fn contains_returns_false_at_start_stop_of_open_interval() {
    let interval = TimeInterval::new(
        JulianDate::with_time_standard(2451545.0, 0.0, TimeStandard::UTC),
        JulianDate::with_time_standard(2451546.0, 0.0, TimeStandard::UTC),
        false,
        false,
    );
    let start = JulianDate::with_time_standard(2451545.0, 0.0, TimeStandard::UTC);
    let stop = JulianDate::with_time_standard(2451546.0, 0.0, TimeStandard::UTC);
    assert!(!interval.contains(&start));
    assert!(!interval.contains(&stop));
}

// === equals / equalsEpsilon ===

#[test]
fn equals_and_equals_epsilon_work() {
    let left = TimeInterval::default();
    let right = TimeInterval::default();
    assert_eq!(left, right);
    assert!(left.equals_epsilon(&right, 0.0));

    // 不同的 start
    let right2 = TimeInterval::new(
        JulianDate::with_time_standard(-1.0, 0.0, TimeStandard::UTC),
        JulianDate::default(),
        true,
        true,
    );
    assert_ne!(left, right2);
    assert!(!left.equals_epsilon(&right2, 0.0));

    // 不同的 stop
    let right3 = TimeInterval::new(
        JulianDate::default(),
        JulianDate::with_time_standard(1.0, 0.0, TimeStandard::UTC),
        true,
        true,
    );
    assert_ne!(left, right3);
    assert!(!left.equals_epsilon(&right3, 0.0));

    // 不同的 is_start_included
    let right4 = TimeInterval::new(JulianDate::default(), JulianDate::default(), false, true);
    assert_ne!(left, right4);
    assert!(!left.equals_epsilon(&right4, 0.0));

    // 不同的 is_stop_included
    let right5 = TimeInterval::new(JulianDate::default(), JulianDate::default(), true, false);
    assert_ne!(left, right5);
    assert!(!left.equals_epsilon(&right5, 0.0));
}

#[test]
fn equals_epsilon_works_within_threshold() {
    let left = TimeInterval::new(
        JulianDate::with_time_standard(0.0, 0.0, TimeStandard::UTC),
        JulianDate::with_time_standard(1.0, 0.0, TimeStandard::UTC),
        true,
        true,
    );
    let right = TimeInterval::new(
        JulianDate::with_time_standard(0.0, 0.0, TimeStandard::UTC),
        JulianDate::with_time_standard(1.0, 1.0, TimeStandard::UTC),
        true,
        true,
    );
    assert!(left.equals_epsilon(&right, 1.0));
    assert!(!left.equals_epsilon(&right, 0.99));
}

// === clone ===

#[test]
fn clone_returns_identical_interval() {
    let interval = TimeInterval::new(
        JulianDate::with_time_standard(1.0, 0.0, TimeStandard::UTC),
        JulianDate::with_time_standard(2.5, 0.0, TimeStandard::UTC),
        true,
        false,
    );
    let cloned = interval.clone();
    assert_eq!(cloned, interval);
}

// === toString ===

#[test]
fn formats_as_iso8601_with_to_string() {
    let start = JulianDate::from_iso8601("2011-07-04T12:00:00Z").unwrap();
    let stop = JulianDate::from_iso8601("2011-07-05T12:00:00Z").unwrap();
    let interval = TimeInterval::new(start, stop, true, true);
    // to_iso8601 should produce "start/stop" format
    let iso = interval.to_iso8601();
    assert_eq!(iso, "2011-07-04T12:00:00Z/2011-07-05T12:00:00Z");
    // 验证往返转换
    let parsed = TimeInterval::from_iso8601(&iso, true, true).unwrap();
    assert_eq!(parsed.start, interval.start);
    assert_eq!(parsed.stop, interval.stop);
}

// === intersect (exhaustive cases from CesiumJS) ===

#[test]
fn intersect_exhaustive_cases() {
    // Triplets: (first, second, expected_result)
    let test_parameters: Vec<TimeInterval> = vec![
        ti(1.0, 2.5, true, true),     // 0
        ti(1.5, 2.0, true, true),     // 1
        ti(1.5, 2.0, true, true),     // 2 (expected)
        ti(1.0, 2.5, true, true),     // 3
        ti(3.0, 4.0, true, true),     // 4
        ti(0.0, 0.0, false, false),   // 5 (expected: EMPTY)
        ti(1.0, 2.5, true, true),     // 6
        ti(2.0, 3.0, true, true),     // 7
        ti(2.0, 2.5, true, true),     // 8 (expected)
        ti(1.0, 2.0, true, true),     // 9
        ti(1.0, 2.0, false, false),   // 10
        ti(1.0, 2.0, false, false),   // 11 (expected)
        ti(1.0, 2.0, false, false),   // 12
        ti(1.0, 2.0, false, false),   // 13
        ti(1.0, 2.0, false, false),   // 14 (expected)
        ti(1.0, 2.0, true, false),    // 15
        ti(1.0, 2.0, false, true),    // 16
        ti(1.0, 2.0, false, false),   // 17 (expected)
        ti(1.0, 2.0, true, false),    // 18
        ti(1.0, 2.0, true, false),    // 19
        ti(1.0, 2.0, true, false),    // 20 (expected)
        ti(1.0, 3.0, false, false),   // 21
        ti(2.0, 4.0, false, false),   // 22
        ti(2.0, 3.0, false, false),   // 23 (expected)
        ti(1.0, 3.0, false, false),   // 24
        ti(2.0, 4.0, true, true),     // 25
        ti(2.0, 3.0, true, false),    // 26 (expected)
        ti(1.0, 1.0, false, false),   // 27
        ti(1.0, 2.0, true, true),     // 28
        ti(0.0, 0.0, false, false),   // 29 (expected: EMPTY)
        ti(1.0, 3.0, true, true),     // 30
        ti(2.0, 3.0, true, true),     // 31
        ti(2.0, 3.0, true, true),     // 32 (expected)
        ti(3.0, 2.0, true, true),     // 33 (empty - stop < start)
        ti(3.0, 3.0, true, true),     // 34
        ti(0.0, 0.0, false, false),   // 35 (expected: EMPTY)
    ];

    let empty = TimeInterval::EMPTY;
    let mut i = 0;
    while i + 2 < test_parameters.len() {
        let first = &test_parameters[i];
        let second = &test_parameters[i + 1];
        let expected = &test_parameters[i + 2];

        let intersect1 = first.intersect(second);
        let intersect2 = second.intersect(first);

        // 两个方向应给出相同结果
        assert_eq!(intersect1, intersect2, "Failed symmetry at index {}", i);
        // 结果应与期望匹配
        let expected_val = if expected.is_empty() && expected.start.day_number == 0
            && expected.stop.day_number == 0 && !expected.is_start_included && !expected.is_stop_included
        {
            empty.clone()
        } else {
            expected.clone()
        };
        assert_eq!(intersect1, expected_val, "Failed at index {}", i);

        i += 3;
    }
}
