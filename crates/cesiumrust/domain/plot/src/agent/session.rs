//! 面向 agent 的**编排门面**（计划 P3）。
//!
//! [`PlotSession`] 把 agent 要编辑的两块服务器状态 —— [`Document`] 及其
//! [`HistoryStack`] —— 打包在一个进程内入口之后。它是每个宿主（一个 Bevy
//! 桥、一个 CLI、一个未来的 MCP shim）共享的单一表面：它不拥有任何引擎专有
//! 的东西，不持有渲染器引用，因此可以完全无头单元测试。
//!
//! 该会话有意保持精简 —— 它转发到 [`action`](super::action) 写路径、
//! [`query`](super::query) 读路径与 [`schema`](super::schema) 文档 I/O。
//! 把它保持为一个薄薄的聚合器（而非重新实现逻辑）意味着纯函数仍是单一
//! 事实来源，那里被证明的不变量在此同样成立。

use crate::agent::action::{apply_action, ActionError, AgentAction, Applied};
use crate::agent::query::{query, ElementSummary, QueryFilter};
use crate::agent::schema::{export_document_json, import_document_json};
use crate::io::PlotIoError;
use crate::model::ids::ElementId;
use crate::model::{Document, ViewContext};
use crate::ops::HistoryStack;

/// 一个标绘文档加上它的 undo / redo 历史，通过单一的面向 agent API 驱动。
#[derive(Debug)]
pub struct PlotSession {
    /// 实时场景文档。公开，以便宿主可以直接读取 / 借用它
    /// （例如把它渲染出去）；每一次*变更*仍应经过 [`Self::apply`]，
    /// 从而保持可撤销且经过校验。
    pub doc: Document,
    /// 与 `doc` 保持同步锁定的 undo / redo 记录器。
    pub history: HistoryStack,
}

impl Default for PlotSession {
    /// 一个没有任何图层的空会话（一个 `Create` 必须先解析出一个图层）。
    fn default() -> Self {
        Self {
            doc: Document::default(),
            history: HistoryStack::new(),
        }
    }
}

impl PlotSession {
    /// 一个空会话（尚无图层 —— 添加一个或使用 [`Self::with_default_layer`]）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 一个预置了单个默认图层的会话，无需显式目标图层即可接受
    /// [`AgentAction::Create`]。
    pub fn with_default_layer() -> Self {
        Self {
            doc: Document::with_default_layer(),
            history: HistoryStack::new(),
        }
    }

    /// 校验、编译、应用并记录一个动作为单个 undo 步骤。
    /// 被拒绝的动作在校验时就短路，不变更任何东西。
    pub fn apply(&mut self, action: AgentAction) -> Result<Applied, ActionError> {
        apply_action(&mut self.doc, &mut self.history, &action)
    }

    /// 按顺序应用动作，每个都是它自己的 undo 步骤。在第一个错误处停止；
    /// 已应用的步骤保持已提交（它们是各自独立的 undo 步骤，因此宿主
    /// 可以一次 `undo` 一步地回滚它们）。
    pub fn apply_all(&mut self, actions: Vec<AgentAction>) -> Result<Vec<Applied>, ActionError> {
        let mut applied = Vec::with_capacity(actions.len());
        for action in actions {
            applied.push(self.apply(action)?);
        }
        Ok(applied)
    }

    /// 撤销最近的步骤，返回它触及的元素 id。
    pub fn undo(&mut self) -> Option<Vec<ElementId>> {
        self.history.undo(&mut self.doc)
    }

    /// 重做最近被撤销的步骤，返回触及的 id。
    pub fn redo(&mut self) -> Option<Vec<ElementId>> {
        self.history.redo(&mut self.doc)
    }

    /// 是否有可撤销的操作。
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// 是否有可重做的操作。
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// 已记录的 undo 步骤数（用于一个 "undo N" 交互）。
    pub fn undo_len(&self) -> usize {
        self.history.undo_len()
    }

    /// 待重做的 redo 步骤数。
    pub fn redo_len(&self) -> usize {
        self.history.redo_len()
    }

    /// 从一个 [`Self::export_doc`] 负载（或任何 GeoJSON
    /// `FeatureCollection`，尽最大努力导入）替换文档。清空历史，因为先前的
    /// 步骤引用的是被丢弃的文档。
    pub fn import_doc(&mut self, text: &str) -> Result<(), PlotIoError> {
        self.doc = import_document_json(text)?;
        self.history.clear();
        Ok(())
    }

    /// 序列化整个文档（无损 GeoJSON）。
    pub fn export_doc(&self) -> String {
        export_document_json(&self.doc)
    }

    /// 对当前文档的只读态势查询，在无视图上下文下求值：每个摘要的
    /// `visible` 只反映手动标志与图层开关。用 [`Self::query_with_view`]
    /// 可额外兼顾比例尺带与每模式显示标志。
    pub fn query(&self, filter: &QueryFilter) -> Vec<ElementSummary> {
        self.query_with_view(filter, None)
    }

    /// 同 [`Self::query`]，但额外传入一个可选的 [`ViewContext`]，使算出的
    /// `visible` 标志也尊重每个元素的比例尺带与每模式
    /// （`show_in_flat` / `show_in_globe`）显示标志 —— 即渲染器会应用的
    /// 同一视图状态。传 `None` 跳过那道门控。
    pub fn query_with_view(
        &self,
        filter: &QueryFilter,
        view: Option<&ViewContext>,
    ) -> Vec<ElementSummary> {
        query(&self.doc, filter, view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::action::StylePatch;
    use crate::geo::GeoPoint;
    use crate::ops::DrawKind;
    use serde_json::{Map, Value};

    fn create(lon: f64, name: &str) -> AgentAction {
        AgentAction::Create {
            kind: DrawKind::Point,
            positions: vec![GeoPoint::surface(lon, 0.0)],
            name: Some(name.into()),
            style: None,
            attributes: None,
            layer: None,
        }
    }

    #[test]
    fn scripted_session_end_to_end() {
        let mut s = PlotSession::with_default_layer();

        // 1. 构建三个元素，记住它们的 id。
        let a = s.apply(create(0.0, "alpha")).unwrap().new_id.unwrap();
        let b = s.apply(create(10.0, "bravo")).unwrap().new_id.unwrap();
        let c = s.apply(create(20.0, "charlie")).unwrap().new_id.unwrap();
        assert_eq!(s.doc.element_count(), 3);
        assert_eq!(s.history.undo_len(), 3);

        // 2. 移动其中一个。
        s.apply(AgentAction::Move {
            target: a,
            delta_lonlat: [5.0, 0.0],
        })
        .unwrap();
        match &s.doc.element(a).unwrap().geometry {
            crate::model::Geometry::Point(p) => assert_eq!((p.lon_deg, p.lat_deg), (5.0, 0.0)),
            other => panic!("{other:?}"),
        }

        // 3. 给另一个改样式 + 设置一个属性。
        s.apply(AgentAction::Style {
            target: b,
            patch: StylePatch {
                color: Some([1.0, 0.0, 0.0, 1.0]),
                ..Default::default()
            },
        })
        .unwrap();
        assert_eq!(s.doc.element(b).unwrap().style.color, [1.0, 0.0, 0.0, 1.0]);

        // 4. 查询看到全部三个（名称过滤器把它收窄）。
        let all = s.query(&QueryFilter::default());
        assert_eq!(all.len(), 3);
        let named = s.query(&QueryFilter {
            name_contains: Some("bravo".into()),
            ..Default::default()
        });
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].id, b);

        // 5. 撤销每一步直到为空。
        assert!(s.can_undo());
        while s.can_undo() {
            s.undo();
        }
        assert_eq!(s.doc.element_count(), 0);
        // 重做把它们全部走回。
        while s.can_redo() {
            s.redo();
        }
        assert_eq!(s.doc.element_count(), 3);
        assert_eq!(s.doc.element(c).unwrap().name, "charlie");
    }

    #[test]
    fn apply_all_stops_at_first_error_and_keeps_prefix() {
        let mut s = PlotSession::with_default_layer();
        let err = s
            .apply_all(vec![
                create(0.0, "ok"),
                AgentAction::Delete {
                    target: ElementId(999),
                },
                create(1.0, "never"),
            ])
            .unwrap_err();
        assert_eq!(err, ActionError::UnknownTarget(ElementId(999)));
        // 第一个 create 已提交；失败的 delete 与尾随的 create 从未运行。
        assert_eq!(s.doc.element_count(), 1);
        assert_eq!(s.history.undo_len(), 1);
    }

    #[test]
    fn export_import_roundtrips_the_session() {
        let mut s = PlotSession::with_default_layer();
        s.apply(create(0.0, "alpha")).unwrap();
        s.apply(create(5.0, "bravo")).unwrap();
        let text = s.export_doc();

        let mut s2 = PlotSession::new();
        s2.import_doc(&text).unwrap();
        assert_eq!(s2.doc.element_count(), 2);
        // 导入清空历史（步骤引用的是被丢弃的文档）。
        assert!(!s2.can_undo());
        // 且还原的文档等于导出的那个。
        assert_eq!(s2.doc, s.doc);
    }

    #[test]
    fn new_session_has_no_layer_so_create_needs_one() {
        let mut s = PlotSession::new();
        let err = s.apply(create(0.0, "orphan")).unwrap_err();
        assert_eq!(err, ActionError::NoActiveLayer);
        // 在一个图层都不存在时无法显式提供图层，所以
        // with_default_layer 构造器是便捷路径。
        assert_eq!(s.doc.element_count(), 0);
    }

    #[test]
    fn query_visible_defaults_true_without_view() {
        let mut s = PlotSession::with_default_layer();
        let id = s.apply(create(0.0, "v")).unwrap().new_id.unwrap();
        let rows = s.query(&QueryFilter::default());
        assert_eq!(rows.len(), 1);
        assert!(rows[0].visible, "manual + layer both default to visible");

        let mut merge = Map::new();
        merge.insert("kind".into(), Value::String("recon".into()));
        s.apply(AgentAction::SetAttributes {
            target: id,
            merge,
        })
        .unwrap();
        s.apply(AgentAction::SetVisible {
            target: id,
            visible: false,
        })
        .unwrap();
        assert!(!s.query(&QueryFilter::default())[0].visible);
    }
}
