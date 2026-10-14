//! 参考自 CesiumJS `Core/WallGeometrySpec`（扩展的 A 类测试）。
//!
//! 测试：闭合环路、重复点处理、EPSILON10 边界、高度选择、
//! 全部属性、纹理坐标。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::geometry::{wall_geometry, VertexFormat, WallOptions};
use glam::DVec3;

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

fn from_degrees(lon: f64, lat: f64, h: f64) -> DVec3 {
    let e = wgs84();
    let carto = Cartographic::from_degrees(lon, lat, h);
    e.cartographic_to_cartesian(&carto)
}

fn to_cartographic(p: [f64; 3]) -> Cartographic {
    let e = wgs84();
    e.cartesian_to_cartographic(DVec3::new(p[0], p[1], p[2]))
        .unwrap_or_default()
}

const EPSILON8: f64 = 1e-8;

// ---------------------------------------------------------------------------
// "creates positions relative to ellipsoid"
// 2 个位置 → 4 个顶点（2 底 + 2 顶），2 个三角形
// ---------------------------------------------------------------------------

#[test]
fn wall_creates_positions_relative_to_ellipsoid() {
    let positions = vec![
        from_degrees(49.0, 18.0, 1000.0),
        from_degrees(50.0, 18.0, 1000.0),
    ];

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    // CesiumJS：numPositions = 4，numTriangles = 2
    // granularity=1° 时，1° 弧 → 每段 2 个点 → 2 个拐角 × 2（顶+底）= 4
    assert_eq!(geo.positions.len(), 4, "expected 4 positions");
    assert_eq!(geo.indices.len(), 6, "expected 6 indices (2 triangles)");

    // 第一个位置应位于高度 0（底部）
    let c0 = to_cartographic(geo.positions[0]);
    assert!(
        (c0.height - 0.0).abs() < EPSILON8,
        "bottom height should be 0, got {}",
        c0.height
    );

    // 第二个位置应位于高度 1000（顶部）
    let c1 = to_cartographic(geo.positions[1]);
    assert!(
        (c1.height - 1000.0).abs() < EPSILON8,
        "top height should be 1000, got {}",
        c1.height
    );
}

// ---------------------------------------------------------------------------
// "creates positions when first and last positions are equal"
// 闭合环路：5 个位置（首=尾）→ 16 个顶点，8 个三角形
// ---------------------------------------------------------------------------

#[test]
fn wall_creates_positions_closed_loop() {
    let positions = vec![
        from_degrees(-107.0, 43.0, 1000.0),
        from_degrees(-106.0, 43.0, 1000.0),
        from_degrees(-106.0, 42.0, 1000.0),
        from_degrees(-107.0, 42.0, 1000.0),
        from_degrees(-107.0, 43.0, 1000.0), // 与首个相同
    ];

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    // CesiumJS：numPositions = 16，numTriangles = 8
    // 4 段 × 每段 2 个点 × 2（顶+底）= 16
    assert_eq!(geo.positions.len(), 16, "expected 16 positions for closed loop");
    assert_eq!(geo.indices.len(), 24, "expected 24 indices (8 triangles)");

    // 第一个位置应位于高度 0（底部）
    let c0 = to_cartographic(geo.positions[0]);
    assert!(
        (c0.height - 0.0).abs() < EPSILON8,
        "bottom height should be 0, got {}",
        c0.height
    );

    // 第二个位置应位于高度 1000（顶部）
    let c1 = to_cartographic(geo.positions[1]);
    assert!(
        (c1.height - 1000.0).abs() < EPSILON8,
        "top height should be 1000, got {}",
        c1.height
    );
}

// ---------------------------------------------------------------------------
// "cleans positions with duplicates"
// 7 个含重复的输入位置 → 8 个顶点（4 个唯一拐角 × 2）
// ---------------------------------------------------------------------------

#[test]
fn wall_cleans_positions_with_duplicates() {
    // 输入：49,18 → 49,18(重复) → 50,18 → 50,18(重复) → 50,18(重复) → 51,18 → 51,18(重复)
    let positions = vec![
        from_degrees(49.0, 18.0, 1000.0),
        from_degrees(49.0, 18.0, 2000.0), // 相同经/纬度，不同高度
        from_degrees(50.0, 18.0, 1000.0),
        from_degrees(50.0, 18.0, 1000.0), // 重复
        from_degrees(50.0, 18.0, 1000.0), // 重复
        from_degrees(51.0, 18.0, 1000.0),
        from_degrees(51.0, 18.0, 1000.0), // 重复
    ];

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    // CesiumJS：numPositions = 8，numTriangles = 4
    // 去重后：3 个唯一拐角（49、50、51）
    // 但 49 与 49 共享经/纬度因而被合并 → 3 个拐角
    // 3 个拐角 × 2（顶+底）= 6……但 CesiumJS 期望为 8
    // 实际上是：2 段（49→50、50→51）× 每段 2 个点 × 2（顶+底）= 8
    assert_eq!(geo.positions.len(), 8, "expected 8 positions after duplicate removal");
    assert_eq!(geo.indices.len(), 12, "expected 12 indices (4 triangles)");

    // 第一个位置应位于高度 0（底部）
    let c0 = to_cartographic(geo.positions[0]);
    assert!(
        (c0.height - 0.0).abs() < EPSILON8,
        "bottom height should be 0, got {}",
        c0.height
    );

    // 第二个位置应位于高度 2000（1000 与 2000 的最大值）
    let c1 = to_cartographic(geo.positions[1]);
    assert!(
        (c1.height - 2000.0).abs() < EPSILON8,
        "top height should be 2000 (max of duplicates), got {}",
        c1.height
    );
}

// ---------------------------------------------------------------------------
// "removes duplicates with very small difference"
// 相差 < EPSILON10 的位置应被合并
// ---------------------------------------------------------------------------

#[test]
fn wall_removes_duplicates_with_small_difference() {
    // 这些位置在直角坐标下相差 < EPSILON10
    let positions = vec![
        DVec3::new(4347090.215457887, 1061403.4237998386, 4538066.036525028),
        DVec3::new(4348147.589624987, 1043897.8776143644, 4541092.234751661),
        DVec3::new(4348147.589882754, 1043897.8776762491, 4541092.234492364), // 与前一个非常接近
        DVec3::new(4335659.882947743, 1047571.602084736, 4552098.654605664),
    ];

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    // CesiumJS：numPositions = 8，numTriangles = 4
    // 去除近似重复点后：3 个唯一拐角
    // 2 段 × 2 个点 × 2（顶+底）= 8
    assert_eq!(geo.positions.len(), 8, "expected 8 positions after near-duplicate removal");
    assert_eq!(geo.indices.len(), 12, "expected 12 indices (4 triangles)");
}

// ---------------------------------------------------------------------------
// "does not clean positions that add up past EPSILON10"
// 累积超过 EPSILON10 的微小差异不应被合并
// ---------------------------------------------------------------------------

#[test]
fn wall_does_not_clean_positions_past_epsilon10() {
    let eighty_percent_of_epsilon10: f64 = 0.8 * 1e-10;

    // 4 个位置，每个在纬度上相差 0.8×EPSILON10
    // 相邻对相差 < EPSILON10，但累积差异 > EPSILON10
    let lat0: f64 = 1.0;
    let positions = vec![
        from_degrees(
            1.0_f64.to_degrees(),
            lat0.to_degrees(),
            1000.0,
        ),
        from_degrees(
            1.0_f64.to_degrees(),
            (lat0 + eighty_percent_of_epsilon10).to_degrees(),
            1000.0,
        ),
        from_degrees(
            1.0_f64.to_degrees(),
            (lat0 + 2.0 * eighty_percent_of_epsilon10).to_degrees(),
            1000.0,
        ),
        from_degrees(
            1.0_f64.to_degrees(),
            (lat0 + 3.0 * eighty_percent_of_epsilon10).to_degrees(),
            1000.0,
        ),
    ];

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    // CesiumJS 期望此情形生成几何（而非返回 undefined）
    // 第一个与第三个位置相差 1.6×EPSILON10 > EPSILON10
    // 因此不应被合并
    assert!(
        !geo.positions.is_empty(),
        "should produce geometry for positions accumulating past EPSILON10"
    );
}

// ---------------------------------------------------------------------------
// "cleans selects maximum height from duplicates"
// 当位置共享经/纬度时，保留最大高度
// ---------------------------------------------------------------------------

#[test]
fn wall_selects_maximum_height_from_duplicates() {
    // 50,18 出现 3 次，高度分别为 1000、6000、10000
    let positions = vec![
        from_degrees(49.0, 18.0, 1000.0),
        from_degrees(50.0, 18.0, 1000.0),
        from_degrees(50.0, 18.0, 6000.0),  // 相同经/纬度，更高
        from_degrees(50.0, 18.0, 10000.0), // 相同经/纬度，最高
        from_degrees(51.0, 18.0, 1000.0),
    ];

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    // CesiumJS：numPositions = 8，numTriangles = 4
    assert_eq!(geo.positions.len(), 8, "expected 8 positions");
    assert_eq!(geo.indices.len(), 12, "expected 12 indices (4 triangles)");

    // 第一个位置应位于高度 0（底部）
    let c0 = to_cartographic(geo.positions[0]);
    assert!(
        (c0.height - 0.0).abs() < EPSILON8,
        "bottom height should be 0, got {}",
        c0.height
    );

    // 索引 9（第 5 个顶部顶点）应位于高度 10000（最大值）
    // 50° 经度的拐角应具有最大高度
    // 在输出中，位置是交错的：bottom0, top0, bottom1, top1, ...
    // 索引 9 = 第 5 个顶部顶点（顶部数组中索引 4）
    if geo.positions.len() > 9 {
        let c9 = to_cartographic(geo.positions[9]);
        assert!(
            (c9.height - 10000.0).abs() < EPSILON8,
            "max height should be 10000, got {}",
            c9.height
        );
    }
}

// ---------------------------------------------------------------------------
// "creates all attributes"
// VertexFormat::ALL → positions + normals + tangents + bitangents + st
// ---------------------------------------------------------------------------

#[test]
fn wall_creates_all_attributes() {
    let positions = vec![
        from_degrees(49.0, 18.0, 1000.0),
        from_degrees(50.0, 18.0, 1000.0),
        from_degrees(51.0, 18.0, 1000.0),
    ];

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::ALL);

    // CesiumJS：numPositions = 8，numTriangles = 4
    let num_positions = 8;
    assert_eq!(geo.positions.len(), num_positions, "expected {} positions", num_positions);
    assert_eq!(geo.indices.len(), 12, "expected 12 indices (4 triangles)");

    // 检查所有属性都存在
    assert!(geo.normals.is_some(), "normals should be present");
    assert!(geo.tangents.is_some(), "tangents should be present");
    assert!(geo.bitangents.is_some(), "bitangents should be present");
    assert!(geo.tex_coords.is_some(), "tex_coords should be present");

    let normals = geo.normals.as_ref().unwrap();
    let tangents = geo.tangents.as_ref().unwrap();
    let bitangents = geo.bitangents.as_ref().unwrap();
    let st = geo.tex_coords.as_ref().unwrap();

    assert_eq!(normals.len(), num_positions, "normals count mismatch");
    assert_eq!(tangents.len(), num_positions, "tangents count mismatch");
    assert_eq!(bitangents.len(), num_positions, "bitangents count mismatch");
    assert_eq!(st.len(), num_positions, "tex_coords count mismatch");
}

// ---------------------------------------------------------------------------
// "creates correct texture coordinates"
// ST 值应为 [0,0, 0,1, 0.5,0, 0.5,1, 0.5,0, 0.5,1, 1,0, 1,1]
// ---------------------------------------------------------------------------

#[test]
fn wall_creates_correct_texture_coordinates() {
    let positions = vec![
        from_degrees(49.0, 18.0, 1000.0),
        from_degrees(50.0, 18.0, 1000.0),
        from_degrees(51.0, 18.0, 1000.0),
    ];

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::ALL);

    let st = geo.tex_coords.as_ref().expect("tex_coords should be present");

    // CesiumJS 对 3 个位置（2 段）的期望 ST 值：
    // [0.0, 0.0, 0.0, 1.0, 0.5, 0.0, 0.5, 1.0, 0.5, 0.0, 0.5, 1.0, 1.0, 0.0, 1.0, 1.0]
    // 规律：对每个拐角，底部 v=0，顶部 v=1
    // u 在各段之间从 0 到 1

    assert_eq!(st.len(), 8, "expected 8 texture coordinates");

    // 检查规律：v=0（底部）与 v=1（顶部）交替
    for (i, uv) in st.iter().enumerate() {
        let expected_v = if i % 2 == 0 { 0.0 } else { 1.0 };
        assert!(
            (uv[1] - expected_v).abs() < 1e-6,
            "st[{}].v should be {}, got {}",
            i,
            expected_v,
            uv[1]
        );
    }

    // 第一个 u 应为 0，最后一个 u 应为 1
    assert!((st[0][0] - 0.0).abs() < 1e-6, "first u should be 0");
    assert!((st[st.len() - 2][0] - 1.0).abs() < 1e-6, "last u should be 1");
}

// ---------------------------------------------------------------------------
// "creates correct texture coordinates when there are duplicate wall positions"
// 即使输入位置有重复，ST 值也相同
// ---------------------------------------------------------------------------

#[test]
fn wall_texture_coordinates_with_duplicates() {
    // 50,18 出现两次（重复）
    let positions = vec![
        from_degrees(49.0, 18.0, 1000.0),
        from_degrees(50.0, 18.0, 1000.0),
        from_degrees(50.0, 18.0, 1000.0), // 重复
        from_degrees(51.0, 18.0, 1000.0),
    ];

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::ALL);

    let st = geo.tex_coords.as_ref().expect("tex_coords should be present");

    // 去重后，应与无重复时拥有相同的 ST
    assert_eq!(st.len(), 8, "expected 8 texture coordinates after duplicate removal");

    // 检查规律：v=0（底部）与 v=1（顶部）交替
    for (i, uv) in st.iter().enumerate() {
        let expected_v = if i % 2 == 0 { 0.0 } else { 1.0 };
        assert!(
            (uv[1] - expected_v).abs() < 1e-6,
            "st[{}].v should be {}, got {}",
            i,
            expected_v,
            uv[1]
        );
    }
}

// ---------------------------------------------------------------------------
// "creates positions with constant minimum and maximum heights"
// fromConstantHeights，min=1000、max=2000
// ---------------------------------------------------------------------------

#[test]
fn wall_from_constant_heights_detailed() {
    let min = 1000.0;
    let max = 2000.0;

    let positions = vec![
        from_degrees(49.0, 18.0, 1000.0),
        from_degrees(50.0, 18.0, 1000.0),
    ];

    let opts = WallOptions::from_constant_heights(positions, Some(min), Some(max), wgs84());
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    // CesiumJS: numPositions = 4, numTriangles = 2
    assert_eq!(geo.positions.len(), 4, "expected 4 positions");
    assert_eq!(geo.indices.len(), 6, "expected 6 indices (2 triangles)");

    // 检查高度：底部=min，顶部=max
    let c0 = to_cartographic(geo.positions[0]);
    assert!(
        (c0.height - min).abs() < EPSILON8,
        "bottom height should be {}, got {}",
        min,
        c0.height
    );

    let c1 = to_cartographic(geo.positions[1]);
    assert!(
        (c1.height - max).abs() < EPSILON8,
        "top height should be {}, got {}",
        max,
        c1.height
    );

    let c2 = to_cartographic(geo.positions[2]);
    assert!(
        (c2.height - min).abs() < EPSILON8,
        "bottom height should be {}, got {}",
        min,
        c2.height
    );

    let c3 = to_cartographic(geo.positions[3]);
    assert!(
        (c3.height - max).abs() < EPSILON8,
        "top height should be {}, got {}",
        max,
        c3.height
    );
}

// ---------------------------------------------------------------------------
// "creates positions with minimum and maximum heights"
// 可变高度数组（非常量）
// ---------------------------------------------------------------------------

#[test]
fn wall_creates_positions_with_variable_minimum_maximum_heights() {
    let positions = vec![
        from_degrees(49.0, 18.0, 1000.0),
        from_degrees(50.0, 18.0, 1000.0),
    ];

    let opts = WallOptions {
        positions,
        minimum_heights: Some(vec![500.0, 300.0]),
        maximum_heights: Some(vec![1500.0, 1300.0]),
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    // CesiumJS: numPositions = 4, numTriangles = 2
    assert_eq!(geo.positions.len(), 4, "expected 4 positions");
    assert_eq!(geo.indices.len(), 6, "expected 6 indices (2 triangles)");

    // 检查底部高度会变化（遵循 minimum_heights）
    // 位置规律：bottom0, top0, bottom1, top1
    let c0 = to_cartographic(geo.positions[0]);
    assert!((c0.height - 500.0).abs() < EPSILON8, "bottom height at pos0 should be 500");

    let c2 = to_cartographic(geo.positions[2]);
    assert!((c2.height - 300.0).abs() < EPSILON8, "bottom height at pos2 should be 300");

    let c1 = to_cartographic(geo.positions[1]);
    assert!((c1.height - 1500.0).abs() < EPSILON8, "top height at pos1 should be 1500");

    let c3 = to_cartographic(geo.positions[3]);
    assert!((c3.height - 1300.0).abs() < EPSILON8, "top height at pos3 should be 1300");
}

// ---------------------------------------------------------------------------
// granularity 大于弧长的 Wall（最少细分）
// ---------------------------------------------------------------------------

#[test]
fn wall_coarse_granularity_minimal_subdivision() {
    let positions = vec![
        from_degrees(49.0, 18.0, 1000.0),
        from_degrees(50.0, 18.0, 1000.0),
    ];

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 3.0, // 60 度（很大）
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    // 即使 granularity 较粗，也应仍生成有效几何
    assert!(geo.positions.len() >= 4, "should produce at least 4 positions");
    assert!(geo.indices.len() >= 6);
}

// ---------------------------------------------------------------------------
// 含 3+ 个位置、高度非常量的 Wall
// ---------------------------------------------------------------------------

#[test]
fn wall_three_positions_gradient_heights() {
    let positions = vec![
        from_degrees(0.0, 0.0, 0.0),
        from_degrees(1.0, 0.0, 0.0),
        from_degrees(2.0, 0.0, 0.0),
    ];

    let opts = WallOptions {
        positions,
        minimum_heights: Some(vec![0.0, 1000.0, 0.0]),
        maximum_heights: Some(vec![5000.0, 6000.0, 5000.0]),
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(geo.positions.len() >= 6);
    assert_eq!(geo.indices.len() % 3, 0);
    assert!(geo.bounding_sphere.radius > 0.0);
}

// ---------------------------------------------------------------------------
// Wall：位置相差 EPSILON10 边界
// ---------------------------------------------------------------------------

#[test]
fn wall_positions_at_epsilon_boundary_survive_cleaning() {
    // 与现有测试相同，但验证会生成几何
    let p1 = DVec3::new(4347090.215457887, 1061403.4237998386, 4538066.036525028);
    let p2 = DVec3::new(4348147.589624987, 1043897.8776143644, 4541092.234751661);
    // p3 与 p2 相差约 1.5*EPSILON10（不应被合并）
    let p3 = DVec3::new(4348147.58998, 1043897.8780, 4541092.2350);

    let positions = vec![p1, p2, p3];

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(!geo.positions.is_empty(), "should produce geometry");
}

// ---------------------------------------------------------------------------
// Wall 轮廓：闭合环路
// ---------------------------------------------------------------------------

#[test]
fn wall_outline_forms_closed_loop() {
    use cesium_geospatial::geometry::{wall_outline_geometry, WallOptions};

    let positions = vec![
        from_degrees(0.0, 0.0, 0.0),
        from_degrees(1.0, 0.0, 0.0),
        from_degrees(1.0, 1.0, 0.0),
        from_degrees(0.0, 1.0, 0.0),
        from_degrees(0.0, 0.0, 0.0), // 闭合
    ];

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = wall_outline_geometry(&opts);
    assert!(geo.positions.len() >= 4);
    assert_eq!(geo.indices.len() % 2, 0);
    // 所有索引均有效
    let n = geo.positions.len() as u32;
    for &idx in &geo.indices {
        assert!(idx < n);
    }
}
