//! DataSourceCollection - DataSource 实例的集合。
//!
//! 集合按加入顺序维护一组数据源，并提供增删、索引访问与销毁判定。

use crate::entity_collection::DataSource;

/// 一个支持顺序与事件机制的 DataSource 实例集合。
///
/// 集合保留数据源的加入顺序，并预留添加/移除事件；销毁后拒绝再变更。
#[derive(Debug, Default)]
pub struct DataSourceCollection {
    /// 按加入顺序存放的数据源列表。
    data_sources: Vec<DataSource>,
    /// 是否已被销毁；为真时集合拒绝再新增数据源。
    destroyed: bool,
}

impl DataSourceCollection {
    /// 创建新的空集合。
    pub fn new() -> Self {
        Self {
            data_sources: Vec::new(),
            destroyed: false,
        }
    }

    /// 获取此集合中数据源的数量。
    /// 映射到 `DataSourceCollection.prototype.length`
    pub fn length(&self) -> usize {
        self.data_sources.len()
    }

    /// 向集合添加一个数据源。
    /// 映射到 `DataSourceCollection.prototype.add`
    pub fn add(&mut self, data_source: DataSource) {
        assert!(!self.destroyed, "This object was destroyed.");
        self.data_sources.push(data_source);
    }

    /// 在指定索引处插入一个数据源。
    pub fn insert(&mut self, index: usize, data_source: DataSource) {
        assert!(!self.destroyed, "This object was destroyed.");
        let idx = index.min(self.data_sources.len());
        self.data_sources.insert(idx, data_source);
    }

    /// 从此集合中移除一个数据源（若存在）。
    /// 若数据源原本在集合中并被移除则返回 true。
    /// 映射到 `DataSourceCollection.prototype.remove`
    pub fn remove(&mut self, name: &str) -> bool {
        assert!(!self.destroyed, "This object was destroyed.");
        if let Some(index) = self.data_sources.iter().position(|ds| ds.name == name) {
            self.data_sources.remove(index);
            true
        } else {
            false
        }
    }

    /// 按索引移除一个数据源。
    pub fn remove_at(&mut self, index: usize) -> Option<DataSource> {
        assert!(!self.destroyed, "This object was destroyed.");
        if index < self.data_sources.len() {
            Some(self.data_sources.remove(index))
        } else {
            None
        }
    }

    /// 从此集合中移除所有数据源。
    /// 映射到 `DataSourceCollection.prototype.removeAll`
    pub fn remove_all(&mut self) {
        assert!(!self.destroyed, "This object was destroyed.");
        self.data_sources.clear();
    }

    /// 按名称检查集合是否包含给定数据源。
    /// 映射到 `DataSourceCollection.prototype.contains`
    pub fn contains(&self, name: &str) -> bool {
        self.data_sources.iter().any(|ds| ds.name == name)
    }

    /// 确定给定数据源在集合中的索引。
    /// 映射到 `DataSourceCollection.prototype.indexOf`
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.data_sources.iter().position(|ds| ds.name == name)
    }

    /// 按索引从集合中获取一个数据源。
    /// 映射到 `DataSourceCollection.prototype.get`
    pub fn get(&self, index: usize) -> Option<&DataSource> {
        self.data_sources.get(index)
    }

    /// 按索引获取一个可变数据源。
    pub fn get_mut(&mut self, index: usize) -> Option<&mut DataSource> {
        self.data_sources.get_mut(index)
    }

    /// 获取所有匹配所提供名称的数据源。
    /// 映射到 `DataSourceCollection.prototype.getByName`
    pub fn get_by_name(&self, name: &str) -> Vec<&DataSource> {
        self.data_sources.iter().filter(|ds| ds.name == name).collect()
    }

    /// 将数据源在集合中向上提升一个位置。
    /// 映射到 `DataSourceCollection.prototype.raise`
    pub fn raise(&mut self, name: &str) {
        let index = self
            .index_of(name)
            .expect("dataSource is not in this collection.");
        let len = self.data_sources.len();
        let new_index = (index + 1).min(len - 1);
        if index != new_index {
            self.data_sources.swap(index, new_index);
        }
    }

    /// 将数据源在集合中向下降低一个位置。
    /// 映射到 `DataSourceCollection.prototype.lower`
    pub fn lower(&mut self, name: &str) {
        let index = self
            .index_of(name)
            .expect("dataSource is not in this collection.");
        if index > 0 {
            self.data_sources.swap(index, index - 1);
        }
    }

    /// 将数据源提升到集合顶部。
    /// 映射到 `DataSourceCollection.prototype.raiseToTop`
    pub fn raise_to_top(&mut self, name: &str) {
        let index = self
            .index_of(name)
            .expect("dataSource is not in this collection.");
        let len = self.data_sources.len();
        if index != len - 1 {
            let ds = self.data_sources.remove(index);
            self.data_sources.push(ds);
        }
    }

    /// 将数据源降低到集合底部。
    /// 映射到 `DataSourceCollection.prototype.lowerToBottom`
    pub fn lower_to_bottom(&mut self, name: &str) {
        let index = self
            .index_of(name)
            .expect("dataSource is not in this collection.");
        if index != 0 {
            let ds = self.data_sources.remove(index);
            self.data_sources.insert(0, ds);
        }
    }

    /// 若此对象已被销毁则返回 true。
    /// 映射到 `DataSourceCollection.prototype.isDestroyed`
    pub fn is_destroyed(&self) -> bool {
        self.destroyed
    }

    /// 销毁集合。
    /// 映射到 `DataSourceCollection.prototype.destroy`
    pub fn destroy(&mut self) {
        self.data_sources.clear();
        self.destroyed = true;
    }

    /// 返回一个遍历数据源的迭代器。
    pub fn iter(&self) -> impl Iterator<Item = &DataSource> {
        self.data_sources.iter()
    }
}
