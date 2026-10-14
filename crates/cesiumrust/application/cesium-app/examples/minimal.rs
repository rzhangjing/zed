//! 最小 CesiumRust 示例：地球 + 相机 + 一个地面点。
//!
//! 这演示了使用六边形架构插件栈的最简单 CesiumRust 应用
//! （底球面 + 领域相机）。
//!
//! 历史沿革（两次裁剪都见 docs/SCOPE_2D3D_IMAGERY_PLOT.md）：
//! - 2026-09 P0 观感清理：原先还注册 `CesiumEntityPlugin` 并用
//!   `cesium_scene_mode::SceneMode` 显式指定场景模式；entity/DataSource
//!   可视化路线已删除，`CesiumCamera` 自带 `Default`（Scene3D），
//!   故这里改用 `..default()`，本示例不再需要 `cesium-scene-mode` 依赖。
//! - 2026-09 P1 裁剪：`CesiumImageryPlugin`（`cesium_bevy_render::imagery`，
//!   地形垂覆影像链）与 `CesiumTerrainTile` 组件已随 `cesium-terrain` /
//!   `cesium-tileset` / `cesium-gltf` / `cesium-styling` 一并删除。真正的
//!   默认底球面影像链（Bing/OSM 瓦片、SSE 四叉树 LOD、GPU 句柄 LRU 缓存）
//!   位于本 package 的 binary 模块 `src/dynamic_globe*`（渲染主路径
//!   Path A），不属于 `cesium-bevy-render` 的公开 API，因此本示例只演示
//!   球体 + 相机 + 一个点。

// legacy CesiumJS-port style debt (deferred.md #18); revisit at M13 lint-cleanup 或本文件在其里程碑被重写时
#![allow(unused_imports)]
use bevy::prelude::*;
use cesium_bevy_render::CesiumCorePlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "CesiumRust — Minimal Example".into(),
                resolution: (1280.0_f32, 720.0_f32).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(CesiumCorePlugin)
        .add_systems(Startup, spawn_scene)
        .run();
}

fn spawn_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut globe_config: ResMut<cesium_bevy_render::GlobeConfig>,
) {
    use cesium_bevy_render::{
        create_ellipsoid_mesh, CesiumGlobe, METERS_PER_RENDER_UNIT,
    };
    use cesium_camera::Camera as DomainCamera;
    use cesium_geospatial::cartographic::Cartographic;
    use cesium_geospatial::ellipsoid::Ellipsoid;
    use glam::{DVec3, Vec3};

    let scale = (1.0 / METERS_PER_RENDER_UNIT) as f32;

    // ── 地球 ──────────────────────────────────────────────────
    globe_config.ellipsoid = Ellipsoid::WGS84;

    let globe_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.18, 0.35),
        perceptual_roughness: 0.85,
        ..default()
    });
    commands.spawn((
        CesiumGlobe,
        Mesh3d(meshes.add(create_ellipsoid_mesh(32, 64))),
        MeshMaterial3d(globe_material),
        Transform::from_scale(Vec3::splat(scale)),
    ));

    // ── 相机 ─────────────────────────────────────────────────
    let cam_pos = {
        let c = Cartographic::from_degrees(-95.0, 40.0, 20_000_000.0);
        Ellipsoid::WGS84.cartographic_to_cartesian(&c)
    };
    let target = {
        let c = Cartographic::from_degrees(-95.0, 40.0, 0.0);
        Ellipsoid::WGS84.cartographic_to_cartesian(&c)
    };
    let dir = (target - cam_pos).normalize();
    let up = cam_pos.normalize();
    let camera = DomainCamera::new(cam_pos, dir, up);

    commands.spawn((
        cesium_bevy_render::CesiumCamera {
            camera,
            enable_collision_detection: true,
            minimum_zoom_distance: 100.0,
            maximum_zoom_distance: 20_000_000.0,
            // scene_mode 取 `CesiumCamera::default()` 的 Scene3D —— 本示例
            // 不再直接命名 `cesium_scene_mode::SceneMode`（见文件头说明）。
            ..default()
        },
        Camera3d::default(),
        Transform::from_translation(cam_pos.as_vec3() * scale)
            .looking_at(target.as_vec3() * scale, up.as_vec3()),
    ));

    // ── 测试点：纽约 ─────────────────────────────────────────
    let ny = ellipsoid_point(-74.006, 40.7128, 1000.0);
    let point_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.0, 0.0),
        emissive: LinearRgba::rgb(50.0, 0.0, 0.0),
        unlit: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(0.0015))),
        MeshMaterial3d(point_mat),
        Transform {
            translation: ny * scale,
            scale: Vec3::splat(3.0),
            ..default()
        },
    ));

    println!("[Minimal] Globe + camera + NY point spawned");
}

fn ellipsoid_point(lon_deg: f64, lat_deg: f64, height: f64) -> Vec3 {
    use cesium_geospatial::cartographic::Cartographic;
    use cesium_geospatial::ellipsoid::Ellipsoid;
    let carto = Cartographic::from_degrees(lon_deg, lat_deg, height);
    let ecef = Ellipsoid::WGS84.cartographic_to_cartesian(&carto);
    Vec3::new(ecef.x as f32, ecef.y as f32, ecef.z as f32)
}
