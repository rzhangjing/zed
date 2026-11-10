//! CircleOutline、FrustumGeometry 与 GroundPolyline
//! 几何的扩展数学属性测试。
//!
//! 参考自 CesiumJS CircleOutlineGeometrySpec/FrustumGeometrySpec/
//! GroundPolylineGeometrySpec 的 A 类测试。

use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::frustum::{OrthographicFrustum, PerspectiveFrustum};
use cesium_geospatial::geometry::{
    circle_outline_geometry, frustum_geometry, frustum_outline_geometry, ground_polyline_geometry,
    FrustumDef, GroundPolylineOptions, PrimitiveType, VertexFormat,
};
use glam::{DQuat, DVec3};

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

fn perspective_frustum() -> FrustumDef {
    FrustumDef::Perspective(PerspectiveFrustum::new(
        std::f64::consts::FRAC_PI_3,
        16.0 / 9.0,
        1.0,
        100.0,
    ))
}

fn orthographic_frustum() -> FrustumDef {
    FrustumDef::Orthographic(OrthographicFrustum {
        width: 10.0,
        aspect_ratio: 16.0 / 9.0,
        near: 1.0,
        far: 100.0,
    })
}

// ===========================================================================
// CircleOutlineGeometry - 扩展的数学属性
// ===========================================================================

#[test]
fn circle_outline_forms_closed_loop() {
    // 最后一个索引对应连回顶点 0（闭合环）
    let center = wgs84().cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let geo = circle_outline_geometry(center, 100_000.0, &wgs84(), std::f64::consts::PI / 180.0);

    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    let n = geo.positions.len() as u32;
    assert!(n >= 3, "need at least 3 segments");

    // Last two indices should be (n-1, 0) closing the loop
    let last_pair = &geo.indices[geo.indices.len() - 2..];
    assert_eq!(last_pair[0], n - 1, "last edge starts at last vertex");
    assert_eq!(last_pair[1], 0, "last edge ends at first vertex (closed)");
}

#[test]
fn circle_outline_all_positions_approximately_equidistant_from_center() {
    // 所有环上位置到中心的测地距离应大致相同
    let ell = wgs84();
    let center = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(10.0, 20.0, 0.0),
    );
    let radius = 200_000.0;
    let geo = circle_outline_geometry(center, radius, &ell, std::f64::consts::PI / 36.0);

    // 计算到中心的欧氏距离 - 应大致相等
    let distances: Vec<f64> = geo
        .positions
        .iter()
        .map(|p| {
            let pos = DVec3::new(p[0], p[1], p[2]);
            (pos - center).length()
        })
        .collect();

    let mean_dist = distances.iter().sum::<f64>() / distances.len() as f64;
    for (i, &d) in distances.iter().enumerate() {
        let deviation = (d - mean_dist).abs() / mean_dist;
        assert!(
            deviation < 0.15,
            "position[{}] distance deviation {:.3} exceeds 15%",
            i,
            deviation
        );
    }
}

#[test]
fn circle_outline_granularity_controls_segment_count() {
    let ell = wgs84();
    let center = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let radius = 500_000.0;

    let coarse = circle_outline_geometry(center, radius, &ell, std::f64::consts::PI / 6.0);
    let fine = circle_outline_geometry(center, radius, &ell, std::f64::consts::PI / 60.0);

    assert!(
        fine.positions.len() > coarse.positions.len(),
        "finer granularity ({}) should produce more vertices than coarser ({})",
        fine.positions.len(),
        coarse.positions.len()
    );
}

#[test]
fn circle_outline_indices_all_valid() {
    let ell = wgs84();
    let center = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(-45.0, 30.0, 0.0),
    );
    let geo = circle_outline_geometry(center, 300_000.0, &ell, std::f64::consts::PI / 18.0);

    let max_idx = geo.positions.len() as u32;
    for (i, &idx) in geo.indices.iter().enumerate() {
        assert!(
            idx < max_idx,
            "index[{}] = {} out of range (max {})",
            i,
            idx,
            max_idx
        );
    }
    // 索引应构成连续的成对：(0,1), (1,2), ..., (n-2,n-1), (n-1,0)
    let n = geo.positions.len() as u32;
    assert_eq!(geo.indices.len(), n as usize * 2);
}

#[test]
fn circle_outline_positions_on_ellipsoid_surface() {
    let ell = wgs84();
    let center = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 45.0, 0.0),
    );
    let geo = circle_outline_geometry(center, 100_000.0, &ell, std::f64::consts::PI / 36.0);

    for (i, p) in geo.positions.iter().enumerate() {
        let pos = DVec3::new(p[0], p[1], p[2]);
        let surface = ell.scale_to_geodetic_surface(pos).unwrap_or(pos);
        let dist = (pos - surface).length();
        assert!(
            dist < 1.0,
            "position[{}] should be on ellipsoid surface, dist={}",
            i,
            dist
        );
    }
}

// ===========================================================================
// FrustumGeometry - 扩展的数学属性
// ===========================================================================

#[test]
fn frustum_geometry_24_vertices_36_indices() {
    // CesiumJS: 6 planes × 4 vertices = 24, 6 planes × 2 triangles × 3 = 36
    let geo = frustum_geometry(
        &perspective_frustum(),
        DVec3::ZERO,
        DQuat::IDENTITY,
        VertexFormat::ALL,
    );
    assert_eq!(geo.positions.len(), 24, "should have 24 vertices (6 planes × 4)");
    assert_eq!(geo.indices.len(), 36, "should have 36 indices (6 planes × 6)");
}

#[test]
fn frustum_geometry_normals_are_unit_vectors() {
    let geo = frustum_geometry(
        &perspective_frustum(),
        DVec3::ZERO,
        DQuat::IDENTITY,
        VertexFormat::POSITION_AND_NORMAL,
    );
    let normals = geo.normals.as_ref().expect("normals should be present");

    for (i, n) in normals.iter().enumerate() {
        let nrm = DVec3::new(n[0], n[1], n[2]);
        assert!(
            (nrm.length() - 1.0).abs() < 1e-10,
            "normal[{}] should be unit length, got {}",
            i,
            nrm.length()
        );
    }
}

#[test]
fn frustum_geometry_tangent_perpendicular_to_normal() {
    let geo = frustum_geometry(
        &perspective_frustum(),
        DVec3::ZERO,
        DQuat::IDENTITY,
        VertexFormat::ALL,
    );
    let normals = geo.normals.as_ref().expect("normals");
    let tangents = geo.tangents.as_ref().expect("tangents");

    for i in 0..normals.len() {
        let n = DVec3::new(normals[i][0], normals[i][1], normals[i][2]);
        let t = DVec3::new(tangents[i][0], tangents[i][1], tangents[i][2]);
        let dot = n.dot(t);
        assert!(
            dot.abs() < 1e-10,
            "normal[{}] · tangent[{}] should be 0, got {}",
            i,
            i,
            dot
        );
    }
}

#[test]
fn frustum_geometry_tex_coords_in_unit_range() {
    let geo = frustum_geometry(
        &perspective_frustum(),
        DVec3::ZERO,
        DQuat::IDENTITY,
        VertexFormat::POSITION_AND_ST,
    );
    let st = geo.tex_coords.as_ref().expect("tex_coords should be present");

    for (i, uv) in st.iter().enumerate() {
        assert!(
            uv[0] >= -1e-10 && uv[0] <= 1.0 + 1e-10,
            "st[{}].u out of [0,1]: {}",
            i,
            uv[0]
        );
        assert!(
            uv[1] >= -1e-10 && uv[1] <= 1.0 + 1e-10,
            "st[{}].v out of [0,1]: {}",
            i,
            uv[1]
        );
    }
}

#[test]
fn frustum_geometry_near_plane_closer_than_far() {
    // 对于位于原点、沿 -Z 方向观察的透视视锥（单位方位），
    // 近平面顶点应比远平面顶点更靠近原点。
    let geo = frustum_geometry(
        &perspective_frustum(),
        DVec3::ZERO,
        DQuat::IDENTITY,
        VertexFormat::POSITION_ONLY,
    );

    // 前 4 个顶点 = 近平面，接下来 4 个 = 远平面
    let near_dists: Vec<f64> = geo.positions[0..4]
        .iter()
        .map(|p| DVec3::new(p[0], p[1], p[2]).length())
        .collect();
    let far_dists: Vec<f64> = geo.positions[4..8]
        .iter()
        .map(|p| DVec3::new(p[0], p[1], p[2]).length())
        .collect();

    let near_avg = near_dists.iter().sum::<f64>() / 4.0;
    let far_avg = far_dists.iter().sum::<f64>() / 4.0;
    assert!(
        near_avg < far_avg,
        "near plane avg dist ({}) should be less than far ({})",
        near_avg,
        far_avg
    );
}

#[test]
fn frustum_geometry_orthographic_24_vertices() {
    let geo = frustum_geometry(
        &orthographic_frustum(),
        DVec3::ZERO,
        DQuat::IDENTITY,
        VertexFormat::ALL,
    );
    assert_eq!(geo.positions.len(), 24);
    assert_eq!(geo.indices.len(), 36);
    assert!(geo.normals.is_some());
    assert!(geo.tangents.is_some());
    assert!(geo.tex_coords.is_some());
}

#[test]
fn frustum_outline_8_positions_24_indices() {
    // CesiumJS: 8 corners, 12 edges × 2 = 24 indices
    let geo = frustum_outline_geometry(&perspective_frustum(), DVec3::ZERO, DQuat::IDENTITY);
    assert_eq!(geo.positions.len(), 8, "frustum outline should have 8 corners");
    assert_eq!(geo.indices.len(), 24, "frustum outline should have 24 indices (12 edges)");
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn frustum_outline_orthographic_correct_structure() {
    let geo = frustum_outline_geometry(&orthographic_frustum(), DVec3::ZERO, DQuat::IDENTITY);
    assert_eq!(geo.positions.len(), 8);
    assert_eq!(geo.indices.len(), 24);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);

    // 所有索引有效
    for &idx in &geo.indices {
        assert!(idx < 8, "index {} out of range", idx);
    }
}

#[test]
fn frustum_geometry_bounding_sphere_contains_all_positions() {
    let geo = frustum_geometry(
        &perspective_frustum(),
        DVec3::new(100.0, 200.0, 300.0),
        DQuat::IDENTITY,
        VertexFormat::POSITION_ONLY,
    );

    let bs = &geo.bounding_sphere;
    for (i, p) in geo.positions.iter().enumerate() {
        let pos = DVec3::new(p[0], p[1], p[2]);
        let dist = (pos - bs.center).length();
        assert!(
            dist <= bs.radius + 1e-6,
            "position[{}] outside bounding sphere: dist={} > radius={}",
            i,
            dist,
            bs.radius
        );
    }
}

// ===========================================================================
// GroundPolylineGeometry - 扩展的数学属性
// ===========================================================================

#[test]
fn ground_polyline_closed_produces_more_vertices() {
    let ell = wgs84();
    let p1 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let p2 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(1.0, 0.0, 0.0),
    );
    let p3 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(1.0, 1.0, 0.0),
    );

    let open_opts = GroundPolylineOptions {
        positions: vec![p1, p2, p3],
        width: 10.0,
        granularity: std::f64::consts::PI / 180.0,
        closed: false,
        ellipsoid: ell,
    };
    let closed_opts = GroundPolylineOptions {
        positions: vec![p1, p2, p3],
        width: 10.0,
        granularity: std::f64::consts::PI / 180.0,
        closed: true,
        ellipsoid: ell,
    };

    let open_geo = ground_polyline_geometry(&open_opts, VertexFormat::POSITION_ONLY);
    let closed_geo = ground_polyline_geometry(&closed_opts, VertexFormat::POSITION_ONLY);

    assert!(
        closed_geo.positions.len() > open_geo.positions.len(),
        "closed ({}) should have more vertices than open ({})",
        closed_geo.positions.len(),
        open_geo.positions.len()
    );
}

#[test]
fn ground_polyline_tex_coords_s_monotonic() {
    let ell = wgs84();
    let p1 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let p2 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(2.0, 0.0, 0.0),
    );

    let opts = GroundPolylineOptions {
        positions: vec![p1, p2],
        width: 5.0,
        granularity: std::f64::consts::PI / 180.0,
        closed: false,
        ellipsoid: ell,
    };
    let geo = ground_polyline_geometry(&opts, VertexFormat::POSITION_AND_ST);
    let st = geo.tex_coords.as_ref().expect("tex_coords should be present");

    // S 坐标应非递减（成对：每个站点为 right,left）
    let n = st.len() / 2;
    for i in 1..n {
        let s_prev = st[(i - 1) * 2][0];
        let s_curr = st[i * 2][0];
        assert!(
            s_curr >= s_prev - 1e-10,
            "s[{}] ({}) should be >= s[{}] ({})",
            i,
            s_curr,
            i - 1,
            s_prev
        );
    }
    // 第一个 s 应为 0，最后一个应为 1
    assert!(st[0][0].abs() < 1e-10, "first s should be 0");
    assert!((st[st.len() - 2][0] - 1.0).abs() < 1e-10, "last s should be 1");
}

#[test]
fn ground_polyline_ribbon_width_approximately_correct() {
    let ell = wgs84();
    let p1 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let p2 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(1.0, 0.0, 0.0),
    );
    let width = 1000.0;

    let opts = GroundPolylineOptions {
        positions: vec![p1, p2],
        width,
        granularity: std::f64::consts::PI / 180.0,
        closed: false,
        ellipsoid: ell,
    };
    let geo = ground_polyline_geometry(&opts, VertexFormat::POSITION_ONLY);

    // 每对顶点（right, left）应相距约 `width`
    let n_pairs = geo.positions.len() / 2;
    for i in 0..n_pairs {
        let right = DVec3::new(
            geo.positions[i * 2][0],
            geo.positions[i * 2][1],
            geo.positions[i * 2][2],
        );
        let left = DVec3::new(
            geo.positions[i * 2 + 1][0],
            geo.positions[i * 2 + 1][1],
            geo.positions[i * 2 + 1][2],
        );
        let dist = (left - right).length();
        assert!(
            (dist - width).abs() < width * 0.01,
            "ribbon width at station {} should be ~{}, got {}",
            i,
            width,
            dist
        );
    }
}

#[test]
fn ground_polyline_normals_point_outward() {
    let ell = wgs84();
    let p1 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let p2 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(1.0, 0.0, 0.0),
    );

    let opts = GroundPolylineOptions {
        positions: vec![p1, p2],
        width: 10.0,
        granularity: std::f64::consts::PI / 180.0,
        closed: false,
        ellipsoid: ell,
    };
    let geo = ground_polyline_geometry(&opts, VertexFormat::POSITION_AND_NORMAL);
    let normals = geo.normals.as_ref().expect("normals should be present");

    // 每条法线应背离椭球中心（与位置的点积 > 0）
    for (i, (p, n)) in geo.positions.iter().zip(normals.iter()).enumerate() {
        let pos = DVec3::new(p[0], p[1], p[2]);
        let nrm = DVec3::new(n[0], n[1], n[2]);
        let dot = pos.normalize().dot(nrm);
        assert!(
            dot > 0.9,
            "normal[{}] should point outward (dot={})",
            i,
            dot
        );
    }
}

#[test]
fn ground_polyline_indices_form_valid_triangles() {
    let ell = wgs84();
    let p1 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let p2 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(2.0, 0.0, 0.0),
    );

    let opts = GroundPolylineOptions {
        positions: vec![p1, p2],
        width: 5.0,
        granularity: std::f64::consts::PI / 180.0,
        closed: false,
        ellipsoid: ell,
    };
    let geo = ground_polyline_geometry(&opts, VertexFormat::POSITION_ONLY);

    assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
    assert_eq!(geo.indices.len() % 3, 0, "indices must form triangles");

    let max_idx = geo.positions.len() as u32;
    for (i, &idx) in geo.indices.iter().enumerate() {
        assert!(idx < max_idx, "index[{}] = {} out of range", i, idx);
    }
}

#[test]
fn ground_polyline_empty_for_single_position() {
    let ell = wgs84();
    let p1 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );

    let opts = GroundPolylineOptions {
        positions: vec![p1],
        width: 5.0,
        granularity: std::f64::consts::PI / 180.0,
        closed: false,
        ellipsoid: ell,
    };
    let geo = ground_polyline_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(geo.positions.is_empty(), "single position should produce empty geometry");
}

#[test]
fn ground_polyline_zero_width_empty() {
    let ell = wgs84();
    let p1 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0),
    );
    let p2 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(1.0, 0.0, 0.0),
    );

    let opts = GroundPolylineOptions {
        positions: vec![p1, p2],
        width: 0.0,
        granularity: std::f64::consts::PI / 180.0,
        closed: false,
        ellipsoid: ell,
    };
    let geo = ground_polyline_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(geo.positions.is_empty(), "zero width should produce empty geometry");
}

#[test]
fn ground_polyline_bounding_sphere_contains_all() {
    let ell = wgs84();
    let p1 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(-10.0, -10.0, 0.0),
    );
    let p2 = ell.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(10.0, 10.0, 0.0),
    );

    let opts = GroundPolylineOptions {
        positions: vec![p1, p2],
        width: 100.0,
        granularity: std::f64::consts::PI / 180.0,
        closed: false,
        ellipsoid: ell,
    };
    let geo = ground_polyline_geometry(&opts, VertexFormat::POSITION_ONLY);

    let bs = &geo.bounding_sphere;
    for (i, p) in geo.positions.iter().enumerate() {
        let pos = DVec3::new(p[0], p[1], p[2]);
        let dist = (pos - bs.center).length();
        assert!(
            dist <= bs.radius + 1.0,
            "position[{}] outside bounding sphere",
            i
        );
    }
}
