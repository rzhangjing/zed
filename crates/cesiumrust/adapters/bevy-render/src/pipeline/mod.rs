//! `cesium-pipeline` 纯 std core 的 Bevy 绑定层（M1.3）。
//!
//! 本模块刻意保持**瘦薄**：它只包含需要 Bevy 类型的那些部分——
//! - 一个 `Handle` 类型的 GPU 缓存（`gpu_handle`），它把所有逐出逻辑
//!   委派给 core 的 `cesium_pipeline::GpuCache`（后者实现 M1.1 的
//!   `EvictionPolicy` 契约），
//! - Component/Resource 桥接（`bindings`），
//! - Bevy 系统装配（`system_wiring`），
//! - 选择性开启的 `CesiumPipelinePlugin`（`bevy_pipeline`）。
//!
//! **这里不驻留任何业务逻辑。** 预算/逐出/过期/重试语义
//! 都由 core crate 拥有，它忠实复现了受保护的
//! 黄金路径 `application/cesium-app/src/dynamic_globe.rs`。
//!
//! # 推广
//! `CesiumPipelinePlugin` 是**选择性开启**的，在 M1.3 中**不**加入默认的
//! cesium-app 运行时（`CESIUM_ENABLE_PIPELINE` 保持 OFF）。把该插件
//! 接入实际 app 被推迟到 M1.4/M1.5。这保证黄金路径
//! 相对本次改动是像素中立的。
//!
//! # 异步运行时
//! core 使用一个 ureq 阻塞 worker 池；本绑定层不引入任何
//! tokio 主运行时（tokio 至多仍是 core 的 reqwest 后端的一个可选传递依赖，
//! 绝不是 pipeline 运行时）。

pub mod bevy_pipeline;
pub mod bindings;
pub mod budget;
pub mod fetch;
pub mod gpu_handle;
pub mod system_wiring;

/// Tile key `(x, y, zoom)`。
///
/// 与 `dynamic_globe.rs:77`（`type TileKey = (u32, u32, u32)`）一致。zoom
/// 分量（`.2`）驱动底层豁免不变式
/// （`dynamic_globe.rs:1483`）。
pub type TileKey = (u32, u32, u32);

pub use bevy_pipeline::CesiumPipelinePlugin;
pub use bindings::{PipelineEvictionStats, PipelineTile};
pub use budget::{
    imagery_texture_budget, terrain_mesh_budget, tileset_mesh_budget, UNBOUNDED,
};
pub use fetch::{fetch_gated, pipeline_gate_enabled, FetchRoute};
pub use gpu_handle::{BevyGpuHandleCache, GpuTileHandles};
pub use system_wiring::gpu_cache_eviction_system;
