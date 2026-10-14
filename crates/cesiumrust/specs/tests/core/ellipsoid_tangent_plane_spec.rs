//! 参考自 `Specs/Core/EllipsoidTangentPlaneSpec`（27 个 it()，19 个 A 类）
//!
//! 省略 8 个 throws 测试（C 类：Rust 类型系统强制构造合法）。
//! result 参数变体被合并到其“返回所有权”对应的测试中。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::ellipsoid_tangent_plane::EllipsoidTangentPlane;
use glam::DVec2;
use glam::DVec3;
use std::f64::consts::PI;

const EPSILON14: f64 = 1e-14;

fn to_radians(degrees: f64) -> f64 {
    degrees * PI / 180.0
}

fn cartesian_from_degrees(lon_deg: f64, lat_deg: f64) -> DVec3 {
    let carto = Cartographic::from_radians(to_radians(lon_deg), to_radians(lat_deg), 0.0);
    Ellipsoid::WGS84.cartographic_to_cartesian(&carto)
}

#[test]
fn constructor_defaults_to_wgs84() {
    let origin = DVec3::new(Ellipsoid::WGS84.radii().x, 0.0, 0.0);
    let tangent_plane = EllipsoidTangentPlane::new(origin, &Ellipsoid::WGS84);
    assert_eq!(*tangent_plane.ellipsoid(), Ellipsoid::WGS84);
    assert!(tangent_plane.origin().abs_diff_eq(origin, EPSILON14));
}

#[test]
fn constructor_sets_expected_values() {
    let tangent_plane = EllipsoidTangentPlane::new(DVec3::X, &Ellipsoid::UNIT_SPHERE);
    assert_eq!(*tangent_plane.ellipsoid(), Ellipsoid::UNIT_SPHERE);
    assert!(tangent_plane.origin().abs_diff_eq(DVec3::X, EPSILON14));
}

#[test]
fn from_points_sets_expected_values() {
    let points = [DVec3::new(2.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 0.0)];
    let tangent_plane = EllipsoidTangentPlane::from_points(&points, &Ellipsoid::UNIT_SPHERE);
    assert_eq!(*tangent_plane.ellipsoid(), Ellipsoid::UNIT_SPHERE);
    // AABB([2,0,0],[0,0,0]) 的中心 = (1,0,0)，缩放到单位球面 = (1,0,0)
    assert!(tangent_plane.origin().abs_diff_eq(DVec3::X, EPSILON14));
}

#[test]
fn project_point_onto_plane_returns_none_for_unsolvable_projections() {
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let tangent_plane = EllipsoidTangentPlane::new(origin, &ellipsoid);
    // 位于 (0,0,1) 的点 - 方向为 (0,0,1)，平面法线为 (1,0,0)
    // 从 (0,0,1) 沿方向 (0,0,1) 的射线平行于平面 x=1
    let position = DVec3::new(0.0, 0.0, 1.0);
    let result = tangent_plane.project_point_onto_plane(position);
    assert!(result.is_none());
}

#[test]
fn project_point_onto_plane_works() {
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let tangent_plane = EllipsoidTangentPlane::new(origin, &ellipsoid);

    let position = DVec3::new(1.0, 0.0, 1.0);
    let expected = DVec2::new(0.0, 1.0);
    let result = tangent_plane.project_point_onto_plane(position).unwrap();
    assert!(
        result.abs_diff_eq(expected, EPSILON14),
        "got {:?}",
        result
    );
}

#[test]
fn project_points_onto_plane_works() {
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let tangent_plane = EllipsoidTangentPlane::new(origin, &ellipsoid);

    let positions = [
        DVec3::new(1.0, 0.0, 1.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(1.0, 1.0, 0.0),
    ];
    let expected = [
        DVec2::new(0.0, 1.0),
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
    ];
    let results = tangent_plane.project_points_onto_plane(&positions);
    assert_eq!(results.len(), 3);
    for i in 0..3 {
        assert!(
            results[i].abs_diff_eq(expected[i], EPSILON14),
            "index {}: got {:?}",
            i,
            results[i]
        );
    }
}

#[test]
fn project_points_onto_plane_skips_unprojectable_points() {
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let tangent_plane = EllipsoidTangentPlane::new(origin, &ellipsoid);

    let positions = [
        DVec3::new(1.0, 0.0, 1.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0), // 无法投影
        DVec3::new(1.0, 1.0, 0.0),
        DVec3::new(0.0, 1.0, 0.0), // 无法投影
    ];
    let expected = [
        DVec2::new(0.0, 1.0),
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
    ];
    let results = tangent_plane.project_points_onto_plane(&positions);
    assert_eq!(results.len(), 3);
    for i in 0..3 {
        assert!(
            results[i].abs_diff_eq(expected[i], EPSILON14),
            "index {}: got {:?}",
            i,
            results[i]
        );
    }
}

#[test]
fn project_point_onto_ellipsoid_works() {
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let tangent_plane = EllipsoidTangentPlane::new(origin, &ellipsoid);

    let position = DVec2::new(2.0, 2.0);
    let expected = DVec3::new(1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0);
    let result = tangent_plane.project_point_onto_ellipsoid(position);
    assert!(
        result.abs_diff_eq(expected, EPSILON14),
        "got {:?}",
        result
    );
}

#[test]
fn project_points_onto_ellipsoid_works() {
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let tangent_plane = EllipsoidTangentPlane::new(origin, &ellipsoid);

    let positions = [DVec2::new(2.0, -2.0), DVec2::new(2.0, 2.0)];
    let expected = [
        DVec3::new(1.0 / 3.0, 2.0 / 3.0, -2.0 / 3.0),
        DVec3::new(1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0),
    ];
    let results = tangent_plane.project_points_onto_ellipsoid(&positions);
    assert_eq!(results.len(), 2);
    for i in 0..2 {
        assert!(
            results[i].abs_diff_eq(expected[i], EPSILON14),
            "index {}: got {:?}",
            i,
            results[i]
        );
    }
}

#[test]
fn project_point_to_nearest_on_plane_works() {
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let tangent_plane = EllipsoidTangentPlane::new(origin, &ellipsoid);

    let position = DVec3::new(1.0, 0.0, 1.0);
    let expected = DVec2::new(0.0, 1.0);
    let result = tangent_plane.project_point_to_nearest_on_plane(position);
    assert!(
        result.abs_diff_eq(expected, EPSILON14),
        "got {:?}",
        result
    );
}

#[test]
fn project_point_to_nearest_on_plane_works_from_various_distances() {
    let ellipsoid = Ellipsoid::ZERO;
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let tangent_plane = EllipsoidTangentPlane::new(origin, &ellipsoid);

    let expected = DVec2::new(0.0, 0.0);
    let points = [
        DVec3::new(2.0, 0.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
    ];
    for p in &points {
        let result = tangent_plane.project_point_to_nearest_on_plane(*p);
        assert!(
            result.abs_diff_eq(expected, EPSILON14),
            "point {:?}: got {:?}",
            p,
            result
        );
    }
}

#[test]
fn project_points_to_nearest_on_plane_works() {
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let tangent_plane = EllipsoidTangentPlane::new(origin, &ellipsoid);

    let positions = [
        DVec3::new(1.0, 0.0, 1.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(1.0, 1.0, 0.0),
    ];
    let expected = [
        DVec2::new(0.0, 1.0),
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
    ];
    let results = tangent_plane.project_points_to_nearest_on_plane(&positions);
    assert_eq!(results.len(), 3);
    for i in 0..3 {
        assert!(
            results[i].abs_diff_eq(expected[i], EPSILON14),
            "index {}: got {:?}",
            i,
            results[i]
        );
    }
}

#[test]
fn project_points_onto_ellipsoid_with_arbitrary_ellipsoid_using_from_points() {
    let points = [
        cartesian_from_degrees(-72.0, 40.0),
        cartesian_from_degrees(-68.0, 35.0),
        cartesian_from_degrees(-75.0, 30.0),
        cartesian_from_degrees(-70.0, 30.0),
        cartesian_from_degrees(-68.0, 40.0),
    ];

    let tangent_plane = EllipsoidTangentPlane::from_points(&points, &Ellipsoid::WGS84);
    let points_2d = tangent_plane.project_points_onto_plane(&points);
    let positions_back = tangent_plane.project_points_onto_ellipsoid(&points_2d);

    // 第一个点应能精确往返转换
    let eps = 1e-5; // toBeCloseTo 的默认精度
    assert!(
        (positions_back[0].x - points[0].x).abs() < eps,
        "x: {} vs {}",
        positions_back[0].x,
        points[0].x
    );
    assert!(
        (positions_back[0].y - points[0].y).abs() < eps,
        "y: {} vs {}",
        positions_back[0].y,
        points[0].y
    );
    assert!(
        (positions_back[0].z - points[0].z).abs() < eps,
        "z: {} vs {}",
        positions_back[0].z,
        points[0].z
    );
}
