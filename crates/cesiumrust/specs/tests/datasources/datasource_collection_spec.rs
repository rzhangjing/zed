//! DataSources/DataSourceCollectionSpec → Rust 集成测试
//! 覆盖：add、remove、contains、indexOf、get、getByName、raise、lower、
//! raiseToTop、lowerToBottom、removeAll、destroy

use cesium_datasource::datasource_collection::DataSourceCollection;
use cesium_datasource::entity_collection::DataSource;

// ─── 基本操作 ───────────────────────────────────────────────────────

#[test]
fn dsc_contains_get_length_index_of() {
    let mut collection = DataSourceCollection::new();
    let _source = DataSource::new("source1");

    assert_eq!(collection.length(), 0);
    assert!(!collection.contains("source1"));

    collection.add(DataSource::new("source0"));
    collection.add(DataSource::new("source1"));
    collection.add(DataSource::new("source2"));

    assert_eq!(collection.length(), 3);
    assert_eq!(collection.get(1).unwrap().name, "source1");
    assert_eq!(collection.index_of("source1"), Some(1));
    assert!(collection.contains("source1"));

    collection.remove("source0");
    assert_eq!(collection.index_of("source1"), Some(0));

    assert!(collection.remove("source1"));
    assert!(!collection.contains("source1"));
}

#[test]
fn dsc_get_by_name() {
    let mut collection = DataSourceCollection::new();
    collection.add(DataSource::new("Name1"));
    collection.add(DataSource::new("Name1"));
    collection.add(DataSource::new("Name2"));

    let result = collection.get_by_name("Name1");
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].name, "Name1");
    assert_eq!(result[1].name, "Name1");
}

#[test]
fn dsc_remove_fails_for_non_member() {
    let mut collection = DataSourceCollection::new();
    assert!(!collection.remove("nonexistent"));
}

// ─── 排序操作 ────────────────────────────────────────────────────

#[test]
fn dsc_raise() {
    let mut collection = DataSourceCollection::new();
    collection.add(DataSource::new("a"));
    collection.add(DataSource::new("b"));
    collection.add(DataSource::new("c"));

    collection.raise("a");
    assert_eq!(collection.get(0).unwrap().name, "b");
    assert_eq!(collection.get(1).unwrap().name, "a");
    assert_eq!(collection.get(2).unwrap().name, "c");
}

#[test]
fn dsc_raise_at_top_no_op() {
    let mut collection = DataSourceCollection::new();
    collection.add(DataSource::new("a"));
    collection.add(DataSource::new("b"));

    // 提升最后一个元素应为空操作
    collection.raise("b");
    assert_eq!(collection.get(0).unwrap().name, "a");
    assert_eq!(collection.get(1).unwrap().name, "b");
}

#[test]
fn dsc_lower() {
    let mut collection = DataSourceCollection::new();
    collection.add(DataSource::new("a"));
    collection.add(DataSource::new("b"));
    collection.add(DataSource::new("c"));

    collection.lower("c");
    assert_eq!(collection.get(0).unwrap().name, "a");
    assert_eq!(collection.get(1).unwrap().name, "c");
    assert_eq!(collection.get(2).unwrap().name, "b");
}

#[test]
fn dsc_lower_at_bottom_no_op() {
    let mut collection = DataSourceCollection::new();
    collection.add(DataSource::new("a"));
    collection.add(DataSource::new("b"));

    // 降低第一个元素应为空操作
    collection.lower("a");
    assert_eq!(collection.get(0).unwrap().name, "a");
    assert_eq!(collection.get(1).unwrap().name, "b");
}

#[test]
fn dsc_raise_to_top() {
    let mut collection = DataSourceCollection::new();
    collection.add(DataSource::new("a"));
    collection.add(DataSource::new("b"));
    collection.add(DataSource::new("c"));

    collection.raise_to_top("a");
    assert_eq!(collection.get(0).unwrap().name, "b");
    assert_eq!(collection.get(1).unwrap().name, "c");
    assert_eq!(collection.get(2).unwrap().name, "a");
}

#[test]
fn dsc_lower_to_bottom() {
    let mut collection = DataSourceCollection::new();
    collection.add(DataSource::new("a"));
    collection.add(DataSource::new("b"));
    collection.add(DataSource::new("c"));

    collection.lower_to_bottom("c");
    assert_eq!(collection.get(0).unwrap().name, "c");
    assert_eq!(collection.get(1).unwrap().name, "a");
    assert_eq!(collection.get(2).unwrap().name, "b");
}

// ─── removeAll ──────────────────────────────────────────────────────────────

#[test]
fn dsc_remove_all() {
    let mut collection = DataSourceCollection::new();
    collection.add(DataSource::new("a"));
    collection.add(DataSource::new("b"));
    collection.add(DataSource::new("c"));

    collection.remove_all();
    assert_eq!(collection.length(), 0);
}

// ─── destroy ────────────────────────────────────────────────────────────────

#[test]
fn dsc_destroy() {
    let mut collection = DataSourceCollection::new();
    collection.add(DataSource::new("a"));
    collection.add(DataSource::new("b"));

    assert!(!collection.is_destroyed());
    collection.destroy();
    assert!(collection.is_destroyed());
    assert_eq!(collection.length(), 0);
}

// ─── insert ─────────────────────────────────────────────────────────────────

#[test]
fn dsc_insert_at_index() {
    let mut collection = DataSourceCollection::new();
    collection.add(DataSource::new("a"));
    collection.add(DataSource::new("c"));

    collection.insert(1, DataSource::new("b"));
    assert_eq!(collection.get(0).unwrap().name, "a");
    assert_eq!(collection.get(1).unwrap().name, "b");
    assert_eq!(collection.get(2).unwrap().name, "c");
}

// ─── remove_at ──────────────────────────────────────────────────────────────

#[test]
fn dsc_remove_at() {
    let mut collection = DataSourceCollection::new();
    collection.add(DataSource::new("a"));
    collection.add(DataSource::new("b"));
    collection.add(DataSource::new("c"));

    let removed = collection.remove_at(1);
    assert_eq!(removed.unwrap().name, "b");
    assert_eq!(collection.length(), 2);
    assert_eq!(collection.get(1).unwrap().name, "c");
}

#[test]
fn dsc_remove_at_out_of_bounds() {
    let mut collection = DataSourceCollection::new();
    collection.add(DataSource::new("a"));

    let removed = collection.remove_at(5);
    assert!(removed.is_none());
}
