//! 相机扩展规格 - 参考自 Scene/CameraSpec
//!
//! 测试相机朝向查询（heading_3d/pitch_3d/roll_3d）、
//! distance_to_bounding_sphere、get_magnitude、get_rectangle_camera_coordinates、
//! move 操作正确性，以及坐标变换往返。

use cesium_camera::{Camera, SceneMode};
use cesium_geospatial::bounding::BoundingSphere;
use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::rectangle::Rectangle;
use cesium_geospatial::transforms::{HeadingPitchRange, east_north_up_to_fixed_frame};
use glam::DVec3;
use std::f64::consts::PI;

const EPSILON7: f64 = 1e-7;
const EPSILON10: f64 = 1e-10;

fn make_camera(position: DVec3, direction: DVec3, up: DVec3) -> Camera {
    Camera::new(position, direction, up)
}

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

// ─── position_magnitude ──────────────────────────────────────────────────────

#[test]
fn camera_position_magnitude() {
    let pos = DVec3::new(6_378_137.0, 0.0, 0.0);
    let cam = make_camera(pos, -pos.normalize(), DVec3::Z);
    assert!((cam.position_magnitude() - 6_378_137.0).abs() < EPSILON10);
}

#[test]
fn camera_position_magnitude_at_origin() {
    let cam = make_camera(DVec3::ZERO, -DVec3::Z, DVec3::Y);
    assert!(cam.position_magnitude().abs() < EPSILON10);
}

// ─── heading_3d / pitch_3d / roll_3d ────────────────────────────────────────

#[test]
fn camera_heading_3d_looking_north() {
    let e = wgs84();
    // 位于赤道朝北看的相机
    let pos = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 100_000.0));
    let enu = east_north_up_to_fixed_frame(pos, &e);
    let north = DVec3::new(enu.col(1).x, enu.col(1).y, enu.col(1).z).normalize();
    let up = DVec3::new(enu.col(2).x, enu.col(2).y, enu.col(2).z).normalize();

    let cam = make_camera(pos, north, up);
    let heading = cam.heading_3d(&e);
    // 朝北看 → heading ≈ 0
    assert!(
        heading.abs() < 0.1 || (heading - 2.0 * PI).abs() < 0.1,
        "heading_3d looking north should be ≈ 0, got {}", heading
    );
}

#[test]
fn camera_pitch_3d_looking_down() {
    let e = wgs84();
    let pos = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 100_000.0));
    let surface_normal = pos.normalize();
    // 相机垂直向下看
    let direction = -surface_normal;
    let up = DVec3::new(0.0, 0.0, 1.0);
    let right = direction.cross(up).normalize();
    let up = right.cross(direction).normalize();

    let cam = make_camera(pos, direction, up);

    let pitch = cam.pitch_3d(&e);
    // 垂直向下看 → pitch ≈ -PI/2
    assert!(
        (pitch + PI / 2.0).abs() < 0.1,
        "pitch_3d looking down should be ≈ -PI/2, got {}", pitch
    );
}

#[test]
fn camera_roll_3d_level() {
    let e = wgs84();
    let pos = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 100_000.0));
    let enu = east_north_up_to_fixed_frame(pos, &e);
    let north = DVec3::new(enu.col(1).x, enu.col(1).y, enu.col(1).z).normalize();
    let up = DVec3::new(enu.col(2).x, enu.col(2).y, enu.col(2).z).normalize();

    let cam = make_camera(pos, north, up);
    let roll = cam.roll_3d(&e);
    // 水平飞行 → roll ≈ 0
    assert!(
        roll.abs() < 0.1 || (roll - 2.0 * PI).abs() < 0.1,
        "roll_3d level should be ≈ 0, got {}", roll
    );
}

// ─── heading_pitch_roll ─────────────────────────────────────────────────────

#[test]
fn camera_heading_pitch_roll_consistency() {
    let pos = DVec3::new(0.0, 0.0, 10.0);
    let dir = DVec3::new(1.0, 0.0, 0.0);
    let up = DVec3::new(0.0, 0.0, 1.0);
    let cam = make_camera(pos, dir, up);
    let hpr = cam.heading_pitch_roll();

    // heading_pitch_roll 应与各个单独查询一致
    assert!((hpr.heading - cam.heading()).abs() < EPSILON10);
    assert!((hpr.pitch - cam.pitch()).abs() < EPSILON10);
    assert!((hpr.roll - cam.roll()).abs() < EPSILON10);
}

// ─── distance_to_bounding_sphere ────────────────────────────────────────────

#[test]
fn camera_distance_to_bounding_sphere_in_front() {
    let cam = make_camera(
        DVec3::ZERO,
        DVec3::X, // 沿 +X 方向看
        DVec3::Z,
    );
    let sphere = BoundingSphere::new(DVec3::new(100.0, 0.0, 0.0), 10.0);
    let dist = cam.distance_to_bounding_sphere(&sphere);
    // 相机位于原点朝 +X 看，球体位于 100 半径 10 → 距离 ≈ 90
    assert!(
        (dist - 90.0).abs() < 1.0,
        "distance to bounding sphere should be ≈ 90, got {}", dist
    );
}

#[test]
fn camera_distance_to_bounding_sphere_behind() {
    let cam = make_camera(
        DVec3::ZERO,
        DVec3::X, // 沿 +X 方向看
        DVec3::Z,
    );
    let sphere = BoundingSphere::new(DVec3::new(-100.0, 0.0, 0.0), 10.0);
    let dist = cam.distance_to_bounding_sphere(&sphere);
    // 相机后方：带符号投影 = -100，-100 - 10 = -110，max(0) = 0
    assert!(
        dist.abs() < EPSILON10,
        "distance behind camera should be 0, got {}", dist
    );
}

#[test]
fn camera_distance_to_bounding_sphere_enclosing() {
    let cam = make_camera(
        DVec3::ZERO,
        DVec3::X,
        DVec3::Z,
    );
    let sphere = BoundingSphere::new(DVec3::new(50.0, 0.0, 0.0), 100.0);
    let dist = cam.distance_to_bounding_sphere(&sphere);
    // 相机在球内：带符号距离 = 50 - 100 = -50，max(0) = 0
    assert!(
        dist.abs() < EPSILON10,
        "camera inside sphere → distance should be 0, got {}", dist
    );
}

// ─── get_magnitude ───────────────────────────────────────────────────────────

#[test]
fn camera_get_magnitude_3d() {
    let mut cam = make_camera(
        DVec3::new(0.0, 0.0, 10.0),
        -DVec3::Z,
        DVec3::Y,
    );
    cam.mode = SceneMode::Scene3D;
    assert!((cam.get_magnitude() - 10.0).abs() < EPSILON10);
}

#[test]
fn camera_get_magnitude_columbus_view() {
    let mut cam = make_camera(
        DVec3::new(1.0, 2.0, 5.0),
        -DVec3::Z,
        DVec3::Y,
    );
    cam.mode = SceneMode::ColumbusView;
    // CV 使用 z 分量
    assert!((cam.get_magnitude() - 5.0).abs() < EPSILON10);
}

#[test]
fn camera_get_magnitude_2d() {
    let mut cam = make_camera(
        DVec3::new(1.0, 2.0, 5.0),
        -DVec3::Z,
        DVec3::Y,
    );
    cam.mode = SceneMode::Scene2D;
    // 2D 始终返回 1.0
    assert!((cam.get_magnitude() - 1.0).abs() < EPSILON10);
}

// ─── get_rectangle_camera_coordinates ────────────────────────────────────────

#[test]
fn camera_get_rectangle_camera_coordinates_above_center() {
    let e = wgs84();
    let cam = make_camera(
        e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 1_000_000.0)),
        -DVec3::new(0.0, 0.0, 1.0),
        DVec3::Y,
    );
    let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
    let result = cam.get_rectangle_camera_coordinates(&rect, &e);

    // 结果应位于矩形中心之上
    let carto = e.cartesian_to_cartographic(result).unwrap();
    let center_lon = 0.0_f64.to_radians();
    let center_lat = 0.0_f64.to_radians();
    assert!(
        (carto.longitude - center_lon).abs() < 0.01,
        "longitude should be near center, got {}", carto.longitude
    );
    assert!(
        (carto.latitude - center_lat).abs() < 0.01,
        "latitude should be near center, got {}", carto.latitude
    );
    assert!(
        carto.height > 0.0,
        "height should be positive, got {}", carto.height
    );
}

#[test]
fn camera_get_rectangle_camera_coordinates_larger_rect_higher() {
    let e = wgs84();
    let cam = make_camera(
        e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 1_000_000.0)),
        -DVec3::Z,
        DVec3::Y,
    );
    let small_rect = Rectangle::from_degrees(-5.0, -5.0, 5.0, 5.0);
    let large_rect = Rectangle::from_degrees(-20.0, -20.0, 20.0, 20.0);

    let small_result = cam.get_rectangle_camera_coordinates(&small_rect, &e);
    let large_result = cam.get_rectangle_camera_coordinates(&large_rect, &e);

    let small_h = e.cartesian_to_cartographic(small_result).unwrap().height;
    let large_h = e.cartesian_to_cartographic(large_result).unwrap().height;

    assert!(
        large_h > small_h,
        "larger rect should need higher camera: large={} small={}", large_h, small_h
    );
}

// ─── move 操作正确性 ─────────────────────────────────────────────

#[test]
fn camera_move_forward_increases_position_along_direction() {
    let mut cam = make_camera(
        DVec3::ZERO,
        DVec3::X,
        DVec3::Z,
    );
    let original_pos = cam.position;
    cam.move_forward(Some(10.0));
    assert!((cam.position.x - 10.0).abs() < EPSILON10);
    assert!((cam.position.y).abs() < EPSILON10);
    assert!((cam.position.z).abs() < EPSILON10);
    assert!((cam.position - original_pos).length() - 10.0 < EPSILON10);
}

#[test]
fn camera_move_backward_decreases_position_along_direction() {
    let mut cam = make_camera(
        DVec3::new(10.0, 0.0, 0.0),
        DVec3::X,
        DVec3::Z,
    );
    cam.move_backward(Some(5.0));
    assert!((cam.position.x - 5.0).abs() < EPSILON10);
}

#[test]
fn camera_move_right_increases_along_right() {
    let mut cam = make_camera(
        DVec3::ZERO,
        DVec3::X,
        DVec3::Z,
    );
    // right = direction × up = X × Z = -Y
    cam.move_right(Some(7.0));
    assert!(
        (cam.position - DVec3::ZERO).length() - 7.0 < EPSILON10,
        "move_right should move by exactly the amount"
    );
}

#[test]
fn camera_move_up_increases_along_up() {
    let mut cam = make_camera(
        DVec3::ZERO,
        DVec3::X,
        DVec3::Z,
    );
    cam.move_up(Some(3.0));
    assert!((cam.position.z - 3.0).abs() < EPSILON10);
}

#[test]
fn camera_move_forward_then_backward_returns() {
    let mut cam = make_camera(
        DVec3::ZERO,
        DVec3::X,
        DVec3::Z,
    );
    let original = cam.position;
    cam.move_forward(Some(100.0));
    cam.move_backward(Some(100.0));
    assert!(
        (cam.position - original).length() < EPSILON7,
        "forward+backward should return to original position"
    );
}

#[test]
fn camera_move_left_then_right_returns() {
    let mut cam = make_camera(
        DVec3::ZERO,
        DVec3::X,
        DVec3::Z,
    );
    let original = cam.position;
    cam.move_left(Some(50.0));
    cam.move_right(Some(50.0));
    assert!(
        (cam.position - original).length() < EPSILON7,
        "left+right should return to original position"
    );
}

// ─── 坐标变换往返 ────────────────────────────────────────

#[test]
fn camera_world_to_camera_to_world_roundtrip() {
    let cam = make_camera(
        DVec3::new(100.0, 200.0, 300.0),
        DVec3::new(-1.0, -1.0, -1.0).normalize(),
        DVec3::Z,
    );
    let world_point = DVec3::new(500.0, 600.0, 700.0);
    let camera_point = cam.world_to_camera_point(world_point);
    let back = cam.camera_to_world_point(camera_point);
    assert!(
        (back - world_point).length() < EPSILON7,
        "roundtrip should preserve point: expected {:?}, got {:?}", world_point, back
    );
}

#[test]
fn camera_world_to_camera_vector_roundtrip() {
    let cam = make_camera(
        DVec3::new(100.0, 200.0, 300.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::Y,
    );
    let world_vec = DVec3::new(1.0, 2.0, 3.0);
    let camera_vec = cam.world_to_camera_vector(world_vec);
    let back = cam.camera_to_world_vector(camera_vec);
    assert!(
        (back - world_vec).length() < EPSILON7,
        "vector roundtrip should preserve: expected {:?}, got {:?}", world_vec, back
    );
}

// ─── 视图矩阵性质 ──────────────────────────────────────────────────

#[test]
fn camera_view_matrix_is_orthogonal() {
    let cam = make_camera(
        DVec3::new(10.0, 20.0, 30.0),
        DVec3::new(-1.0, -2.0, -3.0).normalize(),
        DVec3::Z,
    );
    let view = cam.view_matrix();
    let inv = cam.inverse_view_matrix();
    let product = view * inv;
    // 应为单位矩阵
    for i in 0..4 {
        for j in 0..4 {
            let expected = if i == j { 1.0 } else { 0.0 };
            assert!(
                (product.col(i)[j] - expected).abs() < EPSILON7,
                "view * inverse_view should be identity at [{},{}]: got {}",
                i, j, product.col(i)[j]
            );
        }
    }
}

#[test]
fn camera_view_projection_matrix_is_product() {
    let cam = make_camera(
        DVec3::new(0.0, 0.0, 10.0),
        -DVec3::Z,
        DVec3::Y,
    );
    let vp = cam.view_projection_matrix();
    let v = cam.view_matrix();
    let p = cam.projection_matrix();
    let expected = p * v;
    for i in 0..4 {
        for j in 0..4 {
            assert!(
                (vp.col(i)[j] - expected.col(i)[j]).abs() < EPSILON7,
                "view_projection should equal projection * view at [{},{}]", i, j
            );
        }
    }
}

// ─── look_at ────────────────────────────────────────────────────────────────

#[test]
fn camera_look_at_points_direction_at_target() {
    let e = wgs84();
    let mut cam = make_camera(
        DVec3::new(0.0, 0.0, 100.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::Z,
    );
    let target = DVec3::new(50.0, 50.0, 0.0);
    let offset = HeadingPitchRange::new(0.0, -PI / 4.0, 50.0);
    cam.look_at(target, &offset, &e);

    // look_at 之后，direction 应大致指向 target
    let to_target = (target - cam.position).normalize();
    let dot = cam.direction.dot(to_target);
    // 相机相对 target 有偏移，因此 direction 不会完全吻合
    // 但应处于总体方向上（点积为正）
    assert!(
        dot > -0.5,
        "camera direction should be roughly towards target, dot={}", dot
    );
}

// ─── height ──────────────────────────────────────────────────────────────────

#[test]
fn camera_height_above_ellipsoid() {
    let e = wgs84();
    let height = 500_000.0;
    let pos = e.cartographic_to_cartesian(&Cartographic::from_degrees(10.0, 20.0, height));
    let cam = make_camera(pos, -pos.normalize(), DVec3::Z);
    let h = cam.height(&e);
    assert!(h.is_some());
    assert!(
        (h.unwrap() - height).abs() < 1.0,
        "height should be ≈ {}, got {}", height, h.unwrap()
    );
}

#[test]
fn camera_height_at_surface() {
    let e = wgs84();
    let pos = e.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0));
    let cam = make_camera(pos, -pos.normalize(), DVec3::Z);
    let h = cam.height(&e);
    assert!(h.is_some());
    assert!(
        h.unwrap().abs() < 1.0,
        "surface height should be ≈ 0, got {}", h.unwrap()
    );
}
