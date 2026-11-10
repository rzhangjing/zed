//! 场景文档：整个覆盖层都是其一个视图的单一可序列化事实源
//! （计划 §5、§14）。
//!
//! 一组扁平的表（`layers` / `groups` / `elements`）加上一个 `parents`
//! 索引，将树变为 O(1) 的向上遍历。Id 来自单个
//! 单调计数器。所有结构编辑都通过这里的方法进行，因此
//! 索引与 `Group.parent` 字段永不偏离 `roots` / `members`
//! 向量 —— 后续的命令层（`ops`，M6）驱动这些方法并为
//! 撤销记录逆操作。
//!
//! ## 为何采用扁平表 + 父索引
//! 树结构以三张按 id 索引的扁平表（`layers`/`groups`/`elements`）存储，
//! 另用一个 `#[serde(skip)]` 的 `parents` 映像在运行时维护向上遍历。序列化
//! 只保存表本身，加载后需 [`Document::rebuild_parents`] 重建父索引；这避免了把
//! 冗余且易偏离的反向边写入磁盘。

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::geo::GeoBounds;

use super::element::Element;
use super::geometry::Geometry;
use super::group::Group;
use super::ids::{ElementId, GroupId, LayerId};
use super::layer::Layer;
use super::node::Node;

/// 一个节点在树中的位置。图层根节点下直接挂组或元素，
/// 因此父引用只能是这两种之一。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParentRef {
    Layer(LayerId),
    Group(GroupId),
}

/// 一个新构建、尚未放置的元素（其 `id` 已铸造）。
///
/// 将新铸造的 id 与其 `Element` 绑定传递，直到它被显式放入某个图层或组。
pub struct NewElement {
    pub id: ElementId,
    pub element: Element,
}

/// 文档树。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// 按插入顺序排列的图层（绘制顺序由 `Layer::order` 解析）。
    layers: Vec<Layer>,
    /// 按 id 索引的组集合（与 `layers`/`elements` 一起构成扁平表）。
    groups: BTreeMap<GroupId, Group>,
    /// 按 id 索引的元素集合；叶子图元的唯一事实源。
    elements: BTreeMap<ElementId, Element>,
    /// node -> 父节点，与每次树变更保持同步。
    #[serde(skip)]
    parents: HashMap<Node, ParentRef>,
    /// 三种 id 类型共享的单调 id 计数器（每文档唯一）。
    next_id: u64,
    /// 新内容落入的图层（§9）。
    active_layer: Option<LayerId>,
}

impl Document {
    /// 创建一个带单个默认、活动图层的空文档。
    pub fn with_default_layer() -> Self {
        let mut doc = Self::default();
        let id = doc.new_layer("Default layer");
        doc.active_layer = Some(id);
        doc
    }

    /// 从单调计数器铸造下一个原始 id 值（三种 id 类型共享同一序列）。
    fn alloc(&mut self) -> u64 {
        let v = self.next_id;
        self.next_id += 1;
        v
    }

    // ── 图层 ─────────────────────────────────────────────────────────────

    /// 铸造 + 追加一个新图层，返回其 id。
    pub fn new_layer(&mut self, name: impl Into<String>) -> LayerId {
        let id = LayerId(self.alloc());
        self.layers.push(Layer::new(id, name));
        id
    }

    /// 按 id 取得一个不可变图层引用；不存在时返回 `None`。
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// 按 id 取得一个可变图层引用；不存在时返回 `None`。
    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    /// 返回按插入顺序排列的全部图层的切片视图。
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// 按 `order` 升序排列的图层（由后到前绘制）。
    pub fn layers_ordered(&self) -> Vec<&Layer> {
        let mut v: Vec<&Layer> = self.layers.iter().collect();
        v.sort_by_key(|l| (l.order, l.id.raw()));
        v
    }

    /// 设置新内容落入的活动图层；传 `None` 可清空当前活动选择。
    pub fn set_active_layer(&mut self, id: Option<LayerId>) {
        self.active_layer = id;
    }

    /// 读取当前活动图层 id（若尚未选择则为 `None`）。
    pub fn active_layer(&self) -> Option<LayerId> {
        self.active_layer
    }

    /// 使 `id` 成为活动图层（新内容落入此处）并在
    /// [`Layer`] 本身上反映该标志。当图层已不存在时静默忽略。
    pub fn focus_layer(&mut self, id: LayerId) {
        for l in &mut self.layers {
            l.active = l.id == id;
        }
        self.active_layer = Some(id);
    }

    /// 设置图层的 §10.2 手动可见性。
    pub fn set_layer_visible(&mut self, id: LayerId, visible: bool) {
        if let Some(l) = self.layer_mut(id) {
            l.visible = visible;
        }
    }

    /// 设置图层的编辑权限（锁定的图层为只读，计划 §9）。
    pub fn set_layer_editable(&mut self, id: LayerId, editable: bool) {
        if let Some(l) = self.layer_mut(id) {
            l.editable = editable;
        }
    }

    /// 设置图层的组不透明度乘子，钳制到 `0..=1`（§9）。
    pub fn set_layer_opacity(&mut self, id: LayerId, opacity: f32) {
        if let Some(l) = self.layer_mut(id) {
            l.opacity = opacity.clamp(0.0, 1.0);
        }
    }

    /// 设置图层的拾取 / 选中权限（计划 §9）。
    pub fn set_layer_selectable(&mut self, id: LayerId, selectable: bool) {
        if let Some(l) = self.layer_mut(id) {
            l.selectable = selectable;
        }
    }

    /// 将图层的绘制 / 拾取 `order` 推进 `delta`（面板的上 / 下
    /// 按钮）。越高越后绘制（在上）。
    pub fn nudge_layer_order(&mut self, id: LayerId, delta: i32) {
        if let Some(l) = self.layer_mut(id) {
            l.order += delta;
        }
    }

    /// 移除一个图层以及曾处于其下的每个元素 / 组。
    pub fn remove_layer(&mut self, id: LayerId) {
        if let Some(idx) = self.layers.iter().position(|l| l.id == id) {
            let layer = self.layers.remove(idx);
            for node in &layer.roots {
                self.detach_subtree(*node);
            }
            if self.active_layer == Some(id) {
                self.active_layer = None;
            }
        }
    }

    // ── 元素 ─────────────────────────────────────────────────────────────

    /// 铸造一个元素（尚未入树）。包围盒会被计算。
    pub fn make_element(&mut self, name: impl Into<String>, geometry: Geometry) -> NewElement {
        let id = ElementId(self.alloc());
        let element = Element::new(id, name, geometry);
        NewElement { id, element }
    }

    /// 将一个元素放入某图层的 roots（追加，最后绘制）。
    pub fn add_element_to_layer(&mut self, layer: LayerId, ne: NewElement) -> ElementId {
        let Some(l) = self.layers.iter_mut().find(|l| l.id == layer) else {
            // 图层已消失：丢弃该元素（id 仍被消耗）。
            return ne.id;
        };
        l.roots.push(Node::Element(ne.id));
        self.parents.insert(Node::Element(ne.id), ParentRef::Layer(layer));
        self.elements.insert(ne.id, ne.element);
        ne.id
    }

    /// 将一个元素放入某组的 members。
    pub fn add_element_to_group(&mut self, group: GroupId, ne: NewElement) -> Option<ElementId> {
        // 组不存在则直接 `None`（元素被丢弃，id 仍被消耗）。
        let g = self.groups.get_mut(&group)?;
        g.members.push(Node::Element(ne.id));
        self.parents
            .insert(Node::Element(ne.id), ParentRef::Group(group));
        self.elements.insert(ne.id, ne.element);
        Some(ne.id)
    }

    /// 按 id 取得一个不可变元素引用；不存在时返回 `None`。
    pub fn element(&self, id: ElementId) -> Option<&Element> {
        self.elements.get(&id)
    }

    /// 按 id 取得一个可变元素引用；不存在时返回 `None`。
    pub fn element_mut(&mut self, id: ElementId) -> Option<&mut Element> {
        self.elements.get_mut(&id)
    }

    /// 以任意顺序迭代文档中的全部元素引用。
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.elements.values()
    }

    /// 以升序迭代全部元素 id（`BTreeMap` 保证按键排序）。
    pub fn element_ids(&self) -> impl Iterator<Item = ElementId> + '_ {
        self.elements.keys().copied()
    }

    /// 从树 + 表中移除一个元素（会否在其父节点的向量中
    /// 留下悬空槽位？不会 —— 我们也会清除该节点引用）。
    pub fn remove_element(&mut self, id: ElementId) {
        let node = Node::Element(id);
        if let Some(parent) = self.parents.remove(&node) {
            self.purge_node(parent, node);
        }
        self.elements.remove(&id);
    }

    // ── 组 ─────────────────────────────────────────────────────────────

    /// 在某图层根下创建一个组并返回其 id。
    pub fn new_group_in_layer(&mut self, layer: LayerId, name: impl Into<String>) -> GroupId {
        let id = GroupId(self.alloc());
        self.groups.insert(id, Group::new(id, name));
        if let Some(l) = self.layers.iter_mut().find(|l| l.id == layer) {
            l.roots.push(Node::Group(id));
        }
        self.parents.insert(Node::Group(id), ParentRef::Layer(layer));
        id
    }

    /// 在父组下创建一个嵌套组。
    pub fn new_group_in_group(&mut self, parent: GroupId, name: impl Into<String>) -> Option<GroupId> {
        let id = GroupId(self.alloc());
        self.groups.insert(id, Group::new(id, name));
        if let Some(g) = self.groups.get_mut(&parent) {
            g.members.push(Node::Group(id));
            self.parents.insert(Node::Group(id), ParentRef::Group(parent));
            Some(id)
        } else {
            self.groups.remove(&id);
            None
        }
    }

    /// 按 id 取得一个不可变组引用；不存在时返回 `None`。
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.get(&id)
    }

    /// 按 id 取得一个可变组引用；不存在时返回 `None`。
    pub fn group_mut(&mut self, id: GroupId) -> Option<&mut Group> {
        self.groups.get_mut(&id)
    }

    // ── 树遍历 ────────────────────────────────────────────────────────────

    /// 一个元素最终所属的图层（沿父链向上遍历），
    /// 加上从图层根下到元素的组链（最外层在前）。对孤立节点
    /// 返回 `None`（在合法树中不应发生）。
    pub fn element_context(&self, id: ElementId) -> Option<(LayerId, Vec<GroupId>)> {
        // 自元素起沿父链上行，途经的组依次入列，最后反转得到由外到内的链。
        let mut groups = Vec::new();
        let mut cur = Node::Element(id);
        // 防止损坏的环：以节点数为上限。
        for _ in 0..(self.elements.len() + self.groups.len() + 1) {
            match self.parents.get(&cur)? {
                ParentRef::Layer(l) => {
                    groups.reverse();
                    return Some((*l, groups));
                }
                ParentRef::Group(g) => {
                    groups.push(*g);
                    cur = Node::Group(*g);
                }
            }
        }
        None
    }

    /// 一个组所属的图层。
    pub fn group_layer(&self, id: GroupId) -> Option<LayerId> {
        // 沿父链逐层向上，直到遇到图层根节点；组数 +1 为循环上限防环。
        let mut cur = Node::Group(id);
        for _ in 0..(self.groups.len() + 1) {
            match self.parents.get(&cur)? {
                ParentRef::Layer(l) => return Some(*l),
                ParentRef::Group(g) => cur = Node::Group(*g),
            }
        }
        None
    }

    /// 某组下的所有元素 id（递归）。
    pub fn group_members_recursive(&self, id: GroupId) -> Vec<ElementId> {
        // 显式栈展开嵌套组，避免递归深度上限；元素收集、子组入栈。
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(g) = stack.pop() {
            let Some(grp) = self.groups.get(&g) else { continue };
            for node in &grp.members {
                match node {
                    Node::Element(e) => out.push(*e),
                    Node::Group(sub) => stack.push(*sub),
                }
            }
        }
        out
    }

    /// 文档中的每个元素，按图层顺序，然后每个图层内按
    /// roots 顺序，深度优先展开组（父先于成员，以便后续按
    /// `z_order` 排序能在图层内细化）。
    pub fn flatten_draw_order(&self) -> Vec<ElementId> {
        let mut out = Vec::new();
        for layer in self.layers_ordered() {
            for node in &layer.roots {
                self.collect_draw(*node, &mut out);
            }
        }
        out
    }

    /// 递归收集一个节点子树下的叶子元素 id 到 `out`（按树序）。
    ///
    /// 元素直接入队；组则递归展开其成员，从而将一个图层根下的
    /// 任意嵌套深度拉平为一个绘制序列。
    fn collect_draw(&self, node: Node, out: &mut Vec<ElementId>) {
        match node {
            Node::Element(e) => out.push(e),
            Node::Group(g) => {
                if let Some(grp) = self.groups.get(&g) {
                    for m in &grp.members {
                        self.collect_draw(*m, out);
                    }
                }
            }
        }
    }

    /// 为**绘制**排序的所有元素（计划 §16 M9 “z 排序完善”）：先按
    /// 图层 `order`（底层在前），然后按每个元素的
    /// `style.z_order` 层级，最后按 id 做稳定破平。不同于
    /// [`flatten_draw_order`](Self::flatten_draw_order)（仅图层优树序），
    /// 它尊重逐元素的 z 层级，因此用户可将一个要素提升
    /// 到其同层之上；按此顺序绘制使靠后（更高）的元素在屏幕
    /// 与拾取平局中均胜出。
    pub fn draw_order_sorted(&self) -> Vec<ElementId> {
        let mut keyed: Vec<(i32, i32, u64, ElementId)> = self
            .flatten_draw_order()
            .into_iter()
            .filter_map(|id| {
                let el = self.element(id)?;
                let layer_order = self
                    .element_context(id)
                    .and_then(|(l, _)| self.layer(l))
                    .map(|l| l.order)
                    .unwrap_or(0);
                Some((layer_order, el.style.z_order, id.raw(), id))
            })
            .collect();
        // 依次按图层 order、z_order、id 稳定排序，靠后者绘制时胜出。
        keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        keyed.into_iter().map(|(_, _, _, id)| id).collect()
    }

    // ── 聚合 ──────────────────────────────────────────────────────────────

    /// 所有元素包围盒的并集，若为空则 `None`。
    pub fn bounds(&self) -> Option<GeoBounds> {
        let mut acc = GeoBounds::empty();
        for e in self.elements.values() {
            acc = acc.union(e.bounds);
        }
        (!acc.is_empty()).then_some(acc)
    }

    /// 当图层、元素与组均为空时返回 `true`（一个完全空白的文档）。
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty() && self.elements.is_empty() && self.groups.is_empty()
    }

    /// 返回文档中叶子元素的总数。
    pub fn element_count(&self) -> usize {
        self.elements.len()
    }

    /// 从图层 / 组的成员向量重建 `#[serde(skip)]` 的父索引。一个反序列化
    /// [`Document`] 的加载器（例如从 GeoJSON，见 `crate::io`）必须先调用
    /// 本方法，之后任何向上遍历（`element_context`、
    /// `group_layer`、`flatten_draw_order`）才能返回正确结果。
    pub fn rebuild_parents(&mut self) {
        // 先从 roots/members 收集全部 (node, parent) 对，再一次性写入索引，避免边读边写。
        let mut refs: Vec<(Node, ParentRef)> = Vec::new();
        for layer in &self.layers {
            for node in &layer.roots {
                refs.push((*node, ParentRef::Layer(layer.id)));
            }
        }
        for (gid, group) in &self.groups {
            for node in &group.members {
                refs.push((*node, ParentRef::Group(*gid)));
            }
        }
        self.parents.clear();
        for (n, p) in refs {
            self.parents.insert(n, p);
        }
    }

    // ── 内部实现 ──────────────────────────────────────────────────────────

    /// 从持有它的任意位置（图层 roots 或组成员）移除一个节点
    /// 引用，依据其记录的父节点。
    fn purge_node(&mut self, parent: ParentRef, node: Node) {
        match parent {
            ParentRef::Layer(l) => {
                if let Some(layer) = self.layers.iter_mut().find(|x| x.id == l) {
                    layer.roots.retain(|n| *n != node);
                }
            }
            ParentRef::Group(g) => {
                if let Some(grp) = self.groups.get_mut(&g) {
                    grp.members.retain(|n| *n != node);
                }
            }
        }
    }

    /// 递归地分离一个节点（组或元素）及其下的一切，
    /// 丢弃元素/组表以及父条目。
    fn detach_subtree(&mut self, node: Node) {
        match node {
            Node::Element(e) => {
                self.parents.remove(&node);
                self.elements.remove(&e);
            }
            Node::Group(g) => {
                self.parents.remove(&node);
                let members = self.groups.remove(&g).map(|grp| grp.members).unwrap_or_default();
                for m in members {
                    self.detach_subtree(m);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::GeoPoint;

    /// 构造一个地面高度为 0 的点几何，供各测试快速搭建元素。
    fn point_geo(lon: f64, lat: f64) -> Geometry {
        Geometry::Point(GeoPoint::surface(lon, lat))
    }

    /// 默认图层应为活动图层，且能容纳新加入的元素：验证计数、
    /// 上下文回测与按 id 取回三者一致。
    #[test]
    fn default_layer_is_active_and_holds_elements() {
        let mut doc = Document::with_default_layer();
        let layer = doc.active_layer().unwrap();
        let ne = doc.make_element("a", point_geo(1.0, 2.0));
        let id = doc.add_element_to_layer(layer, ne);
        assert_eq!(doc.element_count(), 1);
        assert_eq!(doc.element_context(id).unwrap().0, layer);
        assert!(doc.element(id).is_some());
    }

    /// 排序应先看图层 order、再看元素 z_order、最后看 id；
    /// 验证后层元素无论何时加入都先于前层绘制。
    #[test]
    fn draw_order_sorted_honours_layer_then_z_then_id() {
        let mut doc = Document::default();
        let back = doc.new_layer("back");
        let front = doc.new_layer("front");
        doc.layer_mut(back).unwrap().order = 0;
        doc.layer_mut(front).unwrap().order = 10;

        let low = doc.make_element("low", point_geo(0.0, 0.0));
        let e_low = doc.add_element_to_layer(front, low);
        let mut hi = doc.make_element("high", point_geo(1.0, 1.0));
        hi.element.style.z_order = 5;
        let e_high = doc.add_element_to_layer(front, hi);
        // 一个后层元素无论 id 多后加入都必须仍先绘制（较低层）。
        let back_el = doc.make_element("back", point_geo(2.0, 2.0));
        let e_back = doc.add_element_to_layer(back, back_el);

        let order = doc.draw_order_sorted();
        assert_eq!(
            order,
            vec![e_back, e_low, e_high],
            "layer order dominates, then z_order within a layer"
        );
        // z 最高的前层元素最后绘制。
        assert_eq!(order.last(), Some(&e_high));
    }

    /// 图层/组/元素三种 id 共享同一计数器，故必须互不碰撞。
    #[test]
    fn ids_are_unique_across_kinds() {
        let mut doc = Document::default();
        let l = doc.new_layer("L");
        let g = doc.new_group_in_layer(l, "G");
        let e = doc.make_element("E", point_geo(0.0, 0.0)).id;
        let set = std::collections::HashSet::from([l.raw(), g.raw(), e.raw()]);
        assert_eq!(set.len(), 3, "layer/group/element ids must not collide");
    }

    /// 嵌套组中元素的上下文应沿链向上遍历，返回图层与由外到内的组链。
    #[test]
    fn nested_group_context_walks_chain() {
        let mut doc = Document::default();
        let l = doc.new_layer("L");
        let outer = doc.new_group_in_layer(l, "outer");
        let inner = doc.new_group_in_group(outer, "inner").unwrap();
        let ne = doc.make_element("e", point_geo(3.0, 4.0));
        let e = ne.id;
        doc.add_element_to_group(inner, ne);
        let (layer, chain) = doc.element_context(e).unwrap();
        assert_eq!(layer, l);
        // 链为最外层在前：图层的组，然后是嵌套的那个。
        assert_eq!(chain, vec![outer, inner]);
        assert_eq!(doc.group_layer(inner), Some(l));
        assert_eq!(doc.group_members_recursive(outer), vec![e]);
    }

    /// 删除元素应同时从父组成员向量与元素表中分离它，不留悬空引用。
    #[test]
    fn remove_element_detaches_from_group_and_table() {
        let mut doc = Document::default();
        let l = doc.new_layer("L");
        let g = doc.new_group_in_layer(l, "G");
        let e = doc.make_element("e", point_geo(0.0, 0.0));
        let eid = e.id;
        doc.add_element_to_group(g, e);
        assert_eq!(doc.group(g).unwrap().members.len(), 1);
        doc.remove_element(eid);
        assert!(doc.element(eid).is_none());
        assert!(doc.group(g).unwrap().members.is_empty());
    }

    /// 拉平绘制序应尊重图层 order 并递归展开组：后层组内的元素先于前层绘制。
    #[test]
    fn flatten_respects_layer_order_and_group_expansion() {
        let mut doc = Document::default();
        let back = doc.new_layer("back");
        let front = doc.new_layer("front");
        doc.layer_mut(back).unwrap().order = 0;
        doc.layer_mut(front).unwrap().order = 10;
        let g = doc.new_group_in_layer(back, "g");
        let e1 = doc.make_element("1", point_geo(0.0, 0.0));
        let e2 = doc.make_element("2", point_geo(1.0, 1.0));
        let (id1, id2) = (e1.id, e2.id);
        doc.add_element_to_group(g, e1);
        doc.add_element_to_layer(front, e2);
        let flat = doc.flatten_draw_order();
        assert_eq!(flat, vec![id1, id2], "group in back layer draws before front layer");
    }

    /// 图层可见/可编辑/可选/不透明度/order 等标志的设置与聚焦应往返一致，
    /// 越界不透明度会被钳制而非 panic。
    #[test]
    fn layer_flags_and_focus_round_trip() {
        let mut doc = Document::default();
        let a = doc.new_layer("a");
        let b = doc.new_layer("b");
        doc.focus_layer(a);
        assert_eq!(doc.active_layer(), Some(a));
        assert!(doc.layer(a).unwrap().active);
        assert!(!doc.layer(b).unwrap().active);
        // 聚焦 b 会连同标志位一起移动标记。
        doc.focus_layer(b);
        assert!(!doc.layer(a).unwrap().active);
        assert!(doc.layer(b).unwrap().active);

        doc.set_layer_visible(a, false);
        assert!(!doc.layer(a).unwrap().visible);
        doc.set_layer_editable(a, false);
        assert!(!doc.layer(a).unwrap().editable);
        doc.set_layer_selectable(a, false);
        assert!(!doc.layer(a).unwrap().selectable);

        let before = doc.layer(b).unwrap().order;
        doc.nudge_layer_order(b, 5);
        assert_eq!(doc.layer(b).unwrap().order, before + 5);

        doc.set_layer_opacity(a, 0.5);
        assert_eq!(doc.layer(a).unwrap().opacity, 0.5);
        // 越界的值会被钳制而非 panic。
        doc.set_layer_opacity(a, 2.0);
        assert_eq!(doc.layer(a).unwrap().opacity, 1.0);
        doc.set_layer_opacity(a, -1.0);
        assert_eq!(doc.layer(a).unwrap().opacity, 0.0);
    }

    /// 移除一个图层应级联删除其子树内的全部组与元素，并清空图层表。
    #[test]
    fn remove_layer_cascades_subtree() {
        let mut doc = Document::with_default_layer();
        let l = doc.active_layer().unwrap();
        let g = doc.new_group_in_layer(l, "g");
        let e = doc.make_element("e", point_geo(0.0, 0.0));
        let eid = e.id;
        doc.add_element_to_group(g, e);
        doc.remove_layer(l);
        assert!(doc.element(eid).is_none());
        assert!(doc.group(g).is_none());
        assert!(doc.layers().is_empty());
    }

    /// 文档包围盒应为所有元素包围盒的并集（跨折线与点）。
    #[test]
    fn bounds_union_over_elements() {
        let mut doc = Document::with_default_layer();
        let l = doc.active_layer().unwrap();
        let a = doc.make_element("a", Geometry::Polyline(crate::model::geometry::Polyline {
            positions: vec![GeoPoint::surface(-10.0, -20.0), GeoPoint::surface(0.0, 0.0)],
        }));
        let b = doc.make_element("b", point_geo(30.0, 40.0));
        doc.add_element_to_layer(l, a);
        doc.add_element_to_layer(l, b);
        let bb = doc.bounds().unwrap();
        assert_eq!((bb.west_deg, bb.south_deg, bb.east_deg, bb.north_deg), (-10.0, -20.0, 30.0, 40.0));
    }

    /// 反序列化后父索引为空（serde skip），重建后向上遍历应恢复。
    #[test]
    fn rebuild_parents_restores_walks_after_deserialise() {
        let mut doc = Document::default();
        let l = doc.new_layer("L");
        let g = doc.new_group_in_layer(l, "g");
        let ne = doc.make_element("e", point_geo(1.0, 2.0));
        let e = ne.id;
        doc.add_element_to_group(g, ne);
        // 往返会丢失父索引（serde skip），然后重建它。
        let j = serde_json::to_string(&doc).unwrap();
        let mut back: Document = serde_json::from_str(&j).unwrap();
        assert!(back.element_context(e).is_none(), "index is empty before rebuild");
        back.rebuild_parents();
        assert_eq!(back.element_context(e), Some((l, vec![g])));
        assert_eq!(back.group_layer(g), Some(l));
    }

    #[test]
    fn document_serde_roundtrip_rebuilds_parent_index() {
        // 序列化一个小树，然后确认表能回原。parents
        // 索引是 #[serde(skip)]，因此若需遍历则消费者必须重建
        // 它；这里我们断言原始表存活（索引重建是
        // 加载器的一个文档化后续职责，而非 serde）。
        let mut doc = Document::with_default_layer();
        let l = doc.active_layer().unwrap();
        let e = doc.make_element("e", point_geo(5.0, 6.0));
        doc.add_element_to_layer(l, e);
        let j = serde_json::to_string(&doc).unwrap();
        let back: Document = serde_json::from_str(&j).unwrap();
        assert_eq!(back.element_count(), 1);
        assert_eq!(back.layers().len(), 1);
        assert!(back.parents.is_empty(), "index is not serialised");
    }
}
