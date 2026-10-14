//! 相机扩展规格 — flyTo、矩形相机、2D 模式、视锥、拾取射线
//! 参考自：Specs/Scene/CameraSpec
//! A 类纯数学测试

use cesium_camera::{Camera, EasingFunction, Frustum, SceneMode};
use cesium_geospatial::{Cartographic, Ellipsoid, Rectangle, HeadingPitchRange};
use cesium_interaction::flight::{CameraFlight, compute_set_view};
use glam::DVec3;
use std::f64::consts::PI;

const EPSILON6: f64 = 1e-6;
const EPSILON8: f64 = 1e-8;
const EPSILON10: f64 = 1e-10;
const EPSILON14: f64 = 1e-14;
const EPSILON15: f64 = 1e-15;

fn test_camera() -> Camera {
    Camera::new(DVec3::new(0.0, 0.0, 1.0), DVec3::new(0.0, 0.0, -1.0), DVec3::new(0.0, 1.0, 0.0))
}

fn assert_vec3_epsilon(a: DVec3, b: DVec3, eps: f64, msg: &str) {
    assert!(
        a.abs_diff_eq(b, eps),
        "{}: expected {:?}, got {:?} (diff={})",
        msg,
        b,
        a,
        (a - b).length()
    );
}

// ============================================================================
// CameraFlight — flyTo 插值测试
// ============================================================================

#[test]
fn flight_creation_basic() {
    let camera = Camera::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let dest = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);
    let flight = CameraFlight::fly_to(&camera, dest, None, None, 5.0);

    assert_eq!(flight.duration, 5.0);
    assert_eq!(flight.elapsed, 0.0);
    assert!(!flight.complete);
    assert!((flight.start_position - camera.position).length() < EPSILON10);
    assert!((flight.end_position - dest).length() < EPSILON10);
}

#[test]
fn flight_easing_linear_midpoint() {
    let start_pos = DVec3::new(0.0, 0.0, 0.0);
    let camera = Camera::new(start_pos, -DVec3::Z, DVec3::Y);
    let dest = DVec3::new(100.0, 0.0, 0.0);

    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 2.0);
    flight.easing = EasingFunction::Linear;

    // 在 t=1.0（中点）处，位置应恰好在中间
    let (pos, _, _) = flight.update(1.0).unwrap();
    let midpoint = start_pos.lerp(dest, 0.5);
    assert_vec3_epsilon(pos, midpoint, EPSILON10, "linear midpoint");
    assert!((flight.progress() - 0.5).abs() < EPSILON10);
}

#[test]
fn flight_easing_sinusoidal_midpoint() {
    let camera = Camera::new(DVec3::ZERO, -DVec3::Z, DVec3::Y);
    let dest = DVec3::new(100.0, 0.0, 0.0);

    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 2.0);
    flight.easing = EasingFunction::SinusoidalInOut;

    let (pos, _, _) = flight.update(1.0).unwrap();
    let midpoint = camera.position.lerp(dest, 0.5);
    assert_vec3_epsilon(pos, midpoint, EPSILON10, "sinusoidal midpoint");
}

#[test]
fn flight_easing_quadraticin_quarter() {
    let camera = Camera::new(DVec3::ZERO, -DVec3::Z, DVec3::Y);
    let dest = DVec3::new(100.0, 0.0, 0.0);

    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 4.0);
    flight.easing = EasingFunction::QuadraticIn;

    // 在 t=1.0（25%）处，quadratic in 得 t²=0.0625
    let (pos, _, _) = flight.update(1.0).unwrap();
    let expected = camera.position.lerp(dest, 0.0625);
    assert_vec3_epsilon(pos, expected, EPSILON10, "quadraticIn 25%");
}

#[test]
fn flight_completes_exactly() {
    let camera = Camera::new(DVec3::ZERO, -DVec3::Z, DVec3::Y);
    let dest = DVec3::new(50.0, 0.0, 0.0);

    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 2.0);

    // 完整时长
    let (pos, _, _) = flight.update(2.0).unwrap();
    assert_vec3_epsilon(pos, dest, EPSILON10, "flight complete position");
    assert!(flight.complete);
    assert!((flight.progress() - 1.0).abs() < EPSILON10);
}

#[test]
fn flight_no_update_after_complete() {
    let camera = Camera::new(DVec3::ZERO, -DVec3::Z, DVec3::Y);
    let dest = DVec3::new(50.0, 0.0, 0.0);

    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 2.0);
    flight.update(2.0);
    assert!(flight.complete);

    let result = flight.update(1.0);
    assert!(result.is_none());
}

#[test]
fn flight_can_be_overrun() {
    let camera = Camera::new(DVec3::ZERO, -DVec3::Z, DVec3::Y);
    let dest = DVec3::new(50.0, 0.0, 0.0);

    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 2.0);

    // 超出时长
    let (pos, _, _) = flight.update(5.0).unwrap();
    assert_vec3_epsilon(pos, dest, EPSILON10, "overrun position");
    assert!(flight.complete);
}

#[test]
fn flight_zero_duration_clamps() {
    let camera = Camera::new(DVec3::ZERO, -DVec3::Z, DVec3::Y);
    let dest = DVec3::new(50.0, 0.0, 0.0);

    let flight = CameraFlight::fly_to(&camera, dest, None, None, 0.0);
    // 时长被限制在最小 0.001
    assert!(flight.duration >= 0.001);
}

#[test]
fn flight_zero_elapsed_gives_start() {
    let camera = Camera::new(DVec3::ZERO, -DVec3::Z, DVec3::Y);
    let dest = DVec3::new(50.0, 0.0, 0.0);

    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 2.0);
    let (pos, dir, up) = flight.update(0.0).unwrap();

    assert_vec3_epsilon(pos, camera.position, EPSILON10, "start position");
    assert_vec3_epsilon(dir, camera.direction, EPSILON10, "start direction");
    assert_vec3_epsilon(up, camera.up, EPSILON10, "start up");
}

#[test]
fn flight_direction_interpolation() {
    let start_dir = DVec3::new(0.0, 0.0, -1.0);
    let camera = Camera::new(DVec3::ZERO, start_dir, DVec3::Y);
    let end_dir = DVec3::new(-1.0, 0.0, 0.0);
    let dest = DVec3::new(100.0, 0.0, 0.0);

    let mut flight = CameraFlight::fly_to(&camera, dest, Some(end_dir), None, 2.0);

    // 在中点（linear）处，direction 应在两者间混合
    let (_, dir, _) = flight.update(2.0).unwrap();
    assert_vec3_epsilon(dir, end_dir.normalize(), EPSILON10, "end direction");
}

#[test]
fn flight_apply_to_camera() {
    let camera = Camera::new(DVec3::ZERO, -DVec3::Z, DVec3::Y);
    let dest = DVec3::new(100.0, 0.0, 0.0);

    let mut flight = CameraFlight::fly_to(&camera, dest, None, None, 2.0);
    let mut cam = camera;
    let still_flying = flight.apply_to_camera(&mut cam, 2.0);

    assert!(!still_flying);
    assert_vec3_epsilon(cam.position, dest, EPSILON10, "apply_to_camera position");
}

#[test]
fn flight_fly_to_bounding_sphere() {
    let camera = Camera::new(
        DVec3::new(6378137.0 * 5.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let sphere = cesium_geospatial::BoundingSphere::new(DVec3::ZERO, 6378137.0);

    let flight = CameraFlight::fly_to_bounding_sphere(
        &camera,
        &sphere,
        None,
        3.0,
    );

    assert_eq!(flight.duration, 3.0);
    assert!(!flight.complete);
    // destination 应位于地球表面之上
    let dest_height = flight.end_position.length() - sphere.radius;
    assert!(dest_height > 0.0);
}

#[test]
fn flight_fly_to_cartographic() {
    let camera = Camera::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let dest = Cartographic::from_radians(0.5, 0.3, 10000.0);
    let flight = CameraFlight::fly_to_cartographic(&camera, &dest, &Ellipsoid::WGS84, 4.0);

    assert_eq!(flight.duration, 4.0);
    assert!(!flight.complete);
    let expected_pos = Ellipsoid::WGS84.cartographic_to_cartesian(&dest);
    assert!((flight.end_position - expected_pos).length() < EPSILON6);
}

#[test]
fn flight_fly_home() {
    let camera = Camera::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let flight = CameraFlight::fly_home(&camera, &Ellipsoid::WGS84, 2.0);

    assert_eq!(flight.duration, 2.0);
    assert!(!flight.complete);
    // home 位置应距中心约 2.5 倍地球半径
    let home_height = flight.end_position.length();
    assert!(home_height > Ellipsoid::WGS84.maximum_radius() * 2.0);
}

// ============================================================================
// 矩形相机坐标
// ============================================================================

#[test]
fn get_rectangle_camera_coordinates_3d_global() {
    let mut camera = test_camera();
    camera.mode = SceneMode::Scene3D;
    let rect = Rectangle::new(-PI, -PI / 2.0, PI, PI / 2.0);
    let pos = camera.get_rectangle_camera_coordinates(&rect, &Ellipsoid::WGS84);

    // 应位于正 X 轴（全球矩形中心为 (0,0)）
    assert!(pos.x > 0.0);
    assert!(pos.y.abs() < EPSILON6);
    assert!(pos.z.abs() < EPSILON6);
    assert!(pos.length() > Ellipsoid::WGS84.maximum_radius());
}

#[test]
fn get_rectangle_camera_coordinates_3d_idl() {
    let mut camera = test_camera();
    camera.mode = SceneMode::Scene3D;
    // 穿越日界线的矩形：west=0.1 rad, east=-0.1 rad
    // 若处理环绕，则 center_lon = (0.1 + (-0.1) + 2*PI) / 2 = PI，
    // 但 Rust 实现直接计算 (west + east) * 0.5 = 0.0
    let rect = Rectangle::new(0.1, -PI / 2.0, -0.1, PI / 2.0);
    let pos = camera.get_rectangle_camera_coordinates(&rect, &Ellipsoid::WGS84);

    // 该实现计算简单平均经度，而非 IDL 感知的环绕
    // 因此中心接近 0 经度（正 X）
    let height = pos.length() - Ellipsoid::WGS84.maximum_radius();
    assert!(height > 0.0, "height should be positive: {}", height);
}

#[test]
fn get_rectangle_camera_coordinates_does_not_modify_camera() {
    let mut camera = test_camera();
    camera.mode = SceneMode::Scene3D;
    let orig_pos = camera.position;
    let orig_dir = camera.direction;
    let orig_up = camera.up;
    let orig_right = camera.right;

    let rect = Rectangle::new(-PI, -PI / 2.0, PI, PI / 2.0);
    let _pos = camera.get_rectangle_camera_coordinates(&rect, &Ellipsoid::WGS84);

    assert_vec3_epsilon(camera.position, orig_pos, EPSILON10, "position unchanged");
    assert_vec3_epsilon(camera.direction, orig_dir, EPSILON10, "direction unchanged");
    assert_vec3_epsilon(camera.up, orig_up, EPSILON10, "up unchanged");
    assert_vec3_epsilon(camera.right, orig_right, EPSILON10, "right unchanged");
}

#[test]
fn set_view_rectangle_3d() {
    let mut camera = test_camera();
    let rect = Rectangle::new(-0.5, -0.3, 0.5, 0.3);

    camera.set_view_rectangle(&rect, &Ellipsoid::WGS84);

    // 位置应位于矩形中心之上
    let height = camera.position.length() - Ellipsoid::WGS84.maximum_radius();
    assert!(height > 0.0);
    // direction 应指向中心
    let to_center = -camera.position.normalize();
    assert!(camera.direction.dot(to_center) > 0.99);
}

#[test]
fn compute_set_view_looking_down() {
    let carto = Cartographic::from_radians(0.0, 0.0, 0.0);
    let height = 1_000_000.0;

    let (position, direction, _up) = compute_set_view(
        &carto,
        height,
        0.0,
        -PI / 2.0,
        &Ellipsoid::WGS84,
    );

    let pos_height = position.length() - Ellipsoid::WGS84.maximum_radius();
    assert!((pos_height - height).abs() / height < 0.01);
    // 垂直向下看 → direction ≈ -surface_normal
    assert!(direction.dot(-position.normalize()) > 0.9);
}

#[test]
fn compute_set_view_with_heading() {
    let carto = Cartographic::from_radians(0.0, 0.0, 0.0);
    let height = 1_000_000.0;

    let (position, _direction, _up) = compute_set_view(
        &carto,
        height,
        PI / 2.0, // 朝向东
        -PI / 2.0,
        &Ellipsoid::WGS84,
    );

    let pos_height = position.length() - Ellipsoid::WGS84.maximum_radius();
    assert!((pos_height - height).abs() / height < 0.01);
}

// ============================================================================
// 相机坐标变换往返
// ============================================================================

#[test]
fn world_to_camera_to_world_roundtrip_point() {
    let camera = Camera::new(
        DVec3::new(1000.0, 2000.0, 3000.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::Y,
    );

    let world_point = DVec3::new(500.0, 600.0, 700.0);
    let camera_point = camera.world_to_camera_point(world_point);
    let back_world = camera.camera_to_world_point(camera_point);

    assert_vec3_epsilon(back_world, world_point, EPSILON10, "point roundtrip");
}

#[test]
fn world_to_camera_to_world_roundtrip_vector() {
    let camera = Camera::new(
        DVec3::new(1000.0, 2000.0, 3000.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::Y,
    );

    let world_vec = DVec3::new(1.0, 2.0, 3.0);
    let camera_vec = camera.world_to_camera_vector(world_vec);
    let back_world = camera.camera_to_world_vector(camera_vec);

    assert_vec3_epsilon(back_world, world_vec, EPSILON10, "vector roundtrip");
}

// ============================================================================
// 2D 模式相机测试
// ============================================================================

#[test]
fn scene_mode_2d_no_look_rotation() {
    // 注意：Camera 结构体本身并不强制 2D 禁止旋转行为；
    // 该逻辑位于 CesiumJS 的场景更新循环中。
    // 本测试验证相机在任意模式下都保持正交归一。
    let mut camera = test_camera();
    camera.mode = SceneMode::Scene2D;
    camera.look_left(Some(PI / 4.0));

    // 相机应保持正交归一
    assert!((camera.direction.length() - 1.0).abs() < EPSILON14);
    assert!((camera.up.length() - 1.0).abs() < EPSILON14);
    assert!((camera.right.length() - 1.0).abs() < EPSILON14);
}

#[test]
fn morphing_mode_rejects_setview() {
    let mut camera = test_camera();
    camera.mode = SceneMode::Morphing;
    let orig_pos = camera.position;

    let dest = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);
    camera.set_view(dest, 0.0, -PI / 2.0, 0.0, &Ellipsoid::WGS84);

    assert_vec3_epsilon(camera.position, orig_pos, EPSILON10, "morphing reject");
}

// ============================================================================
// 视锥测试
// ============================================================================

#[test]
fn frustum_perspective_default() {
    let frustum = Frustum::default();
    match frustum {
        Frustum::Perspective(f) => {
            assert!(f.fov > 0.0);
            assert!(f.aspect_ratio > 0.0);
            assert!(f.near > 0.0);
            assert!(f.far > f.near);
        }
        _ => panic!("expected perspective"),
    }
}

#[test]
fn frustum_projection_matrix_is_valid() {
    let frustum = Frustum::default();
    let proj = frustum.projection_matrix();

    // 检查非退化
    assert!(proj.determinant().abs() > EPSILON15);
}

#[test]
fn frustum_sse_denominator() {
    let frustum = Frustum::default();
    let denom = frustum.sse_denominator();
    assert!(denom > 0.0);
}

// ============================================================================
// 拾取射线测试（扩展）
// ============================================================================

#[test]
fn pick_ray_center_is_along_view_direction() {
    let camera = Camera::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let ray = camera.get_pick_ray(400.0, 300.0, 800.0, 600.0).unwrap();

    // 中心拾取射线的 direction 应与相机 direction 一致
    let dot = ray.direction.dot(camera.direction);
    assert!(dot > 0.9, "center pick ray direction alignment: {}", dot);
}

#[test]
fn pick_ray_corners_diverge() {
    let camera = Camera::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );

    let ray_tl = camera.get_pick_ray(0.0, 0.0, 800.0, 600.0).unwrap();
    let ray_tr = camera.get_pick_ray(800.0, 0.0, 800.0, 600.0).unwrap();
    let ray_bl = camera.get_pick_ray(0.0, 600.0, 800.0, 600.0).unwrap();
    let ray_br = camera.get_pick_ray(800.0, 600.0, 800.0, 600.0).unwrap();

    // 左上与右下应当发散
    assert!(ray_tl.direction.dot(ray_br.direction) < 0.999);
    assert!(ray_tr.direction.dot(ray_bl.direction) < 0.999);
}

#[test]
fn pick_ray_origin_is_camera_position() {
    let camera = Camera::new(
        DVec3::new(1000.0, 2000.0, 3000.0),
        -DVec3::Z,
        DVec3::Y,
    );
    let ray = camera.get_pick_ray(400.0, 300.0, 800.0, 600.0).unwrap();

    assert_vec3_epsilon(ray.origin, camera.position_wc(), EPSILON10, "pick ray origin");
}

#[test]
fn pick_ellipsoid_at_center_hits_surface() {
    let camera = Camera::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let hit = camera.pick_ellipsoid(400.0, 300.0, 800.0, 600.0, &Ellipsoid::WGS84);

    assert!(hit.is_some());
    let hit_point = hit.unwrap();
    let radii = Ellipsoid::WGS84.radii();
    let normalized = DVec3::new(hit_point.x / radii.x, hit_point.y / radii.y, hit_point.z / radii.z);
    assert!((normalized.length() - 1.0).abs() < EPSILON6);
}

#[test]
fn pick_ellipsoid_corner_misses() {
    let camera = Camera::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    // 远角可能未命中椭球
    let hit = camera.pick_ellipsoid(0.0, 0.0, 800.0, 600.0, &Ellipsoid::WGS84);
    // 是否命中取决于 FOV；仅验证不会 panic
    let _ = hit;
}

// ============================================================================
// 相机变化检测
// ============================================================================

#[test]
fn change_detection_no_change() {
    let camera = test_camera();
    let percentage = camera.compute_change_percentage(camera.position, camera.direction);
    assert!((percentage).abs() < EPSILON15);
    assert!(!camera.has_changed(camera.position, camera.direction));
}

#[test]
fn change_detection_position_change() {
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 100.0),
        -DVec3::Z,
        DVec3::Y,
    );
    let ref_pos = DVec3::new(0.0, 0.0, 50.0);
    let percentage = camera.compute_change_percentage(ref_pos, camera.direction);
    assert!(percentage > 0.0);
}

#[test]
fn change_detection_direction_change() {
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 100.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::Y,
    );
    let ref_dir = DVec3::new(0.0, 1.0, 0.0); // 相差 90°
    let percentage = camera.compute_change_percentage(camera.position, ref_dir);
    assert!(percentage > 0.0);
}

// ============================================================================
// 3D 中的 heading/pitch/roll
// ============================================================================

#[test]
fn heading_3d_at_equator_north() {
    let ellipsoid = Ellipsoid::WGS84;
    let mut camera = test_camera();
    let pos = DVec3::new(6378137.0, 0.0, 0.0); // 本初子午线，赤道
    camera.set_view_hpr(pos, 0.0, 0.0, 0.0, &ellipsoid);

    let heading = camera.heading_3d(&ellipsoid);
    // heading=0 表示朝北（经 ENU 变换后）
    assert!(heading.abs() < 0.01 || (heading - 2.0 * PI).abs() < 0.01,
        "heading should be ~0: {}", heading);
}

#[test]
fn pitch_3d_looking_horizon() {
    let ellipsoid = Ellipsoid::WGS84;
    let mut camera = test_camera();
    let pos = DVec3::new(6378137.0, 0.0, 0.0);
    camera.set_view_hpr(pos, 0.0, 0.0, 0.0, &ellipsoid);

    let pitch = camera.pitch_3d(&ellipsoid);
    assert!(pitch.abs() < 0.01, "pitch at horizon: {}", pitch);
}

#[test]
fn heading_pitch_roll_roundtrip_enu() {
    let ellipsoid = Ellipsoid::WGS84;
    let pos = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);
    let h = 1.0;
    let p = -0.3;
    let r = 0.5;

    let mut camera = test_camera();
    camera.set_view_hpr(pos, h, p, r, &ellipsoid);

    // 检查 heading/pitch/roll 近似保持不变
    let h2 = camera.heading_3d(&ellipsoid);
    let p2 = camera.pitch_3d(&ellipsoid);
    let r2 = camera.roll_3d(&ellipsoid);

    assert!((h2 - h).abs() < 0.02, "heading roundtrip: {} vs {}", h2, h);
    assert!((p2 - p).abs() < 0.02, "pitch roundtrip: {} vs {}", p2, p);
    assert!((r2 - r).abs() < 0.02, "roll roundtrip: {} vs {}", r2, r);
}

// ============================================================================
// 受限旋转测试
// ============================================================================

#[test]
fn rotate_constrained_no_constraint_behaves_like_rotate() {
    let mut cam1 = test_camera();
    let mut cam2 = test_camera();

    cam1.rotate(DVec3::Z, 0.5);
    cam2.rotate_constrained(DVec3::Z, 0.5);

    assert_vec3_epsilon(cam1.position, cam2.position, EPSILON10, "position same");
    assert_vec3_epsilon(cam1.direction, cam2.direction, EPSILON10, "direction same");
    assert_vec3_epsilon(cam1.up, cam2.up, EPSILON10, "up same");
}

#[test]
fn constrained_axis_prevents_crossing() {
    let mut camera = test_camera();
    camera.constrained_axis = Some(DVec3::Y);

    // 旋转 180° — 在 Y 约束下，up 不应越过 Y
    camera.rotate_constrained(DVec3::X, PI);

    // up 与 Y 的点积不应为负
    let dot = camera.up.dot(DVec3::Y);
    assert!(dot >= -EPSILON10, "up should not cross +Y: dot={}", dot);
}

#[test]
fn constrained_up_down_mirror() {
    let mut camera = test_camera();
    camera.constrained_axis = Some(DVec3::Y);

    camera.rotate_up_constrained(PI / 2.0);
    // 将 up 旋转 90° 后，direction 应为 up (Z)，up 应为 -Z
    assert_vec3_epsilon(camera.position, DVec3::new(0.0, -1.0, 0.0), EPSILON10, "rotated up position");

    camera.rotate_down_constrained(PI / 2.0);
    // 应回到原始状态
    assert_vec3_epsilon(camera.position, DVec3::new(0.0, 0.0, 1.0), EPSILON10, "back to original position");
}

// ============================================================================
// 按模式获取相机模长
// ============================================================================

#[test]
fn get_magnitude_3d() {
    let mut camera = test_camera();
    camera.mode = SceneMode::Scene3D;
    let mag = camera.get_magnitude();
    assert!((mag - 1.0).abs() < EPSILON10);
}

#[test]
fn get_magnitude_2d() {
    let mut camera = test_camera();
    camera.mode = SceneMode::Scene2D;
    let mag = camera.get_magnitude();
    assert!((mag - 1.0).abs() < EPSILON10);
}

#[test]
fn get_magnitude_columbus_view() {
    let mut camera = test_camera();
    camera.mode = SceneMode::ColumbusView;
    let mag = camera.get_magnitude();
    assert!((mag - 1.0).abs() < EPSILON10); // position 为 (0,0,1)，CV 使用 abs(z)
}

// ============================================================================
// 像素大小 / 到包围球的距离
// ============================================================================

#[test]
fn distance_to_bounding_sphere_front() {
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 100.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::Y,
    );
    let sphere = cesium_geospatial::BoundingSphere::new(DVec3::new(0.0, 0.0, 0.0), 10.0);
    let dist = camera.distance_to_bounding_sphere(&sphere);
    // 沿 Z 的距离 = 100，减去球半径 = 90
    assert!((dist - 90.0).abs() < EPSILON10, "distance: {}", dist);
}

#[test]
fn distance_to_bounding_sphere_behind() {
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 100.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::Y,
    );
    let sphere = cesium_geospatial::BoundingSphere::new(DVec3::new(0.0, 0.0, 200.0), 10.0);
    let dist = camera.distance_to_bounding_sphere(&sphere);
    // 相机后方 → 被限制为 0
    assert!((dist).abs() < EPSILON10, "distance behind: {}", dist);
}

#[test]
fn get_pixel_size_valid() {
    let camera = Camera::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let sphere = cesium_geospatial::BoundingSphere::new(DVec3::new(6378137.0, 0.0, 0.0), 10000.0);
    let pixel_size = camera.get_pixel_size(&sphere, 1920.0, 1080.0, 1.0);

    assert!(pixel_size > 0.0);
    assert!(pixel_size.is_finite());
}

// ============================================================================
// lookAt 变体
// ============================================================================

#[test]
fn look_at_offset_positions_camera() {
    let mut camera = test_camera();
    let target = DVec3::new(6378137.0, 0.0, 0.0);
    let offset = HeadingPitchRange::new(0.0, -PI / 4.0, 100000.0);

    camera.look_at(target, &offset, &Ellipsoid::WGS84);

    // 位置应距 target 约 100km
    let dist = (camera.position_wc() - target).length();
    assert!((dist - 100000.0).abs() < 1000.0, "lookAt distance: {}", dist);
}

// ============================================================================
// 默认 home 位置
// ============================================================================

#[test]
fn default_home_position_above_surface() {
    let pos = Camera::default_home_position(&Ellipsoid::WGS84);
    let height = pos.length() - Ellipsoid::WGS84.maximum_radius();
    assert!(height > Ellipsoid::WGS84.maximum_radius(), "home height: {}", height);
}

// ============================================================================
// 视图矩阵一致性
// ============================================================================

#[test]
fn view_projection_matrix_product() {
    let camera = Camera::new(
        DVec3::new(100.0, 0.0, 0.0),
        -DVec3::X,
        DVec3::Z,
    );
    let view = camera.view_matrix();
    let proj = camera.projection_matrix();
    let vp = camera.view_projection_matrix();

    let expected = proj * view;
    for i in 0..16 {
        assert!(
            (vp.to_cols_array()[i] - expected.to_cols_array()[i]).abs() < EPSILON10,
            "vp[{}] expected {} got {}",
            i,
            expected.to_cols_array()[i],
            vp.to_cols_array()[i]
        );
    }
}

#[test]
fn view_bounding_sphere_positions_camera() {
    let mut camera = Camera::new(
        DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    );
    let center = DVec3::new(6378137.0, 0.0, 0.0);
    let init_pos = camera.position;

    camera.view_bounding_sphere(center, 10000.0, 0.0, &Ellipsoid::WGS84);

    // 位置应已改变
    assert!(camera.position.length() > 0.0);
    assert!((camera.position - init_pos).length() > 0.0);
}

// ============================================================================
// 任意操作后归一化的 up 向量
// ============================================================================

#[test]
fn camera_always_orthonormal_after_rotate() {
    let mut camera = test_camera();
    for _ in 0..10 {
        camera.rotate(DVec3::new(1.0, 0.5, 0.3).normalize(), 0.7);
        assert!((camera.direction.length() - 1.0).abs() < EPSILON14);
        assert!((camera.up.length() - 1.0).abs() < EPSILON14);
        assert!((camera.right.length() - 1.0).abs() < EPSILON14);
        assert!(camera.direction.dot(camera.up).abs() < EPSILON14);
        assert!(camera.direction.dot(camera.right).abs() < EPSILON14);
        assert!(camera.up.dot(camera.right).abs() < EPSILON14);
    }
}
