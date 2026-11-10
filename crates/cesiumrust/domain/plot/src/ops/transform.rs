//! 纯几何变换（计划 §8）：一个元素可以经历的顶点级编辑 —— 平移、
//! 绕主枢轴旋转、绕主枢轴缩放，以及替换单个顶点。全部在经/纬度
//! 空间操作，因此与视图模式无关（rotate / scale 是计划允许的
//! 元素变换的 2D 平面近似；圆 / 椭圆的真实尺寸被解析地重新缩放）。
//!
//! ## 设见与约束
//! 所有变换都是纯函数：接受一个 [`Geometry`] 并返回一个变换后的新值，
//! 绝不就地修改入参（内部先 `clone` 再就地遍历顶点）。顶点遍历的顶序与
//! [`Geometry::vertices`] 严格一致，因此 [`set_vertex`] 的扁平索引与遍历结果可互相对齐。
//!
//! ## 平面近似的含义
//! 旋转与缩放直接作用于经/纬度数值，在高纬度或大尺度上会与球面真实值偏离；
//! 这是标绘编辑场景下可接受的权衡（元素通常局部且小）。参数化面（圆/椭圆）
//! 只重新锁定其中心，半径/半轴则在 [`scale`] 里按同一因子解析地缩放。

use crate::geo::GeoPoint;
use crate::model::geometry::{Geometry, PathSegment};

/// 将 `geo` 的每个存储顶点平移 `(dlon, dlat)` 度。
///
/// 平移直接累加到经/纬度上，高度保持不变；对所有几何类型都是精确的
/// （不涉及投影近似）。
pub fn translate(geo: &Geometry, dlon: f64, dlat: f64) -> Geometry {
    // 先 clone，再就地遍历每个顶点累加偏移，保证不修改入参。
    let mut out = geo.clone();
    map_vertices_in_place(&mut out, &mut |p| {
        p.lon_deg += dlon;
        p.lat_deg += dlat;
    });
    out
}

/// 将每个顶点绕 `pivot`（经/纬度）旋转 `deg` 度（在平面
/// 经/纬度坐标系中逆时针）。参数化面保持其尺寸，仅重新锁定
/// 它们的定义中心。
///
/// 标准 2D 旋转矩阵作用于相对枢轴的偏移量：先取顶点与枢轴的差，
/// 旋转后再加回枢轴。高度不受影响，枢轴自身保持不动。
pub fn rotate(geo: &Geometry, pivot: (f64, f64), deg: f64) -> Geometry {
    // 角度转弧度并预算 cos/sin，避免在每个顶点重复三角运算。
    let rad = deg.to_radians();
    let (cos, sin) = (rad.cos(), rad.sin());
    let mut out = geo.clone();
    // 逐顶点：计算相对枢轴偏移 (dx,dy)，施加旋转矩阵，再平移回枢轴。
    map_vertices_in_place(&mut out, &mut |p| {
        let dx = p.lon_deg - pivot.0;
        let dy = p.lat_deg - pivot.1;
        p.lon_deg = pivot.0 + dx * cos - dy * sin;
        p.lat_deg = pivot.1 + dx * sin + dy * cos;
    });
    out
}

/// 将每个顶点到 `pivot` 的距离按 `factor` 缩放。参数化面的
/// 半径 / 半轴按同一因子缩放。
///
/// 顶点缩放是绕枢轴的线性插值：新位置 = 枢轴 + (原位置 - 枢轴) * factor。
/// factor>1 放大、factor<1 缩小；因子为负会得到中心对称的镜像。
pub fn scale(geo: &Geometry, pivot: (f64, f64), factor: f64) -> Geometry {
    // 逐顶点按同一枢轴缩放坐标，参数化尺寸在下方单独处理。
    let mut out = geo.clone();
    map_vertices_in_place(&mut out, &mut |p| {
        p.lon_deg = pivot.0 + (p.lon_deg - pivot.0) * factor;
        p.lat_deg = pivot.1 + (p.lat_deg - pivot.1) * factor;
    });
    // 参数化面：它们的真实尺寸存在半径/半轴字段里，需按同一因子解析缩放。
    match &mut out {
        Geometry::Circle(c) => c.radius_m *= factor,
        Geometry::Ellipse(e) => {
            e.semi_major_m *= factor;
            e.semi_minor_m *= factor;
        }
        _ => {}
    }
    out
}

/// 用 `at` 替换第 `index` 个存储顶点（顺序遵循 [`Geometry::vertices`]）。
/// 当索引越界或几何无可寻址顶点时返回 `None`。
///
/// `try_take` 是一个递增计数器，保证多顶点几何（Point/Icon/Label）按同一
/// 顶序占用索引；矩形与复合体不暴露可编辑顶点，因此直接返回 `None`。
pub fn set_vertex(geo: &Geometry, index: usize, at: GeoPoint) -> Option<Geometry> {
    // 克隆后在副本上修改，保留“变换不改动入参”的约定。
    let mut out = geo.clone();
    // 一个按调用递返 0,1,2… 的计数器，用于按顶序匹配目标索引。
    let mut next = 0usize;
    let mut try_take = move || {
        let i = next;
        next += 1;
        i
    };
    match &mut out {
        // 单顶点几何：只有当计数器递到的索引等于目标时才替换。
        Geometry::Point(p) => {
            if try_take() == index {
                *p = at;
                Some(out)
            } else {
                None
            }
        }
        // 图标与标签都只有一个锥点位置，占一个顶点位。
        Geometry::Icon(ic) => {
            if try_take() == index {
                ic.at = at;
                Some(out)
            } else {
                None
            }
        }
        Geometry::Label(l) => {
            if try_take() == index {
                l.at = at;
                Some(out)
            } else {
                None
            }
        }
        Geometry::Polyline(pl) => {
            // 折线顶点存于一个数组，可直接按索引寻址。
            if index < pl.positions.len() {
                pl.positions[index] = at;
                Some(out)
            } else {
                None
            }
        }
        Geometry::Polygon(pg) => set_in_rings(&mut pg.outer, &mut pg.holes, index, at).then_some(out),
        // 矩形由四边标量描述、无显式顶点；复合体递归处理属于另一层职责：均不支持。
        Geometry::Rectangle(_) | Geometry::Composite(_) => None,
        Geometry::Circle(c) => {
            // 圆只暴露圆心作为可编辑顶点（索引 0）；半径不可通过 set_vertex 改。
            if index == 0 {
                c.center = at;
                Some(out)
            } else {
                None
            }
        }
        Geometry::Ellipse(e) => {
            if index == 0 {
                e.center = at;
                Some(out)
            } else {
                None
            }
        }
        Geometry::Arc(a) => match index {
            // 圆弧固定三个顶点：0=起点、1=圆心、2=终点。
            0 => {
                a.start = at;
                Some(out)
            }
            1 => {
                a.center = at;
                Some(out)
            }
            2 => {
                a.end = at;
                Some(out)
            }
            _ => None,
        },
        Geometry::Path(p) => {
            // 逐段扫描：每段贡献其项点数（直线=环长，弧=3），定位后在段内局部改写。
            let mut cur = 0usize;
            for seg in &mut p.segments {
                let ring_len = match seg {
                    PathSegment::Line(r) => r.len(),
                    PathSegment::Arc(_) => 3,
                };
                if index < cur + ring_len {
                    let local = index - cur;
                    match seg {
                        PathSegment::Line(r) => r[local] = at,
                        PathSegment::Arc(a) => match local {
                            0 => a.start = at,
                            1 => a.center = at,
                            _ => a.end = at,
                        },
                    }
                    return Some(out);
                }
                cur += ring_len;
            }
            None
        }
    }
}

/// 对每个存储顶点应用 `f`，顺序与 [`Geometry::vertices`]
/// 产出它们时一致。圆 / 椭圆只暴露它们的核心。
///
/// 这是 translate/rotate/scale 共用的遍历引擎：按几何变体就地递历
/// 每个可编辑顶点，确保变换顶序与索引基编辑一致。
fn map_vertices_in_place(geo: &mut Geometry, f: &mut dyn FnMut(&mut GeoPoint)) {
    match geo {
        // 单点类几何：只有一个定义顶点。
        Geometry::Point(p) => f(p),
        Geometry::Icon(i) => f(&mut i.at),
        Geometry::Label(l) => f(&mut l.at),
        Geometry::Polyline(pl) => pl.positions.iter_mut().for_each(&mut *f),
        // 多边形：先外环再逐个孔洞，与 vertices 的展平顺序一致。
        Geometry::Polygon(pg) => {
            pg.outer.iter_mut().for_each(&mut *f);
            for h in &mut pg.holes {
                h.iter_mut().for_each(&mut *f);
            }
        }
        Geometry::Rectangle(r) => {
            // 矩形以对角两点表示：展开为西南/东北两个可编辑点，变换后再写回四边。
            let mut sw = GeoPoint::surface(r.west, r.south);
            let mut en = GeoPoint::surface(r.east, r.north);
            f(&mut sw);
            f(&mut en);
            r.west = sw.lon_deg;
            r.south = sw.lat_deg;
            r.east = en.lon_deg;
            r.north = en.lat_deg;
        }
        Geometry::Circle(c) => f(&mut c.center),
        Geometry::Ellipse(e) => f(&mut e.center),
        Geometry::Arc(a) => {
            f(&mut a.start);
            f(&mut a.center);
            f(&mut a.end);
        }
        Geometry::Path(p) => {
            for seg in &mut p.segments {
                match seg {
                    PathSegment::Line(r) => r.iter_mut().for_each(&mut *f),
                    PathSegment::Arc(a) => {
                        f(&mut a.start);
                        f(&mut a.center);
                        f(&mut a.end);
                    }
                }
            }
        }
        Geometry::Composite(c) => {
            for part in &mut c.parts {
                map_vertices_in_place(part, &mut *f);
            }
        }
    }
}

/// 在一个外环 + 孔洞上设置扁平化索引的顶点。
///
/// 索引先尝试落在外环上，若越界再逐孔洞累加偏移，与 [`Geometry::vertices`]
/// 将外环与孔洞展平为单一序列的顺序保持一致；全部越界时返回 `false`。
fn set_in_rings(outer: &mut [GeoPoint], holes: &mut [Vec<GeoPoint>], index: usize, at: GeoPoint) -> bool {
    // 目标落在外环范围内：直接按索引写入。
    if index < outer.len() {
        outer[index] = at;
        return true;
    }
    let mut cur = outer.len();
    // 越过外环后逐孔洞累加偏移，直到目标索引落入当前孔洞。
    for h in holes {
        if index < cur + h.len() {
            h[index - cur] = at;
            return true;
        }
        cur += h.len();
    }
    false
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::{Arc3, Circle, Polygon, Polyline, Rectangle};

    /// 构造一个地面高度为 0 的测试用地理点（仅需经/纬度）。
    ///
    /// 包一层 `GeoPoint::surface`，避免测试里反复书写高度参数。
    fn p(lon: f64, lat: f64) -> GeoPoint {
        GeoPoint::surface(lon, lat)
    }

    /// 平移应同时作于折线的每个顶点，且保持相对形状不变。
    #[test]
    fn translate_shifts_every_vertex() {
        let g = Geometry::Polyline(Polyline {
            positions: vec![p(0.0, 0.0), p(1.0, 2.0)],
        });
        let t = translate(&g, 10.0, -5.0);
        // 取变换后的顶点列表，逐个比对累加了偏移的坐标。
        let vs = t.vertices();
        assert_eq!((vs[0].lon_deg, vs[0].lat_deg), (10.0, -5.0));
        assert_eq!((vs[1].lon_deg, vs[1].lat_deg), (11.0, -3.0));
    }

    /// 矩形平移时四个边标量应同步偏移，且变换后仍为矩形。
    #[test]
    fn rectangle_translation_moves_corners() {
        // 以四边标量定义的矩形；平移后 west/south/east/north 应各加偏移。
        let g = Geometry::Rectangle(Rectangle {
            west: 0.0,
            south: 0.0,
            east: 2.0,
            north: 3.0,
        });
        match translate(&g, 1.0, 1.0) {
            Geometry::Rectangle(r) => {
                assert_eq!((r.west, r.south, r.east, r.north), (1.0, 1.0, 3.0, 4.0))
            }
            _ => panic!("stays a rectangle"),
        }
    }

    /// 将 (1,0) 绕原点逆时针旋转 90° 应落在 (0,1)，浮点误差容忍 1e-9。
    #[test]
    fn rotate_90_about_origin() {
        let g = Geometry::Point(p(1.0, 0.0));
        match rotate(&g, (0.0, 0.0), 90.0) {
            // 90° 后理论值 (0,1)；因 cos90 非精确 0，用容差比较。
            Geometry::Point(pt) => {
                assert!(pt.lon_deg.abs() < 1e-9, "{}", pt.lon_deg);
                assert!((pt.lat_deg - 1.0).abs() < 1e-9, "{}", pt.lat_deg);
            }
            _ => unreachable!(),
        }
    }

    /// 缩放应同时作用于圆的半径与折线的顶点位置，二者共享同一因子。
    #[test]
    fn scale_moves_radius_and_vertices() {
        let g = Geometry::Circle(Circle {
            center: p(0.0, 0.0),
            radius_m: 100.0,
        });
        match scale(&g, (0.0, 0.0), 3.0) {
            // 半径 100 × 3 应为 300（解析缩放，不依赖顶点遍历）。
            Geometry::Circle(c) => assert!((c.radius_m - 300.0).abs() < 1e-9),
            _ => unreachable!(),
        }
        let line = Geometry::Polyline(Polyline {
            positions: vec![p(0.0, 0.0), p(2.0, 0.0)],
        });
        // 折线顶点按枢轴缩放：(2,0) 经因子 2 应变为 (4,0)。
        let s = scale(&line, (0.0, 0.0), 2.0);
        assert_eq!(s.vertices()[1].lon_deg, 4.0);
    }

    /// 多边形含外环与孔洞时，扁平索引应按外环→孔洞的顺序跨环寻址。
    #[test]
    fn set_vertex_hits_polygon_holes_by_flat_index() {
        // 外环 3 个顶点 + 一个单顶点孔洞：扁平索引 3 应落到孔洞。
        let g = Geometry::Polygon(Polygon {
            outer: vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)],
            holes: vec![vec![p(5.0, 5.0)]],
        });
        // 索引 3 是那个单顶点孔洞。
        // 越界索引（此处为 4）应使 set_vertex 返回 None。
        let edited = set_vertex(&g, 3, p(9.0, 9.0)).unwrap();
        match edited {
            Geometry::Polygon(pg) => assert_eq!(pg.holes[0][0], p(9.0, 9.0)),
            _ => unreachable!(),
        }
        // 越界 → None。
        assert!(set_vertex(&g, 4, p(0.0, 0.0)).is_none());
    }

    /// 圆弧以索引 0/1/2 分别对应起点/圆心/终点，验证按索引编辑圆心。
    #[test]
    fn set_vertex_arc_by_index() {
        // 构造一段圆弧，按索引 1 编辑圆心，验证仅圆心被替换。
        let g = Geometry::Arc(Arc3 {
            start: p(0.0, 0.0),
            center: p(1.0, 1.0),
            end: p(2.0, 0.0),
        });
        match set_vertex(&g, 1, p(5.0, 5.0)) {
            Some(Geometry::Arc(a)) => assert_eq!(a.center, p(5.0, 5.0)),
            other => panic!("{other:?}"),
        }
    }
}
