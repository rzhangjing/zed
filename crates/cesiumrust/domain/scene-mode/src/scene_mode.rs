//! 场景模式及它们之间的形态变换。
//!
//! 定义四种渲染形态及其相互过渡：
//! - 3D（椭圆球地球视图）
//! - 2D（把椭球展开为平铺地图，Web Mercator）
//! - Columbus View（2.5D：地图平铺但带透视观察）
//! - Morphing（模式之间的形态变换过渡）
//!
//! 并提供 3D↔2D 的位置投影/反投影、按模式的相机定位，以及形态过渡所用的
//! [`smoothstep`] 缓动。所有几何以 `f64` 精度、弧度制经纬计算。

use glam::DVec3;
use std::f64::consts::PI;

/// 场景渲染模式。
///
/// 决定地球如何呈现给观察者：立体球面、平面地图、带透视的 Columbus View，
/// 或正处于相互切换的形态变换中。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SceneMode {
    /// 3D 地球视图。
    #[default]
    Scene3D,
    /// 2D 平铺地图（Web Mercator）。
    Scene2D,
    /// Columbus View（平铺地图上的 2.5D 透视）。
    ColumbusView,
    /// 模式之间的形态变换。
    Morphing,
}

impl SceneMode {
    /// 若为 3D 模式则返回 true。
    pub fn is_3d(&self) -> bool {
        matches!(self, Self::Scene3D)
    }

    /// 若为 2D 模式则返回 true。
    pub fn is_2d(&self) -> bool {
        matches!(self, Self::Scene2D)
    }
}

/// 场景模式之间的形态变换状态。
///
/// 跟踪一次从 `from` 到 `to` 的过渡：以 `elapsed`/`duration` 归一化出 `progress`，
/// 到达目标后把 `active` 置否。
#[derive(Debug, Clone)]
pub struct MorphState {
    /// 起始模式。
    pub from: SceneMode,
    /// 目标模式。
    pub to: SceneMode,
    /// 变换进度（0.0 = from，1.0 = to）。
    pub progress: f64,
    /// 形态变换是否正在活动。
    pub active: bool,
    /// 变换时长（秒）。
    pub duration: f64,
    /// 已过时间（秒）。
    pub elapsed: f64,
}

impl Default for MorphState {
    /// 默认形态状态：起止同为 3D、进度已置 1.0、未激活，时长 2.0 秒。
    /// 即一个“已完成且空闲”的初始态，便于直接作为静止场景的形态基线。
    fn default() -> Self {
        Self {
            from: SceneMode::Scene3D,
            to: SceneMode::Scene3D,
            progress: 1.0,
            active: false,
            duration: 2.0,
            elapsed: 0.0,
        }
    }
}

impl MorphState {
    /// 启动一次形态变换过渡。
    ///
    /// 记录起止模式与时长，把进度、已过时间归零并置 `active` 为真，交由 [`update`]
    /// 逐帧推进直至完成。`duration_secs` 为过渡总时长（秒）。
    pub fn start_morph(&mut self, from: SceneMode, to: SceneMode, duration_secs: f64) {
        // 复位计时与进度，标记为进行中
        self.from = from;
        self.to = to;
        self.progress = 0.0;
        self.active = true;
        self.duration = duration_secs;
        self.elapsed = 0.0;
    }

    /// 更新变换进度：按帧增量累加已过时间，并以 elapsed/duration 归一化进度。
    ///
    /// 未激活时直接返回；进度钳制到 [0.0, 1.0]，达到 1.0 即自动结束过渡。
    pub fn update(&mut self, delta_secs: f64) {
        if !self.active {
            return;
        }
        self.elapsed += delta_secs;
        // 线性归一化并夹到 [0,1]，避免超出时长后进度越界
        self.progress = (self.elapsed / self.duration).clamp(0.0, 1.0);
        if self.progress >= 1.0 {
            self.active = false;
        }
    }

    /// 返回当前生效的模式：过渡进行中报告 [`SceneMode::Morphing`]，否则为目标模式。
    pub fn current_mode(&self) -> SceneMode {
        // 激活期间不锁定起止任一模式，统一以 Morphing 表示“正在变换”
        if self.active {
            SceneMode::Morphing
        } else {
            self.to
        }
    }
}

/// 将一个 3D 位置投影为 2D 地图坐标。
///
/// # 参数
/// * `position` - 3D ECEF 位置
/// * `ellipsoid_radius` - 椭球长半轴
///
/// # 返回
/// 2D 位置（x = 经度 * 半径，y = 纬度 * 半径）
pub fn project_to_2d(position: DVec3, ellipsoid_radius: f64) -> DVec3 {
    // 由 ECEF 反算经度：xy 平面内相对 x 轴的方位角
    let lon = position.y.atan2(position.x);
    // 纬度用 z 与向量长度之比的反正弦（球面近似）
    let lat = (position.z / position.length()).asin();

    // 输出以 (经度*半径, 纬度*半径, 相对半径高度) 表示展开平面上的位置
    DVec3::new(
        lon * ellipsoid_radius,
        lat * ellipsoid_radius,
        position.length() - ellipsoid_radius,
    )
}

/// 将 2D 地图坐标反投影为 3D ECEF 位置。
pub fn unproject_from_2d(position_2d: DVec3, ellipsoid_radius: f64) -> DVec3 {
    // 2D 平面坐标除以半径还原弧度制经纬
    let lon = position_2d.x / ellipsoid_radius;
    let lat = position_2d.y / ellipsoid_radius;
    let height = position_2d.z;
    // r 为该点到地心的距离（半径 + 相对高度）
    let r = ellipsoid_radius + height;

    // 标准球面经纬到直角坐标的反投影
    DVec3::new(
        r * lat.cos() * lon.cos(),
        r * lat.cos() * lon.sin(),
        r * lat.sin(),
    )
}

/// 将一个 3D 位置投影为 Columbus View 坐标。
///
/// Columbus View 是一种 2.5D 投影：地图是平铺的，
/// 但以透视方式观察。
pub fn project_to_columbus_view(position: DVec3, ellipsoid_radius: f64) -> DVec3 {
    // 与 2D 相同的经纬反算，仅高度直接取相对椭球面的超出量
    let lon = position.y.atan2(position.x);
    let lat = (position.z / position.length()).asin();
    let height = position.length() - ellipsoid_radius;

    // Columbus View：x = lon，y = lat，z = height（但处于一个平面内）
    DVec3::new(
        lon * ellipsoid_radius,
        lat * ellipsoid_radius,
        height,
    )
}

/// 为形态变换在 3D 与 2D 位置之间插值。
pub fn morph_position(
    position_3d: DVec3,
    position_2d: DVec3,
    progress: f64,
) -> DVec3 {
    // 平滑阶跃，使过渡更自然
    let t = smoothstep(progress);
    position_3d.lerp(position_2d, t)
}

/// 用于缓动的平滑阶跃函数（Hermite 三次插值 3t²-2t³）。
///
/// 输入先把 `t` 钳制到 [0,1]，输出在两端点处一阶导为 0，故过渡起止更柔和，
/// 适合驱动形态变换的插值权重。
pub fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    // 经典 smoothstep 多项式：t*t*(3 - 2t)
    t * t * (3.0 - 2.0 * t)
}

/// 计算给定场景模式下的 camera 位置。
pub fn compute_camera_for_mode(
    mode: SceneMode,
    center_lon: f64,
    center_lat: f64,
    height: f64,
    ellipsoid_radius: f64,
) -> DVec3 {
    match mode {
        // 3D：相机置于半径为 (R+h) 的球面上，按经纬度展开为 ECEF
        SceneMode::Scene3D => {
            let r = ellipsoid_radius + height;
            DVec3::new(
                r * center_lat.cos() * center_lon.cos(),
                r * center_lat.cos() * center_lon.sin(),
                r * center_lat.sin(),
            )
        }
        // 2D：地图平铺，相机直接落在 (lon*R, lat*R)，高度即观察距离
        SceneMode::Scene2D => {
            DVec3::new(
                center_lon * ellipsoid_radius,
                center_lat * ellipsoid_radius,
                height,
            )
        }
        // Columbus View：与 2D 相同的平面定位，但以透视相机从上方俯视
        SceneMode::ColumbusView => {
            DVec3::new(
                center_lon * ellipsoid_radius,
                center_lat * ellipsoid_radius,
                height,
            )
        }
        SceneMode::Morphing => {
            // 变换期间默认采用 3D
            compute_camera_for_mode(SceneMode::Scene3D, center_lon, center_lat, height, ellipsoid_radius)
        }
    }
}

/// 2D 模式的地图投影。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapProjection2D {
    /// 地理（等距矩形）。
    #[default]
    Geographic,
    /// Web Mercator.
    WebMercator,
}

impl MapProjection2D {
    /// 将地理坐标投影为 2D。
    pub fn project(&self, lon: f64, lat: f64, radius: f64) -> DVec3 {
        match self {
            Self::Geographic => DVec3::new(lon * radius, lat * radius, 0.0),
            Self::WebMercator => {
                let x = lon * radius;
                let y = (PI / 4.0 + lat / 2.0).tan().ln() * radius;
                DVec3::new(x, y, 0.0)
            }
        }
    }

    /// 将 2D 坐标反投影为地理坐标。
    pub fn unproject(&self, x: f64, y: f64, radius: f64) -> (f64, f64) {
        match self {
            Self::Geographic => (x / radius, y / radius),
            Self::WebMercator => {
                let lon = x / radius;
                let lat = 2.0 * (y / radius).exp().atan() - PI / 2.0;
                (lon, lat)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EARTH_RADIUS: f64 = 6378137.0;

    #[test]
    fn test_scene_mode_default() {
        assert_eq!(SceneMode::default(), SceneMode::Scene3D);
    }

    #[test]
    fn test_scene_mode_is_3d() {
        assert!(SceneMode::Scene3D.is_3d());
        assert!(!SceneMode::Scene2D.is_3d());
        assert!(!SceneMode::ColumbusView.is_3d());
    }

    #[test]
    fn test_scene_mode_is_2d() {
        assert!(SceneMode::Scene2D.is_2d());
        assert!(!SceneMode::Scene3D.is_2d());
    }

    #[test]
    fn test_morph_state_default() {
        let state = MorphState::default();
        assert!(!state.active);
        assert_eq!(state.progress, 1.0);
    }

    #[test]
    fn test_morph_start() {
        let mut state = MorphState::default();
        state.start_morph(SceneMode::Scene3D, SceneMode::Scene2D, 2.0);
        assert!(state.active);
        assert_eq!(state.progress, 0.0);
        assert_eq!(state.from, SceneMode::Scene3D);
        assert_eq!(state.to, SceneMode::Scene2D);
    }

    #[test]
    fn test_morph_update() {
        let mut state = MorphState::default();
        state.start_morph(SceneMode::Scene3D, SceneMode::Scene2D, 2.0);

        state.update(1.0);
        assert!((state.progress - 0.5).abs() < 1e-10);
        assert!(state.active);

        state.update(1.0);
        assert!((state.progress - 1.0).abs() < 1e-10);
        assert!(!state.active);
    }

    #[test]
    fn test_morph_current_mode() {
        let mut state = MorphState::default();
        state.start_morph(SceneMode::Scene3D, SceneMode::Scene2D, 2.0);

        assert_eq!(state.current_mode(), SceneMode::Morphing);

        state.update(3.0); // 完成
        assert_eq!(state.current_mode(), SceneMode::Scene2D);
    }

    #[test]
    fn test_project_to_2d() {
        // 本初子午线与赤道的交点
        let pos = DVec3::new(EARTH_RADIUS, 0.0, 0.0);
        let pos_2d = project_to_2d(pos, EARTH_RADIUS);

        assert!(pos_2d.x.abs() < 1e-6); // lon = 0
        assert!(pos_2d.y.abs() < 1e-6); // lat = 0
        assert!(pos_2d.z.abs() < 1e-6); // height = 0
    }

    #[test]
    fn test_unproject_from_2d() {
        let pos_2d = DVec3::new(0.0, 0.0, 1000.0);
        let pos_3d = unproject_from_2d(pos_2d, EARTH_RADIUS);

        // 应位于本初子午线与赤道的交点，上方 1000m
        let expected_r = EARTH_RADIUS + 1000.0;
        assert!((pos_3d.x - expected_r).abs() < 1.0);
        assert!(pos_3d.y.abs() < 1.0);
        assert!(pos_3d.z.abs() < 1.0);
    }

    #[test]
    fn test_smoothstep() {
        assert!((smoothstep(0.0) - 0.0).abs() < 1e-10);
        assert!((smoothstep(0.5) - 0.5).abs() < 1e-10);
        assert!((smoothstep(1.0) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_morph_position() {
        let pos_3d = DVec3::new(100.0, 0.0, 0.0);
        let pos_2d = DVec3::new(0.0, 100.0, 0.0);

        let mid = morph_position(pos_3d, pos_2d, 0.5);
        assert!((mid.x - 50.0).abs() < 1e-10);
        assert!((mid.y - 50.0).abs() < 1e-10);
    }

    #[test]
    fn test_compute_camera_3d() {
        let cam = compute_camera_for_mode(SceneMode::Scene3D, 0.0, 0.0, 1000000.0, EARTH_RADIUS);
        let expected_r = EARTH_RADIUS + 1000000.0;
        assert!((cam.x - expected_r).abs() < 1.0);
    }

    #[test]
    fn test_compute_camera_2d() {
        let cam = compute_camera_for_mode(SceneMode::Scene2D, 0.5, 0.3, 1000000.0, EARTH_RADIUS);
        assert!((cam.x - 0.5 * EARTH_RADIUS).abs() < 1.0);
        assert!((cam.y - 0.3 * EARTH_RADIUS).abs() < 1.0);
        assert!((cam.z - 1000000.0).abs() < 1.0);
    }

    #[test]
    fn test_map_projection_geographic() {
        let proj = MapProjection2D::Geographic;
        let pos = proj.project(0.5, 0.3, EARTH_RADIUS);
        assert!((pos.x - 0.5 * EARTH_RADIUS).abs() < 1.0);
        assert!((pos.y - 0.3 * EARTH_RADIUS).abs() < 1.0);

        let (lon, lat) = proj.unproject(pos.x, pos.y, EARTH_RADIUS);
        assert!((lon - 0.5).abs() < 1e-10);
        assert!((lat - 0.3).abs() < 1e-10);
    }

    #[test]
    fn test_map_projection_web_mercator() {
        let proj = MapProjection2D::WebMercator;
        let pos = proj.project(0.0, 0.0, EARTH_RADIUS);
        assert!(pos.x.abs() < 1e-6);
        assert!(pos.y.abs() < 1e-6);

        let (lon, lat) = proj.unproject(0.0, 0.0, EARTH_RADIUS);
        assert!(lon.abs() < 1e-10);
        assert!(lat.abs() < 1e-10);
    }
}
