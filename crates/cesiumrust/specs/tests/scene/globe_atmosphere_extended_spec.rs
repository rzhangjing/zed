//! Globe atmosphere 扩展 specs - GroundAtmosphere/GlobeLighting/SkyAtmosphere/SkyBox
//! 参考自 Scene/SkyAtmosphereSpec + Scene/GlobeSpec（A 类散射）

use cesium_globe::atmosphere::{
    GroundAtmosphere, GlobeLighting, SkyAtmosphereConfig, SkyBoxConfig,
};
use glam::DVec3;

// ─── GroundAtmosphere ───────────────────────────────────────────────────────

#[test]
fn ground_atmosphere_defaults() {
    let atm = GroundAtmosphere::default();
    assert!(atm.rayleigh_coefficients[0] > 0.0);
    assert!(atm.rayleigh_coefficients[1] > 0.0);
    assert!(atm.rayleigh_coefficients[2] > 0.0);
    // 蓝色通道应最强（蓝天）
    assert!(atm.rayleigh_coefficients[2] > atm.rayleigh_coefficients[0]);
    assert!(atm.mie_coefficient > 0.0);
    assert!(atm.mie_g > 0.0 && atm.mie_g < 1.0);
    assert!(atm.scale_height > 0.0);
    assert!(atm.sun_intensity > 0.0);
}

#[test]
fn sky_color_looking_at_sun() {
    let atm = GroundAtmosphere::default();
    let view_dir = DVec3::X; // 朝向太阳看
    let sun_dir = DVec3::X;
    let color = atm.compute_sky_color(view_dir, sun_dir, 0.0);

    // 应当明亮（正视太阳）
    assert!(color[0] > 0.0);
    assert!(color[1] > 0.0);
    assert!(color[2] > 0.0);
}

#[test]
fn sky_color_looking_away_from_sun() {
    let atm = GroundAtmosphere::default();
    let view_dir = -DVec3::X; // 背向太阳看
    let sun_dir = DVec3::X;
    let color = atm.compute_sky_color(view_dir, sun_dir, 0.0);

    // 仍应有一些颜色（散射光）
    assert!(color[0] >= 0.0);
    assert!(color[1] >= 0.0);
    assert!(color[2] >= 0.0);
}

#[test]
fn sky_color_blue_channel_dominant() {
    let atm = GroundAtmosphere::default();
    let view_dir = DVec3::new(0.0, 1.0, 0.0); // 垂直于太阳方向
    let sun_dir = DVec3::X;
    let color = atm.compute_sky_color(view_dir, sun_dir, 0.0);

    // 由于瑞利散射，蓝色应占主导
    // （除非颜色全为零，这不应发生）
    if color[0] + color[1] + color[2] > 0.001 {
        assert!(color[2] >= color[0], "blue >= red for Rayleigh scattering");
    }
}

#[test]
fn sky_color_higher_altitude_dimmer() {
    let atm = GroundAtmosphere::default();
    let view_dir = DVec3::X;
    let sun_dir = DVec3::X;

    let color_low = atm.compute_sky_color(view_dir, sun_dir, 0.0);
    let color_high = atm.compute_sky_color(view_dir, sun_dir, 50000.0);

    // 在更高海拔，大气更稀薄 → 更暗
    let brightness_low: f64 = color_low.iter().sum();
    let brightness_high: f64 = color_high.iter().sum();
    assert!(brightness_high < brightness_low, "higher altitude should be dimmer");
}

#[test]
fn sky_color_all_channels_clamped() {
    let atm = GroundAtmosphere::default();
    let view_dir = DVec3::X;
    let sun_dir = DVec3::X;
    let color = atm.compute_sky_color(view_dir, sun_dir, 0.0);

    for &c in &color {
        assert!(c >= 0.0 && c <= 1.0, "color channel must be in [0,1]");
    }
}

// ─── 地平线辉光 ───────────────────────────────────────────────────────────

#[test]
fn horizon_glow_at_sunset() {
    let atm = GroundAtmosphere::default();
    // 太阳位于地平线（elevation = 0）
    let glow = atm.compute_horizon_glow(0.0);

    // 应当有强烈的橙/红辉光
    assert!(glow[0] > 0.5, "red channel strong at sunset");
    assert!(glow[0] > glow[1], "red > green for sunset glow");
    assert!(glow[1] > glow[2], "green > blue for sunset glow");
}

#[test]
fn horizon_glow_high_sun() {
    let atm = GroundAtmosphere::default();
    // 太阳高悬于地平线之上
    let glow = atm.compute_horizon_glow(1.0); // 约 57 度

    // 应当非常暗（指数衰减）
    assert!(glow[0] < 0.01, "glow should be dim when sun is high");
}

#[test]
fn horizon_glow_below_horizon() {
    let atm = GroundAtmosphere::default();
    // 太阳位于地平线之下
    let glow = atm.compute_horizon_glow(-0.5);

    // 仍应有一些辉光（暮光）
    assert!(glow[0] > 0.0);
}

// ─── 天顶颜色 ───────────────────────────────────────────────────────────

#[test]
fn zenith_color_daytime() {
    let atm = GroundAtmosphere::default();
    let color = atm.compute_zenith_color(1.0); // 太阳高悬

    // 白天是蓝天
    assert!(color[2] > color[0], "blue > red during day");
    assert!(color[2] > color[1], "blue > green during day");
}

#[test]
fn zenith_color_night() {
    let atm = GroundAtmosphere::default();
    let color = atm.compute_zenith_color(-1.0); // 太阳位于地平线之下

    // 夜晚黑暗
    assert!(color[0] < 0.01);
    assert!(color[1] < 0.01);
    assert!(color[2] < 0.01);
}

#[test]
fn zenith_color_sunset_transition() {
    let atm = GroundAtmosphere::default();
    let color_low = atm.compute_zenith_color(0.1);
    let color_high = atm.compute_zenith_color(0.5);

    // 太阳越高 → 天顶越亮
    let b_low: f64 = color_low.iter().sum();
    let b_high: f64 = color_high.iter().sum();
    assert!(b_high > b_low);
}

// ─── SkyAtmosphereConfig ────────────────────────────────────────────────────

#[test]
fn sky_atmosphere_config_defaults() {
    let config = SkyAtmosphereConfig::default();
    assert!(config.show);
    assert!((config.hue_shift).abs() < 1e-10);
    assert!((config.saturation_shift).abs() < 1e-10);
    assert!((config.brightness_shift).abs() < 1e-10);
    // 大气半径应 > 地球半径
    assert!(config.atmosphere_radius > 6378137.0);
}

// ─── SkyBoxConfig ───────────────────────────────────────────────────────────

#[test]
fn sky_box_config_defaults() {
    let config = SkyBoxConfig::default();
    assert!(config.show);
    assert!(config.sources.is_none());
    assert!(config.radius > 1e10, "star radius should be very large");
}

// ─── GlobeLighting ──────────────────────────────────────────────────────────

#[test]
fn globe_lighting_defaults() {
    let lighting = GlobeLighting::default();
    assert!(!lighting.enabled);
    assert!((lighting.sun_direction - DVec3::X).length() < 1e-10);
    assert!(lighting.sun_color[0] > 0.9);
    assert!(lighting.ambient_color[0] < 0.2);
    assert!(lighting.specular_intensity > 0.0);
}
