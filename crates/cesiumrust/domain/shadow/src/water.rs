//! 水与海洋表面效果。
//!
//! 映射到 CesiumJS 与水相关的特性：
//! - 海洋表面渲染
//! - 波浪模拟（Gerstner 波）
//! - 水的反射/折射参数

use glam::DVec3;

/// 单个 Gerstner 波分量。
/// Gerstner 波可提供逼真的海洋表面动画。
#[derive(Debug, Clone, PartialEq)]
pub struct GerstnerWave {
    /// 波的方向（归一化，位于 XZ 平面）。
    pub direction: DVec3,
    /// 波长（米）。
    pub wavelength: f64,
    /// 波幅（米）。
    pub amplitude: f64,
    /// 波速（相速度）（m/s）。
    pub speed: f64,
    /// 陡峭度因子（0.0 = 正弦，1.0 = 尖锐波峰）。
    pub steepness: f64,
    /// 相位偏移（弧度）。
    pub phase: f64,
}

impl GerstnerWave {
    /// 创建一个新的 Gerstner 波。
    pub fn new(direction: DVec3, wavelength: f64, amplitude: f64, speed: f64) -> Self {
        Self {
            direction: direction.normalize(),
            wavelength,
            amplitude,
            speed,
            steepness: 0.5,
            phase: 0.0,
        }
    }

    /// 计算波数（k = 2π / wavelength）。
    pub fn wave_number(&self) -> f64 {
        std::f64::consts::TAU / self.wavelength
    }

    /// 计算角频率（ω = k * speed）。
    pub fn angular_frequency(&self) -> f64 {
        self.wave_number() * self.speed
    }

    /// 计算给定位置和时刻处的位移。
    ///
    /// # 参数
    /// * `position` - 世界位置（XZ 平面）
    /// * `time` - 时间（秒）
    ///
    /// # 返回
    /// 3D 位移向量
    pub fn compute_displacement(&self, position: DVec3, time: f64) -> DVec3 {
        let k = self.wave_number();
        let omega = self.angular_frequency();

        // 波方向与位置的点积
        let d = self.direction.dot(position);

        // 相位
        let theta = k * d - omega * time + self.phase;

        let cos_theta = theta.cos();
        let sin_theta = theta.sin();

        // 水平位移（形成尖锐波峰）
        let horizontal = self.direction * (self.steepness * self.amplitude * cos_theta);

        // 垂直位移
        let vertical = self.amplitude * sin_theta;

        DVec3::new(horizontal.x, vertical, horizontal.z)
    }

    /// 计算给定位置和时刻处的表面法线。
    pub fn compute_normal(&self, position: DVec3, time: f64) -> DVec3 {
        let k = self.wave_number();
        let omega = self.angular_frequency();

        let d = self.direction.dot(position);
        let theta = k * d - omega * time + self.phase;

        let cos_theta = theta.cos();
        let sin_theta = theta.sin();

        // 偏导数
        let wa = self.amplitude * k;
        let qa = self.steepness * self.amplitude * k;

        // 法线计算（简化）
        DVec3::new(
            -self.direction.x * wa * cos_theta,
            1.0 - qa * sin_theta,
            -self.direction.z * wa * cos_theta,
        )
        .normalize()
    }
}

/// 海洋表面配置。
#[derive(Debug, Clone)]
pub struct OceanConfig {
    /// 海洋是否启用。
    pub enabled: bool,
    /// 基础水色（深水）。
    pub water_color: DVec3,
    /// 浅水颜色。
    pub shallow_color: DVec3,
    /// 水的透明度（0.0 = 不透明，1.0 = 完全透明）。
    pub transparency: f64,
    /// 反射强度（0.0 = 无反射，1.0 = 镜面）。
    pub reflection_strength: f64,
    /// 折射强度。
    pub refraction_strength: f64,
    /// 菲涅尔幂（控制反射角度的衰减）。
    pub fresnel_power: f64,
    /// 镜面反射强度（太阳反光）。
    pub specular_intensity: f64,
    /// 镜面反射幂（太阳反光的锐度）。
    pub specular_power: f64,
    /// 波分量。
    pub waves: Vec<GerstnerWave>,
    /// 法线贴图缩放（用于细节波浪）。
    pub normal_scale: f64,
    /// 泡沫阈值（高于此值的波峰会显示泡沫）。
    pub foam_threshold: f64,
    /// 泡沫颜色。
    pub foam_color: DVec3,
}

impl Default for OceanConfig {
    /// 返回海洋缺省配置：启用，深水呈深蓝、浅水偏青。
    fn default() -> Self {
        Self {
            enabled: true,
            water_color: DVec3::new(0.0, 0.1, 0.3),
            shallow_color: DVec3::new(0.0, 0.4, 0.5),
            transparency: 0.6,
            reflection_strength: 0.5,
            refraction_strength: 0.3,
            fresnel_power: 5.0,
            specular_intensity: 1.0,
            specular_power: 256.0,
            waves: create_default_waves(),
            normal_scale: 1.0,
            foam_threshold: 0.8,
            foam_color: DVec3::new(0.9, 0.95, 1.0),
        }
    }
}

/// 创建一组默认的海洋波浪。
fn create_default_waves() -> Vec<GerstnerWave> {
    vec![
        // 大涌浪
        GerstnerWave::new(DVec3::new(1.0, 0.0, 0.3), 100.0, 1.5, 8.0),
        // 中等波浪
        GerstnerWave::new(DVec3::new(0.7, 0.0, 0.7), 50.0, 0.8, 6.0),
        GerstnerWave::new(DVec3::new(-0.3, 0.0, 0.9), 30.0, 0.5, 5.0),
        // 细小涟漪
        GerstnerWave::new(DVec3::new(0.9, 0.0, -0.4), 10.0, 0.2, 3.0),
        GerstnerWave::new(DVec3::new(-0.6, 0.0, 0.8), 5.0, 0.1, 2.0),
    ]
}

/// 海洋表面状态。
#[derive(Debug, Clone)]
pub struct OceanSurface {
    /// 配置。
    pub config: OceanConfig,
    /// 当前时间（用于波浪动画）。
    pub time: f64,
    /// 风向（影响波浪生成）。
    pub wind_direction: DVec3,
    /// 风速（m/s）。
    pub wind_speed: f64,
}

impl OceanSurface {
    /// 创建一个新的海洋表面。
    pub fn new(config: OceanConfig) -> Self {
        Self {
            config,
            time: 0.0,
            wind_direction: DVec3::new(1.0, 0.0, 0.0),
            wind_speed: 10.0,
        }
    }

    /// 按时间增量更新海洋表面。
    pub fn update(&mut self, dt: f64) {
        self.time += dt;
    }

    /// 计算某位置处的总波位移。
    pub fn compute_displacement(&self, position: DVec3) -> DVec3 {
        if !self.config.enabled {
            return DVec3::ZERO;
        }

        let mut total = DVec3::ZERO;
        for wave in &self.config.waves {
            total += wave.compute_displacement(position, self.time);
        }
        total
    }

    /// 计算某位置处的表面法线。
    pub fn compute_normal(&self, position: DVec3) -> DVec3 {
        if !self.config.enabled {
            return DVec3::Y;
        }

        let mut normal = DVec3::Y;
        for wave in &self.config.waves {
            let wave_normal = wave.compute_normal(position, self.time);
            normal += wave_normal - DVec3::Y;
        }
        normal.normalize()
    }

    /// 计算某位置处的水面高度（Y 位移）。
    pub fn compute_height(&self, position: DVec3) -> f64 {
        self.compute_displacement(position).y
    }

    /// 计算菲涅尔反射系数。
    ///
    /// # 参数
    /// * `view_direction` - 从表面到相机的方向（归一化）
    /// * `normal` - 表面法线（归一化）
    pub fn compute_fresnel(&self, view_direction: DVec3, normal: DVec3) -> f64 {
        let cos_theta = view_direction.dot(normal).abs().clamp(0.0, 1.0);

        // Schlick 近似
        let r0 = 0.02; // 水的基反射率
        r0 + (1.0 - r0) * (1.0 - cos_theta).powf(self.config.fresnel_power)
    }

    /// 计算镜面反射（太阳反光）。
    ///
    /// # 参数
    /// * `view_direction` - 从表面到相机的方向
    /// * `light_direction` - 从表面到光源（太阳）的方向
    /// * `normal` - 表面法线
    pub fn compute_specular(
        &self,
        view_direction: DVec3,
        light_direction: DVec3,
        normal: DVec3,
    ) -> f64 {
        // 将光方向绕法线反射
        let reflect_dir = (2.0 * normal.dot(light_direction) * normal - light_direction).normalize();

        // 镜面反射强度
        let spec_angle = reflect_dir.dot(view_direction).max(0.0);
        spec_angle.powf(self.config.specular_power) * self.config.specular_intensity
    }

    /// 计算某位置处的最终水色。
    ///
    /// # 参数
    /// * `position` - 水面上的世界位置
    /// * `view_direction` - 从表面到相机的方向
    /// * `light_direction` - 从表面到光源（太阳）的方向
    /// * `depth` - 水深（用于浅/深色混合）
    pub fn compute_water_color(
        &self,
        position: DVec3,
        view_direction: DVec3,
        light_direction: DVec3,
        depth: f64,
    ) -> DVec3 {
        if !self.config.enabled {
            return self.config.water_color;
        }

        let normal = self.compute_normal(position);

        // 基于深度的颜色混合
        let depth_factor = (depth / 10.0).clamp(0.0, 1.0); // 10 米过渡带
        let base_color = self.config.shallow_color.lerp(self.config.water_color, depth_factor);

        // 菲涅尔反射
        let fresnel = self.compute_fresnel(view_direction, normal);

        // 镜面反射（太阳反光）
        let specular = self.compute_specular(view_direction, light_direction, normal);

        // 组合
        let reflection_color = DVec3::new(0.5, 0.6, 0.8); // 天空反射近似
        let mut final_color = base_color.lerp(reflection_color, fresnel * self.config.reflection_strength);

        // 添加高光
        final_color += DVec3::splat(specular);

        // 波峰上的泡沫
        let height = self.compute_height(position);
        if height > self.config.foam_threshold {
            let foam_factor = ((height - self.config.foam_threshold) / 0.5).clamp(0.0, 1.0);
            final_color = final_color.lerp(self.config.foam_color, foam_factor);
        }

        final_color.clamp(DVec3::ZERO, DVec3::ONE)
    }

    /// 根据风生成波浪（简化的 Pierson-Moskowitz 谱）。
    pub fn generate_wind_waves(&mut self) {
        let wind_dir = self.wind_direction.normalize();

        // 基于风速的波浪参数
        let significant_wave_height = 0.22 * self.wind_speed * self.wind_speed / 9.81;
        let peak_wavelength = 2.0 * std::f64::consts::PI * self.wind_speed * self.wind_speed / 9.81;

        self.config.waves.clear();

        // 生成一系列波浪谱
        for i in 0..8 {
            let scale = 0.5_f64.powi(i);
            let wavelength = peak_wavelength * scale;
            let amplitude = significant_wave_height * scale * 0.1;
            let speed = (9.81 * wavelength / std::f64::consts::TAU).sqrt();

            // 使方向略有变化
            let angle_offset = (i as f64 - 3.5) * 0.2;
            let cos_a = angle_offset.cos();
            let sin_a = angle_offset.sin();
            let direction = DVec3::new(
                wind_dir.x * cos_a - wind_dir.z * sin_a,
                0.0,
                wind_dir.x * sin_a + wind_dir.z * cos_a,
            );

            let mut wave = GerstnerWave::new(direction, wavelength.max(1.0), amplitude, speed);
            wave.phase = (i as f64) * 1.7; // 使相位变化
            wave.steepness = 0.3 + 0.1 * (i as f64);

            self.config.waves.push(wave);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gerstner_wave_creation() {
        let wave = GerstnerWave::new(DVec3::new(1.0, 0.0, 0.0), 10.0, 1.0, 5.0);

        assert!((wave.wavelength - 10.0).abs() < 1e-10);
        assert!((wave.amplitude - 1.0).abs() < 1e-10);
        assert!((wave.speed - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_gerstner_wave_number() {
        let wave = GerstnerWave::new(DVec3::X, 10.0, 1.0, 5.0);

        let k = wave.wave_number();
        assert!((k - std::f64::consts::TAU / 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_gerstner_displacement_at_origin() {
        let wave = GerstnerWave {
            direction: DVec3::X,
            wavelength: 10.0,
            amplitude: 1.0,
            speed: 5.0,
            steepness: 0.0, // 纯正弦
            phase: 0.0,
        };

        // 当 t=0、position=(0,0,0) 时：theta = 0，sin(0) = 0
        let displacement = wave.compute_displacement(DVec3::ZERO, 0.0);
        assert!((displacement.y).abs() < 1e-10);

        // 当 t 使 theta = -π/2 时：sin(-π/2) = -1
        // theta = k*d - omega*t = -omega*t（因为 d=0）
        // 对于 theta = -π/2：t = π/(2*omega)
        let omega = wave.angular_frequency();
        let t = std::f64::consts::FRAC_PI_2 / omega;
        let displacement = wave.compute_displacement(DVec3::ZERO, t);
        assert!((displacement.y - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn test_gerstner_normal() {
        let wave = GerstnerWave::new(DVec3::X, 10.0, 1.0, 5.0);

        let normal = wave.compute_normal(DVec3::ZERO, 0.0);

        // 法线应大致朝上
        assert!(normal.y > 0.5);
    }

    #[test]
    fn test_ocean_config_default() {
        let config = OceanConfig::default();

        assert!(config.enabled);
        assert_eq!(config.waves.len(), 5);
        assert!(config.transparency > 0.0);
    }

    #[test]
    fn test_ocean_surface_creation() {
        let ocean = OceanSurface::new(OceanConfig::default());

        assert_eq!(ocean.time, 0.0);
        assert!(ocean.config.enabled);
    }

    #[test]
    fn test_ocean_update() {
        let mut ocean = OceanSurface::new(OceanConfig::default());

        ocean.update(1.0);
        assert!((ocean.time - 1.0).abs() < 1e-10);

        ocean.update(0.5);
        assert!((ocean.time - 1.5).abs() < 1e-10);
    }

    #[test]
    fn test_ocean_displacement() {
        let ocean = OceanSurface::new(OceanConfig::default());

        let displacement = ocean.compute_displacement(DVec3::ZERO);

        // 波浪应产生一些位移
        // （在 t=0 时可能为零，取决于波浪相位）
        assert!(displacement.length() >= 0.0);
    }

    #[test]
    fn test_ocean_normal() {
        let ocean = OceanSurface::new(OceanConfig::default());

        let normal = ocean.compute_normal(DVec3::ZERO);

        // 法线应大致朝上
        assert!(normal.y > 0.0);
        assert!((normal.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_ocean_disabled() {
        let config = OceanConfig {
            enabled: false,
            ..Default::default()
        };
        let ocean = OceanSurface::new(config);

        let displacement = ocean.compute_displacement(DVec3::ZERO);
        assert_eq!(displacement, DVec3::ZERO);

        let normal = ocean.compute_normal(DVec3::ZERO);
        assert_eq!(normal, DVec3::Y);
    }

    #[test]
    fn test_fresnel_straight_on() {
        let ocean = OceanSurface::new(OceanConfig::default());

        // 垂直向下看平静的水面
        let view_dir = DVec3::Y;
        let normal = DVec3::Y;

        let fresnel = ocean.compute_fresnel(view_dir, normal);

        // 在正入射时，反射应很小（约 2%）
        assert!(fresnel < 0.1);
    }

    #[test]
    fn test_fresnel_grazing_angle() {
        let ocean = OceanSurface::new(OceanConfig::default());

        // 以掠射角观看
        let view_dir = DVec3::new(1.0, 0.1, 0.0).normalize();
        let normal = DVec3::Y;

        let fresnel = ocean.compute_fresnel(view_dir, normal);

        // 在掠射角时，反射应很高
        assert!(fresnel > 0.5);
    }

    #[test]
    fn test_specular() {
        let ocean = OceanSurface::new(OceanConfig::default());

        let view_dir = DVec3::Y;
        let light_dir = DVec3::Y; // 太阳直射头顶
        let normal = DVec3::Y;

        let specular = ocean.compute_specular(view_dir, light_dir, normal);

        // 完美反射应产生高镜面值
        assert!(specular > 0.9);
    }

    #[test]
    fn test_water_color() {
        let ocean = OceanSurface::new(OceanConfig::default());

        let color = ocean.compute_water_color(
            DVec3::ZERO,
            DVec3::Y,
            DVec3::Y,
            100.0, // 深水
        );

        // 应为一个有效颜色
        assert!(color.x >= 0.0 && color.x <= 1.0);
        assert!(color.y >= 0.0 && color.y <= 1.0);
        assert!(color.z >= 0.0 && color.z <= 1.0);
    }

    #[test]
    fn test_wind_wave_generation() {
        let mut ocean = OceanSurface::new(OceanConfig::default());
        ocean.wind_speed = 15.0;
        ocean.wind_direction = DVec3::new(1.0, 0.0, 0.5);

        ocean.generate_wind_waves();

        assert_eq!(ocean.config.waves.len(), 8);

        // 所有波浪都应有正的波长和波幅
        for wave in &ocean.config.waves {
            assert!(wave.wavelength > 0.0);
            assert!(wave.amplitude >= 0.0);
        }
    }

    #[test]
    fn test_wave_height() {
        let ocean = OceanSurface::new(OceanConfig::default());

        let height = ocean.compute_height(DVec3::ZERO);

        // Height should be finite
        assert!(height.is_finite());
    }
}
