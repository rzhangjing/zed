//! cesium-gpx：GPX（GPS Exchange Format）解析器。
//!
//! 领域层 —— 纯 Rust，f64 精度。提供 GPX 文档的解析与到通用 [`DataSource`] 的转换。

pub mod parser;

pub use parser::{
    GpxDocument, GpxMetadata, GpxRoute, GpxRoutePoint, GpxTrack, GpxTrackPoint,
    GpxTrackSegment, GpxWaypoint, gpx_to_datasource, parse_gpx_simple,
};
