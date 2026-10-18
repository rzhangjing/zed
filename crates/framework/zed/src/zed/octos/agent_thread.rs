//! Octos as an agent-thread backend — **in process, with no child process**.
//!
//! This is the P2 seam: the Zed agent panel can open an agent thread whose
//! `AgentConnection` is served by the Octos kernel that already runs inside this
//! process ([`super`]). Nothing here spawns `octos acp`, nothing here reads
//! `agent_servers` settings, and nothing here mirrors Zed's model registry.
//!
//! Three pieces, deliberately separable:
//!
//! 1. **The translation** ([`thread_effects`], [`FrameEffects`]) — a *pure*
//!    function from one kernel frame to the ACP `SessionUpdate`s a thread's
//!    transcript receives, the turn it ends, the approval it raises, and any
//!    line the Octos console should show. Pure because it is the part where a
//!    wrong guess is silent: a duplicated chunk, a tool entry that never
//!    completes, a title that never changes. It decodes with `octos-core`'s own
//!    types (`UiNotification::from_method_and_params`,
//!    `crates/octos/octos-core/src/ui_protocol.rs:7301`), never by reading JSON
//!    keys this host hopes for.
//! 2. **The per-thread session state machine** ([`ThreadSession`]) — which turn
//!    is in flight, where its answer goes, whether a cancel is already in
//!    flight, and which approvals the kernel is still waiting on. Also pure and
//!    tested: `cancel` must not claim things the kernel did not say.
//! 3. **The plumbing** ([`OctosConnection`]) — the [`AgentConnection`] itself:
//!    one OUP session per thread, one fan-out subscription per thread, and
//!    `AcpThread::handle_session_update` on the gpui thread.
//!
//! 4. **Hydration** ([`hydrate_updates`], [`converge`]) — the pure reading of a
//!    `session/hydrate` result (UPCR-2026-009) into ACP updates, and the pure
//!    reading of what a hydrate says a thread that lost frames must converge to.
//!    A loaded session's transcript is rebuilt from it; a subscriber that fell
//!    behind re-reads the session's *state* through it.
//!
//! All four decode with `octos-core`'s own types, never by reading JSON keys
//! this host hopes for.
//!
//! What is deliberately *not* here:
//!
//! * **No mirror of Zed's models.** `model_selector()` (P4) is the *kernel's*
//!   list — the models configured on the Octos profile, one row per
//!   (family, model, route) — and a switch is reported with the kernel's own
//!   "when does this take effect" stamp instead of a claim of immediacy
//!   ([`super::model_selector`]). Zed's registry is not copied into it, and Zed's
//!   models are never written into Octos as fallbacks.
//! * **A credential is bridged, not mirrored.** [`super::key_bridge`] copies a key
//!   Zed holds into the *Octos profile's* own slot — Octos's secret store where
//!   the platform has one, otherwise the profile's `env_vars` in plaintext — which
//!   is the only thing that can work, because the kernel cannot read Zed's keyring.
//! * **No `truncate` / `retry` / `session_modes` / `session_config_options`** —
//!   the trait defaults are used, so the panel shows none of them.
//! * **No media.** `TurnStartParams::media` stays empty. `@file` mentions arrive
//!   as resource blocks and are degraded to text ([`prompt_text`]).
//!
//! The kernel's own frames are the only source of truth for turn state, and the
//! host never invents one: a `turn/interrupt` that is declined, times out or
//! comes back unreadable leaves the turn in flight, exactly as the console does
//! (see [`super::interrupt_turn`]). The same rule governs approvals: a click on
//! a card is *intent* and is answered with `approval/respond`; only the kernel's
//! own `approval/decided` / `approval/auto_resolved` / `approval/cancelled`
//! settles it ([`ApprovalLedger`]).

use std::{
    any::Any,
    collections::HashMap,
    rc::Rc,
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};

use acp_thread::{
    AcpThread, AgentConnection, AgentModelSelector, AgentSessionInfo, AgentSessionList,
    AgentSessionListRequest, AgentSessionListResponse, AgentSessionSetTitle, AuthorizationKind,
    PermissionOptions, RequestPermissionOutcome, SelectedPermissionOutcome,
};
use action_log::ActionLog;
use agent_client_protocol::schema::v1 as acp;
use anyhow::{Context as _, Result, anyhow};
use futures::channel::oneshot;
use gpui::{App, AppContext as _, AsyncApp, Entity, SharedString, Task, WeakEntity};
use octos_core::SessionKey;
use octos_core::ui_protocol::{
    self as oup, ApprovalDecision, ApprovalId, ApprovalRequestedEvent, EnvelopeToolEndStatus,
    HydratedMessage, InputItem, PayloadV2, PlanItemStatus, SessionHydrateParams,
    SessionHydrateResult, TurnId, TurnInterruptParams, TurnLifecycleState, TurnTerminalOutcome,
    UiCursor, UiNotification, UiPlanRecord, hydrate_sections,
};
use project::Project;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use util::path_list::PathList;

use super::connection::Connection;
use super::{Octos, OctosFrame};

/// How long a thread waits for the kernel to accept a rename.
///
/// The kernel's own cap on the title ([`oup::SESSION_TITLE_SET_MAX_CHARS`]) is
/// applied before sending, so this is only patience, never a policy.
const TITLE_SET_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a thread waits for the kernel to answer `session/hydrate`.
///
/// A hydrate reads the session's durable transcript and its live ledger, so it
/// is a real read; the bound is the host's patience, not the kernel's deadline.
const HYDRATE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a session listing waits for the kernel.
const SESSION_LIST_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a thread waits for the kernel to accept an approval decision.
const APPROVAL_RESPOND_TIMEOUT: Duration = Duration::from_secs(30);

/// The option ids a thread's approval card offers.
///
/// **Verbatim** from the ACP backend's own card (`oup.rs:303-310`, the same two
/// the console's approval cards use): two clients that name one decision
/// differently would answer the same kernel approval two different ways.
const APPROVAL_ALLOW_ONCE: &str = "allow-once";
const APPROVAL_DENY: &str = "deny";

// ---------------------------------------------------------------------------
// The backend seam
// ---------------------------------------------------------------------------

/// The running kernel, as something `Agent::Octos` can build a connection from.
///
/// Holds only the two things that make a thread possible — the OUP connection
/// and the fan-out — so a connection handed out here is a view of the kernel
/// that already exists rather than a promise to start one.
pub struct OctosKernel {
    connection: Connection,
    fanout: broadcast::Sender<OctosFrame>,
}

impl agent_ui::octos::OctosBackend for OctosKernel {
    fn connect(
        &self,
        _project: Entity<Project>,
        _cx: &mut App,
    ) -> Result<Rc<dyn AgentConnection>> {
        Ok(Rc::new(OctosConnection {
            connection: self.connection.clone(),
            fanout: self.fanout.clone(),
            sessions: Arc::default(),
        }))
    }
}

/// Publishes this process's kernel as the backend behind `Agent::Octos`.
///
/// Called from [`super::init`] after the kernel has been started: registering
/// before that would advertise a backend whose every session can only fail.
/// With no kernel (autostart off, or a runtime that failed to build) the backend
/// is withdrawn, so the agent picker offers no Octos entry instead of an entry
/// that always errors.
pub fn register_backend(cx: &mut App) {
    let kernel = cx.try_global::<Octos>().and_then(|octos| {
        Some(OctosKernel {
            connection: octos.connection.clone()?,
            fanout: octos.fanout.clone()?,
        })
    });

    match kernel {
        Some(kernel) => {
            log::info!("[octos] the in-process kernel is offered to agent threads");
            agent_ui::octos::register(cx, Rc::new(kernel));
        }
        None => {
            log::info!(
                "[octos] no in-process kernel is running, so agent threads cannot select Octos"
            );
            agent_ui::octos::unregister(cx);
        }
    }
}

// ---------------------------------------------------------------------------
// The connection
// ---------------------------------------------------------------------------

/// One `AgentConnection` over the embedded kernel.
///
/// The panel keys connections by agent, so this is created **once** per project
/// and shared by every Octos thread; the per-thread state lives in
/// [`OctosConnection::sessions`], one entry per `new_session`.
struct OctosConnection {
    connection: Connection,
    fanout: broadcast::Sender<OctosFrame>,
    sessions: Arc<Mutex<HashMap<acp::SessionId, Arc<ThreadSession>>>>,
}

impl OctosConnection {
    /// The thread session `prompt`, `cancel` and `set_title` act on.
    ///
    /// One lookup, so every entry point reports the same thing for an id this
    /// connection never opened: a session that has nowhere to go is an error the
    /// caller sees, never a frame sent at a guess.
    fn session_for(&self, id: &acp::SessionId) -> Result<Arc<ThreadSession>> {
        self.sessions
            .lock()
            .expect("octos sessions")
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("no Octos session is open for {}", id.0))
    }

    /// The per-thread state for `id`, reusing the live one when this connection
    /// already has that kernel session open.
    ///
    /// A load must not leave two competing turn states for one kernel session:
    /// the kernel runs *one* turn per session, so a second `ThreadSession` would
    /// refuse a `turn/start` against a turn only the first one knows about.
    fn session_state(&self, id: &acp::SessionId, session: SessionKey) -> Arc<ThreadSession> {
        self.sessions
            .lock()
            .expect("octos sessions")
            .entry(id.clone())
            .or_insert_with(|| Arc::new(ThreadSession::new(session)))
            .clone()
    }
}

/// One agent thread's session on the kernel.
///
/// The state machine here is the whole of what this host knows about a turn. It
/// is a plain struct with a mutex rather than something the thread owns, because
/// the terminal frame that ends a turn arrives on the fan-out task while the
/// `prompt` future is parked on the gpui thread.
pub struct ThreadSession {
    /// The kernel's key for this session: what every `turn/*` frame names, and
    /// what the process-wide fan-out is filtered by.
    session: SessionKey,
    turn: Mutex<TurnState>,
    /// The ledger checkpoint of the last hydrate this thread ran.
    ///
    /// Live frames that carry a cursor at or below it were already covered by
    /// that hydrate's replay, so applying them again would double an update. A
    /// frame that carries no cursor cannot be judged and is applied — deltas are
    /// live by construction.
    cursor: Mutex<Option<UiCursor>>,
    /// The approvals the kernel is waiting on for this session.
    approvals: Mutex<ApprovalLedger>,
}

impl ThreadSession {
    fn new(session: SessionKey) -> Self {
        Self {
            session,
            turn: Mutex::new(TurnState::default()),
            cursor: Mutex::new(None),
            approvals: Mutex::new(ApprovalLedger::default()),
        }
    }

    /// Remember the checkpoint a hydrate read through.
    fn set_cursor(&self, cursor: UiCursor) {
        *self.cursor.lock().expect("octos hydrate cursor") = Some(cursor);
    }

    /// The checkpoint this thread last hydrated through, if it ever has.
    fn cursor(&self) -> Option<UiCursor> {
        self.cursor.lock().expect("octos hydrate cursor").clone()
    }
}

#[derive(Default)]
struct TurnState {
    running: Option<RunningTurn>,
    /// A `turn/interrupt` was acknowledged with `interrupted: true` and no
    /// terminal frame has settled the turn yet.
    ///
    /// Purely a host-side dedupe: it stops a second interrupt from being sent
    /// for a turn that is already being interrupted, and it is the only thing
    /// that makes the console note differ ("cancelling" vs "cancelled"). It
    /// never asserts that the kernel stopped.
    cancelling: bool,
}

struct RunningTurn {
    id: TurnId,
    /// Where the `prompt` future is waiting. `None` once the prompt has been
    /// answered or abandoned, which is what makes settling idempotent.
    end: Option<oneshot::Sender<TurnEnd>>,
}

impl ThreadSession {
    /// Starts a new turn, unless one is already in flight.
    ///
    /// The kernel runs one turn per session and refuses a second `turn/start`
    /// with a typed `turn_in_progress` error, so the host refuses it here, where
    /// the person gets a message, rather than paying a round trip to lose it.
    fn begin_turn(&self, id: TurnId, end: oneshot::Sender<TurnEnd>) -> Result<(), TurnId> {
        let mut state = self.turn.lock().expect("octos turn state");
        if let Some(running) = &state.running {
            return Err(running.id.clone());
        }
        state.cancelling = false;
        state.running = Some(RunningTurn {
            id,
            end: Some(end),
        });
        Ok(())
    }

    /// The turn in flight, if any: the only id an interrupt may name.
    fn turn_in_flight(&self) -> Option<TurnId> {
        self.turn
            .lock()
            .expect("octos turn state")
            .running
            .as_ref()
            .map(|running| running.id.clone())
    }

    /// Marks this session as cancelling, unless it already is.
    ///
    /// `false` means a cancel is already in flight for this turn, so a second
    /// `turn/interrupt` would be a second claim about it.
    fn begin_cancelling(&self) -> bool {
        let mut state = self.turn.lock().expect("octos turn state");
        if state.cancelling || state.running.is_none() {
            return false;
        }
        state.cancelling = true;
        true
    }

    /// The interrupt did not land: the turn is still the kernel's, so it stays
    /// in flight and a later interrupt may name it again.
    fn abort_cancelling(&self) {
        self.turn.lock().expect("octos turn state").cancelling = false;
    }

    /// Ends this session's turn with `end`.
    ///
    /// `named` is the turn the terminal frame named, when it named one. The
    /// kernel runs one turn per session, so a frame that names this thread's
    /// turn — or names none — ends it. A frame that names a *different* turn is
    /// not proof that this thread's turn ended, but leaving the prompt
    /// unresolved would be worse: it settles as an error that says what was
    /// seen. Idempotent, and a no-op when nothing is in flight.
    fn finish_turn(&self, named: Option<&TurnId>, end: TurnEnd) -> bool {
        let mut state = self.turn.lock().expect("octos turn state");
        state.cancelling = false;
        let Some(mut running) = state.running.take() else {
            return false;
        };
        let end = match named {
            Some(named) if named != &running.id => TurnEnd::Errored(format!(
                "the kernel ended turn {} while this thread was waiting on turn {}",
                named.0, running.id.0
            )),
            _ => end,
        };
        if let Some(sender) = running.end.take() {
            let _ = sender.send(end);
        }
        true
    }

    /// Adopts the turn the kernel says is already running, so a later interrupt
    /// names the turn the kernel actually has rather than the one this host
    /// asked for.
    fn adopt_turn(&self, id: TurnId) {
        let mut state = self.turn.lock().expect("octos turn state");
        match state.running.as_mut() {
            Some(running) => running.id = id,
            None => {
                state.running = Some(RunningTurn { id, end: None });
            }
        }
    }

    /// Stops waiting on this session's turn **without** forgetting which turn
    /// the kernel is running.
    ///
    /// This is what a refused `turn/start` needs: the kernel rejected *this*
    /// input, so this thread's prompt fails — but if the refusal named a turn,
    /// that turn is still the kernel's, and Stop must still be able to name it.
    fn abandon_waiter(&self) -> Option<oneshot::Sender<TurnEnd>> {
        self.turn
            .lock()
            .expect("octos turn state")
            .running
            .as_mut()
            .and_then(|running| running.end.take())
    }
}

impl AgentConnection for OctosConnection {
    fn agent_id(&self) -> project::AgentId {
        project::AgentId::new(agent_ui::octos::OCTOS_AGENT_ID)
    }

    fn telemetry_id(&self) -> SharedString {
        "octos".into()
    }

    /// No authentication: the kernel is already running and its profile is the
    /// kernel's own business, not something a client can present a token for.
    fn auth_methods(&self) -> &[acp::AuthMethod] {
        &[]
    }

    fn authenticate(&self, _method: acp::AuthMethodId, _cx: &mut App) -> Task<Result<()>> {
        Task::ready(Ok(()))
    }

    /// A kernel session outlives the thread that opened it, so a thread can be
    /// re-opened over it: `session/open` on the same key, then `session/hydrate`
    /// for the transcript the kernel already holds.
    fn supports_load_session(&self) -> bool {
        true
    }

    /// Opens an existing kernel session and delivers its transcript.
    ///
    /// The session id *is* the kernel's key (`SessionKey` is one string), so
    /// loading is `session/open` on it — the kernel either already has it in
    /// memory or reads it from the profile's store — followed by `session/hydrate`
    /// (UPCR-2026-009) and [`hydrate_updates`].
    ///
    /// The subscription is taken **before** the hydrate so no frame that lands
    /// during it is lost; the replay's cursor is remembered before the delivery
    /// task starts, so a frame that the hydrate already covered is dropped rather
    /// than applied twice ([`replayed_by_hydrate`]).
    fn load_session(
        self: Rc<Self>,
        session_id: acp::SessionId,
        project: Entity<Project>,
        work_dirs: PathList,
        title: Option<SharedString>,
        cx: &mut App,
    ) -> Task<Result<Entity<AcpThread>>> {
        let connection = self.connection.clone();
        let fanout = self.fanout.clone();
        let session = SessionKey(session_id.0.to_string());
        let cwd = work_dirs
            .paths()
            .first()
            .cloned()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        cx.spawn(async move |cx| {
            let reply = connection
                .call_with_timeout(
                    oup::methods::SESSION_OPEN,
                    super::session_open_params(&session, &cwd),
                    super::SESSION_OPEN_TIMEOUT,
                )
                .await
                .map_err(|failure| {
                    anyhow!("the Octos kernel did not answer session/open: {failure}")
                })?;
            let opened = session_open_result(&reply).map_err(|reason| {
                anyhow!("the Octos kernel did not open session {session}: {reason}")
            })?;
            log::info!("[octos] agent thread loaded session {}", opened.0);

            let frames = fanout.subscribe();
            let history = hydrate(&connection, &opened, None).await?;
            let updates = hydrate_updates(&history);
            let pending = history.pending_approvals.clone().unwrap_or_default();

            let acp_session_id = acp::SessionId::new(opened.0.clone());
            let session_state = self.session_state(&acp_session_id, opened.clone());
            // Before the delivery task starts: from here on, a live frame the
            // replay already covered is recognisable.
            session_state.set_cursor(history.cursor.clone());
            log::info!(
                "[octos] hydrated {} into its thread: {} update(s) through cursor {}:{} \
                 ({} pending approval(s))",
                opened.0,
                updates.len(),
                history.cursor.stream,
                history.cursor.seq,
                pending.len()
            );

            let capabilities = acp::PromptCapabilities::new().embedded_context(true);
            let thread = cx.update(|cx| {
                let action_log = cx.new(|_| ActionLog::new(project.clone()));
                let connection: Rc<dyn AgentConnection> = self.clone();
                cx.new(|cx| {
                    AcpThread::new(
                        None,
                        title,
                        Some(work_dirs),
                        connection,
                        project,
                        action_log,
                        acp_session_id.clone(),
                        watch::Receiver::constant(capabilities),
                        cx,
                    )
                })
            });

            // The replay goes in *before* the delivery task: the transcript is
            // the kernel's history, and a live frame that arrived during the
            // hydrate is applied after it, in arrival order.
            cx.update(|cx| {
                thread.update(cx, |thread, cx| {
                    for update in updates {
                        if let Err(error) = thread.handle_session_update(update, cx) {
                            log::warn!("[octos] the thread refused a hydrated update: {error}");
                        }
                    }
                });
            });

            deliver_frames(
                frames,
                thread.downgrade(),
                session_state.clone(),
                &connection,
                &cx,
            );

            for event in &pending {
                raise_approval(thread.downgrade(), &session_state, &connection, event, &cx);
            }

            Ok(thread)
        })
    }

    fn new_session(
        self: Rc<Self>,
        project: Entity<Project>,
        work_dirs: PathList,
        cx: &mut App,
    ) -> Task<Result<Entity<AcpThread>>> {
        let connection = self.connection.clone();
        let fanout = self.fanout.clone();
        // One session per thread. The console's own session is
        // `_main:zed:embedded`; sharing it would make the second thread's
        // `turn/start` a `turn_in_progress` refusal against the first one's turn.
        let session = thread_session_key(&next_thread_name());
        let cwd = work_dirs
            .paths()
            .first()
            .cloned()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let params = super::session_open_params(&session, &cwd);

        cx.spawn(async move |cx| {
            let reply = connection
                .call_with_timeout(
                    oup::methods::SESSION_OPEN,
                    params,
                    super::SESSION_OPEN_TIMEOUT,
                )
                .await
                .map_err(|failure| {
                    anyhow!("the Octos kernel did not answer session/open: {failure}")
                })?;

            let opened = session_open_result(&reply).map_err(|reason| {
                anyhow!("the Octos kernel did not open a session: {reason}")
            })?;
            log::info!(
                "[octos] agent thread session {} is open",
                opened.0
            );

            let acp_session_id = acp::SessionId::new(opened.0.clone());
            let session_state = self.session_state(&acp_session_id, opened);

            // `AsyncApp::update` hands the closure's value back directly: the
            // entity creation cannot fail here, and if the app is gone this task
            // is not polled at all.
            //
            // `embedded_context`: resource blocks are accepted and degraded to
            // their text by `prompt_text`, and resource *links* are baseline ACP.
            // `image`/`audio` stay off — P2 has no media channel, and advertising
            // one would invite a prompt this backend drops.
            let capabilities = acp::PromptCapabilities::new().embedded_context(true);
            let thread = cx.update(|cx| {
                let action_log = cx.new(|_| ActionLog::new(project.clone()));
                let connection: Rc<dyn AgentConnection> = self.clone();
                cx.new(|cx| {
                    AcpThread::new(
                        None,
                        None,
                        Some(work_dirs),
                        connection,
                        project,
                        action_log,
                        acp_session_id.clone(),
                        watch::Receiver::constant(capabilities),
                        cx,
                    )
                })
            });

            deliver_frames(
                fanout.subscribe(),
                thread.downgrade(),
                session_state,
                &connection,
                &cx,
            );

            Ok(thread)
        })
    }

    fn prompt(&self, params: acp::PromptRequest, cx: &mut App) -> Task<Result<acp::PromptResponse>> {
        let session = match self.session_for(&params.session_id) {
            Ok(session) => session,
            Err(error) => return Task::ready(Err(error)),
        };

        let text = prompt_text(&params.prompt);
        if text.trim().is_empty() {
            return Task::ready(Err(anyhow!(
                "this prompt carries no text, and P2 sends text only (an @file mention \
                 travels as its path, and images are not forwarded)"
            )));
        }

        let turn_id = TurnId::new();
        let turn_params = match turn_start_params(&session.session, &turn_id, &text) {
            Ok(params) => params,
            Err(error) => {
                return Task::ready(Err(anyhow!("could not encode the turn: {error}")));
            }
        };

        // Registered *before* the frame goes out: a terminal frame for this turn
        // cannot be waited on if the place its answer goes does not exist yet.
        let (end_tx, end_rx) = oneshot::channel();
        if let Err(running) = session.begin_turn(turn_id.clone(), end_tx) {
            return Task::ready(Err(anyhow!(
                "a turn is already running on this Octos session ({}); interrupt it first",
                running.0
            )));
        }

        let connection = self.connection.clone();
        cx.spawn(async move |_cx| {
            match connection
                .call_with_timeout(
                    oup::methods::TURN_START,
                    turn_params,
                    super::TURN_START_TIMEOUT,
                )
                .await
            {
                Ok(frame) => {
                    if let Some(error) = frame.get("error") {
                        log::warn!("[octos] turn/start was refused: {error}");
                        match turn_refusal(error) {
                            TurnRefusal::TurnInProgress { turn_id: running, .. } => {
                                // The kernel is running a turn for this session
                                // after all. Adopt its id — Stop has to name
                                // that turn — and fail *this* prompt, whose
                                // input the kernel did not accept.
                                session.adopt_turn(running.clone());
                                if let Some(sender) = session.abandon_waiter() {
                                    let _ = sender.send(TurnEnd::Errored(format!(
                                        "the kernel is already running turn {} on this session",
                                        running.0
                                    )));
                                }
                            }
                            refusal => {
                                session.finish_turn(None, TurnEnd::Errored(refusal.message()));
                            }
                        }
                    }
                }
                Err(failure) => {
                    // A timeout is not a refusal: the kernel may be running the
                    // turn right now. But without an acknowledgement this host
                    // cannot even name the turn to interrupt, and no terminal
                    // frame is guaranteed, so the prompt is settled as an error
                    // instead of hanging the thread forever.
                    log::warn!("[octos] no turn/start acknowledgement: {failure}");
                    session.finish_turn(
                        None,
                        TurnEnd::Errored(format!(
                            "the Octos kernel did not acknowledge the turn: {failure}"
                        )),
                    );
                }
            }

            match end_rx.await {
                Ok(end) => prompt_response(end),
                // The sender is only dropped when the session was dropped, i.e.
                // when nothing can end this turn any more.
                Err(_) => Err(anyhow!(
                    "the Octos session was closed before the turn ended"
                )),
            }
        })
    }

    fn cancel(&self, session_id: &acp::SessionId, cx: &mut App) {
        let session = match self.session_for(session_id) {
            Ok(session) => session,
            Err(error) => {
                log::warn!("[octos] cancel: {error}");
                return;
            }
        };

        // An interrupt is a claim about a named turn; with nothing in flight
        // there is no honest one to make.
        let Some(turn) = session.turn_in_flight() else {
            log::info!("[octos] no turn in flight on {}: nothing to interrupt", session_id.0);
            return;
        };
        if !session.begin_cancelling() {
            log::info!(
                "[octos] an interrupt for turn {} is already in flight",
                turn.0
            );
            return;
        }

        let params = match serde_json::to_value(TurnInterruptParams {
            session_id: session.session.clone(),
            turn_id: turn.clone(),
        }) {
            Ok(params) => params,
            Err(error) => {
                log::error!("[octos] could not encode turn/interrupt: {error}");
                session.abort_cancelling();
                return;
            }
        };

        let connection = self.connection.clone();
        let session_for_reply = session.clone();
        cx.spawn(async move |cx| {
            let reply = connection
                .call_with_timeout(
                    oup::methods::TURN_INTERRUPT,
                    params,
                    super::TURN_INTERRUPT_TIMEOUT,
                )
                .await;

            cx.update(|cx| match reply {
                Ok(frame) => match interrupt_outcome(&frame) {
                    InterruptOutcome::Interrupted => {
                        // The receipt says the kernel captured it; the turn ends
                        // when its terminal frame arrives. Saying "interrupted"
                        // here would be a claim ahead of the evidence.
                        log::info!("[octos] the kernel is interrupting turn {}", turn.0);
                        super::push_note(
                            cx,
                            format!("cancelling turn {} (the kernel captured the interrupt)", turn.0),
                        );
                    }
                    InterruptOutcome::AlreadyOver { terminal_state } => {
                        // The turn had already reached a terminal state, and
                        // this host never saw that terminal frame (it was lost,
                        // or the turn was ended elsewhere). Nothing further is
                        // coming, so the turn is settled here rather than left
                        // generating forever.
                        log::info!(
                            "[octos] turn {} had already ended ({terminal_state})",
                            turn.0
                        );
                        session_for_reply.finish_turn(None, TurnEnd::Interrupted);
                        super::push_note(
                            cx,
                            format!("turn {} had already ended ({terminal_state})", turn.0),
                        );
                    }
                    InterruptOutcome::Declined { reason } => {
                        let reason = reason.unwrap_or_else(|| "no reason given".to_owned());
                        log::warn!("[octos] the kernel declined the interrupt: {reason}");
                        session_for_reply.abort_cancelling();
                        super::push_note(
                            cx,
                            format!(
                                "the kernel declined to interrupt turn {}: {reason} \
                                 (it is still running)",
                                turn.0
                            ),
                        );
                    }
                    InterruptOutcome::Unreadable => {
                        log::warn!("[octos] unreadable turn/interrupt reply: {frame}");
                        session_for_reply.abort_cancelling();
                        super::push_note(
                            cx,
                            "the kernel's interrupt reply was not a turn/interrupt result — \
                             the turn may still be running (see Zed.log)"
                                .to_owned(),
                        );
                    }
                },
                Err(failure) => {
                    log::warn!("[octos] interrupt not acknowledged: {failure}");
                    session_for_reply.abort_cancelling();
                    super::push_note(
                        cx,
                        format!(
                            "the interrupt of turn {} was not acknowledged: {failure}",
                            turn.0
                        ),
                    );
                }
            })
        })
        .detach();
    }

    fn set_title(
        &self,
        session_id: &acp::SessionId,
        _cx: &App,
    ) -> Option<Rc<dyn AgentSessionSetTitle>> {
        let session = self.session_for(session_id).ok()?;
        Some(Rc::new(OctosSetTitle {
            connection: self.connection.clone(),
            session,
        }))
    }

    /// This thread's model picker, over the kernel's own `profile/llm/list`.
    ///
    /// `None` for a session this connection never opened — the same answer every
    /// other entry point gives ([`Self::session_for`]), and the reason the picker
    /// is built from the live [`ThreadSession`] rather than from an id alone: a
    /// picker for a session the kernel does not have could only fail its calls.
    ///
    /// The list is Octos's ([`super::model_selector`]) and the credential a
    /// selection may need is bridged into Octos's own store
    /// ([`super::key_bridge`]); neither direction mirrors Zed's models.
    fn model_selector(&self, session_id: &acp::SessionId) -> Option<Rc<dyn AgentModelSelector>> {
        let session = self.session_for(session_id).ok()?;
        Some(Rc::new(super::model_selector::OctosModelSelector::new(
            self.connection.clone(),
            session.session.clone(),
        )))
    }

    /// Session history, from the kernel's own `session/list`.
    ///
    /// The listing is the kernel's: this host offers the rows the kernel reports
    /// and nothing else — no synthesised entries, no rows invented from the
    /// threads it happens to have open.
    fn session_list(&self, _cx: &mut App) -> Option<Rc<dyn AgentSessionList>> {
        Some(Rc::new(OctosSessionList {
            connection: self.connection.clone(),
        }))
    }

    fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
        self
    }
}

/// `session/list` for the embedded kernel.
struct OctosSessionList {
    connection: Connection,
}

impl AgentSessionList for OctosSessionList {
    fn list_sessions(
        &self,
        request: AgentSessionListRequest,
        cx: &mut App,
    ) -> Task<Result<AgentSessionListResponse>> {
        let connection = self.connection.clone();
        cx.foreground_executor().spawn(async move {
            let params = session_list_params(&request)
                .map_err(|error| anyhow!("could not encode session/list: {error}"))?;
            let reply = connection
                .call_with_timeout(oup::methods::SESSION_LIST, params, SESSION_LIST_TIMEOUT)
                .await
                .map_err(|failure| {
                    anyhow!("the Octos kernel did not answer session/list: {failure}")
                })?;
            if let Some(error) = reply.get("error") {
                return Err(anyhow!("the Octos kernel refused session/list: {error}"));
            }
            let result: oup::SessionListResult = serde_json::from_value(
                reply.get("result").cloned().unwrap_or(Value::Null),
            )
            .context("the kernel's session/list reply was not a SessionListResult")?;
            let sessions = listed_sessions(&result);
            log::info!(
                "[octos] the kernel listed {} session(s){}",
                sessions.len(),
                match &result.workspace_root {
                    Some(root) => format!(" under {root}"),
                    None => String::new(),
                }
            );
            Ok(AgentSessionListResponse::new(sessions))
        })
    }

    fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
        self
    }
}

/// `session/list` params for one listing request.
///
/// `cwd` is forwarded when the caller has one: the kernel only scopes the
/// listing to `<cwd>/.octos` when it is given, and the profile is the one every
/// Octos session here opens under ([`octos_core::MAIN_PROFILE_ID`]). With no
/// cwd the request is the historical empty object, which is the per-profile
/// global listing.
fn session_list_params(request: &AgentSessionListRequest) -> serde_json::Result<Value> {
    let cwd = request
        .cwd
        .as_ref()
        .map(|cwd| cwd.to_string_lossy().into_owned());
    serde_json::to_value(oup::SessionListParams {
        profile_id: cwd.as_ref().map(|_| octos_core::MAIN_PROFILE_ID.to_owned()),
        cwd,
    })
}

/// The rows of a `session/list` result, as agent-thread session infos.
///
/// [`oup::SessionListResult::sessions`] is deliberately an opaque `Value` — the
/// protocol crate forwards the REST handler's `SessionInfo` body byte-for-byte
/// rather than duplicating its DTO (`ui_protocol.rs:3362-3367`) — so the three
/// fields a session row needs are read here by name, and a row that does not
/// carry an `id` is **skipped**: an entry with an invented id would be a session
/// the person could click and that the kernel has never heard of. `updated_at`
/// is RFC3339 in the REST body; anything else is left absent rather than guessed.
fn listed_sessions(result: &oup::SessionListResult) -> Vec<AgentSessionInfo> {
    let Some(rows) = result.sessions.as_array() else {
        log::warn!(
            "[octos] session/list returned {} rather than an array of sessions; \
             offering none",
            result.sessions
        );
        return Vec::new();
    };
    let mut sessions = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(id) = row.get("id").and_then(Value::as_str) else {
            log::warn!("[octos] a session/list row carries no id: {row}");
            continue;
        };
        let mut info = AgentSessionInfo::new(id.to_owned());
        info.title = row
            .get("title")
            .and_then(Value::as_str)
            .map(SharedString::from);
        info.updated_at = row
            .get("updated_at")
            .and_then(Value::as_str)
            .and_then(|stamp| chrono::DateTime::parse_from_rfc3339(stamp).ok())
            .map(|stamp| stamp.with_timezone(&chrono::Utc));
        sessions.push(info);
    }
    sessions
}

/// `session/title.set` for one thread's session.
struct OctosSetTitle {
    connection: Connection,
    session: Arc<ThreadSession>,
}

impl AgentSessionSetTitle for OctosSetTitle {
    fn run(&self, title: SharedString, cx: &mut App) -> Task<Result<()>> {
        let params = match title_set_params(&self.session.session, &title) {
            Ok(params) => params,
            Err(error) => {
                return Task::ready(Err(anyhow!("could not encode the title: {error}")));
            }
        };
        let connection = self.connection.clone();
        cx.spawn(async move |_cx| {
            let reply = connection
                .call_with_timeout(oup::methods::SESSION_TITLE_SET, params, TITLE_SET_TIMEOUT)
                .await
                .map_err(|failure| {
                    anyhow!("the Octos kernel did not answer session/title.set: {failure}")
                })?;
            if let Some(error) = reply.get("error") {
                return Err(anyhow!("the Octos kernel refused the title: {error}"));
            }
            let result: oup::SessionTitleSetResult = serde_json::from_value(
                reply.get("result").cloned().unwrap_or(Value::Null),
            )
            .context("the kernel's session/title.set reply was not a SessionTitleSetResult")?;
            log::info!(
                "[octos] the kernel titled session {} {:?}",
                result.session_id,
                result.title
            );
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Delivery
// ---------------------------------------------------------------------------

/// Subscribes one thread to the kernel's frames and feeds its transcript.
///
/// One subscription per thread, taken off the process-wide fan-out
/// ([`Octos::subscribe`]), because a thread and its receiver should die
/// together: the task ends as soon as the thread entity is gone.
///
/// Host-tool frames never appear here — `peer/tool/call` and `peer/tool/cancel`
/// are served in place on the kernel's runtime by [`super::forward_notifications`]
/// and are not fanned out at all (the kernel blocks on `peer/tool/result` while
/// it runs the turn).
fn deliver_frames(
    mut frames: broadcast::Receiver<OctosFrame>,
    thread: WeakEntity<AcpThread>,
    session: Arc<ThreadSession>,
    connection: &Connection,
    cx: &AsyncApp,
) {
    let session_key = session.session.clone();
    let connection = connection.clone();
    cx.spawn(async move |cx| {
        // Why the loop below ended: the two reasons need different words, and
        // neither may be guessed.
        let mut stream_over = false;
        loop {
            let Some(frame) = super::next_frame(&mut frames).await else {
                stream_over = true;
                break;
            };
            // The resync marker is this host's own, and it means frames are
            // *gone* — for this thread as much as for any other. The session is
            // re-read here rather than described as incomplete.
            if frame.is_resync() {
                let missed = frame.missed_frames().unwrap_or(0);
                log::warn!(
                    "[octos] this thread's subscriber lost {missed} frame(s); \
                     re-reading the session's state"
                );
                if let Err(error) = resync(&connection, &session, &thread, missed, &cx).await {
                    // The session could not be re-read: the thread keeps what it
                    // has, and the person is told the resync did not happen
                    // rather than being left to believe it did.
                    log::warn!(
                        "[octos] could not re-read the session after losing {missed} frame(s): \
                         {error}"
                    );
                    let note = format!(
                        "the kernel stream fell {missed} frame(s) behind, and re-reading the \
                         session failed: {error}"
                    );
                    cx.update(|cx| super::push_note(cx, note));
                }
                continue;
            }

            // The fan-out is process-wide: a frame from another thread's session
            // (or from the console's own `_main:zed:embedded` session) must never
            // be rendered into this transcript.
            if frame_session(&frame).as_ref() != Some(&session_key) {
                continue;
            }

            // A frame the last hydrate already replayed must not be applied a
            // second time: the transcript would show the same answer twice.
            if session
                .cursor()
                .is_some_and(|cursor| replayed_by_hydrate(&frame, &cursor))
            {
                log::debug!(
                    "[octos] dropping a frame the hydrate replay already covered: {}",
                    frame.method
                );
                continue;
            }

            let effects = thread_effects(&frame);
            if effects.is_empty() {
                continue;
            }

            let mut gone = false;
            for update in effects.updates {
                let applied: bool = cx.update(|cx| {
                    thread
                        .update(cx, |thread, cx| {
                            if let Err(error) = thread.handle_session_update(update, cx) {
                                log::warn!("[octos] the thread refused a session update: {error}");
                            }
                        })
                        .is_ok()
                });
                // A failed update means the thread (or the app) is gone; either
                // way there is nobody left to render for.
                if !applied {
                    gone = true;
                    break;
                }
            }

            // Settled even if the thread went away: the prompt future is what
            // the panel is waiting on, and it must not outlive the transcript.
            if let Some((named, end)) = effects.ended {
                session.finish_turn(named.as_ref(), end);
            }

            // Raised whatever else this frame did: an approval is the kernel
            // waiting on the person, which is not a turn ending.
            if let Some(approval) = effects.approval {
                raise_approval(thread.clone(), &session, &connection, &approval, &cx);
            }

            if let Some((approval_id, settlement)) = effects.settled {
                settle_approval(thread.clone(), &session, &approval_id, settlement, &cx);
            }

            if let Some(note) = effects.note {
                cx.update(|cx| super::push_note(cx, note));
            }

            if gone {
                break;
            }
        }

        // Either way no further frame can settle a turn still in flight, and
        // saying so is the difference between an error the person sees and a
        // thread that spins forever. The two causes are not the same thing, so
        // they do not get the same words.
        let reason = if stream_over {
            "the Octos kernel connection closed before the turn ended"
        } else {
            "the agent thread was closed before the turn ended"
        };
        session.finish_turn(None, TurnEnd::Errored(reason.to_owned()));

        // ...and the same for the approvals: the kernel's connection (or the
        // thread) is gone, so a card still waiting for the person can never be
        // answered. It is resolved as cancelled — *not* as denied, which would
        // be a decision nobody made — and said out loud.
        let abandoned = session.abandon_approvals();
        if !abandoned.is_empty() {
            log::warn!(
                "[octos] {} approval(s) were still waiting when the thread ended; \
                 resolving their cards as cancelled",
                abandoned.len()
            );
            for pending in &abandoned {
                cx.update(|cx| {
                    thread
                        .update(cx, |thread, cx| {
                            thread.cancel_tool_call_authorization(&pending.tool_call_id, cx);
                        })
                        .ok();
                });
            }
            cx.update(|cx| {
                super::push_note(
                    cx,
                    format!(
                        "{} approval(s) were abandoned unanswered when the Octos thread ended; \
                         their cards are cancelled",
                        abandoned.len()
                    ),
                );
            });
        }
    })
    .detach();
}

// ---------------------------------------------------------------------------
// The translation (pure)
// ---------------------------------------------------------------------------

/// How a turn ended, as the kernel's terminal frames report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEnd {
    /// `turn/completed`, or a `turn_terminal` envelope with `completed`.
    Completed,
    /// `turn_terminal` with `interrupted`: the interrupt the host sent landed.
    Interrupted,
    /// `turn/error`, or a `turn_terminal` envelope with `errored`. The string is
    /// the kernel's own reason, already rendered for a person.
    Errored(String),
    /// `turn_terminal` with `rate_limited`. The turn is over; the provider, not
    /// the person, stopped it.
    RateLimited,
}

impl TurnEnd {
    /// The ACP stop reason for a turn that ended *without* an error.
    ///
    /// An errored turn has no stop reason at all: it is reported as a failed
    /// prompt, which is what marks the thread as having had an error.
    fn stop_reason(&self) -> acp::StopReason {
        match self {
            Self::Interrupted => acp::StopReason::Cancelled,
            Self::Completed | Self::RateLimited | Self::Errored(_) => acp::StopReason::EndTurn,
        }
    }
}

/// Turns one settled turn into the ACP response the thread is waiting on.
fn prompt_response(end: TurnEnd) -> Result<acp::PromptResponse> {
    match &end {
        TurnEnd::Errored(reason) => Err(anyhow!(reason.clone())),
        _ => Ok(acp::PromptResponse::new(end.stop_reason())),
    }
}

/// Everything one kernel frame means for one agent thread.
///
/// A struct rather than a bare `Vec<SessionUpdate>` because a frame carries
/// several different things that must not be conflated: transcript updates, the
/// end of the turn the thread is waiting on, an approval the person has to
/// answer, the kernel's own settlement of one, and a line for the Octos console.
/// A terminal frame ends a turn *and* carries no update; a warning is a note and
/// nothing else.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameEffects {
    /// The ACP updates the thread's transcript receives, in the frame's order.
    pub updates: Vec<acp::SessionUpdate>,
    /// The turn this frame ends, with the turn id the frame named (if any).
    pub ended: Option<(Option<TurnId>, TurnEnd)>,
    /// An approval the kernel parked this session on: the card belongs on the
    /// thread, and the person's answer is `approval/respond`.
    pub approval: Option<Box<ApprovalRequestedEvent>>,
    /// An approval the kernel settled. Ids only: the kernel's decision, never the
    /// person's intent, is what lands a state.
    pub settled: Option<(ApprovalId, ApprovalSettlement)>,
    /// A line for the Octos console: progress, a warning, the kernel's reason.
    pub note: Option<String>,
}

impl FrameEffects {
    fn is_empty(&self) -> bool {
        self.updates.is_empty()
            && self.ended.is_none()
            && self.approval.is_none()
            && self.settled.is_none()
            && self.note.is_none()
    }

    fn push(&mut self, update: acp::SessionUpdate) {
        self.updates.push(update);
    }

    fn end(&mut self, turn: Option<TurnId>, end: TurnEnd) {
        self.ended = Some((turn, end));
    }

    fn note(&mut self, note: impl Into<String>) {
        self.note = Some(note.into());
    }
}

/// The ACP updates one kernel frame produces for an agent thread.
///
/// The pure reading of a frame; the delivery task is only plumbing around it.
pub fn thread_effects(frame: &OctosFrame) -> FrameEffects {
    let Some(notification) =
        UiNotification::from_method_and_params(&frame.method, frame.params.clone()).ok()
    else {
        // Not a notification this build knows. Nothing is rendered, by design —
        // guessing at an unknown payload is how a transcript grows text the
        // kernel never sent.
        return FrameEffects::default();
    };
    effects_of(notification)
}

/// The whole reading of one decoded notification.
fn effects_of(notification: UiNotification) -> FrameEffects {
    let mut effects = FrameEffects::default();
    match notification {
        // The authoritative assistant/reasoning lanes. `MessageDeltaEvent::text`
        // and `ReasoningDeltaEvent::text` are the only fields these frames
        // carry; a frame spelled `{"delta": …}` does not decode and renders
        // nothing.
        UiNotification::MessageDelta(event) => {
            effects.push(acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                acp::ContentBlock::from(event.text),
            )));
        }
        UiNotification::ReasoningDelta(event) => {
            effects.push(acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
                acp::ContentBlock::from(event.text),
            )));
        }
        UiNotification::ToolStarted(event) => {
            effects.push(acp::SessionUpdate::ToolCall(
                acp::ToolCall::new(event.tool_call_id, event.tool_name.clone())
                    .kind(tool_kind_for(&event.tool_name))
                    .status(acp::ToolCallStatus::InProgress),
            ));
        }
        UiNotification::ToolProgress(event) => {
            // Deliberately *not* a `ToolCall` `content` update: ACP's content
            // list is positional and an update rewrites it in place, so a
            // progress line sent as content would overwrite the tool's output.
            // Progress is what the Octos console is for.
            effects.note(match event.message {
                Some(message) => message,
                None => format!(
                    "a tool is {:.0}% done",
                    event.progress_pct.unwrap_or_default()
                ),
            });
        }
        UiNotification::ToolCompleted(event) => {
            effects.push(tool_end_update(
                event.tool_call_id.clone(),
                event.success.unwrap_or(false),
                event.output_preview,
            ));
        }
        UiNotification::PlanUpdated(event) => {
            effects.push(acp::SessionUpdate::Plan(plan_from_record(&event.plan)));
        }
        UiNotification::ProgressUpdated(event) => {
            effects.note(
                event
                    .metadata
                    .label
                    .or(event.metadata.message)
                    .unwrap_or(event.metadata.kind),
            );
        }
        UiNotification::Warning(event) => {
            effects.note(format!("warning [{}]: {}", event.code, event.message));
        }
        // The kernel parked the session on an approval. The card belongs on this
        // thread; the decision stays the kernel's (`approval/decided` and
        // friends come back as their own frames).
        UiNotification::ApprovalRequested(event) => {
            effects.approval = Some(Box::new(event));
        }
        UiNotification::ApprovalDecided(event) => {
            match settlement_of(&event.decision) {
                Some(settlement) => effects.settled = Some((event.approval_id, settlement)),
                // A decision this build does not know is not a decision it may
                // record: the card stays as it is and the raw string is said out
                // loud instead.
                None => effects.note(format!(
                    "the kernel decided approval {} as {:?}, which this build does not \
                     understand; its card is left as it is",
                    event.approval_id.0,
                    event.decision.as_wire_str()
                )),
            }
        }
        UiNotification::ApprovalAutoResolved(event) => {
            match settlement_of(&event.decision) {
                Some(settlement) => effects.settled = Some((event.approval_id, settlement)),
                None => effects.note(format!(
                    "the kernel auto-resolved approval {} as {:?}, which this build does not \
                     understand; its card is left as it is",
                    event.approval_id.0,
                    event.decision.as_wire_str()
                )),
            }
        }
        UiNotification::ApprovalCancelled(event) => {
            effects.note(format!(
                "the kernel cancelled approval {}: {}",
                event.approval_id.0, event.reason
            ));
            effects.settled = Some((event.approval_id, ApprovalSettlement::Cancelled));
        }
        UiNotification::TurnCompleted(event) => {
            effects.end(Some(event.turn_id), TurnEnd::Completed);
        }
        UiNotification::TurnError(event) => {
            effects.end(
                Some(event.turn_id),
                TurnEnd::Errored(format!("turn failed [{}]: {}", event.code, event.message)),
            );
        }
        // The canonical projection lane. Since #2318 the kernel suppresses the
        // raw `tool/*` lifecycle frames for a connection that negotiated
        // `projection.envelope.v1`, so a client that only reads the raw lane
        // sees no tool activity at all; the envelope carries the same
        // information, from its own type.
        UiNotification::EnvelopeV2(event) => payload_effects(&event.envelope.payload, &mut effects),
        _ => {}
    }
    effects
}

/// What one `projection/envelope` payload means for an agent thread.
///
/// Deliberately narrower than [`effects_of`]: only the payloads the transcript
/// renders are mapped.
fn payload_effects(payload: &PayloadV2, effects: &mut FrameEffects) {
    match payload {
        // **Not rendered.** `assistant_delta` carries the same text that also
        // arrives on `message/delta`; rendering both doubles every answer. The
        // same reasoning applies to `assistant_persisted`, whose text the
        // deltas have already delivered.
        PayloadV2::AssistantDelta { .. } | PayloadV2::AssistantPersisted { .. } => {}
        PayloadV2::ReasoningDelta { .. } => {
            // Also on `reasoning/delta`; rendering both would double the
            // thinking block.
        }
        PayloadV2::ToolStart {
            tool_call_id,
            name,
            arguments_preview: _,
        } => {
            effects.push(acp::SessionUpdate::ToolCall(
                acp::ToolCall::new(tool_call_id.clone(), name.clone())
                    .kind(tool_kind_for(name))
                    .status(acp::ToolCallStatus::InProgress),
            ));
        }
        PayloadV2::ToolProgress { message, .. } => {
            effects.note(message.clone());
        }
        PayloadV2::ToolEnd {
            tool_call_id,
            status,
            error,
            output_preview,
            ..
        } => {
            effects.push(tool_end_update(
                tool_call_id.clone(),
                matches!(status, EnvelopeToolEndStatus::Complete),
                output_preview.clone().or_else(|| error.clone()),
            ));
        }
        PayloadV2::TurnTerminal { outcome, error, .. } => {
            let end = match outcome {
                TurnTerminalOutcome::Completed => TurnEnd::Completed,
                TurnTerminalOutcome::Errored => TurnEnd::Errored(match error {
                    Some(error) => format!("turn failed [{}]: {}", error.code, error.message),
                    None => "the turn failed".to_owned(),
                }),
                TurnTerminalOutcome::Interrupted => TurnEnd::Interrupted,
                TurnTerminalOutcome::RateLimited => TurnEnd::RateLimited,
            };
            effects.end(None, end);
        }
        _ => {}
    }
}

/// A tool completion, whose `content` is the one line the kernel previewed.
fn tool_end_update(
    tool_call_id: impl Into<acp::ToolCallId>,
    success: bool,
    outcome: Option<String>,
) -> acp::SessionUpdate {
    let status = if success {
        acp::ToolCallStatus::Completed
    } else {
        acp::ToolCallStatus::Failed
    };
    let mut fields = acp::ToolCallUpdateFields::new().status(status);
    if let Some(outcome) = outcome {
        fields = fields.content(vec![acp::ToolCallContent::from(acp::ContentBlock::from(
            outcome,
        ))]);
    }
    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(tool_call_id, fields))
}

/// The kernel's live checklist as an ACP plan.
///
/// `priority` is carried faithfully and *only* `priority`: the OUP plan item's
/// `priority` is a free-form display chip (`"P3"`, `"high"`, …), not an ACP
/// priority, so promoting it to `High`/`Low` would be a guess rendered as fact.
/// Every entry is `Medium` and the chip is dropped.
fn plan_from_record(record: &UiPlanRecord) -> acp::Plan {
    acp::Plan::new(
        record
            .items
            .iter()
            .map(|item| {
                acp::PlanEntry::new(
                    item.title.clone(),
                    acp::PlanEntryPriority::Medium,
                    match item.status {
                        PlanItemStatus::Pending => acp::PlanEntryStatus::Pending,
                        PlanItemStatus::InProgress => acp::PlanEntryStatus::InProgress,
                        PlanItemStatus::Completed => acp::PlanEntryStatus::Completed,
                    },
                )
            })
            .collect(),
    )
}

/// Best-effort mapping from an Octos tool name to an ACP [`acp::ToolKind`] so
/// the transcript can pick an icon. Unknown tools fall back to `Other`.
fn tool_kind_for(name: &str) -> acp::ToolKind {
    match name {
        "read_file" | "list_dir" => acp::ToolKind::Read,
        "write_file" | "edit_file" => acp::ToolKind::Edit,
        "glob" | "grep" => acp::ToolKind::Search,
        "shell" => acp::ToolKind::Execute,
        "web_search" | "web_fetch" => acp::ToolKind::Fetch,
        _ => acp::ToolKind::Other,
    }
}

// ---------------------------------------------------------------------------
// Hydration (pure, plus one call)
// ---------------------------------------------------------------------------

/// The `session/hydrate` params for one thread.
///
/// `include` names the four sections a thread renders — the transcript rows
/// ([`hydrate_sections::MESSAGES`]), the thread graph
/// ([`hydrate_sections::THREADS`]), the turn lifecycle
/// ([`hydrate_sections::TURNS`]) and the approvals the kernel is still waiting on
/// ([`hydrate_sections::PENDING_APPROVALS`]) — as the protocol's own tokens,
/// never as strings typed here. `after` is the thread's checkpoint: `Some` asks
/// for only what came after it, `None` for the whole session.
fn hydrate_params(session: &SessionKey, after: Option<UiCursor>) -> serde_json::Result<Value> {
    serde_json::to_value(SessionHydrateParams {
        session_id: session.clone(),
        after,
        include: vec![
            hydrate_sections::MESSAGES.to_owned(),
            hydrate_sections::THREADS.to_owned(),
            hydrate_sections::TURNS.to_owned(),
            hydrate_sections::PENDING_APPROVALS.to_owned(),
        ],
    })
}

/// One `session/hydrate` over the embedded kernel.
///
/// Decoded with `SessionHydrateResult` — the kernel's own type — rather than by
/// reading `result["messages"]` by hand, so the reply cannot drift from this
/// host silently.
async fn hydrate(
    connection: &Connection,
    session: &SessionKey,
    after: Option<UiCursor>,
) -> Result<SessionHydrateResult> {
    let params = hydrate_params(session, after)
        .map_err(|error| anyhow!("could not encode session/hydrate: {error}"))?;
    let reply = connection
        .call_with_timeout(oup::methods::SESSION_HYDRATE, params, HYDRATE_TIMEOUT)
        .await
        .map_err(|failure| anyhow!("the Octos kernel did not answer session/hydrate: {failure}"))?;
    if let Some(error) = reply.get("error") {
        return Err(anyhow!(
            "the Octos kernel refused to hydrate {}: {error}",
            session
        ));
    }
    serde_json::from_value(reply.get("result").cloned().unwrap_or(Value::Null))
        .context("the kernel's session/hydrate reply was not a SessionHydrateResult")
}

/// The ACP updates one `session/hydrate` result replays into a thread.
///
/// This is the load path's transcript, and the order is the whole point:
///
/// * each row is replayed as its own update ([`replay_message`]): a `user` row
///   is the person's own message, an `assistant` row its answer, and an
///   assistant row's `reasoning_content` the thought block *before* that answer;
/// * a `tool` row is **not** rendered. Tool cards are rebuilt from
///   `replayed_tool_envelopes`, keyed by the row's `thread_id`, and each thread's
///   tools are inserted **before that thread's last answer**
///   (`crates/octos/octos-cli/src/commands/acp/oup.rs:221-259`) rather than
///   dumped below the whole conversation. Reconstructing a tool call out of prose
///   is exactly the guess this avoids.
///
/// A thread with no last answer (its rows carry no `thread_id`, or no assistant
/// row) keeps its envelopes until the end and they are replayed there, in
/// kernel order.
pub fn hydrate_updates(result: &SessionHydrateResult) -> Vec<acp::SessionUpdate> {
    let messages = result.messages.clone().unwrap_or_default();
    // The last answer of each thread: the row whose thread's tools belong above.
    let mut last_answers: HashMap<String, u64> = HashMap::new();
    for message in &messages {
        if message.role == "assistant"
            && let Some(thread) = &message.thread_id
        {
            last_answers.insert(thread.clone(), message.seq);
        }
    }

    let mut tools = result.replayed_tool_envelopes.clone().unwrap_or_default();
    let mut updates = Vec::new();
    for message in &messages {
        if message
            .thread_id
            .as_ref()
            .is_some_and(|thread| last_answers.get(thread) == Some(&message.seq))
        {
            let (matching, remaining): (Vec<_>, Vec<_>) = tools
                .into_iter()
                .partition(|envelope| Some(&envelope.thread_id) == message.thread_id.as_ref());
            tools = remaining;
            for envelope in matching {
                updates.extend(payload_updates(&envelope.payload));
            }
        }
        updates.extend(replay_message(message));
    }
    // Anything left belonged to a thread with no answer in the replay window (or
    // to no thread at all): it is still the kernel's, so it is still rendered.
    for envelope in tools {
        updates.extend(payload_updates(&envelope.payload));
    }
    updates
}

/// One hydrated row as the updates a thread renders.
///
/// The reading is the ACP backend's own (`oup.rs:262-282`), including its two
/// edges: a `tool` row renders nothing at all (its card came from the tool
/// envelopes), and an assistant row whose `reasoning_content` is present and
/// non-blank produces its thought block *before* its answer.
pub fn replay_message(message: &HydratedMessage) -> Vec<acp::SessionUpdate> {
    let mut updates = Vec::new();
    if message.role == "assistant"
        && let Some(reasoning) = message
            .reasoning_content
            .as_ref()
            .filter(|text| !text.trim().is_empty())
    {
        updates.push(acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
            acp::ContentBlock::from(reasoning.clone()),
        )));
    }
    if !message.content.trim().is_empty() {
        let chunk = acp::ContentChunk::new(acp::ContentBlock::from(message.content.clone()));
        match message.role.as_str() {
            "user" => updates.push(acp::SessionUpdate::UserMessageChunk(chunk)),
            "assistant" => updates.push(acp::SessionUpdate::AgentMessageChunk(chunk)),
            _ => {}
        }
    }
    updates
}

/// The updates one replayed `projection/envelope` payload carries.
///
/// The same reading a live envelope gets ([`payload_effects`]), so a tool card
/// rebuilt from a hydrate is the tool card live delivery would have produced.
fn payload_updates(payload: &PayloadV2) -> Vec<acp::SessionUpdate> {
    let mut effects = FrameEffects::default();
    payload_effects(payload, &mut effects);
    effects.updates
}

/// Whether a live frame is one a hydrate replay already covered.
///
/// Only frames that carry their own cursor can be judged: `projection/envelope`
/// and `turn/completed` carry a [`UiCursor`], and everything else (a delta, a
/// tool lifecycle frame) is live by construction and is applied. A cursor from a
/// *different* stream is not comparable — comparing two streams' sequence
/// numbers is how a frame that is not a duplicate would be dropped.
///
/// The comparison is `<=`: hydrate's cursor is the highest record it included
/// ("only items strictly greater than `after`" go back), so a frame at or below
/// it was part of the reply.
pub fn replayed_by_hydrate(frame: &OctosFrame, cursor: &UiCursor) -> bool {
    let Ok(notification) =
        UiNotification::from_method_and_params(&frame.method, frame.params.clone())
    else {
        return false;
    };
    let frame_cursor = match &notification {
        UiNotification::EnvelopeV2(event) => event.envelope.cursor.as_ref(),
        UiNotification::TurnCompleted(event) => event.cursor.as_ref(),
        _ => None,
    };
    frame_cursor.is_some_and(|frame| frame.stream == cursor.stream && frame.seq <= cursor.seq)
}

// ---------------------------------------------------------------------------
// Resynchronization (one call, one pure reading)
// ---------------------------------------------------------------------------

/// What one hydrate says a thread that lost frames must converge to.
///
/// Re-reading a session does **not** mean re-typing its transcript into a thread
/// that already shows it: the rows the thread rendered live are not
/// distinguishable from the rows it missed, and replaying all of them would show
/// the same answer twice. What can be converged without inventing anything is
/// the *state*: which turn the kernel has, how it ended, which approvals are
/// still waiting, and where the ledger checkpoint now is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResyncConvergence {
    /// The checkpoint the hydrate read through.
    pub cursor: UiCursor,
    /// The turn the kernel reports as over, with the words for it. `None` when
    /// the kernel reports no finished turn (or only a running one).
    pub ended: Option<(TurnId, TurnEnd)>,
    /// The turn the kernel is running that this host may not know about, so Stop
    /// can name the turn the kernel actually has.
    pub running: Option<TurnId>,
    /// The approvals the kernel is still waiting on.
    pub approvals: Vec<ApprovalRequestedEvent>,
    /// How many transcript rows the kernel holds for this session. Reported so
    /// the console's note can be specific about what was and was not re-typed.
    pub messages_held: usize,
}

/// The reading of one hydrate result for a thread that lost frames.
pub fn converge(result: &SessionHydrateResult) -> ResyncConvergence {
    let mut convergence = ResyncConvergence {
        cursor: result.cursor.clone(),
        ended: None,
        running: None,
        messages_held: result.messages.as_ref().map_or(0, Vec::len),
        approvals: result.pending_approvals.clone().unwrap_or_default(),
    };
    for turn in result.turns.as_deref().unwrap_or_default() {
        match turn.state {
            TurnLifecycleState::Completed => {
                convergence.ended = Some((turn.turn_id.clone(), TurnEnd::Completed));
            }
            TurnLifecycleState::Interrupted => {
                convergence.ended = Some((turn.turn_id.clone(), TurnEnd::Interrupted));
            }
            TurnLifecycleState::Errored => {
                convergence.ended = Some((
                    turn.turn_id.clone(),
                    TurnEnd::Errored(format!(
                        "the kernel reports turn {} as errored (its terminal frame was lost)",
                        turn.turn_id.0
                    )),
                ));
            }
            // `Unknown` is not evidence of anything: a turn whose state the
            // kernel cannot name is left exactly as this host had it.
            TurnLifecycleState::Unknown => {}
            TurnLifecycleState::Active | TurnLifecycleState::Interrupting => {
                // Later turns win: the kernel runs one turn per session, and the
                // last one it names is the one a `turn/interrupt` must name.
                convergence.running = Some(turn.turn_id.clone());
                convergence.ended = None;
            }
        }
    }
    convergence
}

/// Re-reads one session and returns what its thread must converge to.
///
/// `after` is the thread's own checkpoint, so the reply is the delta since the
/// last hydrate rather than the session's whole history. Kept as a free function
/// (rather than inlined in the delivery loop) so the request it builds, and the
/// reading of the reply, are testable without a gpui app.
async fn resync(
    connection: &Connection,
    session: &Arc<ThreadSession>,
    thread: &WeakEntity<AcpThread>,
    dropped: u64,
    cx: &AsyncApp,
) -> Result<ResyncConvergence> {
    let history = hydrate(connection, &session.session, session.cursor()).await?;
    let convergence = converge(&history);
    session.set_cursor(convergence.cursor.clone());

    // The turn first: a lost `turn/completed` is the failure that leaves a
    // thread generating forever, which is the whole reason this path exists.
    let mut note = format!(
        "the kernel stream fell {dropped} frame(s) behind and lost them; \
         re-read the session's state"
    );
    if let Some((turn_id, end)) = &convergence.ended {
        log::warn!(
            "[octos] resync found turn {} over after {} lost frame(s)",
            turn_id.0,
            dropped
        );
        session.finish_turn(Some(turn_id), end.clone());
        note.push_str(&format!(": turn {} is over", turn_id.0));
    } else if let Some(turn_id) = &convergence.running {
        session.adopt_turn(turn_id.clone());
        note.push_str(&format!(": turn {} is still running", turn_id.0));
    } else {
        note.push_str(": the kernel reports no turn for it");
    }

    // The transcript: only for a thread that has rendered nothing yet. A thread
    // with entries keeps them; the rows the kernel holds are *not* re-typed,
    // because the ones it already showed are indistinguishable from the ones it
    // missed and replaying both would double the conversation. The number is
    // reported so the note says what happened rather than apologising vaguely.
    let replayed: Option<bool> = cx.update(|cx| {
        thread
            .update(cx, |thread, cx| {
                if !thread.entries().is_empty() {
                    return false;
                }
                for update in hydrate_updates(&history) {
                    if let Err(error) = thread.handle_session_update(update, cx) {
                        log::warn!("[octos] the thread refused a resumed update: {error}");
                    }
                }
                true
            })
            .ok()
    });
    match replayed {
        Some(true) => note.push_str(&format!(
            "; the thread was empty, so the kernel's {} row(s) were replayed into it",
            convergence.messages_held
        )),
        Some(false) if convergence.messages_held > 0 => note.push_str(&format!(
            "; the kernel holds {} row(s), which were not re-typed into a thread that \
             already shows them",
            convergence.messages_held
        )),
        Some(false) => {}
        None => note.push_str("; the thread is gone, so nothing was replayed"),
    }

    // The approvals: `note_requested` is idempotent, so one the thread already
    // has a card for is not raised twice.
    let raised = convergence
        .approvals
        .iter()
        .filter(|event| raise_approval(thread.clone(), session, connection, event, cx))
        .count();
    if raised > 0 {
        note.push_str(&format!("; re-raised {raised} pending approval(s)"));
    }

    cx.update(|cx| super::push_note(cx, note));
    Ok(convergence)
}

// ---------------------------------------------------------------------------
// Approvals (pure ledger, one call)
// ---------------------------------------------------------------------------

/// What the person asked for on a card.
///
/// This is **intent**, not a decision: it is what the host sends the kernel with
/// `approval/respond`, and it never settles anything by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalIntent {
    Allow,
    Deny,
}

impl ApprovalIntent {
    /// The wire decision this intent asks the kernel for.
    fn decision(self) -> ApprovalDecision {
        match self {
            Self::Allow => ApprovalDecision::Approve,
            Self::Deny => ApprovalDecision::Deny,
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

/// How the kernel settled an approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalSettlement {
    Approved,
    Denied,
    /// The kernel cancelled it before it was answered — *not* a denial, which
    /// would be a decision nobody made.
    Cancelled,
}

/// The kernel's decision as a settlement, when this build understands it.
///
/// `ApprovalDecision` carries an `Unknown` arm for forward compatibility, and an
/// unknown decision is not a settlement: recording one would put a meaning on a
/// string the kernel has not defined.
pub fn settlement_of(decision: &ApprovalDecision) -> Option<ApprovalSettlement> {
    match decision {
        ApprovalDecision::Approve => Some(ApprovalSettlement::Approved),
        ApprovalDecision::Deny => Some(ApprovalSettlement::Denied),
        ApprovalDecision::Unknown(_) => None,
    }
}

/// One approval this thread has raised for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproval {
    pub approval_id: ApprovalId,
    /// The ACP tool call the card is drawn as: the kernel's `approval_id`, as
    /// the ACP backend uses it (`oup.rs:297-299`).
    pub tool_call_id: acp::ToolCallId,
    pub tool: String,
    /// What the person asked for, once they clicked. `None` while the card is
    /// untouched.
    pub intent: Option<ApprovalIntent>,
}

/// The approvals one thread has raised, and how far each has got.
///
/// This type exists to enforce one rule: **an intent is not a decision.** A click
/// records [`PendingApproval::intent`] and sends `approval/respond`; only the
/// kernel's own `approval/decided` / `approval/auto_resolved` /
/// `approval/cancelled` settles the entry ([`ApprovalLedger::settle`]). Until
/// then the entry stays pending, so a settled approval can no longer be answered
/// ([`ApprovalLedger::note_intent`]) and nothing claims the tool was authorized
/// on the strength of a click.
#[derive(Debug, Default)]
pub struct ApprovalLedger {
    pending: Vec<PendingApproval>,
    /// The settlements seen, so a late click cannot answer an approval the kernel
    /// already decided. Bounded: only the most recent are kept, because this
    /// exists to close a race, not as a ledger of record.
    settled: Vec<(ApprovalId, ApprovalSettlement)>,
}

/// How many settlements a thread remembers.
const SETTLED_APPROVALS_KEPT: usize = 64;

impl ApprovalLedger {
    /// Raise a card for `event`, unless this approval is already known.
    ///
    /// `false` means a card for it is already up (or it was already settled): a
    /// hydrate and a live frame can both carry the same request, and two cards
    /// for one approval would be two answers for one question.
    fn note_requested(&mut self, event: &ApprovalRequestedEvent) -> bool {
        if self.get(&event.approval_id).is_some()
            || self.settlement(&event.approval_id).is_some()
        {
            return false;
        }
        self.pending.push(PendingApproval {
            approval_id: event.approval_id.clone(),
            tool_call_id: acp::ToolCallId::new(event.approval_id.0.to_string()),
            tool: event.tool_name.clone(),
            intent: None,
        });
        true
    }

    /// Record what the person asked for, if the kernel has not already settled
    /// it. `Some` is the entry the caller may send `approval/respond` for;
    /// `None` means the kernel is no longer waiting and nothing may be sent.
    fn note_intent(
        &mut self,
        approval_id: &ApprovalId,
        intent: ApprovalIntent,
    ) -> Option<PendingApproval> {
        if self.settlement(approval_id).is_some() {
            return None;
        }
        let entry = self
            .pending
            .iter_mut()
            .find(|entry| &entry.approval_id == approval_id)?;
        // A click is not repeatable: the first one is the answer this host sends.
        if entry.intent.is_some() {
            return None;
        }
        entry.intent = Some(intent);
        Some(entry.clone())
    }

    /// The kernel's decision landed. Returns the card that was still waiting, so
    /// the caller can resolve it to what the kernel actually decided.
    fn settle(
        &mut self,
        approval_id: &ApprovalId,
        settlement: ApprovalSettlement,
    ) -> Option<PendingApproval> {
        self.settled
            .retain(|(id, _)| id != approval_id);
        self.settled.push((approval_id.clone(), settlement));
        if self.settled.len() > SETTLED_APPROVALS_KEPT {
            let excess = self.settled.len() - SETTLED_APPROVALS_KEPT;
            self.settled.drain(..excess);
        }
        let index = self
            .pending
            .iter()
            .position(|entry| &entry.approval_id == approval_id)?;
        Some(self.pending.remove(index))
    }

    /// Everything still waiting, for a session that is ending.
    fn abandon(&mut self) -> Vec<PendingApproval> {
        std::mem::take(&mut self.pending)
    }

    fn get(&self, approval_id: &ApprovalId) -> Option<&PendingApproval> {
        self.pending
            .iter()
            .find(|entry| &entry.approval_id == approval_id)
    }

    /// How the kernel settled this approval, if it has.
    fn settlement(&self, approval_id: &ApprovalId) -> Option<ApprovalSettlement> {
        self.settled
            .iter()
            .find(|(id, _)| id == approval_id)
            .map(|(_, settlement)| *settlement)
    }
}

impl ThreadSession {
    /// Raise `event` on this session's ledger: `false` when it is already known.
    fn note_approval(&self, event: &ApprovalRequestedEvent) -> bool {
        self.approvals
            .lock()
            .expect("octos approvals")
            .note_requested(event)
    }

    /// Claim the right to answer one approval with `intent`.
    ///
    /// `None` means the kernel already settled it (or this session never raised
    /// it), so no frame may be sent: the answer would be a second decision for a
    /// question that is no longer open.
    fn claim_approval(&self, approval_id: &ApprovalId, intent: ApprovalIntent) -> Option<PendingApproval> {
        self.approvals
            .lock()
            .expect("octos approvals")
            .note_intent(approval_id, intent)
    }

    /// Record the kernel's settlement, returning the card that was still waiting.
    fn settle_approval(
        &self,
        approval_id: &ApprovalId,
        settlement: ApprovalSettlement,
    ) -> Option<PendingApproval> {
        self.approvals
            .lock()
            .expect("octos approvals")
            .settle(approval_id, settlement)
    }

    /// Everything still waiting: a session that ends can answer no more.
    fn abandon_approvals(&self) -> Vec<PendingApproval> {
        let abandoned = self.approvals.lock().expect("octos approvals").abandon();
        for entry in &abandoned {
            log::warn!(
                "[octos] approval {} ({}) was still waiting when the session ended",
                entry.approval_id.0,
                entry.tool
            );
        }
        abandoned
    }
}

/// The two options a thread's approval card offers.
///
/// The ids and kinds are the ACP backend's own (`oup.rs:303-310`): `allow-once`
/// (`AllowOnce`) and `deny` (`RejectOnce`). No always-allow option is offered,
/// because the kernel has no scope for one on this frame and inventing one would
/// promise a standing authorization nobody granted.
pub fn approval_options() -> PermissionOptions {
    PermissionOptions::Flat(vec![
        acp::PermissionOption::new(
            APPROVAL_ALLOW_ONCE,
            "Allow once",
            acp::PermissionOptionKind::AllowOnce,
        ),
        acp::PermissionOption::new(APPROVAL_DENY, "Deny", acp::PermissionOptionKind::RejectOnce),
    ])
}

/// The tool-call card one approval is drawn as.
///
/// The same shape the ACP backend builds (`oup.rs:297-302`): the approval id as
/// the tool call id, the kernel's `title` as the card's title, its `body` as the
/// card's content.
pub fn approval_tool_call(event: &ApprovalRequestedEvent) -> acp::ToolCallUpdate {
    acp::ToolCallUpdate::new(
        acp::ToolCallId::new(event.approval_id.0.to_string()),
        acp::ToolCallUpdateFields::new()
            .title(event.title.clone())
            .content(vec![acp::ToolCallContent::from(acp::ContentBlock::from(
                event.body.clone(),
            ))]),
    )
}

/// `approval/respond` for one approval this thread raised.
///
/// Built from the thread's own record — its session key and the approval the
/// kernel raised for it — because the console's table
/// (`peers::HostPeer::approval_respond_params`) holds only the approvals the
/// *console* saw: one recovered from a hydrate was never one of those frames.
fn approval_respond_params(
    session: &SessionKey,
    approval: &PendingApproval,
    intent: ApprovalIntent,
) -> serde_json::Result<Value> {
    serde_json::to_value(oup::ApprovalRespondParams {
        session_id: session.clone(),
        approval_id: approval.approval_id.clone(),
        decision: intent.decision(),
        approval_scope: None,
        client_note: None,
    })
}

/// Raises one approval card on a thread and starts listening for the answer.
///
/// `false` when the card was not raised: this approval is already known to the
/// session (a hydrate and a live frame can both carry it), or the thread is gone.
fn raise_approval(
    thread: WeakEntity<AcpThread>,
    session: &Arc<ThreadSession>,
    connection: &Connection,
    event: &ApprovalRequestedEvent,
    cx: &AsyncApp,
) -> bool {
    if !session.note_approval(event) {
        log::debug!(
            "[octos] approval {} is already on this thread",
            event.approval_id.0
        );
        return false;
    }
    log::info!(
        "[octos] the kernel is waiting on approval {} ({}) for session {}",
        event.approval_id.0,
        event.tool_name,
        session.session.0
    );

    let authorization = cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.request_tool_call_authorization(
                approval_tool_call(event),
                approval_options(),
                AuthorizationKind::PermissionGrant,
                cx,
            )
        })
    });
    let outcome = match authorization {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) => {
            log::warn!(
                "[octos] the thread would not take approval {}: {error}",
                event.approval_id.0
            );
            return false;
        }
        Err(_) => return false,
    };

    let approval_id = event.approval_id.clone();
    let tool = event.tool_name.clone();
    let session = session.clone();
    let connection = connection.clone();
    cx.spawn(async move |cx| {
        let answer = match outcome.await {
            RequestPermissionOutcome::Selected(selected)
                if selected.option_id.0.as_ref() == APPROVAL_ALLOW_ONCE =>
            {
                Some(ApprovalIntent::Allow)
            }
            RequestPermissionOutcome::Selected(_) => Some(ApprovalIntent::Deny),
            // A dismissal is not a denial. Nothing is sent, and the approval
            // stays the kernel's: saying "denied" here would decide for the
            // person, and the kernel's own `approval/cancelled` is what will
            // actually close it.
            RequestPermissionOutcome::Cancelled
            | RequestPermissionOutcome::InterruptedByFollowUp => {
                log::info!(
                    "[octos] the approval card for {} was dismissed without an answer; \
                     the kernel is still waiting on it",
                    approval_id.0
                );
                cx.update(|cx| {
                    super::push_note(
                        cx,
                        format!(
                            "the approval for {tool} was dismissed without an answer: the \
                             kernel is still waiting on it (answer it on the Octos console, \
                             or it will be cancelled when the turn ends)"
                        ),
                    );
                });
                None
            }
        };
        let Some(intent) = answer else {
            return;
        };

        // The gate. The click is intent; this is where the host asks whether the
        // kernel is still waiting at all.
        if session.claim_approval(&approval_id, intent).is_none() {
            log::info!(
                "[octos] approval {} was already settled; not answering it again",
                approval_id.0
            );
            return;
        }

        // Taken out of the console's own table before the frame goes out, the way
        // `decide_approval` does (`super::decide_approval`): one approval is one
        // question, and a second card for it would invite a second answer.
        let from_console = cx.update(|cx| {
            super::peer(cx).and_then(|peer| {
                let params =
                    peer.approval_respond_params(&approval_id, intent == ApprovalIntent::Allow)?;
                peer.take_kernel_approval(&approval_id);
                Some(params)
            })
        });
        let params = match from_console.or_else(|| {
            session
                .approvals
                .lock()
                .expect("octos approvals")
                .get(&approval_id)
                .and_then(|entry| approval_respond_params(&session.session, entry, intent).ok())
        }) {
            Some(params) => params,
            None => {
                log::warn!(
                    "[octos] approval {} left the ledger before it could be answered",
                    approval_id.0
                );
                return;
            }
        };

        log::info!(
            "[octos] asking the kernel to {} approval {} ({tool})",
            intent.description(),
            approval_id.0
        );
        let reply = connection
            .call_with_timeout(
                oup::methods::APPROVAL_RESPOND,
                params,
                APPROVAL_RESPOND_TIMEOUT,
            )
            .await;
        cx.update(|cx| {
            let note = match reply {
                Ok(frame) => match frame.get("error") {
                    Some(error) => format!(
                        "the kernel refused the answer for {tool}: {error} \
                         (its decision is in Zed.log)"
                    ),
                    // The frame was accepted; the kernel's own decision is still
                    // its to make, so this says what was *asked*, not what was
                    // decided.
                    None => format!(
                        "asked the Octos kernel to {} {tool}; waiting for its decision",
                        intent.description()
                    ),
                },
                Err(failure) => format!(
                    "the answer for {tool} was not acknowledged: {failure} \
                     (the kernel may still decide it)"
                ),
            };
            log::info!("[octos] {note}");
            super::push_note(cx, note);
        });
    })
    .detach();
    true
}

/// Applies the kernel's settlement of one approval to its card.
///
/// The card is resolved to what the *kernel* decided, through `AcpThread`'s own
/// API — `authorize_tool_call` for a decision, `cancel_tool_call_authorization`
/// for a cancellation — which is also what tells the waiting responder that the
/// question is closed. It never sends `approval/respond`: the kernel has already
/// answered itself.
fn settle_approval(
    thread: WeakEntity<AcpThread>,
    session: &Arc<ThreadSession>,
    approval_id: &ApprovalId,
    settlement: ApprovalSettlement,
    cx: &AsyncApp,
) {
    let Some(card) = session.settle_approval(approval_id, settlement) else {
        log::debug!(
            "[octos] approval {} settled {} with no card waiting",
            approval_id.0,
            match settlement {
                ApprovalSettlement::Approved => "approved",
                ApprovalSettlement::Denied => "denied",
                ApprovalSettlement::Cancelled => "cancelled",
            }
        );
        return;
    };
    let tool = card.tool.clone();
    let id = card.tool_call_id.clone();
    log::info!(
        "[octos] the kernel settled approval {} ({tool}) as {}",
        approval_id.0,
        match settlement {
            ApprovalSettlement::Approved => "approved",
            ApprovalSettlement::Denied => "denied",
            ApprovalSettlement::Cancelled => "cancelled",
        }
    );
    cx.update(|cx| {
        thread
            .update(cx, |thread, cx| match settlement {
                ApprovalSettlement::Approved => thread.authorize_tool_call(
                    id,
                    SelectedPermissionOutcome::new(
                        acp::PermissionOptionId::new(APPROVAL_ALLOW_ONCE),
                        acp::PermissionOptionKind::AllowOnce,
                    ),
                    cx,
                ),
                ApprovalSettlement::Denied => thread.authorize_tool_call(
                    id,
                    SelectedPermissionOutcome::new(
                        acp::PermissionOptionId::new(APPROVAL_DENY),
                        acp::PermissionOptionKind::RejectOnce,
                    ),
                    cx,
                ),
                ApprovalSettlement::Cancelled => {
                    thread.cancel_tool_call_authorization(&id, cx)
                }
            })
            .ok();

        // What the person asked for, against what the kernel decided: a
        // disagreement is exactly what a person needs to be told about, because
        // the tool did not run the way they asked.
        let note = match (card.intent, settlement) {
            (Some(ApprovalIntent::Allow), ApprovalSettlement::Denied) => Some(format!(
                "the Octos kernel denied {tool} even though it was allowed here: \
                 its policy, not this window, decides"
            )),
            (Some(ApprovalIntent::Deny), ApprovalSettlement::Approved) => Some(format!(
                "the Octos kernel approved {tool} even though it was denied here; \
                 it will run"
            )),
            (Some(ApprovalIntent::Allow), ApprovalSettlement::Cancelled) => Some(format!(
                "the Octos kernel cancelled {tool} after it was allowed here"
            )),
            (Some(ApprovalIntent::Deny), ApprovalSettlement::Cancelled) => Some(format!(
                "the Octos kernel cancelled {tool} before the denial was answered"
            )),
            _ => None,
        };
        if let Some(note) = note {
            log::info!("[octos] {note}");
            super::push_note(cx, note);
        }
    });
}

// ---------------------------------------------------------------------------
// Encoders (pure)
// ---------------------------------------------------------------------------

/// The session key one agent thread opens.
///
/// The same main profile as the console, under `zed`, with a name that belongs
/// to exactly one thread: the kernel keys a session by this string, so a shared
/// name would make the second thread join the first one's conversation — and its
/// `turn/start` would be refused with `turn_in_progress` for as long as the
/// first thread's turn runs.
pub fn thread_session_key(thread: &str) -> SessionKey {
    SessionKey::with_profile(octos_core::MAIN_PROFILE_ID, super::APP_ID, thread)
}

/// A name no other thread in this process uses.
///
/// The process id separates two Zed instances writing into the same Octos home;
/// the counter separates threads inside one process. Nothing here is durable
/// across restarts — a restarted Zed opens new sessions, which is what P2
/// supports.
pub fn next_thread_name() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    format!(
        "thread-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// `turn/start` for one thread's turn, built through the OUP types so the wire
/// shape stays the kernel's.
///
/// `media` is left off the frame entirely (it serialises as `[]` only when
/// empty, and is skipped when empty): P2 sends text. An `@file` mention has
/// already been degraded to its path in `text` by [`prompt_text`]. Every other
/// optional field stays absent, which is what the kernel reads as "no override".
fn turn_start_params(
    session: &SessionKey,
    turn: &TurnId,
    text: &str,
) -> serde_json::Result<Value> {
    Ok(json!({
        "session_id": serde_json::to_value(session)?,
        "turn_id": serde_json::to_value(turn)?,
        "input": [serde_json::to_value(InputItem::Text {
            text: text.to_owned(),
        })?],
    }))
}

/// The text of one ACP prompt, as the kernel's `InputItem::Text` wants it.
///
/// **Every** text block is joined with a newline: a prompt built from several
/// blocks reaches the model whole instead of only its first block. Resource
/// blocks are degraded to their text; resource *links* (a Zed `@file` mention)
/// are degraded to a labelled reference to the path they name, because P2 has no
/// media channel — the file is referred to, not attached. Binary blocks (image,
/// audio) are skipped rather than mis-sent as text.
pub fn prompt_text(blocks: &[acp::ContentBlock]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in blocks {
        match block {
            acp::ContentBlock::Text(text) => parts.push(text.text.clone()),
            acp::ContentBlock::Resource(resource) => {
                if let acp::EmbeddedResourceResource::TextResourceContents(text) =
                    &resource.resource
                {
                    parts.push(text.text.clone());
                }
            }
            acp::ContentBlock::ResourceLink(link) => {
                let label = link.title.as_deref().unwrap_or(&link.name);
                let mut part = format!("[Resource: {label} ({})]", link.uri);
                if let Some(description) = &link.description {
                    part.push('\n');
                    part.push_str(description);
                }
                parts.push(part);
            }
            // `ContentBlock` is `#[non_exhaustive]`: a catch-all is required, and
            // an unknown block is skipped rather than guessed at.
            _ => {}
        }
    }
    parts.join("\n")
}

/// `session/title.set` for one thread's session.
///
/// The title is clamped to the kernel's own cap
/// ([`oup::SESSION_TITLE_SET_MAX_CHARS`]) **by characters**, not bytes: sending
/// more would be refused outright, and a byte-truncated UTF-8 string would not
/// be the title the person typed. `session_id` is the session key's wire string,
/// which is what this RPC's own type asks for.
fn title_set_params(session: &SessionKey, title: &str) -> serde_json::Result<Value> {
    let clamped: String = title.chars().take(oup::SESSION_TITLE_SET_MAX_CHARS).collect();
    if clamped != title {
        log::info!(
            "[octos] truncating a title of {} character(s) to the kernel's cap of {}",
            title.chars().count(),
            oup::SESSION_TITLE_SET_MAX_CHARS
        );
    }
    serde_json::to_value(oup::SessionTitleSetParams {
        session_id: session.0.clone(),
        title: clamped,
    })
}

/// The session key a `session/open` reply opened.
///
/// Decoded with `SessionOpenResult` — the kernel's own type — rather than by
/// reading `result["opened"]["session_id"]` by hand, so the reply cannot drift
/// from this host silently.
fn session_open_result(frame: &Value) -> Result<SessionKey, String> {
    if let Some(error) = frame.get("error") {
        return Err(format!("the kernel refused it: {error}"));
    }
    let Some(result) = frame.get("result") else {
        return Err(format!("the reply carried no result: {frame}"));
    };
    let decoded: oup::SessionOpenResult = serde_json::from_value(result.clone())
        .map_err(|error| format!("the reply was not a SessionOpenResult: {error}"))?;
    Ok(decoded.opened.session_id)
}

/// What the kernel's reply to `turn/start` says, when it refused the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnRefusal {
    /// A turn is already running on this session, and the kernel named it with
    /// a typed `turn_in_progress` error (`turn_in_progress_refusal`,
    /// `crates/octos/octos-cli/src/api/ui_protocol_transport.rs:27653`).
    TurnInProgress { turn_id: TurnId, message: String },
    /// Any other refusal: the kernel named no turn, so nothing of this host's is
    /// in flight.
    Refused { message: String },
}

impl TurnRefusal {
    /// The reason to show the person.
    fn message(&self) -> String {
        match self {
            Self::TurnInProgress { message, .. } | Self::Refused { message } => message.clone(),
        }
    }
}

/// Reads a `turn/start` error object.
///
/// The `data.kind`/`data.turn_id` shape is transport-local — `turn/steer` and
/// its refusal are not core types (`APPUI_METHOD_TURN_STEER`,
/// `crates/octos/octos-cli/src/api/ui_protocol_transport.rs:459`) — so the two
/// fields are read by name here, and the id is decoded with `TurnId`'s own
/// `Deserialize` rather than parsed by hand.
pub fn turn_refusal(error: &Value) -> TurnRefusal {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| error.to_string());
    let named = error
        .get("data")
        .filter(|data| data.get("kind").and_then(Value::as_str) == Some("turn_in_progress"))
        .and_then(|data| data.get("turn_id"))
        .and_then(|turn_id| serde_json::from_value::<TurnId>(turn_id.clone()).ok());
    match named {
        Some(turn_id) => TurnRefusal::TurnInProgress { turn_id, message },
        None => TurnRefusal::Refused { message },
    }
}

/// The session a kernel frame belongs to, as the kernel's own decoder reads it.
///
/// `None` for a frame this build cannot decode: an unattributable frame must not
/// be rendered into *every* thread.
fn frame_session(frame: &OctosFrame) -> Option<SessionKey> {
    UiNotification::from_method_and_params(&frame.method, frame.params.clone())
        .ok()
        .map(|notification| notification.session_id().clone())
}

// Interrupt receipts.
//
// `turn/interrupt`'s reply is `TurnInterruptResult`
// (`crates/octos/octos-core/src/ui_protocol.rs:4795`), and each of its four
// outcomes means something different to a person. The reading and the enum are
// the console's own ([`super::interrupt_outcome`] / [`super::InterruptOutcome`]),
// reused rather than re-derived: an interrupt that the console would refuse to
// call "cancelled" must not become one here either.
use super::{InterruptOutcome, interrupt_outcome};

#[cfg(test)]
#[path = "agent_thread_tests.rs"]
mod tests;
