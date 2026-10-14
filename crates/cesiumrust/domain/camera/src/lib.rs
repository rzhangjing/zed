//! cesium-camera：相机状态、视图矩阵、移动操作。
//! 领域层 - 纯 Rust，f64 精度。
//!
//! 描述相机的位置、朝向（direction/up/right）与视锥体，并提供移动、旋转、拾取等操作。

use cesium_geospatial::{
    math_utils, BoundingSphere, Cartographic, CullingVolume, Ellipsoid, HeadingPitchRange,
    HeadingPitchRoll, OrthographicFrustum, PerspectiveFrustum, Rectangle,
};
use glam::{DMat4, DVec3};
use serde::{Deserialize, Serialize};

/// 场景渲染模式。
/// 决定相机在 2D/3D/ColumbusView/Morphing 各投影模式下的行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SceneMode {
    /// 2D 地图投影（俯视）。
    Scene2D,
    /// 3D 地球视图。
    #[default]
    Scene3D,
    /// 2.5D Columbus 视图（带 3D 对象的平面地图）。
    ColumbusView,
    /// 在模式之间过渡。
    Morphing,
}

/// 用于相机飞行的缓动函数。
/// 提供多种插值曲线，控制飞行起止的加减速手感。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EasingFunction {
    /// 线性插值。
    Linear,
    /// 正弦缓入缓出。
    SinusoidalInOut,
    /// 二次缓入。
    QuadraticIn,
    /// 二次缓出。
    QuadraticOut,
    /// 二次缓入缓出。
    QuadraticInOut,
    /// 三次缓入缓出。
    CubicInOut,
    /// 指数缓入缓出。
    ExponentialInOut,
    /// 五次缓入缓出。
    ///
    /// 用于短暂的相机飞行（`< 1e6` 米），此处期望更平缓的启动/停止。
    /// 五次多项式曲线在两端更平坦、中段更陡，因而启停更柔和。
    QuinticInOut,
}

impl EasingFunction {
    /// 在时间 t（0..1）处求值缓动函数。
    pub fn evaluate(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::SinusoidalInOut => 0.5 * (1.0 - (std::f64::consts::PI * t).cos()),
            Self::QuadraticIn => t * t,
            Self::QuadraticOut => t * (2.0 - t),
            Self::QuadraticInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    -1.0 + (4.0 - 2.0 * t) * t
                }
            }
            Self::CubicInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Self::ExponentialInOut => {
                if t < 0.5 {
                    (2.0_f64).powf(20.0 * t - 10.0) / 2.0
                } else {
                    (2.0 - (2.0_f64).powf(-20.0 * t + 10.0)) / 2.0
                }
            }
            Self::QuinticInOut => {
                if t < 0.5 {
                    16.0 * t * t * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(5) / 2.0
                }
            }
        }
    }
}

/// 相机视锥体类型。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Frustum {
    Perspective(PerspectiveFrustum),
    Orthographic(OrthographicFrustum),
}

impl Default for Frustum {
    /// 默认使用 60° 视场、16:9 宽高比、近平面 1、远平面 5 亿的透视视锥体。
    fn default() -> Self {
        Self::Perspective(PerspectiveFrustum::new(
            math_utils::to_radians(60.0),
            16.0 / 9.0,
            1.0,
            500_000_000.0,
        ))
    }
}

impl Frustum {
    /// 计算投影矩阵。
    pub fn projection_matrix(&self) -> DMat4 {
        match self {
            Self::Perspective(f) => f.projection_matrix(),
            Self::Orthographic(f) => f.projection_matrix(),
        }
    }

    /// 在给定位置/朝向处计算剔除体积。
    pub fn compute_culling_volume(
        &self,
        position: DVec3,
        direction: DVec3,
        up: DVec3,
    ) -> CullingVolume {
        match self {
            Self::Perspective(f) => f.compute_culling_volume(position, direction, up),
            Self::Orthographic(f) => f.compute_culling_volume(position, direction, up),
        }
    }

    /// 获取用于屏幕空间误差计算的 SSE 分母。
    pub fn sse_denominator(&self) -> f64 {
        match self {
            Self::Perspective(f) => f.sse_denominator(),
            Self::Orthographic(f) => 2.0 / f.height(),
        }
    }
}

/// 相机状态：位置 + 朝向（direction/up/right）+ 视锥体。
/// 仅承载核心视角状态，不依赖任何场景对象。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    /// 相机在世界坐标中的位置。
    pub position: DVec3,
    /// 相机的视线方向（单位向量）。
    pub direction: DVec3,
    /// 相机的上方方向（单位向量）。
    pub up: DVec3,
    /// 相机的右方方向（单位向量）。
    pub right: DVec3,
    /// 观察视锥体。
    pub frustum: Frustum,
    /// 默认移动量（米）。
    pub default_move_amount: f64,
    /// 默认查看/旋转量（弧度）。
    pub default_look_amount: f64,
    /// 默认旋转量（弧度）。
    pub default_rotate_amount: f64,
    /// 默认缩放量（米）。
    pub default_zoom_amount: f64,
    /// 场景模式（2D/3D/Columbus View/Morphing）。
    pub mode: SceneMode,
    /// 参考框架变换（单位矩阵 = 世界坐标）。
    pub transform: DMat4,
    /// 若设置，相机无法旋转越过此轴。
    pub constrained_axis: Option<DVec3>,
    /// `changed` 触发前相机需变化的量（0..1）。
    pub percentage_changed: f64,
    /// 2D 模式的最大缩放因子。
    pub maximum_zoom_factor: f64,
}

impl Camera {
    /// 在给定位置创建一台相机，朝向给定方向。
    pub fn new(position: DVec3, direction: DVec3, up: DVec3) -> Self {
        let direction = direction.normalize();
        let up = up.normalize();
        let right = direction.cross(up).normalize();
        let up = right.cross(direction).normalize();

        Self {
            position,
            direction,
            up,
            right,
            frustum: Frustum::default(),
            default_move_amount: 100_000.0,
            default_look_amount: std::f64::consts::PI / 60.0,
            default_rotate_amount: std::f64::consts::PI / 3600.0,
            default_zoom_amount: 100_000.0,
            mode: SceneMode::Scene3D,
            transform: DMat4::IDENTITY,
            constrained_axis: None,
            percentage_changed: 0.5,
            maximum_zoom_factor: 1.5,
        }
    }

    /// 创建一台默认相机，从原点沿 -Z 轴向下看。
    pub fn default_camera() -> Self {
        Self::new(DVec3::ZERO, -DVec3::Z, DVec3::Y)
    }

    // ========================================================================
    // 矩阵计算
    // ========================================================================

    /// 计算视图矩阵。
    /// 由位置与 direction/up/right 正交基构造世界到相机的变换。
    pub fn view_matrix(&self) -> DMat4 {
        compute_view_matrix(self.position, self.direction, self.up, self.right)
    }

    /// 计算逆视图矩阵。
    pub fn inverse_view_matrix(&self) -> DMat4 {
        self.view_matrix().inverse()
    }

    /// 计算投影矩阵。
    pub fn projection_matrix(&self) -> DMat4 {
        self.frustum.projection_matrix()
    }

    /// 计算视图-投影矩阵。
    pub fn view_projection_matrix(&self) -> DMat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// 为当前相机状态计算剔除体积。
    pub fn culling_volume(&self) -> CullingVolume {
        self.frustum
            .compute_culling_volume(self.position, self.direction, self.up)
    }

    // ========================================================================
    // 位置查询
    // ========================================================================

    /// 获取测地位置（经度、纬度、高度）。
    pub fn position_cartographic(&self, ellipsoid: &Ellipsoid) -> Option<Cartographic> {
        ellipsoid.cartesian_to_cartographic(self.position)
    }

    /// 获取椭球表面上方的高度。
    pub fn height(&self, ellipsoid: &Ellipsoid) -> Option<f64> {
        self.position_cartographic(ellipsoid).map(|c| c.height)
    }

    /// 获取相机位置的模长（距中心的距离）。
    pub fn position_magnitude(&self) -> f64 {
        self.position.length()
    }

    // ========================================================================
    // 朝向查询
    // ========================================================================

    /// 获取航向角（弧度）（针对 CV/2D 模式简化）。
    /// 由 direction 与 up 在水平面的投影推算。
    pub fn heading(&self) -> f64 {
        get_heading(self.direction, self.up)
    }

    /// 使用相机位置处的 ENU 框架获取 3D 模式下的航向。
    /// 将 direction/up 变换到当地东-北-天框架后推算水平角。
    pub fn heading_3d(&self, ellipsoid: &Ellipsoid) -> f64 {
        get_heading_3d(self.position, self.direction, self.up, self.right, ellipsoid)
    }

    /// 获取俯仰角（弧度）（针对 CV/2D 模式简化）。
    /// 由 direction 的 z 分量与水平面的夹角推算。
    pub fn pitch(&self) -> f64 {
        get_pitch(self.direction)
    }

    /// 使用相机位置处的表面法线获取 3D 模式下的俯仰。
    /// 以当地椭球法线为基准，度量视线偏离水平面的角度。
    pub fn pitch_3d(&self, ellipsoid: &Ellipsoid) -> f64 {
        get_pitch_3d(self.position, self.direction, ellipsoid)
    }

    /// 获取翻滚角（弧度）（针对 CV/2D 模式简化）。
    /// 由 right 与 up 的 z 分量经 atan2 推算绕视线的旋转。
    pub fn roll(&self) -> f64 {
        get_roll(self.direction, self.up, self.right)
    }

    /// 使用相机位置处的 ENU 框架获取 3D 模式下的翻滚。
    /// 将 right/up 变换到当地框架后，度量绕视线的倾斜。
    pub fn roll_3d(&self, ellipsoid: &Ellipsoid) -> f64 {
        get_roll_3d(self.position, self.direction, self.up, self.right, ellipsoid)
    }

    /// 以结构体形式获取航向、俯仰和翻滚。
    pub fn heading_pitch_roll(&self) -> HeadingPitchRoll {
        HeadingPitchRoll::new(self.heading(), self.pitch(), self.roll())
    }

    // ========================================================================
    // 移动操作
    // ========================================================================

    /// 沿给定方向移动相机给定距离。
    /// 位置沿归一化方向平移 amount，不改变朝向。
    pub fn move_along(&mut self, direction: DVec3, amount: f64) {
        self.position += direction.normalize() * amount;
    }

    /// 向前移动相机（沿视线方向）。
    /// 未指定 amount 时使用 default_move_amount。
    pub fn move_forward(&mut self, amount: Option<f64>) {
        let amount = amount.unwrap_or(self.default_move_amount);
        self.move_along(self.direction, amount);
    }

    /// 向后移动相机（与视线方向相反）。
    /// 以 -amount 沿视线方向移动。
    pub fn move_backward(&mut self, amount: Option<f64>) {
        let amount = amount.unwrap_or(self.default_move_amount);
        self.move_along(self.direction, -amount);
    }

    /// 向右移动相机。
    /// 沿 right 轴平移 amount。
    pub fn move_right(&mut self, amount: Option<f64>) {
        let amount = amount.unwrap_or(self.default_move_amount);
        self.move_along(self.right, amount);
    }

    /// 向左移动相机。
    /// 沿 -right 轴平移 amount。
    pub fn move_left(&mut self, amount: Option<f64>) {
        let amount = amount.unwrap_or(self.default_move_amount);
        self.move_along(self.right, -amount);
    }

    /// 向上移动相机。
    /// 沿 up 轴平移 amount。
    pub fn move_up(&mut self, amount: Option<f64>) {
        let amount = amount.unwrap_or(self.default_move_amount);
        self.move_along(self.up, amount);
    }

    /// 向下移动相机。
    /// 沿 -up 轴平移 amount。
    pub fn move_down(&mut self, amount: Option<f64>) {
        let amount = amount.unwrap_or(self.default_move_amount);
        self.move_along(self.up, -amount);
    }

    /// 绕某个轴旋转相机一个角度（轨道移动位置 + 旋转朝向）。
    /// 位置与三个朝向基向量一同作刚体旋转。
    pub fn rotate(&mut self, axis: DVec3, angle: f64) {
        let axis = axis.normalize();
        // 约定：对角度取负，即绕轴按 -angle 构造旋转四元数。
        let rotation = glam::DQuat::from_axis_angle(axis, -angle);
        self.position = rotation * self.position;
        self.direction = (rotation * self.direction).normalize();
        self.up = (rotation * self.up).normalize();
        self.right = self.direction.cross(self.up).normalize();
        self.up = self.right.cross(self.direction).normalize();
    }

    /// 向上旋转相机（绕 right 轴轨道移动）。
    /// 取负角度绕 right 轴旋转，使视线抬高。
    pub fn rotate_up(&mut self, angle: f64) {
        let axis = self.right;
        self.rotate(axis, -angle);
    }

    /// 向下旋转相机（绕 right 轴轨道移动）。
    /// 按正角度绕 right 轴旋转，使视线压低。
    pub fn rotate_down(&mut self, angle: f64) {
        let axis = self.right;
        self.rotate(axis, angle);
    }

    /// 向左旋转相机（绕 up 轴轨道移动）。
    /// 按正角度绕 up 轴旋转，相机向左环绕。
    pub fn rotate_left(&mut self, angle: f64) {
        let axis = self.up;
        self.rotate(axis, angle);
    }

    /// 向右旋转相机（绕 up 轴轨道移动，负角度）。
    /// 取负角度绕 up 轴旋转，相机向右环绕。
    pub fn rotate_right(&mut self, angle: f64) {
        let axis = self.up;
        self.rotate(axis, -angle);
    }

    /// 沿给定轴查看一个角度（仅旋转 direction 和 up，不改变位置）。
    /// 相当于原地转动镜头，与 rotate 的轨道移动相区分。
    pub fn look(&mut self, axis: DVec3, angle: f64) {
        // 约定：对角度取负，即绕轴按 -angle 构造旋转四元数。
        let rotation = glam::DQuat::from_axis_angle(axis.normalize(), -angle);
        self.direction = (rotation * self.direction).normalize();
        self.up = (rotation * self.up).normalize();
        self.right = self.direction.cross(self.up).normalize();
        self.up = self.right.cross(self.direction).normalize();
    }

    /// 按给定角度向左查看。
    /// 取负角度绕 up 轴原地转动镜头。
    pub fn look_left(&mut self, angle: Option<f64>) {
        let angle = angle.unwrap_or(self.default_look_amount);
        let axis = self.up;
        self.look(axis, -angle);
    }

    /// 按给定角度向右查看。
    /// 按正角度绕 up 轴原地转动镜头。
    pub fn look_right(&mut self, angle: Option<f64>) {
        let angle = angle.unwrap_or(self.default_look_amount);
        let axis = self.up;
        self.look(axis, angle);
    }

    /// 按给定角度向上查看。
    /// 取负角度绕 right 轴原地转动镜头。
    pub fn look_up(&mut self, angle: Option<f64>) {
        let angle = angle.unwrap_or(self.default_look_amount);
        let axis = self.right;
        self.look(axis, -angle);
    }

    /// 按给定角度向下查看。
    /// 按正角度绕 right 轴原地转动镜头。
    pub fn look_down(&mut self, angle: Option<f64>) {
        let angle = angle.unwrap_or(self.default_look_amount);
        let axis = self.right;
        self.look(axis, angle);
    }

    /// 向左扭转相机（逆时针翻滚）。
    /// 绕视线方向（direction 轴）旋转镜头。
    pub fn twist_left(&mut self, angle: f64) {
        let axis = self.direction;
        self.look(axis, angle);
    }

    /// 向右扭转相机（顺时针翻滚）。
    /// 取负角度绕 direction 轴旋转镜头。
    pub fn twist_right(&mut self, angle: f64) {
        let axis = self.direction;
        self.look(axis, -angle);
    }

    /// 设置相机从当前位置看向一个目标。
    /// 重算 direction/up/right 使其对准目标（简化实现）。
    pub fn look_at_point(&mut self, target: DVec3, up: DVec3) {
        self.direction = (target - self.position).normalize();
        self.right = self.direction.cross(up).normalize();
        self.up = self.right.cross(self.direction).normalize();
    }

    /// 通过向前移动放大。
    pub fn zoom_in(&mut self, amount: Option<f64>) {
        self.move_forward(amount);
    }

    /// 通过向后移动缩小。
    pub fn zoom_out(&mut self, amount: Option<f64>) {
        self.move_backward(amount);
    }

    // ========================================================================
    // 设置器
    // ========================================================================

    /// 在某个位置根据 heading/pitch/roll 设置相机的位置和朝向。
    /// 算法步骤：
    /// 1. 计算位置处的 ENU
    /// 2. 调整航向：heading -= PI/2（使得 heading=0 表示北）
    /// 3. 由调整后的 HPR 计算四元数
    /// 4. direction = rotMat 列 0，up = rotMat 列 2
    /// 5. 从 ENU 局部变换到世界
    pub fn set_view_hpr(
        &mut self,
        position: DVec3,
        heading: f64,
        pitch: f64,
        roll: f64,
        ellipsoid: &Ellipsoid,
    ) {
        self.position = position;

        // 计算位置处的 ENU 框架
        let enu = cesium_geospatial::transforms::east_north_up_to_fixed_frame(position, ellipsoid);
        let east = enu.x_axis.truncate();
        let north = enu.y_axis.truncate();
        let up_enu = enu.z_axis.truncate();

        // 调整航向：heading -= PI/2，使 heading=0 表示正北。
        let adjusted_heading = heading - std::f64::consts::FRAC_PI_2;
        let hpr_quat = HeadingPitchRoll::new(adjusted_heading, pitch, roll).to_quaternion();

        // 局部视线/天顶：direction = quat * X，up = quat * Z（对应旋转矩阵第 0/2 列）。
        //           四元数乘基向量等价于取对应列。
        let local_direction = hpr_quat * DVec3::X;
        let local_up = hpr_quat * DVec3::Z;

        // 从 ENU 局部变换到世界
        let enu_rotation = glam::DMat3::from_cols(east, north, up_enu);
        self.direction = (enu_rotation * local_direction).normalize();
        self.up = (enu_rotation * local_up).normalize();
        self.right = self.direction.cross(self.up).normalize();
        self.up = self.right.cross(self.direction).normalize();
    }

    /// 根据 direction/up 向量设置相机的位置和朝向。
    /// 直接以给定的 direction/up 重新正交化出朝向基。
    pub fn set_view_direction(
        &mut self,
        position: DVec3,
        direction: DVec3,
        up: DVec3,
    ) {
        self.position = position;
        self.direction = direction.normalize();
        self.up = up.normalize();
        self.right = self.direction.cross(self.up).normalize();
        self.up = self.right.cross(self.direction).normalize();
    }

    /// 设置相机以查看一个包围球。
    /// 按视锥体张角估算距离，把相机后退到能容纳整球的位置（简化）。
    pub fn view_bounding_sphere(
        &mut self,
        center: DVec3,
        radius: f64,
        offset: f64,
        ellipsoid: &Ellipsoid,
    ) {
        let distance = radius / (self.frustum.sse_denominator() * 0.5).max(0.001) + offset;
        let direction = (center - self.position).normalize();
        if direction.length_squared() < 1e-10 {
            return;
        }
        self.position = center - direction * distance;
        self.look_at_point(center, ellipsoid.geodetic_surface_normal(center).unwrap_or(DVec3::Z));
    }

    // ========================================================================
    // 变换与坐标转换
    // ========================================================================

    /// 设置参考框架变换。
    /// 先把当前世界状态转局部，再切换到新的参考框架。
    pub fn set_transform(&mut self, transform: DMat4) {
        // 保存世界空间状态
        let position_wc = self.position_wc();
        let direction_wc = self.direction_wc();
        let up_wc = self.up_wc();

        self.transform = transform;
        let inv = transform.inverse();

        // 转换到局部框架
        self.position = (inv * position_wc.extend(1.0)).truncate();
        self.direction = (inv * direction_wc.extend(0.0)).truncate().normalize();
        self.up = (inv * up_wc.extend(0.0)).truncate().normalize();
        self.right = self.direction.cross(self.up).normalize();
        self.up = self.right.cross(self.direction).normalize();
    }

    /// 获取世界坐标中的位置。
    pub fn position_wc(&self) -> DVec3 {
        (self.transform * self.position.extend(1.0)).truncate()
    }

    /// 获取世界坐标中的方向。
    pub fn direction_wc(&self) -> DVec3 {
        (self.transform * self.direction.extend(0.0)).truncate().normalize()
    }

    /// 获取世界坐标中的 up 向量。
    pub fn up_wc(&self) -> DVec3 {
        (self.transform * self.up.extend(0.0)).truncate().normalize()
    }

    /// 获取世界坐标中的 right 向量。
    pub fn right_wc(&self) -> DVec3 {
        (self.transform * self.right.extend(0.0)).truncate().normalize()
    }

    /// 将 Cartesian4 从世界坐标变换到相机参考框架。
    /// 用变换的逆把四维齐次坐标转到相机框架。
    pub fn world_to_camera_coordinates(&self, cartesian: glam::DVec4) -> glam::DVec4 {
        self.transform.inverse() * cartesian
    }

    /// 将一个点从世界坐标变换到相机参考框架。
    /// 以 w=1 变换点，保留平移分量。
    pub fn world_to_camera_point(&self, point: DVec3) -> DVec3 {
        (self.transform.inverse() * point.extend(1.0)).truncate()
    }

    /// 将一个向量从世界坐标变换到相机参考框架。
    /// 以 w=0 变换向量，忽略平移分量。
    pub fn world_to_camera_vector(&self, vector: DVec3) -> DVec3 {
        (self.transform.inverse() * vector.extend(0.0)).truncate()
    }

    /// 将 Cartesian4 从相机参考框架变换到世界坐标。
    /// 用变换把四维齐次坐标转回世界框架。
    pub fn camera_to_world_coordinates(&self, cartesian: glam::DVec4) -> glam::DVec4 {
        self.transform * cartesian
    }

    /// 将一个点从相机参考框架变换到世界坐标。
    /// 以 w=1 变换点回世界坐标。
    pub fn camera_to_world_point(&self, point: DVec3) -> DVec3 {
        (self.transform * point.extend(1.0)).truncate()
    }

    /// 将一个向量从相机参考框架变换到世界坐标。
    /// 以 w=0 变换向量回世界坐标。
    pub fn camera_to_world_vector(&self, vector: DVec3) -> DVec3 {
        (self.transform * vector.extend(0.0)).truncate()
    }

    // ========================================================================
    // setView / lookAt / lookAtTransform
    // ========================================================================

    /// 设置相机视图，包含目标位置和朝向。
    /// Morphing 模式下忽略；否则委托给 set_view_hpr。
    ///
    /// # 参数
    /// * `destination` - 目标位置（ECEF）或从矩形计算得出
    /// * `heading` - 航向（弧度，默认 0）
    /// * `pitch` - 俯仰（弧度，默认 -PI/2 = 向下看）
    /// * `roll` - 翻滚（弧度，默认 0）
    /// * `ellipsoid` - 椭球
    pub fn set_view(
        &mut self,
        destination: DVec3,
        heading: f64,
        pitch: f64,
        roll: f64,
        ellipsoid: &Ellipsoid,
    ) {
        if self.mode == SceneMode::Morphing {
            return;
        }
        self.set_view_hpr(destination, heading, pitch, roll, ellipsoid);
    }

    /// 设置相机以查看一个矩形。
    /// 计算查看给定矩形所需的相机位置，令其垂直俯视矩形中心。
    pub fn set_view_rectangle(
        &mut self,
        rectangle: &Rectangle,
        ellipsoid: &Ellipsoid,
    ) {
        let position = self.get_rectangle_camera_coordinates(rectangle, ellipsoid);
        self.position = position;
        self.direction = -position.normalize();
        self.right = self.direction.cross(DVec3::Z).normalize();
        if self.right.length_squared() < 1e-10 {
            self.right = DVec3::X;
        }
        self.up = self.right.cross(self.direction).normalize();
    }

    /// 设置相机以带 HeadingPitchRange 偏移看向目标。
    /// 以目标的 ENU 框架为参考，应用 HeadingPitchRange 偏移。
    pub fn look_at(&mut self, target: DVec3, offset: &HeadingPitchRange, ellipsoid: &Ellipsoid) {
        let transform = cesium_geospatial::transforms::east_north_up_to_fixed_frame(target, ellipsoid);
        self.look_at_transform(transform, offset);
    }

    /// 设置相机以带 Cartesian3 偏移看向目标。
    /// 以目标 ENU 框架为参考，直接用笛卡尔偏移定位相机。
    pub fn look_at_offset(&mut self, target: DVec3, offset: DVec3, ellipsoid: &Ellipsoid) {
        let transform = cesium_geospatial::transforms::east_north_up_to_fixed_frame(target, ellipsoid);
        self.transform = transform;
        self.position = offset;
        self.direction = -offset.normalize();
        let right = self.direction.cross(DVec3::Z);
        if right.length_squared() < 1e-10 {
            self.right = DVec3::X;
        } else {
            self.right = right.normalize();
        }
        self.up = self.right.cross(self.direction).normalize();
    }

    /// 设置相机变换并相对于新框架定位它。
    /// 切换参考框架后按 HeadingPitchRange 偏移定位相机。
    pub fn look_at_transform(&mut self, transform: DMat4, offset: &HeadingPitchRange) {
        self.set_transform(transform);

        // 将 HeadingPitchRange 转换为局部框架中的 Cartesian 偏移
        let cartesian_offset = offset_from_heading_pitch_range(
            offset.heading,
            offset.pitch,
            offset.range,
        );

        self.position = cartesian_offset;
        self.direction = -cartesian_offset.normalize();
        let right = self.direction.cross(DVec3::Z);
        if right.length_squared() < 1e-10 {
            self.right = DVec3::X;
        } else {
            self.right = right.normalize();
        }
        self.up = self.right.cross(self.direction).normalize();
    }

    /// 设置相机变换并以 Cartesian3 偏移定位它。
    /// 切换参考框架后直接用笛卡尔偏移定位相机。
    pub fn look_at_transform_offset(&mut self, transform: DMat4, offset: DVec3) {
        self.set_transform(transform);
        self.position = offset;
        self.direction = -offset.normalize();
        let right = self.direction.cross(DVec3::Z);
        if right.length_squared() < 1e-10 {
            self.right = DVec3::X;
        } else {
            self.right = right.normalize();
        }
        self.up = self.right.cross(self.direction).normalize();
    }

    /// 设置相机变换，保留当前的世界空间位置/朝向。
    /// 仅切换参考框架，保持相机局部状态不变。
    pub fn look_at_transform_no_offset(&mut self, transform: DMat4) {
        self.set_transform(transform);
    }

    // ========================================================================
    // 矩形视图
    // ========================================================================

    /// 计算查看一个矩形所需的相机位置。
    /// 由矩形中心与角范围估算俯视整块区域所需的高度。
    pub fn get_rectangle_camera_coordinates(
        &self,
        rectangle: &Rectangle,
        ellipsoid: &Ellipsoid,
    ) -> DVec3 {
        // 计算矩形的中心（处理 IDL 跨越）
        let center = rectangle.center();
        let center_ecef = ellipsoid.cartographic_to_cartesian(&center);

        // 计算角范围
        let delta_lon = (rectangle.east - rectangle.west).abs();
        let delta_lat = (rectangle.north - rectangle.south).abs();
        let max_delta = delta_lon.max(delta_lat);

        // 计算查看矩形所需的距离
        let fov = match &self.frustum {
            Frustum::Perspective(f) => f.fov,
            Frustum::Orthographic(_) => std::f64::consts::FRAC_PI_3,
        };
        let half_angle = fov * 0.5;
        let arc_length = max_delta * ellipsoid.maximum_radius();
        let distance = (arc_length * 0.5) / half_angle.tan().max(0.001);

        // 将相机定位在中心上方
        let normal = center_ecef.normalize();
        center_ecef + normal * distance.max(ellipsoid.maximum_radius() * 0.1)
    }

    // ========================================================================
    // 包围球工具
    // ========================================================================

    /// 计算相机到包围球的距离。
    /// 沿视线方向的有符号距离减去球半径（不低于 0）。
    pub fn distance_to_bounding_sphere(&self, sphere: &BoundingSphere) -> f64 {
        // 有符号距离：球心相对相机在视线上的投影减去半径。
        let to_center = sphere.center - self.position;
        let distance = to_center.dot(self.direction) - sphere.radius;
        distance.max(0.0)
    }

    /// 根据模式获取相机位置的模长。
    /// 3D 取位置模长，CV 取 |z|，2D 简化返回 1。
    pub fn get_magnitude(&self) -> f64 {
        match self.mode {
            SceneMode::Scene3D => self.position.length(),
            SceneMode::ColumbusView => self.position.z.abs(),
            SceneMode::Scene2D => 1.0, // 简化：将使用视锥体范围
            SceneMode::Morphing => self.position.length(),
        }
    }

    /// 获取参考框架变换的逆。
    /// 返回当前参考框架变换矩阵的逆。
    pub fn inverse_transform(&self) -> DMat4 {
        self.transform.inverse()
    }

    // ========================================================================
    // 拾取
    // ========================================================================

    /// 使用透视视锥体从窗口位置创建拾取射线。
    /// 透视分支：按 NDC 与视锥张角构造汇聚射线。
    ///
    /// # 参数
    /// * `window_x` - 窗口 X 坐标（像素，左=0）
    /// * `window_y` - 窗口 Y 坐标（像素，顶=0）
    /// * `canvas_width` - 画布宽度（像素）
    /// * `canvas_height` - 画布高度（像素）
    pub fn get_pick_ray_perspective(
        &self,
        window_x: f64,
        window_y: f64,
        canvas_width: f64,
        canvas_height: f64,
    ) -> Option<cesium_geospatial::Ray> {
        let (fovy, aspect_ratio, near) = match &self.frustum {
            Frustum::Perspective(f) => (f.fovy(), f.aspect_ratio, f.near),
            Frustum::Orthographic(_) => return None, // 使用正交版本
        };

        let tan_phi = (fovy * 0.5).tan();
        let tan_theta = aspect_ratio * tan_phi;

        // NDC 坐标
        let x = (2.0 / canvas_width) * window_x - 1.0;
        let y = (2.0 / canvas_height) * (canvas_height - window_y) - 1.0;

        let position = self.position_wc();
        let direction_wc = self.direction_wc();
        let right_wc = self.right_wc();
        let up_wc = self.up_wc();

        // direction = normalize(dir*near + right*(x*near*tanTheta) + up*(y*near*tanPhi))
        let dir = direction_wc * near
            + right_wc * (x * near * tan_theta)
            + up_wc * (y * near * tan_phi);

        Some(cesium_geospatial::Ray::new(position, dir.normalize()))
    }

    /// 拾取窗口位置处的椭球表面。
    /// 3D 分支：求拾取射线与椭球的首个交点。
    ///
    /// 返回椭球上的交点，若不可见则返回 None。
    pub fn pick_ellipsoid(
        &self,
        window_x: f64,
        window_y: f64,
        canvas_width: f64,
        canvas_height: f64,
        ellipsoid: &Ellipsoid,
    ) -> Option<DVec3> {
        let ray = self.get_pick_ray_perspective(window_x, window_y, canvas_width, canvas_height)?;
        let intersection = cesium_geospatial::ray_ellipsoid(&ray, ellipsoid)?;
        let t = if intersection.0 > 0.0 { intersection.0 } else { intersection.1 };
        Some(ray.point_at(t))
    }

    /// 使用正交视锥体从窗口位置创建拾取射线。
    /// 正交分支：射线原点按视锥平面偏移，方向恒为视线。
    ///
    /// 对于正交投影，射线原点按窗口位置在视锥体平面内偏移，
    /// 且方向始终是相机方向。
    pub fn get_pick_ray_orthographic(
        &self,
        window_x: f64,
        window_y: f64,
        canvas_width: f64,
        canvas_height: f64,
    ) -> Option<cesium_geospatial::Ray> {
        let (frustum_width, frustum_height) = match &self.frustum {
            Frustum::Orthographic(f) => (f.width, f.height()),
            Frustum::Perspective(_) => return None,
        };

        // 按视锥体半范围缩放的 NDC 坐标
        let mut x = (2.0 / canvas_width) * window_x - 1.0;
        x *= frustum_width * 0.5;
        let mut y = (2.0 / canvas_height) * (canvas_height - window_y) - 1.0;
        y *= frustum_height * 0.5;

        let position = self.position_wc();
        let right_wc = self.right_wc();
        let up_wc = self.up_wc();
        let direction_wc = self.direction_wc();

        let origin = position + right_wc * x + up_wc * y;

        Some(cesium_geospatial::Ray::new(origin, direction_wc))
    }

    /// 从窗口位置创建拾取射线（分发到透视或正交）。
    /// 依视锥体类型分发到透视或正交实现。
    pub fn get_pick_ray(
        &self,
        window_x: f64,
        window_y: f64,
        canvas_width: f64,
        canvas_height: f64,
    ) -> Option<cesium_geospatial::Ray> {
        match &self.frustum {
            Frustum::Perspective(_) => self.get_pick_ray_perspective(window_x, window_y, canvas_width, canvas_height),
            Frustum::Orthographic(_) => self.get_pick_ray_orthographic(window_x, window_y, canvas_width, canvas_height),
        }
    }

    /// 计算包围球在其距相机距离处的像素大小。
    /// 由投影参数把球半径换算为屏幕像素尺寸。
    ///
    /// 返回到包围球距离处一个像素的最大像素尺寸（宽或高）。
    pub fn get_pixel_size(
        &self,
        sphere: &BoundingSphere,
        drawing_buffer_width: f64,
        drawing_buffer_height: f64,
        pixel_ratio: f64,
    ) -> f64 {
        let distance = self.distance_to_bounding_sphere(sphere);
        let (pixel_width, pixel_height) = match &self.frustum {
            Frustum::Perspective(f) => f.pixel_dimensions(drawing_buffer_width, drawing_buffer_height, distance, pixel_ratio),
            Frustum::Orthographic(f) => f.pixel_dimensions(drawing_buffer_width, drawing_buffer_height, distance, pixel_ratio),
        };
        pixel_width.max(pixel_height)
    }

    // ========================================================================
    // 约束旋转
    // ========================================================================

    /// 以强制约束轴的方式进行旋转。
    /// 若设置了 constrained_axis，则防止 up 向量越过它。
    /// 沿约束轴作轨道旋转，保持 up 不越过限制。
    pub fn rotate_constrained(&mut self, axis: DVec3, angle: f64) {
        self.rotate(axis, angle);

        if let Some(constrained) = self.constrained_axis {
            // 若 up 向量越过约束轴，则将其钳制
            let dot = self.up.dot(constrained);
            if dot < 0.0 {
                // 将 up 投影到垂直于约束轴的平面
                let projected = (self.up - constrained * dot).normalize();
                self.up = projected;
                // 重新推导 direction 使其垂直于钳制后的 up
                self.direction = (self.direction - self.up * self.direction.dot(self.up)).normalize();
                self.right = self.direction.cross(self.up).normalize();
            }
        }
    }

    /// 以强制约束轴的方式向上旋转。
    pub fn rotate_up_constrained(&mut self, angle: f64) {
        let axis = self.right;
        self.rotate_constrained(axis, -angle);
    }

    /// 以强制约束轴的方式向下旋转。
    pub fn rotate_down_constrained(&mut self, angle: f64) {
        let axis = self.right;
        self.rotate_constrained(axis, angle);
    }

    /// 以强制约束轴的方式向左旋转。
    pub fn rotate_left_constrained(&mut self, angle: f64) {
        let axis = self.up;
        self.rotate_constrained(axis, angle);
    }

    /// 以强制约束轴的方式向右旋转。
    pub fn rotate_right_constrained(&mut self, angle: f64) {
        let axis = self.up;
        self.rotate_constrained(axis, -angle);
    }

    // ========================================================================
    // 变化检测
    // ========================================================================

    /// 检查相机相对于参考状态是否发生了显著变化。
    /// 若变化超过阈值，返回变化百分比（0..1+）。
    /// 取方向变化与位置变化百分比中的较大值。
    pub fn compute_change_percentage(
        &self,
        reference_position: DVec3,
        reference_direction: DVec3,
    ) -> f64 {
        // 方向变化百分比
        let dir_angle = self.direction.dot(reference_direction).clamp(-1.0, 1.0).acos();
        let fov = match &self.frustum {
            Frustum::Perspective(f) => f.fov,
            Frustum::Orthographic(_) => 1.0,
        };
        let dir_percentage = if fov > 0.0 { dir_angle / (fov * 0.5) } else { dir_angle };

        // 位置变化百分比（相对于高度）
        let distance = (self.position - reference_position).length();
        let height = self.position.length().max(1.0);
        let height_percentage = distance / height;

        dir_percentage.max(height_percentage)
    }

    /// 检查相机是否变化超过 percentage_changed 阈值。
    pub fn has_changed(
        &self,
        reference_position: DVec3,
        reference_direction: DVec3,
    ) -> bool {
        self.compute_change_percentage(reference_position, reference_direction)
            > self.percentage_changed
    }

    // ========================================================================
    // 飞行回家
    // ========================================================================

    /// 返回用于查看默认矩形的默认“家”相机位置。
    /// 根据默认矩形中心推算俯视高度与位置。
    pub fn default_home_position(ellipsoid: &Ellipsoid) -> DVec3 {
        // 默认视图矩形：大致为北美
        let default_rect = Rectangle::new(
            math_utils::to_radians(-95.0),
            math_utils::to_radians(-20.0),
            math_utils::to_radians(-70.0),
            math_utils::to_radians(90.0),
        );
        let center_lon = (default_rect.west + default_rect.east) * 0.5;
        let center_lat = (default_rect.south + default_rect.north) * 0.5;
        let center = Cartographic::from_radians(center_lon, center_lat, 0.0);
        let center_ecef = ellipsoid.cartographic_to_cartesian(&center);

        // 位于中心上方约 2.5 倍地球半径处
        let normal = center_ecef.normalize();
        normal * ellipsoid.maximum_radius() * 2.5
    }
}

impl Default for Camera {
    /// 默认相机：位于原点、沿 -Z 轴向下看。
    fn default() -> Self {
        Self::default_camera()
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 从相机向量计算视图矩阵。
/// 用 right/up/-direction 作行向量拼接，最后一列为平移。
fn compute_view_matrix(position: DVec3, direction: DVec3, up: DVec3, right: DVec3) -> DMat4 {
    DMat4::from_cols_array(&[
        right.x, up.x, -direction.x, 0.0,
        right.y, up.y, -direction.y, 0.0,
        right.z, up.z, -direction.z, 0.0,
        -right.dot(position), -up.dot(position), direction.dot(position), 1.0,
    ])
}

/// 从 direction 和 up 向量计算航向。
/// 由 direction 的水平投影角推算，方向接近垂直时改用 up。
fn get_heading(direction: DVec3, up: DVec3) -> f64 {
    let heading = if (direction.z.abs() - 1.0).abs() > math_utils::EPSILON3 {
        direction.y.atan2(direction.x) - std::f64::consts::FRAC_PI_2
    } else {
        up.y.atan2(up.x) - std::f64::consts::FRAC_PI_2
    };
    let result = math_utils::TWO_PI - math_utils::zero_to_two_pi(heading);
    // 归一化：TWO_PI ≡ 0（航向范围为 [0, TWO_PI)）
    if (result - math_utils::TWO_PI).abs() < 1e-15 {
        0.0
    } else {
        result
    }
}

/// 从 direction 向量计算俯仰。
/// 由 direction 与 z 轴夹角推算偏离水平面的角度。
fn get_pitch(direction: DVec3) -> f64 {
    std::f64::consts::FRAC_PI_2 - direction.z.clamp(-1.0, 1.0).acos()
}

/// 从 direction、up 和 right 向量计算翻滚。
/// 由 right/up 的 z 分量经 atan2 推算绕视线的旋转。
fn get_roll(direction: DVec3, up: DVec3, right: DVec3) -> f64 {
    if (direction.z.abs() - 1.0).abs() > math_utils::EPSILON3 {
        let roll = (-right.z).atan2(up.z);
        math_utils::zero_to_two_pi(roll + math_utils::TWO_PI)
    } else {
        0.0
    }
}

/// 将 HeadingPitchRange 偏移转换为局部 ENU 框架中的 Cartesian3 偏移。
///
/// 算法步骤：
/// 1. 将 pitch 钳制到 [-PI/2, PI/2]
/// 2. heading = zeroToTwoPi(heading) - PI/2
/// 3. pitchQuat = fromAxisAngle(Y, -pitch)
/// 4. headingQuat = fromAxisAngle(Z, -heading)
/// 5. rotQuat = headingQuat * pitchQuat
/// 6. offset = -(rotMatrix * UNIT_X) * range
///
/// 等价的闭式解：直接以四元数乘积展开，避免构造完整旋转矩阵。
fn offset_from_heading_pitch_range(heading: f64, pitch: f64, range: f64) -> DVec3 {
    let pitch = pitch.clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
    // 航向调整：归一化到 [0, 2π) 后减 π/2，使 heading=0 表示正北
    let heading = math_utils::zero_to_two_pi(heading) - std::f64::consts::FRAC_PI_2;
    // pitchQuat = fromAxisAngle(Y, -pitch), headingQuat = fromAxisAngle(Z, -heading)
    // rotQuat = headingQuat * pitchQuat
    let sp = (-pitch / 2.0).sin();
    let cp = (-pitch / 2.0).cos();
    let sh = (-heading / 2.0).sin();
    let ch = (-heading / 2.0).cos();
    // rotQuat = (ch*0 - sh*sp, ch*sp + sh*0, ch*0 - sh*0... )
    let qx = -sh * sp;
    let qy = ch * sp;
    let qz = sh * cp;
    let qw = ch * cp;
    // rotMatrix 列 0（direction = rotMat * UNIT_X）
    let m00 = qw * qw + qx * qx - qy * qy - qz * qz;
    let m10 = 2.0 * (qx * qy + qw * qz);
    let m20 = 2.0 * (qx * qz - qw * qy);
    // offset = -(rotMat * UNIT_X) * range
    DVec3::new(-m00 * range, -m10 * range, -m20 * range)
}

/// 使用相机位置处的 ENU 框架计算 3D 模式下的航向。
/// SCENE3D 分支：
/// 将 direction/up 变换到 ENU 局部框架，然后：
/// heading = TWO_PI - zeroToTwoPi(atan2(dir_local.y, dir_local.x) - PI/2)
/// 若 |dir_local.z| ≈ 1（垂直向上/向下看），则改用 up_local。
fn get_heading_3d(position: DVec3, direction: DVec3, up: DVec3, _right: DVec3, ellipsoid: &Ellipsoid) -> f64 {
    let enu = cesium_geospatial::transforms::east_north_up_to_fixed_frame(position, ellipsoid);
    let east = enu.x_axis.truncate();
    let north = enu.y_axis.truncate();
    let up_enu = enu.z_axis.truncate();

    // 将 direction 变换到 ENU 局部框架
    let dir_local = DVec3::new(direction.dot(east), direction.dot(north), direction.dot(up_enu));

    let heading = if (dir_local.z.abs() - 1.0).abs() <= math_utils::EPSILON3 {
        // 几乎垂直向上或向下看 - 使用 up 向量
        let up_local = DVec3::new(up.dot(east), up.dot(north), up.dot(up_enu));
        math_utils::TWO_PI - math_utils::zero_to_two_pi(up_local.y.atan2(up_local.x) - std::f64::consts::FRAC_PI_2)
    } else {
        math_utils::TWO_PI - math_utils::zero_to_two_pi(dir_local.y.atan2(dir_local.x) - std::f64::consts::FRAC_PI_2)
    };
    // 归一化：TWO_PI ≡ 0（航向范围为 [0, TWO_PI)）
    if (heading - math_utils::TWO_PI).abs() < 1e-15 {
        0.0
    } else {
        heading
    }
}

/// 使用相机位置处的 ENU 框架计算 3D 模式下的俯仰。
/// SCENE3D 分支：
/// pitch = PI/2 - acosClamped(dir_local.z)
fn get_pitch_3d(position: DVec3, direction: DVec3, ellipsoid: &Ellipsoid) -> f64 {
    let enu = cesium_geospatial::transforms::east_north_up_to_fixed_frame(position, ellipsoid);
    let up_enu = enu.z_axis.truncate();

    // dir_local.z = direction 点乘 ENU up 轴（= 测地表面法线）
    let dir_local_z = direction.dot(up_enu);
    std::f64::consts::FRAC_PI_2 - dir_local_z.clamp(-1.0, 1.0).acos()
}

/// 使用相机位置处的 ENU 框架计算 3D 模式下的翻滚。
/// SCENE3D 分支：
/// 若 |dir_local.z| < 1-EPSILON3：roll = zeroToTwoPi(atan2(-right_local.z, up_local.z) + TWO_PI)
/// 否则：roll = 0
// 遗留移植风格技术债（deferred.md #18）；在 M13 lint 清理或本文件在其里程碑被重写时重新审视
#[allow(clippy::let_and_return)]
fn get_roll_3d(position: DVec3, direction: DVec3, up: DVec3, right: DVec3, ellipsoid: &Ellipsoid) -> f64 {
    let enu = cesium_geospatial::transforms::east_north_up_to_fixed_frame(position, ellipsoid);
    let up_enu = enu.z_axis.truncate();

    let dir_local_z = direction.dot(up_enu);

    let roll = if (dir_local_z.abs() - 1.0).abs() > math_utils::EPSILON3 {
        let right_local_z = right.dot(up_enu);
        let up_local_z = up.dot(up_enu);
        math_utils::zero_to_two_pi((-right_local_z).atan2(up_local_z) + math_utils::TWO_PI)
    } else {
        0.0
    };
    roll
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn test_default_camera() {
        let camera = Camera::default_camera();
        assert!(camera.position.abs_diff_eq(DVec3::ZERO, 1e-10));
        assert!(camera.direction.abs_diff_eq(-DVec3::Z, 1e-10));
        assert!(camera.up.abs_diff_eq(DVec3::Y, 1e-10));
        assert!(camera.right.abs_diff_eq(DVec3::X, 1e-10));
    }

    #[test]
    fn test_view_matrix() {
        let camera = Camera::default_camera();
        let view = camera.view_matrix();

        // 对于位于原点、沿 -Z 向下看的相机：
        // 视图矩阵应为单位矩阵（因为我们已经在原点、沿 -Z 向下看）
        // 实际上：right=X，up=Y，-direction=Z
        // 所以旋转部分为单位矩阵，平移为零
        assert!((view.x_axis.x - 1.0).abs() < 1e-10);
        assert!((view.y_axis.y - 1.0).abs() < 1e-10);
        assert!((view.z_axis.z - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_move_forward() {
        let mut camera = Camera::default_camera();
        camera.move_forward(Some(10.0));
        assert!(camera.position.abs_diff_eq(DVec3::new(0.0, 0.0, -10.0), 1e-10));
    }

    #[test]
    fn test_move_right() {
        let mut camera = Camera::default_camera();
        camera.move_right(Some(5.0));
        assert!(camera.position.abs_diff_eq(DVec3::new(5.0, 0.0, 0.0), 1e-10));
    }

    #[test]
    fn test_rotate_horizontal() {
        let mut camera = Camera::default_camera();
        camera.rotate_right(PI / 2.0); // 90 度
        // 以负角度绕 up（Y）旋转 90° 后：
        // direction（-Z）绕 Y 旋转 -PI/2 -> -X
        assert!(
            camera.direction.abs_diff_eq(DVec3::new(-1.0, 0.0, 0.0), 1e-10),
            "direction: {:?}",
            camera.direction
        );
    }

    #[test]
    fn test_look_at_point() {
        let mut camera = Camera::new(
            DVec3::new(0.0, 0.0, 10.0),
            -DVec3::Z,
            DVec3::Y,
        );
        camera.look_at_point(DVec3::ZERO, DVec3::Y);
        assert!(camera.direction.abs_diff_eq(DVec3::new(0.0, 0.0, -1.0), 1e-10));
    }

    #[test]
    fn test_heading_pitch_at_equator() {
        // 位于赤道、水平朝北看的相机
        let ellipsoid = Ellipsoid::WGS84;
        let position = DVec3::new(6378137.0, 0.0, 0.0); // 位于本初子午线上的赤道
        let mut camera = Camera::default_camera();
        camera.set_view_hpr(position, 0.0, 0.0, 0.0, &ellipsoid);

        // 使用 3D getter，航向应为 0（朝北看）
        let heading = camera.heading_3d(&ellipsoid);
        assert!(
            heading.abs() < 0.01 || (heading - math_utils::TWO_PI).abs() < 0.01,
            "heading: {}",
            heading
        );
    }

    #[test]
    fn test_culling_volume() {
        let camera = Camera::default_camera();
        let cv = camera.culling_volume();

        // 相机前方的球体应在内部
        let sphere = cesium_geospatial::BoundingSphere::new(DVec3::new(0.0, 0.0, -100.0), 1.0);
        assert_eq!(
            cv.visibility(&sphere),
            cesium_geospatial::Intersect::Inside
        );
    }

    #[test]
    fn test_orthonormalize_after_rotation() {
        let mut camera = Camera::default_camera();
        camera.rotate(DVec3::new(1.0, 1.0, 0.0).normalize(), 0.5);

        // 验证正交归一性
        assert!((camera.direction.length() - 1.0).abs() < 1e-10);
        assert!((camera.up.length() - 1.0).abs() < 1e-10);
        assert!((camera.right.length() - 1.0).abs() < 1e-10);
        assert!(camera.direction.dot(camera.up).abs() < 1e-10);
        assert!(camera.direction.dot(camera.right).abs() < 1e-10);
        assert!(camera.up.dot(camera.right).abs() < 1e-10);
    }

    // ========================================================================
    // P4：新增相机测试
    // ========================================================================

    #[test]
    fn test_scene_mode_default() {
        let camera = Camera::default_camera();
        assert_eq!(camera.mode, SceneMode::Scene3D);
    }

    #[test]
    fn test_easing_functions() {
        // Linear
        assert!((EasingFunction::Linear.evaluate(0.0)).abs() < 1e-10);
        assert!((EasingFunction::Linear.evaluate(0.5) - 0.5).abs() < 1e-10);
        assert!((EasingFunction::Linear.evaluate(1.0) - 1.0).abs() < 1e-10);

        // SinusoidalInOut
        assert!((EasingFunction::SinusoidalInOut.evaluate(0.0)).abs() < 1e-10);
        assert!((EasingFunction::SinusoidalInOut.evaluate(0.5) - 0.5).abs() < 1e-10);
        assert!((EasingFunction::SinusoidalInOut.evaluate(1.0) - 1.0).abs() < 1e-10);

        // QuadraticIn
        assert!((EasingFunction::QuadraticIn.evaluate(0.5) - 0.25).abs() < 1e-10);

        // QuadraticOut
        assert!((EasingFunction::QuadraticOut.evaluate(0.5) - 0.75).abs() < 1e-10);

        // QuinticInOut：端点固定，中点在 0.5 处对称。
        assert!((EasingFunction::QuinticInOut.evaluate(0.0)).abs() < 1e-10);
        assert!((EasingFunction::QuinticInOut.evaluate(0.5) - 0.5).abs() < 1e-10);
        assert!((EasingFunction::QuinticInOut.evaluate(1.0) - 1.0).abs() < 1e-10);
        // 缓入分支：16 * 0.25^5 = 0.015625。
        assert!((EasingFunction::QuinticInOut.evaluate(0.25) - 0.015625).abs() < 1e-10);
    }

    #[test]
    fn test_quintic_in_out_is_monotonic() {
        let easing = EasingFunction::QuinticInOut;
        let mut previous = -1.0;
        for i in 0..=100 {
            let value = easing.evaluate(i as f64 / 100.0);
            assert!(value >= previous - 1e-12, "quintic ease must not decrease");
            previous = value;
        }
    }

    #[test]
    fn test_position_wc_identity_transform() {
        let camera = Camera::new(
            DVec3::new(1000.0, 2000.0, 3000.0),
            -DVec3::Z,
            DVec3::Y,
        );
        // 使用单位变换时，position_wc == position
        assert!(camera.position_wc().abs_diff_eq(camera.position, 1e-10));
    }

    #[test]
    fn test_set_transform() {
        let mut camera = Camera::new(
            DVec3::new(100.0, 0.0, 0.0),
            -DVec3::X,
            DVec3::Z,
        );
        // 设置一个平移变换
        let transform = DMat4::from_translation(DVec3::new(50.0, 0.0, 0.0));
        camera.set_transform(transform);

        // 局部框架中的位置应在 x 上偏移 -50
        assert!((camera.position.x - 50.0).abs() < 1e-10);
        // 世界位置仍应为 100
        assert!((camera.position_wc().x - 100.0).abs() < 1e-10);
    }

    #[test]
    fn test_world_to_camera_roundtrip() {
        let mut camera = Camera::new(
            DVec3::new(100.0, 200.0, 300.0),
            -DVec3::Z,
            DVec3::Y,
        );
        camera.transform = DMat4::from_translation(DVec3::new(10.0, 20.0, 30.0));

        let world_point = DVec3::new(500.0, 600.0, 700.0);
        let local = camera.world_to_camera_point(world_point);
        let back = camera.camera_to_world_point(local);
        assert!(back.abs_diff_eq(world_point, 1e-6));
    }

    #[test]
    fn test_look_at_with_hpr() {
        let mut camera = Camera::default_camera();
        let target = DVec3::new(6378137.0, 0.0, 0.0);
        let offset = HeadingPitchRange::new(0.0, -PI / 4.0, 1000000.0);

        camera.look_at(target, &offset, &Ellipsoid::WGS84);

        // 相机应位于距目标 range 处
        let world_pos = camera.position_wc();
        let dist = (world_pos - target).length();
        assert!((dist - 1000000.0).abs() / 1000000.0 < 0.01);
    }

    #[test]
    fn test_look_at_offset() {
        let mut camera = Camera::default_camera();
        let target = DVec3::new(0.0, 0.0, 0.0);
        let offset = DVec3::new(1000.0, 0.0, 500.0);

        camera.look_at_offset(target, offset, &Ellipsoid::WGS84);

        // 方向应从偏移指向目标
        let expected_dir = -offset.normalize();
        assert!(camera.direction.abs_diff_eq(expected_dir, 1e-10));
    }

    #[test]
    fn test_get_rectangle_camera_coordinates() {
        let camera = Camera::default_camera();
        let rect = Rectangle::new(
            math_utils::to_radians(-10.0),
            math_utils::to_radians(-10.0),
            math_utils::to_radians(10.0),
            math_utils::to_radians(10.0),
        );

        let pos = camera.get_rectangle_camera_coordinates(&rect, &Ellipsoid::WGS84);

        // 位置应在表面上方
        let height = pos.length() - Ellipsoid::WGS84.maximum_radius();
        assert!(height > 0.0);
    }

    #[test]
    fn test_set_view_rectangle() {
        let mut camera = Camera::default_camera();
        let rect = Rectangle::new(
            math_utils::to_radians(-10.0),
            math_utils::to_radians(-10.0),
            math_utils::to_radians(10.0),
            math_utils::to_radians(10.0),
        );

        camera.set_view_rectangle(&rect, &Ellipsoid::WGS84);

        // 相机应在表面上方、向下看
        let height = camera.position.length() - Ellipsoid::WGS84.maximum_radius();
        assert!(height > 0.0);
        // 方向应有一个指向中心的分量
        assert!(camera.direction.dot(-camera.position.normalize()) > 0.5);
    }

    #[test]
    fn test_distance_to_bounding_sphere() {
        let camera = Camera::new(
            DVec3::new(0.0, 0.0, 1000.0),
            -DVec3::Z,
            DVec3::Y,
        );
        let sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, 0.0), 100.0);

        let dist = camera.distance_to_bounding_sphere(&sphere);
        // 距离应约为 900（1000 - 100）
        assert!((dist - 900.0).abs() < 1.0);
    }

    #[test]
    fn test_get_magnitude() {
        let mut camera = Camera::new(
            DVec3::new(6378137.0 * 2.0, 0.0, 0.0),
            -DVec3::X,
            DVec3::Z,
        );
        camera.mode = SceneMode::Scene3D;
        let mag = camera.get_magnitude();
        assert!((mag - 6378137.0 * 2.0).abs() < 1.0);
    }

    #[test]
    fn test_constrained_rotation() {
        let mut camera = Camera::default_camera();
        camera.constrained_axis = Some(DVec3::Y);

        // 绕 right 轴（X）旋转 - 约束轴 Y 阻止 up 越过
        camera.rotate_constrained(DVec3::X, PI * 0.8);

        // up 应有非负的 Y 分量（未越过约束轴）
        assert!(camera.up.dot(DVec3::Y) >= -1e-10,
            "up.dot(Y) = {}", camera.up.dot(DVec3::Y));
        // 验证保持正交归一
        assert!(camera.direction.dot(camera.up).abs() < 1e-10);
        assert!(camera.direction.length() > 0.99);
    }

    #[test]
    fn test_change_detection() {
        let camera = Camera::new(
            DVec3::new(6378137.0 * 2.0, 0.0, 0.0),
            -DVec3::X,
            DVec3::Z,
        );

        // 相同状态 = 无变化
        let pct = camera.compute_change_percentage(camera.position, camera.direction);
        assert!(pct < 0.01);

        // 位置差异很大 = 显著变化
        let pct = camera.compute_change_percentage(
            DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
            camera.direction,
        );
        assert!(pct > 0.1);
    }

    #[test]
    fn test_has_changed() {
        let camera = Camera::new(
            DVec3::new(6378137.0 * 2.0, 0.0, 0.0),
            -DVec3::X,
            DVec3::Z,
        );

        // 无变化
        assert!(!camera.has_changed(camera.position, camera.direction));

        // 方向变化很大
        assert!(camera.has_changed(camera.position, DVec3::X));
    }

    #[test]
    fn test_default_home_position() {
        let pos = Camera::default_home_position(&Ellipsoid::WGS84);
        // 应位于约 2.5 倍地球半径处
        let mag = pos.length();
        assert!((mag - Ellipsoid::WGS84.maximum_radius() * 2.5).abs() / mag < 0.01);
    }

    #[test]
    fn test_offset_from_heading_pitch_range() {
        // Heading=0, Pitch=0, Range=1000 -> 沿 -Y 的偏移（ENU 中的南方，相机朝北看）
        // 验证：heading 调整为 -PI/2，rotMatrix*X=(0,1,0)，取负→(0,-1,0)
        let offset = offset_from_heading_pitch_range(0.0, 0.0, 1000.0);
        assert!(offset.x.abs() < 1e-10, "x={}", offset.x);
        assert!((offset.y + 1000.0).abs() < 1e-10, "y={}", offset.y);
        assert!(offset.z.abs() < 1e-10, "z={}", offset.z);

        // Heading=0, Pitch=PI/2, Range=1000 -> 沿 -Z 的偏移（平面下方，相机向上看）
        let offset = offset_from_heading_pitch_range(0.0, PI / 2.0, 1000.0);
        assert!(offset.x.abs() < 1e-6, "x={}", offset.x);
        assert!(offset.y.abs() < 1e-6, "y={}", offset.y);
        assert!((offset.z + 1000.0).abs() < 1e-6, "z={}", offset.z);
    }
}
