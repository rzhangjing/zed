//! cesium-bevy-render：Bevy 渲染适配器
//!
//! 本适配器实现 GpuSink port，将领域几何（f64）转换为 Bevy mesh
//! （f32）以供 GPU 渲染。
//!
//! # 架构
//! - `mesh_conversion`：GeometryData → Bevy Mesh（f64 → f32 精度边界）
//! - `ellipsoid_mesh`：WGS84 椭球 mesh 生成
//! - `plugin`：面向 CesiumRust 渲染的 Bevy 插件

pub mod camera;
pub mod clock;
pub mod components;
pub mod fabric_material;
pub mod material_system;
pub mod resources;
pub mod shader_registry;
pub mod atmosphere;
pub mod shadow;
pub mod pipeline;
pub mod headless;

// 2026-09 P1 裁剪：`terrain` / `tileset` / `imagery` 三个模块整体删除 ——
// 地形高程链（Path C）与 3D Tiles 路线不属于「2D/3D 地球影像 + 标绘」目标，
// 且 `imagery` 只服务 `terrain` 的垂覆纹理（其每个 system 都查询
// `CesiumTerrainTile`），删地形即失去锚点。底球面影像由 `dynamic_globe`
// 薄壳（Path A）拥有，不经过本适配器。

pub use camera::{
    camera_control_port_system, CameraControlImpl, CameraControlPort, CameraState, CesiumCamera,
    CesiumCameraPlugin, FlyToRequest,
};
pub use components::CesiumGlobe;
// 2026-09 P0 裁剪：CesiumJS 的 Entity/DataSource 可视化路线整条移除
// （`entity/`、`entity_render.rs`、`datasource/` 及其 CZML/GeoJSON/KML/GPX
// 四个 loader）。它与 `cesium-plot` 是两套并行的标绘数据结构，而本项目的
// 标绘走 `cesium-plot` + `cesium-plot-bevy`；`CesiumEntityPlugin` /
// `CesiumDataSourcePlugin` 从未被 `cesium-app` 注册。
// 仅 `AnimationClock` 被保留（大气/天体系统需要它），现已迁到 `crate::clock`。
pub use clock::AnimationClock;
pub use fabric_material::{FabricKind, FabricMaterial, FabricMaterialPlugin, FabricParams};
pub use material_system::{
    CesiumMaterialPlugin, MaterialAnimationTime, MaterialRef, MaterialSystemResource,
};
pub use resources::{GlobeConfig, RenderScale, TileLoadStats, METERS_PER_RENDER_UNIT};

pub use atmosphere::CesiumAtmospherePlugin;
// 2026-09 P0 裁剪：`effects` / `voxel` / `vector` / `widgets` 四个适配器模块
// 连同其领域 crate 一并删除（复杂观感与 3D Tiles 之外的展示能力，本目标
// 「2D/3D 地球影像 + 标绘」不需要）。原先在此处的 `pub use` 也随之移除。
pub use shadow::{CesiumShadowPlugin, ShadowConfig, ShadowState, ShadowCaster};
pub use pipeline::CesiumPipelinePlugin;

use bevy::pbr::DirectionalLightShadowMap;

use bevy::prelude::*;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::geometry::{self, GeometryData, PrimitiveType, VertexFormat};
use cesium_imagery::blending::PixelColor;

/// 将 f64 GeometryData 转换为 Bevy Mesh（f32 精度边界）
///
/// # RTC（Relative-To-Center）精度桥接
/// `rtc_center` 为 `Some(c)` 时，在截断为 f32 **之前以 f64** 将每个顶点
/// 位置平移 `-c`，生成一个 tile-local mesh，其坐标即使对地球尺度的
/// ECEF 输入也保持足够小以保留 f32 精度。这是标准的
/// CesiumJS/3D Tiles “RTC center” 技巧，应用在本适配器唯一的
/// f64→f32 精度边界上。
///
/// 当为 `None` 时，行为与之前的签名逐字节一致：
/// 位置直接从 f64 转为 f32，不应用任何偏移。
///
/// DEVIATION：公共签名新增 rtc_center: Option<DVec3>（非 additive）；
/// 参见 docs/deviations.md#dev-013
pub fn geometry_to_mesh(geometry: &GeometryData, rtc_center: Option<glam::DVec3>) -> Mesh {
    // 转换位置：f64 → f32，可先在 f64 中重新中心化（见上方文档）
    let positions: Vec<[f32; 3]> = geometry
        .positions
        .iter()
        .map(|p| match rtc_center {
            Some(c) => [(p[0] - c.x) as f32, (p[1] - c.y) as f32, (p[2] - c.z) as f32],
            None => [p[0] as f32, p[1] as f32, p[2] as f32],
        })
        .collect();

    // 转换法线：f64 → f32（若存在）
    let normals: Vec<[f32; 3]> = geometry
        .normals
        .as_ref()
        .map(|n| n.iter().map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]).collect())
        .unwrap_or_default();

    // 转换纹理坐标：f64 → f32（若存在）
    let uvs: Vec<[f32; 2]> = geometry
        .tex_coords
        .as_ref()
        .map(|t| t.iter().map(|v| [v[0] as f32, v[1] as f32]).collect())
        .unwrap_or_default();

    // 索引已是 u32
    let indices = geometry.indices.clone();

    // 基于图元类型确定拓扑。
    let topology = match geometry.primitive_type {
        PrimitiveType::Triangles => bevy::render::mesh::PrimitiveTopology::TriangleList,
        PrimitiveType::Lines => bevy::render::mesh::PrimitiveTopology::LineList,
    };

    let mut mesh = Mesh::new(
        topology,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );

    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    if !normals.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    }
    if !uvs.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    }
    if !indices.is_empty() {
        mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));
    }

    mesh
}

/// 以给定细分数生成一个 WGS84 椭球 mesh
///
/// # 参数
/// * `stacks` - 纬度细分数（默认：64）
/// * `slices` - 经度细分数（默认：128）
///
/// # 返回
/// 一个表示 WGS84 椭球的 Bevy Mesh
pub fn create_ellipsoid_mesh(stacks: u32, slices: u32) -> Mesh {
    let radii = Ellipsoid::WGS84.radii();
    let geometry = geometry::ellipsoid_geometry(radii, stacks, slices, VertexFormat::ALL);
    // 完整椭球 mesh 已以原点为中心；无需 RTC 偏移。
    geometry_to_mesh(&geometry, None)
}

// 2026-09 P1 裁剪：`terrain_mesh_to_bevy(terrain: &TerrainMesh, rtc_center:
// Option<glam::DVec3>) -> Mesh` 随 `cesium-terrain` 一同删除（其唯一调用方是
// 已删的 `src/terrain/render_system.rs`，且 `TerrainMesh` 类型不再存在于
// 工作区）。整颗地球仍由 `create_ellipsoid_mesh()` 表示。

/// 从原始 RGBA 像素数据创建一个 Bevy Image，用作影像纹理。
///
/// # 参数
/// * `width` - 图像宽度（像素）
/// * `height` - 图像高度（像素）
/// * `rgba_data` - 原始 RGBA 像素数据（每像素 4 字节）
///
/// # 返回
/// 一个 Bevy Image 资产
pub fn create_imagery_texture(width: u32, height: u32, rgba_data: Vec<u8>) -> Image {
    Image::new(
        bevy::render::render_resource::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        rgba_data,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::render::render_asset::RenderAssetUsages::default(),
    )
}

/// 从 PixelColor 创建纯色纹理（用于测试/回退）。
pub fn create_solid_color_texture(color: PixelColor, size: u32) -> Image {
    let r = (color.r.clamp(0.0, 1.0) * 255.0) as u8;
    let g = (color.g.clamp(0.0, 1.0) * 255.0) as u8;
    let b = (color.b.clamp(0.0, 1.0) * 255.0) as u8;
    let a = (color.a.clamp(0.0, 1.0) * 255.0) as u8;

    let pixel = [r, g, b, a];
    let data: Vec<u8> = pixel.iter().cycle().take((size * size * 4) as usize).copied().collect();

    create_imagery_texture(size, size, data)
}

/// 创建一个线框盒 mesh，用于可视化包围体。
///
/// # 参数
/// * `center` - 盒子中心（ECEF 坐标）
/// * `half_x` - X 方向的半轴向量
/// * `half_y` - Y 方向的半轴向量
/// * `half_z` - Z 方向的半轴向量
///
/// # 返回
/// 一个以线拓扑表示盒子棱边的 Bevy Mesh
pub fn create_bounding_box_wireframe(
    center: glam::DVec3,
    half_x: glam::DVec3,
    half_y: glam::DVec3,
    half_z: glam::DVec3,
) -> Mesh {
    // 盒子的 8 个角
    let corners: Vec<[f32; 3]> = vec![
        (center - half_x - half_y - half_z).as_vec3().into(),
        (center + half_x - half_y - half_z).as_vec3().into(),
        (center + half_x + half_y - half_z).as_vec3().into(),
        (center - half_x + half_y - half_z).as_vec3().into(),
        (center - half_x - half_y + half_z).as_vec3().into(),
        (center + half_x - half_y + half_z).as_vec3().into(),
        (center + half_x + half_y + half_z).as_vec3().into(),
        (center - half_x + half_y + half_z).as_vec3().into(),
    ];

    // 12 条棱（line list 用 24 个索引）
    let indices: Vec<u32> = vec![
        // 底面
        0, 1, 1, 2, 2, 3, 3, 0,
        // 顶面
        4, 5, 5, 6, 6, 7, 7, 4,
        // 竖直棱
        0, 4, 1, 5, 2, 6, 3, 7,
    ];

    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::LineList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, corners);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));
    mesh
}

/// 创建一个线框球 mesh，用于可视化包围球。
///
/// # 参数
/// * `center` - 球心（ECEF 坐标）
/// * `radius` - 球的半径
/// * `segments` - 每个圆的细分数
///
/// # 返回
/// 一个以线拓扑表示球线框的 Bevy Mesh
pub fn create_bounding_sphere_wireframe(
    center: glam::DVec3,
    radius: f64,
    segments: u32,
) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let center_f32 = center.as_vec3();
    let radius_f32 = radius as f32;

    // 创建 3 个圆（XY、XZ、YZ 平面）
    for plane in 0..3 {
        let base_index = positions.len() as u32;
        for i in 0..segments {
            let angle = 2.0 * std::f32::consts::PI * (i as f32) / (segments as f32);
            let (sin, cos) = angle.sin_cos();

            let pos = match plane {
                0 => glam::Vec3::new(cos * radius_f32, sin * radius_f32, 0.0), // XY
                1 => glam::Vec3::new(cos * radius_f32, 0.0, sin * radius_f32), // XZ
                _ => glam::Vec3::new(0.0, cos * radius_f32, sin * radius_f32), // YZ
            };

            positions.push((center_f32 + pos).into());

            // 连到下一个顶点（环绕）
            let next = if i == segments - 1 { base_index } else { base_index + i + 1 };
            indices.push(base_index + i);
            indices.push(next);
        }
    }

    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::LineList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));
    mesh
}

/// 光照模式选择器 —— 决定 `setup_lighting` 生成哪套光照装置。由应用层
/// （main.rs）在 `CesiumCorePlugin` 构建之前作为 Bevy `Resource` 插入，
/// 以便 Startup 系统能读取它。
///
/// * `FullAmbient` —— 仅均匀环境光（CesiumJS `enableLighting=false`
///   的外观）。这是**默认值**，产生与 M4.1 之前基线像素一致的输出
///   （PSNR=∞）。
/// * `DayNight` —— 环境光 + 带阴影贴图的方向光太阳，
///   由 `celestial_system` 驱动（compute_sun_direction_eci）。
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LightingMode {
    /// 仅均匀环境光照明（v0 基线，零差异）。
    #[default]
    FullAmbient,
    /// 方向光太阳 + 环境光；激活 shadow + atmosphere 插件。
    DayNight,
}

/// 初始化 CesiumRust 核心 Bevy 资源（GlobeConfig、RenderScale、
/// TileLoadStats、AnimationClock）并设置场景光照的插件。
pub struct CesiumCorePlugin;

impl Plugin for CesiumCorePlugin {
    /// 初始化核心渲染资源（GlobeConfig/RenderScale/TileLoadStats 等）并设置场景光照。
    ///
    /// # 参数
    /// - `app`：Bevy 应用
    fn build(&self, app: &mut App) {
        app.init_resource::<GlobeConfig>()
            .init_resource::<RenderScale>()
            .init_resource::<TileLoadStats>()
            .init_resource::<LightingMode>()
            // M4.1：将 AnimationClock 提升至 core，使 celestial_system 能
            // 无需依赖 CesiumEntityPlugin 即可读取它。
            .init_resource::<AnimationClock>()
            .add_systems(Startup, setup_lighting);
    }
}

/// 生成场景光照的系统，依据 [`LightingMode`] 分支。
///
/// ## FullAmbient（默认 —— v0 零差异）
///
/// 仅均匀环境光照明，无方向光太阳：CesiumJS 的默认 globe 以
/// `enableLighting = false` 运行，即整颗星球渲染为全白昼、无昼夜分界线，
/// 我们在此匹配该外观。
///
/// 亮度校准：PBR 环境光管线对 `brightness` 施加一个经验性的
/// ~3.18e-4 物理单位缩放（实测：10 000 会过曝成约 3 倍的泛白，2.2 渲染为全黑），
/// 因此 `1 / 3.18e-4 ~= 3300` 以约 1:1 复现影像反照率 —— CesiumJS 如实显示瓦片。
///
/// 色温：参考的 CesiumJS 全球外观偏冷调（鼠尾草色陆地、中等钢蓝色海洋），
/// 而原始 Bing 反照率渲染偏暖且海洋近乎全黑；一抹冷色偏移（削减红、增强蓝）
/// 加上轻微的亮度提升，将白平衡推向参考效果。
///
/// ## DayNight（M4.1）
///
/// 环境光（降低）+ 一盏照度 10 000 lx 的 `DirectionalLight`，
/// 并启用 Bevy 内置的 `DirectionalLightShadowMap`（4 级联，2048 px）。
/// 光照方向由 `celestial_system` 逐帧驱动
/// （compute_sun_direction_eci → look_to）。阴影级联计算位于
/// `shadow_update_system`，它查询 DirectionalLight 的 transform。
///
/// 近/远以渲染单位计：0.01 / 100.0（× METERS_PER_RENDER_UNIT =
/// 63 781 米 … 637 813 700 米）—— 覆盖从 LEO 到地月空间，且在地球表面
/// 无 z-fighting。
fn setup_lighting(mut commands: Commands, mode: Res<LightingMode>) {
    match *mode {
        LightingMode::FullAmbient => {
            // v0 基线：仅环境光，与 M4.1 之前像素一致。
            commands.insert_resource(AmbientLight {
                color: Color::srgb(0.79, 0.94, 1.17),
                brightness: 3800.0,
            });
        }
        LightingMode::DayNight => {
            // 降低环境光，使方向光太阳主导昼侧。
            commands.insert_resource(AmbientLight {
                color: Color::srgb(0.79, 0.94, 1.17),
                brightness: 1200.0,
            });

            // 带阴影贴图的方向光太阳。
            commands.spawn((
                DirectionalLight {
                    illuminance: 10_000.0,
                    shadows_enabled: true,
                    ..default()
                },
                // 默认 transform；celestial_system 会逐帧为其定向。
                Transform::IDENTITY,
            ));

            // Bevy 内置阴影贴图配置（4 级联，2048 分辨率）。
            commands.insert_resource(DirectionalLightShadowMap {
                size: 2048,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_geospatial::bounding::BoundingSphere;

    #[test]
    fn test_geometry_to_mesh() {
        let radii = Ellipsoid::WGS84.radii();
        let geometry = geometry::ellipsoid_geometry(radii, 8, 16, VertexFormat::ALL);
        let mesh = geometry_to_mesh(&geometry, None);

        // 验证 mesh 具有 position 属性
        assert!(mesh.attribute(Mesh::ATTRIBUTE_POSITION).is_some());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_NORMAL).is_some());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_UV_0).is_some());
    }

    #[test]
    fn test_geometry_to_mesh_rtc_center_none_matches_legacy() {
        let geometry = GeometryData {
            positions: vec![[1.0, 2.0, 3.0], [-4.0, 5.5, -6.25]],
            normals: None,
            tex_coords: None,
            tangents: None,
            bitangents: None,
            indices: vec![0, 1, 0],
            primitive_type: PrimitiveType::Triangles,
            bounding_sphere: BoundingSphere::new(glam::DVec3::ZERO, 10.0),
        };

        let mesh = geometry_to_mesh(&geometry, None);
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
        if let bevy::render::mesh::VertexAttributeValues::Float32x3(pos) = positions {
            assert_eq!(pos[0], [1.0_f32, 2.0_f32, 3.0_f32]);
            assert_eq!(pos[1], [-4.0_f32, 5.5_f32, -6.25_f32]);
        } else {
            panic!("Expected Float32x3 positions");
        }
    }

    #[test]
    fn test_geometry_to_mesh_rtc_center_some_recenters_in_f64() {
        let geometry = GeometryData {
            positions: vec![[1.0, 2.0, 3.0], [-4.0, 5.5, -6.25]],
            normals: None,
            tex_coords: None,
            tangents: None,
            bitangents: None,
            indices: vec![0, 1, 0],
            primitive_type: PrimitiveType::Triangles,
            bounding_sphere: BoundingSphere::new(glam::DVec3::ZERO, 10.0),
        };

        let center = glam::DVec3::new(1.0, 2.0, 3.0);
        let mesh = geometry_to_mesh(&geometry, Some(center));
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
        if let bevy::render::mesh::VertexAttributeValues::Float32x3(pos) = positions {
            assert_eq!(pos[0], [0.0_f32, 0.0_f32, 0.0_f32]);
            assert_eq!(pos[1], [-5.0_f32, 3.5_f32, -9.25_f32]);
        } else {
            panic!("Expected Float32x3 positions");
        }
    }

    #[test]
    fn test_create_ellipsoid_mesh() {
        let mesh = create_ellipsoid_mesh(16, 32);
        assert!(mesh.attribute(Mesh::ATTRIBUTE_POSITION).is_some());
    }

    #[test]
    fn test_create_imagery_texture() {
        let data = vec![255u8; 4 * 4 * 4]; // 4x4 RGBA
        let image = create_imagery_texture(4, 4, data);
        assert_eq!(image.width(), 4);
        assert_eq!(image.height(), 4);
    }

    #[test]
    fn test_create_solid_color_texture() {
        let color = PixelColor::opaque(1.0, 0.0, 0.0);
        let image = create_solid_color_texture(color, 2);
        assert_eq!(image.width(), 2);
        assert_eq!(image.height(), 2);
        // 第一个像素应为红色 (255, 0, 0, 255)
        assert_eq!(image.data[0], 255);
        assert_eq!(image.data[1], 0);
        assert_eq!(image.data[2], 0);
        assert_eq!(image.data[3], 255);
    }

    #[test]
    fn test_create_bounding_box_wireframe() {
        let center = glam::DVec3::ZERO;
        let half_x = glam::DVec3::new(1.0, 0.0, 0.0);
        let half_y = glam::DVec3::new(0.0, 1.0, 0.0);
        let half_z = glam::DVec3::new(0.0, 0.0, 1.0);

        let mesh = create_bounding_box_wireframe(center, half_x, half_y, half_z);

        // 应有 8 个顶点（角点）
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
        if let bevy::render::mesh::VertexAttributeValues::Float32x3(pos) = positions {
            assert_eq!(pos.len(), 8);
        } else {
            panic!("Expected Float32x3 positions");
        }
    }

    #[test]
    fn test_create_bounding_sphere_wireframe() {
        let center = glam::DVec3::ZERO;
        let radius = 100.0;
        let segments = 32;

        let mesh = create_bounding_sphere_wireframe(center, radius, segments);

        // 应有 3 个圆 * segments 个顶点
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
        if let bevy::render::mesh::VertexAttributeValues::Float32x3(pos) = positions {
            assert_eq!(pos.len(), (3 * segments) as usize);
        } else {
            panic!("Expected Float32x3 positions");
        }
    }
}
