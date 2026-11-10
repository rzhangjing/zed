//! 纯多维显隐求值器（计划 §10）。
//!
//! [`eval_visibility`] 是 `(Document, ViewContext, Filters)` 的一个全函数，
//! 返回哪些元素**可见**，以及其子集中哪些**可拾取**
//! （可见*且*在其图层 / 组锁下可选）。十个维度中的每一个
//! 都是一道独立的关卡，以 AND 折叠；时间维度在 M1 中
//! 是一个预留的空操作（返回通过），待 M9 接入时钟。
//!
//! 让它保持为一个自由函数 —— 而非系统、非资源驱动 —— 正是这一点
//! 使得整张显隐规则表无需引擎即可做单元测试（计划 §15）。

use std::collections::BTreeSet;

use serde_json::Value;

use crate::model::document::Document;
use crate::model::element::Element;
use crate::model::filters::Filters;
use crate::model::group::Group;
use crate::model::ids::ElementId;
use crate::model::layer::Layer;
use crate::model::view::{ViewContext, ViewMode};

/// 一次显隐遍历的结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VisibilityResult {
    /// 要绘制的元素，无特定顺序（桥接层会重新排序）。
    pub visible: BTreeSet<ElementId>,
    /// 可见**且**可拾取 —— 用于命中测试的候选集（计划 §7）。
    pub pickable: BTreeSet<ElementId>,
}

/// 针对十个显隐维度评估每个元素。
pub fn eval_visibility(doc: &Document, view: &ViewContext, filters: &Filters) -> VisibilityResult {
    let mut out = VisibilityResult::default();

    // §10.1 总开关：关闭时无任何元素可见（也不可拾取）。
    if !filters.overlay_enabled {
        return out;
    }

    // 逐元素评估：先定位其图层与组链，再依次过可见性与可拾取两道关卡。
    for element in doc.elements() {
        let Some((layer_id, group_chain)) = doc.element_context(element.id) else {
            continue; // 损坏树中的孤立节点 —— 防御性地跳过
        };
        // 元素必归属某图层；查不到图层同样视为孤立而跳过。
        let Some(layer) = doc.layer(layer_id) else {
            continue;
        };

        // 先过十道可见性关卡；任一不通过则完全不参与绘制。
        if !is_visible(doc, element, layer, &group_chain, view, filters) {
            continue;
        }
        out.visible.insert(element.id);

        // 可见之后再看拾取性（可见 ≠ 可拾取：锁定的图层 / 组会排除拾取）。
        if is_pickable(element, layer, &group_chain, doc, filters) {
            out.pickable.insert(element.id);
        }
    }
    out
}

/// 每个*可见性*维度的 AND（§10.2–§10.10，总开关已由
/// 调用方检查）。
fn is_visible(
    doc: &Document,
    element: &Element,
    layer: &Layer,
    group_chain: &[crate::model::ids::GroupId],
    view: &ViewContext,
    filters: &Filters,
) -> bool {
    // 任一维度不通过即短路返回 false；十个维度以 AND 折叠。
    // §10.2 图层开/关。
    if !layer.visible {
        return false;
    }
    // §10.3 链上每个组都可见：任一层被隐藏，元素随之下线（继承隐藏）。
    if !group_chain.iter().all(|g| {
        doc.group(*g).map(|grp| grp.visible).unwrap_or(false)
    }) {
        return false;
    }
    // §10.4 元素自身的手动开关。
    if !element.flags.visible_manual {
        return false;
    }
    // §10.5 类型维度：若设了类型白名单，几何类型不在其中则隐藏。
    if let Some(allowed) = &filters.enabled_types {
        if !allowed.contains(&element.geometry.kind()) {
            return false;
        }
    }
    // §10.6 视图模式维度：地球 / 平面各自有独立开关，缺省两者皆显示。
    let style = &element.style;
    match view.mode {
        ViewMode::Globe if !style.show_in_globe => return false,
        ViewMode::Flat if !style.show_in_flat => return false,
        _ => {}
    }
    // §10.7 比例尺带：当前缩放超出允许区间则隐藏。
    if !element
        .scale_visibility
        .allows(view.pixels_per_world, view.meters_per_pixel)
    {
        return false;
    }
    // §10.8 选中聚焦：开启时非选中元素一律隐藏。
    if filters.only_selected && !filters.selected.contains(&element.id) {
        return false;
    }
    // §10.9 属性维度：谓词集需全部命中（AND 语义）。
    if !attributes_pass(element, &filters.attribute_predicates) {
        return false;
    }
    // §10.10 时间窗口 —— 预留；在 M1 中总是通过（待 M9 接入时钟）。
    true
}

/// 可拾取性（计划 §7/§9）：元素 `selectable`、其图层 `selectable`，且
/// 链上没有*锁定*的组（锁定的组会使成员各自不可抓取）。
/// 选中聚焦过滤器也会收窄可拾取集。
fn is_pickable(
    element: &Element,
    layer: &Layer,
    group_chain: &[crate::model::ids::GroupId],
    doc: &Document,
    filters: &Filters,
) -> bool {
    // 可拾取要求元素、图层均可选，且组链上无锁定的组。
    // 元素或图层任一方关闭选择即不可拾取。
    if !element.flags.selectable || !layer.selectable {
        return false;
    }
    // 锁定的组让成员只能看、不能抓（逐个检查组链）。
    let any_locked = group_chain.iter().any(|g| {
        doc.group(*g)
            .map(|grp: &Group| grp.locked)
            .unwrap_or(false)
    });
    if any_locked {
        return false;
    }
    // 聚焦于选中项时，只有选中的 id 才可拾取。
    if filters.only_selected && !filters.selected.contains(&element.id) {
        return false;
    }
    // 通过以上所有关卡 → 可拾取。
    true
}

/// 属性谓词逐条比对：元素的自由属性需在每个 `(键, 期望值)` 上精确相等；
/// 缺键即视为不匹配。谓词集为空时自然全部通过。
fn attributes_pass(element: &Element, predicates: &[(String, Value)]) -> bool {
    predicates.iter().all(|(k, v)| match element.attributes.get(k) {
        Some(actual) => actual == v,
        None => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::GeoPoint;
    use crate::model::geometry::{Geometry, Polyline};
    use crate::model::ids::LayerId;
    use crate::model::Document;
    use std::collections::HashSet;

    /// 构建一个含两个图层 A / B 的文档，供各维度用例复用。
    fn doc_with_layers() -> (Document, LayerId, LayerId) {
        let mut doc = Document::default();
        let a = doc.new_layer("A");
        let b = doc.new_layer("B");
        (doc, a, b)
    }

    /// 向指定图层添加一个经纬为 (lon, lat) 的点元素，返回其 id。
    fn add_point(doc: &mut Document, layer: LayerId, lon: f64, lat: f64) -> ElementId {
        let ne = doc.make_element("p", Geometry::Point(GeoPoint::surface(lon, lat)));
        let id = ne.id;
        doc.add_element_to_layer(layer, ne);
        id
    }

    /// 构造一个视图上下文：模式 + 每世界像素 + 每像素米数，其余取缺省。
    fn view(mode: ViewMode, px: f64, mpp: f64) -> ViewContext {
        ViewContext {
            mode,
            pixels_per_world: px,
            meters_per_pixel: mpp,
            ..Default::default()
        }
    }

    /// 总开关关闭时一切不可见也不可拾取；打开后元素才浮现。
    #[test]
    fn master_switch_gates_everything() {
        let (mut doc, a, _b) = doc_with_layers();
        let e = add_point(&mut doc, a, 0.0, 0.0);
        let off = Filters::default(); // overlay_enabled 为 false，即总开关关闭
        let r = eval_visibility(&doc, &view(ViewMode::Globe, 0.0, 0.0), &off);
        // 总开关关闭 → 可见集与可拾取集均为空。
        assert!(r.visible.is_empty() && r.pickable.is_empty());

        let on = Filters::enabled();
        let r = eval_visibility(&doc, &view(ViewMode::Globe, 0.0, 0.0), &on);
        // 打开总开关 → 默认图层的点重新可见。
        assert!(r.visible.contains(&e));
    }

    /// 隐藏某个图层只影响其成员，另一图层的元素仍可见。
    #[test]
    fn layer_visibility_hides_members() {
        let (mut doc, a, b) = doc_with_layers();
        let ea = add_point(&mut doc, a, 0.0, 0.0);
        let eb = add_point(&mut doc, b, 1.0, 1.0);
        doc.layer_mut(a).unwrap().visible = false;
        // 隐藏图层 A 后，其成员从可见集移除，另一图层不受影响。
        let r = eval_visibility(&doc, &view(ViewMode::Globe, 0.0, 0.0), &Filters::enabled());
        assert!(!r.visible.contains(&ea), "hidden layer hides its element");
        assert!(r.visible.contains(&eb));
    }

    /// 组链继承隐藏：隐藏父组会连带隐藏其元素；元素自身手动开关同样否决。
    #[test]
    fn manual_flag_and_group_chain_inherit() {
        let (mut doc, a, _b) = doc_with_layers();
        let g = doc.new_group_in_layer(a, "g");
        let ne = doc.make_element("e", Geometry::Point(GeoPoint::surface(0.0, 0.0)));
        let e = ne.id;
        doc.add_element_to_group(g, ne);
        let f = Filters::enabled();
        let v = view(ViewMode::Globe, 0.0, 0.0);

        assert!(eval_visibility(&doc, &v, &f).visible.contains(&e));
        // 隐藏该组 → 元素消失。
        doc.group_mut(g).unwrap().visible = false;
        assert!(!eval_visibility(&doc, &v, &f).visible.contains(&e));
        doc.group_mut(g).unwrap().visible = true;
        // 元素自身手动关闭。
        doc.element_mut(e).unwrap().flags.visible_manual = false;
        assert!(!eval_visibility(&doc, &v, &f).visible.contains(&e));
    }

    /// 类型维度：白名单只放点时，折线被过滤掉而点保留。
    #[test]
    fn type_dimension_filters_kinds() {
        let (mut doc, a, _b) = doc_with_layers();
        let pt = add_point(&mut doc, a, 0.0, 0.0);
        // 另参加一条两点折线，用于验证它被类型白名单排除。
        let line = {
            let ne = doc.make_element(
                "line",
                Geometry::Polyline(Polyline {
                    positions: vec![GeoPoint::surface(0.0, 0.0), GeoPoint::surface(1.0, 1.0)],
                }),
            );
            let id = ne.id;
            doc.add_element_to_layer(a, ne);
            id
        };
        let mut only_points = HashSet::new();
        only_points.insert(crate::model::geometry::GeometryKind::Point);
        // 仅放行 Point 类型的白名单。
        let f = Filters {
            enabled_types: Some(only_points),
            ..Filters::enabled()
        };
        let r = eval_visibility(&doc, &view(ViewMode::Globe, 0.0, 0.0), &f);
        assert!(r.visible.contains(&pt));
        assert!(!r.visible.contains(&line));
    }

    /// 视图模式维度：仅平面可见的元素在地球视图被隐藏、平面视图显示。
    #[test]
    fn view_mode_dimension() {
        let (mut doc, a, _b) = doc_with_layers();
        let e = add_point(&mut doc, a, 0.0, 0.0);
        doc.element_mut(e).unwrap().style.show_in_globe = false; // 标为仅平面可见
        // 地球视图隐藏它，平面视图显示它。
        let rg = eval_visibility(&doc, &view(ViewMode::Globe, 0.0, 0.0), &Filters::enabled());
        assert!(!rg.visible.contains(&e));
        let rf = eval_visibility(&doc, &view(ViewMode::Flat, 0.0, 0.0), &Filters::enabled());
        assert!(rf.visible.contains(&e));
    }

    /// 比例尺带维度：低于最小缩放过远时隐藏，足够近时显示。
    #[test]
    fn scale_band_dimension() {
        let (mut doc, a, _b) = doc_with_layers();
        let e = add_point(&mut doc, a, 0.0, 0.0);
        doc.element_mut(e).unwrap().scale_visibility.min_pixels_per_world = Some(100.0);
        // 设最小每世界像素为 100；低于该缩放则隐藏。
        let f = Filters::enabled();
        // 缩小 (px=50) → 隐藏；放大 (px=200) → 显示。
        assert!(!eval_visibility(&doc, &view(ViewMode::Flat, 50.0, 0.0), &f).visible.contains(&e));
        assert!(eval_visibility(&doc, &view(ViewMode::Flat, 200.0, 0.0), &f).visible.contains(&e));
    }

    /// 选中聚焦同时收窄可见集与可拾取集：只有选中项两者皆命中。
    #[test]
    fn selection_focus_dimension_narrows_both_sets() {
        let (mut doc, a, _b) = doc_with_layers();
        let e1 = add_point(&mut doc, a, 0.0, 0.0);
        let e2 = add_point(&mut doc, a, 1.0, 1.0);
        let f = Filters {
            only_selected: true,
            selected: [e1].into_iter().collect(),
            ..Filters::enabled()
        };
        // 只选中 e1；预期可见与可拾取都只剩 e1。
        let r = eval_visibility(&doc, &view(ViewMode::Globe, 0.0, 0.0), &f);
        assert_eq!(r.visible, [e1].into_iter().collect::<BTreeSet<_>>());
        assert!(r.pickable.contains(&e1));
        assert!(!r.pickable.contains(&e2));
    }

    /// 属性维度按 AND 语义过滤：谓词 side=friend 只放行友方点。
    #[test]
    fn attribute_dimension_and_semantics() {
        let (mut doc, a, _b) = doc_with_layers();
        let friend = add_point(&mut doc, a, 0.0, 0.0);
        // 为两点分别打上 side 属性：friend 与 hostile。
        doc.element_mut(friend)
            .unwrap()
            .attributes
            .insert("side".into(), Value::String("friend".into()));
        let hostile = add_point(&mut doc, a, 1.0, 1.0);
        doc.element_mut(hostile)
            .unwrap()
            .attributes
            .insert("side".into(), Value::String("hostile".into()));

        let f = Filters {
            attribute_predicates: vec![("side".into(), Value::String("friend".into()))],
            ..Filters::enabled()
        };
        let r = eval_visibility(&doc, &view(ViewMode::Globe, 0.0, 0.0), &f);
        assert!(r.visible.contains(&friend));
        assert!(!r.visible.contains(&hostile));
    }

    /// 锁定的组：成员仍可见但不可拾取（只读不抓）。
    #[test]
    fn locked_group_and_unselectable_layer_block_pickable_only() {
        let (mut doc, a, _b) = doc_with_layers();
        let g = doc.new_group_in_layer(a, "g");
        let ne = doc.make_element("e", Geometry::Point(GeoPoint::surface(0.0, 0.0)));
        let e = ne.id;
        doc.add_element_to_group(g, ne);
        doc.group_mut(g).unwrap().locked = true;
        // 锁定不影响可见性，只排除拾取。
        let r = eval_visibility(&doc, &view(ViewMode::Globe, 0.0, 0.0), &Filters::enabled());
        assert!(r.visible.contains(&e), "locked but still visible");
        assert!(!r.pickable.contains(&e), "locked group is not pickable");
    }

    /// 不可选的图层：成员仍可见，但从可拾取集中排除。
    #[test]
    fn unselectable_layer_blocks_pickable() {
        let (mut doc, a, _b) = doc_with_layers();
        let e = add_point(&mut doc, a, 0.0, 0.0);
        doc.layer_mut(a).unwrap().selectable = false;
        // 图层不可选 → 元素可见但从可拾取集排除。
        let r = eval_visibility(&doc, &view(ViewMode::Globe, 0.0, 0.0), &Filters::enabled());
        assert!(r.visible.contains(&e));
        assert!(!r.pickable.contains(&e));
    }
}
