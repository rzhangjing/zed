//! 一个 FIFO 队列数据结构。

use std::collections::VecDeque;

/// 一个支持 peek、contains、clear 和 sort 的 FIFO 队列。
#[derive(Debug, Clone)]
pub struct Queue<T> {
    /// 底层双端队列：尾部入队、头部出队。
    deque: VecDeque<T>,
}

impl<T> Default for Queue<T> {
    /// 默认构造一个空队列。
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Queue<T> {
    /// 创建新的空队列。
    pub fn new() -> Self {
        Self {
            deque: VecDeque::new(),
        }
    }

    /// 返回队列中元素的个数。
    pub fn length(&self) -> usize {
        self.deque.len()
    }

    /// 向队列尾部添加一个元素。
    pub fn enqueue(&mut self, item: T) {
        self.deque.push_back(item);
    }

    /// 移除并返回队列前部的元素。
    /// 若队列为空则返回 None。
    pub fn dequeue(&mut self) -> Option<T> {
        self.deque.pop_front()
    }

    /// 返回队列前部元素的引用，不移除它。
    /// 若队列为空则返回 None。
    pub fn peek(&self) -> Option<&T> {
        self.deque.front()
    }

    /// 若队列包含给定项则返回 true。
    pub fn contains(&self, item: &T) -> bool
    where
        T: PartialEq,
    {
        self.deque.contains(item)
    }

    /// 从队列中移除所有元素。
    pub fn clear(&mut self) {
        self.deque.clear();
    }

    /// 使用给定的比较器排序队列中的元素。
    /// 排序后，队列前部为“最小”的元素。
    pub fn sort<F>(&mut self, comparator: F)
    where
        F: FnMut(&T, &T) -> std::cmp::Ordering,
    {
        let mut vec: Vec<T> = self.deque.drain(..).collect();
        vec.sort_by(comparator);
        self.deque = VecDeque::from(vec);
    }
}
