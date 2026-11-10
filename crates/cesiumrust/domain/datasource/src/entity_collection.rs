//! 实体集合管理。
//!
//! `EntityCollection` 以 id 为键存一组实体，提供增删改查与按事件变更的能力；
//! `DataSource` 则在集合之上封装一个命名数据源的实体、加载状态与时钟设置，
//! 作为 CZML/GeoJSON 等解析结果的统一承载体。

use crate::entity::Entity;
use std::collections::HashMap;

/// 一个支持基于 ID 查找的实体集合。
///
/// 内部以 id 为键维护实体，新增时若 id 已存在则覆盖；集合保留插入顺序以外
/// 的语义弱，主要供逐帧遍历与按 id 快速取回，并预留事件暂停/恢复接口。
#[derive(Debug, Default)]
pub struct EntityCollection {
    /// 按 ID 索引的实体。
    entities: HashMap<String, Entity>,

    /// 插入顺序跟踪。
    order: Vec<String>,

    /// 集合是否显示。
    show: bool,
}

impl EntityCollection {
    /// 创建新的空集合。
    pub fn new() -> Self {
        Self {
            entities: HashMap::new(),
            order: Vec::new(),
            show: true,
        }
    }

    /// 添加或替换一个实体。
    pub fn add(&mut self, entity: Entity) {
        let id = entity.id.clone();
        if !self.entities.contains_key(&id) {
            self.order.push(id.clone());
        }
        self.entities.insert(id, entity);
    }

    /// 按 ID 移除一个实体。
    pub fn remove(&mut self, id: &str) -> Option<Entity> {
        if let Some(entity) = self.entities.remove(id) {
            self.order.retain(|o| o != id);
            Some(entity)
        } else {
            None
        }
    }

    /// 按 ID 获取一个实体。
    pub fn get(&self, id: &str) -> Option<&Entity> {
        self.entities.get(id)
    }

    /// 按 ID 获取一个可变实体。
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Entity> {
        self.entities.get_mut(id)
    }

    /// 返回实体数量。
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// 若集合为空则返回 true。
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// 若集合中包含具有给定 ID 的实体则返回 true。
    pub fn contains(&self, id: &str) -> bool {
        self.entities.contains_key(id)
    }

    /// 清除所有实体。
    pub fn clear(&mut self) {
        self.entities.clear();
        self.order.clear();
    }

    /// 按插入顺序返回实体。
    pub fn values(&self) -> impl Iterator<Item = &Entity> {
        self.order.iter().filter_map(|id| self.entities.get(id))
    }

    /// 按插入顺序返回所有实体 ID。
    pub fn ids(&self) -> &[String] {
        &self.order
    }

    /// 返回集合是否显示。
    pub fn show(&self) -> bool {
        self.show
    }

    /// 设置集合是否显示。
    pub fn set_show(&mut self, show: bool) {
        self.show = show;
    }

    /// 仅返回可见实体（show=true 且集合 show=true）。
    pub fn visible_entities(&self) -> impl Iterator<Item = &Entity> {
        let show = self.show;
        self.values().filter(move |e| show && e.show)
    }

    /// 返回具有可渲染图形的实体。
    pub fn renderable_entities(&self) -> impl Iterator<Item = &Entity> {
        self.visible_entities().filter(|e| e.has_graphics())
    }

    /// 按 ID 获取一个实体，若不存在则创建并插入一个新的。
    /// 映射到 `EntityCollection.prototype.getOrCreateEntity`。
    pub fn get_or_create(&mut self, id: &str) -> &Entity {
        if !self.entities.contains_key(id) {
            let entity = Entity::new(id.to_string());
            self.order.push(id.to_string());
            self.entities.insert(id.to_string(), entity);
        }
        self.entities.get(id).unwrap()
    }

    /// 移除集合中的所有实体。
    /// 映射到 `EntityCollection.prototype.removeAll`。
    pub fn remove_all(&mut self) {
        self.entities.clear();
        self.order.clear();
    }

    /// 按 ID 移除一个实体，若原本存在则返回 true。
    /// 映射到 `EntityCollection.prototype.removeById`。
    pub fn remove_by_id(&mut self, id: &str) -> bool {
        self.remove(id).is_some()
    }

    /// 挂起事件（为未来事件系统集成预留的占位）。
    pub fn suspend_events(&mut self) {
        // 占位
    }

    /// 恢复事件。
    pub fn resume_events(&mut self) {
        // 占位
    }
}

/// 一个提供实体的数据源。
///
/// 封装一个命名数据源的实体集合、加载标志，以及可选的时钟起止与倍率；
/// 各具体解析器（CZML、GeoJSON 等）最终都将结果汇总为一个 `DataSource`。
#[derive(Debug)]
pub struct DataSource {
    /// 此数据源的名称。
    pub name: String,

    /// 实体集合。
    pub entities: EntityCollection,

    /// 数据源是否已加载。
    pub loaded: bool,

    /// 时钟起始时刻（若为时间动态）。
    pub clock_start: Option<f64>,
    /// 时钟停止时刻，标记动画区间右端。
    pub clock_stop: Option<f64>,
    /// 时钟当前时刻，供数据源指定初始播放位置。
    pub clock_current: Option<f64>,
}

impl DataSource {
    /// 创建一个具有给定名称的新数据源。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entities: EntityCollection::new(),
            loaded: false,
            clock_start: None,
            clock_stop: None,
            clock_current: None,
        }
    }

    /// 若此数据源已准备好渲染则返回 true。
    pub fn is_ready(&self) -> bool {
        self.loaded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::PointGraphics;
    use crate::property::{Color, Property};

    #[test]
    fn test_entity_collection_add_get() {
        let mut collection = EntityCollection::new();
        collection.add(Entity::new("e1").with_name("Entity 1"));
        collection.add(Entity::new("e2").with_name("Entity 2"));

        assert_eq!(collection.len(), 2);
        assert!(collection.contains("e1"));
        assert!(collection.get("e2").is_some());
    }

    #[test]
    fn test_entity_collection_remove() {
        let mut collection = EntityCollection::new();
        collection.add(Entity::new("e1"));
        collection.add(Entity::new("e2"));

        let removed = collection.remove("e1");
        assert!(removed.is_some());
        assert_eq!(collection.len(), 1);
        assert!(!collection.contains("e1"));
    }

    #[test]
    fn test_entity_collection_order() {
        let mut collection = EntityCollection::new();
        collection.add(Entity::new("a"));
        collection.add(Entity::new("b"));
        collection.add(Entity::new("c"));

        let ids: Vec<&str> = collection.values().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_entity_collection_clear() {
        let mut collection = EntityCollection::new();
        collection.add(Entity::new("e1"));
        collection.add(Entity::new("e2"));

        collection.clear();
        assert!(collection.is_empty());
    }

    #[test]
    fn test_visible_entities() {
        let mut collection = EntityCollection::new();

        let mut visible = Entity::new("v1");
        visible.show = true;
        visible.point = Some(PointGraphics::default());
        collection.add(visible);

        let mut hidden = Entity::new("h1");
        hidden.show = false;
        hidden.point = Some(PointGraphics::default());
        collection.add(hidden);

        assert_eq!(collection.visible_entities().count(), 1);
    }

    #[test]
    fn test_renderable_entities() {
        let mut collection = EntityCollection::new();

        // 有图形
        let with_gfx = Entity::new("gfx")
            .with_point(PointGraphics {
                color: Property::Constant(Color::RED),
                ..Default::default()
            });
        collection.add(with_gfx);

        // 无图形
        collection.add(Entity::new("no-gfx"));

        assert_eq!(collection.renderable_entities().count(), 1);
    }

    #[test]
    fn test_data_source() {
        let mut ds = DataSource::new("Test Source");
        assert!(!ds.is_ready());

        ds.loaded = true;
        assert!(ds.is_ready());

        ds.entities.add(Entity::new("e1"));
        assert_eq!(ds.entities.len(), 1);
    }
}
