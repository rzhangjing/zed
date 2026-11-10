//! 阴影贴图扩展规范 - 测试 ShadowMap、ShadowBias 及级联计算
//!
//! 覆盖：bias 构造、阴影贴图构造、淡出因子、级联分层

use cesium_shadow::{ShadowBias, ShadowMap, ShadowMapConfig};
use glam::DVec3;

const EPSILON3: f64 = 1e-3;
const EPSILON6: f64 = 1e-6;

// ─── ShadowBias 构造 ─────────────────────────────────────────────────

#[test]
fn shadow_bias_terrain_default() {
    let bias = ShadowBias::terrain(false);
    assert!(bias.polygon_offset);
    assert!(bias.polygon_offset_factor > 0.0);
    assert!(bias.polygon_offset_units > 0.0);
    assert!(!bias.normal_offset);
    assert!(bias.depth_bias > 0.0);
}

#[test]
fn shadow_bias_terrain_with_normal_offset() {
    let bias = ShadowBias::terrain(true);
    assert!(bias.normal_offset);
    assert!(bias.normal_offset_scale > 0.0);
}

#[test]
fn shadow_bias_primitive_default() {
    let bias = ShadowBias::primitive(false);
    assert!(bias.polygon_offset);
    assert!(!bias.normal_offset);
    assert!(bias.depth_bias > 0.0);
}

#[test]
fn shadow_bias_primitive_with_normal_offset() {
    let bias = ShadowBias::primitive(true);
    assert!(bias.normal_offset);
}

#[test]
fn shadow_bias_point_default() {
    let bias = ShadowBias::point(false);
    assert!(bias.depth_bias > 0.0);
}

#[test]
fn shadow_bias_terrain_vs_primitive() {
    let terrain = ShadowBias::terrain(false);
    let primitive = ShadowBias::primitive(false);
    // 地形的 depth bias 应大于图元
    assert!(
        terrain.depth_bias > primitive.depth_bias,
        "terrain depth_bias {} should be > primitive {}",
        terrain.depth_bias,
        primitive.depth_bias
    );
}

// ─── ShadowMap 构造 ──────────────────────────────────────────────────

#[test]
fn shadow_map_for_sun() {
    let sun_dir = DVec3::new(0.0, 0.0, -1.0);
    let shadow_map = ShadowMap::for_sun(sun_dir);
    assert!(shadow_map.pass_count() >= 1);
}

#[test]
fn shadow_map_for_point_light() {
    let position = DVec3::new(10.0, 10.0, 10.0);
    let radius = 100.0;
    let shadow_map = ShadowMap::for_point_light(position, radius);
    assert!(shadow_map.pass_count() >= 1);
}

#[test]
fn shadow_map_for_spot_light() {
    let position = DVec3::new(0.0, 10.0, 0.0);
    let direction = DVec3::new(0.0, -1.0, 0.0);
    let shadow_map = ShadowMap::for_spot_light(position, direction);
    assert!(shadow_map.pass_count() >= 1);
}

#[test]
fn shadow_map_new_with_config() {
    let config = ShadowMapConfig::default();
    let light_dir = DVec3::new(0.0, 0.0, -1.0);
    let shadow_map = ShadowMap::new(config, light_dir);
    assert!(shadow_map.pass_count() >= 1);
}

// ─── ShadowMap 淡出 ──────────────────────────────────────────────────────────

#[test]
fn shadow_map_fade_factor_noon() {
    let config = ShadowMapConfig::default();
    let shadow_map = ShadowMap::new(config, DVec3::new(0.0, 0.0, -1.0));
    let fade = shadow_map.compute_fade_factor(std::f64::consts::FRAC_PI_2);
    // 正午（高仰角）应有完整阴影
    assert!(
        (fade - 1.0).abs() < EPSILON3,
        "noon fade should be ~1.0, got {}",
        fade
    );
}

#[test]
fn shadow_map_fade_factor_sunset() {
    let config = ShadowMapConfig::default();
    let shadow_map = ShadowMap::new(config, DVec3::new(1.0, 0.0, 0.0));
    let fade = shadow_map.compute_fade_factor(0.0);
    // 日落（低仰角）应淡出阴影
    assert!(
        fade < 1.0,
        "sunset fade should be < 1.0, got {}",
        fade
    );
}

#[test]
fn shadow_map_fade_factor_night() {
    let config = ShadowMapConfig::default();
    let shadow_map = ShadowMap::new(config, DVec3::new(0.0, 0.0, 1.0));
    let fade = shadow_map.compute_fade_factor(-std::f64::consts::FRAC_PI_4);
    // 夜晚（负仰角）应无阴影
    assert!(
        fade.abs() < EPSILON3,
        "night fade should be ~0.0, got {}",
        fade
    );
}

// ─── ShadowMap 级联分层 ────────────────────────────────────────────────

#[test]
fn shadow_map_cascade_splits_linear() {
    let config = ShadowMapConfig::default();
    let shadow_map = ShadowMap::new(config, DVec3::new(0.0, 0.0, -1.0));
    let near = 1.0;
    let far = 100.0;
    let lambda = 0.0; // 纯线性
    let splits = shadow_map.compute_cascade_splits(near, far, lambda);
    // 应产生至少 1 个分层
    assert!(!splits.is_empty(), "should produce at least 1 split");
    // 分层值应单调递增
    for i in 1..splits.len() {
        assert!(
            splits[i] > splits[i - 1],
            "splits should be increasing: {} <= {}",
            splits[i],
            splits[i - 1]
        );
    }
}

#[test]
fn shadow_map_cascade_splits_logarithmic() {
    let config = ShadowMapConfig::default();
    let shadow_map = ShadowMap::new(config, DVec3::new(0.0, 0.0, -1.0));
    let near = 1.0;
    let far = 100.0;
    let lambda = 1.0; // 纯对数
    let splits = shadow_map.compute_cascade_splits(near, far, lambda);
    // 应产生至少 1 个分层
    assert!(!splits.is_empty(), "should produce at least 1 split");
    // 对数分层在靠近相机处应更密集
    if splits.len() >= 3 {
        let gap1 = splits[1] - splits[0];
        let gap2 = splits[2] - splits[1];
        assert!(
            gap2 > gap1,
            "log splits should have larger gaps further away: gap1={}, gap2={}",
            gap1,
            gap2
        );
    }
}

#[test]
fn shadow_map_cascade_splits_practical() {
    let config = ShadowMapConfig::default();
    let shadow_map = ShadowMap::new(config, DVec3::new(0.0, 0.0, -1.0));
    let near = 0.1;
    let far = 1000.0;
    let lambda = 0.5; // 实际混合
    let splits = shadow_map.compute_cascade_splits(near, far, lambda);
    // 所有分层应位于 [near, far] 内
    for &split in &splits {
        assert!(
            split >= near && split <= far,
            "split {} out of range [{}, {}]",
            split,
            near,
            far
        );
    }
}

// ─── ShadowMap bias 类型 ────────────────────────────────────────────────────

#[test]
fn shadow_map_bias_for_type_terrain() {
    let config = ShadowMapConfig::default();
    let shadow_map = ShadowMap::new(config, DVec3::new(0.0, 0.0, -1.0));
    let bias = shadow_map.bias_for_type(cesium_shadow::ShadowBiasType::Terrain);
    assert!(bias.depth_bias > 0.0);
}

#[test]
fn shadow_map_bias_for_type_primitive() {
    let config = ShadowMapConfig::default();
    let shadow_map = ShadowMap::new(config, DVec3::new(0.0, 0.0, -1.0));
    let bias = shadow_map.bias_for_type(cesium_shadow::ShadowBiasType::Primitive);
    assert!(bias.depth_bias > 0.0);
}

// ─── ShadowMap 更新 ────────────────────────────────────────────────────────

#[test]
fn shadow_map_update_fade() {
    let config = ShadowMapConfig::default();
    let mut shadow_map = ShadowMap::new(config, DVec3::new(0.0, 0.0, -1.0));
    shadow_map.update_fade(std::f64::consts::FRAC_PI_4);
    // 应更新内部淡出状态
    let fade = shadow_map.compute_fade_factor(std::f64::consts::FRAC_PI_4);
    assert!(fade >= 0.0 && fade <= 1.0);
}

#[test]
fn shadow_map_pass_count_cascades() {
    let mut config = ShadowMapConfig::default();
    config.cascade_count = 4;
    let shadow_map = ShadowMap::new(config, DVec3::new(0.0, 0.0, -1.0));
    assert_eq!(shadow_map.pass_count(), 4);
}
