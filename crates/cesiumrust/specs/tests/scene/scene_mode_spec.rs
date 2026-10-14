//! Scene/SceneMode + 变形 → Rust 集成测试。
//!
//! 参考 CesiumJS：
//! - Scene/SceneMode（模式枚举、getMorphTime）
//! - Scene/SceneMode 变形过渡
//!
//! A 类测试：模式属性、变形状态机、smoothstep、
//! 2D 投影/反投影、Columbus View、各模式相机、MapProjection2D。
//! C 类省略：实际 Scene 渲染、相机控制器集成。

use cesium_scene_mode::{
    compute_camera_for_mode, morph_position, project_to_2d, project_to_columbus_view,
    smoothstep, unproject_from_2d, MapProjection2D, MorphState, SceneMode,
};
use glam::DVec3;
use std::f64::consts::PI;

const EARTH_RADIUS: f64 = 6378137.0;

// === SceneMode 枚举 ===

#[test]
fn scene_mode_default_is_3d() {
    assert_eq!(SceneMode::default(), SceneMode::Scene3D);
}

#[test]
fn scene_mode_is_3d() {
    assert!(SceneMode::Scene3D.is_3d());
    assert!(!SceneMode::Scene2D.is_3d());
    assert!(!SceneMode::ColumbusView.is_3d());
    assert!(!SceneMode::Morphing.is_3d());
}

#[test]
fn scene_mode_is_2d() {
    assert!(SceneMode::Scene2D.is_2d());
    assert!(!SceneMode::Scene3D.is_2d());
    assert!(!SceneMode::ColumbusView.is_2d());
}

// === MorphState ===

#[test]
fn morph_state_default_inactive() {
    let state = MorphState::default();
    assert!(!state.active);
    assert_eq!(state.progress, 1.0);
    assert_eq!(state.from, SceneMode::Scene3D);
    assert_eq!(state.to, SceneMode::Scene3D);
}

#[test]
fn morph_state_start_morph() {
    let mut state = MorphState::default();
    state.start_morph(SceneMode::Scene3D, SceneMode::Scene2D, 2.0);
    assert!(state.active);
    assert_eq!(state.progress, 0.0);
    assert_eq!(state.from, SceneMode::Scene3D);
    assert_eq!(state.to, SceneMode::Scene2D);
    assert_eq!(state.duration, 2.0);
    assert_eq!(state.elapsed, 0.0);
}

#[test]
fn morph_state_update_progress() {
    let mut state = MorphState::default();
    state.start_morph(SceneMode::Scene3D, SceneMode::Scene2D, 2.0);

    state.update(0.5);
    assert!((state.progress - 0.25).abs() < 1e-10);
    assert!(state.active);

    state.update(0.5);
    assert!((state.progress - 0.5).abs() < 1e-10);
    assert!(state.active);

    state.update(1.0);
    assert!((state.progress - 1.0).abs() < 1e-10);
    assert!(!state.active);
}

#[test]
fn morph_state_update_clamps_progress() {
    let mut state = MorphState::default();
    state.start_morph(SceneMode::Scene3D, SceneMode::Scene2D, 1.0);

    state.update(5.0); // 过冲
    assert_eq!(state.progress, 1.0);
    assert!(!state.active);
}

#[test]
fn morph_state_update_noop_when_inactive() {
    let mut state = MorphState::default();
    state.update(1.0);
    assert_eq!(state.progress, 1.0); // 保持不变
}

#[test]
fn morph_state_current_mode_morphing() {
    let mut state = MorphState::default();
    state.start_morph(SceneMode::Scene3D, SceneMode::Scene2D, 2.0);
    assert_eq!(state.current_mode(), SceneMode::Morphing);
}

#[test]
fn morph_state_current_mode_after_complete() {
    let mut state = MorphState::default();
    state.start_morph(SceneMode::Scene3D, SceneMode::Scene2D, 1.0);
    state.update(2.0);
    assert_eq!(state.current_mode(), SceneMode::Scene2D);
}

// === smoothstep ===

#[test]
fn smoothstep_boundaries() {
    assert!((smoothstep(0.0) - 0.0).abs() < 1e-10);
    assert!((smoothstep(1.0) - 1.0).abs() < 1e-10);
}

#[test]
fn smoothstep_midpoint() {
    // smoothstep(0.5) = 0.5*0.5*(3-2*0.5) = 0.25*2 = 0.5
    assert!((smoothstep(0.5) - 0.5).abs() < 1e-10);
}

#[test]
fn smoothstep_clamps_input() {
    assert!((smoothstep(-1.0) - 0.0).abs() < 1e-10);
    assert!((smoothstep(2.0) - 1.0).abs() < 1e-10);
}

#[test]
fn smoothstep_quarter() {
    // smoothstep(0.25) = 0.25*0.25*(3-2*0.25) = 0.0625*2.5 = 0.15625
    assert!((smoothstep(0.25) - 0.15625).abs() < 1e-10);
}

// === morph_position ===

#[test]
fn morph_position_at_zero() {
    let pos_3d = DVec3::new(100.0, 200.0, 300.0);
    let pos_2d = DVec3::new(10.0, 20.0, 30.0);
    let result = morph_position(pos_3d, pos_2d, 0.0);
    assert!((result.x - 100.0).abs() < 1e-10);
    assert!((result.y - 200.0).abs() < 1e-10);
    assert!((result.z - 300.0).abs() < 1e-10);
}

#[test]
fn morph_position_at_one() {
    let pos_3d = DVec3::new(100.0, 200.0, 300.0);
    let pos_2d = DVec3::new(10.0, 20.0, 30.0);
    let result = morph_position(pos_3d, pos_2d, 1.0);
    assert!((result.x - 10.0).abs() < 1e-10);
    assert!((result.y - 20.0).abs() < 1e-10);
    assert!((result.z - 30.0).abs() < 1e-10);
}

#[test]
fn morph_position_at_midpoint() {
    let pos_3d = DVec3::new(100.0, 0.0, 0.0);
    let pos_2d = DVec3::new(0.0, 100.0, 0.0);
    let result = morph_position(pos_3d, pos_2d, 0.5);
    // smoothstep(0.5) = 0.5，因此在 t=0.5 处 lerp
    assert!((result.x - 50.0).abs() < 1e-10);
    assert!((result.y - 50.0).abs() < 1e-10);
}

// === project_to_2d / unproject_from_2d ===

#[test]
fn project_to_2d_equator_prime_meridian() {
    let pos = DVec3::new(EARTH_RADIUS, 0.0, 0.0);
    let pos_2d = project_to_2d(pos, EARTH_RADIUS);
    assert!(pos_2d.x.abs() < 1e-6); // lon = 0
    assert!(pos_2d.y.abs() < 1e-6); // lat = 0
    assert!(pos_2d.z.abs() < 1e-6); // height = 0
}

#[test]
fn project_to_2d_north_pole() {
    let pos = DVec3::new(0.0, 0.0, EARTH_RADIUS);
    let pos_2d = project_to_2d(pos, EARTH_RADIUS);
    // lat = PI/2, y = lat * radius
    assert!((pos_2d.y - PI / 2.0 * EARTH_RADIUS).abs() < 1.0);
}

#[test]
fn project_unproject_roundtrip() {
    let original = DVec3::new(EARTH_RADIUS + 1000.0, 0.0, 0.0);
    let pos_2d = project_to_2d(original, EARTH_RADIUS);
    let recovered = unproject_from_2d(pos_2d, EARTH_RADIUS);
    assert!((recovered.x - original.x).abs() < 1.0);
    assert!((recovered.y - original.y).abs() < 1.0);
    assert!((recovered.z - original.z).abs() < 1.0);
}

// === project_to_columbus_view ===

#[test]
fn project_to_columbus_view_equator() {
    let pos = DVec3::new(EARTH_RADIUS, 0.0, 0.0);
    let cv = project_to_columbus_view(pos, EARTH_RADIUS);
    assert!(cv.x.abs() < 1e-6); // lon = 0
    assert!(cv.y.abs() < 1e-6); // lat = 0
    assert!(cv.z.abs() < 1e-6); // height = 0
}

// === compute_camera_for_mode ===

#[test]
fn compute_camera_3d_at_origin() {
    let cam = compute_camera_for_mode(SceneMode::Scene3D, 0.0, 0.0, 1000000.0, EARTH_RADIUS);
    let expected_r = EARTH_RADIUS + 1000000.0;
    assert!((cam.x - expected_r).abs() < 1.0);
    assert!(cam.y.abs() < 1.0);
    assert!(cam.z.abs() < 1.0);
}

#[test]
fn compute_camera_2d() {
    let cam = compute_camera_for_mode(SceneMode::Scene2D, 0.5, 0.3, 1000000.0, EARTH_RADIUS);
    assert!((cam.x - 0.5 * EARTH_RADIUS).abs() < 1.0);
    assert!((cam.y - 0.3 * EARTH_RADIUS).abs() < 1.0);
    assert!((cam.z - 1000000.0).abs() < 1.0);
}

#[test]
fn compute_camera_columbus_view() {
    let cam = compute_camera_for_mode(SceneMode::ColumbusView, 0.5, 0.3, 500000.0, EARTH_RADIUS);
    assert!((cam.x - 0.5 * EARTH_RADIUS).abs() < 1.0);
    assert!((cam.y - 0.3 * EARTH_RADIUS).abs() < 1.0);
    assert!((cam.z - 500000.0).abs() < 1.0);
}

// === MapProjection2D ===

#[test]
fn map_projection_geographic_roundtrip() {
    let proj = MapProjection2D::Geographic;
    let pos = proj.project(0.5, 0.3, EARTH_RADIUS);
    assert!((pos.x - 0.5 * EARTH_RADIUS).abs() < 1.0);
    assert!((pos.y - 0.3 * EARTH_RADIUS).abs() < 1.0);

    let (lon, lat) = proj.unproject(pos.x, pos.y, EARTH_RADIUS);
    assert!((lon - 0.5).abs() < 1e-10);
    assert!((lat - 0.3).abs() < 1e-10);
}

#[test]
fn map_projection_web_mercator_origin() {
    let proj = MapProjection2D::WebMercator;
    let pos = proj.project(0.0, 0.0, EARTH_RADIUS);
    assert!(pos.x.abs() < 1e-6);
    assert!(pos.y.abs() < 1e-6);
}

#[test]
fn map_projection_web_mercator_roundtrip() {
    let proj = MapProjection2D::WebMercator;
    let lon = 0.3;
    let lat = 0.5;
    let pos = proj.project(lon, lat, EARTH_RADIUS);
    let (recovered_lon, recovered_lat) = proj.unproject(pos.x, pos.y, EARTH_RADIUS);
    assert!((recovered_lon - lon).abs() < 1e-10);
    assert!((recovered_lat - lat).abs() < 1e-10);
}
