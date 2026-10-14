//! Quaternion fromRotationMatrix / fromHeadingPitchRoll / lerp / slerp / equalsEpsilon 测试。
//! 参考 CesiumJS `Specs/Core/QuaternionSpec` A 类测试（扩展覆盖）。

use cesium_geospatial::math_utils;
use cesium_geospatial::quaternion_ext as qext;
use cesium_geospatial::transforms::HeadingPitchRoll;
use glam::{DMat3, DQuat, DVec3};

const EPSILON14: f64 = math_utils::EPSILON14;
const EPSILON10: f64 = math_utils::EPSILON10;

fn quat_epsilon_eq(a: DQuat, b: DQuat, epsilon: f64) -> bool {
    qext::equals_epsilon(a, b, epsilon)
}

// ---------------------------------------------------------------------------
// fromRotationMatrix
// ---------------------------------------------------------------------------

#[test]
fn from_rotation_matrix_trace_positive() {
    // 单位矩阵：trace = 3 > 0
    let m = DMat3::IDENTITY;
    let q = qext::from_rotation_matrix(&m);
    assert!(quat_epsilon_eq(q, DQuat::IDENTITY, EPSILON14));
}

#[test]
fn from_rotation_matrix_m00_max() {
    // 绕 X 轴 180 度：m00=1、m11=-1、m22=-1，trace=-1
    // m00 为最大对角元
    let m = DMat3::from_cols_array(&[1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, -1.0]);
    let q = qext::from_rotation_matrix(&m);
    // 期望：绕 X 轴旋转 PI => (1, 0, 0, 0) 或 (-1, 0, 0, 0)
    let expected = DQuat::from_axis_angle(DVec3::X, std::f64::consts::PI);
    assert!(quat_epsilon_eq(q, expected, EPSILON10));
}

#[test]
fn from_rotation_matrix_m11_max() {
    // 绕 Y 轴 180 度：m00=-1、m11=1、m22=-1，trace=-1
    let m = DMat3::from_cols_array(&[-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0]);
    let q = qext::from_rotation_matrix(&m);
    let expected = DQuat::from_axis_angle(DVec3::Y, std::f64::consts::PI);
    assert!(quat_epsilon_eq(q, expected, EPSILON10));
}

#[test]
fn from_rotation_matrix_m22_max() {
    // 绕 Z 轴 180 度：m00=-1、m11=-1、m22=1，trace=-1
    let m = DMat3::from_cols_array(&[-1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0]);
    let q = qext::from_rotation_matrix(&m);
    let expected = DQuat::from_axis_angle(DVec3::Z, std::f64::consts::PI);
    assert!(quat_epsilon_eq(q, expected, EPSILON10));
}

#[test]
fn from_rotation_matrix_roundtrip() {
    // 构造四元数，转为矩阵，再转回来
    let original = DQuat::from_axis_angle(DVec3::new(1.0, 2.0, 3.0).normalize(), 1.23);
    let matrix = cesium_geospatial::matrix3_ext::from_quaternion(original);
    let recovered = qext::from_rotation_matrix(&matrix);
    // 四元数可能符号相反
    let dot = original.dot(recovered);
    assert!((dot.abs() - 1.0).abs() < EPSILON10);
}

#[test]
fn from_rotation_matrix_view_matrix() {
    // 典型的视图旋转（相机沿 -Z 方向看，up = Y）
    let angle = std::f64::consts::FRAC_PI_4;
    let m = DMat3::from_rotation_z(angle);
    let q = qext::from_rotation_matrix(&m);
    let expected = DQuat::from_axis_angle(DVec3::Z, angle);
    assert!(quat_epsilon_eq(q, expected, EPSILON10));
}

// ---------------------------------------------------------------------------
// fromHeadingPitchRoll
// ---------------------------------------------------------------------------

#[test]
fn from_heading_pitch_roll_heading_only() {
    let hpr = HeadingPitchRoll::new(std::f64::consts::FRAC_PI_2, 0.0, 0.0);
    let q = hpr.to_quaternion();
    // Heading 是绕 -Z（等价于负 Z 轴）的旋转
    // CesiumJS：heading about negative z
    assert!((q.length() - 1.0).abs() < EPSILON14);
    // 验证它是有效旋转
    let m = cesium_geospatial::matrix3_ext::from_quaternion(q);
    let det = m.determinant();
    assert!((det - 1.0).abs() < EPSILON10);
}

#[test]
fn from_heading_pitch_roll_pitch_only() {
    let hpr = HeadingPitchRoll::new(0.0, std::f64::consts::FRAC_PI_4, 0.0);
    let q = hpr.to_quaternion();
    assert!((q.length() - 1.0).abs() < EPSILON14);
}

#[test]
fn from_heading_pitch_roll_roll_only() {
    let hpr = HeadingPitchRoll::new(0.0, 0.0, std::f64::consts::FRAC_PI_4);
    let q = hpr.to_quaternion();
    assert!((q.length() - 1.0).abs() < EPSILON14);
}

#[test]
fn from_heading_pitch_roll_all_angles() {
    let hpr = HeadingPitchRoll::from_degrees(45.0, 30.0, 15.0);
    let q = hpr.to_quaternion();
    assert!((q.length() - 1.0).abs() < EPSILON14);
    // 回环：转回 HPR
    let hpr2 = HeadingPitchRoll::from_quaternion(q);
    assert!((hpr2.heading - hpr.heading).abs() < EPSILON10);
    assert!((hpr2.pitch - hpr.pitch).abs() < EPSILON10);
    assert!((hpr2.roll - hpr.roll).abs() < EPSILON10);
}

// ---------------------------------------------------------------------------
// lerp
// ---------------------------------------------------------------------------

#[test]
fn lerp_at_endpoints() {
    let start = DQuat::IDENTITY;
    let end = DQuat::from_axis_angle(DVec3::Z, std::f64::consts::FRAC_PI_2);
    let at_0 = qext::quaternion_lerp(start, end, 0.0);
    let at_1 = qext::quaternion_lerp(start, end, 1.0);
    assert!(quat_epsilon_eq(at_0, start, EPSILON14));
    assert!(quat_epsilon_eq(at_1, end, EPSILON14));
}

#[test]
fn lerp_midpoint() {
    let start = DQuat::IDENTITY;
    let end = DQuat::from_axis_angle(DVec3::Z, std::f64::consts::FRAC_PI_2);
    let mid = qext::quaternion_lerp(start, end, 0.5);
    // Lerp 是逐分量的线性插值（不归一化）
    let expected = DQuat::from_xyzw(
        (start.x + end.x) * 0.5,
        (start.y + end.y) * 0.5,
        (start.z + end.z) * 0.5,
        (start.w + end.w) * 0.5,
    );
    assert!(quat_epsilon_eq(mid, expected, EPSILON14));
}

#[test]
fn lerp_extrapolate_forward() {
    let start = DQuat::IDENTITY;
    let end = DQuat::from_axis_angle(DVec3::Z, 0.5);
    let result = qext::quaternion_lerp(start, end, 2.0);
    // t=2：start + 2*(end-start) = 2*end - start
    let expected = DQuat::from_xyzw(
        2.0 * end.x - start.x,
        2.0 * end.y - start.y,
        2.0 * end.z - start.z,
        2.0 * end.w - start.w,
    );
    assert!(quat_epsilon_eq(result, expected, EPSILON14));
}

// ---------------------------------------------------------------------------
// slerp
// ---------------------------------------------------------------------------

#[test]
fn slerp_at_endpoints() {
    let start = DQuat::IDENTITY;
    let end = DQuat::from_axis_angle(DVec3::Z, std::f64::consts::FRAC_PI_2);
    let at_0 = qext::cesium_slerp(start, end, 0.0);
    let at_1 = qext::cesium_slerp(start, end, 1.0);
    assert!(quat_epsilon_eq(at_0, start, EPSILON14));
    assert!(quat_epsilon_eq(at_1, end, EPSILON14));
}

#[test]
fn slerp_midpoint_is_normalized() {
    let start = DQuat::IDENTITY;
    let end = DQuat::from_axis_angle(DVec3::Z, std::f64::consts::FRAC_PI_2);
    let mid = qext::cesium_slerp(start, end, 0.5);
    assert!((mid.length() - 1.0).abs() < EPSILON14);
}

#[test]
fn slerp_obtuse_angles() {
    // dot < 0 时，slerp 应取反其中一个四元数以走短弧
    let start = DQuat::IDENTITY;
    let end = DQuat::from_axis_angle(DVec3::Z, std::f64::consts::PI * 0.75);
    let mid = qext::cesium_slerp(start, end, 0.5);
    assert!((mid.length() - 1.0).abs() < EPSILON14);
    // 中点应位于角度的一半处
    let expected = DQuat::from_axis_angle(DVec3::Z, std::f64::consts::PI * 0.375);
    let dot = mid.dot(expected).abs();
    assert!((dot - 1.0).abs() < EPSILON10);
}

// ---------------------------------------------------------------------------
// equalsEpsilon
// ---------------------------------------------------------------------------

#[test]
fn equals_epsilon_works() {
    let a = DQuat::IDENTITY;
    let b = DQuat::from_xyzw(1e-15, 0.0, 0.0, 1.0);
    assert!(qext::equals_epsilon(a, b, EPSILON14));
    assert!(!qext::equals_epsilon(a, b, 1e-16));
}
