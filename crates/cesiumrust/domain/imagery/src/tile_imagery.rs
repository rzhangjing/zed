//! 瓦片影像关联。
//!
//! 记录某个瓦片上各图层影像的矩形范围与 UV 映射，供混合时取样定位。

use cesium_geospatial::rectangle::Rectangle;
use serde::{Deserialize, Serialize};

use crate::imagery_state::ImageryState;

/// 表示地形瓦片与影像瓦片之间的关联。
///
/// 它追踪特定地形瓦片与影像图层组合的影像加载状态。
/// 映射到 CesiumJS `TileImagery`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileImagery {
    /// 影像图层 ID。
    pub layer_id: u64,

    /// 瓦片的 X 坐标。
    pub x: u32,

    /// 瓦片的 Y 坐标。
    pub y: u32,

    /// 瓦片的层级。
    pub level: u32,

    /// 此影像瓦片的当前状态。
    pub state: ImageryState,

    /// 纹理坐标平移（用于重投影）。
    pub texture_translation: [f64; 2],

    /// 纹理坐标缩放（用于重投影）。
    pub texture_scale: [f64; 2],

    /// 此影像瓦片覆盖的矩形区域。
    pub rectangle: Rectangle,

    /// 此影像瓦片是否需要重投影。
    pub needs_reprojection: bool,
}

impl TileImagery {
    /// 创建一个新的瓦片影像关联。
    pub fn new(layer_id: u64, x: u32, y: u32, level: u32, rectangle: Rectangle) -> Self {
        Self {
            layer_id,
            x,
            y,
            level,
            state: ImageryState::Unloaded,
            texture_translation: [0.0, 0.0],
            texture_scale: [1.0, 1.0],
            rectangle,
            needs_reprojection: false,
        }
    }

    /// 设置状态。
    pub fn with_state(mut self, state: ImageryState) -> Self {
        self.state = state;
        self
    }

    /// 设置用于重投影的纹理坐标。
    pub fn with_texture_coords(mut self, translation: [f64; 2], scale: [f64; 2]) -> Self {
        self.texture_translation = translation;
        self.texture_scale = scale;
        self.needs_reprojection = true;
        self
    }

    /// 将此影像标记为需要重投影。
    pub fn set_needs_reprojection(&mut self, needs: bool) {
        self.needs_reprojection = needs;
    }

    /// 若此影像已可渲染则返回 true。
    pub fn is_ready(&self) -> bool {
        self.state.is_renderable()
    }

    /// 若应为此影像发起请求则返回 true。
    pub fn should_request(&self) -> bool {
        self.state.should_request()
    }

    /// 计算瓦片内给定位置的纹理坐标。
    ///
    /// # 参数
    /// * `u` - 地形瓦片内的 U 坐标（0.0 到 1.0）
    /// * `v` - 地形瓦片内的 V 坐标（0.0 到 1.0）
    ///
    /// # 返回
    /// 用于采样影像纹理的纹理坐标 [u, v]
    pub fn compute_texture_coords(&self, u: f64, v: f64) -> [f64; 2] {
        [
            u * self.texture_scale[0] + self.texture_translation[0],
            v * self.texture_scale[1] + self.texture_translation[1],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_tile_imagery() {
        let tile = TileImagery::new(
            1,
            0,
            0,
            0,
            Rectangle::from_degrees(-180.0, -90.0, 0.0, 90.0),
        );

        assert_eq!(tile.layer_id, 1);
        assert_eq!(tile.x, 0);
        assert_eq!(tile.y, 0);
        assert_eq!(tile.level, 0);
        assert_eq!(tile.state, ImageryState::Unloaded);
    }

    #[test]
    fn test_state_transitions() {
        let mut tile = TileImagery::new(1, 0, 0, 0, Rectangle::MAX_VALUE);

        assert!(tile.should_request());
        assert!(!tile.is_ready());

        tile.state = ImageryState::Transitioning;
        assert!(!tile.should_request());
        assert!(!tile.is_ready());

        tile.state = ImageryState::Ready;
        assert!(!tile.should_request());
        assert!(tile.is_ready());
    }

    #[test]
    fn test_texture_coords() {
        let tile = TileImagery::new(1, 0, 0, 0, Rectangle::MAX_VALUE)
            .with_texture_coords([0.25, 0.25], [0.5, 0.5]);

        let coords = tile.compute_texture_coords(0.5, 0.5);
        assert!((coords[0] - 0.5).abs() < 1e-10); // 0.5 * 0.5 + 0.25
        assert!((coords[1] - 0.5).abs() < 1e-10);
    }
}
