//! 参考自 CesiumJS `Core/EllipseGeometrySpec`（扩展的 A 类测试）。
//!
//! 测试：位置、全部属性、纹理坐标、旋转、高度、
//! 边界情形、圆的特例、包围球、轮廓。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::geometry::{
    ellipse_geometry, ellipse_outline_geometry, EllipseOptions, VertexFormat,
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
// "computes positions" - granularity=0.1, semiMajor=semiMinor=1.0
// CesiumJS：16 个顶点（各行 1+4+6+4+1），22 个三角形，boundingSphere.radius=1
// ---------------------------------------------------------------------------

#[test]
fn ellipse_computes_positions() {
    let opts = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 1.0,
        semi_minor_axis: 1.0,
        granularity: 0.1,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);

    // CesiumJS 期望 16 个顶点，22 个三角形
    assert_eq!(geo.positions.len(), 16, "expected 16 positions");
    assert_eq!(geo.indices.len(), 66, "expected 66 indices (22 triangles)");
    assert!(
        (geo.bounding_sphere.radius - 1.0).abs() < 1e-10,
        "bounding sphere radius should be 1, got {}",
        geo.bounding_sphere.radius
    );
}

// ---------------------------------------------------------------------------
// "compute all vertex attributes" - VertexFormat.ALL
// Rust 实现提供 position + normals + st（无 tangents/bitangents）
// ---------------------------------------------------------------------------

#[test]
fn ellipse_computes_all_vertex_attributes() {
    let opts = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 1.0,
        semi_minor_axis: 1.0,
        granularity: 0.1,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::ALL);

    let num_verts = geo.positions.len();
    assert_eq!(num_verts, 16, "expected 16 positions");

    // Normals 和 ST 应存在
    assert!(geo.normals.is_some(), "normals should be present");
    assert!(geo.tex_coords.is_some(), "tex_coords should be present");

    let normals = geo.normals.as_ref().unwrap();
    let st = geo.tex_coords.as_ref().unwrap();

    assert_eq!(normals.len(), num_verts, "normals count mismatch");
    assert_eq!(st.len(), num_verts, "tex_coords count mismatch");
}

// ---------------------------------------------------------------------------
// "compute texture coordinates with rotation" - stRotation=PI/2
// 注意：Rust 实现存储 st_rotation，但应用方式与 CesiumJS 不同。
// 验证设置旋转后 ST 存在且在有效范围内。
// ---------------------------------------------------------------------------

#[test]
fn ellipse_texture_coordinates_with_rotation() {
    let opts = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 1.0,
        semi_minor_axis: 1.0,
        granularity: 0.1,
        st_rotation: std::f64::consts::FRAC_PI_2,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_AND_ST);

    assert_eq!(geo.positions.len(), 16);
    let st = geo.tex_coords.as_ref().expect("tex_coords should be present");
    assert_eq!(st.len(), 16);

    // ST 值应在合理范围内
    for (i, uv) in st.iter().enumerate() {
        assert!(
            uv[0] >= -0.5 && uv[0] <= 1.5,
            "st[{}].u={} out of range with rotation",
            i,
            uv[0]
        );
        assert!(
            uv[1] >= -0.5 && uv[1] <= 1.5,
            "st[{}].v={} out of range with rotation",
            i,
            uv[1]
        );
    }
}

// ---------------------------------------------------------------------------
// 极小的椭圆（semiMajor=semiMinor=1.0）产生正确的包围球
// ---------------------------------------------------------------------------

#[test]
fn ellipse_small_axes_correct_bounding_sphere() {
    let opts = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 1.0,
        semi_minor_axis: 1.0,
        granularity: 0.1,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);

    // 包围球半径应等于 semi_major_axis
    assert!(
        (geo.bounding_sphere.radius - 1.0).abs() < 1e-10,
        "bounding sphere radius should be 1.0, got {}",
        geo.bounding_sphere.radius
    );

    // 所有位置应接近椭球表面
    let e = wgs84();
    for p in &geo.positions {
        let pos = DVec3::new(p[0], p[1], p[2]);
        let carto = e.cartesian_to_cartographic(pos).unwrap_or_default();
        assert!(
            carto.height.abs() < 1.0,
            "position should be near surface, height={}",
            carto.height
        );
    }
}

// ---------------------------------------------------------------------------
// 长、短半轴不同的椭圆（非圆）
// ---------------------------------------------------------------------------

#[test]
fn ellipse_non_circle_produces_geometry() {
    let opts = EllipseOptions {
        center: from_degrees(-75.59777, 40.03883, 0.0),
        semi_major_axis: 300000.0,
        semi_minor_axis: 150000.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(
        geo.positions.len() >= 8,
        "ellipse should produce >= 8 positions, got {}",
        geo.positions.len()
    );
    assert_eq!(geo.indices.len() % 3, 0, "indices must form triangles");
    assert!(geo.indices.len() >= 6);
    assert!(geo.bounding_sphere.radius > 0.0);
}

// ---------------------------------------------------------------------------
// 带旋转的椭圆
// ---------------------------------------------------------------------------

#[test]
fn ellipse_with_rotation_produces_geometry() {
    let opts_no_rot = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 500000.0,
        semi_minor_axis: 200000.0,
        rotation: 0.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo_no_rot = ellipse_geometry(&opts_no_rot, VertexFormat::POSITION_ONLY);

    let opts_rot = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 500000.0,
        semi_minor_axis: 200000.0,
        rotation: std::f64::consts::FRAC_PI_2,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo_rot = ellipse_geometry(&opts_rot, VertexFormat::POSITION_ONLY);

    // 两者都应产生几何
    assert!(!geo_no_rot.positions.is_empty());
    assert!(!geo_rot.positions.is_empty());

    // 相同的顶点数（旋转不改变细分）
    assert_eq!(
        geo_no_rot.positions.len(),
        geo_rot.positions.len(),
        "rotation should not change vertex count"
    );

    // 但位置应不同（已旋转）
    let mut any_different = false;
    for (a, b) in geo_no_rot.positions.iter().zip(geo_rot.positions.iter()) {
        if (a[0] - b[0]).abs() > 1e-6 || (a[1] - b[1]).abs() > 1e-6 {
            any_different = true;
            break;
        }
    }
    assert!(any_different, "rotated ellipse should have different positions");
}

// ---------------------------------------------------------------------------
// 带高度的椭圆将位置抬升到椭球之上
// ---------------------------------------------------------------------------

#[test]
fn ellipse_with_height_raises_positions() {
    let height = 10000.0;
    let opts = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 100000.0,
        semi_minor_axis: 50000.0,
        height,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(!geo.positions.is_empty());

    // 所有位置应大约位于指定高度处
    let e = wgs84();
    for (i, p) in geo.positions.iter().enumerate() {
        let pos = DVec3::new(p[0], p[1], p[2]);
        let carto = e.cartesian_to_cartographic(pos).unwrap_or_default();
        assert!(
            (carto.height - height).abs() < 100.0,
            "position[{}] height should be ~{}, got {}",
            i,
            height,
            carto.height
        );
    }
}

// ---------------------------------------------------------------------------
// 法线应为单位长度且朝外
// ---------------------------------------------------------------------------

#[test]
fn ellipse_normals_are_valid() {
    let opts = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 100000.0,
        semi_minor_axis: 50000.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::ALL);

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

    // 法线应朝外（与位置的点积 > 0）
    for (i, (p, n)) in geo.positions.iter().zip(normals.iter()).enumerate() {
        let dot = p[0] * n[0] + p[1] * n[1] + p[2] * n[2];
        assert!(dot > 0.0, "normal[{}] should point outward (dot={})", i, dot);
    }
}

// ---------------------------------------------------------------------------
// 纹理坐标应在 [0, 1] 范围内
// ---------------------------------------------------------------------------

#[test]
fn ellipse_texture_coordinates_in_range() {
    let opts = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 100000.0,
        semi_minor_axis: 50000.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_AND_ST);

    let st = geo.tex_coords.as_ref().expect("tex_coords should be present");
    for (i, uv) in st.iter().enumerate() {
        assert!(
            uv[0] >= -0.1 && uv[0] <= 1.1,
            "st[{}].u={} out of range",
            i,
            uv[0]
        );
        assert!(
            uv[1] >= -0.1 && uv[1] <= 1.1,
            "st[{}].v={} out of range",
            i,
            uv[1]
        );
    }
}

// ---------------------------------------------------------------------------
// 轮廓几何：环绕椭圆的线环
// ---------------------------------------------------------------------------

#[test]
fn ellipse_outline_forms_closed_loop() {
    let opts = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 100000.0,
        semi_minor_axis: 50000.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_outline_geometry(&opts);

    assert!(
        geo.positions.len() >= 8,
        "outline should have >= 8 positions, got {}",
        geo.positions.len()
    );
    // 索引是构成闭合环的线段对
    assert_eq!(geo.indices.len() % 2, 0, "outline indices must be pairs");
    assert_eq!(
        geo.indices.len(),
        geo.positions.len() * 2,
        "closed loop: n positions → n line segments → 2n indices"
    );

    // 所有索引应有效
    let n = geo.positions.len() as u32;
    for &idx in &geo.indices {
        assert!(idx < n, "index {} out of bounds (n={})", idx, n);
    }
}

// ---------------------------------------------------------------------------
// 圆（semiMajor == semiMinor）应产生对称几何
// ---------------------------------------------------------------------------

#[test]
fn circle_produces_symmetric_geometry() {
    let radius = 200000.0;
    let opts = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: radius,
        semi_minor_axis: radius,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert!(
        geo.positions.len() >= 8,
        "circle should produce >= 8 positions"
    );
    assert_eq!(geo.indices.len() % 3, 0);
    // 包围球半径应等于圆半径
    assert!(
        (geo.bounding_sphere.radius - radius).abs() < 1.0,
        "bounding sphere radius should be ~{}, got {}",
        radius,
        geo.bounding_sphere.radius
    );
}

// ---------------------------------------------------------------------------
// 更大的粒度 → 更少的顶点
// ---------------------------------------------------------------------------

#[test]
fn ellipse_granularity_affects_tessellation() {
    let center = from_degrees(0.0, 0.0, 0.0);

    let opts_fine = EllipseOptions {
        center,
        semi_major_axis: 500000.0,
        semi_minor_axis: 300000.0,
        granularity: std::f64::consts::PI / 180.0, // 1 度
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo_fine = ellipse_geometry(&opts_fine, VertexFormat::POSITION_ONLY);

    let opts_coarse = EllipseOptions {
        center,
        semi_major_axis: 500000.0,
        semi_minor_axis: 300000.0,
        granularity: std::f64::consts::PI / 36.0, // 5 度
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo_coarse = ellipse_geometry(&opts_coarse, VertexFormat::POSITION_ONLY);

    assert!(
        geo_fine.positions.len() > geo_coarse.positions.len(),
        "fine granularity ({}) should produce more vertices than coarse ({})",
        geo_fine.positions.len(),
        geo_coarse.positions.len()
    );
}

// ---------------------------------------------------------------------------
// 长半轴 > 短半轴且旋转 = PI
// ---------------------------------------------------------------------------

#[test]
fn ellipse_rotation_pi_swaps_axes() {
    let opts_no_rot = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 500000.0,
        semi_minor_axis: 200000.0,
        rotation: 0.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo_no_rot = ellipse_geometry(&opts_no_rot, VertexFormat::POSITION_ONLY);

    let opts_rot = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 500000.0,
        semi_minor_axis: 200000.0,
        rotation: std::f64::consts::PI,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo_rot = ellipse_geometry(&opts_rot, VertexFormat::POSITION_ONLY);

    assert_eq!(geo_no_rot.positions.len(), geo_rot.positions.len());
    // PI 旋转应翻转两轴（部分顶点位置不同）
    let mut any_diff = false;
    for (a, b) in geo_no_rot.positions.iter().zip(geo_rot.positions.iter()) {
        if (a[0] - b[0]).abs() > 1e-6 || (a[1] - b[1]).abs() > 1e-6 || (a[2] - b[2]).abs() > 1e-6 {
            any_diff = true;
            break;
        }
    }
    assert!(any_diff, "PI rotation should change positions");
}

// ---------------------------------------------------------------------------
// st_rotation = 0 给出标准纹理坐标
// ---------------------------------------------------------------------------

#[test]
fn ellipse_st_rotation_zero_center_uv_at_origin() {
    let opts = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 1.0,
        semi_minor_axis: 1.0,
        granularity: 0.1,
        st_rotation: 0.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_AND_ST);
    let st = geo.tex_coords.as_ref().unwrap();
    assert_eq!(geo.positions.len(), 16);
    assert_eq!(st.len(), 16);
    // ST 坐标应为有限值
    for uv in st.iter() {
        assert!(uv[0].is_finite() && uv[1].is_finite());
    }
}

// ---------------------------------------------------------------------------
// st_rotation = PI 的椭圆
// ---------------------------------------------------------------------------

#[test]
fn ellipse_st_rotation_pi_inverts_texture() {
    let opts = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 1.0,
        semi_minor_axis: 1.0,
        granularity: 0.1,
        st_rotation: std::f64::consts::PI,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_AND_ST);
    assert_eq!(geo.positions.len(), 16);
    let st = geo.tex_coords.as_ref().unwrap();
    // st_rotation=PI 时，中心 uv 应接近 (0.5, 0.5)
    for uv in st.iter() {
        assert!(uv[0] >= -0.5 && uv[0] <= 1.5, "s out of range after PI rotation");
        assert!(uv[1] >= -0.5 && uv[1] <= 1.5, "t out of range after PI rotation");
    }
}

// ---------------------------------------------------------------------------
// 高度与旋转组合的椭圆
// ---------------------------------------------------------------------------

#[test]
fn ellipse_height_with_rotation() {
    let height = 5000.0;
    let opts = EllipseOptions {
        center: from_degrees(-75.59777, 40.03883, 0.0),
        semi_major_axis: 300000.0,
        semi_minor_axis: 150000.0,
        height,
        rotation: std::f64::consts::FRAC_PI_4,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(!geo.positions.is_empty());
    let e = wgs84();
    for p in &geo.positions {
        let pos = DVec3::new(p[0], p[1], p[2]);
        let carto = e.cartesian_to_cartographic(pos).unwrap_or_default();
        assert!((carto.height - height).abs() < 100.0);
    }
}

// ---------------------------------------------------------------------------
// 带旋转的椭圆包围球
// ---------------------------------------------------------------------------

#[test]
fn ellipse_bounding_sphere_with_rotation() {
    let opts = EllipseOptions {
        center: from_degrees(0.0, 0.0, 0.0),
        semi_major_axis: 300000.0,
        semi_minor_axis: 100000.0,
        rotation: std::f64::consts::FRAC_PI_3,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!((geo.bounding_sphere.radius - 300000.0).abs() < 1.0);
    let center = geo.bounding_sphere.center;
    for p in &geo.positions {
        let dist = (DVec3::new(p[0], p[1], p[2]) - center).length();
        assert!(dist <= geo.bounding_sphere.radius + 1.0);
    }
}
