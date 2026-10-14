//! cesium-quadtree：四叉树遍历与瓦片调度。
//!
//! 领域层 —— 纯 Rust，f64 精度。
//!
//! 子模块：
//! - [`traversal`]：四叉树遍历与 SSE 驱动的 LOD 选择
//! - [`cache`]：瓦片加载队列与 LRU 缓存
//! - [`quadtree_tile_adjacency`]：瓦片东/西/南/北邻接查询

pub mod cache;
pub mod quadtree_tile_adjacency;
pub mod traversal;

pub use cache::{
    QueuedTile, SchedulerConfig, SchedulerStats, TileCache, TileId, TileLoadQueue, TilePriority,
};
pub use traversal::{
    QuadtreeConfig, QuadtreePrimitive, QuadtreeTile, TileState, TraversalResult,
};
