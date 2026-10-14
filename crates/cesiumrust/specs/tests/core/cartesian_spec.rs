//! Core/Cartesian3Spec、Cartesian2Spec、Cartesian4Spec → Rust 集成测试
//! 在 CesiumRust 中，Cartesian3 = glam::DVec3、Cartesian2 = glam::DVec2、Cartesian4 = glam::DVec4
//! 这些测试验证地理空间领域所使用的向量运算。

use cesium_specs::{assert_approx, assert_vec3_epsilon, assert_vec2_epsilon, assert_vec4_epsilon, epsilon};
use glam::{DVec2, DVec3, DVec4};

// === Cartesian3 (DVec3) ===

#[test]
fn test_cartesian3_default() {
    let v = DVec3::ZERO;
    assert_approx!(v.x, 0.0, epsilon::EPSILON15);
    assert_approx!(v.y, 0.0, epsilon::EPSILON15);
    assert_approx!(v.z, 0.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_construct() {
    let v = DVec3::new(1.0, 2.0, 3.0);
    assert_approx!(v.x, 1.0, epsilon::EPSILON15);
    assert_approx!(v.y, 2.0, epsilon::EPSILON15);
    assert_approx!(v.z, 3.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_from_array() {
    let arr = [0.0, 1.0, 2.0, 3.0, 0.0];
    let v = DVec3::from_slice(&arr[1..4]);
    assert_approx!(v.x, 1.0, epsilon::EPSILON15);
    assert_approx!(v.y, 2.0, epsilon::EPSILON15);
    assert_approx!(v.z, 3.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_clone() {
    let v = DVec3::new(1.0, 2.0, 3.0);
    let cloned = v;
    assert_eq!(v, cloned);
}

#[test]
fn test_cartesian3_maximum_component() {
    assert_approx!(DVec3::new(2.0, 1.0, 0.0).max_element(), 2.0, epsilon::EPSILON15);
    assert_approx!(DVec3::new(1.0, 2.0, 0.0).max_element(), 2.0, epsilon::EPSILON15);
    assert_approx!(DVec3::new(1.0, 2.0, 3.0).max_element(), 3.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_minimum_component() {
    assert_approx!(DVec3::new(1.0, 2.0, 3.0).min_element(), 1.0, epsilon::EPSILON15);
    assert_approx!(DVec3::new(2.0, 1.0, 3.0).min_element(), 1.0, epsilon::EPSILON15);
    assert_approx!(DVec3::new(2.0, 1.0, 0.0).min_element(), 0.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_min_by_component() {
    let a = DVec3::new(2.0, -15.0, 26.5);
    let b = DVec3::new(1.0, -20.0, 26.4);
    let result = a.min(b);
    assert_approx!(result.x, 1.0, epsilon::EPSILON15);
    assert_approx!(result.y, -20.0, epsilon::EPSILON15);
    assert_approx!(result.z, 26.4, epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_max_by_component() {
    let a = DVec3::new(2.0, -15.0, 26.5);
    let b = DVec3::new(1.0, -20.0, 26.4);
    let result = a.max(b);
    assert_approx!(result.x, 2.0, epsilon::EPSILON15);
    assert_approx!(result.y, -15.0, epsilon::EPSILON15);
    assert_approx!(result.z, 26.5, epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_add() {
    let a = DVec3::new(1.0, 2.0, 3.0);
    let b = DVec3::new(4.0, 5.0, 6.0);
    let result = a + b;
    assert_vec3_epsilon!(result, DVec3::new(5.0, 7.0, 9.0), epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_subtract() {
    let a = DVec3::new(4.0, 5.0, 6.0);
    let b = DVec3::new(1.0, 2.0, 3.0);
    let result = a - b;
    assert_vec3_epsilon!(result, DVec3::new(3.0, 3.0, 3.0), epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_multiply_by_scalar() {
    let v = DVec3::new(1.0, 2.0, 3.0);
    let result = v * 2.0;
    assert_vec3_epsilon!(result, DVec3::new(2.0, 4.0, 6.0), epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_divide_by_scalar() {
    let v = DVec3::new(2.0, 4.0, 6.0);
    let result = v / 2.0;
    assert_vec3_epsilon!(result, DVec3::new(1.0, 2.0, 3.0), epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_negate() {
    let v = DVec3::new(1.0, -2.0, 3.0);
    let result = -v;
    assert_vec3_epsilon!(result, DVec3::new(-1.0, 2.0, -3.0), epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_dot() {
    let a = DVec3::new(1.0, 2.0, 3.0);
    let b = DVec3::new(4.0, 5.0, 6.0);
    assert_approx!(a.dot(b), 32.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_cross() {
    let a = DVec3::new(1.0, 0.0, 0.0);
    let b = DVec3::new(0.0, 1.0, 0.0);
    let result = a.cross(b);
    assert_vec3_epsilon!(result, DVec3::new(0.0, 0.0, 1.0), epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_magnitude() {
    let v = DVec3::new(3.0, 4.0, 0.0);
    assert_approx!(v.length(), 5.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_normalize() {
    let v = DVec3::new(3.0, 4.0, 0.0);
    let n = v.normalize();
    assert_approx!(n.length(), 1.0, epsilon::EPSILON15);
    assert_approx!(n.x, 0.6, epsilon::EPSILON15);
    assert_approx!(n.y, 0.8, epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_distance() {
    let a = DVec3::new(1.0, 0.0, 0.0);
    let b = DVec3::new(4.0, 0.0, 0.0);
    assert_approx!(a.distance(b), 3.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_lerp() {
    let a = DVec3::new(0.0, 0.0, 0.0);
    let b = DVec3::new(10.0, 20.0, 30.0);
    let result = a.lerp(b, 0.5);
    assert_vec3_epsilon!(result, DVec3::new(5.0, 10.0, 15.0), epsilon::EPSILON15);
}

#[test]
fn test_cartesian3_angle_between() {
    let a = DVec3::new(1.0, 0.0, 0.0);
    let b = DVec3::new(0.0, 1.0, 0.0);
    let angle = a.angle_between(b);
    assert_approx!(angle, std::f64::consts::FRAC_PI_2, epsilon::EPSILON14);
}

// === Cartesian2 (DVec2) ===

#[test]
fn test_cartesian2_default() {
    let v = DVec2::ZERO;
    assert_approx!(v.x, 0.0, epsilon::EPSILON15);
    assert_approx!(v.y, 0.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian2_construct() {
    let v = DVec2::new(1.0, 2.0);
    assert_approx!(v.x, 1.0, epsilon::EPSILON15);
    assert_approx!(v.y, 2.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian2_add() {
    let a = DVec2::new(1.0, 2.0);
    let b = DVec2::new(3.0, 4.0);
    assert_vec2_epsilon!(a + b, DVec2::new(4.0, 6.0), epsilon::EPSILON15);
}

#[test]
fn test_cartesian2_dot() {
    let a = DVec2::new(1.0, 2.0);
    let b = DVec2::new(3.0, 4.0);
    assert_approx!(a.dot(b), 11.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian2_magnitude() {
    let v = DVec2::new(3.0, 4.0);
    assert_approx!(v.length(), 5.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian2_normalize() {
    let v = DVec2::new(3.0, 4.0);
    let n = v.normalize();
    assert_approx!(n.length(), 1.0, epsilon::EPSILON15);
}

// === Cartesian4 (DVec4) ===

#[test]
fn test_cartesian4_default() {
    let v = DVec4::ZERO;
    assert_approx!(v.x, 0.0, epsilon::EPSILON15);
    assert_approx!(v.y, 0.0, epsilon::EPSILON15);
    assert_approx!(v.z, 0.0, epsilon::EPSILON15);
    assert_approx!(v.w, 0.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian4_construct() {
    let v = DVec4::new(1.0, 2.0, 3.0, 4.0);
    assert_approx!(v.x, 1.0, epsilon::EPSILON15);
    assert_approx!(v.w, 4.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian4_add() {
    let a = DVec4::new(1.0, 2.0, 3.0, 4.0);
    let b = DVec4::new(5.0, 6.0, 7.0, 8.0);
    assert_vec4_epsilon!(a + b, DVec4::new(6.0, 8.0, 10.0, 12.0), epsilon::EPSILON15);
}

#[test]
fn test_cartesian4_dot() {
    let a = DVec4::new(1.0, 2.0, 3.0, 4.0);
    let b = DVec4::new(5.0, 6.0, 7.0, 8.0);
    assert_approx!(a.dot(b), 70.0, epsilon::EPSILON15);
}

#[test]
fn test_cartesian4_magnitude() {
    let v = DVec4::new(1.0, 2.0, 2.0, 0.0);
    assert_approx!(v.length(), 3.0, epsilon::EPSILON15);
}
