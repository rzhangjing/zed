//! KML 导出功能。
//!
//! 将内存中的实体、样式与几何序列化为符合 OGC KML 2.2 规范的 XML 文本。
//! 支持点、线（LineString）、面（Polygon）、模型（glTF 引用）四类几何，
//! 以及图标、线、面、标注四种样式与外部文件（KMZ）的组织。

use std::collections::HashMap;

// ============================================================================
// KmlExportOptions
// ============================================================================

/// KML 导出的选项。
#[derive(Debug, Clone, PartialEq)]
pub struct KmlExportOptions {
    /// KML 文档名称。
    pub name: String,
    /// KML 文档描述。
    pub description: Option<String>,
    /// 是否导出时间动态数据。
    pub time_dynamic: bool,
    /// 是否导出模型（glTF）引用。
    pub export_model: bool,
    /// 是否导出图像资源。
    pub export_images: bool,
    /// KML 版本（2.2 为标准）。
    pub kml_version: String,
}

impl Default for KmlExportOptions {
    /// 返回导出选项的默认值：文档名为 "Cesium Export"，KML 版本 2.2，导出模型与图像。
    fn default() -> Self {
        Self {
            name: "Cesium Export".to_string(),
            description: None,
            time_dynamic: false,
            export_model: true,
            export_images: true,
            kml_version: "2.2".to_string(),
        }
    }
}

// ============================================================================
// KmlExportResult
// ============================================================================

/// KML 导出操作的结果。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KmlExportResult {
    /// KML 文档内容。
    pub kml: String,
    /// KML 引用的外部文件（图像、模型）。
    pub external_files: HashMap<String, Vec<u8>>,
    /// KMZ（压缩的 KML）内容，若已请求。
    pub kmz: Option<Vec<u8>>,
}

// ============================================================================
// KmlExporter
// ============================================================================

/// 用于将实体转换为 KML 格式的 KML 导出器。
#[derive(Debug, Clone, PartialEq)]
pub struct KmlExporter {
    /// 导出选项。
    pub options: KmlExportOptions,
    /// 命名空间声明。
    namespaces: Vec<(String, String)>,
    /// 样式定义。
    styles: Vec<KmlExportStyle>,
    /// 已添加的地标集合。
    placemarks: Vec<KmlExportPlacemark>,
}

impl KmlExporter {
    /// 使用默认选项创建一个新导出器。
    pub fn new() -> Self {
        // 预置 KML/gx/atom 三个标准命名空间声明
        Self {
            options: KmlExportOptions::default(),
            namespaces: vec![
                ("xmlns".to_string(), "http://www.opengis.net/kml/2.2".to_string()),
                ("xmlns:gx".to_string(), "http://www.google.com/kml/ext/2.2".to_string()),
                ("xmlns:atom".to_string(), "http://www.w3.org/2005/Atom".to_string()),
            ],
            styles: Vec::new(),
            placemarks: Vec::new(),
        }
    }

    /// 使用自定义选项创建。
    pub fn with_options(options: KmlExportOptions) -> Self {
        // 复用 new() 的命名空间与空容器，仅替换选项
        Self {
            options,
            ..Self::new()
        }
    }

    /// 添加一个样式定义。
    pub fn add_style(&mut self, style: KmlExportStyle) {
        // 样式追加到 Document 级样式表
        self.styles.push(style);
    }

    /// 添加一个地标。
    pub fn add_placemark(&mut self, placemark: KmlExportPlacemark) {
        // 地标追加到文档末尾
        self.placemarks.push(placemark);
    }

    /// 生成 KML 文档。
    pub fn to_kml(&self) -> String {
        let mut kml = String::new();

        // XML 声明
        kml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");

        // 带命名空间的 KML 根元素
        kml.push_str("<kml");
        for (prefix, uri) in &self.namespaces {
            kml.push_str(&format!(" {}=\"{}\"", prefix, uri));
        }
        kml.push_str(">\n");

        // 文档
        kml.push_str("  <Document>\n");

        // 名称
        kml.push_str(&format!("    <name>{}</name>\n", escape_xml(&self.options.name)));

        // 描述
        if let Some(ref desc) = self.options.description {
            kml.push_str(&format!("    <description>{}</description>\n", escape_xml(desc)));
        }

        // 样式
        for style in &self.styles {
            kml.push_str(&style.to_kml(4));
        }

        // 地标
        for placemark in &self.placemarks {
            kml.push_str(&placemark.to_kml(4));
        }

        kml.push_str("  </Document>\n");
        kml.push_str("</kml>\n");

        kml
    }

    /// 导出为 KmlExportResult。
    pub fn export(&self) -> KmlExportResult {
        // 当前仅生成 KML 文本，外部文件与 KMZ 留空
        KmlExportResult {
            kml: self.to_kml(),
            external_files: HashMap::new(),
            kmz: None,
        }
    }
}

impl Default for KmlExporter {
    /// 以默认选项构造导出器，等价于 [`KmlExporter::new`]。
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// KmlExportStyle
// ============================================================================

/// 用于 KML 导出的样式定义。
#[derive(Debug, Clone, PartialEq)]
pub struct KmlExportStyle {
    /// 样式 ID。
    pub id: String,
    /// 图标样式（用于点）。
    pub icon_style: Option<KmlExportIconStyle>,
    /// 线样式（用于线）。
    pub line_style: Option<KmlExportLineStyle>,
    /// 面样式（用于多边形）。
    pub poly_style: Option<KmlExportPolyStyle>,
    /// 标注样式。
    pub label_style: Option<KmlExportLabelStyle>,
}

impl KmlExportStyle {
    /// 创建一个新的样式。
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            icon_style: None,
            line_style: None,
            poly_style: None,
            label_style: None,
        }
    }

    /// 为此样式生成 KML。
    pub fn to_kml(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let mut kml = format!("{}<Style id=\"{}\">\n", pad, self.id);

        if let Some(ref icon) = self.icon_style {
            // 图标样式：输出颜色、缩放与可选的 Icon href
            kml.push_str(&format!("{}  <IconStyle>\n", pad));
            kml.push_str(&format!("{}    <color>{}</color>\n", pad, icon.color));
            kml.push_str(&format!("{}    <scale>{}</scale>\n", pad, icon.scale));
            if let Some(ref href) = icon.icon_href {
                kml.push_str(&format!("{}    <Icon><href>{}</href></Icon>\n", pad, href));
            }
            kml.push_str(&format!("{}  </IconStyle>\n", pad));
        }

        if let Some(ref line) = self.line_style {
            // 线样式：输出颜色与宽度
            kml.push_str(&format!("{}  <LineStyle>\n", pad));
            kml.push_str(&format!("{}    <color>{}</color>\n", pad, line.color));
            kml.push_str(&format!("{}    <width>{}</width>\n", pad, line.width));
            kml.push_str(&format!("{}  </LineStyle>\n", pad));
        }

        if let Some(ref poly) = self.poly_style {
            // 面样式：填充与轮廓以 0/1 布尔输出
            kml.push_str(&format!("{}  <PolyStyle>\n", pad));
            kml.push_str(&format!("{}    <color>{}</color>\n", pad, poly.color));
            kml.push_str(&format!("{}    <fill>{}</fill>\n", pad, if poly.fill { 1 } else { 0 }));
            kml.push_str(&format!("{}    <outline>{}</outline>\n", pad, if poly.outline { 1 } else { 0 }));
            kml.push_str(&format!("{}  </PolyStyle>\n", pad));
        }

        if let Some(ref label) = self.label_style {
            // 标注样式：输出颜色与缩放
            kml.push_str(&format!("{}  <LabelStyle>\n", pad));
            kml.push_str(&format!("{}    <color>{}</color>\n", pad, label.color));
            kml.push_str(&format!("{}    <scale>{}</scale>\n", pad, label.scale));
            kml.push_str(&format!("{}  </LabelStyle>\n", pad));
        }

        kml.push_str(&format!("{}</Style>\n", pad));
        kml
    }
}

/// 用于 KML 导出的图标样式。
#[derive(Debug, Clone, PartialEq)]
pub struct KmlExportIconStyle {
    /// KML 格式的颜色（aabbggrr）。
    pub color: String,
    /// 缩放因子。
    pub scale: f64,
    /// 图标 href。
    pub icon_href: Option<String>,
}

/// 用于 KML 导出的线样式。
#[derive(Debug, Clone, PartialEq)]
pub struct KmlExportLineStyle {
    /// KML 格式的颜色（aabbggrr）。
    pub color: String,
    /// 线宽（像素）。
    pub width: f64,
}

/// 用于 KML 导出的面样式。
#[derive(Debug, Clone, PartialEq)]
pub struct KmlExportPolyStyle {
    /// KML 格式的颜色（aabbggrr）。
    pub color: String,
    /// 是否填充多边形。
    pub fill: bool,
    /// 是否绘制轮廓。
    pub outline: bool,
}

/// 用于 KML 导出的标注样式。
#[derive(Debug, Clone, PartialEq)]
pub struct KmlExportLabelStyle {
    /// KML 格式的颜色（aabbggrr）。
    pub color: String,
    /// 缩放因子。
    pub scale: f64,
}

// ============================================================================
// KmlExportPlacemark
// ============================================================================

/// 用于 KML 导出的地标。
#[derive(Debug, Clone, PartialEq)]
pub struct KmlExportPlacemark {
    /// 地标名称。
    pub name: String,
    /// 地标描述。
    pub description: Option<String>,
    /// 样式 URL 引用（例如 "#style1"）。
    pub style_url: Option<String>,
    /// 几何。
    pub geometry: KmlExportGeometry,
}

impl KmlExportPlacemark {
    /// 创建一个新的地标。
    pub fn new(name: &str, geometry: KmlExportGeometry) -> Self {
        Self {
            name: name.to_string(),
            description: None,
            style_url: None,
            geometry,
        }
    }

    /// 为此地标生成 KML。
    pub fn to_kml(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        // 地标：名称/描述经 XML 转义，可选 styleUrl 引用，几何递归缩进 2 空格
        let mut kml = format!("{}<Placemark>\n", pad);
        kml.push_str(&format!("{}  <name>{}</name>\n", pad, escape_xml(&self.name)));

        if let Some(ref desc) = self.description {
            kml.push_str(&format!("{}  <description>{}</description>\n", pad, escape_xml(desc)));
        }

        if let Some(ref style_url) = self.style_url {
            kml.push_str(&format!("{}  <styleUrl>{}</styleUrl>\n", pad, style_url));
        }

        kml.push_str(&self.geometry.to_kml(indent + 2));
        kml.push_str(&format!("{}</Placemark>\n", pad));
        kml
    }
}

/// 用于 KML 导出的几何类型。
#[derive(Debug, Clone, PartialEq)]
pub enum KmlExportGeometry {
    /// 点几何。
    Point {
        /// 经度、纬度、高度。
        coordinates: Vec<[f64; 3]>,
    },
    /// LineString（线）几何。
    LineString {
        /// 坐标。
        coordinates: Vec<[f64; 3]>,
        /// 是否沿地形细分（tessellate）。
        tessellate: bool,
    },
    /// 多边形几何。
    Polygon {
        /// 外边界坐标。
        outer_boundary: Vec<[f64; 3]>,
        /// 内边界（空洞）。
        inner_boundaries: Vec<Vec<[f64; 3]>>,
    },
    /// 模型（glTF）引用。
    Model {
        /// 模型 href。
        href: String,
        /// 位置 [lon, lat, alt]。
        location: [f64; 3],
        /// 航向角（度）。
        heading: f64,
        /// 仰俯角（度）。
        tilt: f64,
        /// 翻滚角（度）。
        roll: f64,
        /// 缩放。
        scale: f64,
    },
}

impl KmlExportGeometry {
    /// 为此几何生成 KML。
    pub fn to_kml(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match self {
            Self::Point { coordinates } => {
                // 点：将坐标序列写入单个 <coordinates>
                let mut kml = format!("{}<Point>\n", pad);
                kml.push_str(&format!("{}  <coordinates>{}</coordinates>\n", pad, format_coordinates(coordinates)));
                kml.push_str(&format!("{}</Point>\n", pad));
                kml
            }
            Self::LineString { coordinates, tessellate } => {
                // 线：tessellate 为真时输出沿地形细分标记
                let mut kml = format!("{}<LineString>\n", pad);
                if *tessellate {
                    kml.push_str(&format!("{}  <tessellate>1</tessellate>\n", pad));
                }
                kml.push_str(&format!("{}  <coordinates>{}</coordinates>\n", pad, format_coordinates(coordinates)));
                kml.push_str(&format!("{}</LineString>\n", pad));
                kml
            }
            Self::Polygon { outer_boundary, inner_boundaries } => {
                // 面：外边界包于 outerBoundaryIs 的 LinearRing，内边界（空洞）逐个包于 innerBoundaryIs
                let mut kml = format!("{}<Polygon>\n", pad);
                kml.push_str(&format!("{}  <outerBoundaryIs>\n", pad));
                kml.push_str(&format!("{}    <LinearRing>\n", pad));
                kml.push_str(&format!("{}      <coordinates>{}</coordinates>\n", pad, format_coordinates(outer_boundary)));
                kml.push_str(&format!("{}    </LinearRing>\n", pad));
                kml.push_str(&format!("{}  </outerBoundaryIs>\n", pad));

                for inner in inner_boundaries {
                    kml.push_str(&format!("{}  <innerBoundaryIs>\n", pad));
                    kml.push_str(&format!("{}    <LinearRing>\n", pad));
                    kml.push_str(&format!("{}      <coordinates>{}</coordinates>\n", pad, format_coordinates(inner)));
                    kml.push_str(&format!("{}    </LinearRing>\n", pad));
                    kml.push_str(&format!("{}  </innerBoundaryIs>\n", pad));
                }

                kml.push_str(&format!("{}</Polygon>\n", pad));
                kml
            }
            Self::Model { href, location, heading, tilt, roll, scale } => {
                // 模型：分别输出 Location（位置）、Orientation（朝向角）、Scale（缩放）与 Link（href）
                let mut kml = format!("{}<Model>\n", pad);
                kml.push_str(&format!("{}  <Location>\n", pad));
                kml.push_str(&format!("{}    <longitude>{}</longitude>\n", pad, location[0]));
                kml.push_str(&format!("{}    <latitude>{}</latitude>\n", pad, location[1]));
                kml.push_str(&format!("{}    <altitude>{}</altitude>\n", pad, location[2]));
                kml.push_str(&format!("{}  </Location>\n", pad));
                kml.push_str(&format!("{}  <Orientation>\n", pad));
                kml.push_str(&format!("{}    <heading>{}</heading>\n", pad, heading));
                kml.push_str(&format!("{}    <tilt>{}</tilt>\n", pad, tilt));
                kml.push_str(&format!("{}    <roll>{}</roll>\n", pad, roll));
                kml.push_str(&format!("{}  </Orientation>\n", pad));
                kml.push_str(&format!("{}  <Scale>\n", pad));
                kml.push_str(&format!("{}    <x>{}</x><y>{}</y><z>{}</z>\n", pad, scale, scale, scale));
                kml.push_str(&format!("{}  </Scale>\n", pad));
                kml.push_str(&format!("{}  <Link><href>{}</href></Link>\n", pad, href));
                kml.push_str(&format!("{}</Model>\n", pad));
                kml
            }
        }
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 将坐标格式化为 KML 坐标字符串。
fn format_coordinates(coords: &[[f64; 3]]) -> String {
    // 每个点格式化为 "经度,纬度,高度"，点之间以空格分隔（KML 坐标约定）
    coords
        .iter()
        .map(|c| format!("{},{},{}", c[0], c[1], c[2]))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 转义 XML 特殊字符。
fn escape_xml(s: &str) -> String {
    // 依次替换 XML 五个保留字符，注意 & 必须最先替换以避免二次转义
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// 将 RGBA 颜色转换为 KML 颜色格式（aabbggrr）。
pub fn rgba_to_kml_color(r: f64, g: f64, b: f64, a: f64) -> String {
    // KML 采用 aabbggrr 逆序：alpha、蓝、绿、红各占一个字节，分量 [0,1] 映射到 [0,255]
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        (a * 255.0) as u8,
        (b * 255.0) as u8,
        (g * 255.0) as u8,
        (r * 255.0) as u8
    )
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kml_export_options_default() {
        let opts = KmlExportOptions::default();
        assert_eq!(opts.name, "Cesium Export");
        assert_eq!(opts.kml_version, "2.2");
        assert!(opts.export_model);
        assert!(opts.export_images);
    }

    #[test]
    fn test_kml_exporter_basic() {
        let exporter = KmlExporter::new();
        let kml = exporter.to_kml();

        assert!(kml.contains("<?xml version=\"1.0\""));
        assert!(kml.contains("<kml"));
        assert!(kml.contains("xmlns=\"http://www.opengis.net/kml/2.2\""));
        assert!(kml.contains("<Document>"));
        assert!(kml.contains("<name>Cesium Export</name>"));
    }

    #[test]
    fn test_kml_exporter_with_placemark() {
        let mut exporter = KmlExporter::new();
        exporter.add_placemark(KmlExportPlacemark::new(
            "Test Point",
            KmlExportGeometry::Point {
                coordinates: vec![[-122.0, 37.0, 0.0]],
            },
        ));

        let kml = exporter.to_kml();
        assert!(kml.contains("<Placemark>"));
        assert!(kml.contains("<name>Test Point</name>"));
        assert!(kml.contains("<Point>"));
        assert!(kml.contains("-122,37,0"));
    }

    #[test]
    fn test_kml_export_style() {
        let style = KmlExportStyle {
            id: "style1".to_string(),
            icon_style: Some(KmlExportIconStyle {
                color: "ff0000ff".to_string(),
                scale: 1.5,
                icon_href: Some("icon.png".to_string()),
            }),
            line_style: None,
            poly_style: None,
            label_style: None,
        };

        let kml = style.to_kml(2);
        assert!(kml.contains("<Style id=\"style1\">"));
        assert!(kml.contains("<IconStyle>"));
        assert!(kml.contains("<color>ff0000ff</color>"));
        assert!(kml.contains("<scale>1.5</scale>"));
    }

    #[test]
    fn test_kml_export_polygon() {
        let geometry = KmlExportGeometry::Polygon {
            outer_boundary: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0],
            ],
            inner_boundaries: vec![],
        };

        let kml = geometry.to_kml(2);
        assert!(kml.contains("<Polygon>"));
        assert!(kml.contains("<outerBoundaryIs>"));
        assert!(kml.contains("<LinearRing>"));
    }

    #[test]
    fn test_kml_export_model() {
        let geometry = KmlExportGeometry::Model {
            href: "model.gltf".to_string(),
            location: [-122.0, 37.0, 100.0],
            heading: 45.0,
            tilt: 0.0,
            roll: 0.0,
            scale: 1.0,
        };

        let kml = geometry.to_kml(2);
        assert!(kml.contains("<Model>"));
        assert!(kml.contains("<Location>"));
        assert!(kml.contains("<longitude>-122</longitude>"));
        assert!(kml.contains("<href>model.gltf</href>"));
    }

    #[test]
    fn test_rgba_to_kml_color() {
        // 红色，完全不透明
        assert_eq!(rgba_to_kml_color(1.0, 0.0, 0.0, 1.0), "ff0000ff");
        // 蓝色，半透明
        assert_eq!(rgba_to_kml_color(0.0, 0.0, 1.0, 0.5), "7fff0000");
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("a & b"), "a &amp; b");
        assert_eq!(escape_xml("<tag>"), "&lt;tag&gt;");
    }

    #[test]
    fn test_kml_export_result() {
        let exporter = KmlExporter::new();
        let result = exporter.export();

        assert!(!result.kml.is_empty());
        assert!(result.external_files.is_empty());
        assert!(result.kmz.is_none());
    }
}
