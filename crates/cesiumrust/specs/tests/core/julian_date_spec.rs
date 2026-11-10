//! Core/JulianDateSpec → Rust integration tests
//! 162 original it() blocks. JS-specific tests (undefined params, Date type checks) skipped.
//! Ported: constructor, fromIso8601, toIso8601, toDate, arithmetic, comparison, computeTaiMinusUtc

use cesium_time::{JulianDate, TimeStandard};

/// Helper: create JulianDate from date components (UTC) - equivalent to JulianDate.fromDate(new Date(Date.UTC(...)))
fn from_utc(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32, ms: f64) -> JulianDate {
    JulianDate::from_date_components(y, m, d, h, min, s, ms)
}

// === Constructor tests ===

#[test]
fn construct_default_date() {
    // CesiumJS: new JulianDate() → dayNumber=0, secondsOfDay=10 (UTC 0 + 10s offset)
    let d = JulianDate::new(0.0, 0.0);
    // 默认是 UTC，因此内部 TAI = UTC + 10（第一个闰秒偏移）
    assert_eq!(d.day_number, 0);
    assert!((d.seconds_of_day - 10.0).abs() < 1e-10);
}

#[test]
fn construct_date_with_fractional_day() {
    let jd = JulianDate::with_time_standard(2448257.75, 0.0, TimeStandard::UTC);
    let expected = JulianDate::with_time_standard(2448257.0, 64826.0, TimeStandard::TAI);
    assert_eq!(jd, expected);
}

#[test]
fn construct_date_from_basic_tai_components() {
    let jd = JulianDate::with_time_standard(12.0, 12.5, TimeStandard::TAI);
    assert_eq!(jd.day_number, 12);
    assert!((jd.seconds_of_day - 12.5).abs() < 1e-10);
}

#[test]
fn construct_utc_just_before_leap_second() {
    let expected = JulianDate::with_time_standard(2443874.0, 43216.0, TimeStandard::TAI);
    let jd = JulianDate::with_time_standard(2443874.0, 43199.0, TimeStandard::UTC);
    assert_eq!(jd.day_number, expected.day_number);
    assert!((jd.seconds_of_day - expected.seconds_of_day).abs() < 1e-10);
}

#[test]
fn construct_utc_equivalent_to_leap_second_table_entry() {
    let expected = JulianDate::with_time_standard(2443874.0, 43218.0, TimeStandard::TAI);
    let jd = JulianDate::with_time_standard(2443874.0, 43200.0, TimeStandard::UTC);
    assert_eq!(jd.day_number, expected.day_number);
    assert!((jd.seconds_of_day - expected.seconds_of_day).abs() < 1e-10);
}

#[test]
fn construct_utc_just_after_leap_second() {
    let expected = JulianDate::with_time_standard(2443874.0, 43219.0, TimeStandard::TAI);
    let jd = JulianDate::with_time_standard(2443874.0, 43201.0, TimeStandard::UTC);
    assert_eq!(jd.day_number, expected.day_number);
    assert!((jd.seconds_of_day - expected.seconds_of_day).abs() < 1e-10);
}

#[test]
fn construct_with_more_seconds_than_a_day() {
    let jd = JulianDate::with_time_standard(12.0, 86401.0, TimeStandard::TAI);
    assert_eq!(jd.day_number, 13);
    assert!((jd.seconds_of_day - 1.0).abs() < 1e-10);
}

#[test]
fn construct_with_negative_seconds() {
    let jd = JulianDate::with_time_standard(12.0, -1.0, TimeStandard::TAI);
    assert_eq!(jd.day_number, 11);
    assert!((jd.seconds_of_day - 86399.0).abs() < 1e-10);
}

#[test]
fn construct_with_partial_day_and_seconds() {
    let jd = JulianDate::with_time_standard(12.5, -1.0, TimeStandard::TAI);
    assert_eq!(jd.day_number, 12);
    assert!((jd.seconds_of_day - 43199.0).abs() < 1e-10);
}

#[test]
fn construct_with_default_time_standard_is_utc() {
    let jd_default = JulianDate::new(12.0, 12.5);
    let jd_utc = JulianDate::with_time_standard(12.0, 12.5, TimeStandard::UTC);
    assert_eq!(jd_default, jd_utc);
}

// === fromDate equivalent tests (using from_date_components) ===

#[test]
fn from_date_jan_1_1991() {
    // January 1, 1991 06:00:00 UTC → dayNumber=2448257, secondsOfDay=64826
    let jd = from_utc(1991, 1, 1, 6, 0, 0, 0.0);
    assert_eq!(jd.day_number, 2448257);
    assert!((jd.seconds_of_day - 64826.0).abs() < 1e-10);
}

#[test]
fn from_date_july_4_2011() {
    // July 4, 2011 12:00:00 UTC → dayNumber=2455747, secondsOfDay=34
    let jd = from_utc(2011, 7, 4, 12, 0, 0, 0.0);
    assert_eq!(jd.day_number, 2455747);
    assert!((jd.seconds_of_day - 34.0).abs() < 1e-10);
}

#[test]
fn from_date_dec_31_2021() {
    // December 31, 2021 18:00:00 UTC → dayNumber=2459580, secondsOfDay=21637
    let jd = from_utc(2021, 12, 31, 18, 0, 0, 0.0);
    assert_eq!(jd.day_number, 2459580);
    assert!((jd.seconds_of_day - 21637.0).abs() < 1e-10);
}

#[test]
fn from_date_sep_1_2011() {
    // September 1, 2011 12:00:00 UTC → dayNumber=2455806, secondsOfDay=34
    let jd = from_utc(2011, 9, 1, 12, 0, 0, 0.0);
    assert_eq!(jd.day_number, 2455806);
    assert!((jd.seconds_of_day - 34.0).abs() < 1e-10);
}

#[test]
fn from_date_nov_17_2039() {
    // 11/17/2039 12:00:00 AM UTC → dayNumber=2466109, secondsOfDay=43237
    let jd = from_utc(2039, 11, 17, 0, 0, 0, 0.0);
    assert_eq!(jd.day_number, 2466109);
    assert!((jd.seconds_of_day - 43237.0).abs() < 1e-10);
}

// === fromIso8601 valid dates ===

#[test]
fn iso8601_calendar_date_basic() {
    let expected = from_utc(2009, 8, 1, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("20090801").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_calendar_date_extended() {
    let expected = from_utc(2009, 8, 1, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("2009-08-01").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_feb_29_basic() {
    let expected = from_utc(2000, 2, 29, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("20000229").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_feb_29_extended() {
    let expected = from_utc(2000, 2, 29, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("2000-02-29").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_ordinal_date_basic() {
    let expected = from_utc(1985, 4, 12, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("1985102").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_ordinal_date_extended() {
    let expected = from_utc(1985, 4, 12, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("1985-102").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_ordinal_leap_year() {
    let expected = from_utc(2000, 12, 31, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("2000-366").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_week_date_basic() {
    let expected = from_utc(1985, 4, 12, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("1985W155").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_week_date_extended() {
    let expected = from_utc(2008, 9, 27, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("2008-W39-6").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_calendar_week_basic() {
    let expected = from_utc(1985, 4, 7, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("1985W15").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_calendar_week_extended() {
    let expected = from_utc(2008, 9, 21, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("2008-W39").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_calendar_month() {
    let expected = from_utc(1985, 4, 1, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("1985-04").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_utc_time_basic() {
    let expected = from_utc(2009, 8, 1, 12, 30, 25, 0.0);
    let computed = JulianDate::from_iso8601("20090801T123025Z").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_utc_time_extended() {
    let expected = from_utc(2009, 8, 1, 12, 30, 25, 0.0);
    let computed = JulianDate::from_iso8601("2009-08-01T12:30:25Z").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_fractional_seconds_basic() {
    let expected = JulianDate::with_time_standard(2455045.0, 1825.5125423, TimeStandard::UTC);
    let computed = JulianDate::from_iso8601("20090801T123025.5125423Z").unwrap();
    assert!(computed.equals_epsilon(&expected, 1e-10));
}

#[test]
fn iso8601_fractional_seconds_extended() {
    let expected = JulianDate::with_time_standard(2455045.0, 1825.5125423, TimeStandard::UTC);
    let computed = JulianDate::from_iso8601("2009-08-01T12:30:25.5125423Z").unwrap();
    assert!(computed.equals_epsilon(&expected, 1e-10));
}

#[test]
fn iso8601_comma_fractional_seconds() {
    let expected = JulianDate::with_time_standard(2455045.0, 1825.5125423, TimeStandard::UTC);
    let computed = JulianDate::from_iso8601("20090801T123025,5125423Z").unwrap();
    assert!(computed.equals_epsilon(&expected, 1e-10));
}

#[test]
fn iso8601_no_seconds_basic() {
    let expected = from_utc(2009, 8, 1, 12, 30, 0, 0.0);
    let computed = JulianDate::from_iso8601("20090801T1230Z").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_no_seconds_extended() {
    let expected = from_utc(2009, 8, 1, 12, 30, 0, 0.0);
    let computed = JulianDate::from_iso8601("2009-08-01T12:30Z").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_fractional_minutes_basic() {
    let expected = from_utc(2009, 8, 1, 12, 30, 30, 0.0);
    let computed = JulianDate::from_iso8601("20090801T1230.5Z").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_fractional_minutes_extended() {
    let expected = from_utc(2009, 8, 1, 12, 30, 30, 0.0);
    let computed = JulianDate::from_iso8601("2009-08-01T12:30.5Z").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_no_minutes_seconds_basic() {
    let expected = from_utc(2009, 8, 1, 12, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("20090801T12Z").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_no_minutes_seconds_extended() {
    let expected = from_utc(2009, 8, 1, 12, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("2009-08-01T12Z").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_fractional_hours_basic() {
    let expected = from_utc(2009, 8, 1, 12, 30, 0, 0.0);
    let computed = JulianDate::from_iso8601("20090801T12.5Z").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_fractional_hours_extended() {
    let expected = from_utc(2009, 8, 1, 12, 30, 0, 0.0);
    let computed = JulianDate::from_iso8601("2009-08-01T12.5Z").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_leap_second() {
    let computed = JulianDate::from_iso8601("2008-12-31T23:59:60Z").unwrap();
    let expected = JulianDate::with_time_standard(2454832.0, 43233.0, TimeStandard::TAI);
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_within_leap_second() {
    let computed = JulianDate::from_iso8601("2008-12-31T23:59:60.123456789Z").unwrap();
    let expected = JulianDate::with_time_standard(2454832.0, 43233.123456789, TimeStandard::TAI);
    assert!(computed.equals_epsilon(&expected, 1e-10));
}

#[test]
fn iso8601_leap_second_offset_behind() {
    let computed = JulianDate::from_iso8601("2008-12-31T22:59:60-01").unwrap();
    let expected = JulianDate::with_time_standard(2454832.0, 43233.0, TimeStandard::TAI);
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_leap_second_offset_ahead() {
    let computed = JulianDate::from_iso8601("2009-01-01T00:59:60+01").unwrap();
    let expected = JulianDate::with_time_standard(2454832.0, 43233.0, TimeStandard::TAI);
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_24_midnight_notation() {
    let expected = from_utc(2009, 8, 2, 0, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("2009-08-01T24:00:00Z").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_offset_crosses_previous_month() {
    let expected = from_utc(1985, 3, 31, 23, 59, 0, 0.0);
    let computed = JulianDate::from_iso8601("1985-04-01T00:59:00+01").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_offset_crosses_next_month() {
    let expected = from_utc(1985, 4, 1, 0, 59, 0, 0.0);
    let computed = JulianDate::from_iso8601("1985-03-31T23:59:00-01").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_offset_crosses_next_year() {
    let expected = from_utc(2008, 12, 31, 23, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("2009-01-01T01:00:00+02").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_offset_crosses_previous_year() {
    let expected = from_utc(2009, 1, 1, 1, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("2008-12-31T23:00:00-02").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_utc_offset_basic() {
    let expected = from_utc(2008, 11, 10, 12, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("2008-11-10T14:00:00+02").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_utc_offset_extended() {
    let expected = from_utc(2008, 11, 10, 11, 30, 0, 0.0);
    let computed = JulianDate::from_iso8601("2008-11-10T14:00:00+02:30").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_zero_offset_extended() {
    let expected = from_utc(2008, 11, 10, 14, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("2008-11-10T14:00:00+00:00").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_zero_offset_short() {
    let expected = from_utc(2008, 11, 10, 14, 0, 0, 0.0);
    let computed = JulianDate::from_iso8601("2008-11-10T14:00:00+00").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_offset_no_seconds_basic() {
    let expected = from_utc(2009, 8, 1, 12, 30, 0, 0.0);
    let computed = JulianDate::from_iso8601("20090801T0730-0500").unwrap();
    assert_eq!(computed, expected);
}

#[test]
fn iso8601_offset_no_seconds_extended() {
    let expected = from_utc(2009, 8, 1, 12, 30, 0, 0.0);
    let computed = JulianDate::from_iso8601("2009-08-01T07:30-05:00").unwrap();
    assert_eq!(computed, expected);
}

// === fromIso8601 invalid dates ===

#[test]
fn iso8601_fail_ordinal_day_less_than_1() { assert!(JulianDate::from_iso8601("2009-000").is_none()); }
#[test]
fn iso8601_fail_ordinal_day_more_than_365() { assert!(JulianDate::from_iso8601("2001-366").is_none()); }
#[test]
fn iso8601_fail_invalid_yymmdd() { assert!(JulianDate::from_iso8601("200905").is_none()); }
#[test]
fn iso8601_fail_missing_t_delimiter() { assert!(JulianDate::from_iso8601("2009-08-0112:30.5Z").is_none()); }
#[test]
fn iso8601_fail_wrong_delimiter() { assert!(JulianDate::from_iso8601("2009-08-01Q12:30.5Z").is_none()); }
#[test]
fn iso8601_fail_garbage() { assert!(JulianDate::from_iso8601("this is not a date").is_none()); }
#[test]
fn iso8601_fail_interval() { assert!(JulianDate::from_iso8601("2007-03-01T13:00:00Z/2008-05-11T15:30:00Z").is_none()); }
#[test]
fn iso8601_fail_too_many_year_digits() { assert!(JulianDate::from_iso8601("20091-05-19").is_none()); }
#[test]
fn iso8601_fail_too_many_month_digits() { assert!(JulianDate::from_iso8601("2009-100-19").is_none()); }
#[test]
fn iso8601_fail_month_13() { assert!(JulianDate::from_iso8601("2009-13-19").is_none()); }
#[test]
fn iso8601_fail_month_0() { assert!(JulianDate::from_iso8601("2009-00-19").is_none()); }
#[test]
fn iso8601_fail_jan_32() { assert!(JulianDate::from_iso8601("2009-01-32").is_none()); }
#[test]
fn iso8601_fail_feb_29_non_leap() { assert!(JulianDate::from_iso8601("2009-02-29").is_none()); }
#[test]
fn iso8601_fail_feb_30_leap() { assert!(JulianDate::from_iso8601("2000-02-30").is_none()); }
#[test]
fn iso8601_fail_mar_32() { assert!(JulianDate::from_iso8601("2000-03-32").is_none()); }
#[test]
fn iso8601_fail_apr_31() { assert!(JulianDate::from_iso8601("2000-04-31").is_none()); }
#[test]
fn iso8601_fail_may_32() { assert!(JulianDate::from_iso8601("2000-05-32").is_none()); }
#[test]
fn iso8601_fail_jun_31() { assert!(JulianDate::from_iso8601("2000-06-31").is_none()); }
#[test]
fn iso8601_fail_jul_32() { assert!(JulianDate::from_iso8601("2000-07-32").is_none()); }
#[test]
fn iso8601_fail_aug_32() { assert!(JulianDate::from_iso8601("2000-08-32").is_none()); }
#[test]
fn iso8601_fail_sep_31() { assert!(JulianDate::from_iso8601("2000-09-31").is_none()); }
#[test]
fn iso8601_fail_oct_32() { assert!(JulianDate::from_iso8601("2000-10-32").is_none()); }
#[test]
fn iso8601_fail_nov_31() { assert!(JulianDate::from_iso8601("2000-11-31").is_none()); }
#[test]
fn iso8601_fail_dec_32() { assert!(JulianDate::from_iso8601("2000-12-32").is_none()); }
#[test]
fn iso8601_fail_hour_24_with_seconds() { assert!(JulianDate::from_iso8601("2000-12-15T24:00:01").is_none()); }
#[test]
fn iso8601_fail_hour_24_with_minutes() { assert!(JulianDate::from_iso8601("2000-12-15T24:01:00").is_none()); }
#[test]
fn iso8601_fail_minute_60() { assert!(JulianDate::from_iso8601("2000-12-15T12:60").is_none()); }
#[test]
fn iso8601_fail_second_61() { assert!(JulianDate::from_iso8601("2000-12-15T12:59:61").is_none()); }
#[test]
fn iso8601_fail_day_0() { assert!(JulianDate::from_iso8601("2009-01-00").is_none()); }
#[test]
fn iso8601_fail_too_many_dashes() { assert!(JulianDate::from_iso8601("2009--01-01").is_none()); }
#[test]
fn iso8601_fail_garbage_offset() { assert!(JulianDate::from_iso8601("2000-12-15T12:59:23ZZ+-050708::1234").is_none()); }
#[test]
fn iso8601_fail_double_decimal() { assert!(JulianDate::from_iso8601("2000-12-15T12:59:22..2").is_none()); }
#[test]
fn iso8601_fail_mixed_format_1() { assert!(JulianDate::from_iso8601("200108-01").is_none()); }
#[test]
fn iso8601_fail_mixed_format_2() { assert!(JulianDate::from_iso8601("2001-0801").is_none()); }
#[test]
fn iso8601_fail_week_mixed_1() { assert!(JulianDate::from_iso8601("2008-W396").is_none()); }
#[test]
fn iso8601_fail_week_mixed_2() { assert!(JulianDate::from_iso8601("2008W39-6").is_none()); }
#[test]
fn iso8601_fail_trailing_dash() { assert!(JulianDate::from_iso8601("2001-").is_none()); }
#[test]
fn iso8601_fail_time_mixed_1() { assert!(JulianDate::from_iso8601("2000-12-15T22:0100").is_none()); }
#[test]
fn iso8601_fail_time_mixed_2() { assert!(JulianDate::from_iso8601("2000-12-15T2201:00").is_none()); }

// === toDate equivalent (to_gregorian_date) ===

#[test]
fn to_date_works_with_tai() {
    let jd = JulianDate::with_time_standard(2455927.157772, 0.0, TimeStandard::UTC);
    let g = jd.to_gregorian_date();
    assert_eq!(g.year, 2011);
    assert_eq!(g.month, 12);
    assert_eq!(g.day, 31);
    assert_eq!(g.hour, 15);
    assert_eq!(g.minute, 47);
    assert_eq!(g.second, 11);
    assert!((g.millisecond - 500.0).abs() < 10.0);
}

#[test]
fn to_date_second_before_leap_second() {
    let jd = JulianDate::with_time_standard(2450630.0, 43229.0, TimeStandard::TAI);
    let g = jd.to_gregorian_date();
    assert_eq!(g.year, 1997);
    assert_eq!(g.month, 6);
    assert_eq!(g.day, 30);
    assert_eq!(g.hour, 23);
    assert_eq!(g.minute, 59);
    assert_eq!(g.second, 59);
}

#[test]
fn to_date_on_leap_second() {
    let jd = JulianDate::with_time_standard(2450630.0, 43230.0, TimeStandard::TAI);
    let g = jd.to_gregorian_date();
    // 闰秒期间：second=60 或重复的 59
    assert_eq!(g.year, 1997);
    assert_eq!(g.month, 6);
    assert_eq!(g.day, 30);
    assert_eq!(g.hour, 23);
    assert_eq!(g.minute, 59);
    assert_eq!(g.second, 60);
    assert!(g.is_leap_second);
}

#[test]
fn to_date_second_after_leap_second() {
    let jd = JulianDate::with_time_standard(2450630.0, 43231.0, TimeStandard::TAI);
    let g = jd.to_gregorian_date();
    assert_eq!(g.year, 1997);
    assert_eq!(g.month, 7);
    assert_eq!(g.day, 1);
    assert_eq!(g.hour, 0);
    assert_eq!(g.minute, 0);
    assert_eq!(g.second, 0);
}

#[test]
fn to_date_before_all_leap_seconds() {
    let jd = JulianDate::with_time_standard(2440109.0, 43210.0, TimeStandard::TAI);
    let g = jd.to_gregorian_date();
    assert_eq!(g.year, 1968);
    assert_eq!(g.month, 9);
    assert_eq!(g.day, 10);
    assert_eq!(g.hour, 0);
    assert_eq!(g.minute, 0);
    assert_eq!(g.second, 0);
}

#[test]
fn to_date_after_all_leap_seconds() {
    let jd = JulianDate::with_time_standard(2466109.0, 43237.0, TimeStandard::TAI);
    let g = jd.to_gregorian_date();
    assert_eq!(g.year, 2039);
    assert_eq!(g.month, 11);
    assert_eq!(g.day, 17);
    assert_eq!(g.hour, 0);
    assert_eq!(g.minute, 0);
    assert_eq!(g.second, 0);
}

// === toIso8601 ===

#[test]
fn to_iso8601_second_before_leap() {
    let s = "1997-06-30T23:59:59Z";
    let jd = JulianDate::from_iso8601(s).unwrap();
    assert_eq!(jd.to_iso8601(), s);
}

#[test]
fn to_iso8601_on_leap_second() {
    let s = "1997-06-30T23:59:60Z";
    let jd = JulianDate::from_iso8601(s).unwrap();
    assert_eq!(jd.to_iso8601(), s);
}

#[test]
fn to_iso8601_second_after_leap() {
    let s = "1997-07-01T00:00:00Z";
    let jd = JulianDate::from_iso8601(s).unwrap();
    assert_eq!(jd.to_iso8601(), s);
}

#[test]
fn to_iso8601_before_all_leap_seconds() {
    let s = "1968-09-10T00:00:00Z";
    let jd = JulianDate::from_iso8601(s).unwrap();
    assert_eq!(jd.to_iso8601(), s);
}

#[test]
fn to_iso8601_after_all_leap_seconds() {
    let s = "2031-11-17T00:00:00Z";
    let jd = JulianDate::from_iso8601(s).unwrap();
    assert_eq!(jd.to_iso8601(), s);
}

#[test]
fn to_iso8601_without_precision() {
    let s = "0950-01-02T03:04:05.5Z";
    let jd = JulianDate::from_iso8601(s).unwrap();
    assert_eq!(jd.to_iso8601(), s);
}

#[test]
fn to_iso8601_pads_zeros() {
    let s = "0950-01-02T03:04:05.005Z";
    let jd = JulianDate::from_iso8601(s).unwrap();
    assert_eq!(jd.to_iso8601_with_precision(Some(3)), s);
}

#[test]
fn to_iso8601_no_ms_when_zero() {
    let s = "0950-01-02T03:04:05Z";
    let jd = JulianDate::from_iso8601(s).unwrap();
    assert_eq!(jd.to_iso8601(), s);
}

#[test]
fn to_iso8601_with_precision() {
    let iso_date = "0950-01-02T03:04:05.012345Z";
    let jd = JulianDate::from_iso8601(iso_date).unwrap();
    assert_eq!(jd.to_iso8601_with_precision(Some(0)), "0950-01-02T03:04:05Z");
    assert_eq!(jd.to_iso8601_with_precision(Some(1)), "0950-01-02T03:04:05.0Z");
    assert_eq!(jd.to_iso8601_with_precision(Some(2)), "0950-01-02T03:04:05.01Z");
    assert_eq!(jd.to_iso8601_with_precision(Some(3)), "0950-01-02T03:04:05.012Z");
    assert_eq!(jd.to_iso8601_with_precision(Some(4)), "0950-01-02T03:04:05.0123Z");
    assert_eq!(jd.to_iso8601_with_precision(Some(5)), "0950-01-02T03:04:05.01234Z");
    assert_eq!(jd.to_iso8601_with_precision(Some(6)), "0950-01-02T03:04:05.012345Z");
    assert_eq!(jd.to_iso8601_with_precision(Some(7)), "0950-01-02T03:04:05.0123450Z");
}

// === secondsDifference / daysDifference ===

#[test]
fn seconds_difference_works() {
    let start = from_utc(2011, 7, 4, 12, 0, 0, 0.0);
    let end = from_utc(2011, 7, 5, 12, 1, 0, 0.0);
    let diff = end.seconds_difference(&start);
    assert!((diff - 86460.0).abs() < 1e-5);
}

#[test]
fn days_difference_works() {
    let start = from_utc(2011, 7, 4, 12, 0, 0, 0.0);
    let end = from_utc(2011, 7, 5, 14, 24, 0, 0.0);
    let diff = end.days_difference(&start);
    assert!((diff - 1.1).abs() < 1e-10);
}

#[test]
fn days_difference_negative() {
    let end = from_utc(2011, 7, 4, 12, 0, 0, 0.0);
    let start = from_utc(2011, 7, 5, 14, 24, 0, 0.0);
    let diff = end.days_difference(&start);
    assert!((diff - (-1.1)).abs() < 1e-10);
}

// === addSeconds / addMinutes / addHours / addDays ===

#[test]
fn add_seconds_whole() {
    let start = from_utc(2011, 7, 4, 12, 0, 30, 0.0);
    let end = start.add_seconds(95.0);
    let g = end.to_gregorian_date();
    assert_eq!(g.second, 5);
    assert_eq!(g.minute, 2);
}

#[test]
fn add_seconds_fraction_1() {
    let start = JulianDate::with_time_standard(2454832.0, 0.0, TimeStandard::TAI);
    let end = start.add_seconds(1.5);
    assert!((end.seconds_difference(&start) - 1.5).abs() < 1e-10);
}

#[test]
fn add_seconds_fraction_2() {
    let start = from_utc(2011, 8, 11, 6, 0, 0, 0.0);
    let end = start.add_seconds(0.5);
    assert!((end.seconds_difference(&start) - 0.5).abs() < 1e-10);
}

#[test]
fn add_seconds_negative() {
    let start = from_utc(2011, 7, 4, 12, 1, 30, 0.0);
    let end = start.add_seconds(-60.0);
    assert!((end.seconds_difference(&start) - (-60.0)).abs() < 1e-10);
}

#[test]
fn add_seconds_more_than_day() {
    let seconds = 86400.0 * 7.0 + 15.0;
    let start = JulianDate::with_time_standard(2448444.0, 0.0, TimeStandard::UTC);
    let end = start.add_seconds(seconds);
    assert!((end.seconds_difference(&start) - seconds).abs() < 1e-10);
}

#[test]
fn add_seconds_negative_more_than_day() {
    let seconds = -86400.0 * 7.0 - 15.0;
    let start = JulianDate::with_time_standard(2448444.0, 0.0, TimeStandard::UTC);
    let end = start.add_seconds(seconds);
    assert!((end.seconds_difference(&start) - seconds).abs() < 1e-10);
}

#[test]
fn add_minutes_works() {
    let start = from_utc(2011, 7, 4, 12, 0, 0, 0.0);
    let end = start.add_minutes(65.0);
    let g = end.to_gregorian_date();
    assert_eq!(g.minute, 5);
    assert_eq!(g.hour, 13);
}

#[test]
fn add_minutes_negative() {
    let start = from_utc(2011, 7, 4, 12, 0, 0, 0.0);
    let end = start.add_minutes(-35.0);
    let g = end.to_gregorian_date();
    assert_eq!(g.minute, 25);
    assert_eq!(g.hour, 11);
}

#[test]
fn add_hours_works() {
    let start = from_utc(2011, 7, 4, 12, 0, 0, 0.0);
    let end = start.add_hours(6.0);
    let g = end.to_gregorian_date();
    assert_eq!(g.hour, 18);
}

#[test]
fn add_hours_negative() {
    let start = from_utc(2011, 7, 4, 12, 0, 0, 0.0);
    let end = start.add_hours(-6.0);
    let g = end.to_gregorian_date();
    assert_eq!(g.hour, 6);
}

#[test]
fn add_days_works() {
    let start = from_utc(2011, 7, 4, 12, 0, 0, 0.0);
    let end = start.add_days(32.0);
    let g = end.to_gregorian_date();
    assert_eq!(g.day, 5);
    assert_eq!(g.month, 8);
}

#[test]
fn add_days_negative() {
    let start = from_utc(2011, 7, 4, 12, 0, 0, 0.0);
    let end = start.add_days(-4.0);
    let g = end.to_gregorian_date();
    assert_eq!(g.day, 30);
    assert_eq!(g.month, 6);
}

// === Comparison ===

#[test]
fn less_than_works() {
    let start = from_utc(1991, 7, 6, 12, 0, 0, 0.0);
    let end = from_utc(2011, 7, 6, 12, 1, 0, 0.0);
    assert!(start.less_than(&end));
}

#[test]
fn less_than_equal_values() {
    let start = from_utc(1991, 7, 6, 12, 0, 0, 0.0);
    let end = from_utc(1991, 7, 6, 12, 0, 0, 0.0);
    assert!(!start.less_than(&end));
    assert!(start.less_than(&end.add_seconds(1.0)));
}

#[test]
fn less_than_different_time_standards() {
    let start = JulianDate::with_time_standard(0.0, 0.0, TimeStandard::TAI);
    let end = JulianDate::with_time_standard(0.0, 0.0, TimeStandard::UTC);
    // UTC 0 → TAI +10, so TAI(0,0) < TAI(0,10)
    assert!(start.less_than(&end));
}

#[test]
fn greater_than_works() {
    let start = from_utc(2011, 7, 6, 12, 1, 0, 0.0);
    let end = from_utc(1991, 7, 6, 12, 0, 0, 0.0);
    assert!(start.greater_than(&end));
}

#[test]
fn greater_than_equal_values() {
    let start = from_utc(1991, 7, 6, 12, 0, 0, 0.0);
    let end = from_utc(1991, 7, 6, 12, 0, 0, 0.0);
    assert!(!start.greater_than(&end));
    assert!(start.greater_than(&end.add_seconds(-1.0)));
}

#[test]
fn greater_than_different_time_standards() {
    let start = JulianDate::with_time_standard(0.0, 0.0, TimeStandard::UTC);
    let end = JulianDate::with_time_standard(0.0, 0.0, TimeStandard::TAI);
    // UTC(0,0) → TAI(0,10) > TAI(0,0)
    assert!(start.greater_than(&end));
}

#[test]
fn equals_epsilon_works() {
    let original = from_utc(2011, 9, 7, 12, 55, 0, 0.0);
    let clone = original.add_seconds(1.0);
    assert!(original.equals_epsilon(&clone, 2.0));
}

// === totalDays ===

#[test]
fn total_days_works() {
    let total_days = 2455784.7500058;
    let jd = JulianDate::with_time_standard(total_days, 0.0, TimeStandard::TAI);
    assert!((jd.total_days() - total_days).abs() < 1e-10);
}

// === computeTaiMinusUtc ===

#[test]
fn compute_tai_minus_utc_before_all() {
    let jd = from_utc(1970, 7, 11, 12, 0, 0, 0.0);
    assert!((jd.compute_tai_minus_utc() - 10.0).abs() < 1e-10);
}

#[test]
fn compute_tai_minus_utc_second_before_leap() {
    let jd = JulianDate::with_time_standard(2456109.0, 43233.0, TimeStandard::TAI);
    assert!((jd.compute_tai_minus_utc() - 34.0).abs() < 1e-10);
}

#[test]
fn compute_tai_minus_utc_on_leap() {
    let jd = JulianDate::with_time_standard(2456109.0, 43234.0, TimeStandard::TAI);
    assert!((jd.compute_tai_minus_utc() - 34.0).abs() < 1e-10);
}

#[test]
fn compute_tai_minus_utc_second_after_leap() {
    let jd = JulianDate::with_time_standard(2456109.0, 43235.0, TimeStandard::TAI);
    assert!((jd.compute_tai_minus_utc() - 35.0).abs() < 1e-10);
}

#[test]
fn compute_tai_minus_utc_after_all() {
    let jd = JulianDate::with_time_standard(2556109.0, 43237.0, TimeStandard::TAI);
    assert!((jd.compute_tai_minus_utc() - 37.0).abs() < 1e-10);
}

// === fromGregorianDate roundtrip ===

#[test]
fn from_gregorian_date_roundtrip() {
    let iso1 = "2017-01-01T10:01:01.5Z";
    let julian1 = JulianDate::from_iso8601(iso1).unwrap();
    let gregorian = julian1.to_gregorian_date();
    let julian2 = JulianDate::from_gregorian_date(&gregorian);
    let iso2 = julian2.to_iso8601();
    assert_eq!(iso1, iso2);
    assert_eq!(julian1, julian2);
}
