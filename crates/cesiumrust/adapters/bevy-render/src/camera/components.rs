// legacy CesiumJS-port style debt (deferred.md #18); revisit at M13 lint-cleanup 或本文件在其里程碑被重写时
#![allow(clippy::derivable_impls)]
use bevy::prelude::*;
use cesium_camera::Camera;
use cesium_scene_mode::SceneMode;

/// 主相机组件，包装领域层的 Camera。
#[derive(Component)]
pub struct CesiumCamera {
    /// 领域层相机（位置/朝向/分景参数）。
    pub camera: Camera,
    /// 当前场景模式（2D/3D/Columbus）。
    pub scene_mode: SceneMode,
    /// 是否启用碰撞检测（限制镜头入地）。
    pub enable_collision_detection: bool,
    /// 最近缩放距离（米）。
    pub minimum_zoom_distance: f64,
    /// 最远缩放距离（米）。
    pub maximum_zoom_distance: f64,
}

impl Default for CesiumCamera {
    /// 默认：Scene3D、开启碰撞检测、缩放范围 100m~2000万 m。
    fn default() -> Self {
        Self {
            camera: Camera::default_camera(),
            scene_mode: SceneMode::Scene3D,
            enable_collision_detection: true,
            minimum_zoom_distance: 100.0,
            maximum_zoom_distance: 20_000_000.0,
        }
    }
}

impl CesiumCamera {
    /// 用指定领域相机与场景模式新建组件（其余参数取默认）。
    ///
    /// # 参数
    /// - `camera`：领域层相机
    /// - `scene_mode`：初始场景模式
    pub fn new(camera: Camera, scene_mode: SceneMode) -> Self {
        Self {
            camera,
            scene_mode,
            ..Default::default()
        }
    }
}

/// 请求将相机飞向一个制图目的地。
#[derive(Event)]
pub struct FlyToRequest {
    /// 目标制图坐标（经/纬/高）。
    pub destination: cesium_geospatial::Cartographic,
    /// 飞行动画时长（秒）。
    pub duration_secs: f64,
}

/// 相机飞行完成时发出。
#[derive(Event)]
pub struct FlightComplete;

/// 用于相机控制的鼠标与触控输入状态。
#[derive(Resource, Default)]
pub struct CameraInputState {
    /// 左键是否按下（通用于旋转/轨道）。
    pub left_mouse_down: bool,
    /// 右键是否按下（通用于俯仰）。
    pub right_mouse_down: bool,
    /// 中键是否按下（通用于平移）。
    pub middle_mouse_down: bool,
    /// 上一帧鼠标位置（用于计算增量）。
    pub last_mouse_pos: Option<Vec2>,
    /// 轨道灵敏度。
    pub orbit_sensitivity: f32,
    /// 缩放灵敏度。
    pub zoom_sensitivity: f32,
    /// 平移灵敏度。
    pub pan_sensitivity: f32,
    /// 是否有触控在手。
    pub touch_active: bool,
    /// 上一次双指距（用于捻合缩放）。
    pub last_touch_distance: Option<f32>,
    /// 上一次双指中心（用于平移）。
    pub last_touch_center: Option<Vec2>,
}

/// 活动飞行动画状态（存为一个资源）。
#[derive(Resource, Default)]
pub struct ActiveFlight {
    /// 当前正在进行的飞行（无则为 None）。
    pub flight: Option<cesium_interaction::CameraFlight>,
}

/// 活动场景模式渐变状态。
#[derive(Resource)]
pub struct ActiveMorph {
    /// 当前渐变状态（起止模式与进度）。
    pub state: cesium_scene_mode::MorphState,
}

impl Default for ActiveMorph {
    /// 默认：使用领域层 MorphState 的默认值。
    fn default() -> Self {
        Self {
            state: cesium_scene_mode::MorphState::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 默认相机应为 Scene3D、开启碰撞、缩放范围符合预设。
    fn test_cesium_camera_default() {
        let cc = CesiumCamera::default();
        assert_eq!(cc.scene_mode, SceneMode::Scene3D);
        assert!(cc.enable_collision_detection);
        assert!((cc.minimum_zoom_distance - 100.0).abs() < 1e-10);
        assert!((cc.maximum_zoom_distance - 20_000_000.0).abs() < 1e-10);
    }

    #[test]
    /// 默认输入状态应为无按键、无历史位置。
    fn test_camera_input_state_default() {
        let state = CameraInputState::default();
        assert!(!state.left_mouse_down);
        assert!(!state.right_mouse_down);
        assert!(!state.middle_mouse_down);
        assert!(state.last_mouse_pos.is_none());
    }

    #[test]
    /// new 应以传入的相机与模式初始化。
    fn test_cesium_camera_new() {
        let cam = Camera::default_camera();
        let cc = CesiumCamera::new(cam.clone(), SceneMode::Scene2D);
        assert_eq!(cc.scene_mode, SceneMode::Scene2D);
        assert_eq!(cc.camera.position, cam.position);
    }
}
