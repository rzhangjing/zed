//! 轮廓几何规格 - 参考自多个 OutlineGeometrySpec 文件
//!
//! 测试 box、ellipsoid、circle、rectangle、cylinder、plane、wall、corridor、
//! ellipse 与 frustum 轮廓几何生成器。

use cesium_geospatial::geometry::{
    box_outline_geometry, circle_outline_geometry, cylinder_outline_geometry,
    ellipsoid_outline_geometry, plane_outline_geometry, rectangle_outline_geometry,
    PrimitiveType,
};
use cesium_geospatial::geometry::corridor::{corridor_outline_geometry, CorridorOptions};
use cesium_geospatial::geometry::ellipse::{ellipse_outline_geometry, EllipseOptions};
use cesium_geospatial::geometry::frustum_geo::{frustum_outline_geometry, FrustumDef};
use cesium_geospatial::frustum::PerspectiveFrustum;
use cesium_geospatial::geometry::wall::{wall_outline_geometry, WallOptions};
use cesium_geospatial::{Cartographic, Ellipsoid, Rectangle};
use glam::{DQuat, DVec3};

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

fn equator_center() -> DVec3 {
    wgs84().cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0))
}

// ─── Box 轮廓 ───────────────────────────────────────────────────────────

#[test]
fn box_outline_has_8_vertices_and_12_edges() {
    // BoxOutlineGeometrySpec: "computes positions"
    let geo = box_outline_geometry(DVec3::new(-1.0, -2.0, -3.0), DVec3::new(1.0, 2.0, 3.0));
    assert_eq!(geo.positions.len(), 8, "box outline should have 8 corner vertices");
    assert_eq!(geo.indices.len(), 24, "box outline should have 12 edges * 2 indices");
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn box_outline_positions_at_corners() {
    let min = DVec3::new(-1.0, -2.0, -3.0);
    let max = DVec3::new(1.0, 2.0, 3.0);
    let geo = box_outline_geometry(min, max);

    // 所有位置应在角点处（每个坐标都是 min 或 max）
    for p in &geo.positions {
        let x_ok = (p[0] - min.x).abs() < 1e-10 || (p[0] - max.x).abs() < 1e-10;
        let y_ok = (p[1] - min.y).abs() < 1e-10 || (p[1] - max.y).abs() < 1e-10;
        let z_ok = (p[2] - min.z).abs() < 1e-10 || (p[2] - max.z).abs() < 1e-10;
        assert!(x_ok && y_ok && z_ok, "position {:?} should be at a corner", p);
    }
}

#[test]
fn box_outline_bounding_sphere() {
    let min = DVec3::new(-1.0, -1.0, -1.0);
    let max = DVec3::new(1.0, 1.0, 1.0);
    let geo = box_outline_geometry(min, max);

    let expected_center = DVec3::ZERO;
    let expected_radius = 3.0_f64.sqrt(); // 2x2x2 立方体的半对角线

    assert!((geo.bounding_sphere.center - expected_center).length() < 1e-10);
    assert!((geo.bounding_sphere.radius - expected_radius).abs() < 1e-10);
}

// ─── Ellipsoid 轮廓 ─────────────────────────────────────────────────────

#[test]
fn ellipsoid_outline_three_great_circles() {
    // EllipsoidOutlineGeometrySpec: "computes positions"
    let radii = DVec3::new(1.0, 1.0, 1.0);
    let stacks = 8;
    let slices = 8;
    let geo = ellipsoid_outline_geometry(radii, stacks, slices);

    // 3 个圆：XY (slices+1), XZ (stacks+1), YZ (stacks+1)
    let expected_vertices = (slices + 1) + (stacks + 1) + (stacks + 1);
    assert_eq!(geo.positions.len(), expected_vertices as usize);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn ellipsoid_outline_positions_on_surface() {
    let radii = DVec3::new(2.0, 3.0, 4.0);
    let geo = ellipsoid_outline_geometry(radii, 16, 16);

    for p in &geo.positions {
        // 椭球方程：(x/a)^2 + (y/b)^2 + (z/c)^2 = 1
        let val = (p[0] / radii.x).powi(2) + (p[1] / radii.y).powi(2) + (p[2] / radii.z).powi(2);
        assert!((val - 1.0).abs() < 1e-6, "position {:?} should be on ellipsoid surface, val={}", p, val);
    }
}

// ─── Circle 轮廓 ────────────────────────────────────────────────────────

#[test]
fn circle_outline_ring_of_segments() {
    // CircleOutlineGeometrySpec: "computes positions"
    let e = wgs84();
    let center = equator_center();
    let radius = 100000.0;
    let granularity = std::f64::consts::PI / 18.0; // 10 度
    let geo = circle_outline_geometry(center, radius, &e, granularity);

    assert!(!geo.positions.is_empty());
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    assert_eq!(geo.indices.len() % 2, 0, "line indices should be pairs");
}

#[test]
fn circle_outline_positions_at_correct_distance() {
    let e = wgs84();
    let center = equator_center();
    let radius = 50000.0;
    let granularity = std::f64::consts::PI / 36.0;
    let geo = circle_outline_geometry(center, radius, &e, granularity);

    // 所有位置应距中心约 `radius`（在表面上）
    for p in &geo.positions {
        let pos = DVec3::from(*p);
        let surface = e.scale_to_geodetic_surface(pos).unwrap_or(pos);
        let dist = (surface - center).length();
        assert!(
            (dist - radius).abs() < radius * 0.01,
            "distance {} should be ≈ radius {}", dist, radius
        );
    }
}

// ─── Rectangle 轮廓 ─────────────────────────────────────────────────────

#[test]
fn rectangle_outline_computes_positions() {
    // RectangleOutlineGeometrySpec: "computes positions"
    let e = wgs84();
    let rect = Rectangle::new(-2.0, -1.0, 0.0, 1.0);
    let geo = rectangle_outline_geometry(&rect, &e, 1.0);

    // 当 granularity=1.0 时：每条边约 2 段
    assert!(geo.positions.len() >= 8, "rectangle outline should have at least 8 positions");
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    assert_eq!(geo.indices.len() % 2, 0);
}

#[test]
fn rectangle_outline_positions_on_ellipsoid() {
    let e = wgs84();
    let rect = Rectangle::from_degrees(-10.0, -5.0, 10.0, 5.0);
    let granularity = std::f64::consts::PI / 18.0;
    let geo = rectangle_outline_geometry(&rect, &e, granularity);

    for p in &geo.positions {
        let pos = DVec3::from(*p);
        let carto = e.cartesian_to_cartographic(pos).unwrap();
        assert!(
            carto.height.abs() < 1.0,
            "outline position should be on surface, height={}", carto.height
        );
    }
}

#[test]
fn rectangle_outline_indices_valid() {
    let e = wgs84();
    let rect = Rectangle::from_degrees(-30.0, -20.0, 30.0, 20.0);
    let granularity = std::f64::consts::PI / 18.0;
    let geo = rectangle_outline_geometry(&rect, &e, granularity);

    let n = geo.positions.len() as u32;
    for (i, &idx) in geo.indices.iter().enumerate() {
        assert!(idx < n, "index[{}] = {} out of bounds (n={})", i, idx, n);
    }
}

// ─── Cylinder 轮廓 ──────────────────────────────────────────────────────

#[test]
fn cylinder_outline_two_circles_and_verticals() {
    // CylinderOutlineGeometrySpec: "computes positions"
    let geo = cylinder_outline_geometry(10.0, 5.0, 5.0, 16);

    // 底圆 (slices+1) + 顶圆 (slices+1) + 垂直线 (min(slices,16)*2)
    let expected = (16 + 1) + (16 + 1) + 16 * 2;
    assert_eq!(geo.positions.len(), expected);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn cylinder_outline_positions_at_correct_z() {
    let length = 10.0;
    let geo = cylinder_outline_geometry(length, 5.0, 5.0, 16);
    let half = length / 2.0;

    // 位置应在 z = -half 或 z = +half 处
    for p in &geo.positions {
        let z = p[2];
        assert!(
            (z - half).abs() < 1e-10 || (z + half).abs() < 1e-10,
            "z={} should be at ±{}", z, half
        );
    }
}

#[test]
fn cylinder_outline_cone_top_radius_zero() {
    // 圆锥：top_radius = 0
    let geo = cylinder_outline_geometry(10.0, 0.0, 5.0, 16);
    assert!(!geo.positions.is_empty());
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

// ─── Plane 轮廓 ─────────────────────────────────────────────────────────

#[test]
fn plane_outline_unit_quad() {
    // PlaneOutlineGeometrySpec: "computes positions"
    let geo = plane_outline_geometry();

    assert_eq!(geo.positions.len(), 4);
    assert_eq!(geo.indices.len(), 8); // 4 条边 * 2
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn plane_outline_positions_at_half_unit() {
    let geo = plane_outline_geometry();

    for p in &geo.positions {
        assert!((p[0].abs() - 0.5).abs() < 1e-10, "x={} should be ±0.5", p[0]);
        assert!((p[1].abs() - 0.5).abs() < 1e-10, "y={} should be ±0.5", p[1]);
        assert!(p[2].abs() < 1e-10, "z={} should be 0", p[2]);
    }
}

// ─── Wall 轮廓 ──────────────────────────────────────────────────────────

#[test]
fn wall_outline_basic() {
    // WallOutlineGeometrySpec: "computes positions"
    let e = wgs84();
    let positions = vec![
        e.cartographic_to_cartesian(&Cartographic::from_degrees(19.0, 47.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(19.0, 48.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(20.0, 48.0, 0.0)),
    ];
    let opts = WallOptions::from_constant_heights(positions, Some(0.0), Some(10000.0), e);
    let geo = wall_outline_geometry(&opts);

    assert!(!geo.positions.is_empty());
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    assert_eq!(geo.indices.len() % 2, 0);
}

#[test]
fn wall_outline_indices_valid() {
    let e = wgs84();
    let positions = vec![
        e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 1.0, 0.0)),
    ];
    let opts = WallOptions::from_constant_heights(positions, Some(0.0), Some(5000.0), e);
    let geo = wall_outline_geometry(&opts);

    let n = geo.positions.len() as u32;
    for &idx in &geo.indices {
        assert!(idx < n, "index {} out of bounds (n={})", idx, n);
    }
}

// ─── Corridor 轮廓 ──────────────────────────────────────────────────────

#[test]
fn corridor_outline_closed_loop() {
    // CorridorOutlineGeometrySpec: "computes positions"
    let e = wgs84();
    let positions = vec![
        e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 1.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 1.0, 0.0)),
    ];
    let opts = CorridorOptions {
        positions,
        width: 50000.0,
        height: 0.0,
        granularity: std::f64::consts::PI / 180.0,
        corner_type: Default::default(),
        ellipsoid: e,
    };
    let geo = corridor_outline_geometry(&opts);

    assert!(!geo.positions.is_empty());
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    // 轮廓应构成闭合环
    assert_eq!(geo.indices.len() % 2, 0);
}

#[test]
fn corridor_outline_indices_valid() {
    let e = wgs84();
    let positions = vec![
        e.cartographic_to_cartesian(&Cartographic::from_degrees(-10.0, 0.0, 0.0)),
        e.cartographic_to_cartesian(&Cartographic::from_degrees(10.0, 0.0, 0.0)),
    ];
    let opts = CorridorOptions {
        positions,
        width: 100000.0,
        height: 0.0,
        granularity: std::f64::consts::PI / 18.0,
        corner_type: Default::default(),
        ellipsoid: e,
    };
    let geo = corridor_outline_geometry(&opts);

    let n = geo.positions.len() as u32;
    for (i, &idx) in geo.indices.iter().enumerate() {
        assert!(idx < n, "index[{}] = {} out of bounds (n={})", i, idx, n);
    }
}

// ─── Ellipse 轮廓 ───────────────────────────────────────────────────────

#[test]
fn ellipse_outline_ring() {
    // EllipseOutlineGeometrySpec: "computes positions"
    let e = wgs84();
    let opts = EllipseOptions {
        center: equator_center(),
        semi_major_axis: 100000.0,
        semi_minor_axis: 50000.0,
        rotation: 0.0,
        st_rotation: 0.0,
        height: 0.0,
        granularity: std::f64::consts::PI / 36.0,
        ellipsoid: e,
    };
    let geo = ellipse_outline_geometry(&opts);

    assert!(!geo.positions.is_empty());
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    // 线环：n 个顶点，n*2 个索引
    assert_eq!(geo.indices.len(), geo.positions.len() * 2);
}

#[test]
fn ellipse_outline_indices_form_loop() {
    let e = wgs84();
    let opts = EllipseOptions {
        center: equator_center(),
        semi_major_axis: 80000.0,
        semi_minor_axis: 40000.0,
        rotation: 0.0,
        st_rotation: 0.0,
        height: 0.0,
        granularity: std::f64::consts::PI / 18.0,
        ellipsoid: e,
    };
    let geo = ellipse_outline_geometry(&opts);

    let n = geo.positions.len() as u32;
    for &idx in &geo.indices {
        assert!(idx < n, "index {} out of bounds (n={})", idx, n);
    }
}

// ─── Frustum 轮廓 ───────────────────────────────────────────────────────

#[test]
fn frustum_outline_8_vertices_12_edges() {
    // FrustumOutlineGeometrySpec: "computes positions"
    let frustum = FrustumDef::Perspective(PerspectiveFrustum::new(
        std::f64::consts::FRAC_PI_3,
        16.0 / 9.0,
        1.0,
        100.0,
    ));
    let geo = frustum_outline_geometry(&frustum, DVec3::ZERO, DQuat::IDENTITY);

    assert_eq!(geo.positions.len(), 8, "frustum outline should have 8 corners");
    assert_eq!(geo.indices.len(), 24, "frustum outline should have 12 edges * 2");
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn frustum_outline_near_far_depths() {
    let frustum = FrustumDef::Perspective(PerspectiveFrustum::new(
        std::f64::consts::FRAC_PI_3,
        1.0,
        2.0,
        50.0,
    ));
    let geo = frustum_outline_geometry(&frustum, DVec3::ZERO, DQuat::IDENTITY);

    // 前 4 个位置是近平面（z ≈ 2），后 4 个是远平面（z ≈ 50）
    for p in &geo.positions[0..4] {
        assert!((p[2] - 2.0).abs() < 1e-6, "near corner z={} should be ≈ 2", p[2]);
    }
    for p in &geo.positions[4..8] {
        assert!((p[2] - 50.0).abs() < 1e-6, "far corner z={} should be ≈ 50", p[2]);
    }
}

// ─── 通用属性 ─────────────────────────────────────────────────────

#[test]
fn all_outlines_use_lines_primitive() {
    let e = wgs84();

    let box_geo = box_outline_geometry(DVec3::splat(-1.0), DVec3::ONE);
    let ellipsoid_geo = ellipsoid_outline_geometry(DVec3::ONE, 8, 8);
    let circle_geo = circle_outline_geometry(equator_center(), 10000.0, &e, 0.1);
    let rect_geo = rectangle_outline_geometry(&Rectangle::from_degrees(0.0, 0.0, 1.0, 1.0), &e, 0.1);
    let cylinder_geo = cylinder_outline_geometry(10.0, 5.0, 5.0, 8);
    let plane_geo = plane_outline_geometry();

    assert_eq!(box_geo.primitive_type, PrimitiveType::Lines);
    assert_eq!(ellipsoid_geo.primitive_type, PrimitiveType::Lines);
    assert_eq!(circle_geo.primitive_type, PrimitiveType::Lines);
    assert_eq!(rect_geo.primitive_type, PrimitiveType::Lines);
    assert_eq!(cylinder_geo.primitive_type, PrimitiveType::Lines);
    assert_eq!(plane_geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn all_outlines_have_valid_bounding_spheres() {
    let e = wgs84();

    let geos = vec![
        box_outline_geometry(DVec3::splat(-1.0), DVec3::ONE),
        ellipsoid_outline_geometry(DVec3::ONE, 8, 8),
        circle_outline_geometry(equator_center(), 10000.0, &e, 0.1),
        rectangle_outline_geometry(&Rectangle::from_degrees(0.0, 0.0, 1.0, 1.0), &e, 0.1),
        cylinder_outline_geometry(10.0, 5.0, 5.0, 8),
        plane_outline_geometry(),
    ];

    for geo in &geos {
        assert!(geo.bounding_sphere.radius >= 0.0, "bounding sphere radius should be non-negative");
        // 所有位置应在包围球内
        for p in &geo.positions {
            let dist = (DVec3::from(*p) - geo.bounding_sphere.center).length();
            assert!(
                dist <= geo.bounding_sphere.radius + 1e-6,
                "position distance {} exceeds bounding sphere radius {}",
                dist, geo.bounding_sphere.radius
            );
        }
    }
}
