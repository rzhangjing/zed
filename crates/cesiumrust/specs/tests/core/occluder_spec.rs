//! 参考自 `Specs/Core/OccluderSpec`（30 个 it()，18 个 A 类）
//!
//! 省略 B 类（throws）测试，因为 Rust 的类型系统会强制输入合法。

use cesium_geospatial::bounding::BoundingSphere;
use cesium_geospatial::occluder::{Occluder, Visibility};
use cesium_geospatial::{Ellipsoid, Rectangle};
use glam::DVec3;

#[test]
fn can_entirely_eclipse_a_smaller_occludee() {
    let giant_sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, -1.5), 0.5);
    let little_sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, -2.75), 0.25);
    let camera_position = DVec3::ZERO;
    let occluder = Occluder::new(&giant_sphere, camera_position);
    assert_eq!(occluder.is_bounding_sphere_visible(&little_sphere), false);
    assert_eq!(occluder.compute_visibility(&little_sphere), Visibility::None);
}

#[test]
fn can_have_a_fully_visible_occludee() {
    let big_sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, -1.5), 0.5);
    let little_sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, -2.75), 0.25);
    let camera_position = DVec3::ZERO;
    let occluder = Occluder::new(&little_sphere, camera_position);
    assert!(occluder.radius() < big_sphere.radius);
    assert_eq!(occluder.is_bounding_sphere_visible(&big_sphere), true);
    assert_eq!(occluder.compute_visibility(&big_sphere), Visibility::Full);
}

#[test]
fn blocks_the_occludee_when_both_are_aligned_and_the_same_size() {
    let sphere1 = BoundingSphere::new(DVec3::new(0.0, 0.0, -1.5), 0.5);
    let sphere2 = BoundingSphere::new(DVec3::new(0.0, 0.0, -2.5), 0.5);
    let camera_position = DVec3::ZERO;
    let occluder = Occluder::new(&sphere1, camera_position);
    assert_eq!(occluder.is_bounding_sphere_visible(&sphere2), false);
    assert_eq!(occluder.compute_visibility(&sphere2), Visibility::None);
}

#[test]
fn can_have_a_fully_visible_occludee_lateral() {
    let sphere1 = BoundingSphere::new(DVec3::new(-1.25, 0.0, -1.5), 0.5);
    let sphere2 = BoundingSphere::new(DVec3::new(1.25, 0.0, -1.5), 0.5);
    let camera_position = DVec3::ZERO;
    let occluder = Occluder::new(&sphere1, camera_position);
    assert_eq!(occluder.compute_visibility(&sphere2), Visibility::Full);
}

#[test]
fn can_partially_block_an_occludee_without_intersecting() {
    let camera_position = DVec3::ZERO;
    let occluder_bs = BoundingSphere::new(DVec3::new(0.0, 0.0, -2.0), 1.0);
    let occluder = Occluder::new(&occluder_bs, camera_position);
    let occludee_bs = BoundingSphere::new(DVec3::new(0.5, 0.5, -3.0), 1.0);
    assert_eq!(occluder.compute_visibility(&occludee_bs), Visibility::Partial);
}

#[test]
fn can_partially_block_an_occludee_when_it_intersects_laterally() {
    let camera_position = DVec3::ZERO;
    let occluder_bs = BoundingSphere::new(DVec3::new(-0.5, 0.0, -1.0), 1.0);
    let occluder = Occluder::new(&occluder_bs, camera_position);
    let occludee_bs = BoundingSphere::new(DVec3::new(0.5, 0.0, -1.0), 1.0);
    assert_eq!(occluder.compute_visibility(&occludee_bs), Visibility::Partial);
}

#[test]
fn can_partially_block_an_occludee_when_it_intersects_vertically() {
    let camera_position = DVec3::ZERO;
    let occluder_bs = BoundingSphere::new(DVec3::new(0.0, 0.0, -2.0), 1.0);
    let occluder = Occluder::new(&occluder_bs, camera_position);
    let occludee_bs = BoundingSphere::new(DVec3::new(0.0, 0.5, -2.5), 1.0);
    assert_eq!(occluder.compute_visibility(&occludee_bs), Visibility::Partial);
}

#[test]
fn reports_full_visibility_when_occludee_is_larger_than_occluder() {
    let little_sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, -1.5), 0.5);
    let big_sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, -3.0), 1.0);
    let camera_position = DVec3::ZERO;
    let occluder = Occluder::new(&little_sphere, camera_position);
    assert_eq!(occluder.compute_visibility(&big_sphere), Visibility::Full);
}

#[test]
fn can_compute_an_occludee_point() {
    let occluder_bs = BoundingSphere::new(DVec3::new(0.0, 0.0, -8.0), 2.0);
    let positions = vec![
        DVec3::new(-1.085, 0.0, -6.221),
        DVec3::new(1.085, 0.0, -6.221),
    ];
    let tile_occluder_sphere = BoundingSphere::from_points(&positions);
    let occludee_position = tile_occluder_sphere.center;
    let result = Occluder::compute_occludee_point(&occluder_bs, occludee_position, &positions);
    assert!(result.is_some());
    let point = result.unwrap();
    let expected = DVec3::new(0.0, 0.0, -5.0);
    assert!(
        (point - expected).length() < 0.1,
        "Expected {:?}, got {:?}",
        expected,
        point
    );
}

#[test]
fn can_compute_a_rotation_vector_major_axis_0() {
    let camera_position = DVec3::ZERO;
    let occluder_bs = BoundingSphere::new(DVec3::new(5.0, 0.0, 0.0), 2.0);
    let occluder = Occluder::new(&occluder_bs, camera_position);
    let occludee_bs = BoundingSphere::new(DVec3::new(8.0, 0.0, 0.0), 1.0);
    let occludee = Occluder::new(&occludee_bs, camera_position);

    let occluder_position = occluder.position();
    let occludee_position = occludee.position();
    let occluder_plane_normal = (occludee_position - occluder_position).normalize();
    let occluder_plane_d = -occluder_plane_normal.dot(occluder_position);

    let temp_vec0 = DVec3::new(
        occluder_plane_normal.x.abs(),
        occluder_plane_normal.y.abs(),
        occluder_plane_normal.z.abs(),
    );
    let mut major_axis = if temp_vec0.x > temp_vec0.y { 0 } else { 1 };
    if (major_axis == 0 && temp_vec0.z > temp_vec0.x)
        || (major_axis == 1 && temp_vec0.z > temp_vec0.y)
    {
        major_axis = 2;
    }
    assert_eq!(major_axis, 0);
    let a_rotation_vector =
        Occluder::any_rotation_vector(occluder_position, occluder_plane_normal, occluder_plane_d);
    assert!(a_rotation_vector.length() > 0.99);
}

#[test]
fn can_compute_a_rotation_vector_major_axis_1() {
    let camera_position = DVec3::ZERO;
    let occluder_bs = BoundingSphere::new(DVec3::new(5.0, 0.0, 0.0), 2.0);
    let occluder = Occluder::new(&occluder_bs, camera_position);
    let occludee_bs = BoundingSphere::new(DVec3::new(7.0, 2.0, 0.0), 1.0);
    let occludee = Occluder::new(&occludee_bs, camera_position);

    let occluder_position = occluder.position();
    let occludee_position = occludee.position();
    let occluder_plane_normal = (occludee_position - occluder_position).normalize();
    let occluder_plane_d = -occluder_plane_normal.dot(occluder_position);

    let temp_vec0 = DVec3::new(
        occluder_plane_normal.x.abs(),
        occluder_plane_normal.y.abs(),
        occluder_plane_normal.z.abs(),
    );
    let mut major_axis = if temp_vec0.x > temp_vec0.y { 0 } else { 1 };
    if (major_axis == 0 && temp_vec0.z > temp_vec0.x)
        || (major_axis == 1 && temp_vec0.z > temp_vec0.y)
    {
        major_axis = 2;
    }
    assert_eq!(major_axis, 1);
    let a_rotation_vector =
        Occluder::any_rotation_vector(occluder_position, occluder_plane_normal, occluder_plane_d);
    assert!(a_rotation_vector.length() > 0.99);
}

#[test]
fn can_compute_a_rotation_vector_major_axis_2() {
    let camera_position = DVec3::ZERO;
    let occluder_bs = BoundingSphere::new(DVec3::new(5.0, 0.0, 0.0), 2.0);
    let occluder = Occluder::new(&occluder_bs, camera_position);
    let occludee_bs = BoundingSphere::new(DVec3::new(6.0, 0.0, 2.0), 1.0);
    let occludee = Occluder::new(&occludee_bs, camera_position);

    let occluder_position = occluder.position();
    let occludee_position = occludee.position();
    let occluder_plane_normal = (occludee_position - occluder_position).normalize();
    let occluder_plane_d = -occluder_plane_normal.dot(occluder_position);

    let temp_vec0 = DVec3::new(
        occluder_plane_normal.x.abs(),
        occluder_plane_normal.y.abs(),
        occluder_plane_normal.z.abs(),
    );
    let mut major_axis = if temp_vec0.x > temp_vec0.y { 0 } else { 1 };
    if (major_axis == 0 && temp_vec0.z > temp_vec0.x)
        || (major_axis == 1 && temp_vec0.z > temp_vec0.y)
    {
        major_axis = 2;
    }
    assert_eq!(major_axis, 2);
    let a_rotation_vector =
        Occluder::any_rotation_vector(occluder_position, occluder_plane_normal, occluder_plane_d);
    assert!(a_rotation_vector.length() > 0.99);
}

#[test]
fn can_have_an_invisible_occludee_point() {
    let camera_position = DVec3::new(0.0, 0.0, -8.0);
    let occluder_bs = BoundingSphere::new(DVec3::new(0.0, 0.0, -8.0), 2.0);
    let occluder = Occluder::new(&occluder_bs, camera_position);
    let positions = vec![
        DVec3::new(-0.25, 0.0, -5.3),
        DVec3::new(0.25, 0.0, -5.3),
    ];
    let tile_occluder_sphere = BoundingSphere::from_points(&positions);
    let occludee_position = tile_occluder_sphere.center;
    let result = Occluder::compute_occludee_point(&occluder_bs, occludee_position, &positions);
    assert!(result.is_some());
    let point = result.unwrap();
    let bs = BoundingSphere::new(point, 0.0);
    assert_eq!(occluder.is_bounding_sphere_visible(&bs), false);
    assert_eq!(occluder.compute_visibility(&bs), Visibility::None);
}

#[test]
fn can_have_a_visible_occludee_point() {
    let camera_position = DVec3::new(3.0, 0.0, -8.0);
    let occluder_bs = BoundingSphere::new(DVec3::new(0.0, 0.0, -8.0), 2.0);
    let occluder = Occluder::new(&occluder_bs, camera_position);
    let positions = vec![
        DVec3::new(-0.25, 0.0, -5.3),
        DVec3::new(0.25, 0.0, -5.3),
    ];
    let tile_occluder_sphere = BoundingSphere::from_points(&positions);
    let occludee_position = tile_occluder_sphere.center;
    let result = Occluder::compute_occludee_point(&occluder_bs, occludee_position, &positions);
    assert!(result.is_some());
    let point = result.unwrap();
    assert_eq!(
        occluder.is_bounding_sphere_visible(&BoundingSphere::new(point, 0.0)),
        true
    );
}

#[test]
fn compute_invalid_occludee_point_from_rectangle() {
    let rectangle = Rectangle::MAX_VALUE;
    let ellipsoid = Ellipsoid::WGS84;
    let result = Occluder::compute_occludee_point_from_rectangle(&rectangle, &ellipsoid);
    assert!(result.is_none());
}

#[test]
fn compute_valid_occludee_point_from_rectangle() {
    let edge = std::f64::consts::PI / 32.0;
    let rectangle = Rectangle::new(-edge, -edge, edge, edge);
    let ellipsoid = Ellipsoid::WGS84;
    let positions = rectangle.subsample(&ellipsoid, 0.0);
    let bs = BoundingSphere::from_points(&positions);
    let point = Occluder::compute_occludee_point(
        &BoundingSphere::new(DVec3::ZERO, ellipsoid.minimum_radius()),
        bs.center,
        &positions,
    );
    let actual = Occluder::compute_occludee_point_from_rectangle(&rectangle, &ellipsoid);
    assert_eq!(actual, point);
}

#[test]
fn from_bounding_sphere_works() {
    let camera_position = DVec3::new(3.0, 0.0, -8.0);
    let occluder_bs = BoundingSphere::new(DVec3::new(0.0, 0.0, -8.0), 2.0);
    let occluder0 = Occluder::new(&occluder_bs, camera_position);
    let occluder1 = Occluder::from_bounding_sphere(&occluder_bs, camera_position);

    assert_eq!(occluder1.position(), occluder0.position());
    assert_eq!(occluder1.radius(), occluder0.radius());
}
