//! NodeTransformationProperty 的测试 - 参考自 NodeTransformationPropertySpec
//!
//! 原始：7 个 it() → 5 个 A 类（2 个 C 类：result-param/definitionChanged 已省略）

use cesium_datasource::node_transformation_property::NodeTransformationProperty;
use cesium_datasource::property_system::property::{
    ConstantProperty, TimeIntervalCollectionProperty,
};
use cesium_datasource::property_system::value::PropertyValue;
use cesium_time::{JulianDate, TimeInterval};
use glam::{DQuat, DVec3};
use std::sync::Arc;

fn jd(seconds: f64) -> JulianDate {
    JulianDate::new(0.0, seconds)
}

// === 默认构造函数 ===

#[test]
fn test_node_transformation_default_constructor() {
    let property = NodeTransformationProperty::new();
    assert!(property.is_constant());
    assert!(property.translation().is_none());
    assert!(property.rotation().is_none());
    assert!(property.scale().is_none());

    let result = property.get_value(&jd(0.0));
    assert_eq!(result.translation, DVec3::ZERO);
    assert_eq!(result.rotation, DQuat::IDENTITY);
    assert_eq!(result.scale, DVec3::ONE);
}

// === 带选项的构造函数 ===

#[test]
fn test_node_transformation_constructor_with_options() {
    let translation = DVec3::Y;
    let rotation = DQuat::from_xyzw(0.5, 0.5, 0.5, 0.5);
    let scale = DVec3::X;

    let property = NodeTransformationProperty::with_values(translation, rotation, scale);
    assert!(property.translation().is_some());
    assert!(property.rotation().is_some());
    assert!(property.scale().is_some());

    let result = property.get_value(&jd(0.0));
    assert_eq!(result.translation, translation);
    assert_eq!(result.rotation, rotation);
    assert_eq!(result.scale, scale);
}

// === 适用于常量值 ===

#[test]
fn test_node_transformation_constant_values() {
    let mut property = NodeTransformationProperty::new();
    property.set_translation(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::Y),
    ))));
    property.set_rotation(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Quaternion(DQuat::from_xyzw(0.5, 0.5, 0.5, 0.5)),
    ))));
    property.set_scale(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::X),
    ))));

    let result = property.get_value(&jd(0.0));
    assert_eq!(result.translation, DVec3::Y);
    assert_eq!(result.rotation, DQuat::from_xyzw(0.5, 0.5, 0.5, 0.5));
    assert_eq!(result.scale, DVec3::X);
}

// === 适用于动态值 ===

#[test]
fn test_node_transformation_dynamic_values() {
    let mut property = NodeTransformationProperty::new();

    let mut tic_translation = TimeIntervalCollectionProperty::new();
    let mut tic_rotation = TimeIntervalCollectionProperty::new();
    let mut tic_scale = TimeIntervalCollectionProperty::new();

    let start = jd(86400.0); // JulianDate(1, 0)
    let stop = jd(172800.0); // JulianDate(2, 0)

    tic_translation.add_interval(
        TimeInterval::new(start, stop, true, true),
        Some(PropertyValue::Cartesian3(DVec3::Y)),
    );
    tic_rotation.add_interval(
        TimeInterval::new(start, stop, true, true),
        Some(PropertyValue::Quaternion(DQuat::from_xyzw(0.5, 0.5, 0.5, 0.5))),
    );
    tic_scale.add_interval(
        TimeInterval::new(start, stop, true, true),
        Some(PropertyValue::Cartesian3(DVec3::X)),
    );

    property.set_translation(Some(Arc::new(tic_translation)));
    property.set_rotation(Some(Arc::new(tic_rotation)));
    property.set_scale(Some(Arc::new(tic_scale)));

    assert!(!property.is_constant());

    let result = property.get_value(&start);
    assert_eq!(result.translation, DVec3::Y);
    assert_eq!(result.rotation, DQuat::from_xyzw(0.5, 0.5, 0.5, 0.5));
    assert_eq!(result.scale, DVec3::X);
}

// === equals ===

#[test]
fn test_node_transformation_equals() {
    let mut left = NodeTransformationProperty::new();
    left.set_translation(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::Y),
    ))));
    left.set_rotation(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Quaternion(DQuat::from_xyzw(0.5, 0.5, 0.5, 0.5)),
    ))));
    left.set_scale(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::X),
    ))));

    let mut right = NodeTransformationProperty::new();
    right.set_translation(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::Y),
    ))));
    right.set_rotation(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Quaternion(DQuat::from_xyzw(0.5, 0.5, 0.5, 0.5)),
    ))));
    right.set_scale(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::X),
    ))));

    assert!(left.equals(&right));

    // 不同的 scale
    right.set_scale(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::ZERO),
    ))));
    assert!(!left.equals(&right));

    // 恢复 scale，不同的 translation
    right.set_scale(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::X),
    ))));
    right.set_translation(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::ZERO),
    ))));
    assert!(!left.equals(&right));

    // 恢复 translation，不同的 rotation
    right.set_translation(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Cartesian3(DVec3::Y),
    ))));
    right.set_rotation(Some(Arc::new(ConstantProperty::new(
        PropertyValue::Quaternion(DQuat::from_xyzw(0.0, 0.0, 0.0, 0.0)),
    ))));
    assert!(!left.equals(&right));
}
