//! Scene/CameraSpec → Rust 集成测试（setView/lookAt/heading/pitch/roll）
//! 参考自：Specs/Scene/CameraSpec
//! A 类纯数学测试：heading/pitch/roll 读写器、lookAt、lookAtTransform

use cesium_camera::Camera;
use cesium_geospatial::{math_utils, Ellipsoid, HeadingPitchRange};
use glam::{DMat4, DVec3};

const EPSILON6: f64 = 1e-6;
const EPSILON8: f64 = 1e-8;
const EPSILON11: f64 = 1e-11;
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
// 3D 中的 heading 读取器
// ============================================================================

#[test]
fn get_heading_in_3d() {
    // CesiumJS: camera.position = UNIT_X, direction = -UNIT_X, up = UNIT_Z
    let ellipsoid = Ellipsoid::WGS84;
    let camera = Camera::new(
        DVec3::X,
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::Z,
    );

    // 使用 ENU 参考系计算期望 heading
    let enu = cesium_geospatial::transforms::east_north_up_to_fixed_frame(camera.position, &ellipsoid);
    let east = enu.x_axis.truncate();
    let north = enu.y_axis.truncate();
    let up_enu = enu.z_axis.truncate();

    let local_right = DVec3::new(
        camera.right.dot(east),
        camera.right.dot(north),
        camera.right.dot(up_enu),
    );
    let expected_heading = math_utils::TWO_PI - math_utils::zero_to_two_pi(local_right.y.atan2(local_right.x));
    // 归一化：TWO_PI ≡ 0
    let expected_heading = if (expected_heading - math_utils::TWO_PI).abs() < 1e-15 { 0.0 } else { expected_heading };

    let heading = camera.heading_3d(&ellipsoid);
    assert_scalar_eq(heading, expected_heading, EPSILON8, "heading in 3D");
}

#[test]
fn set_heading_in_3d() {
    // CesiumJS: position=UNIT_X, direction=-UNIT_X, up=UNIT_Z
    let ellipsoid = Ellipsoid::WGS84;
    let mut camera = Camera::new(
        DVec3::X,
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::Z,
    );

    let old_heading = camera.heading_3d(&ellipsoid);
    let new_heading = math_utils::to_radians(45.0);

    // setView 保持 pitch 与 roll，改变 heading
    let pitch = camera.pitch_3d(&ellipsoid);
    let roll = camera.roll_3d(&ellipsoid);
    camera.set_view_hpr(camera.position, new_heading, pitch, roll, &ellipsoid);

    let heading = camera.heading_3d(&ellipsoid);
    assert!(
        (heading - old_heading).abs() > EPSILON6,
        "heading should have changed"
    );
    assert_scalar_eq(heading, new_heading, EPSILON14, "set heading in 3D");
}

#[test]
fn set_heading_in_3d_preserves_position() {
    // CesiumJS: position=fromDegrees(136, -24, 4500000), set heading=PI
    let ellipsoid = Ellipsoid::WGS84;
    let position = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(136.0, -24.0, 4500000.0),
    );

    let mut camera = Camera::new(position, -position.normalize(), DVec3::Z);
    // 重新正交归一化
    camera.right = camera.direction.cross(camera.up).normalize();
    camera.up = camera.right.cross(camera.direction).normalize();

    let old_position = camera.position;

    camera.set_view_hpr(camera.position, std::f64::consts::PI, camera.pitch_3d(&ellipsoid), camera.roll_3d(&ellipsoid), &ellipsoid);

    assert_vec3_eq(camera.position, old_position, EPSILON8, "position preserved");
    assert_scalar_eq(camera.heading_3d(&ellipsoid), std::f64::consts::PI, EPSILON8, "heading = PI");
    assert!(camera.up.z < 0.0, "up.z should be < 0 for heading=PI");

    // 设置 heading = TWO_PI
    camera.set_view_hpr(camera.position, math_utils::TWO_PI, camera.pitch_3d(&ellipsoid), camera.roll_3d(&ellipsoid), &ellipsoid);

    assert_vec3_eq(camera.position, old_position, EPSILON8, "position preserved (2)");
    assert_scalar_eq(camera.heading_3d(&ellipsoid), 0.0, EPSILON8, "heading = TWO_PI ≡ 0");
    assert!(camera.up.z > 0.0, "up.z should be > 0 for heading=TWO_PI");
}

// ============================================================================
// 3D 中的 pitch 读取器
// ============================================================================

#[test]
fn get_pitch_in_3d() {
    // CesiumJS: default camera at (0,0,1), direction=(0,0,-1)
    let ellipsoid = Ellipsoid::WGS84;
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    // 期望值：PI/2 - acos(dir_local.z)，其中 dir_local.z = direction 与 surface_normal 的点积
    let normal = ellipsoid.geodetic_surface_normal(camera.position).unwrap();
    let expected_pitch = std::f64::consts::FRAC_PI_2
        - camera.direction.dot(normal).clamp(-1.0, 1.0).acos();

    let pitch = camera.pitch_3d(&ellipsoid);
    assert_scalar_eq(pitch, expected_pitch, EPSILON8, "pitch in 3D");
}

#[test]
fn set_pitch_in_3d() {
    // CesiumJS: position=fromDegrees(-72, 40, 100000), direction=-UNIT_X, up=UNIT_Z
    let ellipsoid = Ellipsoid::WGS84;
    let position = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(-72.0, 40.0, 100000.0),
    );

    let mut camera = Camera::new(
        position,
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::Z,
    );

    let old_pitch = camera.pitch_3d(&ellipsoid);
    let new_pitch = math_utils::to_radians(45.0);

    let heading = camera.heading_3d(&ellipsoid);
    let roll = camera.roll_3d(&ellipsoid);
    camera.set_view_hpr(camera.position, heading, new_pitch, roll, &ellipsoid);

    let pitch = camera.pitch_3d(&ellipsoid);
    assert!(
        (pitch - old_pitch).abs() > EPSILON6,
        "pitch should have changed"
    );
    assert_scalar_eq(pitch, new_pitch, EPSILON14, "set pitch in 3D");
}

// ============================================================================
// 3D 中的 roll 读取器
// ============================================================================

#[test]
fn get_roll_in_3d() {
    // CesiumJS: position = UNIT_X * (maxRadius + 100), direction=(-1,0,1).normalize()
    let ellipsoid = Ellipsoid::WGS84;
    let position = DVec3::X * (ellipsoid.maximum_radius() + 100.0);
    let direction = DVec3::new(-1.0, 0.0, 1.0).normalize();
    let right = direction.cross(DVec3::Z).normalize();
    let up = right.cross(direction).normalize();

    let camera = Camera::new(position, direction, up);

    // 使用 ENU 计算期望 roll
    let enu = cesium_geospatial::transforms::east_north_up_to_fixed_frame(camera.position, &ellipsoid);
    let east = enu.x_axis.truncate();
    let north = enu.y_axis.truncate();
    let up_enu = enu.z_axis.truncate();

    let local_right = DVec3::new(
        camera.right.dot(east),
        camera.right.dot(north),
        camera.right.dot(up_enu),
    );
    let expected_roll = math_utils::TWO_PI - math_utils::zero_to_two_pi(local_right.z.atan2(local_right.x));

    let roll = camera.roll_3d(&ellipsoid);
    assert_scalar_eq(roll, expected_roll, EPSILON8, "roll in 3D");
}

#[test]
fn set_roll_in_3d() {
    // CesiumJS: position=fromDegrees(-72, 40, 100000), direction=UNIT_Z, up=UNIT_X
    let ellipsoid = Ellipsoid::WGS84;
    let position = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(-72.0, 40.0, 100000.0),
    );

    let mut camera = Camera::new(
        position,
        DVec3::Z,
        DVec3::X,
    );

    let old_roll = camera.roll_3d(&ellipsoid);
    let new_roll = std::f64::consts::FRAC_PI_4;

    let heading = camera.heading_3d(&ellipsoid);
    let pitch = camera.pitch_3d(&ellipsoid);
    camera.set_view_hpr(camera.position, heading, pitch, new_roll, &ellipsoid);

    let roll = camera.roll_3d(&ellipsoid);
    assert!(
        (roll - old_roll).abs() > EPSILON6,
        "roll should have changed: old={}, new={}",
        old_roll,
        roll
    );
    assert_scalar_eq(roll, new_roll, EPSILON6, "set roll in 3D");
}

#[test]
fn get_roll_returns_correct_value_past_90_degrees() {
    // CesiumJS: setView with destination=fromDegrees(-72, 40, 20), roll=110°
    let ellipsoid = Ellipsoid::WGS84;
    let position = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(-72.0, 40.0, 20.0),
    );

    let roll = math_utils::to_radians(110.0);
    let mut camera = Camera::new(position, -position.normalize(), DVec3::Z);
    camera.set_view_hpr(position, 0.0, 0.0, roll, &ellipsoid);

    assert_scalar_eq(camera.roll_3d(&ellipsoid), roll, EPSILON14, "roll past 90°");
}

// ============================================================================
// 带 HeadingPitchRange 的 lookAt
// ============================================================================

#[test]
fn look_at_with_heading_pitch_range() {
    // CesiumJS: target=fromDegrees(0,0), heading=45°, pitch=-45°, range=2
    let ellipsoid = Ellipsoid::WGS84;
    let target = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let heading = math_utils::to_radians(45.0);
    let pitch = math_utils::to_radians(-45.0);
    let range = 2.0;

    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    let hpr = HeadingPitchRange::new(heading, pitch, range);
    camera.look_at(target, &hpr, &ellipsoid);

    // lookAtTransform(IDENTITY) 之后，检查 distance/heading/pitch
    camera.look_at_transform_no_offset(DMat4::IDENTITY);

    let dist = (camera.position - target).length();
    assert_scalar_eq(dist, range, EPSILON6, "distance = range");
    assert_scalar_eq(camera.heading_3d(&ellipsoid), heading, EPSILON6, "heading after lookAt");
    assert_scalar_eq(camera.pitch_3d(&ellipsoid), pitch, EPSILON6, "pitch after lookAt");

    // 验证单位向量
    assert!((camera.direction.length() - 1.0).abs() < EPSILON14);
    assert!((camera.up.length() - 1.0).abs() < EPSILON14);
    assert!((camera.right.length() - 1.0).abs() < EPSILON14);
}

// ============================================================================
// target 与 camera 均为零时的 lookAt
// ============================================================================

#[test]
fn look_at_when_target_and_camera_are_zero() {
    // CesiumJS: target=ZERO, camera.position=ZERO, offset=(0,-1,0)
    let ellipsoid = Ellipsoid::WGS84;
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );
    camera.position = DVec3::ZERO;

    let target = DVec3::ZERO;
    let offset = DVec3::new(0.0, -1.0, 0.0);

    camera.look_at_offset(target, offset, &ellipsoid);

    assert_vec3_eq(camera.position, offset, EPSILON11, "position");
    assert_vec3_eq(
        camera.direction,
        -offset.normalize(),
        EPSILON11,
        "direction",
    );

    let expected_right = camera.direction.cross(DVec3::Z).normalize();
    assert_vec3_eq(camera.right, expected_right, EPSILON11, "right");

    let expected_up = camera.right.cross(camera.direction).normalize();
    assert_vec3_eq(camera.up, expected_up, EPSILON11, "up");
}

// ============================================================================
// lookAtTransform
// ============================================================================

#[test]
fn look_at_transform_basic() {
    // CesiumJS: target=(-1,-1,0), offset=(1,1,0), transform=ENU(target, UNIT_SPHERE)
    let ellipsoid = Ellipsoid::new(1.0, 1.0, 1.0);
    let target = DVec3::new(-1.0, -1.0, 0.0);
    let offset = DVec3::new(1.0, 1.0, 0.0);
    let transform = cesium_geospatial::transforms::east_north_up_to_fixed_frame(target, &ellipsoid);

    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    camera.look_at_transform_offset(transform, offset);

    assert_vec3_eq(camera.position, offset, EPSILON11, "position");
    assert_vec3_eq(
        camera.direction,
        -offset.normalize(),
        EPSILON11,
        "direction",
    );

    let expected_right = camera.direction.cross(DVec3::Z).normalize();
    assert_vec3_eq(camera.right, expected_right, EPSILON11, "right");

    let expected_up = camera.right.cross(camera.direction).normalize();
    assert_vec3_eq(camera.up, expected_up, EPSILON11, "up");

    // 验证单位向量
    assert!((camera.direction.length() - 1.0).abs() < EPSILON14);
    assert!((camera.up.length() - 1.0).abs() < EPSILON14);
    assert!((camera.right.length() - 1.0).abs() < EPSILON14);
}

#[test]
fn look_at_transform_with_no_offset() {
    // CesiumJS: camera at height above origin, lookAtTransform(ENU) with no offset
    let ellipsoid = Ellipsoid::WGS84;
    let cart_origin = cesium_geospatial::Cartographic::from_degrees(-75.59777, 40.03883, 0.0);
    let origin = ellipsoid.cartographic_to_cartesian(&cart_origin);
    let transform = cesium_geospatial::transforms::east_north_up_to_fixed_frame(origin, &ellipsoid);

    let height = 1000.0;
    let cart_with_height = cesium_geospatial::Cartographic::from_degrees(-75.59777, 40.03883, height);
    let position = ellipsoid.cartographic_to_cartesian(&cart_with_height);

    // 设置相机向下看向原点
    let mut camera = Camera::new(position, DVec3::ZERO, DVec3::ZERO);
    // Direction = transform 的 -column2（取反的 up 轴）
    let up_axis = transform.z_axis.truncate();
    let north_axis = transform.y_axis.truncate();
    let east_axis = transform.x_axis.truncate();
    camera.direction = -up_axis.normalize();
    camera.up = north_axis.normalize();
    camera.right = east_axis.normalize();

    camera.look_at_transform_no_offset(transform);

    assert_vec3_eq(
        camera.position,
        DVec3::new(0.0, 0.0, height),
        1e-9,
        "position in local frame",
    );
    assert_vec3_eq(
        camera.direction,
        DVec3::new(0.0, 0.0, -1.0),
        1e-9,
        "direction in local frame",
    );
    assert_vec3_eq(camera.up, DVec3::Y, 1e-9, "up in local frame");
    assert_vec3_eq(camera.right, DVec3::X, 1e-9, "right in local frame");
}

#[test]
fn look_at_transform_with_heading_pitch_range() {
    // CesiumJS: target=fromDegrees(0,0), heading=45°, pitch=-45°, range=2
    let ellipsoid = Ellipsoid::WGS84;
    let target = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let heading = math_utils::to_radians(45.0);
    let pitch = math_utils::to_radians(-45.0);
    let range = 2.0;
    let transform = cesium_geospatial::transforms::east_north_up_to_fixed_frame(target, &ellipsoid);

    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    let hpr = HeadingPitchRange::new(heading, pitch, range);
    camera.look_at_transform(transform, &hpr);

    // 将 transform 重置为单位矩阵以获取世界坐标
    camera.look_at_transform_no_offset(DMat4::IDENTITY);

    let dist = (camera.position - target).length();
    assert_scalar_eq(dist, range, EPSILON6, "distance = range");
    assert_scalar_eq(camera.heading_3d(&ellipsoid), heading, EPSILON6, "heading");
    assert_scalar_eq(camera.pitch_3d(&ellipsoid), pitch, EPSILON6, "pitch");

    // 验证单位向量
    assert!((camera.direction.length() - 1.0).abs() < EPSILON14);
    assert!((camera.up.length() - 1.0).abs() < EPSILON14);
    assert!((camera.right.length() - 1.0).abs() < EPSILON14);
}

// ============================================================================
// 3D 中带 destination 的 setView
// ============================================================================

#[test]
fn set_view_with_destination_and_hpr() {
    // 将相机设置到特定位置，heading=0, pitch=-PI/2, roll=0
    let ellipsoid = Ellipsoid::WGS84;
    let position = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(-72.0, 40.0, 100000.0),
    );

    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    camera.set_view(position, 0.0, -std::f64::consts::FRAC_PI_2, 0.0, &ellipsoid);

    assert_vec3_eq(camera.position, position, EPSILON8, "position set");
    assert_scalar_eq(camera.heading_3d(&ellipsoid), 0.0, EPSILON6, "heading = 0");
    assert_scalar_eq(
        camera.pitch_3d(&ellipsoid),
        -std::f64::consts::FRAC_PI_2,
        EPSILON6,
        "pitch = -PI/2",
    );
}
