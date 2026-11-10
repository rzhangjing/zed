//! GlobeSurface 扩展 specs - 参考自 GlobeSpec
//!
//! 测试 NearFarScalar 插值、GlobeSurface 射线拾取、
//! 地平距离/俯角、可见半球、瓦片 SSE 计算、
//! GlobeTranslucency、ShadowMode、GlobeConfig 默认值。

use cesium_globe::{GlobeConfig, GlobeSurface, GlobeTranslucency, NearFarScalar, ShadowMode};
use cesium_geospatial::Ellipsoid;
use glam::DVec3;

// ─── NearFarScalar ─────────────────────────────────────────────────────────

#[test]
fn near_far_scalar_at_near() {
    let nfs = NearFarScalar::new(100.0, 10.0, 1000.0, 1.0);
    assert!((nfs.interpolate(100.0) - 10.0).abs() < 1e-10);
}

#[test]
fn near_far_scalar_at_far() {
    let nfs = NearFarScalar::new(100.0, 10.0, 1000.0, 1.0);
    assert!((nfs.interpolate(1000.0) - 1.0).abs() < 1e-10);
}

#[test]
fn near_far_scalar_before_near_clamps() {
    let nfs = NearFarScalar::new(100.0, 10.0, 1000.0, 1.0);
    assert!((nfs.interpolate(50.0) - 10.0).abs() < 1e-10);
}

#[test]
fn near_far_scalar_beyond_far_clamps() {
    let nfs = NearFarScalar::new(100.0, 10.0, 1000.0, 1.0);
    assert!((nfs.interpolate(2000.0) - 1.0).abs() < 1e-10);
}

#[test]
fn near_far_scalar_midpoint() {
    let nfs = NearFarScalar::new(0.0, 0.0, 100.0, 100.0);
    assert!((nfs.interpolate(50.0) - 50.0).abs() < 1e-10);
}

#[test]
fn near_far_scalar_quarter() {
    let nfs = NearFarScalar::new(0.0, 0.0, 100.0, 200.0);
    assert!((nfs.interpolate(25.0) - 50.0).abs() < 1e-10);
}

// ─── GlobeSurface 拾取 ─────────────────────────────────────────────────────

#[test]
fn globe_pick_from_above_hits() {
    let globe = GlobeSurface::new();
    let origin = DVec3::new(0.0, 0.0, Ellipsoid::WGS84.maximum_radius() + 1000000.0);
    let direction = DVec3::new(0.0, 0.0, -1.0);

    let hit = globe.pick(origin, direction);
    assert!(hit.is_some());
    let point = hit.unwrap();
    // 应在靠近北极处命中（z ≈ 极半径）
    assert!(point.z > 6300000.0);
}

#[test]
fn globe_pick_misses() {
    let globe = GlobeSurface::new();
    // 射线背向球体
    let origin = DVec3::new(0.0, 0.0, Ellipsoid::WGS84.maximum_radius() + 1000000.0);
    let direction = DVec3::new(0.0, 0.0, 1.0); // 背向

    let hit = globe.pick(origin, direction);
    assert!(hit.is_none());
}

#[test]
fn globe_pick_tangent_misses() {
    let globe = GlobeSurface::new();
    // 射线平行于表面，远离球体
    let r = Ellipsoid::WGS84.maximum_radius();
    let origin = DVec3::new(0.0, r + 100000.0, 0.0);
    let direction = DVec3::new(1.0, 0.0, 0.0); // 平行

    let hit = globe.pick(origin, direction);
    assert!(hit.is_none());
}

#[test]
fn globe_pick_from_inside() {
    let globe = GlobeSurface::new();
    // 原点位于椭球内部
    let origin = DVec3::new(0.0, 0.0, 0.0);
    let direction = DVec3::new(0.0, 0.0, 1.0);

    let hit = globe.pick(origin, direction);
    assert!(hit.is_some());
    let point = hit.unwrap();
    // 应沿 +z 方向命中表面
    assert!(point.z > 6300000.0);
}

// ─── 地平距离 / 俯角 ─────────────────────────────────────────

#[test]
fn horizon_distance_zero_height() {
    let globe = GlobeSurface::new();
    let d = globe.horizon_distance(0.0);
    assert!(d.abs() < 1e-6);
}

#[test]
fn horizon_distance_positive_height() {
    let globe = GlobeSurface::new();
    let r = Ellipsoid::WGS84.maximum_radius();
    let h = 10000.0; // 10 km
    let expected = (2.0 * r * h + h * h).sqrt();
    let d = globe.horizon_distance(h);
    assert!((d - expected).abs() < 1.0);
    // 在地球上 10km 高度处应约为 357 km
    assert!(d > 300000.0 && d < 400000.0);
}

#[test]
fn horizon_dip_angle_zero_height() {
    let globe = GlobeSurface::new();
    let dip = globe.horizon_dip_angle(0.0);
    assert!(dip.abs() < 1e-10);
}

#[test]
fn horizon_dip_angle_positive() {
    let globe = GlobeSurface::new();
    let dip = globe.horizon_dip_angle(10000.0);
    // 俯角应为较小的正值（几度）
    assert!(dip > 0.0);
    assert!(dip < 0.1); // 小于约 5.7 度
}

// ─── 可见半球 ────────────────────────────────────────────────────

#[test]
fn visible_hemisphere_facing_camera() {
    let globe = GlobeSurface::new();
    let r = Ellipsoid::WGS84.maximum_radius();
    // +Z 表面上的点，相机沿 +Z 更远处
    let position = DVec3::new(0.0, 0.0, r * 0.99);
    let camera = DVec3::new(0.0, 0.0, r * 2.0);
    assert!(globe.is_on_visible_hemisphere(position, camera));
}

#[test]
fn visible_hemisphere_facing_away() {
    let globe = GlobeSurface::new();
    let r = Ellipsoid::WGS84.maximum_radius();
    // -Z 表面上的点，相机在 +Z 一侧
    let position = DVec3::new(0.0, 0.0, -r * 0.99);
    let camera = DVec3::new(0.0, 0.0, r * 2.0);
    assert!(!globe.is_on_visible_hemisphere(position, camera));
}

// ─── 瓦片 SSE ──────────────────────────────────────────────────────────────

#[test]
fn compute_tile_sse_basic() {
    let globe = GlobeSurface::new();
    let sse = globe.compute_tile_sse(100.0, 10000.0, 1080.0, 1.0);
    // SSE = (100 * 1080) / (10000 * 1.0) = 10.8
    assert!((sse - 10.8).abs() < 1e-6);
}

#[test]
fn compute_tile_sse_zero_distance() {
    let globe = GlobeSurface::new();
    let sse = globe.compute_tile_sse(100.0, 0.0, 1080.0, 1.0);
    assert_eq!(sse, f64::MAX);
}

#[test]
fn should_refine_tile_above_threshold() {
    let globe = GlobeSurface::new();
    // 默认 maximum_screen_space_error = 2.0
    assert!(globe.should_refine_tile(5.0));
    assert!(!globe.should_refine_tile(1.0));
    assert!(!globe.should_refine_tile(2.0)); // 并非严格大于
}

// ─── GlobeTranslucency ─────────────────────────────────────────────────────

#[test]
fn translucency_disabled_returns_one() {
    let t = GlobeTranslucency::new(false);
    assert!((t.front_alpha() - 1.0).abs() < 1e-10);
    assert!((t.back_alpha() - 1.0).abs() < 1e-10);
}

#[test]
fn translucency_enabled_uses_configured_alpha() {
    let mut t = GlobeTranslucency::new(true);
    t.front_face_alpha = 0.7;
    t.back_face_alpha = 0.3;
    assert!((t.front_alpha() - 0.7).abs() < 1e-10);
    assert!((t.back_alpha() - 0.3).abs() < 1e-10);
}

#[test]
fn translucency_default() {
    let t = GlobeTranslucency::default();
    assert!(!t.enabled);
    assert!((t.front_face_alpha - 1.0).abs() < 1e-10);
    assert!((t.back_face_alpha - 1.0).abs() < 1e-10);
}

// ─── ShadowMode / GlobeConfig ──────────────────────────────────────────────

#[test]
fn shadow_mode_default() {
    assert_eq!(ShadowMode::default(), ShadowMode::ReceiveOnly);
}

#[test]
fn globe_config_defaults() {
    let config = GlobeConfig::default();
    assert!(config.show);
    assert!(!config.depth_test_against_terrain);
    assert!(!config.translucency_enabled);
    assert!(config.maximum_screen_space_error > 0.0);
    assert!(config.tile_cache_size > 0);
}

// ─── GlobeSurface 法线 ───────────────────────────────────────────────────

#[test]
fn surface_normal_at_equator() {
    let globe = GlobeSurface::new();
    let r = Ellipsoid::WGS84.maximum_radius();
    let position = DVec3::new(r, 0.0, 0.0);
    let normal = globe.get_surface_normal(position);
    // 在赤道的 X 轴上，法线应指向 +X
    assert!((normal.x - 1.0).abs() < 0.01);
    assert!(normal.y.abs() < 0.01);
    assert!(normal.z.abs() < 0.01);
}

#[test]
fn surface_normal_at_pole() {
    let globe = GlobeSurface::new();
    let r = Ellipsoid::WGS84.minimum_radius();
    let position = DVec3::new(0.0, 0.0, r);
    let normal = globe.get_surface_normal(position);
    // 在北极，法线应指向 +Z
    assert!(normal.x.abs() < 0.01);
    assert!(normal.y.abs() < 0.01);
    assert!((normal.z - 1.0).abs() < 0.01);
}
