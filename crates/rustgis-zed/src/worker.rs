//! The engine on a thread of its own.
//!
//! `rustgis_engine::Session` is not `Send`, and a frame can take tens of milliseconds, so a host
//! that has a UI thread must not own one. [`spawn_engine`] moves a [`MapEngine`] onto a worker
//! thread and talks to it with [`EngineCommand`]s, answering with [`EngineEvent`]s:
//!
//! ```no_run
//! let mut engine = rustgis_zed::spawn_engine();
//! engine.send(rustgis_zed::EngineCommand::Render { width: 800, height: 600 });
//! // ... later, from a timer or a channel-driven task:
//! while let Some(event) = engine.try_recv() {
//!     if let rustgis_zed::EngineEvent::Frame(frame) = event {
//!         println!("{}x{}", frame.width, frame.height);
//!     }
//! }
//! ```
//!
//! Nothing here blocks: `send` hands a command over, `try_recv` looks for an answer, and dropping
//! the handle asks the thread to stop and joins it.

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;

use crate::{CameraStatus, Frame, MapEngine, Notice};

/// Something for the engine thread to do.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineCommand {
    /// Draw the map at this size and answer with [`EngineEvent::Frame`].
    Render { width: u32, height: u32 },
    /// Move the map by a screen offset (`Camera::pan_px`), then redraw at the last size.
    Pan { dx: f64, dy: f64 },
    /// Zoom by `factor` about a pixel of the last frame, then redraw.
    Zoom { x: f64, y: f64, factor: f64 },
    /// Point the camera at a lon/lat centre, then redraw.
    Center { lon: f64, lat: f64 },
    /// Change the last frame's size without moving the camera, then redraw.
    Resize { width: u32, height: u32 },
    /// Read GeoJSON into the map, then redraw.
    LoadGeoJson { name: String, bytes: Vec<u8> },
    /// Run a RustGIS command, then redraw.
    Command { id: String, params: serde_json::Value },
    /// Turn label drawing on or off, then redraw.
    SetLabels { on: bool },
    /// Stop the thread.
    Shutdown,
}

/// What the engine thread answers with. Every command answers with at least one
/// [`EngineEvent::Frame`] (or [`EngineEvent::Failed`]), followed by an [`EngineEvent::Status`].
#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    /// A drawn frame, at the size it was drawn for.
    Frame(Frame),
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
    commands: Sender<EngineCommand>,
    events: Receiver<EngineEvent>,
    thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
    /// Hand the engine a command. Fails only once the thread has stopped.
    pub fn send(&self, command: EngineCommand) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "the RustGIS engine thread has stopped".to_string())
    }

    /// A second handle to the command queue, for a task that loads a file or builds a request off
    /// the UI thread.
    pub fn sender(&self) -> Sender<EngineCommand> {
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
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<EngineEvent> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        // Best effort: if the thread already stopped, the receiver is gone and `send` fails.
        let _ = self.commands.send(EngineCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start the engine thread. The sample project is parsed there, so this returns immediately; the
/// first event is [`EngineEvent::Ready`].
pub fn spawn_engine() -> EngineHandle {
    let (commands, command_rx) = channel::<EngineCommand>();
    let (event_tx, events) = channel::<EngineEvent>();
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

fn run_engine(commands: Receiver<EngineCommand>, events: Sender<EngineEvent>) {
    let mut engine = MapEngine::new();
    if events.send(EngineEvent::Ready).is_err() {
        return;
    }
    // The size the map is drawn at, for commands that redraw at "the last size".
    let mut size = engine.size();
    while let Ok(command) = commands.recv() {
        match command {
            EngineCommand::Shutdown => break,
            EngineCommand::SetLabels { on } => engine.set_labels(on),
            EngineCommand::LoadGeoJson { name, bytes } => {
                match engine.load_geojson_named(&bytes, &name) {
                    Ok(_) => {}
                    Err(error) => {
                        send_failed(&events, "load_geojson", error);
                        continue;
                    }
                }
            }
            EngineCommand::Command { id, params } => {
                if let Err(error) = engine.command(&id, params) {
                    send_failed(&events, format!("command {id}"), error);
                    continue;
                }
            }
            EngineCommand::Pan { dx, dy } => engine.pan(dx, dy),
            EngineCommand::Zoom { x, y, factor } => engine.zoom_at(x, y, factor),
            EngineCommand::Resize { width, height } => {
                size = (width, height);
                engine.resize(width, height);
            }
            EngineCommand::Center { lon, lat } => {
                if let Err(error) = engine.center_on(lon, lat) {
                    send_failed(&events, "center", error);
                    continue;
                }
            }
            EngineCommand::Render { width, height } => size = (width, height),
        }
        // Commands that only changed a switch still redraw: the frame is the answer.
        let (width, height) = size;
        let notices = engine.take_notices();
        let camera = engine.camera_status();
        let layers = engine.layer_count();
        match engine.render_rgba(width, height) {
            Ok(frame) => {
                if events.send(EngineEvent::Frame(frame)).is_err() {
                    break;
                }
            }
            Err(error) => send_failed(&events, "render", error),
        }
        // Always: the status bar's camera and layer count come from here, and a host that threw
        // away the frame still has to see where the map ended up.
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

fn send_failed(events: &Sender<EngineEvent>, command: impl Into<String>, error: impl Into<String>) {
    let _ = events.send(EngineEvent::Failed {
        command: command.into(),
        error: error.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_engine_thread_draws_a_frame_and_answers_a_pan() {
        let engine = spawn_engine();
        assert_eq!(
            engine.recv_timeout(Duration::from_secs(120)),
            Some(EngineEvent::Ready)
        );

        engine
            .send(EngineCommand::Render {
                width: 320,
                height: 200,
            })
            .expect("the engine thread is running");
        let first = loop {
            match engine.recv_timeout(Duration::from_secs(180)) {
                Some(EngineEvent::Frame(frame)) => break frame,
                Some(EngineEvent::Failed { error, .. }) => panic!("render failed: {error}"),
                Some(_) => continue,
                None => panic!("the engine thread stopped without drawing"),
            }
        };
        assert_eq!((first.width, first.height), (320, 200));
        assert!(!first.is_blank(), "the sample project drew nothing");

        engine
            .send(EngineCommand::Pan { dx: 40.0, dy: 0.0 })
            .expect("the engine thread is running");
        let second = loop {
            match engine.recv_timeout(Duration::from_secs(180)) {
                Some(EngineEvent::Frame(frame)) => break frame,
                Some(_) => continue,
                None => panic!("the engine thread stopped without redrawing"),
            }
        };
        assert!(
            second.different_pixels(&first).unwrap_or(0) > 320 * 200 / 4,
            "a 40px pan should have redrawn a good part of the view"
        );
    }
}
