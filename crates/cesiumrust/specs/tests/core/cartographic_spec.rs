//! Core/CartographicSpec → Rust 集成测试（对齐实现）。
//!
//! 对齐实现原始 CesiumJS `Specs/Core/CartographicSpec`
//!（24 个 `it()` 用例）。参考值逐字使用，以便 Rust 实现
//! 针对与 CesiumJS 完全相同的基准真值进行验证。
//!
//! 平台适配（依据验证计划，均有文档说明）：
//! - CesiumJS 的 "works with a result parameter" 变体测试 JS 的内存复用
//!   API 契约（`returnedResult === result`）。Rust 返回拥有所有权的值且没有
//!   result-parameter API，因此这些变体被下方的拥有返回
//!   测试所归并（计算值相同，单一代码路径）。
//! - CesiumJS 的 "throws without longitude/latitude" 和 "throws when there is no
//!   cartesian" 用例测试运行时的 null 检查。Rust 的类型系统使 null
//!   参数无法表示（编译期安全），因此这些错误路径
//!   没有对应的 Rust 版本。这些用例的 "defaults altitude" 部分通过
//!   显式传入高度 0.0 来移植（Rust 没有可选参数）。
//! - `Ellipsoid.default` 是一种 JS 可变全局模式。"uses default
//!   ellipsoid" 测试会设置 `Ellipsoid.default = Ellipsoid.MOON`；在 Rust 中
//!   椭球是显式传入的，因此这些测试直接传入 `&Ellipsoid::MOON`
//!   （转换行为相同，无全局状态）。
//! - `clone` 映射到 Rust 派生的 `Clone`；`equals` 映射到派生的 `PartialEq`。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::math_utils::to_radians;
use cesium_specs::{assert_vec3_epsilon, epsilon};
use glam::DVec3;

// --- 来自原始规范的参考值 ---

#[allow(clippy::excessive_precision)]
fn surface_cartesian() -> DVec3 {
    DVec3::new(4094327.7921465295, 1909216.4044747739, 4487348.4088659193)
}
fn surface_cartographic() -> Cartographic {
    Cartographic::from_radians(to_radians(25.0), to_radians(45.0), 0.0)
}
#[allow(clippy::excessive_precision)]
fn moon_position() -> DVec3 {
    DVec3::new(1593514.338295244, 691991.9979835141, 20442.318221152018)
}
fn moon_cartographic() -> Cartographic {
    Cartographic::from_degrees(23.47315, 0.67416, 0.0)
}

// --- 构造函数 ---

// "default constructor sets expected properties"
#[test]
fn test_default_constructor_sets_expected_properties() {
    let c = Cartographic::default();
    assert_eq!(c.longitude, 0.0);
    assert_eq!(c.latitude, 0.0);
    assert_eq!(c.height, 0.0);
}

// "constructor sets expected properties from parameters"
#[test]
fn test_constructor_sets_expected_properties_from_parameters() {
    let c = Cartographic::from_radians(1.0, 2.0, 3.0);
    assert_eq!(c.longitude, 1.0);
    assert_eq!(c.latitude, 2.0);
    assert_eq!(c.height, 3.0);
}

// --- toCartesian ---

// "toCartesian conversion from Cartographic input to Cartesian3 output"
// 原用例断言 `Cartographic.toCartesian(c)` toEqual `ellipsoid.cartographicToCartesian(c)`。
#[test]
fn test_to_cartesian_conversion() {
    let lon = to_radians(150.0);
    let lat = to_radians(-40.0);
    let height = 100000.0;
    let ellipsoid = Ellipsoid::WGS84;
    let c = Cartographic::from_radians(lon, lat, height);
    let actual = Cartographic::to_cartesian(&c, &ellipsoid);
    let expected = ellipsoid.cartographic_to_cartesian(&c);
    assert_eq!(actual, expected);
}

// "toCartesian uses default ellipsoid"
// 原用例设置 `Ellipsoid.default = Ellipsoid.MOON`；Rust 显式传入 MOON。
#[test]
fn test_to_cartesian_uses_moon_ellipsoid() {
    let cartographic = moon_cartographic();
    let position = Cartographic::to_cartesian(&cartographic, &Ellipsoid::MOON);
    assert_vec3_epsilon!(position, moon_position(), epsilon::EPSILON8);
}

// --- fromRadians ---

// "fromRadians works without a result parameter"
//（"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_from_radians() {
    let c = Cartographic::from_radians(std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_4, 100.0);
    assert_eq!(c.longitude, std::f64::consts::FRAC_PI_2);
    assert_eq!(c.latitude, std::f64::consts::FRAC_PI_4);
    assert_eq!(c.height, 100.0);
}

// "fromRadians throws without longitude or latitude parameter but defaults altitude"
// "throws" 部分没有对应的 Rust 版本（类型安全、无 null）。"defaults
// altitude" 部分做了适配：Rust 没有可选参数，因此显式
// 传入高度 0.0（即 JS 的默认值）。
#[test]
fn test_from_radians_defaults_altitude() {
    let c = Cartographic::from_radians(std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_4, 0.0);
    assert_eq!(c.longitude, std::f64::consts::FRAC_PI_2);
    assert_eq!(c.latitude, std::f64::consts::FRAC_PI_4);
    assert_eq!(c.height, 0.0);
}

// --- fromDegrees ---

// "fromDegrees works without a result parameter"
//（"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_from_degrees() {
    let c = Cartographic::from_degrees(90.0, 45.0, 100.0);
    assert_eq!(c.longitude, std::f64::consts::FRAC_PI_2);
    assert_eq!(c.latitude, std::f64::consts::FRAC_PI_4);
    assert_eq!(c.height, 100.0);
}

// "fromDegrees throws without longitude or latitude parameter but defaults altitude"
//（适配方式同 test_from_radians_defaults_altitude）
#[test]
fn test_from_degrees_defaults_altitude() {
    let c = Cartographic::from_degrees(90.0, 45.0, 0.0);
    assert_eq!(c.longitude, std::f64::consts::FRAC_PI_2);
    assert_eq!(c.latitude, std::f64::consts::FRAC_PI_4);
    assert_eq!(c.height, 0.0);
}

// --- fromCartesian ---

// "fromCartesian works without a result parameter"
//（"with a result parameter" 变体已被归并：Rust 返回拥有所有权的值）
#[test]
fn test_from_cartesian() {
    let c = Cartographic::from_cartesian(surface_cartesian(), &Ellipsoid::WGS84).unwrap();
    assert!(c.equals_epsilon(&surface_cartographic(), epsilon::EPSILON8));
}

// "fromCartesian works without an ellipsoid"
// 原用例省略了椭球（默认为 WGS84）；Rust 显式传入 WGS84。
#[test]
fn test_from_cartesian_default_ellipsoid_wgs84() {
    let c = Cartographic::from_cartesian(surface_cartesian(), &Ellipsoid::WGS84).unwrap();
    assert!(c.equals_epsilon(&surface_cartographic(), epsilon::EPSILON8));
}

// "fromCartesian uses default ellipsoid"
// 原用例设置 `Ellipsoid.default = Ellipsoid.MOON`；Rust 显式传入 MOON。
#[test]
fn test_from_cartesian_uses_moon_ellipsoid() {
    let cartographic = Cartographic::from_cartesian(moon_position(), &Ellipsoid::MOON).unwrap();
    assert!(cartographic.equals_epsilon(&moon_cartographic(), epsilon::EPSILON8));
}

// "fromCartesian works with a value that is above the ellipsoid surface"
#[test]
fn test_from_cartesian_above_surface() {
    let cartographic1 = Cartographic::from_degrees(35.766989, 33.333602, 3000.0);
    // 在默认（WGS84）椭球上调用 Cartesian3.fromRadians。
    let cartesian1 = Ellipsoid::WGS84.cartographic_to_cartesian(&cartographic1);
    let cartographic2 = Cartographic::from_cartesian(cartesian1, &Ellipsoid::WGS84).unwrap();
    assert!(cartographic2.equals_epsilon(&cartographic1, epsilon::EPSILON8));
}

// "fromCartesian works with a value that is bellow the ellipsoid surface"
#[test]
fn test_from_cartesian_below_surface() {
    let cartographic1 = Cartographic::from_degrees(35.766989, 33.333602, -3000.0);
    let cartesian1 = Ellipsoid::WGS84.cartographic_to_cartesian(&cartographic1);
    let cartographic2 = Cartographic::from_cartesian(cartesian1, &Ellipsoid::WGS84).unwrap();
    assert!(cartographic2.equals_epsilon(&cartographic1, epsilon::EPSILON8));
}

// --- clone ---

// "clone without a result parameter"
//（"with a result parameter" 和 "'this' result parameter" 变体已被
//  归并：Rust 的 Clone 总是返回一个新的拥有所有权的值）
#[test]
fn test_clone() {
    let cartographic = Cartographic::from_radians(1.0, 2.0, 3.0);
    let result = cartographic.clone();
    assert_ne!(
        &cartographic as *const Cartographic,
        &result as *const Cartographic
    );
    assert_eq!(cartographic, result);
}

// --- equals / equalsEpsilon ---

// "equals"
//（`equals(undefined)` 用例在 Rust 中无法表示——类型安全的相等）
#[test]
fn test_equals() {
    let cartographic = Cartographic::from_radians(1.0, 2.0, 3.0);
    assert!(cartographic == Cartographic::from_radians(1.0, 2.0, 3.0));
    assert!(cartographic != Cartographic::from_radians(2.0, 2.0, 3.0));
    assert!(cartographic != Cartographic::from_radians(2.0, 1.0, 3.0));
    assert!(cartographic != Cartographic::from_radians(1.0, 2.0, 4.0));
}

// "equalsEpsilon"
//（`equalsEpsilon(undefined, 1)` 用例在 Rust 中无法表示）
#[test]
fn test_equals_epsilon() {
    let cartographic = Cartographic::from_radians(1.0, 2.0, 3.0);
    assert!(cartographic.equals_epsilon(&Cartographic::from_radians(1.0, 2.0, 3.0), 0.0));
    assert!(cartographic.equals_epsilon(&Cartographic::from_radians(1.0, 2.0, 3.0), 1.0));
    assert!(cartographic.equals_epsilon(&Cartographic::from_radians(2.0, 2.0, 3.0), 1.0));
    assert!(cartographic.equals_epsilon(&Cartographic::from_radians(1.0, 3.0, 3.0), 1.0));
    assert!(cartographic.equals_epsilon(&Cartographic::from_radians(1.0, 2.0, 4.0), 1.0));
    assert!(!cartographic.equals_epsilon(&Cartographic::from_radians(2.0, 2.0, 3.0), 0.99999));
    assert!(!cartographic.equals_epsilon(&Cartographic::from_radians(1.0, 3.0, 3.0), 0.99999));
    assert!(!cartographic.equals_epsilon(&Cartographic::from_radians(1.0, 2.0, 4.0), 0.99999));
}

// --- toString ---

// "toString"
#[test]
fn test_to_string() {
    let cartographic = Cartographic::from_radians(1.123, 2.345, 6.789);
    assert_eq!(format!("{}", cartographic), "(1.123, 2.345, 6.789)");
}
