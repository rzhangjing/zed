//! Core/GeometryInstanceAttributeSpec + ColorGeometryInstanceAttributeSpec
//! + ShowGeometryInstanceAttributeSpec + DistanceDisplayConditionGeometryInstanceAttributeSpec
//! → Rust 集成测试（仅 A 类）

use cesium_geospatial::attribute_compression::ComponentDatatype;
use cesium_geospatial::color::Color;
use cesium_geospatial::geometry_instance_attribute::{
    ColorGeometryInstanceAttribute, DistanceDisplayConditionGeometryInstanceAttribute,
    GeometryInstanceAttribute, ShowGeometryInstanceAttribute,
};

// ─── GeometryInstanceAttribute ──────────────────────────────────────────────

#[test]
fn geometry_instance_attribute_constructor() {
    let attr = GeometryInstanceAttribute::new(
        ComponentDatatype::UnsignedByte,
        4,
        true,
        vec![255.0, 255.0, 0.0, 255.0],
    );
    assert_eq!(attr.component_datatype, ComponentDatatype::UnsignedByte);
    assert_eq!(attr.components_per_attribute, 4);
    assert!(attr.normalize);
    assert_eq!(attr.value, vec![255.0, 255.0, 0.0, 255.0]);
}

#[test]
#[should_panic(expected = "components_per_attribute must be between 1 and 4")]
fn geometry_instance_attribute_throws_invalid_components() {
    GeometryInstanceAttribute::new(
        ComponentDatatype::UnsignedByte,
        7,
        false,
        vec![1.0],
    );
}

// ─── ColorGeometryInstanceAttribute ─────────────────────────────────────────

#[test]
fn color_attribute_constructor() {
    let attr = ColorGeometryInstanceAttribute::new(1.0, 1.0, 0.0, 0.5);
    assert_eq!(attr.component_datatype(), ComponentDatatype::UnsignedByte);
    assert_eq!(attr.components_per_attribute(), 4);
    assert!(attr.normalize());

    let expected = Color::new(1.0, 1.0, 0.0, 0.5).to_bytes();
    assert_eq!(attr.value, expected);
}

#[test]
fn color_attribute_from_color() {
    let color = Color::AQUA;
    let attr = ColorGeometryInstanceAttribute::from_color(&color);
    assert_eq!(attr.component_datatype(), ComponentDatatype::UnsignedByte);
    assert_eq!(attr.components_per_attribute(), 4);
    assert!(attr.normalize());
    assert_eq!(attr.value, color.to_bytes());
}

#[test]
fn color_attribute_to_value() {
    let color = Color::AQUA;
    let expected = color.to_bytes();
    assert_eq!(ColorGeometryInstanceAttribute::to_value(&color), expected);
}

#[test]
fn color_attribute_equals() {
    let color = ColorGeometryInstanceAttribute::new(0.1, 0.2, 0.3, 0.4);
    // 相同引用
    assert!(ColorGeometryInstanceAttribute::equals(Some(&color), Some(&color)));
    // 相等的值
    let same = ColorGeometryInstanceAttribute::new(0.1, 0.2, 0.3, 0.4);
    assert!(ColorGeometryInstanceAttribute::equals(Some(&color), Some(&same)));
    // red 不同
    let diff_r = ColorGeometryInstanceAttribute::new(0.5, 0.2, 0.3, 0.4);
    assert!(!ColorGeometryInstanceAttribute::equals(Some(&color), Some(&diff_r)));
    // green 不同
    let diff_g = ColorGeometryInstanceAttribute::new(0.1, 0.5, 0.3, 0.4);
    assert!(!ColorGeometryInstanceAttribute::equals(Some(&color), Some(&diff_g)));
    // blue 不同
    let diff_b = ColorGeometryInstanceAttribute::new(0.1, 0.2, 0.5, 0.4);
    assert!(!ColorGeometryInstanceAttribute::equals(Some(&color), Some(&diff_b)));
    // alpha 不同
    let diff_a = ColorGeometryInstanceAttribute::new(0.1, 0.2, 0.3, 0.5);
    assert!(!ColorGeometryInstanceAttribute::equals(Some(&color), Some(&diff_a)));
    // None 情况
    assert!(!ColorGeometryInstanceAttribute::equals(Some(&color), None));
    assert!(!ColorGeometryInstanceAttribute::equals(None, Some(&color)));
}

// ─── ShowGeometryInstanceAttribute ──────────────────────────────────────────

#[test]
fn show_attribute_constructor() {
    let attr = ShowGeometryInstanceAttribute::new(false);
    assert_eq!(attr.component_datatype(), ComponentDatatype::UnsignedByte);
    assert_eq!(attr.components_per_attribute(), 1);
    assert!(!attr.normalize());
    assert_eq!(attr.value, [0u8]);
}

#[test]
fn show_attribute_to_value() {
    assert_eq!(ShowGeometryInstanceAttribute::to_value(true), [1u8]);
    assert_eq!(ShowGeometryInstanceAttribute::to_value(false), [0u8]);
}

#[test]
fn show_attribute_default_is_true() {
    let attr = ShowGeometryInstanceAttribute::new(true);
    assert_eq!(attr.value, [1u8]);
}

// ─── DistanceDisplayConditionGeometryInstanceAttribute ──────────────────────

#[test]
fn distance_display_attribute_constructor() {
    let attr = DistanceDisplayConditionGeometryInstanceAttribute::new(10.0, 100.0);
    assert_eq!(attr.component_datatype(), ComponentDatatype::Float);
    assert_eq!(attr.components_per_attribute(), 2);
    assert!(!attr.normalize());
    assert_eq!(attr.value, [10.0f32, 100.0]);
}

#[test]
#[should_panic(expected = "far distance must be greater than near distance")]
fn distance_display_attribute_throws_far_less_than_near() {
    DistanceDisplayConditionGeometryInstanceAttribute::new(100.0, 10.0);
}

#[test]
fn distance_display_attribute_from_ddc() {
    let attr =
        DistanceDisplayConditionGeometryInstanceAttribute::from_distance_display_condition(
            10.0, 100.0,
        );
    assert_eq!(attr.component_datatype(), ComponentDatatype::Float);
    assert_eq!(attr.components_per_attribute(), 2);
    assert!(!attr.normalize());
    assert_eq!(attr.value, [10.0f32, 100.0]);
}

#[test]
#[should_panic(expected = "distanceDisplayCondition.far distance must be greater")]
fn distance_display_attribute_from_ddc_throws() {
    DistanceDisplayConditionGeometryInstanceAttribute::from_distance_display_condition(
        100.0, 10.0,
    );
}

#[test]
fn distance_display_attribute_to_value() {
    let result = DistanceDisplayConditionGeometryInstanceAttribute::to_value(10.0, 200.0);
    assert_eq!(result, [10.0f32, 200.0]);
}
