//! Quaternion 扩展函数的测试 - 参考自 QuaternionSpec
//!
//! 原始：124 个 it() → 12 个 A 类（CesiumJS 特有：computeAxis/computeAngle/log/exp/squad/fastSlerp/fastSquad）
//! B 类（glam 代理：fromAxisAngle/slerp/dot/multiply/conjugate/normalize/inverse 等）已被覆盖。

use cesium_geospatial::math_utils;
use cesium_geospatial::quaternion_ext::*;
use glam::{DQuat, DVec3};
use std::f64::consts::{FRAC_PI_4, PI};

fn quat_epsilon_eq(a: DQuat, b: DQuat, epsilon: f64) -> bool {
    (a.x - b.x).abs() <= epsilon
        && (a.y - b.y).abs() <= epsilon
        && (a.z - b.z).abs() <= epsilon
        && (a.w - b.w).abs() <= epsilon
}

fn vec3_epsilon_eq(a: DVec3, b: DVec3, epsilon: f64) -> bool {
    (a.x - b.x).abs() <= epsilon
        && (a.y - b.y).abs() <= epsilon
        && (a.z - b.z).abs() <= epsilon
}

fn from_axis_angle(axis: DVec3, angle: f64) -> DQuat {
    let half = angle / 2.0;
    let s = half.sin();
    let axis = axis.normalize();
    DQuat::from_xyzw(axis.x * s, axis.y * s, axis.z * s, half.cos())
}

// === computeAxis ===

#[test]
fn test_compute_axis_works() {
    // 使用 60 度以确保半角的 sin/cos 不相等
    let angle = PI / 3.0;
    let cos = (angle / 2.0).cos();
    let sin = (angle / 2.0).sin();
    let expected = DVec3::new(2.0, 3.0, 6.0).normalize();
    let quaternion = DQuat::from_xyzw(
        sin * expected.x,
        sin * expected.y,
        sin * expected.z,
        cos,
    );
    let result = compute_axis(quaternion);
    assert!(
        vec3_epsilon_eq(result, expected, math_utils::EPSILON15),
        "compute_axis: got {:?}, expected {:?}",
        result,
        expected
    );
}

#[test]
fn test_compute_axis_w_equals_1() {
    let expected = DVec3::new(1.0, 0.0, 0.0);
    let quaternion = DQuat::from_xyzw(4.0, 2.0, 3.0, 1.0);
    let result = compute_axis(quaternion);
    assert_eq!(result, expected);
}

#[test]
fn test_compute_axis_w_equals_neg_1() {
    let expected = DVec3::new(1.0, 0.0, 0.0);
    let quaternion = DQuat::from_xyzw(4.0, 2.0, 3.0, -1.0);
    let result = compute_axis(quaternion);
    assert_eq!(result, expected);
}

// === computeAngle ===

#[test]
fn test_compute_angle_works() {
    // 使用 60 度以确保半角的 sin/cos 不相等
    let angle = PI / 3.0;
    let cos = (angle / 2.0).cos();
    let sin = (angle / 2.0).sin();
    let axis = DVec3::new(2.0, 3.0, 6.0).normalize();
    let quaternion = DQuat::from_xyzw(sin * axis.x, sin * axis.y, sin * axis.z, cos);
    let result = compute_angle(quaternion);
    assert!(
        (result - angle).abs() <= math_utils::EPSILON15,
        "compute_angle: got {}, expected {}",
        result,
        angle
    );
}

// === log ===

#[test]
fn test_quaternion_log_works() {
    let axis = DVec3::new(1.0, -1.0, 1.0).normalize();
    let angle = FRAC_PI_4;
    let quat = from_axis_angle(axis, angle);

    let log = quaternion_log(quat);
    let expected = axis * (angle * 0.5);
    assert!(
        vec3_epsilon_eq(log, expected, math_utils::EPSILON15),
        "log: got {:?}, expected {:?}",
        log,
        expected
    );
}

// === exp ===

#[test]
fn test_quaternion_exp_works() {
    let axis = DVec3::new(1.0, -1.0, 1.0).normalize();
    let angle = FRAC_PI_4;
    let cartesian = axis * (angle * 0.5);

    let exp = quaternion_exp(cartesian);
    let expected = from_axis_angle(axis, angle);
    assert!(
        quat_epsilon_eq(exp, expected, math_utils::EPSILON15),
        "exp: got {:?}, expected {:?}",
        exp,
        expected
    );
}

// === squad and computeInnerQuadrangle ===

#[test]
fn test_squad_and_compute_inner_quadrangle() {
    let q0 = from_axis_angle(DVec3::X, 0.0);
    let q1 = from_axis_angle(DVec3::X, FRAC_PI_4);
    let q2 = from_axis_angle(DVec3::Z, FRAC_PI_4);
    let q3 = from_axis_angle(DVec3::X, -FRAC_PI_4);

    let s1 = compute_inner_quadrangle(q0, q1, q2);
    let s2 = compute_inner_quadrangle(q1, q2, q3);

    let result = squad(q1, q2, s1, s2, 0.0);
    assert!(
        quat_epsilon_eq(result, q1, math_utils::EPSILON15),
        "squad at t=0 should equal q1: got {:?}, expected {:?}",
        result,
        q1
    );
}

// === fastSlerp ===

#[test]
fn test_fast_slerp_works() {
    let start = DQuat::from_xyzw(0.0, 0.0, 0.0, 1.0).normalize();
    let end = DQuat::from_xyzw(0.0, 0.0, FRAC_PI_4.sin(), FRAC_PI_4.cos());
    let expected = DQuat::from_xyzw(0.0, 0.0, (PI / 8.0).sin(), (PI / 8.0).cos());

    let result = fast_slerp(start, end, 0.5);
    assert!(
        quat_epsilon_eq(result, expected, math_utils::EPSILON6),
        "fastSlerp: got {:?}, expected {:?}",
        result,
        expected
    );
}

#[test]
fn test_fast_slerp_obtuse_angles() {
    let start = DQuat::from_xyzw(0.0, 0.0, 0.0, -1.0).normalize();
    let end = DQuat::from_xyzw(0.0, 0.0, FRAC_PI_4.sin(), FRAC_PI_4.cos());
    let expected = DQuat::from_xyzw(0.0, 0.0, -(PI / 8.0).sin(), -(PI / 8.0).cos());

    let result = fast_slerp(start, end, 0.5);
    assert!(
        quat_epsilon_eq(result, expected, math_utils::EPSILON6),
        "fastSlerp obtuse: got {:?}, expected {:?}",
        result,
        expected
    );
}

#[test]
fn test_fast_slerp_vs_slerp() {
    let start = DQuat::from_xyzw(0.0, 0.0, 0.0, 1.0).normalize();
    let end = DQuat::from_xyzw(0.0, 0.0, FRAC_PI_4.sin(), FRAC_PI_4.cos());

    for &t in &[0.25, 0.5, 0.75] {
        let expected = cesium_slerp(start, end, t);
        let actual = fast_slerp(start, end, t);
        assert!(
            quat_epsilon_eq(actual, expected, math_utils::EPSILON6),
            "fastSlerp vs slerp at t={}: got {:?}, expected {:?}",
            t,
            actual,
            expected
        );
    }
}

// === fastSquad ===

#[test]
fn test_fast_squad_works() {
    let q0 = from_axis_angle(DVec3::X, 0.0);
    let q1 = from_axis_angle(DVec3::X, FRAC_PI_4);
    let q2 = from_axis_angle(DVec3::Z, FRAC_PI_4);
    let q3 = from_axis_angle(DVec3::X, -FRAC_PI_4);

    let s1 = compute_inner_quadrangle(q0, q1, q2);
    let s2 = compute_inner_quadrangle(q1, q2, q3);

    let result = fast_squad(q1, q2, s1, s2, 0.0);
    assert!(
        quat_epsilon_eq(result, q1, math_utils::EPSILON6),
        "fastSquad at t=0 should equal q1: got {:?}, expected {:?}",
        result,
        q1
    );
}

#[test]
fn test_fast_squad_vs_squad() {
    let q0 = from_axis_angle(DVec3::X, 0.0);
    let q1 = from_axis_angle(DVec3::X, FRAC_PI_4);
    let q2 = from_axis_angle(DVec3::Z, FRAC_PI_4);
    let q3 = from_axis_angle(DVec3::X, -FRAC_PI_4);

    let s1 = compute_inner_quadrangle(q0, q1, q2);
    let s2 = compute_inner_quadrangle(q1, q2, q3);

    for &t in &[0.25, 0.5, 0.75] {
        let actual = fast_squad(q1, q2, s1, s2, t);
        let expected = squad(q1, q2, s1, s2, t);
        assert!(
            quat_epsilon_eq(actual, expected, math_utils::EPSILON6),
            "fastSquad vs squad at t={}: got {:?}, expected {:?}",
            t,
            actual,
            expected
        );
    }
}
