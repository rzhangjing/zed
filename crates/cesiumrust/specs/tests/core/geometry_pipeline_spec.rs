//! GeometryPipeline 规格测试 - 参考自 Core/GeometryPipelineSpec
//! A 类测试：toWireframe、computeNormal、computeTangentAndBitangent、
//! projectTo2D、encodeAttribute、transformToWorldCoordinates、compressVertices、
//! reorderForPreVertexCache、fitToUnsignedShortIndices、splitLongitude、
//! createLineSegmentsForVectors

use cesium_geospatial::geometry::{
    box_geometry, combine_geometries, compress_vertices, compute_normal,
    compute_tangent_and_bitangent, create_line_segments_for_vectors, encode_attribute,
    encode_f64_to_f32_pair, fit_to_unsigned_short_indices, project_to_2d,
    reorder_for_pre_vertex_cache, split_longitude, to_wireframe,
    transform_to_world_coordinates, GeometryData, PrimitiveType, VertexFormat,
};
use cesium_geospatial::bounding::BoundingSphere;
use cesium_geospatial::Ellipsoid;
use glam::{DVec3, DMat4};

// ─── toWireframe ─────────────────────────────────────────────────────────────

#[test]
fn wireframe_converts_triangles() {
    let mut geo = GeometryData {
        positions: vec![[0.0; 3]; 6],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2, 3, 4, 5],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };
    to_wireframe(&mut geo);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    assert_eq!(geo.indices.len(), 12);
    assert_eq!(geo.indices[0], 0);
    assert_eq!(geo.indices[1], 1);
    assert_eq!(geo.indices[2], 1);
    assert_eq!(geo.indices[3], 2);
    assert_eq!(geo.indices[4], 2);
    assert_eq!(geo.indices[5], 0);
    assert_eq!(geo.indices[6], 3);
    assert_eq!(geo.indices[7], 4);
    assert_eq!(geo.indices[8], 4);
    assert_eq!(geo.indices[9], 5);
    assert_eq!(geo.indices[10], 5);
    assert_eq!(geo.indices[11], 3);
}

// ─── computeNormal ───────────────────────────────────────────────────────────

#[test]
fn compute_normal_one_triangle() {
    // XY 平面内的三角形：(0,0,0), (1,0,0), (0,1,0)
    // 法线应为 (0,0,1)
    let mut geo = GeometryData {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };
    compute_normal(&mut geo);
    let normals = geo.normals.unwrap();
    for n in &normals {
        assert!((n[0]).abs() < 1e-10);
        assert!((n[1]).abs() < 1e-10);
        assert!((n[2] - 1.0).abs() < 1e-10);
    }
}

#[test]
fn compute_normal_two_triangles() {
    // XY 平面内共享一条边的两个三角形
    let mut geo = GeometryData {
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2, 1, 3, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };
    compute_normal(&mut geo);
    let normals = geo.normals.unwrap();
    assert_eq!(normals.len(), 4);
    // 所有法线都应为 (0,0,1)
    for n in &normals {
        assert!((n[2] - 1.0).abs() < 1e-6);
    }
}

#[test]
fn compute_normal_degenerate_triangle() {
    // 退化三角形（全部为同一点）→ 法线默认为 (0,0,1)
    let mut geo = GeometryData {
        positions: vec![[1.0, 2.0, 3.0], [1.0, 2.0, 3.0], [1.0, 2.0, 3.0]],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };
    compute_normal(&mut geo);
    let normals = geo.normals.unwrap();
    // 退化 → normalize_or(DVec3::Z) → (0,0,1)
    for n in &normals {
        assert!((n[2] - 1.0).abs() < 1e-10);
    }
}

#[test]
fn compute_normal_box_geometry() {
    let mut geo = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::POSITION_ONLY,
    );
    assert!(geo.normals.is_none());
    compute_normal(&mut geo);
    let normals = geo.normals.as_ref().unwrap();
    assert_eq!(normals.len(), geo.positions.len());
    // 所有法线都应为单位长度
    for n in normals {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6);
    }
}

// ─── computeTangentAndBitangent ──────────────────────────────────────────────

#[test]
fn compute_tangent_bitangent_one_triangle() {
    // XY 平面内带 UV 的三角形
    let mut geo = GeometryData {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: Some(vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]]),
        tex_coords: Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };
    compute_tangent_and_bitangent(&mut geo);
    let tangents = geo.tangents.as_ref().unwrap();
    let bitangents = geo.bitangents.as_ref().unwrap();
    assert_eq!(tangents.len(), 3);
    assert_eq!(bitangents.len(), 3);
    // 切线应沿 X (1,0,0)，副切线沿 Y (0,1,0)
    for t in tangents {
        assert!((t[0] - 1.0).abs() < 1e-6);
        assert!((t[1]).abs() < 1e-6);
    }
    for b in bitangents {
        assert!((b[0]).abs() < 1e-6);
        assert!((b[1] - 1.0).abs() < 1e-6);
    }
}

#[test]
fn compute_tangent_bitangent_box() {
    let mut geo = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::ALL,
    );
    assert!(geo.tangents.is_none());
    compute_tangent_and_bitangent(&mut geo);
    assert!(geo.tangents.is_some());
    assert!(geo.bitangents.is_some());
    let tangents = geo.tangents.as_ref().unwrap();
    assert_eq!(tangents.len(), geo.positions.len());
    // 所有切线都应为单位长度
    for t in tangents {
        let len = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6);
    }
}

// ─── projectTo2D ─────────────────────────────────────────────────────────────

#[test]
fn project_to_2d_basic() {
    let ellipsoid = Ellipsoid::WGS84;
    let p1 = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(10.0, 20.0, 0.0),
    );
    let p2 = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(30.0, 40.0, 0.0),
    );

    let positions = vec![[p1.x, p1.y, p1.z], [p2.x, p2.y, p2.z]];
    let (pos3d, pos2d) = project_to_2d(&positions, &ellipsoid);

    // 3D 位置应保持不变
    assert!((pos3d[0][0] - p1.x).abs() < 1e-10);
    assert!((pos3d[0][1] - p1.y).abs() < 1e-10);
    assert!((pos3d[0][2] - p1.z).abs() < 1e-10);

    // 2D 位置应被投影（以米为单位的经度/纬度）
    // GeographicProjection：x = lon * a, y = lat * a, z = height
    let a = ellipsoid.maximum_radius();
    let expected_x1 = 10.0_f64.to_radians() * a;
    let expected_y1 = 20.0_f64.to_radians() * a;
    assert!((pos2d[0][0] - expected_x1).abs() < 1.0); // 在 1 米以内
    assert!((pos2d[0][1] - expected_y1).abs() < 1.0);
}

// ─── encodeAttribute ─────────────────────────────────────────────────────────

#[test]
fn encode_f64_pair_roundtrip() {
    let value = 1234567.890123;
    let (high, low) = encode_f64_to_f32_pair(value);
    let reconstructed = high as f64 + low as f64;
    assert!((reconstructed - value).abs() < 1e-3);
}

#[test]
fn encode_attribute_positions() {
    let positions = vec![
        [100000.0, 200000.0, 300000.0],
        [400000.0, 500000.0, 600000.0],
    ];
    let (high, low) = encode_attribute(&positions);
    assert_eq!(high.len(), 2);
    assert_eq!(low.len(), 2);

    // high + low 应近似原值
    for i in 0..2 {
        for j in 0..3 {
            let reconstructed = high[i][j] as f64 + low[i][j] as f64;
            assert!((reconstructed - positions[i][j]).abs() < 0.01);
        }
    }
}

// ─── transformToWorldCoordinates ─────────────────────────────────────────────

#[test]
fn transform_to_world_coordinates_translation() {
    let mut geo = GeometryData {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: Some(vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]]),
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };

    let model_matrix = DMat4::from_translation(DVec3::new(10.0, 20.0, 30.0));
    transform_to_world_coordinates(&mut geo, &model_matrix);

    // 位置应被平移
    assert!((geo.positions[0][0] - 10.0).abs() < 1e-10);
    assert!((geo.positions[0][1] - 20.0).abs() < 1e-10);
    assert!((geo.positions[0][2] - 30.0).abs() < 1e-10);
    assert!((geo.positions[1][0] - 11.0).abs() < 1e-10);

    // 法线应保持不变（平移不影响法线）
    let normals = geo.normals.as_ref().unwrap();
    assert!((normals[0][2] - 1.0).abs() < 1e-6);
}

#[test]
fn transform_to_world_coordinates_identity() {
    let mut geo = GeometryData {
        positions: vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
        normals: Some(vec![[0.0, 0.0, 1.0], [1.0, 0.0, 0.0]]),
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 0],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };

    let original_positions = geo.positions.clone();
    let model_matrix = DMat4::IDENTITY;
    transform_to_world_coordinates(&mut geo, &model_matrix);

    // 应保持不变
    for i in 0..2 {
        for j in 0..3 {
            assert!((geo.positions[i][j] - original_positions[i][j]).abs() < 1e-10);
        }
    }
}

#[test]
fn transform_to_world_coordinates_scale() {
    let mut geo = GeometryData {
        positions: vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        normals: Some(vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };

    let model_matrix = DMat4::from_scale(DVec3::splat(2.0));
    transform_to_world_coordinates(&mut geo, &model_matrix);

    // 位置应被缩放
    assert!((geo.positions[0][0] - 2.0).abs() < 1e-10);
    assert!((geo.positions[1][1] - 2.0).abs() < 1e-10);
    assert!((geo.positions[2][2] - 2.0).abs() < 1e-10);

    // 法线应保持单位长度（均匀缩放）
    let normals = geo.normals.as_ref().unwrap();
    for n in normals {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6);
    }
}

// ─── compressVertices ────────────────────────────────────────────────────────

#[test]
fn compress_vertices_without_normals() {
    let geo = GeometryData {
        positions: vec![[0.0; 3]; 3],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };
    assert!(compress_vertices(&geo).is_none());
}

#[test]
fn compress_vertices_with_normals() {
    let geo = GeometryData {
        positions: vec![[0.0; 3]; 3],
        normals: Some(vec![[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };
    let compressed = compress_vertices(&geo).unwrap();
    // 3 顶点 * 1 u32（仅法线）= 3 个 u32
    assert_eq!(compressed.len(), 3);
}

#[test]
fn compress_vertices_with_normals_and_st() {
    let geo = GeometryData {
        positions: vec![[0.0; 3]; 3],
        normals: Some(vec![[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
        tex_coords: Some(vec![[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]]),
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };
    let compressed = compress_vertices(&geo).unwrap();
    // 3 顶点 * 2 u32（法线 + st）= 6 个 u32
    assert_eq!(compressed.len(), 6);
}

// ─── createLineSegmentsForVectors ────────────────────────────────────────────

#[test]
fn create_line_segments_for_normals() {
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
    let lines = create_line_segments_for_vectors(&positions, &normals, 1.0);

    assert_eq!(lines.primitive_type, PrimitiveType::Lines);
    assert_eq!(lines.positions.len(), 6); // 3 顶点 * 2（起点 + 终点）
    assert_eq!(lines.indices.len(), 6);

    // 第一条线：(0,0,0) → (0,0,1)
    assert!((lines.positions[0][0]).abs() < 1e-10);
    assert!((lines.positions[0][1]).abs() < 1e-10);
    assert!((lines.positions[0][2]).abs() < 1e-10);
    assert!((lines.positions[1][0]).abs() < 1e-10);
    assert!((lines.positions[1][1]).abs() < 1e-10);
    assert!((lines.positions[1][2] - 1.0).abs() < 1e-10);

    // 包围球半径应为原值 + 长度
    assert!(lines.bounding_sphere.radius > 1.0);
}

// ─── reorderForPreVertexCache ────────────────────────────────────────────────

#[test]
fn reorder_for_pre_vertex_cache_basic() {
    let mut geo = GeometryData {
        positions: vec![
            [0.0, 0.0, 0.0], // 0
            [1.0, 0.0, 0.0], // 1
            [0.0, 1.0, 0.0], // 2
            [1.0, 1.0, 0.0], // 3
            [0.5, 0.5, 0.0], // 4（未使用）
            [2.0, 0.0, 0.0], // 5
        ],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![5, 3, 2, 0, 1, 3],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };

    reorder_for_pre_vertex_cache(&mut geo);

    // 顶点 4（未使用）应被移除
    assert_eq!(geo.positions.len(), 5);
    // 第一个索引应为 0（重映射后）
    assert_eq!(geo.indices[0], 0);
}

#[test]
fn reorder_for_pre_vertex_cache_removes_unused() {
    let mut geo = GeometryData {
        positions: vec![
            [0.0; 3], // 0 - 已使用
            [1.0; 3], // 1 - 未使用
            [2.0; 3], // 2 - 已使用
            [3.0; 3], // 3 - 未使用
            [4.0; 3], // 4 - 已使用
        ],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 2, 4],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };

    reorder_for_pre_vertex_cache(&mut geo);

    // 应仅剩 3 个顶点
    assert_eq!(geo.positions.len(), 3);
    assert_eq!(geo.indices, vec![0, 1, 2]);
}

// ─── fitToUnsignedShortIndices ───────────────────────────────────────────────

#[test]
fn fit_to_unsigned_short_no_change() {
    let geo = GeometryData {
        positions: vec![[0.0; 3]; 3],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };

    let result = fit_to_unsigned_short_indices(&geo);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].positions.len(), 3);
    assert_eq!(result[0].indices, vec![0, 1, 2]);
}

#[test]
fn fit_to_unsigned_short_splits_large_geometry() {
    // 创建含 > 65536 顶点的几何体
    let num_vertices = 65537;
    let positions: Vec<[f64; 3]> = (0..num_vertices)
        .map(|i| [i as f64, 0.0, 0.0])
        .collect();

    // 创建引用所有顶点的三角形
    let mut indices: Vec<u32> = Vec::new();
    for i in 0..(num_vertices - 2) {
        indices.push(i as u32);
        indices.push((i + 1) as u32);
        indices.push((i + 2) as u32);
    }

    let geo = GeometryData {
        positions,
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 100000.0),
        primitive_type: PrimitiveType::Triangles,
    };

    let result = fit_to_unsigned_short_indices(&geo);
    assert!(result.len() >= 2, "Should split into at least 2 geometries");

    // 每个子几何体应含 <= 65536 顶点
    for sub_geo in &result {
        assert!(sub_geo.positions.len() <= 65536);
    }
}

// ─── splitLongitude ──────────────────────────────────────────────────────────

#[test]
fn split_longitude_does_nothing_for_non_crossing() {
    let ellipsoid = Ellipsoid::WGS84;
    // 完全位于东半球的几何体
    let p0 = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(10.0, 0.0, 0.0),
    );
    let p1 = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(20.0, 0.0, 0.0),
    );
    let p2 = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(15.0, 10.0, 0.0),
    );

    let geo = GeometryData {
        positions: vec![[p0.x, p0.y, p0.z], [p1.x, p1.y, p1.z], [p2.x, p2.y, p2.z]],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, ellipsoid.maximum_radius()),
        primitive_type: PrimitiveType::Triangles,
    };

    let result = split_longitude(&geo, &ellipsoid);
    assert_eq!(result.len(), 1); // 不应拆分
}

#[test]
fn split_longitude_splits_crossing_geometry() {
    let ellipsoid = Ellipsoid::WGS84;
    // 创建在 IDL 两侧都有三角形的几何体
    // 东侧：170°E-175°E，西侧：170°W-175°W（= -170° 至 -175°）
    let e0 = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(170.0, 0.0, 0.0),
    );
    let e1 = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(175.0, 0.0, 0.0),
    );
    let e2 = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(172.0, 5.0, 0.0),
    );
    let w0 = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(-170.0, 0.0, 0.0),
    );
    let w1 = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(-175.0, 0.0, 0.0),
    );
    let w2 = ellipsoid.cartographic_to_cartesian(
        &cesium_geospatial::Cartographic::from_degrees(-172.0, 5.0, 0.0),
    );

    let geo = GeometryData {
        positions: vec![
            [e0.x, e0.y, e0.z], // 0: 东
            [e1.x, e1.y, e1.z], // 1: 东
            [e2.x, e2.y, e2.z], // 2: 东
            [w0.x, w0.y, w0.z], // 3: 西
            [w1.x, w1.y, w1.z], // 4: 西
            [w2.x, w2.y, w2.z], // 5: 西
        ],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2, 3, 4, 5], // 2 个三角形：一个东，一个西
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, ellipsoid.maximum_radius()),
        primitive_type: PrimitiveType::Triangles,
    };

    let result = split_longitude(&geo, &ellipsoid);
    // 应拆分为 2 个几何体：东三角形和西三角形
    assert!(result.len() >= 2, "Should split into east and west parts, got {}", result.len());
}

// ─── 几何生成器（详细） ─────────────────────────────────────────────

#[test]
fn box_geometry_detailed() {
    let geo = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::ALL,
    );
    assert_eq!(geo.positions.len(), 24); // 6 面 * 4 顶点
    assert_eq!(geo.indices.len(), 36); // 6 面 * 2 三角形 * 3
    assert!(geo.normals.is_some());
    assert!(geo.tex_coords.is_some());
    assert_eq!(geo.primitive_type, PrimitiveType::Triangles);

    // 所有法线都应为单位长度
    let normals = geo.normals.as_ref().unwrap();
    for n in normals {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6);
    }

    // 包围球应包含该盒子
    assert!(geo.bounding_sphere.radius >= 1.0);
}

#[test]
fn ellipsoid_geometry_detailed() {
    let radii = DVec3::new(1.0, 2.0, 3.0);
    let geo = cesium_geospatial::geometry::ellipsoid_geometry(radii, 8, 16, VertexFormat::ALL);
    assert_eq!(geo.positions.len(), 9 * 17); //（stacks+1）*（slices+1）
    assert_eq!(geo.indices.len(), 8 * 16 * 6);
    assert!(geo.normals.is_some());
    assert!(geo.tex_coords.is_some());

    // 包围球半径应为最大半径
    assert!((geo.bounding_sphere.radius - 3.0).abs() < 1e-10);
}

#[test]
fn sphere_geometry_bounding_sphere() {
    let geo = cesium_geospatial::geometry::sphere_geometry(5.0, 8, 16, VertexFormat::POSITION_ONLY);
    assert!((geo.bounding_sphere.radius - 5.0).abs() < 1e-10);
    assert!(geo.normals.is_none());
}

#[test]
fn cylinder_geometry_detailed() {
    let geo = cesium_geospatial::geometry::cylinder_geometry(2.0, 1.0, 3.0, 16, VertexFormat::ALL);
    assert!(!geo.positions.is_empty());
    assert!(geo.normals.is_some());
    assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
    // 索引数应为 3 的倍数（三角形）
    assert_eq!(geo.indices.len() % 3, 0);
}

// ─── combineInstances / combineGeometries ───────────────────────────────────

fn points_geometry(positions: Vec<[f64; 3]>) -> GeometryData {
    GeometryData {
        positions,
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: Vec::new(),
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    }
}

#[test]
fn combine_instances_combines_one_geometry() {
    let geo = points_geometry(vec![[0.0, 0.0, 0.0]]);
    let combined = combine_geometries(&[geo.clone()]);
    assert_eq!(combined.positions, geo.positions);
    assert_eq!(combined.primitive_type, PrimitiveType::Triangles);
    assert!(combined.indices.is_empty());
}

#[test]
fn combine_instances_combines_several_geometries_without_indices() {
    let a = points_geometry(vec![[0.0, 0.0, 0.0]]);
    let b = points_geometry(vec![[1.0, 1.0, 1.0]]);
    let combined = combine_geometries(&[a, b]);
    assert_eq!(
        combined.positions,
        vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]]
    );
    assert!(combined.indices.is_empty());
    assert_eq!(combined.primitive_type, PrimitiveType::Triangles);
}

#[test]
fn combine_instances_combines_several_geometries_with_indices() {
    let mut a = GeometryData {
        positions: vec![[0.0; 3], [1.0; 3], [2.0; 3]],
        normals: Some(vec![[0.0; 3], [1.0; 3], [2.0; 3]]),
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };
    a.normals = Some(vec![[0.0; 3], [1.0; 3], [2.0; 3]]);

    let b = GeometryData {
        positions: vec![[3.0; 3], [4.0; 3], [5.0; 3]],
        normals: None, // 非全部含有 → 从结果中丢弃
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, 1.0),
        primitive_type: PrimitiveType::Triangles,
    };

    let combined = combine_geometries(&[a, b]);
    assert_eq!(combined.positions.len(), 6);
    assert_eq!(combined.positions[3], [3.0, 3.0, 3.0]);
    assert_eq!(combined.positions[5], [5.0, 5.0, 5.0]);
    // 索引按第一个几何体的顶点数偏移。
    assert_eq!(combined.indices, vec![0, 1, 2, 3, 4, 5]);
    // normal 仅存在于第一个几何体 → 被丢弃。
    assert!(combined.normals.is_none());
    assert_eq!(combined.primitive_type, PrimitiveType::Triangles);
}

#[test]
fn combine_instances_combines_bounding_spheres() {
    let a = GeometryData {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::new(0.5, 0.5, 0.0), 1.0),
        primitive_type: PrimitiveType::Triangles,
    };
    let b = GeometryData {
        positions: vec![[1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: vec![0, 1, 2],
        bounding_sphere: BoundingSphere::new(DVec3::new(1.5, 0.5, 0.0), 1.0),
        primitive_type: PrimitiveType::Triangles,
    };

    let expected = a.bounding_sphere.union(&b.bounding_sphere);
    let combined = combine_geometries(&[a, b]);
    assert!((combined.bounding_sphere.center - expected.center).length() < 1e-10);
    assert!((combined.bounding_sphere.radius - expected.radius).abs() < 1e-10);
}
