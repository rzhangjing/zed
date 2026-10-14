pub mod components;
pub mod controller_system;
pub mod control_impl;
pub mod flight_system;
pub mod scene_mode_system;
pub mod touch_system;
pub mod update_system;

pub use components::{ActiveFlight, ActiveMorph, CameraInputState, CesiumCamera, FlightComplete, FlyToRequest};
pub use controller_system::camera_controller_system;
pub use control_impl::{
    camera_control_port_system, CameraControlImpl, CameraControlPort, CameraState,
};
pub use flight_system::camera_flight_system;
pub use scene_mode_system::scene_mode_system;
pub use touch_system::{camera_touch_system, TouchCameraState};
pub use update_system::camera_update_system;

use bevy::prelude::*;

/// 注册 CesiumRust 相机系统与资源的 Plugin。
pub struct CesiumCameraPlugin;

impl Plugin for CesiumCameraPlugin {
    /// 初始化相机相关资源与事件，并在各阶段挂载控制/触控/更新/飞行/模式系统。
    ///
    /// # 参数
    /// - `app`：Bevy 应用
    fn build(&self, app: &mut App) {
        // 输入状态/飞行/渐变/触控/控制端口等资源与两个事件均需提前注册。
        app.init_resource::<CameraInputState>()
            .init_resource::<ActiveFlight>()
            .init_resource::<ActiveMorph>()
            .init_resource::<TouchCameraState>()
            .init_resource::<CameraControlPort>()
            .add_event::<FlyToRequest>()
            .add_event::<FlightComplete>()
            .add_systems(PreUpdate, camera_controller_system)
            // 双指触控 → 相机（无触控输入时惰性；在鼠标
            // 控制器之后、PostUpdate 的 Transform 写入之前运行）。
            .add_systems(Update, camera_touch_system)
            .add_systems(
                PostUpdate,
                (
                    // M2.5：CameraControl 驱动端桥接，置于 Transform
                    // 写入器之前，以便端口驱动的位姿是
                    // 被转换为 render units 的那个。
                    camera_control_port_system.before(camera_update_system),
                    camera_update_system,
                    camera_flight_system,
                    scene_mode_system,
                ),
            );
    }
}
