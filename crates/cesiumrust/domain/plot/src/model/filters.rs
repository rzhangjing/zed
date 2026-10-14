//! 用户切换的各个独立可见性“维度”（计划 §10）。每个维度
//! 都在 `eval_visibility` 内以逻辑 AND 折叠；它们全部默认为透传，
//! 因此一个裸文档会完整渲染。

use std::collections::{BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use super::geometry::GeometryKind;
use super::ids::ElementId;

/// 由 `crate::visibility::eval_visibility` 逐元素评估的过滤器开关。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Filters {
    /// §10.1 覆盖层总开关。`false` 隐藏整个覆盖层。
    pub overlay_enabled: bool,
    /// §10.5 类型维度。`None` = 每种类型都通过；`Some(set)` 仅允许
    /// 列表中的类型。
    pub enabled_types: Option<HashSet<GeometryKind>>,
    /// §10.8 选中聚焦。为 `true` 时，只显示 `selected` 中存在的 id
    /// （其余在 M1 中被视为隐藏，而非仅淡化）。
    pub only_selected: bool,
    /// 当前选中集，当 `only_selected` 设置时被查阅。
    pub selected: BTreeSet<ElementId>,
    /// §10.9 属性维度。一组 `key == value` 谓词；一个
    /// 元素仅当满足**全部**谓词时才通过（AND）。空 = 通过。
    pub attribute_predicates: Vec<(String, serde_json::Value)>,
}

impl Filters {
    /// 全部开启：覆盖层启用、无类型限制、无选中聚焦、
    /// 无属性谓词。
    pub fn none() -> Self {
        Self::default()
    }

    /// 一个总开关为开的过滤器集（默认的 `none()` 其
    /// `overlay_enabled == false`，因为 `Default` 是全 false）。
    pub fn enabled() -> Self {
        Self {
            overlay_enabled: true,
            ..Self::default()
        }
    }

    /// 切换 §10.1 覆盖层总开关。
    pub fn toggle_overlay(&mut self) {
        self.overlay_enabled = !self.overlay_enabled;
    }

    /// `kind` 是否通过类型维度（§10.5）。`None`（无限制）
    /// 意味着每种类型都开启。
    pub fn type_enabled(&self, kind: GeometryKind) -> bool {
        match &self.enabled_types {
            None => true,
            Some(set) => set.contains(&kind),
        }
    }

    /// 翻转一个类型开关。`None`（全开）的限制会先物化为
    /// 完整集，以便能单独关掉某一种类型。
    pub fn toggle_type(&mut self, kind: GeometryKind) {
        let set = self
            .enabled_types
            .get_or_insert_with(|| GeometryKind::ALL.iter().copied().collect());
        if !set.remove(&kind) {
            set.insert(kind);
        }
    }

    /// 切换 §10.8 选中聚焦开关。
    pub fn toggle_only_selected(&mut self) {
        self.only_selected = !self.only_selected;
    }

    /// 添加或替换一个 §10.9 `key == value` 属性谓词。键是
    /// 唯一的：重新添加一个已存在的键会就地覆盖其值。
    pub fn add_attribute_predicate(&mut self, key: impl Into<String>, value: serde_json::Value) {
        let key = key.into();
        if let Some(slot) = self
            .attribute_predicates
            .iter_mut()
            .find(|(k, _)| *k == key)
        {
            slot.1 = value;
        } else {
            self.attribute_predicates.push((key, value));
        }
    }

    /// 移除 `key` 上的所有谓词；返回是否有内容被删除。
    pub fn remove_attribute_predicate(&mut self, key: &str) -> bool {
        let before = self.attribute_predicates.len();
        self.attribute_predicates.retain(|(k, _)| k != key);
        self.attribute_predicates.len() != before
    }

    /// 丢弃所有属性谓词。
    pub fn clear_attribute_predicates(&mut self) {
        self.attribute_predicates.clear();
    }

    /// 是否有任何 §10.5 / §10.8 / §10.9 限制处于激活（供面板
    /// 给过滤器控件上角标使用）。
    pub fn is_restrictive(&self) -> bool {
        self.enabled_types.is_some() || self.only_selected || !self.attribute_predicates.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_disables_overlay() {
        assert!(!Filters::default().overlay_enabled);
    }

    #[test]
    fn enabled_turns_master_on() {
        let f = Filters::enabled();
        assert!(f.overlay_enabled);
        assert!(f.enabled_types.is_none());
        assert!(f.attribute_predicates.is_empty());
    }

    #[test]
    fn type_toggle_materialises_all_then_isolates_one() {
        let mut f = Filters::enabled();
        // 尚无限制 → 每种类型都读为启用。
        for k in GeometryKind::all() {
            assert!(f.type_enabled(*k));
        }
        // 关掉一个会物化为去掉该类型的完整集。
        f.toggle_type(GeometryKind::Polygon);
        assert!(!f.type_enabled(GeometryKind::Polygon));
        assert!(f.type_enabled(GeometryKind::Point));
        // 再打开它会重新启用它。
        f.toggle_type(GeometryKind::Polygon);
        assert!(f.type_enabled(GeometryKind::Polygon));
    }

    #[test]
    fn overlay_and_focus_toggles_flip() {
        let mut f = Filters::enabled();
        f.toggle_overlay();
        assert!(!f.overlay_enabled);
        f.toggle_only_selected();
        assert!(f.only_selected);
    }

    #[test]
    fn attribute_predicates_add_replace_remove() {
        let mut f = Filters::enabled();
        assert!(!f.is_restrictive());
        f.add_attribute_predicate("side", serde_json::json!("friend"));
        assert_eq!(f.attribute_predicates.len(), 1);
        assert!(f.is_restrictive());
        // 重新添加相同的键会替换（从不重复）。
        f.add_attribute_predicate("side", serde_json::json!("hostile"));
        assert_eq!(f.attribute_predicates.len(), 1);
        assert_eq!(f.attribute_predicates[0].1, serde_json::json!("hostile"));
        // 第二个键会追加。
        f.add_attribute_predicate("alt", serde_json::json!(12));
        assert_eq!(f.attribute_predicates.len(), 2);
        assert!(f.remove_attribute_predicate("side"));
        assert!(!f.remove_attribute_predicate("missing"));
        assert_eq!(f.attribute_predicates.len(), 1);
        f.clear_attribute_predicates();
        assert!(f.attribute_predicates.is_empty());
    }
}
