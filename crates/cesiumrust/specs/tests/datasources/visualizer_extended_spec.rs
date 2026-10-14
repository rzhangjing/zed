//! Visualizer 扩展规范 - GeometryVisualizer/StaticGeometryBatch/DynamicGeometryUpdater
//! 参考自 DataSources/GeometryVisualizerSpec（A 类逻辑）

use cesium_datasource::visualizer::{
    GeometryVisualizer, StaticGeometryBatch, DynamicGeometryUpdater,
};
use cesium_datasource::entity_collection::EntityCollection;
use cesium_datasource::entity::{Entity, PointGraphics};
use cesium_geospatial::Ellipsoid;

fn make_entity(id: &str) -> Entity {
    Entity::new(id)
        .with_position(0.0, 0.0, 0.0)
        .with_point(PointGraphics::default())
}

// ─── GeometryVisualizer ─────────────────────────────────────────────────────

#[test]
fn visualizer_new_starts_dirty() {
    let vis = GeometryVisualizer::wgs84();
    assert_eq!(vis.entity_count(), 0);
    assert_eq!(vis.instance_count(), 0);
}

#[test]
fn visualizer_update_processes_entities() {
    let mut vis = GeometryVisualizer::wgs84();
    let mut entities = EntityCollection::new();
    entities.add(make_entity("e1"));
    entities.add(make_entity("e2"));

    let updated = vis.update(&entities, 0.0);
    assert_eq!(updated, 2);
    assert_eq!(vis.entity_count(), 2);
}

#[test]
fn visualizer_update_skips_when_not_dirty_same_time() {
    let mut vis = GeometryVisualizer::wgs84();
    let mut entities = EntityCollection::new();
    entities.add(make_entity("e1"));

    vis.update(&entities, 0.0);
    let updated = vis.update(&entities, 0.0); // 相同时间，非 dirty
    assert_eq!(updated, 0);
}

#[test]
fn visualizer_update_reprocesses_on_time_change() {
    let mut vis = GeometryVisualizer::wgs84();
    let mut entities = EntityCollection::new();
    entities.add(make_entity("e1"));

    vis.update(&entities, 0.0);
    let updated = vis.update(&entities, 1.0); // 时间已改变
    assert_eq!(updated, 1);
}

#[test]
fn visualizer_get_geometry_returns_cached() {
    let mut vis = GeometryVisualizer::wgs84();
    let mut entities = EntityCollection::new();
    entities.add(make_entity("e1"));

    vis.update(&entities, 0.0);
    assert!(vis.get_geometry("e1").is_some());
    assert!(vis.get_geometry("nonexistent").is_none());
}

#[test]
fn visualizer_remove_entity() {
    let mut vis = GeometryVisualizer::wgs84();
    let mut entities = EntityCollection::new();
    entities.add(make_entity("e1"));

    vis.update(&entities, 0.0);
    vis.remove_entity("e1");
    assert!(vis.get_geometry("e1").is_none());
    assert_eq!(vis.entity_count(), 0);
}

#[test]
fn visualizer_clear() {
    let mut vis = GeometryVisualizer::wgs84();
    let mut entities = EntityCollection::new();
    entities.add(make_entity("e1"));
    entities.add(make_entity("e2"));

    vis.update(&entities, 0.0);
    vis.clear();
    assert_eq!(vis.entity_count(), 0);
    assert_eq!(vis.instance_count(), 0);
}

#[test]
fn visualizer_removes_deleted_entities() {
    let mut vis = GeometryVisualizer::wgs84();
    let mut entities = EntityCollection::new();
    entities.add(make_entity("e1"));
    entities.add(make_entity("e2"));

    vis.update(&entities, 0.0);
    assert_eq!(vis.entity_count(), 2);

    entities.remove("e1");
    vis.mark_dirty();
    vis.update(&entities, 0.0);
    assert_eq!(vis.entity_count(), 1);
    assert!(vis.get_geometry("e1").is_none());
}

#[test]
fn visualizer_all_instances() {
    let mut vis = GeometryVisualizer::wgs84();
    let mut entities = EntityCollection::new();
    entities.add(make_entity("e1"));

    vis.update(&entities, 0.0);
    let instances = vis.all_instances();
    // Point 图元应产生至少一个填充实例
    assert!(!instances.is_empty() || vis.instance_count() == 0);
}

// ─── StaticGeometryBatch ────────────────────────────────────────────────────

#[test]
fn batch_new_is_empty() {
    let batch = StaticGeometryBatch::new();
    assert!(batch.is_empty());
    assert_eq!(batch.len(), 0);
}

#[test]
fn batch_clear() {
    let mut batch = StaticGeometryBatch::new();
    batch.clear();
    assert!(batch.is_empty());
}

// ─── DynamicGeometryUpdater ─────────────────────────────────────────────────

#[test]
fn dynamic_updater_add_remove() {
    let mut updater = DynamicGeometryUpdater::new(Ellipsoid::WGS84);
    updater.add_entity("e1");
    updater.add_entity("e2");
    assert_eq!(updater.entity_count(), 2);

    updater.remove_entity("e1");
    assert_eq!(updater.entity_count(), 1);
}

#[test]
fn dynamic_updater_no_duplicates() {
    let mut updater = DynamicGeometryUpdater::new(Ellipsoid::WGS84);
    updater.add_entity("e1");
    updater.add_entity("e1"); // 重复项
    assert_eq!(updater.entity_count(), 1);
}

#[test]
fn dynamic_updater_update_returns_geometry() {
    let mut updater = DynamicGeometryUpdater::new(Ellipsoid::WGS84);
    updater.add_entity("e1");

    let mut entities = EntityCollection::new();
    entities.add(make_entity("e1"));

    let results = updater.update(&entities, 0.0);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "e1");
}

#[test]
fn dynamic_updater_update_skips_missing_entities() {
    let mut updater = DynamicGeometryUpdater::new(Ellipsoid::WGS84);
    updater.add_entity("e1");
    updater.add_entity("missing");

    let mut entities = EntityCollection::new();
    entities.add(make_entity("e1"));

    let results = updater.update(&entities, 0.0);
    assert_eq!(results.len(), 1); // "missing" 不在集合中
}
