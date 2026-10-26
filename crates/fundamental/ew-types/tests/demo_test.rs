//! 用 demo 目录做端到端断言：提取、流转解析、合规检查。

use std::path::PathBuf;

use ew_types::api::{self, Format};
use ew_types::check::check;
use ew_types::extract::extract_all;

fn demo_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/demo"))
}

fn bad_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/demo/bad"))
}

#[test]
fn extracts_struct_with_field_metadata() {
    let r = extract_all(&demo_root());
    assert!(r.errors.is_empty(), "解析不应有错误：{:?}", r.errors);

    let order = r
        .cards
        .iter()
        .find(|c| c.name == "Order")
        .expect("应有 Order");

    assert_eq!(order.kind, "struct");
    assert_eq!(order.path, "domain::order");
    assert_eq!(order.fields.len(), 6);
    assert!(order.invariants.iter().any(|i| i.contains("非空")));

    let total = order.fields.iter().find(|f| f.name == "total").unwrap();
    assert_eq!(total.ty, "Money");
    assert_eq!(total.unit.as_deref(), Some("分"));
    assert_eq!(total.range.as_deref(), Some("≥ 1"));
    assert_eq!(total.precision.as_deref(), Some("两位小数"));
    assert!(total.missing.is_empty());

    let status = order.fields.iter().find(|f| f.name == "status").unwrap();
    assert_eq!(status.default.as_deref(), Some("Pending"));

    let created = order.fields.iter().find(|f| f.name == "created_at").unwrap();
    assert_eq!(created.unit.as_deref(), Some("UTC"));
}

#[test]
fn extracts_ctor_and_checks() {
    let r = extract_all(&demo_root());
    let order = r.cards.iter().find(|c| c.name == "Order").unwrap();
    let ctor = order.ctor.as_ref().expect("应识别到 try_new");
    assert_eq!(ctor.name, "try_new");
    assert!(ctor.sig.contains("Result<Order, OrderError>"), "sig = {}", ctor.sig);
    assert!(ctor.checks.len() >= 2, "应取到 # 错误 段：{:?}", ctor.checks);
}

#[test]
fn parses_transitions_in_both_forms() {
    let r = extract_all(&demo_root());

    let status = r.cards.iter().find(|c| c.name == "OrderStatus").unwrap();
    assert_eq!(status.transitions.len(), 3, "bullet 形式应解析出 3 条边");
    assert!(status
        .transitions
        .iter()
        .any(|t| t.from == "Pending" && t.to == "Paid" && t.via == "支付成功"));

    let role = r.cards.iter().find(|c| c.name == "Role").unwrap();
    assert_eq!(role.transitions.len(), 2, "链式图应解析出 2 条边");
    assert_eq!(role.transitions[0].from, "Guest");
    assert_eq!(role.transitions[0].to, "Member");
    assert_eq!(role.transitions[1].to, "Verified");
}

#[test]
fn handles_generic_enum_with_payload() {
    let r = extract_all(&demo_root());
    let load = r.cards.iter().find(|c| c.name == "Load").unwrap();
    assert_eq!(load.generics, vec!["T".to_string()]);
    assert_eq!(load.variants.len(), 5);
    let ready = load.variants.iter().find(|v| v.name == "Ready").unwrap();
    assert_eq!(ready.payload.len(), 1);
    assert_eq!(ready.payload[0].ty, "Vec<T>");
}

#[test]
fn detects_newtype() {
    let r = extract_all(&demo_root());
    let id = r.cards.iter().find(|c| c.name == "OrderId").unwrap();
    assert_eq!(id.kind, "newtype");
    assert_eq!(id.fields.len(), 1);
    assert_eq!(id.fields[0].ty, "String");
}

#[test]
fn compliant_demo_has_no_errors() {
    let r = extract_all(&demo_root());
    let report = check(&r.cards);
    assert_eq!(
        report.errors(),
        0,
        "demo 应完全合规，实际违规：{:?}",
        report
            .violations
            .iter()
            .map(|v| format!("{} {} {}", v.rule, v.target, v.message))
            .collect::<Vec<_>>()
    );
}

#[test]
fn non_compliant_tree_is_caught() {
    let r = extract_all(&bad_root());
    let report = check(&r.cards);
    let rules: Vec<&str> = report.violations.iter().map(|v| v.rule).collect();
    assert!(rules.contains(&"TY-01"), "应报类型缺注释");
    assert!(rules.contains(&"FD-01"), "应报字段缺注释");
    assert!(rules.contains(&"FD-02"), "应报缺单位段");
    assert!(rules.contains(&"EN-01"), "应报变体缺注释");
    assert!(rules.contains(&"EN-02"), "应报兜底变体");
}

#[test]
fn json_and_compact_render() {
    let r = extract_all(&demo_root());
    let order = r.cards.iter().find(|c| c.name == "Order").unwrap();

    let json = api::get(&r.cards, "Order", Format::Json).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).expect("应是合法 JSON");
    assert_eq!(value["name"], "Order");
    assert_eq!(value["fields"][4]["unit"], "分");
    assert!(value["fields"][0]["unit"].is_null());

    let compact = api::get(&r.cards, "Order", Format::Compact).unwrap();
    assert!(compact.contains("Order · struct · domain::order"));
    assert!(compact.contains("单位:分"));
    assert_eq!(order.fields.len(), 6);
}

#[test]
fn refs_finds_referencing_types() {
    let r = extract_all(&demo_root());
    let text = api::refs(&r.cards, "Money");
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let names: Vec<String> = value["referencedBy"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["from"].as_str().unwrap().to_string())
        .collect();
    assert!(names.iter().any(|n| n == "Order"), "Order 引用了 Money");
    assert!(names.iter().any(|n| n == "LineItem"), "LineItem 引用了 Money");
}

#[test]
fn unknown_type_returns_error_not_guess() {
    let r = extract_all(&demo_root());
    let err = api::get(&r.cards, "NotExist", Format::Json).unwrap_err();
    assert!(err.contains("未找到"));
}
