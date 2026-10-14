//! 场景文档的 IO 适配器（计划 §12，M8）。
//!
//! 目前仅有 [`geojson`]：一个无损（通过 `x-plot` 扩展成员）且
//! 可互操作（标准 GeoJSON 几何）的编码器 / 解码器。

pub mod geojson;

pub use geojson::{from_geojson, to_geojson, PlotIoError};
