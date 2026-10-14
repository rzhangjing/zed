//! 编辑桥接层（计划 §8，M6）：将选择集 + 键盘 / 拖拽手势转化为
//! 可撤销的 [`PlotCommand`]，应用到 [`PlotDocument`] 并记录到
//! [`PlotHistory`] 栈，使每次编辑都可撤销。
//!
//! 命令构建器（[`delete_commands`]、[`duplicate_commands`]、
//! [`translate_commands`]）*对文档是纯函数*——无 ECS、无窗口——
//! 因此整个移动 / 复制 / 删除 / 微调契约可以 headless 单测，
//! 与核心 ops 一致。两个系统只是薄壳：
//!  * [`edit_system`] 从键盘驱动（Delete / Ctrl+D / Ctrl+Z /
//!    Ctrl+Y / 方向键微调）；
//!  * [`drag_move_system`] 将指针在已选元素上的拖拽折叠为单个
//!    平移命令（一次拖拽 == 一步撤销，计划 §8）。
//!
//! 绘制工具激活时（绘制 FSM 拥有按键）以及 headless 时（无相机 / 窗口），
//! 编辑处于惰性状态，因此黄金基线绝不会触发它们。

use bevy::input::mouse::MouseButton;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use cesium_plot::geo::GeoPoint;
use cesium_plot::model::geometry::Geometry;
use cesium_plot::model::ids::ElementId;
use cesium_plot::model::{Document, Style, ViewMode};
use cesium_plot::ops::{transform, PlotCommand};

use crate::interaction::{screen_to_geo, PlotInteraction};
use crate::picking::PlotSelectionChanged;
use crate::resources::{
    PlotDocument, PlotHistory, PlotHover, PlotInputCapture, PlotSelection, PlotViewCtx,
};

/// 指针必须移动的最小屏幕距离（逻辑像素），超过此阈值按下才转为拖拽
/// （这样对已选元素的普通点击不会意外微调它）。
const DRAG_THRESHOLD_PX: f32 = 4.0;
/// 单次方向键微调的步长，单位为经纬度（粗粒度但与视图无关的
/// 步进；精确定位使用拖拽）。
const NUDGE_DEG: f64 = 0.05;

/// 为每个仍存在于层中的已选 id 构建移除命令。
///
/// # 参数
/// - `doc`：文档，用于查找元素及其所属层。
/// - `ids`：待删除的元素 id 集合。
///
/// # 返回
/// 一个包含所有 RemoveElement 步的 composite（可撤销恢复）。
pub fn delete_commands(doc: &Document, ids: impl IntoIterator<Item = ElementId>) -> PlotCommand {
    let steps: Vec<PlotCommand> = ids
        .into_iter()
        .filter_map(|id| {
            let element = doc.element(id)?.clone();
            let (layer, _) = doc.element_context(id)?;
            Some(PlotCommand::RemoveElement {
                element: Box::new(element),
                layer,
            })
        })
        .collect();
    PlotCommand::Composite { steps }
}

/// 为每个已选元素构建新增命令，将其几何 / 样式 /
/// 属性克隆到新分配的 id 上并放入同一层。名称追加
/// “副本”后缀以在面板中区分复制元素（M7）。
///
/// # 参数
/// - `doc`：可变文档，用于铸造新元素 id。
/// - `ids`：待复制的已选元素 id。
/// # 返回
/// 包含所有 AddElement 步的 composite。
pub fn duplicate_commands(doc: &mut Document, ids: &[ElementId]) -> PlotCommand {
    let mut steps = Vec::new();
    for &id in ids {
        // 在分配之前快照源，以便 `make_element` 的借用结束。
        let src = match doc.element(id) {
            Some(e) => e.clone(),
            None => continue,
        };
        let layer = match doc.element_context(id) {
            Some(c) => c.0,
            None => continue,
        };
        let mut ne = doc.make_element(format!("{} 副本", src.name), src.geometry);
        ne.element.style = src.style;
        ne.element.attributes = src.attributes;
        ne.element.flags = src.flags;
        ne.element.scale_visibility = src.scale_visibility;
        ne.element.time_window = src.time_window;
        steps.push(PlotCommand::AddElement {
            layer,
            element: Box::new(ne.element),
        });
    }
    PlotCommand::Composite { steps }
}

/// 构建移动命令：将每个可编辑的已选元素平移
/// `(dlon, dlat)` 度，折叠为一个 composite（一步撤销）。
///
/// # 参数
/// - `doc`：文档（不可编辑元素被跳过）。
/// - `ids`：已选元素 id。
/// - `dlon`/`dlat`：经纬度平移量。
///
/// # 返回
/// 一个包含各 UpdateGeometry 步的 composite（一步撤销）。
pub fn translate_commands(doc: &Document, ids: &[ElementId], dlon: f64, dlat: f64) -> PlotCommand {
    let steps: Vec<PlotCommand> = ids
        .iter()
        .filter_map(|&id| {
            let e = doc.element(id)?;
            if !e.flags.editable {
                return None;
            }
            let before = e.geometry.clone();
            let after = transform::translate(&before, dlon, dlat);
            Some(PlotCommand::UpdateGeometry {
                id,
                before: Box::new(before),
                after: Box::new(after),
            })
        })
        .collect();
    PlotCommand::Composite { steps }
}

/// 当前选择集的指针拖拽移动（计划 §8 “拖拽选中集”）。按下的元素必须
/// 已被选中且可编辑；移动从存储的 `before` 几何实时应用，
/// 并在释放时折叠为单条命令。
#[derive(Resource, Default)]
pub struct PlotDrag {
    /// 按下已抓住一个选中集（尚未超过移动阈值）。
    active: bool,
    /// 指针已超过 [`DRAG_THRESHOLD_PX`]——移动现已激活。
    armed: bool,
    /// 按下时的屏幕位置（用于阈值判断）。
    start_px: Vec2,
    /// 移动激活时光标下的地理坐标。
    start_geo: Option<GeoPoint>,
    /// 正在被移动的元素。
    moving: Vec<ElementId>,
    /// 在激活时抓取的它们的几何（撤销时的 `before`）。
    before: Vec<(ElementId, Geometry)>,
}

/// 将命令应用到文档并记录（跳过空 composite）。
///
/// # 参数
/// - `plot_doc`/`history`：目标文档与命令栈。
/// - `command`：待提交的 [`PlotCommand`]。
fn commit(plot_doc: &mut PlotDocument, history: &mut PlotHistory, command: PlotCommand) {
    if matches!(&command, PlotCommand::Composite { steps } if steps.is_empty()) {
        return;
    }
    command.apply(&mut plot_doc.doc);
    history.0.record(command);
    plot_doc.mark_dirty();
}

/// 提交路径的公开入口，以便 M7 面板能将样式和
/// 几何编辑路由到键盘 / 拖拽系统使用的同一撤销记录管线
/// （一次面板编辑 == 一步撤销）。
///
/// # 参数
/// - `plot_doc`/`history`：目标文档与命令栈。
/// - `command`：待提交的 [`PlotCommand`]。
pub fn apply_command(plot_doc: &mut PlotDocument, history: &mut PlotHistory, command: PlotCommand) {
    commit(plot_doc, history, command);
}

/// 为每个在 `mutate` 下样式确实改变的已选元素构建样式编辑命令，
/// 捕获 before/after 以使编辑可撤销。
/// 已携带目标样式的元素被跳过，因此空操作产生
/// 空 composite（[`apply_command`] 会丢弃它而不记录）。
///
/// # 参数
/// - `doc`：文档，提供当前样式。
/// - `ids`：已选元素 id。
/// - `mutate`：将旧样式映射为新样式的闭包。
pub fn style_command(
    doc: &Document,
    ids: &[ElementId],
    mutate: &dyn Fn(&Style) -> Style,
) -> PlotCommand {
    let steps: Vec<PlotCommand> = ids
        .iter()
        .filter_map(|id| {
            let element = doc.element(*id)?;
            let before = element.style.clone();
            let after = mutate(&before);
            if before == after {
                return None;
            }
            Some(PlotCommand::SetStyle {
                id: *id,
                before: Box::new(before),
                after: Box::new(after),
            })
        })
        .collect();
    PlotCommand::Composite { steps }
}

/// 移除不再存在于文档中的选择 id（撤销新增后）。
///
/// # 参数
/// - `selection`：待修剪的选择集。
/// - `doc`：当前文档（判断元素是否仍存在）。
/// - `events`：修剪发生时发送选择变更事件。
fn prune_selection(
    selection: &mut PlotSelection,
    doc: &Document,
    events: &mut EventWriter<PlotSelectionChanged>,
) {
    let before = selection.0.len();
    selection.0.retain(|id| doc.element(*id).is_some());
    if selection.0.len() != before {
        events.send(PlotSelectionChanged {
            selected: selection.0.len() as u32,
        });
    }
}

/// 键盘编辑系统（参见模块文档）。
///
/// # 参数
/// - `plot_doc`/`history`：可撤销编辑的文档与命令栈。
/// - `selection`：当前选择集（热键作用目标）。
/// - `interaction`：交互 FSM，绘制期间热键属于它。
/// - `keys`：键盘输入状态。
/// - `selection_events`：选择变化时广播。
pub fn edit_system(
    mut plot_doc: ResMut<PlotDocument>,
    mut history: ResMut<PlotHistory>,
    mut selection: ResMut<PlotSelection>,
    interaction: Res<PlotInteraction>,
    keys: Res<ButtonInput<KeyCode>>,
    mut selection_events: EventWriter<PlotSelectionChanged>,
) {
    // 绘制激活时编辑热键属于绘制 FSM。
    if interaction.is_drawing() {
        return;
    }
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    // 先撤销 / 重做（它们在空选择时也起作用）。
    if ctrl && keys.just_pressed(KeyCode::KeyZ) {
        let moved = if shift {
            history.0.redo(&mut plot_doc.doc)
        } else {
            history.0.undo(&mut plot_doc.doc)
        };
        if moved.is_some() {
            plot_doc.mark_dirty();
            prune_selection(&mut selection, &plot_doc.doc, &mut selection_events);
        }
        return;
    }
    if ctrl && keys.just_pressed(KeyCode::KeyY) {
        if history.0.redo(&mut plot_doc.doc).is_some() {
            plot_doc.mark_dirty();
            prune_selection(&mut selection, &plot_doc.doc, &mut selection_events);
        }
        return;
    }

    let ids: Vec<ElementId> = selection.0.iter().copied().collect();
    if ids.is_empty() {
        return;
    }

    // 删除。
    if keys.just_pressed(KeyCode::Delete) {
        let command = delete_commands(&plot_doc.doc, ids);
        commit(&mut plot_doc, &mut history, command);
        if selection.clear() {
            selection_events.send(PlotSelectionChanged { selected: 0 });
        }
        return;
    }

    // 复制（Ctrl+D）。
    if ctrl && keys.just_pressed(KeyCode::KeyD) {
        let command = duplicate_commands(&mut plot_doc.doc, &ids);
        let new_ids = command.targets();
        commit(&mut plot_doc, &mut history, command);
        selection.0 = new_ids.into_iter().collect();
        selection_events.send(PlotSelectionChanged {
            selected: selection.0.len() as u32,
        });
        return;
    }

    // 微调（方向键，每次按下只走一步以免按住时洪泛历史）。
    let mut dlon = 0.0;
    let mut dlat = 0.0;
    if keys.just_pressed(KeyCode::ArrowLeft) {
        dlon -= NUDGE_DEG;
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        dlon += NUDGE_DEG;
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        dlat += NUDGE_DEG;
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        dlat -= NUDGE_DEG;
    }
    if dlon != 0.0 || dlat != 0.0 {
        let command = translate_commands(&plot_doc.doc, &ids, dlon, dlat);
        commit(&mut plot_doc, &mut history, command);
    }
}

/// 指针拖拽移动系统（参见模块文档）。
///
/// # 参数
/// - `drag`：拖拽状态机（active/armed/位移/快照）。
/// - `capture`：输入捕获门控（拖拽期间暂停相机）。
/// - `plot_doc`/`history`：实时平移目标与命令栈。
/// - `selection`/`interaction`/`hover`：拖拽发起所需状态。
/// - `ctx`/`windows`/`cams`/`mouse`/`keys`：屏幕→地理投影所需输入。
#[allow(clippy::too_many_arguments)]
pub fn drag_move_system(
    mut drag: ResMut<PlotDrag>,
    mut capture: ResMut<PlotInputCapture>,
    mut plot_doc: ResMut<PlotDocument>,
    mut history: ResMut<PlotHistory>,
    selection: Res<PlotSelection>,
    interaction: Res<PlotInteraction>,
    hover: Res<PlotHover>,
    ctx: Res<PlotViewCtx>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cams: Query<(&Camera, &GlobalTransform, &Projection)>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    // 绘制期间 FSM 拥有指针；Ctrl+拖拽是框选（M7）。
    if interaction.is_drawing() {
        return;
    }
    let cursor = windows.get_single().ok().and_then(|w| w.cursor_position());
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);

    if drag.active {
        if mouse.just_released(MouseButton::Left) {
            end_drag(&mut plot_doc, &mut history, &mut drag);
            capture.0 = false;
            *drag = PlotDrag::default();
            return;
        }
        let Some(cur) = cursor else { return };
        if !drag.armed {
            if (cur - drag.start_px).length() < DRAG_THRESHOLD_PX {
                return;
            }
            let Some((cam, ct)) = active_cam(&cams, ctx.mode) else {
                return;
            };
            let Some(start) = screen_to_geo(cam, ct, ctx.mode, cur) else {
                return;
            };
            drag.start_geo = Some(start);
            drag.before = drag
                .moving
                .iter()
                .filter_map(|id| plot_doc.doc.element(*id).map(|e| (*id, e.geometry.clone())))
                .collect();
            drag.armed = true;
        }
        let Some((cam, ct)) = active_cam(&cams, ctx.mode) else {
            return;
        };
        let Some(now) = screen_to_geo(cam, ct, ctx.mode, cur) else {
            return;
        };
        let Some(start) = drag.start_geo else {
            return;
        };
        let (dlon, dlat) = (now.lon_deg - start.lon_deg, now.lat_deg - start.lat_deg);
        for (id, base) in &drag.before {
            if let Some(e) = plot_doc.doc.element_mut(*id) {
                e.set_geometry(transform::translate(base, dlon, dlat));
            }
        }
        plot_doc.mark_dirty();
        return;
    }

    // 在已选且可编辑的元素上按下时抓住拖拽；对新元素的
    // 普通点击留给拾取器处理。
    if mouse.just_pressed(MouseButton::Left) && !ctrl {
        if let Some(hit) = hover.0 {
            if selection.contains(hit.element) {
                drag.active = true;
                drag.armed = false;
                drag.start_px = cursor.unwrap_or(Vec2::ZERO);
                drag.moving = selection.0.iter().copied().collect();
                capture.0 = true;
            }
        }
    }
}

/// 将已完成的拖拽折叠为单条记录的移动命令（无移动则为空操作）。
///
/// # 参数
/// - `plot_doc`/`history`：目标文档与命令栈。
/// - `drag`：拖拽状态（未 armed 则直接返回）。
fn end_drag(plot_doc: &mut PlotDocument, history: &mut PlotHistory, drag: &mut PlotDrag) {
    if !drag.armed {
        return;
    }
    let mut steps = Vec::new();
    for (id, base) in &drag.before {
        let Some(e) = plot_doc.doc.element(*id) else {
            continue;
        };
        if &e.geometry != base {
            steps.push(PlotCommand::UpdateGeometry {
                id: *id,
                before: Box::new(base.clone()),
                after: Box::new(e.geometry.clone()),
            });
        }
    }
    if !steps.is_empty() {
        let command = PlotCommand::Composite { steps };
        command.apply(&mut plot_doc.doc);
        history.0.record(command);
        plot_doc.mark_dirty();
    }
}

/// 为视图模式选择激活相机（先匹配投影类型，否则任选
/// 一个激活的）——与拾取 / 同步系统使用相同的规则。
///
/// # 参数
/// - `cams`：相机查询（Camera + GlobalTransform + Projection）。
/// - `mode`：当前视图模式，决定优先匹配哪种投影。
fn active_cam<'q>(
    cams: &'q Query<(&Camera, &GlobalTransform, &Projection)>,
    mode: ViewMode,
) -> Option<(&'q Camera, &'q GlobalTransform)> {
    let want_persp = matches!(mode, ViewMode::Globe);
    let mut fallback = None;
    for (c, ct, p) in cams.iter() {
        if !c.is_active {
            continue;
        }
        let is_persp = matches!(p, Projection::Perspective(_));
        if is_persp == want_persp {
            return Some((c, ct));
        }
        if fallback.is_none() {
            fallback = Some((c, ct));
        }
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_plot::geo::GeoPoint;
    use cesium_plot::model::geometry::Polyline;
    use cesium_plot::model::{Document, Element};

    /// 构造一个地理点（表面高度）的测试简写。
    fn p(lon: f64, lat: f64) -> GeoPoint {
        GeoPoint::surface(lon, lat)
    }

    /// 构造一个含单条双顶点多段线的默认层文档，返回其 id。
    fn line_doc() -> (Document, ElementId) {
        let mut doc = Document::with_default_layer();
        let layer = doc.active_layer().unwrap();
        let ne = doc.make_element(
            "l",
            Geometry::Polyline(Polyline {
                positions: vec![p(0.0, 0.0), p(1.0, 1.0)],
            }),
        );
        let id = ne.id;
        doc.add_element_to_layer(layer, ne);
        (doc, id)
    }

    /// 删除命令应移除元素，逆命令可撤销恢复它。
    #[test]
    fn delete_command_removes_then_undo_restores() {
        let (mut doc, id) = line_doc();
        let cmd = delete_commands(&doc, [id]);
        assert_eq!(cmd.targets(), vec![id]);
        cmd.apply(&mut doc);
        assert!(doc.element(id).is_none());
        cmd.inverse().apply(&mut doc);
        assert!(doc.element(id).is_some(), "undo re-adds the element");
    }

    /// 复制命令应铸造一个新 id 并克隆几何，逆命令移除副本。
    #[test]
    fn duplicate_command_adds_a_new_id() {
        let (mut doc, id) = line_doc();
        let cmd = duplicate_commands(&mut doc, &[id]);
        let new_ids = cmd.targets();
        assert_eq!(new_ids.len(), 1);
        assert_ne!(new_ids[0], id, "the copy has a fresh id");
        cmd.apply(&mut doc);
        assert_eq!(doc.element_count(), 2);
        assert_eq!(
            doc.element(new_ids[0]).unwrap().geometry,
            doc.element(id).unwrap().geometry
        );
        cmd.inverse().apply(&mut doc);
        assert_eq!(doc.element_count(), 1);
    }

    /// 平移命令应移动所有顶点，逆命令回到原位。
    #[test]
    fn translate_command_moves_and_undoes() {
        let (mut doc, id) = line_doc();
        let cmd = translate_commands(&doc, &[id], 2.0, 1.0);
        cmd.apply(&mut doc);
        let moved = doc.element(id).unwrap().geometry.vertices();
        assert_eq!((moved[0].lon_deg, moved[0].lat_deg), (2.0, 1.0));
        cmd.inverse().apply(&mut doc);
        let back = doc.element(id).unwrap().geometry.vertices();
        assert_eq!((back[0].lon_deg, back[0].lat_deg), (0.0, 0.0));
    }

    /// 不可编辑的已选元素应被平移命令跳过（产生空 composite）。
    #[test]
    fn translate_skips_ineditable_elements() {
        let (mut doc, id) = line_doc();
        doc.element_mut(id).unwrap().flags.editable = false;
        let cmd = translate_commands(&doc, &[id], 5.0, 5.0);
        assert!(
            matches!(&cmd, PlotCommand::Composite { steps } if steps.is_empty()),
            "an uneditable selection yields no steps"
        );
    }

    /// 历史栈应能回放复制命令：应用后计数增长，撤销后回落。
    #[test]
    fn history_roundtrips_a_duplicate() {
        use cesium_plot::ops::HistoryStack;
        let (mut doc, id) = line_doc();
        let mut h = HistoryStack::new();
        let dup = duplicate_commands(&mut doc, &[id]);
        dup.apply(&mut doc);
        h.record(dup);
        assert_eq!(doc.element_count(), 2);
        h.undo(&mut doc);
        assert_eq!(doc.element_count(), 1);
    }

    /// armed 拖拽结束时应将移动折叠为单条可撤销命令。
    #[test]
    fn end_drag_records_a_single_move_when_armed() {
        let (doc, id) = line_doc();
        let mut plot_doc = PlotDocument {
            doc,
            revision: 0,
            dirty: false,
        };
        let mut history = PlotHistory::default();
        // 先在文档中移动元素，然后将其折叠为已完成的拖拽。
        let base = Geometry::Polyline(Polyline {
            positions: vec![p(0.0, 0.0), p(1.0, 1.0)],
        });
        let mut drag = PlotDrag {
            active: true,
            armed: true,
            start_px: Vec2::ZERO,
            start_geo: Some(p(0.0, 0.0)),
            moving: vec![id],
            before: vec![(id, base)],
        };
        plot_doc.doc.element_mut(id).unwrap().set_geometry(Geometry::Polyline(Polyline {
            positions: vec![p(3.0, 3.0), p(4.0, 4.0)],
        }));
        end_drag(&mut plot_doc, &mut history, &mut drag);
        assert!(history.0.can_undo());
        history.0.undo(&mut plot_doc.doc);
        let g = plot_doc.doc.element(id).unwrap().geometry.vertices();
        assert_eq!((g[0].lon_deg, g[0].lat_deg), (0.0, 0.0));
    }

    /// 未 armed 的拖拽（仅按下未超阈值）应是空操作，不入栈。
    #[test]
    fn end_drag_is_a_noop_when_not_armed() {
        let (doc, _id) = line_doc();
        let mut plot_doc = PlotDocument {
            doc,
            revision: 0,
            dirty: false,
        };
        let mut history = PlotHistory::default();
        // 已按下（active）但从未超过移动阈值（未 armed）。
        let mut drag = PlotDrag {
            active: true,
            ..PlotDrag::default()
        };
        end_drag(&mut plot_doc, &mut history, &mut drag);
        assert!(!history.0.can_undo());
    }

    /// 元素的 `cloned()` 快照应与文档中的后续修改彼此独立。
    #[test]
    fn a_moved_snapshot_is_independent() {
        let (mut doc, id) = line_doc();
        let snapshot = doc.element(id).cloned().unwrap();
        doc.element_mut(id).unwrap().name = "changed".into();
        assert_eq!(snapshot.name, "l");
        let _ = Element::new(ElementId(123), "unused", Geometry::Point(p(0.0, 0.0)));
    }
}
