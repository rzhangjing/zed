//! The seam between [`crate::Agent::Octos`] and the in-process Octos kernel.
//!
//! Octos runs **inside this process** (see `crate::zed::octos` in the `zed`
//! binary crate): there is no `octos acp` child process to spawn and no
//! `agent_servers` entry in settings to point at. That breaks the usual shape of
//! an `AgentServer` — [`crate::Agent::server`] builds one from nothing but an
//! `AgentId`, and the thing that actually knows how to talk to Octos lives in a
//! crate that `agent_ui` cannot depend on (the app binary depends on `agent_ui`,
//! not the other way round).
//!
//! So the dependency is inverted by one [`Global`]: the app binary registers an
//! [`OctosBackend`] at startup, and [`OctosAgentServer`] — the trivial
//! `AgentServer` behind `Agent::Octos` — hands every `connect` to it. The server
//! itself is deliberately thin: all protocol work (OUP frames, session keys,
//! turn state) stays in the host crate that owns the kernel connection.
//!
//! When nothing is registered (a build without the kernel, or a test app),
//! `connect` fails with a readable reason rather than pretending to work.

use std::rc::Rc;

use acp_thread::AgentConnection;
use agent_servers::{AgentServer, AgentServerDelegate};
use anyhow::{Result, anyhow};
use gpui::{App, Entity, Global, Task};
use project::{AgentId, Project};
use ui::IconName;

/// The `AgentId` in-process Octos is selected under.
///
/// This is a *real* `AgentId`, so every existing `Agent::Custom`-shaped path
/// (thread metadata, draft prompts, the sidebar) already knows how to persist
/// and restore a thread bound to it.
pub const OCTOS_AGENT_ID: &str = "octos";

/// The label a person sees for [`OCTOS_AGENT_ID`].
pub const OCTOS_AGENT_LABEL: &str = "Octos";

/// What the host process provides when Octos is embedded in it.
///
/// Implemented by the `zed` binary crate over its own kernel connection. The
/// returned connection opens one OUP session per agent thread; this seam makes
/// no promise about *how* — that is the host's business, and Octos owns the
/// protocol.
pub trait OctosBackend: 'static {
    /// A connection over the kernel this process already has running.
    ///
    /// `Err` means the kernel is embedded but cannot be used (a kernel that
    /// failed to start, for instance) — never "try a subprocess instead".
    fn connect(&self, project: Entity<Project>, cx: &mut App) -> Result<Rc<dyn AgentConnection>>;
}

/// The registered backend, if this process has one.
#[derive(Default)]
pub struct OctosBackendHandle(Option<Rc<dyn OctosBackend>>);

impl Global for OctosBackendHandle {}

/// Publishes `backend` as this process's Octos seam.
///
/// Called once, from the app's Octos init, after the kernel has been started —
/// registering before the kernel exists would advertise a backend whose
/// sessions can only fail.
pub fn register(cx: &mut App, backend: Rc<dyn OctosBackend>) {
    match cx.try_global::<OctosBackendHandle>() {
        Some(_) => cx.global_mut::<OctosBackendHandle>().0 = Some(backend),
        None => cx.set_global(OctosBackendHandle(Some(backend))),
    }
}

/// Withdraws the backend, if any: the kernel is no longer usable, so no new
/// thread may be opened over it. Threads that are already open keep their
/// connection (and fail their calls, which is the honest outcome).
pub fn unregister(cx: &mut App) {
    if cx.try_global::<OctosBackendHandle>().is_some() {
        cx.global_mut::<OctosBackendHandle>().0 = None;
    }
}

/// The registered backend, if this process has one.
pub fn backend(cx: &App) -> Option<Rc<dyn OctosBackend>> {
    cx.try_global::<OctosBackendHandle>()?.0.clone()
}

/// Whether `Agent::Octos` should be offered in the UI at all.
///
/// False in a process with no embedded kernel — the picker then shows no Octos
/// entry, which is better than an entry whose every thread fails to connect.
pub fn is_available(cx: &App) -> bool {
    backend(cx).is_some()
}

/// The `AgentServer` behind [`crate::Agent::Octos`].
pub struct OctosAgentServer;

impl AgentServer for OctosAgentServer {
    fn logo(&self) -> IconName {
        IconName::Sparkle
    }

    fn agent_id(&self) -> AgentId {
        AgentId::new(OCTOS_AGENT_ID)
    }

    fn connect(
        &self,
        _delegate: AgentServerDelegate,
        project: Entity<Project>,
        cx: &mut App,
    ) -> Task<Result<Rc<dyn AgentConnection>>> {
        // Nothing to await: the kernel is already running in this process, so
        // "connecting" is handing out a handle to it. A future that needs no
        // waiting is still a future, because that is the trait.
        let result = match backend(cx) {
            Some(backend) => backend.connect(project, cx),
            None => Err(anyhow!(
                "Octos is not running in this process, so there is no kernel to open a \
                 session on (the kernel is embedded, not spawned — see `ZED_OCTOS_AUTOSTART`)"
            )),
        };
        Task::ready(result)
    }

    fn into_any(self: Rc<Self>) -> Rc<dyn std::any::Any> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    struct TestBackend;

    impl OctosBackend for TestBackend {
        fn connect(
            &self,
            _project: Entity<Project>,
            _cx: &mut App,
        ) -> Result<Rc<dyn AgentConnection>> {
            Err(anyhow!("a test backend cannot open a session"))
        }
    }

    /// The picker's gate: no backend, no Octos entry.
    #[gpui::test]
    fn availability_follows_registration(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assert!(!is_available(cx), "nothing is registered yet");
            assert!(backend(cx).is_none());

            register(cx, Rc::new(TestBackend));
            assert!(is_available(cx), "a registered backend is available");

            unregister(cx);
            assert!(!is_available(cx), "a withdrawn backend is not");
            // Registering twice must not panic: it replaces.
            register(cx, Rc::new(TestBackend));
            register(cx, Rc::new(TestBackend));
            assert!(is_available(cx));
        });
    }

    /// The `AgentServer` here is the identity of the Octos agent: the id a
    /// thread is keyed by and the id the connection reports must be the same
    /// string, or the panel's connection store would key two entries for one
    /// agent.
    #[test]
    fn the_server_names_the_octos_agent() {
        let server = OctosAgentServer;
        assert_eq!(server.agent_id(), AgentId::new(OCTOS_AGENT_ID));
        assert_eq!(OCTOS_AGENT_ID, crate::octos::OCTOS_AGENT_ID);
        assert_eq!(crate::Agent::Octos.id(), AgentId::new(OCTOS_AGENT_ID));
    }
}
