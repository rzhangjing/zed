//! Clock 规格 - 参考自 Specs/Core/ClockSpec
//! 27 个原始 it() 块 → 已移植 16 个 A 类测试
//! 跳过 11 个 C 类：1 throws + 2 events(onStop) + 8 SYSTEM_CLOCK 模式（jasmine.clock 模拟）

use cesium_time::{Clock, ClockOptions, ClockRange, ClockStep, JulianDate};

// ============================================================================
// 构造函数测试（8）
// ============================================================================

#[test]
fn sets_default_parameters_when_constructed() {
    let clock = Clock::from_options(&ClockOptions::default());

    // stopTime = startTime + 1 day
    let expected_stop = clock.start_time.add_days(1.0);
    assert_eq!(clock.stop_time, expected_stop);
    // startTime == currentTime
    assert_eq!(clock.start_time, clock.current_time);
    // defaults
    assert_eq!(clock.multiplier, 1.0);
    assert_eq!(clock.clock_step, ClockStep::SystemClockMultiplier);
    assert_eq!(clock.clock_range, ClockRange::Unbounded);
    assert_eq!(clock.can_animate, true);
    assert_eq!(clock.should_animate, false);
}

#[test]
fn sets_provided_constructor_parameters_correctly() {
    let start = JulianDate::new(12.0, 0.0);
    let stop = JulianDate::new(112.0, 0.0);
    let current_time = JulianDate::new(13.0, 0.0);
    let step = ClockStep::TickDependent;
    let range = ClockRange::LoopStop;
    let multiplier = 1.5;

    let clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        stop_time: Some(stop),
        current_time: Some(current_time),
        clock_step: Some(step),
        multiplier: Some(multiplier),
        clock_range: Some(range),
        ..Default::default()
    });

    assert_eq!(clock.start_time, start);
    assert_eq!(clock.stop_time, stop);
    assert_eq!(clock.current_time, current_time);
    assert_eq!(clock.clock_step, step);
    assert_eq!(clock.clock_range, range);
    assert_eq!(clock.multiplier, multiplier);
    assert_eq!(clock.can_animate, true);
    assert_eq!(clock.should_animate, false);

    // canAnimate: false
    let clock = Clock::from_options(&ClockOptions {
        can_animate: Some(false),
        ..Default::default()
    });
    assert_eq!(clock.can_animate, false);

    // shouldAnimate: true
    let clock = Clock::from_options(&ClockOptions {
        should_animate: Some(true),
        ..Default::default()
    });
    assert_eq!(clock.should_animate, true);
}

#[test]
fn works_when_constructed_with_no_current_time_parameter() {
    let start = JulianDate::new(12.0, 0.0);
    let stop = JulianDate::new(112.0, 0.0);
    let step = ClockStep::TickDependent;
    let range = ClockRange::LoopStop;
    let multiplier = 1.5;

    let clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        stop_time: Some(stop),
        clock_step: Some(step),
        multiplier: Some(multiplier),
        clock_range: Some(range),
        ..Default::default()
    });

    assert_eq!(clock.start_time, start);
    assert_eq!(clock.stop_time, stop);
    // currentTime 默认为 startTime
    assert_eq!(clock.current_time, start);
    assert_eq!(clock.clock_step, step);
    assert_eq!(clock.clock_range, range);
    assert_eq!(clock.multiplier, multiplier);
    assert_eq!(clock.can_animate, true);
    assert_eq!(clock.should_animate, false);
}

#[test]
fn works_when_constructed_with_no_start_time_parameter() {
    let stop = JulianDate::new(112.0, 0.0);
    let current_time = JulianDate::new(13.0, 0.0);
    let step = ClockStep::TickDependent;
    let range = ClockRange::LoopStop;
    let multiplier = 1.5;

    let clock = Clock::from_options(&ClockOptions {
        stop_time: Some(stop),
        current_time: Some(current_time),
        clock_step: Some(step),
        multiplier: Some(multiplier),
        clock_range: Some(range),
        ..Default::default()
    });

    // startTime 默认为 currentTime
    assert_eq!(clock.start_time, current_time);
    assert_eq!(clock.stop_time, stop);
    assert_eq!(clock.current_time, current_time);
    assert_eq!(clock.clock_step, step);
    assert_eq!(clock.clock_range, range);
    assert_eq!(clock.multiplier, multiplier);
    assert_eq!(clock.can_animate, true);
    assert_eq!(clock.should_animate, false);
}

#[test]
fn works_when_constructed_with_no_start_time_or_stop_time() {
    let current_time = JulianDate::new(12.0, 0.0);
    let step = ClockStep::TickDependent;
    let range = ClockRange::LoopStop;
    let multiplier = 1.5;

    let clock = Clock::from_options(&ClockOptions {
        current_time: Some(current_time),
        clock_step: Some(step),
        multiplier: Some(multiplier),
        clock_range: Some(range),
        ..Default::default()
    });

    let expected_stop = current_time.add_days(1.0);
    // startTime 默认为 currentTime
    assert_eq!(clock.start_time, current_time);
    // stopTime 默认为 startTime + 1 天
    assert_eq!(clock.stop_time, expected_stop);
    assert_eq!(clock.current_time, current_time);
    assert_eq!(clock.clock_step, step);
    assert_eq!(clock.clock_range, range);
    assert_eq!(clock.multiplier, multiplier);
    assert_eq!(clock.can_animate, true);
    assert_eq!(clock.should_animate, false);
}

#[test]
fn works_when_constructed_with_no_start_time_or_current_time() {
    let stop = JulianDate::new(13.0, 0.0);
    let step = ClockStep::TickDependent;
    let range = ClockRange::LoopStop;
    let multiplier = 1.5;

    let clock = Clock::from_options(&ClockOptions {
        stop_time: Some(stop),
        clock_step: Some(step),
        multiplier: Some(multiplier),
        clock_range: Some(range),
        ..Default::default()
    });

    // currentTime 默认为 stopTime - 1 天
    let expected_start = stop.add_days(-1.0);
    assert_eq!(clock.start_time, expected_start);
    assert_eq!(clock.stop_time, stop);
    assert_eq!(clock.current_time, expected_start);
    assert_eq!(clock.clock_step, step);
    assert_eq!(clock.clock_range, range);
    assert_eq!(clock.multiplier, multiplier);
    assert_eq!(clock.can_animate, true);
    assert_eq!(clock.should_animate, false);
}

#[test]
fn works_when_constructed_with_no_current_time_or_stop_time() {
    let start = JulianDate::new(12.0, 0.0);
    let step = ClockStep::TickDependent;
    let range = ClockRange::LoopStop;
    let multiplier = 1.5;

    let clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        clock_step: Some(step),
        multiplier: Some(multiplier),
        clock_range: Some(range),
        ..Default::default()
    });

    let expected_stop = start.add_days(1.0);
    assert_eq!(clock.start_time, start);
    assert_eq!(clock.stop_time, expected_stop);
    // currentTime 默认为 startTime
    assert_eq!(clock.current_time, start);
    assert_eq!(clock.clock_step, step);
    assert_eq!(clock.clock_range, range);
    assert_eq!(clock.multiplier, multiplier);
    assert_eq!(clock.can_animate, true);
    assert_eq!(clock.should_animate, false);
}

#[test]
fn works_when_constructed_with_no_stop_time_parameter() {
    let start = JulianDate::new(12.0, 0.0);
    let current_time = JulianDate::new(12.0, 0.0);
    let step = ClockStep::TickDependent;
    let range = ClockRange::LoopStop;
    let multiplier = 1.5;

    let clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        current_time: Some(current_time),
        clock_step: Some(step),
        multiplier: Some(multiplier),
        clock_range: Some(range),
        ..Default::default()
    });

    let expected_stop = start.add_days(1.0);
    assert_eq!(clock.start_time, start);
    assert_eq!(clock.stop_time, expected_stop);
    assert_eq!(clock.current_time, current_time);
    assert_eq!(clock.clock_step, step);
    assert_eq!(clock.clock_range, range);
    assert_eq!(clock.multiplier, multiplier);
    assert_eq!(clock.can_animate, true);
    assert_eq!(clock.should_animate, false);
}

// ============================================================================
// TICK_DEPENDENT 模式测试（8）
// ============================================================================

#[test]
fn animates_forward_in_tick_dependent_mode() {
    let start = JulianDate::new(0.0, 0.0);
    let stop = JulianDate::new(1.0, 0.0);
    let current_time = JulianDate::new(0.5, 0.0);
    let multiplier = 1.5;

    let mut clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        stop_time: Some(stop),
        current_time: Some(current_time),
        clock_step: Some(ClockStep::TickDependent),
        multiplier: Some(multiplier),
        clock_range: Some(ClockRange::LoopStop),
        should_animate: Some(true),
        ..Default::default()
    });
    assert_eq!(clock.current_time, current_time);

    let mut expected = current_time.add_seconds(multiplier);
    let result = clock.tick(0.0);
    assert_eq!(result, expected);
    assert_eq!(clock.current_time, expected);

    expected = expected.add_seconds(multiplier);
    let result = clock.tick(0.0);
    assert_eq!(result, expected);
    assert_eq!(clock.current_time, expected);
}

#[test]
fn animates_backwards_in_tick_dependent_mode() {
    let start = JulianDate::new(0.0, 0.0);
    let stop = JulianDate::new(1.0, 0.0);
    let current_time = JulianDate::new(0.5, 0.0);
    let multiplier = -1.5;

    let mut clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        stop_time: Some(stop),
        current_time: Some(current_time),
        clock_step: Some(ClockStep::TickDependent),
        multiplier: Some(multiplier),
        clock_range: Some(ClockRange::LoopStop),
        should_animate: Some(true),
        ..Default::default()
    });
    assert_eq!(clock.current_time, current_time);

    let mut expected = current_time.add_seconds(multiplier);
    let result = clock.tick(0.0);
    assert_eq!(result, expected);
    assert_eq!(clock.current_time, expected);

    expected = expected.add_seconds(multiplier);
    let result = clock.tick(0.0);
    assert_eq!(result, expected);
    assert_eq!(clock.current_time, expected);
}

#[test]
fn animates_forwards_past_stop_time_in_unbounded_tick_dependent_mode() {
    let start = JulianDate::new(0.0, 0.0);
    let stop = JulianDate::new(1.0, 0.0);
    let current_time = stop;
    let multiplier = 1.5;

    let mut clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        stop_time: Some(stop),
        current_time: Some(current_time),
        clock_step: Some(ClockStep::TickDependent),
        multiplier: Some(multiplier),
        clock_range: Some(ClockRange::Unbounded),
        should_animate: Some(true),
        ..Default::default()
    });
    assert_eq!(clock.current_time, current_time);

    let mut expected = current_time.add_seconds(multiplier);
    let result = clock.tick(0.0);
    assert_eq!(result, expected);
    assert_eq!(clock.current_time, expected);

    expected = expected.add_seconds(multiplier);
    let result = clock.tick(0.0);
    assert_eq!(result, expected);
    assert_eq!(clock.current_time, expected);
}

#[test]
fn animates_backwards_past_start_time_in_unbounded_tick_dependent_mode() {
    let start = JulianDate::new(0.0, 0.0);
    let stop = JulianDate::new(1.0, 0.0);
    let current_time = start;
    let multiplier = -1.5;

    let mut clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        stop_time: Some(stop),
        current_time: Some(current_time),
        clock_step: Some(ClockStep::TickDependent),
        multiplier: Some(multiplier),
        clock_range: Some(ClockRange::Unbounded),
        should_animate: Some(true),
        ..Default::default()
    });
    assert_eq!(clock.current_time, current_time);

    let mut expected = current_time.add_seconds(multiplier);
    let result = clock.tick(0.0);
    assert_eq!(result, expected);
    assert_eq!(clock.current_time, expected);

    expected = expected.add_seconds(multiplier);
    let result = clock.tick(0.0);
    assert_eq!(result, expected);
    assert_eq!(clock.current_time, expected);
}

#[test]
fn loops_back_to_start_time_when_animating_forward_past_stop_in_loop_stop_tick_dependent_mode() {
    let start = JulianDate::new(0.0, 0.0);
    let stop = JulianDate::new(1.0, 0.0);
    let current_time = stop;
    let multiplier = 1.5;

    let mut clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        stop_time: Some(stop),
        current_time: Some(current_time),
        clock_step: Some(ClockStep::TickDependent),
        multiplier: Some(multiplier),
        clock_range: Some(ClockRange::LoopStop),
        should_animate: Some(true),
        ..Default::default()
    });
    assert_eq!(clock.current_time, current_time);

    // 第一次 tick：stop + 1.5 溢出 → 回绕到 start + 1.5
    let mut expected = start.add_seconds(multiplier);
    let result = clock.tick(0.0);
    assert_eq!(result, expected);
    assert_eq!(clock.current_time, expected);

    // 第二次 tick：(start + 1.5) + 1.5 = start + 3.0，溢出 → start + (3.0 - 1day_secs)
    // 但 1 天 = 86400 秒，因此 start + 3.0 < stop。无溢出。
    expected = expected.add_seconds(multiplier);
    let result = clock.tick(0.0);
    assert_eq!(result, expected);
    assert_eq!(clock.current_time, expected);
}

#[test]
fn stops_at_start_when_animating_backwards_past_start_in_loop_stop_tick_dependent_mode() {
    let start = JulianDate::new(0.0, 0.0);
    let stop = JulianDate::new(1.0, 0.0);
    let current_time = start;
    let multiplier = -100.0;

    let mut clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        stop_time: Some(stop),
        current_time: Some(current_time),
        clock_step: Some(ClockStep::TickDependent),
        multiplier: Some(multiplier),
        clock_range: Some(ClockRange::LoopStop),
        should_animate: Some(true),
        ..Default::default()
    });

    assert_eq!(clock.current_time, current_time);
    let result = clock.tick(0.0);
    assert_eq!(result, start);
    assert_eq!(clock.current_time, start);
}

#[test]
fn stops_at_stop_time_when_animating_forwards_past_stop_in_clamped_tick_dependent_mode() {
    let start = JulianDate::new(0.0, 0.0);
    let stop = JulianDate::new(1.0, 0.0);
    let current_time = stop;
    let multiplier = 100.0;

    let mut clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        stop_time: Some(stop),
        current_time: Some(current_time),
        clock_step: Some(ClockStep::TickDependent),
        multiplier: Some(multiplier),
        clock_range: Some(ClockRange::Clamped),
        should_animate: Some(true),
        ..Default::default()
    });

    assert_eq!(clock.current_time, current_time);
    let result = clock.tick(0.0);
    assert_eq!(result, stop);
    assert_eq!(clock.current_time, stop);
}

#[test]
fn stops_at_start_time_when_animating_backwards_past_start_in_clamped_tick_dependent_mode() {
    let start = JulianDate::new(0.0, 0.0);
    let stop = JulianDate::new(1.0, 0.0);
    let current_time = start;
    let multiplier = -100.0;

    let mut clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        stop_time: Some(stop),
        current_time: Some(current_time),
        clock_step: Some(ClockStep::TickDependent),
        multiplier: Some(multiplier),
        clock_range: Some(ClockRange::Clamped),
        should_animate: Some(true),
        ..Default::default()
    });

    assert_eq!(clock.current_time, current_time);
    let result = clock.tick(0.0);
    assert_eq!(result, start);
    assert_eq!(clock.current_time, start);
}

// ============================================================================
// SYSTEM_CLOCK_MULTIPLIER 模式测试（改编自 C 类 jasmine.clock 测试）
// ============================================================================

#[test]
fn uses_multiplier_in_system_clock_multiplier_mode() {
    // Adapted: instead of jasmine.clock().tick(1000), we pass delta_secs = 1.0
    let start = JulianDate::new(0.0, 0.0);
    let stop = JulianDate::new(1.0, 0.0);

    let mut clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        stop_time: Some(stop),
        current_time: Some(start),
        clock_step: Some(ClockStep::SystemClockMultiplier),
        multiplier: Some(2.0),
        should_animate: Some(true),
        ..Default::default()
    });

    // 第一次 tick，elapsed 为 0 → 不推进
    let time1 = clock.tick(0.0);
    assert_eq!(time1, start);

    // 第二次 tick，elapsed 为 1.0 秒 → 推进 2.0 * 1.0 = 2.0 秒
    let time2 = clock.tick(1.0);
    let expected = start.add_seconds(2.0);
    assert_eq!(time2, expected);
    assert_eq!(clock.current_time, expected);
}

#[test]
fn does_not_advance_if_should_animate_is_false() {
    let start = JulianDate::new(0.0, 0.0);
    let stop = JulianDate::new(1.0, 0.0);

    let mut clock = Clock::from_options(&ClockOptions {
        start_time: Some(start),
        stop_time: Some(stop),
        current_time: Some(start),
        clock_step: Some(ClockStep::SystemClockMultiplier),
        multiplier: Some(1.0),
        should_animate: Some(false),
        ..Default::default()
    });

    // shouldAnimate = false → 不推进
    let time1 = clock.tick(1.0);
    assert_eq!(time1, start);
    assert_eq!(clock.current_time, start);

    // 启用动画
    clock.should_animate = true;
    let time2 = clock.tick(1.0);
    let expected = start.add_seconds(1.0);
    assert_eq!(time2, expected);

    // 切换到 TICK_DEPENDENT
    clock.current_time = start;
    clock.clock_step = ClockStep::TickDependent;

    clock.should_animate = false;
    let time3 = clock.tick(0.0);
    assert_eq!(time3, start);
    assert_eq!(clock.current_time, start);

    clock.should_animate = true;
    let time4 = clock.tick(0.0);
    let expected = start.add_seconds(1.0); // multiplier = 1.0
    assert_eq!(time4, expected);
}
