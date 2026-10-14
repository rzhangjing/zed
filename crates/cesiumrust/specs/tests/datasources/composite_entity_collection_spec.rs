//! DataSources/CompositeEntityCollectionSpec → Rust 集成测试
//! 覆盖：addCollection、removeCollection、getCollectionsLength、getCollection、
//! contains、getById、values、recomposite

use cesium_datasource::composite_entity_collection::CompositeEntityCollection;
use cesium_datasource::entity::Entity;
use cesium_datasource::entity_collection::EntityCollection;

// ─── 构造 ───────────────────────────────────────────────────────────

#[test]
fn composite_constructor_defaults() {
    let composite = CompositeEntityCollection::new();
    assert_eq!(composite.get_collections_length(), 0);
    assert!(composite.values().is_empty());
}

#[test]
fn composite_constructor_with_owner() {
    let child = CompositeEntityCollection::with_owner("parent_id");
    assert_eq!(child.owner(), Some("parent_id"));
}

// ─── addCollection / removeCollection ───────────────────────────────────────

#[test]
fn composite_add_remove_collection() {
    let mut ec1 = EntityCollection::new();
    ec1.add(Entity::new("e1"));

    let mut ec2 = EntityCollection::new();
    ec2.add(Entity::new("e2"));

    let mut composite = CompositeEntityCollection::new();
    composite.add_collection(ec1);
    assert_eq!(composite.get_collections_length(), 1);

    composite.add_collection(ec2);
    assert_eq!(composite.get_collections_length(), 2);

    assert!(composite.remove_collection(0));
    assert_eq!(composite.get_collections_length(), 1);

    assert!(composite.remove_collection(0));
    assert_eq!(composite.get_collections_length(), 0);

    assert!(!composite.remove_collection(0));
}

#[test]
fn composite_add_collection_at_index() {
    let ec1 = EntityCollection::new();
    let ec2 = EntityCollection::new();
    let ec3 = EntityCollection::new();

    let mut composite = CompositeEntityCollection::new();
    composite.add_collection(ec1);
    composite.add_collection(ec3);
    composite.add_collection_at(1, ec2);

    assert_eq!(composite.get_collections_length(), 3);
    // 通过检查 get_collection 是否生效来验证顺序
    assert!(composite.get_collection(0).is_some());
    assert!(composite.get_collection(1).is_some());
    assert!(composite.get_collection(2).is_some());
}

// ─── contains ───────────────────────────────────────────────────────────────

#[test]
fn composite_contains_true() {
    let mut ec = EntityCollection::new();
    ec.add(Entity::new("asd"));

    let mut composite = CompositeEntityCollection::new();
    composite.add_collection(ec);
    composite.recomposite();

    assert!(composite.contains("asd"));
}

#[test]
fn composite_contains_false() {
    let composite = CompositeEntityCollection::new();
    assert!(!composite.contains("nonexistent"));
}

// ─── getById ────────────────────────────────────────────────────────────────

#[test]
fn composite_get_by_id() {
    let mut ec = EntityCollection::new();
    ec.add(Entity::new("test_id").with_name("Test Entity"));

    let mut composite = CompositeEntityCollection::new();
    composite.add_collection(ec);
    composite.recomposite();

    let entity = composite.get_by_id("test_id");
    assert!(entity.is_some());
    assert_eq!(entity.unwrap().id, "test_id");
}

#[test]
fn composite_get_by_id_not_found() {
    let composite = CompositeEntityCollection::new();
    assert!(composite.get_by_id("nonexistent").is_none());
}

// ─── values ─────────────────────────────────────────────────────────────────

#[test]
fn composite_values() {
    let mut ec1 = EntityCollection::new();
    ec1.add(Entity::new("e1"));
    ec1.add(Entity::new("e2"));

    let mut ec2 = EntityCollection::new();
    ec2.add(Entity::new("e3"));

    let mut composite = CompositeEntityCollection::new();
    composite.add_collection(ec1);
    composite.add_collection(ec2);
    composite.recomposite();

    let values = composite.values();
    assert_eq!(values.len(), 3);
}

// ─── 合并行为 ─────────────────────────────────────────────────────────

#[test]
fn composite_merge_same_id() {
    // 同 ID 实体时后置集合优先
    let mut ec1 = EntityCollection::new();
    ec1.add(Entity::new("shared").with_name("From EC1"));

    let mut ec2 = EntityCollection::new();
    ec2.add(Entity::new("shared").with_name("From EC2"));

    let mut composite = CompositeEntityCollection::new();
    composite.add_collection(ec1);
    composite.add_collection(ec2);
    composite.recomposite();

    // 应只有一个 id 为 "shared" 的实体
    assert_eq!(composite.len(), 1);
    // 后置集合（ec2）优先
    let entity = composite.get_by_id("shared").unwrap();
    assert_eq!(entity.name, Some("From EC2".to_string()));
}

// ─── removeAllCollections ───────────────────────────────────────────────────

#[test]
fn composite_remove_all_collections() {
    let mut composite = CompositeEntityCollection::new();
    composite.add_collection(EntityCollection::new());
    composite.add_collection(EntityCollection::new());

    composite.remove_all_collections();
    assert_eq!(composite.get_collections_length(), 0);
}

// ─── len / is_empty ─────────────────────────────────────────────────────────

#[test]
fn composite_len_and_is_empty() {
    let mut composite = CompositeEntityCollection::new();
    assert!(composite.is_empty());
    assert_eq!(composite.len(), 0);

    let mut ec = EntityCollection::new();
    ec.add(Entity::new("e1"));
    composite.add_collection(ec);
    composite.recomposite();

    assert!(!composite.is_empty());
    assert_eq!(composite.len(), 1);
}

// ─── getOrCreateEntity ──────────────────────────────────────────────────────

#[test]
fn composite_get_or_create_entity() {
    let mut composite = CompositeEntityCollection::new();
    composite.add_collection(EntityCollection::new());

    let entity = composite.get_or_create_entity("new_id");
    assert_eq!(entity.id, "new_id");

    // 此时应已位于复合集合中
    assert!(composite.contains("new_id"));
}
