//! Sky dome：几何、材质，以及 `sky_atmosphere.wgsl` 的
//! CPU 侧 f32 镜像（M5-C）。
//!
//! # 这是什么
//! 上游将天空渲染为 `Scene/SkyAtmosphere`：一个环绕地球的大球，
//! 由逐像素单次散射积分着色
//! （`czm_computeScattering` + `czm_computeAtmosphereColor`）。本模块是其
//! cesiumrust 对应物：
//!
//! * [`build_sky_dome_mesh`] — dome 几何：一个**测地二十面球**
//!   （`Sphere::ico`，非 UV 球），半径为 [`SKY_DOME_RADIUS`] 个 render unit，
//!   从内部渲染。在 [`SKY_DOME_SUBDIVISIONS`] = 5 时那是
//!   `20·(s+1)²` = 720 个三角形，且由欧拉公式 `10·(s+1)²+2` = **362
//!   个顶点**；粗糙的 12° 边无害，因为片元着色器
//!   归一化 `world_position - camera`，它能从一个弦中点精确复现
//!   大圆方向（残差误差是边角的二阶量，在 1920 px / 60° 画面上约 0.008° ≈ ¼ px）。
//! * [`SkyDomeMaterial`] — 在 `@group(2) @binding(0)` 处绑定
//!   [`sky_atmosphere.wgsl`](super) 的 Bevy `Material`。
//! * [`SkyAtmosphereParams`] — uniform 块，持有每一个已从米
//!   （领域，f64）转为 render unit（GPU，f32）的物理
//!   参数。见 [`SkyAtmosphereParams::from_domain`]。
//! * [`spawn_sky_dome`] / [`despawn_sky_dome`] — 由 [`super::sky_system::sky_dome_setup`]
//!   驱动的实体 spawn/拆除逻辑。
//! * 那些 `*_f32` 自由函数 — WGSL 中每个 helper 的一个**逐运算 CPU 镜像**，
//!   使生产散射算法可在无头下做单元测试
//!   （无 GPU、无窗口、无 wgpu），而非只能在 xvfb e2e runner 上被检查。
//!
//! # 门控
//! [`sky_dome_gate_enabled()`] 是对 `CESIUM_ENABLE_SKYDOME` 的适配层本地读取，
//! 即 `feature_flags::skydome_enabled()` 在 cesium-app 中读取以决定
//! `CesiumAtmospherePlugin` 是否被注册的同一个变量（main.rs L513-516）。它在此被复制，
//! 因为适配层无法导入应用层（DDD）；解析委派给
//! [`crate::pipeline::fetch::gate_from_env_value`]，其真值集
//! （`"1"|"true"|"yes"|"on"`，去空白 + 转小写）与 `feature_flags::env_flag`
//! 逐字节相同。与 `effects::graph::postprocess_gate_enabled` 和
//! `tileset::content_loader::gltf_upgrade_gate_enabled` 同一模式。
//!
//! 当门控 OFF 时插件从不被注册，[`super::sky_system::sky_system`] 从不运行，
//! 而天空保持 M5-C 之前的 `ClearColor`——因此八个 v0 基线
//! 不受扰动（PSNR = infinity）。
//!
//! # 渲染顺序
//! 需求是“dome 在星野之后、地球之前”。Bevy 的 `Core3d` 阶段顺序是固定的
//! （`Opaque3d` → `AlphaMask3d` → `Transmissive3d` → `Transparent3d`），所以
//! 若没有自定义 render-graph 节点（M5-E0 领域），一个透明 dome 永远无法*字面上*
//! 画在不透明地球之前。取而代之，三种机制组合出所需的像素结果：
//!
//! 1. **dome 在星野之后** — 两个实体都以世界原点为中心，所以
//!    `ViewRangefinder3d::distance_translation` 对两者返回*相同*的视图空间 Z，
//!    而 `Transparent3d` 的 `sort_by_key(distance)`（升序，即由后到前）
//!    将是一场掷硬币——不可复现的帧。[`SkyDomeMaterial::depth_bias`] 将
//!    [`SKY_DOME_DEPTH_BIAS`] 加到 dome 的排序距离上，这把它在 `orbit_camera`
//!    的 `[1.005, 20.0]` 范围内每个相机距离上都严格钉在星野*之后*。这很重要，
//!    因为星野是 `AlphaMode::Blend`，而 alpha 混合不满足交换律。
//! 2. **地球遮蔽 dome** — `AlphaMode::Premultiplied` 把 dome 放入 `Transparent3d`，
//!    其 `depth_write_enabled = false` 且 `depth_compare = GreaterEqual`
//!    （reversed-Z）。不透明地球已经写入了深度缓冲，所以它后面每个 dome 片元
//!    都未通过深度测试。与“先画 dome、地球覆于其上”逐像素相同。
//! 3. **单一壳层** — `cull_mode = Some(Face::Front)`（经由
//!    [`SkyDomeMaterial::specialize`]）只保留远半球，所以散射积分沿一条射线
//!    从不被应用两次。与 `application/cesium-app/src/atmosphere_glow.rs` L83
//!    同一手法。
//!
//! 记录为 `docs/deviations.md#dev-021`。
//!
//! # 单位
//! `1 render unit = METERS_PER_RENDER_UNIT = 6_378_137 m`。长度被它除，
//! 散射系数被它乘，因而其乘积（光学深度）加上每个密度比 `exp(-h/H)`
//! 在重缩放下**完全不变**。正是这一不变性使 f64 领域参考与 f32 GPU
//! 结果可比；它由 [`tests::unit_invariance_optical_depth_and_density`] 数值断言。

use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{
    AsBindGroup, Face, RenderPipelineDescriptor, Shader, ShaderRef, ShaderType,
    SpecializedMeshPipelineError,
};
use cesium_atmosphere::scattering::AtmosphereParameters;

use crate::resources::METERS_PER_RENDER_UNIT;

/// cesium-app 中 `feature_flags::skydome_enabled()` 读取的环境变量，它门控
/// `CesiumAtmospherePlugin` 的注册（main.rs L513-516）。在此复制
/// 是因为适配层无法导入应用层（DDD）；见模块文档。
pub const ENV_ENABLE_SKYDOME: &str = "CESIUM_ENABLE_SKYDOME";

/// 对 [`ENV_ENABLE_SKYDOME`] 的适配层本地求值，语义上等同于
/// `feature_flags::skydome_enabled()`。
#[inline]
pub fn sky_dome_gate_enabled() -> bool {
    crate::pipeline::fetch::gate_from_env_value(std::env::var(ENV_ENABLE_SKYDOME).ok())
}

/// 指向内嵌单次散射 WGSL 着色器的强 handle。
///
/// 经由 [`crate::shader_registry::try_load_internal_shader`] 从
/// [`super::CesiumAtmospherePlugin`] 以无头安全方式注册，绝不通过裸
/// `load_internal_asset!`（后者在 `MinimalPlugins` 下会 panic——见 `docs/deviations.md#dev-005`）。
///
/// 该 u128 是 `"SKYDOMEATMOS\0\0"` 的 ASCII 字节，遵循
/// `FABRIC_MATERIAL_SHADER_HANDLE` 约定。
pub const SKY_ATMOSPHERE_SHADER_HANDLE: Handle<Shader> =
    Handle::weak_from_u128(0x534B_5944_4F4D_4541_544D_4F53_0000);

/// dome 半径，以 render unit 计。
///
/// 约束，全部由 [`tests::dome_radius_fits_the_camera_frustum`] 断言：
/// * `> orbit_camera::OrbitState::max_distance`（20.0），使相机总在 dome
///   *内部*；
/// * `< starfield` 的球半径（50.0），使 dome 包住地球但坐在星野
///   内部，让星星被它熄灭；
/// * `< orbit_camera::CAMERA_FAR`（200.0），使其不被裁剪；
/// * `> cesium_atmosphere::scattering::constants::OUTER_RADIUS / MPU`
///   （= 1.0157），使 dome 在几何上包住被建模的大气。
pub const SKY_DOME_RADIUS: f32 = 40.0;

/// dome mesh 的二十面球细分级别。
///
/// `5` 产生 `20 * 4^5 = 20480` 个三角形，即约 160 段轮廓——
/// 1080p 下亚 0.2 px 的弦误差，所以边缘读作一个平滑圆。
/// 散射积分本身是逐片元的，所以几何只需足够圆，
/// 使 `world_position` 插值保持精确。
pub const SKY_DOME_SUBDIVISIONS: u32 = 5;

/// 排序距离偏置，强制 dome 在 `Transparent3d` 中严格位于星野之后。
/// 取正因 `Transparent3d::sort` 对视图空间 Z 是*升序*，而相机朝 −Z 看，
/// 所以升序 == 由后到前；因此更大的距离意味着“更晚绘制”。见模块文档。
pub const SKY_DOME_DEPTH_BIAS: f32 = 1000.0;

/// `sky_atmosphere.wgsl` 的 `MODE_RAYMARCH`：生产级 Hillaire/CesiumJS 双
/// ray-march 单次散射。
pub const MODE_RAYMARCH: u32 = 0;

/// `sky_atmosphere.wgsl` 的 `MODE_CLOSED_FORM`：领域
/// [`cesium_atmosphere::scattering::compute_sky_color`] 在海水准面的逐运算镜像，用作
/// GPU↔CPU 一致性探针。非生产外观。
pub const MODE_CLOSED_FORM: u32 = 1;

/// Rayleigh 相位归一化 `3/(16*pi)`。
///
/// 在蓝图（`computeAtmosphereColor` L33，那里写作
/// `3.0/50.2654824574`）与领域（`scattering.rs` L82）中相同——两者唯一
/// 达成一致的相位常数。
///
/// 该字面量写作 *f64 表达式*而非小数，以便
/// (a) `clippy::excessive_precision` 不会触发，且 (b) 该值可证明是
/// 精确实数 `3/(16*pi)` 的正确舍入 f32，它与 naga 将 `sky_atmosphere.wgsl`
/// L193 中同一十进制字面量收窄到的值逐位相等。
/// [`tests::wgsl_constants_match_the_rust_constants`] 断言那一等位性。
pub const RAYLEIGH_PHASE_K: f32 = (3.0_f64 / (16.0 * std::f64::consts::PI)) as f32;

/// Mie/Henyey-Greenstein 相位归一化 `1/(4*pi)`——**领域**值
/// （`scattering.rs` L94），也是本项目的默认。选择它是因 M5-C
/// 验收门控是与领域 f64 参考保持一致。写作 f64
/// 表达式的原因与 [`RAYLEIGH_PHASE_K`] 的等位性理由相同。
pub const MIE_PHASE_K_DOMAIN: f32 = (1.0_f64 / (4.0 * std::f64::consts::PI)) as f32;

/// Mie 相位归一化 `3/(8*pi)`——**蓝图**值
/// （`computeAtmosphereColor` L35，那里写作 `3.0/25.1327412287`）。
/// 恰好是 [`MIE_PHASE_K_DOMAIN`] 的 1.5 倍；形状相同，归一化不同。
/// 保留它是为着色器可切换到忠于蓝图的输出。写作 f64
/// 表达式；与 `sky_atmosphere.wgsl` L196 逐位相等。
pub const MIE_PHASE_K_BLUEPRINT: f32 = (3.0_f64 / (8.0 * std::f64::consts::PI)) as f32;

/// 地平线/天空阶跃-分裂 sigmoid 的锐度
/// （`sky_atmosphere.wgsl` D1），以 `sin(elevation)` 为单位。
pub const HORIZON_SPLIT_SHARPNESS: f32 = 8.0;

/// `computeScattering` L26 的 `PRIMARY_STEPS_MAX`。
pub const PRIMARY_STEPS_MAX: u32 = 16;

/// `computeScattering` L27 的 `LIGHT_STEPS_MAX`。
pub const LIGHT_STEPS_MAX: u32 = 4;

/// 平方 epsilon，低于它太阳方向 uniform 不被重新推送，从而
/// 在 `FIXED_TIME` 下 bind group 不会每帧被弄脏。
const SUN_UNIFORM_EPSILON_SQ: f32 = 1.0e-12;

/// `sky_atmosphere.wgsl::DEGENERATE_DIRECTION_EPSILON` 的镜像（D9 守卫）。
/// 在此值或更低时，`direction.dot(direction)` 计为零，且
/// [`ray_sphere_interval_f32`] 报告一次未命中，而非除以 `2*a = 0`。
///
/// 到达该函数的两个生产方向都会先被归一化（片元头里的 `ray_direction`，
/// [`update_sun_direction`] 里的 `sun_direction`），所以该值要么是 1.0 要么恰好是 0.0，
/// 阈值只需区分这两者。
pub const DEGENERATE_DIRECTION_EPSILON: f32 = 1.0e-12;

// ---------------------------------------------------------------------------
// Uniform 块
// ---------------------------------------------------------------------------

/// `sky_atmosphere.wgsl` 的 `@group(2) @binding(0)` uniform 块。
///
/// 字段顺序、填充与偏移是承重性的：它们必须与 WGSL
/// `struct SkyAtmosphereParams` 完全匹配（encase std140 布局，总大小 80
/// 字节，对齐 16）。[`tests::wgsl_uniform_layout_matches_rust`] 守卫它。
///
/// 位于一个带 `allow(dead_code)` 的私有模块中，因为 `ShaderType`
/// derive 会为每个字段生成访问器，而只有部分从 Rust 读取——这与
/// `fabric_material.rs` 对 `FabricParams` 使用的假阳性抑制模式相同。
mod sky_params {
    #![allow(dead_code)]
    use super::*;

    /// GPU 侧大气参数，以 **render unit** 和 **f32** 计。
    #[derive(ShaderType, Debug, Clone, Copy, PartialEq)]
    pub struct SkyAtmosphereParams {
        /// 朝太阳的单位向量，世界空间（offset 0）。
        ///
        /// 这里的世界空间*就是* `celestial_system` 发布的 ECI 参考系，所以
        /// 这正是驱动 `DirectionalLight` 的那个向量。
        pub sun_direction: Vec3,
        /// 地球表面半径，render unit——`inner_radius / MPU` = 1.0（offset 12）。
        pub inner_radius: f32,
        /// Rayleigh 尺度高度——`8000 m / MPU`（offset 16）。
        pub rayleigh_scale_height: f32,
        /// Mie 尺度高度——`1200 m / MPU`（offset 20）。
        pub mie_scale_height: f32,
        /// 大气外半径——`(Earth + 100_000 m) / MPU`（offset 24）。
        pub outer_radius: f32,
        /// 将下一个 `vec3` 对齐到 16 的 std140 填充（offset 28）。
        pub pad0: f32,
        /// Rayleigh 散射系数 × MPU，每米 → 每 render unit（offset 32）。
        pub rayleigh_coefficient: Vec3,
        /// Mie 散射系数 × MPU（offset 44）。
        pub mie_coefficient: f32,
        /// Henyey-Greenstein 各向异性 `g`（offset 48）。无量纲。
        pub mie_anisotropy: f32,
        /// 太阳强度乘子（offset 52）。无量纲。
        pub solar_intensity: f32,
        /// Mie 相位归一化常数（offset 56）。见 [`MIE_PHASE_K_DOMAIN`]。
        pub mie_phase_k: f32,
        /// [`MODE_RAYMARCH`] 或 [`MODE_CLOSED_FORM`]（offset 60）。
        pub mode: u32,
        /// 主 ray-march 步数预算（offset 64）。
        pub primary_steps_max: u32,
        /// 光线 ray-march 步数预算（offset 68）。
        pub light_steps_max: u32,
        /// std140 尾部填充（offset 72）。
        pub pad1: u32,
        /// std140 尾部填充（offset 76）。
        pub pad2: u32,
    }

    impl Default for SkyAtmosphereParams {
        /// 默认：从领域 [`AtmosphereParameters::default`] 导出 uniform 镜像。
        fn default() -> Self {
            Self::from_domain(&AtmosphereParameters::default())
        }
    }
}

pub use sky_params::SkyAtmosphereParams;

impl SkyAtmosphereParams {
    /// 将领域的 f64 米制 [`AtmosphereParameters`] 转为
    /// f32 render-unit uniform 块。
    ///
    /// **每个长度都除以 [`METERS_PER_RENDER_UNIT`]，每个每米
    /// 散射系数都乘以它**——这两次转换在光学深度中相互抵消，所以着色结果
    /// 是单位不变的（见模块文档与
    /// [`tests::unit_invariance_optical_depth_and_density`]）。
    ///
    /// `sun_direction` 被播种为 `Vec3::X`——与
    /// [`LightingParams::default()`](super::celestial_system::LightingParams) 所携带的
    /// 同一值——并由 [`super::sky_system::sky_system`] 每帧从
    /// `LightingParams` 刷新。
    ///
    /// 它绝不能是 `Vec3::ZERO`。光线-球求交除以
    /// `2 * dot(direction, direction)`，所以零方向会使它成为 `0/0 =
    /// NaN`；`max(interval.y, 0.0)` 不会过滤它（`NaN < 0.0` 为假），
    /// 且在 `AlphaMode::Premultiplied` 下 NaN alpha 会在被 bloom/FXAA
    /// 进一步扩散之前涂抹整片天空。见 WGSL `D9` 注。
    /// [`ray_sphere_interval_f32`] 现在已对此作守卫，但向一个被作除数的字段
    /// 播种一个*退化*值才是真正的缺陷：守卫是纵深防御，这个才是修复。
    pub fn from_domain(p: &AtmosphereParameters) -> Self {
        let mpu = METERS_PER_RENDER_UNIT;
        Self {
            sun_direction: Vec3::X,
            // 长度：米 -> render unit
            inner_radius: (p.inner_radius / mpu) as f32,
            outer_radius: (p.outer_radius / mpu) as f32,
            rayleigh_scale_height: (p.rayleigh_scale_height / mpu) as f32,
            mie_scale_height: (p.mie_scale_height / mpu) as f32,
            // 每米系数 -> 每 render unit 系数
            rayleigh_coefficient: Vec3::new(
                (p.rayleigh_coefficients[0] * mpu) as f32,
                (p.rayleigh_coefficients[1] * mpu) as f32,
                (p.rayleigh_coefficients[2] * mpu) as f32,
            ),
            mie_coefficient: (p.mie_coefficient * mpu) as f32,
            // 无量纲：在 GPU 边界直接 f64 -> f32 收窄
            mie_anisotropy: p.mie_anisotropy as f32,
            solar_intensity: p.solar_intensity as f32,
            mie_phase_k: MIE_PHASE_K_DOMAIN,
            mode: MODE_RAYMARCH,
            primary_steps_max: PRIMARY_STEPS_MAX,
            light_steps_max: LIGHT_STEPS_MAX,
            pad0: 0.0,
            pad1: 0,
            pad2: 0,
        }
    }

    /// 大气厚度，以 render unit 计（`outer_radius - inner_radius`）。
    #[inline]
    pub fn thickness(&self) -> f32 {
        self.outer_radius - self.inner_radius
    }
}

// ---------------------------------------------------------------------------
// 材质
// ---------------------------------------------------------------------------

/// 将 `sky_atmosphere.wgsl` 绑定到 dome mesh 的 Bevy 材质。
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct SkyDomeMaterial {
    /// 单次散射 uniform 块。
    #[uniform(0)]
    pub params: SkyAtmosphereParams,
}

impl Material for SkyDomeMaterial {
    /// 指向已注册的 `sky_atmosphere.wgsl` 片元着色器 handle。
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SKY_ATMOSPHERE_SHADER_HANDLE)
    }

    /// 取 Premultiplied，使片元的前向散射辐亮度按 `dst = radiance + dst * (1 - alpha)`
    /// 合成到 dome 后方的星野/地球上。
    ///
    /// 注意这里什么是、什么不是逐通道的（WGSL `D3` 注带有同一修正）：
    /// **前向散射**是逐通道且不加衰减地被相加，这就是物理正确的单次散射
    /// 合成。**背景衰减**则不是——`Premultiplied` 携带一个*标量* alpha，
    /// 所以坐在 dome 后的任何东西都被 `1 - mean(transmittance)` 熄灭，
    /// 三个通道共用一个数。逐通道透射率 vec3 之所以被计算，是因为物理如此要求
    /// 且 CPU 镜像/测试会检查它，但只有其算术平均值到达帧缓冲。
    ///
    /// `Premultiplied` 还强制 `depth_write_enabled = false` 并把 mesh 路由进
    /// `Transparent3d`，这正是让不透明地球通过深度测试遮蔽 dome 的原因。
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    /// 把 dome 的 `Transparent3d` 排序距离严格钉在星野之上，使两个
    /// 以原点为中心的球永不相持（见模块文档，机制 1）。
    fn depth_bias(&self) -> f32 {
        SKY_DOME_DEPTH_BIAS
    }

    /// 用 `Face::Front` 覆盖默认的 `cull_mode: Some(Face::Back)`
    /// 只保留*远离*相机的半球，所以散射积分沿每条射线只应用一次
    /// （模块文档，机制 3）。
    ///
    /// `descriptor.vertex.buffers` 在此 hook 运行前已由
    /// `MeshPipeline::specialize` 填好（bevy_pbr 0.15 `material.rs` L411 → L422），
    /// 所以只触及 primitive 状态。
    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = sky_dome_cull_mode();
        Ok(())
    }
}

/// dome 的光栅化器 cull mode，从 [`Material::specialize`] 中抽出，
/// 使该决策可在无头下做单元测试（构造 `MaterialPipeline` 需要一个活的 `RenderDevice`）。
///
/// 取 `Face::Front` 因为相机总在 dome *内部*（[`SKY_DOME_RADIUS`] >
/// `orbit_camera` 的 20.0 最大距离）：近半球正面朝向并被剔除，沿每条
/// 射线恰好留一个壳层。与 `application/cesium-app/src/atmosphere_glow.rs` L83 同一手法。
pub fn sky_dome_cull_mode() -> Option<Face> {
    Some(Face::Front)
}

/// sky dome 实体的标记，以便它能独立于场景其余部分被查找并拆除。
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct SkyDome;

// ---------------------------------------------------------------------------
// 几何 + 实体 spawn
// ---------------------------------------------------------------------------

/// 构建 dome mesh：一个半径为 [`SKY_DOME_RADIUS`] render unit 的完整二十面球，
/// 以世界原点（地球中心）为中心。
///
/// 是一个*完整*球而非半球：在 `cull_mode = Face::Front` 下近一半被剔除，
/// 所以可见几何恰好是远半球，且对每种相机朝向地平线都保持闭合
/// （一个固定半球会在相机背离其轴时留下一个洞）。
///
/// `SphereKind::Ico` 是应用层 glow 壳层使用的同一构建器
/// （`application/cesium-app/src/atmosphere_glow.rs`），且它发出
/// `MeshPipeline::specialize` 所请求的属性集。
pub fn build_sky_dome_mesh(meshes: &mut Assets<Mesh>) -> Handle<Mesh> {
    meshes.add(build_sky_dome_mesh_asset())
}

/// dome [`Mesh`] 本身，不带 [`Assets`] 存储——抽出以便它可在无头下
/// 做单元测试（构造 `Assets<Mesh>` 需要一个 `AssetServer`）。
pub fn build_sky_dome_mesh_asset() -> Mesh {
    Sphere::new(SKY_DOME_RADIUS)
        .mesh()
        .ico(SKY_DOME_SUBDIVISIONS)
        // 对这些常量而言不会失败：`ico` 只拒绝一个非有限半径
        // 或高于 16 的细分级别。与 `application/cesium-app/src/atmosphere_glow.rs` L54
        // 同样的 `.expect` 形态。
        .expect("sky dome icosphere subdivision failed")
}

/// Spawn sky dome 实体。
///
/// `Transform` 是单位：dome 与地球和星野同心，这正是使它们的
/// `Transparent3d` 排序距离相持的原因（也因此使 [`SKY_DOME_DEPTH_BIAS`] 成为必要）。
pub fn spawn_sky_dome(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<SkyDomeMaterial>,
    params: SkyAtmosphereParams,
) -> Entity {
    let mesh = build_sky_dome_mesh(meshes);
    let material = materials.add(SkyDomeMaterial { params });
    commands
        .spawn((SkyDome, Mesh3d(mesh), MeshMaterial3d(material)))
        .id()
}

/// 拆除每个 sky dome 实体。幂等。
pub fn despawn_sky_dome(commands: &mut Commands, domes: impl Iterator<Item = Entity>) {
    for entity in domes {
        commands.entity(entity).despawn();
    }
}

/// 将太阳方向重新推入 dome 材质，但仅当它移动超过 [`SUN_UNIFORM_EPSILON_SQ`] 时。
/// 在 `FIXED_TIME` 下时钟被冻结，所以这在首帧之后是空操作，
/// 保持基线捕获确定且 bind group 未被触及。
///
/// 若 uniform 被写入则返回 `true`。
pub fn update_sun_direction(
    materials: &mut Assets<SkyDomeMaterial>,
    handle: &Handle<SkyDomeMaterial>,
    sun_direction: Vec3,
) -> bool {
    let Some(material) = materials.get_mut(handle) else {
        return false;
    };
    let next = sun_direction.normalize_or_zero();
    let delta_sq = (material.params.sun_direction - next).length_squared();
    if delta_sq <= SUN_UNIFORM_EPSILON_SQ {
        return false;
    }
    material.params.sun_direction = next;
    true
}

// ---------------------------------------------------------------------------
// WGSL helper 的 CPU 镜像（逐运算，f32）
// ---------------------------------------------------------------------------

/// `sky_atmosphere.wgsl::approximate_tanh` 的镜像（蓝图
/// `approximateTanh` L7-10）。
#[inline]
pub fn approximate_tanh_f32(x: f32) -> f32 {
    let x2 = x * x;
    let numerator = x * (27.0 + x2);
    let denominator = 27.0 + 9.0 * x2;
    (numerator / denominator).clamp(-1.0, 1.0)
}

/// `sky_atmosphere.wgsl::ray_sphere_interval` 的镜像（蓝图
/// `raySphereIntersectionInterval` L1-37），球以原点为中心。
/// 返回 `(start, stop)`；`stop <= start` 表示“无交点”。
pub fn ray_sphere_interval_f32(origin: Vec3, direction: Vec3, radius: f32) -> (f32, f32) {
    let oc = origin;
    let a = direction.dot(direction);
    // D9（见 WGSL 头）：下面的 `two_a` 会恰好是 0.0，而 `b`/`det`
    // 也恰好是 0.0，所以 `t0 = (-0 - 0) / 0` = NaN。改为报告一次未命中。
    if a < DEGENERATE_DIRECTION_EPSILON {
        return (1.0, -1.0);
    }
    let b = 2.0 * direction.dot(oc);
    let radius_sq = radius * radius;
    let oc_sq = oc.dot(oc);
    let c = oc_sq - radius_sq;
    let b_sq = b * b;
    let four_ac = 4.0 * a * c;
    let det = b_sq - four_ac;
    if det < 0.0 {
        return (1.0, -1.0);
    }
    let sqrt_det = det.sqrt();
    let two_a = 2.0 * a;
    ((-b - sqrt_det) / two_a, (-b + sqrt_det) / two_a)
}

/// `sky_atmosphere.wgsl::rayleigh_phase` 的镜像（蓝图
/// `computeAtmosphereColor` L33 ≡ 领域 `scattering.rs` L81-83）。
#[inline]
pub fn rayleigh_phase_f32(cos_theta: f32) -> f32 {
    let cos_sq = cos_theta * cos_theta;
    let one_plus_cos_sq = 1.0 + cos_sq;
    RAYLEIGH_PHASE_K * one_plus_cos_sq
}

/// `sky_atmosphere.wgsl::mie_phase` 的镜像（蓝图
/// `computeAtmosphereColor` L35 ≡ 领域 `scattering.rs` L90-95）。
///
/// `k` 是归一化常数；传 [`MIE_PHASE_K_DOMAIN`] 以求领域一致，或传
/// [`MIE_PHASE_K_BLUEPRINT`] 以求蓝图一致。每个乘积都分别绑定，所以
/// 没有东西融合进一个 FMA，与 WGSL 和（禁用 fast-math 的）领域代码都匹配。
pub fn mie_phase_f32(cos_theta: f32, g: f32, k: f32) -> f32 {
    let g_sq = g * g;
    let cos_sq = cos_theta * cos_theta;
    let one_minus_g_sq = 1.0 - g_sq;
    let one_plus_cos_sq = 1.0 + cos_sq;
    let numerator_a = one_minus_g_sq * one_plus_cos_sq;
    let two_plus_g_sq = 2.0 + g_sq;
    let one_plus_g_sq = 1.0 + g_sq;
    let two_g = 2.0 * g;
    let two_g_cos = two_g * cos_theta;
    let base = one_plus_g_sq - two_g_cos;
    let base_p15 = base.max(1.0e-20).powf(1.5);
    let denominator_a = two_plus_g_sq * base_p15;
    let scaled_numerator = k * numerator_a;
    scaled_numerator / denominator_a
}

/// `sky_atmosphere.wgsl::horizon_split_weight` 的镜像（蓝图
/// `computeScattering` L50-53 的 D1 重写）。
#[inline]
pub fn horizon_split_weight_f32(sin_elevation: f32) -> f32 {
    let sharpened = sin_elevation * HORIZON_SPLIT_SHARPNESS;
    let t = approximate_tanh_f32(sharpened);
    0.5 * (1.0 + t)
}

/// `sky_atmosphere.wgsl::sin_elevation_at` 的镜像（D1 重写）。
pub fn sin_elevation_at_f32(ray_origin: Vec3, ray_direction: Vec3) -> f32 {
    let origin_length = ray_origin.length();
    if origin_length < 1.0e-9 {
        return 0.0;
    }
    let up = ray_origin / origin_length;
    ray_direction.dot(up)
}

/// `sky_atmosphere.wgsl::closed_form_radiance` 的镜像，即领域
/// `scattering.rs::compute_sky_color` 在 `camera_height = 0.0` 处的镜像（此处两个
/// `atmospheric_density` 因子都恰好是 1.0）。
///
/// 这是 GPU↔CPU 一致性探针：它消费 render-unit f32 参数，
/// 且必须在 f32 舍入误差内复现 f64 米制的领域函数。
pub fn closed_form_sky_color_f32(view_direction: Vec3, params: &SkyAtmosphereParams) -> [f32; 3] {
    let cos_theta = view_direction.dot(params.sun_direction);
    let rayleigh_p = rayleigh_phase_f32(cos_theta);
    let mie_p = mie_phase_f32(cos_theta, params.mie_anisotropy, params.mie_phase_k);

    // scattering.rs L131-133，camera_height = 0.0 时：exp(-0) = 1。
    let rayleigh_density = 1.0_f32;
    let mie_density = 1.0_f32;

    // scattering.rs L136.
    let path_length = params.thickness();

    // scattering.rs L139-143，逐通道，每个绑定一个乘积。
    let mut color = [0.0_f32; 3];
    for (channel, beta) in color.iter_mut().zip(
        [
            params.rayleigh_coefficient.x,
            params.rayleigh_coefficient.y,
            params.rayleigh_coefficient.z,
        ]
        .iter(),
    ) {
        let beta_times_density = *beta * rayleigh_density;
        let beta_density_phase = beta_times_density * rayleigh_p;
        let rayleigh_term = beta_density_phase * path_length;

        let mie_beta_density = params.mie_coefficient * mie_density;
        let mie_beta_density_phase = mie_beta_density * mie_p;
        let mie_term = mie_beta_density_phase * path_length;

        let summed = rayleigh_term + mie_term;
        *channel = summed * params.solar_intensity;
    }
    color
}

/// 一次主 ray march 产生的累加器。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScatteringMarch {
    /// `rayleighAccumulation`（`computeScattering` L77/L136），render unit。
    pub rayleigh_accumulation: Vec3,
    /// `mieAccumulation`（L78/L137），render unit。
    pub mie_accumulation: Vec3,
    /// `opticalDepth`（L79/L99）：`x` = rayleigh，`y` = mie。
    pub optical_depth: Vec2,
    /// 实际走的主步数（`PRIMARY_STEPS`，L66）。
    pub primary_steps: i32,
    /// 实际走的光步数（`LIGHT_STEPS`，L67）。
    pub light_steps: i32,
    /// `w_inside_atmosphere`（L65 的 D2 重写），在 `[0, 1]` 内。
    pub w_inside_atmosphere: f32,
    /// `w_stop_gt_lprl`（L53 的 D1 重写），在 `[0, 1]` 内。
    pub w_stop_gt_lprl: f32,
}

/// 一条射线的最终着色天空。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyShading {
    /// 前向散射辐亮度（`computeAtmosphereColor` L41），按曝光缩放。
    pub radiance: Vec3,
    /// 逐通道透射率（`computeScattering` L148 的 D3 重写）。
    /// 逐通道计算是因为物理如此要求，但只有其均值到达帧缓冲
    /// ——见 [`SkyShading::alpha`]。
    pub transmittance: Vec3,
    /// `1 - mean(transmittance)`，经 clamp。这是 `AlphaMode::Premultiplied`
    /// 混合实际用来衰减背景的**标量**（一个 L1/3 塌缩，而蓝图 B1 L148 用 L2
    /// `length()`），所以上面逐通道的 [`SkyShading::transmittance`] 并*不*逐通道地
    /// 熄灭星野。
    pub alpha: f32,
    /// 产生它的 march。
    pub march: ScatteringMarch,
}

/// `sky_atmosphere.wgsl::march_single_scattering` 的镜像
/// （`computeScattering` L83-141，应用了 D1/D2/D5 重写）。
///
/// 当射线未命中大气时返回 `None`（L39-44），此时
/// 着色器发出一个完全透明的片元。
pub fn march_single_scattering_f32(
    ray_origin: Vec3,
    ray_direction: Vec3,
    primary_ray_length: f32,
    params: &SkyAtmosphereParams,
) -> Option<ScatteringMarch> {
    // L39-44.
    let interval = ray_sphere_interval_f32(ray_origin, ray_direction, params.outer_radius);
    if interval.1 <= interval.0 {
        return None;
    }

    // L56-59.
    let start_0 = interval.0;
    let start = start_0.max(0.0);
    let stop = interval.1.min(primary_ray_length);
    if stop <= start {
        return None;
    }

    // L64-65，D2 重写：无量纲相机高度。D8 在海平面将其向下取整，
    // 使地面情形恰好是 0.5，而非它任一侧差一个 f32 ulp
    // （那会让 B1 L66 的截断把 10 舍到 9）。
    let thickness = params.thickness();
    let origin_radius = ray_origin.length();
    let camera_height = (origin_radius - params.inner_radius).max(0.0);
    let camera_height_norm = camera_height / thickness;
    let w_tanh = approximate_tanh_f32(camera_height_norm);
    let w_inside_atmosphere = 1.0 - 0.5 * (1.0 + w_tanh);

    // L66-67，clamp 到 >= 1，使下面的除法不会除以零。
    let primary_steps_f = params.primary_steps_max as f32 - w_inside_atmosphere * 12.0;
    let light_steps_f = params.light_steps_max as f32 - w_inside_atmosphere * 2.0;
    let primary_steps = (primary_steps_f as i32).max(1);
    let light_steps = (light_steps_f as i32).max(1);

    // L70, L73-75, with the D1 rewrite of `w_stop_gt_lprl`.
    let w_stop_gt_lprl = horizon_split_weight_f32(sin_elevation_at_f32(ray_origin, ray_direction));
    let ray_position_length = start;
    let total_ray_length = stop - ray_position_length;
    let tri = (primary_steps * (primary_steps + 1)) as f32;
    let half_tri = tri * 0.5;
    let one_minus_w_inside = 1.0 - w_inside_atmosphere;
    // L74 用 `1.0 - w_stop_gt_lprl` 驱动斜坡，而*不*用相机高度权重：
    // 见 WGSL 注释，说明为何在此替换它会静默禁用
    // 地平线/天空阶跃-分裂策略。
    let one_minus_w_stop = 1.0 - w_stop_gt_lprl;
    let ramp_numerator = one_minus_w_stop * total_ray_length;
    let ray_step_length_increase = w_inside_atmosphere * (ramp_numerator / half_tri);
    // L75.
    let base_weight = one_minus_w_inside.max(w_stop_gt_lprl);
    let base_numerator = base_weight * total_ray_length;
    let base_denominator = (7.0 * w_inside_atmosphere).max(primary_steps as f32);
    let ray_step_length = base_numerator / base_denominator;

    // L77-80.
    let height_scale = Vec2::new(params.rayleigh_scale_height, params.mie_scale_height);
    let mut optical_depth = Vec2::ZERO;
    let mut rayleigh_accumulation = Vec3::ZERO;
    let mut mie_accumulation = Vec3::ZERO;
    let mut cursor = ray_position_length;
    let mut step_length = ray_step_length;

    // L83-141.
    for _ in 0..primary_steps {
        // L92, L95.
        let sample_length = cursor + step_length;
        let sample_position = ray_origin + ray_direction * sample_length;
        let sample_radius = sample_position.length();
        // D7（见 WGSL 头）：在海平面向下取整，使一条穿过地球的主射线
        // 无法产生 `exp(+797) = inf`，进而在 L136 产生值为 NaN 的
        // `inf * exp(-inf)`。
        let sample_height = (sample_radius - params.inner_radius).max(0.0);

        // L98-99：对 (rayleigh, mie) 逐分量。
        let neg_height_over_scale = -sample_height / height_scale;
        let sample_density = neg_height_over_scale.exp() * step_length;
        optical_depth += sample_density;

        // L102-105.
        let light_direction = params.sun_direction;
        let light_interval =
            ray_sphere_interval_f32(sample_position, light_direction, params.outer_radius);
        // L105 只除以 `.stop`，而非 `stop - start`：光线总在外壳内部
        // 开始，所以 `start` 为负，减去它会使 march 越过大气边界。见 WGSL 注释。
        let light_stop = light_interval.1.max(0.0);
        let light_step_length = light_stop / light_steps as f32;

        // L111-130.
        let mut light_optical_depth = Vec2::ZERO;
        let mut light_cursor = 0.0_f32;
        for _ in 0..light_steps {
            // L120 采样每段的*中点*。
            let light_sample_length = light_cursor + light_step_length * 0.5;
            let light_position = sample_position + light_direction * light_sample_length;
            let light_radius = light_position.length();
            // D7：对每个被遮蔽的采样，光线都穿过地球。
            let light_height = (light_radius - params.inner_radius).max(0.0);
            let light_neg_h_over_scale = -light_height / height_scale;
            light_optical_depth += light_neg_h_over_scale.exp() * light_step_length;
            // L129：游标在采样*之后*才前进。
            light_cursor += light_step_length;
        }

        // L133：双向（主 + 光）消光，逐通道。
        let total_depth = optical_depth + light_optical_depth;
        let mie_depth = params.mie_coefficient * total_depth.y;
        let rayleigh_depth = params.rayleigh_coefficient * total_depth.x;
        let extinction = mie_depth + rayleigh_depth;
        let attenuation = (-extinction).exp();

        // L136-137.
        let rayleigh_contribution = attenuation * sample_density.x;
        let mie_contribution = attenuation * sample_density.y;
        rayleigh_accumulation += rayleigh_contribution;
        mie_accumulation += mie_contribution;

        // L140：`rayPositionLength += (rayStepLength += rayStepLengthIncrease)`。
        // 内层 `+=` 排在先，所以步长先增长，游标再据其前进。
        // 相反的顺序会把第一个之后的每个采样都偏移一个增量
        // （在地面累计约 5 km）。
        step_length += ray_step_length_increase;
        cursor += step_length;
    }

    Some(ScatteringMarch {
        rayleigh_accumulation,
        mie_accumulation,
        optical_depth,
        primary_steps,
        light_steps,
        w_inside_atmosphere,
        w_stop_gt_lprl,
    })
}

/// 整个 `sky_atmosphere.wgsl` mode-0 片元主体的镜像：march、应用
/// 相位函数与太阳强度（蓝图 `computeAtmosphereColor` L33-41），
/// 然后导出 D3 透射率。
///
/// `exposure` 是 Bevy 的 `view.exposure`。
pub fn shade_sky_f32(
    ray_origin: Vec3,
    ray_direction: Vec3,
    primary_ray_length: f32,
    params: &SkyAtmosphereParams,
    exposure: f32,
) -> Option<SkyShading> {
    let march = march_single_scattering_f32(ray_origin, ray_direction, primary_ray_length, params)?;

    // L144-145.
    let betas = params.rayleigh_coefficient;
    let rayleigh_color = betas * march.rayleigh_accumulation;
    let mie_color = march.mie_accumulation * params.mie_coefficient;

    // 前向散射组合，对应着色器相位/强度段 L33-41.
    let cos_theta = ray_direction.dot(params.sun_direction);
    let rayleigh_p = rayleigh_phase_f32(cos_theta);
    let mie_p = mie_phase_f32(cos_theta, params.mie_anisotropy, params.mie_phase_k);

    let rayleigh_scattered = rayleigh_color * rayleigh_p;
    let mie_scattered = mie_color * mie_p;
    let scattered = rayleigh_scattered + mie_scattered;
    // 先绑定标量增益：`scattered * (solar * exposure)`。从左到右相乘
    // （`(scattered * solar) * exposure`，即 WGSL 片元过去所写的）
    // 是一种不同的 f32 舍入——最多差 1 ULP。
    // f32 乘法满足交换律但不满足结合律。
    let gain = params.solar_intensity * exposure;
    let radiance = scattered * gain;

    // D3：前向散射保持逐通道；*背景*衰减是下面的标量，因为
    // `AlphaMode::Premultiplied` 用一个 alpha 混合。这里是 L1/3
    // 均值，蓝图 B1 L148 是 L2 `length()`——都是标量。
    let total_mie_depth = params.mie_coefficient * march.optical_depth.y;
    let total_rayleigh_depth = betas * march.optical_depth.x;
    let extinction = total_mie_depth + total_rayleigh_depth;
    let transmittance = (-extinction).exp();
    let mean_transmittance = transmittance.dot(Vec3::splat(1.0 / 3.0));
    let alpha = (1.0 - mean_transmittance).clamp(0.0, 1.0);

    Some(SkyShading {
        radiance,
        transmittance,
        alpha,
        march,
    })
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atmosphere::celestial_system::LightingParams;
    use crate::atmosphere::sky_system::SkyAtmosphere;
    use crate::atmosphere::CesiumAtmospherePlugin;
    use crate::clock::AnimationClock;
    use crate::pipeline::fetch::gate_from_env_value;
    use bevy::asset::AssetPlugin;
    use bevy::render::mesh::VertexAttributeValues;
    use bevy::render::render_resource::PrimitiveTopology;
    use cesium_atmosphere::celestial::compute_sun_direction_eci;
    use cesium_atmosphere::scattering::{
        compute_horizon_glow, compute_sky_color, mie_phase as mie_phase_domain,
    };
    use glam::DVec3;
    use std::sync::OnceLock;

    /// `AnimationClock::default()` 的 epoch：2024-01-01T00:00:00 UTC
    /// （`clock.rs` 的 `impl Default for AnimationClock`）。`FIXED_TIME` 把时钟冻结在其
    /// 起点，所以这个 Julian 日期是*每个* `v2_sky` 基线截图被捕获时所处的
    /// ——整个捕获所依赖的唯一输入。
    ///
    /// 它是从时钟读出的，而非拼写为名义上的 `2_460_310.5`，因为
    /// `JulianDate::from_date_components` 把 UTC 转为 **TAI**
    /// （`domain/time/src/julian_date.rs` L248-249），而 2024 年 TAI-UTC 为 37 s，
    /// 所以时钟发布 `2_460_310.5004282407`。那正是 `celestial_system` 交给
    /// `compute_sun_direction_eci` 的数，而那个函数逐字消费它的参数
    /// （`domain/atmosphere/src/celestial.rs` L29-30 —— 无 TAI->UTC 转换），所以在此
    /// 一具字面的 UTC Julian 日期会把 `v2_sky` 姿态从渲染实际绘制的天空
    /// 脱同步 37 s 的太阳运动。以太阳平均每热带年 360 deg 的 ECI 速率，那是
    /// 7.4e-6 rad，即 4.2e-4 deg —— 在 1920 px / 60 deg 画面上亚像素，所以它本不会被
    /// 看见，但它仍会是*错的*，且从它推导的每个数（`gpu_params` 里的 CPU 参考、
    /// 六个 TOML 姿态）都会携带同一偏移。读时钟使两者按构造同源，并把下面的
    /// epoch 断言从一个容差变为一个精确的逐位比较。
    fn frozen_julian_date() -> f64 {
        AnimationClock::default().current_time().total_days()
    }

    /// [`frozen_julian_date`] 处的太阳方向，拼写出来以让 `compute_sun_direction_eci`
    /// 中的任何漂移都在此处高声失败，而非静默使全部六个
    /// `specs/scripts/v2_sky.toml` 截图失效。
    ///
    /// **`docs/deferred.md` #46（领域 TAI->UTC，计划于 M13）的反向锚点。**
    /// 该值被烘焙在*适配层边界*之上，建立在一个已知的领域缺陷之上：
    /// `compute_sun_direction_eci` 逐字消费它的 Julian-日期参数，而 CesiumJS
    /// `Simon1994PlanetaryPositions.computeSunPositionInEarthInertialFrame` 以
    /// `julianDate = JulianDate.toUtc(julianDate)` 开头。`AnimationClock` 交出的是一个
    /// **TAI** 日期（`JulianDate::from_date_components` 转换 UTC->TAI，2024 年 37 s），
    /// 所以本模块中每个太阳方向都被系统性地偏移了 37 s 的太阳运动——
    /// 7.4e-6 rad，4.2e-4 deg，在 1920 px / 60 deg 画面上约 0.014 px。视觉上是零，
    /// 数值上非零。
    ///
    /// 冻结这个*带偏移的*值恰恰是使 `v2_sky` 在今天内部自洽的原因：六个 TOML 姿态、
    /// [`gpu_params`] 里的 CPU 参考与被渲染绘制的天空共享一个 epoch。这一耦合双向成立，
    /// 所以当 M13 给 `domain/atmosphere` 添加缺失的 TAI->UTC 转换时，
    /// **必须在同一变更中重新推导这个常数并重新捕获六个 `specs/baselines/v2_sky`
    /// 截图**。只修领域会留下这个锚点——因而也留下
    /// [`tests::sky_baseline_poses_encode_the_three_lighting_regimes`] 和每个 TOML 姿态——
    /// 静默地钉在修复前的天空上。
    ///
    /// 这一 trip-wire 是刻意的：[`tests::frozen_sun_direction_is_the_baseline_constant`]
    /// 在领域一变就转红。那红意味着**“基线已过时，重新推导并重新捕获”**，
    /// 而非“revert 领域修复”。
    const FROZEN_SUN_DIRECTION: [f64; 3] = [0.174_508_36, -0.903_425_27, -0.391_624_33];

    /// `orbit_camera` 的距离边界，在此重述是因为适配层
    /// 无法导入应用层（DDD）。真值取自：
    /// `application/cesium-app/src/orbit_camera.rs`。
    const ORBIT_MAX_DISTANCE: f32 = 20.0;
    /// `main.rs` 的透视远平面。
    const CAMERA_FAR: f32 = 200.0;
    /// `application/cesium-app/src/starfield.rs` L114 spawn 的星 sphere 半径；星野
    /// 必须包住 dome，dome 的透射率才能将其熄灭。
    const STARFIELD_RADIUS: f32 = 50.0;

    /// CRLF 归一化后的 `sky_atmosphere.wgsl`，使多行 `contains`
    /// 断言无论检出的行尾如何都能成立。
    fn wgsl() -> &'static str {
        static NORMALISED: OnceLock<String> = OnceLock::new();
        NORMALISED.get_or_init(|| include_str!("sky_atmosphere.wgsl").replace("\r\n", "\n"))
    }

    /// 着色器开头的仅-`//`-注释块——任务所要求的蓝图引用
    /// （"顶部注释必须标注蓝本路径 + 行号"）。
    fn wgsl_header() -> String {
        wgsl()
            .lines()
            .take_while(|line| line.starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn frozen_sun() -> DVec3 {
        compute_sun_direction_eci(frozen_julian_date())
    }

    fn vec3_of(v: DVec3) -> Vec3 {
        Vec3::new(v.x as f32, v.y as f32, v.z as f32)
    }

    fn dvec3_of(v: Vec3) -> DVec3 {
        DVec3::new(f64::from(v.x), f64::from(v.y), f64::from(v.z))
    }

    fn rel_err(got: f64, expected: f64) -> f64 {
        (got - expected).abs() / expected.abs().max(1.0e-12)
    }

    /// 从领域默认播种的 GPU 参数，太阳钉在冻结的
    /// v2_sky 基线方向。
    fn gpu_params() -> SkyAtmosphereParams {
        let mut params = SkyAtmosphereParams::from_domain(&AtmosphereParameters::default());
        params.sun_direction = vec3_of(frozen_sun());
        params
    }

    // -- 冻结的时间基准 -------------------------------------------------

    #[test]
    fn frozen_sun_direction_is_the_baseline_constant() {
        let sun = frozen_sun();
        println!("FROZEN_SUN_DIRECTION = [{:.9}, {:.9}, {:.9}]", sun.x, sun.y, sun.z);
        for (got, expected) in [sun.x, sun.y, sun.z].iter().zip(FROZEN_SUN_DIRECTION) {
            assert!(
                (got - expected).abs() < 1.0e-8,
                "the frozen sun direction drifted: got [{:.9}, {:.9}, {:.9}], the v2_sky baselines were captured against {FROZEN_SUN_DIRECTION:?}",
                sun.x, sun.y, sun.z
            );
        }
        assert!(
            (sun.length() - 1.0).abs() < 1.0e-12,
            "the sun direction must be a unit vector, got {}",
            sun.length()
        );
    }

    // -- 米 -> render-unit 的红线 ----------------------------------

    /// M5-C 红线：每个米制的*长度*在到达 GPU 前都被
    /// `METERS_PER_RENDER_UNIT` 除，而每个*每米系数*都被它乘。这证明重缩放
    /// 对着色器实际积分的两个量是**惰性的**，正是这一点使 f32 GPU 结果与
    /// f64 领域结果可比，而非仅仅相似。
    #[test]
    fn unit_invariance_optical_depth_and_density() {
        let domain = AtmosphereParameters::default();
        let gpu = SkyAtmosphereParams::from_domain(&domain);
        let mpu = METERS_PER_RENDER_UNIT;
        let atmosphere_height_m = domain.outer_radius - domain.inner_radius;

        // (1) 长度除以 MPU
        for (label, metres, render_units) in [
            ("inner_radius", domain.inner_radius, f64::from(gpu.inner_radius)),
            ("outer_radius", domain.outer_radius, f64::from(gpu.outer_radius)),
            (
                "rayleigh_scale_height",
                domain.rayleigh_scale_height,
                f64::from(gpu.rayleigh_scale_height),
            ),
            (
                "mie_scale_height",
                domain.mie_scale_height,
                f64::from(gpu.mie_scale_height),
            ),
        ] {
            let err = rel_err(render_units * mpu, metres);
            assert!(err < 1.0e-6, "{label}: {render_units} ru * MPU != {metres} m (rel err {err:.3e})");
        }
        // `thickness()` 减去两个精确到两位小数的半径，所以 f32
        // 的相消损失约 4e-6 相对值。那是整个转换中最大的单一舍入，
        // 且仍在下面一致性预算内 25 倍。
        let thickness_err = rel_err(f64::from(gpu.thickness()) * mpu, atmosphere_height_m);
        assert!(
            thickness_err < 1.0e-5,
            "atmosphere thickness: {thickness_err:.3e}"
        );

        // (2) 系数乘以 MPU
        for (label, per_metre, render_units) in [
            ("rayleigh.r", domain.rayleigh_coefficients[0], f64::from(gpu.rayleigh_coefficient.x)),
            ("rayleigh.g", domain.rayleigh_coefficients[1], f64::from(gpu.rayleigh_coefficient.y)),
            ("rayleigh.b", domain.rayleigh_coefficients[2], f64::from(gpu.rayleigh_coefficient.z)),
            ("mie", domain.mie_coefficient, f64::from(gpu.mie_coefficient)),
        ] {
            let err = rel_err(render_units, per_metre * mpu);
            assert!(err < 1.0e-6, "{label}: {render_units} != {per_metre}/m * MPU (rel err {err:.3e})");
        }

        // (3) 真正重要的不变量：光学深度 `beta * L` 与密度 `exp(-h/H)` 都是比值，
        //     所以它们完全不动。
        let mut worst_optical_depth = 0.0_f64;
        for (label, beta_per_metre, beta_render_unit) in [
            ("rayleigh.r", domain.rayleigh_coefficients[0], gpu.rayleigh_coefficient.x),
            ("rayleigh.g", domain.rayleigh_coefficients[1], gpu.rayleigh_coefficient.y),
            ("rayleigh.b", domain.rayleigh_coefficients[2], gpu.rayleigh_coefficient.z),
            ("mie", domain.mie_coefficient, gpu.mie_coefficient),
        ] {
            let od_metres = beta_per_metre * atmosphere_height_m;
            let od_render_units = f64::from(beta_render_unit) * f64::from(gpu.thickness());
            let err = rel_err(od_render_units, od_metres);
            assert!(
                err < 1.0e-5,
                "optical depth for {label} is not unit-invariant: {od_metres:.6} m vs {od_render_units:.6} ru (rel err {err:.3e})"
            );
            worst_optical_depth = worst_optical_depth.max(err);
        }

        let mut worst_density = 0.0_f64;
        for (scale_metres, scale_render_units) in [
            (domain.rayleigh_scale_height, f64::from(gpu.rayleigh_scale_height)),
            (domain.mie_scale_height, f64::from(gpu.mie_scale_height)),
        ] {
            for height_metres in [0.0_f64, 1_000.0, 8_000.0, 50_000.0, 100_000.0] {
                let density_metres = (-height_metres / scale_metres).exp();
                let density_render_units = (-(height_metres / mpu) / scale_render_units).exp();
                // 指数把尺度高度 f32 舍入放大 |h/H| 倍，
                // 故用 1e-4 而非 1e-6
                let err =
                    (density_render_units - density_metres).abs() / density_metres.max(f64::MIN_POSITIVE);
                assert!(
                    err < 1.0e-4,
                    "density at {height_metres} m / H={scale_metres} m is not unit-invariant (rel err {err:.3e})"
                );
                worst_density = worst_density.max(err);
            }
        }
        println!(
            "unit invariance: worst optical-depth rel err {worst_optical_depth:.3e}, worst density rel err {worst_density:.3e}"
        );
    }

    /// `from_domain` 逐字段执行转换，并让无量纲量原样不动。
    #[test]
    fn from_domain_converts_metres_to_render_units() {
        let domain = AtmosphereParameters::default();
        let gpu = SkyAtmosphereParams::from_domain(&domain);

        // 这条红线，作为一个*推导*而非十进制字面量陈述：
        // 每个米值字段都必须以被 METERS_PER_RENDER_UNIT 除后到达。
        // `MPU` 在此重述而非从 `crate::resources` 读取，所以这个常数
        // 本身也被此测试钉住，而 `rel_err` 给这个比较恰好所需的 f32
        // 预算（1 ulp = 6e-8 相对值）——一个手工舍入的十进制字面量比它所比较的 f32 更粗。
        const MPU: f64 = 6_378_137.0;
        assert_eq!(domain.inner_radius, 6_378_137.0, "WGS84 semi-major axis");
        assert_eq!(domain.outer_radius, 6_478_137.0, "inner + ATMOSPHERE_HEIGHT = 100 km");
        assert_eq!(domain.rayleigh_scale_height, 8_000.0);
        assert_eq!(domain.mie_scale_height, 1_200.0);
        assert_eq!(gpu.inner_radius, 1.0, "the Earth radius is the render unit by definition");
        assert!(
            rel_err(f64::from(gpu.outer_radius), domain.outer_radius / MPU) < 1.0e-7,
            "{}",
            gpu.outer_radius
        );
        assert!(
            rel_err(f64::from(gpu.rayleigh_scale_height), domain.rayleigh_scale_height / MPU) < 1.0e-7,
            "{}",
            gpu.rayleigh_scale_height
        );
        assert!(
            rel_err(f64::from(gpu.mie_scale_height), domain.mie_scale_height / MPU) < 1.0e-7,
            "{}",
            gpu.mie_scale_height
        );
        assert!((gpu.rayleigh_coefficient.x - 36.9932).abs() < 1.0e-3, "{}", gpu.rayleigh_coefficient.x);
        assert!((gpu.rayleigh_coefficient.y - 86.1048).abs() < 1.0e-3, "{}", gpu.rayleigh_coefficient.y);
        assert!((gpu.rayleigh_coefficient.z - 211.1165).abs() < 1.0e-2, "{}", gpu.rayleigh_coefficient.z);
        assert!((gpu.mie_coefficient - 133.9409).abs() < 1.0e-2, "{}", gpu.mie_coefficient);
        assert!((gpu.thickness() - 0.015_678_6).abs() < 1.0e-6, "{}", gpu.thickness());

        // 无量纲：原样传递
        assert_eq!(gpu.mie_anisotropy, domain.mie_anisotropy as f32);
        assert_eq!(gpu.solar_intensity, domain.solar_intensity as f32);
        // 领域 Mie 归一化胜出（D6），且生产为 mode 0
        assert_eq!(gpu.mie_phase_k, MIE_PHASE_K_DOMAIN);
        assert_eq!(gpu.mode, MODE_RAYMARCH);
        assert_eq!(gpu.primary_steps_max, PRIMARY_STEPS_MAX);
        assert_eq!(gpu.light_steps_max, LIGHT_STEPS_MAX);
        // 太阳不是 `from_domain` 的职责，但播种值绝不能退化：它在
        // `ray_sphere_interval_f32` 内被作除数（WGSL D9），而 `Vec3::X` 正是
        // `LightingParams::default()` 所携带的，所以一个在 `sky_system` 刷新 uniform 前
        // spawn 的 dome 会着色一片真实天空而非一片 NaN 天空。
        assert_eq!(gpu.sun_direction, Vec3::X);
        assert!(
            gpu.sun_direction.length_squared() >= DEGENERATE_DIRECTION_EPSILON,
            "the seed must never trip the D9 degenerate-direction guard"
        );
        // padding stays zero so the 80-byte upload is deterministic
        assert_eq!((gpu.pad0, gpu.pad1, gpu.pad2), (0.0, 0, 0));
    }

    // -- CPU 参考一致性门控 ---------------------------------------

    /// 验收门控的“CPU 参考 vs GPU”比较，取无头下可达的最紧
    /// 分辨率。
    ///
    /// `sky_atmosphere.wgsl::closed_form_radiance`（mode 1）是对
    /// [`closed_form_sky_color_f32`] 的逐运算转写，而后者又是领域 f64
    /// [`compute_sky_color`] 在海平面的逐运算转写。因此这约束了*整条*
    /// f64 领域 → f32 render-unit → WGSL 链：GPU 相对 CPU 参考所做的
    /// 一切差异，都只是上面 unit-invariance 测试已证明为单位中性的量的
    /// f32 舍入。
    ///
    /// 字面的设备回读另需一块 GPU；那一半委派给 xvfb e2e runner
    /// （`.github/workflows/cesiumrust-e2e.yml`，M11.2）—— 见 `docs/deferred.md`。
    #[test]
    fn closed_form_f32_mirror_matches_f64_domain() {
        let domain = AtmosphereParameters::default();
        let gpu = gpu_params();
        let sun = frozen_sun();
        // 一个以太阳为第一轴的标准正交基，使扫描恰好
        // 命中每个散射角一次
        let helper = if sun.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
        let u = sun.cross(helper).normalize();

        let mut worst = 0.0_f64;
        let mut worst_degree = 0.0_f64;
        for degree in 0..=360 {
            let theta = f64::from(degree).to_radians();
            let view = (sun * theta.cos() + u * theta.sin()).normalize();
            let expected = compute_sky_color(view, sun, 0.0, &domain);
            let got = closed_form_sky_color_f32(vec3_of(view), &gpu);
            for (e, g) in expected.iter().zip(got.iter()) {
                let err = rel_err(f64::from(*g), *e);
                if err > worst {
                    worst = err;
                    worst_degree = f64::from(degree);
                }
            }
        }
        println!("closed-form parity vs domain compute_sky_color: worst rel err {worst:.3e} at theta = {worst_degree} deg");
        assert!(
            worst < 1.0e-4,
            "the f32 render-unit mirror diverges from the f64 domain reference by {worst:.3e} (worst at theta = {worst_degree} deg)"
        );
    }

    /// D6：领域用 `1/(4*pi)` 归一化 Henyey-Greenstein 相位，
    /// 蓝图（`computeAtmosphereColor` L35）用 `3/(8*pi)`。形状
    /// 相同，恰好相差 1.5 倍；着色器两者都暴露并默认取
    /// 领域值。
    #[test]
    fn mie_phase_domain_and_blueprint_differ_only_by_the_normalisation() {
        let ratio = f64::from(MIE_PHASE_K_BLUEPRINT) / f64::from(MIE_PHASE_K_DOMAIN);
        assert!((ratio - 1.5).abs() < 1.0e-6, "3/(8*pi) must be 1.5 * 1/(4*pi), got {ratio}");

        let domain = AtmosphereParameters::default();
        let mut worst = 0.0_f64;
        for degree in 0..=360 {
            let cos_theta = f64::from(degree).to_radians().cos();
            let expected = mie_phase_domain(cos_theta, domain.mie_anisotropy);
            let got = f64::from(mie_phase_f32(
                cos_theta as f32,
                domain.mie_anisotropy as f32,
                MIE_PHASE_K_DOMAIN,
            ));
            let err = rel_err(got, expected);
            assert!(err < 1.0e-5, "mie_phase at cos={cos_theta} diverges by {err:.3e}");
            worst = worst.max(err);
        }
        println!("mie_phase f32-mirror vs f64 domain: worst rel err {worst:.3e}");

        // 而 Rayleigh 相位与领域*以及*蓝图完全
        // 一致（scattering.rs L82 == computeAtmosphereColor 着色器 L33），逐位相同
        for degree in 0..=180 {
            let cos_theta = f64::from(degree).to_radians().cos();
            let expected = cesium_atmosphere::scattering::rayleigh_phase(cos_theta);
            let got = f64::from(rayleigh_phase_f32(cos_theta as f32));
            assert!(rel_err(got, expected) < 1.0e-6, "rayleigh_phase at cos={cos_theta}");
        }
    }

    /// B2（`approximateTanh` L7-10）：奇函数、饱和到 `[-1, 1]`，且
    /// 跟随 `tanh` 达约 2e-2 —— 这些都是 B1 对它的全部需求，因为它只把
    /// 它当作一个 0..1 权重使用。
    #[test]
    fn approximate_tanh_is_odd_bounded_and_close_to_tanh() {
        assert_eq!(approximate_tanh_f32(0.0), 0.0);
        for x in [-1.0e6_f32, -8.0, -2.0, -1.0, -0.5, -1.0e-6, 1.0e-6, 0.5, 1.0, 2.0, 8.0, 1.0e6] {
            let y = approximate_tanh_f32(x);
            assert!((-1.0..=1.0).contains(&y), "approximate_tanh({x}) = {y} escaped [-1, 1]");
            assert!(
                (y + approximate_tanh_f32(-x)).abs() < 1.0e-6,
                "approximate_tanh must be odd, got {y} at {x}"
            );
            assert!(
                (y - x.tanh()).abs() < 3.0e-2,
                "approximate_tanh({x}) = {y} vs tanh = {}",
                x.tanh()
            );
        }
        // 因此它所喂给的 D1 权重是一条以地平线为中心的单调 0..1 sigmoid，
        // 其 10/90 交点位于地平线两侧各几度处
        assert!(horizon_split_weight_f32(-1.0) < 0.01);
        assert!(horizon_split_weight_f32(-0.12) < 0.15, "looking 7 deg below the horizon");
        assert!((horizon_split_weight_f32(0.0) - 0.5).abs() < 1.0e-6);
        assert!(horizon_split_weight_f32(0.12) > 0.85, "looking 7 deg above the horizon");
        assert!(horizon_split_weight_f32(1.0) > 0.99);
        // 而 `sin_elevation_at` 确实是朝当地天顶的投影
        let up = vec3_of(frozen_sun());
        assert!((sin_elevation_at_f32(up * 2.0, up) - 1.0).abs() < 1.0e-6);
        assert!((sin_elevation_at_f32(up * 2.0, -up) + 1.0).abs() < 1.0e-6);
        assert!(sin_elevation_at_f32(Vec3::ZERO, up).abs() < 1.0e-9, "a degenerate origin has no zenith");
    }

    /// B4（`raySphereIntersectionInterval` L1-37），以原点为中心：着色器
    /// 分支所依据的四种几何情形。
    #[test]
    fn ray_sphere_interval_covers_every_geometric_case() {
        let gpu = gpu_params();
        let outer = gpu.outer_radius;

        // (a) 真正未命中（判别式为负）-> 空区间哨兵
        let (start, stop) = ray_sphere_interval_f32(Vec3::new(0.0, 0.0, 3.0), Vec3::X, outer);
        assert_eq!((start, stop), (1.0, -1.0), "a tangent-outside ray must report EMPTY_INTERVAL");

        // (b) 在外部、朝中心 -> 弦 [d-r, d+r]
        let (start, stop) = ray_sphere_interval_f32(Vec3::new(0.0, 0.0, 3.0), Vec3::NEG_Z, outer);
        assert!((start - (3.0 - outer)).abs() < 1.0e-5, "entry {start}");
        assert!((stop - (3.0 + outer)).abs() < 1.0e-5, "exit {stop}");

        // (c) 在内部 -> start < 0 < stop，横跨整条弦
        let (start, stop) = ray_sphere_interval_f32(Vec3::new(0.0, 0.0, 0.5), Vec3::Z, outer);
        assert!(start < 0.0 && stop > 0.0, "an interior origin must straddle zero: ({start}, {stop})");
        assert!((stop - start - 2.0 * outer).abs() < 1.0e-4, "chord {start}..{stop}");

        // (d) 恰好相切 -> 一个退化区间，调用方将其读作未命中
        let (start, stop) = ray_sphere_interval_f32(Vec3::new(0.0, 0.0, outer), Vec3::X, outer);
        assert!(stop - start < 1.0e-6, "a tangent ray must have a zero-length interval, got ({start}, {stop})");
    }

    /// Ryan H1 / WGSL `D9`。[`ray_sphere_interval_f32`] 除以
    /// `2 * dot(direction, direction)`，所以一个零长度方向会使其成为
    /// `0/0`。此处逐字转写了未加守卫的形式，以证明该 NaN 是真实而非理
    /// 论上的，而交付的函数必须改为返回有限的未命中哨兵。
    ///
    /// 为何这里的 NaN 是灾难性而非表面性的：光线区间喂给
    /// `max(interval.stop, 0.0)`，而 `max`（WGSL 与 glam 皆然）除非 `e1 < e2`
    /// 否则返回其**第一个**参数 —— `NaN < 0.0` 为假，所以
    /// 该 NaN 直接穿过步长、累加器与
    /// `radiance`。在 `AlphaMode::Premultiplied` 下，一个 NaN alpha 会涂抹到整片天空，
    /// 而 bloom / FXAA 的邻域采样随后会把它带进
    /// 那些从未看过这条退化射线的像素。
    #[test]
    fn a_degenerate_direction_returns_the_empty_interval_instead_of_nan() {
        let outer = gpu_params().outer_radius;

        /// 蓝图 B4（`raySphereIntersectionInterval`）原样写就，
        /// 即本函数在 `D9` 守卫之前是什么。
        fn unguarded(origin: Vec3, direction: Vec3, radius: f32) -> (f32, f32) {
            let oc = origin;
            let a = direction.dot(direction);
            let b = 2.0 * direction.dot(oc);
            let c = oc.dot(oc) - radius * radius;
            let det = b * b - 4.0 * a * c;
            if det < 0.0 {
                return (1.0, -1.0);
            }
            let sqrt_det = det.sqrt();
            let two_a = 2.0 * a;
            ((-b - sqrt_det) / two_a, (-b + sqrt_det) / two_a)
        }

        // 光线被投射的每一个原点：壳层内（所有
        // march 采样点）与壳层外。对它们全部都有 `a = b = det = 0`，所以
        // `det < 0.0` 的提前返回从不触发，除法得以到达。
        for origin in [
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, 1.001),
            Vec3::new(0.0, 0.0, 3.0),
            Vec3::X * (outer + 0.5),
            -Vec3::Y * (outer * 2.0),
        ] {
            let (before_start, before_stop) = unguarded(origin, Vec3::ZERO, outer);
            assert!(
                before_start.is_nan() && before_stop.is_nan(),
                "the un-guarded form must produce the NaN `D9` describes, got ({before_start}, {before_stop})"
            );

            let (start, stop) = ray_sphere_interval_f32(origin, Vec3::ZERO, outer);
            assert!(
                start.is_finite() && stop.is_finite(),
                "H1 regression: NaN escaped the guard for origin {origin:?}, got ({start}, {stop})"
            );
            assert_eq!(
                (start, stop),
                (1.0, -1.0),
                "a degenerate direction has no interval to report, so the caller must read it as a miss"
            );
        }

        // 守卫不得吞掉任何*非*退化方向，无论其多短：
        // `a` 与 1e-12 比较，即 |direction| < 1e-6，而两个
        // 生产方向都被归一化为 |d| = 1。
        for scale in [1.0e-3_f32, 1.0e-4, 1.0e-5, 1.0] {
            let direction = Vec3::NEG_Z * scale;
            assert!(
                direction.dot(direction) >= DEGENERATE_DIRECTION_EPSILON,
                "scale {scale} must stay above the guard"
            );
            let (start, stop) = ray_sphere_interval_f32(Vec3::new(0.0, 0.0, 3.0), direction, outer);
            assert!(start.is_finite() && stop.is_finite(), "scale {scale}: ({start}, {stop})");
            assert!(stop > start, "a merely short direction is still a real ray: ({start}, {stop})");
        }
    }

    /// Ryan H1 端到端贯穿 mode-0 管线：uniform 中
    /// `sun_direction == Vec3::ZERO` —— 这是 [`SkyAtmosphereParams::from_domain`]
    /// 过去用于播种的值，也是若 `LightingParams::sun_direction` 退化时
    /// [`update_sun_direction`] 仍会写入的值（它用
    /// `normalize_or_zero()` 归一化）—— 每个累加器都必须保持有限。
    #[test]
    fn a_zero_sun_direction_leaves_the_whole_march_finite() {
        let params = SkyAtmosphereParams {
            sun_direction: Vec3::ZERO,
            ..gpu_params()
        };
        let origin = Vec3::new(0.0, 0.0, params.inner_radius + 0.001);
        let direction = Vec3::Y;

        // 主方向是一个真正的单位向量，所以命中壳层且
        // march 运行；只有*光线*射线是退化的
        let march = march_single_scattering_f32(origin, direction, SKY_DOME_RADIUS, &params)
            .expect("the primary ray must still intersect the shell");
        for value in [
            march.optical_depth.x,
            march.optical_depth.y,
            march.rayleigh_accumulation.x,
            march.rayleigh_accumulation.y,
            march.rayleigh_accumulation.z,
            march.mie_accumulation.x,
            march.mie_accumulation.y,
            march.mie_accumulation.z,
        ] {
            assert!(value.is_finite(), "H1 regression: NaN escaped the march accumulators");
        }

        let shading = shade_sky_f32(origin, direction, SKY_DOME_RADIUS, &params, 1.0)
            .expect("as above");
        assert!(shading.radiance.is_finite(), "H1 regression: {shading:?}");
        assert!(shading.transmittance.is_finite(), "H1 regression: {shading:?}");
        assert!(
            shading.alpha.is_finite() && (0.0..=1.0).contains(&shading.alpha),
            "a NaN alpha is what poisons the premultiplied blend: {shading:?}"
        );

        // 光线报告一次未命中，所以其光学深度不贡献任何东西，
        // 而退化情形降级为“未计算阴影”—— 可见地错误、有限，
        // 且在下一帧可恢复。这正是偏爱哨兵而非 NaN 的全部意义。
        let healthy = shade_sky_f32(origin, direction, SKY_DOME_RADIUS, &gpu_params(), 1.0)
            .expect("as above");
        println!(
            "zero-sun radiance {:?} alpha {:.6} | healthy radiance {:?} alpha {:.6}",
            shading.radiance, shading.alpha, healthy.radiance, healthy.alpha
        );
    }

    // -- 着色器源码契约 -------------------------------------------

    /// 本任务的硬性要求（plan L152 + 风险表 L347）：着色器
    /// 头必须以**路径和行号**引用蓝图、它所对照验证的 CPU
    /// 参考，以及每一处刻意的偏差。
    #[test]
    fn wgsl_header_cites_blueprint_paths_and_line_numbers() {
        let header = wgsl_header();
        assert!(
            header.lines().count() > 100,
            "the citation block must be the shader's leading comment, got {} lines",
            header.lines().count()
        );

        for path in [
            "cesium-rs/crates/cesium-shaders/shaders/Builtin/Functions/computeScattering.glsl",
            "packages/engine/Source/Shaders/Builtin/Functions/computeScattering.glsl",
            "approximateTanh.glsl",
            "computeGroundAtmosphereScattering.glsl",
            "raySphereIntersectionInterval.glsl",
            "computeAtmosphereColor.glsl",
            "SkyAtmosphereFS.glsl",
            "domain/atmosphere/src/scattering.rs",
        ] {
            assert!(header.contains(path), "the header must cite the blueprint path `{path}`");
        }
        // 精确到行，而非仅精确到文件
        for line_ref in [
            "L16-24", "L25", "L26-27", "L34", "L39-44", "L50", "L53", "L64", "L65", "L66-67",
            "L83-89", "L98-99", "L102-105", "L133", "L136-137", "L144-145", "L148",
            "L43-73", "L81-83", "L90-95", "L102-104", "L118-146",
        ] {
            assert!(header.contains(line_ref), "the header must cite blueprint line `{line_ref}`");
        }
        // 每一处刻意偏差都被记录并编号
        for index in 1..=9 {
            let marker = format!("// D{index} ");
            assert!(header.contains(&marker), "deviation D{index} must be documented in the header");
        }
        assert!(header.contains("REWRITE, not a"), "the header must say this is a rewrite, not a transpilation");
        assert!(header.contains("DELIBERATE DEVIATIONS FROM THE BLUEPRINT"));
        // 两条数学红线都写在着色器自身里
        assert!(header.contains("METERS_PER_RENDER_UNIT = 6378137"));
        assert!(header.contains("NO FMA CONTRACTION"));
        assert!(header.contains("docs/deviations.md#dev-021"));
    }

    /// 着色器与 Bevy 的结构性契约，外加 no-FMA 红线。
    #[test]
    fn wgsl_is_a_bevy_material_fragment_with_a_premultiplied_output() {
        let source = wgsl();

        // 恰好是前向 `Material` 片元所需的两个 naga_oil import
        let imports = source.lines().filter(|line| line.starts_with("#import")).collect::<Vec<_>>();
        assert_eq!(
            imports,
            vec![
                "#import bevy_pbr::forward_io::VertexOutput",
                "#import bevy_pbr::mesh_view_bindings"
            ],
            "the imports must match the naga-validation stubs below"
        );
        // Bevy 为 `Material` 上的 `#[uniform(0)]` 预留的 bind group
        assert!(source.contains("@group(2) @binding(0) var<uniform> params: SkyAtmosphereParams;"));
        // 单一入口点，带 `Material` 期望的签名
        assert!(source.contains("@fragment\nfn fragment(in: VertexOutput) -> @location(0) vec4<f32> {"));
        // Bevy 0.15 的 `View` 字段是 `world_position`；pre-0.15 的
        // `view_world_position` 会静默读到垃圾值。
        assert!(source.contains("view.world_position"));
        assert!(!source.contains("view_world_position"));
        assert!(source.contains("view.exposure"));
        // 禁用 FMA 收缩：`a*b + c` 必须保留它的两次舍入
        assert!(!source.contains("fma("), "the shader must not contract a*b+c into an FMA");
        // 两种模式都存在，且未命中路径完全透明
        assert!(source.contains("if (params.mode == MODE_CLOSED_FORM) {"));
        assert_eq!(
            source.matches("return vec4<f32>(0.0, 0.0, 0.0, 0.0);").count(),
            2,
            "the degenerate-ray and the missed-atmosphere paths must both emit a transparent fragment"
        );
        // 预乘合成
        assert!(source.contains("return vec4<f32>(radiance, alpha);"));
        assert!(source.contains("return vec4<f32>(closed_form_radiance(ray_direction), 1.0);"));
        // D7：两处密度求值都在海平面处向下取整
        assert!(source.contains("let sample_height = max(sample_radius - params.inner_radius, 0.0);"));
        assert!(source.contains("let light_height = max(light_radius - params.inner_radius, 0.0);"));
        // D9 (Ryan H1)：光线方向是被除数，所以退化者
        // 必须在 `2*a` 归零之前被拒绝
        assert!(
            source.contains("const DEGENERATE_DIRECTION_EPSILON: f32 = 1.0e-12;"),
            "the guard threshold must be a named const, bit-equal to the Rust mirror's"
        );
        assert!(source.contains("if (a < DEGENERATE_DIRECTION_EPSILON) {"));
        assert!(
            source.contains("let a = dot(direction, direction);"),
            "the guard must sit on the same `a` that becomes `two_a`"
        );
    }

    /// 两个 `#import` 的桩。naga 无预处理器，所以它们被
    /// 替换为恰好是着色器所读绑定的声明：
    /// `VertexOutput.world_position`（forward_io）与 `view.world_position` /
    /// `view.exposure`（mesh_view_bindings）。其余全是真实的着色器
    /// 文本，因此这验证了实际的散射代码。
    const WGSL_IMPORT_STUBS: &str = "\
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
}

struct View {
    world_position: vec3<f32>,
    exposure: f32,
}

@group(0) @binding(0) var<uniform> view: View;
";

    fn stubbed_wgsl() -> String {
        let mut source = String::from(WGSL_IMPORT_STUBS);
        for line in wgsl().lines() {
            if line.starts_with("#import") {
                continue;
            }
            source.push_str(line);
            source.push('\n');
        }
        source
    }

    /// 证明着色器为真的最有力无头证据：它被 **naga** 解析并
    /// 类型检查，而 naga 正是 `bevy_render` 在 GPU 路径上编译它所用的
    /// 同一个 WGSL 前端（naga 23.1，已作为
    /// 传递依赖在 `Cargo.lock` 中）。一次字面的设备回读仍需要 xvfb ——
    /// `.github/workflows/cesiumrust-e2e.yml`（M11.2）。
    #[test]
    fn sky_atmosphere_wgsl_parses_and_type_checks_under_naga() {
        let source = stubbed_wgsl();
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|error| {
            panic!("sky_atmosphere.wgsl does not parse:\n{}", error.emit_to_string(&source))
        });
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("sky_atmosphere.wgsl does not validate");

        let entry_points = module
            .entry_points
            .iter()
            .map(|entry| (entry.name.as_str(), entry.stage))
            .collect::<Vec<_>>();
        assert_eq!(
            entry_points,
            vec![("fragment", naga::ShaderStage::Fragment)],
            "the shader must expose exactly one fragment entry point"
        );
    }

    /// uniform 块是承重的：`encase` 按位置写入
    /// [`SkyAtmosphereParams`]，所以 Rust 与 WGSL 必须逐字段、
    /// 逐偏移达成一致。
    #[test]
    fn wgsl_uniform_block_matches_the_rust_struct_field_by_field() {
        let rust_fields = [
            "sun_direction", "inner_radius", "rayleigh_scale_height", "mie_scale_height",
            "outer_radius", "pad0", "rayleigh_coefficient", "mie_coefficient",
            "mie_anisotropy", "solar_intensity", "mie_phase_k", "mode",
            "primary_steps_max", "light_steps_max", "pad1", "pad2",
        ];
        let offsets = [0u32, 12, 16, 20, 24, 28, 32, 44, 48, 52, 56, 60, 64, 68, 72, 76];

        // (1) 解析 WGSL 声明及其注释的 std140 偏移
        let body = wgsl()
            .split("struct SkyAtmosphereParams {")
            .nth(1)
            .expect("sky_atmosphere.wgsl must declare struct SkyAtmosphereParams")
            .split("};")
            .next()
            .expect("the uniform struct must be closed");
        let mut parsed: Vec<(String, u32)> = Vec::new();
        for line in body.lines() {
            let (declaration, comment) = line.split_once("//").unwrap_or((line, ""));
            let declaration = declaration.trim();
            if declaration.is_empty() {
                continue;
            }
            let name = declaration.split(':').next().unwrap_or_default().trim();
            let offset = comment.trim().split(':').next().unwrap_or_default().trim();
            parsed.push((
                name.to_string(),
                offset.parse::<u32>().unwrap_or_else(|error| {
                    panic!("uniform field `{name}` carries no std140 offset comment (`{offset}`): {error}")
                }),
            ));
        }
        assert_eq!(parsed.len(), rust_fields.len(), "uniform field count: {parsed:?}");
        for (index, (name, offset)) in parsed.iter().enumerate() {
            // WGSL 的 pad 带 `_` 前缀，Rust 的不带
            assert_eq!(
                name.trim_start_matches('_'),
                rust_fields[index],
                "uniform field order/name mismatch at {index}: {parsed:?}"
            );
            assert_eq!(*offset, offsets[index], "std140 offset mismatch for `{}`", rust_fields[index]);
        }

        // (2) 从字段类型重新计算那些偏移，使上面的表
        //     不会静默地偏离 std140 规则
        let sizes_aligns = [
            (12u32, 16u32), // vec3<f32>
            (4, 4), (4, 4), (4, 4), (4, 4), (4, 4),
            (12, 16), // vec3<f32>
            (4, 4), (4, 4), (4, 4), (4, 4), (4, 4), (4, 4), (4, 4), (4, 4), (4, 4),
        ];
        let mut cursor = 0u32;
        let mut computed = Vec::with_capacity(sizes_aligns.len());
        for (size, align) in sizes_aligns {
            cursor = cursor.div_ceil(align) * align;
            computed.push(cursor);
            cursor += size;
        }
        assert_eq!(computed.as_slice(), &offsets[..], "std140 offsets recomputed from the field types disagree");
        assert_eq!(cursor.div_ceil(16) * 16, 80, "the uniform block must be 80 bytes");
        assert!(
            wgsl().contains("// size 80, align 16"),
            "the WGSL block must document its 80-byte / 16-aligned size"
        );

        // (3) 并让 `encase` 自己定夺 —— 这才是 GPU
        //     实际收到的布局
        assert_eq!(
            ShaderType::size(&gpu_params()).get(),
            80,
            "encase's std140 size for SkyAtmosphereParams must equal the WGSL block"
        );
    }

    /// WGSL 的 `const` 必须与它们所镜像的 Rust `const` **逐位相等**，
    /// 否则上面的 CPU 参考测试就会在测试与着色器所运行的
    /// 不同的数学。
    #[test]
    fn wgsl_constants_are_bit_equal_to_the_rust_constants() {
        fn const_literal(line_prefix: &str) -> String {
            let line = wgsl()
                .lines()
                .find(|line| line.starts_with(line_prefix))
                .unwrap_or_else(|| panic!("sky_atmosphere.wgsl has no `{line_prefix}`"));
            line.split('=')
                .nth(1)
                .unwrap_or_default()
                .trim()
                .trim_end_matches(';')
                .to_string()
        }

        for (name, rust_value) in [
            ("RAYLEIGH_PHASE_K", RAYLEIGH_PHASE_K),
            ("MIE_PHASE_K_BLUEPRINT", MIE_PHASE_K_BLUEPRINT),
            ("HORIZON_SPLIT_SHARPNESS", HORIZON_SPLIT_SHARPNESS),
            ("DEGENERATE_DIRECTION_EPSILON", DEGENERATE_DIRECTION_EPSILON),
        ] {
            let literal = const_literal(&format!("const {name}: f32 ="));
            let wgsl_value: f32 = literal
                .parse()
                .unwrap_or_else(|error| panic!("cannot parse `{name} = {literal}`: {error}"));
            assert_eq!(
                wgsl_value.to_bits(),
                rust_value.to_bits(),
                "{name} is not bit-equal between WGSL ({literal}) and Rust"
            );
        }
        for (name, rust_value) in [("MODE_RAYMARCH", MODE_RAYMARCH), ("MODE_CLOSED_FORM", MODE_CLOSED_FORM)] {
            let literal = const_literal(&format!("const {name}: u32 ="));
            let wgsl_value: u32 = literal
                .trim_end_matches('u')
                .parse()
                .unwrap_or_else(|error| panic!("cannot parse `{name} = {literal}`: {error}"));
            assert_eq!(wgsl_value, rust_value, "{name} differs between WGSL and Rust");
        }
        // 领域的 Mie 归一化是*默认 uniform 值*，所以
        // WGSL 只在文字中提及它 —— 但它必须提及
        assert!(
            wgsl().contains("0.07957747154594767"),
            "the WGSL must document the domain Mie normalisation it defaults to"
        );
        assert_eq!(MIE_PHASE_K_DOMAIN.to_bits(), ((1.0_f64 / (4.0 * std::f64::consts::PI)) as f32).to_bits());
    }

    /// Ryan M2。`scattered * solar * exposure` 按
    /// `(scattered*solar)*exposure` 结合；Rust 镜像先绑定 `gain = solar*exposure`。
    /// f32 乘法可交换但不可结合，所以两者
    /// 可能不同。这把 WGSL 钉在镜像的顺序上——于是未来一次
    /// 在 **mode 0** 上的 GPU-vs-CPU 一致性探针（它确实携带 `view.exposure`，
    /// 不同于 mode-1 闭式探针）便不会产生一个无法解释的
    /// 末位不匹配——并度量该差异究竟有多大。
    #[test]
    fn the_gain_association_is_pinned_to_the_rust_mirror() {
        // (1) 源码契约
        let source = wgsl();
        assert!(
            source.contains("let gain = params.solar_intensity * view.exposure;"),
            "the fragment tail must bind the scalar gain exactly as shade_sky_f32 does"
        );
        assert!(source.contains("let radiance = scattered * gain;"));
        assert!(
            !source.contains("scattered * params.solar_intensity * view.exposure"),
            "the un-associated form must not come back: it is (scattered*solar)*exposure, \
             a different f32 rounding from the Rust mirror's scattered*(solar*exposure)"
        );
        assert!(
            wgsl_header().contains("not associative"),
            "the header op-for-op rule must document the association clause"
        );

        // (2) 量级：确定性 xorshift64 扫描（无 RNG，所以见证集
        //     在每次运行与每个平台上都相同），覆盖跨越 2^-7 .. 2^5 的
        //     规格化正 f32 三元组。
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut xorshift = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let normal_f32 = |bits: u64| {
            let exponent = (bits % 13) << 23; // 偏移指数字段 120..=132
            let mantissa = (bits >> 8) & 0x007F_FFFF;
            f32::from_bits(0x3C00_0000 + exponent as u32 + mantissa as u32)
        };
        let mut differing = 0_u64;
        let mut worst_ulps = 0_i64;
        let mut worst_rel = 0.0_f64;
        for _ in 0..200_000 {
            let scattered = normal_f32(xorshift());
            let solar = normal_f32(xorshift());
            let exposure = normal_f32(xorshift());
            let left_to_right = (scattered * solar) * exposure; // WGSL 过去所写的形式
            let gain_first = scattered * (solar * exposure); // shade_sky_f32 所写的形式
            if left_to_right.to_bits() != gain_first.to_bits() {
                differing += 1;
                worst_ulps = worst_ulps.max(
                    i64::from(left_to_right.to_bits() as i32 - gain_first.to_bits() as i32).abs(),
                );
                let rel = (f64::from(left_to_right) - f64::from(gain_first)).abs()
                    / f64::from(gain_first);
                worst_rel = worst_rel.max(rel);
            }
        }
        println!(
            "gain re-association: {differing}/200000 triples differ, worst {worst_ulps} ULP, worst rel {worst_rel:.3e}"
        );
        assert!(differing > 0, "the scan must actually find witnesses, or it proves nothing");
        // 两次额外舍入，所以严格界限是 2*f32::EPSILON 相对值
        // (= 2^-22)，在最小相对 ulp (2^-24) 处即 **4 ULP** --
        // 而非评审注记估计的 1 ULP。带余量断言。
        assert!(
            worst_rel <= 3.0 * f64::from(f32::EPSILON),
            "re-association must cost at most two roundings, got rel {worst_rel:.3e}"
        );
        assert!(worst_ulps <= 4, "worst {worst_ulps} ULP exceeds the two-rounding bound");
    }

    // -- 几何 / 材质 / 门控 -------------------------------------------

    /// *被 spawn 的 mesh* 实际携带的半径，从其顶点
    /// 缓冲回读。
    ///
    /// 经由 mesh 而非把 [`SKY_DOME_RADIUS`] 与相邻常量比较，重要在两方
    /// 面：一次 const-对-const 比较会在编译期折叠（所以它在运行期什么都不断言
    /// 且触发 `clippy::assertions_on_constants`），而只有顶点缓冲才能
    /// 抓到 `build_sky_dome_mesh_asset` 偏离所记录的半径。
    fn dome_mesh_radius() -> f32 {
        let mesh = build_sky_dome_mesh_asset();
        let VertexAttributeValues::Float32x3(positions) = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .expect("the dome mesh must carry positions")
        else {
            panic!("dome positions must be Float32x3");
        };
        positions
            .iter()
            .map(|position| Vec3::from_slice(position).length())
            .fold(0.0_f32, f32::max)
    }

    #[test]
    fn dome_radius_fits_between_the_camera_and_the_starfield() {
        let gpu = gpu_params();
        let mesh_radius = dome_mesh_radius();
        assert!(mesh_radius > ORBIT_MAX_DISTANCE, "the camera must always be inside the dome");
        assert!(mesh_radius < STARFIELD_RADIUS, "the dome must stay inside the starfield so the stars can be extinguished by it");
        assert!(mesh_radius < CAMERA_FAR, "the dome must not be clipped by the far plane");
        assert!(mesh_radius > gpu.outer_radius, "the dome must geometrically enclose the modelled atmosphere");
        // 不是 `assert_eq!`：Bevy 的测地构建器先把每个顶点在 f32 中归一化
        // 再*然后*按半径缩放，所以缓冲会超出约 1 ulp
        // （40.000004 对照所记录的 40.0）。
        assert!(
            (mesh_radius - SKY_DOME_RADIUS).abs() < 1.0e-4,
            "the spawned mesh must carry the documented radius, got {mesh_radius}"
        );
        // `Transparent3d::sort` 对视图空间 Z 是*升序*（bevy_core_pipeline
        // 0.15 `core_3d/mod.rs` L515-517）且相机朝 -Z 看，所以
        // 升序 == 由后到前，而一个*正*偏置意味着“更晚绘制”。
        // 经由 `Material::depth_bias` 读取，使符号检查是关于管线将实际排序
        // 的那个材质的一条运行期事实，而非一个折叠后的 const。
        let bias = SkyDomeMaterial { params: gpu }.depth_bias();
        assert!(bias > 0.0, "the bias must be positive to sort the dome after the starfield");
        assert_eq!(bias, SKY_DOME_DEPTH_BIAS);
        assert_eq!(sky_dome_cull_mode(), Some(Face::Front), "only the far shell may be rasterised");
    }

    #[test]
    fn material_properties_pin_the_render_order() {
        let material = SkyDomeMaterial { params: gpu_params() };
        assert_eq!(
            material.alpha_mode(),
            AlphaMode::Premultiplied,
            "Premultiplied -> depth_write_enabled = false + GreaterEqual, so the opaque globe occludes the dome"
        );
        assert_eq!(material.depth_bias(), SKY_DOME_DEPTH_BIAS);
        match SkyDomeMaterial::fragment_shader() {
            ShaderRef::Handle(handle) => assert_eq!(
                handle, SKY_ATMOSPHERE_SHADER_HANDLE,
                "the dome must use the headless-safe registered shader"
            ),
            ShaderRef::Path(path) => panic!("the dome must not load its shader from a path: {path:?}"),
            // naga_oil `#import` 解析；我们的着色器是自包含的。
            ShaderRef::Default => panic!("the dome must not fall back to the default PBR shader"),
        }
    }

    #[test]
    fn dome_mesh_is_a_closed_shell_at_the_right_radius() {
        let mesh = build_sky_dome_mesh_asset();
        let VertexAttributeValues::Float32x3(positions) = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .expect("the dome mesh must carry positions")
        else {
            panic!("dome positions must be Float32x3");
        };
        // Bevy 的 `ico(s)` 是一次*测地*切分 -- 20 条二十面体棱边
        // 每条被切成 `s + 1` 段 -- 所以它产出 `20 * (s+1)^2`
        // 个三角形，且由欧拉公式（闭合三角壳层 `V = F/2 + 2`），
        // `10 * (s+1)^2 + 2` 个顶点。断言这一推导而非一个
        // 手工计数的字面量，能在常量被重调时保持此处诚实。
        let segments = SKY_DOME_SUBDIVISIONS + 1;
        let expected_vertices = 10 * segments * segments + 2;
        assert_eq!(
            positions.len(),
            expected_vertices as usize,
            "ico({SKY_DOME_SUBDIVISIONS}) must be a closed geodesic shell"
        );
        // 12 度的三角形边，而这就足够了：片元着色器
        // 归一化 `world_position - camera`，而归一化一个弦中点
        // *精确地*复现大圆方向，所以残余的方向误差是边角的二阶量
        // （约 0.008 度，在 1920px / 60 度画面上约四分之一个像素）。
        assert!(
            expected_vertices >= 300,
            "a full-screen dome needs a smooth limb, got {expected_vertices} vertices"
        );
        let mut worst_radius = 0.0_f32;
        for position in positions {
            worst_radius = worst_radius.max((Vec3::from_slice(position).length() - SKY_DOME_RADIUS).abs());
        }
        assert!(
            worst_radius < 1.0e-3,
            "every vertex must sit on the {SKY_DOME_RADIUS} ru sphere, worst off by {worst_radius}"
        );
        // 朝外的法线正是使 `Face::Front` 剔除*近*半球的东西
        let VertexAttributeValues::Float32x3(normals) = mesh
            .attribute(Mesh::ATTRIBUTE_NORMAL)
            .expect("the dome mesh must carry normals")
        else {
            panic!("dome normals must be Float32x3");
        };
        assert_eq!(normals.len(), positions.len());
        let mut worst_dot = 1.0_f32;
        for (position, normal) in positions.iter().zip(normals.iter()) {
            let radial = Vec3::from_slice(position).normalize();
            worst_dot = worst_dot.min(radial.dot(Vec3::from_slice(normal)));
        }
        assert!(worst_dot > 0.999, "normals must be radial, worst dot {worst_dot}");
        assert_eq!(mesh.primitive_topology(), PrimitiveTopology::TriangleList);
        println!(
            "sky dome mesh: {} vertices, worst radius error {worst_radius:.3e}, worst normal dot {worst_dot:.6}",
            positions.len()
        );
    }

    #[test]
    fn gate_parsing_matches_feature_flags_without_touching_the_environment() {
        // `sky_dome_gate_enabled()` 是 `gate_from_env_value(env::var(..))`，而
        // `feature_flags::env_flag`（应用层，适配层无法读取）
        // 接受完全相同的真值集。断言这个*纯*函数
        // 便在不改动并行测试线程所共享的进程环境的前提下同时覆盖两者。
        for (raw, expected) in [
            (None, false),
            (Some(""), false),
            (Some("0"), false),
            (Some("false"), false),
            (Some("no"), false),
            (Some("off"), false),
            (Some("anything else"), false),
            (Some("1"), true),
            (Some("true"), true),
            (Some("yes"), true),
            (Some("on"), true),
            (Some(" TRUE "), true),
            (Some("On"), true),
            (Some("YeS"), true),
        ] {
            assert_eq!(
                gate_from_env_value(raw.map(str::to_string)),
                expected,
                "CESIUM_ENABLE_SKYDOME={raw:?}"
            );
        }
        assert_eq!(ENV_ENABLE_SKYDOME, "CESIUM_ENABLE_SKYDOME");
        // 无论真实变量被设成什么，读取它都不得 panic
        let _ = sky_dome_gate_enabled();
    }

    // -- 生产级 ray-march ---------------------------------------------

    /// 一台位于向阳面、恰在水面之上的相机，沿切向观看
    /// （90 度散射）。
    fn sunlit_surface_setup() -> (SkyAtmosphereParams, Vec3, Vec3) {
        let params = gpu_params();
        let origin = params.sun_direction * (params.inner_radius + 0.001);
        let tangent = params.sun_direction.cross(Vec3::Z);
        let direction = if tangent.length_squared() < 1.0e-8 {
            Vec3::X
        } else {
            tangent.normalize()
        };
        (params, origin, direction)
    }

    /// 在光学薄极限下，通道排序仅由 Rayleigh 系数
    /// 决定（`beta_b : beta_g : beta_r = 33.1 : 13.5 : 5.8`），而
    /// 在真实（厚）极限下，透射率仍按波长排序。
    #[test]
    fn raymarch_orders_the_channels_by_wavelength() {
        let (params, origin, direction) = sunlit_surface_setup();

        // 光学薄、Mie 关闭：双向消光 ~ 1，所以该比值
        // 必须复现系数比值
        let mut thin = params;
        thin.rayleigh_coefficient *= 1.0e-3;
        thin.mie_coefficient = 0.0;
        let thin = shade_sky_f32(origin, direction, SKY_DOME_RADIUS, &thin, 1.0)
            .expect("the thin ray must still intersect the shell");
        assert!(thin.radiance.is_finite());
        assert!(thin.radiance.z > thin.radiance.y, "blue must dominate in the thin limit, got {:?}", thin.radiance);
        assert!(thin.radiance.y > thin.radiance.x, "green must sit between blue and red, got {:?}", thin.radiance);
        let got = f64::from(thin.radiance.z / thin.radiance.x);
        let expected = 33.1 / 5.8;
        assert!(
            rel_err(got, expected) < 2.0e-2,
            "thin-limit blue/red = {got}, expected the coefficient ratio {expected}"
        );

        // 真实系数：消光是波长选择性的，所以蓝光被
        // 消光得最多
        let thick = shade_sky_f32(origin, direction, SKY_DOME_RADIUS, &params, 1.0)
            .expect("the thick ray must still intersect the shell");
        assert!(thick.radiance.is_finite() && thick.transmittance.is_finite());
        assert!(thick.radiance.min_element() >= 0.0, "in-scattered radiance cannot be negative: {:?}", thick.radiance);
        assert!(
            thick.transmittance.x > thick.transmittance.y
                && thick.transmittance.y > thick.transmittance.z,
            "transmittance must fall with wavelength-selective extinction: {:?}",
            thick.transmittance
        );
        for channel in [thick.transmittance.x, thick.transmittance.y, thick.transmittance.z] {
            assert!((0.0..=1.0).contains(&channel), "transmittance must be a fraction, got {channel}");
        }
        assert!((0.0..=1.0).contains(&thick.alpha));
        // alpha 恰为平均透射率的补（D3）
        let mean = thick.transmittance.dot(Vec3::splat(1.0 / 3.0));
        assert!((thick.alpha - (1.0 - mean).clamp(0.0, 1.0)).abs() < 1.0e-6);
        println!(
            "90 deg scattering: thin radiance {:?}, thick radiance {:?}, transmittance {:?}, alpha {:.6}",
            thin.radiance, thick.radiance, thick.transmittance, thick.alpha
        );
    }

    /// D7 回归守卫。一条穿透地球的主射线会把采样点放到
    /// 表面之下，那里 `-h/H` 达到 +797 而
    /// `exp(+797)` 在 f32 中是 `inf`。没有海平面下限，累加
    /// 会变成 `inf * exp(-inf)` = `inf * 0` = **NaN**，这在 GPU 上
    /// 非确定性，且会破坏每一条基线。
    #[test]
    fn raymarch_through_the_earth_stays_finite() {
        let params = gpu_params();
        let origin = params.sun_direction * (params.inner_radius + 0.001);
        let direction = -params.sun_direction; // 笔直向下，穿过行星
        let shading = shade_sky_f32(origin, direction, SKY_DOME_RADIUS, &params, 1.0)
            .expect("the origin is inside the shell, so the ray must intersect it");
        let march = shading.march;
        for value in [
            shading.radiance.x, shading.radiance.y, shading.radiance.z,
            shading.transmittance.x, shading.transmittance.y, shading.transmittance.z,
            shading.alpha, march.optical_depth.x, march.optical_depth.y,
            march.rayleigh_accumulation.x, march.rayleigh_accumulation.z,
            march.mie_accumulation.y,
        ] {
            assert!(value.is_finite(), "D7 regression: NaN/inf escaped the march");
        }
        assert!(march.optical_depth.x > 0.0 && march.optical_depth.y > 0.0);
        assert_eq!(
            shading.transmittance,
            Vec3::ZERO,
            "a ray through the whole atmosphere plus the planet transmits nothing"
        );
        assert!((shading.alpha - 1.0).abs() < 1.0e-6);
        assert!(
            shading.radiance.length() < 1.0e-3,
            "the fully shadowed ray must be black, got {:?}",
            shading.radiance
        );
    }

    /// 夜侧：太阳在当地地平线之下，所以每条光线都
    /// 穿过地球，双向消光饱和到零。
    #[test]
    fn raymarch_on_the_night_side_is_dark_and_the_day_side_is_bright() {
        let params = gpu_params();
        let night_up = -params.sun_direction;
        let night_origin = night_up * (params.inner_radius + 0.001);
        let night = shade_sky_f32(night_origin, night_up, SKY_DOME_RADIUS, &params, 1.0)
            .expect("the night-side zenith ray must still intersect the shell");
        assert!(night.radiance.is_finite(), "got {:?}", night.radiance);
        assert!(
            night.radiance.length() < 1.0e-2,
            "the night sky must be dark, got {:?}",
            night.radiance
        );

        let day_up = params.sun_direction;
        let day_origin = day_up * (params.inner_radius + 0.001);
        let day = shade_sky_f32(day_origin, day_up, SKY_DOME_RADIUS, &params, 1.0)
            .expect("the sub-solar zenith ray must still intersect the shell");
        assert!(day.radiance.is_finite(), "got {:?}", day.radiance);
        assert!(
            day.radiance.length() > night.radiance.length(),
            "the day side must be brighter than the night side: day {:?} vs night {:?}",
            day.radiance,
            night.radiance
        );
        assert!(day.radiance.length() > 1.0e-3, "the sub-solar zenith must be lit, got {:?}", day.radiance);
        println!("day zenith radiance {:?}, night zenith radiance {:?}", day.radiance, night.radiance);
    }

    #[test]
    fn raymarch_rejects_rays_that_never_enter_the_atmosphere() {
        let params = gpu_params();
        // (a) 一次真正的几何未命中
        let outside = Vec3::new(0.0, 0.0, SKY_DOME_RADIUS);
        assert!(march_single_scattering_f32(outside, Vec3::X, SKY_DOME_RADIUS, &params).is_none());
        assert!(shade_sky_f32(outside, Vec3::X, SKY_DOME_RADIUS, &params, 1.0).is_none());
        // (b) 在外部且远离：无限直线仍相交，但
        //     完全在原点之后，所以 `stop = min(interval.y, length)`
        //     将其拒绝（B1 L56-59）
        assert!(march_single_scattering_f32(Vec3::new(0.0, 0.0, 3.0), Vec3::Z, 39.0, &params).is_none());
        // (c) 一条退化的零长度射线
        let inside = params.sun_direction * (params.inner_radius + 0.001);
        assert!(march_single_scattering_f32(inside, params.sun_direction, 0.0, &params).is_none());
    }

    /// 带 D2 改写的 B1 L66-67：步数预算随相机
    /// 沉入大气而收缩，而 `max(1, ..)` 卡口意味着其下方的
    /// 任何除法都永远不会除以零。
    #[test]
    fn raymarch_step_budget_shrinks_as_the_camera_descends() {
        let params = gpu_params();
        let up = params.sun_direction;
        let mut previous_steps = PRIMARY_STEPS_MAX as i32 + 1;
        let mut previous_weight = f32::NEG_INFINITY;
        for height_in_thicknesses in [100.0_f32, 10.0, 3.0, 1.0, 0.3, 0.1, 0.0] {
            let origin = up * (params.inner_radius + height_in_thicknesses * params.thickness());
            let march = march_single_scattering_f32(origin, -up, SKY_DOME_RADIUS, &params)
                .unwrap_or_else(|| {
                    panic!("a ray from {height_in_thicknesses} thicknesses up, aimed at the Earth, must hit the shell")
                });
            assert!(march.primary_steps >= 1 && march.light_steps >= 1, "the L66-67 clamp must hold");
            assert!(march.primary_steps <= PRIMARY_STEPS_MAX as i32);
            assert!(march.light_steps <= LIGHT_STEPS_MAX as i32);
            assert!((0.0..=1.0).contains(&march.w_inside_atmosphere));
            assert!(
                march.primary_steps <= previous_steps,
                "the budget must shrink monotonically as the camera descends: {} after {}",
                march.primary_steps,
                previous_steps
            );
            assert!(march.w_inside_atmosphere >= previous_weight);
            previous_steps = march.primary_steps;
            previous_weight = march.w_inside_atmosphere;
        }
        // 在海平面处 `w_inside_atmosphere == 0.5` 恰好成立，所以 B1 L66-67 给出
        // 16 - int(0.5*12) = 10 个主步数
        assert_eq!(previous_steps, 10, "at the surface the D2 weight must be exactly 0.5");
        assert!((previous_weight - 0.5).abs() < 1.0e-6);

        // 在大气高层之上，完整的 B1 预算被用满
        let far = up * (params.inner_radius + 100.0 * params.thickness());
        let march = march_single_scattering_f32(far, -up, SKY_DOME_RADIUS, &params)
            .expect("a ray from far above, aimed at the Earth, must hit the shell");
        assert_eq!(
            (march.primary_steps, march.light_steps),
            (PRIMARY_STEPS_MAX as i32, LIGHT_STEPS_MAX as i32),
            "B1 L26-27 PRIMARY_STEPS_MAX / LIGHT_STEPS_MAX must be reachable"
        );
        assert!(march.w_inside_atmosphere < 1.0e-6);
    }

    #[test]
    fn transmittance_and_alpha_stay_in_range_for_every_view_direction() {
        let params = gpu_params();
        let origin = params.sun_direction * (params.inner_radius + 0.001);
        let helper = if params.sun_direction.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
        let u = params.sun_direction.cross(helper).normalize();
        let mut hits = 0;
        for degree in 0..=180 {
            let theta = (f64::from(degree).to_radians()) as f32;
            let direction = (params.sun_direction * theta.cos() + u * theta.sin()).normalize();
            let Some(shading) = shade_sky_f32(origin, direction, SKY_DOME_RADIUS, &params, 1.0) else {
                continue;
            };
            hits += 1;
            assert!(shading.radiance.is_finite() && shading.transmittance.is_finite());
            assert!(
                shading.radiance.min_element() >= 0.0,
                "in-scattered radiance cannot be negative at {degree} deg: {:?}",
                shading.radiance
            );
            for channel in [shading.transmittance.x, shading.transmittance.y, shading.transmittance.z] {
                assert!((0.0..=1.0).contains(&channel), "transmittance must be a fraction at {degree} deg: {channel}");
            }
            assert!((0.0..=1.0).contains(&shading.alpha), "alpha must be a fraction at {degree} deg: {}", shading.alpha);
        }
        assert_eq!(hits, 181, "every direction from a camera inside the shell must intersect it");
    }

    // -- v2_sky 基线 -------------------------------------------------

    /// 构造一个 `orbit_camera` 风格的姿态：相机坐在沿 `zenith` 的 `distance`
    /// 个 render unit 处，看向世界原点。`up_hint` 固定滚转
    /// （先对 `zenith` 做 Gram-Schmidt 正交化）。
    fn look_at_origin_pose(zenith: DVec3, up_hint: DVec3, distance: f64) -> (DVec3, Quat) {
        let back = zenith.normalize();
        let position = back * distance;
        let up = (up_hint - up_hint.dot(back) * back).normalize();
        let right = up.cross(back);
        let rotation = Quat::from_mat3(&Mat3::from_cols(vec3_of(right), vec3_of(up), vec3_of(back)));
        (position, rotation)
    }

    /// 三个 v2_sky 光照制式，表示为相对冻结太阳的 `(zenith, up_hint)` 对。
    /// 制式*就是*太阳相对相机当地地平线的仰角，即 `dot(sun, zenith)` ——
    /// 因为 `orbit_camera` 总是看向原点，因而总是沿 `-zenith` 看。
    fn regime_axes(regime: &str, sun: DVec3) -> (DVec3, DVec3) {
        match regime {
            // 太阳位于当地天顶（+90 度）
            "noon" => (sun, DVec3::Z),
            // 太阳恰好落在当地地平线上（0 度）；`right` 结果与
            // `sun` 相等，所以日落位于画面的右边缘
            "dusk" => {
                let horizontal = (DVec3::Z - DVec3::Z.dot(sun) * sun).normalize();
                (horizontal, horizontal.cross(sun))
            }
            // 太阳位于天底（-90 度）：相机站在对日点上，
            // 而行星遮蔽了每条光线
            "night" => (-sun, DVec3::Z),
            other => unreachable!("unknown v2_sky regime {other}"),
        }
    }

    /// 按文件顺序的六个 `specs/scripts/v2_sky.toml` 镜头：三个制式
    /// 取标准轨道距离，随后同样三个取宽距离。
    /// 帧号遵循 v1_postfx/v2_fxaa 的 180 帧节奏。
    fn baseline_shots() -> Vec<(String, u32, f64, DVec3, Quat)> {
        let sun = frozen_sun();
        let mut shots = Vec::with_capacity(6);
        let mut frame = 180_u32;
        for distance in [3.0_f64, 8.0] {
            for regime in ["noon", "dusk", "night"] {
                let (zenith, up_hint) = regime_axes(regime, sun);
                let (position, rotation) = look_at_origin_pose(zenith, up_hint, distance);
                shots.push((format!("sky_{regime}_{}", distance as u32), frame, distance, position, rotation));
                frame += 180;
            }
        }
        shots
    }

    /// 转录进 `specs/scripts/v2_sky.toml` 的六个姿态。把这些数值
    /// 也留在这里，可将 TOML 与代码锁在一起：若任一方漂移，
    /// 本测试就会失败并指出是哪个镜头。
    const BASELINE_SHOT_TABLE: [(&str, u32, [f64; 3], [f32; 4]); 6] = [
        ("sky_noon_3", 180, [0.523525, -2.710276, -1.174873], [0.830360, 0.079463, 0.052540, 0.549024]),
        ("sky_dusk_3", 360, [0.222823, -1.153549, 2.760376], [0.127207, 0.154130, -0.623691, 0.755694]),
        ("sky_night_3", 540, [-0.523525, 2.710276, 1.174873], [-0.052540, 0.549024, 0.830360, -0.079463]),
        ("sky_noon_8", 720, [1.396067, -7.227402, -3.132995], [0.830360, 0.079463, 0.052540, 0.549024]),
        ("sky_dusk_8", 900, [0.594195, -3.076132, 7.361002], [0.127207, 0.154130, -0.623691, 0.755694]),
        ("sky_night_8", 1080, [-1.396067, 7.227402, 3.132995], [-0.052540, 0.549024, 0.830360, -0.079463]),
    ];

    #[test]
    fn sky_baseline_poses_encode_the_three_lighting_regimes() {
        let sun = frozen_sun();
        let sun32 = vec3_of(sun);
        let shots = baseline_shots();
        assert_eq!(shots.len(), 6);
        // 漂移是被*收集*而非就地断言，所以对六个姿态中任一的改动
        // 会在一次运行中报告整张修正后的表，而非
        // 在第一个镜头失败并藏起其余五个。
        let mut drift: Vec<String> = Vec::new();

        for (index, (label, frame, distance, position, rotation)) in shots.iter().enumerate() {
            assert_eq!(*frame, 180 * (index as u32 + 1), "{label} must be captured on frame {frame}");
            assert!((position.length() - *distance).abs() < 1.0e-9, "{label} distance");

            // orbit_camera 看向世界原点
            let forward = *rotation * Vec3::NEG_Z;
            let to_origin = (-vec3_of(*position)).normalize();
            assert!(
                forward.dot(to_origin) > 0.9999,
                "{label} must look at the world origin, dot {}",
                forward.dot(to_origin)
            );

            // 制式是太阳相对相机当地地平线的仰角
            let elevation = vec3_of(*position).normalize().dot(sun32);
            let expected = match label.split('_').nth(1).unwrap_or_default() {
                "noon" => 1.0_f32,
                "dusk" => 0.0,
                "night" => -1.0,
                other => unreachable!("{other}"),
            };
            assert!(
                (elevation - expected).abs() < 1.0e-5,
                "{label}: the sun elevation must be {expected}, got {elevation}"
            );

            // ……而姿态正是 v2_sky.toml 所携带的那个
            let (toml_label, toml_frame, toml_pos, toml_quat) = BASELINE_SHOT_TABLE[index];
            let mut drifted = toml_label != label.as_str() || toml_frame != *frame;
            for (axis, want) in [position.x, position.y, position.z].iter().zip(toml_pos) {
                drifted |= (axis - want).abs() >= 1.0e-6;
            }
            for (component, want) in [rotation.x, rotation.y, rotation.z, rotation.w].iter().zip(toml_quat) {
                drifted |= (component - want).abs() >= 1.0e-6;
            }
            if drifted {
                drift.push(format!(
                    "        (\"{label}\", {frame}, [{:.6}, {:.6}, {:.6}], [{:.6}, {:.6}, {:.6}, {:.6}]),",
                    position.x,
                    position.y,
                    position.z,
                    rotation.x,
                    rotation.y,
                    rotation.z,
                    rotation.w
                ));
            }

            println!(
                "[[shot]]\nframe = {frame}\nname = \"{label}\"\npos = [{:.6}, {:.6}, {:.6}]\nquat = [{:.6}, {:.6}, {:.6}, {:.6}]\nfov_y = 60.0\n",
                position.x, position.y, position.z,
                rotation.x, rotation.y, rotation.z, rotation.w
            );
        }

        assert!(
            drift.is_empty(),
            "BASELINE_SHOT_TABLE has drifted from the computed poses, so \
             specs/scripts/v2_sky.toml is stale too. Replace the table rows with:\n{}",
            drift.join("\n")
        );

        // "dusk" 还额外把太阳放到画面的右轴上
        for (label, _, _, _, rotation) in shots.iter().filter(|shot| shot.0.contains("dusk")) {
            let right = *rotation * Vec3::X;
            assert!(
                right.dot(sun32) > 0.9999,
                "{label}: the sunset must sit on the frame's right edge, dot {}",
                right.dot(sun32)
            );
        }
    }

    // -- 无头 app 集成 --------------------------------------------

    /// 一个安装了插件且**无** asset 后端的无头 app，
    /// 对应 `terrain/mod.rs` 的独立插件测试。
    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(CesiumAtmospherePlugin);
        app.world_mut().insert_resource(AnimationClock::default());
        app
    }

    fn dome_count(app: &App) -> usize {
        app.world()
            .iter_entities()
            .filter(|entity| entity.contains::<SkyDome>())
            .count()
    }

    fn dome_material(app: &App) -> Handle<SkyDomeMaterial> {
        let mut found = None;
        for entity in app.world().iter_entities() {
            if let Some(material) = entity.get::<MeshMaterial3d<SkyDomeMaterial>>() {
                found = Some(material.0.clone());
            }
        }
        found.expect("the dome must carry a SkyDomeMaterial")
    }

    /// M5-C 之前的 CPU 天空，从 `sky_system` 的 gate-OFF 分支逐字
    /// 转写（含其无相机 `view_dir == sun_dir` 回退），使那
    /// 分支有一个独立预言机，而非与自己比较。
    fn expected_clear_color(sun_direction: &Vec3, params: &AtmosphereParameters) -> Color {
        let sun_dir = dvec3_of(*sun_direction);
        let view_dir = sun_dir; // 无头：`camera_query.get_single()` 失败
        let sky_color = compute_sky_color(view_dir, sun_dir, 1000.0, params);
        let horizon_glow = compute_horizon_glow(sun_dir.z);
        let r = (sky_color[0] as f32 * 0.3 + horizon_glow[0] as f32 * 0.3).clamp(0.0, 1.0);
        let g = (sky_color[1] as f32 * 0.3 + horizon_glow[1] as f32 * 0.3).clamp(0.0, 1.0);
        let b = (sky_color[2] as f32 * 0.3 + horizon_glow[2] as f32 * 0.3).clamp(0.0, 1.0);
        Color::srgb(r, g, b)
    }

    /// `celestial_system` 的确切发布规则：f64 ECI 太阳方向 ->
    /// f32 -> 重归一化，在 app 自身时钟报告的历元处。
    ///
    /// 从*实时 app* 而非从 [`frozen_julian_date`] 推导这个预言机，使它成为
    /// 关于系统的预言机，而不仅是关于算术的：它端到端地复现
    /// `celestial_system` 的发布规则（f64 ECI 方向 -> 逐分量 `as f32` -> 重归一化），
    /// 所以系统所执行的重归一化——`vec3_of(frozen_sun())` 所省略的那一步——
    /// 也是被检查内容的一部分。两者现在按构造在历元上一致，
    /// 这正是两处都读取时钟的意义。这些无头 app 中没有任何东西
    /// 会推进时钟（`time_dynamic_update_system` 不属于 `CesiumAtmospherePlugin`），
    /// 所以历元跨帧稳定，值也是确定性的。
    fn published_sun(app: &App) -> Vec3 {
        let julian_date = app.world().resource::<AnimationClock>().current_time().total_days();
        vec3_of(compute_sun_direction_eci(julian_date)).normalize_or_zero()
    }

    /// dev-005 / dev-011 类缺陷守卫：`CesiumAtmospherePlugin` 必须能在
    /// `MinimalPlugins` 下构建并运行帧——无 `AssetPlugin`、无
    /// `RenderPlugin`、无 `Assets<Shader>`——而不 panic。这正是
    /// `shader_registry::try_load_internal_shader` 与
    /// `Option<ResMut<Assets<_>>>` 系统参数所换来的。
    #[test]
    fn plugin_builds_and_runs_frames_headlessly() {
        let mut app = headless_app();
        app.update();
        app.update();
        assert!(
            app.world().contains_resource::<ClearColor>(),
            "the plugin must own ClearColor so it cannot be used without it"
        );
        assert_eq!(
            app.world().resource::<SkyAtmosphere>().dome,
            sky_dome_gate_enabled(),
            "the plugin must seed SkyAtmosphere::dome from CESIUM_ENABLE_SKYDOME"
        );
        // 无 asset 后端 -> MaterialPlugin 被跳过 -> 无法 spawn 任何 dome，
        // 且 setup 系统必须降级为空操作
        assert!(!crate::shader_registry::asset_backend_available(&app));
        assert_eq!(dome_count(&app), 0);
    }

    /// 门控 OFF：`sky_system` 必须**逐字节**复现 M5-C 之前的 CPU `ClearColor` 天空。
    /// 这就是“门控 OFF -> 八个 v0 基线零差异”的红线。
    #[test]
    fn gate_off_reproduces_the_pre_m5c_clear_color_sky_byte_for_byte() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<SkyAtmosphere>().dome = false;
        app.update();

        let lighting = app.world().resource::<LightingParams>().clone();
        let sky = app.world().resource::<SkyAtmosphere>().clone();
        let clear = app.world().resource::<ClearColor>().0;

        // `celestial_system` 必须发布时钟所隐含的那个确切太阳……
        let expected_sun = published_sun(&app);
        assert!(
            (lighting.sun_direction - expected_sun).length() < 1.0e-6,
            "celestial_system must publish the clock's sun, got {:?} want {:?}",
            lighting.sun_direction,
            expected_sun
        );
        // ……而那个时钟必须坐在 v2_sky 基线历元上，正是它使一次
        // FIXED_TIME 捕获具有确定性，也正是它把 `specs/scripts/v2_sky.toml`
        // （由 `frozen_sun()` 构建）中的姿态绑定到渲染上。
        // `frozen_julian_date()` 自己读取 `AnimationClock::default()`，所以这是
        // 精确而非容差的：那个容差本会让 TOML 姿态从渲染出的天空
        // 悄然漂移而无人察觉。
        let julian_date = app.world().resource::<AnimationClock>().current_time().total_days();
        assert_eq!(
            julian_date.to_bits(),
            frozen_julian_date().to_bits(),
            "AnimationClock::default() must sit bit-exactly on the v2_sky baseline epoch"
        );

        let expected = expected_clear_color(&lighting.sun_direction, &sky.atmosphere_params);
        assert_eq!(clear, expected, "the gate-OFF sky must be byte-for-byte the pre-M5-C ClearColor");
        assert_ne!(clear, Color::BLACK, "the CPU sky must actually paint something");
        println!("gate-OFF ClearColor = {clear:?}");
    }

    /// Gate ON: the WGSL owns the sky, so the CPU ClearColor branch must not run
    /// at all — otherwise the dome composite is painted twice.
    #[test]
    fn gate_on_leaves_the_clear_color_alone() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<SkyAtmosphere>().dome = true;
        let before = app.world().resource::<ClearColor>().0;
        app.update();
        assert_eq!(
            app.world().resource::<ClearColor>().0,
            before,
            "with the dome on, sky_system must not touch ClearColor"
        );
        // 而 gate-OFF 值不同，证明两条分支确实分裂
        app.world_mut().resource_mut::<SkyAtmosphere>().dome = false;
        app.update();
        assert_ne!(
            app.world().resource::<ClearColor>().0,
            before,
            "with the dome off the CPU sky must take over"
        );
    }

    /// 当 asset 后端存在时插件注册
    /// `MaterialPlugin::<SkyDomeMaterial>`，`sky_dome_setup` spawn 恰好一个
    /// 由领域参数播种的 dome，`sky_system` 推送冻结的太阳
    /// uniform（然后停止弄脏它，这正是使 `FIXED_TIME`
    /// 捕获逐位可复现的原因），而把门控关掉则拆除它。
    #[test]
    fn sky_dome_setup_is_idempotent_and_tears_down() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(AssetPlugin::default());
        // `AssetPlugin` 提供一个 `AssetServer` 但*不*提供 `Assets<Shader>`；那
        // 存储通常来自渲染栈。`MaterialPlugin` 内部添加
        // `PrepassPipelinePlugin`，其 `build` 调用
        // `load_internal_asset!` 并无条件解引用它
        // （bevy_pbr-0.15.3 `prepass/mod.rs` L70）—— 因此插件现在也对
        // `shader_registry::shader_assets_available` 设门，而本测试必须
        // 复现 `RenderPlugin` 本会插入的东西。完全限定以
        // 免需要任何 trait 导入。
        <App as bevy::asset::AssetApp>::init_asset::<bevy::render::render_resource::Shader>(
            &mut app,
        );
        // `Assets<Mesh>` 来自 `MeshPlugin`，同属渲染栈
        // 的一部分，在 `AssetPlugin` 下同样缺失。`sky_dome_setup` 把它取作
        // `Option<ResMut<_>>`，当它为 `None` 时提前返回，所以没有
        // 这一行 dome 就根本不 spawn，而下面的测试会失败在
        // `dome_count == 1` 上而非缺失的存储上 —— 一次静默空操作。
        <App as bevy::asset::AssetApp>::init_asset::<bevy::render::mesh::Mesh>(&mut app);
        app.add_plugins(CesiumAtmospherePlugin);
        app.world_mut().insert_resource(AnimationClock::default());
        app.world_mut().resource_mut::<SkyAtmosphere>().dome = true;

        assert!(
            app.world().contains_resource::<Assets<SkyDomeMaterial>>(),
            "with an AssetServer and Assets<Shader> the plugin must register MaterialPlugin::<SkyDomeMaterial>"
        );

        app.update();
        app.update(); // 第二帧不得再添加第二个 dome
        assert_eq!(dome_count(&app), 1, "exactly one dome must exist");

        let expected_sun = published_sun(&app);
        let handle = dome_material(&app);
        {
            let materials = app.world().resource::<Assets<SkyDomeMaterial>>();
            let params = &materials.get(&handle).expect("the dome material must exist").params;
            assert_eq!(params.inner_radius, 1.0);
            assert_eq!(params.mode, MODE_RAYMARCH, "production must ray-march, not use the parity probe");
            assert_eq!(params.primary_steps_max, PRIMARY_STEPS_MAX);
            assert_eq!(params.light_steps_max, LIGHT_STEPS_MAX);
            assert!(
                (params.sun_direction - expected_sun).length() < 1.0e-6,
                "sky_system must push the clock's sun into the uniform, got {:?} want {:?}",
                params.sun_direction,
                expected_sun
            );
        }
        // 重新推送同一方向不得再次弄脏材质
        {
            let mut materials = app.world_mut().resource_mut::<Assets<SkyDomeMaterial>>();
            assert!(
                !update_sun_direction(&mut materials, &handle, expected_sun),
                "a frozen clock must leave the bind group untouched after the first frame"
            );
            assert!(update_sun_direction(&mut materials, &handle, -expected_sun), "a real change must go through");
            // 上面的取反*生效了*，所以存储的方向现在是
            // `-expected_sun`，把它放回去本身就是一次真正的改动。只有
            // 在那做完之后，存储值才重新是 `+expected_sun`，而这正是
            // 下面的重缩放检查实际所需的前提条件 ——
            // 对照 `-expected_sun` 断言时，`expected_sun * 7.5` 归一化为一个
            // 相距 2.0 的方向，并被正确地报告为一次改动。
            assert!(
                update_sun_direction(&mut materials, &handle, expected_sun),
                "restoring the negated direction must go through"
            );
            assert!(
                !update_sun_direction(&mut materials, &handle, expected_sun * 7.5),
                "a rescaled direction normalises to the value already stored"
            );
        }

        // 拆除
        app.world_mut().resource_mut::<SkyAtmosphere>().dome = false;
        app.update();
        assert_eq!(dome_count(&app), 0, "gate OFF must despawn the dome");
    }

    // -- 应用层的 Ryan H1 --------------------------------------------

    /// 一个带有刚好足以注册 `MaterialPlugin` 的渲染栈的 app：`AssetPlugin`
    /// 提供 `AssetServer`，加上渲染栈本会插入的两个 `init_asset`
    /// 存储（为何两者都需要见
    /// [`sky_dome_setup_is_idempotent_and_tears_down`]）。
    /// `with_clock = false` 复现那个没有
    /// `AnimationClock` 的入口点，即 Ryan 的 H1 窗口的*无界*一半。
    fn asset_backed_app(with_clock: bool) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(AssetPlugin::default());
        <App as bevy::asset::AssetApp>::init_asset::<bevy::render::render_resource::Shader>(
            &mut app,
        );
        <App as bevy::asset::AssetApp>::init_asset::<bevy::render::mesh::Mesh>(&mut app);
        app.add_plugins(CesiumAtmospherePlugin);
        if with_clock {
            app.world_mut().insert_resource(AnimationClock::default());
        }
        app.world_mut().resource_mut::<SkyAtmosphere>().dome = true;
        app
    }

    fn material_params(app: &App, handle: &Handle<SkyDomeMaterial>) -> SkyAtmosphereParams {
        app.world()
            .resource::<Assets<SkyDomeMaterial>>()
            .get(handle)
            .expect("the dome material must exist")
            .params
    }

    /// Ryan H1，spawn 帧：dome 材质的 `sun_direction` 绝不可在退化状态下
    /// 被*观测到*。
    ///
    /// Bevy 0.15 的 `.chain()` 在被链起的各系统**之间**应用延迟命令
    /// （bevy_ecs-0.15.4 `schedule/schedule.rs`，`chain_second` 测试；
    /// `chain_ignore_deferred()` 是退出项），所以 `sky_system` 确实看到
    /// `sky_dome_setup` 在同一帧 spawn 的实体，并在渲染提取前刷新
    /// uniform。这在此处是被*断言*的而非假设的 ——
    /// 评审注记预测了一个一帧的 `get_single() -> Err` 窗口，而上面的
    /// `.chain()` 语义正是使该预测在带时钟路径上不成立的原因。播种修复（`Vec3::X`）
    /// 覆盖了确实仍然存在的路径：一个退化的 `LightingParams::sun_direction` 经过
    /// `normalize_or_zero()`，以及下面那个无时钟的 app。
    #[test]
    fn the_spawn_frame_uniform_is_never_degenerate() {
        let mut app = asset_backed_app(true);
        app.update(); // 恰好一帧
        assert_eq!(dome_count(&app), 1, "the dome must exist after the first Update");
        let handle = dome_material(&app);
        let params = material_params(&app, &handle);
        let sun = params.sun_direction;
        assert!(sun.is_finite());
        assert!(
            sun.length_squared() >= DEGENERATE_DIRECTION_EPSILON,
            "the uniform must never be observable in a degenerate state, got {sun:?}"
        );
        let expected = published_sun(&app);
        assert!(
            (sun - expected).length() < 1.0e-6,
            "`.chain()` flushes commands between chained systems, so sky_system must already \
             have refreshed the seed on the spawn frame: got {sun:?} want {expected:?}"
        );
        let shading = shade_sky_f32(
            Vec3::new(0.0, 0.0, params.inner_radius + 0.001),
            Vec3::Y,
            SKY_DOME_RADIUS,
            &params,
            1.0,
        )
        .expect("the zenith ray hits the shell");
        assert!(
            shading.radiance.is_finite() && shading.alpha.is_finite(),
            "spawn-frame sky must be finite: {shading:?}"
        );
    }

    /// Ryan H1 的**无界**一半：没有 `AnimationClock` 时，`sky_system`
    /// 在其时钟守卫（L87-90）处返回，所以没有任何东西刷新
    /// uniform，而 `from_domain` 播种的无论是什么就是 GPU 得到的 —— 在*每*一帧上，
    /// 而非仅一帧。修复前该播种是 `Vec3::ZERO`，即一片永久的 NaN 天空；
    /// 现在播种是 `Vec3::X`，而 D9 守卫甚至让一个被强制的 ZERO 保持有限。
    ///
    /// 这也是门控切换的情形：`SkyAtmosphere::dome` 关掉再打开会经由
    /// `from_domain` 拆除并重新 spawn，所以播种是一个刚重新 spawn 的 dome
    /// 直到下次刷新前所携带的东西。
    #[test]
    fn without_a_clock_the_seed_survives_and_still_shades_a_finite_sky() {
        let mut app = asset_backed_app(false);
        app.update();
        app.update();
        app.update();
        assert_eq!(dome_count(&app), 1);
        let handle = dome_material(&app);
        let params = material_params(&app, &handle);
        assert_eq!(params.sun_direction, Vec3::X, "the seed must survive untouched");
        assert!(params.sun_direction.length_squared() >= DEGENERATE_DIRECTION_EPSILON);
        let origin = Vec3::new(0.0, 0.0, params.inner_radius + 0.001);
        let shading = shade_sky_f32(origin, Vec3::Y, SKY_DOME_RADIUS, &params, 1.0)
            .expect("the zenith ray hits the shell");
        assert!(shading.radiance.is_finite(), "got {:?}", shading.radiance);
        assert!(shading.transmittance.is_finite() && shading.alpha.is_finite());

        // 而修复前的播种，被强制放回后，由 D9 守卫吸收
        let degenerate = SkyAtmosphereParams {
            sun_direction: Vec3::ZERO,
            ..params
        };
        let shading = shade_sky_f32(origin, Vec3::Y, SKY_DOME_RADIUS, &degenerate, 1.0)
            .expect("the primary ray is unaffected by the sun direction");
        assert!(shading.radiance.is_finite(), "D9 guard regression: {shading:?}");
        assert!(shading.alpha.is_finite(), "D9 guard regression: {shading:?}");
    }
}



