//! Simon1994PlanetaryPositions - 在地心惯性系中计算日/月位置。
//!
//! 参考：Simon et al. 1994, "Numerical expressions for precession formulae
//! and mean elements for the Moon and the planets"
//!
//! 本模块按 Simon 等人 1994 年的数值展开式，先求地月质心与月球、地球在
//! Simon1994 参考系中的位置，再经固定的坐标轴变换矩阵换算到 J2000 地心
//! 惯性系，对外提供 [`compute_sun_position_in_earth_inertial_frame`] 与
//! [`compute_moon_position_in_earth_inertial_frame`] 两个入口。轨道根数到笛卡尔
//! 位置的转换基于简化的二体模型，平近点角经牛顿迭代解算为真近点角。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(dead_code, unused_mut, clippy::neg_multiply)]
use cesium_time::JulianDate;
use glam::{DVec3, DMat3};

const TDT_MINUS_TAI: f64 = 32.184;
const J2000D: f64 = 2451545.0;
const METERS_PER_KILOMETER: f64 = 1000.0;
const RADIANS_PER_DEGREE: f64 = std::f64::consts::PI / 180.0;
const RADIANS_PER_ARC_SECOND: f64 = std::f64::consts::PI / (180.0 * 3600.0);
const METERS_PER_ASTRONOMICAL_UNIT: f64 = 1.4959787e11; // IAU 1976 值
const SECONDS_PER_DAY: f64 = 86400.0;
const DAYS_PER_JULIAN_CENTURY: f64 = 36525.0;
const TWO_PI: f64 = 2.0 * std::f64::consts::PI;
const EPSILON8: f64 = 1.0e-8;

/// 计算 TDB 与 TT 的差值（秒），采用 Spencer 的近似公式。
///
/// # 参数
/// - `days_since_j2000_in_terrestrial_time`：以 TT 计的自 J2000 起天数。
///
/// # 返回
/// `TDB - TT`（秒），量级约毫秒，源于地球轨道偏心率引起的周期项。
fn compute_tdb_minus_tt_spice(days_since_j2000_in_terrestrial_time: f64) -> f64 {
    let g = 6.239996 + 0.0172019696544 * days_since_j2000_in_terrestrial_time;
    1.657e-3 * (g + 1.671e-2 * g.sin()).sin()
}

/// 将给定的 TAI 儒略日期转换为 TDB 儒略日期。
///
/// # 参数
/// - `date`：以 TAI 计的时刻。
///
/// # 返回
/// 先加固定常数得到 TT，再叠加 TDB-TT 周期项得到的 TDB 时刻。
fn tai_to_tdb(date: &JulianDate) -> JulianDate {
    // 将 TAI 转换为 TT
    let tt = date.add_seconds(TDT_MINUS_TAI);
    // 将 TT 转换为 TDB
    let days = tt.total_days() - J2000D;
    tt.add_seconds(compute_tdb_minus_tt_spice(days))
}

/// 将任意角度归一化到 `[0, 2π)` 区间。
///
/// # 参数
/// - `angle`：待归一化的角度（弧度）。
///
/// # 返回
/// 与之终边相同、落在 `[0, 2π)` 内的角度。
fn zero_to_two_pi(angle: f64) -> f64 {
    let mut result = angle % TWO_PI;
    if result < 0.0 {
        result += TWO_PI;
    }
    result
}

/// 由平近点角求解偏近点角（牛顿-拉夫森迭代解开普勒方程）。
///
/// # 参数
/// - `mean_anomaly`：平近点角 M（弧度）。
/// - `eccentricity`：轨道偏心率 e。
///
/// # 返回
/// 偏近点角 E，满足 `M = E - e·sinE`；迭代收敛后补回被剥离的整圈数。
fn mean_anomaly_to_eccentric_anomaly(mean_anomaly: f64, eccentricity: f64) -> f64 {
    let revs = (mean_anomaly / TWO_PI).floor();
    let mut ma = mean_anomaly - revs * TWO_PI;

    // 迭代的起始值
    let mut iteration_value = ma
        + (eccentricity * ma.sin()) / (1.0 - (ma + eccentricity).sin() + ma.sin());

    // 对开普勒方程进行 Newton-Raphson 迭代
    let mut eccentric_anomaly = f64::MAX;
    for _ in 0..50 {
        if (eccentric_anomaly - iteration_value).abs() <= EPSILON8 {
            break;
        }
        eccentric_anomaly = iteration_value;
        let nr_function =
            eccentric_anomaly - eccentricity * eccentric_anomaly.sin() - ma;
        let d_nr_function = 1.0 - eccentricity * eccentric_anomaly.cos();
        iteration_value = eccentric_anomaly - nr_function / d_nr_function;
    }

    iteration_value + revs * TWO_PI
}

/// 由偏近点角求解真近点角。
///
/// # 参数
/// - `eccentric_anomaly`：偏近点角 E（弧度）。
/// - `eccentricity`：轨道偏心率 e。
///
/// # 返回
/// 真近点角 ν（弧度），保持与输入的整圈数一致。
fn eccentric_anomaly_to_true_anomaly(eccentric_anomaly: f64, eccentricity: f64) -> f64 {
    let revs = (eccentric_anomaly / TWO_PI).floor();
    let ea = eccentric_anomaly - revs * TWO_PI;

    let true_anomaly_x = ea.cos() - eccentricity;
    let true_anomaly_y = ea.sin() * (1.0 - eccentricity * eccentricity).sqrt();

    let mut true_anomaly = true_anomaly_y.atan2(true_anomaly_x);
    true_anomaly = zero_to_two_pi(true_anomaly);
    if ea < 0.0 {
        true_anomaly -= TWO_PI;
    }

    true_anomaly + revs * TWO_PI
}

/// 由平近点角直接求取真近点角（串联偏近点角两步转换）。
///
/// # 参数
/// - `mean_anomaly`：平近点角 M（弧度）。
/// - `eccentricity`：轨道偏心率 e。
///
/// # 返回
/// 真近点角 ν（弧度）。
fn mean_anomaly_to_true_anomaly(mean_anomaly: f64, eccentricity: f64) -> f64 {
    let ea = mean_anomaly_to_eccentric_anomaly(mean_anomaly, eccentricity);
    eccentric_anomaly_to_true_anomaly(ea, eccentricity)
}

/// 计算从近焦点（PQW）系到惯性笛卡尔系的变换矩阵。
/// 返回一个 DMat3（列主序）。
///
/// # 参数
/// - `argument_of_periapsis`：近地点幅角 ω（弧度）。
/// - `inclination`：轨道倾角 i（弧度）。
/// - `right_ascension`：升交点赤经 Ω（弧度）。
fn perifocal_to_cartesian_matrix(
    argument_of_periapsis: f64,
    inclination: f64,
    right_ascension: f64,
) -> DMat3 {
    let cosap = argument_of_periapsis.cos();
    let sinap = argument_of_periapsis.sin();
    let cosi = inclination.cos();
    let sini = inclination.sin();
    let cosraan = right_ascension.cos();
    let sinraan = right_ascension.sin();

    // 列主序：col0、col1、col2
    DMat3::from_cols_array(&[
        cosraan * cosap - sinraan * sinap * cosi,  // col0, row0
        sinraan * cosap + cosraan * sinap * cosi,  // col0, row1
        sinap * sini,                               // col0, row2
        -cosraan * sinap - sinraan * cosap * cosi, // col1, row0
        -sinraan * sinap + cosraan * cosap * cosi, // col1, row1
        cosap * sini,                               // col1, row2
        sinraan * sini,                             // col2, row0
        -cosraan * sini,                            // col2, row1
        cosi,                                       // col2, row2
    ])
}

/// 由轨道根数计算惯性笛卡尔位置（Simon1994 简化二体模型）。
///
/// # 参数
/// - `semimajor_axis`：半长轴（米）。
/// - `eccentricity`：偏心率 e。
/// - `inclination`：倾角 i（弧度）；为负时翻转并调整升交点经度。
/// - `longitude_of_perigee`：近地点经度 ϖ（弧度）。
/// - `longitude_of_node`：升交点经度 Ω（弧度）。
/// - `mean_longitude`：平经度 L（弧度）。
///
/// # 返回
/// 近焦点系位置经旋转矩阵变换到惯性系后的笛卡尔坐标。
fn elements_to_cartesian(
    semimajor_axis: f64,
    eccentricity: f64,
    mut inclination: f64,
    longitude_of_perigee: f64,
    mut longitude_of_node: f64,
    mean_longitude: f64,
) -> DVec3 {
    if inclination < 0.0 {
        inclination = -inclination;
        longitude_of_node += std::f64::consts::PI;
    }

    let radius_of_periapsis = semimajor_axis * (1.0 - eccentricity);
    let argument_of_periapsis = longitude_of_perigee - longitude_of_node;
    let right_ascension_of_ascending_node = longitude_of_node;
    let true_anomaly =
        mean_anomaly_to_true_anomaly(mean_longitude - longitude_of_perigee, eccentricity);

    let perifocal_to_equatorial = perifocal_to_cartesian_matrix(
        argument_of_periapsis,
        inclination,
        right_ascension_of_ascending_node,
    );

    let semilatus = radius_of_periapsis * (1.0 + eccentricity);
    let costheta = true_anomaly.cos();
    let sintheta = true_anomaly.sin();
    let denom = 1.0 + eccentricity * costheta;
    let radius = semilatus / denom;

    let result = DVec3::new(radius * costheta, radius * sintheta, 0.0);
    perifocal_to_equatorial * result
}

// 来自第 5.8 节
const SEMI_MAJOR_AXIS0: f64 = 1.0000010178; // * METERS_PER_ASTRONOMICAL_UNIT
const MEAN_LONGITUDE0: f64 = 100.46645683; // 角度
const MEAN_LONGITUDE1: f64 = 1295977422.83429; // 角秒

// 来自表 6
const P1U: f64 = 16002.0;
const P2U: f64 = 21863.0;
const P3U: f64 = 32004.0;
const P4U: f64 = 10931.0;
const P5U: f64 = 14529.0;
const P6U: f64 = 16368.0;
const P7U: f64 = 15318.0;
const P8U: f64 = 32794.0;

const CA1: f64 = 64.0e-7;
const CA2: f64 = -152.0e-7;
const CA3: f64 = 62.0e-7;
const CA4: f64 = -8.0e-7;
const CA5: f64 = 32.0e-7;
const CA6: f64 = -41.0e-7;
const CA7: f64 = 19.0e-7;
const CA8: f64 = -11.0e-7;

const SA1: f64 = -150.0e-7;
const SA2: f64 = -46.0e-7;
const SA3: f64 = 68.0e-7;
const SA4: f64 = 54.0e-7;
const SA5: f64 = 14.0e-7;
const SA6: f64 = 24.0e-7;
const SA7: f64 = -28.0e-7;
const SA8: f64 = 22.0e-7;

const Q1U: f64 = 10.0;
const Q2U: f64 = 16002.0;
const Q3U: f64 = 21863.0;
const Q4U: f64 = 10931.0;
const Q5U: f64 = 1473.0;
const Q6U: f64 = 32004.0;
const Q7U: f64 = 4387.0;
const Q8U: f64 = 73.0;

const CL1: f64 = -325.0e-7;
const CL2: f64 = -322.0e-7;
const CL3: f64 = -79.0e-7;
const CL4: f64 = 232.0e-7;
const CL5: f64 = -52.0e-7;
const CL6: f64 = 97.0e-7;
const CL7: f64 = 55.0e-7;
const CL8: f64 = -41.0e-7;

const SL1: f64 = -105.0e-7;
const SL2: f64 = -137.0e-7;
const SL3: f64 = 258.0e-7;
const SL4: f64 = 35.0e-7;
const SL5: f64 = -116.0e-7;
const SL6: f64 = -88.0e-7;
const SL7: f64 = -112.0e-7;
const SL8: f64 = -80.0e-7;

/// 获取一个描述地月质心运动的点（第 6 节）。
///
/// # 参数
/// - `date`：儒略日期（TAI），内部换算为 TDB 后求自 J2000 的世纪数。
///
/// # 返回
/// 地月质心在 Simon1994 坐标系中的惯性位置（米）。
fn compute_simon_earth_moon_barycenter(date: &JulianDate) -> DVec3 {
    let tdb = tai_to_tdb(date);
    let x = tdb.total_days() - J2000D;
    let t = x / (DAYS_PER_JULIAN_CENTURY * 10.0);

    let u = 0.3595362 * t;
    let semimajor_axis = SEMI_MAJOR_AXIS0 * METERS_PER_ASTRONOMICAL_UNIT
        + CA1 * METERS_PER_ASTRONOMICAL_UNIT * (P1U * u).cos()
        + SA1 * METERS_PER_ASTRONOMICAL_UNIT * (P1U * u).sin()
        + CA2 * METERS_PER_ASTRONOMICAL_UNIT * (P2U * u).cos()
        + SA2 * METERS_PER_ASTRONOMICAL_UNIT * (P2U * u).sin()
        + CA3 * METERS_PER_ASTRONOMICAL_UNIT * (P3U * u).cos()
        + SA3 * METERS_PER_ASTRONOMICAL_UNIT * (P3U * u).sin()
        + CA4 * METERS_PER_ASTRONOMICAL_UNIT * (P4U * u).cos()
        + SA4 * METERS_PER_ASTRONOMICAL_UNIT * (P4U * u).sin()
        + CA5 * METERS_PER_ASTRONOMICAL_UNIT * (P5U * u).cos()
        + SA5 * METERS_PER_ASTRONOMICAL_UNIT * (P5U * u).sin()
        + CA6 * METERS_PER_ASTRONOMICAL_UNIT * (P6U * u).cos()
        + SA6 * METERS_PER_ASTRONOMICAL_UNIT * (P6U * u).sin()
        + CA7 * METERS_PER_ASTRONOMICAL_UNIT * (P7U * u).cos()
        + SA7 * METERS_PER_ASTRONOMICAL_UNIT * (P7U * u).sin()
        + CA8 * METERS_PER_ASTRONOMICAL_UNIT * (P8U * u).cos()
        + SA8 * METERS_PER_ASTRONOMICAL_UNIT * (P8U * u).sin();

    let mean_longitude = MEAN_LONGITUDE0 * RADIANS_PER_DEGREE
        + MEAN_LONGITUDE1 * RADIANS_PER_ARC_SECOND * t
        + CL1 * (Q1U * u).cos()
        + SL1 * (Q1U * u).sin()
        + CL2 * (Q2U * u).cos()
        + SL2 * (Q2U * u).sin()
        + CL3 * (Q3U * u).cos()
        + SL3 * (Q3U * u).sin()
        + CL4 * (Q4U * u).cos()
        + SL4 * (Q4U * u).sin()
        + CL5 * (Q5U * u).cos()
        + SL5 * (Q5U * u).sin()
        + CL6 * (Q6U * u).cos()
        + SL6 * (Q6U * u).sin()
        + CL7 * (Q7U * u).cos()
        + SL7 * (Q7U * u).sin()
        + CL8 * (Q8U * u).cos()
        + SL8 * (Q8U * u).sin();

    // 所有常量均来自第 5.8 节
    let eccentricity = 0.0167086342 - 0.0004203654 * t;
    let longitude_of_perigee =
        102.93734808 * RADIANS_PER_DEGREE + 11612.3529 * RADIANS_PER_ARC_SECOND * t;
    let inclination = 469.97289 * RADIANS_PER_ARC_SECOND * t;
    let longitude_of_node =
        174.87317577 * RADIANS_PER_DEGREE - 8679.27034 * RADIANS_PER_ARC_SECOND * t;

    elements_to_cartesian(
        semimajor_axis,
        eccentricity,
        inclination,
        longitude_of_perigee,
        longitude_of_node,
        mean_longitude,
    )
}

/// 获取一个描述月球位置的点（第 4 节）。
///
/// # 参数
/// - `date`：儒略日期（TAI），内部换算为 TDB 并求各阶时间幂。
///
/// # 返回
/// 月球在 Simon1994 坐标系中的惯性位置（米），含大量周期撒动项。
fn compute_simon_moon(date: &JulianDate) -> DVec3 {
    let tdb = tai_to_tdb(date);
    let x = tdb.total_days() - J2000D;
    let t = x / DAYS_PER_JULIAN_CENTURY;
    let t2 = t * t;
    let t3 = t2 * t;
    let t4 = t3 * t;

    // 来自第 3.4 节（b.1）的项
    let mut semimajor_axis = 383397.7725 + 0.004 * t;
    let mut eccentricity = 0.055545526 - 0.000000016 * t;
    let inclination_constant = 5.15668983 * RADIANS_PER_DEGREE;
    let mut inclination_sec_part =
        -0.00008 * t + 0.02966 * t2 - 0.000042 * t3 - 0.00000013 * t4;
    let longitude_of_perigee_constant = 83.35324312 * RADIANS_PER_DEGREE;
    let mut longitude_of_perigee_sec_part =
        14643420.2669 * t - 38.2702 * t2 - 0.045047 * t3 + 0.00021301 * t4;
    let longitude_of_node_constant = 125.04455501 * RADIANS_PER_DEGREE;
    let mut longitude_of_node_sec_part =
        -6967919.3631 * t + 6.3602 * t2 + 0.007625 * t3 - 0.00003586 * t4;
    let mean_longitude_constant = 218.31664563 * RADIANS_PER_DEGREE;
    let mut mean_longitude_sec_part =
        1732559343.4847 * t - 6.391 * t2 + 0.006588 * t3 - 0.00003169 * t4;

    // 来自第 3.5 节 b 的 Delaunay 参数
    let d = 297.85019547 * RADIANS_PER_DEGREE
        + RADIANS_PER_ARC_SECOND
            * (1602961601.209 * t - 6.3706 * t2 + 0.006593 * t3 - 0.00003169 * t4);
    let f = 93.27209062 * RADIANS_PER_DEGREE
        + RADIANS_PER_ARC_SECOND
            * (1739527262.8478 * t - 12.7512 * t2 - 0.001037 * t3 + 0.00000417 * t4);
    let l = 134.96340251 * RADIANS_PER_DEGREE
        + RADIANS_PER_ARC_SECOND
            * (1717915923.2178 * t + 31.8792 * t2 + 0.051635 * t3 - 0.0002447 * t4);
    let lprime = 357.52910918 * RADIANS_PER_DEGREE
        + RADIANS_PER_ARC_SECOND
            * (129596581.0481 * t - 0.5532 * t2 + 0.000136 * t3 - 0.00001149 * t4);
    let psi = 310.17137918 * RADIANS_PER_DEGREE
        - RADIANS_PER_ARC_SECOND
            * (6967051.436 * t + 6.2068 * t2 + 0.007618 * t3 - 0.00003219 * t4);

    // 添加来自表 4 的项
    let two_d = 2.0 * d;
    let four_d = 4.0 * d;
    let six_d = 6.0 * d;
    let two_l = 2.0 * l;
    let three_l = 3.0 * l;
    let four_l = 4.0 * l;
    let two_f = 2.0 * f;

    semimajor_axis += 3400.4 * two_d.cos()
        - 635.6 * (two_d - l).cos()
        - 235.6 * l.cos()
        + 218.1 * (two_d - lprime).cos()
        + 181.0 * (two_d + l).cos();

    eccentricity += 0.014216 * (two_d - l).cos()
        + 0.008551 * (two_d - two_l).cos()
        - 0.001383 * l.cos()
        + 0.001356 * (two_d + l).cos()
        - 0.001147 * (four_d - three_l).cos()
        - 0.000914 * (four_d - two_l).cos()
        + 0.000869 * (two_d - lprime - l).cos()
        - 0.000627 * two_d.cos()
        - 0.000394 * (four_d - four_l).cos()
        + 0.000282 * (two_d - lprime - two_l).cos()
        - 0.000279 * (d - l).cos()
        - 0.000236 * two_l.cos()
        + 0.000231 * four_d.cos()
        + 0.000229 * (six_d - four_l).cos()
        - 0.000201 * (two_l - two_f).cos();

    inclination_sec_part += 486.26 * (two_d - two_f).cos()
        - 40.13 * two_d.cos()
        + 37.51 * two_f.cos()
        + 25.73 * (two_l - two_f).cos()
        + 19.97 * (two_d - lprime - two_f).cos();

    longitude_of_perigee_sec_part += -55609.0 * (two_d - l).sin()
        - 34711.0 * (two_d - two_l).sin()
        - 9792.0 * l.sin()
        + 9385.0 * (four_d - three_l).sin()
        + 7505.0 * (four_d - two_l).sin()
        + 5318.0 * (two_d + l).sin()
        + 3484.0 * (four_d - four_l).sin()
        - 3417.0 * (two_d - lprime - l).sin()
        - 2530.0 * (six_d - four_l).sin()
        - 2376.0 * two_d.sin()
        - 2075.0 * (two_d - three_l).sin()
        - 1883.0 * two_l.sin()
        - 1736.0 * (six_d - 5.0 * l).sin()
        + 1626.0 * lprime.sin()
        - 1370.0 * (six_d - three_l).sin();

    longitude_of_node_sec_part += -5392.0 * (two_d - two_f).sin()
        - 540.0 * lprime.sin()
        - 441.0 * two_d.sin()
        + 423.0 * two_f.sin()
        - 288.0 * (two_l - two_f).sin();

    mean_longitude_sec_part += -3332.9 * two_d.sin()
        + 1197.4 * (two_d - l).sin()
        - 662.5 * lprime.sin()
        + 396.3 * l.sin()
        - 218.0 * (two_d - lprime).sin();

    // 添加来自表 5 的项
    let two_psi = 2.0 * psi;
    let three_psi = 3.0 * psi;
    inclination_sec_part += 46.997 * psi.cos() * t
        - 0.614 * (two_d - two_f + psi).cos() * t
        + 0.614 * (two_d - two_f - psi).cos() * t
        - 0.0297 * two_psi.cos() * t2
        - 0.0335 * psi.cos() * t2
        + 0.0012 * (two_d - two_f + two_psi).cos() * t2
        - 0.00016 * psi.cos() * t3
        + 0.00004 * three_psi.cos() * t3
        + 0.00004 * two_psi.cos() * t3;

    let perigee_and_mean = 2.116 * psi.sin() * t
        - 0.111 * (two_d - two_f - psi).sin() * t
        - 0.0015 * psi.sin() * t2;
    longitude_of_perigee_sec_part += perigee_and_mean;
    mean_longitude_sec_part += perigee_and_mean;

    longitude_of_node_sec_part += -520.77 * psi.sin() * t
        + 13.66 * (two_d - two_f + psi).sin() * t
        + 1.12 * (two_d - psi).sin() * t
        - 1.06 * (two_f - psi).sin() * t
        + 0.66 * two_psi.sin() * t2
        + 0.371 * psi.sin() * t2
        - 0.035 * (two_d - two_f + two_psi).sin() * t2
        - 0.015 * (two_d - two_f + psi).sin() * t2
        + 0.0014 * psi.sin() * t3
        - 0.0011 * three_psi.sin() * t3
        - 0.0009 * two_psi.sin() * t3;

    // 加上常量并转换单位
    semimajor_axis *= METERS_PER_KILOMETER;
    let inclination = inclination_constant + inclination_sec_part * RADIANS_PER_ARC_SECOND;
    let longitude_of_perigee =
        longitude_of_perigee_constant + longitude_of_perigee_sec_part * RADIANS_PER_ARC_SECOND;
    let mean_longitude =
        mean_longitude_constant + mean_longitude_sec_part * RADIANS_PER_ARC_SECOND;
    let longitude_of_node =
        longitude_of_node_constant + longitude_of_node_sec_part * RADIANS_PER_ARC_SECOND;

    elements_to_cartesian(
        semimajor_axis,
        eccentricity,
        inclination,
        longitude_of_perigee,
        longitude_of_node,
        mean_longitude,
    )
}

const MOON_EARTH_MASS_RATIO: f64 = 0.012300034;
const EARTH_FACTOR: f64 = MOON_EARTH_MASS_RATIO / (MOON_EARTH_MASS_RATIO + 1.0) * -1.0;

/// 计算地球相对地月质心的位置偏移。
///
/// # 参数
/// - `date`：儒略日期（TAI）。
///
/// # 返回
/// 月球位置乘以地月质量比导出的地球因子，即地球绕地月质心的位置。
fn compute_simon_earth(date: &JulianDate) -> DVec3 {
    compute_simon_moon(date) * EARTH_FACTOR
}

/// 从 Simon1994 坐标系到 J2000 的坐标轴变换。
/// CesiumJS 的 Matrix3 构造函数接受 (col0row0, col1row0, col2row0, col0row1, col1row1, col2row1, col0row2, col1row2, col2row2)
/// 但内部以列主序存储。glam 的 from_cols_array 接受 [col0.x, col0.y, col0.z, col1.x, ...]
const AXES_TRANSFORMATION: DMat3 = DMat3::from_cols_array(&[
    1.0000000000000002,      // col0.x (col0row0)
    -5.154129427414611e-16,  // col0.y (col0row1)
    -2.23970096136568e-16,   // col0.z (col0row2)
    5.619723173785822e-16,   // col1.x (col1row0)
    0.9174820620691819,      // col1.y (col1row1)
    0.39777715593191376,     // col1.z (col1row2)
    4.690511510146299e-19,   // col2.x (col2row0)
    -0.39777715593191376,    // col2.y (col2row1)
    0.9174820620691819,      // col2.z (col2row2)
]);

/// 计算太阳在地心惯性系中的位置。
pub fn compute_sun_position_in_earth_inertial_frame(date: &JulianDate) -> DVec3 {
    // 第一次正变换：将地月质心位置取反
    let emb = compute_simon_earth_moon_barycenter(date);
    let mut result = -emb;

    // 第二次正变换：从地月质心中减去地球偏移
    let earth = compute_simon_earth(date);
    result -= earth;

    // 应用坐标轴变换
    AXES_TRANSFORMATION * result
}

/// 计算月球在地心惯性系中的位置。
pub fn compute_moon_position_in_earth_inertial_frame(date: &JulianDate) -> DVec3 {
    let moon = compute_simon_moon(date);
    AXES_TRANSFORMATION * moon
}
