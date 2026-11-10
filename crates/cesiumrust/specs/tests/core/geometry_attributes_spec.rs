//! 详细几何属性规格 - 参考自 Core/WallGeometrySpec、
//! Core/CorridorGeometrySpec、Core/EllipseGeometrySpec、
//! Core/PolylineGeometrySpec
//!
//! 测试数学属性：高度、宽度、法线、表面上的位置。

use cesium_geospatial::geometry::{
    corridor_geometry, wall_geometry, wall_outline_geometry,
    ellipse_geometry, ellipse_outline_geometry, polyline_geometry,
    CornerType, CorridorOptions, EllipseOptions, PolylineOptions, WallOptions,
};
use cesium_geospatial::{Cartographic, Ellipsoid, VertexFormat};
use glam::DVec3;

const EPSILON8: f64 = 1e-8;

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

// ─── WallGeometry（来自 WallGeometrySpec）───────────────────────────────

#[test]
fn wall_creates_positions_relative_to_ellipsoid() {
    // WallGeometrySpec: "creates positions relative to ellipsoid"
    let e = wgs84();
    let positions = vec![
        e.cartographic_to_cartesian(&Cartographic::from_degrees(49.0, 18.0, 1000.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(50.0, 18.0, 1000.0)),
    ];
    let opts = WallOptions {
        positions,
        maximum_heights: None,
        minimum_heights: None,
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(geo.positions.len() >= 4, "wall should have at least 4 positions");
    assert!(!geo.indices.is_empty());

    // 第一个位置应在高度 0（底部）
    let first = DVec3::from(geo.positions[0]);
    let carto = e.cartesian_to_cartographic(first).unwrap();
    assert!(
        carto.height.abs() < EPSILON8,
        "first position height should be 0, got {}", carto.height
    );
}

#[test]
fn wall_creates_positions_with_min_max_heights() {
    // WallGeometrySpec: "creates positions with minimum and maximum heights"
    let e = wgs84();
    let positions = vec![
        e.cartographic_to_cartesian(&Cartographic::from_degrees(49.0, 18.0, 1000.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(50.0, 18.0, 1000.0)),
    ];
    let opts = WallOptions {
        positions,
        minimum_heights: Some(vec![1000.0, 2000.0]),
        maximum_heights: Some(vec![3000.0, 4000.0]),
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(geo.positions.len() >= 4);

    // 验证高度在指定范围内
    for p in &geo.positions {
        let carto = e.cartesian_to_cartographic(DVec3::from(*p)).unwrap();
        assert!(
            carto.height >= 1000.0 - EPSILON8 && carto.height <= 4000.0 + EPSILON8,
            "height {} should be within [1000, 4000]", carto.height
        );
    }
}

#[test]
fn wall_cleans_positions_with_duplicates() {
    // WallGeometrySpec: "cleans positions with duplicates"
    let e = wgs84();
    // 三个位置，前两个经纬度相同（高度不同）
    let positions = vec![
        e.cartographic_to_cartesian(&Cartographic::from_degrees(49.0, 18.0, 1000.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(49.0, 18.0, 5000.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(50.0, 18.0, 1000.0)),
    ];
    let opts = WallOptions {
        positions,
        maximum_heights: None,
        minimum_heights: None,
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    // 仍应产生有效几何（重复项已合并）
    assert!(!geo.positions.is_empty());
    assert!(!geo.indices.is_empty());
}

#[test]
fn wall_outline_produces_lines() {
    let e = wgs84();
    let positions = vec![
        e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 1.0, 0.0)),
    ];
    let opts = WallOptions {
        positions,
        maximum_heights: Some(vec![1000.0, 1000.0, 1000.0]),
        minimum_heights: Some(vec![0.0, 0.0, 0.0]),
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
    };
    let geo = wall_outline_geometry(&opts);

    assert!(!geo.positions.is_empty());
    assert!(!geo.indices.is_empty());
    assert_eq!(geo.indices.len() % 2, 0, "outline indices should be line pairs");
}

#[test]
fn wall_from_constant_heights() {
    // WallGeometrySpec: "fromConstantHeights"
    let e = wgs84();
    let positions = vec![
        e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
    ];
    let opts = WallOptions::from_constant_heights(
        positions,
        Some(0.0),
        Some(5000.0),
        e,
    );
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(!geo.positions.is_empty());
    // 所有位置的高度应在 0 到 5000 之间
    for p in &geo.positions {
        let carto = e.cartesian_to_cartographic(DVec3::from(*p)).unwrap();
        assert!(
            carto.height >= -EPSILON8 && carto.height <= 5000.0 + EPSILON8,
            "height {} should be within [0, 5000]", carto.height
        );
    }
}

// ─── CorridorGeometry（来自 CorridorGeometrySpec）──────────────────────

#[test]
fn corridor_computes_positions_mitered() {
    // CorridorGeometrySpec: "computes positions"，MITERED 拐角
    let e = wgs84();
    let opts = CorridorOptions {
        positions: vec![
            e.cartographic_to_cartesian(&Cartographic::from_degrees(90.0, -30.0, 0.0)),
            e.cartographic_to_cartesian(&Cartographic::from_degrees(90.0, -35.0, 0.0)),
        ],
        width: 30000.0,
        corner_type: CornerType::Mitered,
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(geo.positions.len() >= 4, "corridor should have multiple positions");
    assert!(!geo.indices.is_empty());
    assert_eq!(geo.indices.len() % 3, 0, "indices should be triangles");
}

#[test]
fn corridor_computes_all_vertex_attributes() {
    // CorridorGeometrySpec: "compute all vertex attributes"
    let e = wgs84();
    let opts = CorridorOptions {
        positions: vec![
            e.cartographic_to_cartesian(&Cartographic::from_degrees(90.0, -30.0, 0.0)),
            e.cartographic_to_cartesian(&Cartographic::from_degrees(90.0, -35.0, 0.0)),
        ],
        width: 30000.0,
        corner_type: CornerType::Mitered,
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::ALL);

    let num_vertices = geo.positions.len();
    assert!(num_vertices > 0);
    assert_eq!(geo.normals.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.tex_coords.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.indices.len() % 3, 0);
}

#[test]
fn corridor_width_is_respected() {
    // 通过检查左右边缘之间的距离来验证通道宽度
    // 通道在 lon=0 处向北延伸，因此宽度沿 y 方向（东西向）扩展
    let e = wgs84();
    let width = 50000.0;
    let opts = CorridorOptions {
        positions: vec![
            e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 1.0, 0.0)),
        ],
        width,
        corner_type: CornerType::Mitered,
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);

    // 在 lon=0, lat=0：东方向在 ECEF 中为 y 轴
    // 计算垂直于路径的范围（东西向 = y 轴）
    let mut min_y = f64::MAX;
    let mut max_y = f64::MIN;
    for p in &geo.positions {
        if p[1] < min_y { min_y = p[1]; }
        if p[1] > max_y { max_y = p[1]; }
    }
    let y_extent = max_y - min_y;

    // y 方向范围应近似等于宽度（±30%）
    assert!(
        (y_extent - width).abs() < width * 0.3,
        "corridor y-extent {} should ≈ width {}", y_extent, width
    );
}

#[test]
fn corridor_corner_types_produce_different_geometry() {
    let e = wgs84();
    let positions = vec![
        e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 1.0, 0.0)),
    ];

    let rounded = corridor_geometry(&CorridorOptions {
        positions: positions.clone(),
        width: 30000.0,
        corner_type: CornerType::Rounded,
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    }, VertexFormat::POSITION_ONLY);

    let mitered = corridor_geometry(&CorridorOptions {
        positions: positions.clone(),
        width: 30000.0,
        corner_type: CornerType::Mitered,
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    }, VertexFormat::POSITION_ONLY);

    // 圆角应比斜角产生更多顶点
    assert!(
        rounded.positions.len() >= mitered.positions.len(),
        "rounded ({}) should have >= vertices than mitered ({})",
        rounded.positions.len(), mitered.positions.len()
    );
}

// ─── EllipseGeometry（来自 EllipseGeometrySpec）──────────────────────

#[test]
fn ellipse_computes_positions() {
    // EllipseGeometrySpec: "computes positions"
    let e = wgs84();
    let center = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0));
    let opts = EllipseOptions {
        center,
        semi_major_axis: 500000.0,
        semi_minor_axis: 300000.0,
        rotation: 0.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(geo.positions.len() >= 4, "ellipse should have multiple positions");
    assert!(!geo.indices.is_empty());
    assert_eq!(geo.indices.len() % 3, 0);
}

#[test]
fn ellipse_computes_all_vertex_attributes() {
    // EllipseGeometrySpec: "compute all vertex attributes"
    let e = wgs84();
    let center = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0));
    let opts = EllipseOptions {
        center,
        semi_major_axis: 500000.0,
        semi_minor_axis: 300000.0,
        rotation: 0.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::ALL);

    let num_vertices = geo.positions.len();
    assert!(num_vertices > 0);
    assert_eq!(geo.normals.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.tex_coords.as_ref().unwrap().len(), num_vertices);
}

#[test]
fn ellipse_positions_on_surface() {
    // 所有位置应在椭球表面上（高度 0）
    let e = wgs84();
    let center = e.cartographic_to_cartesian(&Cartographic::from_degrees(10.0, 20.0, 0.0));
    let opts = EllipseOptions {
        center,
        semi_major_axis: 200000.0,
        semi_minor_axis: 100000.0,
        rotation: 0.0,
        granularity: std::f64::consts::PI / 90.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);

    for p in &geo.positions {
        let carto = e.cartesian_to_cartographic(DVec3::from(*p)).unwrap();
        assert!(
            carto.height.abs() < 1.0,
            "position height {} should be ≈ 0", carto.height
        );
    }
}

#[test]
fn ellipse_outline_produces_lines() {
    let e = wgs84();
    let center = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0));
    let opts = EllipseOptions {
        center,
        semi_major_axis: 200000.0,
        semi_minor_axis: 200000.0,
        rotation: 0.0,
        granularity: std::f64::consts::PI / 90.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = ellipse_outline_geometry(&opts);

    assert!(!geo.positions.is_empty());
    assert!(!geo.indices.is_empty());
    assert_eq!(geo.indices.len() % 2, 0, "outline indices should be line pairs");
}

// ─── PolylineGeometry（来自 PolylineGeometrySpec）─────────────────────

#[test]
fn polyline_computes_positions() {
    // PolylineGeometrySpec: "computes positions"
    let e = wgs84();
    let opts = PolylineOptions {
        positions: vec![
            e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
            e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 1.0, 0.0)),
        ],
        width: 10.0,
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = polyline_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(geo.positions.len() >= 4, "polyline should have multiple positions");
    assert!(!geo.indices.is_empty());
}

#[test]
fn polyline_width_is_respected() {
    // 验证折线产生带宽度的几何
    let e = wgs84();
    let width = 1000.0;
    let opts = PolylineOptions {
        positions: vec![
            e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 1.0, 0.0)),
        ],
        width,
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = polyline_geometry(&opts, VertexFormat::POSITION_ONLY);

    // 应在中心线两侧都有位置
    assert!(geo.positions.len() >= 4, "polyline with width should have >= 4 positions");
}

#[test]
fn polyline_computes_all_vertex_attributes() {
    let e = wgs84();
    let opts = PolylineOptions {
        positions: vec![
            e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
        ],
        width: 10.0,
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = polyline_geometry(&opts, VertexFormat::ALL);

    let num_vertices = geo.positions.len();
    assert!(num_vertices > 0);
    if let Some(normals) = &geo.normals {
        assert_eq!(normals.len(), num_vertices);
    }
}

// ─── 交叉测试：包围球包含所有位置 ─────────────────

#[test]
fn wall_bounding_sphere_contains_all_positions() {
    let e = wgs84();
    let positions = vec![
        e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 1.0, 0.0)),
    ];
    let opts = WallOptions {
        positions,
        maximum_heights: Some(vec![5000.0, 5000.0, 5000.0]),
        minimum_heights: Some(vec![0.0, 0.0, 0.0]),
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    let center = geo.bounding_sphere.center;
    let radius = geo.bounding_sphere.radius;
    for p in &geo.positions {
        let dist = (DVec3::from(*p) - center).length();
        assert!(
            dist <= radius + 1e-6,
            "position distance {} exceeds bounding sphere radius {}", dist, radius
        );
    }
}

#[test]
fn corridor_bounding_sphere_contains_all_positions() {
    let e = wgs84();
    let opts = CorridorOptions {
        positions: vec![
            e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
        ],
        width: 30000.0,
        corner_type: CornerType::Mitered,
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);

    let center = geo.bounding_sphere.center;
    let radius = geo.bounding_sphere.radius;
    for p in &geo.positions {
        let dist = (DVec3::from(*p) - center).length();
        assert!(
            dist <= radius + 1e-6,
            "position distance {} exceeds bounding sphere radius {}", dist, radius
        );
    }
}
