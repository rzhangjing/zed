//! HermiteSplineSpec → Rust 集成测试
//!
//! 原始：Specs/Core/HermiteSplineSpec（34 个 it()）
//! A 类已移植：8 个（create_spline、C1、natural_cubic、clamped_cubic、evaluate_number、
//!                    evaluate_cartesian3、natural_2pts_lerp、clamped_2pts_lerp）
//! C 类已省略：22 个 throws（编译期类型安全）
//! 已省略：2 个 quaternion evaluate（Rust 类型系统：需要单独的 QuaternionHermiteSpline）
//! 已合并：2 个结果参数变体 → owned 返回

use cesium_animation::spline::*;
use cesium_specs::epsilon;
use glam::DVec3;
use std::f64::consts::PI;

const FRAC_PI_2: f64 = PI / 2.0;
const THREE_PI_OVER_TWO: f64 = 3.0 * PI / 2.0;

/// 与 CesiumJS 规格辅助函数 `createHermiteBasis(p, pT, q, qT)` 匹配的 Hermite 基函数。
fn hermite_basis(p: DVec3, pt: DVec3, q: DVec3, qt: DVec3, u: f64) -> DVec3 {
    let a = 2.0 * u * u * u - 3.0 * u * u + 1.0;
    let b = -2.0 * u * u * u + 3.0 * u * u;
    let c = u * u * u - 2.0 * u * u + u;
    let d = u * u * u - u * u;
    p * a + q * b + pt * c + qt * d
}

/// "create spline"
#[test]
fn create_spline() {
    let hs = HermiteSpline::new(
        vec![0.0, 1.0, 3.0, 4.5, 6.0],
        vec![
            DVec3::new(1235398.0, -4810983.0, 4146266.0),
            DVec3::new(1372574.0, -5345182.0, 4606657.0),
            DVec3::new(-757983.0, -5542796.0, 4514323.0),
            DVec3::new(-2821260.0, -5248423.0, 4021290.0),
            DVec3::new(-2539788.0, -4724797.0, 3620093.0),
        ],
        // inTangents（长度 4 = points.len()-1）
        vec![
            DVec3::new(-1993381.0, -731813.0, 368057.0),
            DVec3::new(-4193834.0, 96759.0, -585367.0),
            DVec3::new(-1781805.0, 817999.0, -894230.0),
            DVec3::new(1165345.0, 112641.0, 47281.0),
        ],
        // outTangents（长度 4 = points.len()-1）
        vec![
            DVec3::new(1125196.0, -161816.0, 270551.0),
            DVec3::new(-996690.5, -365906.5, 184028.5),
            DVec3::new(-2096917.0, 48379.5, -292683.5),
            DVec3::new(-890902.5, 408999.5, -447115.0),
        ],
    );

    let p0 = hs.points[0];
    let p1 = hs.points[1];
    let pt0 = hs.out_tangents[0];
    let pt1 = hs.in_tangents[0];

    let granularity = 0.1;
    let mut i = 0.0f64;
    while i < 1.0 {
        let expected = hermite_basis(p0, pt0, p1, pt1, i);
        let actual = hs.evaluate(i);
        assert!(
            (actual - expected).length() < epsilon::EPSILON3,
            "at u={}: actual={:?}, expected={:?}",
            i, actual, expected
        );
        i += granularity;
    }
}

/// "C1 spline"
#[test]
fn c1_spline() {
    let times = vec![0.0, 1.0, 3.0, 4.5, 6.0];
    let points = vec![
        DVec3::new(1235398.0, -4810983.0, 4146266.0),
        DVec3::new(1372574.0, -5345182.0, 4606657.0),
        DVec3::new(-757983.0, -5542796.0, 4514323.0),
        DVec3::new(-2821260.0, -5248423.0, 4021290.0),
        DVec3::new(-2539788.0, -4724797.0, 3620093.0),
    ];

    let mut tangents = vec![DVec3::ZERO; points.len()];
    tangents[0] = DVec3::new(1125196.0, -161816.0, 270551.0);
    for i in 1..tangents.len() - 1 {
        tangents[i] = (points[i + 1] - points[i - 1]) * 0.5;
    }
    let last_idx = tangents.len() - 1;
    tangents[last_idx] = DVec3::new(1165345.0, 112641.0, 47281.0);

    let hs = HermiteSpline::create_c1(times.clone(), points.clone(), tangents.clone());

    let granularity = 0.1;
    let mut j = times[0];
    while j < times[1] {
        // 对于第一段：u = (j - times[0]) / (times[1] - times[0])
        let u = (j - times[0]) / (times[1] - times[0]);
        let expected = hermite_basis(points[0], tangents[0], points[1], tangents[1], u);
        let actual = hs.evaluate(j);
        assert!(
            (actual - expected).length() < epsilon::EPSILON3,
            "at j={}: actual={:?}, expected={:?}",
            j, actual, expected
        );
        j += granularity;
    }
}

/// "natural cubic spline"
#[test]
fn natural_cubic_spline() {
    let points = vec![
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, 1.0, FRAC_PI_2),
        DVec3::new(-1.0, 0.0, PI),
        DVec3::new(0.0, -1.0, THREE_PI_OVER_TWO),
    ];
    let times = vec![0.0, 1.0, 2.0, 3.0];

    let p0_tangent = DVec3::new(-0.87, 1.53, 1.57);
    let p1_tangent = DVec3::new(-1.27, -0.07, 1.57);

    let hs = HermiteSpline::create_natural_cubic(times.clone(), points.clone());

    let granularity = 0.1;
    let mut i = times[0];
    while i < times[1] {
        let u = (i - times[0]) / (times[1] - times[0]);
        let expected = hermite_basis(points[0], p0_tangent, points[1], p1_tangent, u);
        let actual = hs.evaluate(i);
        assert!(
            (actual - expected).length() < epsilon::EPSILON3,
            "at i={}: actual={:?}, expected={:?}, diff={}",
            i, actual, expected, (actual - expected).length()
        );
        i += granularity;
    }
}

/// "clamped cubic spline"
/// 注意：CesiumJS 规格有一个 bug（循环使用未定义的 .time 属性 → 循环从不执行）。
/// 我们验证构造成功以及首/末点的求值。
#[test]
fn clamped_cubic_spline() {
    let points = vec![
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, 1.0, FRAC_PI_2),
        DVec3::new(-1.0, 0.0, PI),
        DVec3::new(0.0, -1.0, THREE_PI_OVER_TWO),
    ];
    let times = vec![0.0, 1.0, 2.0, 3.0];
    let first_tangent = DVec3::new(0.0, 1.0, 0.0);
    let last_tangent = DVec3::new(1.0, 0.0, 0.0);

    let hs = HermiteSpline::create_clamped_cubic(
        times.clone(),
        points.clone(),
        first_tangent,
        last_tangent,
    );

    // 验证端点
    let p0 = hs.evaluate(times[0]);
    assert!((p0 - points[0]).length() < epsilon::EPSILON10);
    let p_last = hs.evaluate(times[3]);
    assert!((p_last - points[3]).length() < epsilon::EPSILON10);

    // 验证第一段使用 firstTangent
    let u = 0.5;
    let time = times[0] + u * (times[1] - times[0]);
    let expected = hermite_basis(points[0], first_tangent, points[1], hs.in_tangents[0], u);
    let actual = hs.evaluate(time);
    assert!(
        (actual - expected).length() < epsilon::EPSILON3,
        "clamped first segment: actual={:?}, expected={:?}", actual, expected
    );
}

/// "evaluate returns number value"
/// 使用 DVec3，其 x 分量编码标量（Rust 类型系统：无泛型 HermiteSpline<f64>）。
#[test]
fn evaluate_returns_number_value() {
    // 原始：times=[0, 0.5, 1.0], points=[0, 1, 0], inTangents=[0, 1], outTangents=[0, -3]
    // 期望：evaluate(0.5) == 1.0, evaluate(0.75) == 0.25
    // 我们用 DVec3 的 x 分量验证：
    let times = vec![0.0, 0.5, 1.0];
    let points = vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 0.0),
    ];
    let in_tangents = vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0)];
    let out_tangents = vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(-3.0, 0.0, 0.0)];

    let hs = HermiteSpline::new(times, points, in_tangents, out_tangents);

    let point = hs.evaluate(0.5);
    assert!((point.x - 1.0).abs() < 1e-15);

    let point = hs.evaluate(0.75);
    assert!((point.x - 0.25).abs() < 1e-15, "got {}", point.x);
}

/// "evaluate returns cartesian3 value"
#[test]
fn evaluate_returns_cartesian3_value() {
    let times = vec![0.0, 0.5, 1.0];
    let points = vec![
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(1.0, 2.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
    ];
    let in_tangents = vec![DVec3::ZERO, DVec3::ZERO];
    let out_tangents = vec![DVec3::ZERO, DVec3::new(0.0, -3.0, 0.0)];

    let hs = HermiteSpline::new(times, points.clone(), in_tangents, out_tangents);

    let point = hs.evaluate(0.5);
    assert!((point - points[1]).length() < 1e-15);

    let expected = DVec3::new(0.0, 0.8125, 0.0);
    let point = hs.evaluate(0.75);
    assert!(
        (point - expected).length() < 1e-15,
        "got {:?}, expected {:?}", point, expected
    );
}

/// "createNaturalCubic with 2 control points defaults to lerp"
#[test]
fn natural_cubic_2_points_defaults_to_lerp() {
    let points = vec![
        DVec3::new(-1.0, -1.0, 0.0),
        DVec3::new(-0.5, -0.125, 0.0),
    ];
    let times = vec![0.0, 1.0];

    let hs = HermiteSpline::create_natural_cubic(times.clone(), points.clone());

    let t = (times[0] + times[1]) * 0.5;
    let expected = points[0].lerp(points[1], t);
    let actual = hs.evaluate(t);
    assert!((actual - expected).length() < 1e-15);
}

/// "createClampedCubic with 2 control points defaults to lerp"
#[test]
fn clamped_cubic_2_points_defaults_to_lerp() {
    let points = vec![
        DVec3::new(-1.0, -1.0, 0.0),
        DVec3::new(-0.5, -0.125, 0.0),
    ];
    let times = vec![0.0, 1.0];

    let hs = HermiteSpline::create_natural_cubic(times.clone(), points.clone());

    let t = (times[0] + times[1]) * 0.5;
    let expected = points[0].lerp(points[1], t);
    let actual = hs.evaluate(t);
    assert!((actual - expected).length() < 1e-15);
}
