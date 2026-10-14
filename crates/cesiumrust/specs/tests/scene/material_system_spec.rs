//! 材质系统规范 - 参考自 Scene/MaterialSpec
//!
//! 测试 MaterialSystem、FabricTemplate、Material、MaterialComponents、
//! UniformValue、TranslucentSpec 及内置材质。

use cesium_material::{
    FabricTemplate, MaterialComponents, MaterialOptions, MaterialSystem,
    TranslucentSpec, UniformValue,
};
use serde_json::json;
use std::collections::BTreeMap;

// ─── MaterialComponents ──────────────────────────────────────────────────────

#[test]
fn material_components_default_is_empty() {
    let mc = MaterialComponents::default();
    assert!(mc.is_empty());
    assert!(mc.diffuse.is_none());
    assert!(mc.specular.is_none());
    assert!(mc.shininess.is_none());
    assert!(mc.normal.is_none());
    assert!(mc.emission.is_none());
    assert!(mc.alpha.is_none());
}

#[test]
fn material_components_iter_skips_none() {
    let mc = MaterialComponents {
        diffuse: Some("vec3(1.0)".to_string()),
        specular: None,
        shininess: None,
        normal: None,
        emission: None,
        alpha: Some("1.0".to_string()),
    };
    let items: Vec<_> = mc.iter().collect();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].0, "diffuse");
    assert_eq!(items[1].0, "alpha");
}

#[test]
fn material_components_iter_canonical_order() {
    let mc = MaterialComponents {
        diffuse: Some("vec3(0.5)".to_string()),
        specular: Some("0.1".to_string()),
        shininess: Some("10.0".to_string()),
        normal: Some("vec3(0.0, 0.0, 1.0)".to_string()),
        emission: Some("vec3(0.0)".to_string()),
        alpha: Some("1.0".to_string()),
    };
    let names: Vec<_> = mc.iter().map(|(name, _)| name).collect();
    assert_eq!(
        names,
        vec!["diffuse", "specular", "shininess", "normal", "emission", "alpha"]
    );
}

// ─── 从 JSON 构造 FabricTemplate ────────────────────────────────────────────────

#[test]
fn fabric_template_from_json_with_type() {
    let json_val = json!({
        "type": "Color",
        "uniforms": {
            "color": [1.0, 0.0, 0.0, 1.0]
        }
    });
    let template = FabricTemplate::from_json(&json_val).unwrap();
    assert_eq!(template.type_name, Some("Color".to_string()));
    assert!(template.uniforms.contains_key("color"));
}

#[test]
fn fabric_template_from_json_with_components() {
    let json_val = json!({
        "components": {
            "diffuse": "vec3(0.5, 0.5, 0.5)",
            "specular": "0.0",
            "shininess": "1.0"
        }
    });
    let template = FabricTemplate::from_json(&json_val).unwrap();
    assert!(template.components.is_some());
    let comps = template.components.as_ref().unwrap();
    assert_eq!(comps.diffuse.as_deref(), Some("vec3(0.5, 0.5, 0.5)"));
    assert_eq!(comps.specular.as_deref(), Some("0.0"));
    assert_eq!(comps.shininess.as_deref(), Some("1.0"));
}

#[test]
fn fabric_template_from_json_with_source() {
    let json_val = json!({
        "source": "czm_material czm_getMaterial(czm_materialInput input) { ... }"
    });
    let template = FabricTemplate::from_json(&json_val).unwrap();
    assert!(template.source.is_some());
}

#[test]
fn fabric_template_validate_source_and_components_exclusive() {
    // source 和 components 应都被拒绝
    let json_val = json!({
        "source": "czm_material czm_getMaterial(czm_materialInput input) { ... }",
        "components": {
            "diffuse": "vec3(1.0)"
        }
    });
    let result = FabricTemplate::from_json(&json_val);
    // 应校验失败或优雅处理
    match result {
        Err(_) => {} // 预期 - 校验应拒绝
        Ok(t) => {
            // 若未拒绝，validate() 应捕获它
            let v = t.validate();
            assert!(v.is_err(), "source+components should be invalid");
        }
    }
}

#[test]
fn fabric_template_validate_rejects_unknown_component() {
    let json_val = json!({
        "components": {
            "unknown_component": "vec3(1.0)"
        }
    });
    let result = FabricTemplate::from_json(&json_val);
    // 应解析或校验失败
    match result {
        Err(_) => {} // 预期
        Ok(t) => {
            let v = t.validate();
            assert!(v.is_err(), "unknown component should fail validation");
        }
    }
}

#[test]
fn fabric_template_merge_over() {
    let base_json = json!({
        "uniforms": {
            "color": [1.0, 0.0, 0.0, 1.0]
        }
    });
    let overlay_json = json!({
        "uniforms": {
            "color": [0.0, 1.0, 0.0, 1.0],
            "strength": 0.5
        }
    });
    let base = FabricTemplate::from_json(&base_json).unwrap();
    let mut overlay = FabricTemplate::from_json(&overlay_json).unwrap();
    overlay.merge_over(&base);
    // 覆盖的 uniforms 应生效
    assert!(overlay.uniforms.contains_key("color"));
    assert!(overlay.uniforms.contains_key("strength"));
}

// ─── MaterialSystem ──────────────────────────────────────────────────────────

#[test]
fn material_system_with_builtins() {
    let sys = MaterialSystem::with_builtin_materials();
    assert!(sys.len() > 0, "should have builtin materials");
    assert!(!sys.is_empty());
}

#[test]
fn material_system_from_type_color() {
    let sys = MaterialSystem::with_builtin_materials();
    let result = sys.from_type("Color", BTreeMap::new());
    assert!(result.is_ok(), "Color type should be available: {:?}", result.err());
    let mat = result.unwrap();
    assert_eq!(mat.type_name(), "Color");
}

#[test]
fn material_system_from_type_normal_map() {
    let sys = MaterialSystem::with_builtin_materials();
    let result = sys.from_type("NormalMap", BTreeMap::new());
    assert!(result.is_ok(), "NormalMap type should be available: {:?}", result.err());
}

#[test]
fn material_system_from_type_unknown() {
    let sys = MaterialSystem::with_builtin_materials();
    let result = sys.from_type("NonExistentMaterial", BTreeMap::new());
    assert!(result.is_err(), "unknown type should fail");
}

#[test]
fn material_system_create_material_from_components() {
    let mut sys = MaterialSystem::new();
    let opts = MaterialOptions {
        fabric: FabricTemplate {
            components: Some(MaterialComponents {
                diffuse: Some("vec3(1.0, 0.0, 0.0)".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let result = sys.create_material(opts);
    assert!(result.is_ok());
    let mat = result.unwrap();
    assert!(!mat.shader_source().is_empty());
}

#[test]
fn material_system_get_material_after_add() {
    let mut sys = MaterialSystem::new();
    let opts = MaterialOptions {
        fabric: FabricTemplate {
            type_name: Some("TestMat".to_string()),
            components: Some(MaterialComponents {
                diffuse: Some("vec3(0.5)".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    sys.create_material(opts).unwrap();
    let mat = sys.get_material("TestMat");
    assert!(mat.is_some());
}

// ─── Material ────────────────────────────────────────────────────────────────

#[test]
fn material_shader_source_non_empty() {
    let sys = MaterialSystem::with_builtin_materials();
    let mat = sys.from_type("Color", BTreeMap::new()).unwrap();
    assert!(!mat.shader_source().is_empty());
    assert!(
        mat.shader_source().contains("czm_getMaterial"),
        "shader should contain czm_getMaterial"
    );
}

#[test]
fn material_is_translucent_with_alpha() {
    let mut sys = MaterialSystem::new();
    let opts = MaterialOptions {
        fabric: FabricTemplate {
            components: Some(MaterialComponents {
                diffuse: Some("vec3(1.0)".to_string()),
                alpha: Some("0.5".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let mat = sys.create_material(opts).unwrap();
    // alpha < 1.0 的材质应为半透明
    assert!(mat.is_translucent());
}

#[test]
fn material_uniforms_from_builtin() {
    let sys = MaterialSystem::with_builtin_materials();
    let mat = sys.from_type("Color", BTreeMap::new()).unwrap();
    let uniforms = mat.uniforms();
    assert!(
        uniforms.contains_key("color") || !uniforms.is_empty(),
        "Color material should have uniforms"
    );
}

// ─── TranslucentSpec ─────────────────────────────────────────────────────────

#[test]
fn translucent_spec_always() {
    let spec = TranslucentSpec::Always;
    let empty = BTreeMap::new();
    assert!(spec.evaluate(&empty));
}

#[test]
fn translucent_spec_never() {
    let spec = TranslucentSpec::Never;
    let empty = BTreeMap::new();
    assert!(!spec.evaluate(&empty));
}

#[test]
fn translucent_spec_any_alpha_lt1() {
    let spec = TranslucentSpec::AnyAlphaLt1(vec!["color"]);
    let mut uniforms = BTreeMap::new();
    uniforms.insert("color".to_string(), UniformValue::Vec4([1.0, 0.0, 0.0, 0.5]));
    assert!(spec.evaluate(&uniforms), "alpha < 1.0 should be translucent");
    uniforms.insert("color".to_string(), UniformValue::Vec4([1.0, 0.0, 0.0, 1.0]));
    assert!(!spec.evaluate(&uniforms), "alpha = 1.0 should not be translucent");
}

// ─── UniformValue ────────────────────────────────────────────────────────────

#[test]
fn uniform_value_float() {
    let val = UniformValue::Float(1.5);
    match val {
        UniformValue::Float(f) => assert!((f - 1.5).abs() < 1e-10),
        _ => panic!("expected Float"),
    }
}

#[test]
fn uniform_value_vec3() {
    let val = UniformValue::Vec3([1.0, 2.0, 3.0]);
    match val {
        UniformValue::Vec3(v) => {
            assert!((v[0] - 1.0).abs() < 1e-10);
            assert!((v[1] - 2.0).abs() < 1e-10);
            assert!((v[2] - 3.0).abs() < 1e-10);
        }
        _ => panic!("expected Vec3"),
    }
}

#[test]
fn uniform_value_vec4() {
    let val = UniformValue::Vec4([1.0, 0.0, 0.0, 1.0]);
    match val {
        UniformValue::Vec4(v) => {
            assert!((v[0] - 1.0).abs() < 1e-10);
            assert!((v[3] - 1.0).abs() < 1e-10);
        }
        _ => panic!("expected Vec4"),
    }
}

#[test]
fn uniform_value_bool() {
    let val = UniformValue::Bool(true);
    match val {
        UniformValue::Bool(b) => assert!(b),
        _ => panic!("expected Bool"),
    }
}

#[test]
fn uniform_value_ivector3() {
    let val = UniformValue::IVec3([1, 2, 3]);
    match val {
        UniformValue::IVec3(v) => {
            assert_eq!(v[0], 1);
            assert_eq!(v[1], 2);
            assert_eq!(v[2], 3);
        }
        _ => panic!("expected IVec3"),
    }
}

// ─── 内置材质类型 ─────────────────────────────────────────────────

#[test]
fn builtin_material_types_available() {
    let sys = MaterialSystem::with_builtin_materials();
    // 检查一些常见的内置材质类型
    let expected_types = ["Color", "DiffuseMap", "NormalMap", "Water", "Grid"];
    for type_name in &expected_types {
        assert!(
            sys.get_material(type_name).is_some(),
            "builtin material '{}' should be available", type_name
        );
    }
}

#[test]
fn builtin_material_color_has_correct_uniforms() {
    let sys = MaterialSystem::with_builtin_materials();
    let mat = sys.from_type("Color", BTreeMap::new()).unwrap();
    assert_eq!(mat.type_name(), "Color");
    let has_color_uniform = mat.uniforms().contains_key("color");
    assert!(has_color_uniform, "Color material should have 'color' uniform");
}

#[test]
fn builtin_material_diffuse_map_has_image_uniform() {
    let sys = MaterialSystem::with_builtin_materials();
    let mat = sys.from_type("DiffuseMap", BTreeMap::new()).unwrap();
    assert_eq!(mat.type_name(), "DiffuseMap");
    let has_image = mat.uniforms().contains_key("image");
    assert!(has_image, "DiffuseMap should have 'image' uniform");
}

#[test]
fn builtin_material_grid_has_uniforms() {
    let sys = MaterialSystem::with_builtin_materials();
    let mat = sys.from_type("Grid", BTreeMap::new()).unwrap();
    assert_eq!(mat.type_name(), "Grid");
    assert!(!mat.uniforms().is_empty(), "Grid should have uniforms");
}

// ─── FabricTemplate 边缘情形 ───────────────────────────────────────────────

#[test]
fn fabric_template_empty_json() {
    let json_val = json!({});
    let result = FabricTemplate::from_json(&json_val);
    assert!(result.is_ok(), "empty JSON should parse ok");
    let t = result.unwrap();
    assert!(t.type_name.is_none());
    assert!(t.components.is_none());
    assert!(t.source.is_none());
}

#[test]
fn fabric_template_with_sub_materials() {
    let json_val = json!({
        "materials": {
            "sub1": {
                "type": "Color",
                "uniforms": {
                    "color": [1.0, 0.0, 0.0, 1.0]
                }
            }
        },
        "components": {
            "diffuse": "sub1.diffuse"
        }
    });
    let result = FabricTemplate::from_json(&json_val);
    assert!(result.is_ok());
    let t = result.unwrap();
    assert!(t.materials.contains_key("sub1"));
}

#[test]
fn fabric_template_from_json_str() {
    let json_str = r#"{"type": "Color", "uniforms": {"color": [1.0, 1.0, 1.0, 1.0]}}"#;
    let result = FabricTemplate::from_json_str(json_str);
    assert!(result.is_ok());
    let t = result.unwrap();
    assert_eq!(t.type_name.as_deref(), Some("Color"));
}
