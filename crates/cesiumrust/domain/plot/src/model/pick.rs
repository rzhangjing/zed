//! 一个拾取结果：光标命中了哪个元素、在哪个部分，并提供足够的
//! 上下文以排名候选者（计划 §7）。
//!
//! `PickHit` 是普通数据，因此整个命中 → 选中的排名可在无头环境下
//! 单元测试；桥接层只将几何投影到屏幕并调用纯 [`crate::geom::hit`] 基本体，
//! 然后将每个候选者通过 [`pick_best`] 折叠。

use serde::{Deserialize, Serialize};

use super::ids::{ElementId, LayerId};

/// 光标落在元素上的哪一部分。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Part {
    /// 面的填充内部（拾取优先级最低）。
    Body,
    /// 索引 `i` 处的顶点手柄。
    Vertex(usize),
    /// 第 `i` 条边 / 段（线描边或多边形边界）。
    Edge(usize),
}

/// 候选者排名桶（§7：标记先于线先于多边形边先于
/// 多边形填充；越小越优先）。
pub const RANK_MARKER: u8 = 0;
pub const RANK_LINE: u8 = 1;
pub const RANK_POLY_EDGE: u8 = 2;
pub const RANK_POLY_BODY: u8 = 3;

/// 光标命中的一个元素，准备好与其他候选者排名。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickHit {
    /// 光标下的元素。
    pub element: ElementId,
    /// 命中的是哪个部分。
    pub part: Part,
    /// 它所在的图层（供下游 §9 权限检查使用）。
    pub layer: LayerId,
    /// 元素在图层内的绘制 / 拾取顺序（越高越胜出平局）。
    pub z_order: i32,
    /// 排名桶（`RANK_*` 常量之一）。
    pub rank: u8,
    /// 从光标到命中特征的屏幕距离（像素）；用于破平。
    pub screen_dist: f64,
}

impl PickHit {
    /// 排序键：rank 升序，然后 `z_order` 降序，然后屏幕距离
    /// 升序。字典序，因此可组合成单个 `min`。
    fn key(&self) -> (u8, i32, ordering::F64) {
        (self.rank, -self.z_order, ordering::F64(self.screen_dist))
    }
}

/// 在候选者中选出胜者（计划 §7 优先级），若为空则返回 `None`。
pub fn pick_best(cands: &[PickHit]) -> Option<PickHit> {
    cands
        .iter()
        .min_by(|a, b| a.key().cmp(&b.key()))
        .copied()
}

/// 一个小型全序包装器，使 `f64`（非 `Ord`）能置于排序键中。
mod ordering {
    /// 一个包装器，赋予 `f64` 确定性的 `Ord`（NaN 排最后）。
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct F64(pub f64);
    impl Eq for F64 {}
    impl PartialOrd for F64 {
        /// 委派给全序 `cmp`（总返回 `Some`）。
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for F64 {
        /// 按值比较；NaN（不可比）归为 `Equal` 以保持确定性。
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.0.partial_cmp(&other.0).unwrap_or(std::cmp::Ordering::Equal)
        }
    }
}
