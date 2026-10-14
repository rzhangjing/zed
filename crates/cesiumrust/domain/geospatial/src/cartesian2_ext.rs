//! Cartesian2 的扩展函数。
//! 提供超越基础向量运算的静态方法。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::needless_range_loop)]
use crate::math_utils;
use glam::DVec2;

/// 一个 Cartesian2 的打包长度：2。
pub const PACKED_LENGTH: usize = 2;

/// 将一个 Cartesian2 打包到数组中给定的起始索引处。
/// 映射到 CesiumJS `Cartesian2.pack`
pub fn pack(value: DVec2, array: &mut [f64], starting_index: usize) {
    array[starting_index] = value.x;
    array[starting_index + 1] = value.y;
}

/// 从数组中给定的起始索引处解包出一个 Cartesian2。
/// 映射到 CesiumJS `Cartesian2.unpack`
pub fn unpack(array: &[f64], starting_index: usize) -> DVec2 {
    DVec2::new(array[starting_index], array[starting_index + 1])
}

/// 将一个 Cartesian2 数组展平为一个分量数组。
/// 映射到 CesiumJS `Cartesian2.packArray`
pub fn pack_array(array: &[DVec2]) -> Vec<f64> {
    let length = array.len();
    let mut result = vec![0.0f64; length * 2];
    for i in 0..length {
        pack(array[i], &mut result, i * 2);
    }
    result
}

/// 将一个分量数组解包为一个 Cartesian2 数组。
/// 映射到 CesiumJS `Cartesian2.unpackArray`
pub fn unpack_array(array: &[f64]) -> Vec<DVec2> {
    let length = array.len() / 2;
    let mut result = Vec::with_capacity(length);
    for i in 0..length {
        result.push(unpack(array, i * 2));
    }
    result
}

/// 在偏移处从数组的前两个元素创建一个 Cartesian2。
/// 映射到 CesiumJS `Cartesian2.fromArray`
pub fn from_array(array: &[f64], starting_index: usize) -> DVec2 {
    DVec2::new(array[starting_index], array[starting_index + 1])
}

/// 返回具有最大值的分量。
/// 映射到 CesiumJS `Cartesian2.maximumComponent`
pub fn maximum_component(cartesian: DVec2) -> f64 {
    cartesian.x.max(cartesian.y)
}

/// 返回具有最小值的分量。
/// 映射到 CesiumJS `Cartesian2.minimumComponent`
pub fn minimum_component(cartesian: DVec2) -> f64 {
    cartesian.x.min(cartesian.y)
}

/// 计算给定的 Cartesian 的平方量级。
/// 映射到 CesiumJS `Cartesian2.magnitudeSquared`
pub fn magnitude_squared(cartesian: DVec2) -> f64 {
    cartesian.x * cartesian.x + cartesian.y * cartesian.y
}

/// 计算 Cartesian 的量级（长度）。
/// 映射到 CesiumJS `Cartesian2.magnitude`
pub fn magnitude(cartesian: DVec2) -> f64 {
    magnitude_squared(cartesian).sqrt()
}

/// 计算两个向量的 2D 叉积（返回标量的 z 分量）。
/// 映射到 CesiumJS `Cartesian2.cross`
pub fn cross(left: DVec2, right: DVec2) -> f64 {
    left.x * right.y - left.y * right.x
}

/// 计算两点之间的距离。
/// 映射到 CesiumJS `Cartesian2.distance`
pub fn distance(left: DVec2, right: DVec2) -> f64 {
    (left - right).length()
}

/// 计算两点之间的平方距离。
/// 映射到 CesiumJS `Cartesian2.distanceSquared`
pub fn distance_squared(left: DVec2, right: DVec2) -> f64 {
    (left - right).length_squared()
}

/// 使用给定的 cartesians 计算在 t 处的线性插值或外推。
/// 映射到 CesiumJS `Cartesian2.lerp`
pub fn lerp(start: DVec2, end: DVec2, t: f64) -> DVec2 {
    start + (end - start) * t
}

/// 计算两个向量之间的夹角。
/// 映射到 CesiumJS `Cartesian2.angleBetween`
pub fn angle_between(left: DVec2, right: DVec2) -> f64 {
    let cross_val = cross(left, right);
    let dot_val = left.dot(right);
    cross_val.abs().atan2(dot_val)
}

/// 返回与给定的 Cartesian 最正交的轴。
/// 映射到 CesiumJS `Cartesian2.mostOrthogonalAxis`
pub fn most_orthogonal_axis(cartesian: DVec2) -> DVec2 {
    let f = cartesian.normalize_or_zero();
    let f = DVec2::new(f.x.abs(), f.y.abs());

    if f.x <= f.y {
        DVec2::X
    } else {
        DVec2::Y
    }
}

/// 若在给定的 epsilon 范围内 left 与 right 相等则返回 true。
/// 映射到 CesiumJS `Cartesian2.equalsEpsilon`
pub fn equals_epsilon(
    left: DVec2,
    right: DVec2,
    relative_epsilon: f64,
    absolute_epsilon: f64,
) -> bool {
    math_utils::equals_epsilon(left.x, right.x, relative_epsilon, absolute_epsilon)
        && math_utils::equals_epsilon(left.y, right.y, relative_epsilon, absolute_epsilon)
}

/// 将每个分量约束到给定的 min/max 范围内。
/// 映射到 CesiumJS `Cartesian2.clamp`
pub fn clamp(value: DVec2, min: DVec2, max: DVec2) -> DVec2 {
    DVec2::new(
        math_utils::clamp(value.x, min.x, max.x),
        math_utils::clamp(value.y, min.y, max.y),
    )
}

/// 计算一个新的 Cartesian2，其每个分量都被设为绝对值。
/// 映射到 CesiumJS `Cartesian2.abs`
pub fn abs(cartesian: DVec2) -> DVec2 {
    DVec2::new(cartesian.x.abs(), cartesian.y.abs())
}

/// 计算两个 Cartesian 的按分量乘积。
/// 映射到 CesiumJS `Cartesian2.multiplyComponents`
pub fn multiply_components(left: DVec2, right: DVec2) -> DVec2 {
    DVec2::new(left.x * right.x, left.y * right.y)
}

/// 计算两个 Cartesian 的按分量商。
/// 映射到 CesiumJS `Cartesian2.divideComponents`
pub fn divide_components(left: DVec2, right: DVec2) -> DVec2 {
    DVec2::new(left.x / right.x, left.y / right.y)
}
