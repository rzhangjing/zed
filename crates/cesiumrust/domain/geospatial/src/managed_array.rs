//! 一个将长度与容量分开跟踪的托管数组。

/// 一个类数组的数据结构，自行管理容量，
/// 将逻辑长度与预留容量分开跟踪。
#[derive(Debug, Clone)]
pub struct ManagedArray<T: Default + Clone> {
    /// 底层存储；容量可能大于逻辑 `length`。
    values: Vec<T>,
    /// 当前逻辑长度（有效元素个数）。
    length: usize,
}

impl<T: Default + Clone> ManagedArray<T> {
    /// 创建一个具有给定初始长度的新 ManagedArray。
    /// 内部存储被初始化为 `length` 个元素。
    pub fn new(length: usize) -> Self {
        Self {
            values: vec![T::default(); length],
            length,
        }
    }

    /// 返回数组的逻辑长度。
    pub fn length(&self) -> usize {
        self.length
    }

    /// 设置逻辑长度。若增长，新元素被默认初始化。
    /// 若缩小，则保留容量。
    pub fn set_length(&mut self, length: usize) {
        self.resize(length);
    }

    /// 返回内部值切片的引用（直至预留容量）。
    pub fn values(&self) -> &[T] {
        &self.values
    }

    /// 返回预留容量（内部存储长度）。
    pub fn capacity(&self) -> usize {
        self.values.len()
    }

    /// 获取给定索引处的元素。
    ///
    /// # Panic
    /// 若 `index >= length` 则 Panic。
    pub fn get(&self, index: usize) -> &T {
        assert!(index < self.length, "index out of bounds");
        &self.values[index]
    }

    /// 设置给定索引处的元素，必要时调整大小。
    pub fn set(&mut self, index: usize, value: T) {
        if index >= self.length {
            self.resize(index + 1);
        }
        self.values[index] = value;
    }

    /// 返回最后一个元素，若为空则返回 None。
    pub fn peek(&self) -> Option<&T> {
        if self.length == 0 {
            None
        } else {
            Some(&self.values[self.length - 1])
        }
    }

    /// 将一个值压入数组末尾。
    pub fn push(&mut self, value: T) {
        if self.length < self.values.len() {
            self.values[self.length] = value;
        } else {
            self.values.push(value);
        }
        self.length += 1;
    }

    /// 从数组弹出最后一个元素。
    /// 若数组为空则返回 None。
    pub fn pop(&mut self) -> Option<T> {
        if self.length == 0 {
            return None;
        }
        self.length -= 1;
        let value = self.values[self.length].clone();
        self.values[self.length] = T::default();
        Some(value)
    }

    /// 至少预留 `capacity` 个元素的内部存储。
    /// 不改变逻辑长度。
    pub fn reserve(&mut self, capacity: usize) {
        if capacity > self.values.len() {
            self.values.resize(capacity, T::default());
        }
    }

    /// 调整逻辑长度。若增长，新元素被默认初始化。
    /// 若缩小，保留容量但清除尾部元素。
    pub fn resize(&mut self, length: usize) {
        if length > self.values.len() {
            self.values.resize(length, T::default());
        }
        // 缩小时清除尾部引用
        if length < self.length {
            for i in length..self.length {
                if i < self.values.len() {
                    self.values[i] = T::default();
                }
            }
        }
        self.length = length;
    }

    /// 将内部存储裁剪到给定容量（若未指定则用当前长度）。
    pub fn trim(&mut self, capacity: Option<usize>) {
        let target = capacity.unwrap_or(self.length).max(self.length);
        self.values.resize(target, T::default());
    }
}
