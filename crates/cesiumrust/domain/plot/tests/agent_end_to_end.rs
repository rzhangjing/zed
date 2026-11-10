//! 自闭环、跑门的 agent 赋能门面演示（计划 `Agent 赋能态势标绘`）。
//!
//! 与 crate 内的单元测试不同，这是一个**集成测试**：它完全像一个外部宿主
//! 那样消费 `cesium-plot` —— 经 `cesium_plot::agent` 下的公共再导出，且用
//! **原始 JSON 负载**（绝不手工构造 Rust 意图类型）。这使它成为一个一石二鸟
//! 的检查：
//!
//!  * 它证明整个门面从 crate 外面确实可达 / 好用（缺一个 `pub use` 会在此
//!    编译失败，而非在生产中）；
//!  * 它锁定真实的 "agent → server → agent" 循环端到端：拉取动作 schema，
//!    发送一个 `Create`，读回铸造的 id，把那个 id 喂进后续的 `Move`，
//!    查询 / 度量结果，撤销它，并往返整个文档 —— 全都带着一个被验证为
//!    不变更任何东西的被拒绝动作。

use cesium_plot::agent::{
    action_json_schema, measure_area, measure_length, ActionError, AgentAction, ElementSummary,
    PlotSession, QueryFilter, EXAMPLE_ACTION,
};
use cesium_plot::model::{ElementId, GeometryKind, ViewContext};
use serde_json::{json, Value};

/// `rows` 中对应 `id` 的摘要，否则 panic。
fn only(rows: &[ElementSummary], id: ElementId) -> &ElementSummary {
    rows.iter()
        .find(|e| e.id == id)
        .unwrap_or_else(|| panic!("no summary for {id:?} among {:?}", ids(rows)))
}

fn ids(rows: &[ElementSummary]) -> Vec<ElementId> {
    rows.iter().map(|e| e.id).collect()
}

#[test]
fn agent_drives_a_plot_session_over_json() {
    let mut session = PlotSession::with_default_layer();

    // ── 0. 契约发现 ───────────────────────────────────────────────
    // 一个 agent 可以在发送任何东西之前自我描述：拉取动作词汇表的 JSON Schema
    // 和一份规范的示例负载。
    let schema: Value = action_json_schema();
    assert_eq!(schema["title"], json!("AgentAction"));
    assert!(schema["$defs"]["AgentAction"]["oneOf"].is_array());
    // 该示例是一个真实、可应用的意图（像其他任何输入一样反序列化）。
    let example: AgentAction = serde_json::from_str(EXAMPLE_ACTION).unwrap();
    let friendly_id = session.apply(example).unwrap().new_id.expect("example creates");

    // ── 1. 写：发送一个原始 JSON 的 Create（一个敌方的禁飞区矩形） ─────
    let create_json = r#"{
        "Create": {
            "kind": "Rectangle",
            "positions": [
                { "lon_deg": 10.0, "lat_deg": 10.0, "height_m": 0.0 },
                { "lon_deg": 20.0, "lat_deg": 20.0, "height_m": 0.0 }
            ],
            "name": "deny-zone",
            "attributes": { "side": "hostile", "priority": 2 }
        }
    }"#;
    let create: AgentAction = serde_json::from_str(create_json).unwrap();
    let zone = session.apply(create).unwrap().new_id.expect("zone created");

    assert_eq!(session.doc.element_count(), 2);
    assert_eq!(session.undo_len(), 2);

    // ── 2. 读：把态势查询回来（属性 + 类型过滤器） ───────────
    let hostiles = session.query(&QueryFilter {
        attributes: vec![("side".into(), json!("hostile"))],
        ..Default::default()
    });
    assert_eq!(ids(&hostiles), vec![zone], "only the zone is hostile");
    assert_eq!(only(&hostiles, zone).name, "deny-zone");

    let rects = session.query(&QueryFilter {
        kind: Some(GeometryKind::Rectangle),
        ..Default::default()
    });
    assert_eq!(ids(&rects), vec![zone]);

    // agent 可借此"理解"场景的几何量。
    let perimeter = measure_length(&session.doc, zone).unwrap();
    let area = measure_area(&session.doc, zone).unwrap();
    assert!(perimeter > 1_000_000.0, "a 10°×10° zone perimeter, got {perimeter}");
    assert!(area > 1_000_000.0, "a 10°×10° zone area, got {area}");
    // 一条开放折线量得零包围面积。
    assert_eq!(measure_area(&session.doc, friendly_id).unwrap(), 0.0);

    // 视图感知的查询：在默认的 Globe 视图下两个元素都保持可见，
    // 借此走一遍读路径的比例尺带 / 每模式门控分支。
    let with_view =
        session.query_with_view(&QueryFilter::default(), Some(&ViewContext::default()));
    assert_eq!(with_view.len(), 2);
    assert!(with_view.iter().all(|e| e.visible));

    // ── 3. 把服务器铸造的 id 串回一个后续的 Move ────────────
    let bounds_before = only(&session.query(&QueryFilter::default()), zone).bounds;
    let move_json = format!(
        r#"{{"Move": {{ "target": {}, "delta_lonlat": [5.0, 0.0] }} }}"#,
        zone.raw()
    );
    let mv: AgentAction = serde_json::from_str(&move_json).unwrap();
    let touched = session.apply(mv).unwrap().touched_ids;
    assert_eq!(touched, vec![zone]);

    let bounds_after = only(&session.query(&QueryFilter::default()), zone).bounds;
    assert!(
        (bounds_after.west_deg - bounds_before.west_deg - 5.0).abs() < 1e-9,
        "west should shift +5°: {bounds_before:?} → {bounds_after:?}"
    );

    // ── 4. 撤销该移动；该区精确弹回 ────────────────────────
    assert!(session.can_undo());
    let undone = session.undo().expect("undo reports touched ids");
    assert_eq!(undone, vec![zone]);
    let bounds_restored = only(&session.query(&QueryFilter::default()), zone).bounds;
    assert_eq!(bounds_restored, bounds_before);

    // ── 5. 被拒绝的动作先经校验且不变更任何东西 ──────────
    let bad: AgentAction = serde_json::from_str(r#"{ "Delete": { "target": 424242 } }"#).unwrap();
    let err = session.apply(bad).unwrap_err();
    assert_eq!(err, ActionError::UnknownTarget(ElementId::new(424242)));
    assert_eq!(session.doc.element_count(), 2, "failed action left no trace");

    // ── 6. 通过公共门面导出 / 再导入整个文档 ───
    let json_text = session.export_doc();
    let mut restored = PlotSession::new();
    restored.import_doc(&json_text).unwrap();
    assert_eq!(restored.doc, session.doc, "lossless document round-trip");
    // 导入清空历史（先前的步骤属于被丢弃的文档）。
    assert!(!restored.can_undo());
}

#[test]
fn batch_and_sequential_creation_differ_in_undo_granularity() {
    // 一个 Batch 折叠为一个 undo 步骤；顺序的 apply 各是一个步骤。
    let mut session = PlotSession::with_default_layer();
    let mk = |lon: f64| {
        json!({ "Create": {
            "kind": "Point",
            "positions": [{ "lon_deg": lon, "lat_deg": 0.0, "height_m": 0.0 }]
        }})
    };
    let batch_json = json!({ "Batch": { "actions": [ mk(1.0), mk(2.0), mk(3.0) ] } });
    let batch: AgentAction = serde_json::from_str(&batch_json.to_string()).unwrap();
    session.apply(batch).unwrap();
    assert_eq!(session.doc.element_count(), 3);
    assert_eq!(session.undo_len(), 1, "batch is a single undo step");

    assert!(session.undo().is_some(), "one undo clears the whole batch");
    assert_eq!(session.doc.element_count(), 0);
}
