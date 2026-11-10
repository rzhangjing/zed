//! 相机拾取与像素大小测试。
//! 参考自 CesiumJS CameraSpec：getPickRay 正射、getPixelSize、
//! distanceToBoundingSphere、带 lookAt 的 getPickRay。

use cesium_camera::{Camera, Frustum};
use cesium_geospatial::{
    BoundingSphere, Ellipsoid, HeadingPitchRange, OrthographicFrustum, PerspectiveFrustum,
};
use glam::DVec3;

const EPSILON14: f64 = 1e-14;
const EPSILON10: f64 = 1e-10;
const EPSILON9: f64 = 1e-9;
const EPSILON6: f64 = 1e-6;

fn default_camera() -> Camera {
    // 与 CesiumJS 测试设置相同：position=(0,0,1), dir=(0,0,-1), up=(0,1,0)
    Camera::new(DVec3::Z, -DVec3::Z, DVec3::Y)
}

// ============================================================================
// getPickRay 正射
// ============================================================================

#[test]
fn test_get_pick_ray_orthographic_3d() {
    // 参考自: "get pick ray orthographic in 3D"
    let mut camera = default_camera();
    camera.frustum = Frustum::Orthographic(OrthographicFrustum::new(20.0, 1.0, 1.0, 21.0));

    let canvas_width = 512.0;
    let canvas_height = 384.0;
    let window_x = (3.0 / 5.0) * canvas_width; // 307.2
    let window_y = (1.0 - 3.0 / 5.0) * canvas_height; // 153.6

    let ray = camera
        .get_pick_ray_orthographic(window_x, window_y, canvas_width, canvas_height)
        .expect("should produce a ray");

    // x_ndc = (2/512)*307.2 - 1 = 0.2; x = 0.2 * (20*0.5) = 2.0
    // y_ndc = (2/384)*(384-153.6) - 1 = 0.2; y = 0.2 * (20*0.5) = 2.0
    // Camera: pos=(0,0,1), right=(1,0,0), up=(0,1,0), dir=(0,0,-1)
    // origin = (0,0,1) + (1,0,0)*2 + (0,1,0)*2 = (2, 2, 1)
    let expected_origin = DVec3::new(2.0, 2.0, 1.0);
    assert!(
        (ray.origin - expected_origin).length() < EPSILON14,
        "origin: {:?}, expected: {:?}",
        ray.origin,
        expected_origin
    );

    // direction = camera directionWC = (0, 0, -1)
    let expected_dir = DVec3::new(0.0, 0.0, -1.0);
    assert!(
        (ray.direction - expected_dir).length() < EPSILON14,
        "direction: {:?}, expected: {:?}",
        ray.direction,
        expected_dir
    );
}

#[test]
fn test_get_pick_ray_orthographic_center() {
    // 屏幕中心的拾取射线应有 origin = 相机位置
    let mut camera = default_camera();
    camera.frustum = Frustum::Orthographic(OrthographicFrustum::new(20.0, 1.0, 1.0, 21.0));

    let canvas_width = 512.0;
    let canvas_height = 384.0;
    let window_x = canvas_width / 2.0;
    let window_y = canvas_height / 2.0;

    let ray = camera
        .get_pick_ray_orthographic(window_x, window_y, canvas_width, canvas_height)
        .expect("should produce a ray");

    // 在中心：x_ndc=0, y_ndc=0 → origin = 相机位置
    let expected_origin = DVec3::new(0.0, 0.0, 1.0);
    assert!(
        (ray.origin - expected_origin).length() < EPSILON14,
        "origin: {:?}, expected: {:?}",
        ray.origin,
        expected_origin
    );
    assert!(
        (ray.direction - DVec3::new(0.0, 0.0, -1.0)).length() < EPSILON14,
        "direction: {:?}",
        ray.direction
    );
}

#[test]
fn test_get_pick_ray_orthographic_returns_none_for_perspective() {
    let camera = default_camera(); // 默认为透视
    let result = camera.get_pick_ray_orthographic(100.0, 100.0, 512.0, 384.0);
    assert!(result.is_none());
}

#[test]
fn test_get_pick_ray_dispatches_perspective() {
    let camera = default_camera(); // 透视视锥
    let ray = camera.get_pick_ray(256.0, 192.0, 512.0, 384.0);
    assert!(ray.is_some());
    // 在屏幕中心，direction 应为相机 direction
    let ray = ray.unwrap();
    assert!(
        (ray.direction - DVec3::new(0.0, 0.0, -1.0)).length() < EPSILON10,
        "direction: {:?}",
        ray.direction
    );
}

#[test]
fn test_get_pick_ray_dispatches_orthographic() {
    let mut camera = default_camera();
    camera.frustum = Frustum::Orthographic(OrthographicFrustum::new(20.0, 1.0, 1.0, 21.0));
    let ray = camera.get_pick_ray(256.0, 192.0, 512.0, 384.0);
    assert!(ray.is_some());
    // 在中心，正射射线 origin = 相机位置，direction = 相机 direction
    let ray = ray.unwrap();
    assert!(
        (ray.origin - DVec3::new(0.0, 0.0, 1.0)).length() < EPSILON14,
        "origin: {:?}",
        ray.origin
    );
}

// ============================================================================
// getPixelSize
// ============================================================================

#[test]
fn test_get_pixel_size_perspective() {
    // 参考自: "getPixelSize"
    let camera = default_camera();
    // 默认透视：fov=60°, aspect=16/9, near=1, far=500M

    let sphere = BoundingSphere::new(DVec3::ZERO, 0.5);
    let drawing_buffer_width = 1024.0;
    let drawing_buffer_height = 768.0;
    let pixel_ratio = 1.0;

    let pixel_size = camera.get_pixel_size(&sphere, drawing_buffer_width, drawing_buffer_height, pixel_ratio);

    // distance = |proj((0,0,-1) onto (0,0,-1))| - 0.5 = 1.0 - 0.5 = 0.5
    let distance = camera.distance_to_bounding_sphere(&sphere);
    assert!((distance - 0.5).abs() < EPSILON10, "distance: {}", distance);

    // 期望值：max(pixelWidth, pixelHeight)
    // tan_phi = tan(30°) ≈ 0.5774
    // tan_theta = (16/9) * tan(30°) ≈ 1.0264
    // pixelWidth = 2*1*0.5*1.0264/1024 ≈ 0.001003
    // pixelHeight = 2*1*0.5*0.5774/768 ≈ 0.000752
    // max = pixelWidth
    let expected = match &camera.frustum {
        Frustum::Perspective(f) => {
            let (pw, ph) = f.pixel_dimensions(drawing_buffer_width, drawing_buffer_height, distance, pixel_ratio);
            pw.max(ph)
        }
        _ => unreachable!(),
    };
    assert!(
        (pixel_size - expected).abs() < EPSILON14,
        "pixel_size: {}, expected: {}",
        pixel_size,
        expected
    );
    assert!(pixel_size > 0.0);
}

#[test]
fn test_get_pixel_size_orthographic() {
    let mut camera = default_camera();
    camera.frustum = Frustum::Orthographic(OrthographicFrustum::new(10.0, 1.0, 1.0, 100.0));

    let sphere = BoundingSphere::new(DVec3::ZERO, 0.5);
    let pixel_size = camera.get_pixel_size(&sphere, 100.0, 100.0, 1.0);

    // Orthographic: pixelWidth = 1.0 * 10 / 100 = 0.1
    //               pixelHeight = 1.0 * 10 / 100 = 0.1 (aspect=1 → height=width)
    // max = 0.1
    assert!(
        (pixel_size - 0.1).abs() < EPSILON10,
        "pixel_size: {}, expected: 0.1",
        pixel_size
    );
}

#[test]
fn test_get_pixel_size_with_pixel_ratio() {
    let mut camera = default_camera();
    camera.frustum = Frustum::Orthographic(OrthographicFrustum::new(10.0, 1.0, 1.0, 100.0));

    let sphere = BoundingSphere::new(DVec3::ZERO, 0.5);
    let pixel_size = camera.get_pixel_size(&sphere, 100.0, 100.0, 2.0);

    // pixel_ratio=2: pixelWidth = 2.0 * 10 / 100 = 0.2
    assert!(
        (pixel_size - 0.2).abs() < EPSILON10,
        "pixel_size: {}, expected: 0.2",
        pixel_size
    );
}

// ============================================================================
// distanceToBoundingSphere
// ============================================================================

#[test]
fn test_distance_to_bounding_sphere() {
    // 参考自: "distanceToBoundingSphere"
    let camera = default_camera();
    // Camera at (0,0,1), direction (0,0,-1)
    // Sphere at origin, radius 0.5
    let sphere = BoundingSphere::new(DVec3::ZERO, 0.5);
    let distance = camera.distance_to_bounding_sphere(&sphere);
    // to_center = (0,0,1), proj onto dir (0,0,-1) = dot((0,0,1),(0,0,-1)) = -1
    // proj_vec = (0,0,-1)*(-1) = (0,0,1), |proj_vec| = 1.0
    // distance = max(1.0 - 0.5, 0) = 0.5
    assert!(
        (distance - 0.5).abs() < EPSILON10,
        "distance: {}, expected: 0.5",
        distance
    );
}

#[test]
fn test_distance_to_bounding_sphere_behind() {
    let camera = default_camera();
    // 球体在相机后方
    let sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, 5.0), 0.5);
    let distance = camera.distance_to_bounding_sphere(&sphere);
    // to_center = sphere.center - camera.pos = (0,0,4)
    // signed dist = (0,0,4) · (0,0,-1) - 0.5 = -4 - 0.5 = -4.5
    // max(0, -4.5) = 0
    assert!(
        distance.abs() < EPSILON10,
        "distance: {}, expected: 0 (behind camera)",
        distance
    );
}

// ============================================================================
// 带 lookAt 的 getPickRay
// ============================================================================

#[test]
fn test_get_pick_ray_with_look_at_perspective_3d() {
    // 参考自: "get pick ray with lookAt perspective in 3D"
    let ellipsoid = Ellipsoid::WGS84;
    let mut camera = default_camera();

    // lookAt target=(6378137,0,0)，带 Cartesian3 偏移 (0,-1,0)
    let target = DVec3::new(6378137.0, 0.0, 0.0);
    let offset = DVec3::new(0.0, -1.0, 0.0);
    camera.look_at_offset(target, offset, &ellipsoid);

    let canvas_width = 512.0;
    let canvas_height = 384.0;
    // 在底部中心拾取
    let window_x = canvas_width / 2.0;
    let window_y = canvas_height;

    let ray = camera
        .get_pick_ray_perspective(window_x, window_y, canvas_width, canvas_height)
        .expect("should produce a ray");

    // lookAt 之后：
    // ENU at (6378137,0,0): east=(0,1,0), north=(0,0,1), up=(1,0,0)
    // transform = ENU matrix
    // local position = offset = (0,-1,0)
    // local direction = -offset.normalize() = (0,1,0)
    // local right = dir×Z = (1,0,0) [since (0,1,0)×(0,0,1)=(1,0,0)]
    // local up = right×dir = (0,0,1)
    //
    // positionWC = transform * (0,-1,0,1) = target + R*(0,-1,0)
    //   R*(0,-1,0) = 0*east + (-1)*north + 0*up = (0,0,-1)
    //   positionWC = (6378137, 0, -1)
    // directionWC = R*(0,1,0) = north = (0,0,1)
    // rightWC = R*(1,0,0) = east = (0,1,0)
    // upWC = R*(0,0,1) = up_enu = (1,0,0)
    //
    // Perspective: fov=60°, aspect=16/9, near=1
    // tan_phi = tan(30°), tan_theta = (16/9)*tan(30°)
    // x_ndc = (2/512)*256 - 1 = 0
    // y_ndc = (2/384)*(384-384) - 1 = -1
    //
    // dir = dirWC*1 + rightWC*(0) + upWC*(-1*1*tan_phi)
    //     = (0,0,1) + (-tan_phi, 0, 0)
    //     = (-tan_phi, 0, 1)
    // normalize: length = sqrt(tan²_phi + 1)

    let tan_phi = (std::f64::consts::PI / 6.0).tan(); // tan(30°)
    let expected_dir = DVec3::new(-tan_phi, 0.0, 1.0).normalize();

    // 验证 origin
    let expected_origin = DVec3::new(6378137.0, 0.0, -1.0);
    assert!(
        (ray.origin - expected_origin).length() < EPSILON6,
        "origin: {:?}, expected: {:?}",
        ray.origin,
        expected_origin
    );

    // 验证 direction
    assert!(
        (ray.direction - expected_dir).length() < EPSILON9,
        "direction: {:?}, expected: {:?}",
        ray.direction,
        expected_dir
    );
}

#[test]
fn test_get_pick_ray_with_look_at_center() {
    // lookAt 后屏幕中心的拾取射线应为相机 direction
    let ellipsoid = Ellipsoid::WGS84;
    let mut camera = default_camera();

    let target = DVec3::new(6378137.0, 0.0, 0.0);
    let offset = DVec3::new(0.0, -1.0, 0.0);
    camera.look_at_offset(target, offset, &ellipsoid);

    let canvas_width = 512.0;
    let canvas_height = 384.0;
    let ray = camera
        .get_pick_ray_perspective(canvas_width / 2.0, canvas_height / 2.0, canvas_width, canvas_height)
        .expect("should produce a ray");

    // 在中心：x_ndc=0, y_ndc=0 → direction = normalize(dirWC * near) = dirWC
    let expected_dir = camera.direction_wc();
    assert!(
        (ray.direction - expected_dir).length() < EPSILON10,
        "direction: {:?}, expected: {:?}",
        ray.direction,
        expected_dir
    );
}

#[test]
fn test_get_pick_ray_with_look_at_hpr() {
    // 使用带 HeadingPitchRange 的 lookAt，然后获取拾取射线
    let ellipsoid = Ellipsoid::WGS84;
    let mut camera = default_camera();

    let target = DVec3::new(6378137.0, 0.0, 0.0);
    let hpr = HeadingPitchRange::new(0.0, -std::f64::consts::FRAC_PI_2, 100.0);
    camera.look_at(target, &hpr, &ellipsoid);

    let canvas_width = 512.0;
    let canvas_height = 384.0;
    let ray = camera
        .get_pick_ray(canvas_width / 2.0, canvas_height / 2.0, canvas_width, canvas_height)
        .expect("should produce a ray");

    // 在中心，拾取射线 direction = 相机 directionWC
    let expected_dir = camera.direction_wc();
    assert!(
        (ray.direction - expected_dir).length() < EPSILON10,
        "direction: {:?}, expected: {:?}",
        ray.direction,
        expected_dir
    );

    // origin 应为相机 positionWC
    let expected_origin = camera.position_wc();
    assert!(
        (ray.origin - expected_origin).length() < EPSILON10,
        "origin: {:?}, expected: {:?}",
        ray.origin,
        expected_origin
    );
}

// ============================================================================
// 视锥 pixel_dimensions 验证
// ============================================================================

#[test]
fn test_perspective_pixel_dimensions_formula() {
    // 验证公式与 CesiumJS PerspectiveOffCenterFrustum.getPixelDimensions 一致
    let frustum = PerspectiveFrustum::new(
        cesium_geospatial::math_utils::to_radians(60.0),
        16.0 / 9.0,
        1.0,
        500_000_000.0,
    );

    let width = 1024.0;
    let height = 768.0;
    let distance = 1000.0;
    let pixel_ratio = 1.0;

    let (pw, ph) = frustum.pixel_dimensions(width, height, distance, pixel_ratio);

    // CesiumJS 公式：
    // top = near * tan(fovy/2) = 1 * tan(30°)
    // pixelHeight = 2 * pixelRatio * distance * (top/near) / drawingBufferHeight
    //             = 2 * 1 * 1000 * tan(30°) / 768
    let tan_30 = (std::f64::consts::PI / 6.0).tan();
    let expected_ph = 2.0 * 1000.0 * tan_30 / 768.0;
    let expected_pw = 2.0 * 1000.0 * tan_30 * (16.0 / 9.0) / 1024.0;

    assert!(
        (ph - expected_ph).abs() < EPSILON10,
        "pixel_height: {}, expected: {}",
        ph,
        expected_ph
    );
    assert!(
        (pw - expected_pw).abs() < EPSILON10,
        "pixel_width: {}, expected: {}",
        pw,
        expected_pw
    );
}

#[test]
fn test_orthographic_pixel_dimensions_formula() {
    // 验证公式与 CesiumJS OrthographicOffCenterFrustum.getPixelDimensions 一致
    let frustum = OrthographicFrustum::new(20.0, 2.0, 1.0, 100.0);

    let (pw, ph) = frustum.pixel_dimensions(1024.0, 768.0, 50.0, 1.0);

    // CesiumJS 公式：
    // frustumWidth = right - left = width = 20
    // frustumHeight = top - bottom = height = width/aspect = 10
    // pixelWidth = pixelRatio * frustumWidth / drawingBufferWidth = 1*20/1024
    // pixelHeight = pixelRatio * frustumHeight / drawingBufferHeight = 1*10/768
    let expected_pw = 20.0 / 1024.0;
    let expected_ph = 10.0 / 768.0;

    assert!(
        (pw - expected_pw).abs() < EPSILON10,
        "pixel_width: {}, expected: {}",
        pw,
        expected_pw
    );
    assert!(
        (ph - expected_ph).abs() < EPSILON10,
        "pixel_height: {}, expected: {}",
        ph,
        expected_ph
    );
}
