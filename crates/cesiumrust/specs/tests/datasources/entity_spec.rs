//! DataSources/EntitySpec、EntityCollectionSpec → Rust 集成测试

use cesium_datasource::entity::{
    BillboardGraphics, BoxGraphics, CylinderGraphics, EllipseGraphics, Entity, HeightReference,
    LabelGraphics, ModelGraphics, PointGraphics, PolygonGraphics, PolylineGraphics,
    RectangleGraphics, ShadowMode,
};
use cesium_datasource::entity_collection::{DataSource, EntityCollection};
use cesium_datasource::property::{Color, Property};

// === Entity ===

#[test]
fn test_entity_new() {
    let entity = Entity::new("test-entity".to_string());
    assert_eq!(entity.id, "test-entity");
    assert!(!entity.position.is_defined());
    assert!(entity.point.is_none());
}

#[test]
fn test_entity_with_position() {
    let mut entity = Entity::new("pos-entity".to_string());
    entity.position = Property::Constant([1.0, 2.0, 3.0]);
    assert!(entity.position.is_defined());
    assert_eq!(*entity.position.get_value(0.0).unwrap(), [1.0, 2.0, 3.0]);
}

#[test]
fn test_entity_with_point_graphics() {
    let mut entity = Entity::new("point-entity".to_string());
    entity.point = Some(PointGraphics::default());
    assert!(entity.point.is_some());
}

#[test]
fn test_entity_with_polyline_graphics() {
    let mut entity = Entity::new("polyline-entity".to_string());
    entity.polyline = Some(PolylineGraphics::default());
    assert!(entity.polyline.is_some());
}

#[test]
fn test_entity_with_polygon_graphics() {
    let mut entity = Entity::new("polygon-entity".to_string());
    entity.polygon = Some(PolygonGraphics::default());
    assert!(entity.polygon.is_some());
}

#[test]
fn test_entity_with_billboard_graphics() {
    let mut entity = Entity::new("billboard-entity".to_string());
    entity.billboard = Some(BillboardGraphics::default());
    assert!(entity.billboard.is_some());
}

#[test]
fn test_entity_with_label_graphics() {
    let mut entity = Entity::new("label-entity".to_string());
    entity.label = Some(LabelGraphics::default());
    assert!(entity.label.is_some());
}

#[test]
fn test_entity_with_model_graphics() {
    let mut entity = Entity::new("model-entity".to_string());
    entity.model = Some(ModelGraphics::default());
    assert!(entity.model.is_some());
}

#[test]
fn test_entity_with_ellipse_graphics() {
    let mut entity = Entity::new("ellipse-entity".to_string());
    entity.ellipse = Some(EllipseGraphics::default());
    assert!(entity.ellipse.is_some());
}

#[test]
fn test_entity_with_box_graphics() {
    let mut entity = Entity::new("box-entity".to_string());
    entity.box_graphics = Some(BoxGraphics::default());
    assert!(entity.box_graphics.is_some());
}

#[test]
fn test_entity_with_cylinder_graphics() {
    let mut entity = Entity::new("cylinder-entity".to_string());
    entity.cylinder = Some(CylinderGraphics::default());
    assert!(entity.cylinder.is_some());
}

#[test]
fn test_entity_with_rectangle_graphics() {
    let mut entity = Entity::new("rectangle-entity".to_string());
    entity.rectangle = Some(RectangleGraphics::default());
    assert!(entity.rectangle.is_some());
}

#[test]
fn test_entity_name() {
    let mut entity = Entity::new("id-123".to_string());
    entity.name = Some("My Entity".to_string());
    assert_eq!(entity.name.as_ref().unwrap(), "My Entity");
}

#[test]
fn test_entity_show() {
    let mut entity = Entity::new("show-entity".to_string());
    assert!(entity.show);
    entity.show = false;
    assert!(!entity.show);
}

// === HeightReference ===

#[test]
fn test_height_reference_default() {
    let hr = HeightReference::default();
    assert_eq!(hr, HeightReference::None);
}

#[test]
fn test_height_reference_variants() {
    assert_ne!(HeightReference::ClampToGround, HeightReference::RelativeToGround);
    assert_ne!(HeightReference::ClampToTileset, HeightReference::RelativeToTileset);
}

// === ShadowMode ===

#[test]
fn test_shadow_mode_default() {
    let sm = ShadowMode::default();
    assert_eq!(sm, ShadowMode::Disabled);
}

// === EntityCollection ===

#[test]
fn test_entity_collection_new() {
    let collection = EntityCollection::new();
    assert_eq!(collection.len(), 0);
    assert!(collection.is_empty());
}

#[test]
fn test_entity_collection_add() {
    let mut collection = EntityCollection::new();
    let entity = Entity::new("entity-1".to_string());
    collection.add(entity);
    assert_eq!(collection.len(), 1);
    assert!(!collection.is_empty());
}

#[test]
fn test_entity_collection_get() {
    let mut collection = EntityCollection::new();
    let entity = Entity::new("entity-1".to_string());
    collection.add(entity);
    let retrieved = collection.get("entity-1");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().id, "entity-1");
}

#[test]
fn test_entity_collection_get_nonexistent() {
    let collection = EntityCollection::new();
    assert!(collection.get("nonexistent").is_none());
}

#[test]
fn test_entity_collection_remove() {
    let mut collection = EntityCollection::new();
    collection.add(Entity::new("entity-1".to_string()));
    collection.add(Entity::new("entity-2".to_string()));
    assert_eq!(collection.len(), 2);

    let removed = collection.remove("entity-1");
    assert!(removed.is_some());
    assert_eq!(collection.len(), 1);
    assert!(!collection.contains("entity-1"));
    assert!(collection.contains("entity-2"));
}

#[test]
fn test_entity_collection_remove_nonexistent() {
    let mut collection = EntityCollection::new();
    let removed = collection.remove("nonexistent");
    assert!(removed.is_none());
}

#[test]
fn test_entity_collection_contains() {
    let mut collection = EntityCollection::new();
    collection.add(Entity::new("entity-1".to_string()));
    assert!(collection.contains("entity-1"));
    assert!(!collection.contains("entity-2"));
}

#[test]
fn test_entity_collection_clear() {
    let mut collection = EntityCollection::new();
    collection.add(Entity::new("entity-1".to_string()));
    collection.add(Entity::new("entity-2".to_string()));
    collection.add(Entity::new("entity-3".to_string()));
    assert_eq!(collection.len(), 3);

    collection.clear();
    assert_eq!(collection.len(), 0);
    assert!(collection.is_empty());
}

#[test]
fn test_entity_collection_values() {
    let mut collection = EntityCollection::new();
    collection.add(Entity::new("entity-1".to_string()));
    collection.add(Entity::new("entity-2".to_string()));

    let ids: Vec<&str> = collection.values().map(|e| e.id.as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"entity-1"));
    assert!(ids.contains(&"entity-2"));
}

#[test]
fn test_entity_collection_ids() {
    let mut collection = EntityCollection::new();
    collection.add(Entity::new("a".to_string()));
    collection.add(Entity::new("b".to_string()));
    collection.add(Entity::new("c".to_string()));

    let ids = collection.ids();
    assert_eq!(ids.len(), 3);
    // 应保持插入顺序
    assert_eq!(ids[0], "a");
    assert_eq!(ids[1], "b");
    assert_eq!(ids[2], "c");
}

#[test]
fn test_entity_collection_show() {
    let mut collection = EntityCollection::new();
    assert!(collection.show());
    collection.set_show(false);
    assert!(!collection.show());
}

#[test]
fn test_entity_collection_replace() {
    let mut collection = EntityCollection::new();
    let mut entity1 = Entity::new("entity-1".to_string());
    entity1.name = Some("Original".to_string());
    collection.add(entity1);

    let mut entity1_updated = Entity::new("entity-1".to_string());
    entity1_updated.name = Some("Updated".to_string());
    collection.add(entity1_updated);

    // 应仍只有 1 个实体
    assert_eq!(collection.len(), 1);
    assert_eq!(
        collection.get("entity-1").unwrap().name.as_ref().unwrap(),
        "Updated"
    );
}

// === DataSource ===

#[test]
fn test_data_source_new() {
    let ds = DataSource::new("test-datasource".to_string());
    assert_eq!(ds.name, "test-datasource");
    assert!(ds.entities.is_empty());
}

#[test]
fn test_data_source_add_entity() {
    let mut ds = DataSource::new("test-ds".to_string());
    ds.entities.add(Entity::new("e1".to_string()));
    assert_eq!(ds.entities.len(), 1);
}

// === Color ===

#[test]
fn test_color_new() {
    let color = Color::new(1.0, 0.5, 0.25, 0.75);
    assert!((color.red - 1.0).abs() < 1e-10);
    assert!((color.green - 0.5).abs() < 1e-10);
    assert!((color.blue - 0.25).abs() < 1e-10);
    assert!((color.alpha - 0.75).abs() < 1e-10);
}

#[test]
fn test_color_constants() {
    assert_eq!(Color::WHITE.red, 1.0);
    assert_eq!(Color::BLACK.red, 0.0);
    assert_eq!(Color::RED.red, 1.0);
    assert_eq!(Color::RED.green, 0.0);
}

#[test]
fn test_color_from_hex() {
    let color = Color::from_hex("#FF0000").unwrap();
    assert!((color.red - 1.0).abs() < 1e-10);
    assert!((color.green - 0.0).abs() < 1e-10);
    assert!((color.blue - 0.0).abs() < 1e-10);
}

#[test]
fn test_color_from_hex_with_alpha() {
    let color = Color::from_hex("#FF000080").unwrap();
    assert!((color.red - 1.0).abs() < 1e-10);
    assert!((color.alpha - 128.0 / 255.0).abs() < 1e-10);
}

#[test]
fn test_color_to_f32_array() {
    let color = Color::new(1.0, 0.5, 0.25, 1.0);
    let arr = color.to_f32_array();
    assert!((arr[0] - 1.0).abs() < 1e-6);
    assert!((arr[1] - 0.5).abs() < 1e-6);
}
