//! cesium-plot-bevy —— 面向 2D/3D 态势标绘
//! 叠加层的 Bevy 桥接。
//!
//! 此适配器拥有该特性的 ECS 一半：应用每帧写入的共享桥接资源
//! （[`resources::PlotViewCtx`]、[`resources::PlotInputCapture`]），以及——从后续
//! 里程碑起——将无框架依赖的 [`cesium_plot`] 场景文档投影到渲染层 3 上的
//! 视图同步 / 重投影 / 拾取 / 交互系统。
//!
//! 它从不依赖应用层；应用纯粹通过这些资源与插件
//! （[`CesiumPlotBridgePlugin`]）来驱动它。
//!
//! 设计文档：计划 `cesium-plot_标绘系统总体设计`。

pub mod labels;
pub mod edit;
pub mod interaction;
pub mod io;
pub mod panel;
pub mod picking;
pub mod plugin;
pub mod reproject;
pub mod resources;
pub mod shapes;
pub mod surface;
pub mod sync;
pub mod ui;

pub use labels::PlotUiRoot;
pub use edit::{delete_commands, duplicate_commands, edit_system, translate_commands, PlotDrag};
pub use interaction::{
    screen_to_geo, PlotDrawFinished, PlotInteraction, PlotSetTool, PlotTool,
};
pub use picking::{pick_at, PlotContextMenu, PlotHoverChanged, PlotPickCache, PlotSelectionChanged};
pub use io::{export_geojson, import_geojson};
pub use panel::{apply_panel_action, kind_label, PanelAction, PanelRootEntity};
pub use plugin::CesiumPlotBridgePlugin;
pub use resources::{
    PlotDocument, PlotFilters, PlotHistory, PlotHover, PlotInputCapture, PlotLabel, PlotSelection,
    PlotSnap, PlotViewCtx, PlotViewMode, PlotVisual, PlotVisuals,
};
