//! Core/TridiagonalSystemSolverSpec + Core/NearFarScalarSpec → Rust 集成测试
//!
//! TridiagonalSystemSolver：9 个 it() → 2 个 A 类（7 个 C 类：throws）
//! NearFarScalar：5 个 it() + createPackableSpecs → 4 个 A 类（1 个 C 类：result 参数）

use cesium_animation::tridiagonal_solve;
use cesium_datasource::primitives::NearFarScalar;
use glam::DVec3;

// === TridiagonalSystemSolver ===

#[test]
fn test_tridiagonal_solve_three_unknowns() {
    let l = [1.0, 1.0];
    let d = [-2.175, -2.15, -2.125];
    let u = [1.0, 1.0];
    let r = [
        DVec3::new(-1.625, -1.625, -1.625),
        DVec3::new(0.5, 0.5, 0.5),
        DVec3::new(1.625, 1.625, 1.625),
    ];

    let expected = [
        DVec3::new(0.552, 0.552, 0.552),
        DVec3::new(-0.4244, -0.4244, -0.4244),
        DVec3::new(-0.9644, -0.9644, -0.9644),
    ];

    let actual = tridiagonal_solve(&l, &d, &u, &r);
    assert_eq!(actual.len(), 3);
    for i in 0..3 {
        assert!((actual[i].x - expected[i].x).abs() < 1e-4, "x[{}]: {} vs {}", i, actual[i].x, expected[i].x);
        assert!((actual[i].y - expected[i].y).abs() < 1e-4, "y[{}]: {} vs {}", i, actual[i].y, expected[i].y);
        assert!((actual[i].z - expected[i].z).abs() < 1e-4, "z[{}]: {} vs {}", i, actual[i].z, expected[i].z);
    }
}

#[test]
fn test_tridiagonal_solve_nine_unknowns() {
    let l = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let d = [
        -2.0304, -2.0288, -2.0272, -2.0256, -2.024, -2.0224, -2.0208, -2.0192, -2.0176,
    ];
    let u = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let r = [
        DVec3::new(-1.952, -1.952, -1.952),
        DVec3::new(0.056, 0.056, 0.056),
        DVec3::new(0.064, 0.064, 0.064),
        DVec3::new(0.072, 0.072, 0.072),
        DVec3::new(0.08, 0.08, 0.08),
        DVec3::new(0.088, 0.088, 0.088),
        DVec3::new(0.096, 0.096, 0.096),
        DVec3::new(0.104, 0.104, 0.104),
        DVec3::new(1.112, 1.112, 1.112),
    ];

    let expected = [
        DVec3::new(1.3513, 1.3513, 1.3513),
        DVec3::new(0.7918, 0.7918, 0.7918),
        DVec3::new(0.311, 0.311, 0.311),
        DVec3::new(-0.0974, -0.0974, -0.0974),
        DVec3::new(-0.4362, -0.4362, -0.4362),
        DVec3::new(-0.7055, -0.7055, -0.7055),
        DVec3::new(-0.9025, -0.9025, -0.9025),
        DVec3::new(-1.0224, -1.0224, -1.0224),
        DVec3::new(-1.0579, -1.0579, -1.0579),
    ];

    let actual = tridiagonal_solve(&l, &d, &u, &r);
    assert_eq!(actual.len(), 9);
    for i in 0..9 {
        assert!((actual[i].x - expected[i].x).abs() < 1e-4, "x[{}]: {} vs {}", i, actual[i].x, expected[i].x);
    }
}

// === NearFarScalar ===

#[test]
fn test_near_far_scalar_default() {
    let scalar = NearFarScalar::default();
    assert_eq!(scalar.near, 0.0);
    assert_eq!(scalar.near_value, 0.0);
    assert_eq!(scalar.far, 1.0);
    assert_eq!(scalar.far_value, 0.0);
}

#[test]
fn test_near_far_scalar_with_args() {
    let scalar = NearFarScalar::new(1.0, 1.0, 1.0e6, 0.5);
    assert_eq!(scalar.near, 1.0);
    assert_eq!(scalar.near_value, 1.0);
    assert_eq!(scalar.far, 1.0e6);
    assert_eq!(scalar.far_value, 0.5);
}

#[test]
fn test_near_far_scalar_clone() {
    let scalar = NearFarScalar::new(1.0, 2.0, 3.0, 4.0);
    let cloned = scalar; // Rust 中的拷贝语义
    assert_eq!(scalar, cloned);
}

#[test]
fn test_near_far_scalar_pack_unpack() {
    let scalar = NearFarScalar::new(1.0, 2.0, 3.0, 4.0);
    let packed = [scalar.near, scalar.near_value, scalar.far, scalar.far_value];
    assert_eq!(packed, [1.0, 2.0, 3.0, 4.0]);

    let unpacked = NearFarScalar::new(packed[0], packed[1], packed[2], packed[3]);
    assert_eq!(unpacked, scalar);
}
