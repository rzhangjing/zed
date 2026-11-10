//! 双端优先队列的、基于数组的 min-max 堆实现。
//! 该数据结构支持高效地移除最小和最大元素。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::manual_is_multiple_of)]
use std::cmp::Ordering;

/// 计算完整二叉树中某个节点的层级：
/// `floor(log2(index + 1))`，使用精确的整数运算以避免
/// 在 2 的幂处出现浮点精度问题。
#[inline]
fn level_of(index: usize) -> u32 {
    (index + 1).ilog2()
}

/// 双端优先队列的、基于数组的 min-max 堆实现。
///
/// 若 `a` 的优先级低于 `b`，比较器返回 `Ordering::Less`
/// （即 `comparator(a, b) < 0`）。
pub struct DoubleEndedPriorityQueue<T, F>
where
    F: Fn(&T, &T) -> Ordering,
{
    comparator: F,
    maximum_length: Option<usize>,
    /// 内部数组。超出 `length` 的槽位为 `None`（对应 JS 的 `undefined`）。
    array: Vec<Option<T>>,
    length: usize,
}

impl<T, F> DoubleEndedPriorityQueue<T, F>
where
    F: Fn(&T, &T) -> Ordering,
{
    /// 创建一个新的双端优先队列。
    ///
    /// `maximum_length`：队列的最大长度。若在队列已满时插入元素，
    /// 则移除最小元素。`None` 表示队列大小不受限制。
    pub fn new(comparator: F, maximum_length: Option<usize>) -> Self {
        let array = match maximum_length {
            Some(ml) => (0..ml).map(|_| None).collect(),
            None => Vec::new(),
        };
        Self {
            comparator,
            maximum_length,
            array,
            length: 0,
        }
    }

    /// 获取队列中的元素个数。
    pub fn length(&self) -> usize {
        self.length
    }

    /// 获取队列中的最大元素数，若已设置。
    pub fn maximum_length(&self) -> Option<usize> {
        self.maximum_length
    }

    /// 设置队列中的最大元素数。
    /// 若设置的值小于当前长度，则移除优先级最低的元素。
    /// 若设置为 `None`，则队列大小不受限制。
    pub fn set_maximum_length(&mut self, value: Option<usize>) {
        if let Some(value) = value {
            // 移除元素直到满足最大长度。
            while self.length > value {
                self.remove_minimum();
            }
            // 数组大小固定为最大长度。
            self.array.resize_with(value, || None);
        }
        self.maximum_length = value;
    }

    /// 获取内部数组（超出 `length` 的槽位为 `None`）。
    pub fn internal_array(&self) -> &[Option<T>] {
        &self.array
    }

    /// 获取内部数组的可变引用。
    pub fn internal_array_mut(&mut self) -> &mut [Option<T>] {
        &mut self.array
    }

    /// 队列所使用的比较器。
    pub fn comparator(&self) -> &F {
        &self.comparator
    }

    /// 从队列中移除所有元素。
    pub fn reset(&mut self) {
        self.length = 0;
        if self.maximum_length.is_some() {
            // 解引用所有元素但保持数组大小不变。
            for slot in self.array.iter_mut() {
                *slot = None;
            }
        } else {
            // 通过清空数组来解引用所有元素。
            self.array.clear();
        }
    }

    /// 重新排序队列。
    pub fn resort(&mut self) {
        let length = self.length;
        // 自顶向下修复队列。
        for i in 0..length {
            self.push_up(i);
        }
    }

    /// 向队列中插入一个元素。
    /// 若队列已满，则移除最小元素并返回它。
    /// 若新元素的优先级小于或等于最小元素，则返回该新元素（且不添加）。
    pub fn insert(&mut self, element: T) -> Option<T> {
        let mut removed_element = None;

        if let Some(maximum_length) = self.maximum_length {
            if maximum_length == 0 {
                return None;
            } else if self.length == maximum_length {
                // 直接访问最小元素比调用 getter 更快，
                // 因为它避免了 length == 0 的检查。
                let minimum_element = self.array[0].as_ref().unwrap();
                if (self.comparator)(&element, minimum_element) != Ordering::Greater {
                    // 要插入的元素小于或等于最小元素，因此不插入任何
                    // 内容并提前返回。
                    return Some(element);
                }
                removed_element = self.remove_minimum();
            }
        }

        let index = self.length;
        if index < self.array.len() {
            self.array[index] = Some(element);
        } else {
            self.array.push(Some(element));
        }
        self.length += 1;
        self.push_up(index);

        removed_element
    }

    /// 从队列中移除最小元素并返回它。
    /// 若队列为空，则返回值为 `None`。
    pub fn remove_minimum(&mut self) -> Option<T> {
        let length = self.length;
        if length == 0 {
            return None;
        }

        self.length -= 1;

        // 最小元素始终是根。
        let minimum_element = self.array[0].take();

        if length >= 2 {
            self.array[0] = self.array[length - 1].take();
            self.push_down(0);
        }

        // 解引用被移除的元素。
        self.array[length - 1] = None;

        minimum_element
    }

    /// 从队列中移除最大元素并返回它。
    /// 若队列为空，则返回值为 `None`。
    pub fn remove_maximum(&mut self) -> Option<T> {
        let length = self.length;
        if length == 0 {
            return None;
        }

        self.length -= 1;
        let maximum_element;

        // 若根没有子节点，则最大值就是根。
        // 若根有一个子节点，则最大值就是该子节点。
        if length <= 2 {
            maximum_element = self.array[length - 1].take();
        } else {
            // 否则，最大值是根的两个子节点中较大的那个。
            let maximum_element_index = if self.greater_than(1, 2) { 1 } else { 2 };
            maximum_element = self.array[maximum_element_index].take();

            // 重新平衡堆。
            self.array[maximum_element_index] = self.array[length - 1].take();
            if length >= 4 {
                self.push_down(maximum_element_index);
            }
        }

        // 解引用被移除的元素。
        self.array[length - 1] = None;

        maximum_element
    }

    /// 获取队列中的最小元素，若为空则返回 `None`。
    pub fn get_minimum(&self) -> Option<&T> {
        if self.length == 0 {
            return None;
        }
        // 最小元素始终是根。
        self.array[0].as_ref()
    }

    /// 获取队列中的最大元素，若为空则返回 `None`。
    pub fn get_maximum(&self) -> Option<&T> {
        let length = self.length;
        if length == 0 {
            return None;
        }
        // 若根没有子节点，则最大值就是根。
        // 若根有一个子节点，则最大值就是该子节点。
        if length <= 2 {
            return self.array[length - 1].as_ref();
        }
        // 否则，最大值是根的两个子节点中较大的那个。
        self.array[if self.greater_than(1, 2) { 1 } else { 2 }].as_ref()
    }

    // 辅助函数

    fn less_than(&self, index_a: usize, index_b: usize) -> bool {
        let a = self.array[index_a].as_ref().unwrap();
        let b = self.array[index_b].as_ref().unwrap();
        (self.comparator)(a, b) == Ordering::Less
    }

    /// 判断 index_a 处的元素是否严格大于 index_b 处的元素。
    fn greater_than(&self, index_a: usize, index_b: usize) -> bool {
        let a = self.array[index_a].as_ref().unwrap();
        let b = self.array[index_b].as_ref().unwrap();
        (self.comparator)(a, b) == Ordering::Greater
    }

    /// 将指定索引处的元素向堆顶方向调整，使其落在正确的 min/max 层上。
    fn push_up(&mut self, mut index: usize) {
        if index == 0 {
            return;
        }
        let on_min_level = level_of(index) % 2 == 0;
        let parent_index = (index - 1) / 2;
        let less_than_parent = self.less_than(index, parent_index);

        // 若元素尚未在正确的层级上，则将其移到该层级。
        if less_than_parent != on_min_level {
            self.array.swap(index, parent_index);
            index = parent_index;
        }

        // 只要元素满足以下条件就与其祖父节点交换：
        // 1) 存在祖父节点
        // 2A) 位于 min 层时小于祖父节点
        // 2B) 位于 max 层时大于祖父节点
        while index >= 3 {
            let grandparent_index = (index - 3) / 4;
            if self.less_than(index, grandparent_index) != less_than_parent {
                break;
            }
            self.array.swap(index, grandparent_index);
            index = grandparent_index;
        }
    }

    /// 将指定索引处的元素向堆底方向下沉，维护双端堆的层序不变式。
    fn push_down(&mut self, mut index: usize) {
        let length = self.length;
        let on_min_level = level_of(index) % 2 == 0;

        // 只要存在左子节点就继续循环。
        loop {
            let left_child_index = 2 * index + 1;
            if left_child_index >= length {
                break;
            }

            // 找到最小（或最大）的子节点或孙节点。
            let mut target = left_child_index;
            let right_child_index = left_child_index + 1;
            if right_child_index < length {
                if self.less_than(right_child_index, target) == on_min_level {
                    target = right_child_index;
                }
                let grand_child_start = 2 * left_child_index + 1;
                let grand_child_count = if length > grand_child_start {
                    std::cmp::min(length - grand_child_start, 4)
                } else {
                    0
                };
                for i in 0..grand_child_count {
                    let grand_child_index = grand_child_start + i;
                    if self.less_than(grand_child_index, target) == on_min_level {
                        target = grand_child_index;
                    }
                }
            }

            // 将元素交换到正确的位置。
            if self.less_than(target, index) == on_min_level {
                self.array.swap(target, index);
                if target != left_child_index && target != right_child_index {
                    let parent_of_grandchild_index = (target - 1) / 2;
                    if self.greater_than(target, parent_of_grandchild_index) == on_min_level {
                        self.array.swap(target, parent_of_grandchild_index);
                    }
                }
            }

            index = target;
        }
    }
}

impl<T, F> DoubleEndedPriorityQueue<T, F>
where
    T: Clone,
    F: Clone + Fn(&T, &T) -> Ordering,
{
    /// 克隆该双端优先队列。
    pub fn clone_queue(&self) -> Self {
        Self {
            comparator: self.comparator.clone(),
            maximum_length: self.maximum_length,
            array: self.array.clone(),
            length: self.length,
        }
    }
}
