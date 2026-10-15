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

pub mod connection;
pub mod peers;
pub mod status_window;

use connection::Connection;
use futures::channel::mpsc;
use futures::StreamExt as _;
use gpui::{App, Global, Task, TaskExt as _};
use peers::{ActivityState, ApprovalRequest, HostPeer, ToolActivity};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::runtime::Runtime;

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

/// The embedded Octos kernel and the runtime that drives it.
#[derive(Default)]
pub struct Octos {
    /// Shutting this down ends the embedded connection.
    runtime: Option<Runtime>,
    /// The host side of the OUP connection: the seam future features extend.
    connection: Option<Connection>,
    /// This host's end of the host-tool contract ([`peers`]).
    peer: HostPeer,
    home: Option<PathBuf>,
    state: State,
    /// The console's conversation, oldest first.
    transcript: Vec<Line>,
    /// Whether a turn is in flight.
    pending: bool,
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
                    .call(peers::APPROVAL_RESPOND, params)
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
                    Err(_) => push_note(
                        cx,
                        "the kernel closed the connection before the decision landed".to_owned(),
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
    ));

    let events_task = cx.spawn(async move |cx| {
        while let Some(event) = events_rx.next().await {
            cx.update(|cx| apply_event(cx, event));
        }
    });

    let octos = cx.global_mut::<Octos>();
    octos.home = Some(home);
    octos.connection = Some(connection);
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
        .call(octos_core::ui_protocol::methods::SESSION_OPEN, params)
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
        Err(_) => {
            let _ = events.unbounded_send(Event::Failed {
                reason: "the embedded kernel closed the connection before the session \
                         opened (the kernel's own reason is in Zed.log)"
                    .to_owned(),
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

    let reply = connection.call(peers::PEER_TOOLS_REGISTER, params).await;
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
        Err(_) => {
            log::warn!(
                "[octos] the connection closed while registering this host's tools; \
                 the model will see no Zed tools"
            );
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
async fn forward_notifications(
    mut notifications: mpsc::UnboundedReceiver<Value>,
    connection: Connection,
    peer: HostPeer,
    events: mpsc::UnboundedSender<Event>,
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

        if events
            .unbounded_send(Event::Notification { method, params })
            .is_err()
        {
            return;
        }
    }
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
        Event::Closed => {
            log::info!("[octos] embedded connection closed");
            // The registration outlives nothing: a closed pipe can never be
            // called on again, so every pending call is dead.
            octos.peer.reset();
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
/// rest are ignored rather than guessed at. In particular an
/// `assistant_delta` is **not** rendered: the same text also arrives on
/// `message/delta`, and appending both would double every answer.
fn envelope_action(payload: &octos_core::ui_protocol::PayloadV2) -> ConsoleAction {
    use octos_core::ui_protocol::{EnvelopeToolEndStatus, PayloadV2, TurnTerminalOutcome};

    match payload {
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
        ConsoleAction::Note(note) => push_note(cx, note),
        ConsoleAction::Finished => set_pending(cx, false),
        ConsoleAction::Failed(reason) => {
            push_note(cx, reason);
            set_pending(cx, false);
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
/// back as `message/delta` notifications, which [`apply_notification`] appends
/// to the console.
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

    let params = match turn_start_params(&text) {
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
    }
    refresh_console(cx);

    cx.spawn(async move |cx| {
        let reply = connection.call(octos_core::ui_protocol::methods::TURN_START, params);
        match reply.await {
            Ok(frame) => {
                if let Some(error) = frame.get("error") {
                    cx.update(|cx| {
                        push_note(cx, format!("turn refused: {error}"));
                        set_pending(cx, false);
                        refresh_console(cx);
                    });
                }
            }
            Err(_) => cx.update(|cx| {
                push_note(cx, "the embedded kernel closed the connection".to_owned());
                set_pending(cx, false);
                refresh_console(cx);
            }),
        }
    })
    .detach();
}

/// `turn/start` parameters, built from the OUP types so their wire shape stays
/// the kernel's business rather than this host's.
fn turn_start_params(text: &str) -> serde_json::Result<Value> {
    Ok(json!({
        "session_id": serde_json::to_value(session_key())?,
        "turn_id": serde_json::to_value(octos_core::ui_protocol::TurnId::new())?,
        "input": [serde_json::to_value(octos_core::ui_protocol::InputItem::Text {
            text: text.to_owned(),
        })?],
    }))
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
    use octos_core::ui_protocol::{self as oup, InputItem, TurnStartParams, methods};
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
    /// id, and one text item.
    #[test]
    fn encodes_a_turn_for_the_kernel() {
        let params = turn_start_params("hello").expect("turn params");

        assert!(params["session_id"].as_str().is_some(), "{params}");
        assert!(params["turn_id"].as_str().is_some(), "{params}");
        assert_eq!(params["input"][0]["kind"], json!("text"));
        assert_eq!(params["input"][0]["text"], json!("hello"));

        // A fresh id per turn, or the kernel sees a duplicate.
        let next = turn_start_params("hello again").expect("turn params");
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
            let params = turn_start_params(prompt).expect("turn params");
            let decoded: TurnStartParams = serde_json::from_value(params.clone())
                .expect("the kernel's own turn/start params type");

            assert_eq!(
                decoded.input,
                vec![InputItem::Text {
                    text: prompt.to_owned()
                }],
                "{params}"
            );
            assert!(
                turn_ids.insert(decoded.turn_id),
                "turn ids must be unique per turn: {params}"
            );
        }
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

    /// The envelope's turn terminal settles the turn, and an `assistant_delta`
    /// is deliberately NOT rendered: the same text arrives on `message/delta`
    /// and appending both would double every answer.
    #[test]
    fn an_envelope_terminal_settles_the_turn_and_a_delta_is_left_to_the_raw_lane() {
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
                    text: "already on the raw lane".to_owned(),
                    assistant_segment_id: "seg-1".to_owned(),
                })
            ),
            ConsoleAction::Ignore
        );
    }
}
