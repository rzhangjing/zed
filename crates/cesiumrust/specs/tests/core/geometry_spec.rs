//! 几何生成规格测试 - 参考自 Core/*GeometrySpec
//! 覆盖范围：Corridor、Ellipse、Wall、Polyline、PolylineVolume、CoplanarPolygon、
//! GroundPolyline、Frustum 几何及其轮廓变体。

use cesium_geospatial::geometry::{
    coplanar_polygon_geometry, corridor_geometry, corridor_outline_geometry,
    ellipse_geometry, ellipse_outline_geometry, frustum_geometry, frustum_outline_geometry,
    ground_polyline_geometry, polyline_geometry, polyline_volume_geometry,
    wall_geometry, wall_outline_geometry,
    CoplanarPolygonOptions, CornerType, CorridorOptions, EllipseOptions, FrustumDef,
    GroundPolylineOptions, PolylineOptions, PolylineVolumeOptions, WallOptions,
};
use cesium_geospatial::{Ellipsoid, PerspectiveFrustum, VertexFormat};
use glam::DVec3;

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

fn sample_positions() -> Vec<DVec3> {
    let e = wgs84();
    vec![
        e.cartographic_to_cartesian(&cesium_geospatial::Cartographic::from_degrees(0.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&cesium_geospatial::Cartographic::from_degrees(1.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&cesium_geospatial::Cartographic::from_degrees(1.0, 1.0, 0.0)),
    ]
}

// ─── Corridor ───────────────────────────────────────────────────────────────

#[test]
fn corridor_geometry_produces_positions_and_indices() {
    let opts = CorridorOptions {
        positions: sample_positions(),
        width: 100_000.0,
        corner_type: CornerType::Rounded,
        ellipsoid: wgs84(),
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(!geo.positions.is_empty(), "corridor should produce positions");
    assert!(!geo.indices.is_empty(), "corridor should produce indices");
    assert!(geo.indices.len() % 3 == 0, "indices should be triangles");
}

#[test]
fn corridor_width_affects_geometry() {
    // 沿赤道的直线走廊（向东方向），宽度 = 100km
    // 在 lon≈0、lat=0 处：北方向 ≈ ECEF 中的 z 轴
    // 因此 positions 的 z 范围应近似等于宽度
    use cesium_geospatial::Cartographic;
    let e = wgs84();
    let width = 100_000.0;
    let opts = CorridorOptions {
        positions: vec![
            e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            e.cartographic_to_cartesian(&Cartographic::from_degrees(0.5, 0.0, 0.0)),
        ],
        width,
        corner_type: CornerType::Mitered,
        ellipsoid: e,
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(!geo.positions.is_empty());

    // 计算 z 范围（在赤道处垂直于向东的路径）
    let mut min_z = f64::MAX;
    let mut max_z = f64::MIN;
    for p in &geo.positions {
        if p[2] < min_z { min_z = p[2]; }
        if p[2] > max_z { max_z = p[2]; }
    }
    let z_extent = max_z - min_z;

    // z 范围应近似等于宽度（±30%）
    assert!(
        (z_extent - width).abs() < width * 0.3,
        "corridor z-extent should \u{2248} width: expected ~{width}, got {z_extent}"
    );
}

#[test]
fn corridor_outline_geometry_produces_line_indices() {
    let opts = CorridorOptions {
        positions: sample_positions(),
        width: 50_000.0,
        corner_type: CornerType::Mitered,
        ellipsoid: wgs84(),
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = corridor_outline_geometry(&opts);
    assert!(!geo.positions.is_empty());
    assert!(!geo.indices.is_empty());
    assert!(geo.indices.len() % 2 == 0, "outline indices should be line pairs");
}

#[test]
fn corridor_corner_type_variants() {
    assert_eq!(CornerType::default(), CornerType::Rounded);
    assert_ne!(CornerType::Rounded, CornerType::Mitered);
    assert_ne!(CornerType::Mitered, CornerType::Beveled);
}

// ─── Ellipse ────────────────────────────────────────────────────────────────

#[test]
fn ellipse_geometry_produces_fill() {
    let e = wgs84();
    let center = e.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let opts = EllipseOptions {
        center,
        semi_major_axis: 500_000.0,
        semi_minor_axis: 300_000.0,
        rotation: 0.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_AND_NORMAL);
    assert!(geo.positions.len() >= 4, "ellipse should have multiple vertices");
    assert!(geo.normals.is_some(), "should generate normals with POSITION_AND_NORMAL");
    assert!(!geo.indices.is_empty());
}

#[test]
fn ellipse_outline_geometry_produces_lines() {
    let e = wgs84();
    let center = e.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(10.0, 20.0, 0.0),
    );
    let opts = EllipseOptions {
        center,
        semi_major_axis: 200_000.0,
        semi_minor_axis: 200_000.0,
        rotation: 0.0,
        granularity: std::f64::consts::PI / 90.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = ellipse_outline_geometry(&opts);
    assert!(!geo.positions.is_empty());
    assert!(!geo.indices.is_empty());
    assert!(geo.indices.len() % 2 == 0, "outline should be line pairs");
}

// ─── Wall ───────────────────────────────────────────────────────────────────

#[test]
fn wall_geometry_produces_fill() {
    let opts = WallOptions {
        positions: sample_positions(),
        maximum_heights: None,
        minimum_heights: None,
        ellipsoid: wgs84(),
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(!geo.positions.is_empty());
    assert!(!geo.indices.is_empty());
}

#[test]
fn wall_outline_geometry_produces_lines() {
    let opts = WallOptions {
        positions: sample_positions(),
        maximum_heights: Some(vec![1000.0, 1000.0, 1000.0]),
        minimum_heights: Some(vec![0.0, 0.0, 0.0]),
        ellipsoid: wgs84(),
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = wall_outline_geometry(&opts);
    assert!(!geo.positions.is_empty());
    assert!(!geo.indices.is_empty());
    assert!(geo.indices.len() % 2 == 0);
}

// ─── Polyline ───────────────────────────────────────────────────────────────

#[test]
fn polyline_geometry_produces_positions() {
    let opts = PolylineOptions {
        positions: sample_positions(),
        width: 10.0,
        ellipsoid: wgs84(),
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = polyline_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(!geo.positions.is_empty());
    assert!(!geo.indices.is_empty());
}

// ─── PolylineVolume ─────────────────────────────────────────────────────────

#[test]
fn polyline_volume_geometry_produces_positions() {
    let opts = PolylineVolumeOptions {
        positions: sample_positions(),
        shape: vec![
            [-50.0, -50.0],
            [50.0, -50.0],
            [50.0, 50.0],
            [-50.0, 50.0],
        ],
        ellipsoid: wgs84(),
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = polyline_volume_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(!geo.positions.is_empty());
    assert!(!geo.indices.is_empty());
}

// ─── CoplanarPolygon ────────────────────────────────────────────────────────

#[test]
fn coplanar_polygon_geometry_produces_fill() {
    let opts = CoplanarPolygonOptions {
        positions: sample_positions(),
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = coplanar_polygon_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(!geo.positions.is_empty());
    assert!(!geo.indices.is_empty());
}

// ─── GroundPolyline ─────────────────────────────────────────────────────────

#[test]
fn ground_polyline_geometry_produces_positions() {
    let opts = GroundPolylineOptions {
        positions: sample_positions(),
        width: 5.0,
        ellipsoid: wgs84(),
        granularity: std::f64::consts::PI / 180.0,
        ..Default::default()
    };
    let geo = ground_polyline_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(!geo.positions.is_empty());
    assert!(!geo.indices.is_empty());
}

// ─── Frustum ────────────────────────────────────────────────────────────────

#[test]
fn frustum_geometry_produces_box() {
    let frustum = FrustumDef::Perspective(PerspectiveFrustum {
        fov: std::f64::consts::FRAC_PI_4,
        aspect_ratio: 1.0,
        near: 1.0,
        far: 100.0,
        x_offset: 0.0,
        y_offset: 0.0,
    });
    let origin = DVec3::ZERO;
    let orientation = glam::DQuat::IDENTITY;
    let geo = frustum_geometry(&frustum, origin, orientation, VertexFormat::POSITION_ONLY);
    assert!(geo.positions.len() >= 8, "frustum should have at least 8 corners");
    assert!(!geo.indices.is_empty());
}

#[test]
fn frustum_outline_geometry_produces_lines() {
    let frustum = FrustumDef::Perspective(PerspectiveFrustum {
        fov: std::f64::consts::FRAC_PI_3,
        aspect_ratio: 1.5,
        near: 0.5,
        far: 50.0,
        x_offset: 0.0,
        y_offset: 0.0,
    });
    let origin = DVec3::new(1.0, 2.0, 3.0);
    let orientation = glam::DQuat::IDENTITY;
    let geo = frustum_outline_geometry(&frustum, origin, orientation);
    assert!(geo.positions.len() >= 8);
    assert!(!geo.indices.is_empty(), "frustum outline should have indices");
}

// ─── VertexFormat ───────────────────────────────────────────────────────────

#[test]
fn vertex_format_presets() {
    let all = VertexFormat::ALL;
    assert!(all.position && all.normal && all.st && all.tangent && all.bitangent);

    let pos = VertexFormat::POSITION_ONLY;
    assert!(pos.position && !pos.normal && !pos.st);

    let default = VertexFormat::default();
    assert_eq!(default, VertexFormat::ALL);
}
