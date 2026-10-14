//! AssociativeArray 规格测试 - 参考自：
//! - Specs/Core/AssociativeArraySpec（5 个 it()）
//!
//! A 类测试：2 个（跳过 3 个 JS 特有的 `throws`/undefined-key 测试）

use cesium_geospatial::associative_array::AssociativeArray;

#[test]
fn constructor_has_expected_default_values() {
    let associative_array: AssociativeArray<i32> = AssociativeArray::new();
    assert_eq!(associative_array.length(), 0);
    assert!(associative_array.values().is_empty());
}

#[test]
fn can_manipulate_values() {
    let mut associative_array = AssociativeArray::new();

    assert!(!associative_array.contains("key1"));

    associative_array.set("key1", 1);
    associative_array.set("key2", 2);
    associative_array.set("key3", 3);

    assert_eq!(associative_array.get("key1"), Some(&1));
    assert_eq!(associative_array.get("key2"), Some(&2));
    assert_eq!(associative_array.get("key3"), Some(&3));
    assert_eq!(associative_array.length(), 3);

    assert!(associative_array.contains("key1"));
    assert!(associative_array.contains("key2"));
    assert!(associative_array.contains("key3"));

    {
        let values = associative_array.values();
        assert!(values.contains(&1));
        assert!(values.contains(&2));
        assert!(values.contains(&3));
        assert_eq!(values.len(), 3);
    }

    associative_array.set("key2", 4);
    assert_eq!(associative_array.length(), 3);

    {
        let values = associative_array.values();
        assert!(values.contains(&1));
        assert!(!values.contains(&2));
        assert!(values.contains(&4));
        assert!(values.contains(&3));
        assert_eq!(values.len(), 3);
    }

    assert!(associative_array.remove("key1"));
    assert_eq!(associative_array.get("key1"), None);
    assert!(!associative_array.contains("key1"));
    {
        let values = associative_array.values();
        assert!(!values.contains(&1));
        assert!(values.contains(&4));
        assert!(values.contains(&3));
        assert_eq!(values.len(), 2);
    }
    assert!(!associative_array.remove("key1"));

    associative_array.remove_all();
    assert_eq!(associative_array.length(), 0);
    assert!(associative_array.values().is_empty());
}
