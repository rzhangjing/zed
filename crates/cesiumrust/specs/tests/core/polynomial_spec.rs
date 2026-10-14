//! QuadraticRealPolynomialSpec + CubicRealPolynomialSpec + QuarticRealPolynomialSpec
//! → Rust integration tests
//!
//! Original: QuadraticRealPolynomialSpec (18 it()), CubicRealPolynomialSpec (14 it()),
//!           QuarticRealPolynomialSpec (21 it())
//! A-class ported: 12 + 6 + 11 = 29
//! C-class omitted: 6 + 8 + 10 = 24 (throws)

use cesium_geospatial::polynomial::*;

// =============================================================================
// QuadraticRealPolynomial
// =============================================================================

/// "discriminant"
#[test]
fn quadratic_discriminant_value() {
    let d = quadratic_discriminant(1.0, 2.0, 3.0);
    assert_eq!(d, -8.0);
}

/// "negative b"
#[test]
fn quadratic_negative_b() {
    let roots = quadratic_real_roots(2.0, -4.0, -6.0);
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0], -1.0);
    assert_eq!(roots[1], 3.0);
}

/// "positive b"
#[test]
fn quadratic_positive_b() {
    let roots = quadratic_real_roots(2.0, 4.0, -6.0);
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0], -3.0);
    assert_eq!(roots[1], 1.0);
}

/// "marginally negative radical case"
#[test]
fn quadratic_marginally_negative_radical() {
    let roots = quadratic_real_roots(2.0, -3.999999999999999, 2.0);
    assert_eq!(roots.len(), 2);
    assert!((roots[0] - 1.0).abs() < 1e-15);
    assert!((roots[1] - 1.0).abs() < 1e-15);
}

/// "complex roots"
#[test]
fn quadratic_complex_roots() {
    let roots = quadratic_real_roots(2.0, -4.0, 6.0);
    assert_eq!(roots.len(), 0);
}

/// "intractable case"
#[test]
fn quadratic_intractable() {
    let roots = quadratic_real_roots(0.0, 0.0, -3.0);
    assert_eq!(roots.len(), 0);
}

/// "linear case"
#[test]
fn quadratic_linear_case() {
    let roots = quadratic_real_roots(0.0, 2.0, 8.0);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0], -4.0);
}

/// "2nd order monomial case"
#[test]
fn quadratic_monomial() {
    let roots = quadratic_real_roots(3.0, 0.0, 0.0);
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0], 0.0);
    assert_eq!(roots[1], 0.0);
}

/// "parabolic case with complex roots"
#[test]
fn quadratic_parabolic_complex() {
    let roots = quadratic_real_roots(3.0, 0.0, 18.0);
    assert_eq!(roots.len(), 0);
}

/// "parabolic case with real roots"
#[test]
fn quadratic_parabolic_real() {
    let roots = quadratic_real_roots(2.0, 0.0, -18.0);
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0], -3.0);
    assert_eq!(roots[1], 3.0);
}

/// "zero and negative root case"
#[test]
fn quadratic_zero_and_negative_root() {
    let roots = quadratic_real_roots(2.0, 6.0, 0.0);
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0], -3.0);
    assert_eq!(roots[1], 0.0);
}

/// "zero and positive root case"
#[test]
fn quadratic_zero_and_positive_root() {
    let roots = quadratic_real_roots(2.0, -6.0, 0.0);
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0], 0.0);
    assert_eq!(roots[1], 3.0);
}

// =============================================================================
// CubicRealPolynomial
// =============================================================================

/// "discriminant"
#[test]
fn cubic_discriminant_value() {
    let a = 3.0;
    let b = 2.0;
    let c = 1.0;
    let d = 1.0;
    let expected = b * b * c * c - 4.0 * a * c * c * c - 4.0 * b * b * b * d
        - 27.0 * a * a * d * d
        + 18.0 * a * b * c * d;
    let actual = cubic_discriminant(a, b, c, d);
    assert!((actual - expected).abs() < 1e-14);
}

/// "three repeated roots"
#[test]
fn cubic_three_repeated_roots() {
    let roots = cubic_real_roots(2.0, -12.0, 24.0, -16.0);
    assert_eq!(roots.len(), 3);
    assert!((roots[0] - 2.0).abs() < 1e-15);
    assert!((roots[1] - 2.0).abs() < 1e-15);
    assert!((roots[2] - 2.0).abs() < 1e-15);
}

/// "one unique and two repeated roots"
#[test]
fn cubic_one_unique_two_repeated() {
    let roots = cubic_real_roots(2.0, 2.0, -2.0, -2.0);
    assert_eq!(roots.len(), 3);
    assert!((roots[0] - (-1.0)).abs() < 1e-15);
    assert!((roots[1] - (-1.0)).abs() < 1e-15);
    assert!((roots[2] - 1.0).abs() < 1e-15);
}

/// "three unique roots"
#[test]
fn cubic_three_unique_roots() {
    let roots = cubic_real_roots(2.0, 6.0, -26.0, -30.0);
    assert_eq!(roots.len(), 3);
    assert!((roots[0] - (-5.0)).abs() < 1e-15);
    assert!((roots[1] - (-1.0)).abs() < 1e-15);
    assert!((roots[2] - 3.0).abs() < 1e-15);
}

/// "complex roots"
#[test]
fn cubic_complex_roots() {
    let roots = cubic_real_roots(2.0, -6.0, 10.0, -6.0);
    assert_eq!(roots.len(), 1);
    assert!((roots[0] - 1.0).abs() < 1e-15);
}

/// "quadratic case"
#[test]
fn cubic_quadratic_case() {
    let roots = cubic_real_roots(0.0, 2.0, -4.0, -6.0);
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0], -1.0);
    assert_eq!(roots[1], 3.0);
}

/// "deflated case"
#[test]
fn cubic_deflated_case() {
    let roots = cubic_real_roots(1.0, 0.0, 1.0, 2.0);
    assert_eq!(roots.len(), 1);
    assert!((roots[0] - (-1.0)).abs() < 1e-14);

    let roots = cubic_real_roots(1.0, 0.0, 0.0, -8.0);
    assert_eq!(roots.len(), 3);
    assert!((roots[0] - 2.0).abs() < 1e-14);

    let roots = cubic_real_roots(1.0, 0.0, -1.0, 0.0);
    assert_eq!(roots.len(), 3);
    assert_eq!(roots[0], -1.0);
    assert_eq!(roots[1], 0.0);
    assert_eq!(roots[2], 1.0);

    let roots = cubic_real_roots(1.0, 1.0, 0.0, 0.0);
    assert_eq!(roots.len(), 3);
    assert_eq!(roots[0], -1.0);
    assert_eq!(roots[1], 0.0);
    assert_eq!(roots[2], 0.0);

    let roots = cubic_real_roots(1.0, -1.0, 0.0, 0.0);
    assert_eq!(roots.len(), 3);
    assert_eq!(roots[0], 0.0);
    assert_eq!(roots[1], 0.0);
    assert_eq!(roots[2], 1.0);

    let roots = cubic_real_roots(1.0, 1.0, 1.0, 0.0);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0], 0.0);
}

// =============================================================================
// QuarticRealPolynomial
// =============================================================================

/// "discriminant"
#[test]
fn quartic_discriminant_value() {
    let a = 1.0;
    let b = 2.0;
    let c = 3.0;
    let d = 4.0;
    let e = 5.0;

    let a2 = a * a;
    let a3 = a2 * a;
    let b2 = b * b;
    let b3 = b2 * b;
    let c2 = c * c;
    let c3 = c2 * c;
    let d2 = d * d;
    let d3 = d2 * d;
    let e2 = e * e;
    let e3 = e2 * e;

    let expected = b2 * c2 * d2 - 4.0 * b3 * d3 - 4.0 * a * c3 * d2 + 18.0 * a * b * c * d3
        - 27.0 * a2 * d2 * d2
        + 256.0 * a3 * e3
        + e * (18.0 * b3 * c * d - 4.0 * b2 * c3 + 16.0 * a * c2 * c2
            - 80.0 * a * b * c2 * d
            - 6.0 * a * b2 * d2
            + 144.0 * a2 * c * d2)
        + e2 * (144.0 * a * b2 * c - 27.0 * b2 * b2 - 128.0 * a2 * c2 - 192.0 * a2 * b * d);

    let actual = quartic_discriminant(a, b, c, d, e);
    assert_eq!(actual, expected);
}

/// "four repeated roots"
#[test]
fn quartic_four_repeated_roots() {
    let roots = quartic_real_roots(2.0, -16.0, 48.0, -64.0, 32.0);
    assert_eq!(roots.len(), 4);
    assert!((roots[0] - 2.0).abs() < 1e-15);
    assert!((roots[1] - 2.0).abs() < 1e-15);
    assert!((roots[2] - 2.0).abs() < 1e-15);
    assert!((roots[3] - 2.0).abs() < 1e-15);
}

/// "two pairs of repeated roots"
#[test]
fn quartic_two_pairs_repeated() {
    let roots = quartic_real_roots(2.0, 0.0, -4.0, 0.0, 2.0);
    assert_eq!(roots.len(), 4);
    assert!((roots[0] - (-1.0)).abs() < 1e-15);
    assert!((roots[1] - (-1.0)).abs() < 1e-15);
    assert!((roots[2] - 1.0).abs() < 1e-15);
    assert!((roots[3] - 1.0).abs() < 1e-15);
}

/// "one pair of repeated roots"
#[test]
fn quartic_one_pair_repeated() {
    let roots = quartic_real_roots(2.0, -8.0, 16.0, -16.0, 6.0);
    assert_eq!(roots.len(), 2);
    assert!((roots[0] - 1.0).abs() < 1e-14);
    assert!((roots[1] - 1.0).abs() < 1e-14);
}

/// "two unique and one pair of repeated roots"
#[test]
fn quartic_two_unique_one_pair() {
    let roots = quartic_real_roots(2.0, 8.0, -6.0, -20.0, 16.0);
    assert_eq!(roots.len(), 4);
    assert!((roots[0] - (-4.0)).abs() < 1e-15);
    assert!((roots[1] - (-2.0)).abs() < 1e-15);
    assert!((roots[2] - 1.0).abs() < 1e-15);
    assert!((roots[3] - 1.0).abs() < 1e-15);
}

/// "four unique roots"
#[test]
fn quartic_four_unique_roots() {
    let roots = quartic_real_roots(2.0, 4.0, -26.0, -28.0, 48.0);
    assert_eq!(roots.len(), 4);
    assert!((roots[0] - (-4.0)).abs() < 1e-15);
    assert!((roots[1] - (-2.0)).abs() < 1e-15);
    assert!((roots[2] - 1.0).abs() < 1e-15);
    assert!((roots[3] - 3.0).abs() < 1e-15);
}

/// "complex roots"
#[test]
fn quartic_complex_roots() {
    let roots = quartic_real_roots(3.0, -8.0, 14.0, -8.0, 3.0);
    assert_eq!(roots.len(), 0);
}

/// "cubic case"
#[test]
fn quartic_cubic_case() {
    let roots = quartic_real_roots(0.0, 2.0, 6.0, -26.0, -30.0);
    assert_eq!(roots.len(), 3);
    assert!((roots[0] - (-5.0)).abs() < 1e-15);
    assert!((roots[1] - (-1.0)).abs() < 1e-15);
    assert!((roots[2] - 3.0).abs() < 1e-15);
}

/// "stability 1"
#[test]
fn quartic_stability_1() {
    let a = 1.0;
    let b = -27121.309311434146;
    let c = 0.0;
    let d = -26760.571078686513;
    let e = -1.0;

    let expected = [-0.000037368410630733706, 27121.3093478151];
    let actual = quartic_real_roots(a, b, c, d, e);
    assert_eq!(actual.len(), expected.len());
    assert!((actual[0] - expected[0]).abs() < 1e-12);
    assert!((actual[1] - expected[1]).abs() / expected[1].abs() < 1e-12);
}

/// "stability 2"
#[test]
fn quartic_stability_2() {
    let a = -1.0;
    let b = -26959.661445199898;
    let c = 0.0;
    let d = -26675.609408851604;
    let e = 1.0;

    let expected = [-26959.661481901538, 0.000037487427107407711];
    let actual = quartic_real_roots(a, b, c, d, e);
    assert_eq!(actual.len(), expected.len());
    assert!((actual[0] - expected[0]).abs() / expected[0].abs() < 1e-11);
    assert!((actual[1] - expected[1]).abs() < 1e-11);
}

/// "stability 3"
#[test]
fn quartic_stability_3() {
    let a = -1.0;
    let b = 20607.270539372261;
    let c = 0.0;
    let d = 20333.159863900513;
    let e = 1.0;

    let expected = [-0.000049180747737409547, 20607.270587253341];
    let actual = quartic_real_roots(a, b, c, d, e);
    assert_eq!(actual.len(), expected.len());
    assert!((actual[0] - expected[0]).abs() < 1e-11);
    assert!((actual[1] - expected[1]).abs() / expected[1].abs() < 1e-11);
}
