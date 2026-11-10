//! 单个标绘元素：几何 + 样式 + 业务属性 + 逐元素
//! 标志以及尺度/时间可见性窗口（计划 §5）。
//!
//! 地理包围盒缓存于元素上，并在几何变化时刷新，因此
//! 裁剪 / 缩放波段检查保持 O(1)。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::geo::GeoBounds;
use super::geometry::Geometry;
use super::ids::ElementId;
use super::style::Style;

/// 手动 / 交互标志（与可见性维度无关）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementFlags {
    /// 元素自身的可见性开关（与图层/组链进行 AND）。
    pub visible_manual: bool,
    /// 可被拾取 / 选中吗？
    pub selectable: bool,
    /// 编辑可移动 / 变形它吗？
    pub editable: bool,
}

impl Default for ElementFlags {
    /// 缺省：可见、可选、可编辑 —— 一个刚放置元素的自然初值。
    fn default() -> Self {
        Self {
            visible_manual: true,
            selectable: true,
            editable: true,
        }
    }
}

/// 屏幕/地面尺度可见性波段（§10.7）。每个边界都是可选的；
/// 仅当当前视图落入每个存在的边界内时才显示该元素。单位：`*_px` 是
/// 每世界单位像素数（越大 == 越放大），`*_meters` 是地面每像素米数
/// （越大 == 越缩小）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ScaleVisibility {
    /// 显示所需的最小每世界像素（越大越放大）。
    pub min_pixels_per_world: Option<f64>,
    /// 允许显示的最大每世界像素。
    pub max_pixels_per_world: Option<f64>,
    /// 显示所需的最小地面每像素米数（越大越缩小）。
    pub min_meters_per_pixel: Option<f64>,
    /// 允许显示的最大地面每像素米数。
    pub max_meters_per_pixel: Option<f64>,
}

impl ScaleVisibility {
    /// 当没有波段，或两个已知度量都满足时返回 true。`0.0`
    /// （未知）的度量永不*违反*一个无法保守比较的边界
    /// —— 期望桥接层为其模式填充该度量。
    pub fn allows(&self, pixels_per_world: f64, meters_per_pixel: f64) -> bool {
        // 度量 > 0.0（已知）时才参与边界比较；0.0（未知）永不违反。
        if let Some(m) = self.min_pixels_per_world {
            if pixels_per_world > 0.0 && pixels_per_world < m {
                return false;
            }
        }
        if let Some(m) = self.max_pixels_per_world {
            if pixels_per_world > 0.0 && pixels_per_world > m {
                return false;
            }
        }
        if let Some(m) = self.min_meters_per_pixel {
            if meters_per_pixel > 0.0 && meters_per_pixel < m {
                return false;
            }
        }
        if let Some(m) = self.max_meters_per_pixel {
            if meters_per_pixel > 0.0 && meters_per_pixel > m {
                return false;
            }
        }
        true
    }
}

/// 一个标绘元素。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    /// 文档内唯一的元素 id。
    pub id: ElementId,
    /// 显示名称。
    pub name: String,
    /// 几何形状（位置 / 形状的单一事实源）。
    pub geometry: Geometry,
    /// 样式包（颜色 / 宽 / 每模式显示标志 …）。
    pub style: Style,
    /// Free-form business attributes (敌我/番号/状态 …), GeoJSON `properties`.
    pub attributes: Map<String, Value>,
    /// 手动 / 交互标志集。
    pub flags: ElementFlags,
    /// 尺度可见性波段。
    pub scale_visibility: ScaleVisibility,
    /// 保留的时间窗口 `[start, end)`，以自历元起的秒数表示（§10.10）。
    /// `None` == 总是。M1 不据此门控（见 `visibility`）。
    pub time_window: Option<(f64, f64)>,
    /// 缓存的地理包围盒，与 `geometry` 保持同步。
    pub bounds: GeoBounds,
}

impl Element {
    /// 构建一个元素，不自行铸造 id（调用方提供文档
    /// 分配的 id）并计算其包围盒。
    pub fn new(id: ElementId, name: impl Into<String>, geometry: Geometry) -> Self {
        let bounds = geometry.bounds();
        Self {
            id,
            name: name.into(),
            geometry,
            style: Style::default(),
            attributes: Map::new(),
            flags: ElementFlags::default(),
            scale_visibility: ScaleVisibility::default(),
            time_window: None,
            bounds,
        }
    }

    /// 替换几何并刷新缓存的包围盒。
    pub fn set_geometry(&mut self, geometry: Geometry) {
        self.bounds = geometry.bounds();
        self.geometry = geometry;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::GeoPoint;

    fn pt(lon: f64, lat: f64) -> Geometry {
        Geometry::Point(GeoPoint::surface(lon, lat))
    }

    #[test]
    fn new_caches_bounds() {
        let e = Element::new(ElementId(1), "p", pt(12.0, -3.0));
        assert!(e.flags.visible_manual && e.flags.selectable && e.flags.editable);
        assert_eq!((e.bounds.west_deg, e.bounds.north_deg), (12.0, -3.0));
    }

    #[test]
    fn set_geometry_refreshes_bounds() {
        let mut e = Element::new(ElementId(2), "l", pt(0.0, 0.0));
        e.set_geometry(Geometry::Polyline(super::super::geometry::Polyline {
            positions: vec![GeoPoint::surface(-5.0, -5.0), GeoPoint::surface(7.0, 9.0)],
        }));
        // 包围盒是保守的（大圆加密）盒子：它必须
        // 同时包含两个端点，并保持在跨度一个紧密边距内。
        let b = e.bounds;
        assert!(
            b.west_deg <= -5.0 && b.south_deg <= -5.0 && b.east_deg >= 7.0 && b.north_deg >= 9.0,
            "endpoints must be contained: {b:?}",
        );
        assert!(b.width_deg() < 13.0 && b.height_deg() < 15.0, "bulge margin too wide: {b:?}");
    }

    #[test]
    fn scale_band_respects_present_bounds() {
        let sv = ScaleVisibility {
            min_pixels_per_world: Some(100.0),
            max_pixels_per_world: Some(1000.0),
            ..Default::default()
        };
        assert!(sv.allows(500.0, 0.0));
        assert!(!sv.allows(50.0, 0.0), "too far out");
        assert!(!sv.allows(5000.0, 0.0), "too far in");
        // 未知度量 (0) 永不违反。
        assert!(sv.allows(0.0, 0.0));
        // 空波段总是允许。
        assert!(ScaleVisibility::default().allows(1.0, 1.0));
    }

    #[test]
    fn element_serde_roundtrips() {
        let mut e = Element::new(ElementId(3), "n", pt(1.0, 2.0));
        e.attributes.insert("side".into(), Value::String("friend".into()));
        let j = serde_json::to_string(&e).unwrap();
        let back: Element = serde_json::from_str(&j).unwrap();
        assert_eq!(e, back);
    }
}
