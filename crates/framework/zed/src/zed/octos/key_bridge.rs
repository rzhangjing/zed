//! Zed's own provider credential → the Octos profile's credential slot.
//!
//! **Why a copy is the only mechanism.** Octos resolves a provider key from its
//! *own* profile (`config.env_vars[<api_key_env>]`, read through its own secret
//! store, or the process environment); nothing in the kernel can look up Zed's
//! keyring item, which is a per-URL entry under Zed's own service name. So a
//! credential only Zed holds has to be **copied**, and this module is the only
//! place that does it.
//!
//! **Where a copy lands**, in the order this module tries:
//!
//! 1. **The Octos secret store** — `octos_cli::auth::keychain`, the same store
//!    `octos auth set-key` and the kernel's own relocation path use
//!    (`crates/octos/octos-cli/src/api/auth_handlers.rs:1298-1322`). The key goes
//!    in under the kernel's own per-profile account
//!    (`scoped_account(env_var, profile_id)`, `auth/keychain.rs:69-71`), and the
//!    profile holds only the marker that points at it (`marker_for`, `:74-76`;
//!    resolved on read by `resolve_env_vars`, `:750`). **Profile config: no key.**
//!    This is the path taken whenever the platform has a store at all
//!    (`keychain::is_available()`, `:108-125`).
//! 2. **`profile/llm/upsert`'s `api_key`** —
//!    `ui_protocol_transport.rs:14174-14179` inserts it into
//!    `profile.config.env_vars[<api_key_env>]`, which is persisted in the profile
//!    file (`<octos home>/…/profiles/<profile_id>.json`). With no secret store
//!    that value is **plaintext** — the case on **Windows**, where
//!    `keychain::is_available()` is `false` by design (#2234) and the documented
//!    workarounds are the process environment or `env_vars` (`auth/keychain.rs:127-141`).
//!    The kernel's `relocate_keychain_backed_secrets` (`:14243`) moves a value
//!    into the keychain only for keychain-backed **shapes** — a raw
//!    service-account JSON (`auth_handlers.rs:1310-1312` leaves anything that
//!    does not start with `{` exactly where it is) — so an ordinary API key stays
//!    plaintext there too. This module says so in the note it reports.
//!
//! **No key is ever logged, here or downstream.** `octos::connection`'s `call` and
//! `write_frames` deliberately log no frame, and that is a *constraint on this
//! module*, not a coincidence: on the plaintext path the `profile/llm/upsert`
//! payload **contains the key**, so adding frame logging on that path would print
//! it. The log lines and the note this module produces name the env var, the URL
//! and the destination — never the secret.
//!
//! What this module deliberately does **not** do: write Zed's models into Octos,
//! read Octos's credentials back into Zed, or touch a credential when the kernel
//! says the profile already has one. Nothing is *moved* — the key Zed holds stays
//! where it is.

use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

use super::connection::Connection;
use super::model_selector::{ModelRow, PROFILE_LLM_LIST, result_of};

/// `profile/llm/upsert` — a private const in the AppUI transport
/// (`crates/octos/octos-cli/src/api/ui_protocol_transport.rs:383`); handler
/// `raw_profile_llm_upsert`, `:14148`.
const PROFILE_LLM_UPSERT: &str = "profile/llm/upsert";

/// How long the bridge waits for the kernel to accept a credential.
const PROFILE_LLM_TIMEOUT: Duration = Duration::from_secs(30);

/// The `selection` keys `profile/llm/upsert` accepts and the kernel echoes back
/// (`configured_provider_json`, `ui_protocol_transport.rs:11301-11324`;
/// accepted set `LLM_SELECTION_KNOWN`, `:9832-9841`).
///
/// Echoed verbatim so setting a key cannot clear the model's configured inference
/// overrides: in this schema `absent ≡ null ≡ inherit` (`:9749-9754`), so a
/// re-upsert that omitted them would *erase* them.
const SELECTION_ECHO: &[&str] = &[
    "family_id",
    "model_id",
    "model_hints",
    "context_window",
    "temperature",
    "top_p",
    "reasoning_effort",
];

/// The `route` keys `profile/llm/upsert` accepts (`LLM_ROUTE_KNOWN`, `:9842`).
const ROUTE_ECHO: &[&str] = &["route_id", "label", "base_url", "api_key_env", "api_type"];

/// The keys the kernel echoes but the upsert schema **rejects** as unknown:
/// `cost_per_m` / `strong` are not caller-owned (`:11319-11324` echoes them, but
/// `LLM_SELECTION_KNOWN` does not list them) — the kernel carries them forward
/// from the same-address selection itself (`:14203-14212`). Echoing them back
/// would make the whole payload an `invalid_params`, which is why they are named
/// here rather than copied.
const SELECTION_ECHO_EXCLUDED: &[&str] = &["cost_per_m", "strong"];

/// What to do about one configured model's credential slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyBridgeDecision {
    /// Octos already has a value for the route's `api_key_env` (a plain key, or a
    /// `keychain:` marker, which is a value too). Nothing to do, and nothing is
    /// overwritten: a profile that already works is left alone.
    AlreadyPresent,
    /// Octos has no value and Zed holds a credential for the model's URL: copy it.
    NeedsCopy,
    /// Octos has no value and Zed holds none for that URL — or the route names no
    /// URL to look one up under. There is nothing honest to copy, so the
    /// selection proceeds and the kernel's own refusal (if any) is what the
    /// person sees.
    ZedHasNone,
    /// The route declares no `api_key_env`, so there is no named slot to write
    /// to. The kernel can derive one from the family (`.or_else(||
    /// dashboard_family_api_key_env(&family_id))`, `:14170-14171`), but that name
    /// is not on this wire — inventing it here is exactly the guess this module
    /// refuses to make.
    Skip,
}

/// The pure decision. Both inputs are facts the kernel and Zed each own:
/// `octos_has_value` is the kernel's own `has_api_key` for the addressed model
/// (`configured_provider_json`, `:11289-11291`), `zed_has_credential` is whether
/// Zed's credentials provider answered for the model's `base_url`.
pub(super) fn decide_key_bridge(
    api_key_env: Option<&str>,
    octos_has_value: bool,
    zed_has_credential: bool,
) -> KeyBridgeDecision {
    match api_key_env {
        None => KeyBridgeDecision::Skip,
        Some(_) if octos_has_value => KeyBridgeDecision::AlreadyPresent,
        Some(_) if zed_has_credential => KeyBridgeDecision::NeedsCopy,
        Some(_) => KeyBridgeDecision::ZedHasNone,
    }
}

/// Where a copied key ends up. The caller picks this from
/// `octos_cli::auth::keychain::is_available()`, so the choice is visible at the
/// call site instead of buried in the copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CopyDestination {
    /// The Octos secret store, addressed by the kernel's own scoped account. The
    /// profile config receives a `keychain:<account>` marker, never the key.
    OctosKeychain,
    /// `profile.config.env_vars[<api_key_env>]` inside the profile file —
    /// **plaintext**. This is what a platform with no Octos secret store gets
    /// (Windows, `auth/keychain.rs:108-125`).
    ProfileEnvVarsPlaintext,
}

/// The destination for a host whose secret store is (or is not) available.
pub(super) fn copy_destination(secret_store_available: bool) -> CopyDestination {
    if secret_store_available {
        CopyDestination::OctosKeychain
    } else {
        CopyDestination::ProfileEnvVarsPlaintext
    }
}

/// One configured model, as the **profile** shape of `profile/llm/list` reports
/// it (`profile_llm_list_result`, `ui_protocol_transport.rs:11957-11989`, whose
/// `primary`/`fallbacks` entries are `configured_provider_json`, `:11270-11326`).
///
/// This shape — unlike the per-session status rows the picker reads — carries
/// `api_key_env`, `has_api_key` and `base_url`, which is why the bridge asks the
/// kernel for it separately.
#[derive(Debug, Clone)]
pub(super) struct KeyCandidate {
    /// `family_id` (`:11283`).
    pub(super) family_id: Option<String>,
    /// `model_id` (`:11284`). A candidate without one is not offered.
    pub(super) model_id: String,
    /// `route_id` (`:11286`) — the discriminating half of the address.
    pub(super) route_id: Option<String>,
    /// `api_key_env` (`:11288`) — the env var name the kernel will store the key
    /// under (`:14170-14179`).
    pub(super) api_key_env: Option<String>,
    /// `base_url` (`:11287`) — the URL Zed's credential for this model would be
    /// filed under.
    pub(super) base_url: Option<String>,
    /// `has_api_key` (`:11289-11291`) — the kernel's own answer to "is there a
    /// value for `api_key_env` already". Absent is read as **true** (do not
    /// touch): only an explicit `false` may cause a write.
    pub(super) has_api_key: bool,
    /// The kernel's row, kept whole so [`upsert_params`] can echo the configured
    /// inference fields back verbatim.
    row: Value,
}

/// A non-empty string field of a kernel body, trimmed.
fn text(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

impl KeyCandidate {
    fn from_json(row: &Value) -> Option<Self> {
        let model_id = text(row, "model_id")?;
        Some(Self {
            family_id: text(row, "family_id"),
            model_id,
            route_id: text(row, "route_id"),
            api_key_env: text(row, "api_key_env"),
            base_url: text(row, "base_url"),
            // Fail safe: a body that does not say can never make this host write.
            has_api_key: row
                .get("has_api_key")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            row: row.clone(),
        })
    }

    /// The model's `family/model` address, as the picker's row spells it.
    fn address(&self) -> String {
        match &self.family_id {
            Some(family) => format!("{family}/{}", self.model_id),
            None => self.model_id.clone(),
        }
    }
}

/// The configured models a `profile/llm/list` **profile** result names, primary
/// first.
///
/// A row without a `model_id` is skipped: an address this host invented would be
/// a credential written to a model the kernel never listed.
fn key_candidates(result: &Value) -> Vec<KeyCandidate> {
    let mut candidates = Vec::new();
    let primary = result.get("primary").filter(|value| !value.is_null());
    let fallbacks = result
        .get("fallbacks")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for row in primary.into_iter().chain(fallbacks.iter()) {
        match KeyCandidate::from_json(row) {
            Some(candidate) => candidates.push(candidate),
            None => log::warn!(
                "[octos] a profile/llm/list entry names no model_id, so no credential can be \
                 addressed for it: {row}"
            ),
        }
    }
    candidates
}

/// The candidate a picker row addresses: same family, same model, same route.
///
/// Compared the way the kernel compares an address
/// (`same_llm_selection_address`, `ui_protocol_transport.rs:14209-14212`): the
/// route is part of it, because the same family/model on two routes has two
/// endpoints and two credentials.
pub(super) fn candidate_for<'a>(
    candidates: &'a [KeyCandidate],
    row: &ModelRow,
) -> Option<&'a KeyCandidate> {
    candidates.iter().find(|candidate| {
        candidate.model_id == row.model
            && candidate.family_id.as_deref() == row.family.as_deref()
            && candidate.route_id.as_deref() == row.route.as_deref()
    })
}

/// The `profile/llm/upsert` params that store `secret` for one candidate.
///
/// The payload is the addressed selection **plus** the key: `absent ≡ null ≡
/// inherit` (`:9749-9754`), so every inference field the kernel echoed is echoed
/// back unchanged, minus the two the schema rejects
/// ([`SELECTION_ECHO_EXCLUDED`]). Nothing about the model is re-decided here —
/// only the credential is added.
///
/// `secret` is either the copied key (plaintext destination) or the secret
/// store's marker ([`CopyDestination::OctosKeychain`]). **Never log the result.**
fn upsert_params(candidate: &KeyCandidate, secret: &str) -> Value {
    // The two lists must stay disjoint: a key that is echoed and rejected at the
    // same time would fail the whole upsert (see `SELECTION_ECHO_EXCLUDED`).
    debug_assert!(
        SELECTION_ECHO
            .iter()
            .all(|key| !SELECTION_ECHO_EXCLUDED.contains(key)),
        "an echoed field must not be one the upsert schema rejects"
    );
    let mut selection = Map::new();
    for key in SELECTION_ECHO {
        if let Some(value) = candidate.row.get(*key).filter(|value| !value.is_null()) {
            selection.insert((*key).to_owned(), value.clone());
        }
    }
    // The address must be present even when the kernel's row left a field out of
    // the echo: without it the upsert would address nothing.
    selection
        .entry("family_id".to_owned())
        .or_insert_with(|| json!(candidate.family_id));
    selection
        .entry("model_id".to_owned())
        .or_insert_with(|| json!(candidate.model_id));

    let mut route = Map::new();
    if let Some(route_row) = candidate.row.get("route").filter(|value| value.is_object()) {
        for key in ROUTE_ECHO {
            if let Some(value) = route_row.get(*key).filter(|value| !value.is_null()) {
                route.insert((*key).to_owned(), value.clone());
            }
        }
    }
    if !route.is_empty() {
        selection.insert("route".to_owned(), Value::Object(route));
    }

    json!({
        "profile_id": octos_core::MAIN_PROFILE_ID,
        "selection": Value::Object(selection),
        "api_key": secret,
    })
}

/// The profile's configured models, from the kernel's own profile document.
///
/// Called **without** a `session_id` on purpose: with a session the kernel
/// answers the per-model status rows, which carry no `api_key_env`/`has_api_key`
/// (compare `model_list_result:12009` with `profile_llm_list_result:11957`). The
/// `profile_id` is this host's own (`_main`: every Octos session
/// `agent_thread::thread_session_key` opens is on the main profile), stated
/// rather than derived.
pub(super) async fn profile_candidates(connection: &Connection) -> Result<Vec<KeyCandidate>> {
    let reply = connection
        .call_with_timeout(
            PROFILE_LLM_LIST,
            json!({ "profile_id": octos_core::MAIN_PROFILE_ID }),
            PROFILE_LLM_TIMEOUT,
        )
        .await
        .map_err(|failure| {
            anyhow!("the Octos kernel did not answer {PROFILE_LLM_LIST}: {failure}")
        })?;
    let result = result_of(&reply, "the Octos kernel refused profile/llm/list")?;
    Ok(key_candidates(&result))
}

/// Copies `key` into the Octos profile's slot for `candidate`, and returns the
/// line the console shows.
///
/// The note names the destination because the two destinations are genuinely
/// different for a person: one keeps the key out of the profile file, the other
/// writes it there in plaintext. Their console must not be the same sentence.
///
/// `key` is never logged and never included in the note or in any error.
pub(super) async fn copy(
    connection: &Connection,
    candidate: &KeyCandidate,
    key: &str,
    destination: CopyDestination,
) -> Result<String> {
    let Some(env_var) = candidate.api_key_env.as_deref() else {
        return Err(anyhow!(
            "cannot store a credential for {}: its route declares no api_key_env to store it \
             under",
            candidate.address()
        ));
    };
    let url = candidate
        .base_url
        .as_deref()
        .unwrap_or("the route's endpoint");

    let (secret, landed) = match destination {
        CopyDestination::OctosKeychain => {
            let account = octos_cli::auth::keychain::scoped_account(
                env_var,
                octos_core::MAIN_PROFILE_ID,
            );
            octos_cli::auth::keychain::set_secret(&account, key).map_err(|error| {
                anyhow!(
                    "could not store the credential for {url} in the Octos secret store as \
                     {account}: {error}"
                )
            })?;
            // Only the marker reaches the profile; the key itself is in the store.
            (
                octos_cli::auth::keychain::marker_for(&account),
                format!(
                    "the Octos secret store (account `{account}`), so {env_var} in the profile \
                     holds only a keychain marker"
                ),
            )
        }
        CopyDestination::ProfileEnvVarsPlaintext => (
            key.to_owned(),
            format!(
                "the profile's config.env_vars[{env_var}] — plaintext in the profile file, \
                 because this platform has no Octos secret store"
            ),
        ),
    };

    let reply = connection
        .call_with_timeout(
            PROFILE_LLM_UPSERT,
            upsert_params(candidate, &secret),
            PROFILE_LLM_TIMEOUT,
        )
        .await
        .map_err(|failure| anyhow!("the Octos kernel did not answer {PROFILE_LLM_UPSERT}: {failure}"))?;
    // The refusal is reported by its message, never by its body: the reply of an
    // upsert is not a place to print (and the payload is not a thing to re-log).
    result_of(&reply, "the Octos kernel refused to store the credential")?;

    log::info!(
        "[octos] copied Zed's credential for {url} into {landed} for {}",
        candidate.address()
    );
    Ok(format!(
        "copied Zed's credential for {url} into {landed} — it is now usable by Octos as \
         {env_var} for {}",
        candidate.address()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A credential that must never reach a log. Deliberately not shaped like a
    /// real key, and only ever compared, never printed by a passing assertion.
    const FAKE_KEY: &str = "zz-fake-bridge-key-not-a-secret";

    fn candidate_row(extra: Value) -> Value {
        let mut row = json!({
            "provider": "zai",
            "model": "glm-5.3",
            "family_id": "zai",
            "model_id": "glm-5.3",
            "route": {
                "route_id": "official",
                "base_url": "https://api.z.ai/api/paas/v4",
                "api_key_env": "ZAI_API_KEY",
                "api_type": "openai",
            },
            "route_id": "official",
            "base_url": "https://api.z.ai/api/paas/v4",
            "api_key_env": "ZAI_API_KEY",
            "has_api_key": false,
            "selected": true,
            "available": true,
        });
        let object = row.as_object_mut().expect("an object");
        for (key, value) in extra.as_object().expect("an object") {
            object.insert(key.clone(), value.clone());
        }
        row
    }

    fn candidate(extra: Value) -> KeyCandidate {
        KeyCandidate::from_json(&candidate_row(extra)).expect("a candidate")
    }

    /// A picker row for the same address, built from the *session* shape the
    /// picker reads.
    fn picker_row(model: &str, family: &str, route: Option<&str>) -> ModelRow {
        ModelRow {
            family: Some(family.to_owned()),
            model: model.to_owned(),
            title: None,
            route: route.map(ToOwned::to_owned),
            selected: false,
            available: true,
        }
    }

    /// The four states, and the one that must win any tie: a profile that already
    /// has a value is **never** written to.
    #[test]
    fn the_decision_has_exactly_four_states() {
        assert_eq!(
            decide_key_bridge(Some("ZAI_API_KEY"), true, true),
            KeyBridgeDecision::AlreadyPresent
        );
        assert_eq!(
            decide_key_bridge(Some("ZAI_API_KEY"), true, false),
            KeyBridgeDecision::AlreadyPresent,
            "Zed's copy must not overwrite a key Octos already has"
        );
        assert_eq!(
            decide_key_bridge(Some("ZAI_API_KEY"), false, true),
            KeyBridgeDecision::NeedsCopy
        );
        assert_eq!(
            decide_key_bridge(Some("ZAI_API_KEY"), false, false),
            KeyBridgeDecision::ZedHasNone
        );
        assert_eq!(
            decide_key_bridge(None, false, true),
            KeyBridgeDecision::Skip,
            "no api_key_env is no slot to write to, whatever Zed holds"
        );
        assert_eq!(
            decide_key_bridge(None, true, false),
            KeyBridgeDecision::Skip
        );
    }

    /// The destination is derived, not guessed, and a platform with a secret
    /// store never falls back to the plaintext one.
    #[test]
    fn where_the_key_lands_is_decided_by_whether_octos_has_a_store() {
        assert_eq!(copy_destination(true), CopyDestination::OctosKeychain);
        assert_eq!(
            copy_destination(false),
            CopyDestination::ProfileEnvVarsPlaintext
        );
    }

    /// A body that does not say whether the profile already has a key must not
    /// make this host write: only an explicit `has_api_key: false` does.
    #[test]
    fn a_body_that_does_not_say_has_api_key_is_not_read_as_no_key() {
        let mut row = candidate_row(json!({}));
        row.as_object_mut()
            .expect("an object")
            .remove("has_api_key");
        let candidate = KeyCandidate::from_json(&row).expect("a candidate");
        assert!(
            candidate.has_api_key,
            "absent must mean 'do not touch', not 'overwrite'"
        );
        assert_eq!(
            decide_key_bridge(candidate.api_key_env.as_deref(), candidate.has_api_key, true),
            KeyBridgeDecision::AlreadyPresent
        );

        // And a row without a model_id is not a candidate at all.
        let mut nameless = candidate_row(json!({}));
        nameless.as_object_mut().expect("an object").remove("model_id");
        assert!(KeyCandidate::from_json(&nameless).is_none());
    }

    /// The profile shape is read from `primary` and `fallbacks`, in that order,
    /// and an entry with no model is skipped rather than addressed.
    #[test]
    fn the_profile_document_yields_the_primary_and_the_fallbacks() {
        let mut primary = candidate_row(json!({}));
        primary["has_api_key"] = json!(true);
        let mut nameless = candidate_row(json!({}));
        nameless.as_object_mut().expect("an object").remove("model_id");
        let fallback = candidate_row(json!({ "model_id": "glm-4.6", "api_key_env": "GLM_KEY" }));

        let result = json!({
            "profile_id": "_main",
            "primary": primary,
            "fallbacks": [fallback, nameless],
        });
        let candidates = key_candidates(&result);
        assert_eq!(candidates.len(), 2, "the nameless entry is skipped");
        assert_eq!(candidates[0].model_id, "glm-5.3");
        assert!(candidates[0].has_api_key);
        assert_eq!(candidates[1].model_id, "glm-4.6");
        assert_eq!(candidates[1].api_key_env.as_deref(), Some("GLM_KEY"));
        assert_eq!(candidates[0].address(), "zai/glm-5.3");

        // A profile with nothing configured has no candidate at all.
        assert!(key_candidates(&json!({ "primary": Value::Null, "fallbacks": [] })).is_empty());
    }

    /// The same model on two routes is two candidates: a key copied for one
    /// endpoint must not be filed under the other.
    #[test]
    fn a_route_is_part_of_the_address_that_is_matched() {
        let mut mirror = candidate_row(json!({ "api_key_env": "ZAI_MIRROR_KEY" }));
        mirror["route_id"] = json!("mirror");
        mirror["route"]["route_id"] = json!("mirror");
        let result = json!({ "primary": candidate_row(json!({})), "fallbacks": [mirror] });
        let candidates = key_candidates(&result);

        let official = picker_row("glm-5.3", "zai", Some("official"));
        let mirror_row = picker_row("glm-5.3", "zai", Some("mirror"));
        assert_eq!(
            candidate_for(&candidates, &official)
                .expect("the official one")
                .api_key_env
                .as_deref(),
            Some("ZAI_API_KEY")
        );
        assert_eq!(
            candidate_for(&candidates, &mirror_row)
                .expect("the mirror one")
                .api_key_env
                .as_deref(),
            Some("ZAI_MIRROR_KEY")
        );
        assert!(
            candidate_for(&candidates, &picker_row("glm-5.3", "zai", None)).is_none(),
            "a row whose route the profile does not carry is not an address to write to"
        );
    }

    /// The upsert echoes the configured inference fields back — because `absent ≡
    /// inherit` would otherwise *erase* them (`ui_protocol_transport.rs:9749-9754`)
    /// — while dropping the two the schema rejects, and it carries exactly the
    /// four top-level keys `LLM_UPSERT_KNOWN_TOP` allows.
    #[test]
    fn the_upsert_echoes_the_inference_fields_and_drops_what_the_schema_rejects() {
        let candidate = candidate(json!({
            "context_window": 262144,
            "temperature": 0.2,
            "top_p": 0.9,
            "reasoning_effort": "high",
            "model_hints": { "lacks_vision": true },
            "cost_per_m": 0.6,
            "strong": true,
        }));
        let params = upsert_params(&candidate, FAKE_KEY);

        let mut top: Vec<&str> = params
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        top.sort_unstable();
        assert_eq!(
            top,
            vec!["api_key", "profile_id", "selection"],
            "the upsert schema allows exactly these (LLM_UPSERT_KNOWN_TOP)"
        );
        assert_eq!(params["profile_id"], json!("_main"));

        let selection = params["selection"].as_object().expect("an object");
        assert_eq!(selection["family_id"], json!("zai"));
        assert_eq!(selection["model_id"], json!("glm-5.3"));
        assert_eq!(selection["context_window"], json!(262144));
        assert_eq!(selection["temperature"], json!(0.2));
        assert_eq!(selection["top_p"], json!(0.9));
        assert_eq!(selection["reasoning_effort"], json!("high"));
        assert_eq!(selection["model_hints"], json!({ "lacks_vision": true }));
        for excluded in SELECTION_ECHO_EXCLUDED {
            assert!(
                !selection.contains_key(*excluded),
                "`{excluded}` is not caller-owned and would be rejected as an unknown field \
                 (the kernel carries it forward itself)"
            );
        }

        // The route is echoed with the kernel's own keys only.
        let route = selection["route"].as_object().expect("an object");
        assert_eq!(route["route_id"], json!("official"));
        assert_eq!(route["base_url"], json!("https://api.z.ai/api/paas/v4"));
        assert_eq!(route["api_key_env"], json!("ZAI_API_KEY"));
        assert_eq!(route["api_type"], json!("openai"));
        assert!(!route.contains_key("label"), "an absent key stays absent");

        // A route-less candidate asserts no route rather than an empty one.
        let mut routeless = candidate_row(json!({}));
        routeless.as_object_mut().expect("an object").remove("route");
        let routeless = KeyCandidate::from_json(&routeless).expect("a candidate");
        let params = upsert_params(&routeless, FAKE_KEY);
        assert!(
            params["selection"].get("route").is_none(),
            "no route is named, so none is asserted: {params}"
        );
    }

    /// **The security-relevant shape of the two destinations.** On the keychain
    /// path the payload carries a marker and the key is absent from the wire
    /// entirely; on the plaintext path the key *is* the payload, which is why no
    /// frame on this path may ever be logged.
    #[test]
    fn the_keychain_destination_never_puts_the_key_on_the_wire() {
        let candidate = candidate(json!({}));
        let account = octos_cli::auth::keychain::scoped_account(
            "ZAI_API_KEY",
            octos_core::MAIN_PROFILE_ID,
        );
        let marker = octos_cli::auth::keychain::marker_for(&account);
        assert!(
            octos_cli::auth::keychain::is_marker(&marker),
            "the profile stores a marker, and the kernel resolves it on read"
        );

        let keychain_wire = upsert_params(&candidate, &marker).to_string();
        assert!(
            !keychain_wire.contains(FAKE_KEY),
            "the keychain destination must never carry the secret on the wire"
        );
        assert!(keychain_wire.contains(&marker), "{keychain_wire}");

        let plaintext_wire = upsert_params(&candidate, FAKE_KEY).to_string();
        assert!(
            plaintext_wire.contains(FAKE_KEY),
            "the plaintext destination necessarily carries the secret — therefore no frame \
             on this path is ever logged"
        );
        assert!(plaintext_wire.contains("ZAI_API_KEY"), "{plaintext_wire}");
    }

    /// The copy's wire, against a fake kernel: the payload addresses the model the
    /// kernel listed and reports where the key landed without ever naming it.
    #[tokio::test]
    async fn a_copy_addresses_the_kernels_own_model_and_says_where_the_key_landed() {
        use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

        let (host, kernel) = tokio::io::duplex(64 * 1024);
        let (connection, _incoming) = Connection::new(host);
        let seen: std::sync::Arc<std::sync::Mutex<Vec<Value>>> = Default::default();
        let recorded = seen.clone();
        tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(kernel);
            let mut lines = tokio::io::BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let request: Value = serde_json::from_str(&line).expect("request frame");
                if request["method"] == json!(PROFILE_LLM_UPSERT) {
                    recorded
                        .lock()
                        .expect("recorded")
                        .push(request["params"].clone());
                }
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": { "profile_id": "_main", "applied": true },
                });
                writer
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .expect("write reply");
            }
        });

        let candidate = candidate(json!({}));
        let note = copy(
            &connection,
            &candidate,
            FAKE_KEY,
            CopyDestination::ProfileEnvVarsPlaintext,
        )
        .await
        .expect("the kernel accepted the credential");

        let recorded = seen.lock().expect("recorded").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["selection"]["model_id"], json!("glm-5.3"));
        assert_eq!(
            recorded[0]["selection"]["route"]["api_key_env"],
            json!("ZAI_API_KEY"),
            "the env var name is what the kernel needs to file the key under"
        );

        // The note tells a person what happened and where it landed — and never
        // the secret.
        assert!(note.contains("ZAI_API_KEY"), "{note}");
        assert!(note.contains("plaintext"), "{note}");
        assert!(note.contains("https://api.z.ai/api/paas/v4"), "{note}");
        assert!(
            !note.contains(FAKE_KEY),
            "a note is shown and logged: the key must never be in it"
        );

        // A candidate with no slot is refused rather than guessed at.
        let mut slotless = candidate_row(json!({}));
        slotless["api_key_env"] = Value::Null;
        slotless["route"]["api_key_env"] = Value::Null;
        let slotless = KeyCandidate::from_json(&slotless).expect("a candidate");
        let error = copy(
            &connection,
            &slotless,
            FAKE_KEY,
            CopyDestination::ProfileEnvVarsPlaintext,
        )
        .await
        .expect_err("there is no api_key_env to store it under");
        assert!(error.to_string().contains("no api_key_env"), "{error}");
        assert!(!error.to_string().contains(FAKE_KEY), "{error}");
    }
}
