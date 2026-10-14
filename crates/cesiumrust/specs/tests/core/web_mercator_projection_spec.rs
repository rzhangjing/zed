//! Core/WebMercatorProjectionSpec → Rust 集成测试（对齐实现）。
//!
//! 对齐实现原始 CesiumJS
//! `Specs/Core/WebMercatorProjectionSpec`（12 个 `it()` 用例）。
//! 参考值逐字使用，以便 Rust 实现针对与 CesiumJS 完全相同的
//! 基准真值进行验证。
//!
//! 平台适配（依据验证计划，均有文档说明）：
//! - CesiumJS 的 "project3" / "unproject1" 是 "works with a result parameter"
//!   变体，测试 JS 的内存复用 API 契约（`result === returnValue`）。
//!   Rust 返回拥有所有权的值且没有 result-parameter API，因此这些变体
//!   被下方的拥有返回测试所归并（计算值相同，
//!   单一代码路径）。
//! - CesiumJS 的 "project throws without cartesian" 实际调用
//!   `projection.unproject()` 且不带参数，测试运行时的 null 检查。
//!   Rust 的类型系统使缺失参数无法表示（编译期
//!   安全），因此该错误路径没有对应的 Rust 版本，予以省略。
//! - "unproject is correct at corners" 用例在 CesiumJS 中传入 `Cartesian2`
//!   输入；Rust 的 `unproject` 接受 `DVec3`，因此 `z` 取 0.0
//!   （仅断言经度/纬度，与原用例一致）。
//! - `construct0` 使用 `new WebMercatorProjection()`，其默认值为
//!   `Ellipsoid.default`（测试环境中为 WGS84）；默认构造的
//!   Rust 等价写法是 `WebMercatorProjection::wgs84()`。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::projection::{MapProjection, WebMercatorProjection};
use cesium_geospatial::math_utils::to_radians;
use cesium_specs::math_consts::{PI_OVER_FOUR, PI_OVER_TWO};
use cesium_specs::{assert_approx, epsilon};
use glam::DVec3;
use std::f64::consts::PI;

/// Web Mercator 世界地图（正方形）的投影范围（以米为单位）。
const MAX_MERCATOR_EXTENT: f64 = 20037508.342787;

// "construct0"
#[test]
fn test_construct0() {
    let projection = WebMercatorProjection::wgs84();
    assert_eq!(projection.ellipsoid(), &Ellipsoid::WGS84);
}

// "construct1"
#[test]
fn test_construct1() {
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let projection = WebMercatorProjection::new(ellipsoid);
    assert_eq!(projection.ellipsoid(), &ellipsoid);
}

// "project0"
#[test]
fn test_project0() {
    let height = 10.0;
    let cartographic = Cartographic::from_radians(0.0, 0.0, height);
    let projection = WebMercatorProjection::wgs84();
    assert_eq!(
        projection.project(&cartographic),
        DVec3::new(0.0, 0.0, height)
    );
}

// "project1"
// 期望方程来自 Wolfram MathWorld：
// http://mathworld.wolfram.com/MercatorProjection.html
#[test]
fn test_project1() {
    let ellipsoid = Ellipsoid::WGS84;
    let cartographic = Cartographic::from_radians(PI, PI_OVER_FOUR, 0.0);
    let expected = DVec3::new(
        ellipsoid.maximum_radius() * cartographic.longitude,
        ellipsoid.maximum_radius()
            * (PI / 4.0 + cartographic.latitude / 2.0).tan().ln(),
        0.0,
    );
    let projection = WebMercatorProjection::new(ellipsoid);
    assert_vec3_epsilon_proj(&projection.project(&cartographic), &expected, epsilon::EPSILON8);
}

// "project2"
#[test]
fn test_project2() {
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let cartographic = Cartographic::from_radians(-PI, PI_OVER_FOUR, 0.0);
    let expected = DVec3::new(
        ellipsoid.maximum_radius() * cartographic.longitude,
        ellipsoid.maximum_radius()
            * (PI / 4.0 + cartographic.latitude / 2.0).tan().ln(),
        0.0,
    );
    let projection = WebMercatorProjection::new(ellipsoid);
    assert_vec3_epsilon_proj(&projection.project(&cartographic), &expected, epsilon::EPSILON15);
}

// "unproject0"
#[test]
fn test_unproject0() {
    let cartographic = Cartographic::from_radians(PI_OVER_TWO, PI_OVER_FOUR, 12.0);
    let projection = WebMercatorProjection::wgs84();
    let projected = projection.project(&cartographic);
    let result = projection.unproject(projected);
    assert_approx!(result.longitude, cartographic.longitude, epsilon::EPSILON14);
    assert_approx!(result.latitude, cartographic.latitude, epsilon::EPSILON14);
    assert_approx!(result.height, cartographic.height, epsilon::EPSILON14);
}

// "unproject is correct at corners"
#[test]
fn test_unproject_is_correct_at_corners() {
    let projection = WebMercatorProjection::wgs84();

    let southwest = projection.unproject(DVec3::new(
        -MAX_MERCATOR_EXTENT,
        -MAX_MERCATOR_EXTENT,
        0.0,
    ));
    assert_approx!(southwest.longitude, -PI, epsilon::EPSILON12);
    assert_approx!(
        southwest.latitude,
        to_radians(-85.05112878),
        epsilon::EPSILON11
    );

    let southeast = projection.unproject(DVec3::new(
        MAX_MERCATOR_EXTENT,
        -MAX_MERCATOR_EXTENT,
        0.0,
    ));
    assert_approx!(southeast.longitude, PI, epsilon::EPSILON12);
    assert_approx!(
        southeast.latitude,
        to_radians(-85.05112878),
        epsilon::EPSILON11
    );

    let northeast = projection.unproject(DVec3::new(
        MAX_MERCATOR_EXTENT,
        MAX_MERCATOR_EXTENT,
        0.0,
    ));
    assert_approx!(northeast.longitude, PI, epsilon::EPSILON12);
    assert_approx!(
        northeast.latitude,
        to_radians(85.05112878),
        epsilon::EPSILON11
    );

    let northwest = projection.unproject(DVec3::new(
        -MAX_MERCATOR_EXTENT,
        MAX_MERCATOR_EXTENT,
        0.0,
    ));
    assert_approx!(northwest.longitude, -PI, epsilon::EPSILON12);
    assert_approx!(
        northwest.latitude,
        to_radians(85.05112878),
        epsilon::EPSILON11
    );
}

// "project is correct at corners."
#[test]
fn test_project_is_correct_at_corners() {
    let max_latitude = WebMercatorProjection::MAXIMUM_LATITUDE;
    let projection = WebMercatorProjection::wgs84();

    let southwest = projection.project(&Cartographic::from_radians(-PI, -max_latitude, 0.0));
    assert_approx!(southwest.x, -MAX_MERCATOR_EXTENT, epsilon::EPSILON3);
    assert_approx!(southwest.y, -MAX_MERCATOR_EXTENT, epsilon::EPSILON3);

    let southeast = projection.project(&Cartographic::from_radians(PI, -max_latitude, 0.0));
    assert_approx!(southeast.x, MAX_MERCATOR_EXTENT, epsilon::EPSILON3);
    assert_approx!(southeast.y, -MAX_MERCATOR_EXTENT, epsilon::EPSILON3);

    let northeast = projection.project(&Cartographic::from_radians(PI, max_latitude, 0.0));
    assert_approx!(northeast.x, MAX_MERCATOR_EXTENT, epsilon::EPSILON3);
    assert_approx!(northeast.y, MAX_MERCATOR_EXTENT, epsilon::EPSILON3);

    let northwest = projection.project(&Cartographic::from_radians(-PI, max_latitude, 0.0));
    assert_approx!(northwest.x, -MAX_MERCATOR_EXTENT, epsilon::EPSILON3);
    assert_approx!(northwest.y, MAX_MERCATOR_EXTENT, epsilon::EPSILON3);
}

// "projected y is clamped to valid latitude range."
#[test]
fn test_projected_y_is_clamped_to_valid_latitude_range() {
    let projection = WebMercatorProjection::wgs84();

    let south_pole = projection.project(&Cartographic::from_radians(0.0, -PI_OVER_TWO, 0.0));
    let south_limit = projection.project(&Cartographic::from_radians(
        0.0,
        -WebMercatorProjection::MAXIMUM_LATITUDE,
        0.0,
    ));
    assert_eq!(south_pole.y, south_limit.y);

    let north_pole = projection.project(&Cartographic::from_radians(0.0, PI_OVER_TWO, 0.0));
    let north_limit = projection.project(&Cartographic::from_radians(
        0.0,
        WebMercatorProjection::MAXIMUM_LATITUDE,
        0.0,
    ));
    assert_eq!(north_pole.y, north_limit.y);
}

/// 对投影得到的 DVec3 结果做逐分量的 epsilon 比较。
fn assert_vec3_epsilon_proj(actual: &DVec3, expected: &DVec3, eps: f64) {
    assert_approx!(actual.x, expected.x, eps);
    assert_approx!(actual.y, expected.y, eps);
    assert_approx!(actual.z, expected.z, eps);
}
