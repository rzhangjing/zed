//! QuadtreePrimitive 遍历扩展 specs - 参考自 QuadtreePrimitiveSpec
//!
//! 测试 QuadtreeTile SSE 计算、子瓦片坐标、
//! 使用 mock 瓦片提供者的 QuadtreePrimitive.traverse、TraversalResult。

use cesium_geospatial::bounding::BoundingSphere;
use cesium_quadtree::{
    QuadtreeConfig, QuadtreePrimitive, QuadtreeTile, TileState, TraversalResult,
};
use glam::DVec3;

fn make_tile(x: u32, y: u32, level: u32, geometric_error: f64) -> QuadtreeTile {
    QuadtreeTile::new(
        x,
        y,
        level,
        BoundingSphere {
            center: DVec3::new(0.0, 0.0, 0.0),
            radius: 1000000.0,
        },
        geometric_error,
    )
}

fn make_loaded_tile(x: u32, y: u32, level: u32, geometric_error: f64) -> QuadtreeTile {
    let mut tile = make_tile(x, y, level, geometric_error);
    tile.state = TileState::Loaded;
    tile
}

// ─── QuadtreeTile ──────────────────────────────────────────────────────────

#[test]
fn tile_new_defaults() {
    let tile = make_tile(0, 0, 0, 100.0);
    assert_eq!(tile.x, 0);
    assert_eq!(tile.y, 0);
    assert_eq!(tile.level, 0);
    assert!((tile.geometric_error - 100.0).abs() < 1e-10);
    assert!(tile.has_content);
    assert!(tile.refineable);
    assert_eq!(tile.state, TileState::Unloaded);
}

#[test]
fn tile_children_coords_root() {
    let tile = make_tile(0, 0, 0, 100.0);
    let children = tile.children_coords();
    assert_eq!(children[0], (0, 0));
    assert_eq!(children[1], (1, 0));
    assert_eq!(children[2], (0, 1));
    assert_eq!(children[3], (1, 1));
}

#[test]
fn tile_children_coords_level1() {
    let tile = make_tile(1, 1, 1, 50.0);
    let children = tile.children_coords();
    assert_eq!(children[0], (2, 2));
    assert_eq!(children[1], (3, 2));
    assert_eq!(children[2], (2, 3));
    assert_eq!(children[3], (3, 3));
}

#[test]
fn tile_children_coords_level2() {
    let tile = make_tile(3, 2, 2, 25.0);
    let children = tile.children_coords();
    assert_eq!(children[0], (6, 4));
    assert_eq!(children[1], (7, 4));
    assert_eq!(children[2], (6, 5));
    assert_eq!(children[3], (7, 5));
}

#[test]
fn tile_screen_space_error_close_camera() {
    let mut tile = make_tile(0, 0, 0, 1000.0);
    tile.bounding_sphere = BoundingSphere {
        center: DVec3::ZERO,
        radius: 100.0,
    };

    // 相机非常近 → 高 SSE
    let camera = DVec3::new(0.0, 0.0, 200.0);
    let sse = tile.compute_screen_space_error(camera, 1080.0, std::f64::consts::FRAC_PI_4);

    // distance = 200 - 100 = 100
    // sse_denom = 2 * tan(PI/8) ≈ 0.828
    // SSE = (1000 * 1080) / (100 * 0.828) ≈ 13037
    assert!(sse > 10000.0);
}

#[test]
fn tile_screen_space_error_far_camera() {
    let mut tile = make_tile(0, 0, 0, 1000.0);
    tile.bounding_sphere = BoundingSphere {
        center: DVec3::ZERO,
        radius: 100.0,
    };

    // 相机很远 → 低 SSE
    let camera = DVec3::new(0.0, 0.0, 1000000.0);
    let sse = tile.compute_screen_space_error(camera, 1080.0, std::f64::consts::FRAC_PI_4);

    // distance ≈ 999900
    // SSE = (1000 * 1080) / (999900 * 0.828) ≈ 1.3
    assert!(sse < 5.0);
}

#[test]
fn tile_screen_space_error_minimum_distance() {
    let mut tile = make_tile(0, 0, 0, 100.0);
    tile.bounding_sphere = BoundingSphere {
        center: DVec3::ZERO,
        radius: 1000.0,
    };

    // 相机在包围球内 → distance 被钳制到 1.0
    let camera = DVec3::new(0.0, 0.0, 500.0);
    let sse = tile.compute_screen_space_error(camera, 1080.0, std::f64::consts::FRAC_PI_4);

    // distance = max(500 - 1000, 1.0) = 1.0
    // SSE = (100 * 1080) / (1.0 * 0.828) ≈ 130,374
    assert!(sse > 100000.0);
}

// ─── TileState ─────────────────────────────────────────────────────────────

#[test]
fn tile_state_default() {
    assert_eq!(TileState::default(), TileState::Unloaded);
}

#[test]
fn tile_state_variants() {
    let states = [
        TileState::Unloaded,
        TileState::Loading,
        TileState::Loaded,
        TileState::Rendered,
        TileState::Refined,
    ];
    // 全部互异
    for i in 0..states.len() {
        for j in (i + 1)..states.len() {
            assert_ne!(states[i], states[j]);
        }
    }
}

// ─── QuadtreeConfig ────────────────────────────────────────────────────────

#[test]
fn quadtree_config_defaults() {
    let config = QuadtreeConfig::default();
    assert!((config.maximum_screen_space_error - 2.0).abs() < 1e-10);
    assert_eq!(config.maximum_level, 22);
    assert_eq!(config.minimum_level, 0);
    assert!(!config.fog_culling);
}

// ─── QuadtreePrimitive 遍历 ──────────────────────────────────────────

#[test]
fn traverse_single_tile_no_refine() {
    // 相机很远 → SSE 低于阈值 → 不细分
    let root = make_tile(0, 0, 0, 100.0);
    let config = QuadtreeConfig {
        maximum_screen_space_error: 2.0,
        maximum_level: 22,
        ..Default::default()
    };
    let primitive = QuadtreePrimitive::new(vec![root], config);

    let camera = DVec3::new(0.0, 0.0, 10000000.0); // 很远
    let result = primitive.traverse(camera, 1080.0, std::f64::consts::FRAC_PI_4, &|_, _, _| None);

    assert_eq!(result.tiles_visited, 1);
    assert_eq!(result.tiles_to_render.len(), 1);
    assert_eq!(result.max_depth, 0);
}

#[test]
fn traverse_refines_with_loaded_children() {
    // 相机很近 → SSE 高于阈值 → 细分
    let mut root = make_tile(0, 0, 0, 100000.0);
    root.bounding_sphere = BoundingSphere {
        center: DVec3::ZERO,
        radius: 100.0,
    };

    let config = QuadtreeConfig {
        maximum_screen_space_error: 2.0,
        maximum_level: 1, // 限制为一层细分
        ..Default::default()
    };
    let primitive = QuadtreePrimitive::new(vec![root], config);

    let camera = DVec3::new(0.0, 0.0, 200.0); // 很近

    // 提供已加载的子瓦片
    let result = primitive.traverse(camera, 1080.0, std::f64::consts::FRAC_PI_4, &|x, y, level| {
        let mut child = make_loaded_tile(x, y, level, 50000.0);
        child.bounding_sphere = BoundingSphere {
            center: DVec3::ZERO,
            radius: 50.0,
        };
        Some(child)
    });

    // 应已访问根 + 4 个子瓦片
    assert!(result.tiles_visited > 1);
    assert!(result.max_depth >= 1);
}

#[test]
fn traverse_unloaded_children_fallback_to_parent() {
    // 相机很近 → SSE 高于阈值 → 尝试细分
    // 但子瓦片未加载 → 渲染父瓦片作为回退
    let mut root = make_tile(0, 0, 0, 100000.0);
    root.bounding_sphere = BoundingSphere {
        center: DVec3::ZERO,
        radius: 100.0,
    };

    let config = QuadtreeConfig {
        maximum_screen_space_error: 2.0,
        maximum_level: 22,
        ..Default::default()
    };
    let primitive = QuadtreePrimitive::new(vec![root], config);

    let camera = DVec3::new(0.0, 0.0, 200.0);

    // 提供未加载的子瓦片
    let result = primitive.traverse(camera, 1080.0, std::f64::consts::FRAC_PI_4, &|x, y, level| {
        Some(make_tile(x, y, level, 50000.0)) // state = Unloaded
    });

    // 父瓦片应作为回退位于 tiles_to_render 中
    assert!(!result.tiles_to_render.is_empty());
    // 子瓦片应位于 tiles_to_load 中
    assert!(!result.tiles_to_load.is_empty());
}

#[test]
fn traverse_respects_maximum_level() {
    // 位于最大层级的瓦片 → 即使 SSE 很高也不应细分
    let mut root = make_tile(0, 0, 22, 100000.0);
    root.bounding_sphere = BoundingSphere {
        center: DVec3::ZERO,
        radius: 100.0,
    };

    let config = QuadtreeConfig {
        maximum_screen_space_error: 2.0,
        maximum_level: 22,
        ..Default::default()
    };
    let primitive = QuadtreePrimitive::new(vec![root], config);

    let camera = DVec3::new(0.0, 0.0, 200.0); // 很近 → 高 SSE
    let result = primitive.traverse(camera, 1080.0, std::f64::consts::FRAC_PI_4, &|_, _, _| None);

    // 应渲染根而不细分
    assert_eq!(result.tiles_to_render.len(), 1);
    assert_eq!(result.max_depth, 22);
}

#[test]
fn traverse_non_refineable_tile() {
    let mut root = make_tile(0, 0, 0, 100000.0);
    root.bounding_sphere = BoundingSphere {
        center: DVec3::ZERO,
        radius: 100.0,
    };
    root.refineable = false;

    let config = QuadtreeConfig::default();
    let primitive = QuadtreePrimitive::new(vec![root], config);

    let camera = DVec3::new(0.0, 0.0, 200.0);
    let result = primitive.traverse(camera, 1080.0, std::f64::consts::FRAC_PI_4, &|_, _, _| None);

    // 应渲染而不细分
    assert_eq!(result.tiles_to_render.len(), 1);
    assert_eq!(result.tiles_visited, 1);
}

#[test]
fn traversal_result_default() {
    let result = TraversalResult::default();
    assert!(result.tiles_to_render.is_empty());
    assert!(result.tiles_to_load.is_empty());
    assert_eq!(result.tiles_visited, 0);
    assert_eq!(result.max_depth, 0);
}

#[test]
fn traverse_multiple_roots() {
    // 两个根瓦片（类似 WGS84 半球）
    let root1 = make_tile(0, 0, 0, 100.0);
    let root2 = make_tile(1, 0, 0, 100.0);

    let config = QuadtreeConfig::default();
    let primitive = QuadtreePrimitive::new(vec![root1, root2], config);

    let camera = DVec3::new(0.0, 0.0, 10000000.0);
    let result = primitive.traverse(camera, 1080.0, std::f64::consts::FRAC_PI_4, &|_, _, _| None);

    // 两个根都被访问和渲染
    assert_eq!(result.tiles_visited, 2);
    assert_eq!(result.tiles_to_render.len(), 2);
}
