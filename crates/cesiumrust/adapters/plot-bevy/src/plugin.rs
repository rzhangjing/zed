//! `CesiumPlotBridgePlugin` —— 仅在窗口模式下注册的叠加层桥接。
//!
//! M2–M6 注册共享资源、将场景文档投影到渲染层 3（同时 2D / 3D）的
//! 视图同步系统、拾取系统（悬停 + 点击选择）、绘制 FSM 与编辑层（移动 /
//! 复制 / 删除 / 撤销重做）。该插件只在应用的窗口分
//! 支上添加，因此 headless 离屏基线根本不会实例化
//! 它，并保持字节一致。

use bevy::prelude::*;

use crate::edit::{drag_move_system, edit_system, PlotDrag};
use crate::interaction::{
    draw_preview_system, interaction_system, PlotDrawFinished, PlotInteraction, PlotSetTool,
};
use crate::picking::{pick_system, PlotClickGuard, PlotContextMenu, PlotHoverChanged, PlotPickCache, PlotSelectionChanged};
use crate::panel::{
    bind_panel_camera, panel_click_system, panel_startup, panel_sync_system, PanelRootEntity,
};
use crate::resources::{
    PlotDocument, PlotFilters, PlotHistory, PlotHover, PlotInputCapture, PlotSelection, PlotSnap,
    PlotViewCtx, PlotVisuals,
};
use crate::sync::sync_visuals;
use crate::ui::{
    bind_toolbar_camera, clear_all_system, plot_toolbar, toolbar_click_system, PlotClearAll,
};

/// 接线标绘叠加层桥接的 Bevy 插件。仅由应用在窗口分支上注册，
/// 因此 headless 离屏捕获保持不变。
pub struct CesiumPlotBridgePlugin;

impl Plugin for CesiumPlotBridgePlugin {
    /// 接线标绘叠加层：一次性初始化所有桥接资源、注册事件流，
    /// 并把 Startup/Update 阶段所需系统接入应用调度。
    fn build(&self, app: &mut App) {
        // 初始化叠加层所需的全部 Bevy 资源（视图上下文、文档、过滤器、选择、
        // 交互、历史、拖拽、吸附、拾取缓存、同步状态等）。
        app.init_resource::<PlotViewCtx>()
            .init_resource::<PlotInputCapture>()
            .init_resource::<PlotDocument>()
            .init_resource::<PlotFilters>()
            .init_resource::<PlotVisuals>()
            .init_resource::<PlotSelection>()
            .init_resource::<PlotHover>()
            .init_resource::<PlotInteraction>()
            .init_resource::<PlotHistory>()
            .init_resource::<PlotDrag>()
            .init_resource::<PlotSnap>()
            .init_resource::<PlotPickCache>()
                        .init_resource::<PlotClickGuard>()
            .init_resource::<crate::sync::SyncState>()
            .init_resource::<crate::sync::PlotShapeCache>()
            .init_resource::<crate::ui::PlotToolbarRoot>()
            .init_resource::<PanelRootEntity>()
            // 注册叠加层向外广播的事件流（悬停/选择变更、右键菜单、工具切换、绘制完成）。
            .add_event::<PlotHoverChanged>()
            .add_event::<PlotSelectionChanged>()
            .add_event::<PlotContextMenu>()
            .add_event::<PlotSetTool>()
            .add_event::<PlotDrawFinished>()
            .add_event::<PlotClearAll>()
            // Startup：构建工具栏与右侧属性面板的根实体。
            .add_systems(Startup, (plot_toolbar, panel_startup))
            // Update：每帧按序运行同步→拾取→交互→绘制预览→编辑→拖拽→
            // 工具栏/面板点击与同步等系统。
            .add_systems(
                Update,
                (
                    sync_visuals,
                    pick_system,
                    interaction_system,
                    draw_preview_system,
                    edit_system,
                    drag_move_system,
                    toolbar_click_system,
                    clear_all_system,
                    bind_toolbar_camera,
                    bind_panel_camera,
                    panel_click_system,
                    panel_sync_system,
                ),
            );
    }
}
