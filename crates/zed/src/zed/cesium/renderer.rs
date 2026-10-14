//! The cesiumrust window: a Bevy 0.15 app from `crates/cesiumrust`, rendered
//! offscreen on a thread of its own and presented as a GPUI image.
//!
//! # Why the renderer is on its own thread
//!
//! cesiumrust is a Bevy application, not a library with a frame callback: its
//! only entry point is `App::run()` (`crates/cesiumrust/application/cesium-app/
//! src/main.rs`), and that binary has no library target, so its plugin stack
//! cannot be called as a function. Two consequences shape this module:
//!
//! * Bevy must own its own event loop, so the app is driven by
//!   [`ScheduleRunnerPlugin`] on a dedicated OS thread. Zed's UI thread is never
//!   blocked by a cesium frame, and — importantly — a Bevy `App` is never
//!   constructed on a GPUI executor thread, where `RenderPlugin`'s internal
//!   `block_on` during initialization would panic. A panic in a debug build on
//!   Windows is turned into `process::abort()` while handling a window message,
//!   which would take the whole editor down rather than fail this one window.
//! * The window is presented the way an image is presented: the renderer reads
//!   each finished frame back to the CPU and hands the bytes over a channel,
//!   and the GPUI view turns them into a `RenderImage`.
//!
//! # Offscreen rendering
//!
//! `bevy::winit::WinitPlugin` is disabled and `WindowPlugin` is configured with
//! `primary_window: None` — the same shape as `cesium-bevy-render`'s own
//! `headless` module, whose `headless_window_plugin()` is reused here rather
//! than duplicated. The scene renders into an RGBA8 offscreen image
//! (`RenderTarget::Image`) and Bevy's `Screenshot` performs the
//! `copy_texture_to_buffer` -> `map_async` readback. Nothing in the cesiumrust
//! tree is modified to make this work: `create_offscreen_target`,
//! `retarget_cameras_to_offscreen` and `headless_window_plugin` are all public
//! API of `cesium-bevy-render`, and the frame pull is a `Screenshot` plus an
//! observer this module owns.
//!
//! The globe itself follows `crates/cesiumrust/application/cesium-app/
//! examples/minimal.rs`, which is the smallest scene that crate documents.
//! `cesium-app`'s richer scene (Bing image tiles, LOD, starfield, 2D map mode)
//! lives in that binary's private modules and is therefore out of reach here;
//! the window shows the WGS84 ellipsoid with cesiumrust's own lighting and
//! camera plugins. See `crates/zed/src/zed/cesium.rs` for that caveat written
//! out in full.
//!
//! # Channel discipline
//!
//! The renderer sends on a **capacity-1** channel with `try_send` and keeps the
//! newest frame, dropping older ones. A slow or unfocused window therefore
//! cannot apply backpressure to the render thread, and the thread never blocks
//! on the UI.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::render::camera::RenderTarget;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use cesium_bevy_render::headless::{HeadlessConfig, create_offscreen_target, headless_window_plugin, HeadlessTarget};
use cesium_bevy_render::{
    CesiumCamera, CesiumCorePlugin, CesiumGlobe, GlobeConfig, LightingMode, METERS_PER_RENDER_UNIT,
    create_ellipsoid_mesh,
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

/// Handle to the renderer thread and the newest frame it has produced.
///
/// Dropping this asks the thread to stop and joins it, so closing the window
/// releases the Bevy app, its wgpu device and its image data.
pub struct Renderer {
    frames: Receiver<Frame>,
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Renderer {
    /// Start the renderer thread. Returns immediately; the first frame arrives
    /// only after Bevy has created its wgpu device and rendered once, which
    /// takes a moment.
    pub fn start() -> Self {
        let (tx, frames) = sync_channel(1);
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = running.clone();

        let thread = std::thread::Builder::new()
            .name("cesiumrust-renderer".into())
            .spawn(move || {
                // The thread owns the App for its whole life. Any panic here is
                // contained to this thread; the window simply stops updating.
                run_app(tx, thread_running);
            })
            .ok();

        Self {
            frames,
            running,
            thread,
        }
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
fn run_app(tx: SyncSender<Frame>, running: Arc<AtomicBool>) {
    let mut app = build_app(tx);

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

/// Assemble the Bevy app: `DefaultPlugins` without winit, an offscreen target,
/// and the cesiumrust plugin stack plus a globe and a camera.
fn build_app(tx: SyncSender<Frame>) -> App {
    let mut app = App::new();

    // `DefaultPlugins` minus winit, driven by a schedule runner instead of the
    // OS event loop. `headless_window_plugin()` gives `primary_window: None`
    // (no surface is ever created) and `ExitCondition::DontExit` (the app does
    // not stop just because there are no windows).
    app.add_plugins(
        DefaultPlugins
            .set(headless_window_plugin())
            .disable::<bevy::winit::WinitPlugin>()
            .add(ScheduleRunnerPlugin::run_loop(TICK)),
    );
    app.insert_resource(ClearColor(Color::BLACK));

    // Lighting mode must be inserted before `CesiumCorePlugin`, whose startup
    // system reads it; `FullAmbient` is cesiumrust's default and the one its
    // v0 baselines were captured with.
    app.insert_resource(LightingMode::FullAmbient);
    app.add_plugins(CesiumCorePlugin);

    // The frame readback: request a screenshot of the offscreen target, and on
    // capture hand the pixels to the window through the channel.
    app.add_systems(Startup, (create_target, spawn_scene).chain());
    app.add_systems(Update, (retarget_cameras_offscreen, capture_frame));

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

/// Create the offscreen render target and remember it.
fn create_target(world: &mut World) {
    let config = HeadlessConfig::new(FRAME_WIDTH, FRAME_HEIGHT);
    create_offscreen_target(world, &config);
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

/// Spawn the globe and the camera — the scene from cesiumrust's own
/// `examples/minimal.rs`.
fn spawn_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut globe_config: ResMut<GlobeConfig>,
) {
    use cesium_camera::Camera as DomainCamera;
    use cesium_geospatial::cartographic::Cartographic;
    use cesium_geospatial::ellipsoid::Ellipsoid;

    let scale = (1.0 / METERS_PER_RENDER_UNIT) as f32;

    globe_config.ellipsoid = Ellipsoid::WGS84;

    let globe_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.18, 0.35),
        perceptual_roughness: 0.85,
        ..default()
    });
    commands.spawn((
        CesiumGlobe,
        Mesh3d(meshes.add(create_ellipsoid_mesh(32, 64))),
        MeshMaterial3d(globe_material),
        Transform::from_scale(Vec3::splat(scale)),
    ));

    // A camera looking at North America from 20 000 km up: the same pose
    // cesiumrust's minimal example uses, in the same units.
    let (cam_pos, target) = {
        let eye = Cartographic::from_degrees(-95.0, 40.0, 20_000_000.0);
        let focus = Cartographic::from_degrees(-95.0, 40.0, 0.0);
        (
            Ellipsoid::WGS84.cartographic_to_cartesian(&eye),
            Ellipsoid::WGS84.cartographic_to_cartesian(&focus),
        )
    };
    let dir = (target - cam_pos).normalize();
    let up = cam_pos.normalize();
    let camera = DomainCamera::new(cam_pos, dir, up);

    commands.spawn((
        CesiumCamera {
            camera,
            ..default()
        },
        Camera3d::default(),
        Camera::default(),
        // Required, and not obvious: `create_offscreen_target` makes a target
        // with `sample_count: 1`, while Bevy's default is `Msaa::Sample4`. With
        // the default the camera still renders — its clear colour reaches the
        // target — but no mesh is ever drawn into a target whose sample count
        // does not match, so the window shows a blank globe. Measured: the stock
        // Bevy sphere in `stock_mesh_reaches_the_screen` produced 1 distinct
        // colour with `Sample4` and 70 with `Off`.
        Msaa::Off,
        Transform::from_translation(cam_pos.as_vec3() * scale)
            .looking_at(target.as_vec3() * scale, up.as_vec3()),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The offscreen path really produces frames, and they are not blank.
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
    /// On success it leaves a PNG behind for eyeballing the channel order,
    /// which is the one part of this integration that cannot be judged from
    /// byte counts alone.
    #[test]
    #[ignore = "requires a GPU; run with --ignored, see the doc comment"]
    fn renders_frames_offscreen() {
        let renderer = Renderer::start();

        // Collect several frames rather than trusting the first: the initial
        // readbacks land before the globe's mesh and material have reached the
        // GPU (measured: frames 0-2 are the clear colour, frame 3 onward is the
        // globe). Keeping the frame with the most distinct colours makes the
        // assertion about the steady state, which is what a window shows.
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

        // The globe's material is `Color::srgb(0.08, 0.18, 0.35)` under ambient
        // light, i.e. blue-dominant. Checking the channel order rather than the
        // exact colour is the point: `frame_to_render_image` swaps red and blue
        // for GPUI's BGRA atlas, and a swap applied twice — or not at all —
        // shows up here as a red-dominant globe.
        let centre = {
            let x = frame.width / 2;
            let y = frame.height / 2;
            let i = ((y * frame.width + x) * 4) as usize;
            [frame.bytes[i], frame.bytes[i + 1], frame.bytes[i + 2]]
        };
        eprintln!("[cesiumrust] centre pixel: {centre:?}");
        assert!(
            centre[2] > centre[0],
            "centre pixel {centre:?} is not blue-dominant; the readback's channel order is wrong"
        );

        let path = std::env::temp_dir().join("cesiumrust_offscreen_probe.png");
        image::RgbaImage::from_raw(frame.width, frame.height, frame.bytes)
            .expect("frame is a valid RGBA image")
            .save(&path)
            .expect("probe PNG is writable");
        eprintln!("[cesiumrust] offscreen probe written to {}", path.display());
    }
}
