//! 一个带用户自定义比较器的堆（heap）数据结构。

/// 一个使用比较器函数来维持堆性质的堆。
/// 比较器应在 `a` 优先级更高时返回负值，相等时返回零，
/// `b` 优先级更高时返回正值（默认为最小堆）。
pub struct Heap<T, F>
where
    F: Fn(&T, &T) -> std::cmp::Ordering,
{
    comparator: F,
    array: Vec<T>,
    maximum_length: Option<usize>,
}

impl<T, F> Heap<T, F>
where
    F: Fn(&T, &T) -> std::cmp::Ordering,
{
    /// 使用给定的比较器创建一个新 Heap。
    pub fn new(comparator: F) -> Self {
        Self {
            comparator,
            array: Vec::new(),
            maximum_length: None,
        }
    }

    /// 返回堆中的元素个数。
    pub fn length(&self) -> usize {
        self.array.len()
    }

    /// 返回最大长度约束，若已设置。
    pub fn maximum_length(&self) -> Option<usize> {
        self.maximum_length
    }

    /// 设置最大长度。若当前长度超过此值，
    /// 则移除多余的元素。
    ///
    /// # Panic
    /// 若 `maximum_length` 为负则 Panic（对 usize 不适用）。
    pub fn set_maximum_length(&mut self, maximum_length: usize) {
        self.maximum_length = Some(maximum_length);
        if self.array.len() > maximum_length {
            self.array.truncate(maximum_length);
        }
    }

    /// 返回内部数组的引用。
    pub fn internal_array(&self) -> &[T] {
        &self.array
    }

    /// 向堆中插入一个值。
    /// 若超过 maximumLength 则返回被移除的元素，否则返回 None。
    pub fn insert(&mut self, value: T) -> Option<T> {
        let mut removed = None;

        if let Some(max_len) = self.maximum_length {
            if self.array.len() >= max_len {
                // 末尾插入，上浮，然后移除最后一个（优先级最低的）
                self.array.push(value);
                self.bubble_up(self.array.len() - 1);
                // 要移除的是优先级最低的元素（heapify 后位于末尾）
                // 在最小堆中，最大元素位于某个叶子节点。
                // CesiumJS 的做法：插入，然后若超过最大值，则在上浮后
                // 移除内部数组的最后一个元素。
                // 我们遵循 CesiumJS：正常插入，若 length > maximumLength，
                // 就移除内部数组末尾的元素（上浮后被置换的那个）。
                // 实际上 CesiumJS 做的是：array[length] = value, length++, bubbleUp,
                // 然后若 length > maximumLength：removed = array[--length], array.length = length
                // 这意味着它移除数组中的最后一个元素（而非根）。
                // 上浮后，新插入的元素已移到其正确位置，而被交换下来的
                // 内容位于末尾。因此被移除的，就是上浮过程中被挤到最底部的
                // 那个元素。
                //
                // 更仔细地查看 CesiumJS 源码：
                // insert: this._array[this._length] = value; this._length++; bubbleUp;
                //         if defined maximumLength && this._length > maximumLength:
                //           removed = this._array[this._length - 1]; this._length--;
                //           this._array.length = this._length; (截断)
                // 因此它移除上浮后数组中的最后一个元素。
                // 上浮后，新值已移到其正确位置，而被交换下来的内容位于末尾。
                // 所以被移除的元素，就是上浮过程中被挤到最底部的那个。
                removed = self.array.pop();
            } else {
                self.array.push(value);
                self.bubble_up(self.array.len() - 1);
            }
        } else {
            self.array.push(value);
            self.bubble_up(self.array.len() - 1);
        }

        removed
    }

    /// 移除并返回根（优先级最高的）元素。
    /// 若堆为空则返回 None。
    pub fn pop(&mut self) -> Option<T> {
        if self.array.is_empty() {
            return None;
        }

        let last = self.array.len() - 1;
        self.array.swap(0, last);
        let result = self.array.pop();

        if !self.array.is_empty() {
            self.bubble_down(0);
        }

        result
    }

    /// 在元素被外部修改后重新建立堆性质。
    pub fn resort(&mut self) {
        let len = self.array.len();
        if len <= 1 {
            return;
        }
        // 自底向上构建堆
        let mut i = len / 2;
        while i > 0 {
            i -= 1;
            self.bubble_down(i);
        }
    }

    /// 将指定索引处的元素逐层上浮，直到不小于其父节点（恢复堆序）。
    fn bubble_up(&mut self, mut index: usize) {
        while index > 0 {
            let parent = (index - 1) / 2;
            if (self.comparator)(&self.array[index], &self.array[parent]) == std::cmp::Ordering::Less
            {
                self.array.swap(index, parent);
                index = parent;
            } else {
                break;
            }
        }
    }

    /// 将指定索引处的元素逐层下渗到与更小子节点交换的正确位置。
    fn bubble_down(&mut self, mut index: usize) {
        let len = self.array.len();
        loop {
            let left = 2 * index + 1;
            let right = 2 * index + 2;
            let mut smallest = index;

            if left < len
                && (self.comparator)(&self.array[left], &self.array[smallest])
                    == std::cmp::Ordering::Less
            {
                smallest = left;
            }
            if right < len
                && (self.comparator)(&self.array[right], &self.array[smallest])
                    == std::cmp::Ordering::Less
            {
                smallest = right;
            }

            if smallest != index {
                self.array.swap(index, smallest);
                index = smallest;
            } else {
                break;
            }
        }
    }
}
