//! 几何生成器规格测试 - 参考自 Core/BoxGeometrySpec、
//! Core/SphereGeometrySpec、Core/CylinderGeometrySpec、
//! Core/EllipsoidGeometrySpec、Core/FrustumGeometrySpec、
//! Core/RectangleGeometrySpec
//!
//! 测试生成几何体的顶点数、索引数、包围球、法线以及
//! 数学性质。

use cesium_geospatial::geometry::{
    box_geometry, cylinder_geometry, ellipsoid_geometry, sphere_geometry,
    rectangle_geometry, frustum_geometry, frustum_outline_geometry,
    FrustumDef, PrimitiveType,
};
use cesium_geospatial::{Ellipsoid, PerspectiveFrustum, VertexFormat};
use glam::DVec3;

const EPSILON10: f64 = 1e-10;
const EPSILON7: f64 = 1e-7;

// ─── BoxGeometry（来自 BoxGeometrySpec） ─────────────────────────────────

#[test]
fn box_position_only_creates_optimized_positions() {
    // BoxGeometrySpec: "constructor creates optimized number of positions for VertexFormat.POSITIONS_ONLY"
    let geo = box_geometry(
        DVec3::new(-1.0, -2.0, -3.0),
        DVec3::new(1.0, 2.0, 3.0),
        VertexFormat::POSITION_ONLY,
    );
    // 6 面 * 4 顶点 = 24 位置（用于平面着色的每面独立顶点）
    assert_eq!(geo.positions.len(), 24);
    // 6 面 * 2 三角形 * 3 索引 = 36
    assert_eq!(geo.indices.len(), 36);
    assert!(geo.normals.is_none());
    assert!(geo.tex_coords.is_none());
}

#[test]
fn box_computes_all_vertex_attributes() {
    // BoxGeometrySpec: "constructor computes all vertex attributes"
    let min = DVec3::new(0.0, 0.0, 0.0);
    let max = DVec3::new(1.0, 1.0, 1.0);
    let geo = box_geometry(min, max, VertexFormat::ALL);

    let num_vertices = 24; // 6 面 * 4 顶点
    let num_triangles = 12; // 6 面 * 2 三角形
    assert_eq!(geo.positions.len(), num_vertices);
    assert_eq!(geo.normals.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.tex_coords.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.indices.len(), num_triangles * 3);

    // 包围球中心应位于盒子的中心
    let center = (min + max) * 0.5;
    assert!((geo.bounding_sphere.center - center).length() < EPSILON10);
    // 半径 = 半条对角线
    let expected_radius = (max - min).length() * 0.5;
    assert!((geo.bounding_sphere.radius - expected_radius).abs() < EPSILON10);
}

#[test]
fn box_from_dimensions_concept() {
    // BoxGeometrySpec："fromDimensions" - 以原点为中心、具有给定尺寸的盒子
    let dimensions = DVec3::new(1.0, 2.0, 3.0);
    let half = dimensions * 0.5;
    let geo = box_geometry(-half, half, VertexFormat::POSITION_ONLY);

    assert_eq!(geo.positions.len(), 24);
    assert_eq!(geo.indices.len(), 36);

    // 所有位置应位于 [-half, half] 内
    for p in &geo.positions {
        assert!((p[0] - (-half.x).min(half.x)).abs() < EPSILON10 || (p[0] - half.x).abs() < EPSILON10);
    }
}

#[test]
fn box_normals_perpendicular_to_faces() {
    // 验证每个面都有一致的单位法线
    let geo = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::POSITION_AND_NORMAL,
    );
    let normals = geo.normals.as_ref().unwrap();

    // 每 4 个顶点（一个面）应具有相同的法线
    for face in 0..6 {
        let base = face * 4;
        let n0 = DVec3::from(normals[base]);
        for v in 1..4 {
            let nv = DVec3::from(normals[base + v]);
            assert!((n0 - nv).length() < EPSILON10, "face {} normals should be uniform", face);
        }
        // 法线应为单位长度
        assert!((n0.length() - 1.0).abs() < EPSILON10);
    }
}

#[test]
fn box_degenerate_min_equals_max() {
    // BoxGeometrySpec: "undefined is returned if min and max are equal"
    // Rust 实现生成零尺寸盒子
    let p = DVec3::new(250000.0, 250000.0, 250000.0);
    let geo = box_geometry(p, p, VertexFormat::POSITION_ONLY);
    // 所有位置坠缩到同一点
    for pos in &geo.positions {
        assert!((pos[0] - 250000.0).abs() < EPSILON10);
        assert!((pos[1] - 250000.0).abs() < EPSILON10);
        assert!((pos[2] - 250000.0).abs() < EPSILON10);
    }
    // 包围球半径应为 0
    assert!(geo.bounding_sphere.radius < EPSILON10);
}

// ─── SphereGeometry（来自 SphereGeometrySpec） ───────────────────────────

#[test]
fn sphere_computes_positions() {
    // SphereGeometrySpec："computes positions"，stackPartitions=3、slicePartitions=3
    let geo = sphere_geometry(1.0, 3, 3, VertexFormat::POSITION_ONLY);

    // Rust：(stacks+1) * (slices+1) = 4 * 4 = 16 顶点
    let num_vertices = (3 + 1) * (3 + 1);
    assert_eq!(geo.positions.len(), num_vertices);
    // stacks * slices * 6 = 3 * 3 * 6 = 54 索引
    let num_indices = 3 * 3 * 6;
    assert_eq!(geo.indices.len(), num_indices);
    assert!((geo.bounding_sphere.radius - 1.0).abs() < EPSILON10);
}

#[test]
fn sphere_computes_all_vertex_attributes() {
    // SphereGeometrySpec: "compute all vertex attributes"
    let geo = sphere_geometry(1.0, 3, 3, VertexFormat::ALL);

    let num_vertices = (3 + 1) * (3 + 1);
    assert_eq!(geo.positions.len(), num_vertices);
    assert_eq!(geo.normals.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.tex_coords.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.indices.len(), 3 * 3 * 6);
}

#[test]
fn sphere_positions_on_unit_sphere() {
    // SphereGeometrySpec: "computes attributes for a unit sphere"
    let geo = sphere_geometry(1.0, 6, 8, VertexFormat::POSITION_AND_NORMAL);
    let normals = geo.normals.as_ref().unwrap();

    for i in 0..geo.positions.len() {
        let pos = DVec3::from(geo.positions[i]);
        let normal = DVec3::from(normals[i]);

        // 位置模长应 ≈ 1.0
        assert!(
            (pos.length() - 1.0).abs() < EPSILON10,
            "position {} magnitude {} != 1.0", i, pos.length()
        );

        // 法线应等于归一化后的位置（对于单位球）
        if pos.length() > EPSILON10 {
            let expected_normal = pos.normalize();
            assert!(
                (normal - expected_normal).length() < EPSILON7,
                "normal {} doesn't match normalized position", i
            );
        }
    }
}

#[test]
fn sphere_radius_scales_positions() {
    // 位置到中心的距离应 = radius
    let radius = 5.0;
    let geo = sphere_geometry(radius, 4, 4, VertexFormat::POSITION_ONLY);

    for p in &geo.positions {
        let pos = DVec3::from(*p);
        assert!(
            (pos.length() - radius).abs() < EPSILON10,
            "position magnitude {} != radius {}", pos.length(), radius
        );
    }
    assert!((geo.bounding_sphere.radius - radius).abs() < EPSILON10);
}

// ─── CylinderGeometry（来自 CylinderGeometrySpec） ───────────────────────

#[test]
fn cylinder_computes_positions() {
    // CylinderGeometrySpec："computes positions"，slices=3
    let geo = cylinder_geometry(1.0, 1.0, 1.0, 3, VertexFormat::POSITION_ONLY);

    // Rust：(slices+1) * 2 = 8 顶点（仅侧面，无端盖）
    let num_vertices = (3 + 1) * 2;
    assert_eq!(geo.positions.len(), num_vertices);
    // slices * 6 = 18 索引（每切片 2 三角形）
    let num_indices = 3 * 6;
    assert_eq!(geo.indices.len(), num_indices);
    assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
}

#[test]
fn cylinder_computes_all_vertex_attributes() {
    // CylinderGeometrySpec: "compute all vertex attributes"
    let geo = cylinder_geometry(1.0, 1.0, 1.0, 3, VertexFormat::ALL);

    let num_vertices = (3 + 1) * 2;
    assert_eq!(geo.positions.len(), num_vertices);
    assert_eq!(geo.normals.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.tex_coords.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.indices.len(), 3 * 6);
}

#[test]
fn cylinder_top_radius_zero_cone() {
    // CylinderGeometrySpec: "computes positions with topRadius equals 0"
    let geo = cylinder_geometry(1.0, 0.0, 1.0, 3, VertexFormat::POSITION_ONLY);

    let num_vertices = (3 + 1) * 2;
    assert_eq!(geo.positions.len(), num_vertices);
    assert_eq!(geo.indices.len(), 3 * 6);

    // 顶部顶点应位于原点（radius=0）
    for i in (1..geo.positions.len()).step_by(2) {
        let p = DVec3::from(geo.positions[i]);
        assert!(p.x.abs() < EPSILON10 && p.y.abs() < EPSILON10,
            "top vertex {} should be at center, got ({}, {})", i, p.x, p.y);
    }
}

#[test]
fn cylinder_bottom_radius_zero_inverted_cone() {
    // CylinderGeometrySpec: "computes positions with bottomRadius equals 0"
    let geo = cylinder_geometry(1.0, 1.0, 0.0, 3, VertexFormat::POSITION_ONLY);

    let num_vertices = (3 + 1) * 2;
    assert_eq!(geo.positions.len(), num_vertices);
    assert_eq!(geo.indices.len(), 3 * 6);

    // 底部顶点应位于原点（radius=0）
    for i in (0..geo.positions.len()).step_by(2) {
        let p = DVec3::from(geo.positions[i]);
        assert!(p.x.abs() < EPSILON10 && p.y.abs() < EPSILON10,
            "bottom vertex {} should be at center, got ({}, {})", i, p.x, p.y);
    }
}

#[test]
fn cylinder_bounding_sphere() {
    // 包围球应包含该圆柱
    let length = 2.0;
    let radius = 1.0;
    let geo = cylinder_geometry(length, radius, radius, 8, VertexFormat::POSITION_ONLY);

    let half_length = length * 0.5;
    let expected_radius = (radius * radius + half_length * half_length).sqrt();
    assert!((geo.bounding_sphere.radius - expected_radius).abs() < EPSILON10);
    assert!(geo.bounding_sphere.center.length() < EPSILON10); // 以原点为中心
}

// ─── EllipsoidGeometry（来自 EllipsoidGeometrySpec） ─────────────────────

#[test]
fn ellipsoid_computes_positions() {
    // EllipsoidGeometrySpec："computes positions"，slicePartitions=3、stackPartitions=3
    let radii = DVec3::new(1.0, 1.0, 1.0);
    let geo = ellipsoid_geometry(radii, 3, 3, VertexFormat::POSITION_ONLY);

    // Rust：(stacks+1) * (slices+1) = 4 * 4 = 16
    let num_vertices = (3 + 1) * (3 + 1);
    assert_eq!(geo.positions.len(), num_vertices);
    // stacks * slices * 6 = 54
    assert_eq!(geo.indices.len(), 3 * 3 * 6);
    assert!((geo.bounding_sphere.radius - 1.0).abs() < EPSILON10);
}

#[test]
fn ellipsoid_computes_all_vertex_attributes() {
    // EllipsoidGeometrySpec: "compute all vertex attributes"
    let radii = DVec3::new(1.0, 1.0, 1.0);
    let geo = ellipsoid_geometry(radii, 3, 3, VertexFormat::ALL);

    let num_vertices = (3 + 1) * (3 + 1);
    assert_eq!(geo.positions.len(), num_vertices);
    assert_eq!(geo.normals.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.tex_coords.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.indices.len(), 3 * 3 * 6);
}

#[test]
fn ellipsoid_unit_sphere_properties() {
    // EllipsoidGeometrySpec: "computes attributes for a unit sphere"
    let radii = DVec3::new(1.0, 1.0, 1.0);
    let geo = ellipsoid_geometry(radii, 6, 8, VertexFormat::POSITION_AND_NORMAL);
    let normals = geo.normals.as_ref().unwrap();

    for i in 0..geo.positions.len() {
        let pos = DVec3::from(geo.positions[i]);
        let normal = DVec3::from(normals[i]);

        // 对于单位球，位置模长 ≈ 1.0
        assert!(
            (pos.length() - 1.0).abs() < EPSILON10,
            "position {} magnitude {} != 1.0", i, pos.length()
        );

        // 法线应等于归一化后的位置
        if pos.length() > EPSILON10 {
            let expected_normal = pos.normalize();
            assert!(
                (normal - expected_normal).length() < EPSILON7,
                "normal {} doesn't match normalized position", i
            );
        }
    }
}

#[test]
fn ellipsoid_non_uniform_radii() {
    // 非均匀半径应相应地缩放位置
    let radii = DVec3::new(1.0, 2.0, 3.0);
    let geo = ellipsoid_geometry(radii, 4, 4, VertexFormat::POSITION_ONLY);

    // 包围球半径应为最大半径
    assert!((geo.bounding_sphere.radius - 3.0).abs() < EPSILON10);

    // 所有位置应满足 (x/rx)² + (y/ry)² + (z/rz)² ≈ 1
    for p in &geo.positions {
        let normalized = DVec3::new(p[0] / radii.x, p[1] / radii.y, p[2] / radii.z);
        assert!(
            (normalized.length() - 1.0).abs() < EPSILON7,
            "position ({}, {}, {}) not on ellipsoid surface", p[0], p[1], p[2]
        );
    }
}

// ─── FrustumGeometry（来自 FrustumGeometrySpec） ─────────────────────────

#[test]
fn frustum_computes_all_vertex_attributes() {
    // FrustumGeometrySpec: "constructor computes all vertex attributes"
    let frustum = FrustumDef::Perspective(PerspectiveFrustum {
        fov: (30.0_f64).to_radians(),
        aspect_ratio: 1920.0 / 1080.0,
        near: 1.0,
        far: 3.0,
        x_offset: 0.0,
        y_offset: 0.0,
    });
    let geo = frustum_geometry(&frustum, DVec3::ZERO, glam::DQuat::IDENTITY, VertexFormat::ALL);

    let num_vertices = 24; // 6 平面 * 4 顶点
    let num_triangles = 12; // 6 平面 * 2 三角形
    assert_eq!(geo.positions.len(), num_vertices);
    assert_eq!(geo.normals.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.tex_coords.as_ref().unwrap().len(), num_vertices);
    assert_eq!(geo.indices.len(), num_triangles * 3);
}

#[test]
fn frustum_bounding_sphere() {
    // FrustumGeometrySpec：包围球中心位于视锥轴的中点
    let frustum = FrustumDef::Perspective(PerspectiveFrustum {
        fov: (30.0_f64).to_radians(),
        aspect_ratio: 1920.0 / 1080.0,
        near: 1.0,
        far: 3.0,
        x_offset: 0.0,
        y_offset: 0.0,
    });
    let geo = frustum_geometry(&frustum, DVec3::ZERO, glam::DQuat::IDENTITY, VertexFormat::POSITION_ONLY);

    // 包围球应沿 -Z 轴居中（视锥朝向 -Z 方向）
    // 中心应位于 near 与 far 平面之间
    assert!(geo.bounding_sphere.radius > 1.0);
    assert!(geo.bounding_sphere.radius < 3.0);
}

#[test]
fn frustum_outline_produces_lines() {
    let frustum = FrustumDef::Perspective(PerspectiveFrustum {
        fov: (45.0_f64).to_radians(),
        aspect_ratio: 1.0,
        near: 0.5,
        far: 10.0,
        x_offset: 0.0,
        y_offset: 0.0,
    });
    let geo = frustum_outline_geometry(&frustum, DVec3::ZERO, glam::DQuat::IDENTITY);

    assert!(geo.positions.len() >= 8, "frustum outline needs at least 8 corners");
    assert!(!geo.indices.is_empty());
    assert_eq!(geo.indices.len() % 2, 0, "outline indices should be line pairs");
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

// ─── RectangleGeometry ─────────────────────────────────────────────────────

#[test]
fn rectangle_produces_grid_positions() {
    use cesium_geospatial::Rectangle;
    let ellipsoid = Ellipsoid::WGS84;
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let granularity = std::f64::consts::PI / 18.0; // 10 度

    let geo = rectangle_geometry(&rect, &ellipsoid, granularity, 0.0, VertexFormat::POSITION_ONLY);

    // 应生成位置网格
    assert!(geo.positions.len() >= 4, "rectangle should have at least 4 positions");
    assert!(!geo.indices.is_empty());
    assert_eq!(geo.indices.len() % 3, 0, "indices should be triangles");
    assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
}

#[test]
fn rectangle_normals_point_outward() {
    use cesium_geospatial::Rectangle;
    let ellipsoid = Ellipsoid::WGS84;
    let rect = Rectangle::from_degrees(-5.0, -5.0, 5.0, 5.0);
    let granularity = std::f64::consts::PI / 180.0; // 1 度

    let geo = rectangle_geometry(&rect, &ellipsoid, granularity, 0.0, VertexFormat::POSITION_AND_NORMAL);
    let normals = geo.normals.as_ref().unwrap();

    // 每个法线应朝外（与位置方向的点积 > 0）
    for i in 0..geo.positions.len() {
        let pos = DVec3::from(geo.positions[i]);
        let normal = DVec3::from(normals[i]);
        let pos_dir = pos.normalize();
        let dot = normal.dot(pos_dir);
        assert!(dot > 0.9, "normal {} should point outward, dot = {}", i, dot);
    }
}

#[test]
fn rectangle_bounding_sphere_contains_all_positions() {
    use cesium_geospatial::Rectangle;
    let ellipsoid = Ellipsoid::WGS84;
    let rect = Rectangle::from_degrees(-20.0, -10.0, 20.0, 10.0);
    let granularity = std::f64::consts::PI / 36.0; // 5 度

    let geo = rectangle_geometry(&rect, &ellipsoid, granularity, 0.0, VertexFormat::POSITION_ONLY);

    let center = geo.bounding_sphere.center;
    let radius = geo.bounding_sphere.radius;

    for p in &geo.positions {
        let pos = DVec3::from(*p);
        let dist = (pos - center).length();
        assert!(
            dist <= radius + EPSILON7,
            "position distance {} exceeds bounding sphere radius {}", dist, radius
        );
    }
}

// ─── 横切不变量 ──────────────────────────────────────────────

#[test]
fn all_generators_produce_valid_indices() {
    // 所有索引应引用有效顶点
    let geometries = vec![
        box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::ALL),
        sphere_geometry(1.0, 4, 4, VertexFormat::ALL),
        cylinder_geometry(2.0, 1.0, 1.0, 8, VertexFormat::ALL),
        ellipsoid_geometry(DVec3::new(1.0, 1.0, 1.0), 4, 4, VertexFormat::ALL),
    ];

    for (name, geo) in geometries.iter().enumerate() {
        let num_vertices = geo.positions.len() as u32;
        for (i, &idx) in geo.indices.iter().enumerate() {
            assert!(
                idx < num_vertices,
                "geometry {} index[{}] = {} out of bounds (num_vertices = {})",
                name, i, idx, num_vertices
            );
        }
    }
}

#[test]
fn all_generators_normals_are_unit_length() {
    let geometries = vec![
        box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::POSITION_AND_NORMAL),
        sphere_geometry(2.0, 4, 4, VertexFormat::POSITION_AND_NORMAL),
        cylinder_geometry(2.0, 1.0, 1.5, 8, VertexFormat::POSITION_AND_NORMAL),
        ellipsoid_geometry(DVec3::new(1.0, 2.0, 3.0), 4, 4, VertexFormat::POSITION_AND_NORMAL),
    ];

    for (name, geo) in geometries.iter().enumerate() {
        if let Some(normals) = &geo.normals {
            for (i, n) in normals.iter().enumerate() {
                let len = DVec3::from(*n).length();
                assert!(
                    (len - 1.0).abs() < EPSILON7,
                    "geometry {} normal[{}] length {} != 1.0", name, i, len
                );
            }
        }
    }
}
