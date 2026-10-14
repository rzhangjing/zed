//! 参考自 CesiumJS EllipsoidGeometrySpec/SphereGeometrySpec/BoxGeometrySpec/
//! CylinderGeometrySpec/RectangleOutlineGeometrySpec + GeometryPipeline 扩展。
//!
//! 针对几何生成器的数学属性验证测试。

use cesium_geospatial::bounding::BoundingSphere;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::geometry::{
    box_geometry, box_outline_geometry, combine_geometries,
    compute_normal, compute_tangent_and_bitangent, create_line_segments_for_vectors,
    cylinder_geometry, cylinder_outline_geometry, ellipsoid_geometry, ellipsoid_outline_geometry,
    plane_geometry, plane_outline_geometry, rectangle_outline_geometry,
    reorder_for_pre_vertex_cache, sphere_geometry, to_wireframe, GeometryData, PrimitiveType,
    VertexFormat,
};
use cesium_geospatial::rectangle::Rectangle;
use glam::DVec3;

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

// ===========================================================================
// EllipsoidGeometry - 单位球数学属性
// ===========================================================================

#[test]
fn ellipsoid_unit_sphere_positions_have_unit_magnitude() {
    // CesiumJS: "computes attributes for a unit sphere"
    // 对于单位球，所有位置的模长应为 1.0
    let geo = ellipsoid_geometry(DVec3::splat(1.0), 3, 3, VertexFormat::POSITION_AND_NORMAL);

    for (i, p) in geo.positions.iter().enumerate() {
        let mag = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        assert!(
            (mag - 1.0).abs() < 1e-10,
            "position[{}] magnitude should be 1.0, got {}",
            i,
            mag
        );
    }
}

#[test]
fn ellipsoid_unit_sphere_normals_equal_normalized_positions() {
    // CesiumJS：单位球下 normal === normalize(position)
    let geo = ellipsoid_geometry(DVec3::splat(1.0), 3, 3, VertexFormat::POSITION_AND_NORMAL);
    let normals = geo.normals.as_ref().expect("normals should be present");

    for (i, (p, n)) in geo.positions.iter().zip(normals.iter()).enumerate() {
        let pos = DVec3::new(p[0], p[1], p[2]);
        let nrm = DVec3::new(n[0], n[1], n[2]);
        let expected_normal = pos.normalize_or(DVec3::Z);
        let diff = (nrm - expected_normal).length();
        assert!(
            diff < 1e-7,
            "normal[{}] should equal normalized position, diff={}",
            i,
            diff
        );
    }
}

#[test]
fn ellipsoid_scaled_radii_positions_on_ellipsoid() {
    // 对于半径 (2, 3, 4)，位置应满足 x²/4 + y²/9 + z²/16 = 1
    let radii = DVec3::new(2.0, 3.0, 4.0);
    let geo = ellipsoid_geometry(radii, 8, 8, VertexFormat::POSITION_ONLY);

    for (i, p) in geo.positions.iter().enumerate() {
        let val = (p[0] / 2.0).powi(2) + (p[1] / 3.0).powi(2) + (p[2] / 4.0).powi(2);
        assert!(
            (val - 1.0).abs() < 1e-10,
            "position[{}] should be on ellipsoid surface, got {}",
            i,
            val
        );
    }
}

#[test]
fn ellipsoid_bounding_sphere_radius_equals_max_radii() {
    let radii = DVec3::new(2.0, 3.0, 4.0);
    let geo = ellipsoid_geometry(radii, 8, 8, VertexFormat::POSITION_ONLY);
    assert!(
        (geo.bounding_sphere.radius - 4.0).abs() < 1e-10,
        "bounding sphere radius should be max(radii)=4, got {}",
        geo.bounding_sphere.radius
    );
}

#[test]
fn ellipsoid_vertex_count_formula() {
    // CesiumJS：(stacks+1) * (slices+1) 个顶点，stacks*slices*2 个三角形
    let stacks = 6u32;
    let slices = 8u32;
    let geo = ellipsoid_geometry(DVec3::splat(1.0), stacks, slices, VertexFormat::POSITION_ONLY);

    let expected_verts = (stacks + 1) * (slices + 1);
    let expected_indices = stacks * slices * 6;
    assert_eq!(geo.positions.len(), expected_verts as usize);
    assert_eq!(geo.indices.len(), expected_indices as usize);
}

// ===========================================================================
// SphereGeometry - 半径缩放
// ===========================================================================

#[test]
fn sphere_positions_have_correct_radius() {
    let radius = 5.0;
    let geo = sphere_geometry(radius, 8, 8, VertexFormat::POSITION_ONLY);

    for (i, p) in geo.positions.iter().enumerate() {
        let mag = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        assert!(
            (mag - radius).abs() < 1e-10,
            "position[{}] magnitude should be {}, got {}",
            i,
            radius,
            mag
        );
    }
}

#[test]
fn sphere_bounding_sphere_radius_matches() {
    let radius = 7.5;
    let geo = sphere_geometry(radius, 4, 4, VertexFormat::POSITION_ONLY);
    assert!(
        (geo.bounding_sphere.radius - radius).abs() < 1e-10,
        "bounding sphere radius should be {}, got {}",
        radius,
        geo.bounding_sphere.radius
    );
}

#[test]
fn sphere_tex_coords_in_unit_range() {
    let geo = sphere_geometry(1.0, 8, 8, VertexFormat::POSITION_AND_ST);
    let st = geo.tex_coords.as_ref().expect("tex_coords should be present");

    for (i, uv) in st.iter().enumerate() {
        assert!(
            uv[0] >= -1e-10 && uv[0] <= 1.0 + 1e-10,
            "st[{}].u should be in [0,1], got {}",
            i,
            uv[0]
        );
        assert!(
            uv[1] >= -1e-10 && uv[1] <= 1.0 + 1e-10,
            "st[{}].v should be in [0,1], got {}",
            i,
            uv[1]
        );
    }
}

// ===========================================================================
// BoxGeometry - 数学属性
// ===========================================================================

#[test]
fn box_bounding_sphere_center_is_midpoint() {
    // CesiumJS：对称盒子的 boundingSphere.center === Cartesian3.ZERO
    let min = DVec3::new(-2.0, -3.0, -4.0);
    let max = DVec3::new(2.0, 3.0, 4.0);
    let geo = box_geometry(min, max, VertexFormat::POSITION_ONLY);

    let expected_center = (min + max) * 0.5;
    let diff = (geo.bounding_sphere.center - expected_center).length();
    assert!(diff < 1e-10, "bounding sphere center should be midpoint");
}

#[test]
fn box_bounding_sphere_radius_is_half_diagonal() {
    // CesiumJS: boundingSphere.radius === magnitude(maximum) * 0.5
    let min = DVec3::new(0.0, 0.0, 0.0);
    let max = DVec3::new(1.0, 1.0, 1.0);
    let geo = box_geometry(min, max, VertexFormat::POSITION_ONLY);

    let size = max - min;
    let expected_radius = size.length() * 0.5;
    assert!(
        (geo.bounding_sphere.radius - expected_radius).abs() < 1e-10,
        "bounding sphere radius should be half diagonal, got {}",
        geo.bounding_sphere.radius
    );
}

#[test]
fn box_normals_perpendicular_to_faces() {
    // 每个面法线应与坐标轴对齐（一个分量为 ±1，其余为 0）
    let geo = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::POSITION_AND_NORMAL,
    );
    let normals = geo.normals.as_ref().expect("normals should be present");

    for (i, n) in normals.iter().enumerate() {
        let nrm = DVec3::new(n[0], n[1], n[2]);
        // 应为单位长度
        assert!((nrm.length() - 1.0).abs() < 1e-10, "normal[{}] not unit", i);
        // 应与坐标轴对齐：最大分量应为 1.0
        let max_comp = nrm.x.abs().max(nrm.y.abs()).max(nrm.z.abs());
        assert!(
            (max_comp - 1.0).abs() < 1e-10,
            "normal[{}] should be axis-aligned",
            i
        );
    }
}

#[test]
fn box_positions_at_correct_corners() {
    let min = DVec3::new(-1.0, -2.0, -3.0);
    let max = DVec3::new(1.0, 2.0, 3.0);
    let geo = box_geometry(min, max, VertexFormat::POSITION_ONLY);

    // 每条轴上所有位置的坐标都应取 min 或 max
    for (i, p) in geo.positions.iter().enumerate() {
        let x_ok = (p[0] - min.x).abs() < 1e-10 || (p[0] - max.x).abs() < 1e-10;
        let y_ok = (p[1] - min.y).abs() < 1e-10 || (p[1] - max.y).abs() < 1e-10;
        let z_ok = (p[2] - min.z).abs() < 1e-10 || (p[2] - max.z).abs() < 1e-10;
        assert!(
            x_ok && y_ok && z_ok,
            "position[{}] should be at a corner of the box",
            i
        );
    }
}

#[test]
fn box_24_vertices_36_indices() {
    // CesiumJS：24 个顶点（6 面 × 4），36 个索引（12 三角形 × 3）
    let geo = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::ALL,
    );
    assert_eq!(geo.positions.len(), 24);
    assert_eq!(geo.indices.len(), 36);
}

// ===========================================================================
// CylinderGeometry - 圆锥与包围球
// ===========================================================================

#[test]
fn cylinder_cone_top_radius_zero_positions() {
    // CesiumJS: "computes positions with topRadius equals 0"
    let geo = cylinder_geometry(2.0, 0.0, 1.0, 3, VertexFormat::POSITION_ONLY);

    // 顶部顶点应全部位于 (0, 0, half_length)
    let half_length = 1.0;
    let top_verts: Vec<_> = geo
        .positions
        .iter()
        .filter(|p| (p[2] - half_length).abs() < 1e-10)
        .collect();
    assert!(!top_verts.is_empty(), "should have top vertices");
    for p in &top_verts {
        assert!(
            p[0].abs() < 1e-10 && p[1].abs() < 1e-10,
            "top vertex should be at origin in XY for cone"
        );
    }
}

#[test]
fn cylinder_bounding_sphere_radius() {
    // bounding sphere radius = sqrt(max_radius² + half_length²)
    let length = 4.0;
    let top_r = 2.0;
    let bottom_r = 3.0;
    let geo = cylinder_geometry(length, top_r, bottom_r, 8, VertexFormat::POSITION_ONLY);

    let max_r = top_r.max(bottom_r);
    let half_l = length * 0.5;
    let expected = (max_r * max_r + half_l * half_l).sqrt();
    assert!(
        (geo.bounding_sphere.radius - expected).abs() < 1e-10,
        "bounding sphere radius should be {}, got {}",
        expected,
        geo.bounding_sphere.radius
    );
}

#[test]
fn cylinder_positions_at_correct_z() {
    let length = 6.0;
    let geo = cylinder_geometry(length, 1.0, 1.0, 8, VertexFormat::POSITION_ONLY);
    let half = length / 2.0;

    for p in &geo.positions {
        assert!(
            (p[2] - half).abs() < 1e-10 || (p[2] + half).abs() < 1e-10,
            "z should be ±half_length, got {}",
            p[2]
        );
    }
}

#[test]
fn cylinder_side_vertices_on_radius() {
    let top_r = 2.0;
    let bottom_r = 3.0;
    let geo = cylinder_geometry(4.0, top_r, bottom_r, 8, VertexFormat::POSITION_ONLY);
    let half = 2.0;

    for p in &geo.positions {
        let xy_dist = (p[0] * p[0] + p[1] * p[1]).sqrt();
        if (p[2] - half).abs() < 1e-10 {
            assert!(
                (xy_dist - top_r).abs() < 1e-10,
                "top vertex xy distance should be top_radius"
            );
        } else if (p[2] + half).abs() < 1e-10 {
            assert!(
                (xy_dist - bottom_r).abs() < 1e-10,
                "bottom vertex xy distance should be bottom_radius"
            );
        }
    }
}

// ===========================================================================
// RectangleOutlineGeometry - 扩展
// ===========================================================================

#[test]
fn rectangle_outline_positions_on_ellipsoid() {
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let ell = wgs84();
    let geo = rectangle_outline_geometry(&rect, &ell, std::f64::consts::PI / 180.0);

    assert!(!geo.positions.is_empty());
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);

    // 所有位置应位于椭球表面上
    for (i, p) in geo.positions.iter().enumerate() {
        let pos = DVec3::new(p[0], p[1], p[2]);
        let surface = ell.scale_to_geodetic_surface(pos).unwrap_or(pos);
        let dist = (pos - surface).length();
        assert!(dist < 1.0, "position[{}] should be on surface, dist={}", i, dist);
    }
}

#[test]
fn rectangle_outline_indices_form_line_pairs() {
    let rect = Rectangle::from_degrees(-5.0, -5.0, 5.0, 5.0);
    let geo = rectangle_outline_geometry(&rect, &wgs84(), std::f64::consts::PI / 180.0);

    assert_eq!(geo.indices.len() % 2, 0, "indices must form line pairs");
    // 所有索引应引用有效位置
    let max_idx = geo.positions.len() as u32;
    for &idx in &geo.indices {
        assert!(idx < max_idx, "index {} out of range", idx);
    }
}

#[test]
fn rectangle_outline_granularity_affects_density() {
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let ell = wgs84();

    let coarse = rectangle_outline_geometry(&rect, &ell, std::f64::consts::PI / 9.0); // 20°
    let fine = rectangle_outline_geometry(&rect, &ell, std::f64::consts::PI / 180.0); // 1°

    assert!(
        fine.positions.len() > coarse.positions.len(),
        "finer granularity ({}) should produce more positions than coarser ({})",
        fine.positions.len(),
        coarse.positions.len()
    );
}

#[test]
fn rectangle_outline_bounding_sphere_contains_all() {
    let rect = Rectangle::from_degrees(-30.0, -20.0, 30.0, 20.0);
    let geo = rectangle_outline_geometry(&rect, &wgs84(), std::f64::consts::PI / 180.0);

    let bs = &geo.bounding_sphere;
    for (i, p) in geo.positions.iter().enumerate() {
        let pos = DVec3::new(p[0], p[1], p[2]);
        let dist = (pos - bs.center).length();
        assert!(
            dist <= bs.radius + 1.0,
            "position[{}] should be within bounding sphere",
            i
        );
    }
}

// ===========================================================================
// 轮廓几何 - 结构属性
// ===========================================================================

#[test]
fn box_outline_8_vertices_12_edges() {
    let geo = box_outline_geometry(DVec3::new(-1.0, -2.0, -3.0), DVec3::new(1.0, 2.0, 3.0));
    assert_eq!(geo.positions.len(), 8, "box outline should have 8 corners");
    assert_eq!(geo.indices.len(), 24, "box outline should have 12 edges × 2");
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn ellipsoid_outline_three_great_circles() {
    let geo = ellipsoid_outline_geometry(DVec3::splat(1.0), 8, 8);
    // 3 个圆：(slices+1) + (stacks+1) + (stacks+1) 个顶点
    let expected = (8 + 1) + (8 + 1) + (8 + 1);
    assert_eq!(geo.positions.len(), expected);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    // 所有位置位于单位球上
    for p in &geo.positions {
        let mag = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        assert!((mag - 1.0).abs() < 1e-10, "outline position should be on unit sphere");
    }
}

#[test]
fn cylinder_outline_two_circles_plus_verticals() {
    let slices = 8u32;
    let geo = cylinder_outline_geometry(2.0, 1.0, 1.0, slices);
    // 2 个圆：(slices+1)*2 个顶点 + 竖线：slices.min(16)*2
    let circle_verts = (slices + 1) * 2;
    let num_verticals = slices.min(16);
    let vertical_verts = num_verticals * 2;
    let expected = circle_verts + vertical_verts;
    assert_eq!(geo.positions.len(), expected as usize);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn plane_outline_4_vertices_4_edges() {
    let geo = plane_outline_geometry();
    assert_eq!(geo.positions.len(), 4);
    assert_eq!(geo.indices.len(), 8); // 4 条边 × 2
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    // 全部位于 z=0 平面
    for p in &geo.positions {
        assert!(p[2].abs() < 1e-10, "plane outline should be in z=0");
    }
}

// ===========================================================================
// GeometryPipeline - 扩展属性测试
// ===========================================================================

#[test]
fn combine_geometries_drops_mixed_attributes() {
    // 若一个几何体有法线而另一个没有，结果会丢弃法线
    let geo_with = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::POSITION_AND_NORMAL,
    );
    let geo_without = box_geometry(
        DVec3::new(2.0, 2.0, 2.0),
        DVec3::new(3.0, 3.0, 3.0),
        VertexFormat::POSITION_ONLY,
    );

    let combined = combine_geometries(&[geo_with, geo_without]);
    assert!(
        combined.normals.is_none(),
        "normals should be dropped when not all geometries have them"
    );
    assert_eq!(combined.positions.len(), 48); // 24 + 24
}

#[test]
fn combine_geometries_preserves_shared_attributes() {
    let geo1 = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::POSITION_AND_NORMAL,
    );
    let geo2 = box_geometry(
        DVec3::new(2.0, 2.0, 2.0),
        DVec3::new(3.0, 3.0, 3.0),
        VertexFormat::POSITION_AND_NORMAL,
    );

    let combined = combine_geometries(&[geo1, geo2]);
    assert!(combined.normals.is_some(), "normals should be preserved");
    assert_eq!(combined.normals.as_ref().unwrap().len(), 48);
}

#[test]
fn combine_geometries_offsets_indices() {
    let geo1 = plane_geometry(VertexFormat::POSITION_ONLY); // 4 顶点、6 索引
    let geo2 = plane_geometry(VertexFormat::POSITION_ONLY); // 4 顶点、6 索引

    let combined = combine_geometries(&[geo1, geo2]);
    assert_eq!(combined.positions.len(), 8);
    assert_eq!(combined.indices.len(), 12);
    // 第二个几何体的索引应偏移 4
    assert!(combined.indices[6] >= 4, "second geo indices should be offset");
}

#[test]
fn combine_geometries_bounding_sphere_encompasses_all() {
    let geo1 = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::POSITION_ONLY,
    );
    let geo2 = box_geometry(
        DVec3::new(10.0, 10.0, 10.0),
        DVec3::new(12.0, 12.0, 12.0),
        VertexFormat::POSITION_ONLY,
    );

    let combined = combine_geometries(&[geo1, geo2]);
    // 包围球应包含所有位置
    for p in &combined.positions {
        let pos = DVec3::new(p[0], p[1], p[2]);
        let dist = (pos - combined.bounding_sphere.center).length();
        assert!(
            dist <= combined.bounding_sphere.radius + 1e-6,
            "all positions should be within combined bounding sphere"
        );
    }
}

#[test]
fn reorder_removes_unused_vertices_and_remaps() {
    // 创建一个含未使用顶点的几何体
    let mut geo = GeometryData {
        positions: vec![
            [0.0, 0.0, 0.0], // 0 - 已使用
            [1.0, 0.0, 0.0], // 1 - 已使用
            [0.0, 1.0, 0.0], // 2 - 已使用
            [5.0, 5.0, 5.0], // 3 - 未使用
            [1.0, 1.0, 0.0], // 4 - 已使用
        ],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2, 1, 4, 2],
        bounding_sphere: BoundingSphere::default(),
        primitive_type: PrimitiveType::Triangles,
    };

    reorder_for_pre_vertex_cache(&mut geo);
    assert_eq!(geo.positions.len(), 4, "unused vertex should be removed");
    // 所有索引应有效
    for &idx in &geo.indices {
        assert!((idx as usize) < geo.positions.len());
    }
}

#[test]
fn create_line_segments_doubles_positions() {
    let positions = vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let normals = vec![
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
    ];

    let geo = create_line_segments_for_vectors(&positions, &normals, 2.0);
    assert_eq!(geo.positions.len(), 6, "should double positions (start+end per vector)");
    assert_eq!(geo.indices.len(), 6, "should have 3 line segments × 2 indices");
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);

    // 端点应按 normal * length 偏移
    let end0 = DVec3::new(geo.positions[1][0], geo.positions[1][1], geo.positions[1][2]);
    let expected_end0 = DVec3::new(0.0, 0.0, 2.0); // 原点 + (0,0,1)*2
    assert!((end0 - expected_end0).length() < 1e-10);
}

#[test]
fn compute_normal_produces_unit_normals() {
    let mut geo = sphere_geometry(1.0, 4, 4, VertexFormat::POSITION_ONLY);
    assert!(geo.normals.is_none());
    compute_normal(&mut geo);
    let normals = geo.normals.as_ref().expect("normals should be computed");

    for (i, n) in normals.iter().enumerate() {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!(
            (len - 1.0).abs() < 1e-6,
            "normal[{}] should be unit length, got {}",
            i,
            len
        );
    }
}

#[test]
fn compute_tangent_bitangent_orthogonal_to_normal() {
    let mut geo = sphere_geometry(1.0, 4, 4, VertexFormat::POSITION_AND_ST);
    compute_normal(&mut geo);
    compute_tangent_and_bitangent(&mut geo);

    let normals = geo.normals.as_ref().unwrap();
    let tangents = geo.tangents.as_ref().expect("tangents should be computed");
    let bitangents = geo.bitangents.as_ref().expect("bitangents should be computed");

    for i in 0..normals.len() {
        let n = DVec3::from(normals[i]);
        let t = DVec3::from(tangents[i]);
        let b = DVec3::from(bitangents[i]);

        // 切线 ⊥ 法线
        let nt_dot = n.dot(t).abs();
        assert!(nt_dot < 1e-6, "tangent[{}] should be ⊥ normal, dot={}", i, nt_dot);

        // 副切线 ⊥ 法线
        let nb_dot = n.dot(b).abs();
        assert!(nb_dot < 1e-6, "bitangent[{}] should be ⊥ normal, dot={}", i, nb_dot);

        // 副切线 = cross(normal, tangent)
        let expected_b = n.cross(t);
        let diff = (b - expected_b).length();
        assert!(diff < 1e-6, "bitangent[{}] should = cross(n,t), diff={}", i, diff);
    }
}

#[test]
fn wireframe_preserves_vertex_count() {
    let mut geo = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::POSITION_ONLY,
    );
    let orig_positions = geo.positions.len();
    let tri_count = geo.indices.len() / 3;

    to_wireframe(&mut geo);
    assert_eq!(geo.positions.len(), orig_positions, "wireframe shouldn't change positions");
    assert_eq!(geo.indices.len(), tri_count * 6, "each triangle → 3 edges × 2 indices");
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}
