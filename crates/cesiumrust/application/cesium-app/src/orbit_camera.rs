//! Orbit 相机控制器 —— 左键拖拽旋转，中键拖拽倾斜，右键拖拽 / 滚轮缩放
//! （改变视角高度）。
//!
//! 模仿 CesiumJS 默认 ScreenSpaceCameraController 的行为：
//! 左键拖拽绕地球轨道运行（`rotateEventTypes = LEFT_DRAG`）；右键竖直拖拽与
//! 滚轮改变视角高度（`zoomEventTypes = [RIGHT_DRAG, WHEEL, PINCH]`，触屏
//! 捏合不在桌面范围内），**向上拖 = 放大、向下拖 = 缩小**（与滚轮一致；这一点
//! 与 CesiumJS `handleZoom` 的 `approachingSurface` 刻意相反）；中键拖拽倾斜
//! 视角，即绕画面中心拾取到的地表点做轨道式旋转（`tiltEventTypes` 首项
//! `MIDDLE_DRAG` → `tilt3D` → `tilt3DOnEllipsoid`，见 [`apply_tilt_drag`]）。
//! 缩放两种手势共用同一套「地表高度」语义，见 [`zoom_drag_target_surface`]。
//!
//! 地球处于 ECEF 朝向（北极在 +Z，赤道在 XY 平面），
//! 因此相机绕 Z（极地）轴轨道运行并以 Z 为“上”。
//!
//! ## M2.4 薄壳委派
//!
//! 旋转惯性与飞行插值委派给领域层
//! （`cesium_interaction::{InertiaController, InertiaSample, decay, CameraFlight,
//! compute_flight_duration, select_flight_easing}`）。抓地跟踪
//! 公式与缩放惯性滑行均逐字节保留自久经考验的
//! M0 实现。

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::prelude::*;
use bevy::render::view::RenderLayers;
use cesium_interaction::{
    compute_flight_duration, select_flight_easing, CameraFlight, InertiaController, InertiaSample,
    InertiaState, INERTIA_MAX_CLICK_TIME_THRESHOLD,
};
use cesium_plot_bevy::PlotInputCapture;
use glam::{DMat3, DQuat, DVec2, DVec3};

use crate::globe_lod::EARTH_RADIUS_M;
use crate::map2d::{map_is_3d, MapMode};

/// 相机垂直视场角（弧度）。在 spawn 的投影与拖拽数学之间保持同步，
/// 以使抓地跟踪精确。
pub const CAMERA_FOV_Y: f32 = std::f32::consts::FRAC_PI_3; // 60 度
/// 近平面 —— 保持在相机最近 `min_distance` 高度（≈100 m）远下方，
/// 因此当完全缩小以检视最细（亚米每 texel）瓦片时，正下方的地面仍可见。
/// Reversed-Z（wgpu 默认）能容忍由此产出的 ~1:6.7e8 近:远比而不产生
/// 表面 z-fighting（同层瓦片永不重叠；skirt 下降 + tuck 步长吸收了
/// 掠射角下剩余的深度精度）。
const CAMERA_NEAR: f32 = 0.0000005;
/// 远平面 —— 大到足以容纳星空（半径 ~50）。
const CAMERA_FAR: f32 = 200.0;
/// 地球（赤道）半径，以 render unit 计。
const GLOBE_RADIUS: f32 = 1.0;

/// WGS84 椭球半轴，以 render unit 计。瓦片渲染器在 WGS84 椭球上
/// 细分地面 —— 赤道 `ELLIPSOID_A`，极地 `ELLIPSOID_B`，极轴 = 局部 +Z
/// （参见 `tile_mesh::create_tile_mesh_uv`，其顶点 `z` 携带 `(1 - e²)` 因子）。
/// 抓地拖拽拾取必须与此 SAME 表面相交：在深缩放下相机仅位于
/// 地面之上 ~1e-3 render unit，因此单位球体与椭球之间 ~0.2% 的径向
/// 间隙（在中/高纬度最大）否则会膨胀为巨大的屏幕空间增益误差 ——
/// 即实测的 ~0.58× “不跟手”过冲。
const ELLIPSOID_A: f64 = 1.0; // = GLOBE_RADIUS (EARTH_RADIUS / METERS_PER_RENDER_UNIT)
const ELLIPSOID_B: f64 = 6356752.314245 / 6378137.0; // ≈ 0.99664719（极地 / 赤道）

/// 每拖拽帧的绝对地心角上限（弧度）—— 一个与缩放无关的仅兼底值。
/// 纯弧度上限在深缩放下太紧：在 `min_distance` 处 0.1 rad 对应 ≲ 20 px
/// 的屏幕平移，因此一个普通快速拖拽会被它限流，地图每帧都落后
/// 光标 10-20 px（即实测的“不跟手”）。真正的舒适度限制以每帧像素
/// 表达（`MAX_PAN_PX_PER_FRAME`）并转换为一个感知缩放的角；此弧度值
/// 仅保持两个上限有序（在最近的缩放处 px 上限 ≤ rad 上限）并防止
/// 原始运动路径失控。
const MAX_DRAG_STEP_RAD: f32 = 0.5;

/// 每个拖拽步骤的屏幕平移上限（像素 / 帧）—— 包括固定的抓地
/// 修正与原始运动回退，不论在窗口内与否。弧度上限与缩放无关 ——
/// 在最大缩放下 0.1 rad 仅 ~20 px 平移，这会在地表限流快速拖拽并使
/// 地图落后光标 —— 因此每帧的旋转额外被限定为从不平移超过这么多像素
/// （然后通过 `px · surface_dist / focal` 转换为感知缩放的角）。此值舒适地
/// 高于任何真正的单帧鼠标运动（一次极快的拂动 ≈ 150 px/帧），因此
/// 普通拖拽在所有缩放级别都保持精确 1:1；只有病态尖峰（合并的输入
/// 风暴、接近极点拾取导致经线收敛）会在几帧内被收敛而不是传送视图
/// 或将相机抛过地球。
const MAX_PAN_PX_PER_FRAME: f32 = 600.0;

/// 旋转惯性滑行的默认衰减系数（CesiumJS
/// `inertiaSpin` 默认 ≈ 0.9）。
const INERTIA_SPIN_COEFFICIENT: f64 = 0.9;

/// 缩放拖拽的按键 —— CesiumJS `ScreenSpaceCameraController.zoomEventTypes`
/// 的默认首项是 `RIGHT_DRAG`（另两项是滚轮与触屏捏合，后者不在桌面范围）。
/// 旋转保留在左键，因此两个手势互不争抢；左键按住时旋转优先。
const ZOOM_DRAG_BUTTON: MouseButton = MouseButton::Right;

/// 竖直拖拽**整屏高度**时，高于地表的距离改变 `e^ZOOM_DRAG_RANGE ≈ 4.5` 倍。
///
/// 用「整屏高度 → 固定倍率」的指数映射，而不是 CesiumJS `handleZoom` 的
/// 逐帧 `zoomRate = zoomFactor · minDistance`（`zoomFactor = 5.0`）：逐帧增量
/// 在同样速度的拖拽下与帧率成正比（120 Hz 比 30 Hz 快 4 倍），且 5.0 ×
/// `maximumMovementRatio` 0.1 = 每帧 50% 的变化让缩放手感过于急促。指数映射
/// 只取决于**累计位移**，因此手感与帧率无关，往回拖也能精确抵消。
///
/// 位移的单位是**窗口逻辑像素的光标位移**（见 [`zoom_drag_advance`]），不是
/// 鼠标设备计数 —— 后者随鼠标 DPI 变化（同一段手部移动在高 DPI 鼠标上是几倍
/// 计数），会把「一整屏 = 4.5 倍」变成任意倍率，拖一下就飞出全球视野。
///
/// 灵敏度按用户实机手感定：最初一版（满窗 2.7 倍 + 单次上限 4.5 倍 + 回退
/// 96 px/帧）被反馈太慢，遂把三个数值整体 ×1.5 得到当前值。
const ZOOM_DRAG_RANGE: f32 = 1.5;

/// 单次拖拽的位移上限（以整屏高度为单位）：一次连续拖拽最多把高度改变
/// `e^ZOOM_DRAG_MAX_SPAN ≈ 9.5` 倍。光标在窗口内只能走一屏高（≈ 4.5 倍），
/// 因此这个上限只在光标被拖出窗口、或窗口远小于屏幕时生效 —— 它挡住的是
/// 「一次拖拽直接飞到底」，而不是正常拖动：要横穿
/// `[min_distance, max_distance]` 请松开再拖（或直接用滚轮，一格 1.43 倍）。
const ZOOM_DRAG_MAX_SPAN: f32 = 2.25;

/// 光标不可用（拖出窗口 / 无头环境）时，单帧最多消费的位移（逻辑像素）。
///
/// 这条回退路径只能累计 `MouseMotion` 原始计数，因此必须限流：一次拂动或
/// 卡顿后合并送达的输入风暴可能在一帧内给出上千计数，逐帧限流让它们
/// 收敛成连续的几帧，而不是把相机瞬移出去。与 `ZOOM_DRAG_RANGE` 同步 ×1.5，
/// 使回退路径的最大速率也保持「主路径的 1.5 倍旧速」这一手感。
const ZOOM_DRAG_MAX_PX_PER_FRAME: f32 = 144.0;

/// 倾斜拖拽的按键 —— CesiumJS `ScreenSpaceCameraController.tiltEventTypes`
/// 的默认首项是 `MIDDLE_DRAG`（另三项是触屏捏合、`左键+CTRL`、`右键+CTRL`，
/// 后者不在桌面单键范围内）。旋转（左键）与缩放（右键）优先，因此中键
/// 永远不与它们争抢。
const TILT_DRAG_BUTTON: MouseButton = MouseButton::Middle;

/// 倾斜速率上限 —— CesiumJS
/// `ScreenSpaceCameraController.prototype._maximumRotateRate`（默认 1.77）。
const TILT_MAX_ROTATE_RATE: f64 = 1.77;

/// 倾斜速率下限 —— CesiumJS `_minimumRotateRate`（默认 1/5000）。
const TILT_MIN_ROTATE_RATE: f64 = 1.0 / 5000.0;

/// 倾斜单帧位移比例上限 —— CesiumJS `maximumMovementRatio`（默认 0.1）。
const TILT_MAX_MOVEMENT_RATIO: f64 = 0.1;

/// 「相机正对枢轴」判定阈值 —— CesiumJS `CesiumMath.EPSILON2`（0.01）。
/// 低于它时 `rotateVertical` 走平行分支（改用相机右轴），因为此时
/// `枢轴法线 × 位置` 退化。
const TILT_EPSILON2: f64 = 0.01;

/// 回拉安全边距 —— CesiumJS `CesiumMath.EPSILON4`（1e-4）。
const TILT_EPSILON4: f64 = 1.0e-4;

/// 低空保护的高度容差（米）—— CesiumJS `CesiumMath.EPSILON3`（0.001）。
const TILT_EPSILON3: f64 = 1.0e-3;

/// 低空保护额外的米级安全垫 —— CesiumJS `tilt3DOnEllipsoid` 守卫里的 `- 1.0`。
const TILT_LOW_ALTITUDE_MARGIN_M: f64 = 1.0;

/// 倾斜枢轴的最小高度比例 —— CesiumJS
/// `minHeight = controller.minimumZoomDistance * 0.25`。本 app 的
/// `minimumZoomDistance` 是 [`OrbitState::min_distance`] 高于表面的高度
/// （≈100 m），因此低空阈值 ≈25 m：贴地时禁止继续加大倾角（否则相机会被
/// 转到地表以下）。
const TILT_MIN_HEIGHT_RATIO: f64 = 0.25;

/// orbit 控制相机的标记组件。
#[derive(Component)]
pub struct OrbitCamera;

/// 保存 orbit 状态（围绕目标的球坐标）的资源。
#[derive(Resource)]
pub struct OrbitState {
    /// 方位角，以弧度计（绕地球 Z/极地轴的旋转）。
    pub heading: f32,
    /// 赤道（XY）平面上方的仰角，以弧度计。
    /// 正 = 赤道以北，负 = 以南。
    pub pitch: f32,
    /// 距目标的距离，以 render unit 计。
    pub distance: f32,
    /// 滚轮输入立即移动此值；`distance` 每帧向它滑行（指数缓动），
    /// 从而产生 CesiumJS 风格的缩放惯性，而非每格滚轮硬跳 30%。
    pub target_distance: f32,
    /// orbit 目标（世界空间，地球中心）。
    pub target: Vec3,
    /// 总体旋转灵敏度乘子（1.0 = 由相机几何导出的精确 1:1 表面
    /// 跟踪）。
    pub rotate_speed: f32,
    /// 缩放灵敏度：每单位滚轮的高度-相对-表面变化分数（0.3 = 每格
    /// 向表面靠近/远离 30%）。
    pub zoom_speed: f32,
    /// 最小缩放距离（略高于表面，以便你能检视细节）。
    pub min_distance: f32,
    /// 最大缩放距离。
    pub max_distance: f32,
}

impl Default for OrbitState {
    /// 默认视角：俯角 ~23°、距离 3 个渲染单位，旋转/缩放速度取交互手感值。
    fn default() -> Self {
        Self {
            heading: 0.0,
            pitch: 0.4, // ~23 度，赤道以北
            distance: 3.0,
            target_distance: 3.0,
            target: Vec3::ZERO,
            rotate_speed: 1.0, // 默认精确几何跟踪
            zoom_speed: 0.3,
            min_distance: 1.0000157, // 下降到 ~100 m 高度 -> level ~20 tiles
            max_distance: 20.0,
        }
    }
}

// ── M2.4 旋转惯性状态 ────────────────────────────────────

/// 跟踪 orbit 相机旋转惯性的资源。
///
/// 当一次左键拖拽在一次快速拂动（< [`INERTIA_MAX_CLICK_TIME_THRESHOLD`]
/// 秒）后释放时，上一帧的 heading/pitch 速度被捕获进领域
/// [`InertiaController`] 并以指数衰减每帧滑行。
#[derive(Resource)]
pub struct OrbitInertiaState {
    /// 领域惯性控制器（纯 f64 数学，无 Bevy 依赖）。
    pub controller: InertiaController,
    /// 上一帧左键是否按下。
    was_dragging: bool,
    /// 用于计算增量的上一帧方位角。
    prev_heading: f32,
    /// 用于计算增量的上一帧仰角。
    prev_pitch: f32,
    /// 经过的毫秒数（用于惯性计时的单调时钟）。
    now_ms: f64,
    /// 当前拖拽开始的时间戳（ms）。
    press_time_ms: f64,
    /// 当前拖拽释放的时间戳（ms）。
    release_time_ms: f64,
    /// 惯性滑行是否处于激活状态。
    coasting: bool,
    /// 释放时捕获的每弧度像素 heading 尺度。领域
    /// [`InertiaController`] 在像素空间滑行，因此滑行的像素增量用与
    /// 捕获时*相同*的尺度转回弧度，给出原始弧度速度的精确指数
    /// 衰减。
    capture_scale_h: f32,
    /// 释放时捕获的每弧度像素 pitch 尺度（见上）。
    capture_scale_p: f32,
}

impl Default for OrbitInertiaState {
    /// 默认惯性状态：新建控制器，历史角度与时间戳归零。
    fn default() -> Self {
        Self {
            controller: InertiaController::new(),
            was_dragging: false,
            prev_heading: 0.0,
            prev_pitch: 0.4,
            now_ms: 0.0,
            press_time_ms: 0.0,
            release_time_ms: 0.0,
            coasting: false,
            capture_scale_h: 1.0,
            capture_scale_p: 1.0,
        }
    }
}

// ── M2.4 飞行状态 ────────────────────────────────────────

/// 保存 orbit 相机一个活动的大圆航线相机飞行的资源。
///
/// 当飞行处于活动状态时，orbit 状态由领域 [`CameraFlight`] 的 slerp
/// 插值驱动，而非鼠标输入。
#[derive(Resource, Default)]
pub struct OrbitFlightState {
    /// 当前活动飞行（若有）。
    pub flight: Option<CameraFlight>,
}

/// 请求 orbit 相机飞向一个 ECEF 目的点（米）的事件。
///
/// 发送此事件以触发一次大圆航线飞行，其时长与缓动由领域的
/// [`compute_flight_duration`] 和 [`select_flight_easing`] 自动导出。
#[derive(Event)]
pub struct OrbitFlyToRequest {
    /// ECEF 米下的目标位置。
    pub destination_ecef: DVec3,
}

// ── FIX-ARCBALL：实时 trackball 朝向 ────────────────────────

/// 交互相机的实时 arcball（trackball）朝向。
///
/// `engaged` 在用户在窗口化会话中首次拖拽左键时翻为 `true`，
/// 然后保持 `true`，因此位姿 —— 包括 roll 与越极视图 —— 在帧之间
/// 持续存在。每个确定性捕获路径（`FIXED_CAMERA`、`--camera-script`、
/// M2.4 中性测试、v0 基线）从不馈入鼠标，因此 `engaged` 保持 `false`
/// 且 [`orbit_camera_system`] 继续通过纯 [`compute_camera_transform`] 球面
/// 路径驱动相机 → 逐字节不变。
#[derive(Resource)]
struct Arcball {
    /// 相机系架绕目标（地球中心）的旋转。以 f64 保存：在最大
    /// 缩放下每帧的抓地修正约 1e-7 rad，f32 会将其消灭（两个 O(1)
    /// 单位向量之间差异低于 ε 的灾难性抵消 → 一个无头探针显示整个
    /// 拖拽扫过 0.0 rad），使拖拽冻结。f64 保持那个微小残差有意义；
    /// 仅在渲染位姿时将其缩回 f32。
    orientation: DQuat,
    /// 一次真实拖拽是否已接管球面路径。
    engaged: bool,
    /// Geographic 锚点：当前拖拽中光标下抓取点的单位向量
    /// （target → surface）。每帧旋转系架以使该点粘住光标 → 在任何
    /// 抓取位置/缩放下精确跟踪指针。直到在光标下拾取到一个点之前为
    /// `None`。
    anchor: Option<DVec3>,
    /// 上一帧的左键状态，用于检测一次新按下。
    was_pressed: bool,
}

impl Default for Arcball {
    /// 默认单位四元数（IDENTITY）朝向，未吐合、无锚点。
    fn default() -> Self {
        Self {
            orientation: DQuat::IDENTITY,
            engaged: false,
            anchor: None,
            was_pressed: false,
        }
    }
}

// ── 中键倾斜（CesiumJS `MIDDLE_DRAG` → `tilt3D`）─────────────

/// 中键倾斜拖拽的状态（CesiumJS `tilt3DOnEllipsoid` + `rotate3D` 的等价物）。
///
/// 与 [`Arcball`] 一样，只有在真正发生鼠标输入时才会被触碰，因此每个
/// 确定性捕获路径（`FIXED_CAMERA`、`--camera-script`、v0 基线）都保持
/// `engaged == false`、`aim == IDENTITY` → [`compute_camera_transform`]
/// 的球面路径逐字节不变。
#[derive(Resource)]
struct OrbitTilt {
    /// 视图相对「纯注视地心」基座（[`base_look_at_rotation`]）多出来的旋转：
    /// 渲染位姿的朝向 = 基座 · `aim`。倾斜把相机绕枢轴转过去之后，位置仍由
    /// `heading`/`pitch`/`distance` 表达（因此 globe LOD、缩放、惯性都无需
    /// 知道倾斜存在），而「相机已不再注视地心」这部分差异全部装在这里。
    ///
    /// 用 f64 的理由与 [`Arcball::orientation`] 相同：单帧增量可以小到
    /// 1e-7 rad，f32 会把它消灭。
    aim: DQuat,
    /// 是否已经由倾斜接管过位姿。一旦接管就保持（与 [`Arcball::engaged`]
    /// 同款语义），使倾斜后的视角持续存在直到用户再次操作。
    engaged: bool,
    /// 上一帧的 [`TILT_DRAG_BUTTON`] 状态，用于检测一次新按下（新按下 =
    /// 重新拾取枢轴）。
    was_pressed: bool,
    /// 最近一次 **[`tilt_pivot_from_view`] 拾取到的枢轴**（椭球面上的点）——
    /// CesiumJS `tilt3DOnEllipsoid` 每次事件都从**画布正中心**射线取椭球交点，
    /// 本实现每帧从当前视线重拾一次（理由见 [`apply_tilt_drag`] 的文档）。
    /// `DVec3::ZERO` = 尚未拾取到（此时本帧不倾斜）。
    pivot: DVec3,
}

impl Default for OrbitTilt {
    /// 未倾斜：恒等 `aim`、未接管、无有效枢轴。
    fn default() -> Self {
        Self {
            aim: DQuat::IDENTITY,
            engaged: false,
            was_pressed: false,
            pivot: DVec3::ZERO,
        }
    }
}

/// 右键竖直拖拽缩放的状态：一次拖拽期间累计的竖直位移与按下瞬间锁定的
/// 「高于地表的距离」。
///
/// 之所以要锚定，而不是像滚轮那样只看逐帧增量：拖拽的目标高度是
/// `按下高度 · e^(累计位移 / 窗口高度 · ZOOM_DRAG_RANGE)`（见
/// [`zoom_drag_target_surface`]）。这条映射与帧率无关，且往回拖能精确抵消 ——
/// 逐帧增量在 120 Hz 下会比 30 Hz 快 4 倍，正是「太快」的来源。
#[derive(Resource, Default)]
struct OrbitZoomDrag {
    /// 右键是否正被按下（一次拖拽的生命周期）。松开后重新按下会重新锚定。
    engaged: bool,
    /// 按下瞬间锁定的「高于地表的距离」，本次拖拽期间不变。
    surface_at_press: f32,
    /// 按下瞬间的光标纵坐标（窗口逻辑像素）—— 绝对位移路径的锚点。
    cursor_y_at_press: f32,
    /// 本次拖拽的竖直位移（窗口逻辑像素，向下为正），见 [`zoom_drag_advance`]。
    total_dy: f32,
    /// 上一帧的位移是否取自绝对光标路径（`false` = `MouseMotion` 原始计数
    /// 回退）。两条路径单位不同，切换时 [`zoom_drag_advance`] 会重锚。
    cursor_tracking: bool,
}

/// 本帧是否存在进行中的相机手势（左键旋转 / 右键高度拖拽 / 中键倾斜），
/// 或旋转惯性仍在滑行。
///
/// 瓦片管线用它决定「拖动期间」的自适应预算：
/// - ① 抬高逐帧上传/生成预算（`globe_pipeline::GestureBudget`）
/// - ② 放宽实体上限，让被替换下来的旧级别瓦片活到新级别就绪
///   （`globe_pipeline::GESTURE_ENTITY_SLACK`）
///
/// 由 [`orbit_camera_system`] 每帧写入；无手势时 `active = false`，
/// 预算立即回落为 `DefaultBudget`（黄金路径值）。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraGesture {
    /// 本帧是否正在进行相机手势（含惯性滑行）。
    pub active: bool,
}

// ── M0.1 从环境变量播种相机 ─────────────────────────
// 这是 M3.3 FIXED_CAMERA 的最小前置；M3.3 会用一个合适的配置结构体
// 与验证来正式化该接口。目前我们读取单个环境变量，使捕获 harness
// 能定位相机而无需碰 main.rs 或任何渲染逻辑。
//
// 支持的环境变量（全部可选；未设置 = 像素中性默认）：
//   CESIUM_CAM_LON      — 度数经度（相机位置方位角）
//   CESIUM_CAM_LAT      — 度数纬度（相机位置仰角）
//   CESIUM_CAM_HEIGHT   — render unit 下高于表面的高度（默认地球 R=1）
//   CESIUM_CAM_HEADING  — LON 的别名（若两者都设则优先）
//   CESIUM_CAM_PITCH    — LAT 的别名（若两者都设则优先）
//   CESIUM_CAM_DISTANCE — 直接给定的、距中心的 orbit 距离（覆盖 HEIGHT）
//
// 当这些均未设置时，返回的状态为 `OrbitState::default()`，
// 保证与未修改代码库二进制相同的输出。

/// 从环境变量读取相机播种。当无播种变量存在时返回
/// `OrbitState::default()`（像素中线路径）。
pub(crate) fn orbit_state_from_env() -> OrbitState {
    let mut state = OrbitState::default();
    let mut any_set = false;

    // 助手：从环境变量解析 f32
    let read_f32 = |name: &str| -> Option<f32> {
        std::env::var(name).ok().and_then(|v| v.trim().parse::<f32>().ok())
    };

    // heading：CESIUM_CAM_HEADING 优先于 CESIUM_CAM_LON
    if let Some(h) = read_f32("CESIUM_CAM_HEADING").or(read_f32("CESIUM_CAM_LON")) {
        state.heading = h.to_radians();
        any_set = true;
    }

    // pitch：CESIUM_CAM_PITCH 优先于 CESIUM_CAM_LAT
    if let Some(p) = read_f32("CESIUM_CAM_PITCH").or(read_f32("CESIUM_CAM_LAT")) {
        state.pitch = p.to_radians();
        any_set = true;
    }

    // distance：CESIUM_CAM_DISTANCE 覆盖 HEIGHT
    if let Some(d) = read_f32("CESIUM_CAM_DISTANCE") {
        state.distance = d;
        state.target_distance = d;
        any_set = true;
    } else if let Some(h) = read_f32("CESIUM_CAM_HEIGHT") {
        let d = GLOBE_RADIUS + h;
        state.distance = d;
        state.target_distance = d;
        any_set = true;
    }

    if any_set {
        info!(
            "[camera-seed] env override: heading={:.4} pitch={:.4} distance={:.4}",
            state.heading, state.pitch, state.distance
        );
    }
    state
}

/// 设置 orbit 相机的插件。
pub struct OrbitCameraPlugin;

impl Plugin for OrbitCameraPlugin {
    /// 插件装配入口：播种初始相机资源，挂载相机/惯性/飞行三段链式
    /// 更新系统（仅在 3D 地图模式下运行）。
    ///
    /// # 参数
    /// - `app`：Bevy 应用。
    fn build(&self, app: &mut App) {
        // M0.1：从环境变量播种初始相机（未设置时像素中性）
        let initial_state = orbit_state_from_env();
        app.insert_resource(initial_state)
            .init_resource::<MapMode>()
            .init_resource::<OrbitInertiaState>()
            .init_resource::<OrbitFlightState>()
            .init_resource::<Arcball>()
            .init_resource::<OrbitTilt>()
            .init_resource::<OrbitZoomDrag>()
            .init_resource::<CameraGesture>()
            .add_event::<OrbitFlyToRequest>()
            .add_systems(Startup, spawn_orbit_camera)
            .add_systems(
                Update,
                (orbit_camera_system, orbit_inertia_system, orbit_flight_system)
                    .chain()
                    // 根据当前地图模式门控整个 3D 路径。`MapMode`
                    // 默认为 ThreeD，因此在默认（及每个确定性）会话中这
                    // 些系统与以往完全一样运行；它们只在用户切到 2D 地图
                    // 后退让。
                    .run_if(map_is_3d),
            );
    }
}

/// 按当前轨道状态计算变换并 spawn 一个透视投影的 3D 相机实体。
fn spawn_orbit_camera(mut commands: Commands, state: Res<OrbitState>) {
    let transform = compute_camera_transform(&state);
    // 自定义透视投影：一个小近平面使相机能非常接近表面以检视影像
    // 细节，而远平面仍能触及星空。
    let projection = PerspectiveProjection {
        fov: CAMERA_FOV_Y,
        near: CAMERA_NEAR,
        far: CAMERA_FAR,
        ..default()
    };

    // M4.2 / M5-E1 说明（P0 观感清理）：原先这里按
    // `postprocess_builtin_enabled()` 二选一——ON 时 spawn 一个
    // `Camera{hdr:true}` + `Tonemapping::AcesFitted` + `Bloom::NATURAL` 的
    // HDR 相机，并在 `postprocess_enabled()` 下插入 FXAA 触发组件
    // `CesiumFxaa`（渲染图节点已随 `adapters/bevy-render/src/effects` 删除）。
    // 后处理/色调映射属于「复杂观感」，不在目标内，因此相机现在恒定使用
    // `Tonemapping::None`，精确保留 v0 基线（CesiumJS 原样显示影像；
    // TonyMcMapFace 需要本工作区被禁用的 `tonemapping_luts` feature）。
    // 图层 0 = 3D 地球场景，图层 2 = 共享 UI，图层 3 = 标绘
    // overlay。2D 地图相机拥有图层 1，因此两者从不渲染对方的
    // 世界；两只相机都拾取图层 3，因此标绘在两种模式下都显示。
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        OrbitCamera,
        RenderLayers::from_layers(&[0, 2, 3]),
        Projection::Perspective(projection),
        transform,
    ));
}

/// 右键竖直拖拽的目标「高于地表的距离」（render unit）：**向上拖 = 放大**
/// （接近地表），**向下拖 = 缩小**（拉远）—— 与滚轮方向一致（上滚 = 放大）。
///
/// 与 CesiumJS 的一处**有意取反**：CesiumJS `handleZoom` 的
/// `approachingSurface = diff > 0` 意味着右键**向下**拖是接近地面。这里按本项目
/// 的手感取反，使右键拖拽与滚轮、以及用户的直觉（往上 = 拉近）一致。
///
/// `total_dy` 是**本次拖拽的累计竖直位移**（按下瞬间归零），因此目标高度是
/// `surface_at_press · e^(总位移 / 窗口高度 · ZOOM_DRAG_RANGE)`：与帧率无关、
/// 可逆、且高低空手感一致（线性映射在贴近地表时会一步撞地）。累计位移按
/// `ZOOM_DRAG_MAX_SPAN` 个窗口高度钳制。
///
/// 与 CesiumJS 的两处有意省略（保留原样）：
/// - 不做 `percentage = clamp(|unitPositionDotDirection|, 0.25, 1)` 修正：本 app
///   的轨道目标恒为地心，`direction = -normalize(position)`，该点乘恒为 -1，
///   于是 `percentage` 恒为 1、公式退化为 1。它只在斜视（look-at 地表）下才有意义。
/// - 不做地形拾取（`pickPosition` / `_minimumPickingTerrainDistance`）：同样因为
///   相机始终注视地心，不存在「拾取点比目标更远」的斜视场景。
fn zoom_drag_target_surface(surface_at_press: f32, total_dy: f32, win_h: f32) -> f32 {
    let span = (total_dy / win_h.max(1.0) * ZOOM_DRAG_RANGE)
        .clamp(-ZOOM_DRAG_MAX_SPAN, ZOOM_DRAG_MAX_SPAN);
    surface_at_press * span.exp()
}

/// 本帧的拖拽位移：返回 `(total_dy, cursor_y_at_press, cursor_tracking)`。
///
/// 优先用**绝对光标位移** `cursor_y - cursor_y_at_press`：它与鼠标 DPI、指针
/// 速度设置无关（同一段手部移动在任何鼠标上都产生同样的手感），且天然被窗口
/// 与屏幕边界钳制 —— 光标最多走一屏，因此一次拖拽不可能「瞬间」把地球拉到
/// 天边。左键旋转走的是同一条绝对光标路径（`cursor_ray_*`），两者手感一致。
///
/// 光标不可用（拖出窗口被 winit 置为 `None`，或 `Window` 不存在的最小 App /
/// 无头测试）时回退到 `MouseMotion` 原始计数的累计，但**逐帧限流**到
/// [`ZOOM_DRAG_MAX_PX_PER_FRAME`]：这条路径的单位是设备计数（随 DPI 变化），
/// 且卡顿后一次送达的合并输入可能是上千计数，不限流就会把高度瞬移出去。
///
/// 两条路径**单位不同**，因此切换的那一帧必须重锚（`anchor = cursor_y -
/// total_dy`），否则拖出窗口再拖回来会让高度跳变：累计位移在重锚前后保持连续。
/// `cursor_tracking` 记住上一帧用的是哪条路径。
fn zoom_drag_advance(
    cursor_y: Option<f32>,
    cursor_y_at_press: f32,
    total_dy: f32,
    cursor_tracking: bool,
    raw_dy: f32,
) -> (f32, f32, bool) {
    match cursor_y {
        Some(y) => {
            let anchor = if cursor_tracking {
                cursor_y_at_press
            } else {
                y - total_dy
            };
            (y - anchor, anchor, true)
        }
        None => {
            let cap = ZOOM_DRAG_MAX_PX_PER_FRAME;
            (total_dy + raw_dy.clamp(-cap, cap), cursor_y_at_press, false)
        }
    }
}

// ── 中键倾斜：纯几何助手 ────────────────────────────────

/// 椭球（[`ELLIPSOID_A`] / [`ELLIPSOID_B`]，render unit）上、从 `center` 沿
/// 单位方向 `dir` 出发的表面点。`dir` 会被归一化；零向量返回 `None`。
fn ellipsoid_point_along(dir: DVec3, center: DVec3) -> Option<DVec3> {
    let u = dir.try_normalize()?;
    let inv = ((u.x * u.x + u.y * u.y) / (ELLIPSOID_A * ELLIPSOID_A)
        + (u.z * u.z) / (ELLIPSOID_B * ELLIPSOID_B))
        .sqrt();
    Some(center + u / inv)
}

/// `point` 处的椭球大地法线（单位向量）—— 即该点 ENU 系的 +Z。
fn ellipsoid_normal_at(point: DVec3, center: DVec3) -> Option<DVec3> {
    let p = point - center;
    DVec3::new(
        p.x / (ELLIPSOID_A * ELLIPSOID_A),
        p.y / (ELLIPSOID_A * ELLIPSOID_A),
        p.z / (ELLIPSOID_B * ELLIPSOID_B),
    )
    .try_normalize()
}

/// CesiumJS `rotate3D` 的 `rotateRate`。
///
/// `tilt3DOnEllipsoid` 在施加旋转前把 `_rotateFactor` / `_rotateRateRangeAdjustment`
/// 都临时设为 `1.0`，所以 `rate = rho - 1.0`，其中 `rho` 是相机在枢轴 ENU 系
/// 中的位置模长（米）= 相机到枢轴的斜距。任何高于约 2.8 m 的相机都会让
/// `rho - 1.0` 超过 `_maximumRotateRate = 1.77` 而饱和 —— 即倾斜速率实际上
/// 与高度无关（这与旋转/缩放不同，后两者都随高度变化）。
fn tilt_rotate_rate(pivot_distance_m: f64) -> f64 {
    (pivot_distance_m - 1.0).clamp(TILT_MIN_ROTATE_RATE, TILT_MAX_ROTATE_RATE)
}

/// 本帧的 `(delta_phi, delta_theta)` —— CesiumJS `rotate3D` 的
/// `phiWindowRatio = (start.x - end.x) / clientWidth`、
/// `thetaWindowRatio = (start.y - end.y) / clientHeight`（都按
/// `maximumMovementRatio` 钳制），乘速率后分别乘 `2π` / `π`。
///
/// 与 CesiumJS 的一处有意收紧：它只对比例做 `Math.min(ratio, 0.1)` 单侧钳制，
/// 负方向（大幅反向拖拽 / 合并的输入风暴）没有上限。这里两侧都钳到
/// `maximumMovementRatio`，与缩放路径（[`zoom_drag_delta_surface`]）一致，
/// 以免一帧把视角甩过地球；单帧真实位移远小于该上限，因此手感无差别。
fn tilt_deltas(pivot_distance_m: f64, dx: f64, dy: f64, win_w: f64, win_h: f64) -> (f64, f64) {
    let rate = tilt_rotate_rate(pivot_distance_m);
    let phi = (-dx / win_w.max(1.0)).clamp(-TILT_MAX_MOVEMENT_RATIO, TILT_MAX_MOVEMENT_RATIO);
    let theta = (-dy / win_h.max(1.0)).clamp(-TILT_MAX_MOVEMENT_RATIO, TILT_MAX_MOVEMENT_RATIO);
    (
        rate * phi * std::f64::consts::TAU,
        rate * theta * std::f64::consts::PI,
    )
}

/// CesiumJS `camera.rotate(axis, turn_angle)`：绕**参考系原点**（倾斜时是枢轴）
/// 施加 `Quaternion.fromAxisAngle(axis, -turnAngle)` —— 相机的位置与朝向
/// 一起转。这里直接用 `turn_angle` 表达「向量实际转过的角」（内部取负已
/// 折算好），避免 CesiumJS 那套 `rotateRight → rotateHorizontal → rotate`
/// 的双重取负在这里被重复一次。
fn rotate_pose_about(
    pivot: DVec3,
    position: DVec3,
    rotation: DQuat,
    axis: DVec3,
    turn_angle: f64,
) -> (DVec3, DQuat) {
    let Some(axis) = axis.try_normalize() else {
        return (position, rotation);
    };
    let q = DQuat::from_axis_angle(axis, turn_angle);
    (pivot + q * (position - pivot), (q * rotation).normalize())
}

/// CesiumJS `rotateUp(delta_theta)` → `rotateVertical(camera, -delta_theta)` 的
/// 轴 / 角度选择，返回 `(轴, 向量实际转过的角)`；`None` = 该分支不施加任何
/// 旋转。
///
/// CesiumJS 原文（`Camera.js` `rotateVertical`）：把相机位置单位化为 `p`，
/// 与 `constrainedAxis` 比较得 `northParallel` / `southParallel`；两者皆假时
/// 绕 `cross(constrainedAxis, p)` 旋转，并把角度夹在「不许越过枢轴正上方 /
/// 正下方」的边界内（`EPSILON4` 边距）；恰好与枢轴轴平行时改用相机右轴，
/// 且只有把相机带离该奇点的那一侧角度生效。
fn tilt_vertical_turn(
    up: DVec3,
    unit_p: DVec3,
    right: DVec3,
    delta_theta: f64,
) -> Option<(DVec3, f64)> {
    // `rotateUp(a)` 把 `-a` 交给 `rotateVertical`；`camera.rotate(axis, angle)`
    // 再内部取负，因此向量实际转过的角是 `-a`。
    let a = -delta_theta;
    let north_parallel = unit_p.abs_diff_eq(up, TILT_EPSILON2);
    let south_parallel = unit_p.abs_diff_eq(-up, TILT_EPSILON2);
    if !north_parallel && !south_parallel {
        let mut angle = a;
        let to_axis = unit_p.dot(up).clamp(-1.0, 1.0).acos();
        if angle > 0.0 && angle > to_axis {
            angle = to_axis - TILT_EPSILON4;
        }
        let to_neg_axis = unit_p.dot(-up).clamp(-1.0, 1.0).acos();
        if angle < 0.0 && -angle > to_neg_axis {
            angle = -to_neg_axis + TILT_EPSILON4;
        }
        Some((up.cross(unit_p), -angle))
    } else if (north_parallel && a < 0.0) || (south_parallel && a > 0.0) {
        Some((right, -a))
    } else {
        None
    }
}

/// 施加一次倾斜步：先绕枢轴大地法线做水平旋转（`rotateRight(delta_phi)`），
/// 再绕 `up × p` 做竖直旋转（`rotateUp(delta_theta)`，轴 / 角度选择见
/// [`tilt_vertical_turn`]）。`scale` 同时缩放两个角度，供「落在地板上」的
/// 二分收缩使用。
///
/// 返回 `(位置, 朝向, 是否施加了竖直旋转)`：第三项为 `false` 表示竖直分支按
/// CesiumJS 的规则不旋转（此时只有 `delta_phi != 0.0` 才算真的动了）。
fn tilt_step(
    pivot: DVec3,
    position: DVec3,
    rotation: DQuat,
    up: DVec3,
    delta_phi: f64,
    delta_theta: f64,
    scale: f64,
) -> (DVec3, DQuat, bool) {
    // 水平：`camera.rotateRight(delta_phi)` → `rotateHorizontal(-delta_phi)`
    // → `camera.rotate(constrainedAxis = up, -delta_phi)`，两次取负相消 ⇒ 绕
    // 枢轴大地法线施加 +delta_phi。
    let (position, rotation) = rotate_pose_about(pivot, position, rotation, up, delta_phi * scale);
    // 竖直：`camera.right` 取水平旋转**之后**的系架右轴，与 CesiumJS 中
    // `rotateRight` 先于 `rotateUp` 执行一致。
    let unit_p = (position - pivot).try_normalize().unwrap_or(up);
    match tilt_vertical_turn(up, unit_p, rotation * DVec3::X, delta_theta * scale) {
        Some((axis, turn)) => {
            let (position, rotation) = rotate_pose_about(pivot, position, rotation, axis, turn);
            (position, rotation, true)
        }
        None => (position, rotation, false),
    }
}

/// 中键倾斜的枢轴：**当前视线**在地表上的落点（CesiumJS `tilt3DOnEllipsoid`
/// 每帧从画布正中心拾取的那个点）。
///
/// 视线命中椭球 ⇒ 取近端交点；视线打空（看向天空 / 地平线之上）⇒ 退化为
/// 「射线到地心的最近点所对应的地表点」（CesiumJS 那里用
/// `IntersectionTests.grazingAltitudeLocation` 取地平切点，作用相同）。两者都
/// 失败（退化几何）时返回 `None`，调用方保留上一帧的枢轴。
fn tilt_pivot_from_view(position: DVec3, rotation: DQuat, target: DVec3) -> Option<DVec3> {
    // 本 app 的相机沿局部 −Z 看（[`base_look_at_rotation`] 把 −Z 指向 target）。
    let forward = rotation * -DVec3::Z;
    if let Some(direction) = pick_surface_dir_ellipsoid(position, forward, target) {
        if let Some(point) = ellipsoid_point_along(direction, target) {
            return Some(point);
        }
    }
    // 打空：把射线到地心的最近点投到椭球面上。
    let ray = position - target;
    let closest = ray + forward * (-ray.dot(forward));
    ellipsoid_point_along(closest, target)
}

/// 把本帧的中键拖拽增量施加到相机位姿上（CesiumJS `tilt3DOnEllipsoid`）。
///
/// 顺时针序（与 CesiumJS 相同）：重拾枢轴 → 低空守卫 → 绕枢轴大地法线做水平
/// 旋转（`rotateRight(delta_phi)`）→ 绕 `up × p` 做竖直旋转
/// （`rotateUp(delta_theta)`）→ 把结果写回 `heading`/`pitch`/`distance` 与
/// [`OrbitTilt::aim`]。
///
/// 枢轴**每帧**重拾（见 [`tilt_pivot_from_view`]），与 CesiumJS 从画布正中心
/// 拾取一致。这不是可以省掉的一步：若把枢轴固定为按下时那一点，相机走的是一
/// 条穿过地心的圆，高度越转越低，单帧大角度时能一步扎到地表以下（实测中键拖
/// 几下就会看到「斜插进地球内部」的画面）。每帧重拾后相机始终看着枢轴，每帧
/// 的下沉量正比于当前高度，低空守卫才真正拦得住。
///
/// 相机到枢轴的距离在单帧内不变（纯旋转），所以「下沉/抬起」表现为相机绕枢轴
/// 转过去后对地心的远近变化。
fn apply_tilt_drag(
    state: &mut OrbitState,
    arcball: &mut Arcball,
    tilt: &mut OrbitTilt,
    middle_pressed: bool,
    deltas: (f64, f64),
    win: (f64, f64),
    flight_active: bool,
) {
    // CesiumJS 在飞行 / `look` 期间不接管倾斜；我们同样让位（否则飞行系统
    // 会覆盖本帧结果，而枢轴会停留在过期位姿上）。
    if flight_active {
        tilt.was_pressed = middle_pressed;
        return;
    }

    let target = state.target.as_dvec3();
    let position = orbit_position_f64(state, target);
    let (dx, dy) = deltas;

    // 当前朝向：arcball 系架 · aim（arcball 未接管时即纯注视地心的基座）。
    let rotation = if arcball.engaged {
        arcball.orientation * tilt.aim
    } else {
        base_look_at_rotation(position, target)
    };

    tilt.was_pressed = middle_pressed;
    if !middle_pressed {
        return;
    }

    // 枢轴：每帧从当前视线方向重拾（命中椭球 ⇒ 视线落点；打空 ⇒ 射线到地心
    // 最近点对应的地表点）。按下不动时也刷新一次：此时视线穿过地心，得到的
    // 就是相机脚下的天底点。打空且退化时保留上一帧的枢轴。
    if let Some(pivot) = tilt_pivot_from_view(position, rotation, target) {
        tilt.pivot = pivot;
    }
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    if tilt.pivot == DVec3::ZERO {
        return;
    }
    let pivot = tilt.pivot;
    let up = ellipsoid_normal_at(pivot, target).unwrap_or(DVec3::ZERO);
    if up == DVec3::ZERO {
        return;
    }

    // 低空守卫（CesiumJS `tilt3DOnEllipsoid` 首段）：贴着地面时禁止「向上拖」
    // （`dy < 0` = 光标上移 = 继续加大倾角），否则相机会被转到地表以下。
    // 高度与阈值都用「半径 1 球面」约定（render unit → 米）：本 app 的
    // `min_distance` / `max_distance` 就是这套单位（`min_distance` = 1.0000157
    // ⇒ 球面之上 100 m）。若改用椭球大地高，极区会多出约 21 km 的差异，守卫
    // 在本 app 的尺度下将几乎永不触发。
    let height = ((position - target).length() - f64::from(GLOBE_RADIUS)) * EARTH_RADIUS_M;
    let min_height =
        f64::from(state.min_distance - GLOBE_RADIUS) * EARTH_RADIUS_M * TILT_MIN_HEIGHT_RATIO;
    if height - min_height - TILT_LOW_ALTITUDE_MARGIN_M < TILT_EPSILON3 && dy < 0.0 {
        return;
    }

    // 当前位姿的 f64 表达（位置与朝向都在上面算好）。
    let pivot_distance_m = (position - pivot).length() * EARTH_RADIUS_M;
    let (delta_phi, delta_theta) = tilt_deltas(pivot_distance_m, dx, dy, win.0, win.1);

    // 地板：与低空守卫同一个阈值（本 app 的「半径 1 球面」约定，render unit）。
    let floor = f64::from(GLOBE_RADIUS)
        + f64::from(state.min_distance - GLOBE_RADIUS) * TILT_MIN_HEIGHT_RATIO;

    let (mut new_position, mut new_rotation, vertical_applied) =
        tilt_step(pivot, position, rotation, up, delta_phi, delta_theta, 1.0);
    // 本帧净旋转为零（例如相机正对枢轴时向下拖：CesiumJS 的竖直分支不旋转）
    // 时不得「接管」朝向 —— 否则会把恒等旋转写进 [`OrbitTilt::aim`]，并在
    // 单测里表现为「engaged 但姿态未变」。
    if !vertical_applied && delta_phi == 0.0 {
        return;
    }
    if (new_position - target).length() < floor {
        // 完整增量会越过地板 ⇒ 二分收缩本帧角度，让相机**落在地板上**，而不是
        // 整帧丢弃。丢弃会「永久卡死」：相机贴近地面但距枢轴很远时（视线近乎
        // 水平），一个鼠标步就能把它推下地表，于是每帧都被丢弃、中键再也推不动
        // （实测：默认视点持续向上拖 ~200 帧后会僵在 9.5 km 高度）。
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            let (probe, _, _) =
                tilt_step(pivot, position, rotation, up, delta_phi, delta_theta, mid);
            if (probe - target).length() < floor {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        if lo <= 0.0 {
            return;
        }
        let (clamped_position, clamped_rotation, _) =
            tilt_step(pivot, position, rotation, up, delta_phi, delta_theta, lo);
        if (clamped_position - target).length() < floor {
            return;
        }
        new_position = clamped_position;
        new_rotation = clamped_rotation;
    }
    let (position, rotation) = (new_position, new_rotation);

    // 写回球面状态：倾斜后的位置仍可表达为 (heading, pitch, distance)，因此
    // 缩放、globe LOD、惯性捕获全部继续有效；「相机不再注视地心」的部分
    // 全部落在 [`OrbitTilt::aim`]。
    let offset = position - target;
    let distance = offset.length();
    if !distance.is_finite() || distance <= f64::EPSILON {
        return;
    }
    state.heading = offset.y.atan2(offset.x) as f32;
    state.pitch = (offset.z / distance).clamp(-1.0, 1.0).asin() as f32;
    state.distance = distance as f32;
    // 缩放惯性的目标是「地表之上高度」，倾斜改变了它，同步目标以免弛豫把
    // 相机拉回倾斜前的高度。
    state.target_distance = state.distance;

    let base = base_look_at_rotation(position, target);
    arcball.engaged = true;
    arcball.orientation = base;
    arcball.anchor = None;
    tilt.aim = (base.inverse() * rotation).normalize();
    tilt.engaged = true;
}

/// 系统：读取鼠标输入并更新相机 transform。
///
/// 这是**原始 M0 系统** —— 抓地公式与缩放惯性滑行逐字节保留。
/// 旋转惯性滑行由 [`orbit_inertia_system`] 处理，它运行在此之后。
#[allow(clippy::too_many_arguments)] // Bevy 系统：每个资源/事件/查询一个参数
fn orbit_camera_system(
    mut state: ResMut<OrbitState>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut motion_events: EventReader<MouseMotion>,
    mut wheel_events: EventReader<MouseWheel>,
    time: Res<Time>,
    mut query: Query<&mut Transform, With<OrbitCamera>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<OrbitCamera>>,
    mut arcball: ResMut<Arcball>,
    mut tilt: ResMut<OrbitTilt>,
    mut zoom_drag: ResMut<OrbitZoomDrag>,
    inertia: Res<OrbitInertiaState>,
    mut gesture: ResMut<CameraGesture>,
    flight: Res<OrbitFlightState>,
    capture: Option<Res<PlotInputCapture>>,
) {
    // 标绘 overlay 本帧拥有指针：排空已排队的运动/滚轮事件，以便它们
    // 不会在控制权交回时累积并猛拉相机，然后退让。用 `Option<Res>` 使本
    // 系统在桥接插件未注册（无头 / 标绘禁用）以及在最小单元测试 App 中
    // 仍能运行。
    if capture.is_some_and(|c| c.is_captured()) {
        motion_events.clear();
        wheel_events.clear();
        // 指针归标绘所有：不是相机手势，预算应回落。
        gesture.active = false;
        return;
    }
    // 旋转：左键拖拽 —— 光标固定的“抓地”。按下时我们拾取指针下的
    // Geographic 点（射线 → 球体），然后每帧绕目标旋转系架，使那个
    // 相同的点随着光标移动而粘住它。这能在任意抓取位置与缩放下精确
    // 跟踪指针 —— 之前的每像素切线增益只与单一屏幕中心点匹配，因此
    // 在其他位置抓取时两个轴上都显得脱离。当 OS 光标位置不可用时会运
    // 行一个几何回退，因此手感会优雅降级而非卡死。

    // 高度拖拽：右键竖直拖动。左键按住时旋转优先，两者不同时生效。本帧的竖直
    // 位移必须在这里消费 —— 下面所有分支都会 `motion_events.clear()` 排空队列。
    let pressed = mouse_buttons.pressed(MouseButton::Left);
    let zoom_dragging = !pressed && mouse_buttons.pressed(ZOOM_DRAG_BUTTON);
    let win = windows.get_single().ok();
    let win_h = win.map(|w| w.height()).unwrap_or(720.0);
    let cursor_y = win.and_then(|w| w.cursor_position()).map(|p| p.y);
    // 按下瞬间锚定「高于地表的距离」与光标纵坐标；松开 = 本次拖拽结束，
    // 下次按下重新锚定。位移只在拖拽期间增长，因此往回拖能把高度精确
    // 带回按下时的值（`zoom_drag_advance`：优先取绝对光标位移，无光标时
    // 回退到 `MouseMotion` 原始计数并逐帧限流）。
    if zoom_dragging && !zoom_drag.engaged {
        let min_surf = state.min_distance - GLOBE_RADIUS;
        let max_surf = state.max_distance - GLOBE_RADIUS;
        zoom_drag.surface_at_press =
            (state.target_distance - GLOBE_RADIUS).clamp(min_surf, max_surf);
        zoom_drag.cursor_y_at_press = cursor_y.unwrap_or(win_h * 0.5);
        zoom_drag.total_dy = 0.0;
        zoom_drag.cursor_tracking = cursor_y.is_some();
        zoom_drag.engaged = true;
    } else if !zoom_dragging {
        zoom_drag.engaged = false;
    }
    // `MouseMotion` 队列总是要排空（下面每个分支都会 `clear()`），因此这里
    // 无条件读一次；只有光标不可用的回退路径才消费它的值。
    let raw_dy = if zoom_dragging {
        motion_events.read().map(|ev| ev.delta.y).sum::<f32>()
    } else {
        0.0
    };
    if zoom_dragging {
        let (total_dy, anchor, tracking) = zoom_drag_advance(
            cursor_y,
            zoom_drag.cursor_y_at_press,
            zoom_drag.total_dy,
            zoom_drag.cursor_tracking,
            raw_dy,
        );
        zoom_drag.total_dy = total_dy;
        zoom_drag.cursor_y_at_press = anchor;
        zoom_drag.cursor_tracking = tracking;
    }

    // 倾斜：中键拖拽（CesiumJS `tiltEventTypes` 首项 `MIDDLE_DRAG` → `tilt3D`）。
    // 左键旋转与右键缩放都优先于它，因此三种手势互斥。本帧位移同样必须在
    // 下面 `motion_events.clear()` 之前取走。
    let tilt_pressed = !pressed && !zoom_dragging && mouse_buttons.pressed(TILT_DRAG_BUTTON);
    // 相机手势状态（供瓦片管线自适应预算）：左键旋转 / 右键高度 / 中键倾斜
    // 任一按住，或旋转惯性仍在滑行，都视为「手势进行中」。惯性读数来自上一帧
    // （`orbit_inertia_system` 在本系统之后运行），一帧延迟对预算没有影响。
    gesture.active = pressed || zoom_dragging || tilt_pressed || inertia.coasting;
    let (tilt_dx, tilt_dy) = if tilt_pressed {
        motion_events.read().fold((0.0f32, 0.0f32), |(x, y), ev| {
            (x + ev.delta.x, y + ev.delta.y)
        })
    } else {
        (0.0, 0.0)
    };
    if pressed {
        // 新按下（新抓取）：重置锚点以便我们在光标下重新拾取，若
        // 这是历次首次拖拽则闭锁球面位姿。
        if !arcball.was_pressed {
            if !arcball.engaged {
                arcball.orientation = arcball_quat_from_spherical(&state).as_dquat();
                arcball.engaged = true;
            }
            arcball.anchor = None;
        }

        let focal = (win_h * 0.5) / (CAMERA_FOV_Y * 0.5).tan();
        let surface_dist = (state.distance - GLOBE_RADIUS).max(0.001);

        // 穿过光标的世界空间拾取射线，然后它命中的表面方向。优先使用
        // 相机的*真实*投影（`viewport_to_world`，即 Bevy 实际的
        // `clip_from_view` + 渲染的 `GlobalTransform`），因此“光标下的点”按构造
        // 就是用户看到在光标下的点 —— 这消除了每个手工建模假设（视锥
        // 数学、transform 约定、上轴）。手工写的 `cursor_ray_world` 仅作为相机
        // 尚不可查询那些帧的回退存活。两者都在下游拓宽到 f64。
        let bevy_ray = win.and_then(|w| {
            cameras
                .get_single()
                .ok()
                .and_then(|(cam, ct)| cursor_ray_bevy(cam, ct, w))
        });
        let picked = bevy_ray
            .or_else(|| {
                win.and_then(|w| {
                    cursor_ray_world(w, arcball.orientation, state.distance, state.target)
                })
            })
            .and_then(|(o, d)| pick_surface_dir_ellipsoid(o, d, state.target.as_dvec3()));

        match (arcball.anchor, picked) {
            (Some(anchor), Some(bdir)) => {
                // 旋转系架使被抓的点重新回到光标下（精确 1:1）。增量无关
                // 紧要（绝对光标）。注意：故意不用 `Quat::from_rotation_arc` —— 当
                // `dot > 1 − ε` 它会提前返回 `IDENTITY`，而在最大缩放下锚点与光标
                // 表面方向收敛到 ~3e-5 rad 以内，因此它们的 f32 dot 舍入为 1.0 →
                // 拖拽冻结在 0 旋转。`rotation_from_unit_dir` 能恢复那个微小角度。
                //
                // 然后该修正每帧被钳位 —— 但上限是感知缩放的：
                // `MAX_PAN_PX_PER_FRAME` 像素的屏幕平移在当前高度转换为地心角
                // （`px · surface_dist / focal`），从不超过全局弧度兼底。一个固定
                // 弧度上限在整球缩放下看着安全，却将深缩放限流到地面：在
                // `min_distance` 处 0.1 rad ≈ 20 px/帧，因此每次快速拖拽都留下
                // 10-20 px 延迟（即“不跟手”）。普通拖拽在任何缩放从不触发像素
                // 上限（中球弦长 ≲ 0.03 rad，最大缩放步长 ≲ 1e-4 → 仍精确 1:1），
                // 而一个大的单帧光标增量（一次拂动 / 合并输入风暴）或一次
                // 接近极点的拾取 —— 经线收敛，一个水平鼠标移动映射为巨大的地心
                // 摆动 —— 会在几帧内被收敛，而不是传送视图或将相机抛过地球
                // （“地球没了”）。
                let step_cap =
                    (f64::from(MAX_PAN_PX_PER_FRAME) * f64::from(surface_dist / focal))
                        .min(f64::from(MAX_DRAG_STEP_RAD));
                // 通过绕其自身轴缩放抓地旋转来应用总体灵敏度乘子
                // `rotate_speed`（1.0 = 精确 1:1），然后每帧钳位。之前离球回退会
                // 消费此值；现在固定路径拥有它，因此该字段保持有效。
                let (grab_axis, grab_ang) = rotation_from_unit_dir(bdir, anchor).to_axis_angle();
                let r = clamp_rotation_angle(
                    DQuat::from_axis_angle(grab_axis, f64::from(state.rotate_speed) * grab_ang),
                    step_cap as f32,
                );
                arcball.orientation = (r * arcball.orientation).normalize();
                motion_events.clear();
            }
            (maybe_anchor, Some(bdir)) => {
                // 首个拾取帧（或光标重新进入地球）：将锚点闭锁到光标下的点，
                // 本帧不旋转。在此也应用几何增益会与下一帧的拾取修正相冲突
                // （它会把刚闭锁的点旋转回光标下），产生拖拽起始的传送。跟踪
                // 从下一帧开始干净地展开。
                let _ = maybe_anchor;
                arcball.anchor = Some(bdir);
                motion_events.clear();
            }
            (_, None) => {
                // 光标不再解析为地球上的一个点 —— 它被拖出了可见圆盘进入
                // 天空。根据明确的产品决策：完全不旋转，只冻结拖拽。重置锚点
                // 并排空鼠标运动积压，以便光标在球外时视图保持不动，且当光标
                // 返回时在下一个拾取帧重新干净地闭锁（无陈旧增量抛甩，无快速
                // 越边旋转）。
                arcball.anchor = None;
                motion_events.clear();
            }
        }

        // 重新导出 heading/pitch（故意丢弃 roll），使球面消费者 —— globe LOD
        // 子相机点、惯性捕获 —— 保持有效。
        let (h, p) = orbit_from_orientation(arcball.orientation);
        state.heading = h;
        state.pitch = p;
    } else {
        // 即使未拖拽也消费事件以避免累积
        motion_events.clear();
    }
    arcball.was_pressed = pressed;

    // 缩放：鼠标滚轮 —— 对高于表面的高度作乘性缩放，而非距中心的
    // 距离。接近地面时，距中心距离 ≈ R，因此它的固定比例是相对于表面
    // 上方小高度的巨大比例（一格就会撞进地面），而往回拉又显得迟钝。改为
    // 缩放高于表面的高度，可在任意高度给出一致的感知缩放：掠地时温和，
    // 从远处接近时快速。
    for ev in wheel_events.read() {
        let min_surf = state.min_distance - GLOBE_RADIUS;
        let max_surf = state.max_distance - GLOBE_RADIUS;
        let surface_dist = (state.target_distance - GLOBE_RADIUS).clamp(min_surf, max_surf);
        // ev.y > 0（向上滚）= 缩小 -> 缩小高于表面的高度。
        let zoom_factor = 1.0 - ev.y * state.zoom_speed;
        let new_surf = (surface_dist * zoom_factor).clamp(min_surf, max_surf);
        state.target_distance = GLOBE_RADIUS + new_surf;
    }

    // 缩放：右键竖直拖动 —— 与滚轮路径共用「高于表面的高度」这一把标尺，
    // 因此两个手势可以互换使用，且都被 `[min_distance, max_distance]` 钳制。
    // **向上拖 = 放大**（接近地表）、**向下拖 = 缩小**（拉远），与滚轮方向一致。
    // 目标高度 = `按下高度 · e^(累计位移 / 窗口高度 · ZOOM_DRAG_RANGE)`，见
    // [`zoom_drag_target_surface`]：与帧率无关、可逆、一次拖拽不超过
    // `e^ZOOM_DRAG_MAX_SPAN ≈ 9.5` 倍，且位移取窗口逻辑像素的光标位移
    // （见 [`zoom_drag_advance`]）—— 与鼠标 DPI 无关，因此不会「一拖就飞出天边」。
    if zoom_drag.engaged {
        let min_surf = state.min_distance - GLOBE_RADIUS;
        let max_surf = state.max_distance - GLOBE_RADIUS;
        let new_surf =
            zoom_drag_target_surface(zoom_drag.surface_at_press, zoom_drag.total_dy, win_h)
                .clamp(min_surf, max_surf);
        state.target_distance = GLOBE_RADIUS + new_surf;
        // 拖拽路径**直接**跟随，不经过下面的弛豫：输入的绝对光标位移本身是
        // 平滑的，再叠一层约 `1/10 s` 的缓动只会读作「拖动与光标不同步」。
        // 弛豫保留给滚轮 —— 那里是离散的一格一跳，硬跳会读作瓦片“晃动”。
        state.distance = state.target_distance;
    }

    // 缩放弛豫：使 `distance` 向滚轮设定的目标滑行，使场景连续缩放
    // （CesiumJS 以同样方式缓动缩放；一个硬的一格一跳会读作瓦片“晃动”）。
    // 右键拖拽不走这里：它在上面的分支里已经与光标逐像素同步。
    let k = 1.0 - (-10.0f32 * time.delta_secs()).exp();
    state.distance += (state.target_distance - state.distance) * k;
    if (state.target_distance - state.distance).abs() < 1.0e-5 {
        state.distance = state.target_distance;
    }

    // 倾斜：中键拖拽 —— 绕「按下瞬间画布中心命中的椭球点」旋转相机
    // （CesiumJS `tilt3DOnEllipsoid`）。左键「抓地」旋转、滚轮/右键缩放都
    // 已在上面消费完本帧输入，因此这里再改位姿不会与它们打架。
    let win_size = windows
        .get_single()
        .map(|w| (f64::from(w.width()), f64::from(w.height())))
        .unwrap_or((1280.0, 720.0));
    apply_tilt_drag(
        &mut state,
        &mut arcball,
        &mut tilt,
        tilt_pressed,
        (f64::from(tilt_dx), f64::from(tilt_dy)),
        win_size,
        flight.flight.is_some(),
    );

    // 应用 transform：engaged 时用 trackball 位姿，否则用纯球面北向上
    // 路径（与 arcball 前的基线字节相同）。倾斜多出来的旋转（相机不再注视
    // 地心那部分）挂在 [`OrbitTilt::aim`] 上；`tilt.engaged == false` 时
    // 完全不触碰朝向，因此无倾斜输入的路径逐字节不变。
    if let Ok(mut transform) = query.get_single_mut() {
        *transform = if arcball.engaged {
            transform_from_arcball(arcball.orientation.as_quat(), state.distance, state.target)
        } else {
            compute_camera_transform(&state)
        };
        if tilt.engaged {
            transform.rotation = (transform.rotation.as_dquat() * tilt.aim)
                .normalize()
                .as_quat();
        }
    }
}

/// M2.4：旋转惯性滑行系统（委派给领域 [`InertiaController`]）。
///
/// 在 [`orbit_camera_system`] 之后运行。跟踪帧之间的 heading/pitch 增量；
/// 在一次快速拂动释放时，捕获速度并以指数衰减滑行。当飞行处于活动
/// 状态时被抑制。
#[allow(clippy::too_many_arguments)] // Bevy system: one param per resource/event/query
fn orbit_inertia_system(
    mut state: ResMut<OrbitState>,
    mut inertia: ResMut<OrbitInertiaState>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    flight_state: Res<OrbitFlightState>,
    mut query: Query<&mut Transform, With<OrbitCamera>>,
    windows: Query<&Window>,
    mut arcball: ResMut<Arcball>,
) {
    // 推进惯性计时的单调时钟。
    inertia.now_ms += time.delta_secs() as f64 * 1000.0;

    let is_dragging = mouse_buttons.pressed(MouseButton::Left);
    let flight_active = flight_state.flight.is_some();

    // 用于弧度↔像素边界转换的窗口高度（领域 InertiaController 在像素
    // 空间滑行，参见 `inertia_pixel_scale`）。
    let win_h = windows
        .get_single()
        .map(|w| w.height())
        .unwrap_or(720.0);

    // ── 检测拖拽开始 ──────────────────────────────────────────────
    if is_dragging && !inertia.was_dragging {
        inertia.coasting = false;
        inertia.controller.deactivate(InertiaState::Spin);
        inertia.press_time_ms = inertia.now_ms;
    }

    // ── 检测拖拽释放 → 捕获惯性（弧度 → 像素）──────────
    if inertia.was_dragging && !is_dragging && !flight_active {
        inertia.release_time_ms = inertia.now_ms;
        let hold_secs = (inertia.release_time_ms - inertia.press_time_ms) / 1000.0;
        let heading_delta = (state.heading - inertia.prev_heading) as f64;
        let pitch_delta = (state.pitch - inertia.prev_pitch) as f64;

        if hold_secs < INERTIA_MAX_CLICK_TIME_THRESHOLD
            && (heading_delta.abs() > 1e-8 || pitch_delta.abs() > 1e-8)
        {
            // 将弧度速度转为像素空间：领域在像素中滑行，其
            // `INERTIA_STOP_DISTANCE` 守卫（0.5 px）对原始弧度（约 0.01）毫无
            // 意义。该尺度镜像抓地投影，因此往返精确。
            let (scale_h, scale_p) = inertia_pixel_scale(&state, win_h);
            inertia.capture_scale_h = scale_h;
            inertia.capture_scale_p = scale_p;
            // capture 存储 motion = (end - start) * 0.5，因此传入 end = 2×delta。
            // trackball engaged 时 `heading` 通过 atan2 重新导出，越极时可能环绕
            // ±π；将每帧增量归一化，以免一次环绕伪装成巨大速度（→ 失控滑行）。
            let motion_px = DVec2::new(
                wrap_pi(heading_delta as f32) as f64 * scale_h as f64,
                pitch_delta * scale_p as f64,
            );
            inertia
                .controller
                .capture(InertiaState::Spin, DVec2::ZERO, motion_px * 2.0);
            inertia.controller.activate(Some(InertiaState::Spin));
            inertia.coasting = true;
        } else {
            inertia.coasting = false;
        }
    }

    inertia.was_dragging = is_dragging;

    // ── 以指数衰减滑行（像素 → 弧度）───────────────────
    if inertia.coasting && !is_dragging && !flight_active {
        let sample = InertiaSample::new(
            INERTIA_SPIN_COEFFICIENT,
            inertia.press_time_ms,
            inertia.release_time_ms,
            inertia.now_ms,
        );
        // 在可变借用 `inertia` 之前快照捕获时的尺度。
        let scale_h = inertia.capture_scale_h as f64;
        let scale_p = inertia.capture_scale_p as f64;
        match inertia.controller.maintain(InertiaState::Spin, &sample) {
            Some(delta_px) => {
                let d_heading = (delta_px.x / scale_h) as f32;
                let d_pitch = (delta_px.y / scale_p) as f32;
                if arcball.engaged {
                    // 以拖拽驱动它相同的方式滑行实时 trackball：绕相机 up
                    // 偏航，绕相机 right 俯仰。
                    let up = (arcball.orientation * DVec3::Y).normalize();
                    let right = (arcball.orientation * DVec3::X).normalize();
                    let q = DQuat::from_axis_angle(up, f64::from(d_heading))
                        * DQuat::from_axis_angle(right, f64::from(d_pitch));
                    arcball.orientation = (q * arcball.orientation).normalize();
                    let (h, p) = orbit_from_orientation(arcball.orientation);
                    state.heading = h;
                    state.pitch = p;
                    if let Ok(mut transform) = query.get_single_mut() {
                        *transform = transform_from_arcball(
                            arcball.orientation.as_quat(),
                            state.distance,
                            state.target,
                        );
                    }
                } else {
                    state.heading += d_heading;
                    state.pitch = (state.pitch + d_pitch).clamp(-1.5, 1.5);
                    if let Ok(mut transform) = query.get_single_mut() {
                        *transform = compute_camera_transform(&state);
                    }
                }
            }
            None => {
                inertia.coasting = false;
            }
        }
    }

    // 存储当前 heading/pitch，用于下一帧的增量计算。
    inertia.prev_heading = state.heading;
    inertia.prev_pitch = state.pitch;
}

/// 在当前 orbit 状态下的每弧度像素尺度因子 `(heading, pitch)`。
///
/// 领域 [`InertiaController`] 在**像素空间**滑行 —— 其 `INERTIA_STOP_DISTANCE`
/// 守卫是 0.5 px —— 因此应用边界在捕获前将旋转增量从弧度转为像素，
/// 在 [`InertiaController::maintain`] 后再转回弧度。这些因子是几何抓地增益的
/// 精确逆：`focal = (H/2)/tan(fov/2)`，`surface_dist = distance - R`（与拖拽
/// 回退所用的同一逆）。捕获与滑行使用同一尺度使像素往返无损，因此
/// 滑行的运动是释放时弧度速度的干净指数衰减。
fn inertia_pixel_scale(state: &OrbitState, win_h: f32) -> (f32, f32) {
    let focal = (win_h * 0.5) / (CAMERA_FOV_Y * 0.5).tan();
    let surface_dist = (state.distance - GLOBE_RADIUS).max(0.001);
    let s = focal / surface_dist;
    (s, s)
}

/// 为 arcball 相机系架构建穿过 OS 光标的世界空间拾取射线（原点 +
/// 单位方向）。当光标位置不可用（例如指针在窗口外）时为 `None`。使用
/// 自定义视锥（`CAMERA_FOV_Y`）与窗口宽高比；相机沿其局部 -Z 看，位于
/// `target + orientation·(Ẑ · distance)`。
fn cursor_ray_world(
    window: &Window,
    orientation: DQuat,
    distance: f32,
    target: Vec3,
) -> Option<(DVec3, DVec3)> {
    let cursor = window.cursor_position()?;
    let w = window.width().max(1.0);
    let h = window.height().max(1.0);
    let aspect = w / h;
    let x_ndc = (cursor.x / w) * 2.0 - 1.0;
    let y_ndc = 1.0 - (cursor.y / h) * 2.0;
    let tan_y = (CAMERA_FOV_Y * 0.5).tan();
    let tan_x = tan_y * aspect;
    // 下游一切（锚点拾取 + 旋转）都以 f64 运行，使微小的最大缩放
    // 残差得以留存；像素/FOV 输入被精确拓宽。
    let dir_cam = DVec3::new(
        f64::from(x_ndc * tan_x),
        f64::from(y_ndc * tan_y),
        -1.0,
    )
    .normalize();
    let origin = target.as_dvec3() + orientation * (DVec3::Z * f64::from(distance));
    let dir = (orientation * dir_cam).normalize();
    Some((origin, dir))
}

/// 使用相机的真实投影构建穿过 OS 光标的世界空间拾取射线 ——
/// `Camera::viewport_to_world` 复合了实际的 `clip_from_view` 矩阵与渲染的
/// `GlobalTransform`，因此它返回的射线正是屏幕上那一帧绘制所沿的视线。
/// 这是 [`cursor_ray_world`] 的非循环替代：我们不再假设手工推导的视锥与
/// 渲染器匹配，而是去问渲染器。近平面 `origin` + 单位 `direction` 立即
/// 拓宽到 f64，使下游锚点旋转保持最大缩放下的微小角精度（f32 射线
/// 方向携带 ~1e-7 rad，远低于 ~1e-5 rad 的信号）。
fn cursor_ray_bevy(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    window: &Window,
) -> Option<(DVec3, DVec3)> {
    let cursor = window.cursor_position()?;
    let ray = camera.viewport_to_world(camera_transform, cursor).ok()?;
    let origin = ray.origin.as_dvec3();
    let dir = Vec3::from(ray.direction).as_dvec3();
    Some((origin, dir))
}

/// 将一条世界射线与 WGS84 椭球（以 `center` 为中心，x/y 为赤道半径
/// `ELLIPSOID_A`，z 为极地半径 `ELLIPSOID_B` —— 与瓦片网格细分所在的 SAME
/// 表面）相交，返回从中心到就近命中点的单位方向。未命中或唯一交点在
/// 相机后方时为 `None`。这取代了旧的单位球拾取，用于抓地拖拽，使
/// 锚点是用户在地面上实际看到的那个点，而非其上方漂浮的球体 ——
/// 两者之间的径向间隙正是使深缩放拖拽过冲的原因。
fn pick_surface_dir_ellipsoid(origin: DVec3, dir: DVec3, center: DVec3) -> Option<DVec3> {
    // 各向异性缩放到单位球空间，解 |o + t·d|² = 1，将就近命中点缩回
    // 世界。`dir` 在世界中是单位向量，但缩放后不是，因此保留一般二次式
    // （a ≠ 1）。
    let s = DVec3::new(1.0 / ELLIPSOID_A, 1.0 / ELLIPSOID_A, 1.0 / ELLIPSOID_B);
    let os = (origin - center) * s;
    let ds = dir * s;
    let a = ds.dot(ds);
    let half_b = os.dot(ds);
    let c = os.dot(os) - 1.0;
    let disc = half_b * half_b - a * c;
    if disc <= 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let t0 = (-half_b - sq) / a;
    let t = if t0 > 0.0 { t0 } else { (-half_b + sq) / a };
    if t <= 0.0 {
        return None;
    }
    let hit = center + (os + ds * t) / s;
    (hit - center).try_normalize()
}

/// 将单位方向 `from` 变到单位方向 `to` 的最短弧旋转，计算为
/// `axis = from × to`，`angle = atan2(|axis|, from·to)`。
///
/// 这取代了 `Quat::from_rotation_arc`，它对抓地锚点不可用：只要 `dot > 1 − ε`
/// 它就退回 `IDENTITY`。在最大缩放（相机位于表面上方 ~765 m）下，两个
/// 地心表面方向 —— 闭锁的锚点与当前光标拾取 —— 仅相差 ~3e-5 rad，因此它们
/// 的 f32 dot 舍入为 1.0 且固定旋转退化为单位，使拖拽冻结在 0 px。叉积
/// 量级（~3e-5）仍高出 f32 次正规 floor 数个量级，因此 `atan2` 恢复真实
/// 微小角度，抓取点一路到地表都 1:1 跟踪光标 —— 同时同一绝对锚点数学在
/// 宽视野/整球缩放下保持精确（从不过旋），不像一个随相机高度缩放的
/// 每像素增益。
fn rotation_from_unit_dir(from: DVec3, to: DVec3) -> DQuat {
    let axis = from.cross(to);
    let sin = axis.length();
    let cos = from.dot(to);
    if sin < 1.0e-12 {
        // （反）平行：无有意义的旋转轴。
        return if cos < 0.0 {
            // 绕任一垂直于 `from` 的单位轴旋转 180°。
            let perp = if from.x.abs() < from.y.abs() { DVec3::X } else { DVec3::Y };
            from.cross(perp)
                .try_normalize()
                .map(|a| DQuat::from_axis_angle(a, std::f64::consts::PI))
                .unwrap_or(DQuat::IDENTITY)
        } else {
            DQuat::IDENTITY
        };
    }
    DQuat::from_axis_angle(axis / sin, sin.atan2(cos))
}

/// 将旋转的角软上限到至多 `max_angle` 弧度，保留其轴。用于限定每
/// 帧的“抓地”修正：一个巨大的单帧残差（快速拂动、合并输入风暴，或
/// 经线收敛的近极拾取）否则会传送视图；钳位使其能在几帧内平滑收敛。
/// 一个近单位或已经很小的旋转会原样返回，因此普通拖拽保持字节相同
/// （精确 1:1）—— 只有过大的猛拉会被软化。
fn clamp_rotation_angle(q: DQuat, max_angle: f32) -> DQuat {
    let (axis, angle) = q.to_axis_angle();
    let max_angle = f64::from(max_angle);
    if angle <= max_angle || axis.length_squared() < 1.0e-12 {
        return q;
    }
    DQuat::from_axis_angle(axis, max_angle)
}

/// 将一个角归一化到 (-π, π] 区间。防止 trackball 穿越极点时 ±π 方位角
/// 环绕对惯性捕获造成影响。
fn wrap_pi(a: f32) -> f32 {
    let two_pi = 2.0 * std::f32::consts::PI;
    let mut x = (a + std::f32::consts::PI) % two_pi;
    if x < 0.0 {
        x += two_pi;
    }
    x - std::f32::consts::PI
}

/// M2.4：大圆航线飞行系统（委派给领域 [`CameraFlight`]）。
///
/// 最后运行，使飞行对 orbit 状态有最终决定权。使用领域的大圆 slerp
/// 插值，配合从 [`compute_flight_duration`] 与 [`select_flight_easing`] 导出的
/// 自动时长/缓动。
fn orbit_flight_system(
    mut flight_state: ResMut<OrbitFlightState>,
    mut state: ResMut<OrbitState>,
    time: Res<Time>,
    mut query: Query<&mut Transform, With<OrbitCamera>>,
    mut fly_requests: EventReader<OrbitFlyToRequest>,
    mut arcball: ResMut<Arcball>,
) {
    // 处理新的 fly-to 请求。一次 fly-to 将控制权交回确定性的北向上球面
    // 路径，因此丢弃实时 trackball。
    for request in fly_requests.read() {
        arcball.engaged = false;
        arcball.anchor = None;
        orbit_fly_to(&state, request.destination_ecef, &mut flight_state);
    }

    let flight = match flight_state.flight.as_mut() {
        Some(f) if !f.complete => f,
        _ => return,
    };

    let dt = time.delta_secs() as f64;
    if let Some((position, _direction, _up)) = flight.update(dt) {
        let meters_per_render_unit = 6378137.0_f64;
        let (heading, pitch, distance) = ecef_to_orbit(position, meters_per_render_unit);
        state.heading = heading;
        state.pitch = pitch.clamp(-1.5, 1.5);
        state.distance = distance;
        state.target_distance = distance;
        if let Ok(mut transform) = query.get_single_mut() {
            *transform = compute_camera_transform(&state);
        }
    }

    if flight.complete {
        flight_state.flight = None;
    }
}

/// 发起一次到给定 ECEF 目的点（米）的大圆航线飞行。
///
/// 时长与缓动使用领域的 [`compute_flight_duration`] 与 [`select_flight_easing`]
/// 从距离自动导出。
pub(crate) fn orbit_fly_to(
    state: &OrbitState,
    destination_ecef: DVec3,
    flight_state: &mut OrbitFlightState,
) {
    let meters_per_render_unit = 6378137.0_f64;
    let position = orbit_position_to_ecef(state, meters_per_render_unit);
    let direction = -position.normalize();
    let cam = cesium_camera::Camera::new(position, direction, DVec3::Z);

    let distance = (destination_ecef - position).length();
    let duration = compute_flight_duration(distance);
    let mut flight = CameraFlight::fly_to(&cam, destination_ecef, None, None, duration);
    flight.easing = select_flight_easing(distance);
    flight_state.flight = Some(flight);
}

/// 将 orbit 状态转为米下的 ECEF 位置。
fn orbit_position_to_ecef(state: &OrbitState, meters_per_render_unit: f64) -> DVec3 {
    let d = state.distance as f64 * meters_per_render_unit;
    let cos_pitch = (state.pitch as f64).cos();
    let sin_pitch = (state.pitch as f64).sin();
    let heading = state.heading as f64;
    DVec3::new(
        d * cos_pitch * heading.cos(),
        d * cos_pitch * heading.sin(),
        d * sin_pitch,
    )
}

/// 将一个 ECEF 位置（米）转回 orbit 球坐标。
fn ecef_to_orbit(position: DVec3, meters_per_render_unit: f64) -> (f32, f32, f32) {
    let r = position.length();
    let distance = (r / meters_per_render_unit) as f32;
    let pitch =
        position.z.atan2((position.x * position.x + position.y * position.y).sqrt()) as f32;
    let heading = position.y.atan2(position.x) as f32;
    (heading, pitch, distance)
}

/// 从球面 orbit 状态计算相机 Transform。
///
/// 地球是 ECEF：北极在 +Z，赤道在 XY 平面。相机位置以围绕 Z（极地）
/// 轴的球坐标表达：
///   x = distance * cos(pitch) * cos(heading)
///   y = distance * cos(pitch) * sin(heading)
///   z = distance * sin(pitch)
/// 且相机的“上”是地球的 +Z 轴，因此北总是朝上。
fn compute_camera_transform(state: &OrbitState) -> Transform {
    let cos_pitch = state.pitch.cos();
    let sin_pitch = state.pitch.sin();

    let offset = Vec3::new(
        state.distance * cos_pitch * state.heading.cos(),
        state.distance * cos_pitch * state.heading.sin(),
        state.distance * sin_pitch,
    );

    let position = state.target + offset;
    Transform::from_translation(position).looking_at(state.target, Vec3::Z)
}

// ── FIX-ARCBALL 助手 ────────────────────────────────────

/// 球面 orbit 状态给出的相机位置（f64）。与 [`compute_camera_transform`] 的
/// 展开同式，但保留 f64 —— 倾斜的每帧增量可以小到 1e-7 render unit 量级。
fn orbit_position_f64(state: &OrbitState, target: DVec3) -> DVec3 {
    let (heading, pitch, distance) = (
        f64::from(state.heading),
        f64::from(state.pitch),
        f64::from(state.distance),
    );
    target
        + DVec3::new(
            distance * pitch.cos() * heading.cos(),
            distance * pitch.cos() * heading.sin(),
            distance * pitch.sin(),
        )
}

/// 「注视地心、北朝上」的基座朝向，f64 版本（与
/// `Transform::looking_at(target, Vec3::Z)` 同约定：局部 +Z 从目标指向相机，
/// 局部 +Y 尽量贴近世界 +Z）。
///
/// 与 `looking_at` 的唯一区别是退化处理：相机恰在极轴上时「视线 × 世界上」
/// 为零向量，`looking_at` 会产出 NaN/退化系架。这里改用与 `back` 垂直的
/// 备用种子方向（CesiumJS 的 `eastNorthUpToFixedFrame` 在极点同样需要人为
/// 指定东向），保证倾斜路径永不产生 NaN。
fn base_look_at_rotation(position: DVec3, target: DVec3) -> DQuat {
    let back = (position - target).try_normalize().unwrap_or(DVec3::Z);
    let world_up = DVec3::Z;
    let up = (world_up - back * world_up.dot(back))
        .try_normalize()
        .or_else(|| {
            let seed = if back.z.abs() > 0.9 {
                DVec3::X
            } else {
                DVec3::Z
            };
            (seed - back * seed.dot(back)).try_normalize()
        })
        .unwrap_or(DVec3::X);
    let right = up.cross(back);
    DQuat::from_mat3(&DMat3::from_cols(right, up, back))
}

/// 构建一个复现当前球面状态旧式北向上极地位姿的 arcball 朝向。用于
/// 在拖拽开始的那一刻将 trackball 闭锁到现有视图，因此接管是无缝的。
fn arcball_quat_from_spherical(state: &OrbitState) -> Quat {
    compute_camera_transform(state).rotation
}

/// 由 arcball 朝向得到的相机 `Transform`：相机位于
/// `target + orientation·(Ẑ · distance)` 并沿 `-orientation·Ẑ` 回看。对于由
/// [`arcball_quat_from_spherical`] 产生的朝向，它与 [`compute_camera_transform`]
/// 相同（系架的 +Z 轴从 target 指向相机，因此 -Z —— Bevy 的相机前方 —— 目
/// 标向 target）。
fn transform_from_arcball(orientation: Quat, distance: f32, target: Vec3) -> Transform {
    let position = target + orientation * (Vec3::Z * distance);
    Transform::from_translation(position).with_rotation(orientation)
}

/// 从 arcball 朝向导出 `(heading, pitch)` —— 仅视图方向，故意丢弃 roll ——
/// 使球面消费者（globe LOD 子相机点、惯性）在 trackball 存活时保持填充。
fn orbit_from_orientation(orientation: DQuat) -> (f32, f32) {
    let dir = orientation * DVec3::Z; // normalize(position - target)
    (
        dir.y.atan2(dir.x) as f32,
        dir.z.clamp(-1.0, 1.0).asin() as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbit_state_default_is_pixel_neutral() {
        let state = OrbitState::default();
        assert_eq!(state.heading, 0.0);
        assert!((state.pitch - 0.4).abs() < 1e-6);
        assert!((state.distance - 3.0).abs() < 1e-6);
    }

    #[test]
    fn zoom_drag_up_zooms_in_and_down_zooms_out() {
        // 用户要求的映射：向上拖（dy < 0）= 放大（接近地表，高度变小），
        // 向下拖（dy > 0）= 缩小（拉远）。这与滚轮方向一致（上滚 = 放大），
        // 并且**有意**与 CesiumJS `handleZoom` 的 `approachingSurface = diff > 0`
        // （向下拖接近地面）相反。
        let up = zoom_drag_target_surface(1.0, -72.0, 720.0);
        let down = zoom_drag_target_surface(1.0, 72.0, 720.0);
        assert!(up < 1.0, "向上拖必须接近地表，得到 {up}");
        assert!(down > 1.0, "向下拖必须拉远，得到 {down}");
        // 72 px = 0.1 个窗口高度 → e^(0.1 · ZOOM_DRAG_RANGE) ≈ 1.162 倍，
        // 且两个方向互为倒数（往回拖能精确抵消）。
        assert!(
            (down - (0.1 * ZOOM_DRAG_RANGE).exp()).abs() < 1e-5,
            "得到 {down}"
        );
        assert!((up * down - 1.0).abs() < 1e-5, "up={up} down={down}");
        // 整屏高度（720 px）的位移改变 e^ZOOM_DRAG_RANGE ≈ 4.48 倍。
        let full_window = zoom_drag_target_surface(1.0, 720.0, 720.0);
        assert!(
            (full_window - ZOOM_DRAG_RANGE.exp()).abs() < 1e-5,
            "得到 {full_window}"
        );
        // 零位移 = 原地不动。
        assert!((zoom_drag_target_surface(1.0, 0.0, 720.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn zoom_drag_span_is_capped_per_drag() {
        // 单次拖拽的累计位移按 `ZOOM_DRAG_MAX_SPAN` 个窗口高度计：最多
        // e^2.25 ≈ 9.49 倍，而不是把相机瞬移到天边。光标在窗口内物理上只能
        // 走一屏高（≈ 4.48 倍，见下一个断言），因此这条上限只在光标被拖出
        // 窗口、或窗口远小于屏幕时才生效。
        let huge = zoom_drag_target_surface(1.0, 100_000.0, 720.0);
        assert!(
            (huge - ZOOM_DRAG_MAX_SPAN.exp()).abs() < 1e-3,
            "得到 {huge}"
        );
        // 窗口高度退化（0）时 `max(1.0)` 兜底，且仍受同一上限保护。
        let degenerate = zoom_drag_target_surface(1.0, 1.0e9, 0.0);
        assert!((degenerate - ZOOM_DRAG_MAX_SPAN.exp()).abs() < 1e-3);
        // 一屏高的拖拽不触发上限：正常拖拽永远够不到它。
        let full_window = zoom_drag_target_surface(1.0, 720.0, 720.0);
        assert!(
            (full_window - ZOOM_DRAG_RANGE.exp()).abs() < 1e-5,
            "得到 {full_window}"
        );
    }

    #[test]
    fn zoom_drag_advance_prefers_cursor_and_rate_limits_fallback() {
        // 首选路径：绝对光标位移（逻辑像素），与鼠标 DPI / 指针速度设置无关，
        // 并忽略原始计数 —— 光标走过的逻辑像素是唯一尺度。拖回按下点 = 0。
        let (total, anchor, tracking) = zoom_drag_advance(Some(300.0), 100.0, 0.0, true, 9_999.0);
        assert!((total - 200.0).abs() < 1e-6, "得到 {total}");
        assert!((anchor - 100.0).abs() < 1e-6, "得到 {anchor}");
        assert!(tracking);
        let (back, _, _) = zoom_drag_advance(Some(100.0), 100.0, 200.0, true, 0.0);
        assert!(back.abs() < 1e-6, "得到 {back}");
        // 回退路径（无 `Window`）：逐帧限流到 `ZOOM_DRAG_MAX_PX_PER_FRAME`，
        // 一次拂动 / 卡顿后合并送达的输入风暴不会把相机瞬移出去。
        let (capped, _, tracking) = zoom_drag_advance(None, 0.0, 0.0, false, 100_000.0);
        assert!(
            (capped - ZOOM_DRAG_MAX_PX_PER_FRAME).abs() < 1e-6,
            "得到 {capped}"
        );
        assert!(!tracking);
        let (capped_up, _, _) = zoom_drag_advance(None, 0.0, 0.0, false, -100_000.0);
        assert!((capped_up + ZOOM_DRAG_MAX_PX_PER_FRAME).abs() < 1e-6);
        // 门限内的小增量原样累计（正常拖拽不受限流影响）。
        let (within_limit, _, _) = zoom_drag_advance(None, 0.0, 5.0, false, 6.0);
        assert!((within_limit - 11.0).abs() < 1e-6, "得到 {within_limit}");
    }

    #[test]
    fn zoom_drag_advance_rebases_when_the_path_switches() {
        // 拖出窗口：光标被 winit 置为 `None`（`CursorLeft`），改用原始计数继续。
        let (raw_total, anchor, tracking) = zoom_drag_advance(None, 100.0, 200.0, true, 40.0);
        assert!((raw_total - 240.0).abs() < 1e-6, "得到 {raw_total}");
        assert!((anchor - 100.0).abs() < 1e-6, "锚点在回退路径上不动");
        assert!(!tracking);
        // 拖回窗口：光标重新可用。若沿用旧锚点（100）会得到 500-100=400，凭空
        // 多出 160 px；正确做法是重锚（`anchor = 500 - 240 = 260`）保持连续。
        let (back_total, rebased, tracking) =
            zoom_drag_advance(Some(500.0), anchor, raw_total, tracking, 0.0);
        assert!((back_total - 240.0).abs() < 1e-6, "得到 {back_total}");
        assert!((rebased - 260.0).abs() < 1e-6, "得到 {rebased}");
        assert!(tracking);
        // 重锚之后继续用绝对位移：再往下 60 px ⇒ 300。
        let (continued, _, _) = zoom_drag_advance(Some(560.0), rebased, back_total, true, 0.0);
        assert!((continued - 300.0).abs() < 1e-6, "得到 {continued}");
    }

    #[test]
    fn zoom_drag_factor_is_independent_of_height() {
        // 乘性映射：同样的像素位移在任何高度给出同一个倍率，因此掠地与远观
        // 的感知速率一致（这也是「太快」被消除的原因 —— 旧实现是
        // `5.0 · 高度 · 位移/窗高`，每帧最多改变 50%）。
        let near = zoom_drag_target_surface(0.01, 36.0, 720.0) / 0.01;
        let far = zoom_drag_target_surface(1.0, 36.0, 720.0);
        assert!((far - near).abs() < 1e-6, "near={near} far={far}");
    }

    #[test]
    fn clamp_rotation_angle_bounds_large_steps_only() {
        // 一个小的旋转（普通拖拽步长）原样通过，字节相同。
        let small = DQuat::from_axis_angle(DVec3::Z, 0.02);
        assert_eq!(clamp_rotation_angle(small, MAX_DRAG_STEP_RAD), small);

        // 一个巨大旋转（传送大小的猛拉）被钳位到上限同时保留其轴，
        // 因此视图滑行而非跳跃。
        let big = DQuat::from_axis_angle(DVec3::X, 1.2);
        let (axis, angle) = clamp_rotation_angle(big, MAX_DRAG_STEP_RAD).to_axis_angle();
        assert!((angle - f64::from(MAX_DRAG_STEP_RAD)).abs() < 1e-5);
        assert!((axis.abs() - DVec3::X.abs()).length() < 1e-5);
    }

    #[test]
    fn clamp_rotation_angle_preserves_1to1_tracking_scale() {
        // 在整球缩放下，一个普通快速拖拽步长（~0.03 rad）必须不被钳位，
        // 因此抓取点仍精确落在光标下。
        let typical = DQuat::from_axis_angle(DVec3::Y, 0.03);
        assert_eq!(clamp_rotation_angle(typical, MAX_DRAG_STEP_RAD), typical);
    }

    #[test]
    fn compute_transform_produces_correct_position() {
        let state = OrbitState {
            heading: 0.0,
            pitch: 0.0,
            distance: 3.0,
            ..Default::default()
        };
        let t = compute_camera_transform(&state);
        // heading=0、pitch=0 时：position = (3, 0, 0)
        assert!((t.translation.x - 3.0).abs() < 1e-5);
        assert!((t.translation.y).abs() < 1e-5);
        assert!((t.translation.z).abs() < 1e-5);
    }

    #[test]
    fn ecef_to_orbit_roundtrip() {
        let mpru = 6378137.0_f64;
        let state = OrbitState {
            heading: 0.5,
            pitch: 0.3,
            distance: 3.0,
            ..Default::default()
        };
        let ecef = orbit_position_to_ecef(&state, mpru);
        let (h, p, d) = ecef_to_orbit(ecef, mpru);
        assert!((h - state.heading).abs() < 1e-5);
        assert!((p - state.pitch).abs() < 1e-5);
        assert!((d - state.distance).abs() < 1e-4);
    }

    #[test]
    fn flight_duration_and_easing_from_domain() {
        // 短跳 -> quintic，最少 1s。
        assert!((compute_flight_duration(500_000.0) - 1.0).abs() < 1e-12);
        assert_eq!(
            select_flight_easing(500_000.0),
            cesium_camera::EasingFunction::QuinticInOut
        );
        // 长跳 -> cubic，上限 5s。
        assert!((compute_flight_duration(10_000_000.0) - 5.0).abs() < 1e-12);
        assert_eq!(
            select_flight_easing(2_000_000.0),
            cesium_camera::EasingFunction::CubicInOut
        );
    }

    #[test]
    fn orbit_fly_to_creates_valid_flight() {
        let state = OrbitState::default();
        let mut fs = OrbitFlightState::default();
        let dest = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);
        orbit_fly_to(&state, dest, &mut fs);
        assert!(fs.flight.is_some());
        let f = fs.flight.as_ref().unwrap();
        assert!(!f.complete);
        assert!(f.duration >= 1.0 && f.duration <= 5.0);
    }

    #[test]
    fn inertia_coasts_in_pixel_space_via_boundary_conversion() {
        // 领域 InertiaController 在像素空间滑行（0.5 px 停止守卫），因此
        // 应用在捕获时将弧度速度→像素，在 maintain 时再转回弧度。馈入原始
        // 弧度（~0.01）会直接穿过停止守卫且永不滑行。
        let state = OrbitState {
            heading: 0.0,
            pitch: 0.4,
            distance: 3.0,
            ..Default::default()
        };
        let (scale_h, scale_p) = inertia_pixel_scale(&state, 720.0);
        // 在 distance=3（surface_dist=2）、focal≈623.5 时：尺度 >>1 px/rad，
        // 因此一次 0.02 rad 拂动是多像素运动，能越过守卫。
        assert!(scale_h > 100.0 && scale_p > 100.0);

        // 一次真实拂动：一帧内 ~0.02 rad heading / 0.01 rad pitch。
        let heading_delta = 0.02_f64;
        let pitch_delta = 0.01_f64;
        let motion_px = DVec2::new(heading_delta * scale_h as f64, pitch_delta * scale_p as f64);

        let mut ctrl = InertiaController::new();
        ctrl.capture(InertiaState::Spin, DVec2::ZERO, motion_px * 2.0);
        ctrl.activate(Some(InertiaState::Spin));

        // 首个滑行帧（释放后 16 ms）：仍高于守卫。
        let sample = InertiaSample::new(INERTIA_SPIN_COEFFICIENT, 0.0, 0.0, 16.0);
        let delta_px = ctrl.maintain(InertiaState::Spin, &sample).expect("coasting");

        // 转回弧度：decay(0.016s, 0.9) = exp(-2.5*0.016) ≈ 0.9608，因此滑行的
        // 弧度速度略低于释放速度。
        let heading_back = delta_px.x / scale_h as f64;
        let pitch_back = delta_px.y / scale_p as f64;
        assert!(heading_back > 0.0 && heading_back <= heading_delta);
        assert!(
            (heading_back - heading_delta * 0.9608).abs() < 1e-3,
            "heading coast {heading_back} should ≈ {}",
            heading_delta * 0.9608
        );
        assert!(pitch_back > 0.0 && pitch_back <= pitch_delta);
    }

    #[test]
    fn inertia_pixel_scale_matches_grab_the_globe_gain() {
        // 尺度必须是抓地增益的精确逆，以使弧度→像素→弧度的往返无损。
        // 两轴都使用简单几何增益（focal/surface_dist）；旧的 1/cos(pitch)
        // 经线收敛因子属于旧式 ECEF-绕-Z 模型，在 arcball（相机相关）跟踪接管
        // 时已被丢弃。
        let state = OrbitState {
            pitch: 0.3,
            distance: 4.0,
            ..Default::default()
        };
        let win_h = 900.0_f32;
        let (scale_h, scale_p) = inertia_pixel_scale(&state, win_h);
        let focal = (win_h * 0.5) / (CAMERA_FOV_Y * 0.5).tan();
        let surface_dist = state.distance - GLOBE_RADIUS;
        assert!((scale_h - focal / surface_dist).abs() < 1e-3);
        assert!((scale_p - focal / surface_dist).abs() < 1e-3);

        // 往返：一个弧度增量 → 像素 → 弧度是恒等。
        let d_heading = 0.05_f64;
        let px = d_heading * scale_h as f64;
        assert!((px / scale_h as f64 - d_heading).abs() < 1e-9);
    }

    #[test]
    fn inertia_stops_after_threshold() {
        let mut ctrl = InertiaController::new();
        ctrl.capture(InertiaState::Spin, DVec2::ZERO, DVec2::new(0.02, 0.0));
        // 保持 0.5s ≥ INERTIA_MAX_CLICK_TIME_THRESHOLD → 无滑行。
        let sample = InertiaSample::new(INERTIA_SPIN_COEFFICIENT, 0.0, 500.0, 516.0);
        assert!(ctrl.maintain(InertiaState::Spin, &sample).is_none());
    }

    /// M2.4 验证门：无头关键帧播放中性。
    ///
    /// 精确复现 `--headless --camera-script` 路径：`camera_script_system` 每帧
    /// 覆写 `OrbitState` 且**无**鼠标/滚轮/fly-to 输入，因此委派的旋转惯性与
    /// 大圆航线飞行系统必须贡献恰好为零。所得的 `Transform` 因此等于脚本状态
    /// 的纯抓地 transform —— 与旧式 M0 构建位相同（pos/quat 差
    /// `0.0 < 1e-4` render unit）对每个脚本位姿都成立。
    ///
    /// 十个不同的脚本位姿代替代十个手势脚本；中性论证是逐帧且与脚本无关的，
    /// 因此这覆盖了整个系列。在 `MinimalPlugins`（无 GPU / 渲染后端）上运行。
    #[test]
    fn headless_keyframe_playback_matches_legacy_within_1e4() {
        // 一个 10 位姿的脚本轨迹（heading 扫过一整圈，pitch 与 distance 变化）
        // —— 捕获脚本的确定性等价物。
        let scripted: Vec<(f32, f32, f32)> = (0..10)
            .map(|i| {
                let t = i as f32 / 9.0;
                (
                    t * std::f32::consts::TAU,
                    0.4 + 0.15 * t,
                    3.0 - 0.75 * t,
                )
            })
            .collect();

        let mut max_pos_err = 0.0_f32;
        let mut max_quat_err = 0.0_f32;

        for &(heading, pitch, distance) in &scripted {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .add_event::<MouseMotion>()
                .add_event::<MouseWheel>()
                .add_event::<OrbitFlyToRequest>()
                .init_resource::<ButtonInput<MouseButton>>()
                .init_resource::<OrbitInertiaState>()
                .init_resource::<OrbitFlightState>()
                .init_resource::<Arcball>()
                .init_resource::<OrbitTilt>()
                .init_resource::<OrbitZoomDrag>()
                .init_resource::<CameraGesture>()
                .insert_resource(OrbitState {
                    heading,
                    pitch,
                    distance,
                    target_distance: distance,
                    ..Default::default()
                })
                .add_systems(
                    Update,
                    (orbit_camera_system, orbit_inertia_system, orbit_flight_system).chain(),
                );
            let cam = app.world_mut().spawn((OrbitCamera, Transform::IDENTITY)).id();

            // 一个无输入事件的无头帧（镜像播放）。
            app.update();

            let got = *app.world().get::<Transform>(cam).expect("camera transform");
            // 旧式 transform 是脚本状态的纯函数（抓地主体逐字节保留；
            // 因 target_distance == distance 而滑行是空操作）。
            let expected = compute_camera_transform(&OrbitState {
                heading,
                pitch,
                distance,
                target_distance: distance,
                ..Default::default()
            });

            max_pos_err = max_pos_err.max(got.translation.distance(expected.translation));
            let quat_err = got
                .rotation
                .to_array()
                .iter()
                .zip(expected.rotation.to_array().iter())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            max_quat_err = max_quat_err.max(quat_err);
        }

        assert!(
            max_pos_err < 1e-4,
            "playback pos drift {max_pos_err} render units must be < 1e-4"
        );
        assert!(
            max_quat_err < 1e-4,
            "playback quat drift {max_quat_err} must be < 1e-4"
        );
    }

    /// 端到端：右键竖直拖拽经 [`orbit_camera_system`] 真正改变视角高度。
    ///
    /// 覆盖六件事：①手势确实被接线（右键 + `MouseMotion` → 高度变化）；
    /// ②向上拖 = 放大、向下拖 = 缩小（用户要求的映射）；
    /// ③同一段总位移分多帧送达结果相同（帧率无关，这正是「太快」的修因）；
    /// ④单次连续拖拽的位移被封顶在 `ZOOM_DRAG_MAX_SPAN` 个窗口高度（≈ 9.5 倍），
    /// 松开再拖才能继续 —— 一次拖拽不可能「瞬间」把地球拉到天边；
    /// ⑤高度最终仍被 `[min_distance, max_distance]` 钳制 —— 反复拖拽既不会
    /// 穿地也不会飞出 `max_distance`；⑥拖拽期间 `distance` 逐帧等于
    /// `target_distance`（拖动与光标同步，弛豫只服务滚轮）。窗口缺失
    /// （无头 / 最小 App）时走 `MouseMotion` 原始计数回退路径，逐帧限流
    /// 144 px，而本测试每帧 72 px 低于该门限，因此等价于无限制。`win_h`
    /// 回退 720，与本测试一致。
    #[test]
    fn right_drag_zoom_changes_height_and_respects_limits() {
        /// 构造一个只含轨道相机三个系统的最小 App，相机实体已生成。
        ///
        /// 刻意不注册 `Window`：走 `MouseMotion` 原始计数回退路径（逐帧限流
        /// 144 px），而本测试每帧 72 px 低于门限，因此等价于不受限。
        fn drag_app() -> App {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .add_event::<MouseMotion>()
                .add_event::<MouseWheel>()
                .add_event::<OrbitFlyToRequest>()
                .init_resource::<ButtonInput<MouseButton>>()
                .init_resource::<OrbitInertiaState>()
                .init_resource::<OrbitFlightState>()
                .init_resource::<Arcball>()
                .init_resource::<OrbitTilt>()
                .init_resource::<OrbitZoomDrag>()
                .init_resource::<CameraGesture>()
                .insert_resource(OrbitState::default())
                .add_systems(
                    Update,
                    (
                        orbit_camera_system,
                        orbit_inertia_system,
                        orbit_flight_system,
                    )
                        .chain(),
                );
            app.world_mut().spawn((OrbitCamera, Transform::IDENTITY));
            app
        }

        /// 按下右键、每帧发一次 `dy` 像素的竖直位移，返回最终的 `target_distance`。
        fn drag(dy: f32, frames: usize) -> f32 {
            drag_cycles(dy, frames, 1)
        }

        /// 分 `cycles` 次「按下 → 拖 `frames` 帧 → 松开」的完整拖拽：每次松开都
        /// 重新锚定，因此单次拖拽受 `ZOOM_DRAG_MAX_SPAN` 限制，跨越整个高度区间
        /// 需要多拖几次 —— 与真实交互一致（光标只能在窗口内走一屏高）。
        fn drag_cycles(dy: f32, frames: usize, cycles: usize) -> f32 {
            let mut app = drag_app();
            for _ in 0..cycles {
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .press(ZOOM_DRAG_BUTTON);
                for _ in 0..frames {
                    app.world_mut()
                        .resource_mut::<Events<MouseMotion>>()
                        .send(MouseMotion {
                            delta: Vec2::new(0.0, dy),
                        });
                    app.update();
                    // 拖拽路径不经过弛豫：每一帧都必须与光标同步，否则拖动会
                    // 读作「响应不正确」（滚轮路径仍保留弛豫）。
                    let s = app.world().resource::<OrbitState>();
                    assert!(
                        (s.distance - s.target_distance).abs() < 1e-6,
                        "拖拽期间 distance({}) 必须等于 target({})",
                        s.distance,
                        s.target_distance
                    );
                }
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .release(ZOOM_DRAG_BUTTON);
                app.update();
            }
            app.world().resource::<OrbitState>().target_distance
        }

        // 一帧向上 72 px：span = -72/720 = -0.1 个窗高，高于地表 2.0 →
        // 2.0·e^(-0.1·ZOOM_DRAG_RANGE) ≈ 1.7224。
        let one_frame_up = drag(-72.0, 1);
        let expected_up = 1.0 + 2.0 * (-0.1 * ZOOM_DRAG_RANGE).exp();
        assert!(
            (one_frame_up - expected_up).abs() < 1e-5,
            "向上拖 72 px 应放大到 {expected_up}，得到 {one_frame_up}"
        );

        // 同一段总位移分两帧送达 → 结果逐位接近（帧率无关）。
        let split_up = drag(-36.0, 2);
        assert!(
            (split_up - one_frame_up).abs() < 1e-5,
            "分段拖拽 {split_up} 应与一次拖拽 {one_frame_up} 相同"
        );

        // 一帧向下 72 px = 反向拉远，且与向上拖互为倒数。
        let one_frame_down = drag(72.0, 1);
        let expected_down = 1.0 + 2.0 * (0.1 * ZOOM_DRAG_RANGE).exp();
        assert!(
            (one_frame_down - expected_down).abs() < 1e-5,
            "向下拖 72 px 应拉远到 {expected_down}，得到 {one_frame_down}"
        );

        // 单次连续拖拽的累计位移被封顶在 `ZOOM_DRAG_MAX_SPAN` 个窗口高度：
        // 200 帧 × 72 px 远超一屏，但高度只改变 e^2.25 ≈ 9.49 倍 —— 这正是
        // 「一拖地球瞬间变小」的封堵点。
        let one_drag = drag(-72.0, 200);
        let capped = 1.0 + 2.0 * (-ZOOM_DRAG_MAX_SPAN).exp();
        assert!(
            (one_drag - capped).abs() < 1e-4,
            "单次拖拽应封顶在 {capped}，得到 {one_drag}"
        );

        // 松开再拖可以继续：8 次完整拖拽足以横穿整个高度区间，并停在
        // `min_distance` / `max_distance`（既不会穿地也不会飞出）。
        let state = OrbitState::default();
        let floor = drag_cycles(-72.0, 200, 8);
        assert!(
            (floor - state.min_distance).abs() < 1e-6,
            "反复向上拖拽必须停在 min_distance，得到 {floor}"
        );

        // 持续向下拖 → 收敛到 `max_distance`。
        let ceiling = drag_cycles(72.0, 200, 8);
        assert!(
            (ceiling - state.max_distance).abs() < 1e-4,
            "反复向下拖拽必须停在 max_distance，得到 {ceiling}"
        );
    }

    /// 「右键高度拖拽 = 相机手势」在系统层面成立（瓦片管线据此抬高预算）。
    /// 与 `middle_drag_end_to_end_tilts_camera` 的中键断言互补。
    #[test]
    fn right_drag_marks_camera_gesture() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_event::<MouseMotion>()
            .add_event::<MouseWheel>()
            .add_event::<OrbitFlyToRequest>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<OrbitInertiaState>()
            .init_resource::<OrbitFlightState>()
            .init_resource::<Arcball>()
            .init_resource::<OrbitTilt>()
            .init_resource::<OrbitZoomDrag>()
            .init_resource::<CameraGesture>()
            .insert_resource(OrbitState::default())
            .add_systems(
                Update,
                (orbit_camera_system, orbit_inertia_system, orbit_flight_system).chain(),
            );
        app.world_mut().spawn((OrbitCamera, Transform::IDENTITY));

        // 无输入帧 ⇒ 稳定帧预算。
        app.update();
        assert!(
            !app.world().resource::<CameraGesture>().active,
            "无输入帧不应被标记为相机手势"
        );

        // 按住右键并向上拖 ⇒ 手势进行中。
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(ZOOM_DRAG_BUTTON);
        app.world_mut()
            .resource_mut::<Events<MouseMotion>>()
            .send(MouseMotion {
                delta: Vec2::new(0.0, -72.0),
            });
        app.update();
        assert!(
            app.world().resource::<CameraGesture>().active,
            "右键高度拖拽期间必须标记为相机手势（瓦片管线据此抬高预算）"
        );

        // 松开右键 ⇒ 手势结束，预算回落。
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(ZOOM_DRAG_BUTTON);
        app.update();
        assert!(
            !app.world().resource::<CameraGesture>().active,
            "松开右键后必须回落为稳定帧预算"
        );
    }

    /// CesiumJS `tilt3DOnEllipsoid` 临时置 `_rotateFactor = 1.0`、
    /// `_rotateRateRangeAdjustment = 1.0`，因此倾斜速率为
    /// `clamp(rho - 1.0, 1/5000, 1.77)` —— 与高度无关，任何高于约 2.8 m 的
    /// 相机都饱和。
    #[test]
    fn tilt_rotate_rate_matches_cesiumjs_saturation() {
        assert!((tilt_rotate_rate(1.0) - TILT_MIN_ROTATE_RATE).abs() < 1e-12);
        assert!((tilt_rotate_rate(1.5) - 0.5).abs() < 1e-12);
        assert!((tilt_rotate_rate(2.77) - TILT_MAX_ROTATE_RATE).abs() < 1e-12);
        // 400 km 轨道也是 1.77（与自旋路径不同：后者 rate = h/radius）。
        assert!((tilt_rotate_rate(400_000.0) - TILT_MAX_ROTATE_RATE).abs() < 1e-12);
    }

    /// `rotate3D` 的符号与钳制：`phiWindowRatio = (start.x - end.x)/width`、
    /// `thetaWindowRatio = (start.y - end.y)/height` ⇒ 向右拖（`dx > 0`）与
    /// 向下拖（`dy > 0`）都取负；比例按 `maximumMovementRatio` 双侧钳制；
    /// 再乘 `2π` / `π`。
    #[test]
    fn tilt_deltas_follow_cesiumjs_signs_and_caps() {
        let rate = tilt_rotate_rate(3.0); // 饱和值 1.77
        let (phi, theta) = tilt_deltas(3.0, 72.0, -36.0, 1440.0, 720.0);
        // dx > 0 → phi 负；dy < 0（向上拖）→ theta 正。
        assert!((phi - (-rate * 0.05 * std::f64::consts::TAU)).abs() < 1e-12);
        assert!((theta - (rate * 0.05 * std::f64::consts::PI)).abs() < 1e-12);

        // 超出一整个窗口的位移被钳到 `TILT_MAX_MOVEMENT_RATIO`（双侧）。
        let (phi_big, theta_big) = tilt_deltas(3.0, 1.0e9, -1.0e9, 1440.0, 720.0);
        assert!((phi_big - (-rate * TILT_MAX_MOVEMENT_RATIO * std::f64::consts::TAU)).abs() < 1e-12);
        assert!((theta_big - (rate * TILT_MAX_MOVEMENT_RATIO * std::f64::consts::PI)).abs() < 1e-12);
        // 零位移 → 零旋转（按下不动不会漂移）。
        assert_eq!(tilt_deltas(3.0, 0.0, 0.0, 1440.0, 720.0), (0.0, 0.0));
    }

    /// 基座朝向必须与旧式球面路径（`compute_camera_transform` 的
    /// `looking_at`）一致 —— 否则倾斜接管的瞬间视角会跳一下。
    #[test]
    fn base_look_at_rotation_matches_spherical_baseline() {
        for (heading, pitch, distance) in [
            (0.0f32, 0.4f32, 3.0f32),
            (1.2, -0.8, 1.5),
            (-2.5, 1.3, 12.0),
            (0.0, 0.0, 3.0),
        ] {
            let state = OrbitState {
                heading,
                pitch,
                distance,
                target_distance: distance,
                ..OrbitState::default()
            };
            let target = state.target.as_dvec3();
            let base = base_look_at_rotation(orbit_position_f64(&state, target), target);
            let expected = compute_camera_transform(&state).rotation.as_dquat();
            // 四元数与它的反向表示同一个旋转。
            let dot = base.dot(expected).abs();
            assert!(
                (dot - 1.0).abs() < 1e-6,
                "heading={heading} pitch={pitch} distance={distance}: dot={dot}"
            );
        }
    }

    /// `rotateVertical` 的三个分支：非平行时绕 `up × p` 并夹在枢轴正上方 /
    /// 正下方边界内；恰好平行时只有「把相机带离奇点」的一侧生效；其余不转。
    #[test]
    fn tilt_vertical_turn_handles_branches() {
        let up = DVec3::Z;
        // 非平行：绕 up × p。
        let p = DVec3::new(1.0, 0.0, 1.0).normalize();
        let (axis, _) = tilt_vertical_turn(up, p, DVec3::X, 0.1).expect("non-parallel");
        // 轴按 CesiumJS 原样未归一化（`Quaternion.fromAxisAngle` 内部归一化），
        // 因此这里比较前先归一化。
        let axis = axis.normalize();
        let expected_axis = (up.cross(p)).normalize();
        assert!(axis.dot(expected_axis) > 0.999);
        // 角度被夹在「不许越过枢轴正上方」的边界内：`to_axis = π/4`。
        let (_, turn) = tilt_vertical_turn(up, p, DVec3::X, -1.0e9).expect("non-parallel");
        assert!(turn.abs() < std::f64::consts::FRAC_PI_4 + 1e-3);

        // 正上方（p == +up）：`a = -delta_theta`，只有 `north_parallel && a < 0`
        // 生效，即 `delta_theta > 0` —— 光标**上移**才能抬升视线，与 CesiumJS
        // `ScreenSpaceCameraControllerSpec.js:1595 "does not tilt in the wrong
        // direction"`（相机正对地心时向下拖不动）一致。
        let (axis, turn) = tilt_vertical_turn(up, up, DVec3::X, 0.1).expect("north parallel");
        assert_eq!(axis, DVec3::X);
        assert!((turn - 0.1).abs() < 1e-12);
        assert!(tilt_vertical_turn(up, up, DVec3::X, -0.1).is_none());
        // 正下方：符号相反。
        assert!(tilt_vertical_turn(up, -up, DVec3::X, 0.1).is_none());
        let (axis, turn) = tilt_vertical_turn(up, -up, DVec3::X, -0.1).expect("south parallel");
        assert_eq!(axis, DVec3::X);
        assert!((turn + 0.1).abs() < 1e-12);
    }

    /// 端到端：中键按下拖拽经 [`apply_tilt_drag`] 真正倾斜相机 —— 绕枢轴
    /// 旋转（到枢轴的距离守恒），相机不再注视地心，`target_distance` 与
    /// `distance` 同步以免缩放弛豫把视角拉回倾斜前的高度。
    #[test]
    fn middle_drag_tilts_about_pivot_and_keeps_distance() {
        let mut state = OrbitState::default();
        let mut arcball = Arcball::default();
        let mut tilt = OrbitTilt::default();
        let target = state.target.as_dvec3();
        let position0 = orbit_position_f64(&state, target);

        // 新按下（无位移）：只捕获枢轴 = 相机脚下的天底点。
        apply_tilt_drag(
            &mut state,
            &mut arcball,
            &mut tilt,
            true,
            (0.0, 0.0),
            (1440.0, 720.0),
            false,
        );
        assert!(!tilt.engaged, "按下不动不应倾斜");
        let pivot = tilt.pivot;
        // 天底点必须落在椭球面上：(x²+y²)/A² + z²/B² = 1。
        let on_ellipsoid = (pivot.x * pivot.x + pivot.y * pivot.y) / (ELLIPSOID_A * ELLIPSOID_A)
            + (pivot.z * pivot.z) / (ELLIPSOID_B * ELLIPSOID_B);
        assert!(
            (on_ellipsoid - 1.0).abs() < 1e-6,
            "天底点必须落在椭球面上，得到 {on_ellipsoid}"
        );
        assert!(
            pivot.dot(position0.normalize()) > 0.99,
            "天底点必须与相机同侧（近端交点）"
        );

        // CesiumJS 规范 `does not tilt in the wrong direction`：相机正对地心
        // （p 与枢轴法线平行）时，向**下**拖不产生任何倾斜。
        let mut down_state = OrbitState::default();
        let mut down_arcball = Arcball::default();
        let mut down_tilt = OrbitTilt::default();
        apply_tilt_drag(
            &mut down_state,
            &mut down_arcball,
            &mut down_tilt,
            true,
            (0.0, 0.0),
            (1440.0, 720.0),
            false,
        );
        let before = down_state.pitch;
        apply_tilt_drag(
            &mut down_state,
            &mut down_arcball,
            &mut down_tilt,
            true,
            (0.0, 72.0),
            (1440.0, 720.0),
            false,
        );
        assert!(
            (down_state.pitch - before).abs() < 1e-9,
            "正对地心时向下拖不应倾斜（CesiumJS 规范）"
        );
        assert!(!down_tilt.engaged);

        // 向上拖 72 px（ratio 恰好 0.1）—— CesiumJS 规范 `tilts in 3D` 正是
        // 从画布中心拖到 1/4 高度（向上）。
        apply_tilt_drag(
            &mut state,
            &mut arcball,
            &mut tilt,
            true,
            (0.0, -72.0),
            (1440.0, 720.0),
            false,
        );
        assert!(tilt.engaged);
        assert!(arcball.engaged);
        let position1 = orbit_position_f64(&state, target);
        assert!(
            (state.pitch - 0.4).abs() > 1e-3,
            "倾斜必须改变 pitch，得到 {}",
            state.pitch
        );
        assert!(
            ((position1 - pivot).length() - (position0 - pivot).length()).abs() < 1e-3,
            "绕枢轴旋转 ⇒ 到枢轴距离守恒"
        );
        assert!(
            (state.distance - state.target_distance).abs() < 1e-6,
            "倾斜后必须同步 target_distance"
        );
        // 相机不再注视地心：局部 +Z（背向）与 (position - target) 出现夹角。
        let rotation = arcball.orientation * tilt.aim;
        let back = rotation * DVec3::Z;
        let to_centre = (position1 - target).normalize();
        assert!(
            back.dot(to_centre) < 1.0 - 1e-6,
            "倾斜后视线不再穿过地心：dot = {}",
            back.dot(to_centre)
        );
        assert!((rotation.length() - 1.0).abs() < 1e-9);
    }

    /// 连续向上拖（多帧）时相机只会平缓下降，最终停在地板上 —— 不会像「固定
    /// 枢轴」那样绕着一个点俯冲进地球内部，也不会因为整帧丢弃而卡在半空推不动。
    /// 这是枢轴必须每帧重拾、且越界帧必须「收缩角度」而不是「丢弃」的回归测试。
    #[test]
    fn repeated_upward_drag_never_dives_into_the_globe() {
        let mut state = OrbitState::default();
        let mut arcball = Arcball::default();
        let mut tilt = OrbitTilt::default();
        let target = state.target.as_dvec3();
        let floor = f64::from(GLOBE_RADIUS)
            + f64::from(state.min_distance - GLOBE_RADIUS) * TILT_MIN_HEIGHT_RATIO;
        for frame in 0..600 {
            apply_tilt_drag(
                &mut state,
                &mut arcball,
                &mut tilt,
                true,
                (0.0, -3.0),
                (1440.0, 720.0),
                false,
            );
            let distance = (orbit_position_f64(&state, target) - target).length();
            assert!(
                distance >= floor - 1e-6,
                "第 {frame} 帧相机降到地板以下：{distance} < {floor}"
            );
            assert!(
                distance > f64::from(GLOBE_RADIUS),
                "第 {frame} 帧相机钻进地球：{distance}"
            );
        }
        // 持续向上拖最终应把相机带到地板上（而不是僵在半空），且不越过地板。
        let distance = (orbit_position_f64(&state, target) - target).length();
        assert!(
            distance <= floor + 1e-5,
            "持续向上拖应把相机带到地板，得到 {distance}（地板 {floor}）"
        );

        // 到了地板之后仍要能反方向推回来（没有永久卡死）。
        let pitch_at_floor = state.pitch;
        apply_tilt_drag(
            &mut state,
            &mut arcball,
            &mut tilt,
            true,
            (0.0, 72.0),
            (1440.0, 720.0),
            false,
        );
        assert!(
            (state.pitch - pitch_at_floor).abs() > 1e-4,
            "到达地板后向下拖必须能把相机推回去"
        );
    }

    /// 低空守卫（CesiumJS `tilt3DOnEllipsoid` 首段）：贴着地面时禁止「向上拖」
    /// 继续加大倾角 —— 否则相机会被转到地表以下。
    #[test]
    fn middle_drag_low_altitude_guard_blocks_upward_drag() {
        /// 在给定高度上做一次「按下不动 + 向上拖 72 px」手势，返回是否倾斜。
        fn tilt_up(distance: f32) -> bool {
            let mut state = OrbitState {
                distance,
                target_distance: distance,
                ..OrbitState::default()
            };
            let mut arcball = Arcball::default();
            let mut tilt = OrbitTilt::default();
            apply_tilt_drag(
                &mut state,
                &mut arcball,
                &mut tilt,
                true,
                (0.0, 0.0),
                (1440.0, 720.0),
                false,
            );
            apply_tilt_drag(
                &mut state,
                &mut arcball,
                &mut tilt,
                true,
                (0.0, -72.0),
                (1440.0, 720.0),
                false,
            );
            tilt.engaged
        }

        // 高空：向上拖生效（证明这条路径本身可用）。
        assert!(tilt_up(3.0), "高空向上拖必须倾斜");
        // 低空（高度 ≈ 12.7 m < `min_height` = 25 m）：守卫拦住向上拖。
        assert!(!tilt_up(1.000002), "低空向上拖必须被守卫拦住");
    }

    /// 无中键输入时倾斜路径完全不触碰位姿（`tilt.engaged` 保持 false ⇒
    /// 系统也绝不修改 transform 朝向 ⇒ 确定性捕获路径逐字节不变）。
    #[test]
    fn tilt_is_inert_without_middle_button() {
        let mut state = OrbitState::default();
        let mut arcball = Arcball::default();
        let mut tilt = OrbitTilt::default();
        let before = (
            state.heading,
            state.pitch,
            state.distance,
            state.target_distance,
        );
        apply_tilt_drag(
            &mut state,
            &mut arcball,
            &mut tilt,
            false,
            (120.0, -80.0),
            (1440.0, 720.0),
            false,
        );
        assert_eq!(
            (
                state.heading,
                state.pitch,
                state.distance,
                state.target_distance
            ),
            before
        );
        assert!(!tilt.engaged);
        assert!(!arcball.engaged);
    }

    /// 端到端：中键按下 + `MouseMotion` 经 [`orbit_camera_system`] 真正倾斜
    /// 相机（覆盖「手势已接线」这一点，而不只是纯函数）。
    #[test]
    fn middle_drag_end_to_end_tilts_camera() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_event::<MouseMotion>()
            .add_event::<MouseWheel>()
            .add_event::<OrbitFlyToRequest>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<OrbitInertiaState>()
            .init_resource::<OrbitFlightState>()
            .init_resource::<Arcball>()
            .init_resource::<OrbitTilt>()
            .init_resource::<OrbitZoomDrag>()
            .init_resource::<CameraGesture>()
            .insert_resource(OrbitState::default())
            .add_systems(
                Update,
                (orbit_camera_system, orbit_inertia_system, orbit_flight_system).chain(),
            );
        let camera = app.world_mut().spawn((OrbitCamera, Transform::IDENTITY)).id();

        // 一帧无输入 ⇒ 写入纯球面基座位姿。
        app.update();
        assert!(
            !app.world().resource::<CameraGesture>().active,
            "无输入帧不应被标记为相机手势（预算须回落）"
        );
        let before = *app.world().entity(camera).get::<Transform>().unwrap();
        let state = OrbitState::default();
        let target = state.target.as_dvec3();
        let pivot = ellipsoid_point_along(orbit_position_f64(&state, target) - target, target)
            .expect("nadir point");

        // 按住中键，向上拖 72 px（CesiumJS 规范 `tilts in 3D` 的方向）：
        // 一帧 = ratio 0.1 × rate 1.77 × π。
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(TILT_DRAG_BUTTON);
        app.world_mut()
            .resource_mut::<Events<MouseMotion>>()
            .send(MouseMotion {
                delta: Vec2::new(0.0, -72.0),
            });
        app.update();

        let after = *app.world().entity(camera).get::<Transform>().unwrap();
        assert!(
            app.world().resource::<CameraGesture>().active,
            "中键拖拽期间必须标记为相机手势（瓦片管线据此抬高预算）"
        );
        assert!(
            app.world().resource::<OrbitTilt>().engaged,
            "中键拖拽必须接管倾斜"
        );
        let q_before = before.rotation.as_dquat();
        let q_after = after.rotation.as_dquat();
        assert!(q_before.dot(q_after).abs() < 1.0 - 1e-4, "朝向必须真的变了");
        // 绕枢轴旋转：到枢轴的距离守恒（f32 位姿重建的容差）。
        let d_before = (before.translation.as_dvec3() - pivot).length();
        let d_after = (after.translation.as_dvec3() - pivot).length();
        assert!(
            (d_before - d_after).abs() < 1e-3,
            "到枢轴的距离必须守恒：{d_before} vs {d_after}"
        );
        // 相机不再注视地心：局部 +Z 与 (position - target) 有夹角。
        let back = after.rotation.as_dquat() * DVec3::Z;
        let to_centre = (after.translation.as_dvec3() - target).normalize();
        assert!(back.dot(to_centre) < 1.0 - 1e-5, "倾斜后视线不应再穿过地心");
    }
}
