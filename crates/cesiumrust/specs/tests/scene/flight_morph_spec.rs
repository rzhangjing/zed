//! Scene/CameraFlight + SceneMorph → Rust 集成测试。
//!
//! 参考 CesiumJS：
//! - Scene/Camera (flyTo/flyToBoundingSphere/flyHome)
//! - Scene/Scene（变形过渡）
//!
//! A 类测试：CameraFlight 创建/更新/进度/完成/应用、
//! compute_look_at/compute_set_view、SceneMorph start/update/complete/cancel。

use cesium_camera::{Camera, EasingFunction, SceneMode};
use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_interaction::flight::{compute_look_at, compute_set_view, CameraFlight, FlightOptions};
use cesium_interaction::morphing::{MorphState, SceneMorph};
use glam::DVec3;

fn test_camera() -> Camera {
    Camera::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    )
}

// === CameraFlight 创建 ===

#[test]
fn flight_fly_to_creation() {
    let camera = test_camera();
    let dest = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);
    let flight = CameraFlight::fly_to(&camera, dest, None, None, 2.0);
    assert_eq!(flight.start_position, camera.position);
    assert_eq!(flight.end_position, dest);
    assert_eq!(flight.duration, 2.0);
    assert!(!flight.complete);
    assert_eq!(flight.elapsed, 0.0);
}

#[test]
fn flight_minimum_duration() {
    let camera = test_camera();
    let dest = DVec3::new(6378137.0, 0.0, 0.0);
    let flight = CameraFlight::fly_to(&camera, dest, None, None, 0.0);
    // 时长被钳制到 0.001
    assert!(flight.duration >= 0.001);
}

#[test]
fn flight_default_direction_looks_at_center() {
    let camera = test_camera();
    let dest = DVec3::new(6378137.0, 0.0, 0.0);
    let flight = CameraFlight::fly_to(&camera, dest, None, None, 1.0);
    // Default end_direction = -dest.normalize()
    let expected = -dest.normalize();
    assert!((flight.end_direction - expected).length() < 1e-10);
}

#[test]
fn flight_explicit_direction() {
    let camera = test_camera();
    let dest = DVec3::new(6378137.0, 0.0, 0.0);
    let dir = DVec3::new(0.0, -1.0, 0.0);
    let flight = CameraFlight::fly_to(&camera, dest, Some(dir), None, 1.0);
    assert!((flight.end_direction - dir.normalize()).length() < 1e-10);
}

// === CameraFlight 更新 ===

#[test]
fn flight_update_start() {
    let camera = test_camera();
    let dest = DVec3::new(6378137.0, 0.0, 0.0);
    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 2.0);
    let (pos, _, _) = flight.update(0.0).unwrap();
    assert!((pos - camera.position).length() < 1.0);
}

#[test]
fn flight_update_complete() {
    let camera = test_camera();
    let dest = DVec3::new(6378137.0, 0.0, 0.0);
    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 1.0);
    let (pos, _, _) = flight.update(1.0).unwrap();
    assert!((pos - dest).length() < 1.0);
    assert!(flight.complete);
}

#[test]
fn flight_update_after_complete_returns_none() {
    let camera = test_camera();
    let dest = DVec3::new(6378137.0, 0.0, 0.0);
    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 1.0);
    flight.update(2.0); // 超出时长
    assert!(flight.update(0.1).is_none());
}

#[test]
fn flight_progress() {
    let camera = test_camera();
    let dest = DVec3::new(6378137.0, 0.0, 0.0);
    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 4.0);
    flight.update(1.0);
    assert!((flight.progress() - 0.25).abs() < 1e-10);
    flight.update(1.0);
    assert!((flight.progress() - 0.5).abs() < 1e-10);
}

#[test]
fn flight_apply_to_camera() {
    let camera = test_camera();
    let dest = DVec3::new(6378137.0, 0.0, 0.0);
    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 1.0);
    let mut cam = camera.clone();
    let still_flying = flight.apply_to_camera(&mut cam, 1.0);
    assert!(!still_flying);
    assert!((cam.position - dest).length() < 1.0);
}

// === FlightOptions ===

#[test]
fn flight_options_default() {
    let opts = FlightOptions::default();
    assert_eq!(opts.duration, 3.0);
    assert_eq!(opts.easing, EasingFunction::SinusoidalInOut);
    assert!(opts.heading.is_none());
    assert!(opts.direction.is_none());
}

#[test]
fn flight_with_options() {
    let camera = test_camera();
    let opts = FlightOptions {
        destination: DVec3::new(6378137.0, 0.0, 0.0),
        duration: 5.0,
        easing: EasingFunction::Linear,
        ..Default::default()
    };
    let flight = CameraFlight::fly_to_with_options(&camera, &opts);
    assert_eq!(flight.duration, 5.0);
    assert_eq!(flight.easing, EasingFunction::Linear);
}

// === compute_look_at ===

#[test]
fn look_at_position_is_target_plus_offset() {
    let target = DVec3::new(6378137.0, 0.0, 0.0);
    let offset = DVec3::new(1000000.0, 0.0, 0.0);
    let (position, _, _) = compute_look_at(target, offset);
    assert!((position - (target + offset)).length() < 1e-6);
}

#[test]
fn look_at_direction_points_to_target() {
    let target = DVec3::new(6378137.0, 0.0, 0.0);
    let offset = DVec3::new(0.0, 0.0, 5000000.0);
    let (position, direction, up) = compute_look_at(target, offset);
    let expected_dir = (target - position).normalize();
    assert!((direction - expected_dir).length() < 1e-10);
    // up 垂直于 direction
    assert!(direction.dot(up).abs() < 1e-10);
}

// === compute_set_view ===

#[test]
fn set_view_looking_down() {
    let carto = Cartographic::from_radians(0.0, 0.0, 0.0);
    let height = 1000000.0;
    let (position, direction, _) = compute_set_view(
        &carto, height, 0.0,
        -std::f64::consts::FRAC_PI_2,
        &Ellipsoid::WGS84,
    );
    let pos_height = position.length() - Ellipsoid::WGS84.maximum_radius();
    assert!((pos_height - height).abs() / height < 0.01);
    let to_center = -position.normalize();
    assert!(direction.dot(to_center) > 0.9);
}

// === SceneMorph ===

#[test]
fn morph_default_idle() {
    let morph = SceneMorph::new();
    assert_eq!(morph.state, MorphState::Idle);
    assert!(!morph.is_morphing());
    assert_eq!(morph.progress(), 0.0);
}

#[test]
fn morph_start_transition() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();
    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 2.0);
    assert!(morph.is_morphing());
    assert_eq!(morph.duration, 2.0);
}

#[test]
fn morph_same_mode_no_op() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();
    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::Scene3D, &Ellipsoid::WGS84, 2.0);
    assert!(!morph.is_morphing());
}

#[test]
fn morph_update_progress() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();
    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 2.0);
    let mut cam = camera.clone();
    let still_morphing = morph.update(1.0, &mut cam);
    assert!(still_morphing);
    assert!((morph.progress() - 0.5).abs() < 1e-10);
    assert_eq!(cam.mode, SceneMode::Morphing);
}

#[test]
fn morph_update_completes() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();
    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 1.0);
    let mut cam = camera.clone();
    let still_morphing = morph.update(1.0, &mut cam);
    assert!(!still_morphing);
    assert!(!morph.is_morphing());
    assert_eq!(cam.mode, SceneMode::Scene2D);
}

#[test]
fn morph_complete_immediate() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();
    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 5.0);
    let mut cam = camera.clone();
    morph.complete_morph(&mut cam);
    assert!(!morph.is_morphing());
    assert_eq!(cam.mode, SceneMode::Scene2D);
    assert!((cam.position - morph.end_position).length() < 1e-6);
}

#[test]
fn morph_cancel_restores_source() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();
    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 5.0);
    let mut cam = camera.clone();
    morph.update(1.0, &mut cam); // 部分变形
    morph.cancel_morph(&mut cam);
    assert!(!morph.is_morphing());
    assert_eq!(cam.mode, SceneMode::Scene3D);
    assert!((cam.position - camera.position).length() < 1e-6);
}

#[test]
fn morph_update_idle_no_op() {
    let mut morph = SceneMorph::new();
    let mut cam = test_camera();
    let result = morph.update(1.0, &mut cam);
    assert!(!result);
}
