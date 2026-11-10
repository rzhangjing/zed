//! 插值算法规范 - 参考自 Core/LinearApproximationSpec、
//! Core/LagrangePolynomialApproximationSpec、Core/HermitePolynomialApproximationSpec
//!
//! 测试 SampledProperty 使用的底层插值算法。

use cesium_datasource::property_system::{
    ExtrapolationType, HermitePolynomialApproximation, InterpolationAlgorithm,
    InterpolationAlgorithmKind, LagrangePolynomialApproximation, LinearApproximation,
};

// ─── ExtrapolationType ─────────────────────────────────────────────────────

#[test]
fn extrapolation_type_to_u32() {
    assert_eq!(ExtrapolationType::None.to_u32(), 0);
    assert_eq!(ExtrapolationType::Hold.to_u32(), 1);
    assert_eq!(ExtrapolationType::Extrapolate.to_u32(), 2);
}

#[test]
fn extrapolation_type_from_u32() {
    assert_eq!(ExtrapolationType::from_u32(0), ExtrapolationType::None);
    assert_eq!(ExtrapolationType::from_u32(1), ExtrapolationType::Hold);
    assert_eq!(ExtrapolationType::from_u32(2), ExtrapolationType::Extrapolate);
    assert_eq!(ExtrapolationType::from_u32(99), ExtrapolationType::None);
}

#[test]
fn extrapolation_type_default() {
    assert_eq!(ExtrapolationType::default(), ExtrapolationType::None);
}

// ─── InterpolationAlgorithmKind ────────────────────────────────────────────

#[test]
fn algorithm_kind_from_name() {
    assert_eq!(
        InterpolationAlgorithmKind::from_name("Linear"),
        Some(InterpolationAlgorithmKind::Linear)
    );
    assert_eq!(
        InterpolationAlgorithmKind::from_name("Lagrange"),
        Some(InterpolationAlgorithmKind::Lagrange)
    );
    assert_eq!(
        InterpolationAlgorithmKind::from_name("Hermite"),
        Some(InterpolationAlgorithmKind::Hermite)
    );
    assert_eq!(InterpolationAlgorithmKind::from_name("Unknown"), None);
}

#[test]
fn algorithm_kind_dispatch() {
    let linear = InterpolationAlgorithmKind::Linear.algorithm();
    assert_eq!(linear.name(), "Linear");
    let lagrange = InterpolationAlgorithmKind::Lagrange.algorithm();
    assert_eq!(lagrange.name(), "Lagrange");
    let hermite = InterpolationAlgorithmKind::Hermite.algorithm();
    assert_eq!(hermite.name(), "Hermite");
}

// ─── LinearApproximation ───────────────────────────────────────────────────

#[test]
fn linear_name() {
    assert_eq!(LinearApproximation.name(), "Linear");
}

#[test]
fn linear_required_data_points() {
    // Linear 无论阶数如何始终需要 2 个点
    assert_eq!(LinearApproximation.get_required_data_points(1, 0), 2);
    assert_eq!(LinearApproximation.get_required_data_points(5, 0), 2);
}

#[test]
fn linear_does_not_support_derivatives() {
    assert!(!LinearApproximation.supports_derivatives());
}

#[test]
fn linear_interpolate_midpoint() {
    // x_table = [0, 10], y_table = [0, 100], stride=1
    let result = LinearApproximation.interpolate_order_zero(5.0, &[0.0, 10.0], &[0.0, 100.0], 1);
    assert_eq!(result.len(), 1);
    assert!((result[0] - 50.0).abs() < 1e-10);
}

#[test]
fn linear_interpolate_at_endpoints() {
    let x_table = [0.0, 10.0];
    let y_table = [20.0, 80.0];
    let r0 = LinearApproximation.interpolate_order_zero(0.0, &x_table, &y_table, 1);
    assert!((r0[0] - 20.0).abs() < 1e-10);
    let r1 = LinearApproximation.interpolate_order_zero(10.0, &x_table, &y_table, 1);
    assert!((r1[0] - 80.0).abs() < 1e-10);
}

#[test]
fn linear_interpolate_multi_stride() {
    // 每个样本 2 个分量：y_table = [x0,y0, x1,y1]
    let x_table = [0.0, 10.0];
    let y_table = [0.0, 100.0, 10.0, 200.0];
    let result = LinearApproximation.interpolate_order_zero(5.0, &x_table, &y_table, 2);
    assert_eq!(result.len(), 2);
    assert!((result[0] - 5.0).abs() < 1e-10);
    assert!((result[1] - 150.0).abs() < 1e-10);
}

#[test]
fn linear_interpolate_quarter() {
    let result = LinearApproximation.interpolate_order_zero(2.5, &[0.0, 10.0], &[0.0, 100.0], 1);
    assert!((result[0] - 25.0).abs() < 1e-10);
}

// ─── LagrangePolynomialApproximation ───────────────────────────────────────

#[test]
fn lagrange_name() {
    assert_eq!(LagrangePolynomialApproximation.name(), "Lagrange");
}

#[test]
fn lagrange_required_data_points() {
    // 需要 degree+1 个点（最少 2 个）
    assert_eq!(LagrangePolynomialApproximation.get_required_data_points(1, 0), 2);
    assert_eq!(LagrangePolynomialApproximation.get_required_data_points(2, 0), 3);
    assert_eq!(LagrangePolynomialApproximation.get_required_data_points(4, 0), 5);
}

#[test]
fn lagrange_does_not_support_derivatives() {
    assert!(!LagrangePolynomialApproximation.supports_derivatives());
}

#[test]
fn lagrange_linear_two_points() {
    // 2 个点时，Lagrange = linear
    let result = LagrangePolynomialApproximation.interpolate_order_zero(
        5.0, &[0.0, 10.0], &[0.0, 100.0], 1,
    );
    assert!((result[0] - 50.0).abs() < 1e-10);
}

#[test]
fn lagrange_quadratic_three_points() {
    // y = x^2: 取点 x=0,1,2 → y=0,1,4
    let x_table = [0.0, 1.0, 2.0];
    let y_table = [0.0, 1.0, 4.0];
    // 在 x=0.5 插值 → y=0.25
    let result = LagrangePolynomialApproximation.interpolate_order_zero(
        0.5, &x_table, &y_table, 1,
    );
    assert!((result[0] - 0.25).abs() < 1e-10);
}

#[test]
fn lagrange_passes_through_all_points() {
    let x_table = [0.0, 1.0, 2.0, 3.0];
    let y_table = [1.0, 3.0, 2.0, 5.0];
    for i in 0..4 {
        let result = LagrangePolynomialApproximation.interpolate_order_zero(
            x_table[i], &x_table, &y_table, 1,
        );
        assert!((result[0] - y_table[i]).abs() < 1e-10,
            "Failed at point {}: got {}, expected {}", i, result[0], y_table[i]);
    }
}

#[test]
fn lagrange_multi_stride() {
    // 2 个分量：y = [x, x^2]，x=0,1,2
    let x_table = [0.0, 1.0, 2.0];
    let y_table = [0.0, 0.0, 1.0, 1.0, 2.0, 4.0];
    let result = LagrangePolynomialApproximation.interpolate_order_zero(
        0.5, &x_table, &y_table, 2,
    );
    assert!((result[0] - 0.5).abs() < 1e-10);
    assert!((result[1] - 0.25).abs() < 1e-10);
}

// ─── HermitePolynomialApproximation ────────────────────────────────────────

#[test]
fn hermite_name() {
    assert_eq!(HermitePolynomialApproximation.name(), "Hermite");
}

#[test]
fn hermite_supports_derivatives() {
    assert!(HermitePolynomialApproximation.supports_derivatives());
}

#[test]
fn hermite_required_data_points() {
    // (degree+1)/(input_order+1), min 2
    assert_eq!(HermitePolynomialApproximation.get_required_data_points(1, 0), 2);
    assert_eq!(HermitePolynomialApproximation.get_required_data_points(3, 1), 2);
    assert_eq!(HermitePolynomialApproximation.get_required_data_points(5, 1), 3);
}

#[test]
fn hermite_order_zero_linear() {
    // 2 个点且无导数时，Hermite 零阶 = linear
    let result = HermitePolynomialApproximation.interpolate_order_zero(
        5.0, &[0.0, 10.0], &[0.0, 100.0], 1,
    );
    assert!((result[0] - 50.0).abs() < 1e-10);
}

#[test]
fn hermite_order_zero_passes_through_points() {
    let x_table = [0.0, 1.0, 2.0];
    let y_table = [10.0, 20.0, 15.0];
    for i in 0..3 {
        let result = HermitePolynomialApproximation.interpolate_order_zero(
            x_table[i], &x_table, &y_table, 1,
        );
        assert!((result[0] - y_table[i]).abs() < 1e-8,
            "Failed at point {}: got {}, expected {}", i, result[0], y_table[i]);
    }
}

#[test]
fn hermite_interpolate_with_derivatives() {
    // y = x^2, y' = 2x，x=0,1
    // x_table = [0, 1]，y_table 采用 stride=1、input_order=1：
    // y_table 布局：[y0, y'0, y1, y'1] = [0, 0, 1, 2]
    let x_table = [0.0, 1.0];
    let y_table = [0.0, 0.0, 1.0, 2.0];
    let result = HermitePolynomialApproximation.interpolate(
        0.5, &x_table, &y_table, 1, 1, 0,
    );
    // y(0.5) = 0.25
    assert!((result[0] - 0.25).abs() < 1e-8);
}

#[test]
fn hermite_interpolate_output_derivative() {
    // y = x^2, y' = 2x，x=0,1
    let x_table = [0.0, 1.0];
    let y_table = [0.0, 0.0, 1.0, 2.0];
    // output_order=1 → 返回 [y, y']
    let result = HermitePolynomialApproximation.interpolate(
        0.5, &x_table, &y_table, 1, 1, 1,
    );
    assert_eq!(result.len(), 2);
    assert!((result[0] - 0.25).abs() < 1e-8);
    assert!((result[1] - 1.0).abs() < 1e-8); // y'(0.5) = 1.0
}

#[test]
fn hermite_multi_stride_order_zero() {
    // 2 个分量，2 个点
    let x_table = [0.0, 10.0];
    let y_table = [0.0, 100.0, 10.0, 200.0];
    let result = HermitePolynomialApproximation.interpolate_order_zero(
        5.0, &x_table, &y_table, 2,
    );
    assert_eq!(result.len(), 2);
    assert!((result[0] - 5.0).abs() < 1e-10);
    assert!((result[1] - 150.0).abs() < 1e-10);
}
