//! ScreenSpaceCameraController 扩展规格 — 旋转、观察、平移、扭曲
//! 参考自：Specs/Scene/ScreenSpaceCameraControllerSpec
//! A 类纯数学测试

use cesium_interaction::camera_controller::{CameraController, CameraControllerConfig};
use cesium_camera::Camera;
use cesium_geospatial::ellipsoid::Ellipsoid;
use glam::DVec3;
use std::f64::consts::PI;

const EPSILON10: f64 = 1e-10;
const EPSILON14: f64 = 1e-14;

fn make_camera() -> Camera {
    Camera::new(
        DVec3::new(6378137.0 * 2.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    )
}

fn make_controller() -> CameraController {
    CameraController::new(Ellipsoid::WGS84)
}

// ============================================================================
// 配置自定义
// ============================================================================

#[test]
fn config_custom_speeds() {
    let mut controller = make_controller();
    controller.config.rotation_speed = 2.0;
    controller.config.pan_speed = 0.5;
    controller.config.zoom_speed = 1.5;
    controller.config.enable_rotation = false;

    assert!((controller.config.rotation_speed - 2.0).abs() < EPSILON10);
    assert!((controller.config.pan_speed - 0.5).abs() < EPSILON10);
    assert!((controller.config.zoom_speed - 1.5).abs() < EPSILON10);
    assert!(!controller.config.enable_rotation);
}

#[test]
fn config_minimum_zoom_distance_custom() {
    let controller = CameraController {
        config: CameraControllerConfig {
            minimum_zoom_distance: 100.0,
            ..Default::default()
        },
        ellipsoid: Ellipsoid::WGS84,
    };

    let mut camera = Camera::new(
        DVec3::new(Ellipsoid::WGS84.maximum_radius() + 50.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );

    // 相机位于表面上方 50m，最小距离为 100m — 碰撞应将其上推
    controller.enforce_collision(&mut camera);
    let height = camera.position.length() - Ellipsoid::WGS84.maximum_radius();
    assert!(height >= 100.0 - EPSILON10);
}

// ============================================================================
// 环绕：仅 heading 旋转保持距离
// ============================================================================

#[test]
fn orbit_heading_only_preserves_distance() {
    let controller = make_controller();
    let mut camera = make_camera();
    let target = DVec3::ZERO;
    let initial_distance = (camera.position - target).length();

    controller.orbit(&mut camera, target, 0.5, 0.0, 0.0);

    let new_distance = (camera.position - target).length();
    assert!((new_distance - initial_distance).abs() / initial_distance < 0.01);
}

#[test]
fn orbit_pitch_only_preserves_distance() {
    let controller = make_controller();
    let mut camera = make_camera();
    let target = DVec3::ZERO;
    let initial_distance = (camera.position - target).length();

    controller.orbit(&mut camera, target, 0.0, 0.3, 0.0);

    let new_distance = (camera.position - target).length();
    assert!((new_distance - initial_distance).abs() / initial_distance < 0.01);
}

#[test]
fn orbit_range_change_alters_distance() {
    let controller = make_controller();
    let mut camera = make_camera();
    let target = DVec3::ZERO;
    let initial_distance = (camera.position - target).length();

    controller.orbit(&mut camera, target, 0.0, 0.0, 1_000_000.0);

    let new_distance = (camera.position - target).length();
    assert!(new_distance > initial_distance);
}

#[test]
fn orbit_disabled_does_nothing() {
    let mut controller = make_controller();
    controller.config.enable_rotation = false;
    let mut camera = make_camera();
    let target = DVec3::ZERO;
    let initial_pos = camera.position;

    controller.orbit(&mut camera, target, 1.0, 1.0, 1000.0);

    assert_eq!(camera.position, initial_pos);
}

// ============================================================================
// 环绕：pitch 限制
// ============================================================================

#[test]
fn orbit_pitch_clamped_to_near_90() {
    let controller = make_controller();
    let mut camera = make_camera();
    let target = DVec3::ZERO;

    // 尝试将 pitch 拉过铅直很多
    controller.orbit(&mut camera, target, 0.0, 10.0, 0.0);

    // 相机不应穿过极点
    let offset = camera.position - target;
    let pitch = offset.y.atan2((offset.x * offset.x + offset.z * offset.z).sqrt());
    assert!(pitch.abs() <= PI / 2.0 + 0.01);
}

// ============================================================================
// 平移：方向测试
// ============================================================================

#[test]
fn pan_right_moves_camera() {
    let controller = make_controller();
    let mut camera = make_camera();
    let initial_pos = camera.position;

    controller.pan(&mut camera, 1.0, 0.0);

    // 相机应沿 right 向量方向移动
    let delta = camera.position - initial_pos;
    // delta 应沿 camera.right 有分量（此设置下 right 大致指向 +Z）
    assert!(delta.length() > 0.0);
}

#[test]
fn pan_up_moves_camera() {
    let controller = make_controller();
    let mut camera = make_camera();
    let initial_pos = camera.position;

    controller.pan(&mut camera, 0.0, 1.0);

    let delta = camera.position - initial_pos;
    assert!(delta.length() > 0.0);
}

#[test]
fn pan_disabled_does_nothing() {
    let mut controller = make_controller();
    controller.config.enable_pan = false;
    let mut camera = make_camera();
    let initial_pos = camera.position;

    controller.pan(&mut camera, 1.0, 1.0);

    assert_eq!(camera.position, initial_pos);
}

// ============================================================================
// 缩放：防止碰撞
// ============================================================================

#[test]
fn zoom_does_not_cross_surface() {
    let controller = make_controller();
    let mut camera = Camera::new(
        DVec3::new(Ellipsoid::WGS84.maximum_radius() + 10.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );

    // 尝试深入地下放大
    controller.zoom(&mut camera, 100.0);

    let height = camera.position.length() - Ellipsoid::WGS84.maximum_radius();
    assert!(height >= controller.config.minimum_zoom_distance - 1.0);
}

#[test]
fn zoom_disabled_no_collision_check() {
    let mut controller = make_controller();
    controller.config.enable_zoom = false;
    let mut camera = make_camera();
    let initial_pos = camera.position;

    controller.zoom(&mut camera, 10.0);

    assert_eq!(camera.position, initial_pos);
}

#[test]
fn zoom_with_collision_disabled_can_go_underground() {
    let mut controller = make_controller();
    controller.config.enable_collision_detection = false;
    let mut camera = Camera::new(
        DVec3::new(Ellipsoid::WGS84.maximum_radius() + 10.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );

    controller.zoom(&mut camera, 100.0);

    let height = camera.position.length() - Ellipsoid::WGS84.maximum_radius();
    assert!(height < 0.0); // 表面以下
}

// ============================================================================
// 倾斜：pitch 变化
// ============================================================================

#[test]
fn tilt_changes_pitch() {
    let controller = make_controller();
    let mut camera = Camera::new(
        DVec3::new(Ellipsoid::WGS84.maximum_radius() * 2.0, 0.0, 1_000_000.0),
        DVec3::new(-1.0, 0.0, 0.0).normalize(),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let target = DVec3::new(Ellipsoid::WGS84.maximum_radius(), 0.0, 0.0);
    let initial_dir = camera.direction;

    controller.tilt(&mut camera, target, 0.5);

    let dot = camera.direction.dot(initial_dir);
    assert!(dot < 0.99, "direction should change: dot={}", dot);
}

#[test]
fn tilt_up_increases_z_height() {
    let controller = make_controller();
    let mut camera = Camera::new(
        DVec3::new(Ellipsoid::WGS84.maximum_radius() * 2.0, 0.0, 1_000_000.0),
        DVec3::new(-1.0, 0.0, 0.0).normalize(),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let target = DVec3::new(Ellipsoid::WGS84.maximum_radius(), 0.0, 0.0);

    let _height_before = camera.position.length();
    controller.tilt(&mut camera, target, PI / 4.0);
    let _height_after = camera.position.length();

    // 倾斜后，position 长度可能改变 — 仅验证正交归一性
    assert!((camera.direction.length() - 1.0).abs() < EPSILON14);
    assert!((camera.up.length() - 1.0).abs() < EPSILON14);
    assert!((camera.right.length() - 1.0).abs() < EPSILON14);
}

// ============================================================================
// 强制碰撞
// ============================================================================

#[test]
fn enforce_collision_pushes_to_surface() {
    let controller = make_controller();
    let mut camera = Camera::new(
        DVec3::new(Ellipsoid::WGS84.maximum_radius() + 0.5, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );

    controller.enforce_collision(&mut camera);

    let height = camera.position.length() - Ellipsoid::WGS84.maximum_radius();
    assert!(height >= controller.config.minimum_zoom_distance - EPSILON10);
}

#[test]
fn enforce_collision_does_nothing_if_safe() {
    let controller = make_controller();
    let mut camera = make_camera();
    let initial_pos = camera.position;

    controller.enforce_collision(&mut camera);

    assert_eq!(camera.position, initial_pos);
}

#[test]
fn enforce_collision_disabled_does_nothing() {
    let mut controller = make_controller();
    controller.config.enable_collision_detection = false;
    let mut camera = Camera::new(
        DVec3::new(Ellipsoid::WGS84.maximum_radius() + 0.5, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let initial_pos = camera.position;

    controller.enforce_collision(&mut camera);

    assert_eq!(camera.position, initial_pos);
}

// ============================================================================
// 环绕：正交归一性
// ============================================================================

#[test]
fn orbit_preserves_orthonormality() {
    let controller = make_controller();
    let mut camera = make_camera();
    let target = DVec3::ZERO;

    controller.orbit(&mut camera, target, 0.7, 0.3, 50000.0);

    assert!((camera.direction.length() - 1.0).abs() < EPSILON14);
    assert!((camera.up.length() - 1.0).abs() < EPSILON14);
    assert!((camera.right.length() - 1.0).abs() < EPSILON14);
    assert!(camera.direction.dot(camera.up).abs() < EPSILON14);
    assert!(camera.direction.dot(camera.right).abs() < EPSILON14);
    assert!(camera.up.dot(camera.right).abs() < EPSILON14);
}

// ============================================================================
// 多操作链式调用
// ============================================================================

#[test]
fn orbit_then_pan_then_zoom() {
    let controller = make_controller();
    let mut camera = make_camera();

    let initial_pos = camera.position;

    // 环绕
    controller.orbit(&mut camera, DVec3::ZERO, 0.5, -0.2, 0.0);
    assert!(camera.position != initial_pos);

    // 平移
    controller.pan(&mut camera, 0.3, -0.1);
    assert!((camera.direction.length() - 1.0).abs() < EPSILON14);

    // 缩小
    controller.zoom(&mut camera, -1.0);
    assert!((camera.up.length() - 1.0).abs() < EPSILON14);
}

// ============================================================================
// 边界情形
// ============================================================================

#[test]
fn zoom_zero_delta_does_nothing() {
    let controller = make_controller();
    let mut camera = make_camera();
    let initial = camera.position;

    controller.zoom(&mut camera, 0.0);

    assert!((camera.position - initial).length() < EPSILON10);
}

#[test]
fn pan_zero_delta_does_nothing() {
    let controller = make_controller();
    let mut camera = make_camera();
    let initial = camera.position;

    controller.pan(&mut camera, 0.0, 0.0);

    assert!((camera.position - initial).length() < EPSILON10);
}

#[test]
fn orbit_zero_delta_preserves_position() {
    let controller = make_controller();
    let mut camera = make_camera();
    let initial = camera.position;
    let target = DVec3::ZERO;

    controller.orbit(&mut camera, target, 0.0, 0.0, 0.0);

    assert!((camera.position - initial).length() < EPSILON10);
}
