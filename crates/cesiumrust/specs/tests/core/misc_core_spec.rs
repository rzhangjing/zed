//! 杂项 Core 规格测试 - 参考自：
//! - Specs/Core/isLeapYearSpec（4 个 it()，1 个 A 类）
//! - Specs/Core/IntervalSpec（2 个 it()，2 个 A 类）
//! - Specs/Core/NearFarScalarSpec（5 个 it()，2 个 A 类）
//! - Specs/Core/VertexFormatSpec（2 个 it()，1 个 A 类）
//! - Specs/Core/TridiagonalSystemSolverSpec（9 个 it()，2 个 A 类）
//!
//! A 类测试总计：8 个

use cesium_animation::tridiagonal_solve;
use cesium_datasource::primitives::NearFarScalar;
use cesium_geospatial::bounding::Interval;
use cesium_geospatial::geometry::VertexFormat;
use cesium_time::is_leap_year;
use glam::DVec3;

// ============================================================
// isLeapYear
// ============================================================

#[test]
fn is_leap_year_check_valid_leap_years() {
    assert!(is_leap_year(2000));
    assert!(is_leap_year(2004));
    assert!(!is_leap_year(2003));
    assert!(!is_leap_year(2300));
    assert!(is_leap_year(2400));
    assert!(!is_leap_year(-1));
    assert!(is_leap_year(-2000));
}

// ============================================================
// Interval
// ============================================================

#[test]
fn interval_constructs_without_arguments() {
    let interval = Interval::default();
    assert_eq!(interval.start, 0.0);
    assert_eq!(interval.stop, 0.0);
}

#[test]
fn interval_constructs_with_arguments() {
    let interval = Interval::new(1.0, 2.0);
    assert_eq!(interval.start, 1.0);
    assert_eq!(interval.stop, 2.0);
}

// ============================================================
// NearFarScalar
// ============================================================

#[test]
fn near_far_scalar_constructs_without_arguments() {
    let scalar = NearFarScalar::default();
    assert_eq!(scalar.near, 0.0);
    assert_eq!(scalar.near_value, 0.0);
    assert_eq!(scalar.far, 1.0);
    assert_eq!(scalar.far_value, 0.0);
}

#[test]
fn near_far_scalar_constructs_with_arguments() {
    let scalar = NearFarScalar::new(1.0, 1.0, 1.0e6, 0.5);
    assert_eq!(scalar.near, 1.0);
    assert_eq!(scalar.near_value, 1.0);
    assert_eq!(scalar.far, 1.0e6);
    assert_eq!(scalar.far_value, 0.5);
}

// ============================================================
// VertexFormat
// ============================================================

#[test]
fn vertex_format_clone() {
    let vf = VertexFormat {
        position: true,
        normal: true,
        st: false,
        tangent: false,
        bitangent: false,
    };
    let cloned = vf; // VertexFormat 是 Copy
    assert_eq!(cloned.position, vf.position);
    assert_eq!(cloned.normal, vf.normal);
    assert_eq!(cloned.st, vf.st);
    assert_eq!(cloned.tangent, vf.tangent);
    assert_eq!(cloned.bitangent, vf.bitangent);
}

// ============================================================
// TridiagonalSystemSolver
// ============================================================

#[test]
fn tridiagonal_solve_three_unknowns() {
    let l = [1.0, 1.0];
    let d = [-2.175, -2.15, -2.125];
    let u = [1.0, 1.0];
    let r = [
        DVec3::new(-1.625, 0.0, 0.0),
        DVec3::new(0.5, 0.0, 0.0),
        DVec3::new(1.625, 0.0, 0.0),
    ];

    let expected = [
        DVec3::new(0.552, 0.0, 0.0),
        DVec3::new(-0.4244, 0.0, 0.0),
        DVec3::new(-0.9644, 0.0, 0.0),
    ];

    let actual = tridiagonal_solve(&l, &d, &u, &r);

    assert_eq!(actual.len(), expected.len());
    for i in 0..3 {
        assert!(
            (actual[i].x - expected[i].x).abs() < 1e-4,
            "actual[{}].x = {}, expected {}",
            i, actual[i].x, expected[i].x
        );
    }
}

#[test]
fn tridiagonal_solve_nine_unknowns() {
    let l = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let d = [
        -2.0304, -2.0288, -2.0272, -2.0256, -2.024, -2.0224, -2.0208, -2.0192, -2.0176,
    ];
    let u = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let r = [
        DVec3::new(-1.952, 0.0, 0.0),
        DVec3::new(0.056, 0.0, 0.0),
        DVec3::new(0.064, 0.0, 0.0),
        DVec3::new(0.072, 0.0, 0.0),
        DVec3::new(0.08, 0.0, 0.0),
        DVec3::new(0.088, 0.0, 0.0),
        DVec3::new(0.096, 0.0, 0.0),
        DVec3::new(0.104, 0.0, 0.0),
        DVec3::new(1.112, 0.0, 0.0),
    ];

    let expected: [f64; 9] = [
        1.3513, 0.7918, 0.311, -0.0974, -0.4362, -0.7055, -0.9025, -1.0224, -1.0579,
    ];

    let actual = tridiagonal_solve(&l, &d, &u, &r);

    assert_eq!(actual.len(), 9);
    for i in 0..9 {
        assert!(
            (actual[i].x - expected[i]).abs() < 1e-4,
            "actual[{}].x = {}, expected {}",
            i, actual[i].x, expected[i]
        );
    }
}
