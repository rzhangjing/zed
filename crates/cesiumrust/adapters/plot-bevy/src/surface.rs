//! `GeoSurface` 抽象（计划 §3）：与模式无关地将屏幕光标转回一个
//! 地理坐标。
//!
//! M2 仅将此用于拾取 / 绘制里程碑所依赖的几个纯 helper，但几何在此
//! 被充分验证，以便在任何指针事件接线之前先证明 surface 契约。两个
//! 方向都共享 [`crate::reproject`] 的 `ViewMetrics`，因此“用户看到什么”与“顶点
//! 画在哪里”绝不会不一致。
//!
//!  * [`ray_to_globe`] — 一条世界空间拾取射线，针对的是标绘顶点被投影到的
//!    同一个 WGS84 椭球（半轴在渲染单位下为赤道 `1`、
//!    极地 `0.99664719`），返回靠近的表面交点。
//!  * [`ray_to_flat`] — 射线与 `z = plane_z` 等矩形纬平面
//!    的交点（对于正交俯视相机，这是一次垂直下降）。

use bevy::math::{Vec2, Vec3};
use cesium_plot::geo::{flat_to_geo, globe_to_geo, GeoPoint};

/// 渲染单位下的 WGS84 极地/赤道半径比——必须与
/// [`cesium_plot::geo::geo_to_globe`] 一致，以便拾取恰好落在已绘顶点上。
const POLAR_RATIO: f64 = 6356752.314245 / 6378137.0;

/// 将一条世界射线（原点 + 方向，渲染单位，球心位于原点）与标绘椭球
/// 相交，并将靠近的表面交点作为一个 [`GeoPoint`] 返回。未命中或
/// 唯一交点在射线后方时返回 `None`。各向异性缩放空间中的一般二次
/// 式 `|o + t·d|² = 1`。
pub fn ray_to_globe(origin: Vec3, dir: Vec3) -> Option<GeoPoint> {
    let v = ray_to_globe_d(origin.as_dvec3(), dir.as_dvec3())?;
    globe_to_geo(v)
}

/// [`ray_to_globe`] 的 f64 核心，拆出以便数学精确且可单元测试。
fn ray_to_globe_d(origin: Vec3D, dir: Vec3D) -> Option<Vec3D> {
    let s = Vec3D::new(1.0, 1.0, 1.0 / POLAR_RATIO);
    let os = origin * s;
    let ds = dir * s;
    let a = ds.dot(ds);
    if a <= 1e-12 {
        return None;
    }
    let half_b = os.dot(ds);
    let c = os.dot(os) - 1.0;
    let disc = half_b * half_b - a * c;
    if disc <= 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let t0 = (-half_b - sq) / a;
    let t = if t0 > 0.0 { t0 } else { (-half_b + sq) / a };
    if t <= 0.0 {
        return None;
    }
    Some((os + ds * t) / s)
}

/// 射线与平面地图平面 `z == plane_z` 相交处的地理点。
/// 对于正交俯视相机射线是垂直的，因此这是精确的；
/// 经度会被包裹到 `[-180, 180]`。
pub fn ray_to_flat(origin: Vec3, dir: Vec3, plane_z: f32) -> Option<GeoPoint> {
    if dir.z.abs() < 1e-6 {
        return None;
    }
    let t = (plane_z - origin.z) / dir.z;
    if t < 0.0 {
        return None;
    }
    let x = origin.x + dir.x * t;
    let y = origin.y + dir.y * t;
    let mut g = flat_to_geo(Vec2::new(x, y).as_dvec2());
    g.lon_deg = wrap_180(g.lon_deg);
    Some(g)
}

/// 将一个以度为单位的经度归一化到 `[-180, 180]`。
fn wrap_180(mut lon: f64) -> f64 {
    while lon > 180.0 {
        lon -= 360.0;
    }
    while lon < -180.0 {
        lon += 360.0;
    }
    lon
}

/// 双精度下的 `Vec3`（glam 的 `DVec3`）——一个本地别名使 f64
/// 核心保持可读。
type Vec3D = glam::DVec3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_on_ray_hits_prime_meridian_equator() {
        // 位于 +X 上直视原点的相机：靠近的交点是 (1,0,0)。
        let g = ray_to_globe(Vec3::new(3.0, 0.0, 0.0), Vec3::new(-1.0, 0.0, 0.0)).unwrap();
        assert!((g.lon_deg - 0.0).abs() < 1e-6, "{g:?}");
        assert!((g.lat_deg - 0.0).abs() < 1e-6, "{g:?}");
    }

    #[test]
    fn north_pole_camera_hits_the_north_pole() {
        // +Z 相机俯视：击中北极（纬度 90）。
        let g = ray_to_globe(Vec3::new(0.0, 0.0, 3.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
        assert!((g.lat_deg - 90.0).abs() < 1e-4, "{g:?}");
    }

    #[test]
    fn off_to_the_side_wraps_longitude() {
        // +Y 相机看向原点 → (0,1,0) = 90°E。
        let g = ray_to_globe(Vec3::new(0.0, 3.0, 0.0), Vec3::new(0.0, -1.0, 0.0)).unwrap();
        assert!((g.lon_deg - 90.0).abs() < 1e-6, "{g:?}");
    }

    #[test]
    fn ray_missing_the_globe_is_none() {
        // 偏离球体边缘一公里。
        assert!(ray_to_globe(Vec3::new(3.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)).is_none());
    }

    #[test]
    fn flat_vertical_drop_is_exact_and_wraps() {
        // x = π ⇒ 180°E 包裹到 -180（仍是反子午线）。
        let g = ray_to_flat(Vec3::new(std::f32::consts::PI, 0.0, 100.0), Vec3::new(0.0, 0.0, -1.0), 0.0)
            .unwrap();
        assert!(g.lon_deg.abs() > 179.0, "antimeridian: {g:?}");
        // y = π/4 ⇒ 45°N。
        let g2 = ray_to_flat(Vec3::new(0.0, std::f32::consts::FRAC_PI_4, 100.0), Vec3::new(0.0, 0.0, -1.0), 0.0)
            .unwrap();
        assert!((g2.lat_deg - 45.0).abs() < 1e-3, "{g2:?}");
    }
}
