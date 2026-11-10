//! DataSources/VelocityVectorPropertySpec → Rust 集成测试
//! 覆盖：构造、isConstant、getValue（归一化/非归一化）、
//! equals、position 变化

use cesium_datasource::property_system::position::SampledPositionProperty;
use cesium_datasource::property_system::value::ReferenceFrame;
use cesium_datasource::velocity_vector_property::VelocityVectorProperty;
use cesium_time::JulianDate;
use glam::DVec3;
use std::sync::Arc;

// ─── 构造 ───────────────────────────────────────────────────────────

#[test]
fn vvp_default_construct() {
    let property = VelocityVectorProperty::new();
    assert!(property.is_constant());
    assert!(property.position().is_none());
    assert!(property.normalize());
    assert!(property.get_value(&JulianDate::new(2451545.0, 0.0)).is_none());
}

#[test]
fn vvp_construct_with_arguments() {
    let position = Arc::new(SampledPositionProperty::new(ReferenceFrame::Fixed, 0));
    let property = VelocityVectorProperty::with_position(
        position.clone() as Arc<dyn cesium_datasource::property_system::property::DynProperty>,
        false,
    );

    assert!(property.is_constant());
    assert!(property.position().is_some());
    assert!(!property.normalize());
}

// ─── getValue 归一化 ────────────────────────────────────────────────────

#[test]
fn vvp_normalized_value() {
    let mut position = SampledPositionProperty::new(ReferenceFrame::Fixed, 0);
    let times = [
        JulianDate::new(2451545.0, 0.0),
        JulianDate::new(2451545.0, 1.0 / 60.0),
    ];
    let values = [
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(20.0, 0.0, 0.0),
    ];
    position.add_samples(&times, &values, None);

    let property = VelocityVectorProperty::with_position(
        Arc::new(position) as Arc<dyn cesium_datasource::property_system::property::DynProperty>,
        true,
    );

    let expected_direction = DVec3::new(1.0, 0.0, 0.0);
    let result = property.get_value(&times[0]);
    assert!(result.is_some());
    let v = result.unwrap();
    assert!((v.x - expected_direction.x).abs() < 1e-10);
    assert!((v.y - expected_direction.y).abs() < 1e-10);
    assert!((v.z - expected_direction.z).abs() < 1e-10);
}

// ─── getValue 非归一化 ──────────────────────────────────────────────────

#[test]
fn vvp_unnormalized_value() {
    let mut position = SampledPositionProperty::new(ReferenceFrame::Fixed, 0);
    let times = [
        JulianDate::new(2451545.0, 0.0),
        JulianDate::new(2451545.0, 1.0),
    ];
    let values = [
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(20.0, 0.0, 0.0),
    ];
    position.add_samples(&times, &values, None);

    let property = VelocityVectorProperty::with_position(
        Arc::new(position) as Arc<dyn cesium_datasource::property_system::property::DynProperty>,
        false,
    );

    let expected_velocity = DVec3::new(20.0, 0.0, 0.0);
    let result = property.get_value(&times[0]);
    assert!(result.is_some());
    let v = result.unwrap();
    assert!((v.x - expected_velocity.x).abs() < 1e-10);
    assert!((v.y - expected_velocity.y).abs() < 1e-10);
    assert!((v.z - expected_velocity.z).abs() < 1e-10);
}

// ─── 零速度 ──────────────────────────────────────────────────────────

#[test]
fn vvp_normalized_zero_velocity_returns_none() {
    let mut position = SampledPositionProperty::new(ReferenceFrame::Fixed, 0);
    let times = [
        JulianDate::new(2451545.0, 0.0),
        JulianDate::new(2451545.0, 1.0),
    ];
    let values = [
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 0.0),
    ];
    position.add_samples(&times, &values, None);

    let property = VelocityVectorProperty::with_position(
        Arc::new(position) as Arc<dyn cesium_datasource::property_system::property::DynProperty>,
        true,
    );

    // normalize=true 的零速度应返回 None
    let result = property.get_value(&times[0]);
    assert!(result.is_none());
}

#[test]
fn vvp_unnormalized_zero_velocity_returns_zero() {
    let mut position = SampledPositionProperty::new(ReferenceFrame::Fixed, 0);
    let times = [
        JulianDate::new(2451545.0, 0.0),
        JulianDate::new(2451545.0, 1.0),
    ];
    let values = [
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 0.0),
    ];
    position.add_samples(&times, &values, None);

    let property = VelocityVectorProperty::with_position(
        Arc::new(position) as Arc<dyn cesium_datasource::property_system::property::DynProperty>,
        false,
    );

    // normalize=false 的零速度应返回零向量
    let result = property.get_value(&times[0]);
    assert!(result.is_some());
    let v = result.unwrap();
    assert!((v.x).abs() < 1e-10);
    assert!((v.y).abs() < 1e-10);
    assert!((v.z).abs() < 1e-10);
}

// ─── 无 position ────────────────────────────────────────────────────────────

#[test]
fn vvp_no_position_returns_none() {
    let property = VelocityVectorProperty::new();
    let result = property.get_value(&JulianDate::new(2451545.0, 0.0));
    assert!(result.is_none());
}

// ─── equals ─────────────────────────────────────────────────────────────────

#[test]
fn vvp_equals_both_empty() {
    let left = VelocityVectorProperty::new();
    let right = VelocityVectorProperty::new();
    assert!(left.equals(&right));
}

#[test]
fn vvp_equals_different_position() {
    let position = Arc::new(SampledPositionProperty::new(ReferenceFrame::Fixed, 0));
    let mut left = VelocityVectorProperty::new();
    let right = VelocityVectorProperty::new();

    left.set_position(Some(position as Arc<dyn cesium_datasource::property_system::property::DynProperty>));
    assert!(!left.equals(&right));
}

#[test]
fn vvp_equals_same_position() {
    let position = Arc::new(SampledPositionProperty::new(ReferenceFrame::Fixed, 0));
    let left = VelocityVectorProperty::with_position(
        position.clone() as Arc<dyn cesium_datasource::property_system::property::DynProperty>,
        true,
    );
    let right = VelocityVectorProperty::with_position(
        position as Arc<dyn cesium_datasource::property_system::property::DynProperty>,
        true,
    );
    assert!(left.equals(&right));
}

// ─── set_normalize ──────────────────────────────────────────────────────────

#[test]
fn vvp_set_normalize() {
    let mut property = VelocityVectorProperty::new();
    assert!(property.normalize());

    property.set_normalize(false);
    assert!(!property.normalize());

    property.set_normalize(true);
    assert!(property.normalize());
}

// ─── 3D 速度方向 ──────────────────────────────────────────────────

#[test]
fn vvp_3d_normalized_direction() {
    let mut position = SampledPositionProperty::new(ReferenceFrame::Fixed, 0);
    let times = [
        JulianDate::new(2451545.0, 0.0),
        JulianDate::new(2451545.0, 1.0 / 60.0),
    ];
    let values = [
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(1.0, 1.0, 1.0),
    ];
    position.add_samples(&times, &values, None);

    let property = VelocityVectorProperty::with_position(
        Arc::new(position) as Arc<dyn cesium_datasource::property_system::property::DynProperty>,
        true,
    );

    let result = property.get_value(&times[0]);
    assert!(result.is_some());
    let v = result.unwrap();

    // 应被归一化为 (1,1,1)/sqrt(3)
    let expected = 1.0 / 3.0_f64.sqrt();
    assert!((v.x - expected).abs() < 1e-10);
    assert!((v.y - expected).abs() < 1e-10);
    assert!((v.z - expected).abs() < 1e-10);

    // 验证其为单位长度
    assert!((v.length() - 1.0).abs() < 1e-10);
}
