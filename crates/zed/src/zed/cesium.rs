//! `Help > cesiumrust`: the cesiumrust 3D globe in a window of its own.
//!
//! cesiumrust (`crates/cesiumrust`) is a CesiumJS rewrite in Rust built on
//! Bevy 0.15. It ships as an application, not as a renderer, so this module is
//! the glue that makes it visible inside Zed:
//!
//! * [`renderer`] owns a Bevy `App` on a dedicated thread. It disables
//!   `WinitPlugin`, renders offscreen, and publishes finished frames over a
//!   capacity-1 channel.
//! * [`RendererView`] below is the GPUI view. It converts the newest frame into
//!   a `RenderImage`, shows it, and forwards the mouse to the viewer.
//!
//! # The scene is cesiumrust's own viewer, not a subset of it
//!
//! The Bevy app is assembled with `cesium_app::add_viewer_scene`
//! (`crates/cesiumrust/application/cesium-app/src/lib.rs`), which is the same
//! function `cargo run -p cesium-app` calls. The window therefore shows what
//! the standalone viewer shows: Bing Maps imagery on a screen-space-error LOD
//! quadtree, the baked global composite on the fallback sphere, the atmosphere
//! rim glow, the starfield, the orbit camera, and the viewer's own `bevy_ui`
//! overlays — the 2D/3D toggle, the lat/lon readout and the plot panel.
//!
//! That is a change from the previous revision, which assembled a scene by hand
//! out of `cesium-bevy-render`'s public plugins. That scene was cesiumrust's
//! `examples/minimal.rs`: the WGS84 ellipsoid with a flat dark-blue material
//! and nothing else — no imagery chain, no LOD, no starfield — because
//! `cesium-app` had no library target and its viewer was unreachable. The
//! window showed a plain blue sphere.
//!
//! # Two ordering constraints
//!
//! 1. The Bevy app is started lazily, from the window's first paint. A Bevy
//!    `RenderPlugin` initialization performs a `block_on`, which must not
//!    happen on a GPUI executor thread; and starting it lazily keeps the window
//!    tests free of GPU work, so the tests exercise the action, the menu item
//!    and the window without needing a render device.
//! 2. Frames are collected by a task on GPUI's background executor rather than
//!    by blocking a paint. It notifies only when a frame has actually arrived,
//!    so an idle window does not repaint for nothing.
//!
//! Nothing on this path panics. A renderer that cannot start (no GPU, or a
//! driver that refuses a second wgpu device) leaves the window showing its
//! loading state; a failure inside the Bevy thread is contained to that thread.
//! That matters because a panic raised while handling a window message becomes
//! `process::abort()` in a debug build on Windows, which would take the whole
//! editor down rather than fail this one window.

mod renderer;

use std::sync::{Arc, Mutex};

use gpui::{
    App, AppContext as _, Context, FocusHandle, Focusable, ImageSource, IntoElement, MouseButton,
    ObjectFit, Pixels, Point, Render, RenderImage, ScrollDelta, Size, TitlebarOptions, WeakEntity,
    Window, WindowBounds, WindowOptions, div, img, prelude::*, px, size,
};
use smallvec::SmallVec;

use renderer::{FRAME_HEIGHT, FRAME_WIDTH, Frame, Renderer, ViewerButton, ViewerInput};

/// `ZED_CESIUMRUST_OPEN` — `1`/anything but `0` opens the globe window as Zed
/// starts.
///
/// The window's menu item's path without the menu, for verifying the window on a
/// machine where clicking through `Help` is not what is being tested. Mirrors
/// `ZED_OCTOS_OPEN_STATUS` in `zed::octos`.
pub const ENV_OPEN: &str = "ZED_CESIUMRUST_OPEN";

/// Registers the startup override, if any. Called once from `zed::init`.
pub fn init(cx: &mut App) {
    if std::env::var(ENV_OPEN).is_ok_and(|value| value != "0") {
        log::info!("[cesiumrust] opening the globe window at startup ({ENV_OPEN})");
        cx.defer(open_cesiumrust);
    }
}

/// Directory the viewer keeps its downloaded imagery tiles in, under Zed's own
/// data directory.
///
/// cesiumrust's own default is `./tile_cache` relative to the **current working
/// directory** (`cesium-app`'s `feature_flags::tile_cache_dir`), which is right
/// for a standalone viewer and wrong for an embedded one: Zed is launched from
/// whatever directory the user happened to be in, so the default would scatter
/// a `tile_cache/` through their file system. `paths::data_dir()` is the same
/// directory Zed already keeps its databases and logs in
/// (`crates/paths/src/paths.rs:129`), so the cache lands next to them and is
/// removed along with the rest of the user data.
///
/// The directory itself is created lazily, by the first tile that is written
/// (`tile_cache::fetch_tile`), so a session that never reaches Bing leaves
/// nothing behind.
fn tile_cache_root() -> std::path::PathBuf {
    paths::data_dir().join("cesiumrust").join("tile_cache")
}

/// Opens the cesiumrust window, or focuses the one already open.
///
/// Mirrors `open_gpui_kit_showcase`: one window is enough, so a second
/// `Help > cesiumrust` activates the existing window instead of opening
/// another.
pub fn open_cesiumrust(cx: &mut App) {
    if let Some(existing) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<RendererView>())
    {
        if let Err(error) = existing.update(cx, |_, window, _| window.activate_window()) {
            log::warn!("[cesiumrust] could not focus the globe window: {error}");
        }
        return;
    }

    let window_size = Size {
        width: px(1280.),
        height: px(720.),
    };

    let opened = cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("cesiumrust".into()),
                // The system title bar; see `open_gpui_kit_showcase` for why.
                // This window draws no window chrome of its own, so a
                // transparent title bar (which is what hides the system one)
                // would leave it with no minimize, maximize or close button.
                appears_transparent: false,
                traffic_light_position: None,
            }),
            window_bounds: Some(WindowBounds::centered(window_size, cx)),
            window_min_size: Some(size(px(480.), px(270.))),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| RendererView::new(window, cx)),
    );
    if let Err(error) = opened {
        log::warn!("[cesiumrust] could not open the globe window: {error}");
    }
}

/// GPUI view that shows the newest frame the renderer has published.
pub struct RendererView {
    /// The frame currently on screen.
    image: Option<Arc<RenderImage>>,
    /// The Bevy thread and the channel it publishes on, and the channel input
    /// is pushed down.
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
    focus_handle: FocusHandle,
}

impl RendererView {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            image: None,
            renderer: Arc::new(Mutex::new(None)),
            renderer_started: false,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Start the renderer and its frame-collection task, once.
    fn ensure_renderer(&mut self, cx: &mut Context<Self>) {
        if self.renderer_started {
            return;
        }
        self.renderer_started = true;

        if let Ok(mut slot) = self.renderer.lock() {
            *slot = Some(Renderer::start(tile_cache_root()));
        }

        let renderer = self.renderer.clone();
        // Spawned through the entity, so the task receives an `AsyncApp` it can
        // use to touch the view. It runs on GPUI's foreground executor and
        // never touches a window, so it cannot block a paint.
        cx.spawn(async move |view, cx| collect_frames(view, renderer, cx).await)
            .detach();
    }

    /// The size the image is drawn at, in logical pixels. Also the size of the
    /// view's own window, which the renderer lays the scene's UI out for.
    fn frame_size(&self) -> (f32, f32) {
        self.image
            .as_ref()
            .map(|image| {
                let frame_size = image.size(0);
                (
                    frame_size.width.0.max(1) as f32,
                    frame_size.height.0.max(1) as f32,
                )
            })
            .unwrap_or((FRAME_WIDTH as f32, FRAME_HEIGHT as f32))
    }
}

/// Poll the renderer for frames until the window goes away.
///
/// `WeakEntity::upgrade` returning `None` is the exit condition: it means the
/// window was closed and the `Renderer` dropped, which also stops the Bevy
/// thread.
async fn collect_frames(
    view: WeakEntity<RendererView>,
    renderer: Arc<Mutex<Option<Renderer>>>,
    cx: &mut gpui::AsyncApp,
) {
    loop {
        // Paced to the renderer's own tick. There is nothing to do between
        // frames, and this keeps an idle window from spinning.
        smol::Timer::after(std::time::Duration::from_millis(16)).await;

        let frame = take_latest(&renderer);

        let Some(view) = view.upgrade() else {
            return;
        };
        if let Some(frame) = frame {
            view.update(cx, |view, cx| {
                view.image = Some(frame_to_render_image(&frame));
                cx.notify();
            });
        }
    }
}

impl Focusable for RendererView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RendererView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_renderer(cx);

        let (width, height) = self.frame_size();
        let image = self.image.clone();
        // What the image is drawn as, in logical pixels, so a mouse position can
        // be mapped onto the viewer's own window (see `viewer_position`).
        let frame = (width, height);
        // Every listener below is a `Fn`, not a `FnMut`, and none of them may
        // borrow the view: input is a side effect on the renderer, which lives
        // behind an `Arc`. Cloning the handle keeps `Render` free to hand the
        // callbacks to GPUI, and keeps a mouse move from marking the view
        // changed (and so repainting) — the frame collector is what notifies.
        let renderer = self.renderer.clone();
        let focus_handle = self.focus_handle.clone();

        div()
            .size_full()
            .bg(gpui::black())
            .flex()
            .items_center()
            .justify_center()
            .child(match image {
                Some(image) => div()
                    .w(px(width))
                    .h(px(height))
                    .child(
                        img(ImageSource::Render(image))
                            .object_fit(ObjectFit::Contain)
                            .size_full(),
                    )
                    .into_any_element(),
                None => div()
                    .text_color(gpui::white())
                    .child("cesiumrust — starting the Bevy renderer…")
                    .into_any_element(),
            })
            .on_mouse_down(MouseButton::Left, {
                let renderer = renderer.clone();
                move |event, window, cx| {
                    focus_handle.focus(window, cx);
                    press_button(
                        &renderer,
                        ViewerButton::Left,
                        true,
                        event.position,
                        window,
                        frame,
                    );
                }
            })
            // The viewer's camera reads `ButtonInput<MouseButton>`; every button
            // it uses has to be forwarded, and every press needs a matching
            // release or the grab would never let go.
            .on_mouse_down(MouseButton::Right, {
                let renderer = renderer.clone();
                move |event, window, _cx| {
                    press_button(
                        &renderer,
                        ViewerButton::Right,
                        true,
                        event.position,
                        window,
                        frame,
                    );
                }
            })
            .on_mouse_down(MouseButton::Middle, {
                let renderer = renderer.clone();
                move |event, window, _cx| {
                    press_button(
                        &renderer,
                        ViewerButton::Middle,
                        true,
                        event.position,
                        window,
                        frame,
                    );
                }
            })
            .on_mouse_up(MouseButton::Left, {
                let renderer = renderer.clone();
                move |event, window, _cx| {
                    press_button(
                        &renderer,
                        ViewerButton::Left,
                        false,
                        event.position,
                        window,
                        frame,
                    );
                }
            })
            .on_mouse_up(MouseButton::Right, {
                let renderer = renderer.clone();
                move |event, window, _cx| {
                    press_button(
                        &renderer,
                        ViewerButton::Right,
                        false,
                        event.position,
                        window,
                        frame,
                    );
                }
            })
            .on_mouse_up(MouseButton::Middle, {
                let renderer = renderer.clone();
                move |event, window, _cx| {
                    press_button(
                        &renderer,
                        ViewerButton::Middle,
                        false,
                        event.position,
                        window,
                        frame,
                    );
                }
            })
            // A release outside the window never reaches `on_mouse_up` (its
            // hitbox is not hovered), which would leave the viewer rotating or
            // zooming forever. Leaving the window is the closest thing to a
            // release this element ever sees, so it stands in for one.
            .on_mouse_exit({
                let renderer = renderer.clone();
                move |_event, _window, _cx| {
                    for button in [
                        ViewerButton::Left,
                        ViewerButton::Right,
                        ViewerButton::Middle,
                    ] {
                        forward(&renderer, ViewerInput::Button { button, pressed: false });
                    }
                }
            })
            .on_mouse_move({
                let renderer = renderer.clone();
                move |event, window, _cx| {
                    let position = viewer_position(event.position, window.viewport_size(), frame);
                    forward(
                        &renderer,
                        ViewerInput::Cursor {
                            x: position.x.as_f32(),
                            y: position.y.as_f32(),
                        },
                    );
                }
            })
            .on_scroll_wheel({
                let renderer = renderer.clone();
                move |event, window, _cx| {
                    let lines = match event.delta {
                        // GPUI's line delta and Bevy's `MouseWheel::y` agree on
                        // sign: positive is scrolling up, away from the user.
                        ScrollDelta::Lines(delta) => delta.y,
                        // A pixel delta (a precision touchpad, on the platforms
                        // whose backend reports pixels) has to be turned into
                        // lines, because the viewer's zoom is defined per line
                        // (`OrbitState::zoom_speed`, 30% of the height per
                        // line). 50 px is a conventional line.
                        ScrollDelta::Pixels(delta) => delta.y.as_f32() / 50.0,
                    };
                    if lines == 0.0 {
                        return;
                    }
                    // Where the wheel was, so the viewer's cursor and its
                    // picking agree with the render it is scrolling over.
                    let position = viewer_position(event.position, window.viewport_size(), frame);
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

/// Forward a button down/up, with the cursor position it happened at.
///
/// The position matters as much as the button: the viewer's left-drag rotation
/// picks the surface point under the cursor on the press frame.
fn press_button(
    renderer: &Arc<Mutex<Option<Renderer>>>,
    button: ViewerButton,
    pressed: bool,
    position: Point<Pixels>,
    window: &Window,
    frame: (f32, f32),
) {
    let position = viewer_position(position, window.viewport_size(), frame);
    forward(
        renderer,
        ViewerInput::Cursor {
            x: position.x.as_f32(),
            y: position.y.as_f32(),
        },
    );
    forward(renderer, ViewerInput::Button { button, pressed });
}

/// Map a position in this window to the viewer's own window coordinates.
///
/// The frame is drawn at its native size and centred, so the image's origin is
/// the window's centre minus half a frame. The viewer's window is
/// `FRAME_WIDTH x FRAME_HEIGHT` physical pixels at a scale factor of 1 (see
/// `renderer::spawn_virtual_window`), so one pixel of the image element is one
/// pixel of the viewer's window and no scaling is needed.
fn viewer_position(
    position: Point<Pixels>,
    viewport: Size<Pixels>,
    frame: (f32, f32),
) -> Point<Pixels> {
    let origin_x = (viewport.width.as_f32() - frame.0) / 2.0;
    let origin_y = (viewport.height.as_f32() - frame.1) / 2.0;
    gpui::point(
        px(position.x.as_f32() - origin_x),
        px(position.y.as_f32() - origin_y),
    )
}

/// The newest frame, discarding any the window has not drawn yet, so the window
/// shows the latest state rather than a backlog.
fn take_latest(renderer: &Arc<Mutex<Option<Renderer>>>) -> Option<Frame> {
    let guard = renderer.lock().ok()?;
    guard.as_ref().and_then(Renderer::take_latest)
}

/// Convert a readback frame into a `RenderImage`.
///
/// Bevy hands back tightly packed 8-bit pixels in the offscreen target's order.
/// GPUI's atlas stores image bytes as BGRA — see `gpui_wgpu`'s
/// `swizzle_upload_data`, which swaps the first and third channel *into* BGRA
/// when the atlas is RGBA — so the readback is swapped here, once, to keep red
/// and blue where cesiumrust put them. This is the one place the channel order
/// is decided; nothing downstream re-swizzles.
fn frame_to_render_image(frame: &Frame) -> Arc<RenderImage> {
    let mut bytes = frame.bytes.clone();
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }

    let buffer: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_raw(frame.width, frame.height, bytes)
            // A size mismatch cannot be drawn; fall back to a black frame
            // rather than panic on a readback that arrived malformed.
            .unwrap_or_else(|| image::ImageBuffer::new(frame.width, frame.height));
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
        let frame = Frame {
            width: 2,
            height: 1,
            // Two pixels: (R,G,B,A) = (10,20,30,40) and (50,60,70,80).
            bytes: vec![10, 20, 30, 40, 50, 60, 70, 80],
        };
        let image = frame_to_render_image(&frame);

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
    /// drawn as an empty frame instead of panicking. This matters more than it
    /// looks: a panic raised while handling a window message becomes
    /// `process::abort()` in a debug build on Windows, taking the editor down.
    #[test]
    fn a_malformed_frame_does_not_panic() {
        let frame = Frame {
            width: 4,
            height: 4,
            // Four bytes for a 4x4 frame: 64 bytes short.
            bytes: vec![0; 4],
        };
        let image = frame_to_render_image(&frame);
        assert_eq!(image.size(0).width.0, 4);
        assert_eq!(image.size(0).height.0, 4);
    }

    /// A window-relative position becomes the same position on the viewer's own
    /// window, with the centred image's origin removed. The frame is drawn at
    /// its native size, so the centre of the window is the centre of the frame
    /// whatever the window's size.
    #[test]
    fn mouse_positions_map_onto_the_viewers_window() {
        let frame = (1280.0, 720.0);

        // A window exactly the frame's size: no offset.
        let centred = viewer_position(
            gpui::point(px(10.0), px(20.0)),
            Size {
                width: px(1280.0),
                height: px(720.0),
            },
            frame,
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
        );
        assert_eq!((wide.x.as_f32(), wide.y.as_f32()), (100.0, 50.0));
    }
}
