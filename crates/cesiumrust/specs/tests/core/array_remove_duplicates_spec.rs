//! Core/arrayRemoveDuplicatesSpec → Rust 集成测试
//! 25 个原始 it() 块 → 移植 21 个 A 类测试
//!
//! 跳过的 C 类测试：
//! - "returns undefined" - Rust 使用 Option/空切片（编译期安全）
//! - "anonymous types" / "Spherical type" - Rust 是静态类型（DVec3 覆盖了逻辑）
//! - "doesn't modify removedIndices length===1" - 合并进无重复测试

use cesium_geospatial::array_utils::array_remove_duplicates;
use cesium_geospatial::math_utils::EPSILON10;
use glam::DVec3;

/// CesiumJS Cartesian3.equalsEpsilon - 同时检查绝对与相对 epsilon
fn vec3_equals_epsilon(left: &DVec3, right: &DVec3, epsilon: f64) -> bool {
    let dx = (left.x - right.x).abs();
    let dy = (left.y - right.y).abs();
    let dz = (left.z - right.z).abs();
    // CesiumJS equalsEpsilon: abs(l-r) <= epsilon * max(1, abs(l), abs(r))
    let ex = epsilon * left.x.abs().max(right.x.abs()).max(1.0);
    let ey = epsilon * left.y.abs().max(right.y.abs()).max(1.0);
    let ez = epsilon * left.z.abs().max(right.z.abs()).max(1.0);
    dx <= ex && dy <= ey && dz <= ez
}

// ============================================================================
// 无重复
// ============================================================================

#[test]
fn returns_positions_if_none_removed_length_1() {
    let positions = vec![DVec3::ZERO];
    let (result, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert_eq!(result.len(), 1);
    assert!(removed.is_empty());
}

#[test]
fn returns_positions_if_none_removed_length_gt_1() {
    let positions = vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z];
    let (result, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert_eq!(result.len(), 4);
    assert!(removed.is_empty());
}

#[test]
fn wrapping_returns_positions_if_none_removed() {
    let positions = vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z];
    let (result, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, true);
    assert_eq!(result.len(), 4);
    assert!(removed.is_empty());
}

// ============================================================================
// 基本去重
// ============================================================================

#[test]
fn removes_duplicates() {
    let positions = vec![
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(3.0),
        DVec3::splat(3.0),
    ];
    let expected = vec![DVec3::splat(1.0), DVec3::splat(2.0), DVec3::splat(3.0)];
    let (result, _) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert_eq!(result, expected);
}

#[test]
fn doesnt_remove_nonadjacent_duplicates() {
    let positions = vec![
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(1.0),
        DVec3::splat(3.0),
        DVec3::splat(3.0),
    ];
    let expected = vec![
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(1.0),
        DVec3::splat(3.0),
    ];
    let (result, _) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert_eq!(result, expected);
}

#[test]
fn works_with_empty_array() {
    let positions: Vec<DVec3> = vec![];
    let (result, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert!(result.is_empty());
    assert!(removed.is_empty());
}

// ============================================================================
// Epsilon 行为
// ============================================================================

#[test]
fn removes_positions_within_absolute_epsilon10() {
    let positions = vec![
        DVec3::new(1.0, 1.0, 1.0),
        DVec3::new(1.0, 2.0, 3.0),
        DVec3::new(1.0, 2.0, 3.0 + EPSILON10),
    ];
    let expected = vec![DVec3::new(1.0, 1.0, 1.0), DVec3::new(1.0, 2.0, 3.0)];
    let (result, _) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert_eq!(result, expected);
}

#[test]
fn removes_positions_within_relative_epsilon10() {
    let positions = vec![
        DVec3::new(0.0, 0.0, 1000000.0),
        DVec3::new(0.0, 0.0, 3000000.0),
        DVec3::new(0.0, 0.0, 3000000.0002),
    ];
    let expected = vec![
        DVec3::new(0.0, 0.0, 1000000.0),
        DVec3::new(0.0, 0.0, 3000000.0),
    ];
    let (result, _) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert_eq!(result, expected);
}

#[test]
fn keeps_positions_that_add_up_past_relative_epsilon10() {
    let eighty_percent = 0.8 * EPSILON10;
    let positions = vec![
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, 1.0 + eighty_percent),
        DVec3::new(0.0, 0.0, 1.0 + 2.0 * eighty_percent),
        DVec3::new(0.0, 0.0, 1.0 + 3.0 * eighty_percent),
    ];
    // 第一个与第二个在 epsilon 之内 → 移除第二个
    // 第三个与第一个比较（v0 保持在第一个）：2*0.8=1.6 > 1.0 epsilon → 保留
    // 第四个与第三个比较：diff = 0.8*epsilon < epsilon → 移除
    let expected = vec![
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, 1.0 + 2.0 * eighty_percent),
    ];
    let (result, _) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert_eq!(result, expected);
}

// ============================================================================
// 首尾环绕行为
// ============================================================================

#[test]
fn doesnt_remove_first_last_without_wrapping() {
    let positions = vec![
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(3.0),
        DVec3::splat(1.0),
    ];
    let (result, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert_eq!(result.len(), 4);
    assert!(removed.is_empty());
}

#[test]
fn wrapping_removes_duplicate_first_and_last() {
    let positions = vec![
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(3.0),
        DVec3::splat(1.0),
    ];
    let expected = vec![DVec3::splat(1.0), DVec3::splat(2.0), DVec3::splat(3.0)];
    let (result, _) = array_remove_duplicates(&positions, vec3_equals_epsilon, true);
    assert_eq!(result, expected);
}

#[test]
fn wrapping_removes_duplicates_including_first_and_last() {
    let positions = vec![
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(2.0),
        DVec3::splat(3.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
    ];
    let expected = vec![DVec3::splat(1.0), DVec3::splat(2.0), DVec3::splat(3.0)];
    let (result, _) = array_remove_duplicates(&positions, vec3_equals_epsilon, true);
    assert_eq!(result, expected);
}

#[test]
fn wrapping_removes_string_of_duplicates_at_end() {
    let positions = vec![
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(3.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
    ];
    let expected = vec![DVec3::splat(1.0), DVec3::splat(2.0), DVec3::splat(3.0)];
    let (result, _) = array_remove_duplicates(&positions, vec3_equals_epsilon, true);
    assert_eq!(result, expected);
}

#[test]
fn wrapping_doesnt_remove_nonadjacent_duplicates() {
    let positions = vec![
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(1.0),
        DVec3::splat(3.0),
        DVec3::splat(1.0),
    ];
    // 首尾环绕：last(1,1,1)==first(1,1,1) → 移除最后一个
    // 同时检查相邻：无相邻重复
    let expected = vec![
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(1.0),
        DVec3::splat(3.0),
    ];
    let (result, _) = array_remove_duplicates(&positions, vec3_equals_epsilon, true);
    assert_eq!(result, expected);
}

// ============================================================================
// removedIndices 跟踪
// ============================================================================

#[test]
fn removed_indices_empty_when_no_duplicates_length_1() {
    let positions = vec![DVec3::ZERO];
    let (_, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert!(removed.is_empty());
}

#[test]
fn removed_indices_empty_when_no_duplicates_length_gt_1() {
    let positions = vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z];
    let (_, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert!(removed.is_empty());
}

#[test]
fn removed_indices_modified_when_duplicates() {
    let positions = vec![DVec3::ZERO, DVec3::X, DVec3::X, DVec3::Y, DVec3::Z, DVec3::Z];
    let expected = vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z];
    let (result, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert_eq!(result, expected);
    assert_eq!(removed, vec![2, 5]);
}

#[test]
fn removed_indices_empty_without_wrapping_when_first_eq_last() {
    let positions = vec![
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(3.0),
        DVec3::splat(1.0),
    ];
    let (_, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, false);
    assert!(removed.is_empty());
}

#[test]
fn removed_indices_wrapped_when_first_eq_last() {
    let positions = vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z, DVec3::ZERO];
    let expected = vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z];
    let (result, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, true);
    assert_eq!(result, expected);
    assert_eq!(removed, vec![4]);
}

#[test]
fn removed_indices_with_duplicates_and_wrapping() {
    let positions = vec![
        DVec3::ZERO,
        DVec3::ZERO,
        DVec3::X,
        DVec3::Y,
        DVec3::Y,
        DVec3::Z,
        DVec3::Z,
        DVec3::ZERO,
    ];
    let expected = vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z];
    let (result, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, true);
    assert_eq!(result, expected);
    assert_eq!(removed, vec![1, 4, 6, 7]);
}

#[test]
fn wrapping_removed_indices_with_string_of_duplicates() {
    let positions = vec![
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(3.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
    ];
    let expected = vec![DVec3::splat(1.0), DVec3::splat(2.0), DVec3::splat(3.0)];
    let (result, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, true);
    assert_eq!(result, expected);
    assert_eq!(removed, vec![1, 4, 5, 6, 7, 8]);
}

#[test]
fn wrapping_removed_indices_with_multiple_strings() {
    let positions = vec![
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(3.0),
        DVec3::splat(3.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
        DVec3::splat(3.0),
        DVec3::splat(3.0),
        DVec3::splat(1.0),
        DVec3::splat(1.0),
    ];
    let expected = vec![
        DVec3::splat(1.0),
        DVec3::splat(2.0),
        DVec3::splat(3.0),
        DVec3::splat(1.0),
        DVec3::splat(3.0),
    ];
    let (result, removed) = array_remove_duplicates(&positions, vec3_equals_epsilon, true);
    assert_eq!(result, expected);
    assert_eq!(removed, vec![1, 4, 6, 7, 9, 10, 11]);
}
