//! 瓦片缓存与加载队列管理。
//!
//! 提供面向已加载瓦片的 LRU 缓存、基于优先级与相机距离的加载队列，
//! 以及超容量时按 LRU 逐出并交由调用方清理的瓦片替换策略。

use std::collections::{HashMap, VecDeque};

/// 瓦片标识符（x, y, level）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileId {
    /// 瓦片 X 坐标。
    pub x: u32,
    /// 瓦片 Y 坐标。
    pub y: u32,
    /// 瓦片层级。
    pub level: u32,
}

impl TileId {
    /// 创建一个新的瓦片 ID。
    pub fn new(x: u32, y: u32, level: u32) -> Self {
        // 三元组 (x, y, level) 唯一标识一个瓦片。
        Self { x, y, level }
    }
}

/// 瓦片加载优先级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum TilePriority {
    /// 低优先级（预加载）。
    Low = 0,
    /// 普通优先级。
    #[default]
    Normal = 1,
    /// 高优先级（可见）。
    High = 2,
    /// 关键优先级（视图中心）。
    Critical = 3,
}

/// 加载队列中的一个瓦片。
#[derive(Debug, Clone)]
pub struct QueuedTile {
    /// 瓦片标识符。
    pub id: TileId,
    /// 加载优先级。
    pub priority: TilePriority,
    /// 入队时的帧号。
    pub frame_number: u64,
    /// 到 camera 的距离（用于优先级排序）。
    pub distance: f64,
}

/// 瓦片的加载队列。
///
/// 基于优先级与距离管理瓦片的加载顺序。
#[derive(Debug, Default)]
pub struct TileLoadQueue {
    /// 队列中的瓦片。
    queue: VecDeque<QueuedTile>,
    /// 最大队列长度。
    max_size: usize,
}

impl TileLoadQueue {
    /// 创建一个新的加载队列。
    pub fn new(max_size: usize) -> Self {
        // 初始化空队列并设定最大长度。
        Self {
            queue: VecDeque::new(),
            max_size,
        }
    }

    /// 将一个瓦片加入队列。
    pub fn enqueue(&mut self, tile: QueuedTile) {
        // 队列已满时先淘汰“优先级最低且距离最远”者，为新瓦片腾位。
        if self.queue.len() >= self.max_size {
            // 移除优先级最低的瓦片
            // 以 (priority, -distance) 取最小：先逐优先级最低且距离最远者。
            if let Some(min_idx) = self
                .queue
                .iter()
                .enumerate()
                .min_by_key(|(_, t)| (t.priority, -(t.distance as i64)))
                .map(|(i, _)| i)
            {
                self.queue.remove(min_idx);
            }
        }
        self.queue.push_back(tile);
    }

    /// 获取下一个待加载的瓦片（优先级最高、距离最近）。
    pub fn dequeue(&mut self) -> Option<QueuedTile> {
        // 队列空则无待加载瓦片。
        if self.queue.is_empty() {
            return None;
        }

        // 以 (priority, -distance) 取最大：优先返回优先级高且距离近者。
        let best_idx = self
            .queue
            .iter()
            .enumerate()
            .max_by_key(|(_, t)| (t.priority, -(t.distance as i64)))
            .map(|(i, _)| i)?;

        self.queue.remove(best_idx)
    }

    /// 返回队列中瓦片的数量。
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// 若队列为空则返回 true。
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// 清空队列。
    pub fn clear(&mut self) {
        self.queue.clear();
    }
}

/// 面向已加载瓦片的 LRU 缓存。
///
/// 以访问顺序队列实现最近最少使用逐出，被逐出瓦片暂存以便调用方清理资源。
#[derive(Debug)]
pub struct TileCache<T> {
    /// 已缓存的瓦片。
    tiles: HashMap<TileId, T>,
    /// 访问顺序（最近访问在后）。
    access_order: VecDeque<TileId>,
    /// 最大缓存大小。
    max_size: usize,
    /// 被逐出的瓦片（供清理）。
    evicted: Vec<(TileId, T)>,
}

impl<T> TileCache<T> {
    /// 创建一个新的瓦片缓存。
    pub fn new(max_size: usize) -> Self {
        // 初始化空缓存、空访问顺序与空逐出列表。
        Self {
            tiles: HashMap::new(),
            access_order: VecDeque::new(),
            max_size,
            evicted: Vec::new(),
        }
    }

    /// 从缓存获取一个瓦片。
    pub fn get(&mut self, id: &TileId) -> Option<&T> {
        // 命中则把该瓦片移到访问顺序尾部，推迟其被逐出。
        if self.tiles.contains_key(id) {
            // 更新访问顺序
            self.access_order.retain(|x| x != id);
            self.access_order.push_back(*id);
            // 命中时返回引用；未命中则返回 None（缓存不代为加载）。
            self.tiles.get(id)
        } else {
            None
        }
    }

    /// 向缓存插入一个瓦片。
    pub fn insert(&mut self, id: TileId, tile: T) {
        // 达到容量上限则循环弹出访问顺序首部（最久未用）直至有空位。
        // 必要时逐出
        while self.tiles.len() >= self.max_size {
            if let Some(oldest) = self.access_order.pop_front() {
                if let Some(evicted_tile) = self.tiles.remove(&oldest) {
                    self.evicted.push((oldest, evicted_tile));
                }
            } else {
                break;
            }
        }

        self.tiles.insert(id, tile);
        // 将新瓦片置于访问顺序尾部（最近使用）。
        self.access_order.push_back(id);
    }

    /// 从缓存移除一个瓦片。
    pub fn remove(&mut self, id: &TileId) -> Option<T> {
        // 同步从访问顺序中剔除该瓦片。
        self.access_order.retain(|x| x != id);
        self.tiles.remove(id)
    }

    /// 若缓存包含该瓦片则返回 true。
    pub fn contains(&self, id: &TileId) -> bool {
        self.tiles.contains_key(id)
    }

    /// 返回已缓存瓦片的数量。
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// 若缓存为空则返回 true。
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// 取走被逐出的瓦片以供清理。
    pub fn take_evicted(&mut self) -> Vec<(TileId, T)> {
        // 移交逐出列表所有权，调用方据此释放 GPU/内存资源。
        std::mem::take(&mut self.evicted)
    }

    /// 清空缓存。
    pub fn clear(&mut self) {
        // 同时清空瓦片表与访问顺序队列。
        self.tiles.clear();
        self.access_order.clear();
    }
}

/// 瓦片调度器配置。
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// 每帧最多加载的瓦片数。
    pub max_loads_per_frame: usize,
    /// 缓存中最多瓦片数。
    pub max_cache_size: usize,
    /// 加载队列中最多瓦片数。
    pub max_queue_size: usize,
    /// 是否按距离排优先级。
    pub prioritize_by_distance: bool,
}

impl Default for SchedulerConfig {
    /// 返回缺省调度配置：每帧加载 4、缓存 512、队列 256、按距离优先。
    fn default() -> Self {
        Self {
            max_loads_per_frame: 4,
            max_cache_size: 512,
            max_queue_size: 256,
            prioritize_by_distance: true,
        }
    }
}

/// 瓦片调度器统计。
#[derive(Debug, Clone, Copy, Default)]
pub struct SchedulerStats {
    /// 本帧加载的瓦片数。
    pub loaded_this_frame: u32,
    /// 缓存中的瓦片数。
    pub cached_tiles: u32,
    /// 队列中的瓦片数。
    pub queued_tiles: u32,
    /// 缓存命中。
    pub cache_hits: u64,
    /// 缓存未命中。
    pub cache_misses: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_id() {
        let id = TileId::new(1, 2, 3);
        assert_eq!(id.x, 1);
        assert_eq!(id.y, 2);
        assert_eq!(id.level, 3);
    }

    #[test]
    fn test_tile_priority_ordering() {
        assert!(TilePriority::Critical > TilePriority::High);
        assert!(TilePriority::High > TilePriority::Normal);
        assert!(TilePriority::Normal > TilePriority::Low);
    }

    #[test]
    fn test_load_queue_basic() {
        let mut queue = TileLoadQueue::new(10);
        assert!(queue.is_empty());

        queue.enqueue(QueuedTile {
            id: TileId::new(0, 0, 0),
            priority: TilePriority::Normal,
            frame_number: 1,
            distance: 1000.0,
        });

        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());
    }

    #[test]
    fn test_load_queue_priority() {
        let mut queue = TileLoadQueue::new(10);

        queue.enqueue(QueuedTile {
            id: TileId::new(0, 0, 0),
            priority: TilePriority::Low,
            frame_number: 1,
            distance: 100.0,
        });
        queue.enqueue(QueuedTile {
            id: TileId::new(1, 1, 1),
            priority: TilePriority::Critical,
            frame_number: 1,
            distance: 200.0,
        });
        queue.enqueue(QueuedTile {
            id: TileId::new(2, 2, 2),
            priority: TilePriority::Normal,
            frame_number: 1,
            distance: 50.0,
        });

        // 应先出队优先级最高的
        let tile = queue.dequeue().unwrap();
        assert_eq!(tile.priority, TilePriority::Critical);
    }

    #[test]
    fn test_load_queue_distance_tiebreak() {
        let mut queue = TileLoadQueue::new(10);

        queue.enqueue(QueuedTile {
            id: TileId::new(0, 0, 0),
            priority: TilePriority::Normal,
            frame_number: 1,
            distance: 1000.0,
        });
        queue.enqueue(QueuedTile {
            id: TileId::new(1, 1, 1),
            priority: TilePriority::Normal,
            frame_number: 1,
            distance: 100.0, // 更近
        });

        // 应先出队更近的瓦片（同优先级）
        let tile = queue.dequeue().unwrap();
        assert_eq!(tile.id, TileId::new(1, 1, 1));
    }

    #[test]
    fn test_load_queue_max_size() {
        let mut queue = TileLoadQueue::new(2);

        queue.enqueue(QueuedTile {
            id: TileId::new(0, 0, 0),
            priority: TilePriority::Low,
            frame_number: 1,
            distance: 100.0,
        });
        queue.enqueue(QueuedTile {
            id: TileId::new(1, 1, 1),
            priority: TilePriority::Normal,
            frame_number: 1,
            distance: 100.0,
        });
        queue.enqueue(QueuedTile {
            id: TileId::new(2, 2, 2),
            priority: TilePriority::High,
            frame_number: 1,
            distance: 100.0,
        });

        // 应逐出最低优先级
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn test_cache_basic() {
        let mut cache = TileCache::new(10);
        assert!(cache.is_empty());

        cache.insert(TileId::new(0, 0, 0), "tile0");
        assert_eq!(cache.len(), 1);
        assert!(cache.contains(&TileId::new(0, 0, 0)));
    }

    #[test]
    fn test_cache_get() {
        let mut cache = TileCache::new(10);
        cache.insert(TileId::new(0, 0, 0), "tile0");

        assert_eq!(cache.get(&TileId::new(0, 0, 0)), Some(&"tile0"));
        assert_eq!(cache.get(&TileId::new(1, 1, 1)), None);
    }

    #[test]
    fn test_cache_lru_eviction() {
        let mut cache = TileCache::new(2);

        cache.insert(TileId::new(0, 0, 0), "tile0");
        cache.insert(TileId::new(1, 1, 1), "tile1");
        cache.insert(TileId::new(2, 2, 2), "tile2"); // 应逐出 tile0

        assert_eq!(cache.len(), 2);
        assert!(!cache.contains(&TileId::new(0, 0, 0)));
        assert!(cache.contains(&TileId::new(1, 1, 1)));
        assert!(cache.contains(&TileId::new(2, 2, 2)));

        let evicted = cache.take_evicted();
        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0].0, TileId::new(0, 0, 0));
    }

    #[test]
    fn test_cache_access_updates_lru() {
        let mut cache = TileCache::new(2);

        cache.insert(TileId::new(0, 0, 0), "tile0");
        cache.insert(TileId::new(1, 1, 1), "tile1");

        // 访问 tile0，使其成为最近使用
        cache.get(&TileId::new(0, 0, 0));

        // 插入 tile2 - 应逐出 tile1（最久未使用）
        cache.insert(TileId::new(2, 2, 2), "tile2");

        assert!(cache.contains(&TileId::new(0, 0, 0)));
        assert!(!cache.contains(&TileId::new(1, 1, 1)));
        assert!(cache.contains(&TileId::new(2, 2, 2)));
    }

    #[test]
    fn test_cache_remove() {
        let mut cache = TileCache::new(10);
        cache.insert(TileId::new(0, 0, 0), "tile0");

        let removed = cache.remove(&TileId::new(0, 0, 0));
        assert_eq!(removed, Some("tile0"));
        assert!(!cache.contains(&TileId::new(0, 0, 0)));
    }

    #[test]
    fn test_scheduler_config_default() {
        let config = SchedulerConfig::default();
        assert_eq!(config.max_loads_per_frame, 4);
        assert_eq!(config.max_cache_size, 512);
        assert_eq!(config.max_queue_size, 256);
        assert!(config.prioritize_by_distance);
    }

    #[test]
    fn test_scheduler_stats_default() {
        let stats = SchedulerStats::default();
        assert_eq!(stats.loaded_this_frame, 0);
        assert_eq!(stats.cached_tiles, 0);
        assert_eq!(stats.cache_hits, 0);
    }
}
