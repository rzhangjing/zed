//! The engine on a thread of its own.
//!
//! `rustgis_engine::Session` is not `Send`, and a full frame can take hundreds of milliseconds in
//! an unoptimised build, so a host that has a UI thread must not own one. [`spawn_engine`] moves a
//! [`MapEngine`] onto a worker thread and talks to it with [`EngineCommand`]s, answering with
//! [`EngineEvent`]s.
//!
//! # Coalescing and backpressure
//!
//! A pointer gesture produces far more input events than the renderer can draw frames. The window
//! therefore keeps **at most one frame in flight** and folds every event since the last frame
//! into one [`EngineCommand::Interact`] (see `zed::rustgis`); the command channel is bounded
//! ([`COMMAND_QUEUE`]) and [`EngineHandle::send`] uses `try_send`, so a host that stops draining
//! events is told the engine is busy instead of growing an unbounded queue of stale views. The
//! event channel is bounded too ([`EVENT_QUEUE`]): the engine waits for the host to take a frame
//! rather than buffering megabytes of pixels.
//!
//! Interaction never runs `Session::run`: [`EngineCommand::Interact`], [`EngineCommand::ZoomCenter`]
//! and [`EngineCommand::World`] move the camera and draw a [`Quality::Preview`] frame.
//! [`EngineCommand::Settle`], or any data/edit command, draws [`Quality::Full`] again. The worker
//! watches how long a preview takes and halves its canvas once when a frame misses the 33 ms
//! budget, so a slow (debug) build degrades in sharpness rather than responsiveness.
//!
//! ```no_run
//! let mut engine = rustgis_zed::spawn_engine();
//! engine.send(rustgis_zed::EngineCommand::Render {
//!     width: 800,
//!     height: 600,
//!     quality: rustgis_zed::Quality::Full,
//! });
//! // ... later, from a timer or a channel-driven task:
//! while let Some(event) = engine.try_recv() {
//!     if let rustgis_zed::EngineEvent::Frame(frame) = event {
//!         println!("{}x{} {:?}", frame.width, frame.height, frame.quality);
//!     }
//! }
//! ```
//!
//! Nothing here blocks the caller: `send` hands a command over (or says the engine is busy),
//! `try_recv` looks for an answer, and dropping the handle asks the thread to stop and joins it.

use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{BgraFrame, CameraStatus, MapEngine, Notice, Quality};

/// How many commands may wait for the engine. The window keeps at most one frame in flight, so
/// the queue only holds commands from other threads (a file load) in practice.
pub const COMMAND_QUEUE: usize = 8;
/// How many events may wait for the host: a few preview frames, at most one full frame. A host
/// that stops reading makes the engine wait rather than buffer pixels without bound.
pub const EVENT_QUEUE: usize = 4;
/// The frame budget a preview tries to meet; above it the worker halves the preview canvas once.
const PREVIEW_BUDGET: Duration = Duration::from_millis(33);
/// A preview this fast may go back to the sharper divisor.
const PREVIEW_COMFORT: Duration = Duration::from_millis(12);
/// The sharpest and bluntest preview divisors: half and quarter of each side.
const PREVIEW_DIVISOR_MIN: u8 = 2;
const PREVIEW_DIVISOR_MAX: u8 = 4;

/// Something for the engine thread to do.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineCommand {
    /// Draw the map at this size and quality and answer with [`EngineEvent::Frame`].
    Render {
        width: u32,
        height: u32,
        quality: Quality,
    },
    /// Apply one pointer gesture — a pan by `(dx, dy)` screen pixels and/or a zoom by `factor`
    /// about `(x, y)` of the last frame — then draw a preview. Any number of mouse events between
    /// two frames becomes one of these.
    Interact {
        width: u32,
        height: u32,
        pan: (f64, f64),
        zoom: Option<(f64, f64, f64)>,
    },
    /// Zoom by `factor` about the view centre (the `Zoom In`/`Zoom Out` buttons and `+`/`-`),
    /// then draw a preview.
    ZoomCenter {
        width: u32,
        height: u32,
        factor: f64,
    },
    /// Fit the World Atlas extent (the `World` button), then draw a preview.
    World { width: u32, height: u32 },
    /// Draw a full-quality frame at the current camera: the idle settle after a gesture.
    Settle { width: u32, height: u32 },
    /// Read GeoJSON into the map, then draw a full frame.
    LoadGeoJson { name: String, bytes: Vec<u8> },
    /// Run a RustGIS command (`Session::run`), then draw a full frame.
    Command {
        id: String,
        params: serde_json::Value,
    },
    /// Turn label drawing on or off, then draw a full frame.
    SetLabels { on: bool, width: u32, height: u32 },
    /// Stop the thread.
    Shutdown,
}

/// One drawn frame, already in GPUI's BGRA byte order.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineFrame {
    pub width: u32,
    pub height: u32,
    /// Straight (non-premultiplied) BGRA, row-major, top-left first.
    pub bgra: Vec<u8>,
    /// [`Quality::Full`] for a settled frame, [`Quality::Preview`] for a gesture frame.
    pub quality: Quality,
    /// How long the worker spent on it, including the move and the render.
    pub elapsed: Duration,
}

impl EngineFrame {
    /// The frame as the adapter's [`BgraFrame`], for tests and for a host that wants the type.
    pub fn as_bgra_frame(&self) -> BgraFrame {
        BgraFrame {
            width: self.width,
            height: self.height,
            bgra: self.bgra.clone(),
        }
    }
}

/// What the engine thread answers with. Every command answers with at least one
/// [`EngineEvent::Frame`] (or [`EngineEvent::Failed`]), followed by an [`EngineEvent::Status`].
#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    /// A drawn frame, at the size it was drawn for (a preview is smaller than the view).
    Frame(EngineFrame),
    /// The map as it stands after the command: the camera, the layer count, and whatever notices
    /// RustGIS raised.
    Status {
        camera: CameraStatus,
        layers: usize,
        notices: Vec<Notice>,
    },
    /// The command failed; the map is unchanged.
    Failed { command: String, error: String },
    /// The engine is up (sent once, first).
    Ready,
}

/// A handle to the engine thread.
pub struct EngineHandle {
    commands: SyncSender<EngineCommand>,
    events: Receiver<EngineEvent>,
    thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
    /// Hand the engine a command without waiting. Fails when the bounded queue is full (the
    /// engine is still drawing; a host showing only the newest view can drop the request) or
    /// once the thread has stopped.
    pub fn send(&self, command: EngineCommand) -> Result<(), String> {
        self.commands.try_send(command).map_err(|error| match error {
            TrySendError::Full(_) => "the RustGIS engine is busy".to_string(),
            TrySendError::Disconnected(_) => "the RustGIS engine thread has stopped".to_string(),
        })
    }

    /// A second handle to the command queue, for a task that loads a file or builds a request off
    /// the UI thread. Unlike [`EngineHandle::send`] this one may wait for room: such a task has no
    /// UI to keep responsive.
    pub fn sender(&self) -> SyncSender<EngineCommand> {
        self.commands.clone()
    }

    /// Take the next event, without waiting.
    pub fn try_recv(&self) -> Option<EngineEvent> {
        match self.events.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }

    /// Wait for the next event, up to `timeout`. `None` on a timeout or a stopped thread.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<EngineEvent> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        // Best effort: if the thread is mid-frame the send fails and `recv` sees the disconnect
        // once that frame is done, which is also when `join` returns.
        let _ = self.commands.try_send(EngineCommand::Shutdown);
        // Disconnect the event channel **before** joining. A worker blocked sending a frame to a
        // host that stopped reading would otherwise never see the disconnect, and `join` would
        // wait forever (e.g. a host that drops the handle after a panic, with frames queued).
        let (_closed, placeholder) = std::sync::mpsc::channel();
        drop(std::mem::replace(&mut self.events, placeholder));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start the engine thread. The sample project is parsed there, so this returns immediately; the
/// first event is [`EngineEvent::Ready`].
pub fn spawn_engine() -> EngineHandle {
    let (commands, command_rx) = sync_channel::<EngineCommand>(COMMAND_QUEUE);
    let (event_tx, events) = sync_channel::<EngineEvent>(EVENT_QUEUE);
    let thread = std::thread::Builder::new()
        .name("rustgis-zed".to_string())
        .spawn(move || run_engine(command_rx, event_tx))
        .ok();
    if thread.is_none() {
        log::error!("[rustgis] could not start the engine thread");
    }
    EngineHandle {
        commands,
        events,
        thread,
    }
}

fn run_engine(commands: Receiver<EngineCommand>, events: SyncSender<EngineEvent>) {
    let mut engine = MapEngine::new();
    if events.send(EngineEvent::Ready).is_err() {
        return;
    }
    // The size the map is drawn at. A preview is scaled down from this by `MapEngine` itself.
    let mut size = engine.size();
    // Started sharp and halved only when a frame misses its budget (a debug build).
    let mut preview_divisor = PREVIEW_DIVISOR_MIN;
    while let Ok(command) = commands.recv() {
        let started = Instant::now();
        let quality = match command {
            EngineCommand::Shutdown => break,
            EngineCommand::Render {
                width,
                height,
                quality,
            } => {
                size = (width, height);
                quality
            }
            EngineCommand::Settle { width, height } => {
                size = (width, height);
                Quality::Full
            }
            EngineCommand::SetLabels { on, width, height } => {
                size = (width, height);
                engine.set_labels(on);
                Quality::Full
            }
            EngineCommand::LoadGeoJson { name, bytes } => {
                if let Err(error) = engine.load_geojson_named(&bytes, &name) {
                    send_failed(&events, "load_geojson", error);
                    continue;
                }
                Quality::Full
            }
            EngineCommand::Command { id, params } => {
                if let Err(error) = engine.command(&id, params) {
                    send_failed(&events, format!("command {id}"), error);
                    continue;
                }
                Quality::Full
            }
            EngineCommand::Interact {
                width,
                height,
                pan,
                zoom,
            } => {
                size = (width, height);
                engine.resize(width, height);
                let moved = Instant::now();
                if pan != (0.0, 0.0) {
                    engine.pan(pan.0, pan.1);
                }
                if let Some((x, y, factor)) = zoom {
                    engine.zoom_at(x, y, factor);
                }
                log::debug!(
                    "[rustgis] gesture applied in {:.3} ms",
                    moved.elapsed().as_secs_f64() * 1000.0
                );
                Quality::Preview {
                    divisor: preview_divisor,
                }
            }
            EngineCommand::ZoomCenter {
                width,
                height,
                factor,
            } => {
                size = (width, height);
                engine.resize(width, height);
                engine.zoom_center(factor);
                Quality::Preview {
                    divisor: preview_divisor,
                }
            }
            EngineCommand::World { width, height } => {
                size = (width, height);
                engine.resize(width, height);
                engine.zoom_world();
                Quality::Preview {
                    divisor: preview_divisor,
                }
            }
        };
        let (width, height) = size;
        // The status bar's camera and layer count are taken before the frame is drawn, so they
        // describe the map the frame shows.
        let notices = engine.take_notices();
        let camera = engine.camera_status();
        let layers = engine.layer_count();
        match engine.render_bgra_quality(width, height, quality) {
            Ok(frame) => {
                let elapsed = started.elapsed();
                if quality.is_preview() {
                    preview_divisor = next_preview_divisor(preview_divisor, elapsed);
                }
                log::info!(
                    "[rustgis] {}x{} {:?} frame in {:.1} ms (preview divisor {preview_divisor})",
                    frame.width,
                    frame.height,
                    quality,
                    elapsed.as_secs_f64() * 1000.0
                );
                let event = EngineEvent::Frame(EngineFrame {
                    width: frame.width,
                    height: frame.height,
                    bgra: frame.bgra,
                    quality,
                    elapsed,
                });
                if events.send(event).is_err() {
                    break;
                }
            }
            Err(error) => send_failed(&events, "render", error),
        }
        // Always: a host that threw away the frame still has to see where the map ended up (and
        // to know that its one in-flight frame is done).
        if events
            .send(EngineEvent::Status {
                camera,
                layers,
                notices,
            })
            .is_err()
        {
            break;
        }
    }
}

/// Halve the preview canvas when a frame misses the budget, and sharpen it again when frames are
/// comfortably fast (a release build stays at half resolution).
fn next_preview_divisor(current: u8, elapsed: Duration) -> u8 {
    let next = if elapsed > PREVIEW_BUDGET {
        (current * 2).min(PREVIEW_DIVISOR_MAX)
    } else if elapsed < PREVIEW_COMFORT {
        (current / 2).max(PREVIEW_DIVISOR_MIN)
    } else {
        current
    };
    if next != current {
        log::info!(
            "[rustgis] preview divisor {current} -> {next} (frame took {:.1} ms)",
            elapsed.as_secs_f64() * 1000.0
        );
    }
    next
}

fn send_failed(
    events: &SyncSender<EngineEvent>,
    command: impl Into<String>,
    error: impl Into<String>,
) {
    let _ = events.send(EngineEvent::Failed {
        command: command.into(),
        error: error.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wait for the next frame, panicking if the engine reports a failure or stops.
    fn next_frame(engine: &EngineHandle) -> EngineFrame {
        loop {
            match engine.recv_timeout(Duration::from_secs(180)) {
                Some(EngineEvent::Frame(frame)) => break frame,
                Some(EngineEvent::Failed { command, error }) => {
                    panic!("{command} failed: {error}")
                }
                Some(_) => continue,
                None => panic!("the engine thread stopped without drawing"),
            }
        }
    }

    /// Wait for the frame **and** the status that follows it: the pair describes the same camera.
    fn next_frame_and_status(engine: &EngineHandle) -> (EngineFrame, CameraStatus) {
        let mut frame = None;
        let mut camera = None;
        while frame.is_none() || camera.is_none() {
            match engine.recv_timeout(Duration::from_secs(180)) {
                Some(EngineEvent::Frame(f)) => frame = Some(f),
                Some(EngineEvent::Status {
                    camera: status, ..
                }) => camera = Some(status),
                Some(EngineEvent::Failed { command, error }) => {
                    panic!("{command} failed: {error}")
                }
                Some(EngineEvent::Ready) => {}
                None => panic!("the engine thread stopped without drawing"),
            }
        }
        (
            frame.expect("a frame precedes the status"),
            camera.expect("a status follows the frame"),
        )
    }

    #[test]
    fn a_full_frame_is_not_a_preview_and_a_preview_is_labelled_as_one() {
        let engine = spawn_engine();
        assert_eq!(
            engine.recv_timeout(Duration::from_secs(120)),
            Some(EngineEvent::Ready)
        );

        engine
            .send(EngineCommand::Render {
                width: 320,
                height: 200,
                quality: Quality::Full,
            })
            .expect("the engine thread is running");
        let (full, full_camera) = next_frame_and_status(&engine);
        assert_eq!((full.width, full.height), (320, 200));
        assert_eq!(full.quality, Quality::Full);
        assert!(
            !full.as_bgra_frame().is_blank(),
            "the sample project drew nothing"
        );

        // A pan is one interaction: it answers with a preview at half the canvas and a camera at
        // the new centre, and it does not run a command (`Session::run`).
        engine
            .send(EngineCommand::Interact {
                width: 320,
                height: 200,
                pan: (40.0, 0.0),
                zoom: None,
            })
            .expect("the engine thread is running");
        let (preview, preview_camera) = next_frame_and_status(&engine);
        assert_eq!(preview.quality, Quality::Preview { divisor: 2 });
        assert_eq!((preview.width, preview.height), (160, 100));
        assert!(
            (preview_camera.center_lon - full_camera.center_lon).abs() > 0.5,
            "a 40px pan should move the centre: {} -> {}",
            full_camera.center_lon,
            preview_camera.center_lon
        );
        assert!(!preview.as_bgra_frame().is_blank());

        // Settling brings the full canvas back at the same camera.
        engine
            .send(EngineCommand::Settle {
                width: 320,
                height: 200,
            })
            .expect("the engine thread is running");
        let (settled, settled_camera) = next_frame_and_status(&engine);
        assert_eq!((settled.width, settled.height), (320, 200));
        assert_eq!(settled.quality, Quality::Full);
        assert_eq!(settled_camera.center_lon, preview_camera.center_lon);
    }

    #[test]
    fn the_command_queue_reports_busy_instead_of_growing() {
        let engine = spawn_engine();
        assert_eq!(
            engine.recv_timeout(Duration::from_secs(120)),
            Some(EngineEvent::Ready)
        );

        // Fill the queue from this thread while the worker is still starting its first frame.
        let mut busy = 0;
        for _ in 0..(COMMAND_QUEUE * 4) {
            // Drain as a real host would; otherwise the bounded event channel blocks the worker
            // and the queue would fill for a reason the test is not about.
            while engine.try_recv().is_some() {}
            if engine
                .send(EngineCommand::Interact {
                    width: 320,
                    height: 200,
                    pan: (1.0, 0.0),
                    zoom: None,
                })
                .is_err()
            {
                busy += 1;
            }
        }
        assert!(busy > 0, "the bounded queue never reported itself full");

        // The engine is still alive and answers the commands it accepted.
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            while engine.try_recv().is_some() {}
            if engine
                .send(EngineCommand::Settle {
                    width: 320,
                    height: 200,
                })
                .is_ok()
            {
                break;
            }
            assert!(Instant::now() < deadline, "the engine never made room for a settle");
            std::thread::sleep(Duration::from_millis(5));
        }
        // Earlier interactions may still answer before the settle does; the settle's own frame is
        // the first full 320×200 one in the queue.
        let frame = loop {
            let frame = next_frame(&engine);
            if frame.quality == Quality::Full && (frame.width, frame.height) == (320, 200) {
                break frame;
            }
        };
        assert_eq!((frame.width, frame.height), (320, 200));
    }

    #[test]
    fn a_world_button_frame_is_a_preview_and_a_bad_command_is_reported() {
        let engine = spawn_engine();
        assert_eq!(
            engine.recv_timeout(Duration::from_secs(120)),
            Some(EngineEvent::Ready)
        );

        engine
            .send(EngineCommand::World {
                width: 320,
                height: 200,
            })
            .expect("the engine thread is running");
        let preview = next_frame(&engine);
        assert_eq!(preview.quality, Quality::Preview { divisor: 2 });

        engine
            .send(EngineCommand::Command {
                id: "no.such.command".to_string(),
                params: serde_json::json!({}),
            })
            .expect("the engine thread is running");
        loop {
            match engine.recv_timeout(Duration::from_secs(120)) {
                Some(EngineEvent::Failed { error, .. }) => {
                    assert!(error.contains("unknown command"), "{error}");
                    break;
                }
                Some(_) => continue,
                None => panic!("the engine thread stopped without reporting the failure"),
            }
        }
    }
}
