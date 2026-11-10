//! 动画时钟资源。
//!
//! 本模块只承载 [`AnimationClock`] —— 一个薄薄的资源包装，持有
//! `cesium_animation::timeline::AnimationController`。
//!
//! # 为什么单独成模块
//!
//! 该时钟原先住在 `entity::time_system`（Entity/DataSource 可视化路线内部）。
//! 但 `atmosphere::{celestial_system, sky_system}` 在 **不依赖 Entity 路线**
//! 的前提下也需要读它（`CesiumCorePlugin` 自 M4.1 起无条件注册本资源，正是
//! 为了让 `celestial_system` 能脱离 `CesiumEntityPlugin` 工作）。
//!
//! 2026-09 的 P0 裁剪删除了整条 Entity/DataSource 适配器路线，因此把时钟
//! 提升到 crate 根模块，使「日月与星空驱动」与「实体可视化」彻底解耦：
//! 删掉 `entity`/`datasource` 不会连带打断大气与星空。

use bevy::prelude::*;
use cesium_animation::timeline::AnimationController;
use cesium_time::clock::Clock;
use cesium_time::julian_date::JulianDate;

/// 用于控制动画播放的资源。
#[derive(Resource)]
pub struct AnimationClock {
    /// 底层动画控制器（持有 Clock 与播放状态）。
    pub controller: AnimationController,
}

impl AnimationClock {
    /// 用起止时间新建时钟（当前时间从 start 开始，初始暂停）。
    ///
    /// # 参数
    /// - `start`：起始儒略日
    /// - `stop`：结束儒略日
    pub fn new(start: JulianDate, stop: JulianDate) -> Self {
        let clock = Clock::new(start, stop, start);
        Self {
            controller: AnimationController::new(clock),
        }
    }

    /// 开始播放。
    pub fn play(&mut self) {
        self.controller.play();
    }

    /// 暂停（保留当前时间）。
    pub fn pause(&mut self) {
        self.controller.pause();
    }

    /// 停止并回到起点。
    pub fn stop(&mut self) {
        self.controller.stop();
    }

    /// 跳转到指定的绝对时间。
    ///
    /// # 参数
    /// - `time`：目标儒略日
    pub fn seek(&mut self, time: JulianDate) {
        self.controller.seek(time);
    }

    /// 按 [0,1] 比例跳转到时间轴上的对应位置。
    ///
    /// # 参数
    /// - `fraction`：进度比例
    pub fn seek_fraction(&mut self, fraction: f64) {
        self.controller.seek_fraction(fraction);
    }

    /// 设置时间倍速乘子。
    ///
    /// # 参数
    /// - `multiplier`：倍速（1=实时）
    pub fn set_speed(&mut self, multiplier: f64) {
        self.controller.set_speed(multiplier);
    }

    /// 取当前时间（儒略日）。
    pub fn current_time(&self) -> JulianDate {
        self.controller.clock.current_time
    }

    /// 取当前进度比例 [0,1]。
    pub fn progress(&self) -> f64 {
        self.controller.progress()
    }

    /// 是否处于播放（非暂停）状态。
    pub fn is_playing(&self) -> bool {
        !self.controller.paused
    }
}

/// 默认动画时钟（从 epoch 到 epoch+24h）。
impl Default for AnimationClock {
    /// 默认：2024-01-01 起、历 24 小时。
    fn default() -> Self {
        let start = JulianDate::from_date_components(2024, 1, 1, 0, 0, 0, 0.0);
        let stop = start.add_seconds(86400.0);
        Self::new(start, stop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认时钟必须停在 2024-01-01T00:00:00 UTC，历 24 小时、初始暂停。
    ///
    /// 这是 v2_sky 基线的锚点（`sky_dome.rs` 的 `frozen_julian_date` 断言
    /// 依赖它），因此搬迁模块后必须保持逐位一致。
    #[test]
    fn default_epoch_is_bit_exact() {
        let clock = AnimationClock::default();
        let expected = JulianDate::from_date_components(2024, 1, 1, 0, 0, 0, 0.0);
        assert_eq!(clock.current_time().total_days(), expected.total_days());
        assert!(!clock.is_playing(), "默认时钟应为暂停态");
        assert_eq!(clock.progress(), 0.0);
    }

    /// 播放/暂停/据比例跳转的基本语义。
    #[test]
    fn play_pause_and_seek_fraction() {
        let mut clock = AnimationClock::default();
        clock.play();
        assert!(clock.is_playing());
        clock.pause();
        assert!(!clock.is_playing());

        clock.seek_fraction(0.5);
        let half = clock.progress();
        assert!((half - 0.5).abs() < 1e-9, "progress={half}");
    }
}
