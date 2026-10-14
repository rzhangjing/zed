//! 材质缓存与 25 个内置 Fabric 材质。
//!
//! 本模块维护类型名到 [`CachedMaterial`] 的映射，并在底部
//! 逐项注册全部内置材质。领域层不使用可变全局变量，而是使用
//! 显式的 [`MaterialSystem`] 值，因此缓存可测试且可按应用作用域划分。

use crate::error::MaterialError;
use crate::fabric::{FabricTemplate, MaterialComponents};
use crate::glsl;
use crate::material::{build_material, Material, MaterialOptions};
use crate::translucent::TranslucentSpec;
use crate::uniform::{UniformValue, DEFAULT_IMAGE_ID};
use std::collections::{BTreeMap, HashMap};

/// 缓存的材质定义：原始 Fabric 模板加上其半透明规则。
///
/// 对应缓存中的单个条目，即注册时传入的 `{ fabric, translucent }` 对象。
#[derive(Debug, Clone)]
pub struct CachedMaterial {
    /// 原始 Fabric 模板（uniforms + source/components）。
    pub fabric: FabricTemplate,
    /// 半透明规则（缓存条目的 `translucent` 成员）。
    pub translucent: Option<TranslucentSpec>,
}

impl CachedMaterial {
    /// 由 components 形式的模板构造一个缓存材质（用于声明式分量定义）。
    fn from_components(
        type_name: &str,
        uniforms: BTreeMap<String, UniformValue>,
        components: MaterialComponents,
        translucent: Option<TranslucentSpec>,
    ) -> Self {
        CachedMaterial {
            fabric: FabricTemplate {
                type_name: Some(type_name.to_string()),
                uniforms,
                materials: BTreeMap::new(),
                components: Some(components),
                source: None,
            },
            translucent,
        }
    }

    /// 由自定义 source 形式的模板构造一个缓存材质（用于内联 GLSL 定义）。
    fn from_source(
        type_name: &str,
        uniforms: BTreeMap<String, UniformValue>,
        source: &str,
        translucent: Option<TranslucentSpec>,
    ) -> Self {
        CachedMaterial {
            fabric: FabricTemplate {
                type_name: Some(type_name.to_string()),
                uniforms,
                materials: BTreeMap::new(),
                components: None,
                source: Some(source.to_string()),
            },
            translucent,
        }
    }
}

/// 内置材质所用 uniform 映射的简写构造器。
/// 将颜色参数包装为 vec4。
fn color(r: f64, g: f64, b: f64, a: f64) -> UniformValue {
    UniformValue::Vec4([r, g, b, a])
}
/// 将一个二维向量的两个分量包装为 vec2 uniform。
fn vec2(x: f64, y: f64) -> UniformValue {
    UniformValue::Vec2([x, y])
}
/// 将一个标量包装为 float uniform。
fn float(v: f64) -> UniformValue {
    UniformValue::Float(v)
}
/// 将一个布尔值包装为 bool uniform。
fn bool_u(v: bool) -> UniformValue {
    UniformValue::Bool(v)
}
/// 返回默认图像哨兵作为 sampler2D uniform。
fn default_image() -> UniformValue {
    UniformValue::Sampler2D(DEFAULT_IMAGE_ID.to_string())
}
/// 将一个通道 swizzle 字符串包装为 channels uniform。
fn channels(s: &str) -> UniformValue {
    UniformValue::Channels(s.to_string())
}

/// 用于保持内置定义可读性的 uniform 映射构建器。
struct U(BTreeMap<String, UniformValue>);
impl U {
    /// 创建一个空的构建器。
    fn new() -> Self {
        U(BTreeMap::new())
    }
    /// 链式插入一个键值对，返回自身以便继续构建。
    fn set(mut self, key: &str, value: UniformValue) -> Self {
        self.0.insert(key.to_string(), value);
        self
    }
    /// 消耗构建器并产出最终的 uniform 映射。
    fn build(self) -> BTreeMap<String, UniformValue> {
        self.0
    }
}

/// 材质缓存 + 工厂。
///
/// 持有类型名到缓存定义的映射，并提供按模板/按类型名构造材质的
/// 入口点。
#[derive(Debug, Clone, Default)]
pub struct MaterialSystem {
    /// 类型名 -> 已缓存的材质定义。
    cache: HashMap<String, CachedMaterial>,
}

impl MaterialSystem {
    /// 不含内置材质的空缓存。
    pub fn new() -> Self {
        MaterialSystem {
            cache: HashMap::new(),
        }
    }

    /// 预置 25 个内置材质的缓存。
    pub fn with_builtin_materials() -> Self {
        let mut system = MaterialSystem::new();
        // 逐项插入内置定义（以类型名为键）
        for (name, material) in builtin_materials() {
            system.cache.insert(name, material);
        }
        system
    }

    /// 注册一种材质类型（将定义写入缓存）。
    pub fn add_material(&mut self, type_name: &str, material: CachedMaterial) {
        self.cache.insert(type_name.to_string(), material);
    }

    /// 按类型名查找已缓存的材质定义。
    pub fn get_material(&self, type_name: &str) -> Option<&CachedMaterial> {
        self.cache.get(type_name)
    }

    /// 缓存材质类型的数量。
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// 缓存是否为空。
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// 从 Fabric 模板构建材质（供构建流水线和测试使用的低层入口）。
    pub(crate) fn build(
        &self,
        fabric: FabricTemplate,
        strict: bool,
        translucent: Option<bool>,
    ) -> Result<Material, MaterialError> {
        // 重命名计数器从 0 起；丢弃返回的半透明规则计数，仅取材质
        let mut count = 0usize;
        let (material, _collected) =
            build_material(fabric, strict, translucent, &self.cache, &mut count)?;
        Ok(material)
    }

    /// 从选项创建材质。
    ///
    /// 当结果类型是新的（不在缓存中）时，之后将其加入缓存，
    /// 以便后续同类型的构造可复用其模板。
    pub fn create_material(&mut self, options: MaterialOptions) -> Result<Material, MaterialError> {
        // 先取模板声明的类型名，用于判断是否已在缓存中
        let type_name = options
            .fabric
            .type_name
            .clone()
            .unwrap_or_else(|| options.fabric.type_name.clone().unwrap_or_default());

        // 类型非空且已缓存时，不应重复回写模板
        let already_cached = !type_name.is_empty() && self.cache.contains_key(&type_name);

        // 走完整的构建流水线得到已装配的材质
        let material = self.build(options.fabric.clone(), options.strict, options.translucent)?;

        // 将新类型加入缓存（它们自身没有半透明规则；新建材质的
        // translucent 属性缺省，即此处的 `None`）。
        if !already_cached {
            self.cache.insert(
                material.type_name().to_string(),
                CachedMaterial {
                    fabric: options.fabric,
                    translucent: None,
                },
            );
        }

        Ok(material)
    }

    /// 从已有的缓存类型创建新材质。
    ///
    /// 以类型名 + uniforms 覆盖项为入口；未知类型会报错。
    pub fn from_type(
        &self,
        type_name: &str,
        overrides: BTreeMap<String, UniformValue>,
    ) -> Result<Material, MaterialError> {
        // 未知类型直接报错
        if !self.cache.contains_key(type_name) {
            return Err(MaterialError::UnknownMaterialType {
                type_name: type_name.to_string(),
            });
        }

        // 以类型名 + 覆盖 uniforms 组装一个最小模板再委托构建
        let mut fabric = FabricTemplate {
            type_name: Some(type_name.to_string()),
            ..Default::default()
        };
        fabric.uniforms = overrides;

        self.build(fabric, false, None)
    }
}

/// 25 个内置材质，按注册时的声明顺序排列。
///
/// 每一项都是一个 [`CachedMaterial`]：包含原始 Fabric 模板（uniforms 与
/// 分量表达式或内联 GLSL 源码）以及对应的半透明规则。前六个
/// （Color/Image/DiffuseMap/AlphaMap/SpecularMap/EmissionMap）以声明式
/// components 形式定义，其余则引入 [`glsl`] 模块中以内联源码形式定义。
fn builtin_materials() -> Vec<(String, CachedMaterial)> {
    // 预先按 25 项分配容量，逐项 append
    let mut out: Vec<(String, CachedMaterial)> = Vec::with_capacity(25);
    // add 将（类型名, 定义）追加到输出列表
    let mut add = |name: &str, m: CachedMaterial| out.push((name.to_string(), m));

    // Color：纯色材质。仅一个 color(vec4) uniform；
    // diffuse 取 color.rgb（经 gamma 校正），alpha 取 color.a。
    // 属于声明式分量定义。默认 alpha=0.5，故 color.alpha<1 时半透明。
    add(
        "Color",
        // 采用声明式 components 定义（非内联 GLSL 源码）
        CachedMaterial::from_components(
            "Color",
            U::new().set("color", color(1.0, 0.0, 0.0, 0.5)).build(),
            MaterialComponents {
                diffuse: Some("color.rgb".to_string()),
                alpha: Some("color.a".to_string()),
                ..Default::default()
            },
            Some(TranslucentSpec::AnyAlphaLt1(vec!["color"])),
        ),
    );

    // Image：图像贴图材质。sampler2D image + vec2 repeat + vec4 color；
    // 对 uv 做 fract(repeat*st) 平铺采样，再与 color 相乘调制。
    // 仅当 color.alpha<1 时半透明（图像自身 alpha 不参与判定）。
    add(
        "Image",
        // 采用声明式 components 定义，diffuse/alpha 由采样与 color 相乘
        CachedMaterial::from_components(
            "Image",
            U::new()
                .set("image", default_image())
                .set("repeat", vec2(1.0, 1.0))
                .set("color", color(1.0, 1.0, 1.0, 1.0))
                .build(),
            MaterialComponents {
                diffuse: Some(
                    "texture(image, fract(repeat * materialInput.st)).rgb * color.rgb".to_string(),
                ),
                alpha: Some(
                    "texture(image, fract(repeat * materialInput.st)).a * color.a".to_string(),
                ),
                ..Default::default()
            },
            Some(TranslucentSpec::AnyAlphaLt1(vec!["color"])),
        ),
    );

    // DiffuseMap：漫反射贴图。以 channels(默认 "rgb") 作为文本 swizzle，
    // 将采样结果的指定通道赋给 diffuse。channels 非真正 uniform，从不半透明。
    add(
        "DiffuseMap",
        // 采用声明式 components 定义，仅给出 diffuse 分量
        CachedMaterial::from_components(
            "DiffuseMap",
            U::new()
                .set("image", default_image())
                .set("channels", channels("rgb"))
                .set("repeat", vec2(1.0, 1.0))
                .build(),
            MaterialComponents {
                diffuse: Some("texture(image, fract(repeat * materialInput.st)).channels".to_string()),
                ..Default::default()
            },
            Some(TranslucentSpec::Never),
        ),
    );

    // AlphaMap：透明度贴图。取图像单通道（channel 默认 "a"）赋给 alpha。
    // 总是需要半透明混合（Always）。
    add(
        "AlphaMap",
        // 采用声明式 components 定义，仅给出 alpha 分量
        CachedMaterial::from_components(
            "AlphaMap",
            U::new()
                .set("image", default_image())
                .set("channel", channels("a"))
                .set("repeat", vec2(1.0, 1.0))
                .build(),
            MaterialComponents {
                alpha: Some("texture(image, fract(repeat * materialInput.st)).channel".to_string()),
                ..Default::default()
            },
            Some(TranslucentSpec::Always),
        ),
    );

    // SpecularMap：高光贴图。取图像单通道（channel 默认 "r"）赋给 specular。
    // 不参与透明度，从不半透明。
    add(
        "SpecularMap",
        // 采用声明式 components 定义，仅给出 specular 分量
        CachedMaterial::from_components(
            "SpecularMap",
            U::new()
                .set("image", default_image())
                .set("channel", channels("r"))
                .set("repeat", vec2(1.0, 1.0))
                .build(),
            MaterialComponents {
                specular: Some(
                    "texture(image, fract(repeat * materialInput.st)).channel".to_string(),
                ),
                ..Default::default()
            },
            Some(TranslucentSpec::Never),
        ),
    );

    // EmissionMap：自发光贴图。以 channels(默认 "rgb") 采样并赋给 emission。
    // 从不半透明。
    add(
        "EmissionMap",
        // 采用声明式 components 定义，仅给出 emission 分量
        CachedMaterial::from_components(
            "EmissionMap",
            U::new()
                .set("image", default_image())
                .set("channels", channels("rgb"))
                .set("repeat", vec2(1.0, 1.0))
                .build(),
            MaterialComponents {
                emission: Some(
                    "texture(image, fract(repeat * materialInput.st)).channels".to_string(),
                ),
                ..Default::default()
            },
            Some(TranslucentSpec::Never),
        ),
    );

    // BumpMap：高度图凹凸材质。以内联 GLSL 源码实现，含 strength 与
    // imageDimensions 配套 uniform。从不半透明。
    add(
        "BumpMap",
        CachedMaterial::from_source(
            "BumpMap",
            U::new()
                .set("image", default_image())
                .set("channel", channels("r"))
                .set("strength", float(0.8))
                .set("repeat", vec2(1.0, 1.0))
                .build(),
            // BumpMap 凹凸：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::BUMP_MAP_MATERIAL,
            // 半透明规则：从不（仅改变法线，不影响 alpha）。
            Some(TranslucentSpec::Never),
        ),
    );

    // NormalMap：法线贴图材质。以内联 GLSL 实现，channels 默认 "rgb"，
    // strength 控制凹凸强度。从不半透明。
    add(
        "NormalMap",
        CachedMaterial::from_source(
            "NormalMap",
            U::new()
                .set("image", default_image())
                .set("channels", channels("rgb"))
                .set("strength", float(0.8))
                .set("repeat", vec2(1.0, 1.0))
                .build(),
            // NormalMap 法线贴图：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::NORMAL_MAP_MATERIAL,
            // 半透明规则：从不。
            Some(TranslucentSpec::Never),
        ),
    );

    // Grid：网格线材质。color + cellAlpha + lineCount/lineThickness/lineOffset；
    // 当 color.alpha 或 cellAlpha 任一 <1 时半透明。
    add(
        "Grid",
        CachedMaterial::from_source(
            "Grid",
            U::new()
                .set("color", color(0.0, 1.0, 0.0, 1.0))
                .set("cellAlpha", float(0.1))
                .set("lineCount", vec2(8.0, 8.0))
                .set("lineThickness", vec2(1.0, 1.0))
                .set("lineOffset", vec2(0.0, 0.0))
                .build(),
            // Grid 网格线：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::GRID_MATERIAL,
            // 半透明规则：color 或 cellAlpha 任一 alpha<1。
            Some(TranslucentSpec::AnyAlphaLt1(vec!["color", "cellAlpha"])),
        ),
    );

    // Stripe：条纹材质。horizontal 控制方向，even/oddColor 交替，
    // offset/repeat 调节相位与密度；任一颜色 alpha<1 时半透明。
    add(
        "Stripe",
        CachedMaterial::from_source(
            "Stripe",
            U::new()
                .set("horizontal", bool_u(true))
                .set("evenColor", color(1.0, 1.0, 1.0, 0.5))
                .set("oddColor", color(0.0, 0.0, 1.0, 0.5))
                .set("offset", float(0.0))
                .set("repeat", float(5.0))
                .build(),
            // Stripe 条纹：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::STRIPE_MATERIAL,
            // 半透明规则：evenColor 或 oddColor 任一 alpha<1。
            Some(TranslucentSpec::AnyAlphaLt1(vec!["evenColor", "oddColor"])),
        ),
    );

    // Checkerboard：棋盘格材质。lightColor/darkColor + vec2 repeat；
    // 任一颜色 alpha<1 时半透明。
    add(
        "Checkerboard",
        CachedMaterial::from_source(
            "Checkerboard",
            U::new()
                .set("lightColor", color(1.0, 1.0, 1.0, 0.5))
                .set("darkColor", color(0.0, 0.0, 0.0, 0.5))
                .set("repeat", vec2(5.0, 5.0))
                .build(),
            // Checkerboard 棋盘格：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::CHECKERBOARD_MATERIAL,
            // 半透明规则：lightColor 或 darkColor 任一 alpha<1。
            Some(TranslucentSpec::AnyAlphaLt1(vec!["lightColor", "darkColor"])),
        ),
    );

    // Dot：点阵材质。lightColor/darkColor + vec2 repeat；以内联 GLSL 生成圆点。
    // 任一颜色 alpha<1 时半透明。
    add(
        "Dot",
        CachedMaterial::from_source(
            "Dot",
            U::new()
                .set("lightColor", color(1.0, 1.0, 0.0, 0.75))
                .set("darkColor", color(0.0, 1.0, 1.0, 0.75))
                .set("repeat", vec2(5.0, 5.0))
                .build(),
            // Dot 点阵：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::DOT_MATERIAL,
            // 半透明规则：lightColor 或 darkColor 任一 alpha<1。
            Some(TranslucentSpec::AnyAlphaLt1(vec!["lightColor", "darkColor"])),
        ),
    );

    // Water：水面材质。含 baseWaterColor/blendColor、specularMap/normalMap 两张
    // 纹理，以及 frequency/animationSpeed/amplitude/specularIntensity/fadeFactor
    // 等标量控制波浪与高光。baseWaterColor 或 blendColor 的 alpha<1 时半透明。
    add(
        "Water",
        CachedMaterial::from_source(
            "Water",
            U::new()
                .set("baseWaterColor", color(0.2, 0.3, 0.6, 1.0))
                .set("blendColor", color(0.0, 1.0, 0.699, 1.0))
                .set("specularMap", default_image())
                .set("normalMap", default_image())
                .set("frequency", float(10.0))
                .set("animationSpeed", float(0.01))
                .set("amplitude", float(1.0))
                .set("specularIntensity", float(0.5))
                .set("fadeFactor", float(1.0))
                .build(),
            // Water 水面：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::WATER_MATERIAL,
            // 半透明规则：baseWaterColor 或 blendColor 任一 alpha<1。
            Some(TranslucentSpec::AnyAlphaLt1(vec![
                "baseWaterColor",
                "blendColor",
            ])),
        ),
    );

    // RimLighting：边缘光照材质。color + rimColor + width；沿视图边缘产生光晕。
    // color 或 rimColor 的 alpha<1 时半透明。
    add(
        "RimLighting",
        CachedMaterial::from_source(
            "RimLighting",
            U::new()
                .set("color", color(1.0, 0.0, 0.0, 0.7))
                .set("rimColor", color(1.0, 1.0, 1.0, 0.4))
                .set("width", float(0.3))
                .build(),
            // RimLighting 边缘光照：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::RIM_LIGHTING_MATERIAL,
            // 半透明规则：color 或 rimColor 任一 alpha<1。
            Some(TranslucentSpec::AnyAlphaLt1(vec!["color", "rimColor"])),
        ),
    );

    // Fade：淡出材质。fadeInColor/fadeOutColor + maximumDistance/repeat/
    // fadeDirection/time；根据与中心的距离在两色间插值淡出。
    // 任一颜色的 alpha<1 时半透明。
    add(
        "Fade",
        CachedMaterial::from_source(
            "Fade",
            U::new()
                .set("fadeInColor", color(1.0, 0.0, 0.0, 1.0))
                .set("fadeOutColor", color(0.0, 0.0, 0.0, 0.0))
                .set("maximumDistance", float(0.5))
                .set("repeat", bool_u(true))
                .set("fadeDirection", vec2(1.0, 1.0))
                .set("time", vec2(0.5, 0.5))
                .build(),
            // Fade 淡出：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::FADE_MATERIAL,
            // 半透明规则：fadeInColor 或 fadeOutColor 任一 alpha<1。
            Some(TranslucentSpec::AnyAlphaLt1(vec![
                "fadeInColor",
                "fadeOutColor",
            ])),
        ),
    );

    // PolylineArrow：箭头线材质。仅 color；总是半透明（Always），用于带方向的箭头。
    add(
        "PolylineArrow",
        CachedMaterial::from_source(
            "PolylineArrow",
            U::new().set("color", color(1.0, 1.0, 1.0, 1.0)).build(),
            // PolylineArrow 箭头线：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::POLYLINE_ARROW_MATERIAL,
            // 半透明规则：总是（Always）。
            Some(TranslucentSpec::Always),
        ),
    );

    // PolylineDash：虚线材质。color/gapColor + dashLength/dashPattern（位掩码）；
    // 总是半透明（Always）。
    add(
        "PolylineDash",
        CachedMaterial::from_source(
            "PolylineDash",
            U::new()
                .set("color", color(1.0, 0.0, 1.0, 1.0))
                .set("gapColor", color(0.0, 0.0, 0.0, 0.0))
                .set("dashLength", float(16.0))
                .set("dashPattern", float(255.0))
                .build(),
            // PolylineDash 虚线：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::POLYLINE_DASH_MATERIAL,
            // 半透明规则：总是（Always）。
            Some(TranslucentSpec::Always),
        ),
    );

    // PolylineGlow：发光线材质。color + glowPower/taperPower 控制光晕衰减；
    // 总是半透明（Always）。
    add(
        "PolylineGlow",
        CachedMaterial::from_source(
            "PolylineGlow",
            U::new()
                .set("color", color(0.0, 0.5, 1.0, 1.0))
                .set("glowPower", float(0.25))
                .set("taperPower", float(1.0))
                .build(),
            // PolylineGlow 发光线：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::POLYLINE_GLOW_MATERIAL,
            // 半透明规则：总是（Always）。
            Some(TranslucentSpec::Always),
        ),
    );

    // PolylineOutline：描边线材质。color + outlineColor + outlineWidth；
    // color 或 outlineColor 的 alpha<1 时半透明。
    add(
        "PolylineOutline",
        CachedMaterial::from_source(
            "PolylineOutline",
            U::new()
                .set("color", color(1.0, 1.0, 1.0, 1.0))
                .set("outlineColor", color(1.0, 0.0, 0.0, 1.0))
                .set("outlineWidth", float(1.0))
                .build(),
            // PolylineOutline 描边线：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::POLYLINE_OUTLINE_MATERIAL,
            // 半透明规则：color 或 outlineColor 任一 alpha<1。
            Some(TranslucentSpec::AnyAlphaLt1(vec!["color", "outlineColor"])),
        ),
    );

    // ElevationContour：高程等高线材质。spacing/color/width；从不半透明。
    add(
        "ElevationContour",
        CachedMaterial::from_source(
            "ElevationContour",
            U::new()
                .set("spacing", float(100.0))
                .set("color", color(1.0, 0.0, 0.0, 1.0))
                .set("width", float(1.0))
                .build(),
            // ElevationContour 高程等高线：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::ELEVATION_CONTOUR_MATERIAL,
            // 半透明规则：从不。
            Some(TranslucentSpec::Never),
        ),
    );

    // ElevationRamp：高程渐变材质。image + minimumHeight/maximumHeight；
    // 将高程映射为颜色带。从不半透明。
    add(
        "ElevationRamp",
        CachedMaterial::from_source(
            "ElevationRamp",
            U::new()
                .set("image", default_image())
                .set("minimumHeight", float(0.0))
                .set("maximumHeight", float(10000.0))
                .build(),
            // ElevationRamp 高程渐变：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::ELEVATION_RAMP_MATERIAL,
            // 半透明规则：从不。
            Some(TranslucentSpec::Never),
        ),
    );

    // SlopeRamp：坡度渐变材质。仅 image；将坡度映射为颜色带。从不半透明。
    add(
        "SlopeRamp",
        CachedMaterial::from_source(
            "SlopeRamp",
            U::new().set("image", default_image()).build(),
            // SlopeRamp 坡度渐变：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::SLOPE_RAMP_MATERIAL,
            // 半透明规则：从不。
            Some(TranslucentSpec::Never),
        ),
    );

    // AspectRamp：宽高比（朝向）渐变材质。仅 image；从不半透明。
    add(
        "AspectRamp",
        CachedMaterial::from_source(
            "AspectRamp",
            U::new().set("image", default_image()).build(),
            // AspectRamp 朝向渐变：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::ASPECT_RAMP_MATERIAL,
            // 半透明规则：从不。
            Some(TranslucentSpec::Never),
        ),
    );

    // ElevationBand：高程分层材质。heights/colors 两张纹理；总是半透明（Always）。
    add(
        "ElevationBand",
        CachedMaterial::from_source(
            "ElevationBand",
            U::new()
                .set("heights", default_image())
                .set("colors", default_image())
                .build(),
            // ElevationBand 高程分层：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::ELEVATION_BAND_MATERIAL,
            // 半透明规则：总是（Always）。
            Some(TranslucentSpec::Always),
        ),
    );

    // WaterMask：水面遮罩材质。waterColor/landColor；从不半透明。
    // 用于标记哪些区域为水面。
    add(
        "WaterMask",
        CachedMaterial::from_source(
            "WaterMask",
            U::new()
                .set("waterColor", color(1.0, 1.0, 1.0, 1.0))
                .set("landColor", color(0.0, 0.0, 0.0, 0.0))
                .build(),
            // WaterMask 水面遮罩：逐字嵌入的内联 GLSL 着色器源码常量。
            glsl::WATER_MASK_MATERIAL,
            // 半透明规则：从不。
            Some(TranslucentSpec::Never),
        ),
    );

    // 逐项返回内置材质定义，由调用方写入缓存
    out
}

/// 内置材质类型名。与 [`builtin_materials`] 中的定义一一对应。
pub const BUILTIN_MATERIAL_TYPES: [&str; 25] = [
    "Color",
    "Image",
    "DiffuseMap",
    "AlphaMap",
    "SpecularMap",
    "EmissionMap",
    "BumpMap",
    "NormalMap",
    "Grid",
    "Stripe",
    "Checkerboard",
    "Dot",
    "Water",
    "RimLighting",
    "Fade",
    "PolylineArrow",
    "PolylineDash",
    "PolylineGlow",
    "PolylineOutline",
    "ElevationContour",
    "ElevationRamp",
    "SlopeRamp",
    "AspectRamp",
    "ElevationBand",
    "WaterMask",
];

#[cfg(test)]
mod tests {
    use super::*;

    // 内置缓存应含 25 个材质且每个类型名均可查
    #[test]
    fn test_builtin_cache_has_25_materials() {
        let system = MaterialSystem::with_builtin_materials();
        assert_eq!(system.len(), 25);
        for name in BUILTIN_MATERIAL_TYPES {
            assert!(
                system.get_material(name).is_some(),
                "missing built-in material: {}",
                name
            );
        }
    }

    // from_type("Color") 应成功构建且默认半透明
    #[test]
    fn test_from_type_color() {
        let system = MaterialSystem::with_builtin_materials();
        let m = system.from_type("Color", BTreeMap::new()).unwrap();
        assert_eq!(m.type_name(), "Color");
        assert!(m.shader_source().contains("czm_getMaterial"));
        assert!(m.is_translucent()); // 默认 alpha 0.5
    }

    // from_type 传入的 uniforms 应覆盖内置默认值
    #[test]
    fn test_from_type_with_override() {
        let system = MaterialSystem::with_builtin_materials();
        let mut overrides = BTreeMap::new();
        overrides.insert("color".to_string(), color(0.0, 1.0, 0.0, 1.0));
        let m = system.from_type("Color", overrides).unwrap();
        assert_eq!(
            m.uniforms().get("color"),
            Some(&UniformValue::Vec4([0.0, 1.0, 0.0, 1.0]))
        );
        assert!(!m.is_translucent()); // alpha 1.0
    }

    // 未知类型名应报 UnknownMaterialType
    #[test]
    fn test_from_type_unknown_errors() {
        let system = MaterialSystem::with_builtin_materials();
        let err = system.from_type("DoesNotExist", BTreeMap::new()).unwrap_err();
        assert!(matches!(
            err,
            MaterialError::UnknownMaterialType { ref type_name } if type_name == "DoesNotExist"
        ));
    }

    // 所有内置材质都应能成功构建且非空源码
    #[test]
    fn test_all_builtins_build_successfully() {
        let system = MaterialSystem::with_builtin_materials();
        for name in BUILTIN_MATERIAL_TYPES {
            let m = system
                .from_type(name, BTreeMap::new())
                .unwrap_or_else(|e| panic!("built-in material {} failed to build: {}", name, e));
            assert_eq!(m.type_name(), name);
            assert!(!m.shader_source().is_empty());
        }
    }

    // 验证各内置材质预期的半透明/不透明分类
    #[test]
    fn test_builtin_translucency_expectations() {
        let system = MaterialSystem::with_builtin_materials();
        // 始终半透明的内置材质。
        for name in ["AlphaMap", "PolylineArrow", "PolylineDash", "PolylineGlow", "ElevationBand"] {
            assert!(
                system.from_type(name, BTreeMap::new()).unwrap().is_translucent(),
                "{} should be translucent",
                name
            );
        }
        // 从不半透明的内置材质。
        for name in ["DiffuseMap", "SpecularMap", "EmissionMap", "BumpMap", "NormalMap", "ElevationContour", "WaterMask"] {
            assert!(
                !system.from_type(name, BTreeMap::new()).unwrap().is_translucent(),
                "{} should be opaque",
                name
            );
        }
    }

    // create_material 应将新类型回写缓存供后续复用
    #[test]
    fn test_create_material_caches_new_type() {
        let mut system = MaterialSystem::with_builtin_materials();
        let fabric = FabricTemplate::from_json_str(
            r#"{"type": "MyCustom", "components": {"diffuse": "vec3(1.0)"}}"#,
        )
        .unwrap();
        let m = system
            .create_material(MaterialOptions {
                strict: false,
                translucent: None,
                fabric,
            })
            .unwrap();
        assert_eq!(m.type_name(), "MyCustom");
        assert!(system.get_material("MyCustom").is_some());

        // 相同类型的第二个材质复用已缓存的模板。
        let m2 = system.from_type("MyCustom", BTreeMap::new()).unwrap();
        assert_eq!(m2.type_name(), "MyCustom");
        assert!(m2.shader_source().contains("material.diffuse"));
    }

    // Grid 的 cellAlpha 与 color.alpha 任一 <1 均触发半透明
    #[test]
    fn test_grid_translucency_via_cell_alpha() {
        let system = MaterialSystem::with_builtin_materials();
        // 默认 Grid：color alpha 1.0 但 cellAlpha 0.1 -> 半透明。
        assert!(system.from_type("Grid", BTreeMap::new()).unwrap().is_translucent());
        // cellAlpha 1.0 且 color alpha 1.0 -> 不透明。
        let mut overrides = BTreeMap::new();
        overrides.insert("cellAlpha".to_string(), float(1.0));
        overrides.insert("color".to_string(), color(0.0, 1.0, 0.0, 1.0));
        assert!(!system.from_type("Grid", overrides).unwrap().is_translucent());
    }
}
