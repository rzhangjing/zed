//! 在途去重集合。
//!
//! 照搬 `dynamic_globe.rs` 中 `enqueue_tiles` 的去重逻辑（L406、L417）：
//! 一个已在 `in_flight` 或 `queued` 中的瓦片绝不会被重复提交，从而
//! 防止重复下载和冗余的网格构建。

use std::collections::HashSet;
use std::hash::Hash;
use std::sync::Mutex;

/// 面向在途瓦片请求的线程安全去重追踪器。
///
/// 对应 dynamic_globe.rs 中的 `TileManager::in_flight: HashSet<TileKey>` 和
/// `TileManager::queued: HashSet<TileKey>`。
pub struct Dedup<K: Hash + Eq + Copy> {
    /// 受互斥锁保护的在途/已排队键集（去重集本体）。
    inner: Mutex<HashSet<K>>,
}

impl<K: Hash + Eq + Copy> Dedup<K> {
    /// 创建一个空去重集。
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashSet::new()),
        }
    }

    /// 尝试插入一个键。若为新插入（非重复）则返回 `true`，
    /// 若已存在（被去重 —— 跳过提交）则返回 `false`。
    ///
    /// 对应 L406：`if mgr.tile_entities.contains_key(&key) || mgr.queued.contains(&key) { continue; }`
    /// 以及 L417：`!mgr.in_flight.contains(&key)`。
    pub fn insert(&self, key: K) -> bool {
        self.inner.lock().unwrap().insert(key)
    }

    /// 移除一个键（瓦片已完成或被取消）。
    ///
    /// 对应 L1051：`mgr.in_flight.remove(&key)`。
    pub fn remove(&self, key: &K) -> bool {
        self.inner.lock().unwrap().remove(key)
    }

    /// 检查一个键当前是否在途。
    pub fn contains(&self, key: &K) -> bool {
        self.inner.lock().unwrap().contains(key)
    }

    /// 当前在途键的数量。
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// 若没有在途键则返回 true。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 清空所有条目（例如在流水线重置时）。
    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }
}

impl<K: Hash + Eq + Copy> Default for Dedup<K> {
    /// 默认构造一个空去重集（等价于 [`Dedup::new`]）。
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TileKey = (u32, u32, u32);

    #[test]
    fn insert_new_key_returns_true() {
        let d = Dedup::<TileKey>::new();
        assert!(d.insert((1, 2, 3)));
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn insert_duplicate_returns_false() {
        let d = Dedup::<TileKey>::new();
        assert!(d.insert((1, 2, 3)));
        assert!(!d.insert((1, 2, 3)));
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn remove_allows_reinsert() {
        let d = Dedup::<TileKey>::new();
        d.insert((4, 5, 6));
        assert!(d.remove(&(4, 5, 6)));
        assert!(d.insert((4, 5, 6)));
    }

    #[test]
    fn contains_check() {
        let d = Dedup::<TileKey>::new();
        d.insert((7, 8, 9));
        assert!(d.contains(&(7, 8, 9)));
        assert!(!d.contains(&(0, 0, 0)));
    }

    #[test]
    fn clear_empties_set() {
        let d = Dedup::<TileKey>::new();
        d.insert((1, 1, 1));
        d.insert((2, 2, 2));
        d.clear();
        assert!(d.is_empty());
    }
}
