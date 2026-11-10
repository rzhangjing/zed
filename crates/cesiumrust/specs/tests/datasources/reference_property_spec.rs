//! CesiumJS DataSources/ReferencePropertySpec A 类测试的对齐实现。
//!
//! 原始：24 个 it() 测试。A 类（纯逻辑，无事件/spy）：10 个测试。
//! 基于事件的测试（definitionChanged 追踪）属 B 类。
//! Throws 测试属 C 类（Rust 改用类型系统 / Option 表达）。

use cesium_datasource::property_system::{
    ConstantProperty, DynProperty, MapPropertyResolver, PropertyValue, ReferenceProperty,
};
use cesium_time::JulianDate;
use std::sync::Arc;

fn jd(day: f64, seconds: f64) -> JulianDate {
    JulianDate::new(day, seconds)
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// ===========================================================================
// 构造函数
// ===========================================================================

#[test]
fn reference_property_constructor_sets_expected_values() {
    // 参考自："constructor sets expected values"
    let resolver = Arc::new(MapPropertyResolver::new());
    let property = ReferenceProperty::new(
        resolver,
        "testId",
        names(&["foo", "bar", "baz"]),
    );

    assert_eq!(property.target_id(), "testId");
    assert_eq!(
        property.target_property_names(),
        &["foo".to_string(), "bar".to_string(), "baz".to_string()]
    );
}

// ===========================================================================
// fromString
// ===========================================================================

#[test]
fn reference_property_from_string_sets_expected_values() {
    // 参考自："fromString sets expected values"
    let resolver = Arc::new(MapPropertyResolver::new());
    let property = ReferenceProperty::from_string(resolver, "testId#foo.bar.baz");

    assert_eq!(property.target_id(), "testId");
    assert_eq!(
        property.target_property_names(),
        &["foo".to_string(), "bar".to_string(), "baz".to_string()]
    );
}

#[test]
fn reference_property_from_string_works_with_escaped_values() {
    // 参考自："fromString works with escaped values"
    let resolver = Arc::new(MapPropertyResolver::new());
    let property = ReferenceProperty::from_string(
        resolver,
        r"\#identif\\\#ier\.#propertyName.\.abc\\.def",
    );

    assert_eq!(property.target_id(), "#identif\\#ier.");
    assert_eq!(
        property.target_property_names(),
        &[
            "propertyName".to_string(),
            ".abc\\".to_string(),
            "def".to_string()
        ]
    );
}

// ===========================================================================
// getValue / isConstant（带解析）
// ===========================================================================

#[test]
fn reference_property_get_value_returns_undefined_if_target_not_resolved() {
    // 参考自："getValue returns undefined if target entity can not be resolved"
    let resolver = Arc::new(MapPropertyResolver::new());
    let property = ReferenceProperty::from_string(resolver, "testId#foo.bar");
    let time = jd(2451545.0, 0.0);

    assert_eq!(property.get_value(&time), PropertyValue::Undefined);
}

#[test]
fn reference_property_get_value_returns_undefined_if_property_not_resolved() {
    // 参考自："getValue returns undefined if target property can not be resolved"
    // 在 "testId#billboard" 注册属性，但查询 "testId#billboard.scale"
    let mut r = MapPropertyResolver::new();
    r.insert(
        "testId",
        &names(&["billboard"]),
        Arc::new(ConstantProperty::new(PropertyValue::Number(5.0))),
    );
    let resolver = Arc::new(r);

    let property = ReferenceProperty::from_string(resolver, "testId#billboard.scale");
    let time = jd(2451545.0, 0.0);
    assert_eq!(property.get_value(&time), PropertyValue::Undefined);
}

#[test]
fn reference_property_is_constant_true_when_unresolved() {
    // 参考自："isConstant returns true when target entity does not exist"
    let resolver = Arc::new(MapPropertyResolver::new());
    let property = ReferenceProperty::from_string(resolver, "nonExistent#foo");

    assert!(property.is_constant());
}

#[test]
fn reference_property_properly_tracks_resolved_property() {
    // 参考自："properly tracks resolved property"（A 类子集：getValue/isConstant）
    let mut resolver = MapPropertyResolver::new();
    resolver.insert(
        "testId",
        &names(&["billboard", "scale"]),
        Arc::new(ConstantProperty::new(PropertyValue::Number(5.0))),
    );
    let resolver = Arc::new(resolver);

    let property = ReferenceProperty::from_string(resolver, "testId#billboard.scale");
    let time = jd(2451545.0, 0.0);

    assert!(property.is_constant());
    assert_eq!(
        property.get_value(&time),
        PropertyValue::Number(5.0)
    );

    // resolved_property 返回底层属性
    let resolved = property.resolved_property();
    assert!(resolved.is_some());
    assert_eq!(resolved.unwrap().get_value(&time), PropertyValue::Number(5.0));
}

#[test]
fn reference_property_resolved_property_none_when_unresolvable() {
    let resolver = Arc::new(MapPropertyResolver::new());
    let property = ReferenceProperty::from_string(resolver, "missing#foo.bar");

    assert!(property.resolved_property().is_none());
}

// ===========================================================================
// equals
// ===========================================================================

#[test]
fn reference_property_equals_works() {
    // 参考自："equals works"
    let resolver1 = Arc::new(MapPropertyResolver::new());
    let resolver2 = Arc::new(MapPropertyResolver::new());

    let left = ReferenceProperty::from_string(resolver1.clone(), "objectId#foo.bar");
    let right = ReferenceProperty::from_string(resolver1.clone(), "objectId#foo.bar");
    assert!(left.equals(&right));

    // collection（resolver）不同
    let right2 = ReferenceProperty::from_string(resolver2.clone(), "objectId#foo.bar");
    assert!(!left.equals(&right2));

    // target id 不同
    let right3 = ReferenceProperty::from_string(resolver1.clone(), "otherObjectId#foo.bar");
    assert!(!left.equals(&right3));

    // 子属性数量不同
    let right4 = ReferenceProperty::from_string(resolver1.clone(), "objectId#foo");
    assert!(!left.equals(&right4));

    // 长度相同的子属性序列不同
    let right5 = ReferenceProperty::from_string(resolver1.clone(), "objectId#foo.baz");
    assert!(!left.equals(&right5));
}

// ===========================================================================
// reference_frame 委派
// ===========================================================================

#[test]
fn reference_property_reference_frame_delegates_to_resolved() {
    // 参考自："works with position properties"（A 类子集：referenceFrame）
    use cesium_datasource::property_system::{ConstantPositionProperty, ReferenceFrame};
    use glam::DVec3;

    let mut resolver = MapPropertyResolver::new();
    let pos_prop = ConstantPositionProperty::new(DVec3::new(1.0, 2.0, 3.0));
    resolver.insert(
        "testId",
        &names(&["position"]),
        Arc::new(pos_prop),
    );
    let resolver = Arc::new(resolver);

    let property = ReferenceProperty::from_string(resolver.clone(), "testId#position");
    assert_eq!(property.reference_frame(), Some(ReferenceFrame::Fixed));

    // 不存在的引用没有参考系
    let property2 = ReferenceProperty::from_string(resolver, "nonExistent#position");
    assert_eq!(property2.reference_frame(), None);
}
