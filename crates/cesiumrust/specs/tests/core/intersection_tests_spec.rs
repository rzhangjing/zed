//! Core/IntersectionTestsSpec → Rust integration tests
//! A 类测试用例的对齐实现（不含 "throws" 测试）。

use cesium_geospatial::bounding::BoundingSphere;
use cesium_geospatial::ray::{
    line_segment_plane, line_segment_triangle, ray_ellipsoid, ray_plane, ray_sphere,
    ray_triangle, triangle_plane_intersection, Plane, Ray,
};
use cesium_geospatial::Ellipsoid;
use cesium_specs::{assert_approx, assert_vec3_epsilon, epsilon};
use glam::DVec3;

// ============================================================
// rayPlane
// ============================================================

#[test]
fn ray_plane_intersects() {
    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(-1.0, 0.0, 0.0));
    let plane = Plane::new(DVec3::X, -1.0);
    let intersection = ray_plane(&ray, &plane).unwrap();
    assert_vec3_epsilon!(intersection, DVec3::new(1.0, 0.0, 0.0), epsilon::EPSILON15);
}

#[test]
fn ray_plane_misses() {
    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    let plane = Plane::new(DVec3::X, -1.0);
    assert!(ray_plane(&ray, &plane).is_none());
}

#[test]
fn ray_plane_misses_parallel() {
    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(0.0, 1.0, 0.0));
    let plane = Plane::new(DVec3::X, -1.0);
    assert!(ray_plane(&ray, &plane).is_none());
}

// ============================================================
// rayTriangle
// ============================================================

#[test]
fn ray_triangle_intersects_front_face() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let ray = Ray::new(DVec3::Z, -DVec3::Z);
    let intersection = ray_triangle(&ray, p0, p1, p2, false).unwrap();
    assert_vec3_epsilon!(intersection, DVec3::ZERO, epsilon::EPSILON15);
}

#[test]
fn ray_triangle_intersects_back_face_without_culling() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let ray = Ray::new(-DVec3::Z, DVec3::Z);
    let intersection = ray_triangle(&ray, p0, p1, p2, false).unwrap();
    assert_vec3_epsilon!(intersection, DVec3::ZERO, epsilon::EPSILON15);
}

#[test]
fn ray_triangle_does_not_intersect_back_face_with_culling() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let ray = Ray::new(-DVec3::Z, DVec3::Z);
    assert!(ray_triangle(&ray, p0, p1, p2, true).is_none());
}

#[test]
fn ray_triangle_does_not_intersect_outside_0_1_edge() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let ray = Ray::new(DVec3::new(0.0, -1.0, 1.0), -DVec3::Z);
    assert!(ray_triangle(&ray, p0, p1, p2, false).is_none());
}

#[test]
fn ray_triangle_does_not_intersect_outside_1_2_edge() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let ray = Ray::new(DVec3::new(1.0, 1.0, 1.0), -DVec3::Z);
    assert!(ray_triangle(&ray, p0, p1, p2, false).is_none());
}

#[test]
fn ray_triangle_does_not_intersect_outside_2_0_edge() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let ray = Ray::new(DVec3::new(-1.0, 1.0, 1.0), -DVec3::Z);
    assert!(ray_triangle(&ray, p0, p1, p2, false).is_none());
}

#[test]
fn ray_triangle_does_not_intersect_parallel() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let ray = Ray::new(DVec3::new(-1.0, 0.0, 1.0), DVec3::X);
    assert!(ray_triangle(&ray, p0, p1, p2, false).is_none());
}

#[test]
fn ray_triangle_does_not_intersect_behind_origin() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let ray = Ray::new(DVec3::Z, DVec3::Z);
    assert!(ray_triangle(&ray, p0, p1, p2, false).is_none());
}

// ============================================================
// lineSegmentTriangle
// ============================================================

#[test]
fn line_segment_triangle_intersects_front_face() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let v0 = DVec3::Z;
    let v1 = -DVec3::Z;
    let intersection = line_segment_triangle(v0, v1, p0, p1, p2, false).unwrap();
    assert_vec3_epsilon!(intersection, DVec3::ZERO, epsilon::EPSILON15);
}

#[test]
fn line_segment_triangle_intersects_back_face_without_culling() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let v0 = -DVec3::Z;
    let v1 = DVec3::Z;
    let intersection = line_segment_triangle(v0, v1, p0, p1, p2, false).unwrap();
    assert_vec3_epsilon!(intersection, DVec3::ZERO, epsilon::EPSILON15);
}

#[test]
fn line_segment_triangle_does_not_intersect_back_face_with_culling() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let v0 = -DVec3::Z;
    let v1 = DVec3::Z;
    assert!(line_segment_triangle(v0, v1, p0, p1, p2, true).is_none());
}

#[test]
fn line_segment_triangle_does_not_intersect_outside_0_1_edge() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let v0 = DVec3::new(0.0, -1.0, 1.0);
    let v1 = v0 + (-DVec3::Z);
    assert!(line_segment_triangle(v0, v1, p0, p1, p2, false).is_none());
}

#[test]
fn line_segment_triangle_does_not_intersect_outside_1_2_edge() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let v0 = DVec3::new(1.0, 1.0, 1.0);
    let v1 = v0 + (-DVec3::Z);
    assert!(line_segment_triangle(v0, v1, p0, p1, p2, false).is_none());
}

#[test]
fn line_segment_triangle_does_not_intersect_outside_2_0_edge() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let v0 = DVec3::new(-1.0, 1.0, 1.0);
    let v1 = v0 + (-DVec3::Z);
    assert!(line_segment_triangle(v0, v1, p0, p1, p2, false).is_none());
}

#[test]
fn line_segment_triangle_does_not_intersect_parallel() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let v0 = DVec3::new(-1.0, 0.0, 1.0);
    let v1 = v0 + DVec3::X;
    assert!(line_segment_triangle(v0, v1, p0, p1, p2, false).is_none());
}

#[test]
fn line_segment_triangle_does_not_intersect_behind_v0() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let v0 = DVec3::Z;
    let v1 = DVec3::Z * 2.0;
    assert!(line_segment_triangle(v0, v1, p0, p1, p2, false).is_none());
}

#[test]
fn line_segment_triangle_does_not_intersect_behind_v1() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 0.0);
    let v0 = DVec3::Z * 2.0;
    let v1 = DVec3::Z;
    assert!(line_segment_triangle(v0, v1, p0, p1, p2, false).is_none());
}

// ============================================================
// raySphere
// ============================================================

#[test]
fn ray_sphere_outside_intersections() {
    let unit_sphere = BoundingSphere::new(DVec3::ZERO, 1.0);

    // From +X toward origin
    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(-1.0, 0.0, 0.0));
    let (start, stop) = ray_sphere(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    // From +Y toward origin
    let ray = Ray::new(DVec3::new(0.0, 2.0, 0.0), DVec3::new(0.0, -1.0, 0.0));
    let (start, stop) = ray_sphere(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    // From +Z toward origin
    let ray = Ray::new(DVec3::new(0.0, 0.0, 2.0), DVec3::new(0.0, 0.0, -1.0));
    let (start, stop) = ray_sphere(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    // Tangent-ish from (1,1,0) toward -X: start=1
    let ray = Ray::new(DVec3::new(1.0, 1.0, 0.0), DVec3::new(-1.0, 0.0, 0.0));
    let (start, _stop) = ray_sphere(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);

    // From -X toward +X
    let ray = Ray::new(DVec3::new(-2.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    let (start, stop) = ray_sphere(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    // From -Y toward +Y
    let ray = Ray::new(DVec3::new(0.0, -2.0, 0.0), DVec3::new(0.0, 1.0, 0.0));
    let (start, stop) = ray_sphere(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    // From -Z toward +Z
    let ray = Ray::new(DVec3::new(0.0, 0.0, -2.0), DVec3::new(0.0, 0.0, 1.0));
    let (start, stop) = ray_sphere(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    // From (-1,-1,0) toward +X: start=1
    let ray = Ray::new(DVec3::new(-1.0, -1.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    let (start, _stop) = ray_sphere(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);

    // 背离：无相交
    let ray = Ray::new(DVec3::new(-2.0, 0.0, 0.0), DVec3::new(-1.0, 0.0, 0.0));
    assert!(ray_sphere(&ray, &unit_sphere).is_none());

    let ray = Ray::new(DVec3::new(0.0, -2.0, 0.0), DVec3::new(0.0, -1.0, 0.0));
    assert!(ray_sphere(&ray, &unit_sphere).is_none());

    let ray = Ray::new(DVec3::new(0.0, 0.0, -2.0), DVec3::new(0.0, 0.0, -1.0));
    assert!(ray_sphere(&ray, &unit_sphere).is_none());
}

#[test]
fn ray_sphere_ray_inside_pointing_in() {
    let sphere = BoundingSphere::new(DVec3::ZERO, 5000.0);
    let origin = DVec3::new(200.0, 0.0, 0.0);
    let direction = -origin.normalize();
    let ray = Ray::new(origin, direction);

    let (start, stop) = ray_sphere(&ray, &sphere).unwrap();
    assert_approx!(start, 0.0, epsilon::EPSILON14);
    assert_approx!(stop, sphere.radius + origin.x, epsilon::EPSILON14);
}

#[test]
fn ray_sphere_ray_inside_pointing_out() {
    let sphere = BoundingSphere::new(DVec3::ZERO, 5000.0);
    let origin = DVec3::new(200.0, 0.0, 0.0);
    let direction = origin.normalize();
    let ray = Ray::new(origin, direction);

    let (start, stop) = ray_sphere(&ray, &sphere).unwrap();
    assert_approx!(start, 0.0, epsilon::EPSILON14);
    assert_approx!(stop, sphere.radius - origin.x, epsilon::EPSILON14);
}

#[test]
fn ray_sphere_tangent_intersections() {
    let unit_sphere = BoundingSphere::new(DVec3::ZERO, 1.0);
    let ray = Ray::new(DVec3::X, DVec3::Z);
    assert!(ray_sphere(&ray, &unit_sphere).is_none());
}

#[test]
fn ray_sphere_no_intersections() {
    let unit_sphere = BoundingSphere::new(DVec3::ZERO, 1.0);

    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 1.0));
    assert!(ray_sphere(&ray, &unit_sphere).is_none());

    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(0.0, 0.0, -1.0));
    assert!(ray_sphere(&ray, &unit_sphere).is_none());

    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(0.0, 1.0, 0.0));
    assert!(ray_sphere(&ray, &unit_sphere).is_none());

    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(0.0, -1.0, 0.0));
    assert!(ray_sphere(&ray, &unit_sphere).is_none());
}

#[test]
fn ray_sphere_intersection_with_non_origin_center() {
    let sphere = BoundingSphere::new(DVec3::new(200.0, 0.0, 0.0), 1.0);

    let ray = Ray::new(DVec3::new(202.0, 0.0, 0.0), DVec3::new(-1.0, 0.0, 0.0));
    let (start, stop) = ray_sphere(&ray, &sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(200.0, 2.0, 0.0), DVec3::new(0.0, -1.0, 0.0));
    let (start, stop) = ray_sphere(&ray, &sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(200.0, 0.0, 2.0), DVec3::new(0.0, 0.0, -1.0));
    let (start, stop) = ray_sphere(&ray, &sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(201.0, 1.0, 0.0), DVec3::new(-1.0, 0.0, 0.0));
    let (start, _stop) = ray_sphere(&ray, &sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(198.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    let (start, stop) = ray_sphere(&ray, &sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(200.0, -2.0, 0.0), DVec3::new(0.0, 1.0, 0.0));
    let (start, stop) = ray_sphere(&ray, &sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(200.0, 0.0, -2.0), DVec3::new(0.0, 0.0, 1.0));
    let (start, stop) = ray_sphere(&ray, &sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(199.0, -1.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    let (start, _stop) = ray_sphere(&ray, &sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);

    // 背离
    let ray = Ray::new(DVec3::new(198.0, 0.0, 0.0), DVec3::new(-1.0, 0.0, 0.0));
    assert!(ray_sphere(&ray, &sphere).is_none());

    let ray = Ray::new(DVec3::new(200.0, -2.0, 0.0), DVec3::new(0.0, -1.0, 0.0));
    assert!(ray_sphere(&ray, &sphere).is_none());

    let ray = Ray::new(DVec3::new(200.0, 0.0, -2.0), DVec3::new(0.0, 0.0, -1.0));
    assert!(ray_sphere(&ray, &sphere).is_none());
}

// ============================================================
// rayEllipsoid
// ============================================================

#[test]
fn ray_ellipsoid_outside_intersections() {
    let unit_sphere = Ellipsoid::UNIT_SPHERE;

    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(-1.0, 0.0, 0.0));
    let (start, stop) = ray_ellipsoid(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(0.0, 2.0, 0.0), DVec3::new(0.0, -1.0, 0.0));
    let (start, stop) = ray_ellipsoid(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(0.0, 0.0, 2.0), DVec3::new(0.0, 0.0, -1.0));
    let (start, stop) = ray_ellipsoid(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(1.0, 1.0, 0.0), DVec3::new(-1.0, 0.0, 0.0));
    let (start, _stop) = ray_ellipsoid(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(-2.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    let (start, stop) = ray_ellipsoid(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(0.0, -2.0, 0.0), DVec3::new(0.0, 1.0, 0.0));
    let (start, stop) = ray_ellipsoid(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(0.0, 0.0, -2.0), DVec3::new(0.0, 0.0, 1.0));
    let (start, stop) = ray_ellipsoid(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);
    assert_approx!(stop, 3.0, epsilon::EPSILON14);

    let ray = Ray::new(DVec3::new(-1.0, -1.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    let (start, _stop) = ray_ellipsoid(&ray, &unit_sphere).unwrap();
    assert_approx!(start, 1.0, epsilon::EPSILON14);

    // 背离
    let ray = Ray::new(DVec3::new(-2.0, 0.0, 0.0), DVec3::new(-1.0, 0.0, 0.0));
    assert!(ray_ellipsoid(&ray, &unit_sphere).is_none());

    let ray = Ray::new(DVec3::new(0.0, -2.0, 0.0), DVec3::new(0.0, -1.0, 0.0));
    assert!(ray_ellipsoid(&ray, &unit_sphere).is_none());

    let ray = Ray::new(DVec3::new(0.0, 0.0, -2.0), DVec3::new(0.0, 0.0, -1.0));
    assert!(ray_ellipsoid(&ray, &unit_sphere).is_none());
}

#[test]
fn ray_ellipsoid_ray_inside_pointing_in() {
    let ellipsoid = Ellipsoid::WGS84;
    let origin = DVec3::new(20000.0, 0.0, 0.0);
    let direction = -origin.normalize();
    let ray = Ray::new(origin, direction);

    let (start, stop) = ray_ellipsoid(&ray, &ellipsoid).unwrap();
    assert_approx!(start, 0.0, epsilon::EPSILON14);
    assert_approx!(stop, ellipsoid.radii().x + origin.x, epsilon::EPSILON14);
}

#[test]
fn ray_ellipsoid_ray_inside_pointing_out() {
    let ellipsoid = Ellipsoid::WGS84;
    let origin = DVec3::new(20000.0, 0.0, 0.0);
    let direction = origin.normalize();
    let ray = Ray::new(origin, direction);

    let (start, stop) = ray_ellipsoid(&ray, &ellipsoid).unwrap();
    assert_approx!(start, 0.0, epsilon::EPSILON14);
    assert_approx!(stop, ellipsoid.radii().x - origin.x, epsilon::EPSILON14);
}

#[test]
fn ray_ellipsoid_tangent_intersections() {
    let unit_sphere = Ellipsoid::UNIT_SPHERE;
    let ray = Ray::new(DVec3::X, DVec3::Z);
    assert!(ray_ellipsoid(&ray, &unit_sphere).is_none());
}

#[test]
fn ray_ellipsoid_no_intersections() {
    let unit_sphere = Ellipsoid::UNIT_SPHERE;

    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 1.0));
    assert!(ray_ellipsoid(&ray, &unit_sphere).is_none());

    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(0.0, 0.0, -1.0));
    assert!(ray_ellipsoid(&ray, &unit_sphere).is_none());

    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(0.0, 1.0, 0.0));
    assert!(ray_ellipsoid(&ray, &unit_sphere).is_none());

    let ray = Ray::new(DVec3::new(2.0, 0.0, 0.0), DVec3::new(0.0, -1.0, 0.0));
    assert!(ray_ellipsoid(&ray, &unit_sphere).is_none());
}

// ============================================================
// lineSegmentPlane
// ============================================================

#[test]
fn line_segment_plane_intersects() {
    let normal = DVec3::Y;
    let point = DVec3::new(0.0, 2.0, 0.0);
    let plane = Plane::from_point_normal(point, normal);

    let end_point0 = DVec3::new(1.0, 1.0, 0.0);
    let end_point1 = DVec3::new(1.0, 3.0, 0.0);

    let intersection = line_segment_plane(end_point0, end_point1, &plane).unwrap();
    assert_vec3_epsilon!(intersection, DVec3::new(1.0, 2.0, 0.0), epsilon::EPSILON15);
}

#[test]
fn line_segment_plane_misses_behind() {
    let plane = Plane::new(DVec3::X, 0.0);
    let end_point0 = DVec3::new(-2.0, 0.0, 0.0);
    let end_point1 = DVec3::new(-5.0, 0.0, 0.0);
    assert!(line_segment_plane(end_point0, end_point1, &plane).is_none());
}

#[test]
fn line_segment_plane_misses_in_front() {
    let plane = Plane::new(DVec3::X, 0.0);
    let end_point0 = DVec3::new(5.0, 0.0, 0.0);
    let end_point1 = DVec3::new(2.0, 0.0, 0.0);
    assert!(line_segment_plane(end_point0, end_point1, &plane).is_none());
}

#[test]
fn line_segment_plane_misses_parallel() {
    let plane = Plane::new(DVec3::X, 0.0);
    let end_point0 = DVec3::new(0.0, -1.0, 0.0);
    let end_point1 = DVec3::new(0.0, 1.0, 0.0);
    assert!(line_segment_plane(end_point0, end_point1, &plane).is_none());
}

// ============================================================
// trianglePlaneIntersection
// ============================================================

#[test]
fn triangle_is_in_front_of_plane() {
    let plane = Plane::new(DVec3::Z, 0.0);
    let p0 = DVec3::new(0.0, 0.0, 2.0);
    let p1 = DVec3::new(0.0, 1.0, 2.0);
    let p2 = DVec3::new(1.0, 0.0, 2.0);
    assert!(triangle_plane_intersection(p0, p1, p2, &plane).is_none());
}

#[test]
fn triangle_is_behind_plane() {
    let plane = Plane::new(-DVec3::Z, 0.0);
    let p0 = DVec3::new(0.0, 0.0, 2.0);
    let p1 = DVec3::new(0.0, 1.0, 2.0);
    let p2 = DVec3::new(1.0, 0.0, 2.0);
    assert!(triangle_plane_intersection(p0, p1, p2, &plane).is_none());
}

#[test]
fn triangle_intersects_plane_with_p0_behind() {
    let plane = Plane::new(DVec3::Z, -1.0);
    let p0 = DVec3::new(0.0, 0.0, 0.0);
    let p1 = DVec3::new(0.0, 1.0, 2.0);
    let p2 = DVec3::new(0.0, -1.0, 2.0);

    let result = triangle_plane_intersection(p0, p1, p2, &plane).unwrap();
    assert_eq!(result.indices.len(), 3 + 6);
    // positions[indices[0]] == p0
    assert!(result.positions[result.indices[0] as usize].abs_diff_eq(p0, 1e-15));
}

#[test]
fn triangle_intersects_plane_with_p1_behind() {
    let plane = Plane::new(DVec3::Z, -1.0);
    let p0 = DVec3::new(0.0, -1.0, 2.0);
    let p1 = DVec3::new(0.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 2.0);

    let result = triangle_plane_intersection(p0, p1, p2, &plane).unwrap();
    assert_eq!(result.indices.len(), 3 + 6);
    assert!(result.positions[result.indices[0] as usize].abs_diff_eq(p1, 1e-15));
}

#[test]
fn triangle_intersects_plane_with_p2_behind() {
    let plane = Plane::new(DVec3::Z, -1.0);
    let p0 = DVec3::new(0.0, 1.0, 2.0);
    let p1 = DVec3::new(0.0, -1.0, 2.0);
    let p2 = DVec3::new(0.0, 0.0, 0.0);

    let result = triangle_plane_intersection(p0, p1, p2, &plane).unwrap();
    assert_eq!(result.indices.len(), 3 + 6);
    assert!(result.positions[result.indices[0] as usize].abs_diff_eq(p2, 1e-15));
}

#[test]
fn triangle_intersects_plane_with_p0_in_front() {
    let plane = Plane::new(DVec3::Y, -1.0);
    let p0 = DVec3::new(0.0, 2.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(-1.0, 0.0, 0.0);

    let result = triangle_plane_intersection(p0, p1, p2, &plane).unwrap();
    assert_eq!(result.indices.len(), 6 + 3);
    // p0 is in front → behind triangle starts with p1
    assert!(result.positions[result.indices[0] as usize].abs_diff_eq(p1, 1e-15));
    assert!(result.positions[result.indices[1] as usize].abs_diff_eq(p2, 1e-15));
}

#[test]
fn triangle_intersects_plane_with_p1_in_front() {
    let plane = Plane::new(DVec3::Y, -1.0);
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(0.0, 2.0, 0.0);
    let p2 = DVec3::new(1.0, 0.0, 0.0);

    let result = triangle_plane_intersection(p0, p1, p2, &plane).unwrap();
    assert_eq!(result.indices.len(), 6 + 3);
    // p1 is in front → behind triangle starts with p2
    assert!(result.positions[result.indices[0] as usize].abs_diff_eq(p2, 1e-15));
    assert!(result.positions[result.indices[1] as usize].abs_diff_eq(p0, 1e-15));
}

#[test]
fn triangle_intersects_plane_with_p2_in_front() {
    let plane = Plane::new(DVec3::Y, -1.0);
    let p0 = DVec3::new(1.0, 0.0, 0.0);
    let p1 = DVec3::new(-1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 2.0, 0.0);

    let result = triangle_plane_intersection(p0, p1, p2, &plane).unwrap();
    assert_eq!(result.indices.len(), 6 + 3);
    // p2 is in front → behind triangle starts with p0
    assert!(result.positions[result.indices[0] as usize].abs_diff_eq(p0, 1e-15));
    assert!(result.positions[result.indices[1] as usize].abs_diff_eq(p1, 1e-15));
}
