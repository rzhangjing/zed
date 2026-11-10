//! 天体 + 大气散射规格
//! 参考自 CesiumJS Core/Simon1994PlanetaryPositionsSpec + Scene/SkyAtmosphereSpec

use cesium_atmosphere::{
    atmospheric_density, compute_gmst, compute_horizon_glow, compute_moon_direction_eci,
    compute_moon_position_eci, compute_moon_position_ecef, compute_sky_color,
    compute_sun_direction_eci, compute_sun_position_eci, compute_sun_position_ecef, eci_to_ecef,
    mie_phase, rayleigh_phase, AtmosphereParameters, LightingConfig, AU_IN_METERS, J2000_EPOCH,
};
use glam::DVec3;
use std::f64::consts::PI;

// ==================== 天体：太阳 ====================

#[test]
fn sun_position_at_j2000_approximately_1au() {
    let pos = compute_sun_position_eci(J2000_EPOCH);
    let dist = pos.length();
    // 在 1 AU 的 2% 以内
    assert!((dist - AU_IN_METERS).abs() / AU_IN_METERS < 0.02);
}

#[test]
fn sun_direction_is_normalized() {
    let dir = compute_sun_direction_eci(J2000_EPOCH);
    assert!((dir.length() - 1.0).abs() < 1e-10);
}

#[test]
fn sun_position_ecef_same_magnitude_as_eci() {
    let eci = compute_sun_position_eci(J2000_EPOCH);
    let ecef = compute_sun_position_ecef(J2000_EPOCH);
    assert!((eci.length() - ecef.length()).abs() / eci.length() < 1e-10);
}

#[test]
fn sun_position_varies_with_date() {
    let pos1 = compute_sun_position_eci(J2000_EPOCH);
    let pos2 = compute_sun_position_eci(J2000_EPOCH + 182.5); // 约 6 个月后
    // Direction 应显著不同
    let dot = pos1.normalize().dot(pos2.normalize());
    assert!(dot < 0.5); // 并非同一方向
}

// ==================== 天体：月球 ====================

#[test]
fn moon_position_distance_range() {
    let pos = compute_moon_position_eci(J2000_EPOCH);
    let dist_km = pos.length() / 1000.0;
    // 月球：356,000 - 407,000 km
    assert!(dist_km > 350_000.0);
    assert!(dist_km < 410_000.0);
}

#[test]
fn moon_direction_normalized() {
    let dir = compute_moon_direction_eci(J2000_EPOCH);
    assert!((dir.length() - 1.0).abs() < 1e-10);
}

#[test]
fn moon_position_ecef_preserves_distance() {
    let eci = compute_moon_position_eci(J2000_EPOCH);
    let ecef = compute_moon_position_ecef(J2000_EPOCH);
    assert!((eci.length() - ecef.length()).abs() / eci.length() < 1e-10);
}

// ==================== 天体：GMST + ECI→ECEF ====================

#[test]
fn gmst_in_valid_range() {
    let gmst = compute_gmst(J2000_EPOCH);
    assert!(gmst >= 0.0);
    assert!(gmst < 2.0 * PI);
}

#[test]
fn gmst_advances_with_time() {
    let g1 = compute_gmst(J2000_EPOCH);
    let g2 = compute_gmst(J2000_EPOCH + 1.0); // +1 天
    // 地球每天自转约 360.98°，因此 GMST 应前进
    assert!((g2 - g1).abs() > 0.01); // 至少有一些变化
}

#[test]
fn eci_to_ecef_preserves_magnitude() {
    let eci = DVec3::new(1.0e11, 2.0e10, 3.0e10);
    let ecef = eci_to_ecef(eci, J2000_EPOCH);
    assert!((eci.length() - ecef.length()).abs() / eci.length() < 1e-10);
}

#[test]
fn eci_to_ecef_z_unchanged() {
    // GMST 旋转绕 Z 轴，因此 Z 分量应保持不变
    let eci = DVec3::new(1.0e11, 2.0e10, 5.0e10);
    let ecef = eci_to_ecef(eci, J2000_EPOCH);
    assert!((eci.z - ecef.z).abs() < 1.0); // Z 不变
}

// ==================== 散射：相函数 ====================

#[test]
fn rayleigh_phase_symmetry() {
    // Rayleigh 相位对称：P(cos) = P(-cos)
    let forward = rayleigh_phase(1.0);
    let backward = rayleigh_phase(-1.0);
    assert!((forward - backward).abs() < 1e-15);
}

#[test]
fn rayleigh_phase_perpendicular() {
    // 在 90 度（cos_theta=0）：P = 3/(16*pi)
    let p = rayleigh_phase(0.0);
    let expected = 3.0 / (16.0 * PI);
    assert!((p - expected).abs() < 1e-15);
}

#[test]
fn mie_phase_forward_scattering() {
    // g 为正时，前向散射（cos_theta=1）应更强
    let forward = mie_phase(1.0, 0.758);
    let backward = mie_phase(-1.0, 0.758);
    assert!(forward > backward);
}

#[test]
fn mie_phase_g_zero_still_varies() {
    // g=0 消除不对称，但公式仍含 (1+cos²θ) 项
    let forward = mie_phase(1.0, 0.0);
    let perp = mie_phase(0.0, 0.0);
    // 前向 (cos²=1) → (1+1)=2，垂直 (cos²=0) → (1+0)=1
    assert!(forward > perp);
    // 对称：前向 == 后向
    let backward = mie_phase(-1.0, 0.0);
    assert!((forward - backward).abs() < 1e-15);
}

// ==================== 散射：密度 ====================

#[test]
fn atmospheric_density_at_surface_is_one() {
    let d = atmospheric_density(0.0, 8000.0);
    assert!((d - 1.0).abs() < 1e-15);
}

#[test]
fn atmospheric_density_decays_with_height() {
    let d0 = atmospheric_density(0.0, 8000.0);
    let d8k = atmospheric_density(8000.0, 8000.0);
    let d16k = atmospheric_density(16000.0, 8000.0);
    assert!(d0 > d8k);
    assert!(d8k > d16k);
    // 在一个尺度高度处：e^-1
    assert!((d8k - (-1.0_f64).exp()).abs() < 1e-10);
}

// ==================== 散射：天空颜色 ====================

#[test]
fn sky_color_nonzero_toward_sun() {
    let params = AtmosphereParameters::default();
    let view = DVec3::new(1.0, 0.0, 0.0);
    let sun = DVec3::new(1.0, 0.0, 0.0); // 朝向太阳看
    let color = compute_sky_color(view, sun, 0.0, &params);
    assert!(color[0] > 0.0);
    assert!(color[1] > 0.0);
    assert!(color[2] > 0.0);
}

#[test]
fn sky_color_blue_dominant_at_surface() {
    let params = AtmosphereParameters::default();
    let view = DVec3::new(0.0, 0.0, 1.0);
    let sun = DVec3::new(1.0, 0.0, 0.0);
    let color = compute_sky_color(view, sun, 0.0, &params);
    // 由于 Rayleigh，蓝色（索引 2）应最强
    assert!(color[2] > color[0]); // 蓝 > 红
}

// ==================== 地平线辉光 ====================

#[test]
fn horizon_glow_sunset_reddish() {
    // 太阳略低于地平线（elevation < 0）
    let glow = compute_horizon_glow(-0.1);
    // 应为暖色调（R > B）
    assert!(glow[0] > glow[2]);
}

#[test]
fn horizon_glow_high_sun_bluish() {
    // 太阳高悬（elevation = pi/2）
    let glow = compute_horizon_glow(PI / 2.0);
    // 应偏蓝
    assert!(glow[2] > glow[0]);
}

// ==================== LightingConfig ====================

#[test]
fn lighting_config_sun_elevation() {
    let mut config = LightingConfig::default();
    config.sun_direction = DVec3::new(0.0, 0.0, 1.0); // 太阳位于正上方
    let surface_normal = DVec3::new(0.0, 0.0, 1.0);
    let elevation = config.sun_elevation_at(surface_normal);
    // 太阳在头顶 → elevation = pi/2
    assert!((elevation - PI / 2.0).abs() < 1e-10);
}

#[test]
fn lighting_config_sun_at_horizon() {
    let mut config = LightingConfig::default();
    config.sun_direction = DVec3::new(1.0, 0.0, 0.0); // 太阳在地平线
    let surface_normal = DVec3::new(0.0, 0.0, 1.0);
    let elevation = config.sun_elevation_at(surface_normal);
    // 太阳垂直于法线 → elevation = 0
    assert!((elevation).abs() < 1e-10);
}
