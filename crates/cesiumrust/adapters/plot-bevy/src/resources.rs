//! 应用与标绘叠加层之间共享的桥接资源。
//!
//! 这些是视图侧的状态载体：应用（拥有相机和
//! `MapMode`）每帧写入 [`PlotViewCtx`]，桥接层的视图同步 /
//! 拾取 / 交互系统读取它。从 M2 起桥接层还拥有场景文档
//! （[`PlotDocument`]）、可见性开关（[`PlotFilters`]）
//! 以及元素 → 实体协调注册表（[`PlotVisuals`]）。
//!
//! 投影模式的权威定义在核心层的 [`cesium_plot::model::ViewMode`]；
//! 桥接层通过别名引入（而非导入应用的 `MapMode`），因此
//! 适配器绝不依赖应用层（DDD）。

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use cesium_plot::model::ids::ElementId;
use cesium_plot::model::{Document, Filters, PickHit, ViewMode};
use cesium_plot::ops::{HistoryStack, SnapConfig};

/// 叠加层渲染所对的投影模式——整个工作区的唯一
/// 定义，从纯核心层别名引入（参见 [`cesium_plot::model::ViewMode`]）。
pub type PlotViewMode = ViewMode;

/// 每帧传给叠加层的视图上下文：当前激活的投影模式以及
/// 屏幕 ↔ 世界转换所需的度量。由应用的
/// `sync_plot_view_ctx` 系统写入；由桥接层的重投影 / 拾取读取。
#[derive(Resource, Clone, Copy, Debug)]
pub struct PlotViewCtx {
    /// 当前激活的投影模式。
    pub mode: PlotViewMode,
    /// 平面地图缩放（每世界单位像素数）；在 2D 相机启动前为 `0.0`。
    pub flat_zoom: f32,
    /// 主窗口宽度（逻辑像素）。
    pub screen_w: f32,
    /// 主窗口高度（逻辑像素）。
    pub screen_h: f32,
}

impl Default for PlotViewCtx {
    /// 默认视图上下文：当前模式、缩放与各屏幕维度均从 0/默认起步。
    fn default() -> Self {
        Self {
            mode: PlotViewMode::default(),
            flat_zoom: 0.0,
            screen_w: 0.0,
            screen_h: 0.0,
        }
    }
}

/// 输入捕获门控：为 `true` 时标绘工具拥有指针，因此
/// 3D 轨道和 2D 平移/缩放相机系统必须在该帧让位，以避免
/// 相机与编辑操作冲突。
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlotInputCapture(pub bool);

impl PlotInputCapture {
    /// 指针输入当前是否被标绘叠加层捕获。
    #[inline]
    pub fn is_captured(&self) -> bool {
        self.0
    }
}

/// 叠加层所呈现的场景文档，加上一个变更计数器供同步
/// 系统读取以决定是否重新协调。结构编辑通过
/// [`PlotDocument::mark_dirty`] 进行，因此 revision 始终随内容前进。
#[derive(Resource)]
pub struct PlotDocument {
    /// 不依赖框架的文档。
    pub doc: Document,
    /// 单调递增的 revision，每次内容变更时自增（`mark_dirty`）。
    pub revision: u64,
    /// 文档自上次完整协调以来发生变化时置位；
    /// 由同步系统追赶后清除。
    pub dirty: bool,
}

impl Default for PlotDocument {
    /// 默认创建一个空文档且 `dirty=true`，以便首帧就触发一次完整协调。
    fn default() -> Self {
        Self {
            doc: Document::default(),
            revision: 0,
            dirty: true,
        }
    }
}

impl PlotDocument {
    /// 记录一次内容变更：推进 revision 并标记为待协调。
    pub fn mark_dirty(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
    }
}

/// 叠加层的可见性开关（计划 §10），包装后桥接层可以
/// `Deref`/`DerefMut` 到 [`Filters`]。默认总开关为 ON
/// 且无其他限制——空文档时什么都不绘制，
/// 因此窗口基线不受影响。
#[derive(Resource)]
pub struct PlotFilters(pub Filters);

impl Default for PlotFilters {
    /// 默认总开关为启用（空文档下什么都不绘制，不影响窗口基线）。
    fn default() -> Self {
        Self(Filters::enabled())
    }
}

impl std::ops::Deref for PlotFilters {
    type Target = Filters;
    /// 只读透传到内部 [`Filters`]。
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for PlotFilters {
    /// 可变透传到内部 [`Filters`]。
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// 每元素的视图侧簿记：当前哪些 ECS 实体在渲染它
/// （网格 和/或 标签），以便下次协调时能更新或销毁
/// 而不会泄漏。
#[derive(Debug, Clone, Default)]
pub struct VisualEntry {
    /// 承载元素网格的实体（点 / 图标 / 多段线 ribbon）。
    pub mesh: Option<Entity>,
    /// 网格资产句柄，保留以便就地更新 ribbon 几何。
    pub mesh_handle: Option<Handle<Mesh>>,
    /// 元素的材质，保留以便就地应用颜色 / 不透明度编辑。
    pub mat: Option<Handle<bevy::pbr::StandardMaterial>>,
    /// 承载填充面的实体（多边形 / 矩形 / 圆 / 椭圆）。
    pub fill: Option<Entity>,
    /// 面的剖分网格句柄（每次内容变更重建）。
    pub fill_handle: Option<Handle<Mesh>>,
    /// 面填充材质。
    pub fill_mat: Option<Handle<bevy::pbr::StandardMaterial>>,
    /// 承载面屏幕恒定宽轮廓描边的实体。
    pub outline: Option<Entity>,
    /// 轮廓 ribbon 网格句柄（每帧重写）。
    pub outline_handle: Option<Handle<Mesh>>,
    /// 轮廓材质。
    pub outline_mat: Option<Handle<bevy::pbr::StandardMaterial>>,
    /// 承载元素标签文本节点的实体。
    pub label: Option<Entity>,
}

/// 将每个元素映射到其活跃视觉实体的注册表。由同步
/// 系统拥有；每次协调时查阅。
#[derive(Resource, Default)]
pub struct PlotVisuals {
    /// 元素 → 其视觉条目。
    pub entries: HashMap<ElementId, VisualEntry>,
    /// 点 / 图标 billboard 的共享单位 quad 网格句柄，懒创建。
    pub quad: Option<Handle<Mesh>>,
}

/// 标记在渲染单个元素的网格实体上。
#[derive(Component, Clone, Copy, Debug)]
pub struct PlotVisual {
    /// 该实体绘制的元素。
    pub element: ElementId,
}

/// 标记在渲染单个元素标签的 UI 文本实体上。
#[derive(Component, Clone, Copy, Debug)]
pub struct PlotLabel {
    /// 该标签所属的元素。
    pub element: ElementId,
}

/// 当前选择集（计划 §8）。跨层；拾取系统在单击时替换它，
/// 交互 FSM 后续扩展（ctrl / 框选）。
#[derive(Resource, Default, Clone, PartialEq, Eq)]
pub struct PlotSelection(pub HashSet<ElementId>);

impl PlotSelection {
    /// `id` 当前是否被选中。
    #[inline]
    pub fn contains(&self, id: ElementId) -> bool {
        self.0.contains(&id)
    }
    /// 用单个元素替换选择（未变则返回 false）。
    pub fn select_one(&mut self, id: ElementId) -> bool {
        if self.0.len() == 1 && self.0.contains(&id) {
            return false;
        }
        self.0.clear();
        self.0.insert(id);
        true
    }
    /// 清除选择（已为空则返回 false）。
    pub fn clear(&mut self) -> bool {
        if self.0.is_empty() {
            return false;
        }
        self.0.clear();
        true
    }
}

/// 光标当前所指的元素（若有），由拾取系统每帧解析。
/// 同步系统读取它以产生悬停反馈，交互 FSM（M5）读取它以进行
/// 拖拽 / 编辑决策。
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct PlotHover(pub Option<PickHit>);

/// 撤销 / 重做栈（计划 §8 / §14，M6）。每次结构编辑——绘制提交、
/// 删除 / 复制、移动 / 顶点 / 旋转 / 缩放——都作用到 [`PlotDocument`]
/// 并在此记录为单步，因此纯 [`HistoryStack`] 契约（在核心层
/// headless 验证）可以原封不动地驱动活跃叠加层。
#[derive(Resource, Default)]
pub struct PlotHistory(pub HistoryStack);

/// 绘制 FSM 的坐标吸附可调参数（计划 §16 M9 “吸附”），包装核心层的纯
/// [`SnapConfig`]。`Deref`/`DerefMut` 直通。默认
/// **禁用**，因此将其接入 `interaction_system` 绝不会改变现有绘制行为，
/// 直到用户（或应用）开启它。
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct PlotSnap(pub SnapConfig);

impl std::ops::Deref for PlotSnap {
    type Target = SnapConfig;
    /// 只读透传到内部 [`SnapConfig`]。
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for PlotSnap {
    /// 可变透传到内部 [`SnapConfig`]。
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
