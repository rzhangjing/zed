//! 一个极简的绘制工具栏（计划 §8 / §17.5）：一小条按钮，每个
//! [`DrawKind`] 一个，发布交互 FSM 会接收的 [`PlotSetTool`] 事件。叠加层
//! 拥有它，且只在窗口分支上 spawn（headless 从不添加该插件，因此基线
//! 保持字节一致）；应用也可直接通过事件驱动工具，使此 UI 保持可选。

use bevy::prelude::*;

use cesium_plot::model::ids::ElementId;
use cesium_plot::ops::DrawKind;

use crate::edit::{apply_command, delete_commands};
use crate::interaction::{PlotSetTool, PlotTool};
use crate::resources::{PlotDocument, PlotHistory, PlotSelection};

/// 每个工具栏按钮所选的工具。
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct ToolButton(PlotTool);

/// 工具栏根节点的标记，以免被重复 spawn。
#[derive(Component)]
pub(crate) struct ToolbarRoot;

/// 请求清空文档中所有已绘制图元。工具栏 "Clear" 按钮在回到 idle
/// 之外额外广播此事件，由 [`clear_all_system`] 兑现为一次可撤销的删除。
#[derive(Event, Clone, Copy, Debug)]
pub struct PlotClearAll;

/// 标记工具栏里的 "Clear" 按钮，以便点击时除 [`PlotSetTool`] 外
/// 再广播一个 [`PlotClearAll`]（其余按钮只切工具，不清图元）。
#[derive(Component)]
pub(crate) struct ClearButton;

/// 记住已 spawn 的工具栏根节点的资源（为与插件的其他资源保持对称而存在；
/// 根节点在 [`plot_toolbar`] 中只 spawn 一次）。
#[derive(Resource, Default)]
pub struct PlotToolbarRoot {
    /// 已 spawn 的根 UI 实体。
    pub root: Option<Entity>,
}

/// 每个按钮的标签 + 工具，按显示顺序排列。
///
/// 仅用 ASCII：内置的 FiraSans 没有 CJK 字形，中文标签会渲染成豆腐块；
/// 直接携带 [`PlotTool`] 也消除了旧代码里靠"取消"字符串比较的哨兵逻辑。
const BUTTONS: &[(&str, PlotTool)] = &[
    ("Point", PlotTool::Draw(DrawKind::Point)),
    ("Line", PlotTool::Draw(DrawKind::Polyline)),
    ("Area", PlotTool::Draw(DrawKind::Polygon)),
    ("Rect", PlotTool::Draw(DrawKind::Rectangle)),
    ("Circle", PlotTool::Draw(DrawKind::Circle)),
    ("Clear", PlotTool::Idle),
];

/// spawn 工具栏（一个 Startup 系统）。按钮位于左上角；最后一个是一个
/// 回到 idle 的特殊取消按钮。
///
/// # 参数
/// - `commands`：spawn 根节点与逐按钮实体的 ECS 命令。
/// - `root_res`：记录已 spawn 根节点的资源，避免重复创建。
pub fn plot_toolbar(mut commands: Commands, mut root_res: ResMut<PlotToolbarRoot>) {
    let root = commands
        .spawn((
            ToolbarRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(8.0),
                top: Val::Px(8.0),
                column_gap: Val::Px(6.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.1, 0.1, 0.1, 0.6)),
            GlobalZIndex(2000),
        ))
        .id();
    root_res.root = Some(root);

    for (label, tool) in BUTTONS {
        let tool = *tool;
        let is_clear = *label == "Clear";
        commands.entity(root).with_children(|c| {
            // 标签作为按钮的**子节点**，由按钮自身的 flex（justify/align
            // Center）居中。旧实现把标签作为按钮的**兄弟节点** + Absolute(0,0)，
            // 绝对定位相对的是工具栏根而非按钮，导致 6 个标签全部堆叠在
            // 左上角——看起来就像乱码。
            let mut btn = c.spawn((
                ToolButton(tool),
                Button,
                Node {
                    width: Val::Px(48.0),
                    height: Val::Px(28.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.2, 0.4, 0.7, 1.0)),
            ));
            // 只有 "Clear" 额外挂上 ClearButton 标记，触发清空文档。
            if is_clear {
                btn.insert(ClearButton);
            }
            btn.with_children(|b| {
                b.spawn((
                    Text::new(*label),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 1.0, 1.0)),
                ));
            });
        });
    }
}

/// 每帧把工具栏根的 `TargetCamera` 绑到激活相机 —— 与面板的
/// [`crate::panel::bind_panel_camera`] 同款的多相机 UI 不变量。
///
/// 本应用同时存在 3D 透视与 2D 正交两台相机，未绑定 `TargetCamera` 的
/// UI 根会落到非激活相机上，导致工具栏在 3D/2D 切换或某些启动顺序下不显示
/// （面板因已绑定而一直可见）。这里对工具栏根补上同样的绑定。
pub(crate) fn bind_toolbar_camera(
    mut commands: Commands,
    ctx: Res<crate::resources::PlotViewCtx>,
    cams: Query<(Entity, &Camera, &Projection)>,
    roots: Query<(Entity, Option<&TargetCamera>), With<ToolbarRoot>>,
) {
    if roots.is_empty() {
        return;
    }
    let Some(active) = crate::panel::active_camera(&cams, ctx.mode) else {
        return;
    };
    for (e, bound) in roots.iter() {
        if bound.map(|t| t.0) != Some(active) {
            commands.entity(e).insert(TargetCamera(active));
        }
    }
}

/// 将被按下的工具栏按钮转为一个 [`PlotSetTool`] 事件（Bevy 0.15 没有
/// `Interaction::Clicked`，因此按下沿就是触发器）。
///
/// # 参数
/// - `interactions`：携带 [`ToolButton`] 且状态变化的按钮查询。
/// - `tool_events`：向 FSM 发布所选工具的写出器。
pub(crate) fn toolbar_click_system(
    interactions: Query<(&Interaction, &ToolButton, Option<&ClearButton>), Changed<Interaction>>,
    mut tool_events: EventWriter<PlotSetTool>,
    mut clear_events: EventWriter<PlotClearAll>,
) {
    for (interaction, button, clear) in interactions.iter() {
        if matches!(interaction, Interaction::Pressed) {
            tool_events.send(PlotSetTool(button.0));
            // “Clear” 除切回 idle 外还广播一次清空：旧行为只发 Idle 不删图元，
            // 导致误画的超大图元无法从工具栏移除。
            if clear.is_some() {
                clear_events.send(PlotClearAll);
            }
        }
    }
}

/// 清空文档中的全部图元（可撤销），并清除选择集。返回是否确实删除了元素。
///
/// 与面板 Delete 共用同一可撤销管线：把所有元素折叠为一个
/// [`PlotCommand::Composite`] 的删除命令，一次 Clear == 一步撤销。
///
/// # 参数
/// - `plot_doc`/`history`/`selection`：目标文档、命令栈与选择集。
///
/// # 返回
/// 有图元被删除时 `true`；文档已空时 `false`（no-op）。
pub(crate) fn clear_all(
    plot_doc: &mut PlotDocument,
    history: &mut PlotHistory,
    selection: &mut PlotSelection,
) -> bool {
    let ids: Vec<ElementId> = plot_doc.doc.element_ids().collect();
    if ids.is_empty() {
        return false;
    }
    let command = delete_commands(&plot_doc.doc, ids);
    apply_command(plot_doc, history, command);
    selection.clear();
    true
}

/// 消费 [`PlotClearAll`] 事件，把工具栏 "Clear" 的点击兑现为 [`clear_all`]。
pub(crate) fn clear_all_system(
    mut clears: EventReader<PlotClearAll>,
    mut plot_doc: ResMut<PlotDocument>,
    mut history: ResMut<PlotHistory>,
    mut selection: ResMut<PlotSelection>,
) {
    for _ in clears.read() {
        clear_all(&mut plot_doc, &mut history, &mut selection);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_plot::geo::GeoPoint;
    use cesium_plot::model::geometry::Geometry;
    use crate::interaction::commit_geometry;

    /// Clear 应删光全部图元、清空选择，且可一步撤销恢复。
    #[test]
    fn clear_all_removes_every_element_and_is_undoable() {
        let mut plot_doc = PlotDocument::default();
        let mut history = PlotHistory::default();
        let mut selection = PlotSelection::default();
        // 画两个点（commit_geometry 会自动建层并选中）。
        for i in 0..2u64 {
            let g = Geometry::Point(GeoPoint::surface(i as f64, 0.0));
            commit_geometry(&mut plot_doc, &mut history, &mut selection, DrawKind::Point, g);
        }
        assert_eq!(plot_doc.doc.element_count(), 2);
        assert!(!selection.0.is_empty(), "commit selects the new element");
        // Clear 全删 + 清选择。
        assert!(clear_all(&mut plot_doc, &mut history, &mut selection));
        assert_eq!(plot_doc.doc.element_count(), 0);
        assert!(selection.0.is_empty(), "clear empties the selection");
        // 空文档再 Clear 是 no-op（不往历史里记空命令）。
        assert!(!clear_all(&mut plot_doc, &mut history, &mut selection));
        // 撤销这一次 Clear → 两个元素回来。
        history.0.undo(&mut plot_doc.doc);
        assert_eq!(plot_doc.doc.element_count(), 2, "undo restores all");
    }
}
