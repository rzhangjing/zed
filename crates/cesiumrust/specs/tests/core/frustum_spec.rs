//! Core/PerspectiveFrustumSpec、OrthographicFrustumSpec、CullingVolumeSpec
//! → Rust 集成测试

use cesium_geospatial::bounding::BoundingSphere;
use cesium_geospatial::frustum::{OrthographicFrustum, PerspectiveFrustum};
use cesium_geospatial::math_utils::to_radians;
use cesium_geospatial::ray::Intersect;
use cesium_specs::{assert_approx, epsilon};
use glam::DVec3;

// === PerspectiveFrustum ===

#[test]
fn test_perspective_frustum_new() {
    let frustum = PerspectiveFrustum::new(to_radians(60.0), 16.0 / 9.0, 0.1, 1000.0);
    assert_approx!(frustum.fov, to_radians(60.0), epsilon::EPSILON15);
    assert_approx!(frustum.aspect_ratio, 16.0 / 9.0, epsilon::EPSILON15);
    assert_approx!(frustum.near, 0.1, epsilon::EPSILON15);
    assert_approx!(frustum.far, 1000.0, epsilon::EPSILON15);
}

#[test]
fn test_perspective_frustum_fovy() {
    let frustum = PerspectiveFrustum::new(to_radians(60.0), 1.0, 1.0, 100.0);
    assert_approx!(frustum.fovy(), to_radians(60.0), epsilon::EPSILON15);
}

#[test]
fn test_perspective_frustum_fov_x() {
    let frustum = PerspectiveFrustum::new(to_radians(60.0), 16.0 / 9.0, 0.1, 1000.0);
    let fov_x = frustum.fov_x();
    // 对于宽纵横比，水平 FOV 应比垂直更宽
    assert!(fov_x > frustum.fov);
}

#[test]
fn test_perspective_projection_matrix_valid() {
    let frustum = PerspectiveFrustum::new(to_radians(60.0), 16.0 / 9.0, 0.1, 1000.0);
    let proj = frustum.projection_matrix();
    // 透视矩阵：w_axis.w = 0，z_axis.w = -1
    assert_approx!(proj.w_axis.w, 0.0, epsilon::EPSILON10);
    assert_approx!(proj.z_axis.w, -1.0, epsilon::EPSILON10);
}

#[test]
fn test_perspective_infinite_projection_matrix() {
    let frustum = PerspectiveFrustum::new(to_radians(60.0), 1.0, 1.0, 100.0);
    let proj = frustum.infinite_projection_matrix();
    // 仍应是透视矩阵
    assert_approx!(proj.z_axis.w, -1.0, epsilon::EPSILON10);
}

#[test]
fn test_perspective_pixel_dimensions() {
    let frustum = PerspectiveFrustum::new(to_radians(60.0), 1.0, 1.0, 1000.0);
    let (pw, ph) = frustum.pixel_dimensions(1024.0, 1024.0, 100.0, 1.0);
    assert!(pw > 0.0);
    assert!(ph > 0.0);
    // Aspect 1.0 → 方形像素
    assert_approx!(pw, ph, epsilon::EPSILON10);
}

#[test]
fn test_perspective_sse_denominator() {
    let frustum = PerspectiveFrustum::new(to_radians(60.0), 1.0, 1.0, 100.0);
    let denom = frustum.sse_denominator();
    assert!(denom > 0.0);
    assert_approx!(denom, 2.0 * to_radians(30.0).tan(), epsilon::EPSILON10);
}

// === CullingVolume ===

#[test]
fn test_culling_volume_sphere_inside() {
    let frustum = PerspectiveFrustum::new(to_radians(90.0), 1.0, 1.0, 100.0);
    let cv = frustum.compute_culling_volume(
        DVec3::ZERO,
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::Y,
    );
    // 球体完全位于视锥内
    let sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, -10.0), 1.0);
    assert_eq!(cv.visibility(&sphere), Intersect::Inside);
}

#[test]
fn test_culling_volume_sphere_outside_behind() {
    let frustum = PerspectiveFrustum::new(to_radians(60.0), 1.0, 1.0, 100.0);
    let cv = frustum.compute_culling_volume(
        DVec3::ZERO,
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::Y,
    );
    // 球体在相机后方
    let sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, 10.0), 1.0);
    assert_eq!(cv.visibility(&sphere), Intersect::Outside);
}

#[test]
fn test_culling_volume_sphere_outside_beyond_far() {
    let frustum = PerspectiveFrustum::new(to_radians(60.0), 1.0, 1.0, 100.0);
    let cv = frustum.compute_culling_volume(
        DVec3::ZERO,
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::Y,
    );
    // 球体在远平面之外
    let sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, -200.0), 1.0);
    assert_eq!(cv.visibility(&sphere), Intersect::Outside);
}

#[test]
fn test_culling_volume_sphere_intersecting() {
    let frustum = PerspectiveFrustum::new(to_radians(60.0), 1.0, 1.0, 100.0);
    let cv = frustum.compute_culling_volume(
        DVec3::ZERO,
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::Y,
    );
    // 球体跨越近平面
    let sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, -1.0), 2.0);
    let vis = cv.visibility(&sphere);
    assert!(vis == Intersect::Intersecting || vis == Intersect::Inside);
}

#[test]
fn test_culling_volume_plane_mask() {
    let frustum = PerspectiveFrustum::new(to_radians(90.0), 1.0, 1.0, 100.0);
    let cv = frustum.compute_culling_volume(
        DVec3::ZERO,
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::Y,
    );
    let sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, -10.0), 1.0);
    let mask = cv.visibility_with_plane_mask(&sphere, 0);
    // 不应为 u32::MAX（其含义是位于外部）
    assert_ne!(mask, u32::MAX);
}

// === OrthographicFrustum ===

#[test]
fn test_orthographic_frustum_new() {
    let frustum = OrthographicFrustum::new(10.0, 2.0, 0.1, 100.0);
    assert_approx!(frustum.width, 10.0, epsilon::EPSILON15);
    assert_approx!(frustum.aspect_ratio, 2.0, epsilon::EPSILON15);
    assert_approx!(frustum.near, 0.1, epsilon::EPSILON15);
    assert_approx!(frustum.far, 100.0, epsilon::EPSILON15);
}

#[test]
fn test_orthographic_frustum_height() {
    let frustum = OrthographicFrustum::new(10.0, 2.0, 0.1, 100.0);
    assert_approx!(frustum.height(), 5.0, epsilon::EPSILON10);
}

#[test]
fn test_orthographic_projection_matrix_valid() {
    let frustum = OrthographicFrustum::new(10.0, 1.0, 0.1, 100.0);
    let proj = frustum.projection_matrix();
    // 正交：w 行 = (0, 0, 0, 1)
    assert_approx!(proj.x_axis.w, 0.0, epsilon::EPSILON10);
    assert_approx!(proj.y_axis.w, 0.0, epsilon::EPSILON10);
    assert_approx!(proj.z_axis.w, 0.0, epsilon::EPSILON10);
    assert_approx!(proj.w_axis.w, 1.0, epsilon::EPSILON10);
}

#[test]
fn test_orthographic_pixel_dimensions() {
    let frustum = OrthographicFrustum::new(10.0, 1.0, 0.1, 100.0);
    let (pw, ph) = frustum.pixel_dimensions(100.0, 100.0, 50.0, 1.0);
    assert_approx!(pw, 0.1, epsilon::EPSILON10);
    assert_approx!(ph, 0.1, epsilon::EPSILON10);
}

#[test]
fn test_orthographic_culling_volume() {
    let frustum = OrthographicFrustum::new(10.0, 1.0, 1.0, 100.0);
    let cv = frustum.compute_culling_volume(
        DVec3::ZERO,
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::Y,
    );
    // 球体位于正交视体内
    let sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, -10.0), 1.0);
    assert_eq!(cv.visibility(&sphere), Intersect::Inside);
}
