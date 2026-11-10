use bevy::prelude::*;
use cesium_bevy_render::{
    CesiumMaterialPlugin,
    MaterialAnimationTime, MaterialRef, MaterialSystemResource,
    FabricKind,
};
use cesium_material::{MaterialSystem, UniformValue};

use super::create_test_app;

#[test]
fn test_material_plugin_registers() {
    let mut app = create_test_app();
    app.add_plugins(CesiumMaterialPlugin);
}

#[test]
fn test_material_animation_time_resource_initialized() {
    let mut app = create_test_app();
    app.add_plugins(CesiumMaterialPlugin);

    let time = app.world().get_resource::<MaterialAnimationTime>();
    assert!(time.is_some());
}

#[test]
fn test_material_system_resource_initialization() {
    let mut app = create_test_app();

    let system = MaterialSystemResource::with_builtin_materials();
    app.world_mut().insert_resource(system);

    let res = app.world().get_resource::<MaterialSystemResource>();
    assert!(res.is_some());

    let m = res.unwrap().0.from_type("Color", std::collections::BTreeMap::new());
    assert!(m.is_ok());
}

#[test]
fn test_material_ref_component() {
    let mut app = create_test_app();
    app.add_plugins(CesiumMaterialPlugin);

    let mat_ref = MaterialRef::new("Color");

    let entity = app.world_mut().spawn(mat_ref).id();

    let mat_ref = app.world().get::<MaterialRef>(entity);
    assert!(mat_ref.is_some());
    assert_eq!(mat_ref.unwrap().type_name, "Color");
}

#[test]
fn test_material_ref_with_uniforms() {
    let mut app = create_test_app();
    app.add_plugins(CesiumMaterialPlugin);

    let mut uniforms = std::collections::BTreeMap::new();
    uniforms.insert("color".to_string(), UniformValue::Vec4([1.0, 0.0, 0.0, 1.0]));

    let mat_ref = MaterialRef::with_uniforms("Checkerboard", uniforms);

    let entity = app.world_mut().spawn(mat_ref).id();

    let mat_ref = app.world().get::<MaterialRef>(entity);
    assert!(mat_ref.is_some());
    let mat_ref = mat_ref.unwrap();
    assert_eq!(mat_ref.type_name, "Checkerboard");
    assert!(mat_ref.uniforms.contains_key("color"));
}

#[test]
fn test_fabric_kind_mapping() {
    assert_eq!(FabricKind::from_type_name("Color"), FabricKind::Color);
    assert_eq!(FabricKind::from_type_name("Image"), FabricKind::Image);
    assert_eq!(FabricKind::from_type_name("Water"), FabricKind::Water);
    assert_eq!(FabricKind::from_type_name("Checkerboard"), FabricKind::Checkerboard);
    assert_eq!(FabricKind::from_type_name("RimLighting"), FabricKind::RimLighting);
    assert_eq!(FabricKind::from_type_name("PolylineArrow"), FabricKind::PolylineArrow);
    assert_eq!(FabricKind::from_type_name("ElevationContour"), FabricKind::ElevationContour);
    assert_eq!(FabricKind::from_type_name("PolylineGlow"), FabricKind::PolylineGlow);
    assert_eq!(FabricKind::from_type_name("BumpMap"), FabricKind::BumpMap);
    assert_eq!(FabricKind::from_type_name("WaterMask"), FabricKind::WaterMask);
    assert_eq!(FabricKind::from_type_name("UnknownType"), FabricKind::Color);
}

#[test]
fn test_fabric_material_from_domain_color() {
    let system = MaterialSystem::with_builtin_materials();
    let m = system.from_type("Color", std::collections::BTreeMap::new()).unwrap();

    let fm = cesium_bevy_render::fabric_material::fabric_material_from_domain(
        &m,
        Handle::<Image>::default(),
    );

    assert_eq!(fm.params.kind, FabricKind::Color as u32);
}

#[test]
fn test_fabric_material_from_domain_water() {
    use std::collections::BTreeMap;

    let system = MaterialSystem::with_builtin_materials();
    let mut overrides = BTreeMap::new();
    overrides.insert("animationSpeed".to_string(), UniformValue::Float(0.5));
    let m = system.from_type("Water", overrides).unwrap();

    let fm = cesium_bevy_render::fabric_material::fabric_material_from_domain(
        &m,
        Handle::<Image>::default(),
    );

    assert_eq!(fm.params.kind, FabricKind::Water as u32);
    assert!((fm.params.extra_c.w - 0.5).abs() < 1e-6);
}

#[test]
fn test_fabric_material_alpha_mode() {
    let system = MaterialSystem::with_builtin_materials();

    let grid = system.from_type("Grid", std::collections::BTreeMap::new()).unwrap();
    let fm_grid = cesium_bevy_render::fabric_material::fabric_material_from_domain(
        &grid,
        Handle::<Image>::default(),
    );
    assert!(matches!(fm_grid.alpha_mode(), AlphaMode::Blend));
}
