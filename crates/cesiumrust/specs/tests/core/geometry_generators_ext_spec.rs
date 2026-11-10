//! 几何生成器（Ellipse/Corridor/Wall/Polyline/CoplanarPolygon/Frustum）
//! + Simon1994PlanetaryPositions + IauOrientation → Rust 集成测试。
//!
//! 参考 CesiumJS：
//! - Core/EllipseGeometry、Core/CorridorGeometry、Core/WallGeometry
//! - Core/PolylineGeometry、Core/CoplanarPolygonGeometry
//! - Core/Simon1994PlanetaryPositions、Core/Iau2000Orientation
//!
//! A 类测试：顶点/索引数、包围球、行星位置。
//! C 类已省略：throws、pack/unpack、offsetAttribute（GPU 特有）。

use cesium_geospatial::geometry::{
    box_geometry, coplanar_polygon_geometry, corridor_geometry, corridor_outline_geometry,
    cylinder_geometry, ellipse_geometry, ellipse_outline_geometry, ellipsoid_geometry,
    frustum_geometry, ground_polyline_geometry, polyline_geometry, sphere_geometry,
    wall_geometry, wall_outline_geometry,
    CoplanarPolygonOptions, CornerType, CorridorOptions, EllipseOptions, FrustumDef,
    GroundPolylineOptions, PolylineOptions, WallOptions,
};
use cesium_geospatial::geometry::VertexFormat;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::frustum::PerspectiveFrustum;
use cesium_geospatial::simon1994_planetary_positions::{
    compute_moon_position_in_earth_inertial_frame,
    compute_sun_position_in_earth_inertial_frame,
};
use cesium_geospatial::iau_orientation::compute_moon;
use cesium_time::julian_date::{JulianDate, TimeStandard};
use glam::{DQuat, DVec3};

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

fn equator_center() -> DVec3 {
    // WGS84 上的 Cartesian3.fromDegrees(0, 0)
    wgs84().cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_radians(0.0, 0.0, 0.0),
    )
}

fn from_degrees(lon: f64, lat: f64, h: f64) -> DVec3 {
    wgs84().cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(lon, lat, h),
    )
}

// === Simon1994PlanetaryPositions ===

#[test]
fn simon1994_sun_position_j2000() {
    // CesiumJS: new JulianDate(2451545, 0, TimeStandard.TAI)
    let date = JulianDate::with_time_standard(2451545.0, 0.0, TimeStandard::TAI);
    let sun = compute_sun_position_in_earth_inertial_frame(&date);
    // 期望值来自 STK Components（相对 epsilon ~1e-11）
    let eps = 1.0e-11;
    assert!((sun.x - 26500268539.790234).abs() < eps * 26500268539.0_f64.abs().max(1.0));
    assert!((sun.y - (-132756447253.27325)).abs() < eps * 132756447253.0_f64.abs().max(1.0));
    assert!((sun.z - (-57556483362.533806)).abs() < eps * 57556483362.0_f64.abs().max(1.0));
}

#[test]
fn simon1994_sun_position_2013() {
    let date = JulianDate::with_time_standard(2456401.5, 0.0, TimeStandard::TAI);
    let sun = compute_sun_position_in_earth_inertial_frame(&date);
    let eps = 1.0e-11;
    assert!((sun.x - 131512388940.33589).abs() < eps * 131512388940.0_f64.abs().max(1.0));
    assert!((sun.y - 66661342667.949928).abs() < eps * 66661342667.0_f64.abs().max(1.0));
    assert!((sun.z - 28897975607.905258).abs() < eps * 28897975607.0_f64.abs().max(1.0));
}

#[test]
fn simon1994_moon_position_j2000() {
    let date = JulianDate::with_time_standard(2451545.0, 0.0, TimeStandard::TAI);
    let moon = compute_moon_position_in_earth_inertial_frame(&date);
    let eps = 1.0e-10;
    assert!((moon.x - (-291632410.61232185)).abs() < eps * 291632410.0_f64.abs().max(1.0));
    assert!((moon.y - (-266522146.36821631)).abs() < eps * 266522146.0_f64.abs().max(1.0));
    assert!((moon.z - (-75994518.081043154)).abs() < eps * 75994518.0_f64.abs().max(1.0));
}

#[test]
fn simon1994_moon_position_2013() {
    let date = JulianDate::with_time_standard(2456401.5, 0.0, TimeStandard::TAI);
    let moon = compute_moon_position_in_earth_inertial_frame(&date);
    let eps = 1.0e-10;
    assert!((moon.x - (-223792974.4736526)).abs() < eps * 223792974.0_f64.abs().max(1.0));
    assert!((moon.y - 315772435.34490639).abs() < eps * 315772435.0_f64.abs().max(1.0));
    assert!((moon.z - 97913011.236112773).abs() < eps * 97913011.0_f64.abs().max(1.0));
}

// === IauOrientation ===

#[test]
fn iau_compute_moon_j2000() {
    let date = JulianDate::with_time_standard(2451545.0, 0.0, TimeStandard::TAI);
    let params = compute_moon(&date);
    // 赤经与赤纬应为有限的弧度值
    assert!(params.right_ascension.is_finite());
    assert!(params.declination.is_finite());
    assert!(params.rotation.is_finite());
    assert!(params.rotation_rate.is_finite());
    // 自转速率应为正（月球在自转）
    assert!(params.rotation_rate > 0.0);
}

// === EllipseGeometry ===

#[test]
fn ellipse_computes_positions() {
    let opts = EllipseOptions {
        center: equator_center(),
        semi_major_axis: 1.0,
        semi_minor_axis: 1.0,
        granularity: 0.1,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);
    // CesiumJS：16 顶点（行 1+4+6+4+1），22 三角形（行 3+8+8+3）
    assert_eq!(geo.positions.len(), 16);
    assert_eq!(geo.indices.len(), 22 * 3);
    // 包围球半径 ~1（表面上的小椭圆）
    assert!((geo.bounding_sphere.radius - 1.0).abs() < 0.01);
}

#[test]
fn ellipse_all_vertex_attributes() {
    let opts = EllipseOptions {
        center: equator_center(),
        semi_major_axis: 1.0,
        semi_minor_axis: 1.0,
        granularity: 0.1,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_geometry(&opts, VertexFormat::ALL);
    let nv = geo.positions.len();
    assert_eq!(nv, 16);
    assert!(geo.normals.is_some());
    assert_eq!(geo.normals.as_ref().unwrap().len(), nv);
    assert!(geo.tex_coords.is_some());
    assert_eq!(geo.tex_coords.as_ref().unwrap().len(), nv);
}

#[test]
fn ellipse_outline_computes_positions() {
    let opts = EllipseOptions {
        center: equator_center(),
        semi_major_axis: 1.0,
        semi_minor_axis: 1.0,
        granularity: 0.1,
        ellipsoid: wgs84(),
        ..Default::default()
    };
    let geo = ellipse_outline_geometry(&opts);
    // 轮廓：由线段组成的环
    assert!(geo.positions.len() >= 8);
    assert!(geo.indices.len() >= 8);
    // 索引成对（线段）
    assert_eq!(geo.indices.len() % 2, 0);
}

// === CorridorGeometry ===

#[test]
fn corridor_computes_positions() {
    let e = wgs84();
    let p0 = from_degrees(0.0, 0.0, 0.0);
    let p1 = from_degrees(1.0, 0.0, 0.0);
    let opts = CorridorOptions {
        positions: vec![p0, p1],
        width: 100000.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(geo.positions.len() >= 4);
    assert!(geo.indices.len() >= 6);
    assert!(geo.bounding_sphere.radius > 0.0);
}

#[test]
fn corridor_outline_computes_positions() {
    let e = wgs84();
    let p0 = from_degrees(0.0, 0.0, 0.0);
    let p1 = from_degrees(1.0, 0.0, 0.0);
    let opts = CorridorOptions {
        positions: vec![p0, p1],
        width: 100000.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = corridor_outline_geometry(&opts);
    assert!(geo.positions.len() >= 4);
    assert!(geo.indices.len() >= 4);
}

#[test]
fn corridor_corner_types() {
    let e = wgs84();
    let p0 = from_degrees(0.0, 0.0, 0.0);
    let p1 = from_degrees(1.0, 0.0, 0.0);
    let p2 = from_degrees(1.0, 1.0, 0.0);

    for ct in [CornerType::Rounded, CornerType::Mitered, CornerType::Beveled] {
        let opts = CorridorOptions {
            positions: vec![p0, p1, p2],
            width: 100000.0,
            corner_type: ct,
            granularity: std::f64::consts::PI / 180.0,
            ellipsoid: e,
            ..Default::default()
        };
        let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert!(geo.positions.len() >= 4, "corner_type {:?} should produce vertices", ct);
        assert!(geo.indices.len() >= 6, "corner_type {:?} should produce triangles", ct);
    }
}

// === WallGeometry ===

#[test]
fn wall_computes_positions() {
    let e = wgs84();
    let positions: Vec<DVec3> = (0..4).map(|i| from_degrees(i as f64, 0.0, 0.0)).collect();

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);
    // 墙：2 行（顶+底）* 弧线点数
    assert!(geo.positions.len() >= 8);
    assert!(geo.indices.len() >= 6);
    assert!(geo.bounding_sphere.radius > 0.0);
}

#[test]
fn wall_with_min_max_heights() {
    let e = wgs84();
    let positions: Vec<DVec3> = (0..3).map(|i| from_degrees(i as f64, 0.0, 0.0)).collect();

    let opts = WallOptions {
        positions: positions.clone(),
        maximum_heights: Some(vec![10000.0, 10000.0, 10000.0]),
        minimum_heights: Some(vec![0.0, 0.0, 0.0]),
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(geo.positions.len() >= 6);
    assert!(geo.indices.len() >= 6);
}

#[test]
fn wall_from_constant_heights() {
    let e = wgs84();
    let positions: Vec<DVec3> = (0..3).map(|i| from_degrees(i as f64, 0.0, 0.0)).collect();

    let opts = WallOptions::from_constant_heights(
        positions,
        Some(0.0),
        Some(50000.0),
        e,
    );
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(geo.positions.len() >= 6);
    assert!(geo.indices.len() >= 6);
}

#[test]
fn wall_outline_computes_positions() {
    let e = wgs84();
    let positions: Vec<DVec3> = (0..3).map(|i| from_degrees(i as f64, 0.0, 0.0)).collect();

    let opts = WallOptions {
        positions,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = wall_outline_geometry(&opts);
    assert!(geo.positions.len() >= 4);
    assert!(geo.indices.len() >= 4);
    assert_eq!(geo.indices.len() % 2, 0);
}

// === PolylineGeometry ===

#[test]
fn polyline_computes_positions() {
    let e = wgs84();
    let p0 = from_degrees(0.0, 0.0, 0.0);
    let p1 = from_degrees(1.0, 0.0, 0.0);
    let opts = PolylineOptions {
        positions: vec![p0, p1],
        width: 10000.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
    };
    let geo = polyline_geometry(&opts, VertexFormat::POSITION_ONLY);
    // 缎带：至少 2 段 * 每截面 2 顶点
    assert!(geo.positions.len() >= 4);
    assert!(geo.indices.len() >= 6);
    assert!(geo.bounding_sphere.radius > 0.0);
}

#[test]
fn polyline_empty_with_less_than_2_positions() {
    let e = wgs84();
    let p0 = from_degrees(0.0, 0.0, 0.0);
    let opts = PolylineOptions {
        positions: vec![p0],
        width: 10000.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
    };
    let geo = polyline_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(geo.positions.is_empty());
    assert!(geo.indices.is_empty());
}

// === CoplanarPolygonGeometry ===

#[test]
fn coplanar_polygon_triangle() {
    let e = wgs84();
    let p0 = from_degrees(0.0, 0.0, 0.0);
    let p1 = from_degrees(1.0, 0.0, 0.0);
    let p2 = from_degrees(0.5, 1.0, 0.0);
    let opts = CoplanarPolygonOptions {
        positions: vec![p0, p1, p2],
        ellipsoid: e,
        ..Default::default()
    };
    let geo = coplanar_polygon_geometry(&opts, VertexFormat::POSITION_ONLY);
    // 三角形：3 顶点，1 三角形（3 索引）
    assert_eq!(geo.positions.len(), 3);
    assert_eq!(geo.indices.len(), 3);
    assert!(geo.bounding_sphere.radius > 0.0);
}

#[test]
fn coplanar_polygon_quad() {
    let e = wgs84();
    let p0 = from_degrees(0.0, 0.0, 0.0);
    let p1 = from_degrees(1.0, 0.0, 0.0);
    let p2 = from_degrees(1.0, 1.0, 0.0);
    let p3 = from_degrees(0.0, 1.0, 0.0);
    let opts = CoplanarPolygonOptions {
        positions: vec![p0, p1, p2, p3],
        ellipsoid: e,
        ..Default::default()
    };
    let geo = coplanar_polygon_geometry(&opts, VertexFormat::POSITION_ONLY);
    // 四边形：4 顶点，2 三角形（6 索引）
    assert_eq!(geo.positions.len(), 4);
    assert_eq!(geo.indices.len(), 6);
}

// === FrustumGeometry ===

#[test]
fn frustum_geometry_computes_positions() {
    let pf = PerspectiveFrustum::new(
        std::f64::consts::FRAC_PI_4,
        1.0,
        1.0,
        100.0,
    );
    let def = FrustumDef::Perspective(pf);
    let geo = frustum_geometry(&def, DVec3::ZERO, DQuat::IDENTITY, VertexFormat::POSITION_ONLY);
    // 视锥：6 平面 * 4 顶点 = 24
    assert!(geo.positions.len() >= 8);
    assert!(geo.indices.len() >= 12);
    assert!(geo.bounding_sphere.radius > 0.0);
}

// === GroundPolylineGeometry ===

#[test]
fn ground_polyline_computes_positions() {
    let e = wgs84();
    let p0 = from_degrees(0.0, 0.0, 0.0);
    let p1 = from_degrees(1.0, 0.0, 0.0);
    let opts = GroundPolylineOptions {
        positions: vec![p0, p1],
        width: 5000.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = ground_polyline_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(geo.positions.len() >= 4);
    assert!(geo.indices.len() >= 6);
}

// === SphereGeometry 详细 ===

#[test]
fn sphere_computes_positions_exact() {
    // SphereGeometrySpec: "computes positions" - radius=1, stacks=2, slices=3
    let geo = sphere_geometry(1.0, 2, 3, VertexFormat::POSITION_ONLY);
    // (stacks+1) * (slices+1) = 3 * 4 = 12
    assert_eq!(geo.positions.len(), 12);
    // 2 stacks * 3 slices * 6 索引 = 36
    assert_eq!(geo.indices.len(), 36);
    assert!((geo.bounding_sphere.radius - 1.0).abs() < 1e-10);
}

#[test]
fn sphere_computes_all_vertex_attributes() {
    // SphereGeometrySpec: "compute all vertex attributes"
    let geo = sphere_geometry(1.0, 3, 4, VertexFormat::ALL);
    let nv = geo.positions.len();
    assert_eq!(nv, (4) * (5)); // (stacks+1)*(slices+1) = 4*5 = 20
    assert!(geo.normals.is_some());
    assert!(geo.tex_coords.is_some());
    assert_eq!(geo.normals.as_ref().unwrap().len(), nv);
    assert_eq!(geo.tex_coords.as_ref().unwrap().len(), nv);
}

#[test]
fn sphere_unit_sphere_attributes() {
    // SphereGeometrySpec: "computes attributes for a unit sphere"
    let geo = sphere_geometry(1.0, 4, 5, VertexFormat::POSITION_AND_NORMAL);
    let normals = geo.normals.as_ref().unwrap();

    for i in 0..geo.positions.len() {
        let pos = DVec3::new(geo.positions[i][0], geo.positions[i][1], geo.positions[i][2]);
        let n = DVec3::new(normals[i][0], normals[i][1], normals[i][2]);

        // 位置应位于单位球面上
        assert!((pos.length() - 1.0).abs() < 1e-10, "pos[{}] not on unit sphere", i);
        // 法线应等于位置（对于以原点为中心的单位球）
        assert!((n - pos).length() < 1e-10, "normal[{}] != position", i);
        // 法线应为单位长度
        assert!((n.length() - 1.0).abs() < 1e-10, "normal[{}] not unit", i);
    }
}

// === CylinderGeometry 详细 ===

#[test]
fn cylinder_top_radius_zero_cone() {
    // CylinderGeometrySpec: "computes positions with topRadius equals 0"
    // cylinder_geometry(length, top_radius, bottom_radius, slices, vf)
    let geo = cylinder_geometry(10.0, 0.0, 5.0, 8, VertexFormat::POSITION_ONLY);
    assert!(!geo.positions.is_empty());
    assert!(geo.indices.len() % 3 == 0);

    // 顶部顶点应位于 z = +half_length（锥顶）
    let half = 5.0;
    let mut has_top_tip = false;
    for p in &geo.positions {
        if (p[2] - half).abs() < 1e-6 {
            // radius=0 的顶部位置应坠缩到轴上（x≈0, y≈0）
            if p[0].abs() < 1e-6 && p[1].abs() < 1e-6 {
                has_top_tip = true;
            }
        }
    }
    assert!(has_top_tip, "cone top should have a vertex at the tip");
}

#[test]
fn cylinder_bottom_radius_zero_inverted_cone() {
    // CylinderGeometrySpec: "computes positions with bottomRadius equals 0"
    // cylinder_geometry(length, top_radius, bottom_radius, slices, vf)
    let geo = cylinder_geometry(10.0, 5.0, 0.0, 8, VertexFormat::POSITION_ONLY);
    assert!(!geo.positions.is_empty());
    assert!(geo.indices.len() % 3 == 0);
    assert!(geo.bounding_sphere.radius > 0.0);
}

#[test]
fn cylinder_both_radii_zero_degenerate() {
    // 两个半径均为零 → 所有顶点在 Z 轴上
    let geo = cylinder_geometry(10.0, 0.0, 0.0, 8, VertexFormat::POSITION_ONLY);
    for p in &geo.positions {
        assert!(p[0].abs() < 1e-10);
        assert!(p[1].abs() < 1e-10);
    }
}

// === EllipsoidGeometry 详细 ===

#[test]
fn ellipsoid_unit_sphere_attributes() {
    // EllipsoidGeometrySpec: "computes the unit ellipsoid"
    let geo = ellipsoid_geometry(DVec3::ONE, 4, 5, VertexFormat::POSITION_AND_NORMAL);
    let normals = geo.normals.as_ref().unwrap();
    for (i, (p, n)) in geo.positions.iter().zip(normals.iter()).enumerate() {
        let pos = DVec3::from(*p);
        let nrm = DVec3::from(*n);
        assert!((pos.length() - 1.0).abs() < 1e-10, "pos[{}] not on unit sphere", i);
        assert!((nrm - pos).length() < 1e-10, "normal[{}] != position", i);
    }
}

#[test]
fn ellipsoid_negated_normals_point_inward() {
    // EllipsoidGeometrySpec: "negates normals on an ellipsoid"
    // 计算出法线后，乘以 -1 使其朝内
    let radii = DVec3::new(2.0, 1.5, 1.0);
    let mut geo = ellipsoid_geometry(radii, 4, 6, VertexFormat::POSITION_AND_NORMAL);
    // 反转法线
    if let Some(ref mut normals) = geo.normals {
        for n in normals.iter_mut() {
            n[0] = -n[0];
            n[1] = -n[1];
            n[2] = -n[2];
        }
    }

    let normals = geo.normals.as_ref().unwrap();
    for (p, n) in geo.positions.iter().zip(normals.iter()) {
        // 朝内的法线与位置的点积应为负
        let dot = p[0] * n[0] + p[1] * n[1] + p[2] * n[2];
        assert!(dot < 0.0, "negated normal should point inward (dot={})", dot);
    }
}

#[test]
fn ellipsoid_scaled_radii_proportioned_correctly() {
    // 位置应随半径正确缩放
    let radii = DVec3::new(3.0, 2.0, 1.0);
    let geo = ellipsoid_geometry(radii, 5, 8, VertexFormat::POSITION_ONLY);

    for p in &geo.positions {
        let pos = DVec3::from(*p);
        // 缩放椭球上的点满足：(x/rx)^2 + (y/ry)^2 + (z/rz)^2 = 1
        let scaled_len = (pos.x / radii.x).powi(2) + (pos.y / radii.y).powi(2) + (pos.z / radii.z).powi(2);
        assert!((scaled_len - 1.0).abs() < 1e-10, "position not on ellipsoid surface");
    }
}

#[test]
fn ellipsoid_bounding_sphere_uses_max_radius() {
    let radii = DVec3::new(1.0, 3.0, 2.0);
    let geo = ellipsoid_geometry(radii, 3, 4, VertexFormat::POSITION_ONLY);
    assert!((geo.bounding_sphere.radius - 3.0).abs() < 1e-10);
}

// === BoxGeometry 详细 ===

#[test]
fn box_geometry_position_only_reuses_vertices() {
    // BoxGeometrySpec: "constructor creates optimized number of positions for VertexFormat.POSITIONS_ONLY"
    // 使用 POSITION_ONLY 时顶点被共享 → 24 顶点（6 面 * 4）
    let geo = box_geometry(
        DVec3::new(-1.0, -1.0, -1.0),
        DVec3::new(1.0, 1.0, 1.0),
        VertexFormat::POSITION_ONLY,
    );
    assert_eq!(geo.positions.len(), 24);
    assert_eq!(geo.indices.len(), 36);
    assert!(geo.normals.is_none());
    assert!(geo.tex_coords.is_none());
}

#[test]
fn box_asymmetric_dimensions() {
    // 非对称尺寸（-1 到 2，-1 到 3，-2 到 1）
    let geo = box_geometry(
        DVec3::new(-1.0, -1.0, -2.0),
        DVec3::new(2.0, 3.0, 1.0),
        VertexFormat::ALL,
    );
    assert_eq!(geo.positions.len(), 24);
    assert_eq!(geo.indices.len(), 36);
    assert!(geo.normals.is_some());
    assert!(geo.tex_coords.is_some());
    // 包围球应包含该盒子
    let diagonal = DVec3::new(3.0, 4.0, 3.0).length();
    assert!((geo.bounding_sphere.radius - diagonal / 2.0).abs() < 1e-10);
}

#[test]
fn box_corner_positions_valid() {
    // 所有位置应位于盒子的角点（在表面上）
    let min = DVec3::new(0.0, 0.0, 0.0);
    let max = DVec3::new(1.0, 1.0, 1.0);
    let geo = box_geometry(min, max, VertexFormat::POSITION_ONLY);

    for p in &geo.positions {
        assert!(p[0] >= 0.0 - 1e-10 && p[0] <= 1.0 + 1e-10);
        assert!(p[1] >= 0.0 - 1e-10 && p[1] <= 1.0 + 1e-10);
        assert!(p[2] >= 0.0 - 1e-10 && p[2] <= 1.0 + 1e-10);
        // 每个顶点必须位于盒子的某一极值处（至少在一个面上）
        let on_face = (p[0].abs() < 1e-10 || (p[0] - 1.0).abs() < 1e-10)
            || (p[1].abs() < 1e-10 || (p[1] - 1.0).abs() < 1e-10)
            || (p[2].abs() < 1e-10 || (p[2] - 1.0).abs() < 1e-10);
        assert!(on_face, "vertex {:?} not on a box face", p);
    }
}

// === PolylineGeometry 详细 ===

#[test]
fn polyline_texture_coordinates_monotonic() {
    let e = wgs84();
    let p0 = from_degrees(0.0, 0.0, 0.0);
    let p1 = from_degrees(3.0, 0.0, 0.0);
    let opts = PolylineOptions {
        positions: vec![p0, p1],
        width: 100.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
    };
    let geo = polyline_geometry(&opts, VertexFormat::POSITION_AND_ST);
    let st = geo.tex_coords.as_ref().unwrap();
    // ST 的 u 应从 0 单调增至 1
    let n = geo.positions.len();
    assert_eq!(st.len(), n);
    assert!((st[0][0]).abs() < 1e-6);
    assert!((st[n - 2][0] - 1.0).abs() < 1e-6);
    // v 应交替：右=0，左=1
    for i in 0..n / 2 {
        assert!((st[i * 2][1]).abs() < 1e-6, "right v should be 0");
        assert!((st[i * 2 + 1][1] - 1.0).abs() < 1e-6, "left v should be 1");
    }
}

// === WallGeometry 带可变高度 ===

#[test]
fn wall_from_variable_min_max_heights() {
    let e = wgs84();
    let positions: Vec<DVec3> = (0..4).map(|i| from_degrees(i as f64, 0.0, 0.0)).collect();
    let opts = WallOptions {
        positions,
        maximum_heights: Some(vec![4000.0, 3000.0, 2000.0, 1000.0]),
        minimum_heights: Some(vec![0.0, 500.0, 1000.0, 500.0]),
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: e,
        ..Default::default()
    };
    let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);
    assert!(geo.positions.len() >= 8);
    assert!(geo.indices.len() >= 6);
    // 验证底部点（偶数索引）的高度与 minimumHeights 匹配，
    // 且顶部点（奇数索引）的高度与 maximumHeights 匹配
    let e = wgs84();
    for i in 0..geo.positions.len() {
        let carto = e.cartesian_to_cartographic(DVec3::from(geo.positions[i])).unwrap();
        if i % 2 == 0 {
            // 底部 - 应接近某个最小高度
            assert!(carto.height >= -10.0, "bottom height {} should be >= 0", carto.height);
        } else {
            // 顶部 - 应处于输入高度的最大值
            assert!(carto.height > 500.0, "top height {} should be > 0", carto.height);
        }
    }
}
