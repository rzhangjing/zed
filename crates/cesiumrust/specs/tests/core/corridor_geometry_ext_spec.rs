//! 参考自 CesiumJS `Core/CorridorGeometrySpec`（扩展的 A 类测试）。
//!
//! 测试内容：位置、全部属性、右转/左转、圆角/斜角、
//! 直线通道、边界情形、纹理坐标、法线。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::geometry::{
    corridor_geometry, corridor_outline_geometry, CornerType, CorridorOptions, VertexFormat,
};
use glam::DVec3;

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

fn from_degrees(lon: f64, lat: f64, h: f64) -> DVec3 {
    let e = wgs84();
    let carto = Cartographic::from_degrees(lon, lat, h);
    e.cartographic_to_cartesian(&carto)
}

// ---------------------------------------------------------------------------
// "computes positions" - 相距 5° 的 2 个位置，MITERED，width=30000
// CesiumJS：12 个顶点，10 个三角形。Rust 的条带三角化可能不同。
// ---------------------------------------------------------------------------

#[test]
fn corridor_computes_positions_mitered() {
    let opts = CorridorOptions {
        positions: vec![from_degrees(90.0, -30.0, 0.0), from_degrees(90.0, -35.0, 0.0)],
        width: 30000.0,
        corner_type: CornerType::Mitered,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);

    // 应产生至少 4 个顶点（右侧 2 + 左侧 2，最少）
    assert!(
        geo.positions.len() >= 4,
        "expected >= 4 positions, got {}",
        geo.positions.len()
    );
    // 索引必须能被 3 整除（三角形）
    assert_eq!(geo.indices.len() % 3, 0, "indices must form complete triangles");
    assert!(
        geo.indices.len() >= 6,
        "expected >= 2 triangles, got {} indices",
        geo.indices.len()
    );
    // 包围球应有效
    assert!(geo.bounding_sphere.radius > 0.0);
}

// ---------------------------------------------------------------------------
// "compute all vertex attributes" - VertexFormat.ALL
// ---------------------------------------------------------------------------

#[test]
fn corridor_computes_all_vertex_attributes() {
    let opts = CorridorOptions {
        positions: vec![from_degrees(90.0, -30.0, 0.0), from_degrees(90.0, -35.0, 0.0)],
        width: 30000.0,
        corner_type: CornerType::Mitered,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::ALL);

    let num_verts = geo.positions.len();
    assert!(num_verts >= 4, "expected >= 4 positions");

    // 所有属性都应存在
    assert!(geo.normals.is_some(), "normals should be present");
    assert!(geo.tangents.is_some(), "tangents should be present");
    assert!(geo.bitangents.is_some(), "bitangents should be present");
    assert!(geo.tex_coords.is_some(), "tex_coords should be present");

    let normals = geo.normals.as_ref().unwrap();
    let tangents = geo.tangents.as_ref().unwrap();
    let bitangents = geo.bitangents.as_ref().unwrap();
    let st = geo.tex_coords.as_ref().unwrap();

    assert_eq!(normals.len(), num_verts, "normals count mismatch");
    assert_eq!(tangents.len(), num_verts, "tangents count mismatch");
    assert_eq!(bitangents.len(), num_verts, "bitangents count mismatch");
    assert_eq!(st.len(), num_verts, "tex_coords count mismatch");
}

// ---------------------------------------------------------------------------
// "computes right turn" - 构成右转的 3 个位置，MITERED
// CesiumJS：8 个顶点，6 个三角形
// ---------------------------------------------------------------------------

#[test]
fn corridor_computes_right_turn() {
    let opts = CorridorOptions {
        positions: vec![
            from_degrees(90.0, -30.0, 0.0),
            from_degrees(90.0, -31.0, 0.0),
            from_degrees(91.0, -31.0, 0.0),
        ],
        width: 30000.0,
        corner_type: CornerType::Mitered,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(
        geo.positions.len() >= 6,
        "right turn should produce >= 6 positions, got {}",
        geo.positions.len()
    );
    assert_eq!(geo.indices.len() % 3, 0);
    assert!(geo.indices.len() >= 6);
}

// ---------------------------------------------------------------------------
// "computes left turn" - 构成左转的 3 个位置，MITERED
// CesiumJS：8 个顶点，6 个三角形
// ---------------------------------------------------------------------------

#[test]
fn corridor_computes_left_turn() {
    let opts = CorridorOptions {
        positions: vec![
            from_degrees(90.0, -30.0, 0.0),
            from_degrees(90.0, -31.0, 0.0),
            from_degrees(89.0, -31.0, 0.0),
        ],
        width: 30000.0,
        corner_type: CornerType::Mitered,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(
        geo.positions.len() >= 6,
        "left turn should produce >= 6 positions, got {}",
        geo.positions.len()
    );
    assert_eq!(geo.indices.len() % 3, 0);
    assert!(geo.indices.len() >= 6);
}

// ---------------------------------------------------------------------------
// "computes with rounded corners" - 4 个位置，ROUNDED
// 圆角应比斜角产生更多顶点
// ---------------------------------------------------------------------------

#[test]
fn corridor_computes_with_rounded_corners() {
    let positions = vec![
        from_degrees(90.0, -30.0, 0.0),
        from_degrees(90.0, -31.0, 0.0),
        from_degrees(89.0, -31.0, 0.0),
        from_degrees(89.0, -32.0, 0.0),
    ];

    let opts_rounded = CorridorOptions {
        positions: positions.clone(),
        width: 30000.0,
        corner_type: CornerType::Rounded,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo_rounded = corridor_geometry(&opts_rounded, VertexFormat::POSITION_AND_ST);

    assert!(
        geo_rounded.positions.len() >= 8,
        "rounded should produce >= 8 positions, got {}",
        geo_rounded.positions.len()
    );
    assert_eq!(geo_rounded.indices.len() % 3, 0);
    // ST 应存在
    assert!(geo_rounded.tex_coords.is_some());
    let st = geo_rounded.tex_coords.as_ref().unwrap();
    assert_eq!(st.len(), geo_rounded.positions.len());

    // 与斜角对比：圆角应有更多顶点
    let opts_beveled = CorridorOptions {
        positions,
        width: 30000.0,
        corner_type: CornerType::Beveled,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo_beveled = corridor_geometry(&opts_beveled, VertexFormat::POSITION_ONLY);

    assert!(
        geo_rounded.positions.len() > geo_beveled.positions.len(),
        "rounded ({}) should have more vertices than beveled ({})",
        geo_rounded.positions.len(),
        geo_beveled.positions.len()
    );
}

// ---------------------------------------------------------------------------
// "computes with beveled corners" - 4 个位置，BEVELED
// CesiumJS：10 个顶点，8 个三角形
// ---------------------------------------------------------------------------

#[test]
fn corridor_computes_with_beveled_corners() {
    let opts = CorridorOptions {
        positions: vec![
            from_degrees(90.0, -30.0, 0.0),
            from_degrees(90.0, -31.0, 0.0),
            from_degrees(89.0, -31.0, 0.0),
            from_degrees(89.0, -32.0, 0.0),
        ],
        width: 30000.0,
        corner_type: CornerType::Beveled,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(
        geo.positions.len() >= 8,
        "beveled should produce >= 8 positions, got {}",
        geo.positions.len()
    );
    assert_eq!(geo.indices.len() % 3, 0);
    assert!(geo.indices.len() >= 6);
}

// ---------------------------------------------------------------------------
// "computes sharp turns" - 夹角尖锐的 5 个位置，BEVELED
// CesiumJS：13 个顶点，11 个三角形
// ---------------------------------------------------------------------------

#[test]
fn corridor_computes_sharp_turns() {
    let opts = CorridorOptions {
        positions: vec![
            from_degrees(2.00571672577652, 52.7781459942399, 0.0),
            from_degrees(1.99188457974115, 52.7764958852886, 0.0),
            from_degrees(2.01325961458495, 52.7674170680511, 0.0),
            from_degrees(1.98708058340534, 52.7733979856253, 0.0),
            from_degrees(2.00634853946644, 52.7650460748473, 0.0),
        ],
        width: 100.0,
        corner_type: CornerType::Beveled,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(
        geo.positions.len() >= 8,
        "sharp turns should produce >= 8 positions, got {}",
        geo.positions.len()
    );
    assert_eq!(geo.indices.len() % 3, 0);
    assert!(geo.indices.len() >= 6);
}

// ---------------------------------------------------------------------------
// "computes straight corridors" - 3 个共线位置，BEVELED，granularity=PI/6
// CesiumJS：4 个顶点，2 个三角形（共线 → 无拐角，最少细分）
// ---------------------------------------------------------------------------

#[test]
fn corridor_computes_straight_corridors() {
    let opts = CorridorOptions {
        positions: vec![
            from_degrees(-67.655, 0.0, 0.0),
            from_degrees(-67.655, 15.0, 0.0),
            from_degrees(-67.655, 20.0, 0.0),
        ],
        width: 400000.0,
        corner_type: CornerType::Beveled,
        granularity: std::f64::consts::PI / 6.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);

    // 直线通道：应产生顶点数最少的几何体
    assert!(
        geo.positions.len() >= 4,
        "straight corridor should produce >= 4 positions, got {}",
        geo.positions.len()
    );
    assert_eq!(geo.indices.len() % 3, 0);
    assert!(geo.indices.len() >= 6);
}

// ---------------------------------------------------------------------------
// "undefined is returned if less than 2 positions or width <= 0"
// ---------------------------------------------------------------------------

#[test]
fn corridor_returns_empty_for_invalid_input() {
    let e = wgs84();

    // 仅 1 个位置
    let opts1 = CorridorOptions {
        positions: vec![from_degrees(-72.0, 35.0, 0.0)],
        width: 100000.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo1 = corridor_geometry(&opts1, VertexFormat::POSITION_ONLY);
    assert!(geo1.positions.is_empty(), "1 position should return empty");

    // 宽度 = 0
    let opts2 = CorridorOptions {
        positions: vec![
            from_degrees(-67.655, 0.0, 0.0),
            from_degrees(-67.655, 15.0, 0.0),
            from_degrees(-67.655, 20.0, 0.0),
        ],
        width: 0.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo2 = corridor_geometry(&opts2, VertexFormat::POSITION_ONLY);
    assert!(geo2.positions.is_empty(), "width=0 should return empty");

    // 宽度 < 0
    let opts3 = CorridorOptions {
        positions: vec![
            from_degrees(-67.655, 0.0, 0.0),
            from_degrees(-67.655, 15.0, 0.0),
            from_degrees(-67.655, 20.0, 0.0),
        ],
        width: -100.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo3 = corridor_geometry(&opts3, VertexFormat::POSITION_ONLY);
    assert!(geo3.positions.is_empty(), "width<0 should return empty");
}

// ---------------------------------------------------------------------------
// "createGeometry returns undefined without 2 unique positions"
// 重复位置（相同经纬度）应产生空几何体
// ---------------------------------------------------------------------------

#[test]
fn corridor_returns_empty_for_duplicate_positions() {
    // 同一位置出现两次
    let opts = CorridorOptions {
        positions: vec![from_degrees(90.0, -30.0, 0.0), from_degrees(90.0, -30.0, 0.0)],
        width: 10000.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(
        geo.positions.is_empty(),
        "duplicate positions should return empty geometry"
    );
}

// ---------------------------------------------------------------------------
// 纹理坐标：右边缘 v=0，左边缘 v=1
// ---------------------------------------------------------------------------

#[test]
fn corridor_texture_coordinates_pattern() {
    let opts = CorridorOptions {
        positions: vec![from_degrees(90.0, -30.0, 0.0), from_degrees(90.0, -35.0, 0.0)],
        width: 30000.0,
        corner_type: CornerType::Mitered,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::ALL);

    let st = geo.tex_coords.as_ref().expect("tex_coords should be present");
    let num_verts = geo.positions.len();
    assert_eq!(st.len(), num_verts);

    // 实现先存储右边缘（v=0），再存左边缘（v=1）
    // 右边缘：顶点的前半部分
    let half = num_verts / 2;
    for i in 0..half {
        assert!(
            (st[i][1] - 0.0).abs() < 1e-6,
            "right edge st[{}].v should be 0, got {}",
            i,
            st[i][1]
        );
    }
    // 左边缘：顶点的后半部分
    for i in half..num_verts {
        assert!(
            (st[i][1] - 1.0).abs() < 1e-6,
            "left edge st[{}].v should be 1, got {}",
            i,
            st[i][1]
        );
    }

    // 第一个右边缘 u 应为 0，最后一个右边缘 u 应为 1
    assert!((st[0][0] - 0.0).abs() < 1e-6, "first right u should be 0");
    if half > 1 {
        assert!(
            (st[half - 1][0] - 1.0).abs() < 1e-6,
            "last right u should be 1, got {}",
            st[half - 1][0]
        );
    }
}

// ---------------------------------------------------------------------------
// 法线应为单位长度并背离椭球中心
// ---------------------------------------------------------------------------

#[test]
fn corridor_normals_are_valid() {
    let opts = CorridorOptions {
        positions: vec![from_degrees(90.0, -30.0, 0.0), from_degrees(90.0, -35.0, 0.0)],
        width: 30000.0,
        corner_type: CornerType::Mitered,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::ALL);

    let normals = geo.normals.as_ref().expect("normals should be present");
    for (i, n) in normals.iter().enumerate() {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!(
            (len - 1.0).abs() < 1e-6,
            "normal[{}] should be unit length, got {}",
            i,
            len
        );
    }

    // 法线通常应朝外（与位置的点积 > 0）
    for (i, (p, n)) in geo.positions.iter().zip(normals.iter()).enumerate() {
        let dot = p[0] * n[0] + p[1] * n[1] + p[2] * n[2];
        assert!(
            dot > 0.0,
            "normal[{}] should point outward (dot={})",
            i,
            dot
        );
    }
}

// ---------------------------------------------------------------------------
// 轮廓几何：由线段对构成的闭合环
// ---------------------------------------------------------------------------

#[test]
fn corridor_outline_forms_closed_loop() {
    let opts = CorridorOptions {
        positions: vec![
            from_degrees(90.0, -30.0, 0.0),
            from_degrees(90.0, -31.0, 0.0),
            from_degrees(91.0, -31.0, 0.0),
        ],
        width: 30000.0,
        corner_type: CornerType::Mitered,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_outline_geometry(&opts);

    assert!(
        geo.positions.len() >= 4,
        "outline should have >= 4 positions"
    );
    // 索引为线段对
    assert_eq!(geo.indices.len() % 2, 0, "outline indices must be pairs");
    // 应构成闭合环：最后一对索引连回 0
    let n = geo.positions.len() as u32;
    for &idx in &geo.indices {
        assert!(idx < n, "index {} out of bounds (n={})", idx, n);
    }
    // 最后一对应为 (n-1, 0) 以闭合环
    let last_pair_start = geo.indices.len() - 2;
    assert_eq!(geo.indices[last_pair_start], n - 1);
    assert_eq!(geo.indices[last_pair_start + 1], 0);
}

// ---------------------------------------------------------------------------
// 带高度的通道：位置应被抬升到椭球之上
// ---------------------------------------------------------------------------

#[test]
fn corridor_with_height_raises_positions() {
    let height = 10000.0;
    let opts = CorridorOptions {
        positions: vec![from_degrees(90.0, -30.0, 0.0), from_degrees(90.0, -35.0, 0.0)],
        width: 30000.0,
        height,
        corner_type: CornerType::Mitered,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(!geo.positions.is_empty());

    // 所有位置应位于椭球表面之上
    // 注意：通道位置由中线偏移 half_width，
    // 因此基于法线的高度抬升是近似的（30km 宽度容差约 50m）
    let e = wgs84();
    for (i, p) in geo.positions.iter().enumerate() {
        let pos = DVec3::new(p[0], p[1], p[2]);
        let carto = e.cartesian_to_cartographic(pos).unwrap_or_default();
        assert!(
            (carto.height - height).abs() < 50.0,
            "position[{}] height should be ~{}, got {}",
            i,
            height,
            carto.height
        );
    }
}

// ---------------------------------------------------------------------------
// 粒度影响顶点数
// ---------------------------------------------------------------------------

#[test]
fn corridor_granularity_affects_tessellation() {
    let positions = vec![
        from_degrees(90.0, -30.0, 0.0),
        from_degrees(90.0, -35.0, 0.0),
    ];
    let opts_fine = CorridorOptions {
        positions: positions.clone(),
        width: 30000.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo_fine = corridor_geometry(&opts_fine, VertexFormat::POSITION_ONLY);
    let opts_coarse = CorridorOptions {
        positions,
        width: 30000.0,
        granularity: std::f64::consts::PI / 6.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo_coarse = corridor_geometry(&opts_coarse, VertexFormat::POSITION_ONLY);
    assert!(
        geo_fine.positions.len() > geo_coarse.positions.len(),
        "fine ({}) > coarse ({})",
        geo_fine.positions.len(),
        geo_coarse.positions.len()
    );
}

#[test]
fn corridor_with_height_and_rounded() {
    let height = 5000.0;
    let opts = CorridorOptions {
        positions: vec![
            from_degrees(90.0, -30.0, 0.0),
            from_degrees(90.0, -31.0, 0.0),
            from_degrees(91.0, -31.0, 0.0),
        ],
        width: 30000.0,
        height,
        corner_type: CornerType::Rounded,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(!geo.positions.is_empty());
}

#[test]
fn corridor_two_positions_defaults() {
    let opts = CorridorOptions {
        positions: vec![
            from_degrees(-72.0, 35.0, 0.0),
            from_degrees(-71.0, 35.0, 0.0),
        ],
        width: 100000.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(geo.positions.len() >= 4);
    assert_eq!(geo.indices.len() % 3, 0);
}

#[test]
fn corridor_very_small_width_still_handled() {
    let geo = corridor_geometry(
        &CorridorOptions {
            positions: vec![from_degrees(0.0, 0.0, 0.0), from_degrees(0.0, 1.0, 0.0)],
            width: 1e-12,
            ellipsoid: wgs84(),
            ..Default::default()
        },
        VertexFormat::POSITION_ONLY,
    );
    if !geo.positions.is_empty() {
        assert!(geo.indices.len() % 3 == 0);
    }
}
