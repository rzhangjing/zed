//! Intersections2DSpec → Rust integration tests
//!
//! Original: Specs/Core/Intersections2DSpec (23 it())
//! A-class ported: 23 (clipTriangleAtAxisAlignedThreshold ×14, computeBarycentricCoordinates ×4,
//!                     computeLineSegmentLineSegmentIntersection ×5)

use cesium_geospatial::ray::{
    clip_triangle_at_axis_aligned_threshold, compute_barycentric_coordinates,
    compute_line_segment_line_segment_intersection,
};

// === clipTriangleAtAxisAlignedThreshold ===

/// "eliminates a triangle that is entirely on the wrong side of the threshold"
#[test]
fn clip_eliminate_entirely_wrong_side() {
    let result = clip_triangle_at_axis_aligned_threshold(0.1, false, 0.2, 0.3, 0.4);
    assert_eq!(result.len(), 0);
}

/// "keeps a triangle that is entirely on the correct side of the threshold"
#[test]
fn clip_keep_entirely_correct_side() {
    let result = clip_triangle_at_axis_aligned_threshold(0.1, true, 0.2, 0.3, 0.4);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0], 0.0);
    assert_eq!(result[1], 1.0);
    assert_eq!(result[2], 2.0);
}

/// "adds two vertices on threshold when point 0 is on the wrong side and above"
#[test]
fn clip_point0_wrong_side_above() {
    let result = clip_triangle_at_axis_aligned_threshold(0.5, false, 0.6, 0.4, 0.2);
    assert_eq!(result.len(), 10);
    assert_eq!(result[0], 1.0);
    assert_eq!(result[1], 2.0);
    assert_eq!(result[2], -1.0);
    assert_eq!(result[3], 0.0);
    assert_eq!(result[4], 2.0);
    assert!((result[5] - 0.25).abs() < 1e-14);
    assert_eq!(result[6], -1.0);
    assert_eq!(result[7], 0.0);
    assert_eq!(result[8], 1.0);
    assert!((result[9] - 0.5).abs() < 1e-14);
}

/// "adds two vertices on threshold when point 0 is on the wrong side and below"
#[test]
fn clip_point0_wrong_side_below() {
    let result = clip_triangle_at_axis_aligned_threshold(0.5, true, 0.4, 0.6, 0.8);
    assert_eq!(result.len(), 10);
    assert_eq!(result[0], 1.0);
    assert_eq!(result[1], 2.0);
    assert_eq!(result[2], -1.0);
    assert_eq!(result[3], 0.0);
    assert_eq!(result[4], 2.0);
    assert!((result[5] - 0.25).abs() < 1e-14);
    assert_eq!(result[6], -1.0);
    assert_eq!(result[7], 0.0);
    assert_eq!(result[8], 1.0);
    assert!((result[9] - 0.5).abs() < 1e-14);
}

/// "adds two vertices on threshold when point 1 is on the wrong side and above"
#[test]
fn clip_point1_wrong_side_above() {
    let result = clip_triangle_at_axis_aligned_threshold(0.5, false, 0.2, 0.6, 0.4);
    assert_eq!(result.len(), 10);
    assert_eq!(result[0], 2.0);
    assert_eq!(result[1], 0.0);
    assert_eq!(result[2], -1.0);
    assert_eq!(result[3], 1.0);
    assert_eq!(result[4], 0.0);
    assert!((result[5] - 0.25).abs() < 1e-14);
    assert_eq!(result[6], -1.0);
    assert_eq!(result[7], 1.0);
    assert_eq!(result[8], 2.0);
    assert!((result[9] - 0.5).abs() < 1e-14);
}

/// "adds two vertices on threshold when point 1 is on the wrong side and below"
#[test]
fn clip_point1_wrong_side_below() {
    let result = clip_triangle_at_axis_aligned_threshold(0.5, true, 0.8, 0.4, 0.6);
    assert_eq!(result.len(), 10);
    assert_eq!(result[0], 2.0);
    assert_eq!(result[1], 0.0);
    assert_eq!(result[2], -1.0);
    assert_eq!(result[3], 1.0);
    assert_eq!(result[4], 0.0);
    assert!((result[5] - 0.25).abs() < 1e-14);
    assert_eq!(result[6], -1.0);
    assert_eq!(result[7], 1.0);
    assert_eq!(result[8], 2.0);
    assert!((result[9] - 0.5).abs() < 1e-14);
}

/// "adds two vertices on threshold when point 2 is on the wrong side and above"
#[test]
fn clip_point2_wrong_side_above() {
    let result = clip_triangle_at_axis_aligned_threshold(0.5, false, 0.4, 0.2, 0.6);
    assert_eq!(result.len(), 10);
    assert_eq!(result[0], 0.0);
    assert_eq!(result[1], 1.0);
    assert_eq!(result[2], -1.0);
    assert_eq!(result[3], 2.0);
    assert_eq!(result[4], 1.0);
    assert!((result[5] - 0.25).abs() < 1e-14);
    assert_eq!(result[6], -1.0);
    assert_eq!(result[7], 2.0);
    assert_eq!(result[8], 0.0);
    assert!((result[9] - 0.5).abs() < 1e-14);
}

/// "adds two vertices on threshold when point 2 is on the wrong side and below"
#[test]
fn clip_point2_wrong_side_below() {
    let result = clip_triangle_at_axis_aligned_threshold(0.5, true, 0.6, 0.8, 0.4);
    assert_eq!(result.len(), 10);
    assert_eq!(result[0], 0.0);
    assert_eq!(result[1], 1.0);
    assert_eq!(result[2], -1.0);
    assert_eq!(result[3], 2.0);
    assert_eq!(result[4], 1.0);
    assert!((result[5] - 0.25).abs() < 1e-14);
    assert_eq!(result[6], -1.0);
    assert_eq!(result[7], 2.0);
    assert_eq!(result[8], 0.0);
    assert!((result[9] - 0.5).abs() < 1e-14);
}

/// "adds two vertices on threshold when only point 0 is on the right side and below"
#[test]
fn clip_only_point0_right_side_below() {
    let result = clip_triangle_at_axis_aligned_threshold(0.5, false, 0.4, 0.6, 0.8);
    assert_eq!(result.len(), 9);
    assert_eq!(result[0], 0.0);
    assert_eq!(result[1], -1.0);
    assert_eq!(result[2], 1.0);
    assert_eq!(result[3], 0.0);
    assert!((result[4] - 0.5).abs() < 1e-14);
    assert_eq!(result[5], -1.0);
    assert_eq!(result[6], 2.0);
    assert_eq!(result[7], 0.0);
    assert!((result[8] - 0.75).abs() < 1e-14);
}

/// "adds two vertices on threshold when only point 0 is on the right side and above"
#[test]
fn clip_only_point0_right_side_above() {
    let result = clip_triangle_at_axis_aligned_threshold(0.5, true, 0.6, 0.4, 0.2);
    assert_eq!(result.len(), 9);
    assert_eq!(result[0], 0.0);
    assert_eq!(result[1], -1.0);
    assert_eq!(result[2], 1.0);
    assert_eq!(result[3], 0.0);
    assert!((result[4] - 0.5).abs() < 1e-14);
    assert_eq!(result[5], -1.0);
    assert_eq!(result[6], 2.0);
    assert_eq!(result[7], 0.0);
    assert!((result[8] - 0.75).abs() < 1e-14);
}

/// "adds two vertices on threshold when only point 1 is on the right side and below"
#[test]
fn clip_only_point1_right_side_below() {
    let result = clip_triangle_at_axis_aligned_threshold(0.5, false, 0.8, 0.4, 0.6);
    assert_eq!(result.len(), 9);
    assert_eq!(result[0], 1.0);
    assert_eq!(result[1], -1.0);
    assert_eq!(result[2], 2.0);
    assert_eq!(result[3], 1.0);
    assert!((result[4] - 0.5).abs() < 1e-14);
    assert_eq!(result[5], -1.0);
    assert_eq!(result[6], 0.0);
    assert_eq!(result[7], 1.0);
    assert!((result[8] - 0.75).abs() < 1e-14);
}

/// "adds two vertices on threshold when only point 1 is on the right side and above"
#[test]
fn clip_only_point1_right_side_above() {
    let result = clip_triangle_at_axis_aligned_threshold(0.5, true, 0.2, 0.6, 0.4);
    assert_eq!(result.len(), 9);
    assert_eq!(result[0], 1.0);
    assert_eq!(result[1], -1.0);
    assert_eq!(result[2], 2.0);
    assert_eq!(result[3], 1.0);
    assert!((result[4] - 0.5).abs() < 1e-14);
    assert_eq!(result[5], -1.0);
    assert_eq!(result[6], 0.0);
    assert_eq!(result[7], 1.0);
    assert!((result[8] - 0.75).abs() < 1e-14);
}

/// "adds two vertices on threshold when only point 2 is on the right side and below"
#[test]
fn clip_only_point2_right_side_below() {
    let result = clip_triangle_at_axis_aligned_threshold(0.5, false, 0.6, 0.8, 0.4);
    assert_eq!(result.len(), 9);
    assert_eq!(result[0], 2.0);
    assert_eq!(result[1], -1.0);
    assert_eq!(result[2], 0.0);
    assert_eq!(result[3], 2.0);
    assert!((result[4] - 0.5).abs() < 1e-14);
    assert_eq!(result[5], -1.0);
    assert_eq!(result[6], 1.0);
    assert_eq!(result[7], 2.0);
    assert!((result[8] - 0.75).abs() < 1e-14);
}

/// "adds two vertices on threshold when only point 2 is on the right side and above"
#[test]
fn clip_only_point2_right_side_above() {
    let result = clip_triangle_at_axis_aligned_threshold(0.5, true, 0.4, 0.2, 0.6);
    assert_eq!(result.len(), 9);
    assert_eq!(result[0], 2.0);
    assert_eq!(result[1], -1.0);
    assert_eq!(result[2], 0.0);
    assert_eq!(result[3], 2.0);
    assert!((result[4] - 0.5).abs() < 1e-14);
    assert_eq!(result[5], -1.0);
    assert_eq!(result[6], 1.0);
    assert_eq!(result[7], 2.0);
    assert!((result[8] - 0.75).abs() < 1e-14);
}

// === computeBarycentricCoordinates ===

/// "returns the correct result for positions on a triangle vertex"
#[test]
fn barycentric_at_vertices() {
    let (x, y, z) = compute_barycentric_coordinates(0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0);
    assert!((x - 1.0).abs() < 1e-15);
    assert!((y - 0.0).abs() < 1e-15);
    assert!((z - 0.0).abs() < 1e-15);

    let (x, y, z) = compute_barycentric_coordinates(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0);
    assert!((x - 0.0).abs() < 1e-15);
    assert!((y - 1.0).abs() < 1e-15);
    assert!((z - 0.0).abs() < 1e-15);

    let (x, y, z) = compute_barycentric_coordinates(0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0);
    assert!((x - 0.0).abs() < 1e-15);
    assert!((y - 0.0).abs() < 1e-15);
    assert!((z - 1.0).abs() < 1e-15);
}

/// "returns the correct result for a position in the barycenter of a triangle"
#[test]
fn barycentric_at_barycenter() {
    let (x, y, z) = compute_barycentric_coordinates(0.0, 0.0, 0.0, 1.0, -1.0, -0.5, 1.0, -0.5);
    assert!((x - 1.0 / 3.0).abs() < 1e-15);
    assert!((y - 1.0 / 3.0).abs() < 1e-15);
    assert!((z - 1.0 / 3.0).abs() < 1e-15);
}

/// "returns the correct result for a position on an edge between two vertices"
#[test]
fn barycentric_on_edges() {
    let (x, y, z) = compute_barycentric_coordinates(1.5, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 2.0);
    assert!((x - 0.5).abs() < 1e-15);
    assert!((y - 0.5).abs() < 1e-15);
    assert!((z - 0.0).abs() < 1e-15);

    let (x, y, z) = compute_barycentric_coordinates(1.5, 1.5, 1.0, 1.0, 2.0, 1.0, 1.0, 2.0);
    assert!((x - 0.0).abs() < 1e-15);
    assert!((y - 0.5).abs() < 1e-15);
    assert!((z - 0.5).abs() < 1e-15);

    let (x, y, z) = compute_barycentric_coordinates(1.0, 1.5, 1.0, 1.0, 2.0, 1.0, 1.0, 2.0);
    assert!((x - 0.5).abs() < 1e-15);
    assert!((y - 0.0).abs() < 1e-15);
    assert!((z - 0.5).abs() < 1e-15);
}

/// "returns the correct result for a position outside a triangle"
#[test]
fn barycentric_outside() {
    let (x, y, z) = compute_barycentric_coordinates(0.5, 0.5, 1.0, 1.0, 2.0, 1.0, 1.0, 2.0);
    assert!(x > 0.0);
    assert!(y < 0.0);
    assert!(z < 0.0);

    let (x, y, z) = compute_barycentric_coordinates(2.1, 0.99, 1.0, 1.0, 2.0, 1.0, 1.0, 2.0);
    assert!(x < 0.0);
    assert!(y > 0.0);
    assert!(z < 0.0);

    let (x, y, z) = compute_barycentric_coordinates(0.99, 2.1, 1.0, 1.0, 2.0, 1.0, 1.0, 2.0);
    assert!(x < 0.0);
    assert!(y < 0.0);
    assert!(z > 0.0);
}

// === computeLineSegmentLineSegmentIntersection ===

/// "returns the correct result for intersection point"
#[test]
fn line_segment_intersection_point() {
    let (x, y) =
        compute_line_segment_line_segment_intersection(0.0, 0.0, 0.0, 2.0, -1.0, 1.0, 1.0, 1.0)
            .unwrap();
    assert!((x - 0.0).abs() < 1e-15);
    assert!((y - 1.0).abs() < 1e-15);

    let (x, y) = compute_line_segment_line_segment_intersection(
        0.0, 0.0, 10.0, 5.0, 0.0, 5.0, 10.0, 0.0,
    )
    .unwrap();
    assert!((x - 5.0).abs() < 1e-15);
    assert!((y - 2.5).abs() < 1e-15);

    let (x, y) = compute_line_segment_line_segment_intersection(
        0.0, -5.0, 4.0, 3.0, -2.0, 1.0, 4.0, -2.0,
    )
    .unwrap();
    assert!((x - 2.0).abs() < 1e-15);
    assert!((y - (-1.0)).abs() < 1e-15);
}

/// "returns the correct result for intersection point on a vertex"
#[test]
fn line_segment_intersection_on_vertex() {
    let (x, y) =
        compute_line_segment_line_segment_intersection(0.0, 0.0, 0.0, 2.0, -1.0, 0.0, 1.0, 0.0)
            .unwrap();
    assert!((x - 0.0).abs() < 1e-15);
    assert!((y - 0.0).abs() < 1e-15);

    let (x, y) =
        compute_line_segment_line_segment_intersection(0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 2.0, 0.0)
            .unwrap();
    assert!((x - 1.0).abs() < 1e-15);
    assert!((y - 1.0).abs() < 1e-15);

    let (x, y) =
        compute_line_segment_line_segment_intersection(0.0, 0.0, 4.0, 3.0, 5.0, 0.0, 4.0, 3.0)
            .unwrap();
    assert!((x - 4.0).abs() < 1e-15);
    assert!((y - 3.0).abs() < 1e-15);
}

/// "returns undefined for non-intersecting lines"
#[test]
fn line_segment_no_intersection() {
    assert!(compute_line_segment_line_segment_intersection(
        0.0, 0.0, 0.0, 5.0, 0.1, 4.8, 5.0, 0.0
    )
    .is_none());

    assert!(compute_line_segment_line_segment_intersection(
        10.0, 0.0, 0.0, -10.0, 0.0, 0.0, -8.0, -8.0
    )
    .is_none());
}

/// "returns undefined for parallel lines"
#[test]
fn line_segment_parallel() {
    assert!(compute_line_segment_line_segment_intersection(
        0.0, 0.0, 0.0, 2.0, 1.0, 1.0, 1.0, 4.0
    )
    .is_none());

    assert!(compute_line_segment_line_segment_intersection(
        1.0, 1.0, 4.0, 4.0, 0.0, 0.0, 3.0, 3.0
    )
    .is_none());
}

/// "returns undefined for coincident lines"
#[test]
fn line_segment_coincident() {
    assert!(compute_line_segment_line_segment_intersection(
        0.0, 0.0, 0.0, 2.0, 0.0, 1.0, 0.0, 4.0
    )
    .is_none());

    assert!(compute_line_segment_line_segment_intersection(
        0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0
    )
    .is_none());
}
