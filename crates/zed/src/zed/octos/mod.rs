//! The Octos kernel, embedded in Zed's process.
//!
//! Architecture this module is built for: **Zed is the host** — the GUI and
//! display-control framework, owning windows, panels, app records, adapters and
//! authorization — and **Octos is the agent** — owning agent execution and the
//! OUP runtime contracts. Features added later are Octos *tools*; features that
//! need a UI are Zed views on this side of the boundary. So the integration is
//! a host↔kernel seam rather than a one-off screen:
//!
//! ```text
//!   Zed (host)                                   Octos (kernel, in-process)
//!   ─────────────────────────────────────────    ────────────────────────────
//!   Help > Octos · future panels/views    ──▶    OUP methods (session/*, turn/*)
//!   zed::octos::service().connection()    ◀──▶   crates/octos/octos-cli/src/embedded.rs
//!   host peers (app tools, UI actions)    ◀──▶   peer/tool/call → result
//! ```
//!
//! What runs today: [`start`] boots an in-process kernel on a Tokio runtime Zed
//! owns, opens one session, and keeps the connection ([`connection::Connection`])
//! available to the rest of the process. `Help > Octos` opens
//! [`status_window`] — the first host-side Octos UI module, and the template for
//! the panels that follow (turn progress, approvals, tool activity).
//!
//! The frames the kernel initiates are **fanned out** as well as routed to the
//! console: [`Octos::subscribe`] hands any host-side consumer its own stream of
//! [`OctosFrame`]s, which is the seam the panels above are built on. The console
//! keeps its own path, unchanged — see [`forward_notifications`].
//!
//! The contract, from `crates/octos/octos-cli/src/embedded.rs`:
//!
//! * **8 MiB worker stacks are mandatory.** The AppUI dispatcher's poll chain is
//!   deeper than Tokio's default 2 MiB worker stack, so the runtime below is
//!   built with [`WORKER_STACK_SIZE`]. On a default runtime the first
//!   `session/open` overflows the worker stack and **aborts the process** rather
//!   than returning an error.
//! * **The host owns the runtime and the pipe.** `serve_io` serves one
//!   connection until the host drops its end; it is not a listener and does not
//!   outlive the connection.
//! * **A private home.** State lives in `<home>/.octos`, configuration in
//!   `<home>/.config/octos`. `home` defaults to `octos/` inside Zed's data
//!   directory. Without a stored main profile `serve_io` fails with "Configure
//!   the local LLM profile", which [`open`] surfaces to the user.
//! * **A session is what makes it do anything.** One `session/open` frame goes
//!   out on connect, matching the crate's own host test.
//!
//! Build cost: `octos-cli` is pulled in with `default-features = false,
//! features = ["api"]` (see the workspace manifest), so the CLI's default
//! `embed-llama` (libclang) and `impersonate` (BoringSSL/cmake) builds are not
//! part of Zed's build. A `--workspace` build unifies features with
//! `octos-cli`'s own binary and therefore does enable them, as before.
//!
//! Environment overrides:
//!
//! * `ZED_OCTOS_HOME` — the embedded kernel's home directory.
//! * `ZED_OCTOS_AUTOSTART` — `0`/`false`/`off` stops it starting with Zed.
//! * `ZED_OCTOS_OPEN_STATUS` — `1` opens the Octos window as Zed starts.

pub mod agent_thread;
pub mod connection;
pub mod key_bridge;
pub mod model_selector;
pub mod peers;
pub mod status_window;

use connection::{CallFailure, Connection};
use futures::channel::mpsc;
use futures::StreamExt as _;
use gpui::{App, Global, Task, TaskExt as _};
use octos_core::ui_protocol::{QuestionId, TurnId, TurnInterruptParams, TurnInterruptResult};
use peers::{ActivityState, ApprovalRequest, AskedQuestions, HostPeer, ToolActivity};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{runtime::Runtime, sync::broadcast};

/// `0`, `false` or `off` stops the kernel from starting with Zed.
const ENV_AUTOSTART: &str = "ZED_OCTOS_AUTOSTART";
/// The embedded kernel's home directory.
const ENV_HOME: &str = "ZED_OCTOS_HOME";
/// 1 opens the Octos window as Zed starts (the menu item's path, without the
/// menu) — handy for verifying the window on a machine where clicking is not it.
const ENV_OPEN_STATUS: &str = "ZED_OCTOS_OPEN_STATUS";
/// `0`, `false` or `off` stops this host registering its own tools
/// ([`peers`]) on the session. The kernel then offers the model no Zed tool.
const ENV_HOST_TOOLS: &str = "ZED_OCTOS_HOST_TOOLS";

/// The app id this host opens its session under.
const APP_ID: &str = "zed";
/// The session name this host opens under that app id.
const SESSION_NAME: &str = "embedded";

/// Required by `octos_cli::embedded`: the dispatcher's poll chain does not fit
/// in Tokio's default 2 MiB worker stack, and overflowing aborts the process.
/// Keep in sync with `crates/octos/octos-cli/src/embedded.rs`.
const WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

/// Capacity of the OUP duplex pipe.
const PIPE_CAPACITY: usize = 1024 * 1024;

/// How many kernel frames the fan-out ([`Octos::subscribe`]) keeps for a
/// subscriber that has fallen behind.
///
/// `broadcast` is a bounded ring: past this many unread frames the oldest are
/// dropped and the subscriber is told (`RecvError::Lagged` → [`next_frame`]'s
/// resync marker), so the number trades buffer size against how often a UI that
/// polls on the gpui thread has to resynchronize. 1024 frames is a second or
/// two of streaming deltas — a burst a slow-but-live consumer catches up on —
/// while a frame is a small `String` + `Value`, so the buffer stays in the low
/// hundreds of KiB per subscriber. It is deliberately not tiny: losing a
/// `turn/completed` costs the consumer its turn state.
const FANOUT_CAPACITY: usize = 1024;

/// How long the host waits for the kernel to open the session, register this
/// host's tools, acknowledge a turn, a decision or an interrupt.
///
/// These are the host's own patience, not the kernel's deadlines: a timeout is
/// reported as "no answer", never as "the kernel refused" and never as "the
/// operation was undone". `turn/start` gets the longest window because its ack
/// is owed after job admission on a kernel that may be loading a profile.
const SESSION_OPEN_TIMEOUT: Duration = Duration::from_secs(30);
const TOOLS_REGISTER_TIMEOUT: Duration = Duration::from_secs(30);
const TURN_START_TIMEOUT: Duration = Duration::from_secs(120);
const TURN_INTERRUPT_TIMEOUT: Duration = Duration::from_secs(30);
const APPROVAL_RESPOND_TIMEOUT: Duration = Duration::from_secs(30);
const USER_QUESTION_RESPOND_TIMEOUT: Duration = Duration::from_secs(30);

/// Where the embedded kernel is in its lifecycle.
#[derive(Default)]
enum State {
    #[default]
    Stopped,
    Starting,
    Running {
        session_id: Option<String>,
    },
    Failed {
        reason: String,
    },
}

/// What the embedded runtime reports back to the gpui thread.
enum Event {
    SessionOpened { session_id: String },
    /// The reply to `peer/tools/register`: the tools this host now offers.
    ToolsRegistered { version: u64 },
    /// The kernel refused this host's tool set; nothing Zed offers is visible
    /// to the model.
    ToolsRefused { reason: String },
    /// A frame the kernel initiated: turn progress, deltas, approvals.
    Notification { method: String, params: Value },
    /// Something worth a console line that is not a kernel failure.
    Note(String),
    Failed { reason: String },
    Closed,
}

/// One line of the Octos console.
#[derive(Clone)]
pub enum Line {
    /// What the user sent.
    You(String),
    /// What the kernel streamed back.
    Octos(String),
    /// Something the host noticed.
    Note(String),
}

impl Line {
    /// The console's rendering of this line.
    pub fn text(&self) -> String {
        match self {
            Line::You(text) => format!("you: {text}"),
            Line::Octos(text) => format!("octos: {text}"),
            Line::Note(text) => format!("· {text}"),
        }
    }
}

/// One kernel-initiated frame, as a fan-out subscriber ([`Octos::subscribe`])
/// sees it.
///
/// Deliberately the *frame* rather than a decoded [`Event`]: a subscriber reads
/// `method`/`params` with the same authoritative decoder the console uses,
/// instead of being handed a second interpretation of the protocol that can
/// drift from it.
///
/// Host-tool frames are not here at all. `peer/tool/call` and `peer/tool/cancel`
/// are the host-tool transport — the kernel blocks on `peer/tool/result` while
/// it runs the turn — and [`forward_notifications`] answers them in place on the
/// kernel's runtime. They are never fanned out, because a subscriber is by
/// definition not on that critical path.
#[derive(Debug, Clone, PartialEq)]
pub struct OctosFrame {
    pub method: String,
    pub params: Value,
}

/// The frame seam host-side Octos views are built on: [`next_frame`] and the
/// marker it hands a subscriber that fell behind are what an agent thread's
/// transcript reads (`agent_thread::deliver_frames`).
impl OctosFrame {
    /// The method of the marker [`next_frame`] hands back when a subscriber fell
    /// behind and lost frames (see [`Octos::subscribe`]).
    ///
    /// It is this host's own method, never one the kernel sends, so a subscriber
    /// can tell "the frames I missed are gone; re-read the state you track" from
    /// a real kernel frame. Losing `turn/completed` this way is exactly how a
    /// thread would stay "running" forever, which is why the loss is reported
    /// rather than swallowed.
    pub const RESYNC_METHOD: &'static str = "octos/resync";

    /// Whether this is the resync marker rather than a kernel frame.
    pub fn is_resync(&self) -> bool {
        self.method == Self::RESYNC_METHOD
    }

    /// How many frames were lost before a resync marker, if this is one. The
    /// count is the marker's only field.
    pub fn missed_frames(&self) -> Option<u64> {
        self.is_resync()
            .then(|| self.params.get("missed").and_then(Value::as_u64))
            .flatten()
    }

    /// The marker [`next_frame`] returns for a subscriber that fell behind.
    fn resync(missed: u64) -> Self {
        Self {
            method: Self::RESYNC_METHOD.to_owned(),
            params: json!({ "missed": missed }),
        }
    }
}

/// The next frame for a fan-out subscriber ([`Octos::subscribe`]), with the loss
/// a slow subscriber suffered made *visible* instead of silent.
///
/// `broadcast` is a bounded ring: a receiver that does not keep up loses the
/// oldest frames, and the only place that loss can be observed is here. So this
/// is where it is handled, and never by ignoring it:
///
/// * the number of frames lost is logged (they are gone; a subscriber must not
///   be left believing it saw everything);
/// * the subscriber is handed [`OctosFrame::RESYNC_METHOD`] with that count in
///   `missed` — the explicit "re-read the state you track" signal. A lost
///   `turn/completed` is a thread that otherwise stays in flight forever.
///
/// The frames still in the ring follow the marker, in kernel order. `None` means
/// the connection is gone: no further frame will ever arrive, so a subscriber
/// must treat it as needing resynchronization too.
pub async fn next_frame(receiver: &mut broadcast::Receiver<OctosFrame>) -> Option<OctosFrame> {
    match receiver.recv().await {
        Ok(frame) => Some(frame),
        Err(broadcast::error::RecvError::Lagged(missed)) => {
            log::warn!(
                "[octos] a frame subscriber fell {missed} frame(s) behind and lost them \
                 (a lost turn/completed would leave a thread running forever); \
                 telling it to resynchronize"
            );
            Some(OctosFrame::resync(missed))
        }
        Err(broadcast::error::RecvError::Closed) => None,
    }
}

/// The embedded Octos kernel and the runtime that drives it.
#[derive(Default)]
pub struct Octos {
    /// Shutting this down ends the embedded connection.
    runtime: Option<Runtime>,
    /// The host side of the OUP connection: the seam future features extend.
    connection: Option<Connection>,
    /// Fans the kernel's frames out to host-side subscribers
    /// ([`Octos::subscribe`]). `None` until a kernel is started, and taken back
    /// when the connection ends so that subscribers see their stream close
    /// instead of waiting on frames that cannot come.
    fanout: Option<broadcast::Sender<OctosFrame>>,
    /// This host's end of the host-tool contract ([`peers`]).
    peer: HostPeer,
    home: Option<PathBuf>,
    state: State,
    /// The console's conversation, oldest first.
    transcript: Vec<Line>,
    /// Whether a turn is in flight.
    pending: bool,
    /// The turn an interrupt would name: the id [`send_turn`] minted, updated
    /// from the `turn/start` receipt when the kernel reports it used another
    /// one (a steer into the turn already running).
    turn_id: Option<TurnId>,
    /// Forwards [`Event`]s from the runtime onto the gpui thread.
    events: Option<Task<()>>,
}

impl Global for Octos {}

/// The embedded kernel, if this process has one.
///
/// This is the entry point for host-side Octos features: call
/// `service(cx).and_then(|octos| octos.connection())` to reach the kernel.
pub fn service(cx: &App) -> Option<&Octos> {
    let octos = cx.try_global::<Octos>()?;
    octos.connection.is_some().then_some(octos)
}

impl Octos {
    /// The host side of the OUP connection to the kernel.
    pub fn connection(&self) -> Option<&Connection> {
        self.connection.as_ref()
    }

    /// Subscribes to the frames the kernel initiates — read them with
    /// [`next_frame`], which is also where a lagged subscriber is told what it
    /// lost.
    ///
    /// `None` when no kernel is running in this process: there is no frame to
    /// fan out and nothing to resynchronize against. A subscriber's stream ends
    /// (`None` from [`next_frame`]) when the connection does, which the caller
    /// must likewise treat as "the state you track is stale".
    ///
    /// This is the seam for a consumer that reaches the kernel through the
    /// global. An agent thread does not: it is handed the same
    /// `broadcast::Sender` when its connection is built, and subscribes off that
    /// (`agent_thread::OctosKernel`), so nothing in a production build calls this
    /// yet — hence the attribute rather than a caller invented to use it.
    #[allow(dead_code)]
    pub fn subscribe(&self) -> Option<broadcast::Receiver<OctosFrame>> {
        self.fanout.as_ref().map(broadcast::Sender::subscribe)
    }

    /// The kernel's home directory.
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    /// Whether a session is open.
    pub fn session_id(&self) -> Option<&str> {
        match &self.state {
            State::Running { session_id } => session_id.as_deref(),
            _ => None,
        }
    }

    /// Why the kernel is not running, if it failed.
    pub fn failure(&self) -> Option<&str> {
        match &self.state {
            State::Failed { reason } => Some(reason),
            _ => None,
        }
    }

    /// This host's end of the host-tool contract ([`peers`]).
    pub fn peer(&self) -> &HostPeer {
        &self.peer
    }

    /// The host tools the kernel is running, oldest first.
    pub fn tool_activity(&self) -> Vec<ToolActivity> {
        self.peer.activity()
    }

    /// The approvals waiting on the person, oldest first.
    pub fn approvals(&self) -> Vec<ApprovalRequest> {
        self.peer.approvals()
    }

    /// The structured questions the kernel is waiting on, oldest first.
    pub fn questions(&self) -> Vec<AskedQuestions> {
        self.peer.questions()
    }
}

/// The host peer, if this process has one. The console and any future panel go
/// through this rather than reaching into the global directly.
pub fn peer(cx: &App) -> Option<&HostPeer> {
    service(cx).map(Octos::peer)
}

/// The host tools the kernel is running.
pub fn tool_activity(cx: &App) -> Vec<ToolActivity> {
    service(cx).map(Octos::tool_activity).unwrap_or_default()
}

/// The approvals waiting on the person.
pub fn approvals(cx: &App) -> Vec<ApprovalRequest> {
    service(cx).map(Octos::approvals).unwrap_or_default()
}

/// The structured questions the kernel is waiting on, oldest first.
pub fn pending_questions(cx: &App) -> Vec<AskedQuestions> {
    service(cx).map(Octos::questions).unwrap_or_default()
}

/// Answer one approval, whether it is this host's own tool or the kernel's.
///
/// The two travel by different routes and that difference matters:
///
/// * A **host tool**'s approval was raised by this process (`confirm: app` in
///   the declaration, so the kernel raises none of its own). The decision runs
///   the tool locally and only then answers the kernel with the call's
///   outcome — no side effect precedes the answer.
/// * The **kernel's** own tool's approval belongs to the kernel's policy. The
///   host sends `approval/respond` and applies nothing itself: an
///   authorization decision it does not own must not be re-implemented here.
pub fn decide_approval(cx: &mut App, approval_id: &octos_core::ui_protocol::ApprovalId, approve: bool) {
    // Read the pending entry out of the global first: `push_note` /
    // `refresh_console` need `&mut App`, which the borrow would hold.
    let (peer, connection, request) = match cx.try_global::<Octos>() {
        Some(octos) => (
            octos.peer.clone(),
            octos.connection.clone(),
            octos.peer.approval(approval_id),
        ),
        None => return,
    };
    let Some(request) = request else {
        log::info!(
            "[octos] approval {} was already answered or cancelled",
            approval_id.0
        );
        refresh_console(cx);
        return;
    };
    let Some(connection) = connection else {
        push_note(
            cx,
            "cannot answer the approval: the embedded kernel connection is closed".to_owned(),
        );
        refresh_console(cx);
        return;
    };

    match request.source {
        peers::ApprovalSource::HostTool => {
            // `decide` matches on the call id, which is the host call's own.
            let decision = peer.decide(&request.call_id, approve, &connection);
            log::info!(
                "[octos] {} host tool call {} ({})",
                if approve { "approved" } else { "denied" },
                request.call_id,
                request.tool
            );
            push_note(
                cx,
                format!(
                    "{} {} — {decision:?}",
                    if approve { "approved" } else { "denied" },
                    request.tool
                ),
            );
        }
        peers::ApprovalSource::Kernel { .. } => {
            let params = peer.approval_respond_params(approval_id, approve);
            // Taken out before the frame goes out: a second click must not send
            // a second answer for the same approval.
            let taken = peer.take_kernel_approval(approval_id);
            let Some(params) = params else {
                log::warn!(
                    "[octos] cannot answer kernel approval {}: it is not a kernel approval",
                    approval_id.0
                );
                refresh_console(cx);
                return;
            };
            if taken.is_none() {
                refresh_console(cx);
                return;
            }
            let tool = request.tool.clone();
            let connection = connection.clone();
            log::info!(
                "[octos] {} kernel approval {} ({tool})",
                if approve { "approved" } else { "denied" },
                approval_id.0
            );
            cx.spawn(async move |cx| {
                let reply = connection
                    .call_with_timeout(
                        peers::APPROVAL_RESPOND,
                        params,
                        APPROVAL_RESPOND_TIMEOUT,
                    )
                    .await;
                cx.update(|cx| match reply {
                    Ok(frame) => match frame.get("error") {
                        Some(error) => {
                            push_note(cx, format!("the kernel refused the decision: {error}"));
                        }
                        None => push_note(
                            cx,
                            format!(
                                "{} {tool}",
                                if approve { "approved" } else { "denied" }
                            ),
                        ),
                    },
                    Err(failure) => push_note(
                        cx,
                        format!("the decision was not acknowledged: {failure}"),
                    ),
                });
                cx.update(refresh_console);
            })
            .detach();
        }
    }
    refresh_console(cx);
}

/// Starts the embedded kernel with Zed (unless disabled) and wires shutdown.
///
/// Called once from [`super::init`].
pub fn init(cx: &mut App) {
    cx.set_global(Octos::default());

    cx.on_app_quit(|cx| {
        shutdown(cx);
        async {}
    })
    .detach();

    init_host_tools(cx);

    if autostart_enabled() {
        start(cx);
    } else {
        log::info!("[octos] not starting with Zed (ZED_OCTOS_AUTOSTART)");
    }

    // The agent panel's in-process backend, published only once the kernel is
    // known to be running: a backend registered before that would offer threads
    // that can only fail to open a session.
    agent_thread::register_backend(cx);

    // The menu item's path without the menu, for verifying the window on a
    // machine where clicking it is not what you are testing.
    if env::var(ENV_OPEN_STATUS).is_ok_and(|value| value != "0") {
        cx.defer(open_status);
    }

}

/// Install the executor that runs this host's tools, and the pump that delivers
/// an approved editor open to the foreground.
///
/// The split is forced by the runtimes: a tool runs on the kernel's Tokio
/// runtime ([`forward_notifications`]), an editor tab opens on the gpui thread.
/// The channel between them carries exactly one thing — an
/// [`peers::EditorOpen`] that has already been authorized, approved and
/// checked for cancellation — so nothing that reaches the UI is a *proposal*;
/// it is a decided effect.
fn init_host_tools(cx: &mut App) {
    let (opens, mut opened) = mpsc::unbounded::<peers::EditorOpen>();
    cx.global_mut::<Octos>()
        .peer
        .with_executor(Arc::new(status_window::EditorOpenExecutor::new(move |open| {
            // The executor's own failure to hand off is the only thing this
            // can report: the receiving side logs the open itself.
            if opens.unbounded_send(open).is_err() {
                log::warn!("[octos] an approved editor open was dropped: the UI pump is gone");
            }
        })));

    // A test has no window for an open to land in, and no turn to produce one;
    // the pump would only outlive the test's app.
    if cfg!(test) {
        return;
    }

    cx.spawn(async move |cx| {
        while let Some(open) = opened.next().await {
            cx.update(|cx| apply_editor_open(cx, &open));
        }
    })
    .detach();
}

/// Open an approved file in the active workspace.
///
/// A real editor effect with no fallback pretence: if there is no workspace to
/// open it in, the open is logged and dropped — it is never reported as done.
fn apply_editor_open(cx: &mut App, open: &peers::EditorOpen) {
    let options = workspace::OpenOptions {
        visible: Some(workspace::OpenVisible::All),
        ..Default::default()
    };
    let window = cx.active_window();
    let Some(window) = window.and_then(|window| window.downcast::<workspace::MultiWorkspace>()) else {
        log::warn!(
            "[octos] approved editor open of {} has no workspace window to land in",
            open.path.display()
        );
        return;
    };
    let path = open.path.clone();
    let result = window.update(cx, |multi_workspace, window, cx| {
        let workspace = multi_workspace.workspace().clone();
        workspace.update(cx, |workspace, cx| {
            workspace.open_abs_path(path.clone(), options, window, cx)
        })
    });
    match result {
        Ok(task) => {
            let path = open.path.clone();
            task.detach_and_log_err(cx);
            log::info!("[octos] opened {} in the editor", path.display());
        }
        Err(error) => {
            log::warn!(
                "[octos] could not open {} in the editor: {error}",
                open.path.display()
            );
        }
    }
}


/// Help > Octos Status: show the Octos window (see [status_window]).
///
/// A window rather than a modal prompt: the embedded frontend has no HTTP
/// console to open, and the Octos surface is meant to sit next to the user's
/// work — the same reason later Octos panels belong here too.
pub fn open_status(cx: &mut App) {
    status_window::open(cx);
}

/// The status as one copyable block (the window's Copy button and the log).
fn status(cx: &App) -> String {
    let mut text = format!(
        "Octos {} is embedded in this Zed process (no octos subprocess).\n\n",
        octos_cli::embedded::VERSION
    );
    for (label, value) in rows(cx) {
        text.push_str(&format!("{label}: {value}\n"));
    }
    text.push('\n');
    for hint in hints(cx) {
        text.push_str(&format!("{hint}\n"));
    }
    text.trim_end().to_owned()
}

/// A one-line summary for the console's header.
pub fn state_line(cx: &App) -> String {
    let mut parts: Vec<String> = rows(cx)
        .into_iter()
        .filter(|(label, _)| label == "State" || label == "Connection")
        .map(|(_, value)| value)
        .collect();
    if is_pending(cx) {
        parts.push("turn in flight".to_owned());
    }
    parts.join("  ·  ")
}

/// This host's tool registration, as the console states it.
///
/// The three states are genuinely different and a person needs to tell them
/// apart: the host offers its tools *and the kernel accepted them* (calls will
/// arrive), the host offered them and the kernel refused (they will not), or
/// the host offered none at all ([`ENV_HOST_TOOLS`], or no session yet).
pub fn tools_line(cx: &App) -> String {
    let Some(peer) = peer(cx) else {
        return "no kernel".to_owned();
    };
    if !peer.is_registered() {
        return "not registered".to_owned();
    }
    let count = peers::host_tools().len();
    match peer.workspace() {
        Some(workspace) => format!(
            "{count} host tool(s), workspace {}",
            workspace.display()
        ),
        None => format!("{count} host tool(s)"),
    }
}

/// The window's label/value rows.
fn rows(cx: &App) -> Vec<(String, String)> {
    let octos = service(cx);
    let state = match octos {
        Some(octos) if octos.session_id().is_some() => {
            format!("running (session {})", octos.session_id().expect("checked"))
        }
        Some(octos) => match octos.failure() {
            Some(reason) => format!("failed — {reason}"),
            None => "starting".to_owned(),
        },
        None => "not running".to_owned(),
    };
    // The pipe and the session are separate: a session can fail to open while
    // the connection to the kernel stays up (and vice versa).
    let connection = match octos.and_then(Octos::connection) {
        Some(_) => "open — OUP over an in-process pipe",
        None => "closed",
    };
    let home = octos
        .and_then(|octos| octos.home().map(Path::to_path_buf))
        .unwrap_or_else(home)
        .display()
        .to_string();

    vec![
        ("Kernel".to_owned(), format!("octos {}", octos_cli::embedded::VERSION)),
        ("State".to_owned(), state),
        ("Connection".to_owned(), connection.to_owned()),
        ("Host tools".to_owned(), tools_line(cx)),
        ("Home".to_owned(), home),
        ("Runs as".to_owned(), "a library in this process".to_owned()),
    ]
}

/// What the user can do about the state above.
fn hints(cx: &App) -> Vec<String> {
    let mut hints = Vec::new();
    if let Some(reason) = service(cx).and_then(Octos::failure) {
        if reason.to_lowercase().contains("profile") {
            hints.push(format!(
                "A session needs a stored Octos main profile in the home above: \
                 cargo run -p octos-cli -- init --cwd <dir> writes one, or point \
                 {ENV_HOME} at a directory that already has one."
            ));
        } else {
            hints.push(
                "The kernel's own reason is in Zed.log, on the [octos] lines.".to_owned(),
            );
        }
    }
    hints.push(format!(
        "{ENV_AUTOSTART}=0 stops Octos starting with Zed; {ENV_OPEN_STATUS}=1 opens \
         this window at startup."
    ));
    hints
}

/// Spawns the embedded kernel, if it is not already starting or running.
fn start(cx: &mut App) {
    if cx.global::<Octos>().runtime.is_some() {
        return;
    }

    let home = home();
    if let Err(error) = fs::create_dir_all(&home) {
        let reason = format!("could not create {}: {error}", home.display());
        log::error!("[octos] {reason}");
        cx.global_mut::<Octos>().state = State::Failed { reason };
        return;
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("octos-embedded")
        .enable_all()
        .thread_stack_size(WORKER_STACK_SIZE)
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let reason = format!("could not build the Octos runtime: {error}");
            log::error!("[octos] {reason}");
            cx.global_mut::<Octos>().state = State::Failed { reason };
            return;
        }
    };

    let (events, mut events_rx) = mpsc::unbounded();
    let (host, kernel) = tokio::io::duplex(PIPE_CAPACITY);

    // Both tasks belong to the kernel's runtime; entering it lets `Connection`
    // spawn its reader and writer there.
    let (connection, notifications) = {
        let _entered = runtime.enter();
        Connection::new(host)
    };

    // The fan-out lane: every host-side consumer of the kernel's frames gets its
    // own subscription off this sender. The console's own lane is separate and
    // unchanged — `forward_notifications` writes to both, in that order.
    let (fanout, _) = broadcast::channel::<OctosFrame>(FANOUT_CAPACITY);

    log::info!("[octos] starting the embedded kernel (home {})", home.display());
    let kernel_home = home.clone();
    let kernel_events = events.clone();
    runtime.spawn(async move {
        let (reader, writer) = tokio::io::split(kernel);
        match octos_cli::embedded::serve_io(&kernel_home, reader, writer).await {
            Ok(()) => log::info!("[octos] kernel connection closed cleanly"),
            Err(error) => {
                // Report *why*. A fresh home has no stored LLM profile yet, and
                // that message is the actionable part for the user.
                let reason = format!("{error:#}");
                log::error!("[octos] kernel stopped: {reason}");
                let _ = kernel_events.unbounded_send(Event::Failed { reason });
            }
        }
    });

    let cwd = env::current_dir().unwrap_or_else(|_| home.clone());
    let peer = cx.global::<Octos>().peer.clone();
    runtime.spawn(open_session(
        connection.clone(),
        peer.clone(),
        cwd,
        events.clone(),
    ));
    runtime.spawn(forward_notifications(
        notifications,
        connection.clone(),
        peer,
        events.clone(),
        fanout.clone(),
    ));

    let events_task = cx.spawn(async move |cx| {
        while let Some(event) = events_rx.next().await {
            cx.update(|cx| apply_event(cx, event));
        }
    });

    let octos = cx.global_mut::<Octos>();
    octos.home = Some(home);
    octos.connection = Some(connection);
    octos.fanout = Some(fanout);
    octos.runtime = Some(runtime);
    octos.state = State::Starting;
    octos.events = Some(events_task);
}

/// Stops the embedded kernel. Wired to app quit.
pub fn shutdown(cx: &mut App) {
    let octos = cx.global_mut::<Octos>();
    octos.events.take();
    // Dropping the connection closes the host's end of the pipe, which is what
    // `serve_io` observes as the end of its connection.
    octos.connection.take();
    // Nothing will be fanned out again either: taking the sender ends every
    // subscriber's stream (`RecvError::Closed` → `None` from `next_frame`)
    // instead of leaving a subscriber waiting on frames that cannot come.
    octos.fanout.take();
    // A registration belongs to the pipe it was made on: once the pipe is gone
    // the kernel cannot call this host any more, so nothing may stay pending.
    octos.peer.reset();
    if let Some(runtime) = octos.runtime.take() {
        // Do not wait: shutting down in the background keeps a slow drain off
        // the quit path.
        runtime.shutdown_background();
        log::info!("[octos] embedded kernel stopped");
    }
    octos.state = State::Stopped;
}

/// Opens this host's session and, once it is open, registers this host's tools
/// on it ([`peers`]).
///
/// The two steps are sequential on purpose: a tool set registered before the
/// session exists would name a session the kernel has not admitted, and a
/// failure to register must not look like a failure to open the session.
async fn open_session(
    connection: Connection,
    peer: HostPeer,
    cwd: PathBuf,
    events: mpsc::UnboundedSender<Event>,
) {
    let session = session_key();
    let params = session_open_params(&session, &cwd);
    log::info!("[octos] opening session {session}");

    match connection
        .call_with_timeout(
            octos_core::ui_protocol::methods::SESSION_OPEN,
            params,
            SESSION_OPEN_TIMEOUT,
        )
        .await
    {
        Ok(frame) => match frame.get("error") {
            Some(error) => {
                let _ = events.unbounded_send(Event::Failed {
                    reason: error.to_string(),
                });
            }
            None => {
                let _ = events.unbounded_send(Event::SessionOpened {
                    session_id: frame["result"]["opened"]["session_id"].to_string(),
                });
                register_host_tools(connection, peer, session, cwd, events).await;
            }
        },
        Err(failure) => {
            log::error!("[octos] session/open did not complete: {failure}");
            let _ = events.unbounded_send(Event::Failed {
                reason: format!(
                    "{failure} — the kernel's own reason, if it gave one, is in Zed.log \
                     on the [octos] lines"
                ),
            });
        }
    }
}

/// `peer/tools/register` for this host's session.
///
/// Best-effort by design: the kernel owns whether it accepts a set, and a host
/// with no tools is a working host. Every outcome — acceptance, refusal, a
/// closed pipe — is logged and reported, never silently dropped.
async fn register_host_tools(
    connection: Connection,
    peer: HostPeer,
    session: octos_core::SessionKey,
    workspace: PathBuf,
    events: mpsc::UnboundedSender<Event>,
) {
    if !host_tools_enabled() {
        log::info!("[octos] not registering host tools ({ENV_HOST_TOOLS})");
        return;
    }
    let params = HostPeer::register_params(&session.0);
    log::info!(
        "[octos] registering {} host tool(s) on session {}",
        peers::host_tools().len(),
        session
    );

    let reply = connection
        .call_with_timeout(
            peers::PEER_TOOLS_REGISTER,
            params,
            TOOLS_REGISTER_TIMEOUT,
        )
        .await;
    match reply {
        Ok(frame) => match frame.get("error") {
            Some(error) => {
                let reason = peer.registration_failed(error);
                log::error!("[octos] the kernel refused this host's tools: {reason}");
                let _ = events.unbounded_send(Event::ToolsRefused { reason });
            }
            None => {
                let result = frame.get("result").cloned().unwrap_or(Value::Null);
                match peer.registered(&session.0, workspace, &result) {
                    Ok(version) => {
                        log::info!(
                            "[octos] host tools registered ({} tools, version {version})",
                            peers::host_tools().len()
                        );
                        let _ = events.unbounded_send(Event::ToolsRegistered { version });
                    }
                    Err(reason) => {
                        log::error!("[octos] the kernel's register reply is unusable: {reason}");
                        let _ = events.unbounded_send(Event::Failed { reason });
                    }
                }
            }
        },
        Err(failure) => {
            // A timeout or a closed pipe here is not a session failure: the
            // session is open and usable, the model simply sees no Zed tools.
            // It is still reported, never silently dropped.
            log::warn!(
                "[octos] this host's tools were not registered ({failure}); \
                 the model will see no Zed tools"
            );
            let _ = events.unbounded_send(Event::Note(format!(
                "host tools were not registered: {failure}"
            )));
        }
    }
}

/// Whether this host registers its tools on the session. Tests never do.
fn host_tools_enabled() -> bool {
    !matches!(
        env::var(ENV_HOST_TOOLS).as_deref(),
        Ok("0" | "false" | "off")
    )
}

/// Kernel-initiated frames: the kernel's own notifications, and the two
/// host-tool frames only this host can serve.
///
/// The host-tool frames are answered **here**, on the kernel's runtime, not on
/// the gpui thread: the kernel is blocking on `peer/tool/result` while it runs
/// the turn, and routing a tool call through the UI queue would put the editor's
/// frame rate on the critical path of every tool the agent runs.
///
/// `peer/tool/call` and `peer/tool/cancel` are *not* `UiNotification`s —
/// `UiNotification::from_method_and_params`
/// (`crates/octos/octos-core/src/ui_protocol.rs:7301`) has no arm for them, because
/// they are the host-tool transport rather than UI protocol payloads. They are
/// decoded with their own typed params ([`peers::HostPeer::decode_call`]),
/// still never by reading keys by hand. Every other frame goes to the console
/// through the same authoritative decoder it always used.
///
/// Two lanes, in a fixed order per frame: host-tool frames are served in place
/// and stop there; every other frame is fanned out to subscribers
/// ([`Octos::subscribe`]) **first**, then sent to the console exactly as before.
async fn forward_notifications(
    mut notifications: mpsc::UnboundedReceiver<Value>,
    connection: Connection,
    peer: HostPeer,
    events: mpsc::UnboundedSender<Event>,
    fanout: broadcast::Sender<OctosFrame>,
) {
    while let Some(frame) = notifications.next().await {
        let method = frame
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("-")
            .to_owned();
        let params = frame.get("params").cloned().unwrap_or(Value::Null);

        if method == peers::PEER_TOOL_CALL {
            match HostPeer::decode_call(&params) {
                Ok(call) => {
                    log::info!(
                        "[octos] peer/tool/call {} ({}) on {}",
                        call.name,
                        call.call_id,
                        call.session_id
                    );
                    peer.handle_call(&connection, &call);
                }
                // A frame this host cannot read is a kernel/host protocol
                // disagreement. It is logged verbatim and *dropped*: guessing
                // at a call's fields is how a host runs a tool the kernel did
                // not ask for.
                Err(error) => {
                    log::error!("[octos] unreadable peer/tool/call: {error:?}; frame {params}");
                }
            }
            continue;
        }
        if method == peers::PEER_TOOL_CANCEL {
            match HostPeer::decode_cancel(&params) {
                Ok(cancel) => peer.handle_cancel(&cancel),
                Err(error) => {
                    log::error!("[octos] unreadable peer/tool/cancel: {error:?}; frame {params}");
                }
            }
            continue;
        }

        // Fan out first, then the console, so both lanes see the kernel's order.
        //
        // This `Err` is **not** a failure: it means "no subscriber is listening
        // right now" — the normal state of a process nobody has subscribed in —
        // and it must be ignored. Only the console's own send below means the
        // routing is over; treating a missing subscriber as a broken connection
        // would stop routing frames to the console the moment nobody watched.
        // (A subscriber that falls behind is a different case entirely and is
        // reported by `next_frame`, never here.)
        let _ = fanout.send(OctosFrame {
            method: method.clone(),
            params: params.clone(),
        });

        if events
            .unbounded_send(Event::Notification { method, params })
            .is_err()
        {
            return;
        }
    }

    // The connection is over, so this lane ends here. Dropping this sender is
    // half of what closes every subscriber's stream — the other half is the
    // sender the global still holds, which `apply_event`'s `Event::Closed` arm
    // takes. A subscriber sees `RecvError::Closed` (and `next_frame` `None`)
    // rather than waiting forever on a connection that will send no more.
    drop(fanout);
    let _ = events.unbounded_send(Event::Closed);
}

fn apply_event(cx: &mut App, event: Event) {
    // Kernel-initiated frames append to the transcript, which needs `&mut App`
    // rather than the borrow of the kernel global the state arms below hold.
    if matches!(&event, Event::Notification { .. }) {
        if let Event::Notification { method, params } = event {
            apply_notification(cx, &method, &params);
            refresh_console(cx);
        }
        return;
    }

    let octos = cx.global_mut::<Octos>();
    let mut note = None;
    match event {
        Event::SessionOpened { session_id } => {
            log::info!("[octos] session open");
            octos.state = State::Running {
                session_id: Some(session_id),
            };
        }
        Event::ToolsRegistered { version } => {
            log::info!("[octos] host tools registered (version {version})");
            note = Some(format!(
                "host tools registered ({} tool(s), version {version})",
                peers::host_tools().len()
            ));
        }
        Event::ToolsRefused { reason } => {
            log::error!("[octos] host tools refused: {reason}");
            note = Some(format!("host tools refused: {reason}"));
        }
        Event::Failed { reason } => {
            log::error!("[octos] {reason}");
            octos.state = State::Failed { reason };
        }
        Event::Note(message) => {
            note = Some(message);
        }
        Event::Closed => {
            log::info!("[octos] embedded connection closed");
            // The registration outlives nothing: a closed pipe can never be
            // called on again, so every pending call is dead — and so is every
            // question the kernel was waiting on.
            octos.peer.reset();
            // Nothing will be fanned out again on this connection either:
            // dropping the last sender is what ends every subscriber's stream
            // (`forward_notifications` drops its own clone as it returns), so a
            // subscriber is told the stream is over instead of waiting on
            // frames that cannot come.
            octos.fanout.take();
            // Nothing can still be running on a connection that ended, and no
            // terminal frame will ever arrive to say so.
            octos.pending = false;
            octos.turn_id = None;
            // Keep the reason a failure reported.
            if !matches!(octos.state, State::Failed { .. }) {
                octos.state = State::Stopped;
            }
        }
        Event::Notification { .. } => {}
    }
    if let Some(note) = note {
        push_note(cx, note);
    }
}

/// What one kernel-initiated frame means for the console.
///
/// Kept separate from [`apply_notification`] so the reading of a frame is a
/// pure function of its method and params — testable without a gpui `App`.
#[derive(Debug, PartialEq)]
enum ConsoleAction {
    /// Streamed answer text belonging to the turn in flight.
    Append(String),
    /// A tool the kernel is running (its own tools — `shell`, file edits, …).
    /// `None` state means the tool finished; `success` carries the outcome.
    KernelTool {
        tool_call_id: String,
        tool: String,
        running: bool,
        /// The completion's one-line preview, or the progress message.
        detail: String,
        success: Option<bool>,
    },
    /// The kernel parked a turn on an approval. The person answers it here
    /// (`approval/respond`), because the kernel — not the host — owns what the
    /// approval is for.
    KernelApproval(Box<peers::KernelApproval>),
    /// That approval is over: decided or cancelled. Ids only.
    KernelApprovalClosed {
        approval_id: octos_core::ui_protocol::ApprovalId,
    },
    /// The kernel parked a turn on a structured question (UPCR-2026-023). The
    /// person answers it here (`user_question/respond`); like an approval, the
    /// kernel owns what is being asked and this host only renders and answers
    /// it.
    KernelQuestion(Box<AskedQuestions>),
    /// Turn progress the console shows as a note.
    Note(String),
    /// The turn ended; the console is no longer waiting on it.
    Finished,
    /// The turn failed; the string is what to show the user.
    Failed(String),
    /// A frame this host does not render yet.
    Ignore,
}

/// The authoritative reading of one kernel-initiated frame.
///
/// The field names are **decoded, not guessed**: `params` goes through the
/// kernel's own notification dispatcher, so the wire shape stays
/// `octos-core`'s contract instead of a list of spellings this host hopes for.
/// The mapping is `UiNotification::from_method_and_params`
/// (`crates/octos/octos-core/src/ui_protocol.rs:7301`), which is also what produces
/// these frames (`UiNotification::into_rpc_notification`, same file `:7180`,
/// serializes each payload straight into `params`). The payloads:
///
/// * `MessageDelta` / `ReasoningDelta` — `MessageDeltaEvent` /
///   `ReasoningDeltaEvent` (`:5375` / `:5384`), both `text: String`. A delta
///   frame carries **no** `delta` or `content` field.
/// * `ToolStarted` / `ToolProgress` / `ToolCompleted` — `ToolStartedEvent` /
///   `ToolProgressEvent` / `ToolCompletedEvent` (`:5455` / `:5473` / `:5487`):
///   `tool_call_id`, `tool_name`, and on completion `success` +
///   `output_preview` + `duration_ms`. Progress carries `message` and
///   `progress_pct`, never a tool name.
/// * `ApprovalRequested` — `ApprovalRequestedEvent` (`:5690`): `approval_id`,
///   `tool_name`, `title`, `body`, optional `risk` and `typed_details`.
/// * `ApprovalDecided` / `ApprovalCancelled` — (`:5759` / `:5808`): ids and the
///   decision, no title/body.
/// * `TurnCompleted` — `TurnCompletedEvent` (`:6498`); no text at all.
/// * `TurnError` — `TurnErrorEvent` (`:6538`), whose reason is `message`
///   (with `code`), *not* `text`.
fn console_action(method: &str, params: &Value) -> ConsoleAction {
    use octos_core::ui_protocol::UiNotification;

    let notification = match UiNotification::from_method_and_params(method, params.clone()) {
        Ok(notification) => notification,
        // Not a ui_protocol notification this build knows (the raw frame is
        // logged by `apply_notification`). Nothing to render, by design.
        Err(_) => return ConsoleAction::Ignore,
    };

    match notification {
        UiNotification::MessageDelta(event) => ConsoleAction::Append(event.text),
        UiNotification::ReasoningDelta(event) => ConsoleAction::Append(event.text),
        UiNotification::ToolStarted(event) => ConsoleAction::KernelTool {
            tool_call_id: event.tool_call_id,
            tool: event.tool_name,
            running: true,
            detail: String::new(),
            success: None,
        },
        UiNotification::ToolProgress(event) => ConsoleAction::KernelTool {
            tool_call_id: event.tool_call_id,
            // A progress frame carries no tool name: the entry it belongs to
            // already exists (a lifecycle that started with `tool/started`).
            tool: String::new(),
            running: true,
            detail: event.message.unwrap_or_default(),
            success: None,
        },
        UiNotification::ToolCompleted(event) => ConsoleAction::KernelTool {
            tool_call_id: event.tool_call_id,
            tool: event.tool_name,
            running: false,
            detail: event.output_preview.unwrap_or_default(),
            success: event.success,
        },
        UiNotification::ApprovalRequested(event) => {
            ConsoleAction::KernelApproval(Box::new(peers::KernelApproval::from_event(&event)))
        }
        UiNotification::ApprovalDecided(event) => ConsoleAction::KernelApprovalClosed {
            approval_id: event.approval_id,
        },
        UiNotification::ApprovalCancelled(event) => ConsoleAction::KernelApprovalClosed {
            approval_id: event.approval_id,
        },
        // UPCR-2026-023. The connection advertises `user_question.v1`
        // (`stdio_defaults().user_question_v1 == true`,
        // `crates/octos/octos-cli/src/api/ui_protocol_transport.rs:2436`), the
        // kernel installs the requester on that basis (`:41129`), and the wait
        // has no deadline of its own (`:6952`) — so a frame that lands here and
        // is *ignored* parks the turn forever. It is rendered and answered.
        UiNotification::UserQuestionRequested(event) => {
            ConsoleAction::KernelQuestion(Box::new(AskedQuestions::from_event(&event)))
        }
        UiNotification::ProgressUpdated(event) => ConsoleAction::Note(match event.metadata.label {
            Some(label) => label,
            None => match event.metadata.message {
                Some(message) => message,
                None => event.metadata.kind,
            },
        }),
        UiNotification::Warning(event) => {
            ConsoleAction::Note(format!("warning [{}]: {}", event.code, event.message))
        }
        UiNotification::TurnCompleted(_) => ConsoleAction::Finished,
        UiNotification::TurnError(event) => {
            ConsoleAction::Failed(format!("turn failed [{}]: {}", event.code, event.message))
        }
        // The canonical projection lane. Since #2318 the kernel suppresses the
        // raw `tool/*` lifecycle frames for a connection that negotiated
        // `projection.envelope.v1` (see `send_raw_notification_ephemeral`,
        // `crates/octos/octos-cli/src/api/ui_protocol_transport.rs:46984`), so a
        // client that only reads the raw lane sees no tool activity at all.
        // This is the same information, decoded from its own type.
        UiNotification::EnvelopeV2(event) => envelope_action(&event.envelope.payload),
        _ => ConsoleAction::Ignore,
    }
}

/// What one `projection/envelope` payload means for the console.
///
/// Only the payloads that carry something the console renders are mapped; the
/// rest are ignored rather than guessed at.
///
/// **The envelope is the answer lane, not a copy of it.** `message/delta` never
/// reaches this host: `live_event_passes_capability_filter`
/// (`crates/octos/octos-cli/src/api/ui_protocol_transport.rs:23318`, the
/// `MessageDelta | ReasoningDelta | ToolStarted | …` arm at `:23337`) drops
/// every raw source record on the wire for *every* connection — "every
/// connection is a canonical v2 consumer" — and the ephemeral direct-send path
/// applies the same filter (`send_notification_ephemeral`, same file `:47801`).
/// So an `assistant_delta` that is not rendered here is an answer the person
/// never sees. (The same holds for the raw `tool/*` lifecycle above.)
fn envelope_action(payload: &octos_core::ui_protocol::PayloadV2) -> ConsoleAction {
    use octos_core::ui_protocol::{EnvelopeToolEndStatus, PayloadV2, TurnTerminalOutcome};

    match payload {
        PayloadV2::AssistantDelta { text, .. } => ConsoleAction::Append(text.clone()),
        // The thinking lane is on the envelope for the same reason the answer
        // is; the raw `reasoning/delta` is filtered out with `message/delta`.
        PayloadV2::ReasoningDelta { text } => ConsoleAction::Append(text.clone()),
        // **Not rendered.** `assistant_persisted` carries the same text its own
        // deltas already streamed, so appending it would print the answer
        // twice.
        PayloadV2::AssistantPersisted { .. } => ConsoleAction::Ignore,
        PayloadV2::ToolStart {
            tool_call_id,
            name,
            ..
        } => ConsoleAction::KernelTool {
            tool_call_id: tool_call_id.clone(),
            tool: name.clone(),
            running: true,
            detail: String::new(),
            success: None,
        },
        PayloadV2::ToolProgress {
            tool_call_id,
            message,
        } => ConsoleAction::KernelTool {
            tool_call_id: tool_call_id.clone(),
            // The payload carries no name: the entry this belongs to was
            // opened by its own `tool_start`.
            tool: String::new(),
            running: true,
            detail: message.clone(),
            success: None,
        },
        PayloadV2::ToolEnd {
            tool_call_id,
            status,
            error,
            output_preview,
            ..
        } => ConsoleAction::KernelTool {
            tool_call_id: tool_call_id.clone(),
            tool: String::new(),
            running: false,
            detail: output_preview
                .clone()
                .or_else(|| error.clone())
                .unwrap_or_default(),
            success: Some(matches!(status, EnvelopeToolEndStatus::Complete)),
        },
        PayloadV2::TurnTerminal { outcome, error, .. } => match outcome {
            TurnTerminalOutcome::Completed => ConsoleAction::Finished,
            TurnTerminalOutcome::Errored => ConsoleAction::Failed(match error {
                Some(error) => format!("turn failed [{}]: {}", error.code, error.message),
                None => "the turn failed".to_owned(),
            }),
            TurnTerminalOutcome::Interrupted => {
                ConsoleAction::Note("the turn was interrupted".to_owned())
            }
            TurnTerminalOutcome::RateLimited => {
                ConsoleAction::Note("the provider rate-limited the turn".to_owned())
            }
        },
        _ => ConsoleAction::Ignore,
    }
}

/// Applies one kernel-initiated frame to the console's transcript.
fn apply_notification(cx: &mut App, method: &str, params: &Value) {
    // Every frame is kept in the log verbatim, so a disagreement between this
    // host's reading and the kernel's wire shape stays diagnosable from
    // Zed.log. The decision itself never reads fields off this JSON.
    log::debug!("[octos] kernel frame {method}: {params}");

    match console_action(method, params) {
        ConsoleAction::Append(text) => append_to_answer(cx, &text),
        ConsoleAction::KernelTool {
            tool_call_id,
            tool,
            running,
            detail,
            success,
        } => {
            // The kernel's own tools are not this host's to run; they are
            // recorded on the same activity list so the person sees one story.
            let state = if running {
                ActivityState::Running
            } else if success.unwrap_or(false) {
                ActivityState::Done
            } else {
                ActivityState::Failed
            };
            // A host tool the kernel started is reported under the model's
            // alias; the call the host tracked has the dotted name, which is
            // what a person recognises.
            let tool = match peer(cx).and_then(|peer| peer.pending_tool(&tool_call_id)) {
                Some(dotted) => dotted,
                None => tool,
            };
            let note = if running {
                String::new()
            } else {
                detail.clone()
            };
            if let Some(peer) = peer(cx) {
                peer.note_kernel_tool(&tool_call_id, &tool, state, note);
            }
            refresh_console(cx);
        }
        ConsoleAction::KernelApproval(approval) => {
            log::info!(
                "[octos] the kernel is asking about {} (approval {})",
                approval.tool_name,
                approval.approval_id.0
            );
            if let Some(peer) = peer(cx) {
                peer.note_kernel_approval(&approval);
            }
            refresh_console(cx);
        }
        ConsoleAction::KernelApprovalClosed { approval_id } => {
            if let Some(peer) = peer(cx) {
                peer.forget_kernel_approval(&approval_id);
            }
            refresh_console(cx);
        }
        ConsoleAction::KernelQuestion(asked) => {
            log::info!(
                "[octos] the kernel is waiting on a question ({}): {}",
                asked.question_id.0,
                asked.title
            );
            if let Some(peer) = peer(cx) {
                peer.note_kernel_question(&asked);
            }
            refresh_console(cx);
        }
        ConsoleAction::Note(note) => push_note(cx, note),
        ConsoleAction::Finished => {
            set_pending(cx, false);
            clear_turn_id(cx);
            drop_questions(cx, "the turn ended");
        }
        ConsoleAction::Failed(reason) => {
            push_note(cx, reason);
            set_pending(cx, false);
            clear_turn_id(cx);
            drop_questions(cx, "the turn failed");
        }
        ConsoleAction::Ignore => {}
    }
}

/// The console's transcript, oldest first.
pub fn transcript(cx: &App) -> Vec<Line> {
    cx.try_global::<Octos>()
        .map(|octos| octos.transcript.clone())
        .unwrap_or_default()
}

/// Whether a turn can be sent: a kernel with an open session.
pub fn can_send(cx: &App) -> bool {
    service(cx).is_some_and(|octos| octos.session_id().is_some())
}

/// Whether a turn is in flight.
pub fn is_pending(cx: &App) -> bool {
    cx.try_global::<Octos>().is_some_and(|octos| octos.pending)
}

/// Sends `text` to the embedded kernel as one turn.
///
/// The reply to `turn/start` only acknowledges the turn; the answer streams
/// back as `projection/envelope` `assistant_delta` payloads, which
/// [`apply_notification`] appends to the console.
///
/// The id this host mints is remembered as *the* turn in flight, because that
/// is what a later `turn/interrupt` has to name. The reply can override it: a
/// kernel that merged the input into a turn already running answers with that
/// turn's id and `steered: true` (see [`turn_start_receipt`]), and the console
/// says so instead of presenting it as a new conversation.
pub fn send_turn(cx: &mut App, text: String) {
    let preview: String = text.chars().take(80).collect();
    log::info!("[octos] prompt submitted: {preview}");

    let Some(connection) = service(cx).and_then(Octos::connection).cloned() else {
        log::warn!("[octos] cannot send: the embedded kernel is not running");
        push_note(cx, "the embedded kernel is not running".to_owned());
        refresh_console(cx);
        return;
    };
    if !can_send(cx) {
        log::warn!("[octos] cannot send: no session is open");
        push_note(
            cx,
            "no session is open — configure the Octos main profile first".to_owned(),
        );
        refresh_console(cx);
        return;
    }

    let (turn_id, params) = match turn_start_params(&text) {
        Ok(params) => params,
        Err(error) => {
            push_note(cx, format!("could not encode the turn: {error}"));
            refresh_console(cx);
            return;
        }
    };

    if cx.try_global::<Octos>().is_some() {
        let octos = cx.global_mut::<Octos>();
        octos.transcript.push(Line::You(text));
        // The answer streams into this line.
        octos.transcript.push(Line::Octos(String::new()));
        octos.pending = true;
        // Until the receipt says otherwise, the turn an interrupt names is the
        // one this host asked for.
        octos.turn_id = Some(turn_id.clone());
    }
    refresh_console(cx);

    cx.spawn(async move |cx| {
        let reply = connection
            .call_with_timeout(
                octos_core::ui_protocol::methods::TURN_START,
                params,
                TURN_START_TIMEOUT,
            )
            .await;
        match reply {
            Ok(frame) => {
                if let Some(error) = frame.get("error") {
                    let running = running_turn_from_refusal(error);
                    cx.update(|cx| {
                        log::warn!("[octos] turn refused: {error}");
                        push_note(cx, format!("turn refused: {error}"));
                        if let Some(turn_id) = running {
                            // The kernel refused because a turn is already
                            // running, and it named it. A refusal is not "the
                            // session is idle": that turn stays the one Stop/Esc
                            // interrupts, so the console keeps it in flight.
                            if cx.try_global::<Octos>().is_some() {
                                cx.global_mut::<Octos>().turn_id = Some(turn_id);
                            }
                            set_pending(cx, true);
                            push_note(
                                cx,
                                "a turn is already running for this session — \
                                 Stop (Esc) interrupts it"
                                    .to_owned(),
                            );
                        } else {
                            set_pending(cx, false);
                            clear_turn_id(cx);
                        }
                        refresh_console(cx);
                    });
                    return;
                }
                let receipt = turn_start_receipt(&frame, &turn_id);
                cx.update(|cx| {
                    let note = if cx.try_global::<Octos>().is_some() {
                        let octos = cx.global_mut::<Octos>();
                        apply_turn_receipt(&receipt, &mut octos.turn_id)
                    } else {
                        None
                    };
                    if let Some(note) = note {
                        push_note(cx, note);
                    }
                    refresh_console(cx);
                });
            }
            Err(failure) => cx.update(|cx| {
                // A timeout is deliberately NOT treated as a refusal: the
                // kernel may be running the turn right now, and only its own
                // terminal frame can settle that. The turn stays in flight so
                // Stop/Esc remains available.
                log::warn!("[octos] no turn/start acknowledgement: {failure}");
                push_note(cx, format!("no turn acknowledgment: {failure}"));
                if matches!(failure, CallFailure::Closed { .. }) {
                    set_pending(cx, false);
                    clear_turn_id(cx);
                }
                refresh_console(cx);
            }),
        }
    })
    .detach();
}

/// `turn/start` parameters, built from the OUP types so their wire shape stays
/// the kernel's business rather than this host's. Returns the minted turn id
/// with them: it is what an interrupt has to name.
fn turn_start_params(text: &str) -> serde_json::Result<(TurnId, Value)> {
    let turn_id = TurnId::new();
    let params = json!({
        "session_id": serde_json::to_value(session_key())?,
        "turn_id": serde_json::to_value(&turn_id)?,
        "input": [serde_json::to_value(octos_core::ui_protocol::InputItem::Text {
            text: text.to_owned(),
        })?],
    });
    Ok((turn_id, params))
}

/// `turn/interrupt` parameters, from the OUP type
/// (`TurnInterruptParams`, `crates/octos/octos-core/src/ui_protocol.rs:2237`) so the
/// wire shape is the kernel's.
fn turn_interrupt_params(
    session: &octos_core::SessionKey,
    turn: &TurnId,
) -> serde_json::Result<Value> {
    serde_json::to_value(TurnInterruptParams {
        session_id: session.clone(),
        turn_id: turn.clone(),
    })
}

/// What the kernel's `turn/start` reply says about the turn this host asked for.
///
/// The reply to `turn/start` is `TurnStartResult` — `{"accepted": true}`
/// (`crates/octos/octos-cli/src/api/ui_protocol_transport.rs:25956`) — which names
/// neither a turn nor a steer, so the id [`send_turn`] minted stands unless the
/// reply names another one. A reply that *does* carry `{turn_id, steered}` is
/// the steer receipt the same dispatcher sends when input is merged into the
/// turn already running (`:26940`, `:27003`). There is no core type for it —
/// `turn/steer` is transport-local (`APPUI_METHOD_TURN_STEER`, `:459`) — so the
/// two fields are read by name here, and the id is decoded with `TurnId`'s own
/// `Deserialize` rather than parsed by hand.
#[derive(Debug, PartialEq)]
struct TurnReceipt {
    /// The turn the kernel is running this input under.
    turn_id: TurnId,
    /// True when the input was merged into the turn already in flight.
    steered: bool,
}

fn turn_start_receipt(frame: &Value, minted: &TurnId) -> TurnReceipt {
    let result = frame.get("result");
    let turn_id = result
        .and_then(|result| result.get("turn_id"))
        .and_then(|turn_id| serde_json::from_value::<TurnId>(turn_id.clone()).ok())
        .unwrap_or_else(|| minted.clone());
    let steered = result
        .and_then(|result| result.get("steered"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    TurnReceipt { turn_id, steered }
}

/// Applies one receipt to the turn state, returning the note to record.
///
/// Pure, so what an interrupt would act on is testable without a gpui `App`.
/// A steered input must not read as a new conversation: the answer to it
/// streams into the turn that was already running.
fn apply_turn_receipt(receipt: &TurnReceipt, current: &mut Option<TurnId>) -> Option<String> {
    *current = Some(receipt.turn_id.clone());
    if receipt.steered {
        log::info!(
            "[octos] the kernel steered this input into the turn already running ({})",
            receipt.turn_id.0
        );
        return Some(
            "this input was merged into the turn already in flight (the kernel steered it)"
                .to_owned(),
        );
    }
    None
}

/// The turn a refused `turn/start` names, when it was refused because one is
/// already running.
///
/// This checkout does not steer a `turn/start` into a running turn: it refuses
/// it with a typed `turn_in_progress` error whose `data` carries the running
/// turn's id (`turn_in_progress_refusal`,
/// `crates/octos/octos-cli/src/api/ui_protocol_transport.rs:27653`). Adopting that id
/// is what keeps Stop meaningful — a refusal here usually means something *is*
/// running, and clearing the turn would take Stop away from exactly the turn
/// that needs it. `None` for any other refusal, where nothing of this host's is
/// in flight.
fn running_turn_from_refusal(error: &Value) -> Option<TurnId> {
    let data = error.get("data")?;
    if data.get("kind").and_then(Value::as_str) != Some("turn_in_progress") {
        return None;
    }
    serde_json::from_value::<TurnId>(data.get("turn_id")?.clone()).ok()
}

/// What the kernel's `turn/interrupt` reply means, decoded from
/// `TurnInterruptResult` (`crates/octos/octos-core/src/ui_protocol.rs:4795`) rather
/// than read by key: `interrupted`, `reason` and `terminal_state` are the
/// protocol's, and each means something different to the person.
#[derive(Debug, PartialEq)]
enum InterruptOutcome {
    /// The kernel captured the interrupt: the turn is ending.
    Interrupted,
    /// The turn had already reached a terminal state. Nothing was interrupted
    /// now, so the console must not claim it was.
    AlreadyOver { terminal_state: String },
    /// The kernel declined, with its own reason (`turn_id_mismatch`, …).
    Declined { reason: Option<String> },
    /// The reply was not a readable `TurnInterruptResult`.
    Unreadable,
}

fn interrupt_outcome(frame: &Value) -> InterruptOutcome {
    let Some(result) = frame.get("result") else {
        return InterruptOutcome::Unreadable;
    };
    let Ok(result) = serde_json::from_value::<TurnInterruptResult>(result.clone()) else {
        return InterruptOutcome::Unreadable;
    };
    if result.interrupted {
        InterruptOutcome::Interrupted
    } else if let Some(terminal_state) = result.terminal_state {
        InterruptOutcome::AlreadyOver { terminal_state }
    } else {
        InterruptOutcome::Declined {
            reason: result.reason,
        }
    }
}

/// Cancels the turn in flight with `turn/interrupt` — the console's Stop button
/// and Esc.
///
/// With no turn in flight this sends **nothing**: an interrupt is a claim about
/// a named turn, and with nothing pending there is no honest one to make. The
/// turn is only marked settled from the kernel's own reply; a declined,
/// timed-out or unreadable interrupt leaves it in flight and says so.
pub fn interrupt_turn(cx: &mut App) {
    if !is_pending(cx) {
        log::info!("[octos] no turn in flight: not sending turn/interrupt");
        return;
    }
    let named = cx.try_global::<Octos>().and_then(|octos| {
        let connection = octos.connection.clone()?;
        let session = octos.session_id()?.to_owned();
        let turn = octos.turn_id.clone()?;
        Some((connection, session, turn))
    });
    let Some((connection, session, turn)) = named else {
        log::warn!("[octos] cannot interrupt: the connection, session or turn id is missing");
        push_note(
            cx,
            "cannot interrupt: the kernel connection or the turn id is gone".to_owned(),
        );
        refresh_console(cx);
        return;
    };
    let params = match turn_interrupt_params(&octos_core::SessionKey(session), &turn) {
        Ok(params) => params,
        Err(error) => {
            log::error!("[octos] could not encode turn/interrupt: {error}");
            push_note(cx, format!("could not encode the interrupt: {error}"));
            refresh_console(cx);
            return;
        }
    };

    log::info!("[octos] interrupting turn {}", turn.0);
    cx.spawn(async move |cx| {
        let reply = connection
            .call_with_timeout(
                octos_core::ui_protocol::methods::TURN_INTERRUPT,
                params,
                TURN_INTERRUPT_TIMEOUT,
            )
            .await;
        cx.update(|cx| {
            match reply {
                Ok(frame) => match frame.get("error") {
                    Some(error) => {
                        log::warn!("[octos] the kernel refused the interrupt: {error}");
                        push_note(cx, format!("the kernel refused the interrupt: {error}"));
                    }
                    None => match interrupt_outcome(&frame) {
                        InterruptOutcome::Interrupted => {
                            log::info!("[octos] turn interrupted");
                            push_note(cx, "the turn was interrupted".to_owned());
                            set_pending(cx, false);
                            clear_turn_id(cx);
                        }
                        InterruptOutcome::AlreadyOver { terminal_state } => {
                            log::info!("[octos] the turn had already ended ({terminal_state})");
                            push_note(
                                cx,
                                format!("the turn had already ended ({terminal_state})"),
                            );
                            set_pending(cx, false);
                            clear_turn_id(cx);
                        }
                        InterruptOutcome::Declined { reason } => {
                            let reason =
                                reason.unwrap_or_else(|| "no reason given".to_owned());
                            log::warn!("[octos] the kernel declined the interrupt: {reason}");
                            push_note(cx, format!("the kernel declined the interrupt: {reason}"));
                        }
                        InterruptOutcome::Unreadable => {
                            log::warn!("[octos] unreadable turn/interrupt reply: {frame}");
                            push_note(
                                cx,
                                "the kernel's interrupt reply was not a turn/interrupt result — \
                                 the turn may still be running (see Zed.log)"
                                    .to_owned(),
                            );
                        }
                    },
                },
                Err(failure) => {
                    log::warn!("[octos] interrupt not acknowledged: {failure}");
                    push_note(cx, format!("the interrupt was not acknowledged: {failure}"));
                }
            }
            refresh_console(cx);
        });
    })
    .detach();
}

/// The person's choice of one option on one pending question.
pub fn select_question_option(
    cx: &mut App,
    question_id: &QuestionId,
    question_index: usize,
    label: &str,
) {
    let Some(peer) = peer(cx) else {
        return;
    };
    if !peer.select_question_option(question_id, question_index, label) {
        log::warn!(
            "[octos] refusing to select '{label}' on question {} #{question_index}: \
             it is not an option the kernel offered",
            question_id.0
        );
    }
    refresh_console(cx);
}

/// Answer one pending structured question from the console.
pub fn answer_question(cx: &mut App, question_id: &QuestionId, free_text: &str) {
    send_question_response(cx, question_id, free_text, false);
}

/// Decline to answer: the explicit exit that keeps the kernel from parking.
///
/// It sends one empty answer per question with a note saying the person
/// declined. That is a *legal* `user_question/respond` (the server validates the
/// answer count and the labels, `crates/octos/octos-cli/src/contracts/questions.rs:293`,
/// but never requires a non-empty answer) and it resolves the waiter, so the
/// blocked `ask_user_question` tool returns `Answered` with empty answers and
/// the turn continues. Interrupting the turn is the other way out — the kernel
/// drops pending questions on turn exit — but that ends the whole turn, which
/// declining does not.
pub fn dismiss_question(cx: &mut App, question_id: &QuestionId) {
    send_question_response(cx, question_id, "", true);
}

/// One `user_question/respond` frame, answered or dismissed.
fn send_question_response(
    cx: &mut App,
    question_id: &QuestionId,
    free_text: &str,
    dismissal: bool,
) {
    let (peer, connection, asked) = match cx.try_global::<Octos>() {
        Some(octos) => (
            octos.peer.clone(),
            octos.connection.clone(),
            octos.peer.question(question_id),
        ),
        None => return,
    };
    let Some(asked) = asked else {
        log::info!("[octos] question {} is no longer pending", question_id.0);
        refresh_console(cx);
        return;
    };
    let Some(connection) = connection else {
        push_note(
            cx,
            "cannot answer the question: the embedded kernel connection is closed".to_owned(),
        );
        refresh_console(cx);
        return;
    };

    let free_text = free_text.trim();
    if !free_text.is_empty() && !peer.question_accepts_free_text(question_id) {
        push_note(
            cx,
            "none of the kernel's questions accepts free text; only the selected options are sent"
                .to_owned(),
        );
    }
    let note = if dismissal {
        "dismissed in Zed without an answer"
    } else {
        "answered in Zed"
    };
    let Some(params) = peer.question_respond_params(
        question_id,
        (!free_text.is_empty()).then_some(free_text),
        Some(note),
    ) else {
        push_note(
            cx,
            format!("could not build an answer for question {}", question_id.0),
        );
        refresh_console(cx);
        return;
    };

    // Taken out before the frame goes out: a second click must not send a
    // second answer for the same question.
    let taken = peer.take_kernel_question(question_id);
    if taken.is_none() {
        refresh_console(cx);
        return;
    }
    let title = asked.title.clone();
    log::info!(
        "[octos] {} the kernel's question ({title})",
        if dismissal { "dismissed" } else { "answered" }
    );

    let question_id = question_id.clone();
    cx.spawn(async move |cx| {
        let reply = connection
            .call_with_timeout(
                octos_core::ui_protocol::methods::USER_QUESTION_RESPOND,
                params,
                USER_QUESTION_RESPOND_TIMEOUT,
            )
            .await;
        cx.update(|cx| {
            match reply {
                Ok(frame) => match frame.get("error") {
                    Some(error) => {
                        // `unknown`/`stale` mean the kernel is not waiting any
                        // more (the turn ended, or the question was answered
                        // elsewhere). Anything else leaves the entry `Pending`
                        // on the kernel side, so the card goes back and the
                        // person can correct the answer.
                        let kind = question_error_kind(error);
                        let gone = matches!(
                            kind.as_deref(),
                            Some("user_question_stale" | "user_question_unknown")
                        );
                        if gone {
                            log::info!(
                                "[octos] question {} was no longer pending: {error}",
                                question_id.0
                            );
                            push_note(
                                cx,
                                format!("the question about {title} was no longer pending"),
                            );
                        } else {
                            if let Some(taken) = taken.clone() {
                                peer.note_kernel_question(&taken);
                            }
                            log::warn!(
                                "[octos] the kernel refused the answer to {}: {error}",
                                question_id.0
                            );
                            push_note(
                                cx,
                                format!("the kernel refused the answer about {title}: {error}"),
                            );
                        }
                    }
                    None => push_note(
                        cx,
                        format!(
                            "{} the kernel's question about {title}",
                            if dismissal { "dismissed" } else { "answered" }
                        ),
                    ),
                },
                Err(failure) => {
                    // The answer never landed: put the card back, so nobody is
                    // left believing it was sent.
                    if let Some(taken) = taken.clone() {
                        peer.note_kernel_question(&taken);
                    }
                    log::warn!(
                        "[octos] answer to question {} not acknowledged: {failure}",
                        question_id.0
                    );
                    push_note(
                        cx,
                        format!("the answer about {title} was not acknowledged: {failure}"),
                    );
                }
            }
            refresh_console(cx);
        });
    })
    .detach();
    refresh_console(cx);
}

/// The machine-stable `kind` on a `user_question/*` RPC error
/// (`crates/octos/octos-cli/src/contracts/questions.rs:315`), read as a
/// diagnostic only: the decision above is "is the kernel still waiting".
fn question_error_kind(error: &Value) -> Option<String> {
    error
        .get("data")
        .and_then(|data| data.get("kind"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn push_note(cx: &mut App, note: String) {
    if cx.try_global::<Octos>().is_some() {
        let octos = cx.global_mut::<Octos>();
        octos.transcript.push(Line::Note(note));
        trim_transcript(octos);
    }
}

fn append_to_answer(cx: &mut App, text: &str) {
    if cx.try_global::<Octos>().is_some() {
        let octos = cx.global_mut::<Octos>();
        match octos.transcript.last_mut() {
            Some(Line::Octos(answer)) => answer.push_str(text),
            _ => octos.transcript.push(Line::Octos(text.to_owned())),
        }
        trim_transcript(octos);
    }
}

fn trim_transcript(octos: &mut Octos) {
    const KEEP: usize = 400;
    if octos.transcript.len() > KEEP {
        octos.transcript.drain(..octos.transcript.len() - KEEP);
    }
}

fn set_pending(cx: &mut App, pending: bool) {
    if cx.try_global::<Octos>().is_some() {
        let octos = cx.global_mut::<Octos>();
        octos.pending = pending;
    }
}

/// Forget the turn an interrupt would name: nothing is running any more, so
/// naming one would be a claim about a turn that no longer exists.
fn clear_turn_id(cx: &mut App) {
    if cx.try_global::<Octos>().is_some() {
        cx.global_mut::<Octos>().turn_id = None;
    }
}

/// Drops every pending question because the turn that asked is over.
///
/// The kernel drains its own pending questions when the turn exits
/// (`cancel_pending_for_turn`,
/// `crates/octos/octos-cli/src/api/ui_protocol_transport.rs:43316`), so a card left
/// on screen could only ever send an answer the kernel has already forgotten.
fn drop_questions(cx: &mut App, reason: &str) {
    let dropped = match peer(cx) {
        Some(peer) => {
            let pending = peer.questions();
            for asked in &pending {
                peer.forget_kernel_question(&asked.question_id);
            }
            pending.len()
        }
        None => 0,
    };
    if dropped > 0 {
        push_note(
            cx,
            format!("{dropped} unanswered question(s) dropped: {reason}"),
        );
        refresh_console(cx);
    }
}

/// Asks the console window to redraw, if it is open.
fn refresh_console(cx: &mut App) {
    if let Some(window) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<status_window::OctosConsole>())
    {
        let _ = window.update(cx, |_, _, cx| cx.notify());
    }
}

/// The session this host opens: the main profile, under `zed`.
fn session_key() -> octos_core::SessionKey {
    octos_core::SessionKey::with_profile(octos_core::MAIN_PROFILE_ID, APP_ID, SESSION_NAME)
}

/// The `session/open` parameters, matching the shape
/// `crates/octos/octos-cli/src/embedded.rs` documents and tests.
fn session_open_params(session: &octos_core::SessionKey, cwd: &Path) -> Value {
    json!({
        "session_id": session,
        "profile_id": octos_core::MAIN_PROFILE_ID,
        "cwd": cwd.to_string_lossy(),
    })
}

fn autostart_enabled() -> bool {
    // `cargo test -p zed` calls `init`; tests must not boot the kernel.
    if cfg!(test) {
        return false;
    }
    !matches!(
        env::var(ENV_AUTOSTART).as_deref(),
        Ok("0" | "false" | "off")
    )
}

/// The kernel's home: `<Zed data dir>/octos` unless overridden.
fn home() -> PathBuf {
    match env::var_os(ENV_HOME) {
        Some(home) => PathBuf::from(home),
        None => paths::data_dir().join("octos"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use octos_core::ui_protocol::{
        self as oup, InputItem, TurnStartParams, UserQuestionRequestedEvent,
        UserQuestionRespondParams, methods,
    };
    use std::collections::HashSet;

    /// A kernel frame's `params`, serialized from the real `octos-core` type
    /// rather than typed as a JSON literal — the point is that the test cannot
    /// drift from the wire shape on its own.
    fn wire<T: serde::Serialize>(value: &T) -> Value {
        serde_json::to_value(value).expect("wire params")
    }

    #[test]
    fn opens_the_main_profile_under_zed() {
        let params = session_open_params(&session_key(), Path::new("E:\\work"));

        assert_eq!(
            params["profile_id"],
            json!(octos_core::MAIN_PROFILE_ID)
        );
        assert_eq!(params["cwd"], json!("E:\\work"));
        assert!(
            params["session_id"]
                .as_str()
                .is_some_and(|session| session.contains(APP_ID)),
            "the session key should name this host: {params}"
        );
    }

    #[test]
    fn the_worker_stack_matches_the_embedded_requirement() {
        assert_eq!(WORKER_STACK_SIZE, 8 * 1024 * 1024);
    }

    /// The console's turn frame: the OUP types decide the wire shape, so this
    /// only pins what the kernel's dispatcher requires — a session, a fresh turn
    /// id, and one text item. The id comes back with the frame because it is
    /// what an interrupt has to name.
    #[test]
    fn encodes_a_turn_for_the_kernel() {
        let (turn_id, params) = turn_start_params("hello").expect("turn params");

        assert!(params["session_id"].as_str().is_some(), "{params}");
        assert_eq!(params["turn_id"], json!(turn_id), "{params}");
        assert_eq!(params["input"][0]["kind"], json!("text"));
        assert_eq!(params["input"][0]["text"], json!("hello"));

        // A fresh id per turn, or the kernel sees a duplicate.
        let (next_id, next) = turn_start_params("hello again").expect("turn params");
        assert_ne!(turn_id, next_id);
        assert_ne!(params["turn_id"], next["turn_id"]);
    }

    /// `turn/start` must decode back into the kernel's own params type, and
    /// every turn needs a distinct id — the kernel refuses a duplicate.
    ///
    /// Decoding here rather than reading keys by hand is what makes this the
    /// authoritative check: `TurnStartParams`
    /// (`crates/octos/octos-core/src/ui_protocol.rs:2125`) requires exactly
    /// `session_id`, `turn_id` and `input`, and `InputItem::Text` (`:2065`)
    /// tags itself `kind: "text"`.
    #[test]
    fn every_turn_round_trips_and_carries_a_unique_id() {
        let mut turn_ids = HashSet::new();

        for prompt in ["one", "two", "three"] {
            let (turn_id, params) = turn_start_params(prompt).expect("turn params");
            let decoded: TurnStartParams = serde_json::from_value(params.clone())
                .expect("the kernel's own turn/start params type");

            assert_eq!(
                decoded.input,
                vec![InputItem::Text {
                    text: prompt.to_owned()
                }],
                "{params}"
            );
            assert_eq!(
                decoded.turn_id, turn_id,
                "the id the caller is handed must be the one on the wire: {params}"
            );
            assert!(
                turn_ids.insert(decoded.turn_id),
                "turn ids must be unique per turn: {params}"
            );
        }
    }

    /// A reply frame as [`Connection`] delivers one: the transport envelope
    /// with an authoritative result inside it.
    fn reply_frame<T: serde::Serialize>(result: &T) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": "1",
            "result": serde_json::to_value(result).expect("result"),
        })
    }

    /// The steer receipt `turn/steer` answers with
    /// (`crates/octos/octos-cli/src/api/ui_protocol_transport.rs:26940`, `:27003`) —
    /// built by hand only because the protocol has no core type for it:
    /// `turn/steer` is transport-local (`APPUI_METHOD_TURN_STEER`, `:459`).
    fn steer_result(turn_id: &TurnId, steered: bool) -> Value {
        json!({ "turn_id": turn_id, "steered": steered })
    }

    /// A `turn/start` reply that names no turn is `TurnStartResult`
    /// (`crates/octos/octos-cli/src/api/ui_protocol_transport.rs:25956`): the id this
    /// host minted is the turn in flight, and nothing was steered.
    #[test]
    fn a_plain_turn_receipt_keeps_the_minted_id() {
        let (minted, _) = turn_start_params("hello").expect("turn params");
        let frame = reply_frame(&oup::TurnStartResult::accepted());

        let receipt = turn_start_receipt(&frame, &minted);
        assert_eq!(receipt.turn_id, minted);
        assert!(!receipt.steered, "{frame}");

        let mut current = Some(minted.clone());
        assert_eq!(apply_turn_receipt(&receipt, &mut current), None);
        assert_eq!(current, Some(minted));
    }

    /// The steer receipt: input merged into the turn already running answers
    /// with that turn's id and `steered: true`
    /// (`ui_protocol_transport.rs:26940`, `:27003`). The console must say the
    /// input was merged rather than present it as a new conversation — and the
    /// turn a later interrupt names has to become the one the kernel used.
    #[test]
    fn a_steered_turn_receipt_adopts_the_running_turn_and_says_so() {
        let (minted, _) = turn_start_params("and also this").expect("turn params");
        let running = TurnId::new();
        let frame = reply_frame(&steer_result(&running, true));

        let receipt = turn_start_receipt(&frame, &minted);
        assert!(receipt.steered, "{frame}");
        assert_eq!(
            receipt.turn_id, running,
            "the id the kernel used wins over the minted one"
        );

        let mut current = Some(minted);
        let note = apply_turn_receipt(&receipt, &mut current).expect("a steered input is noted");
        assert!(
            note.contains("merged into the turn already in flight"),
            "{note}"
        );

        // The interrupt this host would send names exactly that turn.
        let params =
            turn_interrupt_params(&session_key(), current.as_ref().expect("current turn"))
                .expect("interrupt params");
        let decoded: TurnInterruptParams =
            serde_json::from_value(params).expect("the kernel's own turn/interrupt params");
        assert_eq!(decoded.turn_id, running);
    }

    /// `steered: false` with a server-minted id (the `turn/steer` fallback when
    /// no turn was running, `ui_protocol_transport.rs:27003`) is adopted too, but
    /// is not reported as a merge.
    #[test]
    fn an_unsteered_receipt_with_another_id_is_not_a_merge() {
        let (minted, _) = turn_start_params("hello").expect("turn params");
        let server_turn = TurnId::new();
        let frame = reply_frame(&steer_result(&server_turn, false));

        let receipt = turn_start_receipt(&frame, &minted);
        assert!(!receipt.steered);
        assert_eq!(receipt.turn_id, server_turn);

        let mut current = Some(minted);
        assert_eq!(apply_turn_receipt(&receipt, &mut current), None);
        assert_eq!(current, Some(server_turn));
    }

    /// What this kernel actually does with a second `turn/start` on a busy
    /// session: it refuses it with a typed `turn_in_progress` error naming the
    /// running turn (`turn_in_progress_refusal`,
    /// `crates/octos/octos-cli/src/api/ui_protocol_transport.rs:27653`). The
    /// console adopts that id, because a refusal here means a turn *is*
    /// running — clearing the turn state would take Stop away from it.
    #[test]
    fn a_refused_start_adopts_the_running_turn_it_names() {
        let running = TurnId::new();
        let refusal = json!({
            "code": -32600,
            "message": "a turn is already running for this session",
            "data": { "kind": "turn_in_progress", "turn_id": running },
        });
        assert_eq!(running_turn_from_refusal(&refusal), Some(running));

        // Other refusals name no turn: nothing of this host's is in flight.
        assert_eq!(
            running_turn_from_refusal(&json!({
                "code": -32000,
                "message": "runtime unavailable",
            })),
            None
        );
        assert_eq!(
            running_turn_from_refusal(&json!({
                "code": -32600,
                "message": "busy",
                "data": { "kind": "turn_in_progress" },
            })),
            None,
            "a refusal with no turn id cannot be interrupted"
        );
        assert_eq!(
            running_turn_from_refusal(&json!({
                "code": -32600,
                "message": "busy",
                "data": { "kind": "something_else", "turn_id": TurnId::new() },
            })),
            None
        );
    }

    /// `turn/interrupt` is about one named turn: the frame decodes into the
    /// kernel's own `TurnInterruptParams`
    /// (`crates/octos/octos-core/src/ui_protocol.rs:2237`) with this host's session
    /// and turn, and the reply is read as its own type — only a captured
    /// interrupt may be reported to the person as one.
    #[test]
    fn an_interrupt_names_the_session_and_the_turn_in_flight() {
        let session = session_key();
        let turn = TurnId::new();

        let params = turn_interrupt_params(&session, &turn).expect("interrupt params");
        let decoded: TurnInterruptParams = serde_json::from_value(params.clone())
            .expect("the kernel's own turn/interrupt params type");
        assert_eq!(decoded.session_id, session, "{params}");
        assert_eq!(decoded.turn_id, turn, "{params}");

        assert_eq!(
            interrupt_outcome(&reply_frame(&TurnInterruptResult::interrupted_ok())),
            InterruptOutcome::Interrupted
        );
        assert_eq!(
            interrupt_outcome(&reply_frame(&TurnInterruptResult::ack_timed_out())),
            InterruptOutcome::Interrupted,
            "the server captured the interrupt; only the wire ack was uncertain"
        );
        assert_eq!(
            interrupt_outcome(&reply_frame(&TurnInterruptResult::declined("turn_id_mismatch"))),
            InterruptOutcome::Declined {
                reason: Some("turn_id_mismatch".to_owned())
            }
        );
        assert_eq!(
            interrupt_outcome(&reply_frame(&TurnInterruptResult::already_terminal(
                "completed",
                false
            ))),
            InterruptOutcome::AlreadyOver {
                terminal_state: "completed".to_owned()
            }
        );
        // An error frame is not a result: nothing may be reported as settled.
        assert_eq!(
            interrupt_outcome(&json!({ "code": -32000, "message": "unknown turn" })),
            InterruptOutcome::Unreadable
        );
    }

    /// Two questions, one of them multi-select: the console's selections become
    /// a real `user_question/respond` payload, read back with the kernel's own
    /// params type. Selection is confined to the labels the kernel offered —
    /// the server rejects anything else
    /// (`validate_answers`, `crates/octos/octos-cli/src/contracts/questions.rs:293`).
    #[test]
    fn a_user_question_is_rendered_and_answered_with_authoritative_frames() {
        let question_id = QuestionId::new();
        let event = UserQuestionRequestedEvent::new(
            session_key(),
            question_id.clone(),
            TurnId::new(),
            "Which web framework should I scaffold?",
            "The repo has no server crate yet.",
            vec![
                oup::UserQuestion {
                    header: "Framework".to_owned(),
                    question: "Which one?".to_owned(),
                    options: vec![
                        oup::UserQuestionOption {
                            label: "axum".to_owned(),
                            description: "tower-based".to_owned(),
                        },
                        oup::UserQuestionOption {
                            label: "actix".to_owned(),
                            description: "actor-based".to_owned(),
                        },
                    ],
                    multi_select: false,
                    allow_free_text: true,
                },
                oup::UserQuestion {
                    header: "Extras".to_owned(),
                    question: "Anything else?".to_owned(),
                    options: vec![oup::UserQuestionOption {
                        label: "tests".to_owned(),
                        description: "add tests".to_owned(),
                    }],
                    multi_select: true,
                    allow_free_text: true,
                },
            ],
        );

        // The regression this pins: this frame used to fall into `_ => Ignore`.
        // It is a blocking boundary with no kernel-side deadline
        // (`ui_protocol_transport.rs:6952`), so an ignored one parks the turn
        // forever.
        let ConsoleAction::KernelQuestion(asked) =
            console_action(methods::USER_QUESTION_REQUESTED, &wire(&event))
        else {
            panic!("a user_question/requested frame must decode as a question");
        };
        assert_eq!(asked.question_id, question_id);
        assert_eq!(asked.session_id, session_key().0);
        assert_eq!(asked.title, "Which web framework should I scaffold?");
        assert_eq!(asked.body, "The repo has no server crate yet.");
        assert_eq!(asked.selections.len(), 2, "one selection slot per question");

        let peer = HostPeer::default();
        peer.note_kernel_question(&asked);
        assert_eq!(peer.questions().len(), 1);

        assert!(peer.select_question_option(&asked.question_id, 0, "axum"));
        assert!(
            !peer.select_question_option(&asked.question_id, 0, "rocket"),
            "a label the kernel never offered must not be selectable"
        );
        assert!(peer.select_question_option(&asked.question_id, 1, "tests"));
        // Single-select replaces, multi-select accumulates, and a second click
        // on the chosen option clears it (an answer may be empty). The return
        // value is "was that a real option", not "is it selected now".
        assert!(peer.select_question_option(&asked.question_id, 0, "actix"));
        assert!(peer.select_question_option(&asked.question_id, 1, "tests"));
        assert_eq!(
            peer.question(&asked.question_id).expect("pending").selections,
            vec![vec!["actix".to_owned()], Vec::new()],
            "the cleared multi-select question keeps an empty slot"
        );
        assert!(peer.question_accepts_free_text(&asked.question_id));

        let params = peer
            .question_respond_params(
                &asked.question_id,
                Some("and a vite front end"),
                Some("answered in Zed"),
            )
            .expect("respond params");
        let decoded: UserQuestionRespondParams =
            serde_json::from_value(params.clone()).expect("the kernel's own respond params");
        assert_eq!(decoded.session_id, session_key(), "{params}");
        assert_eq!(decoded.question_id, question_id);
        assert_eq!(decoded.answers.len(), 2, "one answer per question: {params}");
        assert_eq!(decoded.answers[0].selected_labels, vec!["actix".to_owned()]);
        assert_eq!(
            decoded.answers[0].free_text.as_deref(),
            Some("and a vite front end"),
            "free text goes to the first question that accepts it"
        );
        assert_eq!(decoded.answers[1].free_text, None);
        assert!(decoded.answers[1].selected_labels.is_empty());
        assert_eq!(decoded.client_note.as_deref(), Some("answered in Zed"));

        // An answer no longer needed is dropped, once.
        assert!(peer.take_kernel_question(&asked.question_id).is_some());
        assert!(peer.take_kernel_question(&asked.question_id).is_none());
        assert!(peer.questions().is_empty());
    }

    /// The "no answer" exit: the same one-answer-per-question shape with every
    /// answer empty. That is a legal `user_question/respond` —
    /// `validate_answers` rejects a wrong count, an unknown label, several
    /// labels on a single-select or disallowed free text, never an empty answer
    /// (`crates/octos/octos-cli/src/contracts/questions.rs:293`) — which is what
    /// makes dismissing safe: the blocked `ask_user_question` tool unblocks as
    /// `Answered` with empty answers instead of the turn parking forever.
    #[test]
    fn dismissing_sends_one_empty_answer_per_question() {
        let event = UserQuestionRequestedEvent::new(
            session_key(),
            QuestionId::new(),
            TurnId::new(),
            "Pick",
            "Two questions",
            vec![
                oup::UserQuestion {
                    header: "One".to_owned(),
                    question: "First?".to_owned(),
                    options: vec![oup::UserQuestionOption {
                        label: "a".to_owned(),
                        description: String::new(),
                    }],
                    multi_select: false,
                    allow_free_text: false,
                },
                oup::UserQuestion {
                    header: "Two".to_owned(),
                    question: "Second?".to_owned(),
                    options: vec![oup::UserQuestionOption {
                        label: "b".to_owned(),
                        description: String::new(),
                    }],
                    multi_select: false,
                    allow_free_text: false,
                },
            ],
        );
        let peer = HostPeer::default();
        peer.note_kernel_question(&AskedQuestions::from_event(&event));

        assert!(
            !peer.question_accepts_free_text(&event.question_id),
            "neither question offers a free-text escape"
        );
        let params = peer
            .question_respond_params(
                &event.question_id,
                Some("text the kernel would reject"),
                Some("dismissed in Zed without an answer"),
            )
            .expect("a dismissal is still answerable");
        let decoded: UserQuestionRespondParams =
            serde_json::from_value(params).expect("the kernel's own respond params");

        assert_eq!(decoded.answers.len(), 2, "the server requires one per question");
        for answer in &decoded.answers {
            assert!(answer.selected_labels.is_empty(), "{answer:?}");
            assert_eq!(
                answer.free_text, None,
                "free text is dropped rather than sent to a question that forbids it"
            );
        }
        assert_eq!(
            decoded.client_note.as_deref(),
            Some("dismissed in Zed without an answer")
        );
    }

    /// The console reads a delta frame through the kernel's own notification
    /// type, so the text is `MessageDeltaEvent::text`
    /// (`crates/octos/octos-core/src/ui_protocol.rs:5380`) and the reasoning channel
    /// is the same shape at `:5389`.
    #[test]
    fn decodes_delta_frames_from_their_authoritative_types() {
        let delta = oup::MessageDeltaEvent {
            session_id: session_key(),
            topic: None,
            turn_id: oup::TurnId::new(),
            text: "Hello".to_owned(),
        };
        assert_eq!(
            console_action(methods::MESSAGE_DELTA, &wire(&delta)),
            ConsoleAction::Append("Hello".to_owned())
        );

        let reasoning = oup::ReasoningDeltaEvent {
            session_id: session_key(),
            topic: None,
            turn_id: oup::TurnId::new(),
            text: "weighing it up".to_owned(),
        };
        assert_eq!(
            console_action(methods::MESSAGE_REASONING_DELTA, &wire(&reasoning)),
            ConsoleAction::Append("weighing it up".to_owned())
        );

        let completed = oup::TurnCompletedEvent {
            session_id: session_key(),
            topic: None,
            turn_id: oup::TurnId::new(),
            cursor: None,
            tokens_in: None,
            tokens_out: None,
            session_result: None,
            token_usage: None,
        };
        assert_eq!(
            console_action(methods::TURN_COMPLETED, &wire(&completed)),
            ConsoleAction::Finished
        );
    }

    /// The regression this pins: the console used to fall back to `delta` or
    /// `content` when `text` was missing. Neither is a field of
    /// `MessageDeltaEvent`, so such a frame is not a delta frame and must not
    /// be rendered as answer text.
    #[test]
    fn a_delta_frame_without_the_authoritative_field_is_not_rendered() {
        let guessed = json!({
            "session_id": session_key(),
            "turn_id": oup::TurnId::new(),
            "delta": "not a field of MessageDeltaEvent",
            "content": "also not a field of MessageDeltaEvent",
        });

        assert_eq!(
            console_action(methods::MESSAGE_DELTA, &guessed),
            ConsoleAction::Ignore,
            "only the authoritative `text` field streams into the answer"
        );
    }

    /// `turn/error` carries `code` and `message`
    /// (`crates/octos/octos-core/src/ui_protocol.rs:6543`), never `text`. A frame
    /// that happens to include a `text` field must still be reported from
    /// `message`.
    #[test]
    fn a_turn_error_reports_the_kernels_message_not_a_guessed_text() {
        let error = oup::TurnErrorEvent {
            session_id: session_key(),
            topic: None,
            turn_id: oup::TurnId::new(),
            code: "llm_error".to_owned(),
            message: "the model refused".to_owned(),
            token_usage: None,
            partial_result: None,
        };
        let mut params = wire(&error);
        params["text"] = json!("a decoy the old fallback would have shown");

        assert_eq!(
            console_action(methods::TURN_ERROR, &params),
            ConsoleAction::Failed("turn failed [llm_error]: the model refused".to_owned())
        );
    }

    /// Frames that are not notification payloads this host renders (and
    /// non-protocol frames such as the kernel's own `session/notify`) are
    /// ignored rather than turned into transcript text.
    #[test]
    fn unknown_frames_are_ignored() {
        assert_eq!(
            console_action("session/notify", &json!({})),
            ConsoleAction::Ignore
        );
        assert_eq!(
            console_action(methods::MESSAGE_DELTA, &json!({ "garbage": true })),
            ConsoleAction::Ignore
        );
    }

    /// The seam host-side features call: with no kernel started, every accessor
    /// answers "no" rather than panicking.
    #[test]
    fn an_unstarted_kernel_exposes_nothing() {
        let octos = Octos::default();
        assert!(octos.connection().is_none());
        assert!(octos.home().is_none());
        assert!(octos.session_id().is_none());
        assert!(octos.failure().is_none());
        assert!(!octos.peer().is_registered());
        assert!(octos.tool_activity().is_empty());
        assert!(octos.approvals().is_empty());
    }

    /// The kernel's own tool lifecycle reaches the console as one activity
    /// entry, read from `ToolStartedEvent` / `ToolProgressEvent` /
    /// `ToolCompletedEvent` (`crates/octos/octos-core/src/ui_protocol.rs:5455` /
    /// `:5473` / `:5487`) rather than from guessed keys.
    #[test]
    fn decodes_kernel_tool_activity_from_its_authoritative_types() {
        let started = oup::ToolStartedEvent {
            session_id: session_key(),
            topic: None,
            turn_id: oup::TurnId::new(),
            tool_call_id: "tc-1".to_owned(),
            tool_name: "read_file".to_owned(),
            arguments: Some(json!({ "path": "a.rs" })),
        };
        assert_eq!(
            console_action(methods::TOOL_STARTED, &wire(&started)),
            ConsoleAction::KernelTool {
                tool_call_id: "tc-1".to_owned(),
                tool: "read_file".to_owned(),
                running: true,
                detail: String::new(),
                success: None,
            }
        );

        let progress = oup::ToolProgressEvent {
            session_id: session_key(),
            topic: None,
            turn_id: oup::TurnId::new(),
            tool_call_id: "tc-1".to_owned(),
            message: Some("halfway".to_owned()),
            progress_pct: Some(50.0),
        };
        assert_eq!(
            console_action(methods::TOOL_PROGRESS, &wire(&progress)),
            ConsoleAction::KernelTool {
                tool_call_id: "tc-1".to_owned(),
                // A progress frame carries no tool name: the entry it belongs
                // to already exists.
                tool: String::new(),
                running: true,
                detail: "halfway".to_owned(),
                success: None,
            }
        );

        let completed = oup::ToolCompletedEvent {
            session_id: session_key(),
            topic: None,
            turn_id: oup::TurnId::new(),
            tool_call_id: "tc-1".to_owned(),
            tool_name: "read_file".to_owned(),
            success: Some(true),
            output_preview: Some("42 lines".to_owned()),
            duration_ms: Some(12),
        };
        assert_eq!(
            console_action(methods::TOOL_COMPLETED, &wire(&completed)),
            ConsoleAction::KernelTool {
                tool_call_id: "tc-1".to_owned(),
                tool: "read_file".to_owned(),
                running: false,
                detail: "42 lines".to_owned(),
                success: Some(true),
            }
        );
    }

    /// An approval the *kernel* raises arrives as its own typed payload
    /// (`ApprovalRequestedEvent`, `crates/octos/octos-core/src/ui_protocol.rs:5690`)
    /// and is answered with `approval/respond`. The console must not confuse it
    /// with a Zed tool's own approval, which has no kernel frame at all.
    #[test]
    fn decodes_a_kernel_approval_from_its_authoritative_type() {
        let approval_id = oup::ApprovalId::new();
        let event = oup::ApprovalRequestedEvent {
            session_id: session_key(),
            topic: None,
            approval_id: approval_id.clone(),
            turn_id: oup::TurnId::new(),
            tool_name: "shell".to_owned(),
            title: "Run a command".to_owned(),
            body: "cargo test".to_owned(),
            approval_kind: Some("command".to_owned()),
            risk: Some("destructive".to_owned()),
            typed_details: Some(oup::ApprovalTypedDetails::command(
                oup::ApprovalCommandDetails {
                    argv: vec!["cargo".to_owned(), "test".to_owned()],
                    command_line: Some("cargo test".to_owned()),
                    cwd: None,
                    env_keys: Vec::new(),
                    tool_call_id: Some("tc-9".to_owned()),
                },
                None,
            )),
            render_hints: None,
        };

        let ConsoleAction::KernelApproval(approval) =
            console_action(methods::APPROVAL_REQUESTED, &wire(&event))
        else {
            panic!("an approval frame must decode as an approval");
        };
        assert_eq!(approval.approval_id, approval_id);
        assert_eq!(approval.session_id, session_key().0);
        assert_eq!(approval.tool_name, "shell");
        assert_eq!(approval.title, "Run a command");
        assert_eq!(approval.body, "cargo test");
        assert_eq!(approval.risk.as_deref(), Some("destructive"));
        // The kernel's tool call id lives in the typed details; reading it is
        // what lets the console correlate the card with the tool's activity.
        assert_eq!(approval.tool_call_id.as_deref(), Some("tc-9"));

        // A decision on that approval closes the card rather than rendering a
        // second one.
        let decided = oup::ApprovalDecidedEvent::manual(
            session_key(),
            approval_id.clone(),
            oup::TurnId::new(),
            oup::ApprovalDecision::Deny,
            "zed",
        );
        assert_eq!(
            console_action(methods::APPROVAL_DECIDED, &wire(&decided)),
            ConsoleAction::KernelApprovalClosed { approval_id }
        );
    }

    /// `progress/updated` and `warning` are notes, not answer text: the model
    /// did not say either one.
    #[test]
    fn progress_and_warnings_are_notes() {
        let progress = oup::ProgressUpdatedEvent::new(
            session_key(),
            Some(oup::TurnId::new()),
            oup::UiProgressMetadata {
                message: Some("compacting context".to_owned()),
                ..oup::UiProgressMetadata::new("compaction")
            },
        );
        assert_eq!(
            console_action(methods::PROGRESS_UPDATED, &wire(&progress)),
            ConsoleAction::Note("compacting context".to_owned())
        );

        let warning = oup::WarningEvent {
            session_id: session_key(),
            turn_id: None,
            code: "tool_truncated".to_owned(),
            message: "the output was capped".to_owned(),
        };
        assert_eq!(
            console_action(methods::WARNING, &wire(&warning)),
            ConsoleAction::Note("warning [tool_truncated]: the output was capped".to_owned())
        );
    }

    /// The host-tool transport is not a `UiNotification`: the console's decoder
    /// must ignore those frames rather than fail on them, because
    /// `forward_notifications` answers them on the kernel's runtime (see
    /// `peers`).
    #[test]
    fn host_tool_frames_are_not_ui_notifications() {
        for method in [peers::PEER_TOOL_CALL, peers::PEER_TOOL_CANCEL] {
            assert_eq!(
                console_action(method, &json!({ "call_id": "ptc-1" })),
                ConsoleAction::Ignore,
                "{method}"
            );
        }
    }

    /// A kernel envelope's frame params, produced by the kernel's own
    /// `into_rpc_notification` — the flattened wire shape (routing keys plus
    /// the bare envelope fields), not a hand-built literal.
    fn envelope(payload: oup::PayloadV2) -> Value {
        oup::UiNotification::EnvelopeV2(oup::EnvelopeV2Notification {
            session_id: session_key(),
            topic: None,
            envelope: oup::EnvelopeV2 {
                thread_id: "zed-embedded".to_owned(),
                seq: 1,
                cursor: None,
                turn_id: oup::TurnId::new().0.to_string(),
                client_message_id: None,
                payload,
            },
        })
        .into_rpc_notification()
        .expect("envelope frame")
        .params
    }

    /// Since #2318 the raw `tool/*` frames are suppressed for a connection that
    /// negotiated the canonical envelope (see `send_raw_notification_ephemeral`,
    /// `crates/octos/octos-cli/src/api/ui_protocol_transport.rs:46984`), so the
    /// envelope is the lane tool activity actually arrives on. The console
    /// reads it from `PayloadV2`'s own variants, which carry the same
    /// lifecycle the raw frames did.
    #[test]
    fn decodes_tool_activity_from_the_canonical_envelope() {
        assert_eq!(
            console_action(
                methods::PROJECTION_ENVELOPE,
                &envelope(oup::PayloadV2::ToolStart {
                    tool_call_id: "tc-3".to_owned(),
                    name: "shell".to_owned(),
                    arguments_preview: Some("cargo test".to_owned()),
                })
            ),
            ConsoleAction::KernelTool {
                tool_call_id: "tc-3".to_owned(),
                tool: "shell".to_owned(),
                running: true,
                detail: String::new(),
                success: None,
            }
        );

        assert_eq!(
            console_action(
                methods::PROJECTION_ENVELOPE,
                &envelope(oup::PayloadV2::ToolProgress {
                    tool_call_id: "tc-3".to_owned(),
                    message: "compiling".to_owned(),
                })
            ),
            ConsoleAction::KernelTool {
                tool_call_id: "tc-3".to_owned(),
                tool: String::new(),
                running: true,
                detail: "compiling".to_owned(),
                success: None,
            }
        );

        assert_eq!(
            console_action(
                methods::PROJECTION_ENVELOPE,
                &envelope(oup::PayloadV2::ToolEnd {
                    tool_call_id: "tc-3".to_owned(),
                    status: oup::EnvelopeToolEndStatus::Error,
                    error: Some("compile failed".to_owned()),
                    reason: None,
                    output_preview: None,
                    duration_ms: Some(900),
                })
            ),
            ConsoleAction::KernelTool {
                tool_call_id: "tc-3".to_owned(),
                tool: String::new(),
                running: false,
                detail: "compile failed".to_owned(),
                success: Some(false),
            }
        );

        assert_eq!(
            console_action(
                methods::PROJECTION_ENVELOPE,
                &envelope(oup::PayloadV2::ToolEnd {
                    tool_call_id: "tc-4".to_owned(),
                    status: oup::EnvelopeToolEndStatus::Complete,
                    error: None,
                    reason: None,
                    output_preview: Some("2 passed".to_owned()),
                    duration_ms: Some(1200),
                })
            ),
            ConsoleAction::KernelTool {
                tool_call_id: "tc-4".to_owned(),
                tool: String::new(),
                running: false,
                detail: "2 passed".to_owned(),
                success: Some(true),
            }
        );
    }

    /// The envelope's turn terminal settles the turn, and its `assistant_delta`
    /// **is** the answer: the raw `message/delta` lane is filtered out for every
    /// connection (`live_event_passes_capability_filter`,
    /// `crates/octos/octos-cli/src/api/ui_protocol_transport.rs:23337`), so a
    /// console that ignored this payload showed an empty answer — the exact
    /// "tool card, then nothing" report. The persisted copy of the same text is
    /// still not rendered: its own deltas already streamed it.
    #[test]
    fn an_envelope_terminal_settles_the_turn_and_an_envelope_delta_is_the_answer() {
        assert_eq!(
            console_action(
                methods::PROJECTION_ENVELOPE,
                &envelope(oup::PayloadV2::TurnTerminal {
                    outcome: oup::TurnTerminalOutcome::Completed,
                    error: None,
                    token_usage: None,
                })
            ),
            ConsoleAction::Finished
        );
        assert_eq!(
            console_action(
                methods::PROJECTION_ENVELOPE,
                &envelope(oup::PayloadV2::TurnTerminal {
                    outcome: oup::TurnTerminalOutcome::Interrupted,
                    error: None,
                    token_usage: None,
                })
            ),
            ConsoleAction::Note("the turn was interrupted".to_owned())
        );
        assert_eq!(
            console_action(
                methods::PROJECTION_ENVELOPE,
                &envelope(oup::PayloadV2::AssistantDelta {
                    text: "the answer".to_owned(),
                    assistant_segment_id: "seg-1".to_owned(),
                })
            ),
            ConsoleAction::Append("the answer".to_owned())
        );
        assert_eq!(
            console_action(
                methods::PROJECTION_ENVELOPE,
                &envelope(oup::PayloadV2::ReasoningDelta {
                    text: "the thinking".to_owned(),
                })
            ),
            ConsoleAction::Append("the thinking".to_owned())
        );
        assert_eq!(
            console_action(
                methods::PROJECTION_ENVELOPE,
                &envelope(oup::PayloadV2::AssistantPersisted {
                    text: "the answer".to_owned(),
                    assistant_segment_id: "seg-1".to_owned(),
                    meta: serde_json::from_value(json!({
                        "message_id": "m-1",
                        "persisted_at": "2026-10-16T14:06:32.143261Z",
                    }))
                    .expect("meta"),
                })
            ),
            ConsoleAction::Ignore
        );
    }

    /// **A discriminator for the answer lane.** The raw delta still renders, so
    /// the console is correct on a kernel that still sends it; and the envelope
    /// renders too. They cannot both arrive — the filter above drops the raw
    /// record whenever its v2 projection exists — so "both arms render" is not
    /// "every answer doubles".
    #[test]
    fn both_delta_lanes_render_and_cannot_both_arrive() {
        let raw = console_action(
            methods::MESSAGE_DELTA,
            &serde_json::to_value(oup::MessageDeltaEvent {
                session_id: session_key(),
                topic: None,
                turn_id: oup::TurnId::new(),
                text: "half".to_owned(),
            })
            .expect("delta params"),
        );
        assert_eq!(raw, ConsoleAction::Append("half".to_owned()));
    }

    // -----------------------------------------------------------------------
    // The frame fan-out (`Octos::subscribe`)
    // -----------------------------------------------------------------------

    /// One kernel-initiated frame as [`Connection`] hands it to
    /// [`forward_notifications`]: the transport envelope around a method and its
    /// params.
    fn notification(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "method": method, "params": params })
    }

    /// Runs [`forward_notifications`] over `frames`, wired the way [`start`]
    /// wires it: one connection, one console lane, one fan-out.
    ///
    /// The notification queue is dropped once the frames are in it, so the loop
    /// ends exactly as it does when the kernel closes its end of the pipe — after
    /// routing those frames, with `Event::Closed` behind them.
    ///
    /// Returns, in order: the peer the loop recorded on, the console's lane, the
    /// kernel's own end of the pipe (kept so a host answer is a write that
    /// succeeds rather than a broken pipe), and the loop's task.
    fn forward_frames(
        frames: Vec<Value>,
        fanout: broadcast::Sender<OctosFrame>,
    ) -> (
        HostPeer,
        mpsc::UnboundedReceiver<Event>,
        tokio::io::DuplexStream,
        tokio::task::JoinHandle<()>,
    ) {
        let (host, kernel) = tokio::io::duplex(64 * 1024);
        let (connection, _kernel_frames) = Connection::new(host);
        let peer = HostPeer::default();
        let (events, events_rx) = mpsc::unbounded::<Event>();
        let (notifications, notifications_rx) = mpsc::unbounded::<Value>();
        for frame in frames {
            notifications
                .unbounded_send(frame)
                .expect("the loop's queue is live until it is dropped below");
        }
        drop(notifications);

        let loop_task = tokio::spawn(forward_notifications(
            notifications_rx,
            connection,
            peer.clone(),
            events,
            fanout,
        ));
        (peer, events_rx, kernel, loop_task)
    }

    /// A subscriber gets the frames the kernel initiates — and never the two
    /// host-tool frames, which [`forward_notifications`] serves in place: the
    /// kernel blocks on `peer/tool/result` while it runs the turn, and a
    /// subscriber is by definition not on that critical path.
    #[tokio::test]
    async fn a_subscriber_sees_kernel_frames_but_never_host_tool_frames() {
        let delta = oup::MessageDeltaEvent {
            session_id: session_key(),
            topic: None,
            turn_id: TurnId::new(),
            text: "Hello".to_owned(),
        };
        let call = peers::ToolCallParams {
            peer: None,
            session_id: session_key().0,
            context_id: None,
            turn_id: TurnId::new(),
            call_id: "ptc-1".to_owned(),
            tool_call_id: "tc-1".to_owned(),
            args_digest: "sha256:00".to_owned(),
            name: "zed.workspace.read".to_owned(),
            app: None,
            caller: None,
            args: json!({ "path": "a.rs" }),
            risk: None,
            confirm_required: false,
            timeout_ms: None,
            tools_version: None,
        };

        let (fanout, mut subscriber) = broadcast::channel::<OctosFrame>(FANOUT_CAPACITY);
        let (peer, mut events, _kernel, loop_task) = forward_frames(
            vec![
                notification(methods::MESSAGE_DELTA, wire(&delta)),
                notification(peers::PEER_TOOL_CALL, wire(&call)),
                notification(
                    peers::PEER_TOOL_CANCEL,
                    // `ToolCancelParams` is `Deserialize`-only, so this frame is
                    // the wire shape the kernel sends rather than a serialized
                    // core type.
                    json!({ "call_id": call.call_id, "reason": "cancelled" }),
                ),
            ],
            fanout,
        );

        // The subscriber sees the delta...
        let frame = next_frame(&mut subscriber)
            .await
            .expect("the delta the kernel initiated");
        assert_eq!(frame.method, methods::MESSAGE_DELTA);
        assert_eq!(frame.params, wire(&delta));
        assert!(!frame.is_resync(), "this is a kernel frame, not our marker");

        // ...and then its stream ends with the connection: neither host-tool
        // frame was fanned out.
        assert!(
            next_frame(&mut subscriber).await.is_none(),
            "peer/tool/call and peer/tool/cancel are the host-tool transport, \
             not frames for a subscriber"
        );

        // The in-place path really served them: an unregistered peer refuses the
        // call, and that refusal is recorded on the peer itself.
        let activity = peer.activity();
        assert_eq!(activity.len(), 1, "{activity:?}");
        assert_eq!(activity[0].call_id, "ptc-1");
        assert_eq!(activity[0].state, ActivityState::Refused);

        // The console's own lane is exactly what it was: one notification, then
        // the close.
        assert!(
            matches!(
                tokio::time::timeout(Duration::from_secs(5), events.next())
                    .await
                    .expect("the console's notification"),
                Some(Event::Notification { method, .. }) if method == methods::MESSAGE_DELTA
            ),
            "the console must still be routed the delta"
        );
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), events.next())
                .await
                .expect("the close"),
            Some(Event::Closed)
        ));
        loop_task.await.expect("the routing loop ends cleanly");
    }

    /// A frame with nobody subscribed still reaches the console.
    ///
    /// `broadcast::Sender::send` returns `Err` when there is no subscriber at
    /// all — a normal state, not the "the console is gone" failure the loop's
    /// other send means. Reading it as that would stop routing frames the moment
    /// nobody watched.
    #[tokio::test]
    async fn a_frame_with_no_subscriber_still_reaches_the_console() {
        let delta = oup::MessageDeltaEvent {
            session_id: session_key(),
            topic: None,
            turn_id: TurnId::new(),
            text: "no one is listening".to_owned(),
        };

        let (fanout, nobody) = broadcast::channel::<OctosFrame>(FANOUT_CAPACITY);
        // The receiver `broadcast::channel` hands back is the only subscriber
        // this run would have; dropping it makes every `send` below fail.
        drop(nobody);
        assert_eq!(fanout.receiver_count(), 0);

        let (_peer, mut events, _kernel, loop_task) = forward_frames(
            vec![notification(methods::MESSAGE_DELTA, wire(&delta))],
            fanout,
        );

        assert!(
            matches!(
                tokio::time::timeout(Duration::from_secs(5), events.next())
                    .await
                    .expect("the console's notification"),
                Some(Event::Notification { method, params })
                    if method == methods::MESSAGE_DELTA && params == wire(&delta)
            ),
            "the console's lane does not depend on anybody subscribing"
        );
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), events.next())
                .await
                .expect("the close"),
            Some(Event::Closed)
        ));
        loop_task.await.expect("the routing loop ends cleanly");
    }

    /// `broadcast` is a bounded ring, so a subscriber that falls behind loses
    /// frames — and the loss is made visible rather than swallowed: it is logged
    /// with the count, and the subscriber is handed this host's resync marker. A
    /// lost `turn/completed` is a thread that would otherwise stay "running"
    /// forever.
    #[tokio::test]
    async fn a_lagged_subscriber_is_told_what_it_lost() {
        // Two frames of ring, five frames sent, none read in between: the three
        // oldest are gone.
        let (fanout, mut raw) = broadcast::channel::<OctosFrame>(2);
        let mut subscriber = fanout.subscribe();
        for index in 0..5 {
            fanout
                .send(OctosFrame {
                    method: methods::MESSAGE_DELTA.to_owned(),
                    params: json!({ "n": index }),
                })
                .expect("both subscribers are live");
        }

        // The raw signal this host turns into its own marker...
        assert!(
            matches!(
                raw.recv().await,
                Err(broadcast::error::RecvError::Lagged(3))
            ),
            "the frames past the ring are exactly the ones lost"
        );

        // ...and the path a subscriber uses: a marker that names the loss.
        let marker = next_frame(&mut subscriber)
            .await
            .expect("a marker, not a silent gap");
        assert!(marker.is_resync());
        assert_eq!(marker.method, OctosFrame::RESYNC_METHOD);
        assert_eq!(marker.missed_frames(), Some(3));

        // The frames the ring still held follow it, in order: a lagged
        // subscriber resynchronizes and keeps reading rather than being cut off.
        let kept = next_frame(&mut subscriber)
            .await
            .expect("the oldest frame the ring still holds");
        assert_eq!(kept.params, json!({ "n": 3 }));
    }

    /// `subscribe` follows the kernel's lifecycle: nothing to fan out before a
    /// kernel is started, a live subscription once the fan-out exists — and a
    /// stream that *ends* when the connection does, rather than one that waits
    /// forever on frames that cannot come.
    #[tokio::test]
    async fn subscribe_follows_the_kernels_lifecycle() {
        let unstarted = Octos::default();
        assert!(
            unstarted.subscribe().is_none(),
            "no kernel, no frames to fan out"
        );

        let (fanout, _) = broadcast::channel::<OctosFrame>(FANOUT_CAPACITY);
        let mut octos = Octos::default();
        octos.fanout = Some(fanout.clone());
        let mut subscriber = octos
            .subscribe()
            .expect("a started kernel serves subscribers");

        fanout
            .send(OctosFrame {
                method: "session/notify".to_owned(),
                params: json!({}),
            })
            .expect("the subscriber is live");
        let frame = next_frame(&mut subscriber).await.expect("the frame");
        assert_eq!(frame.method, "session/notify");

        // The connection ending takes the sender with it (`apply_event`'s
        // `Event::Closed` arm and `forward_notifications` both drop theirs).
        octos.fanout.take();
        drop(fanout);
        assert!(
            next_frame(&mut subscriber).await.is_none(),
            "a closed connection ends the stream instead of hanging it"
        );
    }
}
