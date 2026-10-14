//! Fabric 程序化材质适配器（bevy-render）。
//!
//! 将领域 [`cesium_material::Material`]（一个 Fabric 材质：已组装的 GLSL 源 +
//! uniform 值）桥接为原生的 Bevy/WGSL 程序化材质，使其无需运行时 GLSL→WGSL
//! 转译器即可渲染。
//!
//! 镜像上游内置材质渲染路径：领域层执行既定的确切
//! 文本组装，而本适配器提供对同一批内置程序化图案的 GPU 侧求值（见
//! `shaders/fabric_material.wgsl`，它逐图案复刻上游内置材质着色器）。
//!
//! 覆盖全部 21 种 CesiumJS 内置程序化材质类型：
//! Color(0)..Fade(6) 与 PolylineArrow(7)..WaterMask(20)。

use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Shader, ShaderRef, ShaderType};
use cesium_material::{Material as DomainMaterial, UniformValue};
use cesium_shadow::{OceanConfig, OceanSurface};
use std::collections::BTreeMap;

/// 指向嵌入的 Fabric 材质 WGSL shader 的强 handle。
///
/// 该 shader 通过 [`load_internal_asset!`] 编译进 crate，因此适配器无需外部
/// `assets/` 目录即可工作（应用 crate 无需复制 `.wgsl` 文件）。
pub const FABRIC_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    Handle::weak_from_u128(0x4641_4252_4943_4D41_5445_5249_414C);

/// 程序化图案选择器。取值与 `shaders/fabric_material.wgsl` 中的 `kind` switch 相匹配。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum FabricKind {
    /// 纯色（`Color` 材质）。
    #[default]
    Color = 0,
    /// 平铺图像（`Image` 材质）。
    Image = 1,
    /// 棋盘格（`Checkerboard` 材质）。
    Checkerboard = 2,
    /// 条纹（`Stripe` 材质）。
    Stripe = 3,
    /// 网格线（`Grid` 材质）。
    Grid = 4,
    /// 圆点（`Dot` 材质）。
    Dot = 5,
    /// 距离淡入淡出（`Fade` 材质）。
    Fade = 6,
    /// 折线上的箭头（`PolylineArrow` 材质）。
    PolylineArrow = 7,
    /// 虚线折线（`PolylineDash` 材质）。
    PolylineDash = 8,
    /// 发光折线（`PolylineGlow` 材质）。
    PolylineGlow = 9,
    /// 描边折线（`PolylineOutline` 材质）。
    PolylineOutline = 10,
    /// 按高程的等高线（`ElevationContour` 材质）。
    ElevationContour = 11,
    /// 按高程的颜色渐变（`ElevationRamp` 材质）。
    ElevationRamp = 12,
    /// 按坡向的颜色渐变（`AspectRamp` 材质）。
    AspectRamp = 13,
    /// 按坡度的颜色渐变（`SlopeRamp` 材质）。
    SlopeRamp = 14,
    /// 法线贴图（`NormalMap` 材质）。
    NormalMap = 15,
    /// 凹凸贴图（`BumpMap` 材质）。
    BumpMap = 16,
    /// 动画水面（`Water` 材质）。
    Water = 17,
    /// 边缘光效果（`RimLighting` 材质）。
    RimLighting = 18,
    /// 离散高程带（`ElevationBand` 材质）。
    ElevationBand = 19,
    /// 水陆掩膜着色（`WaterMask` 材质）。
    WaterMask = 20,
}

impl FabricKind {
    /// 将 CesiumJS 内置材质类型名映射为一个 [`FabricKind`]。
    /// 未知 / 自定义类型回退到 [`FabricKind::Color`]。
    pub fn from_type_name(type_name: &str) -> Self {
        match type_name {
            "Color" => FabricKind::Color,
            "Image" => FabricKind::Image,
            "Checkerboard" => FabricKind::Checkerboard,
            "Stripe" => FabricKind::Stripe,
            "Grid" => FabricKind::Grid,
            "Dot" => FabricKind::Dot,
            "Fade" => FabricKind::Fade,
            "PolylineArrow" => FabricKind::PolylineArrow,
            "PolylineDash" => FabricKind::PolylineDash,
            "PolylineGlow" => FabricKind::PolylineGlow,
            "PolylineOutline" => FabricKind::PolylineOutline,
            "ElevationContour" => FabricKind::ElevationContour,
            "ElevationRamp" => FabricKind::ElevationRamp,
            "AspectRamp" => FabricKind::AspectRamp,
            "SlopeRamp" => FabricKind::SlopeRamp,
            "NormalMap" => FabricKind::NormalMap,
            "BumpMap" => FabricKind::BumpMap,
            "Water" => FabricKind::Water,
            "RimLighting" => FabricKind::RimLighting,
            "ElevationBand" => FabricKind::ElevationBand,
            "WaterMask" => FabricKind::WaterMask,
            _ => FabricKind::Color,
        }
    }
}

// `ShaderType` derive（encase 0.10）会为每个字段生成一个
// `const _: fn() = || { fn check() { .. } }` 编译期 trait-bound 断言。内部的
// `fn check` 有意从不被*调用*（它只是强制检查字段类型的 bound），因此 Rust 1.95+
// 的 `dead_code` lint 会报告它 —— 这是第三方生成代码中的误报。将该 derive 隔离到
// 一个带窄范围 `#![allow(dead_code)]` 的子模块中，即可在不禁用本文件其余部分该
// lint 的前提下消除它。
mod fabric_params {
    #![allow(dead_code)]
    use super::*;

    /// 面向 [`FabricMaterial`](super::FabricMaterial) 的 GPU uniform 块。
    ///
    /// 字段从领域材质的 uniform map 打包而来。其布局必须与
    /// `shaders/fabric_material.wgsl` 中的 `FabricParams` 结构相匹配。
    #[derive(ShaderType, Debug, Clone)]
    pub struct FabricParams {
        /// [`FabricKind`] 判别值。
        pub kind: u32,
        /// Stripe 的 `horizontal` 标志 (0/1)。
        pub horizontal: u32,
        /// Fade 的 `repeat` 标志 (0/1)。
        pub repeat_flag: u32,
        /// Grid 的 `czm_pixelRatio`（整数，通常为 1）。
        pub pixel_ratio: u32,
        /// 主色（light/even/color/fadeIn/waterColor/baseColor）。
        pub color_a: Vec4,
        /// 次色（dark/odd/fadeOut/outlineColor/rimColor/landColor/gapColor）。
        pub color_b: Vec4,
        /// 图像着色颜色。
        pub color_c: Vec4,
        /// x=repeat.x, y=repeat.y, z=stripe offset, w=fade maximumDistance.
        pub repeat_offset: Vec4,
        /// x=lineCount.x, y=lineCount.y, z=lineThickness.x, w=lineThickness.y.
        pub line_params: Vec4,
        /// x=lineOffset.x, y=lineOffset.y, z=cellAlpha, w=(spare).
        pub line_off_cell: Vec4,
        /// x=fadeDirection.x, y=fadeDirection.y, z=time.x, w=time.y.
        pub fade_dir_time: Vec4,
        /// x=glowPower, y=taperPower, z=outlineWidth/rimWidth, w=dashLength.
        pub extra_a: Vec4,
        /// x=spacing(contour), y=contourWidth, z=strength(normal/bump), w=dashPattern.
        pub extra_b: Vec4,
        /// x=minHeight(ramp/band), y=maxHeight(ramp/band), z=frameNumber(water), w=animationSpeed.
        pub extra_c: Vec4,
        /// M5-D Water (Water 着色器)：x=frequency, y=amplitude, z=specularIntensity,
        /// w=fadeFactor。必须保持为最后一个字段，以使每个既有 uniform 偏移
        /// （kind..extra_c）不变 —— 结构体中间的插入会移动 encase 布局并损坏
        /// 全部 21 种情形（WGSL/Rust 必须匹配）。
        pub water_a: Vec4,
    }

    impl Default for FabricParams {
        /// 默认：kind=0(Color)，各 uniform 字段取 CesiumJS 内置默认（water 参数镜像领域 cache）。
        fn default() -> Self {
            Self {
                kind: 0,
                horizontal: 0,
                repeat_flag: 0,
                pixel_ratio: 1,
                color_a: Vec4::ONE,
                color_b: Vec4::ZERO,
                color_c: Vec4::ONE,
                repeat_offset: Vec4::new(1.0, 1.0, 0.0, 0.5),
                line_params: Vec4::new(8.0, 8.0, 1.0, 1.0),
                line_off_cell: Vec4::new(0.0, 0.0, 0.1, 0.0),
                fade_dir_time: Vec4::new(1.0, 1.0, 0.5, 0.5),
                extra_a: Vec4::new(1.0, 0.0, 0.3, 16.0),
                extra_b: Vec4::new(1000.0, 2.0, 0.5, 255.0),
                extra_c: Vec4::new(0.0, 1000.0, 0.0, 0.5),
                // Water 着色器 的默认值镜像领域 material/cache.rs：
                // frequency=10, amplitude=1, specularIntensity=0.5, fadeFactor=1.
                water_a: Vec4::new(10.0, 1.0, 0.5, 1.0),
            }
        }
    }
}
pub use fabric_params::FabricParams;

/// 一个渲染 CesiumJS Fabric 程序化图案的 Bevy 材质。
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct FabricMaterial {
    /// 打包的 uniform 块。
    #[uniform(0)]
    pub params: FabricParams,
    /// 由 `Sampler2D` uniform 使用的纹理（例如 `Image` 材质）。
    #[texture(1)]
    #[sampler(2)]
    pub image: Handle<Image>,
    /// M5-D Water `normalMap`（Water 着色器）。LINEAR 切线空间数据 → 所绑定的
    /// [`Image`] 必须使用 `TextureFormat::Rgba8Unorm`（绝不用
    /// `Rgba8UnormSrgb`，那会对法线二次编码）。非 water 类型从不采样此绑定；
    /// 它回退到 `image`。
    #[texture(3)]
    #[sampler(4)]
    pub normal_map: Handle<Image>,
    /// M5-D Water `specularMap`（Water 着色器）。LINEAR 掩膜数据 → 与 `normal_map`
    /// 相同的 `Rgba8Unorm` 规则。由 case 17u 以 `.r` 采样。
    #[texture(5)]
    #[sampler(6)]
    pub specular_map: Handle<Image>,
    /// 材质是否半透明（驱动 [`AlphaMode`]）。
    /// 镜像领域层的 `Material.isTranslucent()`。
    pub translucent: bool,
}

impl Material for FabricMaterial {
    /// 指向已嵌入二进制的 `fabric_material.wgsl` 片元着色器 handle。
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(FABRIC_MATERIAL_SHADER_HANDLE)
    }

    /// 依 `translucent` 选 Blend（半透）或 Opaque（不透明）。
    fn alpha_mode(&self) -> AlphaMode {
        if self.translucent {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        }
    }
}

// ---------------------------------------------------------------------------
// Uniform 打包辅助函数
// ---------------------------------------------------------------------------

/// 将 [`UniformValue`] 尝试读取为 `Vec4`（`Vec3` 补 w=1）。
fn vec4_of(v: &UniformValue) -> Option<[f32; 4]> {
    match v {
        UniformValue::Vec4(a) => Some([a[0] as f32, a[1] as f32, a[2] as f32, a[3] as f32]),
        UniformValue::Vec3(a) => Some([a[0] as f32, a[1] as f32, a[2] as f32, 1.0]),
        _ => None,
    }
}

/// 将 [`UniformValue`] 尝试读取为 `Vec2`。
fn vec2_of(v: &UniformValue) -> Option<[f32; 2]> {
    match v {
        UniformValue::Vec2(a) => Some([a[0] as f32, a[1] as f32]),
        _ => None,
    }
}

/// 将 [`UniformValue`] 尝试读取为 `f32`。
fn float_of(v: &UniformValue) -> Option<f32> {
    match v {
        UniformValue::Float(f) => Some(*f as f32),
        _ => None,
    }
}

/// 将 [`UniformValue`] 尝试读取为 `bool`。
fn bool_of(v: &UniformValue) -> Option<bool> {
    match v {
        UniformValue::Bool(b) => Some(*b),
        _ => None,
    }
}

/// 按名取一个 `Vec4` uniform，缺失时回退到默认。
fn get_vec4(u: &BTreeMap<String, UniformValue>, name: &str, default: [f32; 4]) -> Vec4 {
    Vec4::from_slice(&u.get(name).and_then(vec4_of).unwrap_or(default))
}

/// 按名取一个 `[f32; 2]` uniform，缺失时回退到默认。
fn get_vec2(u: &BTreeMap<String, UniformValue>, name: &str, default: [f32; 2]) -> [f32; 2] {
    u.get(name).and_then(vec2_of).unwrap_or(default)
}

/// 按名取一个 `f32` uniform，缺失时回退到默认。
fn get_float(u: &BTreeMap<String, UniformValue>, name: &str, default: f32) -> f32 {
    u.get(name).and_then(float_of).unwrap_or(default)
}

/// 按名取一个 `bool` uniform，缺失时回退到默认。
fn get_bool(u: &BTreeMap<String, UniformValue>, name: &str, default: bool) -> bool {
    u.get(name).and_then(bool_of).unwrap_or(default)
}

/// 从领域 [`DomainMaterial`] 构建一个可渲染的 [`FabricMaterial`]。
///
/// `image` handle 为任意 `Sampler2D` uniform（CesiumJS 的 `czm_defaultImage`）
/// 提供供给，并作为回退支撑 Water 的 `normalMap` / `specularMap` 绑定。半透明度
/// 取自领域材质的 `is_translucent()`，以使 alpha mode 匹配 CesiumJS 行为。
///
/// 对于带真实程序化生成贴图的 Water，请使用
/// [`fabric_material_from_domain_with_maps`] 或 [`water_material_from_preset`]。
pub fn fabric_material_from_domain(
    domain_material: &DomainMaterial,
    image: Handle<Image>,
) -> FabricMaterial {
    fabric_material_from_domain_with_maps(domain_material, image.clone(), image.clone(), image)
}

/// 与 [`fabric_material_from_domain`] 类似，但绑定显式的 Water `normalMap` /
/// `specularMap` handle（M5-D）。两者都必须为线性（`Rgba8Unorm`）图像 ——
/// 见 sRGB 红线。非 water 类型忽略这些绑定。
pub fn fabric_material_from_domain_with_maps(
    domain_material: &DomainMaterial,
    image: Handle<Image>,
    normal_map: Handle<Image>,
    specular_map: Handle<Image>,
) -> FabricMaterial {
    let u = domain_material.uniforms();
    let kind = FabricKind::from_type_name(domain_material.type_name());

    let mut params = FabricParams {
        kind: kind as u32,
        ..Default::default()
    };

    match kind {
        FabricKind::Color => {
            params.color_a = get_vec4(u, "color", [1.0, 0.0, 0.0, 0.5]);
        }
        FabricKind::Image => {
            let repeat = get_vec2(u, "repeat", [1.0, 1.0]);
            params.repeat_offset = Vec4::new(repeat[0], repeat[1], 0.0, 0.5);
            params.color_c = get_vec4(u, "color", [1.0, 1.0, 1.0, 1.0]);
        }
        FabricKind::Checkerboard | FabricKind::Dot => {
            let repeat = get_vec2(u, "repeat", [5.0, 5.0]);
            params.repeat_offset = Vec4::new(repeat[0], repeat[1], 0.0, 0.5);
            params.color_a = get_vec4(u, "lightColor", [1.0, 1.0, 1.0, 0.5]);
            params.color_b = get_vec4(u, "darkColor", [0.0, 0.0, 0.0, 0.5]);
        }
        FabricKind::Stripe => {
            let repeat = get_float(u, "repeat", 5.0);
            let offset = get_float(u, "offset", 0.0);
            params.repeat_offset = Vec4::new(repeat, repeat, offset, 0.5);
            params.horizontal = u32::from(get_bool(u, "horizontal", true));
            params.color_a = get_vec4(u, "evenColor", [1.0, 1.0, 1.0, 0.5]);
            params.color_b = get_vec4(u, "oddColor", [0.0, 0.0, 1.0, 0.5]);
        }
        FabricKind::Grid => {
            let line_count = get_vec2(u, "lineCount", [8.0, 8.0]);
            let line_thickness = get_vec2(u, "lineThickness", [1.0, 1.0]);
            let line_offset = get_vec2(u, "lineOffset", [0.0, 0.0]);
            let cell_alpha = get_float(u, "cellAlpha", 0.1);
            params.line_params = Vec4::new(
                line_count[0],
                line_count[1],
                line_thickness[0],
                line_thickness[1],
            );
            params.line_off_cell = Vec4::new(line_offset[0], line_offset[1], cell_alpha, 0.0);
            params.color_a = get_vec4(u, "color", [0.0, 1.0, 0.0, 1.0]);
        }
        FabricKind::Fade => {
            let max_dist = get_float(u, "maximumDistance", 0.5);
            params.repeat_offset = Vec4::new(1.0, 1.0, 0.0, max_dist);
            params.repeat_flag = u32::from(get_bool(u, "repeat", true));
            let fade_dir = get_vec2(u, "fadeDirection", [1.0, 1.0]);
            let time = get_vec2(u, "time", [0.5, 0.5]);
            params.fade_dir_time = Vec4::new(fade_dir[0], fade_dir[1], time[0], time[1]);
            params.color_a = get_vec4(u, "fadeInColor", [1.0, 0.0, 0.0, 1.0]);
            params.color_b = get_vec4(u, "fadeOutColor", [0.0, 0.0, 0.0, 0.0]);
        }
        // --- 新增材质类型 ---
        FabricKind::PolylineArrow => {
            params.color_a = get_vec4(u, "color", [1.0, 1.0, 1.0, 1.0]);
        }
        FabricKind::PolylineDash => {
            params.color_a = get_vec4(u, "color", [1.0, 1.0, 1.0, 1.0]);
            params.color_b = get_vec4(u, "gapColor", [0.0, 0.0, 0.0, 0.0]);
            params.extra_a.w = get_float(u, "dashLength", 16.0);
            params.extra_b.w = get_float(u, "dashPattern", 255.0);
        }
        FabricKind::PolylineGlow => {
            params.color_a = get_vec4(u, "color", [0.0, 1.0, 1.0, 1.0]);
            params.extra_a.x = get_float(u, "glowPower", 0.25);
            params.extra_a.y = get_float(u, "taperPower", 1.0);
        }
        FabricKind::PolylineOutline => {
            params.color_a = get_vec4(u, "color", [1.0, 1.0, 1.0, 1.0]);
            params.color_b = get_vec4(u, "outlineColor", [0.0, 0.0, 0.0, 1.0]);
            params.extra_a.z = get_float(u, "outlineWidth", 0.3);
        }
        FabricKind::ElevationContour => {
            params.color_a = get_vec4(u, "color", [1.0, 1.0, 1.0, 1.0]);
            params.extra_b.x = get_float(u, "spacing", 1000.0);
            params.extra_b.y = get_float(u, "width", 2.0);
        }
        FabricKind::ElevationRamp => {
            params.extra_c.x = get_float(u, "minimumHeight", 0.0);
            params.extra_c.y = get_float(u, "maximumHeight", 1000.0);
        }
        FabricKind::AspectRamp => {
            // 使用图像纹理做渐变
        }
        FabricKind::SlopeRamp => {
            // 使用图像纹理做渐变
        }
        FabricKind::NormalMap => {
            let repeat = get_vec2(u, "repeat", [1.0, 1.0]);
            params.repeat_offset = Vec4::new(repeat[0], repeat[1], 0.0, 0.0);
            params.extra_b.z = get_float(u, "strength", 0.5);
        }
        FabricKind::BumpMap => {
            let repeat = get_vec2(u, "repeat", [1.0, 1.0]);
            params.repeat_offset = Vec4::new(repeat[0], repeat[1], 0.0, 0.0);
            params.extra_b.z = get_float(u, "strength", 0.5);
        }
        FabricKind::Water => {
            // Water 着色器 uniform（默认值镜像领域 material/cache.rs）。
            params.color_a = get_vec4(u, "baseWaterColor", [0.2, 0.3, 0.6, 1.0]);
            params.color_b = get_vec4(u, "blendColor", [0.0, 1.0, 0.699, 1.0]);
            // extra_c.z = czm_frameNumber（按帧计数器，由 material_system 设置）；
            // extra_c.w = animationSpeed。Water 着色器 L18：time = frameNumber * speed。
            params.extra_c.z = 0.0;
            params.extra_c.w = get_float(u, "animationSpeed", 0.01);
            // water_a：frequency / amplitude / specularIntensity / fadeFactor。
            params.water_a.x = get_float(u, "frequency", 10.0);
            params.water_a.y = get_float(u, "amplitude", 1.0);
            params.water_a.z = get_float(u, "specularIntensity", 0.5);
            params.water_a.w = get_float(u, "fadeFactor", 1.0);
        }
        FabricKind::RimLighting => {
            params.color_a = get_vec4(u, "color", [1.0, 1.0, 1.0, 1.0]);
            params.color_b = get_vec4(u, "rimColor", [0.3, 0.3, 1.0, 1.0]);
            params.extra_a.z = get_float(u, "width", 0.3);
        }
        FabricKind::ElevationBand => {
            params.extra_c.x = get_float(u, "minimumHeight", 0.0);
            params.extra_c.y = get_float(u, "maximumHeight", 1000.0);
        }
        FabricKind::WaterMask => {
            params.color_a = get_vec4(u, "waterColor", [0.1, 0.3, 0.7, 1.0]);
            params.color_b = get_vec4(u, "landColor", [0.3, 0.6, 0.2, 1.0]);
            params.extra_c.x = 0.0; // 水位
        }
    }

    FabricMaterial {
        params,
        image,
        normal_map,
        specular_map,
        translucent: domain_material.is_translucent(),
    }
}

// ---------------------------------------------------------------------------
// M5-D：Water 法线 / 镜面贴图生成（领域 cesium_shadow → 适配器）
// ---------------------------------------------------------------------------

/// 面向 Water 材质 showcase / 基线的海况预设。
///
/// 每个预设驱动一个领域 [`OceanSurface`]（来自 `cesium_shadow::water` 的
/// Gerstner 波叠加），外加推荐的 Water 材质 uniform 覆盖，将先前未被消费的
/// `OceanConfig` / `create_default_waves` / `generate_wind_waves` 领域代码
/// 接入渲染适配器（M5-D 改动面 4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaterPreset {
    /// 平静 —— 微风，重新生成的小浪谱。
    Calm,
    /// 中浪 —— 领域默认的 5 波叠加（`create_default_waves`）。
    Medium,
    /// 大浪 —— 强风，重新生成的谱。
    Rough,
}

impl WaterPreset {
    /// 用于实体命名 / 基线文件名的简短 ASCII 标签。
    pub fn label(&self) -> &'static str {
        match self {
            WaterPreset::Calm => "calm",
            WaterPreset::Medium => "medium",
            WaterPreset::Rough => "rough",
        }
    }

    /// 构建领域海洋状态（消费 `OceanConfig::default()` → `create_default_waves`；
    /// Calm/Rough 使用 `generate_wind_waves`）。
    pub fn ocean(&self) -> OceanSurface {
        let mut ocean = OceanSurface::new(OceanConfig::default());
        match self {
            // Medium 保留默认的 create_default_waves() 5 波叠加。
            WaterPreset::Medium => {}
            WaterPreset::Calm => {
                ocean.wind_speed = 5.0;
                ocean.generate_wind_waves();
            }
            WaterPreset::Rough => {
                ocean.wind_speed = 20.0;
                ocean.generate_wind_waves();
            }
        }
        ocean
    }

    /// Water 材质 uniform 覆盖（frequency / amplitude / animationSpeed /
    /// specularIntensity），按海况分别调校。
    pub fn uniform_overrides(&self) -> Vec<(&'static str, UniformValue)> {
        match self {
            WaterPreset::Calm => vec![
                ("frequency", UniformValue::Float(6.0)),
                ("amplitude", UniformValue::Float(0.5)),
                ("animationSpeed", UniformValue::Float(0.004)),
                ("specularIntensity", UniformValue::Float(0.3)),
            ],
            WaterPreset::Medium => vec![
                ("frequency", UniformValue::Float(10.0)),
                ("amplitude", UniformValue::Float(1.0)),
                ("animationSpeed", UniformValue::Float(0.01)),
                ("specularIntensity", UniformValue::Float(0.5)),
            ],
            WaterPreset::Rough => vec![
                ("frequency", UniformValue::Float(16.0)),
                ("amplitude", UniformValue::Float(2.5)),
                ("animationSpeed", UniformValue::Float(0.03)),
                ("specularIntensity", UniformValue::Float(0.8)),
            ],
        }
    }
}

/// 通过在 `tile_size_m` 米的 UV 瓦片上采样领域 [`OceanSurface`]（Gerstner 波），
/// 生成一个切线空间的 Water `normalMap`。
///
/// sRGB 红线：这是 LINEAR 方向数据，因此 [`Image`] 使用 `TextureFormat::Rgba8Unorm`
/// —— 绝不用 `Rgba8UnormSrgb`（那会对法线二次编码）。米制换算红线：被采样的位置
/// 与波浪振幅保持在米空间；返回的法线是无量纲方向（坡度 = m/m），因此此处不施加
/// `METERS_PER_RENDER_UNIT` 除法 —— 该换算在 shader 侧施加于 Water 着色器 的 1e10
/// 淡出除数（见 `shaders/fabric_material.wgsl`）。
pub fn generate_water_normal_map(size: u32, ocean: &OceanSurface, tile_size_m: f64) -> Image {
    let denom = size.saturating_sub(1).max(1) as f64;
    let enc = |c: f64| ((c * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = x as f64 / denom;
            let v = y as f64 / denom;
            let pos = DVec3::new(u * tile_size_m, 0.0, v * tile_size_m);
            // 来自 Gerstner 波叠加的世界空间（Y-up）海洋法线。
            let n = ocean.compute_normal(pos);
            // Water 着色器 切线空间是 Z-up；将 Y-up 世界重映射为 Z-up 切线。
            data.extend_from_slice(&[enc(n.x), enc(n.z), enc(n.y), 255]);
        }
    }
    Image::new(
        bevy::render::render_resource::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8Unorm,
        bevy::render::render_asset::RenderAssetUsages::default(),
    )
}

/// 从海浪波峰高度生成一个 Water `specularMap`（水/非水掩膜，由 Water 着色器
/// 以 `.r` 采样）。LINEAR 掩膜数据 → `Rgba8Unorm`（sRGB 红线）。保持明亮
/// （≈0.6..1.0）以使水面可见：Water 着色器 将 alpha 乘以该值，因此暗掩膜会消失。
pub fn generate_water_specular_map(size: u32, ocean: &OceanSurface, tile_size_m: f64) -> Image {
    let denom = size.saturating_sub(1).max(1) as f64;
    let foam = ocean.config.foam_threshold.max(1e-6);
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = x as f64 / denom;
            let v = y as f64 / denom;
            let pos = DVec3::new(u * tile_size_m, 0.0, v * tile_size_m);
            let h = ocean.compute_height(pos); // 米
            // 将波峰高度归一化到 [0,1]，再映射到一个明亮的掩膜带
            // [0.6, 1.0] 以使水面可见：Water 着色器 将 alpha 乘以该值，
            // 因此暗掩膜会使表面消失。
            let crest = ((h / foam) * 0.5 + 0.5).clamp(0.0, 1.0);
            let mask = 0.6 + 0.4 * crest;
            let b = (mask * 255.0) as u8;
            data.extend_from_slice(&[b, b, b, 255]);
        }
    }
    Image::new(
        bevy::render::render_resource::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8Unorm,
        bevy::render::render_asset::RenderAssetUsages::default(),
    )
}

/// 便捷函数：为 `preset` 构建一个 Water [`FabricMaterial`]，生成其法线/镜面贴图
/// 并插入 `images`。将 `cesium_shadow` 的使用保留在适配器内部，因此应用 crate
/// 无需依赖它。
pub fn water_material_from_preset(
    images: &mut Assets<Image>,
    domain_material: &DomainMaterial,
    fallback: Handle<Image>,
    preset: WaterPreset,
) -> FabricMaterial {
    let ocean = preset.ocean();
    let normal_map = images.add(generate_water_normal_map(128, &ocean, 200.0));
    let specular_map = images.add(generate_water_specular_map(128, &ocean, 200.0));
    fabric_material_from_domain_with_maps(domain_material, fallback, normal_map, specular_map)
}

/// 将 [`FabricMaterial`] 注册到 Bevy 资产/管线系统的插件。
pub struct FabricMaterialPlugin;

impl Plugin for FabricMaterialPlugin {
    /// 注册 [`FabricMaterial`] 及其内嵌 WGSL 着色器。
    ///
    /// # 参数
    /// - `app`：Bevy 应用
    fn build(&self, app: &mut App) {
        // 将 WGSL shader 嵌入二进制，因此宿主应用无需外部资产路径。
        //
        // 无头安好（M5.1）：裸 `load_internal_asset!` 会解引用 `Assets<Shader>`，
        // 它在 `MinimalPlugins` 测试 app（无 `AssetPlugin`）下缺失并会 panic。
        // `try_load_internal_shader` 对该资源进行守卫，缺失时降级为空操作
        // （`None`），同时在 GPU 路径上以*相同的* `AssetId` 插入*相同的*
        // `include_str!` 嵌入源（像素中性）。
        // 参见 docs/deviations.md#dev-005 / docs/deferred.md#6（在 M5.1 解决）。
        crate::shader_registry::try_load_internal_shader(
            app,
            FABRIC_MATERIAL_SHADER_HANDLE,
            include_str!("../shaders/fabric_material.wgsl"),
            std::path::Path::new(file!())
                .parent()
                .unwrap()
                .join("../shaders/fabric_material.wgsl")
                .to_string_lossy(),
        );

        // `MaterialPlugin::build` 调用 `init_asset::<M>()`，它解引用 `AssetServer`
        // 资源并在其缺失时（无头）panic。Bevy 0.15 已守卫了 `MaterialPlugin` 与
        // `RenderAssetPlugin` 的 `RenderApp` 子 app 部分（`get_sub_app_mut(RenderApp)`），
        // 因此资产后端是唯一未受守卫的隐患。当后端不可用时跳过整个插件；
        // `FabricMaterial` 仍可作为纯 CPU 侧类型（组件 / `fabric_material_from_domain`）
        // 供无头测试使用。
        if crate::shader_registry::asset_backend_available(app) {
            app.add_plugins(MaterialPlugin::<FabricMaterial>::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_material::MaterialSystem;

    fn build(type_name: &str) -> DomainMaterial {
        let system = MaterialSystem::with_builtin_materials();
        system
            .from_type(type_name, BTreeMap::new())
            .unwrap_or_else(|e| panic!("failed to build {}: {}", type_name, e))
    }

    #[test]
    fn test_kind_mapping() {
        assert_eq!(FabricKind::from_type_name("Checkerboard"), FabricKind::Checkerboard);
        assert_eq!(FabricKind::from_type_name("Stripe"), FabricKind::Stripe);
        assert_eq!(FabricKind::from_type_name("Grid"), FabricKind::Grid);
        assert_eq!(FabricKind::from_type_name("Color"), FabricKind::Color);
        assert_eq!(FabricKind::from_type_name("SomeCustom"), FabricKind::Color);
    }

    #[test]
    fn test_kind_mapping_extended() {
        assert_eq!(FabricKind::from_type_name("PolylineArrow"), FabricKind::PolylineArrow);
        assert_eq!(FabricKind::from_type_name("PolylineGlow"), FabricKind::PolylineGlow);
        assert_eq!(FabricKind::from_type_name("ElevationContour"), FabricKind::ElevationContour);
        assert_eq!(FabricKind::from_type_name("Water"), FabricKind::Water);
        assert_eq!(FabricKind::from_type_name("RimLighting"), FabricKind::RimLighting);
        assert_eq!(FabricKind::from_type_name("BumpMap"), FabricKind::BumpMap);
        assert_eq!(FabricKind::from_type_name("WaterMask"), FabricKind::WaterMask);
        assert_eq!(FabricKind::from_type_name("UnknownType"), FabricKind::Color);
    }

    #[test]
    fn test_from_domain_checkerboard() {
        let m = build("Checkerboard");
        let handle = Handle::<Image>::default();
        let fm = fabric_material_from_domain(&m, handle);
        assert_eq!(fm.params.kind, FabricKind::Checkerboard as u32);
        // 默认 repeat 为 (5, 5)。
        assert!((fm.params.repeat_offset.x - 5.0).abs() < 1e-6);
        assert!((fm.params.repeat_offset.y - 5.0).abs() < 1e-6);
        // 默认 lightColor 为白色（alpha 0.5）→ 半透明。
        assert!(fm.translucent);
    }

    #[test]
    fn test_from_domain_grid() {
        let m = build("Grid");
        let fm = fabric_material_from_domain(&m, Handle::<Image>::default());
        assert_eq!(fm.params.kind, FabricKind::Grid as u32);
        assert!((fm.params.line_params.x - 8.0).abs() < 1e-6); // lineCount.x
        assert!((fm.params.line_off_cell.z - 0.1).abs() < 1e-6); // cellAlpha
        // 默认 Grid 为半透明（cellAlpha 0.1）。
        assert!(fm.translucent);
    }

    #[test]
    fn test_from_domain_stripe() {
        let m = build("Stripe");
        let fm = fabric_material_from_domain(&m, Handle::<Image>::default());
        assert_eq!(fm.params.kind, FabricKind::Stripe as u32);
        assert!((fm.params.repeat_offset.x - 5.0).abs() < 1e-6); // repeat
        assert_eq!(fm.params.horizontal, 1); // 默认 horizontal = true
    }

    #[test]
    fn test_from_domain_color_opaque_override() {
        let system = MaterialSystem::with_builtin_materials();
        let mut overrides = BTreeMap::new();
        overrides.insert(
            "color".to_string(),
            UniformValue::Vec4([0.0, 1.0, 0.0, 1.0]),
        );
        let m = system.from_type("Color", overrides).unwrap();
        let fm = fabric_material_from_domain(&m, Handle::<Image>::default());
        assert_eq!(fm.params.kind, FabricKind::Color as u32);
        assert!(!fm.translucent); // alpha 1.0 → 不透明
        assert!((fm.params.color_a.y - 1.0).abs() < 1e-6); // 绿色
    }

    #[test]
    fn test_alpha_mode_follows_translucency() {
        let m = build("Grid");
        let fm = fabric_material_from_domain(&m, Handle::<Image>::default());
        assert!(matches!(fm.alpha_mode(), AlphaMode::Blend));

        let system = MaterialSystem::with_builtin_materials();
        let mut overrides = BTreeMap::new();
        overrides.insert("cellAlpha".to_string(), UniformValue::Float(1.0));
        overrides.insert(
            "color".to_string(),
            UniformValue::Vec4([0.0, 1.0, 0.0, 1.0]),
        );
        let opaque = system.from_type("Grid", overrides).unwrap();
        let fm2 = fabric_material_from_domain(&opaque, Handle::<Image>::default());
        assert!(matches!(fm2.alpha_mode(), AlphaMode::Opaque));
    }

    #[test]
    fn test_from_domain_polyline_arrow() {
        let system = MaterialSystem::with_builtin_materials();
        let m = system.from_type("PolylineArrow", BTreeMap::new()).unwrap();
        let fm = fabric_material_from_domain(&m, Handle::<Image>::default());
        assert_eq!(fm.params.kind, FabricKind::PolylineArrow as u32);
    }

    #[test]
    fn test_from_domain_elevation_contour() {
        let system = MaterialSystem::with_builtin_materials();
        let mut overrides = BTreeMap::new();
        overrides.insert("spacing".to_string(), UniformValue::Float(500.0));
        overrides.insert("width".to_string(), UniformValue::Float(3.0));
        let m = system.from_type("ElevationContour", overrides).unwrap();
        let fm = fabric_material_from_domain(&m, Handle::<Image>::default());
        assert_eq!(fm.params.kind, FabricKind::ElevationContour as u32);
        assert!((fm.params.extra_b.x - 500.0).abs() < 1e-6);
        assert!((fm.params.extra_b.y - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_from_domain_rim_lighting() {
        let system = MaterialSystem::with_builtin_materials();
        let mut overrides = BTreeMap::new();
        overrides.insert("width".to_string(), UniformValue::Float(0.5));
        let m = system.from_type("RimLighting", overrides).unwrap();
        let fm = fabric_material_from_domain(&m, Handle::<Image>::default());
        assert_eq!(fm.params.kind, FabricKind::RimLighting as u32);
        assert!((fm.params.extra_a.z - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_from_domain_water() {
        let system = MaterialSystem::with_builtin_materials();
        let mut overrides = BTreeMap::new();
        overrides.insert("animationSpeed".to_string(), UniformValue::Float(0.3));
        let m = system.from_type("Water", overrides).unwrap();
        let fm = fabric_material_from_domain(&m, Handle::<Image>::default());
        assert_eq!(fm.params.kind, FabricKind::Water as u32);
        assert!((fm.params.extra_c.w - 0.3).abs() < 1e-6);
    }

    #[test]
    fn test_from_domain_water_mask() {
        let system = MaterialSystem::with_builtin_materials();
        let m = system.from_type("WaterMask", BTreeMap::new()).unwrap();
        let fm = fabric_material_from_domain(&m, Handle::<Image>::default());
        assert_eq!(fm.params.kind, FabricKind::WaterMask as u32);
    }

    #[test]
    fn test_from_domain_water_packs_water_a() {
        // M5-D：Water 着色器 的 frequency/amplitude/specularIntensity/fadeFactor
        // 落在 water_a（cache.rs 默认值 10 / 1 / 0.5 / 1）。
        let m = build("Water");
        let fm = fabric_material_from_domain(&m, Handle::<Image>::default());
        assert!((fm.params.water_a.x - 10.0).abs() < 1e-6);
        assert!((fm.params.water_a.y - 1.0).abs() < 1e-6);
        assert!((fm.params.water_a.z - 0.5).abs() < 1e-6);
        assert!((fm.params.water_a.w - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_water_normal_map_is_linear_rgba8unorm() {
        // sRGB 红线：切线空间法线是 LINEAR → Rgba8Unorm。
        let ocean = WaterPreset::Medium.ocean();
        let img = generate_water_normal_map(16, &ocean, 200.0);
        assert_eq!(
            img.texture_descriptor.format,
            bevy::render::render_resource::TextureFormat::Rgba8Unorm
        );
        assert_eq!(img.texture_descriptor.size.width, 16);
        assert_eq!(img.texture_descriptor.size.height, 16);
    }

    #[test]
    fn test_water_specular_map_is_linear_and_bright() {
        let ocean = WaterPreset::Rough.ocean();
        let img = generate_water_specular_map(8, &ocean, 200.0);
        assert_eq!(
            img.texture_descriptor.format,
            bevy::render::render_resource::TextureFormat::Rgba8Unorm
        );
        // 掩膜带 [0.6, 1.0] 使水面可见 → r >= ~153。
        assert!(img.data.iter().step_by(4).all(|&r| r >= 150));
    }

    #[test]
    fn test_water_preset_labels_and_overrides() {
        assert_eq!(WaterPreset::Calm.label(), "calm");
        assert_eq!(WaterPreset::Medium.label(), "medium");
        assert_eq!(WaterPreset::Rough.label(), "rough");
        assert_eq!(WaterPreset::Rough.uniform_overrides().len(), 4);
        // Medium 消费 create_default_waves()（5 个波）；Rough 重新生成 8 个。
        assert_eq!(WaterPreset::Medium.ocean().config.waves.len(), 5);
        assert_eq!(WaterPreset::Rough.ocean().config.waves.len(), 8);
    }

    // ------------------------------------------------------------------
    // Ryan C1 防线：naga 解析 + 校验 `fabric_material.wgsl`。
    //
    // 上述 14 个测试只演练 Rust 侧的 uniform 打包与纹理格式；它们都没有将 WGSL
    // 过一遍 naga，这正是某个被用作调用的 WGSL 保留字（`mod(...)`）能瞒过 CI
    // 转绿的原因。naga 正是 Bevy 编译所用的前端（bevy_render -> naga 23.1），
    // 因此在此驱动它能将那一类缺陷变成硬性失败。
    // ------------------------------------------------------------------

    /// 为两个 `#import` 提供的桩（naga 没有预处理器）。精确声明 shader 读取的
    /// 那些绑定：`VertexOutput.world_position` / `.world_normal` / `.uv`
    /// （forward_io）与 `view.world_position`（mesh_view_bindings）。其余全部是
    /// 真实的 shader 文本，因此这校验了实际的 21 情形程序化代码。
    const WGSL_IMPORT_STUBS: &str = "\
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct View {
    world_position: vec3<f32>,
    exposure: f32,
}

@group(0) @binding(0) var<uniform> view: View;
";

    /// 将 `fabric_material.wgsl` 重建为 naga 可解析的独立 WGSL：剥离 `#import`
    /// 行（由 [`WGSL_IMPORT_STUBS`] 替代）并解析 `#ifdef / #else / #endif` 块。
    /// [`FabricMaterial`] 不设自定义 `shader_def`（它不覆写 `Material::specialize`），
    /// 因此每个受守卫的符号（`VERTEX_UVS_A`）都是 UNDEFINED → 取 `#else` 分支。
    fn stubbed_wgsl() -> String {
        let source = include_str!("../shaders/fabric_material.wgsl").replace("\r\n", "\n");
        let mut out = String::from(WGSL_IMPORT_STUBS);
        // 非嵌套的 `#ifdef`，带可选 `#else`；所有符号均未定义。
        let mut skipping = false;
        for line in source.lines() {
            let t = line.trim_start();
            if t.starts_with("#import") || t.starts_with("#endif") {
                skipping = false;
                continue;
            }
            if t.starts_with("#ifdef") {
                skipping = true; // 未定义符号 → 丢弃 #if 分支
                continue;
            }
            if t.starts_with("#else") {
                skipping = false; // 保留回退分支
                continue;
            }
            if skipping {
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// Ryan C1 回归门禁：整个 `fabric_material.wgsl`（全部 21 个 Fabric 情形）
    /// 必须能在 naga 下解析并进行类型检查。一个被用作调用的 WGSL 保留字
    /// （`mod(...)`，现为 `glsl_mod(...)`）或未声明的标识符（`in.world_tangent`、
    /// `mesh_view_bindings::view`）能正常解析但在 lowering 时失败，静默地丢弃
    /// 每个管线（全部 21 个情形不可渲染）。
    #[test]
    fn fabric_material_wgsl_parses_and_validates_under_naga() {
        let source = stubbed_wgsl();
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|error| {
            panic!(
                "fabric_material.wgsl does not parse:\n{}",
                error.emit_to_string(&source)
            )
        });
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("fabric_material.wgsl does not validate");

        // 单个 forward `Material` 片元入口，即 Bevy 期望的签名。
        let entry_points = module
            .entry_points
            .iter()
            .map(|entry| (entry.name.as_str(), entry.stage))
            .collect::<Vec<_>>();
        assert_eq!(
            entry_points,
            vec![("fragment", naga::ShaderStage::Fragment)],
            "fabric_material.wgsl must expose exactly one fragment entry point"
        );
    }

    /// 针对本轮所修复的确切缺陷的快速、精确的源码契约断言，从而在 naga lowering
    /// 运行之前就能诊断出回归。
    #[test]
    fn fabric_material_wgsl_avoids_the_known_reserved_word_and_dead_branch_traps() {
        let source = include_str!("../shaders/fabric_material.wgsl").replace("\r\n", "\n");
        // 先剥离 `//` 行注释：恰恰是这些修复的说明性注释在文字中提到了 `mod()`、
        // `VERTEX_TANGENTS` 与 `world_tangent`，因此扫描原始源会误报。该契约针对的是代码。
        let code = source
            .lines()
            .map(|line| match line.find("//") {
                Some(idx) => &line[..idx],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n");
        // Ryan C1：每个 `mod(` 都必须是 `glsl_mod(` 辅助函数 —— 剥离这些后不得
        // 残留裸 `mod(`（一个 WGSL 保留字）。
        assert!(
            !code.replace("glsl_mod(", "").contains("mod("),
            "fabric_material.wgsl calls the WGSL reserved word `mod`; use glsl_mod"
        );
        // Daniel L3：已失效的 VERTEX_TANGENTS 分支曾读取一个未声明的
        // `in.world_tangent`；它必须保持移除（无不需编译的路径）。
        assert!(
            !code.contains("VERTEX_TANGENTS"),
            "dead VERTEX_TANGENTS branch must stay removed"
        );
        assert!(
            !code.contains("world_tangent"),
            "world_tangent is not declared in this shader's VertexOutput"
        );
        // `view` 绑定必须以非限定形式引用：naga_oil 将 `#import bevy_pbr::mesh_view_bindings`
        // 解析进全局作用域，而 `::` 不是合法的 WGSL（裸 naga lowering 会拒绝它）。
        assert!(
            !code.contains("mesh_view_bindings::"),
            "reference `view`, not the non-WGSL `mesh_view_bindings::view`"
        );
        assert!(code.contains("view.world_position"));
        // WGSL 禁止 swizzle 赋值（`v.rgb = ...`、`v.xy /= ...`）；naga 会拒绝它，
        // 整个 shader 无法 lowering。本轮修复的四处都在此断言为不存在，以实现快速、
        // 精确的预诊断。
        for pat in [".rgb =", ".rgba =", ".xy =", ".xyz =", ".xy /=", ".xy *="] {
            assert!(
                !code.contains(pat),
                "fabric_material.wgsl uses illegal WGSL swizzle assignment `{pat}`"
            );
        }
    }
}
