//! VelocityOrientationProperty 测试 - 参考自 VelocityOrientationPropertySpec
//!
//! 原始：14 it() → 7 A 类（7 C 类：events/spy/system-time 已省略）

use cesium_datasource::property_system::position::SampledPositionProperty;
use cesium_datasource::property_system::property::ConstantProperty;
use cesium_datasource::property_system::value::{PropertyValue, ReferenceFrame};
use cesium_datasource::velocity_orientation_property::VelocityOrientationProperty;
use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::transforms::rotation_matrix_from_position_velocity;
use cesium_time::JulianDate;
use glam::{DQuat, DVec3};
use std::sync::Arc;

fn jd(seconds: f64) -> JulianDate {
    JulianDate::new(0.0, seconds)
}

fn from_degrees(lon_deg: f64, lat_deg: f64, height: f64) -> DVec3 {
    Ellipsoid::WGS84.cartographic_to_cartesian(&Cartographic::from_degrees(lon_deg, lat_deg, height))
}

// === 构造 ===

#[test]
fn test_velocity_orientation_default_construct() {
    let property = VelocityOrientationProperty::new();
    assert!(property.is_constant());
    assert!(property.position().is_none());
    assert_eq!(*property.ellipsoid(), Ellipsoid::WGS84);
}

#[test]
fn test_velocity_orientation_construct_with_args() {
    let position = Arc::new(ConstantProperty::new(PropertyValue::Cartesian3(
        DVec3::X,
    )));
    let property =
        VelocityOrientationProperty::with_position(position.clone(), Ellipsoid::UNIT_SPHERE);
    assert!(property.position().is_some());
    assert_eq!(*property.ellipsoid(), Ellipsoid::UNIT_SPHERE);
}

// === getValue ===

#[test]
fn test_velocity_orientation_get_value() {
    // 位置沿赤道向东移动
    let times = vec![jd(0.0), jd(1.0 / 60.0)];
    let values = vec![from_degrees(0.0, 0.0, 0.0), from_degrees(1.0, 0.0, 0.0)];

    let velocity = (values[1] - values[0]).normalize();

    let mut position = SampledPositionProperty::new(ReferenceFrame::Fixed, 0);
    position.add_samples(&times, &values, None);

    let property =
        VelocityOrientationProperty::with_position(Arc::new(position), Ellipsoid::WGS84);

    let pos_at_t0 = from_degrees(0.0, 0.0, 0.0);
    let matrix = rotation_matrix_from_position_velocity(pos_at_t0, velocity, &Ellipsoid::WGS84);
    let expected = DQuat::from_mat3(&matrix);

    let result = property.get_value(&times[0]);
    assert!(result.is_some());
    let q = result.unwrap();
    // 四元数可能符号不同（q 和 -q 表示相同旋转）
    let dot = q.dot(expected).abs();
    assert!(
        (dot - 1.0).abs() < 1e-10,
        "quaternion mismatch: dot={}",
        dot
    );
}

// === 零速度 ===

#[test]
fn test_velocity_orientation_zero_velocity() {
    // 常量位置 → 零速度 → undefined
    let position = Arc::new(ConstantProperty::new(PropertyValue::Cartesian3(
        from_degrees(0.0, 0.0, 0.0),
    )));
    let property = VelocityOrientationProperty::with_position(position, Ellipsoid::WGS84);
    let result = property.get_value(&jd(0.0));
    assert!(result.is_none());
}

// === 未定义位置 ===

#[test]
fn test_velocity_orientation_undefined_position() {
    // 无 position 属性 → undefined
    let property = VelocityOrientationProperty::new();
    let result = property.get_value(&jd(0.0));
    assert!(result.is_none());
}

// === 单个样本（无法计算速度）===

#[test]
fn test_velocity_orientation_single_sample() {
    // 外推为 NONE 时，在单个样本外查询返回 undefined
    let mut position = SampledPositionProperty::new(ReferenceFrame::Fixed, 0);
    position.add_samples(&[jd(1.0)], &[from_degrees(0.0, 0.0, 0.0)], None);
    // 在 time 0（样本之前）查询 - 默认外推下仍可能生效
    // 但速度将为零，因为两次求值返回相同的值
    let property =
        VelocityOrientationProperty::with_position(Arc::new(position), Ellipsoid::WGS84);
    // 在精确样本时间，前向差分给出相同值 → 零速度
    let result = property.get_value(&jd(1.0));
    // 线性外推下，t 和 t+dt 都求值为同一常量 → 零速度
    assert!(result.is_none());
}

// === equals ===

#[test]
fn test_velocity_orientation_equals() {
    let position = Arc::new(ConstantProperty::new(PropertyValue::Cartesian3(DVec3::X)));

    let left = VelocityOrientationProperty::new();
    let right = VelocityOrientationProperty::new();
    assert!(left.equals(&right));

    let mut left2 = VelocityOrientationProperty::with_position(position.clone(), Ellipsoid::WGS84);
    assert!(!left2.equals(&right));

    let right2 = VelocityOrientationProperty::with_position(position.clone(), Ellipsoid::WGS84);
    assert!(left2.equals(&right2));

    // 不同的椭球
    left2.set_ellipsoid(Ellipsoid::UNIT_SPHERE);
    assert!(!left2.equals(&right2));
}
