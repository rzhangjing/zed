//! Scene/ImageryLayer 混合 → Rust 集成测试。
//!
//! 参考 CesiumJS：
//! - Scene/ImageryLayer（混合、颜色调整、alpha）
//! - Scene/ImageryLayerCollection（图层合成）
//!
//! A 类测试：compute_effective_alpha、apply_color_adjustments、
//! blend_pixel（Standard/Additive/Multiplicative）、composite_layers、
//! should_render_for_split.
//! C 类省略：WebGL 纹理操作、重投影、provider 异步。

use cesium_imagery::blending::{
    apply_color_adjustments, blend_pixel, composite_layers, compute_effective_alpha,
    should_render_for_split, PixelColor,
};
use cesium_imagery::{AlphaBlendingMode, ImageryLayer, SplitDirection};
use cesium_geospatial::rectangle::Rectangle;

fn make_layer() -> ImageryLayer {
    ImageryLayer::new(1, Rectangle::MAX_VALUE)
}

// === PixelColor ===

#[test]
fn pixel_color_constants() {
    assert_eq!(PixelColor::TRANSPARENT.a, 0.0);
    assert_eq!(PixelColor::BLACK.a, 1.0);
    assert_eq!(PixelColor::BLACK.r, 0.0);
    assert_eq!(PixelColor::WHITE.r, 1.0);
    assert_eq!(PixelColor::WHITE.a, 1.0);
}

#[test]
fn pixel_color_opaque() {
    let c = PixelColor::opaque(0.3, 0.6, 0.9);
    assert!((c.r - 0.3).abs() < 1e-10);
    assert!((c.g - 0.6).abs() < 1e-10);
    assert!((c.b - 0.9).abs() < 1e-10);
    assert!((c.a - 1.0).abs() < 1e-10);
}

// === compute_effective_alpha ===

#[test]
fn effective_alpha_default_day() {
    let layer = make_layer();
    let alpha = compute_effective_alpha(&layer, true);
    assert!((alpha - 1.0).abs() < 1e-10);
}

#[test]
fn effective_alpha_night_modulation() {
    let mut layer = make_layer();
    layer.night_alpha = 0.3;
    let alpha = compute_effective_alpha(&layer, false);
    assert!((alpha - 0.3).abs() < 1e-10);
}

#[test]
fn effective_alpha_combined_day() {
    let mut layer = make_layer();
    layer.alpha = 0.8;
    layer.day_alpha = 0.5;
    let alpha = compute_effective_alpha(&layer, true);
    assert!((alpha - 0.4).abs() < 1e-10);
}

#[test]
fn effective_alpha_clamped() {
    let mut layer = make_layer();
    layer.alpha = 2.0;
    layer.day_alpha = 2.0;
    let alpha = compute_effective_alpha(&layer, true);
    assert!((alpha - 1.0).abs() < 1e-10);
}

// === apply_color_adjustments ===

#[test]
fn color_adjust_brightness() {
    let mut layer = make_layer();
    layer.brightness = 1.5;
    let color = PixelColor::opaque(0.4, 0.2, 0.6);
    let adjusted = apply_color_adjustments(color, &layer);
    assert!((adjusted.r - 0.6).abs() < 1e-10);
    assert!((adjusted.g - 0.3).abs() < 1e-10);
    assert!((adjusted.b - 0.9).abs() < 1e-10);
}

#[test]
fn color_adjust_contrast() {
    let mut layer = make_layer();
    layer.contrast = 2.0;
    let color = PixelColor::opaque(0.75, 0.25, 0.5);
    let adjusted = apply_color_adjustments(color, &layer);
    // contrast: (value - 0.5) * 2.0 + 0.5
    // r: (0.75 - 0.5) * 2 + 0.5 = 1.0
    // g: (0.25 - 0.5) * 2 + 0.5 = 0.0
    // b: (0.5 - 0.5) * 2 + 0.5 = 0.5
    assert!((adjusted.r - 1.0).abs() < 1e-10);
    assert!((adjusted.g - 0.0).abs() < 1e-10);
    assert!((adjusted.b - 0.5).abs() < 1e-10);
}

#[test]
fn color_adjust_saturation_zero() {
    let mut layer = make_layer();
    layer.saturation = 0.0;
    let color = PixelColor::opaque(1.0, 0.0, 0.0);
    let adjusted = apply_color_adjustments(color, &layer);
    // luminance = 0.2126*1 + 0.7152*0 + 0.0722*0 = 0.2126
    let lum = 0.2126;
    assert!((adjusted.r - lum).abs() < 1e-6);
    assert!((adjusted.g - lum).abs() < 1e-6);
    assert!((adjusted.b - lum).abs() < 1e-6);
}

#[test]
fn color_adjust_gamma() {
    let mut layer = make_layer();
    layer.gamma = 2.0;
    let color = PixelColor::opaque(0.25, 0.64, 1.0);
    let adjusted = apply_color_adjustments(color, &layer);
    // gamma: value^(1/2) = sqrt(value)
    assert!((adjusted.r - 0.5).abs() < 1e-10);
    assert!((adjusted.g - 0.8).abs() < 1e-10);
    assert!((adjusted.b - 1.0).abs() < 1e-10);
}

#[test]
fn color_adjust_no_change_at_defaults() {
    let layer = make_layer();
    let color = PixelColor::opaque(0.3, 0.5, 0.7);
    let adjusted = apply_color_adjustments(color, &layer);
    assert!((adjusted.r - 0.3).abs() < 1e-10);
    assert!((adjusted.g - 0.5).abs() < 1e-10);
    assert!((adjusted.b - 0.7).abs() < 1e-10);
}

// === blend_pixel ===

#[test]
fn blend_standard_full_alpha() {
    let dst = PixelColor::opaque(0.0, 0.0, 1.0);
    let src = PixelColor::opaque(1.0, 0.0, 0.0);
    let result = blend_pixel(dst, src, AlphaBlendingMode::Standard, 1.0);
    assert!((result.r - 1.0).abs() < 1e-10);
    assert!((result.b - 0.0).abs() < 1e-10);
}

#[test]
fn blend_standard_half_alpha() {
    let dst = PixelColor::opaque(0.0, 0.0, 1.0);
    let src = PixelColor::opaque(1.0, 0.0, 0.0);
    let result = blend_pixel(dst, src, AlphaBlendingMode::Standard, 0.5);
    assert!((result.r - 0.5).abs() < 1e-10);
    assert!((result.b - 0.5).abs() < 1e-10);
}

#[test]
fn blend_standard_zero_alpha() {
    let dst = PixelColor::opaque(0.2, 0.4, 0.6);
    let src = PixelColor::opaque(1.0, 0.0, 0.0);
    let result = blend_pixel(dst, src, AlphaBlendingMode::Standard, 0.0);
    assert!((result.r - 0.2).abs() < 1e-10);
    assert!((result.g - 0.4).abs() < 1e-10);
    assert!((result.b - 0.6).abs() < 1e-10);
}

#[test]
fn blend_additive() {
    let dst = PixelColor::opaque(0.3, 0.3, 0.3);
    let src = PixelColor::opaque(0.5, 0.2, 0.0);
    let result = blend_pixel(dst, src, AlphaBlendingMode::Additive, 1.0);
    assert!((result.r - 0.8).abs() < 1e-10);
    assert!((result.g - 0.5).abs() < 1e-10);
    assert!((result.b - 0.3).abs() < 1e-10);
}

#[test]
fn blend_additive_clamped() {
    let dst = PixelColor::opaque(0.8, 0.9, 0.5);
    let src = PixelColor::opaque(0.5, 0.5, 0.5);
    let result = blend_pixel(dst, src, AlphaBlendingMode::Additive, 1.0);
    assert!((result.r - 1.0).abs() < 1e-10); // 钳制
    assert!((result.g - 1.0).abs() < 1e-10); // 钳制
    assert!((result.b - 1.0).abs() < 1e-10);
}

#[test]
fn blend_multiplicative() {
    let dst = PixelColor::opaque(0.5, 0.8, 1.0);
    let src = PixelColor::opaque(0.5, 0.5, 0.5);
    let result = blend_pixel(dst, src, AlphaBlendingMode::Multiplicative, 1.0);
    assert!((result.r - 0.25).abs() < 1e-10);
    assert!((result.g - 0.4).abs() < 1e-10);
    assert!((result.b - 0.5).abs() < 1e-10);
}

// === composite_layers ===

#[test]
fn composite_single_opaque_layer() {
    let layer = make_layer();
    let layers = vec![&layer];
    let colors = vec![PixelColor::opaque(1.0, 0.0, 0.0)];
    let result = composite_layers(&layers, &colors, true, PixelColor::BLACK);
    assert!((result.r - 1.0).abs() < 1e-10);
    assert!((result.g - 0.0).abs() < 1e-10);
}

#[test]
fn composite_two_layers_top_covers() {
    let layer1 = make_layer();
    let layer2 = make_layer();
    let layers = vec![&layer1, &layer2];
    let colors = vec![
        PixelColor::opaque(1.0, 0.0, 0.0),
        PixelColor::opaque(0.0, 0.0, 1.0),
    ];
    let result = composite_layers(&layers, &colors, true, PixelColor::TRANSPARENT);
    // Layer2（蓝，alpha=1）完全覆盖 layer1（红）
    assert!((result.r - 0.0).abs() < 1e-10);
    assert!((result.b - 1.0).abs() < 1e-10);
}

#[test]
fn composite_hidden_layer_skipped() {
    let mut layer1 = make_layer();
    layer1.show = false;
    let layer2 = make_layer();
    let layers = vec![&layer1, &layer2];
    let colors = vec![
        PixelColor::opaque(1.0, 0.0, 0.0),
        PixelColor::opaque(0.0, 1.0, 0.0),
    ];
    let result = composite_layers(&layers, &colors, true, PixelColor::BLACK);
    // layer1 隐藏，仅应用 layer2（绿）
    assert!((result.g - 1.0).abs() < 1e-10);
    assert!((result.r - 0.0).abs() < 1e-10);
}

#[test]
fn composite_with_semi_transparent() {
    let mut layer = make_layer();
    layer.alpha = 0.5;
    let layers = vec![&layer];
    let colors = vec![PixelColor::opaque(1.0, 0.0, 0.0)];
    let result = composite_layers(&layers, &colors, true, PixelColor::BLACK);
    // 50% 红 + 50% 黑 = (0.5, 0, 0)
    assert!((result.r - 0.5).abs() < 1e-10);
}

// === should_render_for_split ===

#[test]
fn split_none_always_renders() {
    let mut layer = make_layer();
    layer.split_direction = SplitDirection::None;
    assert!(should_render_for_split(&layer, 0.5, 0.0));
    assert!(should_render_for_split(&layer, 0.5, 1.0));
}

#[test]
fn split_left_only_left() {
    let mut layer = make_layer();
    layer.split_direction = SplitDirection::Left;
    assert!(should_render_for_split(&layer, 0.5, 0.3));
    assert!(!should_render_for_split(&layer, 0.5, 0.7));
}

#[test]
fn split_right_only_right() {
    let mut layer = make_layer();
    layer.split_direction = SplitDirection::Right;
    assert!(!should_render_for_split(&layer, 0.5, 0.3));
    assert!(should_render_for_split(&layer, 0.5, 0.7));
}

#[test]
fn split_boundary_exact() {
    let mut layer = make_layer();
    layer.split_direction = SplitDirection::Left;
    // 在精确分割位置，Left 渲染 (<=)
    assert!(should_render_for_split(&layer, 0.5, 0.5));

    layer.split_direction = SplitDirection::Right;
    // 在精确分割位置，Right 不渲染 (>)
    assert!(!should_render_for_split(&layer, 0.5, 0.5));
}
