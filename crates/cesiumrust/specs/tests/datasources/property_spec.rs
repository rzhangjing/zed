//! DataSources/PropertySpec、ConstantPropertySpec、SampledPropertySpec、
//! CompositePropertySpec、CallbackPropertySpec、ReferencePropertySpec
//! → Rust 集成测试

use cesium_datasource::property::{Color, Property};
use cesium_datasource::property_system::{
    ConstantProperty, DynProperty, SampledProperty, CallbackProperty, CompositeProperty,
    PackableType, PropertyValue, ReferenceFrame,
    ConstantPositionProperty,
    ReferenceProperty, MapPropertyResolver,
    ExtrapolationType, InterpolationAlgorithmKind,
};
use cesium_time::JulianDate;
use glam::DVec3;
use std::sync::Arc;

// === 简单 Property<T> 枚举（遗留） ===

#[test]
fn test_property_constant_get_value() {
    let prop: Property<f64> = Property::Constant(42.0);
    assert!(prop.is_constant());
    assert!(prop.is_defined());
    assert_eq!(*prop.get_value(0.0).unwrap(), 42.0);
    assert_eq!(*prop.get_value(999.0).unwrap(), 42.0);
}

#[test]
fn test_property_sampled_nearest() {
    let prop: Property<f64> = Property::Sampled(vec![
        (0.0, 10.0),
        (10.0, 20.0),
        (20.0, 30.0),
    ]);
    assert!(!prop.is_constant());
    assert!(prop.is_defined());
    assert_eq!(*prop.get_value(0.0).unwrap(), 10.0);
    assert_eq!(*prop.get_value(9.0).unwrap(), 20.0);
    assert_eq!(*prop.get_value(20.0).unwrap(), 30.0);
}

#[test]
fn test_property_undefined() {
    let prop: Property<f64> = Property::Undefined;
    assert!(!prop.is_defined());
    assert!(!prop.is_constant());
    assert!(prop.get_value(0.0).is_none());
}

#[test]
fn test_property_color_constant() {
    let prop: Property<Color> = Property::Constant(Color::RED);
    let c = prop.get_value(0.0).unwrap();
    assert!((c.red - 1.0).abs() < 1e-10);
    assert!((c.green - 0.0).abs() < 1e-10);
}

#[test]
fn test_property_vec_sampled() {
    let prop: Property<[f64; 3]> = Property::Sampled(vec![
        (0.0, [1.0, 2.0, 3.0]),
        (100.0, [4.0, 5.0, 6.0]),
    ]);
    let val = prop.get_value(0.0).unwrap();
    assert_eq!(*val, [1.0, 2.0, 3.0]);
    let val2 = prop.get_value(99.0).unwrap();
    assert_eq!(*val2, [4.0, 5.0, 6.0]);
}

// === ConstantProperty（property_system） ===

#[test]
fn test_constant_property_number() {
    let prop = ConstantProperty::new(PropertyValue::Number(42.0));
    let time = JulianDate::from_unix_seconds(0.0);
    assert!(prop.is_constant());
    assert_eq!(prop.get_value(&time), PropertyValue::Number(42.0));
}

#[test]
fn test_constant_property_set_value() {
    let mut prop = ConstantProperty::new(PropertyValue::Number(1.0));
    prop.set_value(PropertyValue::Number(99.0));
    let time = JulianDate::from_unix_seconds(0.0);
    assert_eq!(prop.get_value(&time), PropertyValue::Number(99.0));
}

#[test]
fn test_constant_property_equals() {
    let a = ConstantProperty::new(PropertyValue::Number(5.0));
    let b = ConstantProperty::new(PropertyValue::Number(5.0));
    let c = ConstantProperty::new(PropertyValue::Number(10.0));
    assert!(a.equals(&b));
    assert!(!a.equals(&c));
}

#[test]
fn test_constant_property_cartesian3() {
    let prop = ConstantProperty::new(PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0)));
    let time = JulianDate::from_unix_seconds(0.0);
    assert_eq!(
        prop.get_value(&time),
        PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0))
    );
}

#[test]
fn test_constant_property_color() {
    let prop = ConstantProperty::new(PropertyValue::Color([1.0, 0.0, 0.0, 1.0]));
    let time = JulianDate::from_unix_seconds(0.0);
    assert_eq!(
        prop.get_value(&time),
        PropertyValue::Color([1.0, 0.0, 0.0, 1.0])
    );
}

// === SampledProperty ===

#[test]
fn test_sampled_property_new() {
    let prop = SampledProperty::new(PackableType::Number);
    assert_eq!(prop.property_type(), PackableType::Number);
    assert_eq!(prop.sample_count(), 0);
    assert_eq!(prop.interpolation_degree(), 1);
}

#[test]
fn test_sampled_property_add_sample() {
    let mut prop = SampledProperty::new(PackableType::Number);
    let t0 = JulianDate::from_unix_seconds(0.0);
    let t1 = JulianDate::from_unix_seconds(10.0);
    prop.add_sample(t0, &PropertyValue::Number(100.0), &[]);
    prop.add_sample(t1, &PropertyValue::Number(200.0), &[]);
    assert_eq!(prop.sample_count(), 2);
}

#[test]
fn test_sampled_property_get_value_exact() {
    let mut prop = SampledProperty::new(PackableType::Number);
    let t0 = JulianDate::from_unix_seconds(0.0);
    let t1 = JulianDate::from_unix_seconds(10.0);
    prop.add_sample(t0, &PropertyValue::Number(100.0), &[]);
    prop.add_sample(t1, &PropertyValue::Number(200.0), &[]);

    let val = prop.get_value(&t0);
    assert_eq!(val, PropertyValue::Number(100.0));
}

#[test]
fn test_sampled_property_interpolation() {
    let mut prop = SampledProperty::new(PackableType::Number);
    let t0 = JulianDate::from_unix_seconds(0.0);
    let t1 = JulianDate::from_unix_seconds(10.0);
    prop.add_sample(t0, &PropertyValue::Number(0.0), &[]);
    prop.add_sample(t1, &PropertyValue::Number(100.0), &[]);

    // 精确采样时间应返回精确值
    let val0 = prop.get_value(&t0);
    assert_eq!(val0, PropertyValue::Number(0.0));
    let val1 = prop.get_value(&t1);
    assert_eq!(val1, PropertyValue::Number(100.0));
}

#[test]
fn test_sampled_property_cartesian3() {
    let mut prop = SampledProperty::new(PackableType::Cartesian3);
    let t0 = JulianDate::from_unix_seconds(0.0);
    let t1 = JulianDate::from_unix_seconds(10.0);
    prop.add_sample(t0, &PropertyValue::Cartesian3(DVec3::new(0.0, 0.0, 0.0)), &[]);
    prop.add_sample(t1, &PropertyValue::Cartesian3(DVec3::new(10.0, 20.0, 30.0)), &[]);

    // 精确采样时间应返回精确值
    let val0 = prop.get_value(&t0);
    assert_eq!(val0, PropertyValue::Cartesian3(DVec3::new(0.0, 0.0, 0.0)));
    let val1 = prop.get_value(&t1);
    assert_eq!(val1, PropertyValue::Cartesian3(DVec3::new(10.0, 20.0, 30.0)));
}

#[test]
fn test_sampled_property_set_interpolation_options() {
    let mut prop = SampledProperty::new(PackableType::Number);
    prop.set_interpolation_options(
        Some(InterpolationAlgorithmKind::Lagrange),
        Some(3),
    );
    assert_eq!(prop.interpolation_algorithm(), InterpolationAlgorithmKind::Lagrange);
    assert_eq!(prop.interpolation_degree(), 3);
}

#[test]
fn test_sampled_property_extrapolation() {
    let mut prop = SampledProperty::new(PackableType::Number);
    prop.set_forward_extrapolation_type(ExtrapolationType::Hold);
    prop.set_backward_extrapolation_type(ExtrapolationType::Hold);

    let t0 = JulianDate::from_unix_seconds(10.0);
    let t1 = JulianDate::from_unix_seconds(20.0);
    prop.add_sample(t0, &PropertyValue::Number(100.0), &[]);
    prop.add_sample(t1, &PropertyValue::Number(200.0), &[]);

    // 首个样本之前：应保持首个值
    let before = JulianDate::from_unix_seconds(0.0);
    let val = prop.get_value(&before);
    assert_eq!(val, PropertyValue::Number(100.0));

    // 末个样本之后：应保持末个值
    let after = JulianDate::from_unix_seconds(30.0);
    let val = prop.get_value(&after);
    assert_eq!(val, PropertyValue::Number(200.0));
}

#[test]
fn test_sampled_property_is_not_constant() {
    let mut prop = SampledProperty::new(PackableType::Number);
    let t0 = JulianDate::from_unix_seconds(0.0);
    prop.add_sample(t0, &PropertyValue::Number(1.0), &[]);
    assert!(!prop.is_constant());
}

// === CallbackProperty ===

#[test]
fn test_callback_property() {
    let prop = CallbackProperty::new(
        |_time: &JulianDate| PropertyValue::Number(42.0),
        true,
    );
    let time = JulianDate::from_unix_seconds(0.0);
    assert!(prop.is_constant());
    assert_eq!(prop.get_value(&time), PropertyValue::Number(42.0));
}

#[test]
fn test_callback_property_not_constant() {
    let prop = CallbackProperty::new(
        |time: &JulianDate| PropertyValue::Number(time.total_days()),
        false,
    );
    assert!(!prop.is_constant());
    let time = JulianDate::from_unix_seconds(86400.0); // 1 天
    let val = prop.get_value(&time);
    if let PropertyValue::Number(v) = val {
        // 应约为 1 天（unix 纪元为 JD 2440587.5）
        assert!(v > 2440587.0 && v < 2440589.0);
    } else {
        panic!("Expected Number");
    }
}

// === CompositeProperty ===

#[test]
fn test_composite_property_empty() {
    let prop = CompositeProperty::new();
    let time = JulianDate::from_unix_seconds(0.0);
    assert!(prop.is_constant());
    assert_eq!(prop.get_value(&time), PropertyValue::Undefined);
}

// === ConstantPositionProperty ===

#[test]
fn test_constant_position_property() {
    let pos = ConstantPositionProperty::new(DVec3::new(1.0, 2.0, 3.0));
    let time = JulianDate::from_unix_seconds(0.0);
    assert!(pos.is_constant());
    let val = pos.get_value(&time);
    assert_eq!(val, PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0)));
}

#[test]
fn test_constant_position_property_reference_frame() {
    let pos = ConstantPositionProperty::with_reference_frame(
        DVec3::new(1.0, 0.0, 0.0),
        ReferenceFrame::Fixed,
    );
    assert_eq!(pos.reference_frame(), Some(ReferenceFrame::Fixed));
}

#[test]
fn test_constant_position_property_undefined() {
    let pos = ConstantPositionProperty::undefined();
    let time = JulianDate::from_unix_seconds(0.0);
    assert_eq!(pos.get_value(&time), PropertyValue::Undefined);
}

#[test]
fn test_constant_position_property_set_value() {
    let mut pos = ConstantPositionProperty::undefined();
    pos.set_value(Some(DVec3::new(5.0, 6.0, 7.0)), None);
    assert_eq!(pos.value(), Some(DVec3::new(5.0, 6.0, 7.0)));
}

#[test]
fn test_constant_position_property_equals() {
    let a = ConstantPositionProperty::new(DVec3::new(1.0, 2.0, 3.0));
    let b = ConstantPositionProperty::new(DVec3::new(1.0, 2.0, 3.0));
    let c = ConstantPositionProperty::new(DVec3::new(4.0, 5.0, 6.0));
    assert!(a.equals(&b));
    assert!(!a.equals(&c));
}

// === ReferenceProperty ===

#[test]
fn test_reference_property_from_string() {
    let resolver = Arc::new(MapPropertyResolver::new());
    let prop = ReferenceProperty::from_string(resolver, "Satellite/A#position");
    assert_eq!(prop.target_id(), "Satellite/A");
    assert_eq!(prop.target_property_names(), &["position".to_string()]);
}

#[test]
fn test_reference_property_nested_path() {
    let resolver = Arc::new(MapPropertyResolver::new());
    let prop = ReferenceProperty::from_string(resolver, "obj#graphics.color");
    assert_eq!(prop.target_id(), "obj");
    assert_eq!(
        prop.target_property_names(),
        &["graphics".to_string(), "color".to_string()]
    );
}

// === PackableType ===

#[test]
fn test_packable_type_packed_length() {
    assert_eq!(PackableType::Number.packed_length(), 1);
    assert_eq!(PackableType::Cartesian2.packed_length(), 2);
    assert_eq!(PackableType::Cartesian3.packed_length(), 3);
    assert_eq!(PackableType::Quaternion.packed_length(), 4);
    assert_eq!(PackableType::Color.packed_length(), 4);
}

// === PropertyValue ===

#[test]
fn test_property_value_is_undefined() {
    assert!(PropertyValue::Undefined.is_undefined());
    assert!(!PropertyValue::Number(1.0).is_undefined());
}

#[test]
fn test_property_value_variants() {
    let n = PropertyValue::Number(3.14);
    assert_eq!(n, PropertyValue::Number(3.14));

    let b = PropertyValue::Boolean(true);
    assert_eq!(b, PropertyValue::Boolean(true));

    let t = PropertyValue::Text("hello".to_string());
    assert_eq!(t, PropertyValue::Text("hello".to_string()));
}
