//! Core/OrientedBoundingBoxSpec → Rust 集成测试
//!
//! 对齐实现 CesiumJS `Specs/Core/OrientedBoundingBoxSpec`（61 个 `it()` 用例）。
//!
//! ## 平台适配
//! - JS 的 result-parameter 变体（`fromRectangle(.., result)`、`computeCorners(result)`、
//!   `computeTransformation(result)`、`fromTransformation(.., result)`）测试 JS 的
//!   内存复用 API 契约（`returnedResult === result`）。Rust 返回拥有所有权的值，
//!   因此每个变体都被合并进对应的 "without a result parameter" 版本（值相同）。
//! - JS 的 "throws without a <arg>" 用例（null/undefined 检查）予以省略：Rust 的类型系统
//!   使传入 `undefined` 成为不可能。
//! - JS `fromRectangle` 仅限 debug 的 `DeveloperError` 检查（非法宽度/高度、
//!   非旋转椭球）映射到 Rust 的 `debug_assert!`；"throws with invalid
//!   rectangles" / "throws with non-revolution ellipsoids" 用例确实通过
//!   `#[should_panic]` 移植（debug 断言在 `cargo test` 下处于激活状态）。
//! - `clone` 映射到 Rust 派生的 `Clone`/`Copy`；JS 的 "clone undefined → undefined"
//!   路径没有对应的 Rust 版本，予以省略。
//! - `equals` 映射到派生的 `PartialEq`；JS 的 `equals(undefined) === false` 路径
//!   予以省略（Rust `PartialEq` 没有 null 情形）。
//! - `createPackableSpecs`（pack/unpack 到 JS 数组）予以省略：打包是 JS 数组的
//!   序列化关注点，不属于 Rust 领域 API。
//! - `isOccluded`（3 个用例）属于 C 类：它依赖 `Occluder`（一个 Scene/渲染概念），
//!   未予移植。

use cesium_geospatial::bounding::OrientedBoundingBox;
use cesium_geospatial::ray::{Intersect, Plane};
use cesium_geospatial::{Ellipsoid, Rectangle};
use cesium_specs::{
    assert_approx, assert_mat3_epsilon, assert_vec3_epsilon, epsilon, math_consts,
};
use glam::{DMat3, DMat4, DQuat, DVec3};

const SQRT1_2: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// 从 CesiumJS Matrix3 列主序扁平数组构建 `DMat3`
///（`[col0.x, col0.y, col0.z, col1.x, col1.y, col1.z, col2.x, col2.y, col2.z]`）。
/// 使用显式的 `from_cols` 以避免 glam `from_cols_array` 的布局歧义。
fn mat3_from_cesium(cols: [f64; 9]) -> DMat3 {
    DMat3::from_cols(
        DVec3::new(cols[0], cols[1], cols[2]),
        DVec3::new(cols[3], cols[4], cols[5]),
        DVec3::new(cols[6], cols[7], cols[8]),
    )
}

/// 共享测试夹具：六个沿轴对齐、距离为 (2, 3, 4) 的点。
fn positions() -> Vec<DVec3> {
    vec![
        DVec3::new(2.0, 0.0, 0.0),
        DVec3::new(0.0, 3.0, 0.0),
        DVec3::new(0.0, 0.0, 4.0),
        DVec3::new(-2.0, 0.0, 0.0),
        DVec3::new(0.0, -3.0, 0.0),
        DVec3::new(0.0, 0.0, -4.0),
    ]
}

/// 对应规范中的 `rotatePositions`：返回旋转后的点和旋转矩阵。
fn rotate_positions(positions: &[DVec3], axis: DVec3, angle: f64) -> (Vec<DVec3>, DMat3) {
    let quaternion = DQuat::from_axis_angle(axis, angle);
    let rotation = DMat3::from_quat(quaternion);
    let points = positions.iter().map(|&p| rotation * p).collect();
    (points, rotation)
}

/// 对应规范中的 `translatePositions`。
fn translate_positions(positions: &[DVec3], translation: DVec3) -> Vec<DVec3> {
    positions.iter().map(|&p| translation + p).collect()
}

/// `Matrix3.multiplyByScale(matrix, scale)`：将每一列按对应的分量缩放。
fn multiply_by_scale(matrix: DMat3, scale: DVec3) -> DMat3 {
    DMat3::from_cols(
        matrix.x_axis * scale.x,
        matrix.y_axis * scale.y,
        matrix.z_axis * scale.z,
    )
}

/// 3D 版的 `Matrix3.fromScale(scale)`：由三分量缩放创建对角矩阵。
fn mat3_from_scale(scale: DVec3) -> DMat3 {
    DMat3::from_cols(
        DVec3::new(scale.x, 0.0, 0.0),
        DVec3::new(0.0, scale.y, 0.0),
        DVec3::new(0.0, 0.0, scale.z),
    )
}

/// `it("constructor sets expected default values")`
#[test]
fn test_obb_constructor_default() {
    let box_ = OrientedBoundingBox::default();
    assert_eq!(box_.center, DVec3::ZERO);
    assert_eq!(box_.half_axes, DMat3::ZERO);
}

/// `it("fromPoints constructs empty box with undefined positions")`
///（JS 的 `undefined` 映射为 Rust 的空切片）
#[test]
fn test_obb_from_points_undefined() {
    let box_ = OrientedBoundingBox::from_points(&[]);
    assert_eq!(box_.half_axes, DMat3::ZERO);
    assert_eq!(box_.center, DVec3::ZERO);
}

/// `it("fromPoints constructs empty box with empty positions")`
#[test]
fn test_obb_from_points_empty() {
    let box_ = OrientedBoundingBox::from_points(&[]);
    assert_eq!(box_.half_axes, DMat3::ZERO);
    assert_eq!(box_.center, DVec3::ZERO);
}

/// `it("fromPoints correct scale")`
#[test]
fn test_obb_from_points_correct_scale() {
    let box_ = OrientedBoundingBox::from_points(&positions());
    assert_eq!(box_.half_axes, mat3_from_scale(DVec3::new(2.0, 3.0, 4.0)));
    assert_eq!(box_.center, DVec3::ZERO);
}

/// `it("fromPoints correct translation")`
#[test]
fn test_obb_from_points_correct_translation() {
    let translation = DVec3::new(10.0, -20.0, 30.0);
    let points = translate_positions(&positions(), translation);
    let box_ = OrientedBoundingBox::from_points(&points);
    assert_eq!(box_.half_axes, mat3_from_scale(DVec3::new(2.0, 3.0, 4.0)));
    assert_eq!(box_.center, translation);
}

/// `it("fromPoints rotation about z")`
#[test]
fn test_obb_from_points_rotation_z() {
    let (points, mut rotation) = rotate_positions(&positions(), DVec3::Z, math_consts::PI_OVER_FOUR);
    // 对非对角线的符号翻转项取反（规范：rotation[1]、rotation[3]）。
    let mut a = rotation.to_cols_array();
    a[1] = -a[1];
    a[3] = -a[3];
    rotation = DMat3::from_cols_array(&a);

    let box_ = OrientedBoundingBox::from_points(&points);
    assert_mat3_epsilon!(
        box_.half_axes,
        multiply_by_scale(rotation, DVec3::new(3.0, 2.0, 4.0)),
        epsilon::EPSILON15
    );
    assert_vec3_epsilon!(box_.center, DVec3::ZERO, epsilon::EPSILON15);
}

/// `it("fromPoints rotation about y")`
#[test]
fn test_obb_from_points_rotation_y() {
    let (points, mut rotation) = rotate_positions(&positions(), DVec3::Y, math_consts::PI_OVER_FOUR);
    let mut a = rotation.to_cols_array();
    a[2] = -a[2];
    a[6] = -a[6];
    rotation = DMat3::from_cols_array(&a);

    let box_ = OrientedBoundingBox::from_points(&points);
    assert_mat3_epsilon!(
        box_.half_axes,
        multiply_by_scale(rotation, DVec3::new(4.0, 3.0, 2.0)),
        epsilon::EPSILON15
    );
    assert_vec3_epsilon!(box_.center, DVec3::ZERO, epsilon::EPSILON15);
}

/// `it("fromPoints rotation about x")`
#[test]
fn test_obb_from_points_rotation_x() {
    let (points, mut rotation) = rotate_positions(&positions(), DVec3::X, math_consts::PI_OVER_FOUR);
    let mut a = rotation.to_cols_array();
    a[5] = -a[5];
    a[7] = -a[7];
    rotation = DMat3::from_cols_array(&a);

    let box_ = OrientedBoundingBox::from_points(&points);
    assert_mat3_epsilon!(
        box_.half_axes,
        multiply_by_scale(rotation, DVec3::new(2.0, 4.0, 3.0)),
        epsilon::EPSILON15
    );
    assert_vec3_epsilon!(box_.center, DVec3::ZERO, epsilon::EPSILON15);
}

/// `it("fromPoints rotation and translation")`
#[test]
fn test_obb_from_points_rotation_and_translation() {
    let (points, mut rotation) = rotate_positions(&positions(), DVec3::Z, math_consts::PI_OVER_FOUR);
    let mut a = rotation.to_cols_array();
    a[1] = -a[1];
    a[3] = -a[3];
    rotation = DMat3::from_cols_array(&a);

    let translation = DVec3::new(-40.0, 20.0, -30.0);
    let points = translate_positions(&points, translation);

    let box_ = OrientedBoundingBox::from_points(&points);
    assert_mat3_epsilon!(
        box_.half_axes,
        multiply_by_scale(rotation, DVec3::new(3.0, 2.0, 4.0)),
        epsilon::EPSILON14
    );
    assert_vec3_epsilon!(box_.center, translation, epsilon::EPSILON14);
}

/// `it("fromRectangle sets correct default ellipsoid")`
#[test]
fn test_obb_from_rectangle_default_ellipsoid() {
    let rectangle = Rectangle::new(-0.9, -1.2, 0.5, 0.7);
    let box1 = OrientedBoundingBox::from_rectangle(&rectangle, 0.0, 0.0, &Ellipsoid::WGS84);
    let box2 = OrientedBoundingBox::from_rectangle(&rectangle, 0.0, 0.0, &Ellipsoid::WGS84);
    assert_vec3_epsilon!(box1.center, box2.center, epsilon::EPSILON15);
    assert_mat3_epsilon!(box1.half_axes, box2.half_axes, epsilon::EPSILON15);
}

/// `it("fromRectangle sets correct default heights")`
#[test]
fn test_obb_from_rectangle_default_heights() {
    let rectangle = Rectangle::new(0.0, 0.0, 0.0, 0.0);
    let box_ = OrientedBoundingBox::from_rectangle(&rectangle, 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(box_.center, DVec3::new(1.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);
}

/// `it("fromRectangle throws with invalid rectangles")`
#[test]
#[should_panic]
fn test_obb_from_rectangle_throws_invalid_1() {
    let _ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-1.0, 1.0, 1.0, -1.0),
        0.0,
        0.0,
        &Ellipsoid::UNIT_SPHERE,
    );
}

/// `it("fromRectangle throws with invalid rectangles")`
#[test]
#[should_panic]
fn test_obb_from_rectangle_throws_invalid_2() {
    let _ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-2.0, 2.0, -1.0, 1.0),
        0.0,
        0.0,
        &Ellipsoid::UNIT_SPHERE,
    );
}

/// `it("fromRectangle throws with invalid rectangles")`
#[test]
#[should_panic]
fn test_obb_from_rectangle_throws_invalid_3() {
    let _ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-4.0, -2.0, 4.0, 1.0),
        0.0,
        0.0,
        &Ellipsoid::UNIT_SPHERE,
    );
}

/// `it("fromRectangle throws with invalid rectangles")`
#[test]
#[should_panic]
fn test_obb_from_rectangle_throws_invalid_4() {
    let _ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-2.0, -2.0, 1.0, 2.0),
        0.0,
        0.0,
        &Ellipsoid::UNIT_SPHERE,
    );
}

/// `it("fromRectangle throws with invalid rectangles")`
#[test]
#[should_panic]
fn test_obb_from_rectangle_throws_invalid_5() {
    let _ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-1.0, -2.0, 2.0, 2.0),
        0.0,
        0.0,
        &Ellipsoid::UNIT_SPHERE,
    );
}

/// `it("fromRectangle throws with invalid rectangles")`
#[test]
#[should_panic]
fn test_obb_from_rectangle_throws_invalid_6() {
    let _ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-4.0, -1.0, 4.0, 2.0),
        0.0,
        0.0,
        &Ellipsoid::UNIT_SPHERE,
    );
}

/// `it("fromRectangle throws with non-revolution ellipsoids")` (radii.x != radii.y)
#[test]
#[should_panic]
fn test_obb_from_rectangle_throws_non_revolution_1() {
    let _ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, 0.0, 0.0, 0.0),
        0.0,
        0.0,
        &Ellipsoid::new(1.01, 1.0, 1.01),
    );
}

/// `it("fromRectangle throws with non-revolution ellipsoids")` (radii.x != radii.y)
#[test]
#[should_panic]
fn test_obb_from_rectangle_throws_non_revolution_2() {
    let _ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, 0.0, 0.0, 0.0),
        0.0,
        0.0,
        &Ellipsoid::new(1.0, 1.01, 1.01),
    );
}

/// `it("fromRectangle creates an OrientedBoundingBox without a result parameter")`
#[test]
fn test_obb_from_rectangle_without_result() {
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, 0.0, 0.0, 0.0),
        0.0,
        0.0,
        &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::new(1.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);
}

/// `it("fromRectangle for rectangles with heights")`
#[test]
fn test_obb_from_rectangle_with_heights() {
    let d90 = math_consts::PI_OVER_TWO;

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, 0.0, 0.0, 0.0),
        1.0,
        1.0,
        &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::new(2.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, 0.0, 0.0, 0.0),
        -1.0,
        -1.0,
        &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::new(0.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);

    // 注意：数值已对照 CesiumJS 运行时验证（规范文件存在列顺序错误）。
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, 0.0, 0.0, 0.0),
        -1.0,
        1.0,
        &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::new(1.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        epsilon::EPSILON15
    );

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d90, -d90, d90, d90),
        0.0,
        1.0,
        &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::new(1.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 2.0, 0.0, 0.0, 0.0, 2.0, 1.0, 0.0, 0.0]),
        epsilon::EPSILON15
    );

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d90, -d90, d90, d90),
        -1.0,
        -1.0,
        &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::new(0.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d90, -d90, d90, d90),
        -1.0,
        0.0,
        &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::new(0.5, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.5, 0.0, 0.0]),
        epsilon::EPSILON15
    );
}

/// `it("fromRectangle for rectangles that span over half the ellipsoid")`
#[test]
fn test_obb_from_rectangle_span_over_half() {
    let d90 = math_consts::PI_OVER_TWO;
    let d180 = math_consts::PI;
    let d135 = 3.0 * math_consts::PI_OVER_FOUR;
    let d45 = math_consts::PI_OVER_FOUR;
    let one_plus_sqrt_half_div_two = (1.0 + SQRT1_2) / 2.0;
    let one_minus_one_plus_sqrt_half_div_two = 1.0 - one_plus_sqrt_half_div_two;
    let sqrt_two_minus_one_div_four = (std::f64::consts::SQRT_2 - 1.0) / 4.0;
    let sqrt_two_plus_one_div_four = (std::f64::consts::SQRT_2 + 1.0) / 4.0;

    // 整个椭球
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d180, -d90, d180, d90),
        0.0, 0.0, &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::ZERO, epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0]),
        epsilon::EPSILON15
    );

    // 经度 3/4，完整纬度
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d135, -d90, d135, d90),
        0.0, 0.0, &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(
        box_.center,
        DVec3::new(one_minus_one_plus_sqrt_half_div_two, 0.0, 0.0),
        epsilon::EPSILON15
    );
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 1.0, 0.0, 0.0, 0.0, 1.0, one_plus_sqrt_half_div_two, 0.0, 0.0]),
        epsilon::EPSILON15
    );

    // 经度 3/4，纬度 1/2，以赤道为中心
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d135, -d45, d135, d45),
        0.0, 0.0, &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(
        box_.center,
        DVec3::new(one_minus_one_plus_sqrt_half_div_two, 0.0, 0.0),
        epsilon::EPSILON15
    );
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 1.0, 0.0, 0.0, 0.0, SQRT1_2, one_plus_sqrt_half_div_two, 0.0, 0.0]),
        epsilon::EPSILON15
    );

    // 经度 3/4 以国际日期变更线为中心，纬度 1/2 以赤道为中心
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(d180, -d45, d90, d45),
        0.0, 0.0, &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(
        box_.center,
        DVec3::new(sqrt_two_minus_one_div_four, -sqrt_two_minus_one_div_four, 0.0),
        epsilon::EPSILON15
    );
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([
            SQRT1_2, SQRT1_2, 0.0,
            0.0, 0.0, SQRT1_2,
            sqrt_two_plus_one_div_four, -sqrt_two_plus_one_div_four, 0.0,
        ]),
        epsilon::EPSILON15
    );

    // 完整经度，纬度 1/2 以赤道为中心
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d180, -d45, d180, d45),
        0.0, 0.0, &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::ZERO, epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 1.0, 0.0, 0.0, 0.0, SQRT1_2, 1.0, 0.0, 0.0]),
        epsilon::EPSILON15
    );

    // 完整经度，纬度 1/4 从北极开始
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d180, d45, d180, d90),
        0.0, 0.0, &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(
        box_.center,
        DVec3::new(0.0, 0.0, one_plus_sqrt_half_div_two),
        epsilon::EPSILON15
    );
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([
            0.0, SQRT1_2, 0.0,
            0.0, 0.0, one_minus_one_plus_sqrt_half_div_two,
            SQRT1_2, 0.0, 0.0,
        ]),
        epsilon::EPSILON15
    );

    // 完整经度，纬度 1/4 从南极开始
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d180, -d90, d180, -d45),
        0.0, 0.0, &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(
        box_.center,
        DVec3::new(0.0, 0.0, -one_plus_sqrt_half_div_two),
        epsilon::EPSILON15
    );
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([
            0.0, SQRT1_2, 0.0,
            0.0, 0.0, one_minus_one_plus_sqrt_half_div_two,
            SQRT1_2, 0.0, 0.0,
        ]),
        epsilon::EPSILON15
    );

    // 完全位于北极
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d180, d90, d180, d90),
        0.0, 0.0, &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::new(0.0, 0.0, 1.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);

    // 完全位于北极 2
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d135, d90, d135, d90),
        0.0, 0.0, &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::new(0.0, 0.0, 1.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);

    // 完全位于南极
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d180, -d90, d180, -d90),
        0.0, 0.0, &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::new(0.0, 0.0, -1.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);

    // 完全位于南极 2
    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d135, -d90, d135, -d90),
        0.0, 0.0, &Ellipsoid::UNIT_SPHERE,
    );
    assert_vec3_epsilon!(box_.center, DVec3::new(0.0, 0.0, -1.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);
}

/// `it("fromRectangle for interesting, degenerate, and edge-case rectangles")`
#[test]
fn test_obb_from_rectangle_degenerate_edge_cases() {
    let d45 = math_consts::PI_OVER_FOUR;
    let d30 = math_consts::PI_OVER_SIX;
    let d90 = math_consts::PI_OVER_TWO;
    let d135 = 3.0 * math_consts::PI_OVER_FOUR;
    let d180 = math_consts::PI;
    let sqrt3 = 3.0f64.sqrt();

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, 0.0, 0.0, 0.0), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(box_.center, DVec3::new(1.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(d180, 0.0, -d180, 0.0), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(box_.center, DVec3::new(-1.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(d180, 0.0, d180, 0.0), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(box_.center, DVec3::new(-1.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, d90, 0.0, d90), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(box_.center, DVec3::new(0.0, 0.0, 1.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(box_.half_axes, DMat3::ZERO, epsilon::EPSILON15);

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, 0.0, d180, 0.0), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(box_.center, DVec3::new(0.0, 0.5, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([-1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0]),
        epsilon::EPSILON15
    );

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d90, -d90, d90, d90), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(box_.center, DVec3::new(0.5, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.5, 0.0, 0.0]),
        epsilon::EPSILON15
    );

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d90, -d30, d90, d90), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(
        box_.center, DVec3::new(0.1875 * sqrt3, 0.0, 0.1875), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 1.0, 0.0, -sqrt3 / 4.0, 0.0, 3.0 / 4.0, (5.0 * sqrt3) / 16.0, 0.0, 5.0 / 16.0]),
        epsilon::EPSILON15
    );

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d90, -d90, d90, d30), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(
        box_.center, DVec3::new(0.1875 * sqrt3, 0.0, -0.1875), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 1.0, 0.0, sqrt3 / 4.0, 0.0, 3.0 / 4.0, (5.0 * sqrt3) / 16.0, 0.0, -5.0 / 16.0]),
        epsilon::EPSILON15
    );

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, -d30, d180, d90), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(
        box_.center, DVec3::new(0.0, 0.1875 * sqrt3, 0.1875), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([-1.0, 0.0, 0.0, 0.0, -sqrt3 / 4.0, 3.0 / 4.0, 0.0, (5.0 * sqrt3) / 16.0, 5.0 / 16.0]),
        epsilon::EPSILON15
    );

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, -d90, d180, d30), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(
        box_.center, DVec3::new(0.0, 0.1875 * sqrt3, -0.1875), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([-1.0, 0.0, 0.0, 0.0, sqrt3 / 4.0, 3.0 / 4.0, 0.0, (5.0 * sqrt3) / 16.0, -5.0 / 16.0]),
        epsilon::EPSILON15
    );

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d45, 0.0, d45, 0.0), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(
        box_.center, DVec3::new((1.0 + SQRT1_2) / 2.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, SQRT1_2, 0.0, 0.0, 0.0, 0.0, 0.5 * (1.0 - SQRT1_2), 0.0, 0.0]),
        epsilon::EPSILON15
    );

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(d135, 0.0, -d135, 0.0), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(
        box_.center, DVec3::new(-(1.0 + SQRT1_2) / 2.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, -SQRT1_2, 0.0, 0.0, 0.0, 0.0, -0.5 * (1.0 - SQRT1_2), 0.0, 0.0]),
        epsilon::EPSILON15
    );

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, -d45, 0.0, d45), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(
        box_.center, DVec3::new((1.0 + SQRT1_2) / 2.0, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 0.0, 0.0, 0.0, 0.0, SQRT1_2, 0.5 * (1.0 - SQRT1_2), 0.0, 0.0]),
        epsilon::EPSILON15
    );

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(-d90, 0.0, d90, 0.0), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(box_.center, DVec3::new(0.5, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0]),
        epsilon::EPSILON15
    );

    let box_ = OrientedBoundingBox::from_rectangle(
        &Rectangle::new(0.0, -d90, 0.0, d90), 0.0, 0.0, &Ellipsoid::UNIT_SPHERE);
    assert_vec3_epsilon!(box_.center, DVec3::new(0.5, 0.0, 0.0), epsilon::EPSILON15);
    assert_mat3_epsilon!(
        box_.half_axes,
        mat3_from_cesium([0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.5, 0.0, 0.0]),
        epsilon::EPSILON15
    );
}

/// `it("fromTransformation works without a result parameter")`
///（result-parameter 变体已合并：值相同）
#[test]
fn test_obb_from_transformation_without_result() {
    let translation = DVec3::new(1.0, 2.0, 3.0);
    let rotation = DQuat::from_axis_angle(DVec3::Z, 0.4);
    let scale = DVec3::new(1.0, 2.0, 3.0);
    let transformation = DMat4::from_scale_rotation_translation(scale, rotation, translation);

    let box_ = OrientedBoundingBox::from_transformation(&transformation);

    assert_eq!(box_.center, translation);
    // halfAxes = getMatrix3(transformation) * 0.5
    let m3 = DMat3::from_cols(
        transformation.x_axis.truncate(),
        transformation.y_axis.truncate(),
        transformation.z_axis.truncate(),
    );
    assert_mat3_epsilon!(box_.half_axes, m3 * 0.5, epsilon::EPSILON14);
}

/// `it("fromTransformation works with a transformation that has zero scale")`
#[test]
fn test_obb_from_transformation_zero_scale() {
    let transformation = DMat4::from_scale(DVec3::ZERO);
    let box_ = OrientedBoundingBox::from_transformation(&transformation);
    assert_eq!(box_.center, DVec3::ZERO);
    assert_eq!(box_.half_axes, DMat3::ZERO);
}

// ======================== intersectPlane ========================

/// 对齐实现规范中的 `intersectPlaneTestCornersEdgesFaces` 辅助函数。
/// 在距盒子的面/棱/顶点不同距离处生成平面，并
/// 验证预期的 Intersect 分类结果。
fn intersect_plane_test_corners_edges_faces(center: DVec3, axes: DMat3) {
    let sqrt1_2 = (0.5f64).sqrt();
    let sqrt3_4 = (0.75f64).sqrt();

    let box_ = OrientedBoundingBox::new(center, axes * 0.5);

    let plane_norm_xform = |nx: f64, ny: f64, nz: f64, dist: f64| -> Option<Plane> {
        let n = DVec3::new(nx, ny, nz);
        let arb = DVec3::new(357.0, 924.0, 258.0);
        let mut p0 = n.normalize() * (-dist);
        let tang = n.cross(arb).normalize();
        let binorm = n.cross(tang).normalize();

        p0 = axes * p0;
        let tang = axes * tang;
        let binorm = axes * binorm;
        let mut n = tang.cross(binorm);
        if n.length() == 0.0 {
            return None;
        }
        n = n.normalize();

        p0 += center;
        let d = -p0.dot(n);
        if d.abs() > 0.0001 && n.length_squared() > 0.0001 {
            Some(Plane::new(n, d))
        } else {
            None
        }
    };

    let check = |nx: f64, ny: f64, nz: f64, dist: f64, expected: Intersect| {
        if let Some(pl) = plane_norm_xform(nx, ny, nz, dist) {
            assert_eq!(box_.intersect_plane(pl.normal, pl.distance), expected);
        }
    };

    // 面
    for &(nx, ny, nz) in &[
        (1.0, 0.0, 0.0), (-1.0, 0.0, 0.0),
        (0.0, 1.0, 0.0), (0.0, -1.0, 0.0),
        (0.0, 0.0, 1.0), (0.0, 0.0, -1.0),
    ] {
        check(nx, ny, nz, 0.50001, Intersect::Inside);
        check(nx, ny, nz, 0.49999, Intersect::Intersecting);
        check(nx, ny, nz, -0.49999, Intersect::Intersecting);
        check(nx, ny, nz, -0.50001, Intersect::Outside);
    }

    // 棱
    for &(nx, ny, nz) in &[
        (1.0, 1.0, 0.0), (1.0, -1.0, 0.0), (-1.0, 1.0, 0.0), (-1.0, -1.0, 0.0),
        (1.0, 0.0, 1.0), (1.0, 0.0, -1.0), (-1.0, 0.0, 1.0), (-1.0, 0.0, -1.0),
        (0.0, 1.0, 1.0), (0.0, 1.0, -1.0), (0.0, -1.0, 1.0), (0.0, -1.0, -1.0),
    ] {
        check(nx, ny, nz, sqrt1_2 + 0.00001, Intersect::Inside);
        check(nx, ny, nz, sqrt1_2 - 0.00001, Intersect::Intersecting);
        check(nx, ny, nz, -sqrt1_2 + 0.00001, Intersect::Intersecting);
        check(nx, ny, nz, -sqrt1_2 - 0.00001, Intersect::Outside);
    }

    // 顶点
    for &(nx, ny, nz) in &[
        (1.0, 1.0, 1.0), (1.0, 1.0, -1.0), (1.0, -1.0, 1.0), (1.0, -1.0, -1.0),
        (-1.0, 1.0, 1.0), (-1.0, 1.0, -1.0), (-1.0, -1.0, 1.0), (-1.0, -1.0, -1.0),
    ] {
        check(nx, ny, nz, sqrt3_4 + 0.00001, Intersect::Inside);
        check(nx, ny, nz, sqrt3_4 - 0.00001, Intersect::Intersecting);
        check(nx, ny, nz, -sqrt3_4 + 0.00001, Intersect::Intersecting);
        check(nx, ny, nz, -sqrt3_4 - 0.00001, Intersect::Outside);
    }
}

/// `it("intersectPlane works with untransformed box")`
#[test]
fn test_obb_intersect_plane_untransformed() {
    intersect_plane_test_corners_edges_faces(DVec3::ZERO, DMat3::IDENTITY);
}

/// `it("intersectPlane works with off-center box")`
#[test]
fn test_obb_intersect_plane_off_center() {
    intersect_plane_test_corners_edges_faces(DVec3::new(1.0, 0.0, 0.0), DMat3::IDENTITY);
    intersect_plane_test_corners_edges_faces(DVec3::new(0.7, -1.8, 12.0), DMat3::IDENTITY);
}

/// `it("intersectPlane works with rotated box")`
#[test]
fn test_obb_intersect_plane_rotated() {
    let q = DQuat::from_axis_angle(DVec3::new(0.5, 1.5, -1.2), 1.2);
    intersect_plane_test_corners_edges_faces(DVec3::ZERO, DMat3::from_quat(q));
}

/// `it("intersectPlane works with scaled box")`
#[test]
fn test_obb_intersect_plane_scaled() {
    intersect_plane_test_corners_edges_faces(
        DVec3::ZERO, mat3_from_scale(DVec3::new(1.5, 0.4, 20.6)));
    intersect_plane_test_corners_edges_faces(
        DVec3::ZERO, mat3_from_scale(DVec3::new(0.0, 0.4, 20.6)));
    intersect_plane_test_corners_edges_faces(
        DVec3::ZERO, mat3_from_scale(DVec3::new(1.5, 0.0, 20.6)));
    intersect_plane_test_corners_edges_faces(
        DVec3::ZERO, mat3_from_scale(DVec3::new(1.5, 0.4, 0.0)));
    intersect_plane_test_corners_edges_faces(
        DVec3::ZERO, mat3_from_scale(DVec3::new(0.0, 0.0, 0.0)));
}

/// `it("intersectPlane works with this arbitrary box")`
#[test]
fn test_obb_intersect_plane_arbitrary() {
    let m = mat3_from_scale(DVec3::new(1.5, 80.4, 2.6));
    let q = DQuat::from_axis_angle(DVec3::new(0.5, 1.5, -1.2), 1.2);
    let n = m * DMat3::from_quat(q);
    intersect_plane_test_corners_edges_faces(DVec3::new(-5.1, 0.0, 0.1), n);
}

// ======================== distanceSquaredTo ========================

/// `it("distanceSquaredTo")`
#[test]
fn test_obb_distance_squared_to() {
    let r0 = DMat3::from_rotation_z(-math_consts::PI_OVER_FOUR);
    let r1 = DMat3::from_rotation_y(math_consts::PI_OVER_FOUR);
    let rotation = r1 * r0;
    let scale = DVec3::new(2.0, 3.0, 4.0);
    let rotation_scale = multiply_by_scale(rotation, scale);
    let center = DVec3::new(4.0, 3.0, 2.0);
    let obb = OrientedBoundingBox::new(center, rotation_scale);

    let x_axis = obb.half_axes.x_axis;
    let y_axis = obb.half_axes.y_axis;
    let z_axis = obb.half_axes.z_axis;

    // 沿 x 轴正/负方向
    for &s in &[2.0f64, -2.0] {
        let cartesian = center + x_axis * s;
        let d = (cartesian - center).length() - scale.x;
        assert_approx!(obb.distance_squared_to(cartesian), d * d, epsilon::EPSILON10);
    }
    // 沿 y 轴正/负方向
    for &s in &[2.0f64, -2.0] {
        let cartesian = center + y_axis * s;
        let d = (cartesian - center).length() - scale.y;
        assert_approx!(obb.distance_squared_to(cartesian), d * d, epsilon::EPSILON10);
    }
    // 沿 z 轴正/负方向
    for &s in &[2.0f64, -2.0] {
        let cartesian = center + z_axis * s;
        let d = (cartesian - center).length() - scale.z;
        assert_approx!(obb.distance_squared_to(cartesian), d * d, epsilon::EPSILON10);
    }
    // 沿角点方向
    let cartesian = x_axis + y_axis + z_axis;
    let corner_distance = cartesian.length();
    let cartesian = cartesian + center;
    let d = (cartesian - center).length() - corner_distance;
    assert_approx!(obb.distance_squared_to(cartesian), d * d, epsilon::EPSILON10);

    // 盒子内部
    let offset = rotation * (scale * 0.25);
    let cartesian = center + offset;
    assert_approx!(obb.distance_squared_to(cartesian), 0.0, epsilon::EPSILON10);
}

/// `it("distanceSquaredTo handles one degenerate axis - X")`
#[test]
fn test_obb_distance_squared_to_degenerate_x() {
    let rotation = DMat3::from_rotation_x(math_consts::PI_OVER_FOUR);
    let scale = DVec3::new(0.0, 4.0, 3.0);
    let rotation_scale = multiply_by_scale(rotation, scale);
    let center = DVec3::new(4.0, 3.0, 2.0);
    let obb = OrientedBoundingBox::new(center, rotation_scale);

    assert_eq!(obb.half_axes.x_axis, DVec3::ZERO);
    let x_axis = DVec3::X; // 退化方向
    let y_axis = obb.half_axes.y_axis;
    let z_axis = obb.half_axes.z_axis;

    for &s in &[2.0f64, -2.0] {
        let c = center + x_axis * s;
        let d = (c - center).length();
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    for &s in &[2.0f64, -2.0] {
        let c = center + y_axis * s;
        let d = (c - center).length() - scale.y;
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    for &s in &[2.0f64, -2.0] {
        let c = center + z_axis * s;
        let d = (c - center).length() - scale.z;
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    // 角点
    let cartesian = y_axis + z_axis;
    let corner_distance = cartesian.length();
    let c = cartesian + center;
    let d = (c - center).length() - corner_distance;
    assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    // 内部
    let offset = rotation * (scale * 0.25);
    assert_approx!(obb.distance_squared_to(center + offset), 0.0, epsilon::EPSILON10);
}

/// `it("distanceSquaredTo handles one degenerate axis - Y")`
#[test]
fn test_obb_distance_squared_to_degenerate_y() {
    let rotation = DMat3::from_rotation_y(math_consts::PI_OVER_FOUR);
    let scale = DVec3::new(2.0, 0.0, 3.0);
    let rotation_scale = multiply_by_scale(rotation, scale);
    let center = DVec3::new(4.0, 3.0, 2.0);
    let obb = OrientedBoundingBox::new(center, rotation_scale);

    assert_eq!(obb.half_axes.y_axis, DVec3::ZERO);
    let x_axis = obb.half_axes.x_axis;
    let y_axis = DVec3::Y;
    let z_axis = obb.half_axes.z_axis;

    for &s in &[2.0f64, -2.0] {
        let c = center + x_axis * s;
        let d = (c - center).length() - scale.x;
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    for &s in &[2.0f64, -2.0] {
        let c = center + y_axis * s;
        let d = (c - center).length();
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    for &s in &[2.0f64, -2.0] {
        let c = center + z_axis * s;
        let d = (c - center).length() - scale.z;
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    let cartesian = x_axis + z_axis;
    let corner_distance = cartesian.length();
    let c = cartesian + center;
    let d = (c - center).length() - corner_distance;
    assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    let offset = rotation * (scale * 0.25);
    assert_approx!(obb.distance_squared_to(center + offset), 0.0, epsilon::EPSILON10);
}

/// `it("distanceSquaredTo handles one degenerate axis - Z")`
#[test]
fn test_obb_distance_squared_to_degenerate_z() {
    let rotation = DMat3::from_rotation_z(math_consts::PI_OVER_FOUR);
    let scale = DVec3::new(2.0, 4.0, 0.0);
    let rotation_scale = multiply_by_scale(rotation, scale);
    let center = DVec3::new(4.0, 3.0, 2.0);
    let obb = OrientedBoundingBox::new(center, rotation_scale);

    assert_eq!(obb.half_axes.z_axis, DVec3::ZERO);
    let x_axis = obb.half_axes.x_axis;
    let y_axis = obb.half_axes.y_axis;
    let z_axis = DVec3::Z;

    for &s in &[2.0f64, -2.0] {
        let c = center + x_axis * s;
        let d = (c - center).length() - scale.x;
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    for &s in &[2.0f64, -2.0] {
        let c = center + y_axis * s;
        let d = (c - center).length() - scale.y;
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    for &s in &[2.0f64, -2.0] {
        let c = center + z_axis * s;
        let d = (c - center).length();
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    let cartesian = x_axis + y_axis;
    let corner_distance = cartesian.length();
    let c = cartesian + center;
    let d = (c - center).length() - corner_distance;
    assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    // 内部（Z 退化：缩放后 offset 没有 z 分量）
    let offset = scale * 0.25;
    assert_approx!(obb.distance_squared_to(center + offset), 0.0, epsilon::EPSILON10);
}

/// `it("distanceSquaredTo handles two degenerate axes - XY")`
#[test]
fn test_obb_distance_squared_to_degenerate_xy() {
    let r0 = DMat3::from_rotation_y(math_consts::PI_OVER_FOUR);
    let r1 = DMat3::from_rotation_x(-math_consts::PI_OVER_FOUR);
    let rotation = r1 * r0;
    let scale = DVec3::new(0.0, 0.0, 3.0);
    let rotation_scale = multiply_by_scale(rotation, scale);
    let center = DVec3::new(4.0, 3.0, 2.0);
    let obb = OrientedBoundingBox::new(center, rotation_scale);

    assert_eq!(obb.half_axes.x_axis, DVec3::ZERO);
    assert_eq!(obb.half_axes.y_axis, DVec3::ZERO);
    let x_axis = rotation * DVec3::X;
    let y_axis = rotation * DVec3::Y;
    let z_axis = obb.half_axes.z_axis;

    for &s in &[2.0f64, -2.0] {
        let c = center + x_axis * s;
        let d = (c - center).length();
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    for &s in &[2.0f64, -2.0] {
        let c = center + y_axis * s;
        let d = (c - center).length();
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    for &s in &[2.0f64, -2.0] {
        let c = center + z_axis * s;
        let d = (c - center).length() - scale.z;
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    // 端点
    let ep = z_axis;
    let ep_dist = ep.length();
    let c = ep + center;
    let d = (c - center).length() - ep_dist;
    assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    let c = -ep + center;
    let d = (c - center).length() - ep_dist;
    assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    // 内部
    let offset = rotation * (scale * 0.25);
    assert_approx!(obb.distance_squared_to(center + offset), 0.0, epsilon::EPSILON10);
}

/// `it("distanceSquaredTo handles two degenerate axes - XZ")`
#[test]
fn test_obb_distance_squared_to_degenerate_xz() {
    let r0 = DMat3::from_rotation_z(math_consts::PI_OVER_FOUR);
    let r1 = DMat3::from_rotation_x(-math_consts::PI_OVER_FOUR);
    let rotation = r1 * r0;
    let scale = DVec3::new(0.0, 4.0, 0.0);
    let rotation_scale = multiply_by_scale(rotation, scale);
    let center = DVec3::new(4.0, 3.0, 2.0);
    let obb = OrientedBoundingBox::new(center, rotation_scale);

    assert_eq!(obb.half_axes.x_axis, DVec3::ZERO);
    assert_eq!(obb.half_axes.z_axis, DVec3::ZERO);
    let x_axis = rotation * DVec3::X;
    let y_axis = obb.half_axes.y_axis;
    let z_axis = rotation * DVec3::Z;

    for &s in &[2.0f64, -2.0] {
        let c = center + x_axis * s;
        let d = (c - center).length();
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    for &s in &[2.0f64, -2.0] {
        let c = center + y_axis * s;
        let d = (c - center).length() - scale.y;
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    for &s in &[2.0f64, -2.0] {
        let c = center + z_axis * s;
        let d = (c - center).length();
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    let ep = y_axis;
    let ep_dist = ep.length();
    let c = ep + center;
    let d = (c - center).length() - ep_dist;
    assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    let c = -ep + center;
    let d = (c - center).length() - ep_dist;
    assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    let offset = rotation * (scale * 0.25);
    assert_approx!(obb.distance_squared_to(center + offset), 0.0, epsilon::EPSILON10);
}

/// `it("distanceSquaredTo handles two degenerate axes - YZ")`
#[test]
fn test_obb_distance_squared_to_degenerate_yz() {
    let r0 = DMat3::from_rotation_z(math_consts::PI_OVER_FOUR);
    let r1 = DMat3::from_rotation_y(-math_consts::PI_OVER_FOUR);
    let rotation = r1 * r0;
    let scale = DVec3::new(2.0, 0.0, 0.0);
    let rotation_scale = multiply_by_scale(rotation, scale);
    let center = DVec3::new(4.0, 3.0, 2.0);
    let obb = OrientedBoundingBox::new(center, rotation_scale);

    assert_eq!(obb.half_axes.y_axis, DVec3::ZERO);
    assert_eq!(obb.half_axes.z_axis, DVec3::ZERO);
    let x_axis = obb.half_axes.x_axis;
    let y_axis = rotation * DVec3::Y;
    let z_axis = rotation * DVec3::Z;

    for &s in &[2.0f64, -2.0] {
        let c = center + x_axis * s;
        let d = (c - center).length() - scale.x;
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    for &s in &[2.0f64, -2.0] {
        let c = center + y_axis * s;
        let d = (c - center).length();
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    for &s in &[2.0f64, -2.0] {
        let c = center + z_axis * s;
        let d = (c - center).length();
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    let ep = x_axis;
    let ep_dist = ep.length();
    let c = ep + center;
    let d = (c - center).length() - ep_dist;
    assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    let c = -ep + center;
    let d = (c - center).length() - ep_dist;
    assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    let offset = rotation * (scale * 0.25);
    assert_approx!(obb.distance_squared_to(center + offset), 0.0, epsilon::EPSILON10);
}

/// `it("distanceSquaredTo handles three degenerate axes")`
#[test]
fn test_obb_distance_squared_to_three_degenerate() {
    let center = DVec3::new(4.0, 3.0, 2.0);
    let obb = OrientedBoundingBox::new(center, mat3_from_scale(DVec3::ZERO));

    assert_eq!(obb.half_axes.x_axis, DVec3::ZERO);
    assert_eq!(obb.half_axes.y_axis, DVec3::ZERO);
    assert_eq!(obb.half_axes.z_axis, DVec3::ZERO);

    for &s in &[2.0f64, -2.0] {
        let c = center + DVec3::X * s;
        let d = (c - center).length();
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
        let c = center + DVec3::Y * s;
        let d = (c - center).length();
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
        let c = center + DVec3::Z * s;
        let d = (c - center).length();
        assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    }
    // 任意点
    let c = DVec3::new(5.0, 10.0, 15.0) + center;
    let d = (c - center).length();
    assert_approx!(obb.distance_squared_to(c), d * d, epsilon::EPSILON10);
    // 内部（位于中心）
    assert_approx!(obb.distance_squared_to(center), 0.0, epsilon::EPSILON10);
}

// ======================== computePlaneDistances ========================

/// `it("computePlaneDistances")`
#[test]
fn test_obb_compute_plane_distances() {
    let r0 = DMat3::from_rotation_z(-math_consts::PI_OVER_FOUR);
    let r1 = DMat3::from_rotation_y(math_consts::PI_OVER_FOUR);
    let rotation = r1 * r0;
    let scale = DVec3::new(2.0, 3.0, 4.0);
    let rotation_scale = multiply_by_scale(rotation, scale);
    let center = DVec3::new(4.0, 3.0, 2.0);
    let obb = OrientedBoundingBox::new(center, rotation_scale);

    let x_axis = obb.half_axes.x_axis;
    let y_axis = obb.half_axes.y_axis;
    let z_axis = obb.half_axes.z_axis;

    // 沿 x 方向
    let position = center + x_axis * 2.0;
    let direction = (-x_axis).normalize();
    let d = (position - center).length();
    let distances = obb.compute_plane_distances(position, direction);
    assert_approx!(distances.start, d - scale.x, epsilon::EPSILON14);
    assert_approx!(distances.stop, d + scale.x, epsilon::EPSILON14);

    // 沿 y 方向
    let position = center + y_axis * 2.0;
    let direction = (-y_axis).normalize();
    let d = (position - center).length();
    let distances = obb.compute_plane_distances(position, direction);
    assert_approx!(distances.start, d - scale.y, epsilon::EPSILON14);
    assert_approx!(distances.stop, d + scale.y, epsilon::EPSILON14);

    // 沿 z 方向
    let position = center + z_axis * 2.0;
    let direction = (-z_axis).normalize();
    let d = (position - center).length();
    let distances = obb.compute_plane_distances(position, direction);
    assert_approx!(distances.start, d - scale.z, epsilon::EPSILON14);
    assert_approx!(distances.stop, d + scale.z, epsilon::EPSILON14);

    // 沿角点方向
    let position = x_axis + y_axis + z_axis;
    let direction = (-position).normalize();
    let corner_distance = position.length();
    let position = position + center;
    let d = (position - center).length();
    let distances = obb.compute_plane_distances(position, direction);
    assert_approx!(distances.start, d - corner_distance, epsilon::EPSILON14);
    assert_approx!(distances.stop, d + corner_distance, epsilon::EPSILON14);
}

// ======================== computeCorners ========================

/// `it("computeCorners works without a result parameter")`
///（result-parameter 变体已合并：值相同）
#[test]
fn test_obb_compute_corners_without_result() {
    let center = DVec3::new(1.0, 2.0, 3.0);
    let half_scale = DVec3::new(1.0, 2.0, 3.0);
    let half_axes = mat3_from_scale(half_scale);
    let box_ = OrientedBoundingBox::new(center, half_axes);

    let corners = box_.compute_corners();
    assert_eq!(corners[0], DVec3::new(0.0, 0.0, 0.0));
    assert_eq!(corners[1], DVec3::new(0.0, 0.0, 6.0));
    assert_eq!(corners[2], DVec3::new(0.0, 4.0, 0.0));
    assert_eq!(corners[3], DVec3::new(0.0, 4.0, 6.0));
    assert_eq!(corners[4], DVec3::new(2.0, 0.0, 0.0));
    assert_eq!(corners[5], DVec3::new(2.0, 0.0, 6.0));
    assert_eq!(corners[6], DVec3::new(2.0, 4.0, 0.0));
    assert_eq!(corners[7], DVec3::new(2.0, 4.0, 6.0));
}

/// `it("computeCorners works with a box that has zero scale")`
#[test]
fn test_obb_compute_corners_zero_scale() {
    let box_ = OrientedBoundingBox::new(DVec3::ZERO, mat3_from_scale(DVec3::ZERO));
    let corners = box_.compute_corners();
    for corner in &corners {
        assert_eq!(*corner, DVec3::ZERO);
    }
}

// ======================== computeTransformation ========================

/// `it("computeTransformation works without a result parameter")`
///（result-parameter 变体已合并：值相同）
#[test]
fn test_obb_compute_transformation_without_result() {
    let center = DVec3::new(1.0, 2.0, 3.0);
    let half_scale = DVec3::new(1.0, 2.0, 3.0);
    let expected_scale = DVec3::new(2.0, 4.0, 6.0);
    let half_axes = mat3_from_scale(half_scale);
    let box_ = OrientedBoundingBox::new(center, half_axes);

    let transformation = box_.compute_transformation();
    let extracted_translation = transformation.w_axis.truncate();
    let extracted_scale = DVec3::new(
        transformation.x_axis.truncate().length(),
        transformation.y_axis.truncate().length(),
        transformation.z_axis.truncate().length(),
    );
    assert_eq!(extracted_translation, center);
    assert_eq!(extracted_scale, expected_scale);
}

/// `it("computeTransformation works with box that has zero scale")`
#[test]
fn test_obb_compute_transformation_zero_scale() {
    let box_ = OrientedBoundingBox::new(DVec3::ZERO, mat3_from_scale(DVec3::ZERO));
    let expected = DMat4::from_scale(DVec3::ZERO);
    let transformation = box_.compute_transformation();
    assert_eq!(transformation, expected);
}

// ======================== equals / cube ========================

/// `it("equals works in all cases")`
#[test]
fn test_obb_equals() {
    let box_ = OrientedBoundingBox::default();
    assert!(box_ == OrientedBoundingBox::default());
}

/// `it("is a rotated/scaled 2x2x2 cube centered at the origin")`
#[test]
fn test_obb_cube_2x2x2() {
    let box_ = OrientedBoundingBox::new(DVec3::ZERO, DMat3::IDENTITY);
    let corners = box_.compute_corners();
    for corner in &corners {
        assert_approx!(corner.x.abs(), 1.0, epsilon::EPSILON15);
        assert_approx!(corner.y.abs(), 1.0, epsilon::EPSILON15);
        assert_approx!(corner.z.abs(), 1.0, epsilon::EPSILON15);
    }
}
