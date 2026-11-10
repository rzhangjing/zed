//! GeoJSON 数据源解析。
//!
//! 本模块把 GeoJSON（RFC 7946）文本解析为内部数据源与实体集合：
//! 顶层可为单个 feature、feature 集合或裸几何对象，逐层递归展开为
//! 点、折线、多边形三类图形；每个 feature 的属性会附带在实体上，
//! 名称从常见的 name/NAME/title 等字段中嗅探，缺省时按序号生成 id。

use crate::entity::{Entity, PointGraphics, PolygonGraphics, PolylineGraphics};
use crate::entity_collection::DataSource;
use crate::property::{Color, Property};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// GeoJSON 解析错误。
#[derive(Debug, Error)]
pub enum GeoJsonError {
    /// JSON 解析错误。
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    /// 不支持的几何类型。
    #[error("Unsupported geometry type: {0}")]
    UnsupportedGeometry(String),

    /// 无效的坐标。
    #[error("Invalid coordinate at index {0}")]
    InvalidCoordinate(usize),
}

/// 一个 GeoJSON 对象（顶层）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum GeoJson {
    /// 单个 feature。
    Feature(Feature),
    /// feature 的集合。
    FeatureCollection(FeatureCollection),
    /// Point 几何对象。
    Point(PointGeometry),
    /// MultiPoint 几何。
    MultiPoint(MultiPointGeometry),
    /// LineString 几何。
    LineString(LineStringGeometry),
    /// MultiLineString 几何。
    MultiLineString(MultiLineStringGeometry),
    /// Polygon 几何。
    Polygon(PolygonGeometry),
    /// MultiPolygon 几何。
    MultiPolygon(MultiPolygonGeometry),
}

/// 一个 GeoJSON FeatureCollection。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureCollection {
    /// 此集合中的 features。
    pub features: Vec<Feature>,
}

/// 一个 GeoJSON Feature。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feature {
    /// 几何（可为 null）。
    pub geometry: Option<Geometry>,
    /// feature 属性。
    #[serde(default)]
    pub properties: serde_json::Value,
    /// feature ID。
    #[serde(default)]
    pub id: Option<serde_json::Value>,
}

/// 一个 GeoJSON Geometry。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Geometry {
    /// Point 几何。
    Point(PointGeometry),
    /// MultiPoint 几何。
    MultiPoint(MultiPointGeometry),
    /// LineString 几何。
    LineString(LineStringGeometry),
    /// MultiLineString 几何。
    MultiLineString(MultiLineStringGeometry),
    /// Polygon 几何。
    Polygon(PolygonGeometry),
    /// MultiPolygon 几何。
    MultiPolygon(MultiPolygonGeometry),
    /// GeometryCollection。
    GeometryCollection(GeometryCollection),
}

/// Point 几何。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointGeometry {
    /// [经度, 纬度, 可选高度]
    pub coordinates: Vec<f64>,
}

/// MultiPoint 几何。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiPointGeometry {
    /// 位置数组。
    pub coordinates: Vec<Vec<f64>>,
}

/// LineString 几何。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineStringGeometry {
    /// 位置数组。
    pub coordinates: Vec<Vec<f64>>,
}

/// MultiLineString 几何。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiLineStringGeometry {
    /// LineString 坐标数组的数组。
    pub coordinates: Vec<Vec<Vec<f64>>>,
}

/// Polygon 几何。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolygonGeometry {
    /// 环的数组（第一个为外环，其余为内孔）。
    pub coordinates: Vec<Vec<Vec<f64>>>,
}

/// MultiPolygon 几何。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiPolygonGeometry {
    /// Polygon 坐标数组的数组。
    pub coordinates: Vec<Vec<Vec<Vec<f64>>>>,
}

/// GeometryCollection。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeometryCollection {
    /// 此集合中的几何。
    pub geometries: Vec<Geometry>,
}

/// GeoJSON 加载的选项。
#[derive(Debug, Clone)]
pub struct GeoJsonOptions {
    /// 点的默认标记颜色。
    pub marker_color: Color,
    /// 默认标记尺寸（像素）。
    pub marker_size: f64,
    /// 线/轮廓的默认描边颜色。
    pub stroke: Color,
    /// 默认描边宽度。
    pub stroke_width: f64,
    /// 多边形的默认填充颜色。
    pub fill: Color,
    /// 是否贴合地面。
    pub clamp_to_ground: bool,
}

impl Default for GeoJsonOptions {
    /// 构造缺省加载选项：红色标记、黄色描边、半透明黄填充、不贴地。
    fn default() -> Self {
        Self {
            marker_color: Color::RED,
            marker_size: 8.0,
            stroke: Color::YELLOW,
            stroke_width: 2.0,
            fill: Color::new(1.0, 1.0, 0.0, 0.5), // 半透明黄色
            clamp_to_ground: false,
        }
    }
}

/// 将 GeoJSON 字符串解析为 DataSource。
///
/// 先用 serde 反序列化顶层对象，再交由 `process_geojson` 递归展开；
/// 共享一个自增计数器为无显式 id 的几何生成实体编号，最后标记已加载。
pub fn parse_geojson(json: &str, options: &GeoJsonOptions) -> Result<DataSource, GeoJsonError> {
    let geojson: GeoJson = serde_json::from_str(json)?;
    // 创建以 GeoJSON 命名的数据源，实体将逐步加入其集合。
    let mut ds = DataSource::new("GeoJSON");

    // 自增计数器为无显式 id 的几何分配实体编号。
    let mut id_counter = 0u64;
    process_geojson(&geojson, options, &mut ds, &mut id_counter)?;

    // 全部处理完毕，标记数据源为已加载。
    ds.loaded = true;
    Ok(ds)
}

/// 递归地处理一个 GeoJSON 对象。
///
/// 集合类型逐一分派给 `process_feature` 或直接创建实体；单几何类型
/// 按点/多点/线/多边形各分支创建对应图形，并在创建后递增 id 计数器。
fn process_geojson(
    geojson: &GeoJson,
    options: &GeoJsonOptions,
    ds: &mut DataSource,
    id_counter: &mut u64,
) -> Result<(), GeoJsonError> {
    match geojson {
        GeoJson::FeatureCollection(fc) => {
            // 集合逐个展开，每个 feature 递归进入处理。
            for feature in &fc.features {
                process_feature(feature, options, ds, id_counter)?;
            }
        }
        GeoJson::Feature(feature) => {
            process_feature(feature, options, ds, id_counter)?;
        }
        GeoJson::Point(pt) => {
            let entity = create_point_entity(*id_counter, &pt.coordinates, None, options);
            ds.entities.add(entity);
            *id_counter += 1;
        }
        GeoJson::MultiPoint(mpt) => {
            for coord in &mpt.coordinates {
                let entity = create_point_entity(*id_counter, coord, None, options);
                ds.entities.add(entity);
                *id_counter += 1;
            }
        }
        GeoJson::LineString(ls) => {
            let entity = create_polyline_entity(*id_counter, &ls.coordinates, None, options);
            ds.entities.add(entity);
            *id_counter += 1;
        }
        GeoJson::MultiLineString(mls) => {
            for line in &mls.coordinates {
                let entity = create_polyline_entity(*id_counter, line, None, options);
                ds.entities.add(entity);
                *id_counter += 1;
            }
        }
        GeoJson::Polygon(poly) => {
            let entity = create_polygon_entity(*id_counter, &poly.coordinates, None, options);
            ds.entities.add(entity);
            *id_counter += 1;
        }
        GeoJson::MultiPolygon(mpoly) => {
            for polygon in &mpoly.coordinates {
                let entity = create_polygon_entity(*id_counter, polygon, None, options);
                ds.entities.add(entity);
                *id_counter += 1;
            }
        }
    }
    Ok(())
}

/// 处理一个 GeoJSON feature。
///
/// 先取出几何（为空则直接跳过），从属性中嗅探名称与属性表，再按
/// 几何具体种类创建点/线/面实体；GeometryCollection 则逐项递归。
fn process_feature(
    feature: &Feature,
    options: &GeoJsonOptions,
    ds: &mut DataSource,
    id_counter: &mut u64,
) -> Result<(), GeoJsonError> {
    let geometry = match &feature.geometry {
        Some(g) => g,
        None => return Ok(()), // 跳过没有几何的 feature
    };

    // 从属性中嗅探名称，并抽取完整属性表随实体附带。
    let name = extract_name(&feature.properties);
    let properties = extract_properties(&feature.properties);

    // 依几何具体种类创建对应的点/线/面实体。
    match geometry {
        Geometry::Point(pt) => {
            let entity = create_point_entity(*id_counter, &pt.coordinates, name.as_deref(), options)
                .with_properties(properties);
            ds.entities.add(entity);
            *id_counter += 1;
        }
        Geometry::MultiPoint(mpt) => {
            for coord in &mpt.coordinates {
                let entity =
                    create_point_entity(*id_counter, coord, name.as_deref(), options);
                ds.entities.add(entity);
                *id_counter += 1;
            }
        }
        Geometry::LineString(ls) => {
            let entity =
                create_polyline_entity(*id_counter, &ls.coordinates, name.as_deref(), options)
                    .with_properties(properties);
            ds.entities.add(entity);
            *id_counter += 1;
        }
        Geometry::MultiLineString(mls) => {
            for line in &mls.coordinates {
                let entity =
                    create_polyline_entity(*id_counter, line, name.as_deref(), options);
                ds.entities.add(entity);
                *id_counter += 1;
            }
        }
        Geometry::Polygon(poly) => {
            let entity =
                create_polygon_entity(*id_counter, &poly.coordinates, name.as_deref(), options)
                    .with_properties(properties);
            ds.entities.add(entity);
            *id_counter += 1;
        }
        Geometry::MultiPolygon(mpoly) => {
            for polygon in &mpoly.coordinates {
                let entity =
                    create_polygon_entity(*id_counter, polygon, name.as_deref(), options);
                ds.entities.add(entity);
                *id_counter += 1;
            }
        }
        Geometry::GeometryCollection(gc) => {
            for geom in &gc.geometries {
                let feature = Feature {
                    geometry: Some(geom.clone()),
                    properties: feature.properties.clone(),
                    id: feature.id.clone(),
                };
                process_feature(&feature, options, ds, id_counter)?;
            }
        }
    }
    Ok(())
}

/// 将一个 GeoJSON 位置 [lon_deg, lat_deg, alt?] 转换为弧度 [lon_rad, lat_rad, height]。
///
/// 经度、纬度按度转弧度，高度视为米直接保留；坐标不足三个时以 0 补齐。
fn position_to_radians(coord: &[f64]) -> [f64; 3] {
    // 缺失的分量以 0 兜底，高度视为米不做转换。
    let lon = coord.first().copied().unwrap_or(0.0).to_radians();
    let lat = coord.get(1).copied().unwrap_or(0.0).to_radians();
    let height = coord.get(2).copied().unwrap_or(0.0);
    [lon, lat, height]
}

/// 由一个 GeoJSON 位置创建点实体。
///
/// 坐标先转为弧度并作为常量位置，再以选项里的标记颜色与尺寸
/// 构造点图形；若传入名称则附加到实体上。
fn create_point_entity(
    id: u64,
    coord: &[f64],
    name: Option<&str>,
    options: &GeoJsonOptions,
) -> Entity {
    // 先把坐标转为弧度三元组，再作为常量位置挂到实体。
    let pos = position_to_radians(coord);
    let mut entity = Entity::new(format!("geojson-{}", id))
        // 以弧度坐标作常量位置，并附带上标记颜色的点图形。
        .with_position(pos[0], pos[1], pos[2])
        .with_point(PointGraphics {
            color: Property::Constant(options.marker_color),
            pixel_size: Property::Constant(options.marker_size),
            ..Default::default()
        });

    if let Some(n) = name {
        entity = entity.with_name(n);
    }
    entity
}

/// 由 GeoJSON 坐标创建一个 polyline 实体。
///
/// 将整组坐标逐个转为弧度位置存为常量序列，宽度/颜色/是否贴地均取自
/// 选项；若传入名称则附加到实体上。
fn create_polyline_entity(
    id: u64,
    coords: &[Vec<f64>],
    name: Option<&str>,
    options: &GeoJsonOptions,
) -> Entity {
    // 逐坐标转弧度并收集为常量位置序列。
    let positions: Vec<[f64; 3]> = coords.iter().map(|c| position_to_radians(c)).collect();

    // 组装折线图形：位置/宽度/颜色/贴地均以常量属性写入。
    let mut entity = Entity::new(format!("geojson-{}", id)).with_polyline(PolylineGraphics {
        positions: Property::Constant(positions),
        width: Property::Constant(options.stroke_width),
        color: Property::Constant(options.stroke),
        clamp_to_ground: Property::Constant(options.clamp_to_ground),
        ..Default::default()
    });

    if let Some(n) = name {
        entity = entity.with_name(n);
    }
    entity
}

/// 由 GeoJSON 坐标创建一个多边形实体。
///
/// 取环数组的首个为外环、其余为内孔，均转为弧度位置；填充/轮廓颜色与
/// 宽度取自选项，并固定开启轮廓；若传入名称则附加到实体上。
fn create_polygon_entity(
    id: u64,
    rings: &[Vec<Vec<f64>>],
    name: Option<&str>,
    options: &GeoJsonOptions,
) -> Entity {
    // 首个环作外环，转为弧度位置。
    let exterior: Vec<[f64; 3]> = rings
        .first()
        .map(|ring| ring.iter().map(|c| position_to_radians(c)).collect())
        .unwrap_or_default();

    // 其余环均作为内孔，逐环转为弧度位置。
    let holes: Vec<Vec<[f64; 3]>> = rings
        .iter()
        .skip(1)
        .map(|ring| ring.iter().map(|c| position_to_radians(c)).collect())
        .collect();

    // 组装多边形图形：外环/内孔/填充与轮廓颜色均以常量属性写入。
    let mut entity = Entity::new(format!("geojson-{}", id)).with_polygon(PolygonGraphics {
        positions: Property::Constant(exterior),
        holes,
        material: Property::Constant(options.fill),
        outline: Property::Constant(true),
        outline_color: Property::Constant(options.stroke),
        outline_width: Property::Constant(options.stroke_width),
        ..Default::default()
    });

    if let Some(n) = name {
        entity = entity.with_name(n);
    }
    entity
}

/// 从 GeoJSON 属性中提取名称。
///
/// 仅当属性为对象时，依次尝试常见的 name/NAME/Name/title/TITLE 字段，
/// 命中第一个字符串值即返回；否则返回 `None`。
fn extract_name(properties: &serde_json::Value) -> Option<String> {
    if let Some(obj) = properties.as_object() {
        // 尝试常见的名称字段
        for key in &["name", "NAME", "Name", "title", "TITLE"] {
            if let Some(serde_json::Value::String(s)) = obj.get(*key) {
                return Some(s.clone());
            }
        }
    }
    None
}

/// 将所有属性提取为一个 HashMap。
///
/// 属性为对象时逐键值克隆入表，否则返回空表，供实体附带展示。
fn extract_properties(
    properties: &serde_json::Value,
) -> std::collections::HashMap<String, serde_json::Value> {
    // 仅对象型属性可展开为键值表，否则返回空表。
    match properties.as_object() {
        Some(obj) => obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        None => std::collections::HashMap::new(),
    }
}

/// 供 Entity 批量添加属性的辅助 trait。
trait EntityExt {
    /// 批量挂载属性表并返回自身，供链式调用。
    fn with_properties(
        self,
        properties: std::collections::HashMap<String, serde_json::Value>,
    ) -> Self;
}

impl EntityExt for Entity {
    /// 将解析得到的属性 HashMap 写入实体的 properties 字段。
    fn with_properties(
        mut self,
        properties: std::collections::HashMap<String, serde_json::Value>,
    ) -> Self {
        self.properties = properties;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证单个 Point feature 解析：恰好生成一个带名称且含点图形的实体。
    /// 名称取自 properties 的 name 字段，坐标转为弧度常量位置。
    #[test]
    fn test_parse_point() {
        let json = r#"{
            "type": "Feature",
            "geometry": {
                "type": "Point",
                "coordinates": [102.0, 0.5]
            },
            "properties": {
                "name": "Test Point"
            }
        }"#;

        let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
        assert_eq!(ds.entities.len(), 1);

        let entity = ds.entities.values().next().unwrap();
        assert_eq!(entity.name, Some("Test Point".to_string()));
        assert!(entity.point.is_some());
    }

    /// 验证 LineString feature 解析：生成一个含折线图形的实体，
    /// 三个端点坐标转为长度为 3 的位置序列。
    #[test]
    fn test_parse_linestring() {
        let json = r#"{
            "type": "Feature",
            "geometry": {
                "type": "LineString",
                "coordinates": [[102.0, 0.0], [103.0, 1.0], [104.0, 0.0]]
            },
            "properties": {}
        }"#;

        let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
        assert_eq!(ds.entities.len(), 1);

        let entity = ds.entities.values().next().unwrap();
        assert!(entity.polyline.is_some());
        let positions = entity.polyline.as_ref().unwrap().positions.get_value(0.0).unwrap();
        assert_eq!(positions.len(), 3);
    }

    /// 验证 Polygon feature 解析：生成一个含多边形图形的实体，
    /// 并从属性中正确取到名称 Test Polygon。
    #[test]
    fn test_parse_polygon() {
        let json = r#"{
            "type": "Feature",
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[100.0, 0.0], [101.0, 0.0], [101.0, 1.0], [100.0, 1.0], [100.0, 0.0]]]
            },
            "properties": {"name": "Test Polygon"}
        }"#;

        let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
        assert_eq!(ds.entities.len(), 1);

        let entity = ds.entities.values().next().unwrap();
        assert!(entity.polygon.is_some());
        assert_eq!(entity.name, Some("Test Polygon".to_string()));
    }

    /// 验证 FeatureCollection 解析：两个点 feature 展开为两个实体，
    /// 编号沿 id 计数器递增以保证唯一。
    #[test]
    fn test_parse_feature_collection() {
        let json = r#"{
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "geometry": {"type": "Point", "coordinates": [0.0, 0.0]},
                    "properties": {}
                },
                {
                    "type": "Feature",
                    "geometry": {"type": "Point", "coordinates": [1.0, 1.0]},
                    "properties": {}
                }
            ]
        }"#;

        let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
        assert_eq!(ds.entities.len(), 2);
    }

    /// 验证带孔多边形解析：两个环中首个作外环、余下作内孔，
    /// 因此 holes 长度为 1 且内孔含 5 个坐标点。
    #[test]
    fn test_parse_polygon_with_hole() {
        let json = r#"{
            "type": "Feature",
            "geometry": {
                "type": "Polygon",
                "coordinates": [
                    [[100.0, 0.0], [101.0, 0.0], [101.0, 1.0], [100.0, 1.0], [100.0, 0.0]],
                    [[100.2, 0.2], [100.8, 0.2], [100.8, 0.8], [100.2, 0.8], [100.2, 0.2]]
                ]
            },
            "properties": {}
        }"#;

        let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
        let entity = ds.entities.values().next().unwrap();
        let polygon = entity.polygon.as_ref().unwrap();
        assert_eq!(polygon.holes.len(), 1);
        assert_eq!(polygon.holes[0].len(), 5);
    }

    /// 验证坐标度转弧度：180/90 度分别对应 π 与 π/2，高度 1000 原样保留。
    /// 确保 position_to_radians 只转换经纬度而不改动米制高度。
    #[test]
    fn test_position_to_radians() {
        let pos = position_to_radians(&[180.0, 90.0, 1000.0]);
        assert!((pos[0] - std::f64::consts::PI).abs() < 1e-10);
        assert!((pos[1] - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
        assert!((pos[2] - 1000.0).abs() < 1e-10);
    }

    /// 验证自定义选项生效：标记色/尺寸取自 options 而非缺省值，
    /// 断言蓝色通道为 1 且像素尺寸等于传入的 20。
    #[test]
    fn test_custom_options() {
        let json = r#"{
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [0.0, 0.0]},
            "properties": {}
        }"#;

        let options = GeoJsonOptions {
            marker_color: Color::BLUE,
            marker_size: 20.0,
            ..Default::default()
        };

        let ds = parse_geojson(json, &options).unwrap();
        let entity = ds.entities.values().next().unwrap();
        let point = entity.point.as_ref().unwrap();
        let color = point.color.get_value(0.0).unwrap();
        assert!((color.blue - 1.0).abs() < 1e-10);
        let size = point.pixel_size.get_value(0.0).unwrap();
        assert!((*size - 20.0).abs() < 1e-10);
    }
}
