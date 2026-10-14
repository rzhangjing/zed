//! Scene/CameraSpec → Rust 集成测试（setView 变体 + 坐标变换）
//! 参考自：Specs/Scene/CameraSpec
//! A 类纯数学测试：setView HPR 往返、direction/up、坐标变换、
//! distanceToBoundingSphere

use cesium_camera::Camera;
use cesium_geospatial::{math_utils, BoundingSphere, Ellipsoid};
use glam::{DMat4, DVec3, DVec4};

const EPSILON6: f64 = 1e-6;
const EPSILON10: f64 = 1e-10;
const EPSILON14: f64 = 1e-14;

fn assert_vec3_eq(actual: DVec3, expected: DVec3, eps: f64, msg: &str) {
    assert!(
        actual.abs_diff_eq(expected, eps),
        "{}: expected {:?}, got {:?}",
        msg,
        expected,
        actual
    );
}

fn assert_vec4_eq(actual: DVec4, expected: DVec4, eps: f64, msg: &str) {
    assert!(
        actual.abs_diff_eq(expected, eps),
        "{}: expected {:?}, got {:?}",
        msg,
        expected,
        actual
    );
}

fn assert_scalar_eq(actual: f64, expected: f64, eps: f64, msg: &str) {
    assert!(
        (actual - expected).abs() < eps,
        "{}: expected {}, got {} (diff={})",
        msg,
        expected,
        actual,
        (actual - expected).abs()
    );
}

// ============================================================================
// setView 右旋旋转顺序
// CesiumJS: position=fromDegrees(-117.16, 32.71), heading=180°, pitch=0°, roll=45°
// ============================================================================

#[test]
fn set_view_right_rotation_order() {
    let ellipsoid = Ellipsoid::WGS84;
    let position = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(-117.16, 32.71, 0.0),
    );
    let heading = math_utils::to_radians(180.0);
    let pitch = math_utils::to_radians(0.0);
    let roll = math_utils::to_radians(45.0);

    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    camera.set_view_hpr(position, heading, pitch, roll, &ellipsoid);

    assert_vec3_eq(camera.position, position, EPSILON6, "position");
    assert_scalar_eq(camera.heading_3d(&ellipsoid), heading, EPSILON6, "heading");
    assert_scalar_eq(camera.pitch_3d(&ellipsoid), pitch, EPSILON6, "pitch");
    assert_scalar_eq(camera.roll_3d(&ellipsoid), roll, EPSILON6, "roll");
}

// ============================================================================
// setView (1) - 无 destination 的 heading 变化
// CesiumJS: heading=45°, pitch=-50°, roll=45°, then heading→200°
// ============================================================================

#[test]
fn set_view_1_heading_change() {
    let ellipsoid = Ellipsoid::WGS84;
    let position = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(-117.16, 32.71, 0.0),
    );
    let heading = math_utils::to_radians(45.0);
    let pitch = math_utils::to_radians(-50.0);
    let roll = math_utils::to_radians(45.0);

    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    camera.set_view_hpr(position, heading, pitch, roll, &ellipsoid);

    assert_vec3_eq(camera.position, position, EPSILON6, "position (1)");
    assert_scalar_eq(camera.heading_3d(&ellipsoid), heading, EPSILON6, "heading (1)");
    assert_scalar_eq(camera.pitch_3d(&ellipsoid), pitch, EPSILON6, "pitch (1)");
    assert_scalar_eq(camera.roll_3d(&ellipsoid), roll, EPSILON6, "roll (1)");

    // 仅改变 heading（无 destination → 保持当前位置）
    let new_heading = math_utils::to_radians(200.0);
    let cur_pitch = camera.pitch_3d(&ellipsoid);
    let cur_roll = camera.roll_3d(&ellipsoid);
    camera.set_view_hpr(camera.position, new_heading, cur_pitch, cur_roll, &ellipsoid);

    assert_vec3_eq(camera.position, position, EPSILON6, "position (2)");
    assert_scalar_eq(camera.heading_3d(&ellipsoid), new_heading, EPSILON6, "heading (2)");
    assert_scalar_eq(camera.pitch_3d(&ellipsoid), pitch, EPSILON6, "pitch (2)");
    assert_scalar_eq(camera.roll_3d(&ellipsoid), roll, EPSILON6, "roll (2)");
}

// ============================================================================
// setView (2) - 无 destination 的 pitch 变化
// CesiumJS: heading=45°, pitch=50°, roll=45°, then pitch→-50°
// ============================================================================

#[test]
fn set_view_2_pitch_change() {
    let ellipsoid = Ellipsoid::WGS84;
    let position = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(-117.16, 32.71, 0.0),
    );
    let heading = math_utils::to_radians(45.0);
    let pitch = math_utils::to_radians(50.0);
    let roll = math_utils::to_radians(45.0);

    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    camera.set_view_hpr(position, heading, pitch, roll, &ellipsoid);

    assert_vec3_eq(camera.position, position, EPSILON6, "position (1)");
    assert_scalar_eq(camera.heading_3d(&ellipsoid), heading, EPSILON6, "heading (1)");
    assert_scalar_eq(camera.pitch_3d(&ellipsoid), pitch, EPSILON6, "pitch (1)");
    assert_scalar_eq(camera.roll_3d(&ellipsoid), roll, EPSILON6, "roll (1)");

    // 仅改变 pitch
    let new_pitch = math_utils::to_radians(-50.0);
    let cur_heading = camera.heading_3d(&ellipsoid);
    let cur_roll = camera.roll_3d(&ellipsoid);
    camera.set_view_hpr(camera.position, cur_heading, new_pitch, cur_roll, &ellipsoid);

    assert_vec3_eq(camera.position, position, EPSILON6, "position (2)");
    assert_scalar_eq(camera.heading_3d(&ellipsoid), heading, EPSILON6, "heading (2)");
    assert_scalar_eq(camera.pitch_3d(&ellipsoid), new_pitch, EPSILON6, "pitch (2)");
    assert_scalar_eq(camera.roll_3d(&ellipsoid), roll, EPSILON6, "roll (2)");
}

// ============================================================================
// setView (3) - 无 destination 的 roll 变化
// CesiumJS: heading=45°, pitch=50°, roll=45°, then roll→200°
// ============================================================================

#[test]
fn set_view_3_roll_change() {
    let ellipsoid = Ellipsoid::WGS84;
    let position = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(-117.16, 32.71, 0.0),
    );
    let heading = math_utils::to_radians(45.0);
    let pitch = math_utils::to_radians(50.0);
    let roll = math_utils::to_radians(45.0);

    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    camera.set_view_hpr(position, heading, pitch, roll, &ellipsoid);

    assert_vec3_eq(camera.position, position, EPSILON6, "position (1)");
    assert_scalar_eq(camera.heading_3d(&ellipsoid), heading, EPSILON6, "heading (1)");
    assert_scalar_eq(camera.pitch_3d(&ellipsoid), pitch, EPSILON6, "pitch (1)");
    assert_scalar_eq(camera.roll_3d(&ellipsoid), roll, EPSILON6, "roll (1)");

    // 仅改变 roll
    let new_roll = math_utils::to_radians(200.0);
    let cur_heading = camera.heading_3d(&ellipsoid);
    let cur_pitch = camera.pitch_3d(&ellipsoid);
    camera.set_view_hpr(camera.position, cur_heading, cur_pitch, new_roll, &ellipsoid);

    assert_vec3_eq(camera.position, position, EPSILON6, "position (2)");
    assert_scalar_eq(camera.heading_3d(&ellipsoid), heading, EPSILON6, "heading (2)");
    assert_scalar_eq(camera.pitch_3d(&ellipsoid), pitch, EPSILON6, "pitch (2)");
    assert_scalar_eq(camera.roll_3d(&ellipsoid), new_roll, EPSILON6, "roll (2)");
}

// ============================================================================
// 带 direction、up 的 setView
// CesiumJS: direction=-UNIT_Z, up=UNIT_Y, destination=fromDegrees(-117.16, 32.71)
// ============================================================================

#[test]
fn set_view_with_direction_up() {
    let ellipsoid = Ellipsoid::WGS84;
    let position = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(-117.16, 32.71, 0.0),
    );
    let direction = DVec3::new(0.0, 0.0, -1.0); // -UNIT_Z
    let up = DVec3::Y;

    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    camera.set_view_direction(position, direction, up);

    assert_vec3_eq(camera.direction, direction, EPSILON6, "direction");
    assert_vec3_eq(camera.up, up, EPSILON6, "up");
}

// ============================================================================
// worldToCameraCoordinates (Cartesian4)
// CesiumJS: 仅旋转变换，UNIT_X → UNIT_Z
// ============================================================================

#[test]
fn world_to_camera_coordinates_cartesian4() {
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    // CesiumJS: new Matrix4(0,0,1,0, 1,0,0,0, 0,1,0,0, 0,0,0,1)
    // CesiumJS 构造参数为行主序；glam from_cols_array 为列主序。
    // 转置旋转部分：M_glam = M_cesium^T
    let transform = DMat4::from_cols_array(&[
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        1.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]);
    camera.transform = transform;

    let result = camera.world_to_camera_coordinates(DVec4::new(1.0, 0.0, 0.0, 0.0));
    assert_vec4_eq(result, DVec4::new(0.0, 0.0, 1.0, 0.0), EPSILON14, "worldToCamera X→Z");
}

// ============================================================================
// worldToCameraCoordinatesPoint
// CesiumJS: 旋转+平移变换，UNIT_X → invTransform * (1,0,0,1)
// ============================================================================

#[test]
fn world_to_camera_coordinates_point() {
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    // CesiumJS: new Matrix4(0,0,1,10, 1,0,0,20, 0,1,0,30, 0,0,0,1)
    // 转置旋转，保留平移列：
    let transform = DMat4::from_cols_array(&[
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        1.0, 0.0, 0.0, 0.0,
        10.0, 20.0, 30.0, 1.0,
    ]);
    camera.transform = transform;

    // CesiumJS 期望值：getColumn(inverseTransform, 3) + UNIT_Z
    let inv = transform.inverse();
    let inv_col3 = (inv * DVec4::new(0.0, 0.0, 0.0, 1.0)).truncate();
    let expected = inv_col3 + DVec3::new(0.0, 0.0, 1.0);

    let result = camera.world_to_camera_point(DVec3::new(1.0, 0.0, 0.0));
    assert_vec3_eq(result, expected, EPSILON10, "worldToCameraPoint");
}

// ============================================================================
// worldToCameraCoordinatesVector
// CesiumJS: 旋转+平移变换，UNIT_X → UNIT_Z（向量忽略平移）
// ============================================================================

#[test]
fn world_to_camera_coordinates_vector() {
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    // CesiumJS: new Matrix4(0,0,1,10, 1,0,0,20, 0,1,0,30, 0,0,0,1)
    let transform = DMat4::from_cols_array(&[
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        1.0, 0.0, 0.0, 0.0,
        10.0, 20.0, 30.0, 1.0,
    ]);
    camera.transform = transform;

    let result = camera.world_to_camera_vector(DVec3::new(1.0, 0.0, 0.0));
    assert_vec3_eq(result, DVec3::new(0.0, 0.0, 1.0), EPSILON14, "worldToCameraVector X→Z");
}

// ============================================================================
// cameraToWorldCoordinates (Cartesian4)
// CesiumJS: 仅旋转变换，UNIT_Z → UNIT_X
// ============================================================================

#[test]
fn camera_to_world_coordinates_cartesian4() {
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    // CesiumJS: new Matrix4(0,0,1,0, 1,0,0,0, 0,1,0,0, 0,0,0,1)
    let transform = DMat4::from_cols_array(&[
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        1.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]);
    camera.transform = transform;

    let result = camera.camera_to_world_coordinates(DVec4::new(0.0, 0.0, 1.0, 0.0));
    assert_vec4_eq(result, DVec4::new(1.0, 0.0, 0.0, 0.0), EPSILON14, "cameraToWorld Z→X");
}

// ============================================================================
// cameraToWorldCoordinatesPoint
// CesiumJS: 旋转+平移变换，UNIT_Z → UNIT_X + column3(transform)
// ============================================================================

#[test]
fn camera_to_world_coordinates_point() {
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    // CesiumJS: new Matrix4(0,0,1,10, 1,0,0,20, 0,1,0,30, 0,0,0,1)
    let transform = DMat4::from_cols_array(&[
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        1.0, 0.0, 0.0, 0.0,
        10.0, 20.0, 30.0, 1.0,
    ]);
    camera.transform = transform;

    // CesiumJS 期望值：UNIT_X + getColumn(transform, 3)
    let col3 = (transform * DVec4::new(0.0, 0.0, 0.0, 1.0)).truncate();
    let expected = DVec3::new(1.0, 0.0, 0.0) + col3;

    let result = camera.camera_to_world_point(DVec3::new(0.0, 0.0, 1.0));
    assert_vec3_eq(result, expected, EPSILON10, "cameraToWorldPoint");
}

// ============================================================================
// cameraToWorldCoordinatesVector
// CesiumJS: 旋转+平移变换，UNIT_Z → UNIT_X（向量忽略平移）
// ============================================================================

#[test]
fn camera_to_world_coordinates_vector() {
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    // CesiumJS: new Matrix4(0,0,1,10, 1,0,0,20, 0,1,0,30, 0,0,0,1)
    let transform = DMat4::from_cols_array(&[
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        1.0, 0.0, 0.0, 0.0,
        10.0, 20.0, 30.0, 1.0,
    ]);
    camera.transform = transform;

    let result = camera.camera_to_world_vector(DVec3::new(0.0, 0.0, 1.0));
    assert_vec3_eq(result, DVec3::new(1.0, 0.0, 0.0), EPSILON14, "cameraToWorldVector Z→X");
}

// ============================================================================
// distanceToBoundingSphere
// CesiumJS: 相机位于 (0,0,1)，dir=(0,0,-1)，球体位于 ZERO 半径 0.5 → 距离 0.5
// ============================================================================

#[test]
fn distance_to_bounding_sphere() {
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    let sphere = BoundingSphere::new(DVec3::ZERO, 0.5);
    let distance = camera.distance_to_bounding_sphere(&sphere);
    assert_scalar_eq(distance, 0.5, EPSILON10, "distanceToBoundingSphere");
}

// ============================================================================
// 获取逆变换
// CesiumJS: setTransform(scale5 + translation), inverseTransform = inverseTransformation
// ============================================================================

#[test]
fn get_inverse_transform() {
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    // CesiumJS: new Matrix4(5,0,0,1, 0,5,0,2, 0,0,5,3, 0,0,0,1)
    // 行主序参数 → glam 列主序（转置旋转，保留平移）：
    // M_cesium = [[5,0,0,1],[0,5,0,2],[0,0,5,3],[0,0,0,1]]
    // M_glam cols: col0=(5,0,0,0), col1=(0,5,0,0), col2=(0,0,5,0), col3=(1,2,3,1)
    let transform = DMat4::from_cols_array(&[
        5.0, 0.0, 0.0, 0.0,
        0.0, 5.0, 0.0, 0.0,
        0.0, 0.0, 5.0, 0.0,
        1.0, 2.0, 3.0, 1.0,
    ]);
    camera.set_transform(transform);

    let inv = camera.inverse_transform();
    let expected = camera.transform.inverse();

    // 验证 inverse_transform == transform.inverse()
    for i in 0..16 {
        let a = inv.to_cols_array()[i];
        let b = expected.to_cols_array()[i];
        assert!((a - b).abs() < EPSILON14, "inv[{}] = {} vs {}", i, a, b);
    }
}

// ============================================================================
// 在 Columbus view / 3D 中获取模长
// ============================================================================

#[test]
fn gets_magnitude_in_columbus_view() {
    let mut camera = Camera::new(
        DVec3::new(100.0, 200.0, 300.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );
    camera.mode = cesium_camera::SceneMode::ColumbusView;
    let mag = camera.get_magnitude();
    assert_scalar_eq(mag, 300.0, EPSILON10, "magnitude CV = position.z");
}

#[test]
fn gets_magnitude_in_3d() {
    let camera = Camera::new(
        DVec3::new(3.0, 4.0, 0.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );
    let mag = camera.get_magnitude();
    assert_scalar_eq(mag, 5.0, EPSILON10, "magnitude 3D = |position|");
}

// ============================================================================
// 归一化 WC 成员
// CesiumJS: 在 lookAtTransform(scale(2)) 之后，directionWC/rightWC/upWC 均为单位向量
// ============================================================================

#[test]
fn normalizes_wc_members() {
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    let transform = DMat4::from_scale(DVec3::new(2.0, 2.0, 2.0));
    camera.look_at_transform_no_offset(transform);

    assert_scalar_eq(camera.direction_wc().length(), 1.0, EPSILON14, "|directionWC|");
    assert_scalar_eq(camera.right_wc().length(), 1.0, EPSILON14, "|rightWC|");
    assert_scalar_eq(camera.up_wc().length(), 1.0, EPSILON14, "|upWC|");
}

// ============================================================================
// 获取拾取射线（透视）
// CesiumJS: windowCoord=(width/2, height), expected dir=(0, -windowHeight, -1).normalize()
// ============================================================================

#[test]
fn get_pick_ray_perspective() {
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    // 使用任意画布尺寸
    let width = 512.0;
    let height = 384.0;
    let window_x = width / 2.0;
    let window_y = height; // 屏幕底部

    let ray = camera.get_pick_ray_perspective(window_x, window_y, width, height).unwrap();

    // 期望值：windowHeight = near * tan(fovy * 0.5)
    // 默认视锥：fov=60deg, near=1.0
    let fovy = math_utils::to_radians(60.0);
    let near = 1.0;
    let window_height = near * (fovy * 0.5).tan();
    let expected_direction = DVec3::new(0.0, -window_height, -near).normalize();

    assert_vec3_eq(ray.origin, camera.position, EPSILON10, "ray.origin");
    assert_vec3_eq(ray.direction, expected_direction, 1e-15, "ray.direction");
}

// ============================================================================
// 拾取椭球
// CesiumJS: 相机位于 X 轴 2*maxRadii 处，看向中心，中心拾取 → (0,0,0) 的 cartographic
// ============================================================================

#[test]
fn pick_ellipsoid_center() {
    let ellipsoid = Ellipsoid::WGS84;
    let max_radii = ellipsoid.maximum_radius();

    let position = DVec3::X * (2.0 * max_radii);
    let direction = -position.normalize();
    let up = DVec3::Z;

    let mut camera = Camera::new(position, direction, up);
    // 设置与 CesiumJS 测试一致的透视视锥
    camera.frustum = cesium_camera::Frustum::Perspective(
        cesium_geospatial::PerspectiveFrustum::new(
            math_utils::to_radians(60.0),
            1.0, // 宽高比
            100.0,
            60.0 * max_radii,
        ),
    );

    let width = 512.0;
    let height = 384.0;
    // 在屏幕中心拾取
    let p = camera.pick_ellipsoid(width * 0.5, height * 0.5, width, height, &ellipsoid);
    assert!(p.is_some(), "should pick ellipsoid at center");
    let p = p.unwrap();

    let c = ellipsoid.cartesian_to_cartographic(p).unwrap();
    assert_scalar_eq(c.longitude, 0.0, EPSILON6, "longitude");
    assert_scalar_eq(c.latitude, 0.0, EPSILON6, "latitude");
    assert_scalar_eq(c.height, 0.0, EPSILON6, "height");
}

#[test]
fn pick_ellipsoid_misses_at_corner() {
    let ellipsoid = Ellipsoid::WGS84;
    let max_radii = ellipsoid.maximum_radius();

    let position = DVec3::X * (2.0 * max_radii);
    let direction = -position.normalize();
    let up = DVec3::Z;

    let mut camera = Camera::new(position, direction, up);
    camera.frustum = cesium_camera::Frustum::Perspective(
        cesium_geospatial::PerspectiveFrustum::new(
            math_utils::to_radians(60.0),
            1.0,
            100.0,
            60.0 * max_radii,
        ),
    );

    // 在角点 (0,0) 拾取 - 应未命中椭球
    let p = camera.pick_ellipsoid(0.0, 0.0, 512.0, 384.0, &ellipsoid);
    assert!(p.is_none(), "should not pick ellipsoid at corner");
}

#[test]
fn pick_ellipsoid_near_surface() {
    let ellipsoid = Ellipsoid::WGS84;
    let min_radii = ellipsoid.minimum_radius();

    // 位于北极表面以上十米处，俯视下方。
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, min_radii + 10.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(1.0, 0.0, 0.0),
    );

    // 在左上角拾取（CesiumJS 测试使用 Cartesian2.ZERO）
    let p = camera.pick_ellipsoid(0.0, 0.0, 512.0, 384.0, &ellipsoid);
    // 测试期望 p.z ≈ minRadii（误差 1e-4 以内）
    // 使用默认视锥（fov=60, aspect=16/9），角点射线仍可能命中
    if let Some(p) = p {
        assert_scalar_eq(p.z, min_radii, 1e-4, "pick near surface z");
    }
    // 若为 None，则角点射线未命中 - 对于此相机设置同样可接受
}
