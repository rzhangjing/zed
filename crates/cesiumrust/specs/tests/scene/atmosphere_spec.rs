//! Scene/SkyAtmosphereSpec、SunSpec、MoonSpec → Rust 集成测试

use cesium_atmosphere::{
    compute_sun_position_eci, compute_moon_position_eci,
    rayleigh_phase, mie_phase, AU_IN_METERS,
};

// === 常量 ===

#[test]
fn test_au_in_meters() {
    assert!((AU_IN_METERS - 1.495978707e11).abs() < 1e5);
}

// === 太阳位置 ===

#[test]
fn test_sun_position_eci() {
    // J2000 历元
    let pos = compute_sun_position_eci(0.0);
    // 太阳应约 1 AU 远
    let distance = (pos.x * pos.x + pos.y * pos.y + pos.z * pos.z).sqrt();
    assert!((distance - AU_IN_METERS).abs() / AU_IN_METERS < 0.02); // 在 2% 以内
}

// === 月球位置 ===

#[test]
fn test_moon_position_eci() {
    let pos = compute_moon_position_eci(0.0);
    // 月球应约 384,400 km 远
    let distance = (pos.x * pos.x + pos.y * pos.y + pos.z * pos.z).sqrt();
    assert!(distance > 3.5e8 && distance < 4.1e8);
}

// === 散射 ===

#[test]
fn test_rayleigh_phase() {
    // rayleigh_phase(cos_theta) = 3/(16*pi) * (1 + cos^2)
    let phase = rayleigh_phase(1.0); // cos(0) = 1
    let expected = 3.0 / (16.0 * std::f64::consts::PI) * 2.0;
    assert!((phase - expected).abs() < 1e-10);
}

#[test]
fn test_mie_phase() {
    let phase = mie_phase(1.0, 0.9); // cos(0), g=0.9
    assert!(phase > 0.0);
}
