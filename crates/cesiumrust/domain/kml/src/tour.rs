//! KML Tour（巡游）支持。
//!
//! 将 KML 巡游表达为一串播放列表条目（飞行至与等待交替），提供
//! 播放、逐条目前进与总时长汇总等纯数据层的导航语义。

use glam::DVec3;

// ============================================================================
// KmlTourFlyTo
// ============================================================================

/// KML 巡游播放列表中的一个 fly-to（飞行至）条目。
///
/// 描述一次相机飞行：目标位置、时长、航向/仰俯/距离与插值模式。
#[derive(Debug, Clone, PartialEq)]
pub struct KmlTourFlyTo {
    /// fly-to 的时长（秒）。
    pub duration: f64,
    /// 目标位置（经度、纬度、高度），单位为度/米。
    pub position: DVec3,
    /// 航向角（度）。
    pub heading: Option<f64>,
    /// 仰俯角（度）。
    pub tilt: Option<f64>,
    /// 距离（与目标的间距），单位为米。
    pub range: Option<f64>,
    /// 是否使用大圆路径（而非线性）。
    pub fly_to_mode: FlyToMode,
}

/// fly-to 插值模式。
///
/// 决定相机在位置/朝向变化时采用平滑路径还是跳跃式的落地动画。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlyToMode {
    /// 平滑的相机路径。
    #[default]
    Smooth,
    /// 跳跃效果。
    Bounce,
}

impl KmlTourFlyTo {
    /// 创建一个新的 fly-to 条目。
    pub fn new(duration: f64, position: DVec3) -> Self {
        Self {
            duration,
            position,
            heading: None,
            tilt: None,
            range: None,
            fly_to_mode: FlyToMode::Smooth,
        }
    }

    /// 设置航向角。
    pub fn with_heading(mut self, heading: f64) -> Self {
        self.heading = Some(heading);
        self
    }

    /// 设置仰俯角。
    pub fn with_tilt(mut self, tilt: f64) -> Self {
        self.tilt = Some(tilt);
        self
    }

    /// 设置距离。
    pub fn with_range(mut self, range: f64) -> Self {
        self.range = Some(range);
        self
    }

    /// 设置 fly-to 模式。
    pub fn with_mode(mut self, mode: FlyToMode) -> Self {
        self.fly_to_mode = mode;
        self
    }
}

// ============================================================================
// KmlTourWait
// ============================================================================

/// KML 巡游播放列表中的一个等待条目。
///
/// 仅携带一段暂停时长，用于在相邻飞行之间插入停顿。
#[derive(Debug, Clone, PartialEq)]
pub struct KmlTourWait {
    /// 等待时长（秒）。
    pub duration: f64,
}

impl KmlTourWait {
    /// 创建一个新的等待条目。
    pub fn new(duration: f64) -> Self {
        Self { duration }
    }
}

// ============================================================================
// KmlTourEntry
// ============================================================================

/// 一个播放列表条目（fly-to 或 wait 之一）。
///
/// 统一两种条目的时长接口，供巡游按序迭代。
#[derive(Debug, Clone, PartialEq)]
pub enum KmlTourEntry {
    /// 飞行至某个位置。
    FlyTo(KmlTourFlyTo),
    /// 等待一段时长。
    Wait(KmlTourWait),
}

impl KmlTourEntry {
    /// 获取此条目的时长。
    pub fn duration(&self) -> f64 {
        match self {
            Self::FlyTo(f) => f.duration,
            Self::Wait(w) => w.duration,
        }
    }
}

// ============================================================================
// KmlTour
// ============================================================================

/// 一个带有播放列表条目的 KML 巡游。
///
/// 按序保存 fly-to/wait 条目，并跟踪当前播放索引与是否在播。
#[derive(Debug, Clone, PartialEq)]
pub struct KmlTour {
    /// 巡游 ID。
    pub id: String,
    /// 巡游名称。
    pub name: String,
    /// 播放列表条目。
    pub playlist: Vec<KmlTourEntry>,
    /// 当前播放列表索引。
    pub playlist_index: usize,
    /// 巡游是否正在播放。
    pub is_playing: bool,
}

impl KmlTour {
    /// 创建一个新的巡游。
    pub fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            playlist: Vec::new(),
            playlist_index: 0,
            is_playing: false,
        }
    }

    /// 向播放列表添加一个 fly-to 条目。
    pub fn add_fly_to(&mut self, fly_to: KmlTourFlyTo) {
        self.playlist.push(KmlTourEntry::FlyTo(fly_to));
    }

    /// 向播放列表添加一个 wait 条目。
    pub fn add_wait(&mut self, wait: KmlTourWait) {
        self.playlist.push(KmlTourEntry::Wait(wait));
    }

    /// 向播放列表添加一个通用条目。
    pub fn add_entry(&mut self, entry: KmlTourEntry) {
        self.playlist.push(entry);
    }

    /// 获取巡游的总时长。
    pub fn total_duration(&self) -> f64 {
        // 各条目时长求和即巡游总时长
        self.playlist.iter().map(|e| e.duration()).sum()
    }

    /// 获取条目数。
    pub fn entry_count(&self) -> usize {
        self.playlist.len()
    }

    /// 开始播放巡游。
    pub fn play(&mut self) {
        self.is_playing = true;
        self.playlist_index = 0;
    }

    /// 停止巡游。
    pub fn stop(&mut self) {
        self.is_playing = false;
        self.playlist_index = 0;
    }

    /// 前进到下一个条目。若巡游已完成则返回 false。
    pub fn advance(&mut self) -> bool {
        // 索引前进一位；越过末尾则置为停止并返回 false
        if self.playlist_index < self.playlist.len() {
            self.playlist_index += 1;
        }
        if self.playlist_index >= self.playlist.len() {
            self.is_playing = false;
            return false;
        }
        true
    }

    /// 获取当前条目。
    pub fn current_entry(&self) -> Option<&KmlTourEntry> {
        self.playlist.get(self.playlist_index)
    }

    /// 巡游是否已完成。
    pub fn is_complete(&self) -> bool {
        self.playlist_index >= self.playlist.len()
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kml_tour_fly_to() {
        let fly_to = KmlTourFlyTo::new(5.0, DVec3::new(-122.0, 37.0, 1000.0))
            .with_heading(45.0)
            .with_tilt(60.0)
            .with_range(5000.0)
            .with_mode(FlyToMode::Bounce);

        assert_eq!(fly_to.duration, 5.0);
        assert_eq!(fly_to.heading, Some(45.0));
        assert_eq!(fly_to.tilt, Some(60.0));
        assert_eq!(fly_to.range, Some(5000.0));
        assert_eq!(fly_to.fly_to_mode, FlyToMode::Bounce);
    }

    #[test]
    fn test_kml_tour_wait() {
        let wait = KmlTourWait::new(2.5);
        assert_eq!(wait.duration, 2.5);
    }

    #[test]
    fn test_kml_tour_entry_duration() {
        let fly_to = KmlTourEntry::FlyTo(KmlTourFlyTo::new(3.0, DVec3::ZERO));
        let wait = KmlTourEntry::Wait(KmlTourWait::new(1.5));

        assert_eq!(fly_to.duration(), 3.0);
        assert_eq!(wait.duration(), 1.5);
    }

    #[test]
    fn test_kml_tour_playlist() {
        let mut tour = KmlTour::new("tour1", "City Tour");

        tour.add_fly_to(KmlTourFlyTo::new(5.0, DVec3::new(-122.0, 37.0, 0.0)));
        tour.add_wait(KmlTourWait::new(2.0));
        tour.add_fly_to(KmlTourFlyTo::new(4.0, DVec3::new(-121.0, 38.0, 0.0)));

        assert_eq!(tour.entry_count(), 3);
        assert_eq!(tour.total_duration(), 11.0);
    }

    #[test]
    fn test_kml_tour_playback() {
        let mut tour = KmlTour::new("tour1", "Test");
        tour.add_fly_to(KmlTourFlyTo::new(1.0, DVec3::ZERO));
        tour.add_wait(KmlTourWait::new(1.0));

        assert!(!tour.is_playing);
        assert_eq!(tour.playlist_index, 0);

        tour.play();
        assert!(tour.is_playing);
        assert_eq!(tour.playlist_index, 0);
        assert!(!tour.is_complete());

        // 逐个前进条目
        assert!(tour.advance());
        assert_eq!(tour.playlist_index, 1);
        assert!(!tour.is_complete());

        assert!(!tour.advance()); // 最后一个条目
        assert!(tour.is_complete());
        assert!(!tour.is_playing);
    }

    #[test]
    fn test_kml_tour_stop() {
        let mut tour = KmlTour::new("tour1", "Test");
        tour.add_fly_to(KmlTourFlyTo::new(1.0, DVec3::ZERO));
        tour.add_wait(KmlTourWait::new(1.0));

        tour.play();
        tour.advance();
        assert_eq!(tour.playlist_index, 1);

        tour.stop();
        assert!(!tour.is_playing);
        assert_eq!(tour.playlist_index, 0);
    }

    #[test]
    fn test_kml_tour_current_entry() {
        let mut tour = KmlTour::new("tour1", "Test");
        tour.add_fly_to(KmlTourFlyTo::new(5.0, DVec3::new(1.0, 2.0, 3.0)));
        tour.add_wait(KmlTourWait::new(2.0));

        let entry = tour.current_entry().unwrap();
        assert_eq!(entry.duration(), 5.0);

        tour.advance();
        let entry = tour.current_entry().unwrap();
        assert_eq!(entry.duration(), 2.0);
    }
}
