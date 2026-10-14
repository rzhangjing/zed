//! Core/EllipsoidSpec → Rust 集成测试（对齐实现）。
//!
//! 对齐实现原始 CesiumJS `Specs/Core/EllipsoidSpec`
//! （67 个 `it()` 用例 + createPackableSpecs）。原始 STK-Components 参考值
//! 逐字沿用，从而针对与 CesiumJS 完全相同的基准真值验证 Rust 实现。
//!
//! 平台适配（按验证计划均有文档说明）：
//! - CesiumJS "works with a result parameter" 变体测试的是 JS 内存复用
//!   API 契约（`returnedResult === result`）。Rust 返回拥有所有权的值且没有
//!   result-parameter API，因此这些变体被下方拥有返回值的
//!   测试归并（计算数值完全相同，单一代码路径）。
//! - CesiumJS "throws with no <arg>" 用例测试运行时空值检查。Rust 的类型
//!   系统使空参数无法表示（编译期安全），因此这些
//!   错误路径没有 Rust 对应版本。
//! - `Ellipsoid.default` 静态可变 setter 是 JS 全局状态模式，在 Rust 中
//!   没有对应版本（椭球体都是显式传入）。
//! - `geocentricSurfaceNormal === Cartesian3.normalize`（函数同一性）被
//!   适配为行为测试（返回归一化后的向量）。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::math_utils::*;
use cesium_geospatial::rectangle::Rectangle;
use cesium_specs::{assert_approx, assert_vec2_epsilon, assert_vec3_epsilon, epsilon};
use glam::{DVec2, DVec3};

// --- 来自原始规范的参考值（使用 STK Components 计算）---

fn radii() -> DVec3 {
    DVec3::new(1.0, 2.0, 3.0)
}
fn radii_squared() -> DVec3 {
    DVec3::new(1.0, 4.0, 9.0)
}
fn radii_to_the_fourth() -> DVec3 {
    DVec3::new(1.0, 16.0, 81.0)
}
fn one_over_radii() -> DVec3 {
    DVec3::new(1.0, 0.5, 1.0 / 3.0)
}
fn one_over_radii_squared() -> DVec3 {
    DVec3::new(1.0, 0.25, 1.0 / 9.0)
}
const MINIMUM_RADIUS: f64 = 1.0;
const MAXIMUM_RADIUS: f64 = 3.0;

#[allow(clippy::excessive_precision)]
fn space_cartesian() -> DVec3 {
    DVec3::new(4582719.8827300891, -4582719.8827300882, 1725510.4250797231)
}
#[allow(clippy::excessive_precision)]
fn space_cartesian_geodetic_surface_normal() -> DVec3 {
    DVec3::new(
        0.6829975339864266,
        -0.68299753398642649,
        0.25889908678270795,
    )
}
fn space_cartographic() -> Cartographic {
    Cartographic::from_radians(to_radians(-45.0), to_radians(15.0), 330000.0)
}
#[allow(clippy::excessive_precision)]
fn space_cartographic_geodetic_surface_normal() -> DVec3 {
    DVec3::new(
        0.68301270189221941,
        -0.6830127018922193,
        0.25881904510252074,
    )
}
#[allow(clippy::excessive_precision)]
fn surface_cartesian() -> DVec3 {
    DVec3::new(4094327.7921465295, 1909216.4044747739, 4487348.4088659193)
}
fn surface_cartographic() -> Cartographic {
    Cartographic::from_radians(to_radians(25.0), to_radians(45.0), 0.0)
}

// --- 构造函数 / 派生字段 ---

// "default constructor creates zero Ellipsoid"
#[test]
fn test_default_constructor_creates_zero_ellipsoid() {
    let e = Ellipsoid::new(0.0, 0.0, 0.0);
    assert_eq!(e.radii(), DVec3::ZERO);
    assert_eq!(e.radii_squared(), DVec3::ZERO);
    assert_eq!(e.radii_to_the_fourth(), DVec3::ZERO);
    assert_eq!(e.one_over_radii(), DVec3::ZERO);
    assert_eq!(e.one_over_radii_squared(), DVec3::ZERO);
    assert_eq!(e.minimum_radius(), 0.0);
    assert_eq!(e.maximum_radius(), 0.0);
}

// "fromCartesian3 creates zero Ellipsoid with no parameters"
//（JS 无参 fromCartesian3 == 零半径；Rust 等价形式为 from_cartesian3(ZERO)）
#[test]
fn test_from_cartesian3_creates_zero_ellipsoid() {
    let e = Ellipsoid::from_cartesian3(DVec3::ZERO);
    assert_eq!(e.radii(), DVec3::ZERO);
    assert_eq!(e.radii_squared(), DVec3::ZERO);
    assert_eq!(e.radii_to_the_fourth(), DVec3::ZERO);
    assert_eq!(e.one_over_radii(), DVec3::ZERO);
    assert_eq!(e.one_over_radii_squared(), DVec3::ZERO);
    assert_eq!(e.minimum_radius(), 0.0);
    assert_eq!(e.maximum_radius(), 0.0);
}

// "constructor computes correct values"
#[test]
fn test_constructor_computes_correct_values() {
    let e = Ellipsoid::new(radii().x, radii().y, radii().z);
    assert_eq!(e.radii(), radii());
    assert_eq!(e.radii_squared(), radii_squared());
    assert_eq!(e.radii_to_the_fourth(), radii_to_the_fourth());
    assert_eq!(e.one_over_radii(), one_over_radii());
    assert_eq!(e.one_over_radii_squared(), one_over_radii_squared());
    assert_eq!(e.minimum_radius(), MINIMUM_RADIUS);
    assert_eq!(e.maximum_radius(), MAXIMUM_RADIUS);
}

// "fromCartesian3 computes correct values"
#[test]
fn test_from_cartesian3_computes_correct_values() {
    let e = Ellipsoid::from_cartesian3(radii());
    assert_eq!(e.radii(), radii());
    assert_eq!(e.radii_squared(), radii_squared());
    assert_eq!(e.radii_to_the_fourth(), radii_to_the_fourth());
    assert_eq!(e.one_over_radii(), one_over_radii());
    assert_eq!(e.one_over_radii_squared(), one_over_radii_squared());
    assert_eq!(e.minimum_radius(), MINIMUM_RADIUS);
    assert_eq!(e.maximum_radius(), MAXIMUM_RADIUS);
}

// --- 大地表面法线 ---

// "geodeticSurfaceNormalCartographic works without a result parameter"
// （"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_geodetic_surface_normal_cartographic() {
    let e = Ellipsoid::WGS84;
    let result = e.geodetic_surface_normal_cartographic(&space_cartographic());
    assert_vec3_epsilon!(
        result,
        space_cartographic_geodetic_surface_normal(),
        epsilon::EPSILON15
    );
}

// "geodeticSurfaceNormal works without a result parameter"
// （"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_geodetic_surface_normal() {
    let e = Ellipsoid::WGS84;
    let result = e.geodetic_surface_normal(space_cartesian()).unwrap();
    assert_vec3_epsilon!(
        result,
        space_cartesian_geodetic_surface_normal(),
        epsilon::EPSILON15
    );
}

// "geodeticSurfaceNormal returns undefined when given the origin"
#[test]
fn test_geodetic_surface_normal_returns_none_at_origin() {
    let e = Ellipsoid::WGS84;
    assert!(e.geodetic_surface_normal(DVec3::ZERO).is_none());
}

// --- cartographicToCartesian / cartesianToCartographic ---

// "cartographicToCartesian works without a result parameter"
// （"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_cartographic_to_cartesian() {
    let e = Ellipsoid::WGS84;
    let result = e.cartographic_to_cartesian(&space_cartographic());
    assert_vec3_epsilon!(result, space_cartesian(), epsilon::EPSILON7);
}

// "cartographicArrayToCartesianArray works without a result parameter"
// （"with a result parameter" 变体已被归并：Rust 返回拥有所有权的 Vec）
#[test]
fn test_cartographic_array_to_cartesian_array() {
    let e = Ellipsoid::WGS84;
    let result = e.cartographic_array_to_cartesian_array(&[space_cartographic(), surface_cartographic()]);
    assert_eq!(result.len(), 2);
    assert_vec3_epsilon!(result[0], space_cartesian(), epsilon::EPSILON7);
    assert_vec3_epsilon!(result[1], surface_cartesian(), epsilon::EPSILON7);
}

// "cartesianToCartographic works without a result parameter"
// （"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_cartesian_to_cartographic() {
    let e = Ellipsoid::WGS84;
    let result = e.cartesian_to_cartographic(surface_cartesian()).unwrap();
    assert!(result.equals_epsilon(&surface_cartographic(), epsilon::EPSILON8));
}

// "cartesianToCartographic works close to center"
// 原始规范使用 toEqual（精确相等）——验证逐位一致的浮点路径。
#[test]
#[allow(clippy::excessive_precision)]
fn test_cartesian_to_cartographic_close_to_center() {
    let result = Ellipsoid::WGS84
        .cartesian_to_cartographic(DVec3::new(1e-50, 1e-60, 1e-70))
        .unwrap();
    assert_eq!(result.longitude, 9.999999999999999e-11);
    assert_eq!(result.latitude, 1.0067394967422763e-20);
    assert_eq!(result.height, -6378137.0);
}

// "cartesianToCartographic return undefined very close to center"
#[test]
fn test_cartesian_to_cartographic_none_very_close_to_center() {
    let e = Ellipsoid::WGS84;
    assert!(e
        .cartesian_to_cartographic(DVec3::new(1e-150, 1e-150, 1e-150))
        .is_none());
}

// "cartesianToCartographic return undefined at center"
#[test]
fn test_cartesian_to_cartographic_none_at_center() {
    let e = Ellipsoid::WGS84;
    assert!(e.cartesian_to_cartographic(DVec3::ZERO).is_none());
}

// "cartesianArrayToCartographicArray works without a result parameter"
// （"with a result parameter" 变体已被归并：Rust 返回拥有所有权的 Vec）
#[test]
fn test_cartesian_array_to_cartographic_array() {
    let e = Ellipsoid::WGS84;
    let result = e.cartesian_array_to_cartographic_array(&[space_cartesian(), surface_cartesian()]);
    assert_eq!(result.len(), 2);
    assert!(result[0]
        .unwrap()
        .equals_epsilon(&space_cartographic(), epsilon::EPSILON7));
    assert!(result[1]
        .unwrap()
        .equals_epsilon(&surface_cartographic(), epsilon::EPSILON7));
}

// --- scaleToGeodeticSurface ---

// "scaleToGeodeticSurface scaled in the x direction"
#[test]
fn test_scale_to_geodetic_surface_x() {
    let e = Ellipsoid::new(1.0, 2.0, 3.0);
    let result = e.scale_to_geodetic_surface(DVec3::new(9.0, 0.0, 0.0)).unwrap();
    assert_eq!(result, DVec3::new(1.0, 0.0, 0.0));
}

// "scaleToGeodeticSurface scaled in the y direction"
#[test]
fn test_scale_to_geodetic_surface_y() {
    let e = Ellipsoid::new(1.0, 2.0, 3.0);
    let result = e.scale_to_geodetic_surface(DVec3::new(0.0, 8.0, 0.0)).unwrap();
    assert_eq!(result, DVec3::new(0.0, 2.0, 0.0));
}

// "scaleToGeodeticSurface scaled in the z direction"
#[test]
fn test_scale_to_geodetic_surface_z() {
    let e = Ellipsoid::new(1.0, 2.0, 3.0);
    let result = e.scale_to_geodetic_surface(DVec3::new(0.0, 0.0, 8.0)).unwrap();
    assert_eq!(result, DVec3::new(0.0, 0.0, 3.0));
}

// "scaleToGeodeticSurface works without a result parameter"
// （"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
#[allow(clippy::excessive_precision)]
fn test_scale_to_geodetic_surface_general() {
    let e = Ellipsoid::new(1.0, 2.0, 3.0);
    let expected = DVec3::new(0.2680893773941855, 1.1160466902266495, 2.3559801120411263);
    let result = e.scale_to_geodetic_surface(DVec3::new(4.0, 5.0, 6.0)).unwrap();
    assert_vec3_epsilon!(result, expected, epsilon::EPSILON16);
}

// "scaleToGeodeticSurface returns undefined at center"
#[test]
fn test_scale_to_geodetic_surface_none_at_center() {
    let e = Ellipsoid::new(1.0, 2.0, 3.0);
    assert!(e.scale_to_geodetic_surface(DVec3::ZERO).is_none());
}

// --- scaleToGeocentricSurface ---

// "scaleToGeocentricSurface scaled in the x direction"
#[test]
fn test_scale_to_geocentric_surface_x() {
    let e = Ellipsoid::new(1.0, 2.0, 3.0);
    let result = e.scale_to_geocentric_surface(DVec3::new(9.0, 0.0, 0.0)).unwrap();
    assert_eq!(result, DVec3::new(1.0, 0.0, 0.0));
}

// "scaleToGeocentricSurface scaled in the y direction"
#[test]
fn test_scale_to_geocentric_surface_y() {
    let e = Ellipsoid::new(1.0, 2.0, 3.0);
    let result = e.scale_to_geocentric_surface(DVec3::new(0.0, 8.0, 0.0)).unwrap();
    assert_eq!(result, DVec3::new(0.0, 2.0, 0.0));
}

// "scaleToGeocentricSurface scaled in the z direction"
#[test]
fn test_scale_to_geocentric_surface_z() {
    let e = Ellipsoid::new(1.0, 2.0, 3.0);
    let result = e.scale_to_geocentric_surface(DVec3::new(0.0, 0.0, 8.0)).unwrap();
    assert_eq!(result, DVec3::new(0.0, 0.0, 3.0));
}

// "scaleToGeocentricSurface works without a result parameter"
// （"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
#[allow(clippy::excessive_precision)]
fn test_scale_to_geocentric_surface_general() {
    let e = Ellipsoid::new(1.0, 2.0, 3.0);
    let expected = DVec3::new(0.7807200583588266, 0.9759000729485333, 1.1710800875382399);
    let result = e.scale_to_geocentric_surface(DVec3::new(4.0, 5.0, 6.0)).unwrap();
    assert_vec3_epsilon!(result, expected, epsilon::EPSILON16);
}

// --- transformPositionToScaledSpace / FromScaledSpace ---

// "transformPositionToScaledSpace works without a result parameter"
// （"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_transform_position_to_scaled_space() {
    let e = Ellipsoid::new(2.0, 3.0, 4.0);
    let result = e.transform_position_to_scaled_space(DVec3::new(4.0, 6.0, 8.0));
    assert_vec3_epsilon!(result, DVec3::new(2.0, 2.0, 2.0), epsilon::EPSILON16);
}

// "transformPositionFromScaledSpace works without a result parameter"
// （"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_transform_position_from_scaled_space() {
    let e = Ellipsoid::new(2.0, 3.0, 4.0);
    let result = e.transform_position_from_scaled_space(DVec3::new(2.0, 2.0, 2.0));
    assert_vec3_epsilon!(result, DVec3::new(4.0, 6.0, 8.0), epsilon::EPSILON16);
}

// --- equals / toString ---

// "equals works in all cases"
//（`equals(undefined)` 用例在 Rust 中无法表示——类型安全的相等性）
#[test]
fn test_equals() {
    let e = Ellipsoid::new(1.0, 0.0, 0.0);
    assert!(e == Ellipsoid::new(1.0, 0.0, 0.0));
    assert!(e != Ellipsoid::new(1.0, 1.0, 0.0));
}

// "toString produces expected values"
#[test]
fn test_to_string() {
    let e = Ellipsoid::new(1.0, 2.0, 3.0);
    assert_eq!(format!("{}", e), "(1, 2, 3)");
}

// --- 构造函数校验 ---

// "constructor throws if x less than 0"
#[test]
#[should_panic]
fn test_constructor_throws_x_negative() {
    let _ = Ellipsoid::new(-1.0, 0.0, 0.0);
}

// "constructor throws if y less than 0"
#[test]
#[should_panic]
fn test_constructor_throws_y_negative() {
    let _ = Ellipsoid::new(0.0, -1.0, 0.0);
}

// "constructor throws if z less than 0"
#[test]
#[should_panic]
fn test_constructor_throws_z_negative() {
    let _ = Ellipsoid::new(0.0, 0.0, -1.0);
}

// "expect Ellipsoid.geocentricSurfaceNormal is be Cartesian3.normalize"
// 从函数同一性检查适配为行为检查（Rust 没有
// 函数同一性语义）：地心表面法线即归一化后的
// 位置向量。
#[test]
fn test_geocentric_surface_normal_is_normalize() {
    let e = Ellipsoid::WGS84;
    let p = space_cartesian();
    assert_vec3_epsilon!(e.geocentric_surface_normal(p), p.normalize(), epsilon::EPSILON15);
}

// --- clone ---

// "clone copies any object with the proper structure"
// "clone uses result parameter if provided"（已归并：Rust Clone 返回拥有所有权的值）
#[test]
fn test_clone() {
    let e = Ellipsoid::new(1.0, 2.0, 3.0);
    let cloned = e.clone();
    assert_eq!(cloned, e);
    assert_eq!(cloned.radii(), radii());
    assert_eq!(cloned.radii_squared(), radii_squared());
    assert_eq!(cloned.minimum_radius(), MINIMUM_RADIUS);
    assert_eq!(cloned.maximum_radius(), MAXIMUM_RADIUS);
}

// --- getSurfaceNormalIntersectionWithZAxis ---

// "getSurfaceNormalIntersectionWithZAxis throws if the ellipsoid is not an
//  ellipsoid of revolution"
#[test]
#[should_panic]
fn test_surface_normal_intersection_throws_not_revolution() {
    let e = Ellipsoid::new(1.0, 2.0, 3.0);
    let _ = e.get_surface_normal_intersection_with_z_axis(DVec3::ZERO, None);
}

// "getSurfaceNormalIntersectionWithZAxis throws if the ellipsoid has radii.z === 0"
//（原始使用 Ellipsoid(1,2,0)；旋转体检查先触发——要点在于
//  退化的椭球体会 panic）
#[test]
#[should_panic]
fn test_surface_normal_intersection_throws_z_zero() {
    let e = Ellipsoid::new(1.0, 2.0, 0.0);
    let _ = e.get_surface_normal_intersection_with_z_axis(DVec3::ZERO, None);
}

// "getSurfaceNormalIntersectionWithZAxis works without a result parameter"
// （"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_surface_normal_intersection_works() {
    let e = Ellipsoid::WGS84;
    let cartographic = Cartographic::from_degrees(35.23, 33.23, 0.0);
    let cartesian_on_the_surface = e.cartographic_to_cartesian(&cartographic);
    let result = e.get_surface_normal_intersection_with_z_axis(cartesian_on_the_surface, None);
    assert!(result.is_some());
}

// "getSurfaceNormalIntersectionWithZAxis returns undefined if the result is outside
//  the ellipsoid with buffer parameter"
#[test]
fn test_surface_normal_intersection_none_with_buffer() {
    let e = Ellipsoid::WGS84;
    let cartographic = Cartographic::from_degrees(35.23, 33.23, 0.0);
    let cartesian_on_the_surface = e.cartographic_to_cartesian(&cartographic);
    let result = e.get_surface_normal_intersection_with_z_axis(
        cartesian_on_the_surface,
        Some(e.radii().z),
    );
    assert!(result.is_none());
}

// "getSurfaceNormalIntersectionWithZAxis returns undefined if the result is outside
//  the ellipsoid without buffer parameter"
#[test]
fn test_surface_normal_intersection_none_without_buffer() {
    let major_axis = 10.0;
    let minor_axis = 1.0;
    let e = Ellipsoid::new(major_axis, major_axis, minor_axis);
    let cartographic = Cartographic::from_degrees(45.0, 90.0, 0.0);
    let cartesian_on_the_surface = e.cartographic_to_cartesian(&cartographic);
    let result = e.get_surface_normal_intersection_with_z_axis(cartesian_on_the_surface, None);
    assert!(result.is_none());
}

// "getSurfaceNormalIntersectionWithZAxis returns a result that is equal to a value
//  that computed in a different way"
#[test]
fn test_surface_normal_intersection_matches_alternate_computation() {
    let e = Ellipsoid::WGS84;
    let cartographic = Cartographic::from_degrees(35.23, 33.23, 0.0);
    let mut cartesian_on_the_surface = e.cartographic_to_cartesian(&cartographic);
    let surface_normal = e.geodetic_surface_normal(cartesian_on_the_surface).unwrap();
    let magnitude = cartesian_on_the_surface.x / surface_normal.x;

    let expected = DVec3::new(
        0.0,
        0.0,
        cartesian_on_the_surface.z - surface_normal.z * magnitude,
    );
    let result = e
        .get_surface_normal_intersection_with_z_axis(cartesian_on_the_surface, None)
        .unwrap();
    assert_vec3_epsilon!(result, expected, epsilon::EPSILON8);

    // 赤道处
    cartesian_on_the_surface = DVec3::new(e.radii().x, 0.0, 0.0);
    let result = e
        .get_surface_normal_intersection_with_z_axis(cartesian_on_the_surface, None)
        .unwrap();
    assert_vec3_epsilon!(result, DVec3::ZERO, epsilon::EPSILON8);
}

// "getSurfaceNormalIntersectionWithZAxis returns a result that when it's used as an
//  origin for a vector with the surface normal direction it produces an accurate
//  cartographic"
#[test]
fn test_surface_normal_intersection_produces_accurate_cartographic() {
    let e = Ellipsoid::WGS84;

    // 一般位置
    let mut cartographic = Cartographic::from_degrees(35.23, 33.23, 0.0);
    let mut cartesian_on_the_surface = e.cartographic_to_cartesian(&cartographic);
    let mut surface_normal = e.geodetic_surface_normal(cartesian_on_the_surface).unwrap();
    let mut result = e
        .get_surface_normal_intersection_with_z_axis(cartesian_on_the_surface, None)
        .unwrap();
    let surface_normal_with_length = surface_normal * e.maximum_radius();
    let position = result + surface_normal_with_length;
    let mut result_cartographic = e.cartesian_to_cartographic(position).unwrap();
    result_cartographic.height = 0.0;
    assert!(result_cartographic.equals_epsilon(&cartographic, epsilon::EPSILON8));

    // 北极处
    cartographic = Cartographic::from_degrees(0.0, 90.0, 0.0);
    cartesian_on_the_surface = DVec3::new(0.0, 0.0, e.radii().z);
    surface_normal = e.geodetic_surface_normal(cartesian_on_the_surface).unwrap();
    result = e
        .get_surface_normal_intersection_with_z_axis(cartesian_on_the_surface, None)
        .unwrap();
    let surface_normal_with_length = surface_normal * e.maximum_radius();
    let position = result + surface_normal_with_length;
    let mut result_cartographic = e.cartesian_to_cartographic(position).unwrap();
    result_cartographic.height = 0.0;
    assert!(result_cartographic.equals_epsilon(&cartographic, epsilon::EPSILON8));
}

// --- getLocalCurvature ---

// "getLocalCurvature returns expected values at the equator"
#[test]
fn test_local_curvature_at_equator() {
    let e = Ellipsoid::WGS84;
    let cartographic = Cartographic::from_degrees(0.0, 0.0, 0.0);
    let cartesian_on_the_surface = e.cartographic_to_cartesian(&cartographic);
    let result = e.get_local_curvature(cartesian_on_the_surface).unwrap();
    let expected = DVec2::new(
        1.0 / e.maximum_radius(),
        e.maximum_radius() / (e.minimum_radius() * e.minimum_radius()),
    );
    assert_vec2_epsilon!(result, expected, epsilon::EPSILON8);
}

// "getLocalCurvature returns expected values at the north pole"
#[test]
fn test_local_curvature_at_north_pole() {
    let e = Ellipsoid::WGS84;
    let cartographic = Cartographic::from_degrees(0.0, 90.0, 0.0);
    let cartesian_on_the_surface = e.cartographic_to_cartesian(&cartographic);
    let result = e.get_local_curvature(cartesian_on_the_surface).unwrap();
    let semi_latus_rectum = (e.maximum_radius() * e.maximum_radius()) / e.minimum_radius();
    let expected = DVec2::new(1.0 / semi_latus_rectum, 1.0 / semi_latus_rectum);
    assert_vec2_epsilon!(result, expected, epsilon::EPSILON8);
}

// --- squaredXOverSquaredZ ---

// "ellipsoid is initialized with _squaredXOverSquaredZ property"
#[test]
fn test_squared_x_over_squared_z() {
    let e = Ellipsoid::new(4.0, 4.0, 3.0);
    let expected = e.radii_squared().x / e.radii_squared().z;
    assert_eq!(e.squared_x_over_squared_z(), expected);
}

// --- surfaceArea ---

// "computes surfaceArea"
#[test]
fn test_surface_area() {
    let full = Rectangle::new(-PI_F64, -PI_OVER_TWO, PI_F64, PI_OVER_TWO);

    // 扁球体表面积
    let e = Ellipsoid::new(4.0, 4.0, 3.0);
    let a2 = e.radii_squared().x;
    let c2 = e.radii_squared().z;
    let ecc = (1.0 - c2 / a2).sqrt();
    let area = TWO_PI * a2 + PI_F64 * (c2 / ecc) * ((1.0 + ecc) / (1.0 - ecc)).ln();
    assert_approx!(e.surface_area(&full), area, epsilon::EPSILON3);

    // 长球体表面积
    let e = Ellipsoid::new(3.0, 3.0, 4.0);
    let a2 = e.radii_squared().x;
    let c2 = e.radii_squared().z;
    let ecc = (1.0 - a2 / c2).sqrt();
    let a = e.radii().x;
    let c = e.radii().z;
    let area = TWO_PI * a2 + TWO_PI * ((a * c) / ecc) * ecc.asin();
    assert_approx!(e.surface_area(&full), area, epsilon::EPSILON3);
}

// --- Packable（createPackableSpecs）---

// createPackableSpecs：packedLength / pack / unpack 往返。
#[test]
fn test_packed_length() {
    assert_eq!(Ellipsoid::PACKED_LENGTH, 3);
}

#[test]
fn test_pack_unpack_roundtrip() {
    let e = Ellipsoid::WGS84;
    let mut array = [0.0f64; 3];
    e.pack(&mut array, 0);
    assert_eq!(array[0], Ellipsoid::WGS84.radii().x);
    assert_eq!(array[1], Ellipsoid::WGS84.radii().y);
    assert_eq!(array[2], Ellipsoid::WGS84.radii().z);

    let unpacked = Ellipsoid::unpack(&array, 0);
    assert_eq!(unpacked, e);
}
