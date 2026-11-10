//! 一个双向链表。节点通过 `Rc<RefCell<_>>` 共享，以便调用方
//! 可以持有节点的句柄（共享所有权语义）并按标识
//! 进行比较。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::unnecessary_map_or)]
use std::cell::RefCell;
use std::rc::Rc;

/// 对双向链表节点的共享引用。
pub type NodeRef<T> = Rc<RefCell<DoublyLinkedListNode<T>>>;

/// 双向链表中的一个节点。
pub struct DoublyLinkedListNode<T> {
    pub item: T,
    pub previous: Option<NodeRef<T>>,
    pub next: Option<NodeRef<T>>,
}

/// 一个双向链表。
pub struct DoublyLinkedList<T> {
    /// 首节点引用，空表时为 None。
    head: Option<NodeRef<T>>,
    /// 尾节点引用，空表时为 None。
    tail: Option<NodeRef<T>>,
    /// 当前链表中的节点数量。
    length: usize,
}

impl<T> Default for DoublyLinkedList<T> {
    /// 默认构造一个空链表。
    fn default() -> Self {
        Self::new()
    }
}

impl<T> DoublyLinkedList<T> {
    /// 创建新的空双向链表。
    pub fn new() -> Self {
        Self {
            head: None,
            tail: None,
            length: 0,
        }
    }

    /// 获取链表中节点的数量。
    pub fn length(&self) -> usize {
        self.length
    }

    /// 获取头节点（若有）。
    pub fn head(&self) -> Option<NodeRef<T>> {
        self.head.clone()
    }

    /// 获取尾节点（若有）。
    pub fn tail(&self) -> Option<NodeRef<T>> {
        self.tail.clone()
    }

    /// 将项添加到链表末尾，返回新节点。
    pub fn add(&mut self, item: T) -> NodeRef<T> {
        let node = Rc::new(RefCell::new(DoublyLinkedListNode {
            item,
            previous: self.tail.clone(),
            next: None,
        }));

        if let Some(tail) = &self.tail {
            tail.borrow_mut().next = Some(node.clone());
            self.tail = Some(node.clone());
        } else {
            self.head = Some(node.clone());
            self.tail = Some(node.clone());
        }

        self.length += 1;

        node
    }

    /// 从链表中移除给定节点。若 `node` 为 `None` 则不做任何事
    /// （把 `None` 视作空操作）。
    pub fn remove(&mut self, node: Option<&NodeRef<T>>) {
        if let Some(node) = node {
            remove_node(self, node);
            self.length -= 1;
        }
    }

    /// 将 `next_node` 移到 `node` 之后。
    pub fn splice(&mut self, node: &NodeRef<T>, next_node: &NodeRef<T>) {
        if Rc::ptr_eq(node, next_node) {
            return;
        }

        // 移除 next_node，然后插入到 node 之后。
        remove_node(self, next_node);

        let old_node_next = node.borrow().next.clone();
        node.borrow_mut().next = Some(next_node.clone());

        // 若 node 是尾节点，则 next_node 成为新的尾节点。
        let node_is_tail = self
            .tail
            .as_ref()
            .map_or(false, |tail| Rc::ptr_eq(tail, node));
        if node_is_tail {
            self.tail = Some(next_node.clone());
        } else if let Some(old_next) = &old_node_next {
            old_next.borrow_mut().previous = Some(next_node.clone());
        }

        next_node.borrow_mut().next = old_node_next;
        next_node.borrow_mut().previous = Some(node.clone());
    }
}

/// 从链表中解除对指定节点的链接（不回收内存，由调用方持有）。
///
/// 根据该节点是否有前驱/后继，分四种情形重新接回相邻节点并更新 head/tail。
fn remove_node<T>(list: &mut DoublyLinkedList<T>, node: &NodeRef<T>) {
    let previous = node.borrow().previous.clone();
    let next = node.borrow().next.clone();

    if let (Some(prev), Some(next)) = (&previous, &next) {
        prev.borrow_mut().next = Some(next.clone());
        next.borrow_mut().previous = Some(prev.clone());
    } else if let Some(prev) = &previous {
        // 移除最后一个节点。
        prev.borrow_mut().next = None;
        list.tail = Some(prev.clone());
    } else if let Some(next) = &next {
        // 移除第一个节点。
        next.borrow_mut().previous = None;
        list.head = Some(next.clone());
    } else {
        // 移除链表中唯一的节点。
        list.head = None;
        list.tail = None;
    }

    node.borrow_mut().next = None;
    node.borrow_mut().previous = None;
}
