//! 拾取子系统（计划 §7）：将屏幕光标转换为其下方最佳
//! [`PickHit`]，驱动悬停反馈和点击选择，并发出交互 FSM 和 UI
//! 消费的悬停 / 选择事件。
//!
//! 逻辑拆分为两部分以兼顾正确性和可测性：
//!  * [`pick_at`] 是一个**纯查询**——给定文档、当前可拾取
//!    id 集和*任意*地理 → 屏幕投影器，它投影每个候选
//!    几何，运行 [`cesium_plot::geom::hit`] 图元并通过 [`pick_best`] 折叠
//!    结果。它无 Bevy 依赖，因此整个命中 / 排名 / 选择契约可以 headless
//!    单测（计划 §15）。
//!  * [`pick_system`] 是薄 ECS 壳：采集激活相机的
//!    度量，用 [`crate::labels::world_to_screen`] 构建投影器——
//!    渲染器放置网格和标签时使用的*同一*调用，因此“你拾取什么”
//!    在构造上就是“你看到什么”——然后应用
//!    悬停 / 点击状态变更。

use std::collections::BTreeSet;

use bevy::input::mouse::MouseButton;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use cesium_plot::geo::{GeoBounds, GeoPoint};
use cesium_plot::geom::hit;
use cesium_plot::model::geometry::Geometry;
use cesium_plot::model::ids::ElementId;
use cesium_plot::model::{
    pick_best, Part, PickHit, Style, ViewContext, ViewMode, Document, RANK_POLY_BODY,
    RANK_POLY_EDGE,
};
use cesium_plot::visibility::eval_visibility;

use crate::labels::world_to_screen;
use crate::reproject::ViewMetrics;
use crate::resources::{PlotDocument, PlotFilters, PlotHover, PlotSelection, PlotViewCtx};
use crate::shapes;

/// 层的 `order` 在合成绘制 / 拾取优先级中的权重（层
/// 胜过其下任何层内 `z_order` 差异）。
const LAYER_WEIGHT: i32 = 100_000;

/// 一个廉价的等价键，捕获决定 [`pick_at`] 每帧结果的所有输入。连续两帧
/// 共享同一键则产生相同的悬停，因此跳过昂贵的逐元素投影 / 剖分并
/// 复用缓存的 [`PickHit`]。它故意包含解析后的
/// `pickable` 集（`Filters` 编辑唯一的浮现方式，因为 `Filters` 无
/// revision 计数器）以及投影器闭包捕获的相机位姿 / 投影。
#[derive(Clone, PartialEq)]
pub struct PickKey {
    /// 光标位置（逻辑像素，double 精度以避免拖拽微抖动误判）。
    cursor: [f64; 2],
    /// 文档 revision，内容变化时使缓存键失效。
    revision: u64,
    /// 当前是否为 3D 地球（globe）模式。
    globe: bool,
    /// 2D 平面缩放（每世界单位像素数）。
    flat_zoom: f32,
    /// 主窗口宽高（逻辑像素）。
    screen: [f32; 2],
    /// 相机平移（局部坐标，3D 投影器闭包捕获）。
    cam_t: [f32; 3],
    /// 相机旋转四元数（投影器闭包捕获）。
    cam_r: [f32; 4],
    /// 相机焦距（像素），决定透视拾取的投影尺度。
    focal_px: f64,
    /// 是否启用透视投影（区别于正交）。
    persp: bool,
    /// 解析后的可拾取元素集（`Filters` 编辑的唯一体现途径）。
    pickable: Vec<ElementId>,
}

/// [`pick_system`] 的缓存：最后一次悬停结果及其计算时的 [`PickKey`]。
/// 仅窗口模式存在（随桥接插件存活），因此 headless
/// 黄金基线不受影响。
#[derive(Resource, Default)]
pub struct PlotPickCache {
    /// 上一次计算所用的 [`PickKey`]（为 `None` 表示尚无缓存）。
    key: Option<PickKey>,
    /// 与 `key` 对应的缓存悬停结果。
    best: Option<PickHit>,
}

/// 点击 / 拖拽甄别：记录左键按下时的光标屏幕位置，以便在松手时
/// 判断这是一次“点击”（位移小于阈值，应改变选择）还是一次“拖拽相机
/// 的起手”（位移超过阈值，不应改变选择）。没有它，拖动地球时按下的
/// 那一下会被 `pick_system` 当成点空白→误清选择（画完点后一拖就变蓝）。
#[derive(Resource, Default)]
pub struct PlotClickGuard {
    /// 本序列左键按下时的光标位置（逻辑像素）；`None` 表示未按下。
    press: Option<[f64; 2]>,
}

/// 按下到松手的光标位移在此阈值内（逻辑像素）视为一次点击；
/// 超过则视为拖拽（与 [`crate::edit`] 的 `DRAG_THRESHOLD_PX` 保持同一手感）。
const CLICK_DRAG_THRESHOLD_PX: f64 = 4.0;

/// 悬停元素变化时发出（计划 §14）。
#[derive(Event, Clone, Copy, Debug, PartialEq)]
pub struct PlotHoverChanged {
    /// 光标下的新命中（若有）。
    pub hit: Option<PickHit>,
}

/// 选择集变化时发出（计划 §14）。
#[derive(Event, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlotSelectionChanged {
    /// 新选择的尺寸。
    pub selected: u32,
}

/// 右键点击时发出（计划 §16 M9 “右键菜单”）：UI / 应用拥有
/// 实际菜单，这里只报告请求位置及其下方（若有）内容，
/// 以便菜单可提供上下文动作。其他一切不变——桥接层只提出意图。
#[derive(Event, Clone, Copy, Debug, PartialEq)]
pub struct PlotContextMenu {
    /// 光标下的元素，若右键命中了一个。
    pub target: Option<ElementId>,
    /// 屏幕位置（逻辑像素）用于锚定菜单。
    pub screen: Vec2,
}

/// 通过激活相机将地理坐标投影为屏幕像素，投影失败时（在相机后方、
/// 超出视口等）返回 `None`。
type Projector<'a> = dyn Fn(GeoPoint) -> Option<[f64; 2]> + 'a;

/// 将单个几何与 `cursor` 碰撞，返回 `(part, screen_dist, rank)`。
/// 只测试 M2/M3 可绘制类型；面 / 圆锥曲线在 M4 采样后获得填充内部命中。
///
/// # 参数
/// - `geo`：待碰撞的几何体（点/图标/标签/线/面等）。
/// - `style`：元素样式，提供命中容差（point_size、width_px 等）。
/// - `mode`：视图模式，决定描边是否做大圆加密。
/// - `project`：地理→屏幕像素的投影器闭包。
/// - `cursor`：待测光标屏幕坐标。
///
/// # 返回
/// 命中时返回 `(部件, 屏幕距离, 优先级 rank)`，未命中返回 `None`。
fn hit_geometry(
    geo: &Geometry,
    style: &Style,
    mode: ViewMode,
    project: &Projector<'_>,
    cursor: [f64; 2],
) -> Option<(Part, f64, u8)> {
    match geo {
        Geometry::Point(p) => {
            let sp = project(*p)?;
            let r = (style.point_size_px as f64 * 0.5).max(hit::DEFAULT_TOL_PX);
            let (part, d) = hit::hit_point(cursor, sp, r)?;
            Some((part, d, cesium_plot::model::RANK_MARKER))
        }
        Geometry::Icon(i) => {
            let sp = project(i.at)?;
            let size = style.icon.map(|x| x.size_px as f64).unwrap_or(32.0);
            let r = (size * 0.5).max(hit::DEFAULT_TOL_PX);
            let (part, d) = hit::hit_point(cursor, sp, r)?;
            Some((part, d, cesium_plot::model::RANK_MARKER))
        }
        Geometry::Label(l) => {
            let sp = project(l.at)?;
            // 宽松文本框代理：锚点周围的固定半径。
            let r = hit::DEFAULT_TOL_PX.max(10.0);
            let (part, d) = hit::hit_point(cursor, sp, r)?;
            Some((part, d, cesium_plot::model::RANK_MARKER))
        }
        Geometry::Polyline(_) | Geometry::Arc(_) | Geometry::Path(_) => {
            // 采样 / 大圆加密后的描边，与渲染的完全一致
            // （通过 [`crate::shapes`]）。要求每个顶点都能投影，
            // 以便 part 索引与几何的真实顶点对齐。
            let pos = shapes::stroke_positions(geo, mode)?;
            let mut pts: Vec<[f64; 2]> = Vec::with_capacity(pos.len());
            for g in &pos {
                pts.push(project(*g)?);
            }
            let tol = style.width_px as f64 * 0.5 + hit::DEFAULT_TOL_PX;
            let (part, d) = hit::hit_polyline(cursor, &pts, tol)?;
            Some((part, d, cesium_plot::model::RANK_LINE))
        }
        Geometry::Polygon(_) | Geometry::Rectangle(_) | Geometry::Circle(_)
        | Geometry::Ellipse(_) => {
            let (outer, holes) = shapes::face_rings(geo, mode)?;
            let mut o: Vec<[f64; 2]> = Vec::with_capacity(outer.len());
            for g in &outer {
                o.push(project(*g)?);
            }
            let mut hs: Vec<Vec<[f64; 2]>> = Vec::with_capacity(holes.len());
            for h in &holes {
                let mut ring = Vec::with_capacity(h.len());
                for g in h {
                    ring.push(project(*g)?);
                }
                hs.push(ring);
            }
            let tol = style.width_px as f64 * 0.5 + hit::DEFAULT_TOL_PX;
            let (part, d) = hit::hit_polygon_multi(cursor, &o, &hs, tol)?;
            let rank = if part == Part::Body {
                RANK_POLY_BODY
            } else {
                RANK_POLY_EDGE
            };
            Some((part, d, rank))
        }
        _ => None, // 点/图标/标签已在上方处理；组合保留给 M9
    }
}

/// 几何的窄相位命中是否足够昂贵（它投影一个采样 / 加密的顶点链）
/// 以至于值得先做粗相位拒绝。点类型只投影单个点，跳过粗相位。
fn is_multi_vertex(geo: &Geometry) -> bool {
    matches!(
        geo,
        Geometry::Polyline(_)
            | Geometry::Polygon(_)
            | Geometry::Rectangle(_)
            | Geometry::Circle(_)
            | Geometry::Ellipse(_)
            | Geometry::Arc(_)
            | Geometry::Path(_)
    )
}

/// 粗相位屏幕测试。将元素的保守地理 `bounds` 以 4×4 网格投影为屏幕 AABB，
/// 仅在光标可证明在其外部（以 `tol` px 填充）时返回 `false`。绝不会
/// 误拒：空包围盒或任何网格采样无法投影（几何在相机外或后方）
/// 都保守地报告为可能命中。bounds 来自
/// [`cesium_plot::model::Geometry::bounds`]，即球体加密后的范围，
/// 因此它也覆盖大圆隆起。
///
/// # 参数
/// - `bounds`：元素的保守地理包围盒。
/// - `project`：地理→屏幕投影器。
/// - `cursor`：待测光标屏幕坐标。
/// - `tol`：屏幕容差（像素），向各方向外扩 AABB。
///
/// # 返回
/// 光标可能命中时返回 `true`（保守），可证明在屏幕外时才返回 `false`。
fn bounds_might_hit(
    bounds: &GeoBounds,
    project: &Projector<'_>,
    cursor: [f64; 2],
    tol: f64,
) -> bool {
    if bounds.is_empty() {
        return true;
    }
    const FRACS: [f64; 4] = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for &fx in &FRACS {
        let lon = bounds.west_deg + (bounds.east_deg - bounds.west_deg) * fx;
        for &fy in &FRACS {
            let lat = bounds.south_deg + (bounds.north_deg - bounds.south_deg) * fy;
            match project(GeoPoint::surface(lon, lat)) {
                Some(sp) => {
                    min[0] = min[0].min(sp[0]);
                    min[1] = min[1].min(sp[1]);
                    max[0] = max[0].max(sp[0]);
                    max[1] = max[1].max(sp[1]);
                }
                // 无法证明元素在屏幕外 → 保留它。
                None => return true,
            }
        }
    }
    cursor[0] >= min[0] - tol
        && cursor[0] <= max[0] + tol
        && cursor[1] >= min[1] - tol
        && cursor[1] <= max[1] + tol
}

/// 纯拾取查询（计划 §14）。在 `pickable` 元素中返回 `cursor` 下的最佳
/// [`PickHit`]，或 `None`。`project` 将地理坐标映射为屏幕像素；
/// 桥接层提供基于相机的版本。
///
/// # 参数
/// - `doc`：场景文档，提供元素几何/样式/包围盒与层级 order。
/// - `pickable`：本帧可拾取的元素集（已应用 `Filters`）。
/// - `cursor`：待测光标屏幕坐标。
/// - `mode`：视图模式（2D/3D）。
/// - `project`：地理→屏幕像素投影器。
///
/// # 返回
/// `pickable` 中 `cursor` 下的最佳 [`PickHit`]（按 rank→z_order→屏幕距离排序），否则 `None`。
pub fn pick_at(
    doc: &Document,
    pickable: &BTreeSet<ElementId>,
    cursor: [f64; 2],
    mode: ViewMode,
    project: &Projector<'_>,
) -> Option<PickHit> {
    let mut candidates = Vec::new();
    for &id in pickable {
        let Some(element) = doc.element(id) else {
            continue;
        };
        let Some((layer_id, _)) = doc.element_context(id) else {
            continue;
        };
        let order = doc
            .layer(layer_id)
            .map(|l| l.order)
            .unwrap_or(0)
            .saturating_mul(LAYER_WEIGHT)
            + element.style.z_order;
        // 粗相位：昂贵的逐顶点投影 / 剖分之前先做廉价的屏幕-AABB 拒绝，
        // 且仅对多顶点类型（一个点已经投影了一个坐标，
        // 粗筛它的开销比节省的更多）。保守的 bounds 保证这绝不会
        // 丢弃真正的命中。
        if is_multi_vertex(&element.geometry) {
            let tol = element.style.width_px as f64 * 0.5 + hit::DEFAULT_TOL_PX + 4.0;
            if !bounds_might_hit(&element.bounds, project, cursor, tol) {
                continue;
            }
        }
        if let Some((part, screen_dist, rank)) =
            hit_geometry(&element.geometry, &element.style, mode, project, cursor)
        {
            candidates.push(PickHit {
                element: id,
                part,
                layer: layer_id,
                z_order: order,
                rank,
                screen_dist,
            });
        }
    }
    pick_best(&candidates)
}

/// 每帧拾取系统（参见模块文档）。
///
/// 从窗口光标与激活相机重建 [`PickKey`]，若与上帧相同则复用
/// 缓存悬停，仅在键变化时才走廉价的 `pick_at` 全量碰撞。随后根据
/// 鼠标按键与交互状态发布悬停/选择/右键菜单事件。
#[allow(clippy::too_many_arguments)]
pub fn pick_system(
    ctx: Res<PlotViewCtx>,
    plot_doc: Res<PlotDocument>,
    filters: Res<PlotFilters>,
    mut cache: ResMut<PlotPickCache>,
    mut click_guard: ResMut<PlotClickGuard>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cams: Query<(&Camera, &GlobalTransform, &Projection)>,
    mouse: Res<ButtonInput<MouseButton>>,
    interaction: Res<crate::interaction::PlotInteraction>,
    mut hover: ResMut<PlotHover>,
    mut selection: ResMut<PlotSelection>,
    mut hover_events: EventWriter<PlotHoverChanged>,
    mut selection_events: EventWriter<PlotSelectionChanged>,
    mut menu_events: EventWriter<PlotContextMenu>,
) {
    // 光标必须在窗口内；离开的光标清除悬停。
    let cursor = match windows.get_single() {
        Ok(w) => w.cursor_position().map(|c| [c.x as f64, c.y as f64]),
        Err(_) => None,
    };
    let Some(cursor) = cursor else {
        if hover.0.is_some() {
            hover.0 = None;
            hover_events.send(PlotHoverChanged { hit: None });
        }
        return;
    };

    // 激活相机（匹配视图模式的投影，否则任选一个激活的）——与
    // [`crate::sync::sync_visuals`] 使用相同的选择规则，因此拾取和渲染
    // 始终对哪个相机定义屏幕达成一致。
    let want_perspective = matches!(ctx.mode, cesium_plot::model::ViewMode::Globe);
    let mut exact: Option<(&Camera, &GlobalTransform, &Projection)> = None;
    let mut fallback: Option<(&Camera, &GlobalTransform, &Projection)> = None;
    for (c, ct, p) in cams.iter() {
        if !c.is_active {
            continue;
        }
        let is_persp = matches!(p, Projection::Perspective(_));
        let matches = if want_perspective { is_persp } else { !is_persp };
        if matches {
            exact = Some((c, ct, p));
            break;
        }
        if fallback.is_none() {
            fallback = Some((c, ct, p));
        }
    }
    let Some((cam, ct, proj)) = exact.or(fallback) else {
        return;
    };
    let focal_px = match proj {
        Projection::Perspective(pp) => {
            let half = (pp.fov * 0.5).tan() as f64;
            if half > 1e-9 {
                (ctx.screen_h as f64 * 0.5) / half
            } else {
                0.0
            }
        }
        Projection::Orthographic(_) => 0.0,
    };
    let metrics = ViewMetrics {
        mode: ctx.mode,
        pixels_per_world: ctx.flat_zoom as f64,
        focal_px,
        cam_pos: ct.translation(),
    };

    // 可拾取集 = 当前筛选下可见 ∧ 可选择（计划 §7）。
    let ppw_rep = match ctx.mode {
        cesium_plot::model::ViewMode::Flat => metrics.pixels_per_world,
        cesium_plot::model::ViewMode::Globe => metrics.pixels_per_world_at(Vec3::ZERO),
    };
    let view = ViewContext {
        mode: ctx.mode,
        pixels_per_world: ppw_rep,
        meters_per_pixel: metrics.meters_per_pixel(),
        screen_w: ctx.screen_w as f64,
        screen_h: ctx.screen_h as f64,
        time_s: 0.0,
    };
    let pickable = eval_visibility(&plot_doc.doc, &view, &filters.0).pickable;

    // 只要喂给 `pick_at` 的每个输入都未变（光标、文档 revision、相机位姿 / 投影、视口、
    // 以及解析后的 pickable 集）就复用上次悬停。这是大场景的
    // 重要常量因子优化：静止光标在移动相机上仍会重新拾取，但
    // 空闲帧跳过整个逐元素投影 / 剖分。
    let key = PickKey {
        cursor,
        revision: plot_doc.revision,
        globe: matches!(ctx.mode, cesium_plot::model::ViewMode::Globe),
        flat_zoom: ctx.flat_zoom,
        screen: [ctx.screen_w, ctx.screen_h],
        cam_t: ct.translation().to_array(),
        cam_r: ct.rotation().to_array(),
        focal_px,
        persp: matches!(proj, Projection::Perspective(_)),
        pickable: pickable.iter().copied().collect(),
    };
    let best = if cache.key.as_ref() == Some(&key) {
        cache.best
    } else {
        let project = |g: GeoPoint| -> Option<[f64; 2]> {
            world_to_screen(cam, ct, metrics.project(g)).map(|v| [v.x as f64, v.y as f64])
        };
        let b = pick_at(&plot_doc.doc, &pickable, cursor, ctx.mode, &project);
        cache.key = Some(key);
        cache.best = b;
        b
    };

    if best != hover.0 {
        hover.0 = best;
        hover_events.send(PlotHoverChanged { hit: best });
    }

    // 普通左键点击用命中元素替换选择集，或在点击空白时清除
    // 它（ctrl / 框选多选在 M5 落地）。绘制工具激活时
    // 点击属于 FSM（添加顶点），因此拾取器完全让位以避免重复处理同一按下。
    if interaction.is_drawing() {
        return;
    }
    // 选择变更延到松手，并只在“按下→松手位移小于阈值”时提交，从而把
    // 拖拽相机的起手与真正的点击区分开（否则画完点后一拖地球就误清选择）。
    if mouse.just_pressed(MouseButton::Left) {
        click_guard.press = Some(cursor);
    }
    if mouse.just_released(MouseButton::Left) {
        if let Some(start) = click_guard.press.take() {
            let moved =
                ((cursor[0] - start[0]).powi(2) + (cursor[1] - start[1]).powi(2)).sqrt();
            if moved < CLICK_DRAG_THRESHOLD_PX {
                let changed = match best {
                    Some(hit) => selection.select_one(hit.element),
                    None => selection.clear(),
                };
                if changed {
                    selection_events.send(PlotSelectionChanged {
                        selected: selection.0.len() as u32,
                    });
                }
            }
        }
    }

    // 右键点击提出上下文菜单意图（M9）：报告命中（若有）
    // 和锚定位置；应用决定显示什么菜单（若有）。
    if mouse.just_pressed(MouseButton::Right) {
        menu_events.send(PlotContextMenu {
            target: best.map(|h| h.element),
            screen: Vec2::new(cursor[0] as f32, cursor[1] as f32),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_plot::geo::GeoPoint;
    use cesium_plot::model::geometry::{LabelGeometry, Polyline, Rectangle};
    use cesium_plot::model::ids::ElementId;
    use cesium_plot::model::{Document, Geometry, LabelAnchor};

    /// 一个将经纬度直接映射为像素（× 4）的投影器，以便测试
    /// 无需相机即可推理屏幕位置。
    fn quad_projector(g: GeoPoint) -> Option<[f64; 2]> {
        Some([g.lon_deg * 4.0, g.lat_deg * 4.0])
    }

    /// 构造一个含两个点元素（a 在原点、b 在 (50,50)）的单图层文档，
    /// 返回文档与两个元素 id，供拾取命中/排序测试复用。
    fn point_doc() -> (Document, ElementId, ElementId) {
        let mut doc = Document::default();
        let layer = doc.new_layer("L");
        let a = doc.make_element("a", Geometry::Point(GeoPoint::surface(0.0, 0.0)));
        let b = doc.make_element("b", Geometry::Point(GeoPoint::surface(50.0, 50.0)));
        let ea = a.id;
        let eb = b.id;
        doc.add_element_to_layer(layer, a);
        doc.add_element_to_layer(layer, b);
        (doc, ea, eb)
    }

    /// 光标远离所有已投影元素时应返回 `None`（无任何候选命中）。
    #[test]
    fn pick_at_misses_when_cursor_is_off_every_element() {
        let (doc, _, _) = point_doc();
        let pickable: BTreeSet<ElementId> = doc.elements().map(|e| e.id).collect();
        // 远离两个投影点。
        assert!(pick_at(&doc, &pickable, [500.0, 500.0], ViewMode::Flat, &quad_projector).is_none());
    }

    #[test]
    fn globe_pick_follows_the_great_circle_not_the_flat_chord() {
        // M9 拾取双模式证明： 60°N 上的东西长线段的大圆向极地隆起。
        // 隆起上的一个点在 Globe 模式下可拾取（描边沿球体加密）
        // 但在 Flat 模式下未命中（直线 2D 弦从其南方远处经过）。这测试了
        // 与渲染器共享的模式感知采样。
        let mut doc = Document::default();
        let layer = doc.new_layer("L");
        let line = doc.make_element(
            "route",
            Geometry::Polyline(Polyline {
                positions: vec![GeoPoint::surface(0.0, 60.0), GeoPoint::surface(60.0, 60.0)],
            }),
        );
        let id = line.id;
        doc.add_element_to_layer(layer, line);
        let pickable: BTreeSet<ElementId> = [id].into_iter().collect();

        // 极地最远的 Globe 加密采样（大圆顶点）。
        let globe_pts =
            shapes::stroke_positions(&doc.element(id).unwrap().geometry, ViewMode::Globe).unwrap();
        let bulge = globe_pts
            .iter()
            .fold(globe_pts[0], |a, &b| if b.lat_deg > a.lat_deg { b } else { a });
        assert!(bulge.lat_deg > 61.5, "expected a clear poleward bulge, got {bulge:?}");
        let cursor = [bulge.lon_deg * 4.0, bulge.lat_deg * 4.0];

        assert!(
            pick_at(&doc, &pickable, cursor, ViewMode::Globe, &quad_projector).is_some(),
            "Globe pick must hit the great-circle sample"
        );
        assert!(
            pick_at(&doc, &pickable, cursor, ViewMode::Flat, &quad_projector).is_none(),
            "Flat chord must miss the bulge"
        );
    }

    /// 光标精确落在点 a 的投影像素上时，应命中 a 且 rank 为标记级。
    /// 验证最近点选择与优先级标签均与几何类型一致。
    #[test]
    fn pick_at_hits_the_nearest_point() {
        let (doc, ea, _eb) = point_doc();
        let pickable: BTreeSet<ElementId> = doc.elements().map(|e| e.id).collect();
        // Cursor right on point a's projected pixel (0, 0).
        let hit = pick_at(&doc, &pickable, [1.0, -1.0], ViewMode::Flat, &quad_projector).unwrap();
        assert_eq!(hit.element, ea);
        assert_eq!(hit.rank, cesium_plot::model::RANK_MARKER);
    }

    /// 标记与线共享同一像素时，标记（rank 0）应压过线描边（rank 1）。
    #[test]
    fn marker_outranks_a_line_crossing_the_same_pixel() {
        // A marker at (0,0) and a line through (0,0) → the marker (rank 0) wins
        // over the line stroke (rank 1) even though the cursor is on both.
        let mut doc = Document::default();
        let layer = doc.new_layer("L");
        let marker = doc.make_element("m", Geometry::Point(GeoPoint::surface(0.0, 0.0)));
        let line = doc.make_element(
            "l",
            Geometry::Polyline(Polyline {
                positions: vec![
                    GeoPoint::surface(-100.0, 0.0),
                    GeoPoint::surface(100.0, 0.0),
                ],
            }),
        );
        let mid = marker.id;
        doc.add_element_to_layer(layer, marker);
        doc.add_element_to_layer(layer, line);
        let pickable: BTreeSet<ElementId> = doc.elements().map(|e| e.id).collect();
        // Cursor at the shared (0,0) screen pixel.
        let hit = pick_at(&doc, &pickable, [0.0, 0.0], ViewMode::Flat, &quad_projector).unwrap();
        assert_eq!(hit.element, mid, "marker beats line");
        assert_eq!(hit.rank, cesium_plot::model::RANK_MARKER);
    }

    /// 同一 rank 内 z_order 更高的点元素应胜出（5 压过 1）。
    /// 确认堆叠次序作为次级排序键参与最佳命中评选。
    #[test]
    fn higher_z_order_wins_within_the_same_rank() {
        let mut doc = Document::default();
        let layer = doc.new_layer("L");
        let mut low = doc.make_element("low", Geometry::Point(GeoPoint::surface(0.0, 0.0)));
        low.element.style.z_order = 1;
        let mut high = doc.make_element("high", Geometry::Point(GeoPoint::surface(0.0, 0.0)));
        high.element.style.z_order = 5;
        let low_id = low.id;
        let high_id = high.id;
        doc.add_element_to_layer(layer, low);
        doc.add_element_to_layer(layer, high);
        let _ = low_id;
        let pickable: BTreeSet<ElementId> = doc.elements().map(|e| e.id).collect();
        let hit = pick_at(&doc, &pickable, [0.0, 0.0], ViewMode::Flat, &quad_projector).unwrap();
        assert_eq!(hit.element, high_id, "z_order 5 beats 1");
    }

    /// 仅可拾取集合内的元素参与碰撞：集合外的 `ea` 光标处无命中。
    #[test]
    fn only_the_pickable_set_is_considered() {
        let (doc, _ea, eb) = point_doc();
        // Only `eb` in the pickable set → a cursor on `ea` yields nothing.
        let pickable: BTreeSet<ElementId> = [eb].into_iter().collect();
        assert!(pick_at(&doc, &pickable, [0.0, 0.0], ViewMode::Flat, &quad_projector).is_none());
        let hit = pick_at(&doc, &pickable, [200.0, 200.0], ViewMode::Flat, &quad_projector).unwrap();
        assert_eq!(hit.element, eb);
    }

    /// 折线拾取应先在顶点处报告 Vertex(索引)，远离顶点时报告 Edge(索引)。
    #[test]
    fn polyline_reports_vertex_then_edge() {
        let mut doc = Document::default();
        let layer = doc.new_layer("L");
        let line = doc.make_element(
            "l",
            Geometry::Polyline(Polyline {
                positions: vec![
                    GeoPoint::surface(0.0, 0.0),
                    GeoPoint::surface(10.0, 0.0),
                    GeoPoint::surface(20.0, 0.0),
                ],
            }),
        );
        doc.add_element_to_layer(layer, line);
        let pickable: BTreeSet<ElementId> = doc.elements().map(|e| e.id).collect();
        // Near vertex 1 (screen 40,0) → a Vertex part with index 1.
        let hit = pick_at(&doc, &pickable, [40.0, 0.0], ViewMode::Flat, &quad_projector).unwrap();
        assert_eq!(hit.part, Part::Vertex(1));
        // Along edge 0, away from vertices (screen 20,1) → Edge(0).
        let hit = pick_at(&doc, &pickable, [20.0, 1.0], ViewMode::Flat, &quad_projector).unwrap();
        assert_eq!(hit.part, Part::Edge(0));
    }

    /// 标签锚点应作为标记级元素可被拾取（rank = RANK_MARKER）。
    #[test]
    fn label_anchor_is_pickable() {
        let mut doc = Document::default();
        let layer = doc.new_layer("L");
        let lbl = doc.make_element(
            "lbl",
            Geometry::Label(LabelGeometry {
                at: GeoPoint::surface(5.0, 5.0),
                text: "hi".into(),
                anchor: LabelAnchor::Center,
                offset_px: [0.0, 0.0],
            }),
        );
        doc.add_element_to_layer(layer, lbl);
        let pickable: BTreeSet<ElementId> = doc.elements().map(|e| e.id).collect();
        // (5,5) → screen (20,20).
        let hit = pick_at(&doc, &pickable, [20.0, 20.0], ViewMode::Flat, &quad_projector).unwrap();
        assert_eq!(hit.rank, cesium_plot::model::RANK_MARKER);
    }

    /// 矩形内部命中返回 Body（rank = 面体），边界命中返回 Edge（rank = 边）。
    #[test]
    fn polygon_interior_beats_nothing_and_edge_is_a_ring() {
        // A rectangle (0,0)-(10,10) → screen (0,0)-(40,40). Cursor in the middle
        // hits the *body*; cursor on the top edge hits an *edge* (boundary).
        let mut doc = Document::default();
        let layer = doc.new_layer("L");
        let rect = doc.make_element(
            "r",
            Geometry::Rectangle(Rectangle {
                west: 0.0,
                south: 0.0,
                east: 10.0,
                north: 10.0,
            }),
        );
        doc.add_element_to_layer(layer, rect);
        let pickable: BTreeSet<ElementId> = doc.elements().map(|e| e.id).collect();
        let body = pick_at(&doc, &pickable, [20.0, 20.0], ViewMode::Flat, &quad_projector).unwrap();
        assert_eq!(body.part, Part::Body);
        assert_eq!(body.rank, RANK_POLY_BODY);
        let edge = pick_at(&doc, &pickable, [20.0, 40.0], ViewMode::Flat, &quad_projector).unwrap();
        assert!(matches!(edge.part, Part::Edge(_)), "{:#?}", edge.part);
        assert_eq!(edge.rank, RANK_POLY_EDGE);
    }

    /// 粗相位应保守保留：内部/松弛范围内/投影失败/空盒都不误拒，仅明确在外才剔除。
    #[test]
    fn bounds_broad_phase_includes_overlaps_and_unknowns() {
        let b = GeoBounds::from_points(&[
            GeoPoint::surface(0.0, 0.0),
            GeoPoint::surface(10.0, 10.0),
        ]);
        // quad_projector maps the box to screen (0,0)-(40,40).
        assert!(bounds_might_hit(&b, &quad_projector, [20.0, 20.0], 6.0)); // interior
        assert!(bounds_might_hit(&b, &quad_projector, [46.0, 46.0], 6.0)); // within slack
        assert!(!bounds_might_hit(&b, &quad_projector, [70.0, 20.0], 6.0)); // clearly outside
        // An empty box never culls.
        assert!(bounds_might_hit(&GeoBounds::empty(), &quad_projector, [9999.0, 9999.0], 0.0));
        // A projector that cannot project some sample keeps the element (no false reject).
        let flaky = |g: GeoPoint| -> Option<[f64; 2]> {
            if g.lat_deg > 3.0 {
                None
            } else {
                Some([g.lon_deg * 4.0, g.lat_deg * 4.0])
            }
        };
        assert!(bounds_might_hit(&b, &flaky, [9999.0, 9999.0], 0.0));
    }

    /// 粗相位网格采样不应丢弃远处角点包围的大矩形内部命中。
    #[test]
    fn broad_phase_does_not_drop_an_interior_face_hit() {
        // A rectangle spanning the cursor: the body hit sits far from every
        // control corner, yet the grid-sampled projected box still contains it.
        let mut doc = Document::default();
        let layer = doc.new_layer("L");
        let rect = doc.make_element(
            "r",
            Geometry::Rectangle(Rectangle {
                west: -50.0,
                south: -50.0,
                east: 50.0,
                north: 50.0,
            }),
        );
        doc.add_element_to_layer(layer, rect);
        let pickable: BTreeSet<ElementId> = doc.elements().map(|e| e.id).collect();
        let hit = pick_at(&doc, &pickable, [0.0, 0.0], ViewMode::Flat, &quad_projector).unwrap();
        assert_eq!(hit.part, Part::Body);
    }
}
