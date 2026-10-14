//! Matrix 与 Quaternion 规格测试 - 参考自 Core/Matrix2Spec、Matrix3Spec、Matrix4Spec、QuaternionSpec
//! 覆盖范围：DMat2/DMat3/DMat4 运算、DQuat 旋转、Transforms 集成。

use cesium_geospatial::transforms::{HeadingPitchRoll, TranslationRotationScale};
use glam::{DMat2, DMat3, DMat4, DQuat, DVec3};
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

const EPS: f64 = 1e-10;

// ─── Matrix2 ────────────────────────────────────────────────────────────────

#[test]
fn mat2_identity() {
    let m = DMat2::IDENTITY;
    assert_eq!(m.x_axis.x, 1.0);
    assert_eq!(m.y_axis.y, 1.0);
    assert_eq!(m.x_axis.y, 0.0);
}

#[test]
fn mat2_determinant() {
    let m = DMat2::from_cols_array(&[1.0, 2.0, 3.0, 4.0]);
    let det = m.determinant();
    assert!((det - (-2.0)).abs() < EPS, "det should be -2, got {det}");
}

#[test]
fn mat2_inverse() {
    let m = DMat2::from_cols_array(&[4.0, 7.0, 2.0, 6.0]);
    let inv = m.inverse();
    let product = m * inv;
    let id = DMat2::IDENTITY;
    assert!((product.x_axis.x - id.x_axis.x).abs() < EPS);
    assert!((product.y_axis.y - id.y_axis.y).abs() < EPS);
}

#[test]
fn mat2_transpose() {
    let m = DMat2::from_cols_array(&[1.0, 2.0, 3.0, 4.0]);
    let t = m.transpose();
    assert_eq!(t.x_axis.y, m.y_axis.x);
    assert_eq!(t.y_axis.x, m.x_axis.y);
}

// ─── Matrix3 ────────────────────────────────────────────────────────────────

#[test]
fn mat3_identity() {
    let m = DMat3::IDENTITY;
    assert_eq!(m.x_axis.x, 1.0);
    assert_eq!(m.y_axis.y, 1.0);
    assert_eq!(m.z_axis.z, 1.0);
}

#[test]
fn mat3_determinant() {
    let m = DMat3::from_cols_array(&[
        1.0, 0.0, 0.0,
        0.0, 2.0, 0.0,
        0.0, 0.0, 3.0,
    ]);
    let det = m.determinant();
    assert!((det - 6.0).abs() < EPS);
}

#[test]
fn mat3_from_rotation_z() {
    let angle = FRAC_PI_4;
    let m = DMat3::from_rotation_z(angle);
    let v = m * DVec3::X;
    assert!((v.x - angle.cos()).abs() < EPS);
    assert!((v.y - angle.sin()).abs() < EPS);
}

#[test]
fn mat3_inverse() {
    let m = DMat3::from_cols_array(&[
        2.0, 0.0, 0.0,
        0.0, 3.0, 0.0,
        0.0, 0.0, 4.0,
    ]);
    let inv = m.inverse();
    let product = m * inv;
    let id = DMat3::IDENTITY;
    assert!((product.x_axis.x - id.x_axis.x).abs() < EPS);
    assert!((product.y_axis.y - id.y_axis.y).abs() < EPS);
    assert!((product.z_axis.z - id.z_axis.z).abs() < EPS);
}

// ─── Matrix4 ────────────────────────────────────────────────────────────────

#[test]
fn mat4_identity() {
    let m = DMat4::IDENTITY;
    assert_eq!(m.w_axis.w, 1.0);
    assert_eq!(m.x_axis.x, 1.0);
}

#[test]
fn mat4_translation() {
    let m = DMat4::from_translation(DVec3::new(1.0, 2.0, 3.0));
    let p = m.transform_point3(DVec3::ZERO);
    assert!((p.x - 1.0).abs() < EPS);
    assert!((p.y - 2.0).abs() < EPS);
    assert!((p.z - 3.0).abs() < EPS);
}

#[test]
fn mat4_scale() {
    let m = DMat4::from_scale(DVec3::new(2.0, 3.0, 4.0));
    let p = m.transform_point3(DVec3::ONE);
    assert!((p.x - 2.0).abs() < EPS);
    assert!((p.y - 3.0).abs() < EPS);
    assert!((p.z - 4.0).abs() < EPS);
}

#[test]
fn mat4_inverse() {
    let m = DMat4::from_translation(DVec3::new(5.0, -3.0, 7.0))
        * DMat4::from_scale(DVec3::splat(2.0));
    let inv = m.inverse();
    let product = m * inv;
    for i in 0..4 {
        for j in 0..4 {
            let expected = if i == j { 1.0 } else { 0.0 };
            assert!((product.col(i)[j] - expected).abs() < 1e-8);
        }
    }
}

#[test]
fn mat4_rotation_x() {
    let m = DMat4::from_rotation_x(FRAC_PI_2);
    let v = m.transform_point3(DVec3::Y);
    assert!((v.y - 0.0).abs() < EPS);
    assert!((v.z - 1.0).abs() < EPS);
}

#[test]
fn mat4_determinant() {
    let m = DMat4::from_scale(DVec3::new(2.0, 3.0, 4.0));
    let det = m.determinant();
    assert!((det - 24.0).abs() < EPS);
}

// ─── Quaternion ─────────────────────────────────────────────────────────────

#[test]
fn quat_identity() {
    let q = DQuat::IDENTITY;
    assert_eq!(q.w, 1.0);
    assert_eq!(q.x, 0.0);
    assert_eq!(q.y, 0.0);
    assert_eq!(q.z, 0.0);
}

#[test]
fn quat_from_axis_angle() {
    let q = DQuat::from_axis_angle(DVec3::Z, FRAC_PI_2);
    let v = q * DVec3::X;
    assert!((v.x - 0.0).abs() < EPS);
    assert!((v.y - 1.0).abs() < EPS);
}

#[test]
fn quat_normalize() {
    let q = DQuat::from_xyzw(1.0, 2.0, 3.0, 4.0);
    let n = q.normalize();
    let len = (n.x * n.x + n.y * n.y + n.z * n.z + n.w * n.w).sqrt();
    assert!((len - 1.0).abs() < EPS);
}

#[test]
fn quat_conjugate() {
    let q = DQuat::from_xyzw(1.0, 2.0, 3.0, 4.0);
    let c = q.conjugate();
    assert_eq!(c.x, -q.x);
    assert_eq!(c.y, -q.y);
    assert_eq!(c.z, -q.z);
    assert_eq!(c.w, q.w);
}

#[test]
fn quat_slerp_endpoints() {
    let a = DQuat::IDENTITY;
    let b = DQuat::from_axis_angle(DVec3::Y, PI);
    let s0 = a.slerp(b, 0.0);
    let s1 = a.slerp(b, 1.0);
    assert!((s0.w - a.w).abs() < EPS);
    assert!((s1.y - b.y).abs() < EPS || (s1.y + b.y).abs() < EPS);
}

#[test]
fn quat_slerp_midpoint() {
    let a = DQuat::IDENTITY;
    let b = DQuat::from_axis_angle(DVec3::Z, FRAC_PI_2);
    let mid = a.slerp(b, 0.5);
    let expected = DQuat::from_axis_angle(DVec3::Z, FRAC_PI_4);
    assert!((mid.w - expected.w).abs() < 1e-8);
    assert!((mid.z - expected.z).abs() < 1e-8);
}

#[test]
fn quat_multiply() {
    let q1 = DQuat::from_axis_angle(DVec3::Z, FRAC_PI_2);
    let q2 = DQuat::from_axis_angle(DVec3::Z, FRAC_PI_2);
    let combined = q1 * q2;
    let v = combined * DVec3::X;
    assert!((v.x - (-1.0)).abs() < EPS, "90+90=180 rotation of X should give -X");
}

// ─── Transforms integration ─────────────────────────────────────────────────

#[test]
fn heading_pitch_roll_default() {
    let hpr = HeadingPitchRoll::new(0.0, 0.0, 0.0);
    assert_eq!(hpr.heading, 0.0);
    assert_eq!(hpr.pitch, 0.0);
    assert_eq!(hpr.roll, 0.0);
}

#[test]
fn heading_pitch_roll_from_degrees() {
    let hpr = HeadingPitchRoll {
        heading: FRAC_PI_4,
        pitch: 0.0,
        roll: 0.0,
    };
    assert!((hpr.heading - FRAC_PI_4).abs() < EPS);
}

#[test]
fn translation_rotation_scale_default() {
    let trs = TranslationRotationScale::new(DVec3::ZERO, DQuat::IDENTITY, DVec3::ONE);
    assert_eq!(trs.translation, DVec3::ZERO);
    assert_eq!(trs.rotation, DQuat::IDENTITY);
    assert_eq!(trs.scale, DVec3::ONE);
}

#[test]
fn translation_rotation_scale_to_matrix() {
    let trs = TranslationRotationScale {
        translation: DVec3::new(1.0, 2.0, 3.0),
        rotation: DQuat::IDENTITY,
        scale: DVec3::splat(2.0),
    };
    let m = trs.to_matrix4();
    let p = m.transform_point3(DVec3::ZERO);
    assert!((p.x - 1.0).abs() < EPS);
    assert!((p.y - 2.0).abs() < EPS);
    assert!((p.z - 3.0).abs() < EPS);
}
