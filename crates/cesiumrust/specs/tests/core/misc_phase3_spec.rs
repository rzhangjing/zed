//! Phase 3 杂项规格测试 - 参考自：
//! - Specs/Core/ConstantSplineSpec（14 个 it()）
//! - Specs/Core/QueueSpec（9 个 it()）
//! - Specs/Core/VerticalExaggerationSpec（8 个 it()）
//! - Specs/Core/srgbToLinearSpec（4 个 it()）
//! - Specs/Core/WireframeIndexGeneratorSpec（9 个 it()）
//!
//! A 类测试：33 个（ConstantSpline 5 + Queue 8 + VerticalExaggeration 8 + srgbToLinear 4 + Wireframe 8）

use cesium_animation::ConstantSpline;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::queue::Queue;
use cesium_geospatial::vertical_exaggeration::{get_height, get_position, srgb_to_linear};
use cesium_geospatial::wireframe::{
    create_wireframe_indices, get_wireframe_indices_count, PrimitiveType,
};
use glam::DVec3;

// ============================================================
// ConstantSpline
// ============================================================

#[test]
fn constant_spline_value_returns_input() {
    let value = DVec3::new(1.0, 2.0, 3.0);
    let spline = ConstantSpline::new(value);
    assert_eq!(spline.value, value);
}

#[test]
fn constant_spline_wrap_time_returns_zero() {
    let spline = ConstantSpline::new(DVec3::new(10.0, 0.0, 0.0));
    assert_eq!(spline.wrap_time(-0.5), 0.0);
    assert_eq!(spline.wrap_time(2.5), 0.0);
}

#[test]
fn constant_spline_clamp_time_returns_zero() {
    let spline = ConstantSpline::new(DVec3::new(10.0, 0.0, 0.0));
    assert_eq!(spline.clamp_time(-0.5), 0.0);
    assert_eq!(spline.clamp_time(2.5), 0.0);
}

#[test]
fn constant_spline_evaluate_returns_constant() {
    let value = DVec3::new(1.0, 2.0, 3.0);
    let spline = ConstantSpline::new(value);
    assert_eq!(spline.evaluate(0.0), value);
    assert_eq!(spline.evaluate(999.0), value);
}

#[test]
fn constant_spline_evaluate_scalar() {
    let value = DVec3::new(10.0, 0.0, 0.0);
    let spline = ConstantSpline::new(value);
    assert_eq!(spline.evaluate(0.0), value);
}

// ============================================================
// Queue
// ============================================================

#[test]
fn queue_can_enqueue_and_dequeue() {
    let mut queue = Queue::new();
    queue.enqueue(1);
    queue.enqueue(2);
    queue.enqueue(3);

    assert_eq!(queue.dequeue(), Some(1));
    assert_eq!(queue.dequeue(), Some(2));
    assert_eq!(queue.dequeue(), Some(3));
}

#[test]
fn queue_returns_none_when_dequeueing_while_empty() {
    let mut queue: Queue<i32> = Queue::new();
    assert_eq!(queue.dequeue(), None);
}

#[test]
fn queue_updates_length() {
    let mut queue = Queue::new();
    assert_eq!(queue.length(), 0);

    queue.enqueue("a");
    assert_eq!(queue.length(), 1);

    queue.dequeue();
    assert_eq!(queue.length(), 0);
}

#[test]
fn queue_can_peek() {
    let mut queue = Queue::new();
    queue.enqueue(1);
    queue.enqueue(2);

    assert_eq!(queue.peek(), Some(&1));
    assert_eq!(queue.length(), 2);
}

#[test]
fn queue_returns_none_when_peeking_while_empty() {
    let queue: Queue<i32> = Queue::new();
    assert_eq!(queue.peek(), None);
}

#[test]
fn queue_can_check_contains() {
    let mut queue = Queue::new();
    queue.enqueue(1);

    assert!(queue.contains(&1));
    assert!(!queue.contains(&2));
}

#[test]
fn queue_can_clear() {
    let mut queue = Queue::new();
    queue.enqueue(1);
    queue.enqueue(2);

    queue.clear();
    assert_eq!(queue.length(), 0);
}

#[test]
fn queue_can_sort() {
    let mut queue = Queue::new();
    queue.enqueue(99);
    queue.enqueue(6);
    queue.enqueue(1);
    queue.enqueue(53);
    queue.enqueue(4);
    queue.enqueue(0);

    queue.dequeue(); // 移除 99

    queue.sort(|a, b| a.cmp(b));

    assert_eq!(queue.dequeue(), Some(0));
    assert_eq!(queue.dequeue(), Some(1));
    assert_eq!(queue.dequeue(), Some(4));
    assert_eq!(queue.dequeue(), Some(6));
    assert_eq!(queue.dequeue(), Some(53));
}

// ============================================================
// VerticalExaggeration
// ============================================================

#[test]
fn vertical_exaggeration_get_height_unchanged_with_scale_1() {
    assert_eq!(get_height(100.0, 1.0, 0.0), 100.0);
}

#[test]
fn vertical_exaggeration_get_height_scales_up() {
    assert_eq!(get_height(150.0, 2.0, 100.0), 200.0);
}

#[test]
fn vertical_exaggeration_get_height_no_change_at_relative() {
    assert_eq!(get_height(100.0, 1.0, 100.0), 100.0);
}

#[test]
fn vertical_exaggeration_get_height_scales_down() {
    assert_eq!(get_height(100.0, 2.0, 200.0), 0.0);
}

fn from_radians(lon: f64, lat: f64, height: f64) -> DVec3 {
    Ellipsoid::WGS84.cartographic_to_cartesian(
        &cesium_geospatial::cartographic::Cartographic::from_radians(lon, lat, height),
    )
}

#[test]
fn vertical_exaggeration_get_position_unchanged_with_scale_1() {
    let position = from_radians(0.0, 0.0, 100.0);
    let result = get_position(position, &Ellipsoid::WGS84, 1.0, 0.0);
    assert!((result - position).length() < 1e-8);
}

#[test]
fn vertical_exaggeration_get_position_scales_up() {
    let position = from_radians(0.0, 0.0, 150.0);
    let expected = from_radians(0.0, 0.0, 200.0);
    let result = get_position(position, &Ellipsoid::WGS84, 2.0, 100.0);
    assert!((result - expected).length() < 1e-8);
}

#[test]
fn vertical_exaggeration_get_position_no_change_at_relative() {
    let position = from_radians(0.0, 0.0, 100.0);
    let result = get_position(position, &Ellipsoid::WGS84, 1.0, 100.0);
    assert!((result - position).length() < 1e-8);
}

#[test]
fn vertical_exaggeration_get_position_scales_down() {
    let position = from_radians(0.0, 0.0, 100.0);
    let expected = from_radians(0.0, 0.0, 0.0);
    let result = get_position(position, &Ellipsoid::WGS84, 2.0, 200.0);
    assert!((result - expected).length() < 1e-8);
}

// ============================================================
// srgbToLinear
// ============================================================

#[test]
fn srgb_to_linear_converts_0() {
    assert_eq!(srgb_to_linear(0.0), 0.0);
}

#[test]
fn srgb_to_linear_converts_low_value() {
    let result = srgb_to_linear(0.0386);
    assert!(
        (result - 0.003).abs() < 0.0005,
        "got {}",
        result
    );
}

#[test]
fn srgb_to_linear_converts_high_value() {
    let result = srgb_to_linear(0.5);
    assert!(
        (result - 0.214).abs() < 0.0005,
        "got {}",
        result
    );
}

#[test]
fn srgb_to_linear_converts_1() {
    assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-10);
}

// ============================================================
// WireframeIndexGenerator
// ============================================================

#[test]
fn wireframe_returns_none_for_non_triangles() {
    assert!(create_wireframe_indices(PrimitiveType::Points, 6, None).is_none());
    assert!(create_wireframe_indices(PrimitiveType::Lines, 6, None).is_none());
    assert!(create_wireframe_indices(PrimitiveType::LineStrip, 6, None).is_none());
    assert!(create_wireframe_indices(PrimitiveType::LineLoop, 6, None).is_none());
}

#[test]
fn wireframe_works_for_triangles() {
    let expected: Vec<u32> = vec![0, 1, 1, 2, 2, 0, 3, 4, 4, 5, 5, 3];
    let result = create_wireframe_indices(PrimitiveType::Triangles, 6, None).unwrap();
    assert_eq!(result, expected);
}

#[test]
fn wireframe_works_for_triangles_from_indices() {
    let indices: Vec<u32> = vec![1, 0, 2, 4, 5, 3];
    let expected: Vec<u32> = vec![1, 0, 0, 2, 2, 1, 4, 5, 5, 3, 3, 4];
    let result = create_wireframe_indices(PrimitiveType::Triangles, 6, Some(&indices)).unwrap();
    assert_eq!(result, expected);
}

#[test]
fn wireframe_works_for_triangle_strip() {
    let expected: Vec<u32> = vec![
        0, 1, // 第一条边
        1, 2, 2, 0, // 第一个三角形的其余边
        2, 3, 3, 1, // 第二个三角形
        3, 4, 4, 2, // 第三个三角形
        4, 5, 5, 3, // 第四个三角形
    ];
    let result = create_wireframe_indices(PrimitiveType::TriangleStrip, 6, None).unwrap();
    assert_eq!(result, expected);
}

#[test]
fn wireframe_works_for_triangle_strip_from_indices() {
    let indices: Vec<u32> = vec![1, 0, 2, 4, 5, 3];
    let expected: Vec<u32> = vec![
        1, 0, // 第一条边
        0, 2, 2, 1, // 第一个三角形
        2, 4, 4, 0, // 第二个三角形
        4, 5, 5, 2, // 第三个三角形
        5, 3, 3, 4, // 第四个三角形
    ];
    let result =
        create_wireframe_indices(PrimitiveType::TriangleStrip, 6, Some(&indices)).unwrap();
    assert_eq!(result, expected);
}

#[test]
fn wireframe_works_for_triangle_fan() {
    let expected: Vec<u32> = vec![
        0, 1, // 第一条边
        1, 2, 2, 0, // 第一个三角形
        2, 3, 3, 0, // 第二个三角形
        3, 4, 4, 0, // 第三个三角形
        4, 5, 5, 0, // 第四个三角形
    ];
    let result = create_wireframe_indices(PrimitiveType::TriangleFan, 6, None).unwrap();
    assert_eq!(result, expected);
}

#[test]
fn wireframe_works_for_triangle_fan_from_indices() {
    let indices: Vec<u32> = vec![1, 0, 2, 4, 5, 3];
    let expected: Vec<u32> = vec![
        1, 0, // 第一条边
        0, 2, 2, 1, // 第一个三角形
        2, 4, 4, 1, // 第二个三角形
        4, 5, 5, 1, // 第三个三角形
        5, 3, 3, 1, // 第四个三角形
    ];
    let result =
        create_wireframe_indices(PrimitiveType::TriangleFan, 6, Some(&indices)).unwrap();
    assert_eq!(result, expected);
}

#[test]
fn wireframe_get_count() {
    assert_eq!(get_wireframe_indices_count(PrimitiveType::Points, 6), 6);
    assert_eq!(get_wireframe_indices_count(PrimitiveType::Lines, 6), 6);
    assert_eq!(get_wireframe_indices_count(PrimitiveType::Triangles, 6), 12);
    assert_eq!(get_wireframe_indices_count(PrimitiveType::TriangleStrip, 6), 18);
    assert_eq!(get_wireframe_indices_count(PrimitiveType::TriangleFan, 6), 18);
}
