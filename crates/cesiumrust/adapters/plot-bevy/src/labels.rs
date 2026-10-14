//! 通过紧贴已投影地理锚点的 bevy_ui 文本渲染标签。
//!
//! 标签是唯一一个存活在 UI 而非渲染层里的叠加层图元：单个
//! [`PlotUiRoot`] 节点拥有一个 [`TargetCamera`]，同步系统会将其重新指向当前
//! 激活的相机，而每个标签都是一个子节点，其绝对 `left` / `top`
//! 跟踪其锚点的屏幕投影。因为投影来自用于度量网格的同一个 [`Camera`]，
//! 标签会在 2D 与 3D 视图下都恰好位于其几何被绘制的位置（计划 §2 “同显”）。

use bevy::prelude::*;
use cesium_plot::model::geometry::LabelGeometry;
use cesium_plot::model::{LabelAnchor, Style};

use crate::resources::PlotLabel;

/// 每个标绘标签都作为子节点挂到的唯一 UI 根节点的标记。
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PlotUiRoot;

/// spawn 标绘 UI 根节点（一个满窗口、透明的容器）。只存在
/// 一个；同步系统惰性地创建它，并使其 `TargetCamera`
/// 指向当前激活的相机。
pub fn spawn_ui_root(commands: &mut Commands) -> Entity {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::NONE),
            GlobalZIndex(1000),
            PlotUiRoot,
        ))
        .id()
}

/// 将一个世界点投影到当前激活相机的视口，返回逻辑像素坐标
/// （左上角原点）。当点位于相机后方或投影失败时返回 `None`。
pub fn world_to_screen(camera: &Camera, ct: &GlobalTransform, world: Vec3) -> Option<Vec2> {
    camera.world_to_viewport(ct, world).ok()
}

/// 逐锚点的像素平移，作用于标签的投影屏幕点，以便文本框按
/// [`LabelAnchor`] 的要求对齐。`text_px` 是文本框的粗略测量尺寸。
pub fn anchor_offset(anchor: LabelAnchor, offset_px: [f32; 2], text_px: Vec2) -> Vec2 {
    let (ax, ay) = match anchor {
        LabelAnchor::Center => (-text_px.x * 0.5, -text_px.y * 0.5),
        LabelAnchor::Left => (0.0, -text_px.y * 0.5),
        LabelAnchor::Right => (-text_px.x, -text_px.y * 0.5),
        LabelAnchor::Top => (-text_px.x * 0.5, 0.0),
        LabelAnchor::Bottom => (-text_px.x * 0.5, -text_px.y),
    };
    Vec2::new(ax + offset_px[0], ay + offset_px[1])
}

/// 在 `root` 下为一个元素 spawn 一个标签文本节点。该节点开始时
/// 在屏幕外；同步系统每帧写入其绝对位置。
///
/// # 参数
/// - `commands`：spawn 文本节点的 ECS 命令。
/// - `root`：标签挂接其下的 UI 根实体。
/// - `element`：该标签所属元素的 id（同步系统据此追踪）。
/// - `geo`/`style`：文本内容与字体/颜色样式。
///
/// # 返回
/// 新 spawn 的标签节点实体。
pub fn spawn_label(
    commands: &mut Commands,
    root: Entity,
    element: cesium_plot::model::ids::ElementId,
    geo: &LabelGeometry,
    style: &Style,
) -> Entity {
    let (size_px, rgba) = match &style.text {
        Some(t) => (t.size_px, t.color),
        None => (16.0, style.color),
    };
    commands
        .spawn((
            PlotLabel { element },
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(-9999.0),
                top: Val::Px(-9999.0),
                ..default()
            },
            Text::new(geo.text.clone()),
            TextFont {
                font_size: size_px,
                ..default()
            },
            TextColor(Color::srgba(rgba[0], rgba[1], rgba[2], rgba[3])),
        ))
        .set_parent(root)
        .id()
}
