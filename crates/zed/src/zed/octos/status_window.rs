//! The Octos console: the host-side surface for the embedded kernel.
//!
//! One process, one executable: the console renders in a Zed window and talks to
//! Octos over the in-process OUP connection ([`super::connection`]) — the kernel
//! is a library in this process, so there is no `octos` subprocess and nothing
//! extra to ship. Typing a line sends one `turn/start`; the answer streams back
//! as `message/delta` notifications and is appended to the transcript.
//!
//! ## Why a window rather than a dockable panel
//!
//! A `workspace::Panel` would put this next to the project panel, and that is
//! where it eventually belongs. It is not where it belongs *yet*, for three
//! reasons that are all about this stage:
//!
//! 1. **The seam under test is the turn loop, not docking.** Approvals and tool
//!    activity have to be reachable and keyboard-answerable while a turn is
//!    live; a window that is already wired to the kernel does that today, and
//!    `Help > Octos`/`ZED_OCTOS_OPEN_STATUS=1` open it deterministically on a
//!    machine where clicking is not what is being tested. A panel would add
//!    `Panel::position`/`position_is_valid`/serialization/focus-routing surface
//!    before any of the protocol work could be exercised on real hardware.
//! 2. **Two panels is a worse answer than one window.** The status window
//!    (`super::status`) is real, useful and already the target of
//!    `ZED_OCTOS_OPEN_STATUS`. Folding it into the console keeps one Octos
//!    surface; a panel would leave two, with the old one orphaned.
//! 3. **A panel needs a workspace; the kernel does not.** A window owns its own
//!    lifetime and can be open with no project at all — which is exactly the
//!    state a host is in when the kernel fails to find a profile and the
//!    console's whole job is to say why.
//!
//! The reverse argument — approvals want to be visible while you look at your
//! work — is why the window is deliberately *not* modal and why it is opened
//! once and focused thereafter. If it later wants to be a dock, the rendering
//! below is the whole of what moves; nothing in [`super::peers`] or in the
//! approval state machine assumes a window.
//!
//! ## What it renders
//!
//! * the transcript (answers, reasoning, notes),
//! * **tool activity**: the agent's own tools (`octos tool`) and this host's
//!   (`zed tool`), one list — a tool call is not a line of conversation,
//! * **approvals**: every pending one, waiting for `y`/`n` (or the buttons).
//!
//! Opened by `Help > Octos`, which focuses an existing window rather than
//! opening a second one, the way the About window behaves. With no Octos profile
//! configured the kernel refuses to open a session, so the console shows that
//! reason and how to fix it instead of an input box that cannot work.

use gpui::{
    App, AppContext as _, ClipboardItem, Context, FocusHandle, KeyDownEvent, Render, SharedString,
    Size, TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions, point, px,
};
use release_channel::ReleaseChannel;
use std::sync::Arc;
use theme::ActiveTheme as _;
use ui::{Button, ButtonStyle, Color, Label, LabelSize, TintColor, prelude::*};

use super::peers::{EditorOpen, HostToolExecutor, RunPlan, ToolActivity, ToolOutcome};

/// How many transcript lines the console keeps on screen.
const VISIBLE_LINES: usize = 16;

/// How many activity entries the console keeps on screen.
const VISIBLE_ACTIVITY: usize = 8;

/// The Octos console window's root view.
///
/// Owns only the input buffer: the conversation lives with the kernel
/// connection in [`super::Octos`], so the console cannot drift out of sync with
/// what was actually sent.
pub struct OctosConsole {
    input: String,
    focus: FocusHandle,
}

impl OctosConsole {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            input: String::new(),
            focus: cx.focus_handle(),
        }
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        let text = std::mem::take(&mut self.input);
        if text.trim().is_empty() {
            return;
        }
        super::send_turn(cx, text);
        cx.notify();
    }

    /// Answer `approval`, if there is one, and drop the input buffer's focus
    /// claim on the keystroke.
    fn decide(&mut self, approval_id: &octos_core::ui_protocol::ApprovalId, approve: bool, cx: &mut Context<Self>) {
        super::decide_approval(cx, approval_id, approve);
        cx.notify();
    }

    /// Answer the newest pending approval. The keyboard shortcut a person
    /// reaches for when a card appears has to work without first tabbing to it.
    fn decide_newest(&mut self, approve: bool, cx: &mut Context<Self>) -> bool {
        let pending = super::approvals(cx);
        let Some(oldest) = pending.last() else {
            return false;
        };
        self.decide(&oldest.approval_id, approve, cx);
        true
    }
}

/// The activity list's rendering of one entry.
fn activity_line(entry: &ToolActivity) -> SharedString {
    let origin = if entry.tool.starts_with("zed.") {
        "zed tool"
    } else if entry.tool.starts_with('(') {
        // The registration line: not a tool at all.
        "host"
    } else {
        "octos tool"
    };
    let detail = if entry.detail.is_empty() {
        String::new()
    } else {
        format!(" — {}", first_line(&entry.detail, 120))
    };
    SharedString::from(format!(
        "{origin}  {}  [{}]{detail}",
        if entry.summary.is_empty() {
            entry.tool.clone()
        } else {
            entry.summary.clone()
        },
        entry.state.label()
    ))
}

/// The first line of `text`, capped — a tool result can be a whole file.
fn first_line(text: &str, cap: usize) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= cap {
        return line.to_owned();
    }
    let mut capped: String = line.chars().take(cap).collect();
    capped.push('…');
    capped
}

impl Render for OctosConsole {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = super::state_line(cx);
        let tools = super::tools_line(cx);
        let transcript = super::transcript(cx);
        let first = transcript.len().saturating_sub(VISIBLE_LINES);
        let lines: Vec<SharedString> = transcript[first..]
            .iter()
            .map(|line| SharedString::from(line.text()))
            .collect();
        let hints: Vec<SharedString> = if transcript.is_empty() {
            super::hints(cx)
                .into_iter()
                .map(SharedString::from)
                .collect()
        } else {
            Vec::new()
        };
        let can_send = super::can_send(cx);
        let pending = super::is_pending(cx);

        let activity = super::tool_activity(cx);
        let activity_first = activity.len().saturating_sub(VISIBLE_ACTIVITY);
        let activity_lines: Vec<SharedString> = activity[activity_first..]
            .iter()
            .map(activity_line)
            .collect();

        let approvals = super::approvals(cx);
        let approval_cards: Vec<_> = approvals
            .iter()
            .map(|approval| {
                let approval_id = approval.approval_id.clone();
                let approve_id = approval_id.clone();
                v_flex()
                    .id(SharedString::from(format!(
                        "octos-approval-{}",
                        approval_id.0
                    )))
                    .gap_1()
                    .p_2()
                    .rounded_sm()
                    .bg(cx.theme().colors().element_background)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Label::new(SharedString::from(format!(
                                    "{} · {}",
                                    approval.source.label(),
                                    approval.tool
                                )))
                                .size(LabelSize::Small),
                            )
                            .when_some(approval.risk.clone(), |this, risk| {
                                this.child(
                                    Label::new(SharedString::from(risk))
                                        .color(Color::Muted)
                                        .size(LabelSize::XSmall),
                                )
                            }),
                    )
                    .child(
                        Label::new(SharedString::from(approval.summary.clone()))
                            .size(LabelSize::XSmall),
                    )
                    .child(
                        Label::new(SharedString::from(first_line(&approval.detail, 240)))
                            .color(Color::Muted)
                            .size(LabelSize::XSmall),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new(
                                    SharedString::from(format!("deny-{}", approval_id.0)),
                                    "Deny (n)",
                                )
                                .style(ButtonStyle::OutlinedGhost)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.decide(&approval_id, false, cx);
                                })),
                            )
                            .child(
                                Button::new(
                                    SharedString::from(format!("approve-{}", approve_id.0)),
                                    "Approve (y)",
                                )
                                .style(ButtonStyle::Tinted(TintColor::Accent))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.decide(&approve_id, true, cx);
                                })),
                            ),
                    )
            })
            .collect();

        let input_display: SharedString = if self.input.is_empty() {
            if can_send {
                "Type a message and press Enter".into()
            } else {
                "No session open — see the lines above".into()
            }
        } else {
            format!("{}▌", self.input).into()
        };

        v_flex()
            .id("octos-console")
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .text_color(cx.theme().colors().text)
            .p_4()
            .gap_3()
            // The prompt owns the keyboard: the root tracks focus and handles
            // keys, so typing keeps working after a button has taken focus (gpui
            // bubbles key events up from the focused element to here).
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                let keystroke = &event.keystroke;
                if keystroke.modifiers.control
                    || keystroke.modifiers.alt
                    || keystroke.modifiers.platform
                {
                    return;
                }
                match keystroke.key.as_str() {
                    "enter" => this.send(cx),
                    "backspace" => {
                        this.input.pop();
                        cx.notify();
                    }
                    // A pending approval is answered from anywhere in the
                    // window: the person's decision must not depend on having
                    // tabbed to the right button.
                    "y" if !this.input.is_empty() => {
                        this.input.push('y');
                        cx.notify();
                    }
                    "n" if !this.input.is_empty() => {
                        this.input.push('n');
                        cx.notify();
                    }
                    "y" => {
                        if !this.decide_newest(true, cx) {
                            this.input.push('y');
                            cx.notify();
                        }
                    }
                    "n" => {
                        if !this.decide_newest(false, cx) {
                            this.input.push('n');
                            cx.notify();
                        }
                    }
                    _ => {
                        if let Some(text) = keystroke.key_char.as_ref() {
                            this.input.push_str(text);
                            cx.notify();
                        }
                    }
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| this.focus.focus(window, cx)))
            .child(
                h_flex()
                    .justify_between()
                    .child(Headline::new("Octos"))
                    .child(
                        Label::new(SharedString::from(format!("{state}  ·  {tools}")))
                            .color(Color::Muted)
                            .size(LabelSize::XSmall),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .gap_1()
                    .overflow_hidden()
                    .children(
                        lines
                            .into_iter()
                            .map(|line| Label::new(line).size(LabelSize::Small)),
                    )
                    .children(hints.into_iter().map(|hint| {
                        Label::new(hint).color(Color::Muted).size(LabelSize::XSmall)
                    })),
            )
            .when(!approval_cards.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .child(
                            Label::new("Waiting on you")
                                .color(Color::Muted)
                                .size(LabelSize::XSmall),
                        )
                        .children(approval_cards),
                )
            })
            .when(!activity_lines.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .child(
                            Label::new("Tool activity")
                                .color(Color::Muted)
                                .size(LabelSize::XSmall),
                        )
                        .children(
                            activity_lines
                                .into_iter()
                                .map(|line| Label::new(line).size(LabelSize::XSmall)),
                        ),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .child(
                                Label::new(input_display)
                                    .color(if self.input.is_empty() {
                                        Color::Muted
                                    } else {
                                        Color::Default
                                    })
                                    .size(LabelSize::Small),
                            ),
                    )
                    .child(
                        Button::new("send", if pending { "Sending…" } else { "Send" })
                            .style(ButtonStyle::Tinted(TintColor::Accent))
                            .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                    )
                    .child(
                        Button::new("copy", "Copy")
                            .style(ButtonStyle::OutlinedGhost)
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(super::status(cx)));
                            })),
                    ),
            )
    }
}

/// The app's executor for host tools.
///
/// The default ([`super::peers::BuiltinExecutor`]) serves reads and refuses an
/// open: it has no `App` to hand one to. This one adds the delivery — a
/// callback the app installs, invoked with the cleared, approved effect and
/// nothing else. The executor runs on the kernel's runtime, where no `App`
/// exists, so the callback must be a `Send + Sync` handoff (the app uses a
/// channel; see [`super::init_host_tools`]).
pub struct EditorOpenExecutor {
    open: Arc<dyn Fn(EditorOpen) + Send + Sync>,
}

impl EditorOpenExecutor {
    pub fn new(open: impl Fn(EditorOpen) + Send + Sync + 'static) -> Self {
        Self {
            open: Arc::new(open),
        }
    }
}

impl HostToolExecutor for EditorOpenExecutor {
    /// Reads are served inline; an open is published. The plan reaching here
    /// has already passed authorization, the person's gate and the
    /// cancellation check (`HostPeer::decide`), which is what makes publishing
    /// the effect the whole of "running" it.
    fn execute(&self, plan: &RunPlan) -> ToolOutcome {
        match &plan.args {
            super::peers::Args::Read { path, max_bytes } => {
                super::peers::read_workspace_file(&plan.workspace, path, *max_bytes)
            }
            super::peers::Args::Open { path, line } => {
                let absolute = plan.workspace.join(path);
                let canonical = super::peers::canonical_path(&absolute);
                self.publish_editor_open(EditorOpen {
                    path: canonical,
                    workspace: plan.workspace.clone(),
                    line: *line,
                });
                ToolOutcome::ok(serde_json::json!({ "opened": path, "line": line }))
            }
        }
    }

    fn publish_editor_open(&self, open: EditorOpen) {
        (self.open)(open);
    }
}

/// Opens the console, or focuses the one that is already open.
pub fn open(cx: &mut App) {
    // Rendering needs `theme::GlobalTheme`, and `render` would panic without it
    // ("no state of type theme::GlobalTheme exists"). A menu click always
    // happens long after the theme is registered, but the
    // `ZED_OCTOS_OPEN_STATUS` startup path can run before that, so wait for it
    // instead of opening a window that cannot draw.
    if cx.try_global::<theme::GlobalTheme>().is_none() {
        log::info!("[octos] waiting for the theme before opening the console");
        cx.spawn(async move |cx| {
            for _ in 0..300 {
                if cx.update(|cx| cx.try_global::<theme::GlobalTheme>().is_some()) {
                    break;
                }
                smol::Timer::after(std::time::Duration::from_millis(100)).await;
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
        .find_map(|window| window.downcast::<OctosConsole>())
    {
        if let Err(error) = existing.update(cx, |_, window, _| window.activate_window()) {
            log::warn!("[octos] could not focus the console: {error}");
        }
        return;
    }

    let window_size = Size {
        width: px(680.),
        height: px(520.),
    };

    let opened = cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Octos".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(12.), px(12.))),
            }),
            window_bounds: Some(WindowBounds::centered(window_size, cx)),
            kind: WindowKind::Normal,
            app_id: Some(ReleaseChannel::global(cx).app_id().to_owned()),
            ..Default::default()
        },
        |window, cx| {
            let console = cx.new(OctosConsole::new);
            let focus = console.read(cx).focus.clone();
            window.activate_window();
            // Focus after the first frame: an element that has not been rendered
            // yet cannot take focus, so focusing here directly would be lost.
            window.defer(cx, move |window, cx| focus.focus(window, cx));
            console.into()
        },
    );

    match opened {
        Ok(_) => log::info!(
            "[octos] console opened ({} window(s) now open)",
            cx.windows().len()
        ),
        Err(error) => log::error!("[octos] could not open the console: {error}"),
    }
}
