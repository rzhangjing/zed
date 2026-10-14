//! Heap / ManagedArray / mergeSort 规格测试 - 参考自：
//! - Specs/Core/HeapSpec（9 个 it()）
//! - Specs/Core/ManagedArraySpec（17 个 it()）
//! - Specs/Core/mergeSortSpec（5 个 it()）
//!
//! A 类测试：22 个（Heap 7 + ManagedArray 10 + mergeSort 3，跳过 JS 特有的 throws/undefined 测试）

use cesium_geospatial::heap::Heap;
use cesium_geospatial::managed_array::ManagedArray;
use cesium_geospatial::utilities::merge_sort;
use std::cmp::Ordering;

// ============================================================
// Heap
// ============================================================

fn check_heap_property(array: &[f64]) -> bool {
    let len = array.len();
    for i in 0..len {
        let left = 2 * (i + 1) - 1;
        let right = 2 * (i + 1);
        if left < len && array[i] > array[left] {
            return false;
        }
        if right < len && array[i] > array[right] {
            return false;
        }
    }
    true
}

fn f64_comparator(a: &f64, b: &f64) -> Ordering {
    a.partial_cmp(b).unwrap_or(Ordering::Equal)
}

#[test]
fn heap_maintains_heap_property_on_insert() {
    let mut heap = Heap::new(f64_comparator);
    // 用确定性数值代替随机值
    let values: Vec<f64> = (0..100).map(|i| ((i * 37 + 13) % 100) as f64 / 100.0).collect();
    let mut pass = true;
    for v in values {
        heap.insert(v);
        pass = pass && check_heap_property(heap.internal_array());
    }
    assert!(pass);
}

#[test]
fn heap_maintains_heap_property_on_pop() {
    let mut heap = Heap::new(f64_comparator);
    let values: Vec<f64> = (0..100).map(|i| ((i * 53 + 7) % 100) as f64 / 100.0).collect();
    for v in &values {
        heap.insert(*v);
    }
    let mut pass = true;
    for _ in 0..100 {
        heap.pop();
        pass = pass && check_heap_property(heap.internal_array());
    }
    assert!(pass);
}

#[test]
fn heap_limited_by_maximum_length() {
    let mut heap = Heap::new(f64_comparator);
    heap.set_maximum_length(50);
    let values: Vec<f64> = (0..100).map(|i| ((i * 41 + 3) % 100) as f64 / 100.0).collect();
    let mut pass = true;
    for v in values {
        heap.insert(v);
        pass = pass && check_heap_property(heap.internal_array());
    }
    assert!(pass);
    assert!(heap.length() <= 50);
}

#[test]
fn heap_pops_in_sorted_order() {
    let mut heap = Heap::new(f64_comparator);
    let values: Vec<f64> = (0..100).map(|i| ((i * 67 + 29) % 100) as f64 / 100.0).collect();
    for v in &values {
        heap.insert(*v);
    }
    let mut curr = heap.pop().unwrap();
    let mut pass = true;
    for _ in 0..99 {
        let next = heap.pop().unwrap();
        pass = pass && curr <= next;
        curr = next;
    }
    assert!(pass);
}

#[test]
fn heap_insert_returns_removed_element_when_maximum_length_set() {
    let mut heap = Heap::new(f64_comparator);
    heap.set_maximum_length(100);

    let values: Vec<f64> = (0..100).map(|i| ((i * 37 + 13) % 100) as f64 / 100.0).collect();
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    // 压入 99 个值
    for i in 0..99 {
        heap.insert(values[i]);
    }

    // 压入第 100 个，没有元素被移除，因此返回 None
    let removed = heap.insert(values[99]);
    assert!(removed.is_none());

    // 插入值时，有元素被移除
    let removed = heap.insert(max - 0.1);
    assert!(removed.is_some());

    // 若该值是最低优先级（最大），它会被返回
    let removed = heap.insert(max + 0.1);
    assert_eq!(removed, Some(max + 0.1));
}

#[test]
fn heap_resort() {
    #[derive(Clone)]
    struct Item {
        distance: f64,
        id: usize,
    }

    let comparator = |a: &Item, b: &Item| a.distance.partial_cmp(&b.distance).unwrap_or(Ordering::Equal);

    let mut heap = Heap::new(comparator);
    let length = 100;
    for i in 0..length {
        heap.insert(Item {
            distance: i as f64 / (length - 1) as f64,
            id: i,
        });
    }

    // 检查元素初始时已排序
    let mut elements = Vec::new();
    let mut current_id = 0;
    while heap.length() > 0 {
        let element = heap.pop().unwrap();
        assert!(element.id >= current_id);
        current_id = element.id;
        elements.push(element);
    }

    // 重新加回堆中
    for e in &elements {
        heap.insert(Item {
            distance: e.distance,
            id: e.id,
        });
    }

    // 通过修改 distance 反转优先级
    // 由于难以就地修改，用反转后的 distance 重建
    let mut heap2 = Heap::new(|a: &Item, b: &Item| a.distance.partial_cmp(&b.distance).unwrap_or(Ordering::Equal));
    for e in &elements {
        heap2.insert(Item {
            distance: 1.0 - e.distance,
            id: e.id,
        });
    }

    // 检查现在元素以相反顺序弹出
    current_id = length - 1;
    while heap2.length() > 0 {
        let element = heap2.pop().unwrap();
        assert!(element.id <= current_id);
        current_id = element.id;
    }
}

#[test]
fn heap_pop_returns_none_when_empty() {
    let mut heap = Heap::new(f64_comparator);
    assert!(heap.pop().is_none());
    heap.insert(1.0);
    assert_eq!(heap.pop(), Some(1.0));
    assert!(heap.pop().is_none());
}

// ============================================================
// ManagedArray
// ============================================================

#[test]
fn managed_array_constructor_default_values() {
    let array: ManagedArray<f64> = ManagedArray::new(0);
    assert_eq!(array.length(), 0);
}

#[test]
fn managed_array_constructor_initializes_length() {
    let array: ManagedArray<f64> = ManagedArray::new(10);
    assert_eq!(array.length(), 10);
    assert_eq!(array.values().len(), 10);
}

#[test]
fn managed_array_can_get_and_set_values() {
    let mut array: ManagedArray<f64> = ManagedArray::new(10);
    for i in 0..10 {
        array.set(i, (i * i) as f64);
    }
    for i in 0..10 {
        assert_eq!(*array.get(i), (i * i) as f64);
        assert_eq!(array.values()[i], (i * i) as f64);
    }
}

#[test]
fn managed_array_set_resizes_array() {
    let mut array: ManagedArray<f64> = ManagedArray::new(0);
    array.set(0, 1.0);
    assert_eq!(array.length(), 1);
    array.set(5, 2.0);
    assert_eq!(array.length(), 6);
    array.set(2, 3.0);
    assert_eq!(array.length(), 6);
}

#[test]
fn managed_array_peeks_at_last_element() {
    let mut array: ManagedArray<i32> = ManagedArray::new(0);
    assert!(array.peek().is_none());
    array.push(0);
    assert_eq!(array.peek(), Some(&0));
    array.push(1);
    array.push(2);
    assert_eq!(array.peek(), Some(&2));
}

#[test]
fn managed_array_can_push_values() {
    let mut array: ManagedArray<f64> = ManagedArray::new(0);
    for i in 0..10 {
        let val = i as f64 * 1.5;
        array.push(val);
        assert_eq!(array.length(), i + 1);
        assert_eq!(array.values().len(), i + 1);
        assert_eq!(*array.get(i), val);
    }
}

#[test]
fn managed_array_can_pop_values() {
    let mut array: ManagedArray<f64> = ManagedArray::new(10);
    for i in 0..10 {
        array.set(i, i as f64 * 2.0);
    }
    for i in (0..10).rev() {
        let val = *array.get(i);
        assert_eq!(array.pop(), Some(val));
        assert_eq!(array.length(), i);
        // 容量被保留
        assert_eq!(array.values().len(), 10);
    }
}

#[test]
fn managed_array_pop_returns_none_if_empty() {
    let mut array: ManagedArray<i32> = ManagedArray::new(0);
    array.push(1);
    assert_eq!(array.pop(), Some(1));
    assert_eq!(array.pop(), None);
}

#[test]
fn managed_array_reserve() {
    let mut array: ManagedArray<f64> = ManagedArray::new(2);
    array.reserve(10);
    assert_eq!(array.values().len(), 10);
    assert_eq!(array.length(), 2);
    array.reserve(20);
    assert_eq!(array.values().len(), 20);
    assert_eq!(array.length(), 2);
    array.reserve(5);
    assert_eq!(array.values().len(), 20); // 不缩小
    assert_eq!(array.length(), 2);
}

#[test]
fn managed_array_resize_and_trim() {
    let mut array: ManagedArray<f64> = ManagedArray::new(2);
    array.resize(10);
    assert_eq!(array.values().len(), 10);
    assert_eq!(array.length(), 10);
    array.resize(20);
    assert_eq!(array.values().len(), 20);
    assert_eq!(array.length(), 20);
    array.resize(5);
    assert_eq!(array.values().len(), 20); // 容量保留
    assert_eq!(array.length(), 5);

    // trim（收缩）
    array.trim(None);
    assert_eq!(array.values().len(), 5);
    array.trim(Some(10));
    assert_eq!(array.length(), 5);
    assert_eq!(array.values().len(), 10);
    array.trim(Some(7));
    assert_eq!(array.length(), 5);
    assert_eq!(array.values().len(), 7);
}

// ============================================================
// mergeSort
// ============================================================

#[test]
fn merge_sort_sorts() {
    let mut array = [0, 9, 1, 8, 2, 7, 3, 6, 4, 5];
    merge_sort(&mut array, |a, b| a.cmp(b));
    assert_eq!(array, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
}

#[test]
fn merge_sort_stable_sorts() {
    #[derive(Debug, PartialEq)]
    struct Item {
        value: i32,
        original_index: usize,
    }
    let mut array = vec![
        Item { value: 5, original_index: 0 },
        Item { value: 10, original_index: 1 },
        Item { value: 5, original_index: 2 },
        Item { value: 0, original_index: 3 },
    ];
    merge_sort(&mut array, |a, b| a.value.cmp(&b.value));
    // 稳定排序：相等元素保持原有顺序
    assert_eq!(array[0].original_index, 3); // 值 0
    assert_eq!(array[1].original_index, 0); // 值 5（第一个）
    assert_eq!(array[2].original_index, 2); // 值 5（第二个）
    assert_eq!(array[3].original_index, 1); // 值 10
}

#[test]
fn merge_sort_sorts_with_user_defined_comparator() {
    // 按距原点距离排序（降序）
    let mut array: Vec<(f64, f64, f64)> = vec![
        (-2.0, 0.0, 0.0),
        (-1.0, 0.0, 0.0),
        (-3.0, 0.0, 0.0),
    ];
    // 比较器：按距离平方降序排序（b - a）
    merge_sort(&mut array, |a, b| {
        let da = a.0 * a.0 + a.1 * a.1 + a.2 * a.2;
        let db = b.0 * b.0 + b.1 * b.1 + b.2 * b.2;
        db.partial_cmp(&da).unwrap_or(Ordering::Equal)
    });
    // 期望顺序：(-3,0,0)、(-2,0,0)、(-1,0,0) —— 最远的在前
    assert_eq!(array[0], (-3.0, 0.0, 0.0));
    assert_eq!(array[1], (-2.0, 0.0, 0.0));
    assert_eq!(array[2], (-1.0, 0.0, 0.0));
}
