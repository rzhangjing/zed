//! Core/GeographicProjectionSpec → Rust 集成测试（对齐实现）。
//!
//! 对齐实现原始 CesiumJS
//! `Specs/Core/GeographicProjectionSpec`（9 个 `it()` 用例）。
//! 参考值原样使用，以便针对与 CesiumJS 完全相同的基准真值
//! 验证 Rust 实现。
//!
//! 平台适配（按验证计划予以记录）：
//! - CesiumJS "project3" / "unproject1" 为 "works with a result parameter"
//!   变体，用于测试 JS 内存复用 API 契约（`result === returnValue`）。
//!   Rust 返回自有值且没有 result-parameter API，因此这些变体
//!   由下方的 owned-return 测试涵盖（计算值相同，
//!   单一代码路径）。
//! - CesiumJS "project throws without cartesian" 实际调用
//!   `projection.unproject()` 且不带参数，用于测试运行期空值检查。
//!   Rust 类型系统使缺失参数无法表示（编译期安全），
//!   故该错误路径没有 Rust 对应项，予以省略。
//! - `construct0` 使用 `new GeographicProjection()`，其默认值为
//!   `Ellipsoid.default`（测试环境中为 WGS84）；默认构造的 Rust 等价形式
//!   为 `GeographicProjection::wgs84()`。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::projection::{GeographicProjection, MapProjection};
use cesium_specs::math_consts::{PI_OVER_FOUR, PI_OVER_TWO};
use glam::DVec3;
use std::f64::consts::PI;

// "construct0"
#[test]
fn test_construct0() {
    let projection = GeographicProjection::wgs84();
    assert_eq!(projection.ellipsoid(), &Ellipsoid::WGS84);
}

// "construct1"
#[test]
fn test_construct1() {
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let projection = GeographicProjection::new(ellipsoid);
    assert_eq!(projection.ellipsoid(), &ellipsoid);
}

// "project0"
#[test]
fn test_project0() {
    let height = 10.0;
    let cartographic = Cartographic::from_radians(0.0, 0.0, height);
    let projection = GeographicProjection::wgs84();
    assert_eq!(
        projection.project(&cartographic),
        DVec3::new(0.0, 0.0, height)
    );
}

// "project1"
#[test]
fn test_project1() {
    let ellipsoid = Ellipsoid::WGS84;
    let cartographic = Cartographic::from_radians(PI, PI_OVER_TWO, 0.0);
    let expected = DVec3::new(
        PI * ellipsoid.radii().x,
        PI_OVER_TWO * ellipsoid.radii().x,
        0.0,
    );
    let projection = GeographicProjection::new(ellipsoid);
    assert_eq!(projection.project(&cartographic), expected);
}

// "project2"
#[test]
fn test_project2() {
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    let cartographic = Cartographic::from_radians(-PI, PI_OVER_TWO, 0.0);
    let expected = DVec3::new(-PI, PI_OVER_TWO, 0.0);
    let projection = GeographicProjection::new(ellipsoid);
    assert_eq!(projection.project(&cartographic), expected);
}

// "unproject0"
#[test]
fn test_unproject0() {
    let cartographic = Cartographic::from_radians(PI_OVER_TWO, PI_OVER_FOUR, 12.0);
    let projection = GeographicProjection::wgs84();
    let projected = projection.project(&cartographic);
    assert_eq!(projection.unproject(projected), cartographic);
}
