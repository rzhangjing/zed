//! Scene/ImageryLayerSpec, ImageryLayerCollectionSpec → Rust integration tests

use cesium_imagery::{ImageryLayerCollection, ImageryState, SplitDirection, AlphaBlendingMode};

// === ImageryState ===

#[test]
fn test_imagery_state_default() {
    let state = ImageryState::default();
    assert_eq!(state, ImageryState::Unloaded);
}

#[test]
fn test_imagery_state_variants() {
    assert_ne!(ImageryState::Unloaded, ImageryState::Transitioning);
    assert_ne!(ImageryState::Transitioning, ImageryState::Ready);
    assert_ne!(ImageryState::Ready, ImageryState::Failed);
}

// === SplitDirection ===

#[test]
fn test_split_direction_default() {
    let dir = SplitDirection::default();
    assert_eq!(dir, SplitDirection::None);
}

#[test]
fn test_split_direction_values() {
    assert_eq!(SplitDirection::Left as i8, -1);
    assert_eq!(SplitDirection::None as i8, 0);
    assert_eq!(SplitDirection::Right as i8, 1);
}

// === AlphaBlendingMode ===

#[test]
fn test_alpha_blending_mode_default() {
    let mode = AlphaBlendingMode::default();
    assert_eq!(mode, AlphaBlendingMode::Standard);
}

#[test]
fn test_alpha_blending_mode_variants() {
    assert_ne!(AlphaBlendingMode::Standard, AlphaBlendingMode::Additive);
    assert_ne!(AlphaBlendingMode::Additive, AlphaBlendingMode::Multiplicative);
}

// === ImageryLayerCollection ===

#[test]
fn test_imagery_layer_collection_new() {
    let collection = ImageryLayerCollection::new();
    assert_eq!(collection.len(), 0);
    assert!(collection.is_empty());
}
