//! cesium-shadow：阴影贴图与水/海洋效果。
//!
//! 领域层 —— 纯 Rust，f64 精度。
//!
//! 包含两个子模块：
//! - [`shadow_map`]：级联阴影贴图的配置、偏移与过滤
//! - [`water`]：水/海洋表面渲染参数

pub mod shadow_map;
pub mod water;

pub use shadow_map::{
    PcfConfig, ShadowBias, ShadowBiasType, ShadowCameraParams, ShadowCascade,
    ShadowLightType, ShadowMap, ShadowMapConfig, ShadowMapType,
    SHADOW_MAP_MAXIMUM_DISTANCE,
};
pub use water::{GerstnerWave, OceanConfig, OceanSurface};
