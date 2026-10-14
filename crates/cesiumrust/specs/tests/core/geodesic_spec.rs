//! 参考自 `Specs/Core/EllipsoidGeodesicSpec`（22 个 it()，14 个 A 类）
//!
//! 省略 8 个 throws 测试（C 类：Rust 类型系统强制构造合法）。
//! 2 个 result 参数测试被合并到其“返回所有权”对应的测试中。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::geodesic::EllipsoidGeodesic;
use std::f64::consts::PI;

const EPSILON11: f64 = 1e-11;
const EPSILON13: f64 = 1e-13;
const EPSILON12: f64 = 1e-12;
const PI_OVER_TWO: f64 = PI / 2.0;

#[test]
fn works_with_two_points() {
    let fifteen_degrees = PI / 12.0;
    let start = Cartographic::from_radians(fifteen_degrees, fifteen_degrees, 0.0);
    let thirty_degrees = PI / 6.0;
    let end = Cartographic::from_radians(thirty_degrees, thirty_degrees, 0.0);

    let geodesic = EllipsoidGeodesic::new(start, end, &Ellipsoid::WGS84);
    assert_eq!(start.longitude, geodesic.start().longitude);
    assert_eq!(start.latitude, geodesic.start().latitude);
    assert_eq!(end.longitude, geodesic.end().longitude);
    assert_eq!(end.latitude, geodesic.end().latitude);
}

#[test]
fn sets_end_points() {
    let start = Cartographic::from_radians(PI_OVER_TWO, 0.0, 0.0);
    let end = Cartographic::from_radians(PI_OVER_TWO, PI_OVER_TWO, 0.0);
    let mut geodesic = EllipsoidGeodesic::new(start, end, &Ellipsoid::WGS84);
    geodesic.set_end_points(start, end);
    assert_eq!(start.longitude, geodesic.start().longitude);
    assert_eq!(start.latitude, geodesic.start().latitude);
    assert_eq!(end.longitude, geodesic.end().longitude);
    assert_eq!(end.latitude, geodesic.end().latitude);
}

#[test]
fn gets_start_heading() {
    let ellipsoid = Ellipsoid::new(6.0, 6.0, 3.0);
    let start = Cartographic::from_radians(PI_OVER_TWO, 0.0, 0.0);
    let end = Cartographic::from_radians(PI, 0.0, 0.0);

    let geodesic = EllipsoidGeodesic::new(start, end, &ellipsoid);
    assert!(
        (PI_OVER_TWO - geodesic.start_heading()).abs() < EPSILON11,
        "start_heading = {}",
        geodesic.start_heading()
    );
}

#[test]
fn gets_end_heading() {
    let ellipsoid = Ellipsoid::new(6.0, 6.0, 3.0);
    let start = Cartographic::from_radians(PI_OVER_TWO, 0.0, 0.0);
    let end = Cartographic::from_radians(PI, 0.0, 0.0);

    let geodesic = EllipsoidGeodesic::new(start, end, &ellipsoid);
    assert!(
        (PI_OVER_TWO - geodesic.end_heading()).abs() < EPSILON11,
        "end_heading = {}",
        geodesic.end_heading()
    );
}

#[test]
fn computes_distance_at_equator() {
    let ellipsoid = Ellipsoid::new(6.0, 6.0, 3.0);
    let start = Cartographic::from_radians(PI_OVER_TWO, 0.0, 0.0);
    let end = Cartographic::from_radians(PI, 0.0, 0.0);

    let geodesic = EllipsoidGeodesic::new(start, end, &ellipsoid);
    let expected = PI_OVER_TWO * 6.0;
    assert!(
        (expected - geodesic.surface_distance()).abs() < EPSILON11,
        "distance = {}",
        geodesic.surface_distance()
    );
}

#[test]
fn computes_distance_very_close_to_equator() {
    // 参见 https://github.com/CesiumGS/cesium/issues/9248
    let ellipsoid = Ellipsoid::new(6.0, 6.0, 3.0);
    let epsilon10: f64 = 1e-10;
    let start = Cartographic::from_radians(-epsilon10, epsilon10, 0.0);
    let end = Cartographic::from_radians(epsilon10, epsilon10, 0.0);

    let geodesic = EllipsoidGeodesic::new(start, end, &ellipsoid);
    assert!(!geodesic.surface_distance().is_nan());
}

#[test]
fn computes_distance_at_meridian() {
    let ellipsoid = Ellipsoid::new(6.0, 6.0, 6.0);
    let fifteen_degrees = PI / 12.0;
    let start = Cartographic::from_radians(PI_OVER_TWO, fifteen_degrees, 0.0);
    let fortyfive_degrees = PI / 4.0;
    let end = Cartographic::from_radians(PI_OVER_TWO, fortyfive_degrees, 0.0);

    let geodesic = EllipsoidGeodesic::new(start, end, &ellipsoid);
    let thirty_degrees = PI / 6.0;
    let expected = thirty_degrees * 6.0;
    assert!(
        (expected - geodesic.surface_distance()).abs() < EPSILON11,
        "distance = {}",
        geodesic.surface_distance()
    );
}

#[test]
fn computes_distance_at_pole() {
    let ellipsoid = Ellipsoid::new(6.0, 6.0, 6.0);
    let seventyfive_degrees = (PI / 12.0) * 5.0;
    let fortyfive_degrees = PI / 4.0;
    let start = Cartographic::from_radians(0.0, -fortyfive_degrees, 0.0);
    let end = Cartographic::from_radians(PI, -seventyfive_degrees, 0.0);

    let geodesic = EllipsoidGeodesic::new(start, end, &ellipsoid);
    let sixty_degrees = PI / 3.0;
    let expected = sixty_degrees * 6.0;
    assert!(
        (expected - geodesic.surface_distance()).abs() < EPSILON11,
        "distance = {}",
        geodesic.surface_distance()
    );
}

#[test]
fn interpolates_start_and_end_points() {
    let fifteen_degrees = PI / 12.0;
    let start = Cartographic::from_radians(fifteen_degrees, fifteen_degrees, 0.0);
    let thirty_degrees = PI / 6.0;
    let end = Cartographic::from_radians(thirty_degrees, thirty_degrees, 0.0);

    let geodesic = EllipsoidGeodesic::new(start, end, &Ellipsoid::WGS84);
    let distance = geodesic.surface_distance();

    let first = geodesic.interpolate_using_surface_distance(0.0);
    let last = geodesic.interpolate_using_surface_distance(distance);

    assert!((start.longitude - first.longitude).abs() < EPSILON13);
    assert!((start.latitude - first.latitude).abs() < EPSILON13);
    assert!((end.longitude - last.longitude).abs() < EPSILON13);
    assert!((end.latitude - last.latitude).abs() < EPSILON13);
}

#[test]
fn interpolates_midpoint() {
    let fifteen_degrees = PI / 12.0;
    let start = Cartographic::from_radians(fifteen_degrees, 0.0, 0.0);
    let fortyfive_degrees = PI / 4.0;
    let end = Cartographic::from_radians(fortyfive_degrees, 0.0, 0.0);
    let thirty_degrees = PI / 6.0;

    let geodesic = EllipsoidGeodesic::new(start, end, &Ellipsoid::WGS84);
    let distance = Ellipsoid::WGS84.maximum_radius() * fifteen_degrees;

    let midpoint = geodesic.interpolate_using_surface_distance(distance);

    assert!((thirty_degrees - midpoint.longitude).abs() < EPSILON13);
    assert!((0.0 - midpoint.latitude).abs() < EPSILON13);
}

#[test]
fn interpolates_start_and_end_points_using_fraction() {
    let fifteen_degrees = PI / 12.0;
    let start = Cartographic::from_radians(fifteen_degrees, fifteen_degrees, 0.0);
    let thirty_degrees = PI / 6.0;
    let end = Cartographic::from_radians(thirty_degrees, thirty_degrees, 0.0);

    let geodesic = EllipsoidGeodesic::new(start, end, &Ellipsoid::WGS84);

    let first = geodesic.interpolate_using_fraction(0.0);
    let last = geodesic.interpolate_using_fraction(1.0);

    assert!((start.longitude - first.longitude).abs() < EPSILON13);
    assert!((start.latitude - first.latitude).abs() < EPSILON13);
    assert!((end.longitude - last.longitude).abs() < EPSILON13);
    assert!((end.latitude - last.latitude).abs() < EPSILON13);
}

#[test]
fn interpolates_midpoint_using_fraction() {
    let fifteen_degrees = PI / 12.0;
    let start = Cartographic::from_radians(fifteen_degrees, 0.0, 0.0);
    let fortyfive_degrees = PI / 4.0;
    let end = Cartographic::from_radians(fortyfive_degrees, 0.0, 0.0);
    let thirty_degrees = PI / 6.0;

    let geodesic = EllipsoidGeodesic::new(start, end, &Ellipsoid::WGS84);

    let midpoint = geodesic.interpolate_using_fraction(0.5);

    assert!((thirty_degrees - midpoint.longitude).abs() < EPSILON12);
    assert!((0.0 - midpoint.latitude).abs() < EPSILON12);
}
