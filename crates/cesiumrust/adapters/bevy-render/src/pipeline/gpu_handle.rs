//! 一个带 Bevy `Handle` 类型的 GPU 缓存 —— 对 core
//! `cesium_pipeline::GpuCache` 的一层薄包装。
//!
//! 所有逐出逻辑（`dynamic_globe.rs::evict_gpu_cache` 的三条不变式，
//! L1476-1502）都住在 core crate。本包装仅：
//! 1. 把值类型固定为 Bevy 资产 `Handle`。
//! 2. 在逐出时释放 GPU 资产 —— drop 一个 Bevy `Handle` 会递减该资产的
//!    强引用计数，这正是 L1493-1498 处的"移除 handle"那一步
//!    （`gpu_textures.remove(&old)` 等）。
//! 3. 把 live-entity 谓词桥接到一个 Bevy `Query`（见 `system_wiring`）。
//!
//! 三条不变式（委派，而非重新实现）：
//! - **BASE_LAYER 豁免**（L1483）：`z <= BASE_LAYER_ZOOM` 从不被逐出。
//! - **Live-entity 推迟**（L1487-1491）：live tile 被推回（花屏防护核心）。
//! - **终止性**（L1471-1472）：`MAX_TILE_ENTITIES(1800) << MAX_GPU_CACHE_ENTRIES(3000)`。

use bevy::prelude::*;
use cesium_pipeline::base_layer::BaseLayerGuard;
use cesium_pipeline::gpu_cache::{EvictionResult, GpuCache};
use cesium_pipeline::DefaultBudget;
use cesium_ports_driven::EvictionPolicy;

use super::TileKey;

/// 单个 tile 的 GPU 资产 handle（Bevy 强引用）。
///
/// 对应 `dynamic_globe.rs` 里的逐 tile handle map（`gpu_textures`、
/// `gpu_meshes`、`gpu_materials`，L1493-1495）。drop 这些会释放
/// 底层 GPU 资产。
pub struct GpuTileHandles {
    /// 影像/albedo 贴图 handle（`gpu_textures`，L1493）。
    pub texture: Handle<Image>,
    /// 地形/tile mesh handle（`gpu_meshes`，L1495）。
    pub mesh: Option<Handle<Mesh>>,
    /// PBR 材质 handle（`gpu_materials`，L1494）。
    pub material: Option<Handle<StandardMaterial>>,
}

/// 一个 Bevy `Resource`，包装 core 的 FIFO GPU handle 缓存。
///
/// 重活（FIFO 顺序、底层锁、live 推迟、终止性）都由
/// `cesium_pipeline::GpuCache` 承担，它同时也实现了 M1.1 的
/// `EvictionPolicy<TileKey>` 契约。见 [`BevyGpuHandleCache::as_eviction_policy`]。
#[derive(Resource)]
pub struct BevyGpuHandleCache {
    /// 被包装的 core 层 FIFO GPU handle 缓存（同时作为逐出策略）。
    inner: GpuCache<TileKey, GpuTileHandles>,
}

impl BevyGpuHandleCache {
    /// 以一个显式容量与底层 zoom 创建缓存。
    ///
    /// 对 `TileKey = (x, y, zoom)`，`zoom_of` 是 `|k| k.2`，对应
    /// `dynamic_globe.rs:1483` 处的 `old.2 <= BASE_LAYER_ZOOM` 检查。
    pub fn new(max_entries: usize, base_layer_zoom: u32) -> Self {
        Self {
            inner: GpuCache::new(
                max_entries,
                BaseLayerGuard::with_zoom(base_layer_zoom),
                |k: &TileKey| k.2,
            ),
        }
    }

    /// 使用黄金路径默认值创建一个缓存
    /// （`MAX_GPU_CACHE_ENTRIES = 3000`、`BASE_LAYER_ZOOM = 3`）。
    pub fn with_defaults() -> Self {
        Self::new(
            DefaultBudget::MAX_GPU_CACHE_ENTRIES,
            DefaultBudget::BASE_LAYER_ZOOM,
        )
    }

    /// 插入（或更新）一个 tile 的 GPU handle。重新插入一个已存在的
    /// key 不改变它的 FIFO 位置（对应 dynamic_globe：重新上传
    /// 不重置逐出优先级）。
    pub fn insert(&mut self, key: TileKey, handles: GpuTileHandles) {
        self.inner.insert(key, handles);
    }

    /// 借用某个 tile 已缓存的 handle。
    pub fn get(&self, key: &TileKey) -> Option<&GpuTileHandles> {
        self.inner.get(key)
    }

    /// 某个 tile 当前是否拥有已缓存的 GPU handle。
    pub fn contains_key(&self, key: &TileKey) -> bool {
        self.inner.contains_key(key)
    }

    /// 显式移除某个 tile 的 handle（返回它们，以便调用方
    /// 保留或丢弃）。相对 FIFO 逐出而言罕见。
    pub fn remove(&mut self, key: &TileKey) -> Option<GpuTileHandles> {
        self.inner.remove(key)
    }

    /// 已缓存 tile 的数量。
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// 缓存是否为空。
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// 标记一个 tile 是否由一个 live entity 支撑（backed），喂给不变式 2。
    /// 委派给 core 缓存的 live-set。
    pub fn set_live(&mut self, key: TileKey, live: bool) {
        self.inner.set_live(key, live);
    }

    /// 运行一趟 FIFO 逐出，为被逐出的 tile 释放 GPU handle。
    ///
    /// `is_live` 对应 `dynamic_globe.rs:1487`
    /// （`mgr.tile_entities.contains_key(&old)`）。被逐出的 handle 在 core 缓存
    /// 内部被 drop，释放它们的 Bevy 资产强引用 —— 这是绑定层对
    /// L1493-1498（`gpu_*.remove(&old)`）的等价物。
    ///
    /// 返回 `(evicted, deferred)` 计数以供观测，恰如带 M0.4 插桩的
    /// `evict_gpu_cache` 返回值。
    pub fn evict<F>(&mut self, is_live: F) -> EvictionResult
    where
        F: Fn(&TileKey) -> bool,
    {
        self.inner.evict(is_live)
    }

    /// 把这个缓存视作 core 的 M1.1 `EvictionPolicy<TileKey>` trait 对象。
    ///
    /// 用以说明绑定层并不重新实现逐出：三条不变式
    /// 都是直接从 core 契约查询而来。
    pub fn as_eviction_policy(&self) -> &dyn EvictionPolicy<TileKey> {
        &self.inner
    }
}

impl Default for BevyGpuHandleCache {
    /// 默认：转发到 [`BevyGpuHandleCache::with_defaults`] 的默认容量/zoom。
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_ports_driven::BudgetPolicy;

    /// 构建一个占位 handle 集（payload 身份无关紧要 —— 逐出
    /// 完全由 tile key 驱动）。
    fn handles(id: u128) -> GpuTileHandles {
        GpuTileHandles {
            texture: Handle::weak_from_u128(id),
            mesh: None,
            material: None,
        }
    }

    /// 不变式 3（FIFO 顺序）+ 逐出：最旧的*死*条目先走。
    /// 对应 `dynamic_globe.rs:1479-1500`。
    #[test]
    fn fifo_eviction_drops_oldest_dead() {
        let mut cache = BevyGpuHandleCache::new(3, 3);
        cache.insert((0, 0, 5), handles(1));
        cache.insert((1, 0, 5), handles(2));
        cache.insert((2, 0, 5), handles(3));
        cache.insert((3, 0, 5), handles(4)); // 超出容量 → 逐出 1

        let r = cache.evict(|_| false);
        assert_eq!(r.evicted, 1);
        assert_eq!(r.deferred, 0);
        assert!(!cache.contains_key(&(0, 0, 5)), "oldest must be evicted");
        assert!(cache.contains_key(&(3, 0, 5)), "newest must be kept");
    }

    /// 不变式 1（BASE_LAYER 永久豁免）：`z <= 3` 从不被逐出。
    /// 对应 `dynamic_globe.rs:1483`（`if old.2 <= BASE_LAYER_ZOOM { continue }`）。
    #[test]
    fn base_layer_permanently_exempt() {
        let mut cache = BevyGpuHandleCache::new(2, 3);
        cache.insert((0, 0, 3), handles(1)); // 底层（z=3）
        cache.insert((1, 1, 5), handles(2)); // 普通
        cache.insert((2, 2, 5), handles(3)); // 普通
        cache.insert((3, 3, 5), handles(4)); // 普通

        let r = cache.evict(|_| false);
        assert_eq!(r.evicted, 1);
        assert!(cache.contains_key(&(0, 0, 3)), "base layer must survive");
        assert!(!cache.contains_key(&(1, 1, 5)), "oldest normal evicted");
    }

    /// 不变式 2（live-entity 推回推迟 —— 花屏防护核心）。
    /// 对应 `dynamic_globe.rs:1487-1491`。
    #[test]
    fn live_entity_deferred_and_pushed_back() {
        let mut cache = BevyGpuHandleCache::new(2, 3);
        cache.insert((1, 1, 5), handles(1)); // 最旧，但 LIVE
        cache.insert((2, 2, 5), handles(2));
        cache.insert((3, 3, 5), handles(3)); // 超出容量

        // 将最旧的那个标为 live：它必须被推迟，而是逐出下一个
        // 死条目。
        let r = cache.evict(|k| *k == (1, 1, 5));
        assert_eq!(r.deferred, 1);
        assert!(cache.contains_key(&(1, 1, 5)), "live tile must be deferred");
        assert!(!cache.contains_key(&(2, 2, 5)), "dead tile evicted instead");
    }

    /// 不变式 3（终止性）：因为 live entity 的上限远低于
    /// 缓存容量，逐出总是终止并释放死条目。
    /// 对应 `dynamic_globe.rs:1471-1472`。
    #[test]
    fn termination_guaranteed_live_below_capacity() {
        // 全局不变式：MAX_TILE_ENTITIES(1800) << MAX_GPU_CACHE_ENTRIES(3000)。
        // 经由 BudgetPolicy 契约读取（运行时值，而非 const）。
        let budget = DefaultBudget;
        assert!(budget.max_tile_entities() < budget.max_gpu_cache_entries());

        let mut cache = BevyGpuHandleCache::new(5, 3);
        let live_a = (6, 0, 5);
        let live_b = (7, 0, 5);
        for i in 0..6u32 {
            cache.insert((i, 0, 5), handles(u128::from(i))); // 前端 6 个 dead
        }
        cache.insert(live_a, handles(100)); // 后端 2 个 live
        cache.insert(live_b, handles(101));

        // 8 个条目，上限 5 → 恰好逐出最旧的 3 个 dead；那 2 个 live tile
        // 位于尾部，永不被触及。确定性地终止。
        let r = cache.evict(|k| *k == live_a || *k == live_b);
        assert_eq!(r.evicted, 3);
        assert_eq!(r.deferred, 0);
        assert_eq!(cache.len(), 5);
        assert!(cache.contains_key(&live_a) && cache.contains_key(&live_b));
    }

    /// 该包装将不变式委派给 core 的 `EvictionPolicy`
    /// 契约，而非重新实现它们。
    #[test]
    fn delegates_to_core_eviction_policy() {
        let mut cache = BevyGpuHandleCache::with_defaults();
        cache.insert((0, 0, 3), handles(1)); // 底层
        cache.set_live((0, 0, 3), true);

        let policy = cache.as_eviction_policy();
        // 不变式：从 core 暴露的 FIFO 顺序。
        assert_eq!(policy.evict_order().len(), 1);
        // 不变式 1：底层（z<=3）从不被逐出；z>3 可逐出。
        assert!(policy.never_evict(&(0, 0, 3)));
        assert!(!policy.never_evict(&(0, 0, 5)));
        // 不变式 2：live-entity 推迟标志取自 core 的 live-set。
        assert!(policy.defer_if_live(&(0, 0, 3)));
    }

    /// `with_defaults` 必须使用黄金路径的容量 + 底层 zoom。
    #[test]
    fn defaults_match_golden_path_constants() {
        let cache = BevyGpuHandleCache::with_defaults();
        let policy = cache.as_eviction_policy();
        // BASE_LAYER_ZOOM = 3 → z=3 豁免，z=4 不豁免。
        assert!(policy.never_evict(&(0, 0, 3)));
        assert!(!policy.never_evict(&(0, 0, 4)));
        assert!(cache.is_empty());
    }
}
