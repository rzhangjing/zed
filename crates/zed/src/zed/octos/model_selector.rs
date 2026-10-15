//! The Octos agent thread's model picker: `profile/llm/list` and
//! `profile/llm/select` over the same in-process OUP connection a thread's
//! turns use.
//!
//! Three boundaries this module exists to keep:
//!
//! * **The list is the kernel's.** It is the models configured on the *profile*,
//!   one row per (family, model, route). Zed's own model registry is not mirrored
//!   into it, and Zed's models are never written into Octos as fallbacks: Octos's
//!   fallback list *is* its failover chain, and filling it with Zed's catalog
//!   would turn a failover policy into a model directory.
//! * **A selection promotes an already-configured model.** The kernel refuses a
//!   model that is not configured on the profile
//!   (`raw_profile_llm_select`, `crates/octos/octos-cli/src/api/ui_protocol_transport.rs:13988-14015`),
//!   and this host reports that refusal verbatim instead of pretending the switch
//!   happened.
//! * **A switch is never claimed to be immediate.** The kernel stamps *when* a
//!   committed change takes effect (`runtime_disposition` /`restart_required` /
//!   `effective_from`, `:14100-14113` and `:14886-14963`), and [`select_note`]
//!   turns that into the one line the person reads. Nothing here re-opens a
//!   session to make a switch *look* live — the note says what the kernel said.
//!
//! Favourites stay Zed's: `favorite_model_ids` and `toggle_favorite_model` keep
//! the trait's defaults, because Zed's favourite-model setting is Zed's own list
//! and not a property of the Octos profile.
//!
//! **Wire shapes.** `profile/llm/list` and `profile/llm/select` have no
//! `octos-core` type: `crates/octos/octos-core/src/ui_protocol.rs` does not
//! contain the string `profile/llm` at all, and the AppUI transport keeps both
//! the params (`RawProfileParams:9588`, `RawProfileLlmSelectParams:9798`) and
//! the bodies (`configured_model_status_json:11991`, `model_list_result:12009`,
//! `profile_llm_list_result:11957`) private to itself. So the keys below are read
//! by name, each next to the line that emits it. Nothing is invented: a field the
//! kernel does not send is simply absent here.

use std::time::Duration;

use acp_thread::{AgentModelId, AgentModelInfo, AgentModelList, AgentModelSelector};
use anyhow::{Result, anyhow};
use gpui::{App, AsyncApp, Task};
use language_model::{ApiKey, DisabledReason};
use octos_core::SessionKey;
use serde_json::{Map, Value, json};

use super::connection::Connection;

/// `profile/llm/list` — a private const in the AppUI transport
/// (`crates/octos/octos-cli/src/api/ui_protocol_transport.rs:363`) with no
/// `octos-core` type, so the spelling lives here. Also used by
/// [`super::key_bridge`], which needs the same method's *profile* shape.
pub(super) const PROFILE_LLM_LIST: &str = "profile/llm/list";
/// `profile/llm/select` — the same transport, `:364`.
const PROFILE_LLM_SELECT: &str = "profile/llm/select";

/// How long the picker waits for the kernel to list or switch models.
///
/// The host's patience, not the kernel's deadline: a timeout is reported as "no
/// answer" and never as "the model was switched".
const PROFILE_LLM_TIMEOUT: Duration = Duration::from_secs(30);

/// The reason shown for a row the kernel reports as unavailable.
///
/// Deliberately generic: `available: false` is the kernel saying "the runtime
/// cannot activate this one", and *why* is the kernel's to say. This host does
/// not guess at a missing key or a bad endpoint.
const UNAVAILABLE: &str = "the Octos kernel reports this model as unavailable on this profile";

/// One row of a `profile/llm/list` reply, as `model_list_result` shapes it
/// (`crates/octos/octos-cli/src/api/ui_protocol_transport.rs:12009-12038`).
///
/// The keys are named in each field's doc with the line of
/// `configured_model_status_json` (`:11991-12007`) that emits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ModelRow {
    /// `family` (`:12000`) — the kernel's provider family for this model.
    pub(super) family: Option<String>,
    /// `model` (`:11993`) — the kernel's model id. A row without one is not
    /// offered: a model the kernel did not name is a model it never listed.
    pub(super) model: String,
    /// `title` (`:11995-11999`) — the kernel's own display string.
    pub(super) title: Option<String>,
    /// `route` (`:12001`) — the route id. The same (family, model) pair can be
    /// configured on two routes with different endpoints and credentials, so this
    /// is part of the row's address, not decoration.
    pub(super) route: Option<String>,
    /// `selected` (`:12002`) — this row is the profile's active primary.
    pub(super) selected: bool,
    /// `available` (`:12003`) — the kernel says the runtime can activate it. The
    /// kernel emits `true` unconditionally today; `false` is read as "not
    /// selectable" rather than ignored, and a **missing** flag is not read as
    /// false (absent ≠ false).
    pub(super) available: bool,
}

impl ModelRow {
    /// The row the kernel sent, or `None` when it names no model.
    fn from_json(row: &Value) -> Option<Self> {
        let model = text(row, "model")?;
        Some(Self {
            family: text(row, "family"),
            model,
            title: text(row, "title"),
            route: text(row, "route"),
            selected: row.get("selected").and_then(Value::as_bool).unwrap_or(false),
            available: row
                .get("available")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        })
    }

    /// The picker's id for this row: `family/model`, with `#route` appended when
    /// the kernel named a route.
    ///
    /// The address is exactly what the kernel itself compares when it decides
    /// which configured entry a request addresses
    /// (`same_llm_selection_address`, `:14209-14212`), so two rows that differ
    /// only by route are two entries here too. Nothing is *encoded* that a
    /// selection would then have to decode: [`select_row`] re-reads the kernel's
    /// list and sends the matched row's own `family`/`model`/`route` strings.
    fn id(&self) -> AgentModelId {
        let mut id = self.address();
        if let Some(route) = &self.route {
            id.push('#');
            id.push_str(route);
        }
        AgentModelId::new(id)
    }

    /// `family/model` — the row's route-independent address.
    pub(super) fn address(&self) -> String {
        match &self.family {
            Some(family) => format!("{family}/{}", self.model),
            None => self.model.clone(),
        }
    }

    /// The row as the picker's model info.
    fn info(&self) -> AgentModelInfo {
        AgentModelInfo {
            id: self.id(),
            name: self.title.clone().unwrap_or_else(|| self.address()).into(),
            // The kernel's route id is real, discriminating data and the only
            // thing a person can use to tell two same-named rows apart, so it is
            // shown rather than dropped.
            description: self
                .route
                .as_ref()
                .map(|route| format!("route {route}").into()),
            // No icon, no "latest" badge and no price: the profile's list carries
            // none of those, and a picker that invents them is lying about what
            // the kernel knows.
            icon: None,
            is_latest: false,
            cost: None,
            disabled: (!self.available).then(|| DisabledReason::new(UNAVAILABLE)),
        }
    }
}

/// A non-empty string field of a kernel body, trimmed. `null` and `""` are both
/// "the kernel did not set this".
fn text(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// The rows of a `profile/llm/list` **result** (`model_list_result`, `:12009`),
/// in the kernel's order.
///
/// A row without a model id is skipped and logged, the way
/// `agent_thread::listed_sessions` skips a session row without an id: an entry
/// with an invented id is a model the person could pick and the kernel has never
/// heard of.
fn model_rows(result: &Value) -> Vec<ModelRow> {
    let Some(rows) = result.get("models").and_then(Value::as_array) else {
        log::warn!(
            "[octos] profile/llm/list returned no `models` array; offering no model \
             (the reply was: {result})"
        );
        return Vec::new();
    };
    let mut models = Vec::with_capacity(rows.len());
    for row in rows {
        match ModelRow::from_json(row) {
            Some(model) => models.push(model),
            None => log::warn!("[octos] a profile/llm/list row names no model: {row}"),
        }
    }
    models
}

/// The picker's list from a `profile/llm/list` result.
///
/// A profile with nothing configured is an error with a next step in it, not an
/// empty picker: the person needs to know that Octos has no model yet, and
/// `AgentModelList::Flat(vec![])` cannot say that.
fn models_from_result(result: &Value) -> Result<AgentModelList> {
    let rows = model_rows(result);
    if rows.is_empty() {
        return Err(anyhow!(
            "the Octos profile has no model configured (profile/llm/list listed none); \
             configure one on the profile first (`octos init`, or the profile's provider \
             step), then reopen this picker"
        ));
    }
    Ok(AgentModelList::Flat(rows.iter().map(ModelRow::info).collect()))
}

/// The row the kernel reports as the profile's active primary (`selected`,
/// `:12002`).
fn active_model_from_result(result: &Value) -> Result<AgentModelInfo> {
    model_rows(result)
        .iter()
        .find(|row| row.selected)
        .map(ModelRow::info)
        .ok_or_else(|| {
            anyhow!("the Octos kernel listed no selected model for this profile's session")
        })
}

/// The kernel's own words for a refused request, without the rest of the error
/// body.
///
/// `message` is what the kernel wrote for a person ("… is not configured on
/// profile 'main'", `:13988-13996`); `code` is the JSON-RPC code. The `data`
/// object is deliberately not rendered: it exists for programmatic handling (the
/// auth-scope violation, for one) and adds nothing to what is shown.
fn refusal(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("the kernel gave no message");
    match error.get("code").and_then(Value::as_i64) {
        Some(code) => format!("{message} (code {code})"),
        None => message.to_owned(),
    }
}

/// The `result` of a reply, or the kernel's refusal as an error.
///
/// Shared with [`super::key_bridge`]: one place decides what "the kernel said no"
/// looks like, so a refusal can never be mistaken for a body somewhere else.
pub(super) fn result_of(reply: &Value, what: &str) -> Result<Value> {
    if let Some(error) = reply.get("error") {
        return Err(anyhow!("{what}: {}", refusal(error)));
    }
    Ok(reply.get("result").cloned().unwrap_or(Value::Null))
}

/// What the kernel's `profile/llm/select` receipt says about **when** the switch
/// takes effect — the one thing this host must not round up to "now".
///
/// The kernel stamps the disposition itself
/// (`commit_profile_llm_runtime_transition`, `:14886-14948`;
/// `stamp_profile_llm_runtime_transition`, `:14953-14963`;
/// `ProfileLlmRuntimeTransition::wire_status`, `:14859-14869`):
///
/// * `reloaded` — the caches were evicted and the runtime rebuilt.
/// * `deferred` — nothing was cached to rebuild yet.
/// * `restart_required` (`restart_required: true`, also set for a startup-pinned
///   profile at `:14106-14113`) — the profile's runtime is pinned to what the
///   kernel booted with: the selection is **saved**, and the kernel keeps serving
///   the previous model.
/// * `persisted_but_not_live` — saved, but the rebuild failed; `runtime_error`
///   says why and the next turn retries it.
/// * `unchanged` — nothing was persisted (the row was already the primary).
///
/// `effective_from: "next_turn"` accompanies every disposition but `unchanged`
/// (`:14865-14866`), so "next turn" is the kernel's word and not this host's
/// guess. For a pinned profile the note says the switch is *not* live even for a
/// new thread, because that is what the kernel reported — a note that promised
/// otherwise would be the host inventing a behaviour.
fn select_note(result: &Value) -> String {
    let model = result
        .get("selected")
        .and_then(|selected| {
            ["title", "model"]
                .into_iter()
                .find_map(|key| text(selected, key))
        })
        .unwrap_or_else(|| "the model".to_owned());
    let disposition = result
        .get("runtime_disposition")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let applied = result.get("applied").and_then(Value::as_bool);
    let restart_required = result
        .get("restart_required")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if restart_required {
        return format!(
            "{model} {} this profile's primary, but this Octos profile's runtime is \
             pinned to what the kernel booted with: it keeps serving the previous model \
             until the kernel restarts — a new thread does not change that. \
             (kernel disposition: {disposition})",
            if applied == Some(false) {
                "is already saved as"
            } else {
                "was saved as"
            }
        );
    }

    match disposition {
        "reloaded" => format!(
            "{model} is the profile's primary; the kernel rebuilt the profile's runtime, \
             so the next turn uses it"
        ),
        "deferred" => format!(
            "{model} is the profile's primary; the kernel had no cached runtime to \
             rebuild, so the next session/turn that builds one uses it"
        ),
        "persisted_but_not_live" => format!(
            "{model} was saved as the profile's primary, but the kernel could not rebuild \
             the profile runtime ({}); the next turn retries it",
            text(result, "runtime_error").unwrap_or_else(|| "no reason given".to_owned())
        ),
        "unchanged" => {
            format!("{model} is already the profile's primary; nothing changed")
        }
        other => format!("the kernel reported the selection as '{other}'"),
    }
}

/// `profile/llm/list` params for one thread's session.
///
/// `session_id` is enough: the kernel resolves the profile out of the session key
/// (`raw_profile_id`, `:10398-10408`) and, *with* a session, answers the per-model
/// status rows this picker reads (`model_list_result`, `:12009`) rather than the
/// profile document (`profile_llm_list_result`, `:11957`). No `profile_id` is
/// sent, because the session already names it and sending both would give two
/// sources for one fact.
fn list_params(session: &SessionKey) -> Value {
    json!({ "session_id": session })
}

/// `profile/llm/select` params for one row (`RawProfileLlmSelectParams`,
/// `:9798-9811`; handler `raw_profile_llm_select`, `:13923`).
///
/// `route_id` is sent **whenever the kernel listed one**. The handler treats the
/// route as a real discriminator — the same family/model can be configured on two
/// routes with different endpoints and credentials — and refuses an ambiguous
/// family/model match with "configured on multiple routes — pass the route_id"
/// (`:13998-14004`). The route string is the kernel's own, taken from the row.
fn select_params(session: &SessionKey, row: &ModelRow) -> Value {
    let mut params = Map::new();
    params.insert("session_id".to_owned(), json!(session));
    params.insert("model_id".to_owned(), json!(row.model));
    if let Some(family) = &row.family {
        params.insert("family_id".to_owned(), json!(family));
    }
    if let Some(route) = &row.route {
        params.insert("route_id".to_owned(), json!(route));
    }
    Value::Object(params)
}

/// One reply to a request this host sent, or "no answer".
async fn call(connection: &Connection, method: &str, params: Value) -> Result<Value> {
    connection
        .call_with_timeout(method, params, PROFILE_LLM_TIMEOUT)
        .await
        .map_err(|failure| anyhow!("the Octos kernel did not answer {method}: {failure}"))
}

/// The kernel's rows for one thread's session.
async fn rows(connection: &Connection, session: &SessionKey) -> Result<Vec<ModelRow>> {
    let reply = call(connection, PROFILE_LLM_LIST, list_params(session)).await?;
    let result = result_of(&reply, "the Octos kernel refused profile/llm/list")?;
    Ok(model_rows(&result))
}

/// The picker's list for one thread's session.
async fn models(connection: &Connection, session: &SessionKey) -> Result<AgentModelList> {
    let reply = call(connection, PROFILE_LLM_LIST, list_params(session)).await?;
    let result = result_of(&reply, "the Octos kernel refused profile/llm/list")?;
    models_from_result(&result)
}

/// The active primary for one thread's session.
async fn selected(connection: &Connection, session: &SessionKey) -> Result<AgentModelInfo> {
    let reply = call(connection, PROFILE_LLM_LIST, list_params(session)).await?;
    let result = result_of(&reply, "the Octos kernel refused profile/llm/list")?;
    active_model_from_result(&result)
}

/// The row a picker id names, re-read from the kernel.
///
/// The id is matched against a *fresh* list rather than decoded: the address a
/// selection must carry (`family`, `model`, `route`) is then the kernel's own
/// string for a row the kernel is currently listing, which is also what makes a
/// removed model a readable error instead of a frame about a model the profile no
/// longer has.
async fn row_for(
    connection: &Connection,
    session: &SessionKey,
    model_id: &AgentModelId,
) -> Result<ModelRow> {
    let rows = rows(connection, session).await?;
    rows.into_iter().find(|row| row.id() == *model_id).ok_or_else(|| {
        anyhow!(
            "the Octos kernel does not list a model with the id {model_id} on this session's \
             profile; the profile's configured models changed since the picker was opened"
        )
    })
}

/// Sends one selection and returns the note its receipt earned.
///
/// The wire half, kept separate from the trait's `select_model` so it can be
/// tested against a fake kernel; the credential bridge happens in the caller.
async fn select_row(
    connection: &Connection,
    session: &SessionKey,
    row: &ModelRow,
) -> Result<String> {
    let reply = call(
        connection,
        PROFILE_LLM_SELECT,
        select_params(session, row),
    )
    .await?;
    let result = result_of(&reply, "the Octos kernel refused the model selection")?;
    let note = select_note(&result);
    log::info!("[octos] model selection on {session}: {note}");
    Ok(note)
}

/// Zed's own credential for `url`, if this Zed has one.
///
/// This is the same lookup Zed's language-model providers use
/// (`ApiKey::load_from_system_keychain`, `crates/language_model/src/api_key.rs:240`),
/// through the process's own [`CredentialsProvider`]. It is **not** bypassed
/// anywhere: in a dev build the provider is `DevelopmentCredentialsProvider`, a
/// plaintext file in the config dir (`zed_credentials_provider.rs:110-140`); in a
/// release build it is the system keychain, and the OS may put up its own prompt.
/// A refusal — or an empty key — means "this Zed has nothing to copy", never
/// "copy anyway".
///
/// The returned key is never logged, never put in a note and never in an error:
/// the only place it goes is the frame the bridge sends.
async fn read_zed_key(url: &str, cx: &mut AsyncApp) -> Option<String> {
    let credentials = cx.update(|cx| zed_credentials_provider::global(cx));
    match ApiKey::load_from_system_keychain(url, credentials.as_ref(), cx).await {
        Ok(key) => {
            let key = key.key().trim().to_owned();
            (!key.is_empty()).then_some(key)
        }
        Err(error) => {
            // `AuthenticateError`'s own Display: "credentials not found", or the
            // provider's message. No key is involved (there is none).
            log::info!(
                "[octos] no Zed credential could be read for {url} to offer the Octos \
                 profile: {error}"
            );
            None
        }
    }
}

/// The credential bridge for one row, if it applies.
///
/// Deciding happens against the kernel's own profile document
/// (`profile/llm/list` **without** a session: only that shape carries
/// `api_key_env`/`has_api_key`/`base_url`, `configured_provider_json:11270-11326`),
/// so this host never has to guess whether Octos already has a key. See
/// [`super::key_bridge`] for where a copied key lands.
///
/// The copy runs *before* the selection: the person clicking a model is asking to
/// use it, and a profile that cannot activate it is refused by the kernel. Doing
/// it the other way round would mean matching on the kernel's error text, which is
/// exactly the kind of guess this host does not make.
async fn bridge_row(
    connection: &Connection,
    row: &ModelRow,
    cx: &mut AsyncApp,
) -> Result<Option<String>> {
    let candidates = super::key_bridge::profile_candidates(connection).await?;
    let Some(candidate) = super::key_bridge::candidate_for(&candidates, row) else {
        return Ok(None);
    };
    let zed_key = match candidate.base_url.as_deref() {
        Some(url) => read_zed_key(url, cx).await,
        None => None,
    };
    match super::key_bridge::decide_key_bridge(
        candidate.api_key_env.as_deref(),
        candidate.has_api_key,
        zed_key.is_some(),
    ) {
        super::key_bridge::KeyBridgeDecision::NeedsCopy => {
            let key = zed_key.expect("NeedsCopy is only decided when Zed holds one");
            let note = super::key_bridge::copy(
                connection,
                &candidate,
                &key,
                super::key_bridge::copy_destination(
                    octos_cli::auth::keychain::is_available(),
                ),
            )
            .await?;
            Ok(Some(note))
        }
        super::key_bridge::KeyBridgeDecision::AlreadyPresent
        | super::key_bridge::KeyBridgeDecision::ZedHasNone
        | super::key_bridge::KeyBridgeDecision::Skip => Ok(None),
    }
}

/// One Octos thread's model picker.
///
/// Holds the thread's kernel session rather than a cached list: every call
/// re-reads `profile/llm/list`, so what the picker shows is the profile as it is
/// now and a stale row can never become a selection.
pub(super) struct OctosModelSelector {
    connection: Connection,
    session: SessionKey,
}

impl OctosModelSelector {
    pub(super) fn new(connection: Connection, session: SessionKey) -> Self {
        Self {
            connection,
            session,
        }
    }
}

impl AgentModelSelector for OctosModelSelector {
    fn list_models(&self, cx: &mut App) -> Task<Result<AgentModelList>> {
        let connection = self.connection.clone();
        let session = self.session.clone();
        cx.spawn(async move |_cx| models(&connection, &session).await)
    }

    fn select_model(&self, model_id: AgentModelId, cx: &mut App) -> Task<Result<()>> {
        let connection = self.connection.clone();
        let session = self.session.clone();
        cx.spawn(async move |cx| {
            let row = row_for(&connection, &session, &model_id).await?;
            if let Some(note) = bridge_row(&connection, &row, cx).await? {
                cx.update(|cx| super::push_note(cx, note));
            }
            let note = select_row(&connection, &session, &row).await?;
            cx.update(|cx| super::push_note(cx, note));
            Ok(())
        })
    }

    fn selected_model(&self, cx: &mut App) -> Task<Result<AgentModelInfo>> {
        let connection = self.connection.clone();
        let session = self.session.clone();
        cx.spawn(async move |_cx| selected(&connection, &session).await)
    }

    /// No footer.
    ///
    /// The picker's footer is a single "Configure" button that dispatches Zed's
    /// own `agent::OpenSettings` (`agent_ui/src/model_selector.rs:373-385`,
    /// `ui/model_selector_components.rs:230-252`) — it configures **Zed's**
    /// models. An Octos profile is not configured there, so rendering it would
    /// send a person who wants to change their Octos models to the wrong settings
    /// page. The honest place for that is the Octos profile itself.
    fn should_render_footer(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

    /// One `profile/llm/list` **result**, in the shape `model_list_result`
    /// emits (`crates/octos/octos-cli/src/api/ui_protocol_transport.rs:12037`).
    /// Hand-typed on purpose: what is under test is this host's reading of the
    /// keys the kernel writes, so the fixture spells them out.
    fn listed(rows: Value) -> Value {
        json!({ "session_id": "_main:zed:t", "models": rows })
    }

    fn row_json(model: &str, family: &str, selected: bool) -> Value {
        json!({
            "model": model,
            "provider": family,
            "title": format!("{family} / {model}"),
            "family": family,
            "route": "official",
            "selected": selected,
            "available": true,
            "queue_mode": "adaptive",
            "qoe_policy": "profile",
        })
    }

    fn session() -> SessionKey {
        SessionKey::with_profile(octos_core::MAIN_PROFILE_ID, "zed", "t")
    }

    /// What a fake kernel answers with: the `result` body, or an `error` body —
    /// the shape a refused request really has (`ui_protocol_transport.rs:13990`).
    #[derive(Clone)]
    enum Reply {
        Ok(Value),
        Err(Value),
    }

    /// A fake kernel that answers `profile/llm/list` and `profile/llm/select`
    /// with canned replies, recording the params it was asked with.
    async fn fake_kernel(
        list: Reply,
        select: Reply,
    ) -> (
        Connection,
        std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    ) {
        let (host, kernel) = tokio::io::duplex(64 * 1024);
        let (connection, _incoming) = Connection::new(host);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = seen.clone();
        tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(kernel);
            let mut lines = tokio::io::BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let request: Value = serde_json::from_str(&line).expect("request frame");
                let method = request["method"].as_str().unwrap_or_default().to_owned();
                recorded
                    .lock()
                    .expect("recorded requests")
                    .push((method.clone(), request["params"].clone()));
                let reply = match method.as_str() {
                    PROFILE_LLM_LIST => list.clone(),
                    _ => select.clone(),
                };
                let mut response = json!({ "jsonrpc": "2.0", "id": request["id"] });
                let object = response.as_object_mut().expect("an object");
                match reply {
                    Reply::Ok(result) => {
                        object.insert("result".to_owned(), result);
                    }
                    Reply::Err(error) => {
                        object.insert("error".to_owned(), error);
                    }
                }
                writer
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .expect("write reply");
            }
        });
        (connection, seen)
    }

    /// Every key a row carries survives into the picker: the id is the kernel's
    /// address, the name its title, the description its route — and both rows are
    /// offered, in the kernel's order.
    #[test]
    fn the_kernels_rows_become_the_pickers_list() {
        let result = listed(json!([
            row_json("deepseek-chat", "deepseek", true),
            row_json("glm-5.3", "zai", false),
        ]));

        let AgentModelList::Flat(models) = models_from_result(&result).expect("a list") else {
            panic!("a profile's configured models are a flat list");
        };
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id.as_str(), "deepseek/deepseek-chat#official");
        assert_eq!(models[0].name.to_string(), "deepseek / deepseek-chat");
        assert_eq!(
            models[0].description.clone().map(|route| route.to_string()),
            Some("route official".to_owned())
        );
        assert_eq!(models[1].id.as_str(), "zai/glm-5.3#official");
        assert!(models.iter().all(|model| model.disabled.is_none()));
        // Nothing the kernel does not send is invented.
        assert!(models.iter().all(|model| model.icon.is_none()));
        assert!(models.iter().all(|model| !model.is_latest));
        assert!(models.iter().all(|model| model.cost.is_none()));
    }

    /// `available: false` is a model a person can see and cannot pick. And a row
    /// the kernel left the flag off is *not* unavailable: absent is not false.
    #[test]
    fn an_unavailable_model_is_offered_but_not_selectable() {
        let unavailable = row_json("gone", "zai", false);
        let mut unavailable = unavailable;
        unavailable["available"] = json!(false);
        // A row from a kernel that does not send the flag at all.
        let mut unflagged = row_json("plain", "zai", false);
        unflagged.as_object_mut().expect("an object").remove("available");

        let result = listed(json!([unavailable, unflagged]));
        let AgentModelList::Flat(models) = models_from_result(&result).expect("a list") else {
            panic!("flat");
        };
        assert_eq!(
            models[0]
                .disabled
                .clone()
                .map(|reason| reason.0.to_string()),
            Some(UNAVAILABLE.to_owned()),
            "an unavailable model must not be selectable"
        );
        assert!(
            models[1].disabled.is_none(),
            "a missing `available` means the kernel said nothing, not `false`"
        );
    }

    /// A row without a model id is not offered: an id this host invented would be
    /// a model the person could click and the kernel has never heard of.
    #[test]
    fn a_row_without_a_model_id_is_not_offered() {
        let mut nameless = row_json("x", "zai", false);
        nameless.as_object_mut().expect("an object").remove("model");
        let mut blank = row_json("x", "zai", false);
        blank["model"] = json!("   ");

        let result = listed(json!([nameless, blank, row_json("real", "zai", true)]));
        let AgentModelList::Flat(models) = models_from_result(&result).expect("a list") else {
            panic!("flat");
        };
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id.as_str(), "zai/real#official");
    }

    /// A profile with nothing configured is an error that carries the next step,
    /// not an empty picker.
    #[test]
    fn a_profile_with_no_model_is_an_error_not_an_empty_list() {
        let error = models_from_result(&listed(json!([]))).expect_err("nothing is configured");
        assert!(error.to_string().contains("no model configured"), "{error}");

        let error = active_model_from_result(&listed(json!([]))).expect_err("nothing is selected");
        assert!(error.to_string().contains("no selected model"), "{error}");
    }

    /// `selected` is the kernel's answer to "which model is active", and a
    /// listing that names none is not silently turned into the first row.
    #[test]
    fn the_selected_row_is_the_active_model() {
        let result = listed(json!([
            row_json("deepseek-chat", "deepseek", false),
            row_json("glm-5.3", "zai", true),
        ]));
        let active = active_model_from_result(&result).expect("a selected row");
        assert_eq!(active.id.as_str(), "zai/glm-5.3#official");

        let none_selected = listed(json!([row_json("deepseek-chat", "deepseek", false)]));
        assert!(
            active_model_from_result(&none_selected).is_err(),
            "no row is the active model, so there is nothing to report as one"
        );
    }

    /// The same family/model on two routes is two entries with two ids, and a
    /// selection carries the route of the row it came from — the discriminator the
    /// kernel requires (`ui_protocol_transport.rs:13964-14004`).
    #[test]
    fn a_same_model_on_two_routes_is_two_entries_each_with_its_own_route() {
        let mut mirror = row_json("glm-5.3", "zai", false);
        mirror["route"] = json!("mirror");
        let result = listed(json!([row_json("glm-5.3", "zai", true), mirror]));

        let AgentModelList::Flat(models) = models_from_result(&result).expect("a list") else {
            panic!("flat");
        };
        assert_ne!(models[0].id, models[1].id, "two routes are two models");
        assert_eq!(models[0].id.as_str(), "zai/glm-5.3#official");
        assert_eq!(models[1].id.as_str(), "zai/glm-5.3#mirror");

        let rows = model_rows(&result);
        assert_eq!(
            select_params(&session(), &rows[1]),
            json!({
                "session_id": "_main:zed:t",
                "model_id": "glm-5.3",
                "family_id": "zai",
                "route_id": "mirror",
            }),
            "the route must be the row's own, not the primary's"
        );
    }

    /// The select params are exactly the keys `RawProfileLlmSelectParams` accepts
    /// (`deny_unknown_fields`), and a row with no route names no route.
    #[test]
    fn a_selection_names_the_kernels_own_address_and_nothing_else() {
        let result = listed(json!([row_json("deepseek-chat", "deepseek", true)]));
        let rows = model_rows(&result);
        assert_eq!(
            select_params(&session(), &rows[0]),
            json!({
                "session_id": "_main:zed:t",
                "model_id": "deepseek-chat",
                "family_id": "deepseek",
                "route_id": "official",
            })
        );

        let mut routeless = row_json("local", "ollama", true);
        routeless["route"] = Value::Null;
        routeless["family"] = Value::Null;
        let rows = model_rows(&listed(json!([routeless])));
        assert_eq!(
            select_params(&session(), &rows[0]),
            json!({ "session_id": "_main:zed:t", "model_id": "local" }),
            "an absent family/route is absent on the wire, never `null` fields the \
             kernel would have to interpret"
        );
        // The id of a routeless row is still a usable identity.
        assert_eq!(rows[0].id().as_str(), "local");
    }

    /// The kernel's refusal is reported in its own words: "not configured on
    /// profile" is the real path for a model the profile does not carry
    /// (`raw_profile_llm_select`, `:13988-13996`), and the person needs to read
    /// it rather than see a generic failure.
    #[test]
    fn a_refused_selection_is_reported_with_the_kernels_own_reason() {
        let reply = json!({
            "jsonrpc": "2.0",
            "id": "7",
            "error": {
                "code": -32004,
                "message": "configured model: glm-5.3 (not configured on profile 'main' — save \
                            it first via profile/llm/upsert / the onboarding provider step)",
                "data": { "kind": "not_found" },
            },
        });
        let error = result_of(&reply, "the Octos kernel refused the model selection")
            .expect_err("a refusal is not a selection");
        let message = error.to_string();
        assert!(
            message.contains("not configured on profile 'main'"),
            "the kernel's own reason must survive: {message}"
        );
        assert!(message.contains("-32004"), "and its code: {message}");
        // The `data` bag is not dumped into what a person reads.
        assert!(!message.contains("not_found"), "{message}");

        // A successful reply carries a result, and a result-less one is not an
        // error: `select` always answers with an object.
        let reply = json!({ "jsonrpc": "2.0", "id": "7", "result": { "applied": true } });
        assert_eq!(
            result_of(&reply, "x").expect("a result"),
            json!({ "applied": true })
        );
    }

    /// **No switch is ever reported as immediate.** Each disposition the kernel
    /// can stamp gets the note it earns, and the pinned profile — the one case
    /// where neither "now" nor "the next thread" is true — says exactly that.
    #[test]
    fn a_switch_is_never_reported_as_immediate() {
        let receipt = |disposition: &str, restart: bool, applied: bool| {
            json!({
                "session_id": "_main:zed:t",
                "selected": { "model": "glm-5.3", "title": "zai / glm-5.3", "selected": true },
                "applied": applied,
                "runtime_disposition": disposition,
                "restart_required": restart,
                "effective_from": if disposition == "unchanged" { Value::Null } else { json!("next_turn") },
            })
        };

        let reloaded = select_note(&receipt("reloaded", false, true));
        assert!(reloaded.contains("zai / glm-5.3"), "{reloaded}");
        assert!(reloaded.contains("next turn"), "{reloaded}");

        let deferred = select_note(&receipt("deferred", false, true));
        assert!(deferred.contains("next session/turn"), "{deferred}");

        let failed = {
            let mut receipt = receipt("persisted_but_not_live", false, true);
            receipt["runtime_error"] = json!("no key for ZAI_API_KEY");
            select_note(&receipt)
        };
        assert!(failed.contains("could not rebuild"), "{failed}");
        assert!(failed.contains("no key for ZAI_API_KEY"), "{failed}");

        // The pinned profile: saved, but the kernel keeps serving what it booted
        // with — and a *new thread* does not change that either. This is the note
        // that must not be rounded up to "next thread".
        let pinned = select_note(&receipt("restart_required", true, true));
        assert!(pinned.contains("pinned to what the kernel booted with"), "{pinned}");
        assert!(pinned.contains("keeps serving the previous model"), "{pinned}");
        assert!(pinned.contains("until the kernel restarts"), "{pinned}");
        assert!(pinned.contains("a new thread does not change that"), "{pinned}");

        // An idempotent re-select on a pinned profile: nothing was persisted, and
        // the note says so instead of claiming a change.
        let already = select_note(&receipt("unchanged", true, false));
        assert!(already.contains("is already saved"), "{already}");
        assert!(already.contains("pinned to what the kernel booted"), "{already}");

        let unchanged = select_note(&receipt("unchanged", false, false));
        assert!(unchanged.contains("already the profile's primary"), "{unchanged}");

        // Whatever the kernel reports, "now"/"immediately" is never this host's
        // word.
        for (disposition, restart, applied) in [
            ("reloaded", false, true),
            ("deferred", false, true),
            ("persisted_but_not_live", false, true),
            ("restart_required", true, true),
            ("unchanged", true, false),
            ("unchanged", false, false),
            ("something-new", false, true),
        ] {
            let note = select_note(&receipt(disposition, restart, applied));
            for overclaim in ["immediately", " is now ", "takes effect now"] {
                assert!(
                    !note.contains(overclaim),
                    "`{overclaim}` is not the kernel's word: {note}"
                );
            }
        }
    }

    /// The wire against a fake kernel: the picker asks for **this thread's**
    /// session (which is what makes the kernel answer the per-model status rows),
    /// and the reply is read with the row mapper.
    #[tokio::test]
    async fn the_picker_asks_for_this_threads_session_and_maps_the_rows() {
        let list = listed(json!([row_json("glm-5.3", "zai", true)]));
        let (connection, seen) = fake_kernel(Reply::Ok(list), Reply::Ok(Value::Null)).await;

        let mapped = models(&connection, &session()).await.expect("a list");
        let AgentModelList::Flat(models) = mapped else {
            panic!("flat");
        };
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id.as_str(), "zai/glm-5.3#official");

        let requests = seen.lock().expect("recorded").clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0.as_str(), PROFILE_LLM_LIST);
        assert_eq!(
            requests[0].1,
            json!({ "session_id": "_main:zed:t" }),
            "the session is what tells the kernel which profile to list"
        );

        // And the same call is what `selected_model` reads.
        let active = selected(&connection, &session()).await.expect("active");
        assert_eq!(active.id.as_str(), "zai/glm-5.3#official");
    }

    /// A model the kernel no longer lists cannot be selected: the selection
    /// re-reads the list, so a stale picker id fails with a readable reason
    /// rather than sending a frame about a model the profile does not have.
    #[tokio::test]
    async fn a_model_the_kernel_no_longer_lists_is_not_selectable() {
        let list = listed(json!([row_json("glm-5.3", "zai", true)]));
        let (connection, _seen) = fake_kernel(Reply::Ok(list), Reply::Ok(Value::Null)).await;

        let error = row_for(
            &connection,
            &session(),
            &AgentModelId::new("zai/gone#official"),
        )
        .await
        .expect_err("it is not listed");
        assert!(error.to_string().contains("does not list a model"), "{error}");
    }

    /// A selection's wire, against a fake kernel: the params are the row's own
    /// address, and the note comes from the receipt the kernel answered with.
    #[tokio::test]
    async fn a_selection_sends_the_rows_address_and_reads_the_receipt() {
        let list = listed(json!([row_json("glm-5.3", "zai", true)]));
        let receipt = json!({
            "session_id": "_main:zed:t",
            "selected": { "model": "glm-5.3", "title": "zai / glm-5.3", "selected": true },
            "applied": true,
            "runtime_disposition": "restart_required",
            "restart_required": true,
        });
        let (connection, seen) = fake_kernel(Reply::Ok(list), Reply::Ok(receipt)).await;

        let row = row_for(
            &connection,
            &session(),
            &AgentModelId::new("zai/glm-5.3#official"),
        )
        .await
        .expect("listed");
        let note = select_row(&connection, &session(), &row)
            .await
            .expect("the kernel answered");

        let requests = seen.lock().expect("recorded").clone();
        let (method, params) = requests.last().expect("a select request");
        assert_eq!(method.as_str(), PROFILE_LLM_SELECT);
        assert_eq!(
            params,
            &json!({
                "session_id": "_main:zed:t",
                "model_id": "glm-5.3",
                "family_id": "zai",
                "route_id": "official",
            })
        );
        assert!(note.contains("restart"), "{note}");
    }

    /// The kernel refusing a selection is an `Err` for the caller — never a note
    /// that reads like a switch. This is the "not configured on profile" path
    /// (`raw_profile_llm_select`, `ui_protocol_transport.rs:13988-13996`), which is
    /// what a person sees for a model their Octos profile does not carry.
    #[tokio::test]
    async fn a_refused_selection_never_becomes_a_note() {
        let list = listed(json!([row_json("glm-5.3", "zai", true)]));
        let refusal = json!({
            "code": -32004,
            "message": "configured model: glm-5.3 (not configured on profile 'main' — save it \
                        first via profile/llm/upsert / the onboarding provider step)",
        });
        // The refusal advertises the id it *does* list, so the host reaches the
        // select frame and the kernel's answer is what fails it.
        let (connection, seen) = fake_kernel(Reply::Ok(list), Reply::Err(refusal)).await;

        let row = row_for(
            &connection,
            &session(),
            &AgentModelId::new("zai/glm-5.3#official"),
        )
        .await
        .expect("listed");

        let error = select_row(&connection, &session(), &row)
            .await
            .expect_err("a refusal is not a selection");
        let message = error.to_string();
        assert!(
            message.contains("not configured on profile 'main'"),
            "the kernel's own reason must reach the caller: {message}"
        );
        assert_eq!(
            seen.lock().expect("recorded").len(),
            2,
            "the selection was really attempted: a list and a select"
        );
    }
}
