//! `Help > cesiumrust`: the cesiumrust 3D globe in a window of its own.
//!
//! cesiumrust (`crates/cesiumrust`) is a CesiumJS rewrite in Rust built on
//! Bevy 0.15. It ships as a standalone application rather than an embeddable
//! renderer, so this module is the glue that makes one of its scenes visible
//! inside Zed:
//!
//! * [`renderer`] owns a Bevy `App` on a dedicated thread. It disables
//!   `WinitPlugin`, renders offscreen, and publishes finished frames over a
//!   capacity-1 channel.
//! * [`RendererView`] below is the GPUI view. It converts the newest frame into
//!   a `RenderImage` and shows it.
//!
//! # What this window is, and what it is not
//!
//! The scene is the one cesiumrust documents for embedders,
//! `crates/cesiumrust/application/cesium-app/examples/minimal.rs`: the WGS84
//! ellipsoid, cesiumrust's own lighting (`CesiumCorePlugin`) and its camera. It
//! is the richest scene reachable without changing cesiumrust's sources.
//!
//! `cesium-app`'s full viewer is **not** reproduced. Its richer content — Bing
//! imagery tiles, the screen-space-error LOD quadtree, the starfield, 2D map
//! mode, the plot overlay — lives in that binary's private modules
//! (`dynamic_globe`, `tile_mesh`, `globe_lod`, `map2d`, `starfield`), and the
//! package has no library target, so none of it is callable from here. Lifting
//! it would mean rewriting cesiumrust rather than embedding it, which this
//! integration deliberately does not do.
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
    ObjectFit, Render, RenderImage, Size, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowOptions, div, img, prelude::*, px, size,
};
use smallvec::SmallVec;

use renderer::{FRAME_HEIGHT, FRAME_WIDTH, Frame, Renderer};

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
                appears_transparent: true,
                traffic_light_position: Some(gpui::point(px(12.), px(12.))),
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
    /// The Bevy thread and the channel it publishes on.
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
            *slot = Some(Renderer::start());
        }

        let renderer = self.renderer.clone();
        // Spawned through the entity, so the task receives an `AsyncApp` it can
        // use to touch the view. It runs on GPUI's foreground executor and
        // never touches a window, so it cannot block a paint.
        cx.spawn(async move |view, cx| collect_frames(view, renderer, cx).await)
            .detach();
    }

    /// The size the image should be drawn at, in device pixels.
    fn frame_size(&self) -> (u32, u32) {
        self.image
            .as_ref()
            .map(|image| {
                let frame_size = image.size(0);
                (
                    frame_size.width.0.max(1) as u32,
                    frame_size.height.0.max(1) as u32,
                )
            })
            .unwrap_or((FRAME_WIDTH, FRAME_HEIGHT))
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

        div()
            .size_full()
            .bg(gpui::black())
            .flex()
            .items_center()
            .justify_center()
            .child(match image {
                Some(image) => div()
                    .w(px(width as f32))
                    .h(px(height as f32))
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
                let focus_handle = self.focus_handle.clone();
                move |_, window, cx| focus_handle.focus(window, cx)
            })
    }
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
}
