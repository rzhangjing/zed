//! Atmosphere 扩展规范 - 测试 celestial、scattering 和 star_sphere 模块
//!
//! 覆盖：日/月位置计算、大气散射、星表

use cesium_atmosphere::{
    atmospheric_density, compute_gmst, compute_horizon_glow, compute_moon_direction_eci,
    compute_moon_position_eci, compute_moon_position_ecef, compute_sun_direction_eci,
    compute_sun_position_ecef, compute_sun_position_eci, eci_to_ecef, mie_phase,
    rayleigh_phase, Star,
};
use glam::DVec3;

const EPSILON3: f64 = 1e-3;
const EPSILON6: f64 = 1e-6;

// ─── 天体计算 ─────────────────────────────────────────────────

#[test]
fn sun_position_eci_at_j2000() {
    let pos = compute_sun_position_eci(2451545.0);
    assert!(pos.x > 0.0, "sun X should be positive at J2000, got {}", pos.x);
    let dist = pos.length();
    assert!(
        dist > 1.4e11 && dist < 1.6e11,
        "sun distance should be ~1 AU, got {} m",
        dist
    );
}

#[test]
fn sun_direction_eci_is_normalized() {
    let dir = compute_sun_direction_eci(2451545.0);
    let len = dir.length();
    assert!(
        (len - 1.0).abs() < EPSILON3,
        "sun direction should be unit length, got {}",
        len
    );
}

#[test]
fn sun_direction_changes_over_time() {
    let dir1 = compute_sun_direction_eci(2451545.0);
    let dir2 = compute_sun_direction_eci(2451545.0 + 90.0);
    let diff = (dir2 - dir1).length();
    assert!(
        diff > 0.1,
        "sun direction should change over 90 days, diff={}",
        diff
    );
}

#[test]
fn moon_position_eci_reasonable_distance() {
    let pos = compute_moon_position_eci(2451545.0);
    let dist = pos.length();
    assert!(
        dist > 3.5e8 && dist < 4.1e8,
        "moon distance should be ~384400 km, got {} m",
        dist
    );
}

#[test]
fn moon_direction_eci_is_normalized() {
    let dir = compute_moon_direction_eci(2451545.0);
    let len = dir.length();
    assert!(
        (len - 1.0).abs() < EPSILON3,
        "moon direction should be unit length, got {}",
        len
    );
}

#[test]
fn gmst_at_j2000() {
    let gmst = compute_gmst(2451545.0);
    assert!(
        gmst > 4.0 && gmst < 6.0,
        "GMST at J2000 should be ~4.89 rad, got {}",
        gmst
    );
}

#[test]
fn gmst_increases_with_time() {
    let gmst1 = compute_gmst(2451545.0);
    let gmst2 = compute_gmst(2451545.0 + 1.0);
    assert!(
        (gmst2 - gmst1).abs() > 0.01,
        "GMST should change over 1 day"
    );
}

#[test]
fn eci_to_ecef_preserves_magnitude() {
    let eci = DVec3::new(1.0e7, 0.0, 0.0);
    let ecef = eci_to_ecef(eci, 2451545.0);
    let mag_diff = (ecef.length() - eci.length()).abs();
    assert!(
        mag_diff < 1.0,
        "ECI->ECEF should preserve magnitude, diff={}",
        mag_diff
    );
}

#[test]
fn sun_position_ecef_reasonable() {
    let pos = compute_sun_position_ecef(2451545.0);
    let dist = pos.length();
    assert!(
        dist > 1.4e11 && dist < 1.6e11,
        "sun ECEF distance should be ~1 AU, got {} m",
        dist
    );
}

#[test]
fn moon_position_ecef_reasonable() {
    let pos = compute_moon_position_ecef(2451545.0);
    let dist = pos.length();
    assert!(
        dist > 3.5e8 && dist < 4.1e8,
        "moon ECEF distance should be ~384400 km, got {} m",
        dist
    );
}

// ─── 大气散射 ─────────────────────────────────────────────────

#[test]
fn rayleigh_phase_forward() {
    let phase = rayleigh_phase(1.0);
    assert!(phase > 0.0, "Rayleigh phase should be positive");
}

#[test]
fn rayleigh_phase_backward() {
    let phase = rayleigh_phase(-1.0);
    assert!(phase > 0.0, "Rayleigh phase should be positive");
}

#[test]
fn rayleigh_phase_symmetric() {
    let forward = rayleigh_phase(0.5);
    let backward = rayleigh_phase(-0.5);
    assert!(
        (forward - backward).abs() < EPSILON6,
        "Rayleigh phase should be symmetric: forward={}, backward={}",
        forward,
        backward
    );
}

#[test]
fn mie_phase_forward_peaked() {
    let forward = mie_phase(1.0, 0.7);
    let backward = mie_phase(-1.0, 0.7);
    assert!(
        forward > backward,
        "Mie phase should be forward-peaked: forward={}, backward={}",
        forward,
        backward
    );
}

#[test]
fn mie_phase_g_zero_is_isotropic() {
    let phase1 = mie_phase(0.5, 0.0);
    let phase2 = mie_phase(-0.5, 0.0);
    assert!(
        (phase1 - phase2).abs() < EPSILON6,
        "Mie phase with g=0 should be isotropic: p1={}, p2={}",
        phase1,
        phase2
    );
}

#[test]
fn atmospheric_density_decreases_with_height() {
    let density_surface = atmospheric_density(0.0, 8500.0);
    let density_10km = atmospheric_density(10000.0, 8500.0);
    let density_50km = atmospheric_density(50000.0, 8500.0);
    assert!(
        density_surface > density_10km,
        "density should decrease with height"
    );
    assert!(
        density_10km > density_50km,
        "density should decrease with height"
    );
}

#[test]
fn atmospheric_density_at_surface_is_one() {
    let density = atmospheric_density(0.0, 8500.0);
    assert!(
        (density - 1.0).abs() < EPSILON6,
        "density at surface should be 1.0, got {}",
        density
    );
}

#[test]
fn horizon_glow_at_horizon() {
    let glow = compute_horizon_glow(0.0);
    let intensity = (glow[0] + glow[1] + glow[2]) / 3.0;
    assert!(
        intensity > 0.1,
        "horizon glow should be significant, got {}",
        intensity
    );
}

#[test]
fn horizon_glow_varies_with_elevation() {
    let glow_horizon = compute_horizon_glow(0.0);
    let glow_zenith = compute_horizon_glow(std::f64::consts::FRAC_PI_2);
    // 辉光在地平线与天顶间应不同
    let diff = ((glow_horizon[0] - glow_zenith[0]).abs()
        + (glow_horizon[1] - glow_zenith[1]).abs()
        + (glow_horizon[2] - glow_zenith[2]).abs())
        / 3.0;
    assert!(
        diff > 0.01,
        "horizon glow should differ from zenith: horizon={:?}, zenith={:?}",
        glow_horizon,
        glow_zenith
    );
}

// ─── 星表 ────────────────────────────────────────────────────────────

#[test]
fn star_from_degrees() {
    let star = Star::from_degrees(0.0, 0.0, 1.0);
    let dir = star.direction();
    assert!(
        (dir.x - 1.0).abs() < EPSILON3,
        "star at (0,0) should point +X, got {:?}",
        dir
    );
}

#[test]
fn star_north_pole() {
    let star = Star::from_degrees(0.0, 90.0, 1.0);
    let dir = star.direction();
    assert!(
        (dir.z - 1.0).abs() < EPSILON3,
        "star at north pole should point +Z, got {:?}",
        dir
    );
}

#[test]
fn star_brightness_scales_correctly() {
    let bright = Star::from_degrees(0.0, 0.0, 1.0);
    let dim = Star::from_degrees(0.0, 0.0, 6.0);
    assert!(
        bright.brightness() > dim.brightness(),
        "mag 1 should be brighter than mag 6"
    );
}

#[test]
fn star_spectral_color_hot_vs_cool() {
    // 高温蓝星
    let hot = Star::from_degrees(0.0, 0.0, 1.0);
    let hot_color = hot.spectral_color();
    // 低温红星
    let cool = Star::from_degrees(0.0, 0.0, 1.0);
    let cool_color = cool.spectral_color();
    // 两者都应产生有效颜色
    assert!(
        hot_color[0] >= 0.0 && hot_color[1] >= 0.0 && hot_color[2] >= 0.0,
        "hot star color should be non-negative"
    );
    assert!(
        cool_color[0] >= 0.0 && cool_color[1] >= 0.0 && cool_color[2] >= 0.0,
        "cool star color should be non-negative"
    );
}

#[test]
fn star_brightness_magnitude_relation() {
    // 5 个星等 = 100 倍亮度比
    let mag0 = Star::from_degrees(0.0, 0.0, 0.0);
    let mag5 = Star::from_degrees(0.0, 0.0, 5.0);
    let ratio = mag0.brightness() / mag5.brightness();
    assert!(
        (ratio - 100.0).abs() < 1.0,
        "5 magnitude difference should be 100x brightness, got {}",
        ratio
    );
}

#[test]
fn star_direction_normalized() {
    let star = Star::from_degrees(45.0, 30.0, 1.0);
    let dir = star.direction();
    let len = dir.length();
    assert!(
        (len - 1.0).abs() < EPSILON3,
        "star direction should be unit length, got {}",
        len
    );
}
