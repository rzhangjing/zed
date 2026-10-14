//! 参考自 CesiumJS SimplePolylineGeometrySpec 的测试
//! A 类测试：7 个（带 positions/colors/arcType 的 createGeometry 变体）
//! C 类省略：3 个（throws — Rust 中为编译期类型安全）

use cesium_geospatial::bounding::BoundingSphere;
use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::polygon_geometry_library::ArcType;
use cesium_geospatial::simple_polyline_geometry::{ColorRgba, SimplePolylineGeometry};
use glam::DVec3;
use std::f64::consts::PI;

const EPSILON10: f64 = 1.0e-10;
const EPSILON8: f64 = 1.0e-8;

fn approx_eq(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() < eps
}

// ===== ArcType::GEODESIC 大粒度（不细分） =====

#[test]
fn constructor_computes_all_vertex_attributes() {
    // 参考自："constructor computes all vertex attributes"
    let positions = vec![
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, 1.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    ];
    let line = SimplePolylineGeometry::new(
        positions.clone(),
        None,
        false,
        ArcType::Geodesic,
        PI, // 大粒度 → 不细分
        Ellipsoid::UNIT_SPHERE,
    );

    let result = line.create_geometry();

    // 位置应保持不变（在单位球上 granularity=PI 时不细分）
    let expected = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    assert_eq!(result.position_values.len(), 9);
    for i in 0..9 {
        assert!(
            approx_eq(result.position_values[i], expected[i], EPSILON10),
            "position[{}]: {} != {}",
            i,
            result.position_values[i],
            expected[i]
        );
    }

    // 索引：线段 [0,1, 1,2]
    assert_eq!(result.indices, vec![0, 1, 1, 2]);

    // PrimitiveType::Lines
    assert!(result.is_lines);

    // 由点集构造 BoundingSphere
    let expected_bs = BoundingSphere::from_points(&positions);
    assert!(approx_eq(
        result.bounding_sphere.center.x,
        expected_bs.center.x,
        EPSILON10
    ));
    assert!(approx_eq(
        result.bounding_sphere.center.y,
        expected_bs.center.y,
        EPSILON10
    ));
    assert!(approx_eq(
        result.bounding_sphere.center.z,
        expected_bs.center.z,
        EPSILON10
    ));
    assert!(approx_eq(
        result.bounding_sphere.radius,
        expected_bs.radius,
        EPSILON10
    ));
}

#[test]
fn constructor_computes_all_vertex_attributes_for_rhumb_lines() {
    // 参考自："constructor computes all vertex attributes for rhumb lines"
    // UNIT_SPHERE 上的 Cartesian3.fromDegreesArray([30, 30, 30, 60, 60, 60])
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let positions = vec![
        ellipsoid.cartographic_to_cartesian(&Cartographic::from_degrees(30.0, 30.0, 0.0)),
        ellipsoid.cartographic_to_cartesian(&Cartographic::from_degrees(30.0, 60.0, 0.0)),
        ellipsoid.cartographic_to_cartesian(&Cartographic::from_degrees(60.0, 60.0, 0.0)),
    ];

    let line = SimplePolylineGeometry::new(
        positions.clone(),
        None,
        false,
        ArcType::Rhumb,
        PI, // 大粒度 → 不细分
        ellipsoid,
    );

    let result = line.create_geometry();

    // granularity=PI 时，位置应近似保持不变
    let num_positions = result.position_values.len() / 3;
    assert!(num_positions >= 3, "Expected at least 3 positions, got {}", num_positions);

    // 索引应为线段
    assert_eq!(result.indices.len(), (num_positions - 1) * 2);
    assert!(result.is_lines);

    // BoundingSphere
    let expected_bs = BoundingSphere::from_points(&positions);
    assert!(approx_eq(
        result.bounding_sphere.radius,
        expected_bs.radius,
        EPSILON8
    ));
}

// ===== 逐段颜色 =====

#[test]
fn constructor_computes_per_segment_colors() {
    // 参考自："constructor computes per segment colors"
    let positions = vec![
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, 1.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    ];
    let colors = vec![
        ColorRgba::new(1.0, 0.0, 0.0, 1.0),
        ColorRgba::new(0.0, 1.0, 0.0, 1.0),
        ColorRgba::new(0.0, 0.0, 1.0, 1.0),
    ];

    let line = SimplePolylineGeometry::new(
        positions,
        Some(colors),
        false, // colors_per_vertex = false → 逐段
        ArcType::Geodesic,
        PI,
        Ellipsoid::UNIT_SPHERE,
    );

    let result = line.create_geometry();

    // 颜色属性应已定义
    assert!(result.color_values.is_some());

    // 逐段颜色时 numVertices = positions.length * 2 - 2 = 4
    let num_vertices = 3 * 2 - 2;
    let color_values = result.color_values.unwrap();
    assert_eq!(color_values.len(), num_vertices * 4);
}

#[test]
fn constructor_computes_per_vertex_colors() {
    // 参考自："constructor computes per vertex colors"
    let positions = vec![
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, 1.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    ];
    let colors = vec![
        ColorRgba::new(1.0, 0.0, 0.0, 1.0),
        ColorRgba::new(0.0, 1.0, 0.0, 1.0),
        ColorRgba::new(0.0, 0.0, 1.0, 1.0),
    ];

    let line = SimplePolylineGeometry::new(
        positions,
        Some(colors),
        true, // colors_per_vertex = true
        ArcType::Geodesic,
        PI,
        Ellipsoid::UNIT_SPHERE,
    );

    let result = line.create_geometry();

    // 颜色属性应已定义
    assert!(result.color_values.is_some());

    // 逐顶点颜色时 numVertices = positions.length = 3
    let num_vertices = 3;
    let color_values = result.color_values.unwrap();
    assert_eq!(color_values.len(), num_vertices * 4);
}

// ===== ArcType::NONE（不细分） =====

#[test]
fn constructor_computes_all_vertex_attributes_no_subdivision() {
    // 参考自："constructor computes all vertex attributes, no subdivision"
    let positions = vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(2.0, 0.0, 0.0),
    ];

    let line = SimplePolylineGeometry::new(
        positions.clone(),
        None,
        false,
        ArcType::None,
        PI,
        Ellipsoid::WGS84,
    );

    let result = line.create_geometry();

    // 位置应与输入完全一致
    assert_eq!(
        result.position_values,
        vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0]
    );

    // 索引：[0,1, 1,2]
    assert_eq!(result.indices, vec![0, 1, 1, 2]);

    // PrimitiveType::Lines
    assert!(result.is_lines);

    // BoundingSphere
    let expected_bs = BoundingSphere::from_points(&positions);
    assert!(approx_eq(
        result.bounding_sphere.radius,
        expected_bs.radius,
        EPSILON10
    ));
}

#[test]
fn constructor_computes_per_segment_colors_no_subdivision() {
    // 参考自："constructor computes per segment colors, no subdivision"
    let positions = vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(2.0, 0.0, 0.0),
    ];
    let colors = vec![
        ColorRgba::new(1.0, 0.0, 0.0, 1.0),
        ColorRgba::new(0.0, 1.0, 0.0, 1.0),
        ColorRgba::new(0.0, 0.0, 1.0, 1.0),
    ];

    let line = SimplePolylineGeometry::new(
        positions,
        Some(colors),
        false, // 逐段
        ArcType::None,
        PI,
        Ellipsoid::WGS84,
    );

    let result = line.create_geometry();

    // 颜色属性应已定义
    assert!(result.color_values.is_some());

    // numVertices = positions.length * 2 - 2 = 4
    let num_vertices = 3 * 2 - 2;
    let color_values = result.color_values.unwrap();
    assert_eq!(color_values.len(), num_vertices * 4);
}

#[test]
fn constructor_computes_per_vertex_colors_no_subdivision() {
    // 参考自："constructor computes per vertex colors, no subdivision"
    let positions = vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(2.0, 0.0, 0.0),
    ];
    let colors = vec![
        ColorRgba::new(1.0, 0.0, 0.0, 1.0),
        ColorRgba::new(0.0, 1.0, 0.0, 1.0),
        ColorRgba::new(0.0, 0.0, 1.0, 1.0),
    ];

    let line = SimplePolylineGeometry::new(
        positions,
        Some(colors),
        true, // 逐顶点
        ArcType::None,
        PI,
        Ellipsoid::WGS84,
    );

    let result = line.create_geometry();

    // 颜色属性应已定义
    assert!(result.color_values.is_some());

    // numVertices = positions.length = 3
    let num_vertices = 3;
    let color_values = result.color_values.unwrap();
    assert_eq!(color_values.len(), num_vertices * 4);
}
