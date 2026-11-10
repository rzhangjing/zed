//! 面向 agent 的**意图协议**（计划 P0）。
//!
//! 一个外部 agent（LLM 工具调用者、脚本、服务）从不触及渲染器、
//! 顶点或 GPU —— 它产生一个小巧、稳定、可 JSON 序列化的
//! [`AgentAction`]。[`compile`] 通过填入服务器拥有的、agent 必须
//! *不能* 猜测的事实，将一个动作编译为可逆的 [`PlotCommand`]：
//! 它从实时 [`Document`] 读到的 `before` 状态、新铸造的元素 id，
//! 以及解析后的目标图层。[`apply_action`] 随后作为单个 undo 步骤
//! 应用 + 记录该命令。
//!
//! 为何用一个外障而非直接暴露 [`PlotCommand`]：命令携带
//! `before` / `id` / `layer`，是一个内部的记录单元，会将实现
//! 细节冻结进公共契约。意图层使外部保持稳定而内部演进，且它是
//! 意图层使外部保持稳定而内部演进，且它是
//! 每一次写入在触及文档前被*校验*（未知 id、点太少、无活动图层）的地方。
//!
//! ## 典型流程
//! [`apply_action`] 先 [`compile`]：校验意图、读取 `before` 快照、
//! 铸造新 id、解析目标图层，产出一个 [`PlotCommand`]；随后把命令
//! `apply` 到文档并记入 [`HistoryStack`]，一次动作恰对应一个 undo 步骤。
//! 若校验失败，命令永不生成，文档保持原状（除一个可能被消耗的 id 计数器）。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::geo::GeoPoint;
use crate::model::geometry::Geometry;
use crate::model::ids::{ElementId, LayerId};
use crate::model::{Document, HeightReference, Rgba, Style};
use crate::ops::transform;
use crate::ops::{commit_draft, DrawKind, HistoryStack, PlotCommand};

/// 一个最小的、渐进式的样式编辑 —— 每个字段都可选，只有存在的那些
/// 会被 [`StylePatch::apply_to`] 复制到元素当前的 [`Style`] 上。
/// 镜像 `Style` 中与 agent 相关的子集。
///
/// 补丁采用“只设给出字段”的语义，以便 agent 表达局部编辑意图。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StylePatch {
    /// 线/轮廓主色（RGBA，各通道 0.0..1.0）；`Some(_)` 覆盖，缺省保留原值。
    pub color: Option<Rgba>,
    /// 整体不透明度（0.0 全透..1.0 不透明）。
    pub opacity: Option<f32>,
    /// 线宽（像素）。
    pub width_px: Option<f32>,
    /// 多边形填充色；`Some(_)` 设置它，`None` 保持不变。
    pub fill: Option<Rgba>,
    /// 点标记的像素尺寸。
    pub point_size_px: Option<f32>,
    /// 绘制层级，越大越靠上层。
    pub z_order: Option<i32>,
    /// 高度参考基准（相对地面/绝对等）。
    pub height_reference: Option<HeightReference>,
    /// 是否启用深度测试以被地形遮挡。
    pub depth_test: Option<bool>,
    /// 平面模式下是否显示该元素。
    pub show_in_flat: Option<bool>,
    /// 球面模式下是否显示该元素。
    pub show_in_globe: Option<bool>,
}

impl StylePatch {
    /// 将本补丁折叠到 `base` 上，返回新样式。
    pub fn apply_to(&self, base: &Style) -> Style {
        // 从 base 出发逐字段折叠：仅覆盖补丁中显式给出的属性，其余沿用原值。
        let mut s = base.clone();
        if let Some(v) = self.color {
            s.color = v;
        }
        if let Some(v) = self.opacity {
            s.opacity = v;
        }
        if let Some(v) = self.width_px {
            s.width_px = v;
        }
        if let Some(v) = self.fill {
            s.fill = Some(v);
        }
        if let Some(v) = self.point_size_px {
            s.point_size_px = v;
        }
        if let Some(v) = self.z_order {
            s.z_order = v;
        }
        if let Some(v) = self.height_reference {
            s.height_reference = v;
        }
        if let Some(v) = self.depth_test {
            s.depth_test = v;
        }
        if let Some(v) = self.show_in_flat {
            s.show_in_flat = v;
        }
        if let Some(v) = self.show_in_globe {
            s.show_in_globe = v;
        }
        s
    }
}

/// 单个高层编辑意图。它有意**不**携带任何服务器拥有的事实
/// （无 `before`、无铸造的 id、无解析的图层）：[`compile`] 提供
/// 这些并校验请求。
///
/// 每个变体对应一类高层意图，由 [`compile`] 翻译为底层可逆命令。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentAction {
    /// 从一个绘制类型 + 其控制点创建一个图元，折叠经过交互绘制
    /// 工具所用的同一 [`commit_draft`] 规则。更丰富的几何用后续
    /// [`AgentAction::SetGeometry`] 放置。
    Create {
        /// 绘制的图元类型（点/线/面等）。
        kind: DrawKind,
        /// 控制点序列，按 `kind` 的最小点数校验后提交为几何。
        positions: Vec<GeoPoint>,
        /// 元素可读名称；缺省为空串。
        #[serde(default)]
        name: Option<String>,
        /// 初始样式补丁，折叠到默认样式之上。
        #[serde(default)]
        style: Option<StylePatch>,
        /// 自由形式业务属性（敌我/番号/状态等）。
        #[serde(default)]
        attributes: Option<Map<String, Value>>,
        /// 目标图层；默认为文档的活动图层。
        #[serde(default)]
        layer: Option<LayerId>,
    },
    /// 按 id 删除一个元素（在命令中保留完整以便 undo 恢复它）。
    Delete { target: ElementId },
    /// 将元素的所有顶点平移 `(dlon, dlat)` 度。
    Move {
        /// 被平移元素的 id。
        target: ElementId,
        /// 经/纬方向的平移量（度）。
        delta_lonlat: [f64; 2],
    },
    /// 替换一个元素的整个几何（任意 [`Geometry`] 变体）。
    SetGeometry {
        /// 目标元素 id。
        target: ElementId,
        /// 用于整体替换的新几何。
        geometry: Geometry,
    },
    /// 修补一个元素的样式（只有存在的字段会改变）。
    Style {
        /// 目标元素 id。
        target: ElementId,
        /// 要折叠到现有样式上的补丁。
        patch: StylePatch,
    },
    /// 合并自由形式的业务属性（敌我 / 番号 / 状态 …）；同名的现有键
    /// 被覆盖，其他保留。
    SetAttributes {
        /// 目标元素 id。
        target: ElementId,
        /// 要并入的属性映射，同名键覆盖、其余保留。
        merge: Map<String, Value>,
    },
    /// 翻转手动可见性标志。
    SetVisible {
        /// 目标元素 id。
        target: ElementId,
        /// 期望的手动可见性标志。
        visible: bool,
    },
    /// 多个动作编译成一个 [`PlotCommand::Composite`] —— 单个
    /// undo 步骤。子动作必须引用已存在的元素（一个批次不会
    /// 解析指向同批次中早前子动作所创建元素的引用）；要那样做请用
    /// 顺序的 [`apply_action`] 调用。
    Batch { actions: Vec<AgentAction> },
}

/// 一个动作无法编译的原因。每个失败都在 [`compile`] 中、*在*
/// 文档被变更之前检测到，因此一个被拒绝的动作会使场景保持
/// 不变（除了一个隐式消耗的 id 计数器）。
///
/// 错误携带足够上下文，供 agent 据此修正意图后重试。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    #[error("no element with id {0:?}")]
    UnknownTarget(ElementId),
    #[error("{kind:?} needs at least {need} points, got {got}")]
    TooFewPoints {
        /// 所请求的绘制类型。
        kind: DrawKind,
        /// 该类型所需的最小点数。
        need: usize,
        /// 实际提供的点数。
        got: usize,
    },
    #[error("no active layer to add into — set one or create a layer first")]
    NoActiveLayer,
    #[error("geometry rejected by validation: {0}")]
    InvalidGeometry(String),
    #[error("value out of range for field `{field}`")]
    OutOfRange { field: &'static str },
}

/// 一个已应用动作做了什么 —— 足够让宿主调和它的视图。
///
/// `touched_ids` 列出受影响叶子元素，`new_id` 仅当创建了元素时非空。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Applied {
    /// 结果命令触及的每个叶子元素 id。
    pub touched_ids: Vec<ElementId>,
    /// 当一个动作创建了新元素时，那个新建元素的 id。
    pub new_id: Option<ElementId>,
}

/// 针对 `doc` 校验 `action` 并产出它映射到的可逆命令 —— 但**不**应用
/// 它。这会读取 `before` 状态、解析目标图层，并（对于创建）通过
/// [`Document::make_element`] 铸造一个新 id。
///
/// 因此一个仅做编译的调用不会改动文档内容，只可能推进 id 计数器。
pub fn compile(doc: &mut Document, action: &AgentAction) -> Result<PlotCommand, ActionError> {
    match action {
        AgentAction::Create {
            kind,
            positions,
            name,
            style,
            attributes,
            layer,
        } => {
            // 图层缺省取文档活动图层，两者都缺则无目标可写入。
            let target = (*layer)
                .or_else(|| doc.active_layer())
                .ok_or(ActionError::NoActiveLayer)?;
            // 用与交互绘制相同的 draft 提交规则生成几何，点数不足即失败。
            let geometry = commit_draft(*kind, positions).ok_or(ActionError::TooFewPoints {
                kind: *kind,
                need: kind.min_points(),
                got: positions.len(),
            })?;
            // 铸造新元素 id 并写入名称与几何。
            let mut ne = doc.make_element(name.clone().unwrap_or_default(), geometry);
            if let Some(patch) = style {
                // 若给了样式补丁，折叠到默认样式之上。
                ne.element.style = patch.apply_to(&ne.element.style);
            }
            if let Some(attrs) = attributes {
                // 逐个并入自由形式的业务属性。
                for (k, v) in attrs {
                    ne.element.attributes.insert(k.clone(), v.clone());
                }
            }
            // 组装为新增元素命令：图层与铸造的 id 由服务器侧决定。
            Ok(PlotCommand::AddElement {
                layer: target,
                element: Box::new(ne.element),
            })
        }
        AgentAction::Delete { target } => {
            // 先取出待删元素的完整副本，以便 undo 时原样恢复。
            let el = doc
                .element(*target)
                .ok_or(ActionError::UnknownTarget(*target))?
                .clone();
            // 解析元素当前所属图层。
            let layer = doc
                .element_context(*target)
                .map(|(l, _)| l)
                .ok_or(ActionError::UnknownTarget(*target))?;
            // 携带完整副本与所属图层，以便 undo 将元素放回原图层。
            Ok(PlotCommand::RemoveElement {
                element: Box::new(el),
                layer,
            })
        }
        AgentAction::Move {
            target,
            delta_lonlat,
        } => {
            // 读取当前几何作为 before。
            let before = doc
                .element(*target)
                .ok_or(ActionError::UnknownTarget(*target))?
                .geometry
                .clone();
            // 按给定的经/纬增量平移得到 after。
            let after = transform::translate(&before, delta_lonlat[0], delta_lonlat[1]);
            Ok(PlotCommand::UpdateGeometry {
                id: *target,
                before: Box::new(before),
                after: Box::new(after),
            })
        }
        AgentAction::SetGeometry { target, geometry } => {
            // 记录旧几何，将整体替换为新提供的任意几何变体。
            let before = doc
                .element(*target)
                .ok_or(ActionError::UnknownTarget(*target))?
                .geometry
                .clone();
            Ok(PlotCommand::UpdateGeometry {
                id: *target,
                before: Box::new(before),
                after: Box::new(geometry.clone()),
            })
        }
        AgentAction::Style { target, patch } => {
            // 取现有样式作 before。
            let before = doc
                .element(*target)
                .ok_or(ActionError::UnknownTarget(*target))?
                .style
                .clone();
            // 把补丁折叠到 before 上得到 after。
            let after = patch.apply_to(&before);
            Ok(PlotCommand::SetStyle {
                id: *target,
                before: Box::new(before),
                after: Box::new(after),
            })
        }
        AgentAction::SetAttributes { target, merge } => {
            // 复制旧属性并合并新键，同名键覆盖。
            let before = doc
                .element(*target)
                .ok_or(ActionError::UnknownTarget(*target))?
                .attributes
                .clone();
            let mut after = before.clone();
            for (k, v) in merge {
                after.insert(k.clone(), v.clone());
            }
            Ok(PlotCommand::SetAttributes {
                id: *target,
                before,
                after,
            })
        }
        AgentAction::SetVisible { target, visible } => {
            // 读取手动可见性标志作 before。
            let before = doc
                .element(*target)
                .ok_or(ActionError::UnknownTarget(*target))?
                .flags
                .visible_manual;
            // 只翻转手动可见标志，before/after 均为布尔。
            Ok(PlotCommand::SetVisibilityFlag {
                id: *target,
                before,
                after: *visible,
            })
        }
        AgentAction::Batch { actions } => {
            // 逐个编译子动作，任一失败即整体中止。
            let mut steps = Vec::with_capacity(actions.len());
            for a in actions {
                steps.push(compile(doc, a)?);
            }
            // 汇成单个复合命令，对应一次 undo。
            Ok(PlotCommand::Composite { steps })
        }
    }
}

/// 在 `doc` 上编译、应用并记录 `action` 作为单个 undo 步骤。校验
/// 失败会在任何变更之前短路，且**不会**被记录。
pub fn apply_action(
    doc: &mut Document,
    history: &mut HistoryStack,
    action: &AgentAction,
) -> Result<Applied, ActionError> {
    // 先编译校验；失败即短路，不改动文档也不记录。
    let command = compile(doc, action)?;
    // 创建类命令回报新元素 id，其余无。
    let new_id = match &command {
        PlotCommand::AddElement { element, .. } => Some(element.id),
        _ => None,
    };
    // 应用命令、收集受影响 id，并作为一步记入历史。
    command.apply(doc);
    let touched_ids = command.targets();
    // 记入历史栈，一次动作恰对应一条 undo 记录。
    history.record(command);
    Ok(Applied {
        touched_ids,
        new_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ids::ElementId;

    // 构造一个位于地表（高度 0）的地理点。
    fn pt(lon: f64, lat: f64) -> GeoPoint {
        GeoPoint::surface(lon, lat)
    }

    // 创建一个带默认图层、可直接写入的文档。
    fn doc() -> Document {
        Document::with_default_layer()
    }

    /// 创建应新增一个元素，单次 undo 后再度移除。
    #[test]
    fn create_adds_and_undo_removes() {
        // 默认文档带一个活动图层，可直接写入。
        let mut doc = doc();
        let mut h = HistoryStack::new();
        let applied = apply_action(
            &mut doc,
            &mut h,
            &AgentAction::Create {
                kind: DrawKind::Point,
                positions: vec![pt(3.0, 4.0)],
                name: Some("watch".into()),
                style: None,
                attributes: None,
                layer: None,
            },
        )
        .unwrap();
        // 新建元素的 id 应回报于 Applied，且名称写入成功。
        let id = applied.new_id.expect("created id reported");
        assert_eq!(doc.element_count(), 1);
        assert_eq!(doc.element(id).unwrap().name, "watch");
        // 单次 undo 会再次移除它。
        h.undo(&mut doc);
        assert_eq!(doc.element_count(), 0);
    }

    /// 创建时折叠样式补丁与属性到新建元素。
    #[test]
    fn create_applies_style_and_attributes() {
        // 先备好一个自由属性（side=hostile）。
        let mut doc = doc();
        let mut h = HistoryStack::new();
        let mut attrs = Map::new();
        attrs.insert("side".into(), Value::String("hostile".into()));
        // 只给颜色补丁，其余字段沿用默认样式。
        let applied = apply_action(
            &mut doc,
            &mut h,
            &AgentAction::Create {
                kind: DrawKind::Point,
                positions: vec![pt(0.0, 0.0)],
                name: None,
                style: Some(StylePatch {
                    color: Some([1.0, 0.0, 0.0, 1.0]),
                    ..Default::default()
                }),
                attributes: Some(attrs),
                layer: None,
            },
        )
        .unwrap();
        let e = doc.element(applied.new_id.unwrap()).unwrap();
        assert_eq!(e.style.color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(e.attributes["side"], Value::String("hostile".into()));
        // 未传 name 时元素名为空串。
        assert_eq!(e.name, "");
    }

    /// 平移后 undo 应恢复原几何坐标。
    #[test]
    fn move_then_undo_restores_geometry() {
        // 先创建一个位于 (1,1) 的点元素。
        let mut doc = doc();
        let mut h = HistoryStack::new();
        let id = apply_action(
            &mut doc,
            &mut h,
            &AgentAction::Create {
                kind: DrawKind::Point,
                positions: vec![pt(1.0, 1.0)],
                name: None,
                style: None,
                attributes: None,
                layer: None,
            },
        )
        .unwrap()
        .new_id
        .unwrap();
        apply_action(
            &mut doc,
            &mut h,
            &AgentAction::Move {
                target: id,
                delta_lonlat: [2.0, -3.0],
            },
        )
        .unwrap();
        // 平移量 [2,-3] 应把 (1,1) 移到 (3,-2)。
        match &doc.element(id).unwrap().geometry {
            Geometry::Point(p) => assert_eq!((p.lon_deg, p.lat_deg), (3.0, -2.0)),
            g => panic!("{g:?}"),
        }
        h.undo(&mut doc);
        match &doc.element(id).unwrap().geometry {
            Geometry::Point(p) => assert_eq!((p.lon_deg, p.lat_deg), (1.0, 1.0)),
            g => panic!("{g:?}"),
        }
    }

    /// 置几何与改样式都应是可 undo 的独立步骤。
    #[test]
    fn set_geometry_and_style_are_undoable() {
        // 先创建一个点元素作为后续改写对象。
        let mut doc = doc();
        let mut h = HistoryStack::new();
        let id = apply_action(
            &mut doc,
            &mut h,
            &AgentAction::Create {
                kind: DrawKind::Point,
                positions: vec![pt(0.0, 0.0)],
                name: None,
                style: None,
                attributes: None,
                layer: None,
            },
        )
        .unwrap()
        .new_id
        .unwrap();
        // 将几何设为一条折线。
        apply_action(
            &mut doc,
            &mut h,
            &AgentAction::SetGeometry {
                target: id,
                geometry: Geometry::Polyline(crate::model::geometry::Polyline {
                    positions: vec![pt(0.0, 0.0), pt(1.0, 1.0)],
                }),
            },
        )
        .unwrap();
        // 点几何改为折线后，kind 应变为 Line。
        assert_eq!(doc.element(id).unwrap().geometry.kind(), crate::model::GeometryKind::Line);
        h.undo(&mut doc);
        assert_eq!(doc.element(id).unwrap().geometry.kind(), crate::model::GeometryKind::Point);
        // 样式补丁颜色，然后 undo。
        apply_action(
            &mut doc,
            &mut h,
            &AgentAction::Style {
                target: id,
                patch: StylePatch {
                    width_px: Some(9.0),
                    ..Default::default()
                },
            },
        )
        .unwrap();
        // 样式补丁只改宽，undo 后应回到非 9.0 的原值。
        assert_eq!(doc.element(id).unwrap().style.width_px, 9.0);
        h.undo(&mut doc);
        assert_ne!(doc.element(id).unwrap().style.width_px, 9.0);
    }

    /// 合并属性只新增键，undo 后丢弃新键保留旧键。
    #[test]
    fn set_attributes_merges_and_undo_restores() {
        // 先创建一个带属性 a 的元素。
        let mut doc = doc();
        let mut h = HistoryStack::new();
        let id = apply_action(
            &mut doc,
            &mut h,
            &AgentAction::Create {
                kind: DrawKind::Point,
                positions: vec![pt(0.0, 0.0)],
                name: None,
                style: None,
                attributes: Some({
                    let mut m = Map::new();
                    m.insert("a".into(), Value::from(1));
                    m
                }),
                layer: None,
            },
        )
        .unwrap()
        .new_id
        .unwrap();
        apply_action(
            &mut doc,
            &mut h,
            &AgentAction::SetAttributes {
                target: id,
                merge: {
                    let mut m = Map::new();
                    m.insert("b".into(), Value::from(2));
                    m
                },
            },
        )
        .unwrap();
        // 合并后 a、b 两键应共存。
        let e = doc.element(id).unwrap();
        assert_eq!(e.attributes["a"], Value::from(1));
        assert_eq!(e.attributes["b"], Value::from(2));
        h.undo(&mut doc);
        // Undo 丢弃合并的键但保留原始的。
        let e = doc.element(id).unwrap();
        assert!(e.attributes.contains_key("a"));
        // undo 后合并进去的新键 b 应被丢弃。
        assert!(!e.attributes.contains_key("b"));
    }

    /// 翻转手动可见性后可 undo 回原值。
    #[test]
    fn set_visible_roundtrips() {
        // 先创建一个默认可见的点元素。
        let mut doc = doc();
        let mut h = HistoryStack::new();
        let id = apply_action(
            &mut doc,
            &mut h,
            &AgentAction::Create {
                kind: DrawKind::Point,
                positions: vec![pt(0.0, 0.0)],
                name: None,
                style: None,
                attributes: None,
                layer: None,
            },
        )
        .unwrap()
        .new_id
        .unwrap();
        // 将手动可见置为 false。
        apply_action(&mut doc, &mut h, &AgentAction::SetVisible { target: id, visible: false })
            .unwrap();
        assert!(!doc.element(id).unwrap().flags.visible_manual);
        h.undo(&mut doc);
        assert!(doc.element(id).unwrap().flags.visible_manual);
    }

    /// 删除元素后 undo 应连同其几何原样恢复。
    #[test]
    fn delete_then_undo_restores_the_element() {
        // 先创建一个位于 (5,6) 的点，供删除与恢复验证。
        let mut doc = doc();
        let mut h = HistoryStack::new();
        let id = apply_action(
            &mut doc,
            &mut h,
            &AgentAction::Create {
                kind: DrawKind::Point,
                positions: vec![pt(5.0, 6.0)],
                name: None,
                style: None,
                attributes: None,
                layer: None,
            },
        )
        .unwrap()
        .new_id
        .unwrap();
        // 删除后元素应不可达。
        apply_action(&mut doc, &mut h, &AgentAction::Delete { target: id }).unwrap();
        assert!(doc.element(id).is_none());
        h.undo(&mut doc);
        let back = doc.element(id).expect("restored by undo");
        match &back.geometry {
            Geometry::Point(p) => assert_eq!((p.lon_deg, p.lat_deg), (5.0, 6.0)),
            g => panic!("{g:?}"),
        }
    }

    /// 批处理应作为单一 undo 步骤一次性应用与回退。
    #[test]
    fn batch_is_a_single_undo_step() {
        // 构造一个在指定经度上创建点的闭包，便于批内复用。
        let mut doc = doc();
        let mut h = HistoryStack::new();
        let mk = |lon: f64| AgentAction::Create {
            kind: DrawKind::Point,
            positions: vec![pt(lon, 0.0)],
            name: None,
            style: None,
            attributes: None,
            layer: None,
        };
        apply_action(
            &mut doc,
            &mut h,
            &AgentAction::Batch {
                actions: vec![mk(1.0), mk(2.0), mk(3.0)],
            },
        )
        .unwrap();
        assert_eq!(doc.element_count(), 3);
        // 批处理只留下一条 undo 记录。
        assert_eq!(h.undo_len(), 1, "batch is one undo step");
        h.undo(&mut doc);
        assert_eq!(doc.element_count(), 0);
    }

    /// 目标 id 未知时编译失败且不改变文档、不记历史。
    #[test]
    fn unknown_target_errors_without_mutation() {
        let mut doc = doc();
        let mut h = HistoryStack::new();
        let err = apply_action(&mut doc, &mut h, &AgentAction::Delete { target: ElementId(999) })
            .unwrap_err();
        // 失败的动作不应改变文档，也不应可 undo。
        assert_eq!(err, ActionError::UnknownTarget(ElementId(999)));
        assert_eq!(doc.element_count(), 0);
        assert!(!h.can_undo(), "a failed action is not recorded");
    }

    /// 点数不足时编译报 TooFewPoints，携带所需与实际数量。
    #[test]
    fn too_few_points_errors() {
        // 多边形至少需 3 点，这里只给 2 点。
        let mut doc = doc();
        let mut h = HistoryStack::new();
        let err = apply_action(
            &mut doc,
            &mut h,
            &AgentAction::Create {
                kind: DrawKind::Polygon,
                positions: vec![pt(0.0, 0.0), pt(1.0, 1.0)],
                name: None,
                style: None,
                attributes: None,
                layer: None,
            },
        )
        .unwrap_err();
        assert_eq!(
            err,
            ActionError::TooFewPoints {
                kind: DrawKind::Polygon,
                need: 3,
                got: 2
            }
        );
        assert_eq!(doc.element_count(), 0);
        assert!(!h.can_undo());
    }

    /// 无任何活动图层时创建报 NoActiveLayer。
    #[test]
    fn no_active_layer_errors() {
        // 一个完全没有图层的文档。
        let mut doc = Document::default();
        let mut h = HistoryStack::new();
        // 无图层文档上创建应报 NoActiveLayer。
        let err = apply_action(
            &mut doc,
            &mut h,
            &AgentAction::Create {
                kind: DrawKind::Point,
                positions: vec![pt(0.0, 0.0)],
                name: None,
                style: None,
                attributes: None,
                layer: None,
            },
        )
        .unwrap_err();
        assert_eq!(err, ActionError::NoActiveLayer);
    }

    /// 动作应能通过 JSON 无损往返序列化。
    #[test]
    fn actions_roundtrip_through_json() {
        // 一个携带样式与名称的创建动作。
        let a = AgentAction::Create {
            kind: DrawKind::Polyline,
            positions: vec![pt(0.0, 0.0), pt(1.0, 2.0)],
            name: Some("route".into()),
            style: Some(StylePatch {
                color: Some([0.0, 1.0, 0.0, 1.0]),
                ..Default::default()
            }),
            attributes: None,
            layer: None,
        };
        // 序列化后再反序列化应与原动作相等。
        let s = serde_json::to_string(&a).unwrap();
        let back: AgentAction = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }
}
