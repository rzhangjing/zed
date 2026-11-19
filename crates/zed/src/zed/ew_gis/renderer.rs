//! The EW GIS render thread: a headless `ew_gis::viewer::Viewer` on a thread of
//! its own, publishing finished frames to the GPUI view.
//!
//! `ew_gis` is a library now (`crates/fundamental/ew_gis`), but its renderer
//! still blocks: an offscreen frame is copied to a readback buffer and mapped
//! with `device.poll`. That may not happen on a GPUI executor thread, so the
//! viewer is built and driven here, on a plain OS thread, and the window only
//! ever sees finished bytes.
//!
//! The channel discipline is the same as the cesiumrust window's: a
//! **capacity-1** frame channel with `try_send`, keeping the newest frame and
//! dropping older ones, so a slow window can never apply backpressure to the
//! render thread. A `pending` flag stops the thread from rendering a frame
//! nobody has collected yet (a minimised window must not peg a core).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use ew_gis::scene::SceneMode;
use ew_gis::screenshot::FrameRgba;
use ew_gis::viewer::{Viewer, ViewerKey, ViewerOptions};

/// Target interval between frames. The CPU readback is the real limit — at
/// 1280x720 it costs roughly 10 ms — so this paces the loop near 60 fps without
/// spinning when a frame is already waiting for the window.
const TICK: Duration = Duration::from_millis(16);

/// How long the thread waits before re-checking a full frame channel.
const IDLE: Duration = Duration::from_millis(4);

/// A finished frame or the error that stopped the viewer.
pub enum FrameMessage {
    Frame(FrameRgba),
    Error(String),
}

/// One input event the window forwards to the viewer.
///
/// Positions are in the viewer's own pixel coordinates (physical pixels of the
/// offscreen frame), not the window's logical pixels; the GPUI side maps them.
#[derive(Debug, Clone, Copy)]
pub enum ViewerInput {
    Cursor {
        x: f32,
        y: f32,
    },
    MouseDown {
        x: f32,
        y: f32,
    },
    MouseUp,
    /// Positive `lines` scrolls up, zooming in.
    Wheel {
        lines: f32,
    },
    Key(ViewerKey),
    Resize {
        width: u32,
        height: u32,
    },
}

/// Handle to the render thread and the newest frame it has produced.
///
/// Dropping this asks the thread to stop and joins it, so closing the window
/// releases the wgpu device, the tile atlas and every texture.
pub struct Renderer {
    frames: Receiver<FrameMessage>,
    input: Sender<ViewerInput>,
    running: Arc<AtomicBool>,
    /// A frame the window has not collected yet. Cleared by [`Self::take_latest`].
    pending: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Renderer {
    /// Start the render thread. Returns immediately; the first frame arrives
    /// only after the headless wgpu device, the tile atlas and the globe have
    /// been created, which takes a moment.
    pub fn start(width: u32, height: u32) -> Self {
        let (tx, frames) = sync_channel(1);
        let (input, input_rx) = std::sync::mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));
        let pending = Arc::new(AtomicBool::new(false));
        let thread_running = running.clone();
        let thread_pending = pending.clone();

        let thread = std::thread::Builder::new()
            .name("ew-gis-renderer".into())
            .spawn(move || {
                // The thread owns the viewer for its whole life. Any panic here
                // is contained to this thread; the window keeps its last frame.
                run(tx, input_rx, thread_running, thread_pending, width, height);
            })
            .ok();

        Self {
            frames,
            input,
            running,
            pending,
            thread,
        }
    }

    /// Forward one input event to the viewer.
    pub fn send_input(&self, input: ViewerInput) {
        let _ = self.input.send(input);
    }

    /// The newest frame or error, if one has arrived since the last call.
    /// Older frames are discarded: the window shows the latest state, not a
    /// backlog.
    pub fn take_latest(&self) -> Option<FrameMessage> {
        let mut latest = None;
        while let Ok(message) = self.frames.try_recv() {
            latest = Some(message);
        }
        self.pending.store(false, Ordering::Relaxed);
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

fn run(
    tx: SyncSender<FrameMessage>,
    input: Receiver<ViewerInput>,
    running: Arc<AtomicBool>,
    pending: Arc<AtomicBool>,
    width: u32,
    height: u32,
) {
    let mut viewer = match Viewer::new(ViewerOptions {
        width,
        height,
        mode: SceneMode::Plane,
        ..ViewerOptions::default()
    }) {
        Ok(viewer) => viewer,
        Err(error) => {
            let _ = tx.try_send(FrameMessage::Error(error));
            return;
        }
    };
    log::info!(
        "[ew-gis] headless viewer started: {}x{} ({}x{} requested)",
        viewer.gpu.width,
        viewer.gpu.height,
        width,
        height
    );

    let mut next = Instant::now();
    // Startup measurement. The first frames include the initial tile uploads and
    // the window's first paint, so three samples are logged: the first is
    // conservative, the later ones are the steady state. They exist so a bug
    // report about "the window is slow" has numbers in it.
    let mut report_from = Instant::now();
    let mut measured_frames = 0u64;
    let mut reported_frames = 0u64;
    while running.load(Ordering::Relaxed) {
        while let Ok(input) = input.try_recv() {
            apply(&mut viewer, input);
        }

        // Do not render ahead of the window: a frame that has not been
        // collected yet is the freshest state the user can see.
        if pending.load(Ordering::Relaxed) {
            std::thread::sleep(IDLE);
            continue;
        }

        let render_started = Instant::now();
        match viewer.render_rgba() {
            Ok(frame) => {
                let render_and_readback_ms = render_started.elapsed().as_secs_f64() * 1000.0;
                measured_frames += 1;
                if measured_frames == 300 || measured_frames == 600 || measured_frames == 900 {
                    let elapsed = report_from.elapsed().as_secs_f64();
                    let frames = measured_frames - reported_frames;
                    log::info!(
                        "[ew-gis] {}x{}: frames {}-{} in {:.2} s ({:.1} fps presented, last render+readback {:.2} ms)",
                        frame.width,
                        frame.height,
                        reported_frames + 1,
                        measured_frames,
                        elapsed,
                        if elapsed > 0.0 {
                            frames as f64 / elapsed
                        } else {
                            0.0
                        },
                        render_and_readback_ms
                    );
                    reported_frames = measured_frames;
                    report_from = Instant::now();
                }
                pending.store(true, Ordering::Relaxed);
                match tx.try_send(FrameMessage::Frame(frame)) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {}
                    // The window is gone; nothing left to publish to.
                    Err(TrySendError::Disconnected(_)) => return,
                }
            }
            Err(error) => {
                log::error!("[ew-gis] offscreen frame failed: {error}");
                let _ = tx.try_send(FrameMessage::Error(error));
                return;
            }
        }

        next += TICK;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
    }
}

fn apply(viewer: &mut Viewer, input: ViewerInput) {
    match input {
        ViewerInput::Cursor { x, y } => viewer.mouse_move(x, y),
        ViewerInput::MouseDown { x, y } => viewer.mouse_down(x, y),
        ViewerInput::MouseUp => viewer.mouse_up(),
        ViewerInput::Wheel { lines } => viewer.wheel(lines),
        ViewerInput::Key(key) => {
            viewer.key_down(key);
        }
        ViewerInput::Resize { width, height } => {
            viewer.resize(width, height);
            log::info!("[ew-gis] frame target resized to {width}x{height}");
        }
    }
}
