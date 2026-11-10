//! GPX（GPS Exchange Format）解析器。
//!
//! 提供 GPX 1.x 文档的轻量级解析能力，覆盖三类地理要素：
//! - 航点（waypoint，`<wpt>`）：离散的兴趣点，携带坐标、高程与元信息；
//! - 轨迹（track，`<trk>`）：按 `<trkseg>` 分段记录的连续定位序列；
//! - 航线（route，`<rte>`）：由若干 `<rtept>` 航路点连成的规划路径。
//!
//! 解析得到的 [`GpxDocument`] 可经 [`gpx_to_datasource`] 转换为通用
//! [`DataSource`]：航点渲染为点要素，轨迹与航线渲染为折线要素。

use cesium_datasource::entity::{Entity, PointGraphics, PolylineGraphics};
use cesium_datasource::entity_collection::DataSource;
use cesium_datasource::property::{Color, Property};
use cesium_geospatial::cartographic::Cartographic;

/// 一个 GPX 航点（`<wpt>`）。
///
/// 表示一个独立的地理兴趣点：`latitude`/`longitude` 为 WGS84 经纬度（度），
/// 可选字段承载高程、时间戳与展示用的名称/描述等元信息。
#[derive(Debug, Clone, PartialEq)]
pub struct GpxWaypoint {
    /// 纬度（度）。
    pub latitude: f64,
    /// 经度（度）。
    pub longitude: f64,
    /// 高程（米）。
    pub elevation: Option<f64>,
    /// 时间戳（ISO 8601）。
    pub time: Option<String>,
    /// 名称。
    pub name: Option<String>,
    /// 备注。
    pub comment: Option<String>,
    /// 描述。
    pub description: Option<String>,
    /// 符号名称。
    pub symbol: Option<String>,
    /// 类型。
    pub waypoint_type: Option<String>,
}

impl GpxWaypoint {
    /// 创建一个新航点，除经纬度外的可选字段全部置为 `None`。
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
            elevation: None,
            time: None,
            name: None,
            comment: None,
            description: None,
            symbol: None,
            waypoint_type: None,
        }
    }

    /// 转换为 [`Cartographic`]。
    ///
    /// 输入经纬度以「度」存储，此处转为弧度并按 (longitude, latitude, height)
    /// 顺序构造；`elevation` 缺省视为 0.0 米。
    pub fn to_cartographic(&self) -> Cartographic {
        Cartographic::from_radians(
            self.longitude.to_radians(),
            self.latitude.to_radians(),
            self.elevation.unwrap_or(0.0),
        )
    }
}

/// 一个 GPX 轨迹（`<trk>`）。
///
/// 由元信息与若干 `<trkseg>` 轨迹段组成；分段用于表达采集中断造成的不连续。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpxTrack {
    /// 轨迹名称。
    pub name: Option<String>,
    /// 轨迹备注。
    pub comment: Option<String>,
    /// 轨迹描述。
    pub description: Option<String>,
    /// 轨迹段。
    pub segments: Vec<GpxTrackSegment>,
}

/// 一个 GPX 轨迹段（`<trkseg>`）。
///
/// 一段连续的轨迹点序列，段内各点按时序相邻。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpxTrackSegment {
    /// 轨迹点。
    pub points: Vec<GpxTrackPoint>,
}

/// 一个 GPX 轨迹点（`<trkpt>`）。
///
/// 轨迹上的单次定位采样，`lat`/`lon` 必填，`ele`/`time` 可选。
#[derive(Debug, Clone, PartialEq)]
pub struct GpxTrackPoint {
    /// 纬度（度）。
    pub latitude: f64,
    /// 经度（度）。
    pub longitude: f64,
    /// 高程（米）。
    pub elevation: Option<f64>,
    /// 时间戳（ISO 8601）。
    pub time: Option<String>,
}

impl GpxTrackPoint {
    /// 创建一个新的轨迹点，`elevation`/`time` 默认为 `None`。
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
            elevation: None,
            time: None,
        }
    }

    /// 转换为 [`Cartographic`]。
    ///
    /// 输入经纬度以「度」存储，此处转为弧度并按 (longitude, latitude, height)
    /// 顺序构造；`elevation` 缺省视为 0.0 米。
    pub fn to_cartographic(&self) -> Cartographic {
        Cartographic::from_radians(
            self.longitude.to_radians(),
            self.latitude.to_radians(),
            self.elevation.unwrap_or(0.0),
        )
    }
}

/// 一个 GPX 航线（`<rte>`）。
///
/// 由若干航路点组成的**规划**路径（区别于实时采集的轨迹），含元信息与点序列。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpxRoute {
    /// 航线名称。
    pub name: Option<String>,
    /// 航线备注。
    pub comment: Option<String>,
    /// 航线描述。
    pub description: Option<String>,
    /// 航线点。
    pub points: Vec<GpxRoutePoint>,
}

/// 一个 GPX 航线点（`<rtept>`）。
///
/// 航线上的一个航路点（检查点/途经点），`lat`/`lon` 必填，`ele`/`name` 可选。
#[derive(Debug, Clone, PartialEq)]
pub struct GpxRoutePoint {
    /// 纬度（度）。
    pub latitude: f64,
    /// 经度（度）。
    pub longitude: f64,
    /// 高程（米）。
    pub elevation: Option<f64>,
    /// 名称。
    pub name: Option<String>,
}

impl GpxRoutePoint {
    /// 创建一个新的航线点，`elevation`/`name` 默认为 `None`。
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
            elevation: None,
            name: None,
        }
    }

    /// 转换为 [`Cartographic`]。
    ///
    /// 输入经纬度以「度」存储，此处转为弧度并按 (longitude, latitude, height)
    /// 顺序构造；`elevation` 缺省视为 0.0 米。
    pub fn to_cartographic(&self) -> Cartographic {
        Cartographic::from_radians(
            self.longitude.to_radians(),
            self.latitude.to_radians(),
            self.elevation.unwrap_or(0.0),
        )
    }
}

/// GPX 元数据。
///
/// 描述整个 GPX 文档级别的属性（名称、描述、作者、创建时间、关键词）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpxMetadata {
    /// 文档名称。
    pub name: Option<String>,
    /// 文档描述。
    pub description: Option<String>,
    /// 作者名称。
    pub author: Option<String>,
    /// 创建时间（ISO 8601）。
    pub time: Option<String>,
    /// 关键词。
    pub keywords: Option<String>,
}

/// 一个 GPX 文档。
///
/// 解析结果的顶层容器，聚合元数据与三类要素集合。
#[derive(Debug, Clone, Default)]
pub struct GpxDocument {
    /// 元数据。
    pub metadata: GpxMetadata,
    /// 航点。
    pub waypoints: Vec<GpxWaypoint>,
    /// 轨迹。
    pub tracks: Vec<GpxTrack>,
    /// 航线。
    pub routes: Vec<GpxRoute>,
}

/// 将一个 GPX 文档转换为通用 [`DataSource`]。
///
/// 映射规则：每个 `<wpt>` 生成一个带红色点符号的 entity；每个 `<trkseg>`
/// 生成一条蓝色折线（轨迹）；每条 `<rte>` 生成一条绿色折线（航线）。
/// entity 名按类型加索引自动编号，几何位置统一把「度」转成「弧度」。
pub fn gpx_to_datasource(doc: &GpxDocument) -> DataSource {
    // 数据源名取元数据 name，缺省回退为字面量 "GPX"。
    let name = doc
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| "GPX".to_string());
    let mut ds = DataSource::new(name);

    // 将航点作为 point 添加：固定红色、像素尺寸 8，位置三元组为 (lon, lat, height) 弧度。
    for (i, wpt) in doc.waypoints.iter().enumerate() {
        let mut entity = Entity::new(format!("waypoint_{}", i));
        entity.name = wpt.name.clone();

        entity.position = Property::Constant([
            wpt.longitude.to_radians(),
            wpt.latitude.to_radians(),
            wpt.elevation.unwrap_or(0.0),
        ]);

        entity.point = Some(PointGraphics {
            color: Property::Constant(Color::RED),
            pixel_size: Property::Constant(8.0),
            ..Default::default()
        });

        ds.entities.add(entity);
    }

    // 将轨迹作为折线添加：逐段（trkseg）产出一条蓝色折线，顶点为该段各 trkpt 的弧度坐标
    for (i, track) in doc.tracks.iter().enumerate() {
        for (j, segment) in track.segments.iter().enumerate() {
            let mut entity = Entity::new(format!("track_{}_{}", i, j));
            entity.name = track.name.clone();

            // 逐点转弧度组装顶点数组，顺序为 (lon, lat, height)，与 entity 位置约定一致
            let positions: Vec<[f64; 3]> = segment
                .points
                .iter()
                .map(|p| {
                    [
                        p.longitude.to_radians(),
                        p.latitude.to_radians(),
                        p.elevation.unwrap_or(0.0),
                    ]
                })
                .collect();

            entity.polyline = Some(PolylineGraphics {
                positions: Property::Constant(positions),
                width: Property::Constant(3.0),
                color: Property::Constant(Color::BLUE),
                ..Default::default()
            });

            ds.entities.add(entity);
        }
    }

    // 将航线作为折线添加：整条 rte 产出一条绿色折线，顶点为各 rtept 的弧度坐标
    for (i, route) in doc.routes.iter().enumerate() {
        let mut entity = Entity::new(format!("route_{}", i));
        entity.name = route.name.clone();

        // 航线顶点同样转弧度并按 (lon, lat, height) 排列，整条 rte 合成单条折线
        let positions: Vec<[f64; 3]> = route
            .points
            .iter()
            .map(|p| {
                [
                    p.longitude.to_radians(),
                    p.latitude.to_radians(),
                    p.elevation.unwrap_or(0.0),
                ]
            })
            .collect();

        entity.polyline = Some(PolylineGraphics {
            positions: Property::Constant(positions),
            width: Property::Constant(3.0),
            color: Property::Constant(Color::GREEN),
            ..Default::default()
        });

        ds.entities.add(entity);
    }

    ds
}

/// 简单的 GPX 解析器（基础实现）。
///
/// 基于字符串扫描的宽容解析，不依赖完整 XML 库：只识别 GPX 关心的标签，
/// 适合结构规整的 GPX 文件；遇到缺必填字段的输入按 Err 返回而非静默纠正。
pub fn parse_gpx_simple(xml: &str) -> Result<GpxDocument, String> {
    let mut doc = GpxDocument::default();

    // 解析元数据：顶层只取首个 name/desc 作为文档名与描述
    if let Some(name) = extract_tag_content(xml, "name") {
        doc.metadata.name = Some(name);
    }
    if let Some(desc) = extract_tag_content(xml, "desc") {
        doc.metadata.description = Some(desc);
    }

    // 解析航点
    for wpt_xml in extract_all_tags(xml, "wpt") {
        let wpt = parse_waypoint(&wpt_xml)?;
        doc.waypoints.push(wpt);
    }

    // 解析轨迹
    for trk_xml in extract_all_tags(xml, "trk") {
        let track = parse_track(&trk_xml)?;
        doc.tracks.push(track);
    }

    // 解析航线
    for rte_xml in extract_all_tags(xml, "rte") {
        let route = parse_route(&rte_xml)?;
        doc.routes.push(route);
    }

    Ok(doc)
}

/// 解析一个航点元素（`<wpt>`）。
///
/// `lat`/`lon` 属性为必填，缺失即返回 Err；其余为可选子标签，逐个填充。
fn parse_waypoint(xml: &str) -> Result<GpxWaypoint, String> {
    // 必填：纬度 lat（解析为 f64）
    let lat = extract_attribute(xml, "lat")
        .and_then(|s| s.parse().ok())
        .ok_or("Missing lat attribute")?;
    let lon = extract_attribute(xml, "lon")
        .and_then(|s| s.parse().ok())
        .ok_or("Missing lon attribute")?;

    let mut wpt = GpxWaypoint::new(lat, lon);
    // 可选子标签逐个填充：ele 需解析为 f64，解析失败（含缺失）归为 None；其余为纯文本。
    wpt.elevation = extract_tag_content(xml, "ele").and_then(|s| s.parse().ok());
    wpt.time = extract_tag_content(xml, "time");
    wpt.name = extract_tag_content(xml, "name");
    wpt.comment = extract_tag_content(xml, "cmt");
    wpt.description = extract_tag_content(xml, "desc");
    wpt.symbol = extract_tag_content(xml, "sym");
    wpt.waypoint_type = extract_tag_content(xml, "type");

    Ok(wpt)
}

/// 解析一个轨迹元素（`<trk>`）。
///
/// 一条轨迹可含多个 `<trkseg>` 分段；元信息 name/cmt/desc 取自 `<trk>` 直属子标签。
fn parse_track(xml: &str) -> Result<GpxTrack, String> {
    // 先逐段解析并收集所有轨迹段，再连同轨迹级元信息组装
    let mut segments = Vec::new();
    for seg_xml in extract_all_tags(xml, "trkseg") {
        let segment = parse_track_segment(&seg_xml)?;
        segments.push(segment);
    }

    // 组装轨迹：name/cmt/desc 取自 trk 直属子标签，segments 为上面逐段解析的结果
    Ok(GpxTrack {
        name: extract_tag_content(xml, "name"),
        comment: extract_tag_content(xml, "cmt"),
        description: extract_tag_content(xml, "desc"),
        segments,
    })
}

/// 解析一个轨迹段元素（`<trkseg>`）。
///
/// 段内每个 `<trkpt>` 需带 `lat`/`lon` 属性，可选 `ele`（高程）与 `time`（时间戳）。
fn parse_track_segment(xml: &str) -> Result<GpxTrackSegment, String> {
    let mut segment = GpxTrackSegment::default();

    // 段内每个 trkpt 都必须带 lat/lon，任一点缺必填即整体报错（fail-fast）。
    for pt_xml in extract_all_tags(xml, "trkpt") {
        let lat = extract_attribute(&pt_xml, "lat")
            .and_then(|s| s.parse().ok())
            .ok_or("Missing lat attribute")?;
        let lon = extract_attribute(&pt_xml, "lon")
            .and_then(|s| s.parse().ok())
            .ok_or("Missing lon attribute")?;

        let mut pt = GpxTrackPoint::new(lat, lon);
        pt.elevation = extract_tag_content(&pt_xml, "ele").and_then(|s| s.parse().ok());
        pt.time = extract_tag_content(&pt_xml, "time");

        segment.points.push(pt);
    }

    Ok(segment)
}

/// 解析一个航线元素（`<rte>`）。
///
/// 航线是一串有序的 `<rtept>` 航路点（规划路径，区别于实时采集的轨迹）。
fn parse_route(xml: &str) -> Result<GpxRoute, String> {
    // 逐个提取 rtept，再与航线级 name/cmt/desc 一起组装
    let mut points = Vec::new();
    for pt_xml in extract_all_tags(xml, "rtept") {
        let lat = extract_attribute(&pt_xml, "lat")
            .and_then(|s| s.parse().ok())
            .ok_or("Missing lat attribute")?;
        let lon = extract_attribute(&pt_xml, "lon")
            .and_then(|s| s.parse().ok())
            .ok_or("Missing lon attribute")?;

        let mut pt = GpxRoutePoint::new(lat, lon);
        pt.elevation = extract_tag_content(&pt_xml, "ele").and_then(|s| s.parse().ok());
        pt.name = extract_tag_content(&pt_xml, "name");

        points.push(pt);
    }

    Ok(GpxRoute {
        name: extract_tag_content(xml, "name"),
        comment: extract_tag_content(xml, "cmt"),
        description: extract_tag_content(xml, "desc"),
        points,
    })
}

/// 提取成对标签 `<tag>…</tag>` 之间的文本内容。
///
/// 只匹配**精确**的开标签 `<tag>`（不含带属性的变体），返回首个出现处的
/// 内部文本并去除首尾空白；任一端标签缺失都返回 `None`。
fn extract_tag_content(xml: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{}>", tag);
    let end_tag = format!("</{}>", tag);

    // 定位开标签之后的内容起点，再在其后查找闭标签
    let start = xml.find(&start_tag)? + start_tag.len();
    let end = xml[start..].find(&end_tag)? + start;

    // 截取的内部文本可能含换行/缩进，trim 去空白后再返回
    Some(xml[start..end].trim().to_string())
}

/// 提取形如 `<tag…>…</tag>`（或自闭合 `<tag…/>`）的所有出现，返回各元素的原始片段。
///
/// 采用向前扫描：从 `search_start` 起找下一个开标签，判断是否自闭合后推进行指针，
/// 因此能正确处理同一父元素下的多个兄弟元素（如多个 `<trkpt>`）。
fn extract_all_tags(xml: &str, tag: &str) -> Vec<String> {
    let mut results = Vec::new();
    let start_tag = format!("<{}", tag);
    let end_tag = format!("</{}>", tag);

    // 游标式向前扫描：每处理一个元素都把 search_start 推进到其后，避免重复匹配同一开标签。
    let mut search_start = 0;
    while let Some(start) = xml[search_start..].find(&start_tag) {
        let abs_start = search_start + start;
        // 定位本开标签的 '>'，据此判断该元素是自闭合还是成对出现。
        let tag_end = match xml[abs_start..].find('>') {
            Some(pos) => pos + abs_start,
            None => break,
        };
        if xml.as_bytes()[tag_end - 1] == b'/' {
            // 自闭合标签
            results.push(xml[abs_start..=tag_end].to_string());
            search_start = tag_end + 1;
        } else if let Some(end) = xml[abs_start..].find(&end_tag) {
            // 成对标签：截取含闭标签在内的完整原始片段，供后续递归解析其内部结构
            let abs_end = abs_start + end + end_tag.len();
            results.push(xml[abs_start..abs_end].to_string());
            search_start = abs_end;
        } else {
            // 找不到闭标签：输入畸形，停止扫描而不 panic
            break;
        }
    }

    results
}

/// 从一段 XML 文本中提取 `attr="…"` 形式的属性值（双引号定界）。
///
/// 以 `attr="` 为锚点定位起点，取到下一个 `"` 为止；未找到则返回 `None`。
fn extract_attribute(xml: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr);
    // 起点跳过整个 `attr="` 锚点；任一锚点缺失都由 `?` 短路返回 None。
    let start = xml.find(&pattern)? + pattern.len();
    let end = xml[start..].find('"')? + start;
    Some(xml[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpx_waypoint_creation() {
        let wpt = GpxWaypoint::new(37.7749, -122.4194);
        assert!((wpt.latitude - 37.7749).abs() < 1e-10);
        assert!((wpt.longitude - (-122.4194)).abs() < 1e-10);
        assert!(wpt.elevation.is_none());
    }

    #[test]
    fn test_gpx_waypoint_to_cartographic() {
        let mut wpt = GpxWaypoint::new(37.7749, -122.4194);
        wpt.elevation = Some(100.0);

        let carto = wpt.to_cartographic();
        assert!((carto.latitude - 37.7749_f64.to_radians()).abs() < 1e-10);
        assert!((carto.longitude - (-122.4194_f64.to_radians())).abs() < 1e-10);
        assert!((carto.height - 100.0).abs() < 1e-10);
    }

    #[test]
    fn test_gpx_track_point() {
        let mut pt = GpxTrackPoint::new(37.0, -122.0);
        pt.elevation = Some(50.0);
        pt.time = Some("2024-01-01T12:00:00Z".to_string());

        assert!((pt.latitude - 37.0).abs() < 1e-10);
        assert!(pt.elevation.is_some());
        assert!(pt.time.is_some());
    }

    #[test]
    fn test_gpx_route_point() {
        let mut pt = GpxRoutePoint::new(37.0, -122.0);
        pt.name = Some("Checkpoint".to_string());

        assert!((pt.latitude - 37.0).abs() < 1e-10);
        assert_eq!(pt.name, Some("Checkpoint".to_string()));
    }

    #[test]
    fn test_parse_gpx_simple_waypoint() {
        let gpx = r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1">
  <metadata>
    <name>Test GPX</name>
  </metadata>
  <wpt lat="37.7749" lon="-122.4194">
    <ele>10.5</ele>
    <name>San Francisco</name>
    <desc>A city</desc>
  </wpt>
</gpx>"#;

        let doc = parse_gpx_simple(gpx).unwrap();
        assert_eq!(doc.metadata.name, Some("Test GPX".to_string()));
        assert_eq!(doc.waypoints.len(), 1);

        let wpt = &doc.waypoints[0];
        assert!((wpt.latitude - 37.7749).abs() < 1e-10);
        assert!((wpt.longitude - (-122.4194)).abs() < 1e-10);
        assert!((wpt.elevation.unwrap() - 10.5).abs() < 1e-10);
        assert_eq!(wpt.name, Some("San Francisco".to_string()));
    }

    #[test]
    fn test_parse_gpx_simple_track() {
        let gpx = r#"<gpx>
  <trk>
    <name>Morning Run</name>
    <trkseg>
      <trkpt lat="37.0" lon="-122.0">
        <ele>10</ele>
      </trkpt>
      <trkpt lat="37.1" lon="-122.1">
        <ele>20</ele>
      </trkpt>
    </trkseg>
  </trk>
</gpx>"#;

        let doc = parse_gpx_simple(gpx).unwrap();
        assert_eq!(doc.tracks.len(), 1);

        let track = &doc.tracks[0];
        assert_eq!(track.name, Some("Morning Run".to_string()));
        assert_eq!(track.segments.len(), 1);
        assert_eq!(track.segments[0].points.len(), 2);
    }

    #[test]
    fn test_parse_gpx_simple_route() {
        let gpx = r#"<gpx>
  <rte>
    <name>Route 1</name>
    <rtept lat="37.0" lon="-122.0">
      <name>Start</name>
    </rtept>
    <rtept lat="37.5" lon="-122.5">
      <name>End</name>
    </rtept>
  </rte>
</gpx>"#;

        let doc = parse_gpx_simple(gpx).unwrap();
        assert_eq!(doc.routes.len(), 1);

        let route = &doc.routes[0];
        assert_eq!(route.name, Some("Route 1".to_string()));
        assert_eq!(route.points.len(), 2);
        assert_eq!(route.points[0].name, Some("Start".to_string()));
    }

    #[test]
    fn test_gpx_to_datasource() {
        let doc = GpxDocument {
            metadata: GpxMetadata {
                name: Some("Test".to_string()),
                ..Default::default()
            },
            waypoints: vec![GpxWaypoint::new(37.0, -122.0)],
            tracks: vec![GpxTrack {
                name: Some("Track".to_string()),
                segments: vec![GpxTrackSegment {
                    points: vec![
                        GpxTrackPoint::new(37.0, -122.0),
                        GpxTrackPoint::new(37.1, -122.1),
                    ],
                }],
                ..Default::default()
            }],
            routes: Vec::new(),
        };

        let ds = gpx_to_datasource(&doc);
        assert_eq!(ds.name, "Test");
        assert_eq!(ds.entities.len(), 2); // 1 个航点 + 1 个轨迹段
    }

    #[test]
    fn test_extract_tag_content() {
        let xml = "<name>Test Name</name>";
        assert_eq!(extract_tag_content(xml, "name"), Some("Test Name".to_string()));
    }

    #[test]
    fn test_extract_attribute() {
        let xml = r#"<wpt lat="37.0" lon="-122.0">"#;
        assert_eq!(extract_attribute(xml, "lat"), Some("37.0".to_string()));
        assert_eq!(extract_attribute(xml, "lon"), Some("-122.0".to_string()));
    }

    #[test]
    fn test_gpx_metadata_default() {
        let metadata = GpxMetadata::default();
        assert!(metadata.name.is_none());
        assert!(metadata.description.is_none());
        assert!(metadata.author.is_none());
    }

    #[test]
    fn test_gpx_document_default() {
        let doc = GpxDocument::default();
        assert!(doc.waypoints.is_empty());
        assert!(doc.tracks.is_empty());
        assert!(doc.routes.is_empty());
    }

    #[test]
    fn test_multiple_waypoints() {
        let gpx = r#"<gpx>
  <wpt lat="37.0" lon="-122.0"><name>W1</name></wpt>
  <wpt lat="38.0" lon="-123.0"><name>W2</name></wpt>
  <wpt lat="39.0" lon="-124.0"><name>W3</name></wpt>
</gpx>"#;

        let doc = parse_gpx_simple(gpx).unwrap();
        assert_eq!(doc.waypoints.len(), 3);
        assert_eq!(doc.waypoints[0].name, Some("W1".to_string()));
        assert_eq!(doc.waypoints[2].name, Some("W3".to_string()));
    }
}
