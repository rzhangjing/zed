//! Morphing/Camera 扩展规范 - SceneMorph 模式过渡 + CameraFlight 扩展
//! 参考自 Scene/SceneSpec 变形 + Scene/CameraSpec 飞行（A 类）

use cesium_interaction::{SceneMorph, CameraFlight, FlightOptions, compute_look_at};
use cesium_camera::{Camera, SceneMode};
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::BoundingSphere;
use glam::DVec3;

fn test_camera() -> Camera {
    Camera::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    )
}

// ─── SceneMorph 模式过渡 ────────────────────────────────────────────

#[test]
fn morph_3d_to_2d_starts() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();

    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 2.0);

    assert!(morph.is_morphing());
    assert!((morph.progress()).abs() < 1e-10);
}

#[test]
fn morph_3d_to_columbus_view() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();

    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::ColumbusView, &Ellipsoid::WGS84, 1.5);

    assert!(morph.is_morphing());
    assert!((morph.duration - 1.5).abs() < 1e-10);
}

#[test]
fn morph_2d_to_3d() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();

    morph.start_morph(&camera, SceneMode::Scene2D, SceneMode::Scene3D, &Ellipsoid::WGS84, 3.0);

    assert!(morph.is_morphing());
}

#[test]
fn morph_update_sets_morphing_mode() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();
    let mut cam = camera.clone();

    morph.start_morph(&cam, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 2.0);
    let still_morphing = morph.update(0.5, &mut cam);

    assert!(still_morphing);
    assert_eq!(cam.mode, SceneMode::Morphing);
}

#[test]
fn morph_update_halfway_progress() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();
    let mut cam = camera.clone();

    morph.start_morph(&cam, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 2.0);
    morph.update(1.0, &mut cam); // 中途

    assert!((morph.progress() - 0.5).abs() < 1e-10);
}

#[test]
fn morph_completes_sets_target_mode() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();
    let mut cam = camera.clone();

    morph.start_morph(&cam, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 1.0);
    let still_morphing = morph.update(1.0, &mut cam);

    assert!(!still_morphing);
    assert_eq!(cam.mode, SceneMode::Scene2D);
    assert!(!morph.is_morphing());
}

#[test]
fn morph_cancel_restores_source_mode() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();
    let mut cam = camera.clone();

    morph.start_morph(&cam, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 2.0);
    morph.update(0.5, &mut cam);
    morph.cancel_morph(&mut cam);

    assert_eq!(cam.mode, SceneMode::Scene3D);
    assert!(!morph.is_morphing());
    // 位置应恢复到起点
    assert!((cam.position - camera.position).length() < 1e-6);
}

#[test]
fn morph_complete_sets_end_position() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();
    let mut cam = camera.clone();

    morph.start_morph(&cam, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 2.0);
    let end_pos = morph.end_position;
    morph.complete_morph(&mut cam);

    assert!((cam.position - end_pos).length() < 1e-6);
    assert_eq!(cam.mode, SceneMode::Scene2D);
}

#[test]
fn morph_same_mode_is_noop() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();

    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::Scene3D, &Ellipsoid::WGS84, 2.0);

    assert!(!morph.is_morphing());
}

#[test]
fn morph_minimum_duration_clamped() {
    let camera = test_camera();
    let mut morph = SceneMorph::new();

    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 0.0);

    // 时长应被钳制到至少 0.001
    assert!(morph.duration >= 0.001);
}

// ─── CameraFlight 扩展 ──────────────────────────────────────────────────

#[test]
fn flight_fly_home() {
    let camera = test_camera();
    let flight = CameraFlight::fly_home(&camera, &Ellipsoid::WGS84, 2.0);

    assert!((flight.duration - 2.0).abs() < 1e-10);
    assert!(!flight.complete);
    // 终点位置应为"home"视图（赤道上方）
    assert!(flight.end_position.length() > Ellipsoid::WGS84.maximum_radius());
}

#[test]
fn flight_fly_to_bounding_sphere() {
    let camera = test_camera();
    let center = DVec3::new(6378137.0, 0.0, 0.0);
    let radius = 100000.0;
    let sphere = BoundingSphere::new(center, radius);

    let flight = CameraFlight::fly_to_bounding_sphere(
        &camera,
        &sphere,
        None,
        3.0,
    );

    assert!((flight.duration - 3.0).abs() < 1e-10);
    // 终点位置应偏离中心若干个半径倍数
    let dist = (flight.end_position - center).length();
    assert!(dist > radius, "camera should be outside the bounding sphere");
}

#[test]
fn flight_update_smooth_interpolation() {
    let camera = test_camera();
    let destination = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);
    let mut flight = CameraFlight::fly_to(&camera, destination, None, None, 4.0);

    // 以规则间隔收集位置
    let mut positions = Vec::new();
    for _ in 0..4 {
        if let Some((pos, _, _)) = flight.update(1.0) {
            positions.push(pos);
        }
    }

    // 位置应单调地靠近目的地
    for i in 1..positions.len() {
        let d_prev = (positions[i - 1] - destination).length();
        let d_curr = (positions[i] - destination).length();
        assert!(d_curr <= d_prev, "flight should approach destination monotonically");
    }
}

#[test]
fn flight_with_options_custom_duration() {
    let camera = test_camera();
    let options = FlightOptions {
        duration: 5.0,
        ..Default::default()
    };
    let destination = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);

    let flight = CameraFlight::fly_to(&camera, destination, None, None, options.duration);

    assert!((flight.duration - 5.0).abs() < 1e-10);
}

#[test]
fn compute_look_at_up_perpendicular() {
    let target = DVec3::new(6378137.0, 0.0, 0.0);
    let offset = DVec3::new(0.0, 0.0, 1000000.0); // 从上方俯视

    let (_position, direction, up) = compute_look_at(target, offset);

    // up 应垂直于 direction
    assert!(direction.dot(up).abs() < 1e-10, "up must be perpendicular to direction");
    // up 应被归一化
    assert!((up.length() - 1.0).abs() < 1e-10);
}
