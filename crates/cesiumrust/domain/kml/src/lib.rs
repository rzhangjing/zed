//! cesium-kml：KML（Keyhole Markup Language，可标记语言）解析器与导出器。
//!
//! Domain 层 - 纯 Rust，f64 精度。
//!
//! 提供三个子模块：parser 解析 KML 文档与几何，tour 处理 KML 导航天令，
//! export 将实体与几何序列化回 KML 文本。

pub mod export;
pub mod parser;
pub mod tour;

pub use export::{
    rgba_to_kml_color, KmlExportGeometry, KmlExportIconStyle, KmlExportLabelStyle,
    KmlExportLineStyle, KmlExportOptions, KmlExportPlacemark, KmlExportPolyStyle,
    KmlExportResult, KmlExportStyle, KmlExporter,
};
pub use parser::{
    KmlCoordinate, KmlDocument, KmlGeometry, KmlIconStyle, KmlLabelStyle, KmlLineStyle,
    KmlPlacemark, KmlPolyStyle, KmlStyle, kml_to_datasource, parse_coordinates,
    parse_kml_color, parse_kml_simple,
};
pub use tour::{FlyToMode, KmlTour, KmlTourEntry, KmlTourFlyTo, KmlTourWait};
