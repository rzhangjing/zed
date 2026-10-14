//! 级联阴影（CSM）适配层：把 Bevy 的平行光与相机位姿送入
//! `cesium-shadow` 领域层计算级联分割，将结果写回 [`ShadowState`]
//! 供渲染读取。
//!
//! [`shadow_update_system`] 每帧从光/相机提取参数，重建
//! [`ShadowMap`] 的级联视图投影矩阵与淡出因子。配置与运行
//! 状态分离：[`ShadowConfig`] 为可调参数，[`ShadowState`] 为逐帧输出。

use bevy::prelude::*;
use cesium_shadow::{
    PcfConfig, ShadowCameraParams,
    ShadowLightType, ShadowMap, ShadowMapConfig, ShadowMapType,
};
use glam::DVec3;

/// 阴影渲染的可调配置资源。
#[derive(Resource, Debug, Clone)]
pub struct ShadowConfig {
    /// 总开关：关闭时系统直接返回。
    pub enabled: bool,
    /// 级联层数。
    pub cascade_count: u32,
    /// 阴影最远距离（米）。
    pub max_distance: f64,
    /// 深度偏移，防止自遮挡。
    pub bias: f64,
    /// 法线偏移，缓解并齿与漏光。
    pub normal_bias: f64,
    /// 是否启用软阴影（PCF）。
    pub soft_shadows: bool,
    /// PCF 采样核尺寸（奇数）。
    pub pcf_kernel_size: u32,
    /// 阴影暗度（0=全暗，1=无阴影）。
    pub darkness: f64,
    /// 阴影贴图分辨率（边长）。
    pub resolution: u32,
    /// 是否在级联边界启用淡出混合。
    pub fading_enabled: bool,
}

impl Default for ShadowConfig {
    /// 默认：启用、4 级联、5000 米、软阴影、3 核 PCF、2048 分辨率。
    fn default() -> Self {
        Self {
            enabled: true,
            cascade_count: 4,
            max_distance: 5000.0,
            bias: 0.0005,
            normal_bias: 0.02,
            soft_shadows: true,
            pcf_kernel_size: 3,
            darkness: 0.3,
            resolution: 2048,
            fading_enabled: true,
        }
    }
}

impl ShadowConfig {
    /// 把适配层配置映射为领域层 [`ShadowMapConfig`]（固定为级联/平行光）。
    ///
    /// # 返回
    /// 填齐字段的领域配置，其中 PCF 开关跟随 `soft_shadows`。
    pub fn to_domain_config(&self) -> ShadowMapConfig {
        ShadowMapConfig {
            enabled: self.enabled,
            shadow_map_type: ShadowMapType::Cascaded,
            light_type: ShadowLightType::Directional,
            resolution: self.resolution,
            cascade_count: self.cascade_count,
            bias: self.bias,
            normal_bias: self.normal_bias,
            soft_shadows: self.soft_shadows,
            pcf: PcfConfig {
                enabled: self.soft_shadows,
                kernel_size: self.pcf_kernel_size,
                use_poisson_disk: false,
            },
            darkness: self.darkness,
            is_fixed: false,
            maximum_distance: self.max_distance,
            normal_offset: true,
            fading_enabled: self.fading_enabled,
            point_light_radius: 100.0,
            maximum_cascade_distances: [25.0, 150.0, 700.0, f64::MAX],
        }
    }
}

/// 逐帧更新的阴影运行状态资源（供渲染侧读取）。
#[derive(Resource, Clone)]
pub struct ShadowState {
    /// 是否需要重新计算（系统完成后置假）。
    pub needs_update: bool,
    /// 每级联：(光视图投影矩阵按列展平, split_near, split_far)。
    pub cascades: Vec<([f32; 16], f32, f32)>,
    /// 级联边界淡出因子。
    pub fade_factor: f64,
    /// 归一化的光方（从表面指向光源）。
    pub light_direction: DVec3,
}

impl Default for ShadowState {
    /// 默认需更新、空级联、无淡出、光方向 (0.5,-1,0.5) 归一化。
    fn default() -> Self {
        Self {
            needs_update: true,
            cascades: Vec::new(),
            fade_factor: 1.0,
            light_direction: DVec3::new(0.5, -1.0, 0.5).normalize(),
        }
    }
}

/// 标记参与阴影投射的实体（当前仅作语义标记）。
#[derive(Component, Debug, Clone)]
pub struct ShadowCaster;

/// 阴影插件：注册配置与状态资源，并添加更新系统。
pub struct CesiumShadowPlugin;

impl Plugin for CesiumShadowPlugin {
    /// 初始化 [`ShadowConfig`]/[`ShadowState`]，在 `Update` 添加 [`shadow_update_system`]。
    ///
    /// # 参数
    /// - `app`：待配置的 Bevy App
    fn build(&self, app: &mut App) {
        app.init_resource::<ShadowConfig>()
            .init_resource::<ShadowState>()
            .add_systems(Update, shadow_update_system);
    }
}

/// 更新系统：从平行光与相机取参，重建级联阴影矩阵与淡出因子。
///
/// # 参数
/// - `config`：阴影配置（总开关与参数）
/// - `state`：逐帧输出状态（会被覆写）
/// - `directional_light_query`：唯一平行光的变换
/// - `camera_query`：唯一 3D 相机的变换
pub fn shadow_update_system(
    config: Res<ShadowConfig>,
    mut state: ResMut<ShadowState>,
    directional_light_query: Query<&Transform, With<DirectionalLight>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
) {
    if !config.enabled {
        // 阴影总开关关闭：不做任何计算。
        return;
    }

    if let Ok(light_transform) = directional_light_query.get_single() {
        // 取平行光前向作为光传播方向，取负得到“指向光源”方向。
        let light_dir = DVec3::new(
            light_transform.forward().x as f64,
            light_transform.forward().y as f64,
            light_transform.forward().z as f64,
        )
        .normalize();
        state.light_direction = -light_dir;
    }

    if let Ok((_camera, cam_transform)) = camera_query.get_single() {
        // 提取相机位置/前向/上方向（f32→f64）作为级联包围参考。
        let cam_pos = DVec3::new(
            cam_transform.translation().x as f64,
            cam_transform.translation().y as f64,
            cam_transform.translation().z as f64,
        );
        let cam_forward = DVec3::new(
            cam_transform.forward().x as f64,
            cam_transform.forward().y as f64,
            cam_transform.forward().z as f64,
        );
        let cam_up = DVec3::new(
            cam_transform.up().x as f64,
            cam_transform.up().y as f64,
            cam_transform.up().z as f64,
        );

        // 由适配配置构造领域阴影图，携当前光方向。
        let domain_config = config.to_domain_config();
        let mut shadow_map = ShadowMap::new(domain_config, state.light_direction);

        // FOV/宽高比为固定假设值（未从相机读取，保持确定性）。
        let fov_y = 60.0_f64.to_radians();
        let aspect = 16.0 / 9.0;

        // 组装领域层所需的阴影相机参数（以相机位姿为包围参考）。
        let shadow_cam = ShadowCameraParams {
            position: cam_pos,
            direction: cam_forward,
            up: cam_up,
            fov_y,
            aspect_ratio: aspect,
        };

        // 阴影贴图深度范围：近端固定 0.1，远端取配置最远距离。
        let near = 0.1;
        let far = config.max_distance;

        // 根据相机参数计算各深度级的级联分割。
        shadow_map.update_cascades(&shadow_cam, near, far);

        // 把领域级联展平为 f32 矩阵写回状态（矩阵按列优先）。
        state.cascades.clear();
        for cascade in &shadow_map.cascades {
            let mat: [[f64; 4]; 4] = cascade.light_view_projection.to_cols_array_2d();
            // 逐列展平为 GPU 友好的一维 [f32;16]（f64→f32）。
            let f32_mat: [f32; 16] = [
                mat[0][0] as f32,
                mat[0][1] as f32,
                mat[0][2] as f32,
                mat[0][3] as f32,
                mat[1][0] as f32,
                mat[1][1] as f32,
                mat[1][2] as f32,
                mat[1][3] as f32,
                mat[2][0] as f32,
                mat[2][1] as f32,
                mat[2][2] as f32,
                mat[2][3] as f32,
                mat[3][0] as f32,
                mat[3][1] as f32,
                mat[3][2] as f32,
                mat[3][3] as f32,
            ];
            state.cascades.push((
                f32_mat,
                cascade.split_near as f32,
                cascade.split_far as f32,
            ));
        }

        // 用光源仰角更新边界淡出，避免硬切换闪烁。
        let light_elevation = state.light_direction.y.max(0.0).asin();
        shadow_map.update_fade(light_elevation);
        state.fade_factor = shadow_map.fade_factor;

        // 本帧已刷新，下帧无变化时不需重建。
        state.needs_update = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 验证阴影配置默认值。
    fn test_shadow_config_default() {
        let config = ShadowConfig::default();
        assert!(config.enabled);
        assert_eq!(config.cascade_count, 4);
        assert_eq!(config.max_distance, 5000.0);
        assert!(config.soft_shadows);
        assert_eq!(config.pcf_kernel_size, 3);
    }

    #[test]
    /// 验证适配配置到领域配置的字段映射。
    fn test_shadow_config_to_domain() {
        let config = ShadowConfig::default();
        let domain = config.to_domain_config();
        assert!(domain.enabled);
        assert_eq!(domain.cascade_count, 4);
        assert_eq!(domain.resolution, 2048);
        assert!(domain.soft_shadows);
        assert_eq!(domain.pcf.kernel_size, 3);
    }

    #[test]
    /// 验证阴影状态默认值。
    fn test_shadow_state_default() {
        let state = ShadowState::default();
        assert!(state.needs_update);
        assert!(state.cascades.is_empty());
        assert_eq!(state.fade_factor, 1.0);
        assert!(state.light_direction.length() > 0.0);
    }

    #[test]
    /// 验证 PCF 滤波对多数未遮挡/全遮挡的返回。
    fn test_pcf_filter() {
        let pcf = PcfConfig {
            enabled: true,
            kernel_size: 3,
            use_poisson_disk: false,
        };
        let comparisons = vec![1.0, -1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
        let factor = pcf.filter(&comparisons);
        assert!(factor > 0.8);

        let all_shadowed = vec![-1.0; 9];
        assert_eq!(pcf.filter(&all_shadowed), 0.0);
    }
}
