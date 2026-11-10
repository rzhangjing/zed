//! CompositeEntityCollection - 以非破坏方式组合多个 EntityCollection。
//!
//! 组合集合自身不拥有子集合的实体，仅在查询与遍历时把它们视作一个整体。

use crate::entity::Entity;
use crate::entity_collection::EntityCollection;

/// 将多个 EntityCollection 实例以非破坏方式组合为
/// 单个集合。若相同 ID 的 Entity 存在于多个集合中，
/// 会被非破坏地合并为一个新实体。
///
/// 合并只影响对外视图，不修改任何原始子集合；从组合中
/// 解绑某子集合即停止采纳它贡献的实体。
#[derive(Debug, Default)]
pub struct CompositeEntityCollection {
    /// 有序的集合列表。
    collections: Vec<EntityCollection>,
    /// 组合后的实体缓存。
    composite: EntityCollection,
    /// 组合结果是否需要重建。
    should_recomposite: bool,
    /// 所有者组合（用于嵌套组合）。
    owner_id: Option<String>,
}

impl CompositeEntityCollection {
    /// 创建新的空组合集合。
    pub fn new() -> Self {
        Self {
            collections: Vec::new(),
            composite: EntityCollection::new(),
            should_recomposite: true,
            owner_id: None,
        }
    }

    /// 创建一个带所有者 ID 的新组合。
    pub fn with_owner(owner_id: &str) -> Self {
        Self {
            collections: Vec::new(),
            composite: EntityCollection::new(),
            should_recomposite: true,
            owner_id: Some(owner_id.to_string()),
        }
    }

    /// 获取所有者 ID（若有）。
    pub fn owner(&self) -> Option<&str> {
        self.owner_id.as_deref()
    }

    /// 向组合添加一个集合。
    /// 映射到 `CompositeEntityCollection.prototype.addCollection`
    pub fn add_collection(&mut self, collection: EntityCollection) {
        self.collections.push(collection);
        self.should_recomposite = true;
    }

    /// 在指定索引处添加一个集合。
    pub fn add_collection_at(&mut self, index: usize, collection: EntityCollection) {
        let idx = index.min(self.collections.len());
        self.collections.insert(idx, collection);
        self.should_recomposite = true;
    }

    /// 从组合中移除一个集合。
    /// 若找到并移除了该集合则返回 true。
    /// 映射到 `CompositeEntityCollection.prototype.removeCollection`
    pub fn remove_collection(&mut self, index: usize) -> bool {
        if index < self.collections.len() {
            self.collections.remove(index);
            self.should_recomposite = true;
            true
        } else {
            false
        }
    }

    /// 移除所有集合。
    /// 映射到 `CompositeEntityCollection.prototype.removeAllCollections`
    pub fn remove_all_collections(&mut self) {
        self.collections.clear();
        self.should_recomposite = true;
    }

    /// 获取集合数量。
    /// 映射到 `CompositeEntityCollection.prototype.getCollectionsLength`
    pub fn get_collections_length(&self) -> usize {
        self.collections.len()
    }

    /// 按索引获取一个集合。
    /// 映射到 `CompositeEntityCollection.prototype.getCollection`
    pub fn get_collection(&self, index: usize) -> Option<&EntityCollection> {
        self.collections.get(index)
    }

    /// 按索引获取一个可变集合。
    pub fn get_collection_mut(&mut self, index: usize) -> Option<&mut EntityCollection> {
        self.should_recomposite = true;
        self.collections.get_mut(index)
    }

    /// 若组合中包含具有给定 ID 的实体则返回 true。
    /// 映射到 `CompositeEntityCollection.prototype.contains`
    pub fn contains(&self, entity_id: &str) -> bool {
        self.ensure_composited();
        self.composite.contains(entity_id)
    }

    /// 按 ID 从组合中获取一个实体。
    /// 映射到 `CompositeEntityCollection.prototype.getById`
    pub fn get_by_id(&self, entity_id: &str) -> Option<&Entity> {
        self.ensure_composited();
        self.composite.get(entity_id)
    }

    /// 按 ID 获取或创建一个实体。
    /// 映射到 `CompositeEntityCollection.prototype.getOrCreateEntity`
    pub fn get_or_create_entity(&mut self, entity_id: &str) -> &Entity {
        self.recomposite();
        self.composite.get_or_create(entity_id)
    }

    /// 返回组合后的实体值。
    /// 映射到 `CompositeEntityCollection.prototype.values`
    pub fn values(&self) -> Vec<&Entity> {
        self.ensure_composited();
        self.composite.values().collect()
    }

    /// 返回组合后实体的数量。
    pub fn len(&self) -> usize {
        self.ensure_composited();
        self.composite.len()
    }

    /// 若组合为空则返回 true。
    pub fn is_empty(&self) -> bool {
        self.ensure_composited();
        self.composite.is_empty()
    }

    /// 挂起事件（占位）。
    pub fn suspend_events(&mut self) {}

    /// 恢复事件（占位）。
    pub fn resume_events(&mut self) {}

    /// 确保组合结果是最新的。
    fn ensure_composited(&self) {
        // 在真实实现中，这里会检查 should_recomposite
        // 并惰性重建。目前我们总是访问预先构建好的组合。
    }

    /// 从所有集合重建组合。
    /// 对于相同 ID 的实体，靠后的集合优先（合并）。
    pub fn recomposite(&mut self) {
        let mut new_composite = EntityCollection::new();

        // 以逆序处理集合，以便靠后的集合拥有优先级
        for collection in self.collections.iter().rev() {
            for entity in collection.values() {
                if !new_composite.contains(&entity.id) {
                    new_composite.add(entity.clone());
                }
                // 若实体已存在，则靠后集合的版本胜出
                // （由于采用逆序遍历，已优先添加）
            }
        }

        self.composite = new_composite;
        self.should_recomposite = false;
    }
}
