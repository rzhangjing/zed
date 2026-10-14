//! Frustum 与 CullingVolume - 视锥定义与剔除。

use crate::bounding::{AxisAlignedBoundingBox, BoundingSphere};
use crate::ray::{Intersect, Plane};
use glam::{DMat4, DVec3};
use serde::{Deserialize, Serialize};

/// 可与剔除平面进行测试的包围体的 trait。
/// 映射到 CesiumJS 中鸭子类型的 `boundingVolume.intersectPlane(plane)`。
pub trait Cullable {
    /// 判断该包围体位于平面的哪一侧。
    fn cullable_intersect_plane(&self, plane: &Plane) -> Intersect;
}

impl Cullable for BoundingSphere {
    /// 球的剔除测试：委托给球与平面的相交判定。
    fn cullable_intersect_plane(&self, plane: &Plane) -> Intersect {
        self.intersect_plane(plane.normal, plane.distance)
    }
}

impl Cullable for AxisAlignedBoundingBox {
    /// 轴对齐包围盒的剔除测试：委托给盒与平面的相交判定。
    fn cullable_intersect_plane(&self, plane: &Plane) -> Intersect {
        self.intersect_plane(plane.normal, plane.distance)
    }
}

/// 由 6 个裁剪平面定义的剔除体。
/// 映射到 CesiumJS `CullingVolume`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CullingVolume {
    /// 6 个裁剪平面：左、右、下、上、近、远。
    pub planes: [Plane; 6],
}

impl CullingVolume {
    /// 对象完全在剔除体之外。
    /// 映射到 `CullingVolume.MASK_OUTSIDE`
    pub const MASK_OUTSIDE: u32 = 0xffffffff;
    /// 对象完全在剔除体之内。
    /// 映射到 `CullingVolume.MASK_INSIDE`
    pub const MASK_INSIDE: u32 = 0x00000000;
    /// 对象可能与剔除体的所有平面相交。
    /// 映射到 `CullingVolume.MASK_INDETERMINATE`
    pub const MASK_INDETERMINATE: u32 = 0x7fffffff;

    /// 由包围球构造一个剔除体。
    /// 创建六个平面构成一个包含该球体的盒子，
    /// 在世界坐标中对齐到 x、y、z 轴。
    /// 映射到 `CullingVolume.fromBoundingSphere`
    pub fn from_bounding_sphere(sphere: &BoundingSphere) -> Self {
        let center = sphere.center;
        let radius = sphere.radius;
        let faces = [DVec3::X, DVec3::Y, DVec3::Z];

        let mut planes = [Plane::ORIGIN_XY_PLANE; 6];
        let mut plane_index = 0;

        for face_normal in &faces {
            // plane0：法线 = faceNormal，过 (center - faceNormal * radius)
            let point0 = center - *face_normal * radius;
            planes[plane_index] = Plane::from_point_normal(point0, *face_normal);

            // plane1：法线 = -faceNormal，过 (center + faceNormal * radius)
            let point1 = center + *face_normal * radius;
            planes[plane_index + 1] = Plane::from_point_normal(point1, -*face_normal);

            plane_index += 2;
        }

        CullingVolume { planes }
    }

    /// 判断某个包围体相对于本剔除体的可见性。
    /// 映射到 `CullingVolume.computeVisibility`
    pub fn visibility(&self, volume: &impl Cullable) -> Intersect {
        let mut intersecting = false;

        for plane in &self.planes {
            let result = volume.cullable_intersect_plane(plane);
            if result == Intersect::Outside {
                return Intersect::Outside;
            } else if result == Intersect::Intersecting {
                intersecting = true;
            }
        }

        if intersecting {
            Intersect::Intersecting
        } else {
            Intersect::Inside
        }
    }

    /// 使用父级平面掩码计算可见性，用于层级式剔除。
    /// 映射到 `CullingVolume.computeVisibilityWithPlaneMask`
    pub fn visibility_with_plane_mask(
        &self,
        volume: &impl Cullable,
        parent_plane_mask: u32,
    ) -> u32 {
        if parent_plane_mask == Self::MASK_OUTSIDE || parent_plane_mask == Self::MASK_INSIDE {
            return parent_plane_mask;
        }

        let mut mask = Self::MASK_INSIDE;

        for (k, plane) in self.planes.iter().enumerate() {
            let flag = if k < 31 { 1u32 << k } else { 0 };
            if k < 31 && (parent_plane_mask & flag) == 0 {
                // 已知包围体位于此平面之内（INSIDE）
                continue;
            }

            let result = volume.cullable_intersect_plane(plane);
            if result == Intersect::Outside {
                return Self::MASK_OUTSIDE;
            } else if result == Intersect::Intersecting {
                mask |= flag;
            }
        }

        mask
    }
}

/// 由视野角、宽高比以及近/远平面定义的透视视锥。
/// 映射到 CesiumJS `PerspectiveFrustum`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PerspectiveFrustum {
    /// 视野角（垂直，弧度）。
    pub fov: f64,
    /// 宽高比（宽 / 高）。
    pub aspect_ratio: f64,
    /// 到近平面的距离。
    pub near: f64,
    /// 到远平面的距离。
    pub far: f64,
    /// 可选的水平视野偏移（用于偏心投影）。
    pub x_offset: f64,
    /// 可选的垂直视野偏移。
    pub y_offset: f64,
}

impl PerspectiveFrustum {
    /// 以给定视野角、宽高比、近/远平面距离构造透视视锥。
    pub fn new(fov: f64, aspect_ratio: f64, near: f64, far: f64) -> Self {
        Self {
            fov,
            aspect_ratio,
            near,
            far,
            x_offset: 0.0,
            y_offset: 0.0,
        }
    }

    /// 垂直视野角（弧度）。
    /// 映射到 `PerspectiveFrustum.fovy`
    #[inline]
    pub fn fovy(&self) -> f64 {
        self.fov
    }

    /// 水平视野角。
    pub fn fov_x(&self) -> f64 {
        2.0 * (self.fov_y_half().tan() * self.aspect_ratio).atan()
    }

    /// 垂直半视野角（弧度），即 fov 的一半。
    fn fov_y_half(&self) -> f64 {
        self.fov * 0.5
    }

    /// 计算投影矩阵。
    /// 映射到 `PerspectiveFrustum.projectionMatrix`
    pub fn projection_matrix(&self) -> DMat4 {
        let fovy_half = self.fov_y_half();
        let tan_fovy = fovy_half.tan();
        let top = self.near * tan_fovy;
        let bottom = -top;
        let right = top * self.aspect_ratio;
        let left = -right;

        // 应用偏移
        let left = left + self.x_offset * self.near;
        let right = right + self.x_offset * self.near;
        let bottom = bottom + self.y_offset * self.near;
        let top = top + self.y_offset * self.near;

        perspective_off_center(left, right, bottom, top, self.near, self.far)
    }

    /// 计算无限远平面投影矩阵（用于阴影映射）。
    /// 映射到 `PerspectiveFrustum.infiniteProjectionMatrix`
    pub fn infinite_projection_matrix(&self) -> DMat4 {
        let fovy_half = self.fov_y_half();
        let tan_fovy = fovy_half.tan();
        let top = self.near * tan_fovy;
        let bottom = -top;
        let right = top * self.aspect_ratio;
        let left = -right;

        let e = 1e-10_f64;
        DMat4::from_cols_array(&[
            2.0 * self.near / (right - left), 0.0, 0.0, 0.0,
            0.0, 2.0 * self.near / (top - bottom), 0.0, 0.0,
            (right + left) / (right - left), (top + bottom) / (top - bottom), -1.0 + e, -1.0,
            0.0, 0.0, (-2.0 + e) * self.near, 0.0,
        ])
    }

    /// 计算本视锥在给定位置/朝向下的剔除体。
    /// 映射到 `PerspectiveOffCenterFrustum.computeCullingVolume`
    pub fn compute_culling_volume(&self, position: DVec3, direction: DVec3, up: DVec3) -> CullingVolume {
        let right = direction.cross(up);

        // 计算偏心视锥参数（与 projection_matrix 相同）
        let fovy_half = self.fov_y_half();
        let tan_fovy = fovy_half.tan();
        let t = self.near * tan_fovy;
        let b = -t;
        let r = t * self.aspect_ratio;
        let l = -r;

        // 应用偏移
        let l = l + self.x_offset * self.near;
        let r = r + self.x_offset * self.near;
        let b = b + self.y_offset * self.near;
        let t = t + self.y_offset * self.near;

        let near_center = position + direction * self.near;
        let far_center = position + direction * self.far;

        // 左平面：从 position 到近平面左边缘的方向，与 up 叉积
        let left_normal = (near_center + right * l - position).cross(up).normalize();
        let left_plane = Plane::from_point_normal(position, left_normal);

        // 右平面：up 与从 position 到近平面右边缘方向的叉积
        let right_normal = up.cross(near_center + right * r - position).normalize();
        let right_plane = Plane::from_point_normal(position, right_normal);

        // 下平面：right 与从 position 到近平面下边缘方向的叉积
        let bottom_normal = right.cross(near_center + up * b - position).normalize();
        let bottom_plane = Plane::from_point_normal(position, bottom_normal);

        // 上平面：从 position 到近平面上边缘的方向，与 right 叉积
        let top_normal = (near_center + up * t - position).cross(right).normalize();
        let top_plane = Plane::from_point_normal(position, top_normal);

        // 近平面：法线沿视线方向
        let near_plane = Plane::from_point_normal(near_center, direction);

        // 远平面：法线指向视线反方向
        let far_plane = Plane::from_point_normal(far_center, -direction);

        CullingVolume {
            planes: [left_plane, right_plane, bottom_plane, top_plane, near_plane, far_plane],
        }
    }

    /// 计算给定距离处的像素尺寸。
    /// 映射到 `PerspectiveOffCenterFrustum.getPixelDimensions`
    ///
    /// # 参数
    /// * `drawing_buffer_width` - 绘制缓冲区的宽度（像素）
    /// * `drawing_buffer_height` - 绘制缓冲区的高度（像素）
    /// * `distance` - 相机到物体的距离
    /// * `pixel_ratio` - 像素比（默认 1.0）
    ///
    /// 返回 (pixel_width, pixel_height) - 在给定距离下一个像素在世界单位下的大小。
    pub fn pixel_dimensions(&self, drawing_buffer_width: f64, drawing_buffer_height: f64, distance: f64, pixel_ratio: f64) -> (f64, f64) {
        let tan_phi = self.fov_y_half().tan();
        let tan_theta = tan_phi * self.aspect_ratio;
        let pixel_width = (2.0 * pixel_ratio * distance * tan_theta) / drawing_buffer_width;
        let pixel_height = (2.0 * pixel_ratio * distance * tan_phi) / drawing_buffer_height;
        (pixel_width, pixel_height)
    }

    /// 为屏幕空间误差（SSE）计算计算 sse 分母。
    pub fn sse_denominator(&self) -> f64 {
        2.0 * self.fov_y_half().tan()
    }
}

/// 由宽度、宽高比以及近/远平面定义的正射视锥。
/// 映射到 CesiumJS `OrthographicFrustum`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OrthographicFrustum {
    /// 视锥在近平面处的宽度。
    pub width: f64,
    /// 宽高比（宽 / 高）。
    pub aspect_ratio: f64,
    /// 到近平面的距离。
    pub near: f64,
    /// 到远平面的距离。
    pub far: f64,
}

impl OrthographicFrustum {
    /// 以给定宽度、宽高比、近/远平面距离构造正射视锥。
    pub fn new(width: f64, aspect_ratio: f64, near: f64, far: f64) -> Self {
        Self {
            width,
            aspect_ratio,
            near,
            far,
        }
    }

    /// 视锥的高度。
    pub fn height(&self) -> f64 {
        self.width / self.aspect_ratio
    }

    /// 计算投影矩阵。
    /// 映射到 `OrthographicFrustum.projectionMatrix`
    pub fn projection_matrix(&self) -> DMat4 {
        let right = self.width * 0.5;
        let left = -right;
        let top = self.height() * 0.5;
        let bottom = -top;

        DMat4::from_cols_array(&[
            2.0 / (right - left), 0.0, 0.0, 0.0,
            0.0, 2.0 / (top - bottom), 0.0, 0.0,
            0.0, 0.0, -2.0 / (self.far - self.near), 0.0,
            -(right + left) / (right - left), -(top + bottom) / (top - bottom),
            -(self.far + self.near) / (self.far - self.near), 1.0,
        ])
    }

    /// 计算剔除体。
    pub fn compute_culling_volume(&self, position: DVec3, direction: DVec3, up: DVec3) -> CullingVolume {
        let right = direction.cross(up).normalize();
        let half_width = self.width * 0.5;
        let half_height = self.height() * 0.5;

        let near_center = position + direction * self.near;
        let far_center = position + direction * self.far;

        let near_plane = Plane::from_point_normal(near_center, direction);
        let far_plane = Plane::from_point_normal(far_center, -direction);
        let left_plane = Plane::from_point_normal(position - right * half_width, right);
        let right_plane = Plane::from_point_normal(position + right * half_width, -right);
        let bottom_plane = Plane::from_point_normal(position - up * half_height, up);
        let top_plane = Plane::from_point_normal(position + up * half_height, -up);

        CullingVolume {
            planes: [left_plane, right_plane, bottom_plane, top_plane, near_plane, far_plane],
        }
    }

    /// 计算给定距离处的像素尺寸。
    /// 映射到 `OrthographicOffCenterFrustum.getPixelDimensions`
    ///
    /// 返回 (pixel_width, pixel_height) - 一个像素在世界单位下的大小。
    pub fn pixel_dimensions(&self, drawing_buffer_width: f64, drawing_buffer_height: f64, _distance: f64, pixel_ratio: f64) -> (f64, f64) {
        let pixel_width = (pixel_ratio * self.width) / drawing_buffer_width;
        let pixel_height = (pixel_ratio * self.height()) / drawing_buffer_height;
        (pixel_width, pixel_height)
    }
}

// --- 偏心视锥 ---

/// 由六个裁剪平面距离（左、右、上、下、近、远）定义的透视视锥。
///
/// 这是 `PerspectiveFrustum` 所使用的底层视锥；它允许
/// 偏心（非对称）投影。`left`/`right`/`top`/`bottom` 为
/// `Option`，因为 CesiumJS 在被设置前将它们留为 `undefined`，而在它们
/// 被设置前访问投影矩阵会抛出 `DeveloperError`。
/// 映射到 CesiumJS `PerspectiveOffCenterFrustum`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PerspectiveOffCenterFrustum {
    /// 左裁剪平面距离（设置前为 `undefined`）。
    pub left: Option<f64>,
    /// 右裁剪平面距离（设置前为 `undefined`）。
    pub right: Option<f64>,
    /// 上裁剪平面距离（设置前为 `undefined`）。
    pub top: Option<f64>,
    /// 下裁剪平面距离（设置前为 `undefined`）。
    pub bottom: Option<f64>,
    /// 近平面的距离（默认 `1.0`）。
    pub near: f64,
    /// 远平面的距离（默认 `500000000.0`）。
    pub far: f64,
}

impl Default for PerspectiveOffCenterFrustum {
    /// 默认构造：四边尚未确定（None），近平面 1.0、远平面 5.0e+08。
    fn default() -> Self {
        Self {
            left: None,
            right: None,
            top: None,
            bottom: None,
            near: 1.0,
            far: 500_000_000.0,
        }
    }
}

impl PerspectiveOffCenterFrustum {
    /// 创建一个默认的（空）偏心透视视锥。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由显式边界创建一个偏心透视视锥。
    pub fn from_bounds(left: f64, right: f64, bottom: f64, top: f64, near: f64, far: f64) -> Self {
        Self {
            left: Some(left),
            right: Some(right),
            top: Some(top),
            bottom: Some(bottom),
            near,
            far,
        }
    }

    /// 解析四个侧向边界，若任一未设置则 panic
    /// （未设置即等价于抛出 `DeveloperError`）。
    fn bounds(&self) -> (f64, f64, f64, f64) {
        let left = self
            .left
            .expect("right, left, top, bottom, near, or far parameters are not set.");
        let right = self
            .right
            .expect("right, left, top, bottom, near, or far parameters are not set.");
        let top = self
            .top
            .expect("right, left, top, bottom, near, or far parameters are not set.");
        let bottom = self
            .bottom
            .expect("right, left, top, bottom, near, or far parameters are not set.");
        (left, right, bottom, top)
    }

    /// 透视投影矩阵。
    /// 映射到 `PerspectiveOffCenterFrustum.projectionMatrix`
    pub fn projection_matrix(&self) -> DMat4 {
        let (left, right, bottom, top) = self.bounds();
        perspective_off_center(left, right, bottom, top, self.near, self.far)
    }

    /// 带无限远平面的透视投影矩阵。
    /// 映射到 `PerspectiveOffCenterFrustum.infiniteProjectionMatrix`
    pub fn infinite_projection_matrix(&self) -> DMat4 {
        let (left, right, bottom, top) = self.bounds();
        infinite_perspective_off_center(left, right, bottom, top, self.near)
    }

    /// 在本视锥于给定姿态下创建一个剔除体。
    /// 映射到 `PerspectiveOffCenterFrustum.computeCullingVolume`
    pub fn compute_culling_volume(&self, position: DVec3, direction: DVec3, up: DVec3) -> CullingVolume {
        let (left, right, bottom, top) = self.bounds();
        let l = left;
        let r = right;
        let b = bottom;
        let t = top;
        let n = self.near;
        let f = self.far;

        let right_vec = direction.cross(up);
        let near_center = position + direction * n;
        let far_center = position + direction * f;

        // 左平面：normalize(nearCenter + right*l - position) x up
        let left_normal = (near_center + right_vec * l - position).cross(up).normalize();
        let left_plane = Plane::from_point_normal(position, left_normal);

        // 右平面：up x normalize(nearCenter + right*r - position)
        let right_normal = up.cross(near_center + right_vec * r - position).normalize();
        let right_plane = Plane::from_point_normal(position, right_normal);

        // 下平面：right x normalize(nearCenter + up*b - position)
        let bottom_normal = right_vec.cross(near_center + up * b - position).normalize();
        let bottom_plane = Plane::from_point_normal(position, bottom_normal);

        // 上平面：normalize(nearCenter + up*t - position) x right
        let top_normal = (near_center + up * t - position).cross(right_vec).normalize();
        let top_plane = Plane::from_point_normal(position, top_normal);

        // 近平面：法线沿视线方向，过近中心。
        let near_plane = Plane::from_point_normal(near_center, direction);

        // 远平面：法线指向视线反方向，过远中心。
        let far_plane = Plane::from_point_normal(far_center, -direction);

        CullingVolume {
            planes: [left_plane, right_plane, bottom_plane, top_plane, near_plane, far_plane],
        }
    }

    /// 返回像素的宽度和高度（单位：米）。
    /// 映射到 `PerspectiveOffCenterFrustum.getPixelDimensions`
    pub fn pixel_dimensions(
        &self,
        drawing_buffer_width: f64,
        drawing_buffer_height: f64,
        distance: f64,
        pixel_ratio: f64,
    ) -> (f64, f64) {
        let top = self
            .top
            .expect("right, left, top, bottom, near, or far parameters are not set.");
        let right = self
            .right
            .expect("right, left, top, bottom, near, or far parameters are not set.");

        let inverse_near = 1.0 / self.near;
        let tan_theta = top * inverse_near;
        let pixel_height = (2.0 * pixel_ratio * distance * tan_theta) / drawing_buffer_height;
        let tan_theta = right * inverse_near;
        let pixel_width = (2.0 * pixel_ratio * distance * tan_theta) / drawing_buffer_width;
        (pixel_width, pixel_height)
    }

    /// 逐分量相等。
    /// 映射到 `PerspectiveOffCenterFrustum.equals`
    pub fn equals(&self, other: &Self) -> bool {
        self.right == other.right
            && self.left == other.left
            && self.top == other.top
            && self.bottom == other.bottom
            && self.near == other.near
            && self.far == other.far
    }

    /// 在相对/绝对容差内的逐分量相等。
    /// 映射到 `PerspectiveOffCenterFrustum.equalsEpsilon`
    pub fn equals_epsilon(&self, other: &Self, relative_epsilon: f64, absolute_epsilon: f64) -> bool {
        crate::math_utils::equals_epsilon(self.right.unwrap_or(f64::NAN), other.right.unwrap_or(f64::NAN), relative_epsilon, absolute_epsilon)
            && crate::math_utils::equals_epsilon(self.left.unwrap_or(f64::NAN), other.left.unwrap_or(f64::NAN), relative_epsilon, absolute_epsilon)
            && crate::math_utils::equals_epsilon(self.top.unwrap_or(f64::NAN), other.top.unwrap_or(f64::NAN), relative_epsilon, absolute_epsilon)
            && crate::math_utils::equals_epsilon(self.bottom.unwrap_or(f64::NAN), other.bottom.unwrap_or(f64::NAN), relative_epsilon, absolute_epsilon)
            && crate::math_utils::equals_epsilon(self.near, other.near, relative_epsilon, absolute_epsilon)
            && crate::math_utils::equals_epsilon(self.far, other.far, relative_epsilon, absolute_epsilon)
    }
}

/// 由六个裁剪平面距离（左、右、上、下、近、远）定义的正射视锥。
/// 映射到 CesiumJS `OrthographicOffCenterFrustum`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OrthographicOffCenterFrustum {
    /// 左裁剪平面（设置前为 `undefined`）。
    pub left: Option<f64>,
    /// 右裁剪平面（设置前为 `undefined`）。
    pub right: Option<f64>,
    /// 上裁剪平面（设置前为 `undefined`）。
    pub top: Option<f64>,
    /// 下裁剪平面（设置前为 `undefined`）。
    pub bottom: Option<f64>,
    /// 近平面的距离（默认 `1.0`）。
    pub near: f64,
    /// 远平面的距离（默认 `500000000.0`）。
    pub far: f64,
}

impl Default for OrthographicOffCenterFrustum {
    /// 默认构造：四边尚未确定（None），近平面 1.0、远平面 5.0e+08。
    fn default() -> Self {
        Self {
            left: None,
            right: None,
            top: None,
            bottom: None,
            near: 1.0,
            far: 500_000_000.0,
        }
    }
}

impl OrthographicOffCenterFrustum {
    /// 创建一个默认的（空）偏心正射视锥。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由显式边界创建一个偏心正射视锥。
    pub fn from_bounds(left: f64, right: f64, bottom: f64, top: f64, near: f64, far: f64) -> Self {
        Self {
            left: Some(left),
            right: Some(right),
            top: Some(top),
            bottom: Some(bottom),
            near,
            far,
        }
    }

    /// 解析四个侧向边界，若任一未设置则 panic
    /// （未设置即等价于抛出 `DeveloperError`）。
    fn bounds(&self) -> (f64, f64, f64, f64) {
        let left = self
            .left
            .expect("right, left, top, bottom, near, or far parameters are not set.");
        let right = self
            .right
            .expect("right, left, top, bottom, near, or far parameters are not set.");
        let top = self
            .top
            .expect("right, left, top, bottom, near, or far parameters are not set.");
        let bottom = self
            .bottom
            .expect("right, left, top, bottom, near, or far parameters are not set.");
        (left, right, bottom, top)
    }

    /// 正射投影矩阵。
    /// 映射到 `OrthographicOffCenterFrustum.projectionMatrix`
    pub fn projection_matrix(&self) -> DMat4 {
        let (left, right, bottom, top) = self.bounds();
        orthographic_off_center(left, right, bottom, top, self.near, self.far)
    }

    /// 在本视锥于给定姿态下创建一个剔除体。
    /// 映射到 `OrthographicOffCenterFrustum.computeCullingVolume`
    pub fn compute_culling_volume(&self, position: DVec3, direction: DVec3, up: DVec3) -> CullingVolume {
        let (left, right, bottom, top) = self.bounds();
        let l = left;
        let r = right;
        let b = bottom;
        let t = top;
        let n = self.near;
        let f = self.far;

        // 注意：正射变体会对 right 向量归一化。
        let right_vec = direction.cross(up).normalize();
        let near_center = position + direction * n;

        // 左平面：法线 = right，过 nearCenter + right*l。
        let left_plane = Plane::from_point_normal(near_center + right_vec * l, right_vec);
        // 右平面：法线 = -right，过 nearCenter + right*r。
        let right_plane = Plane::from_point_normal(near_center + right_vec * r, -right_vec);
        // 下平面：法线 = up，过 nearCenter + up*b。
        let bottom_plane = Plane::from_point_normal(near_center + up * b, up);
        // 上平面：法线 = -up，过 nearCenter + up*t。
        let top_plane = Plane::from_point_normal(near_center + up * t, -up);
        // 近平面：法线沿视线方向，过近中心。
        let near_plane = Plane::from_point_normal(near_center, direction);
        // 远平面：法线指向视线反方向，过远中心。
        let far_plane = Plane::from_point_normal(position + direction * f, -direction);

        CullingVolume {
            planes: [left_plane, right_plane, bottom_plane, top_plane, near_plane, far_plane],
        }
    }

    /// 返回像素的宽度和高度（单位：米）。
    /// 映射到 `OrthographicOffCenterFrustum.getPixelDimensions`
    pub fn pixel_dimensions(
        &self,
        drawing_buffer_width: f64,
        drawing_buffer_height: f64,
        _distance: f64,
        pixel_ratio: f64,
    ) -> (f64, f64) {
        let (left, right, bottom, top) = self.bounds();
        let frustum_width = right - left;
        let frustum_height = top - bottom;
        let pixel_width = (pixel_ratio * frustum_width) / drawing_buffer_width;
        let pixel_height = (pixel_ratio * frustum_height) / drawing_buffer_height;
        (pixel_width, pixel_height)
    }

    /// 逐分量相等。
    /// 映射到 `OrthographicOffCenterFrustum.equals`
    pub fn equals(&self, other: &Self) -> bool {
        self.right == other.right
            && self.left == other.left
            && self.top == other.top
            && self.bottom == other.bottom
            && self.near == other.near
            && self.far == other.far
    }

    /// 在相对/绝对容差内的逐分量相等。
    /// 映射到 `OrthographicOffCenterFrustum.equalsEpsilon`
    pub fn equals_epsilon(&self, other: &Self, relative_epsilon: f64, absolute_epsilon: f64) -> bool {
        crate::math_utils::equals_epsilon(self.right.unwrap_or(f64::NAN), other.right.unwrap_or(f64::NAN), relative_epsilon, absolute_epsilon)
            && crate::math_utils::equals_epsilon(self.left.unwrap_or(f64::NAN), other.left.unwrap_or(f64::NAN), relative_epsilon, absolute_epsilon)
            && crate::math_utils::equals_epsilon(self.top.unwrap_or(f64::NAN), other.top.unwrap_or(f64::NAN), relative_epsilon, absolute_epsilon)
            && crate::math_utils::equals_epsilon(self.bottom.unwrap_or(f64::NAN), other.bottom.unwrap_or(f64::NAN), relative_epsilon, absolute_epsilon)
            && crate::math_utils::equals_epsilon(self.near, other.near, relative_epsilon, absolute_epsilon)
            && crate::math_utils::equals_epsilon(self.far, other.far, relative_epsilon, absolute_epsilon)
    }
}

// --- 辅助函数 ---

/// 创建一个偏心透视投影矩阵。
/// 映射到 `Matrix4.computePerspectiveOffCenter`
fn perspective_off_center(left: f64, right: f64, bottom: f64, top: f64, near: f64, far: f64) -> DMat4 {
    DMat4::from_cols_array(&[
        2.0 * near / (right - left), 0.0, 0.0, 0.0,
        0.0, 2.0 * near / (top - bottom), 0.0, 0.0,
        (right + left) / (right - left), (top + bottom) / (top - bottom), -(far + near) / (far - near), -1.0,
        0.0, 0.0, -2.0 * far * near / (far - near), 0.0,
    ])
}

/// 创建一个带无限远平面的偏心透视投影矩阵。
/// 映射到 `Matrix4.computeInfinitePerspectiveOffCenter`
fn infinite_perspective_off_center(left: f64, right: f64, bottom: f64, top: f64, near: f64) -> DMat4 {
    DMat4::from_cols_array(&[
        2.0 * near / (right - left), 0.0, 0.0, 0.0,
        0.0, 2.0 * near / (top - bottom), 0.0, 0.0,
        (right + left) / (right - left), (top + bottom) / (top - bottom), -1.0, -1.0,
        0.0, 0.0, -2.0 * near, 0.0,
    ])
}

/// 创建一个偏心正射投影矩阵。
/// 映射到 `Matrix4.computeOrthographicOffCenter`
fn orthographic_off_center(left: f64, right: f64, bottom: f64, top: f64, near: f64, far: f64) -> DMat4 {
    let mut a = 1.0 / (right - left);
    let mut b = 1.0 / (top - bottom);
    let mut c = 1.0 / (far - near);

    let tx = -(right + left) * a;
    let ty = -(top + bottom) * b;
    let tz = -(far + near) * c;
    a *= 2.0;
    b *= 2.0;
    c *= -2.0;

    DMat4::from_cols_array(&[
        a, 0.0, 0.0, 0.0,
        0.0, b, 0.0, 0.0,
        0.0, 0.0, c, 0.0,
        tx, ty, tz, 1.0,
    ])
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::math_utils;

    #[test]
    fn test_perspective_projection_matrix() {
        let frustum = PerspectiveFrustum::new(
            math_utils::to_radians(60.0),
            16.0 / 9.0,
            0.1,
            1000.0,
        );
        let proj = frustum.projection_matrix();
        // 检查它是一个有效的透视矩阵（右下角为 0，w 行含 -1）
        assert!((proj.w_axis.w).abs() < 1e-10);
        assert!((proj.z_axis.w - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn test_perspective_fov_x() {
        let frustum = PerspectiveFrustum::new(
            math_utils::to_radians(60.0),
            16.0 / 9.0,
            0.1,
            1000.0,
        );
        let fov_x = frustum.fov_x();
        assert!(fov_x > frustum.fov); // 更宽的宽高比 → 更宽的水平 FOV
    }

    #[test]
    fn test_culling_volume_sphere_inside() {
        let frustum = PerspectiveFrustum::new(
            math_utils::to_radians(90.0),
            1.0,
            1.0,
            100.0,
        );
        let position = DVec3::ZERO;
        let direction = DVec3::new(0.0, 0.0, -1.0);
        let up = DVec3::Y;
        let cv = frustum.compute_culling_volume(position, direction, up);

        // 位于相机前方、完全在视锥内的球
        let sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, -10.0), 1.0);
        assert_eq!(cv.visibility(&sphere), Intersect::Inside);
    }

    #[test]
    fn test_culling_volume_sphere_outside() {
        let frustum = PerspectiveFrustum::new(
            math_utils::to_radians(60.0),
            1.0,
            1.0,
            100.0,
        );
        let position = DVec3::ZERO;
        let direction = DVec3::new(0.0, 0.0, -1.0);
        let up = DVec3::Y;
        let cv = frustum.compute_culling_volume(position, direction, up);

        // 位于相机后方的球
        let sphere = BoundingSphere::new(DVec3::new(0.0, 0.0, 10.0), 1.0);
        assert_eq!(cv.visibility(&sphere), Intersect::Outside);
    }

    #[test]
    fn test_pixel_dimensions() {
        let frustum = PerspectiveFrustum::new(
            math_utils::to_radians(60.0),
            1.0,
            1.0,
            1000.0,
        );
        let (pw, ph) = frustum.pixel_dimensions(1024.0, 1024.0, 100.0, 1.0);
        assert!(pw > 0.0);
        assert!(ph > 0.0);
        assert!((pw - ph).abs() < 1e-10); // 宽高比 1.0 → 正方形像素
    }

    #[test]
    fn test_orthographic_projection() {
        let frustum = OrthographicFrustum::new(10.0, 1.0, 0.1, 100.0);
        let proj = frustum.projection_matrix();
        // 正射：w 行应为 (0, 0, 0, 1)
        assert!((proj.x_axis.w).abs() < 1e-10);
        assert!((proj.y_axis.w).abs() < 1e-10);
        assert!((proj.z_axis.w).abs() < 1e-10);
        assert!((proj.w_axis.w - 1.0).abs() < 1e-10);
    }
}
