//! 参考自 `Specs/Core/EllipsoidalOccluderSpec`（24 个 it()，约 17 个 A 类）
//!
//! 省略 1 个 throws 测试（C 类：Rust 类型系统）。
//! 3 个 grazingAltitudeLocation 测试延后处理（t16c）。
//! 合并了 3 个 result 参数/throws 变体。

use cesium_geospatial::bounding::BoundingSphere;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::ellipsoidal_occluder::EllipsoidalOccluder;
use cesium_geospatial::rectangle::Rectangle;
use glam::DVec3;

const EPSILON14: f64 = 1e-14;

#[test]
fn uses_ellipsoid() {
    let ellipsoid = Ellipsoid::new(2.0, 3.0, 4.0);
    let occluder = EllipsoidalOccluder::new(ellipsoid, None);
    assert_eq!(*occluder.ellipsoid(), ellipsoid);
}

#[test]
fn is_point_visible_example_works_as_claimed() {
    let camera_position = DVec3::new(0.0, 0.0, 2.5);
    let ellipsoid = Ellipsoid::new(1.0, 1.1, 0.9);
    let occluder = EllipsoidalOccluder::new(ellipsoid, Some(camera_position));
    let point = DVec3::new(0.0, -3.0, -3.0);
    assert!(occluder.is_point_visible(point));
}

#[test]
fn is_scaled_space_point_visible_example_works_as_claimed() {
    let camera_position = DVec3::new(0.0, 0.0, 2.5);
    let ellipsoid = Ellipsoid::new(1.0, 1.1, 0.9);
    let occluder = EllipsoidalOccluder::new(ellipsoid, Some(camera_position));
    let point = DVec3::new(0.0, -3.0, -3.0);
    let scaled_space_point = ellipsoid.transform_position_to_scaled_space(point);
    assert!(occluder.is_scaled_space_point_visible(scaled_space_point));
}

#[test]
fn is_scaled_space_point_visible_possibly_under_ellipsoid_example_works_as_claimed() {
    let camera_position = DVec3::new(0.0, 0.0, 1.0);
    let ellipsoid = Ellipsoid::new(1.0, 1.0, 1.0);
    let occluder = EllipsoidalOccluder::new(ellipsoid, Some(camera_position));
    let height = -0.5;

    // 测试 1：位于对角线上的点，深入单位球一半
    let direction = DVec3::new(1.0, 1.0, 1.0).normalize();
    let point = direction * 0.5;
    let scaled_space_point = occluder.compute_horizon_culling_point(point, &[point]).unwrap();
    let scaled_space_point_shrunk = occluder
        .compute_horizon_culling_point_possibly_under_ellipsoid(point, &[point], Some(height))
        .unwrap();

    assert!(!occluder.is_scaled_space_point_visible(scaled_space_point));
    assert!(occluder.is_scaled_space_point_visible_possibly_under_ellipsoid(
        scaled_space_point_shrunk,
        Some(height)
    ));

    // 测试 2：位于 +y 轴上的点，深入单位球一半
    let direction = DVec3::new(0.0, 1.0, 0.0);
    let point = direction * 0.5;
    let scaled_space_point = occluder.compute_horizon_culling_point(point, &[point]).unwrap();
    let scaled_space_point_shrunk = occluder
        .compute_horizon_culling_point_possibly_under_ellipsoid(point, &[point], Some(height))
        .unwrap();

    assert!(!occluder.is_scaled_space_point_visible(scaled_space_point));
    assert!(!occluder.is_scaled_space_point_visible_possibly_under_ellipsoid(
        scaled_space_point_shrunk,
        Some(height)
    ));
}

#[test]
fn reports_not_visible_when_point_is_directly_behind_ellipsoid() {
    let ellipsoid = Ellipsoid::WGS84;
    let mut occluder = EllipsoidalOccluder::new(ellipsoid, None);
    occluder.set_camera_position(DVec3::new(7000000.0, 0.0, 0.0));

    let point = DVec3::new(-7000000.0, 0.0, 0.0);
    assert!(!occluder.is_point_visible(point));
}

#[test]
fn reports_not_visible_when_point_is_directly_behind_ellipsoid_and_camera_is_inside() {
    let ellipsoid = Ellipsoid::WGS84;
    let mut occluder = EllipsoidalOccluder::new(ellipsoid, None);
    occluder.set_camera_position(DVec3::new(ellipsoid.minimum_radius() - 100.0, 0.0, 0.0));

    let point = DVec3::new(-7000000.0, 0.0, 0.0);
    assert!(!occluder.is_point_visible(point));
}

#[test]
fn reports_visible_when_point_is_in_front_of_ellipsoid() {
    let ellipsoid = Ellipsoid::WGS84;
    let mut occluder = EllipsoidalOccluder::new(ellipsoid, None);
    occluder.set_camera_position(DVec3::new(7000000.0, 0.0, 0.0));

    let point = DVec3::new(6900000.0, 0.0, 0.0);
    assert!(occluder.is_point_visible(point));
}

#[test]
fn reports_visible_when_point_is_in_opposite_direction_from_ellipsoid() {
    let ellipsoid = Ellipsoid::WGS84;
    let mut occluder = EllipsoidalOccluder::new(ellipsoid, None);
    occluder.set_camera_position(DVec3::new(7000000.0, 0.0, 0.0));

    let point = DVec3::new(7100000.0, 0.0, 0.0);
    assert!(occluder.is_point_visible(point));
}

#[test]
fn reports_not_visible_when_point_is_over_horizon() {
    let ellipsoid = Ellipsoid::WGS84;
    let mut occluder = EllipsoidalOccluder::new(ellipsoid, None);
    occluder.set_camera_position(DVec3::new(7000000.0, 0.0, 0.0));

    let point = DVec3::new(4510635.0, 4510635.0, 0.0);
    assert!(!occluder.is_point_visible(point));
}

// --- computeHorizonCullingPoint ---

#[test]
fn compute_horizon_culling_point_returns_point_on_ellipsoid_when_single_position_on_center_line() {
    let ellipsoid = Ellipsoid::new(12345.0, 4567.0, 8910.0);
    let occluder = EllipsoidalOccluder::new(ellipsoid, None);
    let positions = [DVec3::new(12345.0, 0.0, 0.0)];
    let direction_to_point = DVec3::new(1.0, 0.0, 0.0);

    let result = occluder
        .compute_horizon_culling_point(direction_to_point, &positions)
        .unwrap();

    assert!(
        (result.x - 1.0).abs() < EPSILON14,
        "x: {}",
        result.x
    );
    assert!(
        (result.y - 0.0).abs() < EPSILON14,
        "y: {}",
        result.y
    );
    assert!(
        (result.z - 0.0).abs() < EPSILON14,
        "z: {}",
        result.z
    );
}

#[test]
fn compute_horizon_culling_point_returns_none_when_horizon_parallel_to_center_line() {
    let ellipsoid = Ellipsoid::new(12345.0, 4567.0, 8910.0);
    let occluder = EllipsoidalOccluder::new(ellipsoid, None);
    let positions = [DVec3::new(0.0, 4567.0, 0.0)];
    let direction_to_point = DVec3::new(1.0, 0.0, 0.0);

    let result = occluder.compute_horizon_culling_point(direction_to_point, &positions);
    assert!(result.is_none());
}

#[test]
fn compute_horizon_culling_point_returns_none_when_point_in_opposite_direction() {
    let ellipsoid = Ellipsoid::new(12345.0, 4567.0, 8910.0);
    let occluder = EllipsoidalOccluder::new(ellipsoid, None);
    let positions = [DVec3::new(-14000.0, -1000.0, 0.0)];
    let direction_to_point = DVec3::new(1.0, 0.0, 0.0);

    let result = occluder.compute_horizon_culling_point(direction_to_point, &positions);
    assert!(result.is_none());
}

#[test]
fn compute_horizon_culling_point_returns_none_when_any_point_in_opposite_direction() {
    let ellipsoid = Ellipsoid::new(1.0, 1.0, 1.0);
    let occluder = EllipsoidalOccluder::new(ellipsoid, None);
    let positions = [DVec3::new(2.0, 0.0, 0.0), DVec3::new(-1.0, 0.0, 0.0)];
    let direction_to_point = DVec3::new(1.0, 0.0, 0.0);

    let result = occluder.compute_horizon_culling_point(direction_to_point, &positions);
    assert!(result.is_none());
}

#[test]
fn compute_horizon_culling_point_returns_none_when_direction_is_zero() {
    let ellipsoid = Ellipsoid::new(1.0, 1.0, 1.0);
    let occluder = EllipsoidalOccluder::new(ellipsoid, None);
    let positions = [DVec3::new(1.0, 0.0, 0.0)];
    let direction_to_point = DVec3::ZERO;

    let result = occluder.compute_horizon_culling_point(direction_to_point, &positions);
    assert!(result.is_none());
}

#[test]
fn compute_horizon_culling_point_possibly_under_ellipsoid_works() {
    let ellipsoid = Ellipsoid::new(12345.0, 4567.0, 8910.0);
    let occluder = EllipsoidalOccluder::new(ellipsoid, None);
    let positions = [DVec3::new(12344.0, 0.0, 0.0)];
    let direction_to_point = DVec3::new(1.0, 0.0, 0.0);

    let result = occluder
        .compute_horizon_culling_point_possibly_under_ellipsoid(
            direction_to_point,
            &positions,
            Some(-1.0),
        )
        .unwrap();

    assert!(
        (result.x - 1.0).abs() < EPSILON14,
        "x: {}",
        result.x
    );
    assert!(
        (result.y - 0.0).abs() < EPSILON14,
        "y: {}",
        result.y
    );
    assert!(
        (result.z - 0.0).abs() < EPSILON14,
        "z: {}",
        result.z
    );
}

// --- computeHorizonCullingPointFromVertices ---

#[test]
fn compute_horizon_culling_point_from_vertices_produces_same_answers() {
    let ellipsoid = Ellipsoid::new(12345.0, 12345.0, 12345.0);
    let occluder = EllipsoidalOccluder::new(ellipsoid, None);

    let positions = [
        DVec3::new(-12345.0, 12345.0, 12345.0),
        DVec3::new(-12346.0, 12345.0, 12345.0),
        DVec3::new(-12446.0, 12445.0, 12445.0),
    ];
    let bounding_sphere = BoundingSphere::from_points(&positions);

    let center = DVec3::new(-12000.0, 12000.0, 12000.0);

    let mut vertices = Vec::new();
    for &position in &positions {
        vertices.push(position.x - center.x);
        vertices.push(position.y - center.y);
        vertices.push(position.z - center.z);
        vertices.push(1.0);
        vertices.push(2.0);
        vertices.push(3.0);
        vertices.push(4.0);
    }

    let result1 = occluder
        .compute_horizon_culling_point(bounding_sphere.center, &positions)
        .unwrap();
    let result2 = occluder
        .compute_horizon_culling_point_from_vertices(
            bounding_sphere.center,
            &vertices,
            7,
            center,
        )
        .unwrap();

    assert!(
        (result1.x - result2.x).abs() < EPSILON14,
        "x: {} vs {}",
        result1.x,
        result2.x
    );
    assert!(
        (result1.y - result2.y).abs() < EPSILON14,
        "y: {} vs {}",
        result1.y,
        result2.y
    );
    assert!(
        (result1.z - result2.z).abs() < EPSILON14,
        "z: {} vs {}",
        result1.z,
        result2.z
    );
}

#[test]
fn compute_horizon_culling_point_from_vertices_possibly_under_ellipsoid_works() {
    let ellipsoid = Ellipsoid::new(12345.0, 4567.0, 8910.0);
    let occluder = EllipsoidalOccluder::new(ellipsoid, None);
    let vertices = [12344.0, 0.0, 0.0];
    let direction_to_point = DVec3::new(1.0, 0.0, 0.0);
    let center = DVec3::ZERO;

    let result = occluder
        .compute_horizon_culling_point_from_vertices_possibly_under_ellipsoid(
            direction_to_point,
            &vertices,
            3,
            center,
            Some(-1.0),
        )
        .unwrap();

    assert!(
        (result.x - 1.0).abs() < EPSILON14,
        "x: {}",
        result.x
    );
    assert!(
        (result.y - 0.0).abs() < EPSILON14,
        "y: {}",
        result.y
    );
    assert!(
        (result.z - 0.0).abs() < EPSILON14,
        "z: {}",
        result.z
    );
}

// --- computeHorizonCullingPointFromRectangle ---

#[test]
fn compute_horizon_culling_point_from_rectangle_returns_none_for_global_rectangle() {
    let ellipsoid = Ellipsoid::new(12345.0, 12345.0, 12345.0);
    let occluder = EllipsoidalOccluder::new(ellipsoid, None);
    let rectangle = Rectangle::MAX_VALUE;
    let result = occluder.compute_horizon_culling_point_from_rectangle(&rectangle, &ellipsoid);
    assert!(result.is_none());
}
