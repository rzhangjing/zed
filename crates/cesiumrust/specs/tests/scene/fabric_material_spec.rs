//! Fabric 材质系统规范
//! 参考自 CesiumJS Scene/MaterialSpec

use cesium_material::{
    uniform_value_from_json, BUILTIN_MATERIAL_TYPES, FabricTemplate,
    MaterialComponents, MaterialError, MaterialOptions, MaterialSystem, TranslucentSpec,
    UniformValue, COMPONENT_PROPERTIES, TEMPLATE_PROPERTIES,
};
use serde_json::json;
use std::collections::BTreeMap;

// ==================== 常量 ====================

#[test]
fn template_properties_match_cesiumjs() {
    assert_eq!(
        TEMPLATE_PROPERTIES,
        ["type", "materials", "uniforms", "components", "source"]
    );
}

#[test]
fn component_properties_match_cesiumjs() {
    assert_eq!(
        COMPONENT_PROPERTIES,
        ["diffuse", "specular", "shininess", "normal", "emission", "alpha"]
    );
}

// ==================== UniformValue ====================

#[test]
fn uniform_value_glsl_types() {
    assert_eq!(UniformValue::Float(1.0).glsl_type(), "float");
    assert_eq!(UniformValue::Bool(true).glsl_type(), "bool");
    assert_eq!(UniformValue::Vec2([0.0; 2]).glsl_type(), "vec2");
    assert_eq!(UniformValue::Vec3([0.0; 3]).glsl_type(), "vec3");
    assert_eq!(UniformValue::Vec4([0.0; 4]).glsl_type(), "vec4");
    assert_eq!(UniformValue::Mat3([0.0; 9]).glsl_type(), "mat3");
    assert_eq!(UniformValue::Mat4([0.0; 16]).glsl_type(), "mat4");
    assert_eq!(UniformValue::Sampler2D("x".into()).glsl_type(), "sampler2D");
    assert_eq!(UniformValue::Channels("rgb".into()).glsl_type(), "channels");
}

#[test]
fn uniform_value_from_json_number() {
    let v = uniform_value_from_json(&json!(3.14)).unwrap();
    assert_eq!(v, UniformValue::Float(3.14));
}

#[test]
fn uniform_value_from_json_bool() {
    let v = uniform_value_from_json(&json!(true)).unwrap();
    assert_eq!(v, UniformValue::Bool(true));
}

#[test]
fn uniform_value_from_json_vec2_xy() {
    let v = uniform_value_from_json(&json!({"x": 1.0, "y": 2.0})).unwrap();
    assert_eq!(v, UniformValue::Vec2([1.0, 2.0]));
}

#[test]
fn uniform_value_from_json_vec4_rgba() {
    let v = uniform_value_from_json(&json!({"red": 0.5, "green": 0.6, "blue": 0.7, "alpha": 0.8}))
        .unwrap();
    assert_eq!(v, UniformValue::Vec4([0.5, 0.6, 0.7, 0.8]));
}

#[test]
fn uniform_value_from_json_string_is_sampler() {
    let v = uniform_value_from_json(&json!("image.png")).unwrap();
    assert_eq!(v, UniformValue::Sampler2D("image.png".to_string()));
}

#[test]
fn uniform_value_alpha_or_scalar() {
    assert_eq!(UniformValue::Float(0.5).alpha_or_scalar(), Some(0.5));
    assert_eq!(
        UniformValue::Vec4([1.0, 0.0, 0.0, 0.3]).alpha_or_scalar(),
        Some(0.3)
    );
    assert_eq!(UniformValue::Vec2([1.0, 2.0]).alpha_or_scalar(), None);
}

// ==================== MaterialComponents ====================

#[test]
fn material_components_is_empty() {
    let c = MaterialComponents::default();
    assert!(c.is_empty());

    let c2 = MaterialComponents {
        diffuse: Some("vec3(1.0)".to_string()),
        ..Default::default()
    };
    assert!(!c2.is_empty());
}

#[test]
fn material_components_iter_canonical_order() {
    let c = MaterialComponents {
        diffuse: Some("d".to_string()),
        specular: Some("s".to_string()),
        shininess: Some("sh".to_string()),
        normal: Some("n".to_string()),
        emission: Some("e".to_string()),
        alpha: Some("a".to_string()),
    };
    let names: Vec<_> = c.iter().map(|(n, _)| n).collect();
    assert_eq!(names, vec!["diffuse", "specular", "shininess", "normal", "emission", "alpha"]);
}

#[test]
fn material_components_iter_skips_none() {
    let c = MaterialComponents {
        diffuse: Some("color.rgb".to_string()),
        alpha: Some("color.a".to_string()),
        ..Default::default()
    };
    let names: Vec<_> = c.iter().map(|(n, _)| n).collect();
    assert_eq!(names, vec!["diffuse", "alpha"]);
}

// ==================== FabricTemplate 解析 ====================

#[test]
fn fabric_parse_empty_object() {
    let t = FabricTemplate::from_json_str("{}").unwrap();
    assert!(t.type_name.is_none());
    assert!(t.uniforms.is_empty());
    assert!(t.materials.is_empty());
    assert!(t.components.is_none());
    assert!(t.source.is_none());
}

#[test]
fn fabric_parse_type_and_uniforms() {
    let t = FabricTemplate::from_json(&json!({
        "type": "Color",
        "uniforms": {
            "color": {"red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 0.5}
        }
    }))
    .unwrap();
    assert_eq!(t.type_name.as_deref(), Some("Color"));
    assert_eq!(
        t.uniforms.get("color"),
        Some(&UniformValue::Vec4([1.0, 0.0, 0.0, 0.5]))
    );
}

#[test]
fn fabric_parse_nested_materials() {
    let t = FabricTemplate::from_json(&json!({
        "materials": {
            "diffuseMap": {"type": "DiffuseMap", "uniforms": {"image": "test.png"}}
        },
        "components": {"diffuse": "diffuseMap.diffuse"}
    }))
    .unwrap();
    assert_eq!(t.materials.len(), 1);
    let sub = t.materials.get("diffuseMap").unwrap();
    assert_eq!(sub.type_name.as_deref(), Some("DiffuseMap"));
}

#[test]
fn fabric_invalid_top_level_property_errors() {
    let err = FabricTemplate::from_json(&json!({"invalid_prop": 1})).unwrap_err();
    assert!(matches!(err, MaterialError::InvalidPropertyName { .. }));
}

#[test]
fn fabric_invalid_component_property_errors() {
    let err = FabricTemplate::from_json(&json!({
        "components": {"glossy": "1.0"}
    }))
    .unwrap_err();
    assert!(matches!(err, MaterialError::InvalidPropertyName { .. }));
}

// ==================== FabricTemplate 校验 ====================

#[test]
fn fabric_validate_source_and_components_conflict() {
    let t = FabricTemplate::from_json(&json!({
        "source": "czm_material czm_getMaterial(czm_materialInput mi) {}",
        "components": {"diffuse": "vec3(1.0)"}
    }))
    .unwrap();
    assert_eq!(t.validate(), Err(MaterialError::SourceAndComponents));
}

#[test]
fn fabric_validate_uniform_material_name_conflict() {
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

#[test]
fn fabric_validate_valid_template_passes() {
    let t = FabricTemplate::from_json(&json!({
        "type": "Color",
        "uniforms": {"color": {"red": 1.0, "green": 1.0, "blue": 1.0, "alpha": 1.0}},
        "components": {"diffuse": "color.rgb", "alpha": "color.a"}
    }))
    .unwrap();
    assert!(t.validate().is_ok());
}

// ==================== FabricTemplate 合并 ====================

#[test]
fn fabric_merge_user_wins_over_base() {
    let mut user = FabricTemplate::from_json(&json!({
        "type": "Color",
        "uniforms": {"color": {"red": 0.0, "green": 1.0, "blue": 0.0, "alpha": 1.0}}
    }))
    .unwrap();
    let base = FabricTemplate::from_json(&json!({
        "type": "Color",
        "uniforms": {
            "color": {"red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 0.5},
            "extra": 2.0
        },
        "components": {"diffuse": "color.rgb", "alpha": "color.a"}
    }))
    .unwrap();

    user.merge_over(&base);
    // 用户的 color 生效
    assert_eq!(
        user.uniforms.get("color"),
        Some(&UniformValue::Vec4([0.0, 1.0, 0.0, 1.0]))
    );
    // 从 base 填充 Extra
    assert_eq!(user.uniforms.get("extra"), Some(&UniformValue::Float(2.0)));
    // 从 base 填充 Components
    assert!(user.components.is_some());
}

// ==================== TranslucentSpec ====================

#[test]
fn translucent_spec_always_never() {
    let empty = BTreeMap::new();
    assert!(TranslucentSpec::Always.evaluate(&empty));
    assert!(!TranslucentSpec::Never.evaluate(&empty));
}

#[test]
fn translucent_spec_any_alpha_lt1() {
    let spec = TranslucentSpec::AnyAlphaLt1(vec!["color"]);
    let mut uniforms = BTreeMap::new();
    uniforms.insert("color".to_string(), UniformValue::Vec4([1.0, 0.0, 0.0, 0.5]));
    assert!(spec.evaluate(&uniforms));

    uniforms.insert("color".to_string(), UniformValue::Vec4([1.0, 0.0, 0.0, 1.0]));
    assert!(!spec.evaluate(&uniforms));
}

#[test]
fn translucent_spec_missing_uniform_not_translucent() {
    let spec = TranslucentSpec::AnyAlphaLt1(vec!["color"]);
    let empty = BTreeMap::new();
    assert!(!spec.evaluate(&empty));
}

// ==================== MaterialSystem ====================

#[test]
fn material_system_builtin_has_25_types() {
    let system = MaterialSystem::with_builtin_materials();
    assert_eq!(system.len(), 25);
    assert!(!system.is_empty());
}

#[test]
fn material_system_builtin_types_list() {
    // 验证一些已知类型存在
    let system = MaterialSystem::with_builtin_materials();
    assert!(system.get_material("Color").is_some());
    assert!(system.get_material("Image").is_some());
    assert!(system.get_material("DiffuseMap").is_some());
    assert!(system.get_material("AlphaMap").is_some());
    assert!(system.get_material("SpecularMap").is_some());
    assert!(system.get_material("Grid").is_some());
    assert!(system.get_material("Stripe").is_some());
    assert!(system.get_material("Checkerboard").is_some());
    assert!(system.get_material("Dot").is_some());
    assert!(system.get_material("Water").is_some());
    assert!(system.get_material("Nonexistent").is_none());
}

#[test]
fn material_system_from_type_color() {
    let system = MaterialSystem::with_builtin_materials();
    let material = system.from_type("Color", BTreeMap::new()).unwrap();
    assert_eq!(material.type_name(), "Color");
    // 默认 color uniform 应存在
    assert!(material.uniforms().contains_key("color"));
}

#[test]
fn material_system_from_type_with_overrides() {
    let system = MaterialSystem::with_builtin_materials();
    let mut overrides = BTreeMap::new();
    overrides.insert(
        "color".to_string(),
        UniformValue::Vec4([0.0, 1.0, 0.0, 1.0]),
    );
    let material = system.from_type("Color", overrides).unwrap();
    assert_eq!(
        material.uniforms().get("color"),
        Some(&UniformValue::Vec4([0.0, 1.0, 0.0, 1.0]))
    );
}

#[test]
fn material_system_from_type_unknown_errors() {
    let system = MaterialSystem::with_builtin_materials();
    let err = system.from_type("DoesNotExist", BTreeMap::new()).unwrap_err();
    assert!(matches!(err, MaterialError::UnknownMaterialType { .. }));
}

#[test]
fn material_system_create_material_new_type() {
    let mut system = MaterialSystem::with_builtin_materials();
    let initial_len = system.len();

    let options = MaterialOptions {
        strict: false,
        translucent: None,
        fabric: FabricTemplate::from_json(&json!({
            "type": "MyCustom",
            "uniforms": {"brightness": 0.5},
            "components": {"diffuse": "vec3(brightness)"}
        }))
        .unwrap(),
    };

    let material = system.create_material(options).unwrap();
    assert_eq!(material.type_name(), "MyCustom");
    assert_eq!(system.len(), initial_len + 1);
    assert!(system.get_material("MyCustom").is_some());
}

#[test]
fn material_is_translucent_color_alpha() {
    let system = MaterialSystem::with_builtin_materials();
    // 默认 Color 的 alpha=0.5 → 半透明
    let material = system.from_type("Color", BTreeMap::new()).unwrap();
    assert!(material.is_translucent());

    // 覆盖 alpha 为 1.0 → 不透明
    let mut overrides = BTreeMap::new();
    overrides.insert(
        "color".to_string(),
        UniformValue::Vec4([1.0, 0.0, 0.0, 1.0]),
    );
    let opaque = system.from_type("Color", overrides).unwrap();
    assert!(!opaque.is_translucent());
}

#[test]
fn material_shader_source_nonempty() {
    let system = MaterialSystem::with_builtin_materials();
    let material = system.from_type("Color", BTreeMap::new()).unwrap();
    assert!(!material.shader_source().is_empty());
    assert!(material.shader_source().contains("czm_getMaterial"));
}

#[test]
fn builtin_material_types_constant() {
    // BUILTIN_MATERIAL_TYPES 应列出全部 25 个
    assert_eq!(BUILTIN_MATERIAL_TYPES.len(), 25);
    assert!(BUILTIN_MATERIAL_TYPES.contains(&"Color"));
    assert!(BUILTIN_MATERIAL_TYPES.contains(&"Water"));
}
