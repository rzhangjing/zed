//! Matrix2 扩展函数的测试。
//! 参考 CesiumJS `Specs/Core/Matrix2Spec` 的 A 类测试。

use cesium_geospatial::matrix2_ext as m2;
use cesium_geospatial::math_utils;
use glam::DVec2;

const EPSILON14: f64 = math_utils::EPSILON14;

#[test]
fn from_column_major_array_works() {
    // 列主序：col0=(1,2)，col1=(3,4)
    let values = [1.0, 2.0, 3.0, 4.0];
    let m = m2::from_column_major_array(&values);
    assert_eq!(m, [1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn from_row_major_array_works() {
    // 行主序：row0=(1,2)，row1=(3,4)
    // 列主序结果：col0=(1,3)，col1=(2,4)
    let values = [1.0, 2.0, 3.0, 4.0];
    let m = m2::from_row_major_array(&values);
    assert_eq!(m, [1.0, 3.0, 2.0, 4.0]);
}

#[test]
fn from_scale_works() {
    let m = m2::from_scale(DVec2::new(2.0, 3.0));
    assert_eq!(m, [2.0, 0.0, 0.0, 3.0]);
}

#[test]
fn from_uniform_scale_works() {
    let m = m2::from_uniform_scale(2.0);
    assert_eq!(m, [2.0, 0.0, 0.0, 2.0]);
}

#[test]
fn from_rotation_works() {
    let angle = std::f64::consts::FRAC_PI_2;
    let m = m2::from_rotation(angle);
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    // 列主序：[cos, sin, -sin, cos]
    assert!((m[0] - cos_a).abs() < EPSILON14);
    assert!((m[1] - sin_a).abs() < EPSILON14);
    assert!((m[2] - (-sin_a)).abs() < EPSILON14);
    assert!((m[3] - cos_a).abs() < EPSILON14);
}

#[test]
fn pack_and_unpack() {
    let m = [1.0, 2.0, 3.0, 4.0];
    let mut array = vec![0.0; 6];
    m2::pack(&m, &mut array, 1);
    assert_eq!(&array[1..5], &[1.0, 2.0, 3.0, 4.0]);

    let unpacked = m2::unpack(&array, 1);
    assert_eq!(unpacked, m);
}

#[test]
fn pack_array_and_unpack_array() {
    let matrices = vec![[1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0]];
    let mut packed = Vec::new();
    m2::pack_array(&matrices, &mut packed);
    assert_eq!(packed.len(), 8);

    let unpacked = m2::unpack_array(&packed);
    assert_eq!(unpacked, matrices);
}

#[test]
fn get_element_index_works() {
    // column * 2 + row
    assert_eq!(m2::get_element_index(0, 0), 0);
    assert_eq!(m2::get_element_index(0, 1), 1);
    assert_eq!(m2::get_element_index(1, 0), 2);
    assert_eq!(m2::get_element_index(1, 1), 3);
}

#[test]
fn get_column_works() {
    let m = [1.0, 2.0, 3.0, 4.0];
    assert_eq!(m2::get_column(&m, 0), DVec2::new(1.0, 2.0));
    assert_eq!(m2::get_column(&m, 1), DVec2::new(3.0, 4.0));
}

#[test]
fn set_column_works() {
    let m = [1.0, 2.0, 3.0, 4.0];
    let result = m2::set_column(&m, 0, DVec2::new(5.0, 6.0));
    assert_eq!(result, [5.0, 6.0, 3.0, 4.0]);
}

#[test]
fn get_row_works() {
    let m = [1.0, 2.0, 3.0, 4.0];
    // 第 0 行：位于索引 0 和 2 的元素
    assert_eq!(m2::get_row(&m, 0), DVec2::new(1.0, 3.0));
    // 第 1 行：位于索引 1 和 3 的元素
    assert_eq!(m2::get_row(&m, 1), DVec2::new(2.0, 4.0));
}

#[test]
fn set_row_works() {
    let m = [1.0, 2.0, 3.0, 4.0];
    let result = m2::set_row(&m, 0, DVec2::new(5.0, 6.0));
    assert_eq!(result, [5.0, 2.0, 6.0, 4.0]);
}

#[test]
fn get_scale_works() {
    let m = m2::from_scale(DVec2::new(2.0, 3.0));
    let scale = m2::get_scale(&m);
    assert!((scale.x - 2.0).abs() < EPSILON14);
    assert!((scale.y - 3.0).abs() < EPSILON14);
}

#[test]
fn get_maximum_scale_works() {
    let m = m2::from_scale(DVec2::new(2.0, 3.0));
    assert!((m2::get_maximum_scale(&m) - 3.0).abs() < EPSILON14);
}

#[test]
fn set_scale_preserves_rotation() {
    let rotation = m2::from_rotation(std::f64::consts::FRAC_PI_4);
    let scaled = m2::set_scale(&rotation, DVec2::new(2.0, 3.0));
    let extracted_scale = m2::get_scale(&scaled);
    assert!((extracted_scale.x - 2.0).abs() < EPSILON14);
    assert!((extracted_scale.y - 3.0).abs() < EPSILON14);
}

#[test]
fn get_rotation_removes_scale() {
    let rotation = m2::from_rotation(std::f64::consts::FRAC_PI_4);
    let scaled = m2::set_scale(&rotation, DVec2::new(2.0, 3.0));
    let extracted = m2::get_rotation(&scaled);
    assert!(m2::equals_epsilon(&extracted, &rotation, EPSILON14));
}

#[test]
fn set_rotation_preserves_scale() {
    let original = m2::from_scale(DVec2::new(2.0, 3.0));
    let rotation = m2::from_rotation(std::f64::consts::FRAC_PI_4);
    let result = m2::set_rotation(&original, &rotation);
    let scale = m2::get_scale(&result);
    assert!((scale.x - 2.0).abs() < EPSILON14);
    assert!((scale.y - 3.0).abs() < EPSILON14);
}

#[test]
fn multiply_works() {
    let a = [1.0, 2.0, 3.0, 4.0];
    let b = [5.0, 6.0, 7.0, 8.0];
    let result = m2::multiply(&a, &b);
    // col0: a * b_col0 = (1*5+3*6, 2*5+4*6) = (23, 34)
    // col1: a * b_col1 = (1*7+3*8, 2*7+4*8) = (31, 46)
    assert_eq!(result, [23.0, 34.0, 31.0, 46.0]);
}

#[test]
fn multiply_by_vector_works() {
    let m = [1.0, 2.0, 3.0, 4.0];
    let v = DVec2::new(5.0, 6.0);
    let result = m2::multiply_by_vector(&m, v);
    // (1*5+3*6, 2*5+4*6) = (23, 34)
    assert_eq!(result, DVec2::new(23.0, 34.0));
}

#[test]
fn multiply_by_scale_works() {
    let m = [1.0, 2.0, 3.0, 4.0];
    let result = m2::multiply_by_scale(&m, DVec2::new(2.0, 3.0));
    assert_eq!(result, [2.0, 4.0, 9.0, 12.0]);
}

#[test]
fn transpose_works() {
    let m = [1.0, 2.0, 3.0, 4.0];
    let result = m2::transpose(&m);
    assert_eq!(result, [1.0, 3.0, 2.0, 4.0]);
}

#[test]
fn abs_works() {
    let m = [-1.0, 2.0, -3.0, 4.0];
    let result = m2::abs(&m);
    assert_eq!(result, [1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn negate_works() {
    let m = [1.0, -2.0, 3.0, -4.0];
    let result = m2::negate(&m);
    assert_eq!(result, [-1.0, 2.0, -3.0, 4.0]);
}

#[test]
fn add_and_subtract() {
    let a = [1.0, 2.0, 3.0, 4.0];
    let b = [5.0, 6.0, 7.0, 8.0];
    assert_eq!(m2::add(&a, &b), [6.0, 8.0, 10.0, 12.0]);
    assert_eq!(m2::subtract(&a, &b), [-4.0, -4.0, -4.0, -4.0]);
}

#[test]
fn equals_and_equals_epsilon() {
    let a = [1.0, 2.0, 3.0, 4.0];
    let b = [1.0, 2.0, 3.0, 4.0];
    assert!(m2::equals(&a, &b));

    let c = [1.0 + 1e-15, 2.0, 3.0, 4.0];
    assert!(!m2::equals(&a, &c));
    assert!(m2::equals_epsilon(&a, &c, EPSILON14));
    assert!(!m2::equals_epsilon(&a, &c, 1e-16));
}

#[test]
fn equals_array_works() {
    let m = [1.0, 2.0, 3.0, 4.0];
    let array = [0.0, 1.0, 2.0, 3.0, 4.0, 0.0];
    assert!(m2::equals_array(&m, &array, 1));
    assert!(!m2::equals_array(&m, &array, 0));
}
