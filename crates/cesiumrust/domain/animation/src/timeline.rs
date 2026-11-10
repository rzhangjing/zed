//! 时间轴与动画控制器。
//!
//! 封装时间轴的配置、可见范围与播放控制，将底层时钟的推进、
//! 循环回绕、边界夹取统一为一组面向播放头的高层操作。

use cesium_time::clock::Clock;
use cesium_time::julian_date::JulianDate;

/// 时间轴配置。
///
/// 描述一段可播放时间区间的起止、可见性与像素到秒的缩放比例。
#[derive(Debug, Clone)]
pub struct TimelineConfig {
    /// 时间轴起始时间。
    pub start_time: JulianDate,
    /// 时间轴结束时间。
    pub end_time: JulianDate,
    /// 时间轴是否可见。
    pub visible: bool,
    /// 缩放级别（每像素秒数）。
    pub seconds_per_pixel: f64,
}

impl TimelineConfig {
    /// 创建一个新的时间轴配置。
    pub fn new(start_time: JulianDate, end_time: JulianDate) -> Self {
        let duration = end_time.seconds_difference(&start_time);
        Self {
            start_time,
            end_time,
            visible: true,
            // 缩放默认把整段时长铺到约 1000px 宽，得每像素秒数
            seconds_per_pixel: duration / 1000.0, // 默认：1000px 宽
        }
    }

    /// 返回总时长（秒）。
    pub fn duration_seconds(&self) -> f64 {
        // 终点与起点的儒略日秒差即总时长
        self.end_time.seconds_difference(&self.start_time)
    }
}

/// 用于 UI 渲染的时间轴状态。
///
/// 快照当前播放头位置、可见窗口区间以及是否触达起/终点的边界标志。
#[derive(Debug, Clone)]
pub struct TimelineState {
    /// 当前时间位置。
    pub current_time: JulianDate,
    /// 可见起始时间。
    pub visible_start: JulianDate,
    /// 可见结束时间。
    pub visible_end: JulianDate,
    /// 播放头是否位于起点。
    pub at_start: bool,
    /// 播放头是否位于终点。
    pub at_end: bool,
}

/// 用于播放的动画控制器。
///
/// 封装播放/反向/暂停/停止、循环回绕与拨盘环调速等播放控制语义。
#[derive(Debug, Clone)]
pub struct AnimationController {
    /// 底层时钟。
    pub clock: Clock,
    /// 播放速度乘数。
    pub speed_multiplier: f64,
    /// 播放是否已暂停。
    pub paused: bool,
    /// 是否循环播放。
    pub looping: bool,
    /// 拨盘环角度（-1.0 到 1.0）。
    pub shuttle_ring_angle: f64,
}

impl AnimationController {
    /// 创建一个新的动画控制器。
    pub fn new(clock: Clock) -> Self {
        Self {
            clock,
            speed_multiplier: 1.0,
            paused: true,
            looping: true,
            shuttle_ring_angle: 0.0,
        }
    }

    /// 向前播放动画。
    pub fn play(&mut self) {
        // 恢复播放，并把时钟 multiplier 设为速度绝对值（保证正向）
        self.paused = false;
        self.clock.multiplier = self.speed_multiplier.abs();
    }

    /// 反向播放动画。
    pub fn play_reverse(&mut self) {
        // 反向：把速度乘数取负，并同步到底层时钟的 multiplier
        self.paused = false;
        self.speed_multiplier = -self.speed_multiplier.abs();
        self.clock.multiplier = self.speed_multiplier;
    }

    /// 暂停动画。
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// 停止并重置到起点。
    pub fn stop(&mut self) {
        // 停止：置为暂停并把播放头复位到起始时间
        self.paused = true;
        self.clock.current_time = self.clock.start_time;
    }

    /// 设置速度乘数。
    pub fn set_speed(&mut self, multiplier: f64) {
        self.speed_multiplier = multiplier;
        // 仅在非暂停时把新速度映射到时钟；若处于倒放则保持负号
        if !self.paused {
            self.clock.multiplier = if self.clock.multiplier >= 0.0 {
                multiplier.abs()
            } else {
                -multiplier.abs()
            };
        }
    }

    /// 设置拨盘环角度（-1.0 到 1.0）。
    pub fn set_shuttle_ring(&mut self, angle: f64) {
        self.shuttle_ring_angle = angle.clamp(-1.0, 1.0);
        // 将角度映射为速度：0 = 暂停，±1 = 最大速度
        let max_speed = 1000.0; // 满偏时 1000 倍实时
        self.speed_multiplier = self.shuttle_ring_angle * max_speed;
        // 拨盘接近零位（|angle|<0.01）视为松开，自动进入暂停
        if self.shuttle_ring_angle.abs() < 0.01 {
            self.paused = true;
        } else {
            self.paused = false;
            self.clock.multiplier = self.speed_multiplier;
        }
    }

    /// 将动画推进 delta 秒。
    pub fn tick(&mut self, delta_secs: f64) -> JulianDate {
        // 暂停时时间不推进，原样返回当前时间
        if self.paused {
            return self.clock.current_time;
        }

        // 实际推进量 = 帧间隔 × 速度乘数（乘数为负时实现倒放）
        let effective_delta = delta_secs * self.speed_multiplier;
        let new_time = self.clock.current_time.add_seconds(effective_delta);

        // 循环模式：用欧几里得取模把超出总时长的秒数回绕到起点之后
        if self.looping {
            let duration = self.clock.stop_time.seconds_difference(&self.clock.start_time);
            if duration > 0.0 {
                let elapsed = new_time.seconds_difference(&self.clock.start_time);
                let wrapped = elapsed.rem_euclid(duration);
                self.clock.current_time = self.clock.start_time.add_seconds(wrapped);
            } else {
                self.clock.current_time = new_time;
            }
        } else {
            // 非循环：把时间夹取到 [起点, 终点]，触达边界则自动暂停
            if new_time.less_than(&self.clock.start_time) {
                self.clock.current_time = self.clock.start_time;
                self.paused = true;
            } else if self.clock.stop_time.less_than(&new_time) {
                self.clock.current_time = self.clock.stop_time;
                self.paused = true;
            } else {
                self.clock.current_time = new_time;
            }
        }

        self.clock.current_time
    }

    /// 定位到特定时间。
    pub fn seek(&mut self, time: JulianDate) {
        // 直接把播放头设为给定时间，不改动暂停/速度等播放状态
        self.clock.current_time = time;
    }

    /// 定位到时间轴的一个分数位置（0.0 到 1.0）。
    pub fn seek_fraction(&mut self, fraction: f64) {
        // 偏移量 = 总时长 × clamp(fraction, 0, 1)，再叠加到起点
        let duration = self.clock.stop_time.seconds_difference(&self.clock.start_time);
        let offset = duration * fraction.clamp(0.0, 1.0);
        self.clock.current_time = self.clock.start_time.add_seconds(offset);
    }

    /// 以分数（0.0 到 1.0）返回当前进度。
    pub fn progress(&self) -> f64 {
        // 进度 = 已用秒数 / 总时长，夹取到 [0,1]；总时长非正时返回 0
        let duration = self.clock.stop_time.seconds_difference(&self.clock.start_time);
        if duration <= 0.0 {
            return 0.0;
        }
        let elapsed = self.clock.current_time.seconds_difference(&self.clock.start_time);
        (elapsed / duration).clamp(0.0, 1.0)
    }

    /// 若正在向前播放则返回 true。
    pub fn is_playing_forward(&self) -> bool {
        // 非暂停且速度乘数为正 → 正向播放
        !self.paused && self.speed_multiplier > 0.0
    }

    /// 若正在反向播放则返回 true。
    pub fn is_playing_reverse(&self) -> bool {
        // 非暂停且速度乘数为负 → 反向播放
        !self.paused && self.speed_multiplier < 0.0
    }
}

/// 动画控制器的速度预设。
///
/// 提供从实时到每秒一天的离散倍率，便于 UI 下拉选择常见播放速率。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpeedPreset {
    /// 1 倍实时。
    RealTime,
    /// 2 倍速。
    Fast2x,
    /// 5 倍速。
    Fast5x,
    /// 10 倍速。
    Fast10x,
    /// 60 倍速（每秒 1 分钟）。
    Fast60x,
    /// 3600 倍速（每秒 1 小时）。
    Fast3600x,
    /// 86400 倍速（每秒 1 天）。
    Fast86400x,
}

impl SpeedPreset {
    /// 返回此预设的乘数。
    pub fn multiplier(&self) -> f64 {
        // 每个预设映射为一个固定的实时倍率常数
        match self {
            Self::RealTime => 1.0,
            Self::Fast2x => 2.0,
            Self::Fast5x => 5.0,
            Self::Fast10x => 10.0,
            Self::Fast60x => 60.0,
            Self::Fast3600x => 3600.0,
            Self::Fast86400x => 86400.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_clock() -> Clock {
        let start = JulianDate::from_date_components(2024, 1, 1, 0, 0, 0, 0.0);
        let stop = start.add_seconds(3600.0); // 1 小时
        Clock::new(start, stop, start)
    }

    #[test]
    fn test_timeline_config() {
        let start = JulianDate::from_date_components(2024, 1, 1, 0, 0, 0, 0.0);
        let end = start.add_seconds(7200.0);
        let config = TimelineConfig::new(start, end);

        assert!((config.duration_seconds() - 7200.0).abs() < 1e-10);
        assert!(config.visible);
    }

    #[test]
    fn test_animation_controller_creation() {
        let clock = create_test_clock();
        let controller = AnimationController::new(clock);

        assert!(controller.paused);
        assert_eq!(controller.speed_multiplier, 1.0);
        assert!(controller.looping);
    }

    #[test]
    fn test_play_pause() {
        let clock = create_test_clock();
        let mut controller = AnimationController::new(clock);

        controller.play();
        assert!(!controller.paused);
        assert!(controller.is_playing_forward());

        controller.pause();
        assert!(controller.paused);
    }

    #[test]
    fn test_play_reverse() {
        let clock = create_test_clock();
        let mut controller = AnimationController::new(clock);

        controller.play_reverse();
        assert!(!controller.paused);
        assert!(controller.is_playing_reverse());
    }

    #[test]
    fn test_tick_advances_time() {
        let clock = create_test_clock();
        let start = clock.current_time;
        let mut controller = AnimationController::new(clock);
        controller.play();

        let new_time = controller.tick(1.0); // 1 倍速下 1 秒
        let elapsed = new_time.seconds_difference(&start);
        assert!((elapsed - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_tick_with_speed() {
        let clock = create_test_clock();
        let start = clock.current_time;
        let mut controller = AnimationController::new(clock);
        controller.set_speed(60.0);
        controller.play();

        let new_time = controller.tick(1.0); // 60 倍速下 1 秒
        let elapsed = new_time.seconds_difference(&start);
        assert!((elapsed - 60.0).abs() < 1e-10);
    }

    #[test]
    fn test_tick_paused() {
        let clock = create_test_clock();
        let start = clock.current_time;
        let mut controller = AnimationController::new(clock);
        // 默认处于暂停

        let new_time = controller.tick(1.0);
        assert_eq!(new_time, start);
    }

    #[test]
    fn test_loop_wrapping() {
        let clock = create_test_clock();
        let start = clock.start_time;
        let mut controller = AnimationController::new(clock);
        controller.looping = true;
        controller.play();

        // 推进超过终点（3600 + 100 秒）
        let new_time = controller.tick(3700.0);
        let elapsed = new_time.seconds_difference(&start);
        assert!((elapsed - 100.0).abs() < 1e-10);
    }

    #[test]
    fn test_no_loop_clamp() {
        let clock = create_test_clock();
        let stop = clock.stop_time;
        let mut controller = AnimationController::new(clock);
        controller.looping = false;
        controller.play();

        // 推进超过终点
        let new_time = controller.tick(3700.0);
        assert_eq!(new_time, stop);
        assert!(controller.paused); // 到达终点自动暂停
    }

    #[test]
    fn test_seek() {
        let clock = create_test_clock();
        let start = clock.start_time;
        let mut controller = AnimationController::new(clock);

        let target = start.add_seconds(1800.0);
        controller.seek(target);
        assert_eq!(controller.clock.current_time, target);
    }

    #[test]
    fn test_seek_fraction() {
        let clock = create_test_clock();
        let start = clock.start_time;
        let mut controller = AnimationController::new(clock);

        controller.seek_fraction(0.5);
        let elapsed = controller.clock.current_time.seconds_difference(&start);
        assert!((elapsed - 1800.0).abs() < 1e-10);
    }

    #[test]
    fn test_progress() {
        let clock = create_test_clock();
        let mut controller = AnimationController::new(clock);

        controller.seek_fraction(0.25);
        assert!((controller.progress() - 0.25).abs() < 1e-10);
    }

    #[test]
    fn test_shuttle_ring() {
        let clock = create_test_clock();
        let mut controller = AnimationController::new(clock);

        controller.set_shuttle_ring(0.5);
        assert!(!controller.paused);
        assert!(controller.speed_multiplier > 0.0);

        controller.set_shuttle_ring(0.0);
        assert!(controller.paused);
    }

    #[test]
    fn test_stop() {
        let clock = create_test_clock();
        let start = clock.start_time;
        let mut controller = AnimationController::new(clock);
        controller.play();
        controller.tick(100.0);

        controller.stop();
        assert!(controller.paused);
        assert_eq!(controller.clock.current_time, start);
    }

    #[test]
    fn test_speed_presets() {
        assert_eq!(SpeedPreset::RealTime.multiplier(), 1.0);
        assert_eq!(SpeedPreset::Fast60x.multiplier(), 60.0);
        assert_eq!(SpeedPreset::Fast86400x.multiplier(), 86400.0);
    }
}
