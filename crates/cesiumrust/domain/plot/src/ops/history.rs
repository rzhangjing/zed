//! 撤销 / 重做历史栈（计划 §8 / §14）：一个纯、与视图无关的
//! [`PlotCommand`] 记录器，桥接层从它的编辑手势和键盘快捷键驱动。
//!
//! 这个栈有意设计得很笨，以便极易测试：它存储正向命令，
//! 在 `undo` 时应用它们的逆命令，且从不检查 [`Document`] 本身。
//! `record` 会清空重做分支（一次撤销后的新编辑会使重做尾部失效），
//! 符合每个桌面编辑器的契约。

use crate::model::document::Document;

use super::command::PlotCommand;

/// 一个无界（由一个可选上限约束）的撤销 / 重做栈。
#[derive(Debug, Clone, Default)]
pub struct HistoryStack {
    /// 正向命令栈（最近应用的在栈顶），undo 从这里弹出。
    undo: Vec<PlotCommand>,
    /// 已被撤销的命令栈，redo 从这里弹出重新应用。
    redo: Vec<PlotCommand>,
    /// `Some(n)` 将撤销分支修剪为最新的 `n` 个条目；`None` = 无限
    /// （一次标绘会话的内存占用小到可以保留）。
    cap: Option<usize>,
}

impl HistoryStack {
    /// 一个无限的栈。
    pub fn new() -> Self {
        Self::default()
    }

    /// 一个至多保留 `cap` 个撤销步骤的栈。
    pub fn with_cap(cap: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            cap: Some(cap),
        }
    }

    /// 记录一个刚应用的命令，丢弃重做分支。
    pub fn record(&mut self, command: PlotCommand) {
        self.undo.push(command);
        // 一次新编辑使重做分支失效（每个桌面编辑器的契约）。
        self.redo.clear();
        // 超出上限时从栈底丢弃最旧的撤销步骤。
        if let Some(cap) = self.cap {
            while self.undo.len() > cap {
                self.undo.remove(0);
            }
        }
    }

    /// 当至少有一个命令可撤销时为 true。
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// 当至少有一个命令可重做时为 true。
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// 已记录的撤销步骤数（用于一个“撤销 N”标签 / 测试）。
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// 待处理的重做步骤数。
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// 撤销最近的命令：应用它的逆命令，将其移到重做
    /// 分支。返回该命令触及的 id，以便桥接层调和
    /// 那些视觉效果；栈为空时返回 `None`。
    pub fn undo(&mut self, doc: &mut Document) -> Option<Vec<crate::model::ids::ElementId>> {
        let command = self.undo.pop()?;
        // 应用逆命令回退，并把原命令转入重做分支。
        let inverse = command.inverse();
        inverse.apply(doc);
        let targets = command.targets();
        self.redo.push(command);
        Some(targets)
    }

    /// 重做最近被撤销的命令：重新应用它，将其移回撤销
    /// 分支。返回触及的 id；无可重做项时返回 `None`。
    pub fn redo(&mut self, doc: &mut Document) -> Option<Vec<crate::model::ids::ElementId>> {
        let command = self.redo.pop()?;
        // 重新应用命令，将其移回撤销分支。
        command.apply(doc);
        let targets = command.targets();
        self.undo.push(command);
        Some(targets)
    }

    /// 遗忘所有历史（例如在加载一个新文档之后）。
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::GeoPoint;
    use crate::model::geometry::Geometry;
    use crate::model::ids::{ElementId, LayerId};
    use crate::model::{Document, Element};

    fn p(lon: f64, lat: f64) -> GeoPoint {
        GeoPoint::surface(lon, lat)
    }

    /// 构造一个向指定图层添加点的 AddElement 命令（测试用）。
    fn add(id: u64, layer: LayerId) -> PlotCommand {
        PlotCommand::AddElement {
            layer,
            element: Box::new(Element::new(ElementId(id), "pt", Geometry::Point(p(0.0, 0.0)))),
        }
    }

    /// 镜像桥接层契约：将一个命令应用到文档，然后记录
    /// 它，使其可撤销。
    fn commit(h: &mut HistoryStack, doc: &mut Document, command: PlotCommand) {
        command.apply(doc);
        h.record(command);
    }

    /// 初始栈为空：既不可撤销也不可重做。
    #[test]
    fn starts_empty() {
        let h = HistoryStack::new();
        assert!(!h.can_undo());
        assert!(!h.can_redo());
    }

    /// 撤销 / 重做沿栈前后行走，文档状态随之回退与恢复。
    #[test]
    fn undo_redo_walks_the_stack() {
        let mut doc = Document::with_default_layer();
        let layer = doc.active_layer().unwrap();
        let mut h = HistoryStack::new();

        commit(&mut h, &mut doc, add(1, layer));
        commit(&mut h, &mut doc, add(2, layer));
        assert_eq!(doc.element_count(), 2);
        assert!(h.can_undo() && !h.can_redo());

        // 撤销第二个 add → 元素 2 消失，它变为可重做。
        let touched = h.undo(&mut doc).unwrap();
        assert_eq!(touched, vec![ElementId(2)]);
        assert!(doc.element(ElementId(2)).is_none());
        assert!(h.can_undo() && h.can_redo());

        // 撤销第一个 → 文档为空。
        h.undo(&mut doc);
        assert!(doc.element(ElementId(1)).is_none());
        assert!(!h.can_undo() && h.can_redo());

        // 按原始顺序重做两个。
        h.redo(&mut doc);
        h.redo(&mut doc);
        assert!(doc.element(ElementId(1)).is_some());
        assert!(doc.element(ElementId(2)).is_some());
        assert!(h.can_undo() && !h.can_redo());
    }

    /// 撤销后的一次新记录会清空重做尾部，使重做不可用。
    #[test]
    fn recording_after_undo_drops_the_redo_branch() {
        let mut doc = Document::with_default_layer();
        let layer = doc.active_layer().unwrap();
        let mut h = HistoryStack::new();
        commit(&mut h, &mut doc, add(1, layer));
        h.undo(&mut doc);
        assert!(h.can_redo());
        // 一次新编辑会清除重做尾部。
        commit(&mut h, &mut doc, add(3, layer));
        assert!(!h.can_redo());
        // 被撤销的 add1 使撤销分支变空；add3 现在是唯一的步骤。
        assert_eq!(h.undo_len(), 1);
    }

    /// 上限会将最旧的撤销步骤修剪掉，只保留最新的 cap 个。
    #[test]
    fn cap_trims_the_oldest_undos() {
        let doc = Document::with_default_layer();
        let layer = doc.active_layer().unwrap();
        let mut h = HistoryStack::with_cap(2);
        h.record(add(1, layer));
        h.record(add(2, layer));
        h.record(add(3, layer));
        assert_eq!(h.undo_len(), 2);
    }

    /// 在空栈上撤销 / 重做是安全的空操作（返回 None）。
    #[test]
    fn undo_on_empty_stack_is_a_noop() {
        let mut doc = Document::with_default_layer();
        let mut h = HistoryStack::new();
        assert!(h.undo(&mut doc).is_none());
        assert!(h.redo(&mut doc).is_none());
    }
}
