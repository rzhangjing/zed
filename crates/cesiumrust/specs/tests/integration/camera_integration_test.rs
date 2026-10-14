use bevy::prelude::*;
use cesium_bevy_render::{
    CesiumCamera, CesiumCameraPlugin, FlyToRequest,
    camera::CameraInputState,
};
use cesium_camera::Camera;
use cesium_scene_mode::SceneMode;

use super::create_test_app;

#[test]
fn test_camera_plugin_registers() {
    let mut app = create_test_app();
    app.add_plugins(CesiumCameraPlugin);
}

#[test]
fn test_camera_input_state_resource_initialized() {
    let mut app = create_test_app();
    app.add_plugins(CesiumCameraPlugin);

    let state = app.world().get_resource::<CameraInputState>();
    assert!(state.is_some());
    let state = state.unwrap();
    assert!(!state.left_mouse_down);
    assert!(!state.right_mouse_down);
    assert!(!state.middle_mouse_down);
    assert!(state.last_mouse_pos.is_none());
}

#[test]
fn test_cesium_camera_component_creation() {
    let mut app = create_test_app();
    app.add_plugins(CesiumCameraPlugin);

    let camera = Camera::default_camera();
    let cc = CesiumCamera::new(camera.clone(), SceneMode::Scene3D);

    let entity = app.world_mut().spawn(cc).id();

    let cc = app.world().get::<CesiumCamera>(entity);
    assert!(cc.is_some());
    assert_eq!(cc.unwrap().scene_mode, SceneMode::Scene3D);
}

#[test]
fn test_camera_scene_modes() {
    let mut app = create_test_app();
    app.add_plugins(CesiumCameraPlugin);

    let modes = vec![SceneMode::Scene2D, SceneMode::Scene3D, SceneMode::ColumbusView, SceneMode::Morphing];
    for mode in modes {
        let cc = CesiumCamera::new(Camera::default_camera(), mode);
        let entity = app.world_mut().spawn(cc).id();

        let cc = app.world().get::<CesiumCamera>(entity);
        assert!(cc.is_some());
        assert_eq!(cc.unwrap().scene_mode, mode);
    }
}

#[test]
fn test_fly_to_event_system() {
    use cesium_geospatial::Cartographic;

    let mut app = create_test_app();
    app.add_plugins(CesiumCameraPlugin);

    let mut events = app.world_mut().resource_mut::<Events<FlyToRequest>>();
    events.send(FlyToRequest {
        destination: Cartographic::from_degrees(-75.0, 40.0, 1000.0),
        duration_secs: 2.0,
    });

    let events = app.world().resource::<Events<FlyToRequest>>();
    let mut reader = events.get_reader();
    let drained: Vec<_> = reader.read(events).collect();
    assert_eq!(drained.len(), 1);
    assert!((drained[0].destination.longitude - (-75.0_f64).to_radians()).abs() < 1e-10);
    assert!((drained[0].duration_secs - 2.0).abs() < 1e-10);
}

#[test]
fn test_camera_default_values() {
    let cc = CesiumCamera::default();
    assert_eq!(cc.scene_mode, SceneMode::Scene3D);
    assert!(cc.enable_collision_detection);
    assert!((cc.minimum_zoom_distance - 100.0).abs() < 1e-10);
    assert!((cc.maximum_zoom_distance - 20_000_000.0).abs() < 1e-10);
}
