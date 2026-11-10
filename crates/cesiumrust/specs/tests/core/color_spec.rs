//! 参考自 `Specs/Core/ColorSpec`（98 个 it()，约 58 个 A 类）
//!
//! 省略 40 个 throws 测试（C 类：Rust 类型系统强制输入合法）。
//! 省略 fromRandom 测试（B 类：随机行为）。

use cesium_geospatial::color::Color;

const EPSILON15: f64 = 1e-15;

fn color_eq_epsilon(a: Color, b: Color, eps: f64) -> bool {
    (a.red - b.red).abs() <= eps
        && (a.green - b.green).abs() <= eps
        && (a.blue - b.blue).abs() <= eps
        && (a.alpha - b.alpha).abs() <= eps
}

#[test]
fn construct_with_default_values() {
    let v = Color::default();
    assert_eq!(v.red, 1.0);
    assert_eq!(v.green, 1.0);
    assert_eq!(v.blue, 1.0);
    assert_eq!(v.alpha, 1.0);
}

#[test]
fn constructing_with_arguments() {
    let v = Color::new(0.1, 0.2, 0.3, 0.4);
    assert_eq!(v.red, 0.1);
    assert_eq!(v.green, 0.2);
    assert_eq!(v.blue, 0.3);
    assert_eq!(v.alpha, 0.4);
}

#[test]
fn from_bytes_with_arguments() {
    let v = Color::from_bytes(0, 255, 51, 102);
    assert_eq!(v.red, 0.0);
    assert_eq!(v.green, 1.0);
    assert_eq!(v.blue, 0.2);
    assert_eq!(v.alpha, 0.4);
}

#[test]
fn to_bytes_returns_same_values() {
    let c = Color::from_bytes(5, 87, 23, 88);
    let bytes = c.to_bytes();
    assert_eq!(bytes, [5, 87, 23, 88]);
}

#[test]
fn to_bytes_works() {
    let color = Color::new(0.1, 0.2, 0.3, 0.4);
    let bytes = color.to_bytes();
    assert_eq!(bytes, [25, 51, 76, 102]);
}

#[test]
fn byte_to_float_works() {
    assert_eq!(Color::byte_to_float(0), 0.0);
    assert_eq!(Color::byte_to_float(255), 1.0);
    assert_eq!(Color::byte_to_float(127), 127.0 / 255.0);
}

#[test]
fn float_to_byte_works() {
    assert_eq!(Color::float_to_byte(0.0), 0);
    assert_eq!(Color::float_to_byte(1.0), 255);
    assert_eq!(Color::float_to_byte(127.0 / 255.0), 127);
}

#[test]
fn from_cartesian4_returns_correct_values() {
    let color = Color::from_cartesian4(1.0, 2.0, 3.0, 4.0);
    assert_eq!(color, Color::new(1.0, 2.0, 3.0, 4.0));
}

#[test]
fn equals_works() {
    let v = Color::new(0.1, 0.2, 0.3, 0.4);
    let v2 = Color::new(0.1, 0.2, 0.3, 0.4);
    let v3 = Color::new(0.1, 0.2, 0.3, 0.5);
    let v4 = Color::new(0.1, 0.2, 0.5, 0.4);
    let v5 = Color::new(0.1, 0.5, 0.3, 0.4);
    let v6 = Color::new(0.5, 0.2, 0.3, 0.4);
    assert!(v == v2);
    assert!(v != v3);
    assert!(v != v4);
    assert!(v != v5);
    assert!(v != v6);
}

#[test]
fn equals_epsilon_works() {
    let v = Color::new(0.1, 0.2, 0.3, 0.4);
    let v2 = Color::new(0.1, 0.2, 0.3, 0.4);
    let v3 = Color::new(0.1, 0.2, 0.3, 0.5);
    let v4 = Color::new(0.1, 0.2, 0.5, 0.4);
    let v5 = Color::new(0.1, 0.5, 0.3, 0.4);
    let v6 = Color::new(0.5, 0.2, 0.3, 0.4);

    assert!(v.equals_epsilon(&v2, 0.0));
    assert!(!v.equals_epsilon(&v3, 0.0));
    assert!(!v.equals_epsilon(&v4, 0.0));
    assert!(!v.equals_epsilon(&v5, 0.0));
    assert!(!v.equals_epsilon(&v6, 0.0));

    assert!(v.equals_epsilon(&v2, 0.1));
    assert!(v.equals_epsilon(&v3, 0.1));
    assert!(v.equals_epsilon(&v4, 0.2));
    assert!(v.equals_epsilon(&v5, 0.3));
    assert!(v.equals_epsilon(&v6, 0.4));
}

#[test]
fn to_css_color_string_produces_expected_output() {
    assert_eq!(Color::WHITE.to_css_color_string(), "rgb(255,255,255)");
    assert_eq!(Color::RED.to_css_color_string(), "rgb(255,0,0)");
    assert_eq!(Color::BLUE.to_css_color_string(), "rgb(0,0,255)");
    assert_eq!(Color::LIME.to_css_color_string(), "rgb(0,255,0)");
    assert_eq!(Color::new(0.0, 0.0, 0.0, 1.0).to_css_color_string(), "rgb(0,0,0)");
    assert_eq!(Color::new(0.1, 0.2, 0.3, 0.4).to_css_color_string(), "rgba(25,51,76,0.4)");
}

#[test]
fn to_css_hex_string_produces_expected_output() {
    assert_eq!(Color::WHITE.to_css_hex_string(), "#ffffff");
    assert_eq!(Color::RED.to_css_hex_string(), "#ff0000");
    assert_eq!(Color::BLUE.to_css_hex_string(), "#0000ff");
    assert_eq!(Color::LIME.to_css_hex_string(), "#00ff00");
    assert_eq!(Color::new(0.0, 0.0, 0.0, 1.0).to_css_hex_string(), "#000000");
    assert_eq!(Color::new(0.1, 0.2, 0.3, 0.4).to_css_hex_string(), "#19334c66");
}

#[test]
fn from_css_color_string_supports_transparent() {
    assert_eq!(Color::from_css_color_string("transparent"), Some(Color::new(0.0, 0.0, 0.0, 0.0)));
}

#[test]
fn from_css_color_string_supports_rgb_format() {
    assert_eq!(Color::from_css_color_string("#369"), Some(Color::new(0.2, 0.4, 0.6, 1.0)));
}

#[test]
fn from_css_color_string_supports_rgb_lowercase() {
    assert_eq!(Color::from_css_color_string("#f00"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("#0f0"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("#00f"), Some(Color::BLUE));
}

#[test]
fn from_css_color_string_supports_rgb_uppercase() {
    assert_eq!(Color::from_css_color_string("#F00"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("#0F0"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("#00F"), Some(Color::BLUE));
}

#[test]
fn from_css_color_string_supports_rgba_format() {
    assert_eq!(Color::from_css_color_string("#369c"), Some(Color::new(0.2, 0.4, 0.6, 0.8)));
}

#[test]
fn from_css_color_string_supports_rgba_uppercase() {
    assert_eq!(Color::from_css_color_string("#369C"), Some(Color::new(0.2, 0.4, 0.6, 0.8)));
}

#[test]
fn from_css_color_string_supports_rrggbb_format() {
    assert_eq!(Color::from_css_color_string("#336699"), Some(Color::new(0.2, 0.4, 0.6, 1.0)));
}

#[test]
fn from_css_color_string_supports_rrggbb_lowercase() {
    assert_eq!(Color::from_css_color_string("#ff0000"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("#00ff00"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("#0000ff"), Some(Color::BLUE));
}

#[test]
fn from_css_color_string_supports_rrggbb_uppercase() {
    assert_eq!(Color::from_css_color_string("#FF0000"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("#00FF00"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("#0000FF"), Some(Color::BLUE));
}

#[test]
fn from_css_color_string_supports_rrggbbaa_format() {
    assert_eq!(Color::from_css_color_string("#336699cc"), Some(Color::new(0.2, 0.4, 0.6, 0.8)));
}

#[test]
fn from_css_color_string_supports_rrggbbaa_uppercase() {
    assert_eq!(Color::from_css_color_string("#336699CC"), Some(Color::new(0.2, 0.4, 0.6, 0.8)));
}

#[test]
fn from_css_color_string_supports_rgb_absolute() {
    assert_eq!(Color::from_css_color_string("rgb(255, 0, 0)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("rgb(0, 255, 0)"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("rgb(0, 0, 255)"), Some(Color::BLUE));
    assert_eq!(Color::from_css_color_string("rgb(51, 102, 204)"), Some(Color::new(0.2, 0.4, 0.8, 1.0)));
}

#[test]
fn from_css_color_string_supports_rgb_absolute_space_separated() {
    assert_eq!(Color::from_css_color_string("rgb(255 0 0)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("rgb(0 255 0)"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("rgb(0 0 255)"), Some(Color::BLUE));
    assert_eq!(Color::from_css_color_string("rgb(51 102 204)"), Some(Color::new(0.2, 0.4, 0.8, 1.0)));
}

#[test]
fn from_css_color_string_supports_rgb_percentages() {
    assert_eq!(Color::from_css_color_string("rgb(100%, 0, 0)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("rgb(0, 100%, 0)"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("rgb(0, 0, 100%)"), Some(Color::BLUE));
    assert_eq!(Color::from_css_color_string("rgb(20%, 40%, 80%)"), Some(Color::new(0.2, 0.4, 0.8, 1.0)));
}

#[test]
fn from_css_color_string_supports_rgb_percentages_space_separated() {
    assert_eq!(Color::from_css_color_string("rgb(100% 0 0)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("rgb(0 100% 0)"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("rgb(0 0 100%)"), Some(Color::BLUE));
    assert_eq!(Color::from_css_color_string("rgb(20% 40% 80%)"), Some(Color::new(0.2, 0.4, 0.8, 1.0)));
}

#[test]
fn from_css_color_string_supports_rgba_absolute() {
    assert_eq!(Color::from_css_color_string("rgba(255, 0, 0, 1.0)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("rgba(0, 255, 0, 1.0)"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("rgba(0, 0, 255, 1.0)"), Some(Color::BLUE));
    assert_eq!(Color::from_css_color_string("rgba(51, 102, 204, 0.6)"), Some(Color::new(0.2, 0.4, 0.8, 0.6)));
}

#[test]
fn from_css_color_string_supports_rgba_absolute_space_separated() {
    assert_eq!(Color::from_css_color_string("rgba(255 0 0 / 1.0)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("rgba(0 255 0 / 1.0)"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("rgba(0 0 255 / 1.0)"), Some(Color::BLUE));
    assert_eq!(Color::from_css_color_string("rgba(51 102 204 / 0.6)"), Some(Color::new(0.2, 0.4, 0.8, 0.6)));
}

#[test]
fn from_css_color_string_supports_rgba_percentages() {
    assert_eq!(Color::from_css_color_string("rgba(100%, 0, 0, 1.0)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("rgba(0, 100%, 0, 1.0)"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("rgba(0, 0, 100%, 1.0)"), Some(Color::BLUE));
    assert_eq!(Color::from_css_color_string("rgba(20%, 40%, 80%, 0.6)"), Some(Color::new(0.2, 0.4, 0.8, 0.6)));
}

#[test]
fn from_css_color_string_supports_rgba_percentages_space_separated() {
    assert_eq!(Color::from_css_color_string("rgba(100% 0 0 / 1.0)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("rgba(0 100% 0 / 1.0)"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("rgba(0 0 100% / 1.0)"), Some(Color::BLUE));
    assert_eq!(Color::from_css_color_string("rgba(20% 40% 80% / 0.6)"), Some(Color::new(0.2, 0.4, 0.8, 0.6)));
}

#[test]
fn from_css_color_string_supports_named_colors() {
    assert_eq!(Color::from_css_color_string("red"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("GREEN"), Some(Color::GREEN));
    assert_eq!(Color::from_css_color_string("BLue"), Some(Color::BLUE));
}

#[test]
fn from_css_color_string_supports_hsl() {
    assert_eq!(Color::from_css_color_string("hsl(0, 100%, 50%)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("hsl(120, 100%, 50%)"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("hsl(240, 100%, 50%)"), Some(Color::BLUE));
    let c = Color::from_css_color_string("hsl(220, 60%, 50%)").unwrap();
    assert!(color_eq_epsilon(c, Color::new(0.2, 0.4, 0.8, 1.0), EPSILON15));
}

#[test]
fn from_css_color_string_supports_hsl_space_separated() {
    assert_eq!(Color::from_css_color_string("hsl(0 100% 50%)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("hsl(120 100% 50%)"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("hsl(240 100% 50%)"), Some(Color::BLUE));
    let c = Color::from_css_color_string("hsl(220 60% 50%)").unwrap();
    assert!(color_eq_epsilon(c, Color::new(0.2, 0.4, 0.8, 1.0), EPSILON15));
}

#[test]
fn from_css_color_string_supports_hsla() {
    assert_eq!(Color::from_css_color_string("hsla(0, 100%, 50%, 1.0)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("hsla(120, 100%, 50%, 1.0)"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("hsla(240, 100%, 50%, 1.0)"), Some(Color::BLUE));
    let c = Color::from_css_color_string("hsla(220, 60%, 50%, 0.6)").unwrap();
    assert!(color_eq_epsilon(c, Color::new(0.2, 0.4, 0.8, 0.6), EPSILON15));
}

#[test]
fn from_css_color_string_supports_hsla_space_separated() {
    assert_eq!(Color::from_css_color_string("hsla(0 100% 50% / 1.0)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("hsla(120 100% 50% / 1.0)"), Some(Color::LIME));
    assert_eq!(Color::from_css_color_string("hsla(240 100% 50% / 1.0)"), Some(Color::BLUE));
    let c = Color::from_css_color_string("hsla(220 60% 50% / 0.6)").unwrap();
    assert!(color_eq_epsilon(c, Color::new(0.2, 0.4, 0.8, 0.6), EPSILON15));
}

#[test]
fn from_css_color_string_wraps_hue() {
    assert_eq!(Color::from_css_color_string("hsl(720, 100%, 50%)"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("hsla(720, 100%, 50%, 1.0)"), Some(Color::RED));
}

#[test]
fn from_css_color_string_returns_none_for_unknown() {
    assert_eq!(Color::from_css_color_string("not a color"), None);
}

#[test]
fn from_css_color_string_handles_spaces() {
    assert_eq!(Color::from_css_color_string(" rgb( 0, 0, 255)"), Some(Color::BLUE));
    assert_eq!(Color::from_css_color_string("rgb( 255, 255, 255) "), Some(Color::WHITE));
    assert_eq!(Color::from_css_color_string("rgb (0 0 255) "), Some(Color::BLUE));
    assert_eq!(Color::from_css_color_string("  #FF0000"), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("#FF0  "), Some(Color::YELLOW));
    assert_eq!(Color::from_css_color_string(" hsla(720,   100%, 50%, 1.0)  "), Some(Color::RED));
    assert_eq!(Color::from_css_color_string("hsl (720, 100%, 50%)"), Some(Color::RED));
}

#[test]
fn from_hsl_produces_expected_output() {
    assert_eq!(Color::from_hsl(0.0, 1.0, 0.5, 1.0), Color::RED);
    assert_eq!(Color::from_hsl(120.0 / 360.0, 1.0, 0.5, 1.0), Color::LIME);
    assert_eq!(Color::from_hsl(240.0 / 360.0, 1.0, 0.5, 1.0), Color::BLUE);
    let c = Color::from_hsl(220.0 / 360.0, 0.6, 0.5, 0.7);
    assert!(color_eq_epsilon(c, Color::new(0.2, 0.4, 0.8, 0.7), EPSILON15));
}

#[test]
fn from_hsl_wraps_hue() {
    assert_eq!(Color::from_hsl(5.0, 1.0, 0.5, 1.0), Color::RED);
}

#[test]
fn from_alpha_works() {
    let result = Color::from_alpha(&Color::RED, 0.5);
    assert_eq!(result, Color::new(1.0, 0.0, 0.0, 0.5));
}

#[test]
fn with_alpha_works() {
    let result = Color::RED.with_alpha(0.5);
    assert_eq!(result, Color::new(1.0, 0.0, 0.0, 0.5));
}

#[test]
fn to_string_produces_correct_results() {
    assert_eq!(Color::new(0.1, 0.2, 0.3, 0.4).to_string(), "(0.1, 0.2, 0.3, 0.4)");
}

#[test]
fn can_convert_to_and_from_rgba() {
    let color = Color::from_bytes(0xff, 0xcc, 0x00, 0xee);
    let rgba = color.to_rgba();
    assert!(rgba > 0);
    let new_color = Color::from_rgba(rgba);
    assert_eq!(color, new_color);
}

#[test]
fn can_brighten() {
    let dark = Color::new(0.2, 0.4, 0.6, 0.8);
    let brighter = dark.brighten(0.5);
    assert_eq!(brighter.red, 0.6);
    assert_eq!(brighter.green, 0.7);
    assert_eq!(brighter.blue, 0.8);
    assert_eq!(brighter.alpha, 0.8);
}

#[test]
fn can_darken() {
    let dark = Color::new(0.1, 0.6, 0.8, 0.8);
    let darker = dark.darken(0.2);
    assert!((darker.red - 0.08).abs() < EPSILON15);
    assert!((darker.green - 0.48).abs() < EPSILON15);
    assert!((darker.blue - 0.64).abs() < EPSILON15);
    assert!((darker.alpha - 0.8).abs() < EPSILON15);
}

#[test]
fn can_add() {
    let left = Color::new(0.1, 0.2, 0.3, 0.4);
    let right = Color::new(0.3, 0.3, 0.3, 0.3);
    let result = left.add(&right);
    assert_eq!(result.red, 0.4);
    assert_eq!(result.green, 0.5);
    assert_eq!(result.blue, 0.6);
    assert_eq!(result.alpha, 0.7);
}

#[test]
fn can_subtract() {
    let left = Color::new(1.0, 1.0, 1.0, 1.0);
    let right = Color::new(0.1, 0.2, 0.3, 0.4);
    let result = left.subtract(&right);
    assert_eq!(result.red, 0.9);
    assert_eq!(result.green, 0.8);
    assert_eq!(result.blue, 0.7);
    assert_eq!(result.alpha, 0.6);
}

#[test]
fn can_multiply() {
    let left = Color::new(0.1, 0.2, 0.3, 0.4);
    let right = Color::new(0.2, 0.2, 0.2, 0.2);
    let result = left.multiply(&right);
    assert!((result.red - 0.02).abs() < EPSILON15);
    assert!((result.green - 0.04).abs() < EPSILON15);
    assert!((result.blue - 0.06).abs() < EPSILON15);
    assert!((result.alpha - 0.08).abs() < EPSILON15);
}

#[test]
fn can_divide() {
    let left = Color::new(0.1, 0.2, 0.1, 0.2);
    let right = Color::new(0.2, 0.2, 0.4, 0.4);
    let result = left.divide(&right);
    assert!((result.red - 0.5).abs() < EPSILON15);
    assert!((result.green - 1.0).abs() < EPSILON15);
    assert!((result.blue - 0.25).abs() < EPSILON15);
    assert!((result.alpha - 0.5).abs() < EPSILON15);
}

#[test]
fn can_mod() {
    let left = Color::new(0.1, 0.2, 0.3, 0.2);
    let right = Color::new(0.2, 0.2, 0.2, 0.4);
    let result = left.modulo(&right);
    assert!((result.red - 0.1).abs() < EPSILON15);
    assert!((result.green - 0.0).abs() < EPSILON15);
    assert!((result.blue - 0.1).abs() < EPSILON15);
    assert!((result.alpha - 0.2).abs() < EPSILON15);
}

#[test]
fn can_multiply_by_scalar() {
    let color = Color::new(0.1, 0.2, 0.3, 0.4);
    let result = color.multiply_by_scalar(2.0);
    assert!((result.red - 0.2).abs() < EPSILON15);
    assert!((result.green - 0.4).abs() < EPSILON15);
    assert!((result.blue - 0.6).abs() < EPSILON15);
    assert!((result.alpha - 0.8).abs() < EPSILON15);
}

#[test]
fn can_divide_by_scalar() {
    let color = Color::new(0.1, 0.2, 0.3, 0.4);
    let result = color.divide_by_scalar(2.0);
    assert!((result.red - 0.05).abs() < EPSILON15);
    assert!((result.green - 0.1).abs() < EPSILON15);
    assert!((result.blue - 0.15).abs() < EPSILON15);
    assert!((result.alpha - 0.2).abs() < EPSILON15);
}

#[test]
fn can_lerp() {
    let color_a = Color::new(0.0, 0.0, 0.0, 0.0);
    let color_b = Color::new(1.0, 1.0, 1.0, 1.0);
    let result = Color::lerp(&color_a, &color_b, 0.5);
    assert!((result.red - 0.5).abs() < EPSILON15);
    assert!((result.green - 0.5).abs() < EPSILON15);
    assert!((result.blue - 0.5).abs() < EPSILON15);
    assert!((result.alpha - 0.5).abs() < EPSILON15);
}

#[test]
fn pack_and_unpack() {
    let color = Color::new(0.1, 0.2, 0.3, 0.4);
    let mut array = [0.0f64; 4];
    color.pack(&mut array, 0);
    assert_eq!(array, [0.1, 0.2, 0.3, 0.4]);
    let unpacked = Color::unpack(&array, 0);
    assert_eq!(unpacked, color);
}
