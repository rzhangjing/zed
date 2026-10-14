//! Core/PolylinePipelineSpec → Rust 集成测试
//! 15 个原始 it() 块 → 移植 8 个 A 类测试
//!
//! 跳过的 C 类测试：
//! - "generateArc throws without positions" - 编译期类型安全
//! - "generateRhumbArc throws without positions" - 编译期类型安全
//!
//! 跳过（generateRhumbArc 尚未实现 - 5 个测试）：
//! - generateRhumbArc：height/subdivides/empty/one position/return values

use cesium_geospatial::polyline_pipeline::{generate_arc, wrap_longitude, ArcOptions};
use cesium_geospatial::transforms::east_north_up_to_fixed_frame;
use cesium_geospatial::{Cartographic, Ellipsoid};
use cesium_specs::{assert_vec3_epsilon, epsilon};
use glam::DVec3;
use std::f64::consts::FRAC_PI_2;

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

fn from_degrees(lon: f64, lat: f64, height: f64) -> DVec3 {
    wgs84().cartographic_to_cartesian(&Cartographic::from_degrees(lon, lat, height))
}

// ============================================================================
// wrapLongitude
// ============================================================================

#[test]
fn wrap_longitude_basic() {
    let positions = vec![
        from_degrees(-75.163789, 39.952335, 0.0),
        from_degrees(-80.2264393, 25.7889689, 0.0),
    ];
    let result = wrap_longitude(&positions, None);
    assert_eq!(result.lengths.len(), 1);
    assert_eq!(result.lengths[0], 2);
}

#[test]
fn wrap_longitude_empty_array() {
    let result = wrap_longitude(&[], None);
    assert_eq!(result.lengths.len(), 0);
}

#[test]
fn wrap_longitude_breaks_polyline_into_segments() {
    let positions = vec![
        from_degrees(-179.0, 39.0, 0.0),
        from_degrees(2.0, 25.0, 0.0),
    ];
    let result = wrap_longitude(&positions, None);
    assert_eq!(result.lengths.len(), 2);
    assert_eq!(result.lengths[0], 2);
    assert_eq!(result.lengths[1], 2);
}

#[test]
fn wrap_longitude_breaks_polyline_into_segments_with_model_matrix() {
    let center = from_degrees(-179.0, 39.0, 0.0);
    let matrix = east_north_up_to_fixed_frame(center, &wgs84());

    let positions = vec![DVec3::ZERO, DVec3::new(0.0, 100000000.0, 0.0)];
    let result = wrap_longitude(&positions, Some(&matrix));
    assert_eq!(result.lengths.len(), 2);
    assert_eq!(result.lengths[0], 2);
    assert_eq!(result.lengths[1], 2);
}

// ============================================================================
// generateArc
// ============================================================================

#[test]
fn generate_arc_accepts_height_for_single_value() {
    let positions = vec![from_degrees(0.0, 0.0, 0.0)];
    let heights = [30.0];
    let opts = ArcOptions {
        positions: &positions,
        heights: Some(&heights),
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: &wgs84(),
    };
    let result = generate_arc(&opts);
    assert_eq!(result.len(), 1);
    let expected = from_degrees(0.0, 0.0, 30.0);
    assert_vec3_epsilon!(result[0], expected, epsilon::EPSILON6);
}

#[test]
fn generate_arc_subdivides_in_half() {
    let p1 = from_degrees(0.0, 0.0, 0.0);
    let p2 = from_degrees(90.0, 0.0, 0.0);
    let p3 = from_degrees(45.0, 0.0, 0.0); // 期望的中点

    let positions = vec![p1, p2];
    let opts = ArcOptions {
        positions: &positions,
        heights: None,
        granularity: FRAC_PI_2 / 2.0, // PI_OVER_TWO / 2
        ellipsoid: &wgs84(),
    };
    let result = generate_arc(&opts);

    // 应产生 3 个点：起点、中点、终点
    assert_eq!(result.len(), 3, "expected 3 points, got {}", result.len());
    assert_vec3_epsilon!(result[0], p1, epsilon::EPSILON4);
    assert_vec3_epsilon!(result[2], p2, epsilon::EPSILON4);
    assert_vec3_epsilon!(result[1], p3, epsilon::EPSILON4);
}

#[test]
fn generate_arc_works_with_empty_array() {
    let opts = ArcOptions {
        positions: &[],
        heights: None,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: &wgs84(),
    };
    let result = generate_arc(&opts);
    assert_eq!(result.len(), 0);
}

#[test]
fn generate_arc_works_with_one_position() {
    let unit_sphere = Ellipsoid::UNIT_SPHERE;
    let positions = vec![DVec3::Z]; // 单位球上的 UNIT_Z
    let opts = ArcOptions {
        positions: &positions,
        heights: None,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: &unit_sphere,
    };
    let result = generate_arc(&opts);
    assert_eq!(result.len(), 1);
    assert_vec3_epsilon!(result[0], DVec3::new(0.0, 0.0, 1.0), epsilon::EPSILON15);
}
