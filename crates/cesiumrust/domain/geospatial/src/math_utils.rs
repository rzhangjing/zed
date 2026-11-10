//! 数学工具 —— 角度、数值常量与辅助函数，
//! 在整个 geospatial 领域中被广泛使用。

use std::f64::consts::PI;

/// PI 常量
pub const PI_F64: f64 = PI;

/// 2 * PI
pub const TWO_PI: f64 = 2.0 * PI;

/// PI / 2
pub const PI_OVER_TWO: f64 = PI / 2.0;

/// PI / 3
pub const PI_OVER_THREE: f64 = PI / 3.0;

/// PI / 4
pub const PI_OVER_FOUR: f64 = PI / 4.0;

/// PI / 6
pub const PI_OVER_SIX: f64 = PI / 6.0;

/// 3 * PI / 2
pub const THREE_PI_OVER_TWO: f64 = 3.0 * PI / 2.0;

/// 月球的平均半径，以米为单位（IAU 2009）。CesiumJS
/// `Ellipsoid.MOON`（一个具有此半径的球体）会使用它。映射到 `CesiumMath.LUNAR_RADIUS`。
pub const LUNAR_RADIUS: f64 = 1737400.0;

/// 1e-1 epsilon（相对容差）
pub const EPSILON1: f64 = 1e-1;
/// 1e-2 epsilon（相对容差）
pub const EPSILON2: f64 = 1e-2;
/// 1e-3 epsilon（相对容差）
pub const EPSILON3: f64 = 1e-3;
/// 1e-4 epsilon（相对容差）
pub const EPSILON4: f64 = 1e-4;
/// 1e-5 epsilon（相对容差）
pub const EPSILON5: f64 = 1e-5;
/// 1e-6 epsilon（相对容差）
pub const EPSILON6: f64 = 1e-6;
/// 1e-7 epsilon（相对容差）
pub const EPSILON7: f64 = 1e-7;
/// 1e-8 epsilon（相对容差）
pub const EPSILON8: f64 = 1e-8;
/// 1e-9 epsilon（相对容差）
pub const EPSILON9: f64 = 1e-9;
/// 1e-10 epsilon（相对容差）
pub const EPSILON10: f64 = 1e-10;
/// 1e-11 epsilon（相对容差）
pub const EPSILON11: f64 = 1e-11;
/// 1e-12 epsilon（相对容差）
pub const EPSILON12: f64 = 1e-12;
/// 1e-13 epsilon（相对容差）
pub const EPSILON13: f64 = 1e-13;
/// 1e-14 epsilon（相对容差）
pub const EPSILON14: f64 = 1e-14;
/// 1e-15 epsilon（相对容差）
pub const EPSILON15: f64 = 1e-15;
/// 1e-16 epsilon（相对容差）
pub const EPSILON16: f64 = 1e-16;
/// 1e-17 epsilon（相对容差）
pub const EPSILON17: f64 = 1e-17;
/// 1e-18 epsilon（相对容差）
pub const EPSILON18: f64 = 1e-18;
/// 1e-19 epsilon（相对容差）
pub const EPSILON19: f64 = 1e-19;
/// 1e-20 epsilon（相对容差）
pub const EPSILON20: f64 = 1e-20;
/// 1e-21 epsilon（相对容差）
pub const EPSILON21: f64 = 1e-21;

/// 用于判断一个值是否为零的数值。
pub const ZERO: f64 = 0.0;

/// 将角度转换为弧度。
/// 映射到 CesiumMath.toRadians
#[inline]
pub fn to_radians(degrees: f64) -> f64 {
    degrees * PI / 180.0
}

/// 将弧度转换为角度。
/// 映射到 CesiumMath.toDegrees
#[inline]
pub fn to_degrees(radians: f64) -> f64 {
    radians * 180.0 / PI
}

/// 将一个值约束在两个值之间。
/// 映射到 CesiumMath.clamp
#[inline]
pub fn clamp(value: f64, min: f64, max: f64) -> f64 {
    value.max(min).min(max)
}

/// 返回值的符号：正为 1，负为 -1，零为 0，NaN 为 NaN。
/// 映射到 CesiumMath.sign
#[inline]
pub fn sign(value: f64) -> f64 {
    if value > 0.0 {
        1.0
    } else if value < 0.0 {
        -1.0
    } else {
        value // 保留 0.0、-0.0 和 NaN
    }
}

/// 使用 signNotZero 返回值的符号：
/// 若 >= 0 为 1，若 < 0 为 -1。
/// 映射到 CesiumMath.signNotZero
#[inline]
pub fn sign_not_zero(value: f64) -> f64 {
    if value < 0.0 { -1.0 } else { 1.0 }
}

/// 在两个值之间线性插值。
/// 映射到 CesiumMath.lerp
#[inline]
pub fn lerp(p: f64, q: f64, time: f64) -> f64 {
    (1.0 - time) * p + time * q
}

/// 返回归一化到 [-PI, PI] 的弧度角。
/// 映射到 CesiumMath.negativePiToPi
pub fn negative_pi_to_pi(angle: f64) -> f64 {
    if (-PI..=PI).contains(&angle) {
        return angle;
    }
    (angle + PI).rem_euclid(TWO_PI) - PI
}

/// 返回归一化到 [0, 2*PI] 的弧度角。
/// 映射到 CesiumMath.zeroToTwoPi
pub fn zero_to_two_pi(angle: f64) -> f64 {
    let mod_val = angle % TWO_PI;
    if (mod_val.abs() < EPSILON14 && angle.abs() > EPSILON14) || mod_val < 0.0 {
        mod_val + TWO_PI
    } else {
        mod_val
    }
}

/// 判断两个值是否在一个 epsilon 范围内相等。
/// 映射到 CesiumMath.equalsEpsilon
#[inline]
pub fn equals_epsilon(left: f64, right: f64, relative_epsilon: f64, absolute_epsilon: f64) -> bool {
    let diff = (left - right).abs();
    diff <= absolute_epsilon || diff <= relative_epsilon * left.abs().max(right.abs())
}

/// 计算一个数的阶乘。
pub fn factorial(n: u32) -> u64 {
    (1..=n as u64).product()
}

/// 给定角度和半径，计算圆的弦长。
/// 映射到 CesiumMath.chordLength
#[inline]
pub fn chord_length(angle: f64, radius: f64) -> f64 {
    2.0 * radius * (angle * 0.5).sin()
}

/// 给定两个向量的量级和点积，计算它们之间夹角的余弦值。
#[inline]
pub fn cos_angle(dot: f64, mag_a: f64, mag_b: f64) -> f64 {
    clamp(dot / (mag_a * mag_b), -1.0, 1.0)
}

/// 将一个以弧度为单位的经度转换到 [-PI, PI] 范围。
#[inline]
pub fn convert_longitude_range(longitude: f64) -> f64 {
    negative_pi_to_pi(longitude)
}

/// 计算一个值以指定底数的对数。
#[inline]
pub fn log_base(value: f64, base: f64) -> f64 {
    value.ln() / base.ln()
}

/// 计算一个数的以 2 为底的对数。
/// 映射到 CesiumMath.log2（`Math.log(number) * Math.LOG2E`）。
#[inline]
pub fn log2(number: f64) -> f64 {
    number.ln() * std::f64::consts::LOG2_E
}

/// 计算一个值的立方根。
#[inline]
pub fn cbrt(value: f64) -> f64 {
    value.cbrt()
}

/// 使用向下取整除法计算除法的余数。
#[inline]
pub fn mod_f64(m: f64, n: f64) -> f64 {
    ((m % n) + n) % n
}

/// 判断一个值是否在给定的 epsilon 范围内接近零。
#[inline]
pub fn is_zero(value: f64) -> bool {
    value.abs() < EPSILON14
}

/// 将 [-1.0, 1.0] 范围内的标量转换为 [0, range_maximum] 范围内的 SNORM。
/// 映射到 CesiumMath.toSNorm
#[inline]
pub fn to_snorm(value: f64, range_maximum: f64) -> f64 {
    ((clamp(value, -1.0, 1.0) * 0.5 + 0.5) * range_maximum).round()
}

/// 将 [0, range_maximum] 范围内的 SNORM 值转换为 [-1.0, 1.0] 范围内的标量。
/// 映射到 CesiumMath.fromSNorm
#[inline]
pub fn from_snorm(value: f64, range_maximum: f64) -> f64 {
    (clamp(value, 0.0, range_maximum) / range_maximum) * 2.0 - 1.0
}

/// 将一个值从 [range_minimum, range_maximum] 归一化到 [0.0, 1.0]。
/// 映射到 CesiumMath.normalize
#[inline]
pub fn normalize(value: f64, range_minimum: f64, range_maximum: f64) -> f64 {
    let range = (range_maximum - range_minimum).max(0.0);
    if range == 0.0 {
        0.0
    } else {
        clamp((value - range_minimum) / range, 0.0, 1.0)
    }
}

/// 将一个角度约束到纬度范围 [-PI/2, PI/2]。
/// 映射到 CesiumMath.clampToLatitudeRange
#[inline]
pub fn clamp_to_latitude_range(angle: f64) -> f64 {
    clamp(angle, -PI_OVER_TWO, PI_OVER_TWO)
}

/// 判断 left < right，将在 epsilon 范围内的值视为相等。
/// 映射到 CesiumMath.lessThan
#[inline]
pub fn less_than(left: f64, right: f64, absolute_epsilon: f64) -> bool {
    left - right < -absolute_epsilon
}

/// 判断 left <= right，将在 epsilon 范围内的值视为相等。
/// 映射到 CesiumMath.lessThanOrEquals
#[inline]
pub fn less_than_or_equals(left: f64, right: f64, absolute_epsilon: f64) -> bool {
    left - right < absolute_epsilon
}

/// 判断 left > right，将在 epsilon 范围内的值视为相等。
/// 映射到 CesiumMath.greaterThan
#[inline]
pub fn greater_than(left: f64, right: f64, absolute_epsilon: f64) -> bool {
    left - right > absolute_epsilon
}

/// 判断 left >= right，将在 epsilon 范围内的值视为相等。
/// 映射到 CesiumMath.greaterThanOrEquals
#[inline]
pub fn greater_than_or_equals(left: f64, right: f64, absolute_epsilon: f64) -> bool {
    left - right > -absolute_epsilon
}

/// 递增 n，当超过 maximum_value 时回绕到 minimum_value。
/// 映射到 CesiumMath.incrementWrap
#[inline]
pub fn increment_wrap(n: i64, maximum_value: i64, minimum_value: i64) -> i64 {
    let n = n + 1;
    if n > maximum_value { minimum_value } else { n }
}

/// 判断一个非负整数是否为 2 的幂。
/// 映射到 CesiumMath.isPowerOfTwo
#[inline]
pub fn is_power_of_two(n: u32) -> bool {
    n != 0 && (n & (n - 1)) == 0
}

/// 计算 >= n 的下一个 2 的幂。
/// 映射到 CesiumMath.nextPowerOfTwo
pub fn next_power_of_two(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut v = n - 1;
    v |= v >> 1;
    v |= v >> 2;
    v |= v >> 4;
    v |= v >> 8;
    v |= v >> 16;
    v + 1
}

/// 计算 <= n 的上一个 2 的幂。
/// 映射到 CesiumMath.previousPowerOfTwo
pub fn previous_power_of_two(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut v = n;
    v |= v >> 1;
    v |= v >> 2;
    v |= v >> 4;
    v |= v >> 8;
    v |= v >> 16;
    v - (v >> 1)
}

/// 计算 acos(clamp(value, -1, 1))，永不返回 NaN。
/// 映射到 CesiumMath.acosClamped
#[inline]
pub fn acos_clamped(value: f64) -> f64 {
    clamp(value, -1.0, 1.0).acos()
}

/// 计算 asin(clamp(value, -1, 1))，永不返回 NaN。
/// 映射到 CesiumMath.asinClamped
#[inline]
pub fn asin_clamped(value: f64) -> f64 {
    clamp(value, -1.0, 1.0).asin()
}

/// 使用多项式逼近的快速近似 atan。
/// 映射到 CesiumMath.fastApproximateAtan
#[inline]
pub fn fast_approximate_atan(x: f64) -> f64 {
    x * (-0.1784 * x.abs() - 0.0663 * x * x + 1.0301)
}

/// 使用范围规约 + fast_approximate_atan 的快速近似 atan2。
/// 映射到 CesiumMath.fastApproximateAtan2
pub fn fast_approximate_atan2(x: f64, y: f64) -> f64 {
    let t = x.abs();
    let opposite = y.abs();
    let adjacent = t.max(opposite);
    let opposite = t.min(opposite);
    let opposite_over_adjacent = opposite / adjacent;
    let mut t = fast_approximate_atan(opposite_over_adjacent);
    // 撤销范围规约
    t = if y.abs() > x.abs() { PI_OVER_TWO - t } else { t };
    t = if x < 0.0 { PI - t } else { t };
    if y < 0.0 { -t } else { t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_radians() {
        assert!((to_radians(180.0) - PI).abs() < EPSILON15);
        assert!((to_radians(90.0) - PI_OVER_TWO).abs() < EPSILON15);
        assert!((to_radians(0.0)).abs() < EPSILON15);
    }

    #[test]
    fn test_to_degrees() {
        assert!((to_degrees(PI) - 180.0).abs() < EPSILON13);
        assert!((to_degrees(PI_OVER_TWO) - 90.0).abs() < EPSILON13);
    }

    #[test]
    fn test_clamp() {
        assert_eq!(clamp(5.0, 0.0, 10.0), 5.0);
        assert_eq!(clamp(-1.0, 0.0, 10.0), 0.0);
        assert_eq!(clamp(15.0, 0.0, 10.0), 10.0);
    }

    #[test]
    fn test_zero_to_two_pi() {
        assert!((zero_to_two_pi(0.0)).abs() < EPSILON14);
        assert!((zero_to_two_pi(TWO_PI) - TWO_PI).abs() < EPSILON14 || zero_to_two_pi(TWO_PI).abs() < EPSILON14);
        assert!((zero_to_two_pi(-PI_OVER_TWO) - THREE_PI_OVER_TWO).abs() < EPSILON14);
    }

    #[test]
    fn test_negative_pi_to_pi() {
        assert!((negative_pi_to_pi(0.0)).abs() < EPSILON14);
        assert!((negative_pi_to_pi(PI) - PI).abs() < EPSILON14);
        assert!((negative_pi_to_pi(THREE_PI_OVER_TWO) - (-PI_OVER_TWO)).abs() < EPSILON14);
    }
}
