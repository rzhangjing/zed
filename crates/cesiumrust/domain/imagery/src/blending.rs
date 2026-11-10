//! 多层影像混合。
//!
//! 为多个影像图层实现颜色合成，支持不同的混合模式、昼/夜 alpha
//! 以及分割方向。

use crate::imagery_layer::ImageryLayer;
use crate::AlphaBlendingMode;

/// 线性 RGBA 空间中的像素颜色（每个通道 0.0..1.0）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl PixelColor {
    /// 透明黑。
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    /// 不透明黑。
    pub const BLACK: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };

    /// 不透明白。
    pub const WHITE: Self = Self {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    /// 创建一个新的像素颜色。
    pub fn new(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }

    /// 由 RGB 创建一个不透明颜色。
    pub fn opaque(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b, a: 1.0 }
    }
}

/// 给定昼/夜条件计算图层的有效 alpha。
///
/// 映射到 CesiumJS `ImageryLayer._computeAlpha`
///
/// # 参数
/// * `layer` - 影像图层
/// * `is_day` - 瓦片是否位于地球昼侧
///
/// # 返回
/// 有效 alpha 值
pub fn compute_effective_alpha(layer: &ImageryLayer, is_day: bool) -> f64 {
    let mut alpha = layer.alpha;

    // 应用昼/夜 alpha 调制
    if is_day {
        alpha *= layer.day_alpha;
    } else {
        alpha *= layer.night_alpha;
    }

    alpha.clamp(0.0, 1.0)
}

/// 对像素应用亮度、对比度、色相、饱和度和 gamma 调整。
///
/// 映射到 CesiumJS 影像图层的颜色调整。
///
/// # 参数
/// * `color` - 输入的像素颜色
/// * `layer` - 带调整参数的影像图层
///
/// # 返回
/// 调整后的像素颜色
pub fn apply_color_adjustments(color: PixelColor, layer: &ImageryLayer) -> PixelColor {
    let mut r = color.r;
    let mut g = color.g;
    let mut b = color.b;

    // 应用亮度
    if (layer.brightness - 1.0).abs() > 1e-10 {
        r *= layer.brightness;
        g *= layer.brightness;
        b *= layer.brightness;
    }

    // 应用对比度
    if (layer.contrast - 1.0).abs() > 1e-10 {
        r = apply_contrast(r, layer.contrast);
        g = apply_contrast(g, layer.contrast);
        b = apply_contrast(b, layer.contrast);
    }

    // 应用饱和度
    if (layer.saturation - 1.0).abs() > 1e-10 {
        let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        r = luminance + (r - luminance) * layer.saturation;
        g = luminance + (g - luminance) * layer.saturation;
        b = luminance + (b - luminance) * layer.saturation;
    }

    // 应用 gamma
    if (layer.gamma - 1.0).abs() > 1e-10 {
        let inv_gamma = 1.0 / layer.gamma;
        r = r.max(0.0).powf(inv_gamma);
        g = g.max(0.0).powf(inv_gamma);
        b = b.max(0.0).powf(inv_gamma);
    }

    PixelColor {
        r: r.clamp(0.0, 1.0),
        g: g.clamp(0.0, 1.0),
        b: b.clamp(0.0, 1.0),
        a: color.a,
    }
}

/// 对单个通道应用对比度调整。
fn apply_contrast(value: f64, contrast: f64) -> f64 {
    ((value - 0.5) * contrast + 0.5).clamp(0.0, 1.0)
}

/// 使用指定的混合模式将源像素混合到目标像素上。
///
/// # 参数
/// * `dst` - 目标（背景）像素
/// * `src` - 源（前景）像素
/// * `mode` - alpha 混合模式
/// * `layer_alpha` - 有效的图层 alpha
///
/// # 返回
/// 混合后的像素
pub fn blend_pixel(
    dst: PixelColor,
    src: PixelColor,
    mode: AlphaBlendingMode,
    layer_alpha: f64,
) -> PixelColor {
    let src_alpha = src.a * layer_alpha;

    match mode {
        AlphaBlendingMode::Standard => {
            // 标准 alpha 合成：result = src * src_alpha + dst * (1 - src_alpha)
            let inv_alpha = 1.0 - src_alpha;
            PixelColor {
                r: src.r * src_alpha + dst.r * inv_alpha,
                g: src.g * src_alpha + dst.g * inv_alpha,
                b: src.b * src_alpha + dst.b * inv_alpha,
                a: src_alpha + dst.a * inv_alpha,
            }
        }
        AlphaBlendingMode::Additive => {
            // 叠加混合：result = src * src_alpha + dst
            PixelColor {
                r: (dst.r + src.r * src_alpha).clamp(0.0, 1.0),
                g: (dst.g + src.g * src_alpha).clamp(0.0, 1.0),
                b: (dst.b + src.b * src_alpha).clamp(0.0, 1.0),
                a: (dst.a + src_alpha).clamp(0.0, 1.0),
            }
        }
        AlphaBlendingMode::Multiplicative => {
            // 正片叠底混合：result = src * dst（受 alpha 调制）
            let inv_alpha = 1.0 - src_alpha;
            PixelColor {
                r: src.r * dst.r * src_alpha + dst.r * inv_alpha,
                g: src.g * dst.g * src_alpha + dst.g * inv_alpha,
                b: src.b * dst.b * src_alpha + dst.b * inv_alpha,
                a: src_alpha + dst.a * inv_alpha,
            }
        }
    }
}

/// 从底到顶合成多个影像图层。
///
/// # 参数
/// * `layers` - 按从底到顶顺序排列的图层
/// * `layer_colors` - 来自每个图层纹理的像素颜色
/// * `is_day` - 瓦片是否位于昼侧
/// * `base_color` - 应用任何影像之前的基础（地形）颜色
///
/// # 返回
/// 最终合成的像素颜色
pub fn composite_layers(
    layers: &[&ImageryLayer],
    layer_colors: &[PixelColor],
    is_day: bool,
    base_color: PixelColor,
) -> PixelColor {
    let mut result = base_color;

    for (layer, &src_color) in layers.iter().zip(layer_colors.iter()) {
        if !layer.show {
            continue;
        }

        let effective_alpha = compute_effective_alpha(layer, is_day);
        if effective_alpha < 1e-10 {
            continue;
        }

        // 应用颜色调整
        let adjusted = apply_color_adjustments(src_color, layer);

        // 混合到结果上
        result = blend_pixel(result, adjusted, layer.alpha_blending_mode, effective_alpha);
    }

    result
}

/// 判断对于给定的分割位置，图层是否应被渲染。
///
/// # 参数
/// * `layer` - 影像图层
/// * `split_position` - 归一化的分割位置（0.0 到 1.0）
/// * `tile_center_x` - 瓦片的归一化 X 中心（0.0 到 1.0）
///
/// # 返回
/// 若该图层应为此瓦片渲染则为 True
pub fn should_render_for_split(
    layer: &ImageryLayer,
    split_position: f64,
    tile_center_x: f64,
) -> bool {
    use crate::SplitDirection;

    match layer.split_direction {
        SplitDirection::None => true,
        SplitDirection::Left => tile_center_x <= split_position,
        SplitDirection::Right => tile_center_x > split_position,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_geospatial::rectangle::Rectangle;

    fn create_test_layer() -> ImageryLayer {
        ImageryLayer::new(1, Rectangle::MAX_VALUE)
    }

    #[test]
    fn test_effective_alpha_day() {
        let layer = create_test_layer();
        let alpha = compute_effective_alpha(&layer, true);
        assert!((alpha - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_effective_alpha_night() {
        let mut layer = create_test_layer();
        layer.night_alpha = 0.5;
        let alpha = compute_effective_alpha(&layer, false);
        assert!((alpha - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_effective_alpha_combined() {
        let mut layer = create_test_layer();
        layer.alpha = 0.8;
        layer.day_alpha = 0.5;
        let alpha = compute_effective_alpha(&layer, true);
        assert!((alpha - 0.4).abs() < 1e-10);
    }

    #[test]
    fn test_blend_standard() {
        let dst = PixelColor::opaque(0.0, 0.0, 1.0); // 蓝色背景
        let src = PixelColor::opaque(1.0, 0.0, 0.0); // 红色前景

        let result = blend_pixel(dst, src, AlphaBlendingMode::Standard, 0.5);

        // 50% 红 + 50% 蓝 = 紫
        assert!((result.r - 0.5).abs() < 1e-10);
        assert!((result.b - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_blend_additive() {
        let dst = PixelColor::opaque(0.3, 0.3, 0.3);
        let src = PixelColor::opaque(0.5, 0.0, 0.0);

        let result = blend_pixel(dst, src, AlphaBlendingMode::Additive, 1.0);

        assert!((result.r - 0.8).abs() < 1e-10);
        assert!((result.g - 0.3).abs() < 1e-10);
    }

    #[test]
    fn test_blend_multiplicative() {
        let dst = PixelColor::opaque(0.5, 0.8, 1.0);
        let src = PixelColor::opaque(0.5, 0.5, 0.5);

        let result = blend_pixel(dst, src, AlphaBlendingMode::Multiplicative, 1.0);

        assert!((result.r - 0.25).abs() < 1e-10);
        assert!((result.g - 0.4).abs() < 1e-10);
        assert!((result.b - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_composite_multiple_layers() {
        let layer1 = create_test_layer();
        let layer2 = create_test_layer();

        let layers = vec![&layer1, &layer2];
        let colors = vec![
            PixelColor::opaque(1.0, 0.0, 0.0), // 红
            PixelColor::opaque(0.0, 0.0, 1.0), // 蓝
        ];

        let result = composite_layers(&layers, &colors, true, PixelColor::TRANSPARENT);

        // 图层1（红，alpha=1）叠加到透明 → 红
        // 图层2（蓝，alpha=1）叠加到红 → 蓝（完全覆盖）
        assert!((result.r - 0.0).abs() < 1e-10);
        assert!((result.b - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_composite_with_alpha() {
        let mut layer1 = create_test_layer();
        layer1.alpha = 0.5;
        let layer2 = create_test_layer();

        let layers = vec![&layer1, &layer2];
        let colors = vec![
            PixelColor::opaque(1.0, 0.0, 0.0), // 50% 的红
            PixelColor::new(0.0, 0.0, 1.0, 0.5), // 50% 像素 alpha 的蓝
        ];

        let result = composite_layers(&layers, &colors, true, PixelColor::BLACK);

        // 图层1之后：0.5*红 + 0.5*黑 = (0.5, 0, 0)
        // 图层2之后：0.5*蓝 + 0.5*(0.5, 0, 0) = (0.25, 0, 0.5)
        assert!((result.r - 0.25).abs() < 1e-10);
        assert!((result.b - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_color_adjustments_brightness() {
        let mut layer = create_test_layer();
        layer.brightness = 1.5;

        let color = PixelColor::opaque(0.4, 0.2, 0.6);
        let adjusted = apply_color_adjustments(color, &layer);

        assert!((adjusted.r - 0.6).abs() < 1e-10);
        assert!((adjusted.g - 0.3).abs() < 1e-10);
        assert!((adjusted.b - 0.9).abs() < 1e-10);
    }

    #[test]
    fn test_color_adjustments_saturation() {
        let mut layer = create_test_layer();
        layer.saturation = 0.0; // 完全去饱和

        let color = PixelColor::opaque(1.0, 0.0, 0.0);
        let adjusted = apply_color_adjustments(color, &layer);

        // 纯红的亮度 = 0.2126
        let lum = 0.2126;
        assert!((adjusted.r - lum).abs() < 1e-6);
        assert!((adjusted.g - lum).abs() < 1e-6);
        assert!((adjusted.b - lum).abs() < 1e-6);
    }

    #[test]
    fn test_split_direction() {
        use crate::SplitDirection;

        let mut layer = create_test_layer();

        // 无分割 - 始终渲染
        layer.split_direction = SplitDirection::None;
        assert!(should_render_for_split(&layer, 0.5, 0.3));
        assert!(should_render_for_split(&layer, 0.5, 0.7));

        // 左分割 - 仅渲染分割线左侧
        layer.split_direction = SplitDirection::Left;
        assert!(should_render_for_split(&layer, 0.5, 0.3));
        assert!(!should_render_for_split(&layer, 0.5, 0.7));

        // 右分割 - 仅渲染分割线右侧
        layer.split_direction = SplitDirection::Right;
        assert!(!should_render_for_split(&layer, 0.5, 0.3));
        assert!(should_render_for_split(&layer, 0.5, 0.7));
    }
}
