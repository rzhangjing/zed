//! Fabric 材质展示（P1.3 视觉验证 + M5-D Water 基线）。
//!
//! 将域 [`cesium_material`] crate 的内置 Fabric 程序化材质渲染到
//! 排列成弧形的球体上，位于地球朝向相机的一侧。
//! 这考验了完整管线：
//!
//! 域 `MaterialSystem::from_type`（Fabric JSON -> `Material`）->
//! 适配器 `fabric_material_from_domain`（uniform 打包）->
//! GPU `FabricMaterial`（WGSL 程序化图案）。
//!
//! M5-D 新增三个 Water 海况条目（calm / medium / rough），由域
//! [`cesium_shadow::OceanSurface`]（Gerstner 波叠加）驱动，带
//! 程序化生成的法线 / 高光贴图，因此复刻的
//! `Water` 程序化图案（case 17u）可被捕获为 `specs/baselines/v2_water/*`。
//!
//! 对应 P1.3 验收标准 "棋盘/条纹/网格材质贴球"（checkerboard /
//! stripe / grid 材质贴球），外加其他内置图案以及
//! M5-D Water 材质。

use bevy::math::DVec3;
use bevy::prelude::*;
use cesium_bevy_render::fabric_material::{
    fabric_material_from_domain, water_material_from_preset, FabricMaterial, WaterPreset,
};
use cesium_bevy_render::{
    create_imagery_texture, geometry_to_mesh, CesiumMaterialPlugin, MaterialSystemResource,
};
use cesium_geospatial::geometry::{self, VertexFormat};
use cesium_material::{MaterialSystem, UniformValue};
use std::collections::BTreeMap;

/// 注册 Fabric 材质管线并生成展示场景的插件。
///
/// 通过 `CESIUM_ENABLE_MATERIAL_SHOWCASE=1` 选择性启用（见 `main.rs`）；默认关闭
/// 以保持 v0 基线像素中性。
pub struct MaterialShowcasePlugin;

impl Plugin for MaterialShowcasePlugin {
    /// 插件装配入口：挂载材质插件与内置材质资源，启动时生成展示场景。
    ///
    /// # 参数
    /// - `app`：Bevy 应用。
    fn build(&self, app: &mut App) {
        // `CesiumMaterialPlugin` 打包了 `FabricMaterialPlugin` +
        // `MaterialAnimationTime` 资源 + 每帧 Water 动画系统
        //（`update_material_uniforms`），因此 Water 用例可动画。
        // `MaterialSystemResource` 被插入，以便打包的
        // `apply_fabric_materials` 系统能找到它并保持静默空操作（展示
        // 直接生成 `FabricMaterial`，不经 `MaterialRef`）。
        app.add_plugins(CesiumMaterialPlugin)
            .insert_resource(MaterialSystemResource::with_builtin_materials())
            .add_systems(Startup, setup_material_showcase);
    }
}

/// 单个展示条目：一个内置材质类型 + uniform 覆盖。
struct ShowcaseEntry {
    /// 内置 Fabric 材质类型名（例如 `"Checkerboard"`）。
    type_name: &'static str,
    /// 在材质默认值之上应用的 uniform 覆盖。
    overrides: Vec<(&'static str, UniformValue)>,
    /// 设置后，这是一个由域 `OceanSurface` 预设（calm / medium / rough）驱动、
    /// 带程序化生成法线 / 高光贴图的 Water 条目。
    water: Option<WaterPreset>,
}

/// 十个展示的 Fabric 材质：七个静态内置（槽位 0..=6），随后
/// 三个 M5-D Water 海况 Calm/Medium/Rough（槽位 7/8/9）。
///
/// 从 [`setup_material_showcase`] 抽取，以便 Mark M-3 测试可在不生成 GPU 应用
/// 的情况下断言数量、Water 槽位和预设顺序。
fn showcase_entries() -> Vec<ShowcaseEntry> {
    vec![
        ShowcaseEntry {
            type_name: "Color",
            overrides: vec![("color", UniformValue::Vec4([0.9, 0.15, 0.15, 1.0]))],
            water: None,
        },
        ShowcaseEntry {
            type_name: "Checkerboard",
            overrides: vec![],
            water: None,
        },
        ShowcaseEntry {
            type_name: "Stripe",
            overrides: vec![
                ("evenColor", UniformValue::Vec4([1.0, 1.0, 1.0, 1.0])),
                ("oddColor", UniformValue::Vec4([0.1, 0.3, 0.9, 1.0])),
            ],
            water: None,
        },
        ShowcaseEntry {
            type_name: "Grid",
            overrides: vec![
                ("color", UniformValue::Vec4([0.0, 1.0, 0.45, 1.0])),
                ("cellAlpha", UniformValue::Float(0.15)),
            ],
            water: None,
        },
        ShowcaseEntry {
            type_name: "Dot",
            overrides: vec![
                ("lightColor", UniformValue::Vec4([1.0, 0.85, 0.0, 1.0])),
                ("darkColor", UniformValue::Vec4([0.12, 0.18, 0.35, 1.0])),
            ],
            water: None,
        },
        ShowcaseEntry {
            type_name: "Fade",
            overrides: vec![],
            water: None,
        },
        ShowcaseEntry {
            type_name: "Image",
            overrides: vec![("repeat", UniformValue::Vec2([3.0, 3.0]))],
            water: None,
        },
        // ── M5-D：三个 Water 海况（calm / medium / rough） ──
        ShowcaseEntry {
            type_name: "Water",
            overrides: vec![],
            water: Some(WaterPreset::Calm),
        },
        ShowcaseEntry {
            type_name: "Water",
            overrides: vec![],
            water: Some(WaterPreset::Medium),
        },
        ShowcaseEntry {
            type_name: "Water",
            overrides: vec![],
            water: Some(WaterPreset::Rough),
        },
    ]
}

/// 每个内置 Fabric 材质生成一个球体，排列成弧形，位于地球朝向
/// 相机的一侧，使每个图案都清晰可见。
fn setup_material_showcase(
    mut commands: Commands,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut images: Option<ResMut<Assets<Image>>>,
    mut fabric_materials: Option<ResMut<Assets<FabricMaterial>>>,
) {
    // 无头安全（Mark M-3）：在 `MinimalPlugins` 下没有资源后端，因此
    // 这些 `ResMut<Assets<_>>` 缺失，非 `Option` 参数在 Startup 系统运行时会 panic。
    // 改为降级为空操作，仿照
    // `sky_system::sky_dome_setup` 的 `Option<ResMut<_>>` 模式。在 GPU 路径上
    // 三者都存在，行为不变（像素中性）。
    let (Some(meshes), Some(images), Some(fabric_materials)) = (
        meshes.as_deref_mut(),
        images.as_deref_mut(),
        fabric_materials.as_deref_mut(),
    ) else {
        return;
    };

    let system = MaterialSystem::with_builtin_materials();

    // 程序化演示纹理：供 `Image` 材质使用，并作为支撑每个材质
    // 采样器绑定的 `czm_defaultImage` 替身。
    // 颜色数据 → Rgba8UnormSrgb（经 create_imagery_texture）。Water 的
    // 法线/高光贴图单独生成为 LINEAR Rgba8Unorm。
    let demo_image = images.add(make_demo_image());

    // 一个共享的单位半径球体网格，按实例缩放。
    let sphere_geometry =
        geometry::ellipsoid_geometry(DVec3::splat(1.0), 32, 64, VertexFormat::ALL);
    let sphere_mesh = meshes.add(geometry_to_mesh(&sphere_geometry, None));

    let entries = showcase_entries();

    let n = entries.len();
    // 弧形布局：球体位于绕地球中心、半径为 `arc_radius` 的圆上，
    // 在朝向相机的半球上按 `total_span_deg` 展开。
    let arc_radius = 1.9_f32;
    let sphere_radius = 0.24_f32;
    let total_span_deg = 150.0_f32; // -75° .. +75°

    for (i, entry) in entries.iter().enumerate() {
        let mut overrides = BTreeMap::new();
        for (key, value) in &entry.overrides {
            overrides.insert((*key).to_string(), value.clone());
        }
        // Water 预设在域默认值之上贡献各自的 frequency/amplitude/animationSpeed/
        // specularIntensity 覆盖。
        if let Some(preset) = entry.water {
            for (key, value) in preset.uniform_overrides() {
                overrides.insert(key.to_string(), value);
            }
        }

        let domain_material = system
            .from_type(entry.type_name, overrides)
            .unwrap_or_else(|e| panic!("failed to build material {}: {}", entry.type_name, e));

        let material = if let Some(preset) = entry.water {
            water_material_from_preset(&mut *images, &domain_material, demo_image.clone(), preset)
        } else {
            fabric_material_from_domain(&domain_material, demo_image.clone())
        };

        let t = if n == 1 { 0.5 } else { i as f32 / (n as f32 - 1.0) };
        let angle = (-total_span_deg / 2.0 + t * total_span_deg).to_radians();
        let position = Vec3::new(arc_radius * angle.sin(), 0.0, arc_radius * angle.cos());

        let label = match entry.water {
            Some(preset) => format!("Water_{}", preset.label()),
            None => entry.type_name.to_string(),
        };
        commands.spawn((
            Name::new(format!("FabricMaterial_{}", label)),
            Mesh3d(sphere_mesh.clone()),
            MeshMaterial3d(fabric_materials.add(material)),
            Transform::from_translation(position).with_scale(Vec3::splat(sphere_radius)),
        ));
    }
}

/// 构建一张小型彩色测试卡纹理，使 `Image` 材质有
/// 独特的内容可采样（红/绿渐变 + 棋盘蓝通道）。
///
/// 颜色（类影像）数据 → 经 [`create_imagery_texture`] 得 `Rgba8UnormSrgb`。
/// 此 Srgb 格式绝不可复用于 Water 的线性法线/高光贴图。
fn make_demo_image() -> Image {
    let size = 64u32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let fx = x as f32 / (size - 1) as f32;
            let fy = y as f32 / (size - 1) as f32;
            let r = (fx * 255.0) as u8;
            let g = (fy * 255.0) as u8;
            let b = (((x / 8) + (y / 8)) % 2 * 255) as u8;
            data.extend_from_slice(&[r, g, b, 255]);
        }
    }
    create_imagery_texture(size, size, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (a) 展示插件必须能安装并在无头模式下运行而不 panic。
    ///
    /// 在 `MinimalPlugins` 下没有资源后端，因此 `Startup` 系统
    /// [`setup_material_showcase`] 会被调度并运行，但通过其 `Option<ResMut<_>>` 守卫
    ///（无 `Assets<Mesh/Image/FabricMaterial>`）降级为空操作；
    /// `CesiumMaterialPlugin` 同样将其 `Update` 系统关闭。这正是
    /// Mark M-3 关注点：展示是唯一的无测试 M5 交付模块。
    #[test]
    fn material_showcase_plugin_headless_minimal_does_not_panic() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(MaterialShowcasePlugin);
        // 不得 panic（Startup 系统提前返回；无材质 Update 系统）。
        app.update();
        // 展示所依赖的 CPU 侧资源即便在无头模式下也存在。
        assert!(app.world().get_resource::<MaterialSystemResource>().is_some());
    }

    /// (b) 共十个条目；Water 预设 Calm/Medium/Rough 占据槽位
    /// 7/8/9（且仅这些），因此 `v2_water.toml` 的三个特写 1:1 对应。
    #[test]
    fn showcase_entries_are_ten_with_three_water_presets_in_slots_7_8_9() {
        let entries = showcase_entries();
        assert_eq!(entries.len(), 10, "showcase must present 10 Fabric materials");

        let water_slots: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.water.is_some())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            water_slots,
            vec![7, 8, 9],
            "exactly three Water entries, in the final slots"
        );
        assert!(entries[..7].iter().all(|e| e.water.is_none()));

        assert_eq!(entries[7].type_name, "Water");
        assert_eq!(entries[8].type_name, "Water");
        assert_eq!(entries[9].type_name, "Water");
        assert_eq!(entries[7].water, Some(WaterPreset::Calm));
        assert_eq!(entries[8].water, Some(WaterPreset::Medium));
        assert_eq!(entries[9].water, Some(WaterPreset::Rough));
    }

    /// (c) Water 弧形姿态必须与 `specs/scripts/v2_water.toml` 匹配，该文件
    /// 记录了三个特写于弧形角 41.67° / 58.33° / 75.00°
    ///（对精确值 41.6667 / 58.3333 / 75.0 的 2 位小数取整，即布局公式
    /// 对 10 个中的槽位 7/8/9 在 150° 跨度上产生的值）。
    ///
    /// 此处独立重算该公式（不通过运行 GPU 设置），
    /// 针对精确解析值（1e-3）紧密断言，并对 toml 记录的度数
    /// 具备取整感知（5e-3 = 最后打印位的一半）。
    /// 每个相机姿态 `3.5·(sinθ, 0, cosθ)` 会与 toml 存储的 `pos` 一同打印，
    /// 以便任何偏移都能产生一份可直接粘贴的脚本校正表。
    #[test]
    fn water_arc_angles_match_the_v2_water_script_poses() {
        // 从 `setup_material_showcase` 镜像的布局常量。
        let n = 10_f32;
        let arc_radius = 1.9_f32;
        let total_span_deg = 150.0_f32;
        let angle_deg = |i: f32| -> f32 {
            let t = i / (n - 1.0);
            -total_span_deg / 2.0 + t * total_span_deg
        };

        // (槽位, 精确解析度数, v2_water.toml 记录的度数,
        //  半径 3.5 处 toml 存储的相机 pos [x, z])。
        let cases = [
            (7_f32, 41.6667, 41.67, [2.326_8, 2.614_5]),
            (8.0, 58.3333, 58.33, [2.978_9, 1.839_3]),
            (9.0, 75.0, 75.00, [3.380_7, 0.905_8]),
        ];

        for (slot, exact, documented, toml_pos) in cases {
            let got = angle_deg(slot);
            // 布局公式的紧密自洽性。
            assert!(
                (got - exact).abs() < 1e-3,
                "arc formula drift at slot {slot}: got {got:.6}°, expected {exact:.6}°"
            );
            // 对 toml 记录度数的取整感知匹配。
            assert!(
                (got - documented).abs() < 5e-3,
                "slot {slot} angle {got:.4}° no longer matches v2_water.toml's {documented}°"
            );
            // 打印相机姿态（半径 3.5）与 toml 存储的 pos 的对比，以便偏移
            // 产生一份校正表。
            let theta = got.to_radians();
            let cam = Vec3::new(3.5 * theta.sin(), 0.0, 3.5 * theta.cos());
            let sphere = Vec3::new(arc_radius * theta.sin(), 0.0, arc_radius * theta.cos());
            println!(
                "slot {slot:.0}: angle={got:.4}°  sphere_pos=({:.4}, 0, {:.4})  \
                 camera=({:.4}, {:.4}, {:.4})  toml_pos=({:.4}, 0, {:.4})",
                sphere.x, sphere.z, cam.x, cam.y, cam.z,
                toml_pos[0], toml_pos[1],
            );
        }
    }
}
