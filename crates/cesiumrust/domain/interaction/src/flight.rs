//! 相机飞行动画（fly_to、look_at）。
//!
//! 提供相机沿大圆弧的飞行动画：按时长与缓动在若干帧内
//! 插值位置与朝向，覆盖定向飞行、包围球飞行与返航等场景。

use cesium_camera::{Camera, EasingFunction};
use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::{BoundingSphere, HeadingPitchRange};
use glam::DVec3;

/// 相机飞行的选项。
#[derive(Debug, Clone)]
pub struct FlightOptions {
    /// 目标位置（ECEF）。
    pub destination: DVec3,
    /// 目标航向角（弧度）。
    pub heading: Option<f64>,
    /// 目标俯仰角（弧度）。
    pub pitch: Option<f64>,
    /// 目标翻滚角（弧度）。
    pub roll: Option<f64>,
    /// 目标方向（覆盖 heading/pitch）。
    pub direction: Option<DVec3>,
    /// 目标 up 向量。
    pub up: Option<DVec3>,
    /// 飞行时长（以秒计）。
    pub duration: f64,
    /// 缓动函数。
    pub easing: EasingFunction,
}

impl Default for FlightOptions {
    /// 默认飞行选项：目的地为原点，朝向/时长均为空（交由自动推导）。
    fn default() -> Self {
        Self {
            destination: DVec3::ZERO,
            heading: None,
            pitch: None,
            roll: None,
            direction: None,
            up: None,
            duration: 3.0,
            easing: EasingFunction::SinusoidalInOut,
        }
    }
}

/// 一条相机路径飞行动画。
#[derive(Debug, Clone)]
pub struct CameraFlight {
    /// 起始位置。
    pub start_position: DVec3,
    /// 结束位置。
    pub end_position: DVec3,
    /// 起始方向。
    pub start_direction: DVec3,
    /// 结束方向。
    pub end_direction: DVec3,
    /// 起始 up 向量。
    pub start_up: DVec3,
    /// 结束 up 向量。
    pub end_up: DVec3,
    /// 总时长（以秒计）。
    pub duration: f64,
    /// 已经过的时间（以秒计）。
    pub elapsed: f64,
    /// 飞行是否已完成。
    pub complete: bool,
    /// 飞行的缓动函数。
    pub easing: EasingFunction,
}

impl CameraFlight {
    /// 创建一个 flyTo 动画。
    ///
    /// # 参数
    /// * `camera` - 当前相机状态
    /// * `destination` - 目标位置（ECEF）
    /// * `direction` - 目标视线方向（可选）
    /// * `duration` - 飞行时长（以秒计）
    pub fn fly_to(
        camera: &Camera,
        destination: DVec3,
        direction: Option<DVec3>,
        up: Option<DVec3>,
        duration: f64,
    ) -> Self {
        let end_direction = direction.unwrap_or_else(|| {
            // 默认：从 destination 看向地心
            -destination.normalize()
        });
        let end_up = up.unwrap_or(DVec3::Z);

        Self {
            start_position: camera.position,
            end_position: destination,
            start_direction: camera.direction,
            end_direction: end_direction.normalize(),
            start_up: camera.up,
            end_up: end_up.normalize(),
            duration: duration.max(0.001),
            elapsed: 0.0,
            complete: false,
            easing: EasingFunction::SinusoidalInOut,
        }
    }

    /// 从大地坐标创建一个 flyTo。
    pub fn fly_to_cartographic(
        camera: &Camera,
        destination: &Cartographic,
        ellipsoid: &Ellipsoid,
        duration: f64,
    ) -> Self {
        let ecef = ellipsoid.cartographic_to_cartesian(destination);
        let direction = -ecef.normalize(); // 向下俯视
        Self::fly_to(camera, ecef, Some(direction), Some(DVec3::Z), duration)
    }

    /// 按时间增量更新飞行，并返回插值后的相机状态。
    ///
    /// # 参数
    /// * `dt` - 以秒计的时间增量
    ///
    /// # 返回
    /// 插值后的相机位置、方向与 up 向量
    pub fn update(&mut self, dt: f64) -> Option<(DVec3, DVec3, DVec3)> {
        if self.complete {
            return None;
        }

        self.elapsed += dt;
        let t = (self.elapsed / self.duration).clamp(0.0, 1.0);

        if t >= 1.0 {
            self.complete = true;
        }

        // 应用缓动函数
        let t_eased = self.easing.evaluate(t);

        // 大圆弧（slerp）插值，配合抛物线式高度拱起，使相机沿地球表面
        // 扫掠，而非走一条笔直的弦线。
        // 方向向量同样做 slerp，使旋转保持在最短角路径上
        // （四元数 slerp 保证旋转平滑且不过冲）。
        let position = slerp_great_arc(self.start_position, self.end_position, t_eased);
        let direction = slerp_unit(self.start_direction, self.end_direction, t_eased);
        let up = slerp_unit(self.start_up, self.end_up, t_eased);

        Some((position, direction, up))
    }

    /// 将当前飞行状态应用到相机。
    pub fn apply_to_camera(&mut self, camera: &mut Camera, dt: f64) -> bool {
        if let Some((position, direction, up)) = self.update(dt) {
            camera.position = position;
            camera.direction = direction;
            camera.right = direction.cross(up).normalize();
            camera.up = camera.right.cross(direction).normalize();
            !self.complete
        } else {
            false
        }
    }

    /// 返回进度（0.0 到 1.0）。
    pub fn progress(&self) -> f64 {
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }

    /// 从完整选项创建一个飞行。
    /// 依据目标、朝向、时长与缓动等完整参数构造飞行。
    pub fn fly_to_with_options(camera: &Camera, options: &FlightOptions) -> Self {
        let end_direction = if let Some(dir) = options.direction {
            dir.normalize()
        } else {
            // 由 heading/pitch 计算，或默认看向中心
            -options.destination.normalize()
        };
        let end_up = options.up.unwrap_or(DVec3::Z).normalize();

        Self {
            start_position: camera.position,
            end_position: options.destination,
            start_direction: camera.direction,
            end_direction,
            start_up: camera.up,
            end_up,
            duration: options.duration.max(0.001),
            elapsed: 0.0,
            complete: false,
            easing: options.easing,
        }
    }

    /// 创建一个用于查看包围球的飞行。
    /// 计算合适的距离与偏移，飞至可容纳整个包围球。
    pub fn fly_to_bounding_sphere(
        camera: &Camera,
        sphere: &BoundingSphere,
        offset: Option<&HeadingPitchRange>,
        duration: f64,
    ) -> Self {
        let default_offset = HeadingPitchRange::new(0.0, -std::f64::consts::FRAC_PI_4, 0.0);
        let offset = offset.unwrap_or(&default_offset);

        // 若未指定则计算距离
        let range = if offset.range > 0.0 {
            offset.range
        } else {
            // 默认：由球半径和 FOV 计算
            let fov = match &camera.frustum {
                cesium_camera::Frustum::Perspective(f) => f.fov,
                cesium_camera::Frustum::Orthographic(_) => std::f64::consts::FRAC_PI_3,
            };
            sphere.radius / (fov * 0.5).sin().max(0.001)
        };

        // 由球心 + 偏移计算 destination
        let cos_pitch = offset.pitch.cos();
        let dest_offset = DVec3::new(
            range * cos_pitch * offset.heading.cos(),
            range * cos_pitch * offset.heading.sin(),
            range * offset.pitch.sin(),
        );
        let destination = sphere.center + dest_offset;
        let direction = (sphere.center - destination).normalize();

        Self {
            start_position: camera.position,
            end_position: destination,
            start_direction: camera.direction,
            end_direction: direction,
            start_up: camera.up,
            end_up: DVec3::Z,
            duration: duration.max(0.001),
            elapsed: 0.0,
            complete: false,
            easing: EasingFunction::SinusoidalInOut,
        }
    }

    /// 创建一个飞往默认 home 视图的飞行。
    /// 飞往由椭球推算的默认 home 位置。
    pub fn fly_home(camera: &Camera, ellipsoid: &Ellipsoid, duration: f64) -> Self {
        let destination = Camera::default_home_position(ellipsoid);
        let direction = -destination.normalize();
        Self::fly_to(camera, destination, Some(direction), Some(DVec3::Z), duration)
    }

    /// 创建一个 `duration` 与 `easing` 均由行进距离自动推导的大圆弧飞行。
    ///
    /// - `duration = clamp(distance / 1e6, 1.0, 5.0)` 秒
    ///   （参见 [`compute_flight_duration`]）。
    /// - `easing` = 短途跳跃（`< 1e6` 米）用五次 in-out，否则用
    ///   三次 in-out（参见 [`select_flight_easing`]）。
    ///
    /// 当省略 `duration` 时，采用与距离自适应的缺省时长，
    /// 由 [`compute_flight_duration`] 推导。
    pub fn fly_to_great_arc(
        camera: &Camera,
        destination: DVec3,
        direction: Option<DVec3>,
        up: Option<DVec3>,
    ) -> Self {
        let distance = (destination - camera.position).length();
        let duration = compute_flight_duration(distance);
        let mut flight = Self::fly_to(camera, destination, direction, up, duration);
        flight.easing = select_flight_easing(distance);
        flight
    }
}

/// 计算一个“lookAt”相机朝向。
///
/// 将相机定位到从给定偏移看向目标。
///
/// # 参数
/// * `target` - 要看向的点（ECEF）
/// * `offset` - 相对目标的偏移（局部 ENU 或世界坐标）
///
/// # 返回
/// 相机位置、方向与 up 向量
pub fn compute_look_at(target: DVec3, offset: DVec3) -> (DVec3, DVec3, DVec3) {
    let position = target + offset;
    let direction = (target - position).normalize();

    // 选择不与 direction 平行的 up 向量
    let world_up = if direction.dot(DVec3::Z).abs() > 0.99 {
        DVec3::Y
    } else {
        DVec3::Z
    };

    let right = direction.cross(world_up).normalize();
    let up = right.cross(direction).normalize();

    (position, direction, up)
}

/// 计算一个从给定高度向下俯视某个大地坐标位置的相机视图。
///
/// # 参数
/// * `cartographic` - 要看向的位置
/// * `height` - 地表上方高度（米）
/// * `heading` - 相机航向角（弧度）
/// * `pitch` - 相机俯仰角（弧度，负值 = 向下看）
/// * `ellipsoid` - 椭球
///
/// # 返回
/// 相机位置、方向与 up 向量
pub fn compute_set_view(
    cartographic: &Cartographic,
    height: f64,
    heading: f64,
    pitch: f64,
    ellipsoid: &Ellipsoid,
) -> (DVec3, DVec3, DVec3) {
    // 目标上方的位置
    let target_carto = Cartographic::from_radians(
        cartographic.longitude,
        cartographic.latitude,
        height,
    );
    let position = ellipsoid.cartographic_to_cartesian(&target_carto);

    // 目标处的表面法线
    let surface_normal = position.normalize();

    // 由俯仰角和航向角计算方向
    // pitch = -PI/2 表示垂直向下看
    let pitch_from_nadir = pitch + std::f64::consts::FRAC_PI_2;

    // 方向：将表面法线按俯仰角旋转
    let east = DVec3::Z.cross(surface_normal).normalize();
    let north = surface_normal.cross(east).normalize();

    // 应用航向旋转以得到倾斜平面
    let tilt_dir = north * heading.cos() + east * heading.sin();

    // 方向是向下看与倾斜的组合
    let direction = (-surface_normal * pitch_from_nadir.cos() + tilt_dir * pitch_from_nadir.sin())
        .normalize();

    // `right` 必须垂直于视线方向。当垂直向下/向上看时，方向与表面法线
    // （反）平行，因此朴素的 `direction × normal` 会退化为零向量（normalize
    // 后→ NaN）。回退到航向的倾斜方向（水平的，⊥ normal），
    // 它会得到自然的以北为参考的 up；一个垂直轴可覆盖剩下的
    // 退化情形（极地处 `tilt_dir` 自身崩塌）。
    let right_ref = if direction.cross(surface_normal).length_squared() < 1e-18 {
        if tilt_dir.length_squared() < 1e-18 {
            perpendicular_axis(direction)
        } else {
            tilt_dir
        }
    } else {
        surface_normal
    };
    let right = direction.cross(right_ref).normalize();
    let up = right.cross(direction).normalize();

    (position, direction, up)
}

/// 飞行拱起的峰值半径系数。
///
/// 飞行中段的高度上限设为 `get_altitude(...) * 0.2` 的经验系数。鼓包
/// 按扫过角度缩放，因此共线的端点不会产生拱起。
const ARC_PEAK_FACTOR: f64 = 0.2;

/// 将 `v` 绕 `axis` 旋转 `angle`（Rodrigues 旋转公式）。
fn rotate_about_axis(v: DVec3, axis: DVec3, angle: f64) -> DVec3 {
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    v * cos_a + axis.cross(v) * sin_a + axis * axis.dot(v) * (1.0 - cos_a)
}

/// 返回任意一个垂直于 `v` 的单位向量（用于反平行的
/// slerp 回退，此时大圆弧平面本无法确定）。
fn perpendicular_axis(v: DVec3) -> DVec3 {
    let helper = if v.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
    v.cross(helper).normalize()
}

/// 在两个方向之间做球面线性插值，将其视为单位向量。
///
/// 处理朴素的 `sin` 加权公式会除以零的两种退化情形：
/// - 近平行（`dot ≈ 1`）：返回 `start`（弧的扫过角为零）；
/// - 近反平行（`dot ≈ -1`）：绕任意垂直轴旋转 `π·t`，
///   保持扫过连续且中点定义良好。
fn slerp_unit(start: DVec3, end: DVec3, t: f64) -> DVec3 {
    let a = start.normalize();
    let b = end.normalize();
    let dot = a.dot(b).clamp(-1.0, 1.0);

    if dot > 1.0 - 1e-12 {
        return a;
    }
    if dot < -1.0 + 1e-12 {
        let axis = perpendicular_axis(a);
        return rotate_about_axis(a, axis, std::f64::consts::PI * t);
    }

    let omega = dot.acos();
    let sin_omega = omega.sin();
    let wa = ((1.0 - t) * omega).sin() / sin_omega;
    let wb = (t * omega).sin() / sin_omega;
    (a * wa + b * wb).normalize()
}

/// 一个幂曲线拱起高度函数：当两个端点都低于 `altitude` 时在
/// `altitude` 处达到峰值，否则退化为普通的线性插值。
/// `power = 8`、`factor = 1e6` 给出适中拱形；
/// 在 `t = 0` 和 `t = 1` 处曲线复现端点高度，并在中间平滑地
/// 向 `altitude` 上升。
fn arc_height(start_height: f64, end_height: f64, altitude: f64, t: f64) -> f64 {
    const POWER: i32 = 8;
    const FACTOR: f64 = 1_000_000.0;
    let max_height = start_height.max(end_height);
    if max_height < altitude {
        let root = 1.0 / f64::from(POWER);
        let s = -((altitude - start_height) * FACTOR).powf(root);
        let e = ((altitude - end_height) * FACTOR).powf(root);
        let x = t * (e - s) + s;
        return -(x.powi(POWER)) / FACTOR + altitude;
    }
    start_height + (end_height - start_height) * t
}

/// 沿 `start` 与 `end` 之间的大圆弧插值一个位置。
///
/// 方向绕椭球中心做 slerp（恒定半径扫掠），而半径遵循 [`arc_height`]，
/// 使路径向外拱起，而非笔直地穿过地球走一条弦线。共线的端点（扫过角
/// 为零）不会产生鼓包，退化为一径向移动。
fn slerp_great_arc(start: DVec3, end: DVec3, t: f64) -> DVec3 {
    let start_radius = start.length();
    let end_radius = end.length();
    // 位于中心的端点没有方向；回退到直线 lerp。
    if start_radius < 1e-9 || end_radius < 1e-9 {
        return start + (end - start) * t;
    }

    let start_dir = start / start_radius;
    let end_dir = end / end_radius;
    let omega = start_dir.angle_between(end_dir);
    let direction = slerp_unit(start_dir, end_dir, t);

    let mean_radius = 0.5 * (start_radius + end_radius);
    let bulge = (omega / std::f64::consts::PI) * mean_radius * ARC_PEAK_FACTOR;
    let peak_radius = start_radius.max(end_radius) + bulge;

    direction * arc_height(start_radius, end_radius, peak_radius, t)
}

/// 由行进距离计算飞行时长：
/// `clamp(distance / 1e6, 1.0, 5.0)` 秒。
///
/// 短途跳跃至少给满一秒以免瞬间到位；超长飞行上限为五秒。
/// 这是一种基于距离缩放的时长启发式，
/// 使观感随飞行远近平滑变化。
pub fn compute_flight_duration(distance: f64) -> f64 {
    (distance / 1_000_000.0).clamp(1.0, 5.0)
}

/// 按行进距离选择飞行缓动：短途飞行（`< 1e6` 米，起停更柔和）用五次
/// in-out，长途用三次 in-out。
pub fn select_flight_easing(distance: f64) -> EasingFunction {
    if distance < 1_000_000.0 {
        EasingFunction::QuinticInOut
    } else {
        EasingFunction::CubicInOut
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 平滑阶跃插值（Hermite）- 测试辅助函数。
    fn smoothstep(t: f64) -> f64 {
        t * t * (3.0 - 2.0 * t)
    }

    fn create_test_camera() -> Camera {
        Camera::new(
            DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
            DVec3::new(-1.0, 0.0, 0.0),
            DVec3::new(0.0, 0.0, 1.0),
        )
    }

    #[test]
    fn test_camera_flight_creation() {
        let camera = create_test_camera();
        let destination = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);

        let flight = CameraFlight::fly_to(&camera, destination, None, None, 2.0);

        assert_eq!(flight.start_position, camera.position);
        assert_eq!(flight.end_position, destination);
        assert_eq!(flight.duration, 2.0);
        assert!(!flight.complete);
    }

    #[test]
    fn test_camera_flight_update() {
        let camera = create_test_camera();
        let destination = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);

        let mut flight = CameraFlight::fly_to(&camera, destination, None, None, 2.0);

        // 在 t=0
        let (pos, _, _) = flight.update(0.0).unwrap();
        assert!((pos - camera.position).length() < 1.0);

        // 在 t=1（中点）
        let (pos, _, _) = flight.update(1.0).unwrap();
        let midpoint = (camera.position + destination) / 2.0;
        assert!((pos - midpoint).length() / midpoint.length() < 0.01);

        // 在 t=2（终点）
        let (pos, _, _) = flight.update(1.0).unwrap();
        assert!((pos - destination).length() < 1.0);
        assert!(flight.complete);
    }

    #[test]
    fn test_camera_flight_progress() {
        let camera = create_test_camera();
        let destination = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);

        let mut flight = CameraFlight::fly_to(&camera, destination, None, None, 4.0);

        flight.update(1.0);
        assert!((flight.progress() - 0.25).abs() < 1e-10);

        flight.update(1.0);
        assert!((flight.progress() - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_camera_flight_apply() {
        let camera = create_test_camera();
        let destination = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);

        let mut flight = CameraFlight::fly_to(&camera, destination, None, None, 1.0);
        let mut camera = camera;

        // 应用完整时长
        let still_flying = flight.apply_to_camera(&mut camera, 1.0);
        assert!(!still_flying); // 飞行完成
        assert!((camera.position - destination).length() < 1.0);
    }

    #[test]
    fn test_compute_look_at() {
        let target = DVec3::new(6378137.0, 0.0, 0.0);
        let offset = DVec3::new(1000000.0, 0.0, 0.0);

        let (position, direction, up) = compute_look_at(target, offset);

        // 位置应为 target + offset
        assert!((position - (target + offset)).length() < 1e-6);

        // 方向应从 position 指向 target
        let expected_dir = (target - position).normalize();
        assert!((direction - expected_dir).length() < 1e-10);

        // up 应垂直于 direction
        assert!(direction.dot(up).abs() < 1e-10);
    }

    #[test]
    fn test_compute_set_view() {
        let carto = Cartographic::from_radians(0.0, 0.0, 0.0);
        let height = 1000000.0;

        let (position, direction, _up) = compute_set_view(
            &carto,
            height,
            0.0,
            -std::f64::consts::FRAC_PI_2, // 垂直向下看
            &Ellipsoid::WGS84,
        );

        // 位置应在赤道/本初子午线上方的给定高度处
        let pos_height = position.length() - Ellipsoid::WGS84.maximum_radius();
        assert!((pos_height - height).abs() / height < 0.01);

        // 方向应大致指向中心（向下看）
        let to_center = -position.normalize();
        assert!(direction.dot(to_center) > 0.9);
    }

    #[test]
    fn test_smoothstep() {
        assert!((smoothstep(0.0)).abs() < 1e-10);
        assert!((smoothstep(1.0) - 1.0).abs() < 1e-10);
        assert!((smoothstep(0.5) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_fly_to_cartographic() {
        let camera = create_test_camera();
        let dest = Cartographic::from_radians(0.5, 0.3, 10000.0);

        let flight = CameraFlight::fly_to_cartographic(&camera, &dest, &Ellipsoid::WGS84, 3.0);

        assert_eq!(flight.duration, 3.0);
        assert!(!flight.complete);
        // 结束位置应在椭球上给定的大地坐标处
        let expected_pos = Ellipsoid::WGS84.cartographic_to_cartesian(&dest);
        assert!((flight.end_position - expected_pos).length() < 1.0);
    }

    #[test]
    fn test_slerp_unit_endpoints_and_antiparallel() {
        let a = DVec3::X;
        let b = DVec3::Y;
        assert!((slerp_unit(a, b, 0.0) - a).length() < 1e-12);
        assert!((slerp_unit(a, b, 1.0) - b).length() < 1e-12);
        // 反平行回退必须保持单位长度且连续。
        let mid = slerp_unit(DVec3::X, -DVec3::X, 0.5);
        assert!((mid.length() - 1.0).abs() < 1e-12);
    }

    /// 任务要求：大圆弧中点与测地线中点
    /// `normalize(start_dir + end_dir)` 的偏离 < 1e-6 弧度。
    #[test]
    fn test_great_arc_midpoint_deviation_below_epsilon() {
        let r = 6378137.0 * 2.0;
        let camera = Camera::new(DVec3::new(r, 0.0, 0.0), -DVec3::X, DVec3::Z);
        let destination = DVec3::new(0.0, r, 0.0);
        let mut flight = CameraFlight::fly_to(&camera, destination, None, None, 2.0);

        // t = 0.5（SinusoidalInOut(0.5) == 0.5）。
        let (pos, _, _) = flight.update(1.0).unwrap();
        let expected_dir = (DVec3::X + DVec3::Y).normalize();
        let deviation = expected_dir.dot(pos.normalize()).clamp(-1.0, 1.0).acos();
        assert!(
            deviation < 1e-6,
            "midpoint angular deviation {deviation} rad exceeds 1e-6"
        );
    }

    #[test]
    fn test_great_arc_bulges_outward() {
        let r = 6378137.0 * 2.0;
        let camera = Camera::new(DVec3::new(r, 0.0, 0.0), -DVec3::X, DVec3::Z);
        let destination = DVec3::new(0.0, r, 0.0);
        let mut flight = CameraFlight::fly_to(&camera, destination, None, None, 2.0);

        let (pos, _, _) = flight.update(1.0).unwrap();
        // 拱起将飞行中段半径抬高到两个端点之上。
        assert!(pos.length() > r, "arc should bulge: |pos| = {}", pos.length());
    }

    #[test]
    fn test_arc_height_reproduces_endpoints() {
        assert!((arc_height(100.0, 200.0, 500.0, 0.0) - 100.0).abs() < 1e-6);
        assert!((arc_height(100.0, 200.0, 500.0, 1.0) - 200.0).abs() < 1e-6);
        let mid = arc_height(100.0, 200.0, 500.0, 0.5);
        assert!(mid > 200.0 && mid <= 500.0 + 1e-6, "arch peak out of range: {mid}");
        // altitude 低于两个端点 ⇒ 线性分支。
        assert!((arc_height(100.0, 200.0, 150.0, 0.5) - 150.0).abs() < 1e-6);
    }

    #[test]
    fn test_compute_flight_duration_clamps() {
        assert!((compute_flight_duration(500_000.0) - 1.0).abs() < 1e-12);
        assert!((compute_flight_duration(2_500_000.0) - 2.5).abs() < 1e-12);
        assert!((compute_flight_duration(10_000_000.0) - 5.0).abs() < 1e-12);
    }

    #[test]
    fn test_select_flight_easing_by_distance() {
        assert_eq!(select_flight_easing(500_000.0), EasingFunction::QuinticInOut);
        assert_eq!(select_flight_easing(2_000_000.0), EasingFunction::CubicInOut);
    }

    #[test]
    fn test_fly_to_great_arc_derives_duration_and_easing() {
        let camera = create_test_camera();
        let destination = DVec3::new(0.0, 6378137.0 * 3.0, 0.0);
        let flight = CameraFlight::fly_to_great_arc(&camera, destination, None, None);
        // |dest - pos| = 3R√2 ≈ 2.7e7 → duration 钳制到 5.0，缓动为三次。
        assert!((flight.duration - 5.0).abs() < 1e-9);
        assert_eq!(flight.easing, EasingFunction::CubicInOut);
    }
}
