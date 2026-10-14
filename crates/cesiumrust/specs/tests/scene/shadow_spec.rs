//! Shadow 规范 - 参考自 Scene/ShadowMapSpec
//! 覆盖：ShadowMapConfig、ShadowMap、ShadowLightType、ShadowMapType、
//! ShadowCascade, ShadowBias, OceanSurface, GerstnerWave, OceanConfig

use cesium_shadow::{
    GerstnerWave, OceanConfig, OceanSurface, ShadowLightType, ShadowMap, ShadowMapConfig,
    ShadowMapType,
};
use glam::DVec3;

// ─── ShadowMapConfig ────────────────────────────────────────────────────────

#[test]
fn shadow_map_config_default() {
    let config = ShadowMapConfig::default();
    assert!(config.enabled);
    assert_eq!(config.resolution, 2048);
}

#[test]
fn shadow_map_type_variants() {
    assert_ne!(ShadowMapType::Single, ShadowMapType::Cascaded);
}

#[test]
fn shadow_light_type_variants() {
    assert_ne!(ShadowLightType::Directional, ShadowLightType::Point);
    assert_ne!(ShadowLightType::Point, ShadowLightType::Spot);
}

// ─── ShadowMap ──────────────────────────────────────────────────────────────

#[test]
fn shadow_map_creation() {
    let config = ShadowMapConfig::default();
    let map = ShadowMap::new(config, DVec3::new(0.0, -1.0, 0.0));
    assert!(map.config.enabled);
    assert!(map.needs_update);
}

#[test]
fn shadow_map_cascades() {
    let config = ShadowMapConfig {
        cascade_count: 4,
        ..Default::default()
    };
    let map = ShadowMap::new(config, DVec3::new(0.0, -1.0, 0.0));
    // 级联在调用 update 前为空；config 存储数量
    assert_eq!(map.config.cascade_count, 4);
    assert!(map.cascades.is_empty());
}

// ─── Ocean ──────────────────────────────────────────────────────────────────

#[test]
fn ocean_config_default() {
    let config = OceanConfig::default();
    assert!(config.enabled);
}

#[test]
fn gerstner_wave_creation() {
    let wave = GerstnerWave {
        direction: DVec3::new(1.0, 0.0, 0.0),
        wavelength: 10.0,
        amplitude: 0.5,
        speed: 2.0,
        steepness: 0.3,
        phase: 0.0,
    };
    assert_eq!(wave.wavelength, 10.0);
    assert_eq!(wave.amplitude, 0.5);
}

#[test]
fn ocean_surface_creation() {
    let surface = OceanSurface::new(OceanConfig::default());
    assert_eq!(surface.time, 0.0);
}
