//! 参考自 CesiumJS 的测试：
//! - Simon1994PlanetaryPositionsSpec（3 个 A 类：太阳位置、月亮位置、太阳从东方升起）
//! - Iau2000OrientationSpec（1 个 A 类：计算月亮）
//! - IauOrientationAxesSpec（1 个 A 类：计算 ICRF 到月固系）

use cesium_geospatial::simon1994_planetary_positions::{
    compute_moon_position_in_earth_inertial_frame, compute_sun_position_in_earth_inertial_frame,
};
use cesium_geospatial::iau_orientation::{compute_moon, evaluate_icrf_to_fixed};
use cesium_time::JulianDate;

const EPSILON2: f64 = 1.0e-2;
const EPSILON3: f64 = 1.0e-3;
const EPSILON4: f64 = 1.0e-4;
const EPSILON13: f64 = 1.0e-13;

/// 辅助函数：由 day_number + seconds_of_day（TAI）创建 JulianDate
fn jd(day: f64, seconds: f64) -> JulianDate {
    JulianDate::with_time_standard(day, seconds, cesium_time::TimeStandard::TAI)
}

// ===== Simon1994PlanetaryPositions: computeSunPositionInEarthInertialFrame =====

#[test]
fn test_computes_correct_sun_position() {
    // J2000 历元
    let date = jd(2451545.0, 0.0);
    let sun = compute_sun_position_in_earth_inertial_frame(&date);
    assert!((sun.x - 26500268539.790234).abs() < EPSILON2 * 26500268539.790234_f64.abs().max(1.0));
    assert!((sun.y - (-132756447253.27325)).abs() < EPSILON2 * 132756447253.27325);
    assert!((sun.z - (-57556483362.533806)).abs() < EPSILON2 * 57556483362.533806);

    // 2013-04-05
    let date = jd(2456401.5, 0.0);
    let sun = compute_sun_position_in_earth_inertial_frame(&date);
    assert!((sun.x - 131512388940.33589).abs() < EPSILON3 * 131512388940.33589);
    assert!((sun.y - 66661342667.949928).abs() < EPSILON3 * 66661342667.949928);
    assert!((sun.z - 28897975607.905258).abs() < EPSILON3 * 28897975607.905258);

    // 2012-03-01
    let date = jd(2455998.591667, 0.0);
    let sun = compute_sun_position_in_earth_inertial_frame(&date);
    assert!((sun.x - 147109989956.19534).abs() < EPSILON3 * 147109989956.19534);
    assert!((sun.y - (-19599996881.217579)).abs() < EPSILON3 * 19599996881.217579);
    assert!((sun.z - (-8497578102.7696457)).abs() < EPSILON3 * 8497578102.7696457);
}

// ===== Simon1994PlanetaryPositions: computeMoonPositionInEarthInertialFrame =====

#[test]
fn test_computes_correct_moon_position() {
    // J2000 历元
    let date = jd(2451545.0, 0.0);
    let moon = compute_moon_position_in_earth_inertial_frame(&date);
    assert!((moon.x - (-291632410.61232185)).abs() < EPSILON4 * 291632410.61232185);
    assert!((moon.y - (-266522146.36821631)).abs() < EPSILON4 * 266522146.36821631);
    assert!((moon.z - (-75994518.081043154)).abs() < EPSILON4 * 75994518.081043154);

    // 2013-04-05
    let date = jd(2456401.5, 0.0);
    let moon = compute_moon_position_in_earth_inertial_frame(&date);
    assert!((moon.x - (-223792974.4736526)).abs() < EPSILON4 * 223792974.4736526);
    assert!((moon.y - 315772435.34490639).abs() < EPSILON4 * 315772435.34490639);
    assert!((moon.z - 97913011.236112773).abs() < EPSILON4 * 97913011.236112773);

    // 2012-03-01
    let date = jd(2455998.591667, 0.0);
    let moon = compute_moon_position_in_earth_inertial_frame(&date);
    assert!((moon.x - (-268426117.00202647)).abs() < EPSILON4 * 268426117.00202647);
    assert!((moon.y - (-220468861.73998192)).abs() < EPSILON4 * 220468861.73998192);
    assert!((moon.z - (-110670164.58446842)).abs() < EPSILON4 * 110670164.58446842);
}

// ===== Simon1994PlanetaryPositions: sun rising in east, setting in west =====

/// 简化的 ICRF-到-地球固定系旋转（仅地球自转角）。
/// 足以展示周日运动。
fn icrf_to_fixed_rotation(jd_ut1: f64) -> glam::DMat3 {
    let du = jd_ut1 - 2451545.0;
    // IERS 2003 地球自转角
    let theta = 2.0 * std::f64::consts::PI * (0.7790572732640 + 1.00273781191135448 * du);
    let (s, c) = theta.sin_cos();
    // 绕 Z 轴旋转 -theta（ICRF → 固定系）
    glam::DMat3::from_cols_array(&[
        c, -s, 0.0,
        s,  c, 0.0,
        0.0, 0.0, 1.0,
    ])
}

#[test]
fn test_sun_rising_east_setting_west() {
    // 从 2011 年 7 月 6 日 @ 01:00 UTC 起、覆盖 24 小时的 julian 日期
    // 2011 年 7 月 6 日 00:00 UTC = JD 2455748.5
    let base_day = 2455748.5;
    let mut angles: Vec<f64> = Vec::new();

    for i in 1..25 {
        let date = jd(base_day, i as f64 * 3600.0);
        let position = compute_sun_position_in_earth_inertial_frame(&date);
        // 从惯性系变换到地球固定系
        let rot = icrf_to_fixed_rotation(date.total_days());
        let fixed_pos = rot * position;
        let angle = fixed_pos.y.atan2(fixed_pos.x);
        // convertLongitudeRange：映射到 [-PI, PI]
        let mut lon = angle;
        while lon > std::f64::consts::PI {
            lon -= 2.0 * std::f64::consts::PI;
        }
        while lon < -std::f64::consts::PI {
            lon += 2.0 * std::f64::consts::PI;
        }
        angles.push(lon);
    }

    // 预期在地球固定系中呈顺时针运动（角度递减）
    for i in 1..24 {
        assert!(
            angles[i] < angles[i - 1],
            "angles[{}] = {} should be < angles[{}] = {}",
            i, angles[i], i - 1, angles[i - 1]
        );
    }
}

// ===== Iau2000Orientation: ComputeMoon =====

#[test]
fn test_iau2000_compute_moon() {
    // date = new JulianDate(2451545.0, -32.184, TimeStandard.TAI)
    let date = jd(2451545.0, -32.184);
    let param = compute_moon(&date);

    // 来自 STK Components 的期望结果
    let expected_right_ascension = 4.6575460830237914;
    let expected_declination = 1.1456533675897986;
    let expected_rotation = 0.71899299269222972;
    let expected_rotation_rate = 0.0000026518066425764541;

    assert_eq!(param.right_ascension, expected_right_ascension);
    assert_eq!(param.declination, expected_declination);
    assert_eq!(param.rotation, expected_rotation);
    assert_eq!(param.rotation_rate, expected_rotation_rate);
}

// ===== IauOrientationAxes: evaluate (ICRF to Moon Fixed) =====

#[test]
fn test_iau_orientation_axes_evaluate() {
    // date = new JulianDate(2451545.0, -32.184, TimeStandard.TAI)
    let date = jd(2451545.0, -32.184);
    let mtx = evaluate_icrf_to_fixed(&date);

    // 来自 STK Components 的期望矩阵（在 CesiumJS 中为列主序）
    // Matrix3(col0row0, col0row1, col0row2, col1row0, col1row1, col1row2, col2row0, col2row1, col2row2)
    let expected = glam::DMat3::from_cols_array(&[
        0.784227052091917,    // col0, row0
        -0.62006191525085563, // col0, row1
        -0.022608671404182448, // col0, row2
        0.55784711246016394,  // col1, row0
        0.7205566654668133,   // col1, row1
        -0.41183090094261243, // col1, row2
        0.27165148607559436,  // col2, row0
        0.31035675134719942,  // col2, row1
        0.91097977859342938,  // col2, row2
    ]);

    let result_arr = mtx.to_cols_array();
    let expected_arr = expected.to_cols_array();
    for i in 0..9 {
        assert!(
            (result_arr[i] - expected_arr[i]).abs() < EPSILON13,
            "Matrix element [{}]: got {}, expected {}",
            i, result_arr[i], expected_arr[i]
        );
    }
}
