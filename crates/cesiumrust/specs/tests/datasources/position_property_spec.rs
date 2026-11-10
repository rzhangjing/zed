//! 对齐实现自 CesiumJS DataSources 位置属性规范：
//! - ConstantPositionPropertySpec (15 it())
//! - SampledPositionPropertySpec (27 it())
//! - CompositePositionPropertySpec (12 it())
//! - TimeIntervalCollectionPositionPropertySpec (10 it())
//! - CallbackPositionPropertySpec (8 it())
//!
//! A 类测试（纯逻辑，无 DOM/事件/spy）：约 42 个测试

use cesium_datasource::property_system::{
    convert_to_reference_frame, CallbackPositionProperty, CompositePositionProperty,
    ConstantPositionProperty, DynProperty, ExtrapolationType, InterpolationAlgorithmKind,
    PositionCallbackFn, PropertyValue, ReferenceFrame, SampledPositionProperty,
    TimeIntervalCollectionPositionProperty,
};
use cesium_time::{JulianDate, TimeInterval};
use glam::DVec3;
use std::sync::Arc;

fn jd(day: f64, seconds: f64) -> JulianDate {
    JulianDate::new(day, seconds)
}

// ===========================================================================
// ConstantPositionProperty（源自 ConstantPositionPropertySpec）
// ===========================================================================

#[test]
fn constant_position_constructor_sets_expected_defaults() {
    // "Constructor sets expected defaults"
    let property = ConstantPositionProperty::undefined();
    assert_eq!(
        property.reference_frame(),
        Some(ReferenceFrame::Fixed)
    );

    let property2 = ConstantPositionProperty::with_reference_frame(
        DVec3::new(1.0, 2.0, 3.0),
        ReferenceFrame::Inertial,
    );
    assert_eq!(
        property2.reference_frame(),
        Some(ReferenceFrame::Inertial)
    );
}

#[test]
fn constant_position_get_value_works() {
    // "getValue works without a result parameter"
    let value = DVec3::new(1.0, 2.0, 3.0);
    let property = ConstantPositionProperty::new(value);
    let time = jd(2451545.0, 0.0);

    let result = property.get_value(&time);
    assert_eq!(result, PropertyValue::Cartesian3(value));
}

#[test]
fn constant_position_get_value_returns_in_fixed_frame() {
    // "getValue returns in fixed frame"
    let time = jd(2451545.0, 0.0);
    let value_inertial = DVec3::new(1.0, 2.0, 3.0);
    let value_fixed = convert_to_reference_frame(
        &time,
        value_inertial,
        ReferenceFrame::Inertial,
        ReferenceFrame::Fixed,
    )
    .unwrap();

    let property =
        ConstantPositionProperty::with_reference_frame(value_inertial, ReferenceFrame::Inertial);
    let result = property.get_value(&time);
    assert_eq!(result, PropertyValue::Cartesian3(value_fixed));
}

#[test]
fn constant_position_get_value_works_with_undefined_fixed() {
    // "getValue works with undefined fixed value"
    let property = ConstantPositionProperty::undefined();
    let time = jd(2451545.0, 0.0);
    assert_eq!(property.get_value(&time), PropertyValue::Undefined);
}

#[test]
fn constant_position_get_value_works_with_undefined_inertial() {
    // "getValue works with undefined inertial value"
    let mut p = ConstantPositionProperty::undefined();
    p.set_value(None, Some(ReferenceFrame::Inertial));
    let time = jd(2451545.0, 0.0);
    assert_eq!(p.get_value(&time), PropertyValue::Undefined);
}

#[test]
fn constant_position_get_value_in_reference_frame_works() {
    // "getValueInReferenceFrame works without a result parameter"
    let time = jd(2451545.0, 0.0);
    let value = DVec3::new(1.0, 2.0, 3.0);
    let property = ConstantPositionProperty::new(value);

    let expected = convert_to_reference_frame(
        &time,
        value,
        ReferenceFrame::Fixed,
        ReferenceFrame::Inertial,
    )
    .unwrap();

    let result = property.position_in_reference_frame(&time, ReferenceFrame::Inertial);
    assert_eq!(result, Some(expected));
}

#[test]
fn constant_position_equals_works() {
    // "equals works"
    let left = ConstantPositionProperty::with_reference_frame(
        DVec3::new(1.0, 2.0, 3.0),
        ReferenceFrame::Inertial,
    );
    let right = ConstantPositionProperty::with_reference_frame(
        DVec3::new(1.0, 2.0, 3.0),
        ReferenceFrame::Inertial,
    );
    assert!(left.equals(&right));

    let right2 = ConstantPositionProperty::with_reference_frame(
        DVec3::new(1.0, 2.0, 3.0),
        ReferenceFrame::Fixed,
    );
    assert!(!left.equals(&right2));

    let right3 = ConstantPositionProperty::with_reference_frame(
        DVec3::new(1.0, 2.0, 4.0),
        ReferenceFrame::Inertial,
    );
    assert!(!left.equals(&right3));
}

// ===========================================================================
// SampledPositionProperty（源自 SampledPositionPropertySpec）
// ===========================================================================

#[test]
fn sampled_position_constructor_sets_expected_defaults() {
    // "constructor sets expected defaults"
    let property = SampledPositionProperty::fixed();
    assert_eq!(property.reference_frame(), Some(ReferenceFrame::Fixed));
    assert_eq!(property.interpolation_degree(), 1);
    assert_eq!(
        property.interpolation_algorithm(),
        InterpolationAlgorithmKind::Linear
    );
    assert_eq!(property.number_of_derivatives(), 0);
}

#[test]
fn sampled_position_constructor_sets_expected_values() {
    // "constructor sets expected values"
    let property = SampledPositionProperty::new(ReferenceFrame::Inertial, 1);
    assert_eq!(property.reference_frame(), Some(ReferenceFrame::Inertial));
    assert_eq!(property.interpolation_degree(), 1);
    assert_eq!(
        property.interpolation_algorithm(),
        InterpolationAlgorithmKind::Linear
    );
    assert_eq!(property.number_of_derivatives(), 1);
}

#[test]
fn sampled_position_get_value_works() {
    // "getValue works without a result parameter"
    let time = jd(2451545.0, 0.0);
    let value = DVec3::new(1.0, 2.0, 3.0);
    let mut property = SampledPositionProperty::fixed();
    property.add_sample(time, value, &[]);

    let result = property.get_value(&time);
    assert_eq!(result, PropertyValue::Cartesian3(value));
}

#[test]
fn sampled_position_get_value_returns_in_fixed_frame() {
    // "getValue returns in fixed frame"
    let time = jd(2451545.0, 0.0);
    let value_inertial = DVec3::new(1.0, 2.0, 3.0);
    let value_fixed = convert_to_reference_frame(
        &time,
        value_inertial,
        ReferenceFrame::Inertial,
        ReferenceFrame::Fixed,
    )
    .unwrap();

    let mut property = SampledPositionProperty::new(ReferenceFrame::Inertial, 0);
    property.add_sample(time, value_inertial, &[]);

    let result = property.get_value(&time);
    assert_eq!(result, PropertyValue::Cartesian3(value_fixed));
}

#[test]
fn sampled_position_get_value_in_reference_frame_works() {
    // "getValueInReferenceFrame works without a result parameter"
    let time = jd(2451545.0, 0.0);
    let value = DVec3::new(1.0, 2.0, 3.0);
    let mut property = SampledPositionProperty::fixed();
    property.add_sample(time, value, &[]);

    let expected = convert_to_reference_frame(
        &time,
        value,
        ReferenceFrame::Fixed,
        ReferenceFrame::Inertial,
    )
    .unwrap();

    let result = property.position_in_reference_frame(&time, ReferenceFrame::Inertial);
    assert_eq!(result, Some(expected));
}

#[test]
fn sampled_position_add_samples_packed_array_works() {
    // "addSamplesPackedArray works"
    let data = [0.0, 7.0, 8.0, 9.0, 1.0, 8.0, 9.0, 10.0, 2.0, 9.0, 10.0, 11.0];
    let epoch = jd(0.0, 0.0);

    let mut property = SampledPositionProperty::fixed();
    property.add_samples_packed_array(&data, &epoch);

    assert_eq!(
        property.get_value(&epoch),
        PropertyValue::Cartesian3(DVec3::new(7.0, 8.0, 9.0))
    );
    assert_eq!(
        property.get_value(&jd(0.0, 0.5)),
        PropertyValue::Cartesian3(DVec3::new(7.5, 8.5, 9.5))
    );
}

#[test]
fn sampled_position_add_sample_works() {
    // "addSample works"
    let times = [jd(0.0, 0.0), jd(1.0, 0.0), jd(2.0, 0.0)];
    let values = [
        DVec3::new(7.0, 8.0, 9.0),
        DVec3::new(8.0, 9.0, 10.0),
        DVec3::new(9.0, 10.0, 11.0),
    ];

    let mut property = SampledPositionProperty::fixed();
    property.add_sample(times[0], values[0], &[]);
    property.add_sample(times[1], values[1], &[]);
    property.add_sample(times[2], values[2], &[]);

    assert_eq!(
        property.get_value(&times[0]),
        PropertyValue::Cartesian3(values[0])
    );
    assert_eq!(
        property.get_value(&times[1]),
        PropertyValue::Cartesian3(values[1])
    );
    assert_eq!(
        property.get_value(&times[2]),
        PropertyValue::Cartesian3(values[2])
    );
    assert_eq!(
        property.get_value(&jd(0.5, 0.0)),
        PropertyValue::Cartesian3(DVec3::new(7.5, 8.5, 9.5))
    );
}

#[test]
fn sampled_position_add_samples_works() {
    // "addSamples works"
    let times = [jd(0.0, 0.0), jd(1.0, 0.0), jd(2.0, 0.0)];
    let values = [
        DVec3::new(7.0, 8.0, 9.0),
        DVec3::new(8.0, 9.0, 10.0),
        DVec3::new(9.0, 10.0, 11.0),
    ];

    let mut property = SampledPositionProperty::fixed();
    property.add_samples(&times, &values, None);

    assert_eq!(
        property.get_value(&times[0]),
        PropertyValue::Cartesian3(values[0])
    );
    assert_eq!(
        property.get_value(&times[1]),
        PropertyValue::Cartesian3(values[1])
    );
    assert_eq!(
        property.get_value(&jd(0.5, 0.0)),
        PropertyValue::Cartesian3(DVec3::new(7.5, 8.5, 9.5))
    );
}

#[test]
fn sampled_position_can_remove_sample() {
    // "can remove a sample at a date"
    let times = [jd(0.0, 0.0), jd(1.0, 0.0), jd(2.0, 0.0)];
    let values = [
        DVec3::new(7.0, 8.0, 9.0),
        DVec3::new(18.0, 19.0, 110.0),
        DVec3::new(9.0, 10.0, 11.0),
    ];

    let mut property = SampledPositionProperty::fixed();
    property.add_samples(&times, &values, None);

    // 移除不存在的
    let result = property.remove_sample(&jd(4.0, 0.0));
    assert!(!result);

    // 移除中间样本
    let result = property.remove_sample(&times[1]);
    assert!(result);

    assert_eq!(
        property.get_value(&times[0]),
        PropertyValue::Cartesian3(values[0])
    );
    // 移除中间样本导致首尾之间插值
    assert_eq!(
        property.get_value(&times[1]),
        PropertyValue::Cartesian3(DVec3::new(8.0, 9.0, 10.0))
    );
    assert_eq!(
        property.get_value(&times[2]),
        PropertyValue::Cartesian3(values[2])
    );
}

#[test]
fn sampled_position_can_remove_samples_interval() {
    // "can remove samples for a time interval"
    let times = [jd(0.0, 0.0), jd(1.0, 0.0), jd(2.0, 0.0), jd(3.0, 0.0)];
    let values = [
        DVec3::new(7.0, 8.0, 9.0),
        DVec3::new(18.0, 19.0, 110.0),
        DVec3::new(19.0, 20.0, 110.0),
        DVec3::new(10.0, 11.0, 12.0),
    ];

    let mut property = SampledPositionProperty::fixed();
    property.add_samples(&times, &values, None);

    let interval = TimeInterval::new(times[1], times[2], true, true);
    property.remove_samples_interval(&interval);

    assert_eq!(
        property.get_value(&times[0]),
        PropertyValue::Cartesian3(values[0])
    );
    // 移除中间样本导致插值
    assert_eq!(
        property.get_value(&times[1]),
        PropertyValue::Cartesian3(DVec3::new(8.0, 9.0, 10.0))
    );
    assert_eq!(
        property.get_value(&times[2]),
        PropertyValue::Cartesian3(DVec3::new(9.0, 10.0, 11.0))
    );
    assert_eq!(
        property.get_value(&times[3]),
        PropertyValue::Cartesian3(values[3])
    );
}

#[test]
fn sampled_position_add_samples_packed_array_with_derivatives() {
    // "addSamplesPackedArray works with derivatives"
    // stride = 1(time) + 3(position) + 3(derivative) = 7
    let data = [
        0.0, 7.0, 8.0, 9.0, 1.0, 0.0, 0.0,
        1.0, 8.0, 9.0, 10.0, 0.0, 1.0, 0.0,
        2.0, 9.0, 10.0, 11.0, 0.0, 0.0, 1.0,
    ];
    let epoch = jd(0.0, 0.0);

    let mut property = SampledPositionProperty::new(ReferenceFrame::Fixed, 1);
    property.add_samples_packed_array(&data, &epoch);

    assert_eq!(
        property.get_value(&epoch),
        PropertyValue::Cartesian3(DVec3::new(7.0, 8.0, 9.0))
    );
    assert_eq!(
        property.get_value(&jd(0.0, 0.5)),
        PropertyValue::Cartesian3(DVec3::new(7.5, 8.5, 9.5))
    );
}

#[test]
fn sampled_position_add_sample_with_derivatives() {
    // "addSample works with derivatives"
    let times = [jd(0.0, 0.0), jd(1.0, 0.0), jd(2.0, 0.0)];
    let positions = [
        DVec3::new(7.0, 8.0, 9.0),
        DVec3::new(8.0, 9.0, 10.0),
        DVec3::new(9.0, 10.0, 11.0),
    ];
    let velocities = [
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 1.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
    ];

    let mut property = SampledPositionProperty::new(ReferenceFrame::Fixed, 1);
    property.add_sample(times[0], positions[0], &[velocities[0]]);
    property.add_sample(times[1], positions[1], &[velocities[1]]);
    property.add_sample(times[2], positions[2], &[velocities[2]]);

    assert_eq!(
        property.get_value(&times[0]),
        PropertyValue::Cartesian3(positions[0])
    );
    assert_eq!(
        property.get_value(&times[1]),
        PropertyValue::Cartesian3(positions[1])
    );
    assert_eq!(
        property.get_value(&times[2]),
        PropertyValue::Cartesian3(positions[2])
    );
    assert_eq!(
        property.get_value(&jd(0.5, 0.0)),
        PropertyValue::Cartesian3(DVec3::new(7.5, 8.5, 9.5))
    );
}

#[test]
fn sampled_position_add_samples_with_derivatives() {
    // "addSamples works with derivatives"
    let times = [jd(0.0, 0.0), jd(1.0, 0.0), jd(2.0, 0.0)];
    let positions = [
        DVec3::new(7.0, 8.0, 9.0),
        DVec3::new(8.0, 9.0, 10.0),
        DVec3::new(9.0, 10.0, 11.0),
    ];
    let velocities: Vec<Vec<DVec3>> = vec![
        vec![DVec3::new(0.0, 0.0, 1.0)],
        vec![DVec3::new(0.0, 1.0, 0.0)],
        vec![DVec3::new(1.0, 0.0, 0.0)],
    ];

    let mut property = SampledPositionProperty::new(ReferenceFrame::Fixed, 1);
    property.add_samples(&times, &positions, Some(&velocities));

    assert_eq!(
        property.get_value(&times[0]),
        PropertyValue::Cartesian3(positions[0])
    );
    assert_eq!(
        property.get_value(&times[1]),
        PropertyValue::Cartesian3(positions[1])
    );
    assert_eq!(
        property.get_value(&jd(0.5, 0.0)),
        PropertyValue::Cartesian3(DVec3::new(7.5, 8.5, 9.5))
    );
}

#[test]
fn sampled_position_returns_undefined_not_enough_samples() {
    // "Returns undefined if trying to interpolate with less than enough samples."
    let value = DVec3::new(1.0, 2.0, 3.0);
    let time = jd(0.0, 0.0);

    let mut property = SampledPositionProperty::fixed();
    property.add_sample(time, value, &[]);

    assert_eq!(
        property.get_value(&time),
        PropertyValue::Cartesian3(value)
    );
    // 仅 1 个样本时，在其他时间插值返回 undefined
    assert_eq!(
        property.get_value(&jd(0.0, 4.0)),
        PropertyValue::Undefined
    );
}

#[test]
fn sampled_position_equals_interpolators_differ() {
    // "equals works when interpolators differ"
    let left = SampledPositionProperty::fixed();
    let mut right = SampledPositionProperty::fixed();
    assert!(left.equals(&right));

    right.set_interpolation_options(
        Some(InterpolationAlgorithmKind::Lagrange),
        None,
    );
    assert!(!left.equals(&right));
}

#[test]
fn sampled_position_equals_degree_differ() {
    // "equals works when interpolator degree differ"
    let mut left = SampledPositionProperty::fixed();
    left.set_interpolation_options(
        Some(InterpolationAlgorithmKind::Lagrange),
        Some(2),
    );

    let mut right = SampledPositionProperty::fixed();
    right.set_interpolation_options(
        Some(InterpolationAlgorithmKind::Lagrange),
        Some(2),
    );
    assert!(left.equals(&right));

    right.set_interpolation_options(
        Some(InterpolationAlgorithmKind::Lagrange),
        Some(3),
    );
    assert!(!left.equals(&right));
}

#[test]
fn sampled_position_equals_reference_frames_differ() {
    // "equals works when reference frames differ"
    let left = SampledPositionProperty::new(ReferenceFrame::Fixed, 0);
    let right = SampledPositionProperty::new(ReferenceFrame::Inertial, 0);
    assert!(!left.equals(&right));
}

#[test]
fn sampled_position_equals_samples_differ() {
    // "equals works when samples differ"
    let mut left = SampledPositionProperty::fixed();
    let mut right = SampledPositionProperty::fixed();
    assert!(left.equals(&right));

    let time = jd(2451545.0, 0.0);
    let value = DVec3::new(1.0, 2.0, 3.0);
    left.add_sample(time, value, &[]);
    assert!(!left.equals(&right));

    right.add_sample(time, value, &[]);
    assert!(left.equals(&right));
}

#[test]
fn sampled_position_extrapolation_hold() {
    // 外推对位置属性生效
    let mut property = SampledPositionProperty::fixed();
    property.add_sample(jd(0.0, 0.0), DVec3::new(0.0, 0.0, 0.0), &[]);
    property.add_sample(jd(1.0, 0.0), DVec3::new(10.0, 10.0, 10.0), &[]);
    property.set_backward_extrapolation_type(ExtrapolationType::Hold);
    property.set_forward_extrapolation_type(ExtrapolationType::Hold);

    assert_eq!(
        property.get_value(&jd(-1.0, 0.0)),
        PropertyValue::Cartesian3(DVec3::new(0.0, 0.0, 0.0))
    );
    assert_eq!(
        property.get_value(&jd(2.0, 0.0)),
        PropertyValue::Cartesian3(DVec3::new(10.0, 10.0, 10.0))
    );
}

// ===========================================================================
// CompositePositionProperty（源自 CompositePositionPropertySpec）
// ===========================================================================

#[test]
fn composite_position_default_constructor() {
    // "default constructor has expected values"
    let property = CompositePositionProperty::new(ReferenceFrame::Fixed);
    assert_eq!(property.reference_frame(), Some(ReferenceFrame::Fixed));
    assert!(property.is_constant());
    assert_eq!(
        property.get_value(&jd(2451545.0, 0.0)),
        PropertyValue::Undefined
    );
}

#[test]
fn composite_position_constructor_sets_values() {
    // "constructor sets expected values"
    let property = CompositePositionProperty::new(ReferenceFrame::Inertial);
    assert_eq!(property.reference_frame(), Some(ReferenceFrame::Inertial));
}

#[test]
fn composite_position_can_modify_reference_frame() {
    // "can modify reference frame"
    let mut property = CompositePositionProperty::new(ReferenceFrame::Fixed);
    assert_eq!(property.reference_frame(), Some(ReferenceFrame::Fixed));
    property.set_reference_frame(ReferenceFrame::Inertial);
    assert_eq!(property.reference_frame(), Some(ReferenceFrame::Inertial));
}

#[test]
fn composite_position_works_without_result_parameter() {
    // "works without a result parameter"
    let start1 = jd(10.0, 0.0);
    let stop1 = jd(12.0, 0.0);
    let start2 = jd(12.0, 0.0);
    let stop2 = jd(14.0, 0.0);
    let interval1 = TimeInterval::new(start1, stop1, true, true);
    let interval2 = TimeInterval::new(start2, stop2, false, true);

    let inner1 = Arc::new(ConstantPositionProperty::new(DVec3::new(1.0, 2.0, 3.0)))
        as Arc<dyn DynProperty>;
    let inner2 = Arc::new(ConstantPositionProperty::new(DVec3::new(4.0, 5.0, 6.0)))
        as Arc<dyn DynProperty>;

    let mut property = CompositePositionProperty::new(ReferenceFrame::Fixed);
    property.add_interval(interval1, Some(inner1));
    property.add_interval(interval2, Some(inner2));

    assert!(!property.is_constant());

    let result1 = property.get_value(&start1);
    assert_eq!(
        result1,
        PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0))
    );

    let result2 = property.get_value(&stop2);
    assert_eq!(
        result2,
        PropertyValue::Cartesian3(DVec3::new(4.0, 5.0, 6.0))
    );
}

#[test]
fn composite_position_equals() {
    // "equals works"
    let mut left = CompositePositionProperty::new(ReferenceFrame::Fixed);
    let right = CompositePositionProperty::new(ReferenceFrame::Fixed);
    assert!(left.equals(&right));

    let interval = TimeInterval::new(jd(10.0, 0.0), jd(12.0, 0.0), true, true);
    let inner = Arc::new(ConstantPositionProperty::new(DVec3::new(1.0, 2.0, 3.0)))
        as Arc<dyn DynProperty>;
    left.add_interval(interval, Some(inner));
    assert!(!left.equals(&right));
}

// ===========================================================================
// TimeIntervalCollectionPositionProperty
// （源自 TimeIntervalCollectionPositionPropertySpec）
// ===========================================================================

#[test]
fn tic_position_default_constructor() {
    // "default constructor has expected values"
    let property = TimeIntervalCollectionPositionProperty::new(ReferenceFrame::Fixed);
    assert_eq!(property.reference_frame(), Some(ReferenceFrame::Fixed));
    assert!(property.is_constant());
    assert_eq!(
        property.get_value(&jd(2451545.0, 0.0)),
        PropertyValue::Undefined
    );
}

#[test]
fn tic_position_get_value_works() {
    // "getValue works without a result parameter"
    let start1 = jd(10.0, 0.0);
    let stop1 = jd(12.0, 0.0);
    let start2 = jd(12.0, 0.0);
    let stop2 = jd(14.0, 0.0);
    let interval1 = TimeInterval::new(start1, stop1, true, true);
    let interval2 = TimeInterval::new(start2, stop2, false, true);

    let mut property = TimeIntervalCollectionPositionProperty::new(ReferenceFrame::Fixed);
    property.add_interval(interval1, Some(DVec3::new(1.0, 2.0, 3.0)));
    property.add_interval(interval2, Some(DVec3::new(4.0, 5.0, 6.0)));

    let result1 = property.get_value(&start1);
    assert_eq!(
        result1,
        PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0))
    );

    let result2 = property.get_value(&stop2);
    assert_eq!(
        result2,
        PropertyValue::Cartesian3(DVec3::new(4.0, 5.0, 6.0))
    );
}

#[test]
fn tic_position_get_value_returns_in_fixed_frame() {
    // "getValue returns in fixed frame"
    let start1 = jd(10.0, 0.0);
    let stop1 = jd(12.0, 0.0);
    let interval1 = TimeInterval::new(start1, stop1, true, true);
    let value_inertial = DVec3::new(1.0, 2.0, 3.0);
    let value_fixed = convert_to_reference_frame(
        &start1,
        value_inertial,
        ReferenceFrame::Inertial,
        ReferenceFrame::Fixed,
    )
    .unwrap();

    let mut property = TimeIntervalCollectionPositionProperty::new(ReferenceFrame::Inertial);
    property.add_interval(interval1, Some(value_inertial));

    let result = property.get_value(&start1);
    assert_eq!(result, PropertyValue::Cartesian3(value_fixed));
}

#[test]
fn tic_position_equals() {
    // "equals works"
    let mut left = TimeIntervalCollectionPositionProperty::new(ReferenceFrame::Fixed);
    let right = TimeIntervalCollectionPositionProperty::new(ReferenceFrame::Fixed);
    assert!(left.equals(&right));

    let interval = TimeInterval::new(jd(10.0, 0.0), jd(12.0, 0.0), true, true);
    left.add_interval(interval, Some(DVec3::new(1.0, 2.0, 3.0)));
    assert!(!left.equals(&right));

    // 不同的参考系
    let left2 = TimeIntervalCollectionPositionProperty::new(ReferenceFrame::Fixed);
    let right2 = TimeIntervalCollectionPositionProperty::new(ReferenceFrame::Inertial);
    assert!(!left2.equals(&right2));
}

// ===========================================================================
// CallbackPositionProperty（源自 CallbackPositionPropertySpec）
// ===========================================================================

#[test]
fn callback_position_get_value_works() {
    // "getValue works"
    let value = DVec3::new(1.0, 2.0, 3.0);
    let callback: PositionCallbackFn = Arc::new(move |_time| Some(value));
    let property = CallbackPositionProperty::new(callback, false, ReferenceFrame::Fixed);

    let time = jd(2451545.0, 0.0);
    let result = property.get_value(&time);
    assert_eq!(result, PropertyValue::Cartesian3(value));
}

#[test]
fn callback_position_receives_time() {
    // "getValue passes time to callback"
    let callback: PositionCallbackFn = Arc::new(|time: &JulianDate| {
        Some(DVec3::new(time.day_number as f64, 0.0, 0.0))
    });
    let property = CallbackPositionProperty::new(callback, false, ReferenceFrame::Fixed);

    let time = jd(7.0, 0.0);
    let result = property.get_value(&time);
    assert_eq!(
        result,
        PropertyValue::Cartesian3(DVec3::new(7.0, 0.0, 0.0))
    );
}

#[test]
fn callback_position_is_constant() {
    // "isConstant works"
    let callback: PositionCallbackFn = Arc::new(|_| Some(DVec3::ZERO));
    let property_const =
        CallbackPositionProperty::new(Arc::clone(&callback), true, ReferenceFrame::Fixed);
    let property_var =
        CallbackPositionProperty::new(callback, false, ReferenceFrame::Fixed);

    assert!(property_const.is_constant());
    assert!(!property_var.is_constant());
}

#[test]
fn callback_position_equals() {
    // "equals works"
    let callback: PositionCallbackFn = Arc::new(|_| Some(DVec3::new(1.0, 2.0, 3.0)));
    let left = CallbackPositionProperty::new(
        Arc::clone(&callback),
        false,
        ReferenceFrame::Fixed,
    );
    let right = CallbackPositionProperty::new(
        Arc::clone(&callback),
        false,
        ReferenceFrame::Fixed,
    );
    assert!(left.equals(&right));

    // 不同的 is_constant
    let right2 = CallbackPositionProperty::new(
        Arc::clone(&callback),
        true,
        ReferenceFrame::Fixed,
    );
    assert!(!left.equals(&right2));

    // 不同的参考系
    let right3 = CallbackPositionProperty::new(
        callback,
        false,
        ReferenceFrame::Inertial,
    );
    assert!(!left.equals(&right3));
}

#[test]
fn callback_position_returns_undefined_when_callback_returns_none() {
    // 回调返回 None → Undefined
    let callback: PositionCallbackFn = Arc::new(|_| None);
    let property = CallbackPositionProperty::new(callback, false, ReferenceFrame::Fixed);

    let time = jd(2451545.0, 0.0);
    assert_eq!(property.get_value(&time), PropertyValue::Undefined);
}
