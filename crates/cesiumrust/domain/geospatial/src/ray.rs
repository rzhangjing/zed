//! 射线、平面以及相交测试。
//!
//! 本模块定义由起点与方向构成的 [`Ray`]、由单位法线与距离构成的 [`Plane`]，
//! 以及一组射线/线段与椭球、平面、包围球、三角形、包围盒的相交测试。
//! 相交结果多以沿射线的参数距离 `t` 给出，再由 [`Ray::point_at`] 还原为空间点；
//! 平行、共面或交点位于射线后方等情形统一返回 `None`。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::manual_range_contains, clippy::doc_lazy_continuation)]
use crate::bounding::{AxisAlignedBoundingBox, BoundingSphere, OrientedBoundingBox};
use crate::ellipsoid::Ellipsoid;
use crate::math_utils::{EPSILON15, EPSILON6};
use glam::{DMat4, DVec3, DVec4};
use serde::{Deserialize, Serialize};

/// 与平面或剔除体相交测试的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Intersect {
    /// 对象完全在外部。
    Outside,
    /// 对象与边界相交。
    Intersecting,
    /// 对象完全在内部。
    Inside,
}

/// 由起点和方向定义的射线。
/// 映射到 CesiumJS `Ray`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ray {
    /// 射线的起点。
    pub origin: DVec3,
    /// 射线的方向（已归一化）。
    pub direction: DVec3,
}

impl Ray {
    /// 由起点与方向构造一条射线。
    ///
    /// # 参数
    /// - `origin`：射线起点。
    /// - `direction`：射线方向；构造时自动归一化为单位向量。
    pub fn new(origin: DVec3, direction: DVec3) -> Self {
        Self {
            origin,
            direction: direction.normalize(),
        }
    }

    /// 获取射线上参数 t 处的点。
    #[inline]
    pub fn point_at(&self, t: f64) -> DVec3 {
        self.origin + self.direction * t
    }
}

/// 由法线和到原点的距离定义的平面。
/// 平面方程为：normal · x + distance = 0
/// 映射到 CesiumJS `Plane`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Plane {
    /// 平面法线（已归一化）。
    pub normal: DVec3,
    /// 从原点到平面的最短距离。
    pub distance: f64,
}

impl Plane {
    /// 过原点的 XY 平面，法线 +Z。
    /// 映射到 `Plane.ORIGIN_XY_PLANE`
    pub const ORIGIN_XY_PLANE: Self = Self {
        normal: DVec3::Z,
        distance: 0.0,
    };
    /// 过原点的 YZ 平面，法线 +X。
    /// 映射到 `Plane.ORIGIN_YZ_PLANE`
    pub const ORIGIN_YZ_PLANE: Self = Self {
        normal: DVec3::X,
        distance: 0.0,
    };
    /// 过原点的 ZX 平面，法线 +Y。
    /// 映射到 `Plane.ORIGIN_ZX_PLANE`
    pub const ORIGIN_ZX_PLANE: Self = Self {
        normal: DVec3::Y,
        distance: 0.0,
    };

    /// 由法线和距离创建一个平面。
    ///
    /// 忠实于 CesiumJS：法线**原样**存储（不重新归一化）；
    /// 调用方必须提供单位长度的法线。仅在 debug 下进行的归一化检查
    /// 即 `DeveloperError`（在 release 构建中被剥离）。
    /// 映射到 `new Plane(normal, distance)`
    pub fn new(normal: DVec3, distance: f64) -> Self {
        debug_assert!(
            (normal.length() - 1.0).abs() <= crate::math_utils::EPSILON6,
            "normal must be normalized"
        );
        Self { normal, distance }
    }

    /// 由一个点和一个（单位长度）法线创建平面。
    /// 映射到 `Plane.fromPointNormal`
    pub fn from_point_normal(point: DVec3, normal: DVec3) -> Self {
        debug_assert!(
            (normal.length() - 1.0).abs() <= crate::math_utils::EPSILON6,
            "normal must be normalized"
        );
        let distance = -normal.dot(point);
        Self { normal, distance }
    }

    /// 由一般方程系数 `(x, y, z, w)` 创建平面，
    /// 其中 `(x, y, z)` 是单位长度的法线，`w` 是距离。
    /// 映射到 `Plane.fromCartesian4`
    pub fn from_cartesian4(coefficients: DVec4) -> Self {
        let normal = coefficients.truncate();
        debug_assert!(
            (normal.length() - 1.0).abs() <= crate::math_utils::EPSILON6,
            "normal must be normalized"
        );
        Self {
            normal,
            distance: coefficients.w,
        }
    }

    /// 计算从一点到平面的带符号距离。
    /// 映射到 `Plane.getPointDistance`
    pub fn point_distance(&self, point: DVec3) -> f64 {
        self.normal.dot(point) + self.distance
    }

    /// 将一点投影到平面上。
    /// 映射到 `Plane.projectPointOntoPlane`
    pub fn project_point_onto_plane(&self, point: DVec3) -> DVec3 {
        // 沿法线去掉点到平面的带符号距离，即得垂足。
        let dist = self.point_distance(point);
        point - self.normal * dist
    }

    /// 用给定的变换矩阵变换该平面。
    ///
    /// 做法：将“平面作为 Cartesian4”乘以变换的逆转置，
    /// 然后重新归一化为 Hessian 标准式。
    /// 映射到 `Plane.transform`
    pub fn transform(&self, transform: &DMat4) -> Self {
        let inverse_transpose = transform.inverse().transpose();
        let mut plane_as_cartesian4 =
            DVec4::new(self.normal.x, self.normal.y, self.normal.z, self.distance);
        plane_as_cartesian4 = inverse_transpose * plane_as_cartesian4;
        let transformed_normal = plane_as_cartesian4.truncate();
        plane_as_cartesian4 /= transformed_normal.length();
        Plane::from_cartesian4(plane_as_cartesian4)
    }
}

// --- 相交测试 ---
// 映射到 CesiumJS `IntersectionTests`

/// 计算射线与椭球的相交。
/// 返回沿射线的 (t0, t1) 参数，若不相交则返回 None。
pub fn ray_ellipsoid(ray: &Ray, ellipsoid: &Ellipsoid) -> Option<(f64, f64)> {
    ellipsoid.intersection(ray.origin, ray.direction)
}

/// 计算射线与平面的相交。
/// 返回交点，若平行则返回 None。
///
/// 先求方向在法线上的投影作为分母，接近零则视为射线与平面平行而不解。
/// 解出的参数 `t` 为负表示交点在射线反向延长线上，同样返回 None。
pub fn ray_plane(ray: &Ray, plane: &Plane) -> Option<DVec3> {
    let denominator = plane.normal.dot(ray.direction);
    if denominator.abs() < EPSILON15 {
        return None;
    }
    let t = -(plane.normal.dot(ray.origin) + plane.distance) / denominator;
    if t < 0.0 {
        return None;
    }
    Some(ray.point_at(t))
}

/// 计算线段与平面的相交。
/// 返回交点，若线段未穿过平面则返回 None。
///
/// 将线段视为参数 `t∈[0,1]` 的插值，仅当交点落在此区间内才返回，
/// 否则说明线段与平面平行或未跨越平面。
pub fn line_segment_plane(p0: DVec3, p1: DVec3, plane: &Plane) -> Option<DVec3> {
    let difference = p1 - p0;
    // 方向在法线上的投影接近零表示线段与平面平行。
    let n = plane.normal.dot(difference);
    if n.abs() < EPSILON6 {
        return None;
    }
    let t = -(plane.distance + plane.normal.dot(p0)) / n;
    if t < 0.0 || t > 1.0 {
        return None;
    }
    Some(p0 + difference * t)
}

/// 计算射线与包围球的相交。
/// 返回沿射线的参数距离区间 (start, stop)，
/// 若不相交则返回 None。
///
/// 将射线代入球面方程得到关于 `t` 的一元二次方程，判别式为负时无实交点；
/// 取两根后过滤掉位于射线后方（负值）的部分，起点被钳制到不小于 0。
pub fn ray_sphere(ray: &Ray, sphere: &BoundingSphere) -> Option<(f64, f64)> {
    let origin = ray.origin;
    let direction = ray.direction;
    let center = sphere.center;
    let radius_squared = sphere.radius * sphere.radius;

    let diff = origin - center;

    // 构造关于射线参数 t 的二次方程 a·t² + b·t + c = 0 的系数。
    let a = direction.dot(direction);
    let b = 2.0 * direction.dot(diff);
    let c = diff.dot(diff) - radius_squared;

    let det = b * b - 4.0 * a * c;
    // 判别式为负表示射线与球无实交点。
    if det < 0.0 {
        return None;
    }

    let (root0, root1) = if det > 0.0 {
        let denom = 1.0 / (2.0 * a);
        let disc = det.sqrt();
        let r0 = (-b + disc) * denom;
        let r1 = (-b - disc) * denom;
        if r0 < r1 { (r0, r1) } else { (r1, r0) }
    } else {
        // det == 0：重根
        let root = -b / (2.0 * a);
        if root == 0.0 {
            return None;
        }
        (root, root)
    };

    // 公共 API：过滤并钳制
    if root1 < 0.0 {
        return None;
    }
    let start = root0.max(0.0);
    Some((start, root1))
}

/// 将射线与三角形的相交计算为参数距离。
/// 返回沿射线的参数距离 `t`，或 None。
/// 当三角形位于射线后方时，结果可能为负。
///
/// 采用 Möller–Trumbore 思路：以边向量与方向的叉乘构造行列式，用重心
/// 坐标 (u, v) 判断交点是否落在三角形内；`cull_back_faces` 为真时忽略背面。
///
/// # 参数
/// - `ray`：待测射线。
/// - `p0`/`p1`/`p2`：三角形三个顶点。
/// - `cull_back_faces`：为真时剔除背面相交。
pub fn ray_triangle_parametric(
    ray: &Ray,
    p0: DVec3,
    p1: DVec3,
    p2: DVec3,
    cull_back_faces: bool,
) -> Option<f64> {
    let origin = ray.origin;
    let direction = ray.direction;

    let edge0 = p1 - p0;
    let edge1 = p2 - p0;

    let p = direction.cross(edge1);
    let det = edge0.dot(p);

    if cull_back_faces {
        // 背面剔除：行列式小于容差表示从背面射入或平行，直接排除。
        if det < EPSILON6 {
            return None;
        }

        let tvec = origin - p0;
        let u = tvec.dot(p);
        if u < 0.0 || u > det {
            return None;
        }

        let q = tvec.cross(edge0);
        let v = direction.dot(q);
        if v < 0.0 || u + v > det {
            return None;
        }

        Some(edge1.dot(q) / det)
    } else {
        if det.abs() < EPSILON6 {
            return None;
        }
        let inv_det = 1.0 / det;

        let tvec = origin - p0;
        let u = tvec.dot(p) * inv_det;
        if u < 0.0 || u > 1.0 {
            return None;
        }

        let q = tvec.cross(edge0);
        let v = direction.dot(q) * inv_det;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }

        Some(edge1.dot(q) * inv_det)
    }
}

/// 计算射线与三角形的相交（Möller–Trumbore 算法）。
/// 返回交点，或 None。
///
/// 内部复用 [`ray_triangle_parametric`] 得参数 `t`，仅当 `t≥0`（交点在射线
/// 前方）时才沿射线还原为具体交点坐标。
pub fn ray_triangle(
    ray: &Ray,
    v0: DVec3,
    v1: DVec3,
    v2: DVec3,
    cull_back_faces: bool,
) -> Option<DVec3> {
    let t = ray_triangle_parametric(ray, v0, v1, v2, cull_back_faces)?;
    if t < 0.0 {
        return None;
    }
    Some(ray.point_at(t))
}

/// 计算线段与三角形的相交。
/// 返回交点，或 None。
///
/// 以线段起点为射线原点、线段方向为射线方向，解出参数 `t` 后限定其不超过
/// 线段长度，从而只保留位于线段范围内的交点。
pub fn line_segment_triangle(
    v0: DVec3,
    v1: DVec3,
    p0: DVec3,
    p1: DVec3,
    p2: DVec3,
    cull_back_faces: bool,
) -> Option<DVec3> {
    let direction = (v1 - v0).normalize();
    let ray = Ray { origin: v0, direction };

    let t = ray_triangle_parametric(&ray, p0, p1, p2, cull_back_faces)?;
    let segment_length = (v1 - v0).length();
    if t < 0.0 || t > segment_length {
        return None;
    }
    Some(ray.point_at(t))
}

/// 三角形与平面相交的结果。
/// 包含所得三角形的位置和索引。
#[derive(Debug, Clone)]
pub struct TrianglePlaneIntersectionResult {
    pub positions: Vec<DVec3>,
    pub indices: Vec<u32>,
}

/// 计算三角形与平面的相交。
/// 返回所得子三角形的位置和索引，若不相交则返回 None。
///
/// 先统计位于平面后方的顶点数（只能为 1 或 2 才会相交）：1 个在后时拆为
/// 三个子三角形，2 个在后时拆为三个；0 或 3 个在后则三角形完整地在一侧，不相交。
pub fn triangle_plane_intersection(
    p0: DVec3,
    p1: DVec3,
    p2: DVec3,
    plane: &Plane,
) -> Option<TrianglePlaneIntersectionResult> {
    let plane_normal = plane.normal;
    let plane_d = plane.distance;
    let p0_behind = plane_normal.dot(p0) + plane_d < 0.0;
    let p1_behind = plane_normal.dot(p1) + plane_d < 0.0;
    let p2_behind = plane_normal.dot(p2) + plane_d < 0.0;

    let mut num_behind = 0u32;
    if p0_behind { num_behind += 1; }
    if p1_behind { num_behind += 1; }
    if p2_behind { num_behind += 1; }

    match num_behind {
        1 => {
            if p0_behind {
                let u1 = line_segment_plane(p0, p1, plane)?;
                let u2 = line_segment_plane(p0, p2, plane)?;
                Some(TrianglePlaneIntersectionResult {
                    positions: vec![p0, p1, p2, u1, u2],
                    indices: vec![0, 3, 4, 1, 2, 4, 1, 4, 3],
                })
            } else if p1_behind {
                let u1 = line_segment_plane(p1, p2, plane)?;
                let u2 = line_segment_plane(p1, p0, plane)?;
                Some(TrianglePlaneIntersectionResult {
                    positions: vec![p0, p1, p2, u1, u2],
                    indices: vec![1, 3, 4, 2, 0, 4, 2, 4, 3],
                })
            } else {
                // p2_behind
                let u1 = line_segment_plane(p2, p0, plane)?;
                let u2 = line_segment_plane(p2, p1, plane)?;
                Some(TrianglePlaneIntersectionResult {
                    positions: vec![p0, p1, p2, u1, u2],
                    indices: vec![2, 3, 4, 0, 1, 4, 0, 4, 3],
                })
            }
        }
        2 => {
            if !p0_behind {
                let u1 = line_segment_plane(p1, p0, plane)?;
                let u2 = line_segment_plane(p2, p0, plane)?;
                Some(TrianglePlaneIntersectionResult {
                    positions: vec![p0, p1, p2, u1, u2],
                    indices: vec![1, 2, 4, 1, 4, 3, 0, 3, 4],
                })
            } else if !p1_behind {
                let u1 = line_segment_plane(p2, p1, plane)?;
                let u2 = line_segment_plane(p0, p1, plane)?;
                Some(TrianglePlaneIntersectionResult {
                    positions: vec![p0, p1, p2, u1, u2],
                    indices: vec![2, 0, 4, 2, 4, 3, 1, 3, 4],
                })
            } else {
                // !p2_behind
                let u1 = line_segment_plane(p0, p2, plane)?;
                let u2 = line_segment_plane(p1, p2, plane)?;
                Some(TrianglePlaneIntersectionResult {
                    positions: vec![p0, p1, p2, u1, u2],
                    indices: vec![0, 1, 4, 0, 4, 3, 2, 3, 4],
                })
            }
        }
        // numBehind == 0（全部在前）或 3（全部在后）：不相交
        _ => None,
    }
}

/// 计算射线与方向包围盒的相交。
/// 返回沿射线的距离，或 None。
///
/// 先把射线原点与方向变换到 OBB 的局部轴空间，再用 slab 法逐轴裁剪
/// 单位立方体 [-1,1]³ 的参数区间，交集为空或完全在后方时返回 None。
pub fn ray_obb(ray: &Ray, obb: &OrientedBoundingBox) -> Option<f64> {
    let offset = ray.origin - obb.center;

    let u = obb.half_axes.x_axis;
    let v = obb.half_axes.y_axis;
    let w = obb.half_axes.z_axis;

    // 将射线变换到 OBB 局部空间
    // 非单位半轴需除以长度平方得到对偶基向量，才能正确投影到局部坐标。
    let inv_u = if u.length_squared() > 0.0 { u / u.length_squared() } else { DVec3::ZERO };
    let inv_v = if v.length_squared() > 0.0 { v / v.length_squared() } else { DVec3::ZERO };
    let inv_w = if w.length_squared() > 0.0 { w / w.length_squared() } else { DVec3::ZERO };

    let origin_local = DVec3::new(
        offset.dot(inv_u),
        offset.dot(inv_v),
        offset.dot(inv_w),
    );
    let dir_local = DVec3::new(
        ray.direction.dot(inv_u),
        ray.direction.dot(inv_v),
        ray.direction.dot(inv_w),
    );

    // 针对单位立方体 [-1, 1]^3 的 slab 法
    // 逐轴将相交区间与 slab 区间求交，t_min>t_max 即区间为空表示不相交。
    let mut t_min = f64::NEG_INFINITY;
    let mut t_max = f64::INFINITY;

    for i in 0..3 {
        let o = [origin_local.x, origin_local.y, origin_local.z][i];
        let d = [dir_local.x, dir_local.y, dir_local.z][i];

        if d.abs() < EPSILON15 {
            if !(-1.0..=1.0).contains(&o) {
                return None;
            }
        } else {
            let inv_d = 1.0 / d;
            let mut t1 = (-1.0 - o) * inv_d;
            let mut t2 = (1.0 - o) * inv_d;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            t_min = t_min.max(t1);
            t_max = t_max.min(t2);
            if t_min > t_max {
                return None;
            }
        }
    }

    if t_max < 0.0 {
        return None;
    }

    Some(if t_min >= 0.0 { t_min } else { t_max })
}

/// 计算射线与轴对齐包围盒的相交。
/// 返回沿射线的距离，或 None。
///
/// 同 slab 法，但坐标已对齐世界轴，无需变换；逐轴维护相交区间 [t_min, t_max]。
pub fn ray_aabb(ray: &Ray, aabb: &AxisAlignedBoundingBox) -> Option<f64> {
    let mut t_min = f64::NEG_INFINITY;
    let mut t_max = f64::INFINITY;

    for i in 0..3 {
        let o = [ray.origin.x, ray.origin.y, ray.origin.z][i];
        let d = [ray.direction.x, ray.direction.y, ray.direction.z][i];
        let min = [aabb.minimum.x, aabb.minimum.y, aabb.minimum.z][i];
        let max = [aabb.maximum.x, aabb.maximum.y, aabb.maximum.z][i];

        if d.abs() < EPSILON15 {
            if o < min || o > max {
                return None;
            }
        } else {
            let inv_d = 1.0 / d;
            let mut t1 = (min - o) * inv_d;
            let mut t2 = (max - o) * inv_d;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            t_min = t_min.max(t1);
            t_max = t_max.min(t2);
            if t_min > t_max {
                return None;
            }
        }
    }

    if t_max < 0.0 {
        return None;
    }

    Some(if t_min >= 0.0 { t_min } else { t_max })
}

// --- 2D 相交测试 ---
// 映射到 CesiumJS `Intersections2D`

/// 计算三角形内某点的重心坐标。
///
/// # 参数
/// - `point_x`/`point_y`：待求点的 2D 坐标。
/// - `x1,y1,x2,y2,x3,y3`：三角形三个顶点的 2D 坐标。
///
/// # 返回
/// 三元组 (u, v, w)，满足 u+v+w=1；点在三角形内时三者均在 [0,1]。
#[allow(clippy::too_many_arguments)]
pub fn compute_barycentric_coordinates(
    point_x: f64,
    point_y: f64,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    x3: f64,
    y3: f64,
) -> (f64, f64, f64) {
    let x1mx3 = x1 - x3;
    let x3mx2 = x3 - x2;
    let y2my3 = y2 - y3;
    let y1my3 = y1 - y3;
    let inverse_det = 1.0 / (y2my3 * x1mx3 + x3mx2 * y1my3);
    let dpx = point_x - x3;
    let dpy = point_y - y3;

    let u = (y2my3 * dpx + x3mx2 * dpy) * inverse_det;
    let v = (-y1my3 * dpx + x1mx3 * dpy) * inverse_det;
    let w = 1.0 - u - v;
    (u, v, w)
}

/// 在轴对齐阈值处分割一个 2D 三角形并返回所得多边形。
/// 返回一个扁平的 Vec<f64>，其中：
/// - 值 0、1、2 是原始顶点索引
/// - 值 -1 表示一个新的插值顶点，其后跟着 (from_idx, to_idx, ratio)
///
/// # 参数
/// - `threshold`：分割阈值。
/// - `keep_above`：为真保留阈值上方，否则保留下方。
/// - `u0`/`u1`/`u2`：三角形三顶点在分割轴上的坐标。
pub fn clip_triangle_at_axis_aligned_threshold(
    threshold: f64,
    keep_above: bool,
    u0: f64,
    u1: f64,
    u2: f64,
) -> Vec<f64> {
    let mut result: Vec<f64> = Vec::new();

    let u0_behind: bool;
    let u1_behind: bool;
    let u2_behind: bool;
    if keep_above {
        u0_behind = u0 < threshold;
        u1_behind = u1 < threshold;
        u2_behind = u2 < threshold;
    } else {
        u0_behind = u0 > threshold;
        u1_behind = u1 > threshold;
        u2_behind = u2 > threshold;
    }

    let num_behind = (u0_behind as u8) + (u1_behind as u8) + (u2_behind as u8);

    // 恰有一个顶点在后：切下一个三角形，剩四边形拆为两个；两个在后对称处理。
    if num_behind == 1 {
        if u0_behind {
            let u01_ratio = (threshold - u0) / (u1 - u0);
            let u02_ratio = (threshold - u0) / (u2 - u0);
            result.push(1.0);
            result.push(2.0);
            if u02_ratio != 1.0 {
                result.extend_from_slice(&[-1.0, 0.0, 2.0, u02_ratio]);
            }
            if u01_ratio != 1.0 {
                result.extend_from_slice(&[-1.0, 0.0, 1.0, u01_ratio]);
            }
        } else if u1_behind {
            let u12_ratio = (threshold - u1) / (u2 - u1);
            let u10_ratio = (threshold - u1) / (u0 - u1);
            result.push(2.0);
            result.push(0.0);
            if u10_ratio != 1.0 {
                result.extend_from_slice(&[-1.0, 1.0, 0.0, u10_ratio]);
            }
            if u12_ratio != 1.0 {
                result.extend_from_slice(&[-1.0, 1.0, 2.0, u12_ratio]);
            }
        } else if u2_behind {
            let u20_ratio = (threshold - u2) / (u0 - u2);
            let u21_ratio = (threshold - u2) / (u1 - u2);
            result.push(0.0);
            result.push(1.0);
            if u21_ratio != 1.0 {
                result.extend_from_slice(&[-1.0, 2.0, 1.0, u21_ratio]);
            }
            if u20_ratio != 1.0 {
                result.extend_from_slice(&[-1.0, 2.0, 0.0, u20_ratio]);
            }
        }
    } else if num_behind == 2 {
        if !u0_behind && u0 != threshold {
            let u10_ratio = (threshold - u1) / (u0 - u1);
            let u20_ratio = (threshold - u2) / (u0 - u2);
            result.push(0.0);
            result.extend_from_slice(&[-1.0, 1.0, 0.0, u10_ratio]);
            result.extend_from_slice(&[-1.0, 2.0, 0.0, u20_ratio]);
        } else if !u1_behind && u1 != threshold {
            let u21_ratio = (threshold - u2) / (u1 - u2);
            let u01_ratio = (threshold - u0) / (u1 - u0);
            result.push(1.0);
            result.extend_from_slice(&[-1.0, 2.0, 1.0, u21_ratio]);
            result.extend_from_slice(&[-1.0, 0.0, 1.0, u01_ratio]);
        } else if !u2_behind && u2 != threshold {
            let u02_ratio = (threshold - u0) / (u2 - u0);
            let u12_ratio = (threshold - u1) / (u2 - u1);
            result.push(2.0);
            result.extend_from_slice(&[-1.0, 0.0, 2.0, u02_ratio]);
            result.extend_from_slice(&[-1.0, 1.0, 2.0, u12_ratio]);
        }
    } else if num_behind != 3 {
        // 完全在阈值前方
        result.extend_from_slice(&[0.0, 1.0, 2.0]);
    }
    // 否则：完全在后 → 为空

    result
}

/// 计算两条 2D 线段的交点。
/// 若相交则返回 Some((x, y))，若平行/共线/不相交则返回 None。
///
/// # 参数
/// - `x00,y00,x01,y01`：第一条线段的两个端点。
/// - `x10,y10,x11,y11`：第二条线段的两个端点。
///
/// 用两条线段的方向行列式作分母，为零则平行；否则求出两个线性参数 ua、ub，
/// 仅当二者均落在 [0,1] 内时交点才同时位于两条线段上。
#[allow(clippy::too_many_arguments)]
pub fn compute_line_segment_line_segment_intersection(
    x00: f64,
    y00: f64,
    x01: f64,
    y01: f64,
    x10: f64,
    y10: f64,
    x11: f64,
    y11: f64,
) -> Option<(f64, f64)> {
    let numerator1_a = (x11 - x10) * (y00 - y10) - (y11 - y10) * (x00 - x10);
    let numerator1_b = (x01 - x00) * (y00 - y10) - (y01 - y00) * (x00 - x10);
    let denominator1 = (y11 - y10) * (x01 - x00) - (x11 - x10) * (y01 - y00);

    if denominator1 == 0.0 {
        return None;
    }

    let ua1 = numerator1_a / denominator1;
    let ub1 = numerator1_b / denominator1;

    if ua1 >= 0.0 && ua1 <= 1.0 && ub1 >= 0.0 && ub1 <= 1.0 {
        let x = x00 + ua1 * (x01 - x00);
        let y = y00 + ua1 * (y01 - y00);
        return Some((x, y));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 射线正向穿过 XY 平面时应在原点相交。
    fn test_ray_plane_intersection() {
        let ray = Ray::new(DVec3::new(0.0, 0.0, 5.0), DVec3::new(0.0, 0.0, -1.0));
        let plane = Plane::from_point_normal(DVec3::ZERO, DVec3::new(0.0, 0.0, 1.0));
        let hit = ray_plane(&ray, &plane).unwrap();
        assert!(hit.abs_diff_eq(DVec3::ZERO, 1e-10));
    }

    #[test]
    /// 射线与平面平行时无交点。
    fn test_ray_plane_parallel() {
        let ray = Ray::new(DVec3::new(0.0, 0.0, 5.0), DVec3::new(1.0, 0.0, 0.0));
        let plane = Plane::from_point_normal(DVec3::ZERO, DVec3::new(0.0, 0.0, 1.0));
        assert!(ray_plane(&ray, &plane).is_none());
    }

    #[test]
    /// 射线射入包围球应返回区间 (4, 6)。
    fn test_ray_sphere_hit() {
        let ray = Ray::new(DVec3::new(0.0, 0.0, 5.0), DVec3::new(0.0, 0.0, -1.0));
        let sphere = BoundingSphere::new(DVec3::ZERO, 1.0);
        let (start, stop) = ray_sphere(&ray, &sphere).unwrap();
        assert!((start - 4.0).abs() < 1e-10);
        assert!((stop - 6.0).abs() < 1e-10);
    }

    #[test]
    /// 射线偏离球心足够远时不与球相交。
    fn test_ray_sphere_miss() {
        let ray = Ray::new(DVec3::new(0.0, 5.0, 5.0), DVec3::new(0.0, 0.0, -1.0));
        let sphere = BoundingSphere::new(DVec3::ZERO, 1.0);
        assert!(ray_sphere(&ray, &sphere).is_none());
    }

    #[test]
    /// 射线射入三角形平面区域应落在 z=0 上。
    fn test_ray_triangle_hit() {
        let ray = Ray::new(DVec3::new(0.25, 0.25, 1.0), DVec3::new(0.0, 0.0, -1.0));
        let v0 = DVec3::new(0.0, 0.0, 0.0);
        let v1 = DVec3::new(1.0, 0.0, 0.0);
        let v2 = DVec3::new(0.0, 1.0, 0.0);
        let hit = ray_triangle(&ray, v0, v1, v2, false).unwrap();
        assert!((hit.z).abs() < 1e-10);
    }

    #[test]
    /// 射线避开三角形范围时不相交。
    fn test_ray_triangle_miss() {
        let ray = Ray::new(DVec3::new(2.0, 2.0, 1.0), DVec3::new(0.0, 0.0, -1.0));
        let v0 = DVec3::new(0.0, 0.0, 0.0);
        let v1 = DVec3::new(1.0, 0.0, 0.0);
        let v2 = DVec3::new(0.0, 1.0, 0.0);
        assert!(ray_triangle(&ray, v0, v1, v2, false).is_none());
    }

    #[test]
    /// 射线与轴对齐包围盒相交时应返回前沿距离 4。
    fn test_ray_aabb_hit() {
        let ray = Ray::new(DVec3::new(0.0, 0.0, 5.0), DVec3::new(0.0, 0.0, -1.0));
        let aabb = AxisAlignedBoundingBox::new(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0));
        let t = ray_aabb(&ray, &aabb).unwrap();
        assert!((t - 4.0).abs() < 1e-10);
    }

    #[test]
    /// 验证重心坐标计算 (0.5, 0.25, 0.25)。
    fn test_barycentric_coordinates() {
        let (u, v, w) = compute_barycentric_coordinates(0.25, 0.25, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0);
        assert!((u - 0.5).abs() < 1e-10);
        assert!((v - 0.25).abs() < 1e-10);
        assert!((w - 0.25).abs() < 1e-10);
    }
}
