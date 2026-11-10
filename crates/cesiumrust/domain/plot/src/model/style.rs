//! 元素样式：颜色、尺寸、填充/描边、图标/文本、高度参考与
//! 绘制顺序。可直接序列化；渲染桥接层直接读取它（§5）。
//!
//! 颜色是普通的 RGBA `[f32; 4]`（0..1，近线性 —— 桥接层负责
//! 任何色彩空间转换），因此核心保持无引擎依赖且可 diff。

use serde::{Deserialize, Serialize};

/// RGBA 颜色，各分量在 0..=1。
pub type Rgba = [f32; 4];

/// 不透明的白色。
pub const WHITE: Rgba = [1.0, 1.0, 1.0, 1.0];
/// 完全透明。
pub const TRANSPARENT: Rgba = [0.0, 0.0, 0.0, 0.0];

/// 元素的高度如何相对于地形 / 椭球体被解释
/// （Cesium `HeightReference` 语义，计划 §4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum HeightReference {
    /// 直接使用存储的绝对 `height_m`。
    #[default]
    None,
    /// 随贴于地形 / 地球表面（高度强制为表面）。
    ClampToGround,
    /// `height_m` 从地形表面往上度量。
    RelativeToGround,
    /// 椭球体上方的显式米数（在此与 None 相同，为清晰起见保留）。
    Absolute,
}

/// 多边形轮廓（描边）描述。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Outline {
    pub color: Rgba,
    pub width_px: f32,
}

/// 图标绘制覆盖项（图像从几何体的注册表键解析）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct IconStyle {
    pub size_px: f32,
    /// 图标框内的水平/垂直锚点，0..1（0.5, 0.5 == 中心）。
    pub anchor: [f32; 2],
}

/// 一个 [`Label`](super::geometry::LabelGeometry) 或带标签图标的
/// 文本绘制覆盖项。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub size_px: f32,
    pub color: Rgba,
    /// 字符后面的光晕/轮廓颜色，以提升在影像上的可读性。
    pub halo_color: Rgba,
    pub halo_px: f32,
}

/// 元素的完整样式集合。每个字段都有合理的默认值，因此
/// 一个新绘制的元素无需显式样式也能看起来合理。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Style {
    /// 主颜色（点填充、线描边、多边形轮廓基底）。
    pub color: Rgba,
    /// 施加在 `color` 的 alpha 之上的全局不透明度乘子。
    pub opacity: f32,
    /// 屏幕像素下的线 / 轮廓宽度。
    pub width_px: f32,
    /// 多边形填充颜色；`None` = 仅轮廓。
    pub fill: Option<Rgba>,
    /// 显式的轮廓覆盖；`None` = 从 `color`/`width_px` 推导。
    pub outline: Option<Outline>,
    /// 屏幕像素下的点标记直径。
    pub point_size_px: f32,
    /// 图标覆盖项（仅当几何体为 `Icon` 时存在）。
    pub icon: Option<IconStyle>,
    /// 文本覆盖项（仅当几何体携带文本时存在）。
    pub text: Option<TextStyle>,
    /// 高度语义。
    pub height_reference: HeightReference,
    /// 针对地形进行深度测试（true = 会被地球正确遮挡）。
    pub depth_test: bool,
    /// 图层内的相对绘制 / 拾取顺序（越大越后绘制、在上）。
    pub z_order: i32,
    /// 在 2D 平面视图中显示（§10.6）。
    pub show_in_flat: bool,
    /// 在 3D 地球视图中显示（§10.6）。
    pub show_in_globe: bool,
}

impl Default for Style {
    /// 一组面向默认蓝的初值：线宽 2px、半透明填充、带绘光文本，且默认开启地形深度测试。
    fn default() -> Self {
        Self {
            color: [0.16, 0.50, 0.86, 1.0],
            opacity: 1.0,
            width_px: 2.0,
            fill: Some([0.16, 0.50, 0.86, 0.25]),
            outline: None,
            point_size_px: 10.0,
            icon: Some(IconStyle {
                size_px: 32.0,
                anchor: [0.5, 0.5],
            }),
            text: Some(TextStyle {
                size_px: 16.0,
                color: WHITE,
                halo_color: [0.0, 0.0, 0.0, 0.85],
                halo_px: 2.0,
            }),
            height_reference: HeightReference::default(),
            // 计划 §17.3 默认：新元素会被地形遮挡。
            depth_test: true,
            z_order: 0,
            show_in_flat: true,
            show_in_globe: true,
        }
    }
}

impl Style {
    /// `color` 乘以 `opacity` 后的值（有效的绘制颜色）。
    pub fn effective_color(&self) -> Rgba {
        [
            self.color[0],
            self.color[1],
            self.color[2],
            self.color[3] * self.opacity,
        ]
    }

    /// 一份具有不同主颜色的副本（保留其他所有内容）。
    pub fn with_color(mut self, color: Rgba) -> Self {
        self.color = color;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_visible_in_both_views_with_depth_test() {
        let s = Style::default();
        assert!(s.show_in_flat && s.show_in_globe);
        assert!(s.depth_test, "plan §17.3: new elements occluded by default");
        assert_eq!(s.height_reference, HeightReference::None);
    }

    #[test]
    fn effective_color_scales_alpha_only() {
        let s = Style::default();
        let c = s.effective_color();
        assert_eq!(c[0], s.color[0]);
        assert!((c[3] - s.color[3] * s.opacity).abs() < 1e-9);
    }

    #[test]
    fn style_serde_roundtrips() {
        let s = Style::default();
        let j = serde_json::to_string(&s).unwrap();
        let back: Style = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
