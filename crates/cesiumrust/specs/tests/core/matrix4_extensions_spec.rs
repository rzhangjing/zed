//! Core/Matrix4Spec（CesiumJS 特有扩展）→ Rust 集成测试
//! 覆盖：fromRotationTranslation, fromTranslation, fromScale, fromUniformScale,
//! getTranslation, getScale, getMaximumScale, getRotation, multiplyByTranslation,
//! multiplyByScale, computePerspectiveFieldOfView, pack/unpack, equalsEpsilon

use cesium_geospatial::matrix4_ext;
use glam::{DMat3, DMat4, DVec3};
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

const EPS: f64 = 1e-10;

fn assert_mat4_eq(actual: &DMat4, expected: &DMat4, msg: &str) {
    let a = actual.to_cols_array();
    let e = expected.to_cols_array();
    for i in 0..16 {
        assert!(
            (a[i] - e[i]).abs() < EPS,
            "{msg}: element[{i}] expected {}, got {}",
            e[i],
            a[i]
        );
    }
}

// ─── fromRotationTranslation ────────────────────────────────────────────────

#[test]
fn from_rotation_translation_basic() {
    let rotation = DMat3::IDENTITY;
    let translation = DVec3::new(1.0, 2.0, 3.0);
    let result = matrix4_ext::from_rotation_translation(&rotation, translation);

    let expected = DMat4::from_cols_array(&[
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        1.0, 2.0, 3.0, 1.0,
    ]);
    assert_mat4_eq(&result, &expected, "fromRotationTranslation identity");
}

#[test]
fn from_rotation_translation_with_rotation() {
    // 绕 Z 轴旋转 90°
    let rotation = DMat3::from_rotation_z(FRAC_PI_2);
    let translation = DVec3::new(10.0, 20.0, 30.0);
    let result = matrix4_ext::from_rotation_translation(&rotation, translation);

    // 检查平移列
    assert!((result.w_axis.x - 10.0).abs() < EPS);
    assert!((result.w_axis.y - 20.0).abs() < EPS);
    assert!((result.w_axis.z - 30.0).abs() < EPS);
    assert!((result.w_axis.w - 1.0).abs() < EPS);

    // 检查旋转部分（cos90≈0，sin90≈1）
    assert!(result.x_axis.x.abs() < EPS); // cos(90) ≈ 0
    assert!((result.x_axis.y - 1.0).abs() < EPS); // sin(90) ≈ 1
}

// ─── fromTranslation ────────────────────────────────────────────────────────

#[test]
fn from_translation_basic() {
    let t = DVec3::new(5.0, 6.0, 7.0);
    let result = matrix4_ext::from_translation(t);
    let expected = DMat4::from_translation(t);
    assert_mat4_eq(&result, &expected, "fromTranslation");
}

// ─── fromScale / fromUniformScale ───────────────────────────────────────────

#[test]
fn from_scale_basic() {
    let s = DVec3::new(2.0, 3.0, 4.0);
    let result = matrix4_ext::from_scale(s);
    let expected = DMat4::from_scale(s);
    assert_mat4_eq(&result, &expected, "fromScale");
}

#[test]
fn from_uniform_scale_basic() {
    let result = matrix4_ext::from_uniform_scale(3.0);
    let expected = DMat4::from_scale(DVec3::splat(3.0));
    assert_mat4_eq(&result, &expected, "fromUniformScale");
}

// ─── getTranslation ─────────────────────────────────────────────────────────

#[test]
fn get_translation_basic() {
    let m = DMat4::from_translation(DVec3::new(1.0, 2.0, 3.0));
    let t = matrix4_ext::get_translation(&m);
    assert!((t.x - 1.0).abs() < EPS);
    assert!((t.y - 2.0).abs() < EPS);
    assert!((t.z - 3.0).abs() < EPS);
}

#[test]
fn get_translation_from_composed() {
    let rotation = DMat3::from_rotation_z(FRAC_PI_4);
    let translation = DVec3::new(10.0, 20.0, 30.0);
    let m = matrix4_ext::from_rotation_translation(&rotation, translation);
    let t = matrix4_ext::get_translation(&m);
    assert!((t.x - 10.0).abs() < EPS);
    assert!((t.y - 20.0).abs() < EPS);
    assert!((t.z - 30.0).abs() < EPS);
}

// ─── getScale ───────────────────────────────────────────────────────────────

#[test]
fn get_scale_identity() {
    let m = DMat4::IDENTITY;
    let s = matrix4_ext::get_scale(&m);
    assert!((s.x - 1.0).abs() < EPS);
    assert!((s.y - 1.0).abs() < EPS);
    assert!((s.z - 1.0).abs() < EPS);
}

#[test]
fn get_scale_non_uniform() {
    let m = DMat4::from_scale(DVec3::new(2.0, 3.0, 4.0));
    let s = matrix4_ext::get_scale(&m);
    assert!((s.x - 2.0).abs() < EPS);
    assert!((s.y - 3.0).abs() < EPS);
    assert!((s.z - 4.0).abs() < EPS);
}

#[test]
fn get_scale_with_rotation() {
    let rotation = DMat3::from_rotation_z(FRAC_PI_4);
    let m = matrix4_ext::from_rotation_translation(&rotation, DVec3::ZERO);
    let m = m * DMat4::from_scale(DVec3::new(2.0, 3.0, 4.0));
    let s = matrix4_ext::get_scale(&m);
    assert!((s.x - 2.0).abs() < 1e-9);
    assert!((s.y - 3.0).abs() < 1e-9);
    assert!((s.z - 4.0).abs() < 1e-9);
}

// ─── getMaximumScale ────────────────────────────────────────────────────────

#[test]
fn get_maximum_scale_basic() {
    let m = DMat4::from_scale(DVec3::new(2.0, 5.0, 3.0));
    let max_s = matrix4_ext::get_maximum_scale(&m);
    assert!((max_s - 5.0).abs() < EPS);
}

// ─── getRotation ────────────────────────────────────────────────────────────

#[test]
fn get_rotation_identity() {
    let m = DMat4::IDENTITY;
    let r = matrix4_ext::get_rotation(&m);
    let expected = DMat3::IDENTITY;
    let r_arr = r.to_cols_array();
    let e_arr = expected.to_cols_array();
    for i in 0..9 {
        assert!((r_arr[i] - e_arr[i]).abs() < EPS, "getRotation identity[{i}]");
    }
}

#[test]
fn get_rotation_removes_scale() {
    let rotation = DMat3::from_rotation_z(FRAC_PI_4);
    let m = matrix4_ext::from_rotation_translation(&rotation, DVec3::new(1.0, 2.0, 3.0));
    // 应用缩放
    let scaled = DMat4::from_cols(
        m.x_axis * 2.0,
        m.y_axis * 3.0,
        m.z_axis * 4.0,
        m.w_axis,
    );
    let r = matrix4_ext::get_rotation(&scaled);
    // 应恢复原始旋转
    let r_arr = r.to_cols_array();
    let rot_arr = rotation.to_cols_array();
    for i in 0..9 {
        assert!(
            (r_arr[i] - rot_arr[i]).abs() < 1e-9,
            "getRotation removes scale[{i}]: expected {}, got {}",
            rot_arr[i],
            r_arr[i]
        );
    }
}

// ─── multiplyByTranslation ──────────────────────────────────────────────────

#[test]
fn multiply_by_translation_identity() {
    let m = DMat4::IDENTITY;
    let t = DVec3::new(1.0, 2.0, 3.0);
    let result = matrix4_ext::multiply_by_translation(&m, t);
    let expected = DMat4::from_translation(t);
    assert_mat4_eq(&result, &expected, "multiplyByTranslation identity");
}

#[test]
fn multiply_by_translation_composed() {
    let m = DMat4::from_translation(DVec3::new(10.0, 0.0, 0.0));
    let t = DVec3::new(0.0, 5.0, 0.0);
    let result = matrix4_ext::multiply_by_translation(&m, t);
    // 应等价于 m * fromTranslation(t)
    let expected = m * DMat4::from_translation(t);
    assert_mat4_eq(&result, &expected, "multiplyByTranslation composed");
}

// ─── multiplyByScale ────────────────────────────────────────────────────────

#[test]
fn multiply_by_scale_basic() {
    let m = DMat4::IDENTITY;
    let s = DVec3::new(2.0, 3.0, 4.0);
    let result = matrix4_ext::multiply_by_scale(&m, s);
    let expected = DMat4::from_scale(s);
    assert_mat4_eq(&result, &expected, "multiplyByScale identity");
}

#[test]
fn multiply_by_scale_unity_is_clone() {
    let m = DMat4::from_translation(DVec3::new(1.0, 2.0, 3.0));
    let result = matrix4_ext::multiply_by_scale(&m, DVec3::ONE);
    assert_mat4_eq(&result, &m, "multiplyByScale unity");
}

#[test]
fn multiply_by_scale_composed() {
    let m = DMat4::from_translation(DVec3::new(5.0, 6.0, 7.0));
    let s = DVec3::new(2.0, 3.0, 4.0);
    let result = matrix4_ext::multiply_by_scale(&m, s);
    let expected = m * DMat4::from_scale(s);
    assert_mat4_eq(&result, &expected, "multiplyByScale composed");
}

// ─── computePerspectiveFieldOfView ──────────────────────────────────────────

#[test]
fn compute_perspective_fov_basic() {
    let fov_y = FRAC_PI_2; // 90 度
    let aspect = 1.0;
    let near = 1.0;
    let far = 100.0;
    let result = matrix4_ext::compute_perspective_field_of_view(fov_y, aspect, near, far);

    let bottom = (fov_y * 0.5).tan();
    let col1_row1 = 1.0 / bottom;
    let col0_row0 = col1_row1 / aspect;
    let col2_row2 = (far + near) / (near - far);
    let col3_row2 = (2.0 * far * near) / (near - far);

    let arr = result.to_cols_array();
    assert!((arr[0] - col0_row0).abs() < EPS, "col0Row0");
    assert!((arr[5] - col1_row1).abs() < EPS, "col1Row1");
    assert!((arr[10] - col2_row2).abs() < EPS, "col2Row2");
    assert!((arr[11] - (-1.0)).abs() < EPS, "col2Row3 = -1");
    assert!((arr[14] - col3_row2).abs() < EPS, "col3Row2");
    assert!((arr[15] - 0.0).abs() < EPS, "col3Row3 = 0");
}

// ─── pack / unpack ──────────────────────────────────────────────────────────

#[test]
fn pack_basic() {
    let m = DMat4::from_translation(DVec3::new(1.0, 2.0, 3.0));
    let mut array = vec![0.0; 16];
    matrix4_ext::pack(&m, &mut array, 0);
    let unpacked = matrix4_ext::unpack(&array, 0);
    assert_mat4_eq(&unpacked, &m, "pack roundtrip");
}

#[test]
fn unpack_with_offset() {
    let m = DMat4::from_scale(DVec3::new(2.0, 3.0, 4.0));
    let mut array = vec![0.0; 32];
    matrix4_ext::pack(&m, &mut array, 16);
    let result = matrix4_ext::unpack(&array, 16);
    assert_mat4_eq(&result, &m, "unpack with offset");
}

#[test]
fn pack_unpack_roundtrip() {
    let rotation = DMat3::from_rotation_z(0.7);
    let m = matrix4_ext::from_rotation_translation(&rotation, DVec3::new(-1.0, 2.5, 3.14));
    let mut array = vec![0.0; 16];
    matrix4_ext::pack(&m, &mut array, 0);
    let result = matrix4_ext::unpack(&array, 0);
    assert_mat4_eq(&result, &m, "pack/unpack roundtrip");
}

// ─── equalsEpsilon ──────────────────────────────────────────────────────────

#[test]
fn equals_epsilon_exact() {
    let m = DMat4::from_translation(DVec3::new(1.0, 2.0, 3.0));
    assert!(matrix4_ext::equals_epsilon(&m, &m, 0.0));
}

#[test]
fn equals_epsilon_within() {
    let m1 = DMat4::IDENTITY;
    let m2 = DMat4::from_translation(DVec3::new(1e-12, 0.0, 0.0));
    assert!(matrix4_ext::equals_epsilon(&m1, &m2, 1e-10));
}

#[test]
fn equals_epsilon_outside() {
    let m1 = DMat4::IDENTITY;
    let m2 = DMat4::from_translation(DVec3::new(1.0, 0.0, 0.0));
    assert!(!matrix4_ext::equals_epsilon(&m1, &m2, 0.5));
}

// ─── computeView ────────────────────────────────────────────────────────────

#[test]
fn compute_view_basic() {
    // 从原点沿 -Z 方向观察
    let position = DVec3::ZERO;
    let direction = DVec3::new(0.0, 0.0, -1.0);
    let up = DVec3::Y;
    let view = matrix4_ext::compute_view(position, direction, up);
    // 应近似单位矩阵（right=X，up=Y，-direction=Z）
    let expected = DMat4::from_cols_array(&[
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]);
    assert_mat4_eq(&view, &expected, "compute_view basic");
}

#[test]
fn compute_view_with_translation() {
    let position = DVec3::new(1.0, 2.0, 3.0);
    let direction = DVec3::new(0.0, 0.0, -1.0);
    let up = DVec3::Y;
    let view = matrix4_ext::compute_view(position, direction, up);
    // 平移列应为 -position（因为坐标轴为单位矩阵）
    let t = matrix4_ext::get_translation(&view);
    assert!((t.x - (-1.0)).abs() < EPS);
    assert!((t.y - (-2.0)).abs() < EPS);
    assert!((t.z - (-3.0)).abs() < EPS);
}

// ─── fromTranslationQuaternionRotationScale ─────────────────────────────────

#[test]
fn from_trs_identity() {
    let m = matrix4_ext::from_translation_quaternion_rotation_scale(
        DVec3::new(1.0, 2.0, 3.0),
        glam::DQuat::IDENTITY,
        DVec3::ONE,
    );
    let expected = DMat4::from_translation(DVec3::new(1.0, 2.0, 3.0));
    assert_mat4_eq(&m, &expected, "from_trs identity");
}

#[test]
fn from_trs_with_scale() {
    let m = matrix4_ext::from_translation_quaternion_rotation_scale(
        DVec3::ZERO,
        glam::DQuat::IDENTITY,
        DVec3::new(2.0, 3.0, 4.0),
    );
    let scale = matrix4_ext::get_scale(&m);
    assert!((scale.x - 2.0).abs() < EPS);
    assert!((scale.y - 3.0).abs() < EPS);
    assert!((scale.z - 4.0).abs() < EPS);
}

// ─── multiplyTransformation ─────────────────────────────────────────────────

#[test]
fn multiply_transformation_identity() {
    let t = DMat4::from_translation(DVec3::new(1.0, 2.0, 3.0));
    let result = matrix4_ext::multiply_transformation(&t, &DMat4::IDENTITY);
    assert_mat4_eq(&result, &t, "multiply_transformation identity");
}

#[test]
fn multiply_transformation_composed() {
    let t1 = DMat4::from_translation(DVec3::new(1.0, 0.0, 0.0));
    let t2 = DMat4::from_translation(DVec3::new(0.0, 2.0, 0.0));
    let result = matrix4_ext::multiply_transformation(&t1, &t2);
    let t = matrix4_ext::get_translation(&result);
    assert!((t.x - 1.0).abs() < EPS);
    assert!((t.y - 2.0).abs() < EPS);
    assert!((t.z - 0.0).abs() < EPS);
    // 第 4 行应为 [0,0,0,1]
    assert!((result.x_axis.w).abs() < EPS);
    assert!((result.w_axis.w - 1.0).abs() < EPS);
}

// ─── multiplyByPointAsVector ────────────────────────────────────────────────

#[test]
fn multiply_by_point_as_vector_ignores_translation() {
    let m = DMat4::from_translation(DVec3::new(100.0, 200.0, 300.0));
    let v = DVec3::new(1.0, 0.0, 0.0);
    let result = matrix4_ext::multiply_by_point_as_vector(&m, v);
    // 平移不应影响方向
    assert!((result.x - 1.0).abs() < EPS);
    assert!((result.y).abs() < EPS);
    assert!((result.z).abs() < EPS);
}

// ─── inverseTransformation ──────────────────────────────────────────────────

#[test]
fn inverse_transformation_roundtrip() {
    let rot = DMat3::from_rotation_y(FRAC_PI_4);
    let m = matrix4_ext::from_rotation_translation(&rot, DVec3::new(1.0, 2.0, 3.0));
    // 先验证 glam 自身的求逆可用
    let glam_inv = m.inverse();
    let glam_product = m * glam_inv;
    let gp = glam_product.to_cols_array();
    for i in 0..16 {
        let expected = if i % 5 == 0 { 1.0 } else { 0.0 };
        assert!((gp[i] - expected).abs() < 1e-8,
            "glam M*M^-1 element {i}: got {} expected {}", gp[i], expected);
    }
    // 再验证我们的求逆与 glam 结果一致
    let inv = matrix4_ext::inverse_transformation(&m);
    let ic = inv.to_cols_array();
    let gc = glam_inv.to_cols_array();
    for i in 0..16 {
        assert!((ic[i] - gc[i]).abs() < 1e-8,
            "inverse element {i}: ours={} glam={}", ic[i], gc[i]);
    }
}

#[test]
fn inverse_transformation_translation_only() {
    let m = DMat4::from_translation(DVec3::new(5.0, -3.0, 7.0));
    let inv = matrix4_ext::inverse_transformation(&m);
    let t = matrix4_ext::get_translation(&inv);
    assert!((t.x - (-5.0)).abs() < EPS);
    assert!((t.y - 3.0).abs() < EPS);
    assert!((t.z - (-7.0)).abs() < EPS);
}

// ─── setRotation / setTranslation / setScale ────────────────────────────────

#[test]
fn set_rotation_basic() {
    let m = DMat4::from_translation(DVec3::new(1.0, 2.0, 3.0));
    let rot = DMat3::from_rotation_z(FRAC_PI_2);
    let result = matrix4_ext::set_rotation(&m, &rot);
    // 平移保持不变
    let t = matrix4_ext::get_translation(&result);
    assert!((t.x - 1.0).abs() < EPS);
    // 旋转已设置
    let r = matrix4_ext::get_rotation(&result);
    let expected_r = rot;
    let ra = r.to_cols_array();
    let ea = expected_r.to_cols_array();
    for i in 0..9 {
        assert!((ra[i] - ea[i]).abs() < EPS, "set_rotation element {i}");
    }
}

#[test]
fn set_translation_basic() {
    let m = DMat4::IDENTITY;
    let result = matrix4_ext::set_translation(&m, DVec3::new(10.0, 20.0, 30.0));
    let t = matrix4_ext::get_translation(&result);
    assert!((t.x - 10.0).abs() < EPS);
    assert!((t.y - 20.0).abs() < EPS);
    assert!((t.z - 30.0).abs() < EPS);
}

#[test]
fn set_scale_basic() {
    let m = DMat4::IDENTITY;
    let result = matrix4_ext::set_scale(&m, DVec3::new(2.0, 3.0, 4.0));
    let s = matrix4_ext::get_scale(&result);
    assert!((s.x - 2.0).abs() < EPS);
    assert!((s.y - 3.0).abs() < EPS);
    assert!((s.z - 4.0).abs() < EPS);
}

// ─── computeOrthographicOffCenter ───────────────────────────────────────────

#[test]
fn compute_orthographic_off_center_basic() {
    let m = matrix4_ext::compute_orthographic_off_center(-1.0, 1.0, -1.0, 1.0, 0.0, 10.0);
    // 对于对称的 [-1,1] x [-1,1]，对角线应为 [1, 1, -2/(far-near)]
    let cols = m.to_cols_array();
    assert!((cols[0] - 1.0).abs() < EPS); // col0_row0 = 2/(right-left) = 1
    assert!((cols[5] - 1.0).abs() < EPS); // col1_row1 = 2/(top-bottom) = 1
    assert!((cols[10] - (-0.2)).abs() < EPS); // col2_row2 = -2/(far-near) = -0.2
    assert!((cols[14] - (-1.0)).abs() < EPS); // col3_row2 = -(far+near)/(far-near) = -1
}

// ─── computePerspectiveOffCenter ────────────────────────────────────────────

#[test]
fn compute_perspective_off_center_basic() {
    let m = matrix4_ext::compute_perspective_off_center(-1.0, 1.0, -1.0, 1.0, 1.0, 100.0);
    let cols = m.to_cols_array();
    // col0_row0 = 2*near/(right-left) = 2*1/2 = 1
    assert!((cols[0] - 1.0).abs() < EPS);
    // col1_row1 = 2*near/(top-bottom) = 1
    assert!((cols[5] - 1.0).abs() < EPS);
    // col2_row3 = -1
    assert!((cols[11] - (-1.0)).abs() < EPS);
}

// ─── computeInfinitePerspectiveOffCenter ────────────────────────────────────

#[test]
fn compute_infinite_perspective_off_center_basic() {
    let m = matrix4_ext::compute_infinite_perspective_off_center(-1.0, 1.0, -1.0, 1.0, 1.0);
    let cols = m.to_cols_array();
    assert!((cols[0] - 1.0).abs() < EPS);
    assert!((cols[5] - 1.0).abs() < EPS);
    // col2_row2 = -1（无限远）
    assert!((cols[10] - (-1.0)).abs() < EPS);
    // col3_row2 = -2*near = -2
    assert!((cols[14] - (-2.0)).abs() < EPS);
}

// ─── computeViewportTransformation ──────────────────────────────────────────

#[test]
fn compute_viewport_transformation_basic() {
    let m = matrix4_ext::compute_viewport_transformation(0.0, 0.0, 800.0, 600.0, 0.0, 1.0);
    let cols = m.to_cols_array();
    // col0_row0 = width/2 = 400
    assert!((cols[0] - 400.0).abs() < EPS);
    // col1_row1 = height/2 = 300
    assert!((cols[5] - 300.0).abs() < EPS);
    // col2_row2 = depth/2 = 0.5
    assert!((cols[10] - 0.5).abs() < EPS);
    // col3_row0 = x + width/2 = 400
    assert!((cols[12] - 400.0).abs() < EPS);
    // col3_row1 = y + height/2 = 300
    assert!((cols[13] - 300.0).abs() < EPS);
    // col3_row2 = near + depth/2 = 0.5
    assert!((cols[14] - 0.5).abs() < EPS);
}

// ─── abs ────────────────────────────────────────────────────────────────────

#[test]
fn abs_basic() {
    let m = DMat4::from_cols_array(&[
        -1.0, 2.0, -3.0, 4.0,
        -5.0, 6.0, -7.0, 8.0,
        -9.0, 10.0, -11.0, 12.0,
        -13.0, 14.0, -15.0, 16.0,
    ]);
    let result = matrix4_ext::abs(&m);
    let cols = result.to_cols_array();
    for (i, &v) in cols.iter().enumerate() {
        assert!(v >= 0.0, "abs element {i} should be non-negative, got {v}");
    }
    assert!((cols[0] - 1.0).abs() < EPS);
    assert!((cols[4] - 5.0).abs() < EPS);
}
