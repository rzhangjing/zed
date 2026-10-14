//! Core/InterpolationAlgorithms → Rust 集成测试。
//!
//! 参考 CesiumJS：
//! - Core/LinearApproximation
//! - Core/HermitePolynomialApproximation
//! - Core/LagrangePolynomialApproximation
//! - Core/InterpolationAlgorithm
//!
//! A 类测试：lerp/hermite/lagrange/catmull_rom/slerp/interpolate 分派。

use cesium_animation::interpolation::{
    catmull_rom, catmull_rom_vec3, hermite, hermite_vec3, interpolate, lagrange_interpolate,
    lagrange_interpolate_vec3, lerp, lerp_vec3, slerp_vec3, InterpolationType, SamplePoint,
};
use glam::DVec3;

// === lerp ===

#[test]
fn lerp_endpoints() {
    assert!((lerp(0.0, 10.0, 0.0) - 0.0).abs() < 1e-10);
    assert!((lerp(0.0, 10.0, 1.0) - 10.0).abs() < 1e-10);
}

#[test]
fn lerp_midpoint() {
    assert!((lerp(0.0, 10.0, 0.5) - 5.0).abs() < 1e-10);
    assert!((lerp(-10.0, 10.0, 0.5) - 0.0).abs() < 1e-10);
}

#[test]
fn lerp_extrapolate() {
    // t 在 [0,1] 外则外推
    assert!((lerp(0.0, 10.0, 2.0) - 20.0).abs() < 1e-10);
    assert!((lerp(0.0, 10.0, -1.0) - (-10.0)).abs() < 1e-10);
}

#[test]
fn lerp_vec3_basic() {
    let a = DVec3::new(0.0, 0.0, 0.0);
    let b = DVec3::new(10.0, 20.0, 30.0);
    let result = lerp_vec3(a, b, 0.5);
    assert!((result.x - 5.0).abs() < 1e-10);
    assert!((result.y - 10.0).abs() < 1e-10);
    assert!((result.z - 15.0).abs() < 1e-10);
}

// === hermite ===

#[test]
fn hermite_passes_through_endpoints() {
    let start = hermite(1.0, 2.0, 5.0, 3.0, 0.0);
    let end = hermite(1.0, 2.0, 5.0, 3.0, 1.0);
    assert!((start - 1.0).abs() < 1e-10);
    assert!((end - 5.0).abs() < 1e-10);
}

#[test]
fn hermite_zero_tangents_midpoint() {
    // 切线为零时，中点 = 平均值
    let result = hermite(0.0, 0.0, 10.0, 0.0, 0.5);
    assert!((result - 5.0).abs() < 1e-10);
}

#[test]
fn hermite_nonzero_tangents() {
    // 带切线的 Hermite 应会过冲
    let result = hermite(0.0, 10.0, 0.0, 10.0, 0.5);
    // h00=0.5, h10=0.125, h01=0.5, h11=-0.125
    // = 0 + 10*0.125 + 0 + 10*(-0.125) = 0
    assert!(result.abs() < 1e-10);
}

#[test]
fn hermite_vec3_component_wise() {
    let p0 = DVec3::new(0.0, 0.0, 0.0);
    let m0 = DVec3::new(0.0, 0.0, 0.0);
    let p1 = DVec3::new(10.0, 20.0, 30.0);
    let m1 = DVec3::new(0.0, 0.0, 0.0);
    let result = hermite_vec3(p0, m0, p1, m1, 0.5);
    assert!((result.x - 5.0).abs() < 1e-10);
    assert!((result.y - 10.0).abs() < 1e-10);
    assert!((result.z - 15.0).abs() < 1e-10);
}

// === lagrange ===

#[test]
fn lagrange_two_points_linear() {
    let points = vec![SamplePoint::new(0.0, 0.0), SamplePoint::new(1.0, 10.0)];
    let result = lagrange_interpolate(&points, 0.5);
    assert!((result - 5.0).abs() < 1e-10);
}

#[test]
fn lagrange_three_points_quadratic() {
    // y = x^2
    let points = vec![
        SamplePoint::new(0.0, 0.0),
        SamplePoint::new(1.0, 1.0),
        SamplePoint::new(2.0, 4.0),
    ];
    let result = lagrange_interpolate(&points, 1.5);
    assert!((result - 2.25).abs() < 1e-10);
}

#[test]
fn lagrange_single_point() {
    let points = vec![SamplePoint::new(5.0, 42.0)];
    assert!((lagrange_interpolate(&points, 0.0) - 42.0).abs() < 1e-10);
}

#[test]
fn lagrange_empty_returns_zero() {
    assert!((lagrange_interpolate(&[], 1.0) - 0.0).abs() < 1e-10);
}

#[test]
fn lagrange_vec3_basic() {
    let times = vec![0.0, 1.0];
    let values = vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(10.0, 20.0, 30.0)];
    let result = lagrange_interpolate_vec3(&times, &values, 0.5);
    assert!((result.x - 5.0).abs() < 1e-10);
    assert!((result.y - 10.0).abs() < 1e-10);
    assert!((result.z - 15.0).abs() < 1e-10);
}

// === catmull_rom ===

#[test]
fn catmull_rom_passes_through_inner_points() {
    let start = catmull_rom(0.0, 1.0, 4.0, 9.0, 0.0);
    let end = catmull_rom(0.0, 1.0, 4.0, 9.0, 1.0);
    assert!((start - 1.0).abs() < 1e-10);
    assert!((end - 4.0).abs() < 1e-10);
}

#[test]
fn catmull_rom_vec3_component_wise() {
    let p0 = DVec3::new(0.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 2.0, 3.0);
    let p2 = DVec3::new(4.0, 5.0, 6.0);
    let p3 = DVec3::new(9.0, 10.0, 11.0);
    let result = catmull_rom_vec3(p0, p1, p2, p3, 0.0);
    assert!((result.x - 1.0).abs() < 1e-10);
    assert!((result.y - 2.0).abs() < 1e-10);
    assert!((result.z - 3.0).abs() < 1e-10);
}

// === slerp ===

#[test]
fn slerp_same_direction() {
    let a = DVec3::X;
    let result = slerp_vec3(a, a, 0.5);
    assert!((result.x - 1.0).abs() < 1e-10);
    assert!(result.y.abs() < 1e-10);
}

#[test]
fn slerp_perpendicular() {
    let a = DVec3::X;
    let b = DVec3::Y;
    let result = slerp_vec3(a, b, 0.5);
    let expected = std::f64::consts::FRAC_1_SQRT_2;
    assert!((result.x - expected).abs() < 1e-10);
    assert!((result.y - expected).abs() < 1e-10);
}

#[test]
fn slerp_endpoints() {
    let a = DVec3::X;
    let b = DVec3::Y;
    let r0 = slerp_vec3(a, b, 0.0);
    let r1 = slerp_vec3(a, b, 1.0);
    assert!((r0.x - 1.0).abs() < 1e-10);
    assert!((r1.y - 1.0).abs() < 1e-10);
}

// === interpolate 分派 ===

#[test]
fn interpolate_linear_dispatch() {
    let points = vec![
        SamplePoint::new(0.0, 0.0),
        SamplePoint::new(10.0, 100.0),
    ];
    let result = interpolate(InterpolationType::Linear, &points, 5.0);
    assert!((result - 50.0).abs() < 1e-10);
}

#[test]
fn interpolate_hermite_dispatch() {
    let points = vec![
        SamplePoint::with_derivative(0.0, 0.0, 0.0),
        SamplePoint::with_derivative(1.0, 1.0, 0.0),
    ];
    let result = interpolate(InterpolationType::Hermite, &points, 0.5);
    assert!((result - 0.5).abs() < 1e-10);
}

#[test]
fn interpolate_lagrange_dispatch() {
    let points = vec![
        SamplePoint::new(0.0, 0.0),
        SamplePoint::new(1.0, 1.0),
        SamplePoint::new(2.0, 4.0),
    ];
    let result = interpolate(InterpolationType::Lagrange, &points, 0.5);
    assert!((result - 0.25).abs() < 1e-10);
}

#[test]
fn interpolate_empty_returns_zero() {
    assert_eq!(interpolate(InterpolationType::Linear, &[], 0.5), 0.0);
}

#[test]
fn interpolate_single_returns_value() {
    let points = vec![SamplePoint::new(0.0, 42.0)];
    assert_eq!(interpolate(InterpolationType::Linear, &points, 99.0), 42.0);
}

#[test]
fn interpolation_type_default_is_linear() {
    assert_eq!(InterpolationType::default(), InterpolationType::Linear);
}
