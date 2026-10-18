//! Tests for the in-process agent-thread backend.
//!
//! What can be verified here is exactly what is *not* the kernel's to decide:
//! the reading of a frame (pure), the session key a thread opens (pure), the
//! turn state machine (pure), and the encoders that produce the frames this host
//! sends. What cannot is a real turn: this machine has no Octos main profile, so
//! no session can be opened at all. See the P2 report.

use std::collections::HashSet;

use agent_client_protocol::schema::v1 as acp;
use octos_core::SessionKey;
use octos_core::ui_protocol::{
    self as oup, InputItem, MessageDeltaEvent, PlanItemStatus, PlanUpdatedEvent,
    ReasoningDeltaEvent, ToolCompletedEvent, ToolStartedEvent, TurnId, TurnStartParams, UiPlanItem,
    UiPlanRecord,
};
use serde::Serialize;
use serde_json::{Value, json};

use super::*;
use crate::zed::octos::{APP_ID, OctosFrame};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The session one thread in these tests holds.
fn session() -> SessionKey {
    thread_session_key("test-thread")
}

/// A kernel frame as `forward_notifications` fans one out: the method the
/// `UiNotification`'s own `into_rpc_notification` would use, and params
/// serialized from the real `octos-core` event type rather than typed as a JSON
/// literal. The point is that the test cannot drift from the wire shape on its
/// own.
fn frame<T: Serialize>(method: &str, params: &T) -> OctosFrame {
    OctosFrame {
        method: method.to_owned(),
        params: serde_json::to_value(params).expect("wire params"),
    }
}

/// A frame from JSON, for the cases that are *about* a hand-typed shape (the
/// fields the kernel does not send).
fn raw_frame(method: &str, params: Value) -> OctosFrame {
    OctosFrame {
        method: method.to_owned(),
        params,
    }
}

fn message_delta(text: &str) -> OctosFrame {
    frame(
        oup::methods::MESSAGE_DELTA,
        &MessageDeltaEvent {
            session_id: session(),
            topic: None,
            turn_id: TurnId::new(),
            text: text.to_owned(),
        },
    )
}

/// One `projection/envelope` frame carrying `payload`.
fn envelope(payload: Value) -> OctosFrame {
    raw_frame(
        oup::methods::PROJECTION_ENVELOPE,
        json!({
            "session_id": session(),
            "thread_id": "t",
            "seq": 1,
            "turn_id": TurnId::new(),
            "payload": payload,
        }),
    )
}

fn text_of_chunk(chunk: &acp::ContentChunk) -> Option<&str> {
    match &chunk.content {
        acp::ContentBlock::Text(text) => Some(&text.text),
        _ => None,
    }
}

/// The text of the single `AgentMessageChunk` an update is, if it is one.
fn agent_text(update: &acp::SessionUpdate) -> Option<&str> {
    match update {
        acp::SessionUpdate::AgentMessageChunk(chunk) => text_of_chunk(chunk),
        _ => None,
    }
}

fn thought_text(update: &acp::SessionUpdate) -> Option<&str> {
    match update {
        acp::SessionUpdate::AgentThoughtChunk(chunk) => text_of_chunk(chunk),
        _ => None,
    }
}

fn tool_call(update: &acp::SessionUpdate) -> Option<&acp::ToolCall> {
    match update {
        acp::SessionUpdate::ToolCall(call) => Some(call),
        _ => None,
    }
}

fn tool_call_update(update: &acp::SessionUpdate) -> Option<&acp::ToolCallUpdate> {
    match update {
        acp::SessionUpdate::ToolCallUpdate(update) => Some(update),
        _ => None,
    }
}

fn plan(update: &acp::SessionUpdate) -> Option<&acp::Plan> {
    match update {
        acp::SessionUpdate::Plan(plan) => Some(plan),
        _ => None,
    }
}

/// The text of the first content block of a tool-call update, which is the one
/// line the kernel previewed.
fn update_text(update: &acp::ToolCallUpdate) -> Option<String> {
    let content = update.fields.content.as_ref()?.first()?;
    match content {
        acp::ToolCallContent::Content(content) => match &content.content {
            acp::ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// A connection over a fake kernel end, for the tests that need one but never
/// send a frame.
fn connection_for_test() -> OctosConnection {
    let (host, _kernel) = tokio::io::duplex(64 * 1024);
    let (connection, _incoming) = crate::zed::octos::connection::Connection::new(host);
    let (fanout, _) = tokio::sync::broadcast::channel::<OctosFrame>(4);
    OctosConnection {
        connection,
        fanout,
        sessions: Default::default(),
    }
}

// ---------------------------------------------------------------------------
// The session key: one thread, one session
// ---------------------------------------------------------------------------

/// The kernel keys a session by this string. Two threads sharing one would make
/// the second thread join the first one's conversation, and its `turn/start`
/// would be refused with `turn_in_progress` for as long as the first thread's
/// turn runs — which is why the third segment carries a per-thread name.
#[test]
fn every_thread_opens_its_own_session_on_the_main_profile() {
    let first = thread_session_key("thread-1");
    let second = thread_session_key("thread-2");

    assert_ne!(first, second, "two threads must not share a session");
    assert!(
        first
            .0
            .starts_with(&format!("{}:{}:", octos_core::MAIN_PROFILE_ID, APP_ID)),
        "the main profile is kept, so a thread runs as the same agent the console does: {}",
        first.0
    );
    assert_eq!(
        first.0,
        format!("{}:{}:thread-1", octos_core::MAIN_PROFILE_ID, APP_ID)
    );

    // The console's own session is a different key, so a thread never renders
    // the console's conversation (and vice versa).
    let console = SessionKey::with_profile(octos_core::MAIN_PROFILE_ID, APP_ID, "embedded");
    assert_ne!(first, console);
}

#[test]
fn thread_names_are_unique_and_not_the_consoles() {
    let names: HashSet<String> = (0..128).map(|_| next_thread_name()).collect();
    assert_eq!(names.len(), 128, "a name is handed out once");
    assert!(names.iter().all(|name| name.starts_with("thread-")));
    assert!(
        !names.contains("embedded"),
        "the console's session name is not a thread name"
    );
}

// ---------------------------------------------------------------------------
// The prompt: text, and only text
// ---------------------------------------------------------------------------

/// A prompt built from several blocks must reach the model whole. Only the first
/// block arriving is the bug this pins — and the binary block must be skipped
/// rather than mis-sent as text.
#[test]
fn a_prompt_of_many_text_blocks_is_joined_in_order() {
    let blocks = vec![
        acp::ContentBlock::Text(acp::TextContent::new("first line")),
        acp::ContentBlock::Image(acp::ImageContent::new("base64data", "image/png")),
        acp::ContentBlock::Text(acp::TextContent::new("second line")),
    ];

    assert_eq!(prompt_text(&blocks), "first line\nsecond line");
}

/// An `@file` mention arrives as a `ResourceLink`. P2 has no media channel, so it
/// degrades to a labelled reference to the path — text the model can act on —
/// instead of being dropped or attached.
#[test]
fn an_at_file_mention_degrades_to_its_path() {
    let link = acp::ResourceLink::new("main.rs", "file:///work/src/main.rs")
        .title("src/main.rs")
        .description("the entry point");
    let blocks = vec![
        acp::ContentBlock::Text(acp::TextContent::new("look at")),
        acp::ContentBlock::ResourceLink(link),
    ];

    assert_eq!(
        prompt_text(&blocks),
        "look at\n[Resource: src/main.rs (file:///work/src/main.rs)]\nthe entry point"
    );
}

/// An embedded text resource is degraded to its text, which is content the model
/// can use. Advertising `embedded_context` is what invites the client to send
/// one, so it must not be dropped.
#[test]
fn an_embedded_text_resource_is_degraded_to_its_text() {
    let blocks = vec![acp::ContentBlock::Resource(acp::EmbeddedResource::new(
        acp::EmbeddedResourceResource::TextResourceContents(acp::TextResourceContents::new(
            "resource body",
            "file:///tmp/x.txt",
        )),
    ))];

    assert_eq!(prompt_text(&blocks), "resource body");
}

/// A prompt of nothing but an image is empty text, and an empty turn must not be
/// sent: the kernel would either refuse it or run a turn with no input.
#[test]
fn a_prompt_with_no_text_at_all_is_empty() {
    let blocks = vec![acp::ContentBlock::Image(acp::ImageContent::new(
        "base64data",
        "image/png",
    ))];

    assert!(prompt_text(&blocks).trim().is_empty());
}

/// The frame this host sends for a turn, decoded back with the kernel's own
/// params type so the wire shape cannot drift: this session, this turn, one text
/// item, and **no** `media` — P2 does not forward attachments.
#[test]
fn a_turn_names_its_own_session_and_carries_one_text_item() {
    let session = session();
    let turn = TurnId::new();

    let params = turn_start_params(&session, &turn, "hello").expect("turn params");
    let decoded: TurnStartParams =
        serde_json::from_value(params.clone()).expect("the kernel's own turn/start params type");

    assert_eq!(decoded.session_id, session);
    assert_eq!(decoded.turn_id, turn);
    assert_eq!(
        decoded.input,
        vec![InputItem::Text {
            text: "hello".to_owned()
        }]
    );
    assert!(decoded.media.is_empty(), "P2 sends no media: {params}");
    assert_eq!(
        params.get("media"),
        None,
        "an empty media list is left off the frame entirely: {params}"
    );
}

// ---------------------------------------------------------------------------
// The translation
// ---------------------------------------------------------------------------

/// A `message/delta` is the authoritative assistant text lane.
#[test]
fn a_message_delta_is_an_agent_message_chunk() {
    let effects = thread_effects(&message_delta("hello"));

    assert_eq!(effects.updates.len(), 1, "{effects:?}");
    assert_eq!(agent_text(&effects.updates[0]), Some("hello"));
    assert_eq!(effects.ended, None);
    assert_eq!(effects.note, None);
}

/// Reasoning streams on its own lane, and must render as a *thought* — not as
/// answer text, which would put the model's thinking in the reply.
#[test]
fn a_reasoning_delta_is_an_agent_thought_chunk() {
    let effects = thread_effects(&frame(
        oup::methods::MESSAGE_REASONING_DELTA,
        &ReasoningDeltaEvent {
            session_id: session(),
            topic: None,
            turn_id: TurnId::new(),
            text: "weighing options".to_owned(),
        },
    ));

    assert_eq!(effects.updates.len(), 1, "{effects:?}");
    assert_eq!(thought_text(&effects.updates[0]), Some("weighing options"));
    assert_eq!(
        agent_text(&effects.updates[0]),
        None,
        "reasoning must not render as answer text"
    );
}

/// A tool lifecycle: `tool/started` opens the entry with a kind the transcript
/// can icon, and `tool/completed` closes it with the outcome as its content.
#[test]
fn a_tool_lifecycle_opens_and_closes_one_entry() {
    let tool_call_id = "call-1";

    let started = thread_effects(&frame(
        oup::methods::TOOL_STARTED,
        &ToolStartedEvent {
            session_id: session(),
            topic: None,
            turn_id: TurnId::new(),
            tool_call_id: tool_call_id.to_owned(),
            tool_name: "read_file".to_owned(),
            arguments: None,
        },
    ));
    assert_eq!(started.updates.len(), 1, "{started:?}");
    let call = tool_call(&started.updates[0]).expect("a ToolCall");
    assert_eq!(call.tool_call_id.0.as_ref(), tool_call_id);
    assert_eq!(call.title, "read_file");
    assert_eq!(call.kind, acp::ToolKind::Read);
    assert_eq!(call.status, acp::ToolCallStatus::InProgress);

    let completed = thread_effects(&frame(
        oup::methods::TOOL_COMPLETED,
        &ToolCompletedEvent {
            session_id: session(),
            topic: None,
            turn_id: TurnId::new(),
            tool_call_id: tool_call_id.to_owned(),
            tool_name: "read_file".to_owned(),
            success: Some(true),
            output_preview: Some("42 lines".to_owned()),
            duration_ms: Some(12),
        },
    ));
    assert_eq!(completed.updates.len(), 1, "{completed:?}");
    let update = tool_call_update(&completed.updates[0]).expect("a ToolCallUpdate");
    assert_eq!(update.tool_call_id.0.as_ref(), tool_call_id);
    assert_eq!(
        update.fields.status,
        Some(acp::ToolCallStatus::Completed),
        "a successful tool completes"
    );
    assert_eq!(update_text(update), Some("42 lines".to_owned()));
    assert_eq!(completed.ended, None, "a tool ending is not a turn ending");
}

/// A failed tool is *failed*, and its reason is what the person sees.
#[test]
fn a_failed_tool_is_failed_and_says_why() {
    let effects = thread_effects(&frame(
        oup::methods::TOOL_COMPLETED,
        &ToolCompletedEvent {
            session_id: session(),
            topic: None,
            turn_id: TurnId::new(),
            tool_call_id: "call-2".to_owned(),
            tool_name: "shell".to_owned(),
            success: Some(false),
            output_preview: None,
            duration_ms: None,
        },
    ));

    let update = tool_call_update(&effects.updates[0]).expect("a ToolCallUpdate");
    assert_eq!(update.fields.status, Some(acp::ToolCallStatus::Failed));
}

/// A plan snapshot becomes an ACP plan, with each item's status carried
/// faithfully.
#[test]
fn a_plan_snapshot_becomes_an_acp_plan() {
    let effects = thread_effects(&frame(
        oup::methods::PLAN_UPDATED,
        &PlanUpdatedEvent {
            session_id: session(),
            topic: None,
            turn_id: Some(TurnId::new()),
            plan: UiPlanRecord {
                items: vec![
                    UiPlanItem {
                        id: "1".to_owned(),
                        title: "read the code".to_owned(),
                        status: PlanItemStatus::Completed,
                        priority: None,
                    },
                    UiPlanItem {
                        id: "2".to_owned(),
                        title: "write the fix".to_owned(),
                        status: PlanItemStatus::InProgress,
                        priority: Some("P1".to_owned()),
                    },
                ],
                title: Some("fixing".to_owned()),
                updated_at_ms: 7,
            },
        },
    ));

    assert_eq!(effects.updates.len(), 1, "{effects:?}");
    let plan = plan(&effects.updates[0]).expect("a Plan");
    assert_eq!(plan.entries.len(), 2, "{plan:?}");
    assert_eq!(plan.entries[0].content, "read the code");
    assert_eq!(plan.entries[0].status, acp::PlanEntryStatus::Completed);
    assert_eq!(plan.entries[1].status, acp::PlanEntryStatus::InProgress);
}

/// **A discriminator.** The OUP plan item's `priority` is a free-form display
/// chip (`"P1"`, `"P3"`, `"high"`), not an ACP priority. Promoting it would
/// render a guess as fact, so every ACP entry is `Medium` whatever the chip says.
#[test]
fn a_plan_chip_is_not_an_acp_priority() {
    fn plan_with_chip(chip: Option<&str>) -> acp::Plan {
        let effects = thread_effects(&frame(
            oup::methods::PLAN_UPDATED,
            &PlanUpdatedEvent {
                session_id: session(),
                topic: None,
                turn_id: None,
                plan: UiPlanRecord {
                    items: vec![UiPlanItem {
                        id: "1".to_owned(),
                        title: "one task".to_owned(),
                        status: PlanItemStatus::Pending,
                        priority: chip.map(str::to_owned),
                    }],
                    title: None,
                    updated_at_ms: 0,
                },
            },
        ));
        plan(&effects.updates[0]).expect("a Plan").clone()
    }

    for chip in [None, Some("P0"), Some("P1"), Some("P3"), Some("high")] {
        assert_eq!(
            plan_with_chip(chip).entries[0].priority,
            acp::PlanEntryPriority::Medium,
            "a chip {chip:?} is not an ACP priority"
        );
    }
}

/// **A discriminator.** The kernel's delta frames carry `text` and carry no
/// `delta` and no `content`. A frame spelled with one of those names is not the
/// frame this host renders, and rendering it anyway is exactly how a transcript
/// grows text the kernel never sent.
#[test]
fn a_delta_frame_is_not_rendered_from_a_field_the_kernel_does_not_send() {
    for wrong in [
        json!({ "session_id": session(), "turn_id": TurnId::new(), "delta": "text" }),
        json!({ "session_id": session(), "turn_id": TurnId::new(), "content": "text" }),
        // No `text` at all.
        json!({ "session_id": session(), "turn_id": TurnId::new() }),
    ] {
        let effects = thread_effects(&raw_frame(oup::methods::MESSAGE_DELTA, wrong.clone()));
        assert!(
            effects.is_empty(),
            "a non-authoritative delta shape rendered something: {wrong} -> {effects:?}"
        );
    }
}

/// **A discriminator.** Since #2318 the projection envelope carries the *same*
/// assistant text that `message/delta` already delivers. Rendering both doubles
/// every answer, so the envelope's `assistant_delta` (and its persisted final
/// form) renders nothing here.
#[test]
fn an_envelope_assistant_delta_does_not_double_the_answer() {
    let delta = thread_effects(&envelope(json!({
        "type": "assistant_delta",
        "data": { "text": "the answer", "assistant_segment_id": "seg-1" },
    })));
    assert!(
        delta.is_empty(),
        "the envelope's assistant text must not be rendered twice: {delta:?}"
    );

    let persisted = thread_effects(&envelope(json!({
        "type": "assistant_persisted",
        "data": {
            "text": "the answer",
            "assistant_segment_id": "seg-1",
            "meta": {},
        },
    })));
    assert!(
        persisted.is_empty(),
        "the persisted copy of the same text must not be rendered twice: {persisted:?}"
    );

    // The raw lane is where the text comes from, and it does render.
    assert_eq!(
        agent_text(&thread_effects(&message_delta("the answer")).updates[0]),
        Some("the answer")
    );
}

/// The envelope's tool lifecycle *is* rendered: for a connection that negotiated
/// `projection.envelope.v1` the kernel suppresses the raw `tool/*` frames, so a
/// client that only reads the raw lane would see no tool activity at all.
#[test]
fn the_envelope_tool_lifecycle_is_rendered() {
    let start = thread_effects(&envelope(json!({
        "type": "tool_start",
        "data": { "tool_call_id": "call-9", "name": "shell" },
    })));
    let call = tool_call(&start.updates[0]).expect("a ToolCall");
    assert_eq!(call.tool_call_id.0.as_ref(), "call-9");
    assert_eq!(call.kind, acp::ToolKind::Execute);

    let end = thread_effects(&envelope(json!({
        "type": "tool_end",
        "data": {
            "tool_call_id": "call-9",
            "status": "complete",
            "output_preview": "ok",
        },
    })));
    let update = tool_call_update(&end.updates[0]).expect("a ToolCallUpdate");
    assert_eq!(update.fields.status, Some(acp::ToolCallStatus::Completed));
    assert_eq!(update_text(update), Some("ok".to_owned()));
}

/// Progress must not be written into the tool entry's *content*: ACP's content
/// list is positional and an update rewrites it in place, so a progress line
/// there would overwrite the tool's output. It is a console note instead.
#[test]
fn tool_progress_is_a_note_and_not_tool_content() {
    let effects = thread_effects(&envelope(json!({
        "type": "tool_progress",
        "data": { "tool_call_id": "call-9", "message": "halfway" },
    })));

    assert!(effects.updates.is_empty(), "{effects:?}");
    assert_eq!(effects.note, Some("halfway".to_owned()));
}

/// A frame this build cannot decode renders nothing. Guessing is how a
/// transcript grows text the kernel never sent.
#[test]
fn an_unknown_frame_renders_nothing() {
    let effects = thread_effects(&raw_frame("octos/something/new", json!({ "text": "hi" })));
    assert!(effects.is_empty(), "{effects:?}");

    // The host's own resync marker is not a kernel frame at all: the delivery
    // loop handles it (it means frames are *gone*), and it must not decode into
    // an update by accident.
    let resync = OctosFrame::resync(3);
    assert!(resync.is_resync());
    assert_eq!(resync.missed_frames(), Some(3));
    assert!(thread_effects(&resync).is_empty());
}

// ---------------------------------------------------------------------------
// The turn terminal
// ---------------------------------------------------------------------------

/// A completed turn ends the turn and carries no transcript update: the text
/// already streamed on `message/delta`.
#[test]
fn turn_completed_ends_the_turn_without_an_update() {
    let turn_id = TurnId::new();
    let effects = thread_effects(&frame(
        oup::methods::TURN_COMPLETED,
        &oup::TurnCompletedEvent {
            session_id: session(),
            topic: None,
            turn_id: turn_id.clone(),
            cursor: None,
            tokens_in: None,
            tokens_out: None,
            session_result: None,
            token_usage: None,
        },
    ));

    assert!(effects.updates.is_empty(), "{effects:?}");
    assert_eq!(effects.ended, Some((Some(turn_id), TurnEnd::Completed)));
}

/// A failed turn ends the turn *and* says what the kernel said: the code and
/// message are the only fields `TurnErrorEvent` carries for a person.
#[test]
fn turn_error_ends_the_turn_with_the_kernels_own_reason() {
    let turn_id = TurnId::new();
    let effects = thread_effects(&frame(
        oup::methods::TURN_ERROR,
        &oup::TurnErrorEvent {
            session_id: session(),
            topic: None,
            turn_id: turn_id.clone(),
            code: "provider_unavailable".to_owned(),
            message: "no route to the provider".to_owned(),
            token_usage: None,
            partial_result: None,
        },
    ));

    assert_eq!(
        effects.ended,
        Some((
            Some(turn_id),
            TurnEnd::Errored(
                "turn failed [provider_unavailable]: no route to the provider".to_owned()
            )
        ))
    );
}

/// The envelope's terminal covers all four outcomes, including the interrupted
/// one, which has no notification of its own.
#[test]
fn the_envelope_terminal_carries_every_outcome() {
    let terminal = |outcome: &str, error: Option<Value>| {
        let mut payload = json!({ "type": "turn_terminal", "data": { "outcome": outcome } });
        if let Some(error) = error {
            payload["data"]["error"] = error;
        }
        thread_effects(&envelope(payload))
    };

    assert_eq!(
        terminal("completed", None).ended,
        Some((None, TurnEnd::Completed))
    );
    assert_eq!(
        terminal("interrupted", None).ended,
        Some((None, TurnEnd::Interrupted))
    );
    assert_eq!(
        terminal("rate_limited", None).ended,
        Some((None, TurnEnd::RateLimited))
    );
    assert_eq!(
        terminal(
            "errored",
            Some(json!({ "code": "boom", "message": "it broke" }))
        )
        .ended,
        Some((
            None,
            TurnEnd::Errored("turn failed [boom]: it broke".to_owned())
        ))
    );
}

/// What a settled turn becomes for the thread waiting on it: an interrupted turn
/// is a *cancelled* prompt, a completed one ends the turn, and a failed one is a
/// failed prompt — not a silent success.
#[test]
fn a_settled_turn_becomes_the_matching_prompt_response() {
    assert_eq!(
        prompt_response(TurnEnd::Completed)
            .expect("a completed turn")
            .stop_reason,
        acp::StopReason::EndTurn
    );
    assert_eq!(
        prompt_response(TurnEnd::RateLimited)
            .expect("a rate-limited turn")
            .stop_reason,
        acp::StopReason::EndTurn
    );
    assert_eq!(
        prompt_response(TurnEnd::Interrupted)
            .expect("an interrupted turn")
            .stop_reason,
        acp::StopReason::Cancelled
    );

    let error = prompt_response(TurnEnd::Errored("it broke".to_owned()))
        .expect_err("a failed turn is a failed prompt");
    assert!(error.to_string().contains("it broke"), "{error}");
}

// ---------------------------------------------------------------------------
// The turn state machine
// ---------------------------------------------------------------------------

/// A session with one thread's key, for the state-machine tests.
fn thread_session() -> ThreadSession {
    ThreadSession::new(session())
}

/// The kernel runs one turn per session and refuses a second `turn/start`. The
/// host refuses it first, and names the turn that is in the way so the person is
/// told something actionable.
#[test]
fn a_second_prompt_is_refused_while_a_turn_is_in_flight() {
    let state = thread_session();
    let first = TurnId::new();
    let (first_end, _first_rx) = oneshot::channel();
    state
        .begin_turn(first.clone(), first_end)
        .expect("the first turn");

    let (second_end, _second_rx) = oneshot::channel();
    let refused = state
        .begin_turn(TurnId::new(), second_end)
        .expect_err("a second turn cannot start");
    assert_eq!(refused, first, "the refusal names the turn in the way");
    assert_eq!(state.turn_in_flight(), Some(first));
}

/// A terminal frame settles the turn exactly once. A second terminal for the same
/// turn (two lanes carry one terminal) must not resolve anything twice.
#[test]
fn a_terminal_frame_settles_the_turn_once() {
    let state = thread_session();
    let turn = TurnId::new();
    let (end_tx, mut end_rx) = oneshot::channel();
    state.begin_turn(turn.clone(), end_tx).expect("a turn");

    assert!(
        state.finish_turn(Some(&turn), TurnEnd::Completed),
        "the turn's own terminal ends it"
    );
    assert_eq!(
        end_rx.try_recv().expect("the channel is open"),
        Some(TurnEnd::Completed)
    );
    assert_eq!(state.turn_in_flight(), None, "nothing is in flight any more");
    assert!(
        !state.finish_turn(Some(&turn), TurnEnd::Completed),
        "a second terminal has nothing to settle"
    );
}

/// A terminal frame that names a *different* turn is not proof that this thread's
/// turn ended. Leaving the prompt unresolved would hang the thread, so it settles
/// as an error that says exactly what was seen.
#[test]
fn a_terminal_for_another_turn_is_reported_rather_than_swallowed() {
    let state = thread_session();
    let mine = TurnId::new();
    let theirs = TurnId::new();
    let (end_tx, mut end_rx) = oneshot::channel();
    state.begin_turn(mine.clone(), end_tx).expect("a turn");

    assert!(state.finish_turn(Some(&theirs), TurnEnd::Completed));
    let end = end_rx
        .try_recv()
        .expect("the channel is open")
        .expect("settled");
    let reason = match end {
        TurnEnd::Errored(reason) => reason,
        other => panic!("a mismatched terminal must not read as a clean completion: {other:?}"),
    };
    assert!(reason.contains(&theirs.0.to_string()), "{reason}");
    assert!(reason.contains(&mine.0.to_string()), "{reason}");
}

/// A terminal frame that names no turn still ends this thread's turn: the kernel
/// runs one turn per session, and the envelope's terminal is one of the frames
/// that names none.
#[test]
fn an_unnamed_terminal_still_ends_the_turn() {
    let state = thread_session();
    let (end_tx, mut end_rx) = oneshot::channel();
    state.begin_turn(TurnId::new(), end_tx).expect("a turn");

    assert!(state.finish_turn(None, TurnEnd::Interrupted));
    assert_eq!(
        end_rx.try_recv().expect("the channel is open"),
        Some(TurnEnd::Interrupted)
    );
}

/// The kernel refused *this* input because it is already running a turn for this
/// session. This prompt must fail — the kernel will never answer it — but the
/// turn the kernel named must stay known, or Stop would have nothing to name.
#[test]
fn a_refused_start_fails_this_prompt_and_keeps_the_running_turn_named() {
    let state = thread_session();
    let mine = TurnId::new();
    let theirs = TurnId::new();
    let (end_tx, mut end_rx) = oneshot::channel();
    state.begin_turn(mine, end_tx).expect("a turn");

    // What the refusal path does: adopt the kernel's turn, then abandon this
    // prompt's waiter.
    state.adopt_turn(theirs.clone());
    let waiter = state.abandon_waiter().expect("a waiter to abandon");
    waiter
        .send(TurnEnd::Errored("already running".to_owned()))
        .expect("the prompt is listening");

    assert_eq!(
        end_rx.try_recv().expect("the channel is open"),
        Some(TurnEnd::Errored("already running".to_owned()))
    );
    assert_eq!(
        state.turn_in_flight(),
        Some(theirs),
        "the turn the kernel is running is still the one Stop names"
    );
    assert!(
        state.abandon_waiter().is_none(),
        "a second abandon has no waiter"
    );
}

/// Nothing in flight means nothing to interrupt: an interrupt is a claim about a
/// named turn, and this host will not make one up.
#[test]
fn there_is_nothing_to_interrupt_with_no_turn_in_flight() {
    let state = thread_session();
    assert_eq!(state.turn_in_flight(), None);
    assert!(!state.begin_cancelling());
}

/// A cancel in flight is not repeated: a second `turn/interrupt` would be a
/// second claim about the same turn.
#[test]
fn a_cancel_in_flight_is_not_sent_twice() {
    let state = thread_session();
    let (end_tx, _end_rx) = oneshot::channel();
    state.begin_turn(TurnId::new(), end_tx).expect("a turn");

    assert!(state.begin_cancelling(), "the first interrupt goes out");
    assert!(!state.begin_cancelling(), "the second is deduped");

    // A receipt that says the kernel did *not* capture it puts the turn back
    // where a later interrupt may name it again.
    state.abort_cancelling();
    assert!(state.begin_cancelling());
}

// ---------------------------------------------------------------------------
// The interrupt receipts
// ---------------------------------------------------------------------------

/// The four receipts of `turn/interrupt` mean four different things, and only one
/// of them is "the kernel is stopping this turn". The console reads them with the
/// same function, so an interrupt the console refuses to call "cancelled" is not
/// called that here either.
#[test]
fn the_four_interrupt_receipts_mean_four_different_things() {
    // 1. Captured.
    assert_eq!(
        interrupt_outcome(&json!({ "result": { "interrupted": true } })),
        InterruptOutcome::Interrupted
    );

    // 2. The turn had already ended. Nothing was interrupted *now*.
    assert_eq!(
        interrupt_outcome(&json!({
            "result": { "interrupted": false, "terminal_state": "completed" }
        })),
        InterruptOutcome::AlreadyOver {
            terminal_state: "completed".to_owned()
        }
    );

    // 3. Declined, with the kernel's own reason.
    assert_eq!(
        interrupt_outcome(&json!({
            "result": { "interrupted": false, "reason": "turn_id_mismatch" }
        })),
        InterruptOutcome::Declined {
            reason: Some("turn_id_mismatch".to_owned())
        }
    );

    // 4. Not a `TurnInterruptResult` at all: this host must not guess.
    assert_eq!(
        interrupt_outcome(&json!({ "result": { "ok": true } })),
        InterruptOutcome::Unreadable
    );
    assert_eq!(interrupt_outcome(&json!({})), InterruptOutcome::Unreadable);
}

/// Only the "already over" receipt settles the turn without a terminal frame.
/// That is the case where the terminal frame was lost, and leaving the thread
/// generating forever would be the alternative.
#[test]
fn only_an_already_over_receipt_settles_the_turn_without_a_terminal_frame() {
    let state = thread_session();
    let (end_tx, mut end_rx) = oneshot::channel();
    state.begin_turn(TurnId::new(), end_tx).expect("a turn");

    // A declined receipt leaves the turn in flight: it is still the kernel's.
    state.abort_cancelling();
    assert!(state.turn_in_flight().is_some());
    assert!(matches!(end_rx.try_recv(), Ok(None)), "still waiting");

    // The "already over" receipt settles it as interrupted — which, for a turn
    // that had ended before the interrupt arrived, is what the panel shows.
    state.finish_turn(None, TurnEnd::Interrupted);
    assert_eq!(
        end_rx.try_recv().expect("the channel is open"),
        Some(TurnEnd::Interrupted)
    );
}

// ---------------------------------------------------------------------------
// `turn/start` refusals
// ---------------------------------------------------------------------------

/// The typed `turn_in_progress` refusal names the running turn
/// (`turn_in_progress_refusal`,
/// `crates/octos/octos-cli/src/api/ui_protocol_transport.rs:27653`), and that id
/// is decoded with `TurnId`'s own `Deserialize` rather than parsed by hand.
#[test]
fn a_turn_in_progress_refusal_names_the_running_turn() {
    let running = TurnId::new();
    let refusal = turn_refusal(&json!({
        "code": -32600,
        "message": "a turn is already running for this session",
        "data": { "kind": "turn_in_progress", "turn_id": running },
    }));

    assert_eq!(
        refusal,
        TurnRefusal::TurnInProgress {
            turn_id: running,
            message: "a turn is already running for this session".to_owned(),
        }
    );
    assert_eq!(
        refusal.message(),
        "a turn is already running for this session"
    );
}

/// Any other refusal names no turn, so nothing of this host's is in flight and
/// the turn state is cleared rather than adopting a turn that was never named.
#[test]
fn another_refusal_names_no_turn() {
    for error in [
        json!({ "code": -32000, "message": "runtime unavailable" }),
        // The right kind, but no id: a refusal that names no turn cannot be
        // interrupted, so it must not be adopted.
        json!({ "code": -32600, "message": "busy", "data": { "kind": "turn_in_progress" } }),
        // The right id, but not a turn id.
        json!({ "code": -32600, "message": "busy", "data": { "kind": "turn_in_progress", "turn_id": "not-a-uuid" } }),
        json!({ "code": -32600, "message": "busy", "data": { "kind": "something_else", "turn_id": TurnId::new() } }),
    ] {
        match turn_refusal(&error) {
            TurnRefusal::Refused { message } => assert!(!message.is_empty(), "{error}"),
            other => panic!("{error} named a turn it should not have: {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// `session/open` and `session/title.set`
// ---------------------------------------------------------------------------

/// The opened session is read through the kernel's own `SessionOpenResult`, not
/// by reaching into `result["opened"]["session_id"]` by hand.
#[test]
fn the_opened_session_is_read_from_the_kernels_own_reply_type() {
    let opened = session();
    let reply = json!({
        "jsonrpc": "2.0",
        "id": "1",
        "result": { "opened": { "session_id": opened } },
    });

    // The hand-written fixture is itself checked against the kernel's type, so it
    // cannot drift from the reply shape this host actually decodes.
    let decoded: oup::SessionOpenResult =
        serde_json::from_value(reply["result"].clone()).expect("a SessionOpenResult");
    assert_eq!(decoded.opened.session_id, opened);

    assert_eq!(session_open_result(&reply), Ok(opened));
}

/// A refusal and an unreadable reply are both reported with what was actually
/// there — never as an opened session.
#[test]
fn a_refused_or_unreadable_session_open_is_an_error() {
    let refused = session_open_result(&json!({
        "jsonrpc": "2.0",
        "id": "1",
        "error": { "code": -32000, "message": "Configure the local LLM profile" },
    }))
    .expect_err("a refusal is not an opened session");
    assert!(
        refused.contains("Configure the local LLM profile"),
        "{refused}"
    );

    let unreadable =
        session_open_result(&json!({ "result": { "opened": {} } })).expect_err("no session id");
    assert!(unreadable.contains("SessionOpenResult"), "{unreadable}");

    let empty = session_open_result(&json!({})).expect_err("no result at all");
    assert!(empty.contains("no result"), "{empty}");
}

/// The title frame is the kernel's own params type, and the title is clamped to
/// the kernel's cap **by characters**: sending more would be refused, and a
/// byte-truncated string would not be the title the person typed.
#[test]
fn a_title_is_clamped_to_the_kernels_cap_by_characters() {
    let session = session();

    let short = title_set_params(&session, "a name").expect("title params");
    let decoded: oup::SessionTitleSetParams =
        serde_json::from_value(short).expect("the kernel's own title params type");
    assert_eq!(
        decoded.session_id, session.0,
        "the wire string, which is what this RPC's own type asks for"
    );
    assert_eq!(decoded.title, "a name");

    // A multi-byte character: truncating by bytes would split one and send a
    // string that is not the title.
    let long: String = "\u{e9}".repeat(oup::SESSION_TITLE_SET_MAX_CHARS + 40);
    let clamped = title_set_params(&session, &long).expect("title params");
    let decoded: oup::SessionTitleSetParams =
        serde_json::from_value(clamped).expect("title params");
    assert_eq!(
        decoded.title.chars().count(),
        oup::SESSION_TITLE_SET_MAX_CHARS,
        "clamped to the kernel's cap"
    );
    assert_eq!(
        decoded.title,
        "\u{e9}".repeat(oup::SESSION_TITLE_SET_MAX_CHARS)
    );
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

/// The fan-out is process-wide, so every thread must attribute a frame to a
/// session before rendering it — including the console's own session, and
/// including a frame from a *third* session nobody in this process opened.
#[test]
fn a_frame_is_attributed_to_its_session() {
    assert_eq!(frame_session(&message_delta("hi")), Some(session()));

    let other = raw_frame(
        oup::methods::MESSAGE_DELTA,
        serde_json::to_value(&MessageDeltaEvent {
            session_id: thread_session_key("someone-else"),
            topic: None,
            turn_id: TurnId::new(),
            text: "not mine".to_owned(),
        })
        .expect("params"),
    );
    assert_ne!(frame_session(&other), Some(session()));

    // A frame this build cannot decode is attributed to nothing, rather than to
    // every thread.
    assert_eq!(
        frame_session(&raw_frame("octos/something/new", json!({}))),
        None
    );
}

/// The ACP session id a thread is keyed by is the kernel's own session string:
/// the connection store, the thread, and the frames all name one session.
#[test]
fn the_acp_session_id_is_the_kernels_session_string() {
    let session = session();
    let acp_session_id = acp::SessionId::new(session.0.clone());
    assert_eq!(acp_session_id.0.as_ref(), session.0);
}

/// The connection reports the same agent id the server does, so the panel keys
/// one entry for one agent.
#[tokio::test]
async fn the_connection_names_the_octos_agent() {
    let octos = connection_for_test();

    assert_eq!(
        octos.agent_id(),
        project::AgentId::new(agent_ui::octos::OCTOS_AGENT_ID)
    );
    assert_eq!(octos.telemetry_id().as_ref(), "octos");
    assert!(octos.auth_methods().is_empty(), "the kernel needs no auth");
}

/// A session id this connection never opened has nowhere to go, and every entry
/// point says so rather than sending a frame at a guess.
#[tokio::test]
async fn a_session_this_connection_does_not_have_is_an_error() {
    let octos = connection_for_test();
    let unknown = acp::SessionId::new("never-opened");

    let error = match octos.session_for(&unknown) {
        Ok(_) => panic!("nothing is registered for it"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("never-opened"), "{error}");
}

// ---------------------------------------------------------------------------
// Hydration (UPCR-2026-009)
// ---------------------------------------------------------------------------

/// One hydrated transcript row, built from the protocol's own type.
fn row(seq: u64, role: &str, content: &str, thread: Option<&str>) -> oup::HydratedMessage {
    oup::HydratedMessage {
        seq,
        role: role.to_owned(),
        content: content.to_owned(),
        turn_id: None,
        thread_id: thread.map(str::to_owned),
        client_message_id: None,
        persisted_at: chrono::Utc::now(),
        reasoning_content: None,
        message_id: None,
        source: None,
        media: Vec::new(),
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
    }
}

/// A `session/hydrate` result with the sections a thread asks for, and nothing
/// else: every optional section is omitted, which is what the kernel does for a
/// section the client did not request.
fn hydration(cursor: u64) -> SessionHydrateResult {
    SessionHydrateResult {
        session_id: session(),
        cursor: UiCursor {
            stream: "ledger".to_owned(),
            seq: cursor,
        },
        context: None,
        context_state: None,
        messages: None,
        threads: None,
        turns: None,
        pending_approvals: None,
        pending_questions: None,
        replayed_envelopes: None,
        replayed_tool_envelopes: None,
        replayed_projection_envelopes: None,
        projection_thread_sequences: None,
    }
}

/// One replayed tool envelope: a tool started and finished, as the canonical v2
/// lane carries it (`projection/envelope.v2`).
fn tool_envelope(seq: u64, thread: &str, tool_call_id: &str) -> oup::EnvelopeV2 {
    oup::EnvelopeV2 {
        thread_id: thread.to_owned(),
        seq,
        cursor: None,
        turn_id: TurnId::new().0.to_string(),
        client_message_id: None,
        payload: PayloadV2::ToolEnd {
            tool_call_id: tool_call_id.to_owned(),
            status: EnvelopeToolEndStatus::Complete,
            error: None,
            reason: None,
            output_preview: Some("ok".to_owned()),
            duration_ms: None,
        },
    }
}

/// The whole replay order, which is the part of a hydrate where a wrong guess is
/// silent: a person would simply see their tools below the answer that used
/// them, or see a tool row rendered as prose.
#[test]
fn hydration_replays_the_transcript_in_order_and_puts_tools_before_their_answer() {
    let mut history = hydration(9);
    history.messages = Some(vec![
        row(1, "user", "hello", Some("t1")),
        // Reasoning with no answer text of its own: a thought block, and the
        // thought comes before anything the row itself says.
        oup::HydratedMessage {
            reasoning_content: Some("weighing it up".to_owned()),
            ..row(2, "assistant", "", Some("t1"))
        },
        // The last answer of `t1`: its tools belong above this row's updates.
        oup::HydratedMessage {
            reasoning_content: Some("deciding".to_owned()),
            ..row(3, "assistant", "final answer", Some("t1"))
        },
        // A tool *row* renders nothing at all: the card comes from the
        // envelopes, and rendering the row would print the output as prose.
        row(4, "tool", "some tool output", Some("t1")),
    ]);
    history.replayed_tool_envelopes = Some(vec![
        tool_envelope(1, "t1", "call-1"),
        // A thread with no answer in the window keeps its envelope to the end.
        tool_envelope(2, "t2", "call-2"),
    ]);

    let updates = hydrate_updates(&history);

    let shape: Vec<String> = updates
        .iter()
        .map(|update| match update {
            acp::SessionUpdate::UserMessageChunk(chunk) => {
                format!("user:{}", text_of_chunk(chunk).unwrap_or_default())
            }
            acp::SessionUpdate::AgentThoughtChunk(chunk) => {
                format!("thought:{}", text_of_chunk(chunk).unwrap_or_default())
            }
            acp::SessionUpdate::AgentMessageChunk(chunk) => {
                format!("assistant:{}", text_of_chunk(chunk).unwrap_or_default())
            }
            acp::SessionUpdate::ToolCall(call) => {
                format!("tool-start:{}", call.tool_call_id.0)
            }
            acp::SessionUpdate::ToolCallUpdate(update) => {
                format!("tool-end:{}", update.tool_call_id.0)
            }
            other => panic!("the replay produced an update it should not: {other:?}"),
        })
        .collect();

    assert_eq!(
        shape,
        vec![
            "user:hello",
            "thought:weighing it up",
            // `t1`'s tools, inserted before `t1`'s last answer...
            "tool-end:call-1",
            "thought:deciding",
            "assistant:final answer",
            // ...and nothing for the tool row...
            // ...while `t2`'s tool, which has no answer here, comes last.
            "tool-end:call-2",
        ],
        "the replay order is the load path's whole content"
    );

    // Said again, positionally: the tool card is *above* the answer it belongs
    // to, not below the conversation.
    let tool_at = updates
        .iter()
        .position(|update| matches!(update, acp::SessionUpdate::ToolCallUpdate(_)))
        .expect("the tool envelope was replayed");
    let answer_at = updates
        .iter()
        .position(|update| agent_text(update) == Some("final answer"))
        .expect("the answer was replayed");
    assert!(tool_at < answer_at, "{tool_at} < {answer_at}");
    // The `tool` role row contributed nothing: the only updates are the five
    // above, and no update carries its text.
    assert!(
        !updates.iter().any(|update| match update {
            acp::SessionUpdate::AgentMessageChunk(chunk) =>
                text_of_chunk(chunk) == Some("some tool output"),
            acp::SessionUpdate::UserMessageChunk(chunk) =>
                text_of_chunk(chunk) == Some("some tool output"),
            _ => false,
        }),
        "a tool row is not transcript text"
    );
}

/// A hydrate is asked for the four sections a thread renders, as the protocol's
/// own tokens, and for the delta after the thread's checkpoint.
#[test]
fn hydration_asks_for_the_four_sections_after_the_checkpoint() {
    let after = UiCursor {
        stream: "ledger".to_owned(),
        seq: 7,
    };
    let params = hydrate_params(&session(), Some(after.clone())).expect("hydrate params");

    assert_eq!(params["session_id"], json!(session().0));
    assert_eq!(params["after"], serde_json::to_value(&after).unwrap());
    let include: Vec<&str> = params["include"]
        .as_array()
        .expect("include is a list")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        include,
        vec![
            hydrate_sections::MESSAGES,
            hydrate_sections::THREADS,
            hydrate_sections::TURNS,
            hydrate_sections::PENDING_APPROVALS,
        ]
    );

    // No checkpoint: the whole session, and `after` is not sent at all rather
    // than sent as null.
    let full = hydrate_params(&session(), None).expect("hydrate params");
    assert!(full.get("after").is_none(), "{full}");
}

/// The replay and real-time delivery must not overlap. Only a frame that carries
/// its own cursor can be judged, and only against its own stream.
#[test]
fn a_frame_the_hydrate_already_covered_is_recognised() {
    let cursor = UiCursor {
        stream: "ledger".to_owned(),
        seq: 10,
    };

    let completed = |cursor: Option<UiCursor>| {
        frame(
            oup::methods::TURN_COMPLETED,
            &oup::TurnCompletedEvent {
                session_id: session(),
                topic: None,
                turn_id: TurnId::new(),
                cursor,
                tokens_in: None,
                tokens_out: None,
                session_result: None,
                token_usage: None,
            },
        )
    };
    let at = |stream: &str, seq: u64| {
        Some(UiCursor {
            stream: stream.to_owned(),
            seq,
        })
    };

    assert!(
        replayed_by_hydrate(&completed(at("ledger", 10)), &cursor),
        "the cursor itself was part of the hydrate's reply"
    );
    assert!(
        replayed_by_hydrate(&completed(at("ledger", 9)), &cursor),
        "older is covered"
    );
    assert!(
        !replayed_by_hydrate(&completed(at("ledger", 11)), &cursor),
        "a newer record is live"
    );
    assert!(
        !replayed_by_hydrate(&completed(at("other", 3)), &cursor),
        "two streams' sequence numbers are not comparable"
    );
    assert!(
        replayed_by_hydrate(
            &envelope_with_cursor(at("ledger", 4), json!({ "type": "tool_progress", "data": { "tool_call_id": "c", "message": "m" } })),
            &cursor
        ),
        "an envelope the replay covered is recognised too"
    );
    assert!(
        !replayed_by_hydrate(
            &envelope_with_cursor(at("ledger", 40), json!({ "type": "tool_progress", "data": { "tool_call_id": "c", "message": "m" } })),
            &cursor
        ),
        "and a newer envelope is live"
    );
    assert!(
        !replayed_by_hydrate(&completed(None), &cursor),
        "a frame with no cursor cannot be judged, so it is applied"
    );
    assert!(
        !replayed_by_hydrate(&message_delta("live text"), &cursor),
        "a delta carries no cursor and is live by construction"
    );
}

/// The hydrate's `pending_approvals` section is read into the ledger, once: a
/// live `approval/requested` for the same approval must not raise a second card.
#[test]
fn hydration_rebuilds_pending_approvals_once() {
    let approval = ApprovalRequestedEvent::generic(
        session(),
        ApprovalId::new(),
        TurnId::new(),
        "shell",
        "Run a command?",
        "rm -rf /tmp/x",
    );
    let mut history = hydration(12);
    history.pending_approvals = Some(vec![approval.clone()]);

    let convergence = converge(&history);
    assert_eq!(convergence.approvals, vec![approval.clone()]);
    assert_eq!(
        convergence.cursor,
        history.cursor,
        "the checkpoint comes from the reply, not from the host"
    );

    let ledger = ThreadSession::new(session());
    assert!(ledger.note_approval(&approval), "the card goes up");
    assert!(
        !ledger.note_approval(&approval),
        "the same approval must not be raised twice"
    );
    assert!(
        ledger
            .approvals
            .lock()
            .expect("approvals")
            .get(&approval.approval_id)
            .is_some(),
        "and it is still waiting"
    );
}

// ---------------------------------------------------------------------------
// Approvals: intent is not a decision
// ---------------------------------------------------------------------------

fn approval_event(tool: &str) -> ApprovalRequestedEvent {
    ApprovalRequestedEvent::generic(
        session(),
        ApprovalId::new(),
        TurnId::new(),
        tool,
        "Run a command?",
        "cargo test",
    )
}

/// The honesty rule, as a test: clicking *Allow* asks the kernel and records what
/// the person asked for. It does not authorize anything, and it is not a
/// decision — only the kernel's own settlement is.
#[test]
fn an_allow_click_is_intent_and_not_an_authorization() {
    let mut ledger = ApprovalLedger::default();
    let event = approval_event("shell");
    let id = event.approval_id.clone();
    assert!(ledger.note_requested(&event));

    let claimed = ledger
        .note_intent(&id, ApprovalIntent::Allow)
        .expect("the click claims the right to answer");
    assert_eq!(claimed.intent, Some(ApprovalIntent::Allow));
    assert_eq!(claimed.tool, "shell");

    // Nothing has been decided: the entry is still waiting, and the kernel has
    // not said anything.
    assert!(
        ledger.settlement(&id).is_none(),
        "no decision has arrived, so none may be recorded"
    );
    assert_eq!(
        ledger.get(&id).map(|entry| entry.intent),
        Some(Some(ApprovalIntent::Allow)),
        "the click is recorded as intent, and the approval is still open"
    );

    // A second click on the same card answers nothing: one approval, one answer.
    assert!(
        ledger.note_intent(&id, ApprovalIntent::Deny).is_none(),
        "a second click must not send a second answer"
    );

    // And when the kernel does settle it, the entry closes with *its* decision.
    let settled = ledger
        .settle(&id, ApprovalSettlement::Denied)
        .expect("the card was still waiting");
    assert_eq!(settled.intent, Some(ApprovalIntent::Allow));
    assert_eq!(ledger.settlement(&id), Some(ApprovalSettlement::Denied));
    assert!(ledger.get(&id).is_none(), "the entry is closed");
    assert!(
        ledger.note_intent(&id, ApprovalIntent::Allow).is_none(),
        "an answered approval cannot be answered again"
    );
}

/// `cancelled` is not a denial: the kernel withdrew the question, and the host
/// records exactly that. An approval settled *before* anyone clicked must not be
/// answerable afterwards, which is the race the ledger exists for.
#[test]
fn a_cancelled_approval_settles_as_cancelled_and_closes_the_question() {
    let mut ledger = ApprovalLedger::default();
    let event = approval_event("write_file");
    let id = event.approval_id.clone();
    assert!(ledger.note_requested(&event));

    // The kernel cancelled it while the card was still on screen.
    let card = ledger
        .settle(&id, ApprovalSettlement::Cancelled)
        .expect("the card was up");
    assert_eq!(card.intent, None, "nobody had answered it");
    assert_eq!(ledger.settlement(&id), Some(ApprovalSettlement::Cancelled));
    assert!(
        ledger.note_intent(&id, ApprovalIntent::Deny).is_none(),
        "a cancelled approval is no longer open: a denial here would be a \
         decision the kernel never asked for"
    );
    assert!(
        !ledger.note_requested(&event),
        "and a replayed request for it must not re-open a closed question"
    );
}

/// An auto-resolved approval carries the kernel's decision like any other, and a
/// decision this build does not understand is *not* a settlement: recording one
/// would put a meaning on a string the kernel has not defined.
#[test]
fn auto_resolution_carries_the_kernels_decision_and_unknown_ones_do_not() {
    assert_eq!(
        settlement_of(&ApprovalDecision::Approve),
        Some(ApprovalSettlement::Approved)
    );
    assert_eq!(
        settlement_of(&ApprovalDecision::Deny),
        Some(ApprovalSettlement::Denied)
    );
    assert_eq!(
        settlement_of(&ApprovalDecision::Unknown("escalated".to_owned())),
        None
    );

    let mut ledger = ApprovalLedger::default();
    let event = approval_event("shell");
    let id = event.approval_id.clone();
    assert!(ledger.note_requested(&event));
    let settled = ledger
        .settle(&id, ApprovalSettlement::Approved)
        .expect("the card was up");
    assert_eq!(settled.intent, None);
    assert_eq!(ledger.settlement(&id), Some(ApprovalSettlement::Approved));
}

/// A session that ends leaves no card hanging: the entries come out of the
/// ledger, and nothing claims they were decided.
#[test]
fn ending_a_session_abandons_its_open_approvals() {
    let ledger = ThreadSession::new(session());
    let first = approval_event("shell");
    let second = approval_event("write_file");
    assert!(ledger.note_approval(&first));
    assert!(ledger.note_approval(&second));

    let abandoned = ledger.abandon_approvals();
    assert_eq!(abandoned.len(), 2);
    let ids: HashSet<String> = abandoned
        .iter()
        .map(|entry| entry.approval_id.0.to_string())
        .collect();
    assert!(ids.contains(&first.approval_id.0.to_string()));
    assert!(ids.contains(&second.approval_id.0.to_string()));
    assert!(ledger.abandon_approvals().is_empty(), "only once");
}

/// The frames: a request becomes a card, and each of the kernel's three
/// settlements is read as *its* outcome — `cancelled` is not a denial.
#[test]
fn approval_frames_map_to_cards_and_to_the_kernels_own_settlements() {
    let approval = approval_event("shell");
    let requested = thread_effects(&frame(oup::methods::APPROVAL_REQUESTED, &approval));
    assert_eq!(
        requested.approval.as_deref(),
        Some(&approval),
        "the card is the kernel's request"
    );
    assert!(requested.settled.is_none());

    let decided = thread_effects(&frame(
        oup::methods::APPROVAL_DECIDED,
        &oup::ApprovalDecidedEvent::manual(
            session(),
            approval.approval_id.clone(),
            TurnId::new(),
            ApprovalDecision::Approve,
            "policy",
        ),
    ));
    assert_eq!(
        decided.settled,
        Some((approval.approval_id.clone(), ApprovalSettlement::Approved))
    );
    assert!(decided.approval.is_none());

    let auto = thread_effects(&frame(
        oup::methods::APPROVAL_AUTO_RESOLVED,
        &oup::ApprovalAutoResolvedEvent {
            session_id: session(),
            topic: None,
            approval_id: approval.approval_id.clone(),
            turn_id: TurnId::new(),
            tool_name: "shell".to_owned(),
            scope: "session".to_owned(),
            scope_match: "always".to_owned(),
            decision: ApprovalDecision::Deny,
        },
    ));
    assert_eq!(
        auto.settled,
        Some((approval.approval_id.clone(), ApprovalSettlement::Denied))
    );

    let cancelled = thread_effects(&frame(
        oup::methods::APPROVAL_CANCELLED,
        &oup::ApprovalCancelledEvent::turn_interrupted(
            session(),
            approval.approval_id.clone(),
            TurnId::new(),
        ),
    ));
    assert_eq!(
        cancelled.settled,
        Some((approval.approval_id.clone(), ApprovalSettlement::Cancelled)),
        "cancelled is its own outcome, never folded into a denial"
    );
    assert!(
        cancelled.note.is_some(),
        "the kernel's reason is said out loud"
    );

    // The card offers exactly the ACP backend's two options, verbatim: two
    // clients must answer one approval the same way.
    let PermissionOptions::Flat(options) = approval_options() else {
        panic!("the thread's card is a flat pair of options");
    };
    let ids: Vec<&str> = options
        .iter()
        .map(|option| option.option_id.0.as_ref())
        .collect();
    assert_eq!(ids, vec![APPROVAL_ALLOW_ONCE, APPROVAL_DENY]);
    assert_eq!(options[0].kind, acp::PermissionOptionKind::AllowOnce);
    assert_eq!(options[1].kind, acp::PermissionOptionKind::RejectOnce);

    let card = approval_tool_call(&approval);
    assert_eq!(card.tool_call_id.0.as_ref(), approval.approval_id.0.to_string());
    assert_eq!(card.fields.title.as_deref(), Some("Run a command?"));
}

// ---------------------------------------------------------------------------
// Resynchronization
// ---------------------------------------------------------------------------

/// One `projection/envelope` frame with its own ledger cursor, for the cursor
/// tests. The cursor is a *frame* field (the envelope's flattened wire shape),
/// not part of the payload.
fn envelope_with_cursor(cursor: Option<UiCursor>, payload: Value) -> OctosFrame {
    raw_frame(
        oup::methods::PROJECTION_ENVELOPE,
        json!({
            "session_id": session(),
            "thread_id": "t",
            "seq": 1,
            "turn_id": TurnId::new(),
            "cursor": cursor,
            "payload": payload,
        }),
    )
}

/// The lost-turn case, read from a hydrate: the kernel says the turn is over,
/// and that is what the thread converges to. A turn the kernel is still running
/// is adopted instead, so Stop can name it.
#[test]
fn a_hydrate_converges_the_turn_after_lost_frames() {
    let turn = TurnId::new();
    let mut history = hydration(20);
    history.turns = Some(vec![oup::HydratedTurn {
        turn_id: turn.clone(),
        state: TurnLifecycleState::Completed,
        started_at: None,
        completed_at: None,
        thread_id: None,
    }]);
    history.messages = Some(vec![row(1, "user", "hi", Some("t1"))]);
    history.pending_approvals = Some(vec![approval_event("shell")]);

    let convergence = converge(&history);
    assert_eq!(
        convergence.ended,
        Some((turn.clone(), TurnEnd::Completed)),
        "a lost turn/completed is exactly what a resync must recover"
    );
    assert_eq!(convergence.running, None);
    assert_eq!(convergence.messages_held, 1);
    assert_eq!(convergence.approvals.len(), 1);

    let running = TurnId::new();
    let mut history = hydration(21);
    history.turns = Some(vec![oup::HydratedTurn {
        turn_id: running.clone(),
        state: TurnLifecycleState::Active,
        started_at: None,
        completed_at: None,
        thread_id: None,
    }]);
    let convergence = converge(&history);
    assert_eq!(convergence.ended, None);
    assert_eq!(convergence.running, Some(running));

    // A state the kernel cannot name is not evidence of anything.
    let mut history = hydration(22);
    history.turns = Some(vec![oup::HydratedTurn {
        turn_id: TurnId::new(),
        state: TurnLifecycleState::Unknown,
        started_at: None,
        completed_at: None,
        thread_id: None,
    }]);
    let convergence = converge(&history);
    assert_eq!(convergence.ended, None);
    assert_eq!(convergence.running, None);
}

/// The resync's wire, against a fake kernel: the request is `session/hydrate`
/// for **this thread's** session and **its own checkpoint**, the reply is decoded
/// with the kernel's type, and the lost terminal converges from it.
#[tokio::test]
async fn a_resync_hydrates_the_same_session_from_the_threads_checkpoint() {
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

    let (host, kernel) = tokio::io::duplex(64 * 1024);
    let (connection, _incoming) = crate::zed::octos::connection::Connection::new(host);

    // What this fake kernel answers a hydrate with: the turn the thread lost the
    // terminal for, one transcript row, and one approval still pending.
    let turn = TurnId::new();
    let mut answer = hydration(31);
    answer.turns = Some(vec![oup::HydratedTurn {
        turn_id: turn.clone(),
        state: TurnLifecycleState::Completed,
        started_at: None,
        completed_at: None,
        thread_id: None,
    }]);
    answer.messages = Some(vec![row(1, "user", "hi", Some("t1"))]);
    answer.pending_approvals = Some(vec![approval_event("shell")]);
    let reply = serde_json::to_value(&answer).expect("the hydrate result serializes");
    let seen: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));

    let kernel_seen = seen.clone();
    tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(kernel);
        let mut lines = tokio::io::BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let request: Value = serde_json::from_str(&line).expect("request frame");
            let method = request["method"].as_str().unwrap_or_default();
            let mut response = json!({ "jsonrpc": "2.0", "id": request["id"], "result": {} });
            if method == oup::methods::SESSION_HYDRATE {
                *kernel_seen.lock().expect("seen params") = Some(request["params"].clone());
                response["result"] = reply.clone();
            }
            writer
                .write_all(format!("{response}\n").as_bytes())
                .await
                .expect("write reply");
        }
    });

    let thread = ThreadSession::new(session());
    let checkpoint = UiCursor {
        stream: "ledger".to_owned(),
        seq: 7,
    };
    thread.set_cursor(checkpoint.clone());

    // Exactly the call a resync makes: this session, this checkpoint.
    let history = hydrate(&connection, &thread.session, thread.cursor())
        .await
        .expect("a hydrate result");
    let convergence = converge(&history);

    let params = seen
        .lock()
        .expect("seen params")
        .clone()
        .expect("the kernel saw the request");
    assert_eq!(params["session_id"], json!(session().0));
    assert_eq!(
        params["after"],
        serde_json::to_value(&checkpoint).unwrap(),
        "a resync asks only for what came after its checkpoint"
    );

    assert_eq!(
        convergence.ended.map(|(_, end)| end),
        Some(TurnEnd::Completed),
        "the lost turn/completed is recovered from the session itself"
    );
    assert_eq!(convergence.cursor.seq, 31, "the new checkpoint is kept");
    assert_eq!(convergence.messages_held, 1);
    assert_eq!(convergence.approvals.len(), 1);

    // And the replay the resync would apply to an empty thread is the kernel's
    // own transcript, in order.
    let replayed = hydrate_updates(&history);
    assert_eq!(replayed.len(), 1, "{replayed:?}");
    assert_eq!(
        text_of_chunk(match &replayed[0] {
            acp::SessionUpdate::UserMessageChunk(chunk) => chunk,
            other => panic!("the transcript row is a user message, not {other:?}"),
        }),
        Some("hi")
    );
}

// ---------------------------------------------------------------------------
// Session history (`session/list`)
// ---------------------------------------------------------------------------

/// The listing is the kernel's rows and nothing else. `SessionListResult.sessions`
/// is an opaque `Value` by the protocol's own choice, so what can be verified
/// here is that a row without an id is *skipped* (an entry with an invented id is
/// a session the person could click and the kernel never heard of), that a row
/// with one is offered with its title and recency, and that no row is invented.
#[test]
fn the_session_listing_offers_the_kernels_rows_and_skips_unusable_ones() {
    let result = oup::SessionListResult {
        sessions: json!([
            {
                "id": "_main:zed:thread-7",
                "title": "a conversation",
                "updated_at": "2026-04-24T09:30:00Z",
                "message_count": 12,
            },
            // No id: nothing to open, so nothing to offer.
            { "title": "anonymous", "message_count": 3 },
            // An updated_at that is not RFC3339 is left absent rather than
            // guessed at.
            { "id": "_main:zed:thread-8", "updated_at": "yesterday" },
            "not even an object",
        ]),
        workspace_root: None,
        profile_id: None,
    };

    let sessions = listed_sessions(&result);
    assert_eq!(sessions.len(), 2, "{sessions:?}");
    assert_eq!(sessions[0].session_id.0.as_ref(), "_main:zed:thread-7");
    assert_eq!(sessions[0].title.as_deref(), Some("a conversation"));
    assert_eq!(
        sessions[0].updated_at.map(|stamp| stamp.to_rfc3339()),
        Some("2026-04-24T09:30:00+00:00".to_owned())
    );
    assert_eq!(sessions[1].session_id.0.as_ref(), "_main:zed:thread-8");
    assert!(sessions[1].title.is_none());
    assert!(sessions[1].updated_at.is_none(), "yesterday is not a timestamp");

    // A reply that is not a list of sessions is not a listing: the host offers
    // none rather than pretending it found some.
    let broken = oup::SessionListResult {
        sessions: json!({ "sessions": [] }),
        workspace_root: None,
        profile_id: None,
    };
    assert!(listed_sessions(&broken).is_empty());
}

/// The request: a cwd scopes the listing to the project store (and names the
/// profile every Octos session here opens under); no cwd is the historical empty
/// object, which is the per-profile global listing.
#[test]
fn the_session_list_request_forwards_the_project_and_nothing_else() {
    let scoped = session_list_params(&AgentSessionListRequest {
        cwd: Some(std::path::PathBuf::from("E:\\work")),
        cursor: None,
        meta: None,
    })
    .expect("list params");
    assert_eq!(scoped["cwd"], json!("E:\\work"));
    assert_eq!(scoped["profile_id"], json!(octos_core::MAIN_PROFILE_ID));

    let global = session_list_params(&AgentSessionListRequest::default()).expect("list params");
    assert!(global.get("cwd").is_none(), "{global}");
    assert!(global.get("profile_id").is_none(), "{global}");
}

// ---------------------------------------------------------------------------
// The model picker (P4)
// ---------------------------------------------------------------------------

/// The picker exists only for a session this connection actually opened: it is
/// built from the live per-thread state, so there is never a picker that could
/// only fail its calls. `test` is the ACP-side id, which *is* the kernel key.
///
/// What the picker *shows* is tested against a fake kernel in
/// `model_selector`'s own tests; what belongs here is the connection's answer to
/// "does this thread have a picker, and is it Zed's picker or Octos's".
#[tokio::test]
async fn the_model_picker_exists_only_for_an_open_session() {
    let octos = connection_for_test();
    let kernel_session = session();
    let acp_session_id = acp::SessionId::new(kernel_session.0.clone());
    octos.session_state(&acp_session_id, kernel_session);

    let selector: std::rc::Rc<dyn AgentModelSelector> = octos
        .model_selector(&acp_session_id)
        .expect("an open session has a picker");
    assert!(
        !selector.should_render_footer(),
        "the footer is Zed's own model settings, which do not configure an Octos profile"
    );

    let never_opened = acp::SessionId::new("never-opened");
    assert!(
        octos.model_selector(&never_opened).is_none(),
        "a session this connection does not have has no model list to show"
    );
}
