//! Scene/QuadtreePrimitive 瓦片缓存与加载队列 → Rust 集成测试。
//!
//! 参考 CesiumJS：
//! - Scene/QuadtreePrimitive（瓦片加载/缓存行为）
//!
//! A 类测试：TileLoadQueue 优先级/距离排序、max_size 淘汰、
//! TileCache LRU 淘汰/访问排序、SchedulerConfig/Stats 默认值。
//! C 类省略：WebGL 渲染、异步网络加载。

use cesium_quadtree::{
    QueuedTile, SchedulerConfig, SchedulerStats, TileCache, TileId, TileLoadQueue, TilePriority,
};

fn make_queued(id: TileId, priority: TilePriority, distance: f64) -> QueuedTile {
    QueuedTile {
        id,
        priority,
        frame_number: 1,
        distance,
    }
}

// === TileId ===

#[test]
fn tile_id_creation() {
    let id = TileId::new(3, 7, 7);
    assert_eq!(id.x, 3);
    assert_eq!(id.y, 7);
    assert_eq!(id.level, 7);
}

#[test]
fn tile_id_equality() {
    let a = TileId::new(1, 2, 3);
    let b = TileId::new(1, 2, 3);
    let c = TileId::new(1, 2, 4);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// === TilePriority ===

#[test]
fn priority_ordering() {
    assert!(TilePriority::Critical > TilePriority::High);
    assert!(TilePriority::High > TilePriority::Normal);
    assert!(TilePriority::Normal > TilePriority::Low);
}

#[test]
fn priority_default_is_normal() {
    assert_eq!(TilePriority::default(), TilePriority::Normal);
}

// === TileLoadQueue ===

#[test]
fn queue_new_is_empty() {
    let queue = TileLoadQueue::new(10);
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
}

#[test]
fn queue_enqueue_dequeue_single() {
    let mut queue = TileLoadQueue::new(10);
    queue.enqueue(make_queued(TileId::new(0, 0, 0), TilePriority::Normal, 500.0));
    assert_eq!(queue.len(), 1);

    let tile = queue.dequeue().unwrap();
    assert_eq!(tile.id, TileId::new(0, 0, 0));
    assert!(queue.is_empty());
}

#[test]
fn queue_dequeue_empty_returns_none() {
    let mut queue = TileLoadQueue::new(10);
    assert!(queue.dequeue().is_none());
}

#[test]
fn queue_dequeue_by_priority() {
    let mut queue = TileLoadQueue::new(10);
    queue.enqueue(make_queued(TileId::new(0, 0, 0), TilePriority::Low, 100.0));
    queue.enqueue(make_queued(TileId::new(1, 1, 1), TilePriority::Critical, 200.0));
    queue.enqueue(make_queued(TileId::new(2, 2, 2), TilePriority::Normal, 50.0));
    queue.enqueue(make_queued(TileId::new(3, 3, 3), TilePriority::High, 300.0));

    let first = queue.dequeue().unwrap();
    assert_eq!(first.priority, TilePriority::Critical);

    let second = queue.dequeue().unwrap();
    assert_eq!(second.priority, TilePriority::High);

    let third = queue.dequeue().unwrap();
    assert_eq!(third.priority, TilePriority::Normal);

    let fourth = queue.dequeue().unwrap();
    assert_eq!(fourth.priority, TilePriority::Low);
}

#[test]
fn queue_distance_tiebreak_same_priority() {
    let mut queue = TileLoadQueue::new(10);
    queue.enqueue(make_queued(TileId::new(0, 0, 0), TilePriority::Normal, 5000.0));
    queue.enqueue(make_queued(TileId::new(1, 1, 1), TilePriority::Normal, 100.0));
    queue.enqueue(make_queued(TileId::new(2, 2, 2), TilePriority::Normal, 1000.0));

    // 最近的在前（相同优先级）
    let first = queue.dequeue().unwrap();
    assert_eq!(first.id, TileId::new(1, 1, 1));
    let second = queue.dequeue().unwrap();
    assert_eq!(second.id, TileId::new(2, 2, 2));
    let third = queue.dequeue().unwrap();
    assert_eq!(third.id, TileId::new(0, 0, 0));
}

#[test]
fn queue_max_size_evicts_lowest_priority() {
    let mut queue = TileLoadQueue::new(2);
    queue.enqueue(make_queued(TileId::new(0, 0, 0), TilePriority::Low, 100.0));
    queue.enqueue(make_queued(TileId::new(1, 1, 1), TilePriority::Normal, 100.0));
    // 这应淘汰 Low 优先级瓦片
    queue.enqueue(make_queued(TileId::new(2, 2, 2), TilePriority::High, 100.0));

    assert_eq!(queue.len(), 2);
    // 剩余的应为 Normal 和 High
    let first = queue.dequeue().unwrap();
    assert_eq!(first.priority, TilePriority::High);
    let second = queue.dequeue().unwrap();
    assert_eq!(second.priority, TilePriority::Normal);
}

#[test]
fn queue_clear() {
    let mut queue = TileLoadQueue::new(10);
    queue.enqueue(make_queued(TileId::new(0, 0, 0), TilePriority::Normal, 100.0));
    queue.enqueue(make_queued(TileId::new(1, 1, 1), TilePriority::High, 200.0));
    queue.clear();
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
}

// === TileCache (LRU) ===

#[test]
fn cache_new_is_empty() {
    let cache: TileCache<i32> = TileCache::new(10);
    assert!(cache.is_empty());
    assert_eq!(cache.len(), 0);
}

#[test]
fn cache_insert_and_get() {
    let mut cache = TileCache::new(10);
    cache.insert(TileId::new(0, 0, 0), 42);
    cache.insert(TileId::new(1, 1, 1), 99);

    assert_eq!(cache.get(&TileId::new(0, 0, 0)), Some(&42));
    assert_eq!(cache.get(&TileId::new(1, 1, 1)), Some(&99));
    assert_eq!(cache.get(&TileId::new(2, 2, 2)), None);
}

#[test]
fn cache_contains() {
    let mut cache = TileCache::new(10);
    cache.insert(TileId::new(5, 3, 2), "hello");
    assert!(cache.contains(&TileId::new(5, 3, 2)));
    assert!(!cache.contains(&TileId::new(0, 0, 0)));
}

#[test]
fn cache_lru_eviction_order() {
    let mut cache = TileCache::new(3);
    cache.insert(TileId::new(0, 0, 0), "a");
    cache.insert(TileId::new(1, 1, 1), "b");
    cache.insert(TileId::new(2, 2, 2), "c");
    // 缓存已满：[a, b, c]。插入 d → 淘汰 a（LRU）
    cache.insert(TileId::new(3, 3, 3), "d");

    assert_eq!(cache.len(), 3);
    assert!(!cache.contains(&TileId::new(0, 0, 0))); // 被淘汰
    assert!(cache.contains(&TileId::new(1, 1, 1)));
    assert!(cache.contains(&TileId::new(2, 2, 2)));
    assert!(cache.contains(&TileId::new(3, 3, 3)));
}

#[test]
fn cache_access_refreshes_lru() {
    let mut cache = TileCache::new(3);
    cache.insert(TileId::new(0, 0, 0), "a");
    cache.insert(TileId::new(1, 1, 1), "b");
    cache.insert(TileId::new(2, 2, 2), "c");

    // 访问 'a' 刷新它
    cache.get(&TileId::new(0, 0, 0));

    // 插入 d → 应淘汰 'b'（现为 LRU），而非 'a'
    cache.insert(TileId::new(3, 3, 3), "d");

    assert!(cache.contains(&TileId::new(0, 0, 0))); // 已刷新
    assert!(!cache.contains(&TileId::new(1, 1, 1))); // 被淘汰
    assert!(cache.contains(&TileId::new(2, 2, 2)));
    assert!(cache.contains(&TileId::new(3, 3, 3)));
}

#[test]
fn cache_take_evicted() {
    let mut cache = TileCache::new(2);
    cache.insert(TileId::new(0, 0, 0), "a");
    cache.insert(TileId::new(1, 1, 1), "b");
    cache.insert(TileId::new(2, 2, 2), "c"); // 淘汰 "a"

    let evicted = cache.take_evicted();
    assert_eq!(evicted.len(), 1);
    assert_eq!(evicted[0].0, TileId::new(0, 0, 0));
    assert_eq!(evicted[0].1, "a");

    // 第二次调用返回空
    let evicted2 = cache.take_evicted();
    assert!(evicted2.is_empty());
}

#[test]
fn cache_remove() {
    let mut cache = TileCache::new(10);
    cache.insert(TileId::new(1, 2, 3), 100);
    let removed = cache.remove(&TileId::new(1, 2, 3));
    assert_eq!(removed, Some(100));
    assert!(!cache.contains(&TileId::new(1, 2, 3)));
    assert_eq!(cache.len(), 0);
}

#[test]
fn cache_remove_nonexistent() {
    let mut cache: TileCache<i32> = TileCache::new(10);
    let removed = cache.remove(&TileId::new(9, 9, 9));
    assert_eq!(removed, None);
}

#[test]
fn cache_clear() {
    let mut cache = TileCache::new(10);
    cache.insert(TileId::new(0, 0, 0), "a");
    cache.insert(TileId::new(1, 1, 1), "b");
    cache.clear();
    assert!(cache.is_empty());
    assert_eq!(cache.len(), 0);
}

// === SchedulerConfig / Stats ===

#[test]
fn scheduler_config_defaults() {
    let config = SchedulerConfig::default();
    assert_eq!(config.max_loads_per_frame, 4);
    assert_eq!(config.max_cache_size, 512);
    assert_eq!(config.max_queue_size, 256);
    assert!(config.prioritize_by_distance);
}

#[test]
fn scheduler_stats_defaults() {
    let stats = SchedulerStats::default();
    assert_eq!(stats.loaded_this_frame, 0);
    assert_eq!(stats.cached_tiles, 0);
    assert_eq!(stats.queued_tiles, 0);
    assert_eq!(stats.cache_hits, 0);
    assert_eq!(stats.cache_misses, 0);
}
