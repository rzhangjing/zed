//! 面向 CesiumRust 领域类型的 Bevy ECS 组件。
//!
//! 在“混合模式”下，这些领域组件结构体被直接用作渲染的
//! Bevy Component，而 IO 则使用 port trait。
//!
//! 2026-09 P1 裁剪：原先在此定义的 3D Tiles / 地形组件
//! （`CesiumTerrainTile`、`CesiumTilesetRoot` + `TilesetLoadingState`、
//! `CesiumTileNode` + `TileContentState`、`TileContent`）以及只服务地形
//! 垂覆链的 `CesiumImageryLayer` 已随 `cesium-terrain` / `cesium-tileset`
//! 及其适配器模块（`src/terrain`、`src/tileset`、`src/imagery`）删除。
//! 底球面影像由 `application/cesium-app/src/dynamic_globe.rs`（Path A）
//! 直接以自有实体与纹理缓存渲染，不需要这些组件。

use bevy::prelude::*;

/// 主地球实体的标记组件（所有地球子实体的根）。
#[derive(Component)]
pub struct CesiumGlobe;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 标记组件应可作为 Bevy 组件挂载到实体上。
    fn test_cesium_globe_component() {
        let mut world = World::new();
        let entity = world.spawn(CesiumGlobe).id();
        assert!(world.get::<CesiumGlobe>(entity).is_some());
    }
}
