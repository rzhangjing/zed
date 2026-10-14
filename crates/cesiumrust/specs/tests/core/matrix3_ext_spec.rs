//! Matrix3 扩展函数测试。
//! 参考 CesiumJS `Specs/Core/Matrix3Spec` + `Matrix2Spec` A 类测试。

use cesium_geospatial::matrix3_ext as m3;
use cesium_geospatial::math_utils;
use glam::{DMat3, DQuat, DVec3};

const EPSILON14: f64 = math_utils::EPSILON14;

#[test]
fn pack_and_unpack() {
    let m = DMat3::from_cols_array(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let mut array = vec![0.0; 11];
    m3::pack(&m, &mut array, 1);
    assert_eq!(&array[1..10], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);

    let unpacked = m3::unpack(&array, 1);
    assert_eq!(unpacked, m);
}

#[test]
fn from_column_major_array() {
    let array = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
    let m = m3::from_column_major_array(&array, 0);
    // 列主序：col0=(1,2,3)，col1=(4,5,6)，col2=(7,8,9)
    assert_eq!(m.x_axis, DVec3::new(1.0, 2.0, 3.0));
    assert_eq!(m.y_axis, DVec3::new(4.0, 5.0, 6.0));
    assert_eq!(m.z_axis, DVec3::new(7.0, 8.0, 9.0));
}

#[test]
fn from_row_major_array() {
    // 行主序：row0=(1,2,3)，row1=(4,5,6)，row2=(7,8,9)
    let array = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
    let m = m3::from_row_major_array(&array);
    // 列主序：col0=(1,4,7)，col1=(2,5,8)，col2=(3,6,9)
    assert_eq!(m.x_axis, DVec3::new(1.0, 4.0, 7.0));
    assert_eq!(m.y_axis, DVec3::new(2.0, 5.0, 8.0));
    assert_eq!(m.z_axis, DVec3::new(3.0, 6.0, 9.0));
}

#[test]
fn from_quaternion_identity() {
    let q = DQuat::IDENTITY;
    let m = m3::from_quaternion(q);
    assert!(m3::equals_epsilon(&m, &DMat3::IDENTITY, EPSILON14));
}

#[test]
fn from_quaternion_rotation_z_90() {
    // 绕 Z 轴 90 度
    let angle = std::f64::consts::FRAC_PI_2;
    let q = DQuat::from_axis_angle(DVec3::Z, angle);
    let m = m3::from_quaternion(q);

    // 期望：[0, -1, 0; 1, 0, 0; 0, 0, 1]（列主序）
    // col0 = (cos, sin, 0) = (0, 1, 0)
    // col1 = (-sin, cos, 0) = (-1, 0, 0)
    // col2 = (0, 0, 1)
    assert!((m.x_axis.x - 0.0).abs() < EPSILON14);
    assert!((m.x_axis.y - 1.0).abs() < EPSILON14);
    assert!((m.y_axis.x - (-1.0)).abs() < EPSILON14);
    assert!((m.y_axis.y - 0.0).abs() < EPSILON14);
    assert!((m.z_axis.z - 1.0).abs() < EPSILON14);
}

#[test]
fn from_rotation_x() {
    let angle = std::f64::consts::FRAC_PI_2;
    let m = m3::from_rotation_x(angle);
    // col0 = (1, 0, 0)
    // col1 = (0, cos, sin) = (0, 0, 1)
    // col2 = (0, -sin, cos) = (0, -1, 0)
    assert!((m.x_axis.x - 1.0).abs() < EPSILON14);
    assert!((m.y_axis.y - 0.0).abs() < EPSILON14);
    assert!((m.y_axis.z - 1.0).abs() < EPSILON14);
    assert!((m.z_axis.y - (-1.0)).abs() < EPSILON14);
    assert!((m.z_axis.z - 0.0).abs() < EPSILON14);
}

#[test]
fn from_rotation_y() {
    let angle = std::f64::consts::FRAC_PI_2;
    let m = m3::from_rotation_y(angle);
    // col0 = (cos, 0, -sin) = (0, 0, -1)
    // col1 = (0, 1, 0)
    // col2 = (sin, 0, cos) = (1, 0, 0)
    assert!((m.x_axis.x - 0.0).abs() < EPSILON14);
    assert!((m.x_axis.z - (-1.0)).abs() < EPSILON14);
    assert!((m.y_axis.y - 1.0).abs() < EPSILON14);
    assert!((m.z_axis.x - 1.0).abs() < EPSILON14);
    assert!((m.z_axis.z - 0.0).abs() < EPSILON14);
}

#[test]
fn from_rotation_z() {
    let angle = std::f64::consts::FRAC_PI_2;
    let m = m3::from_rotation_z(angle);
    // col0 = (cos, sin, 0) = (0, 1, 0)
    // col1 = (-sin, cos, 0) = (-1, 0, 0)
    // col2 = (0, 0, 1)
    assert!((m.x_axis.x - 0.0).abs() < EPSILON14);
    assert!((m.x_axis.y - 1.0).abs() < EPSILON14);
    assert!((m.y_axis.x - (-1.0)).abs() < EPSILON14);
    assert!((m.y_axis.y - 0.0).abs() < EPSILON14);
    assert!((m.z_axis.z - 1.0).abs() < EPSILON14);
}

#[test]
fn from_scale_works() {
    let m = m3::from_scale(DVec3::new(2.0, 3.0, 4.0));
    assert_eq!(m.x_axis, DVec3::new(2.0, 0.0, 0.0));
    assert_eq!(m.y_axis, DVec3::new(0.0, 3.0, 0.0));
    assert_eq!(m.z_axis, DVec3::new(0.0, 0.0, 4.0));
}

#[test]
fn from_uniform_scale_works() {
    let m = m3::from_uniform_scale(2.0);
    assert_eq!(m, DMat3::from_cols_array(&[2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0]));
}

#[test]
fn get_column_works() {
    let m = DMat3::from_cols_array(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    assert_eq!(m3::get_column(&m, 0), DVec3::new(1.0, 2.0, 3.0));
    assert_eq!(m3::get_column(&m, 1), DVec3::new(4.0, 5.0, 6.0));
    assert_eq!(m3::get_column(&m, 2), DVec3::new(7.0, 8.0, 9.0));
}

#[test]
fn get_row_works() {
    let m = DMat3::from_cols_array(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    assert_eq!(m3::get_row(&m, 0), DVec3::new(1.0, 4.0, 7.0));
    assert_eq!(m3::get_row(&m, 1), DVec3::new(2.0, 5.0, 8.0));
    assert_eq!(m3::get_row(&m, 2), DVec3::new(3.0, 6.0, 9.0));
}

#[test]
fn get_scale_works() {
    let m = m3::from_scale(DVec3::new(2.0, 3.0, 4.0));
    let scale = m3::get_scale(&m);
    assert!((scale.x - 2.0).abs() < EPSILON14);
    assert!((scale.y - 3.0).abs() < EPSILON14);
    assert!((scale.z - 4.0).abs() < EPSILON14);
}

#[test]
fn get_maximum_scale_works() {
    let m = m3::from_scale(DVec3::new(2.0, 3.0, 4.0));
    assert!((m3::get_maximum_scale(&m) - 4.0).abs() < EPSILON14);
}

#[test]
fn get_rotation_works() {
    // 构造一个含旋转 + 缩放的矩阵
    let rotation = m3::from_rotation_z(std::f64::consts::FRAC_PI_4);
    let scaled = m3::set_rotation(&m3::from_scale(DVec3::new(2.0, 3.0, 4.0)), &rotation);
    let extracted = m3::get_rotation(&scaled);
    assert!(m3::equals_epsilon(&extracted, &rotation, EPSILON14));
}

#[test]
fn set_rotation_preserves_scale() {
    let original = m3::from_scale(DVec3::new(2.0, 3.0, 4.0));
    let rotation = m3::from_rotation_z(std::f64::consts::FRAC_PI_4);
    let result = m3::set_rotation(&original, &rotation);

    // 缩放应被保留
    let scale = m3::get_scale(&result);
    assert!((scale.x - 2.0).abs() < EPSILON14);
    assert!((scale.y - 3.0).abs() < EPSILON14);
    assert!((scale.z - 4.0).abs() < EPSILON14);
}

#[test]
fn abs_works() {
    let m = DMat3::from_cols_array(&[-1.0, 2.0, -3.0, 4.0, -5.0, 6.0, -7.0, 8.0, -9.0]);
    let result = m3::abs(&m);
    let expected = DMat3::from_cols_array(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    assert_eq!(result, expected);
}

#[test]
fn equals_epsilon_works() {
    let m1 = DMat3::IDENTITY;
    let m2 = DMat3::IDENTITY;
    assert!(m3::equals_epsilon(&m1, &m2, 0.0));

    let m3_val = DMat3::from_cols_array(&[
        1.0 + 1e-15, 0.0, 0.0,
        0.0, 1.0, 0.0,
        0.0, 0.0, 1.0,
    ]);
    assert!(m3::equals_epsilon(&m1, &m3_val, EPSILON14));
    assert!(!m3::equals_epsilon(&m1, &m3_val, 1e-16));
}

#[test]
fn matrix2_from_rotation_works() {
    let angle = std::f64::consts::FRAC_PI_4;
    let m = m3::matrix2_from_rotation(angle);
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    // 列主序：[cos, sin, -sin, cos]
    assert!((m[0] - cos_a).abs() < EPSILON14);
    assert!((m[1] - sin_a).abs() < EPSILON14);
    assert!((m[2] - (-sin_a)).abs() < EPSILON14);
    assert!((m[3] - cos_a).abs() < EPSILON14);
}

#[test]
fn matrix2_from_scale_works() {
    let m = m3::matrix2_from_scale(2.0);
    assert_eq!(m, [2.0, 0.0, 0.0, 2.0]);
}

#[test]
fn matrix2_pack_unpack() {
    let m = [1.0, 2.0, 3.0, 4.0];
    let mut array = vec![0.0; 6];
    m3::matrix2_pack(&m, &mut array, 1);
    assert_eq!(&array[1..5], &[1.0, 2.0, 3.0, 4.0]);

    let unpacked = m3::matrix2_unpack(&array, 1);
    assert_eq!(unpacked, m);
}

#[test]
fn add_and_subtract() {
    let a = DMat3::from_cols_array(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let b = DMat3::from_cols_array(&[9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0]);
    let sum = m3::add(&a, &b);
    assert_eq!(
        sum.to_cols_array(),
        [10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0]
    );

    let diff = m3::subtract(&a, &b);
    assert_eq!(
        diff.to_cols_array(),
        [-8.0, -6.0, -4.0, -2.0, 0.0, 2.0, 4.0, 6.0, 8.0]
    );
}
