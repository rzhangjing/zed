//! ShadowMap + 海洋/水面规范
//! 参考自 CesiumJS Scene/ShadowMapSpec + 水面渲染逻辑

use cesium_shadow::{
    GerstnerWave, OceanConfig, OceanSurface, PcfConfig, ShadowBias, ShadowBiasType,
    ShadowLightType, ShadowMap, ShadowMapConfig, ShadowMapType, SHADOW_MAP_MAXIMUM_DISTANCE,
};
use glam::DVec3;
use std::f64::consts::PI;

// ==================== ShadowBias ====================

#[test]
fn shadow_bias_terrain_defaults() {
    let bias = ShadowBias::terrain(true);
    assert!(bias.polygon_offset);
    assert!(bias.normal_offset);
    assert!(bias.normal_shading);
    assert!((bias.polygon_offset_factor - 1.1).abs() < 1e-10);
    assert!((bias.normal_offset_scale - 0.5).abs() < 1e-10);
}

#[test]
fn shadow_bias_primitive_defaults() {
    let bias = ShadowBias::primitive(false);
    assert!(bias.polygon_offset);
    assert!(!bias.normal_offset);
    assert!((bias.normal_offset_scale - 0.1).abs() < 1e-10);
    assert!((bias.depth_bias - 0.00002).abs() < 1e-10);
}

#[test]
fn shadow_bias_effective_bias_no_normal_offset() {
    let bias = ShadowBias::primitive(false);
    let normal = DVec3::new(0.0, 1.0, 0.0);
    let light = DVec3::new(0.0, -1.0, 0.0);
    let effective = bias.compute_effective_bias(normal, light);
    // 无法线偏移时，effective = depth_bias
    assert!((effective - bias.depth_bias).abs() < 1e-10);
}

#[test]
fn shadow_bias_effective_bias_with_normal_offset() {
    let bias = ShadowBias::terrain(true);
    let normal = DVec3::new(1.0, 0.0, 0.0); // 垂直于光线
    let light = DVec3::new(0.0, -1.0, 0.0);
    let effective = bias.compute_effective_bias(normal, light);
    // n_dot_l = |dot((1,0,0), (0,1,0))| = 0, slope_factor = 1.0
    // bias = depth_bias + normal_offset_scale * 1.0
    assert!(effective > bias.depth_bias);
}

// ==================== PcfConfig ====================

#[test]
fn pcf_filter_all_lit() {
    let pcf = PcfConfig::default();
    let comparisons = vec![1.0, 0.5, 0.2, 0.1, 0.0, 1.0, 0.3, 0.8, 0.9];
    let result = pcf.filter(&comparisons);
    assert!((result - 1.0).abs() < 1e-10);
}

#[test]
fn pcf_filter_half_shadowed() {
    let pcf = PcfConfig::default();
    let comparisons = vec![1.0, -1.0, 1.0, -1.0];
    let result = pcf.filter(&comparisons);
    assert!((result - 0.5).abs() < 1e-10);
}

#[test]
fn pcf_sample_offsets_kernel3() {
    let pcf = PcfConfig {
        kernel_size: 3,
        use_poisson_disk: false,
        ..Default::default()
    };
    let offsets = pcf.sample_offsets();
    assert_eq!(offsets.len(), 9); // 3x3
}

#[test]
fn pcf_sample_offsets_poisson() {
    let pcf = PcfConfig {
        kernel_size: 8,
        use_poisson_disk: true,
        ..Default::default()
    };
    let offsets = pcf.sample_offsets();
    assert_eq!(offsets.len(), 8);
}

// ==================== ShadowMapConfig ====================

#[test]
fn shadow_map_config_defaults() {
    let config = ShadowMapConfig::default();
    assert!(config.enabled);
    assert_eq!(config.shadow_map_type, ShadowMapType::Cascaded);
    assert_eq!(config.light_type, ShadowLightType::Directional);
    assert_eq!(config.resolution, 2048);
    assert_eq!(config.cascade_count, 4);
    assert!(config.soft_shadows);
    assert!(config.normal_offset);
    assert!(config.fading_enabled);
}

#[test]
fn shadow_map_maximum_distance_constant() {
    assert!((SHADOW_MAP_MAXIMUM_DISTANCE - 20000.0).abs() < 1e-10);
}

// ==================== ShadowMap ====================

#[test]
fn shadow_map_for_sun() {
    let sun_dir = DVec3::new(0.0, -1.0, 0.0);
    let map = ShadowMap::for_sun(sun_dir);
    // 光线方向为 -sun_direction
    assert!((map.light_direction.y - 1.0).abs() < 1e-10);
    assert!(map.needs_update);
    assert!((map.fade_factor - 1.0).abs() < 1e-10);
}

#[test]
fn shadow_map_for_point_light() {
    let pos = DVec3::new(10.0, 20.0, 30.0);
    let map = ShadowMap::for_point_light(pos, 50.0);
    assert_eq!(map.config.light_type, ShadowLightType::Point);
    assert_eq!(map.config.shadow_map_type, ShadowMapType::Single);
    assert_eq!(map.light_position, pos);
    assert!((map.config.point_light_radius - 50.0).abs() < 1e-10);
}

#[test]
fn shadow_map_pass_count() {
    let sun_map = ShadowMap::for_sun(DVec3::new(0.0, -1.0, 0.0));
    assert_eq!(sun_map.pass_count(), 4); // cascade_count = 4

    let point_map = ShadowMap::for_point_light(DVec3::ZERO, 10.0);
    assert_eq!(point_map.pass_count(), 6); // 立方体贴图

    let spot_map = ShadowMap::for_spot_light(DVec3::ZERO, DVec3::new(0.0, -1.0, 0.0));
    assert_eq!(spot_map.pass_count(), 1);
}

#[test]
fn shadow_map_fade_factor() {
    let map = ShadowMap::for_sun(DVec3::new(0.0, -1.0, 0.0));

    // 高仰角 → 无淡出
    assert!((map.compute_fade_factor(PI / 4.0) - 1.0).abs() < 1e-10);
    // 在地平线 → 完全淡出
    assert!((map.compute_fade_factor(0.0)).abs() < 1e-10);
    // 5 度 → 部分淡出
    let partial = map.compute_fade_factor(5.0_f64.to_radians());
    assert!(partial > 0.0 && partial < 1.0);
}

#[test]
fn shadow_map_fade_disabled() {
    let config = ShadowMapConfig {
        fading_enabled: false,
        ..Default::default()
    };
    let map = ShadowMap::new(config, DVec3::new(0.0, -1.0, 0.0));
    assert!((map.compute_fade_factor(0.0) - 1.0).abs() < 1e-10);
}

#[test]
fn shadow_map_cascade_splits() {
    let map = ShadowMap::for_sun(DVec3::new(0.0, -1.0, 0.0));
    let splits = map.compute_cascade_splits(1.0, 1000.0, 0.5);

    assert_eq!(splits.len(), 5); // cascade_count + 1
    assert!((splits[0] - 1.0).abs() < 1e-10); // 近
    assert!((splits[4] - 1000.0).abs() < 1e-10); // 远
    // 分层值应单调递增
    for i in 1..splits.len() {
        assert!(splits[i] > splits[i - 1]);
    }
}

#[test]
fn shadow_map_bias_for_type() {
    let map = ShadowMap::for_sun(DVec3::new(0.0, -1.0, 0.0));
    let terrain = map.bias_for_type(ShadowBiasType::Terrain);
    let primitive = map.bias_for_type(ShadowBiasType::Primitive);
    let point = map.bias_for_type(ShadowBiasType::Point);

    assert!((terrain.normal_offset_scale - 0.5).abs() < 1e-10);
    assert!((primitive.normal_offset_scale - 0.1).abs() < 1e-10);
    assert!(!point.polygon_offset);
}

// ==================== GerstnerWave ====================

#[test]
fn gerstner_wave_wave_number() {
    let wave = GerstnerWave::new(DVec3::new(1.0, 0.0, 0.0), 100.0, 1.0, 5.0);
    let k = wave.wave_number();
    assert!((k - std::f64::consts::TAU / 100.0).abs() < 1e-10);
}

#[test]
fn gerstner_wave_angular_frequency() {
    let wave = GerstnerWave::new(DVec3::new(1.0, 0.0, 0.0), 100.0, 1.0, 5.0);
    let omega = wave.angular_frequency();
    assert!((omega - wave.wave_number() * 5.0).abs() < 1e-10);
}

#[test]
fn gerstner_wave_displacement_at_origin_time0() {
    let wave = GerstnerWave::new(DVec3::new(1.0, 0.0, 0.0), 100.0, 2.0, 5.0);
    // 在位置 (0,0,0)、time=0：theta = 0
    // vertical = amplitude * sin(0) = 0
    let disp = wave.compute_displacement(DVec3::ZERO, 0.0);
    assert!(disp.y.abs() < 1e-10);
}

#[test]
fn gerstner_wave_normal_is_normalized() {
    let wave = GerstnerWave::new(DVec3::new(1.0, 0.0, 0.0), 50.0, 1.5, 6.0);
    let normal = wave.compute_normal(DVec3::new(10.0, 0.0, 5.0), 2.0);
    assert!((normal.length() - 1.0).abs() < 1e-10);
}

// ==================== OceanSurface ====================

#[test]
fn ocean_surface_default_waves() {
    let config = OceanConfig::default();
    assert!(config.enabled);
    assert_eq!(config.waves.len(), 5);
    assert!((config.fresnel_power - 5.0).abs() < 1e-10);
}

#[test]
fn ocean_surface_displacement_disabled() {
    let config = OceanConfig {
        enabled: false,
        ..Default::default()
    };
    let surface = OceanSurface::new(config);
    let disp = surface.compute_displacement(DVec3::new(100.0, 0.0, 200.0));
    assert_eq!(disp, DVec3::ZERO);
}

#[test]
fn ocean_surface_normal_disabled() {
    let config = OceanConfig {
        enabled: false,
        ..Default::default()
    };
    let surface = OceanSurface::new(config);
    let normal = surface.compute_normal(DVec3::new(100.0, 0.0, 200.0));
    assert_eq!(normal, DVec3::Y);
}

#[test]
fn ocean_surface_fresnel_grazing_angle() {
    let surface = OceanSurface::new(OceanConfig::default());
    let normal = DVec3::Y;

    // 垂直向下看 → 低反射
    let view_down = DVec3::Y;
    let fresnel_down = surface.compute_fresnel(view_down, normal);

    // 以掠射角看 → 高反射
    let view_grazing = DVec3::new(1.0, 0.01, 0.0).normalize();
    let fresnel_grazing = surface.compute_fresnel(view_grazing, normal);

    assert!(fresnel_grazing > fresnel_down);
}

#[test]
fn ocean_surface_update_time() {
    let mut surface = OceanSurface::new(OceanConfig::default());
    assert!((surface.time).abs() < 1e-10);
    surface.update(1.5);
    assert!((surface.time - 1.5).abs() < 1e-10);
    surface.update(0.5);
    assert!((surface.time - 2.0).abs() < 1e-10);
}

#[test]
fn ocean_surface_generate_wind_waves() {
    let mut surface = OceanSurface::new(OceanConfig::default());
    surface.wind_speed = 15.0;
    surface.wind_direction = DVec3::new(1.0, 0.0, 0.0);
    surface.generate_wind_waves();

    assert_eq!(surface.config.waves.len(), 8);
    // 所有波浪都应有正波长
    for wave in &surface.config.waves {
        assert!(wave.wavelength > 0.0);
        assert!(wave.amplitude > 0.0);
    }
}
