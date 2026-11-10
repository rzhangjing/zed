//! 逐 loader 的 mesh/贴图上传预算权重（M1.4）。
//!
//! 源自 core 的 `BudgetPolicy`（`cesium_pipeline::DefaultBudget`），它的
//! `MAX_MESH_UPLOADS_PER_FRAME = 12` 是黄金路径的每帧 GPU 上传上限
//! （`dynamic_globe.rs:53`）。M1.4 按相对成本把那个量级拆分到三个
//! 产出 mesh/贴图的 P0 loader，所以一个门控 ON 的帧从不上传超过它的
//! 权重 —— 恰如黄金路径那样精确地约束帧线程 GPU 工作。
//!
//! | 加载器  | 权重 | 理由                                              |
//! |---------|------|-------------------------------------------------------|
//! | terrain | 6    | quantized-mesh + 裙边（最重的逐 tile mesh）        |
//! | tileset | 4    | b3dm/glTF mesh                                    |
//! | imagery | 12   | 贴图上传（保留完整的 mesh 预算量级）        |
//!
//! tileset *json* loader 不产出 mesh，所以它不携带任何预算。
//!
//! # 门控 OFF 中立性
//! 旧（门控 OFF）路径传入 [`UNBOUNDED`]，于是每个已解析 tile 都在帧内被
//! 处理 —— 与迁移前的 drain 逐字节相同，后者没有预算。这些权重只在
//! 门控 ON 的 pipeline 路径上生效。

use cesium_pipeline::DefaultBudget;
use cesium_ports_driven::BudgetPolicy;

/// 旧（门控 OFF）路径的哨兵预算：处理所有已解析 tile。
pub const UNBOUNDED: usize = usize::MAX;

/// Terrain mesh 上传权重（`dynamic_globe` 语义：重的 quantized-mesh）。
pub const TERRAIN_MESH_WEIGHT: usize = 6;
/// Tileset content mesh 上传权重（b3dm/glTF）。
pub const TILESET_MESH_WEIGHT: usize = 4;
/// Imagery 贴图上传权重，与 `MAX_MESH_UPLOADS_PER_FRAME` 对齐。
pub const IMAGERY_TEXTURE_WEIGHT: usize = 12;

/// Terrain 逐帧 mesh 预算，由 core `BudgetPolicy` 封顶，以便未来
/// 对 `MAX_MESH_UPLOADS_PER_FRAME` 的下调会自动收紧这个权重。
pub fn terrain_mesh_budget() -> usize {
    TERRAIN_MESH_WEIGHT.min(DefaultBudget.max_mesh_uploads_per_frame())
}

/// Tileset 逐帧 mesh 预算，由 core `BudgetPolicy` 封顶。
pub fn tileset_mesh_budget() -> usize {
    TILESET_MESH_WEIGHT.min(DefaultBudget.max_mesh_uploads_per_frame())
}

/// Imagery 逐帧贴图预算，由 core `BudgetPolicy` 封顶。
pub fn imagery_texture_budget() -> usize {
    IMAGERY_TEXTURE_WEIGHT.min(DefaultBudget.max_mesh_uploads_per_frame())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_match_spec() {
        assert_eq!(TERRAIN_MESH_WEIGHT, 6);
        assert_eq!(TILESET_MESH_WEIGHT, 4);
        assert_eq!(IMAGERY_TEXTURE_WEIGHT, 12);
    }

    #[test]
    fn budgets_bounded_by_core_policy() {
        // 每个权重都必须被夹到黄金路径的每帧上传上限。
        let cap = DefaultBudget.max_mesh_uploads_per_frame();
        assert_eq!(cap, 12);
        assert_eq!(terrain_mesh_budget(), 6);
        assert_eq!(tileset_mesh_budget(), 4);
        assert_eq!(imagery_texture_budget(), 12);
        assert!(terrain_mesh_budget() <= cap);
        assert!(tileset_mesh_budget() <= cap);
        assert!(imagery_texture_budget() <= cap);
    }

    #[test]
    fn unbounded_is_max_for_legacy_path() {
        // 门控 OFF 必须在帧内 drain 一切（无预算），从而恰如
        // 其本地保留迁移前的行为。
        assert_eq!(UNBOUNDED, usize::MAX);
        assert!(UNBOUNDED > imagery_texture_budget());
    }
}
