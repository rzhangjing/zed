//! 垂直夸大工具。

use crate::ellipsoid::Ellipsoid;
use crate::Cartographic;
use glam::DVec3;

/// 按给定的缩放因子，相对于参考高度缩放一个高度。
///
/// `result = (height - relative_height) * scale + relative_height`
///
/// 先将高度平移到以 relative_height 为零点，乘以夸大因子后再平移回去。
pub fn get_height(height: f64, scale: f64, relative_height: f64) -> f64 {
    (height - relative_height) * scale + relative_height
}

/// 相对于参考高度缩放位置的高度分量。
///
/// 将 position 转换为 cartographic，对高度应用垂直夸大，
/// 然后转回 cartesian。
pub fn get_position(
    position: DVec3,
    ellipsoid: &Ellipsoid,
    vertical_exaggeration: f64,
    vertical_exaggeration_relative_height: f64,
) -> DVec3 {
    // 无法转为测绘坐标（如位于椭球中心）时原样返回 position。
    let cartographic = ellipsoid.cartesian_to_cartographic(position);
    match cartographic {
        Some(carto) => {
            let new_height = get_height(
                carto.height,
                vertical_exaggeration,
                vertical_exaggeration_relative_height,
            );
            ellipsoid.cartographic_to_cartesian(&Cartographic::from_radians(
                carto.longitude,
                carto.latitude,
                new_height,
            ))
        }
        None => position,
    }
}

/// 将 sRGB 分量值转换到线性颜色空间。
///
/// 小于阈值时线性除以 12.92，否则按标准 gamma 曲线幂次变换。
pub fn srgb_to_linear(srgb: f64) -> f64 {
    if srgb <= 0.04045 {
        srgb / 12.92
    } else {
        ((srgb + 0.055) / 1.055).powf(2.4)
    }
}
