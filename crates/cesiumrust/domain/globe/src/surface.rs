//! 地球表面渲染与地形交互。
//!
//! 涵盖以下方面：
//! - 地球表面属性
//! - 针对地形的深度测试
//! - 高程查询
//! - 地球半透明
//! - 地下渲染
//! - 光照淡入淡出距离
//! - 地形平边（skirts）与背面剔除

use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::rectangle::Rectangle;
use glam::DVec3;
use std::f64::consts::PI;

/// 地球渲染的阴影模式。
/// 映射到 CesiumJS `Scene/ShadowMode`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShadowMode {
    /// 禁用阴影。
    Disabled,
    /// 地球仅接收阴影。
    #[default]
    ReceiveOnly,
    /// 地球仅投射阴影。
    CastOnly,
    /// 地球既投射又接收阴影。
    Enabled,
}

/// 用于基于距离插值的近/远标量。
/// 映射到 CesiumJS `Core/NearFarScalar`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NearFarScalar {
    /// 近距。
    pub near: f64,
    /// 近距处的值。
    pub near_value: f64,
    /// 远距。
    pub far: f64,
    /// 远距处的值。
    pub far_value: f64,
}

impl NearFarScalar {
    /// 创建一个新的近/远标量。
    pub fn new(near: f64, near_value: f64, far: f64, far_value: f64) -> Self {
        Self { near, near_value, far, far_value }
    }

    /// 插值给定距离处的值。
    pub fn interpolate(&self, distance: f64) -> f64 {
        if distance <= self.near {
            return self.near_value;
        }
        if distance >= self.far {
            return self.far_value;
        }
        let t = (distance - self.near) / (self.far - self.near);
        self.near_value + t * (self.far_value - self.near_value)
    }
}

/// 地球渲染配置。
///
/// 聚合地表渲染开关、深度测试、半透明、大气偏移等一整套可调参数。
#[derive(Debug, Clone)]
pub struct GlobeConfig {
    /// 是否显示地球。
    pub show: bool,
    /// 是否启用针对地形的深度测试。
    pub depth_test_against_terrain: bool,
    /// 地球是否半透明（用于地下渲染）。
    pub translucency_enabled: bool,
    /// 地球半透明正面 alpha。
    pub front_face_alpha: f64,
    /// 地球半透明背面 alpha。
    pub back_face_alpha: f64,
    /// 是否显示近地大气效果。
    pub show_ground_atmosphere: bool,
    /// 是否显示水面效果。
    pub show_water_effect: bool,
    /// 无可用影像时的基础颜色 [r, g, b]。
    pub base_color: [f64; 3],
    /// 地形瓦片的最大屏幕空间误差。
    pub maximum_screen_space_error: f64,
    /// 瓦片缓存大小（瓦片数）。
    pub tile_cache_size: usize,
    /// 是否启用光照（昼夜）。
    pub enable_lighting: bool,
    /// 用于瓦片调度的后代加载上限。
    pub loading_descendant_limit: u32,
    /// 是否预加载祖先瓦片。
    pub preload_ancestors: bool,
    /// 是否预加载兄弟瓦片。
    pub preload_siblings: bool,
    /// 填充高亮颜色 [r, g, b, a]（None = 无高亮）。
    pub fill_highlight_color: Option<[f64; 4]>,
    /// 地形光照的 Lambert 漫反射乘数。
    pub lambert_diffuse_multiplier: f64,
    /// 是否启用动态大气光照。
    pub dynamic_atmosphere_lighting: bool,
    /// 动态大气光照是否使用太阳方向。
    pub dynamic_atmosphere_lighting_from_sun: bool,
    /// 大气光照强度。
    pub atmosphere_light_intensity: f64,
    /// Rayleigh 散射系数 [r, g, b]。
    pub atmosphere_rayleigh_coefficient: [f64; 3],
    /// Mie 散射系数 [r, g, b]。
    pub atmosphere_mie_coefficient: [f64; 3],
    /// Rayleigh 标高（米）。
    pub atmosphere_rayleigh_scale_height: f64,
    /// Mie 标高（米）。
    pub atmosphere_mie_scale_height: f64,
    /// Mie 各向异性因子（-1.0 到 1.0）。
    pub atmosphere_mie_anisotropy: f64,
    /// 光照淡出距离（米）。
    pub lighting_fade_out_distance: f64,
    /// 光照淡入距离（米）。
    pub lighting_fade_in_distance: f64,
    /// 黑夜淡出距离（米）。
    pub night_fade_out_distance: f64,
    /// 黑夜淡入距离（米）。
    pub night_fade_in_distance: f64,
    /// 是否显示地形平边（skirts）。
    pub show_skirts: bool,
    /// 是否剔除背向的地形。
    pub back_face_culling: bool,
    /// 顶点阴影黑度（0.0 到 1.0）。
    pub vertex_shadow_darkness: f64,
    /// 地下颜色 [r, g, b]（None = 禁用）。
    pub underground_color: Option<[f64; 3]>,
    /// 按距离变化的地下颜色 alpha。
    pub underground_color_alpha_by_distance: Option<NearFarScalar>,
    /// 用于渲染的 Cartographic 限制矩形。
    pub cartographic_limit_rectangle: Rectangle,
    /// 阴影模式。
    pub shadows: ShadowMode,
    /// 大气色相偏移（-1.0 到 1.0）。
    pub atmosphere_hue_shift: f64,
    /// 大气饱和度偏移（-1.0 到 1.0）。
    pub atmosphere_saturation_shift: f64,
    /// 大气亮度偏移（-1.0 到 1.0）。
    pub atmosphere_brightness_shift: f64,
}

impl Default for GlobeConfig {
    /// 默认：显示地球、不对地形做深度测试、不启用半透明，其余取常见默认色调偏移。
    fn default() -> Self {
        let min_radius = Ellipsoid::WGS84.minimum_radius();
        Self {
            show: true,
            depth_test_against_terrain: false,
            translucency_enabled: false,
            front_face_alpha: 1.0,
            back_face_alpha: 1.0,
            show_ground_atmosphere: true,
            show_water_effect: true,
            base_color: [0.0, 0.0, 0.5], // 海洋蓝
            maximum_screen_space_error: 2.0,
            tile_cache_size: 100,
            enable_lighting: false,
            loading_descendant_limit: 20,
            preload_ancestors: true,
            preload_siblings: false,
            fill_highlight_color: None,
            lambert_diffuse_multiplier: 0.9,
            dynamic_atmosphere_lighting: true,
            dynamic_atmosphere_lighting_from_sun: false,
            atmosphere_light_intensity: 10.0,
            atmosphere_rayleigh_coefficient: [5.5e-6, 13.0e-6, 28.4e-6],
            atmosphere_mie_coefficient: [21e-6, 21e-6, 21e-6],
            atmosphere_rayleigh_scale_height: 10000.0,
            atmosphere_mie_scale_height: 3200.0,
            atmosphere_mie_anisotropy: 0.9,
            lighting_fade_out_distance: PI * 0.5 * min_radius,
            lighting_fade_in_distance: PI * min_radius,
            night_fade_out_distance: PI * 0.5 * min_radius,
            night_fade_in_distance: 5.0 * PI * 0.5 * min_radius,
            show_skirts: true,
            back_face_culling: true,
            vertex_shadow_darkness: 0.3,
            underground_color: Some([0.0, 0.0, 0.0]),
            underground_color_alpha_by_distance: Some(NearFarScalar::new(
                min_radius / 1000.0,
                0.0,
                min_radius / 5.0,
                1.0,
            )),
            cartographic_limit_rectangle: Rectangle::MAX_VALUE,
            shadows: ShadowMode::ReceiveOnly,
            atmosphere_hue_shift: 0.0,
            atmosphere_saturation_shift: 0.0,
            atmosphere_brightness_shift: 0.0,
        }
    }
}

/// 用于地形交互的地球表面。
#[derive(Debug, Clone)]
pub struct GlobeSurface {
    /// 椭球形状。
    pub ellipsoid: Ellipsoid,
    /// 配置。
    pub config: GlobeConfig,
    /// 当前视图中的最小地形高度。
    pub minimum_terrain_height: f64,
    /// 当前视图中的最大地形高度。
    pub maximum_terrain_height: f64,
}

impl GlobeSurface {
    /// 创建一个使用 WGS84 椭球的新地球表面。
    pub fn new() -> Self {
        Self {
            ellipsoid: Ellipsoid::WGS84,
            config: GlobeConfig::default(),
            minimum_terrain_height: 0.0,
            maximum_terrain_height: 0.0,
        }
    }

    /// 创建一个使用自定义椭球的地球表面。
    pub fn with_ellipsoid(ellipsoid: Ellipsoid) -> Self {
        Self {
            ellipsoid,
            config: GlobeConfig::default(),
            minimum_terrain_height: 0.0,
            maximum_terrain_height: 0.0,
        }
    }

    /// 获取某位置处的表面法线。
    pub fn get_surface_normal(&self, position: DVec3) -> DVec3 {
        self.ellipsoid
            .geodetic_surface_normal(position)
            .unwrap_or(DVec3::Z)
    }

    /// 获取某 Cartographic 位置处的高度（简化）。
    ///
    /// 在完整实现中，此处会查询地形瓦片。
    pub fn get_height(&self, _cartographic: &Cartographic) -> Option<f64> {
        // 简化：返回 0（椭球表面）
        Some(0.0)
    }

    /// 用一条射线拾取地球表面。
    ///
    /// 返回世界坐标中的交点。
    pub fn pick(&self, ray_origin: DVec3, ray_direction: DVec3) -> Option<DVec3> {
        // 使用标准二次型公式进行射线-椭球相交
        // 对于椭球 x^2/a^2 + y^2/b^2 + z^2/c^2 = 1
        // 射线：P = O + t*D
        // 代入：(O + t*D)^T * M * (O + t*D) = 1
        // 其中 M = diag(1/a^2, 1/b^2, 1/c^2)

        let radii = self.ellipsoid.radii();
        let one_over_radii_sq = DVec3::new(
            1.0 / (radii.x * radii.x),
            1.0 / (radii.y * radii.y),
            1.0 / (radii.z * radii.z),
        );

        let o = ray_origin;
        let d = ray_direction;

        // 二次型系数：A*t^2 + B*t + C = 0
        let o_scaled = o * one_over_radii_sq;
        let d_scaled = d * one_over_radii_sq;

        let a = d.dot(d_scaled);
        let b = 2.0 * o.dot(d_scaled);
        let c = o.dot(o_scaled) - 1.0;

        let discriminant = b * b - 4.0 * a * c;

        if discriminant < 0.0 {
            return None;
        }

        let sqrt_disc = discriminant.sqrt();
        let t1 = (-b - sqrt_disc) / (2.0 * a);
        let t2 = (-b + sqrt_disc) / (2.0 * a);

        // 找到最近的非负 t
        let t = if t1 >= 0.0 {
            t1
        } else if t2 >= 0.0 {
            t2
        } else {
            return None;
        };

        Some(ray_origin + ray_direction * t)
    }

    /// 计算从某高度到地平线的距离。
    ///
    /// 以米返回到地平线的距离。
    pub fn horizon_distance(&self, height: f64) -> f64 {
        let r = self.ellipsoid.maximum_radius();
        // d = sqrt((r + h)^2 - r^2) = sqrt(2*r*h + h^2)
        (2.0 * r * height + height * height).sqrt()
    }

    /// 计算从某高度看地平线的俯角。
    ///
    /// 以弧度返回低于水平面的角度。
    pub fn horizon_dip_angle(&self, height: f64) -> f64 {
        let r = self.ellipsoid.maximum_radius();
        // cos(dip) = r / (r + h)
        (r / (r + height)).acos()
    }

    /// 判断某位置是否处于可见半球上。
    pub fn is_on_visible_hemisphere(&self, position: DVec3, camera_position: DVec3) -> bool {
        let surface_normal = self.get_surface_normal(position);
        let to_camera = (camera_position - position).normalize();
        surface_normal.dot(to_camera) > 0.0
    }

    /// 计算瓦片的近似细节层级。
    pub fn compute_tile_sse(
        &self,
        tile_geometric_error: f64,
        distance: f64,
        viewport_height: f64,
        sse_denominator: f64,
    ) -> f64 {
        // SSE = (geometricError * viewportHeight) / (distance * sseDenominator)
        if distance <= 0.0 {
            return f64::MAX;
        }
        (tile_geometric_error * viewport_height) / (distance * sse_denominator)
    }

    /// 根据 SSE 判断是否应细化瓦片。
    pub fn should_refine_tile(&self, sse: f64) -> bool {
        sse > self.config.maximum_screen_space_error
    }
}

impl Default for GlobeSurface {
    /// 默认地表：直接委托 [`GlobeSurface::new`] 构造带默认配置的实例。
    fn default() -> Self {
        Self::new()
    }
}

/// 地球半透明设置。
///
/// 分别控制正/背面的 alpha，用于透视地球看到内部或背面地形的效果。
#[derive(Debug, Clone)]
pub struct GlobeTranslucency {
    /// 是否启用半透明。
    pub enabled: bool,
    /// 正面 alpha（0.0 = 透明，1.0 = 不透明）。
    pub front_face_alpha: f64,
    /// 背面 alpha。
    pub back_face_alpha: f64,
}

impl Default for GlobeTranslucency {
    /// 默认：不启用半透明，正/背面 alpha 均为 1.0（完全不透明）。
    fn default() -> Self {
        Self {
            enabled: false,
            front_face_alpha: 1.0,
            back_face_alpha: 1.0,
        }
    }
}

impl GlobeTranslucency {
    /// 创建新的半透明设置。
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Default::default()
        }
    }

    /// 计算面向 camera 表面的有效 alpha。
    pub fn front_alpha(&self) -> f64 {
        if self.enabled {
            self.front_face_alpha
        } else {
            1.0
        }
    }

    /// 计算背离 camera 表面的有效 alpha。
    pub fn back_alpha(&self) -> f64 {
        if self.enabled {
            self.back_face_alpha
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_globe_config_default() {
        let config = GlobeConfig::default();
        assert!(config.show);
        assert!(!config.depth_test_against_terrain);
        assert!(!config.translucency_enabled);
        assert!(config.show_ground_atmosphere);
        assert!((config.maximum_screen_space_error - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_globe_surface_normal() {
        let globe = GlobeSurface::new();
        // 在北极点，法线应指向上方（Z 轴）
        let north_pole = DVec3::new(0.0, 0.0, 6356752.3142);
        let normal = globe.get_surface_normal(north_pole);
        assert!(normal.z > 0.99);
    }

    #[test]
    fn test_horizon_distance() {
        let globe = GlobeSurface::new();
        // 在 1000m 高度，地平线应约为 113 km
        let dist = globe.horizon_distance(1000.0);
        assert!(dist > 100_000.0 && dist < 120_000.0);
    }

    #[test]
    fn test_horizon_distance_zero() {
        let globe = GlobeSurface::new();
        let dist = globe.horizon_distance(0.0);
        assert!(dist.abs() < 1e-6);
    }

    #[test]
    fn test_horizon_dip_angle() {
        let globe = GlobeSurface::new();
        // 在 1000m 处，俯角应很小（约 0.03 rad）
        let dip = globe.horizon_dip_angle(1000.0);
        assert!(dip > 0.0 && dip < 0.1);
    }

    #[test]
    fn test_pick_globe_from_above() {
        let globe = GlobeSurface::new();
        // 从上方俯瞰的射线
        let origin = DVec3::new(0.0, 0.0, 10_000_000.0);
        let direction = DVec3::new(0.0, 0.0, -1.0);

        let hit = globe.pick(origin, direction);
        assert!(hit.is_some());

        let hit_point = hit.unwrap();
        // 应在靠近北极点的表面处命中
        assert!((hit_point.z - 6356752.3142).abs() < 1.0);
    }

    #[test]
    fn test_pick_globe_miss() {
        let globe = GlobeSurface::new();
        // 指向远离地球方向的射线
        let origin = DVec3::new(0.0, 0.0, 10_000_000.0);
        let direction = DVec3::new(0.0, 0.0, 1.0);

        let hit = globe.pick(origin, direction);
        assert!(hit.is_none());
    }

    #[test]
    fn test_visible_hemisphere() {
        let globe = GlobeSurface::new();
        let camera = DVec3::new(0.0, 0.0, 10_000_000.0);

        // 北极点应可见
        let north_pole = DVec3::new(0.0, 0.0, 6356752.3142);
        assert!(globe.is_on_visible_hemisphere(north_pole, camera));

        // 南极点应不可见
        let south_pole = DVec3::new(0.0, 0.0, -6356752.3142);
        assert!(!globe.is_on_visible_hemisphere(south_pole, camera));
    }

    #[test]
    fn test_tile_sse_computation() {
        let globe = GlobeSurface::new();

        // 高几何误差、近距离 → 高 SSE
        let sse = globe.compute_tile_sse(1000.0, 1000.0, 1080.0, 1.0);
        assert!(sse > 100.0);

        // 低几何误差、远距离 → 低 SSE
        let sse = globe.compute_tile_sse(1.0, 1_000_000.0, 1080.0, 1.0);
        assert!(sse < 1.0);
    }

    #[test]
    fn test_should_refine_tile() {
        let globe = GlobeSurface::new();
        assert!(globe.should_refine_tile(10.0)); // SSE > 2.0
        assert!(!globe.should_refine_tile(1.0)); // SSE < 2.0
    }

    #[test]
    fn test_globe_translucency() {
        let translucency = GlobeTranslucency::new(true);
        assert!(translucency.enabled);
        assert!((translucency.front_alpha() - 1.0).abs() < 1e-10);

        let mut t2 = GlobeTranslucency::new(true);
        t2.front_face_alpha = 0.5;
        assert!((t2.front_alpha() - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_globe_translucency_disabled() {
        let translucency = GlobeTranslucency::default();
        assert!(!translucency.enabled);
        // 禁用时，始终返回 1.0
        assert!((translucency.front_alpha() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_get_height() {
        let globe = GlobeSurface::new();
        let carto = Cartographic::from_radians(0.0, 0.0, 0.0);
        let height = globe.get_height(&carto);
        assert_eq!(height, Some(0.0));
    }

    #[test]
    fn test_globe_surface_custom_ellipsoid() {
        let ellipsoid = Ellipsoid::new(1000.0, 1000.0, 1000.0);
        let globe = GlobeSurface::with_ellipsoid(ellipsoid);

        let dist = globe.horizon_distance(100.0);
        // 更小的椭球 → 更短的地平线距离
        assert!(dist < 1000.0);
    }
}
