//! Headless proofs for the bridge: the sample project draws, the bytes are a real image, a pan and
//! a zoom move the camera, a command list is there, and GeoJSON loads.
//!
//! The first test writes `target/rustgis-zed-poc.png` so a person can look at what the window will
//! show without opening a window.

use std::path::PathBuf;

use rustgis_zed::{MapEngine, frame_to_png};

/// Where the proof image goes: the workspace's `target/`, so `cargo clean` removes it with the rest
/// of the build output.
fn poc_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // crates/
    path.pop(); // the workspace root
    path.push("target");
    path.push(name);
    path
}

#[test]
fn the_sample_project_renders_a_non_empty_png() {
    let mut engine = MapEngine::new();

    let png = engine
        .render_png(800, 600)
        .expect("the sample project should export");
    assert!(!png.is_empty(), "render_png returned no bytes");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "the bytes are not a PNG");

    let written = poc_path("rustgis-zed-poc.png");
    if let Some(parent) = written.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&written, &png).expect("the proof image should be writable");
    assert!(written.is_file(), "{} was not written", written.display());
}

#[test]
fn a_frame_is_straight_rgba_of_the_requested_size() {
    let mut engine = MapEngine::new();

    let frame = engine
        .render_rgba(800, 600)
        .expect("the sample project should render");
    assert_eq!((frame.width, frame.height), (800, 600));
    assert_eq!(frame.rgba.len(), 800 * 600 * 4);
    assert!(!frame.is_blank(), "the frame is fully transparent");

    // The sample project paints an ocean basemap, so the top-left pixel is opaque.
    assert_eq!(frame.rgba[3], 255, "the ocean should be opaque");

    // Re-encoding the same pixels as PNG must work too (this is the path the window uses when it
    // wants bytes rather than an image surface).
    let png = frame_to_png(&frame).expect("a frame should encode as PNG");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
}

#[test]
fn the_window_defaults_to_1200x800_with_four_sample_layers() {
    let engine = MapEngine::new();
    assert_eq!(engine.size(), (1200, 800));

    let layers = engine.layers();
    // The list is top of the drawing order first: the renderer draws the *last* layer first, so
    // "Countries" (added first) is drawn at the bottom and "Cities" on top.
    let names: Vec<&str> = layers.iter().map(|layer| layer.name.as_str()).collect();
    assert_eq!(names, ["Cities", "Rivers", "Lakes", "Countries"]);
    assert!(layers.iter().all(|layer| layer.features > 0));
    assert!(engine.camera_status().scale > 0.0);
}

#[test]
fn a_pan_moves_the_camera_and_a_zoom_narrows_the_view() {
    let mut engine = MapEngine::new();
    let before = engine.camera_status();

    engine.pan(100.0, 0.0);
    let panned = engine.camera_status();
    assert!(
        (panned.center_lon - before.center_lon).abs() > 1.0,
        "a 100px pan should move the centre: {} -> {}",
        before.center_lon,
        panned.center_lon
    );
    assert!((panned.scale - before.scale).abs() < 1.0, "a pan keeps the scale");

    engine.zoom_at(600.0, 400.0, 2.0);
    let zoomed = engine.camera_status();
    assert!(
        zoomed.scale < panned.scale,
        "zooming in should reduce the scale denominator: {} -> {}",
        panned.scale,
        zoomed.scale
    );
}

#[test]
fn geojson_bytes_become_a_layer() {
    let mut engine = MapEngine::new();
    let before = engine.layers().len();

    let geojson = br#"{"type":"FeatureCollection","features":[
        {"type":"Feature","properties":{"name":"A"},"geometry":{"type":"Point","coordinates":[10.0,20.0]}},
        {"type":"Feature","properties":{"name":"B"},"geometry":{"type":"Point","coordinates":[11.0,21.0]}}
    ]}"#;
    let id = engine
        .load_geojson_named(geojson, "Custom")
        .expect("two points should load");

    let layers = engine.layers();
    assert_eq!(layers.len(), before + 1);
    let added = layers
        .iter()
        .find(|layer| layer.id == id)
        .expect("the new layer is on the map");
    assert_eq!(added.name, "Custom");
    assert_eq!(added.features, 2);

    // Garbage is refused rather than turning into an empty layer.
    assert!(engine.load_geojson(b"not geojson").is_err());
    assert_eq!(engine.layers().len(), before + 1);
}

#[test]
fn the_registry_has_commands_and_they_run() {
    let mut engine = MapEngine::new();

    let commands = engine.commands();
    assert!(commands.len() > 100, "{} commands is too few", commands.len());
    assert!(
        commands.iter().any(|command| command.id == "map.zoomIn"),
        "map.zoomIn is missing from the registry"
    );

    // A navigation command through `Session::run`: the zoom actually changes the camera.
    let scale_before = engine.camera_status().scale;
    engine
        .command("map.zoomIn", serde_json::json!({}))
        .expect("map.zoomIn should run");
    assert!(
        engine.camera_status().scale < scale_before,
        "map.zoomIn did not zoom in: {} -> {}",
        scale_before,
        engine.camera_status().scale
    );

    // An unknown id is an error, not a panic.
    assert!(engine.command("no.such.command", serde_json::json!({})).is_err());
}

#[test]
fn every_layer_from_l0_to_l5_reports_itself() {
    let engine = MapEngine::new();
    let capabilities = engine.capabilities();
    for layer in ["L0", "L1", "L2", "L3", "L4", "L5"] {
        assert!(
            capabilities.iter().any(|c| c.layer == layer),
            "{layer} is not reported"
        );
    }
    assert!(capabilities.iter().all(|c| c.available()));
}
