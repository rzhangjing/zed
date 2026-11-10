//! 带基础层锁定与活实体延迟的 FIFO GPU 缓存。
//!
//! 忠实复刻 `dynamic_globe.rs::evict_gpu_cache`（L1476-1502）：
//!
//! ```text
//! while gpu_tex_order.len() > MAX_GPU_CACHE_ENTRIES {
//!     let old = gpu_tex_order.pop_front();
//!     if old.z <= BASE_LAYER_ZOOM { continue; }       // invariant 1
//!     if tile_entities.contains(old) {                // invariant 2
//!         gpu_tex_order.push_back(old); deferred++;
//!         continue;
//!     }
//!     remove_handles(old); evicted++;
//! }
//! ```
//!
//! 三个不变式：
//! 1. BASE_LAYER 永久豁免（L1483）
//! 2. 活实体回推延迟（L1487-1491）—— 花屏防护核心
//! 3. 终止性：MAX_TILE_ENTITIES(1800) << MAX_GPU_CACHE_ENTRIES(3000)

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;

use cesium_ports_driven::EvictionPolicy;

use crate::base_layer::BaseLayerGuard;

/// 一次驱逐遍历的结果。
#[derive(Debug, Clone, Copy, Default)]
pub struct EvictionResult {
    /// 实际从缓存移除的条目。
    pub evicted: u32,
    /// 因拥有活实体而被延迟（回推）的条目。
    pub deferred: u32,
}

/// 带驱逐策略的 FIFO 序 GPU 句柄缓存。
///
/// 对键类型 `K` 通用（例如 `(u32, u32, u32)` = TileKey）。
/// 存储不透明的句柄值 `V`（例如 GPU 纹理 ID）。
pub struct GpuCache<K, V>
where
    K: Hash + Eq + Copy,
{
    /// FIFO 插入顺序（最旧在前）。对应 `mgr.gpu_tex_order`。
    order: VecDeque<K>,
    /// 实际缓存的值。对应 `mgr.gpu_textures` / `gpu_meshes` 等。
    entries: HashMap<K, V>,
    /// 触发驱逐前的最大条目数（L73：3000）。
    max_entries: usize,
    /// 用于永久豁免的基础层守卫。
    base_guard: BaseLayerGuard,
    /// Zoom 提取器：从键获取 zoom 分量。
    zoom_of: fn(&K) -> u32,
    /// 当前由活实体支撑的键（不变式 2 的延迟集）。
    /// 对应 `mgr.tile_entities` 的成员关系（L1487）。
    live: HashSet<K>,
}

impl<K, V> GpuCache<K, V>
where
    K: Hash + Eq + Copy,
{
    /// 创建一个使用给定容量与 zoom 提取器的缓存。
    ///
    /// `zoom_of` 从键提取 zoom 层级用于基础层检查。对于
    /// `TileKey = (u32, u32, u32)`，它就是 `|k| k.2`。
    pub fn new(max_entries: usize, base_guard: BaseLayerGuard, zoom_of: fn(&K) -> u32) -> Self {
        Self {
            order: VecDeque::new(),
            entries: HashMap::new(),
            max_entries,
            base_guard,
            zoom_of,
            live: HashSet::new(),
        }
    }

    /// 将一个键标记为由活实体支撑（或不再支撑）。
    ///
    /// 宿主在 spawn/despawn 一个瓦片实体时调用此方法，以便
    /// `defer_if_live`（不变式 2，L1487）反映当前的活集。
    pub fn set_live(&mut self, key: K, is_live: bool) {
        if is_live {
            self.live.insert(key);
        } else {
            self.live.remove(&key);
        }
    }

    /// 插入一个键值对。若键已存在，则更新值而不改变 FIFO 顺序
    /// （与 dynamic_globe 行为一致：重上传不会重置驱逐优先级）。
    pub fn insert(&mut self, key: K, value: V) {
        if !self.entries.contains_key(&key) {
            self.order.push_back(key);
        }
        self.entries.insert(key, value);
    }

    /// 获取一个缓存值的引用。
    pub fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key)
    }

    /// 检查一个键是否已缓存。
    pub fn contains_key(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    /// 从缓存中移除一个特定的键。
    pub fn remove(&mut self, key: &K) -> Option<V> {
        if let Some(v) = self.entries.remove(key) {
            // 从顺序队列移除（线性扫描 —— 对于显式移除（相比
            // FIFO 驱逐很罕见）是可以接受的）。
            if let Some(pos) = self.order.iter().position(|k| k == key) {
                self.order.remove(pos);
            }
            Some(v)
        } else {
            None
        }
    }

    /// 当前缓存条目的数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 若缓存为空则返回 true。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 运行 FIFO 驱逐直到 `len() <= max_entries`。
    ///
    /// `is_live` 谓词对应 L1487：`mgr.tile_entities.contains_key(&old)`。
    /// 返回驱逐/延迟计数供 PerfCounters 使用（M0.4 观测）。
    ///
    /// 忠实实现三个不变式：
    /// 1. 基础层键被跳过（永不驱逐）。
    /// 2. 活实体键被回推（延迟）。
    /// 3. 因活实体 << max_entries 而保证终止。
    pub fn evict<F>(&mut self, is_live: F) -> EvictionResult
    where
        F: Fn(&K) -> bool,
    {
        let mut result = EvictionResult::default();

        while self.order.len() > self.max_entries {
            let Some(old) = self.order.pop_front() else {
                break;
            };

            // 不变式 1：基础层永久豁免（L1483）
            let zoom = (self.zoom_of)(&old);
            if self.base_guard.is_base_layer(zoom) {
                // 不计为驱逐 —— 只是跳过。该条目保留在
                // `entries` 中但从 `order` 移除（它永远不会被
                // 驱逐，因此追踪其顺序毫无意义）。
                continue;
            }

            // 不变式 2：活实体延迟（L1487-1491）
            if is_live(&old) {
                self.order.push_back(old);
                result.deferred += 1;
                continue;
            }

            // 实际驱逐
            self.entries.remove(&old);
            result.evicted += 1;
        }

        result
    }

    /// 对 FIFO 顺序的只读访问（用于 `EvictionPolicy` trait 实现）。
    pub fn order(&self) -> &VecDeque<K> {
        &self.order
    }
}

/// `EvictionPolicy` 契约实现（M1.1），表达三个不变式。
///
/// 相比 inherent impl，额外的 `Send + 'static` 约束是端口 trait 所
/// 要求的；它们由 `TileKey = (u32, u32, u32)` 满足。
impl<K, V> EvictionPolicy<K> for GpuCache<K, V>
where
    K: Hash + Eq + Copy + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    /// 不变式：FIFO 顺序，最旧在前（`mgr.gpu_tex_order`，L1479）。
    fn evict_order(&self) -> &VecDeque<K> {
        &self.order
    }

    /// 不变式 1（L1483）：基础层瓦片永不驱逐。
    fn never_evict(&self, key: &K) -> bool {
        self.base_guard.is_base_layer((self.zoom_of)(key))
    }

    /// 不变式 2（L1487）：拥有活实体的瓦片会被延迟（回推）。
    fn defer_if_live(&self, key: &K) -> bool {
        self.live.contains(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TileKey = (u32, u32, u32);

    fn zoom_of(k: &TileKey) -> u32 {
        k.2
    }

    fn make_cache(max: usize) -> GpuCache<TileKey, u64> {
        GpuCache::new(max, BaseLayerGuard::new(), zoom_of)
    }

    #[test]
    fn fifo_eviction_removes_oldest() {
        let mut cache = make_cache(3);
        cache.insert((0, 0, 4), 100);
        cache.insert((1, 0, 4), 200);
        cache.insert((2, 0, 4), 300);
        cache.insert((3, 0, 4), 400); // 超出上限

        let result = cache.evict(|_| false);
        assert_eq!(result.evicted, 1);
        assert_eq!(result.deferred, 0);
        assert!(!cache.contains_key(&(0, 0, 4))); // 最旧的被驱逐
        assert!(cache.contains_key(&(3, 0, 4))); // 最新的保留
    }

    #[test]
    fn base_layer_never_evicted() {
        // 在足够的压力下，普通瓦片会被驱逐但基础层存活。
        // 从 order 弹出的基础层条目会释放空位（与 dynamic_globe 一致，
        // L1483：`continue` 从 gpu_tex_order 移除但保留 GPU 句柄）。
        let mut cache = make_cache(2);
        cache.insert((0, 0, 3), 1); // 基础层（z=3）
        cache.insert((1, 1, 5), 2); // 普通
        cache.insert((2, 2, 5), 3); // 普通
        cache.insert((3, 3, 5), 4); // 普通

        // order len=4 > max=2：
        // Pop (0,0,3)：基础层 → 跳过（从 order 释放，保留在 entries 中）
        // order len=3 > 2：Pop (1,1,5)：普通、非 live → 驱逐
        // order len=2，不再 > 2：退出
        let result = cache.evict(|_| false);
        assert_eq!(result.evicted, 1);
        assert!(cache.contains_key(&(0, 0, 3)));  // 基础层永不驱逐
        assert!(!cache.contains_key(&(1, 1, 5))); // 已驱逐
        assert!(cache.contains_key(&(2, 2, 5)));  // 仍在缓存
        assert!(cache.contains_key(&(3, 3, 5)));  // 仍在缓存
    }

    #[test]
    fn live_entity_deferred() {
        let mut cache = make_cache(2);
        cache.insert((1, 1, 5), 10);
        cache.insert((2, 2, 5), 20);
        cache.insert((3, 3, 5), 30); // 超出上限

        // 将最旧的标记为 "live"
        let result = cache.evict(|k| *k == (1, 1, 5));
        assert_eq!(result.deferred, 1);
        // (1,1,5) 被回推，改为驱逐 (2,2,5)
        assert!(cache.contains_key(&(1, 1, 5)));
        assert!(!cache.contains_key(&(2, 2, 5)));
    }

    #[test]
    fn insert_duplicate_does_not_reorder() {
        let mut cache = make_cache(10);
        cache.insert((1, 1, 4), 100);
        cache.insert((2, 2, 4), 200);
        cache.insert((1, 1, 4), 999); // 更新值，保留位置

        assert_eq!(cache.get(&(1, 1, 4)), Some(&999));
        assert_eq!(cache.order()[0], (1, 1, 4)); // 仍在最前
    }

    #[test]
    fn eviction_policy_trait_expresses_three_invariants() {
        // 直接练习端口 `EvictionPolicy<K>` 实现（dyn 兼容）。
        let mut cache = make_cache(3000);
        cache.insert((0, 0, 3), 1); // 基础层（z=3）
        cache.insert((1, 1, 5), 2); // 普通
        cache.set_live((1, 1, 5), true);

        let policy: &dyn EvictionPolicy<TileKey> = &cache;
        // 不变式：暴露 FIFO 顺序。
        assert_eq!(policy.evict_order().len(), 2);
        // 不变式 1：基础层（z<=3）永不驱逐。
        assert!(policy.never_evict(&(0, 0, 3)));
        assert!(!policy.never_evict(&(1, 1, 5)));
        // 不变式 2：活实体会被延迟。
        assert!(policy.defer_if_live(&(1, 1, 5)));
        assert!(!policy.defer_if_live(&(0, 0, 3)));
    }
}
