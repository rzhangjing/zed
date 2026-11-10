//! cesium-globe：地球表面渲染与大气。
//!
//! 领域层 - 纯 Rust，f64 精度。
//!
//! 分为两个子模块：[`surface`] 负责地球表面渲染与地形交互，[`atmosphere`] 负责
//! 天空大气散射、天空盒与光照。

pub mod atmosphere;
pub mod surface;

pub use atmosphere::{
    GlobeLighting, GroundAtmosphere, SkyAtmosphereConfig, SkyBoxConfig,
};
pub use surface::{GlobeConfig, GlobeSurface, GlobeTranslucency, NearFarScalar, ShadowMode};
