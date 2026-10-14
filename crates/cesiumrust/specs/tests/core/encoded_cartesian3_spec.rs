//! 参考自 `Specs/Core/EncodedCartesian3Spec`（13 个 it()，5 个 A 类）
//!
//! 省略 8 个 throws 测试（C 类：Rust 类型系统强制输入合法）。

use cesium_geospatial::encoded_cartesian3::*;
use glam::DVec3;

#[test]
fn construct_with_default_values() {
    let encoded = EncodedCartesian3::default();
    assert_eq!(encoded.high, DVec3::ZERO);
    assert_eq!(encoded.low, DVec3::ZERO);
}

#[test]
fn encode_encodes_a_negative_value() {
    // 原始规格标题写着 "positive"，但传入的是 -10000000.0
    let (high, low) = encode(-10000000.0);
    assert_eq!(high + low, -10000000.0);
}

#[test]
fn encode_encodes_a_positive_value() {
    // 原始规格标题写着 "negative"，但传入的是 10000000.0
    let (high, low) = encode(10000000.0);
    assert_eq!(high + low, 10000000.0);
}

#[test]
fn from_cartesian_encodes_a_cartesian() {
    let c = DVec3::new(-10000000.0, 0.0, 10000000.0);
    let encoded = from_cartesian(c);

    // "Look mom, no epsilon check."
    assert_eq!(encoded.high.x + encoded.low.x, -10000000.0);
    assert_eq!(encoded.high.y + encoded.low.y, 0.0);
    assert_eq!(encoded.high.z + encoded.low.z, 10000000.0);
}

#[test]
fn write_elements_encodes_a_cartesian() {
    let c = DVec3::new(-10000000.0, 0.0, 10000000.0);
    let encoded = from_cartesian(c);

    let mut positions = [0.0f64; 6];
    write_elements(c, &mut positions, 0);

    assert_eq!(encoded.high.x, positions[0]);
    assert_eq!(encoded.high.y, positions[1]);
    assert_eq!(encoded.high.z, positions[2]);
    assert_eq!(encoded.low.x, positions[3]);
    assert_eq!(encoded.low.y, positions[4]);
    assert_eq!(encoded.low.z, positions[5]);
}
