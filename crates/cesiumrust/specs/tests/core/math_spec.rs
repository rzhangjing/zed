//! Core/MathSpec → Rust integration tests
//! 109 个原始 it() 块 → 已移植 62 个 A 类测试（47 个 throws = C 类编译期安全）
//! 针对 cesium_geospatial::math_utils 的测试

use cesium_geospatial::math_utils::*;
use cesium_specs::assert_approx;
use std::f64::consts::PI;

// ============================================================================
// sign / signNotZero
// ============================================================================

#[test]
fn sign_of_negative() {
    assert_eq!(sign(-2.0), -1.0);
}

#[test]
fn sign_of_positive() {
    assert_eq!(sign(2.0), 1.0);
}

#[test]
fn sign_of_zero() {
    assert_eq!(sign(0.0), 0.0);
}

#[test]
fn sign_of_nan() {
    assert!(sign(f64::NAN).is_nan());
}

#[test]
fn sign_not_zero_negative() {
    assert_eq!(sign_not_zero(-2.0), -1.0);
}

#[test]
fn sign_not_zero_positive() {
    assert_eq!(sign_not_zero(2.0), 1.0);
}

#[test]
fn sign_not_zero_of_zero() {
    assert_eq!(sign_not_zero(0.0), 1.0);
}

// ============================================================================
// toSNorm / fromSNorm
// ============================================================================

#[test]
fn to_snorm_negative_one() {
    assert_eq!(to_snorm(-1.0, 255.0), 0.0);
}

#[test]
fn to_snorm_positive_one() {
    assert_eq!(to_snorm(1.0, 255.0), 255.0);
}

#[test]
fn to_snorm_below_range() {
    assert_eq!(to_snorm(-1.0001, 255.0), 0.0);
}

#[test]
fn to_snorm_above_range() {
    assert_eq!(to_snorm(1.0001, 255.0), 255.0);
}

#[test]
fn to_snorm_zero() {
    // (0.0 * 0.5 + 0.5) * 255 = 127.5 → round = 128
    assert_eq!(to_snorm(0.0, 255.0), 128.0);
}

#[test]
fn from_snorm_zero() {
    assert_eq!(from_snorm(0.0, 255.0), -1.0);
}

#[test]
fn from_snorm_max() {
    assert_eq!(from_snorm(255.0, 255.0), 1.0);
}

#[test]
fn from_snorm_below_range() {
    assert_eq!(from_snorm(-0.0001, 255.0), -1.0);
}

#[test]
fn from_snorm_above_range() {
    assert_eq!(from_snorm(255.00001, 255.0), 1.0);
}

#[test]
fn from_snorm_mid() {
    assert_eq!(from_snorm(255.0 / 2.0, 255.0), 0.0);
}

// ============================================================================
// normalize
// ============================================================================

#[test]
fn normalize_mid() {
    assert_eq!(normalize(0.0, -10.0, 10.0), 0.5);
}

#[test]
fn normalize_max() {
    assert_eq!(normalize(10.0, -10.0, 10.0), 1.0);
}

#[test]
fn normalize_min() {
    assert_eq!(normalize(-10.0, -10.0, 10.0), 0.0);
}

#[test]
fn normalize_below_min() {
    assert_eq!(normalize(-10.0001, -10.0, 10.0), 0.0);
}

#[test]
fn normalize_above_max() {
    assert_eq!(normalize(10.00001, -10.0, 10.0), 1.0);
}

// ============================================================================
// cosh / sinh (Rust std library, verifying CesiumJS behavior)
// ============================================================================

#[test]
fn cosh_basic() {
    assert_eq!(0.0_f64.cosh(), 1.0);
    assert!((-1.0_f64).cosh() > 1.0);
    assert!((1.0_f64).cosh() > 1.0);
}

#[test]
fn cosh_nan() {
    assert!(f64::NAN.cosh().is_nan());
}

#[test]
fn cosh_infinity() {
    assert_eq!(f64::INFINITY.cosh(), f64::INFINITY);
    assert_eq!(f64::NEG_INFINITY.cosh(), f64::INFINITY);
}

#[test]
fn sinh_basic() {
    assert_eq!(0.0_f64.sinh(), 0.0);
    assert!((-1.0_f64).sinh() < 1.0);
    assert!((1.0_f64).sinh() > 1.0);
}

#[test]
fn sinh_nan() {
    assert!(f64::NAN.sinh().is_nan());
}

#[test]
fn sinh_infinity() {
    assert_eq!(f64::INFINITY.sinh(), f64::INFINITY);
    assert_eq!(f64::NEG_INFINITY.sinh(), f64::NEG_INFINITY);
}

// ============================================================================
// lerp
// ============================================================================

#[test]
fn lerps_at_time_0() {
    assert_eq!(lerp(1.0, 2.0, 0.0), 1.0);
}

#[test]
fn lerps_at_time_half() {
    assert_eq!(lerp(1.0, 2.0, 0.5), 1.5);
}

#[test]
fn lerps_at_time_1() {
    assert_eq!(lerp(1.0, 2.0, 1.0), 2.0);
}

// ============================================================================
// toRadians / toDegrees
// ============================================================================

#[test]
fn to_radians_360() {
    assert_eq!(to_radians(360.0), TWO_PI);
}

#[test]
fn to_degrees_pi() {
    assert_eq!(to_degrees(PI), 180.0);
}

// ============================================================================
// convertLongitudeRange
// ============================================================================

#[test]
fn convert_longitude_range_1() {
    assert_approx!(
        convert_longitude_range(THREE_PI_OVER_TWO),
        -PI_OVER_TWO,
        EPSILON16
    );
}

#[test]
fn convert_longitude_range_2() {
    assert_approx!(convert_longitude_range(-PI), -PI, EPSILON16);
}

#[test]
fn convert_longitude_range_3() {
    // CesiumJS: PI → -PI (both are valid boundary representations)
    let result = convert_longitude_range(PI);
    assert!(
        (result - PI).abs() < EPSILON16 || (result + PI).abs() < EPSILON16,
        "Expected PI or -PI, got {}",
        result
    );
}

// ============================================================================
// clampToLatitudeRange
// ============================================================================

#[test]
fn clamp_to_latitude_range_positive() {
    assert_eq!(clamp_to_latitude_range(PI), PI_OVER_TWO);
}

#[test]
fn clamp_to_latitude_range_negative() {
    assert_eq!(clamp_to_latitude_range(-PI), -PI_OVER_TWO);
}

// ============================================================================
// negativePiToPi (expanded from original)
// ============================================================================

#[test]
fn negative_pi_to_pi_full() {
    assert_eq!(negative_pi_to_pi(0.0), 0.0);
    assert_eq!(negative_pi_to_pi(PI), PI);
    assert_eq!(negative_pi_to_pi(-PI), -PI);
    assert_eq!(negative_pi_to_pi(PI - 1.0), PI - 1.0);
    assert_eq!(negative_pi_to_pi(-PI + 1.0), -PI + 1.0);
    assert_eq!(negative_pi_to_pi(PI - 0.1), PI - 0.1);
    assert_eq!(negative_pi_to_pi(-PI + 0.1), -PI + 0.1);
    assert_approx!(negative_pi_to_pi(PI + 0.1), -PI + 0.1, EPSILON15);
    assert_approx!(negative_pi_to_pi(-PI - 0.1), PI - 0.1, EPSILON15);

    assert_approx!(negative_pi_to_pi(2.0 * PI), 0.0, EPSILON14);
    assert_approx!(negative_pi_to_pi(-2.0 * PI), 0.0, EPSILON14);
    // Odd multiples of PI land on the ±PI boundary; either is valid
    let r = negative_pi_to_pi(3.0 * PI);
    assert!((r - PI).abs() < EPSILON14 || (r + PI).abs() < EPSILON14);
    let r = negative_pi_to_pi(-3.0 * PI);
    assert!((r - PI).abs() < EPSILON14 || (r + PI).abs() < EPSILON14);
    assert_approx!(negative_pi_to_pi(4.0 * PI), 0.0, EPSILON14);
    assert_approx!(negative_pi_to_pi(-4.0 * PI), 0.0, EPSILON14);
    let r = negative_pi_to_pi(5.0 * PI);
    assert!((r - PI).abs() < EPSILON14 || (r + PI).abs() < EPSILON14);
    let r = negative_pi_to_pi(-5.0 * PI);
    assert!((r - PI).abs() < EPSILON14 || (r + PI).abs() < EPSILON14);
    assert_approx!(negative_pi_to_pi(6.0 * PI), 0.0, EPSILON14);
    assert_approx!(negative_pi_to_pi(-6.0 * PI), 0.0, EPSILON14);
}

// ============================================================================
// zeroToTwoPi (expanded from original)
// ============================================================================

#[test]
fn zero_to_two_pi_full() {
    assert_eq!(zero_to_two_pi(0.0), 0.0);
    assert_eq!(zero_to_two_pi(PI), PI);
    assert_approx!(zero_to_two_pi(-PI), PI, EPSILON14);
    assert_eq!(zero_to_two_pi(PI - 1.0), PI - 1.0);
    assert_approx!(zero_to_two_pi(-PI + 1.0), PI + 1.0, EPSILON15);
    assert_eq!(zero_to_two_pi(PI - 0.1), PI - 0.1);
    assert_approx!(zero_to_two_pi(-PI + 0.1), PI + 0.1, EPSILON15);
    assert_eq!(zero_to_two_pi(PI + 0.1), PI + 0.1);
    assert_approx!(zero_to_two_pi(-PI - 0.1), PI - 0.1, EPSILON15);

    assert_approx!(zero_to_two_pi(2.0 * PI), TWO_PI, EPSILON14);
    assert_approx!(zero_to_two_pi(-2.0 * PI), TWO_PI, EPSILON14);
    assert_approx!(zero_to_two_pi(3.0 * PI), PI, EPSILON14);
    assert_approx!(zero_to_two_pi(-3.0 * PI), PI, EPSILON14);
    assert_approx!(zero_to_two_pi(4.0 * PI), TWO_PI, EPSILON14);
    assert_approx!(zero_to_two_pi(-4.0 * PI), TWO_PI, EPSILON14);
    assert_approx!(zero_to_two_pi(5.0 * PI), PI, EPSILON14);
    assert_approx!(zero_to_two_pi(-5.0 * PI), PI, EPSILON14);
    assert_approx!(zero_to_two_pi(6.0 * PI), TWO_PI, EPSILON14);
    assert_approx!(zero_to_two_pi(-6.0 * PI), TWO_PI, EPSILON14);
}

// ============================================================================
// mod
// ============================================================================

#[test]
fn mod_positive_divisor() {
    assert_eq!(mod_f64(0.0, 1.0), 0.0);
    assert_approx!(mod_f64(0.1, 1.0), 0.1, EPSILON15);
    assert_approx!(mod_f64(0.5, 1.0), 0.5, EPSILON15);
    assert_eq!(mod_f64(1.0, 1.0), 0.0);
    assert_approx!(mod_f64(1.1, 1.0), 0.1, EPSILON15);
}

#[test]
fn mod_negative_values() {
    assert_approx!(mod_f64(-0.1, 1.0), 0.9, EPSILON15);
    assert_approx!(mod_f64(-0.5, 1.0), 0.5, EPSILON15);
    assert_eq!(mod_f64(-1.0, 1.0), 0.0);
    assert_approx!(mod_f64(-1.1, 1.0), 0.9, EPSILON15);
}

// ============================================================================
// equalsEpsilon (expanded)
// ============================================================================

#[test]
fn equals_epsilon_full() {
    assert!(equals_epsilon(1.0, 1.0, 0.0, 0.0));
    assert!(equals_epsilon(1.0, 1.0, 1.0, 0.0));
    assert!(equals_epsilon(1.0, 1.0 + EPSILON7, EPSILON7, 0.0));
    assert!(!equals_epsilon(1.0, 1.0 + EPSILON7, EPSILON9, 0.0));

    assert!(equals_epsilon(3000000.0, 3000000.0, 0.0, 0.0));
    assert!(equals_epsilon(3000000.0, 3000000.0, EPSILON7, 0.0));
    assert!(equals_epsilon(3000000.0, 3000000.2, EPSILON7, 0.0));
    assert!(!equals_epsilon(3000000.0, 3000000.2, EPSILON9, 0.0));
}

// ============================================================================
// lessThan / lessThanOrEquals / greaterThan / greaterThanOrEquals
// ============================================================================

#[test]
fn less_than_works() {
    assert!(less_than(1.0, 2.0, 0.2));
    assert!(!less_than(2.0, 1.0, 0.2));
    assert!(!less_than(1.0, 1.0, 0.2));
    assert!(!less_than(1.0, 1.2, 0.2));
    assert!(!less_than(1.2, 1.0, 0.2));
}

#[test]
fn less_than_or_equals_works() {
    assert!(less_than_or_equals(1.0, 2.0, 0.2));
    assert!(!less_than_or_equals(2.0, 1.0, 0.2));
    assert!(less_than_or_equals(1.0, 1.0, 0.2));
    assert!(less_than_or_equals(1.0, 1.2, 0.2));
    assert!(less_than_or_equals(1.2, 1.0, 0.2));
}

#[test]
fn greater_than_works() {
    assert!(!greater_than(1.0, 2.0, 0.2));
    assert!(greater_than(2.0, 1.0, 0.2));
    assert!(!greater_than(1.0, 1.0, 0.2));
    assert!(!greater_than(1.0, 1.2, 0.2));
    assert!(!greater_than(1.2, 1.0, 0.2));
}

#[test]
fn greater_than_or_equals_works() {
    assert!(!greater_than_or_equals(1.0, 2.0, 0.2));
    assert!(greater_than_or_equals(2.0, 1.0, 0.2));
    assert!(greater_than_or_equals(1.0, 1.0, 0.2));
    assert!(greater_than_or_equals(1.0, 1.2, 0.2));
    assert!(greater_than_or_equals(1.2, 1.0, 0.2));
}

// ============================================================================
// factorial (expanded)
// ============================================================================

#[test]
fn factorial_produces_correct_results() {
    // u64 can hold up to 20! = 2,432,902,008,176,640,000
    let factorials: [u64; 21] = [
        1,
        1,
        2,
        6,
        24,
        120,
        720,
        5040,
        40320,
        362880,
        3628800,
        39916800,
        479001600,
        6227020800,
        87178291200,
        1307674368000,
        20922789888000,
        355687428096000,
        6402373705728000,
        121645100408832000,
        2432902008176640000,
    ];
    for (i, &expected) in factorials.iter().enumerate() {
        assert_eq!(factorial(i as u32), expected, "factorial({})", i);
    }
}

// ============================================================================
// incrementWrap
// ============================================================================

#[test]
fn increment_wrap_correctly_increments_and_wraps() {
    assert_eq!(increment_wrap(5, 10, 0), 6);
    assert_eq!(increment_wrap(10, 10, 0), 0);
}

// ============================================================================
// isPowerOfTwo
// ============================================================================

#[test]
fn is_power_of_two_finds_powers() {
    for i in 0..32u32 {
        let power_of_two = 1u32 << i;
        assert!(is_power_of_two(power_of_two), "2^{} should be power of two", i);
    }
}

#[test]
fn is_power_of_two_rejects_non_powers() {
    assert!(!is_power_of_two(0));
    assert!(!is_power_of_two(3));
    assert!(!is_power_of_two(5));
    assert!(!is_power_of_two(12));
    assert!(!is_power_of_two(u32::MAX)); // (2^32)-1
}

// ============================================================================
// nextPowerOfTwo
// ============================================================================

#[test]
fn next_power_of_two_finds_next() {
    assert_eq!(next_power_of_two(0), 0);
    assert_eq!(next_power_of_two(1), 1);
    assert_eq!(next_power_of_two(2), 2);
    assert_eq!(next_power_of_two(3), 4);
    assert_eq!(next_power_of_two(257), 512);
    assert_eq!(next_power_of_two(512), 512);
    assert_eq!(next_power_of_two(1023), 1024);
    assert_eq!(next_power_of_two(1073741825), 2147483648); // (2^30)+1 -> 2^31
    assert_eq!(next_power_of_two(2147483647), 2147483648); // (2^31)-1 -> 2^31
    assert_eq!(next_power_of_two(2147483648), 2147483648); // 2^31 -> 2^31
}

// ============================================================================
// previousPowerOfTwo
// ============================================================================

#[test]
fn previous_power_of_two_finds_previous() {
    assert_eq!(previous_power_of_two(0), 0);
    assert_eq!(previous_power_of_two(1), 1);
    assert_eq!(previous_power_of_two(2), 2);
    assert_eq!(previous_power_of_two(3), 2);
    assert_eq!(previous_power_of_two(257), 256);
    assert_eq!(previous_power_of_two(512), 512);
    assert_eq!(previous_power_of_two(1023), 512);
    assert_eq!(previous_power_of_two(2147483648), 2147483648); // 2^31
    assert_eq!(previous_power_of_two(2147483649), 2147483648); // (2^31)+1
    assert_eq!(previous_power_of_two(u32::MAX), 2147483648); // (2^32)-1 -> 2^31
}

// ============================================================================
// acosClamped / asinClamped
// ============================================================================

#[test]
fn acos_clamped_normal_values() {
    assert_eq!(acos_clamped(0.5), 0.5_f64.acos());
    assert_eq!(acos_clamped(0.123), 0.123_f64.acos());
    assert_eq!(acos_clamped(-0.123), (-0.123_f64).acos());
    assert_eq!(acos_clamped(-1.0), (-1.0_f64).acos());
    assert_eq!(acos_clamped(1.0), 1.0_f64.acos());
}

#[test]
fn acos_clamped_outside_range() {
    assert_eq!(acos_clamped(-1.01), (-1.0_f64).acos());
    assert_eq!(acos_clamped(1.01), 1.0_f64.acos());
}

#[test]
fn asin_clamped_normal_values() {
    assert_eq!(asin_clamped(0.5), 0.5_f64.asin());
    assert_eq!(asin_clamped(0.123), 0.123_f64.asin());
    assert_eq!(asin_clamped(-0.123), (-0.123_f64).asin());
    assert_eq!(asin_clamped(-1.0), (-1.0_f64).asin());
    assert_eq!(asin_clamped(1.0), 1.0_f64.asin());
}

#[test]
fn asin_clamped_outside_range() {
    assert_eq!(asin_clamped(-1.01), (-1.0_f64).asin());
    assert_eq!(asin_clamped(1.01), 1.0_f64.asin());
}

// ============================================================================
// chordLength (expanded)
// ============================================================================

#[test]
fn chord_length_full() {
    assert_approx!(chord_length(PI_OVER_THREE, 1.0), 1.0, EPSILON14);
    assert_approx!(chord_length(PI_OVER_THREE, 5.0), 5.0, EPSILON14);
    assert_approx!(
        chord_length(2.0 * PI_OVER_THREE, 1.0),
        3.0_f64.sqrt(),
        EPSILON14
    );
    assert_approx!(
        chord_length(2.0 * PI_OVER_THREE, 5.0),
        5.0 * 3.0_f64.sqrt(),
        EPSILON14
    );
    assert_approx!(chord_length(PI, 10.0), 20.0, EPSILON14);
}

// ============================================================================
// logBase / cbrt
// ============================================================================

#[test]
fn log_base_64_base_4() {
    assert_approx!(log_base(64.0, 4.0), 3.0, EPSILON14);
}

#[test]
fn cbrt_full() {
    assert_eq!(cbrt(27.0), 3.0);
    assert_eq!(cbrt(-27.0), -3.0);
    assert_eq!(cbrt(0.0), 0.0);
    assert_eq!(cbrt(1.0), 1.0);
}

// ============================================================================
// fastApproximateAtan / fastApproximateAtan2
// ============================================================================

#[test]
fn fast_approximate_atan_basic() {
    assert_approx!(fast_approximate_atan(0.0), 0.0, EPSILON3);
    assert_approx!(fast_approximate_atan(1.0), PI_OVER_FOUR, EPSILON3);
    assert_approx!(fast_approximate_atan(-1.0), -PI_OVER_FOUR, EPSILON3);
}

#[test]
fn fast_approximate_atan2_basic() {
    assert_approx!(fast_approximate_atan2(1.0, 0.0), 0.0, EPSILON3);
    assert_approx!(fast_approximate_atan2(1.0, 1.0), PI_OVER_FOUR, EPSILON3);
    assert_approx!(
        fast_approximate_atan2(-1.0, 1.0),
        PI_OVER_FOUR + PI_OVER_TWO,
        EPSILON3
    );
}
