//! CatmullRomSplineSpec → Rust 集成测试
//!
//! 原始：Specs/Core/CatmullRomSplineSpec（11 个 it()）
//! A 类已移植：5 个（sets_tangents、computes_tangents、check_against_hermite、
//!                    evaluate_at_control_point、2pts_lerp）
//! C 类已省略：5 个（构造 throws ×3、求值 throws ×2）
//! 已合并：1 个（结果参数变体 → 返回所有权值）

use cesium_animation::spline::*;
use cesium_specs::epsilon;
use glam::DVec3;

fn setup() -> (Vec<f64>, Vec<DVec3>) {
    let points = vec![
        DVec3::new(-1.0, -1.0, 0.0),
        DVec3::new(-0.5, -0.125, 0.0),
        DVec3::new(0.5, 0.125, 0.0),
        DVec3::new(1.0, 1.0, 0.0),
    ];
    let times = vec![0.0, 1.0, 2.0, 3.0];
    (times, points)
}

/// "sets start and end tangents"
#[test]
fn sets_start_and_end_tangents() {
    let (times, points) = setup();
    let start = points[1] - points[0];
    let end = points[points.len() - 1] - points[points.len() - 2];

    let crs = CatmullRomSpline::with_tangents(
        times,
        points,
        start,
        end,
    );

    assert!((crs.first_tangent - start).length() < 1e-15);
    assert!((crs.last_tangent - end).length() < 1e-15);
}

/// "computes start and end tangents"
#[test]
fn computes_start_and_end_tangents() {
    let (times, points) = setup();

    // CesiumJS formula:
    // start = (2*points[1] - points[2] - points[0]) * 0.5
    let start = (points[1] * 2.0 - points[2] - points[0]) * 0.5;

    // end = (points[n-1] - 2*points[n-2] + points[n-3]) * 0.5
    let n = points.len() - 1;
    let end = (points[n] - points[n - 1] * 2.0 + points[n - 2]) * 0.5;

    let crs = CatmullRomSpline::new(times, points);

    assert!(
        (crs.first_tangent - start).length() < 1e-15,
        "first_tangent: got {:?}, expected {:?}", crs.first_tangent, start
    );
    assert!(
        (crs.last_tangent - end).length() < 1e-15,
        "last_tangent: got {:?}, expected {:?}", crs.last_tangent, end
    );
}

/// "check Catmull-Rom spline against a Hermite spline"
#[test]
fn check_catmull_rom_against_hermite() {
    let (times, points) = setup();
    let crs = CatmullRomSpline::new(times.clone(), points.clone());

    // 通过 createC1 构造等价的 HermiteSpline
    let mut tangents = vec![crs.first_tangent];
    for i in 1..points.len() - 1 {
        tangents.push((points[i + 1] - points[i - 1]) * 0.5);
    }
    tangents.push(crs.last_tangent);

    let hs = HermiteSpline::create_c1(times.clone(), points.clone(), tangents);

    let granularity = 0.5;
    let mut j = times[0];
    while j <= times[points.len() - 1] {
        let h_val = hs.evaluate(j);
        let cr_val = crs.evaluate(j);
        assert!(
            (h_val - cr_val).length() < epsilon::EPSILON4,
            "at j={}: hermite={:?}, catmull_rom={:?}, diff={}",
            j, h_val, cr_val, (h_val - cr_val).length()
        );
        j += granularity;
    }
}

/// "evaluate with result parameter" → merged: evaluate(times[0]) == points[0]
#[test]
fn evaluate_at_control_point() {
    let (times, points) = setup();
    let crs = CatmullRomSpline::new(times.clone(), points.clone());

    let point = crs.evaluate(times[0]);
    assert!((point - points[0]).length() < 1e-15);
}

/// "spline with 2 control points defaults to lerp"
#[test]
fn spline_2_control_points_defaults_to_lerp() {
    let points = vec![
        DVec3::new(-1.0, -1.0, 0.0),
        DVec3::new(-0.5, -0.125, 0.0),
    ];
    let times = vec![0.0, 1.0];

    let crs = CatmullRomSpline::new(times.clone(), points.clone());

    let t = (times[0] + times[1]) * 0.5;
    let expected = points[0].lerp(points[1], t);
    let actual = crs.evaluate(t);
    assert!(
        (actual - expected).length() < 1e-15,
        "got {:?}, expected {:?}", actual, expected
    );
}
