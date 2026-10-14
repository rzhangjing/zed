//! Core/JulianDateSpec、ClockSpec、TimeIntervalSpec → Rust 集成测试
//! 针对 cesium_time crate 的测试

use cesium_time::{JulianDate, GregorianDate, TimeInterval, Clock, ClockRange, ClockStep};
use cesium_specs::{assert_approx, epsilon};

// === JulianDate ===

#[test]
fn test_julian_date_new() {
    // JulianDate::new 将输入视为 UTC 并在内部转换为 TAI
    let jd = JulianDate::new(2451545.0, 0.0);
    // 通过 GregorianDate 往返验证
    let g = jd.to_gregorian_date();
    assert_eq!(g.year, 2000);
    assert_eq!(g.month, 1);
    assert_eq!(g.day, 1);
    assert_eq!(g.hour, 12);
    assert_eq!(g.minute, 0);
    assert_eq!(g.second, 0);
}

#[test]
fn test_julian_date_from_date_components() {
    let jd = JulianDate::from_date_components(2000, 1, 1, 12, 0, 0, 0.0);
    // total_days 基于 TAI（在 J2000 处包含 32 秒闰秒偏移）
    let expected = 2451545.0 + 32.0 / 86400.0;
    assert_approx!(jd.total_days(), expected, epsilon::EPSILON6);
}

#[test]
fn test_julian_date_from_date_components_epoch() {
    let jd = JulianDate::from_date_components(1970, 1, 1, 0, 0, 0, 0.0);
    // total_days 基于 TAI（1972 年前包含 10 秒闰秒偏移）
    let expected = 2440587.5 + 10.0 / 86400.0;
    assert_approx!(jd.total_days(), expected, epsilon::EPSILON6);
}

#[test]
fn test_julian_date_from_gregorian_date() {
    let greg = GregorianDate::new(2000, 1, 1, 12, 0, 0, 0.0, false);
    let jd = JulianDate::from_gregorian_date(&greg);
    // 通过往返验证
    let result = jd.to_gregorian_date();
    assert_eq!(result.year, 2000);
    assert_eq!(result.month, 1);
    assert_eq!(result.day, 1);
    assert_eq!(result.hour, 12);
}

#[test]
fn test_julian_date_to_gregorian_date() {
    let jd = JulianDate::from_date_components(2000, 1, 1, 12, 0, 0, 0.0);
    let greg = jd.to_gregorian_date();
    assert_eq!(greg.year, 2000);
    assert_eq!(greg.month, 1);
    assert_eq!(greg.day, 1);
    assert_eq!(greg.hour, 12);
    assert_eq!(greg.minute, 0);
    assert_eq!(greg.second, 0);
}

#[test]
fn test_julian_date_gregorian_roundtrip() {
    let original = GregorianDate::new(2023, 6, 15, 14, 30, 45, 500.0, false);
    let jd = JulianDate::from_gregorian_date(&original);
    let result = jd.to_gregorian_date();
    assert_eq!(result.year, original.year);
    assert_eq!(result.month, original.month);
    assert_eq!(result.day, original.day);
    assert_eq!(result.hour, original.hour);
    assert_eq!(result.minute, original.minute);
    assert_eq!(result.second, original.second);
    assert_approx!(result.millisecond, original.millisecond, 1.0);
}

#[test]
fn test_julian_date_from_unix_seconds() {
    let jd = JulianDate::from_unix_seconds(0.0);
    // 通过往返验证：unix_seconds 应为 0
    assert_approx!(jd.to_unix_seconds(), 0.0, epsilon::EPSILON3);
}

#[test]
fn test_julian_date_to_unix_seconds() {
    let jd = JulianDate::from_unix_seconds(86400.0);
    assert_approx!(jd.to_unix_seconds(), 86400.0, epsilon::EPSILON3);
}

#[test]
fn test_julian_date_seconds_difference() {
    let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
    let end = JulianDate::from_date_components(2000, 1, 1, 0, 1, 0, 0.0);
    let diff = end.seconds_difference(&start);
    assert_approx!(diff, 60.0, epsilon::EPSILON6);
}

#[test]
fn test_julian_date_days_difference() {
    let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
    let end = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
    let diff = end.days_difference(&start);
    assert_approx!(diff, 1.0, epsilon::EPSILON10);
}

#[test]
fn test_julian_date_add_seconds() {
    let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
    let result = start.add_seconds(3600.0);
    let greg = result.to_gregorian_date();
    assert_eq!(greg.hour, 1);
}

#[test]
fn test_julian_date_add_days() {
    let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
    let result = start.add_days(1.0);
    let greg = result.to_gregorian_date();
    assert_eq!(greg.day, 2);
}

#[test]
fn test_julian_date_comparison() {
    let earlier = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
    let later = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
    assert!(earlier < later);
    assert!(later > earlier);
    assert!(earlier != later);
}

#[test]
fn test_julian_date_equals() {
    let a = JulianDate::from_date_components(2000, 1, 1, 12, 0, 0, 0.0);
    let b = JulianDate::from_date_components(2000, 1, 1, 12, 0, 0, 0.0);
    assert_eq!(a, b);
}

// === GregorianDate ===

#[test]
fn test_gregorian_date_new() {
    let greg = GregorianDate::new(2023, 6, 15, 14, 30, 45, 500.0, false);
    assert_eq!(greg.year, 2023);
    assert_eq!(greg.month, 6);
    assert_eq!(greg.day, 15);
    assert!(!greg.is_leap_second);
}

// === TimeInterval ===

#[test]
fn test_time_interval_new() {
    let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
    let stop = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
    let interval = TimeInterval::new(start, stop, true, true);
    assert_eq!(interval.start, start);
    assert_eq!(interval.stop, stop);
    assert!(interval.is_start_included);
    assert!(interval.is_stop_included);
}

#[test]
fn test_time_interval_contains() {
    let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
    let stop = JulianDate::from_date_components(2000, 1, 3, 0, 0, 0, 0.0);
    let interval = TimeInterval::new(start, stop, true, true);

    let inside = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
    assert!(interval.contains(&inside));

    let outside = JulianDate::from_date_components(2000, 1, 4, 0, 0, 0, 0.0);
    assert!(!interval.contains(&outside));
}

#[test]
fn test_time_interval_is_empty() {
    let start = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
    let stop = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
    let interval = TimeInterval::new(start, stop, true, true);
    assert!(interval.is_empty());
}

// === Clock ===

#[test]
fn test_clock_new() {
    let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
    let stop = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
    let clock = Clock::new(start, stop, start);
    assert_eq!(clock.start_time, start);
    assert_eq!(clock.stop_time, stop);
    assert_eq!(clock.current_time, start);
}

#[test]
fn test_clock_tick() {
    let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
    let stop = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
    let mut clock = Clock::new(start, stop, start);
    clock.multiplier = 60.0;
    clock.clock_step = ClockStep::TickDependent;
    clock.should_animate = true;

    let new_time = clock.tick(1.0);
    let diff = new_time.seconds_difference(&start);
    assert_approx!(diff, 60.0, epsilon::EPSILON6);
}

#[test]
fn test_clock_range_clamp() {
    let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
    let stop = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
    let mut clock = Clock::new(start, stop, stop);
    clock.clock_range = ClockRange::Clamped;
    clock.clock_step = ClockStep::TickDependent;
    clock.multiplier = 60.0;
    clock.should_animate = true;

    let new_time = clock.tick(1.0);
    assert!(new_time <= stop);
}

#[test]
fn test_clock_range_loop() {
    let start = JulianDate::from_date_components(2000, 1, 1, 0, 0, 0, 0.0);
    let stop = JulianDate::from_date_components(2000, 1, 2, 0, 0, 0, 0.0);
    let mut clock = Clock::new(start, stop, stop);
    clock.clock_range = ClockRange::LoopStop;
    clock.clock_step = ClockStep::TickDependent;
    clock.multiplier = 60.0;
    clock.should_animate = true;

    let new_time = clock.tick(1.0);
    assert!(new_time >= start);
}
