//! 使用级联阴影贴图（CSM）的阴影贴图。
//!
//! 本模块实现：
//! - 阴影贴图配置
//! - 用于方向光的级联阴影贴图
//! - 阴影偏移与过滤
//! - 按类型偏移（地形/图元/点光源）
//! - 点光源立方体贴图阴影
//! - 接近地平线时的阴影淡出
//! - PCF 软阴影过滤

use glam::{DMat4, DVec3};

/// 阴影贴图类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowMapType {
    /// 单个阴影贴图（用于点光源/聚光灯）。
    Single,
    /// 级联阴影贴图（用于像太阳这样的方向光）。
    Cascaded,
}

/// 用于阴影贴图的光源类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShadowLightType {
    /// 方向光（太阳）—— 使用级联阴影贴图。
    #[default]
    Directional,
    /// 点光源 —— 使用立方体贴图（6 个面）。
    Point,
    /// 聚光灯 —— 使用单个透视阴影贴图。
    Spot,
}

/// 按类型的阴影偏移配置。
///
/// 映射到 CesiumJS ShadowMap 的 `_terrainBias`、`_primitiveBias`、`_pointBias`。
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowBias {
    /// 是否启用多边形偏移。
    pub polygon_offset: bool,
    /// 多边形偏移因子。
    pub polygon_offset_factor: f64,
    /// 多边形偏移单位。
    pub polygon_offset_units: f64,
    /// 是否启用法线偏移。
    pub normal_offset: bool,
    /// 法线偏移缩放。
    pub normal_offset_scale: f64,
    /// 是否启用法线着色。
    pub normal_shading: bool,
    /// 法线着色平滑度。
    pub normal_shading_smooth: f64,
    /// 深度偏移。
    pub depth_bias: f64,
}

impl ShadowBias {
    /// 地形渲染的默认偏移。
    pub fn terrain(normal_offset: bool) -> Self {
        Self {
            polygon_offset: true,
            polygon_offset_factor: 1.1,
            polygon_offset_units: 4.0,
            normal_offset,
            normal_offset_scale: 0.5,
            normal_shading: true,
            normal_shading_smooth: 0.3,
            depth_bias: 0.0001,
        }
    }

    /// 图元（3D 模型）渲染的默认偏移。
    pub fn primitive(normal_offset: bool) -> Self {
        Self {
            polygon_offset: true,
            polygon_offset_factor: 1.1,
            polygon_offset_units: 4.0,
            normal_offset,
            normal_offset_scale: 0.1,
            normal_shading: true,
            normal_shading_smooth: 0.05,
            depth_bias: 0.00002,
        }
    }

    /// 点光源渲染的默认偏移。
    pub fn point(normal_offset: bool) -> Self {
        Self {
            polygon_offset: false,
            polygon_offset_factor: 1.1,
            polygon_offset_units: 4.0,
            normal_offset,
            normal_offset_scale: 0.0,
            normal_shading: true,
            normal_shading_smooth: 0.1,
            depth_bias: 0.0005,
        }
    }

    /// 计算给定表面法线和光方向的有效偏移。
    pub fn compute_effective_bias(&self, normal: DVec3, light_dir: DVec3) -> f64 {
        let mut bias = self.depth_bias;
        if self.normal_offset {
            let n_dot_l = normal.dot(-light_dir).abs();
            // 坡度因子 = sqrt(1 - (N·L)²)：表面越掠射，偏移越大以抑制阴影痤疮。
            let slope_factor = (1.0 - n_dot_l * n_dot_l).sqrt().max(0.0);
            bias += self.normal_offset_scale * slope_factor;
        }
        bias
    }
}

/// 用于软阴影的 PCF（百分比近似过滤）配置。
#[derive(Debug, Clone, PartialEq)]
pub struct PcfConfig {
    /// 是否启用 PCF。
    pub enabled: bool,
    /// 核大小（1、3、5、7）。
    pub kernel_size: u32,
    /// 是否使用泊松盘采样而非网格。
    pub use_poisson_disk: bool,
}

impl Default for PcfConfig {
    /// 返回 PCF 缺省配置：启用、3×3 核、网格采样（非泊松盘）。
    fn default() -> Self {
        Self {
            enabled: true,
            kernel_size: 3,
            use_poisson_disk: false,
        }
    }
}

impl PcfConfig {
    /// 根据深度比较结果计算 PCF 过滤结果。
    ///
    /// 返回 [0.0, 1.0] 范围内的阴影因子（0 = 完全阴影，1 = 完全照亮）。
    pub fn filter(&self, depth_comparisons: &[f64]) -> f64 {
        if !self.enabled || depth_comparisons.is_empty() {
            return if depth_comparisons.first().copied().unwrap_or(1.0) >= 0.0 {
                1.0
            } else {
                0.0
            };
        }
        let lit_count = depth_comparisons.iter().filter(|&&d| d >= 0.0).count();
        lit_count as f64 / depth_comparisons.len() as f64
    }

    /// 为配置的核生成 PCF 采样偏移。
    pub fn sample_offsets(&self) -> Vec<[f64; 2]> {
        if self.use_poisson_disk {
            return poisson_disk_samples(self.kernel_size);
        }
        let half = (self.kernel_size / 2) as f64;
        let mut offsets = Vec::new();
        for y in 0..self.kernel_size {
            for x in 0..self.kernel_size {
                let ox = (x as f64 - half) / half.max(1.0);
                let oy = (y as f64 - half) / half.max(1.0);
                offsets.push([ox, oy]);
            }
        }
        offsets
    }
}

/// 生成泊松盘采样偏移。
fn poisson_disk_samples(count: u32) -> Vec<[f64; 2]> {
    const POISSON_16: [[f64; 2]; 16] = [
        [-0.9420162, -0.3990622], [0.9455861, -0.7689072],
        [-0.0941841, -0.9293887], [0.3449594, 0.2938776],
        [-0.9158858, 0.4577143], [-0.8154423, -0.8791246],
        [-0.3827754, 0.2767685], [0.9748440, 0.7564838],
        [0.4432333, -0.9751155], [0.5374298, -0.4737342],
        [-0.2649691, -0.4189302], [0.7919751, 0.1909019],
        [-0.2418884, 0.9970651], [-0.8140996, 0.9143759],
        [0.1998413, 0.7864137], [0.1438316, -0.1410079],
    ];
    let n = (count as usize).clamp(1, 16);
    POISSON_16[..n].to_vec()
}

/// 阴影贴图配置。
/// 映射到 CesiumJS `ShadowMap` 选项
#[derive(Debug, Clone)]
pub struct ShadowMapConfig {
    /// 是否启用阴影。
    pub enabled: bool,
    /// 阴影贴图类型。
    pub shadow_map_type: ShadowMapType,
    /// 光源类型。
    pub light_type: ShadowLightType,
    /// 阴影贴图分辨率（宽 = 高）。
    pub resolution: u32,
    /// CSM 的级联数。
    pub cascade_count: u32,
    /// 用于减少阴影瑕斯的偏移。
    pub bias: f64,
    /// 法线偏移。
    pub normal_bias: f64,
    /// 是否使用软阴影（PCF）。
    pub soft_shadows: bool,
    /// PCF 配置。
    pub pcf: PcfConfig,
    /// 阴影的暗度（0.0 = 全黑，1.0 = 无阴影）。
    pub darkness: f64,
    /// 阴影贴图是否固定（不随相机更新）。
    pub is_fixed: bool,
    /// 阴影的最大距离。
    pub maximum_distance: f64,
    /// 是否应用法线偏移。
    pub normal_offset: bool,
    /// 阴影是否在接近地平线时淡出。
    pub fading_enabled: bool,
    /// 点光源半径（用于点光源）。
    pub point_light_radius: f64,
    /// 最大级联距离 [4 个值]。
    pub maximum_cascade_distances: [f64; 4],
}

impl Default for ShadowMapConfig {
    /// 返回级联阴影缺省配置：启用、4 级联、2048 分辨率、软阴影开启。
    fn default() -> Self {
        Self {
            enabled: true,
            shadow_map_type: ShadowMapType::Cascaded,
            light_type: ShadowLightType::Directional,
            resolution: 2048,
            cascade_count: 4,
            bias: 0.0005,
            normal_bias: 0.02,
            soft_shadows: true,
            pcf: PcfConfig::default(),
            darkness: 0.3,
            is_fixed: false,
            maximum_distance: 5000.0,
            normal_offset: true,
            fading_enabled: true,
            point_light_radius: 100.0,
            maximum_cascade_distances: [25.0, 150.0, 700.0, f64::MAX],
        }
    }
}

/// 用于阴影贴图计算的相机参数。
#[derive(Debug, Clone, Copy)]
pub struct ShadowCameraParams {
    /// 相机在世界空间中的位置。
    pub position: DVec3,
    /// 相机视图方向。
    pub direction: DVec3,
    /// 相机的 up 向量。
    pub up: DVec3,
    /// 垂直视场角（弧度）。
    pub fov_y: f64,
    /// 宽高比（宽 / 高）。
    pub aspect_ratio: f64,
}

/// 级联阴影贴图中的一个级联。
#[derive(Debug, Clone)]
pub struct ShadowCascade {
    /// 此级联的光视图-投影矩阵。
    pub light_view_projection: DMat4,
    /// 分割距离（此级联在视图空间中的近平面）。
    pub split_near: f64,
    /// 此级联在视图空间中的远平面。
    pub split_far: f64,
    /// 纹素大小（每纹素的世界单位）。
    pub texel_size: f64,
}

/// 阴影贴图状态。
#[derive(Debug, Clone)]
pub struct ShadowMap {
    /// 配置。
    pub config: ShadowMapConfig,
    /// 光方向（归一化，由光指向场景）。
    pub light_direction: DVec3,
    /// 光位置（用于点光源/聚光灯）。
    pub light_position: DVec3,
    /// 级联（用于 CSM）。
    pub cascades: Vec<ShadowCascade>,
    /// 阴影贴图是否需要更新。
    pub needs_update: bool,
    /// 按类型的偏移配置。
    pub terrain_bias: ShadowBias,
    /// 图元偏移。
    pub primitive_bias: ShadowBias,
    /// 点光源偏移。
    pub point_bias: ShadowBias,
    /// 当前淡出因子（1.0 = 无淡出，0.0 = 完全淡出）。
    pub fade_factor: f64,
    /// 光是否位于视野之外。
    pub out_of_view: bool,
}

/// 全局最大阴影距离。
pub const SHADOW_MAP_MAXIMUM_DISTANCE: f64 = 20000.0;

impl ShadowMap {
    /// 创建一个新的阴影贴图。
    pub fn new(config: ShadowMapConfig, light_direction: DVec3) -> Self {
        let normal_offset = config.normal_offset;
        Self {
            config,
            light_direction: light_direction.normalize(),
            light_position: DVec3::ZERO,
            cascades: Vec::new(),
            needs_update: true,
            terrain_bias: ShadowBias::terrain(normal_offset),
            primitive_bias: ShadowBias::primitive(normal_offset),
            point_bias: ShadowBias::point(normal_offset),
            fade_factor: 1.0,
            out_of_view: false,
        }
    }

    /// 为太阳创建阴影贴图。
    pub fn for_sun(sun_direction: DVec3) -> Self {
        Self::new(ShadowMapConfig::default(), -sun_direction)
    }

    /// 为点光源创建阴影贴图。
    pub fn for_point_light(position: DVec3, radius: f64) -> Self {
        let config = ShadowMapConfig {
            shadow_map_type: ShadowMapType::Single,
            light_type: ShadowLightType::Point,
            cascade_count: 0,
            point_light_radius: radius,
            ..Default::default()
        };
        let mut map = Self::new(config, DVec3::ZERO);
        map.light_position = position;
        map
    }

    /// 为聚光灯创建阴影贴图。
    pub fn for_spot_light(position: DVec3, direction: DVec3) -> Self {
        let config = ShadowMapConfig {
            shadow_map_type: ShadowMapType::Single,
            light_type: ShadowLightType::Spot,
            cascade_count: 0,
            ..Default::default()
        };
        let mut map = Self::new(config, direction.normalize());
        map.light_position = position;
        map
    }

    /// 根据光高度角计算阴影淡出因子。
    ///
    /// 当光接近地平线时阴影会淡出。
    ///
    /// # 参数
    /// * `light_elevation` - 光高度角（弧度）（0 = 地平线，π/2 = 头顶）
    pub fn compute_fade_factor(&self, light_elevation: f64) -> f64 {
        if !self.config.fading_enabled {
            return 1.0;
        }

        // 淡出从高于地平线约 10 度开始，到地平线时完全淡出
        let fade_start = 10.0_f64.to_radians();
        let fade_end = 0.0_f64.to_radians();

        if light_elevation >= fade_start {
            1.0
        } else if light_elevation <= fade_end {
            0.0
        } else {
            (light_elevation - fade_end) / (fade_start - fade_end)
        }
    }

    /// 根据光高度角更新淡出因子。
    pub fn update_fade(&mut self, light_elevation: f64) {
        self.fade_factor = self.compute_fade_factor(light_elevation);
    }

    /// 返回所需的阴影渲染通道数。
    pub fn pass_count(&self) -> usize {
        match self.config.light_type {
            ShadowLightType::Point => 6, // 立方体贴图：6 个面
            ShadowLightType::Spot => 1,
            ShadowLightType::Directional => {
                if self.config.cascade_count > 0 {
                    self.config.cascade_count as usize
                } else {
                    1
                }
            }
        }
    }

    /// 返回给定接收器类型的偏移配置。
    pub fn bias_for_type(&self, receiver_type: ShadowBiasType) -> &ShadowBias {
        match receiver_type {
            ShadowBiasType::Terrain => &self.terrain_bias,
            ShadowBiasType::Primitive => &self.primitive_bias,
            ShadowBiasType::Point => &self.point_bias,
        }
    }

    /// 使用实用分割方案计算级联分割。
    ///
    /// # 参数
    /// * `near` - 相机近平面
    /// * `far` - 相机远平面（或最大阴影距离）
    /// * `lambda` - 对数分割（0.0）与均匀分割（1.0）之间的混合因子
    pub fn compute_cascade_splits(&self, near: f64, far: f64, lambda: f64) -> Vec<f64> {
        let count = self.config.cascade_count as usize;
        let mut splits = Vec::with_capacity(count + 1);

        splits.push(near);

        for i in 1..count {
            let t = i as f64 / count as f64;

            // 对数分割
            let log_split = near * (far / near).powf(t);

            // 均匀分割
            let uniform_split = near + (far - near) * t;

            // 在对数与均匀之间混合
            let split = lambda * log_split + (1.0 - lambda) * uniform_split;
            splits.push(split);
        }

        splits.push(far);
        splits
    }

    /// 计算某个级联的光视图-投影矩阵。
    ///
    /// # 参数
    /// * `camera` - 相机参数
    /// * `cascade_near` - 此级联的近平面距离
    /// * `cascade_far` - 此级联的远平面距离
    pub fn compute_cascade_matrix(
        &self,
        camera: &ShadowCameraParams,
        cascade_near: f64,
        cascade_far: f64,
    ) -> DMat4 {
        // 在世界空间中计算视锥体角点
        let corners = compute_frustum_corners(
            camera.position,
            camera.direction,
            camera.up,
            cascade_near,
            cascade_far,
            camera.fov_y,
            camera.aspect_ratio,
        );

        // 计算视锥体的形心
        let centroid = corners.iter().sum::<DVec3>() / 8.0;

        // 光视图矩阵（沿光方向观察）
        let light_right = self.light_direction.cross(DVec3::Y).normalize();
        let light_up = light_right.cross(self.light_direction).normalize();

        let light_view = look_at_matrix(centroid - self.light_direction * 1000.0, centroid, light_up);

        // 将角点变换到光空间
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut min_z = f64::INFINITY;
        let mut max_z = f64::NEG_INFINITY;

        for corner in &corners {
            let transformed = light_view.transform_point3(*corner);
            min_x = min_x.min(transformed.x);
            max_x = max_x.max(transformed.x);
            min_y = min_y.min(transformed.y);
            max_y = max_y.max(transformed.y);
            min_z = min_z.min(transformed.z);
            max_z = max_z.max(transformed.z);
        }

        // 添加一些边距
        let padding = 10.0;
        min_x -= padding;
        max_x += padding;
        min_y -= padding;
        max_y += padding;
        min_z -= padding;
        max_z += padding;

        // 正交投影
        let light_projection = orthographic_matrix(min_x, max_x, min_y, max_y, min_z, max_z);

        light_projection * light_view
    }

    /// 根据相机更新级联。
    pub fn update_cascades(
        &mut self,
        camera: &ShadowCameraParams,
        near: f64,
        far: f64,
    ) {
        if !self.config.enabled {
            return;
        }

        let effective_far = far.min(self.config.maximum_distance);
        let splits = self.compute_cascade_splits(near, effective_far, 0.5);

        self.cascades.clear();

        for i in 0..self.config.cascade_count as usize {
            let cascade_near = splits[i];
            let cascade_far = splits[i + 1];

            let light_vp = self.compute_cascade_matrix(
                camera,
                cascade_near,
                cascade_far,
            );

            let texel_size = (cascade_far - cascade_near) / self.config.resolution as f64;

            self.cascades.push(ShadowCascade {
                light_view_projection: light_vp,
                split_near: cascade_near,
                split_far: cascade_far,
                texel_size,
            });
        }

        self.needs_update = false;
    }

    /// 计算某个世界位置的阴影因子。
    ///
    /// 返回 [darkness, 1.0] 范围内的值，其中 darkness = 完全阴影。
    pub fn compute_shadow_factor(&self, _world_position: DVec3, view_depth: f64) -> f64 {
        if !self.config.enabled || self.cascades.is_empty() {
            return 1.0;
        }

        // 找到合适的级联
        let cascade = self.cascades.iter().find(|c| {
            view_depth >= c.split_near && view_depth < c.split_far
        });

        match cascade {
            Some(_) => {
                // 在真实实现中，我们会在此采样阴影贴图。
                // 目前，返回一个基于 darkness 的占位值。
                // 实际的阴影查找会将深度与阴影贴图进行比较。
                1.0 // 占位：无阴影（需要实际的深度比较）
            }
            None => 1.0, // 在阴影范围之外
        }
    }

    /// 将阴影应用于颜色。
    pub fn apply_shadow(&self, color: DVec3, shadow_factor: f64) -> DVec3 {
        let effective_factor = shadow_factor * self.fade_factor;
        let factor = self.config.darkness + (1.0 - self.config.darkness) * effective_factor;
        color * factor
    }
}

/// 用于偏移选择的接收器类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowBiasType {
    /// 地形接收器。
    Terrain,
    /// 图元（3D 模型）接收器。
    Primitive,
    /// 点光源接收器。
    Point,
}

/// 计算视图视锥体切片的 8 个角点。
fn compute_frustum_corners(
    camera_position: DVec3,
    camera_direction: DVec3,
    camera_up: DVec3,
    near: f64,
    far: f64,
    fov_y: f64,
    aspect_ratio: f64,
) -> [DVec3; 8] {
    let camera_right = camera_direction.cross(camera_up).normalize();
    let camera_up = camera_right.cross(camera_direction).normalize();

    let near_height = 2.0 * (fov_y / 2.0).tan() * near;
    let near_width = near_height * aspect_ratio;

    let far_height = 2.0 * (fov_y / 2.0).tan() * far;
    let far_width = far_height * aspect_ratio;

    let near_center = camera_position + camera_direction * near;
    let far_center = camera_position + camera_direction * far;

    let near_up = camera_up * (near_height / 2.0);
    let near_right = camera_right * (near_width / 2.0);

    let far_up = camera_up * (far_height / 2.0);
    let far_right = camera_right * (far_width / 2.0);

    [
        // 近平面角点
        near_center - near_right + near_up,
        near_center + near_right + near_up,
        near_center + near_right - near_up,
        near_center - near_right - near_up,
        // 远平面角点
        far_center - far_right + far_up,
        far_center + far_right + far_up,
        far_center + far_right - far_up,
        far_center - far_right - far_up,
    ]
}

/// 创建 look-at 视图矩阵。
fn look_at_matrix(eye: DVec3, target: DVec3, up: DVec3) -> DMat4 {
    let z = (eye - target).normalize();
    let x = up.cross(z).normalize();
    let y = z.cross(x);

    DMat4::from_cols_array(&[
        x.x, y.x, z.x, 0.0,
        x.y, y.y, z.y, 0.0,
        x.z, y.z, z.z, 0.0,
        -x.dot(eye), -y.dot(eye), -z.dot(eye), 1.0,
    ])
}

/// 创建正交投影矩阵。
fn orthographic_matrix(left: f64, right: f64, bottom: f64, top: f64, near: f64, far: f64) -> DMat4 {
    let rcp_width = 1.0 / (right - left);
    let rcp_height = 1.0 / (top - bottom);
    let rcp_depth = 1.0 / (far - near);

    DMat4::from_cols_array(&[
        2.0 * rcp_width, 0.0, 0.0, 0.0,
        0.0, 2.0 * rcp_height, 0.0, 0.0,
        0.0, 0.0, -2.0 * rcp_depth, 0.0,
        -(right + left) * rcp_width, -(top + bottom) * rcp_height, -(far + near) * rcp_depth, 1.0,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_4;

    #[test]
    fn test_shadow_map_creation() {
        let shadow_map = ShadowMap::new(ShadowMapConfig::default(), DVec3::new(0.0, -1.0, 0.0));

        assert!(shadow_map.config.enabled);
        assert_eq!(shadow_map.config.cascade_count, 4);
        assert!(shadow_map.needs_update);
    }

    #[test]
    fn test_shadow_map_for_sun() {
        let sun_direction = DVec3::new(0.5, -0.7, 0.3).normalize();
        let shadow_map = ShadowMap::for_sun(sun_direction);

        // 光方向应与太阳方向相反
        assert!((shadow_map.light_direction - (-sun_direction)).length() < 1e-10);
    }

    #[test]
    fn test_cascade_splits_count() {
        let shadow_map = ShadowMap::new(ShadowMapConfig::default(), DVec3::new(0.0, -1.0, 0.0));

        let splits = shadow_map.compute_cascade_splits(0.1, 1000.0, 0.5);

        // 应有 cascade_count + 1 个分割
        assert_eq!(splits.len(), 5); // 4 个级联 = 5 个分割
        assert!((splits[0] - 0.1).abs() < 1e-10);
        assert!((splits[4] - 1000.0).abs() < 1e-10);
    }

    #[test]
    fn test_cascade_splits_monotonic() {
        let shadow_map = ShadowMap::new(ShadowMapConfig::default(), DVec3::new(0.0, -1.0, 0.0));

        let splits = shadow_map.compute_cascade_splits(0.1, 1000.0, 0.5);

        for i in 1..splits.len() {
            assert!(splits[i] > splits[i - 1]);
        }
    }

    #[test]
    fn test_cascade_splits_lambda() {
        let shadow_map = ShadowMap::new(ShadowMapConfig::default(), DVec3::new(0.0, -1.0, 0.0));

        // Lambda = 0.0：均匀分割
        let uniform = shadow_map.compute_cascade_splits(0.1, 1000.0, 0.0);
        let expected_uniform = 0.1 + (1000.0 - 0.1) * 0.25;
        assert!((uniform[1] - expected_uniform).abs() < 1.0);

        // Lambda = 1.0：对数分割
        let logarithmic = shadow_map.compute_cascade_splits(0.1, 1000.0, 1.0);
        let expected_log = 0.1_f64 * (1000.0_f64 / 0.1_f64).powf(0.25);
        assert!((logarithmic[1] - expected_log).abs() < 0.1);
    }

    #[test]
    fn test_update_cascades() {
        let mut shadow_map = ShadowMap::new(ShadowMapConfig::default(), DVec3::new(0.0, -1.0, 0.0));

        let camera = ShadowCameraParams {
            position: DVec3::ZERO,
            direction: DVec3::new(0.0, 0.0, -1.0),
            up: DVec3::Y,
            fov_y: FRAC_PI_4,
            aspect_ratio: 16.0 / 9.0,
        };

        shadow_map.update_cascades(&camera, 0.1, 1000.0);

        assert_eq!(shadow_map.cascades.len(), 4);
        assert!(!shadow_map.needs_update);
    }

    #[test]
    fn test_cascade_texel_size() {
        let mut shadow_map = ShadowMap::new(ShadowMapConfig::default(), DVec3::new(0.0, -1.0, 0.0));

        let camera = ShadowCameraParams {
            position: DVec3::ZERO,
            direction: DVec3::new(0.0, 0.0, -1.0),
            up: DVec3::Y,
            fov_y: FRAC_PI_4,
            aspect_ratio: 16.0 / 9.0,
        };

        shadow_map.update_cascades(&camera, 0.1, 1000.0);

        for cascade in &shadow_map.cascades {
            assert!(cascade.texel_size > 0.0);
        }
    }

    #[test]
    fn test_shadow_factor_no_cascades() {
        let shadow_map = ShadowMap::new(ShadowMapConfig::default(), DVec3::new(0.0, -1.0, 0.0));

        // 尚未计算任何级联
        let factor = shadow_map.compute_shadow_factor(DVec3::ZERO, 100.0);
        assert_eq!(factor, 1.0);
    }

    #[test]
    fn test_shadow_disabled() {
        let config = ShadowMapConfig {
            enabled: false,
            ..Default::default()
        };
        let shadow_map = ShadowMap::new(config, DVec3::new(0.0, -1.0, 0.0));

        let factor = shadow_map.compute_shadow_factor(DVec3::ZERO, 100.0);
        assert_eq!(factor, 1.0);
    }

    #[test]
    fn test_apply_shadow() {
        let shadow_map = ShadowMap::new(ShadowMapConfig::default(), DVec3::new(0.0, -1.0, 0.0));

        let color = DVec3::new(1.0, 1.0, 1.0);

        // 完全阴影（factor = 0.0）
        let shadowed = shadow_map.apply_shadow(color, 0.0);
        assert!((shadowed.x - shadow_map.config.darkness).abs() < 1e-10);

        // 无阴影（factor = 1.0）
        let lit = shadow_map.apply_shadow(color, 1.0);
        assert!((lit.x - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_frustum_corners() {
        let corners = compute_frustum_corners(
            DVec3::ZERO,
            DVec3::new(0.0, 0.0, -1.0),
            DVec3::Y,
            1.0,
            10.0,
            FRAC_PI_4,
            1.0,
        );

        // 所有角点都应位于相机前方（负 Z）
        for corner in &corners {
            assert!(corner.z < 0.0);
        }
    }

    #[test]
    fn test_look_at_matrix() {
        let matrix = look_at_matrix(
            DVec3::new(0.0, 0.0, 5.0),
            DVec3::ZERO,
            DVec3::Y,
        );

        // 原点应变换到 (0, 0, -5)
        let transformed = matrix.transform_point3(DVec3::ZERO);
        assert!((transformed.x).abs() < 1e-10);
        assert!((transformed.y).abs() < 1e-10);
        assert!((transformed.z - (-5.0)).abs() < 1e-10);
    }

    #[test]
    fn test_orthographic_matrix() {
        let matrix = orthographic_matrix(-1.0, 1.0, -1.0, 1.0, 0.1, 100.0);

        // 中心应映射到 (0, 0, z)
        let center = matrix.transform_point3(DVec3::ZERO);
        assert!((center.x).abs() < 1e-10);
        assert!((center.y).abs() < 1e-10);
    }
}
