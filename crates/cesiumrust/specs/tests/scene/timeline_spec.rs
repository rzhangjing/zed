//! Widgets/Animation/AnimationViewModel + Timeline → Rust 集成测试。
//!
//! 参考 CesiumJS：
//! - Widgets/Animation/AnimationViewModel
//! - Widgets/Timeline/Timeline
//!
//! A 类测试：AnimationController play/pause/reverse/stop/tick/loop/seek、
//! 移动环、进度、TimelineConfig、SpeedPreset。
//! C 类省略：DOM 元素、SVG 渲染、拖拽事件。

use cesium_animation::timeline::{AnimationController, SpeedPreset, TimelineConfig};
use cesium_time::clock::Clock;
use cesium_time::julian_date::JulianDate;

fn make_clock() -> Clock {
    let start = JulianDate::from_date_components(2024, 6, 1, 0, 0, 0, 0.0);
    let stop = start.add_seconds(3600.0); // 1 小时时长
    Clock::new(start, stop, start)
}

fn make_controller() -> AnimationController {
    AnimationController::new(make_clock())
}

// === TimelineConfig ===

#[test]
fn timeline_config_duration() {
    let start = JulianDate::from_date_components(2024, 1, 1, 0, 0, 0, 0.0);
    let end = start.add_seconds(7200.0);
    let config = TimelineConfig::new(start, end);
    assert!((config.duration_seconds() - 7200.0).abs() < 1e-10);
    assert!(config.visible);
}

#[test]
fn timeline_config_seconds_per_pixel() {
    let start = JulianDate::from_date_components(2024, 1, 1, 0, 0, 0, 0.0);
    let end = start.add_seconds(1000.0);
    let config = TimelineConfig::new(start, end);
    // 默认：duration / 1000 像素
    assert!((config.seconds_per_pixel - 1.0).abs() < 1e-10);
}

// === AnimationController 创建 ===

#[test]
fn controller_default_state() {
    let controller = make_controller();
    assert!(controller.paused);
    assert!((controller.speed_multiplier - 1.0).abs() < 1e-10);
    assert!(controller.looping);
    assert!((controller.shuttle_ring_angle - 0.0).abs() < 1e-10);
}

// === 播放 / 暂停 / 倒放 ===

#[test]
fn controller_play() {
    let mut controller = make_controller();
    controller.play();
    assert!(!controller.paused);
    assert!(controller.is_playing_forward());
    assert!(!controller.is_playing_reverse());
}

#[test]
fn controller_pause() {
    let mut controller = make_controller();
    controller.play();
    controller.pause();
    assert!(controller.paused);
    assert!(!controller.is_playing_forward());
}

#[test]
fn controller_play_reverse() {
    let mut controller = make_controller();
    controller.play_reverse();
    assert!(!controller.paused);
    assert!(controller.is_playing_reverse());
    assert!(!controller.is_playing_forward());
    assert!(controller.speed_multiplier < 0.0);
}

#[test]
fn controller_stop_resets_to_start() {
    let mut controller = make_controller();
    let start = controller.clock.start_time;
    controller.play();
    controller.tick(100.0);
    controller.stop();
    assert!(controller.paused);
    assert_eq!(controller.clock.current_time, start);
}

// === Tick ===

#[test]
fn controller_tick_advances_time() {
    let mut controller = make_controller();
    let start = controller.clock.current_time;
    controller.play();
    let new_time = controller.tick(1.0);
    let elapsed = new_time.seconds_difference(&start);
    assert!((elapsed - 1.0).abs() < 1e-10);
}

#[test]
fn controller_tick_with_speed_multiplier() {
    let mut controller = make_controller();
    let start = controller.clock.current_time;
    controller.set_speed(60.0);
    controller.play();
    let new_time = controller.tick(1.0); // 以 60x 过 1 个真实秒
    let elapsed = new_time.seconds_difference(&start);
    assert!((elapsed - 60.0).abs() < 1e-10);
}

#[test]
fn controller_tick_paused_no_change() {
    let mut controller = make_controller();
    let start = controller.clock.current_time;
    // 默认已暂停
    let result = controller.tick(10.0);
    assert_eq!(result, start);
}

#[test]
fn controller_tick_reverse() {
    let mut controller = make_controller();
    // 先移到中间
    controller.seek_fraction(0.5);
    let mid = controller.clock.current_time;
    controller.play_reverse();
    let new_time = controller.tick(1.0);
    // 应已向后移动
    assert!(new_time.less_than(&mid));
}

// === 循环 ===

#[test]
fn controller_loop_wraps_around() {
    let mut controller = make_controller();
    let start = controller.clock.start_time;
    controller.looping = true;
    controller.play();
    // 越过终点前进（3600 + 100 = 3700 秒）
    let new_time = controller.tick(3700.0);
    let elapsed = new_time.seconds_difference(&start);
    assert!((elapsed - 100.0).abs() < 1e-10);
}

#[test]
fn controller_no_loop_clamps_at_end() {
    let mut controller = make_controller();
    let stop = controller.clock.stop_time;
    controller.looping = false;
    controller.play();
    let new_time = controller.tick(5000.0);
    assert_eq!(new_time, stop);
    assert!(controller.paused); // 自动暂停
}

#[test]
fn controller_no_loop_clamps_at_start_reverse() {
    let mut controller = make_controller();
    let start = controller.clock.start_time;
    controller.looping = false;
    controller.play_reverse();
    let new_time = controller.tick(100.0); // 倒放过起点
    assert_eq!(new_time, start);
    assert!(controller.paused);
}

// === 定位 ===

#[test]
fn controller_seek() {
    let mut controller = make_controller();
    let start = controller.clock.start_time;
    let target = start.add_seconds(1800.0);
    controller.seek(target);
    assert_eq!(controller.clock.current_time, target);
}

#[test]
fn controller_seek_fraction() {
    let mut controller = make_controller();
    let start = controller.clock.start_time;
    controller.seek_fraction(0.5);
    let elapsed = controller.clock.current_time.seconds_difference(&start);
    assert!((elapsed - 1800.0).abs() < 1e-10);
}

#[test]
fn controller_seek_fraction_clamped() {
    let mut controller = make_controller();
    let stop = controller.clock.stop_time;
    controller.seek_fraction(2.0); // > 1.0 被钳制
    assert_eq!(controller.clock.current_time, stop);
}

// === 进度 ===

#[test]
fn controller_progress() {
    let mut controller = make_controller();
    controller.seek_fraction(0.25);
    assert!((controller.progress() - 0.25).abs() < 1e-10);
}

#[test]
fn controller_progress_at_start() {
    let controller = make_controller();
    assert!((controller.progress() - 0.0).abs() < 1e-10);
}

// === 移动环 ===

#[test]
fn controller_shuttle_ring_positive() {
    let mut controller = make_controller();
    controller.set_shuttle_ring(0.5);
    assert!(!controller.paused);
    assert!(controller.speed_multiplier > 0.0);
    assert!((controller.shuttle_ring_angle - 0.5).abs() < 1e-10);
}

#[test]
fn controller_shuttle_ring_zero_pauses() {
    let mut controller = make_controller();
    controller.set_shuttle_ring(0.5);
    controller.set_shuttle_ring(0.0);
    assert!(controller.paused);
}

#[test]
fn controller_shuttle_ring_clamped() {
    let mut controller = make_controller();
    controller.set_shuttle_ring(5.0);
    assert!((controller.shuttle_ring_angle - 1.0).abs() < 1e-10);
    controller.set_shuttle_ring(-5.0);
    assert!((controller.shuttle_ring_angle - (-1.0)).abs() < 1e-10);
}

// === SpeedPreset ===

#[test]
fn speed_preset_multipliers() {
    assert!((SpeedPreset::RealTime.multiplier() - 1.0).abs() < 1e-10);
    assert!((SpeedPreset::Fast2x.multiplier() - 2.0).abs() < 1e-10);
    assert!((SpeedPreset::Fast5x.multiplier() - 5.0).abs() < 1e-10);
    assert!((SpeedPreset::Fast10x.multiplier() - 10.0).abs() < 1e-10);
    assert!((SpeedPreset::Fast60x.multiplier() - 60.0).abs() < 1e-10);
    assert!((SpeedPreset::Fast3600x.multiplier() - 3600.0).abs() < 1e-10);
    assert!((SpeedPreset::Fast86400x.multiplier() - 86400.0).abs() < 1e-10);
}

// === set_speed ===

#[test]
fn controller_set_speed_while_playing() {
    let mut controller = make_controller();
    controller.play();
    controller.set_speed(10.0);
    assert!((controller.speed_multiplier - 10.0).abs() < 1e-10);
    assert!((controller.clock.multiplier - 10.0).abs() < 1e-10);
}

#[test]
fn controller_set_speed_while_paused() {
    let mut controller = make_controller();
    // 默认暂停
    controller.set_speed(100.0);
    assert!((controller.speed_multiplier - 100.0).abs() < 1e-10);
    // 暂停时时钟倍率不应改变
    assert!((controller.clock.multiplier - 1.0).abs() < 1e-10);
}
