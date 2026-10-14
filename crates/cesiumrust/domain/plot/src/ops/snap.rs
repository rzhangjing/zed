//! 坐标吸附（计划 §16 M9 “吸附”）。
//!
//! 吸附是对文档的一次**纯**、与视图无关的折叠：给定一个原始
//! 地理点（通过 `screen_to_geo` 解析出的光标），它返回一个调整后的点，
//! 要么落在规则的经纬网格上，要么卡入一个附近的现有**顶点**，
//! 要么投影到一个现有几何的附近**边**（线段）上。桥接层
//! 带着一个 [`SnapConfig`] 调用 [`snap`]，并在草稿顶点被提交前应用
//! 结果；配置默认为*禁用*，因此在用户开启之前现有行为
//! 保持不变。
//!
//! 阈值是地理意义上的：顶点 / 边距离用
//! [`GeoPoint::surface_distance`]（米）度量，网格步长以度为单位 —— 足以
//! 用于编辑，且对无头测试完全确定。

use crate::geo::GeoPoint;
use crate::model::document::Document;
use crate::model::geometry::{Geometry, PathSegment};
use crate::model::ids::ElementId;

/// 一个吸附坐标的来源（保留用于预览 / 状态显示）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapResult {
    /// 无匹配 —— 原始点原样返回。
    None(GeoPoint),
    /// 卡入另一个元素的一个现有顶点。
    Vertex(GeoPoint),
    /// 投影到另一个元素的一条边（线段）上。
    Edge(GeoPoint),
    /// 舍入到吸附网格上。
    Grid(GeoPoint),
}

impl SnapResult {
    /// （可能被调整过的）坐标。
    pub fn point(&self) -> GeoPoint {
        match self {
            SnapResult::None(p)
            | SnapResult::Vertex(p)
            | SnapResult::Edge(p)
            | SnapResult::Grid(p) => *p,
        }
    }

    /// 是否应用了任何调整。
    pub fn snapped(&self) -> bool {
        !matches!(self, SnapResult::None(_))
    }
}

/// 一次吸附遍历的调优参数。`Default` 为**关**，因此将其接入
/// 交互 FSM 永远不会改变现有绘制行为，直到显式启用。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapConfig {
    /// 总开关。为 `false` 时，[`snap`] 总是返回 [`SnapResult::None`]。
    pub enabled: bool,
    /// 在此半径（米）内吸附到现有顶点。`0` 禁用。
    pub vertex_threshold_m: f64,
    /// 在此半径（米）内吸附到现有边。`0` 禁用。
    pub edge_threshold_m: f64,
    /// 规则的经纬网格步长（度）。`None` 禁用网格吸附。
    pub grid_step_deg: Option<f64>,
}

impl Default for SnapConfig {
    /// 缺省即禁用：全零阈值 + 无网格，保证未显式开启时现有行为不变。
    fn default() -> Self {
        Self {
            enabled: false,
            vertex_threshold_m: 0.0,
            edge_threshold_m: 0.0,
            grid_step_deg: None,
        }
    }
}

/// 将一个坐标舍入到规则的 `step_deg` 经纬网格上。非正或非
/// 有限的步长为空操作。
pub fn snap_to_grid(p: GeoPoint, step_deg: f64) -> GeoPoint {
    // 非有限或零步长：原样返回（空操作）。
    if !step_deg.is_finite() || step_deg <= 0.0 {
        return p;
    }
    // 各自除以步长四舍五入后再乘回，即落到最近的网格线。
    let lon = (p.lon_deg / step_deg).round() * step_deg;
    let lat = (p.lat_deg / step_deg).round() * step_deg;
    GeoPoint::new(lon, lat, p.height_m)
}

/// `threshold_m` 内任意*其他*元素的最近现有顶点，若有。
/// `exclude` 跳过当前正在编辑的元素（因此一个移动的顶点不会
/// 卡入自身）。
pub fn snap_to_vertex(
    doc: &Document,
    p: GeoPoint,
    threshold_m: f64,
    exclude: Option<ElementId>,
) -> Option<GeoPoint> {
    if threshold_m <= 0.0 {
        return None;
    }
    let mut best: Option<(f64, GeoPoint)> = None;
    // 遍历全部其他元素的顶点，记录阈值内最近的一个。
    for element in doc.elements() {
        // 正在编辑的元素自身不参与吸附（避免顶点卡入自己）。
        if Some(element.id) == exclude {
            continue;
        }
        for v in element.geometry.vertices() {
            let d = p.surface_distance(v);
            // 仅在严格更近时刷新最优，保证确定性。
            if d <= threshold_m && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, v));
            }
        }
    }
    best.map(|(_, v)| v)
}

/// 一个几何的每一条线段，以连续的顶点对表示。环被视为
/// 闭合（last → first）。点类几何不贡献任何线段。
fn segments(g: &Geometry) -> Vec<(GeoPoint, GeoPoint)> {
    // 将一串点展开为相邻顶点对；closed 时补上末→首的闭合边。
    fn chain(pts: &[GeoPoint], closed: bool, out: &mut Vec<(GeoPoint, GeoPoint)>) {
        for w in pts.windows(2) {
            out.push((w[0], w[1]));
        }
        if closed && pts.len() >= 3 {
            out.push((pts[pts.len() - 1], pts[0]));
        }
    }
    let mut out = Vec::new();
    // 逐几何类型展开为其线段集合；点类不贡献任何段。
    match g {
        Geometry::Polyline(pl) => chain(&pl.positions, false, &mut out),
        Geometry::Polygon(pg) => {
            chain(&pg.outer, true, &mut out);
            for h in &pg.holes {
                chain(h, true, &mut out);
            }
        }
        Geometry::Rectangle(r) => {
            let corners = [
                GeoPoint::surface(r.west, r.south),
                GeoPoint::surface(r.east, r.south),
                GeoPoint::surface(r.east, r.north),
                GeoPoint::surface(r.west, r.north),
            ];
            chain(&corners, true, &mut out);
        }
        Geometry::Path(p) => {
            for s in &p.segments {
                match s {
                    PathSegment::Line(r) => chain(r, false, &mut out),
                    PathSegment::Arc(a) => {
                        out.push((a.start, a.center));
                        out.push((a.center, a.end));
                    }
                }
            }
        }
        Geometry::Composite(c) => {
            for part in &c.parts {
                out.extend(segments(part));
            }
        }
        _ => {}
    }
    out
}

/// 线段 `a`–`b` 上离 `p` 最近的点（在经/纬度上按平面处理，
/// 在编辑尺度下是一个很好的近似）。
fn closest_on_segment(a: GeoPoint, b: GeoPoint, p: GeoPoint) -> GeoPoint {
    // 以经纬度平面近似：求 p 在线段 a–b 上的投影参数 t（钳制到 [0,1]）。
    let dx = b.lon_deg - a.lon_deg;
    let dy = b.lat_deg - a.lat_deg;
    let len2 = dx * dx + dy * dy;
    // 退化线段（长度≈ 0）→ 直接返回端点 a。
    if len2 <= 1e-24 {
        return a;
    }
    let t = (((p.lon_deg - a.lon_deg) * dx + (p.lat_deg - a.lat_deg) * dy) / len2).clamp(0.0, 1.0);
    GeoPoint::surface(a.lon_deg + t * dx, a.lat_deg + t * dy)
}

/// `threshold_m` 内任意现有*边*上的最近点。
pub fn snap_to_edge(
    doc: &Document,
    p: GeoPoint,
    threshold_m: f64,
    exclude: Option<ElementId>,
) -> Option<GeoPoint> {
    if threshold_m <= 0.0 {
        return None;
    }
    let mut best: Option<(f64, GeoPoint)> = None;
    // 遍历其他元素的每条边，取阈值内最近的投影点。
    for element in doc.elements() {
        if Some(element.id) == exclude {
            continue;
        }
        for (a, b) in segments(&element.geometry) {
            let c = closest_on_segment(a, b, p);
            let d = p.surface_distance(c);
            // 仅在严格更近时刷新最优。
            if d <= threshold_m && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, c));
            }
        }
    }
    best.map(|(_, c)| c)
}

/// 吸附一个原始坐标：**顶点 → 边 → 网格** 优先级，每道关卡可选。
/// 禁用的配置（默认）原样返回该点。
pub fn snap(
    doc: &Document,
    p: GeoPoint,
    cfg: &SnapConfig,
    exclude: Option<ElementId>,
) -> SnapResult {
    // 总开关关闭 → 原样返回。
    if !cfg.enabled {
        return SnapResult::None(p);
    }
    // 优先级一：顶点。
    if let Some(v) = snap_to_vertex(doc, p, cfg.vertex_threshold_m, exclude) {
        return SnapResult::Vertex(v);
    }
    // 优先级二：边。
    if let Some(e) = snap_to_edge(doc, p, cfg.edge_threshold_m, exclude) {
        return SnapResult::Edge(e);
    }
    // 优先级三：网格（仅当舍入确实改变了坐标时）。
    if let Some(step) = cfg.grid_step_deg {
        let g = snap_to_grid(p, step);
        if g != p {
            return SnapResult::Grid(g);
        }
    }
    SnapResult::None(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::{Polygon, Polyline, Rectangle};
    use crate::model::Document;

    /// 快捷构造一个地面点（高度 0）。
    fn p(lon: f64, lat: f64) -> GeoPoint {
        GeoPoint::surface(lon, lat)
    }

    /// 构造一份含折线（顶点 (1,1)）与远处矩形的测试文档。
    fn doc() -> Document {
        let mut d = Document::default();
        let l = d.new_layer("L");
        // 一个带显式顶点 (1.0, 1.0) 的折线。
        let line = d.make_element(
            "line",
            Geometry::Polyline(Polyline {
                positions: vec![p(0.0, 0.0), p(1.0, 1.0), p(2.0, 0.0)],
            }),
        );
        d.add_element_to_layer(l, line);
        // 一个远处的矩形（它的角也是顶点）。
        let rect = d.make_element(
            "rect",
            Geometry::Rectangle(Rectangle {
                west: 10.0,
                south: 10.0,
                east: 11.0,
                north: 11.0,
            }),
        );
        d.add_element_to_layer(l, rect);
        d
    }

    /// 缺省配置为禁用：吸附是空操作，原样返回。
    #[test]
    fn disabled_by_default_is_a_noop() {
        let d = doc();
        let raw = p(1.0001, 1.0001);
        // 即便紧邻顶点，未启用时也不应发生吸附。
        let r = snap(&d, raw, &SnapConfig::default(), None);
        assert_eq!(r, SnapResult::None(raw));
        assert!(!r.snapped());
    }

    /// 阈值内的靠近顶点会被卡入到精确顶点坐标。
    #[test]
    fn vertex_snap_latches_within_threshold() {
        let d = doc();
        let cfg = SnapConfig {
            enabled: true,
            vertex_threshold_m: 20_000.0, // ~0.18°
            ..Default::default()
        };
        let near = p(1.0005, 1.0005); // 刚偏离 (1,1) 顶点
        match snap(&d, near, &cfg, None) {
            SnapResult::Vertex(v) => {
                assert!((v.lon_deg - 1.0).abs() < 1e-9 && (v.lat_deg - 1.0).abs() < 1e-9);
            }
            other => panic!("expected vertex snap, got {other:?}"),
        }
    }

    /// 排除当前元素后，来自它的顶点 / 边都不应参与吸附。
    #[test]
    fn vertex_snap_respects_exclude() {
        let d = doc();
        // 按名称找到折线 id
        let line = d
            .elements()
            .find(|e| e.name == "line")
            .unwrap()
            .id;
        let cfg = SnapConfig {
            enabled: true,
            vertex_threshold_m: 20_000.0,
            edge_threshold_m: 20_000.0,
            grid_step_deg: None,
        };
        // 排除每个元素 → 无可吸附项（落到 None）。
        let near = p(1.0005, 1.0005);
        let r = snap(&d, near, &cfg, Some(line));
        // 矩形在远处，因此来自折线的边/顶点也被抑制。
        assert!(matches!(r, SnapResult::None(_)), "exclude should drop the line, got {r:?}");
    }

    /// 靠近线段中部但远离顶点的点，应被投影到该边上。
    #[test]
    fn edge_snap_projects_onto_segment() {
        let d = doc();
        let cfg = SnapConfig {
            enabled: true,
            // 顶点半径很紧，使一个边中点的点优先选边，
            edge_threshold_m: 15_000.0,
            ..Default::default()
        };
        // 一个靠近 (0,0)-(1,1) 线段中部但偏离它，且
        // 远离任何顶点的点。
        let off = p(0.5, 0.45);
        match snap(&d, off, &cfg, None) {
            SnapResult::Edge(c) => {
                // 向对角线的投影保持 lon ≈ lat。
                assert!((c.lon_deg - c.lat_deg).abs() < 1e-6, "{c:?}");
                assert!(c.lon_deg > 0.3 && c.lon_deg < 0.7, "{c:?}");
            }
            other => panic!("expected edge snap, got {other:?}"),
        }
    }

    /// 网格吸附把坐标舍入到指定步长；非正步长为空操作。
    #[test]
    fn grid_snap_rounds_to_step() {
        assert_eq!(snap_to_grid(p(1.03, 2.98), 0.5), p(1.0, 3.0));
        assert_eq!(snap_to_grid(p(-1.24, 45.6), 1.0), p(-1.0, 46.0));
        // 非正的步长为空操作。
        assert_eq!(snap_to_grid(p(1.03, 2.98), 0.0), p(1.03, 2.98));
    }

    /// 顶点 / 边都落空时回退到网格吸附。
    #[test]
    fn grid_used_when_nothing_near() {
        let d = doc();
        let cfg = SnapConfig {
            enabled: true,
            vertex_threshold_m: 100.0,
            edge_threshold_m: 100.0,
            grid_step_deg: Some(0.5),
        };
        // 远离几何体，偏离网格。
        let far = p(50.13, -30.44);
        // 只能命中网格：应舍入到 0.5° 的最近格点。
        match snap(&d, far, &cfg, None) {
            SnapResult::Grid(g) => assert_eq!(g, p(50.0, -30.5)),
            other => panic!("expected grid snap, got {other:?}"),
        }
    }

    /// 闭合多边形环应额外产生一条末→首的闭合边（三角形 3 条）。
    #[test]
    fn closed_polygon_rings_yield_closing_segment() {
        let mut d = Document::default();
        let l = d.new_layer("L");
        let poly = d.make_element(
            "tri",
            Geometry::Polygon(Polygon {
                outer: vec![p(0.0, 0.0), p(2.0, 0.0), p(1.0, 2.0)],
                holes: vec![],
            }),
        );
        let poly_id = poly.id;
        d.add_element_to_layer(l, poly);
        // 一个三角形：2 条连续 + 1 条闭合 = 3 条边（环是闭合的）。
        let segs = segments(&d.element(poly_id).unwrap().geometry);
        assert_eq!(segs.len(), 3);
    }
}
