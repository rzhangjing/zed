//! 场景文档的 GeoJSON 编码 / 解码（计划 §12，M8）。
//!
//! 两个保证，有意分层设计：
//!  * **互操作性** —— 每个要素都携带一个标准 GeoJSON 几何
//!    （`Point` / `LineString` / `Polygon`），使其他工具能读取文件；自由
//!    业务属性被展开到 `properties` 中。
//!  * **无损性** —— 一个顶层 `x-plot` 扩展成员嵌入了整个
//!    [`Document`]（图层、组、id、样式、标志、比例 / 时间窗口）。
//!    我们的读取器优先采用那个载荷，因此 `to_geojson` → `from_geojson` 会
//!    *精确* 恢复文档（由一个往返测试验证）。一个没有
//!    `x-plot` 的文件（外部 GeoJSON）仍会尽力导入：一个默认图层加
//!    每要素一个元素。
//!
//! 没有 GeoJSON 对应形式的几何类型（圆 / 椭圆 / 弧 / 路径 /
//! 复合）仍会导出一个代表性的 `Point` 以供互操作；它们的精确
//! 参数存活在 `x-plot` 文档中。
//!
//! ## 读取优先级
//! [`from_geojson`] 先检查顶层 `x-plot.document`：存在即走无损路径，直接反
//! 序列化为整文档并 [`Document::rebuild_parents`]；否则回退到标准 GeoJSON
//! 尽力导入。因此我们自己的导出总能完整回读，而外部文件至少能拿到几何与名称。

use serde_json::{json, Map, Value};

use crate::geo::GeoPoint;
use crate::model::geometry::{Geometry, Polygon, Polyline};
use crate::model::Document;

/// IO 失败面。
#[derive(Debug, thiserror::Error)]
pub enum PlotIoError {
    /// 格式错误或不可读的 JSON。
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// 输入既不是一个 `x-plot` 文档也不是一个 GeoJSON `FeatureCollection`。
    #[error("expected a GeoJSON FeatureCollection")]
    NotFeatureCollection,
}

/// 将整个文档序列化为一个 GeoJSON `FeatureCollection`（美化输出）。
pub fn to_geojson(doc: &Document) -> Result<String, PlotIoError> {
    // 按绘制序遍历，保证输出要素顺序稳定且与屏幕层叠一致。
    let mut features: Vec<Value> = Vec::new();
    for id in doc.flatten_draw_order() {
        let Some(el) = doc.element(id) else {
            continue;
        };
        // `properties` = 业务属性（展开）+ `name` + `x-plot`。
        // x-plot 存整个元素的序列化，使无损回读能还原样式/标志/id。
        let mut props = match serde_json::to_value(&el.attributes)? {
            // 属性必须是对象才能展开；否则从一个空映射起步。
            Value::Object(m) => m,
            _ => Map::new(),
        };
        props.insert("name".into(), json!(el.name));
        props.insert("x-plot".into(), serde_json::to_value(el)?);
        features.push(json!({
            "type": "Feature",
            "properties": props,
            "geometry": geometry_to_gj(&el.geometry),
        }));
    }

    let mut fc = Map::new();
    // 顶层除标准字段外再挂一个 x-plot 扩展，嵌入整文档以保无损往返。
    fc.insert("type".into(), json!("FeatureCollection"));
    fc.insert("features".into(), Value::Array(features));
    fc.insert(
        "x-plot".into(),
        json!({ "version": 1, "document": serde_json::to_value(doc)? }),
    );
    Ok(serde_json::to_string_pretty(&Value::Object(fc))?)
}

/// 将一个 GeoJSON 字符串解析回一个 [`Document`]。优先采用无损的
/// `x-plot.document` 载荷；否则尽力将外部 GeoJSON 导入
/// 到单个全新图层中。
///
/// 两条路径都返回一个已重建父索引的文档，因此调用方拿到的是可直接
/// 向上遍历（element_context 等）的完整状态。
pub fn from_geojson(s: &str) -> Result<Document, PlotIoError> {
    let v: Value = serde_json::from_str(s)?;

    // 无损路径：我们自己的导出嵌入了完整文档。
    // 命中则直接反序列化并重建父索引，跳过逐要素的尽力导入。
    if let Some(dv) = v.get("x-plot").and_then(|x| x.get("document")) {
        let mut doc: Document = serde_json::from_value(dv.clone())?;
        doc.rebuild_parents();
        return Ok(doc);
    }

    // 外部 GeoJSON：一个默认活动图层，每要素一个元素。
    let feats = v
        .get("features")
        .and_then(|f| f.as_array())
        .ok_or(PlotIoError::NotFeatureCollection)?;
    let mut doc = Document::default();
    // 为外部要素新建一个名为“导入”的默认活动图层并聚焦。
    let layer = doc.new_layer("导入");
    doc.focus_layer(layer);
    // 逐个要素：几何不可解析则跳过；名称缺省为“要素 N”。
    for (i, f) in feats.iter().enumerate() {
        let Some(gj) = f.get("geometry") else {
            continue;
        };
        let Some(geo) = gj_to_geometry(gj) else {
            continue;
        };
        let props = f.get("properties").and_then(|p| p.as_object());
        let name = props
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("要素 {}", i + 1));
        let mut ne = doc.make_element(name, geo);
        // 除 name 外的其余 properties 全部展开为自由业务属性。
        if let Some(p) = props {
            for (k, val) in p {
                if k != "name" {
                    ne.element.attributes.insert(k.clone(), val.clone());
                }
            }
        }
        doc.add_element_to_layer(layer, ne);
    }
    Ok(doc)
}

// ── geometry ⇄ GeoJSON ──────────────────────────────────────────────────────

/// 一个位置 `[lon, lat, height]`。
///
/// 高度总被写出（缺省 0），以便回读时能区分真实高度与缺省值。
fn coord(p: GeoPoint) -> Value {
    json!([p.lon_deg, p.lat_deg, p.height_m])
}

/// 一个线性环坐标列表，闭合该环（GeoJSON 要求首 == 尾；
/// 模型以开放形式存储）。
fn closed_ring(ring: &[GeoPoint]) -> Vec<Value> {
    // 先把开放形式逐点编码，若环非空再追补一个首点副本以闭合。
    let mut v: Vec<Value> = ring.iter().map(|p| coord(*p)).collect();
    if let Some(first) = ring.first() {
        v.push(coord(*first));
    }
    v
}

/// 为互操作而做的尽力标准 GeoJSON 几何。没有精确
/// 对应形式的类型回退到一个代表性的锚点 `Point`（或 `null`）。
fn geometry_to_gj(g: &Geometry) -> Option<Value> {
    // 逐变体映射到最接近的标准 GeoJSON 类型（Icon/Label 降为点）。
    match g {
        Geometry::Point(p) => Some(json!({"type": "Point", "coordinates": coord(*p)})),
        Geometry::Icon(i) => Some(json!({"type": "Point", "coordinates": coord(i.at)})),
        Geometry::Label(l) => Some(json!({"type": "Point", "coordinates": coord(l.at)})),
        Geometry::Polyline(pl) => Some(json!({
            // 折线直接逐点输出为 LineString（无需闭合）。
            "type": "LineString",
            "coordinates": pl.positions.iter().map(|p| coord(*p)).collect::<Vec<_>>(),
        })),
        Geometry::Rectangle(r) => {
            // 矩形展开为四个角（逆时针），再走闭合环约定输出为 Polygon。
            let ring = [
                GeoPoint::surface(r.west, r.south),
                GeoPoint::surface(r.east, r.south),
                GeoPoint::surface(r.east, r.north),
                GeoPoint::surface(r.west, r.north),
            ];
            Some(json!({"type": "Polygon", "coordinates": [closed_ring(&ring)]}))
        }
        Geometry::Polygon(pg) => {
            // 外环在前、孔洞依次跟随，与 GeoJSON Polygon 的环嵌套约定一致。
            let mut rings = vec![closed_ring(&pg.outer)];
            for h in &pg.holes {
                rings.push(closed_ring(h));
            }
            Some(json!({"type": "Polygon", "coordinates": rings}))
        }
        // 无标准形式：为互操作提供一个代表点（精确参数存在于
        // x-plot 文档中）。
        other => other
            .anchor()
            .map(|a| json!({"type": "Point", "coordinates": coord(a)})),
    }
}

/// 单个 GeoJSON 位置 `[lon, lat, h]`。
fn position(v: &Value) -> Option<GeoPoint> {
    // 至多三个数：经/纬必需，高度缺省为 0；少于 2 个元素视为无效。
    let p = v.as_array()?;
    if p.len() < 2 {
        return None;
    }
    let lon = p[0].as_f64()?;
    let lat = p[1].as_f64()?;
    let h = p.get(2).and_then(|x| x.as_f64()).unwrap_or(0.0);
    Some(GeoPoint::new(lon, lat, h))
}

/// 将一个扁平坐标数组（`[lon, lat, h]`…）解析为模型点，并在存在时
/// 去除一个尾部重复的闭合顶点。
fn positions(v: &Value) -> Option<Vec<GeoPoint>> {
    // 任一坐标解析失败则整体返回 None（尽力导入下的严格形状校验）。
    let arr = v.as_array()?;
    let mut out: Vec<GeoPoint> = Vec::with_capacity(arr.len());
    for c in arr {
        out.push(position(c)?);
    }
    // 剔除 GeoJSON 强制但我们省略的闭合重复项（first == last）。
    if out.len() >= 2 {
        let (a, b) = (out.first().unwrap().lon_deg, out.last().unwrap().lon_deg);
        let (al, bl) = (out.first().unwrap().lat_deg, out.last().unwrap().lat_deg);
        if (a - b).abs() < 1e-12 && (al - bl).abs() < 1e-12 {
            out.pop();
        }
    }
    Some(out)
}

/// 外部 GeoJSON 几何 → 模型（尽力而为；不支持的类型 → `None`）。
fn gj_to_geometry(v: &Value) -> Option<Geometry> {
    // 按 GeoJSON type 分派；单点 LineString 降级为 Point，未知类型返回 None。
    let ty = v.get("type")?.as_str()?;
    match ty {
        "Point" => Some(Geometry::Point(position(v.get("coordinates")?)?)),
        // 单点降级为 Point；≥ 2 点作为折线；空集无法表示。
        "MultiPoint" | "LineString" => {
            let pts = positions(v.get("coordinates")?)?;
            if pts.len() == 1 {
                Some(Geometry::Point(pts[0]))
            } else if pts.len() >= 2 {
                Some(Geometry::Polyline(Polyline { positions: pts }))
            } else {
                None
            }
        }
        "Polygon" => {
            // 首个环为外环（需至少 3 点），其余环作为孔洞逐个收集。
            let rings = v.get("coordinates")?.as_array()?;
            let outer = positions(rings.first()?)?;
            if outer.len() < 3 {
                return None;
            }
            let mut holes = Vec::new();
            for r in rings.iter().skip(1) {
                if let Some(h) = positions(r) {
                    if h.len() >= 3 {
                        holes.push(h);
                    }
                }
            }
            Some(Geometry::Polygon(Polygon { outer, holes }))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::{Circle, LabelAnchor, LabelGeometry, Rectangle};
    use crate::model::style::Style;
    use serde_json::Value;

    /// 构造一个地面高度为 0 的测试用地理点（仅需经/纬度）。
    fn p(lon: f64, lat: f64) -> GeoPoint {
        GeoPoint::surface(lon, lat)
    }

        // 一个文档，涵盖图层、一个组、多种几何类型、样式、
        // 属性以及图层标志 / 顺序。
        // 这个富文档同时充当无损往返与互操作导出的共享夹具。
    fn rich_doc() -> Document {
        // 先建两个图层并设不同 order/可见/不透明度，以验证图层标志可无损往返。
        let mut doc = Document::default();
        let back = doc.new_layer("底图");
        let front = doc.new_layer("标绘");
        doc.layer_mut(back).unwrap().order = 0;
        doc.layer_mut(back).unwrap().visible = false;
        doc.layer_mut(front).unwrap().order = 10;
        doc.layer_mut(front).unwrap().opacity = 0.5;
        doc.focus_layer(front);

        // 带一个颜色 + 属性的普通点。
        let mut pt = doc.make_element("观察点", Geometry::Point(p(116.4, 39.9)));
        pt.element.style = Style::default().with_color([1.0, 0.0, 0.0, 1.0]);
        pt.element
            .attributes
            .insert("side".into(), Value::String("friend".into()));
        doc.add_element_to_layer(front, pt);

        // 后层图层中的一个组，容纳一个带标签的元素 + 一个矩形。
        let g = doc.new_group_in_layer(back, "编队");
        let lbl = doc.make_element(
            "标签",
            Geometry::Label(LabelGeometry {
                at: p(10.0, 20.0),
                text: "前沿".into(),
                anchor: LabelAnchor::Bottom,
                offset_px: [0.0, 4.0],
            }),
        );
        doc.add_element_to_group(g, lbl);
        let rect = doc.make_element(
            "区",
            Geometry::Rectangle(Rectangle {
                west: 0.0,
                south: 0.0,
                east: 1.0,
                north: 2.0,
            }),
        );
        doc.add_element_to_group(g, rect);

        // 前层图层中的一条折线、一个带孔多边形和一个圆。
        // 圆最后被手动隐藏，以验证 flags 也能无损往返。
        let line = doc.make_element(
            "路线",
            Geometry::Polyline(Polyline {
                positions: vec![p(0.0, 0.0), p(1.0, 1.0), p(2.0, 0.0)],
            }),
        );
        doc.add_element_to_layer(front, line);
        let poly = doc.make_element(
            "防区",
            Geometry::Polygon(Polygon {
                outer: vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0), p(0.0, 4.0)],
                holes: vec![vec![p(1.0, 1.0), p(2.0, 1.0), p(2.0, 2.0), p(1.0, 2.0)]],
            }),
        );
        doc.add_element_to_layer(front, poly);
        let circle = doc.make_element(
            "射程",
            Geometry::Circle(Circle {
                center: p(5.0, 5.0),
                radius_m: 12_345.0,
            }),
        );
        let cid = circle.id;
        doc.add_element_to_layer(front, circle);
        doc.element_mut(cid).unwrap().flags.visible_manual = false;

        doc
    }

    /// 经 `to_geojson` → `from_geojson` 往返应逐字段恢复原文档（
    /// 依靠无损 `x-plot` 载荷），图层数、元素数与活动图层均不变。
    #[test]
    fn lossless_roundtrip_restores_document_exactly() {
        // 导出再回读，逐字段比对应与原 rich_doc 完全相等。
        let doc = rich_doc();
        let text = to_geojson(&doc).unwrap();
        let back = from_geojson(&text).unwrap();
        // 完全结构相等（图层、组、id、样式、标志、顺序）。
        assert_eq!(back, doc);
        assert_eq!(back.layers().len(), 2);
        assert_eq!(back.element_count(), 6);
        assert_eq!(back.active_layer(), doc.active_layer());
    }

    /// 导出应为点/线/面生成标准 GeoJSON 几何，供其他工具读取；
    /// 验证类型、坐标与多边形闭合环（首坐标 == 尾坐标）。
    #[test]
    fn exports_standard_geometries_for_interop() {
        let mut doc = Document::with_default_layer();
        let l = doc.active_layer().unwrap();
        let a = doc.make_element("pt", Geometry::Point(p(1.5, 2.5)));
        let b = doc.make_element(
            "ln",
            Geometry::Polyline(Polyline {
                positions: vec![p(0.0, 0.0), p(1.0, 1.0)],
            }),
        );
        let c = doc.make_element(
            "pg",
            Geometry::Polygon(Polygon {
                outer: vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)],
                holes: vec![],
            }),
        );
        doc.add_element_to_layer(l, a);
        doc.add_element_to_layer(l, b);
        doc.add_element_to_layer(l, c);
        let text = to_geojson(&doc).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        // 顶层为 FeatureCollection，且每个元素都成为一个独立要素。
        assert_eq!(v["type"], "FeatureCollection");
        let feats = v["features"].as_array().unwrap();
        assert_eq!(feats.len(), 3);
        assert_eq!(feats[0]["geometry"]["type"], "Point");
        assert_eq!(feats[0]["geometry"]["coordinates"], json!([1.5, 2.5, 0.0]));
        assert_eq!(feats[1]["geometry"]["type"], "LineString");
        assert_eq!(feats[1]["geometry"]["coordinates"].as_array().unwrap().len(), 2);
        // 多边形环是闭合的：first coord == last coord（存 3 → 发出 4）。
        let ring = feats[2]["geometry"]["coordinates"][0].as_array().unwrap();
        assert_eq!(ring.len(), 4);
        assert_eq!(ring[0], ring[3]);
        assert_eq!(feats[0]["properties"]["name"], "pt");
    }

    /// 无 `x-plot` 的外部 GeoJSON 应尽力导入到一个活动图层：支持点/多边形，
    /// 不支持的 MultiPolygon 被跳过，且多边形环被去闭合。
    #[test]
    fn foreign_geojson_imports_into_active_layer() {
        let text = r#"{
            "type": "FeatureCollection",
            "features": [
                { "type": "Feature", "properties": { "name": "起点", "side": "hostile" },
                  "geometry": { "type": "Point", "coordinates": [3.0, 4.0] } },
                { "type": "Feature", "properties": { "name": "边界" },
                  "geometry": { "type": "Polygon",
                    "coordinates": [ [[0,0],[2,0],[2,2],[0,2],[0,0]] ] } },
                { "type": "Feature", "properties": {},
                  "geometry": { "type": "MultiPolygon",
                    "coordinates": [ [[[0,0],[1,0],[1,1],[0,0]]] ] } }
            ]
        }"#;
        let doc = from_geojson(text).unwrap();
        // 不支持的 MultiPolygon 要素被跳过 → 2 个元素。
        // 导入的多边形应去闭合，名称与 side 属性都应保留。
        assert_eq!(doc.element_count(), 2);
        let layer = doc.active_layer().unwrap();
        assert_eq!(doc.layer(layer).unwrap().name, "导入");
        let ids: Vec<_> = doc.element_ids().collect();
        let start = doc.element(ids[0]).unwrap();
        // 首要素：名称/几何/展开属性都应保留。
        assert_eq!(start.name, "起点");
        assert_eq!(start.geometry, Geometry::Point(p(3.0, 4.0)));
        assert_eq!(
            start.attributes.get("side"),
            Some(&Value::String("hostile".into()))
        );
        let border = doc.element(ids[1]).unwrap();
        assert!(matches!(border.geometry, Geometry::Polygon(_)));
        // 导入的多边形环被去闭合（4 个唯一顶点，而非 5）。
        match &border.geometry {
            Geometry::Polygon(pg) => assert_eq!(pg.outer.len(), 4),
            _ => unreachable!(),
        }
    }

    /// 既非 `x-plot` 文档也非 FeatureCollection、以及非法 JSON，均应报错。
    #[test]
    fn rejects_non_featurecollection() {
        assert!(matches!(
            from_geojson(r#"{"type": "Feature"}"#),
            Err(PlotIoError::NotFeatureCollection)
        ));
        assert!(matches!(
            from_geojson("not json"),
            Err(PlotIoError::Json(_))
        ));
    }
}
