//! Color —— 带有 CSS 解析、HSL 转换与算术运算的 RGBA 颜色。
//!
//! 颜色以四个 `f64` 分量表示，均归一化到 0.0..1.0：`red`、`green`、`blue`
//! 描述 RGB 通道，`alpha` 描述不透明度。本模块提供：
//!
//! - 与字节（0-255）、u32 打包值、扁平 `f64` 数组之间的相互转换（`pack`/
//!   `unpack`/`to_bytes`/`from_rgba`）；
//! - 对 CSS 颜色字符串的解析与序列化，涵盖十六进制、`rgb()/rgba()`、
//!   `hsl()/hsla()` 函数式记法以及命名颜色关键字；
//! - 逐分量的算术运算（加、减、乘、除、取模、标量缩放）、提亮/加暗，以及
//!   两色之间的线性插值 `lerp`。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::manual_strip)]
use crate::math_utils;

/// 使用红、绿、蓝、 alpha 值（0.0 到 1.0）指定的一种颜色。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// 红色分量，取值 0.0（无）到 1.0（满）。
    pub red: f64,
    /// 绿色分量，取值 0.0 到 1.0。
    pub green: f64,
    /// 蓝色分量，取值 0.0 到 1.0。
    pub blue: f64,
    /// Alpha 不透明度分量，0.0 全透明、1.0 完全不透明。
    pub alpha: f64,
}

impl Default for Color {
    /// 默认颜色：完全不透明的纯白 (1, 1, 1, 1)。
    fn default() -> Self {
        Self { red: 1.0, green: 1.0, blue: 1.0, alpha: 1.0 }
    }
}

impl Color {
    /// 由四个归一化分量直接构造一个颜色。
    ///
    /// # 参数
    /// - `red`/`green`/`blue`：红绿蓝分量，取值 0.0 到 1.0。
    /// - `alpha`：不透明度，0.0 全透明、1.0 完全不透明。
    ///
    /// # 返回
    /// 各分量取给定值的 `Color`；调用方负责保证分量落在合法区间。
    pub fn new(red: f64, green: f64, blue: f64, alpha: f64) -> Self {
        Self { red, green, blue, alpha }
    }

    // --- 命名颜色常量（测试中使用的一部分 + 常见颜色） ---
    //
    // 以下每个常量都是一个完全不透明（alpha = 1.0）的预定义颜色，分量以
    // 0.0..1.0 的浮点表示，与 CSS 颜色关键字一一对应（`named_color` 即据
    // 此按名称返回常量）。数值如 0.5019607843137255 等于 128/255，来自对
    // 应 0-255 色标的归一化。仅收录常见与测试所需的名字，并非完整 CSS 列表。
    pub const WHITE: Self = Self { red: 1.0, green: 1.0, blue: 1.0, alpha: 1.0 };
    pub const BLACK: Self = Self { red: 0.0, green: 0.0, blue: 0.0, alpha: 1.0 };
    pub const RED: Self = Self { red: 1.0, green: 0.0, blue: 0.0, alpha: 1.0 };
    pub const GREEN: Self = Self { red: 0.0, green: 0.5019607843137255, blue: 0.0, alpha: 1.0 };
    pub const LIME: Self = Self { red: 0.0, green: 1.0, blue: 0.0, alpha: 1.0 };
    pub const BLUE: Self = Self { red: 0.0, green: 0.0, blue: 1.0, alpha: 1.0 };
    pub const YELLOW: Self = Self { red: 1.0, green: 1.0, blue: 0.0, alpha: 1.0 };
    pub const CYAN: Self = Self { red: 0.0, green: 1.0, blue: 1.0, alpha: 1.0 };
    pub const MAGENTA: Self = Self { red: 1.0, green: 0.0, blue: 1.0, alpha: 1.0 };
    pub const TRANSPARENT: Self = Self { red: 0.0, green: 0.0, blue: 0.0, alpha: 0.0 };
    pub const ORANGE: Self = Self { red: 1.0, green: 0.6470588235294118, blue: 0.0, alpha: 1.0 };
    pub const PURPLE: Self = Self { red: 0.5019607843137255, green: 0.0, blue: 0.5019607843137255, alpha: 1.0 };
    pub const PINK: Self = Self { red: 1.0, green: 0.7529411764705882, blue: 0.796078431372549, alpha: 1.0 };
    pub const GRAY: Self = Self { red: 0.5019607843137255, green: 0.5019607843137255, blue: 0.5019607843137255, alpha: 1.0 };
    pub const GREY: Self = Self { red: 0.5019607843137255, green: 0.5019607843137255, blue: 0.5019607843137255, alpha: 1.0 };
    pub const BROWN: Self = Self { red: 0.6470588235294118, green: 0.16470588235294117, blue: 0.16470588235294117, alpha: 1.0 };
    pub const NAVY: Self = Self { red: 0.0, green: 0.0, blue: 0.5019607843137255, alpha: 1.0 };
    pub const TEAL: Self = Self { red: 0.0, green: 0.5019607843137255, blue: 0.5019607843137255, alpha: 1.0 };
    pub const OLIVE: Self = Self { red: 0.5019607843137255, green: 0.5019607843137255, blue: 0.0, alpha: 1.0 };
    pub const MAROON: Self = Self { red: 0.5019607843137255, green: 0.0, blue: 0.0, alpha: 1.0 };
    pub const SILVER: Self = Self { red: 0.7529411764705882, green: 0.7529411764705882, blue: 0.7529411764705882, alpha: 1.0 };
    pub const AQUA: Self = Self { red: 0.0, green: 1.0, blue: 1.0, alpha: 1.0 };
    pub const FUCHSIA: Self = Self { red: 1.0, green: 0.0, blue: 1.0, alpha: 1.0 };
    pub const CORNFLOWERBLUE: Self = Self { red: 0.39215686274509803, green: 0.5843137254901961, blue: 0.9294117647058824, alpha: 1.0 };
    pub const ROYALBLUE: Self = Self { red: 0.2549019607843137, green: 0.4117647058823529, blue: 0.8823529411764706, alpha: 1.0 };
    pub const SKYBLUE: Self = Self { red: 0.5294117647058824, green: 0.807843137254902, blue: 0.9215686274509803, alpha: 1.0 };
    pub const STEELBLUE: Self = Self { red: 0.27450980392156865, green: 0.5098039215686274, blue: 0.7058823529411765, alpha: 1.0 };
    pub const DARKBLUE: Self = Self { red: 0.0, green: 0.0, blue: 0.5450980392156862, alpha: 1.0 };
    pub const DARKGREEN: Self = Self { red: 0.0, green: 0.39215686274509803, blue: 0.0, alpha: 1.0 };
    pub const DARKRED: Self = Self { red: 0.5450980392156862, green: 0.0, blue: 0.0, alpha: 1.0 };
    pub const GOLD: Self = Self { red: 1.0, green: 0.8431372549019608, blue: 0.0, alpha: 1.0 };
    pub const INDIGO: Self = Self { red: 0.29411764705882354, green: 0.0, blue: 0.5098039215686274, alpha: 1.0 };
    pub const IVORY: Self = Self { red: 1.0, green: 1.0, blue: 0.9411764705882353, alpha: 1.0 };
    pub const KHAKI: Self = Self { red: 0.9411764705882353, green: 0.9019607843137255, blue: 0.5490196078431373, alpha: 1.0 };
    pub const LAVENDER: Self = Self { red: 0.9019607843137255, green: 0.9019607843137255, blue: 0.9803921568627451, alpha: 1.0 };
    pub const SALMON: Self = Self { red: 0.9803921568627451, green: 0.5019607843137255, blue: 0.4470588235294118, alpha: 1.0 };
    pub const TOMATO: Self = Self { red: 1.0, green: 0.38823529411764707, blue: 0.2784313725490196, alpha: 1.0 };
    pub const VIOLET: Self = Self { red: 0.9333333333333333, green: 0.5098039215686274, blue: 0.9333333333333333, alpha: 1.0 };
    pub const WHEAT: Self = Self { red: 0.9607843137254902, green: 0.8705882352941177, blue: 0.7019607843137254, alpha: 1.0 };

    /// 从字节值（0-255）创建一个 Color。
    pub fn from_bytes(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        // 每个字节除以 255.0 归一化到 0.0..1.0。
        Self {
            red: red as f64 / 255.0,
            green: green as f64 / 255.0,
            blue: blue as f64 / 255.0,
            alpha: alpha as f64 / 255.0,
        }
    }

    /// 转换为字节值 [r, g, b, a]（0-255）。
    pub fn to_bytes(&self) -> [u8; 4] {
        // 逐分量经 float_to_byte 转为 0-255（1.0 精确映射为 255）。
        [
            Self::float_to_byte(self.red),
            Self::float_to_byte(self.green),
            Self::float_to_byte(self.blue),
            Self::float_to_byte(self.alpha),
        ]
    }

    /// 将一个字节（0-255）转换为浮点数（0-1）。
    pub fn byte_to_float(value: u8) -> f64 {
        value as f64 / 255.0
    }

    /// 将一个浮点数（0-1）转换为字节（0-255）。
    pub fn float_to_byte(value: f64) -> u8 {
        // 1.0 需精确映射为 255；其余用 value*256 截断以保留 0 端的正确舍入。
        if value == 1.0 {
            255
        } else {
            (value * 256.0) as u8
        }
    }

    /// 从一个 Cartesian4（x=red、y=green、z=blue、w=alpha）创建一个 Color。
    pub fn from_cartesian4(x: f64, y: f64, z: f64, w: f64) -> Self {
        Self { red: x, green: y, blue: z, alpha: w }
    }

    /// 从 HSL 值创建一个 Color。色相 Hue 为 0..1（环绕），饱和度 saturation 0..1，亮度 lightness 0..1。
    pub fn from_hsl(hue: f64, saturation: f64, lightness: f64, alpha: f64) -> Self {
        let hue = hue % 1.0;
        let mut red = lightness;
        let mut green = lightness;
        let mut blue = lightness;

        // 饱和度为 0 时退化为灰阶，红绿蓝均等于亮度。
        if saturation != 0.0 {
            let m2 = if lightness < 0.5 {
                lightness * (1.0 + saturation)
            } else {
                lightness + saturation - lightness * saturation
            };
            let m1 = 2.0 * lightness - m2;
            red = hue2rgb(m1, m2, hue + 1.0 / 3.0);
            green = hue2rgb(m1, m2, hue);
            blue = hue2rgb(m1, m2, hue - 1.0 / 3.0);
        }

        Self { red, green, blue, alpha }
    }

    /// 从一个 CSS 颜色字符串创建一个 Color。
    /// 支持：#rgb、#rgba、#rrggbb、#rrggbbaa、rgb()、rgba()、hsl()、hsla()、命名颜色。
    /// 若该字符串不是一个有效的 CSS 颜色则返回 None。
    pub fn from_css_color_string(color: &str) -> Option<Self> {
        let color = color.trim();

        // 检查命名颜色
        if let Some(named) = Self::named_color(color) {
            return Some(named);
        }

        // #rgba 或 #rgb
        if let Some(hex) = color.strip_prefix('#') {
            // 十六进制位数决定格式：3/4 位为缩写（每位复制到两位），6/8 位为完整值。
            let hex_lower = hex.to_lowercase();
            let chars: Vec<char> = hex_lower.chars().collect();
            match chars.len() {
                3 => {
                    let r = u8::from_str_radix(&hex_lower[0..1], 16).ok()? as f64 / 15.0;
                    let g = u8::from_str_radix(&hex_lower[1..2], 16).ok()? as f64 / 15.0;
                    let b = u8::from_str_radix(&hex_lower[2..3], 16).ok()? as f64 / 15.0;
                    return Some(Self::new(r, g, b, 1.0));
                }
                4 => {
                    let r = u8::from_str_radix(&hex_lower[0..1], 16).ok()? as f64 / 15.0;
                    let g = u8::from_str_radix(&hex_lower[1..2], 16).ok()? as f64 / 15.0;
                    let b = u8::from_str_radix(&hex_lower[2..3], 16).ok()? as f64 / 15.0;
                    let a = u8::from_str_radix(&hex_lower[3..4], 16).ok()? as f64 / 15.0;
                    return Some(Self::new(r, g, b, a));
                }
                6 => {
                    let r = u8::from_str_radix(&hex_lower[0..2], 16).ok()? as f64 / 255.0;
                    let g = u8::from_str_radix(&hex_lower[2..4], 16).ok()? as f64 / 255.0;
                    let b = u8::from_str_radix(&hex_lower[4..6], 16).ok()? as f64 / 255.0;
                    return Some(Self::new(r, g, b, 1.0));
                }
                8 => {
                    let r = u8::from_str_radix(&hex_lower[0..2], 16).ok()? as f64 / 255.0;
                    let g = u8::from_str_radix(&hex_lower[2..4], 16).ok()? as f64 / 255.0;
                    let b = u8::from_str_radix(&hex_lower[4..6], 16).ok()? as f64 / 255.0;
                    let a = u8::from_str_radix(&hex_lower[6..8], 16).ok()? as f64 / 255.0;
                    return Some(Self::new(r, g, b, a));
                }
                _ => return None,
            }
        }

        // rgb() / rgba() 函数
        let lower = color.to_lowercase();
        if lower.starts_with("rgb") {
            return Self::parse_rgb_functional(color);
        }

        // hsl() / hsla() 函数
        if lower.starts_with("hsl") {
            return Self::parse_hsl_functional(color);
        }

        None
    }

    /// 解析 CSS `rgb()`/`rgba()` 函数式记法。
    ///
    /// 分量可为 0-255 整数或百分比；分隔符允许逗号、空白，以及用于 alpha 的
    /// `/`。缺少 alpha 时默认为 1.0。
    ///
    /// # 参数
    /// - `color`：形如 `rgb(255, 0, 0)` 或 `rgba(0 128 255 / 0.5)` 的字符串。
    ///
    /// # 返回
    /// 解析成功返回对应 `Color`（分量归一化到 0-1），格式非法或缺少三个分量
    /// 时返回 `None`。
    fn parse_rgb_functional(color: &str) -> Option<Self> {
        // 提取括号之间的内容
        let open = color.find('(')?;
        let close = color.rfind(')')?;
        let inner = &color[open + 1..close];

        // 按逗号或空白拆分，并处理用于 alpha 的 '/'
        let normalized = inner.replace('/', " ");
        let parts: Vec<&str> = normalized
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .collect();

        if parts.len() < 3 {
            return None;
        }

        // 带 `%` 的分量按 0-100 归一，否则按 0-255 归一。
        let parse_component = |s: &str| -> Option<f64> {
            let s = s.trim();
            if s.ends_with('%') {
                s[..s.len() - 1].parse::<f64>().ok().map(|v| v / 100.0)
            } else {
                s.parse::<f64>().ok().map(|v| v / 255.0)
            }
        };

        let red = parse_component(parts[0])?;
        let green = parse_component(parts[1])?;
        let blue = parse_component(parts[2])?;
        let alpha = if parts.len() > 3 {
            parts[3].trim().parse::<f64>().ok()?
        } else {
            1.0
        };

        Some(Self::new(red, green, blue, alpha))
    }

    /// 解析 CSS `hsl()`/`hsla()` 函数式记法。
    ///
    /// 色相以度（0-360）给出并归一化到 0-1；饱和度与亮度可为百分比或 0-1
    /// 数值；alpha 缺省为 1.0。
    ///
    /// # 参数
    /// - `color`：形如 `hsl(120, 50%, 50%)` 的字符串。
    ///
    /// # 返回
    /// 解析成功返回经 HSL→RGB 转换的 `Color`，格式非法时返回 `None`。
    fn parse_hsl_functional(color: &str) -> Option<Self> {
        let open = color.find('(')?;
        let close = color.rfind(')')?;
        let inner = &color[open + 1..close];

        let normalized = inner.replace('/', " ");
        let parts: Vec<&str> = normalized
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .collect();

        if parts.len() < 3 {
            return None;
        }

        // 色相以度给出，除以 360 归一化到 0-1。
        let hue = parts[0].trim().parse::<f64>().ok()? / 360.0;
        let sat_str = parts[1].trim();
        let sat = if sat_str.ends_with('%') {
            sat_str[..sat_str.len() - 1].parse::<f64>().ok()? / 100.0
        } else {
            sat_str.parse::<f64>().ok()?
        };
        let light_str = parts[2].trim();
        let light = if light_str.ends_with('%') {
            light_str[..light_str.len() - 1].parse::<f64>().ok()? / 100.0
        } else {
            light_str.parse::<f64>().ok()?
        };
        let alpha = if parts.len() > 3 {
            parts[3].trim().parse::<f64>().ok()?
        } else {
            1.0
        };

        Some(Self::from_hsl(hue, sat, light, alpha))
    }

    /// 将 CSS 颜色关键字映射到对应的命名颜色常量。
    ///
    /// 大小写不敏感（先转大写再匹配），`gray` 与 `grey` 视为等价。
    ///
    /// # 参数
    /// - `name`：颜色关键字，如 `Red`、`CornflowerBlue`。
    ///
    /// # 返回
    /// 命中返回对应常量，未知关键字返回 `None`。
    fn named_color(name: &str) -> Option<Self> {
        match name.to_uppercase().as_str() {
            "WHITE" => Some(Self::WHITE),
            "BLACK" => Some(Self::BLACK),
            "RED" => Some(Self::RED),
            "GREEN" => Some(Self::GREEN),
            "LIME" => Some(Self::LIME),
            "BLUE" => Some(Self::BLUE),
            "YELLOW" => Some(Self::YELLOW),
            "CYAN" => Some(Self::CYAN),
            "MAGENTA" => Some(Self::MAGENTA),
            "TRANSPARENT" => Some(Self::TRANSPARENT),
            "ORANGE" => Some(Self::ORANGE),
            "PURPLE" => Some(Self::PURPLE),
            "PINK" => Some(Self::PINK),
            "GRAY" | "GREY" => Some(Self::GRAY),
            "BROWN" => Some(Self::BROWN),
            "NAVY" => Some(Self::NAVY),
            "TEAL" => Some(Self::TEAL),
            "OLIVE" => Some(Self::OLIVE),
            "MAROON" => Some(Self::MAROON),
            "SILVER" => Some(Self::SILVER),
            "AQUA" => Some(Self::AQUA),
            "FUCHSIA" => Some(Self::FUCHSIA),
            "CORNFLOWERBLUE" => Some(Self::CORNFLOWERBLUE),
            "ROYALBLUE" => Some(Self::ROYALBLUE),
            "SKYBLUE" => Some(Self::SKYBLUE),
            "STEELBLUE" => Some(Self::STEELBLUE),
            "DARKBLUE" => Some(Self::DARKBLUE),
            "DARKGREEN" => Some(Self::DARKGREEN),
            "DARKRED" => Some(Self::DARKRED),
            "GOLD" => Some(Self::GOLD),
            "INDIGO" => Some(Self::INDIGO),
            "IVORY" => Some(Self::IVORY),
            "KHAKI" => Some(Self::KHAKI),
            "LAVENDER" => Some(Self::LAVENDER),
            "SALMON" => Some(Self::SALMON),
            "TOMATO" => Some(Self::TOMATO),
            "VIOLET" => Some(Self::VIOLET),
            "WHEAT" => Some(Self::WHEAT),
            _ => None,
        }
    }

    /// 返回一个 CSS rgb()/rgba() 字符串。
    pub fn to_css_color_string(&self) -> String {
        let r = Self::float_to_byte(self.red);
        let g = Self::float_to_byte(self.green);
        let b = Self::float_to_byte(self.blue);
        // alpha 为 1.0 时省略透明度，输出简写的 rgb()。
        if self.alpha == 1.0 {
            format!("rgb({},{},{})", r, g, b)
        } else {
            format!("rgba({},{},{},{})", r, g, b, self.alpha)
        }
    }

    /// 返回一个 CSS 十六进制字符串（#rrggbb 或 #rrggbbaa）。
    pub fn to_css_hex_string(&self) -> String {
        let r = Self::float_to_byte(self.red);
        let g = Self::float_to_byte(self.green);
        let b = Self::float_to_byte(self.blue);
        // 仅当 alpha 小于 1.0 时才附带输出第 4 个字节（aa）。
        if self.alpha < 1.0 {
            let a = Self::float_to_byte(self.alpha);
            format!("#{:02x}{:02x}{:02x}{:02x}", r, g, b, a)
        } else {
            format!("#{:02x}{:02x}{:02x}", r, g, b)
        }
    }

    /// 转换为一个 u32 RGBA 值（小端字节序：R 在最低字节）。
    pub fn to_rgba(&self) -> u32 {
        let r = Self::float_to_byte(self.red) as u32;
        let g = Self::float_to_byte(self.green) as u32;
        let b = Self::float_to_byte(self.blue) as u32;
        let a = Self::float_to_byte(self.alpha) as u32;
        // R 落在最低字节，Alpha 在最高字节（小端字节序）。
        r | (g << 8) | (b << 16) | (a << 24)
    }

    /// 从一个 u32 RGBA 值（小端字节序）创建一个 Color。
    pub fn from_rgba(rgba: u32) -> Self {
        // 按小端字节序拆出 R/G/B/A 四个字节后转回颜色。
        Self::from_bytes(
            (rgba & 0xFF) as u8,
            ((rgba >> 8) & 0xFF) as u8,
            ((rgba >> 16) & 0xFF) as u8,
            ((rgba >> 24) & 0xFF) as u8,
        )
    }

    /// 返回一个具有给定 alpha 的新 Color。
    pub fn with_alpha(&self, alpha: f64) -> Self {
        // 保留原 RGB，仅替换 alpha 分量。
        Self { alpha, ..*self }
    }

    /// 从一个已有颜色及不同的 alpha 创建一个新 Color。
    pub fn from_alpha(color: &Self, alpha: f64) -> Self {
        Self { alpha, ..*color }
    }

    /// 将当前颜色按给定亮度幅度（0..1）提亮。
    pub fn brighten(&self, magnitude: f64) -> Self {
        // 向白色靠拢：magnitude 越大，各通道越接近 1.0。
        let magnitude = 1.0 - magnitude;
        Self {
            red: 1.0 - (1.0 - self.red) * magnitude,
            green: 1.0 - (1.0 - self.green) * magnitude,
            blue: 1.0 - (1.0 - self.blue) * magnitude,
            alpha: self.alpha,
        }
    }

    /// 将当前颜色按给定幅度（0..1）加暗。
    pub fn darken(&self, magnitude: f64) -> Self {
        // 以 (1 - magnitude) 为系数缩放各通道，magnitude 越大越暗。
        let magnitude = 1.0 - magnitude;
        Self {
            red: self.red * magnitude,
            green: self.green * magnitude,
            blue: self.blue * magnitude,
            alpha: self.alpha,
        }
    }

    /// 按分量相加。
    pub fn add(&self, other: &Self) -> Self {
        Self {
            red: self.red + other.red,
            green: self.green + other.green,
            blue: self.blue + other.blue,
            alpha: self.alpha + other.alpha,
        }
    }

    /// 按分量相减。
    pub fn subtract(&self, other: &Self) -> Self {
        Self {
            red: self.red - other.red,
            green: self.green - other.green,
            blue: self.blue - other.blue,
            alpha: self.alpha - other.alpha,
        }
    }

    /// 按分量相乘。
    pub fn multiply(&self, other: &Self) -> Self {
        Self {
            red: self.red * other.red,
            green: self.green * other.green,
            blue: self.blue * other.blue,
            alpha: self.alpha * other.alpha,
        }
    }

    /// 按分量相除。
    pub fn divide(&self, other: &Self) -> Self {
        Self {
            red: self.red / other.red,
            green: self.green / other.green,
            blue: self.blue / other.blue,
            alpha: self.alpha / other.alpha,
        }
    }

    /// 按分量取模。
    pub fn modulo(&self, other: &Self) -> Self {
        Self {
            red: self.red % other.red,
            green: self.green % other.green,
            blue: self.blue % other.blue,
            alpha: self.alpha % other.alpha,
        }
    }

    /// 将所有分量乘以一个标量。
    pub fn multiply_by_scalar(&self, scalar: f64) -> Self {
        // 四个分量（含 alpha）统一乘以同一标量。
        Self {
            red: self.red * scalar,
            green: self.green * scalar,
            blue: self.blue * scalar,
            alpha: self.alpha * scalar,
        }
    }

    /// 将所有分量除以一个标量。
    pub fn divide_by_scalar(&self, scalar: f64) -> Self {
        Self {
            red: self.red / scalar,
            green: self.green / scalar,
            blue: self.blue / scalar,
            alpha: self.alpha / scalar,
        }
    }

    /// 两个颜色之间的线性插值。
    pub fn lerp(start: &Self, end: &Self, t: f64) -> Self {
        // 四个分量各按参数 t 在起/止值之间线性插值。
        Self {
            red: math_utils::lerp(start.red, end.red, t),
            green: math_utils::lerp(start.green, end.green, t),
            blue: math_utils::lerp(start.blue, end.blue, t),
            alpha: math_utils::lerp(start.alpha, end.alpha, t),
        }
    }

    /// 若当前颜色在给定 epsilon 范围内与 other 相等则返回 true。
    pub fn equals_epsilon(&self, other: &Self, epsilon: f64) -> bool {
        // 四个分量均在 epsilon 容差内相等时视为两色相等。
        (self.red - other.red).abs() <= epsilon
            && (self.green - other.green).abs() <= epsilon
            && (self.blue - other.blue).abs() <= epsilon
            && (self.alpha - other.alpha).abs() <= epsilon
    }

    /// 打包到从 index 开始的数组 [red, green, blue, alpha] 中。
    pub fn pack(&self, array: &mut [f64], starting_index: usize) {
        // 按 [r, g, b, a] 顺序从 starting_index 起连续写入四个分量。
        array[starting_index] = self.red;
        array[starting_index + 1] = self.green;
        array[starting_index + 2] = self.blue;
        array[starting_index + 3] = self.alpha;
    }

    /// 从 index 开始的数组中解包。
    pub fn unpack(array: &[f64], starting_index: usize) -> Self {
        // 从扁平数组的 starting_index 处按 [r, g, b, a] 读回四个分量。
        Self {
            red: array[starting_index],
            green: array[starting_index + 1],
            blue: array[starting_index + 2],
            alpha: array[starting_index + 3],
        }
    }
}

impl std::fmt::Display for Color {
    /// 以 `(r, g, b, a)` 形式打印四个归一化分量。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}, {}, {})", self.red, self.green, self.blue, self.alpha)
    }
}

/// HSL 转 RGB 的辅助函数（映射到 CesiumJS hue2rgb）。
fn hue2rgb(m1: f64, m2: f64, mut h: f64) -> f64 {
    // 将色相环绕回 [0, 1] 区间。
    if h < 0.0 {
        h += 1.0;
    }
    if h > 1.0 {
        h -= 1.0;
    }
    // 按色相所处扇区选择线性插值方式，对应色轮的六等分。
    if h * 6.0 < 1.0 {
        return m1 + (m2 - m1) * 6.0 * h;
    }
    if h * 2.0 < 1.0 {
        return m2;
    }
    if h * 3.0 < 2.0 {
        return m1 + (m2 - m1) * (2.0 / 3.0 - h) * 6.0;
    }
    m1
}
