//! Core/RectangleSpec → Rust 集成测试（对齐实现）。
//!
//! 对齐实现原始 CesiumJS `Specs/Core/RectangleSpec`
//!（112 个 `it()` 用例，包含 `createPackableSpecs` 块）。参考值逐字使用，
//! 以便 Rust 实现针对与 CesiumJS 完全相同的基准真值进行验证。
//!
//! 平台适配（依据验证计划，均有文档说明）：
//! - CesiumJS 的 "works with a result parameter" 变体测试 JS 的内存复用
//!   API 契约（`returnedResult === result`）。Rust 返回拥有所有权的值且没有
//!   result-parameter API，因此这些变体被下方的拥有返回测试所归并
//!  （计算值相同，单一代码路径）。
//! - CesiumJS 的 "throws with no <arg>" 用例测试运行时的 null/undefined 检查。
//!   Rust 的类型系统使 null 参数无法表示（编译期安全），因此这些错误路径
//!   没有对应的 Rust 版本，予以省略。
//! - CesiumJS 的 `Rectangle._validate` / `Rectangle.subsection` 抛出
//!   `DeveloperError`；Rust 移植版返回 `Result<_, String>`，因此
//!   "throws with bad/out-of-range value" 用例确实被移植（以 `is_err()` 形式）。
//! - `clone` 映射到 Rust 派生的 `Clone`；`equals` 映射到派生的 `PartialEq`。

use cesium_geospatial::bounding::BoundingSphere;
use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::math_utils::{to_radians, PI_OVER_TWO, TWO_PI};
use cesium_geospatial::rectangle::Rectangle;
use cesium_specs::{assert_approx, epsilon};
use glam::DVec3;
use std::f64::consts::PI;

// --- 来自原始规范的参考值 ---
const WEST: f64 = -0.9;
const SOUTH: f64 = 0.5;
const EAST: f64 = 1.4;
const NORTH: f64 = 1.0;

fn center() -> Cartographic {
    Cartographic::from_radians((WEST + EAST) / 2.0, (SOUTH + NORTH) / 2.0, 0.0)
}

// --- 构造函数 ---

// "default constructor sets expected values."
#[test]
fn test_default_constructor_sets_expected_values() {
    let rectangle = Rectangle::default();
    assert_eq!(rectangle.west, 0.0);
    assert_eq!(rectangle.south, 0.0);
    assert_eq!(rectangle.north, 0.0);
    assert_eq!(rectangle.east, 0.0);
}

// "constructor sets expected parameter values."
#[test]
fn test_constructor_sets_expected_parameter_values() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    assert_eq!(rectangle.west, WEST);
    assert_eq!(rectangle.south, SOUTH);
    assert_eq!(rectangle.east, EAST);
    assert_eq!(rectangle.north, NORTH);
}

// "computeWidth"
#[test]
fn test_compute_width() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let expected = EAST - WEST;
    assert_eq!(rectangle.width(), expected);

    let rectangle = Rectangle::new(2.0, -1.0, -2.0, 1.0);
    let expected = rectangle.east - rectangle.west + TWO_PI;
    assert_eq!(rectangle.width(), expected);
}

// "computeHeight"
#[test]
fn test_compute_height() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let expected = NORTH - SOUTH;
    assert_eq!(rectangle.height(), expected);
}

// --- fromDegrees / fromRadians ---

// "fromDegrees produces expected values."
//（"works with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_from_degrees_produces_expected_values() {
    let west = -10.0;
    let south = -20.0;
    let east = 10.0;
    let north = 20.0;

    let rectangle = Rectangle::from_degrees(west, south, east, north);
    assert_eq!(rectangle.west, to_radians(west));
    assert_eq!(rectangle.south, to_radians(south));
    assert_eq!(rectangle.east, to_radians(east));
    assert_eq!(rectangle.north, to_radians(north));
}

// "fromRadians produces expected values."
//（"works with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_from_radians_produces_expected_values() {
    let west = -1.0;
    let south = -2.0;
    let east = 1.0;
    let north = 2.0;

    let rectangle = Rectangle::from_radians(west, south, east, north);
    assert_eq!(rectangle.west, west);
    assert_eq!(rectangle.south, south);
    assert_eq!(rectangle.east, east);
    assert_eq!(rectangle.north, north);
}

// --- fromCartographicArray ---

// "fromCartographicArray produces expected values."
//（"works with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_from_cartographic_array_produces_expected_values() {
    let min_lon = Cartographic::from_radians(-0.1, 0.3, 0.0);
    let min_lat = Cartographic::from_radians(0.0, -0.2, 0.0);
    let max_lon = Cartographic::from_radians(0.3, -0.1, 0.0);
    let max_lat = Cartographic::from_radians(0.2, 0.4, 0.0);

    let rectangle = Rectangle::from_cartographic_array(&[min_lat, min_lon, max_lat, max_lon]);
    assert_eq!(rectangle.west, min_lon.longitude);
    assert_eq!(rectangle.south, min_lat.latitude);
    assert_eq!(rectangle.east, max_lon.longitude);
    assert_eq!(rectangle.north, max_lat.latitude);
}

// "fromCartographicArray produces rectangle that crosses IDL."
#[test]
fn test_from_cartographic_array_crosses_idl() {
    let min_lon = Cartographic::from_degrees(-178.0, 3.0, 0.0);
    let min_lat = Cartographic::from_degrees(-179.0, -4.0, 0.0);
    let max_lon = Cartographic::from_degrees(178.0, 3.0, 0.0);
    let max_lat = Cartographic::from_degrees(179.0, 4.0, 0.0);

    let rectangle = Rectangle::from_cartographic_array(&[min_lat, min_lon, max_lat, max_lon]);
    assert_eq!(rectangle.east, min_lon.longitude);
    assert_eq!(rectangle.south, min_lat.latitude);
    assert_eq!(rectangle.west, max_lon.longitude);
    assert_eq!(rectangle.north, max_lat.latitude);
}

// --- fromCartesianArray ---

// "fromCartesianArray produces expected values."
//（"works with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_from_cartesian_array_produces_expected_values() {
    let min_lon = Cartographic::from_radians(-0.1, 0.3, 0.0);
    let min_lat = Cartographic::from_radians(0.0, -0.2, 0.0);
    let max_lon = Cartographic::from_radians(0.3, -0.1, 0.0);
    let max_lat = Cartographic::from_radians(0.2, 0.4, 0.0);

    let wgs84 = Ellipsoid::WGS84;

    let cartesians =
        wgs84.cartographic_array_to_cartesian_array(&[min_lat, min_lon, max_lat, max_lon]);
    let rectangle = Rectangle::from_cartesian_array(&cartesians, &wgs84);
    assert_approx!(rectangle.west, min_lon.longitude, epsilon::EPSILON15);
    assert_approx!(rectangle.south, min_lat.latitude, epsilon::EPSILON15);
    assert_approx!(rectangle.east, max_lon.longitude, epsilon::EPSILON15);
    assert_approx!(rectangle.north, max_lat.latitude, epsilon::EPSILON15);
}

// "fromCartesianArray produces rectangle that crosses IDL."
#[test]
fn test_from_cartesian_array_crosses_idl() {
    let min_lon = Cartographic::from_degrees(-178.0, 3.0, 0.0);
    let min_lat = Cartographic::from_degrees(-179.0, -4.0, 0.0);
    let max_lon = Cartographic::from_degrees(178.0, 3.0, 0.0);
    let max_lat = Cartographic::from_degrees(179.0, 4.0, 0.0);

    let wgs84 = Ellipsoid::WGS84;

    let cartesians =
        wgs84.cartographic_array_to_cartesian_array(&[min_lat, min_lon, max_lat, max_lon]);
    let rectangle = Rectangle::from_cartesian_array(&cartesians, &wgs84);
    assert_eq!(rectangle.east, min_lon.longitude);
    assert_eq!(rectangle.south, min_lat.latitude);
    assert_eq!(rectangle.west, max_lon.longitude);
    assert_eq!(rectangle.north, max_lat.latitude);
}

// --- clone ---

// "clone works without a result parameter."
//（"with a result parameter"、"'this' result parameter" 和 "without
//  rectangle" 变体在 Rust 中已被归并/无法表示：Clone 总是
//  返回一个新的拥有所有权的值，且参数非可选。）
#[test]
fn test_clone() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let returned_result = rectangle.clone();
    assert_eq!(returned_result, rectangle);
}

// --- equals / equalsEpsilon ---

// "Equals works in all cases" + "Static equals works in all cases"
//（Rust 只有单个 `PartialEq`，同时覆盖实例与静态两种形式；
//  `equals(undefined)` 用例无法表示——类型安全的相等。）
#[test]
fn test_equals_works_in_all_cases() {
    let rectangle = Rectangle::new(0.1, 0.2, 0.3, 0.4);
    assert!(rectangle == Rectangle::new(0.1, 0.2, 0.3, 0.4));
    assert!(rectangle != Rectangle::new(0.5, 0.2, 0.3, 0.4));
    assert!(rectangle != Rectangle::new(0.1, 0.5, 0.3, 0.4));
    assert!(rectangle != Rectangle::new(0.1, 0.2, 0.5, 0.4));
    assert!(rectangle != Rectangle::new(0.1, 0.2, 0.3, 0.5));
}

// "Static equals epsilon works in all cases" + "Equals epsilon works in all cases"
//（单个 `equals_epsilon` 覆盖两种形式；`undefined` 用例无法表示。）
#[test]
fn test_equals_epsilon_works_in_all_cases() {
    let rectangle1 = Rectangle::new(0.1, 0.2, 0.3, 0.4);
    assert!(rectangle1.equals_epsilon(&Rectangle::new(0.1, 0.2, 0.3, 0.4), 0.0));
    assert!(!rectangle1.equals_epsilon(&Rectangle::new(0.5, 0.2, 0.3, 0.4), 0.0));
    assert!(!rectangle1.equals_epsilon(&Rectangle::new(0.1, 0.5, 0.3, 0.4), 0.0));
    assert!(!rectangle1.equals_epsilon(&Rectangle::new(0.1, 0.2, 0.5, 0.4), 0.0));
    assert!(!rectangle1.equals_epsilon(&Rectangle::new(0.1, 0.2, 0.3, 0.5), 0.0));
    assert!(rectangle1.equals_epsilon(&Rectangle::new(0.5, 0.2, 0.3, 0.4), 0.4));
    assert!(rectangle1.equals_epsilon(&Rectangle::new(0.1, 0.5, 0.3, 0.4), 0.3));
    assert!(rectangle1.equals_epsilon(&Rectangle::new(0.1, 0.2, 0.5, 0.4), 0.2));
    assert!(rectangle1.equals_epsilon(&Rectangle::new(0.1, 0.2, 0.3, 0.5), 0.1));
    assert!(rectangle1.equals_epsilon(&rectangle1, 0.0));
}

// --- validate ---

// "validate throws with bad west/south/east/north"
//（"throws with no rectangle / no west / no south / no east / no north"
//  用例测试未定义字段的运行时检查——在 Rust 中无法表示，因为
//  所有字段都是必需的 f64。）
#[test]
fn test_validate_ok() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    assert!(rectangle.validate().is_ok());
}

#[test]
fn test_validate_throws_with_bad_west() {
    let rectangle = Rectangle::new(PI * 2.0, SOUTH, EAST, NORTH);
    assert!(rectangle.validate().is_err());
}

#[test]
fn test_validate_throws_with_bad_south() {
    let rectangle = Rectangle::new(WEST, PI * 2.0, EAST, NORTH);
    assert!(rectangle.validate().is_err());
}

#[test]
fn test_validate_throws_with_bad_east() {
    let rectangle = Rectangle::new(WEST, SOUTH, PI * 2.0, NORTH);
    assert!(rectangle.validate().is_err());
}

#[test]
fn test_validate_throws_with_bad_north() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, PI * 2.0);
    assert!(rectangle.validate().is_err());
}

// --- 角点 ---

// "southwest works without a result parameter"
//（"with a result parameter" 和 "throws with no rectangle" 变体已被归并/无法表示。）
#[test]
fn test_southwest() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let returned_result = rectangle.southwest();
    assert_eq!(returned_result.longitude, WEST);
    assert_eq!(returned_result.latitude, SOUTH);
}

// "northwest works without a result parameter"
#[test]
fn test_northwest() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let returned_result = rectangle.northwest();
    assert_eq!(returned_result.longitude, WEST);
    assert_eq!(returned_result.latitude, NORTH);
}

// "northeast works without a result parameter"
#[test]
fn test_northeast() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let returned_result = rectangle.northeast();
    assert_eq!(returned_result.longitude, EAST);
    assert_eq!(returned_result.latitude, NORTH);
}

// "southeast works without a result parameter"
#[test]
fn test_southeast() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let returned_result = rectangle.southeast();
    assert_eq!(returned_result.longitude, EAST);
    assert_eq!(returned_result.latitude, SOUTH);
}

// --- 中心 ---

// "center works without a result parameter"
//（"with a result parameter" 和 "throws with no rectangle" 变体已被归并/无法表示。）
#[test]
fn test_center() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let returned_result = rectangle.center();
    assert!(returned_result.equals_epsilon(&center(), epsilon::EPSILON11));
}

// "center works across IDL"
#[test]
fn test_center_works_across_idl() {
    let rectangle = Rectangle::from_degrees(170.0, 0.0, -170.0, 0.0);
    let returned_result = rectangle.center();
    assert!(returned_result
        .equals_epsilon(&Cartographic::from_degrees(180.0, 0.0, 0.0), epsilon::EPSILON11));

    let rectangle = Rectangle::from_degrees(160.0, 0.0, -170.0, 0.0);
    let returned_result = rectangle.center();
    assert!(returned_result
        .equals_epsilon(&Cartographic::from_degrees(175.0, 0.0, 0.0), epsilon::EPSILON11));

    let rectangle = Rectangle::from_degrees(170.0, 0.0, -160.0, 0.0);
    let returned_result = rectangle.center();
    assert!(returned_result
        .equals_epsilon(&Cartographic::from_degrees(-175.0, 0.0, 0.0), epsilon::EPSILON11));

    let rectangle = Rectangle::from_degrees(160.0, 0.0, 140.0, 0.0);
    let returned_result = rectangle.center();
    assert!(returned_result
        .equals_epsilon(&Cartographic::from_degrees(-30.0, 0.0, 0.0), epsilon::EPSILON11));
}

// --- 相交 ---

// "intersection works without a result parameter"
//（"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_intersection() {
    let rectangle = Rectangle::new(0.5, 0.1, 0.75, 0.9);
    let rectangle2 = Rectangle::new(0.0, 0.25, 1.0, 0.8);
    let expected = Rectangle::new(0.5, 0.25, 0.75, 0.8);
    let returned_result = rectangle.intersection(&rectangle2);
    assert_eq!(returned_result, Some(expected));
}

// "intersection works across the IDL (1)"
#[test]
fn test_intersection_works_across_idl_1() {
    let rectangle1 = Rectangle::from_degrees(170.0, -10.0, -170.0, 10.0);
    let rectangle2 = Rectangle::from_degrees(-175.0, 5.0, -160.0, 15.0);
    let expected = Rectangle::from_degrees(-175.0, 5.0, -170.0, 10.0);
    let r1 = rectangle1.intersection(&rectangle2).unwrap();
    let r2 = rectangle2.intersection(&rectangle1).unwrap();
    assert!((r1.west - expected.west).abs() < 1e-14);
    assert!((r1.south - expected.south).abs() < 1e-14);
    assert!((r1.east - expected.east).abs() < 1e-14);
    assert!((r1.north - expected.north).abs() < 1e-14);
    assert!((r2.west - expected.west).abs() < 1e-14);
    assert!((r2.south - expected.south).abs() < 1e-14);
    assert!((r2.east - expected.east).abs() < 1e-14);
    assert!((r2.north - expected.north).abs() < 1e-14);
}

// "intersection works across the IDL (2)"
#[test]
fn test_intersection_works_across_idl_2() {
    let rectangle1 = Rectangle::from_degrees(170.0, -10.0, -170.0, 10.0);
    let rectangle2 = Rectangle::from_degrees(160.0, 5.0, 175.0, 15.0);
    let expected = Rectangle::from_degrees(170.0, 5.0, 175.0, 10.0);
    assert_eq!(rectangle1.intersection(&rectangle2), Some(expected));
    assert_eq!(rectangle2.intersection(&rectangle1), Some(expected));
}

// "intersection works across the IDL (3)"
#[test]
fn test_intersection_works_across_idl_3() {
    let rectangle1 = Rectangle::from_degrees(170.0, -10.0, -170.0, 10.0);
    let rectangle2 = Rectangle::from_degrees(175.0, 5.0, -175.0, 15.0);
    let expected = Rectangle::from_degrees(175.0, 5.0, -175.0, 10.0);
    assert_eq!(rectangle1.intersection(&rectangle2), Some(expected));
    assert_eq!(rectangle2.intersection(&rectangle1), Some(expected));
}

// "intersection returns undefined for a point"
#[test]
fn test_intersection_returns_none_for_a_point() {
    let rectangle1 = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let rectangle2 = Rectangle::new(EAST, NORTH, EAST + 0.1, NORTH + 0.1);
    assert_eq!(rectangle1.intersection(&rectangle2), None);
    assert_eq!(rectangle2.intersection(&rectangle1), None);
}

// "intersection returns undefined for a east-west line (1)"
#[test]
fn test_intersection_returns_none_for_ew_line_1() {
    let rectangle1 = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let rectangle2 = Rectangle::new(WEST, NORTH, EAST, NORTH + 0.1);
    assert_eq!(rectangle1.intersection(&rectangle2), None);
    assert_eq!(rectangle2.intersection(&rectangle1), None);
}

// "intersection returns undefined for a east-west line (2)"
#[test]
fn test_intersection_returns_none_for_ew_line_2() {
    let rectangle1 = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let rectangle2 = Rectangle::new(WEST, SOUTH + 0.1, EAST, SOUTH);
    assert_eq!(rectangle1.intersection(&rectangle2), None);
    assert_eq!(rectangle2.intersection(&rectangle1), None);
}

// "intersection returns undefined for a north-south line (1)"
#[test]
fn test_intersection_returns_none_for_ns_line_1() {
    let rectangle1 = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let rectangle2 = Rectangle::new(EAST, SOUTH, EAST + 0.1, NORTH);
    assert_eq!(rectangle1.intersection(&rectangle2), None);
    assert_eq!(rectangle2.intersection(&rectangle1), None);
}

// "intersection returns undefined for a north-south line (2)"
#[test]
fn test_intersection_returns_none_for_ns_line_2() {
    let rectangle1 = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let rectangle2 = Rectangle::new(WEST - 0.1, SOUTH, WEST, NORTH);
    assert_eq!(rectangle1.intersection(&rectangle2), None);
    assert_eq!(rectangle2.intersection(&rectangle1), None);
}

// "intersection returns undefined for a north-south line (3)"
#[test]
fn test_intersection_returns_none_for_ns_line_3() {
    let west = to_radians(170.0);
    let south = to_radians(-10.0);
    let east = to_radians(-170.0);
    let north = to_radians(10.0);

    let rectangle1 = Rectangle::new(west, south, east, north);
    let rectangle2 = Rectangle::new(east, south, east + 0.1, north);
    assert_eq!(rectangle1.intersection(&rectangle2), None);
    assert_eq!(rectangle2.intersection(&rectangle1), None);
}

// "intersection returns undefined for a north-south line (4)"
#[test]
fn test_intersection_returns_none_for_ns_line_4() {
    let west = to_radians(170.0);
    let south = to_radians(-10.0);
    let east = to_radians(-170.0);
    let north = to_radians(10.0);

    let rectangle1 = Rectangle::new(west, south, east, north);
    let rectangle2 = Rectangle::new(west - 0.1, south, west, north);
    assert_eq!(rectangle1.intersection(&rectangle2), None);
    assert_eq!(rectangle2.intersection(&rectangle1), None);
}

// "intersection returns undefined if north-south direction is degenerate"
#[test]
fn test_intersection_returns_none_ns_degenerate() {
    let rectangle1 = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let rectangle2 = Rectangle::new(WEST, NORTH + 0.1, EAST, NORTH + 0.2);
    assert_eq!(rectangle1.intersection(&rectangle2), None);
    assert_eq!(rectangle2.intersection(&rectangle1), None);
}

// "intersection returns undefined if east-west direction is degenerate"
#[test]
fn test_intersection_returns_none_ew_degenerate() {
    let rectangle1 = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let rectangle2 = Rectangle::new(EAST + 0.1, SOUTH, EAST + 0.2, NORTH);
    assert_eq!(rectangle1.intersection(&rectangle2), None);
    assert_eq!(rectangle2.intersection(&rectangle1), None);
}

// --- 并集 ---

// "union works without a result parameter"
//（"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_union() {
    let rectangle1 = Rectangle::new(0.5, 0.1, 0.75, 0.9);
    let rectangle2 = Rectangle::new(0.4, 0.0, 0.85, 0.8);
    let expected = Rectangle::new(0.4, 0.0, 0.85, 0.9);
    let returned_result = rectangle1.union(&rectangle2);
    assert_eq!(returned_result, expected);
}

// "union works with first rectangle crossing the IDL"
#[test]
fn test_union_first_rectangle_crossing_idl() {
    let rectangle1 = Rectangle::new(0.5, 0.1, -0.5, 0.9);
    let rectangle2 = Rectangle::new(-0.85, 0.0, -0.4, 0.8);
    let expected = Rectangle::new(0.5, 0.0, -0.4, 0.9);
    let returned_result = rectangle1.union(&rectangle2);
    assert!(returned_result.equals_epsilon(&expected, epsilon::EPSILON15));
}

// "union works with second rectangle crossing the IDL"
#[test]
fn test_union_second_rectangle_crossing_idl() {
    let rectangle1 = Rectangle::new(0.5, 0.1, 0.75, 0.9);
    let rectangle2 = Rectangle::new(0.6, 0.0, -0.2, 0.8);
    let expected = Rectangle::new(0.5, 0.0, -0.2, 0.9);
    let returned_result = rectangle1.union(&rectangle2);
    assert!(returned_result.equals_epsilon(&expected, epsilon::EPSILON15));
}

// "union works with both rectangles crossing the IDL"
#[test]
fn test_union_both_rectangles_crossing_idl() {
    let rectangle1 = Rectangle::new(0.5, 0.1, -0.4, 0.9);
    let rectangle2 = Rectangle::new(0.4, 0.0, -0.5, 0.8);
    let expected = Rectangle::new(0.4, 0.0, -0.4, 0.9);
    let returned_result = rectangle1.union(&rectangle2);
    assert!(returned_result.equals_epsilon(&expected, epsilon::EPSILON15));
}

// "union works with rectangles that span the entire globe"
#[test]
fn test_union_rectangles_span_entire_globe() {
    let rectangle1 = Rectangle::new(-PI, -PI_OVER_TWO, PI, 0.0);
    let rectangle2 = Rectangle::new(-PI, 0.0, PI, PI_OVER_TWO);
    let expected = Rectangle::new(-PI, -PI_OVER_TWO, PI, PI_OVER_TWO);
    let returned_result = rectangle1.union(&rectangle2);
    assert!(returned_result.equals_epsilon(&expected, epsilon::EPSILON15));
}

// --- 扩展 ---

// "expand works if rectangle needs to grow right"
#[test]
fn test_expand_grow_right() {
    let rectangle = Rectangle::new(0.5, 0.1, 0.75, 0.9);
    let cartographic = Cartographic::from_radians(0.85, 0.5, 0.0);
    let expected = Rectangle::new(0.5, 0.1, 0.85, 0.9);
    let result = rectangle.expand(&cartographic);
    assert_eq!(result, expected);
}

// "expand works if rectangle needs to grow left"
#[test]
fn test_expand_grow_left() {
    let rectangle = Rectangle::new(0.5, 0.1, 0.75, 0.9);
    let cartographic = Cartographic::from_radians(0.4, 0.5, 0.0);
    let expected = Rectangle::new(0.4, 0.1, 0.75, 0.9);
    let result = rectangle.expand(&cartographic);
    assert_eq!(result, expected);
}

// "expand works if rectangle needs to grow up"
#[test]
fn test_expand_grow_up() {
    let rectangle = Rectangle::new(0.5, 0.1, 0.75, 0.9);
    let cartographic = Cartographic::from_radians(0.6, 1.0, 0.0);
    let expected = Rectangle::new(0.5, 0.1, 0.75, 1.0);
    let result = rectangle.expand(&cartographic);
    assert_eq!(result, expected);
}

// "expand works if rectangle needs to grow down"
#[test]
fn test_expand_grow_down() {
    let rectangle = Rectangle::new(0.5, 0.1, 0.75, 0.9);
    let cartographic = Cartographic::from_radians(0.6, 0.0, 0.0);
    let expected = Rectangle::new(0.5, 0.0, 0.75, 0.9);
    let result = rectangle.expand(&cartographic);
    assert_eq!(result, expected);
}

// "expand works if rectangle does not need to grow"
#[test]
fn test_expand_no_growth_needed() {
    let rectangle = Rectangle::new(0.5, 0.1, 0.75, 0.9);
    let cartographic = Cartographic::from_radians(0.6, 0.5, 0.0);
    let expected = Rectangle::new(0.5, 0.1, 0.75, 0.9);
    let result = rectangle.expand(&cartographic);
    assert_eq!(result, expected);
}

// "expand works with a result parameter" — 已被归并（拥有返回）。原用例在此处
// 同时向东和向北扩展；验证相同的组合扩展。
#[test]
fn test_expand_grow_right_and_up() {
    let rectangle = Rectangle::new(0.5, 0.1, 0.75, 0.9);
    let cartographic = Cartographic::from_radians(0.85, 1.0, 0.0);
    let expected = Rectangle::new(0.5, 0.1, 0.85, 1.0);
    let result = rectangle.expand(&cartographic);
    assert_eq!(result, expected);
}

// --- 包含 ---

// "contains works"
#[test]
fn test_contains() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    assert!(rectangle.contains(WEST, SOUTH));
    assert!(rectangle.contains(WEST, NORTH));
    assert!(rectangle.contains(EAST, SOUTH));
    assert!(rectangle.contains(EAST, NORTH));
    let c = rectangle.center();
    assert!(rectangle.contains(c.longitude, c.latitude));
    assert!(!rectangle.contains(WEST - 0.1, SOUTH));
    assert!(!rectangle.contains(WEST, NORTH + 0.1));
    assert!(!rectangle.contains(EAST, SOUTH - 0.1));
    assert!(!rectangle.contains(EAST + 0.1, NORTH));
}

// "contains works with rectangle across the IDL"
#[test]
fn test_contains_with_rectangle_across_idl() {
    let west = to_radians(170.0);
    let south = to_radians(-10.0);
    let east = to_radians(-170.0);
    let north = to_radians(10.0);

    let rectangle = Rectangle::new(west, south, east, north);
    assert!(rectangle.contains(west, south));
    assert!(rectangle.contains(west, north));
    assert!(rectangle.contains(east, south));
    assert!(rectangle.contains(east, north));
    let c = rectangle.center();
    assert!(rectangle.contains(c.longitude, c.latitude));
    assert!(!rectangle.contains(west - 0.1, south));
    assert!(!rectangle.contains(west, north + 0.1));
    assert!(!rectangle.contains(east, south - 0.1));
    assert!(!rectangle.contains(east + 0.1, north));
}

// --- 子采样 ---

// "subsample works south of the equator"
//（"works with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_subsample_works_south_of_the_equator() {
    let west = 0.1;
    let south = -0.3;
    let east = 0.2;
    let north = -0.4;
    let rectangle = Rectangle::new(west, south, east, north);
    let wgs84 = Ellipsoid::WGS84;
    let returned_result = rectangle.subsample(&wgs84, 0.0);
    let expected = vec![
        wgs84.cartographic_to_cartesian(&rectangle.northwest()),
        wgs84.cartographic_to_cartesian(&rectangle.northeast()),
        wgs84.cartographic_to_cartesian(&rectangle.southeast()),
        wgs84.cartographic_to_cartesian(&rectangle.southwest()),
    ];
    assert_eq!(returned_result, expected);
}

// "subsample works north of the equator"
#[test]
fn test_subsample_works_north_of_the_equator() {
    let west = 0.1;
    let south = 0.3;
    let east = 0.2;
    let north = 0.4;
    let rectangle = Rectangle::new(west, south, east, north);
    let wgs84 = Ellipsoid::WGS84;
    let returned_result = rectangle.subsample(&wgs84, 0.0);
    let expected = vec![
        wgs84.cartographic_to_cartesian(&rectangle.northwest()),
        wgs84.cartographic_to_cartesian(&rectangle.northeast()),
        wgs84.cartographic_to_cartesian(&rectangle.southeast()),
        wgs84.cartographic_to_cartesian(&rectangle.southwest()),
    ];
    assert_eq!(returned_result, expected);
}

// "subsample works on the equator"
#[test]
fn test_subsample_works_on_the_equator() {
    let west = 0.1;
    let south = -0.1;
    let east = 0.2;
    let north = 0.0;
    let rectangle = Rectangle::new(west, south, east, north);
    let wgs84 = Ellipsoid::WGS84;
    let returned_result = rectangle.subsample(&wgs84, 0.0);
    assert_eq!(returned_result.len(), 6);
    assert_eq!(
        returned_result[0],
        wgs84.cartographic_to_cartesian(&rectangle.northwest())
    );
    assert_eq!(
        returned_result[1],
        wgs84.cartographic_to_cartesian(&rectangle.northeast())
    );
    assert_eq!(
        returned_result[2],
        wgs84.cartographic_to_cartesian(&rectangle.southeast())
    );
    assert_eq!(
        returned_result[3],
        wgs84.cartographic_to_cartesian(&rectangle.southwest())
    );

    let cartographic4 = wgs84.cartesian_to_cartographic(returned_result[4]).unwrap();
    assert_eq!(cartographic4.latitude, 0.0);
    assert_approx!(cartographic4.longitude, west, epsilon::EPSILON16);

    let cartographic5 = wgs84.cartesian_to_cartographic(returned_result[5]).unwrap();
    assert_eq!(cartographic5.latitude, 0.0);
    assert_approx!(cartographic5.longitude, east, epsilon::EPSILON16);
}

// "subsample works at a height above the ellipsoid"
#[test]
fn test_subsample_works_at_a_height_above_the_ellipsoid() {
    let west = 0.1;
    let south = -0.3;
    let east = 0.2;
    let north = -0.4;
    let rectangle = Rectangle::new(west, south, east, north);
    let height = 100000.0;
    let wgs84 = Ellipsoid::WGS84;
    let returned_result = rectangle.subsample(&wgs84, height);

    let mut nw = rectangle.northwest();
    nw.height = height;
    let mut ne = rectangle.northeast();
    ne.height = height;
    let mut se = rectangle.southeast();
    se.height = height;
    let mut sw = rectangle.southwest();
    sw.height = height;

    let expected = vec![
        wgs84.cartographic_to_cartesian(&nw),
        wgs84.cartographic_to_cartesian(&ne),
        wgs84.cartographic_to_cartesian(&se),
        wgs84.cartographic_to_cartesian(&sw),
    ];
    assert_eq!(returned_result, expected);
}

// --- 子区域 ---

// "subsection works with a result parameter" + "subsection works with no result parameter"
//（已合并：Rust 返回拥有所有权的值，单一代码路径）
#[test]
fn test_subsection() {
    let west = 0.0;
    let east = 0.5;
    let south = 0.0;
    let north = 0.5;
    let rectangle = Rectangle::new(west, south, east, north);

    let west_lerp = 0.25;
    let east_lerp = 0.75;
    let south_lerp = 0.25;
    let north_lerp = 0.75;

    let expected = Rectangle::new(0.125, 0.125, 0.375, 0.375);
    let subsection = rectangle.subsection(west_lerp, south_lerp, east_lerp, north_lerp).unwrap();
    assert_eq!(subsection, expected);
}

// "subsection works with empty range"
#[test]
fn test_subsection_empty_range() {
    let west = 0.0;
    let east = 0.5;
    let south = 0.0;
    let north = 0.5;
    let rectangle = Rectangle::new(west, south, east, north);

    let expected = Rectangle::new(west, south, west, south);
    let subsection = rectangle.subsection(0.0, 0.0, 0.0, 0.0).unwrap();
    assert_eq!(subsection, expected);
}

// "subsection works with full range"
#[test]
fn test_subsection_full_range() {
    let west = 0.1;
    let east = 0.9;
    let south = 0.1;
    let north = 0.9;
    let rectangle = Rectangle::new(west, south, east, north);

    let expected = Rectangle::new(west, south, east, north);
    let subsection = rectangle.subsection(0.0, 0.0, 1.0, 1.0).unwrap();
    assert_eq!(subsection, expected);
}

// "subsection works with zero area rectangle"
#[test]
fn test_subsection_zero_area_rectangle() {
    let west = 0.1;
    let east = 0.1;
    let south = 0.1;
    let north = 0.1;
    let rectangle = Rectangle::new(west, south, east, north);

    // 由于该矩形面积为零，这些值不应对最终结果
    // 产生任何影响。
    let expected = Rectangle::new(west, south, east, north);
    let subsection = rectangle.subsection(0.22, 0.22, 0.88, 0.88).unwrap();
    assert_eq!(subsection, expected);
}

// "subsection works with rectangle that crosses IDL and subsection that crosses IDL"
#[test]
fn test_subsection_idl_crosses_idl() {
    let west = to_radians(45.0);
    let east = to_radians(-45.0);
    let south = to_radians(-90.0);
    let north = to_radians(90.0);
    let rectangle = Rectangle::new(west, south, east, north);

    let expected = Rectangle::new(to_radians(112.5), south, to_radians(-112.5), north);
    let subsection = rectangle.subsection(0.25, 0.0, 0.75, 1.0).unwrap();
    assert!(subsection.equals_epsilon(&expected, epsilon::EPSILON14));
}

// "subsection works with rectangle that crosses IDL and subsection that doesn't cross IDL"
#[test]
fn test_subsection_idl_not_crosses_idl() {
    let west = to_radians(45.0);
    let east = to_radians(-45.0);
    let south = to_radians(-90.0);
    let north = to_radians(90.0);
    let rectangle = Rectangle::new(west, south, east, north);

    let expected = Rectangle::new(to_radians(-112.5), south, east, north);
    let subsection = rectangle.subsection(0.75, 0.0, 1.0, 1.0).unwrap();
    assert!(subsection.equals_epsilon(&expected, epsilon::EPSILON14));
}

// "subsection works with rectangle that crosses IDL and subsection with full range"
#[test]
fn test_subsection_idl_full_range() {
    let west = to_radians(45.0);
    let east = to_radians(-45.0);
    let south = to_radians(-90.0);
    let north = to_radians(90.0);
    let rectangle = Rectangle::new(west, south, east, north);

    let expected = Rectangle::new(west, south, east, north);
    let subsection = rectangle.subsection(0.0, 0.0, 1.0, 1.0).unwrap();
    assert!(subsection.equals_epsilon(&expected, epsilon::EPSILON14));
}

// "subsection throws with out of range westLerp"
#[test]
fn test_subsection_throws_out_of_range_west_lerp() {
    let rectangle = Rectangle::default();
    assert!(rectangle.subsection(-0.1, 0.0, 0.0, 0.0).is_err());
    assert!(rectangle.subsection(1.1, 0.0, 0.0, 0.0).is_err());
    assert!(rectangle.subsection(0.5, 0.0, 0.4, 0.0).is_err());
}

// "subsection throws with out of range southLerp"
#[test]
fn test_subsection_throws_out_of_range_south_lerp() {
    let rectangle = Rectangle::default();
    assert!(rectangle.subsection(0.0, -0.1, 0.0, 0.0).is_err());
    assert!(rectangle.subsection(0.0, 1.1, 0.0, 0.0).is_err());
    assert!(rectangle.subsection(0.0, 0.5, 0.0, 0.4).is_err());
}

// "subsection throws with out of range eastLerp"
#[test]
fn test_subsection_throws_out_of_range_east_lerp() {
    let rectangle = Rectangle::default();
    assert!(rectangle.subsection(0.0, 0.0, -0.1, 0.0).is_err());
    assert!(rectangle.subsection(0.0, 0.0, 1.1, 0.0).is_err());
    assert!(rectangle.subsection(0.5, 0.0, 0.4, 0.0).is_err());
}

// "subsection throws with out of range northLerp"
#[test]
fn test_subsection_throws_out_of_range_north_lerp() {
    let rectangle = Rectangle::default();
    assert!(rectangle.subsection(0.0, 0.0, 0.0, -0.1).is_err());
    assert!(rectangle.subsection(0.0, 0.0, 0.0, 1.1).is_err());
    assert!(rectangle.subsection(0.0, 0.5, 0.0, 0.4).is_err());
}

// --- fromBoundingSphere ---

// "fromBoundingSphere works with zero values"
#[test]
fn test_from_bounding_sphere_zero_values() {
    let bounding_sphere = BoundingSphere::default();
    let result = Rectangle::from_bounding_sphere(&bounding_sphere, &Ellipsoid::WGS84);
    let expected = Rectangle::MAX_VALUE;
    assert!(result.equals_epsilon(&expected, epsilon::EPSILON14));
}

// "fromBoundingSphere works with non-zero values"
#[test]
#[allow(clippy::excessive_precision)]
fn test_from_bounding_sphere_non_zero_values() {
    let bounding_sphere = BoundingSphere::new(DVec3::new(10000000.0, 0.0, 0.0), 1000.0);
    let result = Rectangle::from_bounding_sphere(&bounding_sphere, &Ellipsoid::WGS84);
    let expected = Rectangle::new(
        -0.00009999999966666667,
        -0.00010042880729608389,
        0.00009999999966666667,
        0.00010042880729608389,
    );
    assert!(result.equals_epsilon(&expected, epsilon::EPSILON14));
}

// "fromBoundingSphere works with bounding sphere centered at the poles"
#[test]
fn test_from_bounding_sphere_centered_at_poles() {
    let wgs84 = Ellipsoid::WGS84;
    let bounding_sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, wgs84.radii().z), 1000.0);
    let result = Rectangle::from_bounding_sphere(&bounding_sphere, &wgs84);
    let expected = Rectangle::new(-PI_OVER_TWO, 1.5706400668742968, PI, PI_OVER_TWO);
    assert!(result.equals_epsilon(&expected, epsilon::EPSILON14));
}

// --- packable (createPackableSpecs) ---

fn packed_instance() -> [f64; 4] {
    [WEST, SOUTH, EAST, NORTH]
}

// "can pack"
#[test]
fn test_can_pack() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let packed = rectangle.pack();
    assert_eq!(packed, packed_instance());
    assert_eq!(packed.len(), Rectangle::PACKED_LENGTH);
}

// "can roundtrip"
#[test]
fn test_can_roundtrip() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let packed = rectangle.pack();
    let unpacked = Rectangle::unpack(&packed, 0);
    assert_eq!(unpacked, rectangle);
}

// "can unpack"
#[test]
fn test_can_unpack() {
    let packed = packed_instance();
    let unpacked = Rectangle::unpack(&packed, 0);
    assert_eq!(unpacked, Rectangle::new(WEST, SOUTH, EAST, NORTH));
}

// "can pack with startingIndex"
#[test]
fn test_can_pack_with_starting_index() {
    let rectangle = Rectangle::new(WEST, SOUTH, EAST, NORTH);
    let mut packed = [0.0_f64; 5];
    rectangle.pack_into(&mut packed, 1);
    assert_eq!(packed[0], 0.0);
    assert_eq!(&packed[1..5], &packed_instance());
}

// "can unpack with startingIndex"
#[test]
fn test_can_unpack_with_starting_index() {
    let mut packed = [0.0_f64; 5];
    packed[1] = WEST;
    packed[2] = SOUTH;
    packed[3] = EAST;
    packed[4] = NORTH;
    let unpacked = Rectangle::unpack(&packed, 1);
    assert_eq!(unpacked, Rectangle::new(WEST, SOUTH, EAST, NORTH));
}
