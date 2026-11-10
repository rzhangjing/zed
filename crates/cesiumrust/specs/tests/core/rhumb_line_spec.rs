//! 参考自 `Specs/Core/EllipsoidRhumbLineSpec`（49 个 it()，约 40 个 A 类）
//!
//! 省略 7 个 throws 测试（C 类：Rust 类型系统强制构造合法）。
//! 2 个 result 参数测试被合并到其“返回所有权”对应的测试中。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::ellipsoid_rhumb_line::EllipsoidRhumbLine;
use cesium_geospatial::geodesic::EllipsoidGeodesic;
use std::f64::consts::PI;

const EPSILON3: f64 = 1e-3;
const EPSILON4: f64 = 1e-4;
const EPSILON6: f64 = 1e-6;
const EPSILON8: f64 = 1e-8;
const EPSILON12: f64 = 1e-12;
const EPSILON14: f64 = 1e-14;
const PI_OVER_TWO: f64 = PI / 2.0;
const PI_OVER_FOUR: f64 = PI / 4.0;

fn fifteen_degrees() -> f64 {
    PI / 12.0
}
fn thirty_degrees() -> f64 {
    PI / 6.0
}
fn fortyfive_degrees() -> f64 {
    PI / 4.0
}
fn one_degree() -> f64 {
    PI / 180.0
}
fn three_hundred_degrees() -> f64 {
    5.0 * PI / 6.0
}

#[test]
fn can_create_using_from_start_heading_distance() {
    let ellipsoid = Ellipsoid::WGS84;
    let start = Cartographic::from_radians(fifteen_degrees(), fifteen_degrees(), 0.0);
    let heading = fifteen_degrees();
    let distance = fifteen_degrees() * ellipsoid.maximum_radius();

    let rhumb = EllipsoidRhumbLine::from_start_heading_distance(&start, heading, distance, &ellipsoid);
    assert_eq!(start.longitude, rhumb.start().longitude);
    assert_eq!(start.latitude, rhumb.start().latitude);
    assert!((distance - rhumb.surface_distance()).abs() < EPSILON6);
    assert!((heading - rhumb.heading()).abs() < EPSILON12);
}

#[test]
fn works_with_two_points() {
    let start = Cartographic::from_radians(fifteen_degrees(), fifteen_degrees(), 0.0);
    let end = Cartographic::from_radians(thirty_degrees(), thirty_degrees(), 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);
    assert_eq!(start.longitude, rhumb.start().longitude);
    assert_eq!(start.latitude, rhumb.start().latitude);
    assert_eq!(end.longitude, rhumb.end().longitude);
    assert_eq!(end.latitude, rhumb.end().latitude);
}

#[test]
fn sets_end_points() {
    let start = Cartographic::from_radians(PI_OVER_TWO, 0.0, 0.0);
    let end = Cartographic::from_radians(PI_OVER_TWO, PI_OVER_TWO, 0.0);
    let mut rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);
    rhumb.set_end_points(&start, &end);
    assert_eq!(start.longitude, rhumb.start().longitude);
    assert_eq!(end.longitude, rhumb.end().longitude);
}

#[test]
fn gets_heading() {
    let ellipsoid = Ellipsoid::new(6.0, 6.0, 3.0);
    let start = Cartographic::from_radians(PI_OVER_TWO, 0.0, 0.0);
    let end = Cartographic::from_radians(PI, 0.0, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &ellipsoid);
    assert!((PI_OVER_TWO - rhumb.heading()).abs() < EPSILON12);
}

#[test]
fn computes_heading_not_going_over_the_pole() {
    let ellipsoid = Ellipsoid::WGS84;
    let start = Cartographic::from_radians(0.0, 1.2, 0.0);
    let end = Cartographic::from_radians(PI, 1.5, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &ellipsoid);
    assert_ne!(0.0, rhumb.heading());
}

#[test]
fn computes_heading_going_over_the_pole() {
    let ellipsoid = Ellipsoid::WGS84;
    let start = Cartographic::from_radians(1.3, PI_OVER_TWO, 0.0);
    let end = Cartographic::from_radians(0.0, PI / 2.4, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &ellipsoid);
    assert_ne!(0.0, rhumb.heading());
}

#[test]
fn heading_works_when_going_around_the_world_at_constant_latitude() {
    let ellipsoid = Ellipsoid::new(6.0, 6.0, 6.0);
    let start = Cartographic::from_radians(0.0, 0.3, 0.0);
    let end = Cartographic::from_radians(PI_OVER_TWO, 0.3, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &ellipsoid);
    assert!((PI_OVER_TWO - rhumb.heading()).abs() < EPSILON12);

    let start2 = Cartographic::from_radians(3.0 * PI_OVER_TWO, 0.3, 0.0);
    let end2 = Cartographic::from_radians(PI, 0.3, 0.0);
    let rhumb2 = EllipsoidRhumbLine::new(&start2, &end2, &ellipsoid);
    assert!((-PI_OVER_TWO - rhumb2.heading()).abs() < EPSILON12);
}

#[test]
fn computes_heading_for_vertical_lines() {
    let ellipsoid = Ellipsoid::WGS84;
    let start = Cartographic::from_radians(0.0, 1.2, 0.0);
    let end = Cartographic::from_radians(0.0, 1.5, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &ellipsoid);
    assert!((0.0 - rhumb.heading()).abs() < EPSILON12);

    let rhumb2 = EllipsoidRhumbLine::new(&end, &start, &ellipsoid);
    assert!((PI - rhumb2.heading()).abs() < EPSILON12);
}

#[test]
fn computes_distance_at_equator() {
    let ellipsoid = Ellipsoid::new(6.0, 6.0, 3.0);
    let start = Cartographic::from_radians(-PI_OVER_FOUR, 0.0, 0.0);
    let end = Cartographic::from_radians(PI_OVER_FOUR, 0.0, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &ellipsoid);
    let expected = PI_OVER_TWO * 6.0;
    assert!((expected - rhumb.surface_distance()).abs() < EPSILON12);
}

#[test]
fn computes_distance_at_meridian() {
    let ellipsoid = Ellipsoid::new(6.0, 6.0, 6.0);
    let start = Cartographic::from_radians(PI_OVER_TWO, fifteen_degrees(), 0.0);
    let end = Cartographic::from_radians(PI_OVER_TWO, fortyfive_degrees(), 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &ellipsoid);
    let expected = thirty_degrees() * 6.0;
    assert!((expected - rhumb.surface_distance()).abs() < EPSILON12);
}

#[test]
fn computes_equal_distance_on_sphere_for_90_degree_arcs() {
    let ellipsoid = Ellipsoid::new(6.0, 6.0, 6.0);
    let fortyfive_south = Cartographic::from_radians(0.0, -PI_OVER_FOUR, 0.0);
    let fortyfive_north = Cartographic::from_radians(0.0, PI_OVER_FOUR, 0.0);
    let fortyfive_west = Cartographic::from_radians(-PI_OVER_FOUR, 0.0, 0.0);
    let fortyfive_east = Cartographic::from_radians(PI_OVER_FOUR, 0.0, 0.0);

    let west_east = EllipsoidRhumbLine::new(&fortyfive_west, &fortyfive_east, &ellipsoid);
    let south_north = EllipsoidRhumbLine::new(&fortyfive_south, &fortyfive_north, &ellipsoid);
    let east_west = EllipsoidRhumbLine::new(&fortyfive_east, &fortyfive_west, &ellipsoid);
    let north_south = EllipsoidRhumbLine::new(&fortyfive_north, &fortyfive_south, &ellipsoid);

    let expected = PI_OVER_TWO * 6.0;
    assert!((expected - west_east.surface_distance()).abs() < EPSILON12);
    assert!((expected - south_north.surface_distance()).abs() < EPSILON12);
    assert!((west_east.surface_distance() - south_north.surface_distance()).abs() < EPSILON12);
    assert!((expected - east_west.surface_distance()).abs() < EPSILON12);
    assert!((expected - north_south.surface_distance()).abs() < EPSILON12);
    assert!((east_west.surface_distance() - north_south.surface_distance()).abs() < EPSILON12);
}

#[test]
fn computes_distance_at_same_latitude() {
    let ellipsoid = Ellipsoid::new(6.0, 6.0, 6.0);
    let start = Cartographic::from_radians(0.0, -fortyfive_degrees(), 0.0);
    let end = Cartographic::from_radians(PI_OVER_TWO, -fortyfive_degrees(), 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &ellipsoid);
    let distance = fortyfive_degrees().cos() * PI_OVER_TWO * 6.0;
    assert!((distance - rhumb.surface_distance()).abs() < EPSILON12);
}

#[test]
fn computes_heading_and_distance_given_endpoints_on_sphere() {
    let radius = 6378137.0;
    let ellipsoid = Ellipsoid::new(radius, radius, radius);
    let initial = Cartographic::from_radians(fifteen_degrees(), fifteen_degrees(), 0.0);
    let distance = radius * fifteen_degrees();

    let rhumb1 = EllipsoidRhumbLine::from_start_heading_distance(&initial, fifteen_degrees(), distance, &ellipsoid);
    let rhumb2 = EllipsoidRhumbLine::new(&initial, &rhumb1.end(), &ellipsoid);

    assert!((fifteen_degrees() - rhumb2.heading()).abs() < EPSILON12);
    assert!((distance - rhumb2.surface_distance()).abs() < EPSILON6);
}

#[test]
fn computes_heading_and_distance_given_endpoints_on_spheroid() {
    let ellipsoid = Ellipsoid::WGS84;
    let initial = Cartographic::from_radians(fifteen_degrees(), fifteen_degrees(), 0.0);
    let distance = ellipsoid.maximum_radius() * fifteen_degrees();

    let rhumb1 = EllipsoidRhumbLine::from_start_heading_distance(&initial, fifteen_degrees(), distance, &ellipsoid);
    let rhumb2 = EllipsoidRhumbLine::new(&initial, &rhumb1.end(), &ellipsoid);

    assert!((fifteen_degrees() - rhumb2.heading()).abs() < EPSILON12);
    assert!((distance - rhumb2.surface_distance()).abs() < EPSILON6);
}

#[test]
fn tests_sphere_close_to_90_degrees() {
    let radius = 6378137.0;
    let ellipsoid = Ellipsoid::new(radius, radius, radius);
    let initial = Cartographic::from_radians(fifteen_degrees(), fifteen_degrees(), 0.0);
    let distance = radius * fifteen_degrees();

    let headings = [
        89.0 * one_degree(),
        89.9 * one_degree(),
        90.0 * one_degree(),
        90.1 * one_degree(),
        90.02 * one_degree(),
    ];

    for heading in headings {
        let rhumb1 = EllipsoidRhumbLine::from_start_heading_distance(&initial, heading, distance, &ellipsoid);
        let rhumb2 = EllipsoidRhumbLine::new(&initial, &rhumb1.end(), &ellipsoid);
        assert!(
            (rhumb1.heading() - rhumb2.heading()).abs() < EPSILON12,
            "heading mismatch at {}: {} vs {}",
            heading,
            rhumb1.heading(),
            rhumb2.heading()
        );
        assert!(
            (rhumb1.surface_distance() - rhumb2.surface_distance()).abs() < EPSILON6,
            "distance mismatch at {}",
            heading
        );
    }
}

#[test]
fn tests_spheroid_close_to_90_degrees() {
    let ellipsoid = Ellipsoid::WGS84;
    let initial = Cartographic::from_radians(fifteen_degrees(), fifteen_degrees(), 0.0);
    let distance = ellipsoid.maximum_radius() * fifteen_degrees();

    let headings = [
        89.0 * one_degree(),
        89.9 * one_degree(),
        90.0 * one_degree(),
        90.1 * one_degree(),
        90.02 * one_degree(),
    ];

    for heading in headings {
        let rhumb1 = EllipsoidRhumbLine::from_start_heading_distance(&initial, heading, distance, &ellipsoid);
        let rhumb2 = EllipsoidRhumbLine::new(&initial, &rhumb1.end(), &ellipsoid);
        assert!(
            (rhumb1.heading() - rhumb2.heading()).abs() < EPSILON12,
            "heading mismatch at {}",
            heading
        );
        assert!(
            (rhumb1.surface_distance() - rhumb2.surface_distance()).abs() < EPSILON6,
            "distance mismatch at {}",
            heading
        );
    }
}

#[test]
fn test_spheroid_across_meridian() {
    let ellipsoid = Ellipsoid::WGS84;
    let initial = Cartographic::from_radians(-fifteen_degrees(), 0.0, 0.0);
    let final_pt = Cartographic::from_radians(fifteen_degrees(), 0.0, 0.0);
    let distance = ellipsoid.maximum_radius() * 2.0 * fifteen_degrees();

    let rhumb1 = EllipsoidRhumbLine::new(&initial, &final_pt, &ellipsoid);
    let rhumb2 = EllipsoidRhumbLine::from_start_heading_distance(&initial, PI_OVER_TWO, distance, &ellipsoid);

    assert!((rhumb1.heading() - rhumb2.heading()).abs() < EPSILON12);
    assert!((rhumb1.surface_distance() - rhumb2.surface_distance()).abs() < EPSILON6);
}

#[test]
fn test_across_idl_with_pi_range() {
    let ellipsoid = Ellipsoid::WGS84;
    let initial = Cartographic::from_radians(-PI + fifteen_degrees(), 0.0, 0.0);
    let final_pt = Cartographic::from_radians(PI - fifteen_degrees(), 0.0, 0.0);
    let distance = ellipsoid.maximum_radius() * 2.0 * fifteen_degrees();

    let rhumb1 = EllipsoidRhumbLine::new(&initial, &final_pt, &ellipsoid);
    let rhumb2 =
        EllipsoidRhumbLine::from_start_heading_distance(&initial, 3.0 * PI_OVER_TWO, distance, &ellipsoid);

    assert!((-PI_OVER_TWO - rhumb1.heading()).abs() < EPSILON12);
    assert!((distance - rhumb1.surface_distance()).abs() < EPSILON6);
    assert!((rhumb1.heading() - rhumb2.heading()).abs() < EPSILON12);
    assert!((rhumb1.surface_distance() - rhumb2.surface_distance()).abs() < EPSILON6);

    let rhumb3 = EllipsoidRhumbLine::new(&final_pt, &initial, &ellipsoid);
    let rhumb4 =
        EllipsoidRhumbLine::from_start_heading_distance(&final_pt, PI_OVER_TWO, distance, &ellipsoid);
    assert!((PI_OVER_TWO - rhumb3.heading()).abs() < EPSILON12);
    assert!((distance - rhumb3.surface_distance()).abs() < EPSILON6);
    assert!((rhumb3.heading() - rhumb4.heading()).abs() < EPSILON12);
    assert!((rhumb3.surface_distance() - rhumb4.surface_distance()).abs() < EPSILON6);
}

#[test]
fn test_across_equator() {
    let ellipsoid = Ellipsoid::WGS84;
    let initial = Cartographic::from_radians(fifteen_degrees(), -one_degree(), 0.0);
    let final_pt = Cartographic::from_radians(fifteen_degrees(), one_degree(), 0.0);

    let rhumb = EllipsoidRhumbLine::new(&initial, &final_pt, &ellipsoid);
    let geodesic = EllipsoidGeodesic::new(initial, final_pt, &ellipsoid);
    assert!((0.0 - rhumb.heading()).abs() < EPSILON12);
    assert!((geodesic.start_heading() - rhumb.heading()).abs() < EPSILON12);
    assert!((geodesic.surface_distance() - rhumb.surface_distance()).abs() < EPSILON6);
}

#[test]
fn test_on_equator() {
    let ellipsoid = Ellipsoid::WGS84;
    let initial = Cartographic::from_radians(0.0, 0.0, 0.0);
    let final_pt = Cartographic::from_radians(PI - 1.0, 0.0, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&initial, &final_pt, &ellipsoid);
    let geodesic = EllipsoidGeodesic::new(initial, final_pt, &ellipsoid);
    assert!((PI_OVER_TWO - rhumb.heading()).abs() < EPSILON12);
    assert!((geodesic.start_heading() - rhumb.heading()).abs() < EPSILON12);
    assert!(
        (geodesic.surface_distance() - rhumb.surface_distance()).abs() < EPSILON4,
        "geodesic={} rhumb={}",
        geodesic.surface_distance(),
        rhumb.surface_distance()
    );
}

#[test]
fn test_close_to_poles() {
    let ellipsoid = Ellipsoid::WGS84;
    let five_degrees = PI / 36.0;
    let eighty_degrees = 16.0 * five_degrees;
    let distance = fifteen_degrees() * ellipsoid.maximum_radius();

    let initial = Cartographic::from_radians(0.0, eighty_degrees, 0.0);

    let rhumb1 = EllipsoidRhumbLine::from_start_heading_distance(&initial, eighty_degrees, distance, &ellipsoid);
    let rhumb2 = EllipsoidRhumbLine::new(&initial, &rhumb1.end(), &ellipsoid);

    assert!((rhumb1.heading() - rhumb2.heading()).abs() < EPSILON12);
    assert!((rhumb1.surface_distance() - rhumb2.surface_distance()).abs() < EPSILON6);
}

#[test]
fn test_interpolate_fraction() {
    let ellipsoid = Ellipsoid::WGS84;
    let initial = Cartographic::from_radians(0.0, 0.0, 0.0);
    let final_pt = Cartographic::from_radians(PI_OVER_TWO, 0.0, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&initial, &final_pt, &ellipsoid);
    let interpolated = rhumb.interpolate_using_fraction(0.5);

    assert!((PI_OVER_FOUR - interpolated.longitude).abs() < EPSILON12);
    assert!((0.0 - interpolated.latitude).abs() < EPSILON12);
}

#[test]
fn test_interpolate_distance() {
    let ellipsoid = Ellipsoid::WGS84;
    let initial = Cartographic::from_radians(0.0, 0.0, 0.0);
    let final_pt = Cartographic::from_radians(PI_OVER_TWO, 0.0, 0.0);
    let distance = ellipsoid.maximum_radius() * PI_OVER_FOUR;

    let rhumb = EllipsoidRhumbLine::new(&initial, &final_pt, &ellipsoid);
    let interpolated = rhumb.interpolate_using_surface_distance(distance);

    assert!((PI_OVER_FOUR - interpolated.longitude).abs() < EPSILON12);
    assert!((0.0 - interpolated.latitude).abs() < EPSILON12);
}

#[test]
fn interpolates_start_and_end_points() {
    let start = Cartographic::from_radians(fifteen_degrees(), fifteen_degrees(), 0.0);
    let end = Cartographic::from_radians(thirty_degrees(), thirty_degrees(), 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);
    let distance = rhumb.surface_distance();

    let first = rhumb.interpolate_using_surface_distance(0.0);
    let last = rhumb.interpolate_using_surface_distance(distance);

    assert!((start.longitude - first.longitude).abs() < EPSILON12);
    assert!((start.latitude - first.latitude).abs() < EPSILON12);
    assert!((end.longitude - last.longitude).abs() < EPSILON12);
    assert!((end.latitude - last.latitude).abs() < EPSILON12);
}

#[test]
fn interpolates_midpoint() {
    let start = Cartographic::from_radians(fifteen_degrees(), 0.0, 0.0);
    let end = Cartographic::from_radians(fortyfive_degrees(), 0.0, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);
    let distance = Ellipsoid::WGS84.maximum_radius() * fifteen_degrees();

    let midpoint = rhumb.interpolate_using_surface_distance(distance);

    assert!((thirty_degrees() - midpoint.longitude).abs() < EPSILON12);
    assert!((0.0 - midpoint.latitude).abs() < EPSILON12);
}

#[test]
fn interpolates_start_and_end_points_using_fraction() {
    let start = Cartographic::from_radians(fifteen_degrees(), fifteen_degrees(), 0.0);
    let end = Cartographic::from_radians(thirty_degrees(), thirty_degrees(), 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);

    let first = rhumb.interpolate_using_fraction(0.0);
    let last = rhumb.interpolate_using_fraction(1.0);

    assert!((start.longitude - first.longitude).abs() < EPSILON12);
    assert!((start.latitude - first.latitude).abs() < EPSILON12);
    assert!((end.longitude - last.longitude).abs() < EPSILON12);
    assert!((end.latitude - last.latitude).abs() < EPSILON12);
}

#[test]
fn interpolates_midpoint_using_fraction() {
    let start = Cartographic::from_radians(fifteen_degrees(), 0.0, 0.0);
    let end = Cartographic::from_radians(fortyfive_degrees(), 0.0, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);
    let midpoint = rhumb.interpolate_using_fraction(0.5);

    assert!((thirty_degrees() - midpoint.longitude).abs() < EPSILON12);
    assert!((0.0 - midpoint.latitude).abs() < EPSILON12);
}

#[test]
fn interpolates_when_heading_is_near_90_degrees() {
    let start = Cartographic::from_radians(0.0, 0.0, 0.0);
    let end = Cartographic::from_radians(PI / 2.0, 0.0, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);
    assert!((rhumb.heading() - PI / 2.0).abs() < EPSILON12);

    let midpoint = rhumb.interpolate_using_fraction(0.5);
    assert!((fortyfive_degrees() - midpoint.longitude).abs() < EPSILON12);
    assert!((0.0 - midpoint.latitude).abs() < EPSILON12);
}

#[test]
fn interpolates_when_heading_is_near_0_degrees() {
    let start = Cartographic::from_radians(-three_hundred_degrees(), fifteen_degrees(), 0.0);
    let end = Cartographic::from_radians(-three_hundred_degrees(), fortyfive_degrees(), 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);
    assert!((rhumb.heading() - 0.0).abs() < EPSILON12);

    let midpoint = rhumb.interpolate_using_fraction(0.5);
    assert!((-three_hundred_degrees() - midpoint.longitude).abs() < EPSILON12);
    assert!((thirty_degrees() - midpoint.latitude).abs() < EPSILON3);
}

#[test]
fn finds_midpoint_using_intersection_with_longitude() {
    let start = Cartographic::from_radians(fifteen_degrees(), 0.0, 0.0);
    let end = Cartographic::from_radians(fortyfive_degrees(), thirty_degrees(), 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);

    for fraction in [0.5, 0.1, 0.75, 1.1] {
        let point_interp = rhumb.interpolate_using_fraction(fraction);
        let point_intersect = rhumb.find_intersection_with_longitude(point_interp.longitude).unwrap();
        assert!(
            (point_interp.longitude - point_intersect.longitude).abs() < EPSILON12
                && (point_interp.latitude - point_intersect.latitude).abs() < EPSILON12,
            "fraction {}: interp={:?} intersect={:?}",
            fraction,
            point_interp,
            point_intersect
        );
    }
}

#[test]
fn finds_correct_intersection_with_idl() {
    let start = Cartographic::from_degrees(170.0, 10.0, 0.0);
    let end = Cartographic::from_degrees(-170.0, 23.0, 0.0);

    let mut rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);

    let idl1 = rhumb.find_intersection_with_longitude(-PI).unwrap();
    let idl2 = rhumb.find_intersection_with_longitude(PI).unwrap();

    assert!((idl1.longitude - idl2.longitude).abs() < EPSILON12);
    assert!((idl1.latitude - idl2.latitude).abs() < EPSILON12);
    assert!((idl1.longitude - PI).abs() < EPSILON14);
    assert!((idl2.longitude - PI).abs() < EPSILON14);

    rhumb.set_end_points(&end, &start);

    let idl1 = rhumb.find_intersection_with_longitude(-PI).unwrap();
    let idl2 = rhumb.find_intersection_with_longitude(PI).unwrap();

    assert!((idl1.longitude - idl2.longitude).abs() < EPSILON12);
    assert!((idl1.latitude - idl2.latitude).abs() < EPSILON12);
    assert!((idl1.longitude - (-PI)).abs() < EPSILON14);
    assert!((idl2.longitude - (-PI)).abs() < EPSILON14);
}

#[test]
fn intersection_with_longitude_handles_ew_lines() {
    let start = Cartographic::from_radians(fifteen_degrees(), 0.0, 0.0);
    let end = Cartographic::from_radians(fortyfive_degrees(), 0.0, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);

    let midpoint_interp = rhumb.interpolate_using_fraction(0.5);
    let midpoint_intersect = rhumb.find_intersection_with_longitude(midpoint_interp.longitude).unwrap();
    assert!((midpoint_interp.longitude - midpoint_intersect.longitude).abs() < EPSILON12);
    assert!((midpoint_interp.latitude - midpoint_intersect.latitude).abs() < EPSILON12);
}

#[test]
fn intersection_with_longitude_handles_ns_lines() {
    let start = Cartographic::from_radians(fifteen_degrees(), 0.0, 0.0);
    let end = Cartographic::from_radians(fifteen_degrees(), thirty_degrees(), 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);

    let midpoint_interp = rhumb.interpolate_using_fraction(0.5);
    let result = rhumb.find_intersection_with_longitude(midpoint_interp.longitude);
    assert!(result.is_none());
}

#[test]
fn intersection_with_longitude_handles_ns_lines_with_different_longitude() {
    let start = Cartographic::from_radians(fifteen_degrees(), 0.0, 0.0);
    let end = Cartographic::from_radians(fifteen_degrees(), thirty_degrees(), 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);

    let result = rhumb.find_intersection_with_longitude(thirty_degrees()).unwrap();
    assert!((result.latitude - PI_OVER_TWO).abs() < EPSILON12);
}

#[test]
fn finds_midpoint_using_intersection_with_latitude() {
    let start = Cartographic::from_radians(fifteen_degrees(), 0.0, 0.0);
    let end = Cartographic::from_radians(fortyfive_degrees(), thirty_degrees(), 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);

    for fraction in [0.5, 0.1, 0.75, 1.1] {
        let point_interp = rhumb.interpolate_using_fraction(fraction);
        let point_intersect = rhumb.find_intersection_with_latitude(point_interp.latitude).unwrap();
        assert!(
            (point_interp.longitude - point_intersect.longitude).abs() < EPSILON12
                && (point_interp.latitude - point_intersect.latitude).abs() < EPSILON12,
            "fraction {}: interp={:?} intersect={:?}",
            fraction,
            point_interp,
            point_intersect
        );
    }
}

#[test]
fn intersection_with_latitude_handles_ew_lines() {
    let start = Cartographic::from_radians(fifteen_degrees(), 0.0, 0.0);
    let end = Cartographic::from_radians(fortyfive_degrees(), 0.0, 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);

    let midpoint_interp = rhumb.interpolate_using_fraction(0.5);
    let result = rhumb.find_intersection_with_latitude(midpoint_interp.latitude);
    assert!(result.is_none());
}

#[test]
fn intersection_with_latitude_handles_ns_lines() {
    let start = Cartographic::from_radians(fifteen_degrees(), 0.0, 0.0);
    let end = Cartographic::from_radians(fifteen_degrees(), thirty_degrees(), 0.0);

    let rhumb = EllipsoidRhumbLine::new(&start, &end, &Ellipsoid::WGS84);

    let midpoint_interp = rhumb.interpolate_using_fraction(0.5);
    let midpoint_intersect = rhumb.find_intersection_with_latitude(midpoint_interp.latitude).unwrap();
    assert!((midpoint_interp.longitude - midpoint_intersect.longitude).abs() < EPSILON12);
    assert!((midpoint_interp.latitude - midpoint_intersect.latitude).abs() < EPSILON12);
}

#[test]
fn returns_start_point_when_interpolating_at_distance_zero() {
    let ellipsoid = Ellipsoid::WGS84;
    let p0 = glam::DVec3::new(899411.2767873341, -5079219.747324299, 3738850.924729517);
    let p1 = glam::DVec3::new(899411.0994891181, -5079219.778719673, 3738850.9247295167);

    let c0 = ellipsoid.cartesian_to_cartographic(p0).unwrap();
    let c1 = ellipsoid.cartesian_to_cartographic(p1).unwrap();
    let rhumb = EllipsoidRhumbLine::new(&c0, &c1, &ellipsoid);

    let c = rhumb.interpolate_using_surface_distance(0.0);
    let p = Cartographic::to_cartesian(&c, &ellipsoid);

    assert!((p - p0).length() < EPSILON6 * p0.length());
}
