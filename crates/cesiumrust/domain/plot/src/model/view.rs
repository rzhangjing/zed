//! 纯可见性评估器所读取的视图描述符。
//!
//! 核心拥有规范的 [`ViewMode`]（桥接层为其设别名，因此整个
//! 工作区只有一个定义）以及一个 [`ViewContext`]，仅携带多维可见性
//! 规则所需的视图度量。将其放在这里意味着
//! `eval_visibility` 是一个关于 `(Document, ViewContext,
//! Filters)` 的纯函数，无引擎类型 —— 可确定性地进行单元测试。

use serde::{Deserialize, Serialize};

/// 正在针对哪个投影评估覆盖层。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ViewMode {
    /// 3D 地球（WGS84 / ECEF）。
    #[default]
    Globe,
    /// 2D 平面地图（等距圆柱投影）。
    Flat,
}

/// 当前视图“缩进”到什么程度，用两种等价方式表达，以便
/// 可见性规则的每个缩放维度选择更自然的那个：
///  * [`ViewContext::pixels_per_world`] 驱动 2D 的 `min/max_zoom_px` 波段
///    （值越大 == 越靠近）；
///  * [`ViewContext::meters_per_pixel`] 驱动米数波段（越大 ==
///    越远离）。桥接层至少填充其一；另一个可为 `0.0`。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewContext {
    /// 当前投影模式。
    pub mode: ViewMode,
    /// 每世界单位的像素数（2D 尺度度量）。未知时为 `0.0`。
    pub pixels_per_world: f64,
    /// 一个屏幕像素覆盖的地面米数（3D 尺度度量）。未知时为
    /// `0.0`；越小 == 越靠向地表。
    pub meters_per_pixel: f64,
    /// 逻辑像素下的屏幕宽度（M1 规则未使用；为后续保留）。
    pub screen_w: f64,
    /// 逻辑像素下的屏幕高度。
    pub screen_h: f64,
    /// 自历元起的当前动画时间（秒）。在时间维度接入（M9）之前为 `0.0`；
    /// 时间窗口规则在窗口未设置时会忽略它。
    pub time_s: f64,
}

impl Default for ViewContext {
    /// 全零的缺省上下文：各度量均为 0.0（表示尚未由桥接层填充）。
    fn default() -> Self {
        Self {
            mode: ViewMode::default(),
            pixels_per_world: 0.0,
            meters_per_pixel: 0.0,
            screen_w: 0.0,
            screen_h: 0.0,
            time_s: 0.0,
        }
    }
}
