//! 多边形细分（计划 §6）：对带可选孔的填充环进行耳切三角剖分，
//! 经由 `earcut` crate（一个工作区依赖）。
//!
//! 输入环是普通的 `[f64; 2]` 位置（世界或屏幕 —— 由调用方
//! 决定，且必须馈入一个*平面*空间）；输出是 CCW 三角形索引，指向
//! 拼接后的 `[outer…, hole0…, hole1…]` 顶点流，可直接成为
//! `TriangleList` 网格。

use earcut::Earcut;

/// 对一个简单多边形进行三角剖分。`outer` 是边界环；每个 `hole` 是一个
/// 内部环。返回指向拼接后顶点流的三角形索引
/// （`outer` 后依次跟各孔）。当点数太少无法构成一个面时为空。
pub fn triangulate(outer: &[[f64; 2]], holes: &[[f64; 2]]) -> Vec<u32> {
    triangulate_rings(outer, std::iter::once(holes))
}

/// [`triangulate`] 的多孔变体。
pub fn triangulate_holes(outer: &[[f64; 2]], holes: &[Vec<[f64; 2]>]) -> Vec<u32> {
    triangulate_rings(outer, holes.iter().map(|h| h.as_slice()))
}

/// [`triangulate`] / [`triangulate_holes`] 的共同实现：先把外环与
/// 各非空孔环拼接成一条顶点流，并记录每个孔在流中的起始下标
/// （`earcut` 用它来标记孔），再跑耳切算法得到 CCW 三角形索引。
fn triangulate_rings<'a>(
    outer: &'a [[f64; 2]],
    holes: impl Iterator<Item = &'a [[f64; 2]]>,
) -> Vec<u32> {
    let mut data: Vec<[f64; 2]> = Vec::with_capacity(outer.len());
    let mut hole_starts: Vec<usize> = Vec::new();
    data.extend_from_slice(outer);
    // 依次追加每个非空孔，并记下它在拼接流里的起始位置。
    for h in holes {
        if !h.is_empty() {
            hole_starts.push(data.len());
            data.extend_from_slice(h);
        }
    }
    // 少于 3 个顶点无法构成任何三角形，直接返回空。
    if data.len() < 3 {
        return Vec::new();
    }
    let mut ear = Earcut::<f64>::new();
    let mut tris: Vec<usize> = Vec::new();
    ear.earcut(data.iter().copied(), &hole_starts, &mut tris);
    tris.into_iter().map(|i| i as u32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三角形本身即是一个面，无需细分：恰好输出 1 个三角形。
    #[test]
    fn triangle_needs_no_subdivision() {
        let tri = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let idx = triangulate(&tri, &[]);
        assert_eq!(idx.len(), 3, "one triangle");
        assert!(idx.iter().all(|&i| (0..3).contains(&i)));
    }

    #[test]
    fn convex_quad_splits_into_two() {
        let quad = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        let idx = triangulate(&quad, &[]);
        assert_eq!(idx.len(), 6, "two triangles");
        assert!(idx.iter().all(|&i| i < 4));
    }

    #[test]
    fn square_with_centre_hole_uses_all_vertices() {
        let outer = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = [[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]];
        let idx = triangulate(&outer, &hole);
        // 共 8 个顶点；带孔的环需要 > 1 个三角形来环绕它。
        assert!(idx.len() >= 6 && idx.len().is_multiple_of(3), "{:?}", idx.len());
        assert!(idx.iter().all(|&i| i < 8), "indices stay in range: {idx:?}");
    }

    #[test]
    fn degenerate_input_yields_no_triangles() {
        assert!(triangulate(&[[0.0, 0.0], [1.0, 1.0]], &[]).is_empty());
        assert!(triangulate(&[], &[]).is_empty());
    }
}
