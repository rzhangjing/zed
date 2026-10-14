//! cesium-scene-mode：场景模式与形态变换。
//!
//! 领域层 —— 纯 Rust，f64 精度。提供 [`SceneMode`]、形态变换状态 [`MorphState`] 与
//! 2D/3D/Columbus View 之间的投影换算。

pub mod scene_mode;

pub use scene_mode::{
    compute_camera_for_mode, morph_position, project_to_2d, project_to_columbus_view,
    smoothstep, unproject_from_2d, MapProjection2D, MorphState, SceneMode,
};
