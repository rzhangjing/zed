//! Core/IntersectionTestsSpec、RaySpec、PlaneSpec、Intersections2DSpec
//! → Rust 集成测试

use cesium_geospatial::bounding::{AxisAlignedBoundingBox, BoundingSphere, OrientedBoundingBox};
use cesium_geospatial::ray::{
    compute_barycentric_coordinates, ray_aabb, ray_ellipsoid, ray_obb, ray_plane,
    ray_sphere, ray_triangle, Intersect, Plane, Ray,
};
use cesium_geospatial::Ellipsoid;
use cesium_specs::{assert_approx, assert_vec3_epsilon, epsilon};
use glam::DVec3;

// === Ray ===

#[test]
fn test_ray_new() {
    let origin = DVec3::new(1.0, 2.0, 3.0);
    let direction = DVec3::new(0.0, 0.0, 5.0); // 会被归一化
    let r = Ray::new(origin, direction);
    assert_vec3_epsilon!(r.origin, origin, epsilon::EPSILON15);
    assert_approx!(r.direction.length(), 1.0, epsilon::EPSILON15);
}

#[test]
fn test_ray_point_at() {
    let r = Ray::new(DVec3::ZERO, DVec3::new(0.0, 0.0, -1.0));
    let p = r.point_at(5.0);
    assert_vec3_epsilon!(p, DVec3::new(0.0, 0.0, -5.0), epsilon::EPSILON15);
}

// === Plane ===

#[test]
fn test_plane_from_point_normal() {
    let point = DVec3::new(0.0, 0.0, 5.0);
    let normal = DVec3::new(0.0, 0.0, 1.0);
    let plane = Plane::from_point_normal(point, normal);
    assert_approx!(plane.normal.z, 1.0, epsilon::EPSILON15);
    assert_approx!(plane.distance, -5.0, epsilon::EPSILON15);
}

#[test]
fn test_plane_point_distance() {
    let plane = Plane::from_point_normal(DVec3::ZERO, DVec3::new(0.0, 0.0, 1.0));
    let dist = plane.point_distance(DVec3::new(0.0, 0.0, 3.0));
    assert_approx!(dist, 3.0, epsilon::EPSILON15);
}

#[test]
fn test_plane_project_point() {
    let plane = Plane::from_point_normal(DVec3::ZERO, DVec3::new(0.0, 0.0, 1.0));
    let projected = plane.project_point_onto_plane(DVec3::new(1.0, 2.0, 5.0));
    assert_vec3_epsilon!(projected, DVec3::new(1.0, 2.0, 0.0), epsilon::EPSILON14);
}

// === 射线-平面 相交 ===

#[test]
fn test_ray_plane_hit() {
    let r = Ray::new(DVec3::new(0.0, 0.0, 5.0), DVec3::new(0.0, 0.0, -1.0));
    let plane = Plane::from_point_normal(DVec3::ZERO, DVec3::new(0.0, 0.0, 1.0));
    let hit = ray_plane(&r, &plane).unwrap();
    assert_vec3_epsilon!(hit, DVec3::ZERO, epsilon::EPSILON10);
}

#[test]
fn test_ray_plane_parallel_no_hit() {
    let r = Ray::new(DVec3::new(0.0, 0.0, 5.0), DVec3::new(1.0, 0.0, 0.0));
    let plane = Plane::from_point_normal(DVec3::ZERO, DVec3::new(0.0, 0.0, 1.0));
    assert!(ray_plane(&r, &plane).is_none());
}

#[test]
fn test_ray_plane_behind_no_hit() {
    let r = Ray::new(DVec3::new(0.0, 0.0, 5.0), DVec3::new(0.0, 0.0, 1.0));
    let plane = Plane::from_point_normal(DVec3::ZERO, DVec3::new(0.0, 0.0, 1.0));
    assert!(ray_plane(&r, &plane).is_none());
}

// === 射线-球 相交 ===

#[test]
fn test_ray_sphere_hit() {
    let r = Ray::new(DVec3::new(0.0, 0.0, 5.0), DVec3::new(0.0, 0.0, -1.0));
    let sphere = BoundingSphere::new(DVec3::ZERO, 1.0);
    let (start, stop) = ray_sphere(&r, &sphere).unwrap();
    assert_approx!(start, 4.0, epsilon::EPSILON10);
    assert_approx!(stop, 6.0, epsilon::EPSILON10);
}

#[test]
fn test_ray_sphere_miss() {
    let r = Ray::new(DVec3::new(0.0, 5.0, 5.0), DVec3::new(0.0, 0.0, -1.0));
    let sphere = BoundingSphere::new(DVec3::ZERO, 1.0);
    assert!(ray_sphere(&r, &sphere).is_none());
}

#[test]
fn test_ray_sphere_inside() {
    let r = Ray::new(DVec3::ZERO, DVec3::new(1.0, 0.0, 0.0));
    let sphere = BoundingSphere::new(DVec3::ZERO, 5.0);
    let (start, stop) = ray_sphere(&r, &sphere).unwrap();
    assert_approx!(start, 0.0, epsilon::EPSILON10);
    assert_approx!(stop, 5.0, epsilon::EPSILON10);
}

// === 射线-三角形 相交 ===

#[test]
fn test_ray_triangle_hit() {
    let r = Ray::new(DVec3::new(0.25, 0.25, 1.0), DVec3::new(0.0, 0.0, -1.0));
    let v0 = DVec3::new(0.0, 0.0, 0.0);
    let v1 = DVec3::new(1.0, 0.0, 0.0);
    let v2 = DVec3::new(0.0, 1.0, 0.0);
    let hit = ray_triangle(&r, v0, v1, v2, false).unwrap();
    assert_approx!(hit.z, 0.0, epsilon::EPSILON10);
}

#[test]
fn test_ray_triangle_miss_outside() {
    let r = Ray::new(DVec3::new(2.0, 2.0, 1.0), DVec3::new(0.0, 0.0, -1.0));
    let v0 = DVec3::new(0.0, 0.0, 0.0);
    let v1 = DVec3::new(1.0, 0.0, 0.0);
    let v2 = DVec3::new(0.0, 1.0, 0.0);
    assert!(ray_triangle(&r, v0, v1, v2, false).is_none());
}

// === 射线-椭球 相交 ===

#[test]
fn test_ray_ellipsoid_hit() {
    let ellipsoid = Ellipsoid::WGS84;
    let r = Ray::new(
        DVec3::new(ellipsoid.radii().x * 2.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
    );
    let result = ray_ellipsoid(&r, &ellipsoid);
    assert!(result.is_some());
    let (t0, _t1) = result.unwrap();
    assert!(t0 > 0.0);
}

#[test]
fn test_ray_ellipsoid_miss() {
    let ellipsoid = Ellipsoid::WGS84;
    let r = Ray::new(
        DVec3::new(0.0, 0.0, ellipsoid.radii().z * 3.0),
        DVec3::new(1.0, 0.0, 0.0),
    );
    let result = ray_ellipsoid(&r, &ellipsoid);
    assert!(result.is_none());
}

// === 射线-AABB 相交 ===

#[test]
fn test_ray_aabb_hit() {
    let r = Ray::new(DVec3::new(0.0, 0.0, 5.0), DVec3::new(0.0, 0.0, -1.0));
    let aabb = AxisAlignedBoundingBox::new(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0));
    let t = ray_aabb(&r, &aabb).unwrap();
    assert_approx!(t, 4.0, epsilon::EPSILON10);
}

#[test]
fn test_ray_aabb_miss() {
    let r = Ray::new(DVec3::new(5.0, 5.0, 5.0), DVec3::new(1.0, 0.0, 0.0));
    let aabb = AxisAlignedBoundingBox::new(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0));
    assert!(ray_aabb(&r, &aabb).is_none());
}

// === 射线-OBB 相交 ===

#[test]
fn test_ray_obb_hit() {
    let r = Ray::new(DVec3::new(0.0, 0.0, 5.0), DVec3::new(0.0, 0.0, -1.0));
    let half_axes = glam::DMat3::from_diagonal(DVec3::new(1.0, 1.0, 1.0));
    let obb = OrientedBoundingBox::new(DVec3::ZERO, half_axes);
    let t = ray_obb(&r, &obb).unwrap();
    assert_approx!(t, 4.0, epsilon::EPSILON10);
}

#[test]
fn test_ray_obb_miss() {
    let r = Ray::new(DVec3::new(5.0, 5.0, 5.0), DVec3::new(1.0, 0.0, 0.0));
    let half_axes = glam::DMat3::from_diagonal(DVec3::new(1.0, 1.0, 1.0));
    let obb = OrientedBoundingBox::new(DVec3::ZERO, half_axes);
    assert!(ray_obb(&r, &obb).is_none());
}

// === 重心坐标 (Intersections2D) ===

#[test]
fn test_barycentric_at_vertex() {
    let (u, v, w) = compute_barycentric_coordinates(0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0);
    assert_approx!(u, 1.0, epsilon::EPSILON10);
    assert_approx!(v, 0.0, epsilon::EPSILON10);
    assert_approx!(w, 0.0, epsilon::EPSILON10);
}

#[test]
fn test_barycentric_at_centroid() {
    let (u, v, w) = compute_barycentric_coordinates(
        1.0 / 3.0,
        1.0 / 3.0,
        0.0, 0.0,
        1.0, 0.0,
        0.0, 1.0,
    );
    assert_approx!(u, 1.0 / 3.0, epsilon::EPSILON10);
    assert_approx!(v, 1.0 / 3.0, epsilon::EPSILON10);
    assert_approx!(w, 1.0 / 3.0, epsilon::EPSILON10);
}

// === Intersect 枚举 ===

#[test]
fn test_intersect_variants() {
    assert_ne!(Intersect::Outside, Intersect::Inside);
    assert_ne!(Intersect::Intersecting, Intersect::Outside);
    assert_eq!(Intersect::Inside, Intersect::Inside);
}
