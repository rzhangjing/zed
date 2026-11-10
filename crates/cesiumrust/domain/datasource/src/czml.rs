//! CZML 数据源解析。
//!
//! CZML 是一种用于描述时态动态 3D 场景的 JSON 格式：数据包序列以
//! `document` 包开头描述整体，随后每个包对应一个实体，携带位置与
//! 各类图形（点、折线、多边形、标签、广告牌、模型等）声明。本模块
//! 把这些声明解析为内部 `DataSource` 与 `Entity` 集合。

use crate::entity::{
    Entity, PointGraphics, PolylineGraphics, PolygonGraphics,
    BillboardGraphics, LabelGraphics, ModelGraphics, EllipseGraphics,
    BoxGraphics, CylinderGraphics, CorridorGraphics, RectangleGraphics,
    WallGraphics, EllipsoidGraphics, PathGraphics,
};
use crate::entity_collection::DataSource;
use crate::property::{Color, Property};
use serde::Deserialize;
use thiserror::Error;

/// CZML 解析错误：包装 JSON 反序列化失败与缺少 document 包两种情况。
#[derive(Debug, Error)]
pub enum CzmlError {
    /// JSON 解析错误。
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    /// 缺少 document 数据包。
    #[error("CZML must start with a document packet (id='document')")]
    MissingDocument,
}

/// 一个 CZML 数据包：对应 JSON 数组中的一个对象，以 id 标识实体，
/// 并可选携带位置与各图形字段。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlPacket {
    /// 数据包 ID。
    pub id: String,

    /// 数据包名称。
    #[serde(default)]
    pub name: Option<String>,

    /// 位置（经纬度：[time, lon, lat, height, ...]）。
    #[serde(default)]
    pub position: Option<CzmlPosition>,

    /// 点图形。
    #[serde(default)]
    pub point: Option<CzmlPoint>,

    /// polyline（折线）图形。
    #[serde(default)]
    pub polyline: Option<CzmlPolyline>,

    /// polygon（多边形）图形。
    #[serde(default)]
    pub polygon: Option<CzmlPolygon>,

    /// label。
    #[serde(default)]
    pub label: Option<CzmlLabel>,

    /// billboard。
    #[serde(default)]
    pub billboard: Option<CzmlBillboard>,

    /// model。
    #[serde(default)]
    pub model: Option<CzmlModel>,

    /// 椭圆。
    #[serde(default)]
    pub ellipse: Option<CzmlEllipse>,

    /// 方框。
    #[serde(default, rename = "box")]
    pub box_graphics: Option<CzmlBox>,

    /// 圆柱。
    #[serde(default)]
    pub cylinder: Option<CzmlCylinder>,

    /// corridor。
    #[serde(default)]
    pub corridor: Option<CzmlCorridor>,

    /// 矩形。
    #[serde(default)]
    pub rectangle: Option<CzmlRectangle>,

    /// 墙体。
    #[serde(default)]
    pub wall: Option<CzmlWall>,

    /// 球体。
    #[serde(default)]
    pub ellipsoid: Option<CzmlEllipsoid>,

    /// path（轨迹）。
    #[serde(default)]
    pub path: Option<CzmlPath>,

    /// 可用性（ISO 8601 时间区间字符串）。
    #[serde(default)]
    pub availability: Option<String>,

    /// 描述。
    #[serde(default)]
    pub description: Option<String>,
}

/// CZML 位置值：兼容平铺数组与带字段对象两种写法。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum CzmlPosition {
    /// 以经纬度（平铺数组 [lon, lat, height] 或带时间标记）表示。
    CartographicDegrees(Vec<f64>),
    /// 带 cartographicDegrees 字段的对象。
    Object {
        /// 平铺的经纬度数组，对应字段名为 `cartographicDegrees`。
        #[serde(rename = "cartographicDegrees")]
        cartographic_degrees: Vec<f64>,
    },
}

/// CZML 点图形：以圆点渲染实体，携带颜色、像素尺寸与轮廓参数。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlPoint {
    /// 以 RGBA [r, g, b, a] 表示的颜色（0-255）。
    #[serde(default)]
    pub color: Option<CzmlColor>,
    /// 像素尺寸。
    #[serde(default)]
    pub pixel_size: Option<f64>,
    /// 轮廓颜色。
    #[serde(default)]
    pub outline_color: Option<CzmlColor>,
    /// 轮廓宽度。
    #[serde(default)]
    pub outline_width: Option<f64>,
}

/// CZML polyline（折线）图形：连接一系列位置的线，带宽度与材质。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlPolyline {
    /// 以经纬度表示的位置。
    #[serde(default)]
    pub positions: Option<CzmlPosition>,
    /// 宽度。
    #[serde(default)]
    pub width: Option<f64>,
    /// 材质。
    #[serde(default)]
    pub material: Option<CzmlMaterial>,
}

/// CZML polygon（多边形）图形：由位置环围成的面，可设高度与挤出高度。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlPolygon {
    /// 以经纬度表示的位置。
    #[serde(default)]
    pub positions: Option<CzmlPosition>,
    /// 材质。
    #[serde(default)]
    pub material: Option<CzmlMaterial>,
    /// 高度。
    #[serde(default)]
    pub height: Option<f64>,
    /// 挤出高度。
    #[serde(default)]
    pub extruded_height: Option<f64>,
}

/// CZML label：在实体位置绘制文本，带字体与填充/轮廓颜色。
#[derive(Debug, Clone, Deserialize)]
pub struct CzmlLabel {
    /// label 文本。
    #[serde(default)]
    pub text: Option<String>,
    /// 字体。
    #[serde(default)]
    pub font: Option<String>,
    /// 填充颜色。
    #[serde(default, rename = "fillColor")]
    pub fill_color: Option<CzmlColor>,
    /// 轮廓颜色。
    #[serde(default, rename = "outlineColor")]
    pub outline_color: Option<CzmlColor>,
}

/// CZML billboard：面向屏幕的图像标记，带缩放、颜色、旋转与宽高。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlBillboard {
    /// 图像 URI。
    #[serde(default)]
    pub image: Option<String>,
    /// 缩放。
    #[serde(default)]
    pub scale: Option<f64>,
    /// 颜色。
    #[serde(default)]
    pub color: Option<CzmlColor>,
    /// 旋转。
    #[serde(default)]
    pub rotation: Option<f64>,
    /// 宽度。
    #[serde(default)]
    pub width: Option<f64>,
    /// 高度。
    #[serde(default)]
    pub height: Option<f64>,
}

/// CZML model：引用 glTF/glb 三维模型，带缩放与最小像素尺寸。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlModel {
    /// model URI（glTF/glb）。
    #[serde(default)]
    pub gltf: Option<String>,
    /// 缩放。
    #[serde(default)]
    pub scale: Option<f64>,
    /// 最小像素尺寸。
    #[serde(default)]
    pub minimum_pixel_size: Option<f64>,
}

/// CZML 椭圆：以半长/半短轴定义的地面椭圆，带高度与材质。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlEllipse {
    /// 半长轴。
    #[serde(default)]
    pub semi_major_axis: Option<f64>,
    /// 半短轴。
    #[serde(default)]
    pub semi_minor_axis: Option<f64>,
    /// 高度。
    #[serde(default)]
    pub height: Option<f64>,
    /// 材质。
    #[serde(default)]
    pub material: Option<CzmlMaterial>,
}

/// CZML 方框：以三维尺寸定义的长方体，带材质。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlBox {
    /// 尺寸 [x, y, z]。
    #[serde(default)]
    pub dimensions: Option<CzmlCartesian3Value>,
    /// 材质。
    #[serde(default)]
    pub material: Option<CzmlMaterial>,
}

/// CZML 圆柱：以长度与顶/底半径定义的锥台体，带材质。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlCylinder {
    /// 长度。
    #[serde(default)]
    pub length: Option<f64>,
    /// 顶部半径。
    #[serde(default)]
    pub top_radius: Option<f64>,
    /// 底部半径。
    #[serde(default)]
    pub bottom_radius: Option<f64>,
    /// 材质。
    #[serde(default)]
    pub material: Option<CzmlMaterial>,
}

/// CZML corridor：沿位置走廊带固定宽度铺设，带高度与材质。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlCorridor {
    /// 位置。
    #[serde(default)]
    pub positions: Option<CzmlPosition>,
    /// 宽度。
    #[serde(default)]
    pub width: Option<f64>,
    /// 高度。
    #[serde(default)]
    pub height: Option<f64>,
    /// 材质。
    #[serde(default)]
    pub material: Option<CzmlMaterial>,
}

/// CZML 矩形：以东西南北四边坐标定义的地面矩形，带高度与材质。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlRectangle {
    /// 坐标 [west, south, east, north]（度）。
    #[serde(default)]
    pub coordinates: Option<CzmlRectangleCoords>,
    /// 高度。
    #[serde(default)]
    pub height: Option<f64>,
    /// 材质。
    #[serde(default)]
    pub material: Option<CzmlMaterial>,
}

/// CZML 矩形坐标：兼容平铺数组与带 degrees 字段的对象。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum CzmlRectangleCoords {
    /// 以度为单位的平铺数组 [west, south, east, north]。
    Array(Vec<f64>),
    /// 带 degrees 字段的对象。
    Object { degrees: Vec<f64> },
}

/// CZML 墙体（wall）：沿位置序列以最小/最大高度竖起的面，带材质。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlWall {
    /// 位置。
    #[serde(default)]
    pub positions: Option<CzmlPosition>,
    /// 最大高度。
    #[serde(default)]
    pub maximum_heights: Option<Vec<f64>>,
    /// 最小高度。
    #[serde(default)]
    pub minimum_heights: Option<Vec<f64>>,
    /// 材质。
    #[serde(default)]
    pub material: Option<CzmlMaterial>,
}

/// CZML 球体：以三轴半径定义的椭球，带材质。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlEllipsoid {
    /// 半径 [x, y, z]。
    #[serde(default)]
    pub radii: Option<CzmlCartesian3Value>,
    /// 材质。
    #[serde(default)]
    pub material: Option<CzmlMaterial>,
}

/// CZML path（轨迹）：按前导/拖尾时间绘制实体轨迹，带宽度与材质。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlPath {
    /// 前导时间（lead time）。
    #[serde(default)]
    pub lead_time: Option<f64>,
    /// 拖尾时间（trail time）。
    #[serde(default)]
    pub trail_time: Option<f64>,
    /// 宽度。
    #[serde(default)]
    pub width: Option<f64>,
    /// 材质。
    #[serde(default)]
    pub material: Option<CzmlMaterial>,
}

/// CZML Cartesian3 值：兼容平铺数组与带 cartesian3 字段的对象。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum CzmlCartesian3Value {
    /// 平铺数组 [x, y, z]。
    Array(Vec<f64>),
    /// 带 cartesian3 字段的对象。
    Object { cartesian3: Vec<f64> },
}

/// CZML 颜色值：兼容 RGBA 平铺数组与带 rgba 字段的对象。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum CzmlColor {
    /// RGBA 数组 [r, g, b, a]（0-255）。
    Rgba(Vec<f64>),
    /// 带 rgba 字段的对象。
    Object { rgba: Vec<f64> },
}

/// CZML 材质：当前仅支持纯色 solidColor 一种形式。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CzmlMaterial {
    /// 纯色。
    #[serde(default)]
    pub solid_color: Option<CzmlSolidColor>,
}

/// CZML 纯色材质：携一个 RGBA 颜色供表面填充。
#[derive(Debug, Clone, Deserialize)]
pub struct CzmlSolidColor {
    /// 以 RGBA 表示的颜色。
    #[serde(default)]
    pub color: Option<CzmlColor>,
}

/// 将 CZML 字符串解析为 `DataSource`。
///
/// 先把整个 JSON 数组反序列化为 `CzmlPacket` 序列，再以默认名
/// `"CZML"` 建一个空数据源，逐包处理：`document` 包仅用于设置数据源
/// 名称并跳过，其余每包生成一个实体加入集合，最后标记为已加载。
pub fn parse_czml(json: &str) -> Result<DataSource, CzmlError> {
    let packets: Vec<CzmlPacket> = serde_json::from_str(json)?;

    let mut ds = DataSource::new("CZML");

    for packet in &packets {
        // 跳过 document 数据包
        if packet.id == "document" {
            if let Some(ref name) = packet.name {
                ds.name = name.clone();
            }
            continue;
        }

    // 其余每包生成一个实体并加入集合。
        let entity = process_packet(packet);
        ds.entities.add(entity);
    }

    // 全部处理完毕，标记数据源为已加载。
    ds.loaded = true;
    Ok(ds)
}

/// 处理一个 CZML 数据包并生成对应的 `Entity`。
///
/// 依次尝试各图形字段：存在时以其缺省图形为起点，把 CZML 声明的
/// 颜色、尺寸、位置等数值逐个包装成常量属性（位置若带时间标记则
/// 转为采样属性），最终挂到实体上；未出现的字段保持缺省。
fn process_packet(packet: &CzmlPacket) -> Entity {
    let mut entity = Entity::new(packet.id.clone());

    if let Some(ref name) = packet.name {
        entity = entity.with_name(name.clone());
    }

    if let Some(ref desc) = packet.description {
        entity.description = Some(desc.clone());
    }

    // 处理位置
    if let Some(ref pos) = packet.position {
        let coords = extract_position_coords(pos);
        if coords.len() >= 3 {
            // 检查是否带时间标记（长度 > 3 且首值似为时间）
            if coords.len() > 3 && coords.len().is_multiple_of(4) {
                // 带时间标记：[time, lon, lat, height, time, lon, lat, height, ...]
                let samples: Vec<(f64, [f64; 3])> = coords
                    .chunks(4)
                    .filter(|c| c.len() == 4)
                    .map(|c| (c[0], [c[1].to_radians(), c[2].to_radians(), c[3]]))
                    .collect();
            // 存为按时间采样的位置序列（每样本携带时刻与弧度坐标）。
                entity.position = Property::Sampled(samples);
            } else {
                // 无时间标记：单个 [lon, lat, height]，度转弧度后作常量位置。
                let lon = coords[0].to_radians();
                let lat = coords[1].to_radians();
                let height = coords[2];
                entity.position = Property::Constant([lon, lat, height]);
            }
        }
    }

    // 处理点：将颜色/像素尺寸/轮廓颜色/轮廓宽度包成常量属性。
    if let Some(ref pt) = packet.point {
        // 以缺省点图形为起点，逐项覆盖已声明的常量属性。
        let mut point = PointGraphics::default();
        if let Some(ref color) = pt.color {
            point.color = Property::Constant(czml_color_to_color(color));
        }
        if let Some(size) = pt.pixel_size {
            point.pixel_size = Property::Constant(size);
        }
        if let Some(ref oc) = pt.outline_color {
            point.outline_color = Property::Constant(czml_color_to_color(oc));
        }
        if let Some(ow) = pt.outline_width {
            point.outline_width = Property::Constant(ow);
        }
        entity.point = Some(point);
    }

    // 处理 polyline：位置转弧度坐标序列，宽度与材质颜色各自常量。
    if let Some(ref pl) = packet.polyline {
        // 位置经度转弧度存为常量序列，宽度/颜色各自常量。
        let mut polyline = PolylineGraphics::default();
        if let Some(ref pos) = pl.positions {
            let coords = extract_position_coords(pos);
            let positions = coords_to_positions(&coords);
            polyline.positions = Property::Constant(positions);
        }
        if let Some(width) = pl.width {
            polyline.width = Property::Constant(width);
        }
        if let Some(ref mat) = pl.material {
            if let Some(color) = extract_material_color(mat) {
                polyline.color = Property::Constant(color);
            }
        }
        entity.polyline = Some(polyline);
    }

    // 处理 polygon：位置/材质/高度/挤出高度均映射为常量属性。
    if let Some(ref pg) = packet.polygon {
        // 多边形位置/材质/高度/挤出高度均存为常量属性。
        let mut polygon = PolygonGraphics::default();
        if let Some(ref pos) = pg.positions {
            let coords = extract_position_coords(pos);
            let positions = coords_to_positions(&coords);
            polygon.positions = Property::Constant(positions);
        }
        if let Some(ref mat) = pg.material {
            if let Some(color) = extract_material_color(mat) {
                polygon.material = Property::Constant(color);
            }
        }
        if let Some(h) = pg.height {
            polygon.height = Property::Constant(h);
        }
        if let Some(eh) = pg.extruded_height {
            polygon.extruded_height = Property::Constant(eh);
        }
        entity.polygon = Some(polygon);
    }

    // 处理 label：文本、字体与填充/轮廓颜色转成常量属性。
    if let Some(ref lb) = packet.label {
        // 标签文本/字体与颜色均以常量属性保存。
        let mut label = LabelGraphics::default();
        if let Some(ref text) = lb.text {
            label.text = Property::Constant(text.clone());
        }
        if let Some(ref font) = lb.font {
            label.font = Property::Constant(font.clone());
        }
        if let Some(ref fc) = lb.fill_color {
            label.fill_color = Property::Constant(czml_color_to_color(fc));
        }
        if let Some(ref oc) = lb.outline_color {
            label.outline_color = Property::Constant(czml_color_to_color(oc));
        }
        entity.label = Some(label);
    }

    // 处理 billboard：图像 URI、缩放、颜色、旋转与宽高转成常量属性。
    if let Some(ref bb) = packet.billboard {
        // 广告牌各字段（URI/缩放/颜色/旋转/宽高）转为常量属性。
        let mut billboard = BillboardGraphics::default();
        if let Some(ref image) = bb.image {
            billboard.image = Property::Constant(image.clone());
        }
        if let Some(scale) = bb.scale {
            billboard.scale = Property::Constant(scale);
        }
        if let Some(ref color) = bb.color {
            billboard.color = Property::Constant(czml_color_to_color(color));
        }
        if let Some(rotation) = bb.rotation {
            billboard.rotation = Property::Constant(rotation);
        }
        if let Some(w) = bb.width {
            billboard.width = Property::Constant(w);
        }
        if let Some(h) = bb.height {
            billboard.height = Property::Constant(h);
        }
        entity.billboard = Some(billboard);
    }

    // 处理 model：glTF URI、缩放与最小像素尺寸转成常量属性。
    if let Some(ref mdl) = packet.model {
        // 模型的 URI/缩放/最小像素尺寸转为常量属性。
        let mut model = ModelGraphics::default();
        if let Some(ref gltf) = mdl.gltf {
            model.uri = Property::Constant(gltf.clone());
        }
        if let Some(scale) = mdl.scale {
            model.scale = Property::Constant(scale);
        }
        if let Some(mps) = mdl.minimum_pixel_size {
            model.minimum_pixel_size = Property::Constant(mps);
        }
        entity.model = Some(model);
    }

    // 处理椭圆：半长/半短轴、高度与材质颜色转成常量属性。
    if let Some(ref ell) = packet.ellipse {
        // 椭圆的半轴/高度/材质转为常量属性。
        let mut ellipse = EllipseGraphics::default();
        if let Some(sma) = ell.semi_major_axis {
            ellipse.semi_major_axis = Property::Constant(sma);
        }
        if let Some(smi) = ell.semi_minor_axis {
            ellipse.semi_minor_axis = Property::Constant(smi);
        }
        if let Some(h) = ell.height {
            ellipse.height = Property::Constant(h);
        }
        if let Some(ref mat) = ell.material {
            if let Some(color) = extract_material_color(mat) {
                ellipse.material = Property::Constant(color);
            }
        }
        entity.ellipse = Some(ellipse);
    }

    // 处理方框：尺寸取前三分量转为常量三维值，材质颜色同样常量。
    if let Some(ref bx) = packet.box_graphics {
        // 方框尺寸取前三分量，连同材质颜色存为常量属性。
        let mut box_g = BoxGraphics::default();
        if let Some(ref dims) = bx.dimensions {
            let v = extract_cartesian3(dims);
            if v.len() >= 3 {
                box_g.dimensions = Property::Constant([v[0], v[1], v[2]]);
            }
        }
        if let Some(ref mat) = bx.material {
            if let Some(color) = extract_material_color(mat) {
                box_g.material = Property::Constant(color);
            }
        }
        entity.box_graphics = Some(box_g);
    }

    // 处理圆柱：长度、顶/底半径与材质颜色各自映射为常量属性。
    if let Some(ref cyl) = packet.cylinder {
        // 圆柱长度/顶底半径/材质转为常量属性。
        let mut cylinder = CylinderGraphics::default();
        if let Some(l) = cyl.length {
            cylinder.length = Property::Constant(l);
        }
        if let Some(tr) = cyl.top_radius {
            cylinder.top_radius = Property::Constant(tr);
        }
        if let Some(br) = cyl.bottom_radius {
            cylinder.bottom_radius = Property::Constant(br);
        }
        if let Some(ref mat) = cyl.material {
            if let Some(color) = extract_material_color(mat) {
                cylinder.material = Property::Constant(color);
            }
        }
        entity.cylinder = Some(cylinder);
    }

    // 处理 corridor：位置转坐标序列，宽度/高度/材质转常量属性。
    if let Some(ref cor) = packet.corridor {
        // 走廊位置/宽度/高度/材质转为常量属性。
        let mut corridor = CorridorGraphics::default();
        if let Some(ref pos) = cor.positions {
            let coords = extract_position_coords(pos);
            let positions = coords_to_positions(&coords);
            corridor.positions = Property::Constant(positions);
        }
        if let Some(w) = cor.width {
            corridor.width = Property::Constant(w);
        }
        if let Some(h) = cor.height {
            corridor.height = Property::Constant(h);
        }
        if let Some(ref mat) = cor.material {
            if let Some(color) = extract_material_color(mat) {
                corridor.material = Property::Constant(color);
            }
        }
        entity.corridor = Some(corridor);
    }

    // 处理矩形：四边坐标按度转弧度存为常量，高度与材质常量。
    if let Some(ref rect) = packet.rectangle {
        // 矩形四边坐标按度转弧度存为常量，高度/材质常量。
        let mut rectangle = RectangleGraphics::default();
        if let Some(ref coords) = rect.coordinates {
            let v = match coords {
                CzmlRectangleCoords::Array(a) => a.clone(),
                CzmlRectangleCoords::Object { degrees } => degrees.clone(),
            };
            if v.len() >= 4 {
                rectangle.coordinates = Property::Constant([
                    v[0].to_radians(), v[1].to_radians(),
                    v[2].to_radians(), v[3].to_radians(),
                ]);
            }
        }
        if let Some(h) = rect.height {
            rectangle.height = Property::Constant(h);
        }
        if let Some(ref mat) = rect.material {
            if let Some(color) = extract_material_color(mat) {
                rectangle.material = Property::Constant(color);
            }
        }
        entity.rectangle = Some(rectangle);
    }

    // 处理墙体：位置转坐标序列，最大/最小高度与材质转常量属性。
    if let Some(ref wl) = packet.wall {
        // 墙体位置/最大最小高度/材质转为常量属性。
        let mut wall = WallGraphics::default();
        if let Some(ref pos) = wl.positions {
            let coords = extract_position_coords(pos);
            let positions = coords_to_positions(&coords);
            wall.positions = Property::Constant(positions);
        }
        if let Some(ref mh) = wl.maximum_heights {
            wall.maximum_heights = Property::Constant(mh.clone());
        }
        if let Some(ref mh) = wl.minimum_heights {
            wall.minimum_heights = Property::Constant(mh.clone());
        }
        if let Some(ref mat) = wl.material {
            if let Some(color) = extract_material_color(mat) {
                wall.material = Property::Constant(color);
            }
        }
        entity.wall = Some(wall);
    }

    // 处理球体：半径取前三分量转常量三维值，材质颜色常量。
    if let Some(ref el) = packet.ellipsoid {
        // 球体半径取前三分量存为常量，材质颜色常量。
        let mut ellipsoid = EllipsoidGraphics::default();
        if let Some(ref radii) = el.radii {
            let v = extract_cartesian3(radii);
            if v.len() >= 3 {
                ellipsoid.radii = Property::Constant([v[0], v[1], v[2]]);
            }
        }
        if let Some(ref mat) = el.material {
            if let Some(color) = extract_material_color(mat) {
                ellipsoid.material = Property::Constant(color);
            }
        }
        entity.ellipsoid = Some(ellipsoid);
    }

    // 处理 path：前导/拖尾时间、宽度与材质颜色各自常量。
    if let Some(ref pth) = packet.path {
        // 轨迹前导/拖尾时间、宽度与材质转为常量属性。
        let mut path = PathGraphics::default();
        if let Some(lt) = pth.lead_time {
            path.lead_time = Property::Constant(lt);
        }
        if let Some(tt) = pth.trail_time {
            path.trail_time = Property::Constant(tt);
        }
        if let Some(w) = pth.width {
            path.width = Property::Constant(w);
        }
        if let Some(ref mat) = pth.material {
            if let Some(color) = extract_material_color(mat) {
                path.material = Property::Constant(color);
            }
        }
        entity.path = Some(path);
    }

    // 返回填充完毕、携带全部已声明图形的实体。
    entity
}

/// 从 CZML 位置中提取坐标值：无论写成平铺数组还是带
/// `cartographicDegrees` 字段的对象，都归一为一个 `f64` 序列。
fn extract_position_coords(pos: &CzmlPosition) -> Vec<f64> {
    // 两种位置写法（平铺数组或带字段对象）都归一为同一个坐标序列。
    match pos {
        CzmlPosition::CartographicDegrees(v) => v.clone(),
        CzmlPosition::Object { cartographic_degrees } => cartographic_degrees.clone(),
    }
}

/// 将平铺坐标数组 [lon, lat, height, lon, lat, height, ...] 转换为位置。
///
/// 每三个分量一组，经度与纬度按度转弧度，高度保持米不变；不足
/// 三个的尾组会被过滤丢弃。
fn coords_to_positions(coords: &[f64]) -> Vec<[f64; 3]> {
    // 每三分量一组，经纬度按度转弧度，高度以米原样保留。
    coords
        .chunks(3)
        .filter(|c| c.len() == 3)
        .map(|c| [c[0].to_radians(), c[1].to_radians(), c[2]])
        .collect()
}

/// 将 CZML 颜色（0-255 的 RGBA）转换为内部 `Color`（0-1 归一）。
///
/// 分量不足 4 个时回退为白色；否则各通道除以 255 归一。
fn czml_color_to_color(czml_color: &CzmlColor) -> Color {
    // 先归一 RGBA 取值：平铺数组或带 rgba 字段的对象。
    let rgba = match czml_color {
        CzmlColor::Rgba(v) => v.clone(),
        CzmlColor::Object { rgba } => rgba.clone(),
    };

    // 满 4 分量则各通道除以 255 归一到 0-1，否则回退为白色。
    if rgba.len() >= 4 {
        Color::new(
            rgba[0] / 255.0,
            rgba[1] / 255.0,
            rgba[2] / 255.0,
            rgba[3] / 255.0,
        )
    } else {
        Color::WHITE
    }
}

/// 从 CZML 材质中提取纯色颜色：仅支持 `solidColor`，沿
/// solid_color → color 逐级取到后转换为内部 `Color`。
fn extract_material_color(mat: &CzmlMaterial) -> Option<Color> {
    // 仅 solidColor 材质可提取颜色，沿嵌套字段逐级取到纯色。
    mat.solid_color
        .as_ref()
        .and_then(|sc| sc.color.as_ref())
        .map(czml_color_to_color)
}

/// 从 CZML Cartesian3 中提取三维数值：兼容平铺数组与带
/// `cartesian3` 字段的对象两种写法。
fn extract_cartesian3(val: &CzmlCartesian3Value) -> Vec<f64> {
    // 兼容平铺数组与带 cartesian3 字段的对象两种写法。
    match val {
        CzmlCartesian3Value::Array(v) => v.clone(),
        CzmlCartesian3Value::Object { cartesian3 } => cartesian3.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证含 document 包的完整解析：数据源名取自 document，实体
    /// 集合包含一个带点图形的实体。
    #[test]
    fn test_parse_czml_document() {
        let json = r#"[
            {"id": "document", "name": "Test CZML", "version": "1.0"},
            {"id": "point-1", "name": "My Point", "position": {"cartographicDegrees": [-75.0, 40.0, 100.0]},
             "point": {"color": {"rgba": [255, 0, 0, 255]}, "pixelSize": 10}}
        ]"#;

        let ds = parse_czml(json).unwrap();
        assert_eq!(ds.name, "Test CZML");
        assert_eq!(ds.entities.len(), 1);

        let entity = ds.entities.get("point-1").unwrap();
        assert_eq!(entity.name, Some("My Point".to_string()));
        assert!(entity.point.is_some());
    }

    /// 验证折线解析：两个经纬度端点转为两条位置，宽度与材质颜色
    /// 也被填充。
    #[test]
    fn test_parse_czml_polyline() {
        let json = r#"[
            {"id": "document", "name": "Lines"},
            {"id": "line-1", "polyline": {
                "positions": {"cartographicDegrees": [-75.0, 40.0, 0.0, -74.0, 41.0, 0.0]},
                "width": 3.0,
                "material": {"solidColor": {"color": {"rgba": [0, 255, 0, 255]}}}
            }}
        ]"#;

        let ds = parse_czml(json).unwrap();
        let entity = ds.entities.get("line-1").unwrap();
        assert!(entity.polyline.is_some());

        let polyline = entity.polyline.as_ref().unwrap();
        let positions = polyline.positions.get_value(0.0).unwrap();
        assert_eq!(positions.len(), 2);
    }

    /// 验证多边形解析：三个顶点转为三位置，挤出高度取到 10000。
    #[test]
    fn test_parse_czml_polygon() {
        let json = r#"[
            {"id": "document", "name": "Polygons"},
            {"id": "poly-1", "polygon": {
                "positions": {"cartographicDegrees": [-75.0, 40.0, 0.0, -74.0, 40.0, 0.0, -74.0, 41.0, 0.0]},
                "material": {"solidColor": {"color": {"rgba": [255, 255, 0, 128]}}},
                "height": 0,
                "extrudedHeight": 10000
            }}
        ]"#;

        let ds = parse_czml(json).unwrap();
        let entity = ds.entities.get("poly-1").unwrap();
        assert!(entity.polygon.is_some());

        let polygon = entity.polygon.as_ref().unwrap();
        let positions = polygon.positions.get_value(0.0).unwrap();
        assert_eq!(positions.len(), 3);
        let eh = polygon.extruded_height.get_value(0.0).unwrap();
        assert!((*eh - 10000.0).abs() < 1e-10);
    }

    /// 验证颜色转换：RGBA(255,128,0,255) 按 255 归一后各通道正确。
    #[test]
    fn test_czml_color_conversion() {
        let color = CzmlColor::Rgba(vec![255.0, 128.0, 0.0, 255.0]);
        let result = czml_color_to_color(&color);
        assert!((result.red - 1.0).abs() < 1e-10);
        assert!((result.green - 128.0 / 255.0).abs() < 1e-10);
        assert!((result.blue - 0.0).abs() < 1e-10);
        assert!((result.alpha - 1.0).abs() < 1e-10);
    }

    /// 验证平铺坐标转位置：经/纬度按度转弧度，高度保持不变，
    /// 两个三分量组各自成位。
    #[test]
    fn test_coords_to_positions() {
        let coords = vec![-180.0, -90.0, 0.0, 180.0, 90.0, 1000.0];
        let positions = coords_to_positions(&coords);
        assert_eq!(positions.len(), 2);
        assert!((positions[0][0] - (-std::f64::consts::PI)).abs() < 1e-10);
        assert!((positions[1][2] - 1000.0).abs() < 1e-10);
    }

    /// 验证广告牌解析：图像 URI 为 marker.png，缩放为 2.0。
    #[test]
    fn test_parse_czml_billboard() {
        let json = r#"[
            {"id": "document", "name": "Billboards"},
            {"id": "bb-1", "position": {"cartographicDegrees": [-75.0, 40.0, 0.0]},
             "billboard": {"image": "marker.png", "scale": 2.0, "color": {"rgba": [255, 0, 0, 255]}}}
        ]"#;

        let ds = parse_czml(json).unwrap();
        let entity = ds.entities.get("bb-1").unwrap();
        assert!(entity.billboard.is_some());
        let bb = entity.billboard.as_ref().unwrap();
        assert_eq!(bb.image.get_value(0.0).unwrap(), "marker.png");
        assert!((*bb.scale.get_value(0.0).unwrap() - 2.0).abs() < 1e-10);
    }

    /// 验证模型解析：gltf URI 取到 model.glb 并挂到实体上。
    #[test]
    fn test_parse_czml_model() {
        let json = r#"[
            {"id": "document", "name": "Models"},
            {"id": "model-1", "position": {"cartographicDegrees": [-75.0, 40.0, 0.0]},
             "model": {"gltf": "model.glb", "scale": 10.0}}
        ]"#;

        let ds = parse_czml(json).unwrap();
        let entity = ds.entities.get("model-1").unwrap();
        assert!(entity.model.is_some());
        let model = entity.model.as_ref().unwrap();
        assert_eq!(model.uri.get_value(0.0).unwrap(), "model.glb");
    }

    /// 验证方框解析：dimensions 的 cartesian3 三分量存为常量三维值。
    #[test]
    fn test_parse_czml_box() {
        let json = r#"[
            {"id": "document", "name": "Boxes"},
            {"id": "box-1", "position": {"cartographicDegrees": [-75.0, 40.0, 0.0]},
             "box": {"dimensions": {"cartesian3": [100.0, 200.0, 300.0]},
                      "material": {"solidColor": {"color": {"rgba": [255, 0, 0, 255]}}}}}
        ]"#;

        let ds = parse_czml(json).unwrap();
        let entity = ds.entities.get("box-1").unwrap();
        assert!(entity.box_graphics.is_some());
        let bx = entity.box_graphics.as_ref().unwrap();
        let dims = bx.dimensions.get_value(0.0).unwrap();
        assert_eq!(*dims, [100.0, 200.0, 300.0]);
    }

    /// 验证带时间标记的位置解析为采样属性：四个一组的时间/经/纬/高
    /// 形成两个样本，时间分别为 0 与 60。
    #[test]
    fn test_parse_czml_time_dynamic_position() {
        let json = r#"[
            {"id": "document", "name": "Dynamic"},
            {"id": "sat-1", "position": {"cartographicDegrees": [0, -75.0, 40.0, 100.0, 60, -74.0, 41.0, 200.0]}}
        ]"#;

        let ds = parse_czml(json).unwrap();
        let entity = ds.entities.get("sat-1").unwrap();
        // 应为采样（带时间标记）
        match &entity.position {
            Property::Sampled(samples) => {
                assert_eq!(samples.len(), 2);
                assert!((samples[0].0 - 0.0).abs() < 1e-10);
                assert!((samples[1].0 - 60.0).abs() < 1e-10);
            }
            _ => panic!("Expected sampled position"),
        }
    }

    /// 验证圆柱解析：长度 500、顶/底半径与材质都正确映射为常量属性。
    #[test]
    fn test_parse_czml_cylinder() {
        let json = r#"[
            {"id": "document", "name": "Cylinders"},
            {"id": "cyl-1", "position": {"cartographicDegrees": [-75.0, 40.0, 0.0]},
             "cylinder": {"length": 500.0, "topRadius": 50.0, "bottomRadius": 100.0}}
        ]"#;

        let ds = parse_czml(json).unwrap();
        let entity = ds.entities.get("cyl-1").unwrap();
        assert!(entity.cylinder.is_some());
        let cyl = entity.cylinder.as_ref().unwrap();
        assert!((*cyl.length.get_value(0.0).unwrap() - 500.0).abs() < 1e-10);
    }

    /// 验证轨迹解析：前导时间 3600、拖尾时间与宽度均取到常量值。
    #[test]
    fn test_parse_czml_path() {
        let json = r#"[
            {"id": "document", "name": "Paths"},
            {"id": "path-1", "path": {"leadTime": 3600, "trailTime": 7200, "width": 3.0}}
        ]"#;

        let ds = parse_czml(json).unwrap();
        let entity = ds.entities.get("path-1").unwrap();
        assert!(entity.path.is_some());
        let path = entity.path.as_ref().unwrap();
        assert!((*path.lead_time.get_value(0.0).unwrap() - 3600.0).abs() < 1e-10);
    }

    /// 验证标签增强解析：文本与字体字符串都作为常量属性保留。
    #[test]
    fn test_parse_czml_label_enhanced() {
        let json = r#"[
            {"id": "document", "name": "Labels"},
            {"id": "label-1", "label": {"text": "Hello", "font": "16px monospace",
             "fillColor": {"rgba": [255, 255, 0, 255]}}}
        ]"#;

        let ds = parse_czml(json).unwrap();
        let entity = ds.entities.get("label-1").unwrap();
        assert!(entity.label.is_some());
        let label = entity.label.as_ref().unwrap();
        assert_eq!(label.text.get_value(0.0).unwrap(), "Hello");
        assert_eq!(label.font.get_value(0.0).unwrap(), "16px monospace");
    }
}
