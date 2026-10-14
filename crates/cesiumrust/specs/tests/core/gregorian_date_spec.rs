//! Core/GregorianDateSpec → Rust 集成测试
//! 移植了 21 个原始 it() 块（跳过 1 个：JS 类型检查在 Rust 中不适用）

use cesium_time::GregorianDate;
use cesium_time::{is_leap_year, days_in_month};

/// 辅助函数：用缺省值创建 GregorianDate 的缺失参数（与 CesiumJS 构造函数一致）
fn greg(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32, ms: f64, leap: bool) -> GregorianDate {
    GregorianDate::new(y, m, d, h, min, s, ms, leap)
}

// === 使用合法参数 ===

#[test]
fn constructs_any_valid_date() {
    let d = greg(2022, 2, 4, 23, 54, 0, 999.9, false);
    assert_eq!(d.year, 2022);
    assert_eq!(d.month, 2);
    assert_eq!(d.day, 4);
    assert_eq!(d.hour, 23);
    assert_eq!(d.minute, 54);
    assert_eq!(d.second, 0);
    assert_eq!(d.millisecond, 999.9);
    assert!(!d.is_leap_second);
}

#[test]
fn constructs_valid_leap_year_date() {
    let d = greg(2024, 2, 29, 23, 54, 0, 999.9, false);
    assert_eq!(d.year, 2024);
    assert_eq!(d.month, 2);
    assert_eq!(d.day, 29);
    assert_eq!(d.hour, 23);
    assert_eq!(d.minute, 54);
    assert_eq!(d.second, 0);
    assert_eq!(d.millisecond, 999.9);
    assert!(!d.is_leap_second);
}

#[test]
fn constructs_minimum_date_when_no_parameters() {
    let d = GregorianDate::default();
    assert_eq!(d.year, 1);
    assert_eq!(d.month, 1);
    assert_eq!(d.day, 1);
    assert_eq!(d.hour, 0);
    assert_eq!(d.minute, 0);
    assert_eq!(d.second, 0);
    assert_eq!(d.millisecond, 0.0);
    assert!(!d.is_leap_second);
}

#[test]
fn constructs_valid_dates_for_edge_cases_of_days() {
    // 每个月的最大天数都不应 panic
    let _ = greg(2022, 1, 31, 0, 0, 0, 0.0, false);
    let _ = greg(2000, 2, 28, 0, 0, 0, 0.0, false);
    let _ = greg(2020, 2, 29, 0, 0, 0, 0.0, false); // 闰年
    let _ = greg(2022, 3, 31, 0, 0, 0, 0.0, false);
    let _ = greg(2022, 4, 30, 0, 0, 0, 0.0, false);
    let _ = greg(2022, 5, 31, 0, 0, 0, 0.0, false);
    let _ = greg(2022, 6, 30, 0, 0, 0, 0.0, false);
    let _ = greg(2022, 7, 31, 0, 0, 0, 0.0, false);
    let _ = greg(2022, 8, 31, 0, 0, 0, 0.0, false);
    let _ = greg(2022, 9, 30, 0, 0, 0, 0.0, false);
    let _ = greg(2022, 10, 31, 0, 0, 0, 0.0, false);
    let _ = greg(2022, 11, 30, 0, 0, 0, 0.0, false);
    let _ = greg(2022, 12, 31, 0, 0, 0, 0.0, false);
}

#[test]
fn constructs_minimum_date_with_only_year() {
    // CesiumJS：new GregorianDate(2022) → 其余使用缺省值
    let d = greg(2022, 1, 1, 0, 0, 0, 0.0, false);
    assert_eq!(d.year, 2022);
    assert_eq!(d.month, 1);
    assert_eq!(d.day, 1);
    assert_eq!(d.hour, 0);
    assert_eq!(d.minute, 0);
    assert_eq!(d.second, 0);
    assert_eq!(d.millisecond, 0.0);
    assert!(!d.is_leap_second);
}

#[test]
fn constructs_minimum_date_with_year_and_month() {
    let d = greg(2022, 2, 1, 0, 0, 0, 0.0, false);
    assert_eq!(d.year, 2022);
    assert_eq!(d.month, 2);
    assert_eq!(d.day, 1);
    assert_eq!(d.hour, 0);
    assert_eq!(d.minute, 0);
    assert_eq!(d.second, 0);
    assert_eq!(d.millisecond, 0.0);
    assert!(!d.is_leap_second);
}

#[test]
fn constructs_minimum_time_with_year_month_day() {
    let d = greg(2022, 2, 28, 0, 0, 0, 0.0, false);
    assert_eq!(d.year, 2022);
    assert_eq!(d.month, 2);
    assert_eq!(d.day, 28);
    assert_eq!(d.hour, 0);
    assert_eq!(d.minute, 0);
    assert_eq!(d.second, 0);
    assert_eq!(d.millisecond, 0.0);
    assert!(!d.is_leap_second);
}

#[test]
fn constructs_with_year_month_day_hour() {
    let d = greg(2022, 2, 28, 10, 0, 0, 0.0, false);
    assert_eq!(d.year, 2022);
    assert_eq!(d.month, 2);
    assert_eq!(d.day, 28);
    assert_eq!(d.hour, 10);
    assert_eq!(d.minute, 0);
    assert_eq!(d.second, 0);
    assert_eq!(d.millisecond, 0.0);
    assert!(!d.is_leap_second);
}

#[test]
fn constructs_with_year_month_day_hour_minute() {
    let d = greg(2022, 2, 28, 10, 59, 0, 0.0, false);
    assert_eq!(d.year, 2022);
    assert_eq!(d.month, 2);
    assert_eq!(d.day, 28);
    assert_eq!(d.hour, 10);
    assert_eq!(d.minute, 59);
    assert_eq!(d.second, 0);
    assert_eq!(d.millisecond, 0.0);
    assert!(!d.is_leap_second);
}

#[test]
fn constructs_with_year_month_day_hour_minute_second() {
    let d = greg(2022, 2, 28, 10, 59, 59, 0.0, false);
    assert_eq!(d.year, 2022);
    assert_eq!(d.month, 2);
    assert_eq!(d.day, 28);
    assert_eq!(d.hour, 10);
    assert_eq!(d.minute, 59);
    assert_eq!(d.second, 59);
    assert_eq!(d.millisecond, 0.0);
    assert!(!d.is_leap_second);
}

#[test]
fn constructs_date_with_leap_second() {
    let d = greg(2022, 2, 28, 10, 59, 60, 100.0, true);
    assert_eq!(d.year, 2022);
    assert_eq!(d.month, 2);
    assert_eq!(d.day, 28);
    assert_eq!(d.hour, 10);
    assert_eq!(d.minute, 59);
    assert_eq!(d.second, 60);
    assert_eq!(d.millisecond, 100.0);
    assert!(d.is_leap_second);
}

// === 使用非法参数（debug_assert! 在 debug 构建中会 panic）===

#[test]
#[should_panic]
fn throws_for_invalid_year_negative() {
    let _ = greg(-1, 2, 4, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_invalid_year_zero() {
    let _ = greg(0, 2, 4, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_invalid_year_10000() {
    let _ = greg(10000, 2, 4, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_invalid_month_negative() {
    let _ = greg(2022, 0, 4, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_invalid_month_13() {
    let _ = greg(2022, 13, 4, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_invalid_day_zero() {
    let _ = greg(2022, 12, 0, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_invalid_day_32() {
    let _ = greg(2022, 12, 32, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_day_out_of_range_feb_30_leap() {
    let _ = greg(2020, 2, 30, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_day_out_of_range_nov_31() {
    let _ = greg(2022, 11, 31, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_day_out_of_range_apr_31() {
    let _ = greg(2022, 4, 31, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_day_out_of_range_jun_31() {
    let _ = greg(2022, 6, 31, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_day_out_of_range_sep_31() {
    let _ = greg(2022, 9, 31, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_leap_day_non_leap_year() {
    let _ = greg(2022, 2, 29, 23, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_invalid_hour_24() {
    let _ = greg(2022, 2, 4, 24, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_invalid_hour_100() {
    let _ = greg(2022, 11, 4, 100, 54, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_invalid_minute_60() {
    let _ = greg(2022, 2, 4, 15, 60, 0, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_invalid_second_60_not_leap() {
    let _ = greg(2022, 2, 4, 15, 59, 60, 999.9, false);
}

#[test]
#[should_panic]
fn throws_for_invalid_second_61_even_leap() {
    let _ = greg(2022, 2, 4, 7, 59, 61, 999.9, true);
}

#[test]
#[should_panic]
fn throws_for_invalid_millisecond_1000() {
    let _ = greg(2022, 2, 4, 15, 59, 59, 1000.0, false);
}

#[test]
#[should_panic]
fn throws_for_invalid_millisecond_negative() {
    let _ = greg(2022, 2, 4, 15, 1, 0, -1.0, false);
}

// === is_leap_year / days_in_month 工具函数 ===

#[test]
fn test_is_leap_year() {
    assert!(is_leap_year(2000));
    assert!(is_leap_year(2020));
    assert!(is_leap_year(2024));
    assert!(!is_leap_year(1900));
    assert!(!is_leap_year(2022));
    assert!(!is_leap_year(2023));
}

#[test]
fn test_days_in_month() {
    assert_eq!(days_in_month(2022, 1), 31);
    assert_eq!(days_in_month(2022, 2), 28);
    assert_eq!(days_in_month(2020, 2), 29);
    assert_eq!(days_in_month(2000, 2), 29);
    assert_eq!(days_in_month(1900, 2), 28);
    assert_eq!(days_in_month(2022, 4), 30);
    assert_eq!(days_in_month(2022, 12), 31);
}
