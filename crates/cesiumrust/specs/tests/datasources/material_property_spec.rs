//! DataSources/ColorMaterialPropertySpec、GridMaterialPropertySpec、
//! StripeMaterialPropertySpec、CheckerboardMaterialPropertySpec、
//! ImageMaterialPropertySpec、Polyline*MaterialPropertySpec
//! → Rust 集成测试

use cesium_datasource::property_system::{
    ColorMaterialProperty, GridMaterialProperty, StripeMaterialProperty,
    CheckerboardMaterialProperty, ImageMaterialProperty,
    PolylineArrowMaterialProperty, PolylineDashMaterialProperty,
    PolylineGlowMaterialProperty, PolylineOutlineMaterialProperty,
    CompositeMaterialProperty, MaterialProperty, StripeOrientation,
    COLOR_WHITE, COLOR_BLACK,
};
use cesium_datasource::property_system::PropertyValue;
use cesium_time::JulianDate;

fn epoch() -> JulianDate {
    JulianDate::from_unix_seconds(0.0)
}

// === ColorMaterialProperty ===

#[test]
fn test_color_material_property_new() {
    let prop = ColorMaterialProperty::new(Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])));
    assert!(prop.is_constant());
    assert_eq!(prop.get_type(&epoch()), Some("Color".to_string()));
}

#[test]
fn test_color_material_property_from_color() {
    let prop = ColorMaterialProperty::from_color([0.0, 1.0, 0.0, 1.0]);
    let uniforms = prop.get_value(&epoch());
    assert_eq!(
        uniforms.get("color").unwrap(),
        &PropertyValue::Color([0.0, 1.0, 0.0, 1.0])
    );
}

#[test]
fn test_color_material_property_default_white() {
    let prop = ColorMaterialProperty::new(None);
    let uniforms = prop.get_value(&epoch());
    assert_eq!(
        uniforms.get("color").unwrap(),
        &PropertyValue::Color(COLOR_WHITE)
    );
}

#[test]
fn test_color_material_property_set_color() {
    let mut prop = ColorMaterialProperty::new(None);
    prop.set_color(Some(PropertyValue::Color([0.5, 0.5, 0.5, 1.0])));
    let uniforms = prop.get_value(&epoch());
    assert_eq!(
        uniforms.get("color").unwrap(),
        &PropertyValue::Color([0.5, 0.5, 0.5, 1.0])
    );
}

#[test]
fn test_color_material_property_equals() {
    let a = ColorMaterialProperty::from_color([1.0, 0.0, 0.0, 1.0]);
    let b = ColorMaterialProperty::from_color([1.0, 0.0, 0.0, 1.0]);
    let c = ColorMaterialProperty::from_color([0.0, 1.0, 0.0, 1.0]);
    assert!(a.equals(&b));
    assert!(!a.equals(&c));
}

// === GridMaterialProperty ===

#[test]
fn test_grid_material_property_new() {
    let prop = GridMaterialProperty::new();
    assert!(prop.is_constant());
    assert_eq!(prop.get_type(&epoch()), Some("Grid".to_string()));
}

#[test]
fn test_grid_material_property_defaults() {
    let prop = GridMaterialProperty::new();
    let uniforms = prop.get_value(&epoch());
    assert_eq!(
        uniforms.get("color").unwrap(),
        &PropertyValue::Color(COLOR_WHITE)
    );
    assert_eq!(
        uniforms.get("cellAlpha").unwrap(),
        &PropertyValue::Number(0.1)
    );
}

#[test]
fn test_grid_material_property_set_color() {
    let mut prop = GridMaterialProperty::new();
    prop.set_color(Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])));
    let uniforms = prop.get_value(&epoch());
    assert_eq!(
        uniforms.get("color").unwrap(),
        &PropertyValue::Color([1.0, 0.0, 0.0, 1.0])
    );
}

#[test]
fn test_grid_material_property_set_cell_alpha() {
    let mut prop = GridMaterialProperty::new();
    prop.set_cell_alpha(Some(PropertyValue::Number(0.5)));
    let uniforms = prop.get_value(&epoch());
    assert_eq!(
        uniforms.get("cellAlpha").unwrap(),
        &PropertyValue::Number(0.5)
    );
}

#[test]
fn test_grid_material_property_equals() {
    let a = GridMaterialProperty::new();
    let b = GridMaterialProperty::new();
    assert!(a.equals(&b));
}

// === StripeMaterialProperty ===

#[test]
fn test_stripe_material_property_new() {
    let prop = StripeMaterialProperty::new();
    assert!(prop.is_constant());
    assert_eq!(prop.get_type(&epoch()), Some("Stripe".to_string()));
}

#[test]
fn test_stripe_material_property_set_orientation() {
    let mut prop = StripeMaterialProperty::new();
    prop.set_orientation(StripeOrientation::Vertical);
    let uniforms = prop.get_value(&epoch());
    // 垂直方向意味着 horizontal=false
    assert_eq!(
        uniforms.get("horizontal").unwrap(),
        &PropertyValue::Boolean(false)
    );
}

#[test]
fn test_stripe_material_property_defaults() {
    let prop = StripeMaterialProperty::new();
    let uniforms = prop.get_value(&epoch());
    // 默认方向为水平（horizontal=true）
    assert_eq!(
        uniforms.get("horizontal").unwrap(),
        &PropertyValue::Boolean(true)
    );
    // 默认偶数颜色为白色
    assert_eq!(
        uniforms.get("evenColor").unwrap(),
        &PropertyValue::Color(COLOR_WHITE)
    );
    // 默认奇数颜色为黑色
    assert_eq!(
        uniforms.get("oddColor").unwrap(),
        &PropertyValue::Color(COLOR_BLACK)
    );
}

#[test]
fn test_stripe_orientation_to_number() {
    assert_eq!(StripeOrientation::Horizontal.to_number(), 0.0);
    assert_eq!(StripeOrientation::Vertical.to_number(), 1.0);
}

#[test]
fn test_stripe_orientation_from_value() {
    assert_eq!(
        StripeOrientation::from_value(&PropertyValue::Number(0.0)),
        StripeOrientation::Horizontal
    );
    assert_eq!(
        StripeOrientation::from_value(&PropertyValue::Number(1.0)),
        StripeOrientation::Vertical
    );
}

// === CheckerboardMaterialProperty ===

#[test]
fn test_checkerboard_material_property_new() {
    let prop = CheckerboardMaterialProperty::new();
    assert!(prop.is_constant());
    assert_eq!(prop.get_type(&epoch()), Some("Checkerboard".to_string()));
}

#[test]
fn test_checkerboard_material_property_defaults() {
    let prop = CheckerboardMaterialProperty::new();
    let uniforms = prop.get_value(&epoch());
    assert_eq!(
        uniforms.get("lightColor").unwrap(),
        &PropertyValue::Color(COLOR_WHITE)
    );
    assert_eq!(
        uniforms.get("darkColor").unwrap(),
        &PropertyValue::Color(COLOR_BLACK)
    );
}

#[test]
fn test_checkerboard_material_property_set_colors() {
    let mut prop = CheckerboardMaterialProperty::new();
    prop.set_even_color(Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])));
    prop.set_odd_color(Some(PropertyValue::Color([0.0, 0.0, 1.0, 1.0])));
    let uniforms = prop.get_value(&epoch());
    assert_eq!(
        uniforms.get("lightColor").unwrap(),
        &PropertyValue::Color([1.0, 0.0, 0.0, 1.0])
    );
    assert_eq!(
        uniforms.get("darkColor").unwrap(),
        &PropertyValue::Color([0.0, 0.0, 1.0, 1.0])
    );
}

#[test]
fn test_checkerboard_material_property_equals() {
    let a = CheckerboardMaterialProperty::new();
    let b = CheckerboardMaterialProperty::new();
    assert!(a.equals(&b));
}

// === ImageMaterialProperty ===

#[test]
fn test_image_material_property_new() {
    let prop = ImageMaterialProperty::new();
    assert!(prop.is_constant());
    assert_eq!(prop.get_type(&epoch()), Some("Image".to_string()));
}

#[test]
fn test_image_material_property_set_image() {
    let mut prop = ImageMaterialProperty::new();
    prop.set_image(Some(PropertyValue::Text("test.png".to_string())));
    let uniforms = prop.get_value(&epoch());
    assert_eq!(
        uniforms.get("image").unwrap(),
        &PropertyValue::Text("test.png".to_string())
    );
}

#[test]
fn test_image_material_property_default_repeat() {
    let prop = ImageMaterialProperty::new();
    let uniforms = prop.get_value(&epoch());
    // 默认 repeat 为 (1, 1)
    if let PropertyValue::Cartesian2(v) = uniforms.get("repeat").unwrap() {
        assert!((v.x - 1.0).abs() < 1e-10);
        assert!((v.y - 1.0).abs() < 1e-10);
    } else {
        panic!("Expected Cartesian2 for repeat");
    }
}

// === PolylineArrowMaterialProperty ===

#[test]
fn test_polyline_arrow_material_property() {
    let prop = PolylineArrowMaterialProperty::new(None);
    assert!(prop.is_constant());
    assert_eq!(prop.get_type(&epoch()), Some("PolylineArrow".to_string()));
}

#[test]
fn test_polyline_arrow_material_property_default_color() {
    let prop = PolylineArrowMaterialProperty::new(None);
    let uniforms = prop.get_value(&epoch());
    assert_eq!(
        uniforms.get("color").unwrap(),
        &PropertyValue::Color(COLOR_WHITE)
    );
}

// === PolylineDashMaterialProperty ===

#[test]
fn test_polyline_dash_material_property() {
    let prop = PolylineDashMaterialProperty::new();
    assert!(prop.is_constant());
    assert_eq!(prop.get_type(&epoch()), Some("PolylineDash".to_string()));
}

// === PolylineGlowMaterialProperty ===

#[test]
fn test_polyline_glow_material_property() {
    let prop = PolylineGlowMaterialProperty::new();
    assert!(prop.is_constant());
    assert_eq!(prop.get_type(&epoch()), Some("PolylineGlow".to_string()));
}

#[test]
fn test_polyline_glow_material_property_default() {
    let prop = PolylineGlowMaterialProperty::new();
    let uniforms = prop.get_value(&epoch());
    assert_eq!(
        uniforms.get("color").unwrap(),
        &PropertyValue::Color(COLOR_WHITE)
    );
}

// === PolylineOutlineMaterialProperty ===

#[test]
fn test_polyline_outline_material_property() {
    let prop = PolylineOutlineMaterialProperty::new();
    assert!(prop.is_constant());
    assert_eq!(prop.get_type(&epoch()), Some("PolylineOutline".to_string()));
}

// === CompositeMaterialProperty ===

#[test]
fn test_composite_material_property_empty() {
    let prop = CompositeMaterialProperty::new();
    assert!(prop.is_constant());
    assert_eq!(prop.get_type(&epoch()), None);
}
