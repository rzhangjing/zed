//! 桥接领域 Material → Bevy FabricMaterial。
//!
//! 提供 [`CesiumMaterialPlugin`]，它读取实体组件并将 Fabric 程序化材质
//! 应用到 Bevy mesh，外加逐帧 uniform 更新（例如水的动画时间）。

// 遗留的 CesiumJS 移植风格债（deferred.md #18）；在 M13 lint 清理时或本文件在其里程碑被重写时重新审视
#![allow(clippy::type_complexity)]
use bevy::prelude::*;
use cesium_material::{MaterialSystem, UniformValue};

use crate::fabric_material::{
    fabric_material_from_domain, fabric_material_from_domain_with_maps, generate_water_normal_map,
    generate_water_specular_map, FabricKind, FabricMaterial, FabricMaterialPlugin,
};
use cesium_shadow::{OceanConfig, OceanSurface};

/// 引用一个 CesiumJS Fabric 材质以应用到实体的组件。
///
/// 将其附加到任何带有 [`MeshMaterial3d<FabricMaterial>`] 目标的实体
/// （或直接插入 [`FabricMaterial`]），材质便会从领域层生成。
#[derive(Component, Clone, Debug)]
pub struct MaterialRef {
    /// CesiumJS 材质类型名（例如 `"ElevationContour"`、
    /// `"RimLighting"`、`"Color"`）。
    pub type_name: String,
    /// 可选的 uniform 覆盖（以 Fabric uniform 名为键）。
    pub uniforms: std::collections::BTreeMap<String, UniformValue>,
}

impl MaterialRef {
    /// 从类型名创建一个使用默认 uniform 的材质引用。
    pub fn new(type_name: impl Into<String>) -> Self {
        Self {
            type_name: type_name.into(),
            uniforms: std::collections::BTreeMap::new(),
        }
    }

    /// 创建并带有 uniform 覆盖。
    pub fn with_uniforms(
        type_name: impl Into<String>,
        uniforms: std::collections::BTreeMap<String, UniformValue>,
    ) -> Self {
        Self {
            type_name: type_name.into(),
            uniforms,
        }
    }
}

/// 为动画材质保存时间的资源（例如 Water）。
#[derive(Resource, Default)]
pub struct MaterialAnimationTime {
    /// 自启动以来累积的增量秒（遗留的墙钟字段）。
    pub time: f32,
    /// 镜像 CesiumJS `czm_frameNumber` 的单调帧计数器
    /// （水面着色器：`time = czm_frameNumber * animationSpeed`）。
    /// DEVIATION：Water 动画现按*帧*推进（匹配 CesiumJS），
    /// 而非按累积秒；参见 docs/deviations.md#dev-019。
    /// 在 FIXED_TIME（`delta_secs() == 0.0`）下冻结 —— 见 [`advance_animation`]。
    pub frame_number: u32,
    /// 最后发布到 Water 的 `extra_c.z` 的 `frame_number`（Ryan L6）。
    /// 首次写入前为 `None`；等于当前帧 ⇒ 跳过该次写入，
    /// 以免无谓地重新编码 bind group。
    last_written_frame: Option<u32>,
}

/// Water 动画的帧周期（Ryan L4）。
///
/// `extra_c.z` 以 `frame_number as f32` 写入。f32 仅在不超过 2^24
/// （16_777_216 ≈ 3.2 天 @ 60 fps）时才能精确表示每个整数；超过该值后
/// 计数器在 f32 中停止推进、Water 动画冻结，而 `u32` 本身在约 2.3 年后回绕。
/// 水面着色器仅通过 `fract()` 消费 `time`（czm_get_water_noise 将 `time`
/// 乘以采样方向后对 UV 取 `fract`），因此可见运动是周期性的，计数器可以对
/// 一个远低于 2^24 的周期取模。2^20（1_048_576 ≈ 4.8 小时 @ 60 fps）
/// 使 `extra_c.z` 保持精确可表示，并将回绕限制为一次难以察觉的相位跳变。
pub const WATER_FRAME_PERIOD: u32 = 1 << 20;

/// 将材质动画时钟推进 `delta_secs`，返回要发布到 Water 的 `extra_c.z` 的帧计数器。
///
/// Ryan M3 / dev-019 方案 A：在 FIXED_TIME（`delta_secs() == 0.0`，`main.rs`
/// 通过 `TimeUpdateStrategy::ManualDuration(ZERO)` 设置）下帧计数器被冻结，
/// 因此 Water 相位 —— 以及每个捕获的基线 —— 都是位可复现的，而不会随截图前
/// 运行的 Update tick 数漂移。三张 Water 特写仍可区分，因为它们按预设
/// （振幅/频率）不同，而非按相位不同。
fn advance_animation(anim: &mut MaterialAnimationTime, delta_secs: f32) -> u32 {
    anim.time += delta_secs;
    if delta_secs != 0.0 {
        anim.frame_number = anim.frame_number.wrapping_add(1);
    }
    anim.frame_number
}

/// 缓存每个 `kind == Water` 的 [`FabricMaterial`] 的 handle（Ryan L6）。
///
/// `update_material_uniforms` 过去每帧都对整个 `Assets<FabricMaterial>`
/// 调用 `iter_mut()`；每次 `Mut<T>` drop 都无条件地将资产标记为已改变，
/// 于是所有 Water 材质被重新弄脏 → `as_bind_group` 每帧重新编码 192 B 的
/// uniform 并重建 bind group。我们在此缓存 Water 的 handle，仅对这些
/// `get_mut`，且仅当发布的帧确实改变时才做。
///
/// 该缓存能自愈：只要 `Assets<FabricMaterial>::len()` 变化，它就（通过一次
/// 不弄脏的不可变 `iter()`）被重建，因此由应用 showcase 直接 spawn ——
/// 绕过 `apply_fabric_materials` —— 的 Water 材质也会被纳入。
#[derive(Resource, Default)]
pub struct WaterMaterialHandles {
    /// 当前帧缓存的水材质 handle 列表（逐帧写入动画 uniform 的目标）。
    handles: Vec<Handle<FabricMaterial>>,
    /// 上次构建缓存时观察到的 `Assets<FabricMaterial>::len()`，用于判断是否需重建。
    known_len: usize,
}

/// 面向默认 [`OceanConfig`] 的共享 Water 法线/镜面贴图（Daniel L4）。
///
/// `apply_fabric_materials` 过去为*每个* Water 实体都构建一个新的
/// `OceanSurface::new(OceanConfig::default())` 加两张 128×128（各 64 KB）
/// 程序化纹理 —— 对 N 个水体实体产生 O(N) 个逐字节相同的贴图对，因为默认
/// 配置是常量。第一个 Water 实体在此生成该贴图对一次；后续默认配置实体复用
/// 相同的 handle。非默认配置（showcase 的 Calm/Rough 预设）仍按实体生成。
#[derive(Resource, Default)]
pub struct SharedWaterTextures {
    /// 共享的水法线贴图 handle（首个默认配置水体实体生成，后续复用）。
    normal_map: Option<Handle<Image>>,
    /// 共享的水镜面贴图 handle（与法线贴图同生命周期）。
    specular_map: Option<Handle<Image>>,
}

/// 桥接领域 [`cesium_material::Material`] → Bevy [`FabricMaterial`] 的插件。
///
/// 添加如下系统：
/// - 将 [`MaterialRef`] 组件作为 [`FabricMaterial`] 实例应用到实体。
/// - 逐帧更新动画 uniform 值（水时间等）。
pub struct CesiumMaterialPlugin;

impl Plugin for CesiumMaterialPlugin {
    /// 注册 Fabric 材质插件与动画/缓存资源，并接入材质应用与逐帧动画系统。
    ///
    /// # 参数
    /// - `app`：待初始化的 Bevy 应用。
    fn build(&self, app: &mut App) {
        app.add_plugins(FabricMaterialPlugin)
            .init_resource::<MaterialAnimationTime>()
            .init_resource::<WaterMaterialHandles>()
            .init_resource::<SharedWaterTextures>();

        // 这两个 Update 系统解引用 `Assets<Image>` / `Assets<FabricMaterial>`
        // 和 `Time`，它们在无头 `MinimalPlugins` app（无 AssetPlugin）下均缺失
        // —— 在那里运行会 panic。注册依据 `FabricMaterialPlugin` 已用于
        // `MaterialPlugin` 的同一个 `asset_backend_available` 谓词进行门控，
        // 因此无头 app（以及 Mark M-3 showcase 冒烟测试）会将该插件安装为一个
        // 静默空操作，而 GPU 路径保持不变（像素中性）。
        if crate::shader_registry::asset_backend_available(app) {
            app.add_systems(
                Update,
                (apply_fabric_materials, update_material_uniforms),
            );
        }
    }
}

/// 系统：对于带有 [`MaterialRef`] 组件的实体，在领域 [`MaterialSystem`] 中查找
/// 材质类型，提取 uniform 值，并创建一个 Bevy [`FabricMaterial`] 实例。
///
/// 仅当 [`MaterialRef`] 被添加或改变时运行（通过 `Changed<MaterialRef>`）。
fn apply_fabric_materials(
    mut commands: Commands,
    material_system: Option<Res<MaterialSystemResource>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<FabricMaterial>>,
    mut shared_water: ResMut<SharedWaterTextures>,
    query: Query<(Entity, &MaterialRef, Option<&MeshMaterial3d<FabricMaterial>>), Changed<MaterialRef>>,
) {
    let system = match &material_system {
        Some(res) => &res.0,
        None => {
            warn!("MaterialSystemResource not available; skipping material application");
            return;
        }
    };

    // 本帧无新增/改变 → 跳过。避免每帧都向 `Assets<Image>` 分配一个新的
    // 回退图像（M5-D）。
    if query.is_empty() {
        return;
    }

    // 为需要纹理的材质创建一个 1x1 白色回退图像。
    // 不使用纹理的材质（Color 等）会忽略此绑定。
    // 注意：Rgba8UnormSrgb 对这个*颜色*回退是正确的（类影像），
    // 但 Water 的 normalMap/specularMap 是 LINEAR 且在下方使用 Rgba8Unorm ——
    // 切勿将该 Srgb 格式传播到它们（sRGB 红线）。
    let fallback_img = Image::new(
        bevy::render::render_resource::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        vec![255u8, 255, 255, 255],
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    let fallback_handle = images.add(fallback_img);

    for (entity, mat_ref, _existing_material) in &query {
        let domain_material = match system.from_type(&mat_ref.type_name, mat_ref.uniforms.clone()) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    "Failed to build material '{}' for entity {:?}: {}",
                    mat_ref.type_name, entity, e
                );
                continue;
            }
        };

        let fabric_material =
            if FabricKind::from_type_name(&mat_ref.type_name) == FabricKind::Water {
                // M5-D：Water 绑定真实的程序化生成的法线/镜面贴图
                // （Rgba8Unorm LINEAR，来自领域 cesium_shadow OceanSurface）；
                // 其他类型对所有绑定使用回退。
                //
                // Daniel L4：默认 `OceanConfig` 是常量，因此其 2×(128×128)
                // 纹理对仅生成一次，并在所有默认配置的 Water 实体间共享，
                // 而非产生 O(N) 个相同副本。
                let (normal_map, specular_map) =
                    match (&shared_water.normal_map, &shared_water.specular_map) {
                        (Some(n), Some(s)) => (n.clone(), s.clone()),
                        _ => {
                            let ocean = OceanSurface::new(OceanConfig::default());
                            let n = images.add(generate_water_normal_map(128, &ocean, 200.0));
                            let s = images.add(generate_water_specular_map(128, &ocean, 200.0));
                            shared_water.normal_map = Some(n.clone());
                            shared_water.specular_map = Some(s.clone());
                            (n, s)
                        }
                    };
                fabric_material_from_domain_with_maps(
                    &domain_material,
                    fallback_handle.clone(),
                    normal_map,
                    specular_map,
                )
            } else {
                fabric_material_from_domain(&domain_material, fallback_handle.clone())
            };
        let handle = materials.add(fabric_material);
        commands.entity(entity).insert(MeshMaterial3d(handle));
    }
}

/// 系统：为动画材质做逐帧 uniform 更新。
///
/// 当前更新：
/// - Water 时间动画
fn update_material_uniforms(
    time: Res<Time>,
    mut animation_time: ResMut<MaterialAnimationTime>,
    mut materials: ResMut<Assets<FabricMaterial>>,
    mut water_handles: ResMut<WaterMaterialHandles>,
) {
    // Ryan M3 / dev-019 方案 A：推进时钟，并在 FIXED_TIME 成立（`delta_secs() == 0.0`）
    // 时冻结帧计数器，使 Water 相位位可复现。
    let frame_number = advance_animation(&mut animation_time, time.delta_secs());

    // Daniel M5：`extra_c.z` 现承载一个按帧的计数器，镜像 CesiumJS 的
    // `czm_frameNumber`（水面着色器 `time = czm_frameNumber * animationSpeed`），
    // 而非累积秒。因此 Water 动画依赖帧率（60 fps 推进波形的速度是 30 fps 的两倍）。
    // 任何 Water 基线捕获都必须锁定帧索引，而非墙钟时间 —— 见 specs/scripts/v2_water.toml
    // 与 docs/deviations.md#dev-003 / #dev-019 中的说明。

    // Ryan L6：仅当资产集合变化（一次 spawn/removal）时，通过一次不弄脏的
    // 不可变 `iter()` 重建 Water-handle 缓存。覆盖 `apply_fabric_materials` 路径
    // 以及 showcase 直接 spawn 的材质。
    let current_len = materials.len();
    if current_len != water_handles.known_len {
        water_handles.handles = materials
            .iter()
            .filter(|(_, m)| m.params.kind == FabricKind::Water as u32)
            .map(|(id, _)| Handle::Weak(id))
            .collect();
        water_handles.known_len = current_len;
    }

    // 当发布的帧未改变时（FIXED_TIME，或未推进的任何帧）完全跳过写入，
    // 使 bind group 不被无谓地重新编码、Water 材质不被虚假地弄脏（Ryan L6）。
    if animation_time.last_written_frame == Some(frame_number) {
        return;
    }

    // Ryan L4：对 WATER_FRAME_PERIOD 取模，使 `frame as f32` 在远超 2^24 的
    // f32 整数精度悬崖处仍保持精确。
    let phase = (frame_number % WATER_FRAME_PERIOD) as f32;
    for handle in &water_handles.handles {
        if let Some(material) = materials.get_mut(handle) {
            material.params.extra_c.z = phase;
        }
    }
    animation_time.last_written_frame = Some(frame_number);
}

/// 包装 [`MaterialSystem`] 的资源，使其可作为 Bevy 资源使用。
///
/// [`MaterialSystem`] 持有缓存的内置材质类型定义（GLSL 源 + 默认 uniform），
/// 并被 [`apply_fabric_materials`] 所需。
#[derive(Resource)]
pub struct MaterialSystemResource(pub MaterialSystem);

impl MaterialSystemResource {
    /// 创建并预注册所有内置 CesiumJS 材质类型。
    pub fn with_builtin_materials() -> Self {
        Self(MaterialSystem::with_builtin_materials())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        advance_animation, update_material_uniforms, CesiumMaterialPlugin, FabricKind, FabricMaterial,
        MaterialAnimationTime, MaterialRef, WaterMaterialHandles, WATER_FRAME_PERIOD,
    };
    use crate::fabric_material::FabricParams;
    use crate::CesiumCorePlugin;
    use bevy::prelude::*;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    /// 针对 `docs/deviations.md#dev-005` / `docs/deferred.md#6`（在 M5.1 解决）
    /// 的无头回归守卫。
    ///
    /// 复现四个 `specs/tests/integration/material_integration_test.rs` 用例的
    /// 确切场景：一个 `create_test_app()` 风格的 app（`MinimalPlugins` +
    /// [`CesiumCorePlugin`]，无 `AssetPlugin` / `RenderApp` / wgpu device）
    /// 加上 [`CesiumMaterialPlugin`]。在 M5.1 之前，这会在 `FabricMaterialPlugin::build`
    /// 内部 panic（`load_internal_asset!` → 缺失 `Assets<Shader>`，随后
    /// `MaterialPlugin` → 缺失 `AssetServer`）。[`crate::shader_registry`] 中的
    /// 无头安好守卫改为使注册成为空操作，因此插件得以安装，且 CPU 侧的材质 API 保持可用。
    #[test]
    fn cesium_material_plugin_headless_minimal_does_not_panic() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(CesiumCorePlugin)
            .add_plugins(CesiumMaterialPlugin);

        // test_material_animation_time_resource_initialized
        assert!(app.world().get_resource::<MaterialAnimationTime>().is_some());

        // test_material_ref_component
        let entity = app.world_mut().spawn(MaterialRef::new("Color")).id();
        let got = app.world().get::<MaterialRef>(entity);
        assert!(got.is_some());
        assert_eq!(got.unwrap().type_name, "Color");

        // test_material_ref_with_uniforms
        let mut uniforms = std::collections::BTreeMap::new();
        uniforms.insert(
            "color".to_string(),
            cesium_material::UniformValue::Vec4([1.0, 0.0, 0.0, 1.0]),
        );
        let entity2 = app
            .world_mut()
            .spawn(MaterialRef::with_uniforms("Checkerboard", uniforms))
            .id();
        let got2 = app.world().get::<MaterialRef>(entity2).unwrap();
        assert_eq!(got2.type_name, "Checkerboard");
        assert!(got2.uniforms.contains_key("color"));
    }

    /// Ryan M3 / dev-019 方案 A —— 纯时钟推进辅助函数：在 FIXED_TIME
    /// （`delta_secs() == 0.0`）下帧计数器被冻结；活跃时钟则推进。
    #[test]
    fn advance_animation_freezes_frame_number_under_fixed_time() {
        let mut anim = MaterialAnimationTime::default();
        // FIXED_TIME：delta_secs() == 0.0 → 帧计数器不得推进。
        assert_eq!(advance_animation(&mut anim, 0.0), 0);
        assert_eq!(advance_animation(&mut anim, 0.0), 0);
        assert_eq!(anim.frame_number, 0, "FIXED_TIME must freeze the Water phase");
        // 活跃时钟仍单调推进。
        assert_eq!(advance_animation(&mut anim, 1.0 / 60.0), 1);
        assert_eq!(advance_animation(&mut anim, 1.0 / 60.0), 2);
    }

    /// Ryan L4 —— 取模使 `extra_c.z` 在 f32 中保持精确可表示，远在 2^24
    /// 精度悬崖之上，且该周期远低于此值。
    #[test]
    fn water_frame_period_stays_below_the_f32_integer_precision_cliff() {
        // 编译期检查的不变量（对常量使用裸运行时 `assert!` 会触发
        // clippy::assertions_on_constants）；若周期被提升到/超过 2^24（即
        // `frame as f32` 不再精确处），这会使构建失败。
        const _: () = assert!(
            WATER_FRAME_PERIOD < (1 << 24),
            "WATER_FRAME_PERIOD must be < 2^24 so `frame as f32` stays exact"
        );
        // 一个恰在悬崖之下的帧计数器，取模后在 f32 中仍精确。
        let huge: u32 = (1 << 24) + 12345;
        let reduced = huge % WATER_FRAME_PERIOD;
        let phase = reduced as f32;
        assert_eq!(phase as u32, reduced, "reduced phase must survive f32 exactly");
        assert!(phase < 16_777_216.0);
    }

    /// Ryan M3 —— 真实系统：在 FIXED_TIME（`ManualDuration(ZERO)`）下连续两次
    /// `update_material_uniforms` tick 使 Water 的 `extra_c.z` 保持位一致，
    /// 因此捕获的 Water 基线可复现。
    #[test]
    fn update_material_uniforms_freezes_water_phase_under_fixed_time() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        // `main.rs` 为实现位可复现帧所依赖的 FIXED_TIME 契约。
        // Bevy 0.15.3 惯用法（与 main.rs / specs/tests/camera_control.rs 一致）：
        // 策略是一个独立资源；`Time::new_with_update_strategy` 已被移除。
        // `init_resource::<Time>()` 保证 `Res<Time>` 能解析，即使 `MinimalPlugins`
        // 可能不添加 `TimePlugin`；无论哪种方式，手动的 ZERO 时长（以及从不推进的
        // 默认值）都使 `delta_secs() == 0.0` 成立。
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        app.init_resource::<Time>();
        app.init_resource::<MaterialAnimationTime>();
        app.init_resource::<WaterMaterialHandles>();
        app.init_resource::<Assets<FabricMaterial>>();
        app.add_systems(Update, update_material_uniforms);

        // 用一个非零相位播种一个 Water 材质，使任何虚假写入都可见。
        // 结构体字面量（而非 `default()` + 字段重新赋值，那会触发
        // clippy::field_reassign_with_default）。extra_c 默认为
        // `Vec4::new(0.0, 1000.0, 0.0, 0.5)`；我们植入 z = 123.0。
        let params = FabricParams {
            kind: FabricKind::Water as u32,
            extra_c: Vec4::new(0.0, 1000.0, 123.0, 0.5),
            ..Default::default()
        };
        let mat = FabricMaterial {
            params,
            image: Handle::default(),
            normal_map: Handle::default(),
            specular_map: Handle::default(),
            translucent: true,
        };
        let handle = app
            .world_mut()
            .resource_mut::<Assets<FabricMaterial>>()
            .add(mat);

        app.update();
        let z1 = app
            .world()
            .resource::<Assets<FabricMaterial>>()
            .get(&handle)
            .unwrap()
            .params
            .extra_c
            .z;
        app.update();
        let z2 = app
            .world()
            .resource::<Assets<FabricMaterial>>()
            .get(&handle)
            .unwrap()
            .params
            .extra_c
            .z;

        // 冻结帧 (0) ⇒ 相位 0.0，在两次 tick 间一致。
        assert_eq!(z1, z2, "Water phase drifted under FIXED_TIME (dev-019 方案 A)");
        assert_eq!(z2, 0.0, "frozen frame_number 0 must publish phase 0.0");
    }
}
