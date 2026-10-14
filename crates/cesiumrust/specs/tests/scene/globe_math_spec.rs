//! Globe 扩展 specs - 来自 cesium-globe 的纯数学函数
//!
//! 测试：horizon_distance、horizon_dip_angle、is_on_visible_hemisphere，
//! compute_tile_sse, should_refine_tile, get_surface_normal, pick,
//! compute_lit_color、NearFarScalar 插值、GlobeLighting 等。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_globe::atmosphere::{GroundAtmosphere, GlobeLighting, SkyAtmosphereConfig};
use cesium_globe::surface::{GlobeSurface, GlobeTranslucency, NearFarScalar};
use glam::DVec3;
use std::f64::consts::PI;

const EPSILON7: f64 = 1e-7;
const EPSILON10: f64 = 1e-10;

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

// ─── NearFarScalar ───────────────────────────────────────────────────────────

#[test]
fn near_far_scalar_interpolate_at_near() {
    let nfs = NearFarScalar::new(100.0, 0.0, 200.0, 1.0);
    let val = nfs.interpolate(100.0);
    assert!((val - 0.0).abs() < EPSILON10, "at near: expected 0.0, got {}", val);
}

#[test]
fn near_far_scalar_interpolate_at_far() {
    let nfs = NearFarScalar::new(100.0, 0.0, 200.0, 1.0);
    let val = nfs.interpolate(200.0);
    assert!((val - 1.0).abs() < EPSILON10, "at far: expected 1.0, got {}", val);
}

#[test]
fn near_far_scalar_interpolate_midpoint() {
    let nfs = NearFarScalar::new(100.0, 0.0, 200.0, 1.0);
    let val = nfs.interpolate(150.0);
    assert!((val - 0.5).abs() < EPSILON10, "at midpoint: expected 0.5, got {}", val);
}

#[test]
fn near_far_scalar_clamp_below_near() {
    let nfs = NearFarScalar::new(100.0, 0.0, 200.0, 1.0);
    let val = nfs.interpolate(50.0);
    assert!((val - 0.0).abs() < EPSILON10, "below near: expected 0.0, got {}", val);
}

#[test]
fn near_far_scalar_clamp_above_far() {
    let nfs = NearFarScalar::new(100.0, 0.0, 200.0, 1.0);
    let val = nfs.interpolate(300.0);
    assert!((val - 1.0).abs() < EPSILON10, "above far: expected 1.0, got {}", val);
}

#[test]
fn near_far_scalar_non_unit_range() {
    let nfs = NearFarScalar::new(0.0, 10.0, 100.0, 50.0);
    let val = nfs.interpolate(50.0);
    // 中点：10 + (50 - 10) * 0.5 = 30.0
    assert!((val - 30.0).abs() < EPSILON10, "expected 30.0, got {}", val);
}

// ─── GlobeSurface ────────────────────────────────────────────────────────────

#[test]
fn globe_surface_normal_at_equator() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    let pos = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0));
    let normal = surface.get_surface_normal(pos);
    // 在赤道/本初子午线处，法线应沿 +X 方向
    assert!(
        (normal.x - 1.0).abs() < 0.01,
        "normal at equator should be ≈ +X, got {:?}", normal
    );
}

#[test]
fn globe_surface_normal_at_north_pole() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    let pos = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 90.0, 0.0));
    let normal = surface.get_surface_normal(pos);
    // 在北极处，法线应沿 +Z 方向
    assert!(
        (normal.z - 1.0).abs() < 0.01,
        "normal at north pole should be ≈ +Z, got {:?}", normal
    );
}

#[test]
fn globe_surface_normal_is_unit_length() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    for lon in [-180.0, -90.0, 0.0, 45.0, 90.0, 180.0] {
        for lat in [-89.0, -45.0, 0.0, 45.0, 89.0] {
            let pos = e.cartographic_to_cartesian(&Cartographic::from_degrees(lon, lat, 0.0));
            let normal = surface.get_surface_normal(pos);
            let len = normal.length();
            assert!(
                (len - 1.0).abs() < EPSILON7,
                "normal at ({}, {}) length {} != 1.0", lon, lat, len
            );
        }
    }
}

#[test]
fn globe_surface_height_returns_zero() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    let carto = Cartographic::from_degrees(10.0, 20.0, 0.0);
    let h = surface.get_height(&carto);
    assert!(h.is_some());
    assert!(h.unwrap().abs() < 1.0, "ellipsoid surface height should be ≈ 0");
}

// ─── GlobeSurface 地平线计算 ──────────────────────────────────────

#[test]
fn globe_surface_horizon_distance_increases_with_height() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    let h_low = surface.horizon_distance(1000.0);
    let h_high = surface.horizon_distance(100_000.0);
    assert!(
        h_high > h_low,
        "horizon distance should increase with height: low={}, high={}", h_low, h_high
    );
}

#[test]
fn globe_surface_horizon_distance_at_surface() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    let h = surface.horizon_distance(0.0);
    // 在地表高度，地平距离应为 0（或非常小）
    assert!(
        h.abs() < 1.0,
        "horizon distance at surface should be ≈ 0, got {}", h
    );
}

#[test]
fn globe_surface_horizon_dip_angle_increases_with_height() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    let d_low = surface.horizon_dip_angle(1000.0);
    let d_high = surface.horizon_dip_angle(1_000_000.0);
    // 在更高海拔，俯角应更负
    assert!(
        d_high.abs() > d_low.abs(),
        "dip angle magnitude should increase with height: low={}, high={}", d_low, d_high
    );
}

// ─── GlobeSurface 可见半球 ────────────────────────────────────────

#[test]
fn globe_surface_visible_hemisphere_facing() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    let pos = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0));
    let camera = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 100_000.0));
    assert!(
        surface.is_on_visible_hemisphere(pos, camera),
        "position directly below camera should be visible"
    );
}

#[test]
fn globe_surface_visible_hemisphere_opposite() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    let pos = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0));
    let camera = e.cartographic_to_cartesian(&Cartographic::from_degrees(180.0, 0.0, 100_000.0));
    // 位于球体另一侧的位置
    assert!(
        !surface.is_on_visible_hemisphere(pos, camera),
        "position on opposite side should NOT be visible"
    );
}

// ─── GlobeSurface 瓦片 SSE ───────────────────────────────────────────────────

#[test]
fn globe_surface_tile_sse_decreases_with_distance() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    let geometric_error = 100.0;
    let viewport_height = 1080.0;
    let sse_denominator = 2.0 * (PI / 3.0).tan(); // 60 度 FOV
    let sse_near = surface.compute_tile_sse(geometric_error, 1000.0, viewport_height, sse_denominator);
    let sse_far = surface.compute_tile_sse(geometric_error, 10_000.0, viewport_height, sse_denominator);
    assert!(
        sse_near > sse_far,
        "SSE should decrease with distance: near={}, far={}", sse_near, sse_far
    );
}

#[test]
fn globe_surface_tile_sse_zero_geometric_error() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    let sse = surface.compute_tile_sse(0.0, 1000.0, 1080.0, 1.0);
    assert!(
        sse.abs() < EPSILON10,
        "SSE with zero geometric error should be 0, got {}", sse
    );
}

#[test]
fn globe_surface_should_refine_tile() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    // 高 SSE → 应细分
    assert!(surface.should_refine_tile(20.0), "high SSE should refine");
    // 低 SSE → 不应细分
    assert!(!surface.should_refine_tile(0.1), "low SSE should not refine");
}

// ─── GlobeSurface 拾取 ───────────────────────────────────────────────────────

#[test]
fn globe_surface_pick_directly_above() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    // 从赤道上方垂直向下的射线
    let origin = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 100_000.0));
    let direction = -origin.normalize();
    let hit = surface.pick(origin, direction);
    assert!(hit.is_some(), "ray from above should hit globe");
    let hit_pos = hit.unwrap();
    let hit_carto = e.cartesian_to_cartographic(hit_pos).unwrap();
    assert!(
        hit_carto.height.abs() < 100.0,
        "hit should be near surface, height={}", hit_carto.height
    );
}

#[test]
fn globe_surface_pick_misses_when_parallel() {
    let e = wgs84();
    let surface = GlobeSurface::with_ellipsoid(e);
    // 射线平行于表面，远离球体
    let origin = DVec3::new(0.0, 0.0, 100_000_000.0); // 极高处
    let direction = DVec3::X; // 侧向观察
    let hit = surface.pick(origin, direction);
    assert!(hit.is_none(), "parallel ray far from globe should miss");
}

// ─── GlobeTranslucency ───────────────────────────────────────────────────────

#[test]
fn globe_translucency_disabled_by_default() {
    let t = GlobeTranslucency::new(false);
    assert!(!t.enabled);
    assert!((t.front_alpha() - 1.0).abs() < EPSILON10);
}

#[test]
fn globe_translucency_enabled() {
    let t = GlobeTranslucency::new(true);
    assert!(t.enabled);
}

// ─── GroundAtmosphere ────────────────────────────────────────────────────────

#[test]
fn ground_atmosphere_sky_color_nonzero() {
    let atm = GroundAtmosphere::default();
    let view_dir = DVec3::new(0.0, 0.0, -1.0);
    let sun_dir = DVec3::new(1.0, 0.0, 0.0);
    let sky = atm.compute_sky_color(view_dir, sun_dir, 10_000.0);
    let magnitude = (sky[0] * sky[0] + sky[1] * sky[1] + sky[2] * sky[2]).sqrt();
    assert!(
        magnitude > 0.0,
        "sky color should be nonzero, got {:?}", sky
    );
}

#[test]
fn ground_atmosphere_zenith_vs_horizon() {
    let atm = GroundAtmosphere::default();
    let zenith = atm.compute_zenith_color(0.5);
    let horizon = atm.compute_horizon_glow(0.5);
    // 天顶和地平线应产生不同的颜色
    let diff = ((zenith[0] - horizon[0]).powi(2)
        + (zenith[1] - horizon[1]).powi(2)
        + (zenith[2] - horizon[2]).powi(2))
    .sqrt();
    assert!(
        diff > 0.001,
        "zenith and horizon colors should differ: zenith={:?}, horizon={:?}", zenith, horizon
    );
}

#[test]
fn globe_lighting_diffuse_lambertian() {
    let mut lighting = GlobeLighting::default();
    lighting.enabled = true;
    lighting.sun_direction = DVec3::new(1.0, 0.0, 0.0);
    let normal_facing = DVec3::new(1.0, 0.0, 0.0);
    let normal_perpendicular = DVec3::new(0.0, 1.0, 0.0);
    let diffuse_facing = lighting.compute_diffuse(normal_facing);
    let diffuse_perp = lighting.compute_diffuse(normal_perpendicular);
    assert!(
        diffuse_facing > diffuse_perp,
        "facing normal should have more diffuse light: facing={}, perp={}", diffuse_facing, diffuse_perp
    );
}

#[test]
fn globe_lighting_specular_at_perfect_reflection() {
    let mut lighting = GlobeLighting::default();
    lighting.enabled = true;
    let normal = DVec3::new(0.0, 0.0, 1.0);
    let view = DVec3::new(0.0, 0.0, -1.0);
    let spec = lighting.compute_specular(normal, view);
    assert!(
        spec >= 0.0,
        "specular should be non-negative, got {}", spec
    );
}

// ─── SkyAtmosphereConfig ─────────────────────────────────────────────────────

#[test]
fn sky_atmosphere_config_defaults() {
    let config = SkyAtmosphereConfig::default();
    assert!(
        config.atmosphere_radius > 0.0,
        "atmosphere radius should be positive"
    );
}

#[test]
fn sky_atmosphere_config_radius() {
    let config = SkyAtmosphereConfig::default();
    // 大气半径应包住大气层（地球约 100km）
    assert!(
        config.atmosphere_radius > 6_400_000.0,
        "atmosphere radius should be > 6400km, got {}", config.atmosphere_radius
    );
}
