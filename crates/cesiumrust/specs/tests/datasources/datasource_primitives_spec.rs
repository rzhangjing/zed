//! Scene/BillboardCollectionSpec + LabelCollectionSpec + PointPrimitiveCollectionSpec
//! → Rust 集成测试。
//!
//! 参考 CesiumJS：
//! - Scene/Billboard, Scene/BillboardCollection
//! - Scene/Label, Scene/LabelCollection
//! - Scene/PointPrimitive, Scene/PointPrimitiveCollection
//! - Core/NearFarScalar, Core/DistanceDisplayCondition
//!
//! A 类测试：集合 CRUD、默认值、NearFarScalar 插值、
//! DistanceDisplayCondition 可见性、枚举默认值。
//! C 类已省略：WebGL 纹理图集、着色器程序、拾取。

use cesium_datasource::primitives::{
    Billboard, BillboardCollection, DistanceDisplayCondition, HorizontalOrigin,
    Label, LabelCollection, LabelStyle, NearFarScalar, PointPrimitive,
    PointPrimitiveCollection, VerticalOrigin,
};
use cesium_datasource::Color;

// === BillboardCollection ===

#[test]
fn billboard_collection_new_empty() {
    let c = BillboardCollection::new();
    assert!(c.is_empty());
    assert_eq!(c.len(), 0);
    assert!(c.show());
}

#[test]
fn billboard_collection_add_returns_index() {
    let mut c = BillboardCollection::new();
    let i0 = c.add(Billboard::default());
    let i1 = c.add(Billboard { scale: 2.0, ..Default::default() });
    assert_eq!(i0, 0);
    assert_eq!(i1, 1);
    assert_eq!(c.len(), 2);
}

#[test]
fn billboard_collection_get() {
    let mut c = BillboardCollection::new();
    c.add(Billboard {
        position: [1.0, 2.0, 3.0],
        image: Some("test.png".to_string()),
        ..Default::default()
    });
    let bb = c.get(0).unwrap();
    assert_eq!(bb.position, [1.0, 2.0, 3.0]);
    assert_eq!(bb.image.as_deref(), Some("test.png"));
    assert!(c.get(1).is_none());
}

#[test]
fn billboard_collection_remove() {
    let mut c = BillboardCollection::new();
    c.add(Billboard { scale: 1.0, ..Default::default() });
    c.add(Billboard { scale: 2.0, ..Default::default() });
    c.add(Billboard { scale: 3.0, ..Default::default() });

    let removed = c.remove(1).unwrap();
    assert_eq!(removed.scale, 2.0);
    assert_eq!(c.len(), 2);
    // 移除后，索引 1 现为旧的索引 2
    assert_eq!(c.get(1).unwrap().scale, 3.0);
    // 越界
    assert!(c.remove(5).is_none());
}

#[test]
fn billboard_collection_get_mut() {
    let mut c = BillboardCollection::new();
    c.add(Billboard::default());
    c.get_mut(0).unwrap().scale = 5.0;
    assert_eq!(c.get(0).unwrap().scale, 5.0);
}

#[test]
fn billboard_collection_clear() {
    let mut c = BillboardCollection::new();
    c.add(Billboard::default());
    c.add(Billboard::default());
    c.clear();
    assert!(c.is_empty());
}

#[test]
fn billboard_collection_show_toggle() {
    let mut c = BillboardCollection::new();
    assert!(c.show());
    c.set_show(false);
    assert!(!c.show());
}

#[test]
fn billboard_defaults() {
    let bb = Billboard::default();
    assert!(bb.show);
    assert_eq!(bb.position, [0.0; 3]);
    assert_eq!(bb.scale, 1.0);
    assert_eq!(bb.color, Color::WHITE);
    assert_eq!(bb.rotation, 0.0);
    assert_eq!(bb.vertical_origin, VerticalOrigin::Center);
    assert_eq!(bb.horizontal_origin, HorizontalOrigin::Center);
    assert!(!bb.size_in_meters);
    assert!(bb.image.is_none());
}

// === LabelCollection ===

#[test]
fn label_collection_add_get() {
    let mut c = LabelCollection::new();
    c.add(Label {
        text: "Hello".to_string(),
        font: "12px Arial".to_string(),
        fill_color: Color::RED,
        ..Default::default()
    });
    let l = c.get(0).unwrap();
    assert_eq!(l.text, "Hello");
    assert_eq!(l.font, "12px Arial");
    assert_eq!(l.fill_color, Color::RED);
}

#[test]
fn label_collection_remove() {
    let mut c = LabelCollection::new();
    c.add(Label { text: "A".to_string(), ..Default::default() });
    c.add(Label { text: "B".to_string(), ..Default::default() });
    let removed = c.remove(0).unwrap();
    assert_eq!(removed.text, "A");
    assert_eq!(c.len(), 1);
    assert_eq!(c.get(0).unwrap().text, "B");
}

#[test]
fn label_defaults() {
    let l = Label::default();
    assert!(l.show);
    assert_eq!(l.text, "");
    assert_eq!(l.font, "30px sans-serif");
    assert_eq!(l.fill_color, Color::WHITE);
    assert_eq!(l.outline_color, Color::BLACK);
    assert_eq!(l.outline_width, 1.0);
    assert_eq!(l.style, LabelStyle::Fill);
    assert!(!l.show_background);
    assert_eq!(l.background_padding, [7.0, 5.0]);
    assert_eq!(l.vertical_origin, VerticalOrigin::Baseline);
    assert_eq!(l.horizontal_origin, HorizontalOrigin::Left);
    assert_eq!(l.scale, 1.0);
}

// === PointPrimitiveCollection ===

#[test]
fn point_collection_add_get() {
    let mut c = PointPrimitiveCollection::new();
    c.add(PointPrimitive {
        position: [10.0, 20.0, 30.0],
        pixel_size: 15.0,
        color: Color::GREEN,
        outline_width: 2.0,
        ..Default::default()
    });
    let p = c.get(0).unwrap();
    assert_eq!(p.position, [10.0, 20.0, 30.0]);
    assert_eq!(p.pixel_size, 15.0);
    assert_eq!(p.color, Color::GREEN);
}

#[test]
fn point_collection_remove_clear() {
    let mut c = PointPrimitiveCollection::new();
    c.add(PointPrimitive::default());
    c.add(PointPrimitive::default());
    assert_eq!(c.remove(0).unwrap().pixel_size, 10.0);
    assert_eq!(c.len(), 1);
    c.clear();
    assert!(c.is_empty());
}

#[test]
fn point_defaults() {
    let p = PointPrimitive::default();
    assert!(p.show);
    assert_eq!(p.pixel_size, 10.0);
    assert_eq!(p.color, Color::WHITE);
    assert_eq!(p.outline_width, 0.0);
    assert_eq!(p.outline_color, Color::TRANSPARENT);
}

// === NearFarScalar ===

#[test]
fn near_far_scalar_interpolation() {
    let nfs = NearFarScalar::new(100.0, 1.0, 1000.0, 0.0);
    // near 之前 → near_value
    assert!((nfs.value_at_distance(0.0) - 1.0).abs() < 1e-10);
    assert!((nfs.value_at_distance(100.0) - 1.0).abs() < 1e-10);
    // 中点
    assert!((nfs.value_at_distance(550.0) - 0.5).abs() < 1e-10);
    // 四分之一处
    assert!((nfs.value_at_distance(325.0) - 0.75).abs() < 1e-10);
    // 到达 far → far_value
    assert!((nfs.value_at_distance(1000.0) - 0.0).abs() < 1e-10);
    // 超出 far → far_value
    assert!((nfs.value_at_distance(5000.0) - 0.0).abs() < 1e-10);
}

#[test]
fn near_far_scalar_default() {
    let nfs = NearFarScalar::default();
    assert_eq!(nfs.near, 0.0);
    assert_eq!(nfs.near_value, 0.0);
    assert_eq!(nfs.far, 1.0);
    assert_eq!(nfs.far_value, 0.0);
}

// === DistanceDisplayCondition ===

#[test]
fn distance_display_condition_visibility() {
    let ddc = DistanceDisplayCondition::new(100.0, 10000.0);
    assert!(!ddc.is_visible(50.0));
    assert!(ddc.is_visible(100.0));
    assert!(ddc.is_visible(5000.0));
    assert!(ddc.is_visible(10000.0));
    assert!(!ddc.is_visible(10001.0));
}

#[test]
fn distance_display_condition_pack_unpack() {
    let ddc = DistanceDisplayCondition::new(500.0, 50000.0);
    let mut arr = [0.0f64; 4];
    ddc.pack(&mut arr, 1);
    assert_eq!(arr[1], 500.0);
    assert_eq!(arr[2], 50000.0);

    let unpacked = DistanceDisplayCondition::unpack(&arr, 1);
    assert!(ddc.equals(&unpacked));
}

#[test]
fn distance_display_condition_default() {
    let ddc = DistanceDisplayCondition::default();
    assert_eq!(ddc.near, 0.0);
    assert_eq!(ddc.far, f64::MAX);
    assert!(ddc.is_visible(999999.0));
}

// === 枚举默认值 ===

#[test]
fn enum_defaults() {
    assert_eq!(VerticalOrigin::default(), VerticalOrigin::Center);
    assert_eq!(HorizontalOrigin::default(), HorizontalOrigin::Center);
    assert_eq!(LabelStyle::default(), LabelStyle::Fill);
}
