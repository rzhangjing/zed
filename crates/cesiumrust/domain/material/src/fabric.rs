//! Fabric JSON schema：声明式的材质描述语言。
//!
//! `fabric` 选项以声明式 JSON 描述一个材质。Fabric 模板是一个
//! 最多含五个属性的 JSON 对象：
//!
//! - `type`：材质类型名（已存在的或新的）
//! - `uniforms`：uniform 名 → 值 的映射
//! - `materials`：子材质名 → 嵌套 Fabric 模板 的映射
//! - `components`：`czm_material` 分量表达式
//!   （`diffuse`/`specular`/`shininess`/`normal`/`emission`/`alpha`）
//! - `source`：完整自定义的 `czm_getMaterial` GLSL 定义
//!
//! `source` 与 `components` 互斥，二者只能提供其一；若两者均缺失，
//! 则模板被视为空定义，交由材质类型缓存的默认模板填充。

use crate::error::MaterialError;
use crate::uniform::{uniform_value_from_json, UniformValue};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

/// Fabric 模板的合法顶层属性。
/// 解析时据此校验顶层键名。
pub const TEMPLATE_PROPERTIES: [&str; 5] =
    ["type", "materials", "uniforms", "components", "source"];

/// Fabric `components` 对象的合法属性。
/// 解析时据此校验分量键名。
pub const COMPONENT_PROPERTIES: [&str; 6] = [
    "diffuse", "specular", "shininess", "normal", "emission", "alpha",
];

/// Fabric 模板的 `czm_material` 分量表达式。
///
/// 每个条目是一个 GLSL
/// 表达式字符串，在生成的 `czm_getMaterial` 函数体中被赋给对应的
/// `czm_material` 成员。着色器生成的迭代顺序为规范顺序：
/// diffuse、specular、shininess、normal、emission、alpha。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialComponents {
    /// `material.diffuse = <expr>;`（除非融合，否则进行 gamma 校正）。
    pub diffuse: Option<String>,
    /// `material.specular = <expr>;`
    pub specular: Option<String>,
    /// `material.shininess = <expr>;`
    pub shininess: Option<String>,
    /// `material.normal = <expr>;`
    pub normal: Option<String>,
    /// `material.emission = <expr>;`（除非融合，否则进行 gamma 校正）。
    pub emission: Option<String>,
    /// `material.alpha = <expr>;`
    pub alpha: Option<String>,
}

impl MaterialComponents {
    /// 当未设置任何分量表达式时返回 true。
    pub fn is_empty(&self) -> bool {
        // 逐一检查六个分量均为 None
        self.diffuse.is_none()
            && self.specular.is_none()
            && self.shininess.is_none()
            && self.normal.is_none()
            && self.emission.is_none()
            && self.alpha.is_none()
    }

    /// 按规范顺序（diffuse→alpha）迭代各分量表达式。
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &str)> {
        // 固定分量顺序，过滤掉未设置的 None 项
        [
            ("diffuse", self.diffuse.as_deref()),
            ("specular", self.specular.as_deref()),
            ("shininess", self.shininess.as_deref()),
            ("normal", self.normal.as_deref()),
            ("emission", self.emission.as_deref()),
            ("alpha", self.alpha.as_deref()),
        ]
        .into_iter()
        .filter_map(|(name, expr)| expr.map(|e| (name, e)))
    }

    /// 从 JSON 解析 `components` 对象；未提供时返回 None。
    fn parse(json: &JsonValue) -> Result<Option<Self>, MaterialError> {
        let map = match json {
            // Null 表示模板未声明 components
            JsonValue::Null => return Ok(None),
            JsonValue::Object(map) => map,
            // 非对象即为非法的分量结构
            _ => {
                return Err(MaterialError::InvalidPropertyName {
                    property: "<components>".to_string(),
                    expected: COMPONENT_PROPERTIES.join(", "),
                })
            }
        };

        // 逐键校验是否属于合法分量名（对未知键报 invalidName 错误）
        for key in map.keys() {
            if !COMPONENT_PROPERTIES.contains(&key.as_str()) {
                return Err(MaterialError::InvalidPropertyName {
                    property: key.clone(),
                    expected: "'diffuse', 'specular', 'shininess', 'normal', 'emission', or 'alpha'"
                        .to_string(),
                });
            }
        }

        // 提取字符串表达式，缺失或非字符串时返回 None
        // （统一封装为按键取值的闭包，供下方逐分量复用）
        let expr = |key: &str| -> Option<String> {
            map.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
        };

        // 逐分量装配为 MaterialComponents
        Ok(Some(MaterialComponents {
            diffuse: expr("diffuse"),
            specular: expr("specular"),
            shininess: expr("shininess"),
            normal: expr("normal"),
            emission: expr("emission"),
            alpha: expr("alpha"),
        }))
    }
}

/// 已解析的 Fabric 材质模板。
///
/// 从原始 `fabric` 选项深拷贝而来，供后续组装与合并使用。
/// 各字段均可为空，组装阶段再与类型默认模板合并。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FabricTemplate {
    /// 材质类型名（`template.type`）；缺失时在材质构造期间生成一个 GUID。
    pub type_name: Option<String>,
    /// uniform 名 → 值（`template.uniforms`）。
    pub uniforms: BTreeMap<String, UniformValue>,
    /// 子材质名 → 嵌套模板（`template.materials`）。
    pub materials: BTreeMap<String, FabricTemplate>,
    /// 分量表达式（`template.components`）。
    pub components: Option<MaterialComponents>,
    /// 自定义的 `czm_getMaterial` GLSL 源码（`template.source`）。
    pub source: Option<String>,
}

impl FabricTemplate {
    /// 从 JSON 值解析 Fabric 模板。
    pub fn from_json(json: &JsonValue) -> Result<Self, MaterialError> {
        // 顶层必须是 JSON 对象；Null 视为默认模板
        let map = match json {
            JsonValue::Object(map) => map,
            JsonValue::Null => return Ok(FabricTemplate::default()),
            _ => {
                return Err(MaterialError::Json(
                    "fabric must be a JSON object".to_string(),
                ))
            }
        };

        // 校验顶层属性名（对未知键报 invalidName 错误）
        for key in map.keys() {
            if !TEMPLATE_PROPERTIES.contains(&key.as_str()) {
                return Err(MaterialError::InvalidPropertyName {
                    property: key.clone(),
                    expected: "'type', 'materials', 'uniforms', 'components', or 'source'"
                        .to_string(),
                });
            }
        }

        // 提取可选的 type 字段（非字符串则视为缺失）
        let type_name = map
            .get("type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut uniforms = BTreeMap::new();
        // 递归解析每个 uniform 值，并将错误上下文回填为具体 uniform 名
        if let Some(JsonValue::Object(uniform_map)) = map.get("uniforms") {
            for (name, value) in uniform_map {
                uniforms.insert(
                    name.clone(),
                    uniform_value_from_json(value).map_err(|e| match e {
                        MaterialError::InvalidUniformValue { reason, .. } => {
                            MaterialError::InvalidUniformValue {
                                uniform: name.clone(),
                                reason,
                            }
                        }
                        other => other,
                    })?,
                );
            }
        }

        let mut materials = BTreeMap::new();
        // 逐个递归解析嵌套子材质模板（存入以子材质名为键的映射）
        if let Some(JsonValue::Object(material_map)) = map.get("materials") {
            for (name, sub_json) in material_map {
                materials.insert(name.clone(), FabricTemplate::from_json(sub_json)?);
            }
        }

        // 解析可选的 components 对象（缺失则为 None）
        let components = match map.get("components") {
            Some(c) => MaterialComponents::parse(c)?,
            None => None,
        };

        // 提取可选的自定义 GLSL source 字符串
        let source = map
            .get("source")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(FabricTemplate {
            type_name,
            uniforms,
            materials,
            components,
            source,
        })
    }

    /// 从 JSON 字符串解析 Fabric 模板。
    /// 先反序列化为 JSON 值，再委托 [`FabricTemplate::from_json`]。
    pub fn from_json_str(json: &str) -> Result<Self, MaterialError> {
        // 先将文本反序列化为 JSON 值，再走统一的对象解析路径
        let value: JsonValue = serde_json::from_str(json)?;
        Self::from_json(&value)
    }

    /// 校验模板的结构错误。
    ///
    /// 校验规则：
    /// - `source` 与 `components` 不能共存
    /// - uniforms 与 materials 不能共享同名
    ///
    /// 属性名校验已在解析期间完成。
    pub fn validate(&self) -> Result<(), MaterialError> {
        // source 与 components 互斥
        if self.components.is_some() && self.source.is_some() {
            return Err(MaterialError::SourceAndComponents);
        }
        // uniforms 与 materials 不得共享同名
        for name in self.uniforms.keys() {
            if self.materials.contains_key(name) {
                return Err(MaterialError::DuplicateUniformMaterialName {
                    name: name.clone(),
                });
            }
        }
        // 递归校验子材质
        for sub in self.materials.values() {
            sub.validate()?;
        }
        Ok(())
    }

    /// 将 `base` 深合并进 `self`，`self` 优先。
    ///
    /// 用于模板合并：用户提供的模板胜出，其中缺失的任何键
    /// 由缓存的（base）模板填充，子材质递归合并。
    pub fn merge_over(&mut self, base: &FabricTemplate) {
        // 类型名缺失时回退到 base
        if self.type_name.is_none() {
            self.type_name = base.type_name.clone();
        }
        // 逐个 uniform：仅在 self 未定义同名键时从 base 填充
        for (name, value) in &base.uniforms {
            self.uniforms.entry(name.clone()).or_insert_with(|| value.clone());
        }
        // 子材质：同名递归合并，否则整体从 base 拷入
        for (name, sub_base) in &base.materials {
            match self.materials.get_mut(name) {
                Some(sub) => sub.merge_over(sub_base),
                None => {
                    self.materials.insert(name.clone(), sub_base.clone());
                }
            }
        }
        // 分量表达式缺失时取 base
        if self.components.is_none() {
            self.components = base.components.clone();
        }
        // 自定义 GLSL 源码缺失时取 base
        if self.source.is_none() {
            self.source = base.source.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 空 JSON 对象应解析为全空的默认模板
    #[test]
    fn test_parse_minimal() {
        let t = FabricTemplate::from_json_str("{}").unwrap();
        assert!(t.type_name.is_none());
        assert!(t.uniforms.is_empty());
        assert!(t.materials.is_empty());
        assert!(t.components.is_none());
        assert!(t.source.is_none());
    }

    // Color 类型 + RGBA color uniform 应解析为 Vec4
    #[test]
    fn test_parse_color_fabric() {
        let t = FabricTemplate::from_json(&json!({
            "type": "Color",
            "uniforms": {
                "color": {"red": 1.0, "green": 1.0, "blue": 0.0, "alpha": 1.0}
            }
        }))
        .unwrap();
        assert_eq!(t.type_name.as_deref(), Some("Color"));
        assert_eq!(
            t.uniforms.get("color"),
            Some(&UniformValue::Vec4([1.0, 1.0, 0.0, 1.0]))
        );
    }

    // components 仅保留出现的分量，且按声明顺序迭代
    #[test]
    fn test_parse_components() {
        let t = FabricTemplate::from_json(&json!({
            "components": {
                "diffuse": "color.rgb",
                "alpha": "color.a"
            }
        }))
        .unwrap();
        let c = t.components.as_ref().unwrap();
        assert_eq!(c.diffuse.as_deref(), Some("color.rgb"));
        assert_eq!(c.alpha.as_deref(), Some("color.a"));
        assert!(c.specular.is_none());
        let names: Vec<_> = c.iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["diffuse", "alpha"]);
    }

    // 嵌套 materials 应递归解析为子 FabricTemplate
    #[test]
    fn test_parse_nested_materials() {
        let t = FabricTemplate::from_json(&json!({
            "materials": {
                "diffuseMap": {
                    "type": "DiffuseMap"
                }
            },
            "components": {
                "diffuse": "diffuseMap.diffuse"
            }
        }))
        .unwrap();
        assert_eq!(t.materials.len(), 1);
        let sub = t.materials.get("diffuseMap").unwrap();
        assert_eq!(sub.type_name.as_deref(), Some("DiffuseMap"));
    }

    // 顶层出现未知属性名应报 InvalidPropertyName
    #[test]
    fn test_invalid_top_level_property() {
        let err = FabricTemplate::from_json(&json!({"bogus": 1})).unwrap_err();
        assert!(matches!(err, MaterialError::InvalidPropertyName { .. }));
    }

    // components 内出现非法分量名应报 InvalidPropertyName
    #[test]
    fn test_invalid_component_property() {
        let err = FabricTemplate::from_json(&json!({
            "components": {"glossy": "1.0"}
        }))
        .unwrap_err();
        assert!(matches!(err, MaterialError::InvalidPropertyName { .. }));
    }

    // source 与 components 同时存在应校验失败
    #[test]
    fn test_source_and_components_conflict() {
        let t = FabricTemplate::from_json(&json!({
            "source": "czm_material czm_getMaterial(czm_materialInput materialInput) { }",
            "components": {"diffuse": "vec3(1.0)"}
        }))
        .unwrap();
        assert_eq!(t.validate(), Err(MaterialError::SourceAndComponents));
    }

    // uniforms 与 materials 同名应校验失败
    #[test]
    fn test_uniform_material_name_conflict() {
        let t = FabricTemplate::from_json(&json!({
            "uniforms": {"shared": 1.0},
            "materials": {"shared": {"type": "Color"}}
        }))
        .unwrap();
        assert_eq!(
            t.validate(),
            Err(MaterialError::DuplicateUniformMaterialName {
                name: "shared".to_string()
            })
        );
    }

    // 合并优先级：用户模板胜出，缺失键由缓存模板填充
    #[test]
    fn test_merge_over_precedence() {
        let mut user = FabricTemplate::from_json(&json!({
            "type": "Color",
            "uniforms": {
                "color": {"red": 0.0, "green": 1.0, "blue": 0.0, "alpha": 1.0}
            }
        }))
        .unwrap();
        let cached = FabricTemplate::from_json(&json!({
            "type": "Color",
            "uniforms": {
                "color": {"red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 0.5},
                "extra": 2.0
            },
            "components": {"diffuse": "color.rgb", "alpha": "color.a"}
        }))
        .unwrap();

        user.merge_over(&cached);

        // 用户的 color 胜出；extra uniform 由缓存填充；components
        // 由缓存填充。
        assert_eq!(
            user.uniforms.get("color"),
            Some(&UniformValue::Vec4([0.0, 1.0, 0.0, 1.0]))
        );
        assert_eq!(
            user.uniforms.get("extra"),
            Some(&UniformValue::Float(2.0))
        );
        assert!(user.components.is_some());
    }
}
