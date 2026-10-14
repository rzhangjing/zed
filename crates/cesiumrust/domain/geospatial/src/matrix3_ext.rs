//! Matrix3 的扩展函数。
//! 提供超越基础矩阵运算（glam）的静态方法。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::manual_memcpy)]
use glam::{DMat3, DQuat, DVec3};

/// 一个 Matrix3 的打包长度：9。
pub const PACKED_LENGTH: usize = 9;

/// 将一个 Matrix3（列主序）打包到数组中给定的起始索引处。
/// 映射到 CesiumJS `Matrix3.pack`
pub fn pack(value: &DMat3, array: &mut [f64], starting_index: usize) {
    let cols = value.to_cols_array();
    for i in 0..9 {
        array[starting_index + i] = cols[i];
    }
}

/// 从数组中给定的起始索引处，从一个列主序数组解包出一个 Matrix3。
/// 映射到 CesiumJS `Matrix3.unpack`
pub fn unpack(array: &[f64], starting_index: usize) -> DMat3 {
    let mut cols = [0.0f64; 9];
    for i in 0..9 {
        cols[i] = array[starting_index + i];
    }
    DMat3::from_cols_array(&cols)
}

/// 在偏移处从一个列主序数组创建一个 Matrix3。
/// 映射到 CesiumJS `Matrix3.fromColumnMajorArray`
pub fn from_column_major_array(array: &[f64], starting_index: usize) -> DMat3 {
    unpack(array, starting_index)
}

/// 从一个行主序数组创建一个 Matrix3。
/// 映射到 CesiumJS `Matrix3.fromRowMajorArray`
pub fn from_row_major_array(array: &[f64]) -> DMat3 {
    // 行主序：array[row*3+col] → 列主序 DMat3[col][row]
    DMat3::from_cols_array(&[
        array[0], array[3], array[6], // 第 0 列
        array[1], array[4], array[7], // 第 1 列
        array[2], array[5], array[8], // 第 2 列
    ])
}

/// 从一个四元数计算一个 3x3 旋转矩阵。
/// 映射到 CesiumJS `Matrix3.fromQuaternion`
pub fn from_quaternion(quaternion: DQuat) -> DMat3 {
    let x2 = quaternion.x + quaternion.x;
    let y2 = quaternion.y + quaternion.y;
    let z2 = quaternion.z + quaternion.z;

    let xx2 = x2 * quaternion.x;
    let yy2 = y2 * quaternion.y;
    let zz2 = z2 * quaternion.z;
    let xy2 = x2 * quaternion.y;
    let xz2 = x2 * quaternion.z;
    let yz2 = y2 * quaternion.z;
    let wx2 = x2 * quaternion.w;
    let wy2 = y2 * quaternion.w;
    let wz2 = z2 * quaternion.w;

    DMat3::from_cols_array(&[
        1.0 - yy2 - zz2, xy2 + wz2, xz2 - wy2, // 第 0 列
        xy2 - wz2, 1.0 - xx2 - zz2, yz2 + wx2, // 第 1 列
        xz2 + wy2, yz2 - wx2, 1.0 - xx2 - yy2, // 第 2 列
    ])
}

/// 从绕 X 轴的角度计算一个 3x3 旋转矩阵。
/// 映射到 CesiumJS `Matrix3.fromRotationX`
pub fn from_rotation_x(angle: f64) -> DMat3 {
    let cos_angle = angle.cos();
    let sin_angle = angle.sin();
    DMat3::from_cols_array(&[
        1.0, 0.0, 0.0,
        0.0, cos_angle, sin_angle,
        0.0, -sin_angle, cos_angle,
    ])
}

/// 从绕 Y 轴的角度计算一个 3x3 旋转矩阵。
/// 映射到 CesiumJS `Matrix3.fromRotationY`
pub fn from_rotation_y(angle: f64) -> DMat3 {
    let cos_angle = angle.cos();
    let sin_angle = angle.sin();
    DMat3::from_cols_array(&[
        cos_angle, 0.0, -sin_angle,
        0.0, 1.0, 0.0,
        sin_angle, 0.0, cos_angle,
    ])
}

/// 从绕 Z 轴的角度计算一个 3x3 旋转矩阵。
/// 映射到 CesiumJS `Matrix3.fromRotationZ`
pub fn from_rotation_z(angle: f64) -> DMat3 {
    let cos_angle = angle.cos();
    let sin_angle = angle.sin();
    DMat3::from_cols_array(&[
        cos_angle, sin_angle, 0.0,
        -sin_angle, cos_angle, 0.0,
        0.0, 0.0, 1.0,
    ])
}

/// 从一个 DVec3 缩放计算一个 3x3 缩放矩阵。
/// 映射到 CesiumJS `Matrix3.fromScale`
pub fn from_scale(scale: DVec3) -> DMat3 {
    DMat3::from_cols_array(&[
        scale.x, 0.0, 0.0,
        0.0, scale.y, 0.0,
        0.0, 0.0, scale.z,
    ])
}

/// 计算一个 3x3 均匀缩放矩阵。
/// 映射到 CesiumJS `Matrix3.fromUniformScale`
pub fn from_uniform_scale(scale: f64) -> DMat3 {
    DMat3::from_cols_array(&[
        scale, 0.0, 0.0,
        0.0, scale, 0.0,
        0.0, 0.0, scale,
    ])
}

/// 以 DVec3 形式获取索引处矩阵列的副本。
/// 映射到 CesiumJS `Matrix3.getColumn`
pub fn get_column(matrix: &DMat3, index: usize) -> DVec3 {
    match index {
        0 => matrix.x_axis,
        1 => matrix.y_axis,
        2 => matrix.z_axis,
        _ => panic!("index must be 0, 1, or 2"),
    }
}

/// 以 DVec3 形式获取索引处矩阵行的副本。
/// 映射到 CesiumJS `Matrix3.getRow`
pub fn get_row(matrix: &DMat3, index: usize) -> DVec3 {
    let cols = matrix.to_cols_array();
    DVec3::new(cols[index], cols[index + 3], cols[index + 6])
}

/// 假设矩阵为仿射矩阵，计算每列的长度（缩放）。
/// 映射到 CesiumJS `Matrix3.getScale`
pub fn get_scale(matrix: &DMat3) -> DVec3 {
    DVec3::new(
        matrix.x_axis.length(),
        matrix.y_axis.length(),
        matrix.z_axis.length(),
    )
}

/// 假设矩阵为仿射矩阵，计算最大缩放。
/// 映射到 CesiumJS `Matrix3.getMaximumScale`
pub fn get_maximum_scale(matrix: &DMat3) -> f64 {
    let scale = get_scale(matrix);
    scale.x.max(scale.y).max(scale.z)
}

/// 假设矩阵为仿射矩阵，提取旋转矩阵（去除缩放）。
/// 映射到 CesiumJS `Matrix3.getRotation`
pub fn get_rotation(matrix: &DMat3) -> DMat3 {
    let scale = get_scale(matrix);
    DMat3::from_cols(
        matrix.x_axis / scale.x,
        matrix.y_axis / scale.y,
        matrix.z_axis / scale.z,
    )
}

/// 假设矩阵为仿射矩阵，设置旋转（保留缩放）。
/// 映射到 CesiumJS `Matrix3.setRotation`
pub fn set_rotation(matrix: &DMat3, rotation: &DMat3) -> DMat3 {
    let scale = get_scale(matrix);
    DMat3::from_cols(
        rotation.x_axis * scale.x,
        rotation.y_axis * scale.y,
        rotation.z_axis * scale.z,
    )
}

/// 计算两个矩阵之和。
/// 映射到 CesiumJS `Matrix3.add`
pub fn add(left: &DMat3, right: &DMat3) -> DMat3 {
    *left + *right
}

/// 计算两个矩阵之差。
/// 映射到 CesiumJS `Matrix3.subtract`
pub fn subtract(left: &DMat3, right: &DMat3) -> DMat3 {
    *left - *right
}

/// 计算矩阵的逐元素绝对值。
/// 映射到 CesiumJS `Matrix3.abs`
pub fn abs(matrix: &DMat3) -> DMat3 {
    DMat3::from_cols(
        DVec3::new(matrix.x_axis.x.abs(), matrix.x_axis.y.abs(), matrix.x_axis.z.abs()),
        DVec3::new(matrix.y_axis.x.abs(), matrix.y_axis.y.abs(), matrix.y_axis.z.abs()),
        DVec3::new(matrix.z_axis.x.abs(), matrix.z_axis.y.abs(), matrix.z_axis.z.abs()),
    )
}

/// 若在给定的 epsilon 范围内 left 与 right 相等则返回 true。
/// 映射到 CesiumJS `Matrix3.equalsEpsilon`
pub fn equals_epsilon(left: &DMat3, right: &DMat3, epsilon: f64) -> bool {
    let l = left.to_cols_array();
    let r = right.to_cols_array();
    for i in 0..9 {
        if (l[i] - r[i]).abs() > epsilon {
            return false;
        }
    }
    true
}

/// 从一个角度创建一个 2x2 旋转矩阵（以 [f64; 4] 列主序存储）。
/// 映射到 CesiumJS `Matrix2.fromRotation`
pub fn matrix2_from_rotation(angle: f64) -> [f64; 4] {
    let cos_angle = angle.cos();
    let sin_angle = angle.sin();
    // 列主序：[col0row0, col0row1, col1row0, col1row1]
    [cos_angle, sin_angle, -sin_angle, cos_angle]
}

/// 从一个标量创建一个 2x2 缩放矩阵（以 [f64; 4] 列主序存储）。
/// 映射到 CesiumJS `Matrix2.fromScale`
pub fn matrix2_from_scale(scale: f64) -> [f64; 4] {
    [scale, 0.0, 0.0, scale]
}

/// 将一个 2x2 矩阵（列主序）打包到数组中。
/// 映射到 CesiumJS `Matrix2.pack`
pub fn matrix2_pack(value: &[f64; 4], array: &mut [f64], starting_index: usize) {
    array[starting_index] = value[0];
    array[starting_index + 1] = value[1];
    array[starting_index + 2] = value[2];
    array[starting_index + 3] = value[3];
}

/// 从一个列主序数组解包出一个 2x2 矩阵。
/// 映射到 CesiumJS `Matrix2.unpack`
pub fn matrix2_unpack(array: &[f64], starting_index: usize) -> [f64; 4] {
    [
        array[starting_index],
        array[starting_index + 1],
        array[starting_index + 2],
        array[starting_index + 3],
    ]
}
