//! Cartesian2 扩展函数的测试。
//! 参考 CesiumJS `Specs/Core/Cartesian2Spec` 的 A 类测试。

use cesium_geospatial::cartesian2_ext as c2;
use cesium_geospatial::math_utils;
use glam::DVec2;

const EPSILON14: f64 = math_utils::EPSILON14;

#[test]
fn from_array_creates_cartesian2() {
    let v = c2::from_array(&[1.0, 2.0], 0);
    assert_eq!(v, DVec2::new(1.0, 2.0));
}

#[test]
fn from_array_with_offset() {
    let v = c2::from_array(&[0.0, 1.0, 2.0, 0.0], 1);
    assert_eq!(v, DVec2::new(1.0, 2.0));
}

#[test]
fn pack_and_unpack() {
    let v = DVec2::new(1.0, 2.0);
    let mut array = vec![0.0; 4];
    c2::pack(v, &mut array, 1);
    assert_eq!(array[1], 1.0);
    assert_eq!(array[2], 2.0);

    let unpacked = c2::unpack(&array, 1);
    assert_eq!(unpacked, v);
}

#[test]
fn pack_array_and_unpack_array() {
    let input = vec![DVec2::new(1.0, 2.0), DVec2::new(3.0, 4.0)];
    let packed = c2::pack_array(&input);
    assert_eq!(packed, vec![1.0, 2.0, 3.0, 4.0]);

    let unpacked = c2::unpack_array(&packed);
    assert_eq!(unpacked, input);
}

#[test]
fn maximum_component_x_greater() {
    let v = DVec2::new(2.0, 1.0);
    assert_eq!(c2::maximum_component(v), 2.0);
}

#[test]
fn maximum_component_y_greater() {
    let v = DVec2::new(1.0, 2.0);
    assert_eq!(c2::maximum_component(v), 2.0);
}

#[test]
fn minimum_component_x_lesser() {
    let v = DVec2::new(1.0, 2.0);
    assert_eq!(c2::minimum_component(v), 1.0);
}

#[test]
fn minimum_component_y_lesser() {
    let v = DVec2::new(2.0, 1.0);
    assert_eq!(c2::minimum_component(v), 1.0);
}

#[test]
fn magnitude_squared_works() {
    let v = DVec2::new(2.0, 3.0);
    assert_eq!(c2::magnitude_squared(v), 13.0);
}

#[test]
fn magnitude_works() {
    let v = DVec2::new(2.0, 3.0);
    assert!((c2::magnitude(v) - 13.0f64.sqrt()).abs() < 1e-15);
}

#[test]
fn distance_works() {
    let d = c2::distance(DVec2::new(1.0, 0.0), DVec2::new(2.0, 0.0));
    assert_eq!(d, 1.0);
}

#[test]
fn distance_squared_works() {
    let d = c2::distance_squared(DVec2::new(1.0, 0.0), DVec2::new(3.0, 0.0));
    assert_eq!(d, 4.0);
}

#[test]
fn cross_returns_scalar() {
    let left = DVec2::new(0.0, 1.0);
    let right = DVec2::new(1.0, 0.0);
    assert_eq!(c2::cross(left, right), -1.0);
}

#[test]
fn dot_works() {
    let left = DVec2::new(2.0, 3.0);
    let right = DVec2::new(4.0, 5.0);
    assert_eq!(left.dot(right), 23.0);
}

#[test]
fn lerp_normal() {
    let start = DVec2::new(4.0, 8.0);
    let end = DVec2::new(8.0, 20.0);
    let result = c2::lerp(start, end, 0.25);
    assert_eq!(result, DVec2::new(5.0, 11.0));
}

#[test]
fn lerp_extrapolate_forward() {
    let start = DVec2::new(4.0, 8.0);
    let end = DVec2::new(8.0, 20.0);
    let result = c2::lerp(start, end, 2.0);
    assert_eq!(result, DVec2::new(12.0, 32.0));
}

#[test]
fn lerp_extrapolate_backward() {
    let start = DVec2::new(4.0, 8.0);
    let end = DVec2::new(8.0, 20.0);
    let result = c2::lerp(start, end, -1.0);
    assert_eq!(result, DVec2::new(0.0, -4.0));
}

#[test]
fn angle_between_right_angles() {
    let x = DVec2::X;
    let y = DVec2::Y;
    assert!((c2::angle_between(x, y) - math_utils::PI_OVER_TWO).abs() < EPSILON14);
    assert!((c2::angle_between(y, x) - math_utils::PI_OVER_TWO).abs() < EPSILON14);
}

#[test]
fn angle_between_acute() {
    let x = DVec2::new(0.0, 1.0);
    let y = DVec2::new(1.0, 1.0);
    let expected = std::f64::consts::FRAC_PI_4;
    assert!((c2::angle_between(x, y) - expected).abs() < EPSILON14);
    assert!((c2::angle_between(y, x) - expected).abs() < EPSILON14);
}

#[test]
fn angle_between_obtuse() {
    let x = DVec2::new(0.0, 1.0);
    let y = DVec2::new(-1.0, -1.0);
    let expected = std::f64::consts::PI * 3.0 / 4.0;
    assert!((c2::angle_between(x, y) - expected).abs() < EPSILON14);
    assert!((c2::angle_between(y, x) - expected).abs() < EPSILON14);
}

#[test]
fn angle_between_zero() {
    let x = DVec2::X;
    assert_eq!(c2::angle_between(x, x), 0.0);
}

#[test]
fn most_orthogonal_axis_x() {
    let v = DVec2::new(0.0, 1.0);
    assert_eq!(c2::most_orthogonal_axis(v), DVec2::X);
}

#[test]
fn most_orthogonal_axis_y() {
    let v = DVec2::new(1.0, 0.0);
    assert_eq!(c2::most_orthogonal_axis(v), DVec2::Y);
}

#[test]
fn clamp_works() {
    let value = DVec2::new(-1.0, 0.0);
    let min = DVec2::new(0.0, 0.0);
    let max = DVec2::new(1.0, 1.0);
    assert_eq!(c2::clamp(value, min, max), DVec2::new(0.0, 0.0));

    let value = DVec2::new(2.0, 0.0);
    assert_eq!(c2::clamp(value, min, max), DVec2::new(1.0, 0.0));

    let value = DVec2::new(-2.0, 3.0);
    assert_eq!(c2::clamp(value, min, max), DVec2::new(0.0, 1.0));
}

#[test]
fn equals_epsilon_works() {
    let v = DVec2::new(1.0, 2.0);
    assert!(c2::equals_epsilon(v, DVec2::new(1.0, 2.0), 0.0, 0.0));
    assert!(c2::equals_epsilon(v, DVec2::new(2.0, 2.0), 0.0, 1.0));
    assert!(c2::equals_epsilon(v, DVec2::new(1.0, 3.0), 0.0, 1.0));
    assert!(!c2::equals_epsilon(
        v,
        DVec2::new(1.0, 3.0),
        0.0,
        math_utils::EPSILON6
    ));
}

#[test]
fn equals_epsilon_relative() {
    let v = DVec2::new(3000000.0, 4000000.0);
    assert!(c2::equals_epsilon(
        v,
        DVec2::new(3000000.0, 4000000.2),
        math_utils::EPSILON7,
        0.0
    ));
    assert!(c2::equals_epsilon(
        v,
        DVec2::new(3000000.2, 4000000.0),
        math_utils::EPSILON7,
        0.0
    ));
    assert!(!c2::equals_epsilon(
        v,
        DVec2::new(3000000.2, 4000000.2),
        math_utils::EPSILON9,
        0.0
    ));
}

#[test]
fn abs_works() {
    let v = DVec2::new(1.0, -2.0);
    assert_eq!(c2::abs(v), DVec2::new(1.0, 2.0));
}

#[test]
fn multiply_components_works() {
    let left = DVec2::new(2.0, 3.0);
    let right = DVec2::new(4.0, 5.0);
    assert_eq!(c2::multiply_components(left, right), DVec2::new(8.0, 15.0));
}

#[test]
fn divide_components_works() {
    let left = DVec2::new(2.0, 3.0);
    let right = DVec2::new(4.0, 5.0);
    assert_eq!(
        c2::divide_components(left, right),
        DVec2::new(0.5, 0.6)
    );
}

#[test]
fn minimum_by_component_works() {
    let first = DVec2::new(2.0, -15.0);
    let second = DVec2::new(1.0, -20.0);
    assert_eq!(first.min(second), DVec2::new(1.0, -20.0));
}

#[test]
fn maximum_by_component_works() {
    let first = DVec2::new(2.0, -15.0);
    let second = DVec2::new(1.0, -20.0);
    assert_eq!(first.max(second), DVec2::new(2.0, -15.0));
}
