//! PropertyArray 与 PositionPropertyArray 的测试
//! 参考自 PropertyArraySpec（10 个 it()）+ PositionPropertyArraySpec（11 个 it()）
//!
//! A 类：7 + 7 = 14 个测试（C 类：events/spy/result-param 已省略）

use cesium_datasource::property_array::{PositionPropertyArray, PropertyArray};
use cesium_datasource::property_system::property::{ConstantProperty, DynProperty};
use cesium_datasource::property_system::value::PropertyValue;
use cesium_time::JulianDate;
use glam::DVec3;
use std::sync::Arc;

fn time() -> JulianDate {
    JulianDate::new(0.0, 0.0)
}

// === PropertyArray ===

#[test]
fn test_property_array_default_constructor() {
    let property = PropertyArray::new();
    assert!(property.is_constant());
    assert!(property.get_value(&time()).is_none());
}

#[test]
fn test_property_array_constructor_with_value() {
    let value: Vec<Arc<dyn DynProperty>> = vec![
        Arc::new(ConstantProperty::new(PropertyValue::Number(1.0))),
        Arc::new(ConstantProperty::new(PropertyValue::Number(2.0))),
    ];
    let property = PropertyArray::with_value(value);
    let result = property.get_value(&time()).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], PropertyValue::Number(1.0));
    assert_eq!(result[1], PropertyValue::Number(2.0));
}

#[test]
fn test_property_array_undefined_value() {
    let mut property = PropertyArray::new();
    property.set_value(None);
    assert!(property.get_value(&time()).is_none());
}

#[test]
fn test_property_array_ignores_undefined_property_values() {
    // 值为 Undefined 的 ConstantProperty 应被过滤掉
    let value: Vec<Arc<dyn DynProperty>> = vec![Arc::new(ConstantProperty::new(
        PropertyValue::Undefined,
    ))];
    let property = PropertyArray::with_value(value);
    let result = property.get_value(&time()).unwrap();
    assert_eq!(result.len(), 0);
}

#[test]
fn test_property_array_empty_array() {
    let property = PropertyArray::with_value(vec![]);
    let result = property.get_value(&time()).unwrap();
    assert_eq!(result.len(), 0);
}

#[test]
fn test_property_array_equals() {
    let left = PropertyArray::with_value(vec![Arc::new(ConstantProperty::new(
        PropertyValue::Number(1.0),
    ))]);
    let right = PropertyArray::with_value(vec![Arc::new(ConstantProperty::new(
        PropertyValue::Number(1.0),
    ))]);
    assert!(left.equals(&right));

    let right2 = PropertyArray::with_value(vec![Arc::new(ConstantProperty::new(
        PropertyValue::Number(2.0),
    ))]);
    assert!(!left.equals(&right2));

    let empty_left = PropertyArray::new();
    let empty_right = PropertyArray::new();
    assert!(empty_left.equals(&empty_right));
}

#[test]
fn test_property_array_is_constant() {
    let property = PropertyArray::with_value(vec![Arc::new(ConstantProperty::new(
        PropertyValue::Number(2.0),
    ))]);
    assert!(property.is_constant());

    // 空即为常量
    let empty = PropertyArray::new();
    assert!(empty.is_constant());
}

// === PositionPropertyArray ===

#[test]
fn test_position_property_array_default_constructor() {
    let property = PositionPropertyArray::new();
    assert!(property.is_constant());
    assert!(property.get_value(&time()).is_none());
}

#[test]
fn test_position_property_array_constructor_with_value() {
    let value: Vec<Arc<dyn DynProperty>> = vec![
        Arc::new(ConstantProperty::new(PropertyValue::Cartesian3(DVec3::X))),
        Arc::new(ConstantProperty::new(PropertyValue::Cartesian3(DVec3::Z))),
    ];
    let property = PositionPropertyArray::with_value(value);
    let result = property.get_value(&time()).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], DVec3::X);
    assert_eq!(result[1], DVec3::Z);
}

#[test]
fn test_position_property_array_undefined_value() {
    let mut property = PositionPropertyArray::new();
    property.set_value(None);
    assert!(property.get_value(&time()).is_none());
}

#[test]
fn test_position_property_array_ignores_undefined() {
    let value: Vec<Arc<dyn DynProperty>> = vec![Arc::new(ConstantProperty::new(
        PropertyValue::Undefined,
    ))];
    let property = PositionPropertyArray::with_value(value);
    let result = property.get_value(&time()).unwrap();
    assert_eq!(result.len(), 0);
}

#[test]
fn test_position_property_array_empty() {
    let property = PositionPropertyArray::with_value(vec![]);
    let result = property.get_value(&time()).unwrap();
    assert_eq!(result.len(), 0);
}

#[test]
fn test_position_property_array_equals() {
    let left = PositionPropertyArray::with_value(vec![Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::X),
    ))]);
    let right = PositionPropertyArray::with_value(vec![Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::X),
    ))]);
    assert!(left.equals(&right));

    let right2 = PositionPropertyArray::with_value(vec![Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::Z),
    ))]);
    assert!(!left.equals(&right2));

    let empty_left = PositionPropertyArray::new();
    let empty_right = PositionPropertyArray::new();
    assert!(empty_left.equals(&empty_right));
}

#[test]
fn test_position_property_array_is_constant() {
    let property = PositionPropertyArray::with_value(vec![Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::X),
    ))]);
    assert!(property.is_constant());

    let empty = PositionPropertyArray::new();
    assert!(empty.is_constant());
}
