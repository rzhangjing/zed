//! `Help > RustGIS`: the RustGIS map in a window of its own.
//!
//! The map is drawn by `crates/rustgis-zed` (`rustgis_zed::MapEngine`), which wraps the copied
//! RustGIS 0.6.0 layers in `crates/rustgis` (L0 geom, L1 data, L2 map/tiles/analysis/keychain,
//! L3 render/plugins/geolibre, L4 engine, L5 assistant). Zed depends on that adapter crate alone:
//! no `rustgis-*` crate is named here, so the copied tree's layering (and its "no egui below L6"
//! rule) stays intact.
//!
//! ## Threading
//!
//! `MapEngine` owns a `rustgis_engine::Session`, which is not `Send`, and a 1200×800 frame takes
//! tens of milliseconds. The engine therefore lives on its own thread
//! (`rustgis_zed::spawn_engine`) and this window only sends it [`EngineCommand`]s and polls
//! [`EngineEvent`]s every 16 ms; **no `Session` call ever runs on the UI thread**. That is the same
//! shape as the cesiumrust window (`zed::cesium`), which also owns a renderer thread and only
//! shows the newest frame.
//!
//! ## Input
//!
//! Dragging with the left button pans (`Camera::pan_px`), the wheel zooms about the pointer
//! (`Camera::zoom_at`), and the three buttons in the status bar run RustGIS commands through
//! `Session::run` (`map.zoomIn`, `map.zoomOut`, `map.zoomWorld`), which is also the proof that L4
//! dispatch works from this window.
//!
//! ## Startup hooks
//!
//! `ZED_RUSTGIS_OPEN=1` opens the window as Zed starts (the menu item without the menu), and
//! `ZED_RUSTGIS_LOAD=<path.geojson>` adds a layer from that file once the engine is up. Both
//! exist because verifying this window — that it opens, draws the sample project, and loads a
//! file — means looking at it on the machine, and clicking through `Help` is not what such a
//! check is testing. They mirror `ZED_CESIUMRUST_OPEN` / `ZED_MODELTEST_OPEN` / `ZED_DAMENG_OPEN`.

use std::{
    sync::{Arc, mpsc::Sender},
    time::Duration,
};

use gpui::{
    App, AppContext as _, Context, FocusHandle, Focusable, ImageSource, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Point, Render, RenderImage,
    ScrollDelta, ScrollWheelEvent, SharedString, Size, TitlebarOptions, Window, WindowBounds,
    WindowKind, WindowOptions, div, img, px, size,
};
use release_channel::ReleaseChannel;
use smallvec::SmallVec;
use theme::ActiveTheme as _;
use ui::{Button, ButtonStyle, Color, Label, LabelSize, prelude::*};

use rustgis_zed::{CameraStatus, EngineCommand, EngineEvent, EngineHandle, spawn_engine};

/// `ZED_RUSTGIS_OPEN` — `1`/anything but `0` opens the window as Zed starts.
pub const ENV_OPEN: &str = "ZED_RUSTGIS_OPEN";
/// `ZED_RUSTGIS_LOAD` — a GeoJSON file to add as a layer once the window is up.
pub const ENV_LOAD: &str = "ZED_RUSTGIS_LOAD";

/// The window opens at the frame size the adapter's sample project is designed for.
const WINDOW_WIDTH: f32 = 1200.0;
const WINDOW_HEIGHT: f32 = 800.0;

/// How often the window looks for engine events. The engine thread renders serially, so this is a
/// drain rate, not a frame rate.
const POLL_INTERVAL: Duration = Duration::from_millis(16);
/// One wheel notch zooms by this factor.
const ZOOM_PER_TICK: f64 = 1.15;
/// `ScrollDelta::Pixels` per wheel notch, for trackpads that report pixels.
const PIXELS_PER_ZOOM_TICK: f32 = 50.0;

/// The window's root view: the newest frame the engine drew, and what the status bar shows.
pub struct RustGisView {
    engine: EngineHandle,
    image: Option<Arc<RenderImage>>,
    camera: Option<CameraStatus>,
    /// How many layers the map has, as of the last status event.
    layers: usize,
    /// The last error or notice, shown in the status bar until the next one.
    message: Option<SharedString>,
    /// The frame size last asked for, so a repaint does not queue another render at the same size.
    requested: Option<(u32, u32)>,

    /// Where the drag in progress started, in this window's logical pixels.
    drag_from: Option<Point<Pixels>>,
    /// Whether the event-draining task has been started (once per view, not once per paint).
    polling_started: bool,
    focus_handle: FocusHandle,
}

impl RustGisView {
    fn new(cx: &mut Context<Self>) -> Self {
        let engine = spawn_engine();
        // The first frame is asked for as soon as the window knows its size (`Render`); asking
        // here too covers the case where it is opened with its default bounds already in place.
        let _ = engine.send(EngineCommand::Render {
            width: WINDOW_WIDTH as u32,
            height: WINDOW_HEIGHT as u32,
        });

        if let Ok(path) = std::env::var(ENV_LOAD) {
            if path.trim().is_empty() || path.trim() == "0" {
                log::info!("[rustgis] {ENV_LOAD} is empty, not loading a layer");
            } else {
                log::info!("[rustgis] loading a layer from {path} at startup ({ENV_LOAD})");
                load_file_in_background(engine.sender(), path);
            }
        }

        Self {
            engine,
            image: None,
            camera: None,
            layers: 0,
            message: None,
            requested: None,
            drag_from: None,
            polling_started: false,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Drain what the engine thread has sent. Called from the poll task, never from `Render`.
    fn poll(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        // Bounded: a long gesture can queue more frames than this, and the newest is what matters.
        for _ in 0..8 {
            let Some(event) = self.engine.try_recv() else {
                break;
            };
            match event {
                EngineEvent::Ready => {
                    log::info!("[rustgis] the engine thread is ready");
                }
                EngineEvent::Frame(frame) => {
                    self.image = Some(frame_to_render_image(&frame));
                    changed = true;
                }
                EngineEvent::Status {
                    camera,
                    layers,
                    notices,
                } => {
                    self.camera = Some(camera);
                    self.layers = layers;
                    if let Some(notice) = notices.into_iter().last() {
                        self.message = Some(SharedString::from(notice.text));
                    }
                    changed = true;
                }
                EngineEvent::Failed { command, error } => {
                    log::error!("[rustgis] {command} failed: {error}");
                    self.message = Some(SharedString::from(error));
                    changed = true;
                }
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn pan(&mut self, dx: f32, dy: f32) {
        // A pointer delta and a frame pixel are the same unit (`px()` and `viewport_size()` agree),
        // so the gesture maps onto the camera one to one.
        let _ = self.engine.send(EngineCommand::Pan {
            dx: dx as f64,
            dy: dy as f64,
        });
    }

    fn zoom(&mut self, at: Point<Pixels>, ticks: f32) {
        if ticks == 0.0 {
            return;
        }
        let _ = self.engine.send(EngineCommand::Zoom {
            x: at.x.as_f32() as f64,
            y: at.y.as_f32() as f64,
            factor: ZOOM_PER_TICK.powf(ticks as f64),
        });
    }

    /// Run a RustGIS command on the engine thread; a failure shows in the status bar.
    fn run_command(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let _ = self.engine.send(EngineCommand::Command {
            id: id.to_string(),
            params: serde_json::json!({}),
        });
        self.message = Some(SharedString::from(id));
        cx.notify();
    }

    fn status_text(&self) -> SharedString {
        match &self.camera {
            Some(camera) => SharedString::from(format!(
                "centre {}   {}   {} layers",
                camera.center_text(),
                camera.scale_text(),
                self.layers
            )),
            None => SharedString::from("starting the RustGIS engine…"),
        }
    }
}

impl Focusable for RustGisView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RustGisView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.polling_started {
            self.polling_started = true;
            cx.spawn(async move |view, cx| loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                if view.update(cx, |view, cx| view.poll(cx)).is_err() {
                    return;
                }
            })
            .detach();
        }

        // The map takes every pixel the status bar does not: the bar is laid out first
        // (`flex_none`) and the map gets the rest (`flex_1`), so the bar can never be pushed out
        // of the window by a wrong height calculation. `px()` and `viewport_size()` are the same
        // unit, so the frame is simply that many pixels wide and tall — `scale_factor` is for
        // rasterisation density, not layout, and must not multiply it.
        let viewport: Size<Pixels> = window.viewport_size();
        let want = (
            viewport.width.as_f32().max(1.0).round() as u32,
            viewport.height.as_f32().max(1.0).round() as u32,
        );
        if self.requested != Some(want) {
            self.requested = Some(want);
            let _ = self.engine.send(EngineCommand::Render {
                width: want.0,
                height: want.1,
            });
        }

        let image = self.image.clone();
        let status = self.status_text();
        let message = self.message.clone();
        let hint = if self.drag_from.is_some() {
            "dragging"
        } else {
            "drag to pan · wheel to zoom"
        };

        v_flex()
            .id("rustgis")
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .track_focus(&self.focus_handle)
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .relative()
                    .overflow_hidden()
                    .bg(cx.theme().colors().editor_background)
                    .child(match image {
                        Some(image) => img(ImageSource::Render(image))
                            .object_fit(ObjectFit::Fill)
                            .size_full()
                            .into_any_element(),
                        None => div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(cx.theme().colors().text_muted)
                            .child("RustGIS — drawing the sample project…")
                            .into_any_element(),
                    })
                    // The status bar is an overlay inside the map area, at its bottom edge: that
                    // element is laid out for certain, and an overlay keeps the frame's pixels
                    // aligned one to one with pointer positions.
                    .child(
                        h_flex()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .items_center()
                            .gap_3()
                            .px_3()
                            .py_1()
                            .bg(cx.theme().colors().panel_background)
                            .border_t_1()
                            .border_color(cx.theme().colors().border)
                            .child(Label::new(status).size(LabelSize::Small).color(Color::Muted))
                            .child(div().flex_1())
                            .children(message.map(|message| {
                                Label::new(message)
                                    .size(LabelSize::Small)
                                    .color(Color::Accent)
                            }))
                            .child(
                                Button::new("rustgis-zoom-out", "Zoom Out")
                                    .style(ButtonStyle::OutlinedGhost)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.run_command("map.zoomOut", cx)
                                    })),
                            )
                            .child(
                                Button::new("rustgis-zoom-in", "Zoom In")
                                    .style(ButtonStyle::OutlinedGhost)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.run_command("map.zoomIn", cx)
                                    })),
                            )
                            .child(
                                Button::new("rustgis-world", "World")
                                    .style(ButtonStyle::OutlinedGhost)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.run_command("map.zoomWorld", cx)
                                    })),
                            )
                            .child(
                                Label::new(hint)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Hidden),
                            ),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.drag_from = Some(event.position);
                            this.focus_handle.focus(window, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                        if event.pressed_button != Some(MouseButton::Left) {
                            this.drag_from = None;
                            return;
                        }
                        let Some(from) = this.drag_from else {
                            return;
                        };
                        let dx = event.position.x.as_f32() - from.x.as_f32();
                        let dy = event.position.y.as_f32() - from.y.as_f32();
                        this.drag_from = Some(event.position);
                        if dx != 0.0 || dy != 0.0 {
                            this.pan(dx, dy);
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _window, _cx| {
                            this.drag_from = None;
                        }),
                    )
                    .on_scroll_wheel(cx.listener(
                        |this, event: &ScrollWheelEvent, _window, cx| {
                            let ticks = match event.delta {
                                ScrollDelta::Lines(lines) => lines.y,
                                ScrollDelta::Pixels(pixels) => {
                                    f32::from(pixels.y) / PIXELS_PER_ZOOM_TICK
                                }
                            }
                            .clamp(-3.0, 3.0);
                            let at = event.position;
                            this.zoom(at, ticks);
                            cx.stop_propagation();
                        },
                    )),
            )
    }
}

/// Opens the window, or focuses the one that is already open.
///
/// Rendering needs `theme::GlobalTheme` (`cx.theme()` panics without it). The menu path always runs
/// long after the theme is registered, but the `ZED_RUSTGIS_OPEN` startup path can run before
/// that, so wait for it rather than opening a window that cannot draw. Mirrors `zed::model_test`.
pub fn open(cx: &mut App) {
    if cx.try_global::<theme::GlobalTheme>().is_none() {
        log::info!("[rustgis] waiting for the theme before opening the window");
        cx.spawn(async move |cx| {
            for _ in 0..300 {
                if cx.update(|cx| cx.try_global::<theme::GlobalTheme>().is_some()) {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
            cx.update(show_window);
        })
        .detach();
        return;
    }

    show_window(cx);
}

/// Opens the window, or focuses it if it is already open.
fn show_window(cx: &mut App) {
    if let Some(existing) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<RustGisView>())
    {
        if let Err(error) = existing.update(cx, |_, window, _| window.activate_window()) {
            log::warn!("[rustgis] could not focus the RustGIS window: {error}");
        }
        return;
    }

    let window_size = size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT));
    let opened = cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("RustGIS".into()),
                // The system title bar; the window draws no chrome of its own. Same shape as
                // `zed::admin_demo` and `zed::model_test`.
                appears_transparent: false,
                traffic_light_position: None,
            }),
            window_bounds: Some(WindowBounds::centered(window_size, cx)),
            kind: WindowKind::Normal,
            app_id: Some(ReleaseChannel::global(cx).app_id().to_owned()),
            ..Default::default()
        },
        |_window, cx| cx.new(RustGisView::new).into(),
    );

    match opened {
        Ok(_) => log::info!("[rustgis] window opened"),
        Err(error) => log::error!("[rustgis] could not open the window: {error}"),
    }
}

/// Registers the startup override, if any. Called once from `zed::init`.
pub fn init(cx: &mut App) {
    if std::env::var(ENV_OPEN).is_ok_and(|value| value != "0") {
        log::info!("[rustgis] opening the window at startup ({ENV_OPEN})");
        cx.defer(open);
    }
}

/// Read `path` off the UI thread and hand its bytes to the engine thread.
fn load_file_in_background(commands: Sender<EngineCommand>, path: String) {
    std::thread::Builder::new()
        .name("rustgis-zed-load".to_string())
        .spawn(move || match std::fs::read(&path) {
            Ok(bytes) => {
                let name = std::path::Path::new(&path)
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Layer".to_string());
                if commands
                    .send(EngineCommand::LoadGeoJson { name, bytes })
                    .is_err()
                {
                    log::warn!("[rustgis] the engine thread stopped before {path} was loaded");
                }
            }
            Err(error) => log::error!("[rustgis] could not read {path}: {error}"),
        })
        .ok();
}

/// A frame drawn by `tiny-skia` becomes a GPUI image.
///
/// GPUI's atlas stores image bytes as **BGRA** (see `gpui_wgpu`'s `swizzle_upload_data`), while
/// `rustgis_zed::Frame` is straight RGBA, so red and blue are swapped here — once, in this one
/// place. Same rule as `zed::cesium::frame_to_render_image`.
fn frame_to_render_image(frame: &rustgis_zed::Frame) -> Arc<RenderImage> {
    let mut bytes = frame.rgba.clone();
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_raw(frame.width, frame.height, bytes)
            // A size mismatch cannot be drawn; fall back to a black frame rather than panic.
            .unwrap_or_else(|| image::ImageBuffer::new(frame.width, frame.height));
    Arc::new(RenderImage::new(SmallVec::from_elem(
        image::Frame::new(buffer),
        1,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame's channels reach GPUI in BGRA, with alpha untouched.
    #[test]
    fn frames_map_to_render_images_with_swapped_channels() {
        let frame = rustgis_zed::Frame {
            width: 2,
            height: 1,
            rgba: vec![10, 20, 30, 40, 50, 60, 70, 80],
        };
        let image = frame_to_render_image(&frame);

        assert_eq!(image.frame_count(), 1);
        let frame_size = image.size(0);
        assert_eq!((frame_size.width.0, frame_size.height.0), (2, 1));
        assert_eq!(
            image.as_bytes(0),
            Some([30, 20, 10, 40, 70, 60, 50, 80].as_slice())
        );
    }
}
