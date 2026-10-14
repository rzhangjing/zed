use bevy::prelude::*;
use cesium_atmosphere::scattering::{
    AtmosphereParameters, compute_sky_color, compute_horizon_glow,
};
use glam::DVec3;

use crate::atmosphere::celestial_system::LightingParams;
use crate::atmosphere::sky_dome::{
    despawn_sky_dome, spawn_sky_dome, update_sun_direction, SkyAtmosphereParams, SkyDome,
    SkyDomeMaterial,
};
use crate::clock::AnimationClock;

#[derive(Resource, Debug, Clone)]
pub struct SkyAtmosphere {
    /// 是否启用天空/大气渲染。
    pub enabled: bool,
    /// 为 `true` 时，天空由 GPU 单次散射 dome
    /// （`sky_atmosphere.wgsl`）渲染，而非 CPU 的 `ClearColor` 近似。
    ///
    /// 由 [`super::CesiumAtmospherePlugin`] 从 `CESIUM_ENABLE_SKYDOME` 播种；
    /// 默认为 `true`，因为该插件本身仅在 `feature_flags::skydome_enabled()`
    /// 为真时才注册（main.rs L513-516），所以到这个资源存在时门控
    /// 已经关闭。在运行时将其设为 `false` 会回退到 M5-C 之前的
    /// `ClearColor` 路径，并拆掉任何活动的 dome——这正是单元测试使用的逃生舱。
    pub dome: bool,
    /// 领域层大气参数（散射系数/尺度高等）。
    pub atmosphere_params: AtmosphereParameters,
}

impl Default for SkyAtmosphere {
    /// 默认：启用天空与 dome，使用领域层默认大气参数。
    fn default() -> Self {
        Self {
            enabled: true,
            dome: true,
            atmosphere_params: AtmosphereParameters::default(),
        }
    }
}

/// 由 [`SkyAtmosphere::dome`] 驱动的幂等 sky-dome spawn / 拆除。
///
/// 运行在 `Update`（链在 [`sky_system`] 之前）而非 `Startup`，以便
/// 在运行时切换 `dome` 能被响应，也以便一个无头的 `MinimalPlugins` 应用——
/// 其中 `MaterialPlugin::<SkyDomeMaterial>` 被 `shader_registry::asset_backend_available`
/// 跳过，因而 `Assets<SkyDomeMaterial>` 不存在——降级为空操作，
/// 而不是因缺失资源而 panic。出于这一原因，两个 asset 存储都被取为
/// `Option<ResMut<_>>`。
pub fn sky_dome_setup(
    mut commands: Commands,
    sky: Res<SkyAtmosphere>,
    existing: Query<Entity, With<SkyDome>>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<SkyDomeMaterial>>>,
) {
    if !sky.enabled || !sky.dome {
        // 门控 OFF 分支：下面的 CPU ClearColor 路径是唯一的天空
        // 贡献者，因此不得有 dome 残留着去重复绘制它。
        if !existing.is_empty() {
            despawn_sky_dome(&mut commands, existing.iter());
        }
        return;
    }
    if !existing.is_empty() {
        return;
    }
    let (Some(meshes), Some(materials)) = (meshes.as_deref_mut(), materials.as_deref_mut()) else {
        return;
    };
    spawn_sky_dome(
        &mut commands,
        meshes,
        materials,
        SkyAtmosphereParams::from_domain(&sky.atmosphere_params),
    );
}

/// 逐帧计算天空颜色：dome 开启时刷新太阳方向 uniform，否则回退到 CPU ClearColor 近似。
///
/// # 参数
/// - `clock`：动画时钟（缺失则返回）
/// - `lighting`：光照参数（提供太阳方向）
/// - `sky`：天空/大气资源
/// - `clear_color`：背景清屏色（CPU 路径下写入）
/// - `camera_query`：主相机变换（提供视线方向）
/// - `dome_query`：dome 材质引用
/// - `materials`：dome 材质资源（可写，可能不存在）
pub fn sky_system(
    clock: Option<Res<AnimationClock>>,
    lighting: Res<LightingParams>,
    sky: Res<SkyAtmosphere>,
    mut clear_color: ResMut<ClearColor>,
    camera_query: Query<&Transform, With<Camera3d>>,
    dome_query: Query<&MeshMaterial3d<SkyDomeMaterial>, With<SkyDome>>,
    mut materials: Option<ResMut<Assets<SkyDomeMaterial>>>,
) {
    // 无时钟则无法确定时间；未启用天空则不处理。
    let clock = match clock {
        Some(c) => c,
        None => return,
    };
    if !sky.enabled {
        return;
    }

    // 取儒略日（当前未用于 CPU 路径，保留以备扩展）。
    let jd = clock.current_time();
    let julian_date = jd.total_days();

    // 从光照参数拿到太阳方向（f32→f64）。
    let sun_dir = DVec3::new(
        lighting.sun_direction.x as f64,
        lighting.sun_direction.y as f64,
        lighting.sun_direction.z as f64,
    );

    // 太阳高度角（z 分量）用于地平线霞光。
    let sun_elevation = sun_dir.z;

    // 以主相机前方作为视线方向；无相机时退化为看太阳。
    let view_dir = if let Ok(cam_transform) = camera_query.get_single() {
        DVec3::new(
            cam_transform.forward().x as f64,
            cam_transform.forward().y as f64,
            cam_transform.forward().z as f64,
        )
    } else {
        sun_dir
    };

    // ── GPU 单次散射路径 (M5-C) ─────────────────────────────────
    // `sky_atmosphere.wgsl` 拥有天空颜色，所以下面的 CPU ClearColor
    // 绝不应也被写入（那会重复绘制 dome 合成结果）。
    // 只刷新太阳方向 uniform，且仅当它确实
    // 移动时：在 `FIXED_TIME` 下时钟被冻结，所以这会在
    // 首帧之后稳定下来，基线捕获保持逐位可复现。
    if sky.dome {
        if let Ok(dome_material) = dome_query.get_single() {
            if let Some(materials) = materials.as_deref_mut() {
                update_sun_direction(materials, &dome_material.0, lighting.sun_direction);
            }
        }
        let _ = julian_date;
        return;
    }

    // ── 门控 OFF 路径：逐字节复现 M5-C 之前的 CPU ClearColor 天空 ───────
    let sky_color = compute_sky_color(view_dir, sun_dir, 1000.0, &sky.atmosphere_params);
    let horizon_glow = compute_horizon_glow(sun_elevation);

    let r = (sky_color[0] as f32 * 0.3 + horizon_glow[0] as f32 * 0.3).clamp(0.0, 1.0);
    let g = (sky_color[1] as f32 * 0.3 + horizon_glow[1] as f32 * 0.3).clamp(0.0, 1.0);
    let b = (sky_color[2] as f32 * 0.3 + horizon_glow[2] as f32 * 0.3).clamp(0.0, 1.0);

    clear_color.0 = Color::srgb(r, g, b);

    let _ = julian_date;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 默认天空资源应处于启用状态。
    fn test_sky_atmosphere_default() {
        let sky = SkyAtmosphere::default();
        assert!(sky.enabled);
    }

    #[test]
    /// 显式传入 enabled=false 时应为禁用。
    fn test_sky_atmosphere_disabled() {
        let sky = SkyAtmosphere {
            enabled: false,
            ..Default::default()
        };
        assert!(!sky.enabled);
    }

    #[test]
    /// 白天仰视应得到非全黑的天空色。
    fn test_compute_sky_color_blue() {
        let params = AtmosphereParameters::default();
        let view = DVec3::new(0.0, 0.0, 1.0);
        let sun = DVec3::new(0.0, 1.0, 0.0);
        let color = compute_sky_color(view, sun, 0.0, &params);
        assert!(color.iter().any(|&c| c > 0.0), "Sky color should not be black");
    }

    #[test]
    /// 日落时霞光应以红色为主。
    fn test_horizon_glow_sunset() {
        let color = compute_horizon_glow(-0.1);
        assert!(color[0] > color[2], "Red should dominate at sunset");
    }

    #[test]
    /// 正午时霞光应以蓝色为主。
    fn test_horizon_glow_noon() {
        use std::f64::consts::PI;
        let color = compute_horizon_glow(PI / 2.0);
        assert!(color[2] > color[0], "Blue should dominate at noon");
    }
}
