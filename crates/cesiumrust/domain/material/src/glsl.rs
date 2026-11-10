//! 内置材质的 GLSL 着色器源码常量集合。
//!
//! 每个常量以 include_str! 引入 ../shaders/ 目录下对应的着色器文件，
//! 领域层逐字存储并按 Fabric 规则组合这些源码（纯文本处理）。
//! 渲染适配器负责在渲染时将它们翻译为目标着色语言。

/// 宽高比渐变材质的 GLSL 着色器源码。
pub const ASPECT_RAMP_MATERIAL: &str = include_str!("../shaders/AspectRampMaterial.glsl");
/// 凹凸贴图材质的 GLSL 着色器源码。
pub const BUMP_MAP_MATERIAL: &str = include_str!("../shaders/BumpMapMaterial.glsl");
/// 棋盘格材质的 GLSL 着色器源码。
pub const CHECKERBOARD_MATERIAL: &str = include_str!("../shaders/CheckerboardMaterial.glsl");
/// 点阵材质的 GLSL 着色器源码。
pub const DOT_MATERIAL: &str = include_str!("../shaders/DotMaterial.glsl");
/// 高程分层材质的 GLSL 着色器源码。
pub const ELEVATION_BAND_MATERIAL: &str = include_str!("../shaders/ElevationBandMaterial.glsl");
/// 高程等高线材质的 GLSL 着色器源码。
pub const ELEVATION_CONTOUR_MATERIAL: &str =
    include_str!("../shaders/ElevationContourMaterial.glsl");
/// 高程渐变材质的 GLSL 着色器源码。
pub const ELEVATION_RAMP_MATERIAL: &str = include_str!("../shaders/ElevationRampMaterial.glsl");
/// 淡出材质的 GLSL 着色器源码。
pub const FADE_MATERIAL: &str = include_str!("../shaders/FadeMaterial.glsl");
/// 网格材质的 GLSL 着色器源码。
pub const GRID_MATERIAL: &str = include_str!("../shaders/GridMaterial.glsl");
/// 法线贴图材质的 GLSL 着色器源码。
pub const NORMAL_MAP_MATERIAL: &str = include_str!("../shaders/NormalMapMaterial.glsl");
/// 箭头线材质的 GLSL 着色器源码。
pub const POLYLINE_ARROW_MATERIAL: &str = include_str!("../shaders/PolylineArrowMaterial.glsl");
/// 虚线材质的 GLSL 着色器源码。
pub const POLYLINE_DASH_MATERIAL: &str = include_str!("../shaders/PolylineDashMaterial.glsl");
/// 发光线材质的 GLSL 着色器源码。
pub const POLYLINE_GLOW_MATERIAL: &str = include_str!("../shaders/PolylineGlowMaterial.glsl");
/// 描边线材质的 GLSL 着色器源码。
pub const POLYLINE_OUTLINE_MATERIAL: &str =
    include_str!("../shaders/PolylineOutlineMaterial.glsl");
/// 边缘光照材质的 GLSL 着色器源码。
pub const RIM_LIGHTING_MATERIAL: &str = include_str!("../shaders/RimLightingMaterial.glsl");
/// 坡度渐变材质的 GLSL 着色器源码。
pub const SLOPE_RAMP_MATERIAL: &str = include_str!("../shaders/SlopeRampMaterial.glsl");
/// 条纹材质的 GLSL 着色器源码。
pub const STRIPE_MATERIAL: &str = include_str!("../shaders/StripeMaterial.glsl");
/// 水面材质的 GLSL 着色器源码。
pub const WATER_MATERIAL: &str = include_str!("../shaders/Water.glsl");
/// 水面遮罩材质的 GLSL 着色器源码。
pub const WATER_MASK_MATERIAL: &str = include_str!("../shaders/WaterMaskMaterial.glsl");
