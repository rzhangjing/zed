//! Cartesian4 扩展函数的测试。
//! 参考 CesiumJS `Specs/Core/Cartesian4Spec` 的 A 类测试。

use cesium_geospatial::cartesian4_ext as c4;
use cesium_geospatial::math_utils;
use glam::DVec4;

const EPSILON14: f64 = math_utils::EPSILON14;

#[test]
fn from_array_creates_cartesian4() {
    let v = c4::from_array(&[1.0, 2.0, 3.0, 4.0], 0);
    assert_eq!(v, DVec4::new(1.0, 2.0, 3.0, 4.0));
}

#[test]
fn from_array_with_offset() {
    let v = c4::from_array(&[0.0, 1.0, 2.0, 3.0, 4.0, 0.0], 1);
    assert_eq!(v, DVec4::new(1.0, 2.0, 3.0, 4.0));
}

#[test]
fn pack_and_unpack() {
    let v = DVec4::new(1.0, 2.0, 3.0, 4.0);
    let mut array = vec![0.0; 6];
    c4::pack(v, &mut array, 1);
    assert_eq!(array[1], 1.0);
    assert_eq!(array[2], 2.0);
    assert_eq!(array[3], 3.0);
    assert_eq!(array[4], 4.0);

    let unpacked = c4::unpack(&array, 1);
    assert_eq!(unpacked, v);
}

#[test]
fn pack_array_and_unpack_array() {
    let input = vec![
        DVec4::new(1.0, 2.0, 3.0, 4.0),
        DVec4::new(5.0, 6.0, 7.0, 8.0),
    ];
    let packed = c4::pack_array(&input);
    assert_eq!(packed, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);

    let unpacked = c4::unpack_array(&packed);
    assert_eq!(unpacked, input);
}

#[test]
fn maximum_component_works() {
    assert_eq!(c4::maximum_component(DVec4::new(2.0, 1.0, 0.0, -1.0)), 2.0);
    assert_eq!(c4::maximum_component(DVec4::new(1.0, 2.0, 0.0, -1.0)), 2.0);
    assert_eq!(c4::maximum_component(DVec4::new(1.0, 0.0, 3.0, -1.0)), 3.0);
    assert_eq!(c4::maximum_component(DVec4::new(1.0, 0.0, -1.0, 4.0)), 4.0);
}

#[test]
fn minimum_component_works() {
    assert_eq!(c4::minimum_component(DVec4::new(1.0, 2.0, 3.0, 4.0)), 1.0);
    assert_eq!(c4::minimum_component(DVec4::new(2.0, 1.0, 3.0, 4.0)), 1.0);
    assert_eq!(c4::minimum_component(DVec4::new(2.0, 3.0, 1.0, 4.0)), 1.0);
    assert_eq!(c4::minimum_component(DVec4::new(2.0, 3.0, 4.0, 1.0)), 1.0);
}

#[test]
fn magnitude_squared_works() {
    let v = DVec4::new(2.0, 3.0, 4.0, 5.0);
    assert_eq!(c4::magnitude_squared(v), 4.0 + 9.0 + 16.0 + 25.0);
}

#[test]
fn magnitude_works() {
    let v = DVec4::new(2.0, 3.0, 4.0, 5.0);
    assert!((c4::magnitude(v) - 54.0f64.sqrt()).abs() < 1e-15);
}

#[test]
fn distance_works() {
    let d = c4::distance(
        DVec4::new(1.0, 0.0, 0.0, 0.0),
        DVec4::new(2.0, 0.0, 0.0, 0.0),
    );
    assert_eq!(d, 1.0);
}

#[test]
fn distance_squared_works() {
    let d = c4::distance_squared(
        DVec4::new(1.0, 0.0, 0.0, 0.0),
        DVec4::new(3.0, 0.0, 0.0, 0.0),
    );
    assert_eq!(d, 4.0);
}

#[test]
fn lerp_normal() {
    let start = DVec4::new(4.0, 8.0, 12.0, 16.0);
    let end = DVec4::new(8.0, 20.0, 32.0, 44.0);
    let result = c4::lerp(start, end, 0.25);
    assert_eq!(result, DVec4::new(5.0, 11.0, 17.0, 23.0));
}

#[test]
fn lerp_extrapolate_forward() {
    let start = DVec4::new(4.0, 8.0, 12.0, 16.0);
    let end = DVec4::new(8.0, 20.0, 32.0, 44.0);
    let result = c4::lerp(start, end, 2.0);
    assert_eq!(result, DVec4::new(12.0, 32.0, 52.0, 72.0));
}

#[test]
fn lerp_extrapolate_backward() {
    let start = DVec4::new(4.0, 8.0, 12.0, 16.0);
    let end = DVec4::new(8.0, 20.0, 32.0, 44.0);
    let result = c4::lerp(start, end, -1.0);
    assert_eq!(result, DVec4::new(0.0, -4.0, -8.0, -12.0));
}

#[test]
fn angle_between_right_angles() {
    let x = DVec4::new(1.0, 0.0, 0.0, 0.0);
    let y = DVec4::new(0.0, 1.0, 0.0, 0.0);
    assert!((c4::angle_between(x, y) - math_utils::PI_OVER_TWO).abs() < EPSILON14);
    assert!((c4::angle_between(y, x) - math_utils::PI_OVER_TWO).abs() < EPSILON14);
}

#[test]
fn angle_between_zero() {
    let x = DVec4::new(1.0, 0.0, 0.0, 0.0);
    assert!(c4::angle_between(x, x).abs() < EPSILON14);
}

#[test]
fn angle_between_acute() {
    let x = DVec4::new(0.0, 1.0, 0.0, 0.0);
    let y = DVec4::new(1.0, 1.0, 0.0, 0.0);
    let expected = std::f64::consts::FRAC_PI_4;
    assert!((c4::angle_between(x, y) - expected).abs() < EPSILON14);
}

#[test]
fn equals_epsilon_works() {
    let v = DVec4::new(1.0, 2.0, 3.0, 4.0);
    assert!(c4::equals_epsilon(v, DVec4::new(1.0, 2.0, 3.0, 4.0), 0.0, 0.0));
    assert!(c4::equals_epsilon(v, DVec4::new(2.0, 2.0, 3.0, 4.0), 0.0, 1.0));
    assert!(c4::equals_epsilon(v, DVec4::new(1.0, 2.0, 3.0, 5.0), 0.0, 1.0));
    assert!(!c4::equals_epsilon(
        v,
        DVec4::new(1.0, 2.0, 3.0, 5.0),
        0.0,
        math_utils::EPSILON6
    ));
}

#[test]
fn clamp_works() {
    let value = DVec4::new(-1.0, 0.0, 2.0, 0.5);
    let min = DVec4::new(0.0, 0.0, 0.0, 0.0);
    let max = DVec4::new(1.0, 1.0, 1.0, 1.0);
    assert_eq!(c4::clamp(value, min, max), DVec4::new(0.0, 0.0, 1.0, 0.5));
}

#[test]
fn abs_works() {
    let v = DVec4::new(1.0, -2.0, -3.0, 4.0);
    assert_eq!(c4::abs(v), DVec4::new(1.0, 2.0, 3.0, 4.0));
}

#[test]
fn multiply_components_works() {
    let left = DVec4::new(2.0, 3.0, 4.0, 5.0);
    let right = DVec4::new(4.0, 5.0, 6.0, 7.0);
    assert_eq!(
        c4::multiply_components(left, right),
        DVec4::new(8.0, 15.0, 24.0, 35.0)
    );
}

#[test]
fn divide_components_works() {
    let left = DVec4::new(2.0, 3.0, 4.0, 5.0);
    let right = DVec4::new(4.0, 5.0, 8.0, 10.0);
    assert_eq!(
        c4::divide_components(left, right),
        DVec4::new(0.5, 0.6, 0.5, 0.5)
    );
}

#[test]
fn minimum_by_component_works() {
    let first = DVec4::new(2.0, -15.0, 3.0, 1.0);
    let second = DVec4::new(1.0, -20.0, 4.0, 0.0);
    assert_eq!(first.min(second), DVec4::new(1.0, -20.0, 3.0, 0.0));
}

#[test]
fn maximum_by_component_works() {
    let first = DVec4::new(2.0, -15.0, 3.0, 1.0);
    let second = DVec4::new(1.0, -20.0, 4.0, 0.0);
    assert_eq!(first.max(second), DVec4::new(2.0, -15.0, 4.0, 1.0));
}
