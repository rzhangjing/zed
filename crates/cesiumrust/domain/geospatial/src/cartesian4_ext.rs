//! Cartesian4 的扩展函数。
//! 提供超越基础向量运算的静态方法。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::needless_range_loop)]
use crate::math_utils;
use glam::DVec4;

/// 一个 Cartesian4 的打包长度：4。
pub const PACKED_LENGTH: usize = 4;

/// 将一个 Cartesian4 打包到数组中给定的起始索引处。
/// 映射到 CesiumJS `Cartesian4.pack`
pub fn pack(value: DVec4, array: &mut [f64], starting_index: usize) {
    array[starting_index] = value.x;
    array[starting_index + 1] = value.y;
    array[starting_index + 2] = value.z;
    array[starting_index + 3] = value.w;
}

/// 从数组中给定的起始索引处解包出一个 Cartesian4。
/// 映射到 CesiumJS `Cartesian4.unpack`
pub fn unpack(array: &[f64], starting_index: usize) -> DVec4 {
    DVec4::new(
        array[starting_index],
        array[starting_index + 1],
        array[starting_index + 2],
        array[starting_index + 3],
    )
}

/// 将一个 Cartesian4 数组展平为一个分量数组。
/// 映射到 CesiumJS `Cartesian4.packArray`
pub fn pack_array(array: &[DVec4]) -> Vec<f64> {
    let length = array.len();
    let mut result = vec![0.0f64; length * 4];
    for i in 0..length {
        pack(array[i], &mut result, i * 4);
    }
    result
}

/// 将一个分量数组解包为一个 Cartesian4 数组。
/// 映射到 CesiumJS `Cartesian4.unpackArray`
pub fn unpack_array(array: &[f64]) -> Vec<DVec4> {
    let length = array.len() / 4;
    let mut result = Vec::with_capacity(length);
    for i in 0..length {
        result.push(unpack(array, i * 4));
    }
    result
}

/// 在偏移处从数组的前四个元素创建一个 Cartesian4。
/// 映射到 CesiumJS `Cartesian4.fromArray`
pub fn from_array(array: &[f64], starting_index: usize) -> DVec4 {
    DVec4::new(
        array[starting_index],
        array[starting_index + 1],
        array[starting_index + 2],
        array[starting_index + 3],
    )
}

/// 返回具有最大值的分量。
/// 映射到 CesiumJS `Cartesian4.maximumComponent`
pub fn maximum_component(cartesian: DVec4) -> f64 {
    cartesian.x.max(cartesian.y).max(cartesian.z).max(cartesian.w)
}

/// 返回具有最小值的分量。
/// 映射到 CesiumJS `Cartesian4.minimumComponent`
pub fn minimum_component(cartesian: DVec4) -> f64 {
    cartesian.x.min(cartesian.y).min(cartesian.z).min(cartesian.w)
}

/// 计算给定的 Cartesian 的平方量级。
/// 映射到 CesiumJS `Cartesian4.magnitudeSquared`
pub fn magnitude_squared(cartesian: DVec4) -> f64 {
    cartesian.x * cartesian.x
        + cartesian.y * cartesian.y
        + cartesian.z * cartesian.z
        + cartesian.w * cartesian.w
}

/// 计算 Cartesian 的量级（长度）。
/// 映射到 CesiumJS `Cartesian4.magnitude`
pub fn magnitude(cartesian: DVec4) -> f64 {
    magnitude_squared(cartesian).sqrt()
}

/// 计算两点之间的距离。
/// 映射到 CesiumJS `Cartesian4.distance`
pub fn distance(left: DVec4, right: DVec4) -> f64 {
    (left - right).length()
}

/// 计算两点之间的平方距离。
/// 映射到 CesiumJS `Cartesian4.distanceSquared`
pub fn distance_squared(left: DVec4, right: DVec4) -> f64 {
    (left - right).length_squared()
}

/// 使用给定的 cartesians 计算在 t 处的线性插值或外推。
/// 映射到 CesiumJS `Cartesian4.lerp`
pub fn lerp(start: DVec4, end: DVec4, t: f64) -> DVec4 {
    start + (end - start) * t
}

/// 计算两个向量之间的夹角。
/// 映射到 CesiumJS `Cartesian4.angleBetween`
pub fn angle_between(left: DVec4, right: DVec4) -> f64 {
    let dot_val = left.dot(right);
    let magnitude_left_sq = left.dot(left);
    let magnitude_right_sq = right.dot(right);
    let cross_magnitude = (magnitude_left_sq * magnitude_right_sq - dot_val * dot_val)
        .max(0.0)
        .sqrt();
    cross_magnitude.atan2(dot_val)
}

/// 若在给定的 epsilon 范围内 left 与 right 相等则返回 true。
/// 映射到 CesiumJS `Cartesian4.equalsEpsilon`
pub fn equals_epsilon(
    left: DVec4,
    right: DVec4,
    relative_epsilon: f64,
    absolute_epsilon: f64,
) -> bool {
    math_utils::equals_epsilon(left.x, right.x, relative_epsilon, absolute_epsilon)
        && math_utils::equals_epsilon(left.y, right.y, relative_epsilon, absolute_epsilon)
        && math_utils::equals_epsilon(left.z, right.z, relative_epsilon, absolute_epsilon)
        && math_utils::equals_epsilon(left.w, right.w, relative_epsilon, absolute_epsilon)
}

/// 将每个分量约束到给定的 min/max 范围内。
/// 映射到 CesiumJS `Cartesian4.clamp`
pub fn clamp(value: DVec4, min: DVec4, max: DVec4) -> DVec4 {
    DVec4::new(
        math_utils::clamp(value.x, min.x, max.x),
        math_utils::clamp(value.y, min.y, max.y),
        math_utils::clamp(value.z, min.z, max.z),
        math_utils::clamp(value.w, min.w, max.w),
    )
}

/// 计算一个新的 Cartesian4，其每个分量都被设为绝对值。
/// 映射到 CesiumJS `Cartesian4.abs`
pub fn abs(cartesian: DVec4) -> DVec4 {
    DVec4::new(
        cartesian.x.abs(),
        cartesian.y.abs(),
        cartesian.z.abs(),
        cartesian.w.abs(),
    )
}

/// 计算两个 Cartesian 的按分量乘积。
/// 映射到 CesiumJS `Cartesian4.multiplyComponents`
pub fn multiply_components(left: DVec4, right: DVec4) -> DVec4 {
    DVec4::new(
        left.x * right.x,
        left.y * right.y,
        left.z * right.z,
        left.w * right.w,
    )
}

/// 计算两个 Cartesian 的按分量商。
/// 映射到 CesiumJS `Cartesian4.divideComponents`
pub fn divide_components(left: DVec4, right: DVec4) -> DVec4 {
    DVec4::new(
        left.x / right.x,
        left.y / right.y,
        left.z / right.z,
        left.w / right.w,
    )
}
