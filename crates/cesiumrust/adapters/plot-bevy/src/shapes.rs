//! 几何 → 可渲染的顶点环，由视图同步与拾取器共享，以便两者
//! 看到完全相同的剖分后轮廓（计划 §6 / §7）。
//!
//! 参数化图元（矩形 / 圆 / 椭圆 / 弧 / 路径）通过纯函数
//! [`cesium_plot::geom::sample`] 采样；在球面视图下，环还会沿大圆加密度，
//! 使弯曲的边界紧贴球体而非切出一条弦。
//!
//! 本模块只负责“几何→顶点环”的纯采样，不涉及相机或 ECS，
//! 因此渲染与拾取可共享同一套剖分结果以保证两者完全一致。

use cesium_plot::geo::GeoPoint;
use cesium_plot::geom::sample::{
    arc_ring, circle_ring, ellipse_ring, rectangle_ring, subdivide_great_circle,
    DEFAULT_SEGMENTS, GREAT_CIRCLE_STEP_RAD,
};
use cesium_plot::model::geometry::{Geometry, PathSegment};
use cesium_plot::model::ViewMode;

/// 针对*填充面*类几何的闭合边界环（外环 + 洞），对非面几何返回 `None`。
/// 球面会沿大圆加密度。
///
/// # 参数
/// - `geo`：待采样的几何（面类）。
/// - `mode`：视图模式（Globe 时沿大圆加密）。
///
/// # 返回
/// `(外环, 洞环列表)`；非面或外环不足 3 点时 `None`。
pub fn face_rings(geo: &Geometry, mode: ViewMode) -> Option<(Vec<GeoPoint>, Vec<Vec<GeoPoint>>)> {
    let (outer, holes) = match geo {
        Geometry::Polygon(p) => (p.outer.clone(), p.holes.clone()),
        Geometry::Rectangle(r) => (rectangle_ring(r), Vec::new()),
        Geometry::Circle(c) => (circle_ring(c, DEFAULT_SEGMENTS), Vec::new()),
        Geometry::Ellipse(e) => (ellipse_ring(e, DEFAULT_SEGMENTS), Vec::new()),
        _ => return None,
    };
    if outer.len() < 3 {
        return None;
    }
    if matches!(mode, ViewMode::Globe) {
        let outer = subdivide_great_circle(&outer, GREAT_CIRCLE_STEP_RAD);
        let holes = holes
            .iter()
            .map(|h| subdivide_great_circle(h, GREAT_CIRCLE_STEP_RAD))
            .collect();
        Some((outer, holes))
    } else {
        Some((outer, holes))
    }
}

/// 针对*线状*几何（多段线、弧或路径）的单条开放笔触（顶点链），
/// 对点 / 图标 / 标签几何返回 `None`。球面笔触会沿大圆加密度，
/// 使长线沿着球体行进（计划 §6）。
///
/// # 参数
/// - `geo`：待采样的线状几何。
/// - `mode`：视图模式（Globe 时沿大圆加密）。
///
/// # 返回
/// 开放顶点链；非线状或不足 2 点时 `None`。
/// 返回的链与渲染、拾取共享，以保证两者剖分完全一致。
pub fn stroke_positions(geo: &Geometry, mode: ViewMode) -> Option<Vec<GeoPoint>> {
    let pts = match geo {
        Geometry::Polyline(pl) => pl.positions.clone(),
        Geometry::Arc(a) => arc_ring(a, DEFAULT_SEGMENTS),
        Geometry::Path(p) => {
            let mut v: Vec<GeoPoint> = Vec::new();
            for seg in &p.segments {
                let ring = match seg {
                    PathSegment::Line(r) => r.clone(),
                    PathSegment::Arc(a) => arc_ring(a, DEFAULT_SEGMENTS),
                };
                for (i, g) in ring.iter().enumerate() {
                    // 丢弃一个共享的连接顶点，以免相邻段重复
                    // 那个相接点。
                    if i == 0 && v.last().copied() == Some(*g) {
                        continue;
                    }
                    v.push(*g);
                }
            }
            v
        }
        _ => return None,
    };
    if pts.len() < 2 {
        return None;
    }
    if matches!(mode, ViewMode::Globe) {
        Some(subdivide_great_circle(&pts, GREAT_CIRCLE_STEP_RAD))
    } else {
        Some(pts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_plot::model::geometry::{Circle, Path, Polyline, Rectangle};

    /// 圆面应采样为 DEFAULT_SEGMENTS 个顶点的闭合外环，无洞。
    #[test]
    fn circle_becomes_a_face_ring() {
        let geo = Geometry::Circle(Circle {
            center: GeoPoint::surface(0.0, 0.0),
            radius_m: 50_000.0,
        });
        let (outer, holes) = face_rings(&geo, ViewMode::Flat).unwrap();
        assert_eq!(outer.len(), DEFAULT_SEGMENTS);
        assert!(holes.is_empty());
    }

    /// 矩形面应恰好是四角外环。
    #[test]
    fn rectangle_is_a_four_corner_face() {
        let geo = Geometry::Rectangle(Rectangle {
            west: 0.0,
            south: 0.0,
            east: 1.0,
            north: 1.0,
        });
        let (outer, _) = face_rings(&geo, ViewMode::Flat).unwrap();
        assert_eq!(outer.len(), 4);
    }

    /// 多段线应作为笔触而非面：face_rings 返回 None，stroke 返回顶点。
    #[test]
    fn polyline_is_a_stroke_not_a_face() {
        let geo = Geometry::Polyline(Polyline {
            positions: vec![GeoPoint::surface(0.0, 0.0), GeoPoint::surface(1.0, 1.0)],
        });
        assert!(face_rings(&geo, ViewMode::Flat).is_none());
        assert_eq!(stroke_positions(&geo, ViewMode::Flat).unwrap().len(), 2);
    }

    /// 路径拼接时共享的连接顶点应去重（相邻段不重复相接点）。
    #[test]
    fn path_dedups_shared_joins() {
        let geo = Geometry::Path(Path {
            segments: vec![
                PathSegment::Line(vec![
                    GeoPoint::surface(0.0, 0.0),
                    GeoPoint::surface(1.0, 0.0),
                ]),
                PathSegment::Line(vec![
                    GeoPoint::surface(1.0, 0.0),
                    GeoPoint::surface(2.0, 0.0),
                ]),
            ],
        });
        let stroke = stroke_positions(&geo, ViewMode::Flat).unwrap();
        // 相接顶点 (1,0) 只出现一次 → 共 3 个，而非 4 个。
        assert_eq!(stroke.len(), 3, "{stroke:?}");
    }

    /// Globe 模式下长线应沿大圆加密，比 Flat 多出中间顶点。
    #[test]
    fn globe_strokes_are_densified() {
        let geo = Geometry::Polyline(Polyline {
            positions: vec![GeoPoint::surface(0.0, 0.0), GeoPoint::surface(30.0, 0.0)],
        });
        let flat = stroke_positions(&geo, ViewMode::Flat).unwrap();
        let globe = stroke_positions(&geo, ViewMode::Globe).unwrap();
        assert_eq!(flat.len(), 2);
        assert!(globe.len() > 2, "great-circle adds intermediate vertices");
    }
}
