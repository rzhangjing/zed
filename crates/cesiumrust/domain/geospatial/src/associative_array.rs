//! AssociativeArray —— 一个键值对集合，以哈希存储以便快速查找，
//! 同时也提供一个数组以便快速迭代。

use std::collections::HashMap;

/// 一个键值对集合，以哈希存储以便快速查找，
/// 同时维护一个值数组以便快速迭代。
pub struct AssociativeArray<T> {
    /// 值的顺序数组，用于保持插入顺序并支持快速迭代。
    array: Vec<T>,
    /// 键到值的哈希映射，用于按键快速查找。
    hash: HashMap<String, T>,
}

impl<T> Default for AssociativeArray<T>
where
    T: Clone + PartialEq,
{
    /// 默认构造一个空的关联数组。
    fn default() -> Self {
        Self::new()
    }
}

impl<T> AssociativeArray<T>
where
    T: Clone + PartialEq,
{
    /// 创建新的空关联数组。
    pub fn new() -> Self {
        Self {
            array: Vec::new(),
            hash: HashMap::new(),
        }
    }

    /// 获取集合中项的数量。
    pub fn length(&self) -> usize {
        self.array.len()
    }

    /// 获取集合中所有值的数组。
    pub fn values(&self) -> &[T] {
        &self.array
    }

    /// 判断所提供的键是否在数组中。
    pub fn contains(&self, key: &str) -> bool {
        self.hash.contains_key(key)
    }

    /// 将所提供的键与所提供的值关联。若键已
    /// 存在，则用新值覆盖。
    pub fn set(&mut self, key: &str, value: T) {
        let needs_update = match self.hash.get(key) {
            Some(old) => *old != value,
            None => true,
        };
        if needs_update {
            self.remove(key);
            self.array.push(value.clone());
            self.hash.insert(key.to_string(), value);
        }
    }

    /// 检索与所提供键关联的值，若
    /// 集合中不存在该键则返回 `None`。
    pub fn get(&self, key: &str) -> Option<&T> {
        self.hash.get(key)
    }

    /// 从集合中移除一个键值对。
    /// 若已移除则返回 `true`，若键不存在则返回 `false`。
    pub fn remove(&mut self, key: &str) -> bool {
        if let Some(value) = self.hash.remove(key) {
            if let Some(idx) = self.array.iter().position(|v| *v == value) {
                self.array.remove(idx);
            }
            true
        } else {
            false
        }
    }

    /// 清除集合。
    pub fn remove_all(&mut self) {
        if !self.array.is_empty() {
            self.hash.clear();
            self.array.clear();
        }
    }
}
