//! # cesium-material
//!
//! Fabric 材质系统：以 Rust 实现的声明式材质定义与着色器组装引擎。
//!
//! *Fabric* 材质以 JSON 声明式描述（一个 [`FabricTemplate`]），并组装为
//! GLSL 着色器源码加上一组 uniform 值（[`Material`]）。领域层执行的
//! 文本组装包括 token 重命名、子材质拼接、通道替换、
//! `czm_gammaCorrect` 包装等步骤；渲染适配器负责将生成的 GLSL 翻译为目标
//! 着色语言。
//!
//! ## 限界上下文
//!
//! 本 crate 是 **material** 限界上下文（架构方案中的 BC-16）。它仅依赖
//! 无 `glam` 的纯 Rust（`serde`、`serde_json`、`thiserror`），没有框架耦合，
//! 因此每个行为都可单元测试。
//!
//! ## 入口点
//!
//! - [`MaterialSystem::with_builtin_materials`] — 预置 25 个内置
//!   材质的缓存。
//! - [`MaterialSystem::from_type`] — 从缓存的类型构建材质。
//! - [`MaterialSystem::create_material`] — 从完整的 Fabric 模板构建材质。
//! - [`FabricTemplate::from_json_str`] — 解析 Fabric JSON 文档。

pub mod cache;
pub mod error;
pub mod fabric;
pub mod glsl;
pub mod material;
pub mod translucent;
pub mod uniform;

pub use cache::{CachedMaterial, MaterialSystem, BUILTIN_MATERIAL_TYPES};
pub use error::MaterialError;
pub use fabric::{FabricTemplate, MaterialComponents, COMPONENT_PROPERTIES, TEMPLATE_PROPERTIES};
pub use material::{Material, MaterialOptions};
pub use translucent::TranslucentSpec;
pub use uniform::{
    is_channel_string, uniform_value_from_json, CubeMapFaces, UniformValue, DEFAULT_CUBEMAP_ID,
    DEFAULT_IMAGE_ID,
};
