//! The cesiumrust window: `cesium-app`'s own viewer scene, rendered offscreen
//! on a thread of its own and presented as a GPUI image.
//!
//! # Why the renderer is on its own thread
//!
//! cesiumrust is a Bevy application, and Bevy owns its event loop. Two
//! consequences shape this module:
//!
//! * Bevy is driven by [`ScheduleRunnerPlugin`] on a dedicated OS thread. Zed's
//!   UI thread is never blocked by a cesium frame, and — importantly — a Bevy
//!   `App` is never constructed on a GPUI executor thread, where `RenderPlugin`'s
//!   internal `block_on` during initialization would panic. A panic in a debug
//!   build on Windows is turned into `process::abort()` while handling a window
//!   message, which would take the whole editor down rather than fail this one
//!   window.
//! * The window is presented the way an image is presented: the renderer reads
//!   each finished frame back to the CPU and hands the bytes over a channel,
//!   and the GPUI view turns them into a `RenderImage`.
//!
//! # The scene is `cesium-app`'s, not a subset of it
//!
//! [`cesium_app::add_viewer_scene`] is the same function cesium-app's own
//! `main` calls (`crates/cesiumrust/application/cesium-app/src/lib.rs`), so the
//! window shows what `cargo run -p cesium-app` shows: the Bing-imagery tile
//! quadtree with its screen-space-error LOD and its baked global composite on
//! the fallback sphere, the atmosphere glow, the starfield, and the orbit
//! camera. The previous revision assembled the scene by hand out of
//! `cesium-bevy-render`'s public plugins, which is exactly
//! `examples/minimal.rs` — a WGS84 ellipsoid with a flat dark-blue material and
//! no imagery chain at all. That is what the window used to show.
//!
//! # Offscreen rendering
//!
//! `bevy::winit::WinitPlugin` is disabled and `WindowPlugin` is configured with
//! `primary_window: None` — the same shape as `cesium-bevy-render`'s own
//! `headless` module, whose `headless_window_plugin()` is reused here rather
//! than duplicated. The scene renders into an RGBA8 offscreen image
//! (`RenderTarget::Image`) and Bevy's `Screenshot` performs the
//! `copy_texture_to_buffer` -> `map_async` readback. Nothing in the cesiumrust
//! tree is modified to make this work: `add_viewer_scene`,
//! `create_offscreen_target` and `headless_window_plugin` are public API, and
//! the frame pull is a `Screenshot` plus an observer this module owns.
//!
//! # The virtual window
//!
//! `headless_window_plugin()` spawns no window entity, but two parts of the
//! scene need one: `bevy_ui` lays the viewer's 2D/3D toggle, its lat/lon readout
//! and the plot overlay out against a `Window`, and `cesium-app`'s orbit camera
//! grabs the surface point under the cursor with `Window::cursor_position()`
//! for left-drag rotation. So [`spawn_virtual_window`] spawns a plain
//! `Window` + `PrimaryWindow` entity.
//!
//! It creates no surface and no swapchain: `bevy_render`'s `extract_windows`
//! requires a `RawHandleWrapper`, which only `bevy_winit` inserts, so a window
//! entity without one is never extracted and never asked for a surface. Every
//! camera is retargeted to the offscreen image anyway.
//!
//! # Input
//!
//! `winit` is disabled, so nothing synthesises input for the viewer. The GPUI
//! view forwards what the mouse does over the image ([`ViewerInput`]), and
//! [`drain_input`] replays it into the exact resources and event streams
//! `winit` would have filled: `ButtonInput<MouseButton>`, `MouseMotion`,
//! `MouseWheel`, and the `WindowEvent` stream `bevy_picking` reads, plus the
//! virtual window's cursor position. Wheel zoom, right-drag zoom, middle-drag
//! tilt and left-drag grab-rotation therefore all work as they do in the
//! standalone viewer.
//!
//! # Channel discipline
//!
//! The renderer sends on a **capacity-1** channel with `try_send` and keeps the
//! newest frame, dropping older ones. A slow or unfocused window therefore
//! cannot apply backpressure to the render thread, and the thread never blocks
//! on the UI.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TrySendError, sync_channel};

use bevy::app::ScheduleRunnerPlugin;
use bevy::input::ButtonState;
use bevy::input::mouse::{MouseButtonInput, MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::camera::RenderTarget;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent, WindowRef};

use cesium_app::{ViewerOptions, add_viewer_scene_with_options, perf_trace::Cli};
use cesium_bevy_render::LightingMode;
use cesium_bevy_render::headless::{
    HeadlessConfig, HeadlessTarget, create_offscreen_target, headless_window_plugin,
};

/// Resolution the offscreen target renders at. Matches cesiumrust's own
/// headless default (`HeadlessConfig::default`) and its windowed default
/// (`1280x720` in `main.rs`) so what the window shows is the scene as cesiumrust
/// frames it for itself.
pub const FRAME_WIDTH: u32 = 1280;
pub const FRAME_HEIGHT: u32 = 720;

/// Interval between Bevy ticks. cesiumrust's own headless mode ticks as fast as
/// the CPU allows because it only needs one frame; a window stays open, so it
/// is paced near 60 fps to keep a minimised or idle window from pegging a core.
const TICK: std::time::Duration = std::time::Duration::from_millis(16);

/// A finished frame: tightly packed 8-bit pixels, `FRAME_WIDTH * FRAME_HEIGHT
/// * 4` bytes, in the byte order Bevy read back from the offscreen target.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

/// A mouse button, in the terms the window speaks.
///
/// A local enum rather than `bevy::input::MouseButton` so that the GPUI side of
/// this module (`crate::zed::cesium`) does not have to name a Bevy type to send
/// one.
///
/// Only the three buttons the viewer reads are here. A five-button mouse's
/// back/forward buttons are deliberately dropped rather than forwarded: nothing
/// in `cesium-app` reads them, and `bevy_picking` ignores them too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerButton {
    Left,
    Right,
    Middle,
}

impl ViewerButton {
    /// The viewer's own mouse button.
    fn as_bevy(self) -> MouseButton {
        match self {
            Self::Left => MouseButton::Left,
            Self::Right => MouseButton::Right,
            Self::Middle => MouseButton::Middle,
        }
    }
}

/// One input event the window forwards to the viewer.
///
/// Positions are in the viewer's window coordinates: physical pixels with the
/// origin at the top-left, which is the frame `Window::cursor_position()`
/// reports and the frame `bevy_ui` lays out in. Only positions are forwarded —
/// [`drain_input`] derives the movement delta, because GPUI reports an absolute
/// position and nothing else.
#[derive(Debug, Clone, Copy)]
pub enum ViewerInput {
    /// The cursor moved to `(x, y)`.
    Cursor { x: f32, y: f32 },
    /// A mouse button went down or up.
    Button {
        button: ViewerButton,
        pressed: bool,
    },
    /// The wheel scrolled. `lines` is positive scrolling up, i.e. away from the
    /// user, which is `MouseWheel::y > 0`.
    Wheel { lines: f32 },
}

/// Input queued by the window, drained by [`drain_input`] once per frame.
#[derive(Resource)]
struct ForwardedInput {
    /// The queue. A `Mutex` because a Bevy resource must be `Sync` and an
    /// `mpsc::Receiver` is not.
    rx: std::sync::Mutex<Receiver<ViewerInput>>,
    /// The last cursor position seen, replayed into the virtual window every
    /// frame so `Window::cursor_position()` resolves, and the base for the next
    /// movement delta.
    cursor: Option<Vec2>,
}

/// Handle to the renderer thread and the newest frame it has produced.
///
/// Dropping this asks the thread to stop and joins it, so closing the window
/// releases the Bevy app, its wgpu device and its image data.
pub struct Renderer {
    frames: Receiver<Frame>,
    input: Sender<ViewerInput>,
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Renderer {
    /// Start the renderer thread. Returns immediately; the first frame arrives
    /// only after Bevy has created its wgpu device and rendered once, which
    /// takes a moment.
    ///
    /// # Parameters
    /// - `cache_root`: where the viewer keeps the imagery tiles it downloads.
    ///   The host passes a directory under Zed's own data directory (see
    ///   `crate::zed::cesium::tile_cache_root`), because cesiumrust's own
    ///   default is `./tile_cache` relative to the **current working
    ///   directory** — which for a Zed launched from a shell is that shell's
    ///   directory, not Zed's.
    pub fn start(cache_root: PathBuf) -> Self {
        let (tx, frames) = sync_channel(1);
        let (input, input_rx) = std::sync::mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = running.clone();

        let thread = std::thread::Builder::new()
            .name("cesiumrust-renderer".into())
            .spawn(move || {
                // The thread owns the App for its whole life. Any panic here is
                // contained to this thread; the window simply stops updating.
                run_app(tx, input_rx, thread_running, cache_root);
            })
            .ok();

        Self {
            frames,
            input,
            running,
            thread,
        }
    }

    /// Forward one input event to the viewer.
    ///
    /// A closed queue means the render thread is gone, which is not an error
    /// worth reporting: the window is on its way out too.
    pub fn send_input(&self, input: ViewerInput) {
        let _ = self.input.send(input);
    }

    /// The newest frame, if one has arrived since the last call. Older frames
    /// are discarded: the window shows the latest state, not a backlog.
    pub fn take_latest(&self) -> Option<Frame> {
        let mut latest = None;
        while let Ok(frame) = self.frames.try_recv() {
            latest = Some(frame);
        }
        latest
    }

    /// Ask the thread to stop. Idempotent.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            // The thread checks `running` once per tick, so this waits at most
            // about one tick plus the tail of the frame in flight.
            let _ = thread.join();
        }
    }
}

/// Build and drive the Bevy app until `running` goes false.
fn run_app(
    tx: SyncSender<Frame>,
    input: Receiver<ViewerInput>,
    running: Arc<AtomicBool>,
    cache_root: PathBuf,
) {
    let mut app = build_app(tx, input, cache_root);

    // `App::run()` would do these before the first frame; driving `update()`
    // ourselves means doing them by hand. `finish()` is what runs every
    // plugin's `finish` hook — including `ScreenshotPlugin`'s, which is where
    // `CapturedScreenshots` comes from. Without it the first `Screenshot`
    // panics with "could not access system parameter ResMut<CapturedScreenshots>".
    app.finish();
    app.cleanup();

    while running.load(Ordering::Relaxed) {
        app.update();
    }
}

/// Assemble the Bevy app: `DefaultPlugins` without winit, a virtual window, the
/// offscreen target, and cesium-app's own viewer scene.
fn build_app(tx: SyncSender<Frame>, input: Receiver<ViewerInput>, cache_root: PathBuf) -> App {
    let mut app = App::new();

    // `DefaultPlugins` minus winit, driven by a schedule runner instead of the
    // OS event loop. `headless_window_plugin()` gives `primary_window: None`
    // and `ExitCondition::DontExit` (the app does not stop just because there
    // are no windows).
    app.add_plugins(
        DefaultPlugins
            .set(headless_window_plugin())
            .disable::<bevy::winit::WinitPlugin>()
            .add(ScheduleRunnerPlugin::run_loop(TICK)),
    );

    // The viewer, exactly as cesium-app's own `main` assembles it. Only the
    // four arguments that depend on *this* host differ from `main`'s values:
    //
    // * lighting: `main` reads it from `CESIUM_ENABLE_DAY_NIGHT`; `FullAmbient`
    //   is cesiumrust's default and the mode its v0 baselines were captured
    //   with, and reading Zed's environment for a cesium-specific flag would
    //   make this window's look depend on how the editor was launched.
    // * window UI: `main` skips the 2D map plugin and the plot bridge under
    //   `CESIUM_HEADLESS`. This host is not headless — it has the virtual
    //   window below — so it keeps them, which is what puts the 2D/3D toggle,
    //   the readout and the plot overlay in the window.
    // * trace: `Cli::from_env_and_args()` parses `argv`, which here is **Zed's**
    //   command line. The default disables perf-trace, which is what this
    //   window wants and all that `main` does with it in a plain run.
    // * cache: `add_viewer_scene` would leave the tile cache at cesiumrust's
    //   own default, `./tile_cache` under the current working directory. Zed's
    //   working directory is wherever it was launched from, so the host hands
    //   the viewer a directory under Zed's data directory instead.
    add_viewer_scene_with_options(
        &mut app,
        LightingMode::FullAmbient,
        true,
        Cli::default(),
        &ViewerOptions {
            cache_root: Some(cache_root),
        },
    );
    // Read the root back **out of the viewer** rather than logging the argument
    // again: this is what the 2D and 3D imagery chains will actually use, so it
    // is the value that proves the plumbing.
    log::info!(
        "[cesiumrust] imagery tile cache root: {}",
        cesium_app::tile_cache::cache_root_path().display()
    );

    app.insert_resource(ForwardedInput {
        rx: std::sync::Mutex::new(input),
        cursor: None,
    });

    // The frame readback: request a screenshot of the offscreen target, and on
    // capture hand the pixels to the window through the channel.
    app.add_systems(
        Startup,
        (spawn_virtual_window, create_target, force_msaa_off).chain(),
    );
    app.add_systems(
        Update,
        (
            retarget_cameras_offscreen,
            // Every frame, because a camera spawned after startup (or by a
            // plugin that runs later) has to be corrected too. Cheap: the
            // query is over cameras only, and re-inserting is a no-op.
            force_msaa_off,
            capture_frame,
        ),
    );
    app.add_systems(PreUpdate, drain_input.after(bevy::input::InputSystem));

    // ── Making the viewer's own `bevy_ui` react to the pointer ───────────────
    //
    // Two things have to be true for a click on the 2D/3D toggle or on the plot
    // toolbar to reach the systems that act on it (`map2d::on_mode_segment_click`
    // and `cesium_plot_bevy::ui::toolbar_click_system` both watch
    // `Changed<Interaction>`):
    //
    // 1. `bevy_ui`'s `ui_focus_system` has to run and see the pointer. It does
    //    run — `ScheduleRunnerPlugin` drives a normal `Update`/`PreUpdate` loop
    //    and `PrimaryWindow` is the single window below — but it only considers
    //    cameras whose target is a **window**:
    //    `bevy_ui-0.15.1/src/focus.rs:194` matches
    //    `NormalizedRenderTarget::Window(..)` and `return None` for anything
    //    else. Every camera in this app is retargeted to the offscreen image
    //    (`retarget_cameras_offscreen` below), so the cursor map it builds is
    //    empty, no node is ever "moused over", and every `Interaction` stays
    //    `None` for the life of the process. That is the whole reason the
    //    buttons rendered but did nothing.
    // 2. `ButtonInput<MouseButton>::just_pressed` has to survive until
    //    `ui_focus_system` reads it. `bevy_input`'s `mouse_button_input_system`
    //    (`bevy_input-0.15.1/src/mouse.rs:158`) *clears* `just_pressed` at the
    //    start of every frame and then re-applies the frame's `MouseButtonInput`
    //    **events** — which this host never writes, because it writes the
    //    `ButtonInput` resource directly. So `drain_input` must run after
    //    `InputSystem` (hence the `.after` above) or the press it recorded is
    //    erased by the clear, in the same frame, depending on thread ordering.
    //
    // The fix for (1) is to lend the offscreen cameras a window target for the
    // duration of `ui_focus_system` alone. It is deliberately not a second
    // camera: a `bevy_ui` node renders through exactly one camera — the one its
    // `TargetCamera` names (`bevy_ui-0.15.1/src/render/mod.rs:263`) — and that
    // same camera is the one the focus system hit-tests against, so a proxy
    // camera would either stop the UI from reaching the image or stop the UI
    // from being clickable. Loaning the target keeps both, and reuses Bevy's own
    // hit-testing (rounded rectangles, `FocusPolicy::Block`, clipping,
    // `RelativeCursorPosition`) instead of a hand-rolled copy of it.
    app.init_resource::<LendedCameraTargets>().add_systems(
        PreUpdate,
        (
            lend_cameras_to_ui_focus
                .after(drain_input)
                .before(bevy::ui::UiSystem::Focus),
            reclaim_cameras_from_ui_focus.after(bevy::ui::UiSystem::Focus),
        ),
    );

    // The capture side of the `Screenshot` round trip. `Screenshot::image`
    // takes ownership of the trigger's image, whose `data` is the mapped
    // readback buffer.
    app.add_observer(move |trigger: Trigger<ScreenshotCaptured>| {
        let image = &trigger.event().0;
        let width = image.texture_descriptor.size.width;
        let height = image.texture_descriptor.size.height;
        let expected = (width as usize) * (height as usize) * 4;
        if image.data.len() != expected {
            // Bevy only read back a short buffer; drop the frame rather than
            // hand the window a mismatch it would have to defend against.
            return;
        }
        let frame = Frame {
            width,
            height,
            bytes: image.data.clone(),
        };
        // Capacity 1: replace any frame the window has not picked up yet, and
        // never block the render thread on the UI.
        match tx.try_send(frame) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    });

    app
}

/// Spawn the window entity the scene is laid out against.
///
/// See the module docs: `headless_window_plugin` spawns none, but `bevy_ui` and
/// the orbit camera's left-drag grab both read one. The resolution matches the
/// offscreen target so the UI is laid out for the same viewport it is drawn
/// into, and the scale factor stays at its default of 1 so
/// `Window::cursor_position()` and the target's physical pixels are the same
/// units.
///
/// No surface is created for it: `bevy_render` only extracts windows that carry
/// a `RawHandleWrapper`, which `bevy_winit` is the only thing to insert.
fn spawn_virtual_window(mut commands: Commands) {
    commands.spawn((
        Window {
            resolution: (FRAME_WIDTH as f32, FRAME_HEIGHT as f32).into(),
            ..default()
        },
        PrimaryWindow,
    ));
}

/// Create the offscreen render target and remember it.
fn create_target(world: &mut World) {
    let config = HeadlessConfig::new(FRAME_WIDTH, FRAME_HEIGHT);
    create_offscreen_target(world, &config);
}

/// Turn MSAA off on every 3D camera.
///
/// Required, and not obvious: `create_offscreen_target` makes a target with
/// `sample_count: 1`, while Bevy's default `Msaa` is `Sample4` and the sample
/// count lives on the *camera*, not on the target. With the default the camera
/// still renders — its clear colour reaches the target — but no mesh is ever
/// drawn into a target whose sample count does not match, so the window shows a
/// blank globe. Measured: the stock Bevy sphere in `stock_mesh_reaches_the_screen`
/// produced 1 distinct colour with `Sample4` and 70 with `Off`.
///
/// It is a system rather than one line in a spawn bundle because the cameras
/// belong to the scene `add_viewer_scene` registers: this host cannot name the
/// entity that `OrbitCameraPlugin` spawns, and a camera that appears later has
/// to be corrected as well.
fn force_msaa_off(mut commands: Commands, cameras: Query<(Entity, Option<&Msaa>), With<Camera3d>>) {
    for (entity, msaa) in &cameras {
        if msaa != Some(&Msaa::Off) {
            commands.entity(entity).insert(Msaa::Off);
        }
    }
}

/// The camera targets [`lend_cameras_to_ui_focus`] swapped out, and what they
/// were before it did.
///
/// A resource rather than two `Local`s because the lending and the reclaiming
/// are two systems: the first has to run before `ui_focus_system`, the second
/// after it, and nothing else may run in between.
#[derive(Resource, Default)]
struct LendedCameraTargets(Vec<(Entity, RenderTarget)>);

/// Give every offscreen camera a window target, for the one system that needs
/// one. See the note at the registration site in [`build_app`] for why.
///
/// The swap is written through `bypass_change_detection`: it is invisible to
/// every other system — the value is back before any of them runs — so it must
/// not mark `Camera` changed and make `camera_system` rebuild the camera's
/// target info, every frame, for a value no other system ever sees.
fn lend_cameras_to_ui_focus(
    mut lended: ResMut<LendedCameraTargets>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut cameras: Query<(Entity, &mut Camera)>,
) {
    lended.0.clear();
    // `RenderTarget::Window(WindowRef::Primary)` only normalizes to a window if
    // there is a primary window to normalize against. There always is by the
    // time this runs (`spawn_virtual_window` is a `Startup` system), but a
    // missing one turns the swap into a `HTCLIENT`-less camera rather than into
    // a panic.
    if windows.get_single().is_err() {
        return;
    }
    for (entity, mut camera) in &mut cameras {
        if matches!(camera.target, RenderTarget::Image(_)) {
            lended.0.push((entity, camera.target.clone()));
            camera.bypass_change_detection().target = RenderTarget::Window(WindowRef::Primary);
        }
    }
}

/// Put the cameras back, exactly as [`lend_cameras_to_ui_focus`] found them.
fn reclaim_cameras_from_ui_focus(
    mut lended: ResMut<LendedCameraTargets>,
    mut cameras: Query<&mut Camera>,
) {
    for (entity, target) in lended.0.drain(..) {
        if let Ok(mut camera) = cameras.get_mut(entity) {
            camera.bypass_change_detection().target = target;
        }
    }
}

/// Replay the window's input into the resources and event streams the viewer
/// reads.
///
/// In `PreUpdate` so that a frame's input is visible to the `Update` systems
/// that consume it (`orbit_camera_system`, `bevy_picking`'s pointer backend,
/// `bevy_ui`'s focus system) in the same frame.
fn drain_input(
    mut input: ResMut<ForwardedInput>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut window_events: EventWriter<WindowEvent>,
    mut motion_events: EventWriter<MouseMotion>,
    mut wheel_events: EventWriter<MouseWheel>,
) {
    // Without the virtual window there is nothing to address input to, so the
    // queue is drained and dropped rather than left to grow. It cannot normally
    // be missing: `spawn_virtual_window` runs at startup.
    let Ok((window, mut virtual_window)) = windows.get_single_mut() else {
        if let Ok(rx) = input.rx.lock() {
            while rx.try_recv().is_ok() {}
        }
        return;
    };

    let previous = input.cursor;
    let mut cursor = previous;
    if let Ok(rx) = input.rx.lock() {
        while let Ok(event) = rx.try_recv() {
            match event {
                ViewerInput::Cursor { x, y } => {
                    let position = Vec2::new(x, y);
                    // The first position is a placement, not a movement: a
                    // delta against "nowhere" would be a drag the user did not
                    // make.
                    if let Some(previous) = cursor {
                        // `orbit_camera` reads the standalone `MouseMotion`
                        // event; there is no `winit` to produce it.
                        motion_events.send(MouseMotion {
                            delta: position - previous,
                        });
                    }
                    cursor = Some(position);
                }
                ViewerInput::Button { button, pressed } => {
                    // The resource directly, so the camera sees the state this
                    // frame; the `WindowEvent` as well, because that is what
                    // `bevy_picking` turns into pointer presses.
                    let button = button.as_bevy();
                    if pressed {
                        buttons.press(button);
                    } else {
                        buttons.release(button);
                    }
                    let state = if pressed {
                        ButtonState::Pressed
                    } else {
                        ButtonState::Released
                    };
                    window_events.send(WindowEvent::MouseButtonInput(MouseButtonInput {
                        button,
                        state,
                        window,
                    }));
                }
                ViewerInput::Wheel { lines } => {
                    wheel_events.send(MouseWheel {
                        unit: MouseScrollUnit::Line,
                        x: 0.0,
                        y: lines,
                        window,
                    });
                }
            }
        }
    }

    input.cursor = cursor;
    let Some(position) = cursor else {
        return;
    };
    // `winit` is what normally keeps a window's cursor position up to date;
    // disabled here, so this is the only writer. `orbit_camera`'s left-drag
    // grab and `bevy_ui`'s hover/click both read it.
    virtual_window.set_physical_cursor_position(Some(position.as_dvec2()));
    window_events.send(WindowEvent::CursorMoved(CursorMoved {
        window,
        position,
        // `None` on the first position, which is what `CursorMoved` means by an
        // unknown delta.
        delta: previous.map(|previous| position - previous),
    }));
}

/// Keep the 3D camera pointed at the offscreen target. A no-op while the target
/// does not exist, which is what makes the windowed path unaffected.
fn retarget_cameras_offscreen(
    target: Option<Res<HeadlessTarget>>,
    mut cameras: Query<&mut Camera, With<Camera3d>>,
) {
    let Some(target) = target else {
        return;
    };
    for mut camera in &mut cameras {
        if !matches!(camera.target, RenderTarget::Image(_)) {
            camera.target = RenderTarget::Image(target.image.clone());
        }
    }
}

/// Queue one screenshot per frame, so the next frame's pixels are always on
/// their way while the current one is on screen.
fn capture_frame(mut commands: Commands, target: Option<Res<HeadlessTarget>>) {
    let Some(target) = target else {
        return;
    };
    commands.spawn(Screenshot::image(target.image.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The offscreen path really produces frames, and they are the viewer's
    /// scene rather than a bare ball.
    ///
    /// Gated behind `--ignored` because it needs a GPU: it creates a wgpu device
    /// and renders the globe, which a headless or software-rendered CI machine
    /// cannot be assumed to do. It is the check that the readback wiring works
    /// at all, so it is worth running by hand after touching this module:
    ///
    /// ```text
    /// cargo test -p zed --bin zed -- --ignored cesium
    /// ```
    ///
    /// On success it leaves a PNG behind for eyeballing the channel order and
    /// the scene, which is the one part of this integration that cannot be
    /// judged from byte counts alone.
    #[test]
    #[ignore = "requires a GPU; run with --ignored, see the doc comment"]
    fn renders_frames_offscreen() {
        // A cache root inside the test's own temporary directory: the probe must
        // not leave `tile_cache/` in whatever directory `cargo test` ran from.
        let cache_root = std::env::temp_dir().join("cesiumrust_offscreen_probe_cache");
        let renderer = Renderer::start(cache_root);

        // Collect several frames rather than trusting the first: the initial
        // readbacks land before the globe's mesh and material have reached the
        // GPU, and the tile quadtree needs a moment to fetch and upload its
        // first imagery (measured: frames 0-2 are the clear colour, frame 3
        // onward is the globe, imagery lands over the following second).
        // Keeping the frame with the most distinct colours makes the assertion
        // about the steady state, which is what a window shows.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut frames = Vec::new();
        while std::time::Instant::now() < deadline && frames.len() < 40 {
            if let Some(frame) = renderer.take_latest() {
                frames.push(frame);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        assert!(
            !frames.is_empty(),
            "no frame within 30s: the offscreen target, the camera retarget or the \
             screenshot readback is not working"
        );

        let distinct_colours = |frame: &Frame| {
            frame
                .bytes
                .chunks_exact(4)
                .map(|p| [p[0], p[1], p[2]])
                .collect::<std::collections::HashSet<_>>()
        };
        let frame = frames
            .into_iter()
            .max_by_key(|frame| distinct_colours(frame).len())
            .expect("checked non-empty above");
        assert_eq!((frame.width, frame.height), (FRAME_WIDTH, FRAME_HEIGHT));
        assert_eq!(
            frame.bytes.len(),
            (FRAME_WIDTH * FRAME_HEIGHT * 4) as usize,
            "frame is not tightly packed RGBA8"
        );

        // The globe must cover a real share of the frame, not just a few stray
        // pixels. At the reference pose it fills the middle of the 1280x720
        // target; requiring a tenth of the frame leaves room for the pose to
        // change without weakening the check into "something, somewhere drew".
        let lit = frame
            .bytes
            .chunks_exact(4)
            .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
            .count();
        let total = (frame.width * frame.height) as usize;
        eprintln!(
            "[cesiumrust] {} of {total} pixels are lit ({:.1}%)",
            lit,
            100.0 * lit as f32 / total as f32
        );
        assert!(
            lit * 10 > total,
            "only {lit} of {total} pixels are lit: the globe did not reach the render graph"
        );

        // The globe alone would leave the sky black. `StarfieldPlugin` and
        // `AtmosphereGlowPlugin` are what put light outside its disc, so
        // counting lit pixels out there is the check that the window is showing
        // cesium-app's scene and not a bare sphere.
        //
        // The disc is located rather than assumed: at the default pose the globe
        // covers about 61% of the half-height (measured: `focal * tan(asin(R/d))`
        // with `focal = 360/tan(30deg)`, `R = 1`, `d = 3`), and the atmosphere's
        // outermost shell is at 1.04 of that. A pixel past 0.75 of the
        // half-height is therefore outside both, with room to spare.
        let outside_radius = 0.75 * (frame.height as f32) / 2.0;
        let inside_radius = 0.45 * (frame.height as f32) / 2.0;
        let centre = (
            (frame.width as f32) / 2.0,
            (frame.height as f32) / 2.0,
        );
        let mut outside_lit = 0usize;
        let mut outside_red = 0u64;
        let mut outside_blue = 0u64;
        // The imagery check: a flat material is one colour, imagery is not.
        let mut inside_globe_colours = std::collections::HashSet::new();
        for y in 0..frame.height {
            for x in 0..frame.width {
                let dx = x as f32 - centre.0;
                let dy = y as f32 - centre.1;
                let distance = (dx * dx + dy * dy).sqrt();
                if distance <= inside_radius {
                    let i = ((y * frame.width + x) * 4) as usize;
                    inside_globe_colours.insert([
                        frame.bytes[i],
                        frame.bytes[i + 1],
                        frame.bytes[i + 2],
                    ]);
                }
                if distance <= outside_radius {
                    continue;
                }
                let i = ((y * frame.width + x) * 4) as usize;
                let (r, g, b) = (frame.bytes[i], frame.bytes[i + 1], frame.bytes[i + 2]);
                if r > 8 || g > 8 || b > 8 {
                    outside_lit += 1;
                    outside_red += r as u64;
                    outside_blue += b as u64;
                }
            }
        }
        let distinct = distinct_colours(&frame).len();
        eprintln!(
            "[cesiumrust] {distinct} distinct colours in the frame, \
             {} in the globe's disc; {outside_lit} lit pixels outside the globe \
             (r={outside_red} b={outside_blue})",
            inside_globe_colours.len()
        );
        assert!(
            outside_lit > 20,
            "only {outside_lit} lit pixels outside the globe's disc: the starfield and \
             the atmosphere glow are not in the scene, so this is not the viewer's scene"
        );

        // Channel order. The only off-globe light is the starfield (white, so
        // red == blue) and the atmosphere glow, which is
        // `Color::srgba(0.40, 0.68, 1.0)` — blue-dominant. `frame_to_render_image`
        // swaps red and blue for GPUI's BGRA atlas, and a swap applied twice —
        // or not at all — shows up here as a red-dominant sky.
        assert!(
            outside_blue > outside_red,
            "the lit pixels outside the globe are not blue-dominant \
             (red {outside_red}, blue {outside_blue}): the readback's channel order \
             is wrong, or the atmosphere glow is missing"
        );

        // The regression this window was fixed for: a flat material is one
        // colour, `cesium-app`'s viewer is not. Its globe is covered by Bing
        // imagery on a LOD quadtree, plus the baked global composite on the
        // fallback sphere, so the disc is full of distinct colours. A flat
        // sphere — `examples/minimal.rs`, which is what this window used to
        // show — gives exactly one.
        //
        // This is the one assertion that needs the network: the imagery chain
        // fetches tiles from Bing. When it cannot, the disc is a flat fill and
        // this is the check that says so rather than leaving a blue ball to be
        // noticed by eye.
        assert!(
            inside_globe_colours.len() > 4,
            "the globe's disc has only {} distinct colours: it is a flat fill, not \
             imagery. `cesium-app`'s tile pipeline is not running, or it could not \
             reach Bing for tiles",
            inside_globe_colours.len()
        );

        let path = std::env::temp_dir().join("cesiumrust_offscreen_probe.png");
        image::RgbaImage::from_raw(frame.width, frame.height, frame.bytes)
            .expect("frame is a valid RGBA image")
            .save(&path)
            .expect("probe PNG is writable");
        eprintln!("[cesiumrust] offscreen probe written to {}", path.display());
    }
}
