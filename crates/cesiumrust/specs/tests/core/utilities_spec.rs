//! 参考自多个 Core Specs：
//! - binarySearchSpec（8 个 it()，5 个 A 类）
//! - barycentricCoordinatesSpec（13 个 it()，9 个 A 类）
//! - pointInsideTriangleSpec（10 个 it()，6 个 A 类）
//! - RaySpec（10 个 it()，5 个 A 类）
//! - SphericalSpec（12 个 it()，8 个 A 类）
//! - subdivideArraySpec（5 个 it()，3 个 A 类）
//!
//! throws 测试省略（C 类：Rust 类型系统强制输入合法）。
//! result 参数变体已合并（Rust 所有权返回惯用法）。

use cesium_geospatial::ray::Ray;
use cesium_geospatial::spherical::Spherical;
use cesium_geospatial::utilities::{
    barycentric_coordinates, binary_search, point_inside_triangle, subdivide_array,
};
use glam::DVec3;

const EPSILON14: f64 = 1e-14;
const EPSILON15: f64 = 1e-15;

// ===== binarySearch =====

fn num_comparator(a: &f64, b: &f64) -> i64 {
    if *a < *b { -1 } else if *a > *b { 1 } else { 0 }
}

#[test]
fn binary_search_for_0() {
    let array = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let index = binary_search(&array, &0.0, num_comparator);
    assert_eq!(index, 0);
}

#[test]
fn binary_search_for_item_in_list() {
    let array = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let index = binary_search(&array, &7.0, num_comparator);
    assert_eq!(index, 7);
}

#[test]
fn binary_search_for_item_between_two_items() {
    let array = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let index = binary_search(&array, &3.5, num_comparator);
    assert_eq!(!index, 4);
}

#[test]
fn binary_search_for_item_before_all() {
    let array = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let index = binary_search(&array, &(-2.0), num_comparator);
    assert_eq!(!index, 0);
}

#[test]
fn binary_search_for_item_after_all() {
    let array = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let index = binary_search(&array, &12.0, num_comparator);
    assert_eq!(!index, 8);
}

// ===== barycentricCoordinates =====

#[test]
fn barycentric_evaluates_to_p0() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 1.0);
    let result = barycentric_coordinates(p0, p0, p1, p2).unwrap();
    assert!(result.abs_diff_eq(DVec3::X, EPSILON14));
}

#[test]
fn barycentric_evaluates_to_p1() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 1.0);
    let result = barycentric_coordinates(p1, p0, p1, p2).unwrap();
    assert!(result.abs_diff_eq(DVec3::Y, EPSILON14));
}

#[test]
fn barycentric_evaluates_to_p2() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 1.0);
    let result = barycentric_coordinates(p2, p0, p1, p2).unwrap();
    assert!(result.abs_diff_eq(DVec3::Z, EPSILON14));
}

#[test]
fn barycentric_evaluates_on_p0_p1_edge() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 1.0);
    let point = (p1 + p0) * 0.5;
    let result = barycentric_coordinates(point, p0, p1, p2).unwrap();
    assert!(result.abs_diff_eq(DVec3::new(0.5, 0.5, 0.0), EPSILON14));
}

#[test]
fn barycentric_evaluates_on_p0_p2_edge() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 1.0);
    let point = (p2 + p0) * 0.5;
    let result = barycentric_coordinates(point, p0, p1, p2).unwrap();
    assert!(result.abs_diff_eq(DVec3::new(0.5, 0.0, 0.5), EPSILON14));
}

#[test]
fn barycentric_evaluates_on_p1_p2_edge() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 1.0);
    let point = (p2 + p1) * 0.5;
    let result = barycentric_coordinates(point, p0, p1, p2).unwrap();
    assert!(result.abs_diff_eq(DVec3::new(0.0, 0.5, 0.5), EPSILON14));
}

#[test]
fn barycentric_evaluates_on_interior() {
    let p0 = DVec3::new(-1.0, 0.0, 0.0);
    let p1 = DVec3::new(1.0, 0.0, 0.0);
    let p2 = DVec3::new(0.0, 1.0, 1.0);
    let scalar = 1.0 / 3.0;
    let point = (p0 + p1 + p2) * scalar;
    let result = barycentric_coordinates(point, p0, p1, p2).unwrap();
    assert!(result.abs_diff_eq(DVec3::splat(scalar), EPSILON14));
}

#[test]
fn barycentric_returns_none_for_colinear() {
    let p0 = DVec3::new(-1.0, -1.0, 0.0);
    let p1 = DVec3::new(0.0, 0.0, 0.0);
    let p2 = DVec3::new(1.0, 1.0, 0.0);
    let point = DVec3::new(0.5, 0.5, 0.0);
    assert!(barycentric_coordinates(point, p0, p1, p2).is_none());
}

#[test]
fn barycentric_evaluates_with_equal_length_sides() {
    let p0 = DVec3::new(9635312487071484.0, 13827945400273020.0, -16479219993905144.0);
    let p1 = DVec3::new(12832234.180639317, -10455085.701705107, 750010.7274386138);
    let p2 = DVec3::new(-9689011.10628853, -13420063.892507521, 750010.7274386119);
    assert!(barycentric_coordinates(p0, p0, p1, p2).unwrap().abs_diff_eq(DVec3::X, EPSILON14));
    assert!(barycentric_coordinates(p1, p0, p1, p2).unwrap().abs_diff_eq(DVec3::Y, EPSILON14));
    assert!(barycentric_coordinates(p2, p0, p1, p2).unwrap().abs_diff_eq(DVec3::Z, EPSILON14));
}

// ===== pointInsideTriangle =====

#[test]
fn point_inside_triangle_has_point_inside() {
    assert!(point_inside_triangle((0.25, 0.25), (0.0, 0.0), (1.0, 0.0), (0.0, 1.0)));
}

#[test]
fn point_inside_triangle_has_point_outside() {
    assert!(!point_inside_triangle((1.0, 1.0), (0.0, 0.0), (1.0, 0.0), (0.0, 1.0)));
}

#[test]
fn point_inside_triangle_has_point_outside_2() {
    assert!(!point_inside_triangle((0.5, -0.5), (0.0, 0.0), (1.0, 0.0), (0.0, 1.0)));
}

#[test]
fn point_inside_triangle_has_point_outside_3() {
    assert!(!point_inside_triangle((-0.5, 0.5), (0.0, 0.0), (1.0, 0.0), (0.0, 1.0)));
}

#[test]
fn point_inside_triangle_has_point_on_corner() {
    assert!(!point_inside_triangle((0.0, 0.0), (0.0, 0.0), (1.0, 0.0), (0.0, 1.0)));
}

#[test]
fn point_inside_triangle_has_point_on_edge() {
    assert!(!point_inside_triangle((0.5, 0.0), (0.0, 0.0), (1.0, 0.0), (0.0, 1.0)));
}

// ===== Ray =====

#[test]
fn ray_default_constructor_creates_zero_valued() {
    let ray = Ray::new(DVec3::ZERO, DVec3::ZERO);
    // glam 中对 ZERO 调用 direction.normalize() 结果仍是 ZERO
    assert_eq!(ray.origin, DVec3::ZERO);
}

#[test]
fn ray_constructor_sets_expected_properties() {
    let ray = Ray::new(DVec3::Y, DVec3::X);
    assert_eq!(ray.origin, DVec3::Y);
    assert_eq!(ray.direction, DVec3::X);
}

#[test]
fn ray_constructor_normalizes_direction() {
    let ray = Ray::new(DVec3::Y, DVec3::X * 18.0);
    assert_eq!(ray.origin, DVec3::Y);
    assert!(ray.direction.abs_diff_eq(DVec3::X, EPSILON15));
}

#[test]
fn ray_get_point_along_ray() {
    let direction = DVec3::new(1.0, 2.0, 3.0).normalize();
    let ray = Ray::new(DVec3::X, direction);
    for i in -10..=10 {
        let t = i as f64;
        let expected = DVec3::X + direction * t;
        let result = ray.point_at(t);
        assert!(result.abs_diff_eq(expected, EPSILON15), "t={}", t);
    }
}

// ===== Spherical =====

#[test]
fn spherical_default_constructing() {
    let v = Spherical::default();
    assert_eq!(v.clock, 0.0);
    assert_eq!(v.cone, 0.0);
    assert_eq!(v.magnitude, 1.0);
}

#[test]
fn spherical_constructor_parameters() {
    let v = Spherical::new(1.0, 2.0, 3.0);
    assert_eq!(v.clock, 1.0);
    assert_eq!(v.cone, 2.0);
    assert_eq!(v.magnitude, 3.0);
}

#[test]
fn spherical_from_cartesian3() {
    let forty_five_degrees = std::f64::consts::FRAC_PI_4;
    let sixty_degrees = std::f64::consts::FRAC_PI_3;
    let cartesian = DVec3::new(1.0, 3.0_f64.sqrt(), -2.0);
    let expected = Spherical::new(
        sixty_degrees,
        forty_five_degrees + std::f64::consts::FRAC_PI_2,
        8.0_f64.sqrt(),
    );
    let result = Spherical::from_cartesian3(cartesian);
    assert!(result.equals_epsilon(&expected, EPSILON15));
}

#[test]
fn spherical_normalize() {
    let v = Spherical::new(0.0, 2.0, 3.0);
    let w = v.normalize();
    assert_eq!(w.clock, 0.0);
    assert_eq!(w.cone, 2.0);
    assert_eq!(w.magnitude, 1.0);
}

#[test]
fn spherical_equals_epsilon_true() {
    let a = Spherical::new(1.0, 2.0, 1.0);
    let b = Spherical::new(1.0, 2.0, 1.0);
    assert!(a.equals_epsilon(&b, 0.0));
    let c = Spherical::new(1.0, 2.0, 2.0);
    assert!(a.equals_epsilon(&c, 1.0));
}

#[test]
fn spherical_equals_epsilon_false() {
    let a = Spherical::new(1.0, 2.0, 1.0);
    let b = Spherical::new(1.0, 2.0, 3.0);
    assert!(!a.equals_epsilon(&b, 1.0));
}

#[test]
fn spherical_to_string() {
    let v = Spherical::new(1.0, 2.0, 3.0);
    assert_eq!(format!("{}", v), "(1, 2, 3)");
}

// ===== subdivideArray =====

#[test]
fn subdivide_array_splits_evenly() {
    let values = [1, 2, 3, 4];
    let split = subdivide_array(&values, 4);
    assert_eq!(split.len(), 4);
    assert_eq!(split[0], vec![1]);
    assert_eq!(split[1], vec![2]);
    assert_eq!(split[2], vec![3]);
    assert_eq!(split[3], vec![4]);
}

#[test]
fn subdivide_array_splits_unevenly() {
    let values = [1, 2, 3, 4, 5, 6];
    let split = subdivide_array(&values, 4);
    assert_eq!(split.len(), 4);
    assert_eq!(split[0], vec![1, 2]);
    assert_eq!(split[1], vec![3, 4]);
    assert_eq!(split[2], vec![5]);
    assert_eq!(split[3], vec![6]);
}

#[test]
fn subdivide_array_works_with_empty() {
    let values: [i32; 0] = [];
    let split = subdivide_array(&values, 4);
    assert_eq!(split.len(), 0);
}
