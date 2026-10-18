//! The Octos console: the host-side surface for the embedded kernel.
//!
//! One process, one executable: the console renders in a Zed window and talks to
//! Octos over the in-process OUP connection ([`super::connection`]) — the kernel
//! is a library in this process, so there is no `octos` subprocess and nothing
//! extra to ship. Typing a line sends one `turn/start`; the answer streams back
//! as `projection/envelope` `assistant_delta` payloads and is appended to the
//! transcript. That envelope is the *only* text lane: the kernel drops the raw
//! `message/delta` record for every connection (see [`super::envelope_action`]).
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
//!    `ZED_OCTOS_OPEN_CONSOLE=1` opens it deterministically on a machine where
//!    clicking is not what is being tested. A panel would add
//!    `Panel::position`/`position_is_valid`/serialization/focus-routing surface
//!    before any of the protocol work could be exercised on real hardware.
//! 2. **Two panels is a worse answer than one window.** The status window
//!    (`super::status`) is real, useful and already the target of
//!    `ZED_OCTOS_OPEN_CONSOLE`. Folding it into the console keeps one Octos
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
//! * **approvals**: every pending one, waiting for `y`/`n` (or the buttons),
//! * **questions**: the kernel's structured question (`user_question/requested`,
//!   UPCR-2026-023), answered from the options it offered, with free text from
//!   the prompt box and an explicit *Dismiss (no answer)* exit — the kernel's
//!   `ask_user_question` tool blocks with no deadline of its own, so a question
//!   nobody answers parks the turn,
//! * **Stop**: `turn/interrupt` for the turn in flight, also on `Esc`. Both
//!   are handled by this window's own element (`on_key_down` on the root), so
//!   nothing is bound globally and the main window is untouched.
//!
//! Opened by `ZED_OCTOS_OPEN_CONSOLE=1`, which focuses an existing window rather
//! than opening a second one, the way the About window behaves. `Help > Octos`
//! runs the CLI in a terminal instead ([`super::open`]). With no Octos profile
//! configured the kernel refuses to open a session, so the console shows that
//! reason and how to fix it instead of an input box that cannot work.

use gpui::{
    App, AppContext as _, Bounds, ClipboardItem, Context, CursorStyle, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Hsla, KeyDownEvent, Pixels, Point, Render, ShapedLine,
    SharedString, Size, TextAlign, TextRun, TitlebarOptions, UTF16Selection, UnderlineStyle, Window,
    WindowBounds, WindowKind, WindowOptions, canvas, fill, point, px, size,
};
use release_channel::ReleaseChannel;
use std::ops::Range;
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
/// Owns the input buffer: the conversation lives with the kernel connection in
/// [`super::Octos`], so the console cannot drift out of sync with what was
/// actually sent.
///
/// The prompt box is a real text input — see [`prompt_line`] for why a `Label`
/// and `on_key_down` are not one.
pub struct OctosConsole {
    input: String,
    focus: FocusHandle,
    /// Where the caret is, as UTF-8 byte offsets into `input`. Always empty
    /// (a caret) for now: the console has no selection UI, but the platform's
    /// IME asks for a range and `replace_*` edits through one.
    selected_range: Range<usize>,
    /// An IME composition in flight, as a UTF-8 byte range of `input`.
    marked_range: Option<Range<usize>>,
    /// The prompt line as last shaped, for the caret and for the IME's
    /// candidate window (`bounds_for_range`).
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
}

impl OctosConsole {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            input: String::new(),
            focus: cx.focus_handle(),
            selected_range: 0..0,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
        }
    }

    /// Takes the buffer, leaving the caret at the start of an empty box.
    fn take_input(&mut self) -> String {
        self.selected_range = 0..0;
        self.marked_range = None;
        std::mem::take(&mut self.input)
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        let text = self.take_input();
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

    /// Enter: answer the kernel's question when one is waiting, otherwise send
    /// the prompt as a turn.
    ///
    /// The same box serves both because a question parks the turn: a new turn
    /// started while the kernel is waiting on an answer would be refused, so
    /// the box belongs to the answer until it is given.
    fn submit(&mut self, cx: &mut Context<Self>) {
        let waiting = super::pending_questions(cx);
        let Some(asked) = waiting.first() else {
            self.send(cx);
            return;
        };
        let text = self.take_input();
        super::answer_question(cx, &asked.question_id, &text);
        cx.notify();
    }

    /// Deletes backwards: the selection, or the character before the caret.
    ///
    /// The platform has no method for this in [`EntityInputHandler`] — a real
    /// input handles it as an action, and this box has no action context — so
    /// it is the one edit the console makes through its own key handler.
    fn backspace(&mut self, cx: &mut Context<Self>) {
        let range = if self.selected_range.is_empty() {
            let caret = self.selected_range.start;
            match self.input[..caret].chars().next_back() {
                Some(character) => caret - character.len_utf8()..caret,
                None => return,
            }
        } else {
            self.selected_range.clone()
        };
        self.apply_edit(range, "", false, None);
        cx.notify();
    }

    /// The range an edit uses when the platform did not name one: the IME's
    /// composition if there is one, else the selection. That is the platform's
    /// own rule, and it is why a composition replaces itself instead of
    /// appending a second copy of every candidate.
    fn edit_range(&self, range_utf16: Option<&Range<usize>>) -> Range<usize> {
        match range_utf16 {
            Some(range) => {
                let start = offset_from_utf16(&self.input, range.start);
                let end = offset_from_utf16(&self.input, range.end);
                start.min(end)..start.max(end)
            }
            None => self
                .marked_range
                .clone()
                .unwrap_or_else(|| self.selected_range.clone()),
        }
    }

    /// Replaces `range` in the buffer, moving the caret and setting the marked
    /// range. The string surgery itself is [`splice`], which is pure.
    fn apply_edit(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        mark: bool,
        caret_utf16: Option<&Range<usize>>,
    ) {
        let (text, caret, marked) = splice(&self.input, range, new_text, mark, caret_utf16);
        self.input = text;
        self.selected_range = caret;
        self.marked_range = marked;
    }
}

/// Replaces `range` in `content` with `new_text`.
///
/// Returns the new content, the new caret (a UTF-8 byte range) and the new
/// marked range. `caret_utf16` is the platform's `new_selected_range`, which is
/// relative to `new_text` and in UTF-16 code units — not bytes, and not the
/// buffer's offsets.
///
/// Pure on purpose: this is the whole of what an IME edit does to the text, and
/// it is tested without a window.
fn splice(
    content: &str,
    range: Range<usize>,
    new_text: &str,
    mark: bool,
    caret_utf16: Option<&Range<usize>>,
) -> (String, Range<usize>, Option<Range<usize>>) {
    let (start, end) = if range.start <= range.end {
        (range.start, range.end)
    } else {
        (range.end, range.start)
    };
    let start = start.min(content.len());
    let end = end.clamp(start, content.len());
    let mut text = String::with_capacity(content.len() + new_text.len());
    text.push_str(&content[..start]);
    let inserted_at = text.len();
    text.push_str(new_text);
    text.push_str(&content[end..]);
    let inserted = inserted_at..inserted_at + new_text.len();
    let marked = if mark && !new_text.is_empty() {
        Some(inserted.clone())
    } else {
        None
    };
    let caret = match caret_utf16 {
        Some(relative) => {
            let start = inserted_at + offset_from_utf16(new_text, relative.start);
            let end = inserted_at + offset_from_utf16(new_text, relative.end);
            start.min(end)..start.max(end)
        }
        None => inserted.end..inserted.end,
    };
    (text, caret, marked)
}

/// `text`'s UTF-8 byte offset for a UTF-16 code-unit offset.
///
/// The platform speaks UTF-16 (`crates/gpui/src/input.rs`), the buffer speaks
/// UTF-8. For anything past the end this is the length, and it always lands on
/// a character boundary.
fn offset_from_utf16(text: &str, offset: usize) -> usize {
    let mut utf8 = 0;
    let mut utf16 = 0;
    for character in text.chars() {
        if utf16 >= offset {
            break;
        }
        utf16 += character.len_utf16();
        utf8 += character.len_utf8();
    }
    utf8
}

/// `text`'s UTF-16 code-unit offset for a UTF-8 byte offset.
fn offset_to_utf16(text: &str, offset: usize) -> usize {
    let mut utf8 = 0;
    let mut utf16 = 0;
    for character in text.chars() {
        if utf8 >= offset {
            break;
        }
        utf8 += character.len_utf8();
        utf16 += character.len_utf16();
    }
    utf16
}

/// A UTF-8 byte range as the UTF-16 range the platform asked in terms of.
fn range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    offset_to_utf16(text, range.start)..offset_to_utf16(text, range.end)
}

/// A UTF-16 range from the platform as UTF-8 byte offsets into `text`.
fn range_from_utf16(text: &str, range_utf16: &Range<usize>) -> Range<usize> {
    offset_from_utf16(text, range_utf16.start)..offset_from_utf16(text, range_utf16.end)
}

/// The prompt box's text, as the platform's text input sees it.
///
/// # Why this is not a `Label` and an `on_key_down`
///
/// Text does not arrive as key events. On Windows every character is
/// `WM_CHAR`, and every IME interaction is `WM_IME_*`; both are handed to the
/// window's **input handler** and silently dropped when there is none
/// (`handle_char_msg`, `crates/gpui_windows/src/events.rs:473`;
/// `handle_ime_composition_inner`, same file `:730`). A window with no
/// accepting handler also has its IME context *detached*
/// (`update_ime_enabled`, same file `:695`), so the IME is not merely ignored —
/// it is switched off for this window. X11 and Wayland route text the same way
/// (`crates/gpui_linux/src/linux/x11/window.rs:1197`).
///
/// That is exactly the reported behaviour: typed English worked (a key event
/// carries `key_char`), Chinese did not, and there was no caret because a
/// `Label` with a hand-appended `▌` draws one only after the first character.
/// The handler is registered from `paint` (`Window::handle_input`,
/// `crates/gpui/src/window.rs:5018`), which is where the platform expects it —
/// the same shape as gpui's own `TextInput`
/// (`crates/gpui/examples/input.rs:545`).
fn prompt_line(
    view: Entity<OctosConsole>,
    focus: FocusHandle,
    text: SharedString,
    selected_range: Range<usize>,
    marked_range: Option<Range<usize>>,
    placeholder: SharedString,
    text_color: Hsla,
    placeholder_color: Hsla,
    height: Pixels,
) -> impl IntoElement {
    let caret_index = selected_range.end;
    canvas(
        move |_bounds, window, _cx| {
            shape_prompt_line(&text, marked_range.as_ref(), &placeholder, window)
        },
        move |bounds, prompt, window, cx| {
            window.handle_input(&focus, ElementInputHandler::new(bounds, view.clone()), cx);

            let line_height = window.line_height();
            let line = prompt.line.clone();
            // Kept for `bounds_for_range` (the IME's candidate window follows
            // the caret) and `character_index_for_point` (a click in the box).
            view.update(cx, |console, _cx| {
                console.last_layout = Some(line.clone());
                console.last_bounds = Some(bounds);
            });

            let color = if prompt.is_placeholder {
                placeholder_color
            } else {
                text_color
            };
            line.paint(
                bounds.origin,
                line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            )
            .ok();

            // The caret is drawn only while this box has the keyboard, and
            // never over the placeholder — the placeholder is not text.
            if !prompt.is_placeholder && focus.is_focused(window) {
                let x = bounds.left() + line.x_for_index(caret_index.min(line.len()));
                window.paint_quad(fill(
                    Bounds::new(
                        point(x, bounds.top()),
                        size(px(2.), bounds.bottom() - bounds.top()),
                    ),
                    color,
                ));
            }
        },
    )
    // A canvas lays out at its own size, which is nothing at all by default: it
    // would be zero-tall, and the caret — whose height is this box's height —
    // would be invisible. The line's own height is the box's.
    .w_full()
    .h(height)
}

/// What [`prompt_line`] shapes before painting.
struct PromptLine {
    line: ShapedLine,
    is_placeholder: bool,
}

/// Shapes the prompt, underlining an in-flight IME composition.
///
/// `shape_line` panics on a newline, and a paste can carry one; the box is a
/// single line, so they are shown as spaces.
fn shape_prompt_line(
    text: &SharedString,
    marked_range: Option<&Range<usize>>,
    placeholder: &SharedString,
    window: &mut Window,
) -> PromptLine {
    let is_placeholder = text.is_empty();
    let display: SharedString = if is_placeholder {
        placeholder.clone()
    } else {
        text.clone()
    };
    let display: SharedString = if display.contains('\n') {
        display.replace('\n', " ").into()
    } else {
        display
    };

    let style = window.text_style();
    let color = if is_placeholder {
        window.text_style().color.opacity(0.5)
    } else {
        style.color
    };
    let run = TextRun {
        len: display.len(),
        font: style.font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let runs: Vec<TextRun> = match marked_range {
        Some(marked) if !is_placeholder && marked.end <= display.len() => vec![
            TextRun {
                len: marked.start,
                ..run.clone()
            },
            TextRun {
                len: marked.end - marked.start,
                underline: Some(UnderlineStyle {
                    color: Some(color),
                    thickness: px(1.0),
                    wavy: false,
                }),
                ..run.clone()
            },
            TextRun {
                len: display.len() - marked.end,
                ..run
            },
        ]
        .into_iter()
        .filter(|run| run.len > 0)
        .collect(),
        _ => vec![run],
    };

    let font_size = style.font_size.to_pixels(window.rem_size());
    PromptLine {
        line: window
            .text_system()
            .shape_line(display, font_size, &runs, None),
        is_placeholder,
    }
}

impl EntityInputHandler for OctosConsole {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = range_from_utf16(&self.input, &range_utf16);
        adjusted_range.replace(range_to_utf16(&self.input, &range));
        Some(self.input[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: range_to_utf16(&self.input, &self.selected_range),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| range_to_utf16(&self.input, range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.edit_range(range_utf16.as_ref());
        // Said at debug level because this is the line that answers "did the
        // character reach the box at all?": an IME commit, a `WM_CHAR`, and a
        // paste all arrive here and nowhere else.
        log::debug!(
            "[octos] console text input {new_text:?} range={range:?} len={}",
            self.input.len()
        );
        // A pending approval is answered from anywhere in the window: the
        // person's decision must not depend on having tabbed to the right
        // button. The shortcut lives here, not in the key handler, because this
        // is where the character actually arrives — and only an otherwise empty
        // box takes it, so `y` as the first letter of a prompt is still a
        // letter once no approval is waiting.
        if range.is_empty()
            && self.input.is_empty()
            && (new_text == "y" || new_text == "n")
            && self.decide_newest(new_text == "y", cx)
        {
            return;
        }
        self.apply_edit(range, new_text, false, None);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.edit_range(range_utf16.as_ref());
        self.apply_edit(
            range,
            new_text,
            true,
            new_selected_range_utf16.as_ref(),
        );
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        let range = range_from_utf16(&self.input, &range_utf16);
        Some(Bounds::from_corners(
            point(
                element_bounds.left() + line.x_for_index(range.start.min(line.len())),
                element_bounds.top(),
            ),
            point(
                element_bounds.left() + line.x_for_index(range.end.min(line.len())),
                element_bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        if self.input.is_empty() {
            return Some(0);
        }
        let bounds = self.last_bounds?;
        let line = self.last_layout.as_ref()?;
        let local = bounds.localize(&point)?;
        let utf8_index = line.index_for_x(local.x)?;
        Some(offset_to_utf16(&self.input, utf8_index))
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let line_height = window.line_height();
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

        let questions = super::pending_questions(cx);
        let question_cards: Vec<_> = questions
            .iter()
            .map(|asked| {
                let answer_id = asked.question_id.clone();
                let dismiss_id = asked.question_id.clone();
                let question_rows: Vec<_> = asked
                    .questions
                    .iter()
                    .enumerate()
                    .map(|(index, question)| {
                        let selections = asked
                            .selections
                            .get(index)
                            .cloned()
                            .unwrap_or_default();
                        let option_rows: Vec<_> = question
                            .options
                            .iter()
                            .map(|option| {
                                let label = option.label.clone();
                                let chosen = selections.iter().any(|selected| selected == &label);
                                let option_id = asked.question_id.clone();
                                let click_label = label.clone();
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new(
                                            SharedString::from(format!(
                                                "octos-option-{}-{index}-{}",
                                                asked.question_id.0, option.label
                                            )),
                                            SharedString::from(label.clone()),
                                        )
                                        .style(if chosen {
                                            ButtonStyle::Tinted(TintColor::Accent)
                                        } else {
                                            ButtonStyle::OutlinedGhost
                                        })
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            super::select_question_option(
                                                cx,
                                                &option_id,
                                                index,
                                                &click_label,
                                            );
                                            cx.notify();
                                        })),
                                    )
                                    .child(
                                        Label::new(SharedString::from(option.description.clone()))
                                            .color(Color::Muted)
                                            .size(LabelSize::XSmall),
                                    )
                            })
                            .collect();
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new(SharedString::from(format!(
                                    "{} — {}",
                                    question.header, question.question
                                )))
                                .size(LabelSize::XSmall),
                            )
                            .child(
                                Label::new(if question.multi_select {
                                    "choose any of these"
                                } else {
                                    "choose one of these"
                                })
                                .color(Color::Muted)
                                .size(LabelSize::XSmall),
                            )
                            .children(option_rows)
                    })
                    .collect();

                v_flex()
                    .id(SharedString::from(format!(
                        "octos-question-{}",
                        asked.question_id.0
                    )))
                    .gap_1()
                    .p_2()
                    .rounded_sm()
                    .bg(cx.theme().colors().element_background)
                    .child(
                        Label::new(SharedString::from(asked.title.clone()))
                            .size(LabelSize::Small),
                    )
                    .child(
                        Label::new(SharedString::from(first_line(&asked.body, 240)))
                            .color(Color::Muted)
                            .size(LabelSize::XSmall),
                    )
                    .children(question_rows)
                    .child(
                        Label::new(if asked
                            .questions
                            .iter()
                            .any(|question| question.allow_free_text)
                        {
                            "free text goes in the box below (Enter sends it with the choices)"
                        } else {
                            "this question takes no free text"
                        })
                        .color(Color::Muted)
                        .size(LabelSize::XSmall),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new(
                                    SharedString::from(format!("answer-{}", asked.question_id.0)),
                                    "Answer",
                                )
                                .style(ButtonStyle::Tinted(TintColor::Accent))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let text = std::mem::take(&mut this.input);
                                    super::answer_question(cx, &answer_id, &text);
                                    cx.notify();
                                })),
                            )
                            .child(
                                Button::new(
                                    SharedString::from(format!("dismiss-{}", asked.question_id.0)),
                                    "Dismiss (no answer)",
                                )
                                .style(ButtonStyle::OutlinedGhost)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    super::dismiss_question(cx, &dismiss_id);
                                    cx.notify();
                                })),
                            ),
                    )
            })
            .collect();

        let question_waits = !questions.is_empty();

        let placeholder: SharedString = if question_waits {
            "Type free text (optional), Enter answers the question".into()
        } else if can_send {
            "Type a message and press Enter".into()
        } else {
            "No session open — see the lines above".into()
        };

        v_flex()
            .id("octos-console")
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .text_color(cx.theme().colors().text)
            .p_4()
            .gap_3()
            // The prompt owns the keyboard: the root tracks focus and handles
            // the keys the box does not (Enter sends, Escape stops, Backspace
            // edits), so they keep working after a button has taken focus (gpui
            // bubbles key events up from the focused element to here).
            //
            // Printable characters are deliberately **not** handled here. They
            // arrive through the input handler ([`prompt_line`]), and doing both
            // would type every letter twice.
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
                    "enter" => this.submit(cx),
                    // Escape stops the turn in flight. It is deliberately the
                    // *only* thing Escape does: a pending approval is still
                    // answered with y/n, so the riskier reading of Escape —
                    // "approve the thing that is blocking me" — is never taken
                    // by accident. With no turn in flight nothing is sent
                    // (`super::interrupt_turn` logs it and returns).
                    "escape" => super::interrupt_turn(cx),
                    "backspace" => this.backspace(cx),
                    _ => {}
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
            .when(!question_cards.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .child(
                            Label::new("The kernel is waiting on your answer")
                                .color(Color::Muted)
                                .size(LabelSize::XSmall),
                        )
                        .children(question_cards),
                )
            })
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
                            .cursor(CursorStyle::IBeam)
                            .child(prompt_line(
                                cx.entity(),
                                self.focus.clone(),
                                self.input.clone().into(),
                                self.selected_range.clone(),
                                self.marked_range.clone(),
                                placeholder,
                                cx.theme().colors().text,
                                cx.theme().colors().text_placeholder,
                                line_height,
                            )),
                    )
                    .child(
                        Button::new(
                            "send",
                            if question_waits {
                                "Answer"
                            } else if pending {
                                "Sending…"
                            } else {
                                "Send"
                            },
                        )
                        .style(ButtonStyle::Tinted(TintColor::Accent))
                        .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    )
                    .child(
                        // Stop is only live while a turn is in flight: there is
                        // no turn to name otherwise, and the host sends nothing
                        // (`super::interrupt_turn`). Esc does the same thing.
                        Button::new("interrupt", "Stop (Esc)")
                            .style(ButtonStyle::OutlinedGhost)
                            .disabled(!pending)
                            .on_click(cx.listener(|_, _, _, cx| super::interrupt_turn(cx))),
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
    // ("no state of type theme::GlobalTheme exists"). Opening from the menu
    // always happens long after the theme is registered, but the
    // `ZED_OCTOS_OPEN_CONSOLE` startup path can run before that, so wait for it
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
        if let Err(error) = existing.update(cx, |console, window, cx| {
            take_keyboard(window, &console.focus, cx);
        }) {
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
                // The system title bar; see `open_gpui_kit_showcase` for why.
                // The console draws no window chrome of its own, so a
                // transparent title bar (which is what hides the system one)
                // would leave it with no minimize, maximize or close button.
                appears_transparent: false,
                traffic_light_position: None,
            }),
            window_bounds: Some(WindowBounds::centered(window_size, cx)),
            kind: WindowKind::Normal,
            app_id: Some(ReleaseChannel::global(cx).app_id().to_owned()),
            ..Default::default()
        },
        |window, cx| {
            let console = cx.new(OctosConsole::new);
            let focus = console.read(cx).focus.clone();
            // **After the first frame, not during creation.** Taking the
            // keyboard from inside the creation closure does nothing on
            // Windows: the `HWND` has not been shown yet, so the platform's
            // `SetForegroundWindow` cannot promote it and the process's
            // existing window keeps the keyboard — the console then renders,
            // sits behind the editor, and every keystroke goes to the editor.
            // This is not a theory: the old `Help > Octos` path (and the
            // `ZED_OCTOS_OPEN_CONSOLE` startup path) opened exactly such a
            // window, and typing into it produced no `prompt submitted` line at
            // all (`crates/zed/src/zed/octos/mod.rs:1470`).
            window.on_next_frame(move |window, cx| take_keyboard(window, &focus, cx));
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

/// Gives the console the keyboard: the OS foreground *and* gpui's focus.
///
/// Both halves are required and they are not the same thing.
/// `Window::activate_window` is platform-level only (`crates/gpui/src/window.rs:6211`
/// → `platform_window.activate()`); it never moves gpui's own focus. gpui
/// dispatches a keystroke to the focused element and bubbles it up from there
/// (`Window::dispatch_key_event`, same file `:5615`, via
/// `focus_node_id_in_rendered_frame`, `:6049`), and with nothing focused it
/// starts at the *window root* — which is above this view's root, where the
/// console's `on_key_down` lives. A console that is foreground at the OS level
/// but unfocused in gpui therefore swallows every character.
fn take_keyboard(window: &mut Window, focus: &FocusHandle, cx: &mut App) {
    window.activate_window();
    focus.focus(window, cx);
    // Said out loud because "the box does not type" and "the window never took
    // the keyboard" look identical from the outside, and this is the one line
    // that tells them apart in Zed.log.
    log::info!(
        "[octos] console took the keyboard (gpui focus: {})",
        focus.is_focused(window)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The platform speaks UTF-16 code units; the buffer speaks UTF-8 bytes.
    /// Getting this wrong is how a caret lands in the middle of a character and
    /// an IME edit corrupts the text around it — and it is exactly the part
    /// that only shows up with non-ASCII input.
    #[test]
    fn utf16_and_utf8_offsets_agree_across_the_boundary() {
        // "a", "é" (2 bytes, 1 unit), "你" (3 bytes, 1 unit), "😀" (4 bytes,
        // 2 units: a surrogate pair).
        let text = "aé你😀b";

        // Every character boundary, in UTF-16 units.
        let expected = [(0, 0), (1, 1), (3, 2), (6, 3), (10, 5), (11, 6)];
        for (utf8, utf16) in expected {
            assert_eq!(
                offset_to_utf16(text, utf8),
                utf16,
                "utf8 {utf8} in {text:?}"
            );
            assert_eq!(
                offset_from_utf16(text, utf16),
                utf8,
                "utf16 {utf16} in {text:?}"
            );
        }
        // Past the end is the end, never a panic and never a mid-character
        // offset.
        assert_eq!(offset_from_utf16(text, 99), text.len());
        assert_eq!(offset_to_utf16(text, 99), 6);
    }

    /// A composition replaces itself. The second `replace_and_mark` names no
    /// range, so it must land on the marked range — not append beside it —
    /// which is the difference between typing 你 and typing 你你.
    #[test]
    fn a_composition_edit_replaces_the_marked_range() {
        // The first keystroke of a 拼音 composition: "n" arrives as the marked
        // text.
        let (text, caret, marked) = splice("", 0..0, "n", true, None);
        assert_eq!(text, "n");
        assert_eq!(caret, 1..1);
        assert_eq!(marked, Some(0..1));

        // "ni" supersedes it: the platform names no range, so the caller passes
        // the marked range it was told about.
        let marked_range = marked.expect("a marked range");
        let (text, caret, marked) = splice(&text, marked_range, "ni", true, None);
        assert_eq!(text, "ni", "a composition must not append beside itself");
        assert_eq!(caret, 2..2);
        assert_eq!(marked, Some(0..2));

        // Commit: the result string replaces the composition and is no longer
        // marked, so the underline goes away.
        let marked_range = marked.expect("a marked range");
        let (text, caret, marked) = splice(&text, marked_range, "你", false, None);
        assert_eq!(text, "你");
        assert_eq!(caret, 3..3, "the caret is after a 3-byte character");
        assert_eq!(marked, None);
    }

    /// The platform's `new_selected_range` is relative to the inserted text and
    /// in UTF-16 units; the caret it produces is a UTF-8 byte range in the
    /// buffer.
    #[test]
    fn a_relative_utf16_caret_becomes_a_buffer_offset() {
        // "ab" inserted at the end of "x", with the platform asking for the
        // caret after the first inserted unit.
        let (text, caret, _) = splice("x", 1..1, "ab", false, Some(&(1..1)));
        assert_eq!(text, "xab");
        assert_eq!(caret, 2..2);

        // A surrogate-pair emoji: 2 UTF-16 units, 4 UTF-8 bytes.
        let (text, caret, _) = splice("", 0..0, "😀", false, Some(&(2..2)));
        assert_eq!(text, "😀");
        assert_eq!(caret, 4..4, "the caret is after the whole pair");

        // A range that is not at the end of the inserted text.
        let (text, caret, _) = splice("你", 0..3, "ab", false, Some(&(0..1)));
        assert_eq!(text, "ab");
        assert_eq!(caret, 0..1);
    }

    /// An edit that deletes the last character — the one Backspace makes — must
    /// remove a whole character, not a byte: a byte-wise pop on 你 leaves two
    /// bytes that are not valid UTF-8.
    #[test]
    fn a_backspace_range_covers_a_whole_character() {
        let text = "aé你";
        let caret = text.len();
        let character = text[..caret].chars().next_back().expect("a character");
        assert_eq!(caret - character.len_utf8(), 3, "你 starts at byte 3");
        let (text, caret, _) = splice(text, 3..6, "", false, None);
        assert_eq!(text, "aé");
        assert_eq!(caret, 3..3);
    }

    /// Out-of-range and reversed ranges are clamped rather than panicking: they
    /// come from another process, and a slice panic would take the window down.
    #[test]
    fn an_out_of_range_edit_is_clamped() {
        let (text, _, _) = splice("abc", 1..99, "z", false, None);
        assert_eq!(text, "az");
        let (text, _, _) = splice("abc", 99..99, "z", false, None);
        assert_eq!(text, "abcz");
        // Reversed: the two ends are swapped, not collapsed.
        let (text, _, _) = splice("abcd", 3..1, "z", false, None);
        assert_eq!(text, "azd");
    }
}
