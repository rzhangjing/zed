//! 偏心视锥规范 —— 参考自：
//! - Specs/Core/PerspectiveOffCenterFrustumSpec（31 个 it()）
//! - Specs/Core/OrthographicOffCenterFrustumSpec（30 个 it()）
//!
//! A 类测试：29 个（15 个透视 + 14 个正交）。
//!
//! 已省略（C 类）：所有 `throws` 测试（near/far 超出范围、left>right、
//! bottom>top、未定义参数、getPixelDimensions 参数校验——Rust 类型
//! 安全 / debug_assert）、`equals undefined`（JS undefined 处理），以及
//! `clone with result parameter`（JS result-param API）。
//!
//! 关于 "constructs" 的说明：CesiumJS 断言 `f.width === options.width` 且
//! `f.aspectRatio === options.aspectRatio`，但两者在偏心视锥上都是 `undefined`（它没有 width/aspectRatio 属性），因此这两个
//! 比较平凡地是 `undefined === undefined`。Rust 移植版本因此
//! 仅断言有意义的 `near`/`far` 值。

use cesium_geospatial::frustum::{OrthographicOffCenterFrustum, PerspectiveOffCenterFrustum};
use glam::{DMat4, DVec3};

const EPSILON15: f64 = 1e-15;
const EPSILON6: f64 = 1e-6;
const EPSILON4: f64 = 1e-4;
const EPSILON1: f64 = 1e-1;
const EPSILON2: f64 = 1e-2;
const EPSILON7: f64 = 1e-7;

fn assert_approx(a: f64, b: f64, eps: f64, msg: &str) {
    assert!(
        (a - b).abs() < eps,
        "{}: got {}, expected {} (eps={})",
        msg,
        a,
        b,
        eps
    );
}

/// 在 epsilon 内逐元素比较两个 DMat4。
fn assert_mat4_approx(actual: &DMat4, expected: &DMat4, eps: f64, msg: &str) {
    for col in 0..4 {
        for row in 0..4 {
            assert_approx(
                actual.col(col)[row],
                expected.col(col)[row],
                eps,
                &format!("{}[{}][{}]", msg, col, row),
            );
        }
    }
}

// ============================================================================
// PerspectiveOffCenterFrustum（来自 PerspectiveOffCenterFrustumSpec）
// 设置：left=-1, right=1, bottom=-1, top=1, near=1, far=2
// ============================================================================

fn make_perspective_off_center() -> PerspectiveOffCenterFrustum {
    PerspectiveOffCenterFrustum::from_bounds(-1.0, 1.0, -1.0, 1.0, 1.0, 2.0)
}

fn perspective_off_center_planes() -> [(DVec3, f64); 6] {
    let f = make_perspective_off_center();
    let cv = f.compute_culling_volume(DVec3::ZERO, -DVec3::Z, DVec3::Y);
    [
        (cv.planes[0].normal, cv.planes[0].distance),
        (cv.planes[1].normal, cv.planes[1].distance),
        (cv.planes[2].normal, cv.planes[2].distance),
        (cv.planes[3].normal, cv.planes[3].distance),
        (cv.planes[4].normal, cv.planes[4].distance),
        (cv.planes[5].normal, cv.planes[5].distance),
    ]
}

#[test]
fn test_perspective_off_center_constructs() {
    // 参考自：PerspectiveOffCenterFrustumSpec "constructs"
    // width/aspectRatio 在偏心视锥上是 undefined（平凡相等），
    // 因此仅断言 near/far。
    let f = PerspectiveOffCenterFrustum::from_bounds(-1.0, 2.0, -1.0, 5.0, 3.0, 4.0);
    assert_eq!(f.near, 3.0);
    assert_eq!(f.far, 4.0);
}

#[test]
fn test_perspective_off_center_default_constructs() {
    // 参考自： PerspectiveOffCenterFrustumSpec "default constructs"
    let f = PerspectiveOffCenterFrustum::new();
    assert!(f.left.is_none());
    assert!(f.right.is_none());
    assert!(f.top.is_none());
    assert!(f.bottom.is_none());
    assert_eq!(f.near, 1.0);
    assert_eq!(f.far, 500_000_000.0);
}

#[test]
fn test_perspective_off_center_left_plane() {
    // 参考自： "get frustum left plane"
    // 期望：Cartesian4(x, 0, -x, 0)，其中 x = 1/sqrt(2)
    let planes = perspective_off_center_planes();
    let (normal, distance) = planes[0];
    let x = 1.0 / 2.0_f64.sqrt();
    assert_approx(normal.x, x, EPSILON15, "left.x");
    assert_approx(normal.y, 0.0, EPSILON15, "left.y");
    assert_approx(normal.z, -x, EPSILON15, "left.z");
    assert_approx(distance, 0.0, EPSILON15, "left.d");
}

#[test]
fn test_perspective_off_center_right_plane() {
    // 参考自： "get frustum right plane"
    // 期望：Cartesian4(-x, 0, -x, 0)
    let planes = perspective_off_center_planes();
    let (normal, distance) = planes[1];
    let x = 1.0 / 2.0_f64.sqrt();
    assert_approx(normal.x, -x, EPSILON15, "right.x");
    assert_approx(normal.y, 0.0, EPSILON15, "right.y");
    assert_approx(normal.z, -x, EPSILON15, "right.z");
    assert_approx(distance, 0.0, EPSILON15, "right.d");
}

#[test]
fn test_perspective_off_center_bottom_plane() {
    // 参考自： "get frustum bottom plane"
    // 期望：Cartesian4(0, x, -x, 0)
    let planes = perspective_off_center_planes();
    let (normal, distance) = planes[2];
    let x = 1.0 / 2.0_f64.sqrt();
    assert_approx(normal.x, 0.0, EPSILON15, "bottom.x");
    assert_approx(normal.y, x, EPSILON15, "bottom.y");
    assert_approx(normal.z, -x, EPSILON15, "bottom.z");
    assert_approx(distance, 0.0, EPSILON15, "bottom.d");
}

#[test]
fn test_perspective_off_center_top_plane() {
    // 参考自： "get frustum top plane"
    // 期望：Cartesian4(0, -x, -x, 0)
    let planes = perspective_off_center_planes();
    let (normal, distance) = planes[3];
    let x = 1.0 / 2.0_f64.sqrt();
    assert_approx(normal.x, 0.0, EPSILON15, "top.x");
    assert_approx(normal.y, -x, EPSILON15, "top.y");
    assert_approx(normal.z, -x, EPSILON15, "top.z");
    assert_approx(distance, 0.0, EPSILON15, "top.d");
}

#[test]
fn test_perspective_off_center_near_plane() {
    // 参考自： "get frustum near plane"
    // 期望：Cartesian4(0, 0, -1, -1)
    let planes = perspective_off_center_planes();
    let (normal, distance) = planes[4];
    assert_approx(normal.x, 0.0, EPSILON15, "near.x");
    assert_approx(normal.y, 0.0, EPSILON15, "near.y");
    assert_approx(normal.z, -1.0, EPSILON15, "near.z");
    assert_approx(distance, -1.0, EPSILON15, "near.d");
}

#[test]
fn test_perspective_off_center_far_plane() {
    // 参考自： "get frustum far plane"
    // 期望：Cartesian4(0, 0, 1, 2)
    let planes = perspective_off_center_planes();
    let (normal, distance) = planes[5];
    assert_approx(normal.x, 0.0, EPSILON15, "far.x");
    assert_approx(normal.y, 0.0, EPSILON15, "far.y");
    assert_approx(normal.z, 1.0, EPSILON15, "far.z");
    assert_approx(distance, 2.0, EPSILON15, "far.d");
}

#[test]
fn test_perspective_off_center_projection_matrix() {
    // 参考自： "get perspective projection matrix"
    // 期望：Matrix4.computePerspectiveOffCenter(-1, 1, -1, 1, 1, 2)
    let f = make_perspective_off_center();
    let proj = f.projection_matrix();
    let expected = DMat4::from_cols_array(&[
        1.0, 0.0, 0.0, 0.0, // col0: 2*near/(right-left) = 1
        0.0, 1.0, 0.0, 0.0, // col1: 2*near/(top-bottom) = 1
        0.0, 0.0, -3.0, -1.0, // col2: -(far+near)/(far-near) = -3, -1
        0.0, 0.0, -4.0, 0.0, // col3: -2*far*near/(far-near) = -4
    ]);
    assert_mat4_approx(&proj, &expected, EPSILON6, "perspective proj");
}

#[test]
fn test_perspective_off_center_infinite_projection_matrix() {
    // 参考自： "get infinite perspective matrix"
    // 期望：Matrix4.computeInfinitePerspectiveOffCenter(-1, 1, -1, 1, 1)
    let f = make_perspective_off_center();
    let proj = f.infinite_projection_matrix();
    let expected = DMat4::from_cols_array(&[
        1.0, 0.0, 0.0, 0.0, // col0
        0.0, 1.0, 0.0, 0.0, // col1
        0.0, 0.0, -1.0, -1.0, // col2: -1, -1
        0.0, 0.0, -2.0, 0.0, // col3: -2*near = -2
    ]);
    assert_mat4_approx(&proj, &expected, EPSILON6, "infinite perspective proj");
}

#[test]
fn test_perspective_off_center_pixel_dimensions() {
    // 参考自： "get pixel dimensions"
    let f = make_perspective_off_center();
    let (pw, ph) = f.pixel_dimensions(1.0, 1.0, 1.0, 1.0);
    assert_eq!(pw, 2.0);
    assert_eq!(ph, 2.0);
}

#[test]
fn test_perspective_off_center_pixel_dimensions_with_pixel_ratio() {
    // 参考自： "get pixel dimensions with pixel ratio"
    let f = make_perspective_off_center();
    let (pw, ph) = f.pixel_dimensions(1.0, 1.0, 1.0, 2.0);
    assert_eq!(pw, 4.0);
    assert_eq!(ph, 4.0);
}

#[test]
fn test_perspective_off_center_equals() {
    // 参考自： "equals"
    let f = make_perspective_off_center();
    let f2 = PerspectiveOffCenterFrustum::from_bounds(-1.0, 1.0, -1.0, 1.0, 1.0, 2.0);
    assert!(f.equals(&f2));
}

#[test]
fn test_perspective_off_center_equals_epsilon() {
    // 参考自： "equals epsilon"
    let f = make_perspective_off_center();

    let f2 = PerspectiveOffCenterFrustum::from_bounds(-1.0, 1.0, -1.0, 1.0, 1.0, 2.0);
    assert!(f.equals_epsilon(&f2, EPSILON7, EPSILON7));

    let f3 = PerspectiveOffCenterFrustum::from_bounds(-1.0, 1.01, -1.0, 1.01, 1.01, 1.99);
    assert!(f.equals_epsilon(&f3, EPSILON1, EPSILON1));

    let f4 = PerspectiveOffCenterFrustum::from_bounds(-1.0, 1.1, -1.0, 1.0, 1.0, 2.0);
    assert!(!f.equals_epsilon(&f4, EPSILON2, EPSILON2));
}

#[test]
fn test_perspective_off_center_clone() {
    // 参考自： "clone"
    let f = make_perspective_off_center();
    let f2 = f; // Copy 语义参考 CesiumJS clone()
    assert!(f.equals(&f2));
}

// ============================================================================
// OrthographicOffCenterFrustum（来自 OrthographicOffCenterFrustumSpec）
// 设置：left=-1, right=1, bottom=-1, top=1, near=1, far=3
// ============================================================================

fn make_orthographic_off_center() -> OrthographicOffCenterFrustum {
    OrthographicOffCenterFrustum::from_bounds(-1.0, 1.0, -1.0, 1.0, 1.0, 3.0)
}

fn orthographic_off_center_planes() -> [(DVec3, f64); 6] {
    let f = make_orthographic_off_center();
    let cv = f.compute_culling_volume(DVec3::ZERO, -DVec3::Z, DVec3::Y);
    [
        (cv.planes[0].normal, cv.planes[0].distance),
        (cv.planes[1].normal, cv.planes[1].distance),
        (cv.planes[2].normal, cv.planes[2].distance),
        (cv.planes[3].normal, cv.planes[3].distance),
        (cv.planes[4].normal, cv.planes[4].distance),
        (cv.planes[5].normal, cv.planes[5].distance),
    ]
}

#[test]
fn test_orthographic_off_center_constructs() {
    // 参考自：OrthographicOffCenterFrustumSpec "constructs"
    // width/aspectRatio 在偏心视锥上是 undefined（平凡相等），
    // 因此仅断言 near/far。
    let f = OrthographicOffCenterFrustum::from_bounds(-1.0, 2.0, -1.0, 5.0, 3.0, 4.0);
    assert_eq!(f.near, 3.0);
    assert_eq!(f.far, 4.0);
}

#[test]
fn test_orthographic_off_center_default_constructs() {
    // 参考自： OrthographicOffCenterFrustumSpec "default constructs"
    let f = OrthographicOffCenterFrustum::new();
    assert!(f.left.is_none());
    assert!(f.right.is_none());
    assert!(f.top.is_none());
    assert!(f.bottom.is_none());
    assert_eq!(f.near, 1.0);
    assert_eq!(f.far, 500_000_000.0);
}

#[test]
fn test_orthographic_off_center_left_plane() {
    // 参考自： "get frustum left plane"
    // 期望：Cartesian4(1, 0, 0, 1)
    let planes = orthographic_off_center_planes();
    let (normal, distance) = planes[0];
    assert_approx(normal.x, 1.0, EPSILON4, "left.x");
    assert_approx(normal.y, 0.0, EPSILON4, "left.y");
    assert_approx(normal.z, 0.0, EPSILON4, "left.z");
    assert_approx(distance, 1.0, EPSILON4, "left.d");
}

#[test]
fn test_orthographic_off_center_right_plane() {
    // 参考自： "get frustum right plane"
    // 期望：Cartesian4(-1, 0, 0, 1)
    let planes = orthographic_off_center_planes();
    let (normal, distance) = planes[1];
    assert_approx(normal.x, -1.0, EPSILON4, "right.x");
    assert_approx(normal.y, 0.0, EPSILON4, "right.y");
    assert_approx(normal.z, 0.0, EPSILON4, "right.z");
    assert_approx(distance, 1.0, EPSILON4, "right.d");
}

#[test]
fn test_orthographic_off_center_bottom_plane() {
    // 参考自： "get frustum bottom plane"
    // 期望：Cartesian4(0, 1, 0, 1)
    let planes = orthographic_off_center_planes();
    let (normal, distance) = planes[2];
    assert_approx(normal.x, 0.0, EPSILON4, "bottom.x");
    assert_approx(normal.y, 1.0, EPSILON4, "bottom.y");
    assert_approx(normal.z, 0.0, EPSILON4, "bottom.z");
    assert_approx(distance, 1.0, EPSILON4, "bottom.d");
}

#[test]
fn test_orthographic_off_center_top_plane() {
    // 参考自： "get frustum top plane"
    // 期望：Cartesian4(0, -1, 0, 1)
    let planes = orthographic_off_center_planes();
    let (normal, distance) = planes[3];
    assert_approx(normal.x, 0.0, EPSILON4, "top.x");
    assert_approx(normal.y, -1.0, EPSILON4, "top.y");
    assert_approx(normal.z, 0.0, EPSILON4, "top.z");
    assert_approx(distance, 1.0, EPSILON4, "top.d");
}

#[test]
fn test_orthographic_off_center_near_plane() {
    // 参考自： "get frustum near plane"
    // 期望：Cartesian4(0, 0, -1, -1)
    let planes = orthographic_off_center_planes();
    let (normal, distance) = planes[4];
    assert_approx(normal.x, 0.0, EPSILON4, "near.x");
    assert_approx(normal.y, 0.0, EPSILON4, "near.y");
    assert_approx(normal.z, -1.0, EPSILON4, "near.z");
    assert_approx(distance, -1.0, EPSILON4, "near.d");
}

#[test]
fn test_orthographic_off_center_far_plane() {
    // 参考自： "get frustum far plane"
    // 期望：Cartesian4(0, 0, 1, 3)
    let planes = orthographic_off_center_planes();
    let (normal, distance) = planes[5];
    assert_approx(normal.x, 0.0, EPSILON4, "far.x");
    assert_approx(normal.y, 0.0, EPSILON4, "far.y");
    assert_approx(normal.z, 1.0, EPSILON4, "far.z");
    assert_approx(distance, 3.0, EPSILON4, "far.d");
}

#[test]
fn test_orthographic_off_center_projection_matrix() {
    // 参考自： "get orthographic projection matrix"
    // 期望：Matrix4.computeOrthographicOffCenter(-1, 1, -1, 1, 1, 3)
    let f = make_orthographic_off_center();
    let proj = f.projection_matrix();
    let expected = DMat4::from_cols_array(&[
        1.0, 0.0, 0.0, 0.0, // col0: 2/(right-left) = 1
        0.0, 1.0, 0.0, 0.0, // col1: 2/(top-bottom) = 1
        0.0, 0.0, -1.0, 0.0, // col2: -2/(far-near) = -1
        0.0, 0.0, -2.0, 1.0, // col3: tz = -(far+near)/(far-near) = -2
    ]);
    assert_mat4_approx(&proj, &expected, EPSILON6, "orthographic proj");
}

#[test]
fn test_orthographic_off_center_pixel_dimensions() {
    // 参考自： "get pixel dimensions"
    let f = make_orthographic_off_center();
    let (pw, ph) = f.pixel_dimensions(1.0, 1.0, 0.0, 1.0);
    assert_eq!(pw, 2.0);
    assert_eq!(ph, 2.0);
}

#[test]
fn test_orthographic_off_center_pixel_dimensions_with_pixel_ratio() {
    // 参考自： "get pixel dimensions with pixel ratio"
    let f = make_orthographic_off_center();
    let (pw, ph) = f.pixel_dimensions(1.0, 1.0, 0.0, 2.0);
    assert_eq!(pw, 4.0);
    assert_eq!(ph, 4.0);
}

#[test]
fn test_orthographic_off_center_equals() {
    // 参考自： "equals"
    let f = make_orthographic_off_center();
    let f2 = OrthographicOffCenterFrustum::from_bounds(-1.0, 1.0, -1.0, 1.0, 1.0, 3.0);
    assert!(f.equals(&f2));
}

#[test]
fn test_orthographic_off_center_equals_epsilon() {
    // 参考自： "equals epsilon"
    let f = make_orthographic_off_center();

    let f2 = OrthographicOffCenterFrustum::from_bounds(-1.0, 1.0, -1.0, 1.0, 1.0, 3.0);
    assert!(f.equals_epsilon(&f2, EPSILON7, EPSILON7));

    let f3 = OrthographicOffCenterFrustum::from_bounds(-0.99, 1.02, -1.05, 0.99, 1.01, 2.98);
    assert!(f.equals_epsilon(&f3, EPSILON1, EPSILON1));

    let f4 = OrthographicOffCenterFrustum::from_bounds(-1.02, 0.0, -1.005, 1.02, 1.1, 2.9);
    assert!(!f.equals_epsilon(&f4, EPSILON2, EPSILON2));
}

#[test]
fn test_orthographic_off_center_clone() {
    // 参考自： "clone"
    let f = make_orthographic_off_center();
    let f2 = f; // Copy 语义参考 CesiumJS clone()
    assert!(f.equals(&f2));
}
