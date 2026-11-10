//! 用于时间动态值的属性系统。
//!
//! 本模块定义属性求值的核心数据形态：颜色等基础值类型，以及可在常量、
//! 采样值与时间区间集合之间切换的泛型 `Property`。属性既可整体为常量，
//! 也可随时间变化，供实体各视觉外观在每一帧按需取值；缺失时以 `Option`
//! 表达“未定义”，交由上层回退到默认外观。

use serde::{Deserialize, Serialize};

/// 以 RGBA 表示的颜色值（每通道 0.0..1.0）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alpha: f64,
}

impl Color {
    /// 创建新颜色。
    pub fn new(red: f64, green: f64, blue: f64, alpha: f64) -> Self {
        Self { red, green, blue, alpha }
    }

    /// 不透明白。
    pub const WHITE: Self = Self { red: 1.0, green: 1.0, blue: 1.0, alpha: 1.0 };

    /// 不透明黑。
    pub const BLACK: Self = Self { red: 0.0, green: 0.0, blue: 0.0, alpha: 1.0 };

    /// 不透明红。
    pub const RED: Self = Self { red: 1.0, green: 0.0, blue: 0.0, alpha: 1.0 };

    /// 不透明绿。
    pub const GREEN: Self = Self { red: 0.0, green: 1.0, blue: 0.0, alpha: 1.0 };

    /// 不透明蓝。
    pub const BLUE: Self = Self { red: 0.0, green: 0.0, blue: 1.0, alpha: 1.0 };

    /// 不透明黄。
    pub const YELLOW: Self = Self { red: 1.0, green: 1.0, blue: 0.0, alpha: 1.0 };

    /// 不透明青。
    pub const CYAN: Self = Self { red: 0.0, green: 1.0, blue: 1.0, alpha: 1.0 };

    /// 完全透明。
    pub const TRANSPARENT: Self = Self { red: 0.0, green: 0.0, blue: 0.0, alpha: 0.0 };

    /// 从 CSS 十六进制字符串创建颜色（例如 "#FF0000" 或 "#FF000080"）。
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        match hex.len() {
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Self::new(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, 1.0))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some(Self::new(
                    r as f64 / 255.0,
                    g as f64 / 255.0,
                    b as f64 / 255.0,
                    a as f64 / 255.0,
                ))
            }
            _ => None,
        }
    }

    /// 转换为 [f32; 4] 以供 GPU 使用。
    pub fn to_f32_array(&self) -> [f32; 4] {
        [self.red as f32, self.green as f32, self.blue as f32, self.alpha as f32]
    }
}

#[allow(clippy::derivable_impls)]
impl Default for Color {
    /// 缺省颜色为不透明白色。
    fn default() -> Self {
        Self::WHITE
    }
}

/// 可为常量或随时间变化的属性值。
///
/// 属性可以是始终不变的常量，也可以是按采样点或时间区间插值得到的
/// 动态值；泛型参数 `T` 约束被承载的值必须可克隆且可比较，以便做
/// 脏检查与缓存。
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Property<T: Clone + PartialEq> {
    /// 一个常量值。
    Constant(T),
    /// 带时间-值对的采样属性（JulianDate 秒数，值）。
    Sampled(Vec<(f64, T)>),
    /// 未定义（无值）。
    #[default]
    Undefined,
}

impl<T: Clone + PartialEq> Property<T> {
    /// 获取给定时间（自纪元起的秒数）下的值。
    ///
    /// 对于常量属性，始终返回常量。
    /// 对于采样属性，返回最接近的值（不做插值）。
    pub fn get_value(&self, time: f64) -> Option<&T> {
        match self {
            Property::Constant(v) => Some(v),
            Property::Sampled(samples) => {
                if samples.is_empty() {
                    return None;
                }
                // 查找最接近的采样点
                let mut nearest = &samples[0];
                let mut min_dist = (time - nearest.0).abs();
                for sample in samples.iter().skip(1) {
                    let dist = (time - sample.0).abs();
                    if dist < min_dist {
                        min_dist = dist;
                        nearest = sample;
                    }
                }
                Some(&nearest.1)
            }
            Property::Undefined => None,
        }
    }

    /// 若为常量属性则返回 true。
    pub fn is_constant(&self) -> bool {
        matches!(self, Property::Constant(_))
    }

    /// 若此属性有值则返回 true。
    pub fn is_defined(&self) -> bool {
        !matches!(self, Property::Undefined)
    }
}



/// 位置属性（大地坐标：lon, lat, height，以弧度/米计）。
pub type PositionProperty = Property<[f64; 3]>;

/// 颜色属性。
pub type ColorProperty = Property<Color>;

/// 数值属性。
pub type NumberProperty = Property<f64>;

/// 布尔属性。
pub type BoolProperty = Property<bool>;

/// 字符串属性。
pub type StringProperty = Property<String>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_from_hex() {
        let color = Color::from_hex("#FF0000").unwrap();
        assert!((color.red - 1.0).abs() < 1e-10);
        assert!((color.green - 0.0).abs() < 1e-10);
        assert!((color.blue - 0.0).abs() < 1e-10);
        assert!((color.alpha - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_color_from_hex_with_alpha() {
        let color = Color::from_hex("#FF000080").unwrap();
        assert!((color.red - 1.0).abs() < 1e-10);
        assert!((color.alpha - 128.0 / 255.0).abs() < 1e-10);
    }

    #[test]
    fn test_constant_property() {
        let prop: Property<f64> = Property::Constant(42.0);
        assert!(prop.is_constant());
        assert!(prop.is_defined());
        assert_eq!(*prop.get_value(0.0).unwrap(), 42.0);
        assert_eq!(*prop.get_value(100.0).unwrap(), 42.0);
    }

    #[test]
    fn test_sampled_property() {
        let prop: Property<f64> = Property::Sampled(vec![
            (0.0, 10.0),
            (10.0, 20.0),
            (20.0, 30.0),
        ]);
        assert!(!prop.is_constant());
        assert!(prop.is_defined());

        // 最接近 time=0
        assert_eq!(*prop.get_value(0.0).unwrap(), 10.0);
        // 最接近 time=9
        assert_eq!(*prop.get_value(9.0).unwrap(), 20.0);
        // 最接近 time=20
        assert_eq!(*prop.get_value(20.0).unwrap(), 30.0);
    }

    #[test]
    fn test_undefined_property() {
        let prop: Property<f64> = Property::Undefined;
        assert!(!prop.is_defined());
        assert!(prop.get_value(0.0).is_none());
    }

    #[test]
    fn test_color_property() {
        let prop: ColorProperty = Property::Constant(Color::RED);
        let color = prop.get_value(0.0).unwrap();
        assert!((color.red - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_color_to_f32() {
        let color = Color::new(1.0, 0.5, 0.25, 1.0);
        let arr = color.to_f32_array();
        assert!((arr[0] - 1.0).abs() < 1e-6);
        assert!((arr[1] - 0.5).abs() < 1e-6);
        assert!((arr[2] - 0.25).abs() < 1e-6);
    }
}
