//! 屏幕空间相机控制器系统。
//!
//! 将 Bevy 输入事件转为像素增量，并把**所有**相机
//! 数学委派给领域 [`CameraController`]。pixel→radian 转换在适配层
//! 边界用窗口焦距计算，
//! 使领域保持分辨率无关且不受 render-unit 事务干扰。
//!
//! 缩放基于**地表高度**（`position.length() - ellipsoid.maximum_radius()`），
//! 匹配领域算法与 orbit_camera 已验证的手感。

use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use cesium_camera::Frustum;
use cesium_geospatial::Ellipsoid;
use cesium_interaction::{CameraController, CameraControllerConfig};

use crate::camera::components::{CameraInputState, CesiumCamera};

/// 屏幕空间相机控制器：通过鼠标与触控进行环绕、缩放、平移。
///
/// 所有计算都委派给领域 [`CameraController`]；本系统
/// 只将 Bevy 输入事件转为像素增量，并在适配层边界
/// 计算 pixel→radian 缩放因子。
pub fn camera_controller_system(
    mut cameras: Query<&mut CesiumCamera>,
    mut input_state: ResMut<CameraInputState>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut mouse_motion: EventReader<MouseMotion>,
    mut scroll_events: EventReader<MouseWheel>,
    windows: Query<&Window>,
) {
    // --- 跟踪鼠标按键状态 ---
    input_state.left_mouse_down = mouse_buttons.pressed(MouseButton::Left);
    input_state.right_mouse_down = mouse_buttons.pressed(MouseButton::Right);
    input_state.middle_mouse_down = mouse_buttons.pressed(MouseButton::Middle);

    // --- 累加鼠标增量（像素）---
    let mut total_delta = Vec2::ZERO;
    for ev in mouse_motion.read() {
        total_delta += ev.delta;
    }

    // --- 累加滚动（归一化为卡口数）---
    let mut scroll_notches = 0.0_f64;
    for ev in scroll_events.read() {
        match ev.unit {
            MouseScrollUnit::Line => scroll_notches += ev.y as f64,
            MouseScrollUnit::Pixel => scroll_notches += (ev.y as f64) / 100.0,
        }
    }

    let any_input = total_delta != Vec2::ZERO || scroll_notches != 0.0;
    if !any_input {
        return;
    }

    // 用于 pixel→radian 焦距转换的窗口高度。
    let win_h = windows
        .get_single()
        .map(|w| w.height() as f64)
        .unwrap_or(720.0);

    // 灵敏度乘数（资源零初始化时默认为 1.0）。
    let orbit_sens = non_zero_or(input_state.orbit_sensitivity, 1.0);
    let zoom_sens = non_zero_or(input_state.zoom_sensitivity, 1.0);
    let pan_sens = non_zero_or(input_state.pan_sensitivity, 1.0);

    for mut cesium_cam in cameras.iter_mut() {
        // 在对 `camera` 可变借用之前提取配置值。
        let enable_collision = cesium_cam.enable_collision_detection;
        let min_zoom_dist = cesium_cam.minimum_zoom_distance;
        let max_zoom_dist = cesium_cam.maximum_zoom_distance;
        let cam = &mut cesium_cam.camera;

        // 从组件的配置构建一个领域控制器。
        let config = CameraControllerConfig {
            minimum_zoom_distance: min_zoom_dist,
            maximum_zoom_distance: max_zoom_dist,
            rotation_speed: orbit_sens,
            pan_speed: pan_sens,
            zoom_speed: zoom_sens,
            enable_rotation: true,
            enable_pan: true,
            enable_zoom: true,
            enable_collision_detection: enable_collision,
        };
        // 组装领域控制器：椭球固定为 WGS84，仅携带配置。
        let ctrl = CameraController {
            config,
            ellipsoid: Ellipsoid::WGS84,
        };

        // 地表高度（椭球上方米数）——是 pixel→radian 转换
        // 与缩放/平移缩放的参考基准。
        let surface_height = (cam.position.length() - Ellipsoid::WGS84.maximum_radius())
            .abs()
            .max(1.0);

        // 以像素为单位的焦距：f = (H/2) / tan(fov/2)。
        let fov = match &cam.frustum {
            Frustum::Perspective(f) => f.fov,
            Frustum::Orthographic(_) => std::f64::consts::FRAC_PI_3,
        };
        let focal = (win_h * 0.5) / (fov * 0.5).tan();

        // 地表处的 pixel → radian（抓地球缩放因子）。
        let pixel_to_radian = surface_height / focal;

        // --- 环绕 / 自转（左键拖拽）---
        if input_state.left_mouse_down && total_delta != Vec2::ZERO {
            let heading = total_delta.x as f64 * pixel_to_radian;
            let pitch = -total_delta.y as f64 * pixel_to_radian;
            ctrl.spin(cam, heading, pitch);
        }

        // --- 缩放（右键拖拽）：垂直像素 → 窗口比例 ---
        if input_state.right_mouse_down && total_delta.y != 0.0 {
            let zoom_delta = total_delta.y as f64 / win_h;
            ctrl.zoom(cam, zoom_delta);
        }

        // --- 缩放（滚轮）：一格 = 一个缩放单位 ---
        // At surface height 1e6 m with zoom_speed=1: Δheight = -1e5 m per notch.
        if scroll_notches != 0.0 {
            ctrl.zoom(cam, scroll_notches);
        }

        // --- 平移（中键拖拽）---
        if input_state.middle_mouse_down && total_delta != Vec2::ZERO {
            let pan_x = total_delta.x as f64 / focal;
            let pan_y = -total_delta.y as f64 / focal;
            ctrl.pan(cam, pan_x, pan_y);
        }

        // --- 碰撞检测（委派给领域）---
        ctrl.enforce_collision(cam);
    }
}

/// 返回 `value` 的 f64，当值为零时返回 `fallback`。
#[inline]
fn non_zero_or(value: f32, fallback: f64) -> f64 {
    if value != 0.0 {
        value as f64
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_zero_or_returns_value_when_nonzero() {
        assert!((non_zero_or(2.5, 1.0) - 2.5).abs() < 1e-12);
    }

    #[test]
    fn non_zero_or_returns_fallback_when_zero() {
        assert!((non_zero_or(0.0, 1.0) - 1.0).abs() < 1e-12);
    }

    /// 验证：在地表高度 1e6 m 处，一个滚轮卡口（delta=1.0）
    /// 以 zoom_speed=1.0 产生 zoom_amount = height * 0.1 = 1e5 m。
    #[test]
    fn zoom_at_1e6_height_produces_1e5_per_notch() {
        let config = CameraControllerConfig {
            zoom_speed: 1.0,
            ..Default::default()
        };
        let ctrl = CameraController {
            config,
            ellipsoid: Ellipsoid::WGS84,
        };
        let mut cam = cesium_camera::Camera::new(
            glam::DVec3::new(6378137.0 + 1_000_000.0, 0.0, 0.0),
            glam::DVec3::new(-1.0, 0.0, 0.0),
            glam::DVec3::new(0.0, 0.0, 1.0),
        );
        let initial_height = cam.position.length() - Ellipsoid::WGS84.maximum_radius();
        // 向里缩放（正 delta = 向目标靠近）。
        ctrl.zoom(&mut cam, 1.0);
        let new_height = cam.position.length() - Ellipsoid::WGS84.maximum_radius();
        let delta_h = new_height - initial_height;
        // 应约为 -1e5（向地表靠近了 1e5）。
        assert!(
            (delta_h - (-100_000.0)).abs() < 1.0,
            "expected Δheight ≈ -1e5, got {delta_h}"
        );
    }
}
