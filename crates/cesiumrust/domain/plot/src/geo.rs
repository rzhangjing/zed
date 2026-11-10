//! 地理坐标及其向查看器所用两个渲染空间的投影。
//!
//! [`GeoPoint`]（**度**为单位的经/纬度，**米**为单位高于 WGS84
//! 椭球体的高度）是每个标绘坐标的单一事实源。覆盖层
//! 与视图无关：它存储 `GeoPoint` 并让渲染桥接层按活动视图逐个投影：
//!  * [`geo_to_globe`] —— WGS84 椭球体上的 3D ECEF，缩放到查看器的
//!    渲染单位（单位扁椭球、Z-up、`ECEF_m / METERS_PER_RENDER_UNIT`），
//!    因此它与 3D 地球绘制的瓦片网格完全重合。
//!  * [`geo_to_flat`] —— 2D Geographic / 等距圆柱世界单位
//!    （`x = lon_rad`、`y = lat_rad`、R = 1），与 `map2d` 的投影匹配。
//!
//! 复用 `cesium-geospatial` 的 `Cartographic`/`Ellipsoid` 做 WGS84 数学，
//! 因此整个工作区只有一套大地测量实现。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

/// 每渲染单位的米数。在此重复定义（而非从
/// `cesium-bevy-render` 适配器导入）以使本核心 crate 不受渲染
/// 栈影响；必须与 `cesium_bevy_render::METERS_PER_RENDER_UNIT`
/// 以及应用的 `6378137.0` 约定保持同步。
pub const METERS_PER_RENDER_UNIT: f64 = 6_378_137.0;

/// 一个地理坐标：以度为单位的经度/纬度，以米为单位的高于
/// WGS84 椭球体的高度。`height` 被平面（2D）投影忽略。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    /// 以度为单位的经度，向东为正（按约定为 [-180, 180]，不强制）。
    pub lon_deg: f64,
    /// 以度为单位的纬度，向北为正（[-90, 90]）。
    pub lat_deg: f64,
    /// 以米为单位的高于椭球体的高度（0 == 在表面上）。
    pub height_m: f64,
}

impl GeoPoint {
    /// 由度 + 米高度构建。
    #[inline]
    pub fn new(lon_deg: f64, lat_deg: f64, height_m: f64) -> Self {
        Self {
            lon_deg,
            lat_deg,
            height_m,
        }
    }

    /// 一个表面点（height = 0）。
    #[inline]
    pub fn surface(lon_deg: f64, lat_deg: f64) -> Self {
        Self::new(lon_deg, lat_deg, 0.0)
    }

    /// 到另一点的大圆（球面）距离，以米为单位，基于半径为
    /// [`METERS_PER_RENDER_UNIT`] 的球。预留的测地线度量钩子
    /// （计划 §6）与采样自检都建立在此之上。
    #[inline]
    pub fn surface_distance(&self, other: GeoPoint) -> f64 {
        let lat1 = self.lat_deg.to_radians();
        let lat2 = other.lat_deg.to_radians();
        let dlat = lat2 - lat1;
        let dlon = (other.lon_deg - self.lon_deg).to_radians();
        let h = (dlat * 0.5).sin();
        let v = (dlon * 0.5).sin();
        let a = h * h + lat1.cos() * lat2.cos() * v * v;
        2.0 * a.sqrt().asin() * METERS_PER_RENDER_UNIT
    }
}

/// 度转弧度。
#[inline]
fn to_rad(deg: f64) -> f64 {
    deg.to_radians()
}

/// 弧度转度。
#[inline]
fn to_deg(rad: f64) -> f64 {
    rad.to_degrees()
}

/// 地理 → 以米为单位的 WGS84 ECEF。
#[inline]
pub fn geo_to_ecef_meters(p: GeoPoint) -> DVec3 {
    let c = Cartographic::from_degrees(p.lon_deg, p.lat_deg, p.height_m);
    Ellipsoid::WGS84.cartographic_to_cartesian(&c)
}

/// 地理 → 地球上查看器的渲染单位（Z-up 单位扁椭球）。
/// 这是一个 3D 标绘顶点必须占据的位置，以在同一经/纬度处与
/// 渲染的地形表面重合。
#[inline]
pub fn geo_to_globe(p: GeoPoint) -> DVec3 {
    geo_to_ecef_meters(p) / METERS_PER_RENDER_UNIT
}

/// [`geo_to_globe`] 的逆：从渲染单位的地球位置回到地理坐标。
/// 仅在地心退化情形下返回 `None`。
#[inline]
pub fn globe_to_geo(v: DVec3) -> Option<GeoPoint> {
    let meters = v * METERS_PER_RENDER_UNIT;
    Cartographic::from_cartesian(meters, &Ellipsoid::WGS84).map(|c| GeoPoint {
        lon_deg: to_deg(c.longitude),
        lat_deg: to_deg(c.latitude),
        height_m: c.height,
    })
}

/// 地理 → 2D 平面世界单位（`x = lon_rad`、`y = lat_rad`、R = 1），
/// 与 `map2d` 的等距圆柱布局匹配。经度直接取用（不
/// 回绕）；调用方可为了在反子午线附近的放置而自行回绕。
#[inline]
pub fn geo_to_flat(p: GeoPoint) -> DVec2 {
    DVec2::new(to_rad(p.lon_deg), to_rad(p.lat_deg))
}

/// [`geo_to_flat`] 的逆：从平面世界单位回到地理坐标（height 0）。
#[inline]
pub fn flat_to_geo(v: DVec2) -> GeoPoint {
    GeoPoint::surface(to_deg(v.x), to_deg(v.y))
}

/// 以度为单位的轴对齐地理包围盒。对于骑跨反子午线的盒子，`west`
/// 可超出 [-180, 180] 范围；模型保留原始跨度，仅在有需求时才
/// 归一化。高度不被跟踪（仅作 2D 裁剪代理）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoBounds {
    pub west_deg: f64,
    pub south_deg: f64,
    pub east_deg: f64,
    pub north_deg: f64,
}

impl GeoBounds {
    /// 单个点上的退化盒。
    #[inline]
    pub fn from_point(p: GeoPoint) -> Self {
        Self {
            west_deg: p.lon_deg,
            south_deg: p.lat_deg,
            east_deg: p.lon_deg,
            north_deg: p.lat_deg,
        }
    }

    /// 用作 [`GeoBounds::union`] 折叠种子的空/反演哨兵值。
    #[inline]
    pub fn empty() -> Self {
        Self {
            west_deg: f64::MAX,
            south_deg: f64::MAX,
            east_deg: f64::MIN,
            north_deg: f64::MIN,
        }
    }

    /// 当尚未折叠任何内容时为 true。
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.west_deg > self.east_deg || self.south_deg > self.north_deg
    }

    /// 同时包含两者的最小盒。与一个 `empty()` 盒求并会得到
    /// 另一个操作数，因此 `iter().fold(empty(), union)` 可用。
    #[inline]
    pub fn union(self, other: Self) -> Self {
        if other.is_empty() {
            return self;
        }
        if self.is_empty() {
            return other;
        }
        Self {
            west_deg: self.west_deg.min(other.west_deg),
            south_deg: self.south_deg.min(other.south_deg),
            east_deg: self.east_deg.max(other.east_deg),
            north_deg: self.north_deg.max(other.north_deg),
        }
    }

    /// 非空点云的包围盒（先以首个为种子，然后求并）。
    #[inline]
    pub fn from_points<'a>(pts: impl IntoIterator<Item = &'a GeoPoint>) -> Self {
        pts.into_iter()
            .fold(Self::empty(), |acc, p| acc.union(Self::from_point(*p)))
    }

    /// 经度跨度（度，0..360）。
    #[inline]
    pub fn width_deg(&self) -> f64 {
        (self.east_deg - self.west_deg).abs()
    }

    /// 纬度跨度（度，0..180）。
    #[inline]
    pub fn height_deg(&self) -> f64 {
        (self.north_deg - self.south_deg).abs()
    }

    /// `p` 是否位于盒内（每条边都含边界）。一个
    /// 空 / 反演的盒不含有任何东西。这是拾取器用来剔除一个可证明
    /// 无法触及元素的游标的宽相位测试，因此它必须与一个
    /// *保守* 的盒配套使用（见 `Geometry::bounds`）。
    #[inline]
    pub fn contains(&self, p: GeoPoint) -> bool {
        !self.is_empty()
            && p.lon_deg >= self.west_deg
            && p.lon_deg <= self.east_deg
            && p.lat_deg >= self.south_deg
            && p.lat_deg <= self.north_deg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn equator_prime_meridian_is_unit_x() {
        // (0°, 0°) → ECEF (a,0,0) → 渲染 (1,0,0)；Z-up 使北极位于 +Z。
        let v = geo_to_globe(GeoPoint::surface(0.0, 0.0));
        assert!(approx(v.x, 1.0, 1e-9), "{v}");
        assert!(approx(v.y, 0.0, 1e-9), "{v}");
        assert!(approx(v.z, 0.0, 1e-9), "{v}");
    }

    #[test]
    fn north_pole_sits_on_positive_z() {
        let v = geo_to_globe(GeoPoint::surface(0.0, 90.0));
        // 极半径 b / a ≈ 0.99664719（扁），在 +Z 上。
        assert!(approx(v.x, 0.0, 1e-9) && approx(v.y, 0.0, 1e-9), "{v}");
        assert!(approx(v.z, 0.99664719, 1e-6), "{v}");
        assert!(v.z > 0.0, "north pole must be +Z");
    }

    #[test]
    fn east_90_is_unit_y() {
        let v = geo_to_globe(GeoPoint::surface(90.0, 0.0));
        assert!(approx(v.x, 0.0, 1e-9), "{v}");
        assert!(approx(v.y, 1.0, 1e-9), "{v}");
    }

    #[test]
    fn globe_roundtrip_recovers_geo() {
        for (lon, lat) in [
            (0.0, 0.0),
            (116.4, 39.9),
            (-73.98, 40.7),
            (179.9, -89.0),
            (-179.9, 45.0),
        ] {
            let p = GeoPoint::new(lon, lat, 1234.5);
            let back = globe_to_geo(geo_to_globe(p)).expect("valid point");
            assert!(approx(back.lon_deg, lon, 1e-6), "lon {lon} → {}", back.lon_deg);
            assert!(approx(back.lat_deg, lat, 1e-6), "lat {lat} → {}", back.lat_deg);
            assert!(approx(back.height_m, 1234.5, 1e-2), "height {}", back.height_m);
        }
    }

    #[test]
    fn flat_uses_radians_and_inverts() {
        let p = GeoPoint::surface(180.0, 45.0);
        let f = geo_to_flat(p);
        assert!(approx(f.x, std::f64::consts::PI, 1e-9), "{f}");
        assert!(approx(f.y, std::f64::consts::FRAC_PI_4, 1e-9), "{f}");
        let back = flat_to_geo(f);
        assert!(approx(back.lon_deg, 180.0, 1e-9));
        assert!(approx(back.lat_deg, 45.0, 1e-9));
    }

    #[test]
    fn bounds_union_and_fold() {
        assert!(GeoBounds::empty().is_empty());
        let a = GeoBounds::from_point(GeoPoint::surface(10.0, 20.0));
        let b = GeoBounds::from_point(GeoPoint::surface(-30.0, 40.0));
        let u = a.union(b);
        assert_eq!((u.west_deg, u.east_deg, u.south_deg, u.north_deg), (-30.0, 10.0, 20.0, 40.0));
        // 折叠一个点云与成对求并结果一致且不会遗漏任何点。
        let cloud = vec![
            GeoPoint::surface(0.0, 0.0),
            GeoPoint::surface(5.0, -8.0),
            GeoPoint::surface(-2.0, 3.0),
        ];
        let cb = GeoBounds::from_points(&cloud);
        assert_eq!((cb.west_deg, cb.east_deg, cb.south_deg, cb.north_deg), (-2.0, 5.0, -8.0, 3.0));
        assert!(approx(cb.width_deg(), 7.0, 1e-12));
        assert!(approx(cb.height_deg(), 11.0, 1e-12));
        // 与空盒求并是恒等。
        assert_eq!(cb.union(GeoBounds::empty()), cb);
        // contains：含边界，且一个空盒不含有任何东西。
        assert!(cb.contains(GeoPoint::surface(-2.0, 3.0))); // 角点（west,north）
        assert!(cb.contains(GeoPoint::surface(0.0, 0.0))); // 内部
        assert!(!cb.contains(GeoPoint::surface(-2.1, 3.0))); // 西边界外紧邻
        assert!(!cb.contains(GeoPoint::surface(5.0, 3.1))); // 北边界外紧邻
        assert!(!GeoBounds::empty().contains(GeoPoint::surface(0.0, 0.0)));
    }
}
