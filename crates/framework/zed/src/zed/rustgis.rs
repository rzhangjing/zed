//! `Help > RustGIS`: the RustGIS map in a window of its own.
//!
//! The map is drawn by `crates/rustgis-zed` (`rustgis_zed::MapEngine`), which wraps the copied
//! RustGIS 0.6.0 layers in `crates/rustgis` (L0 geom, L1 data, L2 map/tiles/analysis/keychain,
//! L3 render/plugins/geolibre, L4 engine, L5 assistant). Zed depends on that adapter crate alone:
//! no `rustgis-*` crate is named here, so the copied tree's layering (and its "no egui below L6"
//! rule) stays intact.
//!
//! ## Threading and frame pacing
//!
//! `MapEngine` owns a `rustgis_engine::Session`, which is not `Send`, and a full 1200×800 frame
//! costs hundreds of milliseconds even before labels. The engine therefore lives on its own
//! thread (`rustgis_zed::spawn_engine`) and this window only sends it [`EngineCommand`]s and polls
//! [`EngineEvent`]s every 16 ms; **no `Session` call ever runs on the UI thread**.
//!
//! Input events are much faster than frames, so they are folded, never queued:
//!
//! * every mouse move adds to `pending_pan` (and wheel notches multiply `pending_zoom`); no
//!   command is sent from an event handler;
//! * the 16 ms poll loop sends **one** [`EngineCommand::Interact`] at most every 33 ms (≈30 fps),
//!   and only when no earlier frame is still in flight — so the engine queue holds at most one
//!   gesture, and a fast drag can never leave a trail of stale views;
//! * a gesture draws a [`Quality::Preview`] frame (half canvas, labels off, no antialiasing); the
//!   worker halves that canvas once if a preview misses 33 ms (a debug build), and sharpens it
//!   again on a fast machine;
//! * 150 ms after the last gesture (or on mouse-up, after a short grace) the window sends one
//!   [`EngineCommand::Settle`], which draws the full-quality frame with labels.
//!
//! ## Input
//!
//! Dragging with the left button pans (`Camera::pan_px`), the wheel zooms about the pointer
//! (`Camera::zoom_at`), the `Zoom Out`/`Zoom In`/`World` buttons and `+`/`-`/arrow keys move the
//! camera directly. **Navigation never calls `Session::run`** (`map.zoomIn`/`map.zoomOut`/
//! `map.zoomWorld` are `view()` commands — no undo snapshot — but they still pay the session's
//! registry/bookkeeping cost, and a command path is for data, not gestures).
//!
//! ## Startup hooks
//!
//! `ZED_RUSTGIS_OPEN=1` opens the window as Zed starts (the menu item without the menu), and
//! `ZED_RUSTGIS_LOAD=<path.geojson>` adds a layer from that file once the engine is up. Both
//! exist because verifying this window — that it opens, draws the sample project, and loads a
//! file — means looking at it on the machine, and clicking through `Help` is not what such a
//! check is testing. They mirror `ZED_MODELTEST_OPEN` / `ZED_DAMENG_OPEN` / `ZED_PLOT_OPEN`.

use std::{
    sync::{Arc, mpsc::SyncSender},
    time::{Duration, Instant},
};

use gpui::{
    App, AppContext as _, Context, FocusHandle, Focusable, ImageSource, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Point, Render,
    RenderImage, ScrollDelta, ScrollWheelEvent, SharedString, Size, TitlebarOptions, Window,
    WindowBounds, WindowKind, WindowOptions, div, img, px, size,
};
use release_channel::ReleaseChannel;
use smallvec::SmallVec;
use theme::ActiveTheme as _;
use ui::{Button, ButtonStyle, Color, Label, LabelSize, prelude::*};

use rustgis_zed::{CameraStatus, EngineCommand, EngineEvent, EngineFrame, EngineHandle, Quality, spawn_engine};

/// `ZED_RUSTGIS_OPEN` — `1`/anything but `0` opens the window as Zed starts.
pub const ENV_OPEN: &str = "ZED_RUSTGIS_OPEN";
/// `ZED_RUSTGIS_LOAD` — a GeoJSON file to add as a layer once the window is up.
pub const ENV_LOAD: &str = "ZED_RUSTGIS_LOAD";

/// The window opens at the client-area size the adapter's sample project is designed for.
const WINDOW_WIDTH: f32 = 1200.0;
const WINDOW_HEIGHT: f32 = 800.0;

/// How often the window looks for engine events. The engine thread renders serially, so this is a
/// drain/scheduling rate, not a frame rate.
const POLL_INTERVAL: Duration = Duration::from_millis(16);
/// At most one interaction frame per this interval (≈30 fps); events in between are folded into
/// the next one.
const INTERACTION_INTERVAL: Duration = Duration::from_millis(33);
/// How long the map must be still before the full-quality frame is drawn.
const IDLE_SETTLE: Duration = Duration::from_millis(150);
/// After mouse-up the settle may come a little sooner: the gesture is over, no more deltas wait.
const MOUSE_UP_SETTLE: Duration = Duration::from_millis(40);
/// One wheel notch zooms by this factor.
const ZOOM_PER_TICK: f64 = 1.15;
/// `ScrollDelta::Pixels` per wheel notch, for trackpads that report pixels.
const PIXELS_PER_ZOOM_TICK: f32 = 50.0;
/// `+`/`-` and the buttons zoom by this factor, as RustGIS's `map.zoomIn`/`map.zoomOut` do.
const KEY_ZOOM_FACTOR: f64 = 2.0;
/// One arrow-key press pans this many screen pixels.
const KEY_PAN_STEP: f64 = 80.0;
/// The bottom bar's height: two rows (status + buttons) with the bar's padding. The map is drawn
/// for the viewport minus exactly this, and the bar is `flex_none`, so neither can be clipped by
/// the other.
const STATUS_BAR_HEIGHT: f32 = 58.0;

/// The window's root view: the newest frame the engine drew, and what the status bar shows.
pub struct RustGisView {
    engine: EngineHandle,
    image: Option<Arc<RenderImage>>,
    /// The quality of the frame on screen (`None` before the first frame).
    image_quality: Option<Quality>,
    camera: Option<CameraStatus>,
    /// How many layers the map has, as of the last status event.
    layers: usize,
    /// The last error or notice, shown in the status bar until the next one.
    message: Option<SharedString>,
    /// The map area the next frame is drawn for: viewport minus the status bar. Written by
    /// `render` and consumed by `poll`.
    want: Option<(u32, u32)>,
    /// The map area the last render command asked for, so a repaint does not queue another frame
    /// at the same size.
    requested: Option<(u32, u32)>,
    /// The real client size last logged, so the native-vs-GPUI delta is reported once per change.
    reported_client: Option<(u32, u32)>,

    /// Where the drag in progress started, in this window's logical pixels.
    drag_from: Option<Point<Pixels>>,
    /// Screen pixels panned since the last interaction frame was sent.
    pending_pan: (f64, f64),
    /// Zoom accumulated since the last interaction frame was sent: anchor x, y, and the product
    /// of every notch's factor.
    pending_zoom: Option<(f64, f64, f64)>,
    /// A `World` click waiting for the one in-flight frame to finish.
    pending_world: bool,
    /// Whether a frame (or a failure) is still on its way: at most one.
    in_flight: bool,
    /// When the last interaction frame was requested (33 ms throttle).
    last_interaction: Option<Instant>,
    /// When the map becomes idle enough to settle to full quality.
    idle_deadline: Option<Instant>,
    /// What the last frame cost and what it was, for the status line.
    last_frame: Option<(Quality, Duration, u32, u32)>,
    /// Whether the event-draining task has been started (once per view, not once per paint).
    polling_started: bool,
    focus_handle: FocusHandle,
}

impl RustGisView {
    fn new(cx: &mut Context<Self>) -> Self {
        let engine = spawn_engine();
        // The first frame is asked for by the poll loop once `render` has told it the real map
        // area (the viewport minus the status bar), so no frame is drawn at a size that would
        // have to be thrown away.
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
            image_quality: None,
            camera: None,
            layers: 0,
            message: None,
            want: None,
            requested: None,
            reported_client: None,
            drag_from: None,
            pending_pan: (0.0, 0.0),
            pending_zoom: None,
            pending_world: false,
            in_flight: false,
            last_interaction: None,
            idle_deadline: None,
            last_frame: None,
            polling_started: false,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Send a command; `true` when the engine took it (and a frame is now in flight).
    fn send(&mut self, command: EngineCommand) -> bool {
        match self.engine.send(command) {
            Ok(()) => {
                self.in_flight = true;
                true
            }
            Err(error) => {
                if error.contains("busy") {
                    // The engine is still drawing the previous frame. The caller retries on the
                    // next poll; nothing is lost because only the newest view matters.
                    log::debug!("[rustgis] engine busy, request folded into the next frame");
                } else {
                    log::error!("[rustgis] {error}");
                    self.message = Some(SharedString::from(error));
                }
                false
            }
        }
    }

    fn map_area(&self) -> Option<(u32, u32)> {
        self.want
    }

    /// A pointer delta since the last sent interaction.
    fn queue_pan(&mut self, dx: f64, dy: f64) {
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        self.pending_pan.0 += dx;
        self.pending_pan.1 += dy;
        self.idle_deadline = Some(Instant::now() + IDLE_SETTLE);
    }

    /// Zoom by `factor` about the screen point (`x`, `y`), folding repeated notches into one.
    fn queue_zoom(&mut self, x: f64, y: f64, factor: f64) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        self.pending_zoom = fold_zoom(self.pending_zoom.take(), x, y, factor);
        self.idle_deadline = Some(Instant::now() + IDLE_SETTLE);
    }

    /// Zoom about the map centre: the buttons and `+`/`-`.
    fn queue_zoom_center(&mut self, factor: f64) {
        if let Some((width, height)) = self.map_area() {
            self.queue_zoom(width as f64 / 2.0, height as f64 / 2.0, factor);
        }
    }

    /// The `World` button: one direct camera move, not `Session::run("map.zoomWorld")`.
    fn queue_world(&mut self) {
        self.pending_pan = (0.0, 0.0);
        self.pending_zoom = None;
        self.pending_world = true;
        self.idle_deadline = Some(Instant::now() + IDLE_SETTLE);
    }

    /// Drain what the engine thread has sent and schedule the next frame. Called from the poll
    /// task, never from `Render`.
    fn poll(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        let mut newest: Option<EngineFrame> = None;
        let mut status: Option<(CameraStatus, usize, Vec<rustgis_zed::Notice>)> = None;
        let mut failure: Option<String> = None;
        let mut terminal = false;
        // Everything already drawn, but only the newest frame is turned into a texture: an
        // intermediate frame would cost a 1–4 MiB conversion to be thrown away at once.
        while let Some(event) = self.engine.try_recv() {
            match event {
                EngineEvent::Ready => {
                    log::info!("[rustgis] the engine thread is ready");
                }
                EngineEvent::Frame(frame) => {
                    newest = Some(frame);
                    terminal = true;
                }
                EngineEvent::Status {
                    camera,
                    layers,
                    notices,
                } => {
                    status = Some((camera, layers, notices));
                    terminal = true;
                }
                EngineEvent::Failed { command, error } => {
                    log::error!("[rustgis] {command} failed: {error}");
                    failure = Some(error);
                    terminal = true;
                }
            }
        }

        if let Some(frame) = newest {
            let convert = Instant::now();
            let (width, height) = (frame.width, frame.height);
            self.image = Some(bgra_to_render_image(frame.bgra, width, height));
            self.image_quality = Some(frame.quality);
            self.last_frame = Some((frame.quality, frame.elapsed, width, height));
            log::info!(
                "[rustgis] UI: {width}x{height} {:?} in {:.1} ms (render {:.1} ms)",
                frame.quality,
                convert.elapsed().as_secs_f64() * 1000.0,
                frame.elapsed.as_secs_f64() * 1000.0
            );
            changed = true;
        }
        if let Some((camera, layers, notices)) = status {
            self.camera = Some(camera);
            self.layers = layers;
            if let Some(notice) = notices.into_iter().last() {
                self.message = Some(SharedString::from(notice.text));
            }
            changed = true;
        }
        if let Some(error) = failure {
            self.message = Some(SharedString::from(error));
            changed = true;
        }
        if terminal {
            self.in_flight = false;
        }

        let now = Instant::now();
        // 1. A resize first: the old frame is the wrong size for the new layout.
        if let Some(area) = self.want
            && self.requested != Some(area)
            && !self.in_flight
        {
            let quality = if self.image.is_some() {
                Quality::PREVIEW
            } else {
                Quality::Full
            };
            if self.send(EngineCommand::Render {
                width: area.0,
                height: area.1,
                quality,
            }) {
                self.requested = Some(area);
                if quality.is_preview() {
                    self.idle_deadline = Some(now + IDLE_SETTLE);
                }
                changed = true;
            }
        }

        // 2. The `World` button (one command, then a settle).
        if self.pending_world && !self.in_flight {
            let Some(area) = self.want else {
                self.pending_world = false;
                return self.finish_poll(changed, cx);
            };
            if self.send(EngineCommand::World {
                width: area.0,
                height: area.1,
            }) {
                self.pending_world = false;
                self.idle_deadline = Some(now + IDLE_SETTLE);
                changed = true;
            }
        }

        // 3. A gesture: at most one in flight, at most one every 33 ms, everything since the last
        // one folded into it.
        let has_gesture = self.pending_pan != (0.0, 0.0) || self.pending_zoom.is_some();
        if interaction_due(has_gesture, self.in_flight, self.last_interaction, now)
            && !self.pending_world
        {
            if let Some(area) = self.want {
                let pan = std::mem::replace(&mut self.pending_pan, (0.0, 0.0));
                let zoom = self.pending_zoom.take();
                if self.send(EngineCommand::Interact {
                    width: area.0,
                    height: area.1,
                    pan,
                    zoom,
                }) {
                    self.last_interaction = Some(now);
                    self.idle_deadline = Some(now + IDLE_SETTLE);
                    changed = true;
                } else {
                    // Fold the gesture back; the next poll retries.
                    self.pending_pan.0 += pan.0;
                    self.pending_pan.1 += pan.1;
                    if let Some((x, y, factor)) = zoom {
                        self.pending_zoom = fold_zoom(self.pending_zoom, x, y, factor);
                    }
                }
            }
        }

        // 4. Idle: settle to a full-quality frame with labels.
        let has_gesture = self.pending_pan != (0.0, 0.0) || self.pending_zoom.is_some();
        if !has_gesture
            && !self.pending_world
            && !self.in_flight
            && self.image_quality.is_some_and(Quality::is_preview)
            && self.idle_deadline.is_some_and(|deadline| now >= deadline)
        {
            if let Some(area) = self.want
                && self.send(EngineCommand::Settle {
                    width: area.0,
                    height: area.1,
                })
            {
                self.idle_deadline = None;
                self.requested = Some(area);
                changed = true;
            }
        }

        self.finish_poll(changed, cx);
    }

    fn finish_poll(&mut self, changed: bool, cx: &mut Context<Self>) {
        if changed {
            cx.notify();
        }
    }

    /// "lat, lon  scale  N layers" plus the frame that is on screen.
    fn status_text(&self) -> SharedString {
        match &self.camera {
            Some(camera) => SharedString::from(format!(
                "{}   {}   {} layers",
                camera.center_text(),
                camera.scale_text(),
                self.layers
            )),
            None => SharedString::from("starting the RustGIS engine…"),
        }
    }

    /// "preview 600×400 · 24 ms" — the measured frame, so a slow frame is visible, not guessed.
    fn frame_text(&self) -> Option<SharedString> {
        self.last_frame.map(|(quality, elapsed, width, height)| {
            SharedString::from(format!(
                "{} {}×{} · {:.0} ms",
                if quality.is_preview() {
                    "preview"
                } else {
                    "full"
                },
                width,
                height,
                elapsed.as_secs_f64() * 1000.0
            ))
        })
    }

    fn hint_text(&self) -> &'static str {
        if self.drag_from.is_some() {
            "dragging · preview"
        } else {
            "drag to pan · wheel to zoom · +/− zoom · arrows pan"
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
            // Keyboard shortcuts work as soon as the window is up, without a click first.
            self.focus_handle.focus(window, cx);
            cx.spawn(async move |view, cx| loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                if view.update(cx, |view, cx| view.poll(cx)).is_err() {
                    return;
                }
            })
            .detach();
        }

        // GPUI's viewport and the window's real client area can disagree for the first frames on
        // Windows (GPUI seeds the viewport from the size `CreateWindowEx` was given, before the
        // title bar and borders are known). `native_client_size` reads the client area with
        // `GetClientRect` and DWM's extended frame bounds; the root is laid out for the smaller of
        // the two, so the bottom row cannot be pushed below the visible client area.
        let viewport = window.viewport_size();
        let (fit_width, fit_height, dwm_frame) = match native_client_size(window) {
            Some((client, frame)) => (
                viewport.width.min(client.width).as_f32().max(1.0),
                viewport.height.min(client.height).as_f32().max(1.0),
                frame,
            ),
            None => (
                viewport.width.as_f32().max(1.0),
                viewport.height.as_f32().max(1.0),
                None,
            ),
        };
        let (client_width, client_height) = (fit_width.round() as u32, fit_height.round() as u32);
        if self.reported_client != Some((client_width, client_height)) {
            self.reported_client = Some((client_width, client_height));
            let frame = dwm_frame.map_or_else(
                || "native chrome unavailable".to_string(),
                |frame| {
                    format!(
                        "DWM frame {:.1}x{:.1}",
                        frame.width.as_f32(),
                        frame.height.as_f32()
                    )
                },
            );
            log::info!(
                "[rustgis] layout: GPUI viewport {:.1}x{:.1}, client {client_width}x\
                 {client_height}, {frame}, delta {:.1}x{:.1}",
                viewport.width.as_f32(),
                viewport.height.as_f32(),
                viewport.width.as_f32() - fit_width,
                viewport.height.as_f32() - fit_height,
            );
        }

        // The map takes exactly the client area minus the bar; the frame is drawn for that area
        // and `ObjectFit::Fill` maps it one-to-one, so pointer positions and camera pixels agree.
        let map_height = (fit_height - STATUS_BAR_HEIGHT).max(1.0);
        let area = (
            fit_width.max(1.0).round() as u32,
            map_height.round() as u32,
        );
        self.want = Some(area);
        let compact = fit_width < 760.0;

        let image = self.image.clone();
        let status = self.status_text();
        let frame_text = self.frame_text();
        let message = self.message.clone();
        let hint = self.hint_text();
        let area_text = SharedString::from(format!("{}×{}", area.0, area.1));

        v_flex()
            .id("rustgis")
            .w(px(fit_width))
            .h(px(fit_height))
            .relative()
            .flex_col()
            .bg(cx.theme().colors().editor_background)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                let handled = match event.keystroke.key.as_str() {
                    "+" | "=" | "add" => {
                        this.queue_zoom_center(KEY_ZOOM_FACTOR);
                        true
                    }
                    "-" | "_" | "subtract" => {
                        this.queue_zoom_center(1.0 / KEY_ZOOM_FACTOR);
                        true
                    }
                    "left" => {
                        this.queue_pan(-KEY_PAN_STEP, 0.0);
                        true
                    }
                    "right" => {
                        this.queue_pan(KEY_PAN_STEP, 0.0);
                        true
                    }
                    "up" => {
                        this.queue_pan(0.0, KEY_PAN_STEP);
                        true
                    }
                    "down" => {
                        this.queue_pan(0.0, -KEY_PAN_STEP);
                        true
                    }
                    _ => false,
                };
                if handled {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .relative()
                    .overflow_hidden()
                    .bg(cx.theme().colors().editor_background)
                    .child(match image {
                        Some(image) => img(ImageSource::Render(image))
                            // Normal flow, sized to the map area: an absolute image paints over
                            // the status overlay below it, whichever order the two are added in.
                            .w(px(area.0 as f32))
                            .h(px(area.1 as f32))
                            .object_fit(ObjectFit::Fill)
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
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.drag_from = Some(event.position);
                            this.focus_handle.focus(window, cx);
                            cx.notify();
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
                            this.queue_pan(dx as f64, dy as f64);
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _window, _cx| {
                            this.drag_from = None;
                            this.idle_deadline = Some(Instant::now() + MOUSE_UP_SETTLE);
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
                            if ticks != 0.0 {
                                this.queue_zoom(
                                    event.position.x.as_f32() as f64,
                                    event.position.y.as_f32() as f64,
                                    ZOOM_PER_TICK.powf(ticks as f64),
                                );
                                cx.notify();
                            }
                            cx.stop_propagation();
                        },
                    ))
                    .child(
                // The status bar is an overlay at the bottom of the map container, added after
                // the image so it paints above it (an absolute image paints over normal-flow
                // siblings in GPUI). It covers exactly the strip the image does not.
                v_flex()
                    .id("rustgis-status")
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .w_full()
                    .h(px(STATUS_BAR_HEIGHT))
                    .justify_center()
                    .gap_1()
                    .px_3()
                    .py_1()
                    .bg(cx.theme().colors().panel_background)
                    .border_t_1()
                    .border_color(cx.theme().colors().border)
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_3()
                            // The status is the one label that may be long (a long scale); it
                            // shrinks and truncates first so the frame data, the message and the
                            // buttons keep their room.
                            .child(
                                div().flex_1().overflow_hidden().child(
                                    Label::new(status)
                                        .size(LabelSize::Small)
                                        .color(Color::Muted)
                                        .truncate(),
                                ),
                            )
                            .children((!compact).then(|| {
                                Label::new(area_text)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Hidden)
                            }))
                            .children(frame_text.map(|text| {
                                Label::new(text)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                            }))
                            .children(message.map(|message| {
                                div()
                                    .max_w(px((fit_width * 0.38).max(120.0)))
                                    .overflow_hidden()
                                    .child(
                                        Label::new(message)
                                            .size(LabelSize::Small)
                                            .color(Color::Accent)
                                            .truncate(),
                                    )
                            })),
                    )
                    .child(
                        // Two groups with a flexible spacer: on a narrow window the hint drops
                        // out (`compact`) and the buttons wrap rather than overflow the bar.
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                Button::new("rustgis-zoom-out", "Zoom −")
                                    .style(ButtonStyle::OutlinedGhost)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.queue_zoom_center(1.0 / KEY_ZOOM_FACTOR);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("rustgis-zoom-in", "Zoom +")
                                    .style(ButtonStyle::OutlinedGhost)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.queue_zoom_center(KEY_ZOOM_FACTOR);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("rustgis-world", "World")
                                    .style(ButtonStyle::OutlinedGhost)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.queue_world();
                                        cx.notify();
                                    })),
                            )
                            .child(div().flex_1())
                            .children((!compact).then(|| {
                                Label::new(hint)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Hidden)
                            })),
                    ),
                ),
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
fn load_file_in_background(commands: SyncSender<EngineCommand>, path: String) {
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

/// The window's real client area, in logical pixels, and the visible frame for the log.
///
/// `GetClientRect` is the client area itself; DWM's `DWMWA_EXTENDED_FRAME_BOUNDS` is the visible
/// frame without Windows 10's invisible resize border. Returning both lets the caller lay the
/// window out for the smaller of the client area and `viewport_size` (so a bottom row is never
/// drawn below the part of the window a person can see), and record the chrome GPUI's viewport can
/// be ahead by on the first frames. `None` on other platforms and when the native handle is
/// unavailable (the caller then trusts GPUI's viewport).
#[cfg(target_os = "windows")]
fn native_client_size(window: &Window) -> Option<(Size<Pixels>, Option<Size<Pixels>>)> {
    use raw_window_handle::RawWindowHandle;
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

    let handle = raw_window_handle::HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client).ok()? };
    let mut frame = RECT::default();
    let frame_known = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut frame as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok()
    };
    let scale = window.scale_factor().max(0.01);
    let logical = |left: i32, top: i32, right: i32, bottom: i32| {
        size(
            px((right - left) as f32 / scale),
            px((bottom - top) as f32 / scale),
        )
    };
    let client = logical(client.left, client.top, client.right, client.bottom);
    let visible = frame_known.then(|| logical(frame.left, frame.top, frame.right, frame.bottom));
    Some((client, visible))
}

#[cfg(not(target_os = "windows"))]
fn native_client_size(_window: &Window) -> Option<(Size<Pixels>, Option<Size<Pixels>>)> {
    None
}

/// Fold a wheel notch into the zoom already waiting for the next frame: one factor for every
/// notch since the last frame, about the latest pointer position.
fn fold_zoom(
    pending: Option<(f64, f64, f64)>,
    x: f64,
    y: f64,
    factor: f64,
) -> Option<(f64, f64, f64)> {
    Some(match pending {
        Some((_, _, previous)) => (x, y, previous * factor),
        None => (x, y, factor),
    })
}

/// A frame is due only when a gesture is waiting, nothing is in flight, and the 33 ms throttle
/// has elapsed. Every input event in between is folded into that one frame.
fn interaction_due(has_gesture: bool, in_flight: bool, last: Option<Instant>, now: Instant) -> bool {
    has_gesture
        && !in_flight
        && last.is_none_or(|last| now.duration_since(last) >= INTERACTION_INTERVAL)
}

/// A frame drawn by `tiny-skia` becomes a GPUI image.
///
/// The engine thread already swapped the channels (GPUI's atlas stores **BGRA**, see
/// `gpui_wgpu`'s `swizzle_upload_data`), so this only moves the bytes into an `image::Frame`:
/// no per-pixel work and no copy on the UI thread.
fn bgra_to_render_image(bgra: Vec<u8>, width: u32, height: u32) -> Arc<RenderImage> {
    let buffer: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_raw(width, height, bgra)
            // A size mismatch cannot be drawn; fall back to a black frame rather than panic.
            .unwrap_or_else(|| image::ImageBuffer::new(width, height));
    Arc::new(RenderImage::new(SmallVec::from_elem(
        image::Frame::new(buffer),
        1,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BGRA frame reaches GPUI untouched: the engine thread did the channel swap, and the UI
    /// thread must not do it again.
    #[test]
    fn bgra_frames_reach_gpui_without_a_swap() {
        let image = bgra_to_render_image(vec![30, 20, 10, 40, 70, 60, 50, 80], 2, 1);

        assert_eq!(image.frame_count(), 1);
        let frame_size = image.size(0);
        assert_eq!((frame_size.width.0, frame_size.height.0), (2, 1));
        assert_eq!(
            image.as_bytes(0),
            Some([30, 20, 10, 40, 70, 60, 50, 80].as_slice())
        );
    }

    /// `+`/`-` and repeated wheel notches fold into one factor about the latest anchor.
    #[test]
    fn zoom_factors_fold_and_the_latest_anchor_wins() {
        let folded = fold_zoom(None, 111.0, 222.0, 1.15);
        let folded = fold_zoom(folded, 600.0, 371.0, 2.0);
        let (x, y, factor) = folded.expect("two notches are one zoom");
        assert_eq!((x, y), (600.0, 371.0));
        assert!((factor - 2.3).abs() < 1e-9);
    }

    /// One frame at a time, and no more than one every 33 ms while a gesture keeps arriving.
    #[test]
    fn a_gesture_is_one_frame_in_flight_at_the_throttled_rate() {
        let start = Instant::now();
        assert!(interaction_due(true, false, None, start));
        assert!(!interaction_due(true, true, None, start), "a frame is in flight");
        assert!(
            !interaction_due(true, false, Some(start), start + Duration::from_millis(16)),
            "16 ms is inside the 33 ms throttle"
        );
        assert!(interaction_due(
            true,
            false,
            Some(start),
            start + Duration::from_millis(33)
        ));
        assert!(!interaction_due(false, false, None, start), "nothing to draw");
    }
}
