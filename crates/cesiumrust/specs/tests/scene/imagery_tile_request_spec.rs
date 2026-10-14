//! 影像瓦片请求 specs - compute_tile_requests/compute_texture_mapping/ImageryLayer
//! 参考自 Scene/ImageryLayerSpec（A 类瓦片请求计算）

use cesium_imagery::{ImageryLayer, compute_tile_requests, compute_texture_mapping};
use cesium_geospatial::rectangle::Rectangle;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::tiling_scheme::TilingScheme;

fn geographic_scheme() -> TilingScheme {
    TilingScheme::geographic(Ellipsoid::WGS84)
}

fn web_mercator_scheme() -> TilingScheme {
    TilingScheme::web_mercator(Ellipsoid::WGS84)
}

// ─── ImageryLayer 构建器 ───────────────────────────────────────────────────

#[test]
fn imagery_layer_new_defaults() {
    let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE);
    assert_eq!(layer.id, 1);
    assert!((layer.alpha - 1.0).abs() < 1e-10);
    assert!((layer.brightness - 1.0).abs() < 1e-10);
    assert!((layer.contrast - 1.0).abs() < 1e-10);
    assert!((layer.saturation - 1.0).abs() < 1e-10);
    assert!((layer.gamma - 1.0).abs() < 1e-10);
    assert!(layer.show);
    assert_eq!(layer.minimum_level, 0);
    assert_eq!(layer.maximum_level, 25);
    assert_eq!(layer.tile_width, 256);
    assert_eq!(layer.tile_height, 256);
}

#[test]
fn imagery_layer_with_show() {
    let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE).with_show(false);
    assert!(!layer.show);
}

#[test]
fn imagery_layer_with_level_range() {
    let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE).with_level_range(3, 18);
    assert_eq!(layer.minimum_level, 3);
    assert_eq!(layer.maximum_level, 18);
}

#[test]
fn imagery_layer_with_alpha() {
    let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE).with_alpha(0.5);
    assert!((layer.alpha - 0.5).abs() < 1e-10);
}

// ─── compute_tile_requests ──────────────────────────────────────────────────

#[test]
fn tile_requests_full_coverage_level0() {
    let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE);
    let terrain_rect = Rectangle::MAX_VALUE;
    let scheme = geographic_scheme();

    let requests = compute_tile_requests(&layer, &terrain_rect, 0, &scheme);

    // 在 level 0，geographic 方案有 2x1 个瓦片
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.level == 0));
    assert!(requests.iter().all(|r| r.layer_id == 1));
}

#[test]
fn tile_requests_hidden_layer_returns_empty() {
    let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE).with_show(false);
    let terrain_rect = Rectangle::MAX_VALUE;
    let scheme = geographic_scheme();

    let requests = compute_tile_requests(&layer, &terrain_rect, 0, &scheme);
    assert!(requests.is_empty());
}

#[test]
fn tile_requests_level_clamped_to_min() {
    let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE).with_level_range(3, 10);
    let terrain_rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let scheme = geographic_scheme();

    let requests = compute_tile_requests(&layer, &terrain_rect, 0, &scheme);
    assert!(requests.iter().all(|r| r.level == 3));
}

#[test]
fn tile_requests_level_clamped_to_max() {
    let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE).with_level_range(0, 5);
    let terrain_rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let scheme = geographic_scheme();

    let requests = compute_tile_requests(&layer, &terrain_rect, 20, &scheme);
    assert!(requests.iter().all(|r| r.level == 5));
}

#[test]
fn tile_requests_no_intersection_returns_empty() {
    // 图层仅覆盖西半球
    let layer_rect = Rectangle::from_degrees(-180.0, -90.0, 0.0, 90.0);
    let layer = ImageryLayer::new(1, layer_rect);
    // 东半球的地形瓦片
    let terrain_rect = Rectangle::from_degrees(10.0, 10.0, 20.0, 20.0);
    let scheme = geographic_scheme();

    let requests = compute_tile_requests(&layer, &terrain_rect, 2, &scheme);
    assert!(requests.is_empty());
}

#[test]
fn tile_requests_partial_intersection() {
    // 图层覆盖西半球
    let layer_rect = Rectangle::from_degrees(-180.0, -90.0, 0.0, 90.0);
    let layer = ImageryLayer::new(1, layer_rect);
    // 地形瓦片跨越边界
    let terrain_rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let scheme = geographic_scheme();

    let requests = compute_tile_requests(&layer, &terrain_rect, 2, &scheme);
    // 应有一些请求（仅西部部分）
    assert!(!requests.is_empty());
}

#[test]
fn tile_requests_web_mercator_scheme() {
    let layer = ImageryLayer::new(2, Rectangle::MAX_VALUE);
    let terrain_rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let scheme = web_mercator_scheme();

    let requests = compute_tile_requests(&layer, &terrain_rect, 1, &scheme);
    assert!(!requests.is_empty());
    assert!(requests.iter().all(|r| r.layer_id == 2));
    assert!(requests.iter().all(|r| r.level == 1));
}

#[test]
fn tile_requests_higher_level_more_tiles() {
    let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE);
    let terrain_rect = Rectangle::from_degrees(-45.0, -45.0, 45.0, 45.0);
    let scheme = geographic_scheme();

    let requests_l0 = compute_tile_requests(&layer, &terrain_rect, 0, &scheme);
    let requests_l2 = compute_tile_requests(&layer, &terrain_rect, 2, &scheme);

    // 更高层级应有更多（或相等）的瓦片请求
    assert!(requests_l2.len() >= requests_l0.len());
}

// ─── compute_texture_mapping ────────────────────────────────────────────────

#[test]
fn texture_mapping_identity() {
    let rect = Rectangle::from_degrees(-90.0, -45.0, 90.0, 45.0);
    let (translation, scale) = compute_texture_mapping(&rect, &rect);

    assert!((translation[0]).abs() < 1e-10);
    assert!((translation[1]).abs() < 1e-10);
    assert!((scale[0] - 1.0).abs() < 1e-10);
    assert!((scale[1] - 1.0).abs() < 1e-10);
}

#[test]
fn texture_mapping_quadrant() {
    let terrain_rect = Rectangle::from_degrees(-180.0, -90.0, 0.0, 0.0);
    let imagery_rect = Rectangle::from_degrees(-180.0, -90.0, 180.0, 90.0);

    let (translation, scale) = compute_texture_mapping(&terrain_rect, &imagery_rect);

    // 地形覆盖 SW 象限
    assert!((translation[0]).abs() < 1e-10); // 西边缘对齐
    assert!((translation[1]).abs() < 1e-10); // 南边缘对齐
    assert!((scale[0] - 0.5).abs() < 1e-10); // 一半宽度
    assert!((scale[1] - 0.5).abs() < 1e-10); // 一半高度
}

#[test]
fn texture_mapping_offset() {
    let terrain_rect = Rectangle::from_degrees(0.0, 0.0, 90.0, 45.0);
    let imagery_rect = Rectangle::from_degrees(-180.0, -90.0, 180.0, 90.0);

    let (translation, scale) = compute_texture_mapping(&terrain_rect, &imagery_rect);

    // translation_x = (0 - (-180)) / 360 = 0.5
    assert!((translation[0] - 0.5).abs() < 1e-10);
    // translation_y = (0 - (-90)) / 180 = 0.5
    assert!((translation[1] - 0.5).abs() < 1e-10);
    // scale_x = 90 / 360 = 0.25
    assert!((scale[0] - 0.25).abs() < 1e-10);
    // scale_y = 45 / 180 = 0.25
    assert!((scale[1] - 0.25).abs() < 1e-10);
}

#[test]
fn texture_mapping_small_terrain_in_large_imagery() {
    let terrain_rect = Rectangle::from_degrees(10.0, 10.0, 11.0, 11.0);
    let imagery_rect = Rectangle::from_degrees(0.0, 0.0, 20.0, 20.0);

    let (translation, scale) = compute_texture_mapping(&terrain_rect, &imagery_rect);

    // translation_x = (10 - 0) / 20 = 0.5
    assert!((translation[0] - 0.5).abs() < 1e-10);
    // translation_y = (10 - 0) / 20 = 0.5
    assert!((translation[1] - 0.5).abs() < 1e-10);
    // scale = 1/20 = 0.05
    assert!((scale[0] - 0.05).abs() < 1e-10);
    assert!((scale[1] - 0.05).abs() < 1e-10);
}

#[test]
fn texture_mapping_full_terrain_in_smaller_imagery() {
    // 地形大于影像 → scale > 1
    let terrain_rect = Rectangle::from_degrees(-180.0, -90.0, 180.0, 90.0);
    let imagery_rect = Rectangle::from_degrees(-90.0, -45.0, 90.0, 45.0);

    let (_translation, scale) = compute_texture_mapping(&terrain_rect, &imagery_rect);

    // scale_x = 360 / 180 = 2.0
    assert!((scale[0] - 2.0).abs() < 1e-10);
    // scale_y = 180 / 90 = 2.0
    assert!((scale[1] - 2.0).abs() < 1e-10);
}
