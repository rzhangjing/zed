//! 面向 agent 的**契约导出**（计划 P1）。
//!
//! 外部宿主在驱动标绘内核前需要做的两件事：
//!
//!  1. 一份 [`AgentAction`] 线格式的机器可读描述，以纯 JSON Schema [`Value`]
//!     的形式交付，使工具调用者可以将其直接喂入 function-calling / 结构化
//!     输出，而无需任何代码生成依赖；
//!  2. 一个整文档读 / 写门面（`export_document_json` /
//!     [`import_document_json`]），复用无损 GeoJSON 路径，从而 agent 的批量
//!     导入 / 导出入口与系统其余部分已经信任的是同一个。
//!
//! 该 schema 是手写的 —— 有意**不**引入 `schemars`（或任何新的）依赖。
//! [`EXAMPLE_ACTION`] 是一个具体负载，测试会将其解析回一个 [`AgentAction`]，
//! 从而把 schema 钉死在真实类型上，使它们不会悄然偏离。
//!
//! ## 契约稳定性
//! 该 schema 是 agent 与内核之间的公共边界，一旦被外部工具缓存就不能随意
//! 破坏兼容。因此每个字段的可选性、枚举取值集合与 `$ref` 结构都尽量保持
//! 向后兼容：新增字段一律可选、绝不复用旧字段名、枚举只增不减。测试
//! [`schema_is_valid_json_with_expected_shape`] 钉住地标键，[`EXAMPLE_ACTION`]
//! 则保证 schema 与真实 serde 类型不会各自漂移。
//!
//! ## 门面读写
//! [`export_document_json`] 与 [`import_document_json`] 复用同一套无损 GeoJSON
//! 编解码路径，因此 agent 批量导入导出的字节流与交互式工具保存的完全一致，
//! 不存在两套并行的序列化实现。

use serde_json::{json, Value};

use crate::io::{from_geojson, to_geojson, PlotIoError};
use crate::model::Document;

/// 一份稳定的、手写的 JSON Schema（draft 2020-12 子集），描述
/// [`AgentAction`] 信封。外部标签的 serde 枚举被建模为对单键对象的
/// `oneOf`，这正是它们序列化的方式。
///
/// 返回值总是一棵有效的 [`Value`] 树（由 `json!` 宏依构造保证），因此本函数
/// 不会失败；下游可直接将其作为 function-calling 的参数 schema 投递。每个
/// 命名 `$defs` 均可通过 `#/$defs/名字` 自引用，使递归结构（如 [`AgentAction::Batch`]
/// 内含若干部 [`AgentAction`]）能被简洁表达而不必内联展开。
pub fn action_json_schema() -> Value {
    json!({
        // 声明所用 JSON Schema 草案版本，供外部校验器识别。
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "AgentAction",
        "description": "A single high-level plotting intent. The server fills in \
            server-owned facts (before-state, minted ids, resolved layer) and \
            validates the request before anything mutates the document.",
        "$ref": "#/$defs/AgentAction",
        "$defs": {
            // 地理控制点：经/纬度（度）+ 高度（米），三字段均必填。
            "GeoPoint": {
                "type": "object",
                "description": "Longitude / latitude degrees + height metres.",
                "properties": {
                    "lon_deg": { "type": "number" },
                    "lat_deg": { "type": "number" },
                    "height_m": { "type": "number" }
                },
                "required": ["lon_deg", "lat_deg", "height_m"],
                "additionalProperties": false
            },
            // RGBA 颜色：固定 4 个 0..=1 的分量，以定长数组表达。
            "Rgba": {
                "type": "array",
                // 固定长度 4：r/g/b/a 四分量，均取值 0..=1。
                "description": "RGBA colour, components in 0..=1.",
                "items": { "type": "number" },
                "minItems": 4,
                "maxItems": 4
            },
            // 绘制类型：Create 所画的基元，走与交互工具相同的 draft 折叠规则。
            "DrawKind": {
                "type": "string",
                "description": "Primitive a Create is drawn as; folded through \
                    the same draft-commit rules as the interactive tool.",
                "enum": ["Point", "Polyline", "Polygon", "Rectangle", "Circle"]
                // 五种基元与 commit_draft 的最小点数规则一一对应。
            },
            // 高度参考基准：相对地面/贴地/绝对等定位方式。
            // 枚举取值为四个固定标签，与内核 HeightReference 一一对应。
            "HeightReference": {
                "type": "string",
                "enum": ["None", "ClampToGround", "RelativeToGround", "Absolute"]
            },
            // 几何类型标签：内核支持的全部几何种类（含富符号占位）。
            "GeometryKind": {
                "type": "string",
                "enum": [
                    "Point", "Icon", "Label", "Line", "Polygon", "Rectangle",
                    "Circle", "Ellipse", "Arc", "Path", "Composite"
                ]
            },
            // 颜色、不透明度、线宽、填充及点尺等样式字段（均为可选增量）。
            "StylePatch": {
                "type": "object",
                "description": "Incremental style edit; only present fields apply.",
                "properties": {
                    "color": { "$ref": "#/$defs/Rgba" },
                    "opacity": { "type": "number" },
                    "width_px": { "type": "number" },
                    "fill": { "$ref": "#/$defs/Rgba" },
                    "point_size_px": { "type": "number" },
                    "z_order": { "type": "integer" },
                    "height_reference": { "$ref": "#/$defs/HeightReference" },
                    "depth_test": { "type": "boolean" },
                    "show_in_flat": { "type": "boolean" },
                    "show_in_globe": { "type": "boolean" }
                },
                "additionalProperties": false
            },
            // 完整几何并（外部标签）：基本形状为常见情形，其余为符号组合预留位。
            "Geometry": {
                "description": "Full geometry union (externally-tagged). Basic \
                    shapes are the common case; richer variants are the reserved \
                    symbol-composition slot.",
                // 具体形状逐一定义；其余富类型以 `true` 占位（接受任意对象）。
                "oneOf": [
                    { "type": "object", "properties": { "Point": { "$ref": "#/$defs/GeoPoint" } }, "required": ["Point"], "additionalProperties": false },
                    { "type": "object", "properties": { "Polyline": { "type": "object", "properties": { "positions": { "type": "array", "items": { "$ref": "#/$defs/GeoPoint" } } }, "required": ["positions"], "additionalProperties": false } }, "required": ["Polyline"], "additionalProperties": false },
                    { "type": "object", "properties": { "Polygon": { "type": "object", "properties": { "outer": { "type": "array", "items": { "$ref": "#/$defs/GeoPoint" } }, "holes": { "type": "array", "items": { "type": "array", "items": { "$ref": "#/$defs/GeoPoint" } } } }, "required": ["outer", "holes"], "additionalProperties": false } }, "required": ["Polygon"], "additionalProperties": false },
                    { "type": "object", "properties": { "Rectangle": { "type": "object", "properties": { "west": { "type": "number" }, "south": { "type": "number" }, "east": { "type": "number" }, "north": { "type": "number" } }, "required": ["west", "south", "east", "north"], "additionalProperties": false } }, "required": ["Rectangle"], "additionalProperties": false },
                    { "type": "object", "properties": { "Circle": true } },
                    { "type": "object", "properties": { "Icon": true } },
                    { "type": "object", "properties": { "Label": true } },
                    { "type": "object", "properties": { "Ellipse": true } },
                    { "type": "object", "properties": { "Arc": true } },
                    { "type": "object", "properties": { "Path": true } },
                    { "type": "object", "properties": { "Composite": true } }
                ]
            },
            // 元素/图层的整数 id（非负）；两者均为单调递增的内部计数器值。
            // 自由形式的业务属性集合见下方 Attributes 定义。
            "ElementId": { "type": "integer", "minimum": 0 },
            "LayerId": { "type": "integer", "minimum": 0 },
            // 自由形式业务属性：任意键值映射（敌我/番号/状态等），不限制具体字段。
            "Attributes": {
                "type": "object",
                "description": "Free-form business attributes (敌我 / 番号 / 状态 …).",
                "additionalProperties": true
            },
            // 动作信封：八个变体以 oneOf 表达，每个都是单键外部标签对象。
            "AgentAction": {
                "oneOf": [
                    {
                        "type": "object",
                        "properties": {
                            // 创建：由绘制类型 + 控制点新建一个图元（必填 kind/positions）。
                            "Create": {
                                "type": "object",
                                "properties": {
                                    // kind 与 positions 必填；其余可缺省（anyOf 含 null）。
                                    "kind": { "$ref": "#/$defs/DrawKind" },
                                    "positions": { "type": "array", "items": { "$ref": "#/$defs/GeoPoint" } },
                                    "name": { "type": ["string", "null"] },
                                    "style": {
                                        "anyOf": [
                                            { "$ref": "#/$defs/StylePatch" },
                                            { "type": "null" }
                                        ]
                                    },
                                    "attributes": {
                                        "anyOf": [
                                            { "$ref": "#/$defs/Attributes" },
                                            { "type": "null" }
                                        ]
                                    },
                                    "layer": {
                                        "anyOf": [
                                            { "$ref": "#/$defs/LayerId" },
                                            { "type": "null" }
                                        ]
                                    }
                                },
                                "required": ["kind", "positions"],
                                "additionalProperties": false
                            }
                        },
                        "required": ["Create"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            // 删除：按 id 移除一个元素（命令中保留完整副本以便 undo）。
                            "Delete": {
                                "type": "object",
                                "properties": { "target": { "$ref": "#/$defs/ElementId" } },
                                "required": ["target"],
                                "additionalProperties": false
                            }
                        },
                        "required": ["Delete"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            // 移动：将元素所有顶点平移 (dlon, dlat) 度。
                            "Move": {
                                "type": "object",
                                "properties": {
                                    "target": { "$ref": "#/$defs/ElementId" },
                                    // 平移量：[dlon, dlat] 两元素数组，单位为度。
                                    "delta_lonlat": {
                                        "type": "array",
                                        "items": { "type": "number" },
                                        "minItems": 2,
                                        "maxItems": 2
                                    }
                                },
                                "required": ["target", "delta_lonlat"],
                                "additionalProperties": false
                            }
                        },
                        "required": ["Move"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            // 置几何：整体替换一个元素的几何。
                            "SetGeometry": {
                                "type": "object",
                                "properties": {
                                    "target": { "$ref": "#/$defs/ElementId" },
                                    "geometry": { "$ref": "#/$defs/Geometry" }
                                },
                                "required": ["target", "geometry"],
                                "additionalProperties": false
                            }
                        },
                        "required": ["SetGeometry"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            // 样式：对一个元素应用增量样式补丁。
                            "Style": {
                                "type": "object",
                                "properties": {
                                    "target": { "$ref": "#/$defs/ElementId" },
                                    "patch": { "$ref": "#/$defs/StylePatch" }
                                },
                                "required": ["target", "patch"],
                                "additionalProperties": false
                            }
                        },
                        "required": ["Style"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            // 改属性：合并自由形式业务属性，同名键覆盖。
                            "SetAttributes": {
                                "type": "object",
                                "properties": {
                                    "target": { "$ref": "#/$defs/ElementId" },
                                    "merge": { "$ref": "#/$defs/Attributes" }
                                },
                                "required": ["target", "merge"],
                                "additionalProperties": false
                            }
                        },
                        "required": ["SetAttributes"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            // 显隐：翻转一个元素的手动可见性标志。
                            "SetVisible": {
                                "type": "object",
                                "properties": {
                                    "target": { "$ref": "#/$defs/ElementId" },
                                    // 目标手动可见性标志；true 强制显示、false 强制隐藏。
                                    "visible": { "type": "boolean" }
                                },
                                "required": ["target", "visible"],
                                "additionalProperties": false
                            }
                        },
                        "required": ["SetVisible"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            // 批量：多个动作编成一个复合命令，单个 undo 步骤。
                            "Batch": {
                                "type": "object",
                                "properties": {
                                    "actions": {
                                        "type": "array",
                                        "items": { "$ref": "#/$defs/AgentAction" }
                                    }
                                },
                                "required": ["actions"],
                                // Batch 仅携带一个动作数组；子动作递归引用 $defs/AgentAction。
                                "additionalProperties": false
                            }
                        },
                        "required": ["Batch"],
                        "additionalProperties": false
                    }
                ]
            }
        }
    })
}

/// 一份规范的 [`AgentAction`] 负载。既作文档，也作回归锚点：测试把它
/// 解析回真实类型，因此手写的 [`action_json_schema`] 不会悄然偏离该枚举。
///
/// 负载展示了一个典型创建意图：沿两个控制点画一条折线，附带名称、样式
/// 补丁（颜色与线宽）与自由形式属性。因为它是 `r#"..."#` 原始字符串，
/// 内部内容属于数据而非注释，任何修改都需同步更新回归测试。
pub const EXAMPLE_ACTION: &str = r#"{
    "Create": {
        "kind": "Polyline",
        "positions": [
            { "lon_deg": 116.39, "lat_deg": 39.91, "height_m": 0.0 },
            { "lon_deg": 116.45, "lat_deg": 39.94, "height_m": 0.0 }
        ],
        "name": "route-a",
        "style": { "color": [1.0, 0.2, 0.2, 1.0], "width_px": 3.0 },
        "attributes": { "side": "friendly", "priority": 1 }
    }
}"#;

/// 通过无损 GeoJSON 门面序列化整个文档。
///
/// 内存中文档的序列化是不会失败的（每个字段都是普通的 `Serialize` 数据，
/// 且编码器是全函数），所以这里直接返回一个 `String`，而不泄露读取器
/// 可能产生的 [`PlotIoError`]。`expect` 仅用于文档化这个不变量。
pub fn export_document_json(doc: &Document) -> String {
    to_geojson(doc).expect("document JSON serialisation is infallible")
}

/// 从 [`export_document_json`] 负载（或任何 GeoJSON `FeatureCollection`，
/// 尽最大努力导入）重建一个 [`Document`]。
///
/// 与导出共用同一套编解码实现，因此一个由 [`export_document_json`] 写出的
/// 文档经本函数回读应与原文档逐字段相等（无损往返）。
pub fn import_document_json(text: &str) -> Result<Document, PlotIoError> {
    from_geojson(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::action::AgentAction;
    use crate::geo::GeoPoint;
    use crate::model::{Document, HeightReference};
    use serde_json::Value;

    /// 校验手写 schema 仍是有效 JSON 且含约定的地标键与 8 个动作变体，
    /// 并确保经文本编解码往返后结构自洽（无隐性偏离）。
    #[test]
    fn schema_is_valid_json_with_expected_shape() {
        let schema = action_json_schema();
        // `Value` 依构造即为有效；断言这些契约地标，使一个把它们删掉的
        // 粗心编辑会大声失败。地标包括草案版本、标题与顶层 $defs 集合。
        assert!(schema["$schema"].as_str().unwrap().contains("2020-12"));
        assert_eq!(schema["title"], Value::String("AgentAction".into()));
        let defs = schema["$defs"].as_object().unwrap();
        // 逐个检查代表性 $defs 键存在，防止重构时误删公共定义。
        for key in [
            "GeoPoint",
            "Geometry",
            "StylePatch",
            "DrawKind",
            "AgentAction",
        ] {
            assert!(defs.contains_key(key), "missing $defs.{key}");
        }
        let variants = defs["AgentAction"]["oneOf"].as_array().unwrap();
        // 八个变体：Create/Delete/Move/SetGeometry/Style/SetAttributes/SetVisible/Batch。
        assert_eq!(variants.len(), 8, "eight action variants");

        // 经由文本 JSON 编码器往返而无错误。验证 schema 本身可安全序列化。
        let text = serde_json::to_string(&schema).unwrap();
        let reparsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(schema, reparsed);
    }

    /// 将示例负载解析回真实的 [`AgentAction`] 并逐字段断言，再验证
    /// 序列化→反序列化幂等，从而钉住 schema 与类型的一致性。
    #[test]
    fn example_action_parses_into_the_real_type() {
        let action: AgentAction = serde_json::from_str(EXAMPLE_ACTION).unwrap();
        // 展开为一例 Create：验证控制点数、名称、样式补丁与属性都与负载一致。
        match &action {
            AgentAction::Create {
                positions,
                name,
                style,
                attributes,
                ..
            } => {
                // 逐项断言负载字段解析后的实际值。
                assert_eq!(positions.len(), 2);
                assert_eq!(name.as_deref(), Some("route-a"));
                let patch = style.as_ref().unwrap();
                assert_eq!(patch.width_px, Some(3.0));
                assert_eq!(patch.color, Some([1.0, 0.2, 0.2, 1.0]));
                assert_eq!(
                    attributes.as_ref().unwrap()["side"],
                    Value::String("friendly".into())
                );
            }
            other => panic!("example did not parse as Create: {other:?}"),
        }
        // 重新序列化再重新解析是稳定的（schema ↔ 类型不偏离）。
        let again: AgentAction =
            serde_json::from_str(&serde_json::to_string(&action).unwrap()).unwrap();
        assert_eq!(action, again);
    }

    /// 端到端验证门面：先用 [`apply_action`] 创建一个矩形元素，再导出
    /// 并回读，断言元素数量与整文档逐字段无损还原。
    #[test]
    fn document_roundtrips_losslessly_through_the_facade() {
        use crate::agent::action::{apply_action, StylePatch};
        use crate::ops::{DrawKind, HistoryStack};
        // 带默认图层的空文档：提供一个可写入的活动图层供 Create 解析。
        let mut doc = Document::with_default_layer();
        let mut history = HistoryStack::new();
        // 施加一个矩形 Create：两点确定对角，名称与贴地样式随元素一并写入。
        apply_action(
            &mut doc,
            &mut history,
            &AgentAction::Create {
                kind: DrawKind::Rectangle,
                positions: vec![
                    GeoPoint::surface(0.0, 0.0),
                    GeoPoint::surface(2.0, 2.0),
                ],
                name: Some("zone".into()),
                style: Some(StylePatch {
                    height_reference: Some(HeightReference::ClampToGround),
                    ..Default::default()
                }),
                attributes: None,
                // 未指定 layer：compile 会回退到文档活动图层。
                layer: None,
            },
        )
        .unwrap();

        // 导出为 JSON 文本再回读，逐字段比对应与原文档完全相等。
        let text = export_document_json(&doc);
        let back = import_document_json(&text).unwrap();
        assert_eq!(back.element_count(), 1);
        // 无损的 `x-plot` 负载精确还原整个文档。
        assert_eq!(back, doc);
    }

    #[test]
    fn import_rejects_non_json() {
        // 非 JSON 输入必须报错而非 panic，保证门面健壮。
        // 导入路径应把解析失败包装为 [`PlotIoError`] 而非直接崩溃。
        assert!(import_document_json("not json at all").is_err());
    }
}
