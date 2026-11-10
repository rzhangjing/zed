//! 用于环绕、平移与缩放交互的相机控制器。
//!
//! 消费适配器提供的像素/角度增量，按配置的敏感度与开关驱动相机。

use cesium_camera::Camera;
use cesium_geospatial::ellipsoid::Ellipsoid;
use glam::{DVec2, DVec3};

use crate::inertia::{InertiaController, InertiaSample, InertiaState};

/// 相机控制器配置。
#[derive(Debug, Clone)]
pub struct CameraControllerConfig {
    /// 距地表的最小缩放距离（米）。
    pub minimum_zoom_distance: f64,
    /// 距地表的最大缩放距离（米）。
    pub maximum_zoom_distance: f64,
    /// 旋转速度因子。
    pub rotation_speed: f64,
    /// 平移速度因子。
    pub pan_speed: f64,
    /// 缩放速度因子。
    pub zoom_speed: f64,
    /// 是否启用旋转。
    pub enable_rotation: bool,
    /// 是否启用平移。
    pub enable_pan: bool,
    /// 是否启用缩放。
    pub enable_zoom: bool,
    /// 是否启用与椭球的碰撞检测。
    pub enable_collision_detection: bool,
}

impl Default for CameraControllerConfig {
    /// 默认控制器配置：启用全部交互，速度因子为 1.0。
    fn default() -> Self {
        Self {
            minimum_zoom_distance: 1.0,
            maximum_zoom_distance: f64::INFINITY,
            rotation_speed: 1.0,
            pan_speed: 1.0,
            zoom_speed: 1.0,
            enable_rotation: true,
            enable_pan: true,
            enable_zoom: true,
            enable_collision_detection: true,
        }
    }
}

/// 处理用户输入并更新相机的控制器。
///
/// 保存配置与输入状态，将手势增量换算为相机运动。
#[derive(Debug, Clone)]
pub struct CameraController {
    /// 配置。
    pub config: CameraControllerConfig,
    /// 用于表面计算的椭球。
    pub ellipsoid: Ellipsoid,
}

impl CameraController {
    /// 创建一个新的相机控制器。
    pub fn new(ellipsoid: Ellipsoid) -> Self {
        Self {
            config: CameraControllerConfig::default(),
            ellipsoid,
        }
    }

    /// 使相机围绕一个目标点环绕。
    ///
    /// # 参数
    /// * `camera` - 要更新的相机
    /// * `target` - 围绕环绕的点（ECEF）
    /// * `delta_heading` - 航向角的变化（弧度）
    /// * `delta_pitch` - 俯仰角的变化（弧度）
    /// * `delta_range` - 距离的变化（米，正值 = 缩小）
    pub fn orbit(
        &self,
        camera: &mut Camera,
        target: DVec3,
        delta_heading: f64,
        delta_pitch: f64,
        delta_range: f64,
    ) {
        if !self.config.enable_rotation {
            return;
        }

        let heading = delta_heading * self.config.rotation_speed;
        let pitch = delta_pitch * self.config.rotation_speed;

        // 从目标到相机的向量
        let offset = camera.position - target;
        let range = offset.length() + delta_range * self.config.zoom_speed;

        // 钳制距离
        let range = range.max(self.config.minimum_zoom_distance);
        let range = if self.config.maximum_zoom_distance.is_finite() {
            range.min(self.config.maximum_zoom_distance)
        } else {
            range
        };

        // 转换为球坐标
        let mut current_heading = offset.z.atan2(offset.x);
        let horizontal_dist = (offset.x * offset.x + offset.z * offset.z).sqrt();
        let mut current_pitch = offset.y.atan2(horizontal_dist);

        // 应用增量
        current_heading += heading;
        current_pitch += pitch;

        // 钳制俯仰角以避免万向节问题
        let max_pitch = std::f64::consts::FRAC_PI_2 - 0.001;
        current_pitch = current_pitch.clamp(-max_pitch, max_pitch);

        // 转换回笛卡尔坐标
        let cos_pitch = current_pitch.cos();
        let new_offset = DVec3::new(
            range * cos_pitch * current_heading.cos(),
            range * current_pitch.sin(),
            range * cos_pitch * current_heading.sin(),
        );

        camera.position = target + new_offset;
        camera.direction = (target - camera.position).normalize();
        camera.right = camera.direction.cross(DVec3::Y).normalize();
        camera.up = camera.right.cross(camera.direction).normalize();
    }

    /// 沿视平面平移相机。
    ///
    /// # 参数
    /// * `camera` - 要更新的相机
    /// * `delta_x` - 水平平移量（归一化，-1 到 1）
    /// * `delta_y` - 垂直平移量（归一化，-1 到 1）
    pub fn pan(&self, camera: &mut Camera, delta_x: f64, delta_y: f64) {
        if !self.config.enable_pan {
            return;
        }

        // 按到地表的距离缩放平移
        let height = camera.position.length() - self.ellipsoid.maximum_radius();
        let pan_scale = height.abs().max(1000.0) * 0.001 * self.config.pan_speed;

        let move_right = camera.right * (-delta_x * pan_scale);
        let move_up = camera.up * (delta_y * pan_scale);

        camera.position += move_right + move_up;
    }

    /// 将相机放大或缩小。
    ///
    /// # 参数
    /// * `camera` - 要更新的相机
    /// * `delta` - 缩放量（正值 = 放大，负值 = 缩小）
    pub fn zoom(&self, camera: &mut Camera, delta: f64) {
        if !self.config.enable_zoom {
            return;
        }

        // 按到地表的距离缩放
        let height = camera.position.length() - self.ellipsoid.maximum_radius();
        let zoom_amount = height.abs().max(1000.0) * 0.1 * delta * self.config.zoom_speed;

        let movement = camera.direction * zoom_amount;
        let new_position = camera.position + movement;

        // 碰撞检测
        if self.config.enable_collision_detection {
            let new_height = new_position.length() - self.ellipsoid.maximum_radius();
            if new_height < self.config.minimum_zoom_distance {
                return; // 不缩到最小距离以下
            }
        }

        camera.position = new_position;
    }

    /// 俯仰倾斜相机（在看向目标时改变俯仰角）。
    ///
    /// # 参数
    /// * `camera` - 要更新的相机
    /// * `target` - 要看向的点（ECEF）
    /// * `delta_pitch` - 俯仰角的变化（弧度）
    pub fn tilt(&self, camera: &mut Camera, target: DVec3, delta_pitch: f64) {
        let offset = camera.position - target;
        let range = offset.length();

        // 将偏移绕 right 轴旋转
        let surface_normal = target.normalize();
        let right = offset.cross(surface_normal).normalize();
        let rotated = rotate_around_axis(offset.normalize(), right, delta_pitch * self.config.rotation_speed);

        camera.position = target + rotated * range;
        camera.direction = (target - camera.position).normalize();
        camera.right = camera.direction.cross(DVec3::Y).normalize();
        camera.up = camera.right.cross(camera.direction).normalize();
    }

    /// 绕椭球中心旋转（自转）相机：位置与
    /// 朝向一同旋转，因此地球看起来在观察者下方转动。
    ///
    /// 自转分两步：先 `camera.rotate_right(delta_phi)`，再 `camera.rotate_up(delta_theta)`。
    /// 像素→弧度的缩放是适配器的职责；本方法
    /// 接受带符号的角度（以弧度计）。
    ///
    /// # 参数
    /// * `camera` - 要更新的相机
    /// * `delta_heading` - 绕相机 up 轴的旋转（弧度）
    /// * `delta_pitch` - 绕相机 right 轴的旋转（弧度）
    pub fn spin(&self, camera: &mut Camera, delta_heading: f64, delta_pitch: f64) {
        if !self.config.enable_rotation {
            return;
        }
        let heading = delta_heading * self.config.rotation_speed;
        let pitch = delta_pitch * self.config.rotation_speed;
        camera.rotate_right(heading);
        camera.rotate_up(pitch);
    }

    /// 原地环视：旋转朝向（direction/up）而
    /// 不移动位置。
    ///
    /// 环视分两步：先水平 `camera.look_left(angle)`，再绕 right 轴垂直 `look`。
    ///
    /// # 参数
    /// * `camera` - 要更新的相机
    /// * `delta_heading` - 偏航角（弧度）
    /// * `delta_pitch` - 俯仰角（弧度）
    pub fn look(&self, camera: &mut Camera, delta_heading: f64, delta_pitch: f64) {
        if !self.config.enable_rotation {
            return;
        }
        let heading = delta_heading * self.config.rotation_speed;
        let pitch = delta_pitch * self.config.rotation_speed;
        camera.look_left(Some(heading));
        camera.look_up(Some(pitch));
    }

    /// 绕自身视线方向扭转（滚动）相机。
    ///
    /// 翻滚直接施加 `camera.twist_right(theta)`（正值 = 顺时针）。
    ///
    /// # 参数
    /// * `camera` - 要更新的相机
    /// * `delta_angle` - 翻滚角（弧度，正值 = 顺时针）
    pub fn twist(&self, camera: &mut Camera, delta_angle: f64) {
        if !self.config.enable_rotation {
            return;
        }
        camera.twist_right(delta_angle * self.config.rotation_speed);
    }

    // ========================================================================
    // 双指（捻合）触控手势 — M2.6
    // ========================================================================
    //
    // 这些将 [`crate::event_aggregator::CameraEventAggregator`] 产生的
    // 分解后的双指度量（`pinch_distance_delta`、`pinch_angle_delta`、
    // `pinch_midpoint_delta`）映射到相机变换上：
    //
    // | 手势 | 度量 | 相机动作 |
    // |---------|--------|---------------|
    // | 捻合（手指张开/合拢） | 距离 Δ（px） | [`Self::zoom`] |
    // | 旋转（手指绕中点转动） | 角度 Δ（rad） | [`Self::spin`]（航向） |
    // | 拖拽（手指一同平移） | 中点 Δ（px） | [`Self::pan`] |
    //
    // 根据 M2.6 手势模型，双指 **旋转映射到 spin**（
    // 地球在观察者下方转动），而非把旋转当作滚转（翻滚）；
    // 滚动映射仍可通过 [`Self::twist`] +
    // `pinch_twist_pixels` 为偏好它的应用保留。
    //
    // 像素→世界的量级缩放留在适配器边界：像素
    // 增量经适配器提供的 `scale` 透传（镜像
    // [`Self::coast_inertia`]），因此领域只拥有手势→动作的
    // *语义*（哪个分量驱动哪个运动，及其符号），
    // 并保持与分辨率无关、不受 render-unit 干扰。

    /// 双指 **捻合 → 缩放**。
    ///
    /// `distance_delta` 为手指间距的变化（以像素计）
    /// （正值 = 手指张开 = **放大**，与惯常的捻合缩放手感一致）。`scale` 将像素转换为
    /// [`Self::zoom`] 所消费的无维度缩放量（由适配器提供，例如
    /// 基于画布高度的敏感度）。遵循 `enable_zoom`。
    ///
    /// # 参数
    /// * `camera` - 要更新的相机
    /// * `distance_delta` - 本帧手指间距的变化（像素）
    /// * `scale` - 由适配器边界提供的像素→缩放量缩放
    pub fn pinch_zoom(&self, camera: &mut Camera, distance_delta: f64, scale: f64) {
        // 张开手指（正增量）放大（正缩放量）。
        self.zoom(camera, distance_delta * scale);
    }

    /// 双指 **旋转 → spin**（航向）。
    ///
    /// `angle_delta` 为手指连线的旋转（以弧度计）
    /// （正值 = 屏幕上逆时针，来自 `atan2`）。它已经是
    /// 一个角度，因此不施加像素缩放；直接映射到
    /// [`Self::spin`] 的航向分量（pitch = 0）。遵循 `enable_rotation`。
    ///
    /// # 参数
    /// * `camera` - 要更新的相机
    /// * `angle_delta` - 本帧手指连线的旋转（弧度）
    pub fn pinch_rotate(&self, camera: &mut Camera, angle_delta: f64) {
        self.spin(camera, angle_delta, 0.0);
    }

    /// 双指 **拖拽 → 平移**（pan）。
    ///
    /// `midpoint_delta` 为两指共同的平移（以像素计）
    /// （两指中点的变化）。`scale` 将像素转换为
    /// [`Self::pan`] 所消费的归一化平移量（由适配器提供）；
    /// 现有的 `pan` 符号约定（沿 right 的 `-delta_x`、沿 up 的 `+delta_y`）
    /// 给出抓住地球的手感。遵循 `enable_pan`。
    ///
    /// # 参数
    /// * `camera` - 要更新的相机
    /// * `midpoint_delta` - 本帧两指共同的平移（像素）
    /// * `scale` - 由适配器边界提供的像素→平移量缩放
    pub fn pinch_translate(&self, camera: &mut Camera, midpoint_delta: DVec2, scale: f64) {
        self.pan(camera, midpoint_delta.x * scale, midpoint_delta.y * scale);
    }

    /// 为 `slot` 应用一步惯性滑行，用
    /// [`InertiaController::maintain`] 返回的衰减运动驱动相应的
    /// 相机运动。
    ///
    /// 这是惯性维持的领域级整合：每一步中，
    /// 存储的像素运动按
    /// [`crate::inertia::decay`] 指数函数收窄，并馈入 `spin`/`zoom`/`pan`/
    /// `tilt`。`scale` 将像素增量转换为弧度（用于 spin/tilt）或
    /// 米（用于 zoom/pan）；在此提供它可将像素→世界的
    /// 转换 ——— 包括任何 render-unit 缩放 ——— 留在适配器边界。
    ///
    /// # 返回
    /// 相机仍在滑行时返回 `true`，惯性停止后（被释放、
    /// 按下太久，或衰减到停止距离以下）返回 `false`。
    ///
    /// # 参数
    /// * `camera` - 要更新的相机
    /// * `inertia` - 持有已捕获运动的惯性控制器
    /// * `slot` - 要滑行哪个惯性状态
    /// * `target` - tilt 惯性的支点（ECEF）
    /// * `sample` - 逐帧时序 + 衰减系数
    /// * `scale` - 应用于衰减后增量的像素→弧度/米缩放
    pub fn coast_inertia(
        &self,
        camera: &mut Camera,
        inertia: &mut InertiaController,
        slot: InertiaState,
        target: DVec3,
        sample: &InertiaSample,
        scale: f64,
    ) -> bool {
        let delta = match inertia.maintain(slot, sample) {
            Some(delta) => delta,
            None => return false,
        };
        match slot {
            InertiaState::Spin => self.spin(camera, delta.x * scale, delta.y * scale),
            InertiaState::Zoom => self.zoom(camera, -delta.y * scale),
            InertiaState::Translate => self.pan(camera, delta.x * scale, delta.y * scale),
            InertiaState::Tilt => self.tilt(camera, target, delta.y * scale),
        }
        true
    }

    /// 确保相机不处于椭球表面以下。
    pub fn enforce_collision(&self, camera: &mut Camera) {
        if !self.config.enable_collision_detection {
            return;
        }

        let height = camera.position.length() - self.ellipsoid.maximum_radius();
        if height < self.config.minimum_zoom_distance {
            let normal = camera.position.normalize();
            camera.position = normal * (self.ellipsoid.maximum_radius() + self.config.minimum_zoom_distance);
        }
    }
}

/// 将向量绕轴旋转一个角度（Rodrigues 公式）。
fn rotate_around_axis(v: DVec3, axis: DVec3, angle: f64) -> DVec3 {
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    v * cos_a + axis.cross(v) * sin_a + axis * axis.dot(v) * (1.0 - cos_a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_camera() -> Camera {
        // 位于赤道上方、向下看的相机
        Camera::new(
            DVec3::new(6378137.0 * 2.0, 0.0, 0.0),
            DVec3::new(-1.0, 0.0, 0.0),
            DVec3::new(0.0, 0.0, 1.0),
        )
    }

    #[test]
    fn test_camera_controller_creation() {
        let controller = CameraController::new(Ellipsoid::WGS84);
        assert!(controller.config.enable_rotation);
        assert!(controller.config.enable_pan);
        assert!(controller.config.enable_zoom);
    }

    #[test]
    fn test_zoom_in() {
        let controller = CameraController::new(Ellipsoid::WGS84);
        let mut camera = create_test_camera();
        let initial_distance = camera.position.length();

        controller.zoom(&mut camera, 1.0); // 放大

        assert!(camera.position.length() < initial_distance);
    }

    #[test]
    fn test_zoom_out() {
        let controller = CameraController::new(Ellipsoid::WGS84);
        let mut camera = create_test_camera();
        let initial_distance = camera.position.length();

        controller.zoom(&mut camera, -1.0); // 缩小

        assert!(camera.position.length() > initial_distance);
    }

    #[test]
    fn test_zoom_disabled() {
        let mut controller = CameraController::new(Ellipsoid::WGS84);
        controller.config.enable_zoom = false;
        let mut camera = create_test_camera();
        let initial_pos = camera.position;

        controller.zoom(&mut camera, 1.0);

        assert_eq!(camera.position, initial_pos);
    }

    #[test]
    fn test_pan() {
        let controller = CameraController::new(Ellipsoid::WGS84);
        let mut camera = create_test_camera();
        let initial_pos = camera.position;

        controller.pan(&mut camera, 1.0, 0.0);

        assert_ne!(camera.position, initial_pos);
    }

    #[test]
    fn test_orbit() {
        let controller = CameraController::new(Ellipsoid::WGS84);
        let mut camera = create_test_camera();
        let target = DVec3::ZERO;
        let initial_distance = (camera.position - target).length();

        controller.orbit(&mut camera, target, 0.1, 0.0, 0.0);

        // 纯旋转期间距离应保持不变
        let new_distance = (camera.position - target).length();
        assert!((new_distance - initial_distance).abs() / initial_distance < 0.01);
    }

    #[test]
    fn test_collision_detection() {
        let controller = CameraController::new(Ellipsoid::WGS84);
        let mut camera = Camera::new(
            DVec3::new(6378137.0 + 0.5, 0.0, 0.0), // 非常贴近表面
            DVec3::new(-1.0, 0.0, 0.0),
            DVec3::new(0.0, 0.0, 1.0),
        );

        controller.enforce_collision(&mut camera);

        let height = camera.position.length() - Ellipsoid::WGS84.maximum_radius();
        assert!(height >= controller.config.minimum_zoom_distance);
    }

    #[test]
    fn test_rotate_around_axis() {
        let v = DVec3::new(1.0, 0.0, 0.0);
        let axis = DVec3::new(0.0, 0.0, 1.0);
        let angle = std::f64::consts::FRAC_PI_2;

        let rotated = rotate_around_axis(v, axis, angle);

        // 绕 Z 轴 90 度：X → Y
        assert!((rotated.x).abs() < 1e-10);
        assert!((rotated.y - 1.0).abs() < 1e-10);
        assert!((rotated.z).abs() < 1e-10);
    }

    #[test]
    fn test_spin_rotates_about_center_preserving_distance() {
        let controller = CameraController::new(Ellipsoid::WGS84);
        let mut camera = create_test_camera();
        let initial_distance = camera.position.length();
        let initial_pos = camera.position;

        controller.spin(&mut camera, 0.1, 0.0);

        // 位置移动了，但仍处于绕中心的同一球面上。
        assert!((camera.position - initial_pos).length() > 1.0);
        assert!((camera.position.length() - initial_distance).abs() / initial_distance < 1e-9);
        // 朝向保持正交归一。
        assert!((camera.direction.length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn test_spin_disabled_no_change() {
        let mut controller = CameraController::new(Ellipsoid::WGS84);
        controller.config.enable_rotation = false;
        let mut camera = create_test_camera();
        let initial_pos = camera.position;

        controller.spin(&mut camera, 0.5, 0.5);

        assert_eq!(camera.position, initial_pos);
    }

    #[test]
    fn test_look_rotates_orientation_only() {
        let controller = CameraController::new(Ellipsoid::WGS84);
        let mut camera = create_test_camera();
        let initial_pos = camera.position;
        let initial_dir = camera.direction;

        controller.look(&mut camera, 0.2, 0.0);

        // 位置不受影响；方向改变。
        assert_eq!(camera.position, initial_pos);
        assert!(camera.direction.dot(initial_dir) < 0.999999);
        assert!((camera.direction.length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn test_look_disabled_no_change() {
        let mut controller = CameraController::new(Ellipsoid::WGS84);
        controller.config.enable_rotation = false;
        let mut camera = create_test_camera();
        let initial_dir = camera.direction;

        controller.look(&mut camera, 0.3, 0.3);

        assert!((camera.direction - initial_dir).length() < 1e-12);
    }

    #[test]
    fn test_twist_rolls_about_view_direction() {
        let controller = CameraController::new(Ellipsoid::WGS84);
        let mut camera = create_test_camera();
        let initial_pos = camera.position;
        let initial_dir = camera.direction;
        let initial_up = camera.up;

        controller.twist(&mut camera, 0.3);

        // 滚动保持位置与视线方向，但旋转 `up`。
        assert_eq!(camera.position, initial_pos);
        assert!(camera.direction.dot(initial_dir) > 0.999999);
        assert!(camera.up.dot(initial_up) < 0.999999);
        assert!((camera.up.length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn test_coast_inertia_spin_moves_and_decays() {
        use crate::inertia::{InertiaController, InertiaSample, InertiaState};
        use glam::DVec2;

        let controller = CameraController::new(Ellipsoid::WGS84);
        let mut camera = create_test_camera();
        let mut inertia = InertiaController::new();
        inertia.capture(InertiaState::Spin, DVec2::ZERO, DVec2::new(2000.0, 0.0));

        // 像素→弧度缩放由适配器边界提供。
        let scale = 1e-4;
        let target = DVec3::ZERO;

        let s1 = InertiaSample::new(0.9, 0.0, 0.0, 16.0);
        let pos0 = camera.position;
        assert!(controller.coast_inertia(&mut camera, &mut inertia, InertiaState::Spin, target, &s1, scale));
        let step1 = (camera.position - pos0).length();
        assert!(step1 > 0.0);

        // 较后的帧衰减得更多 → 步长更小。
        let s2 = InertiaSample::new(0.9, 0.0, 0.0, 600.0);
        let pos1 = camera.position;
        assert!(controller.coast_inertia(&mut camera, &mut inertia, InertiaState::Spin, target, &s2, scale));
        let step2 = (camera.position - pos1).length();
        assert!(step2 < step1, "inertia step must decay: {step2} !< {step1}");
    }

    #[test]
    fn test_coast_inertia_stops_when_disabled() {
        use crate::inertia::{InertiaController, InertiaSample, InertiaState};
        use glam::DVec2;

        let controller = CameraController::new(Ellipsoid::WGS84);
        let mut camera = create_test_camera();
        let mut inertia = InertiaController::new();
        inertia.capture(InertiaState::Translate, DVec2::ZERO, DVec2::new(800.0, 0.0));
        inertia.deactivate(InertiaState::Translate);

        let sample = InertiaSample::new(0.9, 0.0, 0.0, 16.0);
        let initial = camera.position;
        let coasting = controller.coast_inertia(
            &mut camera,
            &mut inertia,
            InertiaState::Translate,
            DVec3::ZERO,
            &sample,
            1.0,
        );
        assert!(!coasting);
        assert_eq!(camera.position, initial);
    }

    // ========================================================================
    // M2.6 双指（捻合）触控手势序列
    // ========================================================================

    use crate::event_aggregator::CameraEventAggregator;

    /// 推进一步双指手势的单帧。聚合器在一帧的首个
    /// `pinch_move` 上播种 `start`，在第二个上延长 `end`
    /// （复现逐帧捻合的采样方式），因此喂入帧的起始对
    /// `(a1,a2)` 与结束对 `(b1,b2)` 会产生 `metrics(b) - metrics(a)` 的逐帧增量。
    fn pinch_frame(
        agg: &mut CameraEventAggregator,
        t: f64,
        a1: DVec2,
        a2: DVec2,
        b1: DVec2,
        b2: DVec2,
    ) {
        agg.reset(t);
        agg.pinch_move(a1, a2);
        agg.pinch_move(b1, b2);
    }

    /// 将一个完整的双指帧（缩放 + 旋转 + 平移）应用到相机。
    /// `a`/`b` 为帧的起始/结束手指对；`scales` 为
    /// `(zoom_scale, translate_scale)`。
    fn apply_pinch_frame(
        agg: &mut CameraEventAggregator,
        ctrl: &CameraController,
        camera: &mut Camera,
        t: f64,
        a: (DVec2, DVec2),
        b: (DVec2, DVec2),
        scales: (f64, f64),
    ) {
        pinch_frame(agg, t, a.0, a.1, b.0, b.1);
        ctrl.pinch_zoom(camera, agg.pinch_distance_delta(), scales.0);
        ctrl.pinch_rotate(camera, agg.pinch_angle_delta());
        ctrl.pinch_translate(camera, agg.pinch_midpoint_delta(), scales.1);
    }

    #[test]
    fn pinch_spread_zooms_in() {
        let ctrl = CameraController::new(Ellipsoid::WGS84);
        let mut camera = create_test_camera();
        let initial = camera.position.length();
        // 手指张开：间距 100 → 200 px（distance_delta = +100）。
        ctrl.pinch_zoom(&mut camera, 100.0, 0.01);
        assert!(
            camera.position.length() < initial,
            "spreading fingers must zoom in"
        );
    }

    #[test]
    fn pinch_close_zooms_out() {
        let ctrl = CameraController::new(Ellipsoid::WGS84);
        let mut camera = create_test_camera();
        let initial = camera.position.length();
        // 手指捻合：间距 200 → 100 px（distance_delta = -100）。
        ctrl.pinch_zoom(&mut camera, -100.0, 0.01);
        assert!(
            camera.position.length() > initial,
            "closing fingers must zoom out"
        );
    }

    #[test]
    fn pinch_rotate_maps_angle_to_spin_preserving_distance() {
        let ctrl = CameraController::new(Ellipsoid::WGS84);
        let mut camera = create_test_camera();
        let initial = camera.position.length();
        let pos0 = camera.position;
        // 逆时针双指旋转 +0.2 rad → 航向 spin。
        ctrl.pinch_rotate(&mut camera, 0.2);
        // Spin 绕中心旋转：距离保持，位置移动。
        assert!((camera.position.length() - initial).abs() / initial < 1e-9);
        assert!((camera.position - pos0).length() > 1.0);
        assert!((camera.direction.length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn pinch_rotate_sign_follows_angle_direction() {
        let ctrl = CameraController::new(Ellipsoid::WGS84);
        // 逆时针（+）与顺时针（−）旋转必须使相机反向移动。
        let mut ccw = create_test_camera();
        let mut cw = create_test_camera();
        let pos0 = ccw.position;
        ctrl.pinch_rotate(&mut ccw, 0.2);
        ctrl.pinch_rotate(&mut cw, -0.2);
        let d_ccw = ccw.position - pos0;
        let d_cw = cw.position - pos0;
        // 相反的旋转方向 → 位移指向相反方向。
        assert!(
            d_ccw.dot(d_cw) < 0.0,
            "CCW and CW pinch-rotate must spin oppositely"
        );
    }

    #[test]
    fn pinch_drag_translates_opposite_directions() {
        let ctrl = CameraController::new(Ellipsoid::WGS84);
        // +x 与 −x 的中点拖拽必须使相机反向平移。
        let mut right = create_test_camera();
        let mut left = create_test_camera();
        let pos0 = right.position;
        ctrl.pinch_translate(&mut right, DVec2::new(100.0, 0.0), 0.001);
        ctrl.pinch_translate(&mut left, DVec2::new(-100.0, 0.0), 0.001);
        let d_right = right.position - pos0;
        let d_left = left.position - pos0;
        assert!(d_right.length() > 0.0, "drag must translate the camera");
        assert!(
            d_right.dot(d_left) < 0.0,
            "opposite drags must translate oppositely"
        );

        // 对垂直轴做同样的检查。
        let mut up = create_test_camera();
        let mut down = create_test_camera();
        let pos1 = up.position;
        ctrl.pinch_translate(&mut up, DVec2::new(0.0, 100.0), 0.001);
        ctrl.pinch_translate(&mut down, DVec2::new(0.0, -100.0), 0.001);
        assert!((up.position - pos1).dot(down.position - pos1) < 0.0);
    }

    #[test]
    fn pinch_zoom_disabled_honors_flag() {
        let mut ctrl = CameraController::new(Ellipsoid::WGS84);
        ctrl.config.enable_zoom = false;
        let mut camera = create_test_camera();
        let pos0 = camera.position;
        ctrl.pinch_zoom(&mut camera, 200.0, 0.01);
        assert_eq!(camera.position, pos0);
    }

    #[test]
    fn pinch_rotate_disabled_honors_flag() {
        let mut ctrl = CameraController::new(Ellipsoid::WGS84);
        ctrl.config.enable_rotation = false;
        let mut camera = create_test_camera();
        let pos0 = camera.position;
        ctrl.pinch_rotate(&mut camera, 0.5);
        assert_eq!(camera.position, pos0);
    }

    #[test]
    fn pinch_translate_disabled_honors_flag() {
        let mut ctrl = CameraController::new(Ellipsoid::WGS84);
        ctrl.config.enable_pan = false;
        let mut camera = create_test_camera();
        let pos0 = camera.position;
        ctrl.pinch_translate(&mut camera, DVec2::new(120.0, 80.0), 0.001);
        assert_eq!(camera.position, pos0);
    }

    /// 一个三帧捻开序列：每帧将手指多张开一些，
    /// 因此每帧的 `distance_delta` 为正，相机单调地持续放大。
    #[test]
    fn multi_frame_pinch_open_zooms_in_monotonically() {
        let ctrl = CameraController::new(Ellipsoid::WGS84);
        let mut agg = CameraEventAggregator::new();
        let mut camera = create_test_camera();
        agg.pinch_start(DVec2::new(-50.0, 0.0), DVec2::new(50.0, 0.0));

        let mut prev_len = camera.position.length();
        // 帧 k 绕固定中点将间距从 w_k 开到 w_{k+1}。
        let widths = [50.0, 80.0, 120.0, 170.0];
        for k in 0..3 {
            let (a, b) = (widths[k], widths[k + 1]);
            apply_pinch_frame(
                &mut agg,
                &ctrl,
                &mut camera,
                k as f64 / 60.0,
                (DVec2::new(-a, 0.0), DVec2::new(a, 0.0)),
                (DVec2::new(-b, 0.0), DVec2::new(b, 0.0)),
                (0.002, 0.0),
            );
            let len = camera.position.length();
            assert!(
                len < prev_len,
                "frame {k}: pinch-open must keep zooming in ({len} !< {prev_len})"
            );
            prev_len = len;
        }
    }

    /// 一个跨两帧的组合手势：手指张开（缩放）、逆时针旋转
    /// （spin），并同时向右漂移（平移）。三种相机效果
    /// 都必须存在。
    #[test]
    fn combined_pinch_applies_zoom_spin_and_translate() {
        let ctrl = CameraController::new(Ellipsoid::WGS84);
        let mut agg = CameraEventAggregator::new();
        let mut camera = create_test_camera();
        let pos0 = camera.position;
        let len0 = pos0.length();

        agg.pinch_start(DVec2::new(-50.0, 0.0), DVec2::new(50.0, 0.0));
        // 帧 1：张开 50→90，将手指连线略微逆时针旋转，中点
        // 向右漂 40 px。
        apply_pinch_frame(
            &mut agg,
            &ctrl,
            &mut camera,
            0.0,
            (DVec2::new(-50.0, 0.0), DVec2::new(50.0, 0.0)),
            (DVec2::new(-45.0, 20.0), DVec2::new(85.0, 20.0)),
            (0.002, 0.001),
        );

        // 缩放分量：到中心的距离变小。
        assert!(camera.position.length() < len0, "combined gesture must zoom in");
        // spin + 平移分量：位置偏离了纯缩放射线。
        let radial = camera.position.normalize();
        let tangential = camera.position - pos0;
        // 非纯径向 → spin/平移有贡献。
        let perp = tangential - radial * tangential.dot(radial);
        assert!(perp.length() > 1.0, "combined gesture must spin/translate");
    }

    /// 双指旋转释放后，旋转速度被捕获
    /// 到 [`InertiaController`] 中并滑行直至停下（惯性交接）。
    #[test]
    fn pinch_rotate_release_hands_off_to_spin_inertia() {
        use crate::inertia::{InertiaController, InertiaSample, InertiaState};

        let ctrl = CameraController::new(Ellipsoid::WGS84);
        let mut agg = CameraEventAggregator::new();
        let mut camera = create_test_camera();
        agg.pinch_start(DVec2::new(-50.0, 0.0), DVec2::new(50.0, 0.0));

        // 一个旋转帧：手指连线逆时针转动；捕获角度增量。
        pinch_frame(
            &mut agg,
            0.0,
            DVec2::new(-50.0, 0.0),
            DVec2::new(50.0, 0.0),
            DVec2::new(-40.0, 30.0),
            DVec2::new(40.0, -30.0),
        );
        let angle_delta = agg.pinch_angle_delta();
        assert!(angle_delta.abs() > 1e-6, "gesture must produce a rotation");
        ctrl.pinch_rotate(&mut camera, angle_delta);

        // 释放时适配器在边界处捕获旋转速度（弧度 → 像素；
        // 这里用代表性的像素运动）到惯性中。
        let mut inertia = InertiaController::new();
        inertia.capture(InertiaState::Spin, DVec2::ZERO, DVec2::new(angle_delta * 2e4, 0.0));
        inertia.activate(Some(InertiaState::Spin));

        // 滑行持续 spin 并逐帧衰减。
        let scale = 1e-4;
        let s1 = InertiaSample::new(0.9, 0.0, 0.0, 16.0);
        let p0 = camera.position;
        assert!(ctrl.coast_inertia(&mut camera, &mut inertia, InertiaState::Spin, DVec3::ZERO, &s1, scale));
        let step1 = (camera.position - p0).length();
        assert!(step1 > 0.0, "inertia must keep the globe spinning after release");

        let s2 = InertiaSample::new(0.9, 0.0, 0.0, 600.0);
        let p1 = camera.position;
        assert!(ctrl.coast_inertia(&mut camera, &mut inertia, InertiaState::Spin, DVec3::ZERO, &s2, scale));
        let step2 = (camera.position - p1).length();
        assert!(step2 < step1, "spin inertia must decay: {step2} !< {step1}");
    }
}
