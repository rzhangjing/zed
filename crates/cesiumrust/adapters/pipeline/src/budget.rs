//! 默认预算策略 —— 逐字采用的 `dynamic_globe.rs`（L48-73）常量。
//!
//! 这些值是**黄金路径参考**，绝不能偏移。
//! 任何未来的调优都应通过自定义的 `BudgetPolicy` 实现来完成，
//! 而不是修改这些默认值。

use cesium_ports_driven::BudgetPolicy;

/// 与 `dynamic_globe.rs` 常量完全一致的默认预算。
///
/// | 常量 | 值 | 来源行 |
/// |----------|-------|-------------|
/// | `DOWNLOAD_THREADS` | 16 | L48 |
/// | `MAX_MESH_UPLOADS_PER_FRAME` | 12 | L53 |
/// | `MAX_SPAWNS_PER_FRAME` | 16 | L54 |
/// | `MAX_TEXTURE_UPLOADS_PER_FRAME` | 16 | L55 |
/// | `MAX_DESPAWNS_PER_FRAME` | 24 | L58 |
/// | `MAX_TILE_ENTITIES` | 1800 | L65 |
/// | `BASE_LAYER_ZOOM` | 3 | L70 |
/// | `MAX_GPU_CACHE_ENTRIES` | 3000 | L73 |
#[derive(Debug, Clone, Copy)]
pub struct DefaultBudget;

impl DefaultBudget {
    /// `dynamic_globe.rs:48` —— 并行下载/网格构建的工作线程数。
    pub const DOWNLOAD_THREADS: usize = 16;
    /// `dynamic_globe.rs:53` —— 每帧的网格 GPU 上传数。
    pub const MAX_MESH_UPLOADS_PER_FRAME: usize = 12;
    /// `dynamic_globe.rs:54` —— 每帧的实体 spawn 数。
    pub const MAX_SPAWNS_PER_FRAME: usize = 16;
    /// `dynamic_globe.rs:55` —— 每帧的纹理 GPU 上传数。
    pub const MAX_TEXTURE_UPLOADS_PER_FRAME: usize = 16;
    /// `dynamic_globe.rs:58` —— 每帧的实体 despawn 数。
    pub const MAX_DESPAWNS_PER_FRAME: usize = 24;
    /// `dynamic_globe.rs:65` —— 存活瓦片实体的硬上限。
    pub const MAX_TILE_ENTITIES: usize = 1800;
    /// `dynamic_globe.rs:70` —— 最粗的常驻 zoom 层级。
    pub const BASE_LAYER_ZOOM: u32 = 3;
    /// `dynamic_globe.rs:73` —— GPU 句柄缓存的上界。
    pub const MAX_GPU_CACHE_ENTRIES: usize = 3000;
}

impl Default for DefaultBudget {
    /// `DefaultBudget` 为无字段单元结构，默认值即自身。
    fn default() -> Self {
        Self
    }
}

impl BudgetPolicy for DefaultBudget {
    /// 下载工作线程数（固定取 [`DefaultBudget::DOWNLOAD_THREADS`]）。
    #[inline]
    fn download_threads(&self) -> usize {
        Self::DOWNLOAD_THREADS
    }

    /// 每帧网格上传预算上限。
    #[inline]
    fn max_mesh_uploads_per_frame(&self) -> usize {
        Self::MAX_MESH_UPLOADS_PER_FRAME
    }

    /// 每帧实体 spawn 预算上限。
    #[inline]
    fn max_spawns_per_frame(&self) -> usize {
        Self::MAX_SPAWNS_PER_FRAME
    }

    /// 每帧纹理上传预算上限。
    #[inline]
    fn max_texture_uploads_per_frame(&self) -> usize {
        Self::MAX_TEXTURE_UPLOADS_PER_FRAME
    }

    /// 每帧实体 despawn 预算上限。
    #[inline]
    fn max_despawns_per_frame(&self) -> usize {
        Self::MAX_DESPAWNS_PER_FRAME
    }

    /// 同时存活的地表瓦片实体总数上限。
    #[inline]
    fn max_tile_entities(&self) -> usize {
        Self::MAX_TILE_ENTITIES
    }

    /// 基础图层（全球预加载）的缩放层级。
    #[inline]
    fn base_layer_zoom(&self) -> u32 {
        Self::BASE_LAYER_ZOOM
    }

    /// GPU 句柄缓存的条目上界。
    #[inline]
    fn max_gpu_cache_entries(&self) -> usize {
        Self::MAX_GPU_CACHE_ENTRIES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budget_matches_dynamic_globe_constants() {
        let b = DefaultBudget;
        assert_eq!(b.download_threads(), 16);
        assert_eq!(b.max_mesh_uploads_per_frame(), 12);
        assert_eq!(b.max_spawns_per_frame(), 16);
        assert_eq!(b.max_texture_uploads_per_frame(), 16);
        assert_eq!(b.max_despawns_per_frame(), 24);
        assert_eq!(b.max_tile_entities(), 1800);
        assert_eq!(b.base_layer_zoom(), 3);
        assert_eq!(b.max_gpu_cache_entries(), 3000);
    }

    #[test]
    fn termination_invariant_holds() {
        // dynamic_globe.rs L1471-1472：MAX_TILE_ENTITIES << MAX_GPU_CACHE_ENTRIES
        // 保证总存在可驱逐（死）的条目。
        let b = DefaultBudget;
        assert!(
            b.max_tile_entities() < b.max_gpu_cache_entries(),
            "Termination invariant violated: MAX_TILE_ENTITIES({}) must be < MAX_GPU_CACHE_ENTRIES({})",
            b.max_tile_entities(),
            b.max_gpu_cache_entries()
        );
    }
}
