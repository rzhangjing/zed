//! 几何采样 / 细分（计划 §6）。
//!
//! 将参数化图元转换为 [`GeoPoint`] 顶点组成的 [`Ring`]，
//! 并将直线测地线加密为大圆弧，使一条线在地球视图中紧贴球面。
//! 一切皆为纯函数且工作在地理坐标上，因此
//! 桥接层可以用它对其他每个顶点所用的同一重投影来投影
//! 结果环 —— 而且整个过程可无头单元测试。
//!
//! 测地线数学使用半径为 [`METERS_PER_RENDER_UNIT`] 的球面地球
//! （查看器的渲染单位尺度），对于典型标绘范围下的
//! 覆盖层地面圆 / 椭圆而言精度已足够。

use std::f64::consts::TAU;

use crate::geo::{GeoPoint, METERS_PER_RENDER_UNIT};
use crate::model::geometry::{Arc3, Circle, Ellipse, Rectangle, Ring};

/// 一个完整的圆 / 椭圆被采样成的默认段数。
pub const DEFAULT_SEGMENTS: usize = 64;
/// 一个大圆弧跨步被细分成的角步长（弧度，约 1°），使一条
/// 长的地球线跟随球面而非直接割一条弦穿过。
pub const GREAT_CIRCLE_STEP_RAD: f64 = 1.0f64.to_radians();

/// 从 `start` 出发，沿 `bearing_deg`（从正北顺时针）行进 `dist_m` 米的
/// 大圆目的点。球面模型。
pub fn offset_point(start: GeoPoint, bearing_deg: f64, dist_m: f64) -> GeoPoint {
    let r = METERS_PER_RENDER_UNIT;
    let d = dist_m / r; // 角距离（弧度）
    let th = bearing_deg.to_radians();
    let lat1 = start.lat_deg.to_radians();
    let lon1 = start.lon_deg.to_radians();
    let sin_lat2 = lat1.sin() * d.cos() + lat1.cos() * d.sin() * th.cos();
    let lat2 = sin_lat2.clamp(-1.0, 1.0).asin();
    // 经度增量用 atan2 求解，避免接近极点时的奇点。
    let lon2 = lon1
        + (th.sin() * d.sin() * lat1.cos()).atan2(d.cos() - lat1.sin() * sin_lat2);
    GeoPoint::new(lon2.to_degrees(), lat2.to_degrees(), start.height_m)
}

/// 一个地面圆 → 以 `segments` 个顶点，从中心沿恒定 `radius_m`，
/// 从正北开始顺时针扫掠。
pub fn circle_ring(c: &Circle, segments: usize) -> Ring {
    let n = segments.max(3);
    // 从正北起每 360/n 度取一个大圆目的点，拼成闭合环。
    (0..n)
        .map(|i| {
            let bearing = i as f64 * 360.0 / n as f64;
            offset_point(c.center, bearing, c.radius_m)
        })
        .collect()
}

/// 一个地面椭圆 → `segments` 个顶点。长轴指向
/// `rotation_deg`（从北顺时针）；一个点的定位方式是将其
/// （沿长轴、沿短轴）偏移旋转到北/东米分量并从
/// 中心步进（一个扁地 ENU 近似，对标绘而言足够精确）。
pub fn ellipse_ring(e: &Ellipse, segments: usize) -> Ring {
    let n = segments.max(3);
    let rot = e.rotation_deg.to_radians();
    (0..n)
        .map(|i| {
            let phi = i as f64 * TAU / n as f64;
            let along_major = e.semi_major_m * phi.cos();
            let along_minor = e.semi_minor_m * phi.sin();
            // 旋转到北 / 东分量（长轴在 `rot` 相对于 N）。
            let north = along_major * rot.cos() - along_minor * rot.sin();
            let east = along_major * rot.sin() + along_minor * rot.cos();
            let p = offset_point(e.center, 0.0, north);
            offset_point(p, 90.0, east)
        })
        .collect()
}

/// 一个经纬度矩形 → 它的四个角（西-南-东-北），逆时针。
pub fn rectangle_ring(r: &Rectangle) -> Ring {
    vec![
        GeoPoint::surface(r.west, r.south),
        GeoPoint::surface(r.east, r.south),
        GeoPoint::surface(r.east, r.north),
        GeoPoint::surface(r.west, r.north),
    ]
}

/// 一个三点弧（`start` → 经由 `center` → `end`）采样为一条二次
/// 贝塞尔曲线（在经/纬度上），其控制点的选择使曲线在中点恰好
/// 穿过 `center`。端点与中点都是精确的。
pub fn arc_ring(a: &Arc3, segments: usize) -> Ring {
    let n = segments.max(2);
    // 控制点 P1 使 B(0.5) = center  ⇒  P1 = 2·center − (P0 + P2)/2。
    let ctrl = |p0: f64, c: f64, p2: f64| 2.0 * c - 0.5 * (p0 + p2);
    let lon1 = ctrl(a.start.lon_deg, a.center.lon_deg, a.end.lon_deg);
    let lat1 = ctrl(a.start.lat_deg, a.center.lat_deg, a.end.lat_deg);
    let h1 = ctrl(a.start.height_m, a.center.height_m, a.end.height_m);
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            let mt = 1.0 - t;
            let lerp = |p0: f64, pc: f64, p2: f64| {
                mt * mt * p0 + 2.0 * mt * t * pc + t * t * p2
            };
            GeoPoint::new(
                lerp(a.start.lon_deg, lon1, a.end.lon_deg),
                lerp(a.start.lat_deg, lat1, a.end.lat_deg),
                lerp(a.start.height_m, h1, a.end.height_m),
            )
        })
        .collect()
}

/// 球面上一个地理点的单位方向（用于大圆 slerp）。
fn unit_dir(p: GeoPoint) -> glam::DVec3 {
    // 球坐标：x = cosφcosλ, y = cosφsinλ, z = sinφ。
    let lat = p.lat_deg.to_radians();
    let lon = p.lon_deg.to_radians();
    glam::DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin())
}

/// 将球面单位方向反投影回经纬度点（[`unit_dir`] 的逆）；高度取传入值。
fn dir_to_geo(d: glam::DVec3, height_m: f64) -> GeoPoint {
    // 纬 = asin(z)，经 = atan2(y,x)，均转回角度。
    let lat = d.z.clamp(-1.0, 1.0).asin().to_degrees();
    let lon = d.y.atan2(d.x).to_degrees();
    GeoPoint::new(lon, lat, height_m)
}

/// 两个单位方向之间的球面线性插值。
fn slerp(a: glam::DVec3, b: glam::DVec3, t: f64) -> glam::DVec3 {
    let dot = a.dot(b).clamp(-1.0, 1.0);
    let omega = dot.acos();
    // 夹角极小时退化为 a，避免除以 sin(ω)≈ 0。
    if omega < 1e-12 {
        return a;
    }
    let s = omega.sin();
    (a * ((1.0 - t) * omega).sin() + b * (t * omega).sin()) / s
}

/// 加密一条折线，使每个大圆弧跨步被拆为至多 `step_rad`
/// （默认 [`GREAT_CIRCLE_STEP_RAD`]）的子步。端点被保留
/// 且每一对沿球面插值，使一条地球线跟随
/// 表面而非割一条弦。返回细分后的坐标列表。
pub fn subdivide_great_circle(positions: &[GeoPoint], step_rad: f64) -> Vec<GeoPoint> {
    let step = step_rad.max(1e-6);
    let mut out = Vec::new();
    for (i, w) in positions.iter().enumerate() {
        if i + 1 < positions.len() {
            let a = unit_dir(*w);
            let b = unit_dir(positions[i + 1]);
            let omega = a.dot(b).clamp(-1.0, 1.0).acos();
            // 按角跨步切成至多 step_rad 的子步；高度线性插值。
            let n = (omega / step).ceil() as usize;
            out.push(*w);
            for k in 1..n {
                let t = k as f64 / n as f64;
                let h = w.height_m * (1.0 - t) + positions[i + 1].height_m * t;
                out.push(dir_to_geo(slerp(a, b, t), h));
            }
        } else {
            out.push(*w);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn offset_due_north_increases_latitude() {
        let start = GeoPoint::surface(0.0, 0.0);
        let n100 = offset_point(start, 0.0, 1_000_000.0);
        assert!(n100.lat_deg > 8.0 && n100.lat_deg < 10.0, "{n100:?}");
        assert!(approx(n100.lon_deg, 0.0, 1e-9));
        // 正东保持赤道纬度，推进经度。
        let e100 = offset_point(start, 90.0, 1_000_000.0);
        assert!(approx(e100.lat_deg, 0.0, 1e-6), "{e100:?}");
        assert!(e100.lon_deg > 8.0);
    }

    #[test]
    fn circle_is_closed_and_equidistant() {
        let c = Circle {
            center: GeoPoint::surface(10.0, 50.0),
            radius_m: 100_000.0,
        };
        let ring = circle_ring(&c, 36);
        assert_eq!(ring.len(), 36);
        for p in &ring {
            // 每个顶点都距中心 ~radius 米。
            let d = c.center.surface_distance(*p);
            assert!((d - 100_000.0).abs() < 500.0, "{d}");
        }
    }

    #[test]
    fn rectangle_corners() {
        let r = Rectangle {
            west: -10.0,
            south: 20.0,
            east: 30.0,
            north: 40.0,
        };
        let ring = rectangle_ring(&r);
        assert_eq!(ring.len(), 4);
        assert_eq!((ring[0].lon_deg, ring[0].lat_deg), (-10.0, 20.0));
        assert_eq!((ring[2].lon_deg, ring[2].lat_deg), (30.0, 40.0));
    }

    #[test]
    fn arc_hits_endpoints_and_via() {
        let a = Arc3 {
            start: GeoPoint::surface(0.0, 0.0),
            center: GeoPoint::surface(5.0, 5.0),
            end: GeoPoint::surface(10.0, 0.0),
        };
        let ring = arc_ring(&a, 8);
        assert_eq!(ring.len(), 9); // 段数 + 1（含端点）
        assert!(approx(ring[0].lon_deg, 0.0, 1e-9));
        assert!(approx(ring[8].lon_deg, 10.0, 1e-9));
        assert!(approx(ring[4].lon_deg, 5.0, 1e-6), "midpoint via");
        assert!(approx(ring[4].lat_deg, 5.0, 1e-6));
    }

    #[test]
    fn great_circle_densifies_and_preserves_endpoints() {
        let pts = vec![GeoPoint::surface(0.0, 0.0), GeoPoint::surface(90.0, 0.0)];
        let sub = subdivide_great_circle(&pts, 10f64.to_radians());
        assert!(sub.len() > 3, "should add intermediate points");
        assert!(approx(sub.first().unwrap().lon_deg, 0.0, 1e-9));
        assert!(approx(sub.last().unwrap().lon_deg, 90.0, 1e-9));
        // 一个比步长更短的跨步保持单段（仅端点）。
        let short = subdivide_great_circle(&pts, 200f64.to_radians());
        assert_eq!(short.len(), 2);
    }
}
