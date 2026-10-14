//! 基础层 zoom 豁免逻辑。
//!
//! 照搬 `dynamic_globe.rs:1483` —— `z <= BASE_LAYER_ZOOM` 的瓦片构成
//! 永久性的全局回退层（即 CesiumJS 基础影像层的角色）。
//! 它们在启动时下载一次，永不 despawn 或驱逐，因此向从未访问区域的
//! 快速平移会显示模糊影像，而非细瓦片加载期间的黑色基础球体。

/// 判断一个瓦片键是否属于常驻的基础层。
///
/// zoom 分量通过提供的闭包提取，从而保持此函数对键类型通用
/// （例如 `(u32, u32, u32)`，其中 `.2` 是 zoom）。
///
/// 对应 `dynamic_globe.rs:1483`：
/// ```text
/// if old.2 <= BASE_LAYER_ZOOM { continue; }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct BaseLayerGuard {
    /// 永久豁免驱逐的最大 zoom 层级。
    /// 默认：3（`dynamic_globe.rs:70`）。
    pub max_zoom: u32,
}

impl BaseLayerGuard {
    /// 创建一个使用默认基础层 zoom（3）的守卫。
    pub fn new() -> Self {
        Self { max_zoom: 3 }
    }

    /// 创建一个使用自定义基础层 zoom 的守卫。
    pub fn with_zoom(max_zoom: u32) -> Self {
        Self { max_zoom }
    }

    /// 若给定的 zoom 层级位于基础层内（永久豁免驱逐/despawn），
    /// 则返回 true。
    #[inline]
    pub fn is_base_layer(&self, zoom: u32) -> bool {
        zoom <= self.max_zoom
    }
}

impl Default for BaseLayerGuard {
    /// 默认构造一个基础图层守卫（等价于 [`BaseLayerGuard::new`]）。
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_layer_exempt_at_zoom_3_and_below() {
        let guard = BaseLayerGuard::new();
        assert!(guard.is_base_layer(0));
        assert!(guard.is_base_layer(1));
        assert!(guard.is_base_layer(2));
        assert!(guard.is_base_layer(3));
    }

    #[test]
    fn not_base_layer_above_zoom_3() {
        let guard = BaseLayerGuard::new();
        assert!(!guard.is_base_layer(4));
        assert!(!guard.is_base_layer(19));
    }

    #[test]
    fn custom_zoom_level() {
        let guard = BaseLayerGuard::with_zoom(5);
        assert!(guard.is_base_layer(5));
        assert!(!guard.is_base_layer(6));
    }
}
