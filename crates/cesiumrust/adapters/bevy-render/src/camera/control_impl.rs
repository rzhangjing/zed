//! `CameraControl` 驱动端口适配器（M2.5）。
//!
//! 通过 [`CameraControl`] 驱动端口（`cesium-ports-driving`）暴露领域相机
//! 算法——[`CameraController`]、[`CameraFlight`] 以及 `compute_*` 飞行
//! helper——以便外部代码（脚本、widget、测试）可以
//! 以编程方式飞行/设置/朝向/缩放相机，而无需触碰
//! 鼠标 / 键盘 / 触控输入路径。
//!
//! # 分层
//! * 所有相机*数学*都留在领域：大弧 slerp 飞行
//!   （[`CameraFlight::fly_to`] → [`CameraFlight::update`]）、`set_view` /
//!   `look_at` 位姿构造（[`compute_set_view`] / [`compute_look_at`]）与
//!   缩放（[`CameraController::zoom`]）。本适配器只
//!   1. 将端口的 [`Cartographic`] API 转为领域的 ECEF `DVec3`
//!      词汇，以及
//!   2. 为缩放做 meters→normalized-delta 的边界转换，精确
//!      镜像 [`super::controller_system`] / [`super::touch_system`] 中的 pixel→world
//!      转换（量级在边界，语义在领域）。
//! * 一切都是 `f64`（领域）；单一的 `f32` GPU 边界仍是
//!   [`super::update_system`]，本模块不触碰它。
//!
//! # Bevy 桥接
//! [`CameraControlImpl`] 是一个自包含、无 ECS 的对象（可直接
//! 在单元/集成测试中实例化）。[`CameraControlPort`] 将它包装为一个
//! Bevy [`Resource`]，而 [`camera_control_port_system`] 每个 `PostUpdate`（在
//! Transform 写入器之前）将它同步到活的 [`CesiumCamera`] 实体：
//! * 当一个命令已发出或一个飞行处于活动状态时，端口是权威的，
//!   它的相机被推送到实体；
//! * 否则实体是权威的（鼠标/触控/键盘驱动了它），并被
//!   拷回端口，以便下一个命令从当前位姿开始。
//!
//! 无 [`CesiumCamera`] 实体时本系统是惰性的，所以注册它仍不会
//! 扰动那些以其他方式驱动相机的应用。

use bevy::prelude::*;
use cesium_camera::Camera;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::Cartographic;
use cesium_interaction::{
    compute_flight_duration, compute_look_at, compute_set_view, select_flight_easing,
    CameraController, CameraControllerConfig, CameraFlight,
};
use cesium_ports_driving::CameraControl;
use glam::{DQuat, DVec3};
use std::f64::consts::{FRAC_PI_2, TAU};

use crate::camera::components::CesiumCamera;

/// 由 [`CameraControlImpl::get_camera_state`] 返回的相机位姿
/// 只读快照。
///
/// `heading`/`pitch`/`roll` 遵循 CesiumJS `Camera` 约定（弧度）：
/// heading `0` = 北，向东递增；pitch `0` = 地平线，`-π/2` =
/// 正下方；roll `0` = 水平。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraState {
    /// 相机位置（ECEF，米）。
    pub position: DVec3,
    /// 视线方向（单位）。
    pub direction: DVec3,
    /// 上方向（单位）。
    pub up: DVec3,
    /// 右方向（单位）。
    pub right: DVec3,
    /// 以弧度表示的航向（`[0, 2π)`）。
    pub heading: f64,
    /// 以弧度表示的俯仰（`[-π/2, π/2]`）。
    pub pitch: f64,
    /// 以弧度表示的翻滚。
    pub roll: f64,
}

/// `CameraControl` 驱动端口的实现。
///
/// 拥有一个领域 [`Camera`]、参考 [`Ellipsoid`]，以及至多一个活动
/// [`CameraFlight`]。每个端口方法都将几何委派给领域，
/// 并递增一个内部 generation 计数器，以便 Bevy 桥接能区分“一个命令
/// 已发出”与“空闲”。
pub struct CameraControlImpl {
    /// 受控的领域相机（位置/朝向权威）。
    camera: Camera,
    /// 几何计算所依据的参考椭球。
    ellipsoid: Ellipsoid,
    /// 至多一个活动飞行；空闲时为 `None`。
    flight: Option<CameraFlight>,
    /// 命令计数器；每次端口调用递增，供桥接区分“已发命”与“空闲”。
    generation: u64,
}

impl CameraControlImpl {
    /// 创建一个绑定到 `ellipsoid` 上 `camera` 的控制器。
    pub fn new(camera: Camera, ellipsoid: Ellipsoid) -> Self {
        Self {
            camera,
            ellipsoid,
            flight: None,
            generation: 0,
        }
    }

    /// 借用底层领域相机。
    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    /// 可变地借用底层领域相机（由 Bevy 桥接用于
    /// 从活实体重新播种端口）。
    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    /// 参考椭球。
    pub fn ellipsoid(&self) -> &Ellipsoid {
        &self.ellipsoid
    }

    /// 命令 generation；每次端口方法调用都递增。
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// 是否当前有飞行处于活动状态。
    pub fn is_flying(&self) -> bool {
        self.flight.is_some()
    }

    /// 将活动飞行推进 `dt` 秒并应用到相机。
    /// 当这一步后飞行仍在进行时返回 `true`。
    pub fn update(&mut self, dt: f64) -> bool {
        let Some(flight) = self.flight.as_mut() else {
            return false;
        };
        let still_flying = flight.apply_to_camera(&mut self.camera, dt);
        if !still_flying {
            self.flight = None;
        }
        still_flying
    }

    /// 快照当前位姿（位置/朝向 + heading/pitch/roll）。
    pub fn get_camera_state(&self) -> CameraState {
        let (heading, pitch, roll) = compute_heading_pitch_roll(
            self.camera.position,
            self.camera.direction,
            self.camera.up,
        );
        CameraState {
            position: self.camera.position,
            direction: self.camera.direction,
            up: self.camera.up,
            right: self.camera.right,
            heading,
            pitch,
            roll,
        }
    }

    /// 相机当前的制图位置（经/纬/高），若它不在
    /// 椭球中心处。
    pub fn camera_cartographic(&self) -> Option<Cartographic> {
        self.ellipsoid.cartesian_to_cartographic(self.camera.position)
    }

    /// 构建一个默认配置的领域控制器（量级来自端口
    /// 调用，语义/限制来自领域）。
    #[inline]
    fn controller(&self) -> CameraController {
        CameraController {
            config: CameraControllerConfig::default(),
            ellipsoid: self.ellipsoid,
        }
    }

    /// 分派一个绝对位姿，并像 [`Camera::new`] 一样重新将 right/up 标准正交化。
    fn apply_pose(&mut self, position: DVec3, direction: DVec3, up: DVec3) {
        self.camera.position = position;
        let direction = direction.normalize();
        self.camera.direction = direction;
        let right = direction.cross(up).normalize();
        self.camera.right = right;
        self.camera.up = right.cross(direction).normalize();
    }

    /// 当端口传入 `None` 时的默认缩放步长：当前地表上方
    /// 高度的 10%（匹配 [`CameraController::zoom`] 自己的缩放）。
    #[inline]
    fn default_zoom_step(&self) -> f64 {
        let height = (self.camera.position.length() - self.ellipsoid.maximum_radius()).abs();
        (height * 0.1).max(1.0)
    }

    /// 沿其视线方向移动相机 `meters`（正 = 向前
    /// / 向里缩放）。将米度量转为领域 [`CameraController::zoom`] 消费的
    /// 归一化 delta，把碰撞 + 速度限制留在领域。
    fn zoom_by_meters(&mut self, meters: f64) {
        let ctrl = self.controller();
        let height = (self.camera.position.length() - self.ellipsoid.maximum_radius())
            .abs()
            .max(1000.0);
        // 领域 zoom 按 `height * 0.1 * delta * zoom_speed` 移动（zoom_speed = 1）。
        let delta = meters / (height * 0.1);
        ctrl.zoom(&mut self.camera, delta);
        ctrl.enforce_collision(&mut self.camera);
        self.flight = None;
        self.generation += 1;
    }
}

impl Default for CameraControlImpl {
    /// 默认：以 WGS84 椭球与默认 home 位姿（俯视地表）构造控制器。
    fn default() -> Self {
        let ellipsoid = Ellipsoid::WGS84;
        let position = Camera::default_home_position(&ellipsoid);
        let direction = -position.normalize();
        Self::new(Camera::new(position, direction, DVec3::Z), ellipsoid)
    }
}

impl CameraControl for CameraControlImpl {
    /// 将相机瞬移到指定制图位置与 heading/pitch/roll，并取消任何活动飞行。
    fn set_view(&mut self, position: Cartographic, heading: f64, pitch: f64, roll: f64) {
        // `compute_set_view` 将相机置于 (lon, lat) 上方 `height` 处，并按
        // heading/pitch 定向；roll 绕视线方向施加。
        let (pos, dir, up) =
            compute_set_view(&position, position.height, heading, pitch, &self.ellipsoid);
        let (dir, up) = apply_roll(dir, up, roll);
        self.apply_pose(pos, dir, up);
        self.flight = None;
        self.generation += 1;
    }

    /// 启动一段到目的地的飞行动画（由后续 `update` 逐帧推进）。
    fn fly_to(
        &mut self,
        destination: Cartographic,
        heading: Option<f64>,
        pitch: Option<f64>,
        roll: Option<f64>,
        duration_secs: f64,
    ) {
        // 结束位姿恰好是目的地上的一次 `set_view`：heading/pitch
        // 默认为“直视下方”（CesiumJS `flyTo` 默认），而
        // `compute_set_view(0, -π/2)` 会将其复现为 `-destination.normalize()`。
        let h = heading.unwrap_or(0.0);
        let p = pitch.unwrap_or(-FRAC_PI_2);
        let r = roll.unwrap_or(0.0);
        let (dest_ecef, dir, up) =
            compute_set_view(&destination, destination.height, h, p, &self.ellipsoid);
        let (dir, up) = apply_roll(dir, up, r);

        // 大弧飞行：`CameraFlight::update` 绕中心做 slerp 并带一个
        // 抛物线拱；当调用者把 `duration_secs` 留为非正时，时长/缓动
        // 由所行距离推导。
        let distance = (dest_ecef - self.camera.position).length();
        let duration = if duration_secs > 0.0 {
            duration_secs
        } else {
            compute_flight_duration(distance)
        };
        let mut flight =
            CameraFlight::fly_to(&self.camera, dest_ecef, Some(dir), Some(up), duration);
        flight.easing = select_flight_easing(distance);
        self.flight = Some(flight);
        self.generation += 1;
    }

    /// 将相机定位到以给定 heading/pitch/range 环视目标点。
    fn look_at(&mut self, target: Cartographic, heading: f64, pitch: f64, range: f64) {
        let target_ecef = self.ellipsoid.cartographic_to_cartesian(&target);
        let up = target_ecef.normalize();
        let (east, north, _) = local_enu(up);

        // 相机坐落于距目标 `range`、仰角 `-pitch` 且方位角与
        // 视线 heading 相反处（heading = 相机所看的方向）。
        let horiz = range * pitch.cos();
        let vert = -range * pitch.sin();
        let dir_h = -(north * heading.cos() + east * heading.sin());
        let offset = dir_h * horiz + up * vert;

        let (pos, dir, cam_up) = compute_look_at(target_ecef, offset);
        self.apply_pose(pos, dir, cam_up);
        self.flight = None;
        self.generation += 1;
    }

    /// 沿视线向前缩放（`amount` 缺省时取默认步长）。
    fn zoom_in(&mut self, amount: Option<f64>) {
        let meters = amount.unwrap_or_else(|| self.default_zoom_step());
        self.zoom_by_meters(meters.abs());
    }

    /// 沿视线向后缩放（`amount` 缺省时取默认步长）。
    fn zoom_out(&mut self, amount: Option<f64>) {
        let meters = amount.unwrap_or_else(|| self.default_zoom_step());
        self.zoom_by_meters(-meters.abs());
    }

    /// 飞回默认 home 位姿。
    fn home(&mut self) {
        let dest = Camera::default_home_position(&self.ellipsoid);
        let distance = (dest - self.camera.position).length();
        let duration = compute_flight_duration(distance);
        let dir = -dest.normalize();
        let mut flight = CameraFlight::fly_to(&self.camera, dest, Some(dir), Some(DVec3::Z), duration);
        flight.easing = select_flight_easing(distance);
        self.flight = Some(flight);
        self.generation += 1;
    }
}

/// 将 `up` 绕 `direction` 旋转 `roll`（`roll == 0` 时为 no-op）。
#[inline]
fn apply_roll(direction: DVec3, up: DVec3, roll: f64) -> (DVec3, DVec3) {
    if roll == 0.0 {
        return (direction, up);
    }
    let q = DQuat::from_axis_angle(direction.normalize(), roll);
    (direction, (q * up).normalize())
}

/// 大地测量上方向向量 `up`（单位）的局部 East-North-Up 基底。在极点
/// 处 `Z × up` 退化时回退到一个由 `Y` 导出的 east。
fn local_enu(up: DVec3) -> (DVec3, DVec3, DVec3) {
    let mut east = DVec3::Z.cross(up);
    if east.length_squared() < 1e-12 {
        east = DVec3::Y.cross(up);
    }
    let east = east.normalize();
    let north = up.cross(east).normalize();
    (east, north, up)
}

/// 从位姿导出 CesiumJS 约定的 heading/pitch/roll。
fn compute_heading_pitch_roll(position: DVec3, direction: DVec3, up: DVec3) -> (f64, f64, f64) {
    let n = position.normalize();
    if !n.is_finite() || n.length_squared() < 0.5 {
        // 退化（相机在中心处/附近）：无有意义的局部坐标系。
        return (0.0, 0.0, 0.0);
    }
    let (east, north, _) = local_enu(n);

    // Pitch：视线方向低于局部地平线的仰角。
    let pitch = direction.dot(n).clamp(-1.0, 1.0).asin();

    // Heading：方向的水平投影的方位角，从北起算。
    let horiz = direction - n * direction.dot(n);
    let heading = if horiz.length_squared() < 1e-18 {
        0.0
    } else {
        let h = horiz.normalize();
        h.dot(east).atan2(h.dot(north)).rem_euclid(TAU)
    };

    // Roll：从“水平”up 到相机 up 的有符号角，绕 direction。
    let up_level = n - direction * n.dot(direction);
    let roll = if up_level.length_squared() < 1e-18 {
        0.0
    } else {
        let up_level = up_level.normalize();
        let right_level = direction.cross(up_level).normalize();
        up.dot(right_level).atan2(up.dot(up_level))
    };

    (heading, pitch, roll)
}

/// 包装一个 [`CameraControlImpl`] 的 Bevy [`Resource`]，以便驱动端口可以从
/// world 取出并用于以编程方式控制相机。
///
/// 通过转发到内部控制器来实现 [`CameraControl`]，所以调用者
/// 可以直接 `port.fly_to(...)`（需将 trait 纳入作用域）。
#[derive(Resource)]
pub struct CameraControlPort {
    /// 被包装的实际控制器实现。
    control: CameraControlImpl,
    /// 桥接上次观察到的 generation，用于判断是否有新命令。
    last_gen: u64,
}

impl Default for CameraControlPort {
    /// 默认：包一个 [`CameraControlImpl::default`] 并同步 last_gen。
    fn default() -> Self {
        let control = CameraControlImpl::default();
        Self {
            last_gen: control.generation(),
            control,
        }
    }
}

impl CameraControlPort {
    /// 包装一个已有的控制实现。
    pub fn new(control: CameraControlImpl) -> Self {
        Self {
            last_gen: control.generation(),
            control,
        }
    }

    /// 借用内部控制器。
    pub fn control(&self) -> &CameraControlImpl {
        &self.control
    }

    /// 可变地借用内部控制器。
    pub fn control_mut(&mut self) -> &mut CameraControlImpl {
        &mut self.control
    }
}

impl CameraControl for CameraControlPort {
    /// 转发到内部控制器的 `set_view`。
    fn set_view(&mut self, position: Cartographic, heading: f64, pitch: f64, roll: f64) {
        self.control.set_view(position, heading, pitch, roll);
    }

    /// 转发到内部控制器的 `fly_to`。
    fn fly_to(
        &mut self,
        destination: Cartographic,
        heading: Option<f64>,
        pitch: Option<f64>,
        roll: Option<f64>,
        duration_secs: f64,
    ) {
        self.control
            .fly_to(destination, heading, pitch, roll, duration_secs);
    }

    /// 转发到内部控制器的 `look_at`。
    fn look_at(&mut self, target: Cartographic, heading: f64, pitch: f64, range: f64) {
        self.control.look_at(target, heading, pitch, range);
    }

    /// 转发到内部控制器的 `zoom_in`。
    fn zoom_in(&mut self, amount: Option<f64>) {
        self.control.zoom_in(amount);
    }

    /// 转发到内部控制器的 `zoom_out`。
    fn zoom_out(&mut self, amount: Option<f64>) {
        self.control.zoom_out(amount);
    }

    /// 转发到内部控制器的 `home`。
    fn home(&mut self) {
        self.control.home();
    }
}

/// [`CameraControlPort`] 驱动端口与活 [`CesiumCamera`] 实体之间的
/// `PostUpdate` 桥接。
///
/// 在 [`super::camera_update_system`] 之前运行，以便 Transform 写入器看到
/// 端口驱动的位姿。逐帧的权威方向：
/// * 本帧发出了一个命令（`generation` 变化）**或**一个飞行处于
///   活动状态 → 将端口的相机推到实体上；
/// * 否则 → 将实体拷回端口，以便下一个编程命令（例如一次
///   大弧 `fly_to`）从鼠标/触控路径留给相机的位姿开始。
///
/// 无 [`CesiumCamera`] 实体时惰性。
pub fn camera_control_port_system(
    mut cameras: Query<&mut CesiumCamera>,
    mut port: ResMut<CameraControlPort>,
    time: Res<Time>,
) {
    let dt = time.delta_secs_f64();
    let was_flying = port.control.is_flying();
    let still_flying = port.control.update(dt);
    let commanded = port.control.generation() != port.last_gen;
    port.last_gen = port.control.generation();

    let Ok(mut cesium_cam) = cameras.get_single_mut() else {
        return;
    };
    if was_flying || still_flying || commanded {
        cesium_cam.camera = port.control.camera().clone();
    } else {
        *port.control.camera_mut() = cesium_cam.camera.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f64 = 6378137.0;

    fn equator_camera() -> Camera {
        Camera::new(
            DVec3::new(R * 2.0, 0.0, 0.0),
            DVec3::new(-1.0, 0.0, 0.0),
            DVec3::new(0.0, 0.0, 1.0),
        )
    }

    #[test]
    fn heading_pitch_roll_of_a_level_downward_camera() {
        // 赤道，直视下方，up = 北 → heading 0、pitch -π/2、roll 0。
        let cam = equator_camera();
        let (h, p, r) = compute_heading_pitch_roll(cam.position, cam.direction, cam.up);
        assert!(h.abs() < 1e-9 || (h - TAU).abs() < 1e-9, "heading {h}");
        // `asin` 会将在 nadir 处 1-ulp 的点积误差放大到 ~1.5e-8 rad，所以
        // 容差是 1e-6 rad（仍约 3e-6 度——可忽略）。
        assert!((p + FRAC_PI_2).abs() < 1e-6, "pitch {p}");
        assert!(r.abs() < 1e-6, "roll {r}");
    }

    #[test]
    fn set_view_places_camera_above_target_looking_down() {
        let mut ctrl = CameraControlImpl::new(equator_camera(), Ellipsoid::WGS84);
        let carto = Cartographic::from_degrees(10.0, 20.0, 500_000.0);
        ctrl.set_view(carto, 0.0, -FRAC_PI_2, 0.0);
        let expected = Ellipsoid::WGS84.cartographic_to_cartesian(&carto);
        let state = ctrl.get_camera_state();
        assert!((state.position - expected).length() < 1e-3, "{}", state.position);
        // 直视下方 ⇒ direction ≈ -法线，pitch ≈ -π/2。
        assert!((state.pitch + FRAC_PI_2).abs() < 1e-6, "pitch {}", state.pitch);
    }

    #[test]
    fn fly_to_great_arc_lands_on_destination() {
        let mut ctrl = CameraControlImpl::new(equator_camera(), Ellipsoid::WGS84);
        let dest = Cartographic::from_degrees(-75.0, 40.0, 1_000_000.0);
        ctrl.fly_to(dest, None, None, None, 0.0);
        assert!(ctrl.is_flying());
        let expected = Ellipsoid::WGS84.cartographic_to_cartesian(&dest);
        for _ in 0..1000 {
            if !ctrl.update(0.05) {
                break;
            }
        }
        assert!(!ctrl.is_flying(), "flight must complete");
        let err = (ctrl.get_camera_state().position - expected).length();
        assert!(err / R < 1e-9, "fly_to error {err} m");
    }

    #[test]
    fn look_at_holds_range_and_aims_at_target() {
        let mut ctrl = CameraControlImpl::new(equator_camera(), Ellipsoid::WGS84);
        let target = Cartographic::from_degrees(0.0, 0.0, 0.0);
        let range = 2_000_000.0;
        ctrl.look_at(target, 0.0, -FRAC_PI_2, range);
        let target_ecef = Ellipsoid::WGS84.cartographic_to_cartesian(&target);
        let state = ctrl.get_camera_state();
        let dist = (state.position - target_ecef).length();
        assert!((dist - range).abs() / range < 1e-9, "range {dist}");
        let aim = (target_ecef - state.position).normalize();
        assert!(aim.dot(state.direction) > 1.0 - 1e-9, "must aim at target");
    }

    #[test]
    fn zoom_in_then_out_moves_along_view() {
        let mut ctrl = CameraControlImpl::new(equator_camera(), Ellipsoid::WGS84);
        let len0 = ctrl.get_camera_state().position.length();
        ctrl.zoom_in(Some(100_000.0));
        let len1 = ctrl.get_camera_state().position.length();
        assert!((len0 - len1 - 100_000.0).abs() < 1.0, "in: {len0}→{len1}");
        ctrl.zoom_out(Some(250_000.0));
        let len2 = ctrl.get_camera_state().position.length();
        assert!((len2 - len1 - 250_000.0).abs() < 1.0, "out: {len1}→{len2}");
    }
}
