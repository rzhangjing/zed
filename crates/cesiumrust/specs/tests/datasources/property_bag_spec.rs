//! DataSources/PropertyBagSpec → Rust 集成测试
//! 覆盖：PropertyBag 构造、addProperty、removeProperty、hasProperty、
//! getValue、isConstant、equals、merge

use cesium_datasource::property_bag::PropertyBag;
use cesium_datasource::property_system::property::ConstantProperty;
use cesium_datasource::property_system::value::PropertyValue;
use cesium_time::JulianDate;
use std::sync::Arc;

fn time() -> JulianDate {
    JulianDate::new(2451545.0, 0.0)
}

// ─── 构造 ───────────────────────────────────────────────────────────

#[test]
fn property_bag_default_construct() {
    let bag = PropertyBag::new();
    assert!(bag.is_constant());
    assert!(bag.property_names().is_empty());
    let value = bag.get_value(&time());
    assert!(value.is_empty());
}

#[test]
fn property_bag_construct_with_values() {
    let bag = PropertyBag::from_values(&[
        ("a", PropertyValue::Number(1.0)),
        ("b", PropertyValue::Number(2.0)),
    ]);

    assert!(bag.has_property("a"));
    assert!(bag.has_property("b"));
    assert_eq!(bag.property_names().len(), 2);

    let value = bag.get_value(&time());
    assert_eq!(value.get("a"), Some(&PropertyValue::Number(1.0)));
    assert_eq!(value.get("b"), Some(&PropertyValue::Number(2.0)));
}

#[test]
fn property_bag_construct_with_properties() {
    let prop_a = Arc::new(ConstantProperty::new(PropertyValue::Number(1.0)));
    let prop_b = Arc::new(ConstantProperty::new(PropertyValue::Number(2.0)));

    let bag = PropertyBag::from_properties(&[
        ("a", prop_a as Arc<dyn cesium_datasource::property_system::property::DynProperty>),
        ("b", prop_b as Arc<dyn cesium_datasource::property_system::property::DynProperty>),
    ]);

    assert!(bag.has_property("a"));
    assert!(bag.has_property("b"));

    let value = bag.get_value(&time());
    assert_eq!(value.get("a"), Some(&PropertyValue::Number(1.0)));
    assert_eq!(value.get("b"), Some(&PropertyValue::Number(2.0)));
}

// ─── hasProperty ────────────────────────────────────────────────────────────

#[test]
fn property_bag_has_property() {
    let mut bag = PropertyBag::new();
    assert!(!bag.has_property("a"));
    bag.add_property("a");
    assert!(bag.has_property("a"));
}

// ─── addProperty ────────────────────────────────────────────────────────────

#[test]
fn property_bag_add_property_without_value() {
    let mut bag = PropertyBag::new();
    bag.add_property("a");

    assert_eq!(bag.property_names(), &["a".to_string()]);
    assert!(bag.has_property("a"));

    let value = bag.get_value(&time());
    assert_eq!(value.get("a"), Some(&PropertyValue::Undefined));
}

#[test]
fn property_bag_add_property_with_value() {
    let mut bag = PropertyBag::new();
    bag.add_property_value("a", PropertyValue::Number(1.0));

    assert_eq!(bag.property_names(), &["a".to_string()]);

    let value = bag.get_value(&time());
    assert_eq!(value.get("a"), Some(&PropertyValue::Number(1.0)));
}

#[test]
#[should_panic(expected = "propertyName is required")]
fn property_bag_add_property_requires_name() {
    let mut bag = PropertyBag::new();
    bag.add_property("");
}

#[test]
#[should_panic(expected = "already a registered property")]
fn property_bag_add_property_duplicate_throws() {
    let mut bag = PropertyBag::new();
    bag.add_property("a");
    bag.add_property("a");
}

// ─── removeProperty ─────────────────────────────────────────────────────────

#[test]
fn property_bag_remove_property() {
    let mut bag = PropertyBag::new();
    bag.add_property_value("a", PropertyValue::Number(1.0));
    assert!(bag.has_property("a"));

    bag.remove_property("a");

    assert!(bag.property_names().is_empty());
    assert!(!bag.has_property("a"));

    let value = bag.get_value(&time());
    assert!(value.is_empty());
}

#[test]
#[should_panic(expected = "propertyName is required")]
fn property_bag_remove_property_requires_name() {
    let mut bag = PropertyBag::new();
    bag.remove_property("");
}

#[test]
#[should_panic(expected = "is not a registered property")]
fn property_bag_remove_property_not_added_throws() {
    let mut bag = PropertyBag::new();
    bag.remove_property("a");
}

// ─── 带 result 的 getValue ───────────────────────────────────────────────────

#[test]
fn property_bag_get_value_with_result() {
    let bag = PropertyBag::from_values(&[
        ("a", PropertyValue::Number(1.0)),
        ("b", PropertyValue::Number(2.0)),
    ]);

    let mut result = std::collections::HashMap::new();
    result.insert("a".to_string(), PropertyValue::Number(-1.0));
    bag.get_value_with_result(&time(), &mut result);

    assert_eq!(result.get("a"), Some(&PropertyValue::Number(1.0)));
    assert_eq!(result.get("b"), Some(&PropertyValue::Number(2.0)));
}

#[test]
fn property_bag_leaves_extra_properties_in_result() {
    let bag = PropertyBag::from_values(&[
        ("a", PropertyValue::Number(1.0)),
    ]);

    let mut result = std::collections::HashMap::new();
    result.insert("q".to_string(), PropertyValue::Number(-1.0));
    bag.get_value_with_result(&time(), &mut result);

    assert_eq!(result.get("a"), Some(&PropertyValue::Number(1.0)));
    assert_eq!(result.get("q"), Some(&PropertyValue::Number(-1.0)));
}

// ─── isConstant ─────────────────────────────────────────────────────────────

#[test]
fn property_bag_is_constant_all_constant() {
    let mut bag = PropertyBag::new();
    bag.add_property_value("a", PropertyValue::Number(2.0));
    assert!(bag.is_constant());
}

#[test]
fn property_bag_is_constant_empty() {
    let bag = PropertyBag::new();
    assert!(bag.is_constant());
}

// ─── equals ─────────────────────────────────────────────────────────────────

#[test]
fn property_bag_equals_same() {
    let left = PropertyBag::from_values(&[
        ("a", PropertyValue::Number(1.0)),
    ]);
    let right = PropertyBag::from_values(&[
        ("a", PropertyValue::Number(1.0)),
    ]);
    assert!(left.equals(&right));
}

#[test]
fn property_bag_equals_different_values() {
    let left = PropertyBag::from_values(&[
        ("a", PropertyValue::Number(1.0)),
    ]);
    let right = PropertyBag::from_values(&[
        ("a", PropertyValue::Number(2.0)),
    ]);
    assert!(!left.equals(&right));
}

#[test]
fn property_bag_equals_different_names() {
    let left = PropertyBag::from_values(&[
        ("a", PropertyValue::Number(1.0)),
    ]);
    let right = PropertyBag::from_values(&[
        ("b", PropertyValue::Number(1.0)),
    ]);
    assert!(!left.equals(&right));
}

#[test]
fn property_bag_equals_different_length() {
    let left = PropertyBag::from_values(&[
        ("a", PropertyValue::Number(1.0)),
    ]);
    let right = PropertyBag::new();
    assert!(!left.equals(&right));
}

#[test]
fn property_bag_equals_both_empty() {
    let left = PropertyBag::new();
    let right = PropertyBag::new();
    assert!(left.equals(&right));
}

#[test]
fn property_bag_equals_extra_property() {
    let left = PropertyBag::from_values(&[
        ("a", PropertyValue::Number(1.0)),
    ]);
    let mut right = PropertyBag::from_values(&[
        ("a", PropertyValue::Number(1.0)),
    ]);
    right.add_property("c");
    assert!(!left.equals(&right));
}

// ─── merge ──────────────────────────────────────────────────────────────────

#[test]
fn property_bag_merge() {
    let mut left = PropertyBag::new();
    let right = PropertyBag::from_values(&[
        ("a", PropertyValue::Number(1.0)),
        ("b", PropertyValue::Number(2.0)),
    ]);

    left.merge(&right);

    assert!(left.has_property("a"));
    assert!(left.has_property("b"));

    let value = left.get_value(&time());
    assert_eq!(value.get("a"), Some(&PropertyValue::Number(1.0)));
    assert_eq!(value.get("b"), Some(&PropertyValue::Number(2.0)));
}
