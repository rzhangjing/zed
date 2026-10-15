//! cesium-app：CesiumRust 3D 地球查看器
//!
//! 交互式 3D 地球，包含：
//! - 基础球体 + 极地盖片（无 LOD 的兜底安全网）
//! - 带 Bing Maps 卫星影像的动态 LOD 瓦片
//! - Orbit 相机（鼠标拖拽旋转，滚轮缩放）
//! - 大气边缘辉光 + 星空背景
//!
//! # 作为库使用（本文件存在 `lib.rs` 形态的理由）
//!
//! 本 crate 原先只有 `src/main.rs` 一个 target。凡是需要**这个查看器的
//! 同一套场景**、却自己拥有事件循环与渲染目标的宿主，都无法调用 `main`，
//! 只能退回到 `examples/minimal.rs` 那套最小场景 —— 于是窗口里只剩一个
//! 没有影像的纯色球。Zed 的 `Help > cesiumrust` 窗口正是这样的宿主：
//! 专用线程上的 Bevy `App`、禁用 `WinitPlugin`、离屏
//! `RenderTarget::Image` + `Screenshot` 回读。
//!
//! 现在本文件既是 bin 的入口（经 [`run`]）也是 lib 的根：
//! - [`add_viewer_scene`] 是 `main` 与嵌入方**共用的唯一一份**场景装配
//!   （基础球 / 动态影像瓦片 LOD / orbit 相机 / 大气辉光 / 星空 / 门控插件），
//!   并公开了原本私有的模块（`pub mod`）；
//! - [`run`] 是 `main.rs` 原来的入口体，逐字保留，只在场景装配处调用
//!   [`add_viewer_scene`]。
//!
//! 嵌入方仍需自己处理两件与渲染目标有关的事，本 crate 不代为决定：
//! 相机实体上的 `Msaa` 必须与离屏目标的 `sample_count` 一致，且
//! `App::finish()` / `App::cleanup()` 必须由驱动方在首帧前手工调用。

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
// P1 裁剪（2026-09，范围见 docs/SCOPE_2D3D_IMAGERY_PLOT.md）：原先这里还有
// `bevy::tasks::futures_lite::future::{block_on, poll_once}` 与
// `bevy::tasks::{IoTaskPool, Task}` —— 它们只服务本文件内的 Cesium Ion
// 地形端点异步解析，已随地形链一起删除。
use cesium_bevy_render::{
    CesiumCorePlugin, CesiumGlobe,
    CameraControlPort, camera_control_port_system,
    LightingMode, CesiumAtmospherePlugin, CesiumShadowPlugin,
};
// 原本是 `mod`（bin 私有）。改为 `pub mod` 使嵌入方能拿到与 `main` 相同的
// 场景模块 —— 这是本次改动的全部目的；各模块内部仍按原样使用
// `pub(crate)` 保持封装，未改动任何一条业务逻辑。
pub mod orbit_camera;
pub mod starfield;
pub mod atmosphere_glow;
pub mod tile_mesh;
pub mod base_sphere;
pub mod globe_lod;
pub mod globe_textures;
pub mod globe_pipeline;
pub mod tile_cache;
pub mod dynamic_globe;
pub mod feature_flags;
pub mod perf_counters;
pub mod perf_trace;
pub mod capture_script;
pub mod offline_check;
pub mod map2d;
pub mod material_showcase;

use orbit_camera::OrbitCameraPlugin;
use material_showcase::MaterialShowcasePlugin;
use starfield::StarfieldPlugin;
use atmosphere_glow::AtmosphereGlowPlugin;
use base_sphere::BaseSphereMarker;
use tile_mesh::{create_mercator_uv_sphere, create_polar_cap, render_scale};
use feature_flags::{lighting_mode, skydome_enabled, glow_enabled, material_showcase_enabled};
use perf_counters::PerfCounters;
use perf_trace::PerfTracePlugin;
use bevy::time::TimeUpdateStrategy;
use serde::Deserialize;
// P0 观感清理（范围见 docs/SCOPE_2D3D_IMAGERY_PLOT.md）：原先这里从
// `cesium_bevy_render::effects` 导入 M6 Wave A 的相机组件与
// `M6WaveARenderGraphPlugin`（Clipping / Panorama / IBL / OIT / Clouds / Split）。
// 这些复杂观感能力的目标不需要，其领域 crate `cesium-effects` 与适配器模块
// `adapters/bevy-render/src/effects` 均已删除，故导入一并移除。

const TILE_SEGMENTS: u32 = 16;
/// 基础球（回退网格）的半径比例：**刚好**低于地表，以免与瓦片层共面，
/// 同时必须仍位于极地盖片之下（`tile_mesh::create_polar_cap` 的
/// `CAP_TUCK` = 1 − 1e-5 ≈ 64 m），故取 1 − 4e-5 ≈ 256 m。
///
/// 旧值为 0.99（≈63.8 km）：贴地表时任何瓦片缺口都会被读成深不见底的
/// 空洞（回退网格的纯色/合成纹理在脚下 64 km 处）。而 reversed-Z f32
/// 深度的相对分辨率约 2⁻²⁴，最远视距（20 render unit）上约 7.7 m，
/// 256 m 与 64 m 的层间余量都远超它，不会深度冲突。
const BASE_SPHERE_SCALE: f32 = 1.0 - 4.0e-5;
/// 无头验证辅助工具，除非设置了 `CESIUM_SCREENSHOT_AT_FRAME` 否则不生效：
/// 在该帧捕获一张截图并在紧随其后退出。让审阅者无需改变默认
/// 交互式运行就能抓取一个确定性帧（默认或 opt-in 链）。
#[derive(Resource)]
struct AutoScreenshot {
    /// 已推进的帧计数。
    frame: u32,
    /// 触发捕获的目标帧号。
    at_frame: u32,
    /// 截图输出路径模板。
    path: String,
}

/// 达到目标帧时抓取一张截图并在紧随其后请求退出。
fn auto_screenshot_system(
    mut commands: Commands,
    mut shot: ResMut<AutoScreenshot>,
    mut exit: EventWriter<AppExit>,
    cameras: Query<(&Transform, &Projection), With<Camera>>,
) {
    shot.frame += 1;
    if shot.frame == shot.at_frame {
        let path = shot.path.clone();
        // FIX-HL-EXIT：在 CESIUM_HEADLESS 下没有主窗口，因此
        // `Screenshot::primary_window()` 不生效（不会触发任何 capture observer）。使
        // 那个静默的空缺变得响亮；受支持的无头产物是 M11.3
        // 离屏 `CesiumHeadlessPlugin` 捕获路径。
        if feature_flags::headless_enabled() {
            warn!(
                "[shot] AutoScreenshot uses Screenshot::primary_window(), inert under \
                 CESIUM_HEADLESS; rely on the headless offscreen capture for the PNG"
            );
        }
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
        info!("[shot] capturing at frame {}", shot.at_frame);

        // M3：在截图旁写入相机 pos/quat JSON。
        // Schema: {"pos":[x,y,z],"quat":[x,y,z,w],"fov_y":<deg>}
        // 这是 M2.4 回放保真度的参考基准（<1e-4 render unit）。
        if let Ok((transform, projection)) = cameras.get_single() {
            let p = transform.translation;
            let q = transform.rotation;
            let fov_y_deg = match projection {
                Projection::Perspective(pp) => pp.fov.to_degrees(),
                Projection::Orthographic(_) => 0.0,
            };
            let json = format!(
                "{{\"pos\":[{:.6},{:.6},{:.6}],\"quat\":[{:.6},{:.6},{:.6},{:.6}],\"fov_y\":{:.4}}}",
                p.x, p.y, p.z,
                q.x, q.y, q.z, q.w,
                fov_y_deg,
            );
            // 从截图路径推导 .camera.json 路径。
            let cam_path = std::path::Path::new(&path)
                .with_extension("camera.json");
            if let Err(e) = std::fs::write(&cam_path, &json) {
                warn!("[shot] failed to write camera JSON {}: {}", cam_path.display(), e);
            } else {
                info!("[shot] camera state written: {}", cam_path.display());
            }
        }
    }
    if shot.frame == shot.at_frame + 30 {
        exit.send(AppExit::Success);
    }
}

// ── M3.3：FIXED_CAMERA —— 确定性位姿覆盖 ──────────────────

/// `FIXED_CAMERA` 文件的 TOML schema：单个 `[camera]` 表。
#[derive(Debug, Deserialize)]
struct FixedCameraFile {
    /// 单一 `[camera]` 表解析出的固定相机位姿。
    camera: capture_script::CameraPose,
}

/// 保存一个每帧 PostUpdate 都应用的冻结相机位姿的资源。
#[derive(Resource)]
struct FixedCameraPose {
    /// 冻结的相机世界位置。
    pos: Vec3,
    /// 冻结的相机旋转四元数。
    quat: Quat,
    /// 可选的垂直 FOV（度）；`None` 时保留默认。
    fov_y: Option<f32>,
}

/// PostUpdate 系统：用固定位姿覆盖 orbit 相机。
/// 在 orbit_camera 的 Update 之后运行，因此确定性位姿胜出。
fn fixed_camera_system(
    pose: Res<FixedCameraPose>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<Camera>>,
) {
    if let Ok((mut transform, mut projection)) = cameras.get_single_mut() {
        transform.translation = pose.pos;
        transform.rotation = pose.quat;
        if let Some(fov_deg) = pose.fov_y {
            if let Projection::Perspective(ref mut pp) = *projection {
                pp.fov = fov_deg.to_radians();
            }
        }
    }
}

// ── M3.4：批量截图捕获 ─────────────────────────

/// 驱动批量多视图捕获（`CESIUM_SCREENSHOT_SCRIPT`）的资源。
#[derive(Resource)]
struct BatchCapture {
    /// 脚本中待捕获的 shot 列表。
    script: Vec<capture_script::ShotEntry>,
    /// 当前推进到的 shot 下标。
    cursor: usize,
    /// 已推进的帧计数。
    frame: u32,
    /// 截图与元数据的输出目录。
    output_dir: String,
    /// 写入元数据的当前 git SHA。
    git_sha: String,
    /// 冻结的环境变量快照，随每张截图一起写入。
    env_snapshot: serde_json::Value,
}

/// PostUpdate 系统：推进批量捕获调度器。应用到期
/// shot 的相机位姿，spawn 一个 `Screenshot`，写入 `.camera.json` +
/// `.meta.json`，并在所有 shot + 30 帧缓冲帧之后退出。
fn batch_capture_system(
    mut commands: Commands,
    mut batch: ResMut<BatchCapture>,
    mut exit: EventWriter<AppExit>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<Camera>>,
) {
    batch.frame += 1;

    if let Some(idx) = capture_script::next_due(&batch.script, batch.cursor, batch.frame) {
        let entry = batch.script[idx].clone();

        // 应用脚本化的相机位姿（本帧覆盖 orbit_camera）。
        if let Ok((mut transform, mut projection)) = cameras.get_single_mut() {
            transform.translation = Vec3::new(
                entry.pos[0] as f32,
                entry.pos[1] as f32,
                entry.pos[2] as f32,
            );
            transform.rotation = Quat::from_xyzw(
                entry.quat[0] as f32,
                entry.quat[1] as f32,
                entry.quat[2] as f32,
                entry.quat[3] as f32,
            );
            if let Some(fov_deg) = entry.fov_y {
                if let Projection::Perspective(ref mut pp) = *projection {
                    pp.fov = (fov_deg as f32).to_radians();
                }
            }
        }

        // spawn 截图。
        let png_path = format!("{}/{}.png", batch.output_dir, entry.name);
        // FIX-HL-EXIT：批量捕获同样尚未无头感知（参见插件布线处的
        // M11.4 NOTE）；宁可告警也不要静默失败。
        if feature_flags::headless_enabled() {
            warn!(
                "[batch] Screenshot::primary_window() is inert under CESIUM_HEADLESS; \
                 batch capture is not yet headless-aware (deferred to M11.4)"
            );
        }
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(png_path));

        // 写入 .camera.json（与单 shot harness 同 schema）。
        let fov_y_deg = entry.fov_y.unwrap_or(60.0);
        let cam_json = format!(
            "{{\"pos\":[{:.6},{:.6},{:.6}],\"quat\":[{:.6},{:.6},{:.6},{:.6}],\"fov_y\":{:.4}}}",
            entry.pos[0], entry.pos[1], entry.pos[2],
            entry.quat[0], entry.quat[1], entry.quat[2], entry.quat[3],
            fov_y_deg,
        );
        let cam_path = format!("{}/{}.camera.json", batch.output_dir, entry.name);
        if let Err(e) = std::fs::write(&cam_path, &cam_json) {
            warn!("[batch] camera JSON write failed {}: {}", cam_path, e);
        }

        // 写入 .meta.json（供 pixel_diff 门控的 frame/env/git_sha）。
        let meta_json = capture_script::metadata_json(
            &entry,
            batch.frame,
            &batch.git_sha,
            &batch.env_snapshot,
        );
        let meta_path = format!("{}/{}.meta.json", batch.output_dir, entry.name);
        if let Err(e) = std::fs::write(&meta_path, &meta_json) {
            warn!("[batch] metadata write failed {}: {}", meta_path, e);
        }

        info!(
            "[batch] captured '{}' at frame {} ({}/{})",
            entry.name,
            batch.frame,
            idx + 1,
            batch.script.len()
        );
        batch.cursor = idx + 1;
    }

    // 在所有 shot 发射后 + 30 帧缓冲帧供 GPU flush 后退出。
    if batch.cursor >= batch.script.len() && !batch.script.is_empty() {
        let last_frame = batch.script.last().map_or(0, |e| e.frame);
        if batch.frame >= last_frame + 30 {
            info!(
                "[batch] all {} shots captured, exiting",
                batch.script.len()
            );
            exit.send(AppExit::Success);
        }
    }
}

/// 为批量捕获元数据构建 env 快照 JSON。
///
/// P1 注记：`offline_terrain_root` / `terrain_enabled` / `tileset_enabled`
/// 三个键在 P1 删除地形链与 3D Tiles 路线后**刻意保留** —— 它们读的仍是
/// `feature_flags` 的冻结注册表（见该模块文档），键集稳定是跨里程碑
/// 捕获元数据 diff 的前提；删除键会污染此前所有基线的对照。
fn build_env_snapshot() -> serde_json::Value {
    serde_json::json!({
        "strict_offline": feature_flags::strict_offline(),
        "offline_imagery_root": feature_flags::offline_imagery_root().map(|p| p.display().to_string()),
        "offline_terrain_root": feature_flags::offline_terrain_root().map(|p| p.display().to_string()),
        "fixed_time": feature_flags::fixed_time_enabled(),
        "fixed_camera": feature_flags::fixed_camera_path().map(|p| p.display().to_string()),
        "terrain_enabled": feature_flags::terrain_enabled(),
        "tileset_enabled": feature_flags::tileset_enabled(),
    })
}

/// spawn 基础球体与极地盖片的插件。
struct BaseSpherePlugin;

impl Plugin for BaseSpherePlugin {
    /// 插件装配入口：启动时生成基础球体与极地盖片。
    ///
    /// # 参数
    /// - `app`：Bevy 应用。
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_base_sphere);
    }
}

/// 生成基础球体（略小的兼容 composite 的 UV 球）与南/北极盖片。
fn spawn_base_sphere(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let scale = render_scale();

    // 基础球体 —— 高细分 UV 球体（Mercator 映射 V，因此运行时
    // 全球 composite 纹理能像瓦片层一样垂覆）；半径只比地表低
    // `BASE_SPHERE_SCALE`（≈256 m），既保持在瓦片与极地盖片之下，
    // 又让贴地表时的瓦片缺口读作"脚下的地面"而非深不见底的空洞。
    // 纯色只是 composite 前的兜底：一旦基础瓦片层到达，dynamic_globe
    // 就在其上垂覆一个模糊的地球 composite，使瞬时的空缺读作地球而非
    // 一片扁平的蓝色虚空。
    let base_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.15, 0.17, 0.19),
        perceptual_roughness: 1.0,
        ..default()
    });
    commands.spawn((
        CesiumGlobe,
        BaseSphereMarker,
        Mesh3d(meshes.add(create_mercator_uv_sphere(96, 48))),
        MeshMaterial3d(base_material),
        Transform::from_scale(Vec3::splat(scale * BASE_SPHERE_SCALE)),
    ));

    // 极地盖片 —— 北盖使用与受光海洋颜色匹配的钢蓝色，因此
    // 北极点无缝地延续周围的海面（参考：CesiumJS 全球观感）；
    // 南盖为冰白色，因为环绕南极的 85° 瓦片环是白色冰原，
    // 盖片必须延续它。盖片网格内部已按 `CAP_TUCK`（≈64 m）收拢到
    // 地表之下，位于瓦片层之下、基础球之上 ⇒ 有瓦片处瓦片胜出，
    // 极区无瓦片处由盖片兜底。
    let north_cap_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.15, 0.17, 0.19),
        perceptual_roughness: 0.95,
        ..default()
    });
    let south_cap_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.88, 0.92, 0.96),
        perceptual_roughness: 0.95,
        ..default()
    });
    for &north in &[true, false] {
        commands.spawn((
            CesiumGlobe,
            Mesh3d(meshes.add(create_polar_cap(north, TILE_SEGMENTS * 4))),
            MeshMaterial3d(if north {
                north_cap_material.clone()
            } else {
                south_cap_material.clone()
            }),
            Transform::from_scale(Vec3::splat(scale)),
        ));
    }
}

/// 嵌入方对场景装配的**显式**配置。
///
/// 存在的理由是「场景一样、但宿主不一样」：`add_viewer_scene` 装配的是
/// 查看器本身，可其中有少数东西的正确取值取决于**谁在跑它**，而这些东西
/// 不适合由 cesium 自己的环境变量决定（那会让宿主的行为依赖它是怎么被
/// 启动的）。每一项都是**加法式**的：`Default` 逐条等于独立
/// `cargo run -p cesium-app` 的历史行为。
#[derive(Debug, Clone, Default)]
pub struct ViewerOptions {
    /// 在线瓦片磁盘缓存的根目录。
    ///
    /// `None`（默认）＝沿用 cesiumrust 自己的规则：`CESIUM_TILE_CACHE_DIR`
    /// 环境变量，否则**工作目录**下的 `tile_cache`。宿主应当传一个自己的
    /// 数据目录，否则从任意启动目录拉起宿主都会在那里留下 `tile_cache/`。
    ///
    /// 它的优先级高于环境变量（见 [`tile_cache::set_cache_root`]），因为
    /// 显式参数比进程环境更能表达宿主的意图，且宿主读不到自己的环境里
    /// 有没有被设过这个变量。
    pub cache_root: Option<std::path::PathBuf>,
}

/// 把整棵查看器场景注册到 `app` 上 —— `main` 与嵌入方共用的**唯一一份**装配。
///
/// 这是把 `main` 拆成 lib 的落点：调用方负责 `App::new()`、`DefaultPlugins`
/// 与本进程的窗口/事件循环配置，本函数负责**其余全部** —— 也就是原先
/// `main` 从 `insert_resource(ClearColor)` 到 `PerfTracePlugin` 的那一整段，
/// 顺序与门控逐条保留（原有注释随代码一并留在此处）。
///
/// # 参数
/// - `app`：已经装好 `DefaultPlugins`（以及本进程自己的窗口配置）的 app。
///   本函数还要插入若干资源，因此必须在 `App::update()` 之前调用。
/// - `lighting`：光照模式。调用方通常取 [`feature_flags::lighting_mode`]；
///   必须在 `CesiumCorePlugin` 之前插入，其 Startup 系统读它。
/// - `window_ui`：是否注册**只在有窗口时才有意义**的两件东西 ——
///   `Map2dPlugin`（2D 平面图模式 + 它的切换按钮，bevy_ui）与
///   `cesium_plot_bevy::CesiumPlotBridgePlugin`（标绘叠加层，同样是
///   bevy_ui + render layer 3）。`main` 在无头分支里正是靠 `!headless`
///   跳过它们的；离屏嵌入方若没有 `Window` 实体，bevy_ui 布局无从谈起，
///   故取 `false`。
/// - `trace`：perf-trace 的解析结果（`--perf-trace` / camera-script 回放 /
///   无头 auto-exit 上限）。嵌入方**不应**传 `Cli::from_env_and_args()`：
///   那会去读宿主的 `argv`。
pub fn add_viewer_scene(
    app: &mut App,
    lighting: LightingMode,
    window_ui: bool,
    trace: perf_trace::Cli,
) {
    add_viewer_scene_with_options(app, lighting, window_ui, trace, &ViewerOptions::default());
}

/// [`add_viewer_scene`] 的带配置版本：多一个 [`ViewerOptions`]。
///
/// 拆成两个函数而不是给原函数加参数，是为了让**独立 `cesium-app` 的调用点
/// 与它的行为一起冻结**：`main.rs` 继续调用 `add_viewer_scene`，得到的仍然
/// 是 `ViewerOptions::default()`（工作目录下的 `tile_cache`），本次改动因此
/// 对独立查看器是零行为差异。
///
/// # 参数
/// - `options`：见 [`ViewerOptions`]。必须在任何瓦片被取用之前调用 ——
///   缓存根是进程级、首次解析即定的（[`tile_cache::set_cache_root`]），
///   而本函数是宿主装配场景的唯一入口，正是安装它的时机。
pub fn add_viewer_scene_with_options(
    app: &mut App,
    lighting: LightingMode,
    window_ui: bool,
    trace: perf_trace::Cli,
    options: &ViewerOptions,
) {
    // 缓存根：显式选项 > `CESIUM_TILE_CACHE_DIR` > 工作目录下的 `tile_cache`。
    // 这里只是**安装**，不创建目录 —— 目录由首次成功落盘瓦片时按需创建，
    // 因此一个拿不到网络的宿主不会留下一个空目录。
    if let Some(root) = options.cache_root.as_ref() {
        if !tile_cache::set_cache_root(root.clone()) {
            warn!(
                "[tile_cache] cache root already resolved to {}; ignoring the host's {}",
                tile_cache::cache_root_path().display(),
                root.display()
            );
        }
    }

    app.insert_resource(ClearColor(Color::BLACK))
        // M4.1：在 CesiumCorePlugin 之前插入 lighting 模式，以便 setup_lighting
        // 读取它（否则 init_resource 会用 Default=FullAmbient）。
        .insert_resource(lighting)
        // Core：lighting + 地球配置 + AnimationClock
        .add_plugins(CesiumCorePlugin)
        // Camera：鼠标 orbit/缩放
        .add_plugins(OrbitCameraPlugin)
        // 地球渲染
        .add_plugins(BaseSpherePlugin);

    // 地球渲染。P1-1/建议1 (2026-09-27)：黄金路径是 M1.5
    // 薄壳。冻结的遗留单体（`dynamic_globe_legacy`，旧的
    // CESIUMRST_LEGACY_DYNAMIC_GLOBE A/B 分支）在 G4 证明它与薄壳像素中性
    // 之后已被退役（参见
    // PIPELINE_PROMOTION_PLAN.md 与 verification_evidence/g4/）。
    app.add_plugins(dynamic_globe::DynamicGlobePlugin);

    // ── M5-B：AtmosphereGlow ↔ SkyDome 互斥 ────────────────
    // 8-shell 辉光回退与程序化 sky dome 渲染相互重叠的
    // 大气边缘效果。glow_enabled() 返回 !skydome_enabled() 因此
    // 它们永不同时激活。默认（SKYDOME OFF）→ glow ON → v0 零差异。
    // 安全：AtmosphereGlowPlugin 的 fade 系统读取 Res<OrbitState>，后者
    // 因 OrbitCameraPlugin 在上方无条件注册而保证存在
    // （与 #47 terrain 插件独立性同一模式）。
    if glow_enabled() {
        app.add_plugins(AtmosphereGlowPlugin);
    }
    app.add_plugins(StarfieldPlugin);

    // ── 2D 平面图模式 + 切换按钮（仅窗口化）─────────────────
    // 在 CESIUM_HEADLESS 下永不注册，因此确定性的离屏
    // 捕获路径保持单相机、无 UI、无额外 render pass（v0 /
    // FIXED_CAMERA / camera-script 基线保持字节精确）。在默认
    // 窗口化会话中 MapMode 以 ThreeD 开始，因此 3D orbit 路径也
    // 不变地运行直到用户点击按钮。
    if window_ui {
        app.add_plugins(map2d::Map2dPlugin);
    }

    // ── cesium-plot overlay 桥接（仅窗口化）──────────────────────────
    // 在 CESIUM_HEADLESS 下永不注册，并由 `plot_enabled()` 门控
    // （未设时默认 ON，用 CESIUM_ENABLE_PLOT=0 退出）。
    // 2026-09 复核订正：此处原先写「M0 只添加共享的桥接资源（PlotViewCtx /
    // PlotInputCapture）—— 无系统、实体或 render-layer 内容」，该描述**已过时**。
    // 现 `cesium_plot_bevy::CesiumPlotBridgePlugin`（adapters/plot-bevy/src/plugin.rs）
    // 实际注册 16 个资源（PlotViewCtx/PlotDocument/PlotVisuals/PlotSelection/
    // PlotInteraction/PlotHistory/PlotDrag/PlotSnap/PlotPickCache/SyncState/
    // PlotShapeCache/PlotToolbarRoot/PanelRootEntity 等）、6 条事件流，
    // 并在 Startup 挂 `plot_toolbar`+`panel_startup`，在 Update 挂 12 个系统
    // （sync_visuals→pick_system→interaction_system→draw_preview_system→
    // edit_system→drag_move_system→toolbar_click_system→clear_all_system→
    // bind_toolbar_camera→bind_panel_camera→panel_click_system→panel_sync_system）。
    // 视图同步由 app 侧 `map2d.rs:670 sync_plot_view_ctx` 写入 PlotViewCtx。
    // 3D 黄金路径与离屏基线仍保持字节精确：叠加层只走 render layer 3，
    // 且 headless 分支根本不实例化本插件。
    if window_ui && feature_flags::plot_enabled() {
        app.add_plugins(cesium_plot_bevy::CesiumPlotBridgePlugin);
    }

    // ── M4.1：shadow 插件（仅 day_night lighting）───────────────────
    if lighting == LightingMode::DayNight {
        app.add_plugins(CesiumShadowPlugin);
        info!("[M4.1] DayNight lighting: shadow plugin registered");
    }

    // ── M5-B：程序化 sky dome —— 独立的 SKYDOME 门控 ────────────
    // CesiumAtmospherePlugin 由 SKYDOME=1 激活，而非由 DayNight。
    // 将天空渲染与 lighting 模式解耦，让 M5-C 扩展
    // sky_system.rs 而无需碰 main.rs。默认 OFF → v0 零差异。
    // DEVIATION：sky dome 独立于 DayNight 门控，与 AtmosphereGlowPlugin
    //   互斥；参见 docs/deviations.md#dev-015
    if skydome_enabled() {
        app.add_plugins(CesiumAtmospherePlugin);
        info!("[M5-B] SkyDome: CesiumAtmospherePlugin registered");
        // M5-C 注意事项：dome shader 发射*未归一化*的在散射
        // radiance（太阳盘可超过 1.0）。原先依赖 M4.2 在
        // CESIUM_ENABLE_POSTPROCESS_BUILTIN 下布线的 HDR + ACES 色调映射
        // 来显示（docs/deviations.md#dev-010）；该后处理已随 P0 观感清理
        // 移除，因此仅开 SKYDOME 时这些值会落入 LDR framebuffer 并被 clip。
    }

    // ── M4.2 + M5-E1：后处理（已随 P0 观感清理移除）───────────
    // 原实现在 `postprocess_builtin_enabled() || postprocess_enabled()` 下注册
    // `CesiumEffectsPlugin`：
    //   • CESIUM_ENABLE_POSTPROCESS_BUILTIN → M4.2 内建后处理（HDR +
    //     ACES Fitted 色调映射 + Bloom）。
    //   • CESIUM_ENABLE_POSTPROCESS         → M5-E1 自实现 FXAA render-graph 节点
    //     （外加 SSAO 子门控）。
    // 目标（2D/3D 地球影像高效显示 + 导航标绘）明确不需要复杂观感，
    // 其领域 crate `domain/effects` 与适配器模块
    // `adapters/bevy-render/src/effects` 均已删除，因此这里不再注册任何后处理。
    // 默认路径（无 env）本就取 `Tonemapping::None`，故像素中性不变。

    // ── M5-D：Fabric 材质展示（内置 + 3 种 Water 海况）──────
    // 纯附加，env 门控（默认 OFF → v0 基线像素中性）。
    // 通过 CESIUM_ENABLE_MATERIAL_SHOWCASE=1 opt-in 渲染材质球
    // —— 包括复刻的 Water 程序化图案（case 17u）calm/medium/rough
    // 基线，捕获到 specs/baselines/v2_water/（4 PNG：3 张海况
    // 特写 + 1 张弧形总览）。现有插件注册未被改动。
    // 该门控通过 feature_flags 注册表读取（每个 CESIUM_* env
    // 名的单一真相源）而非一个裸字符串字面量。
    if material_showcase_enabled() {
        app.add_plugins(MaterialShowcasePlugin);
        info!("[M5-D] Material showcase: MaterialShowcasePlugin registered");
    }

    // ── M6 Wave A（任务 #81）：Clipping / Panorama / IBL / OIT / Clouds / Split ──
    // 原实现在六个 env 门控（CESIUM_ENABLE_{CLIPPING,PANORAMA,IBL,OIT,CLOUDS,SPLIT}）
    // 下注册 `M6WaveARenderGraphPlugin`，并向相机插入对应的 M6 组件
    // （CesiumClippingPlanes / CesiumPanorama / CesiumIbl / CesiumOit /
    // CesiumClouds / CesiumSplit），以及 M6WaveAConfig / m6_wave_a_prepare /
    // m6_wave_a_setup / m6_demo_* / m6_procedural_sky_cube 等辅助代码。
    //
    // 这些全部是复杂观感能力（屏幕空间裁剪平面、360° 全景捕获、基于图像
    // 的光照 IBL、顺序无关透明 OIT、体积云、分屏多视图），与「2D/3D 地球
    // 影像高效显示 + 导航标绘」的目标无关。其实现所依赖的领域 crate
    // `domain/effects`、适配器模块 `adapters/bevy-render/src/effects` 以及
    // `wgpu` 直接依赖均已随 P0 清理删除，因此这里不再注册任何 M6 节点或组件。
    // 全部门控原本默认 OFF，故默认路径像素中性不变。

    // M0.4：注册 PerfCounters 资源（dynamic_globe 写入，trace 读取）。
    app.insert_resource(PerfCounters::default());

    // ── M2.5：CameraControl 驱动端口 ──────────────────────────
    // 程序化的相机控制（set_view / fly_to / look_at / zoom / home），
    // 委派给领域相机算法。作为一个资源注册，因此
    // 脚本/系统可以获取 `CameraControlPort` 并驱动相机；PostUpdate 桥接
    // 将它同步到任意 `CesiumCamera` 实体，且当不存在时不生效
    // （交互式默认相机仍由 orbit_camera 驱动，
    // 像素中性）。
    app.init_resource::<CameraControlPort>()
        .add_systems(PostUpdate, camera_control_port_system);

    // M0.5：perf-trace 插件（CSV writer + 可选 camera-script 回放
    // + 可选无头 auto-exit）。未给 trace 路径时不生效。
    app.add_plugins(PerfTracePlugin::new(trace));

    // P1 裁剪（2026-09）：原先此处按 `enable_terrain` / `enable_tileset`
    // 注册地形链与 3D Tiles 路线 ——
    //   `CesiumTerrainPlugin`（含为地形垂覆供纹理的 `CesiumImageryPlugin`、
    //   `spawn_terrain_config_task` / `apply_terrain_endpoint` 两个 terrain 系统）
    //   与 `CesiumTilesetPlugin` + `spawn_tileset_root`，以及二者共用的
    //   `stats_logger_system`。
    // 这些插件随 `cesium-terrain` / `cesium-tileset` / `cesium-gltf` /
    // `cesium-styling` 四个域 crate 一并删除：目标是「2D/3D 地球影像 +
    // 标绘」，标绘只贴 WGS84 椭球，不需要地形垂覆与倾斜摄影。
    // 底球面 + Bing 影像的默认渲染路径（dynamic_globe 薄壳）不受影响。
}

/// 应用入口：根据特性开关组装 Bevy 应用并注册各插件与系统。
///
/// 行为与原先的 `fn main` 逐字一致；`src/main.rs` 现在只是一层
/// `fn main() { cesium_app::run() }` 的壳，使这些模块只被编译一次
/// （进 lib），而不再被 bin 与 lib 各编译一份。
pub fn run() {
    // M3.3：无头离线自检 —— 无需 GPU 即可证明 fetcher 布线。
    // 立即退出（无 Bevy app、无窗口、无需 GPU 上下文）。
    if feature_flags::offline_selfcheck() {
        std::process::exit(offline_check::run());
    }

    // M0.3/M0.4/M0.5 的链式插件注册已随地形链与 3D Tiles 路线在 P1 中移除，
    // 见下方 `enable_terrain`/`enable_tileset` 原先的位置说明。
    //
    // P1 注记：`feature_flags` 中的 `terrain_enabled()` / `tileset_enabled()`
    // 访问器按该模块的「冻结注册表」约定保留（它们的名字参与
    // `FlagSnapshot::summary_line()` 的稳定输出），但已无任何消费者。

    // M0.5：用于无头 + trace + camera-script 的 CLI/env 解析。
    let mut cli = perf_trace::Cli::from_env_and_args();

    // ── M11.3：无头模式所有权 ──────────────────────────
    // `CESIUM_HEADLESS`（默认 OFF）现在驱动一次真正的无表面
    // 渲染 → 离屏捕获 → PNG → 通过
    // `CesiumHeadlessPlugin`（下方新增）干净退出。perf-trace 旧的 M0.5
    // 对于同一标志的行为是一个帧数上限的 *auto-exit*，它会在第 N 帧触发
    // 并在异步截图回读（约在同一帧 spawn）落地前杀掉进程 —— 因此
    // PNG 永不会被写入。通过清除 perf-trace 的无头 auto-exit
    // 将退出所有权交给捕获插件；camera-script 回放（一个独立的标志）
    // 不受影响。
    //
    // FIX-HL-EXIT（退出链已验证）：`CesiumHeadlessPlugin` 递减
    // `headless_frames()` Update tick，然后 `headless_capture_tick` spawn
    // 离屏的 `Screenshot::image`，其 `save_to_disk` observer 同步写入 PNG
    // 且第二个 observer 发送 `AppExit::Success`
    // （`headless/mod.rs` L316-320）。因此在此清除 `cli.headless` 是安全的 ——
    // 进程仍会终止且产物仍会被写入；perf-trace 的
    // `headless_exit_system` 仅退让，以便两个退出永不竞态。
    let headless = feature_flags::headless_enabled();
    if headless {
        cli.headless = false;
    }

    let mut app = App::new();

    // M4.1：在插件注册*之前*从 env 读取 lighting 模式，以便
    // CesiumCorePlugin 的 Startup 系统看到正确的值。
    let lighting = lighting_mode();

    // ── M11.3：无头（无表面）渲染模式 ──────────────────
    // 当 `headless` 时，将窗口化的 WindowPlugin 换为一个 `primary_window:
    // None` / `DontExit` 配置，因此 app 渲染时无窗口、无监视器也无
    // 显示服务器。`else` 分支逐字节就是 M11.3 前的窗口化
    // 配置，因此默认启动路径不变（黄金路径中性）。
    if headless {
        // 无表面：既无主窗口也无 winit 事件循环。零窗口时
        // winit 循环会停车等待事件，因此 `Update` 永不会 tick 且捕获→AppExit 路径
        // 永不会触发。改用连续的 ScheduleRunner 驱动帧；离屏捕获
        // 插件（下方新增）渲染 `headless_frames()` 帧，写入
        // PNG，然后发送 AppExit 以终止 runner。
        //
        // FIX-HL-EXIT：`Duration::ZERO` 是有意为之 —— 无头没有可呈现的窗口也
        // 没有 vsync，因此 runner 以 CPU 允许的最快速度 tick，
        // 运行时长的唯一约束是 `headless_frames()` 捕获计数。
        // 人为限流只会拖慢捕获而毫无收益。
        app.add_plugins(
            DefaultPlugins
                .set(cesium_bevy_render::headless::headless_window_plugin())
                .disable::<bevy::winit::WinitPlugin>()
                .add(bevy::app::ScheduleRunnerPlugin::run_loop(
                    std::time::Duration::ZERO,
                )),
        );
    } else {
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "CesiumRust - 3D Globe Viewer".into(),
                resolution: (1280.0_f32, 720.0_f32).into(),
                ..default()
            }),
            ..default()
        }));
    }

    // 场景装配（基础球 / 影像瓦片 LOD / orbit 相机 / 大气辉光 / 星空 /
    // 门控插件）全部搬进了 lib 的 `add_viewer_scene`，`main` 与嵌入方共用
    // 同一份 —— 见该函数的文档。这里只决定两件 `main` 特有的事：是否注册
    // 窗口专属的 2D 平面图与标绘 UI（无头分支原本就跳过），以及
    // perf-trace 的 CLI 从哪里来。
    add_viewer_scene(&mut app, lighting, !headless, cli);

    // P1 裁剪（2026-09）：原先此处按 `enable_terrain` / `enable_tileset`
    // 注册地形链与 3D Tiles 路线 ——
    //   `CesiumTerrainPlugin`（含为地形垂覆供纹理的 `CesiumImageryPlugin`、
    //   `spawn_terrain_config_task` / `apply_terrain_endpoint` 两个 terrain 系统）
    //   与 `CesiumTilesetPlugin` + `spawn_tileset_root`，以及二者共用的
    //   `stats_logger_system`。
    // 这些插件随 `cesium-terrain` / `cesium-tileset` / `cesium-gltf` /
    // `cesium-styling` 四个域 crate 一并删除：目标是「2D/3D 地球影像 +
    // 标绘」，标绘只贴 WGS84 椭球，不需要地形垂覆与倾斜摄影。
    // 底球面 + Bing 影像的默认渲染路径（dynamic_globe 薄壳）不受影响。

    // 用于无头验证的可选确定性截图（默认不生效）。
    if let Some(at_frame) = std::env::var("CESIUM_SCREENSHOT_AT_FRAME")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
    {
        let path = std::env::var("CESIUM_SCREENSHOT").unwrap_or_else(|_| "screenshot.png".into());
        app.insert_resource(AutoScreenshot { frame: 0, at_frame, path });
        app.add_systems(Update, auto_screenshot_system);
    }

    // ── M3.3：FIXED_TIME —— 冻结时钟以确定 lighting ──────
    if feature_flags::fixed_time_enabled() {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ));
        info!("[M3.3] FIXED_TIME: clock frozen (delta=0)");
    }

    // ── M3.3：FIXED_CAMERA —— 确定性位姿覆盖 ────────────────
    if let Some(cam_path) = feature_flags::fixed_camera_path() {
        match std::fs::read_to_string(&cam_path) {
            Ok(text) => match toml::from_str::<FixedCameraFile>(&text) {
                Ok(file) => {
                    let p = file.camera.pos;
                    let q = file.camera.quat;
                    app.insert_resource(FixedCameraPose {
                        pos: Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32),
                        quat: Quat::from_xyzw(
                            q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32,
                        ),
                        fov_y: file.camera.fov_y.map(|f| f as f32),
                    });
                    app.add_systems(PostUpdate, fixed_camera_system);
                    info!("[M3.3] FIXED_CAMERA: pose from {}", cam_path.display());
                }
                Err(e) => warn!("[M3.3] FIXED_CAMERA parse failed: {e}"),
            },
            Err(e) => warn!("[M3.3] FIXED_CAMERA read failed {}: {e}", cam_path.display()),
        }
    }

    // ── M3.4：CESIUM_SCREENSHOT_SCRIPT —— 批量多视图捕获 ───────
    if let Some(script_path) = feature_flags::screenshot_script_path() {
        match capture_script::ShotScript::from_file(&script_path) {
            Ok(script) => {
                let shots = script.sorted().shot;
                let output_dir = std::env::var("CESIUM_SCREENSHOT_DIR")
                    .unwrap_or_else(|_| ".".to_string());
                // 确保输出目录存在。
                let _ = std::fs::create_dir_all(&output_dir);
                info!(
                    "[M3.4] batch capture: {} shots from {} → {}/",
                    shots.len(),
                    script_path.display(),
                    output_dir
                );
                app.insert_resource(BatchCapture {
                    script: shots,
                    cursor: 0,
                    frame: 0,
                    output_dir,
                    git_sha: feature_flags::git_sha(),
                    env_snapshot: build_env_snapshot(),
                });
                app.add_systems(PostUpdate, batch_capture_system);
            }
            Err(e) => warn!("[M3.4] screenshot script parse failed: {e}"),
        }
    }

    // ── M11.3：无头离屏捕获 → PNG → 干净退出 ────────────
    // 仅在 CESIUM_HEADLESS 下激活（默认 OFF → 插件未添加 →
    // 窗口化路径逐字节未被触碰）。创建一个离屏 RGBA8(sRGB)
    // 目标，每帧将场景 Camera3d 重新指向它，渲染
    // `headless_frames()` 预热帧，然后捕获一张 PNG（`tools/pixel_diff`
    // 为像素门控消费的产物）并请求 AppExit。
    // NOTE（推迟至 M11.4）：多视图批量捕获
    // （CESIUM_SCREENSHOT_SCRIPT）与 AutoScreenshot 仍使用
    // Screenshot::primary_window()，因此它们尚未无头感知 —— 它们现在
    // 会发出一个 `warn!`（FIX-HL-EXIT）而非静默失败；通过本插件的单视图
    // 离屏捕获是受支持的无头路径。
    if headless {
        let output = feature_flags::headless_output();
        let frames = feature_flags::headless_frames();
        info!(
            "[M11.3] CESIUM_HEADLESS: offscreen capture → {} after {} frames",
            output.display(),
            frames
        );
        app.add_plugins(cesium_bevy_render::headless::CesiumHeadlessPlugin::new(
            output, frames,
        ));
    }

    app.run();
}

