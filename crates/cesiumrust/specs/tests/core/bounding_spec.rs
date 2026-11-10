//! Core/BoundingSphereSpec、BoundingRectangleSpec、OrientedBoundingBoxSpec、
//! AxisAlignedBoundingBoxSpec → Rust 集成测试

use cesium_geospatial::bounding::{BoundingSphere, OrientedBoundingBox, AxisAlignedBoundingBox};
use cesium_specs::{assert_approx, assert_vec3_epsilon, epsilon};
use glam::DVec3;

// === BoundingSphere ===

#[test]
fn test_bounding_sphere_new() {
    let center = DVec3::new(1.0, 2.0, 3.0);
    let radius = 5.0;
    let bs = BoundingSphere::new(center, radius);
    assert_vec3_epsilon!(bs.center, center, epsilon::EPSILON15);
    assert_approx!(bs.radius, radius, epsilon::EPSILON15);
}

#[test]
fn test_bounding_sphere_default() {
    let bs = BoundingSphere::default();
    assert_vec3_epsilon!(bs.center, DVec3::ZERO, epsilon::EPSILON15);
    assert_approx!(bs.radius, 0.0, epsilon::EPSILON15);
}

#[test]
fn test_bounding_sphere_from_points() {
    let points = vec![
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, -1.0, 0.0),
        DVec3::new(0.0, 1.0, 0.0),
    ];
    let bs = BoundingSphere::from_points(&points);
    assert_approx!(bs.radius, 1.0, epsilon::EPSILON6);
    assert_vec3_epsilon!(bs.center, DVec3::ZERO, epsilon::EPSILON6);
}

#[test]
fn test_bounding_sphere_contains_point() {
    let bs = BoundingSphere::new(DVec3::ZERO, 5.0);
    assert!(bs.contains(DVec3::new(3.0, 0.0, 0.0)));
    assert!(bs.contains(DVec3::new(0.0, 4.0, 0.0)));
    assert!(!bs.contains(DVec3::new(6.0, 0.0, 0.0)));
}

#[test]
fn test_bounding_sphere_distance_to() {
    let bs = BoundingSphere::new(DVec3::ZERO, 1.0);
    let point = DVec3::new(5.0, 0.0, 0.0);
    let dist = bs.distance_to(point);
    assert_approx!(dist, 4.0, epsilon::EPSILON10);
}

#[test]
fn test_bounding_sphere_union() {
    let a = BoundingSphere::new(DVec3::new(-2.0, 0.0, 0.0), 1.0);
    let b = BoundingSphere::new(DVec3::new(2.0, 0.0, 0.0), 1.0);
    let union = BoundingSphere::union(&a, &b);
    assert!(union.radius >= 3.0);
    assert_vec3_epsilon!(union.center, DVec3::ZERO, epsilon::EPSILON6);
}

// === OrientedBoundingBox ===

#[test]
fn test_obb_new() {
    let center = DVec3::new(1.0, 2.0, 3.0);
    let half_axes = glam::DMat3::IDENTITY;
    let obb = OrientedBoundingBox::new(center, half_axes);
    assert_vec3_epsilon!(obb.center, center, epsilon::EPSILON15);
}

#[test]
fn test_obb_from_axes_half_lengths() {
    let center = DVec3::ZERO;
    let obb = OrientedBoundingBox::from_axes_half_lengths(
        center,
        DVec3::X,
        DVec3::Y,
        DVec3::Z,
        1.0,
        2.0,
        3.0,
    );
    assert_vec3_epsilon!(obb.center, center, epsilon::EPSILON10);
}

#[test]
fn test_obb_distance_to() {
    let center = DVec3::ZERO;
    let half_axes = glam::DMat3::from_diagonal(DVec3::new(2.0, 2.0, 2.0));
    let obb = OrientedBoundingBox::new(center, half_axes);
    // 点在内部
    let dist_inside = obb.distance_to(DVec3::new(1.0, 1.0, 1.0));
    assert_approx!(dist_inside, 0.0, epsilon::EPSILON10);
    // 点在外部
    let dist_outside = obb.distance_to(DVec3::new(3.0, 0.0, 0.0));
    assert_approx!(dist_outside, 1.0, epsilon::EPSILON10);
}

// === AxisAlignedBoundingBox ===

#[test]
fn test_aabb_new() {
    let min = DVec3::new(-1.0, -2.0, -3.0);
    let max = DVec3::new(1.0, 2.0, 3.0);
    let aabb = AxisAlignedBoundingBox::new(min, max);
    assert_vec3_epsilon!(aabb.minimum, min, epsilon::EPSILON15);
    assert_vec3_epsilon!(aabb.maximum, max, epsilon::EPSILON15);
}

#[test]
fn test_aabb_center() {
    let min = DVec3::new(0.0, 0.0, 0.0);
    let max = DVec3::new(4.0, 6.0, 8.0);
    let aabb = AxisAlignedBoundingBox::new(min, max);
    // center 是字段，而非方法
    assert_vec3_epsilon!(aabb.center, DVec3::new(2.0, 3.0, 4.0), epsilon::EPSILON15);
}

#[test]
fn test_aabb_contains_point() {
    let min = DVec3::new(-1.0, -1.0, -1.0);
    let max = DVec3::new(1.0, 1.0, 1.0);
    let aabb = AxisAlignedBoundingBox::new(min, max);
    assert!(aabb.contains(DVec3::ZERO));
    assert!(aabb.contains(DVec3::new(0.5, 0.5, 0.5)));
    assert!(!aabb.contains(DVec3::new(2.0, 0.0, 0.0)));
}

#[test]
fn test_aabb_from_points() {
    let points = vec![
        DVec3::new(-1.0, 2.0, -3.0),
        DVec3::new(4.0, -5.0, 6.0),
        DVec3::new(0.0, 0.0, 0.0),
    ];
    let aabb = AxisAlignedBoundingBox::from_points(&points);
    assert_vec3_epsilon!(aabb.minimum, DVec3::new(-1.0, -5.0, -3.0), epsilon::EPSILON15);
    assert_vec3_epsilon!(aabb.maximum, DVec3::new(4.0, 2.0, 6.0), epsilon::EPSILON15);
}
