//! 测地线量测工具（计划 §16 M9 “量测(距离/面积)” 集成点）。
//!
//! 在模型几何上的纯函数，因此距离 / 面积读数可无头单元
//! 测试；桥接层将它们绑定到一个量测工具 / 状态行。距离
//! 复用 [`GeoPoint::surface_distance`]（大圆米数）；面积使用
//! 球面梯形（Green 定理）环积分，对于编辑尺寸的多边形
//! 其误差远小于一个百分点。
//!
//! 这些有意*未* 接入渲染或历史 —— 它们是后续专用
//! 量测里程碑所生长的稳定入口点。

use crate::geo::{GeoPoint, METERS_PER_RENDER_UNIT};
use crate::model::geometry::{Circle, Geometry, Polygon, Rectangle};

/// 一条顶点链的大圆长度（米）；少于两个点时为 `0`。
pub fn path_length_m(points: &[GeoPoint]) -> f64 {
    // 逐相邻顶点对累加大圆距离（windows(2) 对 <2 点自然为空和）。
    points
        .windows(2)
        .map(|w| w[0].surface_distance(w[1]))
        .sum()
}

/// 一个闭合环的周长（最后一个顶点连回第一个），米。
pub fn ring_length_m(ring: &[GeoPoint]) -> f64 {
    if ring.len() < 2 {
        return 0.0;
    }
    let mut total = path_length_m(ring);
    // 再补上末点连回首点的闭合边，构成完整环周长。
    total += ring[ring.len() - 1].surface_distance(ring[0]);
    total
}

/// 一个线性环包围的球面面积（m²，无符号）。使用梯形
/// 线积分  `A = R² · |Σ (λ_{i+1} − λ_i)(sin φ_i + sin φ_{i+1})| / 2`。
/// 退化环（少于 3 点）面积为零。
pub fn ring_area_m2(ring: &[GeoPoint]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    // 遍历每条边（% n 实现环回绕），累加 Δλ · (sinφ_i + sinφ_{i+1})。
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        let dlon = (b.lon_deg - a.lon_deg).to_radians();
        let s = a.lat_deg.to_radians().sin() + b.lat_deg.to_radians().sin();
        sum += dlon * s;
    }
    // R²·|和|/2 得平方米；取绝对值以消除环朝向（顺 / 逆）的影响。
    (METERS_PER_RENDER_UNIT * METERS_PER_RENDER_UNIT * sum / 2.0).abs()
}

/// 多边形面积 = 外环减去其孔洞之和（m²）。
pub fn polygon_area_m2(pg: &Polygon) -> f64 {
    // 从外环面积起，逐个扣掉每个孔洞的面积，最后钳到非负。
    let mut area = ring_area_m2(&pg.outer);
    for h in &pg.holes {
        area -= ring_area_m2(h);
    }
    area.max(0.0)
}

/// 一个地面圆的面积，按平面 `π r²` 处理（m²）—— 半径是一个地面
/// 距离，因此平面形式这里就是预期的语义。
pub fn circle_area_m2(c: &Circle) -> f64 {
    std::f64::consts::PI * c.radius_m * c.radius_m
}

/// 矩形面积（m²）：宽（平均纬度上的一个大圆跨步）乘以
/// 高（一个子午线跨步）。
pub fn rectangle_area_m2(r: &Rectangle) -> f64 {
    // 宽：在平均纬度处量东西向的大圆跨步（避免两极畸变）。
    let mean_lat = (r.south + r.north) / 2.0;
    let west_edge = GeoPoint::surface(r.west, mean_lat);
    let east_edge = GeoPoint::surface(r.east, mean_lat);
    let width = west_edge.surface_distance(east_edge);
    // 高：沿西经线量南北向跨步。
    let height = GeoPoint::surface(r.west, r.south).surface_distance(GeoPoint::surface(r.west, r.north));
    width * height
}

/// 一个几何的总量测长度：折线长度、多边形 / 矩形周长、圆周长；
/// 点类类型为 `0`。
pub fn measure_length_m(g: &Geometry) -> f64 {
    match g {
        Geometry::Polyline(pl) => path_length_m(&pl.positions),
        Geometry::Polygon(pg) => {
            // 多边形总长 = 外环周长 + 各孔洞周长之和。
            let mut t = ring_length_m(&pg.outer);
            for h in &pg.holes {
                t += ring_length_m(h);
            }
            t
        }
        Geometry::Rectangle(r) => {
            let corners = [
                GeoPoint::surface(r.west, r.south),
                GeoPoint::surface(r.east, r.south),
                GeoPoint::surface(r.east, r.north),
                GeoPoint::surface(r.west, r.north),
            ];
            ring_length_m(&corners)
        }
        Geometry::Circle(c) => 2.0 * std::f64::consts::PI * c.radius_m,
        // 复合几何 = 各部分长度之和。
        Geometry::Composite(comp) => comp.parts.iter().map(measure_length_m).sum(),
        _ => 0.0,
    }
}

/// 一个几何的总量测面积；开放 / 点类类型为 `0`。
pub fn measure_area_m2(g: &Geometry) -> f64 {
    // 仅闭合类型有面积；复合按部分求和，其余（开放 / 点）为 0。
    match g {
        Geometry::Polygon(pg) => polygon_area_m2(pg),
        Geometry::Rectangle(r) => rectangle_area_m2(r),
        Geometry::Circle(c) => circle_area_m2(c),
        Geometry::Composite(comp) => comp.parts.iter().map(measure_area_m2).sum(),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::Polyline;

    /// 快捷构造一个地面点（高度 0）。
    fn p(lon: f64, lat: f64) -> GeoPoint {
        GeoPoint::surface(lon, lat)
    }

    /// 一个纬度度在赤道附近的大圆距离≈ 111 km。
    #[test]
    fn one_degree_latitude_is_about_111km() {
        let d = p(0.0, 0.0).surface_distance(p(0.0, 1.0));
        assert!((d - 111_319.0).abs() < 100.0, "got {d}");
    }

    /// 折线总长等于各相邻段之和；单点链长为 0。
    #[test]
    fn path_length_sums_segments() {
        let line = vec![p(0.0, 0.0), p(0.0, 1.0), p(0.0, 2.0)];
        let total = path_length_m(&line);
        let one = p(0.0, 0.0).surface_distance(p(0.0, 1.0));
        assert!((total - 2.0 * one).abs() < 1.0);
        assert_eq!(path_length_m(&[p(0.0, 0.0)]), 0.0);
    }

    /// 赤道附近 1°×1° 环的面积误差应小于 1%；退化环面积为 0。
    #[test]
    fn unit_square_area_near_equator() {
        // 赤道附近一个 1°×1° 的环 ≈ 1.239e10 m²。
        let ring = vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)];
        let area = ring_area_m2(&ring);
        let expect = 1.239e10;
        assert!(
            (area - expect).abs() / expect < 0.01,
            "area {area} not within 1% of {expect}"
        );
        assert_eq!(ring_area_m2(&[p(0.0, 0.0), p(1.0, 1.0)]), 0.0, "degenerate ring");
    }

    /// 多边形面积 = 外环减孔洞，且结果为正。
    #[test]
    fn polygon_area_subtracts_holes() {
        let outer = vec![p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0)];
        let hole = vec![p(0.5, 0.5), p(1.5, 0.5), p(1.5, 1.5), p(0.5, 1.5)];
        let pg = Polygon {
            outer: outer.clone(),
            holes: vec![hole.clone()],
        };
        let with_hole = polygon_area_m2(&pg);
        let net = ring_area_m2(&outer) - ring_area_m2(&hole);
        assert!((with_hole - net).abs() < 1.0);
        assert!(with_hole > 0.0);
    }

    /// 圆的面积（π r²）与周长（2π r）读数一致。
    #[test]
    fn circle_metrics() {
        let c = Circle {
            center: p(0.0, 0.0),
            radius_m: 1000.0,
        };
        assert!((circle_area_m2(&c) - std::f64::consts::PI * 1e6).abs() < 1.0);
        assert!((measure_length_m(&Geometry::Circle(c)) - 2.0 * std::f64::consts::PI * 1000.0).abs() < 1.0);
    }

    /// 按几何类型分派的量测与其部件一致：开放线无面积，矩形≈正方形。
    #[test]
    fn geometry_dispatch_matches_parts() {
        let line = Geometry::Polyline(Polyline {
            positions: vec![p(0.0, 0.0), p(0.0, 1.0)],
        });
        assert!(measure_length_m(&line) > 100_000.0);
        assert_eq!(measure_area_m2(&line), 0.0, "open line has no area");
        let rect = Geometry::Rectangle(Rectangle {
            west: 0.0,
            south: 0.0,
            east: 1.0,
            north: 1.0,
        });
        let rect_area = measure_area_m2(&rect);
        let square_area = ring_area_m2(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)]);
        assert!(
            (rect_area - square_area).abs() / square_area < 0.02,
            "rect {rect_area} vs ring {square_area}"
        );
    }
}
