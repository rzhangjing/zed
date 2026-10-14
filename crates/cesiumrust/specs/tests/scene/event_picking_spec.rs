//! 事件聚合 + 拾取规范
//! 参考自 CesiumJS Scene/CameraEventAggregatorSpec + Scene/SceneSpec（pick）

use cesium_camera::Camera;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::ray::Ray;
use cesium_interaction::event_aggregator::{
    AggregateMovement, CameraEventAggregator, CameraEventType, MouseButton,
};
use cesium_interaction::picking::{
    get_pick_ray, pick_ellipsoid, window_center, world_to_screen, Viewport,
};
use glam::{DVec2, DVec3};

// ==================== AggregateMovement ====================

#[test]
fn aggregate_movement_default_state() {
    let m = AggregateMovement::new();
    assert!(!m.is_button_down);
    assert!(!m.is_moving);
    assert_eq!(m.movement, DVec2::ZERO);
    assert_eq!(m.start_position, DVec2::ZERO);
    assert_eq!(m.end_position, DVec2::ZERO);
}

#[test]
fn aggregate_movement_button_down_sets_positions() {
    let mut m = AggregateMovement::new();
    m.button_down(DVec2::new(10.0, 20.0), 1.5);
    assert!(m.is_button_down);
    assert_eq!(m.start_position, DVec2::new(10.0, 20.0));
    assert_eq!(m.end_position, DVec2::new(10.0, 20.0));
    assert!((m.start_time - 1.5).abs() < 1e-10);
    assert!((m.last_time - 1.5).abs() < 1e-10);
}

#[test]
fn aggregate_movement_button_up_clears_state() {
    let mut m = AggregateMovement::new();
    m.button_down(DVec2::new(5.0, 5.0), 0.0);
    m.button_up(2.0);
    assert!(!m.is_button_down);
    assert!((m.last_time - 2.0).abs() < 1e-10);
}

#[test]
fn aggregate_movement_drag_computes_delta() {
    let mut m = AggregateMovement::new();
    m.button_down(DVec2::new(100.0, 200.0), 0.0);
    m.drag(DVec2::new(130.0, 250.0), 0.5);
    assert!(m.is_moving);
    assert_eq!(m.end_position, DVec2::new(130.0, 250.0));
    assert_eq!(m.movement, DVec2::new(30.0, 50.0));
}

#[test]
fn aggregate_movement_drag_without_button_down_is_noop() {
    let mut m = AggregateMovement::new();
    m.drag(DVec2::new(999.0, 999.0), 1.0);
    assert!(!m.is_moving);
    assert_eq!(m.movement, DVec2::ZERO);
}

#[test]
fn aggregate_movement_wheel_sets_vertical_delta() {
    let mut m = AggregateMovement::new();
    m.wheel(-120.0, 3.0);
    assert!(m.is_moving);
    assert!((m.movement.y - (-120.0)).abs() < 1e-10);
    assert!((m.movement.x).abs() < 1e-10);
    assert!((m.last_time - 3.0).abs() < 1e-10);
}

#[test]
fn aggregate_movement_reset_frame_clears_motion() {
    let mut m = AggregateMovement::new();
    m.button_down(DVec2::new(0.0, 0.0), 0.0);
    m.drag(DVec2::new(50.0, 50.0), 0.1);
    m.reset_frame();
    assert!(!m.is_moving);
    assert_eq!(m.movement, DVec2::ZERO);
    // 按钮仍按下
    assert!(m.is_button_down);
}

// ==================== CameraEventAggregator ====================

#[test]
fn event_aggregator_left_button_workflow() {
    let mut agg = CameraEventAggregator::new();
    agg.reset(0.0);

    agg.button_down(MouseButton::Left, DVec2::new(100.0, 100.0));
    assert!(agg.is_button_down(MouseButton::Left));
    assert!(!agg.is_button_down(MouseButton::Right));
    assert!(!agg.is_button_down(MouseButton::Middle));

    agg.mouse_move(MouseButton::Left, DVec2::new(200.0, 150.0));
    assert!(agg.is_moving(CameraEventType::LeftDrag));
    let delta = agg.get_movement_delta(CameraEventType::LeftDrag);
    assert!((delta.x - 100.0).abs() < 1e-10);
    assert!((delta.y - 50.0).abs() < 1e-10);

    agg.button_up(MouseButton::Left);
    assert!(!agg.is_button_down(MouseButton::Left));
}

#[test]
fn event_aggregator_right_button_independent() {
    let mut agg = CameraEventAggregator::new();
    agg.reset(0.0);

    agg.button_down(MouseButton::Left, DVec2::new(0.0, 0.0));
    agg.button_down(MouseButton::Right, DVec2::new(50.0, 50.0));
    assert!(agg.is_button_down(MouseButton::Left));
    assert!(agg.is_button_down(MouseButton::Right));

    agg.button_up(MouseButton::Left);
    assert!(!agg.is_button_down(MouseButton::Left));
    assert!(agg.is_button_down(MouseButton::Right));
}

#[test]
fn event_aggregator_reset_preserves_button_state() {
    let mut agg = CameraEventAggregator::new();
    agg.reset(0.0);
    agg.button_down(MouseButton::Middle, DVec2::new(10.0, 10.0));
    agg.mouse_move(MouseButton::Middle, DVec2::new(30.0, 40.0));
    assert!(agg.is_moving(CameraEventType::MiddleDrag));

    // 新帧
    agg.reset(1.0 / 60.0);
    assert!(!agg.is_moving(CameraEventType::MiddleDrag));
    assert!(agg.is_button_down(MouseButton::Middle));
}

#[test]
fn event_aggregator_wheel_event() {
    let mut agg = CameraEventAggregator::new();
    agg.reset(0.0);
    agg.wheel(120.0);
    assert!(agg.is_moving(CameraEventType::Wheel));
    let delta = agg.get_movement_delta(CameraEventType::Wheel);
    assert!((delta.y - 120.0).abs() < 1e-10);
}

#[test]
fn event_aggregator_unknown_event_returns_zero() {
    let agg = CameraEventAggregator::new();
    let delta = agg.get_movement_delta(CameraEventType::Pinch);
    assert_eq!(delta, DVec2::ZERO);
    assert!(!agg.is_moving(CameraEventType::LeftDrag));
}

// ==================== 拾取：Viewport ====================

#[test]
fn viewport_aspect_ratio() {
    let vp = Viewport::new(1920.0, 1080.0);
    assert!((vp.aspect_ratio() - 16.0 / 9.0).abs() < 1e-10);
}

#[test]
fn window_center_computation() {
    let vp = Viewport::new(800.0, 600.0);
    let c = window_center(&vp);
    assert!((c.x - 400.0).abs() < 1e-10);
    assert!((c.y - 300.0).abs() < 1e-10);
}

// ==================== 拾取：get_pick_ray ====================

fn test_camera() -> Camera {
    Camera::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    )
}

#[test]
fn pick_ray_center_points_toward_earth() {
    let camera = test_camera();
    let vp = Viewport::new(800.0, 600.0);
    let ray = get_pick_ray(DVec2::new(400.0, 300.0), &vp, &camera).unwrap();
    // 方向应大致为 -X
    assert!(ray.direction.x < -0.9);
}

#[test]
fn pick_ray_invalid_viewport_returns_none() {
    let camera = test_camera();
    let vp = Viewport::new(0.0, 600.0);
    assert!(get_pick_ray(DVec2::new(100.0, 100.0), &vp, &camera).is_none());
}

#[test]
fn pick_ray_corner_differs_from_center() {
    let camera = test_camera();
    let vp = Viewport::new(800.0, 600.0);
    let center = get_pick_ray(DVec2::new(400.0, 300.0), &vp, &camera).unwrap();
    let corner = get_pick_ray(DVec2::new(0.0, 0.0), &vp, &camera).unwrap();
    let dot = center.direction.dot(corner.direction);
    assert!(dot < 0.999);
}

// ==================== 拾取：pick_ellipsoid ====================

#[test]
fn pick_ellipsoid_hit_on_surface() {
    let camera = test_camera();
    let vp = Viewport::new(800.0, 600.0);
    let ray = get_pick_ray(DVec2::new(400.0, 300.0), &vp, &camera).unwrap();
    let hit = pick_ellipsoid(&ray, &Ellipsoid::WGS84).unwrap();
    // 验证在表面上
    let radii = Ellipsoid::WGS84.radii();
    let norm = DVec3::new(hit.x / radii.x, hit.y / radii.y, hit.z / radii.z);
    assert!((norm.length() - 1.0).abs() < 1e-6);
}

#[test]
fn pick_ellipsoid_ray_away_misses() {
    let ray = Ray::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
    );
    assert!(pick_ellipsoid(&ray, &Ellipsoid::WGS84).is_none());
}

#[test]
fn pick_ellipsoid_tangent_ray() {
    // 射线与表面相切（恰好擦过）
    let ray = Ray::new(
        DVec3::new(6378137.0, 6378137.0 * 2.0, 0.0),
        DVec3::new(0.0, 1.0, 0.0),
    );
    // 应未命中（在 Y 方向远离表面）
    assert!(pick_ellipsoid(&ray, &Ellipsoid::WGS84).is_none());
}

// ==================== 拾取：world_to_screen ====================

#[test]
fn world_to_screen_point_in_front() {
    let camera = test_camera();
    let vp = Viewport::new(800.0, 600.0);
    let point = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);
    let screen = world_to_screen(point, &vp, &camera).unwrap();
    // 应靠近中心
    assert!((screen.x - 400.0).abs() < 50.0);
    assert!((screen.y - 300.0).abs() < 50.0);
}

#[test]
fn world_to_screen_behind_camera_returns_none() {
    let camera = test_camera();
    let vp = Viewport::new(800.0, 600.0);
    let point = DVec3::new(6378137.0 * 5.0, 0.0, 0.0);
    assert!(world_to_screen(point, &vp, &camera).is_none());
}
