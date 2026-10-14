//! Stereographic（球面投影）规格测试 - 参考自：
//! - Specs/Core/StereographicSpec（15 个 it()）
//!
//! A 类测试：12 个（跳过 3 个 clone/result 参数类测试）

use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::math_utils;
use cesium_geospatial::stereographic::{PoleTangentPlane, Stereographic};
use glam::DVec2;

const EPSILON7: f64 = 1e-7;

fn from_degrees(lon_deg: f64, lat_deg: f64) -> glam::DVec3 {
    Ellipsoid::WGS84.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_degrees(lon_deg, lat_deg, 0.0),
    )
}

// ============================================================
// 构造
// ============================================================

#[test]
fn stereographic_construct_with_default_values() {
    let s = Stereographic::default();
    assert_eq!(s.x(), 0.0);
    assert_eq!(s.y(), 0.0);
    assert_eq!(s.tangent_plane, PoleTangentPlane::North);
}

#[test]
fn stereographic_construct_with_values() {
    let s = Stereographic::new(DVec2::new(1.0, 2.0), PoleTangentPlane::South);
    assert_eq!(s.x(), 1.0);
    assert_eq!(s.y(), 2.0);
    assert_eq!(s.tangent_plane, PoleTangentPlane::South);
}

// ============================================================
// fromCartesian
// ============================================================

#[test]
fn stereographic_from_cartesian_northern_hemisphere() {
    let s = Stereographic::from_cartesian(from_degrees(30.0, 60.0));
    assert!(
        (s.x() - 0.1347555369).abs() < EPSILON7,
        "x = {}", s.x()
    );
    assert!(
        (s.y() - (-0.2334034365)).abs() < EPSILON7,
        "y = {}", s.y()
    );
    assert_eq!(s.tangent_plane, PoleTangentPlane::North);
}

#[test]
fn stereographic_from_cartesian_at_0_0() {
    let s = Stereographic::from_cartesian(from_degrees(0.0, 0.0));
    assert!((s.x() - 0.0).abs() < EPSILON7, "x = {}", s.x());
    assert!((s.y() - (-1.0)).abs() < EPSILON7, "y = {}", s.y());
    assert_eq!(s.tangent_plane, PoleTangentPlane::North);
}

#[test]
fn stereographic_from_cartesian_southern_hemisphere() {
    let s = Stereographic::from_cartesian(from_degrees(30.0, -60.0));
    assert!(
        (s.x() - 0.1347555369).abs() < EPSILON7,
        "x = {}", s.x()
    );
    assert!(
        (s.y() - (-0.2334034365)).abs() < EPSILON7,
        "y = {}", s.y()
    );
    assert_eq!(s.tangent_plane, PoleTangentPlane::South);
}

// ============================================================
// 经度
// ============================================================

#[test]
fn stereographic_longitude_northern_hemisphere() {
    let cases: [(f64, f64); 5] = [
        (30.0, 60.0),
        (60.0, 30.0),
        (-60.0, 30.0),
        (-135.0, 60.0),
        (135.0, 60.0),
    ];
    for (lon_deg, lat_deg) in cases {
        let s = Stereographic::from_cartesian(from_degrees(lon_deg, lat_deg));
        let expected = math_utils::to_radians(lon_deg);
        assert!(
            (s.longitude() - expected).abs() < EPSILON7,
            "lon_deg={}: got {}, expected {}",
            lon_deg, s.longitude(), expected
        );
    }
}

#[test]
fn stereographic_longitude_southern_hemisphere() {
    let cases: [(f64, f64); 5] = [
        (30.0, -60.0),
        (60.0, -30.0),
        (-60.0, -30.0),
        (-135.0, -60.0),
        (135.0, -60.0),
    ];
    for (lon_deg, lat_deg) in cases {
        let s = Stereographic::from_cartesian(from_degrees(lon_deg, lat_deg));
        let expected = math_utils::to_radians(lon_deg);
        assert!(
            (s.longitude() - expected).abs() < EPSILON7,
            "lon_deg={}: got {}, expected {}",
            lon_deg, s.longitude(), expected
        );
    }
}

// ============================================================
// 等量纬度（Conformal Latitude）
// ============================================================

#[test]
fn stereographic_conformal_latitude_northern_hemisphere() {
    let s = Stereographic::from_cartesian(from_degrees(30.0, 60.0));
    assert!(
        (s.conformal_latitude() - 1.04428418).abs() < EPSILON7,
        "got {}", s.conformal_latitude()
    );

    let s = Stereographic::from_cartesian(from_degrees(60.0, 30.0));
    assert!(
        (s.conformal_latitude() - 0.52069517).abs() < EPSILON7,
        "got {}", s.conformal_latitude()
    );
}

#[test]
fn stereographic_conformal_latitude_southern_hemisphere() {
    let s = Stereographic::from_cartesian(from_degrees(30.0, -60.0));
    assert!(
        (s.conformal_latitude() - (-1.04428418)).abs() < EPSILON7,
        "got {}", s.conformal_latitude()
    );

    let s = Stereographic::from_cartesian(from_degrees(60.0, -30.0));
    assert!(
        (s.conformal_latitude() - (-0.52069517)).abs() < EPSILON7,
        "got {}", s.conformal_latitude()
    );
}

// ============================================================
// getLatitude
// ============================================================

#[test]
fn stereographic_latitude_northern_hemisphere() {
    let s = Stereographic::from_cartesian(from_degrees(30.0, 60.0));
    let lat = s.get_latitude(&Ellipsoid::WGS84);
    assert!(
        (lat - math_utils::to_radians(60.0)).abs() < EPSILON7,
        "got {}", lat
    );

    let s = Stereographic::from_cartesian(from_degrees(60.0, 30.0));
    let lat = s.get_latitude(&Ellipsoid::WGS84);
    assert!(
        (lat - math_utils::to_radians(30.0)).abs() < EPSILON7,
        "got {}", lat
    );
}

#[test]
fn stereographic_latitude_southern_hemisphere() {
    let s = Stereographic::from_cartesian(from_degrees(30.0, -60.0));
    let lat = s.get_latitude(&Ellipsoid::WGS84);
    assert!(
        (lat - math_utils::to_radians(-60.0)).abs() < EPSILON7,
        "got {}", lat
    );

    let s = Stereographic::from_cartesian(from_degrees(60.0, -30.0));
    let lat = s.get_latitude(&Ellipsoid::WGS84);
    assert!(
        (lat - math_utils::to_radians(-30.0)).abs() < EPSILON7,
        "got {}", lat
    );
}

// ============================================================
// fromCartesianArray
// ============================================================

#[test]
fn stereographic_from_cartesian_array() {
    let cartesians = [from_degrees(30.0, 60.0), from_degrees(30.0, -60.0)];
    let results = Stereographic::from_cartesian_array(&cartesians);
    assert_eq!(results.len(), 2);

    assert!((results[0].x() - 0.1347555369).abs() < EPSILON7);
    assert!((results[0].y() - (-0.2334034365)).abs() < EPSILON7);
    assert_eq!(results[0].tangent_plane, PoleTangentPlane::North);

    assert!((results[1].x() - 0.1347555369).abs() < EPSILON7);
    assert!((results[1].y() - (-0.2334034365)).abs() < EPSILON7);
    assert_eq!(results[1].tangent_plane, PoleTangentPlane::South);
}
