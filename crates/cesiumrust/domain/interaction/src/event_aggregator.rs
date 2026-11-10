//! 相机事件聚合系统。
//!
//! 为相机控制逐帧聚合鼠标/键盘/触控事件。
//!
//! 按事件类型维护各自的移动状态，并在每帧开头重置。

use glam::DVec2;

/// 鼠标按钮标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// 鼠标左键。
    Left,
    /// 鼠标右键。
    Right,
    /// 鼠标中键。
    Middle,
}

/// 相机事件类型。
/// 描述按下/抬起/拖拽等驱动相机的输入类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraEventType {
    /// 鼠标左键按下。
    LeftDown,
    /// 鼠标左键抬起。
    LeftUp,
    /// 鼠标左键拖拽。
    LeftDrag,
    /// 鼠标右键按下。
    RightDown,
    /// 鼠标右键抬起。
    RightUp,
    /// 鼠标右键拖拽。
    RightDrag,
    /// 鼠标中键按下。
    MiddleDown,
    /// 鼠标中键抬起。
    MiddleUp,
    /// 鼠标中键拖拽。
    MiddleDrag,
    /// 鼠标滚轮滚动。
    Wheel,
    /// 捻合（触控）。
    Pinch,
}

/// 单个事件类型的聚合移动数据。
#[derive(Debug, Clone, Default)]
pub struct AggregateMovement {
    /// 移动的起始位置。
    pub start_position: DVec2,
    /// 移动的结束位置。
    pub end_position: DVec2,
    /// 总移动增量。
    pub movement: DVec2,
    /// 按钮当前是否处于按下状态。
    pub is_button_down: bool,
    /// 本帧是否发生了移动。
    pub is_moving: bool,
    /// 移动开始的时间（秒）。
    pub start_time: f64,
    /// 最后一次移动发生的时间（秒）。
    pub last_time: f64,
}

impl AggregateMovement {
    /// 创建一个空的新聚合移动。
    pub fn new() -> Self {
        Self::default()
    }

    /// 为新的一帧重置移动状态。
    pub fn reset_frame(&mut self) {
        self.movement = DVec2::ZERO;
        self.is_moving = false;
    }

    /// 记录一次按钮按下事件。
    pub fn button_down(&mut self, position: DVec2, time: f64) {
        self.is_button_down = true;
        self.start_position = position;
        self.end_position = position;
        self.start_time = time;
        self.last_time = time;
    }

    /// 记录一次按钮抬起事件。
    pub fn button_up(&mut self, time: f64) {
        self.is_button_down = false;
        self.last_time = time;
    }

    /// 记录一次拖拽/移动事件。
    pub fn drag(&mut self, position: DVec2, time: f64) {
        if self.is_button_down {
            self.end_position = position;
            self.movement = self.end_position - self.start_position;
            self.is_moving = true;
            self.last_time = time;
        }
    }

    /// 记录一次滚轮事件。
    pub fn wheel(&mut self, delta: f64, time: f64) {
        self.movement = DVec2::new(0.0, delta);
        self.is_moving = true;
        self.last_time = time;
    }
}

/// 一个聚合子移动的起始/结束位置对。
///
/// 记录一次聚合子移动的起始与结束坐标，
/// 上层据此求出差分增量（例如捻合内部的起止对）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StartEnd {
    /// 聚合后的起始位置。
    pub start_position: DVec2,
    /// 聚合后的结束位置。
    pub end_position: DVec2,
}

/// 聚合的双指捻合移动（触控）。
///
/// 保存双指捻合的聚合度量，涵盖间距、角度与中点三部分，
/// 字段含义如下：
/// - `distance` 保存两指间距（标量存于 `.y`）；其增量
///   驱动缩放。
/// - `angle_and_height` 保存手指连线角度（弧度，`.x`）与
///   中点高度（`.y`）；角度增量驱动旋转。
/// - `prev_angle` 防止角度聚合在 360° 处翻转。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PinchMovement {
    /// 手指间距（起始/结束），标量存于 `.y`。
    pub distance: StartEnd,
    /// 手指角度（`.x`，弧度）与中点高度（`.y`），起始/结束。
    pub angle_and_height: StartEnd,
    /// 两指中点（手指之间的中心），起始/结束。其
    /// 逐帧增量是两指共同的平移，驱动
    /// 双指 **拖拽 → 平移** 手势（M2.6）。标准捻合仅
    /// 含缩放与旋转、不跟踪中点平移，因此这一项是为
    /// 旋转/平移触控模型新增的部分。
    pub midpoint: StartEnd,
    /// 用于防翻转回绕的上一个角度。
    pub prev_angle: f64,
}

/// 计算两个手指位置的捻合度量 `(separation, angle, midpoint_height)`。
///
/// `angle` 为 `finger1 → finger2` 连线的方向（弧度）；中点
/// 高度为屏幕 `y` 的平均值，与捻合的 angleAndHeight 语义一致。
fn pinch_metrics(finger1: DVec2, finger2: DVec2) -> (f64, f64, f64) {
    let delta = finger2 - finger1;
    let distance = delta.length();
    let angle = delta.y.atan2(delta.x);
    let height = (finger1.y + finger2.y) * 0.5;
    (distance, angle, height)
}

/// 逐帧聚合相机事件。
/// 按事件类型维护每帧的聚合移动状态。
#[derive(Debug, Clone)]
pub struct CameraEventAggregator {
    /// 每个事件类型的移动状态。
    movements: Vec<(CameraEventType, AggregateMovement)>,
    /// 当前帧时间。
    current_time: f64,
    /// 聚合的双指捻合移动。
    pinch: PinchMovement,
    /// 捻合手势当前是否正在进行。
    pinching: bool,
    /// 下一次捻合移动是否应重新播种每帧的起始值。
    pinch_update_pending: bool,
}

impl Default for CameraEventAggregator {
    /// 默认聚合器：空状态、无活动手势。
    fn default() -> Self {
        Self::new()
    }
}

impl CameraEventAggregator {
    /// 创建一个新的事件聚合器。
    pub fn new() -> Self {
        Self {
            movements: Vec::new(),
            current_time: 0.0,
            pinch: PinchMovement::default(),
            pinching: false,
            pinch_update_pending: false,
        }
    }

    /// 为新一帧重置所有移动。
    pub fn reset(&mut self, time: f64) {
        self.current_time = time;
        for (_, movement) in &mut self.movements {
            movement.reset_frame();
        }
        // 在新帧的首次移动上重新播种捻合的起始/prev-angle
        // （重置后在下一帧首次移动时重新播种）。
        if self.pinching {
            self.pinch_update_pending = true;
        }
    }

    /// 获取或创建某个事件类型的移动状态。
    fn get_movement_mut(&mut self, event_type: CameraEventType) -> &mut AggregateMovement {
        let found = self.movements.iter().position(|(t, _)| *t == event_type);
        let idx = match found {
            Some(idx) => idx,
            None => {
                self.movements.push((event_type, AggregateMovement::new()));
                self.movements.len() - 1
            }
        };
        &mut self.movements[idx].1
    }

    /// 获取某个事件类型的移动状态。
    pub fn get_movement(&self, event_type: CameraEventType) -> Option<&AggregateMovement> {
        self.movements.iter().find(|(t, _)| *t == event_type).map(|(_, m)| m)
    }

    /// 记录一次按钮按下事件。
    pub fn button_down(&mut self, button: MouseButton, position: DVec2) {
        let time = self.current_time;
        let event_type = match button {
            MouseButton::Left => CameraEventType::LeftDown,
            MouseButton::Right => CameraEventType::RightDown,
            MouseButton::Middle => CameraEventType::MiddleDown,
        };
        self.get_movement_mut(event_type).button_down(position, time);

        // 同时将拖拽事件标记为按钮按下
        let drag_type = match button {
            MouseButton::Left => CameraEventType::LeftDrag,
            MouseButton::Right => CameraEventType::RightDrag,
            MouseButton::Middle => CameraEventType::MiddleDrag,
        };
        self.get_movement_mut(drag_type).button_down(position, time);
    }

    /// 记录一次按钮抬起事件。
    pub fn button_up(&mut self, button: MouseButton) {
        let time = self.current_time;
        let event_type = match button {
            MouseButton::Left => CameraEventType::LeftUp,
            MouseButton::Right => CameraEventType::RightUp,
            MouseButton::Middle => CameraEventType::MiddleUp,
        };
        self.get_movement_mut(event_type).button_up(time);

        // 同时将拖拽事件标记为按钮抬起
        let drag_type = match button {
            MouseButton::Left => CameraEventType::LeftDrag,
            MouseButton::Right => CameraEventType::RightDrag,
            MouseButton::Middle => CameraEventType::MiddleDrag,
        };
        self.get_movement_mut(drag_type).button_up(time);
    }

    /// 记录一次鼠标移动/拖拽事件。
    pub fn mouse_move(&mut self, button: MouseButton, position: DVec2) {
        let time = self.current_time;
        let drag_type = match button {
            MouseButton::Left => CameraEventType::LeftDrag,
            MouseButton::Right => CameraEventType::RightDrag,
            MouseButton::Middle => CameraEventType::MiddleDrag,
        };
        self.get_movement_mut(drag_type).drag(position, time);
    }

    /// 记录一次滚轮滚动事件。
    pub fn wheel(&mut self, delta: f64) {
        let time = self.current_time;
        self.get_movement_mut(CameraEventType::Wheel).wheel(delta, time);
    }

    /// 检查某个特定事件类型当前是否在移动。
    pub fn is_moving(&self, event_type: CameraEventType) -> bool {
        self.get_movement(event_type).is_some_and(|m| m.is_moving)
    }

    /// 检查某个按钮当前是否处于按下状态。
    pub fn is_button_down(&self, button: MouseButton) -> bool {
        let drag_type = match button {
            MouseButton::Left => CameraEventType::LeftDrag,
            MouseButton::Right => CameraEventType::RightDrag,
            MouseButton::Middle => CameraEventType::MiddleDrag,
        };
        self.get_movement(drag_type).is_some_and(|m| m.is_button_down)
    }

    /// 获取某个事件类型的移动增量。
    pub fn get_movement_delta(&self, event_type: CameraEventType) -> DVec2 {
        self.get_movement(event_type).map_or(DVec2::ZERO, |m| m.movement)
    }

    // ========================================================================
    // 捻合（触控）聚合
    // ========================================================================

    /// 开始一个双指捻合手势。
    ///
    /// 捻合起始：由初始的手指位置播种距离、角度与高度
    /// 的起始与结束（二者相同），并标记手势开始。
    pub fn pinch_start(&mut self, finger1: DVec2, finger2: DVec2) {
        let (distance, angle, height) = pinch_metrics(finger1, finger2);
        let midpoint = (finger1 + finger2) * 0.5;
        self.pinching = true;
        self.pinch_update_pending = true;
        self.pinch = PinchMovement {
            distance: StartEnd {
                start_position: DVec2::new(0.0, distance),
                end_position: DVec2::new(0.0, distance),
            },
            angle_and_height: StartEnd {
                start_position: DVec2::new(angle, height),
                end_position: DVec2::new(angle, height),
            },
            midpoint: StartEnd {
                start_position: midpoint,
                end_position: midpoint,
            },
            prev_angle: angle,
        };
        let time = self.current_time;
        self.get_movement_mut(CameraEventType::Pinch)
            .button_down(midpoint, time);
    }

    /// 用当前手指位置更新正在进行的捻合。
    ///
    /// 捻合更新遵循每帧重新播种的约定：一帧
    /// 的首次移动重新播种起始和 `prevAngle`，后续移动聚合到结束，
    /// 且角度会回绕以保持与 `prevAngle` 相差在 `π` 以内，从而不会在 360° 处翻转。
    pub fn pinch_move(&mut self, finger1: DVec2, finger2: DVec2) {
        if !self.pinching {
            return;
        }
        let (distance, angle, height) = pinch_metrics(finger1, finger2);
        let midpoint = (finger1 + finger2) * 0.5;

        if self.pinch_update_pending {
            self.pinch.distance.start_position = DVec2::new(0.0, distance);
            self.pinch.angle_and_height.start_position = DVec2::new(angle, height);
            self.pinch.midpoint.start_position = midpoint;
            self.pinch.prev_angle = angle;
            self.pinch_update_pending = false;
        }
        self.pinch.distance.end_position = DVec2::new(0.0, distance);
        self.pinch.angle_and_height.end_position = DVec2::new(angle, height);
        self.pinch.midpoint.end_position = midpoint;

        // 防翻转回绕：把角度约束在 prev 的 ±π 内。
        let mut wrapped = angle;
        let prev = self.pinch.prev_angle;
        let two_pi = std::f64::consts::TAU;
        while wrapped >= prev + std::f64::consts::PI {
            wrapped -= two_pi;
        }
        while wrapped < prev - std::f64::consts::PI {
            wrapped += two_pi;
        }
        self.pinch.angle_and_height.end_position.x = wrapped;

        let time = self.current_time;
        self.get_movement_mut(CameraEventType::Pinch)
            .drag((finger1 + finger2) * 0.5, time);
    }

    /// 结束捻合手势。
    ///
    /// 清除进行标志并抬起捻合槽位。
    pub fn pinch_end(&mut self) {
        self.pinching = false;
        self.pinch_update_pending = false;
        let time = self.current_time;
        self.get_movement_mut(CameraEventType::Pinch).button_up(time);
    }

    /// 捻合手势当前是否正在进行。
    pub fn is_pinching(&self) -> bool {
        self.pinching
    }

    /// 捻合事件槽位是否设置了 `is_button_down`（即处于
    /// `pinch_start` 与 `pinch_end` 之间）。
    pub fn is_button_down_pinch(&self) -> bool {
        self.get_movement(CameraEventType::Pinch).is_some_and(|m| m.is_button_down)
    }

    /// 本帧聚合后的捻合移动。
    pub fn pinch(&self) -> &PinchMovement {
        &self.pinch
    }

    /// 本帧手指间距的变化（像素）；驱动缩放。
    pub fn pinch_distance_delta(&self) -> f64 {
        self.pinch.distance.end_position.y - self.pinch.distance.start_position.y
    }

    /// 本帧手指角度的变化（弧度）；驱动旋转。
    pub fn pinch_angle_delta(&self) -> f64 {
        self.pinch.angle_and_height.end_position.x - self.pinch.prev_angle
    }

    /// 本帧两指中点的变化（像素）；驱动双指
    /// **拖拽 → 平移** 手势。与距离/角度增量一样，它每帧
    /// 重新播种，因此它是两指共同的逐帧平移，而非自 `pinch_start`
    /// 以来的累积偏移。
    pub fn pinch_midpoint_delta(&self) -> DVec2 {
        self.pinch.midpoint.end_position - self.pinch.midpoint.start_position
    }

    /// 旋转增量（以像素计），采用 `(-angle * canvas_width) / 12`
    /// 的缩放约定：角度负号给出与手指转动相反的滚转，
    /// 除 12 为经验敏感度系数。画布宽度由适配层
    /// 边界提供，以使领域保持与分辨率无关。
    pub fn pinch_twist_pixels(&self, canvas_width: f64) -> f64 {
        (-self.pinch_angle_delta() * canvas_width) / 12.0
    }

    // ========================================================================
    // 语义手势增量（为控制器的 spin / look 动作提供输入）
    // ========================================================================

    /// 聚合的旋转手势增量（左键拖拽，像素）。
    ///
    /// 为 [`crate::camera_controller::CameraController::spin`] 提供输入，对应
    /// 默认的左键拖拽旋转绑定；左键拖拽究竟
    /// 变成旋转、平移还是环视，由控制器（基于拾取）决定，
    /// 而非聚合器。
    pub fn spin_delta(&self) -> DVec2 {
        self.get_movement_delta(CameraEventType::LeftDrag)
    }

    /// 聚合的环视手势增量（右键拖拽，像素）。
    ///
    /// 为 [`crate::camera_controller::CameraController::look`] 提供输入。该绑定是
    /// 移植默认值；应用可重新映射。
    pub fn look_delta(&self) -> DVec2 {
        self.get_movement_delta(CameraEventType::RightDrag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregate_movement_new() {
        let movement = AggregateMovement::new();
        assert!(!movement.is_button_down);
        assert!(!movement.is_moving);
        assert_eq!(movement.movement, DVec2::ZERO);
    }

    #[test]
    fn test_button_down_up() {
        let mut movement = AggregateMovement::new();
        movement.button_down(DVec2::new(100.0, 200.0), 0.0);
        assert!(movement.is_button_down);
        assert_eq!(movement.start_position, DVec2::new(100.0, 200.0));

        movement.button_up(1.0);
        assert!(!movement.is_button_down);
    }

    #[test]
    fn test_drag() {
        let mut movement = AggregateMovement::new();
        movement.button_down(DVec2::new(100.0, 100.0), 0.0);
        movement.drag(DVec2::new(150.0, 120.0), 0.5);

        assert!(movement.is_moving);
        assert_eq!(movement.movement, DVec2::new(50.0, 20.0));
    }

    #[test]
    fn test_wheel() {
        let mut movement = AggregateMovement::new();
        movement.wheel(120.0, 0.0);
        assert!(movement.is_moving);
        assert!((movement.movement.y - 120.0).abs() < 1e-10);
    }

    #[test]
    fn test_event_aggregator_basic() {
        let mut agg = CameraEventAggregator::new();
        agg.reset(0.0);

        agg.button_down(MouseButton::Left, DVec2::new(100.0, 100.0));
        assert!(agg.is_button_down(MouseButton::Left));
        assert!(!agg.is_button_down(MouseButton::Right));

        agg.mouse_move(MouseButton::Left, DVec2::new(150.0, 130.0));
        assert!(agg.is_moving(CameraEventType::LeftDrag));

        let delta = agg.get_movement_delta(CameraEventType::LeftDrag);
        assert!((delta.x - 50.0).abs() < 1e-10);
        assert!((delta.y - 30.0).abs() < 1e-10);
    }

    #[test]
    fn test_event_aggregator_reset() {
        let mut agg = CameraEventAggregator::new();
        agg.reset(0.0);

        agg.button_down(MouseButton::Left, DVec2::new(100.0, 100.0));
        agg.mouse_move(MouseButton::Left, DVec2::new(200.0, 200.0));

        // 为新一帧重置
        agg.reset(1.0 / 60.0);
        assert!(!agg.is_moving(CameraEventType::LeftDrag));
        // 按钮应仍处于按下状态
        assert!(agg.is_button_down(MouseButton::Left));
    }

    #[test]
    fn test_event_aggregator_wheel() {
        let mut agg = CameraEventAggregator::new();
        agg.reset(0.0);

        agg.wheel(-120.0);
        assert!(agg.is_moving(CameraEventType::Wheel));
        let delta = agg.get_movement_delta(CameraEventType::Wheel);
        assert!((delta.y - (-120.0)).abs() < 1e-10);
    }

    #[test]
    fn test_multiple_buttons() {
        let mut agg = CameraEventAggregator::new();
        agg.reset(0.0);

        agg.button_down(MouseButton::Left, DVec2::new(0.0, 0.0));
        agg.button_down(MouseButton::Right, DVec2::new(100.0, 100.0));

        assert!(agg.is_button_down(MouseButton::Left));
        assert!(agg.is_button_down(MouseButton::Right));
        assert!(!agg.is_button_down(MouseButton::Middle));

        agg.button_up(MouseButton::Left);
        assert!(!agg.is_button_down(MouseButton::Left));
        assert!(agg.is_button_down(MouseButton::Right));
    }

    #[test]
    fn test_pinch_start_move_end_lifecycle() {
        let mut agg = CameraEventAggregator::new();
        agg.reset(0.0);
        assert!(!agg.is_pinching());

        agg.pinch_start(DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0));
        assert!(agg.is_pinching());
        assert!(agg.is_button_down_pinch());

        agg.pinch_end();
        assert!(!agg.is_pinching());
    }

    #[test]
    fn test_pinch_zoom_distance_delta() {
        let mut agg = CameraEventAggregator::new();
        agg.reset(0.0);
        agg.pinch_start(DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0));
        // 首次移动播种每帧的起始；第二次聚合到结束。
        agg.pinch_move(DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0));
        agg.pinch_move(DVec2::new(0.0, 0.0), DVec2::new(200.0, 0.0));
        assert!((agg.pinch_distance_delta() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn test_pinch_angle_delta_drives_twist() {
        let mut agg = CameraEventAggregator::new();
        agg.reset(0.0);
        agg.pinch_start(DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0));
        agg.pinch_move(DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0)); // 播种角度 0
        agg.pinch_move(DVec2::new(0.0, 0.0), DVec2::new(0.0, 100.0)); // 旋转到 90°
        assert!((agg.pinch_angle_delta() - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        // 像素缩放采用 (-angle * width) / 12 的经验系数。
        let expected = (-std::f64::consts::FRAC_PI_2 * 1200.0) / 12.0;
        assert!((agg.pinch_twist_pixels(1200.0) - expected).abs() < 1e-9);
    }

    #[test]
    fn test_pinch_angle_anti_flip_over_360() {
        let mut agg = CameraEventAggregator::new();
        agg.reset(0.0);
        agg.pinch_start(DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0));
        // 将 prev_angle 播种到略低于 +π。
        agg.pinch_move(DVec2::new(0.0, 0.0), DVec2::new(-100.0, 1.0));
        // 越过到略高于 -π；回绕使增量保持微小而非 ~2π。
        agg.pinch_move(DVec2::new(0.0, 0.0), DVec2::new(-100.0, -1.0));
        let delta = agg.pinch_angle_delta();
        assert!(delta.abs() < 0.1, "anti-flip failed, delta = {delta}");
    }

    #[test]
    fn test_pinch_midpoint_delta_drives_translate() {
        let mut agg = CameraEventAggregator::new();
        agg.reset(0.0);
        agg.pinch_start(DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0));
        // 首次移动播种每帧的中点起始；第二次延长结束。
        // 两指都向右漂 50 px → 中点增量 = (+50, 0)。
        agg.pinch_move(DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0));
        agg.pinch_move(DVec2::new(50.0, 0.0), DVec2::new(150.0, 0.0));
        let mid = agg.pinch_midpoint_delta();
        assert!((mid.x - 50.0).abs() < 1e-9, "midpoint x delta = {}", mid.x);
        assert!(mid.y.abs() < 1e-9);
    }

    #[test]
    fn test_pinch_midpoint_delta_re_seeds_each_frame() {
        let mut agg = CameraEventAggregator::new();
        agg.pinch_start(DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0));
        // 帧 1：中点向 +30 x 漂移。
        agg.reset(0.0);
        agg.pinch_move(DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0));
        agg.pinch_move(DVec2::new(30.0, 0.0), DVec2::new(130.0, 0.0));
        assert!((agg.pinch_midpoint_delta().x - 30.0).abs() < 1e-9);
        // 帧 2 重新播种：漂移从帧 2 自身的起始量起，而非
        // 从 pinch_start 累积。
        agg.reset(1.0 / 60.0);
        agg.pinch_move(DVec2::new(30.0, 0.0), DVec2::new(130.0, 0.0));
        agg.pinch_move(DVec2::new(30.0, 40.0), DVec2::new(130.0, 40.0));
        let mid = agg.pinch_midpoint_delta();
        assert!(mid.x.abs() < 1e-9, "x re-seeded to 0, got {}", mid.x);
        assert!((mid.y - 40.0).abs() < 1e-9, "y delta = {}", mid.y);
    }

    #[test]
    fn test_pinch_move_ignored_without_start() {
        let mut agg = CameraEventAggregator::new();
        agg.reset(0.0);
        agg.pinch_move(DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0));
        assert!(!agg.is_pinching());
        assert!((agg.pinch_distance_delta()).abs() < 1e-12);
    }

    #[test]
    fn test_spin_delta_reads_left_drag() {
        let mut agg = CameraEventAggregator::new();
        agg.reset(0.0);
        agg.button_down(MouseButton::Left, DVec2::new(0.0, 0.0));
        agg.mouse_move(MouseButton::Left, DVec2::new(30.0, 40.0));
        let delta = agg.spin_delta();
        assert!((delta.x - 30.0).abs() < 1e-9);
        assert!((delta.y - 40.0).abs() < 1e-9);
    }

    #[test]
    fn test_look_delta_reads_right_drag() {
        let mut agg = CameraEventAggregator::new();
        agg.reset(0.0);
        agg.button_down(MouseButton::Right, DVec2::new(0.0, 0.0));
        agg.mouse_move(MouseButton::Right, DVec2::new(10.0, -5.0));
        let delta = agg.look_delta();
        assert!((delta.x - 10.0).abs() < 1e-9);
        assert!((delta.y - (-5.0)).abs() < 1e-9);
    }
}
