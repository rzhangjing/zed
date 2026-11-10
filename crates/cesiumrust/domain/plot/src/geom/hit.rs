//! 纯**屏幕像素**命中测试（计划 §6 / §7）。
//!
//! 这里的每个函数都在 `[f64; 2]` 屏幕坐标上工作（左上角原点，
//! y 向下 —— 与 [`bevy::camera::Camera::world_to_viewport`] 返回的同一空间），
//! 因此整个命中 / 容差 / 内部逻辑与投影无关且
//! 无需引擎即可单元测试。桥接层通过活动相机投影一个几何的
//! 地理顶点，然后调用这些函数。
//!
//! 每个函数都返回匹配的 [`Part`] *以及* 到它的屏幕距离，以便
//! 调用方将候选项折叠通过 [`crate::model::pick::pick_best`] 进行
//! §7 的优先级排序。

use crate::model::pick::Part;

/// 线 / 边 / 顶点拾取的默认指针屏幕像素容差。
pub const DEFAULT_TOL_PX: f64 = 6.0;

/// 两个屏幕点之间的欧氏距离。
#[inline]
pub fn distance_to_point(p: [f64; 2], a: [f64; 2]) -> f64 {
    let dx = p[0] - a[0];
    let dy = p[1] - a[1];
    (dx * dx + dy * dy).sqrt()
}

/// 从 `p` 到线段 `a..b` 的最短距离（钳位，因此在端点处
/// 退化为点距离）。
pub fn distance_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let abx = b[0] - a[0];
    let aby = b[1] - a[1];
    let apx = p[0] - a[0];
    let apy = p[1] - a[1];
    let len2 = abx * abx + aby * aby;
    // 将投影参数 t 钳到 [0,1]；退化（零长）线段（len2≈ 0）回退到端点 a。
    let t = if len2 > 1e-12 {
        ((apx * abx + apy * aby) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let cx = a[0] + t * abx;
    let cy = a[1] + t * aby;
    distance_to_point(p, [cx, cy])
}

/// 一个点 / 图标标记：当光标在 `center` 的 `radius_px` 范围内时命中。
/// 将整个标记标记为 [`Part::Body`]。
pub fn hit_point(cursor: [f64; 2], center: [f64; 2], radius_px: f64) -> Option<(Part, f64)> {
    let d = distance_to_point(cursor, center);
    if d <= radius_px.max(0.0) {
        Some((Part::Body, d))
    } else {
        None
    }
}

/// 一条折线：顶点优于边（因此在编辑模式下拐角会抓住拖拽手柄），
/// 且在平局时返回最近的要素。`pts` 是
/// 投影后的屏幕顶点。
pub fn hit_polyline(cursor: [f64; 2], pts: &[[f64; 2]], tol_px: f64) -> Option<(Part, f64)> {
    if pts.is_empty() {
        return None;
    }
    // 最近的顶点。
    let mut best_v: Option<(usize, f64)> = None;
    for (i, p) in pts.iter().enumerate() {
        let d = distance_to_point(cursor, *p);
        if d <= tol_px && best_v.map(|(_, bd)| d < bd).unwrap_or(true) {
            best_v = Some((i, d));
        }
    }
    if let Some((i, d)) = best_v {
        return Some((Part::Vertex(i), d));
    }
    // 最近的线段。
    if pts.len() >= 2 {
        let mut best_e: Option<(usize, f64)> = None;
        for i in 0..pts.len() - 1 {
            let d = distance_to_segment(cursor, pts[i], pts[i + 1]);
            if d <= tol_px && best_e.map(|(_, bd)| d < bd).unwrap_or(true) {
                best_e = Some((i, d));
            }
        }
        if let Some((i, d)) = best_e {
            return Some((Part::Edge(i), d));
        }
    }
    None
}

/// 针对一个闭合环的奇偶射线投射内部测试（最后一个顶点隐式
/// 连回第一个）。
pub fn point_in_ring(p: [f64; 2], ring: &[[f64; 2]]) -> bool {
    if ring.len() < 3 {
        return false;
    }
    let mut inside = false;
    let n = ring.len();
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        let crosses = (a[1] > p[1]) != (b[1] > p[1]);
        if crosses {
            let x_int = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if p[0] < x_int {
                inside = !inside;
            }
        }
    }
    inside
}

/// 一个简单多边形（外环 + 孔洞）：边界命中（任一
/// 环的顶点 / 边）在拼接的 `[outer, holes…]` 顶点流上以最近要素索引解析为
/// [`Part::Edge`]；否则一个内部点
/// （在外环内、在每个孔洞外）解析为 [`Part::Body`]。
pub fn hit_polygon(
    cursor: [f64; 2],
    outer: &[[f64; 2]],
    holes: &[[f64; 2]],
    tol_px: f64,
) -> Option<(Part, f64)> {
    hit_polygon_multi(cursor, outer, std::slice::from_ref(&holes.to_vec()), tol_px)
}

/// 带任意数量内部孔洞环的 [`hit_polygon`]。边索引
/// 在拼接的 `[outer, holes…]` 顶点流上编号。
pub fn hit_polygon_multi(
    cursor: [f64; 2],
    outer: &[[f64; 2]],
    holes: &[Vec<[f64; 2]>],
    tol_px: f64,
) -> Option<(Part, f64)> {
    // 先边界：遍历每个环的边（带回环闭合）。
    let mut best: Option<(usize, f64)> = None;
    let mut base = 0usize;
    for ring in std::iter::once(outer).chain(holes.iter().map(|h| h.as_slice())) {
        let n = ring.len();
        for i in 0..n {
            if n < 2 {
                break;
            }
            let a = ring[i];
            let b = ring[(i + 1) % n];
            let d = distance_to_segment(cursor, a, b);
            if d <= tol_px && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((base + i, d));
            }
        }
        base += n;
    }
    if let Some((i, d)) = best {
        return Some((Part::Edge(i), d));
    }
    // 内部（奇偶，孔洞被 XOR 剔除）。
    if point_in_ring(cursor, outer) && holes.iter().all(|h| !point_in_ring(cursor, h)) {
        return Some((Part::Body, 0.0));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 零长线段退化为点距；跨出线段时钳位到最近端点。
    #[test]
    fn segment_distance_clamps_to_endpoints() {
        // 水平线段 (0,0)-(10,0)。
        assert!((distance_to_segment([5.0, 3.0], [0.0, 0.0], [10.0, 0.0]) - 3.0).abs() < 1e-9);
        // 越过左端 → 到端点的距离（−2, 3）。
        let d = distance_to_segment([-2.0, 3.0], [0.0, 0.0], [10.0, 0.0]);
        assert!((d - (13.0f64).sqrt()).abs() < 1e-9, "{d}");
        // 退化的零长线段行为如同点距离。
        assert!((distance_to_segment([3.0, 4.0], [0.0, 0.0], [0.0, 0.0]) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn point_marker_within_radius_only() {
        assert!(hit_point([2.0, 0.0], [0.0, 0.0], 3.0).is_some());
        assert_eq!(hit_point([2.0, 0.0], [0.0, 0.0], 3.0).unwrap().0, Part::Body);
        assert!(hit_point([4.0, 0.0], [0.0, 0.0], 3.0).is_none());
    }

    #[test]
    fn polyline_prefers_nearest_vertex_then_edge() {
        let pts = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        // 正落在顶点 1 上 → Vertex(1)。
        let (part, _) = hit_polyline([10.0, 0.5], &pts, 2.0).unwrap();
        assert_eq!(part, Part::Vertex(1));
        // 沿边 0 中部，远离顶点 → Edge(0)。
        let (part, _) = hit_polyline([5.0, 0.5], &pts, 2.0).unwrap();
        assert_eq!(part, Part::Edge(0));
        // 远离一切 → 未命中。
        assert!(hit_polyline([5.0, 5.0], &pts, 2.0).is_none());
    }

    #[test]
    fn ring_inside_even_odd() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert!(point_in_ring([5.0, 5.0], &square));
        assert!(!point_in_ring([15.0, 5.0], &square));
        assert!(!point_in_ring([5.0, 5.0], &[[0.0, 0.0], [1.0, 1.0]]), "open ring");
    }

    #[test]
    fn polygon_edge_before_body_and_hole_excludes() {
        let outer = [[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]];
        let hole = [[8.0, 8.0], [12.0, 8.0], [12.0, 12.0], [8.0, 12.0]];
        // 在外边界上 → 一次边命中。
        let (part, _) = hit_polygon([0.5, 10.0], &outer, &hole, 2.0).unwrap();
        assert!(matches!(part, Part::Edge(_)), "boundary: {part:?}");
        // 深居内部，避开孔洞 → Body。
        let (part, d) = hit_polygon([3.0, 3.0], &outer, &hole, 2.0).unwrap();
        assert_eq!(part, Part::Body);
        assert_eq!(d, 0.0);
        // 在孔洞内 → 未命中。
        assert!(hit_polygon([10.0, 10.0], &outer, &hole, 1.0).is_none());
        // 完全在外 → 未命中。
        assert!(hit_polygon([30.0, 30.0], &outer, &hole, 2.0).is_none());
    }
}
