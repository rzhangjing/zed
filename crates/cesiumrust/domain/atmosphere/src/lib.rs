//! cesium-atmosphere：大气与天体领域模型
//!
//! 涵盖天空大气散射、天空盒、太阳与月亮等天体渲染的领域模型，
//! 以及基于简化行星理论的太阳/月亮位置计算。
//!
//! # 特性
//! - 太阳/月亮位置计算（简化的 VSOP87/月球理论）
//! - ECI ↔ ECEF 坐标变换
//! - Rayleigh/Mie 大气散射模型
//! - 天空颜色计算
//! - 光照配置

pub mod celestial;
pub mod scattering;
pub mod star_sphere;

pub use celestial::{
    compute_sun_position_eci, compute_sun_position_ecef,
    compute_sun_direction_eci,
    compute_moon_position_eci, compute_moon_position_ecef,
    compute_moon_direction_eci,
    compute_gmst, eci_to_ecef,
    AU_IN_METERS, J2000_EPOCH,
};
pub use scattering::{
    AtmosphereParameters, SkyBoxConfig, LightingConfig,
    rayleigh_phase, mie_phase, atmospheric_density,
    compute_sky_color, compute_horizon_glow,
};
pub use star_sphere::{
    DynamicAtmosphereLighting, HsbShift, SkyAtmosphereConfig, SkyBoxState,
    Star, StarSphere,
};
