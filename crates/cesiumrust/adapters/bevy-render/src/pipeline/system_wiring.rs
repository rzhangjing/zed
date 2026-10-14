//! pipeline 绑定层的 Bevy 系统装配。
//!
//! 这里的系统是**帧线程安全的**：它们只执行有界、纯 CPU 的
//! ECS 工作（构建一个 live-key 集合，把一次逐出遍历委派给 core）。
//! 无网络、无阻塞、无 tokio——下载都留在 core 的 ureq worker
//! 池上，离开帧线程。

use std::collections::HashSet;

use bevy::prelude::*;

use super::bindings::{PipelineEvictionStats, PipelineTile};
use super::gpu_handle::BevyGpuHandleCache;
use super::TileKey;

/// 逐帧的 GPU handle 缓存逐出。
///
/// 从 `PipelineTile` entity 构建 live-key 集合——`dynamic_globe.rs:1487`
/// （`mgr.tile_entities.contains_key(&old)`）的直接类比——然后
/// 把整个逐出遍历委派给 core 缓存。被逐出的 Bevy `Handle`
/// 在 core 内部被 drop（释放 GPU asset 强引用，L1493-1498）。
///
/// 三个不变式（底层豁免、live entity 推迟、FIFO
/// 终止性）由 core 的 `EvictionPolicy` 强制执行，而非这里。
pub fn gpu_cache_eviction_system(
    mut cache: ResMut<BevyGpuHandleCache>,
    tiles: Query<&PipelineTile>,
    mut stats: ResMut<PipelineEvictionStats>,
) {
    let live: HashSet<TileKey> = tiles.iter().map(|t| t.key).collect();
    let result = cache.evict(|key| live.contains(key));
    stats.evicted = result.evicted;
    stats.deferred = result.deferred;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::gpu_handle::GpuTileHandles;

    fn handles(id: u128) -> GpuTileHandles {
        GpuTileHandles {
            texture: Handle::weak_from_u128(id),
            mesh: None,
            material: None,
        }
    }

    /// 端到端：该系统推迟一个 live tile、逐出一个 dead tile，并把
    /// 计数记录进 `PipelineEvictionStats`。
    #[test]
    fn eviction_system_defers_live_and_evicts_dead() {
        let mut app = App::new();
        app.init_resource::<PipelineEvictionStats>();

        let mut cache = BevyGpuHandleCache::new(2, 3);
        cache.insert((1, 1, 5), handles(1)); // 最旧——将为 LIVE
        cache.insert((2, 2, 5), handles(2)); // dead
        cache.insert((3, 3, 5), handles(3)); // dead，最新
        app.insert_resource(cache);

        // 一个覆盖最旧 key 的 live entity。
        app.world_mut().spawn(PipelineTile::new((1, 1, 5)));
        app.add_systems(Update, gpu_cache_eviction_system);

        app.update();

        let stats = *app.world().resource::<PipelineEvictionStats>();
        assert_eq!(stats.deferred, 1, "live tile must be deferred");
        assert_eq!(stats.evicted, 1, "one dead tile must be evicted");

        let cache = app.world().resource::<BevyGpuHandleCache>();
        assert!(cache.contains_key(&(1, 1, 5)), "live survives");
        assert!(!cache.contains_key(&(2, 2, 5)), "oldest dead evicted");
        assert!(cache.contains_key(&(3, 3, 5)), "newest survives");
    }

    /// 当没有 live entity 且缓存未超额时，该系统是一个空操作。
    #[test]
    fn eviction_system_noop_under_capacity() {
        let mut app = App::new();
        app.init_resource::<PipelineEvictionStats>();

        let mut cache = BevyGpuHandleCache::new(10, 3);
        cache.insert((1, 1, 5), handles(1));
        cache.insert((2, 2, 5), handles(2));
        app.insert_resource(cache);
        app.add_systems(Update, gpu_cache_eviction_system);

        app.update();

        let stats = *app.world().resource::<PipelineEvictionStats>();
        assert_eq!(stats.evicted, 0);
        assert_eq!(stats.deferred, 0);
        assert_eq!(app.world().resource::<BevyGpuHandleCache>().len(), 2);
    }
}
