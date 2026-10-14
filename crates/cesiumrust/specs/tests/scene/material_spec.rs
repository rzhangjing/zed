//! Scene/MaterialSpec → Rust integration tests

use cesium_material::{MaterialSystem, FabricTemplate, BUILTIN_MATERIAL_TYPES};

// === MaterialSystem ===

#[test]
fn test_material_system_with_builtins() {
    let _system = MaterialSystem::with_builtin_materials();
    assert!(!BUILTIN_MATERIAL_TYPES.is_empty());
}

#[test]
fn test_builtin_material_types() {
    assert!(BUILTIN_MATERIAL_TYPES.contains(&"Color"));
    assert!(BUILTIN_MATERIAL_TYPES.contains(&"Image"));
    assert!(BUILTIN_MATERIAL_TYPES.contains(&"Grid"));
}

// === FabricTemplate ===

#[test]
fn test_fabric_template_from_json() {
    let json = r#"{
        "type": "Color",
        "uniforms": {
            "color": [1.0, 0.0, 0.0, 1.0]
        }
    }"#;
    let template = FabricTemplate::from_json_str(json).unwrap();
    assert_eq!(template.type_name.as_deref(), Some("Color"));
}

#[test]
fn test_fabric_template_with_components() {
    let json = r#"{
        "components": {
            "diffuse": "vec3(1.0, 0.0, 0.0)",
            "alpha": 1.0
        }
    }"#;
    let template = FabricTemplate::from_json_str(json).unwrap();
    assert!(template.components.is_some());
}
