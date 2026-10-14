//! 2D 平面图模式 + 2D/3D 切换按钮（参见计划 `cesium-app_2D_地图模式`）。
//!
//! ## 本模块范围
//! 单个 [`MapMode`] 资源决定查看器运行的是久经考验的 3D
//! 地球路径还是新的平面 Geographic（等距矩形）地图。在 2D 中，影像
//! 通过 `x = R·lon, y = R·lat` 以 Web Mercator 瓦片层
//! 铺在 XY 平面上；当某单元格的精确瓦片仍在下载时，回退到其最佳
//! 缓存祖先，因此视图绝不会闪白。
//!
//! ## 确定性契约（硬约束）
//! [`MapMode`] 默认为 [`MapMode::ThreeD`]。3D orbit 系统用 `run_if(map_is_3d)`
//! 门控，因此在默认模式下它们与之前完全一样运行 —— 逐字节。
//! 本插件只在 `main` 的*窗口*分支注册，从不在
//! `CESIUM_HEADLESS` 下，因此离屏捕获路径（v0 /
//! FIXED_CAMERA / camera-script golden 测试）不受影响：无第二相机、无
//! UI、无额外渲染 pass。

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::RenderLayers;

use cesium_plot_bevy::{PlotInputCapture, PlotViewCtx, PlotViewMode};

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::orbit_camera::OrbitCamera;

/// 瓦片数学所需常量的 `f64` 副本（保持为双精度，以避免在深层
/// 缩放级别出现可见的列漂移）。
const PI6: f64 = std::f64::consts::PI;
const TAU6: f64 = std::f64::consts::TAU;

// ── Geographic 世界尺度 ──────────────────────────────────────
// 1 render unit == 地球半径，因此等距矩形平面在
// x ∈ [-π, π]（经度）和 y ∈ [-π/2, π/2]（纬度）上展开。纹理宽度
// 为整个世界，因此用 WORLD_W 对相机中心取模可使网格无缝
// → “无限”经度拖拽。
const WORLD_W: f32 = std::f32::consts::TAU; // 2π，x 方向的完整 360° 周期
const LAT_MAX: f32 = std::f32::consts::FRAC_PI_2; // π/2，y 方向 ±90°

/// 缩放（每世界单位的像素）限制。`ZOOM_MIN` 使整个世界加边距
/// 落在超大基础 quad 内，因此不显示空白边缘；`ZOOM_MAX` 的上限取得
/// 足够高，使 `tile_zoom_for` 能抵达最细瓦片级别 `TILE_Z_MAX`（=19），
/// 从而允许一直放大到 Bing 影像的最高分辨率（而非停在 ~7 级）。
const ZOOM_MIN: f32 = 110.0;
const ZOOM_MAX: f32 = 20_000_000.0;
// 默认值使地图在垂直方向上填满画面（窗口高度 ≈ π·zoom），而非
// 作为一条小带子漂浮在空白边距中。
const ZOOM_DEFAULT: f32 = 240.0;

/// 相机高于平面的高度（render unit）。只需位于正交 near/far
/// 范围带内；投影是正交的，因此除了深度排序外这在视觉上无关紧要。
const CAM_Z: f32 = 100.0;

// ── P2 影像瓦片调参 ────────────────────────────────────
// 瓦片级别的选取使一个 256px 瓦片跨越 ~256 屏幕 px（原生分辨率）。
const TILE_PX_TARGET: f32 = 256.0;
const TILE_Z_MIN: i32 = 1;
const TILE_Z_MAX: i32 = 19;
/// 瓦片漂浮在占位网格（z = 0）上方一点，因此赢得深度测试。
const TILE_Z_ELEV: f32 = 0.5;
/// 下载 2D 影像瓦片的后台工作线程。
const MAP2D_DOWNLOAD_THREADS: usize = 8;

// ── 2D/3D 切换调色板 ──────────────────────────────────────
// `Color::srgba` 在 bevy_color 0.15 中是一个 `const fn`，因此整个主题
// 可以是编译期数据。深色“玻璃”外壳 + 潮蓝强调色用于激活段。
const SW_GLASS: Color = Color::srgba(0.07, 0.10, 0.15, 0.80);
const SW_GLASS_BORDER: Color = Color::srgba(1.0, 1.0, 1.0, 0.16);
const SW_ACCENT: Color = Color::srgba(0.16, 0.50, 0.86, 1.0);
const SW_ACCENT_PRESSED: Color = Color::srgba(0.11, 0.37, 0.66, 1.0);
const SW_HOVER: Color = Color::srgba(1.0, 1.0, 1.0, 0.14);
const SW_HOVER_PRESSED: Color = Color::srgba(1.0, 1.0, 1.0, 0.26);
const SW_ACTIVE_TEXT: Color = Color::srgba(1.0, 1.0, 1.0, 1.0);
const SW_IDLE_TEXT: Color = Color::srgba(0.72, 0.78, 0.86, 1.0);

/// 查看器处于哪种投影。`ThreeD` 是默认值也是 golden
/// 路径；`TwoD` 是平面图。
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MapMode {
    #[default]
    ThreeD,
    TwoD,
}

/// 系统运行条件：平面图激活时为 true。
pub fn map_is_2d(mode: Res<MapMode>) -> bool {
    matches!(*mode, MapMode::TwoD)
}

/// 系统运行条件：3D 地球激活时（默认）为 true。由
/// `OrbitCameraPlugin` 用来门控其系统而不改变 3D 行为。
pub fn map_is_3d(mode: Res<MapMode>) -> bool {
    matches!(*mode, MapMode::ThreeD)
}

/// 2D 正交相机的标记（与 [`OrbitCamera`] 分开保留，
/// 因此两者从不争夺同一个 query）。
#[derive(Component)]
struct Map2dCamera;

/// 2D/3D 分段切换的一个单元格。记住它选择的模式
/// 和它自己的标签实体，因此单个系统能一起重新设置填充 + 文本样式。
#[derive(Component)]
struct ModeSegment {
    /// 该段按钮点击后切入的模式。
    mode: MapMode,
    /// 该段自己的文本标签实体（选中态一起刷新样式）。
    label: Entity,
}

/// 切换器外层胶囊容器的标记。也是一个 UI 根，因此它需要一个
/// 显式的 [`TargetCamera`]（参见 [`sync_ui_target_camera]）。
#[derive(Component)]
struct ModeSwitchRoot;

/// 左下角读数容器的标记 —— 另一个 UI 根。
#[derive(Component)]
struct ReadoutRoot;

/// 携带平面图相机状态的组件（Geographic 世界坐标）。
#[derive(Component)]
struct Map2dCam {
    /// Geographic 世界单位下的相机中心（x = R·lon, y = R·lat）。
    center: Vec2,
    /// 每世界单位的像素；正交缩放为 `1 / zoom`。
    zoom: f32,
}

/// 实时光标读数文本（lon°/lat°）。
#[derive(Component)]
struct CoordText;

/// 实时缩放级别文本。
#[derive(Component)]
struct LevelText;

// ── P2：真实影像瓦片 ────────────────────────────────────

/// 逐瓦片影像 quad 的标记，以其规范 Web-Mercator `(x, y, z)`
/// 为键。放置时使用一个原始（可能越界）列，因此地图在经度上
/// 无限环绕，而纹理按规范列获取。
#[derive(Component)]
struct Map2dTile {
    /// 规范 Web-Mercator 瓦片键 `(x, y, z)`（纹理按规范列获取）。
    key: (u32, u32, u32),
}

/// 一个已解码的瓦片图像，由工作线程送回主世界。
struct Map2dTileImg {
    /// 规范瓦片列 x。
    x: u32,
    /// 规范瓦片行 y。
    y: u32,
    /// 瓦片层级 z。
    z: u32,
    /// 解码后的 RGBA 像素数据。
    rgba: Vec<u8>,
    /// 纹理宽度（像素）。
    width: u32,
    /// 纹理高度（像素）。
    height: u32,
}

/// 2D 影像层的跨帧状态：一个小工作池下载 Bing Aerial 瓦片
/// （或从磁盘读取 `OFFLINE_IMAGERY_ROOT`），解码后的 RGBA 按规范键
/// 缓存，并为每个可见的原始列/行维护一个 quad 实体。与 3D
/// `TileManager` 完全分开，因此 golden 地球路径从不与 2D 平移纠缠。
#[derive(Resource)]
struct Map2dTiler {
    /// 向下载 worker 池发送待取瓦片任务 `(x, y, z)` 的发送端。
    job_tx: mpsc::Sender<(u32, u32, u32)>,
    /// 接收 worker 送回已解码瓦片的通道（互锁包裹以支持多生产者）。
    rx: Mutex<mpsc::Receiver<Map2dTileImg>>,
    /// 按规范键缓存的已上传纹理句柄。
    cache: HashMap<(u32, u32, u32), Handle<Image>>,
    /// 按*放置位置*为键的活动 quad —— `(raw_col, raw_row, level)`。级别
    /// 是键的一部分，因为一个视图会将目标瓦片与其下方作为回退显示的
    /// 更粗祖先混在一起。
    live: HashMap<(i64, i64, u32), Entity>,
    /// 已派发的在途瓦片集，避免重复请求。
    in_flight: HashSet<(u32, u32, u32)>,
    /// 共享的单位 quad mesh（首次使用时懒建）。
    unit_quad: Option<Handle<Mesh>>,
}

impl Default for Map2dTiler {
    /// 建立任务/结果通道并拉起常驻下载 worker 池，各映射置空。
    fn default() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<(u32, u32, u32)>();
        let (res_tx, res_rx) = mpsc::channel::<Map2dTileImg>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        for _ in 0..MAP2D_DOWNLOAD_THREADS {
            let jr = job_rx.clone();
            let tx = res_tx.clone();
            std::thread::spawn(move || map2d_worker(jr, tx));
        }
        Self {
            job_tx,
            rx: Mutex::new(res_rx),
            cache: HashMap::new(),
            live: HashMap::new(),
            in_flight: HashSet::new(),
            unit_quad: None,
        }
    }
}

/// 左键按住时记住上一帧的光标采样。平移对两个在*同一*
/// 空间采样的值做差 —— `Window::cursor_position`（逻辑 px，左上
/// 原点）—— 而非将其与 `MouseMotion`（物理 px，y 向上）混用，
/// 后者会使 2D 拖拽反转且非 1:1。
#[derive(Resource, Default)]
struct Map2dPanCursor {
    /// 上一帧左键按住时光标位置（逻辑 px）；未拖拽时为 `None`。
    last: Option<Vec2>,
}

/// 接入 2D 地图的插件。只在 `main` 的窗口分支注册。
pub struct Map2dPlugin;

impl Plugin for Map2dPlugin {
    /// 插件装配入口：添加 2D 专用系统、启动时 spawn 平面相机与切换 UI，
    /// 并初始化瓦片器/平移光标资源（`MapMode` 本身由轨道相机插件初始化）。
    ///
    /// # 参数
    /// - `app`：Bevy 应用。
    fn build(&self, app: &mut App) {
        // MapMode 本身由 OrbitCameraPlugin（总是存在）初始化；
        // 这里我们只添加 2D 专用系统和启动 spawn。
        app.add_systems(
            Startup,
            (spawn_map2d_camera, build_mode_ui).chain(),
        )
        .init_resource::<Map2dTiler>()
        .init_resource::<Map2dPanCursor>()
        .add_systems(
            Update,
            (
                sync_camera_by_mode,
                sync_ui_target_camera,
                sync_plot_view_ctx,
                (map2d_pan_system, map2d_zoom_system).run_if(map_is_2d),
                apply_map2d_cam.run_if(map_is_2d),
                update_map2d_tiles.run_if(map_is_2d),
                update_mode_segments,
                update_readout,
            )
                .chain(),
        )
        .add_systems(Update, on_mode_segment_click);
    }
}

// ── Spawn ──────────────────────────────────────────────────

fn spawn_map2d_camera(mut commands: Commands) {
    // 对 XY 平面的正交俯视图。单位旋转沿 -Z 看，
    // 屏幕右 = +X（东），屏幕上 = +Y（北）—— 正是 Geographic
    // 布局。`ScalingMode::WindowSize`（默认）使 `scale` == 每像素的世界
    // 单位，因此 `scale = 1 / zoom`。`order` 从低值开始；sync 在 2D 中将其提升。
    let projection = OrthographicProjection {
        scale: 1.0 / ZOOM_DEFAULT,
        near: -1000.0,
        far: 1000.0,
        ..OrthographicProjection::default_3d()
    };
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 0,
            ..default()
        },
        // 工作区禁用了 `tonemapping_luts`，因此 `Camera3d` 的默认值
        // （`TonyMcMapFace`）会每帧记录一条错误。无光照的平面图根本
        // 不需要任何色调映射。
        Tonemapping::None,
        // 图层 1 = 平面影像瓦片，图层 2 = 共享 UI，图层 3 = 标绘
        // overlay。它从不看到地球（图层 0），因此切换模式不会让 3D
        // 场景残留显示；图层 3 与 3D 相机共享。
        RenderLayers::from_layers(&[1, 2, 3]),
        Projection::Orthographic(projection),
        Map2dCamera,
        Map2dCam {
            center: Vec2::ZERO,
            zoom: ZOOM_DEFAULT,
        },
        Transform::from_xyz(0.0, 0.0, CAM_Z),
    ));
}

/// 构建右下角 2D/3D 分段切换胶囊与左下角经纬度/层级读数 UI。
fn build_mode_ui(mut commands: Commands) {
    // ── 右下角 2D/3D 分段切换 ─────────────────────────────────
    // 一个容纳两个圆角单元格的胶囊形玻璃外壳；与当前模式匹配的
    // 单元格带有强调填充，因此控件始终显示你所在的模式（旧的
    // "[ 2D ]" 切换按钮宣传的是*另一个*模式，看起来像噪声）。
    // 悬停/按下反馈位于 `update_mode_segments`。
    let container = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(18.0),
                bottom: Val::Px(18.0),
                column_gap: Val::Px(3.0),
                padding: UiRect::all(Val::Px(4.0)),
                // 1px 边框必须在 `BorderColor` 绘制任何内容之前设好尺寸。
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(SW_GLASS),
            BorderColor(SW_GLASS_BORDER),
            // 容器 42px 高度的一半 → 真正的胶囊形。
            BorderRadius::all(Val::Px(21.0)),
            BoxShadow {
                color: Color::srgba(0.0, 0.0, 0.0, 0.45),
                x_offset: Val::Px(0.0),
                y_offset: Val::Px(4.0),
                spread_radius: Val::Px(0.0),
                blur_radius: Val::Px(12.0),
            },
            // 图层 2：共享 UI，3D 和 2D 相机都能看到。
            RenderLayers::layer(2),
            ModeSwitchRoot,
        ))
        .id();

    spawn_mode_segment(&mut commands, container, MapMode::ThreeD);
    spawn_mode_segment(&mut commands, container, MapMode::TwoD);

    // 左下角读数区块（坐标 + 缩放级别），垂直堆叠。
    let readout = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                bottom: Val::Px(20.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                ..default()
            },
            RenderLayers::layer(2),
            ReadoutRoot,
        ))
        .id();
    commands
        .spawn((
            Text::new(""),
            TextFont {
                font_size: 16.0,
                ..default()
            },
            TextColor(Color::srgb(0.85, 0.9, 0.95)),
            RenderLayers::layer(2),
            CoordText,
        ))
        .set_parent(readout);
    commands
        .spawn((
            Text::new(""),
            TextFont {
                font_size: 16.0,
                ..default()
            },
            TextColor(Color::srgb(0.85, 0.9, 0.95)),
            RenderLayers::layer(2),
            LevelText,
        ))
        .set_parent(readout);
}

/// 切换器的一个胶囊单元格：节点上是 `Button` + `Interaction`，其标签
/// 作为子节点。标签实体被存放在 [`ModeSegment`] 上，因此样式系统
/// 无需每帧遍历 `Children` 就能访问它。
fn spawn_mode_segment(commands: &mut Commands, parent: Entity, mode: MapMode) {
    // 仅用 ASCII：内置的 FiraSans 没有 CJK/`°` 字形（会渲染成豆腐块）。
    let title = match mode {
        MapMode::ThreeD => "3D",
        MapMode::TwoD => "2D",
    };
    let label = commands
        .spawn((
            Text::new(title),
            TextFont {
                font_size: 15.0,
                ..default()
            },
            TextColor(SW_IDLE_TEXT),
            RenderLayers::layer(2),
        ))
        .id();
    let cell = commands
        .spawn((
            Button,
            Interaction::default(),
            Node {
                width: Val::Px(60.0),
                height: Val::Px(32.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            // 非激活单元格是透明的；外壳的玻璃会透出来。
            BackgroundColor(Color::NONE),
            BorderRadius::all(Val::Px(16.0)),
            RenderLayers::layer(2),
            ModeSegment { mode, label },
        ))
        .id();
    commands.entity(cell).set_parent(parent);
    commands.entity(label).set_parent(cell);
}

// ── 模式同步 / 切换 ──────────────────────────────────────────

/// 为当前激活模式只保留一个存活相机。`RenderLayers` 已经
/// 隔离了每个相机所看到的内容，所以我们只需启用匹配的那个并
/// 禁用另一个 —— 无双重渲染、无清除顺序歧义，且
/// 预先存在的 3D 场景实体永不被碰。
fn sync_camera_by_mode(
    mode: Res<MapMode>,
    mut cams: Query<(&mut Camera, Has<OrbitCamera>, Has<Map2dCamera>)>,
) {
    let two_d = matches!(*mode, MapMode::TwoD);
    for (mut c, orbit, flat) in cams.iter_mut() {
        if orbit {
            c.is_active = !two_d;
        } else if flat {
            c.is_active = two_d;
        }
    }
}

/// 将每个 UI 根钉到当前模式下实际激活的相机。
///
/// bevy_ui 将一棵 UI 树绑定到一个相机：`TargetCamera`，否则回退到
/// `DefaultUiCamera` —— 而后者只有当世界只有一个相机时才能解析。
/// `TargetCamera` 自己的文档也说明了这一点：*“如果世界只有一个
/// 相机则是可选的，否则必需。”* 我们至少有两只（orbit + flat），
/// 因此绑定是歧义的，会落在 query 恰好产出的那一只相机上。对
/// **非激活**相机会剔除 `DefaultCameraView`，所以若那个选择恰好是本
/// 模式的休眠相机，整个控件就会完全停止绘制 —— 这正是切换器在
/// 进入 2D 时可能消失的原因。
///
/// 布局由窗口推导且对两只相机相同，因此重新指向这些根是安全的。
/// 写入时会根据“已正确”做守卫，避免在每个空闲帧都产生一个变化
/// tick（以及一次子节点重传播）。
/// [`sync_ui_target_camera`] 的查询过滤器；拆出来是为了让 clippy 的
/// `type_complexity` 满意（嵌套的 `Has`/`Or` 元组会爆默认预算）。
/// 两个 `Query` 生命周期（`'w`、`'s`）保持为 separate 参数 —— 将它们
/// 归为一个会破坏 Bevy `.chain()` 所依赖的 `SystemParam` 约束。
type ModeCamsQuery<'w, 's> =
    Query<'w, 's, (Entity, Has<OrbitCamera>, Has<Map2dCamera>), With<Camera>>;
type UiRootsQuery<'w, 's> =
    Query<'w, 's, Entity, Or<(With<ModeSwitchRoot>, With<ReadoutRoot>)>>;

/// 根据当前地图模式将 UI 根的 [`TargetCamera`] 指向活动相机（2D 或 3D）。
fn sync_ui_target_camera(
    mode: Res<MapMode>,
    cams: ModeCamsQuery,
    roots: UiRootsQuery,
    bound: Query<&TargetCamera>,
    mut commands: Commands,
) {
    let two_d = matches!(*mode, MapMode::TwoD);
    let Some(active) = cams
        .iter()
        .find_map(|(e, orbit, flat)| ((orbit && !two_d) || (flat && two_d)).then_some(e))
    else {
        // 尚无相机拥有此模式（例如 orbit 相机晚些才 spawn）——
        // 下一帧重试，而不是把 UI 钉到空。
        return;
    };
    for root in roots.iter() {
        let already = matches!(bound.get(root), Ok(t) if t.0 == active);
        if !already {
            commands.entity(root).insert(TargetCamera(active));
        }
    }
}

/// 点击一个分段就选择该模式。故意*不是*切换按钮：点击已经
/// 激活的单元格是空操作，因此切换器绝不会在本想按另一个的用户
/// 手下反转。
fn on_mode_segment_click(
    mut mode: ResMut<MapMode>,
    cells: Query<(&ModeSegment, &Interaction), Changed<Interaction>>,
) {
    for (seg, i) in &cells {
        if *i == Interaction::Pressed && *mode != seg.mode {
            *mode = seg.mode;
            info!("[map2d] switched to {:?}", *mode);
        }
    }
}

/// 绘制切换器：激活 [`MapMode`] 的单元格带强调填充，每个单元格
/// 以可见变化响应悬停/按下。写入前先比较值，因此空闲帧不会
/// 引发 UI 变化 tick。
fn update_mode_segments(
    mode: Res<MapMode>,
    mut cells: Query<(&ModeSegment, &Interaction, &mut BackgroundColor)>,
    mut labels: Query<&mut TextColor>,
) {
    for (seg, interaction, mut bg) in cells.iter_mut() {
        let active = *mode == seg.mode;
        let hovered = matches!(*interaction, Interaction::Hovered);
        let pressed = matches!(*interaction, Interaction::Pressed);

        let want_bg = if active {
            if pressed {
                SW_ACCENT_PRESSED
            } else {
                SW_ACCENT
            }
        } else if pressed {
            SW_HOVER_PRESSED
        } else if hovered {
            SW_HOVER
        } else {
            Color::NONE
        };
        if bg.0 != want_bg {
            bg.0 = want_bg;
        }

        if let Ok(mut tc) = labels.get_mut(seg.label) {
            // 激活单元格与任何悬停单元格都显亮；只有静置的
            // 非激活单元格会被调暗。
            let want_tc = if active || hovered {
                SW_ACTIVE_TEXT
            } else {
                SW_IDLE_TEXT
            };
            if tc.0 != want_tc {
                tc.0 = want_tc;
            }
        }
    }
}

// ── 2D 相机交互 ────────────────────────────────────────

/// 左键拖拽平移平面图；光标下抓取 Geographic 世界点会粘住光标，
/// 且中心在经度上环绕（无限拖拽）。
/// 只修改 [`Map2dCam`]；[`apply_map2d_cam`] 将其写入相机。
fn map2d_pan_system(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    read: Query<(&Camera, &GlobalTransform), With<Map2dCamera>>,
    mut write: Query<&mut Map2dCam, With<Map2dCamera>>,
    mut anchor: ResMut<Map2dPanCursor>,
    capture: Option<Res<PlotInputCapture>>,
) {
    // 标绘 overlay 本帧拥有指针：退让并释放抓取锚点，以便释放
    // 捕获时不会恢复一个过期平移。
    if capture.is_some_and(|c| c.is_captured()) {
        anchor.last = None;
        return;
    }
    // 未在拖拽：释放锚点，以便下次按下重新播种而不发生跳变。
    if !mouse.pressed(MouseButton::Left) {
        anchor.last = None;
        return;
    }
    let Ok(win) = windows.get_single() else { return };
    // `cursor_position` 与 `viewport_to_world_2d` 共享同一空间（逻辑 px，左上
    // 原点），因此连续采样能干净地做差 —— 无 DPI 缩放、无轴翻转。
    // 这就是我们不再在此读取 `MouseMotion` 的原因。
    let Some(cursor) = win.cursor_position() else { return };
    let Ok((cam, ct)) = read.get_single() else { return };

    // 拖拽的第一帧：记住抓取起始位置，暂不移动。
    let Some(prev) = anchor.last.replace(cursor) else {
        return;
    };
    if cursor == prev {
        return;
    }

    // 抓图：上一个采样下的世界点落在当前采样下，因此影像精确
    // 地 1:1 跟随光标。
    let (Ok(w_now), Ok(w_prev)) = (
        cam.viewport_to_world_2d(ct, cursor),
        cam.viewport_to_world_2d(ct, prev),
    ) else {
        return;
    };
    let shift = w_prev - w_now;

    if let Ok(mut mc) = write.get_single_mut() {
        let mut center = mc.center + shift;
        center.x = wrap_x(center.x);
        center = clamp_center_y(center, mc.zoom, win.height());
        mc.center = center;
    }
}

/// 滚轮向光标方向缩放：指针下的 Geographic 点在缩放步进中保持
/// 固定（CesiumJS 风格的光标缩放）。
fn map2d_zoom_system(
    mut wheel: EventReader<MouseWheel>,
    windows: Query<&Window>,
    mut cams: Query<(&Camera, &GlobalTransform, &mut Map2dCam), With<Map2dCamera>>,
    capture: Option<Res<PlotInputCapture>>,
) {
    // 标绘 overlay 本帧拥有滚轮：消费事件，以便控制权交回时相机
    // 不会跳变，然后退让。
    if capture.is_some_and(|c| c.is_captured()) {
        wheel.clear();
        return;
    }
    let mut scroll = 0.0f32;
    for w in wheel.read() {
        scroll += w.y;
    }
    if scroll == 0.0 {
        return;
    }
    let Ok(win) = windows.get_single() else { return };
    let Some(cursor) = win.cursor_position() else { return };
    let Ok((cam, ct, mut mc)) = cams.get_single_mut() else { return };

    // 当前缩放下光标下的绝对 Geographic 世界点。
    let Ok(g) = cam.viewport_to_world_2d(ct, cursor) else {
        return;
    };

    let factor = (1.0 + 0.3_f32.min(scroll.abs()) * scroll.signum()).max(0.2);
    let zoom_min = min_zoom_for(win.height());
    let new_zoom = (mc.zoom * factor).clamp(zoom_min, ZOOM_MAX);
    // scale ∝ 1/zoom，因此要将 `g` 保持在光标下，中心需从中心向 `g`
    // 移动 (1 - old/new) 的比例距离。
    let ratio = 1.0 - mc.zoom / new_zoom;
    let mut center = mc.center + (g - mc.center) * ratio;
    center.x = wrap_x(center.x);
    center = clamp_center_y(center, new_zoom, win.height());

    mc.zoom = new_zoom;
    mc.center = center;
}

/// 单一写入者：将 [`Map2dCam`] 的中心/缩放推到存活相机的 transform +
/// 正交缩放（`1 / zoom`）上。与输入系统分开保留，使它们能在映射
/// 光标时读取 `Camera`/`GlobalTransform`，而不同时可变地持有它们。
fn apply_map2d_cam(
    mut q: Query<(&mut Transform, &mut Projection, &Map2dCam), With<Map2dCamera>>,
) {
    for (mut tf, mut proj, mc) in q.iter_mut() {
        tf.translation = Vec3::new(mc.center.x, mc.center.y, CAM_Z);
        if let Projection::Orthographic(p) = proj.as_mut() {
            p.scale = 1.0 / mc.zoom;
        }
    }
}

/// 将应用的视图状态镜像到标绘桥接的 [`PlotViewCtx`]。
///
/// 桥接（一个适配器）不得导入本应用层，因此应用每帧将它拥有的
/// 内容 —— 当前 [`MapMode`]、窗口尺寸与平面图缩放 —— 推入共享
/// 资源。使用 `Option<ResMut>` 以便在标绘桥接插件未注册（无头，
/// 或 `CESIUM_ENABLE_PLOT=0`）时 2D 路径仍有效：此时本系统是一个
/// 无害的空操作。
fn sync_plot_view_ctx(
    mode: Res<MapMode>,
    windows: Query<&Window>,
    cams: Query<&Map2dCam, With<Map2dCamera>>,
    ctx: Option<ResMut<PlotViewCtx>>,
) {
    let Some(mut ctx) = ctx else { return };
    ctx.mode = match *mode {
        MapMode::ThreeD => PlotViewMode::Globe,
        MapMode::TwoD => PlotViewMode::Flat,
    };
    if let Ok(win) = windows.get_single() {
        ctx.screen_w = win.width();
        ctx.screen_h = win.height();
    }
    if let Ok(mc) = cams.get_single() {
        ctx.flat_zoom = mc.zoom;
    }
}

// ── 读数 ──────────────────────────────────────────────

fn update_readout(
    mode: Res<MapMode>,
    windows: Query<&Window>,
    read: Query<(&Camera, &GlobalTransform), With<Map2dCamera>>,
    mut texts: Query<(&mut Text, Has<CoordText>, Has<LevelText>)>,
    cams: Query<&Map2dCam, With<Map2dCamera>>,
) {
    if !matches!(*mode, MapMode::TwoD) {
        // 在 2D→3D 过渡时清除读数一次，使陈旧的 lon/lat 文本
        // 永不残留在地球上。`is_changed` 将其限制为一帧。
        // 仅限于读数标签：此处一次未过滤的清扫曾连切换器的标题
        // 一并置空，在 3D 中留下一个无文字按钮。
        if mode.is_changed() {
            for (mut t, is_coord, is_level) in texts.iter_mut() {
                if (is_coord || is_level) && !t.is_empty() {
                    t.0.clear();
                }
            }
        }
        return;
    }
    let Ok(win) = windows.get_single() else { return };
    let Ok((cam, ct)) = read.get_single() else { return };

    // 单个 `&mut Text` 查询（带 `Has<..>` 角色标签）避免了两个独立
    // `&mut Text` 查询会引发的 ECS 冲突。
    let level = cams.get_single().ok().map(|mc| approx_zoom_level(mc.zoom));
    let lonlat = win.cursor_position().and_then(|cursor| {
        cam.viewport_to_world_2d(ct, cursor).ok().map(|world| {
            let lon = wrap_lon_deg(world.x.to_degrees());
            let lat = world.y.to_degrees().clamp(-90.0, 90.0);
            (lon, lat)
        })
    });

    for (mut t, is_coord, is_level) in texts.iter_mut() {
        if is_coord {
            if let Some((lon, lat)) = lonlat {
                // 仅用 ASCII：内置的 FiraSans 没有 `°`/`≈` 字形（会渲染成豆腐块）。
                t.0 = format!("lon {lon:.4} deg   lat {lat:.4} deg");
            }
        } else if is_level {
            if let Some(z) = level {
                t.0 = format!("zoom level ~ {z}");
            }
        }
    }
}

// ── 纯函数助手（已单元测试）─────────────────────────

/// 将 Geographic x（经度·R，R = 1）包裹到 [-π, π)，以便越过反子午线
/// 拖拽时无缝延续。
fn wrap_x(x: f32) -> f32 {
    let mut v = x;
    while v > std::f32::consts::PI {
        v -= WORLD_W;
    }
    while v < -std::f32::consts::PI {
        v += WORLD_W;
    }
    v
}

/// 世界完整纬度带（±90°，高度 `2·LAT_MAX = π`）仍能覆盖一个
/// `canvas_h` 像素高的视口所需的最小缩放（px / 世界单位）。
/// 缩小超过此值会在地图上/下留下空白边距，因此它是
/// 有效下界（Google-maps 风格“整个世界填满画面”）。
fn min_zoom_for(canvas_h: f32) -> f32 {
    (canvas_h / (2.0 * LAT_MAX)).max(ZOOM_MIN)
}

/// 约束相机中心，使影像始终在垂直方向填满视口：当整个世界
/// 都能放下（缩出）时钉到赤道；否则将可见带保持在 ±90° 内，
/// 使极地边缘从不露出空白背景。经度保持自由（它通过 [`wrap_x`] 无限环绕）。
fn clamp_center_y(center: Vec2, zoom: f32, canvas_h: f32) -> Vec2 {
    let half_h = canvas_h * 0.5 / zoom;
    let cy = if half_h >= LAT_MAX {
        0.0
    } else {
        center.y.clamp(-LAT_MAX + half_h, LAT_MAX - half_h)
    };
    Vec2::new(center.x, cy)
}

/// 将度数经度归一化到 [-180, 180]。
fn wrap_lon_deg(mut lon: f32) -> f32 {
    while lon > 180.0 {
        lon -= 360.0;
    }
    while lon < -180.0 {
        lon += 360.0;
    }
    lon
}

/// 瓦片尺寸规则的反函数：在当前缩放（px / 世界单位，其中一个
/// 世界宽 `WORLD_W` 单位）下，一个瓦片约 `MAX_TILE_SCREEN_PX` 宽时的
/// Web Mercator 级别。
fn approx_zoom_level(zoom: f32) -> i32 {
    const MAX_TILE_SCREEN_PX: f32 = 288.0;
    // tile_px(z) = zoom * WORLD_W / 2^z == MAX_TILE_SCREEN_PX  →  z.
    let z = (zoom * WORLD_W / MAX_TILE_SCREEN_PX).log2();
    z.max(0.0).round() as i32
}

// ── P2：影像瓦片层 ──────────────────────────────────

/// 选取 Web-Mercator 瓦片级别，使一个原生 256px 瓦片在当前 `zoom`
/// （px / 世界单位）下覆盖约 `TILE_PX_TARGET` 屏幕像素。
fn tile_zoom_for(zoom: f32) -> u32 {
    let z = (f64::from(zoom) * TAU6 / TILE_PX_TARGET as f64).log2().round() as i32;
    z.clamp(TILE_Z_MIN, TILE_Z_MAX) as u32
}

/// Mercator 行 `row` 北缘的 Geographic 纬度（弧度）。
fn row_to_lat(row: f64, n: f64) -> f64 {
    (PI6 * (1.0 - 2.0 * row / n)).sinh().atan()
}

/// [`row_to_lat`] 的反函数：纬度 `lat` 的（小数）Mercator 行。
/// 被约束在略靠极地内侧，使 `tan`/`asinh` 永不爆炸到无穷。
fn lat_to_row(lat: f64, n: f64) -> f64 {
    let l = lat.clamp(-1.4844, 1.4844);
    (1.0 - l.tan().asinh() / PI6) * 0.5 * n
}

/// 原始列 `col`、Mercator `row`、在级别分母 `n` 下 Geographic 矩形的
/// 中心 + 尺寸（世界单位）。`col` 可能在 `[0, n)` 之外；返回的 x 随后
/// 位于 ±π 之外，而这正是使经度环绕看起来连续的原因。
fn tile_rect(col: f64, row: f64, n: f64) -> (f32, f32, f32, f32) {
    let x0 = col / n * TAU6 - PI6;
    let x1 = (col + 1.0) / n * TAU6 - PI6;
    let yn = row_to_lat(row, n);
    let ys = row_to_lat(row + 1.0, n);
    (
        ((x0 + x1) * 0.5) as f32,
        ((yn + ys) * 0.5) as f32,
        (x1 - x0) as f32,
        (yn - ys) as f32,
    )
}

/// 将 XYZ 瓦片坐标转换为 Bing Maps quadkey。
fn quadkey(x: u32, y: u32, level: u32) -> String {
    let mut s = String::with_capacity(level as usize);
    for i in (0..level).rev() {
        let m = 1u32 << i;
        let mut d = 0u8;
        if x & m != 0 {
            d |= 1;
        }
        if y & m != 0 {
            d |= 2;
        }
        s.push((b'0' + d) as char);
    }
    s
}

/// XY 平面内的一个单位 quad（局部范围 [-0.5, 0.5]），UV 填满 [0, 1]，因此
/// v=0 是瓦片的北缘（下载图像的第 0 行）。从 +Z 看按 CCW 绕序，使正面
/// 朝向俯视相机。
fn build_unit_quad() -> Mesh {
    let positions = [
        [-0.5, 0.5, 0.0],
        [0.5, 0.5, 0.0],
        [-0.5, -0.5, 0.0],
        [0.5, -0.5, 0.0],
    ];
    let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
    let normals = [[0.0, 0.0, 1.0]; 4];
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.to_vec());
    mesh.insert_indices(Indices::U16(vec![0, 2, 3, 0, 3, 1]));
    mesh
}

/// 一个目标瓦片：`(placement_key, (canonical_key, rect))`。放置键是
/// `(raw_col, raw_row, level)` —— 级别是键的一部分，因为一个视图会将
/// 目标级瓦片与其下方作为回退显示的更粗祖先混在一起。rect 是
/// Geographic 世界单位下的 `(centre_x, centre_y, width, height)`。
type DesiredTile = HashMap<(i64, i64, u32), ((u32, u32, u32), (f32, f32, f32, f32))>;

/// 决定级别 `z` 下目标单格 `(col, row)` 实际应显示哪个瓦片：当 `has`
/// 报告已缓存时显示单格自身的瓦片，否则显示最近的已缓存祖先（一次
/// 向上走一级）。当链上什么都没有缓存时，它在最粗级别 `TILE_Z_MIN` 停下，
/// 因此调用方仍会放置一个（隐藏）占位格并持续请求精确瓦片。
///
/// 返回 `(place_col, place_row, place_level, canonical_key)` —— 前三个通过
/// [`tile_rect`] 定位选中瓦片的矩形，最后一个是用于绘制的缓存键。`col`
/// 可能位于 `[0, 2^z)` 之外（经度环绕）；`div_euclid`/`rem_euclid` 配对使
/// 祖先列与之保持一致。
fn resolve_tile_cell(
    col: i64,
    row: i64,
    z: u32,
    has: impl Fn((u32, u32, u32)) -> bool,
) -> (i64, i64, u32, (u32, u32, u32)) {
    let mut lvl = z;
    loop {
        let span = 1i64 << (z - lvl);
        let acol = col.div_euclid(span);
        let arow = row.div_euclid(span);
        let an = 1i64 << lvl;
        let key = (acol.rem_euclid(an) as u32, arow as u32, lvl);
        if has(key) || lvl == TILE_Z_MIN as u32 {
            return (acol, arow, lvl, key);
        }
        lvl -= 1;
    }
}

/// 两个「中心 + 尺寸」矩形是否相交（`(cx, cy, w, h)`，平面地图世界坐标）。
///
/// 用严格 `<`：恰好相接（共享一条边）不算相交，因此相邻瓦片互不影响。
fn rectangles_overlap(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    (ax - bx).abs() < (aw + bw) * 0.5 && (ay - by).abs() < (ah + bh) * 0.5
}

/// 一个已放置的瓦片实体（`placement = (col, row, level)`，即
/// [`Map2dTiler::live`] 的键）现在是否可以退役（despawn）。
///
/// 背景：快速缩小时目标级瓦片往往还在下载中。若此时立刻销毁上一级别的细
/// 瓦片，屏幕上就只剩 `Hidden` 的（尚无纹理的）粗瓦片 —— 读作整片纯黑，
/// 即清屏色。旧放置与新放置覆盖同一片地面、且按级别堆叠（更细在上），因此
/// 在「覆盖它的每个新放置都已就绪（有纹理可渲染）」之前保留旧放置是无缝的。
///
/// 返回 `true` 当且仅当：
/// - 没有任何 `desired` 放置与它相交 —— 它已离开视口，不会再被看到；或
/// - 每个与它相交的 `desired` 放置都已就绪（`has(canonical_key)`）。
fn placement_can_retire(
    placement: (i64, i64, u32),
    desired: &DesiredTile,
    has: impl Fn((u32, u32, u32)) -> bool,
) -> bool {
    let (col, row, level) = placement;
    // 与步骤 4 构造 `desired` 时同一个约定：放置键是**未包裹**的格子坐标，
    // 矩形也按同一 `tile_rect` 规则重算，因此两者可直接比较。
    let rect = tile_rect(col as f64, row as f64, (1i64 << level) as f64);
    for (canonical, drect) in desired.values() {
        if rectangles_overlap(rect, *drect) && !has(*canonical) {
            return false;
        }
    }
    true
}

/// 为当前视口维护平面图影像层：协调可见瓦片集（spawn/despawn
/// quad），为我们尚未拥有的瓦片启动下载，并在缓存纹理到达时将其
/// 绘制到实体上。
///
/// 每个可见单格显示它能得到的最佳影像：已缓存时显示自身瓦片，
/// 否则以祖先的原生矩形绘制最近已下载的祖先瓦片。瓦片按级别堆叠
/// （更细在上），因此放大时保持粗影像可见，直到更细瓦片到达 ——
/// 无空白、无白闪。任何级别都无缓存的单格会 spawn `Hidden`，并在
/// 其图像到达后由绘制步骤揭示。
#[allow(clippy::too_many_arguments)]
fn update_map2d_tiles(
    mut commands: Commands,
    mut tiler: ResMut<Map2dTiler>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window>,
    cams: Query<(&Camera, &GlobalTransform, &Map2dCam), With<Map2dCamera>>,
    mut live: Query<(&Map2dTile, &MeshMaterial3d<StandardMaterial>, &mut Visibility)>,
) {
    // 1. 将已完成的下载排空进纹理缓存。
    let drained: Vec<Map2dTileImg> = {
        let rx = tiler.rx.lock().unwrap();
        let mut v = Vec::new();
        while let Ok(t) = rx.try_recv() {
            v.push(t);
        }
        v
    };
    for t in drained {
        tiler.in_flight.remove(&(t.x, t.y, t.z));
        let img = Image::new(
            Extent3d {
                width: t.width,
                height: t.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            t.rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        tiler.cache.insert((t.x, t.y, t.z), images.add(img));
    }

    // 2. Read the viewport as a Geographic world rectangle.
    let Ok(win) = windows.get_single() else { return };
    let (Ok(cam), Ok(ct), Ok(mc)) = (
        cams.get_single().map(|c| c.0),
        cams.get_single().map(|c| c.1),
        cams.get_single().map(|c| c.2),
    ) else {
        return;
    };
    let Ok(tl) = cam.viewport_to_world_2d(ct, Vec2::ZERO) else { return };
    let Ok(br) = cam.viewport_to_world_2d(ct, Vec2::new(win.width(), win.height())) else {
        return;
    };

    // 3. 解析目标级别。级别变化不再清空该层：精确瓦片未缓存的
    //    单格会持续显示其最佳缓存祖先，因此放大从不闪白。
    let z = tile_zoom_for(mc.zoom);
    let ni = 1i64 << z;
    let n = ni as f64;

    // 4. 枚举可见的目标矩形。列越过 ±π 边缘并通过 `rem_euclid`
    //    环绕；行被约束到 [0, n)。每个目标单格沿级别阶梯向上走到最粗
    //    的已缓存瓦片，并将*那个*瓦片以其自身原生矩形放置。不同的放置
    //    会去重，因此一个缓存祖先可代替其全部仍缺失的后代；一个后到的
    //    更细瓦片会绘制在上方（更高 z），而一旦其下方再无任何东西需要
    //    该祖先，祖先就退出。
    let col_start = ((f64::from(tl.x) + PI6) / TAU6 * n).floor() as i64;
    let col_end = ((f64::from(br.x) + PI6) / TAU6 * n).floor() as i64;
    let row_start = lat_to_row(f64::from(tl.y), n).floor().max(0.0) as i64;
    let row_end = lat_to_row(f64::from(br.y), n)
        .ceil()
        .min((ni - 1) as f64) as i64;

    let mut desired: DesiredTile = HashMap::new();
    // 我们请求下载的目标级瓦片；作为回退显示的祖先按构造已缓存，
    // 因此从不需获取。
    let mut wants: Vec<(u32, u32, u32)> = Vec::new();
    for row in row_start..=row_end {
        for col in col_start..=col_end {
            wants.push((col.rem_euclid(ni) as u32, row as u32, z));
            let (acol, arow, lvl, akey) =
                resolve_tile_cell(col, row, z, |k| tiler.cache.contains_key(&k));
            let arect = tile_rect(acol as f64, arow as f64, (1i64 << lvl) as f64);
            desired.insert((acol, arow, lvl), (akey, arect));
        }
    }

    // 5. 退役不再需要的放置（已离开视口，或被刚下载完成的更细瓦片取代）。
    //    快速缩小时目标级瓦片可能仍在下载中：此时立刻销毁上一级别的细瓦片会
    //    露出无纹理（`Hidden`）的粗瓦片，读作整片纯黑（清屏色）。因此只要还有
    //    覆盖该放置的新放置尚未就绪，就让旧放置继续顶替显示，等粗瓦片到达后
    //    再无缝退役（见 [`placement_can_retire`]）。
    let live_keys: Vec<(i64, i64, u32)> = tiler.live.keys().copied().collect();
    let mut gone: Vec<(i64, i64, u32)> = Vec::new();
    for pk in live_keys {
        if desired.contains_key(&pk) {
            continue;
        }
        if placement_can_retire(pk, &desired, |k| tiler.cache.contains_key(&k)) {
            gone.push(pk);
        }
    }
    for pk in gone {
        if let Some(e) = tiler.live.remove(&pk) {
            commands.entity(e).despawn();
        }
    }

    // 6. 确保共享的单位 quad 网格存在。
    if tiler.unit_quad.is_none() {
        tiler.unit_quad = Some(meshes.add(build_unit_quad()));
    }
    let quad = tiler.unit_quad.clone().unwrap();

    // 7. Spawn 新放置。高度随级别递增，因此更细瓦片总会覆盖其下方
    //    更粗的回退；任何级别都无缓存的单格会 spawn `Hidden`，并由步骤 9
    //    揭示。
    for (pk, (canon, (cx, cy, w, h))) in &desired {
        if tiler.live.contains_key(pk) {
            continue;
        }
        let tex = tiler.cache.get(canon).cloned();
        let mat = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: tex.clone(),
            unlit: true,
            ..default()
        });
        let vis = if tex.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        let e = commands
            .spawn((
                Map2dTile { key: *canon },
                Mesh3d(quad.clone()),
                MeshMaterial3d(mat),
                Transform::from_xyz(*cx, *cy, TILE_Z_ELEV + pk.2 as f32)
                    .with_scale(Vec3::new(*w, *h, 1.0)),
                RenderLayers::layer(1),
                vis,
            ))
            .id();
        tiler.live.insert(*pk, e);
    }

    // 8. 为我们尚未拥有 / 尚未获取的目标级瓦片排队下载。
    wants.sort_unstable();
    wants.dedup();
    for key in wants {
        if !tiler.cache.contains_key(&key) && tiler.in_flight.insert(key) {
            let _ = tiler.job_tx.send(key);
        }
    }

    // 9. 将缓存纹理绘制到仍然空白的已存活实体上（处理一个瓦片先于
    //    其图像 spawn 的单帧竞态）。
    for (tile, mat_handle, mut vis) in live.iter_mut() {
        if let Some(h) = tiler.cache.get(&tile.key) {
            if let Some(m) = materials.get_mut(mat_handle) {
                if m.base_color_texture.is_none() {
                    m.base_color_texture = Some(h.clone());
                }
            }
            if *vis != Visibility::Visible {
                *vis = Visibility::Visible;
            }
        }
    }
}

/// 一个 2D 下载工作线程：优先命中本地磁盘瓦片缓存（`tile_cache`，与 3D
/// 共享）；未命中则经进程级共享连接池获取 Bing Aerial 瓦片（或在设置了
/// `OFFLINE_IMAGERY_ROOT` 时从磁盘读取 `{root}/{z}/{x}/{y}.png`），解码为
/// RGBA 并将结果送回。
fn map2d_worker(job_rx: Arc<Mutex<mpsc::Receiver<(u32, u32, u32)>>>, tx: mpsc::Sender<Map2dTileImg>) {
    let offline_root = crate::feature_flags::offline_imagery_root();
    loop {
        let job = job_rx.lock().unwrap().recv();
        let Ok((x, y, z)) = job else { return };

        let bytes: Option<Vec<u8>> = if let Some(root) = &offline_root {
            let path = root
                .join(z.to_string())
                .join(x.to_string())
                .join(format!("{y}.png"));
            std::fs::read(path).ok()
        } else {
            // 在线：命中磁盘缓存直接用（零网络）；否则走共享连接池下载并落盘。
            crate::tile_cache::fetch_tile(x, y, z, &quadkey(x, y, z))
        };

        if let Some(data) = bytes {
            if let Ok(img) = image::load_from_memory(&data) {
                let rgba = img.to_rgba8();
                // 与 3D 地球路径（globe_pipeline）保持一致：丢弃 Bing 那种
                // 平滑、明亮、冷色调的"无影像"占位图。若不在此过滤，拖动时
                // 会以难看的纯色块铺在平面上（正是用户看到的蓝色小块）。
                // 跳过发送即可：该键留在在途集不再重试，回退逻辑
                // （最近已缓存祖先 / 隐藏占位）自然接管。
                let (is_ph, _, _) = crate::globe_textures::is_placeholder_tile(&rgba);
                if is_ph {
                    continue;
                }
                let (width, height) = rgba.dimensions();
                let _ = tx.send(Map2dTileImg {
                    x,
                    y,
                    z,
                    rgba: rgba.into_raw(),
                    width,
                    height,
                });
            }
        }
    }
}

// ── 测试 ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod map2d_tests {
    use super::*;

    /// 矩形相交：真实重叠为真，仅共享一条边（或一个角）为假 —— 相邻瓦片
    /// 不能因为相接就被判为互相覆盖。
    #[test]
    fn rectangles_overlap_ignores_touching_edges() {
        let a = (0.0, 0.0, 2.0, 2.0);
        assert!(rectangles_overlap(a, (1.0, 0.0, 2.0, 2.0)));
        assert!(rectangles_overlap(a, (0.0, 0.5, 2.0, 2.0)));
        // 恰好相接（边）与恰好对角相接都不算相交。
        assert!(!rectangles_overlap(a, (2.0, 0.0, 2.0, 2.0)));
        assert!(!rectangles_overlap(a, (2.0, 2.0, 2.0, 2.0)));
        // 完全包含仍然是相交。
        assert!(rectangles_overlap(a, (0.0, 0.0, 0.5, 0.5)));
        // 对称。
        assert!(rectangles_overlap((1.0, 0.0, 2.0, 2.0), a));
    }

    /// 快速缩小时旧级别放置的退役条件：只要覆盖它的新放置还没就绪就必须
    /// 留着（否则屏幕会只剩 `Hidden` 的粗瓦片 = 纯黑）。
    #[test]
    fn placement_waits_for_overlapping_ready_replacement() {
        // 级别 9 的格子 (0, 0)；它的父格（级别 8）覆盖同一片地面。
        let placement = (0i64, 0i64, 9u32);
        let mut desired: DesiredTile = HashMap::new();
        let parent_rect = tile_rect(0.0, 0.0, (1i64 << 8) as f64);
        desired.insert((0, 0, 8), ((7u32, 3u32, 8u32), parent_rect));

        // 父格还没贴图 → 不能退役。
        assert!(!placement_can_retire(placement, &desired, |_| false));
        // 父格就绪 → 可以退役。
        assert!(placement_can_retire(placement, &desired, |_| true));
        // 只对**与它相交**的放置负责：另一个半球的放置未就绪不影响退役。
        let far_rect = tile_rect(512.0, 0.0, (1i64 << 8) as f64);
        let mut far_only: DesiredTile = HashMap::new();
        far_only.insert((0, 0, 8), ((9u32, 9u32, 8u32), far_rect));
        assert!(placement_can_retire(placement, &far_only, |_| false));
        // 没有任何相交放置（已离开视口）→ 立即退役。
        assert!(placement_can_retire(placement, &DesiredTile::new(), |_| false));
    }

    /// 更细的子格不阻塞父格退役方向上的判定，但**子格自己**会被「未就绪的
    /// 父格」阻塞 —— 这正是缩小时要保住的旧影像。
    #[test]
    fn finer_placement_is_retained_until_parent_ready() {
        let mut desired: DesiredTile = DesiredTile::new();
        let parent_rect = tile_rect(3.0, 2.0, (1i64 << 7) as f64);
        desired.insert((3, 2, 7), ((1u32, 1u32, 7u32), parent_rect));
        // 该父格覆盖的四个子格（级别 8）都还没退役。
        for sub in [(6i64, 4i64, 8u32), (7, 4, 8), (6, 5, 8), (7, 5, 8)] {
            assert!(
                !placement_can_retire(sub, &desired, |_| false),
                "{sub:?} 必须等父格就绪"
            );
            assert!(placement_can_retire(sub, &desired, |_| true));
        }
    }

    #[test]
    fn wrap_x_is_periodic_and_bounded() {
        let pi = std::f32::consts::PI;
        // 刚过反子午线会环绕到最西侧，无缝。
        assert!((wrap_x(pi + 0.1) - (-pi + 0.1)).abs() < 1e-4);
        assert!((wrap_x(-pi - 0.1) - (pi - 0.1)).abs() < 1e-4);
        for k in -6..=6 {
            let x = wrap_x(k as f32 * 1.3);
            assert!(x >= -pi - 1e-3 && x <= pi + 1e-3, "out of range: {x}");
        }
    }

    #[test]
    fn wrap_lon_deg_bounds() {
        assert!((wrap_lon_deg(190.0) - (-170.0)).abs() < 1e-4);
        assert!((wrap_lon_deg(-370.0) - (-10.0)).abs() < 1e-4);
        assert!((wrap_lon_deg(45.0) - 45.0).abs() < 1e-4);
    }

    #[test]
    fn zoom_level_monotonic_and_reasonable() {
        let low = approx_zoom_level(ZOOM_MIN);
        let high = approx_zoom_level(ZOOM_MAX);
        assert!(high > low, "level must grow with zoom ({low} -> {high})");
        assert!(low >= 0);
    }

    #[test]
    fn row_lat_is_monotonic_and_symmetric() {
        let n = 256.0;
        // 第 0 行是最北，中间行是赤道，最后一行是最南。
        assert!(row_to_lat(0.0, n) > 1.4, "top row near north pole");
        assert!(row_to_lat(n / 2.0, n).abs() < 1e-9, "middle row is the equator");
        assert!(row_to_lat(n, n) < -1.4, "bottom row near south pole");
        // Row <-> lat 往返。
        for row in [3.0, 40.0, 128.0, 200.0] {
            let lat = row_to_lat(row, n);
            assert!((lat_to_row(lat, n) - row).abs() < 1e-6, "round trip {row}");
        }
    }

    #[test]
    fn tile_rect_tiles_the_world_exactly() {
        let n = 16.0;
        let (c0, _, w, _) = tile_rect(0.0, 0.0, n);
        let (c1, _, w1, _) = tile_rect(1.0, 0.0, n);
        // 所有列共享一个宽度 == 世界周期 / n。
        let world_w = (TAU6 / n) as f32;
        assert!((w - world_w).abs() < 1e-3);
        assert!((w - w1).abs() < 1e-6, "uniform columns");
        // 相邻列中心恰好相隔一个宽度（无缝）。
        assert!((c1 - c0 - w).abs() < 1e-3, "column pitch == width");
        // 列 0 的西缘是 -pi；列 n 的西缘是 +pi（一个世界）。
        assert!((c0 - w / 2.0 + PI6 as f32).abs() < 1e-2, "col 0 west == -pi");
        let (cn, _, wn, _) = tile_rect(n, 0.0, n);
        assert!((cn - wn / 2.0 - PI6 as f32).abs() < 1e-2, "col n west == +pi");
        // 行互相紧接：行 4 南缘 == 行 5 北缘。
        let (_, y4, _, h4) = tile_rect(0.0, 4.0, n);
        let (_, y5, _, h5) = tile_rect(0.0, 5.0, n);
        assert!(
            (y4 - h4 / 2.0 - (y5 + h5 / 2.0)).abs() < 1e-4,
            "rows abut"
        );
    }

    #[test]
    fn quadkey_known_value() {
        // x=3, y=5, z=3 -> "213"（标准 Bing/OSM quadkey）。
        assert_eq!(quadkey(3, 5, 3), "213");
        assert_eq!(quadkey(0, 0, 0), "");
        assert_eq!(quadkey(1, 1, 1), "3");
    }

    #[test]
    fn resolve_prefers_exact_then_nearest_ancestor() {
        use std::collections::HashSet;
        let exact: HashSet<(u32, u32, u32)> = [(5, 3, 4)].into_iter().collect();
        assert_eq!(
            resolve_tile_cell(5, 3, 4, |k| exact.contains(&k)),
            (5, 3, 4, (5, 3, 4)),
            "cached exact tile wins"
        );

        // (5, 3) 在 z=4 → z=3 父级 (2, 1) → z=2 祖级 (1, 0)：尽管请求了
        // 更细级别，仍选中已缓存的那一级。
        let gp: HashSet<(u32, u32, u32)> = [(1, 0, 2)].into_iter().collect();
        assert_eq!(
            resolve_tile_cell(5, 3, 4, |k| gp.contains(&k)),
            (1, 0, 2, (1, 0, 2)),
            "falls back to the nearest cached ancestor"
        );
    }

    #[test]
    fn resolve_without_any_cache_stops_at_coarsest() {
        use std::collections::HashSet;
        let none: HashSet<(u32, u32, u32)> = HashSet::new();
        let (c, r, l, k) = resolve_tile_cell(5, 3, 4, |x| none.contains(&x));
        assert_eq!(l, TILE_Z_MIN as u32, "nothing cached → coarsest level");
        assert_eq!((c, r, k), (0, 0, (0, 0, TILE_Z_MIN as u32)));
    }

    #[test]
    fn resolve_wraps_negative_columns_to_parent() {
        use std::collections::HashSet;
        // z=2 下 col -1 环绕到规范 x=3；其 z=1 父级是 x=1，row 3>>1=1。
        let parent: HashSet<(u32, u32, u32)> = [(1, 1, 1)].into_iter().collect();
        assert_eq!(
            resolve_tile_cell(-1, 3, 2, |k| parent.contains(&k)),
            (-1, 1, 1, (1, 1, 1)),
            "wrapped column maps to its ancestor on the far side"
        );
    }

    #[test]
    fn tile_zoom_grows_with_zoom_and_stays_bounded() {
        assert!(tile_zoom_for(ZOOM_MAX) > tile_zoom_for(ZOOM_MIN));
        for z in [ZOOM_MIN, 500.0, 2000.0, ZOOM_MAX] {
            let t = tile_zoom_for(z);
            assert!((TILE_Z_MIN as u32..=TILE_Z_MAX as u32).contains(&t), "z {t}");
        }
    }

    #[test]
    fn clamp_center_pins_world_and_bounds_poles() {
        // 缩出使整个世界垂直放下 -> 钉到赤道，经度不变。
        let c = clamp_center_y(Vec2::new(0.5, 1.4), 100.0, 700.0);
        assert_eq!(c.y, 0.0);
        assert_eq!(c.x, 0.5);
        // 放大 -> 将带保持在 ±90 内，使极地从不显示空白。
        let lim = LAT_MAX - 700.0 * 0.5 / 2000.0;
        let north = clamp_center_y(Vec2::new(0.0, 5.0), 2000.0, 700.0);
        assert!((north.y - lim).abs() < 1e-4, "north clamp");
        let south = clamp_center_y(Vec2::new(0.0, -5.0), 2000.0, 700.0);
        assert!((south.y + lim).abs() < 1e-4, "south clamp");
    }

    #[test]
    fn min_zoom_fills_frame_and_respects_floor() {
        // 一个 727px 高的 canvas 需要 ~727/π px/unit 才能让高 π 的世界填满。
        let m = min_zoom_for(727.0);
        assert!(m >= ZOOM_MIN);
        assert!((m - 727.0 / std::f32::consts::PI).abs() < 1.0, "m = {m}");
        // 一个极小的 canvas 回退到固定下界。
        assert_eq!(min_zoom_for(100.0), ZOOM_MIN);
    }

    /// “进入 2D 后切换器消失”的回归守卫。拥有两只相机时，UI 根必须
    /// 重新钉到当前 [`MapMode`] 下存活的那只相机，否则 bevy_ui 丢弃其
    /// `DefaultCameraView` 且整个控件停止绘制。在一个最小世界上驱动真正的
    /// sync 系统，并检查根的 [`TargetCamera`] 跟随模式。
    #[test]
    fn ui_roots_follow_active_camera_in_each_mode() {
        let mut app = App::new();
        app.init_resource::<MapMode>();
        app.add_systems(
            Update,
            (sync_camera_by_mode, sync_ui_target_camera).chain(),
        );

        let orbit;
        let flat;
        let switch_root;
        let readout_root;
        {
            let world = app.world_mut();
            orbit = world.spawn((Camera::default(), OrbitCamera)).id();
            flat = world.spawn((Camera::default(), Map2dCamera)).id();
            switch_root = world.spawn(ModeSwitchRoot).id();
            readout_root = world.spawn(ReadoutRoot).id();
        }

        // 默认（ThreeD）：两个根都瞄准 orbit 相机。
        app.update();
        assert_eq!(app.world().get::<TargetCamera>(switch_root).map(|t| t.0), Some(orbit));
        assert_eq!(app.world().get::<TargetCamera>(readout_root).map(|t| t.0), Some(orbit));

        // 切到 2D：两个根必须重新指向 flat 相机 —— 正是旧构建丢失控件的
        // 那一刻。
        *app.world_mut().resource_mut::<MapMode>() = MapMode::TwoD;
        app.update();
        assert_eq!(app.world().get::<TargetCamera>(switch_root).map(|t| t.0), Some(flat));
        assert_eq!(app.world().get::<TargetCamera>(readout_root).map(|t| t.0), Some(flat));

        // 再切回 3D。
        *app.world_mut().resource_mut::<MapMode>() = MapMode::ThreeD;
        app.update();
        assert_eq!(app.world().get::<TargetCamera>(switch_root).map(|t| t.0), Some(orbit));
    }
}
