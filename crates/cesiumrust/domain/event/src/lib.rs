//! cesium-event：类型安全的事件系统。
//!
//! 领域层 —— 纯 Rust，无框架依赖。提供 [`Event`]（可挂多个监听器、带参数类型
//! `Args`）与其无参特化 [`SimpleEvent`]：支持注册 / 注销 / 触发 / 清空监听器，
//! 并借助内部可变性（[`RefCell`]）在只读借用下完成订阅表的增删与派发。

use std::cell::RefCell;
use std::collections::HashMap;

/// 事件监听器的唯一标识符。
///
/// 对新类型封装的自增序号做 opaque 处理：仅用作 [`Event::remove_listener`] 的凭据，
/// 不暴露内部数值语义；派生 `Hash`/`Eq` 以便作为键或放入集合。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerId(u64);

/// 监听器映射的类型别名，用于降低复杂度。
type ListenerMap<Args> = HashMap<u64, Box<dyn Fn(&Args)>>;

/// 一个可以有多个监听器的通用事件。
///
/// 类型参数 `Args` 是触发时传递给各监听器的参数类型（通常是一个元组）。
/// 内部以自增 id 作为键把监听器存入哈希表，因而同一事件可挂载任意多个回调。
pub struct Event<Args: Clone> {
    /// 已注册监听器表：键为自增分配的监听器 id，值为擦除类型后的回调闭包。
    /// 用 [`RefCell`] 包裹以在 `&self`（只读引用）下仍可增删。
    listeners: RefCell<ListenerMap<Args>>,
    /// 下一次注册要使用的自增 id，从 0 起，每次 `add_listener` 递增 1。
    next_id: RefCell<u64>,
}

impl<Args: Clone> Event<Args> {
    /// 创建一个空事件。
    pub fn new() -> Self {
        Self {
            listeners: RefCell::new(HashMap::new()),
            next_id: RefCell::new(0),
        }
    }

    /// 返回当前已订阅的监听器数量（等价于集合基数）。
    pub fn number_of_listeners(&self) -> usize {
        self.listeners.borrow().len()
    }

    /// 若没有任何监听器则返回 true（等价于 `number_of_listeners() == 0`）。
    /// 仅做只读借用，不改变订阅表状态。
    pub fn is_empty(&self) -> bool {
        self.listeners.borrow().is_empty()
    }

    /// 注册一个回调函数，只要事件被触发就会执行。
    ///
    /// 返回一个可用于后续移除该监听器的 [`ListenerId`]。
    pub fn add_listener<F>(&self, listener: F) -> ListenerId
    where
        F: Fn(&Args) + 'static,
    {
        // 取出并预占下一个自增 id（同一事件内保证唯一）
        let mut next_id = self.next_id.borrow_mut();
        let id = *next_id;
        *next_id += 1;

        // 把闭包装箱擦除具体类型后按 id 存入监听器表，并回传其标识
        self.listeners.borrow_mut().insert(id, Box::new(listener));
        ListenerId(id)
    }

    /// 注销先前已注册的回调。
    ///
    /// 若该 id 对应的监听器确实存在并被移除则返回 true，否则返回 false。
    pub fn remove_listener(&self, id: ListenerId) -> bool {
        self.listeners.borrow_mut().remove(&id.0).is_some()
    }

    /// 触发事件：以给定参数依次调用每个已注册的监听器。
    ///
    /// 先以只读借用取得监听器表快照，再遍历所有值同步派发；派发期间不应再次
    /// 变更监听器集合，否则会因借用冲突而 panic。
    pub fn raise(&self, args: &Args) {
        // 只读借用整张监听器表，按哈希存储顺序逐个调用
        let listeners = self.listeners.borrow();
        for listener in listeners.values() {
            listener(args);
        }
    }

    /// 移除所有监听器，使事件回到空订阅状态。
    ///
    /// 以可变借用清空订阅表；此前发放的 [`ListenerId`] 随即失效（再次移除返回 false）。
    pub fn clear(&self) {
        self.listeners.borrow_mut().clear();
    }
}

impl<Args: Clone> Default for Event<Args> {
    /// 默认事件即空事件，语义与 [`Event::new`] 一致。
    fn default() -> Self {
        Self::new()
    }
}

impl<Args: Clone> std::fmt::Debug for Event<Args> {
    /// 手写 Debug：闭包不可打印，故仅以监听器数量近似呈现事件状态。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Event")
            .field("listener_count", &self.number_of_listeners())
            .finish()
    }
}

/// 一个无参数的简单事件。
pub type SimpleEvent = Event<()>;

impl SimpleEvent {
    /// 以无参数触发事件。
    pub fn raise_simple(&self) {
        self.raise(&());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn test_add_and_raise() {
        let event: Event<i32> = Event::new();
        let received = Rc::new(Cell::new(0));
        let received_clone = received.clone();

        event.add_listener(move |val| {
            received_clone.set(*val);
        });

        event.raise(&42);
        assert_eq!(received.get(), 42);
    }

    #[test]
    fn test_multiple_listeners() {
        let event: Event<i32> = Event::new();
        let sum = Rc::new(Cell::new(0));

        let sum1 = sum.clone();
        event.add_listener(move |val| {
            sum1.set(sum1.get() + val);
        });

        let sum2 = sum.clone();
        event.add_listener(move |val| {
            sum2.set(sum2.get() + val * 2);
        });

        event.raise(&10);
        assert_eq!(sum.get(), 30); // 10 + 20
    }

    #[test]
    fn test_remove_listener() {
        let event: Event<i32> = Event::new();
        let count = Rc::new(Cell::new(0));
        let count_clone = count.clone();

        let id = event.add_listener(move |_| {
            count_clone.set(count_clone.get() + 1);
        });

        event.raise(&0);
        assert_eq!(count.get(), 1);

        assert!(event.remove_listener(id));
        event.raise(&0);
        assert_eq!(count.get(), 1); // 不应再递增
    }

    #[test]
    fn test_number_of_listeners() {
        let event: Event<()> = Event::new();
        assert_eq!(event.number_of_listeners(), 0);

        let id1 = event.add_listener(|_| {});
        assert_eq!(event.number_of_listeners(), 1);

        let _id2 = event.add_listener(|_| {});
        assert_eq!(event.number_of_listeners(), 2);

        event.remove_listener(id1);
        assert_eq!(event.number_of_listeners(), 1);
    }

    #[test]
    fn test_simple_event() {
        let event = SimpleEvent::new();
        let fired = Rc::new(Cell::new(false));
        let fired_clone = fired.clone();

        event.add_listener(move |_| {
            fired_clone.set(true);
        });

        assert!(!fired.get());
        event.raise_simple();
        assert!(fired.get());
    }

    #[test]
    fn test_clear() {
        let event: Event<()> = Event::new();
        event.add_listener(|_| {});
        event.add_listener(|_| {});
        assert_eq!(event.number_of_listeners(), 2);

        event.clear();
        assert_eq!(event.number_of_listeners(), 0);
    }
}
