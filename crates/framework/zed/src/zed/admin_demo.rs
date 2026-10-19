//! `Help > Admin Demo`: run the topcoat admin demo **in this process** and
//! watch it from a window.
//!
//! `crates/admin` is topcoat's `demos/admin` application: a small back office
//! (users, products, orders) built from topcoat views, the topcoat router, and
//! toasty over SQLite. Its `src/lib.rs` exposes the whole start-up chain as a
//! library (`admin::prepare`, `admin::serve_prepared`, `admin::serve_on`), so
//! this module owns a Tokio runtime and a listener and nothing else — there is
//! no `admin.exe` to ship, no child process to supervise, and no shell.
//!
//! ## Lifecycle
//!
//! `Start` builds a dedicated multi-thread runtime (2 worker threads, 8 MiB
//! stacks, matching `zed::octos`), then spawns one task on it:
//!
//! 1. `TcpListener::bind(("127.0.0.1", 0))` — the port is chosen by the OS and
//!    reported back, so two runs never collide;
//! 2. `admin::prepare(<Zed data dir>/admin)` — opens SQLite, applies the
//!    embedded migrations and seeds the demo (all idempotent); a failure here
//!    is a *start* failure and is reported as [`Status::Failed`], before the
//!    window ever claims to be running;
//! 3. `Event::Listening { port }` — the window switches to
//!    [`Status::Running`] and `Open in browser` becomes available;
//! 4. `admin::serve_prepared(router, listener, shutdown)` — serves until the
//!    shutdown oneshot fires.
//!
//! `Stop` takes the shutdown sender, resolves it, and waits for the task with a
//! five-second deadline on GPUI's background executor (never on the UI thread).
//! A task that does not finish in time is reported as a stop with a
//! `last_error`; the runtime is then retired with `shutdown_background`, which
//! also cancels a server that refused to stop. Because the listener belongs to
//! that runtime, the port is free as soon as the task ends, and `Start` binds a
//! fresh one.
//!
//! Closing the window stops the server (`Drop`): the shutdown signal is sent and
//! the runtime is shut down in the background, so a closed window cannot leave
//! a loopback server running with no way to reach it.
//!
//! ## The log
//!
//! The window keeps the newest [`LOG_CAPACITY`] lines in [`LogRing`]. It records
//! this module's own lifecycle lines (bind, prepare, listening, stop, errors),
//! each also written through `log::info!`/`log::error!` with an `[admin-demo]`
//! prefix so the same sequence is in `Zed.log`. topcoat's own `tracing` output is
//! not mirrored into the ring; the footer says so.
//!
//! ## Verified shapes
//!
//! The status machine, the ring, the URL and the button enablement are pure and
//! are unit-tested below; `ZED_ADMIN_DEMO_OPEN=1` opens the window at startup
//! (the menu path without the menu) for testing on a real machine.

use std::time::Duration;

use futures::{
    FutureExt as _,
    StreamExt as _,
    channel::mpsc,
    future::{self, Either},
};
use gpui::{
    App, AppContext as _, ClipboardItem, Context, IntoElement, Render, ScrollHandle, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions, div, px, size,
};
use release_channel::ReleaseChannel;
use theme::ActiveTheme as _;
use tokio::{net::TcpListener, runtime::Runtime, task::JoinHandle};
use ui::{Button, ButtonStyle, Color, Label, LabelSize, TintColor, prelude::*};

/// `ZED_ADMIN_DEMO_OPEN` — `1`/anything but `0` opens the window as Zed starts.
///
/// The menu item's path without the menu, for verifying the window on a machine
/// where clicking through `Help` is not what is being tested. Mirrors
/// `ZED_OCTOS_OPEN_CONSOLE`.
pub const ENV_OPEN: &str = "ZED_ADMIN_DEMO_OPEN";

/// `ZED_ADMIN_DEMO_START` — press `Start` as soon as the window opens.
///
/// A scripted-verification hook, like [`ENV_OPEN`]: it runs the button's own
/// handler, so a check of "does the server really listen in this process" does
/// not need a click. Off by default.
pub const ENV_START: &str = "ZED_ADMIN_DEMO_START";

/// `ZED_ADMIN_DEMO_STOP_AFTER_MS` — press `Stop` that many milliseconds after
/// the server starts listening.
///
/// The counterpart of [`ENV_START`] for the stop half of a scripted check. Off
/// by default.
pub const ENV_STOP_AFTER: &str = "ZED_ADMIN_DEMO_STOP_AFTER_MS";

/// How long `Start` waits after the window opens before pressing itself, so the
/// window is mapped and the theme is up.
const AUTO_START_DELAY: Duration = Duration::from_millis(400);

/// How many log lines the window keeps.
pub const LOG_CAPACITY: usize = 500;

/// How long `Stop` waits for the server task before giving up on it.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// The embedded runtime's worker stack, as in `zed::octos`.
const WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

/// The demo's router type, re-exported from `admin` so this module does not
/// depend on topcoat directly.
type Router = admin::Router;

/// Where the demo's database lives: `<Zed data dir>/admin`, never the process's
/// working directory.
fn data_dir() -> std::path::PathBuf {
    paths::data_dir().join("admin")
}

/// Binds a loopback port, prepares the demo's database, and serves until the
/// shutdown oneshot resolves, reporting each step on `events`.
///
/// Split out of `AdminDemoView::start` so the whole body can be wrapped in
/// `catch_unwind`: a panic in the demo's library code becomes a `Failed` status
/// instead of a dead task.
async fn start_server(
    events: mpsc::UnboundedSender<Event>,
    demo_dir: std::path::PathBuf,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(error) => {
            let _ = events.unbounded_send(Event::Failed {
                reason: format!("could not bind 127.0.0.1: {error}"),
            });
            return;
        }
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(error) => {
            let _ = events.unbounded_send(Event::Failed {
                reason: format!("could not read the bound address: {error}"),
            });
            return;
        }
    };

    // Prepare before announcing the port: a database or migration failure is a
    // start failure, not a running server.
    let router: Router = match admin::prepare(Some(demo_dir)).await {
        Ok(router) => router,
        Err(error) => {
            let _ = events.unbounded_send(Event::Failed {
                reason: error.to_string(),
            });
            return;
        }
    };

    let _ = events.unbounded_send(Event::Listening { port });

    let result = admin::serve_prepared(router, listener, async move {
        let _ = shutdown_rx.await;
    })
    .await;
    let _ = events.unbounded_send(Event::Exited {
        error: result.err().map(|error| error.to_string()),
    });
}

/// A panic payload as text, for the `Failed` status.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "a panic without a message".to_owned()
    }
}

/// The URL the demo answers on.
pub fn url_for(port: u16) -> String {
    format!("http://127.0.0.1:{port}/")
}

/// What the window is showing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not running. `last_error` is set when the previous run ended with one.
    Stopped { last_error: Option<String> },
    /// A runtime is up and the listener is being bound / the database prepared.
    Starting,
    /// Listening on `port` in this process.
    Running { port: u16 },
    /// The shutdown signal has been sent; waiting for the task to end.
    Stopping { port: u16 },
    /// The last start failed; nothing is running.
    Failed { reason: String },
}

impl Status {
    /// The status a freshly opened window shows.
    pub fn stopped() -> Self {
        Self::Stopped { last_error: None }
    }

    /// The port the demo is (or was last) listening on, if any.
    pub fn port(&self) -> Option<u16> {
        match self {
            Self::Running { port } | Self::Stopping { port } => Some(*port),
            Self::Stopped { .. } | Self::Starting | Self::Failed { .. } => None,
        }
    }

    /// The URL for [`Status::Running`] / [`Status::Stopping`].
    pub fn url(&self) -> Option<String> {
        self.port().map(url_for)
    }

    /// The one-line state shown next to the title.
    pub fn summary(&self) -> &'static str {
        match self {
            Self::Stopped { .. } => "Stopped",
            Self::Starting => "Starting",
            Self::Running { .. } => "Running",
            Self::Stopping { .. } => "Stopping",
            Self::Failed { .. } => "Failed",
        }
    }

    /// The second line: the reason, the last error, or what is being done.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Stopped { last_error: None } => None,
            Self::Stopped {
                last_error: Some(error),
            } => Some(format!("last error: {error}")),
            Self::Starting => Some("binding a loopback port and preparing the database".to_owned()),
            Self::Running { port } => Some(format!("serving in this process on port {port}")),
            Self::Stopping { port } => Some(format!("stopping the server on port {port}")),
            Self::Failed { reason } => Some(reason.clone()),
        }
    }

    /// The colour of [`Status::summary`].
    pub fn color(&self) -> Color {
        match self {
            Self::Stopped { .. } => Color::Muted,
            Self::Starting | Self::Stopping { .. } => Color::Warning,
            Self::Running { .. } => Color::Success,
            Self::Failed { .. } => Color::Error,
        }
    }
}

/// Which buttons are enabled for a status (and a non-empty log).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buttons {
    pub start: bool,
    pub stop: bool,
    pub open_browser: bool,
    pub clear_log: bool,
    pub copy_log: bool,
}

/// The button state for `status`.
///
/// `log_is_empty` drives `Clear log` / `Copy log`: they act on the ring, so they
/// are available whenever there is something in it.
pub fn buttons_for(status: &Status, log_is_empty: bool) -> Buttons {
    let running = matches!(status, Status::Running { .. });
    let idle = matches!(status, Status::Stopped { .. } | Status::Failed { .. });
    let log = !log_is_empty;
    Buttons {
        start: idle,
        stop: running,
        open_browser: running,
        clear_log: log,
        copy_log: log,
    }
}

/// The newest `capacity` lines, oldest first.
#[derive(Debug)]
pub struct LogRing {
    capacity: usize,
    lines: std::collections::VecDeque<String>,
}

impl LogRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            lines: std::collections::VecDeque::with_capacity(capacity),
        }
    }

    /// Appends `line`, dropping the oldest line when the ring is full.
    pub fn push(&mut self, line: impl Into<String>) {
        if self.capacity == 0 {
            return;
        }
        while self.lines.len() >= self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line.into());
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }

    /// The whole ring as one string, for `Copy log`.
    pub fn joined(&self) -> String {
        let mut joined = String::new();
        for line in self.lines() {
            joined.push_str(line);
            joined.push('\n');
        }
        joined
    }
}

/// The pure part of the window: status plus log.
///
/// Every transition appends the line that explains it, so the log always
/// accounts for what the status says.
#[derive(Debug)]
pub struct DemoState {
    pub status: Status,
    pub log: LogRing,
}

impl DemoState {
    pub fn new() -> Self {
        Self {
            status: Status::stopped(),
            log: LogRing::new(LOG_CAPACITY),
        }
    }

    pub fn buttons(&self) -> Buttons {
        buttons_for(&self.status, self.log.is_empty())
    }

    /// Appends a line to the ring and to `Zed.log`.
    pub fn log(&mut self, line: impl Into<String>) {
        let line = line.into();
        log::info!("{line}");
        self.log.push(line);
    }

    /// Appends a failure line to the ring and to `Zed.log` at error level.
    fn log_error(&mut self, line: impl Into<String>) {
        let line = line.into();
        log::error!("{line}");
        self.log.push(line);
    }

    /// Start was accepted: the runtime exists but nothing is listening yet.
    pub fn begin_start(&mut self) {
        self.status = Status::Starting;
        self.log("[admin-demo] starting the embedded admin demo");
    }

    /// The task bound a port and prepared the database.
    pub fn listening(&mut self, port: u16) {
        self.status = Status::Running { port };
        self.log(format!("[admin-demo] listening on 127.0.0.1:{port}"));
    }

    /// `Stop` was accepted.
    pub fn stop_requested(&mut self) {
        if let Some(port) = self.status.port() {
            self.status = Status::Stopping { port };
            self.log(format!("[admin-demo] stopping the server on port {port}"));
        }
    }

    /// The server task ended on its own (a stop, or a failure while serving).
    pub fn stopped(&mut self, last_error: Option<String>) {
        match &last_error {
            Some(error) => self.log_error(format!("[admin-demo] stopped with an error: {error}")),
            None => self.log("[admin-demo] stopped"),
        }
        self.status = Status::Stopped { last_error };
    }

    /// The start failed; nothing is listening.
    pub fn failed(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.log_error(format!("[admin-demo] failed: {reason}"));
        self.status = Status::Failed { reason };
    }
}

impl Default for DemoState {
    fn default() -> Self {
        Self::new()
    }
}

/// What the server task reports back to the window.
#[derive(Debug)]
enum Event {
    /// Bound and ready. The window may claim [`Status::Running`].
    Listening { port: u16 },
    /// A start failure: binding, the data directory, SQLite, migrations or the
    /// seed. Nothing is listening.
    Failed { reason: String },
    /// The serve loop ended. `error` is set when it ended with one.
    Exited { error: Option<String> },
}

/// The window's root view.
pub struct AdminDemoView {
    state: DemoState,
    runtime: Option<Runtime>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    scroll: ScrollHandle,
    /// From [`ENV_STOP_AFTER`]; `None` unless a scripted check asked for it.
    auto_stop: Option<Duration>,
}

impl AdminDemoView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let auto_stop = std::env::var(ENV_STOP_AFTER)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_millis);

        if std::env::var(ENV_START).is_ok_and(|value| value != "0") {
            log::info!("[admin-demo] auto-starting ({ENV_START})");
            cx.spawn(async move |weak, cx| {
                cx.background_executor().timer(AUTO_START_DELAY).await;
                cx.update(|cx| {
                    if let Some(view) = weak.upgrade() {
                        view.update(cx, |view, cx| view.start(cx));
                    }
                });
            })
            .detach();
        }

        Self {
            state: DemoState::new(),
            runtime: None,
            shutdown: None,
            task: None,
            scroll: ScrollHandle::new(),
            auto_stop,
        }
    }

    /// Shuts the runtime down without waiting for its tasks.
    ///
    /// `shutdown_background` never blocks the GPUI thread, and cancelling the
    /// serve task is what releases the port.
    fn retire_runtime(&mut self) {
        self.shutdown = None;
        self.task = None;
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if !self.state.buttons().start {
            return;
        }

        // A previous run that refused to stop is cancelled here; the new run
        // binds a new port anyway.
        self.retire_runtime();
        self.state.begin_start();

        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("admin-demo")
            .enable_all()
            .thread_stack_size(WORKER_STACK_SIZE)
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let reason = format!("could not build the admin runtime: {error}");
                self.state.failed(reason);
                cx.notify();
                return;
            }
        };

        let demo_dir = data_dir();
        let (events, mut events_rx) = mpsc::unbounded::<Event>();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let panic_events = events.clone();
        let task = runtime.spawn(async move {
            // The demo is library code (topcoat view macros, toasty, SQLite)
            // running on this window's runtime: a panic there must become a
            // `Failed` status, not a task that dies silently and leaves the
            // window saying "Starting" forever.
            let outcome =
                std::panic::AssertUnwindSafe(start_server(events, demo_dir, shutdown_rx))
                    .catch_unwind()
                    .await;
            if let Err(panic) = outcome {
                let _ = panic_events.unbounded_send(Event::Failed {
                    reason: format!("the admin demo panicked: {}", panic_message(panic.as_ref())),
                });
            }
        });

        self.runtime = Some(runtime);
        self.shutdown = Some(shutdown_tx);
        self.task = Some(task);

        // The task reports on an unbounded channel; the window drains it on
        // GPUI's executor, so no `tokio` handle is ever awaited on the UI
        // thread.
        cx.spawn(async move |weak, cx| {
            while let Some(event) = events_rx.next().await {
                let applied = cx.update(|cx| {
                    let Some(view) = weak.upgrade() else {
                        return false;
                    };
                    view.update(cx, |view, cx| view.apply(event, cx));
                    true
                });
                if !applied {
                    break;
                }
            }
        })
        .detach();

        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.state.buttons().stop {
            return;
        }
        if self.state.status.port().is_none() {
            return;
        }

        self.state.stop_requested();

        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let Some(task) = self.task.take() else {
            self.state.stopped(None);
            self.retire_runtime();
            cx.notify();
            return;
        };

        let deadline = cx.background_executor().timer(SHUTDOWN_TIMEOUT);
        cx.spawn(async move |weak, cx| {
            // Waiting happens on GPUI's executor; the join handle is polled,
            // the runtime's workers are already draining the server.
            let ended = async move { task.await };
            let timeout = async move { deadline.await };
            futures::pin_mut!(ended, timeout);

            let result = match future::select(ended, timeout).await {
                Either::Left((joined, _)) => match joined {
                    Ok(()) => Ok(()),
                    Err(error) => Err(format!("the server task failed: {error}")),
                },
                Either::Right(((), _)) => Err(format!(
                    "timed out after {}s waiting for the server to stop",
                    SHUTDOWN_TIMEOUT.as_secs()
                )),
            };

            cx.update(|cx| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                view.update(cx, |view, cx| {
                    view.state.stopped(result.err());
                    view.retire_runtime();
                    cx.notify();
                });
            });
        })
        .detach();

        cx.notify();
    }

    fn open_in_browser(&mut self, cx: &mut Context<Self>) {
        let Some(url) = self.state.status.url() else {
            return;
        };
        log::info!("[admin-demo] opening {url}");
        cx.open_url(&url);
    }

    fn clear_log(&mut self, cx: &mut Context<Self>) {
        self.state.log.clear();
        cx.notify();
    }

    fn copy_log(&mut self, cx: &mut Context<Self>) {
        let text = self.state.log.joined();
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn apply(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Listening { port } => {
                self.state.listening(port);

                if let Some(delay) = self.auto_stop {
                    log::info!("[admin-demo] auto-stopping in {delay:?} ({ENV_STOP_AFTER})");
                    cx.spawn(async move |weak, cx| {
                        cx.background_executor().timer(delay).await;
                        cx.update(|cx| {
                            if let Some(view) = weak.upgrade() {
                                view.update(cx, |view, cx| view.stop(cx));
                            }
                        });
                    })
                    .detach();
                }
            }
            Event::Failed { reason } => {
                self.state.failed(reason);
            }
            Event::Exited { error } => {
                // A stop that the window asked for is reported by the waiter in
                // `stop`; here only an exit the window did not ask for matters.
                if matches!(self.state.status, Status::Running { .. }) {
                    match &error {
                        Some(error) => log::error!("[admin-demo] the server exited: {error}"),
                        None => log::info!("[admin-demo] the server exited"),
                    }
                    self.state.stopped(error);
                    self.retire_runtime();
                }
            }
        }
        self.scroll.scroll_to_bottom();
        cx.notify();
    }
}

impl Drop for AdminDemoView {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.retire_runtime();
    }
}

impl Render for AdminDemoView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let buttons = self.state.buttons();
        let status = self.state.status.clone();
        let status_color = status.color();
        let summary = SharedString::from(status.summary());
        let detail = status.detail().map(SharedString::from);
        let url = status.url().map(SharedString::from);
        let line_count = self.state.log.len();

        let log_rows: Vec<gpui::AnyElement> = self
            .state
            .log
            .lines()
            .map(|line| {
                Label::new(SharedString::from(line.to_owned()))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted)
                    .into_any_element()
            })
            .collect();

        v_flex()
            .id("admin-demo")
            .size_full()
            .gap_3()
            .p_4()
            .bg(cx.theme().colors().editor_background)
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(Label::new("Admin Demo").size(LabelSize::Large))
                    .child(Label::new(summary).color(status_color)),
            )
            .child(
                Label::new(detail.unwrap_or_else(|| SharedString::from("not running")))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                Label::new(url.unwrap_or_else(|| SharedString::from("http://127.0.0.1:<port>/")))
                    .size(LabelSize::Small)
                    .color(if status.port().is_some() {
                        Color::Accent
                    } else {
                        Color::Hidden
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("admin-demo-start", "Start")
                            .style(ButtonStyle::Tinted(TintColor::Accent))
                            .disabled(!buttons.start)
                            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                    )
                    .child(
                        Button::new("admin-demo-stop", "Stop")
                            .style(ButtonStyle::Outlined)
                            .disabled(!buttons.stop)
                            .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                    )
                    .child(
                        Button::new("admin-demo-open", "Open in browser")
                            .style(ButtonStyle::Outlined)
                            .disabled(!buttons.open_browser)
                            .on_click(cx.listener(|this, _, _, cx| this.open_in_browser(cx))),
                    )
                    .child(
                        Button::new("admin-demo-clear", "Clear log")
                            .style(ButtonStyle::OutlinedGhost)
                            .disabled(!buttons.clear_log)
                            .on_click(cx.listener(|this, _, _, cx| this.clear_log(cx))),
                    )
                    .child(
                        Button::new("admin-demo-copy", "Copy log")
                            .style(ButtonStyle::OutlinedGhost)
                            .disabled(!buttons.copy_log)
                            .on_click(cx.listener(|this, _, _, cx| this.copy_log(cx))),
                    ),
            )
            .child(
                Label::new(SharedString::from(format!(
                    "log: {line_count}/{LOG_CAPACITY} lines (newest last)"
                )))
                .size(LabelSize::XSmall)
                .color(Color::Muted),
            )
            .child(
                div()
                    .id("admin-demo-log")
                    .flex_1()
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_md()
                    .p_2()
                    .child(v_flex().gap_0p5().children(log_rows)),
            )
            .child(
                Label::new(
                    "The server runs in this process. Closing the window stops it. \
                     Only this window's own [admin-demo] lines are shown; the demo's \
                     tracing output goes to Zed.log.",
                )
                .size(LabelSize::XSmall)
                .color(Color::Hidden),
            )
    }
}

/// Opens the window, or focuses the one that is already open.
///
/// Rendering needs `theme::GlobalTheme` (`cx.theme()` panics without it). The
/// menu path always runs long after the theme is registered, but the
/// `ZED_ADMIN_DEMO_OPEN` startup path can run before that, so wait for it rather
/// than opening a window that cannot draw. Mirrors `zed::octos`.
pub fn open(cx: &mut App) {
    if cx.try_global::<theme::GlobalTheme>().is_none() {
        log::info!("[admin-demo] waiting for the theme before opening the window");
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
        .find_map(|window| window.downcast::<AdminDemoView>())
    {
        if let Err(error) = existing.update(cx, |_, window, _| window.activate_window()) {
            log::warn!("[admin-demo] could not focus the admin demo window: {error}");
        }
        return;
    }

    let window_size = size(px(980.), px(640.));
    let opened = cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Admin Demo".into()),
                // The system title bar; the window draws no chrome of its own.
                appears_transparent: false,
                traffic_light_position: None,
            }),
            window_bounds: Some(WindowBounds::centered(window_size, cx)),
            kind: WindowKind::Normal,
            app_id: Some(ReleaseChannel::global(cx).app_id().to_owned()),
            ..Default::default()
        },
        |_window, cx| cx.new(AdminDemoView::new).into(),
    );

    match opened {
        Ok(_) => log::info!("[admin-demo] window opened"),
        Err(error) => log::error!("[admin-demo] could not open the window: {error}"),
    }
}

/// Registers the startup override, if any. Called once from `zed::init`.
pub fn init(cx: &mut App) {
    if std::env::var(ENV_OPEN).is_ok_and(|value| value != "0") {
        log::info!("[admin-demo] opening the window at startup ({ENV_OPEN})");
        cx.defer(open);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_drops_the_oldest_lines() {
        let mut ring = LogRing::new(3);
        assert!(ring.is_empty());

        for line in ["one", "two", "three"] {
            ring.push(line);
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.lines().collect::<Vec<_>>(), ["one", "two", "three"]);

        ring.push("four");
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.lines().collect::<Vec<_>>(), ["two", "three", "four"]);
        assert_eq!(ring.joined(), "two\nthree\nfour\n");

        ring.clear();
        assert!(ring.is_empty());
        assert_eq!(ring.joined(), "");
    }

    #[test]
    fn the_ring_keeps_the_last_line_even_at_capacity_one() {
        let mut ring = LogRing::new(1);
        ring.push("old");
        ring.push("new");
        assert_eq!(ring.lines().collect::<Vec<_>>(), ["new"]);
    }

    #[test]
    fn the_state_machine_moves_between_its_states() {
        let mut state = DemoState::new();
        assert_eq!(state.status, Status::stopped());
        assert_eq!(state.status.summary(), "Stopped");
        assert_eq!(state.status.detail(), None);
        assert_eq!(state.status.url(), None);

        state.begin_start();
        assert_eq!(state.status, Status::Starting);
        assert_eq!(state.status.port(), None);

        state.listening(41234);
        assert_eq!(state.status, Status::Running { port: 41234 });
        assert_eq!(state.status.port(), Some(41234));
        assert_eq!(
            state.status.detail().as_deref(),
            Some("serving in this process on port 41234")
        );

        state.stop_requested();
        assert_eq!(state.status, Status::Stopping { port: 41234 });
        assert_eq!(state.status.summary(), "Stopping");

        state.stopped(Some("connection reset".to_owned()));
        assert_eq!(
            state.status,
            Status::Stopped {
                last_error: Some("connection reset".to_owned())
            }
        );
        assert_eq!(
            state.status.detail().as_deref(),
            Some("last error: connection reset")
        );

        state.failed("could not bind 127.0.0.1: permission denied");
        assert!(matches!(state.status, Status::Failed { .. }));
        assert_eq!(state.status.color(), Color::Error);
        assert_eq!(state.status.url(), None);

        // A new start clears the failure.
        state.begin_start();
        assert_eq!(state.status, Status::Starting);
        assert_eq!(state.status.color(), Color::Warning);

        // Every transition left its own line behind.
        let lines: Vec<&str> = state.log.lines().collect();
        assert!(lines.iter().any(|line| line.contains("starting the embedded")));
        assert!(lines
            .iter()
            .any(|line| line.contains("listening on 127.0.0.1:41234")));
        assert!(lines
            .iter()
            .any(|line| line.contains("stopping the server on port 41234")));
        assert!(lines
            .iter()
            .any(|line| line.contains("stopped with an error: connection reset")));
        assert!(lines.iter().any(|line| line.contains("failed: could not bind")));
    }

    #[test]
    fn the_url_is_the_loopback_root() {
        assert_eq!(url_for(3000), "http://127.0.0.1:3000/");
        assert_eq!(
            Status::Running { port: 51234 }.url().as_deref(),
            Some("http://127.0.0.1:51234/")
        );
        assert_eq!(
            Status::Stopping { port: 51234 }.url().as_deref(),
            Some("http://127.0.0.1:51234/")
        );
        assert_eq!(Status::Starting.url(), None);
        assert_eq!(Status::Failed { reason: "x".to_owned() }.url(), None);
    }

    #[test]
    fn buttons_follow_the_status() {
        let idle = buttons_for(&Status::stopped(), true);
        assert_eq!(
            idle,
            Buttons {
                start: true,
                stop: false,
                open_browser: false,
                clear_log: false,
                copy_log: false
            }
        );

        let starting = buttons_for(&Status::Starting, false);
        assert!(!starting.start && !starting.stop && !starting.open_browser);

        let running = buttons_for(&Status::Running { port: 3000 }, true);
        assert!(!running.start && running.stop && running.open_browser);
        // Nothing in the log yet: nothing to clear or copy.
        assert!(!running.clear_log && !running.copy_log);

        let running_with_log = buttons_for(&Status::Running { port: 3000 }, false);
        assert!(running_with_log.clear_log && running_with_log.copy_log);

        let stopping = buttons_for(&Status::Stopping { port: 3000 }, false);
        assert!(!stopping.start && !stopping.stop && !stopping.open_browser);

        let failed = buttons_for(&Status::Failed { reason: "x".to_owned() }, false);
        assert!(failed.start && !failed.stop && !failed.open_browser);

        let stopped_with_error = buttons_for(
            &Status::Stopped {
                last_error: Some("x".to_owned()),
            },
            false,
        );
        assert!(stopped_with_error.start && !stopped_with_error.open_browser);
    }

    #[test]
    fn a_fresh_state_has_nothing_to_show_and_nothing_to_do() {
        let state = DemoState::new();
        assert!(state.log.is_empty());
        assert_eq!(
            state.buttons(),
            Buttons {
                start: true,
                stop: false,
                open_browser: false,
                clear_log: false,
                copy_log: false
            }
        );
    }
}
