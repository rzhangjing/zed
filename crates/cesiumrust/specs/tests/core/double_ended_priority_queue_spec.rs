//! DoubleEndedPriorityQueue 规格测试 - 参考自：
//! - Specs/Core/DoubleEndedPriorityQueueSpec（26 个 it()）
//!
//! A 类测试：24 个（跳过 4 个 JS 特有的 `throws` 构造函数/setter 测试）

use cesium_geospatial::double_ended_priority_queue::DoubleEndedPriorityQueue;
use std::cmp::Ordering;

type Cmp = fn(&i32, &i32) -> Ordering;
type FCmp = fn(&f64, &f64) -> Ordering;

fn cmp_asc(a: &i32, b: &i32) -> Ordering {
    a.cmp(b)
}

fn cmp_f64(a: &f64, b: &f64) -> Ordering {
    a.partial_cmp(b).unwrap_or(Ordering::Equal)
}

fn new_queue() -> DoubleEndedPriorityQueue<i32, Cmp> {
    DoubleEndedPriorityQueue::new(cmp_asc, None)
}

fn new_queue_max(max: usize) -> DoubleEndedPriorityQueue<i32, Cmp> {
    DoubleEndedPriorityQueue::new(cmp_asc, Some(max))
}

/// 确定性的伪随机数生成器（替代 Math.random()）。
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
    fn next_f64(&mut self) -> f64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.state >> 33) as f64) / ((1u64 << 31) as f64)
    }
}

/// 通过 removeMinimum / removeMaximum 排空克隆来校验队列，
/// 并检查结果是否有序。
fn is_valid_queue<T, F>(queue: &DoubleEndedPriorityQueue<T, F>) -> bool
where
    T: Clone + PartialOrd,
    F: Clone + Fn(&T, &T) -> Ordering,
{
    let mut min_array = Vec::new();
    let mut max_array = Vec::new();

    let mut min_queue = queue.clone_queue();
    let mut max_queue = queue.clone_queue();

    while min_queue.length() > 0 {
        min_array.push(min_queue.remove_minimum().unwrap());
    }
    while max_queue.length() > 0 {
        max_array.push(max_queue.remove_maximum().unwrap());
    }

    if min_queue.length() != 0 || max_queue.length() != 0 {
        return false;
    }

    for i in 0..min_array.len().saturating_sub(1) {
        if min_array[i] > min_array[i + 1] {
            return false;
        }
    }
    for i in 0..max_array.len().saturating_sub(1) {
        if max_array[i] < max_array[i + 1] {
            return false;
        }
    }
    true
}

#[test]
fn gets_comparator() {
    let cmp: Cmp = cmp_asc;
    let queue = DoubleEndedPriorityQueue::new(cmp, None);
    let stored = *queue.comparator();
    // 比较行为，而非函数指针地址（编译器并不保证地址稳定，
    // 因此对它们使用 `==` 没有意义）。
    for (a, b) in [(1, 2), (2, 1), (3, 3), (-5, 7)] {
        assert_eq!(stored(&a, &b), cmp(&a, &b));
    }
}

#[test]
fn uses_different_comparator() {
    let cmp: Cmp = |a, b| b.cmp(a);
    let mut queue = DoubleEndedPriorityQueue::new(cmp, None);
    queue.insert(1);
    queue.insert(2);

    // 比较器被翻转，因此 2 是最小值、1 是最大值。
    assert_eq!(queue.length(), 2);
    assert_eq!(queue.get_minimum(), Some(&2));
    assert_eq!(queue.get_maximum(), Some(&1));
}

#[test]
fn checks_state_of_default_empty_queue() {
    let queue = new_queue();
    assert_eq!(queue.length(), 0);
    assert_eq!(queue.maximum_length(), None);
    assert_eq!(queue.internal_array().len(), 0);
    assert_eq!(queue.get_minimum(), None);
    assert_eq!(queue.get_maximum(), None);
}

#[test]
fn inserts_one_element_into_queue() {
    let mut queue = new_queue();
    queue.insert(1);
    assert_eq!(queue.length(), 1);
    assert_eq!(queue.internal_array().len(), 1);
    assert_eq!(queue.get_minimum(), Some(&1));
    assert_eq!(queue.get_maximum(), Some(&1));
}

#[test]
fn inserts_two_elements_into_queue() {
    let mut queue = new_queue();
    queue.insert(1);
    queue.insert(2);
    assert_eq!(queue.length(), 2);
    assert_eq!(queue.internal_array().len(), 2);
    assert_eq!(queue.get_minimum(), Some(&1));
    assert_eq!(queue.get_maximum(), Some(&2));
}

#[test]
fn inserts_three_elements_into_queue() {
    let mut queue = new_queue();
    queue.insert(1);
    queue.insert(2);
    queue.insert(3);
    assert_eq!(queue.length(), 3);
    assert_eq!(queue.internal_array().len(), 3);
    assert_eq!(queue.get_minimum(), Some(&1));
    assert_eq!(queue.get_maximum(), Some(&3));
}

#[test]
fn inserts_four_elements_into_queue() {
    let mut queue = new_queue();
    queue.insert(1);
    queue.insert(2);
    queue.insert(3);
    queue.insert(4);
    assert_eq!(queue.length(), 4);
    assert_eq!(queue.internal_array().len(), 4);
    assert_eq!(queue.get_minimum(), Some(&1));
    assert_eq!(queue.get_maximum(), Some(&4));
}

#[test]
fn insert_removes_and_returns_minimum_when_full() {
    let mut queue = new_queue_max(1);
    let nothing = queue.insert(1);
    let removed = queue.insert(2);

    assert_eq!(queue.length(), 1);
    assert_eq!(queue.maximum_length(), Some(1));
    assert_eq!(queue.internal_array().len(), 1);
    assert_eq!(queue.get_minimum(), Some(&2));
    assert_eq!(queue.get_maximum(), Some(&2));
    assert_eq!(nothing, None);
    assert_eq!(removed, Some(1));
}

#[derive(Clone, Debug, PartialEq)]
struct Obj {
    value: i32,
    id: i32,
}

#[test]
fn insert_returns_element_when_low_priority_and_full() {
    fn obj_cmp(a: &Obj, b: &Obj) -> Ordering {
        a.value.cmp(&b.value)
    }
    let mut queue = DoubleEndedPriorityQueue::new(obj_cmp, Some(2));

    let result1 = queue.insert(Obj { value: 1, id: 0 });
    let result2 = queue.insert(Obj { value: 2, id: 0 });
    let result3 = queue.insert(Obj { value: 1, id: 1 }); // 被忽略：优先级等于最小值
    let result4 = queue.insert(Obj { value: 0, id: 1 }); // 被忽略：优先级低于最小值

    assert_eq!(queue.length(), 2);
    assert_eq!(queue.maximum_length(), Some(2));
    assert_eq!(queue.internal_array().len(), 2);
    assert_eq!(queue.get_minimum().unwrap().id, 0);
    assert_eq!(result1, None);
    assert_eq!(result2, None);
    assert_eq!(result3, Some(Obj { value: 1, id: 1 }));
    assert_eq!(result4, Some(Obj { value: 0, id: 1 }));
}

#[test]
fn remove_and_return_minimum_element() {
    let mut queue = new_queue();
    queue.insert(1);
    queue.insert(2);
    queue.insert(3);

    let minimum_value = queue.remove_minimum();

    assert_eq!(queue.length(), 2);
    assert_eq!(minimum_value, Some(1));
    assert_eq!(queue.get_minimum(), Some(&2));
    // 该元素已被解引用。
    assert_eq!(queue.internal_array()[2], None);
}

#[test]
fn remove_minimum_returns_none_when_empty() {
    let mut queue = new_queue();
    assert_eq!(queue.remove_minimum(), None);
}

#[test]
fn remove_and_return_maximum_element() {
    let mut queue = new_queue();
    queue.insert(1);
    queue.insert(2);
    queue.insert(3);

    let maximum_value = queue.remove_maximum();

    assert_eq!(queue.length(), 2);
    assert_eq!(maximum_value, Some(3));
    assert_eq!(queue.get_maximum(), Some(&2));
    // 该元素已被解引用。
    assert_eq!(queue.internal_array()[2], None);
}

#[test]
fn remove_maximum_returns_none_when_empty() {
    let mut queue = new_queue();
    assert_eq!(queue.remove_maximum(), None);
}

#[test]
fn clones_queue() {
    let mut queue = new_queue_max(4);
    queue.insert(1);
    queue.insert(2);

    let clone = queue.clone_queue();
    assert_eq!(clone.length(), queue.length());
    assert_eq!(clone.maximum_length(), queue.maximum_length());
    assert_eq!(clone.get_maximum(), queue.get_maximum());
    assert_eq!(clone.get_minimum(), queue.get_minimum());
}

#[test]
fn resets_queue() {
    let mut queue = new_queue();
    queue.insert(1);
    queue.insert(2);
    queue.reset();

    assert_eq!(queue.length(), 0);
    assert_eq!(queue.get_minimum(), None);
    assert_eq!(queue.get_maximum(), None);
    // 这些元素已被解引用。
    assert_eq!(queue.internal_array().len(), 0);
}

#[test]
fn resets_queue_with_maximum_length() {
    let mut queue = new_queue_max(1);
    queue.insert(1);
    queue.reset();

    assert_eq!(queue.length(), 0);
    assert_eq!(queue.get_minimum(), None);
    assert_eq!(queue.get_maximum(), None);
    // 该元素已被解引用，但数组大小保持不变。
    assert_eq!(queue.internal_array().len(), 1);
    assert_eq!(queue.internal_array()[0], None);
}

#[test]
fn creates_queue_with_maximum_length_of_zero() {
    let mut queue = new_queue_max(0);
    queue.insert(1);

    assert_eq!(queue.length(), 0);
    assert_eq!(queue.maximum_length(), Some(0));
    assert_eq!(queue.internal_array().len(), 0);
    assert_eq!(queue.get_minimum(), None);
    assert_eq!(queue.get_maximum(), None);
}

#[test]
fn creates_queue_with_maximum_length_of_one() {
    let mut queue = new_queue_max(1);
    queue.insert(1);
    queue.insert(2);

    assert_eq!(queue.length(), 1);
    assert_eq!(queue.maximum_length(), Some(1));
    assert_eq!(queue.internal_array().len(), 1);
    assert_eq!(queue.get_minimum(), Some(&2));
    assert_eq!(queue.get_maximum(), Some(&2));
}

#[test]
fn sets_maximum_length_to_undefined() {
    let mut queue = new_queue();
    queue.set_maximum_length(Some(2));
    queue.insert(1);
    queue.insert(2);

    queue.set_maximum_length(None);
    queue.insert(3);

    assert_eq!(queue.length(), 3);
    assert_eq!(queue.maximum_length(), None);
    assert_eq!(queue.get_minimum(), Some(&1));
    assert_eq!(queue.get_maximum(), Some(&3));
}

#[test]
fn sets_maximum_length_to_less_than_current_length() {
    let mut queue = new_queue();
    let maximum_length: i32 = 5;
    for i in 0..(maximum_length * 2) {
        queue.insert(i);
    }
    queue.set_maximum_length(Some(maximum_length as usize));

    assert_eq!(queue.length(), maximum_length as usize);
    assert_eq!(queue.maximum_length(), Some(maximum_length as usize));
    assert_eq!(queue.internal_array().len(), maximum_length as usize);
    assert_eq!(queue.get_minimum(), Some(&maximum_length));
    assert_eq!(queue.get_maximum(), Some(&(maximum_length * 2 - 1)));
}

#[test]
fn maintains_priority_with_ascending_insertions() {
    let length = 200;
    let maximum_length = 100;
    let mut queue = new_queue_max(maximum_length);

    let mut pass = true;
    for i in 0..length {
        queue.insert(i);
        pass = pass && is_valid_queue(&queue);
    }
    assert!(pass);
}

#[test]
fn maintains_priority_with_descending_insertions() {
    let length = 200;
    let maximum_length = 100;
    let mut queue = new_queue_max(maximum_length);

    let mut pass = true;
    for i in 0..length {
        let value = length - 1 - i;
        queue.insert(value);
        pass = pass && is_valid_queue(&queue);
    }
    assert!(pass);
}

#[test]
fn maintains_priority_with_random_insertions() {
    let length = 200;
    let maximum_length = 100;
    let cmp: FCmp = cmp_f64;
    let mut queue = DoubleEndedPriorityQueue::new(cmp, Some(maximum_length));

    let mut rng = Lcg::new(42);
    let mut pass = true;
    for _ in 0..length {
        let value = rng.next_f64();
        queue.insert(value);
        pass = pass && is_valid_queue(&queue);
    }
    assert!(pass);
}

#[test]
fn resorts_queue() {
    let cmp: FCmp = cmp_f64;
    let mut queue = DoubleEndedPriorityQueue::new(cmp, None);

    let length = 200;
    for _ in 0..length {
        queue.insert(0.0);
    }

    // 将队列中所有值改为随机值，使其无序。
    let mut rng = Lcg::new(123);
    {
        let array = queue.internal_array_mut();
        for slot in array.iter_mut() {
            *slot = Some(rng.next_f64());
        }
    }

    queue.resort();

    assert!(is_valid_queue(&queue));
}
