//! M2.5 集成测试 —— 用于驱动 `CameraControl` 的端口。
//!
//! 测试端口实现（`cesium-bevy-render` 的
//! [`CameraControlImpl`] / [`CameraControlPort`]），它将对域相机
//! 算法暴露为一个可编程的控制接口。涵盖两个层次：
//!
//! 1. **端口对象**（不依赖 ECS）：直接驱动 `set_view` / `fly_to` / `look_at` / `zoom_*` /
//!    `get_camera_state`，并以 ECEF 米为单位进行断言，其中
//!    `fly_to` 的落点误差按 `< 1e-4` 个渲染单位的 M2 门槛
//!    进行检查（`METERS_PER_RENDER_UNIT = 6378137`）。
//! 2. **Bevy 桥接**：一个最小应用注册 [`CameraControlPort`] +
//!    [`camera_control_port_system`]，仅含一个 [`CesiumCamera`] 实体，
//!    并证明端口命令能到达活动实体（既是瞬时的
//!    `set_view`，也是多帧的大圆弧 `fly_to`）。

use std::f64::consts::FRAC_PI_2;
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;

use cesium_bevy_render::camera::camera_control_port_system;
use cesium_bevy_render::{
    CameraControlImpl, CameraControlPort, CesiumCamera, METERS_PER_RENDER_UNIT,
};
use cesium_camera::Camera;
use cesium_geospatial::{Cartographic, Ellipsoid};
use cesium_ports_driving::CameraControl;
use cesium_scene_mode::SceneMode;
use glam::DVec3;

/// WGS84 最大半径（米）== `METERS_PER_RENDER_UNIT`。
const R: f64 = 6378137.0;

/// 位于赤道上空一个半径、+X 轴上、朝向中心的相机。
fn equator_camera() -> Camera {
    Camera::new(
        DVec3::new(R * 2.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    )
}

fn new_control() -> CameraControlImpl {
    CameraControlImpl::new(equator_camera(), Ellipsoid::WGS84)
}

/// 以固定 `dt` 步长驱动 `control` 的活动飞行直至完成，
/// 并设有上限，使无法完成的飞行会显式失败而非挂起。
fn run_flight_to_completion(control: &mut CameraControlImpl, dt: f64) {
    for _ in 0..10_000 {
        if !control.update(dt) {
            return;
        }
    }
    panic!("flight did not complete within 10_000 steps");
}

// ── 端口对象：fly_to ────────────────────────────────────────────────────

/// `fly_to` 沿大圆弧飞行并精确落在目标 ECEF 上；
/// 残差按 M2 门槛（< 1e-4 个渲染单位）断言并打印。
#[test]
fn fly_to_lands_on_destination_within_gate() {
    let mut control = new_control();
    let dest = Cartographic::from_degrees(-75.0, 40.0, 1_000_000.0);

    // duration_secs = 0.0 → 端口从距离推导时长/缓动
    //（compute_flight_duration / select_flight_easing）并对大圆弧做球面插值。
    control.fly_to(dest, None, None, None, 0.0);
    assert!(control.is_flying(), "fly_to must start a flight");

    run_flight_to_completion(&mut control, 0.05);
    assert!(!control.is_flying(), "flight must be finished");

    let expected = Ellipsoid::WGS84.cartographic_to_cartesian(&dest);
    let state = control.get_camera_state();
    let err_m = (state.position - expected).length();
    let err_ru = err_m / METERS_PER_RENDER_UNIT;
    println!(
        "[fly_to] measured pos = ({:.6}, {:.6}, {:.6}) m; expected = ({:.6}, {:.6}, {:.6}) m; \
         err = {:.6e} m = {:.6e} render units",
        state.position.x, state.position.y, state.position.z,
        expected.x, expected.y, expected.z,
        err_m, err_ru,
    );
    assert!(
        err_ru < 1e-4,
        "fly_to landing error {err_ru} render units exceeds the 1e-4 gate ({err_m} m)"
    );
}

/// 显式的 `duration_secs` 会被采纳，且飞行仍落在目标上。
#[test]
fn fly_to_honors_explicit_duration() {
    let mut control = new_control();
    let dest = Cartographic::from_degrees(139.0, 35.0, 2_000_000.0);
    control.fly_to(dest, Some(0.0), Some(-FRAC_PI_2), None, 2.0);

    // 在 2 秒飞行的中点，相机必须严格位于
    // 两端点之间（证明时长真实存在，而非瞬时跳转）。
    control.update(1.0);
    let mid = control.get_camera_state().position;
    let start = DVec3::new(R * 2.0, 0.0, 0.0);
    let end = Ellipsoid::WGS84.cartographic_to_cartesian(&dest);
    assert!(control.is_flying(), "still flying at t=1s of a 2s flight");
    assert!((mid - start).length() > 1.0 && (mid - end).length() > 1.0, "mid must be in-flight");

    run_flight_to_completion(&mut control, 0.05);
    let err_ru = (control.get_camera_state().position - end).length() / METERS_PER_RENDER_UNIT;
    assert!(err_ru < 1e-4, "explicit-duration fly_to error {err_ru} ru");
}

// ── 端口对象：set_view / get_camera_state ───────────────────────────────

/// `set_view` 将相机放置于测绘坐标位置，并通过
/// `get_camera_state` 回报该姿态（位置 + 向下的俯仰角）。
#[test]
fn set_view_positions_camera_and_state_reports_it() {
    let mut control = new_control();
    let carto = Cartographic::from_degrees(10.0, 20.0, 500_000.0);

    control.set_view(carto, 0.0, -FRAC_PI_2, 0.0);

    let expected = Ellipsoid::WGS84.cartographic_to_cartesian(&carto);
    let state = control.get_camera_state();
    let err_ru = (state.position - expected).length() / METERS_PER_RENDER_UNIT;
    assert!(err_ru < 1e-4, "set_view position error {err_ru} ru");
    // 垂直向下看 ⇒ 俯仰角 ≈ -π/2，且方向与法线相反。
    assert!(
        (state.pitch + FRAC_PI_2).abs() < 1e-6,
        "set_view pitch {} should be -π/2",
        state.pitch
    );
    let n = expected.normalize();
    assert!(
        state.direction.dot(-n) > 1.0 - 1e-9,
        "set_view must look straight down"
    );
    // 水平相机的翻滚角约为零。
    assert!(state.roll.abs() < 1e-6, "set_view roll {}", state.roll);
}

// ── 端口对象：look_at ───────────────────────────────────────────────────

/// `look_at` 保持请求的距离并将相机对准目标。
#[test]
fn look_at_holds_range_and_aims_at_target() {
    let mut control = new_control();
    let target = Cartographic::from_degrees(0.0, 0.0, 0.0);
    let range = 2_000_000.0;

    control.look_at(target, 0.0, -FRAC_PI_2, range);

    let target_ecef = Ellipsoid::WGS84.cartographic_to_cartesian(&target);
    let state = control.get_camera_state();
    let dist = (state.position - target_ecef).length();
    assert!(
        (dist - range).abs() / range < 1e-9,
        "look_at range {dist} != {range}"
    );
    let aim = (target_ecef - state.position).normalize();
    assert!(
        aim.dot(state.direction) > 1.0 - 1e-9,
        "look_at must aim at the target"
    );
}

// ── 端口对象：zoom ──────────────────────────────────────────────────────

/// `zoom_in` 沿视线方向按公制量将相机向前移动，
/// `zoom_out` 则向后退，两者均以表面距离衡量。
#[test]
fn zoom_in_and_out_move_along_view() {
    let mut control = new_control();
    let len0 = control.get_camera_state().position.length();

    control.zoom_in(Some(100_000.0));
    let len1 = control.get_camera_state().position.length();
    assert!(
        (len0 - len1 - 100_000.0).abs() < 1.0,
        "zoom_in: {len0} → {len1} (expected -100000)"
    );

    control.zoom_out(Some(250_000.0));
    let len2 = control.get_camera_state().position.length();
    assert!(
        (len2 - len1 - 250_000.0).abs() < 1.0,
        "zoom_out: {len1} → {len2} (expected +250000)"
    );
}

// ── Bevy 桥接 ────────────────────────────────────────────────────────────

/// 构建一个最小应用：`MinimalPlugins`（用于 `Time`）、[`CameraControlPort`]
/// 资源、`PostUpdate` 中的 [`camera_control_port_system`]，以及一个
/// [`CesiumCamera`] 实体。无需输入插件，因为仅测试端口
/// 桥接。
fn bridge_app() -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<CameraControlPort>()
        .add_systems(PostUpdate, camera_control_port_system);
    let e = app
        .world_mut()
        .spawn(CesiumCamera::new(equator_camera(), SceneMode::Scene3D))
        .id();
    (app, e)
}

fn entity_position(app: &App, e: Entity) -> DVec3 {
    app.world().get::<CesiumCamera>(e).unwrap().camera.position
}

/// 通过端口资源的瞬时 `set_view` 会在一次 `app.update()`
/// 后到达活动的 `CesiumCamera` 实体。
#[test]
fn bridge_set_view_updates_cesium_camera_entity() {
    let (mut app, e) = bridge_app();
    let carto = Cartographic::from_degrees(-30.0, 15.0, 800_000.0);

    app.world_mut()
        .resource_mut::<CameraControlPort>()
        .set_view(carto, 0.0, -FRAC_PI_2, 0.0);
    app.update();

    let expected = Ellipsoid::WGS84.cartographic_to_cartesian(&carto);
    let err_ru = (entity_position(&app, e) - expected).length() / METERS_PER_RENDER_UNIT;
    assert!(
        err_ru < 1e-4,
        "bridge set_view error {err_ru} ru: {:?}",
        entity_position(&app, e)
    );
}

/// 通过端口资源驱动的大圆弧 `fly_to` 会跨帧推进
///（通过 `TimeUpdateStrategy` 使用固定 dt），并将 `CesiumCamera` 实体落在
/// 目标上，误差在 1e-4 渲染单位门槛之内。
#[test]
fn bridge_fly_to_flies_cesium_camera_entity_to_destination() {
    let (mut app, e) = bridge_app();
    // 固定 50 毫秒的帧，使飞行确定地完成。
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        Duration::from_secs_f64(0.05),
    ));

    let dest = Cartographic::from_degrees(2.0, -33.0, 1_500_000.0);
    app.world_mut()
        .resource_mut::<CameraControlPort>()
        .fly_to(dest, None, None, None, 1.0);

    // 1 秒飞行、50 毫秒/帧 → 20 帧；运行 40 帧以留余量。
    for _ in 0..40 {
        app.update();
    }

    let expected = Ellipsoid::WGS84.cartographic_to_cartesian(&dest);
    let got = entity_position(&app, e);
    let err_m = (got - expected).length();
    let err_ru = err_m / METERS_PER_RENDER_UNIT;
    println!(
        "[bridge fly_to] entity pos = ({:.6}, {:.6}, {:.6}); err = {:.6e} m = {:.6e} ru",
        got.x, got.y, got.z, err_m, err_ru
    );
    assert!(
        err_ru < 1e-4,
        "bridge fly_to error {err_ru} render units exceeds the 1e-4 gate"
    );
    assert!(
        !app.world()
            .resource::<CameraControlPort>()
            .control()
            .is_flying(),
        "flight must have completed"
    );
}

/// 无命令且无飞行时桥接不活动：实体自身的姿态
/// 会被保留（端口跟踪它而非覆盖它）。
#[test]
fn bridge_is_inert_without_commands() {
    let (mut app, e) = bridge_app();
    let before = entity_position(&app, e);
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(before, entity_position(&app, e), "idle bridge must not move the camera");
}
