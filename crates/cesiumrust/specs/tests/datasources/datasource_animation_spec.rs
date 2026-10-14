//! 参考自 CesiumJS PathVisualizerSpec + AnimationViewModelSpec 的测试
//! A 类逻辑：AnimationClock、interpolate_position、compute_path

use cesium_datasource::animation::{
    interpolate_position, AnimationClock, InterpolationAlgorithm, Keyframe,
};

// ===== AnimationClock =====

#[test]
fn test_clock_new_defaults() {
    let clock = AnimationClock::new(0.0, 10.0);
    assert_eq!(clock.start_time, 0.0);
    assert_eq!(clock.stop_time, 10.0);
    assert_eq!(clock.current_time, 0.0);
    assert_eq!(clock.multiplier, 1.0);
    assert!(!clock.playing);
    assert!(clock.looping);
}

#[test]
fn test_clock_tick_advances_time() {
    let mut clock = AnimationClock::new(0.0, 10.0);
    clock.playing = true;
    clock.tick(1.0);
    assert!((clock.current_time - 1.0).abs() < 1e-10);
    clock.tick(2.0);
    assert!((clock.current_time - 3.0).abs() < 1e-10);
}

#[test]
fn test_clock_tick_not_playing() {
    let mut clock = AnimationClock::new(0.0, 10.0);
    clock.tick(5.0);
    assert_eq!(clock.current_time, 0.0);
}

#[test]
fn test_clock_tick_with_multiplier() {
    let mut clock = AnimationClock::new(0.0, 100.0);
    clock.playing = true;
    clock.multiplier = 2.0;
    clock.tick(1.0);
    assert!((clock.current_time - 2.0).abs() < 1e-10);
}

#[test]
fn test_clock_loop_wraps_around() {
    let mut clock = AnimationClock::new(0.0, 10.0);
    clock.playing = true;
    clock.looping = true;
    clock.tick(12.0);
    // 应回绕：12 % 10 = 2
    assert!((clock.current_time - 2.0).abs() < 1e-10);
}

#[test]
fn test_clock_no_loop_stops_at_end() {
    let mut clock = AnimationClock::new(0.0, 10.0);
    clock.playing = true;
    clock.looping = false;
    clock.tick(12.0);
    assert!((clock.current_time - 10.0).abs() < 1e-10);
    assert!(!clock.playing);
}

#[test]
fn test_clock_progress() {
    let mut clock = AnimationClock::new(0.0, 10.0);
    assert!((clock.progress() - 0.0).abs() < 1e-10);
    clock.current_time = 5.0;
    assert!((clock.progress() - 0.5).abs() < 1e-10);
    clock.current_time = 10.0;
    assert!((clock.progress() - 1.0).abs() < 1e-10);
}

#[test]
fn test_clock_reset_and_seek() {
    let mut clock = AnimationClock::new(0.0, 10.0);
    clock.current_time = 7.0;
    clock.reset();
    assert_eq!(clock.current_time, 0.0);

    clock.seek(5.0);
    assert_eq!(clock.current_time, 5.0);

    // Seek 钳制
    clock.seek(20.0);
    assert_eq!(clock.current_time, 10.0);
    clock.seek(-5.0);
    assert_eq!(clock.current_time, 0.0);
}

// ===== interpolate_position =====

#[test]
fn test_interpolate_empty_returns_none() {
    let result = interpolate_position(&[], 0.0, InterpolationAlgorithm::Linear);
    assert!(result.is_none());
}

#[test]
fn test_interpolate_single_keyframe() {
    let kf = vec![Keyframe { time: 0.0, value: [1.0, 2.0, 3.0] }];
    let result = interpolate_position(&kf, 5.0, InterpolationAlgorithm::Linear);
    assert_eq!(result, Some([1.0, 2.0, 3.0]));
}

#[test]
fn test_interpolate_linear_midpoint() {
    let kf = vec![
        Keyframe { time: 0.0, value: [0.0, 0.0, 0.0] },
        Keyframe { time: 10.0, value: [10.0, 20.0, 30.0] },
    ];
    let result = interpolate_position(&kf, 5.0, InterpolationAlgorithm::Linear).unwrap();
    assert!((result[0] - 5.0).abs() < 1e-10);
    assert!((result[1] - 10.0).abs() < 1e-10);
    assert!((result[2] - 15.0).abs() < 1e-10);
}

#[test]
fn test_interpolate_before_first_and_after_last() {
    let kf = vec![
        Keyframe { time: 1.0, value: [1.0, 1.0, 1.0] },
        Keyframe { time: 5.0, value: [5.0, 5.0, 5.0] },
    ];
    // 首个之前
    let result = interpolate_position(&kf, 0.0, InterpolationAlgorithm::Linear).unwrap();
    assert_eq!(result, [1.0, 1.0, 1.0]);
    // 末个之后
    let result = interpolate_position(&kf, 10.0, InterpolationAlgorithm::Linear).unwrap();
    assert_eq!(result, [5.0, 5.0, 5.0]);
}

#[test]
fn test_interpolate_hermite_smoothstep() {
    let kf = vec![
        Keyframe { time: 0.0, value: [0.0, 0.0, 0.0] },
        Keyframe { time: 1.0, value: [1.0, 1.0, 1.0] },
    ];
    // 在 t=0.5，smoothstep(0.5) = 3*0.25 - 2*0.125 = 0.75 - 0.25 = 0.5
    let result = interpolate_position(&kf, 0.5, InterpolationAlgorithm::Hermite).unwrap();
    assert!((result[0] - 0.5).abs() < 1e-10);

    // 在 t=0.25，smoothstep(0.25) = 3*0.0625 - 2*0.015625 = 0.1875 - 0.03125 = 0.15625
    let result = interpolate_position(&kf, 0.25, InterpolationAlgorithm::Hermite).unwrap();
    assert!((result[0] - 0.15625).abs() < 1e-10);
}

#[test]
fn test_interpolate_lagrange_quadratic() {
    // 一条直线上的三个点：Lagrange 应精确复现 linear
    let kf = vec![
        Keyframe { time: 0.0, value: [0.0, 0.0, 0.0] },
        Keyframe { time: 1.0, value: [1.0, 2.0, 3.0] },
        Keyframe { time: 2.0, value: [2.0, 4.0, 6.0] },
    ];
    let result = interpolate_position(&kf, 0.5, InterpolationAlgorithm::Lagrange).unwrap();
    assert!((result[0] - 0.5).abs() < 1e-8);
    assert!((result[1] - 1.0).abs() < 1e-8);
    assert!((result[2] - 1.5).abs() < 1e-8);
}
