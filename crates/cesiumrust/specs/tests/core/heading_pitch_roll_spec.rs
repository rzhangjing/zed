//! 参考自 HeadingPitchRollSpec（15 个 it()，11 个 A 类）
//! + HeadingPitchRangeSpec（4 个 it()，2 个 A 类）
//! + TranslationRotationScaleSpec（3 个 it()，3 个 A 类）
//!
//! 4 个 throws = C 类（Rust 类型系统）。
//! clone/result 参数变体 = C 类（Rust Copy/Clone 惯用法）。
//! 部分测试已在 transform_spec.rs 中（to_quaternion、from_degrees 基础用例）。

use cesium_geospatial::transforms::{HeadingPitchRoll, HeadingPitchRange, TranslationRotationScale};
use glam::{DQuat, DVec3};
use std::f64::consts::PI;

const DEG2RAD: f64 = PI / 180.0;
const EPSILON11: f64 = 1e-11;
const EPSILON6: f64 = 1e-6;
const EPSILON7: f64 = 1e-7;
const EPSILON9: f64 = 1e-9;

// ===== HeadingPitchRoll =====

#[test]
fn hpr_construct_with_default_values() {
    let hpr = HeadingPitchRoll::default();
    assert_eq!(hpr.heading, 0.0);
    assert_eq!(hpr.pitch, 0.0);
    assert_eq!(hpr.roll, 0.0);
}

#[test]
fn hpr_construct_with_all_values() {
    let hpr = HeadingPitchRoll::new(1.0 * DEG2RAD, 2.0 * DEG2RAD, 3.0 * DEG2RAD);
    assert_eq!(hpr.heading, 1.0 * DEG2RAD);
    assert_eq!(hpr.pitch, 2.0 * DEG2RAD);
    assert_eq!(hpr.roll, 3.0 * DEG2RAD);
}

#[test]
fn hpr_conversion_from_quaternion() {
    let testing_tab: [(f64, f64, f64); 9] = [
        (0.0, 0.0, 0.0),
        (90.0 * DEG2RAD, 0.0, 0.0),
        (-90.0 * DEG2RAD, 0.0, 0.0),
        (0.0, 89.0 * DEG2RAD, 0.0),
        (0.0, -89.0 * DEG2RAD, 0.0),
        (0.0, 0.0, 90.0 * DEG2RAD),
        (0.0, 0.0, -90.0 * DEG2RAD),
        (30.0 * DEG2RAD, 30.0 * DEG2RAD, 30.0 * DEG2RAD),
        (-30.0 * DEG2RAD, -30.0 * DEG2RAD, 45.0 * DEG2RAD),
    ];

    for (heading, pitch, roll) in testing_tab {
        let hpr = HeadingPitchRoll::new(heading, pitch, roll);
        let q = hpr.to_quaternion();
        let result = HeadingPitchRoll::from_quaternion(q);
        assert!(
            (heading - result.heading).abs() < EPSILON11,
            "heading mismatch for ({}, {}, {}): got {}",
            heading, pitch, roll, result.heading
        );
        assert!(
            (pitch - result.pitch).abs() < EPSILON11,
            "pitch mismatch for ({}, {}, {}): got {}",
            heading, pitch, roll, result.pitch
        );
        assert!(
            (roll - result.roll).abs() < EPSILON11,
            "roll mismatch for ({}, {}, {}): got {}",
            heading, pitch, roll, result.roll
        );
    }
}

#[test]
fn hpr_quaternion_rounding_error_pitch() {
    let q = DQuat::from_xyzw(
        8.801218199179452e-17,
        -0.7071067801637715,
        -8.801218315071006e-17,
        -0.7071067822093238,
    );
    let result = HeadingPitchRoll::from_quaternion(q);
    assert_eq!(result.pitch, -(PI / 2.0));
}

#[test]
fn hpr_conversion_from_degrees() {
    let testing_tab: [(f64, f64, f64); 9] = [
        (0.0, 0.0, 0.0),
        (90.0, 0.0, 0.0),
        (-90.0, 0.0, 0.0),
        (0.0, 89.0, 0.0),
        (0.0, -89.0, 0.0),
        (0.0, 0.0, 90.0),
        (0.0, 0.0, -90.0),
        (30.0, 30.0, 30.0),
        (-30.0, -30.0, 45.0),
    ];

    for (h_deg, p_deg, r_deg) in testing_tab {
        let result = HeadingPitchRoll::from_degrees(h_deg, p_deg, r_deg);
        assert!(
            (h_deg * DEG2RAD - result.heading).abs() < EPSILON11,
            "heading mismatch for ({}, {}, {})", h_deg, p_deg, r_deg
        );
        assert!(
            (p_deg * DEG2RAD - result.pitch).abs() < EPSILON11,
            "pitch mismatch for ({}, {}, {})", h_deg, p_deg, r_deg
        );
        assert!(
            (r_deg * DEG2RAD - result.roll).abs() < EPSILON11,
            "roll mismatch for ({}, {}, {})", h_deg, p_deg, r_deg
        );
    }
}

#[test]
fn hpr_equals() {
    let hpr = HeadingPitchRoll::new(1.0, 2.0, 3.0);
    assert!(hpr == HeadingPitchRoll::new(1.0, 2.0, 3.0));
    assert!(hpr != HeadingPitchRoll::new(2.0, 2.0, 3.0));
    assert!(hpr != HeadingPitchRoll::new(1.0, 1.0, 3.0));
    assert!(hpr != HeadingPitchRoll::new(1.0, 2.0, 4.0));
}

#[test]
fn hpr_equals_epsilon() {
    let hpr = HeadingPitchRoll::new(1.0, 2.0, 3.0);
    assert!(hpr.equals_epsilon(&HeadingPitchRoll::new(1.0, 2.0, 3.0), 0.0));
    assert!(hpr.equals_epsilon(&HeadingPitchRoll::new(1.0, 2.0, 3.0), 1.0));
    assert!(hpr.equals_epsilon(&HeadingPitchRoll::new(2.0, 2.0, 3.0), 1.0));
    assert!(hpr.equals_epsilon(&HeadingPitchRoll::new(1.0, 3.0, 3.0), 1.0));
    assert!(hpr.equals_epsilon(&HeadingPitchRoll::new(1.0, 2.0, 4.0), 1.0));
    assert!(!hpr.equals_epsilon(&HeadingPitchRoll::new(2.0, 2.0, 3.0), EPSILON6));
    assert!(!hpr.equals_epsilon(&HeadingPitchRoll::new(1.0, 3.0, 3.0), EPSILON6));
    assert!(!hpr.equals_epsilon(&HeadingPitchRoll::new(1.0, 2.0, 4.0), EPSILON6));

    let hpr2 = HeadingPitchRoll::new(3000000.0, 4000000.0, 5000000.0);
    assert!(hpr2.equals_epsilon(&HeadingPitchRoll::new(3000000.0, 4000000.0, 5000000.0), 0.0));
    assert!(hpr2.equals_epsilon(&HeadingPitchRoll::new(3000000.2, 4000000.0, 5000000.0), EPSILON7));
    assert!(hpr2.equals_epsilon(&HeadingPitchRoll::new(3000000.0, 4000000.2, 5000000.0), EPSILON7));
    assert!(hpr2.equals_epsilon(&HeadingPitchRoll::new(3000000.0, 4000000.0, 5000000.2), EPSILON7));
    assert!(hpr2.equals_epsilon(&HeadingPitchRoll::new(3000000.2, 4000000.2, 5000000.2), EPSILON7));
    assert!(!hpr2.equals_epsilon(&HeadingPitchRoll::new(3000000.2, 4000000.2, 5000000.2), EPSILON9));
}

#[test]
fn hpr_to_string() {
    let hpr = HeadingPitchRoll::new(1.123, 2.345, 6.789);
    assert_eq!(format!("{}", hpr), "(1.123, 2.345, 6.789)");
}

// ===== HeadingPitchRange =====

#[test]
fn hpr_range_construct_with_default_values() {
    let hpr = HeadingPitchRange::default();
    assert_eq!(hpr.heading, 0.0);
    assert_eq!(hpr.pitch, 0.0);
    assert_eq!(hpr.range, 0.0);
}

#[test]
fn hpr_range_construct_with_all_values() {
    let hpr = HeadingPitchRange::new(1.0, 2.0, 3.0);
    assert_eq!(hpr.heading, 1.0);
    assert_eq!(hpr.pitch, 2.0);
    assert_eq!(hpr.range, 3.0);
}

// ===== TranslationRotationScale =====

#[test]
fn trs_default_values() {
    let trs = TranslationRotationScale::default();
    assert_eq!(trs.translation, DVec3::ZERO);
    assert_eq!(trs.rotation, DQuat::IDENTITY);
    assert_eq!(trs.scale, DVec3::ONE);
}

#[test]
fn trs_construct_with_arguments() {
    let translation = DVec3::Y;
    let rotation = DQuat::from_xyzw(0.5, 0.5, 0.5, 0.5);
    let scale = DVec3::X;
    let trs = TranslationRotationScale::new(translation, rotation, scale);
    assert_eq!(trs.translation, translation);
    assert_eq!(trs.rotation, rotation);
    assert_eq!(trs.scale, scale);
}

#[test]
fn trs_equals() {
    let left = TranslationRotationScale::new(
        DVec3::Y,
        DQuat::from_xyzw(0.5, 0.5, 0.5, 0.5),
        DVec3::X,
    );
    let right = TranslationRotationScale::new(
        DVec3::Y,
        DQuat::from_xyzw(0.5, 0.5, 0.5, 0.5),
        DVec3::X,
    );
    assert_eq!(left, right);

    // 不同的 scale
    let right2 = TranslationRotationScale::new(
        DVec3::Y,
        DQuat::from_xyzw(0.5, 0.5, 0.5, 0.5),
        DVec3::ZERO,
    );
    assert_ne!(left, right2);

    // 不同的平移
    let right3 = TranslationRotationScale::new(
        DVec3::ZERO,
        DQuat::from_xyzw(0.5, 0.5, 0.5, 0.5),
        DVec3::X,
    );
    assert_ne!(left, right3);

    // 不同的旋转
    let right4 = TranslationRotationScale::new(
        DVec3::Y,
        DQuat::from_xyzw(0.0, 0.0, 0.0, 0.0),
        DVec3::X,
    );
    assert_ne!(left, right4);
}
