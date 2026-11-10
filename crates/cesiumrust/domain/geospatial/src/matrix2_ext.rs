//! Matrix2 扩展 —— 2×2 矩阵，以列主序 `[f64; 4]` 表示。
//!
//! 布局（列主序）：`[col0row0, col0row1, col1row0, col1row1]`
//! 即 index = column * 2 + row。

use glam::DVec2;

/// Matrix2 的打包长度。
pub const PACKED_LENGTH: usize = 4;

/// 单位矩阵。
pub const IDENTITY: [f64; 4] = [1.0, 0.0, 0.0, 1.0];

/// 零矩阵。
pub const ZERO: [f64; 4] = [0.0, 0.0, 0.0, 0.0];

// ---------------------------------------------------------------------------
// 打包 / 解包
// ---------------------------------------------------------------------------

/// 将 Matrix2 打包到 `array` 中，从 `starting_index` 开始。
pub fn pack(value: &[f64; 4], array: &mut [f64], starting_index: usize) {
    array[starting_index] = value[0];
    array[starting_index + 1] = value[1];
    array[starting_index + 2] = value[2];
    array[starting_index + 3] = value[3];
}

/// 从 `array` 中解包出一个 Matrix2，从 `starting_index` 开始。
pub fn unpack(array: &[f64], starting_index: usize) -> [f64; 4] {
    [
        array[starting_index],
        array[starting_index + 1],
        array[starting_index + 2],
        array[starting_index + 3],
    ]
}

/// `unpack` 的别名。
pub fn from_array(array: &[f64], starting_index: usize) -> [f64; 4] {
    unpack(array, starting_index)
}

/// 将一组 Matrix2 值打包到一个扁平数组中。
pub fn pack_array(array: &[[f64; 4]], result: &mut Vec<f64>) {
    result.resize(array.len() * 4, 0.0);
    for (i, m) in array.iter().enumerate() {
        pack(m, result, i * 4);
    }
}

/// 将一个扁平数组解包为一组 Matrix2 值。
pub fn unpack_array(array: &[f64]) -> Vec<[f64; 4]> {
    let count = array.len() / 4;
    (0..count).map(|i| unpack(array, i * 4)).collect()
}

// ---------------------------------------------------------------------------
// 构造
// ---------------------------------------------------------------------------

/// 从列主序值创建：`[col0row0, col0row1, col1row0, col1row1]`。
pub fn from_column_major_array(values: &[f64]) -> [f64; 4] {
    [values[0], values[1], values[2], values[3]]
}

/// 从行主序值创建：`[row0col0, row0col1, row1col0, row1col1]`。
pub fn from_row_major_array(values: &[f64]) -> [f64; 4] {
    // 行主序：[r0c0, r0c1, r1c0, r1c1]
    // 列主序：[r0c0, r1c0, r0c1, r1c1]
    [values[0], values[2], values[1], values[3]]
}

/// 从非均匀缩放创建一个缩放矩阵。
pub fn from_scale(scale: DVec2) -> [f64; 4] {
    [scale.x, 0.0, 0.0, scale.y]
}

/// 创建一个均匀缩放矩阵。
pub fn from_uniform_scale(scale: f64) -> [f64; 4] {
    [scale, 0.0, 0.0, scale]
}

/// 从以弧度为单位的角度（逆时针）创建一个 2D 旋转矩阵。
pub fn from_rotation(angle: f64) -> [f64; 4] {
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    // 列主序：col0 = (cos, sin)，col1 = (-sin, cos)
    [cos_a, sin_a, -sin_a, cos_a]
}

// ---------------------------------------------------------------------------
// 元素访问
// ---------------------------------------------------------------------------

/// 获取给定 (column, row) 的扁平索引。
pub fn get_element_index(column: usize, row: usize) -> usize {
    column * 2 + row
}

/// 将一个列以 Cartesian2 形式获取。
pub fn get_column(matrix: &[f64; 4], index: usize) -> DVec2 {
    let start = index * 2;
    DVec2::new(matrix[start], matrix[start + 1])
}

/// 从一个 Cartesian2 设置一列。
pub fn set_column(matrix: &[f64; 4], index: usize, cartesian: DVec2) -> [f64; 4] {
    let mut result = *matrix;
    let start = index * 2;
    result[start] = cartesian.x;
    result[start + 1] = cartesian.y;
    result
}

/// 将一行以 Cartesian2 形式获取。
pub fn get_row(matrix: &[f64; 4], index: usize) -> DVec2 {
    DVec2::new(matrix[index], matrix[index + 2])
}

/// 从一个 Cartesian2 设置一行。
pub fn set_row(matrix: &[f64; 4], index: usize, cartesian: DVec2) -> [f64; 4] {
    let mut result = *matrix;
    result[index] = cartesian.x;
    result[index + 2] = cartesian.y;
    result
}

// ---------------------------------------------------------------------------
// 缩放 / 旋转提取
// ---------------------------------------------------------------------------

/// 设置矩阵的缩放，同时保留旋转。
pub fn set_scale(matrix: &[f64; 4], scale: DVec2) -> [f64; 4] {
    let mut result = *matrix;
    // 缩放第 0 列
    let col0_len = (matrix[0] * matrix[0] + matrix[1] * matrix[1]).sqrt();
    if col0_len > 0.0 {
        result[0] = matrix[0] / col0_len * scale.x;
        result[1] = matrix[1] / col0_len * scale.x;
    }
    // 缩放第 1 列
    let col1_len = (matrix[2] * matrix[2] + matrix[3] * matrix[3]).sqrt();
    if col1_len > 0.0 {
        result[2] = matrix[2] / col1_len * scale.y;
        result[3] = matrix[3] / col1_len * scale.y;
    }
    result
}

/// 设置均匀缩放，同时保留旋转。
pub fn set_uniform_scale(matrix: &[f64; 4], scale: f64) -> [f64; 4] {
    set_scale(matrix, DVec2::splat(scale))
}

/// 从矩阵获取缩放。
pub fn get_scale(matrix: &[f64; 4]) -> DVec2 {
    let sx = (matrix[0] * matrix[0] + matrix[1] * matrix[1]).sqrt();
    let sy = (matrix[2] * matrix[2] + matrix[3] * matrix[3]).sqrt();
    DVec2::new(sx, sy)
}

/// 获取最大的缩放分量。
pub fn get_maximum_scale(matrix: &[f64; 4]) -> f64 {
    let s = get_scale(matrix);
    s.x.max(s.y)
}

/// 设置矩阵的旋转，同时保留缩放。
pub fn set_rotation(matrix: &[f64; 4], rotation: &[f64; 4]) -> [f64; 4] {
    let scale = get_scale(matrix);
    [
        rotation[0] * scale.x,
        rotation[1] * scale.x,
        rotation[2] * scale.y,
        rotation[3] * scale.y,
    ]
}

/// 从矩阵中提取旋转（去除缩放）。
pub fn get_rotation(matrix: &[f64; 4]) -> [f64; 4] {
    let scale = get_scale(matrix);
    let sx = if scale.x > 0.0 { scale.x } else { 1.0 };
    let sy = if scale.y > 0.0 { scale.y } else { 1.0 };
    [
        matrix[0] / sx,
        matrix[1] / sx,
        matrix[2] / sy,
        matrix[3] / sy,
    ]
}

// ---------------------------------------------------------------------------
// 算术运算
// ---------------------------------------------------------------------------

/// 相乘两个 2×2 矩阵：`left * right`。
pub fn multiply(left: &[f64; 4], right: &[f64; 4]) -> [f64; 4] {
    // 列主序：result[col*2+row] = sum_k left[k*2+row] * right[col*2+k]
    [
        left[0] * right[0] + left[2] * right[1],
        left[1] * right[0] + left[3] * right[1],
        left[0] * right[2] + left[2] * right[3],
        left[1] * right[2] + left[3] * right[3],
    ]
}

/// 逐元素相加两个矩阵。
pub fn add(left: &[f64; 4], right: &[f64; 4]) -> [f64; 4] {
    [
        left[0] + right[0],
        left[1] + right[1],
        left[2] + right[2],
        left[3] + right[3],
    ]
}

/// 逐元素相减两个矩阵。
pub fn subtract(left: &[f64; 4], right: &[f64; 4]) -> [f64; 4] {
    [
        left[0] - right[0],
        left[1] - right[1],
        left[2] - right[2],
        left[3] - right[3],
    ]
}

/// 将一个矩阵乘以一个列向量。
pub fn multiply_by_vector(matrix: &[f64; 4], cartesian: DVec2) -> DVec2 {
    DVec2::new(
        matrix[0] * cartesian.x + matrix[2] * cartesian.y,
        matrix[1] * cartesian.x + matrix[3] * cartesian.y,
    )
}

/// 将一个矩阵乘以一个标量。
pub fn multiply_by_scalar(matrix: &[f64; 4], scalar: f64) -> [f64; 4] {
    [
        matrix[0] * scalar,
        matrix[1] * scalar,
        matrix[2] * scalar,
        matrix[3] * scalar,
    ]
}

/// 将一个矩阵乘以一个非均匀缩放（按列）。
pub fn multiply_by_scale(matrix: &[f64; 4], scale: DVec2) -> [f64; 4] {
    [
        matrix[0] * scale.x,
        matrix[1] * scale.x,
        matrix[2] * scale.y,
        matrix[3] * scale.y,
    ]
}

/// 将一个矩阵乘以一个均匀缩放。
pub fn multiply_by_uniform_scale(matrix: &[f64; 4], scale: f64) -> [f64; 4] {
    multiply_by_scalar(matrix, scale)
}

/// 对所有元素取负。
pub fn negate(matrix: &[f64; 4]) -> [f64; 4] {
    [-matrix[0], -matrix[1], -matrix[2], -matrix[3]]
}

/// 转置矩阵。
pub fn transpose(matrix: &[f64; 4]) -> [f64; 4] {
    [matrix[0], matrix[2], matrix[1], matrix[3]]
}

/// 对所有元素取绝对值。
pub fn abs(matrix: &[f64; 4]) -> [f64; 4] {
    [
        matrix[0].abs(),
        matrix[1].abs(),
        matrix[2].abs(),
        matrix[3].abs(),
    ]
}

// ---------------------------------------------------------------------------
// 比较
// ---------------------------------------------------------------------------

/// 精确相等。
pub fn equals(left: &[f64; 4], right: &[f64; 4]) -> bool {
    left[0] == right[0] && left[1] == right[1] && left[2] == right[2] && left[3] == right[3]
}

/// 检查矩阵元素是否与数组在偏移处的元素相等。
pub fn equals_array(matrix: &[f64; 4], array: &[f64], offset: usize) -> bool {
    matrix[0] == array[offset]
        && matrix[1] == array[offset + 1]
        && matrix[2] == array[offset + 2]
        && matrix[3] == array[offset + 3]
}

/// epsilon 相等。
pub fn equals_epsilon(left: &[f64; 4], right: &[f64; 4], epsilon: f64) -> bool {
    (left[0] - right[0]).abs() <= epsilon
        && (left[1] - right[1]).abs() <= epsilon
        && (left[2] - right[2]).abs() <= epsilon
        && (left[3] - right[3]).abs() <= epsilon
}
