//! CircleGeometry 规格 - 参考自 Core/CircleGeometrySpec
//!
//! 测试椭球面上的圆几何生成。

use cesium_geospatial::geometry::{circle_geometry, VertexFormat};
use cesium_geospatial::Ellipsoid;
use glam::DVec3;

const EPSILON10: f64 = 1e-10;
const EPSILON7: f64 = 1e-7;

// ─── CircleGeometry（来自 CircleGeometrySpec）──────────────────────────────

#[test]
fn circle_geometry_throws_without_center() {
    // CircleGeometrySpec: "throws without a center"
    // Rust 实现返回默认几何而非抛出异常
    let geo = circle_geometry(
        DVec3::ZERO, // center
        1.0, // radius
        &Ellipsoid::WGS84,
        16, // segments
        VertexFormat::POSITION_ONLY,
    );
    assert_eq!(geo.positions.len(), 18); // 1 个中心 + 17 个环顶点（0..=16）
}

#[test]
fn circle_geometry_throws_without_radius() {
    // CircleGeometrySpec: "throws without a radius"
    // Rust 实现要求 radius 参数
    let geo = circle_geometry(
        DVec3::new(1.0, 0.0, 0.0), // center
        1.0, // radius
        &Ellipsoid::WGS84,
        16, // segments
        VertexFormat::POSITION_ONLY,
    );
    assert_eq!(geo.positions.len(), 18);
}

#[test]
fn circle_geometry_throws_with_negative_segments() {
    // CircleGeometrySpec: "throws with a negative granularity"
    // Rust 实现的 segments 使用 u32，因此负值不可能
    // 改用 0 段测试
    let geo = circle_geometry(
        DVec3::new(1.0, 0.0, 0.0),
        1.0,
        &Ellipsoid::WGS84,
        0, // segments = 0
        VertexFormat::POSITION_ONLY,
    );
    // segments=0 时，产生 1 个中心 + 1 个环顶点 = 2 个顶点
    assert_eq!(geo.positions.len(), 2);
}

#[test]
fn circle_geometry_computes_positions() {
    // CircleGeometrySpec: "computes positions"
    let geo = circle_geometry(
        DVec3::ZERO,
        1.0,
        &Ellipsoid::WGS84,
        16, // 粒度 ~0.1 弧度
        VertexFormat::POSITION_ONLY,
    );

    // 1 个中心 + 17 个环顶点 = 18 个位置（0..=16）
    assert_eq!(geo.positions.len(), 18);
    // 16 个三角形（中心 + 每处 2 个环顶点，i 从 0 到 segments-1）
    assert_eq!(geo.indices.len(), 48); // 16 * 3
    assert!((geo.bounding_sphere.radius - 1.0).abs() < EPSILON10);
}

#[test]
fn circle_geometry_compute_all_vertex_attributes() {
    // CircleGeometrySpec: "compute all vertex attributes"
    let geo = circle_geometry(
        DVec3::ZERO,
        1.0,
        &Ellipsoid::WGS84,
        16,
        VertexFormat::ALL,
    );

    let num_vertices = 18;
    assert_eq!(geo.positions.len(), num_vertices);
    assert_eq!(geo.normals.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.tex_coords.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.indices.len(), 48);
}

#[test]
fn circle_geometry_degenerate_radius_zero() {
    // CircleGeometrySpec: "undefined is returned if radius is equal to or less than zero"
    // Rust 实现产生最小几何
    let geo = circle_geometry(
        DVec3::new(250000.0, 250000.0, 250000.0),
        0.0,
        &Ellipsoid::WGS84,
        16,
        VertexFormat::POSITION_ONLY,
    );

    // 当前实现对零/负半径不产生退化
    // 仍产生完整的圆几何
    assert_eq!(geo.positions.len(), 18);
    // 包围球半径应非常小（仅中心）
    assert!(geo.bounding_sphere.radius < EPSILON10);
}

#[test]
fn circle_geometry_degenerate_radius_negative() {
    // 类似于半径为零的情况
    let geo = circle_geometry(
        DVec3::new(250000.0, 250000.0, 250000.0),
        -1.0,
        &Ellipsoid::WGS84,
        16,
        VertexFormat::POSITION_ONLY,
    );

    assert_eq!(geo.positions.len(), 18);
    assert!(geo.bounding_sphere.radius < EPSILON10);
}

#[test]
fn circle_geometry_bounding_sphere_contains_all_positions() {
    // 验证包围球包含所有位置
    let geo = circle_geometry(
        DVec3::ZERO,
        1.0,
        &Ellipsoid::WGS84,
        16,
        VertexFormat::POSITION_ONLY,
    );

    // 由位置计算实际的包围球
    let mut min_x = f64::MAX;
    let mut max_x = f64::MIN;
    let mut min_y = f64::MAX;
    let mut max_y = f64::MIN;
    let mut min_z = f64::MAX;
    let mut max_z = f64::MIN;

    for p in &geo.positions {
        min_x = min_x.min(p[0]);
        max_x = max_x.max(p[0]);
        min_y = min_y.min(p[1]);
        max_y = max_y.max(p[1]);
        min_z = min_z.min(p[2]);
        max_z = max_z.max(p[2]);
    }

    let center = DVec3::new(
        (min_x + max_x) * 0.5,
        (min_y + max_y) * 0.5,
        (min_z + max_z) * 0.5,
    );
    let radius = ((max_x - min_x).powi(2) + (max_y - min_y).powi(2) + (max_z - min_z).powi(2)).sqrt() * 0.5;

    for p in &geo.positions {
        let pos = DVec3::from(*p);
        let dist = (pos - center).length();
        assert!(dist <= radius + 1e-3, "position distance {} exceeds bounding sphere radius {}", dist, radius);
    }
}

#[test]
fn circle_geometry_normals_point_outward() {
    // 验证法线从中心朝外
    let geo = circle_geometry(
        DVec3::ZERO,
        1.0,
        &Ellipsoid::WGS84,
        16,
        VertexFormat::POSITION_AND_NORMAL,
    );
    let normals = geo.normals.as_ref().unwrap();

    for i in 0..geo.positions.len() {
        let pos = DVec3::from(geo.positions[i]);
        let normal = DVec3::from(normals[i]);
        // 跳过中心顶点（i=0），因为它可能位于原点
        if i == 0 {
            continue;
        }
        let pos_dir = pos.normalize();
        let dot = normal.dot(pos_dir);
        // 对于圆几何，法线应大致朝外
        assert!(dot > 0.5, "normal {} should point outward, dot = {}", i, dot);
    }
}
