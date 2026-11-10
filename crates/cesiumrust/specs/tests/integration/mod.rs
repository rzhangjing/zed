// P1 裁剪（2026-09）：原先这里还有 `tileset_integration_test` /
// `terrain_integration_test` / `imagery_integration_test` 三个模块 ——
// terrain 与 3D Tiles 链（及其地形垂覆影像模块）已随
// `cesium-terrain` / `cesium-tileset` / `cesium-gltf` / `cesium-styling`
// 删除，见 docs/SCOPE_2D3D_IMAGERY_PLOT.md。
pub mod camera_integration_test;
pub mod material_integration_test;

use bevy::prelude::*;
use cesium_bevy_render::CesiumCorePlugin;

pub fn create_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(CesiumCorePlugin);
    app
}
