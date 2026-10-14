//! Matrix4 的扩展函数。
//! 提供超越基础 glam 操作的静态方法。
//! 注意：矩阵以列主序存储，与 glam DMat4 布局一致。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::manual_memcpy)]
use crate::math_utils;
use glam::{DMat3, DMat4, DVec3};

/// 一个 Matrix4 的打包长度：16。
pub const PACKED_LENGTH: usize = 16;

/// 从一个旋转（Matrix3）和平移创建一个 Matrix4。
/// 映射到 CesiumJS `Matrix4.fromRotationTranslation`
///
/// 旋转填入左上 3x3（前三列），平移填入第四列，底部一行为 [0,0,0,1]。
pub fn from_rotation_translation(rotation: &DMat3, translation: DVec3) -> DMat4 {
    DMat4::from_cols_array(&[
        rotation.x_axis.x,
        rotation.x_axis.y,
        rotation.x_axis.z,
        0.0,
        rotation.y_axis.x,
        rotation.y_axis.y,
        rotation.y_axis.z,
        0.0,
        rotation.z_axis.x,
        rotation.z_axis.y,
        rotation.z_axis.z,
        0.0,
        translation.x,
        translation.y,
        translation.z,
        1.0,
    ])
}

/// 从一个平移向量创建一个 Matrix4。
/// 映射到 CesiumJS `Matrix4.fromTranslation`
pub fn from_translation(translation: DVec3) -> DMat4 {
    DMat4::from_cols_array(&[
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        translation.x, translation.y, translation.z, 1.0,
    ])
}

/// 从一个非均匀缩放创建一个 Matrix4。
/// 映射到 CesiumJS `Matrix4.fromScale`
pub fn from_scale(scale: DVec3) -> DMat4 {
    DMat4::from_cols_array(&[
        scale.x, 0.0, 0.0, 0.0,
        0.0, scale.y, 0.0, 0.0,
        0.0, 0.0, scale.z, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ])
}

/// 从一个均匀缩放创建一个 Matrix4。
/// 映射到 CesiumJS `Matrix4.fromUniformScale`
pub fn from_uniform_scale(scale: f64) -> DMat4 {
    from_scale(DVec3::splat(scale))
}

/// 获取一个仿射变换矩阵的平移分量。
/// 映射到 CesiumJS `Matrix4.getTranslation`
pub fn get_translation(matrix: &DMat4) -> DVec3 {
    DVec3::new(matrix.w_axis.x, matrix.w_axis.y, matrix.w_axis.z)
}

/// 获取一个仿射变换矩阵的缩放分量。
/// 映射到 CesiumJS `Matrix4.getScale`
///
/// 取左上 3x3 各列向量的长度作为 X/Y/Z 三轴缩放。
pub fn get_scale(matrix: &DMat4) -> DVec3 {
    let sx = DVec3::new(matrix.x_axis.x, matrix.x_axis.y, matrix.x_axis.z).length();
    let sy = DVec3::new(matrix.y_axis.x, matrix.y_axis.y, matrix.y_axis.z).length();
    let sz = DVec3::new(matrix.z_axis.x, matrix.z_axis.y, matrix.z_axis.z).length();
    DVec3::new(sx, sy, sz)
}

/// 获取一个仿射变换矩阵的最大缩放。
/// 映射到 CesiumJS `Matrix4.getMaximumScale`
pub fn get_maximum_scale(matrix: &DMat4) -> f64 {
    let scale = get_scale(matrix);
    scale.x.max(scale.y).max(scale.z)
}

/// 获取旋转分量（左上 3x3，按缩放归一化）。
/// 映射到 CesiumJS `Matrix4.getRotation`
///
/// 将前三列各除以对应轴的缩放长度，剔除缩放后得到纯旋转子矩阵。
pub fn get_rotation(matrix: &DMat4) -> DMat3 {
    let scale = get_scale(matrix);
    DMat3::from_cols_array(&[
        matrix.x_axis.x / scale.x,
        matrix.x_axis.y / scale.x,
        matrix.x_axis.z / scale.x,
        matrix.y_axis.x / scale.y,
        matrix.y_axis.y / scale.y,
        matrix.y_axis.z / scale.y,
        matrix.z_axis.x / scale.z,
        matrix.z_axis.y / scale.z,
        matrix.z_axis.z / scale.z,
    ])
}

/// 将一个仿射变换矩阵乘以一个隐式平移。
/// 等价于 matrix * fromTranslation(translation)，但更高效。
/// 映射到 CesiumJS `Matrix4.multiplyByTranslation`
pub fn multiply_by_translation(matrix: &DMat4, translation: DVec3) -> DMat4 {
    let x = translation.x;
    let y = translation.y;
    let z = translation.z;

    let tx = x * matrix.x_axis.x
        + y * matrix.y_axis.x
        + z * matrix.z_axis.x
        + matrix.w_axis.x;
    let ty = x * matrix.x_axis.y
        + y * matrix.y_axis.y
        + z * matrix.z_axis.y
        + matrix.w_axis.y;
    let tz = x * matrix.x_axis.z
        + y * matrix.y_axis.z
        + z * matrix.z_axis.z
        + matrix.w_axis.z;

    DMat4::from_cols_array(&[
        matrix.x_axis.x, matrix.x_axis.y, matrix.x_axis.z, matrix.x_axis.w,
        matrix.y_axis.x, matrix.y_axis.y, matrix.y_axis.z, matrix.y_axis.w,
        matrix.z_axis.x, matrix.z_axis.y, matrix.z_axis.z, matrix.z_axis.w,
        tx, ty, tz, matrix.w_axis.w,
    ])
}

/// 将一个仿射变换矩阵乘以一个隐式的非均匀缩放。
/// 映射到 CesiumJS `Matrix4.multiplyByScale`
pub fn multiply_by_scale(matrix: &DMat4, scale: DVec3) -> DMat4 {
    if scale.x == 1.0 && scale.y == 1.0 && scale.z == 1.0 {
        return *matrix;
    }

    DMat4::from_cols_array(&[
        scale.x * matrix.x_axis.x,
        scale.x * matrix.x_axis.y,
        scale.x * matrix.x_axis.z,
        matrix.x_axis.w,
        scale.y * matrix.y_axis.x,
        scale.y * matrix.y_axis.y,
        scale.y * matrix.y_axis.z,
        matrix.y_axis.w,
        scale.z * matrix.z_axis.x,
        scale.z * matrix.z_axis.y,
        scale.z * matrix.z_axis.z,
        matrix.z_axis.w,
        matrix.w_axis.x,
        matrix.w_axis.y,
        matrix.w_axis.z,
        matrix.w_axis.w,
    ])
}

/// 从视场角计算一个透视投影矩阵。
/// 映射到 CesiumJS `Matrix4.computePerspectiveFieldOfView`
///
/// 以半视角正切得到高度缩放，再按宽高比得到宽度缩放；第三列编码 near/far 的深度映射。
pub fn compute_perspective_field_of_view(
    fov_y: f64,
    aspect_ratio: f64,
    near: f64,
    far: f64,
) -> DMat4 {
    let bottom = (fov_y * 0.5).tan();
    let column1_row1 = 1.0 / bottom;
    let column0_row0 = column1_row1 / aspect_ratio;
    let column2_row2 = (far + near) / (near - far);
    let column3_row2 = (2.0 * far * near) / (near - far);

    DMat4::from_cols_array(&[
        column0_row0, 0.0, 0.0, 0.0,
        0.0, column1_row1, 0.0, 0.0,
        0.0, 0.0, column2_row2, -1.0,
        0.0, 0.0, column3_row2, 0.0,
    ])
}

/// 将一个 Matrix4 打包到数组中给定的起始索引处。
/// 映射到 CesiumJS `Matrix4.pack`
pub fn pack(value: &DMat4, array: &mut [f64], starting_index: usize) {
    let cols = value.to_cols_array();
    for (i, &v) in cols.iter().enumerate() {
        array[starting_index + i] = v;
    }
}

/// 从数组中给定的起始索引处解包出一个 Matrix4。
/// 映射到 CesiumJS `Matrix4.unpack`
pub fn unpack(array: &[f64], starting_index: usize) -> DMat4 {
    let mut cols = [0.0f64; 16];
    for i in 0..16 {
        cols[i] = array[starting_index + i];
    }
    DMat4::from_cols_array(&cols)
}

/// 若在给定的 epsilon 范围内 left 与 right 相等则返回 true。
/// 映射到 CesiumJS `Matrix4.equalsEpsilon`
pub fn equals_epsilon(left: &DMat4, right: &DMat4, epsilon: f64) -> bool {
    let l = left.to_cols_array();
    let r = right.to_cols_array();
    l.iter()
        .zip(r.iter())
        .all(|(&a, &b)| math_utils::equals_epsilon(a, b, 0.0, epsilon))
}

/// 从眼睛位置、方向和 up 向量计算一个视图矩阵。
/// 映射到 CesiumJS `Matrix4.computeView`
pub fn compute_view(position: DVec3, direction: DVec3, up: DVec3) -> DMat4 {
    let right = direction.cross(up);
    // 列主序：每一列都是 [right, up, -direction, position] 的转置
    DMat4::from_cols_array(&[
        right.x, up.x, -direction.x, 0.0,
        right.y, up.y, -direction.y, 0.0,
        right.z, up.z, -direction.z, 0.0,
        -right.dot(position), -up.dot(position), direction.dot(position), 1.0,
    ])
}

/// 从平移、四元数旋转和缩放创建一个 Matrix4。
/// 映射到 CesiumJS `Matrix4.fromTranslationQuaternionRotationScale`
pub fn from_translation_quaternion_rotation_scale(
    translation: DVec3,
    rotation: glam::DQuat,
    scale: DVec3,
) -> DMat4 {
    let r = DMat3::from_quat(rotation);
    DMat4::from_cols_array(&[
        r.x_axis.x * scale.x, r.x_axis.y * scale.x, r.x_axis.z * scale.x, 0.0,
        r.y_axis.x * scale.y, r.y_axis.y * scale.y, r.y_axis.z * scale.y, 0.0,
        r.z_axis.x * scale.z, r.z_axis.y * scale.z, r.z_axis.z * scale.z, 0.0,
        translation.x, translation.y, translation.z, 1.0,
    ])
}

/// 相乘两个仿射变换矩阵，忽略第 4 行。
/// 结果的第四行始终为 [0,0,0,1]。
/// 映射到 CesiumJS `Matrix4.multiplyTransformation`
///
/// 前三列只需 3x3 点乘（因右矩阵第 4 行为单位），第 4 列为左矩阵乘以右平移。
pub fn multiply_transformation(left: &DMat4, right: &DMat4) -> DMat4 {
    let l = left.to_cols_array();
    let r = right.to_cols_array();
    let mut out = [0.0f64; 16];
    // 列 0..2：标准 4x4 相乘，但 row3 = [0,0,0,1]
    for col in 0..3 {
        for row in 0..3 {
            out[col * 4 + row] = l[row] * r[col * 4]
                + l[4 + row] * r[col * 4 + 1]
                + l[8 + row] * r[col * 4 + 2];
        }
        out[col * 4 + 3] = 0.0;
    }
    // 列 3（平移）：left * right_col3
    for row in 0..3 {
        out[12 + row] = l[row] * r[12]
            + l[4 + row] * r[13]
            + l[8 + row] * r[14]
            + l[12 + row];
    }
    out[15] = 1.0;
    DMat4::from_cols_array(&out)
}

/// 使用一个 4x4 矩阵变换一个点，将其视为方向（w=0）。
/// 忽略平移。
/// 映射到 CesiumJS `Matrix4.multiplyByPointAsVector`
///
/// 仅用左上 3x3 与向量相乘，因 w=0 而自然丢弃第四列的平移。
pub fn multiply_by_point_as_vector(matrix: &DMat4, point: DVec3) -> DVec3 {
    DVec3::new(
        matrix.x_axis.x * point.x + matrix.y_axis.x * point.y + matrix.z_axis.x * point.z,
        matrix.x_axis.y * point.x + matrix.y_axis.y * point.y + matrix.z_axis.y * point.z,
        matrix.x_axis.z * point.x + matrix.y_axis.z * point.y + matrix.z_axis.z * point.z,
    )
}

/// 计算一个仿射变换矩阵的逆。
/// 对于最后一行为 [0,0,0,1] 的矩阵，比通用求逆更高效。
/// 映射到 CesiumJS `Matrix4.inverseTransformation`
pub fn inverse_transformation(matrix: &DMat4) -> DMat4 {
    // 对于仿射矩阵 [R|t; 0|1]，其逆为 [R^T | -R^T*t; 0 | 1]
    // 提取旋转列
    let col0 = DVec3::new(matrix.x_axis.x, matrix.x_axis.y, matrix.x_axis.z);
    let col1 = DVec3::new(matrix.y_axis.x, matrix.y_axis.y, matrix.y_axis.z);
    let col2 = DVec3::new(matrix.z_axis.x, matrix.z_axis.y, matrix.z_axis.z);
    let t = DVec3::new(matrix.w_axis.x, matrix.w_axis.y, matrix.w_axis.z);

    // R^T 的行就是原始的列
    // new_t = -R^T * t = -(col0.dot(t), col1.dot(t), col2.dot(t))
    let nt = DVec3::new(-col0.dot(t), -col1.dot(t), -col2.dot(t));

    // R^T 以列主序：R^T 的第 i 列 = R 的第 i 行
    // R 的第 0 行 = (col0.x, col1.x, col2.x)
    // R 的第 1 行 = (col0.y, col1.y, col2.y)
    // R 的第 2 行 = (col0.z, col1.z, col2.z)
    DMat4::from_cols_array(&[
        col0.x, col1.x, col2.x, 0.0,
        col0.y, col1.y, col2.y, 0.0,
        col0.z, col1.z, col2.z, 0.0,
        nt.x, nt.y, nt.z, 1.0,
    ])
}

/// 设置矩阵的旋转分量（左上 3x3）。
/// 映射到 CesiumJS `Matrix4.setRotation`
pub fn set_rotation(matrix: &DMat4, rotation: &DMat3) -> DMat4 {
    DMat4::from_cols_array(&[
        rotation.x_axis.x, rotation.x_axis.y, rotation.x_axis.z, matrix.x_axis.w,
        rotation.y_axis.x, rotation.y_axis.y, rotation.y_axis.z, matrix.y_axis.w,
        rotation.z_axis.x, rotation.z_axis.y, rotation.z_axis.z, matrix.z_axis.w,
        matrix.w_axis.x, matrix.w_axis.y, matrix.w_axis.z, matrix.w_axis.w,
    ])
}

/// 设置矩阵的平移分量。
/// 映射到 CesiumJS `Matrix4.setTranslation`
pub fn set_translation(matrix: &DMat4, translation: DVec3) -> DMat4 {
    DMat4::from_cols_array(&[
        matrix.x_axis.x, matrix.x_axis.y, matrix.x_axis.z, matrix.x_axis.w,
        matrix.y_axis.x, matrix.y_axis.y, matrix.y_axis.z, matrix.y_axis.w,
        matrix.z_axis.x, matrix.z_axis.y, matrix.z_axis.z, matrix.z_axis.w,
        translation.x, translation.y, translation.z, matrix.w_axis.w,
    ])
}

/// 设置矩阵的缩放分量（替换左上 3x3 的各列量级）。
/// 映射到 CesiumJS `Matrix4.setScale`
pub fn set_scale(matrix: &DMat4, scale: DVec3) -> DMat4 {
    let current = get_scale(matrix);
    let sx = scale.x / current.x;
    let sy = scale.y / current.y;
    let sz = scale.z / current.z;
    DMat4::from_cols_array(&[
        matrix.x_axis.x * sx, matrix.x_axis.y * sx, matrix.x_axis.z * sx, matrix.x_axis.w,
        matrix.y_axis.x * sy, matrix.y_axis.y * sy, matrix.y_axis.z * sy, matrix.y_axis.w,
        matrix.z_axis.x * sz, matrix.z_axis.y * sz, matrix.z_axis.z * sz, matrix.z_axis.w,
        matrix.w_axis.x, matrix.w_axis.y, matrix.w_axis.z, matrix.w_axis.w,
    ])
}

/// 计算一个正交投影矩阵。
/// 映射到 CesiumJS `Matrix4.computeOrthographicOffCenter`
///
/// 将以 left/right×bottom/top×near/far 定义的长方体线性映射到 NDC 立方体，a/b/c 为各轴半宽。
pub fn compute_orthographic_off_center(
    left: f64, right: f64, bottom: f64, top: f64, near: f64, far: f64,
) -> DMat4 {
    let mut a = 1.0 / (right - left);
    let mut b = 1.0 / (top - bottom);
    let mut c = 1.0 / (far - near);
    let tx = -(right + left) * a;
    let ty = -(top + bottom) * b;
    let tz = -(far + near) * c;
    a *= 2.0;
    b *= 2.0;
    c *= -2.0;

    DMat4::from_cols_array(&[
        a, 0.0, 0.0, 0.0,
        0.0, b, 0.0, 0.0,
        0.0, 0.0, c, 0.0,
        tx, ty, tz, 1.0,
    ])
}

/// 从 off-center 参数计算一个透视投影矩阵。
/// 映射到 CesiumJS `Matrix4.computePerspectiveOffCenter`
pub fn compute_perspective_off_center(
    left: f64, right: f64, bottom: f64, top: f64, near: f64, far: f64,
) -> DMat4 {
    let column0_row0 = (2.0 * near) / (right - left);
    let column1_row1 = (2.0 * near) / (top - bottom);
    let column2_row0 = (right + left) / (right - left);
    let column2_row1 = (top + bottom) / (top - bottom);
    let column2_row2 = -(far + near) / (far - near);
    let column2_row3 = -1.0;
    let column3_row2 = (-2.0 * far * near) / (far - near);

    DMat4::from_cols_array(&[
        column0_row0, 0.0, 0.0, 0.0,
        0.0, column1_row1, 0.0, 0.0,
        column2_row0, column2_row1, column2_row2, column2_row3,
        0.0, 0.0, column3_row2, 0.0,
    ])
}

/// 计算一个无限远透视投影矩阵。
/// 映射到 CesiumJS `Matrix4.computeInfinitePerspectiveOffCenter`
pub fn compute_infinite_perspective_off_center(
    left: f64, right: f64, bottom: f64, top: f64, near: f64,
) -> DMat4 {
    let column0_row0 = (2.0 * near) / (right - left);
    let column1_row1 = (2.0 * near) / (top - bottom);
    let column2_row0 = (right + left) / (right - left);
    let column2_row1 = (top + bottom) / (top - bottom);
    let column2_row2 = -1.0;
    let column2_row3 = -1.0;
    let column3_row2 = -2.0 * near;

    DMat4::from_cols_array(&[
        column0_row0, 0.0, 0.0, 0.0,
        0.0, column1_row1, 0.0, 0.0,
        column2_row0, column2_row1, column2_row2, column2_row3,
        0.0, 0.0, column3_row2, 0.0,
    ])
}

/// 计算一个视口变换矩阵。
/// 映射到 CesiumJS `Matrix4.computeViewportTransformation`
///
/// 将 NDC 的 −1~1 映射到视口像素矩形，深度范围映射到 near~far，平移位于第四列。
pub fn compute_viewport_transformation(
    viewport_x: f64, viewport_y: f64,
    viewport_width: f64, viewport_height: f64,
    near_depth_range: f64, far_depth_range: f64,
) -> DMat4 {
    let half_width = viewport_width * 0.5;
    let half_height = viewport_height * 0.5;
    let half_depth = (far_depth_range - near_depth_range) * 0.5;

    let column0_row0 = half_width;
    let column1_row1 = half_height;
    let column2_row2 = half_depth;
    let column3_row0 = viewport_x + half_width;
    let column3_row1 = viewport_y + half_height;
    let column3_row2 = near_depth_range + half_depth;

    DMat4::from_cols_array(&[
        column0_row0, 0.0, 0.0, 0.0,
        0.0, column1_row1, 0.0, 0.0,
        0.0, 0.0, column2_row2, 0.0,
        column3_row0, column3_row1, column3_row2, 1.0,
    ])
}

/// 计算矩阵的逐元素绝对值。
/// 映射到 CesiumJS `Matrix4.abs`
pub fn abs(matrix: &DMat4) -> DMat4 {
    let cols = matrix.to_cols_array();
    let mut out = [0.0f64; 16];
    for i in 0..16 {
        out[i] = cols[i].abs();
    }
    DMat4::from_cols_array(&out)
}
