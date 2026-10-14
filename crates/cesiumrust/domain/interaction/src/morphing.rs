//! 场景模式变形（在 2D/3D/Columbus View 之间切换）。
//!
//! 记录起止相机状态，按平滑阶跃缓动在若干帧内插值位置、
//! 方向与 up 向量，完成模式间的过渡。

use cesium_camera::{Camera, SceneMode};
use cesium_geospatial::Ellipsoid;
use glam::DVec3;

/// 变形过渡的状态。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum MorphState {
    /// 未在变形 - 稳定处于某个场景模式。
    #[default]
    Idle,
    /// 从一个模式变形到另一个模式。
    Morphing {
        /// 源模式。
        from: SceneMode,
        /// 目标模式。
        to: SceneMode,
        /// 进度（0.0 到 1.0）。
        progress: f64,
    },
}

/// 管理场景模式的变形过渡。
/// 按起止相机状态在若干帧内插值，完成模式间的过渡。
#[derive(Debug, Clone)]
pub struct SceneMorph {
    /// 当前的变形状态。
    pub state: MorphState,
    /// 变形过渡的时长（以秒计）。
    pub duration: f64,
    /// 当前变形中已经过的时间。
    pub elapsed: f64,
    /// 起始相机位置。
    pub start_position: DVec3,
    /// 结束相机位置。
    pub end_position: DVec3,
    /// 起始相机方向。
    pub start_direction: DVec3,
    /// 结束相机方向。
    pub end_direction: DVec3,
    /// 起始相机 up 向量。
    pub start_up: DVec3,
    /// 结束相机 up 向量。
    pub end_up: DVec3,
}

impl Default for SceneMorph {
    /// 默认变形管理器：空闲状态、2 秒过渡、沿 -Z 方向与 +Y 上方向。
    fn default() -> Self {
        Self {
            state: MorphState::Idle,
            duration: 2.0,
            elapsed: 0.0,
            start_position: DVec3::ZERO,
            end_position: DVec3::ZERO,
            start_direction: -DVec3::Z,
            end_direction: -DVec3::Z,
            start_up: DVec3::Y,
            end_up: DVec3::Y,
        }
    }
}

impl SceneMorph {
    /// 创建一个新的场景变形管理器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 返回当前是否正在进行变形。
    pub fn is_morphing(&self) -> bool {
        matches!(self.state, MorphState::Morphing { .. })
    }

    /// 返回当前进度（0.0 到 1.0），若未变形则返回 0。
    pub fn progress(&self) -> f64 {
        match self.state {
            MorphState::Morphing { progress, .. } => progress,
            MorphState::Idle => 0.0,
        }
    }

    /// 在场景模式之间启动一次变形过渡。
    ///
    /// # 参数
    /// * `camera` - 当前相机状态
    /// * `from` - 源场景模式
    /// * `to` - 目标场景模式
    /// * `ellipsoid` - 用于坐标转换的椭球
    /// * `duration` - 以秒计的过渡时长
    pub fn start_morph(
        &mut self,
        camera: &Camera,
        from: SceneMode,
        to: SceneMode,
        ellipsoid: &Ellipsoid,
        duration: f64,
    ) {
        if from == to {
            return;
        }

        // 进入变形状态，重置进度与计时
        self.state = MorphState::Morphing {
            from,
            to,
            progress: 0.0,
        };
        // 时长下限 1ms 避免除零，并重置已用时间
        self.duration = duration.max(0.001);
        self.elapsed = 0.0;

        // 保存起始状态
        self.start_position = camera.position;
        self.start_direction = camera.direction;
        self.start_up = camera.up;

        // 依据目标模式推算变形终点的位置与朝向
        let (end_pos, end_dir, end_up) =
            compute_morph_target(camera, from, to, ellipsoid);
        self.end_position = end_pos;
        self.end_direction = end_dir;
        self.end_up = end_up;
    }

    /// 更新变形过渡。
    ///
    /// # 参数
    /// * `dt` - 以秒计的时间增量
    /// * `camera` - 要更新的相机
    ///
    /// # 返回
    /// 若仍在变形则返回 `true`，若已完成则返回 `false`。
    pub fn update(&mut self, dt: f64, camera: &mut Camera) -> bool {
        let (from, to) = match self.state {
            MorphState::Morphing { from, to, .. } => (from, to),
            MorphState::Idle => return false,
        };

        self.elapsed += dt;
        // 归一化进度：已用时/总时长，钳制到 [0,1]
        let t = (self.elapsed / self.duration).clamp(0.0, 1.0);

        // 平滑阶跃缓动（smoothstep），使起止更柔和
        let t_smooth = t * t * (3.0 - 2.0 * t);

        // 插值相机状态
        // 位置直接线性插值，方向/up 插值后需重新归一化
        camera.position = self.start_position.lerp(self.end_position, t_smooth);
        camera.direction = self.start_direction.lerp(self.end_direction, t_smooth).normalize();
        camera.up = self.start_up.lerp(self.end_up, t_smooth).normalize();
        // 由插值后的 direction/up 重新正交化出 right/up 基
        camera.right = camera.direction.cross(camera.up).normalize();
        camera.up = camera.right.cross(camera.direction).normalize();

        if t >= 1.0 {
            // 到达终点：落位目标模式并回到空闲
            self.state = MorphState::Idle;
            camera.mode = to;
            false
        } else {
            // 尚未完成：推进进度并标记为变形中
            self.state = MorphState::Morphing {
                from,
                to,
                progress: t,
            };
            camera.mode = SceneMode::Morphing;
            true
        }
    }

    /// 立即完成变形过渡。
    /// 跳过剩余动画，把相机直接落到变形终点并切换到目标模式。
    pub fn complete_morph(&mut self, camera: &mut Camera) {
        // 若正在变形，套用预先算好的终点状态
        if let MorphState::Morphing { to, .. } = self.state {
            camera.position = self.end_position;
            camera.direction = self.end_direction;
            camera.up = self.end_up;
            camera.right = camera.direction.cross(camera.up).normalize();
            camera.up = camera.right.cross(camera.direction).normalize();
            camera.mode = to;
        }
        self.state = MorphState::Idle;
    }

    /// 取消变形并返回到源模式。
    /// 丢弃进度，把相机恢复到变形起始状态与源模式。
    pub fn cancel_morph(&mut self, camera: &mut Camera) {
        // 回到起始位置/朝向，并恢复源模式
        if let MorphState::Morphing { from, .. } = self.state {
            camera.position = self.start_position;
            camera.direction = self.start_direction;
            camera.up = self.start_up;
            // 重新正交化朝向基，保证与 direction 两两垂直
            camera.right = camera.direction.cross(camera.up).normalize();
            camera.up = camera.right.cross(camera.direction).normalize();
            camera.mode = from;
        }
        self.state = MorphState::Idle;
    }
}

/// 计算变形过渡的目标相机状态。
/// 依据起止模式对，返回目标位置、视线方向与 up 向量。
fn compute_morph_target(
    camera: &Camera,
    from: SceneMode,
    to: SceneMode,
    ellipsoid: &Ellipsoid,
) -> (DVec3, DVec3, DVec3) {
    match (from, to) {
        // 3D → 2D：将相机移动到俯视视角
        // 保持当前经纬度、抬升高度，令视线沿当地法线垂直向下
        (SceneMode::Scene3D, SceneMode::Scene2D) => {
            let height = camera.position.length() - ellipsoid.maximum_radius();
            let carto = ellipsoid.cartesian_to_cartographic(camera.position);
            if let Some(carto) = carto {
                let pos = ellipsoid.cartographic_to_cartesian(
                    &cesium_geospatial::Cartographic::from_radians(
                        carto.longitude,
                        carto.latitude,
                        height.max(ellipsoid.maximum_radius()),
                    ),
                );
                let dir = -pos.normalize();
                // 以地轴为参考构造俯视 up，退化时回退到 +Y
                let up = DVec3::Z.cross(dir).normalize();
                let up = if up.length_squared() < 1e-10 { DVec3::Y } else { up };
                (pos, dir, up)
            } else {
                (camera.position, camera.direction, camera.up)
            }
        }
        // 2D → 3D：将相机移动到倾斜视角
        // 位置不变，视线相对法线略微下压以营造倾斜透视
        (SceneMode::Scene2D, SceneMode::Scene3D) => {
            let pos = camera.position;
            let normal = pos.normalize();
            // 倾斜以看向地平线
            let dir = (-normal + DVec3::new(0.0, 0.0, 0.3)).normalize();
            let right = dir.cross(DVec3::Z).normalize();
            let up = right.cross(dir).normalize();
            (pos, dir, up)
        }
        // 3D → Columbus View：压平为 2.5D
        // 把经纬度展平为平面坐标，高度沿天顶方向保留
        (SceneMode::Scene3D, SceneMode::ColumbusView) => {
            let carto = ellipsoid.cartesian_to_cartographic(camera.position);
            if let Some(carto) = carto {
                let height = carto.height.max(1000.0);
                // 在 CV 中，位置处于平面坐标系
                let pos = DVec3::new(
                    carto.longitude * ellipsoid.maximum_radius(),
                    carto.latitude * ellipsoid.maximum_radius(),
                    height,
                );
                let dir = -DVec3::Z;
                let up = DVec3::Y;
                (pos, dir, up)
            } else {
                (camera.position, camera.direction, camera.up)
            }
        }
        // Columbus View → 3D
        // 将平面 CV 坐标换算回椭球面上的三维位置
        (SceneMode::ColumbusView, SceneMode::Scene3D) => {
            // 将平面 CV 坐标转换回 3D
            let lon = camera.position.x / ellipsoid.maximum_radius();
            let lat = camera.position.y / ellipsoid.maximum_radius();
            let height = camera.position.z;
            let carto = cesium_geospatial::Cartographic::from_radians(lon, lat, height);
            let pos = ellipsoid.cartographic_to_cartesian(&carto);
            let dir = -pos.normalize();
            let up = DVec3::Z.cross(dir).normalize();
            let up = if up.length_squared() < 1e-10 { DVec3::Y } else { up };
            (pos, dir, up)
        }
        // 默认：保持当前状态
        _ => (camera.position, camera.direction, camera.up),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 构造一台位于赤道上空、朝 -X 看的测试相机
    fn create_test_camera() -> Camera {
        Camera::new(
            DVec3::new(6378137.0 * 2.0, 0.0, 0.0),
            DVec3::new(-1.0, 0.0, 0.0),
            DVec3::new(0.0, 0.0, 1.0),
        )
    }

    // 验证：新建管理器默认处于空闲、非变形、进度为 0
    #[test]
    fn test_morph_state_default() {
        let morph = SceneMorph::new();
        assert_eq!(morph.state, MorphState::Idle);
        assert!(!morph.is_morphing());
        assert!((morph.progress()).abs() < 1e-10);
    }

    // 验证：从 3D 向 2D 发起变形后进入变形中且进度为 0
    #[test]
    fn test_start_morph() {
        let mut morph = SceneMorph::new();
        let camera = create_test_camera();

        morph.start_morph(
            &camera,
            SceneMode::Scene3D,
            SceneMode::Scene2D,
            &Ellipsoid::WGS84,
            2.0,
        );

        assert!(morph.is_morphing());
        assert!((morph.progress()).abs() < 1e-10);
    }

    // 验证：源与目标模式相同时不触发任何变形
    #[test]
    fn test_morph_same_mode_noop() {
        let mut morph = SceneMorph::new();
        let camera = create_test_camera();

        morph.start_morph(
            &camera,
            SceneMode::Scene3D,
            SceneMode::Scene3D,
            &Ellipsoid::WGS84,
            2.0,
        );

        assert!(!morph.is_morphing());
    }

    // 验证：update 在中途保持变形、末尾落位目标模式
    #[test]
    fn test_morph_update() {
        let mut morph = SceneMorph::new();
        let camera = create_test_camera();
        let mut camera = camera;

        morph.start_morph(
            &camera,
            SceneMode::Scene3D,
            SceneMode::Scene2D,
            &Ellipsoid::WGS84,
            2.0,
        );

        // 进行到一半
        let still_morphing = morph.update(1.0, &mut camera);
        assert!(still_morphing);
        assert!((morph.progress() - 0.5).abs() < 0.01);
        assert_eq!(camera.mode, SceneMode::Morphing);

        // 完成
        let still_morphing = morph.update(1.0, &mut camera);
        assert!(!still_morphing);
        assert!(!morph.is_morphing());
        assert_eq!(camera.mode, SceneMode::Scene2D);
    }

    // 验证：complete_morph 立即把相机落到终点并结束变形
    #[test]
    fn test_complete_morph() {
        let mut morph = SceneMorph::new();
        let camera = create_test_camera();
        let mut camera = camera;

        morph.start_morph(
            &camera,
            SceneMode::Scene3D,
            SceneMode::Scene2D,
            &Ellipsoid::WGS84,
            2.0,
        );

        morph.update(0.5, &mut camera);
        morph.complete_morph(&mut camera);

        assert!(!morph.is_morphing());
        assert_eq!(camera.mode, SceneMode::Scene2D);
        assert!(camera.position.abs_diff_eq(morph.end_position, 1e-6));
    }

    // 验证：cancel_morph 恢复源模式并回到起始位置
    #[test]
    fn test_cancel_morph() {
        let mut morph = SceneMorph::new();
        let camera = create_test_camera();
        let mut camera = camera;
        let original_pos = camera.position;

        morph.start_morph(
            &camera,
            SceneMode::Scene3D,
            SceneMode::Scene2D,
            &Ellipsoid::WGS84,
            2.0,
        );

        morph.update(0.5, &mut camera);
        morph.cancel_morph(&mut camera);

        assert!(!morph.is_morphing());
        assert_eq!(camera.mode, SceneMode::Scene3D);
        assert!(camera.position.abs_diff_eq(original_pos, 1e-6));
    }

    // 验证：3D → Columbus View 完成变形后模式切换正确
    #[test]
    fn test_morph_3d_to_columbus_view() {
        let mut morph = SceneMorph::new();
        let camera = create_test_camera();
        let mut camera = camera;

        morph.start_morph(
            &camera,
            SceneMode::Scene3D,
            SceneMode::ColumbusView,
            &Ellipsoid::WGS84,
            1.0,
        );

        assert!(morph.is_morphing());

        // 完成变形
        morph.update(1.0, &mut camera);
        assert_eq!(camera.mode, SceneMode::ColumbusView);
    }

    // 验证：时长 0 会被钳制到最小值，避免除零
    #[test]
    fn test_morph_duration_clamped() {
        let mut morph = SceneMorph::new();
        let camera = create_test_camera();

        morph.start_morph(
            &camera,
            SceneMode::Scene3D,
            SceneMode::Scene2D,
            &Ellipsoid::WGS84,
            0.0, // 应被钳制到 0.001
        );

        assert!(morph.duration >= 0.001);
    }
}
