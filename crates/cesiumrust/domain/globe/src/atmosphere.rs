//! 近地大气与天空渲染。
//!
//! 覆盖三类天空效果：
//! - 天空大气散射与着色（[`SkyAtmosphereConfig`]）
//! - 恒星背景的天空盒（[`SkyBoxConfig`]）
//! - 从地表观察的近地大气参数与 [`GlobeLighting`] 光照模型

use glam::DVec3;

/// 天空大气配置。
///
/// 控制大气层的可见性与色调偏移，并以每位置半径界定散射壳层的外沿。
#[derive(Debug, Clone)]
pub struct SkyAtmosphereConfig {
    /// 是否显示天空大气。
    pub show: bool,
    /// 色相偏移（-1.0 到 1.0）。
    pub hue_shift: f64,
    /// 饱和度偏移（-1.0 到 1.0）。
    pub saturation_shift: f64,
    /// 亮度偏移（-1.0 到 1.0）。
    pub brightness_shift: f64,
    /// 大气的每位置半径（米）。
    pub atmosphere_radius: f64,
}

impl Default for SkyAtmosphereConfig {
    /// 默认：显示大气、三个色调偏移均为 0，大气半径取地球半径 + 60km。
    fn default() -> Self {
        Self {
            show: true,
            hue_shift: 0.0,
            saturation_shift: 0.0,
            brightness_shift: 0.0,
            atmosphere_radius: 6378137.0 + 60000.0, // 地球半径 + 60km 大气
        }
    }
}

/// 用于恒星渲染的天空盒配置。
///
/// 以立方体贴图六面源贴图包裹场景，半径极大以模拟无限远的恒星背景。
#[derive(Debug, Clone)]
pub struct SkyBoxConfig {
    /// 是否显示天空盒。
    pub show: bool,
    /// 立方体贴图各面的源 URL [px, nx, py, ny, pz, nz]。
    pub sources: Option<[String; 6]>,
    /// 恒星球半径。
    pub radius: f64,
}

impl Default for SkyBoxConfig {
    /// 默认：显示天空盒、暂无面源贴图，恒星球半径取极大值 1e15 以近似无限远。
    fn default() -> Self {
        Self {
            show: true,
            sources: None,
            radius: 1e15, // 为恒星设置极大的半径
        }
    }
}

/// 用于从地表观察渲染的近地大气参数。
#[derive(Debug, Clone)]
pub struct GroundAtmosphere {
    /// Rayleigh 散射系数 [r, g, b]。
    pub rayleigh_coefficients: [f64; 3],
    /// Mie 散射系数。
    pub mie_coefficient: f64,
    /// Mie 方向因子（g）。
    pub mie_g: f64,
    /// 大气标高（米）。
    pub scale_height: f64,
    /// 太阳强度因子。
    pub sun_intensity: f64,
}

impl Default for GroundAtmosphere {
    /// 默认：采用标准中纬度大气参数——蓝天 Rayleigh 散射、常见 Mie 系数与
    /// 8000m 标高、太阳强度 20.0，作为天空色计算的基线。
    fn default() -> Self {
        Self {
            // 标准 Rayleigh 散射（蓝天）
            rayleigh_coefficients: [5.5e-6, 13.0e-6, 22.4e-6],
            mie_coefficient: 21e-6,
            mie_g: 0.758,
            scale_height: 8000.0,
            sun_intensity: 20.0,
        }
    }
}

impl GroundAtmosphere {
    /// 计算给定视图与太阳方向下的天空颜色。
    ///
    /// # 参数
    /// * `view_direction` - 归一化的视图方向
    /// * `sun_direction` - 指向太阳的归一化方向
    /// * `camera_height` - camera 高于地表的高度（米）
    ///
    /// # 返回
    /// RGB 颜色 [0.0-1.0]
    pub fn compute_sky_color(
        &self,
        view_direction: DVec3,
        sun_direction: DVec3,
        camera_height: f64,
    ) -> [f64; 3] {
        let cos_theta = view_direction.dot(sun_direction);

        // Rayleigh 相位函数
        let rayleigh_phase = 0.75 * (1.0 + cos_theta * cos_theta);

        // Mie 相位函数（Henyey-Greenstein）
        let g2 = self.mie_g * self.mie_g;
        let mie_phase = (1.0 - g2)
            / (4.0 * std::f64::consts::PI * (1.0 + g2 - 2.0 * self.mie_g * cos_theta).powf(1.5));

        // 光学深度（简化）
        let height_factor = (-camera_height / self.scale_height).exp();

        let mut color = [0.0f64; 3];
        for (c, beta) in color.iter_mut().zip(self.rayleigh_coefficients.iter()) {
            let rayleigh = beta * rayleigh_phase;
            let mie = self.mie_coefficient * mie_phase;
            let optical_depth = (rayleigh + mie) * height_factor;

            // 透射率
            let transmittance = (-optical_depth * 1000.0).exp();

            // 内散射
            let in_scatter = (1.0 - transmittance) * self.sun_intensity;

            *c = (rayleigh / (rayleigh + mie + 1e-10)) * in_scatter;
        }

        // 色调映射（简化 Reinhard）
        for c in color.iter_mut() {
            *c = *c / (1.0 + *c);
            *c = c.clamp(0.0, 1.0);
        }

        color
    }

    /// 计算日出/日落附近的地平线辉光颜色。
    ///
    /// # 参数
    /// * `sun_elevation` - 太阳高度角（弧度）
    ///
    /// # 返回
    /// 地平线辉光的 RGB 颜色
    pub fn compute_horizon_glow(&self, sun_elevation: f64) -> [f64; 3] {
        // 当太阳接近地平线时辉光最强
        let t = (-sun_elevation.abs() / 0.2).exp();

        // 橙/红色辉光
        [
            (1.0 * t).clamp(0.0, 1.0),
            (0.4 * t).clamp(0.0, 1.0),
            (0.1 * t).clamp(0.0, 1.0),
        ]
    }

    /// 计算天顶颜色（头顶正上方的天空）。
    pub fn compute_zenith_color(&self, sun_elevation: f64) -> [f64; 3] {
        // 白天为蓝天，夜晚变暗
        let day_factor = (sun_elevation / 0.3).clamp(0.0, 1.0);

        [
            0.1 * day_factor,
            0.3 * day_factor,
            0.8 * day_factor,
        ]
    }
}

/// 地球渲染的光照配置。
///
/// 描述太阳方向/颜色、环境光、昼夜终止线与水面镜面等参数，供漫反射与高光计算取用。
#[derive(Debug, Clone)]
pub struct GlobeLighting {
    /// 是否启用光照。
    pub enabled: bool,
    /// 太阳方向（归一化，ECEF 中）。
    pub sun_direction: DVec3,
    /// 太阳颜色 [r, g, b]。
    pub sun_color: [f64; 3],
    /// 环境光颜色 [r, g, b]。
    pub ambient_color: [f64; 3],
    /// 是否显示昼夜终止线。
    pub show_terminator: bool,
    /// 水面的镜面反射强度。
    pub specular_intensity: f64,
}

impl Default for GlobeLighting {
    /// 默认：关闭光照（保留恒等明暗），太阳沿 +X、暖白色，环境光微蓝，展示昼夜线、镜面强度 0.5。
    fn default() -> Self {
        Self {
            enabled: false,
            sun_direction: DVec3::X,
            sun_color: [1.0, 1.0, 0.9],
            ambient_color: [0.1, 0.1, 0.15],
            show_terminator: true,
            specular_intensity: 0.5,
        }
    }
}

impl GlobeLighting {
    /// 计算表面点处的漫反射光照因子。
    ///
    /// # 参数
    /// * `surface_normal` - 该点处的表面法线
    ///
    /// # 返回
    /// 漫反射因子 [0.0-1.0]
    pub fn compute_diffuse(&self, surface_normal: DVec3) -> f64 {
        if !self.enabled {
            return 1.0;
        }
        surface_normal.dot(self.sun_direction).max(0.0)
    }

    /// 计算水面的镜面高光。
    ///
    /// # 参数
    /// * `surface_normal` - 表面法线
    /// * `view_direction` - 从表面指向 camera 的方向
    ///
    /// # 返回
    /// 镜面反射强度 [0.0-1.0]
    pub fn compute_specular(&self, surface_normal: DVec3, view_direction: DVec3) -> f64 {
        if !self.enabled || self.specular_intensity <= 0.0 {
            return 0.0;
        }

        // Blinn-Phong 镜面反射
        let half_vector = (self.sun_direction + view_direction).normalize();
        let n_dot_h = surface_normal.dot(half_vector).max(0.0);

        // 水面的高光泽度
        n_dot_h.powf(64.0) * self.specular_intensity
    }

    /// 计算表面的最终受照颜色。
    pub fn compute_lit_color(
        &self,
        base_color: [f64; 3],
        surface_normal: DVec3,
        view_direction: DVec3,
        is_water: bool,
    ) -> [f64; 3] {
        if !self.enabled {
            return base_color;
        }

        let diffuse = self.compute_diffuse(surface_normal);

        let mut result = [0.0f64; 3];
        for i in 0..3 {
            let sun_contrib = base_color[i] * self.sun_color[i] * diffuse;
            let ambient_contrib = base_color[i] * self.ambient_color[i];
            result[i] = (sun_contrib + ambient_contrib).clamp(0.0, 1.0);
        }

        // 为水面添加镜面反射
        if is_water {
            let specular = self.compute_specular(surface_normal, view_direction);
            for (r, sun) in result.iter_mut().zip(self.sun_color.iter()) {
                *r = (*r + specular * sun).clamp(0.0, 1.0);
            }
        }

        result
    }

    /// 计算背光侧颜色（城市灯光近似）。
    pub fn compute_night_color(&self, base_color: [f64; 3]) -> [f64; 3] {
        // 在背光侧显著变暗
        [
            base_color[0] * 0.02,
            base_color[1] * 0.02,
            base_color[2] * 0.05,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    #[test]
    fn test_sky_atmosphere_default() {
        let config = SkyAtmosphereConfig::default();
        assert!(config.show);
        assert!((config.hue_shift).abs() < 1e-10);
        assert!(config.atmosphere_radius > 6378137.0);
    }

    #[test]
    fn test_sky_box_default() {
        let config = SkyBoxConfig::default();
        assert!(config.show);
        assert!(config.sources.is_none());
        assert!(config.radius > 1e10);
    }

    #[test]
    fn test_ground_atmosphere_day() {
        let atmosphere = GroundAtmosphere::default();

        // 抬头仰望，太阳在头顶
        let view = DVec3::new(0.0, 0.0, 1.0);
        let sun = DVec3::new(0.0, 0.0, 1.0);

        let color = atmosphere.compute_sky_color(view, sun, 0.0);

        // 应偏蓝色
        assert!(color[2] > color[0]); // 蓝 > 红
    }

    #[test]
    fn test_ground_atmosphere_sunset() {
        let atmosphere = GroundAtmosphere::default();

        // 太阳位于地平线
        let view = DVec3::new(1.0, 0.0, 0.0);
        let sun = DVec3::new(1.0, 0.0, 0.0);

        let color = atmosphere.compute_sky_color(view, sun, 0.0);

        // 所有通道均应为有效值
        for c in &color {
            assert!(*c >= 0.0 && *c <= 1.0);
        }
    }

    #[test]
    fn test_horizon_glow_sunset() {
        let atmosphere = GroundAtmosphere::default();

        // 太阳略低于地平线
        let glow = atmosphere.compute_horizon_glow(-0.1);

        // 应呈暖色调
        assert!(glow[0] > glow[2]); // 红 > 蓝
    }

    #[test]
    fn test_horizon_glow_noon() {
        let atmosphere = GroundAtmosphere::default();

        // 太阳高挂天空
        let glow = atmosphere.compute_horizon_glow(FRAC_PI_2);

        // 应仅有极少的辉光
        assert!(glow[0] < 0.1);
    }

    #[test]
    fn test_zenith_color_day() {
        let atmosphere = GroundAtmosphere::default();
        let color = atmosphere.compute_zenith_color(0.5);

        // 蓝天
        assert!(color[2] > color[0]);
    }

    #[test]
    fn test_zenith_color_night() {
        let atmosphere = GroundAtmosphere::default();
        let color = atmosphere.compute_zenith_color(-0.5);

        // 暗夜空
        assert!(color[2] < 0.1);
    }

    #[test]
    fn test_globe_lighting_disabled() {
        let lighting = GlobeLighting::default();
        assert!(!lighting.enabled);

        let normal = DVec3::Z;
        assert!((lighting.compute_diffuse(normal) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_globe_lighting_enabled() {
        let lighting = GlobeLighting {
            enabled: true,
            sun_direction: DVec3::new(0.0, 0.0, 1.0),
            ..Default::default()
        };

        // 面向太阳的表面
        let normal = DVec3::new(0.0, 0.0, 1.0);
        let diffuse = lighting.compute_diffuse(normal);
        assert!((diffuse - 1.0).abs() < 1e-10);

        // 背离太阳的表面
        let normal_away = DVec3::new(0.0, 0.0, -1.0);
        let diffuse_away = lighting.compute_diffuse(normal_away);
        assert!(diffuse_away.abs() < 1e-10);
    }

    #[test]
    fn test_specular_water() {
        let lighting = GlobeLighting {
            enabled: true,
            sun_direction: DVec3::new(0.0, 0.0, 1.0),
            ..Default::default()
        };

        let normal = DVec3::new(0.0, 0.0, 1.0);
        let view = DVec3::new(0.0, 0.0, 1.0);

        let specular = lighting.compute_specular(normal, view);
        assert!(specular > 0.0);
    }

    #[test]
    fn test_lit_color() {
        let lighting = GlobeLighting {
            enabled: true,
            sun_direction: DVec3::Z,
            ..Default::default()
        };

        let base = [0.5, 0.5, 0.5];
        let normal = DVec3::Z;
        let view = DVec3::Z;

        let lit = lighting.compute_lit_color(base, normal, view, false);

        // 应比环境光更亮
        assert!(lit[0] > 0.1);
    }

    #[test]
    fn test_night_color() {
        let lighting = GlobeLighting::default();
        let base = [0.5, 0.5, 0.5];
        let night = lighting.compute_night_color(base);

        // 应非常暗
        assert!(night[0] < 0.05);
        assert!(night[1] < 0.05);
    }
}
