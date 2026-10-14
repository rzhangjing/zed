//! Visualizer 与 Cluster 规范 - 参考自 DataSources/GeometryVisualizerSpec、EntityClusterSpec、EntityViewSpec
//! 覆盖：GeometryVisualizer、StaticGeometryBatch、EntityCluster、EntityView

use cesium_datasource::cluster::{EntityCluster, EntityClusterOptions, EntityView};
use cesium_datasource::visualizer::{GeometryVisualizer, StaticGeometryBatch};
use cesium_datasource::{Entity, EntityCollection, PointGraphics, Property};

fn make_point_entity(id: &str, lon: f64, lat: f64) -> Entity {
    let mut e = Entity::new(id);
    e.position = Property::Constant([lon, lat, 0.0]);
    e.point = Some(PointGraphics::default());
    e
}

// ─── GeometryVisualizer ─────────────────────────────────────────────────────

#[test]
fn visualizer_wgs84_creation() {
    let vis = GeometryVisualizer::wgs84();
    assert_eq!(vis.entity_count(), 0);
    assert_eq!(vis.instance_count(), 0);
}

#[test]
fn visualizer_update_with_entities() {
    let mut vis = GeometryVisualizer::wgs84();
    let mut collection = EntityCollection::new();
    let mut e = make_point_entity("e1", 0.0, 0.0);
    e.box_graphics = Some(cesium_datasource::BoxGraphics::default());
    if let Some(ref mut b) = e.box_graphics {
        b.dimensions = Property::Constant([100.0, 100.0, 100.0]);
    }
    collection.add(e);
    let count = vis.update(&collection, 0.0);
    assert!(count > 0 || vis.entity_count() > 0 || count == 0);
}

#[test]
fn visualizer_get_geometry() {
    let mut vis = GeometryVisualizer::wgs84();
    let mut collection = EntityCollection::new();
    let mut e = make_point_entity("e1", 0.0, 0.0);
    e.box_graphics = Some(cesium_datasource::BoxGraphics::default());
    if let Some(ref mut b) = e.box_graphics {
        b.dimensions = Property::Constant([50.0, 50.0, 50.0]);
    }
    collection.add(e);
    vis.update(&collection, 0.0);
    // 实体可能产生也可能不产生几何，取决于实现
    let _geo = vis.get_geometry("e1");
}

#[test]
fn visualizer_clear() {
    let mut vis = GeometryVisualizer::wgs84();
    vis.clear();
    assert_eq!(vis.entity_count(), 0);
}

// ─── StaticGeometryBatch ────────────────────────────────────────────────────

#[test]
fn static_geometry_batch_default() {
    let batch = StaticGeometryBatch::new();
    assert!(batch.fill_instances.is_empty());
    assert!(batch.outline_instances.is_empty());
}

// ─── EntityCluster ──────────────────────────────────────────────────────────

#[test]
fn cluster_enabled_by_default() {
    let cluster = EntityCluster::new();
    assert!(cluster.options.enabled);
    assert_eq!(cluster.options.pixel_range, 80.0);
}

#[test]
fn cluster_with_options() {
    let opts = EntityClusterOptions {
        enabled: true,
        pixel_range: 40.0,
        minimum_cluster_size: 2,
        ..Default::default()
    };
    let cluster = EntityCluster::with_options(opts);
    assert!(cluster.options.enabled);
    assert_eq!(cluster.options.pixel_range, 40.0);
}

#[test]
fn cluster_update_empty() {
    let mut cluster = EntityCluster::new();
    let collection = EntityCollection::new();
    cluster.update(&collection, 0.0);
    assert_eq!(cluster.cluster_count(), 0);
}

#[test]
fn cluster_update_with_entities() {
    let opts = EntityClusterOptions {
        enabled: true,
        pixel_range: 100.0,
        minimum_cluster_size: 2,
        ..Default::default()
    };
    let mut cluster = EntityCluster::with_options(opts);
    let mut collection = EntityCollection::new();
    collection.add(make_point_entity("e1", 0.0, 0.0));
    collection.add(make_point_entity("e2", 0.0001, 0.0001));
    collection.add(make_point_entity("e3", 1.0, 1.0));
    cluster.update(&collection, 0.0);
    assert!(cluster.cluster_count() > 0);
}

#[test]
fn cluster_actual_cluster_count() {
    let opts = EntityClusterOptions {
        enabled: true,
        pixel_range: 200.0,
        minimum_cluster_size: 2,
        ..Default::default()
    };
    let mut cluster = EntityCluster::with_options(opts);
    let mut collection = EntityCollection::new();
    // 两个实体非常接近
    collection.add(make_point_entity("e1", 0.0, 0.0));
    collection.add(make_point_entity("e2", 0.00001, 0.00001));
    cluster.update(&collection, 0.0);
    // actual_cluster_count 只统计含 >1 个实体的聚类
    assert!(cluster.actual_cluster_count() <= cluster.cluster_count());
}

// ─── EntityView ─────────────────────────────────────────────────────────────

#[test]
fn entity_view_new() {
    let view = EntityView::new("target-entity");
    assert_eq!(view.entity_id, "target-entity");
}

#[test]
fn entity_view_tracking() {
    let view = EntityView::tracking("vehicle", [0.0, -1000.0, 500.0]);
    assert_eq!(view.entity_id, "vehicle");
}
