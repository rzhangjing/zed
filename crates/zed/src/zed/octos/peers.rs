//! Host-side peers: the tools Zed offers the embedded Octos kernel.
//!
//! The kernel is the *agent*; Zed is the *host*. When the agent needs something
//! only the host can do — look at this editor's workspace, put a file in front
//! of the person — Octos does not reach across the boundary: it asks. The
//! UPCR-2026-035 host-tool contract is that round trip, and this module is
//! Zed's end of it. The kernel's own side lives in
//! `crates/octos/octos-cli/src/peers/host_tools.rs` (`TurnHostToolRouter` sends the
//! call; `raw_peer_tool_result` completes it) and
//! `crates/octos/octos-agent/src/tools/peer_host_tool.rs` (the declarations and risk
//! gating the kernel applies before it asks).
//!
//! ```text
//!   Zed (host)                                          Octos (kernel)
//!   ─────────────────────────────────────────────────   ──────────────────────────
//!   peer/tools/register  ──── declare tools ──────────▶  peers/<session route>.json
//!   peer/tool/call       ◀─── run zed.workspace.read ──  HostRoutedTool
//!   peer/tool/result     ──── {ok, data} ─────────────▶  oneshot → model step
//!   peer/tool/cancel     ◀─── timeouts / turn interrupt  PendingGuard::drop
//! ```
//!
//! Two boundaries in `.rules` this module exists to keep:
//!
//! * **A tool declaration, or model text, is not authorization.** Registering a
//!   tool with the kernel says what the agent *may ask for*; it grants nothing.
//!   Every `peer/tool/call` is re-checked here, on the host, against the
//!   session this process actually opened, the workspace that session was
//!   opened in, and the registration this connection actually made
//!   ([`HostPeer::plan_call`]). A call that fails any check is refused and
//!   *nothing runs*.
//! * **The request context is a session, not a new peer.** One connection, one
//!   session ([`super::session_key`]), one route: `peer/tool/result` is
//!   correlated back to the call that asked (the `call_id` of
//!   [`HostPeer::plan_call`]'s plan), never to whatever turn happens to be
//!   current, and a result is refused outright unless the call is still pending
//!   and uncancelled. Shared history between transcripts is not this module's
//!   business and is not touched here.
//!
//! Registration is on the *host's own session* (`peer/tools/register` without
//! `peer`). That is the shape for a host process that owns its pipe: the kernel
//! accepts it with no app-peer host token from a stdio connection
//! (`raw_session_tools_register` — `authorize_host_session_call`, which returns
//! early for `ws.is_stdio()`), and every call is routed back to the connection
//! that registered it. An app *peer* (`peer/prepare` + a host binding) would
//! additionally pin the calls to a workspace and memory namespace on the kernel
//! side; this host has exactly one workspace — the one the session was opened
//! in — so the session route is the honest binding, and it is what
//! [`HostPeer::workspace`] re-checks on the host side too.
//!
//! Env: `ZED_OCTOS_HOST_TOOLS=0` stops the registration (the kernel then offers
//! the model no Zed tools at all).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use octos_core::ui_protocol::{
    ApprovalId, QuestionId, RpcError, TurnId, UserQuestionAnswer, UserQuestionRespondParams, methods,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::connection::Connection;

/// `peer/tools/register` — declare (replace) this host session's tool set.
///
/// Not in `octos_core::ui_protocol::methods`: the OUP *client* never sends it,
/// so the core crate exposes no constant (compare `methods::PEER_TOOL_CALL`,
/// which the kernel sends and core does name). Vendored here verbatim from
/// `crates/octos/octos-cli/src/api/ui_protocol_transport.rs:428`.
pub const PEER_TOOLS_REGISTER: &str = "peer/tools/register";
/// `peer/tool/result` — this host's answer to one `peer/tool/call`
/// (`crates/octos/octos-cli/src/api/ui_protocol_transport.rs:430`).
pub const PEER_TOOL_RESULT: &str = "peer/tool/result";

/// The owning app of the tools this host registers. Zed's own tools are a
/// dotted namespace the model sees as `zed_*`.
const APP: &str = "zed";

/// Longest result this host will hand the model, in bytes. The kernel also
/// caps `max_result_bytes` per set; keeping the host's own cap below it means a
/// truncation is reported by the tool that knows it truncated rather than as a
/// kernel-side `result_too_large` the model cannot act on.
pub const MAX_RESULT_BYTES: usize = 64 * 1024;

/// Longest error message handed back to the kernel. The kernel caps at 4 KiB
/// (`PEER_TOOL_ERROR_MESSAGE_MAX_BYTES`); mirror it so a huge `io::Error` never
/// gets a frame rejected for size.
const MAX_ERROR_BYTES: usize = 4 * 1024;

/// How long the kernel waits for one host call before it gives up and cancels.
///
/// The kernel's ceiling (`MAX_CALL_TIMEOUT_MS`,
/// `crates/octos/octos-cli/src/peers/host_tools.rs:62`). A call held for the person
/// is not "gated" (the tools that reach the approval are `confirm: app`, and
/// the kernel only extends a deadline to the approval TTL for a call whose tool
/// the *kernel* gated — see `peer_host_tool.rs`'s `ack.notified()` arm), so this
/// is the real budget a person has to answer before the kernel cancels.
/// Five minutes is the most the contract allows.
pub const CALL_TIMEOUT_MS: u64 = 300_000;

/// Up to this many bytes of a read are returned.
const READ_MAX_BYTES: usize = 48 * 1024;

/// In-flight calls this host will hold. Mirrors the kernel's
/// `MAX_PENDING_CALLS_PER_PEER`; past it the extra call is refused here rather
/// than silently queued.
const MAX_PENDING_CALLS: usize = 16;

/// How many finished calls are kept for the console.
const ACTIVITY_KEEP: usize = 200;

/// How many handled call ids are remembered for duplicate detection. The
/// kernel's own in-flight cap is 16 per host, so a few hundred is far more than
/// any live turn can hold.
const HANDLED_CALLS_KEEP: usize = 1_024;

// ---------------------------------------------------------------------------
// Route key
// ---------------------------------------------------------------------------

/// `[a-z0-9_]`, non-empty, at most 64 bytes: the alphabet
/// `SessionKey::profile_id()` is allowed to produce, so hashing one can never
/// collide with a differently-shaped profile id.
#[cfg(test)]
fn profile_segment_is_safe(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= 64
        && raw
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The kernel's per-session route key, mirroring
/// `crates/octos/octos-cli/src/peers/host_tools.rs:1230` (`session_route_key`): a
/// session whose profile id is a plain `[a-z0-9_]` segment shares the route
/// with that segment's peers; any other session gets its own hashed route.
///
/// The host does not need this to register — the kernel computes the route
/// from the session it is handed — so it exists as the test oracle that keeps
/// the rule above honest (`the_session_route_key_follows_the_kernels_rule`).
#[cfg(test)]
pub fn session_route_key(session: &str) -> String {
    match octos_core::SessionKey(session.to_owned()).profile_id() {
        Some(profile) if profile_segment_is_safe(profile) => profile.to_owned(),
        _ => {
            // FNV-1a/64 is enough here: this is a filename for the host's own
            // registration, not a security boundary. The kernel computes the
            // same string with the same algorithm, and a collision only means
            // two of this host's sessions share a tool set.
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for byte in session.as_bytes() {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            format!("session-{hash:016x}")
        }
    }
}

/// Whether `name` may be a session's tool route (the kernel's
/// `peer_slug_is_safe`): the shape a route key can take on disk.
#[cfg(test)]
pub fn route_key_is_safe(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

// ---------------------------------------------------------------------------
// Declarations
// ---------------------------------------------------------------------------

/// One tool this host declares, as `peer/tools/register` wants it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostTool {
    /// Dotted name in the host's namespace (`zed.workspace.read`).
    pub name: &'static str,
    /// The model-visible alias the kernel derives (`zed_workspace_read`).
    pub model_name: &'static str,
    pub description: &'static str,
    /// JSON Schema for the arguments. Must be a `"type": "object"` schema: the
    /// kernel refuses anything else (`parse_schema`,
    /// `crates/octos/octos-cli/src/peers/host_tools.rs:188`).
    pub input_schema: Value,
    /// `read` | `act` | `destructive` — the kernel gates `destructive`
    /// (`HostToolRisk`, `crates/octos/octos-agent/src/tools/peer_host_tool.rs:58`).
    pub risk: &'static str,
    /// Whether this host demands an explicit approval before it runs the tool.
    pub needs_approval: bool,
}

/// The tools this host offers. Deliberately tiny and low risk: one read-only
/// look at the session's workspace, and one editor action that is gated on the
/// person. Both are real — neither writes to the workspace.
pub fn host_tools() -> Vec<HostTool> {
    vec![
        HostTool {
            name: "zed.workspace.read",
            model_name: "zed_workspace_read",
            description: "Read a UTF-8 text file from the workspace this Zed session was \
                          opened in, confined to that workspace. Returns the file's line \
                          count and its text (truncated). Paths are workspace-relative; \
                          absolute paths and `..` are refused.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative path, e.g. `crates/zed/src/main.rs`."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "description": "Stop after this many bytes (default and ceiling 49152)."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            risk: "read",
            needs_approval: false,
        },
        HostTool {
            name: "zed.editor.open",
            model_name: "zed_editor_open",
            description: "Open a workspace-relative file in Zed's editor (a new tab or a \
                          focus of the existing one). The workspace is not modified. The \
                          person is asked before this runs.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative path to open."
                    },
                    "line": {
                        "type": "integer",
                        "description": "Optional 1-based line to reveal."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            risk: "act",
            needs_approval: true,
        },
    ]
}

/// The declaration a `peer/tool/call` names.
///
/// A call carries the tool's **declared** name (`zed.workspace.read`), not the
/// model-visible alias: the kernel's router sends `HostToolCall::name`, the
/// declaration's `name` (`crates/octos/octos-cli/src/peers/host_tools.rs:2725`; the
/// routing test asserts `calls[0]["name"] == "news.list"`). The alias only
/// exists because provider tool names cannot hold a dot. Accepting either
/// spelling would be a second, guessed-at name surface, so only the declared
/// one resolves.
fn tool(declared_name: &str) -> Option<HostTool> {
    host_tools()
        .into_iter()
        .find(|tool| tool.name == declared_name)
}

// ---------------------------------------------------------------------------
// The call protocol, decoded from the kernel's wire shape
// ---------------------------------------------------------------------------

/// One `peer/tool/call` frame's `params`, as the kernel sends it
/// (`crates/octos/octos-cli/src/peers/host_tools.rs:2716`). Decoded rather than
/// key-scraped: a field this host gets wrong is a call it cannot serve, and
/// serde says so at the boundary.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolCallParams {
    /// The host-owned peer's slug, or `null` for a host *session* set — which
    /// is what [`HostPeer::register_params`] registers.
    #[serde(default)]
    pub peer: Option<String>,
    pub session_id: String,
    #[serde(default)]
    pub context_id: Option<String>,
    pub turn_id: TurnId,
    pub call_id: String,
    /// The provider's tool-call id (the occurrence within the turn). Part of
    /// the frame's shape and kept so the struct is the frame; the host keys
    /// everything off its own `call_id`, which is what the kernel correlates a
    /// result by.
    #[allow(dead_code)]
    pub tool_call_id: String,
    /// `sha256:<hex>` of the arguments, for the host's own dedupe.
    pub args_digest: String,
    /// Declared name (`zed.workspace.read`).
    pub name: String,
    /// The owning app, as the kernel resolved it from the declaration.
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub caller: Option<ToolCaller>,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub risk: Option<String>,
    /// The kernel's `confirm_required` for the call. Informational here: this
    /// host's own gate ([`HostTool::needs_approval`]) is what decides, and the
    /// kernel raising its own approval for a Zed tool would be a declaration
    /// disagreement the host must not paper over.
    #[serde(default)]
    pub confirm_required: bool,
    /// The kernel's deadline for the call. The host does not enforce it: the
    /// kernel owns the timeout and says so with `peer/tool/cancel`.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub tools_version: Option<u64>,
}

/// Who the kernel says is speaking. The host does not take this as proof of
/// anything (a wire field is data, not authorization); it is cross-checked
/// against the session this process registered.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ToolCaller {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub peer: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub context_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<TurnId>,
}

/// One `peer/tool/cancel` frame's `params` (`reason`: `timeout` | `cancelled`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ToolCancelParams {
    #[serde(default)]
    pub peer: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub call_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// The reason the kernel gave for a call it stopped waiting for, as this host
/// recorded it (`peer/tool/cancel`'s `reason`, or `"cancelled"` when absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelledCall {
    pub call_id: String,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Activity for the console
// ---------------------------------------------------------------------------

/// What the console shows for one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityState {
    /// Received, running (or waiting on the person).
    Running,
    /// Waiting for the person's decision.
    AwaitingApproval,
    Done,
    Failed,
    /// The kernel stopped waiting; nothing ran.
    Cancelled,
    /// The host refused it (authorization, arguments, capacity).
    Refused,
}

impl ActivityState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::AwaitingApproval => "needs approval",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Refused => "refused",
        }
    }

    /// Whether an entry in this state is finished.
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running | Self::AwaitingApproval)
    }
}

/// One line of tool activity.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolActivity {
    pub call_id: String,
    pub tool: String,
    pub state: ActivityState,
    /// A short, human-readable argument summary — never the raw args blob.
    pub summary: String,
    /// The refusal/failure reason, or the success note.
    pub detail: String,
    pub args_digest: String,
}

/// A pending approval the person has to answer.
///
/// Two kinds arrive on this one surface. Zed's own tools are asked for here
/// (the kernel raises nothing for them: they are `confirm: app`). Everything
/// else is the kernel's own: the agent's `shell`, its file writes — the
/// kernel's approval policy decides those, and this host only renders the
/// question and sends the answer back. Both look the same to the person, so
/// they are one type; [`ApprovalSource`] is what tells the host how to answer.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalRequest {
    /// The kernel's id for this approval. [`ApprovalSource::Kernel`] responds
    /// with it; [`ApprovalSource::HostTool`] carries it for
    /// `approval/decided` correlation only.
    pub approval_id: ApprovalId,
    /// The call the approval belongs to. For a host tool this is the
    /// `peer/tool/call` id; for a kernel approval it is the kernel's tool call
    /// id, which the host does not act on.
    pub call_id: String,
    pub source: ApprovalSource,
    pub tool: String,
    /// A short line: what is being asked, and of what.
    pub summary: String,
    /// The body the kernel supplied (`ApprovalRequestedEvent::body`) or the
    /// host wrote. Rendered verbatim — it is the kernel's own description of
    /// what the tool would do.
    pub detail: String,
    /// The kernel's risk label (`ApprovalRequestedEvent::risk`), when given.
    pub risk: Option<String>,
}

/// Who owns the decision's *effect*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalSource {
    /// A Zed host tool ([`HostPeer::decide`] runs it locally; no frame is sent
    /// for the decision itself).
    HostTool,
    /// The kernel's own tool. The answer travels back as
    /// `approval/respond`, and the kernel — not this host — applies it.
    Kernel {
        session_id: String,
        turn_id: TurnId,
    },
}

impl ApprovalSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::HostTool => "zed tool",
            Self::Kernel { .. } => "octos tool",
        }
    }
}

// ---------------------------------------------------------------------------
// The plan: every "may this run?" decision, as a pure function
// ---------------------------------------------------------------------------

/// What the host will do with one `peer/tool/call`.
///
/// This is the whole authorization surface, and it is a value so it can be
/// tested without a kernel: [`HostPeer::plan_call`] builds it, and
/// [`HostPeer::handle_call`] is the only thing that acts on it.
#[derive(Debug, Clone, PartialEq)]
pub enum CallPlan {
    /// Run it now.
    Run(RunPlan),
    /// Ask the person first; run only if they approve.
    AskFirst(RunPlan),
    /// Refuse; the refusal goes back to the kernel as a `host:*` error and
    /// nothing runs.
    Refuse(Refusal),
}

/// Why a call was refused. Each maps to a stable `host:<kind>` the audit and
/// the model can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The call names a session this host does not own.
    ForeignSession,
    /// The caller block contradicts the session (a fabricated origin).
    OriginMismatch,
    /// The tool is not one this host declares.
    UnknownTool,
    /// The tool is a host tool but is declared with a different risk than the
    /// frame carries, or the frame's declaration does not match ours.
    DeclarationMismatch,
    /// The arguments are missing, malformed, or outside the workspace.
    BadArguments,
    /// Too many calls already in flight.
    Busy,
    /// This call id was already seen (a replay or a duplicate frame).
    DuplicateCall,
}

impl Refusal {
    /// The `host:<kind>` error kind the kernel will show the model.
    pub fn kind(self) -> &'static str {
        match self {
            Self::ForeignSession => "host:foreign_session",
            Self::OriginMismatch => "host:origin_mismatch",
            Self::UnknownTool => "host:unknown_tool",
            Self::DeclarationMismatch => "host:declaration_mismatch",
            Self::BadArguments => "host:bad_arguments",
            Self::Busy => "host:busy",
            Self::DuplicateCall => "host:duplicate_call",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::ForeignSession => {
                "this call names a session this Zed process did not open; the host \
                 answers only its own session"
            }
            Self::OriginMismatch => {
                "the frame's caller does not match the session the call arrived on"
            }
            Self::UnknownTool => "no such tool is registered by this host",
            Self::DeclarationMismatch => {
                "the call's declaration does not match the tool this host registered"
            }
            Self::BadArguments => "the arguments are not usable for this tool",
            Self::Busy => "this host already has the maximum number of calls in flight",
            Self::DuplicateCall => "this call id was already handled",
        }
    }
}

/// A call cleared to run (or to be held for approval).
#[derive(Debug, Clone, PartialEq)]
pub struct RunPlan {
    pub call_id: String,
    pub tool: HostTool,
    /// The workspace the arguments resolved against: the session's root.
    pub workspace: PathBuf,
    pub args: Args,
    /// A short summary for the console and the approval card.
    pub summary: String,
}

/// Arguments, already validated and confined. Enum rather than raw JSON so a
/// tool body cannot accidentally see an unvalidated argument.
#[derive(Debug, Clone, PartialEq)]
pub enum Args {
    Read {
        /// Workspace-relative, already checked and joined.
        path: String,
        max_bytes: usize,
    },
    Open {
        path: String,
        line: Option<u32>,
    },
}

// ---------------------------------------------------------------------------
// The executor seam
// ---------------------------------------------------------------------------

/// What a tool body returns.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub ok: bool,
    /// `ok`: the structured result; not `ok`: the message.
    pub body: Value,
}

impl ToolOutcome {
    pub fn ok(data: Value) -> Self {
        Self { ok: true, body: data }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            body: Value::String(message.into()),
        }
    }
}

/// The effect an approved call asked for, handed to the host's UI side.
///
/// A side effect never runs on the kernel's task: it is published here and
/// applied by whoever owns the editor. That is what makes "refused" and
/// "cancelled" mean *nothing happened* rather than "the write happened but the
/// result was discarded".
#[derive(Debug, Clone, PartialEq)]
pub struct EditorOpen {
    /// Absolute, canonical, inside the workspace.
    pub path: PathBuf,
    /// The workspace it belongs to.
    pub workspace: PathBuf,
    pub line: Option<u32>,
}

/// Runs host tools. Implemented by [`BuiltinExecutor`] by default and by the app's own
/// in tests, so every authorization path is testable with no kernel.
pub trait HostToolExecutor: Send + Sync {
    /// Execute one cleared call. `cancelled` is checked by the caller before
    /// and after; an implementation may also consult it.
    fn execute(&self, plan: &RunPlan) -> ToolOutcome;

    /// Publish a side effect for the UI to apply. Called only for a call that
    /// was cleared, approved and still uncancelled.
    fn publish_editor_open(&self, open: EditorOpen);

    /// Note a call the host refused (the console's audit line). The executor
    /// decides where that goes; the default is to do nothing.
    fn note_refusal(&self, _tool: &str, _call_id: &str, _refusal: Refusal) {}
}

/// The seam's default executor: reads are served (filesystem only, no `App`
/// needed) and an editor open is *not* delivered anywhere.
///
/// The app installs its own ([`crate::zed::octos::status_window::EditorOpenExecutor`])
/// that adds the delivery; the default exists so a host with no UI wired yet
/// still answers a read instead of refusing it, and so an open it cannot
/// deliver says so rather than claiming a tab appeared.
pub struct BuiltinExecutor;

impl HostToolExecutor for BuiltinExecutor {
    fn execute(&self, plan: &RunPlan) -> ToolOutcome {
        match &plan.args {
            Args::Read { path, max_bytes } => read_workspace_file(&plan.workspace, path, *max_bytes),
            Args::Open { .. } => ToolOutcome::error(
                "this host has no editor wired to deliver an open; nothing was opened",
            ),
        }
    }

    fn publish_editor_open(&self, _open: EditorOpen) {}
}

/// Canonicalize a path, dropping Windows' verbatim prefix.
///
/// `std::fs::canonicalize` returns `\\?\E:\...` on Windows. The kernel's own
/// binding code strips it (it canonicalizes with `dunce`), so a path this host
/// resolves has to come out in the same shape or a comparison against the
/// session's workspace would fail on the prefix alone. A lexical strip is exact
/// here: canonicalizing a non-UNC path never yields a UNC verbatim path, and a
/// path that already carries `\\?\` *is* in its canonical form.
pub fn canonical_path(path: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        if let Some(text) = canonical.to_str() {
            if let Some(stripped) = text.strip_prefix(r"\\?\") {
                // A verbatim UNC path spells the leading `\\` differently, so
                // it is rebuilt rather than stripped.
                if let Some(unc) = stripped.strip_prefix(r"UNC\") {
                    return PathBuf::from(format!(r"\\{unc}"));
                }
                if !stripped.starts_with(r"\\") {
                    return PathBuf::from(stripped);
                }
            }
        }
    }
    canonical
}

/// Read one confined workspace file, returning its text as the tool's result.
pub fn read_workspace_file(workspace: &Path, relative: &str, max_bytes: usize) -> ToolOutcome {
    let root = canonical_path(workspace);
    if !root.is_dir() {
        return ToolOutcome::error(format!(
            "the session workspace {} is unusable: it is not a directory",
            workspace.display()
        ));
    }
    let candidate = root.join(relative);
    // Canonicalize BEFORE reading: a symlink inside the workspace must not be
    // a way out of it.
    let real = canonical_path(&candidate);
    if !real.starts_with(&root) {
        return ToolOutcome::error(format!(
            "'{relative}' resolves outside the workspace ({})",
            real.display()
        ));
    }
    let bytes = match std::fs::read(&real) {
        Ok(bytes) => bytes,
        Err(error) => return ToolOutcome::error(format!("cannot read '{relative}': {error}")),
    };
    let truncated = bytes.len() > max_bytes;
    let slice = bytes.get(..max_bytes.min(bytes.len())).unwrap_or(&bytes);
    let text = match std::str::from_utf8(slice) {
        Ok(text) => text.to_owned(),
        Err(_) => {
            return ToolOutcome::error(format!(
                "'{relative}' is not UTF-8 text ({} bytes)",
                bytes.len()
            ));
        }
    };
    ToolOutcome::ok(json!({
        "path": relative,
        "bytes": bytes.len(),
        "lines": text.lines().count(),
        "truncated": truncated,
        "text": text,
    }))
}

// ---------------------------------------------------------------------------
// Argument validation
// ---------------------------------------------------------------------------

/// A workspace-relative path that cannot escape the workspace.
///
/// Shape checks only; [`read_workspace_file`] still canonicalizes and re-checks,
/// because a symlink can turn a well-shaped relative path into an escape.
fn confined_relative_path(raw: &str) -> Result<String, Refusal> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Refusal::BadArguments);
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err(Refusal::BadArguments);
    }
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => normal.push(part),
            std::path::Component::CurDir => {}
            _ => return Err(Refusal::BadArguments),
        }
    }
    if normal.as_os_str().is_empty() {
        return Err(Refusal::BadArguments);
    }
    Ok(normal.to_string_lossy().replace('\\', "/"))
}

fn validate_args(tool: &HostTool, args: &Value) -> Result<Args, Refusal> {
    let object = args.as_object().ok_or(Refusal::BadArguments)?;
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .ok_or(Refusal::BadArguments)
        .and_then(confined_relative_path)?;
    match tool.name {
        "zed.workspace.read" => {
            let max_bytes = match object.get("max_bytes") {
                None | Some(Value::Null) => READ_MAX_BYTES,
                Some(value) => value
                    .as_u64()
                    .ok_or(Refusal::BadArguments)?
                    .clamp(1, READ_MAX_BYTES as u64) as usize,
            };
            Ok(Args::Read { path, max_bytes })
        }
        "zed.editor.open" => {
            let line = match object.get("line") {
                None | Some(Value::Null) => None,
                Some(value) => {
                    let line = value.as_u64().ok_or(Refusal::BadArguments)?;
                    if line == 0 || line > u64::from(u32::MAX) {
                        return Err(Refusal::BadArguments);
                    }
                    Some(line as u32)
                }
            };
            Ok(Args::Open { path, line })
        }
        _ => Err(Refusal::UnknownTool),
    }
}

/// A one-line, bounded summary of a call's arguments for the console. Never the
/// whole args blob: this is rendered, and a summary is all a reader needs.
fn summarize(tool: &HostTool, args: &Args) -> String {
    match args {
        Args::Read { path, .. } => format!("{} {path}", tool.name),
        Args::Open { path, line } => match line {
            Some(line) => format!("{} {path}:{line}", tool.name),
            None => format!("{} {path}", tool.name),
        },
    }
}

// ---------------------------------------------------------------------------
// The host peer
// ---------------------------------------------------------------------------

/// One in-flight call.
///
/// The cleared [`RunPlan`] is retained, not re-derived on approval: the plan is
/// the record of what was authorized (session, workspace, validated args), and
/// rebuilding it from a second frame would be a second authorization, not a
/// re-check of the first.
#[derive(Debug, Clone)]
struct PendingCall {
    plan: RunPlan,
    turn_id: TurnId,
    args_digest: String,
    /// The approval the person is being asked, once one was raised.
    approval_id: Option<ApprovalId>,
    /// Whether the call's tool was cleared to run without asking.
    approved: bool,
}

/// Zed's end of the host-tool contract.
///
/// Cloneable and shared: the gpui side registers and answers approvals, the
/// kernel's reader task handles calls. Every field is guarded by the same
/// mutex, so a call, its cancellation and its result cannot interleave into a
/// state none of them intended.
#[derive(Clone)]
pub struct HostPeer {
    inner: Arc<Mutex<PeerState>>,
}

impl Default for HostPeer {
    /// A peer with a working (read-only) executor, so a host that installs no
    /// app executor of its own still serves the tools it declares instead of
    /// refusing every call.
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(PeerState {
                executor: Some(Arc::new(BuiltinExecutor)),
                ..PeerState::default()
            })),
        }
    }
}

#[derive(Default)]
struct PeerState {
    /// The session this host registered on, and the workspace it opened in.
    /// `None` until [`HostPeer::registered`] succeeds.
    registration: Option<Registration>,
    pending: HashMap<String, PendingCall>,
    /// Every call id this host has handled, in arrival order, so the
    /// duplicate/`cancelled` questions are answered from the host's own record
    /// rather than from whatever happens to still be in flight.
    handled: HashMap<String, HandledCall>,
    handled_order: VecDeque<String>,
    activity: Vec<ToolActivity>,
    /// Approvals the person has not answered yet, oldest first.
    approvals: Vec<ApprovalRequest>,
    /// Structured user questions the kernel is blocking a turn on, oldest
    /// first (UPCR-2026-023 `user_question/requested`).
    questions: Vec<AskedQuestions>,
    executor: Option<Arc<dyn HostToolExecutor>>,
}

/// What the host remembers about a call id it has already seen.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HandledCall {
    /// The kernel said it stopped waiting; nothing may take effect.
    cancelled_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Registration {
    session_id: String,
    workspace: PathBuf,
    version: u64,
}

impl HostPeer {
    /// A peer with nothing registered yet. Alias for [`HostPeer::default`], for
    /// callers that want the read-only default executor spelled out.
    #[cfg(test)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Install the executor that runs cleared calls.
    ///
    /// Replaces the default. Set before registering, so no call can arrive
    /// against a half-installed seam.
    pub fn with_executor(&self, executor: Arc<dyn HostToolExecutor>) {
        self.with_state(|state| state.executor = Some(executor));
    }

    fn with_state<T>(&self, body: impl FnOnce(&mut PeerState) -> T) -> T {
        let mut state = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        body(&mut state)
    }

    /// The session this host registered on, if registration succeeded. The call
    /// path uses [`Self::is_registered`]; this is the accessor a host surface
    /// uses to say *which* session the tools belong to.
    #[cfg(test)]
    pub fn registered_session(&self) -> Option<String> {
        self.with_state(|state| state.registration.as_ref().map(|r| r.session_id.clone()))
    }

    /// The workspace every call is confined to: the directory the session was
    /// opened in. `None` before registration, when nothing may run.
    ///
    /// The call path confines through the registration it already holds; this
    /// is the accessor a host surface uses to say *where* calls are confined.
    pub fn workspace(&self) -> Option<PathBuf> {
        self.with_state(|state| state.registration.as_ref().map(|r| r.workspace.clone()))
    }

    /// Whether a tool registration is live. The console shows this, so a person
    /// can tell "no tools" from "the kernel refused them".
    pub fn is_registered(&self) -> bool {
        self.with_state(|state| state.registration.is_some())
    }

    /// The console's tool activity, oldest first.
    pub fn activity(&self) -> Vec<ToolActivity> {
        self.with_state(|state| state.activity.clone())
    }

    /// The approvals waiting on the person, oldest first.
    pub fn approvals(&self) -> Vec<ApprovalRequest> {
        self.with_state(|state| state.approvals.clone())
    }

    /// The structured questions the kernel is waiting on, oldest first.
    pub fn questions(&self) -> Vec<AskedQuestions> {
        self.with_state(|state| state.questions.clone())
    }

    /// Calls the kernel is still waiting on — a host tool's open call, or one
    /// held for the person.
    #[cfg(test)]
    pub fn pending_calls(&self) -> Vec<String> {
        self.with_state(|state| {
            let mut ids: Vec<String> = state
                .pending
                .values()
                .map(|pending| pending.plan.call_id.clone())
                .collect();
            ids.sort();
            ids
        })
    }

    /// The dotted name of the tool `call_id` is running, if it is still
    /// pending. This is what lets a console line rendered from a kernel frame
    /// be matched to the host call it belongs to.
    pub fn pending_tool(&self, call_id: &str) -> Option<String> {
        self.with_state(|state| {
            state
                .pending
                .get(call_id)
                .map(|pending| pending.plan.tool.name.to_owned())
        })
    }

    /// Whether `call_id` was cancelled by the kernel. This is the host's own
    /// record: it survives the call, so a late approval finds it and runs
    /// nothing.
    pub fn cancelled(&self, call_id: &str) -> Option<CancelledCall> {
        self.with_state(|state| {
            let reason = state.handled.get(call_id)?.cancelled_reason.clone()?;
            Some(CancelledCall {
                call_id: call_id.to_owned(),
                reason,
            })
        })
    }

    /// Forget everything. Called when the connection closes: a stale
    /// registration must never outlive the pipe it was made on.
    pub fn reset(&self) {
        self.with_state(|state| {
            state.registration = None;
            state.pending.clear();
            state.handled.clear();
            state.handled_order.clear();
            state.approvals.clear();
            state.questions.clear();
            state.activity.clear();
        });
    }

    // -----------------------------------------------------------------------
    // The kernel's own tools and approvals
    // -----------------------------------------------------------------------

    /// Record one kernel-side tool event (the agent's own tools: `shell`,
    /// file edits, …). These are not this host's tools —
    /// [`super::forward_notifications`] only routes `peer/tool/*` here — but
    /// they are the same person's work, and the console shows one list.
    pub fn note_kernel_tool(&self, call_id: &str, tool: &str, state: ActivityState, detail: String) {
        self.with_state(|peer| {
            if let Some(entry) = peer
                .activity
                .iter_mut()
                .rev()
                .find(|entry| entry.call_id == call_id && !entry.state.is_terminal())
            {
                entry.state = state;
                if !detail.is_empty() {
                    entry.detail = detail.clone();
                }
                return;
            }
            peer.activity.push(ToolActivity {
                call_id: call_id.to_owned(),
                tool: tool.to_owned(),
                state,
                summary: String::new(),
                detail,
                args_digest: String::new(),
            });
            trim_activity(peer);
        });
    }

    /// Raise an approval the *kernel* is asking for, so the person can answer
    /// it on the same surface as the host's own.
    ///
    /// The kernel has already parked its turn on this approval
    /// (`ApprovalRequestedEvent` is emitted only after the park is registered);
    /// the host's job is to render it and send `approval/respond` — never to
    /// act on the tool itself.
    pub fn note_kernel_approval(&self, event: &KernelApproval) {
        self.with_state(|peer| {
            peer.approvals.retain(|request| {
                request.approval_id != event.approval_id
                    || !matches!(request.source, ApprovalSource::Kernel { .. })
            });
            peer.approvals.push(ApprovalRequest {
                approval_id: event.approval_id.clone(),
                call_id: event.tool_call_id.clone().unwrap_or_default(),
                source: ApprovalSource::Kernel {
                    session_id: event.session_id.clone(),
                    turn_id: event.turn_id.clone(),
                },
                tool: event.tool_name.clone(),
                summary: event.title.clone(),
                detail: event.body.clone(),
                risk: event.risk.clone(),
            });
        });
    }

    /// The kernel's answer is already known (a decision, or a cancellation):
    /// drop it from the host's list. The host does not apply anything.
    pub fn forget_kernel_approval(&self, approval_id: &ApprovalId) {
        self.with_state(|peer| {
            peer.approvals.retain(|request| {
                &request.approval_id != approval_id
                    || !matches!(request.source, ApprovalSource::Kernel { .. })
            });
        });
    }

    /// Look one pending approval up.
    pub fn approval(&self, approval_id: &ApprovalId) -> Option<ApprovalRequest> {
        self.with_state(|peer| {
            peer.approvals
                .iter()
                .find(|request| &request.approval_id == approval_id)
                .cloned()
        })
    }

    /// The `approval/respond` params for one pending kernel approval.
    ///
    /// Built from the pending entry rather than from the caller's arguments:
    /// the session and the approval id are the kernel's, and a caller must not
    /// be able to answer an approval on a session this host never saw.
    pub fn approval_respond_params(&self, approval_id: &ApprovalId, approve: bool) -> Option<Value> {
        let request = self.approval(approval_id)?;
        let ApprovalSource::Kernel { session_id, .. } = &request.source else {
            return None;
        };
        let params = octos_core::ui_protocol::ApprovalRespondParams {
            session_id: octos_core::SessionKey(session_id.clone()),
            approval_id: approval_id.clone(),
            decision: if approve {
                octos_core::ui_protocol::ApprovalDecision::Approve
            } else {
                octos_core::ui_protocol::ApprovalDecision::Deny
            },
            approval_scope: None,
            client_note: None,
        };
        serde_json::to_value(params).ok()
    }

    /// Take a kernel approval off the list and return it, once.
    pub fn take_kernel_approval(&self, approval_id: &ApprovalId) -> Option<ApprovalRequest> {
        self.with_state(|peer| {
            let index = peer.approvals.iter().position(|request| {
                &request.approval_id == approval_id
                    && matches!(request.source, ApprovalSource::Kernel { .. })
            })?;
            Some(peer.approvals.remove(index))
        })
    }

    // -----------------------------------------------------------------------
    // The kernel's structured questions (UPCR-2026-023)
    // -----------------------------------------------------------------------

    /// Raise a question the *kernel* is blocked on, so the person answers it on
    /// the console.
    ///
    /// The kernel has already parked a turn on it: `user_question/requested` is
    /// emitted only after the pending entry exists and the waiting oneshot is
    /// registered (`crates/octos/octos-cli/src/api/ui_protocol_transport.rs:6854`),
    /// and that await has no timeout of its own (`:6952`). The host's job is to
    /// render it and answer it — an unanswered one hangs the turn.
    pub fn note_kernel_question(&self, asked: &AskedQuestions) {
        self.with_state(|peer| {
            peer.questions
                .retain(|existing| existing.question_id != asked.question_id);
            peer.questions.push(asked.clone());
        });
    }

    /// Drop one question from the console's list, whether it was answered,
    /// dismissed or cancelled by the kernel.
    pub fn forget_kernel_question(&self, question_id: &QuestionId) {
        self.with_state(|peer| {
            peer.questions
                .retain(|existing| &existing.question_id != question_id);
        });
    }

    /// Take one question off the list and return it, once.
    pub fn take_kernel_question(&self, question_id: &QuestionId) -> Option<AskedQuestions> {
        self.with_state(|peer| {
            let index = peer
                .questions
                .iter()
                .position(|existing| &existing.question_id == question_id)?;
            Some(peer.questions.remove(index))
        })
    }

    /// Look one pending question up.
    pub fn question(&self, question_id: &QuestionId) -> Option<AskedQuestions> {
        self.with_state(|peer| {
            peer.questions
                .iter()
                .find(|existing| &existing.question_id == question_id)
                .cloned()
        })
    }

    /// Record the person's choice of one option on one question. Returns
    /// whether the label was a real option of that question.
    ///
    /// Only a label the kernel itself offered can be selected: the server
    /// validates every `selected_labels` entry against the request it stored
    /// (`validate_answers`, `crates/octos/octos-cli/src/contracts/questions.rs:293`)
    /// and rejects the whole answer otherwise, so the host must never invent
    /// one. Clicking the chosen option again clears it — an answer may be
    /// empty, and clearing is how a mis-click is undone.
    pub fn select_question_option(
        &self,
        question_id: &QuestionId,
        question_index: usize,
        label: &str,
    ) -> bool {
        self.with_state(|peer| {
            let Some(asked) = peer
                .questions
                .iter_mut()
                .find(|existing| &existing.question_id == question_id)
            else {
                return false;
            };
            let Some(question) = asked.questions.get(question_index) else {
                return false;
            };
            if !question
                .options
                .iter()
                .any(|option| option.label == label)
            {
                return false;
            }
            let multi_select = question.multi_select;
            let Some(selection) = asked.selections.get_mut(question_index) else {
                return false;
            };
            if multi_select {
                match selection.iter().position(|chosen| chosen == label) {
                    Some(index) => {
                        selection.remove(index);
                    }
                    None => selection.push(label.to_owned()),
                }
            } else if selection.len() == 1 && selection[0] == label {
                selection.clear();
            } else {
                *selection = vec![label.to_owned()];
            }
            true
        })
    }

    /// Whether any part of one pending question accepts free text, so the
    /// console can say what will happen to what the person typed.
    pub fn question_accepts_free_text(&self, question_id: &QuestionId) -> bool {
        self.question(question_id).is_some_and(|asked| {
            asked
                .questions
                .iter()
                .any(|question| question.allow_free_text)
        })
    }

    /// The `user_question/respond` params for one pending question.
    ///
    /// Built from the stored entry — the kernel's session, the kernel's
    /// `question_id`, one answer per question it asked, and only labels it
    /// offered — plus the person's selections and `free_text`. Nothing here is
    /// taken from a caller's arguments, so a stale surface cannot answer a
    /// question the kernel has already replaced.
    ///
    /// `client_note` is what distinguishes an answer from a dismissal:
    /// declining to answer sends the same one-(empty)-answer-per-question
    /// payload with a note saying so, which is the only "no answer" the
    /// protocol has — the server validates the answer *count* and the labels
    /// (`validate_answers`, `crates/octos/octos-cli/src/contracts/questions.rs:293`)
    /// but never requires a non-empty answer, and an empty one unblocks the
    /// waiting tool as `Answered` rather than hanging the turn.
    pub fn question_respond_params(
        &self,
        question_id: &QuestionId,
        free_text: Option<&str>,
        client_note: Option<&str>,
    ) -> Option<Value> {
        let asked = self.question(question_id)?;
        let mut answers: Vec<UserQuestionAnswer> = asked
            .selections
            .iter()
            .map(|selected| UserQuestionAnswer {
                selected_labels: selected.clone(),
                free_text: None,
            })
            .collect();
        // The wire shape has one answer per question; a stored entry that lost
        // that pairing for any reason is not answerable.
        if answers.len() != asked.questions.len() {
            log::error!(
                "[octos] question {} has {} question(s) but {} answer(s) recorded; refusing to answer",
                asked.question_id.0,
                asked.questions.len(),
                answers.len()
            );
            return None;
        }
        if let Some(text) = free_text.map(str::trim).filter(|text| !text.is_empty()) {
            // Free text goes to the first question that allows it: the protocol
            // carries one optional free text per question, and the console has
            // one box. A question that allows none is reported, never silently
            // dropped onto a question the server would then reject.
            match asked
                .questions
                .iter()
                .position(|question| question.allow_free_text)
            {
                Some(index) => answers[index].free_text = Some(text.to_owned()),
                None => log::warn!(
                    "[octos] none of the kernel's questions allows free text; \
                     the typed answer is not being sent"
                ),
            }
        }
        let params = UserQuestionRespondParams {
            session_id: octos_core::SessionKey(asked.session_id.clone()),
            question_id: asked.question_id.clone(),
            answers,
            client_note: client_note.map(ToOwned::to_owned),
        };
        match serde_json::to_value(params) {
            Ok(params) => Some(params),
            Err(error) => {
                log::error!("[octos] could not encode user_question/respond: {error}");
                None
            }
        }
    }
}

/// One structured user question the kernel is blocking a turn on, as this host
/// renders and answers it (UPCR-2026-023 `user_question/requested`).
///
/// Mirrors [`KernelApproval`]: the kernel owns the question and the answer
/// contract, the host owns the surface, and answering it is exactly one
/// `user_question/respond` frame. `selections` is the person's in-progress
/// choice; the wire payload is built from the stored entry in
/// [`HostPeer::question_respond_params`], so a click can never answer a
/// question the kernel has already replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskedQuestions {
    pub session_id: String,
    pub question_id: QuestionId,
    pub turn_id: TurnId,
    pub title: String,
    pub body: String,
    pub questions: Vec<octos_core::ui_protocol::UserQuestion>,
    /// One entry per question, in question order: the option labels chosen so
    /// far, empty when the person has not chosen one.
    pub selections: Vec<Vec<String>>,
}

impl AskedQuestions {
    /// Read one from the notification's typed event
    /// (`UserQuestionRequestedEvent`, `crates/octos/octos-core/src/ui_protocol.rs:5864`).
    pub fn from_event(event: &octos_core::ui_protocol::UserQuestionRequestedEvent) -> Self {
        Self {
            session_id: event.session_id.0.clone(),
            question_id: event.question_id.clone(),
            turn_id: event.turn_id.clone(),
            title: event.title.clone(),
            body: event.body.clone(),
            selections: vec![Vec::new(); event.questions.len()],
            questions: event.questions.clone(),
        }
    }
}

/// The parts of an `ApprovalRequestedEvent`
/// (`crates/octos/octos-core/src/ui_protocol.rs:5690`) this host renders and answers.
///
/// A struct rather than the event itself so the console module does not depend
/// on the whole kernel payload surface, and so the decode lives next to the
/// decision that uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelApproval {
    pub session_id: String,
    pub approval_id: ApprovalId,
    pub turn_id: TurnId,
    pub tool_name: String,
    pub title: String,
    pub body: String,
    pub risk: Option<String>,
    /// The kernel's tool call id, when the payload carries one in its typed
    /// details.
    pub tool_call_id: Option<String>,
}

impl KernelApproval {
    /// Read one from the notification's typed event.
    ///
    /// The kernel's own tool call id lives in whichever typed-details family
    /// the payload carries (a host-routed tool's, or a shell command's); both
    /// are read here rather than assumed. `None` is a real answer — a payload
    /// with no call id still renders and still answers.
    pub fn from_event(event: &octos_core::ui_protocol::ApprovalRequestedEvent) -> Self {
        let tool_call_id = event
            .typed_details
            .as_ref()
            .and_then(|details| {
                details
                    .host_tool
                    .as_ref()
                    .and_then(|host_tool| host_tool.tool_call_id.clone())
                    .or_else(|| {
                        details
                            .command
                            .as_ref()
                            .and_then(|command| command.tool_call_id.clone())
                    })
            });
        Self {
            session_id: event.session_id.0.clone(),
            approval_id: event.approval_id.clone(),
            turn_id: event.turn_id.clone(),
            tool_name: event.tool_name.clone(),
            title: event.title.clone(),
            body: event.body.clone(),
            risk: event.risk.clone(),
            tool_call_id,
        }
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

impl HostPeer {
    /// The `peer/tools/register` params for a session.
    ///
    /// `peer` is deliberately absent: this is a *session* set, which is the
    /// shape for a host that owns its own stdio pipe (see the module docs).
    /// `generic_tools` is absent too — an empty/`None` list means "the
    /// session's usual kernel tools", and narrowing them is the host's policy
    /// decision, not a side effect of wanting Zed tools.
    pub fn register_params(session: &str) -> Value {
        json!({
            "session_id": session,
            "tools": host_tools()
                .into_iter()
                .map(|tool| json!({
                    "name": tool.name,
                    "app": APP,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                    "risk": tool.risk,
                    // A host tool that reaches past the app would say so here;
                    // neither of Zed's does.
                    "outward": false,
                    // `confirm: app` keeps the *kernel* from raising its own
                    // approval for a gated tool: the decision belongs where the
                    // effect would happen (see the module docs on the default
                    // safe posture).
                    "confirm": "app",
                    "background": false
                }))
                .collect::<Vec<Value>>(),
            // Mirrors of the kernel's own defaults
            // (`crates/octos/octos-cli/src/peers/host_tools.rs:65`), pinned explicitly
            // so a change on either side is visible here rather than a silent
            // behaviour shift.
            "call_timeout_ms": CALL_TIMEOUT_MS,
            "max_result_bytes": MAX_RESULT_BYTES,
        })
    }

    /// Record a successful registration.
    pub fn registered(&self, session: &str, workspace: PathBuf, result: &Value) -> Result<u64, String> {
        let version = result
            .get("version")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("the kernel's register reply has no version: {result}"))?;
        let returned = result
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if returned != session {
            return Err(format!(
                "the kernel registered session '{returned}', not '{session}'"
            ));
        }
        self.with_state(|state| {
            state.registration = Some(Registration {
                session_id: session.to_owned(),
                workspace,
                version,
            });
        });
        Ok(version)
    }

    /// Record a registration failure so the console can show it. Returns the
    /// kernel's error text.
    pub fn registration_failed(&self, error: &Value) -> String {
        let text = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| error.to_string());
        self.with_state(|state| {
            state.activity.push(ToolActivity {
                call_id: String::new(),
                tool: "(registration)".to_owned(),
                state: ActivityState::Refused,
                summary: "peer/tools/register".to_owned(),
                detail: text.clone(),
                args_digest: String::new(),
            });
            trim_activity(state);
        });
        text
    }
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

impl HostPeer {
    /// Decode one `peer/tool/call` frame's params. `Err` is a frame this host
    /// cannot even read — logged raw by the caller, never guessed at.
    pub fn decode_call(params: &Value) -> Result<ToolCallParams, RpcError> {
        serde_json::from_value(params.clone())
            .map_err(|error| RpcError::invalid_params(format!("peer/tool/call: {error}")))
    }

    /// Decode one `peer/tool/cancel` frame's params.
    pub fn decode_cancel(params: &Value) -> Result<ToolCancelParams, RpcError> {
        serde_json::from_value(params.clone())
            .map_err(|error| RpcError::invalid_params(format!("peer/tool/cancel: {error}")))
    }

    /// The whole authorization decision for one call.
    ///
    /// Pure: it reads the registration and the pending table, and decides. It
    /// touches no filesystem, no connection and no executor, so the negative
    /// cases (a foreign session, a fabricated caller, an unknown tool, a
    /// path that leaves the workspace) are testable without a kernel.
    pub fn plan_call(&self, call: &ToolCallParams) -> CallPlan {
        let registration = match self.with_state(|state| state.registration.clone()) {
            Some(registration) => registration,
            None => return CallPlan::Refuse(Refusal::ForeignSession),
        };
        // 1. The session: the kernel labels every call with the session it was
        //    made on. This process answers exactly one.
        if call.session_id != registration.session_id {
            return CallPlan::Refuse(Refusal::ForeignSession);
        }
        // 2. The caller block: a wire field, so it is not proof — but it must
        //    at least not contradict the session the call arrived on. A frame
        //    claiming another session or another peer's turn is not this
        //    host's call, whatever it says about itself.
        if let Some(caller) = &call.caller {
            if caller
                .session_id
                .as_deref()
                .is_some_and(|session| session != registration.session_id)
            {
                return CallPlan::Refuse(Refusal::OriginMismatch);
            }
            if caller.context_id.is_some() || call.context_id.is_some() {
                // Request contexts are app-peer constructs; this host registers
                // on a session, and a call claiming a context is not one.
                return CallPlan::Refuse(Refusal::OriginMismatch);
            }
            if caller.peer.is_some() || call.peer.is_some() {
                return CallPlan::Refuse(Refusal::ForeignSession);
            }
        } else if call.peer.is_some() {
            return CallPlan::Refuse(Refusal::ForeignSession);
        }
        // 3. The tool and its declaration. The frame carries the declared risk
        //    the kernel gated on; if it disagrees with what this host declared,
        //    the kernel and the host are not talking about the same tool.
        let Some(tool) = tool(&call.name) else {
            return CallPlan::Refuse(Refusal::UnknownTool);
        };
        if let Some(risk) = call.risk.as_deref() {
            if risk != tool.risk {
                return CallPlan::Refuse(Refusal::DeclarationMismatch);
            }
        }
        // 4. Replay/duplicate: one call id is handled once, ever. The record
        //    lives in the host, not in the in-flight set, so a frame replayed
        //    after the call finished is a duplicate too.
        let known = self.with_state(|state| state.handled.contains_key(&call.call_id));
        if known {
            return CallPlan::Refuse(Refusal::DuplicateCall);
        }
        // 5. Capacity.
        let full = self.with_state(|state| state.pending.len() >= MAX_PENDING_CALLS);
        if full {
            return CallPlan::Refuse(Refusal::Busy);
        }
        // 6. Arguments, confined to the workspace the session was opened in.
        let args = match validate_args(&tool, &call.args) {
            Ok(args) => args,
            Err(refusal) => return CallPlan::Refuse(refusal),
        };
        let summary = summarize(&tool, &args);
        let plan = RunPlan {
            call_id: call.call_id.clone(),
            tool,
            workspace: registration.workspace.clone(),
            args,
            summary,
        };
        // 7. The person's gate. A tool the host marked as needing approval runs
        //    under no circumstances before one is given: the default posture is
        //    refusal, and approval is the exception.
        if plan.tool.needs_approval {
            CallPlan::AskFirst(plan)
        } else {
            CallPlan::Run(plan)
        }
    }

    /// Handle one `peer/tool/call`: authorize, then run, hold, or refuse, and
    /// answer the kernel on `connection`.
    ///
    /// Answers the kernel at most once, and never with an optimistic result: a
    /// refusal and a completed run are final answers; a held call is answered
    /// with the kernel's `awaiting_confirmation` acknowledgement and only
    /// receives its real outcome when the person decides ([`Self::decide`]).
    pub fn handle_call(&self, connection: &Connection, call: &ToolCallParams) -> Outcome {
        let plan = self.plan_call(call);
        match plan {
            CallPlan::Refuse(refusal) => {
                self.record(call, ActivityState::Refused, refusal.message().to_owned());
                let executor = self.with_state(|state| state.executor.clone());
                if let Some(executor) = executor {
                    executor.note_refusal(&call.name, &call.call_id, refusal);
                }
                let sent = connection.notify(
                    PEER_TOOL_RESULT,
                    error_result(&call.session_id, &call.call_id, refusal.kind(), refusal.message()),
                );
                log::info!(
                    "[octos] refused host tool call {} ({}): {} — answered={sent}",
                    call.name,
                    call.call_id,
                    refusal.kind()
                );
                Outcome::Refused(refusal)
            }
            CallPlan::Run(plan) => {
                self.track(call, &plan, true, None);
                let outcome = self.run(&plan);
                self.finish(&call.session_id, &call.call_id, &outcome, connection);
                Outcome::Ran
            }
            CallPlan::AskFirst(plan) => {
                let approval_id = ApprovalId::new();
                self.track(call, &plan, false, Some(approval_id.clone()));
                let request = ApprovalRequest {
                    approval_id: approval_id.clone(),
                    call_id: plan.call_id.clone(),
                    source: ApprovalSource::HostTool,
                    tool: plan.tool.name.to_owned(),
                    summary: plan.summary.clone(),
                    detail: format!(
                        "{} wants to run {} inside {}.",
                        caller_label(call),
                        plan.tool.name,
                        plan.workspace.display()
                    ),
                    risk: call.risk.clone(),
                };
                self.with_state(|state| {
                    state.approvals.push(request.clone());
                    if let Some(entry) = state.activity.iter_mut().rev().find(|entry| {
                        entry.call_id == plan.call_id && !entry.state.is_terminal()
                    }) {
                        entry.state = ActivityState::AwaitingApproval;
                    }
                });
                // The kernel is told the call is *held*, not answered. Its
                // router ignores an "awaiting confirmation" ack for a
                // non-gated tool; for a gated one it extends the deadline to
                // the approval TTL (`peer_host_tool.rs`'s `ack.notified()`
                // arm). Either way the model gets no result until the person
                // decides.
                let sent = connection.notify(
                    PEER_TOOL_RESULT,
                    json!({
                        "session_id": call.session_id,
                        "call_id": call.call_id,
                        "status": "awaiting_confirmation",
                    }),
                );
                log::info!(
                    "[octos] host tool call {} ({}) is waiting on the person, approval {} \
                     (held={sent})",
                    plan.tool.name,
                    plan.call_id,
                    approval_id.0
                );
                Outcome::AwaitingApproval(approval_id)
            }
        }
    }

    fn track(&self, call: &ToolCallParams, plan: &RunPlan, approved: bool, approval_id: Option<ApprovalId>) {
        // The turn and the arguments digest are part of what makes one call
        // distinguishable from another in the log: a repeated tool name with a
        // different digest is a different call, and that is the whole reason
        // the kernel computes one.
        log::info!(
            "[octos] host tool call {} ({}, turn {}) args={}",
            plan.tool.name,
            plan.call_id,
            call.turn_id.0,
            call.args_digest
        );
        self.with_state(|state| {
            // Remembered as handled from the moment work starts: a replayed
            // frame for a running or held call is a duplicate, not a second run.
            remember_handled(state, &call.call_id, None);
            state.pending.insert(
                call.call_id.clone(),
                PendingCall {
                    plan: plan.clone(),
                    turn_id: call.turn_id.clone(),
                    args_digest: call.args_digest.clone(),
                    approval_id,
                    approved,
                },
            );

            state.activity.push(ToolActivity {
                call_id: call.call_id.clone(),
                tool: plan.tool.name.to_owned(),
                state: if approved {
                    ActivityState::Running
                } else {
                    ActivityState::AwaitingApproval
                },
                summary: plan.summary.clone(),
                detail: String::new(),
                args_digest: call.args_digest.clone(),
            });
            trim_activity(state);
        });
    }

    /// Run a cleared call through the installed executor.
    fn run(&self, plan: &RunPlan) -> ToolOutcome {
        let executor = self.with_state(|state| state.executor.clone());
        match executor {
            Some(executor) => executor.execute(plan),
            // No executor: the host has no way to run the tool. Refuse rather
            // than answer `ok` with a lie.
            None => ToolOutcome::error("this host has no executor installed for its tools"),
        }
    }

    /// Record one refused (or impossible) call on the console's activity list.
    fn record(&self, call: &ToolCallParams, state: ActivityState, detail: String) {
        self.with_state(|peer| {
            peer.activity.push(ToolActivity {
                call_id: call.call_id.clone(),
                tool: call.name.clone(),
                state,
                summary: String::new(),
                detail,
                args_digest: call.args_digest.clone(),
            });
            trim_activity(peer);
        });
    }

    /// Answer the kernel with a finished call's outcome, taking the call out of
    /// the pending table first.
    fn finish(&self, session: &str, call_id: &str, outcome: &ToolOutcome, connection: &Connection) {
        let captured = self.with_state(|state| state.pending.remove(call_id));
        match captured {
            Some(_pending) => self.answer(session, call_id, outcome, connection),
            None => log::info!(
                "[octos] host tool call {call_id} finished after the kernel stopped waiting; \
                 the result is dropped"
            ),
        }
    }

    /// Send one completed call's result. The caller owns the pending entry's
    /// lifetime — this only records the outcome and writes the frame, so a call
    /// whose entry [`Self::decide`] already took out is still answered.
    fn answer(&self, session: &str, call_id: &str, outcome: &ToolOutcome, connection: &Connection) {
        self.with_state(|state| {
            if let Some(entry) = state
                .activity
                .iter_mut()
                .rev()
                .find(|entry| entry.call_id == call_id && !entry.state.is_terminal())
            {
                entry.state = if outcome.ok {
                    ActivityState::Done
                } else {
                    ActivityState::Failed
                };
                entry.detail = detail_of(&outcome.body);
            }
        });
        let params = if outcome.ok {
            json!({
                "session_id": session,
                "call_id": call_id,
                "ok": true,
                "data": outcome.body,
            })
        } else {
            json!({
                "session_id": session,
                "call_id": call_id,
                "ok": false,
                "error": { "kind": "tool_failed", "message": bounded(&outcome.body.to_string()) },
            })
        };
        let sent = connection.notify(PEER_TOOL_RESULT, params);
        log::info!(
            "[octos] answered host tool call {call_id} (ok={}) delivered={sent}",
            outcome.ok
        );
    }

    /// The kernel stopped waiting for `call_id`: nothing about this call may
    /// take effect any more.
    ///
    /// The pending entry is removed *before* anything else, so a concurrent
    /// approval cannot see a live call; a tool already running will have its
    /// result dropped by [`Self::finish`] (its `pending.remove` finds nothing).
    pub fn handle_cancel(&self, cancel: &ToolCancelParams) {
        let reason = cancel.reason.clone().unwrap_or_else(|| "cancelled".to_owned());
        let removed = self.with_state(|state| {
            // The call id is remembered whatever session the frame names: a
            // late result for it must be dropped either way, and a cancel is
            // not authorization *for* anything — it only ever stops work.
            let removed = state.pending.remove(&cancel.call_id).is_some();
            remember_handled(
                state,
                &cancel.call_id,
                Some(reason.clone()),
            );
            state.approvals.retain(|request| request.call_id != cancel.call_id);
            if removed {
                if let Some(entry) = state
                    .activity
                    .iter_mut()
                    .rev()
                    .find(|entry| entry.call_id == cancel.call_id && !entry.state.is_terminal())
                {
                    entry.state = ActivityState::Cancelled;
                    entry.detail = format!("the kernel stopped waiting ({reason})");
                }
            }
            removed
        });
        log::info!(
            "[octos] host tool call {} cancelled by the kernel ({reason}); was pending={removed}",
            cancel.call_id
        );
    }

    // -----------------------------------------------------------------------
    // The person's decision
    // -----------------------------------------------------------------------

    /// Record the person's decision for `call_id` and, if it was an approval
    /// and the call is still live, run it and answer the kernel.
    ///
    /// This is the host's own gate for its own tools (the tools are declared
    /// `confirm: app`, so the kernel raises no approval of its own for them).
    /// A denial is a *refusal*: nothing runs, and the kernel gets the denial as
    /// the call's result so the model can move on.
    pub fn decide(&self, call_id: &str, approve: bool, connection: &Connection) -> Decision {
        let taken = self.with_state(|state| {
            let registration = state.registration.clone()?;
            // Peek before taking: a call that never needed the person is left
            // exactly where it was, so a stray click cannot drop a live result.
            if state.pending.get(call_id)?.approved {
                return None;
            }
            let pending = state.pending.remove(call_id)?;
            state.approvals.retain(|request| request.call_id != call_id);
            Some((registration, pending))
        });
        let Some((registration, pending)) = taken else {
            // Cancelled, unknown, already decided, or never gated. Nothing
            // happens — this is the path that makes "denied" and "cancelled"
            // both mean no effect: the pending entry is gone, so there is no
            // plan left to run.
            log::info!("[octos] approval for call {call_id} arrived with no live call; ignored");
            return if self.cancelled(call_id).is_some() {
                Decision::Cancelled
            } else {
                Decision::NoSuchCall
            };
        };
        if !approve {
            let outcome = ToolOutcome::error(format!(
                "the person denied {} for this call",
                pending.plan.tool.name
            ));
            self.answer(&registration.session_id, call_id, &outcome, connection);
            log::info!(
                "[octos] denied host tool call {call_id} ({})",
                pending.plan.tool.name
            );
            return Decision::Denied;
        }
        // One last cancellation check *after* the plan was taken: a cancel that
        // landed while the person was deciding finds no pending entry (it is
        // already out), so the only thing that can have set this is a cancel
        // before the take-out. Either way a cancelled call runs nothing.
        if self.cancelled(call_id).is_some() {
            self.with_state(|state| {
                if let Some(entry) = state
                    .activity
                    .iter_mut()
                    .rev()
                    .find(|entry| entry.call_id == call_id && !entry.state.is_terminal())
                {
                    entry.state = ActivityState::Cancelled;
                    entry.detail = "the kernel stopped waiting before the decision".to_owned();
                }
            });
            return Decision::Cancelled;
        }
        log::info!(
            "[octos] person approved host tool call {} ({}, approval {}); it was held for \
             turn {} with args {}",
            call_id,
            pending.plan.tool.name,
            pending
                .approval_id
                .as_ref()
                .map(|approval| approval.0.to_string())
                .unwrap_or_else(|| "none".to_owned()),
            pending.turn_id.0,
            pending.args_digest,
        );
        let outcome = self.run(&pending.plan);
        self.answer(&registration.session_id, call_id, &outcome, connection);
        Decision::Ran
    }
}

/// What [`HostPeer::decide`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Ran,
    Denied,
    NoSuchCall,
    Cancelled,
}

/// What [`HostPeer::handle_call`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Cleared and run.
    Ran,
    /// Refused; nothing ran.
    Refused(Refusal),
    /// Held for the person.
    AwaitingApproval(ApprovalId),
}

fn caller_label(call: &ToolCallParams) -> String {
    match call.caller.as_ref().and_then(|caller| caller.kind.as_deref()) {
        Some("system") => "the system agent".to_owned(),
        Some("app_peer") => "the app".to_owned(),
        Some(other) => other.to_owned(),
        None => "the agent".to_owned(),
    }
}

fn error_result(session: &str, call_id: &str, kind: &str, message: &str) -> Value {
    json!({
        "session_id": session,
        "call_id": call_id,
        "ok": false,
        "error": { "kind": kind, "message": bounded(message) },
    })
}

/// Cap a string on a char boundary so a kernel frame never exceeds the error
/// budget the kernel checks (`PEER_TOOL_ERROR_MESSAGE_MAX_BYTES`).
fn bounded(text: &str) -> String {
    if text.len() <= MAX_ERROR_BYTES {
        return text.to_owned();
    }
    let mut cut = MAX_ERROR_BYTES;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut capped = text.get(..cut).unwrap_or_default().to_owned();
    capped.push('…');
    capped
}

fn detail_of(body: &Value) -> String {
    match body {
        Value::String(message) => bounded(message),
        other => bounded(&other.to_string()),
    }
}

fn trim_activity(state: &mut PeerState) {
    if state.activity.len() > ACTIVITY_KEEP {
        let excess = state.activity.len() - ACTIVITY_KEEP;
        state.activity.drain(..excess);
    }
}

/// Record that `call_id` was handled (optionally: cancelled, and why).
///
/// Bounded from the front: a long-lived host must not grow an unbounded record
/// of every call it ever saw. Dropping the oldest id only means an ancient
/// replay would be re-planned, which is a re-run of the host's own checks, not
/// a second effect for a live call.
fn remember_handled(state: &mut PeerState, call_id: &str, cancelled_reason: Option<String>) {
    if let Some(existing) = state.handled.get_mut(call_id) {
        // A cancel after the call was already handled still has to be
        // remembered: the kernel stopped waiting, so a late answer is dead.
        if cancelled_reason.is_some() {
            existing.cancelled_reason = cancelled_reason;
        }
        return;
    }
    state.handled.insert(
        call_id.to_owned(),
        HandledCall {
            cancelled_reason,
        },
    );
    state.handled_order.push_back(call_id.to_owned());
    while state.handled_order.len() > HANDLED_CALLS_KEEP {
        if let Some(oldest) = state.handled_order.pop_front() {
            state.handled.remove(&oldest);
        }
    }
}

/// The method names this module speaks, re-exported from `octos-core` so the
/// host and the wire agree by construction rather than by two matching string
/// literals. The two that are *not* in `octos_core::ui_protocol::methods` are
/// above ([`PEER_TOOLS_REGISTER`], [`PEER_TOOL_RESULT`]): the OUP client never
/// sends them, so core does not name them.
pub const PEER_TOOL_CALL: &str = methods::PEER_TOOL_CALL;
pub const PEER_TOOL_CANCEL: &str = methods::PEER_TOOL_CANCEL;
/// The host's answer to an approval the *kernel* raised.
pub const APPROVAL_RESPOND: &str = methods::APPROVAL_RESPOND;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The session every test registers on, and the other one a hostile frame
    /// would name.
    const SESSION: &str = "octos:main:zed#embedded";
    const OTHER_SESSION: &str = "octos:main:zed#somebody-elses";

    /// A uniquely-named scratch directory outside the repository.
    ///
    /// Not `tempfile`: that crate is an optional dependency of this binary
    /// (behind the `test-support` feature), so a `#[cfg(test)]` module cannot
    /// rely on it being compiled in. `create_dir` is the atomic step, so a
    /// collision is retried rather than silently shared.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let base = std::env::temp_dir();
            for attempt in 0..64 {
                let candidate = base.join(format!(
                    "zed-octos-test-{}-{attempt}",
                    uuid::Uuid::new_v4().simple()
                ));
                match std::fs::create_dir(&candidate) {
                    Ok(()) => return Self(candidate),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("could not create {}: {error}", candidate.display()),
                }
            }
            panic!("could not find a free scratch name under {}", base.display());
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            // Best effort: a leaked temp dir is noise, not a test failure.
            if let Err(error) = std::fs::remove_dir_all(&self.0) {
                eprintln!("could not clean up {}: {error}", self.0.display());
            }
        }
    }

    /// A workspace with one readable file, so the read tool has something real
    /// to confine and resolve.
    fn workspace() -> Scratch {
        let dir = Scratch::new();
        std::fs::write(dir.path().join("hello.txt"), "one\ntwo\nthree\n").expect("seed file");
        std::fs::create_dir_all(dir.path().join("nested")).expect("nested dir");
        std::fs::write(dir.path().join("nested/inner.txt"), "inner").expect("inner file");
        dir
    }

    /// Records what it was asked to run, and can be told to report failure.
    #[derive(Default)]
    struct FakeExecutor {
        planned: Mutex<Vec<String>>,
        opens: Mutex<Vec<EditorOpen>>,
        refusals: Mutex<Vec<(String, &'static str)>>,
        fail: bool,
    }

    impl FakeExecutor {
        fn ran(&self) -> Vec<String> {
            self.planned.lock().expect("planned").clone()
        }

        fn opens(&self) -> Vec<EditorOpen> {
            self.opens.lock().expect("opens").clone()
        }
    }

    impl HostToolExecutor for FakeExecutor {
        fn execute(&self, plan: &RunPlan) -> ToolOutcome {
            self.planned
                .lock()
                .expect("planned")
                .push(plan.tool.name.to_owned());
            if self.fail {
                return ToolOutcome::error("the fake refused to run");
            }
            match &plan.args {
                Args::Open { path, line } => {
                    self.publish_editor_open(EditorOpen {
                        path: plan.workspace.join(path),
                        workspace: plan.workspace.clone(),
                        line: *line,
                    });
                    ToolOutcome::ok(json!({ "opened": path }))
                }
                Args::Read { path, max_bytes } => {
                    read_workspace_file(&plan.workspace, path, *max_bytes)
                }
            }
        }

        fn publish_editor_open(&self, open: EditorOpen) {
            self.opens.lock().expect("opens").push(open);
        }

        fn note_refusal(&self, tool: &str, _call_id: &str, refusal: Refusal) {
            self.refusals
                .lock()
                .expect("refusals")
                .push((tool.to_owned(), refusal.kind()));
        }
    }

    /// A registered host peer with a recording executor.
    fn host(workspace: &Path) -> (HostPeer, Arc<FakeExecutor>) {
        let peer = HostPeer::new();
        let executor = Arc::new(FakeExecutor::default());
        peer.with_executor(executor.clone());
        peer.registered(SESSION, workspace.to_path_buf(), &json!({
            "session_id": SESSION,
            "version": 1,
        }))
        .expect("registration");
        (peer, executor)
    }

    /// One `peer/tool/call`'s params, built the way the kernel builds them
    /// (`crates/octos/octos-cli/src/peers/host_tools.rs:2716`): the **declared** name
    /// (`zed.editor.open`), the risk the declaration carries, and the caller
    /// block the kernel fills in.
    fn call_with_risk(name: &str, args: Value, risk: &str) -> ToolCallParams {
        serde_json::from_value(json!({
            "peer": null,
            "session_id": SESSION,
            "turn_id": TurnId::new(),
            "call_id": format!("ptc-{}", uuid::Uuid::new_v4().simple()),
            "tool_call_id": "tc-1",
            "args_digest": "sha256:0000",
            "name": name,
            "app": "zed",
            "caller": {
                "kind": "system",
                "peer": null,
                "session_id": SESSION,
                "context_id": null,
                "turn_id": null,
            },
            "args": args,
            "risk": risk,
            "confirm_required": false,
        }))
        .expect("call params")
    }

    /// A call for one of this host's read tools.
    fn call(name: &str, args: Value) -> ToolCallParams {
        let declared = tool(name).unwrap_or_else(|| panic!("{name} is declared"));
        call_with_risk(name, args, declared.risk)
    }

    /// A call whose declared risk is the frame's own business — for the tools
    /// that are not declared at all.
    fn bare_call(name: &str, args: Value) -> ToolCallParams {
        call_with_risk(name, args, "read")
    }

    /// A host connection over a pipe, plus the peer's end: what the host sends
    /// comes out of `kernel`.
    ///
    /// A multi-threaded runtime, deliberately: `Connection::new` spawns its
    /// writer onto the ambient runtime, and a `current_thread` runtime only
    /// advances those tasks inside `block_on` — a test that calls `notify` from
    /// synchronous code would then never see a byte leave. Two workers make the
    /// writer run for real, which is also how the app drives it.
    fn connection() -> (Connection, tokio::io::DuplexStream) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_stack_size(2 * 1024 * 1024)
            .build()
            .expect("test runtime");
        let (host, kernel) = tokio::io::duplex(64 * 1024);
        let (connection, incoming) = {
            let _entered = runtime.enter();
            Connection::new(host)
        };
        // Nothing initiates frames to the host in these tests, and the runtime
        // must outlive the connection's writer: it is leaked for the test's
        // lifetime (these tests are short and the process ends after them).
        std::mem::forget(incoming);
        std::mem::forget(runtime);
        (connection, kernel)
    }

    /// Read the frames the host wrote, until it stops writing for a moment.
    fn read_frames(kernel: tokio::io::DuplexStream) -> Vec<Value> {
        let mut kernel = kernel;
        read_frames_from(&mut kernel)
    }

    /// [`read_frames`], leaving the pipe open so a later step's frames can be
    /// read too (a call held for approval produces a frame only once the person
    /// decides).
    fn read_frames_from(kernel: &mut tokio::io::DuplexStream) -> Vec<Value> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("reader runtime");
        runtime.block_on(async move {
            let (reader, _writer) = tokio::io::split(kernel);
            let mut reader = tokio::io::BufReader::new(reader);
            let mut frames = Vec::new();
            let mut line = String::new();
            loop {
                line.clear();
                match tokio::time::timeout(
                    std::time::Duration::from_millis(200),
                    tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line),
                )
                .await
                {
                    Ok(Ok(0)) | Err(_) => break,
                    Ok(Ok(_)) => {
                        if let Ok(frame) = serde_json::from_str::<Value>(line.trim()) {
                            frames.push(frame);
                        }
                    }
                    Ok(Err(_)) => break,
                }
            }
            frames
        })
    }

    // -----------------------------------------------------------------------
    // Registration
    // -----------------------------------------------------------------------

    /// The declared set is what `peer/tools/register` expects: a session id, a
    /// tool array whose names are `<app>.<tool>`, and no `peer` (this is a
    /// session set, which is the shape a host that owns its pipe uses).
    #[test]
    fn registers_a_session_tool_set_with_declared_names() {
        let params = HostPeer::register_params(SESSION);

        assert_eq!(params["session_id"], json!(SESSION));
        assert!(
            params.get("peer").is_none(),
            "a session set names no peer: {params}"
        );
        assert!(
            params.get("host_token").is_none(),
            "the stdio connection is the host's own: it needs no app peer's token"
        );
        let tools = params["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), host_tools().len());
        for tool in tools {
            let name = tool["name"].as_str().expect("name");
            let declared = host_tools()
                .into_iter()
                .find(|host| host.name == name)
                .unwrap_or_else(|| panic!("{name} is a declared tool"));
            assert!(name.starts_with("zed."), "{name}");
            // `<app>.<tool>`: the kernel's `validate_app_tool_name` allows 2-4
            // segments of `[a-z][a-z0-9_]`, and the model-visible alias is this
            // same string with its dots replaced.
            assert!(
                (2..=4).contains(&name.split('.').count()),
                "{name}"
            );
            assert_eq!(declared.model_name, name.replace('.', "_"), "{name}");
            assert_eq!(tool["app"], json!(APP));
            assert_eq!(tool["input_schema"]["type"], json!("object"), "{name}");
            assert_eq!(tool["confirm"], json!("app"), "{name}");
            // The gate the host applies has to be visible in the declaration
            // it sends, or the kernel and the host disagree about the tool.
            assert_eq!(
                tool["risk"],
                json!(declared.risk),
                "{name} must declare the risk the host gates on"
            );
        }
        // The kernel's own defaults are restated so a change on either side is
        // visible here rather than a silent behaviour shift.
        assert_eq!(params["call_timeout_ms"], json!(CALL_TIMEOUT_MS));
        assert_eq!(params["max_result_bytes"], json!(MAX_RESULT_BYTES));
    }

    /// A registration the kernel did not accept leaves no route: the host then
    /// refuses every call rather than running one for a session it does not own.
    #[test]
    fn an_unregistered_host_owns_no_session() {
        let peer = HostPeer::new();
        peer.with_executor(Arc::new(FakeExecutor::default()));

        assert!(!peer.is_registered());
        assert_eq!(
            peer.plan_call(&call("zed.workspace.read", json!({"path": "hello.txt"}))),
            CallPlan::Refuse(Refusal::ForeignSession)
        );
        // A refused registration is recorded, not swallowed.
        let text = peer.registration_failed(&json!({
            "code": "peer_tools_invalid",
            "message": "tool 'zed.workspace.read' would be seen as the kernel tool 'x'",
        }));
        assert!(text.contains("peer_tools_invalid") || text.contains("would be seen"), "{text}");
        assert!(peer.activity().iter().any(|entry| entry.tool == "(registration)"));
    }

    /// A reply naming a different session is not this host's registration.
    #[test]
    fn a_reply_for_another_session_is_not_a_registration() {
        let peer = HostPeer::new();
        let error = peer
            .registered(
                SESSION,
                PathBuf::from("E:\\ws"),
                &json!({ "session_id": OTHER_SESSION, "version": 1 }),
            )
            .expect_err("a foreign session must not register");
        assert!(error.contains(OTHER_SESSION), "{error}");
        assert!(!peer.is_registered());

        let error = peer
            .registered(SESSION, PathBuf::from("E:\\ws"), &json!({ "ok": true }))
            .expect_err("a reply with no version is unusable");
        assert!(error.contains("version"), "{error}");
    }

    // -----------------------------------------------------------------------
    // Authorization: the positive path
    // -----------------------------------------------------------------------

    #[test]
    fn a_declared_read_tool_plans_and_runs_inside_the_workspace() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());

        let plan = peer.plan_call(&call("zed.workspace.read", json!({"path": "hello.txt"})));
        let CallPlan::Run(plan) = plan else {
            panic!("a read tool needs no approval: {plan:?}");
        };
        assert_eq!(plan.tool.name, "zed.workspace.read");
        assert_eq!(
            plan.args,
            Args::Read {
                path: "hello.txt".to_owned(),
                max_bytes: READ_MAX_BYTES
            }
        );

        // The executor really reads it: the confinement is not just a field.
        let outcome = executor.execute(&plan);
        assert!(outcome.ok, "{:?}", outcome.body);
        assert_eq!(outcome.body["text"], json!("one\ntwo\nthree\n"));
        assert_eq!(outcome.body["lines"], json!(3));
        assert_eq!(outcome.body["truncated"], json!(false));
    }

    #[test]
    fn a_read_is_truncated_at_the_requested_cap() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());

        let plan = match peer.plan_call(&call(
            "zed.workspace.read",
            json!({"path": "hello.txt", "max_bytes": 4}),
        )) {
            CallPlan::Run(plan) => plan,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            plan.args,
            Args::Read {
                path: "hello.txt".to_owned(),
                max_bytes: 4
            }
        );
        let outcome = executor.execute(&plan);
        assert_eq!(
            outcome.body,
            json!({
                "path": "hello.txt",
                "bytes": 14,
                "lines": 1,
                "truncated": true,
                "text": "one\n",
            })
        );
        assert_eq!(outcome.body["truncated"], json!(true));
        assert_eq!(outcome.body["bytes"], json!(14));
    }

    /// The seam's tool can reach a nested file, and a `max_bytes` request is
    /// clamped to the ceiling rather than trusted.
    #[test]
    fn a_read_is_clamped_and_can_reach_a_subdirectory() {
        let dir = workspace();
        let (peer, _executor) = host(dir.path());

        let plan = match peer.plan_call(&call(
            "zed.workspace.read",
            json!({"path": "nested/inner.txt", "max_bytes": 999_999_999u64}),
        )) {
            CallPlan::Run(plan) => plan,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            plan.args,
            Args::Read {
                path: "nested/inner.txt".to_owned(),
                max_bytes: READ_MAX_BYTES
            }
        );
    }

    // -----------------------------------------------------------------------
    // Authorization: the negative paths
    // -----------------------------------------------------------------------

    /// A call for a session this process did not open is refused, and nothing
    /// runs.
    #[test]
    fn a_call_for_another_session_is_refused() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());

        let mut hostile = call("zed.workspace.read", json!({"path": "hello.txt"}));
        hostile.session_id = OTHER_SESSION.to_owned();
        assert_eq!(
            peer.plan_call(&hostile),
            CallPlan::Refuse(Refusal::ForeignSession)
        );
        assert!(executor.ran().is_empty());
        // The refusal is a frame the kernel can act on, not a dropped call.
        let (connection, kernel) = connection();
        assert_eq!(
            peer.handle_call(&connection, &hostile),
            Outcome::Refused(Refusal::ForeignSession)
        );
        let frames = read_frames(kernel);
        assert_eq!(
            frames
                .iter()
                .filter(|frame| frame["method"] == json!(PEER_TOOL_RESULT))
                .count(),
            1,
            "{frames:?}"
        );
        assert!(executor.ran().is_empty());
    }

    /// The caller block is data, not proof: a frame that contradicts the
    /// session it arrived on is not this host's call.
    #[test]
    fn a_frame_whose_caller_contradicts_its_session_is_refused() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());

        let mut forged = call("zed.workspace.read", json!({"path": "hello.txt"}));
        forged.caller = Some(ToolCaller {
            kind: Some("system".to_owned()),
            peer: None,
            session_id: Some(OTHER_SESSION.to_owned()),
            context_id: None,
            turn_id: None,
        });
        assert_eq!(
            peer.plan_call(&forged),
            CallPlan::Refuse(Refusal::OriginMismatch)
        );

        // A call claiming a request context is not a session call either.
        let mut context = call("zed.workspace.read", json!({"path": "hello.txt"}));
        context.context_id = Some("mini-1".to_owned());
        assert_eq!(
            peer.plan_call(&context),
            CallPlan::Refuse(Refusal::OriginMismatch)
        );

        // A call naming a peer is a peer's call, not this host's session's.
        let mut peer_call = call("zed.workspace.read", json!({"path": "hello.txt"}));
        peer_call.peer = Some("news".to_owned());
        assert_eq!(
            peer.plan_call(&peer_call),
            CallPlan::Refuse(Refusal::ForeignSession)
        );
        assert!(executor.ran().is_empty());
    }

    /// A tool this host never declared is refused — the model asking for a
    /// name is not a declaration.
    #[test]
    fn an_undeclared_tool_is_refused() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());

        for name in [
            "zed.workspace.write",
            "shell",
            "zed_workspace_read",
            "",
        ] {
            assert_eq!(
                peer.plan_call(&bare_call(name, json!({"path": "hello.txt"}))),
                CallPlan::Refuse(Refusal::UnknownTool),
                "{name}"
            );
        }
        assert!(executor.ran().is_empty());
    }

    /// A frame whose declared risk disagrees with the host's declaration is a
    /// kernel/host disagreement, not a call to run.
    #[test]
    fn a_risk_that_disagrees_with_the_declaration_is_refused() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());

        let mut downgraded = call("zed.editor.open", json!({"path": "hello.txt"}));
        downgraded.risk = Some("read".to_owned());
        assert_eq!(
            peer.plan_call(&downgraded),
            CallPlan::Refuse(Refusal::DeclarationMismatch)
        );
        assert!(executor.ran().is_empty());
    }

    /// Confinement: absolute paths, `..`, and a symlink out of the workspace
    /// are all refused. The plan never sees an unconfined path.
    #[test]
    fn arguments_outside_the_workspace_are_refused() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());

        for bad in [
            json!({"path": "C:\\Windows\\System32\\drivers\\etc\\hosts"}),
            json!({"path": "/etc/passwd"}),
            json!({"path": "../../secret"}),
            json!({"path": "nested/../../secret"}),
            json!({"path": ""}),
            json!({"path": 42}),
            json!({}),
            json!("not an object"),
        ] {
            assert_eq!(
                peer.plan_call(&call("zed.workspace.read", bad.clone())),
                CallPlan::Refuse(Refusal::BadArguments),
                "{bad}"
            );
        }
        assert!(executor.ran().is_empty());
    }

    /// A well-shaped relative path can still leave the workspace through a
    /// symlink; the executor resolves before it reads and refuses.
    #[test]
    #[cfg(windows)]
    fn a_symlink_out_of_the_workspace_is_refused_at_read_time() {
        let outside = workspace();
        let dir = workspace();
        // A directory symlink needs no elevation on Windows; a file symlink
        // does, so link the directory and address the file through it.
        if std::os::windows::fs::symlink_dir(outside.path(), dir.path().join("escape")).is_err() {
            // No privilege to create one: the case is unreachable on this host.
            return;
        }
        std::fs::write(outside.path().join("secret.txt"), "secret").expect("secret");

        let (peer, executor) = host(dir.path());
        let plan = match peer.plan_call(&call(
            "zed.workspace.read",
            json!({"path": "escape/secret.txt"}),
        )) {
            CallPlan::Run(plan) => plan,
            other => panic!("the shape check cannot see the link: {other:?}"),
        };
        let outcome = executor.execute(&plan);
        assert!(!outcome.ok, "a symlink must not be a way out: {outcome:?}");
        assert!(
            outcome.body.to_string().contains("outside the workspace"),
            "{outcome:?}"
        );
    }

    /// A call id is handled once. Replaying a frame the host already saw is
    /// refused rather than dealt with twice — both while it is pending and
    /// after it was answered.
    #[test]
    fn a_duplicate_call_id_is_refused() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());
        let (connection, kernel) = connection();

        // Held for the person: the same frame again is a duplicate.
        let held = call("zed.editor.open", json!({"path": "hello.txt"}));
        assert!(matches!(
            peer.handle_call(&connection, &held),
            Outcome::AwaitingApproval(_)
        ));
        assert_eq!(
            peer.plan_call(&held),
            CallPlan::Refuse(Refusal::DuplicateCall)
        );
        assert!(executor.opens().is_empty(), "nothing runs before the person decides");
        assert_eq!(
            peer.handle_call(&connection, &held),
            Outcome::Refused(Refusal::DuplicateCall)
        );

        // Already answered: the frame is still a duplicate.
        let read = call("zed.workspace.read", json!({"path": "hello.txt"}));
        assert_eq!(peer.handle_call(&connection, &read), Outcome::Ran);
        assert_eq!(
            peer.plan_call(&read),
            CallPlan::Refuse(Refusal::DuplicateCall)
        );
        assert_eq!(executor.ran().len(), 1, "the read ran exactly once");
        drop(kernel);
    }

    /// The kernel's own cap on in-flight calls is mirrored, and a call past it
    /// is refused instead of queued.
    #[test]
    fn calls_past_the_in_flight_cap_are_refused() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());
        let (connection, kernel) = connection();

        for _ in 0..MAX_PENDING_CALLS {
            let held = call("zed.editor.open", json!({"path": "hello.txt"}));
            assert!(matches!(
                peer.handle_call(&connection, &held),
                Outcome::AwaitingApproval(_)
            ));
        }
        let extra = call("zed.workspace.read", json!({"path": "hello.txt"}));
        assert_eq!(peer.plan_call(&extra), CallPlan::Refuse(Refusal::Busy));
        drop(kernel);
        assert!(executor.ran().is_empty());
    }

    // -----------------------------------------------------------------------
    // Approval: the default safe posture
    // -----------------------------------------------------------------------

    /// A tool the host gated runs under NO circumstances before the person
    /// approves; the kernel is told the call is held, and the executor is not
    /// invoked.
    #[test]
    fn a_gated_tool_does_not_run_before_the_person_approves() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());
        let (connection, kernel) = connection();

        let open = call("zed.editor.open", json!({"path": "hello.txt", "line": 2}));
        let outcome = peer.handle_call(&connection, &open);
        let Outcome::AwaitingApproval(approval_id) = outcome else {
            panic!("an open must be held: {outcome:?}");
        };

        assert!(executor.opens().is_empty(), "no effect before the person decides");
        assert!(executor.ran().is_empty());
        let approvals = peer.approvals();
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0].approval_id, approval_id);
        assert!(matches!(approvals[0].source, ApprovalSource::HostTool));
        assert_eq!(approvals[0].tool, "zed.editor.open");

        // The kernel was told the call is held, not answered.
        let frames = read_frames(kernel);
        let held = frames
            .iter()
            .find(|frame| frame["method"] == json!(PEER_TOOL_RESULT))
            .expect("a holding frame");
        assert_eq!(held["params"]["status"], json!("awaiting_confirmation"));
        assert_eq!(held["params"]["call_id"], json!(open.call_id));
        assert!(
            held["params"].get("ok").is_none(),
            "a held call is not a result: {held}"
        );
    }

    /// Denying is a refusal with no side effect, and the kernel still gets an
    /// answer so its turn can move on.
    #[test]
    fn denying_an_approved_tool_runs_nothing() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());
        let (connection, kernel) = connection();

        let open = call("zed.editor.open", json!({"path": "hello.txt"}));
        assert!(matches!(
            peer.handle_call(&connection, &open),
            Outcome::AwaitingApproval(_)
        ));
        assert_eq!(peer.decide(&open.call_id, false, &connection), Decision::Denied);

        assert!(executor.opens().is_empty(), "a denial must not open anything");
        assert!(peer.approvals().is_empty(), "the card is gone");
        assert!(peer.pending_calls().is_empty());

        let frames = read_frames(kernel);
        let result = frames
            .iter()
            .filter(|frame| frame["method"] == json!(PEER_TOOL_RESULT))
            .next_back()
            .expect("a final frame");
        assert_eq!(result["params"]["ok"], json!(false));
        assert!(
            result["params"]["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("denied")),
            "{result}"
        );
    }

    /// Approving runs it exactly once, tells the kernel `ok`, and clears the
    /// card.
    #[test]
    fn approving_runs_the_tool_once_and_answers_the_kernel() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());
        let (connection, kernel) = connection();

        let open = call("zed.editor.open", json!({"path": "hello.txt", "line": 3}));
        assert!(matches!(
            peer.handle_call(&connection, &open),
            Outcome::AwaitingApproval(_)
        ));
        assert_eq!(peer.decide(&open.call_id, true, &connection), Decision::Ran);

        let opens = executor.opens();
        assert_eq!(opens.len(), 1);
        assert!(opens[0].path.ends_with("hello.txt"), "{:?}", opens[0]);
        assert_eq!(opens[0].line, Some(3));
        assert_eq!(executor.ran(), vec!["zed.editor.open".to_owned()]);
        assert!(peer.approvals().is_empty());

        // A second decision for the same call is a no-op, not a second open.
        assert_eq!(
            peer.decide(&open.call_id, true, &connection),
            Decision::NoSuchCall
        );
        assert_eq!(executor.opens().len(), 1);

        let frames = read_frames(kernel);
        let result = frames
            .iter()
            .filter(|frame| frame["method"] == json!(PEER_TOOL_RESULT))
            .next_back()
            .expect("a final frame");
        assert_eq!(result["params"]["ok"], json!(true));
        assert_eq!(result["params"]["call_id"], json!(open.call_id));
    }

    /// Deciding a call that is not pending does nothing at all — the path a
    /// stale UI click takes.
    #[test]
    fn deciding_an_unknown_call_does_nothing() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());
        let (connection, kernel) = connection();

        assert_eq!(
            peer.decide("ptc-never-happened", true, &connection),
            Decision::NoSuchCall
        );
        drop(kernel);
        assert!(executor.opens().is_empty());
        assert!(executor.ran().is_empty());
    }

    // -----------------------------------------------------------------------
    // Cancellation
    // -----------------------------------------------------------------------

    /// The regression this pins: a call the kernel stopped waiting for must not
    /// take effect when the person approves it afterwards. The kernel cancels
    /// on a timeout or a turn interrupt, so a late approval is the ordinary
    /// race, not a hypothetical one.
    #[test]
    fn approving_a_cancelled_call_produces_no_effect() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());
        let (connection, kernel) = connection();

        let open = call("zed.editor.open", json!({"path": "hello.txt"}));
        assert!(matches!(
            peer.handle_call(&connection, &open),
            Outcome::AwaitingApproval(_)
        ));
        let cancel = ToolCancelParams {
            peer: None,
            session_id: Some(SESSION.to_owned()),
            call_id: open.call_id.clone(),
            reason: Some("timeout".to_owned()),
        };
        peer.handle_cancel(&cancel);

        // The kernel's cancel is recorded, the card is gone, nothing is left.
        assert!(peer.cancelled(&open.call_id).is_some());
        assert!(peer.approvals().is_empty());
        assert!(peer.pending_calls().is_empty());

        // The person's click lands after the cancel: no open, and no frame.
        assert_eq!(
            peer.decide(&open.call_id, true, &connection),
            Decision::Cancelled
        );
        assert!(executor.opens().is_empty(), "a cancelled call cannot open a file");
        assert!(executor.ran().is_empty());

        // The kernel hears nothing more: it stopped waiting.
        let frames = read_frames(kernel);
        assert!(
            frames
                .iter()
                .filter(|frame| frame["method"] == json!(PEER_TOOL_RESULT))
                .count()
                == 1,
            "only the holding frame was sent: {frames:?}"
        );
    }

    /// A cancel for a call that is not pending is recorded anyway, so a result
    /// arriving afterwards is dropped rather than sent for a dead call.
    #[test]
    fn cancelling_an_unknown_call_still_blocks_a_late_answer() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());
        let (_connection, kernel) = connection();

        let mut read = call("zed.workspace.read", json!({"path": "hello.txt"}));
        peer.handle_cancel(&ToolCancelParams {
            peer: None,
            session_id: None,
            call_id: read.call_id.clone(),
            reason: None,
        });
        assert_eq!(
            peer.cancelled(&read.call_id).map(|call| call.reason),
            Some("cancelled".to_owned())
        );

        // The same call id can never run, whatever the frame now says: it is
        // refused as a duplicate of a call the kernel already gave up on.
        assert_eq!(
            peer.plan_call(&read),
            CallPlan::Refuse(Refusal::DuplicateCall)
        );
        read.session_id = OTHER_SESSION.to_owned();
        assert_eq!(
            peer.plan_call(&read),
            CallPlan::Refuse(Refusal::ForeignSession),
            "the session check comes first: a foreign frame is refused on its own merits"
        );
        drop(kernel);
        assert!(executor.ran().is_empty());
    }

    /// A finished call's result is dropped when the kernel had already stopped
    /// waiting: the pending entry is gone, so nothing is sent.
    #[test]
    fn a_finished_call_after_a_cancel_is_not_answered() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());
        let (connection, kernel) = connection();

        let read = call("zed.workspace.read", json!({"path": "hello.txt"}));
        assert_eq!(
            peer.handle_call(&connection, &read),
            Outcome::Ran
        );
        // Ran means it answered once; a cancel afterwards cannot retract it,
        // but it must not produce a second frame either.
        let frames = read_frames(kernel);
        assert_eq!(
            frames
                .iter()
                .filter(|frame| frame["method"] == json!(PEER_TOOL_RESULT))
                .count(),
            1
        );
        assert!(executor.ran().len() == 1);
    }

    // -----------------------------------------------------------------------
    // Reply correlation and the wire shape
    // -----------------------------------------------------------------------

    /// The reply is correlated by `call_id` and carries the session the call
    /// arrived on, decoded back through the kernel's param shape.
    #[test]
    fn the_result_frame_correlates_by_call_id() {
        let dir = workspace();
        let (peer, _executor) = host(dir.path());
        let (connection, kernel) = connection();

        let read = call("zed.workspace.read", json!({"path": "hello.txt"}));
        assert_eq!(peer.handle_call(&connection, &read), Outcome::Ran);

        let frames = read_frames(kernel);
        let result = frames
            .iter()
            .find(|frame| frame["method"] == json!(PEER_TOOL_RESULT))
            .expect("a result frame");
        assert!(result.get("id").is_none(), "a result is a notification: {result}");
        assert_eq!(result["params"]["call_id"], json!(read.call_id));
        assert_eq!(result["params"]["session_id"], json!(SESSION));
        assert_eq!(result["params"]["ok"], json!(true));
        assert_eq!(result["params"]["data"]["text"], json!("one\ntwo\nthree\n"));
    }

    /// An executor that fails is reported as a failure, never as an empty
    /// success.
    #[test]
    fn a_failing_tool_is_reported_as_failed() {
        let dir = workspace();
        let peer = HostPeer::new();
        let executor = Arc::new(FakeExecutor {
            fail: true,
            ..Default::default()
        });
        peer.with_executor(executor.clone());
        peer.registered(SESSION, dir.path().to_path_buf(), &json!({
            "session_id": SESSION,
            "version": 1,
        }))
        .expect("registration");
        let (connection, kernel) = connection();

        let read = call("zed.workspace.read", json!({"path": "hello.txt"}));
        assert_eq!(peer.handle_call(&connection, &read), Outcome::Ran);

        let frames = read_frames(kernel);
        let result = frames
            .iter()
            .find(|frame| frame["method"] == json!(PEER_TOOL_RESULT))
            .expect("a result frame");
        assert_eq!(result["params"]["ok"], json!(false));
        assert_eq!(result["params"]["error"]["kind"], json!("tool_failed"));
        // The activity list agrees with what was sent.
        assert!(peer
            .activity()
            .iter()
            .any(|entry| entry.state == ActivityState::Failed
                && entry.detail.contains("refused to run")));
    }

    /// A host whose executor cannot run the call reports that, rather than
    /// answering `ok` with a lie. The case is reachable in production too: the
    /// app installs its executor at startup, and a call that outran a failed
    /// install must not look like success.
    #[test]
    fn a_call_whose_executor_cannot_run_it_is_an_error_not_a_success() {
        let dir = workspace();
        let peer = HostPeer::new();
        peer.with_executor(Arc::new(FakeExecutor {
            fail: true,
            ..Default::default()
        }));
        peer.registered(SESSION, dir.path().to_path_buf(), &json!({
            "session_id": SESSION,
            "version": 1,
        }))
        .expect("registration");
        let (connection, kernel) = connection();

        let read = call("zed.workspace.read", json!({"path": "hello.txt"}));
        assert_eq!(peer.handle_call(&connection, &read), Outcome::Ran);
        let frames = read_frames(kernel);
        let result = frames
            .iter()
            .find(|frame| frame["method"] == json!(PEER_TOOL_RESULT))
            .expect("a result frame");
        assert_eq!(result["params"]["ok"], json!(false));
        assert!(
            result["params"]["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("refused to run")),
            "{result}"
        );
    }

    /// The default peer serves a read with no executor installed by the app:
    /// the shipped [`BuiltinExecutor`] is the floor, not a stub.
    #[test]
    fn the_default_executor_serves_a_read() {
        let dir = workspace();
        let peer = HostPeer::default();
        peer.registered(SESSION, dir.path().to_path_buf(), &json!({
            "session_id": SESSION,
            "version": 1,
        }))
        .expect("registration");
        let (connection, mut kernel) = connection();

        let read = call("zed.workspace.read", json!({"path": "hello.txt"}));
        assert_eq!(peer.handle_call(&connection, &read), Outcome::Ran);
        let frames = read_frames_from(&mut kernel);
        let result = frames
            .iter()
            .find(|frame| frame["method"] == json!(PEER_TOOL_RESULT))
            .expect("a result frame");
        assert_eq!(result["params"]["ok"], json!(true), "{result}");
        assert_eq!(result["params"]["data"]["text"], json!("one\ntwo\nthree\n"));

        // An *open* is a different matter: the default executor has no UI to
        // deliver it to, and says so instead of claiming a tab appeared.
        let open = call("zed.editor.open", json!({"path": "hello.txt"}));
        assert!(matches!(
            peer.handle_call(&connection, &open),
            Outcome::AwaitingApproval(_)
        ));
        assert_eq!(peer.decide(&open.call_id, true, &connection), Decision::Ran);
        let frames = read_frames_from(&mut kernel);
        let result = frames
            .iter()
            .filter(|frame| frame["method"] == json!(PEER_TOOL_RESULT))
            .next_back()
            .expect("a final frame");
        assert_eq!(result["params"]["ok"], json!(false), "{result}");
        assert!(
            result["params"]["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("no editor wired")),
            "{result}"
        );
    }

    /// A refused call is answered with a stable `host:<kind>` and the refusal
    /// reaches the executor's audit hook.
    #[test]
    fn a_refusal_is_answered_and_recorded() {
        let dir = workspace();
        let (peer, executor) = host(dir.path());
        let (connection, kernel) = connection();

        let mut hostile = call("zed.workspace.read", json!({"path": "hello.txt"}));
        hostile.session_id = OTHER_SESSION.to_owned();
        assert_eq!(
            peer.handle_call(&connection, &hostile),
            Outcome::Refused(Refusal::ForeignSession)
        );

        assert_eq!(
            executor.refusals.lock().expect("refusals").as_slice(),
            &[("zed.workspace.read".to_owned(), "host:foreign_session")]
        );
        let frames = read_frames(kernel);
        let result = frames
            .iter()
            .find(|frame| frame["method"] == json!(PEER_TOOL_RESULT))
            .expect("a refusal frame");
        assert_eq!(result["params"]["ok"], json!(false));
        assert_eq!(result["params"]["error"]["kind"], json!("host:foreign_session"));
        assert_eq!(result["params"]["call_id"], json!(hostile.call_id));
    }

    // -----------------------------------------------------------------------
    // Decoding
    // -----------------------------------------------------------------------

    /// A call frame is decoded with typed params; a frame missing the fields a
    /// host needs is an error, never a call guessed at.
    #[test]
    fn call_frames_decode_or_error() {
        let params = serde_json::to_value(call("zed.workspace.read", json!({"path": "a"})))
            .expect("wire");
        let decoded = HostPeer::decode_call(&params).expect("decode");
        assert_eq!(decoded.name, "zed.workspace.read");
        assert_eq!(decoded.session_id, SESSION);

        let broken = json!({ "name": "zed.workspace.read" });
        let error = HostPeer::decode_call(&broken).expect_err("missing fields");
        assert_eq!(
            error.code,
            octos_core::ui_protocol::rpc_error_codes::INVALID_PARAMS
        );
        assert!(error.message.contains("peer/tool/call"), "{error:?}");

        let cancel = HostPeer::decode_cancel(&json!({
            "call_id": "ptc-1",
            "reason": "timeout"
        }))
        .expect("cancel decodes");
        assert_eq!(cancel.call_id, "ptc-1");
        assert!(HostPeer::decode_cancel(&json!({})).is_err());
    }

    // -----------------------------------------------------------------------
    // The kernel's own tools and approvals
    // -----------------------------------------------------------------------

    /// The kernel's tools and this host's share one activity list, keyed by the
    /// kernel's call id, so progress updates land on the entry start opened.
    #[test]
    fn kernel_tool_lifecycles_land_on_one_activity_list() {
        let dir = workspace();
        let (peer, _executor) = host(dir.path());

        peer.note_kernel_tool("tc-9", "shell", ActivityState::Running, String::new());
        peer.note_kernel_tool("tc-9", "", ActivityState::Running, "halfway".to_owned());
        peer.note_kernel_tool("tc-9", "shell", ActivityState::Done, "exit 0".to_owned());

        let activity = peer.activity();
        let entry = activity
            .iter()
            .find(|entry| entry.call_id == "tc-9")
            .expect("the kernel's tool is on the list");
        assert_eq!(entry.tool, "shell");
        assert_eq!(entry.state, ActivityState::Done);
        assert_eq!(entry.detail, "exit 0");
    }

    /// A kernel approval is rendered like a host one and answered with the
    /// kernel's own params — built from the pending entry, so a caller cannot
    /// name a session this host never saw.
    #[test]
    fn a_kernel_approval_answers_on_the_kernels_session() {
        let dir = workspace();
        let (peer, _executor) = host(dir.path());
        let approval_id = ApprovalId::new();
        peer.note_kernel_approval(&KernelApproval {
            session_id: SESSION.to_owned(),
            approval_id: approval_id.clone(),
            turn_id: TurnId::new(),
            tool_name: "shell".to_owned(),
            title: "Run a command".to_owned(),
            body: "rm -rf build".to_owned(),
            risk: Some("destructive".to_owned()),
            tool_call_id: Some("tc-7".to_owned()),
        });

        let request = peer.approval(&approval_id).expect("pending");
        assert!(matches!(request.source, ApprovalSource::Kernel { .. }));
        assert_eq!(request.summary, "Run a command");

        let params = peer
            .approval_respond_params(&approval_id, false)
            .expect("respond params");
        let params: octos_core::ui_protocol::ApprovalRespondParams =
            serde_json::from_value(params).expect("the kernel's own params type");
        assert_eq!(params.session_id.0, SESSION);
        assert_eq!(params.approval_id, approval_id);
        assert_eq!(
            params.decision,
            octos_core::ui_protocol::ApprovalDecision::Deny
        );

        // Answering once takes it off the list; a second answer has nothing to
        // send.
        assert!(peer.take_kernel_approval(&approval_id).is_some());
        assert!(peer.approval_respond_params(&approval_id, true).is_none());
    }

    /// A host tool's approval has no `approval/respond` params: the decision is
    /// local, and pretending otherwise would send the kernel an answer for an
    /// approval the kernel never raised.
    #[test]
    fn a_host_tool_approval_has_no_kernel_respond_params() {
        let dir = workspace();
        let (peer, _executor) = host(dir.path());
        let (connection, kernel) = connection();

        let open = call("zed.editor.open", json!({"path": "hello.txt"}));
        let Outcome::AwaitingApproval(approval_id) = peer.handle_call(&connection, &open) else {
            panic!("held");
        };
        assert!(peer.approval_respond_params(&approval_id, true).is_none());
        drop(kernel);
    }

    /// `reset` is what a closed connection does: no registration, no pending
    /// call and no stale approval may survive it.
    #[test]
    fn reset_drops_everything_the_connection_owned() {
        let dir = workspace();
        let (peer, _executor) = host(dir.path());
        let (connection, kernel) = connection();

        let open = call("zed.editor.open", json!({"path": "hello.txt"}));
        assert!(matches!(
            peer.handle_call(&connection, &open),
            Outcome::AwaitingApproval(_)
        ));
        assert!(peer.is_registered());
        assert_eq!(peer.registered_session().as_deref(), Some(SESSION));
        assert_eq!(peer.workspace().as_deref(), Some(dir.path()));
        assert!(!peer.pending_calls().is_empty());

        peer.reset();
        assert!(!peer.is_registered());
        assert!(peer.registered_session().is_none());
        assert!(peer.workspace().is_none());
        assert!(peer.pending_calls().is_empty());
        assert!(peer.approvals().is_empty());
        assert!(peer.activity().is_empty());
        assert_eq!(
            peer.plan_call(&call("zed.workspace.read", json!({"path": "hello.txt"}))),
            CallPlan::Refuse(Refusal::ForeignSession)
        );
        drop(kernel);
    }

    // -----------------------------------------------------------------------
    // The route key, against the kernel's own rule
    // -----------------------------------------------------------------------

    /// The route key mirrors `session_route_key`
    /// (`crates/octos/octos-cli/src/peers/host_tools.rs:1230`). It is a diagnostic
    /// and a test oracle: a divergence from the kernel's rule would show up as
    /// a failing assertion here rather than as registrations landing under
    /// different filenames.
    ///
    /// The rule it mirrors: the route is the session's **profile id** when the
    /// key parses as `{profile}:{channel}:{chat}` (`SessionKey::profile_id`,
    /// `crates/octos/octos-core/src/types.rs:554`), and a hash of the whole key
    /// otherwise. A profile segment that *is* a reserved channel name makes the
    /// key parse the other way, so such a session gets its own hashed route —
    /// exactly what the kernel computes for it.
    #[test]
    fn the_session_route_key_follows_the_kernels_rule() {
        // `dev` is not a reserved channel name and `api` is, so `dev` is the
        // profile: the route is the profile, verbatim.
        assert_eq!(session_route_key("dev:api:chat-1"), "dev");
        assert_eq!(
            session_route_key(&format!("{}#peer-news", "dev:api:chat-1")),
            "dev",
            "a topic suffix is not part of the profile segment"
        );
        // `main` is not a reserved channel name either.
        assert_eq!(session_route_key("main:cli:ops"), "main");
        // `octos` is *not* a profile here (`main` is not a reserved channel
        // name), so the key has no profile segment and gets its own route.
        let odd = session_route_key("octos:main:zed#embedded");
        assert!(odd.starts_with("session-"), "{odd}");
        assert_eq!(odd, session_route_key("octos:main:zed#embedded"));
        assert_ne!(odd, session_route_key("octos:main:zed#other"));
        assert!(route_key_is_safe(&odd), "{odd}");
        // No `SessionKey` at all is still a safe route name, never a path.
        let empty = session_route_key("");
        assert!(route_key_is_safe(&empty), "{empty}");
        for unsafe_name in ["", ".", "..", "a/b", "a\\b", "A", "a b", &"x".repeat(65)] {
            assert!(!route_key_is_safe(unsafe_name), "{unsafe_name:?}");
        }
    }

    /// The executor seam counts what it was asked to run, so the "nothing ran"
    /// assertions above are not vacuous.
    #[test]
    fn the_fake_executor_is_not_silently_inert() {
        let counter = Arc::new(AtomicUsize::new(0));
        struct Counting(Arc<AtomicUsize>);
        impl HostToolExecutor for Counting {
            fn execute(&self, _plan: &RunPlan) -> ToolOutcome {
                self.0.fetch_add(1, Ordering::SeqCst);
                ToolOutcome::ok(json!({}))
            }
            fn publish_editor_open(&self, _open: EditorOpen) {}
        }

        let dir = workspace();
        let peer = HostPeer::new();
        peer.with_executor(Arc::new(Counting(counter.clone())));
        peer.registered(SESSION, dir.path().to_path_buf(), &json!({
            "session_id": SESSION,
            "version": 1,
        }))
        .expect("registration");
        let (connection, kernel) = connection();
        assert_eq!(
            peer.handle_call(&connection, &call("zed.workspace.read", json!({"path": "a"}))),
            Outcome::Ran
        );
        drop(kernel);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
