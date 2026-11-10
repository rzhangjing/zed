//! `CesiumPipelinePlugin`——M1 pipeline 绑定层的选择性开启装配。

use bevy::prelude::*;

use super::bindings::PipelineEvictionStats;
use super::gpu_handle::BevyGpuHandleCache;
use super::system_wiring::gpu_cache_eviction_system;

/// 选择性开启的插件，把 `cesium-pipeline` core 接入一个 Bevy app。
///
/// 注册：
/// - [`BevyGpuHandleCache`] resource（黄金路径默认值：容量 3000，
///   底层 zoom 3），
/// - [`PipelineEvictionStats`] 观测 resource，
/// - 在 `Update` 中的 [`gpu_cache_eviction_system`]。
///
/// # 推广护栏
/// 本插件在 M1.3 中**不**加入默认的 cesium-app 运行时；
/// `CESIUM_ENABLE_PIPELINE` 保持 OFF。向它新增内容对黄金路径是一个空操作，
/// 因为在 M1.4/M1.5 loader 迁移喂入它之前没有任何东西写入缓存——
/// 逐出系统每帧只会发现一个空缓存。
#[derive(Default, Debug, Clone, Copy)]
pub struct CesiumPipelinePlugin;

impl Plugin for CesiumPipelinePlugin {
    /// 初始化 GPU handle 缓存与逐出统计资源，并在 Update 挂载逐出系统。
    ///
    /// # 参数
    /// - `app`：Bevy 应用
    fn build(&self, app: &mut App) {
        app.init_resource::<BevyGpuHandleCache>()
            .init_resource::<PipelineEvictionStats>()
            .add_systems(Update, gpu_cache_eviction_system);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 该插件必须安装它的 resources + system，并在无头模式下运行而不
    /// panic（不需要 render/asset 插件）。
    #[test]
    fn plugin_installs_resources_and_runs() {
        let mut app = App::new();
        app.add_plugins(CesiumPipelinePlugin);
        app.update();

        assert!(app.world().contains_resource::<BevyGpuHandleCache>());
        assert!(app.world().contains_resource::<PipelineEvictionStats>());
        // 空缓存 → 逐出是空操作。
        assert!(app.world().resource::<BevyGpuHandleCache>().is_empty());
        let stats = app.world().resource::<PipelineEvictionStats>();
        assert_eq!(stats.evicted, 0);
        assert_eq!(stats.deferred, 0);
    }
}
