//! Entity 构建器扩展规范 - with_* 方法/has_graphics/merge/枚举
//! 参考自 DataSources/EntitySpec（A 类构建器/逻辑）

use cesium_datasource::entity::{
    Entity, PointGraphics, PolylineGraphics, PolygonGraphics, BillboardGraphics,
    LabelGraphics, ModelGraphics, BoxGraphics, CylinderGraphics, CorridorGraphics,
    RectangleGraphics, WallGraphics, EllipsoidGraphics, PlaneGraphics, PathGraphics,
    PolylineVolumeGraphics, HeightReference, CornerType, ClassificationType, ShadowMode,
};

// ─── 构建方法 ────────────────────────────────────────────────────────

#[test]
fn builder_with_name() {
    let e = Entity::new("test").with_name("My Entity");
    assert_eq!(e.id, "test");
    assert_eq!(e.name, Some("My Entity".to_string()));
}

#[test]
fn builder_with_position() {
    let e = Entity::new("test").with_position(1.0, 2.0, 100.0);
    assert!(e.position.is_defined());
}

#[test]
fn builder_with_point() {
    let e = Entity::new("test").with_point(PointGraphics::default());
    assert!(e.point.is_some());
    assert!(e.has_graphics());
}

#[test]
fn builder_with_polyline() {
    let e = Entity::new("test").with_polyline(PolylineGraphics::default());
    assert!(e.polyline.is_some());
    assert!(e.has_graphics());
}

#[test]
fn builder_with_polygon() {
    let e = Entity::new("test").with_polygon(PolygonGraphics::default());
    assert!(e.polygon.is_some());
}

#[test]
fn builder_with_billboard() {
    let e = Entity::new("test").with_billboard(BillboardGraphics::default());
    assert!(e.billboard.is_some());
}

#[test]
fn builder_with_label() {
    let e = Entity::new("test").with_label(LabelGraphics::default());
    assert!(e.label.is_some());
}

#[test]
fn builder_with_model() {
    let e = Entity::new("test").with_model(ModelGraphics::default());
    assert!(e.model.is_some());
}

#[test]
fn builder_with_box() {
    let e = Entity::new("test").with_box(BoxGraphics::default());
    assert!(e.box_graphics.is_some());
}

#[test]
fn builder_with_cylinder() {
    let e = Entity::new("test").with_cylinder(CylinderGraphics::default());
    assert!(e.cylinder.is_some());
}

#[test]
fn builder_with_corridor() {
    let e = Entity::new("test").with_corridor(CorridorGraphics::default());
    assert!(e.corridor.is_some());
}

#[test]
fn builder_with_rectangle() {
    let e = Entity::new("test").with_rectangle(RectangleGraphics::default());
    assert!(e.rectangle.is_some());
}

#[test]
fn builder_with_wall() {
    let e = Entity::new("test").with_wall(WallGraphics::default());
    assert!(e.wall.is_some());
}

#[test]
fn builder_with_ellipsoid() {
    let e = Entity::new("test").with_ellipsoid(EllipsoidGraphics::default());
    assert!(e.ellipsoid.is_some());
}

#[test]
fn builder_with_plane() {
    let e = Entity::new("test").with_plane(PlaneGraphics::default());
    assert!(e.plane.is_some());
}

#[test]
fn builder_with_path() {
    let e = Entity::new("test").with_path(PathGraphics::default());
    assert!(e.path.is_some());
}

#[test]
fn builder_with_polyline_volume() {
    let e = Entity::new("test").with_polyline_volume(PolylineVolumeGraphics::default());
    assert!(e.polyline_volume.is_some());
}

#[test]
fn builder_with_property() {
    let e = Entity::new("test").with_property("custom", serde_json::json!(42));
    assert!(e.properties.contains_key("custom"));
}

#[test]
fn builder_chaining() {
    let e = Entity::new("test")
        .with_name("Chained")
        .with_position(0.0, 0.0, 0.0)
        .with_point(PointGraphics::default())
        .with_property("key", serde_json::json!("value"));

    assert_eq!(e.name, Some("Chained".to_string()));
    assert!(e.point.is_some());
    assert!(e.has_graphics());
}

// ─── has_graphics ───────────────────────────────────────────────────────────

#[test]
fn has_graphics_false_for_empty_entity() {
    let e = Entity::new("empty");
    assert!(!e.has_graphics());
}

#[test]
fn has_graphics_true_for_any_graphics() {
    let e = Entity::new("test").with_point(PointGraphics::default());
    assert!(e.has_graphics());
}

// ─── 枚举 ──────────────────────────────────────────────────────────────────

#[test]
fn height_reference_default_is_none() {
    assert_eq!(HeightReference::default(), HeightReference::None);
}

#[test]
fn corner_type_default_is_rounded() {
    assert_eq!(CornerType::default(), CornerType::Rounded);
}

#[test]
fn classification_type_default_is_both() {
    assert_eq!(ClassificationType::default(), ClassificationType::Both);
}

#[test]
fn shadow_mode_default_is_disabled() {
    assert_eq!(ShadowMode::default(), ShadowMode::Disabled);
}

// ─── 合并 ──────────────────────────────────────────────────────────────────

#[test]
fn merge_fills_missing_graphics() {
    let mut target = Entity::new("target");
    let source = Entity::new("source").with_point(PointGraphics::default());

    target.merge(&source);
    assert!(target.point.is_some());
}

#[test]
fn merge_does_not_overwrite_existing() {
    let mut target = Entity::new("target").with_name("Original");
    let source = Entity::new("source").with_name("Override");

    target.merge(&source);
    assert_eq!(target.name, Some("Original".to_string()));
}
