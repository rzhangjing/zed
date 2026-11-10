//! 几何不变量规格测试 - 针对所有几何生成器的横切测试。
//!
//! 验证必须对所有几何类型成立的数学不变量：
//! - 有效索引（在范围内）
//! - 包围球包含性
//! - 单位长度法线
//! - 纹理坐标范围 [0,1]
//! - 正确的图元类型
//! - 非空输出
//! - 各属性间顶点数一致

use cesium_geospatial::geometry::{
    box_geometry, box_outline_geometry, circle_geometry, circle_outline_geometry,
    cylinder_geometry, cylinder_outline_geometry, ellipsoid_geometry,
    ellipsoid_outline_geometry, plane_geometry, plane_outline_geometry,
    rectangle_geometry, rectangle_outline_geometry, sphere_geometry,
    PrimitiveType, VertexFormat,
};
use cesium_geospatial::{Ellipsoid, Rectangle};
use glam::DVec3;

const EPSILON7: f64 = 1e-7;
const EPSILON10: f64 = 1e-10;

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

// ─── 有效索引不变量 ────────────────────────────────────────────────

#[test]
fn all_geometries_produce_valid_indices() {
    let e = wgs84();
    let center = e.cartographic_to_cartesian(&cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0));
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);

    let geometries: Vec<(&str, cesium_geospatial::geometry::GeometryData)> = vec![
        ("box", box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::ALL)),
        ("sphere", sphere_geometry(1.0, 8, 8, VertexFormat::ALL)),
        ("cylinder", cylinder_geometry(2.0, 1.0, 1.0, 8, VertexFormat::ALL)),
        ("ellipsoid", ellipsoid_geometry(DVec3::new(1.0, 2.0, 3.0), 8, 8, VertexFormat::ALL)),
        ("circle", circle_geometry(center, 100000.0, &e, 16, VertexFormat::ALL)),
        ("rectangle", rectangle_geometry(&rect, &e, 0.1, 0.0, VertexFormat::ALL)),
        ("plane", plane_geometry(VertexFormat::ALL)),
    ];

    for (name, geo) in &geometries {
        let n = geo.positions.len() as u32;
        for (i, &idx) in geo.indices.iter().enumerate() {
            assert!(
                idx < n,
                "{}: index[{}] = {} out of bounds (n={})",
                name, i, idx, n
            );
        }
    }
}

// ─── 包围球包含性不变量 ──────────────────────────────────

#[test]
fn all_geometries_bounding_sphere_contains_positions() {
    let e = wgs84();
    let center = e.cartographic_to_cartesian(&cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0));
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);

    let geometries: Vec<(&str, cesium_geospatial::geometry::GeometryData)> = vec![
        ("box", box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::POSITION_ONLY)),
        ("sphere", sphere_geometry(1.0, 8, 8, VertexFormat::POSITION_ONLY)),
        ("cylinder", cylinder_geometry(2.0, 1.0, 1.0, 8, VertexFormat::POSITION_ONLY)),
        ("ellipsoid", ellipsoid_geometry(DVec3::new(1.0, 2.0, 3.0), 8, 8, VertexFormat::POSITION_ONLY)),
        ("circle", circle_geometry(center, 100000.0, &e, 16, VertexFormat::POSITION_ONLY)),
        ("rectangle", rectangle_geometry(&rect, &e, 0.1, 0.0, VertexFormat::POSITION_ONLY)),
        ("plane", plane_geometry(VertexFormat::POSITION_ONLY)),
    ];

    for (name, geo) in &geometries {
        let bs_center = geo.bounding_sphere.center;
        let bs_radius = geo.bounding_sphere.radius;

        for (i, p) in geo.positions.iter().enumerate() {
            let pos = DVec3::from(*p);
            let dist = (pos - bs_center).length();
            assert!(
                dist <= bs_radius + 1e-3,
                "{}: position[{}] distance {} exceeds bounding sphere radius {}",
                name, i, dist, bs_radius
            );
        }
    }
}

// ─── 单位长度法线不变量 ─────────────────────────────────────────

#[test]
fn all_geometries_with_normals_have_unit_length() {
    let e = wgs84();
    let center = e.cartographic_to_cartesian(&cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0));
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);

    let geometries: Vec<(&str, cesium_geospatial::geometry::GeometryData)> = vec![
        ("box", box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::POSITION_AND_NORMAL)),
        ("sphere", sphere_geometry(1.0, 8, 8, VertexFormat::POSITION_AND_NORMAL)),
        ("cylinder", cylinder_geometry(2.0, 1.0, 1.0, 8, VertexFormat::POSITION_AND_NORMAL)),
        ("ellipsoid", ellipsoid_geometry(DVec3::new(1.0, 2.0, 3.0), 8, 8, VertexFormat::POSITION_AND_NORMAL)),
        ("circle", circle_geometry(center, 100000.0, &e, 16, VertexFormat::POSITION_AND_NORMAL)),
        ("rectangle", rectangle_geometry(&rect, &e, 0.1, 0.0, VertexFormat::POSITION_AND_NORMAL)),
        ("plane", plane_geometry(VertexFormat::POSITION_AND_NORMAL)),
    ];

    for (name, geo) in &geometries {
        if let Some(normals) = &geo.normals {
            assert_eq!(
                normals.len(),
                geo.positions.len(),
                "{}: normals count {} != positions count {}",
                name, normals.len(), geo.positions.len()
            );
            for (i, n) in normals.iter().enumerate() {
                let len = DVec3::from(*n).length();
                assert!(
                    (len - 1.0).abs() < EPSILON7,
                    "{}: normal[{}] length {} != 1.0",
                    name, i, len
                );
            }
        }
    }
}

// ─── 纹理坐标范围不变量 ────────────────────────────────────

#[test]
fn all_geometries_tex_coords_in_unit_square() {
    let e = wgs84();
    let center = e.cartographic_to_cartesian(&cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0));
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);

    let geometries: Vec<(&str, cesium_geospatial::geometry::GeometryData)> = vec![
        ("box", box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::ALL)),
        ("sphere", sphere_geometry(1.0, 8, 8, VertexFormat::ALL)),
        ("cylinder", cylinder_geometry(2.0, 1.0, 1.0, 8, VertexFormat::ALL)),
        ("ellipsoid", ellipsoid_geometry(DVec3::new(1.0, 2.0, 3.0), 8, 8, VertexFormat::ALL)),
        ("circle", circle_geometry(center, 100000.0, &e, 16, VertexFormat::ALL)),
        ("rectangle", rectangle_geometry(&rect, &e, 0.1, 0.0, VertexFormat::ALL)),
        ("plane", plane_geometry(VertexFormat::ALL)),
    ];

    for (name, geo) in &geometries {
        if let Some(tex_coords) = &geo.tex_coords {
            assert_eq!(
                tex_coords.len(),
                geo.positions.len(),
                "{}: tex_coords count {} != positions count {}",
                name, tex_coords.len(), geo.positions.len()
            );
            for (i, st) in tex_coords.iter().enumerate() {
                assert!(
                    st[0] >= -EPSILON10 && st[0] <= 1.0 + EPSILON10,
                    "{}: tex_coords[{}].s = {} out of [0,1]",
                    name, i, st[0]
                );
                assert!(
                    st[1] >= -EPSILON10 && st[1] <= 1.0 + EPSILON10,
                    "{}: tex_coords[{}].t = {} out of [0,1]",
                    name, i, st[1]
                );
            }
        }
    }
}

// ─── 顶点数一致性不变量 ─────────────────────────────────────

#[test]
fn all_geometries_consistent_vertex_counts() {
    let e = wgs84();
    let center = e.cartographic_to_cartesian(&cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0));
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);

    let geometries: Vec<(&str, cesium_geospatial::geometry::GeometryData)> = vec![
        ("box", box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::ALL)),
        ("sphere", sphere_geometry(1.0, 8, 8, VertexFormat::ALL)),
        ("cylinder", cylinder_geometry(2.0, 1.0, 1.0, 8, VertexFormat::ALL)),
        ("ellipsoid", ellipsoid_geometry(DVec3::new(1.0, 2.0, 3.0), 8, 8, VertexFormat::ALL)),
        ("circle", circle_geometry(center, 100000.0, &e, 16, VertexFormat::ALL)),
        ("rectangle", rectangle_geometry(&rect, &e, 0.1, 0.0, VertexFormat::ALL)),
        ("plane", plane_geometry(VertexFormat::ALL)),
    ];

    for (name, geo) in &geometries {
        let n = geo.positions.len();
        assert!(n > 0, "{}: should have at least one position", name);

        if let Some(normals) = &geo.normals {
            assert_eq!(normals.len(), n, "{}: normals count mismatch", name);
        }
        if let Some(tex_coords) = &geo.tex_coords {
            assert_eq!(tex_coords.len(), n, "{}: tex_coords count mismatch", name);
        }
        if let Some(tangents) = &geo.tangents {
            assert_eq!(tangents.len(), n, "{}: tangents count mismatch", name);
        }
        if let Some(bitangents) = &geo.bitangents {
            assert_eq!(bitangents.len(), n, "{}: bitangents count mismatch", name);
        }
    }
}

// ─── 正确图元类型不变量 ──────────────────────────────────────

#[test]
fn all_triangle_geometries_use_triangles_primitive() {
    let e = wgs84();
    let center = e.cartographic_to_cartesian(&cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0));
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);

    let geometries: Vec<(&str, cesium_geospatial::geometry::GeometryData)> = vec![
        ("box", box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::POSITION_ONLY)),
        ("sphere", sphere_geometry(1.0, 8, 8, VertexFormat::POSITION_ONLY)),
        ("cylinder", cylinder_geometry(2.0, 1.0, 1.0, 8, VertexFormat::POSITION_ONLY)),
        ("ellipsoid", ellipsoid_geometry(DVec3::new(1.0, 2.0, 3.0), 8, 8, VertexFormat::POSITION_ONLY)),
        ("circle", circle_geometry(center, 100000.0, &e, 16, VertexFormat::POSITION_ONLY)),
        ("rectangle", rectangle_geometry(&rect, &e, 0.1, 0.0, VertexFormat::POSITION_ONLY)),
        ("plane", plane_geometry(VertexFormat::POSITION_ONLY)),
    ];

    for (name, geo) in &geometries {
        assert_eq!(
            geo.primitive_type,
            PrimitiveType::Triangles,
            "{}: should use Triangles primitive",
            name
        );
        assert_eq!(
            geo.indices.len() % 3,
            0,
            "{}: indices count {} not divisible by 3",
            name, geo.indices.len()
        );
    }
}

#[test]
fn all_outline_geometries_use_lines_primitive() {
    let e = wgs84();
    let center = e.cartographic_to_cartesian(&cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0));
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);

    let geometries: Vec<(&str, cesium_geospatial::geometry::GeometryData)> = vec![
        ("box_outline", box_outline_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0))),
        ("ellipsoid_outline", ellipsoid_outline_geometry(DVec3::new(1.0, 1.0, 1.0), 8, 8)),
        ("circle_outline", circle_outline_geometry(center, 100000.0, &e, 0.1)),
        ("rectangle_outline", rectangle_outline_geometry(&rect, &e, 0.1)),
        ("cylinder_outline", cylinder_outline_geometry(2.0, 1.0, 1.0, 8)),
        ("plane_outline", plane_outline_geometry()),
    ];

    for (name, geo) in &geometries {
        assert_eq!(
            geo.primitive_type,
            PrimitiveType::Lines,
            "{}: should use Lines primitive",
            name
        );
        assert_eq!(
            geo.indices.len() % 2,
            0,
            "{}: indices count {} not divisible by 2",
            name, geo.indices.len()
        );
    }
}

// ─── 非空输出不变量 ─────────────────────────────────────────────

#[test]
fn all_geometries_produce_non_empty_output() {
    let e = wgs84();
    let center = e.cartographic_to_cartesian(&cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0));
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);

    let geometries: Vec<(&str, cesium_geospatial::geometry::GeometryData)> = vec![
        ("box", box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::POSITION_ONLY)),
        ("sphere", sphere_geometry(1.0, 8, 8, VertexFormat::POSITION_ONLY)),
        ("cylinder", cylinder_geometry(2.0, 1.0, 1.0, 8, VertexFormat::POSITION_ONLY)),
        ("ellipsoid", ellipsoid_geometry(DVec3::new(1.0, 2.0, 3.0), 8, 8, VertexFormat::POSITION_ONLY)),
        ("circle", circle_geometry(center, 100000.0, &e, 16, VertexFormat::POSITION_ONLY)),
        ("rectangle", rectangle_geometry(&rect, &e, 0.1, 0.0, VertexFormat::POSITION_ONLY)),
        ("plane", plane_geometry(VertexFormat::POSITION_ONLY)),
        ("box_outline", box_outline_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0))),
        ("ellipsoid_outline", ellipsoid_outline_geometry(DVec3::new(1.0, 1.0, 1.0), 8, 8)),
        ("circle_outline", circle_outline_geometry(center, 100000.0, &e, 0.1)),
        ("rectangle_outline", rectangle_outline_geometry(&rect, &e, 0.1)),
        ("cylinder_outline", cylinder_outline_geometry(2.0, 1.0, 1.0, 8)),
        ("plane_outline", plane_outline_geometry()),
    ];

    for (name, geo) in &geometries {
        assert!(!geo.positions.is_empty(), "{}: positions should not be empty", name);
        assert!(!geo.indices.is_empty(), "{}: indices should not be empty", name);
        assert!(
            geo.bounding_sphere.radius >= 0.0,
            "{}: bounding sphere radius should be non-negative",
            name
        );
    }
}

// ─── 特定几何不变量 ──────────────────────────────────────────

#[test]
fn sphere_positions_on_surface() {
    let geo = sphere_geometry(1.0, 16, 16, VertexFormat::POSITION_ONLY);
    for (i, p) in geo.positions.iter().enumerate() {
        let pos = DVec3::from(*p);
        let magnitude = pos.length();
        assert!(
            (magnitude - 1.0).abs() < EPSILON10,
            "sphere position[{}] magnitude {} != 1.0",
            i, magnitude
        );
    }
}

#[test]
fn ellipsoid_positions_on_surface() {
    let radii = DVec3::new(1.0, 2.0, 3.0);
    let geo = ellipsoid_geometry(radii, 16, 16, VertexFormat::POSITION_ONLY);
    for (i, p) in geo.positions.iter().enumerate() {
        let x = p[0] / radii.x;
        let y = p[1] / radii.y;
        let z = p[2] / radii.z;
        let val = x * x + y * y + z * z;
        assert!(
            (val - 1.0).abs() < EPSILON7,
            "ellipsoid position[{}] ({}, {}, {}) not on surface, val={}",
            i, p[0], p[1], p[2], val
        );
    }
}

#[test]
fn cylinder_positions_at_correct_z() {
    let length = 4.0;
    let geo = cylinder_geometry(length, 1.0, 1.0, 16, VertexFormat::POSITION_ONLY);
    let half = length / 2.0;

    for (i, p) in geo.positions.iter().enumerate() {
        let z = p[2];
        assert!(
            (z - half).abs() < EPSILON10 || (z + half).abs() < EPSILON10,
            "cylinder position[{}] z={} should be ±{}",
            i, z, half
        );
    }
}

#[test]
fn box_positions_at_corners() {
    let min = DVec3::new(-1.0, -2.0, -3.0);
    let max = DVec3::new(1.0, 2.0, 3.0);
    let geo = box_geometry(min, max, VertexFormat::POSITION_ONLY);

    for (i, p) in geo.positions.iter().enumerate() {
        let x_ok = (p[0] - min.x).abs() < EPSILON10 || (p[0] - max.x).abs() < EPSILON10;
        let y_ok = (p[1] - min.y).abs() < EPSILON10 || (p[1] - max.y).abs() < EPSILON10;
        let z_ok = (p[2] - min.z).abs() < EPSILON10 || (p[2] - max.z).abs() < EPSILON10;
        assert!(
            x_ok && y_ok && z_ok,
            "box position[{}] ({}, {}, {}) not at corner",
            i, p[0], p[1], p[2]
        );
    }
}

#[test]
fn plane_positions_in_xy_plane() {
    let geo = plane_geometry(VertexFormat::POSITION_ONLY);
    for (i, p) in geo.positions.iter().enumerate() {
        assert!(
            p[2].abs() < EPSILON10,
            "plane position[{}] z={} should be 0",
            i, p[2]
        );
        assert!(
            p[0].abs() <= 0.5 + EPSILON10 && p[1].abs() <= 0.5 + EPSILON10,
            "plane position[{}] ({}, {}) out of [-0.5, 0.5]",
            i, p[0], p[1]
        );
    }
}
