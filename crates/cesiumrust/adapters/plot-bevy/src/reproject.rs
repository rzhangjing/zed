//! 由所有桥接视图系统共享的纯地理 → 世界投影数学。
//!
//! 这是计划中 `GeoSurface` 抽象（§3）的具体一半：给定*当前激活*
//! 相机的度量，它将一个 [`GeoPoint`] 转为渲染空间位置，并回答一个恒定像素
//! 尺寸图元所需的逐深度缩放。这里的一切都是普通数字的自由函数——无 ECS、
//! 无 `Query`——因此整个 2D/3D 投影 + 缩放契约在 spawn 第一个
//! 实体之前就可 headless 单测（计划 §15）。
//!
//! 两个坐标系（计划 §3）：
//!  * [`ViewMode::Globe`] — 渲染单位下的 WGS84 ECEF（单位扁球体，
//!    Z 向上），通过 [`geo_to_globe`]；一个透视相机环绕它，因此一个固定
//!    屏幕尺寸需要一个*依赖深度*的世界尺寸（`dist / focal`）。
//!  * [`ViewMode::Flat`] — 等矩形世界单位（`x = lon_rad`、
//!    `y = lat_rad`），通过 [`geo_to_flat`]；一个正交俯视相机使每像素世界数
//!    成为一个常量（`1 / pixels_per_world`）。

use bevy::math::Vec3;
use cesium_plot::geo::{geo_to_flat, geo_to_globe, GeoPoint, METERS_PER_RENDER_UNIT};
use cesium_plot::model::ViewMode;

/// 投影需要了解的关于激活相机的一切，由同步系统每帧
/// 采集一次。纯数据。
#[derive(Clone, Copy, Debug)]
pub struct ViewMetrics {
    /// 当前激活的投影模式。
    pub mode: ViewMode,
    /// 平面地图的每世界单位像素数（`Map2dCam.zoom`）；对球体忽略。
    pub pixels_per_world: f64,
    /// 以像素为单位的透视焦距，`(screen_h / 2) / tan(fov_y / 2)`；
    /// 对平面地图忽略。
    pub focal_px: f64,
    /// 激活相机在渲染单位世界空间中的位置（用于深度缩放）。
    pub cam_pos: Vec3,
}

impl ViewMetrics {
    /// 将一个地理坐标投影到当前激活的渲染空间。对平面地图，z
    /// 分量保持为 `0.0`（调用方叠加叠加层高度），对球体则设为椭球
    /// 表面。
    #[inline]
    pub fn project(&self, geo: GeoPoint) -> Vec3 {
        match self.mode {
            ViewMode::Flat => {
                let p = geo_to_flat(geo);
                Vec3::new(p.x as f32, p.y as f32, 0.0)
            }
            ViewMode::Globe => {
                let v = geo_to_globe(geo);
                Vec3::new(v.x as f32, v.y as f32, v.z as f32)
            }
        }
    }

    /// 相机到点的距离（渲染单位，球体）。平面地图是
    /// 正交的，因此它的“距离”无意义；在那里返回 `1.0`。
    #[inline]
    pub fn depth(&self, world: Vec3) -> f64 {
        match self.mode {
            ViewMode::Flat => 1.0,
            ViewMode::Globe => (self.cam_pos - world).length() as f64,
        }
    }

    /// 在给定点深度下，一个世界单位跨越的像素数。这是
    /// 主缩放度量：其倒数给出每像素世界数，并为 §10.7 缩放带提供输入。
    #[inline]
    pub fn pixels_per_world_at(&self, world: Vec3) -> f64 {
        match self.mode {
            ViewMode::Flat => self.pixels_per_world,
            ViewMode::Globe => {
                let d = self.depth(world).max(1e-6);
                self.focal_px / d
            }
        }
    }

    /// 在给定点深度下，一个屏幕像素覆盖的世界单位数——一个宽度为
    /// `size_px` 的图元必须乘以该因子，以便在任何缩放下都保持屏幕上的恒定
    /// 尺寸（计划 §2 / §15，“屏幕恒定尺寸”）。
    #[inline]
    pub fn world_per_px_at(&self, world: Vec3) -> f64 {
        let ppw = self.pixels_per_world_at(world);
        if ppw > 1e-9 {
            1.0 / ppw
        } else {
            0.0
        }
    }

    /// 环绕目标处每屏幕像素对应的地面米数——§10.7 米
    /// 带度量。平面：`1 世界单位 == 一个弧度 ≈ EARTH_RADIUS` 米。
    #[inline]
    pub fn meters_per_pixel(&self) -> f64 {
        match self.mode {
            ViewMode::Flat => {
                if self.pixels_per_world > 1e-9 {
                    METERS_PER_RENDER_UNIT / self.pixels_per_world
                } else {
                    0.0
                }
            }
            ViewMode::Globe => {
                if self.focal_px > 1e-9 {
                    // 到球心的深度是具代表性的表面。
                    let d = self.cam_pos.length() as f64;
                    (d * METERS_PER_RENDER_UNIT) / self.focal_px
                } else {
                    0.0
                }
            }
        }
    }

    /// 投影一段坐标（多段线 / 环顶点）。
    pub fn project_all(&self, pts: &[GeoPoint]) -> Vec<Vec3> {
        pts.iter().map(|p| self.project(*p)).collect()
    }
}

/// 构造一个面向相机的 billboard [`Vec3`] 缩放，使 XY 平面中的单位 quad
/// （范围 `[-0.5, 0.5]`，即宽 1 世界单位）在 `world` 深度下屏幕上量为 `size_px`。
#[inline]
pub fn billboard_scale(metrics: &ViewMetrics, world: Vec3, size_px: f64) -> Vec3 {
    let s = metrics.world_per_px_at(world) * size_px;
    Vec3::new(s as f32, s as f32, 1.0)
}

/// 在 `world` 深度下，宽度为 `width_px` 的线的半厚度（世界单位）。
#[inline]
pub fn line_half_width(metrics: &ViewMetrics, world: Vec3, width_px: f64) -> f64 {
    metrics.world_per_px_at(world) * width_px * 0.5
}

/// Globe 叠加层默认抬升（屏幕像素）：把叠加几何沿地心径向抬到地球
/// 前面若干像素，避免贴在 `r=1.0` 球面上时被地球深度缓冲遮挡（billboard
/// 被裁半、细线只剩两端）。以像素计量保证任意缩放下都稳定浮在影像之上。
pub const OVERLAY_LIFT_PX: f64 = 3.0;

/// 将叠加层世界点沿地心径向抬升 `lift_px` 个屏幕像素对应的世界量。
/// 仅 Globe 模式生效；Flat 模式原样返回（其高度由 `z` 分量单独处理）。
#[inline]
pub fn lift_globe_overlay(metrics: &ViewMetrics, world: Vec3, lift_px: f64) -> Vec3 {
    if !matches!(metrics.mode, ViewMode::Globe) {
        return world;
    }
    let lift = (metrics.world_per_px_at(world) * lift_px) as f32;
    world + world.normalize_or_zero() * lift
}

/// 一段穿过 `positions` 的屏幕恒定宽ribbon（三角形带）。每条边都垂直于
/// `(边方向, `normal`) 偏移半个宽度（在该顶点处求值），因此一条
/// `width_px` 的线无论缩放如何都保持 `width_px` 宽。`normal` 是 quad 平面
/// 法线（平面地图为 `+Z`，球体为视方向）。返回交错的 `[left, right]` 顶点
/// 加上 CCW 三角形索引。
pub fn ribbon(
    positions: &[Vec3],
    width_px_fn: &dyn Fn(usize) -> f64,
    normal: Vec3,
) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut out_pos: Vec<[f32; 3]> = Vec::with_capacity(positions.len() * 2);
    let mut idx: Vec<u32> = Vec::new();
    if positions.len() < 2 {
        return (out_pos, idx);
    }
    for i in 0..positions.len() {
        let cur = positions[i];
        // 切线：相邻段的平均，在端点处限幅。
        let mut tangent = Vec3::ZERO;
        if i > 0 {
            tangent += (cur - positions[i - 1]).normalize_or_zero();
        }
        if i + 1 < positions.len() {
            tangent += (positions[i + 1] - cur).normalize_or_zero();
        }
        let tangent = tangent.normalize_or_zero();
        // quad 平面内的垂直方向：normal × tangent。
        let side = normal.cross(tangent).normalize_or_zero();
        let half = width_px_fn(i) as f32;
        let l = cur + side * half;
        let r = cur - side * half;
        out_pos.push(l.to_array());
        out_pos.push(r.to_array());
        if i > 0 {
            let b = (i * 2) as u32;
            // quad (b-2,b-1,b,b+1) 拆分为两个 CCW 三角形。
            idx.extend_from_slice(&[b - 2, b - 1, b, b, b - 1, b + 1]);
        }
    }
    (out_pos, idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_plot::geo::globe_to_geo;

    fn flat(zoom: f64) -> ViewMetrics {
        ViewMetrics {
            mode: ViewMode::Flat,
            pixels_per_world: zoom,
            focal_px: 0.0,
            cam_pos: Vec3::ZERO,
        }
    }

    fn globe(cam: Vec3, focal: f64) -> ViewMetrics {
        ViewMetrics {
            mode: ViewMode::Globe,
            pixels_per_world: 0.0,
            focal_px: focal,
            cam_pos: cam,
        }
    }

    #[test]
    fn flat_project_is_radians_in_xy() {
        let m = flat(1.0);
        let p = m.project(GeoPoint::surface(180.0, 45.0));
        assert!((p.x - std::f32::consts::PI).abs() < 1e-5);
        assert!((p.y - std::f32::consts::FRAC_PI_4).abs() < 1e-5);
        assert_eq!(p.z, 0.0);
    }

    #[test]
    fn flat_world_per_px_is_inverse_zoom() {
        let m = flat(200.0);
        let w = m.world_per_px_at(Vec3::new(0.1, 0.1, 0.0));
        assert!((w - 1.0 / 200.0).abs() < 1e-9, "{w}");
        // 与位置无关（正交投影）
        let w2 = m.world_per_px_at(Vec3::new(3.0, -2.0, 0.0));
        assert!((w - w2).abs() < 1e-12);
    }

    #[test]
    fn globe_project_lands_on_unit_ellipsoid() {
        let m = globe(Vec3::new(3.0, 0.0, 0.0), 600.0);
        let world = m.project(GeoPoint::surface(0.0, 0.0));
        // (0°,0°) → 渲染 (1,0,0)；反变换回同一地理坐标。
        let back = globe_to_geo(world.as_dvec3()).unwrap();
        assert!((back.lon_deg - 0.0).abs() < 1e-6);
        assert!((back.lat_deg - 0.0).abs() < 1e-6);
        assert!((world.length() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn lift_globe_overlay_raises_radius_in_globe_only() {
        let m = globe(Vec3::new(3.0, 0.0, 0.0), 600.0);
        let p = m.project(GeoPoint::surface(0.0, 0.0)); // (1,0,0)
        let lifted = lift_globe_overlay(&m, p, OVERLAY_LIFT_PX);
        assert!(lifted.length() > p.length(), "globe lift must push outward");
        // 方向不变（沿地心径向）。
        assert!((lifted.normalize() - p.normalize()).length() < 1e-6);
        // Flat 模式原样返回（不抬升）。
        let f = flat(200.0);
        let fp = f.project(GeoPoint::surface(10.0, 10.0));
        assert_eq!(lift_globe_overlay(&f, fp, OVERLAY_LIFT_PX), fp);
    }

    #[test]
    fn globe_world_per_px_grows_with_depth() {
        // 同焦距、相机更远 → 固定表面点的每像素世界数更大
        // （屏幕上物体缩小），pixels_per_world 也缩小。
        let near = globe(Vec3::new(2.0, 0.0, 0.0), 600.0);
        let far = globe(Vec3::new(6.0, 0.0, 0.0), 600.0);
        let pt = Vec3::new(1.0, 0.0, 0.0);
        let dpp_near = near.world_per_px_at(pt); // 距离 1 → 1/600
        let dpp_far = far.world_per_px_at(pt); // 距离 5 → 5/600
        assert!((dpp_near - 1.0 / 600.0).abs() < 1e-9, "{dpp_near}");
        assert!((dpp_far - 5.0 / 600.0).abs() < 1e-9, "{dpp_far}");
        assert!(dpp_far > dpp_near);
        assert!(near.pixels_per_world_at(pt) > far.pixels_per_world_at(pt));
    }

    #[test]
    fn meters_per_pixel_flat_and_globe() {
        let f = flat(6378137.0); // 赤道半径缩放下 1 px == 1 米
        assert!((f.meters_per_pixel() - 1.0).abs() < 1e-6);
        // 球体：相机在 2 渲染单位处，焦距 1000 → d=2 → 2*6378137/1000。
        let g = globe(Vec3::new(2.0, 0.0, 0.0), 1000.0);
        let mpp = g.meters_per_pixel();
        assert!((mpp - 2.0 * 6378137.0 / 1000.0).abs() < 1.0, "{mpp}");
    }

    #[test]
    fn billboard_scale_matches_requested_px() {
        let m = flat(250.0);
        let s = billboard_scale(&m, Vec3::new(0.0, 0.0, 0.0), 10.0);
        // 10 px / 250 ppw == 0.04 世界单位
        assert!((s.x - 0.04).abs() < 1e-6);
        assert!((s.y - 0.04).abs() < 1e-6);
    }

    #[test]
    fn ribbon_has_two_vertices_per_point_and_two_triangles_per_quad() {
        let pts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
        ];
        let (pos, idx) = ribbon(&pts, &|_| 0.1, Vec3::Z);
        assert_eq!(pos.len(), 6); // 每顶点 2 个
        assert_eq!(idx.len(), 12); // 2 quad × 6 索引…等等，2 quad → 12
    }

    #[test]
    fn ribbon_offsets_perpendicular_to_travel() {
        // 沿 +X 行进，法线 +Z → side = Z×X = +Y，因此左顶点
        // 在中心线上方，右顶点在下方。
        let pts = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)];
        let (pos, _idx) = ribbon(&pts, &|_| 0.2, Vec3::Z);
        // 第一对 = 顶点 0：左(0) 然后右(1)
        assert!(pos[0][1] > 0.0, "left vertex is +Y: {pos:?}");
        assert!(pos[1][1] < 0.0, "right vertex is -Y: {pos:?}");
    }
}
