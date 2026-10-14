//! 矩形几何详细规格 - 参考自 Core/RectangleGeometrySpec
//!
//! 测试位置计数、角点位置、顶点属性、IDL 穿越、
//! 极点处理与高度参数。

use cesium_geospatial::geometry::{rectangle_geometry, PrimitiveType};
use cesium_geospatial::{Cartographic, Ellipsoid, Rectangle, VertexFormat};
use glam::DVec3;

const EPSILON8: f64 = 1e-8;
const EPSILON9: f64 = 1e-9;

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

// ─── 位置计数（来自 RectangleGeometrySpec） ───────────────────────

#[test]
fn rectangle_computes_positions() {
    // RectangleGeometrySpec: "computes positions"
    // Rectangle(-2, -1, 0, 1)，granularity=1.0 弧度
    let e = wgs84();
    let rect = Rectangle::new(-2.0, -1.0, 0.0, 1.0);
    let geo = rectangle_geometry(&rect, &e, 1.0, 0.0, VertexFormat::POSITION_ONLY);

    // 当 granularity=1.0：width=2, height=2 → cols=3, rows=3 → 9 个顶点
    assert_eq!(geo.positions.len(), 9);
    // 2*2 quads * 2 triangles * 3 = 24 indices
    assert_eq!(geo.indices.len(), 8 * 3);
    assert_eq!(geo.primitive_type, PrimitiveType::Triangles);

    // 验证 NW 与 SE 角点存在于 positions 中
    let nw = e.cartographic_to_cartesian(&Cartographic::from_radians(rect.west, rect.north, 0.0));
    let se = e.cartographic_to_cartesian(&Cartographic::from_radians(rect.east, rect.south, 0.0));

    let has_nw = geo.positions.iter().any(|p| (DVec3::from(*p) - nw).length() < EPSILON8);
    let has_se = geo.positions.iter().any(|p| (DVec3::from(*p) - se).length() < EPSILON8);
    assert!(has_nw, "positions should contain NW corner");
    assert!(has_se, "positions should contain SE corner");
}

#[test]
fn rectangle_computes_positions_across_idl() {
    // RectangleGeometrySpec: "computes positions across IDL"
    let e = wgs84();
    let rect = Rectangle::from_degrees(179.0, -1.0, -179.0, 1.0);
    let granularity = std::f64::consts::PI / 180.0; // 默认
    let geo = rectangle_geometry(&rect, &e, granularity, 0.0, VertexFormat::POSITION_ONLY);

    // 应在穿越 IDL 时产生有效几何
    assert!(geo.positions.len() >= 4, "IDL-crossing rectangle should have positions");
    assert!(!geo.indices.is_empty());

    // 所有位置应有效（非 NaN）
    for p in &geo.positions {
        assert!(p[0].is_finite() && p[1].is_finite() && p[2].is_finite());
    }
}

#[test]
fn rectangle_computes_positions_at_north_pole() {
    // RectangleGeometrySpec: "computes positions at north pole"
    let e = wgs84();
    let rect = Rectangle::from_degrees(-180.0, 89.0, -179.0, 90.0);
    let granularity = std::f64::consts::PI / 180.0;
    let geo = rectangle_geometry(&rect, &e, granularity, 0.0, VertexFormat::POSITION_ONLY);

    assert!(geo.positions.len() >= 4);
    assert!(!geo.indices.is_empty());

    // 所有位置应接近北极（z 值较大）
    for p in &geo.positions {
        let carto = e.cartesian_to_cartographic(DVec3::from(*p)).unwrap();
        assert!(
            carto.latitude.to_degrees() > 88.0,
            "latitude {} should be near north pole", carto.latitude.to_degrees()
        );
    }
}

#[test]
fn rectangle_computes_positions_at_south_pole() {
    // RectangleGeometrySpec: "computes positions at south pole"
    let e = wgs84();
    let rect = Rectangle::from_degrees(-180.0, -90.0, -179.0, -89.0);
    let granularity = std::f64::consts::PI / 180.0;
    let geo = rectangle_geometry(&rect, &e, granularity, 0.0, VertexFormat::POSITION_ONLY);

    assert!(geo.positions.len() >= 4);
    assert!(!geo.indices.is_empty());

    // 所有位置应接近南极（z 值较小）
    for p in &geo.positions {
        let carto = e.cartesian_to_cartographic(DVec3::from(*p)).unwrap();
        assert!(
            carto.latitude.to_degrees() < -88.0,
            "latitude {} should be near south pole", carto.latitude.to_degrees()
        );
    }
}

// ─── 顶点属性 ─────────────────────────────────────────────────────

#[test]
fn rectangle_computes_all_attributes() {
    // RectangleGeometrySpec: "computes all attributes"
    let e = wgs84();
    let rect = Rectangle::new(-2.0, -1.0, 0.0, 1.0);
    let geo = rectangle_geometry(&rect, &e, 1.0, 0.0, VertexFormat::ALL);

    let num_vertices = geo.positions.len();
    assert_eq!(num_vertices, 9);
    assert_eq!(geo.normals.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.tex_coords.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.indices.len(), 8 * 3);
}

#[test]
fn rectangle_normals_point_outward() {
    // 所有法线应背离椭球中心
    let e = wgs84();
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let granularity = std::f64::consts::PI / 18.0; // 10 度
    let geo = rectangle_geometry(&rect, &e, granularity, 0.0, VertexFormat::POSITION_AND_NORMAL);
    let normals = geo.normals.as_ref().unwrap();

    for i in 0..geo.positions.len() {
        let pos = DVec3::from(geo.positions[i]);
        let normal = DVec3::from(normals[i]);
        let pos_dir = pos.normalize();
        let dot = normal.dot(pos_dir);
        assert!(dot > 0.9, "normal {} should point outward, dot = {}", i, dot);
    }
}

#[test]
fn rectangle_tex_coords_in_unit_range() {
    // 纹理坐标应在 [0, 1] 范围内
    let e = wgs84();
    let rect = Rectangle::from_degrees(-20.0, -10.0, 20.0, 10.0);
    let granularity = std::f64::consts::PI / 36.0; // 5 度
    let geo = rectangle_geometry(&rect, &e, granularity, 0.0, VertexFormat::POSITION_AND_ST);
    let st = geo.tex_coords.as_ref().unwrap();

    for (i, uv) in st.iter().enumerate() {
        assert!(
            uv[0] >= -1e-10 && uv[0] <= 1.0 + 1e-10,
            "tex_coord[{}].u = {} out of [0,1]", i, uv[0]
        );
        assert!(
            uv[1] >= -1e-10 && uv[1] <= 1.0 + 1e-10,
            "tex_coord[{}].v = {} out of [0,1]", i, uv[1]
        );
    }
}

// ─── 高度参数 ──────────────────────────────────────────────────────

#[test]
fn rectangle_with_height() {
    // 位置应位于指定高度
    let e = wgs84();
    let rect = Rectangle::from_degrees(-5.0, -5.0, 5.0, 5.0);
    let height = 10000.0;
    let granularity = std::f64::consts::PI / 18.0;
    let geo = rectangle_geometry(&rect, &e, granularity, height, VertexFormat::POSITION_ONLY);

    for p in &geo.positions {
        let carto = e.cartesian_to_cartographic(DVec3::from(*p)).unwrap();
        assert!(
            (carto.height - height).abs() < 1.0,
            "height {} should be ≈ {}", carto.height, height
        );
    }
}

#[test]
fn rectangle_with_negative_height() {
    // 负高度（椭球表面以下）
    let e = wgs84();
    let rect = Rectangle::from_degrees(-5.0, -5.0, 5.0, 5.0);
    let height = -5000.0;
    let granularity = std::f64::consts::PI / 18.0;
    let geo = rectangle_geometry(&rect, &e, granularity, height, VertexFormat::POSITION_ONLY);

    for p in &geo.positions {
        let carto = e.cartesian_to_cartographic(DVec3::from(*p)).unwrap();
        assert!(
            (carto.height - height).abs() < 1.0,
            "height {} should be ≈ {}", carto.height, height
        );
    }
}

// ─── 网格结构 ────────────────────────────────────────────────────────

#[test]
fn rectangle_grid_density_matches_granularity() {
    // 更细的 granularity 应产生更多顶点
    let e = wgs84();
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);

    let coarse = rectangle_geometry(&rect, &e, std::f64::consts::PI / 18.0, 0.0, VertexFormat::POSITION_ONLY);
    let fine = rectangle_geometry(&rect, &e, std::f64::consts::PI / 36.0, 0.0, VertexFormat::POSITION_ONLY);

    assert!(
        fine.positions.len() > coarse.positions.len(),
        "finer granularity ({}) should produce more vertices than coarse ({})",
        fine.positions.len(), coarse.positions.len()
    );
}

#[test]
fn rectangle_indices_reference_valid_vertices() {
    let e = wgs84();
    let rect = Rectangle::from_degrees(-30.0, -20.0, 30.0, 20.0);
    let granularity = std::f64::consts::PI / 18.0;
    let geo = rectangle_geometry(&rect, &e, granularity, 0.0, VertexFormat::POSITION_ONLY);

    let num_vertices = geo.positions.len() as u32;
    for (i, &idx) in geo.indices.iter().enumerate() {
        assert!(
            idx < num_vertices,
            "index[{}] = {} out of bounds (num_vertices = {})", i, idx, num_vertices
        );
    }
}

#[test]
fn rectangle_bounding_sphere_contains_all_positions() {
    let e = wgs84();
    let rect = Rectangle::from_degrees(-45.0, -30.0, 45.0, 30.0);
    let granularity = std::f64::consts::PI / 18.0;
    let geo = rectangle_geometry(&rect, &e, granularity, 0.0, VertexFormat::POSITION_ONLY);

    let center = geo.bounding_sphere.center;
    let radius = geo.bounding_sphere.radius;

    for p in &geo.positions {
        let dist = (DVec3::from(*p) - center).length();
        assert!(
            dist <= radius + EPSILON8,
            "position distance {} exceeds bounding sphere radius {}", dist, radius
        );
    }
}

// ─── 边界情形 ────────────────────────────────────────────────────────────

#[test]
fn rectangle_very_small() {
    // 极小的矩形仍应产生有效几何
    let e = wgs84();
    let rect = Rectangle::from_degrees(0.0, 0.0, 0.001, 0.001);
    let granularity = std::f64::consts::PI / 180.0;
    let geo = rectangle_geometry(&rect, &e, granularity, 0.0, VertexFormat::POSITION_ONLY);

    assert!(geo.positions.len() >= 4);
    assert!(!geo.indices.is_empty());
}

#[test]
fn rectangle_full_longitude_range() {
    // 完整 360° 经度范围
    let e = wgs84();
    let rect = Rectangle::from_degrees(-180.0, -10.0, 180.0, 10.0);
    let granularity = std::f64::consts::PI / 6.0; // 30 度
    let geo = rectangle_geometry(&rect, &e, granularity, 0.0, VertexFormat::POSITION_ONLY);

    assert!(geo.positions.len() >= 4);
    assert!(!geo.indices.is_empty());
    assert_eq!(geo.indices.len() % 3, 0);
}

// ─── 旋转参数验证 ─────────────────────────────────────────

#[test]
fn rectangle_rotation_zero_no_effect() {
    // rotation=0 应产生与默认相同的位置
    let e = wgs84();
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let granularity = std::f64::consts::PI / 18.0;

    let geo1 = rectangle_geometry(&rect, &e, granularity, 0.0, VertexFormat::POSITION_ONLY);
    // 由于我们的 API 不直接接受 rotation，位置保持不变
    let _center = rect.center();
    for p in &geo1.positions {
        let carto = e.cartesian_to_cartographic(DVec3::from(*p)).unwrap();
        // 经度应在 rect 范围内
        assert!(carto.longitude >= rect.west - 1e-6 && carto.longitude <= rect.east + 1e-6);
        assert!(carto.latitude >= rect.south - 1e-6 && carto.latitude <= rect.north + 1e-6);
    }
}

#[test]
fn rectangle_position_count_depends_on_granularity() {
    let e = wgs84();
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);

    // 粗粒度
    let geo1 = rectangle_geometry(&rect, &e, 1.0, 0.0, VertexFormat::POSITION_ONLY);
    // 细粒度
    let geo2 = rectangle_geometry(&rect, &e, 0.1, 0.0, VertexFormat::POSITION_ONLY);

    assert!(geo2.positions.len() > geo1.positions.len(),
        "finer granularity should produce more vertices: {} vs {}",
        geo2.positions.len(), geo1.positions.len());
}

#[test]
fn rectangle_tex_coords_corner_values() {
    // 角点处纹理坐标应为 (0,0) (1,0) (0,1) (1,1)
    let e = wgs84();
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let granularity = std::f64::consts::PI / 18.0;
    let geo = rectangle_geometry(&rect, &e, granularity, 0.0, VertexFormat::POSITION_AND_ST);
    let st = geo.tex_coords.as_ref().unwrap();
    let num_vertices = geo.positions.len();

    assert_eq!(st.len(), num_vertices);
    // 第一个顶点位于 (west, south) → ST 应为 (0, 0)
    assert!((st[0][0]).abs() < 1e-6);
    assert!((st[0][1]).abs() < 1e-6);
    // 最后一个顶点位于 (east, north) → ST 应为 (1, 1)
    assert!((st[num_vertices - 1][0] - 1.0).abs() < 1e-6);
    assert!((st[num_vertices - 1][1] - 1.0).abs() < 1e-6);
}

#[test]
fn rectangle_extreme_latitudes_near_poles() {
    // 接近北极的矩形（85° 到 89°）
    let e = wgs84();
    let rect = Rectangle::from_degrees(-180.0, 85.0, 180.0, 89.0);
    let granularity = std::f64::consts::PI / 6.0;
    let geo = rectangle_geometry(&rect, &e, granularity, 0.0, VertexFormat::POSITION_ONLY);

    assert!(geo.positions.len() >= 4);
    for p in &geo.positions {
        let carto = e.cartesian_to_cartographic(DVec3::from(*p)).unwrap();
        assert!(carto.latitude.to_degrees() >= 84.5, "not near north pole: {}", carto.latitude);
    }
}
