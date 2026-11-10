//! Entity 操作规范 - 参考自 DataSources/EntitySpec
//!
//! 测试 Entity 构建方法、has_graphics、is_available、add_property、
//! remove_property 以及 merge 操作。

use cesium_datasource::entity::{
    BillboardGraphics, BoxGraphics, CorridorGraphics, CylinderGraphics,
    EllipsoidGraphics, Entity, LabelGraphics, ModelGraphics, PathGraphics, PlaneGraphics,
    PointGraphics, PolygonGraphics, PolylineGraphics, PolylineVolumeGraphics,
    RectangleGraphics, WallGraphics,
};
use cesium_datasource::property::Property;
use cesium_time::JulianDate;

// ─── Entity 构建方法 ─────────────────────────────────────────────────

#[test]
fn entity_new_has_id() {
    let e = Entity::new("test-entity");
    assert_eq!(e.id, "test-entity");
    assert!(e.name.is_none());
}

#[test]
fn entity_with_name() {
    let e = Entity::new("e1").with_name("Test Entity");
    assert_eq!(e.name.as_deref(), Some("Test Entity"));
}

#[test]
fn entity_with_position() {
    let e = Entity::new("e1").with_position(10.0, 20.0, 100.0);
    // position 为 Property<[f64; 3]>，存储原始 [lon, lat, height]
    match &e.position {
        Property::Constant(v) => {
            assert!((v[0] - 10.0).abs() < 1e-10);
            assert!((v[1] - 20.0).abs() < 1e-10);
            assert!((v[2] - 100.0).abs() < 1e-10);
        }
        _ => panic!("expected Constant position"),
    }
}

#[test]
fn entity_with_point() {
    let e = Entity::new("e1").with_point(PointGraphics::default());
    assert!(e.point.is_some());
}

#[test]
fn entity_with_polyline() {
    let e = Entity::new("e1").with_polyline(PolylineGraphics::default());
    assert!(e.polyline.is_some());
}

#[test]
fn entity_with_polygon() {
    let e = Entity::new("e1").with_polygon(PolygonGraphics::default());
    assert!(e.polygon.is_some());
}

#[test]
fn entity_with_billboard() {
    let e = Entity::new("e1").with_billboard(BillboardGraphics::default());
    assert!(e.billboard.is_some());
}

#[test]
fn entity_with_label() {
    let e = Entity::new("e1").with_label(LabelGraphics::default());
    assert!(e.label.is_some());
}

#[test]
fn entity_with_model() {
    let e = Entity::new("e1").with_model(ModelGraphics::default());
    assert!(e.model.is_some());
}

#[test]
fn entity_with_box() {
    let e = Entity::new("e1").with_box(BoxGraphics::default());
    assert!(e.box_graphics.is_some());
}

#[test]
fn entity_with_cylinder() {
    let e = Entity::new("e1").with_cylinder(CylinderGraphics::default());
    assert!(e.cylinder.is_some());
}

#[test]
fn entity_with_corridor() {
    let e = Entity::new("e1").with_corridor(CorridorGraphics::default());
    assert!(e.corridor.is_some());
}

#[test]
fn entity_with_rectangle() {
    let e = Entity::new("e1").with_rectangle(RectangleGraphics::default());
    assert!(e.rectangle.is_some());
}

#[test]
fn entity_with_wall() {
    let e = Entity::new("e1").with_wall(WallGraphics::default());
    assert!(e.wall.is_some());
}

#[test]
fn entity_with_ellipsoid() {
    let e = Entity::new("e1").with_ellipsoid(EllipsoidGraphics::default());
    assert!(e.ellipsoid.is_some());
}

#[test]
fn entity_with_plane() {
    let e = Entity::new("e1").with_plane(PlaneGraphics::default());
    assert!(e.plane.is_some());
}

#[test]
fn entity_with_path() {
    let e = Entity::new("e1").with_path(PathGraphics::default());
    assert!(e.path.is_some());
}

#[test]
fn entity_with_polyline_volume() {
    let e = Entity::new("e1").with_polyline_volume(PolylineVolumeGraphics::default());
    assert!(e.polyline_volume.is_some());
}

#[test]
fn entity_builder_chaining() {
    let e = Entity::new("chain")
        .with_name("Chained Entity")
        .with_point(PointGraphics::default())
        .with_label(LabelGraphics::default());

    assert_eq!(e.name.as_deref(), Some("Chained Entity"));
    assert!(e.point.is_some());
    assert!(e.label.is_some());
}

// ─── has_graphics ────────────────────────────────────────────────────────────

#[test]
fn entity_has_graphics_false_when_empty() {
    let e = Entity::new("empty");
    assert!(!e.has_graphics());
}

#[test]
fn entity_has_graphics_true_with_point() {
    let e = Entity::new("with-point").with_point(PointGraphics::default());
    assert!(e.has_graphics());
}

#[test]
fn entity_has_graphics_true_with_polyline() {
    let e = Entity::new("with-polyline").with_polyline(PolylineGraphics::default());
    assert!(e.has_graphics());
}

#[test]
fn entity_has_graphics_true_with_polygon() {
    let e = Entity::new("with-polygon").with_polygon(PolygonGraphics::default());
    assert!(e.has_graphics());
}

#[test]
fn entity_has_graphics_true_with_billboard() {
    let e = Entity::new("with-billboard").with_billboard(BillboardGraphics::default());
    assert!(e.has_graphics());
}

#[test]
fn entity_has_graphics_true_with_label() {
    let e = Entity::new("with-label").with_label(LabelGraphics::default());
    assert!(e.has_graphics());
}

#[test]
fn entity_has_graphics_true_with_model() {
    let e = Entity::new("with-model").with_model(ModelGraphics::default());
    assert!(e.has_graphics());
}

// ─── is_available ────────────────────────────────────────────────────────────

#[test]
fn entity_is_available_without_availability() {
    let e = Entity::new("no-avail");
    let time = JulianDate::new(0.0, 0.0);
    // 未设置可用性 → 始终可用
    assert!(e.is_available(&time));
}

// ─── add_property / remove_property ─────────────────────────────────────────

#[test]
fn entity_add_property() {
    let mut e = Entity::new("e1");
    e.add_property("color", serde_json::json!("red"));
    assert_eq!(e.properties.get("color"), Some(&serde_json::json!("red")));
}

#[test]
fn entity_add_multiple_properties() {
    let mut e = Entity::new("e1");
    e.add_property("color", serde_json::json!("red"));
    e.add_property("size", serde_json::json!(10));
    e.add_property("visible", serde_json::json!(true));
    assert_eq!(e.properties.len(), 3);
}

#[test]
fn entity_remove_property() {
    let mut e = Entity::new("e1");
    e.add_property("color", serde_json::json!("red"));
    let removed = e.remove_property("color");
    assert_eq!(removed, Some(serde_json::json!("red")));
    assert!(e.properties.get("color").is_none());
}

#[test]
fn entity_remove_nonexistent_property() {
    let mut e = Entity::new("e1");
    let removed = e.remove_property("nonexistent");
    assert!(removed.is_none());
}

#[test]
fn entity_readd_removed_property() {
    let mut e = Entity::new("e1");
    e.add_property("bob", serde_json::json!(1));
    e.remove_property("bob");
    e.add_property("bob", serde_json::json!(2));
    assert_eq!(e.properties.get("bob"), Some(&serde_json::json!(2)));
}

// ─── merge ───────────────────────────────────────────────────────────────────

#[test]
fn entity_merge_copies_name() {
    let mut target = Entity::new("target");
    let source = Entity::new("source").with_name("Source Name");
    target.merge(&source);
    assert_eq!(target.name.as_deref(), Some("Source Name"));
}

#[test]
fn entity_merge_does_not_overwrite_name() {
    let mut target = Entity::new("target").with_name("Target Name");
    let source = Entity::new("source").with_name("Source Name");
    target.merge(&source);
    assert_eq!(target.name.as_deref(), Some("Target Name"));
}

#[test]
fn entity_merge_copies_custom_properties() {
    let mut target = Entity::new("target");
    let mut source = Entity::new("source");
    source.add_property("custom", serde_json::json!("value"));
    target.merge(&source);
    assert_eq!(
        target.properties.get("custom"),
        Some(&serde_json::json!("value"))
    );
}

#[test]
fn entity_merge_does_not_overwrite_existing_properties() {
    let mut target = Entity::new("target");
    target.add_property("custom", serde_json::json!("original"));
    let mut source = Entity::new("source");
    source.add_property("custom", serde_json::json!("new"));
    target.merge(&source);
    assert_eq!(
        target.properties.get("custom"),
        Some(&serde_json::json!("original"))
    );
}

#[test]
fn entity_merge_copies_graphics() {
    let mut target = Entity::new("target");
    let source = Entity::new("source").with_point(PointGraphics::default());
    target.merge(&source);
    assert!(target.point.is_some());
}

#[test]
fn entity_merge_does_not_overwrite_graphics() {
    let mut target = Entity::new("target").with_point(PointGraphics::default());
    let source = Entity::new("source").with_label(LabelGraphics::default());
    target.merge(&source);
    // target 保留其 point，从 source 获得 label
    assert!(target.point.is_some());
    assert!(target.label.is_some());
}

// ─── Entity with_property ────────────────────────────────────────────────────

#[test]
fn entity_with_property_builder() {
    let e = Entity::new("e1")
        .with_property("color", serde_json::json!("blue"))
        .with_property("size", serde_json::json!(42));
    assert_eq!(e.properties.get("color"), Some(&serde_json::json!("blue")));
    assert_eq!(e.properties.get("size"), Some(&serde_json::json!(42)));
}

// ─── Entity show property ────────────────────────────────────────────────────

#[test]
fn entity_show_defaults_to_true() {
    let e = Entity::new("e1");
    assert!(e.show);
}

#[test]
fn entity_show_can_be_set() {
    let mut e = Entity::new("e1");
    e.show = false;
    assert!(!e.show);
}

// ─── Entity description ──────────────────────────────────────────────────────

#[test]
fn entity_description_defaults_to_none() {
    let e = Entity::new("e1");
    assert!(e.description.is_none());
}

#[test]
fn entity_with_description() {
    let mut e = Entity::new("e1");
    e.description = Some("A test entity".to_string());
    assert_eq!(e.description.as_deref(), Some("A test entity"));
}
