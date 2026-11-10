//! pipeline 绑定层的 Bevy Component/Resource 桥接。
//!
//! 这些是最小化的 ECS 胶水类型，让 core pipeline 缓存能与
//! Bevy entity 互操作。它们不携带任何逻辑。

use bevy::prelude::*;

use super::TileKey;

/// 把一个 entity 标记为 pipeline 管理 tile 的 marker component。
///
/// 携带 tile key，以便系统能把 entity ↔ GPU-cache 条目映射并构建
/// 逐出推迟不变式所使用的 live entity 集合
/// （`dynamic_globe.rs:1487`，`mgr.tile_entities.contains_key(&old)`）。
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PipelineTile {
    /// 本 entity 渲染的 tile key `(x, y, zoom)`。
    pub key: TileKey,
}

impl PipelineTile {
    /// 为给定的 tile key 创建一个 marker。
    pub fn new(key: TileKey) -> Self {
        Self { key }
    }
}

/// 逐帧的逐出计数器，由逐出系统写入。
///
/// 纯观测（M0.4 风格）：镜像被插桩的 `evict_gpu_cache`
/// （`dynamic_globe.rs:1476`）的 `(evicted, deferred)` 返回值。喂给
/// M1.1 `PipelineStats` CSV 映射的 `evict_n` / `evict_deferred` 列。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineEvictionStats {
    /// 本帧被逐出的 GPU handle 条目。
    pub evicted: u32,
    /// 本帧被推迟的条目（live entity 回推，花屏防护）。
    pub deferred: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_tile_carries_key() {
        let t = PipelineTile::new((4, 5, 6));
        assert_eq!(t.key, (4, 5, 6));
        assert_eq!(t.key.2, 6); // zoom 分量驱动底层检查
    }

    #[test]
    fn eviction_stats_default_zeroed() {
        let s = PipelineEvictionStats::default();
        assert_eq!(s.evicted, 0);
        assert_eq!(s.deferred, 0);
    }
}
