//! Core/DistanceDisplayConditionSpec → Rust integration tests
//! 11 original it() blocks + createPackableSpecs → 12 A-class tests ported
//!
//! Omitted C-class tests (JS result-parameter / reference-identity patterns):
//! - static clone with a result parameter (JS result-param)
//! - static clone works with result parameter that is input parameter (JS reference identity)
//! - clone with a result parameter (JS result-param)
//! - clone works with result parameter that is input parameter (JS reference identity)
//! - equals with undefined (JS undefined handling; Rust uses Option or direct comparison)

use cesium_datasource::primitives::DistanceDisplayCondition;

// ============================================================================
// Construction
// ============================================================================

#[test]
fn default_constructs() {
    let dc = DistanceDisplayCondition::default();
    assert_eq!(dc.near, 0.0);
    assert_eq!(dc.far, f64::MAX);
}

#[test]
fn constructs_with_parameters() {
    let near = 10.0;
    let far = 100.0;
    let dc = DistanceDisplayCondition::new(near, far);
    assert_eq!(dc.near, near);
    assert_eq!(dc.far, far);
}

#[test]
fn gets_and_sets_properties() {
    let mut dc = DistanceDisplayCondition::default();
    let near = 10.0;
    let far = 100.0;
    dc.near = near;
    dc.far = far;
    assert_eq!(dc.near, near);
    assert_eq!(dc.far, far);
}

// ============================================================================
// Equality
// ============================================================================

#[test]
fn determines_equality_with_static_function() {
    let dc = DistanceDisplayCondition::new(10.0, 100.0);
    assert!(dc.equals(&DistanceDisplayCondition::new(10.0, 100.0)));
    assert!(!dc.equals(&DistanceDisplayCondition::new(11.0, 100.0)));
    assert!(!dc.equals(&DistanceDisplayCondition::new(10.0, 101.0)));
}

#[test]
fn determines_equality_with_partial_eq() {
    // 映射至 "determines equality with prototype function"
    let dc = DistanceDisplayCondition::new(10.0, 100.0);
    assert_eq!(dc, DistanceDisplayCondition::new(10.0, 100.0));
    assert_ne!(dc, DistanceDisplayCondition::new(11.0, 100.0));
    assert_ne!(dc, DistanceDisplayCondition::new(10.0, 101.0));
}

// ============================================================================
// Clone
// ============================================================================

#[test]
fn clones() {
    let dc = DistanceDisplayCondition::new(10.0, 100.0);
    let result = dc; // 拷贝语义 = clone
    assert_eq!(dc, result);
}

#[test]
fn clone_is_independent() {
    // 映射至 "static clones" —— 验证克隆值相等但相互独立
    let dc = DistanceDisplayCondition::new(10.0, 100.0);
    let mut result = dc;
    result.near = 999.0;
    assert_ne!(dc.near, result.near); // 原始值不变
    assert_eq!(dc.near, 10.0);
}

// ============================================================================
// Pack / Unpack (createPackableSpecs)
// ============================================================================

#[test]
fn packed_length_is_correct() {
    assert_eq!(DistanceDisplayCondition::PACKED_LENGTH, 2);
}

#[test]
fn pack_stores_into_array() {
    let dc = DistanceDisplayCondition::new(1.0, 2.0);
    let mut array = vec![0.0; 2];
    dc.pack(&mut array, 0);
    assert_eq!(array, [1.0, 2.0]);
}

#[test]
fn pack_with_starting_index() {
    let dc = DistanceDisplayCondition::new(1.0, 2.0);
    let mut array = vec![0.0; 4];
    dc.pack(&mut array, 2);
    assert_eq!(array, [0.0, 0.0, 1.0, 2.0]);
}

#[test]
fn unpack_retrieves_from_array() {
    let array = [1.0, 2.0];
    let dc = DistanceDisplayCondition::unpack(&array, 0);
    assert_eq!(dc.near, 1.0);
    assert_eq!(dc.far, 2.0);
}

#[test]
fn unpack_with_starting_index() {
    let array = [9.0, 9.0, 1.0, 2.0];
    let dc = DistanceDisplayCondition::unpack(&array, 2);
    assert_eq!(dc.near, 1.0);
    assert_eq!(dc.far, 2.0);
}
