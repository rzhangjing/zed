//! 几何工具规格测试 - generate_arc/triangulate_polygon/compute_area2d/winding_order
//! 参考自 Core/GeometryPipelineSpec + Core/PolygonPipelineSpec（A 类工具路径）

use cesium_geospatial::geometry::{
    compute_area2d, compute_winding_order, generate_arc, triangulate_polygon, WindingOrder,
};
use cesium_geospatial::Ellipsoid;
use glam::{DVec2, DVec3};

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

// ─── generate_arc ───────────────────────────────────────────────────────────

#[test]
fn arc_single_point_returns_same() {
    let positions = vec![DVec3::new(6378137.0, 0.0, 0.0)];
    let result = generate_arc(&positions, 0.01, &wgs84());
    assert_eq!(result.len(), 1);
    assert!((result[0].x - 6378137.0).abs() < 1.0);
}

#[test]
fn arc_two_points_produces_intermediates() {
    let e = wgs84();
    // 赤道上的两点，相距 10 度
    let start = e.cartographic_to_cartesian(&cesium_geospatial::Cartographic::from_degrees(0.0, 0.0, 0.0));
    let end = e.cartographic_to_cartesian(&cesium_geospatial::Cartographic::from_degrees(10.0, 0.0, 0.0));
    let positions = vec![start, end];
    // granularity = 以弧度计的 1 度
    let granularity = (1.0f64).to_radians();
    let result = generate_arc(&positions, granularity, &e);
    // 应有约 10 段 + 1 个端点
    assert!(result.len() >= 10, "expected >= 10 points, got {}", result.len());
    // 最后一个点应为终点
    let last = result.last().unwrap();
    assert!((last.x - end.x).abs() < 1.0);
    assert!((last.y - end.y).abs() < 1.0);
}

#[test]
fn arc_empty_returns_empty() {
    let result = generate_arc(&[], 0.01, &wgs84());
    assert!(result.is_empty());
}

#[test]
fn arc_coarse_granularity_min_segments() {
    let e = wgs84();
    let start = e.cartographic_to_cartesian(&cesium_geospatial::Cartographic::from_degrees(0.0, 0.0, 0.0));
    let end = e.cartographic_to_cartesian(&cesium_geospatial::Cartographic::from_degrees(1.0, 0.0, 0.0));
    // 极粗的 granularity（大于距离）→ 仍至少有 1 段
    let result = generate_arc(&[start, end], std::f64::consts::PI, &e);
    assert!(result.len() >= 2, "should have at least start + end");
}

// ─── triangulate_polygon ────────────────────────────────────────────────────

#[test]
fn triangulate_triangle() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(0.0, 1.0),
    ];
    let indices = triangulate_polygon(&positions, &[]);
    assert_eq!(indices.len(), 3, "triangle should produce 3 indices");
}

#[test]
fn triangulate_quad() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(1.0, 1.0),
        DVec2::new(0.0, 1.0),
    ];
    let indices = triangulate_polygon(&positions, &[]);
    assert_eq!(indices.len(), 6, "quad should produce 6 indices (2 triangles)");
}

#[test]
fn triangulate_pentagon() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(2.0, 0.0),
        DVec2::new(3.0, 1.0),
        DVec2::new(1.0, 3.0),
        DVec2::new(-1.0, 1.0),
    ];
    let indices = triangulate_polygon(&positions, &[]);
    assert_eq!(indices.len(), 9, "pentagon should produce 9 indices (3 triangles)");
}

#[test]
fn triangulate_less_than_3_points() {
    let positions = vec![DVec2::new(0.0, 0.0), DVec2::new(1.0, 0.0)];
    let indices = triangulate_polygon(&positions, &[]);
    assert!(indices.is_empty());
}

#[test]
fn triangulate_with_hole() {
    // 外部正方形 + 内部正方形孔洞
    let positions = vec![
        // 外侧（逆时针）
        DVec2::new(0.0, 0.0),
        DVec2::new(10.0, 0.0),
        DVec2::new(10.0, 10.0),
        DVec2::new(0.0, 10.0),
        // 内侧孔洞（顺时针）
        DVec2::new(3.0, 3.0),
        DVec2::new(7.0, 3.0),
        DVec2::new(7.0, 7.0),
        DVec2::new(3.0, 7.0),
    ];
    let indices = triangulate_polygon(&positions, &[4]);
    // 8 个顶点带 1 个孔洞 → (8-2)*3 = 18 个索引？实际 (n-2)*3，n=8 → 18
    assert!(!indices.is_empty(), "should produce triangles");
    assert!(indices.len() >= 18, "expected >= 18 indices, got {}", indices.len());
}

// ─── compute_area2d ─────────────────────────────────────────────────────────

#[test]
fn area_unit_square_ccw() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(1.0, 1.0),
        DVec2::new(0.0, 1.0),
    ];
    let area = compute_area2d(&positions);
    assert!((area - 1.0).abs() < 1e-10, "CCW unit square should have area +1");
}

#[test]
fn area_unit_square_cw() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(0.0, 1.0),
        DVec2::new(1.0, 1.0),
        DVec2::new(1.0, 0.0),
    ];
    let area = compute_area2d(&positions);
    assert!((area - (-1.0)).abs() < 1e-10, "CW unit square should have area -1");
}

#[test]
fn area_triangle() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(4.0, 0.0),
        DVec2::new(0.0, 3.0),
    ];
    let area = compute_area2d(&positions);
    assert!((area - 6.0).abs() < 1e-10, "3-4-5 triangle area = 6");
}

#[test]
fn area_less_than_3_points() {
    let positions = vec![DVec2::new(0.0, 0.0), DVec2::new(1.0, 0.0)];
    let area = compute_area2d(&positions);
    assert!(area.abs() < 1e-10);
}

#[test]
fn area_larger_polygon() {
    // 半径为 1 的正六边形：面积 = (3*sqrt(3))/2 ≈ 2.598
    let positions: Vec<DVec2> = (0..6)
        .map(|i| {
            let angle = i as f64 * std::f64::consts::PI / 3.0;
            DVec2::new(angle.cos(), angle.sin())
        })
        .collect();
    let area = compute_area2d(&positions);
    let expected = 3.0 * 3.0f64.sqrt() / 2.0;
    assert!((area - expected).abs() < 1e-10, "hexagon area mismatch: {} vs {}", area, expected);
}

// ─── compute_winding_order ──────────────────────────────────────────────────

#[test]
fn winding_order_ccw() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(0.0, 1.0),
    ];
    assert_eq!(compute_winding_order(&positions), WindingOrder::CounterClockwise);
}

#[test]
fn winding_order_cw() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(0.0, 1.0),
        DVec2::new(1.0, 0.0),
    ];
    assert_eq!(compute_winding_order(&positions), WindingOrder::Clockwise);
}

#[test]
fn winding_order_square_ccw() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(1.0, 1.0),
        DVec2::new(0.0, 1.0),
    ];
    assert_eq!(compute_winding_order(&positions), WindingOrder::CounterClockwise);
}

#[test]
fn winding_order_square_cw() {
    let positions = vec![
        DVec2::new(0.0, 0.0),
        DVec2::new(0.0, 1.0),
        DVec2::new(1.0, 1.0),
        DVec2::new(1.0, 0.0),
    ];
    assert_eq!(compute_winding_order(&positions), WindingOrder::Clockwise);
}
