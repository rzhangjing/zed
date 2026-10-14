//! Occluder - 判断对象是否可见或隐藏在地平线之后。
//!
//! 遮挡体由一个球（位置与半径）和相机位置共同确定一个“可见地平线”平面：
//! 当相机在球外时，从相机向球引两条切线，切点所在平面将空间分为可见区与遮挡区。
//! [`Occluder::set_camera_position`] 预算地平线距离与平面参数，随后各可见性
//! 判定（[`Occluder::is_bounding_sphere_visible`]、[`Occluder::compute_visibility`]）
//! 基于被遮挡球心到相机的距离与地平线距离比较得出结论。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::needless_range_loop, clippy::excessive_precision)]
use crate::bounding::BoundingSphere;
use glam::DVec3;

/// 遮挡查询的可见性结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// 对象不可见（完全遮挡）。
    None = -1,
    /// 对象部分可见。
    Partial = 0,
    /// 对象完全可见。
    Full = 1,
}

/// 由对象的位置和半径以及相机位置导出的遮挡体。
/// 用于判断其他对象是否可见，或者是否隐藏由该遮挡体和相机位置
/// 所定义的可见地平线之后。
/// 映射到 CesiumJS `Core/Occluder`
#[derive(Debug, Clone)]
pub struct Occluder {
    /// 遮挡体球心位置（地心固定系，米）。
    occluder_position: DVec3,
    /// 遮挡体球半径（米）。
    occluder_radius: f64,
    /// 相机到地平线的距离；相机在球内时为 `f64::MAX`（无地平线）。
    horizon_distance: f64,
    /// 地平线平面单位法线；相机在球内时为 `None`。
    horizon_plane_normal: Option<DVec3>,
    /// 地平线平面上的一点；相机在球内时为 `None`。
    horizon_plane_position: Option<DVec3>,
    /// 当前相机位置（地心固定系，米）。
    camera_position: DVec3,
}

impl Occluder {
    /// 由包围球和相机位置创建一个 Occluder。
    /// 映射到 `new Occluder(occluderBoundingSphere, cameraPosition)`
    pub fn new(occluder_bounding_sphere: &BoundingSphere, camera_position: DVec3) -> Self {
        let occluder_position = occluder_bounding_sphere.center;
        let occluder_radius = occluder_bounding_sphere.radius;

        let mut result = Self {
            occluder_position,
            occluder_radius,
            horizon_distance: 0.0,
            horizon_plane_normal: None,
            horizon_plane_position: None,
            camera_position: DVec3::ZERO,
        };
        result.set_camera_position(camera_position);
        result
    }

    /// 由包围球和相机位置创建一个遮挡体。
    /// 映射到 `Occluder.fromBoundingSphere`
    pub fn from_bounding_sphere(
        occluder_bounding_sphere: &BoundingSphere,
        camera_position: DVec3,
    ) -> Self {
        Self::new(occluder_bounding_sphere, camera_position)
    }

    /// 设置相机位置并重新预算地平线参数。
    ///
    /// # 参数
    /// - `camera_position`：新的相机位置。若相机在遮挡球外，则据切线几何
    ///   算出地平线距离与平面法线/位置；若相机在球内，地平线置为 `f64::MAX` 且法线为空。
    fn set_camera_position(&mut self, camera_position: DVec3) {
        self.camera_position = camera_position;

        let camera_to_occluder_vec = self.occluder_position - camera_position;
        let inv_camera_to_occluder_distance = camera_to_occluder_vec.length_squared();
        let occluder_radius_sqrd = self.occluder_radius * self.occluder_radius;

        if inv_camera_to_occluder_distance > occluder_radius_sqrd {
            // 相机在球外：由直角三角形得切线长（地平线距离）与地平线平面。
            let horizon_distance =
                (inv_camera_to_occluder_distance - occluder_radius_sqrd).sqrt();
            let inv_dist = 1.0 / inv_camera_to_occluder_distance.sqrt();
            let horizon_plane_normal = camera_to_occluder_vec * inv_dist;
            let near_plane_distance = horizon_distance * horizon_distance * inv_dist;
            let horizon_plane_position =
                camera_position + horizon_plane_normal * near_plane_distance;

            self.horizon_distance = horizon_distance;
            self.horizon_plane_normal = Some(horizon_plane_normal);
            self.horizon_plane_position = Some(horizon_plane_position);
        } else {
            // 相机在球内（或球面上）：不存在可见地平线。
            self.horizon_distance = f64::MAX;
            self.horizon_plane_normal = None;
            self.horizon_plane_position = None;
        }
    }

    /// 遮挡体的球心位置。
    pub fn position(&self) -> DVec3 {
        self.occluder_position
    }

    /// 遮挡体的半径。
    pub fn radius(&self) -> f64 {
        self.occluder_radius
    }

    /// 相机的位置。
    pub fn camera_position(&self) -> DVec3 {
        self.camera_position
    }

    /// 判断某个球（被遮挡对象）是否因遮挡体而不可见。
    ///
    /// # 参数
    /// - `occludee`：待测的被遮挡包围球。
    ///
    /// # 返回
    /// 存在可见地平线且被遮挡球未被完全遮住时返回 `true`；相机在球内（无地平线）时返回 `false`。
    pub fn is_bounding_sphere_visible(&self, occludee: &BoundingSphere) -> bool {
        let occludee_position = occludee.center;
        let occludee_radius = occludee.radius;

        if self.horizon_distance != f64::MAX {
            let temp_vec = occludee_position - self.occluder_position;
            let mut temp = self.occluder_radius - occludee_radius;
            temp = temp_vec.length_squared() - temp * temp;

            if occludee_radius < self.occluder_radius {
                if temp > 0.0 {
                    temp = temp.sqrt() + self.horizon_distance;
                    let temp_vec2 = occludee_position - self.camera_position;
                    return temp * temp + occludee_radius * occludee_radius
                        > temp_vec2.length_squared();
                }
                return false;
            }

            // 被遮挡对象半径 >= 遮挡体半径
            if temp > 0.0 {
                let temp_vec2 = occludee_position - self.camera_position;
                let temp_vec_magnitude_squared = temp_vec2.length_squared();
                let occluder_radius_squared = self.occluder_radius * self.occluder_radius;
                let occludee_radius_squared = occludee_radius * occludee_radius;
                if (self.horizon_distance * self.horizon_distance + occluder_radius_squared)
                    * occludee_radius_squared
                    > temp_vec_magnitude_squared * occluder_radius_squared
                {
                    return true;
                }
                temp = temp.sqrt() + self.horizon_distance;
                return temp * temp + occludee_radius_squared > temp_vec_magnitude_squared;
            }

            // 被遮挡对象完全包含遮挡体
            return true;
        }

        false
    }

    /// 确定被遮挡对象的可见程度。
    ///
    /// # 参数
    /// - `occludee_bs`：被遮挡对象的包围球。
    ///
    /// # 返回
    /// [`Visibility`]：完全在遮挡体内为 `None`，跨地平线平面为 `Partial`，否则 `Full`。
    pub fn compute_visibility(&self, occludee_bs: &BoundingSphere) -> Visibility {
        let occludee_position = occludee_bs.center;
        let occludee_radius = occludee_bs.radius;

        if occludee_radius > self.occluder_radius {
            // 被遮挡球比遮挡球还大，不可能被完全遮住，直接完全可见。
            return Visibility::Full;
        }

        if self.horizon_distance != f64::MAX {
            let temp_vec = occludee_position - self.occluder_position;
            let mut temp = self.occluder_radius - occludee_radius;
            let occluder_to_occludee_dist_sqrd = temp_vec.length_squared();
            temp = occluder_to_occludee_dist_sqrd - temp * temp;

            if temp > 0.0 {
                // 被遮挡对象并非完全在遮挡体内部
                temp = temp.sqrt() + self.horizon_distance;
                let temp_vec2 = occludee_position - self.camera_position;
                let camera_to_occludee_dist_sqrd = temp_vec2.length_squared();

                if temp * temp + occludee_radius * occludee_radius
                    < camera_to_occludee_dist_sqrd
                {
                    return Visibility::None;
                }

                // 在不相交时检查是完全可见还是部分可见
                temp = self.occluder_radius + occludee_radius;
                temp = occluder_to_occludee_dist_sqrd - temp * temp;
                if temp > 0.0 {
                    temp = temp.sqrt() + self.horizon_distance;
                    return if camera_to_occludee_dist_sqrd
                        < temp * temp + occludee_radius * occludee_radius
                    {
                        Visibility::Full
                    } else {
                        Visibility::Partial
                    };
                }

                // 检查被遮挡对象确实与遮挡体相交时的情形
                if let (Some(hpn), Some(hpp)) =
                    (self.horizon_plane_normal, self.horizon_plane_position)
                {
                    let tv = occludee_position - hpp;
                    return if tv.dot(hpn) > -occludee_radius {
                        Visibility::Partial
                    } else {
                        Visibility::Full
                    };
                }
            }
        }

        Visibility::None
    }

    /// 计算一个可作为可见性函数中被遮挡对象位置的点。
    ///
    /// 沿遮挡体向被遮挡位置方向构造平面，将各候选位置投影到地平线上取最小夹角点，
    /// 再沿该方向外推至遮挡球面上得到一个代表点。
    ///
    /// # 参数
    /// - `occluder_bounding_sphere`：遮挡球。
    /// - `occludee_position`：被遮挡对象参考位置。
    /// - `positions`：候选位置集合（取其中最不利者）。
    ///
    /// # 返回
    /// 代表点（地心固定系）；位置为空、与球心重合或夹角接近 90° 时返回 `None`。
    pub fn compute_occludee_point(
        occluder_bounding_sphere: &BoundingSphere,
        occludee_position: DVec3,
        positions: &[DVec3],
    ) -> Option<DVec3> {
        if positions.is_empty() {
            return None;
        }

        let occluder_position = occluder_bounding_sphere.center;
        let occluder_radius = occluder_bounding_sphere.radius;

        if occluder_position == occludee_position {
            return None;
        }

        // 计算一个法线从遮挡体指向被遮挡对象位置的平面。
        let occluder_plane_normal = (occludee_position - occluder_position).normalize();
        let occluder_plane_d = -occluder_plane_normal.dot(occluder_position);

        let a_rotation_vector = Self::any_rotation_vector(
            occluder_position,
            occluder_plane_normal,
            occluder_plane_d,
        );

        let mut dot = Self::horizon_to_plane_normal_dot_product(
            occluder_bounding_sphere,
            occluder_plane_normal,
            occluder_plane_d,
            a_rotation_vector,
            positions[0],
        )?;

        // 逐个候选位置投影到平面法线，取最小点积（最靠近地平线的切点）。
        for i in 1..positions.len() {
            let temp_dot = Self::horizon_to_plane_normal_dot_product(
                occluder_bounding_sphere,
                occluder_plane_normal,
                occluder_plane_d,
                a_rotation_vector,
                positions[i],
            )?;
            if temp_dot < dot {
                dot = temp_dot;
            }
        }

        // 验证该点积不接近 90 度
        if dot < 0.00174532836589830883577820272085 {
            return None;
        }

        let distance = occluder_radius / dot;
        Some(occluder_position + occluder_plane_normal * distance)
    }

    /// 由矩形计算被遮挡对象点。
    ///
    /// 先用 [`Rectangle::subsample`] 采样矩形边界点并求其包围球；若包围球中
    /// 心不在椭球心，则以最小半径球为遮挡体求代表点，否则返回 `None`。
    pub fn compute_occludee_point_from_rectangle(
        rectangle: &crate::rectangle::Rectangle,
        ellipsoid: &crate::ellipsoid::Ellipsoid,
    ) -> Option<DVec3> {
        let positions = rectangle.subsample(ellipsoid, 0.0);
        let bs = BoundingSphere::from_points(&positions);

        let ellipsoid_center = DVec3::ZERO;
        if ellipsoid_center != bs.center {
            let occluder_bs = BoundingSphere::new(ellipsoid_center, ellipsoid.minimum_radius());
            Self::compute_occludee_point(&occluder_bs, bs.center, &positions)
        } else {
            None
        }
    }

    /// 在遮挡体平面内计算任意一个旋转向量。
    ///
    /// 选取法线绝对值最大的主轴以避免退化，先取平面上一个候选点，再沿单位轴
    /// 投影回平面，最后归一化为从球心指向该点的方向。
    ///
    /// # 参数
    /// - `occluder_position`：遮挡球心。
    /// - `occluder_plane_normal`/`occluder_plane_d`：平面法线与距离系数。
    pub fn any_rotation_vector(
        occluder_position: DVec3,
        occluder_plane_normal: DVec3,
        occluder_plane_d: f64,
    ) -> DVec3 {
        let temp_vec0 = DVec3::new(
            occluder_plane_normal.x.abs(),
            occluder_plane_normal.y.abs(),
            occluder_plane_normal.z.abs(),
        );
        let mut major_axis = if temp_vec0.x > temp_vec0.y { 0 } else { 1 };
        if (major_axis == 0 && temp_vec0.z > temp_vec0.x)
            || (major_axis == 1 && temp_vec0.z > temp_vec0.y)
        {
            major_axis = 2;
        }

        let (mut point_on_plane, unit_axis) = match major_axis {
            0 => (
                DVec3::new(
                    occluder_position.x,
                    occluder_position.y + 1.0,
                    occluder_position.z + 1.0,
                ),
                DVec3::X,
            ),
            1 => (
                DVec3::new(
                    occluder_position.x + 1.0,
                    occluder_position.y,
                    occluder_position.z + 1.0,
                ),
                DVec3::Y,
            ),
            _ => (
                DVec3::new(
                    occluder_position.x + 1.0,
                    occluder_position.y + 1.0,
                    occluder_position.z,
                ),
                DVec3::Z,
            ),
        };

        let u = (occluder_plane_normal.dot(point_on_plane) + occluder_plane_d)
            / -occluder_plane_normal.dot(unit_axis);
        point_on_plane += unit_axis * u;
        (point_on_plane - occluder_position).normalize()
    }

    /// 为特定位置计算旋转向量。
    ///
    /// 若位置方向与平面法线不近似平行，则取二者叉积作为旋转轴；否则回退
    /// 到 [`Occluder::any_rotation_vector`] 预先算好的任意旋转轴。
    fn rotation_vector(
        occluder_position: DVec3,
        occluder_plane_normal: DVec3,
        _occluder_plane_d: f64,
        position: DVec3,
        any_rotation_vector: DVec3,
    ) -> DVec3 {
        let position_direction = (position - occluder_position).normalize();
        if occluder_plane_normal.dot(position_direction)
            < 0.99999998476912904932780850903444
        {
            let cross_product = occluder_plane_normal.cross(position_direction);
            let length = cross_product.length();
            if length > 1e-13 {
                return cross_product.normalize();
            }
        }
        any_rotation_vector
    }

    /// 计算地平线到平面法线的点积。
    ///
    /// 先由位置到球心的距离算出切线地平线参数，再把 position-to-occluder 向量绕
    /// 旋转轴转 90° 得到两个切点方向，分别用法线点积后取较小者作为结果。
    /// 若位置在遮挡球内则返回 `None`。
    fn horizon_to_plane_normal_dot_product(
        occluder_bs: &BoundingSphere,
        occluder_plane_normal: DVec3,
        occluder_plane_d: f64,
        any_rotation_vector: DVec3,
        position: DVec3,
    ) -> Option<f64> {
        let occluder_position = occluder_bs.center;
        let occluder_radius = occluder_bs.radius;

        // 验证位置在遮挡体之外
        let mut position_to_occluder = occluder_position - position;
        let occluder_to_position_distance_squared = position_to_occluder.length_squared();
        let occluder_radius_squared = occluder_radius * occluder_radius;
        if occluder_to_position_distance_squared < occluder_radius_squared {
            return None;
        }

        // 地平线参数
        let horizon_distance_squared =
            occluder_to_position_distance_squared - occluder_radius_squared;
        let horizon_distance = horizon_distance_squared.sqrt();
        let occluder_to_position_distance = occluder_to_position_distance_squared.sqrt();
        let inv_occluder_to_position_distance = 1.0 / occluder_to_position_distance;
        let cos_theta = horizon_distance * inv_occluder_to_position_distance;
        let horizon_plane_distance = cos_theta * horizon_distance;
        position_to_occluder = position_to_occluder.normalize();
        let horizon_plane_position =
            position + position_to_occluder * horizon_plane_distance;
        let horizon_cross_distance = (horizon_distance_squared
            - horizon_plane_distance * horizon_plane_distance)
            .sqrt();

        // 将 position-to-occluder 向量旋转 90 度
        let temp_vec = Self::rotation_vector(
            occluder_position,
            occluder_plane_normal,
            occluder_plane_d,
            position,
            any_rotation_vector,
        );

        let horizon_cross_direction = DVec3::new(
            temp_vec.x * temp_vec.x * position_to_occluder.x
                + (temp_vec.x * temp_vec.y - temp_vec.z) * position_to_occluder.y
                + (temp_vec.x * temp_vec.z + temp_vec.y) * position_to_occluder.z,
            (temp_vec.x * temp_vec.y + temp_vec.z) * position_to_occluder.x
                + temp_vec.y * temp_vec.y * position_to_occluder.y
                + (temp_vec.y * temp_vec.z - temp_vec.x) * position_to_occluder.z,
            (temp_vec.x * temp_vec.z - temp_vec.y) * position_to_occluder.x
                + (temp_vec.y * temp_vec.z + temp_vec.x) * position_to_occluder.y
                + temp_vec.z * temp_vec.z * position_to_occluder.z,
        )
        .normalize();

        // 地平线位置
        let offset = horizon_cross_direction * horizon_cross_distance;

        let temp_vec0 =
            ((horizon_plane_position + offset) - occluder_position).normalize();
        let dot0 = occluder_plane_normal.dot(temp_vec0);

        let temp_vec1 =
            ((horizon_plane_position - offset) - occluder_position).normalize();
        let dot1 = occluder_plane_normal.dot(temp_vec1);

        Some(if dot0 < dot1 { dot0 } else { dot1 })
    }
}
