//! Scene/QuadtreePrimitiveSpec、QuadtreeTileSpec → Rust 集成测试

use cesium_quadtree::{
    QuadtreeConfig, QuadtreePrimitive, QuadtreeTile, TileState, TraversalResult,
};
use cesium_geospatial::bounding::BoundingSphere;
use glam::DVec3;

fn make_tile(x: u32, y: u32, level: u32, geometric_error: f64) -> QuadtreeTile {
    QuadtreeTile::new(
        x,
        y,
        level,
        BoundingSphere::new(DVec3::ZERO, 1_000_000.0),
        geometric_error,
    )
}

// === QuadtreeTile ===

#[test]
fn test_quadtree_tile_creation() {
    let tile = make_tile(1, 2, 3, 50000.0);
    assert_eq!(tile.x, 1);
    assert_eq!(tile.y, 2);
    assert_eq!(tile.level, 3);
    assert_eq!(tile.geometric_error, 50000.0);
    assert!(tile.has_content);
    assert!(tile.refineable);
    assert_eq!(tile.state, TileState::Unloaded);
}

#[test]
fn test_quadtree_tile_children_coords() {
    let tile = make_tile(1, 2, 3, 10000.0);
    let children = tile.children_coords();
    assert_eq!(children[0], (2, 4));
    assert_eq!(children[1], (3, 4));
    assert_eq!(children[2], (2, 5));
    assert_eq!(children[3], (3, 5));
}

#[test]
fn test_screen_space_error_positive() {
    let tile = make_tile(0, 0, 0, 10000.0);
    let camera = DVec3::new(0.0, 0.0, 2_000_000.0);
    let sse = tile.compute_screen_space_error(camera, 1080.0, std::f64::consts::FRAC_PI_4);
    assert!(sse > 0.0);
}

#[test]
fn test_sse_decreases_with_distance() {
    let tile = make_tile(0, 0, 0, 10000.0);
    let fov = std::f64::consts::FRAC_PI_4;
    let sse_near = tile.compute_screen_space_error(
        DVec3::new(0.0, 0.0, 1_500_000.0), 1080.0, fov,
    );
    let sse_far = tile.compute_screen_space_error(
        DVec3::new(0.0, 0.0, 5_000_000.0), 1080.0, fov,
    );
    assert!(sse_near > sse_far);
}

#[test]
fn test_sse_increases_with_geometric_error() {
    let tile_low = make_tile(0, 0, 0, 1000.0);
    let tile_high = make_tile(0, 0, 0, 100000.0);
    let camera = DVec3::new(0.0, 0.0, 2_000_000.0);
    let fov = std::f64::consts::FRAC_PI_4;
    let sse_low = tile_low.compute_screen_space_error(camera, 1080.0, fov);
    let sse_high = tile_high.compute_screen_space_error(camera, 1080.0, fov);
    assert!(sse_high > sse_low);
}

// === TileState ===

#[test]
fn test_tile_state_default() {
    assert_eq!(TileState::default(), TileState::Unloaded);
}

#[test]
fn test_tile_state_variants() {
    assert_ne!(TileState::Unloaded, TileState::Loading);
    assert_ne!(TileState::Loading, TileState::Loaded);
    assert_ne!(TileState::Loaded, TileState::Rendered);
    assert_ne!(TileState::Rendered, TileState::Refined);
}

// === QuadtreeConfig ===

#[test]
fn test_quadtree_config_default() {
    let config = QuadtreeConfig::default();
    assert_eq!(config.maximum_screen_space_error, 2.0);
    assert_eq!(config.maximum_level, 22);
    assert_eq!(config.minimum_level, 0);
    assert!(!config.fog_culling);
}

// === QuadtreePrimitive 遍历 ===

#[test]
fn test_traversal_renders_root_when_far() {
    let root = make_tile(0, 0, 0, 100.0);
    let primitive = QuadtreePrimitive::new(
        vec![root],
        QuadtreeConfig {
            maximum_screen_space_error: 2.0,
            maximum_level: 10,
            ..Default::default()
        },
    );

    let camera = DVec3::new(0.0, 0.0, 10_000_000.0);
    let result = primitive.traverse(camera, 1080.0, std::f64::consts::FRAC_PI_4, &|_, _, _| None);

    assert_eq!(result.tiles_to_render.len(), 1);
    assert_eq!(result.tiles_visited, 1);
}

#[test]
fn test_traversal_queues_children_when_close() {
    let root = make_tile(0, 0, 0, 1_000_000.0);
    let primitive = QuadtreePrimitive::new(
        vec![root],
        QuadtreeConfig {
            maximum_screen_space_error: 2.0,
            maximum_level: 10,
            ..Default::default()
        },
    );

    let camera = DVec3::new(0.0, 0.0, 2_000_000.0);
    let result = primitive.traverse(camera, 1080.0, std::f64::consts::FRAC_PI_4, &|x, y, level| {
        if level <= 2 {
            Some(make_tile(x, y, level, 1_000_000.0 / (level as f64 + 1.0)))
        } else {
            None
        }
    });

    // 作为回退渲染根（子节点未加载）
    assert_eq!(result.tiles_to_render.len(), 1);
    // 4 个子节点排队等待加载
    assert_eq!(result.tiles_to_load.len(), 4);
}

#[test]
fn test_traversal_result_default() {
    let result = TraversalResult::default();
    assert!(result.tiles_to_render.is_empty());
    assert!(result.tiles_to_load.is_empty());
    assert_eq!(result.tiles_visited, 0);
    assert_eq!(result.max_depth, 0);
}
