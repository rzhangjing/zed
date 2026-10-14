//! Cartesian3 的扩展函数。
//! 提供超越基础向量运算的静态方法。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::manual_is_multiple_of)]
use crate::ellipsoid::Ellipsoid;
use crate::math_utils;
use crate::spherical::Spherical;
use glam::DVec3;

/// 一个 Cartesian3 的打包长度：3。
pub const PACKED_LENGTH: usize = 3;

/// 将 Spherical 坐标转换为 Cartesian3。
/// 映射到 CesiumJS `Cartesian3.fromSpherical`
pub fn from_spherical(spherical: &Spherical) -> DVec3 {
    let clock = spherical.clock;
    let cone = spherical.cone;
    let magnitude = spherical.magnitude;
    // radial 为向赤道平面（xy）的投影长度；z 由 cone 的余弦给出。
    let radial = magnitude * cone.sin();
    DVec3::new(
        radial * clock.cos(),
        radial * clock.sin(),
        magnitude * cone.cos(),
    )
}

/// 返回与给定的 Cartesian 最正交的轴。
/// 映射到 CesiumJS `Cartesian3.mostOrthogonalAxis`
pub fn most_orthogonal_axis(cartesian: DVec3) -> DVec3 {
    // 先归一化并取各分量绝对值，最小的那个分量对应的轴即为最正交轴。
    let f = cartesian.normalize_or_zero();
    let f = DVec3::new(f.x.abs(), f.y.abs(), f.z.abs());

    if f.x <= f.y {
        if f.x <= f.z {
            DVec3::X
        } else {
            DVec3::Z
        }
    } else if f.y <= f.z {
        DVec3::Y
    } else {
        DVec3::Z
    }
}

/// 将向量 a 投影到向量 b 上。
/// 映射到 CesiumJS `Cartesian3.projectVector`
pub fn project_vector(a: DVec3, b: DVec3) -> DVec3 {
    // 标量投影系数 (a·b)/(b·b)，再乘以 b 得到 a 在 b 方向上的投影向量。
    let scalar = a.dot(b) / b.dot(b);
    b * scalar
}

/// 计算 left 与 right 之间的中点。
/// 映射到 CesiumJS `Cartesian3.midpoint`
pub fn midpoint(left: DVec3, right: DVec3) -> DVec3 {
    DVec3::new(
        (left.x + right.x) * 0.5,
        (left.y + right.y) * 0.5,
        (left.z + right.z) * 0.5,
    )
}

/// 若在给定的 epsilon 范围内 left 与 right 相等则返回 true。
/// 映射到 CesiumJS `Cartesian3.equalsEpsilon`
pub fn equals_epsilon(
    left: DVec3,
    right: DVec3,
    relative_epsilon: f64,
    absolute_epsilon: f64,
) -> bool {
    math_utils::equals_epsilon(left.x, right.x, relative_epsilon, absolute_epsilon)
        && math_utils::equals_epsilon(left.y, right.y, relative_epsilon, absolute_epsilon)
        && math_utils::equals_epsilon(left.z, right.z, relative_epsilon, absolute_epsilon)
}

/// 将一个 Cartesian3 打包到数组中给定的起始索引处。
/// 映射到 CesiumJS `Cartesian3.pack`
pub fn pack(value: DVec3, array: &mut [f64], starting_index: usize) {
    array[starting_index] = value.x;
    array[starting_index + 1] = value.y;
    array[starting_index + 2] = value.z;
}

/// 从数组中给定的起始索引处解包出一个 Cartesian3。
/// 映射到 CesiumJS `Cartesian3.unpack`
pub fn unpack(array: &[f64], starting_index: usize) -> DVec3 {
    DVec3::new(
        array[starting_index],
        array[starting_index + 1],
        array[starting_index + 2],
    )
}

/// 根据以度为单位的经度和纬度值返回一个 Cartesian3 位置。
/// 映射到 CesiumJS `Cartesian3.fromDegrees`
pub fn from_degrees(
    longitude: f64,
    latitude: f64,
    height: f64,
    ellipsoid: &Ellipsoid,
) -> DVec3 {
    let lon_rad = math_utils::to_radians(longitude);
    let lat_rad = math_utils::to_radians(latitude);
    from_radians(lon_rad, lat_rad, height, ellipsoid)
}

/// 根据以弧度为单位的经度和纬度值返回一个 Cartesian3 位置。
/// 映射到 CesiumJS `Cartesian3.fromRadians`
pub fn from_radians(
    longitude: f64,
    latitude: f64,
    height: f64,
    ellipsoid: &Ellipsoid,
) -> DVec3 {
    let radii_squared = ellipsoid.radii_squared();

    // n 为大地法线方向（单位）；k 为经半径平方缩放后的同方向向量。
    let cos_latitude = latitude.cos();
    let mut n = DVec3::new(
        cos_latitude * longitude.cos(),
        cos_latitude * longitude.sin(),
        latitude.sin(),
    );
    n = n.normalize_or_zero();

    let k = DVec3::new(
        radii_squared.x * n.x,
        radii_squared.y * n.y,
        radii_squared.z * n.z,
    );
    let gamma = (n.dot(k)).sqrt();
    let k = k / gamma;
    let n = n * height;

    k + n
}

/// 根据以度为单位的 [lon, lat, lon, lat, ...] 数组返回一个 Cartesian3 位置数组。
/// 映射到 CesiumJS `Cartesian3.fromDegreesArray`
pub fn from_degrees_array(coordinates: &[f64], ellipsoid: &Ellipsoid) -> Vec<DVec3> {
    assert!(
        coordinates.len() >= 2 && coordinates.len() % 2 == 0,
        "the number of coordinates must be a multiple of 2 and at least 2"
    );
    let mut result = Vec::with_capacity(coordinates.len() / 2);
    for chunk in coordinates.chunks(2) {
        result.push(from_degrees(chunk[0], chunk[1], 0.0, ellipsoid));
    }
    result
}

/// 根据以弧度为单位的 [lon, lat, lon, lat, ...] 数组返回一个 Cartesian3 位置数组。
/// 映射到 CesiumJS `Cartesian3.fromRadiansArray`
pub fn from_radians_array(coordinates: &[f64], ellipsoid: &Ellipsoid) -> Vec<DVec3> {
    assert!(
        coordinates.len() >= 2 && coordinates.len() % 2 == 0,
        "the number of coordinates must be a multiple of 2 and at least 2"
    );
    let mut result = Vec::with_capacity(coordinates.len() / 2);
    for chunk in coordinates.chunks(2) {
        result.push(from_radians(chunk[0], chunk[1], 0.0, ellipsoid));
    }
    result
}

/// 根据以度为单位的 [lon, lat, height, ...] 返回一个 Cartesian3 位置数组。
/// 映射到 CesiumJS `Cartesian3.fromDegreesArrayHeights`
pub fn from_degrees_array_heights(coordinates: &[f64], ellipsoid: &Ellipsoid) -> Vec<DVec3> {
    assert!(
        coordinates.len() >= 3 && coordinates.len() % 3 == 0,
        "the number of coordinates must be a multiple of 3 and at least 3"
    );
    let mut result = Vec::with_capacity(coordinates.len() / 3);
    for chunk in coordinates.chunks(3) {
        result.push(from_degrees(chunk[0], chunk[1], chunk[2], ellipsoid));
    }
    result
}

/// 根据以弧度为单位的 [lon, lat, height, ...] 返回一个 Cartesian3 位置数组。
/// 映射到 CesiumJS `Cartesian3.fromRadiansArrayHeights`
pub fn from_radians_array_heights(coordinates: &[f64], ellipsoid: &Ellipsoid) -> Vec<DVec3> {
    assert!(
        coordinates.len() >= 3 && coordinates.len() % 3 == 0,
        "the number of coordinates must be a multiple of 3 and at least 3"
    );
    let mut result = Vec::with_capacity(coordinates.len() / 3);
    for chunk in coordinates.chunks(3) {
        result.push(from_radians(chunk[0], chunk[1], chunk[2], ellipsoid));
    }
    result
}

/// 将一个 Cartesian3 转换为 Spherical 坐标。
/// 映射到 CesiumJS `Spherical.fromCartesian3`（已在 spherical.rs 中，为便捷起见在此重新导出）
pub fn to_spherical(cartesian: DVec3) -> Spherical {
    Spherical::from_cartesian3(cartesian)
}
