//! Scene/ImageryLayerCollection → Rust 集成测试。
//!
//! 参考 CesiumJS：
//! - Scene/ImageryLayerCollection
//!
//! A 类测试：add/add_at/remove/remove_at/get/raise/lower/raise_to_top/
//! lower_to_bottom/index_of/visible_layers/compute_blended_alpha.

use cesium_imagery::{ImageryLayer, ImageryLayerCollection};
use cesium_geospatial::rectangle::Rectangle;

fn test_layer() -> ImageryLayer {
    ImageryLayer::new(0, Rectangle::MAX_VALUE)
}

// === add/get ===

#[test]
fn collection_add_and_get() {
    let mut c = ImageryLayerCollection::new();
    let id = c.add(test_layer());
    assert_eq!(c.len(), 1);
    assert!(!c.is_empty());
    assert!(c.get(id).is_some());
}

#[test]
fn collection_add_assigns_unique_ids() {
    let mut c = ImageryLayerCollection::new();
    let id1 = c.add(test_layer());
    let id2 = c.add(test_layer());
    let id3 = c.add(test_layer());
    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_eq!(c.len(), 3);
}

#[test]
fn collection_add_at_index() {
    let mut c = ImageryLayerCollection::new();
    let id1 = c.add(test_layer());
    let id2 = c.add(test_layer());
    let id3 = c.add_at(test_layer(), 1); // 插入中间
    assert_eq!(c.index_of(id1), Some(0));
    assert_eq!(c.index_of(id3), Some(1));
    assert_eq!(c.index_of(id2), Some(2));
}

#[test]
fn collection_add_at_clamped() {
    let mut c = ImageryLayerCollection::new();
    let id1 = c.add(test_layer());
    // 索引超出长度 → 追加到末尾
    let id2 = c.add_at(test_layer(), 100);
    assert_eq!(c.index_of(id2), Some(1));
    assert_eq!(c.index_of(id1), Some(0));
}

// === remove ===

#[test]
fn collection_remove_by_id() {
    let mut c = ImageryLayerCollection::new();
    let id = c.add(test_layer());
    let removed = c.remove(id);
    assert!(removed.is_some());
    assert_eq!(c.len(), 0);
}

#[test]
fn collection_remove_nonexistent() {
    let mut c = ImageryLayerCollection::new();
    c.add(test_layer());
    assert!(c.remove(999).is_none());
    assert_eq!(c.len(), 1);
}

#[test]
fn collection_remove_at_index() {
    let mut c = ImageryLayerCollection::new();
    c.add(test_layer());
    c.add(test_layer());
    let removed = c.remove_at(0);
    assert!(removed.is_some());
    assert_eq!(c.len(), 1);
}

#[test]
fn collection_remove_at_out_of_bounds() {
    let mut c = ImageryLayerCollection::new();
    c.add(test_layer());
    assert!(c.remove_at(5).is_none());
}

// === ordering ===

#[test]
fn collection_raise() {
    let mut c = ImageryLayerCollection::new();
    let id1 = c.add(test_layer());
    let id2 = c.add(test_layer());
    c.raise(id1);
    assert_eq!(c.index_of(id1), Some(1));
    assert_eq!(c.index_of(id2), Some(0));
}

#[test]
fn collection_raise_top_no_op() {
    let mut c = ImageryLayerCollection::new();
    let id1 = c.add(test_layer());
    c.add(test_layer());
    // 提升已在顶层的图层
    c.raise(id1); // id1 在 0，移到 1
    c.raise(id1); // id1 在 1（顶层），无操作
    assert_eq!(c.index_of(id1), Some(1));
}

#[test]
fn collection_lower() {
    let mut c = ImageryLayerCollection::new();
    let id1 = c.add(test_layer());
    let id2 = c.add(test_layer());
    c.lower(id2);
    assert_eq!(c.index_of(id2), Some(0));
    assert_eq!(c.index_of(id1), Some(1));
}

#[test]
fn collection_lower_bottom_no_op() {
    let mut c = ImageryLayerCollection::new();
    let id1 = c.add(test_layer());
    c.add(test_layer());
    // 下移已在底层的图层
    c.lower(id1); // id1 在 0，无操作
    assert_eq!(c.index_of(id1), Some(0));
}

#[test]
fn collection_raise_to_top() {
    let mut c = ImageryLayerCollection::new();
    let id1 = c.add(test_layer());
    c.add(test_layer());
    c.add(test_layer());
    c.raise_to_top(id1);
    assert_eq!(c.index_of(id1), Some(2));
}

#[test]
fn collection_lower_to_bottom() {
    let mut c = ImageryLayerCollection::new();
    c.add(test_layer());
    c.add(test_layer());
    let id3 = c.add(test_layer());
    c.lower_to_bottom(id3);
    assert_eq!(c.index_of(id3), Some(0));
}

// === get_at ===

#[test]
fn collection_get_at_index() {
    let mut c = ImageryLayerCollection::new();
    let id1 = c.add(test_layer());
    let id2 = c.add(test_layer());
    assert_eq!(c.get_at(0).unwrap().id, id1);
    assert_eq!(c.get_at(1).unwrap().id, id2);
    assert!(c.get_at(2).is_none());
}

// === visible_layers ===

#[test]
fn collection_visible_layers() {
    let mut c = ImageryLayerCollection::new();
    c.add(test_layer().with_show(true));
    c.add(test_layer().with_show(false));
    c.add(test_layer().with_show(true));
    assert_eq!(c.visible_layers().count(), 2);
}

// === compute_blended_alpha ===

#[test]
fn blended_alpha_single_opaque() {
    let mut c = ImageryLayerCollection::new();
    c.add(test_layer());
    let result = c.compute_blended_alpha(&[1.0]);
    assert!((result - 1.0).abs() < 1e-10);
}

#[test]
fn blended_alpha_two_semi_transparent() {
    let mut c = ImageryLayerCollection::new();
    c.add(test_layer());
    c.add(test_layer());
    // 0.5 + 0.5 * (1 - 0.5) = 0.75
    let result = c.compute_blended_alpha(&[0.5, 0.5]);
    assert!((result - 0.75).abs() < 1e-10);
}

#[test]
fn blended_alpha_hidden_layer_skipped() {
    let mut c = ImageryLayerCollection::new();
    c.add(test_layer().with_show(false));
    c.add(test_layer());
    // 隐藏图层被跳过，仅第二个参与
    let result = c.compute_blended_alpha(&[1.0, 0.5]);
    assert!((result - 0.5).abs() < 1e-10);
}

#[test]
fn blended_alpha_three_layers() {
    let mut c = ImageryLayerCollection::new();
    c.add(test_layer());
    c.add(test_layer());
    c.add(test_layer());
    // a1=0.5, a2=0.5, a3=0.5
    // result = 0.5 + 0.5*0.5 + 0.5*0.25 = 0.5 + 0.25 + 0.125 = 0.875
    let result = c.compute_blended_alpha(&[0.5, 0.5, 0.5]);
    assert!((result - 0.875).abs() < 1e-10);
}
