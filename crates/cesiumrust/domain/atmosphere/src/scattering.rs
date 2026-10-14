//! 大气散射模型。
//!
//! 提供 Rayleigh/Mie 散射近似所需的大气参数与相函数，
//! 用于实现天空颜色计算。
//!
//! 为实现天空颜色计算而提供的 Rayleigh 散射近似。

use glam::DVec3;

/// 地球大气的物理常量。
pub mod constants {
    /// 地球赤道半径（米）。
    pub const EARTH_RADIUS: f64 = 6378137.0;

    /// 大气层高度（米）。
    pub const ATMOSPHERE_HEIGHT: f64 = 100000.0;

    /// 外半径（地球 + 大气层）。
    pub const OUTER_RADIUS: f64 = EARTH_RADIUS + ATMOSPHERE_HEIGHT;

    /// 海平面处的 Rayleigh 散射系数（每米）。
    /// 对应 RGB 波长（680nm、550nm、440nm）的值。
    pub const RAYLEIGH_COEFFICIENTS: [f64; 3] = [5.8e-6, 13.5e-6, 33.1e-6];

    /// Rayleigh 标高（米）。
    pub const RAYLEIGH_SCALE_HEIGHT: f64 = 8000.0;

    /// Mie 散射系数。
    pub const MIE_COEFFICIENT: f64 = 21e-6;

    /// Mie 标高（米）。
    pub const MIE_SCALE_HEIGHT: f64 = 1200.0;

    /// Mie 优先散射方向（各向异性）。
    pub const MIE_ANISOTROPY: f64 = 0.758;

    /// 太阳强度。
    pub const SOLAR_INTENSITY: f64 = 20.0;
}

/// 用于散射计算的大气参数。
#[derive(Debug, Clone)]
pub struct AtmosphereParameters {
    /// 内半径（地球表面）。
    pub inner_radius: f64,
    /// 外半径（大气层边界）。
    pub outer_radius: f64,
    /// Rayleigh 系数 [R, G, B]。
    pub rayleigh_coefficients: [f64; 3],
    /// Rayleigh 标高。
    pub rayleigh_scale_height: f64,
    /// Mie 系数。
    pub mie_coefficient: f64,
    /// Mie 标高。
    pub mie_scale_height: f64,
    /// Mie 各向异性（g 参数）。
    pub mie_anisotropy: f64,
    /// 太阳强度乘子。
    pub solar_intensity: f64,
}

impl Default for AtmosphereParameters {
    /// 默认大气参数：采用地球半径与典型 Rayleigh/Mie 散射系数。
    fn default() -> Self {
        Self {
            inner_radius: constants::EARTH_RADIUS,
            outer_radius: constants::OUTER_RADIUS,
            rayleigh_coefficients: constants::RAYLEIGH_COEFFICIENTS,
            rayleigh_scale_height: constants::RAYLEIGH_SCALE_HEIGHT,
            mie_coefficient: constants::MIE_COEFFICIENT,
            mie_scale_height: constants::MIE_SCALE_HEIGHT,
            mie_anisotropy: constants::MIE_ANISOTROPY,
            solar_intensity: constants::SOLAR_INTENSITY,
        }
    }
}

/// 计算 Rayleigh 相位函数。
///
/// # 参数
/// * `cos_theta` - 散射角的余弦
pub fn rayleigh_phase(cos_theta: f64) -> f64 {
    3.0 / (16.0 * std::f64::consts::PI) * (1.0 + cos_theta * cos_theta)
}

/// 计算 Henyey-Greenstein（Mie）相位函数。
///
/// # 参数
/// * `cos_theta` - 散射角的余弦
/// * `g` - 各向异性参数（-1 到 1）
pub fn mie_phase(cos_theta: f64, g: f64) -> f64 {
    let g2 = g * g;
    let num = (1.0 - g2) * (1.0 + cos_theta * cos_theta);
    let denom = (2.0 + g2) * (1.0 + g2 - 2.0 * g * cos_theta).powf(1.5);
    num / (4.0 * std::f64::consts::PI * denom)
}

/// 使用指数衰减计算给定高度处的大气密度。
///
/// # 参数
/// * `height` - 距表面高度（米）
/// * `scale_height` - 标高（米）
pub fn atmospheric_density(height: f64, scale_height: f64) -> f64 {
    (-height / scale_height).exp()
}

/// 针对给定的视线方向与太阳方向计算近似天空颜色。
///
/// 这是一个简化的单次散射近似。
///
/// # 参数
/// * `view_direction` - 来自相机的归一化视线方向
/// * `sun_direction` - 指向太阳的归一化方向
/// * `camera_height` - 相机距表面高度（米）
/// * `params` - 大气参数
///
/// # 返回
/// 近似天空颜色 [R, G, B]（线性，可能超过 1.0）
pub fn compute_sky_color(
    view_direction: DVec3,
    sun_direction: DVec3,
    camera_height: f64,
    params: &AtmosphereParameters,
) -> [f64; 3] {
    let cos_theta = view_direction.dot(sun_direction);

    // 相位函数
    let rayleigh_p = rayleigh_phase(cos_theta);
    let mie_p = mie_phase(cos_theta, params.mie_anisotropy);

    // 相机高度处的密度
    let height_above_surface = camera_height.max(0.0);
    let rayleigh_density = atmospheric_density(height_above_surface, params.rayleigh_scale_height);
    let mie_density = atmospheric_density(height_above_surface, params.mie_scale_height);

    // 光学深度近似（简化）
    let path_length = params.outer_radius - params.inner_radius;

    let mut color = [0.0f64; 3];
    for (c, beta) in color.iter_mut().zip(params.rayleigh_coefficients.iter()) {
        let rayleigh = beta * rayleigh_density * rayleigh_p * path_length;
        let mie = params.mie_coefficient * mie_density * mie_p * path_length;
        *c = (rayleigh + mie) * params.solar_intensity;
    }

    color
}

/// 基于太阳高度角计算地平线辉光颜色。
///
/// # 参数
/// * `sun_elevation` - 太阳高度角（弧度，负值 = 地平线以下）
///
/// # 返回
/// 地平线辉光颜色 [R, G, B]
pub fn compute_horizon_glow(sun_elevation: f64) -> [f64; 3] {
    // 太阳位于地平线以下：偏红的辉光
    // 太阳位于地平线以上：偏白蓝
    let t = (sun_elevation / (std::f64::consts::FRAC_PI_2)).clamp(-1.0, 1.0);

    if t < 0.0 {
        // 日落/日出颜色
        let factor = 1.0 + t; // -90° 时为 0，地平线处为 1
        [
            0.8 * factor,
            0.3 * factor * factor,
            0.1 * factor * factor * factor,
        ]
    } else {
        // 白天天空
        [0.4 + 0.3 * t, 0.6 + 0.2 * t, 0.9 + 0.1 * t]
    }
}

/// 天空盒配置。
#[derive(Debug, Clone)]
pub struct SkyBoxConfig {
    /// 是否显示天空盒。
    pub show: bool,
    /// 6 个面的源 URI [+X, -X, +Y, -Y, +Z, -Z]。
    pub sources: [Option<String>; 6],
    /// 旋转角度（弧度）。
    pub rotation: f64,
}

impl Default for SkyBoxConfig {
    /// 默认天空盒配置：显示、无纹理源、零旋转。
    fn default() -> Self {
        Self {
            show: true,
            sources: [None, None, None, None, None, None],
            rotation: 0.0,
        }
    }
}

/// 场景的光照配置。
#[derive(Debug, Clone)]
pub struct LightingConfig {
    /// ECEF 中的太阳方向（归一化）。
    pub sun_direction: DVec3,
    /// 太阳颜色 [R, G, B]。
    pub sun_color: [f64; 3],
    /// 太阳强度。
    pub sun_intensity: f64,
    /// 环境光颜色 [R, G, B]。
    pub ambient_color: [f64; 3],
    /// 环境光强度。
    pub ambient_intensity: f64,
    /// 是否启用阴影。
    pub shadows_enabled: bool,
}

impl Default for LightingConfig {
    /// 默认光照配置：沿 +X 的暖色太阳光与基础环境光。
    fn default() -> Self {
        Self {
            sun_direction: DVec3::new(1.0, 0.0, 0.0),
            sun_color: [1.0, 1.0, 0.9],
            sun_intensity: 1.0,
            ambient_color: [0.1, 0.1, 0.15],
            ambient_intensity: 0.3,
            shadows_enabled: true,
        }
    }
}

impl LightingConfig {
    /// 根据儒略日期更新太阳方向。
    pub fn update_from_julian_date(&mut self, julian_date: f64) {
        self.sun_direction = crate::celestial::compute_sun_direction_eci(julian_date);
    }

    /// 计算给定位置处的太阳高度角。
    ///
    /// # 参数
    /// * `surface_normal` - 该位置处的表面法线（归一化，ECEF）
    pub fn sun_elevation_at(&self, surface_normal: DVec3) -> f64 {
        let cos_angle = surface_normal.dot(self.sun_direction);
        std::f64::consts::FRAC_PI_2 - cos_angle.acos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn test_rayleigh_phase() {
        // 前向散射（cos_theta = 1）
        let forward = rayleigh_phase(1.0);
        // 后向散射（cos_theta = -1）
        let backward = rayleigh_phase(-1.0);
        // Rayleigh 对称
        assert!((forward - backward).abs() < 1e-10);

        // 侧向散射（cos_theta = 0）
        let side = rayleigh_phase(0.0);
        assert!(side < forward);
    }

    #[test]
    fn test_mie_phase() {
        let g = 0.758;
        // 对于正的 g，前向散射应最强
        let forward = mie_phase(1.0, g);
        let backward = mie_phase(-1.0, g);
        assert!(forward > backward);
    }

    #[test]
    fn test_atmospheric_density() {
        // 在海平面，密度 = 1
        assert!((atmospheric_density(0.0, 8000.0) - 1.0).abs() < 1e-10);
        // 在一个标高处，密度 = 1/e
        assert!((atmospheric_density(8000.0, 8000.0) - 1.0 / std::f64::consts::E).abs() < 1e-10);
    }

    #[test]
    fn test_compute_sky_color() {
        let params = AtmosphereParameters::default();
        let view = DVec3::new(0.0, 0.0, 1.0);
        let sun = DVec3::new(0.0, 0.0, 1.0);

        let color = compute_sky_color(view, sun, 0.0, &params);

        // 蓝色通道应最强（Rayleigh 散射）
        assert!(color[2] > color[0]); // 蓝 > 红
    }

    #[test]
    fn test_horizon_glow_sunset() {
        // 太阳略低于地平线
        let color = compute_horizon_glow(-0.1);
        assert!(color[0] > color[2]); // 日落时红 > 蓝
    }

    #[test]
    fn test_horizon_glow_noon() {
        // 太阳在头顶
        let color = compute_horizon_glow(PI / 2.0);
        assert!(color[2] > color[0]); // 正午时蓝 > 红
    }

    #[test]
    fn test_lighting_config() {
        let config = LightingConfig::default();
        assert!((config.sun_direction.length() - 1.0).abs() < 1e-10);
        assert!(config.shadows_enabled);
    }

    #[test]
    fn test_sun_elevation() {
        let config = LightingConfig {
            sun_direction: DVec3::new(0.0, 0.0, 1.0), // 太阳直射头顶（Z 轴向上）
            ..Default::default()
        };

        // 表面法线指向太阳
        let elevation = config.sun_elevation_at(DVec3::new(0.0, 0.0, 1.0));
        assert!((elevation - PI / 2.0).abs() < 1e-10); // 90 度

        // 表面法线与太阳垂直
        let elevation = config.sun_elevation_at(DVec3::new(1.0, 0.0, 0.0));
        assert!(elevation.abs() < 1e-10); // 0 度（地平线）
    }
}
