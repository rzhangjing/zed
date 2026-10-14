//! 双指触控 → 相机手势适配器（M2.6，适配层半）。
//!
//! 将 Bevy [`TouchInput`] 事件桥接到 M2.6 阶段 1 中构建的
//! 领域双指提放 API（[`CameraEventAggregator::pinch_move`] +
//! [`CameraController::pinch_zoom`] / [`CameraController::pinch_rotate`] /
//! [`CameraController::pinch_translate`]）。所有相机数学都在领域；本系统只
//!
//! 1. 从原始触控流中跟踪两根活动手指，
//! 2. 每帧向领域聚合器喂一次（在 `agg.reset(t)` 之后），以及
//! 3. 在适配层边界计算 pixel→world 缩放因子，镜像
//!    [`super::controller_system`] 中的焦距转换，使触控
//!    路径与鼠标路径共享抓地球的量级。
//!
//! # 手势映射（CesiumJS `ScreenSpaceCameraController`）
//! | 双指手势 | 领域度量 | 相机动作 |
//! |--------------------|---------------|---------------|
//! | 提放（张开/靠拢） | `pinch_distance_delta` (px) | `pinch_zoom` |
//! | 旋转（绕中点转动） | `pinch_angle_delta` (rad) | `pinch_rotate` → spin |
//! | 拖拽（一起平移） | `pinch_midpoint_delta` (px) | `pinch_translate` → pan |
//!
//! # 边界缩放（分辨率无关性留在领域）
//! * `zoom_scale = 1 / window_height` — 手指分离像素变为窗口
//!   高度的比例，与右键拖拽缩放路径喂给
//!   [`CameraController::zoom`] 的同一单位。
//! * `translate_scale = 1 / focal`，`focal = (H/2) / tan(fov/2)` — 中点
//!   像素变为中键拖拽路径喂给
//!   [`CameraController::pan`] 的平移 delta。
//! * `pinch_rotate` 直接取弧度（无像素缩放）。
//!
//! 无触控输入时本系统是惰性的，所以注册它不会扰动
//! 键盘/鼠标路径。

use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use cesium_camera::Frustum;
use cesium_geospatial::Ellipsoid;
use cesium_interaction::{CameraController, CameraControllerConfig, CameraEventAggregator};
use glam::DVec2;

use crate::camera::components::{CameraInputState, CesiumCamera};

/// Bevy [`TouchInput`] 事件与领域 [`CameraEventAggregator`] 之间的
/// 逐帧桥接。
///
/// 持有聚合器加上产出一帧 delta 所需的最小手指
/// 登记：聚合器在 `reset` 后的第一次 `pinch_move` 上重新播种其提放
/// *起点*，所以我们用上一帧的手指对播种它，并用当前对扩展它，
/// 使上报的 delta 恰好等于本帧的双指运动。
#[derive(Resource)]
pub struct TouchCameraState {
    /// 累加双指提放度量的领域聚合器。
    pub agg: CameraEventAggregator,
    /// （至多两根）活动手指，以屏幕像素中的 `(id, position)` 表示，
    /// 按 id 升序保存以获得稳定的指线方向。
    fingers: Vec<(u64, Vec2)>,
    /// 上一帧的手指对，用于播种聚合器的
    /// 逐帧起点。
    prev_pair: Option<(DVec2, DVec2)>,
}

impl Default for TouchCameraState {
    /// 默认：空手指集、无上一帧对，聚合器全新构造。
    fn default() -> Self {
        Self {
            agg: CameraEventAggregator::new(),
            fingers: Vec::new(),
            prev_pair: None,
        }
    }
}

/// 双指触控相机控制：提放 → 缩放，旋转 → 自转，拖拽 → 平移。
///
/// 读取原始 [`TouchInput`] 事件流，在领域 [`CameraEventAggregator`] 中维护双指提放，
/// 并将每个相机变换委派给领域 [`CameraController`]。pixel→world 转换
/// 在适配层边界计算（见模块文档）。
pub fn camera_touch_system(
    mut cameras: Query<&mut CesiumCamera>,
    input_state: Res<CameraInputState>,
    mut touch_state: ResMut<TouchCameraState>,
    mut touch_events: EventReader<TouchInput>,
    time: Res<Time>,
    windows: Query<&Window>,
) {
    // --- 1. 逐帧重置：聚合器重新播种其提放起点。 ---
    touch_state.agg.reset(time.elapsed_secs_f64());

    // --- 2. 将本帧的原始触控事件折叠进跟踪的手指集合。 ---
    for ev in touch_events.read() {
        match ev.phase {
            TouchPhase::Started => {
                if !touch_state.fingers.iter().any(|(id, _)| *id == ev.id) {
                    touch_state.fingers.push((ev.id, ev.position));
                }
            }
            TouchPhase::Moved => {
                if let Some(slot) = touch_state
                    .fingers
                    .iter_mut()
                    .find(|(id, _)| *id == ev.id)
                {
                    slot.1 = ev.position;
                }
            }
            TouchPhase::Ended | TouchPhase::Canceled => {
                touch_state.fingers.retain(|(id, _)| *id != ev.id);
            }
        }
    }
    // 稳定顺序（按 id），以便指线角度在帧间从不翻转；
    // 只有前两根手指驱动提放。
    touch_state.fingers.sort_by_key(|(id, _)| *id);
    touch_state.fingers.truncate(2);

    // 将当前与上一对快照为拥有的局部变量，以便下面的
    // 聚合器借用与手指登记保持不相交。
    let pair = two_fingers(&touch_state.fingers);
    let prev = touch_state.prev_pair;

    // --- 3. 驱动聚合器：提放生命周期，然后以 prev 播种并
    //        以 current 扩展，使 delta 为本帧的运动。 ---
    let mut new_prev: Option<(DVec2, DVec2)> = None;
    {
        let agg = &mut touch_state.agg;
        match pair {
            Some((f1, f2)) => {
                if !agg.is_pinching() {
                    agg.pinch_start(f1, f2);
                }
                if let Some((p1, p2)) = prev {
                    agg.pinch_move(p1, p2);
                }
                agg.pinch_move(f1, f2);
                new_prev = Some((f1, f2));
            }
            None => {
                if agg.is_pinching() {
                    agg.pinch_end();
                }
            }
        }
    }
    touch_state.prev_pair = new_prev;

    // 必须有一个活动的双指提放才能移动相机。
    let Some((_, _)) = pair else {
        return;
    };

    // --- 4. 逐帧手势 delta（领域拥有的语义）。 ---
    let distance_delta = touch_state.agg.pinch_distance_delta();
    let angle_delta = touch_state.agg.pinch_angle_delta();
    let midpoint_delta = touch_state.agg.pinch_midpoint_delta();

    // --- 5. 适配层边界的 pixel→world 缩放（镜像 controller_system）。 ---
    let win_h = windows
        .get_single()
        .map(|w| w.height() as f64)
        .unwrap_or(720.0)
        .max(1.0);
    // 手指分离 px → 窗口高度的比例（右键拖拽缩放单位）。
    let zoom_scale = 1.0 / win_h;

    let orbit_sens = sens_or(input_state.orbit_sensitivity);
    let zoom_sens = sens_or(input_state.zoom_sensitivity);
    let pan_sens = sens_or(input_state.pan_sensitivity);

    for mut cesium_cam in cameras.iter_mut() {
        // 在对 `camera` 可变借用之前提取配置值。
        let enable_collision = cesium_cam.enable_collision_detection;
        let min_zoom_dist = cesium_cam.minimum_zoom_distance;
        let max_zoom_dist = cesium_cam.maximum_zoom_distance;
        let cam = &mut cesium_cam.camera;

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
        let ctrl = CameraController {
            config,
            ellipsoid: Ellipsoid::WGS84,
        };

        // 以像素为单位的焦距：f = (H/2) / tan(fov/2)（抓地球缩放）。
        let fov = match &cam.frustum {
            Frustum::Perspective(f) => f.fov,
            Frustum::Orthographic(_) => std::f64::consts::FRAC_PI_3,
        };
        let focal = ((win_h * 0.5) / (fov * 0.5).tan()).max(1.0);
        // 中点 px → 平移 delta（中键拖拽平移单位）。
        let translate_scale = 1.0 / focal;

        ctrl.pinch_zoom(cam, distance_delta, zoom_scale);
        ctrl.pinch_rotate(cam, angle_delta);
        ctrl.pinch_translate(cam, midpoint_delta, translate_scale);
        ctrl.enforce_collision(cam);
    }
}

/// 将两根跟踪的手指（屏幕 px，按 id 排序）转为 f64 `DVec2`，
/// 当按下手指不足两根时返回 `None`。
fn two_fingers(fingers: &[(u64, Vec2)]) -> Option<(DVec2, DVec2)> {
    if fingers.len() < 2 {
        return None;
    }
    let a = fingers[0].1;
    let b = fingers[1].1;
    Some((
        DVec2::new(a.x as f64, a.y as f64),
        DVec2::new(b.x as f64, b.y as f64),
    ))
}

/// 灵敏度乘数：资源零初始化时
/// 回退到 `1.0`（镜像 `controller_system` 的 `non_zero_or`）。
#[inline]
fn sens_or(value: f32) -> f64 {
    if value != 0.0 {
        value as f64
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::camera_update_system;
    use cesium_camera::Camera;
    use cesium_scene_mode::SceneMode;

    /// WGS84 最大半径（米）；测试相机坐落于其上方一个半径处。
    const R: f64 = 6378137.0;

    /// 构建一个无头应用：`MinimalPlugins`（提供 `Time`）、触控
    /// 事件 + 状态、`Update` 中的 [`camera_touch_system`] 和 `PostUpdate` 中的
    /// Transform 写入器，一个 [`Window`]，以及一个 [`CesiumCamera`] 实体，
    /// 位于赤道上方一个半径处并看向中心。
    fn touch_app() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_event::<TouchInput>()
            .init_resource::<CameraInputState>()
            .init_resource::<TouchCameraState>()
            .add_systems(Update, camera_touch_system)
            .add_systems(PostUpdate, camera_update_system);
        app.world_mut().spawn(Window::default());
        let cam = Camera::new(
            glam::DVec3::new(R * 2.0, 0.0, 0.0),
            glam::DVec3::new(-1.0, 0.0, 0.0),
            glam::DVec3::new(0.0, 0.0, 1.0),
        );
        let cam_e = app
            .world_mut()
            .spawn((
                CesiumCamera::new(cam, SceneMode::Scene3D),
                Transform::default(),
                Projection::default(),
            ))
            .id();
        // 从领域相机预置 Transform（本帧无触控）。
        app.update();
        (app, cam_e)
    }

    /// 向应用的事件队列发送一个 [`TouchInput`] 事件。
    fn send_touch(app: &mut App, phase: TouchPhase, id: u64, x: f32, y: f32) {
        app.world_mut()
            .resource_mut::<Events<TouchInput>>()
            .send(TouchInput {
                phase,
                position: Vec2::new(x, y),
                window: Entity::PLACEHOLDER,
                force: None,
                id,
            });
    }

    fn cesium_position(app: &App, e: Entity) -> glam::DVec3 {
        app.world().get::<CesiumCamera>(e).unwrap().camera.position
    }

    fn transform_len(app: &App, e: Entity) -> f32 {
        app.world()
            .get::<Transform>(e)
            .unwrap()
            .translation
            .length()
    }

    /// 一次双指张开（分离 100 → 200 px）将相机向里缩放：Bevy
    /// `Transform` 平移向椭球中心收缩。
    #[test]
    fn two_finger_spread_zooms_camera_in() {
        let (mut app, e) = touch_app();
        let initial = transform_len(&app, e);
        // 帧 1：两根手指以分离 100 px 落下（提放播种，无移动）。
        send_touch(&mut app, TouchPhase::Started, 1, -50.0, 0.0);
        send_touch(&mut app, TouchPhase::Started, 2, 50.0, 0.0);
        app.update();
        // 帧 2：手指张开到分离 200 px → distance_delta = +100。
        send_touch(&mut app, TouchPhase::Moved, 1, -100.0, 0.0);
        send_touch(&mut app, TouchPhase::Moved, 2, 100.0, 0.0);
        app.update();

        let after = transform_len(&app, e);
        assert!(after < initial, "pinch-spread must zoom in: {after} !< {initial}");
        // 边界量级：Δ分离 100 px / 720 → Δheight ≈ 88 km。
        let dropped = R * 2.0 - cesium_position(&app, e).length();
        assert!(
            (50_000.0..150_000.0).contains(&dropped),
            "zoom magnitude out of range: {dropped} m"
        );
    }

    /// 一次双指靠拢（分离 200 → 100 px）将相机向外缩放。
    #[test]
    fn two_finger_close_zooms_camera_out() {
        let (mut app, e) = touch_app();
        send_touch(&mut app, TouchPhase::Started, 1, -100.0, 0.0);
        send_touch(&mut app, TouchPhase::Started, 2, 100.0, 0.0);
        app.update();
        let initial = transform_len(&app, e);
        send_touch(&mut app, TouchPhase::Moved, 1, -50.0, 0.0);
        send_touch(&mut app, TouchPhase::Moved, 2, 50.0, 0.0);
        app.update();

        let after = transform_len(&app, e);
        assert!(after > initial, "pinch-close must zoom out: {after} !> {initial}");
    }

    /// 一次双指旋转（指线 0 → 90° 逆时针，分离与中点
    /// 固定）绕中心自转相机：距离保留，位置移动
    /// ——证明 `pinch_angle_delta` 抵达 `pinch_rotate` → `spin`。
    #[test]
    fn two_finger_rotate_spins_camera_preserving_distance() {
        let (mut app, e) = touch_app();
        let pos0 = cesium_position(&app, e);
        let len0 = pos0.length();
        send_touch(&mut app, TouchPhase::Started, 1, -50.0, 0.0);
        send_touch(&mut app, TouchPhase::Started, 2, 50.0, 0.0);
        app.update();
        // 将指线旋转 90° 逆时针；分离（100）与中点（0,0）
        // 不变，所以这是一次纯自转。
        send_touch(&mut app, TouchPhase::Moved, 1, 0.0, -50.0);
        send_touch(&mut app, TouchPhase::Moved, 2, 0.0, 50.0);
        app.update();

        let pos1 = cesium_position(&app, e);
        assert!(
            (pos1.length() - len0).abs() / len0 < 1e-9,
            "spin must preserve distance: {} vs {len0}",
            pos1.length()
        );
        assert!((pos1 - pos0).length() > 1.0, "spin must move the camera");
    }

    /// 一次双指拖拽（两手指都向右漂 60 px，分离与角度
    /// 固定）沿切向平移相机——证明 `pinch_midpoint_delta`
    /// 携焦距边界缩放抵达 `pinch_translate` → `pan`。
    #[test]
    fn two_finger_drag_translates_camera_tangentially() {
        let (mut app, e) = touch_app();
        let pos0 = cesium_position(&app, e);
        send_touch(&mut app, TouchPhase::Started, 1, -50.0, 0.0);
        send_touch(&mut app, TouchPhase::Started, 2, 50.0, 0.0);
        app.update();
        // 中点（0,0）→（60,0）；分离（100）与角度（0）不变。
        send_touch(&mut app, TouchPhase::Moved, 1, 10.0, 0.0);
        send_touch(&mut app, TouchPhase::Moved, 2, 110.0, 0.0);
        app.update();

        let pos1 = cesium_position(&app, e);
        let moved = pos1 - pos0;
        assert!(moved.length() > 1.0, "drag must translate the camera");
        // 平移沿视平面进行 → 大多垂直于半径。
        let radial = pos0.normalize();
        let perp = moved - radial * moved.dot(radial);
        assert!(
            perp.length() > 0.5 * moved.length(),
            "translate must be tangential: perp {} of {}",
            perp.length(),
            moved.length()
        );
    }

    /// 一个组合手势（同时张开 + 旋转 + 漂移）产生全部三种
    /// 效果：向里缩放加上一个切向（自转/平移）分量。
    #[test]
    fn combined_gesture_zooms_spins_and_translates() {
        let (mut app, e) = touch_app();
        let pos0 = cesium_position(&app, e);
        let len0 = pos0.length();
        send_touch(&mut app, TouchPhase::Started, 1, -50.0, 0.0);
        send_touch(&mut app, TouchPhase::Started, 2, 50.0, 0.0);
        app.update();
        send_touch(&mut app, TouchPhase::Moved, 1, -30.0, 40.0);
        send_touch(&mut app, TouchPhase::Moved, 2, 110.0, 20.0);
        app.update();

        let pos1 = cesium_position(&app, e);
        assert!(pos1.length() < len0, "combined gesture must zoom in");
        let radial = pos1.normalize();
        let tangential = pos1 - pos0;
        let perp = tangential - radial * tangential.dot(radial);
        assert!(perp.length() > 1.0, "combined gesture must spin/translate");
    }

    /// 多帧张开单调地缩放（跨多帧的逐帧重新播种起作用，
    /// 镜像领域的序列测试）。
    #[test]
    fn multi_frame_spread_zooms_in_monotonically() {
        let (mut app, e) = touch_app();
        send_touch(&mut app, TouchPhase::Started, 1, -20.0, 0.0);
        send_touch(&mut app, TouchPhase::Started, 2, 20.0, 0.0);
        app.update();
        let mut prev_len = cesium_position(&app, e).length();
        for half in [40.0_f32, 70.0, 110.0, 160.0] {
            send_touch(&mut app, TouchPhase::Moved, 1, -half, 0.0);
            send_touch(&mut app, TouchPhase::Moved, 2, half, 0.0);
            app.update();
            let len = cesium_position(&app, e).length();
            assert!(len < prev_len, "spread to {half} must zoom in: {len} !< {prev_len}");
            prev_len = len;
        }
    }

    /// 抬起一根手指结束提放；随后的单指移动不得
    /// 缩放（双指作用域被干净地释放）。
    #[test]
    fn releasing_a_finger_ends_the_pinch() {
        let (mut app, e) = touch_app();
        send_touch(&mut app, TouchPhase::Started, 1, -50.0, 0.0);
        send_touch(&mut app, TouchPhase::Started, 2, 50.0, 0.0);
        app.update();
        send_touch(&mut app, TouchPhase::Moved, 1, -100.0, 0.0);
        send_touch(&mut app, TouchPhase::Moved, 2, 100.0, 0.0);
        app.update();
        let after_zoom = cesium_position(&app, e).length();

        // 抬起手指 2 → 提放结束。
        send_touch(&mut app, TouchPhase::Ended, 2, 100.0, 0.0);
        app.update();
        // 单独手指 1 的移动必须被双指路径忽略。
        send_touch(&mut app, TouchPhase::Moved, 1, -300.0, 0.0);
        app.update();

        let after_release = cesium_position(&app, e).length();
        assert!(
            (after_release - after_zoom).abs() < 1e-6,
            "after release the pinch must stop: {after_zoom} → {after_release}"
        );
        assert!(
            !app.world().resource::<TouchCameraState>().agg.is_pinching(),
            "aggregator must report not-pinching after release"
        );
    }

    /// 零触控事件时系统惰性：相机逐字节不变，
    /// 证明键盘/鼠标默认路径未被扰动。
    #[test]
    fn no_touch_leaves_camera_unchanged() {
        let (mut app, e) = touch_app();
        let pos0 = cesium_position(&app, e);
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(pos0, cesium_position(&app, e), "touch system must be inert");
    }
}
