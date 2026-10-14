//! 包围体 - BoundingSphere、OrientedBoundingBox、AxisAlignedBoundingBox。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::needless_range_loop, clippy::assign_op_pattern)]
use crate::cartographic::Cartographic;
use crate::ellipsoid::Ellipsoid;
use crate::math_utils::{self, EPSILON10, EPSILON15, EPSILON20, PI_F64, TWO_PI};
use crate::projection::MapProjection;
use crate::ray::{ray_plane, Intersect, Plane, Ray};
use crate::rectangle::Rectangle;
use glam::{DMat3, DMat4, DVec2, DVec3};
use serde::{Deserialize, Serialize};

/// 由一个中心点和半径定义的包围球。
/// 映射到 CesiumJS `BoundingSphere`
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct BoundingSphere {
    /// 球的中心。
    pub center: DVec3,
    /// 球的半径。
    pub radius: f64,
}

impl BoundingSphere {
    /// 由中心与半径直接构造包围球。
    ///
    /// # 参数
    /// - `center`：球心（笛卡尔，米）。
    /// - `radius`：半径（米）。
    pub fn new(center: DVec3, radius: f64) -> Self {
        Self { center, radius }
    }

    /// 计算一个紧贴包围一组 3D 点的包围球。
    /// 同时运行一种朴素算法和 Ritter 算法，并返回较小的那个球。
    /// 映射到 `BoundingSphere.fromPoints`
    pub fn from_points(points: &[DVec3]) -> Self {
        if points.is_empty() {
            return Self {
                center: DVec3::ZERO,
                radius: 0.0,
            };
        }

        // 找出 x、y、z 分量最小/最大的点。
        let mut x_min = points[0];
        let mut y_min = points[0];
        let mut z_min = points[0];
        let mut x_max = points[0];
        let mut y_max = points[0];
        let mut z_max = points[0];
        for &p in points.iter().skip(1) {
            if p.x < x_min.x {
                x_min = p;
            }
            if p.x > x_max.x {
                x_max = p;
            }
            if p.y < y_min.y {
                y_min = p;
            }
            if p.y > y_max.y {
                y_max = p;
            }
            if p.z < z_min.z {
                z_min = p;
            }
            if p.z > z_max.z {
                z_max = p;
            }
        }

        Self::from_points_with_extremes(points, x_min, y_min, z_min, x_max, y_max, z_max)
    }

    /// `from_points`、`from_vertices` 和
    /// `from_encoded_cartesian_vertices` 共用的 Ritter + 朴素核心。`points` 是位置的完整列表，而
    /// `*_min`/`*_max` 参数是沿各轴的极值点。
    fn from_points_with_extremes(
        points: &[DVec3],
        x_min: DVec3,
        y_min: DVec3,
        z_min: DVec3,
        x_max: DVec3,
        y_max: DVec3,
        z_max: DVec3,
    ) -> Self {
        // 计算 x-、y-、z- 跨度（每个分量最小值与最大值之间的距离平方）。
        let x_span = (x_max - x_min).length_squared();
        let y_span = (y_max - y_min).length_squared();
        let z_span = (z_max - z_min).length_squared();

        // 将直径端点设为最大的那个跨度。
        let mut diameter1 = x_min;
        let mut diameter2 = x_max;
        let mut max_span = x_span;
        if y_span > max_span {
            max_span = y_span;
            diameter1 = y_min;
            diameter2 = y_max;
        }
        if z_span > max_span {
            diameter1 = z_min;
            diameter2 = z_max;
        }

        // 由 Ritter 算法得到的初始球。
        let mut ritter_center = (diameter1 + diameter2) * 0.5;
        let mut radius_squared = (diameter2 - ritter_center).length_squared();
        let mut ritter_radius = radius_squared.sqrt();

        // 使用朴素方法找到的球心。
        let min_box_pt = DVec3::new(x_min.x, y_min.y, z_min.z);
        let max_box_pt = DVec3::new(x_max.x, y_max.y, z_max.z);
        let naive_center = (min_box_pt + max_box_pt) * 0.5;

        // 第二遍：找到朴素半径，并修正 Ritter 球以包含所有点。
        let mut naive_radius: f64 = 0.0;
        for &current_pos in points {
            let r = (current_pos - naive_center).length();
            if r > naive_radius {
                naive_radius = r;
            }

            let old_center_to_point_squared = (current_pos - ritter_center).length_squared();
            if old_center_to_point_squared > radius_squared {
                let old_center_to_point = old_center_to_point_squared.sqrt();
                ritter_radius = (ritter_radius + old_center_to_point) * 0.5;
                radius_squared = ritter_radius * ritter_radius;
                let old_to_new = old_center_to_point - ritter_radius;
                ritter_center = (ritter_center * ritter_radius + current_pos * old_to_new)
                    / old_center_to_point;
            }
        }

        if ritter_radius < naive_radius {
            Self {
                center: ritter_center,
                radius: ritter_radius,
            }
        } else {
            Self {
                center: naive_center,
                radius: naive_radius,
            }
        }
    }

    /// 由以扁平数组存储的点（X, Y, Z 顺序）计算紧贴包围球，
    /// 可选带相对中心和 stride。
    /// 映射到 `BoundingSphere.fromVertices`
    ///
    /// # 参数
    /// - `vertices`：扁平顶点数组，按 (X,Y,Z) 交错。
    /// - `center`：顶点坐标的参考中心偏移。
    /// - `stride`：相邻顶点间的元素个数（至少 3）。
    pub fn from_vertices(vertices: &[f64], center: DVec3, stride: usize) -> Self {
        debug_assert!(stride >= 3, "stride must be at least 3");
        if vertices.is_empty() {
            return Self {
                center: DVec3::ZERO,
                radius: 0.0,
            };
        }

        let mut positions = Vec::new();
        let mut i = 0;
        while i + 2 < vertices.len() {
            positions.push(DVec3::new(
                vertices[i] + center.x,
                vertices[i + 1] + center.y,
                vertices[i + 2] + center.z,
            ));
            i += stride;
        }

        if positions.is_empty() {
            return Self {
                center: DVec3::ZERO,
                radius: 0.0,
            };
        }

        let mut x_min = positions[0];
        let mut y_min = positions[0];
        let mut z_min = positions[0];
        let mut x_max = positions[0];
        let mut y_max = positions[0];
        let mut z_max = positions[0];
        for &p in positions.iter().skip(1) {
            if p.x < x_min.x {
                x_min = p;
            }
            if p.x > x_max.x {
                x_max = p;
            }
            if p.y < y_min.y {
                y_min = p;
            }
            if p.y > y_max.y {
                y_max = p;
            }
            if p.z < z_min.z {
                z_min = p;
            }
            if p.z > z_max.z {
                z_max = p;
            }
        }

        Self::from_points_with_extremes(
            &positions, x_min, y_min, z_min, x_max, y_max, z_max,
        )
    }

    /// 由编码的（高/低）扁平数组计算紧贴包围球。
    /// 映射到 `BoundingSphere.fromEncodedCartesianVertices`
    ///
    /// # 参数
    /// - `positions_high`：高位分量的扁平数组。
    /// - `positions_low`：低位分量的扁平数组；两者相加还原完整坐标。
    pub fn from_encoded_cartesian_vertices(positions_high: &[f64], positions_low: &[f64]) -> Self {
        if positions_high.len() != positions_low.len() || positions_high.is_empty() {
            return Self {
                center: DVec3::ZERO,
                radius: 0.0,
            };
        }

        let mut positions = Vec::new();
        let mut i = 0;
        while i + 2 < positions_high.len() {
            positions.push(DVec3::new(
                positions_high[i] + positions_low[i],
                positions_high[i + 1] + positions_low[i + 1],
                positions_high[i + 2] + positions_low[i + 2],
            ));
            i += 3;
        }

        if positions.is_empty() {
            return Self {
                center: DVec3::ZERO,
                radius: 0.0,
            };
        }

        let mut x_min = positions[0];
        let mut y_min = positions[0];
        let mut z_min = positions[0];
        let mut x_max = positions[0];
        let mut y_max = positions[0];
        let mut z_max = positions[0];
        for &p in positions.iter().skip(1) {
            if p.x < x_min.x {
                x_min = p;
            }
            if p.x > x_max.x {
                x_max = p;
            }
            if p.y < y_min.y {
                y_min = p;
            }
            if p.y > y_max.y {
                y_max = p;
            }
            if p.z < z_min.z {
                z_min = p;
            }
            if p.z > z_max.z {
                z_max = p;
            }
        }

        Self::from_points_with_extremes(
            &positions, x_min, y_min, z_min, x_max, y_max, z_max,
        )
    }

    /// 由 2D 投影下的矩形计算包围球。
    /// 映射到 `BoundingSphere.fromRectangle2D`
    ///
    /// # 参数
    /// - `rectangle`：经纬矩形。
    /// - `projection`：将矩形投到 2D 的地图投影。
    pub fn from_rectangle_2d(rectangle: &Rectangle, projection: &dyn MapProjection) -> Self {
        Self::from_rectangle_with_heights_2d(rectangle, projection, 0.0, 0.0)
    }

    /// 由 2D 投影下的矩形计算包围球，并考虑最小和最大高度。
    /// 映射到 `BoundingSphere.fromRectangleWithHeights2D`
    ///
    /// # 参数
    /// - `rectangle`：经纬矩形，取其西南/东北角作投影范围。
    /// - `projection`：将矩形投到平面的地图投影。
    /// - `minimum_height`：底面高度（米）。
    /// - `maximum_height`：顶面高度（米）。
    ///
    /// # 返回
    /// 覆盖投影后 3D 长方体对角线一半的包围球。
    pub fn from_rectangle_with_heights_2d(
        rectangle: &Rectangle,
        projection: &dyn MapProjection,
        minimum_height: f64,
        maximum_height: f64,
    ) -> Self {
        let mut southwest = rectangle.southwest();
        southwest.height = minimum_height;
        let mut northeast = rectangle.northeast();
        northeast.height = maximum_height;

        let lower_left = projection.project(&southwest);
        let upper_right = projection.project(&northeast);

        let width = upper_right.x - lower_left.x;
        let height = upper_right.y - lower_left.y;
        let elevation = upper_right.z - lower_left.z;

        Self {
            center: DVec3::new(
                lower_left.x + width * 0.5,
                lower_left.y + height * 0.5,
                lower_left.z + elevation * 0.5,
            ),
            radius: (width * width + height * height + elevation * elevation).sqrt() * 0.5,
        }
    }

    /// 使用采样的点在 3D 中由矩形计算包围球。
    /// 映射到 `BoundingSphere.fromRectangle3D`
    ///
    /// # 参数
    /// - `rectangle`：经纬矩形。
    /// - `ellipsoid`：采样所依据的椭球。
    /// - `surface_height`：矩形面距椭球的高度（米）。
    pub fn from_rectangle_3d(
        rectangle: &Rectangle,
        ellipsoid: &crate::ellipsoid::Ellipsoid,
        surface_height: f64,
    ) -> Self {
        let positions = rectangle.subsample(ellipsoid, surface_height);
        Self::from_points(&positions)
    }

    /// 由轴对齐盒子的角点计算包围球。
    /// 映射到 `BoundingSphere.fromCornerPoints`
    ///
    /// # 参数
    /// - `corner`/`opposite_corner`：轴对齐盒子的两个对角顶点。
    pub fn from_corner_points(corner: DVec3, opposite_corner: DVec3) -> Self {
        let center = (corner + opposite_corner) * 0.5;
        let radius = center.distance(opposite_corner);
        Self { center, radius }
    }

    /// 创建一个涵盖椭球的包围球。
    /// 映射到 `BoundingSphere.fromEllipsoid`
    ///
    /// # 参数
    /// - `ellipsoid`：目标椭球；球心在原点，半径取其最大半径。
    ///
    /// # 返回
    /// 一个足以包含整个椭球的包围球。
    pub fn from_ellipsoid(ellipsoid: &crate::ellipsoid::Ellipsoid) -> Self {
        Self {
            center: DVec3::ZERO,
            radius: ellipsoid.maximum_radius(),
        }
    }

    /// 计算紧贴包围所提供的一组包围球的包围球。
    /// 映射到 `BoundingSphere.fromBoundingSpheres`
    ///
    /// # 参数
    /// - `spheres`：待统一包围的球列表；空列表退化为零球。
    pub fn from_bounding_spheres(spheres: &[BoundingSphere]) -> Self {
        if spheres.is_empty() {
            return Self {
                center: DVec3::ZERO,
                radius: 0.0,
            };
        }
        if spheres.len() == 1 {
            return spheres[0];
        }
        if spheres.len() == 2 {
            return spheres[0].union(&spheres[1]);
        }

        let positions: Vec<DVec3> = spheres.iter().map(|s| s.center).collect();
        let mut result = Self::from_points(&positions);
        let center = result.center;
        let mut radius = result.radius;
        for s in spheres {
            radius = radius.max(center.distance(s.center) + s.radius);
        }
        result.radius = radius;
        result
    }

    /// 计算紧贴包围一个仿射变换的包围球。
    /// 映射到 `BoundingSphere.fromTransformation`
    ///
    /// # 参数
    /// - `transformation`：4x4 仿射变换；取其平移作为球心、缩放长度一半作为半径。
    pub fn from_transformation(transformation: &glam::DMat4) -> Self {
        let center = transformation.w_axis.truncate();
        let scale = DVec3::new(
            transformation.x_axis.truncate().length(),
            transformation.y_axis.truncate().length(),
            transformation.z_axis.truncate().length(),
        );
        let radius = 0.5 * scale.length();
        Self { center, radius }
    }

    /// 计算从球上最近点到某个点的距离。
    /// （非平方的便捷封装；CesiumJS 暴露的是 `distanceSquaredTo`。）
    ///
    /// # 参数
    /// - `point`：目标点。
    ///
    /// # 返回
    /// 点到球面的最短距离；点在球内时为 0。
    pub fn distance_to(&self, point: DVec3) -> f64 {
        let dist = (point - self.center).length();
        (dist - self.radius).max(0.0)
    }

    /// 计算从球上最近点到某个点的估计距离平方。
    /// 映射到 `BoundingSphere.distanceSquaredTo`
    ///
    /// # 参数
    /// - `cartesian`：目标点。
    ///
    /// # 返回
    /// 点到球面最短距离的平方；点在球内时为 0。
    pub fn distance_squared_to(&self, cartesian: DVec3) -> f64 {
        let distance = (self.center - cartesian).length() - self.radius;
        if distance <= 0.0 {
            0.0
        } else {
            distance * distance
        }
    }

    /// 判断某个点是否在球内。
    ///
    /// # 参数
    /// - `point`：待检测的点。
    ///
    /// # 返回
    /// 若点到球心的距离不超过半径则返回 `true`。
    pub fn contains(&self, point: DVec3) -> bool {
        (point - self.center).length_squared() <= self.radius * self.radius
    }

    /// 计算同时包含两个球的包围球。
    /// 映射到 `BoundingSphere.union`
    ///
    /// # 参数
    /// - `other`：与本球合并的另一个球；返回能同时容纳两者的最小球。
    pub fn union(&self, other: &Self) -> Self {
        let left_center = self.center;
        let left_radius = self.radius;
        let right_center = other.center;
        let right_radius = other.radius;

        let to_right_center = right_center - left_center;
        let center_separation = to_right_center.length();

        if left_radius >= center_separation + right_radius {
            // 左侧球胜出。
            return *self;
        }
        if right_radius >= center_separation + left_radius {
            // 右侧球胜出。
            return *other;
        }

        // 两个切点，各位于每个球的远侧。
        let half_distance_between_tangent_points =
            (left_radius + center_separation + right_radius) * 0.5;
        let center = left_center
            + to_right_center
                * ((-left_radius + half_distance_between_tangent_points) / center_separation);

        Self {
            center,
            radius: half_distance_between_tangent_points,
        }
    }

    /// 扩大该球以包含所提供的点。
    /// 映射到 `BoundingSphere.expand`
    ///
    /// # 参数
    /// - `point`：需被包含的点；若球心不变得只扩大半径。
    pub fn expand(&self, point: DVec3) -> Self {
        let radius = (point - self.center).length();
        Self {
            center: self.center,
            radius: self.radius.max(radius),
        }
    }

    /// 判断球位于平面的哪一侧。
    /// 映射到 `BoundingSphere.intersectPlane`
    ///
    /// # 参数
    /// - `normal`：平面单位法线（约定朝内）。
    /// - `distance`：平面常量项，满足 `dot(normal, x) + distance = 0`。
    pub fn intersect_plane(&self, normal: DVec3, distance: f64) -> Intersect {
        let distance_to_plane = normal.dot(self.center) + distance;

        if distance_to_plane < -self.radius {
            Intersect::Outside
        } else if distance_to_plane < self.radius {
            Intersect::Intersecting
        } else {
            Intersect::Inside
        }
    }

    /// 将一个 4x4 仿射变换矩阵应用于该球。
    /// 映射到 `BoundingSphere.transform`
    ///
    /// # 参数
    /// - `matrix`：4x4 仿射变换；新半径为原半径乘以最大轴缩放。
    pub fn transform(&self, matrix: &glam::DMat4) -> Self {
        let center = matrix.transform_point3(self.center);
        let scale_x = matrix.x_axis.truncate().length();
        let scale_y = matrix.y_axis.truncate().length();
        let scale_z = matrix.z_axis.truncate().length();
        let max_scale = scale_x.max(scale_y).max(scale_z);
        Self {
            center,
            radius: self.radius * max_scale,
        }
    }

    /// 应用一个 4x4 变换矩阵，假设无缩放。
    /// 映射到 `BoundingSphere.transformWithoutScale`
    ///
    /// # 参数
    /// - `matrix`：仅含旋转/平移（无缩放）的 4x4 变换，半径保持不变。
    pub fn transform_without_scale(&self, matrix: &glam::DMat4) -> Self {
        Self {
            center: matrix.transform_point3(self.center),
            radius: self.radius,
        }
    }

    /// 计算沿某方向从某个位置出发的最近和最远距离。
    /// 映射到 `BoundingSphere.computePlaneDistances`
    ///
    /// # 参数
    /// - `position`：观测起点。
    /// - `direction`：沿射线的单位方向；返回该方向上最近/最远距离区间。
    pub fn compute_plane_distances(&self, position: DVec3, direction: DVec3) -> Interval {
        let to_center = self.center - position;
        let mag = direction.dot(to_center);
        Interval {
            start: mag - self.radius,
            stop: mag + self.radius,
        }
    }

    /// 计算球的体积。
    /// 映射到 `BoundingSphere.prototype.volume`
    ///
    /// # 返回
    /// 按 `(4/3)·π·r³` 计算的球体积（立方米）。
    pub fn volume(&self) -> f64 {
        let radius = self.radius;
        (4.0 / 3.0) * std::f64::consts::PI * radius * radius * radius
    }
}

/// 带有起始值和终止值的数值区间。
/// 映射到 CesiumJS `Interval`
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    /// 起始（最小）值。
    pub start: f64,
    /// 终止（最大）值。
    pub stop: f64,
}

impl Interval {
    /// 由起始/终止值构造一个数值区间。
    ///
    /// # 参数
    /// - `start`：起始（最小）值。
    /// - `stop`：终止（最大）值。
    pub fn new(start: f64, stop: f64) -> Self {
        Self { start, stop }
    }
}

/// 由一个中心和半轴定义的方向包围盒。
/// 映射到 CesiumJS `OrientedBoundingBox`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OrientedBoundingBox {
    /// 盒子的中心。
    pub center: DVec3,
    /// 三个半轴向量（各列定义盒子的朝向和尺寸）。
    pub half_axes: DMat3,
}

impl Default for OrientedBoundingBox {
    /// 默认 OBB：中心在原点、半轴均为零矩阵。
    fn default() -> Self {
        Self {
            center: DVec3::ZERO,
            half_axes: DMat3::ZERO,
        }
    }
}

impl OrientedBoundingBox {
    /// 由中心与半轴矩阵直接构造 OBB。
    ///
    /// # 参数
    /// - `center`：盒子中心。
    /// - `half_axes`：三列分别为三个朝向轴乘以各自半长。
    pub fn new(center: DVec3, half_axes: DMat3) -> Self {
        Self { center, half_axes }
    }

    /// 由中心、方向轴和半长创建一个 OBB。
    ///
    /// # 参数
    /// - `center`：盒子中心。
    /// - `u_axis`/`v_axis`/`w_axis`：三个相互垂直的朝向轴。
    /// - `half_u`/`half_v`/`half_w`：沿对应轴的半长（米）。
    pub fn from_axes_half_lengths(
        center: DVec3,
        u_axis: DVec3,
        v_axis: DVec3,
        w_axis: DVec3,
        half_u: f64,
        half_v: f64,
        half_w: f64,
    ) -> Self {
        let half_axes = DMat3::from_cols(
            u_axis * half_u,
            v_axis * half_v,
            w_axis * half_w,
        );
        Self { center, half_axes }
    }

    /// 计算从 OBB 上最近点到某个点的距离。
    /// 映射到 `OrientedBoundingBox.distanceTo`
    ///
    /// # 参数
    /// - `point`：目标点；返回其到盒子表面的最短距离（内部为 0）。
    pub fn distance_to(&self, point: DVec3) -> f64 {
        let offset = point - self.center;

        let u = self.half_axes.x_axis;
        let v = self.half_axes.y_axis;
        let w = self.half_axes.z_axis;

        let u_half = u.length();
        let v_half = v.length();
        let w_half = w.length();

        let u_dir = if u_half > 0.0 { u / u_half } else { DVec3::X };
        let v_dir = if v_half > 0.0 { v / v_half } else { DVec3::Y };
        let w_dir = if w_half > 0.0 { w / w_half } else { DVec3::Z };

        let d_u = offset.dot(u_dir).abs() - u_half;
        let d_v = offset.dot(v_dir).abs() - v_half;
        let d_w = offset.dot(w_dir).abs() - w_half;

        let outside = DVec3::new(d_u.max(0.0), d_v.max(0.0), d_w.max(0.0));
        outside.length()
    }

    /// 将该 OBB 转换为一个包围球。
    ///
    /// 球心与盒子中心重合，半径取三个半轴长度中的最大值。
    pub fn to_bounding_sphere(&self) -> BoundingSphere {
        let radius = self.half_axes.x_axis.length().max(
            self.half_axes.y_axis.length().max(self.half_axes.z_axis.length()),
        );
        BoundingSphere {
            center: self.center,
            radius,
        }
    }

    /// 判断该 OBB 与一个平面的相交情况。
    ///
    /// # 参数
    /// - `normal`：平面单位法线（朝内）。
    /// - `distance`：平面常量项；有效半径为三半轴在法线上投影绝对值之和。
    pub fn intersect_plane(&self, normal: DVec3, distance: f64) -> Intersect {
        let u = self.half_axes.x_axis;
        let v = self.half_axes.y_axis;
        let w = self.half_axes.z_axis;

        let rad_effective =
            normal.dot(u).abs() + normal.dot(v).abs() + normal.dot(w).abs();
        let dist_to_center = normal.dot(self.center) + distance;

        // CesiumJS 约定：平面法线朝内。
        if dist_to_center <= -rad_effective {
            Intersect::Outside
        } else if dist_to_center >= rad_effective {
            Intersect::Inside
        } else {
            Intersect::Intersecting
        }
    }

    /// 为由给定位置计算一个 OrientedBoundingBox。
    ///
    /// 这是 Stefan Gottschalk 的《Collision Queries using Oriented Bounding Boxes》
    /// （博士论文）解法的实现：它构建点的协方差矩阵，提取其特征分解
    /// （经典 Jacobi）以获得盒子朝向，然后沿每个特征轴拟合范围。
    /// 映射到 `OrientedBoundingBox.fromPoints`
    pub fn from_points(points: &[DVec3]) -> Self {
        if points.is_empty() {
            return Self {
                center: DVec3::ZERO,
                half_axes: DMat3::ZERO,
            };
        }

        let length = points.len();
        let inv_length = 1.0 / length as f64;

        let mut mean_point = points[0];
        for p in points.iter().skip(1) {
            mean_point += *p;
        }
        mean_point *= inv_length;

        let mut exx = 0.0;
        let mut exy = 0.0;
        let mut exz = 0.0;
        let mut eyy = 0.0;
        let mut eyz = 0.0;
        let mut ezz = 0.0;
        for p in points {
            let d = *p - mean_point;
            exx += d.x * d.x;
            exy += d.x * d.y;
            exz += d.x * d.z;
            eyy += d.y * d.y;
            eyz += d.y * d.z;
            ezz += d.z * d.z;
        }
        exx *= inv_length;
        exy *= inv_length;
        exz *= inv_length;
        eyy *= inv_length;
        eyz *= inv_length;
        ezz *= inv_length;

        // 列主序协方差矩阵（与 CesiumJS 扁平数组布局一致）。
        let covariance = DMat3::from_cols_array(&[
            exx, exy, exz, exy, eyy, eyz, exz, eyz, ezz,
        ]);

        let (unitary, _diagonal) = compute_eigen_decomposition(covariance);
        let rotation = unitary;

        let v1 = rotation.x_axis;
        let v2 = rotation.y_axis;
        let v3 = rotation.z_axis;

        let mut u1 = f64::MIN;
        let mut u2 = f64::MIN;
        let mut u3 = f64::MIN;
        let mut l1 = f64::MAX;
        let mut l2 = f64::MAX;
        let mut l3 = f64::MAX;
        for p in points {
            u1 = u1.max(v1.dot(*p));
            u2 = u2.max(v2.dot(*p));
            u3 = u3.max(v3.dot(*p));
            l1 = l1.min(v1.dot(*p));
            l2 = l2.min(v2.dot(*p));
            l3 = l3.min(v3.dot(*p));
        }

        let center = v1 * (0.5 * (l1 + u1)) + v2 * (0.5 * (l2 + u2)) + v3 * (0.5 * (l3 + u3));

        let scale = DVec3::new(u1 - l1, u2 - l2, u3 - l3) * 0.5;
        let half_axes = DMat3::from_cols(
            rotation.x_axis * scale.x,
            rotation.y_axis * scale.y,
            rotation.z_axis * scale.z,
        );

        Self { center, half_axes }
    }

    /// 计算一个 OrientedBoundingBox，约束 `Ellipsoid` 表面上的一个 `Rectangle`。
    ///
    /// 对于宽度不超过半个椭球的矩形（`width <= PI`），盒子与矩形中心处的
    /// 切平面对齐；更宽的矩形使用一个绕 Z 轴旋转的平面。映射到 `OrientedBoundingBox.fromRectangle`
    ///
    /// # Panic
    /// 仅在 debug 下进行的 `DeveloperError` 检查（通过 `debug_assert!`）：
    /// `rectangle.width` 必须在 `[0, 2*PI]` 内，`rectangle.height` 在 `[0, PI]` 内，且
    /// 椭球必须是旋转椭球（`radii.x == radii.y`）。
    pub fn from_rectangle(
        rectangle: &Rectangle,
        minimum_height: f64,
        maximum_height: f64,
        ellipsoid: &Ellipsoid,
    ) -> Self {
        debug_assert!(
            rectangle.width() >= 0.0 && rectangle.width() <= TWO_PI,
            "Rectangle width must be between 0 and 2 * pi"
        );
        debug_assert!(
            rectangle.height() >= 0.0 && rectangle.height() <= PI_F64,
            "Rectangle height must be between 0 and pi"
        );
        debug_assert!(
            math_utils::equals_epsilon(
                ellipsoid.radii().x,
                ellipsoid.radii().y,
                EPSILON15,
                EPSILON15
            ),
            "Ellipsoid must be an ellipsoid of revolution (radii.x == radii.y)"
        );

        if rectangle.width() <= PI_F64 {
            // 边界盒将与矩形中心处的切平面对齐。
            let tangent_point_cartographic = rectangle.center();
            let tangent_point = ellipsoid.cartographic_to_cartesian(&tangent_point_cartographic);
            let (tp_origin, x_axis, y_axis, z_axis) = tangent_plane_frame(tangent_point, ellipsoid);
            let plane = Plane::from_point_normal(tp_origin, z_axis);

            // 若矩形跨越赤道，则 CW 改为与赤道对齐
            // （因为它在赤道处向外突出最远）。
            let lon_center = tangent_point_cartographic.longitude;
            let lat_center = if rectangle.south < 0.0 && rectangle.north > 0.0 {
                0.0
            } else {
                tangent_point_cartographic.latitude
            };

            // 使用最大高度处的矩形计算 XY 范围。
            let nc = ellipsoid.cartographic_to_cartesian(&Cartographic::from_radians(
                lon_center,
                rectangle.north,
                maximum_height,
            ));
            let nw = ellipsoid.cartographic_to_cartesian(&Cartographic::from_radians(
                rectangle.west,
                rectangle.north,
                maximum_height,
            ));
            let cw = ellipsoid.cartographic_to_cartesian(&Cartographic::from_radians(
                rectangle.west,
                lat_center,
                maximum_height,
            ));
            let sw = ellipsoid.cartographic_to_cartesian(&Cartographic::from_radians(
                rectangle.west,
                rectangle.south,
                maximum_height,
            ));
            let sc = ellipsoid.cartographic_to_cartesian(&Cartographic::from_radians(
                lon_center,
                rectangle.south,
                maximum_height,
            ));

            let p_nc = project_to_nearest(tp_origin, x_axis, y_axis, z_axis, nc);
            let p_nw = project_to_nearest(tp_origin, x_axis, y_axis, z_axis, nw);
            let p_cw = project_to_nearest(tp_origin, x_axis, y_axis, z_axis, cw);
            let p_sw = project_to_nearest(tp_origin, x_axis, y_axis, z_axis, sw);
            let p_sc = project_to_nearest(tp_origin, x_axis, y_axis, z_axis, sc);

            let min_x = p_nw.x.min(p_cw.x).min(p_sw.x);
            let max_x = -min_x; // 对称

            let max_y = p_nw.y.max(p_nc.y);
            let min_y = p_sw.y.min(p_sc.y);

            // 使用最小高度处的矩形计算最小 Z，因为它比最大高度处更深。
            let nw_low = ellipsoid.cartographic_to_cartesian(&Cartographic::from_radians(
                rectangle.west,
                rectangle.north,
                minimum_height,
            ));
            let sw_low = ellipsoid.cartographic_to_cartesian(&Cartographic::from_radians(
                rectangle.west,
                rectangle.south,
                minimum_height,
            ));
            let min_z = plane.point_distance(nw_low).min(plane.point_distance(sw_low));
            let max_z = maximum_height; // 切平面在 height = 0 处接触表面

            return from_plane_extents(
                tp_origin, x_axis, y_axis, z_axis, min_x, max_x, min_y, max_y, min_z, max_z,
            );
        }

        // 处理矩形宽度大于 PI 的情形（环绕超过半个椭球）。
        let fully_above_equator = rectangle.south > 0.0;
        let fully_below_equator = rectangle.north < 0.0;
        let latitude_nearest_to_equator = if fully_above_equator {
            rectangle.south
        } else if fully_below_equator {
            rectangle.north
        } else {
            0.0
        };
        let center_longitude = rectangle.center().longitude;

        // 平面位于矩形的中心经度以及矩形中最接近赤道的那个纬度。
        // 它绕 Z 轴旋转。
        let mut plane_origin = ellipsoid.cartographic_to_cartesian(&Cartographic::from_radians(
            center_longitude,
            latitude_nearest_to_equator,
            maximum_height,
        ));
        plane_origin.z = 0.0; // 将平面置于赤道上，以简化平面法线的计算
        let is_pole = plane_origin.x.abs() < EPSILON10 && plane_origin.y.abs() < EPSILON10;
        let plane_normal = if !is_pole {
            plane_origin.normalize()
        } else {
            DVec3::X
        };
        let plane_y_axis = DVec3::Z;
        let plane_x_axis = plane_normal.cross(plane_y_axis);
        let plane = Plane::from_point_normal(plane_origin, plane_normal);

        // 获取相对于中心点的地平线点。这将是平面 X 维度上最远的范围。
        let horizon_cartesian = ellipsoid.cartographic_to_cartesian(&Cartographic::from_radians(
            center_longitude + math_utils::PI_OVER_TWO,
            latitude_nearest_to_equator,
            maximum_height,
        ));
        let max_x = plane
            .project_point_onto_plane(horizon_cartesian)
            .dot(plane_x_axis);
        let min_x = -max_x; // 对称

        // 获取最小和最大 Y，使用能给出最大范围的高度。
        let max_y = ellipsoid
            .cartographic_to_cartesian(&Cartographic::from_radians(
                0.0,
                rectangle.north,
                if fully_below_equator {
                    minimum_height
                } else {
                    maximum_height
                },
            ))
            .z;
        let min_y = ellipsoid
            .cartographic_to_cartesian(&Cartographic::from_radians(
                0.0,
                rectangle.south,
                if fully_above_equator {
                    minimum_height
                } else {
                    maximum_height
                },
            ))
            .z;

        let far_z = ellipsoid.cartographic_to_cartesian(&Cartographic::from_radians(
            rectangle.east,
            latitude_nearest_to_equator,
            maximum_height,
        ));
        let min_z = plane.point_distance(far_z);
        let max_z = 0.0; // 平面原点已位于 maxZ

        // min 和 max 均相对于平面坐标轴
        from_plane_extents(
            plane_origin,
            plane_x_axis,
            plane_y_axis,
            plane_normal,
            min_x,
            max_x,
            min_y,
            max_y,
            min_z,
            max_z,
        )
    }

    /// 计算一个包围仿射变换的 OrientedBoundingBox。
    /// 映射到 `OrientedBoundingBox.fromTransformation`
    ///
    /// # 参数
    /// - `transformation`：4x4 仿射变换；平移列作中心，三个缩放列之半作半轴。
    pub fn from_transformation(transformation: &DMat4) -> Self {
        let center = transformation.w_axis.truncate();
        let half_axes = DMat3::from_cols(
            transformation.x_axis.truncate(),
            transformation.y_axis.truncate(),
            transformation.z_axis.truncate(),
        ) * 0.5;
        Self { center, half_axes }
    }

    /// 计算从盒子中最近的点到某个点的估计距离平方。
    /// 若点位于盒子内部则返回 0。
    ///
    /// 这里处理退化轴的情况（一条/两条/三条零长度半轴）。
    /// 映射到 `OrientedBoundingBox.distanceSquaredTo`
    ///
    /// # 参数
    /// - `cartesian`：目标点；返回其到盒子最近点的估计距离平方（内部为 0）。
    pub fn distance_squared_to(&self, cartesian: DVec3) -> f64 {
        // 参见 Geometric Tools for Computer Graphics 10.4.2
        let offset = cartesian - self.center;

        let mut u = self.half_axes.x_axis;
        let mut v = self.half_axes.y_axis;
        let mut w = self.half_axes.z_axis;

        let u_half = u.length();
        let v_half = v.length();
        let w_half = w.length();

        let mut u_valid = true;
        let mut v_valid = true;
        let mut w_valid = true;

        if u_half > 0.0 {
            u /= u_half;
        } else {
            u_valid = false;
        }
        if v_half > 0.0 {
            v /= v_half;
        } else {
            v_valid = false;
        }
        if w_half > 0.0 {
            w /= w_half;
        } else {
            w_valid = false;
        }

        let number_of_degenerate_axes =
            (!u_valid as u8) + (!v_valid as u8) + (!w_valid as u8);

        if number_of_degenerate_axes == 1 {
            let mut degenerate_axis = u;
            let mut valid_axis1 = v;
            let mut valid_axis2 = w;
            if !v_valid {
                degenerate_axis = v;
                valid_axis1 = u;
            } else if !w_valid {
                degenerate_axis = w;
                valid_axis2 = u;
            }

            let valid_axis3 = valid_axis1.cross(valid_axis2);

            if degenerate_axis == u {
                u = valid_axis3;
            } else if degenerate_axis == v {
                v = valid_axis3;
            } else if degenerate_axis == w {
                w = valid_axis3;
            }
        } else if number_of_degenerate_axes == 2 {
            let mut valid_axis1 = u;
            let mut valid_axis1_is = 0u8; // 0 => u，1 => v，2 => w
            if v_valid {
                valid_axis1 = v;
                valid_axis1_is = 1;
            } else if w_valid {
                valid_axis1 = w;
                valid_axis1_is = 2;
            }

            let mut cross_vector = DVec3::Y;
            if cross_vector.abs_diff_eq(valid_axis1, math_utils::EPSILON3) {
                cross_vector = DVec3::X;
            }

            let mut valid_axis2 = valid_axis1.cross(cross_vector);
            valid_axis2 = valid_axis2.normalize();
            let mut valid_axis3 = valid_axis1.cross(valid_axis2);
            valid_axis3 = valid_axis3.normalize();

            match valid_axis1_is {
                0 => {
                    v = valid_axis2;
                    w = valid_axis3;
                }
                1 => {
                    w = valid_axis2;
                    u = valid_axis3;
                }
                _ => {
                    u = valid_axis2;
                    v = valid_axis3;
                }
            }
        } else if number_of_degenerate_axes == 3 {
            u = DVec3::X;
            v = DVec3::Y;
            w = DVec3::Z;
        }

        let p_prime = DVec3::new(offset.dot(u), offset.dot(v), offset.dot(w));

        let mut distance_squared = 0.0;
        let mut d;

        if p_prime.x < -u_half {
            d = p_prime.x + u_half;
            distance_squared += d * d;
        } else if p_prime.x > u_half {
            d = p_prime.x - u_half;
            distance_squared += d * d;
        }

        if p_prime.y < -v_half {
            d = p_prime.y + v_half;
            distance_squared += d * d;
        } else if p_prime.y > v_half {
            d = p_prime.y - v_half;
            distance_squared += d * d;
        }

        if p_prime.z < -w_half {
            d = p_prime.z + w_half;
            distance_squared += d * d;
        } else if p_prime.z > w_half {
            d = p_prime.z - w_half;
            distance_squared += d * d;
        }

        distance_squared
    }

    /// 计算沿 `direction`、从 `position` 到与包围盒相交的各平面的最近和最远距离。
    /// 映射到 `OrientedBoundingBox.computePlaneDistances`
    ///
    /// # 参数
    /// - `position`：观测起点。
    /// - `direction`：射线单位方向；对 8 个角点投影取最小/最大值。
    ///
    /// # 返回
    /// 该方向上盒子投影的最近/最远距离区间。
    pub fn compute_plane_distances(&self, position: DVec3, direction: DVec3) -> Interval {
        let mut min_dist = f64::INFINITY;
        let mut max_dist = f64::NEG_INFINITY;

        let center = self.center;
        let u = self.half_axes.x_axis;
        let v = self.half_axes.y_axis;
        let w = self.half_axes.z_axis;

        let signs: [(f64, f64, f64); 8] = [
            (1.0, 1.0, 1.0),
            (1.0, 1.0, -1.0),
            (1.0, -1.0, 1.0),
            (1.0, -1.0, -1.0),
            (-1.0, 1.0, 1.0),
            (-1.0, 1.0, -1.0),
            (-1.0, -1.0, 1.0),
            (-1.0, -1.0, -1.0),
        ];

        for (su, sv, sw) in signs {
            let corner = center + u * su + v * sv + w * sw;
            let to_center = corner - position;
            let mag = direction.dot(to_center);
            min_dist = min_dist.min(mag);
            max_dist = max_dist.max(mag);
        }

        Interval::new(min_dist, max_dist)
    }

    /// 计算盒子的八个角点，按以下顺序排列：
    /// `(-X,-Y,-Z), (-X,-Y,+Z), (-X,+Y,-Z), (-X,+Y,+Z), (+X,-Y,-Z), (+X,-Y,+Z), (+X,+Y,-Z), (+X,+Y,+Z)`。
    /// 映射到 `OrientedBoundingBox.computeCorners`
    ///
    /// # 返回
    /// 八个角点数组，每个角点为 `center ± 半轴` 的一种组合，
    /// 符号按 (X, Y, Z) 位顺序排列。
    pub fn compute_corners(&self) -> [DVec3; 8] {
        let center = self.center;
        let x_axis = self.half_axes.x_axis;
        let y_axis = self.half_axes.y_axis;
        let z_axis = self.half_axes.z_axis;

        [
            center - x_axis - y_axis - z_axis,
            center - x_axis - y_axis + z_axis,
            center - x_axis + y_axis - z_axis,
            center - x_axis + y_axis + z_axis,
            center + x_axis - y_axis - z_axis,
            center + x_axis - y_axis + z_axis,
            center + x_axis + y_axis - z_axis,
            center + x_axis + y_axis + z_axis,
        ]
    }

    /// 由带向包围盒计算一个变换矩阵（`DMat4`）：
    /// 对半轴施加统一缩放 2，并加上中心作为平移。
    /// 映射到 `OrientedBoundingBox.computeTransformation`
    pub fn compute_transformation(&self) -> DMat4 {
        let rotation_scale = self.half_axes * 2.0;
        DMat4::from_cols(
            rotation_scale.x_axis.extend(0.0),
            rotation_scale.y_axis.extend(0.0),
            rotation_scale.z_axis.extend(0.0),
            self.center.extend(1.0),
        )
    }
}

/// 为椭球在某个点处构建切平面标架 `(origin, x_axis, y_axis, z_axis)`，
/// 即原点投影到大地水准面，
/// 坐标轴取自 East-North-Up 标架。
///
/// # 参数
/// - `origin`：待投影到椭球表面的点。
/// - `ellipsoid`：参考椭球。
///
/// # 返回
/// 四元组：切平面原点与东/北/天三个单位轴（已截去齐次分量）。
fn tangent_plane_frame(origin: DVec3, ellipsoid: &Ellipsoid) -> (DVec3, DVec3, DVec3, DVec3) {
    let origin = ellipsoid
        .scale_to_geodetic_surface(origin)
        .expect("origin must not be at the center of the ellipsoid");
    let enu = crate::transforms::east_north_up_to_fixed_frame(origin, ellipsoid);
    let x_axis = enu.x_axis.truncate();
    let y_axis = enu.y_axis.truncate();
    let z_axis = enu.z_axis.truncate();
    (origin, x_axis, y_axis, z_axis)
}

/// 沿平面法线将一个 3D 点投影到切平面上，返回局部 2D 坐标。
///
/// # 参数
/// - `origin`：切平面原点。
/// - `x_axis`/`y_axis`：平面内的两个基向量。
/// - `normal`：沿其投影射线的平面法线。
/// - `cartesian`：待投影的 3D 点。
///
/// # 返回
/// 点在平面局部 (x, y) 坐标系中的坐标。
fn project_to_nearest(
    origin: DVec3,
    x_axis: DVec3,
    y_axis: DVec3,
    normal: DVec3,
    cartesian: DVec3,
) -> DVec2 {
    let plane = Plane::from_point_normal(origin, normal);
    let ray = Ray::new(cartesian, normal);
    let mut intersection = ray_plane(&ray, &plane);
    if intersection.is_none() {
        let ray = Ray::new(cartesian, -normal);
        intersection = ray_plane(&ray, &plane);
    }
    let intersection = intersection.expect("tangent plane projection must intersect");
    let v = intersection - origin;
    DVec2::new(x_axis.dot(v), y_axis.dot(v))
}

/// 由一个平面的原点/坐标轴以及局部 min/max 范围构建一个 OrientedBoundingBox。
///
/// # 参数
/// - `plane_origin`/`plane_x_axis`/`plane_y_axis`/`plane_z_axis`：盒子坐标系的原点与三轴。
/// - `minimum_x`/`maximum_x`：局部 X 方向的 extent。
/// - `minimum_y`/`maximum_y`：局部 Y 方向的 extent。
/// - `minimum_z`/`maximum_z`：局部 Z 方向的 extent。
///
/// # 返回
/// 中心与半轴由上述范围决定的 OBB。
#[allow(clippy::too_many_arguments)]
fn from_plane_extents(
    plane_origin: DVec3,
    plane_x_axis: DVec3,
    plane_y_axis: DVec3,
    plane_z_axis: DVec3,
    minimum_x: f64,
    maximum_x: f64,
    minimum_y: f64,
    maximum_y: f64,
    minimum_z: f64,
    maximum_z: f64,
) -> OrientedBoundingBox {
    let half_axes = DMat3::from_cols(plane_x_axis, plane_y_axis, plane_z_axis);

    let center_offset = DVec3::new(
        (minimum_x + maximum_x) / 2.0,
        (minimum_y + maximum_y) / 2.0,
        (minimum_z + maximum_z) / 2.0,
    );
    let scale = DVec3::new(
        (maximum_x - minimum_x) / 2.0,
        (maximum_y - minimum_y) / 2.0,
        (maximum_z - minimum_z) / 2.0,
    );

    let center = plane_origin + half_axes * center_offset;
    let half_axes = DMat3::from_cols(
        half_axes.x_axis * scale.x,
        half_axes.y_axis * scale.y,
        half_axes.z_axis * scale.z,
    );

    OrientedBoundingBox {
        center,
        half_axes,
    }
}

// --- Matrix3 特征分解（经典 Jacobi 算法）---
// 映射到 CesiumJS `Matrix3.computeEigenDecomposition`（Golub & Van Loan，第 3 版，8.4.3）
// 及其辅助函数 `computeFrobeniusNorm`、`offDiagonalFrobeniusNorm`、`shurDecomposition`。
// 扁平索引 `[col * 3 + row]` 与 CesiumJS `Matrix3.getElementIndex(col, row)` 一致。

/// 计算 3x3 矩阵的 Frobenius 范数（所有元素平方和的平方根）。
///
/// # 参数
/// - `m`：列主序扁平的 9 元素矩阵。
#[inline]
fn frobenius_norm(m: &[f64; 9]) -> f64 {
    let mut norm = 0.0;
    for i in 0..9 {
        norm += m[i] * m[i];
    }
    norm.sqrt()
}

// 非对角元素对 (col, row)：(2,1)、(2,0)、(1,0) —— 与 CesiumJS colVal/rowVal 一致。
const EIGEN_COL_VAL: [usize; 3] = [2, 2, 1];
const EIGEN_ROW_VAL: [usize; 3] = [1, 0, 0];

/// 计算 3x3 矩阵非对角元素部分的 Frobenius 范数，
/// 用于 Jacobi 迭代判断收敛（非对角项趋于 0 即对角化完成）。
///
/// # 参数
/// - `m`：列主序扁平的 3x3 对称矩阵。
///
/// # 返回
/// `sqrt(2 · Σ offdiag²)`，仅累加三对非对角元素。
#[inline]
fn off_diagonal_frobenius_norm(m: &[f64; 9]) -> f64 {
    let mut norm = 0.0;
    for i in 0..3 {
        let temp = m[EIGEN_COL_VAL[i] * 3 + EIGEN_ROW_VAL[i]];
        norm += 2.0 * temp * temp;
    }
    norm.sqrt()
}

/// 2x2 对称 Schur 分解（Golub & Van Loan 8.4.2）。返回用于削减 `matrix`
/// 中最大非对角项的 Jacobi 旋转矩阵。
///
/// # 参数
/// - `matrix`：列主序的 3x3 对称矩阵。
///
/// # 返回
/// 施加了所选 (p, q) 平面 Givens 旋转的单位矩阵（9 元素扁平）。
fn shur_decomposition(matrix: &[f64; 9]) -> [f64; 9] {
    let tolerance = math_utils::EPSILON15;

    let mut max_diagonal = 0.0;
    let mut rot_axis = 1usize;
    for i in 0..3 {
        let temp = matrix[EIGEN_COL_VAL[i] * 3 + EIGEN_ROW_VAL[i]].abs();
        if temp > max_diagonal {
            rot_axis = i;
            max_diagonal = temp;
        }
    }

    let mut c = 1.0;
    let mut s = 0.0;

    let p = EIGEN_ROW_VAL[rot_axis];
    let q = EIGEN_COL_VAL[rot_axis];

    if matrix[q * 3 + p].abs() > tolerance {
        let qq = matrix[q * 3 + q];
        let pp = matrix[p * 3 + p];
        let qp = matrix[q * 3 + p];

        let tau = (qq - pp) / 2.0 / qp;
        let t = if tau < 0.0 {
            -1.0 / (-tau + (1.0 + tau * tau).sqrt())
        } else {
            1.0 / (tau + (1.0 + tau * tau).sqrt())
        };

        c = 1.0 / (1.0 + t * t).sqrt();
        s = t * c;
    }

    // 已施加 (p, q) Givens 旋转的单位矩阵。
    let mut result = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    result[p * 3 + p] = c;
    result[q * 3 + q] = c;
    result[q * 3 + p] = s;
    result[p * 3 + q] = -s;
    result
}

/// 计算一个对称 3x3 矩阵的特征分解，返回 `(unitary, diagonal)`，
/// 使得 `matrix = unitary * diagonal * unitary^T`。
/// 映射到 `Matrix3.computeEigenDecomposition`。
///
/// # 参数
/// - `matrix`：待分解的对称 3x3 矩阵。
///
/// # 返回
/// `(unitary, diagonal)`：特征向量矩阵与对角特征值矩阵，最多 10 轮 Jacobi 扫描。
fn compute_eigen_decomposition(matrix: DMat3) -> (DMat3, DMat3) {
    let tolerance = EPSILON20;
    let max_sweeps = 10;

    let mut count = 0;
    let mut sweep = 0;

    let mut unitary = DMat3::IDENTITY;
    let mut diag = matrix;

    let epsilon = tolerance * frobenius_norm(&diag.to_cols_array());

    while sweep < max_sweeps && off_diagonal_frobenius_norm(&diag.to_cols_array()) > epsilon {
        let j_matrix = DMat3::from_cols_array(&shur_decomposition(&diag.to_cols_array()));
        let j_matrix_transpose = j_matrix.transpose();
        diag = diag * j_matrix;
        diag = j_matrix_transpose * diag;
        unitary = unitary * j_matrix;

        count += 1;
        if count > 2 {
            sweep += 1;
            count = 0;
        }
    }

    (unitary, diag)
}

/// 由最小和最大角点定义的轴对齐包围盒。
/// 映射到 CesiumJS `AxisAlignedBoundingBox`
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct AxisAlignedBoundingBox {
    /// 最小角点。
    pub minimum: DVec3,
    /// 最大角点。
    pub maximum: DVec3,
    /// 中心（计算得出）。
    pub center: DVec3,
}

impl AxisAlignedBoundingBox {
    /// 由最小/最大角点创建一个 AABB，并将中心计算为其中点。
    /// 映射到 CesiumJS 构造函数 `new AxisAlignedBoundingBox(minimum, maximum)`
    /// 以及 `AxisAlignedBoundingBox.fromCorners`。
    pub fn new(minimum: DVec3, maximum: DVec3) -> Self {
        let center = (minimum + maximum) * 0.5;
        Self {
            minimum,
            maximum,
            center,
        }
    }

    /// 由其角点创建一个 AABB。
    /// 映射到 `AxisAlignedBoundingBox.fromCorners`
    pub fn from_corners(minimum: DVec3, maximum: DVec3) -> Self {
        Self::new(minimum, maximum)
    }

    /// 创建一个带显式中心的 AABB。
    /// 映射到 CesiumJS 构造函数 `new AxisAlignedBoundingBox(minimum, maximum, center)`
    pub fn with_center(minimum: DVec3, maximum: DVec3, center: DVec3) -> Self {
        Self {
            minimum,
            maximum,
            center,
        }
    }

    /// 由一组点创建一个 AABB。
    /// 映射到 `AxisAlignedBoundingBox.fromPoints`
    pub fn from_points(points: &[DVec3]) -> Self {
        if points.is_empty() {
            return Self::new(DVec3::ZERO, DVec3::ZERO);
        }

        let mut minimum = DVec3::new(f64::MAX, f64::MAX, f64::MAX);
        let mut maximum = DVec3::new(f64::MIN, f64::MIN, f64::MIN);

        for p in points {
            minimum = minimum.min(*p);
            maximum = maximum.max(*p);
        }

        Self::new(minimum, maximum)
    }

    /// 判断一个点是否位于 AABB 内部。
    pub fn contains(&self, point: DVec3) -> bool {
        point.x >= self.minimum.x
            && point.x <= self.maximum.x
            && point.y >= self.minimum.y
            && point.y <= self.maximum.y
            && point.z >= self.minimum.z
            && point.z <= self.maximum.z
    }

    /// 计算两个 AABB 的并集。
    pub fn union(&self, other: &Self) -> Self {
        Self::new(
            self.minimum.min(other.minimum),
            self.maximum.max(other.maximum),
        )
    }

    /// 转换为一个包围球。
    pub fn to_bounding_sphere(&self) -> BoundingSphere {
        let center = self.center;
        let radius = (self.maximum - self.minimum).length() * 0.5;
        BoundingSphere { center, radius }
    }

    /// 确定与一个平面的相交情况。
    pub fn intersect_plane(&self, normal: DVec3, distance: f64) -> Intersect {
        let center_dist = normal.dot(self.center) + distance;
        let half_extents = (self.maximum - self.minimum) * 0.5;
        let rad_effective =
            normal.x.abs() * half_extents.x + normal.y.abs() * half_extents.y + normal.z.abs() * half_extents.z;

        // CesiumJS 约定：平面法线朝内。
        if center_dist - rad_effective > 0.0 {
            Intersect::Inside
        } else if center_dist + rad_effective < 0.0 {
            Intersect::Outside
        } else {
            Intersect::Intersecting
        }
    }
}

/// 由一个角点、宽度和高度定义的包围矩形。
/// 映射到 CesiumJS `BoundingRectangle`
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct BoundingRectangle {
    /// 矩形的 x 坐标（左下角）。
    pub x: f64,
    /// 矩形的 y 坐标（左下角）。
    pub y: f64,
    /// 矩形的宽度。
    pub width: f64,
    /// 矩形的高度。
    pub height: f64,
}

impl BoundingRectangle {
    /// 由左下角与宽高构造一个包围矩形。
    ///
    /// # 参数
    /// - `x`/`y`：矩形左下角坐标。
    /// - `width`/`height`：矩形的宽与高。
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 计算一个包围一组 2D 点的包围矩形。
    /// 映射到 `BoundingRectangle.fromPoints`
    pub fn from_points(points: &[DVec2]) -> Self {
        if points.is_empty() {
            return Self::default();
        }

        let mut minimum_x = points[0].x;
        let mut minimum_y = points[0].y;
        let mut maximum_x = points[0].x;
        let mut maximum_y = points[0].y;

        for p in points.iter().skip(1) {
            minimum_x = minimum_x.min(p.x);
            maximum_x = maximum_x.max(p.x);
            minimum_y = minimum_y.min(p.y);
            maximum_y = maximum_y.max(p.y);
        }

        Self {
            x: minimum_x,
            y: minimum_y,
            width: maximum_x - minimum_x,
            height: maximum_y - minimum_y,
        }
    }

    /// 通过一个投影由地理矩形计算包围矩形。
    /// 映射到 `BoundingRectangle.fromRectangle`
    pub fn from_rectangle(rectangle: &Rectangle, projection: &dyn MapProjection) -> Self {
        let lower_left = projection.project(&rectangle.southwest());
        let upper_right = projection.project(&rectangle.northeast());

        Self {
            x: lower_left.x,
            y: lower_left.y,
            width: upper_right.x - lower_left.x,
            height: upper_right.y - lower_left.y,
        }
    }

    /// 计算两个包围矩形的并集。
    /// 映射到 `BoundingRectangle.union`
    pub fn union(&self, other: &Self) -> Self {
        let lower_left_x = self.x.min(other.x);
        let lower_left_y = self.y.min(other.y);
        let upper_right_x = (self.x + self.width).max(other.x + other.width);
        let upper_right_y = (self.y + self.height).max(other.y + other.height);

        Self {
            x: lower_left_x,
            y: lower_left_y,
            width: upper_right_x - lower_left_x,
            height: upper_right_y - lower_left_y,
        }
    }

    /// 扩大矩形直到它包含给定的点。
    /// 映射到 `BoundingRectangle.expand`
    pub fn expand(&self, point: DVec2) -> Self {
        let mut result = *self;

        let width = point.x - result.x;
        let height = point.y - result.y;

        if width > result.width {
            result.width = width;
        } else if width < 0.0 {
            result.width -= width;
            result.x = point.x;
        }

        if height > result.height {
            result.height = height;
        } else if height < 0.0 {
            result.height -= height;
            result.y = point.y;
        }

        result
    }

    /// 判断两个包围矩形是否相交。
    /// 映射到 `BoundingRectangle.intersect`
    pub fn intersect(&self, other: &Self) -> Intersect {
        let left_x = self.x;
        let left_y = self.y;
        let right_x = other.x;
        let right_y = other.y;

        if !(left_x > right_x + other.width
            || left_x + self.width < right_x
            || left_y + self.height < right_y
            || left_y > right_y + other.height)
        {
            Intersect::Intersecting
        } else {
            Intersect::Outside
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bounding_sphere_from_points() {
        let points = vec![
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(-1.0, 0.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
            DVec3::new(0.0, -1.0, 0.0),
        ];
        let bs = BoundingSphere::from_points(&points);
        assert!(bs.center.abs_diff_eq(DVec3::ZERO, 1e-10));
        assert!((bs.radius - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_bounding_sphere_contains() {
        let bs = BoundingSphere::new(DVec3::ZERO, 5.0);
        assert!(bs.contains(DVec3::new(3.0, 0.0, 0.0)));
        assert!(!bs.contains(DVec3::new(6.0, 0.0, 0.0)));
    }

    #[test]
    fn test_bounding_sphere_union() {
        let a = BoundingSphere::new(DVec3::ZERO, 1.0);
        let b = BoundingSphere::new(DVec3::new(3.0, 0.0, 0.0), 1.0);
        let u = a.union(&b);
        assert!((u.center.x - 1.5).abs() < 1e-10);
        assert!((u.radius - 2.5).abs() < 1e-10);
    }

    #[test]
    fn test_aabb_from_points() {
        let points = vec![
            DVec3::new(1.0, 2.0, 3.0),
            DVec3::new(-1.0, -2.0, -3.0),
            DVec3::new(0.5, 0.5, 0.5),
        ];
        let aabb = AxisAlignedBoundingBox::from_points(&points);
        assert_eq!(aabb.minimum, DVec3::new(-1.0, -2.0, -3.0));
        assert_eq!(aabb.maximum, DVec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn test_obb_distance_to() {
        let obb = OrientedBoundingBox::new(
            DVec3::ZERO,
            DMat3::from_cols(
                DVec3::new(1.0, 0.0, 0.0),
                DVec3::new(0.0, 1.0, 0.0),
                DVec3::new(0.0, 0.0, 1.0),
            ),
        );
        // 沿 x 方向位于外部的点
        let dist = obb.distance_to(DVec3::new(3.0, 0.0, 0.0));
        assert!((dist - 2.0).abs() < 1e-10);
        // 内部的点
        let dist = obb.distance_to(DVec3::new(0.5, 0.0, 0.0));
        assert!((dist - 0.0).abs() < 1e-10);
    }
}
