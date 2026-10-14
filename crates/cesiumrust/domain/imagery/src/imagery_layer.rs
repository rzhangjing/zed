//! 影像图层配置。
//!
//! 描述单个影像图层的可见性、透明度、混合模式、裁剪范围与瓦片加载等参数，
//! 是图层集合中最小的可配置单元。

use cesium_geospatial::rectangle::Rectangle;
use serde::{Deserialize, Serialize};

use crate::{AlphaBlendingMode, SplitDirection};

/// 影像图层的配置。
///
/// 它包含可应用于影像图层的所有视觉属性。
/// 映射到 CesiumJS `ImageryLayer`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageryLayer {
    /// 此图层的唯一标识符。
    pub id: u64,

    /// 此图层覆盖的矩形区域。
    pub rectangle: Rectangle,

    /// alpha 混合值（0.0 到 1.0）。
    #[serde(default = "default_alpha")]
    pub alpha: f64,

    /// 地球夜侧的 alpha（0.0 到 1.0）。
    #[serde(default = "default_alpha")]
    pub night_alpha: f64,

    /// 地球昼侧的 alpha（0.0 到 1.0）。
    #[serde(default = "default_alpha")]
    pub day_alpha: f64,

    /// 亮度调整（1.0 = 不变）。
    #[serde(default = "default_one")]
    pub brightness: f64,

    /// 对比度调整（1.0 = 不变）。
    #[serde(default = "default_one")]
    pub contrast: f64,

    /// 色相旋转（弧度）（0.0 = 不变）。
    #[serde(default)]
    pub hue: f64,

    /// 饱和度调整（1.0 = 不变）。
    #[serde(default = "default_one")]
    pub saturation: f64,

    /// Gamma 校正（1.0 = 不变）。
    #[serde(default = "default_one")]
    pub gamma: f64,

    /// 图层是否可见。
    #[serde(default = "default_true")]
    pub show: bool,

    /// alpha 混合模式。
    #[serde(default)]
    pub alpha_blending_mode: AlphaBlendingMode,

    /// 用于分屏对比的分割方向。
    #[serde(default)]
    pub split_direction: SplitDirection,

    /// 最小缩放级别。
    #[serde(default)]
    pub minimum_level: u32,

    /// 最大缩放级别。
    #[serde(default = "default_max_level")]
    pub maximum_level: u32,

    /// 瓦片宽度（像素）。
    #[serde(default = "default_tile_size")]
    pub tile_width: u32,

    /// 瓦片高度（像素）。
    #[serde(default = "default_tile_size")]
    pub tile_height: u32,
}

/// serde 缺省：图层不透明度默认 1.0（完全不透明）。
fn default_alpha() -> f64 {
    1.0
}

/// serde 缺省：各类强度/比例因子的单位值 1.0。
fn default_one() -> f64 {
    1.0
}

/// serde 缺省：布尔开关默认开启。
fn default_true() -> bool {
    true
}

/// serde 缺省：最大详细层级默认 25 级。
fn default_max_level() -> u32 {
    25
}

/// serde 缺省：瓦片边长默认 256 像素。
fn default_tile_size() -> u32 {
    256
}

impl ImageryLayer {
    /// 使用默认设置创建一个新的影像图层。
    pub fn new(id: u64, rectangle: Rectangle) -> Self {
        Self {
            id,
            rectangle,
            alpha: 1.0,
            night_alpha: 1.0,
            day_alpha: 1.0,
            brightness: 1.0,
            contrast: 1.0,
            hue: 0.0,
            saturation: 1.0,
            gamma: 1.0,
            show: true,
            alpha_blending_mode: AlphaBlendingMode::Standard,
            split_direction: SplitDirection::None,
            minimum_level: 0,
            maximum_level: 25,
            tile_width: 256,
            tile_height: 256,
        }
    }

    /// 设置 alpha 值。
    pub fn with_alpha(mut self, alpha: f64) -> Self {
        self.alpha = alpha.clamp(0.0, 1.0);
        self
    }

    /// 设置亮度。
    pub fn with_brightness(mut self, brightness: f64) -> Self {
        self.brightness = brightness.max(0.0);
        self
    }

    /// 设置对比度。
    pub fn with_contrast(mut self, contrast: f64) -> Self {
        self.contrast = contrast.max(0.0);
        self
    }

    /// 设置饱和度。
    pub fn with_saturation(mut self, saturation: f64) -> Self {
        self.saturation = saturation.max(0.0);
        self
    }

    /// 设置 gamma。
    pub fn with_gamma(mut self, gamma: f64) -> Self {
        self.gamma = gamma.max(0.001);
        self
    }

    /// 设置可见性。
    pub fn with_show(mut self, show: bool) -> Self {
        self.show = show;
        self
    }

    /// 设置 alpha 混合模式。
    pub fn with_alpha_blending_mode(mut self, mode: AlphaBlendingMode) -> Self {
        self.alpha_blending_mode = mode;
        self
    }

    /// 设置分割方向。
    pub fn with_split_direction(mut self, direction: SplitDirection) -> Self {
        self.split_direction = direction;
        self
    }

    /// 设置缩放级别范围。
    pub fn with_level_range(mut self, min: u32, max: u32) -> Self {
        self.minimum_level = min;
        self.maximum_level = max;
        self
    }

    /// 设置瓦片大小。
    pub fn with_tile_size(mut self, width: u32, height: u32) -> Self {
        self.tile_width = width;
        self.tile_height = height;
        self
    }

    /// 计算给定光照条件下的有效 alpha。
    ///
    /// # 参数
    /// * `is_night` - 瓦片是否位于地球夜侧
    pub fn effective_alpha(&self, is_night: bool) -> f64 {
        let base_alpha = if is_night { self.night_alpha } else { self.day_alpha };
        base_alpha * self.alpha
    }

    /// 检查级别是否处于此图层的有效范围内。
    pub fn is_level_valid(&self, level: u32) -> bool {
        level >= self.minimum_level && level <= self.maximum_level
    }

    /// 检查矩形是否与此图层的矩形相交。
    pub fn intersects(&self, rectangle: &Rectangle) -> bool {
        self.rectangle.intersection(rectangle).is_some()
    }
}

impl Default for ImageryLayer {
    /// 默认图层：序号 0、覆盖整幅 [`Rectangle::MAX_VALUE`] 范围。
    fn default() -> Self {
        Self::new(0, Rectangle::MAX_VALUE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_layer() {
        let layer = ImageryLayer::new(1, Rectangle::from_degrees(-180.0, -90.0, 180.0, 90.0));
        assert_eq!(layer.id, 1);
        assert_eq!(layer.alpha, 1.0);
        assert!(layer.show);
    }

    #[test]
    fn test_builder_pattern() {
        let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE)
            .with_alpha(0.5)
            .with_brightness(1.2)
            .with_show(false);

        assert_eq!(layer.alpha, 0.5);
        assert_eq!(layer.brightness, 1.2);
        assert!(!layer.show);
    }

    #[test]
    fn test_alpha_clamping() {
        let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE).with_alpha(1.5);
        assert_eq!(layer.alpha, 1.0);

        let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE).with_alpha(-0.5);
        assert_eq!(layer.alpha, 0.0);
    }

    #[test]
    fn test_effective_alpha() {
        let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE)
            .with_alpha(0.8);

        assert_eq!(layer.effective_alpha(false), 0.8); // 白天
        assert_eq!(layer.effective_alpha(true), 0.8); // 夜晚（默认相同）
    }

    #[test]
    fn test_level_validation() {
        let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE)
            .with_level_range(2, 10);

        assert!(!layer.is_level_valid(1));
        assert!(layer.is_level_valid(2));
        assert!(layer.is_level_valid(10));
        assert!(!layer.is_level_valid(11));
    }
}
