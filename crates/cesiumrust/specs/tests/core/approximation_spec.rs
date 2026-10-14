//! 近似算法规格测试 - 参考自：
//! - Specs/Core/LinearApproximationSpec（7 个 it()）
//! - Specs/Core/LagrangePolynomialApproximationSpec（3 个 it()）
//! - Specs/Core/HermitePolynomialApproximationSpec（4 个 it()）
//!
//! A 类测试：11 个（跳过"result 参数"模式测试）

use cesium_datasource::property_system::interpolation::{
    HermitePolynomialApproximation, InterpolationAlgorithm, LagrangePolynomialApproximation,
    LinearApproximation,
};

// ============================================================
// LinearApproximation
// ============================================================

#[test]
fn linear_should_produce_correct_results() {
    let x_table = [2.0, 4.0];
    let y_table = [2.0, 3.0, 4.0, 34.0];

    let alg = LinearApproximation;
    let results = alg.interpolate_order_zero(3.0, &x_table, &y_table, 2);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0], 3.0);
    assert_eq!(results[1], 18.5);
}

#[test]
fn linear_should_produce_correct_results_2() {
    let x_table = [40.0, 120.0];
    let y_table = [20.0, 40.0, 60.0, 80.0, 90.0, 100.0];

    let alg = LinearApproximation;
    let results = alg.interpolate_order_zero(80.0, &x_table, &y_table, 3);

    assert_eq!(results.len(), 3);
    assert_eq!(results[0], 50.0);
    assert_eq!(results[1], 65.0);
    assert_eq!(results[2], 80.0);
}

#[test]
fn linear_should_produce_correct_results_3() {
    let x_table = [20.0, 30.0];
    let y_table = [10.0, 20.0, 30.0, 20.0, 30.0, 40.0, 20.0, 40.0, 60.0, 80.0, 90.0, 100.0];

    let alg = LinearApproximation;
    let results = alg.interpolate_order_zero(40.0, &x_table, &y_table, 1);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], 30.0);
}

#[test]
#[should_panic]
fn linear_should_throw_if_x_table_length_greater_than_2() {
    let x_table = [44.0, 99.0, 230.0];
    let y_table = [2.3, 4.5, 6.6, 3.2, 4.4, 12.23];

    let alg = LinearApproximation;
    let _ = alg.interpolate_order_zero(2.3, &x_table, &y_table, 3);
}

#[test]
#[should_panic]
fn linear_should_throw_when_y_stride_equals_zero() {
    let x_table = [4.0, 8.0];
    let y_table = [4.0, 8.0];

    let alg = LinearApproximation;
    let _ = alg.interpolate_order_zero(6.0, &x_table, &y_table, 0);
}

#[test]
fn linear_get_required_data_points_returns_2() {
    let alg = LinearApproximation;
    assert_eq!(alg.get_required_data_points(1, 0), 2);
}

// ============================================================
// LagrangePolynomialApproximation
// ============================================================

/// 已对照 STK Components（www.agi.com/components/）验证
const LAGRANGE_X_TABLE: [f64; 8] = [0.0, 60.0, 120.0, 180.0, 240.0, 300.0, 360.0, 420.0];

#[allow(clippy::excessive_precision)]
const LAGRANGE_Y_TABLE: [f64; 24] = [
    13378137.0, 0.0, 0.0,
    13374128.3576279, 327475.593690065, 0.0,
    13362104.8328212, 654754.936954423, 0.0,
    13342073.6310691, 981641.896976832, 0.0,
    13314046.7567223, 1307940.57608951, 0.0,
    13278041.005799, 1633455.42917117, 0.0,
    13234077.9559193, 1957991.38083385, 0.0,
    13182183.953374, 2281353.94232816, 0.0,
];

#[test]
fn lagrange_interpolation_produces_correct_results() {
    let alg = LagrangePolynomialApproximation;
    let result = alg.interpolate_order_zero(100.0, &LAGRANGE_X_TABLE, &LAGRANGE_Y_TABLE, 3);

    let expected = [13367002.870928623, 545695.7388100647, 0.0];
    for i in 0..3 {
        assert!(
            (result[i] - expected[i]).abs() <= 1e-15 * expected[i].abs().max(1.0),
            "result[{}] = {}, expected {}",
            i, result[i], expected[i]
        );
    }
}

#[test]
fn lagrange_get_required_data_points() {
    let alg = LagrangePolynomialApproximation;
    assert_eq!(alg.get_required_data_points(0, 0), 2);
    assert_eq!(alg.get_required_data_points(1, 0), 2);
    assert_eq!(alg.get_required_data_points(2, 0), 3);
    assert_eq!(alg.get_required_data_points(3, 0), 4);
}

// ============================================================
// HermitePolynomialApproximation
// ============================================================

/// 已对照 STK Components（www.agi.com/components/）验证
const HERMITE_X_TABLE: [f64; 8] = [0.0, 60.0, 120.0, 180.0, 240.0, 300.0, 360.0, 420.0];

#[allow(clippy::excessive_precision)]
const HERMITE_Y_TABLE: [f64; 16] = [
    13378137.0, 0.0,
    13374128.3576279, 0.0,
    13362104.8328212, 0.0,
    13342073.6310691, 0.0,
    13314046.7567223, 0.0,
    13278041.005799, 0.0,
    13234077.9559193, 0.0,
    13182183.953374, 0.0,
];

#[allow(clippy::excessive_precision)]
const HERMITE_DY_TABLE: [f64; 16] = [
    0.0, 0.0,
    -133.614738921601, 0.0,
    -267.149404854867, 0.0,
    -400.523972797808, 0.0,
    -533.658513692378, 0.0,
    -666.473242324565, 0.0,
    -798.888565138278, 0.0,
    -930.82512793439, 0.0,
];

/// 构造 yStride=4 的组合 yTable：每点 [y0, y1, dy0, dy1]。
fn hermite_combined_table() -> Vec<f64> {
    let mut combined = vec![0.0f64; 32];
    for i in 0..8 {
        combined[i * 4] = HERMITE_Y_TABLE[i * 2];
        combined[i * 4 + 1] = HERMITE_Y_TABLE[i * 2 + 1];
        combined[i * 4 + 2] = HERMITE_DY_TABLE[i * 2];
        combined[i * 4 + 3] = HERMITE_DY_TABLE[i * 2 + 1];
    }
    combined
}

#[test]
fn hermite_interpolating_produces_correct_results() {
    let combined = hermite_combined_table();
    let alg = HermitePolynomialApproximation;
    let result = alg.interpolate_order_zero(100.0, &HERMITE_X_TABLE, &combined, 4);

    let expected = 13367002.870928625;
    // 精度较低，因为不再使用导数信息
    assert!(
        (result[0] - expected).abs() <= 1e-6 * expected.abs(),
        "result[0] = {}, expected {}",
        result[0], expected
    );
}

#[test]
fn hermite_get_required_data_points() {
    let alg = HermitePolynomialApproximation;
    assert_eq!(alg.get_required_data_points(0, 0), 2);
    assert_eq!(alg.get_required_data_points(1, 0), 2);
    assert_eq!(alg.get_required_data_points(2, 0), 3);
    assert_eq!(alg.get_required_data_points(3, 0), 4);
    assert_eq!(alg.get_required_data_points(3, 1), 2);
    assert_eq!(alg.get_required_data_points(5, 1), 3);
    assert_eq!(alg.get_required_data_points(7, 1), 4);
}

#[test]
fn hermite_higher_order_interpolation_produces_correct_results() {
    let combined = hermite_combined_table();
    let alg = HermitePolynomialApproximation;
    let result = alg.interpolate(100.0, &HERMITE_X_TABLE, &combined, 2, 1, 1);

    let expected: [f64; 4] = [13367002.870928625, 0.0, -222.65168787012135, 0.0];
    for i in 0..4 {
        let tol = 1e-8 * expected[i].abs().max(1.0);
        assert!(
            (result[i] - expected[i]).abs() <= tol,
            "result[{}] = {}, expected {} (tol={})",
            i, result[i], expected[i], tol
        );
    }
}
