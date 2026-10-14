//! cesium-imagery：影像图层领域模型
//!
//! 组织影像图层的配置（[`ImageryLayer`]）、集合（[`ImageryLayerCollection`]）、
//! 单张影像的状态机（[`ImageryState`]）、瓦片与影像的关联（[`TileImagery`]）、
//! 瓦片请求调度（[`compute_tile_requests`]）以及多层像素混合（[`blend_pixel`]）。

pub mod imagery_layer;
pub mod imagery_state;
pub mod tile_imagery;
pub mod layer_collection;
pub mod tile_request;
pub mod blending;

pub use imagery_layer::ImageryLayer;
pub use imagery_state::ImageryState;
pub use tile_imagery::TileImagery;
pub use layer_collection::ImageryLayerCollection;
pub use tile_request::{ImageryTileRequest, compute_tile_requests, compute_texture_mapping};
pub use blending::{PixelColor, blend_pixel, composite_layers, compute_effective_alpha, apply_color_adjustments};

use serde::{Deserialize, Serialize};

/// 用于分屏对比的影像分割方向。
///
/// 指定图层渲染到分割视图的左/右侧，或不分隔占据全屏。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SplitDirection {
    /// 使用分割器的左侧。
    Left = -1,
    /// 不分隔，使用全屏。
    #[default]
    None = 0,
    /// 使用分割器的右侧。
    Right = 1,
}

/// 影像图层的 alpha 混合模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AlphaBlendingMode {
    /// 标准 alpha 混合（src * alpha + dst * (1 - alpha)）
    #[default]
    Standard,
    /// 叠加混合（src + dst）
    Additive,
    /// 正片叠底混合（src * dst）
    Multiplicative,
}
