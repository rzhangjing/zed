//! Scene/ScreenSpaceCameraController → Rust 集成测试。
//!
//! 参考 CesiumJS：
//! - Scene/ScreenSpaceCameraController（环绕、平移、缩放、倾斜、碰撞）
//!
//! A 类测试：CameraController orbit/pan/zoom/tilt/enforce_collision、
//! CameraControllerConfig 默认值、rotate_around_axis（Rodrigues）。
//! 省略的 C 类：DOM 事件、指针事件、触摸手势、canvas。

use cesium_interaction::camera_controller::{CameraController, CameraControllerConfig};
use cesium_camera::Camera;
use cesium_geospatial::ellipsoid::Ellipsoid;
use glam::DVec3;

fn make_camera() -> Camera {
    // 相机位于 +X 轴上 2 倍地球半径处，朝原点看
    Camera::new(
        DVec3::new(6378137.0 * 2.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    )
}

fn make_controller() -> CameraController {
    CameraController::new(Ellipsoid::WGS84)
}

// === 配置 ===

#[test]
fn config_defaults() {
    let config = CameraControllerConfig::default();
    assert!((config.minimum_zoom_distance - 1.0).abs() < 1e-10);
    assert!(config.maximum_zoom_distance.is_infinite());
    assert!((config.rotation_speed - 1.0).abs() < 1e-10);
    assert!((config.pan_speed - 1.0).abs() < 1e-10);
    assert!((config.zoom_speed - 1.0).abs() < 1e-10);
    assert!(config.enable_rotation);
    assert!(config.enable_pan);
    assert!(config.enable_zoom);
    assert!(config.enable_collision_detection);
}

#[test]
fn controller_creation() {
    let controller = make_controller();
    assert!(controller.config.enable_rotation);
    assert!(controller.config.enable_pan);
    assert!(controller.config.enable_zoom);
}

// === 缩放 ===

#[test]
fn zoom_in_decreases_distance() {
    let controller = make_controller();
    let mut camera = make_camera();
    let initial = camera.position.length();
    controller.zoom(&mut camera, 1.0);
    assert!(camera.position.length() < initial);
}

#[test]
fn zoom_out_increases_distance() {
    let controller = make_controller();
    let mut camera = make_camera();
    let initial = camera.position.length();
    controller.zoom(&mut camera, -1.0);
    assert!(camera.position.length() > initial);
}

#[test]
fn zoom_disabled_no_change() {
    let mut controller = make_controller();
    controller.config.enable_zoom = false;
    let mut camera = make_camera();
    let initial = camera.position;
    controller.zoom(&mut camera, 1.0);
    assert_eq!(camera.position, initial);
}

#[test]
fn zoom_collision_prevents_underground() {
    let controller = make_controller();
    // 相机非常接近表面
    let mut camera = Camera::new(
        DVec3::new(6378137.0 + 5.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    // 大幅放大
    controller.zoom(&mut camera, 100.0);
    let height = camera.position.length() - Ellipsoid::WGS84.maximum_radius();
    // 不应低于 minimum_zoom_distance
    assert!(height >= controller.config.minimum_zoom_distance - 1.0);
}

// === 平移 ===

#[test]
fn pan_moves_camera() {
    let controller = make_controller();
    let mut camera = make_camera();
    let initial = camera.position;
    controller.pan(&mut camera, 1.0, 0.0);
    assert_ne!(camera.position, initial);
}

#[test]
fn pan_disabled_no_change() {
    let mut controller = make_controller();
    controller.config.enable_pan = false;
    let mut camera = make_camera();
    let initial = camera.position;
    controller.pan(&mut camera, 1.0, 1.0);
    assert_eq!(camera.position, initial);
}

#[test]
fn pan_vertical_moves_differently() {
    let controller = make_controller();
    let mut cam_h = make_camera();
    let mut cam_v = make_camera();
    controller.pan(&mut cam_h, 1.0, 0.0);
    controller.pan(&mut cam_v, 0.0, 1.0);
    // 水平与垂直平移应产生不同位置
    assert_ne!(cam_h.position, cam_v.position);
}

// === 环绕 ===

#[test]
fn orbit_preserves_distance() {
    let controller = make_controller();
    let mut camera = make_camera();
    let target = DVec3::ZERO;
    let initial_distance = (camera.position - target).length();
    controller.orbit(&mut camera, target, 0.1, 0.0, 0.0);
    let new_distance = (camera.position - target).length();
    assert!((new_distance - initial_distance).abs() / initial_distance < 0.01);
}

#[test]
fn orbit_disabled_no_change() {
    let mut controller = make_controller();
    controller.config.enable_rotation = false;
    let mut camera = make_camera();
    let initial = camera.position;
    controller.orbit(&mut camera, DVec3::ZERO, 0.5, 0.5, 0.0);
    assert_eq!(camera.position, initial);
}

#[test]
fn orbit_changes_heading() {
    let controller = make_controller();
    let mut camera = make_camera();
    let target = DVec3::ZERO;
    let initial_pos = camera.position;
    controller.orbit(&mut camera, target, 0.3, 0.0, 0.0);
    // 位置应改变（heading 旋转）
    assert!((camera.position - initial_pos).length() > 1.0);
}

#[test]
fn orbit_zoom_changes_range() {
    let controller = make_controller();
    let mut camera = make_camera();
    let target = DVec3::ZERO;
    let initial_distance = (camera.position - target).length();
    controller.orbit(&mut camera, target, 0.0, 0.0, 1000.0);
    let new_distance = (camera.position - target).length();
    // 正的 delta_range = 缩小
    assert!(new_distance > initial_distance);
}

// === 倾斜 ===

#[test]
fn tilt_changes_position() {
    let controller = make_controller();
    // 相机相对 target 的偏移不得平行于表面法线
    let mut camera = Camera::new(
        DVec3::new(6378137.0 * 1.5, 6378137.0 * 0.5, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let target = DVec3::new(6378137.0, 0.0, 0.0); // 表面点
    let initial = camera.position;
    controller.tilt(&mut camera, target, 0.2);
    assert!((camera.position - initial).length() > 1.0);
}

// === 碰撞 ===

#[test]
fn enforce_collision_pushes_up() {
    let controller = make_controller();
    let mut camera = Camera::new(
        DVec3::new(6378137.0 + 0.5, 0.0, 0.0), // 低于最小距离
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    controller.enforce_collision(&mut camera);
    let height = camera.position.length() - Ellipsoid::WGS84.maximum_radius();
    assert!(height >= controller.config.minimum_zoom_distance - 0.01);
}

#[test]
fn enforce_collision_disabled_no_change() {
    let mut controller = make_controller();
    controller.config.enable_collision_detection = false;
    let mut camera = Camera::new(
        DVec3::new(6378137.0 + 0.5, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let initial = camera.position;
    controller.enforce_collision(&mut camera);
    assert_eq!(camera.position, initial);
}

#[test]
fn enforce_collision_high_altitude_no_change() {
    let controller = make_controller();
    let mut camera = make_camera(); // 位于 2 倍半径处，远高于表面
    let initial = camera.position;
    controller.enforce_collision(&mut camera);
    assert_eq!(camera.position, initial);
}
