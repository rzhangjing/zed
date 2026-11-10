//! Core/GeometryPipelineSpec、PolygonPipelineSpec、PolylinePipelineSpec
//! → 几何管线函数的 Rust 集成测试

use cesium_geospatial::geometry::{
    box_geometry, box_outline_geometry, compute_area2d, compute_normal,
    compute_tangent_and_bitangent, compute_winding_order, cylinder_geometry,
    cylinder_outline_geometry, ellipsoid_geometry, ellipsoid_outline_geometry, generate_arc,
    plane_geometry, plane_outline_geometry, rectangle_geometry, rectangle_outline_geometry,
    sphere_geometry, to_wireframe, triangulate_polygon, PrimitiveType,
    VertexFormat, WindingOrder,
};
use cesium_geospatial::math_utils::to_radians;
use cesium_geospatial::rectangle::Rectangle;
use cesium_geospatial::Ellipsoid;
use glam::{DVec2, DVec3};

// === 几何生成器 ===

#[test]
fn test_ellipsoid_geometry() {
    let geo = ellipsoid_geometry(DVec3::splat(1.0), 16, 32, VertexFormat::ALL);
    assert_eq!(geo.positions.len(), 17 * 33);
    assert!(geo.normals.is_some());
    assert!(geo.tex_coords.is_some());
    assert_eq!(geo.indices.len(), 16 * 32 * 6);
    assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
}

#[test]
fn test_sphere_geometry() {
    let geo = sphere_geometry(5.0, 8, 16, VertexFormat::POSITION_ONLY);
    assert_eq!(geo.positions.len(), 9 * 17);
    assert!(geo.normals.is_none());
    assert!((geo.bounding_sphere.radius - 5.0).abs() < 1e-10);
}

#[test]
fn test_box_geometry() {
    let geo = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::ALL,
    );
    assert_eq!(geo.positions.len(), 24);
    assert_eq!(geo.indices.len(), 36);
}

#[test]
fn test_cylinder_geometry() {
    let geo = cylinder_geometry(2.0, 1.0, 1.0, 16, VertexFormat::ALL);
    assert!(!geo.positions.is_empty());
    assert!(geo.normals.is_some());
    assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
}

#[test]
fn test_plane_geometry() {
    let geo = plane_geometry(VertexFormat::ALL);
    assert_eq!(geo.positions.len(), 4);
    assert_eq!(geo.indices.len(), 6);
}

#[test]
fn test_rectangle_geometry() {
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let geo = rectangle_geometry(
        &rect,
        &Ellipsoid::WGS84,
        to_radians(1.0),
        0.0,
        VertexFormat::ALL,
    );
    assert!(geo.positions.len() > 4);
    assert!(geo.normals.is_some());
}

// === 轮廓几何 ===

#[test]
fn test_box_outline_geometry() {
    let geo = box_outline_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0));
    assert_eq!(geo.positions.len(), 8);
    assert_eq!(geo.indices.len(), 24);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn test_ellipsoid_outline_geometry() {
    let geo = ellipsoid_outline_geometry(DVec3::new(1.0, 2.0, 3.0), 16, 32);
    assert!(!geo.positions.is_empty());
    assert_eq!(geo.indices.len() % 2, 0);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn test_cylinder_outline_geometry() {
    let geo = cylinder_outline_geometry(2.0, 1.0, 1.0, 16);
    assert!(!geo.positions.is_empty());
    assert_eq!(geo.indices.len() % 2, 0);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn test_plane_outline_geometry() {
    let geo = plane_outline_geometry();
    assert_eq!(geo.positions.len(), 4);
    assert_eq!(geo.indices.len(), 8);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn test_rectangle_outline_geometry() {
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let geo = rectangle_outline_geometry(&rect, &Ellipsoid::WGS84, to_radians(1.0));
    assert!(!geo.positions.is_empty());
    assert_eq!(geo.indices.len() % 2, 0);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

// === PolygonPipeline ===

#[test]
fn test_triangulate_polygon_quad() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(1.0, 1.0),
        DVec2::new(0.0, 1.0),
    ];
    let indices = triangulate_polygon(&positions, &[]);
    assert_eq!(indices.len(), 6); // 一个四边形对应 2 个三角形
}

#[test]
fn test_triangulate_polygon_triangle() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(0.5, 1.0),
    ];
    let indices = triangulate_polygon(&positions, &[]);
    assert_eq!(indices.len(), 3); // 1 个三角形
}

#[test]
fn test_compute_area2d_unit_square() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(1.0, 1.0),
        DVec2::new(0.0, 1.0),
    ];
    let area = compute_area2d(&positions);
    assert!((area - 1.0).abs() < 1e-10);
}

#[test]
fn test_compute_area2d_triangle() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(2.0, 0.0),
        DVec2::new(0.0, 2.0),
    ];
    let area = compute_area2d(&positions);
    assert!((area - 2.0).abs() < 1e-10);
}

#[test]
fn test_winding_order_ccw() {
    let ccw = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(0.0, 1.0),
    ];
    assert_eq!(compute_winding_order(&ccw), WindingOrder::CounterClockwise);
}

#[test]
fn test_winding_order_cw() {
    let cw = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(0.0, 1.0),
        DVec2::new(1.0, 0.0),
    ];
    assert_eq!(compute_winding_order(&cw), WindingOrder::Clockwise);
}

// === GeometryPipeline ===

#[test]
fn test_compute_normal() {
    let mut geo = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::POSITION_ONLY,
    );
    assert!(geo.normals.is_none());
    compute_normal(&mut geo);
    assert!(geo.normals.is_some());
    let normals = geo.normals.unwrap();
    assert_eq!(normals.len(), geo.positions.len());
    // 所有法线都应为单位长度
    for n in &normals {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6);
    }
}

#[test]
fn test_compute_tangent_and_bitangent() {
    let mut geo = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::ALL,
    );
    assert!(geo.tangents.is_none());
    compute_tangent_and_bitangent(&mut geo);
    assert!(geo.tangents.is_some());
    assert!(geo.bitangents.is_some());
    let tangents = geo.tangents.unwrap();
    assert_eq!(tangents.len(), geo.positions.len());
}

#[test]
fn test_to_wireframe() {
    let mut geo = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::POSITION_ONLY,
    );
    assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
    let tri_count = geo.indices.len() / 3;
    to_wireframe(&mut geo);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    assert_eq!(geo.indices.len(), tri_count * 6);
}

// === PolylinePipeline (generate_arc) ===

#[test]
fn test_generate_arc() {
    let ellipsoid = Ellipsoid::WGS84;
    let start = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let end = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(10.0, 0.0, 0.0),
    );
    let arc = generate_arc(&[start, end], to_radians(1.0), &ellipsoid);
    assert!(arc.len() > 2); // 应包含中间点
}

#[test]
fn test_generate_arc_single_point() {
    let ellipsoid = Ellipsoid::WGS84;
    let point = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let arc = generate_arc(&[point], to_radians(1.0), &ellipsoid);
    assert_eq!(arc.len(), 1);
}

// === VertexFormat ===

#[test]
fn test_vertex_format_all() {
    let vf = VertexFormat::ALL;
    assert!(vf.position);
    assert!(vf.normal);
    assert!(vf.st);
    assert!(vf.tangent);
    assert!(vf.bitangent);
}

#[test]
fn test_vertex_format_position_only() {
    let vf = VertexFormat::POSITION_ONLY;
    assert!(vf.position);
    assert!(!vf.normal);
    assert!(!vf.st);
    assert!(!vf.tangent);
    assert!(!vf.bitangent);
}
