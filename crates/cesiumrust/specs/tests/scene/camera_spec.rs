//! Scene/CameraSpec → Rust integration tests

use cesium_camera::{SceneMode, EasingFunction};

// === SceneMode ===

#[test]
fn test_scene_mode_default() {
    let mode = SceneMode::default();
    assert_eq!(mode, SceneMode::Scene3D);
}

#[test]
fn test_scene_mode_variants() {
    assert_ne!(SceneMode::Scene2D, SceneMode::Scene3D);
    assert_ne!(SceneMode::Scene3D, SceneMode::ColumbusView);
    assert_ne!(SceneMode::ColumbusView, SceneMode::Morphing);
}

// === EasingFunction ===

#[test]
fn test_easing_linear() {
    let f = EasingFunction::Linear;
    assert!((f.evaluate(0.0) - 0.0).abs() < 1e-10);
    assert!((f.evaluate(0.5) - 0.5).abs() < 1e-10);
    assert!((f.evaluate(1.0) - 1.0).abs() < 1e-10);
}

#[test]
fn test_easing_quadratic_in() {
    let f = EasingFunction::QuadraticIn;
    assert!((f.evaluate(0.0) - 0.0).abs() < 1e-10);
    assert!((f.evaluate(0.5) - 0.25).abs() < 1e-10);
    assert!((f.evaluate(1.0) - 1.0).abs() < 1e-10);
}

#[test]
fn test_easing_sinusoidal() {
    let f = EasingFunction::SinusoidalInOut;
    assert!((f.evaluate(0.0) - 0.0).abs() < 1e-10);
    assert!((f.evaluate(0.5) - 0.5).abs() < 1e-10);
    assert!((f.evaluate(1.0) - 1.0).abs() < 1e-10);
}

#[test]
fn test_easing_clamps_input() {
    let f = EasingFunction::Linear;
    assert!((f.evaluate(-0.5) - 0.0).abs() < 1e-10);
    assert!((f.evaluate(1.5) - 1.0).abs() < 1e-10);
}
