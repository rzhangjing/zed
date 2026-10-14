//! 面向温回退瓦片的隐藏实体 LRU 追踪。
//!
//! 照搬 `dynamic_globe.rs::process_pipeline` 中的 despawn/hidden-LRU 阶段
//! （L1250-1322）：离开可见集的瓦片会被隐藏（Visibility::Hidden）
//! 而非立即 despawn，因此一次缩小（zoom-out）可以重新分区到粗瓦片的
//! 活实体上，而不会闪降到基础球体。
//!
//! 当超过 `MAX_TILE_ENTITIES`（1800）时，最久被隐藏的瓦片会最先被
//! despawn（在 `MAX_DESPAWNS_PER_FRAME` = 24 预算内）。

use std::collections::HashMap;
use std::hash::Hash;

/// 面向隐藏（温回退）瓦片实体的 LRU 追踪器。
///
/// 瓦片在离开可见集时被移入这里。单调递增的 `tick` 决定驱逐顺序
/// （最小的 tick = 最旧 = 超预算时最先 despawn）。
pub struct HiddenLru<K: Hash + Eq + Copy> {
    /// 从瓦片键到其被隐藏时 tick 的映射。
    entries: HashMap<K, u64>,
    /// 单调 tick 计数器（每帧递增）。
    tick: u64,
}

impl<K: Hash + Eq + Copy> HiddenLru<K> {
    /// 创建一个空的 LRU 追踪器。
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            tick: 0,
        }
    }

    /// 推进帧 tick。在 hide/show 操作之前每帧调用一次。
    pub fn advance_frame(&mut self) {
        self.tick += 1;
    }

    /// 将一个瓦片标记为隐藏（离开可见集）。记录当前 tick 用于 LRU 顺序。
    ///
    /// 对应 L1250-1280：实体被设为 Visibility::Hidden，移入隐藏温池。
    pub fn hide(&mut self, key: K) {
        self.entries.insert(key, self.tick);
    }

    /// 将一个瓦片重新标记为可见（重新进入可见集）。从 LRU 中移除。
    ///
    /// 对应 L1285-1300：缩小（zoom-out）时重新激活隐藏瓦片。
    pub fn show(&mut self, key: &K) -> bool {
        self.entries.remove(key).is_some()
    }

    /// 检查一个瓦片当前是否隐藏。
    pub fn is_hidden(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    /// 隐藏瓦片的数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 若没有瓦片被隐藏则返回 true。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 弹出最多 `budget` 个最久被隐藏的键用于 despawn。
    ///
    /// 对应 L1300-1322：当超过 `MAX_TILE_ENTITIES` 时，在
    /// `MAX_DESPAWNS_PER_FRAME`（24）预算内 despawn 最旧的隐藏瓦片。
    pub fn pop_lru(&mut self, budget: usize) -> Vec<K> {
        if self.entries.is_empty() || budget == 0 {
            return Vec::new();
        }

        let mut sorted: Vec<(K, u64)> = self.entries.drain().collect();
        sorted.sort_by_key(|(_, tick)| *tick);

        let take = budget.min(sorted.len());
        let evicted: Vec<K> = sorted.drain(..take).map(|(k, _)| k).collect();

        // 将剩余的条目放回
        self.entries.extend(sorted);

        evicted
    }
}

impl<K: Hash + Eq + Copy> Default for HiddenLru<K> {
    /// 默认构造一个空隐藏 LRU（等价于 [`HiddenLru::new`]）。
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TileKey = (u32, u32, u32);

    #[test]
    fn hide_and_show() {
        let mut lru = HiddenLru::<TileKey>::new();
        lru.advance_frame();
        lru.hide((1, 1, 5));
        assert!(lru.is_hidden(&(1, 1, 5)));
        assert_eq!(lru.len(), 1);

        assert!(lru.show(&(1, 1, 5)));
        assert!(!lru.is_hidden(&(1, 1, 5)));
        assert_eq!(lru.len(), 0);
    }

    #[test]
    fn pop_lru_evicts_oldest_first() {
        let mut lru = HiddenLru::<TileKey>::new();

        lru.advance_frame(); // tick=1
        lru.hide((1, 0, 4));

        lru.advance_frame(); // tick=2
        lru.hide((2, 0, 4));

        lru.advance_frame(); // tick=3
        lru.hide((3, 0, 4));

        let evicted = lru.pop_lru(2);
        assert_eq!(evicted, vec![(1, 0, 4), (2, 0, 4)]);
        assert_eq!(lru.len(), 1);
        assert!(lru.is_hidden(&(3, 0, 4)));
    }

    #[test]
    fn pop_lru_respects_budget() {
        let mut lru = HiddenLru::<TileKey>::new();
        lru.advance_frame();
        for i in 0..10 {
            lru.hide((i, 0, 5));
        }
        let evicted = lru.pop_lru(3);
        assert_eq!(evicted.len(), 3);
        assert_eq!(lru.len(), 7);
    }

    #[test]
    fn show_prevents_eviction() {
        let mut lru = HiddenLru::<TileKey>::new();
        lru.advance_frame();
        lru.hide((1, 0, 4));
        lru.advance_frame();
        lru.hide((2, 0, 4));

        // 重新 show 最旧的那个
        lru.show(&(1, 0, 4));

        let evicted = lru.pop_lru(5);
        assert_eq!(evicted, vec![(2, 0, 4)]);
    }
}
