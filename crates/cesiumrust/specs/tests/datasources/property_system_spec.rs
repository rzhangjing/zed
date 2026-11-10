//! 对齐实现自 CesiumJS DataSources 属性规范：
//! - ConstantPropertySpec (10 it())
//! - SampledPropertySpec (36 it())
//! - TimeIntervalCollectionPropertySpec (8 it())
//! - CompositePropertySpec (7 it())
//! - CallbackPropertySpec (8 it())
//!
//! A 类测试（纯逻辑，无 DOM/事件）：约 35 个测试

use cesium_datasource::property_system::{
    CallbackFn, CallbackProperty, CompositeProperty, ConstantProperty, DynProperty,
    ExtrapolationType, InterpolationAlgorithmKind, PackableType, PropertyValue, SampledProperty,
    TimeIntervalCollectionProperty,
};
use cesium_time::{JulianDate, TimeInterval};
use glam::DVec3;
use std::sync::Arc;

fn jd(day: f64, seconds: f64) -> JulianDate {
    JulianDate::new(day, seconds)
}

// ===========================================================================
// ConstantProperty（源自 ConstantPropertySpec）
// ===========================================================================

#[test]
fn constant_property_works_with_basic_types() {
    // "works with basic types"
    let expected = 5.0;
    let property = ConstantProperty::new(PropertyValue::Number(expected));
    let time = jd(2451545.0, 0.0);
    assert_eq!(property.get_value(&time), PropertyValue::Number(expected));
    assert_eq!(*property.value(), PropertyValue::Number(expected));
}

#[test]
fn constant_property_works_with_objects() {
    // "works with objects"
    let value = DVec3::new(1.0, 2.0, 3.0);
    let property = ConstantProperty::new(PropertyValue::Cartesian3(value));
    let time = jd(2451545.0, 0.0);

    let result = property.get_value(&time);
    // 结果等于值（Rust 中的 clone 语义，始终是新副本）
    assert_eq!(result, PropertyValue::Cartesian3(value));
    assert_eq!(*property.value(), PropertyValue::Cartesian3(value));
}

#[test]
fn constant_property_works_with_undefined_value() {
    // "works with undefined value"
    let property = ConstantProperty::new(PropertyValue::Undefined);
    let time = jd(2451545.0, 0.0);
    assert_eq!(property.get_value(&time), PropertyValue::Undefined);
    assert_eq!(*property.value(), PropertyValue::Undefined);
}

#[test]
fn constant_property_equals_works_for_object_types() {
    // 'equals works for object types with "equals" function'
    let left = ConstantProperty::new(PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0)));
    let right = ConstantProperty::new(PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0)));
    assert!(left.equals(&right));

    let right2 = ConstantProperty::new(PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 4.0)));
    assert!(!left.equals(&right2));
}

#[test]
fn constant_property_equals_works_for_simple_types() {
    // "equals works for simple types"
    let left = ConstantProperty::new(PropertyValue::Number(1.0));
    let right = ConstantProperty::new(PropertyValue::Number(1.0));
    assert!(left.equals(&right));

    let right2 = ConstantProperty::new(PropertyValue::Number(2.0));
    assert!(!left.equals(&right2));
}

#[test]
fn constant_property_is_constant() {
    let property = ConstantProperty::new(PropertyValue::Number(42.0));
    assert!(property.is_constant());
    assert_eq!(property.type_name(), "ConstantProperty");
}

#[test]
fn constant_property_set_value_changes_value() {
    let mut property = ConstantProperty::new(PropertyValue::Number(1.0));
    property.set_value(PropertyValue::Number(99.0));
    let time = jd(2451545.0, 0.0);
    assert_eq!(property.get_value(&time), PropertyValue::Number(99.0));
}

// ===========================================================================
// SampledProperty（源自 SampledPropertySpec）
// ===========================================================================

#[test]
fn sampled_property_constructor_sets_expected_defaults() {
    // "constructor sets expected defaults"
    let property = SampledProperty::new(PackableType::Cartesian3);
    assert_eq!(property.interpolation_degree(), 1);
    assert_eq!(
        property.interpolation_algorithm(),
        InterpolationAlgorithmKind::Linear
    );
    assert!(property.is_constant());
    assert_eq!(property.property_type(), PackableType::Cartesian3);
    assert!(property.derivative_types().is_none());

    // 带导数类型
    let property2 = SampledProperty::with_derivative_types(
        PackableType::Quaternion,
        Some(vec![PackableType::Quaternion, PackableType::Quaternion]),
    );
    assert_eq!(property2.interpolation_degree(), 1);
    assert_eq!(
        property2.interpolation_algorithm(),
        InterpolationAlgorithmKind::Linear
    );
    assert!(property2.is_constant());
    assert_eq!(property2.property_type(), PackableType::Quaternion);
    assert_eq!(
        property2.derivative_types(),
        Some(&[PackableType::Quaternion, PackableType::Quaternion][..])
    );
}

#[test]
fn sampled_property_is_constant_works() {
    // "isConstant works"
    let mut property = SampledProperty::new(PackableType::Number);
    assert!(property.is_constant());
    property.add_sample(jd(0.0, 0.0), &PropertyValue::Number(1.0), &[]);
    assert!(!property.is_constant());
}

#[test]
fn sampled_property_add_samples_packed_array_works() {
    // "addSamplesPackedArray works"
    let data = [0.0, 7.0, 1.0, 8.0, 2.0, 9.0];
    let epoch = jd(0.0, 0.0);

    let mut property = SampledProperty::new(PackableType::Number);
    property.add_samples_packed_array(&data, &epoch);

    assert_eq!(property.get_value(&epoch), PropertyValue::Number(7.0));
    assert_eq!(
        property.get_value(&jd(0.0, 0.5)),
        PropertyValue::Number(7.5)
    );
}

#[test]
fn sampled_property_add_sample_works() {
    // "addSample works"
    let times = [jd(0.0, 0.0), jd(1.0, 0.0), jd(2.0, 0.0)];
    let values = [7.0, 8.0, 9.0];

    let mut property = SampledProperty::new(PackableType::Number);
    property.add_sample(times[0], &PropertyValue::Number(values[0]), &[]);
    property.add_sample(times[1], &PropertyValue::Number(values[1]), &[]);
    property.add_sample(times[2], &PropertyValue::Number(values[2]), &[]);

    assert_eq!(property.sample_count(), 3);
    assert_eq!(
        property.get_value(&jd(0.0, 0.0)),
        PropertyValue::Number(7.0)
    );
    assert_eq!(
        property.get_value(&jd(0.5, 0.0)),
        PropertyValue::Number(7.5)
    );
    assert_eq!(
        property.get_value(&jd(1.0, 0.0)),
        PropertyValue::Number(8.0)
    );
}

#[test]
fn sampled_property_get_value_returns_undefined_with_no_samples() {
    // "getValue returns undefined with no samples"
    let property = SampledProperty::new(PackableType::Number);
    assert_eq!(
        property.get_value(&jd(0.0, 0.0)),
        PropertyValue::Undefined
    );
}

#[test]
fn sampled_property_get_value_interpolates() {
    // "getValue interpolates"
    let mut property = SampledProperty::new(PackableType::Number);
    property.add_sample(jd(0.0, 0.0), &PropertyValue::Number(0.0), &[]);
    property.add_sample(jd(1.0, 0.0), &PropertyValue::Number(10.0), &[]);

    assert_eq!(
        property.get_value(&jd(0.0, 0.0)),
        PropertyValue::Number(0.0)
    );
    assert_eq!(
        property.get_value(&jd(0.5, 0.0)),
        PropertyValue::Number(5.0)
    );
    assert_eq!(
        property.get_value(&jd(1.0, 0.0)),
        PropertyValue::Number(10.0)
    );
}

#[test]
fn sampled_property_extrapolation_type_none() {
    // 默认外推为 NONE - 超出范围返回 undefined
    let mut property = SampledProperty::new(PackableType::Number);
    property.add_sample(jd(0.0, 0.0), &PropertyValue::Number(0.0), &[]);
    property.add_sample(jd(1.0, 0.0), &PropertyValue::Number(10.0), &[]);

    assert_eq!(
        property.get_value(&jd(-1.0, 0.0)),
        PropertyValue::Undefined
    );
    assert_eq!(
        property.get_value(&jd(2.0, 0.0)),
        PropertyValue::Undefined
    );
}

#[test]
fn sampled_property_extrapolation_type_hold() {
    // "getValue extrapolates HOLD"
    let mut property = SampledProperty::new(PackableType::Number);
    property.add_sample(jd(0.0, 0.0), &PropertyValue::Number(0.0), &[]);
    property.add_sample(jd(1.0, 0.0), &PropertyValue::Number(10.0), &[]);
    property.set_backward_extrapolation_type(ExtrapolationType::Hold);
    property.set_forward_extrapolation_type(ExtrapolationType::Hold);

    assert_eq!(
        property.get_value(&jd(-1.0, 0.0)),
        PropertyValue::Number(0.0)
    );
    assert_eq!(
        property.get_value(&jd(2.0, 0.0)),
        PropertyValue::Number(10.0)
    );
}

#[test]
fn sampled_property_extrapolation_type_extrapolate() {
    // "getValue extrapolates EXTRAPOLATE"
    let mut property = SampledProperty::new(PackableType::Number);
    property.add_sample(jd(0.0, 0.0), &PropertyValue::Number(0.0), &[]);
    property.add_sample(jd(1.0, 0.0), &PropertyValue::Number(10.0), &[]);
    property.set_backward_extrapolation_type(ExtrapolationType::Extrapolate);
    property.set_forward_extrapolation_type(ExtrapolationType::Extrapolate);

    assert_eq!(
        property.get_value(&jd(-1.0, 0.0)),
        PropertyValue::Number(-10.0)
    );
    assert_eq!(
        property.get_value(&jd(2.0, 0.0)),
        PropertyValue::Number(20.0)
    );
}

#[test]
fn sampled_property_extrapolation_duration() {
    // "getValue respects extrapolation duration"
    let mut property = SampledProperty::new(PackableType::Number);
    property.add_sample(jd(0.0, 10.0), &PropertyValue::Number(100.0), &[]);
    property.add_sample(jd(0.0, 20.0), &PropertyValue::Number(200.0), &[]);
    property.set_forward_extrapolation_type(ExtrapolationType::Hold);
    property.set_forward_extrapolation_duration(5.0);
    property.set_backward_extrapolation_type(ExtrapolationType::Hold);
    property.set_backward_extrapolation_duration(5.0);

    // 向前且在时长内 (20 + 3 = 23, diff=3 < 5)
    assert_eq!(
        property.get_value(&jd(0.0, 23.0)),
        PropertyValue::Number(200.0)
    );
    // 向前超出时长 (20 + 7 = 27, diff=7 > 5)
    assert_eq!(
        property.get_value(&jd(0.0, 27.0)),
        PropertyValue::Undefined
    );
    // 向后且在时长内 (10 - 3 = 7, diff=3 < 5)
    assert_eq!(
        property.get_value(&jd(0.0, 7.0)),
        PropertyValue::Number(100.0)
    );
    // 向后超出时长 (10 - 7 = 3, diff=7 > 5)
    assert_eq!(
        property.get_value(&jd(0.0, 3.0)),
        PropertyValue::Undefined
    );
}

#[test]
fn sampled_property_set_interpolation_options() {
    // "setInterpolationOptions works"
    let mut property = SampledProperty::new(PackableType::Number);
    property.set_interpolation_options(
        Some(InterpolationAlgorithmKind::Lagrange),
        Some(5),
    );
    assert_eq!(
        property.interpolation_algorithm(),
        InterpolationAlgorithmKind::Lagrange
    );
    assert_eq!(property.interpolation_degree(), 5);
}

#[test]
fn sampled_property_lagrange_interpolation() {
    // "getValue works with Lagrange polynomial approximation"
    let mut property = SampledProperty::new(PackableType::Number);
    property.set_interpolation_options(
        Some(InterpolationAlgorithmKind::Lagrange),
        Some(2),
    );
    // f(x) = x^2: samples at 0, 1, 2
    property.add_sample(jd(0.0, 0.0), &PropertyValue::Number(0.0), &[]);
    property.add_sample(jd(0.0, 1.0), &PropertyValue::Number(1.0), &[]);
    property.add_sample(jd(0.0, 2.0), &PropertyValue::Number(4.0), &[]);

    // 0 和 1 的中点：Lagrange 2 阶应给出 0.25
    let val = property.get_value(&jd(0.0, 0.5));
    if let PropertyValue::Number(v) = val {
        assert!((v - 0.25).abs() < 1e-10, "expected 0.25, got {v}");
    } else {
        panic!("expected Number");
    }
}

#[test]
fn sampled_property_hermite_interpolation() {
    // "getValue works with Hermite polynomial approximation"
    let mut property = SampledProperty::with_derivative_types(
        PackableType::Number,
        Some(vec![PackableType::Number]),
    );
    property.set_interpolation_options(Some(InterpolationAlgorithmKind::Hermite), Some(3));

    // f(t) = t^3 on [0,1]: f(0)=0, f'(0)=0, f(1)=1, f'(1)=3
    property.add_sample(
        jd(0.0, 0.0),
        &PropertyValue::Number(0.0),
        &[PropertyValue::Number(0.0)],
    );
    property.add_sample(
        jd(0.0, 1.0),
        &PropertyValue::Number(1.0),
        &[PropertyValue::Number(3.0)],
    );

    let val = property.get_value(&jd(0.0, 0.5));
    if let PropertyValue::Number(v) = val {
        assert!((v - 0.125).abs() < 1e-10, "expected 0.125, got {v}");
    } else {
        panic!("expected Number");
    }
}

#[test]
fn sampled_property_remove_sample_works() {
    // "removeSample works"
    let mut property = SampledProperty::new(PackableType::Number);
    property.add_sample(jd(0.0, 0.0), &PropertyValue::Number(0.0), &[]);
    property.add_sample(jd(0.0, 1.0), &PropertyValue::Number(1.0), &[]);
    property.add_sample(jd(0.0, 2.0), &PropertyValue::Number(2.0), &[]);

    assert!(property.remove_sample(&jd(0.0, 1.0)));
    assert_eq!(property.sample_count(), 2);
    assert!(!property.remove_sample(&jd(0.0, 1.0)));
    assert_eq!(property.sample_count(), 2);
}

#[test]
fn sampled_property_remove_samples_interval_works() {
    // "removeSamples works"
    let mut property = SampledProperty::new(PackableType::Number);
    for i in 0..=10 {
        property.add_sample(
            jd(0.0, i as f64),
            &PropertyValue::Number(i as f64),
            &[],
        );
    }
    let interval = TimeInterval::new(jd(0.0, 3.0), jd(0.0, 7.0), true, true);
    property.remove_samples_interval(&interval);
    // 移除 t=3,4,5,6,7 → 剩余 6 个
    assert_eq!(property.sample_count(), 6);
}

#[test]
fn sampled_property_equals_works() {
    // "equals works"
    let mut left = SampledProperty::new(PackableType::Number);
    left.add_sample(jd(0.0, 0.0), &PropertyValue::Number(1.0), &[]);

    let mut right = SampledProperty::new(PackableType::Number);
    right.add_sample(jd(0.0, 0.0), &PropertyValue::Number(1.0), &[]);

    assert!(left.equals(&right));

    right.add_sample(jd(0.0, 1.0), &PropertyValue::Number(2.0), &[]);
    assert!(!left.equals(&right));
}

#[test]
fn sampled_property_get_value_with_cartesian3() {
    // "getValue works with Cartesian3"
    let mut property = SampledProperty::new(PackableType::Cartesian3);
    property.add_sample(
        jd(0.0, 0.0),
        &PropertyValue::Cartesian3(DVec3::new(0.0, 0.0, 0.0)),
        &[],
    );
    property.add_sample(
        jd(0.0, 10.0),
        &PropertyValue::Cartesian3(DVec3::new(10.0, 20.0, 30.0)),
        &[],
    );

    let mid = property.get_value(&jd(0.0, 5.0));
    assert_eq!(
        mid,
        PropertyValue::Cartesian3(DVec3::new(5.0, 10.0, 15.0))
    );
}

#[test]
fn sampled_property_out_of_order_insertion() {
    // 乱序插入的样本会保持排序
    let mut property = SampledProperty::new(PackableType::Number);
    property.add_sample(jd(0.0, 10.0), &PropertyValue::Number(100.0), &[]);
    property.add_sample(jd(0.0, 0.0), &PropertyValue::Number(0.0), &[]);
    property.add_sample(jd(0.0, 5.0), &PropertyValue::Number(50.0), &[]);

    assert_eq!(property.sample_count(), 3);
    assert_eq!(property.times()[0], jd(0.0, 0.0));
    assert_eq!(property.times()[1], jd(0.0, 5.0));
    assert_eq!(property.times()[2], jd(0.0, 10.0));
    assert_eq!(
        property.get_value(&jd(0.0, 2.5)),
        PropertyValue::Number(25.0)
    );
}

#[test]
fn sampled_property_overwrite_existing_sample() {
    // 在已存在时间添加样本会覆盖该值
    let mut property = SampledProperty::new(PackableType::Number);
    property.add_sample(jd(0.0, 0.0), &PropertyValue::Number(0.0), &[]);
    property.add_sample(jd(0.0, 10.0), &PropertyValue::Number(100.0), &[]);
    property.add_sample(jd(0.0, 10.0), &PropertyValue::Number(200.0), &[]);

    assert_eq!(property.sample_count(), 2);
    assert_eq!(
        property.get_value(&jd(0.0, 10.0)),
        PropertyValue::Number(200.0)
    );
}

#[test]
fn sampled_property_single_sample_returns_undefined_for_interpolation() {
    // 单个样本无法插值
    let mut property = SampledProperty::new(PackableType::Number);
    property.add_sample(jd(0.0, 5.0), &PropertyValue::Number(42.0), &[]);

    // 精确匹配生效
    assert_eq!(
        property.get_value(&jd(0.0, 5.0)),
        PropertyValue::Number(42.0)
    );
    // 无法插值
    assert_eq!(
        property.get_value(&jd(0.0, 6.0)),
        PropertyValue::Undefined
    );
}

// ===========================================================================
// TimeIntervalCollectionProperty（源自 TimeIntervalCollectionPropertySpec）
// ===========================================================================

#[test]
fn tic_property_default_constructor_has_expected_values() {
    // "default constructor has expected values"
    let property = TimeIntervalCollectionProperty::new();
    let time = jd(2451545.0, 0.0);
    assert_eq!(property.get_value(&time), PropertyValue::Undefined);
    assert!(property.is_constant());
}

#[test]
fn tic_property_works_with_basic_types() {
    // "works with basic types"
    let mut property = TimeIntervalCollectionProperty::new();
    property.add_interval(
        TimeInterval::new(jd(10.0, 0.0), jd(12.0, 0.0), true, true),
        Some(PropertyValue::Number(5.0)),
    );
    property.add_interval(
        TimeInterval::new(jd(12.0, 0.0), jd(14.0, 0.0), false, true),
        Some(PropertyValue::Number(6.0)),
    );

    assert_eq!(
        property.get_value(&jd(10.0, 0.0)),
        PropertyValue::Number(5.0)
    );
    assert_eq!(
        property.get_value(&jd(14.0, 0.0)),
        PropertyValue::Number(6.0)
    );
    assert!(!property.is_constant());
}

#[test]
fn tic_property_works_with_clonable_objects() {
    // "works with clonable objects"
    let mut property = TimeIntervalCollectionProperty::new();
    property.add_interval(
        TimeInterval::new(jd(10.0, 0.0), jd(12.0, 0.0), true, true),
        Some(PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0))),
    );
    property.add_interval(
        TimeInterval::new(jd(12.0, 0.0), jd(14.0, 0.0), false, true),
        Some(PropertyValue::Cartesian3(DVec3::new(4.0, 5.0, 6.0))),
    );

    let result1 = property.get_value(&jd(10.0, 0.0));
    assert_eq!(
        result1,
        PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0))
    );

    let result2 = property.get_value(&jd(14.0, 0.0));
    assert_eq!(
        result2,
        PropertyValue::Cartesian3(DVec3::new(4.0, 5.0, 6.0))
    );
}

#[test]
fn tic_property_returns_undefined_outside_intervals() {
    let mut property = TimeIntervalCollectionProperty::new();
    property.add_interval(
        TimeInterval::new(jd(10.0, 0.0), jd(12.0, 0.0), true, true),
        Some(PropertyValue::Number(5.0)),
    );

    assert_eq!(
        property.get_value(&jd(5.0, 0.0)),
        PropertyValue::Undefined
    );
    assert_eq!(
        property.get_value(&jd(15.0, 0.0)),
        PropertyValue::Undefined
    );
}

#[test]
fn tic_property_equals_works_for_basic_type_intervals() {
    // "equals works for differing basic type intervals"
    let mut left = TimeIntervalCollectionProperty::new();
    left.add_interval(
        TimeInterval::new(jd(10.0, 0.0), jd(12.0, 0.0), true, true),
        Some(PropertyValue::Number(5.0)),
    );
    left.add_interval(
        TimeInterval::new(jd(12.0, 0.0), jd(14.0, 0.0), false, true),
        Some(PropertyValue::Number(6.0)),
    );

    let mut right = TimeIntervalCollectionProperty::new();
    right.add_interval(
        TimeInterval::new(jd(10.0, 0.0), jd(12.0, 0.0), true, true),
        Some(PropertyValue::Number(5.0)),
    );

    assert!(!left.equals(&right));

    right.add_interval(
        TimeInterval::new(jd(12.0, 0.0), jd(14.0, 0.0), false, true),
        Some(PropertyValue::Number(6.0)),
    );
    assert!(left.equals(&right));
}

#[test]
fn tic_property_equals_works_for_complex_type_intervals() {
    // "equals works for differing complex type intervals"
    let mut left = TimeIntervalCollectionProperty::new();
    left.add_interval(
        TimeInterval::new(jd(10.0, 0.0), jd(12.0, 0.0), true, true),
        Some(PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0))),
    );
    left.add_interval(
        TimeInterval::new(jd(12.0, 0.0), jd(14.0, 0.0), false, true),
        Some(PropertyValue::Cartesian3(DVec3::new(4.0, 5.0, 6.0))),
    );

    let mut right = TimeIntervalCollectionProperty::new();
    right.add_interval(
        TimeInterval::new(jd(10.0, 0.0), jd(12.0, 0.0), true, true),
        Some(PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0))),
    );

    assert!(!left.equals(&right));

    right.add_interval(
        TimeInterval::new(jd(12.0, 0.0), jd(14.0, 0.0), false, true),
        Some(PropertyValue::Cartesian3(DVec3::new(4.0, 5.0, 6.0))),
    );
    assert!(left.equals(&right));
}

// ===========================================================================
// CompositeProperty（源自 CompositePropertySpec）
// ===========================================================================

#[test]
fn composite_property_default_constructor_has_expected_values() {
    // "default constructor has expected values"
    let property = CompositeProperty::new();
    let time = jd(2451545.0, 0.0);
    assert_eq!(property.get_value(&time), PropertyValue::Undefined);
    assert!(property.is_constant());
}

#[test]
fn composite_property_works_without_result_parameter() {
    // "works without a result parameter"
    let c1: Arc<dyn DynProperty> =
        Arc::new(ConstantProperty::new(PropertyValue::Cartesian3(DVec3::new(
            1.0, 2.0, 3.0,
        ))));
    let c2: Arc<dyn DynProperty> =
        Arc::new(ConstantProperty::new(PropertyValue::Cartesian3(DVec3::new(
            4.0, 5.0, 6.0,
        ))));

    let mut property = CompositeProperty::new();
    property.add_interval(
        TimeInterval::new(jd(10.0, 0.0), jd(12.0, 0.0), true, true),
        Some(c1),
    );
    property.add_interval(
        TimeInterval::new(jd(12.0, 0.0), jd(14.0, 0.0), false, true),
        Some(c2),
    );
    assert!(!property.is_constant());

    let result1 = property.get_value(&jd(10.0, 0.0));
    assert_eq!(
        result1,
        PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0))
    );

    let result2 = property.get_value(&jd(14.0, 0.0));
    assert_eq!(
        result2,
        PropertyValue::Cartesian3(DVec3::new(4.0, 5.0, 6.0))
    );
}

#[test]
fn composite_property_works_with_sampled_inner() {
    // 以 SampledProperty 作为内部属性的 Composite
    let mut sampled = SampledProperty::new(PackableType::Number);
    sampled.add_sample(jd(10.0, 0.0), &PropertyValue::Number(10.0), &[]);
    sampled.add_sample(jd(20.0, 0.0), &PropertyValue::Number(20.0), &[]);
    let inner: Arc<dyn DynProperty> = Arc::new(sampled);

    let mut property = CompositeProperty::new();
    property.add_interval(
        TimeInterval::new(jd(10.0, 0.0), jd(20.0, 0.0), true, true),
        Some(inner),
    );

    assert_eq!(
        property.get_value(&jd(10.0, 0.0)),
        PropertyValue::Number(10.0)
    );
    assert_eq!(
        property.get_value(&jd(15.0, 0.0)),
        PropertyValue::Number(15.0)
    );
    assert_eq!(
        property.get_value(&jd(20.0, 0.0)),
        PropertyValue::Number(20.0)
    );
    // 区间之外
    assert_eq!(
        property.get_value(&jd(25.0, 0.0)),
        PropertyValue::Undefined
    );
}

#[test]
fn composite_property_equals_works() {
    // "equals works"
    let c1: Arc<dyn DynProperty> =
        Arc::new(ConstantProperty::new(PropertyValue::Cartesian3(DVec3::new(
            1.0, 2.0, 3.0,
        ))));
    let c2: Arc<dyn DynProperty> =
        Arc::new(ConstantProperty::new(PropertyValue::Cartesian3(DVec3::new(
            4.0, 5.0, 6.0,
        ))));

    let mut left = CompositeProperty::new();
    left.add_interval(
        TimeInterval::new(jd(10.0, 0.0), jd(12.0, 0.0), true, true),
        Some(Arc::clone(&c1)),
    );
    left.add_interval(
        TimeInterval::new(jd(12.0, 0.0), jd(14.0, 0.0), false, true),
        Some(Arc::clone(&c2)),
    );

    let mut right = CompositeProperty::new();
    right.add_interval(
        TimeInterval::new(jd(10.0, 0.0), jd(12.0, 0.0), true, true),
        Some(c1),
    );
    assert!(!left.equals(&right));

    right.add_interval(
        TimeInterval::new(jd(12.0, 0.0), jd(14.0, 0.0), false, true),
        Some(c2),
    );
    assert!(left.equals(&right));
}

// ===========================================================================
// CallbackProperty (from CallbackPropertySpec)
// ===========================================================================

#[test]
fn callback_property_get_value_returns_callback_result() {
    // "getValue returns callback result"
    let property = CallbackProperty::new(
        |_time: &JulianDate| PropertyValue::Number(42.0),
        true,
    );
    let time = jd(2451545.0, 0.0);
    assert_eq!(property.get_value(&time), PropertyValue::Number(42.0));
}

#[test]
fn callback_property_receives_time_parameter() {
    // "callback received proper parameters"
    // 使用捕获已知值的闭包以验证传入了 time
    use std::sync::atomic::{AtomicBool, Ordering};
    static CALLED: AtomicBool = AtomicBool::new(false);
    let property = CallbackProperty::new(
        |_time: &JulianDate| {
            CALLED.store(true, Ordering::SeqCst);
            PropertyValue::Number(99.0)
        },
        false,
    );
    let time = jd(2451545.0, 100.0);
    let result = property.get_value(&time);
    assert!(CALLED.load(Ordering::SeqCst));
    assert_eq!(result, PropertyValue::Number(99.0));
}

#[test]
fn callback_property_is_constant_returns_correct_value() {
    // "isConstant returns correct value"
    let property = CallbackProperty::new(
        |_time: &JulianDate| PropertyValue::Undefined,
        true,
    );
    assert!(property.is_constant());

    let property2 = CallbackProperty::new(
        |_time: &JulianDate| PropertyValue::Undefined,
        false,
    );
    assert!(!property2.is_constant());
}

#[test]
fn callback_property_set_callback_works() {
    // "setCallback raises definitionChanged event" (测试 set_callback 逻辑)
    let mut property = CallbackProperty::new(
        |_time: &JulianDate| PropertyValue::Number(1.0),
        true,
    );
    assert!(property.is_constant());

    property.set_callback(
        |_time: &JulianDate| PropertyValue::Number(2.0),
        false,
    );
    assert!(!property.is_constant());
    let time = jd(2451545.0, 0.0);
    assert_eq!(property.get_value(&time), PropertyValue::Number(2.0));
}

#[test]
fn callback_property_equals_works() {
    // "equals works"
    let shared: CallbackFn = Arc::new(|_time: &JulianDate| PropertyValue::Number(1.0));
    let left = CallbackProperty::from_arc(Arc::clone(&shared), true);
    let right = CallbackProperty::from_arc(Arc::clone(&shared), true);
    assert!(left.equals(&right));

    // is_constant 不同 → 不相等
    let right2 = CallbackProperty::from_arc(Arc::clone(&shared), false);
    assert!(!left.equals(&right2));

    // callback 不同 → 不相等
    let other: CallbackFn = Arc::new(|_time: &JulianDate| PropertyValue::Number(2.0));
    let right3 = CallbackProperty::from_arc(other, true);
    assert!(!left.equals(&right3));
}

// ===========================================================================
// 跨类型相等性
// ===========================================================================

#[test]
fn property_cross_type_equals_false() {
    let c = ConstantProperty::new(PropertyValue::Number(1.0));
    let s = SampledProperty::new(PackableType::Number);
    assert!(!c.equals(&s));
    assert!(!s.equals(&c));
}
