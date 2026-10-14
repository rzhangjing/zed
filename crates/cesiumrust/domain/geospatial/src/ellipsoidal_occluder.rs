//! EllipsoidalOccluder - 针对椭球的地平线剔除。

use crate::bounding::BoundingSphere;
use crate::ellipsoid::{normalize_cartesian3, Ellipsoid};
use crate::rectangle::Rectangle;
use glam::DVec3;

/// 判断其他对象是否可见，或者是否隐藏由椭球和相机位置所定义
/// 的可见地平线之后。使用 Horizon Culling 博客文章中描述的算法。
///
/// 映射到 CesiumJS `EllipsoidalOccluder`
#[derive(Debug, Clone)]
pub struct EllipsoidalOccluder {
    /// 作为遮挡体（地平线基准）的参考椭球。
    ellipsoid: Ellipsoid,
    /// 相机在地心固定系下的原始位置。
    camera_position: DVec3,
    /// 相机位置经椭球缩放后的单位空间坐标（使椭球变为单位球）。
    camera_position_in_scaled_space: DVec3,
    /// 缩放空间中相机到地平线（limb）距离的平方，用于视域判断。
    distance_to_limb_in_scaled_space_squared: f64,
}

impl EllipsoidalOccluder {
    /// 创建一个新的椭球遮挡体。
    /// 若提供了 `camera_position`，则立即计算内部缩放空间值。
    ///
    /// 映射到 `new EllipsoidalOccluder(ellipsoid, cameraPosition)`
    pub fn new(ellipsoid: Ellipsoid, camera_position: Option<DVec3>) -> Self {
        let mut occluder = Self {
            ellipsoid,
            camera_position: DVec3::ZERO,
            camera_position_in_scaled_space: DVec3::ZERO,
            distance_to_limb_in_scaled_space_squared: 0.0,
        };
        if let Some(cp) = camera_position {
            occluder.set_camera_position(cp);
        }
        occluder
    }

    /// 获取遮挡椭球。
    #[inline]
    pub fn ellipsoid(&self) -> &Ellipsoid {
        &self.ellipsoid
    }

    /// 获取相机位置。
    #[inline]
    pub fn camera_position(&self) -> DVec3 {
        self.camera_position
    }

    /// 设置相机位置并重新计算内部缩放空间值。
    ///
    /// 映射到 `EllipsoidalOccluder.prototype.cameraPosition` setter
    pub fn set_camera_position(&mut self, camera_position: DVec3) {
        let cv = self.ellipsoid.transform_position_to_scaled_space(camera_position);
        let vh_magnitude_squared = cv.length_squared() - 1.0;

        self.camera_position = camera_position;
        self.camera_position_in_scaled_space = cv;
        self.distance_to_limb_in_scaled_space_squared = vh_magnitude_squared;
    }

    /// 判断某个点（被遮挡对象）是否因遮挡体而不可见。
    ///
    /// 映射到 `EllipsoidalOccluder.prototype.isPointVisible`
    pub fn is_point_visible(&self, occludee: DVec3) -> bool {
        let occludee_scaled_space_position =
            self.ellipsoid.transform_position_to_scaled_space(occludee);
        is_scaled_space_point_visible(
            occludee_scaled_space_position,
            self.camera_position_in_scaled_space,
            self.distance_to_limb_in_scaled_space_squared,
        )
    }

    /// 判断以椭球缩放空间表示的某个点是否因遮挡体而不可见。
    ///
    /// 映射到 `EllipsoidalOccluder.prototype.isScaledSpacePointVisible`
    pub fn is_scaled_space_point_visible(&self, occludee_scaled_space_position: DVec3) -> bool {
        is_scaled_space_point_visible(
            occludee_scaled_space_position,
            self.camera_position_in_scaled_space,
            self.distance_to_limb_in_scaled_space_squared,
        )
    }

    /// 与 `is_scaled_space_point_visible` 类似，但针对的是一个在最小高度低于
    /// 椭球时被该最小高度缩小后的椭球进行测试。
    ///
    /// 映射到 `EllipsoidalOccluder.prototype.isScaledSpacePointVisiblePossiblyUnderEllipsoid`
    pub fn is_scaled_space_point_visible_possibly_under_ellipsoid(
        &self,
        occludee_scaled_space_position: DVec3,
        minimum_height: Option<f64>,
    ) -> bool {
        let ellipsoid = &self.ellipsoid;
        let (cv, vh_magnitude_squared);

        if let Some(mh) = minimum_height {
            // 仅当最小高度为负且未超过最短半轴时，才用 (radius+mh) 重新缩放相机向量；
            // 否则沿用以完整椭球为准的既有缩放空间值。
            if mh < 0.0 && ellipsoid.minimum_radius() > -mh {
                let radii = ellipsoid.radii();
                let cp = self.camera_position;
                cv = DVec3::new(
                    cp.x / (radii.x + mh),
                    cp.y / (radii.y + mh),
                    cp.z / (radii.z + mh),
                );
                vh_magnitude_squared = cv.length_squared() - 1.0;
            } else {
                cv = self.camera_position_in_scaled_space;
                vh_magnitude_squared = self.distance_to_limb_in_scaled_space_squared;
            }
        } else {
            cv = self.camera_position_in_scaled_space;
            vh_magnitude_squared = self.distance_to_limb_in_scaled_space_squared;
        }

        is_scaled_space_point_visible(occludee_scaled_space_position, cv, vh_magnitude_squared)
    }

    /// 从一组位置计算一个可用于地平线剔除的点。
    /// 若无法计算该点（例如位置朝相反方向）则返回 None。
    ///
    /// 映射到 `EllipsoidalOccluder.prototype.computeHorizonCullingPoint`
    pub fn compute_horizon_culling_point(
        &self,
        direction_to_point: DVec3,
        positions: &[DVec3],
    ) -> Option<DVec3> {
        compute_horizon_culling_point_from_positions(
            &self.ellipsoid,
            direction_to_point,
            positions,
        )
    }

    /// 与 `compute_horizon_culling_point` 类似，但当位置低于椭球时相对于一个
    /// 被最小高度缩小后的椭球进行计算。
    ///
    /// 映射到 `EllipsoidalOccluder.prototype.computeHorizonCullingPointPossiblyUnderEllipsoid`
    pub fn compute_horizon_culling_point_possibly_under_ellipsoid(
        &self,
        direction_to_point: DVec3,
        positions: &[DVec3],
        minimum_height: Option<f64>,
    ) -> Option<DVec3> {
        let possibly_shrunk = get_possibly_shrunk_ellipsoid(&self.ellipsoid, minimum_height);
        compute_horizon_culling_point_from_positions(
            &possibly_shrunk,
            direction_to_point,
            positions,
        )
    }

    /// 从带 stride 的顶点数据计算地平线剔除点。
    ///
    /// 映射到 `EllipsoidalOccluder.prototype.computeHorizonCullingPointFromVertices`
    pub fn compute_horizon_culling_point_from_vertices(
        &self,
        direction_to_point: DVec3,
        vertices: &[f64],
        stride: usize,
        center: DVec3,
    ) -> Option<DVec3> {
        compute_horizon_culling_point_from_vertices(
            &self.ellipsoid,
            direction_to_point,
            vertices,
            stride,
            center,
        )
    }

    /// 与 `compute_horizon_culling_point_from_vertices` 类似，但相对于一个
    /// 可能被缩小的椭球进行计算。
    ///
    /// 映射到 `EllipsoidalOccluder.prototype.computeHorizonCullingPointFromVerticesPossiblyUnderEllipsoid`
    pub fn compute_horizon_culling_point_from_vertices_possibly_under_ellipsoid(
        &self,
        direction_to_point: DVec3,
        vertices: &[f64],
        stride: usize,
        center: DVec3,
        minimum_height: Option<f64>,
    ) -> Option<DVec3> {
        let possibly_shrunk = get_possibly_shrunk_ellipsoid(&self.ellipsoid, minimum_height);
        compute_horizon_culling_point_from_vertices(
            &possibly_shrunk,
            direction_to_point,
            vertices,
            stride,
            center,
        )
    }

    /// 由矩形计算地平线剔除点。
    /// 若包围球中心离椭球中心太近则返回 None。
    ///
    /// 映射到 `EllipsoidalOccluder.prototype.computeHorizonCullingPointFromRectangle`
    pub fn compute_horizon_culling_point_from_rectangle(
        &self,
        rectangle: &Rectangle,
        ellipsoid: &Ellipsoid,
    ) -> Option<DVec3> {
        let positions = rectangle.subsample(ellipsoid, 0.0);
        let bs = BoundingSphere::from_points(&positions);

        // 若包围球中心离遮挡体中心太近，
        // 那么试图对它进行地平线剔除就没有意义。
        // 阈值取最短半轴的 0.1 倍：过小中心无法稳定地定义剔除方向。
        if bs.center.length() < 0.1 * ellipsoid.minimum_radius() {
            return None;
        }

        self.compute_horizon_culling_point(bs.center, &positions)
    }
}

// --- 私有辅助函数 ---

/// 缩放空间中的核心可见性测试。
/// 映射到模块级的 `isScaledSpacePointVisible` 函数。
fn is_scaled_space_point_visible(
    occludee_scaled_space_position: DVec3,
    camera_position_in_scaled_space: DVec3,
    distance_to_limb_in_scaled_space_squared: f64,
) -> bool {
    let cv = camera_position_in_scaled_space;
    let vh_magnitude_squared = distance_to_limb_in_scaled_space_squared;
    let vt = occludee_scaled_space_position - cv;
    // vt_dot_vc 为被遮挡点相对相机的向量与相机位置向量的点积取负，正表示朝相机一侧。
    let vt_dot_vc = -vt.dot(cv);

    // 若 vhMagnitudeSquared < 0，则我们位于椭球表面之下，
    // 此时将剔除平面设在 V 上。
    let is_occluded = if vh_magnitude_squared < 0.0 {
        vt_dot_vc > 0.0
    } else {
        vt_dot_vc > vh_magnitude_squared
            && (vt_dot_vc * vt_dot_vc) / vt.length_squared() > vh_magnitude_squared
    };
    !is_occluded
}

/// 由位置数组计算地平线剔除点。
fn compute_horizon_culling_point_from_positions(
    ellipsoid: &Ellipsoid,
    direction_to_point: DVec3,
    positions: &[DVec3],
) -> Option<DVec3> {
    let scaled_space_direction_to_point =
        compute_scaled_space_direction_to_point(ellipsoid, direction_to_point)?;

    let mut result_magnitude = 0.0_f64;

    for &position in positions {
        // 逐位置求其地平线模长，取最大者作为剔除点半径；任一为负（方向相反）则整体不可计算。
        let candidate_magnitude =
            compute_magnitude(ellipsoid, position, scaled_space_direction_to_point);
        if candidate_magnitude < 0.0 {
            return None;
        }
        result_magnitude = result_magnitude.max(candidate_magnitude);
    }

    magnitude_to_point(scaled_space_direction_to_point, result_magnitude)
}

/// 由带 stride 的顶点数据计算地平线剔除点。
fn compute_horizon_culling_point_from_vertices(
    ellipsoid: &Ellipsoid,
    direction_to_point: DVec3,
    vertices: &[f64],
    stride: usize,
    center: DVec3,
) -> Option<DVec3> {
    let scaled_space_direction_to_point =
        compute_scaled_space_direction_to_point(ellipsoid, direction_to_point)?;

    let mut result_magnitude = 0.0_f64;

    let mut i = 0;
    // 按 stride 遍历顶点缓冲，每三个分量叠加 center 还原为世界坐标后求模长。
    while i + 2 < vertices.len() {
        let position = DVec3::new(
            vertices[i] + center.x,
            vertices[i + 1] + center.y,
            vertices[i + 2] + center.z,
        );

        let candidate_magnitude =
            compute_magnitude(ellipsoid, position, scaled_space_direction_to_point);
        if candidate_magnitude < 0.0 {
            return None;
        }
        result_magnitude = result_magnitude.max(candidate_magnitude);

        i += stride;
    }

    magnitude_to_point(scaled_space_direction_to_point, result_magnitude)
}

/// 计算某个点相对于缩放空间方向的模长。
fn compute_magnitude(
    ellipsoid: &Ellipsoid,
    position: DVec3,
    scaled_space_direction_to_point: DVec3,
) -> f64 {
    let scaled_space_position = ellipsoid.transform_position_to_scaled_space(position);
    let mut magnitude_squared = scaled_space_position.length_squared();
    // 先取到地心的实际模长与单位方向，随后再将表面下的点抬升到单位球面。
    let mut magnitude = magnitude_squared.sqrt();
    let direction = scaled_space_position / magnitude;

    // 在本计算的目的下，椭球下方的点被视为位于椭球上。
    magnitude_squared = magnitude_squared.max(1.0);
    magnitude = magnitude.max(1.0);

    let cos_alpha = direction.dot(scaled_space_direction_to_point);
    let sin_alpha = direction.cross(scaled_space_direction_to_point).length();
    // cos_beta/sin_beta 为从单位球心到该点切线的夹角分量；由余弦定理合成地平线交点的模长倒数。
    let cos_beta = 1.0 / magnitude;
    let sin_beta = (magnitude_squared - 1.0).sqrt() * cos_beta;

    1.0 / (cos_alpha * cos_beta - sin_alpha * sin_beta)
}

/// 将沿缩放空间方向的模长转换为一个点。
/// 若模长无效则返回 None。
fn magnitude_to_point(scaled_space_direction_to_point: DVec3, result_magnitude: f64) -> Option<DVec3> {
    // 若没有可供计算的位置、directionToPoint 与所有位置方向相反，
    // 或者我们算出了 NaN 或无穷，则地平线剔除点未定义。
    if result_magnitude <= 0.0 || result_magnitude == f64::INFINITY || result_magnitude.is_nan() {
        return None;
    }

    Some(scaled_space_direction_to_point * result_magnitude)
}

/// 将方向变换到缩放空间并归一化。
/// 若方向为零则返回 None。
///
/// 缩放空间将椭球映为单位球，因此方向向量需按半径逐轴缩放后再归一化，后续角度计算才能用单位球几何。
fn compute_scaled_space_direction_to_point(
    ellipsoid: &Ellipsoid,
    direction_to_point: DVec3,
) -> Option<DVec3> {
    if direction_to_point == DVec3::ZERO {
        return None;
    }

    let scaled = ellipsoid.transform_position_to_scaled_space(direction_to_point);
    Some(normalize_cartesian3(scaled))
}

/// 根据最小高度返回一个可能被缩小的椭球。
///
/// 当最小高度为负且未超过最短半轴时，以各半轴均减去该深度构造缩小椭球；否则原样返回。
fn get_possibly_shrunk_ellipsoid(ellipsoid: &Ellipsoid, minimum_height: Option<f64>) -> Ellipsoid {
    if let Some(mh) = minimum_height {
        if mh < 0.0 && ellipsoid.minimum_radius() > -mh {
            let radii = ellipsoid.radii();
            return Ellipsoid::new(radii.x + mh, radii.y + mh, radii.z + mh);
        }
    }
    *ellipsoid
}
