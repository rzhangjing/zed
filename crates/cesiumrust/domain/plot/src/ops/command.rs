//! 结构性文档操作（计划 §8 / §14）：绘图工具草稿提交
//! 以及会变更 [`Document`] 的 [`PlotCommand`]。
//!
//! 命令是一个*纯* 值：将它应用于文档就是一个普通函数，
//! 因此同一个命令既驱动实时桥接，也（从 M6 起）通过携带一个逆命令
//! 驱动历史栈的撤销。M5 引入绘图流程使用的 add / remove 配对；
//! M6 扩展几何 / 样式 / 变换 / 组编辑。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::geo::GeoPoint;
use crate::model::document::NewElement;
use crate::model::geometry::{Circle, Geometry, Polyline, Polygon, Rectangle};
use crate::model::ids::{ElementId, LayerId};
use crate::model::{Document, Element, Style};

/// 绘图工具正在生成的图元类型。决定一份草稿顶点列表如何
/// 被折叠成一个具体的几何（[`commit_draft`]）以及工具需要
/// 多少次点击才能完成。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrawKind {
    /// 单个点标记（1 次点击）。
    Point,
    /// 一条自由折线（≥ 2 次点击，由 Enter / 双击完成）。
    Polyline,
    /// 一个闭合多边形（≥ 3 次点击）。
    Polygon,
    /// 由两个对角组成的轴对齐矩形（2 次点击）。
    Rectangle,
    /// 由中心 + 一个半径点构成的地面圆（2 次点击）。
    Circle,
}

impl DrawKind {
    /// 仍能构成此几何的最少草稿顶点数。
    #[inline]
    pub fn min_points(self) -> usize {
        // 各几何类型仍能成立的最少顶点数在此集中定义。
        match self {
            DrawKind::Point => 1,
            DrawKind::Polyline => 2,
            DrawKind::Polygon => 3,
            DrawKind::Rectangle | DrawKind::Circle => 2,
        }
    }

    /// 会自动完成绘制的固定点击数（矩形 / 圆 / 点），
    /// 或由手势完成的开放式类型（折线 / 多边形）则为 `None`。
    #[inline]
    pub fn fixed_points(self) -> Option<usize> {
        // 固定点击数的类型自动完成；开放式手势返回 None。
        match self {
            DrawKind::Point => Some(1),
            DrawKind::Rectangle | DrawKind::Circle => Some(2),
            DrawKind::Polyline | DrawKind::Polygon => None,
        }
    }
}

/// 将一份完成的草稿折叠成具体几何，或当顶点太少时返回 `None`。矩形取
/// 两个对角构成的包围盒；圆取中心与半径点之间的大圆距离。
pub fn commit_draft(kind: DrawKind, draft: &[GeoPoint]) -> Option<Geometry> {
    if draft.len() < kind.min_points() {
        return None;
    }
    match kind {
        // 点：单次点击即落点。
        DrawKind::Point => Some(Geometry::Point(draft[0])),
        // 折线：草稿顶点原样保留为位置序列。
        DrawKind::Polyline => Some(Geometry::Polyline(Polyline {
            positions: draft.to_vec(),
        })),
        // 多边形：外环取草稿，孔洞留空待后续编辑。
        DrawKind::Polygon => Some(Geometry::Polygon(Polygon {
            outer: draft.to_vec(),
            holes: Vec::new(),
        })),
        DrawKind::Rectangle => {
            // 矩形：由两对角求经纬包围盒，自动纠正顺序。
            let (a, b) = (draft[0], draft[1]);
            Some(Geometry::Rectangle(Rectangle {
                west: a.lon_deg.min(b.lon_deg),
                east: a.lon_deg.max(b.lon_deg),
                south: a.lat_deg.min(b.lat_deg),
                north: a.lat_deg.max(b.lat_deg),
            }))
        }
        DrawKind::Circle => {
            // 圆：中心到半径点的大圆距离即半径；零距离视为退化。
            let center = draft[0];
            let radius_m = center.surface_distance(draft[1]);
            if radius_m <= 0.0 {
                return None;
            }
            Some(Geometry::Circle(Circle { center, radius_m }))
        }
    }
}

/// 对文档的一次可逆结构性变更（计划 §14）。桥接层通过
/// [`PlotCommand::apply`] 应用它；历史栈记录它及其
/// [`PlotCommand::inverse`]，以便一次编辑可被撤销 / 重做（M6）。
#[derive(Debug, Clone, PartialEq)]
pub enum PlotCommand {
    /// 向一个图层插入一个完成的元素。
    AddElement {
        /// 新元素落入的目标图层 id。
        layer: LayerId,
        /// 要插入的完整新元素（装箱以控制枚举尺寸）。
        element: Box<Element>,
    },
    /// 按 id 删除一个元素（保留完整元素以便操作可逆）。
    RemoveElement {
        /// 被删除元素的完整副本，供 undo 原样恢复。
        element: Box<Element>,
        /// 元素原先所在的图层 id（恢复时回写到此）。
        layer: LayerId,
    },
    /// 替换一个元素的几何（移动 / 顶点编辑 / 旋转 / 缩放
    /// 的结果）。两侧都保留，以便 undo 恢复之前的形状。
    UpdateGeometry {
        /// 被编辑元素的 id。
        id: ElementId,
        /// 变换前的几何，供 undo 回退。
        before: Box<Geometry>,
        /// 变换后的几何，应用时写入。
        after: Box<Geometry>,
    },
    /// 替换一个元素的整个样式包（属性面板编辑）。
    SetStyle {
        /// 目标元素 id。
        id: ElementId,
        /// 编辑前的样式快照。
        before: Box<Style>,
        /// 编辑后的新样式。
        after: Box<Style>,
    },
    /// 翻转一个元素的手动可见性开关（图层 / 面板眼睛按钮）。
    SetVisibilityFlag {
        /// 目标元素 id。
        id: ElementId,
        /// 翻转前的可见性标志。
        before: bool,
        /// 翻转后的可见性标志。
        after: bool,
    },
    /// 替换一个元素的自由形式业务属性（敌我 / 番号 / 状态 …）。
    /// 两个 map 都保留，以便 undo 恢复之前的元数据；agent 层
    /// （M-agent）为 `SetAttributes` 合并发出此命令。
    SetAttributes {
        /// 目标元素 id。
        id: ElementId,
        /// 合并前的完整属性映射。
        before: Map<String, Value>,
        /// 合并后的完整属性映射。
        after: Map<String, Value>,
    },
    /// 作为单个 undo 步骤应用的一组命令（多选编辑）。
    Composite {
        /// 依次应用的子命令列表，整体计为一个 undo 步骤。
        steps: Vec<PlotCommand>,
    },
}

impl PlotCommand {
    /// 通过此命令变更 `doc`。
    pub fn apply(&self, doc: &mut Document) {
        // 应用即纯函数式地改写文档；每种命令只对应一处最小变更。
        match self {
            // 追加：重建 NewElement 后落入目标图层。
            PlotCommand::AddElement { layer, element } => {
                let ne = NewElement {
                    id: element.id,
                    element: (**element).clone(),
                };
                doc.add_element_to_layer(*layer, ne);
            }
            // 移除：按 id 从树与表中摘除元素。
            PlotCommand::RemoveElement { element, .. } => {
                doc.remove_element(element.id);
            }
            // 几何替换：写入 after 形状（set_geometry 会刷新包围盒）。
            PlotCommand::UpdateGeometry { id, after, .. } => {
                if let Some(e) = doc.element_mut(*id) {
                    e.set_geometry((**after).clone());
                }
            }
            // 样式替换：整包覆盖为 after。
            PlotCommand::SetStyle { id, after, .. } => {
                if let Some(e) = doc.element_mut(*id) {
                    e.style = (**after).clone();
                }
            }
            // 可见性：改写元素的手动可见标志。
            PlotCommand::SetVisibilityFlag { id, after, .. } => {
                if let Some(e) = doc.element_mut(*id) {
                    e.flags.visible_manual = *after;
                }
            }
            // 属性：整表替换为 after 映射。
            PlotCommand::SetAttributes { id, after, .. } => {
                if let Some(e) = doc.element_mut(*id) {
                    e.attributes = after.clone();
                }
            }
            // 复合：按顺序依次应用每个子命令。
            PlotCommand::Composite { steps } => {
                for s in steps {
                    s.apply(doc);
                }
            }
        }
    }

    /// 精确逆转此命令的命令（undo）。重新添加一个元素
    /// 会复用其 id，因此 redo / undo 配对是稳定的。
    pub fn inverse(&self) -> PlotCommand {
        // 逐变体构造逆命令：增删互逆，其余将 before/after 互换。
        match self {
            // 追加的逆 = 移除（携带完整元素供回恢复）。
            PlotCommand::AddElement { layer, element } => PlotCommand::RemoveElement {
                element: element.clone(),
                layer: *layer,
            },
            // 移除的逆 = 原样重新追加到同一图层。
            PlotCommand::RemoveElement { element, layer } => PlotCommand::AddElement {
                layer: *layer,
                element: element.clone(),
            },
            // 几何的逆 = before/after 互换。
            PlotCommand::UpdateGeometry { id, before, after } => PlotCommand::UpdateGeometry {
                id: *id,
                before: after.clone(),
                after: before.clone(),
            },
            // 样式的逆 = before/after 互换。
            PlotCommand::SetStyle { id, before, after } => PlotCommand::SetStyle {
                id: *id,
                before: after.clone(),
                after: before.clone(),
            },
            // 可见性的逆 = 标志互换。
            PlotCommand::SetVisibilityFlag { id, before, after } => PlotCommand::SetVisibilityFlag {
                id: *id,
                before: *after,
                after: *before,
            },
            // 属性的逆 = 两个映射互换。
            PlotCommand::SetAttributes { id, before, after } => PlotCommand::SetAttributes {
                id: *id,
                before: after.clone(),
                after: before.clone(),
            },
            PlotCommand::Composite { steps } => PlotCommand::Composite {
                // 逆转整个组：undo 从后往前运行逆命令。
                steps: steps.iter().rev().map(PlotCommand::inverse).collect(),
            },
        }
    }

    /// 此命令触及的 id（用于变更驱动的调和提示）。
    /// 一个复合命令报告它的第一个叶子目标；一个空复合命令报告
    /// 保留 id `0`（无需调和）。
    pub fn target(&self) -> ElementId {
        match self {
            // 增删都以其携带元素的 id 为目标。
            PlotCommand::AddElement { element, .. } => element.id,
            PlotCommand::RemoveElement { element, .. } => element.id,
            // 其余命令直接携带目标 id。
            PlotCommand::UpdateGeometry { id, .. } => *id,
            PlotCommand::SetStyle { id, .. } => *id,
            PlotCommand::SetVisibilityFlag { id, .. } => *id,
            PlotCommand::SetAttributes { id, .. } => *id,
            // 复合取首个叶子的目标；空组回退到保留 id 0。
            PlotCommand::Composite { steps } => {
                steps.first().map(PlotCommand::target).unwrap_or(ElementId(0))
            }
        }
    }

    /// 此命令触及的每个叶子元素 id（一个复合命令会扁平化；桥接层
    /// 用它在 undo 之后知道要调和哪些视觉效果）。
    pub fn targets(&self) -> Vec<ElementId> {
        match self {
            PlotCommand::Composite { steps } => steps.iter().flat_map(PlotCommand::targets).collect(),
            other => vec![other.target()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::GeoPoint;

    fn p(lon: f64, lat: f64) -> GeoPoint {
        GeoPoint::surface(lon, lat)
    }

    #[test]
    fn point_needs_one_click() {
        let g = commit_draft(DrawKind::Point, &[p(1.0, 2.0)]).unwrap();
        assert!(matches!(g, Geometry::Point(pt) if (pt.lon_deg - 1.0).abs() < 1e-9));
        assert!(commit_draft(DrawKind::Point, &[]).is_none());
    }

    #[test]
    fn polyline_and_polygon_need_their_minimum() {
        assert!(commit_draft(DrawKind::Polyline, &[p(0.0, 0.0)]).is_none());
        assert!(commit_draft(DrawKind::Polygon, &[p(0.0, 0.0), p(1.0, 0.0)]).is_none());
        let line = commit_draft(DrawKind::Polyline, &[p(0.0, 0.0), p(1.0, 1.0)]).unwrap();
        match line {
            Geometry::Polyline(pl) => assert_eq!(pl.positions.len(), 2),
            other => panic!("{other:?}"),
        }
        let poly = commit_draft(
            DrawKind::Polygon,
            &[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)],
        )
        .unwrap();
        match poly {
            Geometry::Polygon(pg) => assert_eq!(pg.outer.len(), 3),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rectangle_orders_corners() {
        // 乱序给出的对角 → west/south 仍为最小值。
        let g = commit_draft(DrawKind::Rectangle, &[p(10.0, 20.0), p(-5.0, 3.0)]).unwrap();
        match g {
            Geometry::Rectangle(r) => {
                assert_eq!((r.west, r.east), (-5.0, 10.0));
                assert_eq!((r.south, r.north), (3.0, 20.0));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn circle_radius_is_great_circle_distance() {
        // 赤道上经度一度 ≈ 111.32 km。
        let g = commit_draft(
            DrawKind::Circle,
            &[p(0.0, 0.0), p(1.0, 0.0)],
        )
        .unwrap();
        match g {
            Geometry::Circle(c) => {
                assert!(c.radius_m > 111_000.0 && c.radius_m < 112_000.0, "{}", c.radius_m);
            }
            other => panic!("{other:?}"),
        }
        // 零半径圆（两次相同点击）被拒绝。
        assert!(commit_draft(DrawKind::Circle, &[p(0.0, 0.0), p(0.0, 0.0)]).is_none());
    }

    #[test]
    fn add_then_remove_roundtrips_the_document() {
        let mut doc = Document::with_default_layer();
        let layer = doc.active_layer().unwrap();
        let element = Element::new(ElementId(99), "pt", Geometry::Point(p(1.0, 2.0)));
        let add = PlotCommand::AddElement {
            layer,
            element: Box::new(element.clone()),
        };
        add.apply(&mut doc);
        assert!(doc.element(ElementId(99)).is_some());
        // 它的逆命令将其移除。
        add.inverse().apply(&mut doc);
        assert!(doc.element(ElementId(99)).is_none());
        // 而逆命令的逆命令重新添加（redo）。
        add.inverse().inverse().apply(&mut doc);
        assert!(doc.element(ElementId(99)).is_some());
    }

    #[test]
    fn fixed_and_open_kinds_report_completion() {
        assert_eq!(DrawKind::Point.fixed_points(), Some(1));
        assert_eq!(DrawKind::Rectangle.fixed_points(), Some(2));
        assert_eq!(DrawKind::Circle.fixed_points(), Some(2));
        assert_eq!(DrawKind::Polyline.fixed_points(), None);
        assert_eq!(DrawKind::Polygon.fixed_points(), None);
    }

    fn doc_with_point(id: u64) -> Document {
        let mut doc = Document::with_default_layer();
        let layer = doc.active_layer().unwrap();
        let e = Element::new(ElementId(id), "pt", Geometry::Point(p(0.0, 0.0)));
        doc.add_element_to_layer(layer, NewElement { id: ElementId(id), element: e });
        doc
    }

    #[test]
    fn update_geometry_moves_and_undoes() {
        let mut doc = doc_with_point(7);
        let after = Geometry::Point(p(3.0, 4.0));
        let cmd = PlotCommand::UpdateGeometry {
            id: ElementId(7),
            before: Box::new(Geometry::Point(p(0.0, 0.0))),
            after: Box::new(after.clone()),
        };
        cmd.apply(&mut doc);
        assert_eq!(doc.element(ElementId(7)).unwrap().geometry, after);
        // Undo 恢复原始值并刷新缓存的包围盒。
        cmd.inverse().apply(&mut doc);
        let e = doc.element(ElementId(7)).unwrap();
        assert_eq!(e.geometry, Geometry::Point(p(0.0, 0.0)));
        assert_eq!((e.bounds.west_deg, e.bounds.north_deg), (0.0, 0.0));
    }

    #[test]
    fn set_style_and_visibility_reverse() {
        let mut doc = doc_with_point(8);
        let styled = Style::default().with_color([1.0, 0.0, 0.0, 1.0]);
        let set = PlotCommand::SetStyle {
            id: ElementId(8),
            before: Box::new(Style::default()),
            after: Box::new(styled.clone()),
        };
        set.apply(&mut doc);
        assert_eq!(doc.element(ElementId(8)).unwrap().style, styled);
        set.inverse().apply(&mut doc);
        assert_eq!(doc.element(ElementId(8)).unwrap().style, Style::default());

        let hide = PlotCommand::SetVisibilityFlag {
            id: ElementId(8),
            before: true,
            after: false,
        };
        hide.apply(&mut doc);
        assert!(!doc.element(ElementId(8)).unwrap().flags.visible_manual);
        hide.inverse().apply(&mut doc);
        assert!(doc.element(ElementId(8)).unwrap().flags.visible_manual);
    }

    #[test]
    fn composite_undoes_back_to_front() {
        let mut doc = Document::with_default_layer();
        let layer = doc.active_layer().unwrap();
        for id in 1..=3u64 {
            let e = Element::new(ElementId(id), "p", Geometry::Point(p(0.0, 0.0)));
            doc.add_element_to_layer(layer, NewElement { id: ElementId(id), element: e });
        }
        let step = |id: u64| PlotCommand::SetVisibilityFlag {
            id: ElementId(id),
            before: true,
            after: false,
        };
        let group = PlotCommand::Composite {
            steps: vec![step(1), step(2), step(3)],
        };
        group.apply(&mut doc);
        for id in 1..=3u64 {
            assert!(!doc.element(ElementId(id)).unwrap().flags.visible_manual);
        }
        // targets() 跨叶子扁平化。
        assert_eq!(group.targets(), vec![ElementId(1), ElementId(2), ElementId(3)]);
        // 撤销该组会恢复每个叶子。
        group.inverse().apply(&mut doc);
        for id in 1..=3u64 {
            assert!(doc.element(ElementId(id)).unwrap().flags.visible_manual);
        }
    }
}
