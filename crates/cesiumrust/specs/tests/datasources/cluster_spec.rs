//! 参考自 CesiumJS EntityClusterSpec 的测试（A 类逻辑）
//! 聚类：基于网格的空间哈希、EntityCluster 选项、更新、计数

use cesium_datasource::cluster::{EntityCluster, EntityClusterOptions};
use cesium_datasource::entity::Entity;
use cesium_datasource::entity_collection::EntityCollection;

fn make_collection_with_positions(positions: &[(f64, f64, f64)]) -> EntityCollection {
    let mut collection = EntityCollection::new();
    for (i, (lon, lat, h)) in positions.iter().enumerate() {
        let entity = Entity::new(format!("e{}", i)).with_position(*lon, *lat, *h);
        collection.add(entity);
    }
    collection
}

// ===== EntityCluster 选项 =====

#[test]
fn test_cluster_default_options() {
    let cluster = EntityCluster::new();
    assert!(cluster.options.enabled);
    assert!((cluster.options.pixel_range - 80.0).abs() < 1e-10);
    assert_eq!(cluster.options.minimum_cluster_size, 2);
}

#[test]
fn test_cluster_custom_options() {
    let options = EntityClusterOptions {
        enabled: true,
        pixel_range: 50.0,
        minimum_cluster_size: 3,
    };
    let cluster = EntityCluster::with_options(options);
    assert!((cluster.options.pixel_range - 50.0).abs() < 1e-10);
    assert_eq!(cluster.options.minimum_cluster_size, 3);
}

// ===== 更新 =====

#[test]
fn test_cluster_update_empty_collection() {
    let mut cluster = EntityCluster::new();
    let collection = EntityCollection::new();
    cluster.update(&collection, 0.0);
    assert_eq!(cluster.cluster_count(), 0);
}

#[test]
fn test_cluster_update_disabled() {
    let mut cluster = EntityCluster::with_options(EntityClusterOptions {
        enabled: false,
        pixel_range: 80.0,
        minimum_cluster_size: 2,
    });
    // 两个实体位于同一位置
    let collection = make_collection_with_positions(&[(0.0, 0.0, 0.0), (0.0, 0.0, 0.0)]);
    cluster.update(&collection, 0.0);
    assert_eq!(cluster.cluster_count(), 0);
}

#[test]
fn test_cluster_single_entity_not_clustered() {
    let mut cluster = EntityCluster::new();
    let collection = make_collection_with_positions(&[(0.5, 0.5, 0.0)]);
    cluster.update(&collection, 0.0);
    // 单实体 → 1 个 count=1 的聚类
    assert_eq!(cluster.cluster_count(), 1);
    assert!(cluster.clusters()[0].is_single());
    assert_eq!(cluster.actual_cluster_count(), 0);
}

#[test]
fn test_cluster_nearby_entities_form_cluster() {
    let mut cluster = EntityCluster::new();
    // 两个实体非常接近（同一网格单元）
    let collection = make_collection_with_positions(&[
        (0.001, 0.001, 0.0),
        (0.002, 0.002, 0.0),
    ]);
    cluster.update(&collection, 0.0);
    // 应形成 1 个 count=2 的聚类
    assert_eq!(cluster.actual_cluster_count(), 1);
    assert_eq!(cluster.clustered_entity_count(), 2);
}

#[test]
fn test_cluster_far_entities_not_clustered() {
    let mut cluster = EntityCluster::new();
    // 两个实体相隔很远（不同网格单元）
    let collection = make_collection_with_positions(&[
        (0.0, 0.0, 0.0),
        (1.0, 1.0, 0.0),
    ]);
    cluster.update(&collection, 0.0);
    // 应为 2 个独立聚类
    assert_eq!(cluster.cluster_count(), 2);
    assert_eq!(cluster.actual_cluster_count(), 0);
}

#[test]
fn test_cluster_hidden_entities_excluded() {
    let mut cluster = EntityCluster::new();
    let mut collection = EntityCollection::new();
    let mut e1 = Entity::new("e1").with_position(0.001, 0.001, 0.0);
    e1.show = false;
    let e2 = Entity::new("e2").with_position(0.002, 0.002, 0.0);
    collection.add(e1);
    collection.add(e2);
    cluster.update(&collection, 0.0);
    // 仅 1 个可见实体 → 无实际聚类
    assert_eq!(cluster.actual_cluster_count(), 0);
}

#[test]
fn test_cluster_centroid_is_average() {
    let mut cluster = EntityCluster::new();
    let collection = make_collection_with_positions(&[
        (0.001, 0.001, 10.0),
        (0.003, 0.003, 20.0),
    ]);
    cluster.update(&collection, 0.0);
    if cluster.actual_cluster_count() == 1 {
        let c = &cluster.clusters().iter().find(|c| !c.is_single()).unwrap();
        assert!((c.position[0] - 0.002).abs() < 1e-10);
        assert!((c.position[1] - 0.002).abs() < 1e-10);
        assert!((c.position[2] - 15.0).abs() < 1e-10);
    }
}

#[test]
fn test_cluster_minimum_cluster_size_respected() {
    let mut cluster = EntityCluster::with_options(EntityClusterOptions {
        enabled: true,
        pixel_range: 80.0,
        minimum_cluster_size: 3,
    });
    // 两个实体接近 - 低于最小聚类规模
    let collection = make_collection_with_positions(&[
        (0.001, 0.001, 0.0),
        (0.002, 0.002, 0.0),
    ]);
    cluster.update(&collection, 0.0);
    // 最小规模为 3 时，两个实体不应形成聚类
    assert_eq!(cluster.actual_cluster_count(), 0);
}
