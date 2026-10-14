//! 星球与天空大气增强。
//!
//! 涵盖三类天空渲染能力的领域模型：
//! - 天球上的恒星表渲染（星等、颜色与点大小）
//! - 天空大气的 HSB 偏移、动态光照与逐片元参数
//! - 基于 TEME 框架的天空盒状态
//!
//! 领域层 —— 纯 Rust，f64 精度。

// 遗留移植风格技术债（deferred.md #18）；在 M13 lint 清理或本文件在其里程碑被重写时重新审视
#![allow(clippy::field_reassign_with_default)]
use glam::DVec3;

// ─── 星表 ───────────────────────────────────────────────────────────

/// 星表中的单颗恒星条目。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Star {
    /// 赤经（弧度，0..2π）。
    pub right_ascension: f64,
    /// 赤纬（弧度，-π/2..π/2）。
    pub declination: f64,
    /// 视星等（越小越亮）。
    pub magnitude: f64,
    /// 恒星色温（开尔文），用于光谱颜色。
    pub color_temperature: f64,
}

impl Star {
    /// 由以度为单位的赤经/赤纬和星等创建一颗恒星。
    pub fn from_degrees(ra_deg: f64, dec_deg: f64, magnitude: f64) -> Self {
        Self {
            right_ascension: ra_deg.to_radians(),
            declination: dec_deg.to_radians(),
            magnitude,
            color_temperature: 6500.0, // 默认白色
        }
    }

    /// 计算这颗恒星的单位方向向量（ECI/TEME 框架）。
    pub fn direction(&self) -> DVec3 {
        let cos_dec = self.declination.cos();
        DVec3::new(
            cos_dec * self.right_ascension.cos(),
            cos_dec * self.right_ascension.sin(),
            self.declination.sin(),
        )
    }

    /// 由星等计算视觉亮度（0..1）。
    ///
    /// 使用 Pogson 标度：亮度 ∝ 10^(-0.4 * magnitude)。
    /// 归一化使得星等 0 → 1.0，星等 6 → 约 0.004。
    pub fn brightness(&self) -> f64 {
        10.0_f64.powf(-0.4 * self.magnitude)
    }

    /// 根据恒星的色温计算近似的 RGB 颜色。
    ///
    /// 基于黑体辐射近似（Tanner Helland 算法）。
    pub fn spectral_color(&self) -> [f64; 3] {
        color_from_temperature(self.color_temperature)
    }
}

/// 星球配置与渲染参数。
///
/// 在天球上按星表渲染恒星，支持星等剔除与亮度调整。
#[derive(Debug, Clone)]
pub struct StarSphere {
    /// 是否显示星球。
    pub show: bool,
    /// 星表。
    pub stars: Vec<Star>,
    /// 渲染的最小星等（比它暗的恒星会被剔除）。
    pub minimum_magnitude: f64,
    /// 渲染的最大星等（比它亮的恒星会被剔除）。
    pub maximum_magnitude: f64,
    /// 恒星的点大小（像素，星等 0 的基准大小）。
    pub base_point_size: f64,
    /// 是否对恒星使用 HDR 渲染。
    pub use_hdr: bool,
    /// 整体亮度乘子。
    pub brightness_multiplier: f64,
}

impl Default for StarSphere {
    /// 默认星球：显示星表，星等范围 -2..6，启用 HDR。
    fn default() -> Self {
        Self {
            show: true,
            stars: Vec::new(),
            minimum_magnitude: -2.0,
            maximum_magnitude: 6.0,
            base_point_size: 3.0,
            use_hdr: true,
            brightness_multiplier: 1.0,
        }
    }
}

impl StarSphere {
    /// 创建一个带有内置亮星星表的星球。
    pub fn with_builtin_catalog() -> Self {
        Self {
            stars: builtin_bright_stars(),
            ..Default::default()
        }
    }

    /// 返回在星等范围内可见的恒星。
    pub fn visible_stars(&self) -> impl Iterator<Item = &Star> {
        self.stars
            .iter()
            .filter(|s| s.magnitude >= self.minimum_magnitude && s.magnitude <= self.maximum_magnitude)
    }

    /// 根据星等计算恒星的渲染点大小。
    ///
    /// 较亮的恒星（较低星等）获得较大的点大小。
    pub fn star_point_size(&self, star: &Star) -> f64 {
        let magnitude_range = self.maximum_magnitude - self.minimum_magnitude;
        if magnitude_range <= 0.0 {
            return self.base_point_size;
        }
        let t = (star.magnitude - self.minimum_magnitude) / magnitude_range;
        // 较亮（较低星等）→ 较大大小
        self.base_point_size * (1.0 - t * 0.7)
    }

    /// 计算恒星的最终渲染颜色（应用亮度后）。
    pub fn star_render_color(&self, star: &Star) -> [f64; 3] {
        let base_color = star.spectral_color();
        let brightness = star.brightness() * self.brightness_multiplier;
        [
            base_color[0] * brightness,
            base_color[1] * brightness,
            base_color[2] * brightness,
        ]
    }

    /// 向星表添加一颗恒星。
    pub fn add_star(&mut self, star: Star) {
        self.stars.push(star);
    }

    /// 返回星表中恒星的数量。
    pub fn star_count(&self) -> usize {
        self.stars.len()
    }
}

/// 最亮恒星的内置星表（Hipparcos/Yale BSC 的子集）。
fn builtin_bright_stars() -> Vec<Star> {
    vec![
        // Sirius (α CMa) —— 最亮的恒星
        Star { right_ascension: 101.287_f64.to_radians(), declination: (-16.716_f64).to_radians(), magnitude: -1.46, color_temperature: 9940.0 },
        // Canopus (α Car)
        Star { right_ascension: 95.988_f64.to_radians(), declination: (-52.696_f64).to_radians(), magnitude: -0.74, color_temperature: 7350.0 },
        // Arcturus (α Boo)
        Star { right_ascension: 213.915_f64.to_radians(), declination: 19.182_f64.to_radians(), magnitude: -0.05, color_temperature: 4286.0 },
        // Vega (α Lyr)
        Star { right_ascension: 279.234_f64.to_radians(), declination: 38.784_f64.to_radians(), magnitude: 0.03, color_temperature: 9602.0 },
        // Capella (α Aur)
        Star { right_ascension: 79.172_f64.to_radians(), declination: 45.998_f64.to_radians(), magnitude: 0.08, color_temperature: 4970.0 },
        // Rigel (β Ori)
        Star { right_ascension: 78.634_f64.to_radians(), declination: (-8.202_f64).to_radians(), magnitude: 0.13, color_temperature: 12100.0 },
        // Procyon (α CMi)
        Star { right_ascension: 114.825_f64.to_radians(), declination: 5.225_f64.to_radians(), magnitude: 0.34, color_temperature: 6530.0 },
        // Betelgeuse (α Ori)
        Star { right_ascension: 88.793_f64.to_radians(), declination: 7.407_f64.to_radians(), magnitude: 0.42, color_temperature: 3500.0 },
        // Altair (α Aql)
        Star { right_ascension: 297.696_f64.to_radians(), declination: 8.868_f64.to_radians(), magnitude: 0.77, color_temperature: 7550.0 },
        // Aldebaran (α Tau)
        Star { right_ascension: 68.980_f64.to_radians(), declination: 16.509_f64.to_radians(), magnitude: 0.85, color_temperature: 3910.0 },
        // Antares (α Sco)
        Star { right_ascension: 247.352_f64.to_radians(), declination: (-26.432_f64).to_radians(), magnitude: 1.09, color_temperature: 3660.0 },
        // Spica (α Vir)
        Star { right_ascension: 201.298_f64.to_radians(), declination: (-11.161_f64).to_radians(), magnitude: 1.04, color_temperature: 22400.0 },
        // Pollux (β Gem)
        Star { right_ascension: 116.329_f64.to_radians(), declination: 28.026_f64.to_radians(), magnitude: 1.14, color_temperature: 4666.0 },
        // Fomalhaut (α PsA)
        Star { right_ascension: 344.413_f64.to_radians(), declination: (-29.622_f64).to_radians(), magnitude: 1.16, color_temperature: 8590.0 },
        // Deneb (α Cyg)
        Star { right_ascension: 310.358_f64.to_radians(), declination: 45.280_f64.to_radians(), magnitude: 1.25, color_temperature: 8525.0 },
        // Regulus (α Leo)
        Star { right_ascension: 152.093_f64.to_radians(), declination: 11.967_f64.to_radians(), magnitude: 1.35, color_temperature: 12460.0 },
        // Castor (α Gem)
        Star { right_ascension: 113.650_f64.to_radians(), declination: 31.888_f64.to_radians(), magnitude: 1.58, color_temperature: 10286.0 },
        // Bellatrix (γ Ori)
        Star { right_ascension: 81.283_f64.to_radians(), declination: 6.350_f64.to_radians(), magnitude: 1.64, color_temperature: 22000.0 },
        // Alnilam (ε Ori)
        Star { right_ascension: 84.053_f64.to_radians(), declination: (-1.202_f64).to_radians(), magnitude: 1.69, color_temperature: 27500.0 },
        // Polaris (α UMi) —— 北极星
        Star { right_ascension: 37.954_f64.to_radians(), declination: 89.264_f64.to_radians(), magnitude: 1.98, color_temperature: 6015.0 },
    ]
}

/// 从黑体温度（开尔文）近似 RGB 颜色。
///
/// 基于 Tanner Helland 的色温转 RGB 算法。
fn color_from_temperature(kelvin: f64) -> [f64; 3] {
    let temp = kelvin.clamp(1000.0, 40000.0) / 100.0;

    // 红
    let r = if temp <= 66.0 {
        1.0
    } else {
        let x = temp - 60.0;
        (329.698727446 * x.powf(-0.1332047592) / 255.0).clamp(0.0, 1.0)
    };

    // 绿
    let g = if temp <= 66.0 {
        (99.4708025861 * temp.ln() - 161.1195681661) / 255.0
    } else {
        let x = temp - 60.0;
        (288.1221695283 * x.powf(-0.0755148492) / 255.0).clamp(0.0, 1.0)
    };
    let g = g.clamp(0.0, 1.0);

    // 蓝
    let b = if temp >= 66.0 {
        1.0
    } else if temp <= 19.0 {
        0.0
    } else {
        let x = temp - 10.0;
        (138.5177312231 * x.ln() - 305.0447927307) / 255.0
    };
    let b = b.clamp(0.0, 1.0);

    [r, g, b]
}

// ─── 天空大气增强 ──────────────────────────────────────

/// 动态大气光照类型。
///
/// 指定光照源为太阳、月亮还是固定头顶方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DynamicAtmosphereLighting {
    /// 使用太阳位置进行光照。
    #[default]
    Sun,
    /// 使用月亮位置进行光照。
    Moon,
    /// 将光源视为始终在头顶正上方（无动态光照）。
    None,
}

impl DynamicAtmosphereLighting {
    /// 返回在 shader uniform 中使用的枚举值。
    pub fn to_shader_value(&self) -> f64 {
        match self {
            Self::Sun => 1.0,
            Self::Moon => 2.0,
            Self::None => 0.0,
        }
    }
}

/// 用于大气渲染的色相-饱和度-亮度偏移。
///
/// 分别对应大气颜色的色相、饱和度、亮度三个偏移量。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HsbShift {
    /// 色相偏移（0.0 = 无偏移，1.0 = 完整旋转）。
    pub hue: f64,
    /// 饱和度偏移（-1.0 = 单色，0.0 = 无偏移）。
    pub saturation: f64,
    /// 亮度偏移（-1.0 = 完全黑暗，0.0 = 无偏移）。
    pub brightness: f64,
}

impl HsbShift {
    /// 将 HSB 偏移应用到 RGB 颜色。
    ///
    /// 转换为 HSB，应用偏移，再转换回来。
    pub fn apply(&self, color: [f64; 3]) -> [f64; 3] {
        if self.hue == 0.0 && self.saturation == 0.0 && self.brightness == 0.0 {
            return color;
        }

        let (mut h, mut s, mut b) = rgb_to_hsb(color[0], color[1], color[2]);

        // 应用色相偏移（环绕）
        h = (h + self.hue) % 1.0;
        if h < 0.0 {
            h += 1.0;
        }

        // 应用饱和度偏移
        s = (s + self.saturation).clamp(0.0, 1.0);

        // 应用亮度偏移
        b = (b + self.brightness).clamp(0.0, 1.0);

        hsb_to_rgb(h, s, b)
    }
}

/// 增强的天空大气参数。
///
/// 在基础 `AtmosphereParameters` 之上扩展逐片元、HSB 偏移与动态光照等能力。
#[derive(Debug, Clone)]
pub struct SkyAtmosphereConfig {
    /// 是否显示大气。
    pub show: bool,
    /// 逐片元而非逐顶点计算大气。
    pub per_fragment_atmosphere: bool,
    /// 用于计算天空大气颜色的光照强度。
    pub light_intensity: f64,
    /// Rayleigh 散射系数 [R, G, B]。
    pub rayleigh_coefficient: DVec3,
    /// Mie 散射系数 [R, G, B]。
    pub mie_coefficient: DVec3,
    /// Rayleigh 标高（米）。
    pub rayleigh_scale_height: f64,
    /// Mie 标高（米）。
    pub mie_scale_height: f64,
    /// Mie 各向异性（g 参数，-1..1）。
    pub mie_anisotropy: f64,
    /// 大气颜色的 HSB 偏移。
    pub hsb_shift: HsbShift,
    /// 动态光照类型。
    pub dynamic_lighting: DynamicAtmosphereLighting,
    /// 外椭球缩放因子（大气延伸到表面之外）。
    pub outer_ellipsoid_scale: f64,
    /// 内半径（地球表面，米）。
    pub inner_radius: f64,
}

impl Default for SkyAtmosphereConfig {
    /// 默认天空大气配置：显示、非逐片元、典型散射系数。
    fn default() -> Self {
        Self {
            show: true,
            per_fragment_atmosphere: false,
            light_intensity: 50.0,
            rayleigh_coefficient: DVec3::new(5.5e-6, 13.0e-6, 28.4e-6),
            mie_coefficient: DVec3::new(21e-6, 21e-6, 21e-6),
            rayleigh_scale_height: 10000.0,
            mie_scale_height: 3200.0,
            mie_anisotropy: 0.9,
            hsb_shift: HsbShift::default(),
            dynamic_lighting: DynamicAtmosphereLighting::Sun,
            outer_ellipsoid_scale: 1.025,
            inner_radius: 6378137.0, // WGS84 赤道半径
        }
    }
}

impl SkyAtmosphereConfig {
    /// 计算外半径（大气边界）。
    pub fn outer_radius(&self) -> f64 {
        self.inner_radius * self.outer_ellipsoid_scale
    }

    /// 针对给定的视角/太阳配置计算大气颜色。
    ///
    /// # 参数
    /// * `view_direction` - 来自相机的归一化视线方向
    /// * `sun_direction` - 指向太阳的归一化方向
    /// * `camera_height` - 相机距表面高度（米）
    pub fn compute_color(
        &self,
        view_direction: DVec3,
        sun_direction: DVec3,
        camera_height: f64,
    ) -> [f64; 3] {
        if !self.show {
            return [0.0; 3];
        }

        let cos_theta = view_direction.dot(sun_direction);

        // 相位函数
        let rayleigh_p = rayleigh_phase_fn(cos_theta);
        let mie_p = mie_phase_fn(cos_theta, self.mie_anisotropy);

        // 相机高度处的密度
        let height = camera_height.max(0.0);
        let rayleigh_density = (-height / self.rayleigh_scale_height).exp();
        let mie_density = (-height / self.mie_scale_height).exp();

        // 光学深度
        let path_length = self.outer_radius() - self.inner_radius;

        let mut color = [0.0f64; 3];
        let rayleigh = [self.rayleigh_coefficient.x, self.rayleigh_coefficient.y, self.rayleigh_coefficient.z];
        let mie = [self.mie_coefficient.x, self.mie_coefficient.y, self.mie_coefficient.z];
        for (c, (beta_r, beta_m)) in color.iter_mut().zip(rayleigh.iter().zip(mie.iter())) {
            let rayleigh_val = beta_r * rayleigh_density * rayleigh_p * path_length;
            let mie_val = beta_m * mie_density * mie_p * path_length;
            *c = (rayleigh_val + mie_val) * self.light_intensity;
        }

        // 应用 HSB 偏移
        self.hsb_shift.apply(color)
    }

    /// 返回半径与动态大气颜色的 uniform 向量。
    ///
    /// 将外半径、内半径与光照类型打包为一个三维 uniform。
    pub fn radii_and_dynamic_color(&self) -> DVec3 {
        DVec3::new(
            self.outer_radius(),
            self.inner_radius,
            self.dynamic_lighting.to_shader_value(),
        )
    }
}

/// Rayleigh 相位函数。
fn rayleigh_phase_fn(cos_theta: f64) -> f64 {
    3.0 / (16.0 * std::f64::consts::PI) * (1.0 + cos_theta * cos_theta)
}

/// Henyey-Greenstein（Mie）相位函数。
fn mie_phase_fn(cos_theta: f64, g: f64) -> f64 {
    let g2 = g * g;
    let num = (1.0 - g2) * (1.0 + cos_theta * cos_theta);
    let denom = (2.0 + g2) * (1.0 + g2 - 2.0 * g * cos_theta).powf(1.5);
    num / (4.0 * std::f64::consts::PI * denom)
}

// ─── Sky Box TEME 框架 ─────────────────────────────────────────────────

/// 支持 TEME（True Equator Mean Equinox）框架的天空盒。
///
/// 使用 TEME 轴进行恒星渲染，并按需施加绕极轴旋转。
#[derive(Debug, Clone)]
pub struct SkyBoxState {
    /// 是否显示天空盒。
    pub show: bool,
    /// 6 个立方体贴图面的源 URI [+X, -X, +Y, -Y, +Z, -Z]。
    pub sources: [Option<String>; 6],
    /// 用于 TEME 对齐、绕 Z 轴旋转的角度（弧度）。
    pub teme_rotation: f64,
}

impl Default for SkyBoxState {
    /// 默认天空盒状态：显示、无纹理面、零 TEME 旋转。
    fn default() -> Self {
        Self {
            show: true,
            sources: [None, None, None, None, None, None],
            teme_rotation: 0.0,
        }
    }
}

impl SkyBoxState {
    /// 针对给定的 GMST 角度计算 TEME 到 ECEF 的旋转矩阵。
    ///
    /// 天空盒定义在 TEME 轴中，必须旋转以与 ECEF 框架对齐
    /// 后进行渲染。
    pub fn teme_to_ecef_rotation(&self, gmst: f64) -> [[f64; 3]; 3] {
        let angle = gmst + self.teme_rotation;
        let cos_a = angle.cos();
        let sin_a = angle.sin();

        // 绕 Z 轴旋转
        [
            [cos_a, sin_a, 0.0],
            [-sin_a, cos_a, 0.0],
            [0.0, 0.0, 1.0],
        ]
    }

    /// 将 TEME 方向变换到 ECEF。
    pub fn teme_to_ecef(&self, teme_dir: DVec3, gmst: f64) -> DVec3 {
        let rot = self.teme_to_ecef_rotation(gmst);
        DVec3::new(
            rot[0][0] * teme_dir.x + rot[0][1] * teme_dir.y + rot[0][2] * teme_dir.z,
            rot[1][0] * teme_dir.x + rot[1][1] * teme_dir.y + rot[1][2] * teme_dir.z,
            rot[2][0] * teme_dir.x + rot[2][1] * teme_dir.y + rot[2][2] * teme_dir.z,
        )
    }

    /// 返回是否已定义全部 6 个面源。
    pub fn is_complete(&self) -> bool {
        self.sources.iter().all(|s| s.is_some())
    }
}

// ─── HSB 转换工具 ───────────────────────────────────────────────

/// 将 RGB（0..1）转换为 HSB（H: 0..1, S: 0..1, B: 0..1）。
fn rgb_to_hsb(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / delta) % 6.0 / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    let h = if h < 0.0 { h + 1.0 } else { h };

    let s = if max == 0.0 { 0.0 } else { delta / max };
    let brightness = max;

    (h, s, brightness)
}

/// 将 HSB（H: 0..1, S: 0..1, B: 0..1）转换为 RGB（0..1）。
fn hsb_to_rgb(h: f64, s: f64, b: f64) -> [f64; 3] {
    if s == 0.0 {
        return [b, b, b];
    }

    let h6 = h * 6.0;
    let i = h6.floor() as i32;
    let f = h6 - i as f64;
    let p = b * (1.0 - s);
    let q = b * (1.0 - s * f);
    let t = b * (1.0 - s * (1.0 - f));

    match i % 6 {
        0 => [b, t, p],
        1 => [q, b, p],
        2 => [p, b, t],
        3 => [p, q, b],
        4 => [t, p, b],
        _ => [b, p, q],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    // ─── 恒星测试 ─────────────────────────────────────────────────────

    #[test]
    fn test_star_direction_normalized() {
        let star = Star::from_degrees(101.287, -16.716, -1.46);
        let dir = star.direction();
        assert!((dir.length() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_star_direction_poles() {
        // 位于北天极的恒星
        let star = Star {
            right_ascension: 0.0,
            declination: PI / 2.0,
            magnitude: 2.0,
            color_temperature: 6500.0,
        };
        let dir = star.direction();
        assert!((dir.z - 1.0).abs() < 1e-10);
        assert!(dir.x.abs() < 1e-10);
        assert!(dir.y.abs() < 1e-10);
    }

    #[test]
    fn test_star_brightness_pogson() {
        let bright = Star { magnitude: 0.0, ..Star::from_degrees(0.0, 0.0, 0.0) };
        let dim = Star { magnitude: 5.0, ..Star::from_degrees(0.0, 0.0, 5.0) };

        // 星等 0 → 亮度 1.0
        assert!((bright.brightness() - 1.0).abs() < 1e-10);
        // 星等 5 → 约 0.01
        assert!(dim.brightness() < 0.02);
        assert!(dim.brightness() > 0.005);
    }

    #[test]
    fn test_star_spectral_color_hot() {
        // 炽热的蓝色恒星（20000K）
        let star = Star { color_temperature: 20000.0, ..Star::from_degrees(0.0, 0.0, 0.0) };
        let color = star.spectral_color();
        // 蓝色应占主导
        assert!(color[2] > color[0]);
    }

    #[test]
    fn test_star_spectral_color_cool() {
        // 较冷的红色恒星（3000K）
        let star = Star { color_temperature: 3000.0, ..Star::from_degrees(0.0, 0.0, 0.0) };
        let color = star.spectral_color();
        // 红色应占主导
        assert!(color[0] > color[2]);
    }

    #[test]
    fn test_star_sphere_builtin_catalog() {
        let sphere = StarSphere::with_builtin_catalog();
        assert_eq!(sphere.star_count(), 20);
        assert!(sphere.show);
    }

    #[test]
    fn test_star_sphere_visible_filter() {
        let mut sphere = StarSphere::default();
        sphere.minimum_magnitude = 0.0;
        sphere.maximum_magnitude = 2.0;
        sphere.add_star(Star::from_degrees(0.0, 0.0, -1.0)); // 太亮（低于最小值）
        sphere.add_star(Star::from_degrees(10.0, 10.0, 1.0)); // 可见
        sphere.add_star(Star::from_degrees(20.0, 20.0, 5.0)); // 太暗（高于最大值）

        let visible: Vec<_> = sphere.visible_stars().collect();
        assert_eq!(visible.len(), 1);
        assert!((visible[0].magnitude - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_star_point_size() {
        let sphere = StarSphere {
            minimum_magnitude: 0.0,
            maximum_magnitude: 6.0,
            base_point_size: 4.0,
            ..Default::default()
        };

        let bright = Star::from_degrees(0.0, 0.0, 0.0);
        let dim = Star::from_degrees(0.0, 0.0, 6.0);

        let bright_size = sphere.star_point_size(&bright);
        let dim_size = sphere.star_point_size(&dim);

        // 较亮的恒星应更大
        assert!(bright_size > dim_size);
        assert!((bright_size - 4.0).abs() < 1e-10); // 完整基准大小
    }

    #[test]
    fn test_star_render_color() {
        let sphere = StarSphere {
            brightness_multiplier: 2.0,
            ..Default::default()
        };
        let star = Star { magnitude: 0.0, color_temperature: 6500.0, ..Star::from_degrees(0.0, 0.0, 0.0) };
        let color = sphere.star_render_color(&star);

        // brightness = 10^(-0.4*0) * 2.0 = 2.0
        // 所有通道都应 > 0
        assert!(color[0] > 0.0);
        assert!(color[1] > 0.0);
        assert!(color[2] > 0.0);
    }

    // ─── 天空大气测试 ─────────────────────────────────────────────

    #[test]
    fn test_dynamic_lighting_values() {
        assert_eq!(DynamicAtmosphereLighting::Sun.to_shader_value(), 1.0);
        assert_eq!(DynamicAtmosphereLighting::Moon.to_shader_value(), 2.0);
        assert_eq!(DynamicAtmosphereLighting::None.to_shader_value(), 0.0);
    }

    #[test]
    fn test_hsb_shift_noop() {
        let shift = HsbShift::default();
        let color = [0.5, 0.3, 0.8];
        let result = shift.apply(color);
        assert!((result[0] - color[0]).abs() < 1e-10);
        assert!((result[1] - color[1]).abs() < 1e-10);
        assert!((result[2] - color[2]).abs() < 1e-10);
    }

    #[test]
    fn test_hsb_shift_brightness_down() {
        let shift = HsbShift { brightness: -0.5, ..Default::default() };
        let color = [1.0, 0.0, 0.0]; // 纯红，B=1.0
        let result = shift.apply(color);
        // 亮度应下降
        assert!(result[0] < 1.0);
    }

    #[test]
    fn test_hsb_shift_saturation_zero() {
        let shift = HsbShift { saturation: -1.0, ..Default::default() };
        let color = [1.0, 0.0, 0.0]; // 纯红，S=1.0
        let result = shift.apply(color);
        // 应变为灰度（所有通道相等）
        assert!((result[0] - result[1]).abs() < 1e-6);
        assert!((result[1] - result[2]).abs() < 1e-6);
    }

    #[test]
    fn test_sky_atmosphere_config_defaults() {
        let config = SkyAtmosphereConfig::default();
        assert!(config.show);
        assert!(!config.per_fragment_atmosphere);
        assert!((config.light_intensity - 50.0).abs() < 1e-10);
        assert!((config.mie_anisotropy - 0.9).abs() < 1e-10);
        assert!((config.outer_ellipsoid_scale - 1.025).abs() < 1e-10);
    }

    #[test]
    fn test_sky_atmosphere_outer_radius() {
        let config = SkyAtmosphereConfig::default();
        let outer = config.outer_radius();
        assert!((outer - 6378137.0 * 1.025).abs() < 1.0);
    }

    #[test]
    fn test_sky_atmosphere_compute_color() {
        let config = SkyAtmosphereConfig::default();
        let view = DVec3::new(0.0, 0.0, 1.0);
        let sun = DVec3::new(0.0, 0.0, 1.0);

        let color = config.compute_color(view, sun, 0.0);

        // 应产生非零颜色
        assert!(color[0] > 0.0 || color[1] > 0.0 || color[2] > 0.0);
    }

    #[test]
    fn test_sky_atmosphere_hidden() {
        let config = SkyAtmosphereConfig { show: false, ..Default::default() };
        let color = config.compute_color(DVec3::Z, DVec3::Z, 0.0);
        assert_eq!(color, [0.0; 3]);
    }

    #[test]
    fn test_radii_and_dynamic_color() {
        let config = SkyAtmosphereConfig::default();
        let v = config.radii_and_dynamic_color();
        assert!((v.x - config.outer_radius()).abs() < 1e-6);
        assert!((v.y - config.inner_radius).abs() < 1e-6);
        assert!((v.z - 1.0).abs() < 1e-10); // 太阳
    }

    // ─── Sky Box 测试 ─────────────────────────────────────────────────

    #[test]
    fn test_sky_box_teme_rotation() {
        let sky_box = SkyBoxState::default();
        let rot = sky_box.teme_to_ecef_rotation(0.0);

        // 在 GMST=0 时，旋转应为单位矩阵
        assert!((rot[0][0] - 1.0).abs() < 1e-10);
        assert!((rot[1][1] - 1.0).abs() < 1e-10);
        assert!((rot[2][2] - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_sky_box_teme_to_ecef() {
        let sky_box = SkyBoxState::default();
        let dir = DVec3::new(1.0, 0.0, 0.0);

        // 在 GMST=0 时，应保持不变
        let ecef = sky_box.teme_to_ecef(dir, 0.0);
        assert!((ecef - dir).length() < 1e-10);

        // 在 GMST=π/2 时，X 应旋转到 -Y
        let ecef_90 = sky_box.teme_to_ecef(dir, PI / 2.0);
        assert!(ecef_90.x.abs() < 1e-10);
        assert!((ecef_90.y - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn test_sky_box_is_complete() {
        let mut sky_box = SkyBoxState::default();
        assert!(!sky_box.is_complete());

        sky_box.sources = [
            Some("px.png".into()), Some("nx.png".into()),
            Some("py.png".into()), Some("ny.png".into()),
            Some("pz.png".into()), Some("nz.png".into()),
        ];
        assert!(sky_box.is_complete());
    }

    // ─── HSB 转换测试 ─────────────────────────────────────────────

    #[test]
    fn test_rgb_hsb_roundtrip() {
        let colors = [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.5, 0.3, 0.8],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
        ];

        for c in &colors {
            let (h, s, b) = rgb_to_hsb(c[0], c[1], c[2]);
            let rgb = hsb_to_rgb(h, s, b);
            assert!((rgb[0] - c[0]).abs() < 1e-6, "R mismatch for {:?}", c);
            assert!((rgb[1] - c[1]).abs() < 1e-6, "G mismatch for {:?}", c);
            assert!((rgb[2] - c[2]).abs() < 1e-6, "B mismatch for {:?}", c);
        }
    }

    #[test]
    fn test_color_temperature_white() {
        // 约 6500K 应大致为白色
        let color = color_from_temperature(6500.0);
        assert!(color[0] > 0.9);
        assert!(color[1] > 0.9);
        assert!(color[2] > 0.9);
    }
}
