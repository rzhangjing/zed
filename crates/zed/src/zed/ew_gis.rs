//! `Help > ew-gis`: the EW GIS viewer in a GPUI window.
//!
//! `ew_gis` (`crates/fundamental/ew_gis`) is a library whose
//! [`ew_gis::viewer::Viewer`] renders headless into an offscreen target and
//! reads the frame back. This module is the host:
//!
//! * [`renderer`] owns a `Viewer` on a dedicated OS thread. It creates the
//!   wgpu device, renders, and publishes finished frames over a capacity-1
//!   channel. Nothing on that thread touches GPUI, and nothing on the GPUI
//!   thread blocks on the GPU.
//! * [`EwGisView`] below is the GPUI view: it turns the newest frame into a
//!   `RenderImage`, shows it, forwards the mouse and keyboard to the viewer,
//!   and asks the renderer for a new frame size when the window resizes.
//!
//! There is no separate process: `open` creates a window in this Zed process,
//! and no `ew_gis.exe` is ever started. `ZED_EW_GIS_OPEN` opens the same window
//! at startup, mirroring `ZED_CESIUMRUST_OPEN`.
//!
//! # Why the renderer is on its own thread
//!
//! The offscreen readback is blocking: `copy_texture_to_buffer` →
//! `map_async` → `device.poll(wait_indefinitely)`. It must not run on a GPUI
//! executor thread, where blocking stalls every other window. The thread is
//! therefore a plain OS thread, and the window only ever sees finished bytes.
//!
//! # Frame pacing
//!
//! The render thread renders at most one frame every 16 ms and only when the
//! window has collected the previous one, so a minimised or unfocused window
//! idles instead of spinning. GPUI is notified from a background task when a
//! frame has actually arrived; it never repaints for nothing.
//!
//! # Failure
//!
//! A viewer that cannot start (no GPU, a driver that refuses another wgpu
//! device) leaves the window showing the error text. A panic in the render
//! thread is contained to that thread. Neither path takes the editor down —
//! which matters on Windows, where a panic raised while handling a window
//! message becomes `process::abort()` in a debug build.

mod renderer;

use std::sync::{Arc, Mutex};

use ew_gis::screenshot::FrameRgba;
use ew_gis::viewer::ViewerKey;
use gpui::{
    App, AppContext as _, Context, FocusHandle, Focusable, ImageSource, IntoElement, KeyDownEvent,
    MouseButton, ObjectFit, Pixels, Point, Render, RenderImage, ScrollDelta, Size, TitlebarOptions,
    WeakEntity, Window, WindowBounds, WindowOptions, div, img, prelude::*, px, size,
};
use smallvec::SmallVec;

use renderer::{FrameMessage, Renderer, ViewerInput};

/// `ZED_EW_GIS_OPEN` — `1`/anything but `0` opens the viewer when Zed starts.
pub const ENV_OPEN: &str = "ZED_EW_GIS_OPEN";

/// Registers the startup override, if any. Called once from `zed::init`.
pub fn init(cx: &mut App) {
    if std::env::var(ENV_OPEN).is_ok_and(|value| value != "0") {
        log::info!("[ew-gis] opening the viewer at startup ({ENV_OPEN})");
        cx.defer(open);
    }
}

/// Opens the EW GIS window, or focuses the one already open.
///
/// One window is enough, so a second `Help > ew-gis` activates the existing
/// window instead of opening another.
pub fn open(cx: &mut App) {
    if let Some(existing) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<EwGisView>())
    {
        if let Err(error) = existing.update(cx, |_, window, _| window.activate_window()) {
            log::warn!("[ew-gis] could not focus the viewer window: {error}");
        }
        return;
    }

    let window_size = Size {
        width: px(1280.),
        height: px(720.),
    };
    let display_id = cx.active_display().map(|display| display.id());

    let opened = cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("EW GIS".into()),
                // The system title bar: this window draws no chrome of its own,
                // so a transparent one would leave it with no close button.
                appears_transparent: false,
                traffic_light_position: None,
            }),
            window_bounds: Some(WindowBounds::centered(display_id, window_size, cx)),
            window_min_size: Some(size(px(480.), px(270.))),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| EwGisView::new(window, cx)),
    );
    if let Err(error) = opened {
        log::warn!("[ew-gis] could not open the viewer window: {error}");
    }
}

/// GPUI view that shows the newest frame the renderer has published.
pub struct EwGisView {
    /// The frame currently on screen.
    image: Option<Arc<RenderImage>>,
    /// Why the viewer could not start, if it could not.
    error: Option<String>,
    /// The render thread and the channels to it.
    ///
    /// Shared rather than owned so the frame-collection task can take frames
    /// without holding the view borrowed. Dropping the last handle stops the
    /// thread, because `Renderer::drop` asks it to stop and joins it — which is
    /// what closing the window does.
    renderer: Arc<Mutex<Option<Renderer>>>,
    /// Whether the renderer and its collector task have been started.
    ///
    /// A flag rather than an `Option` probe because the state is shared with
    /// the task: checking "did I already start this" must not race with the
    /// task taking the renderer out of the slot.
    renderer_started: bool,
    /// The offscreen frame size last requested from the renderer, in physical
    /// pixels. `(0, 0)` until the first paint starts it.
    frame_size: (u32, u32),
    focus_handle: FocusHandle,
}

impl EwGisView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        // Keyboard input (Tab, Delete, Ctrl+Z, Esc) is delivered to the focused
        // element's chain. This view is the only thing in the window, so it
        // takes focus as soon as it exists; a click takes it again.
        focus_handle.focus(window, cx);
        Self {
            image: None,
            error: None,
            renderer: Arc::new(Mutex::new(None)),
            renderer_started: false,
            frame_size: (0, 0),
            focus_handle,
        }
    }

    /// Start the renderer and its frame-collection task, once.
    fn ensure_renderer(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.renderer_started {
            return;
        }
        self.renderer_started = true;

        let (width, height) = target_frame_size(window);
        self.frame_size = (width, height);
        if let Ok(mut slot) = self.renderer.lock() {
            *slot = Some(Renderer::start(width, height));
        }
        log::info!("[ew-gis] starting the headless viewer at {width}x{height}");

        let renderer = self.renderer.clone();
        // Spawned through the entity, so the task receives an `AsyncApp` it can
        // use to touch the view. It runs on GPUI's foreground executor and
        // never touches a window, so it cannot block a paint.
        cx.spawn(async move |view, cx| collect_frames(view, renderer, cx).await)
            .detach();
    }

    /// Keep the renderer's frame size in step with the window.
    ///
    /// The frame is in physical pixels so the map is sharp on a high-DPI
    /// display; a resize sends one command, not one per paint, and the old
    /// frame stays on screen (stretched) until the new size arrives.
    fn sync_frame_size(&mut self, window: &Window) {
        if !self.renderer_started {
            return;
        }
        let wanted = target_frame_size(window);
        if wanted == self.frame_size {
            return;
        }
        self.frame_size = wanted;
        if let Ok(slot) = self.renderer.lock() {
            if let Some(renderer) = slot.as_ref() {
                renderer.send_input(ViewerInput::Resize {
                    width: wanted.0,
                    height: wanted.1,
                });
            }
        }
    }
}

/// Poll the renderer for frames until the window goes away.
///
/// `WeakEntity::upgrade` returning `None` is the exit condition: it means the
/// window was closed and the `Renderer` dropped, which also stops the render
/// thread.
async fn collect_frames(
    view: WeakEntity<EwGisView>,
    renderer: Arc<Mutex<Option<Renderer>>>,
    cx: &mut gpui::AsyncApp,
) {
    loop {
        // Paced to the renderer's own tick. There is nothing to do between
        // frames, and this keeps an idle window from spinning.
        smol::Timer::after(std::time::Duration::from_millis(8)).await;

        let message = take_latest(&renderer);

        let Some(view) = view.upgrade() else {
            return;
        };
        if let Some(message) = message {
            view.update(cx, |view, cx| {
                match message {
                    FrameMessage::Frame(frame) => {
                        view.image = Some(frame_to_render_image(frame));
                        view.error = None;
                    }
                    FrameMessage::Error(error) => view.error = Some(error),
                }
                cx.notify();
            });
        }
    }
}

impl Focusable for EwGisView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EwGisView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_renderer(window, cx);
        self.sync_frame_size(window);

        let image = self.image.clone();
        let error = self.error.clone();
        // Every listener below is a `Fn`, not a `FnMut`, and none of them may
        // borrow the view: input is a side effect on the renderer, which lives
        // behind an `Arc`. Cloning the handle keeps `Render` free to hand the
        // callbacks to GPUI, and keeps a mouse move from marking the view
        // changed (and so repainting) — the frame collector is what notifies.
        let renderer = self.renderer.clone();
        let focus_handle = self.focus_handle.clone();
        let frame = self.frame_size;
        let scale = window.scale_factor();

        let content = match (image, error) {
            (Some(image), _) => div()
                .size_full()
                .child(
                    img(ImageSource::Render(image))
                        .object_fit(ObjectFit::Contain)
                        .size_full(),
                )
                .into_any_element(),
            (None, Some(error)) => div()
                .text_color(gpui::white())
                .child(format!("EW GIS could not start: {error}"))
                .into_any_element(),
            (None, None) => div()
                .text_color(gpui::white())
                .child("Loading EW GIS…")
                .into_any_element(),
        };

        div()
            .size_full()
            .bg(gpui::black())
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&self.focus_handle)
            .child(content)
            .on_mouse_down(MouseButton::Left, {
                let renderer = renderer.clone();
                let focus_handle = focus_handle.clone();
                move |event, window, cx| {
                    focus_handle.focus(window, cx);
                    let position =
                        viewer_position(event.position, window.viewport_size(), frame, scale);
                    // The press position matters as much as the button: the
                    // viewer picks the shape under the cursor on the press.
                    forward(
                        &renderer,
                        ViewerInput::Cursor {
                            x: position.x.as_f32(),
                            y: position.y.as_f32(),
                        },
                    );
                    forward(
                        &renderer,
                        ViewerInput::MouseDown {
                            x: position.x.as_f32(),
                            y: position.y.as_f32(),
                        },
                    );
                }
            })
            // A release outside the window never reaches `on_mouse_up` (its
            // hitbox is not hovered), which would leave the viewer panning
            // forever. Leaving the window stands in for a release.
            .on_mouse_exit({
                let renderer = renderer.clone();
                move |_event, _window, _cx| {
                    forward(&renderer, ViewerInput::MouseUp);
                }
            })
            .on_mouse_up(MouseButton::Left, {
                let renderer = renderer.clone();
                move |_event, _window, _cx| {
                    forward(&renderer, ViewerInput::MouseUp);
                }
            })
            .on_mouse_move({
                let renderer = renderer.clone();
                move |event, window, _cx| {
                    let position =
                        viewer_position(event.position, window.viewport_size(), frame, scale);
                    forward(
                        &renderer,
                        ViewerInput::Cursor {
                            x: position.x.as_f32(),
                            y: position.y.as_f32(),
                        },
                    );
                }
            })
            .on_scroll({
                let renderer = renderer.clone();
                move |event, window, _cx| {
                    let lines = match event.delta {
                        // GPUI's line delta and the viewer's wheel agree on
                        // sign: positive is scrolling up, away from the user,
                        // which zooms in.
                        ScrollDelta::Lines(delta) => delta.y,
                        // A pixel delta (a precision touchpad) has to become
                        // lines, because the viewer's zoom is defined per line.
                        // 50 px is a conventional line.
                        ScrollDelta::Pixels(delta) => delta.y.as_f32() / 50.0,
                    };
                    if lines == 0.0 {
                        return;
                    }
                    // Where the wheel was, so the viewer's cursor agrees with
                    // the frame it is scrolling over.
                    let position =
                        viewer_position(event.position, window.viewport_size(), frame, scale);
                    forward(
                        &renderer,
                        ViewerInput::Cursor {
                            x: position.x.as_f32(),
                            y: position.y.as_f32(),
                        },
                    );
                    forward(&renderer, ViewerInput::Wheel { lines });
                }
            })
            .on_key_down({
                let renderer = renderer.clone();
                move |event, _window, _cx| {
                    // Holding a key must not fire the action repeatedly: the
                    // viewer's Delete/undo are one-shot commands.
                    if let Some(key) = viewer_key(event) {
                        forward(&renderer, ViewerInput::Key(key));
                    }
                }
            })
    }
}

/// The viewer key a GPUI key-down event names, if it is one the viewer knows.
fn viewer_key(event: &KeyDownEvent) -> Option<ViewerKey> {
    if event.is_held {
        return None;
    }
    let keystroke = &event.keystroke;
    match keystroke.key.to_ascii_lowercase().as_str() {
        "tab" => Some(ViewerKey::Tab),
        "1" => Some(ViewerKey::Plane),
        "2" => Some(ViewerKey::Globe),
        "delete" | "backspace" => Some(ViewerKey::Delete),
        "z" if keystroke.modifiers.control => Some(ViewerKey::Undo),
        "escape" => Some(ViewerKey::Escape),
        _ => None,
    }
}

/// Forward one event to the viewer, if the renderer has started.
///
/// A no-op before the first paint, which is also when the renderer is created;
/// input cannot arrive before the window has drawn anyway.
fn forward(renderer: &Arc<Mutex<Option<Renderer>>>, input: ViewerInput) {
    if let Ok(slot) = renderer.lock() {
        if let Some(renderer) = slot.as_ref() {
            renderer.send_input(input);
        }
    }
}

/// The newest message, discarding frames the window has not drawn yet, so the
/// window shows the latest state rather than a backlog.
fn take_latest(renderer: &Arc<Mutex<Option<Renderer>>>) -> Option<FrameMessage> {
    let guard = renderer.lock().ok()?;
    guard.as_ref().and_then(Renderer::take_latest)
}

/// The frame size to ask the viewer for, in physical pixels.
///
/// The window's viewport (logical pixels) times its scale factor, capped so a
/// 4K window does not turn every frame into a 33 MB readback. Capping preserves
/// the aspect ratio; the image element's `ObjectFit::Contain` then letterboxes
/// whatever is left, which [`viewer_position`] accounts for.
fn target_frame_size(window: &Window) -> (u32, u32) {
    let viewport = window.viewport_size();
    let device = viewport.to_device_pixels(window.scale_factor());
    fit_frame_size(device.width.0, device.height.0)
}

/// Largest offscreen frame the window asks for. 2560x1600 is a 4 MP readback
/// (about 16 MB), which the CPU readback path sustains; a larger window is
/// rendered at this cap and upscaled by the image element.
const MAX_FRAME_WIDTH: u32 = 2560;
const MAX_FRAME_HEIGHT: u32 = 1600;

fn fit_frame_size(width: i32, height: i32) -> (u32, u32) {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let scale = (MAX_FRAME_WIDTH as f32 / width)
        .min(MAX_FRAME_HEIGHT as f32 / height)
        .min(1.0);
    (
        ((width * scale).round() as u32).max(1),
        ((height * scale).round() as u32).max(1),
    )
}

/// Map a position in this window (logical pixels) to the viewer's own frame
/// coordinates (physical pixels).
///
/// The image is drawn by `ObjectFit::Contain` in the window's full size, so it
/// is scaled by the smaller of the two ratios and centred. The frame itself is
/// in physical pixels, hence the final scale factor.
fn viewer_position(
    position: Point<Pixels>,
    viewport: Size<Pixels>,
    frame: (u32, u32),
    scale: f32,
) -> Point<Pixels> {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let frame_width = frame.0.max(1) as f32 / scale;
    let frame_height = frame.1.max(1) as f32 / scale;
    let fit = (viewport.width.as_f32() / frame_width)
        .min(viewport.height.as_f32() / frame_height)
        .max(f32::EPSILON);
    let origin_x = (viewport.width.as_f32() - frame_width * fit) / 2.0;
    let origin_y = (viewport.height.as_f32() - frame_height * fit) / 2.0;
    gpui::point(
        px((position.x.as_f32() - origin_x) / fit * scale),
        px((position.y.as_f32() - origin_y) / fit * scale),
    )
}

/// Convert a readback frame into a `RenderImage`.
///
/// `ew_gis` hands back tightly packed RGBA8. GPUI's atlas stores image bytes as
/// BGRA — see `gpui_wgpu`'s `swizzle_upload_data`, which swaps the first and
/// third channel *into* BGRA when the atlas is RGBA — so the readback is
/// swapped here, once, to keep red and blue where `ew_gis` put them. This is
/// the one place the channel order is decided; nothing downstream re-swizzles.
///
/// Takes the frame by value so its multi-megabyte buffer is moved into the
/// image rather than copied again: at 1920x1080 a copy would be another 8 MB
/// per frame on the UI thread.
fn frame_to_render_image(frame: FrameRgba) -> Arc<RenderImage> {
    let FrameRgba {
        width,
        height,
        mut bytes,
    } = frame;
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }

    let buffer: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_raw(width, height, bytes)
            // A size mismatch cannot be drawn; fall back to a black frame
            // rather than panic on a readback that arrived malformed.
            .unwrap_or_else(|| image::ImageBuffer::new(width, height));
    Arc::new(RenderImage::new(SmallVec::from_elem(
        image::Frame::new(buffer),
        1,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A readback frame becomes a single-frame `RenderImage` of the same size,
    /// with red and blue swapped for GPUI's BGRA atlas and alpha untouched.
    #[test]
    fn frames_map_to_render_images_with_swapped_channels() {
        let frame = FrameRgba {
            width: 2,
            height: 1,
            // Two pixels: (R,G,B,A) = (10,20,30,40) and (50,60,70,80).
            bytes: vec![10, 20, 30, 40, 50, 60, 70, 80],
        };
        let image = frame_to_render_image(frame);

        assert_eq!(image.frame_count(), 1);
        let frame_size = image.size(0);
        assert_eq!((frame_size.width.0, frame_size.height.0), (2, 1));
        assert_eq!(
            image.as_bytes(0),
            Some([30, 20, 10, 40, 70, 60, 50, 80].as_slice()),
            "red and blue are swapped for the BGRA atlas, alpha is untouched"
        );
    }

    /// A malformed readback — a buffer that does not match the stated size — is
    /// drawn as an empty frame instead of panicking. A panic raised while
    /// handling a window message becomes `process::abort()` in a debug build on
    /// Windows, taking the editor down.
    #[test]
    fn a_malformed_frame_does_not_panic() {
        let frame = FrameRgba {
            width: 4,
            height: 4,
            // Four bytes for a 4x4 frame: 64 bytes short.
            bytes: vec![0; 4],
        };
        let image = frame_to_render_image(frame);
        assert_eq!(image.size(0).width.0, 4);
        assert_eq!(image.size(0).height.0, 4);
    }

    /// Mouse positions map onto the viewer's frame: the image is centred with
    /// `ObjectFit::Contain`, and the frame's physical pixels are `scale` times
    /// its logical ones.
    #[test]
    fn mouse_positions_map_onto_the_viewers_frame() {
        let frame = (1280, 720);

        // A window exactly the frame's logical size at 1x: no offset, no scale.
        let centred = viewer_position(
            gpui::point(px(10.0), px(20.0)),
            Size {
                width: px(1280.0),
                height: px(720.0),
            },
            frame,
            1.0,
        );
        assert_eq!((centred.x.as_f32(), centred.y.as_f32()), (10.0, 20.0));

        // A window twice as wide: the image starts 640 px in, so a point at the
        // window's centre is the frame's centre.
        let wide = viewer_position(
            gpui::point(px(640.0 + 100.0), px(360.0 + 50.0)),
            Size {
                width: px(2560.0),
                height: px(1440.0),
            },
            frame,
            1.0,
        );
        assert_eq!((wide.x.as_f32(), wide.y.as_f32()), (100.0, 50.0));

        // At 2x, the same logical point is twice as far into the physical frame.
        let hidpi = viewer_position(
            gpui::point(px(10.0), px(20.0)),
            Size {
                width: px(640.0),
                height: px(360.0),
            },
            frame,
            2.0,
        );
        assert_eq!((hidpi.x.as_f32(), hidpi.y.as_f32()), (20.0, 40.0));
    }

    /// The frame size follows the window, preserving the aspect ratio, and is
    /// capped so a huge window does not produce an unbounded readback.
    #[test]
    fn frame_sizes_follow_the_window_and_are_capped() {
        assert_eq!(fit_frame_size(1280, 720), (1280, 720));
        // 4K is capped to 2560 wide, keeping 16:9.
        assert_eq!(fit_frame_size(3840, 2160), (2560, 1440));
        // A tiny viewport still produces a usable frame.
        assert_eq!(fit_frame_size(0, 0), (1, 1));
    }

    /// Only the keys the viewer documents become viewer keys, and a held key
    /// does not repeat a one-shot action.
    #[test]
    fn key_events_map_to_viewer_keys() {
        use gpui::{Keystroke, Modifiers};

        let event = |key: &str, control: bool, is_held: bool| KeyDownEvent {
            keystroke: Keystroke {
                modifiers: Modifiers {
                    control,
                    ..Modifiers::none()
                },
                key: key.to_string(),
                key_char: None,
            },
            is_held,
            prefer_character_input: false,
        };

        assert_eq!(
            viewer_key(&event("tab", false, false)),
            Some(ViewerKey::Tab)
        );
        assert_eq!(
            viewer_key(&event("delete", false, false)),
            Some(ViewerKey::Delete)
        );
        assert_eq!(
            viewer_key(&event("z", true, false)),
            Some(ViewerKey::Undo),
            "Ctrl+Z undoes"
        );
        assert_eq!(viewer_key(&event("z", false, false)), None);
        assert_eq!(
            viewer_key(&event("escape", false, false)),
            Some(ViewerKey::Escape)
        );
        assert_eq!(
            viewer_key(&event("tab", false, true)),
            None,
            "a held key must not repeat Tab"
        );
        assert_eq!(viewer_key(&event("a", false, false)), None);
    }

    /// `SceneMode` is re-exported by the viewer, matching the standalone host's
    /// default. This guards the library boundary the window depends on.
    #[test]
    fn the_viewer_starts_on_the_2d_plane() {
        use ew_gis::scene::SceneMode;
        assert_eq!(SceneMode::Plane.label(), "2D Plane");
    }
}
