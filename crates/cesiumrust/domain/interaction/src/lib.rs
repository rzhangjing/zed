//! cesium-interaction：相机控制器、飞行动画、拾取与事件。
//!
//! 领域层 - 纯 Rust，f64 精度。
//!
//! 各子模块职责：
//! - `camera_controller`：屏幕空间相机的拖拽、缩放与旋转交互。
//! - `inertia`：平移/旋转的惯性衰减与滚动停止判定。
//! - `flight`：`fly_to`/`look_at` 相机飞行的时长与缓动计算。
//! - `picking`：窗口坐标到世界射线的拾取与屏幕投影。
//! - `event_aggregator`：将鼠标/触摸事件聚合为相机手势。
//! - `morphing`：2D/3D/ColumbusView 之间的模式变形过渡。

pub mod camera_controller;
pub mod flight;
pub mod inertia;
pub mod picking;
pub mod event_aggregator;
pub mod morphing;

pub use camera_controller::{CameraController, CameraControllerConfig};
pub use flight::{
    compute_flight_duration, compute_look_at, compute_set_view, select_flight_easing, CameraFlight,
    FlightOptions,
};
pub use inertia::{
    decay, InertiaController, InertiaMovementState, InertiaSample, InertiaState,
    INERTIA_MAX_CLICK_TIME_THRESHOLD, INERTIA_STOP_DISTANCE,
};
pub use picking::{Viewport, get_pick_ray, pick_ellipsoid, world_to_screen, window_center};
pub use event_aggregator::{
    AggregateMovement, CameraEventAggregator, CameraEventType, MouseButton, PinchMovement, StartEnd,
};
pub use morphing::{SceneMorph, MorphState};
