//! 天体光照系统：根据动画时钟计算太阳方向，驱动平行光与环境光参数。
//!
//! [`celestial_system`] 逐帧由 [`AnimationClock`] 取儒略日，算出惯性系下的
//! 太阳方向，再按太阳高度角调节日照/环境光颜色与强度。
use bevy::prelude::*;
use cesium_atmosphere::celestial::compute_sun_direction_eci;

use crate::clock::AnimationClock;

/// 光照参数资源（太阳方向/颜色与环境光，供下游着色使用）。
#[derive(Resource, Debug, Clone)]
pub struct LightingParams {
    /// 太阳入射方向（单位向量）。
    pub sun_direction: Vec3,
    /// 太阳直射光颜色 RGB。
    pub sun_color: [f32; 3],
    /// 环境光颜色 RGB。
    pub ambient_color: [f32; 3],
    /// 环境光强度。
    pub ambient_intensity: f32,
}

impl Default for LightingParams {
    /// 默认：太阳沿 +X、暖白色直射、淡蓝环境光、强度 0.3。
    fn default() -> Self {
        Self {
            sun_direction: Vec3::new(1.0, 0.0, 0.0),
            sun_color: [1.0, 1.0, 0.9],
            ambient_color: [0.1, 0.1, 0.15],
            ambient_intensity: 0.3,
        }
    }
}

/// 逐帧根据时钟更新太阳方向与光照颜色，并转动场景平行光的系统。
///
/// # 参数
/// - `clock`：动画时钟（缺失则直接返回）
/// - `params`：光照参数资源（可写）
/// - `light_query`：所有平行光的变换
pub fn celestial_system(
    clock: Option<Res<AnimationClock>>,
    mut params: ResMut<LightingParams>,
    mut light_query: Query<&mut Transform, With<DirectionalLight>>,
) {
    // 无时钟则无法确定时间，直接跳过。
    let clock = match clock {
        Some(c) => c,
        None => return,
    };
    // 取当前儒略日（总天数）作为天文历元。
    let jd = clock.current_time();
    let julian_date = jd.total_days();

    // 计算惯性系（ECI）下的太阳单位方向。
    let sun_dir_eci = compute_sun_direction_eci(julian_date);

    let sun_dir_f32 = Vec3::new(
        sun_dir_eci.x as f32,
        sun_dir_eci.y as f32,
        sun_dir_eci.z as f32,
    );

    // 归一化失败（零向量）时不更新，保持上一次光照。
    let normalized = sun_dir_f32.normalize_or_zero();
    if normalized != Vec3::ZERO {
        params.sun_direction = normalized;

        // 以 ECI z 分量近似太阳高度角：>0 为白昼，暖白高环境光；<0 为夜晚，暗蓝低强度。
        let sun_elevation = sun_dir_eci.z;
        if sun_elevation > 0.0 {
            let t = (sun_elevation * 2.0).clamp(0.0, 1.0) as f32;
            params.sun_color = [1.0, 0.95 + t * 0.05, 0.8 + t * 0.2];
            params.ambient_color = [0.2 + t * 0.1, 0.2 + t * 0.1, 0.3 + t * 0.1];
            params.ambient_intensity = 0.3 + t * 0.3;
        } else {
            let t = (-sun_elevation * 2.0).clamp(0.0, 1.0) as f32;
            params.sun_color = [1.0, 0.5 + t * 0.3, 0.2 + t * 0.3];
            params.ambient_color = [0.05 + t * 0.05, 0.05 + t * 0.05, 0.1 + t * 0.1];
            params.ambient_intensity = 0.1 + t * 0.1;
        }

        // 让平行光沿太阳反方向照射（look_to 指向太阳）。
        for mut light_transform in light_query.iter_mut() {
            light_transform.look_to(-normalized, Vec3::Y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 默认光照：太阳方向为单位向量、颜色与强度符合预设。
    fn test_lighting_params_default() {
        let params = LightingParams::default();
        assert!((params.sun_direction.length() - 1.0).abs() < 1e-10);
        assert_eq!(params.sun_color, [1.0, 1.0, 0.9]);
        assert!((params.ambient_intensity - 0.3).abs() < 1e-10);
    }

    #[test]
    /// J2000 历元的太阳方向应为单位长度。
    fn test_sun_direction_at_j2000() {
        let dir = compute_sun_direction_eci(2451545.0);
        assert!((dir.length() - 1.0).abs() < 1e-10);
    }

    #[test]
    /// 相隔 12 小时，太阳方向应发生明显变化。
    fn test_sun_position_changes_with_time() {
        let dir_a = compute_sun_direction_eci(2451545.0);
        let dir_b = compute_sun_direction_eci(2451545.0 + 0.5);
        let delta = (dir_a - dir_b).length();
        assert!(delta > 0.001, "Sun direction should change after 12 hours");
    }
}
