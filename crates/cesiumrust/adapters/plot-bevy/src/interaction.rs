//! 交互状态机（计划 §8）：绘制工具。M5 落地有限状态逻辑
//! （工具选择 → 点击添加顶点 → 实时草稿 →
//! 提交 / 取消），输入捕获门控在绘制激活时将指针交给标绘叠加层，
//! 以及进行中草稿的橡皮筋预览。编辑 / 移动 / 历史在 M6
//! 以同一命令层叠加其上。
//!
//! 状态转换与 ECS 分离，因此它们是确定性的且无需窗口或相机即可单测：
//! [`PlotInteraction`] 持有当前 [`PlotTool`] 和草稿顶点并通过纯函数
//! [`commit_draft`] 折叠点击。[`interaction_system`] 是薄壳，
//! 采集原始输入，通过*引擎的*逆变换将光标解析为地理坐标
//! （镜像拾取路径，因此绘制的顶点恰好落在光标位置），
//! 并将结果 [`PlotCommand`] 应用到文档。

use bevy::input::mouse::MouseButton;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::view::RenderLayers;
use bevy::window::PrimaryWindow;

use cesium_plot::geo::{flat_to_geo, GeoPoint};
use cesium_plot::model::geometry::Geometry;
use cesium_plot::model::ids::{ElementId, LayerId};
use cesium_plot::model::ViewMode;
use cesium_plot::ops::{commit_draft, snap, DrawKind, PlotCommand};

use crate::reproject::{
    lift_globe_overlay, line_half_width, ribbon, ViewMetrics, OVERLAY_LIFT_PX,
};
use crate::resources::{
    PlotDocument, PlotHistory, PlotInputCapture, PlotSelection, PlotSnap, PlotViewCtx,
};
use crate::surface;
use crate::sync::OVERLAY_LAYER;

/// 叠加层当前处于哪个工具。`Idle` 让相机拥有指针；
/// `Draw` 类型将其捕获以构建新元素。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlotTool {
    /// 无激活工具（默认）：指针驱动相机 / 选择。
    #[default]
    Idle,
    /// 绘制 `kind`：点击追加草稿顶点。
    Draw(DrawKind),
}

/// 请求工具切换（工具栏 / 热键发送这些；FSM 在下一帧开始时应用）。
#[derive(Event, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlotSetTool(pub PlotTool);

/// 绘制提交（或放弃）时发出，以便 UI 响应。
#[derive(Event, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlotDrawFinished {
    /// 提交元素的 id，或为 `None` 表示绘制被取消。
    pub committed: Option<ElementId>,
}

/// 交互 FSM 状态：当前工具和进行中的草稿。
#[derive(Resource, Default)]
pub struct PlotInteraction {
    /// 激活工具。
    pub tool: PlotTool,
    /// 当前绘制累积的草稿顶点。
    pub draft: Vec<GeoPoint>,
}

impl PlotInteraction {
    /// 从空草稿开始绘制 `kind`。
    pub fn begin(&mut self, kind: DrawKind) {
        self.tool = PlotTool::Draw(kind);
        self.draft.clear();
    }

    /// 回到空闲状态，丢弃草稿。
    pub fn stop(&mut self) {
        self.tool = PlotTool::Idle;
        self.draft.clear();
    }

    /// 是否有绘制工具激活。
    #[inline]
    pub fn is_drawing(&self) -> bool {
        matches!(self.tool, PlotTool::Draw(_))
    }

    /// 记录一次点击。当固定点数类型（点 / 矩形 / 圆）
    /// 达到所需数量时返回提交的几何——草稿随后重置为空闲。
    /// 开放类型（多段线 / 多边形）持续绘制直到
    /// [`finish`](Self::finish)。
    pub fn add_point(&mut self, geo: GeoPoint) -> Option<Geometry> {
        let PlotTool::Draw(kind) = self.tool else {
            return None;
        };
        self.draft.push(geo);
        if let Some(n) = kind.fixed_points() {
            if self.draft.len() >= n {
                let g = commit_draft(kind, &self.draft);
                self.stop();
                return g;
            }
        }
        None
    }

    /// 结束开放式绘制：如果草稿有效则折叠（否则继续绘制以便
    /// 用户继续添加点）。
    pub fn finish(&mut self) -> Option<Geometry> {
        let PlotTool::Draw(kind) = self.tool else {
            return None;
        };
        let g = commit_draft(kind, &self.draft);
        if g.is_some() {
            self.stop();
        }
        g
    }

    /// 取消当前绘制。
    pub fn cancel(&mut self) {
        self.stop();
    }

    /// 删除最后一个草稿顶点（Backspace）。
    pub fn backspace(&mut self) {
        if self.is_drawing() {
            self.draft.pop();
        }
    }
}

/// 通过激活相机的逆变换将逻辑像素光标位置解析为地理坐标——
/// 拾取路径的对应版本，用于放置草稿顶点（计划 §3，“落点 = screen_to_geo”）。
///
/// # 参数
/// - `cam`/`ct`：激活相机及其全局变换。
/// - `mode`：视图模式（Flat 走 2D 逆变换，Globe 射线投球面）。
/// - `cursor`：逻辑像素光标位置。
///
/// # 返回
/// 光标下方的地理坐标（投影失败时为 `None`）。
pub fn screen_to_geo(
    cam: &Camera,
    ct: &GlobalTransform,
    mode: ViewMode,
    cursor: Vec2,
) -> Option<GeoPoint> {
    match mode {
        ViewMode::Flat => {
            let xy = cam.viewport_to_world_2d(ct, cursor).ok()?;
            Some(flat_to_geo(xy.as_dvec2()))
        }
        ViewMode::Globe => {
            let ray = cam.viewport_to_world(ct, cursor).ok()?;
            surface::ray_to_globe(ray.origin, ray.direction.into())
        }
    }
}

/// 将完成的几何提交到文档的活动层（文档为空时创建一个），
/// 选中它，在历史栈上记录新增并推进 revision。返回新的 id。
///
/// # 参数
/// - `plot_doc`/`history`/`selection`：目标文档、命令栈与选择集。
/// - `kind`：绘制类型（用作元素名）。
/// - `geometry`：已折叠的完成几何。
///
/// # 返回
/// 新元素的 [`ElementId`]。
pub fn commit_geometry(
    plot_doc: &mut PlotDocument,
    history: &mut PlotHistory,
    selection: &mut PlotSelection,
    kind: DrawKind,
    geometry: Geometry,
) -> ElementId {
    let layer: LayerId = match plot_doc.doc.active_layer() {
        Some(l) => l,
        None => {
            let l = plot_doc.doc.new_layer("Default layer");
            plot_doc.doc.set_active_layer(Some(l));
            l
        }
    };
    let ne = plot_doc.doc.make_element(format!("{kind:?}"), geometry);
    let id = ne.id;
    let element = ne.element;
    let cmd = PlotCommand::AddElement {
        layer,
        element: Box::new(element),
    };
    cmd.apply(&mut plot_doc.doc);
    history.0.record(cmd);
    plot_doc.mark_dirty();
    selection.select_one(id);
    id
}

/// 为 `mode` 选择激活相机（先匹配投影类型，否则任选一个激活的）
/// 并构建其 [`ViewMetrics`] + 旋转——与渲染 / 拾取系统使用相同的选择规则，
/// 以使输入、投影、绘制一致。
///
/// # 参数
/// - `cams`：相机查询（实体 + Camera + GlobalTransform + Projection）。
/// - `ctx`：视图上下文（模式与屏幕尺寸）。
///
/// # 返回
/// 成功时返回 `(实体, 相机, 变换, 旋转, 度量)`；无激活相机时 `None`。
fn active_view<'q>(
    cams: &'q Query<(Entity, &Camera, &GlobalTransform, &Projection)>,
    ctx: &PlotViewCtx,
) -> Option<(Entity, &'q Camera, &'q GlobalTransform, Quat, ViewMetrics)> {
    let want_persp = matches!(ctx.mode, ViewMode::Globe);
    let mut exact: Option<(Entity, &Camera, &GlobalTransform, &Projection)> = None;
    let mut fallback: Option<(Entity, &Camera, &GlobalTransform, &Projection)> = None;
    for (e, c, ct, p) in cams.iter() {
        if !c.is_active {
            continue;
        }
        let is_persp = matches!(p, Projection::Perspective(_));
        if is_persp == want_persp {
            exact = Some((e, c, ct, p));
            break;
        }
        if fallback.is_none() {
            fallback = Some((e, c, ct, p));
        }
    }
    let (e, cam, ct, proj) = exact.or(fallback)?;
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
    Some((e, cam, ct, ct.rotation(), metrics))
}

/// 绘制 FSM 系统（参见模块文档）。
///
/// # 参数
/// - `ctx`/`plot_doc`/`history`/`selection`：视图与可编辑状态。
/// - `interaction`：FSM 资源（工具 + 草稿）。
/// - `capture`：输入捕获门控（绘制期间置位）。
/// - `snap_res`：吸附配置（落点前折叠光标）。
/// - `tool_events`/`done_events`：工具切换入与绘制完成出。
/// - `windows`/`cams`/`mouse`/`keys`：原始输入与投影。
#[allow(clippy::too_many_arguments)]
pub fn interaction_system(
    ctx: Res<PlotViewCtx>,
    mut plot_doc: ResMut<PlotDocument>,
    mut history: ResMut<PlotHistory>,
    mut interaction: ResMut<PlotInteraction>,
    mut capture: ResMut<PlotInputCapture>,
    mut selection: ResMut<PlotSelection>,
    snap_res: Res<PlotSnap>,
    mut tool_events: EventReader<PlotSetTool>,
    mut done_events: EventWriter<PlotDrawFinished>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cams: Query<(Entity, &Camera, &GlobalTransform, &Projection)>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    // 1. 应用已排队的工具切换。
    //    `just_activated` 记录本帧是否刚由工具栏/面板启动了一个绘制工具，
    //    用于吞掉"激活点击"本身（见第 5 步的穿透防护）。
    let mut just_activated = false;
    for ev in tool_events.read() {
        match ev.0 {
            PlotTool::Idle => interaction.stop(),
            PlotTool::Draw(k) => {
                interaction.begin(k);
                just_activated = true;
            }
        }
    }

    // 2. 取消 / 撤销顶点热键。
    if keys.just_pressed(KeyCode::Escape) && interaction.is_drawing() {
        interaction.cancel();
        done_events.send(PlotDrawFinished { committed: None });
    }
    if keys.just_pressed(KeyCode::Backspace) {
        interaction.backspace();
    }

    // 3. 在整个绘制期间捕获指针以使相机让位。
    capture.0 = interaction.is_drawing();
    if !interaction.is_drawing() {
        return;
    }

    let Some((_, cam, ct, _rot, _metrics)) = active_view(&cams, &ctx) else {
        return;
    };
    let cursor = windows.get_single().ok().and_then(|w| w.cursor_position());

    // 只有绘制中才到达此处，因此工具必定是 `Draw(kind)`。
    let kind = match interaction.tool {
        PlotTool::Draw(k) => k,
        PlotTool::Idle => return,
    };

    // 4. 结束开放式绘制：Enter 或右键均可收尾。
    //    右键更符合“画完点右键结束”的直觉；若当前草稿尚不足以
    //    构成有效几何（点数不够），右键视为放弃本次绘制，避免卡在绘制态；
    //    Enter 保持原语义（不成图则继续绘制）。
    if keys.just_pressed(KeyCode::Enter) || mouse.just_pressed(MouseButton::Right) {
        let right_abort = mouse.just_pressed(MouseButton::Right);
        if let Some(g) = interaction.finish() {
            let id = commit_geometry(&mut plot_doc, &mut history, &mut selection, kind, g);
            done_events.send(PlotDrawFinished {
                committed: Some(id),
            });
        } else if right_abort {
            // 右键且无法收尾（点不够）→ 取消，退出绘制工具。
            interaction.cancel();
            done_events.send(PlotDrawFinished { committed: None });
        }
        return;
    }

    // 5. 左键点击放置 / 完成一个顶点。原始光标坐标
    //    先经过（默认禁用的）吸附配置折叠，以使顶点
    //    在落下之前可能附加到附近的网格 / 顶点 / 边。
    //
    //    穿透防护：点亮左上角工具栏按钮的那一下左键，与 Bevy 的
    //    `ButtonInput::just_pressed` 在同一帧同时为真（UI 的 Interaction::Pressed
    //    不吞物理按键），若不跳过本帧，激活工具的这次点击会立刻在按钮背后的
    //    屏幕位置落下第一个顶点——Point 直接落子、Circle/Rect 得到横跨半球的
    //    超大图元。故当本帧刚激活工具时，跳过落点，等下一次真实点击再开始。
    if !just_activated && mouse.just_pressed(MouseButton::Left) {
        if let Some(pos) = cursor {
            if let Some(geo) = screen_to_geo(cam, ct, ctx.mode, pos) {
                let geo = snap(&plot_doc.doc, geo, &snap_res, None).point();
                if let Some(g) = interaction.add_point(geo) {
                    // 固定点数类型自动完成 → 提交它。
                    let id = commit_geometry(&mut plot_doc, &mut history, &mut selection, kind, g);
                    done_events.send(PlotDrawFinished {
                        committed: Some(id),
                    });
                }
            }
        }
    }
}

/// 进行中草稿的预览：穿过已放置顶点的橡皮筋描边
/// 加上到光标的实时段（计划 §8 “实时预览”）。临时实体携带
/// [`PlotPreviewEntity`] 并每帧重建。
///
/// # 参数
/// - `commands`： despawn 旧预览、 spawn 新 ribbon。
/// - `ctx`/`interaction`：视图与当前草稿。
/// - `old`：上一帧的预览实体查询。
/// - `meshes`/`materials`：预览网格与材质资产。
/// - `windows`/`cams`：光标位置与相机投影。
#[allow(clippy::too_many_arguments)]
pub fn draw_preview_system(
    mut commands: Commands,
    ctx: Res<PlotViewCtx>,
    interaction: Res<PlotInteraction>,
    old: Query<Entity, With<PlotPreviewEntity>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<bevy::pbr::StandardMaterial>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cams: Query<(Entity, &Camera, &GlobalTransform, &Projection)>,
) {
    // 移除上一帧的预览。
    for e in old.iter() {
        commands.entity(e).despawn();
    }

    if !interaction.is_drawing() || interaction.draft.is_empty() {
        return;
    }
    let Some((_, cam, ct, rot, metrics)) = active_view(&cams, &ctx) else {
        return;
    };

    // 草稿顶点 + 到实时光标的橡皮筋尾段。
    let mut geos: Vec<GeoPoint> = interaction.draft.clone();
    if let Some(pos) = windows.get_single().ok().and_then(|w| w.cursor_position()) {
        if let Some(g) = screen_to_geo(cam, ct, ctx.mode, pos) {
            geos.push(g);
        }
    }

    // 穿过链的预览描边（橡皮筋）。
    if geos.len() >= 2 {
        let world: Vec<Vec3> = geos
            .iter()
            .map(|g| lift_globe_overlay(&metrics, metrics.project(*g), OVERLAY_LIFT_PX))
            .collect();
        let normal = rot * Vec3::Z;
        let (pos, idx) = ribbon(&world, &|i| line_half_width(&metrics, world[i], 2.0), normal);
        if pos.is_empty() {
            return;
        }
        let n = pos.len();
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; n]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; n]);
        mesh.insert_indices(Indices::U32(idx));
        let mat = materials.add(preview_material());
        commands.spawn((
            PlotPreviewEntity,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(mat),
            RenderLayers::layer(OVERLAY_LAYER),
            Visibility::Visible,
            Transform::IDENTITY,
        ));
    }
}

/// 临时预览实体的标记（每帧重建）。
#[derive(Component)]
pub struct PlotPreviewEntity;

/// 预览描边颜色（半透明白、双面、无光照）。
///
/// 无需参数：返回一个固定的预览材质。
///
/// # 返回
/// 一个半透明白、无光照、双面的 [`StandardMaterial`]。
fn preview_material() -> bevy::pbr::StandardMaterial {
    bevy::pbr::StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.8),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_plot::model::Document;
    use cesium_plot::ops::SnapConfig;

    /// 构造一个地理点（表面高度）的测试简写。
    fn p(lon: f64, lat: f64) -> GeoPoint {
        GeoPoint::surface(lon, lat)
    }

    #[test]
    fn snap_is_off_by_default_so_draws_are_unaffected() {
        // 基线守卫：桥接层的 snap 资源默认为禁用，且
        // 禁用的配置原封不动地返回原始光标坐标（M9）。
        let snap_cfg = PlotSnap::default();
        assert!(!snap_cfg.enabled, "PlotSnap must default to off");
        let doc = Document::default();
        let raw = p(1.234, 5.678);
        let got = snap(&doc, raw, &snap_cfg, None);
        assert_eq!(got.point(), raw, "disabled snap never moves the vertex");
    }

    #[test]
    fn snapped_vertex_lands_identically_in_2d_and_3d() {
        // M9 双模式证明：吸附是地理折叠，因此吸附后的顶点
        // 在*两种*投影下都与其目标重合。将一个接近现有顶点的原始光标
        // 吸附，然后检查在 Flat（2D）和 Globe（3D）度量下它投影到
        // 与目标相同的世界位置。
        let mut doc = Document::default();
        let layer = doc.new_layer("L");
        let target = p(10.0, 20.0);
        let anchor = doc.make_element("a", Geometry::Point(target));
        doc.add_element_to_layer(layer, anchor);

        let cfg = SnapConfig {
            enabled: true,
            vertex_threshold_m: 200_000.0, // 宽松以便 ~6 km 偏移能量 latch
            ..Default::default()
        };
        let raw = p(10.05, 20.05);
        let snapped = snap(&doc, raw, &cfg, None).point();
        assert_eq!(snapped, target, "snapped exactly onto the existing vertex");

        let flat = ViewMetrics {
            mode: ViewMode::Flat,
            pixels_per_world: 100.0,
            focal_px: 0.0,
            cam_pos: Vec3::ZERO,
        };
        let globe = ViewMetrics {
            mode: ViewMode::Globe,
            pixels_per_world: 0.0,
            focal_px: 600.0,
            cam_pos: Vec3::new(3.0, 0.0, 0.0),
        };
        // 相同的地理真相 ⇒ 相同的渲染位置，无论哪种模式。
        assert!(flat.project(snapped).distance(flat.project(target)) < 1e-9);
        assert!(globe.project(snapped).distance(globe.project(target)) < 1e-9);
    }

    /// 点类型在单次点击时自动完成并回到空闲。
    #[test]
    fn point_auto_completes_on_one_click() {
        let mut fs = PlotInteraction::default();
        fs.begin(DrawKind::Point);
        let g = fs.add_point(p(1.0, 2.0)).expect("commits");
        assert!(matches!(g, Geometry::Point(_)));
        assert!(!fs.is_drawing());
    }

    /// 矩形两次点击完成，并将对角点规范排序为 west/south/east/north。
    #[test]
    fn rectangle_completes_on_two_clicks_and_orders_them() {
        let mut fs = PlotInteraction::default();
        fs.begin(DrawKind::Rectangle);
        assert!(fs.add_point(p(10.0, 20.0)).is_none(), "one click still drawing");
        assert!(fs.is_drawing());
        let g = fs.add_point(p(-5.0, 3.0)).expect("second click commits");
        match g {
            Geometry::Rectangle(r) => assert_eq!((r.west, r.south, r.east, r.north), (-5.0, 3.0, 10.0, 20.0)),
            other => panic!("{other:?}"),
        }
        assert!(!fs.is_drawing());
    }

    /// 多段线保持开放直到 Enter 提交（而非靠固定点数自动完成）。
    #[test]
    fn polyline_finishes_on_enter_not_clicks() {
        let mut fs = PlotInteraction::default();
        fs.begin(DrawKind::Polyline);
        fs.add_point(p(0.0, 0.0));
        fs.add_point(p(1.0, 1.0));
        fs.add_point(p(2.0, 0.0));
        assert!(fs.is_drawing(), "polyline stays open");
        let g = fs.finish().expect("enter commits");
        match g {
            Geometry::Polyline(pl) => assert_eq!(pl.positions.len(), 3),
            other => panic!("{other:?}"),
        }
        assert!(!fs.is_drawing());
    }

    /// 未达最小点数时 finish 不应提交，保持继续绘制。
    #[test]
    fn finish_needs_the_minimum_so_short_polyline_stays_open() {
        let mut fs = PlotInteraction::default();
        fs.begin(DrawKind::Polyline);
        fs.add_point(p(0.0, 0.0));
        assert!(fs.finish().is_none(), "one point is not a polyline");
        assert!(fs.is_drawing(), "still drawing");
    }

    /// Backspace 应删除最后一个草稿顶点。
    #[test]
    fn backspace_drops_the_last_vertex() {
        let mut fs = PlotInteraction::default();
        fs.begin(DrawKind::Polygon);
        fs.add_point(p(0.0, 0.0));
        fs.add_point(p(1.0, 0.0));
        fs.backspace();
        assert_eq!(fs.draft.len(), 1);
    }

    /// 取消应同时清空草稿与工具，回到空闲。
    #[test]
    fn cancel_clears_draft_and_tool() {
        let mut fs = PlotInteraction::default();
        fs.begin(DrawKind::Polygon);
        fs.add_point(p(0.0, 0.0));
        fs.cancel();
        assert!(!fs.is_drawing());
        assert!(fs.draft.is_empty());
    }

    /// 提交应将元素加入文档、自动建层、选中并记录为可撤销。
    #[test]
    fn commit_adds_to_document_and_selects() {
        let mut plot_doc = PlotDocument {
            doc: Document::default(),
            revision: 0,
            dirty: true,
        };
        let mut history = PlotHistory::default();
        let mut selection = PlotSelection::default();
        let g = commit_draft(DrawKind::Polyline, &[p(0.0, 0.0), p(1.0, 1.0)]).unwrap();
        let id = commit_geometry(&mut plot_doc, &mut history, &mut selection, DrawKind::Polyline, g);
        assert_eq!(plot_doc.doc.element_count(), 1);
        assert!(plot_doc.doc.active_layer().is_some(), "layer auto-created");
        assert!(selection.contains(id), "committed element is selected");
        assert!(plot_doc.dirty);
        // 新增已被记录因此可撤销（M6）。
        assert!(history.0.can_undo());
        history.0.undo(&mut plot_doc.doc);
        assert_eq!(plot_doc.doc.element_count(), 0);
    }
}
