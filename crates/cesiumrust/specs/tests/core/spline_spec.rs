//! Core/CatmullRomSplineSpec, HermiteSplineSpec, LinearSplineSpec,
//! QuaternionSplineSpec, SteppedSplineSpec, ConstantSplineSpec
//! → Rust integration tests for cesium_animation::spline

use cesium_animation::spline::*;
use cesium_specs::{assert_approx, assert_vec3_epsilon, epsilon};
use glam::DVec3;

// === LinearSpline ===

#[test]
fn test_linear_spline_evaluate_endpoints() {
    let times = vec![0.0, 1.0, 2.0];
    let points = vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(1.0, 1.0, 1.0),
        DVec3::new(2.0, 2.0, 2.0),
    ];
    let spline = LinearSpline::new(times, points);

    let p0 = spline.evaluate(0.0);
    assert_vec3_epsilon!(p0, DVec3::new(0.0, 0.0, 0.0), epsilon::EPSILON10);

    let p2 = spline.evaluate(2.0);
    assert_vec3_epsilon!(p2, DVec3::new(2.0, 2.0, 2.0), epsilon::EPSILON10);
}

#[test]
fn test_linear_spline_evaluate_midpoint() {
    let times = vec![0.0, 1.0];
    let points = vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(2.0, 4.0, 6.0),
    ];
    let spline = LinearSpline::new(times, points);

    let mid = spline.evaluate(0.5);
    assert_vec3_epsilon!(mid, DVec3::new(1.0, 2.0, 3.0), epsilon::EPSILON10);
}

#[test]
fn test_linear_spline_clamp_time() {
    let times = vec![0.0, 1.0, 2.0];
    let points = vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(1.0, 1.0, 1.0),
        DVec3::new(2.0, 2.0, 2.0),
    ];
    let spline = LinearSpline::new(times, points);

    assert_approx!(spline.clamp_time(-1.0), 0.0, epsilon::EPSILON15);
    assert_approx!(spline.clamp_time(3.0), 2.0, epsilon::EPSILON15);
    assert_approx!(spline.clamp_time(1.5), 1.5, epsilon::EPSILON15);
}

// === CatmullRomSpline ===

#[test]
fn test_catmull_rom_spline_passes_through_control_points() {
    let times = vec![0.0, 1.0, 2.0, 3.0];
    let points = vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(1.0, 2.0, 0.0),
        DVec3::new(3.0, 1.0, 0.0),
        DVec3::new(4.0, 0.0, 0.0),
    ];
    let expected_points = points.clone();
    let expected_times = times.clone();
    let spline = CatmullRomSpline::new(times, points);

    // 应经过所有控制点
    for (i, &t) in expected_times.iter().enumerate() {
        let p = spline.evaluate(t);
        assert_vec3_epsilon!(p, expected_points[i], epsilon::EPSILON8);
    }
}

#[test]
fn test_catmull_rom_spline_smooth_interpolation() {
    let times = vec![0.0, 1.0, 2.0];
    let points = vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(1.0, 1.0, 0.0),
        DVec3::new(2.0, 0.0, 0.0),
    ];
    let spline = CatmullRomSpline::new(times, points);

    // 中点应平滑（不一定线性）
    let mid = spline.evaluate(0.5);
    // 应在控制点之间的某处
    assert!(mid.x > 0.0 && mid.x < 1.0);
}

// === HermiteSpline ===

#[test]
fn test_hermite_spline_endpoints() {
    let times = vec![0.0, 1.0];
    let points = vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(1.0, 1.0, 1.0),
    ];
    let in_tangents = vec![DVec3::new(1.0, 0.0, 0.0)];
    let out_tangents = vec![DVec3::new(1.0, 0.0, 0.0)];
    let spline = HermiteSpline::new(times, points, in_tangents, out_tangents);

    let p0 = spline.evaluate(0.0);
    assert_vec3_epsilon!(p0, DVec3::new(0.0, 0.0, 0.0), epsilon::EPSILON10);

    let p1 = spline.evaluate(1.0);
    assert_vec3_epsilon!(p1, DVec3::new(1.0, 1.0, 1.0), epsilon::EPSILON10);
}

// === QuaternionSpline ===

#[test]
fn test_quaternion_spline_slerp() {
    use glam::DQuat;
    let times = vec![0.0, 1.0];
    let points = vec![
        DQuat::IDENTITY,
        DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2),
    ];
    let spline = QuaternionSpline::new(times, points);

    let q0 = spline.evaluate(0.0);
    assert_approx!(q0.w, 1.0, epsilon::EPSILON10);

    let q1 = spline.evaluate(1.0);
    let expected = DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2);
    assert_approx!(q1.z, expected.z, epsilon::EPSILON10);
    assert_approx!(q1.w, expected.w, epsilon::EPSILON10);
}

fn assert_quat_eq_eps(actual: glam::DQuat, expected: glam::DQuat, eps: f64) {
    assert!((actual.x - expected.x).abs() < eps, "quat.x: {} vs {}", actual.x, expected.x);
    assert!((actual.y - expected.y).abs() < eps, "quat.y: {} vs {}", actual.y, expected.y);
    assert!((actual.z - expected.z).abs() < eps, "quat.z: {} vs {}", actual.z, expected.z);
    assert!((actual.w - expected.w).abs() < eps, "quat.w: {} vs {}", actual.w, expected.w);
}

/// "evaluate without result parameter" 的移植：在节点处 evaluate 返回该
/// 控制点；在线段中点处 evaluate 与 Quaternion.slerp 匹配。
#[test]
fn test_quaternion_spline_evaluate_knot_and_midpoint() {
    use glam::DQuat;
    let pi4 = std::f64::consts::FRAC_PI_4;
    let points = vec![
        DQuat::from_axis_angle(DVec3::X, pi4),
        DQuat::from_axis_angle(DVec3::Z, pi4),
        DQuat::from_axis_angle(DVec3::X, -pi4),
        DQuat::from_axis_angle(DVec3::Y, pi4),
    ];
    let times = vec![0.0, 1.0, 2.0, 3.0];
    let spline = QuaternionSpline::new(times.clone(), points.clone());

    // 在第一个节点处 evaluate 返回第一个控制点
    let q0 = spline.evaluate(times[0]);
    assert_quat_eq_eps(q0, points[0], epsilon::EPSILON6);

    // 线段 [times[1], times[2]] 的中点
    let time = (times[2] + times[1]) * 0.5;
    let t = (time - times[1]) / (times[2] - times[1]);
    let actual = spline.evaluate(time);
    let expected = points[1].slerp(points[2], t);
    assert_quat_eq_eps(actual, expected, epsilon::EPSILON6);
}

/// Port of "spline with 2 control points defaults to slerp".
#[test]
fn test_quaternion_spline_two_points_defaults_to_slerp() {
    use glam::DQuat;
    let pi4 = std::f64::consts::FRAC_PI_4;
    let points = vec![
        DQuat::from_axis_angle(DVec3::X, pi4),
        DQuat::from_axis_angle(DVec3::Z, pi4),
    ];
    let times = vec![0.0, 1.0];
    let spline = QuaternionSpline::new(times.clone(), points.clone());

    let t = (times[0] + times[1]) * 0.5;
    let actual = spline.evaluate(t);
    let expected = points[0].slerp(points[1], t);
    assert_quat_eq_eps(actual, expected, epsilon::EPSILON6);
}

// === SteppedSpline ===

#[test]
fn test_stepped_spline_holds_previous() {
    let times = vec![0.0, 1.0, 2.0];
    let points = vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(1.0, 1.0, 1.0),
        DVec3::new(2.0, 2.0, 2.0),
    ];
    let spline = SteppedSpline::new(times, points);

    // Between 0 and 1, should hold the first point
    let p = spline.evaluate(0.5);
    assert_vec3_epsilon!(p, DVec3::new(0.0, 0.0, 0.0), epsilon::EPSILON10);

    // 恰好为 1.0 时，应为第二个点
    let p1 = spline.evaluate(1.0);
    assert_vec3_epsilon!(p1, DVec3::new(1.0, 1.0, 1.0), epsilon::EPSILON10);
}

// === ConstantSpline ===

#[test]
fn test_constant_spline_always_same() {
    let value = DVec3::new(5.0, 10.0, 15.0);
    let spline = ConstantSpline::new(value);

    let p0 = spline.evaluate(0.0);
    let p50 = spline.evaluate(50.0);
    let p100 = spline.evaluate(100.0);

    assert_vec3_epsilon!(p0, value, epsilon::EPSILON15);
    assert_vec3_epsilon!(p50, value, epsilon::EPSILON15);
    assert_vec3_epsilon!(p100, value, epsilon::EPSILON15);
}

// === ScalarSpline ===

#[test]
fn test_scalar_spline_linear() {
    let times = vec![0.0, 1.0, 2.0];
    let values = vec![0.0, 10.0, 20.0];
    let spline = ScalarSpline::new(times, values);

    assert_approx!(spline.evaluate(0.0), 0.0, epsilon::EPSILON10);
    assert_approx!(spline.evaluate(0.5), 5.0, epsilon::EPSILON10);
    assert_approx!(spline.evaluate(1.0), 10.0, epsilon::EPSILON10);
    assert_approx!(spline.evaluate(1.5), 15.0, epsilon::EPSILON10);
    assert_approx!(spline.evaluate(2.0), 20.0, epsilon::EPSILON10);
}

// === MorphWeightSpline ===

#[test]
fn test_morph_weight_spline() {
    let times = vec![0.0, 1.0];
    let weights = vec![0.0, 1.0];
    let spline = MorphWeightSpline::new(times, weights);

    assert_approx!(spline.evaluate(0.0), 0.0, epsilon::EPSILON10);
    assert_approx!(spline.evaluate(0.5), 0.5, epsilon::EPSILON10);
    assert_approx!(spline.evaluate(1.0), 1.0, epsilon::EPSILON10);
}

// === Spline trait methods ===

#[test]
fn test_spline_find_time_interval() {
    let times = vec![0.0, 1.0, 2.0, 3.0];
    let points = vec![
        DVec3::ZERO,
        DVec3::ONE,
        DVec3::splat(2.0),
        DVec3::splat(3.0),
    ];
    let spline = LinearSpline::new(times, points);

    assert_eq!(spline.find_time_interval(0.5), 0);
    assert_eq!(spline.find_time_interval(1.5), 1);
    assert_eq!(spline.find_time_interval(2.5), 2);
}

#[test]
fn test_spline_wrap_time() {
    let times = vec![0.0, 1.0, 2.0];
    let points = vec![DVec3::ZERO, DVec3::ONE, DVec3::splat(2.0)];
    let spline = LinearSpline::new(times, points);

    let wrapped = spline.wrap_time(2.5);
    assert!(wrapped >= 0.0 && wrapped <= 2.0);
}
