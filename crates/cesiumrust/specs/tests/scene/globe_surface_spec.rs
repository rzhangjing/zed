//! 地球表面 + 大气规范
//! 参考自 CesiumJS Scene/GlobeSpec + Scene/GlobeTranslucencySpec

use cesium_globe::{
    GlobeConfig, GlobeLighting, GlobeSurface, GlobeTranslucency, GroundAtmosphere,
    NearFarScalar, ShadowMode, SkyAtmosphereConfig, SkyBoxConfig,
};
use glam::DVec3;

// ==================== NearFarScalar ====================

#[test]
fn near_far_scalar_interpolate_at_near() {
    let nfs = NearFarScalar::new(100.0, 1.0, 1000.0, 0.0);
    assert!((nfs.interpolate(50.0) - 1.0).abs() < 1e-10);
    assert!((nfs.interpolate(100.0) - 1.0).abs() < 1e-10);
}

#[test]
fn near_far_scalar_interpolate_at_far() {
    let nfs = NearFarScalar::new(100.0, 1.0, 1000.0, 0.0);
    assert!((nfs.interpolate(1000.0) - 0.0).abs() < 1e-10);
    assert!((nfs.interpolate(2000.0) - 0.0).abs() < 1e-10);
}

#[test]
fn near_far_scalar_interpolate_midpoint() {
    let nfs = NearFarScalar::new(0.0, 10.0, 100.0, 0.0);
    let mid = nfs.interpolate(50.0);
    assert!((mid - 5.0).abs() < 1e-10);
}

#[test]
fn near_far_scalar_interpolate_quarter() {
    let nfs = NearFarScalar::new(0.0, 0.0, 100.0, 1.0);
    let val = nfs.interpolate(25.0);
    assert!((val - 0.25).abs() < 1e-10);
}

// ==================== GlobeConfig ====================

#[test]
fn globe_config_defaults() {
    let config = GlobeConfig::default();
    assert!(config.show);
    assert!(!config.depth_test_against_terrain);
    assert!(!config.translucency_enabled);
    assert!(config.show_ground_atmosphere);
    assert!(config.show_water_effect);
    assert!((config.maximum_screen_space_error - 2.0).abs() < 1e-10);
    assert_eq!(config.tile_cache_size, 100);
    assert!(!config.enable_lighting);
    assert!(config.preload_ancestors);
    assert!(!config.preload_siblings);
    assert_eq!(config.shadows, ShadowMode::ReceiveOnly);
    assert!(config.show_skirts);
    assert!(config.back_face_culling);
}

#[test]
fn globe_config_atmosphere_defaults() {
    let config = GlobeConfig::default();
    assert!(config.dynamic_atmosphere_lighting);
    assert!(!config.dynamic_atmosphere_lighting_from_sun);
    assert!((config.atmosphere_light_intensity - 10.0).abs() < 1e-10);
    assert!((config.atmosphere_mie_anisotropy - 0.9).abs() < 1e-10);
    assert!((config.atmosphere_hue_shift - 0.0).abs() < 1e-10);
}

// ==================== GlobeSurface ====================

#[test]
fn globe_surface_pick_from_above() {
    let globe = GlobeSurface::new();
    let origin = DVec3::new(0.0, 0.0, 10_000_000.0);
    let direction = DVec3::new(0.0, 0.0, -1.0);
    let hit = globe.pick(origin, direction);
    assert!(hit.is_some());
    let p = hit.unwrap();
    // 应命中靠近北极处 (z ≈ 6356752)
    assert!((p.z - 6356752.3142).abs() < 1.0);
}

#[test]
fn globe_surface_pick_miss() {
    let globe = GlobeSurface::new();
    let origin = DVec3::new(0.0, 0.0, 10_000_000.0);
    let direction = DVec3::new(0.0, 0.0, 1.0); // 背离地球
    assert!(globe.pick(origin, direction).is_none());
}

#[test]
fn globe_surface_pick_equator() {
    let globe = GlobeSurface::new();
    let origin = DVec3::new(10_000_000.0, 0.0, 0.0);
    let direction = DVec3::new(-1.0, 0.0, 0.0);
    let hit = globe.pick(origin, direction);
    assert!(hit.is_some());
    let p = hit.unwrap();
    // 应在赤道命中 x ≈ 6378137
    assert!((p.x - 6378137.0).abs() < 1.0);
}

#[test]
fn globe_surface_horizon_distance() {
    let globe = GlobeSurface::new();
    let dist = globe.horizon_distance(1000.0);
    // d ≈ sqrt(2 * 6378137 * 1000) ≈ 112,944 m
    assert!(dist > 100_000.0 && dist < 120_000.0);
}

#[test]
fn globe_surface_horizon_distance_zero() {
    let globe = GlobeSurface::new();
    assert!(globe.horizon_distance(0.0).abs() < 1e-6);
}

#[test]
fn globe_surface_horizon_dip_angle() {
    let globe = GlobeSurface::new();
    let dip = globe.horizon_dip_angle(1000.0);
    // 小角度（~0.03 rad）
    assert!(dip > 0.0 && dip < 0.1);
}

#[test]
fn globe_surface_visible_hemisphere() {
    let globe = GlobeSurface::new();
    let camera = DVec3::new(0.0, 0.0, 10_000_000.0);
    let north = DVec3::new(0.0, 0.0, 6356752.3142);
    let south = DVec3::new(0.0, 0.0, -6356752.3142);
    assert!(globe.is_on_visible_hemisphere(north, camera));
    assert!(!globe.is_on_visible_hemisphere(south, camera));
}

#[test]
fn globe_surface_tile_sse() {
    let globe = GlobeSurface::new();
    // SSE = (geometricError * viewportHeight) / (distance * sseDenominator)
    let sse = globe.compute_tile_sse(100.0, 10000.0, 1080.0, 1.0);
    let expected = (100.0 * 1080.0) / (10000.0 * 1.0);
    assert!((sse - expected).abs() < 1e-10);
}

#[test]
fn globe_surface_tile_sse_zero_distance() {
    let globe = GlobeSurface::new();
    let sse = globe.compute_tile_sse(100.0, 0.0, 1080.0, 1.0);
    assert_eq!(sse, f64::MAX);
}

#[test]
fn globe_surface_should_refine() {
    let globe = GlobeSurface::new();
    assert!(globe.should_refine_tile(3.0)); // > 2.0
    assert!(!globe.should_refine_tile(1.5)); // < 2.0
}

#[test]
fn globe_surface_normal_at_north_pole() {
    let globe = GlobeSurface::new();
    let normal = globe.get_surface_normal(DVec3::new(0.0, 0.0, 6356752.3142));
    assert!(normal.z > 0.99);
}

// ==================== GlobeTranslucency ====================

#[test]
fn globe_translucency_disabled() {
    let t = GlobeTranslucency::default();
    assert!(!t.enabled);
    assert!((t.front_alpha() - 1.0).abs() < 1e-10);
    assert!((t.back_alpha() - 1.0).abs() < 1e-10);
}

#[test]
fn globe_translucency_enabled() {
    let mut t = GlobeTranslucency::new(true);
    t.front_face_alpha = 0.5;
    t.back_face_alpha = 0.3;
    assert!((t.front_alpha() - 0.5).abs() < 1e-10);
    assert!((t.back_alpha() - 0.3).abs() < 1e-10);
}

// ==================== GroundAtmosphere ====================

#[test]
fn ground_atmosphere_sky_color_nonzero() {
    let atmo = GroundAtmosphere::default();
    let color = atmo.compute_sky_color(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, 1.0),
        0.0,
    );
    assert!(color[0] > 0.0 || color[1] > 0.0 || color[2] > 0.0);
}

#[test]
fn ground_atmosphere_blue_dominant() {
    let atmo = GroundAtmosphere::default();
    let color = atmo.compute_sky_color(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(1.0, 0.0, 0.0),
        0.0,
    );
    // 蓝色通道应占主导（瑞利散射）
    assert!(color[2] >= color[0]);
}

#[test]
fn ground_atmosphere_horizon_glow_sunset() {
    let atmo = GroundAtmosphere::default();
    let glow = atmo.compute_horizon_glow(0.0); // 太阳位于地平线
    assert!(glow[0] > glow[1]); // 红色占主导
    assert!(glow[0] > 0.5);
}

#[test]
fn ground_atmosphere_zenith_day() {
    let atmo = GroundAtmosphere::default();
    let zenith = atmo.compute_zenith_color(0.5); // 太阳高过地平线
    assert!(zenith[2] > zenith[0]); // 蓝色占主导
}

// ==================== GlobeLighting + SkyConfig ====================

#[test]
fn globe_lighting_defaults() {
    let lighting = GlobeLighting::default();
    assert!(!lighting.enabled);
    assert_eq!(lighting.sun_direction, DVec3::X);
    assert!((lighting.specular_intensity - 0.5).abs() < 1e-10);
}

#[test]
fn sky_atmosphere_config_defaults() {
    let config = SkyAtmosphereConfig::default();
    assert!(config.show);
    assert!((config.hue_shift - 0.0).abs() < 1e-10);
    assert!((config.atmosphere_radius - (6378137.0 + 60000.0)).abs() < 1e-10);
}

#[test]
fn sky_box_config_defaults() {
    let config = SkyBoxConfig::default();
    assert!(config.show);
    assert!(config.sources.is_none());
    assert!((config.radius - 1e15).abs() < 1e5);
}
