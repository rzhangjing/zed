//! LinearSplineSpec → Rust 集成测试
//!
//! 原始：Specs/Core/LinearSplineSpec（8 个 it()）
//! A 类已移植：3 个（evaluate_number、evaluate_cartesian3_no_result、evaluate_cartesian3_with_result）
//! C 类已省略：5 个（构造函数 throws ×3，evaluate throws ×2 — 编译期类型安全）

use cesium_animation::spline::*;
use glam::DVec3;

fn setup() -> (Vec<f64>, Vec<DVec3>, Vec<f64>) {
    let times = vec![0.0, 1.0, 2.0, 3.0];
    let cartesian_points = vec![
        DVec3::new(-1.0, -1.0, 0.0),
        DVec3::new(-0.5, -0.125, 0.0),
        DVec3::new(0.5, 0.125, 0.0),
        DVec3::new(1.0, 1.0, 0.0),
    ];
    let number_points = vec![3.0, 5.0, 1.0, 10.0];
    (times, cartesian_points, number_points)
}

/// "evaluate returns number value"
#[test]
fn evaluate_returns_number_value() {
    let (times, _, number_points) = setup();
    // 对基于数值的样条使用 ScalarSpline（对应使用数值点的 LinearSpline）
    let ls = ScalarSpline::new(times.clone(), number_points.clone());

    // evaluate(times[0]) == numberPoints[0]
    let v = ls.evaluate(times[0]);
    assert!((v - number_points[0]).abs() < 1e-15);

    // 中点插值
    let time = (times[0] + times[1]) / 2.0;
    let t = (time - times[0]) / (times[1] - times[0]);
    let expected = (1.0 - t) * number_points[0] + t * number_points[1];
    let v = ls.evaluate(time);
    assert!((v - expected).abs() < 1e-15);
}

/// "evaluate returns cartesian3 value without result parameter"
#[test]
fn evaluate_returns_cartesian3_value() {
    let (times, cartesian_points, _) = setup();
    let ls = LinearSpline::new(times.clone(), cartesian_points.clone());

    // evaluate(times[0]) == cartesianPoints[0]
    let v = ls.evaluate(times[0]);
    assert!((v - cartesian_points[0]).length() < 1e-15);

    // 中点线性插值
    let time = (times[0] + times[1]) / 2.0;
    let t = (time - times[0]) / (times[1] - times[0]);
    let expected = cartesian_points[0].lerp(cartesian_points[1], t);
    let v = ls.evaluate(time);
    assert!((v - expected).length() < 1e-15);
}

/// "evaluate returns cartesian3 value with result parameter"
/// （结果参数变体已合并：Rust 返回 owned 值）
#[test]
fn evaluate_returns_cartesian3_with_result() {
    let (times, cartesian_points, _) = setup();
    let ls = LinearSpline::new(times.clone(), cartesian_points.clone());

    let time = (times[0] + times[1]) / 2.0;
    let t = (time - times[0]) / (times[1] - times[0]);
    let expected = cartesian_points[0].lerp(cartesian_points[1], t);
    let point = ls.evaluate(time);
    assert!((point - expected).length() < 1e-15);
}
