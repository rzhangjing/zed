//! 材质属性扩展规范 - 动态值、isConstant、详细 equals
//!
//! 参考自：GridMaterialPropertySpec、CheckerboardMaterialPropertySpec、
//! StripeMaterialPropertySpec、ImageMaterialPropertySpec、
//! PolylineDashMaterialPropertySpec、PolylineGlowMaterialPropertySpec、
//! PolylineOutlineMaterialPropertySpec、CompositeMaterialPropertySpec
//!
//! A 类测试：dynamic values(7) + isConstant(7) + equals_detailed(3) +
//!                composite(3) + constructor_options(2) = 22

use cesium_datasource::property_system::{
    CheckerboardMaterialProperty, ColorMaterialProperty, CompositeMaterialProperty,
    GridMaterialProperty, ImageMaterialProperty, MaterialProperty, PolylineDashMaterialProperty,
    PolylineGlowMaterialProperty, PolylineOutlineMaterialProperty, StripeMaterialProperty,
    StripeOrientation, COLOR_BLACK, COLOR_WHITE,
};
use cesium_datasource::property_system::property::TimeIntervalCollectionProperty;
use cesium_datasource::property_system::PropertyValue;
use cesium_time::{JulianDate, TimeInterval};
use glam::DVec2;
use std::sync::Arc;

fn t(seconds: f64) -> JulianDate {
    JulianDate::from_unix_seconds(seconds)
}

fn uniform<'a>(uniforms: &'a std::collections::BTreeMap<String, PropertyValue>, key: &str) -> &'a PropertyValue {
    uniforms.get(key).unwrap_or_else(|| panic!("missing uniform '{key}'"))
}

// ─── Grid：动态值 ──────────────────────────────────────────────────

#[test]
fn test_grid_dynamic_values() {
    let mut prop = GridMaterialProperty::new();

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(1.0), t(2.0), true, true),
        Some(PropertyValue::Color([0.0, 0.0, 1.0, 1.0])),
    );
    prop.set_color_property(Some(Arc::new(tic)));

    let mut tic2 = TimeIntervalCollectionProperty::new();
    tic2.add_interval(
        TimeInterval::new(t(1.0), t(2.0), true, true),
        Some(PropertyValue::Number(1.0)),
    );
    prop.set_cell_alpha_property(Some(Arc::new(tic2)));

    // 位于区间内的时间
    let uniforms = prop.get_value(&t(1.5));
    assert_eq!(
        uniform(&uniforms, "color"),
        &PropertyValue::Color([0.0, 0.0, 1.0, 1.0])
    );
    assert_eq!(uniform(&uniforms, "cellAlpha"), &PropertyValue::Number(1.0));

    // 区间之外 → 默认值
    let uniforms2 = prop.get_value(&t(5.0));
    assert_eq!(uniform(&uniforms2, "color"), &PropertyValue::Color(COLOR_WHITE));
    assert_eq!(uniform(&uniforms2, "cellAlpha"), &PropertyValue::Number(0.1));
}

#[test]
fn test_grid_is_constant_with_dynamic() {
    let mut prop = GridMaterialProperty::new();
    assert!(prop.is_constant());

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(0.0), t(10.0), true, true),
        Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])),
    );
    prop.set_color_property(Some(Arc::new(tic)));
    assert!(!prop.is_constant());
}

// ─── Checkerboard：动态值 ──────────────────────────────────────────

#[test]
fn test_checkerboard_dynamic_values() {
    let mut prop = CheckerboardMaterialProperty::new();

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(1.0), t(2.0), true, true),
        Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])),
    );
    prop.set_even_color_property(Some(Arc::new(tic)));

    let uniforms = prop.get_value(&t(1.5));
    assert_eq!(
        uniform(&uniforms, "lightColor"),
        &PropertyValue::Color([1.0, 0.0, 0.0, 1.0])
    );
    // oddColor 未设置 → 默认黑色
    assert_eq!(uniform(&uniforms, "darkColor"), &PropertyValue::Color(COLOR_BLACK));
}

#[test]
fn test_checkerboard_is_constant_with_dynamic() {
    let mut prop = CheckerboardMaterialProperty::new();
    assert!(prop.is_constant());

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(0.0), t(10.0), true, true),
        Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])),
    );
    prop.set_even_color_property(Some(Arc::new(tic)));
    assert!(!prop.is_constant());
}

// ─── Stripe：动态值 ────────────────────────────────────────────────

#[test]
fn test_stripe_dynamic_values() {
    let mut prop = StripeMaterialProperty::new();

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(1.0), t(2.0), true, true),
        Some(PropertyValue::Color([0.0, 1.0, 0.0, 1.0])),
    );
    prop.set_even_color_property(Some(Arc::new(tic)));

    let uniforms = prop.get_value(&t(1.5));
    assert_eq!(
        uniform(&uniforms, "evenColor"),
        &PropertyValue::Color([0.0, 1.0, 0.0, 1.0])
    );
}

#[test]
fn test_stripe_is_constant_with_dynamic() {
    let mut prop = StripeMaterialProperty::new();
    assert!(prop.is_constant());

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(0.0), t(10.0), true, true),
        Some(PropertyValue::Number(0.5)),
    );
    prop.set_offset_property(Some(Arc::new(tic)));
    assert!(!prop.is_constant());
}

// ─── Image：动态值 ─────────────────────────────────────────────────

#[test]
fn test_image_dynamic_values() {
    let mut prop = ImageMaterialProperty::new();

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(1.0), t(2.0), true, true),
        Some(PropertyValue::Text("dynamic.png".to_string())),
    );
    prop.set_image_property(Some(Arc::new(tic)));

    let uniforms = prop.get_value(&t(1.5));
    assert_eq!(
        uniform(&uniforms, "image"),
        &PropertyValue::Text("dynamic.png".to_string())
    );
}

#[test]
fn test_image_is_constant_with_dynamic() {
    let mut prop = ImageMaterialProperty::new();
    assert!(prop.is_constant());

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(0.0), t(10.0), true, true),
        Some(PropertyValue::Text("test.png".to_string())),
    );
    prop.set_image_property(Some(Arc::new(tic)));
    assert!(!prop.is_constant());
}

// ─── PolylineDash：动态值 ──────────────────────────────────────────

#[test]
fn test_polyline_dash_dynamic_values() {
    let mut prop = PolylineDashMaterialProperty::new();

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(1.0), t(2.0), true, true),
        Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])),
    );
    prop.set_color_property(Some(Arc::new(tic)));

    let uniforms = prop.get_value(&t(1.5));
    assert_eq!(
        uniform(&uniforms, "color"),
        &PropertyValue::Color([1.0, 0.0, 0.0, 1.0])
    );
}

#[test]
fn test_polyline_dash_is_constant_with_dynamic() {
    let mut prop = PolylineDashMaterialProperty::new();
    assert!(prop.is_constant());

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(0.0), t(10.0), true, true),
        Some(PropertyValue::Number(2.0)),
    );
    prop.set_dash_length_property(Some(Arc::new(tic)));
    assert!(!prop.is_constant());
}

// ─── PolylineGlow：动态值 ──────────────────────────────────────────

#[test]
fn test_polyline_glow_dynamic_values() {
    let mut prop = PolylineGlowMaterialProperty::new();

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(1.0), t(2.0), true, true),
        Some(PropertyValue::Number(0.5)),
    );
    prop.set_glow_power_property(Some(Arc::new(tic)));

    let uniforms = prop.get_value(&t(1.5));
    assert_eq!(uniform(&uniforms, "glowPower"), &PropertyValue::Number(0.5));
}

#[test]
fn test_polyline_glow_is_constant_with_dynamic() {
    let mut prop = PolylineGlowMaterialProperty::new();
    assert!(prop.is_constant());

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(0.0), t(10.0), true, true),
        Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])),
    );
    prop.set_color_property(Some(Arc::new(tic)));
    assert!(!prop.is_constant());
}

// ─── PolylineOutline：动态值 ───────────────────────────────────────

#[test]
fn test_polyline_outline_dynamic_values() {
    let mut prop = PolylineOutlineMaterialProperty::new();

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(1.0), t(2.0), true, true),
        Some(PropertyValue::Number(5.0)),
    );
    prop.set_outline_width_property(Some(Arc::new(tic)));

    let uniforms = prop.get_value(&t(1.5));
    assert_eq!(uniform(&uniforms, "outlineWidth"), &PropertyValue::Number(5.0));
}

#[test]
fn test_polyline_outline_is_constant_with_dynamic() {
    let mut prop = PolylineOutlineMaterialProperty::new();
    assert!(prop.is_constant());

    let mut tic = TimeIntervalCollectionProperty::new();
    tic.add_interval(
        TimeInterval::new(t(0.0), t(10.0), true, true),
        Some(PropertyValue::Color([0.0, 0.0, 1.0, 1.0])),
    );
    prop.set_outline_color_property(Some(Arc::new(tic)));
    assert!(!prop.is_constant());
}

// ─── 详细 equals ───────────────────────────────────────────────────────

#[test]
fn test_grid_equals_detailed() {
    let mut a = GridMaterialProperty::new();
    a.set_color(Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])));
    a.set_cell_alpha(Some(PropertyValue::Number(0.5)));
    a.set_line_count(Some(PropertyValue::Cartesian2(DVec2::new(4.0, 4.0))));

    let mut b = GridMaterialProperty::new();
    b.set_color(Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])));
    b.set_cell_alpha(Some(PropertyValue::Number(0.5)));
    b.set_line_count(Some(PropertyValue::Cartesian2(DVec2::new(4.0, 4.0))));

    assert!(a.equals(&b));

    // 改变 color → 不相等
    b.set_color(Some(PropertyValue::Color([0.0, 1.0, 0.0, 1.0])));
    assert!(!a.equals(&b));

    // 恢复 color，改变 cellAlpha → 不相等
    b.set_color(Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])));
    b.set_cell_alpha(Some(PropertyValue::Number(0.9)));
    assert!(!a.equals(&b));
}

#[test]
fn test_checkerboard_equals_detailed() {
    let mut a = CheckerboardMaterialProperty::new();
    a.set_even_color(Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])));
    a.set_odd_color(Some(PropertyValue::Color([0.0, 0.0, 1.0, 1.0])));
    a.set_repeat(Some(PropertyValue::Cartesian2(DVec2::new(4.0, 4.0))));

    let mut b = CheckerboardMaterialProperty::new();
    b.set_even_color(Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])));
    b.set_odd_color(Some(PropertyValue::Color([0.0, 0.0, 1.0, 1.0])));
    b.set_repeat(Some(PropertyValue::Cartesian2(DVec2::new(4.0, 4.0))));

    assert!(a.equals(&b));

    b.set_repeat(Some(PropertyValue::Cartesian2(DVec2::new(8.0, 8.0))));
    assert!(!a.equals(&b));
}

#[test]
fn test_stripe_equals_detailed() {
    let mut a = StripeMaterialProperty::new();
    a.set_orientation(StripeOrientation::Vertical);
    a.set_even_color(Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])));

    let mut b = StripeMaterialProperty::new();
    b.set_orientation(StripeOrientation::Vertical);
    b.set_even_color(Some(PropertyValue::Color([1.0, 0.0, 0.0, 1.0])));

    assert!(a.equals(&b));

    b.set_orientation(StripeOrientation::Horizontal);
    assert!(!a.equals(&b));
}

// ─── CompositeMaterialProperty ─────────────────────────────────────────────

#[test]
fn test_composite_material_get_value() {
    let mut prop = CompositeMaterialProperty::new();
    assert!(prop.is_constant());

    let color_mat = Arc::new(ColorMaterialProperty::from_color([1.0, 0.0, 0.0, 1.0]))
        as Arc<dyn MaterialProperty>;
    let grid_mat = Arc::new(GridMaterialProperty::new()) as Arc<dyn MaterialProperty>;

    prop.add_interval(TimeInterval::new(t(0.0), t(10.0), true, false), Some(color_mat));
    prop.add_interval(TimeInterval::new(t(10.0), t(20.0), true, true), Some(grid_mat));
    assert!(!prop.is_constant());

    // 第一个区间 → Color 材质
    assert_eq!(prop.get_type(&t(5.0)), Some("Color".to_string()));
    let uniforms = prop.get_value(&t(5.0));
    assert_eq!(
        uniform(&uniforms, "color"),
        &PropertyValue::Color([1.0, 0.0, 0.0, 1.0])
    );

    // 第二个区间 → Grid 材质
    assert_eq!(prop.get_type(&t(15.0)), Some("Grid".to_string()));
    let uniforms2 = prop.get_value(&t(15.0));
    assert!(uniforms2.contains_key("cellAlpha"));

    // 位于所有区间之外
    assert_eq!(prop.get_type(&t(30.0)), None);
    assert!(prop.get_value(&t(30.0)).is_empty());
}

#[test]
fn test_composite_material_equals() {
    let mut a = CompositeMaterialProperty::new();
    let b = CompositeMaterialProperty::new();
    assert!(a.equals(&b));

    let color_mat = Arc::new(ColorMaterialProperty::from_color([1.0, 0.0, 0.0, 1.0]))
        as Arc<dyn MaterialProperty>;
    a.add_interval(TimeInterval::new(t(0.0), t(10.0), true, true), Some(color_mat));
    assert!(!a.equals(&b));
}

#[test]
fn test_composite_material_is_constant() {
    let mut prop = CompositeMaterialProperty::new();
    assert!(prop.is_constant()); // 空 → 常量

    let grid_mat = Arc::new(GridMaterialProperty::new()) as Arc<dyn MaterialProperty>;
    prop.add_interval(TimeInterval::new(t(0.0), t(10.0), true, true), Some(grid_mat));
    assert!(!prop.is_constant());
}
