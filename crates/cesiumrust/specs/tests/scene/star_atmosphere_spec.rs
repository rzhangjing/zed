//! StarSphere + SkyAtmosphere + SkyBox 规范
//! 参考自 CesiumJS Scene/StarSphereSpec + Scene/SkyAtmosphereSpec + Scene/SkyBoxSpec

use cesium_atmosphere::{
    DynamicAtmosphereLighting, HsbShift, SkyAtmosphereConfig, SkyBoxState, Star, StarSphere,
};
use glam::DVec3;
use std::f64::consts::PI;

// ==================== Star ====================

#[test]
fn star_from_degrees_converts_to_radians() {
    let star = Star::from_degrees(180.0, 45.0, 2.0);
    assert!((star.right_ascension - PI).abs() < 1e-10);
    assert!((star.declination - PI / 4.0).abs() < 1e-10);
    assert!((star.magnitude - 2.0).abs() < 1e-10);
    assert!((star.color_temperature - 6500.0).abs() < 1e-10); // 默认白色
}

#[test]
fn star_direction_is_unit_vector() {
    let star = Star::from_degrees(101.287, -16.716, -1.46);
    let dir = star.direction();
    assert!((dir.length() - 1.0).abs() < 1e-10);
}

#[test]
fn star_direction_at_north_pole() {
    let star = Star {
        right_ascension: 0.0,
        declination: PI / 2.0,
        magnitude: 2.0,
        color_temperature: 6500.0,
    };
    let dir = star.direction();
    assert!((dir.z - 1.0).abs() < 1e-10);
    assert!(dir.x.abs() < 1e-10);
    assert!(dir.y.abs() < 1e-10);
}

#[test]
fn star_direction_at_equator_ra0() {
    let star = Star {
        right_ascension: 0.0,
        declination: 0.0,
        magnitude: 0.0,
        color_temperature: 6500.0,
    };
    let dir = star.direction();
    assert!((dir.x - 1.0).abs() < 1e-10);
    assert!(dir.y.abs() < 1e-10);
    assert!(dir.z.abs() < 1e-10);
}

#[test]
fn star_brightness_pogson_scale() {
    let mag0 = Star::from_degrees(0.0, 0.0, 0.0);
    let mag5 = Star::from_degrees(0.0, 0.0, 5.0);
    let mag_neg1 = Star::from_degrees(0.0, 0.0, -1.0);

    // 星等 0 → 亮度 1.0
    assert!((mag0.brightness() - 1.0).abs() < 1e-10);
    // 暗 5 星等 → 暗 100 倍
    assert!((mag5.brightness() - 0.01).abs() < 1e-4);
    // 负星等 → 亮于 1.0
    assert!(mag_neg1.brightness() > 1.0);
}

#[test]
fn star_spectral_color_hot_blue() {
    let star = Star {
        color_temperature: 20000.0,
        ..Star::from_degrees(0.0, 0.0, 0.0)
    };
    let color = star.spectral_color();
    // 高温恒星偏蓝
    assert!(color[2] > color[0]);
}

#[test]
fn star_spectral_color_cool_red() {
    let star = Star {
        color_temperature: 3000.0,
        ..Star::from_degrees(0.0, 0.0, 0.0)
    };
    let color = star.spectral_color();
    // 低温恒星偏红
    assert!(color[0] > color[2]);
}

// ==================== StarSphere ====================

#[test]
fn star_sphere_builtin_catalog_has_20_stars() {
    let sphere = StarSphere::with_builtin_catalog();
    assert_eq!(sphere.star_count(), 20);
    assert!(sphere.show);
    assert!(sphere.use_hdr);
}

#[test]
fn star_sphere_visible_stars_filters_by_magnitude() {
    let mut sphere = StarSphere::default();
    sphere.minimum_magnitude = 0.0;
    sphere.maximum_magnitude = 2.0;
    sphere.add_star(Star::from_degrees(0.0, 0.0, -1.0)); // 过亮
    sphere.add_star(Star::from_degrees(10.0, 10.0, 1.0)); // 可见
    sphere.add_star(Star::from_degrees(20.0, 20.0, 5.0)); // 过暗

    let visible: Vec<_> = sphere.visible_stars().collect();
    assert_eq!(visible.len(), 1);
    assert!((visible[0].magnitude - 1.0).abs() < 1e-10);
}

#[test]
fn star_sphere_point_size_brighter_is_larger() {
    let sphere = StarSphere {
        minimum_magnitude: 0.0,
        maximum_magnitude: 6.0,
        base_point_size: 4.0,
        ..Default::default()
    };
    let bright = Star::from_degrees(0.0, 0.0, 0.0);
    let dim = Star::from_degrees(0.0, 0.0, 6.0);

    let bright_size = sphere.star_point_size(&bright);
    let dim_size = sphere.star_point_size(&dim);

    assert!(bright_size > dim_size);
    assert!((bright_size - 4.0).abs() < 1e-10); // 在最小星等时为完整基础大小
}

#[test]
fn star_sphere_render_color_applies_brightness() {
    let sphere = StarSphere {
        brightness_multiplier: 2.0,
        ..Default::default()
    };
    let star = Star {
        magnitude: 0.0,
        color_temperature: 6500.0,
        ..Star::from_degrees(0.0, 0.0, 0.0)
    };
    let color = sphere.star_render_color(&star);
    // brightness = 10^(-0.4*0) * 2.0 = 2.0，所有通道 > 0
    assert!(color[0] > 0.0);
    assert!(color[1] > 0.0);
    assert!(color[2] > 0.0);
}

// ==================== HsbShift ====================

#[test]
fn hsb_shift_noop_returns_original() {
    let shift = HsbShift::default();
    let color = [0.5, 0.3, 0.8];
    let result = shift.apply(color);
    assert!((result[0] - color[0]).abs() < 1e-10);
    assert!((result[1] - color[1]).abs() < 1e-10);
    assert!((result[2] - color[2]).abs() < 1e-10);
}

#[test]
fn hsb_shift_brightness_decreases() {
    let shift = HsbShift {
        brightness: -0.5,
        ..Default::default()
    };
    let color = [1.0, 0.0, 0.0];
    let result = shift.apply(color);
    assert!(result[0] < 1.0);
}

#[test]
fn hsb_shift_saturation_zero_grayscale() {
    let shift = HsbShift {
        saturation: -1.0,
        ..Default::default()
    };
    let color = [1.0, 0.0, 0.0];
    let result = shift.apply(color);
    // 所有通道应相等（灰度）
    assert!((result[0] - result[1]).abs() < 1e-6);
    assert!((result[1] - result[2]).abs() < 1e-6);
}

// ==================== DynamicAtmosphereLighting ====================

#[test]
fn dynamic_atmosphere_lighting_shader_values() {
    assert!((DynamicAtmosphereLighting::Sun.to_shader_value() - 1.0).abs() < 1e-10);
    assert!((DynamicAtmosphereLighting::Moon.to_shader_value() - 2.0).abs() < 1e-10);
    assert!((DynamicAtmosphereLighting::None.to_shader_value()).abs() < 1e-10);
}

// ==================== SkyAtmosphereConfig ====================

#[test]
fn sky_atmosphere_config_defaults() {
    let config = SkyAtmosphereConfig::default();
    assert!(config.show);
    assert!(!config.per_fragment_atmosphere);
    assert!((config.light_intensity - 50.0).abs() < 1e-10);
    assert!((config.mie_anisotropy - 0.9).abs() < 1e-10);
    assert!((config.outer_ellipsoid_scale - 1.025).abs() < 1e-10);
    assert!((config.inner_radius - 6378137.0).abs() < 1e-10);
}

#[test]
fn sky_atmosphere_outer_radius() {
    let config = SkyAtmosphereConfig::default();
    let expected = 6378137.0 * 1.025;
    assert!((config.outer_radius() - expected).abs() < 1.0);
}

#[test]
fn sky_atmosphere_compute_color_nonzero() {
    let config = SkyAtmosphereConfig::default();
    let view = DVec3::new(0.0, 0.0, 1.0);
    let sun = DVec3::new(0.0, 0.0, 1.0);
    let color = config.compute_color(view, sun, 0.0);
    assert!(color[0] > 0.0 || color[1] > 0.0 || color[2] > 0.0);
}

#[test]
fn sky_atmosphere_hidden_returns_black() {
    let config = SkyAtmosphereConfig {
        show: false,
        ..Default::default()
    };
    let color = config.compute_color(DVec3::Z, DVec3::Z, 0.0);
    assert_eq!(color, [0.0; 3]);
}

#[test]
fn sky_atmosphere_radii_and_dynamic_color() {
    let config = SkyAtmosphereConfig::default();
    let v = config.radii_and_dynamic_color();
    assert!((v.x - config.outer_radius()).abs() < 1e-6);
    assert!((v.y - config.inner_radius).abs() < 1e-6);
    assert!((v.z - 1.0).abs() < 1e-10); // Sun = 1.0
}

// ==================== SkyBoxState ====================

#[test]
fn sky_box_teme_identity_at_gmst_zero() {
    let sky_box = SkyBoxState::default();
    let rot = sky_box.teme_to_ecef_rotation(0.0);
    assert!((rot[0][0] - 1.0).abs() < 1e-10);
    assert!((rot[1][1] - 1.0).abs() < 1e-10);
    assert!((rot[2][2] - 1.0).abs() < 1e-10);
    assert!(rot[0][1].abs() < 1e-10);
}

#[test]
fn sky_box_teme_to_ecef_rotates_x() {
    let sky_box = SkyBoxState::default();
    let dir = DVec3::new(1.0, 0.0, 0.0);

    // 在 GMST=0 时，不变
    let ecef = sky_box.teme_to_ecef(dir, 0.0);
    assert!((ecef - dir).length() < 1e-10);

    // 在 GMST=π/2 时，X 旋转到 -Y
    let ecef_90 = sky_box.teme_to_ecef(dir, PI / 2.0);
    assert!(ecef_90.x.abs() < 1e-10);
    assert!((ecef_90.y - (-1.0)).abs() < 1e-10);
}

#[test]
fn sky_box_is_complete() {
    let mut sky_box = SkyBoxState::default();
    assert!(!sky_box.is_complete());

    sky_box.sources = [
        Some("px.png".into()),
        Some("nx.png".into()),
        Some("py.png".into()),
        Some("ny.png".into()),
        Some("pz.png".into()),
        Some("nz.png".into()),
    ];
    assert!(sky_box.is_complete());
}
