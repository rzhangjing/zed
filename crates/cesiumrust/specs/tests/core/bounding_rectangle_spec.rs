//! Core/BoundingRectangleSpec → Rust 集成测试
//!
//! 对齐实现自 CesiumJS `Specs/Core/BoundingRectangleSpec`（28 个 `it()` 用例）。
//!
//! ## 平台适配
//! - JS 结果参数变体（`clone(result)`、`fromPoints(p, result)`、
//!   `fromRectangle(r, p, result)`、`union(l, r, result)`、`expand(rect, pt, result)`）
//!   已合并进返回所有权值的测试：Rust 返回所有权值 / 使用 `Copy`。
//! - JS "throws with no <arg>" 用例（null/undefined 检查）已省略：Rust 的类型
//!   系统使传入 `undefined` 不可能。
//! - JS 无参数的 `fromPoints()` / `fromRectangle()` 对应 Rust 空切片 /
//!   必需引用语义；“无矩形”空用例已省略（Rust 要求传入
//!   `&Rectangle`），而“无位置”对应 `from_points(&[])`。
//! - JS 无参数的 `clone()` 返回 `undefined`；Rust `Copy` 没有此路径 → 省略。
//! - `createPackableSpecs`（pack/unpack 到数组）已省略：打包是 JS 数组
//!   序列化关注点，不属于 Rust 领域 API。

use cesium_geospatial::bounding::BoundingRectangle;
use cesium_geospatial::ray::Intersect;
use cesium_geospatial::{Ellipsoid, GeographicProjection, Rectangle};
use cesium_specs::{assert_approx, epsilon};
use glam::DVec2;

/// `it("default constructor sets expected values")`
#[test]
fn test_br_default() {
    let r = BoundingRectangle::default();
    assert_approx!(r.x, 0.0, epsilon::EPSILON15);
    assert_approx!(r.y, 0.0, epsilon::EPSILON15);
    assert_approx!(r.width, 0.0, epsilon::EPSILON15);
    assert_approx!(r.height, 0.0, epsilon::EPSILON15);
}

/// `it("constructor sets expected parameters")`
#[test]
fn test_br_constructor() {
    let r = BoundingRectangle::new(1.0, 2.0, 3.0, 4.0);
    assert_approx!(r.x, 1.0, epsilon::EPSILON15);
    assert_approx!(r.y, 2.0, epsilon::EPSILON15);
    assert_approx!(r.width, 3.0, epsilon::EPSILON15);
    assert_approx!(r.height, 4.0, epsilon::EPSILON15);
}

/// `it("clone without a result parameter")`
#[test]
fn test_br_clone() {
    let r = BoundingRectangle::new(1.0, 2.0, 3.0, 4.0);
    let result = r; // 拷贝语义 == r.clone()
    assert!(r == result);
}

/// `it("equals")`
#[test]
fn test_br_equals() {
    let r = BoundingRectangle::new(1.0, 2.0, 3.0, 4.0);
    assert!(r == BoundingRectangle::new(1.0, 2.0, 3.0, 4.0));
    assert!(r != BoundingRectangle::new(5.0, 2.0, 3.0, 4.0));
    assert!(r != BoundingRectangle::new(1.0, 6.0, 3.0, 4.0));
    assert!(r != BoundingRectangle::new(1.0, 2.0, 7.0, 4.0));
    assert!(r != BoundingRectangle::new(1.0, 2.0, 3.0, 8.0));
}

fn positions() -> Vec<DVec2> {
    vec![
        DVec2::new(3.0, -1.0),
        DVec2::new(2.0, -2.0),
        DVec2::new(1.0, -3.0),
        DVec2::new(0.0, 0.0),
        DVec2::new(-1.0, 1.0),
        DVec2::new(-2.0, 2.0),
        DVec2::new(-3.0, 3.0),
    ]
}

/// `it("create axis aligned bounding rectangle")`
#[test]
fn test_br_from_points() {
    let r = BoundingRectangle::from_points(&positions());
    assert_approx!(r.x, -3.0, epsilon::EPSILON15);
    assert_approx!(r.y, -3.0, epsilon::EPSILON15);
    assert_approx!(r.width, 6.0, epsilon::EPSILON15);
    assert_approx!(r.height, 6.0, epsilon::EPSILON15);
}

/// `it("fromPoints creates an empty rectangle with no positions")`
#[test]
fn test_br_from_points_empty() {
    let r = BoundingRectangle::from_points(&[]);
    assert_approx!(r.x, 0.0, epsilon::EPSILON15);
    assert_approx!(r.y, 0.0, epsilon::EPSILON15);
    assert_approx!(r.width, 0.0, epsilon::EPSILON15);
    assert_approx!(r.height, 0.0, epsilon::EPSILON15);
}

/// `it("create a bounding rectangle from a rectangle")`
#[test]
fn test_br_from_rectangle() {
    let rectangle = Rectangle::MAX_VALUE;
    let projection = GeographicProjection::new(Ellipsoid::UNIT_SPHERE);
    let expected = BoundingRectangle::new(
        rectangle.west,
        rectangle.south,
        rectangle.east - rectangle.west,
        rectangle.north - rectangle.south,
    );
    let result = BoundingRectangle::from_rectangle(&rectangle, &projection);
    assert_approx!(result.x, expected.x, epsilon::EPSILON15);
    assert_approx!(result.y, expected.y, epsilon::EPSILON15);
    assert_approx!(result.width, expected.width, epsilon::EPSILON15);
    assert_approx!(result.height, expected.height, epsilon::EPSILON15);
}

/// `it("intersect works")`
#[test]
fn test_br_intersect() {
    let rectangle1 = BoundingRectangle::new(0.0, 0.0, 4.0, 4.0);
    let rectangle2 = BoundingRectangle::new(2.0, 2.0, 4.0, 4.0);
    let rectangle3 = BoundingRectangle::new(-6.0, 2.0, 4.0, 4.0);
    let rectangle4 = BoundingRectangle::new(8.0, 2.0, 4.0, 4.0);
    let rectangle5 = BoundingRectangle::new(2.0, -6.0, 4.0, 4.0);
    let rectangle6 = BoundingRectangle::new(2.0, 8.0, 4.0, 4.0);

    assert!(rectangle1.intersect(&rectangle2) == Intersect::Intersecting);
    assert!(rectangle1.intersect(&rectangle3) == Intersect::Outside);
    assert!(rectangle1.intersect(&rectangle4) == Intersect::Outside);
    assert!(rectangle1.intersect(&rectangle5) == Intersect::Outside);
    assert!(rectangle1.intersect(&rectangle6) == Intersect::Outside);
}

/// `it("union works without a result parameter")`
#[test]
fn test_br_union() {
    let rectangle1 = BoundingRectangle::new(2.0, 0.0, 1.0, 1.0);
    let rectangle2 = BoundingRectangle::new(-2.0, 0.0, 1.0, 2.0);
    let expected = BoundingRectangle::new(-2.0, 0.0, 5.0, 2.0);
    let result = rectangle1.union(&rectangle2);
    assert!(result == expected);
}

/// `it("expand works if rectangle needs to grow right")`
#[test]
fn test_br_expand_right() {
    let rectangle = BoundingRectangle::new(2.0, 0.0, 1.0, 1.0);
    let point = DVec2::new(4.0, 0.0);
    let expected = BoundingRectangle::new(2.0, 0.0, 2.0, 1.0);
    assert!(rectangle.expand(point) == expected);
}

/// `it("expand works if rectangle needs x to grow left")`
#[test]
fn test_br_expand_left() {
    let rectangle = BoundingRectangle::new(2.0, 0.0, 1.0, 1.0);
    let point = DVec2::new(0.0, 0.0);
    let expected = BoundingRectangle::new(0.0, 0.0, 3.0, 1.0);
    assert!(rectangle.expand(point) == expected);
}

/// `it("expand works if rectangle needs to grow up")`
#[test]
fn test_br_expand_up() {
    let rectangle = BoundingRectangle::new(2.0, 0.0, 1.0, 1.0);
    let point = DVec2::new(2.0, 2.0);
    let expected = BoundingRectangle::new(2.0, 0.0, 1.0, 2.0);
    assert!(rectangle.expand(point) == expected);
}

/// `it("expand works if rectangle needs x to grow down")`
#[test]
fn test_br_expand_down() {
    let rectangle = BoundingRectangle::new(2.0, 0.0, 1.0, 1.0);
    let point = DVec2::new(2.0, -1.0);
    let expected = BoundingRectangle::new(2.0, -1.0, 1.0, 2.0);
    assert!(rectangle.expand(point) == expected);
}

/// `it("expand works if rectangle does not need to grow")`
#[test]
fn test_br_expand_no_grow() {
    let rectangle = BoundingRectangle::new(2.0, 0.0, 1.0, 1.0);
    let point = DVec2::new(2.5, 0.6);
    let expected = BoundingRectangle::new(2.0, 0.0, 1.0, 1.0);
    assert!(rectangle.expand(point) == expected);
}
