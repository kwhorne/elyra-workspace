//! Rendering of a thread transcript: messages, reasoning, tool calls with
//! their results, subagents, plans, task lists, questions, approvals and
//! turn summaries.
//!
//! This module has the rows and the item dispatch; the cards live beside it:
//! `tools` (tool calls, the task list, subagents), `checks` (checks,
//! invariants, testing the tests), `history` (errors fixed, stuck turns,
//! rewritten intent, undo, bisect) and `decisions` (proposals, approvals,
//! plans, cross-examination, questions).

use crate::thread_session::ThreadSession;
use crate::thread_view::{QuestionForm, ThreadView};
use elyra_core::{
    ApprovalDecision, ItemContent, ItemId, PermissionMode, QuestionAnswer, QuestionKind,
    QuestionRequest, TranscriptItem,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

mod checks;
mod decisions;
mod history;
mod tools;

use checks::{
    Check, check_row, checks_follow, invariant_proposal_card, invariants_card,
    invariants_held_before, mutation_card,
};
use decisions::{approval_card, cross_examination_card, plan_card, proposal_card, question_card};
use history::{bisect_card, error_fixed_card, rewrote_card, stuck_card, undo_card};
use tools::{subagent_card, todo_card, tool_row};

const MAX_RESULT_CHARS: usize = 6000;
const COLLAPSE_USER_LINES: usize = 14;

/// One-line human summary of a tool call's input.
pub fn tool_summary(name: &str, input: &Value) -> String {
    let field = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_string);
    let summary = match name {
        "Bash" | "bash" => field("command"),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "read" | "write" | "edit" => {
            field("file_path")
                .or_else(|| field("notebook_path"))
                .or_else(|| field("path"))
        }
        "Grep" | "grep" => field("pattern").map(|p| match field("path") {
            Some(path) => format!("{p}  in {path}"),
            None => p,
        }),
        "Glob" | "find" | "ls" => field("pattern").or_else(|| field("path")),
        "WebFetch" => field("url"),
        "WebSearch" => field("query"),
        "Task" | "Agent" => field("description"),
        "TodoWrite" => input
            .get("todos")
            .and_then(Value::as_array)
            .map(|todos| format!("{} items", todos.len())),
        _ => None,
    };
    let summary = summary.or_else(|| {
        input.as_object().and_then(|object| {
            object
                .values()
                .find_map(|value| value.as_str().map(str::to_string))
        })
    });
    let summary = summary.unwrap_or_default();
    let first_line = summary.lines().next().unwrap_or("").to_string();
    if first_line.chars().count() > 140 {
        format!("{}…", first_line.chars().take(140).collect::<String>())
    } else if summary.lines().count() > 1 {
        format!("{first_line} …")
    } else {
        first_line
    }
}

fn tool_icon(name: &str) -> IconName {
    match name {
        "Bash" | "bash" => IconName::SquareTerminal,
        "Read" | "read" => IconName::FileText,
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "write" | "edit" => IconName::FilePen,
        "Grep" | "Glob" | "WebSearch" | "grep" | "find" | "ls" => IconName::Search,
        "WebFetch" => IconName::Globe,
        "Task" | "Agent" => IconName::Bot,
        "TodoWrite" => IconName::ListTodo,
        _ => IconName::Wrench,
    }
}

fn is_subagent(name: &str) -> bool {
    matches!(name, "Task" | "Agent")
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let head: String = text.chars().take(max).collect();
        format!("{head}\n… ({} more characters)", text.chars().count() - max)
    }
}

fn mono_block(text: String, cx: &App) -> Div {
    div()
        .w_full()
        .p_2()
        .rounded_md()
        .bg(cx.theme().muted)
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(cx.theme().mono_font_size * 0.92)
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

/// Old/new text rendered as removed/added lines.
fn edit_preview(old: &str, new: &str, cx: &App) -> Div {
    let removed = cx.theme().danger.opacity(0.12);
    let added = cx.theme().success.opacity(0.12);
    let line = |sign: &'static str, text: &str, bg: Hsla| {
        h_flex()
            .w_full()
            .bg(bg)
            .child(div().w(px(14.)).flex_none().child(sign))
            .child(
                div()
                    .flex_1()
                    .whitespace_nowrap()
                    .child(text.replace('\t', "    ")),
            )
    };
    let mut rows: Vec<AnyElement> = Vec::new();
    for text in old.lines().take(80) {
        rows.push(line("-", text, removed).into_any_element());
    }
    for text in new.lines().take(80) {
        rows.push(line("+", text, added).into_any_element());
    }
    v_flex()
        .w_full()
        .p_1()
        .rounded_md()
        .overflow_hidden()
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(cx.theme().mono_font_size * 0.92)
        .children(rows)
}

/// Where each tool call's result and a subagent's steps are (by position in
/// the transcript), and which entries get a row of their own. Kept by the
/// view and extended as entries arrive, instead of rebuilt every frame.
#[derive(Default)]
pub struct RowIndex {
    /// How many entries were indexed, and the last one's id.
    seen: (usize, Option<ItemId>),
    results: HashMap<String, usize>,
    children: HashMap<String, Vec<usize>>,
    latest_todo: Option<ItemId>,
    /// The entries with a row of their own, by position.
    pub rows: Vec<usize>,
}

/// What [`RowIndex::update`] did to the rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowChange {
    Unchanged,
    /// Rows were added after the existing ones.
    Appended {
        from: usize,
        count: usize,
    },
    /// The transcript changed otherwise; every row may have moved.
    Rebuilt,
}

impl RowIndex {
    /// Bring the index up to date with `items`.
    pub fn update(&mut self, items: &[TranscriptItem]) -> RowChange {
        let (seen, last) = self.seen;
        if seen == items.len() && last == items.last().map(|i| i.id) {
            return RowChange::Unchanged;
        }
        // New entries after the ones indexed (the usual case), or a
        // transcript that changed otherwise (start over).
        let appended = seen > 0 && seen <= items.len() && items.get(seen - 1).map(|i| i.id) == last;
        let from = if appended {
            seen
        } else {
            *self = Self::default();
            0
        };
        let rows_before = self.rows.len();
        for (index, item) in items.iter().enumerate().skip(from) {
            match &item.content {
                ItemContent::ToolResult { tool_use_id, .. } => {
                    self.results.insert(tool_use_id.clone(), index);
                    continue;
                }
                ItemContent::ToolUse {
                    name,
                    parent_tool_use_id,
                    ..
                } => {
                    if name == "TodoWrite" {
                        self.latest_todo = Some(item.id);
                    }
                    if let Some(parent) = parent_tool_use_id {
                        self.children.entry(parent.clone()).or_default().push(index);
                        continue;
                    }
                }
                ItemContent::Assistant {
                    parent_tool_use_id: Some(parent),
                    ..
                } => {
                    self.children.entry(parent.clone()).or_default().push(index);
                    continue;
                }
                _ => {}
            }
            self.rows.push(index);
        }
        self.seen = (items.len(), items.last().map(|i| i.id));
        if appended {
            RowChange::Appended {
                from: rows_before,
                count: self.rows.len() - rows_before,
            }
        } else {
            RowChange::Rebuilt
        }
    }

    /// The row an entry shows in: its own, or for a tool result or a
    /// subagent's step, the call it belongs to.
    pub fn row_of(&self, items: &[TranscriptItem], item: ItemId) -> Option<usize> {
        let index = items.iter().position(|i| i.id == item)?;
        let owner = match &items[index].content {
            ItemContent::ToolResult { tool_use_id, .. } => self.call_index(items, tool_use_id),
            ItemContent::ToolUse {
                parent_tool_use_id: Some(parent),
                ..
            }
            | ItemContent::Assistant {
                parent_tool_use_id: Some(parent),
                ..
            } => self.call_index(items, parent),
            _ => Some(index),
        }?;
        self.rows.binary_search(&owner).ok()
    }

    fn call_index(&self, items: &[TranscriptItem], tool_use_id: &str) -> Option<usize> {
        items.iter().position(|i| {
            matches!(&i.content, ItemContent::ToolUse { tool_use_id: id, .. } if id == tool_use_id)
        })
    }
}

/// The index read against the transcript it was built from.
struct Lookup<'a> {
    items: &'a [TranscriptItem],
    index: &'a RowIndex,
}

impl<'a> Lookup<'a> {
    fn result(&self, tool_use_id: &str) -> Option<(&'a str, bool)> {
        let index = *self.index.results.get(tool_use_id)?;
        match &self.items.get(index)?.content {
            ItemContent::ToolResult {
                content, is_error, ..
            } => Some((content.as_str(), *is_error)),
            _ => None,
        }
    }

    fn children(&self, tool_use_id: &str) -> Vec<&'a TranscriptItem> {
        self.index
            .children
            .get(tool_use_id)
            .map(|indices| indices.iter().filter_map(|i| self.items.get(*i)).collect())
            .unwrap_or_default()
    }
}

/// The row of the entry at `position` (one of `index.rows`).
pub fn row(
    session: &ThreadSession,
    index: &RowIndex,
    position: usize,
    expanded: &HashSet<ItemId>,
    forms: &HashMap<String, QuestionForm>,
    highlight: Option<ItemId>,
    cx: &Context<ThreadView>,
) -> Option<AnyElement> {
    let item = session.items.get(position)?;
    let lookup = Lookup {
        items: &session.items,
        index,
    };
    let element = render_item(item, session, &lookup, expanded, forms, cx)?;
    Some(if highlight == Some(item.id) {
        div()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().ring)
            .p_1()
            .child(element)
            .into_any_element()
    } else {
        element
    })
}

/// What follows the entries while a turn runs: the thinking and reply being
/// streamed, and what the agent is doing. `reply` is the streaming reply's
/// text state, extended chunk by chunk (parsed incrementally).
pub fn tail(
    session: &ThreadSession,
    reply: Option<&Entity<TextViewState>>,
    cx: &Context<ThreadView>,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    if !session.streaming_thinking.is_empty() && session.streaming_text.is_empty() {
        // The last 400 characters, without copying the whole text.
        let thinking = &session.streaming_thinking;
        let start = thinking
            .char_indices()
            .rev()
            .nth(399)
            .map(|(at, _)| at)
            .unwrap_or(0);
        rows.push(
            div()
                .text_sm()
                .italic()
                .text_color(cx.theme().muted_foreground)
                .child(thinking[start..].to_string())
                .into_any_element(),
        );
    }
    if !session.streaming_text.is_empty() {
        rows.push(match reply {
            Some(state) => TextView::new(state).selectable(true).into_any_element(),
            None => TextView::markdown("md-streaming", session.streaming_text.clone())
                .selectable(true)
                .into_any_element(),
        });
    }
    if let Some(preparing) = &session.preparing {
        rows.push(working_row(preparing.clone(), cx));
    } else if let Some(activity) = &session.activity {
        rows.push(working_row(activity.clone(), cx));
    } else if session.running && session.streaming_text.is_empty() && !session.pending_approval() {
        rows.push(working_row("Working…".into(), cx));
    }
    rows
}

fn hover_actions(
    id: ItemId,
    text: String,
    editable: bool,
    restore: Option<String>,
    pinned: bool,
    cx: &Context<ThreadView>,
) -> Div {
    let copy = text.clone();
    h_flex()
        .gap_0p5()
        .when(!pinned, |this| {
            this.invisible()
                .group_hover("message", |style| style.visible())
        })
        .child(
            Button::new(SharedString::from(format!("pin-{id}")))
                .ghost()
                .xsmall()
                .icon(if pinned {
                    IconName::PinOff
                } else {
                    IconName::Pin
                })
                .tooltip(if pinned {
                    "Unpin"
                } else {
                    "Pin to context (⇧⌘I)"
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.session
                        .update(cx, |session, cx| session.toggle_pin(id, cx))
                })),
        )
        .child(
            Button::new(SharedString::from(format!("copy-{id}")))
                .ghost()
                .xsmall()
                .icon(IconName::Copy)
                .tooltip("Copy")
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                }),
        )
        .when(editable, |this| {
            this.child(
                Button::new(SharedString::from(format!("edit-{id}")))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Pencil)
                    .tooltip("Edit and resend")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_message(text.clone(), window, cx)
                    })),
            )
        })
        .when(restore.is_some(), |this| {
            this.child(
                Button::new(SharedString::from(format!("undo-{id}")))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Undo2)
                    .tooltip(
                        "Undo this instruction: take back what it changed, keep what came after",
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.confirm_undo(id, window, cx)),
                    ),
            )
        })
        .when_some(restore, |this, sha| {
            this.child(
                Button::new(SharedString::from(format!("restore-{id}")))
                    .ghost()
                    .xsmall()
                    .icon(IconName::RotateCcwClock)
                    .tooltip("Restore files to before this message")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.confirm_restore(sha.clone(), window, cx)
                    })),
            )
        })
}

fn render_item(
    item: &TranscriptItem,
    session: &ThreadSession,
    lookup: &Lookup,
    expanded: &HashSet<ItemId>,
    forms: &HashMap<String, QuestionForm>,
    cx: &Context<ThreadView>,
) -> Option<AnyElement> {
    let id = item.id;
    Some(match &item.content {
        ItemContent::User { text, .. }
            if let Some(label) = crate::checks::fix_prompt_label(text) =>
        {
            // Sent by Elyra, not the user: fold it into one line.
            let open = expanded.contains(&id);
            v_flex()
                .w_full()
                .gap_1()
                .child(
                    h_flex()
                        .id(SharedString::from(format!("fix-{id}")))
                        .gap_2()
                        .cursor_pointer()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(Icon::new(IconName::CornerDownLeft).xsmall())
                        .child(
                            if text.starts_with(crate::checks::START_OVER_PROMPT_START) {
                                "Sent the agent at it again".to_string()
                            } else if text.starts_with(crate::cross_examine::REVISE_START) {
                                "Sent the objections you took to the agent".to_string()
                            } else if text.starts_with(crate::mutation::ASK_START) {
                                "Asked the agent for tests that catch the changes".to_string()
                            } else {
                                format!("Sent the failures to the agent · {label}")
                            },
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_expanded(id, cx))),
                )
                .when(open, |this| {
                    this.child(mono_block(truncate(text, MAX_RESULT_CHARS), cx))
                })
                .into_any_element()
        }
        ItemContent::User {
            text, checkpoint, ..
        } => {
            let long = text.lines().count() > COLLAPSE_USER_LINES || text.len() > 2400;
            let open = expanded.contains(&id);
            let shown = if long && !open {
                let head: String = text
                    .lines()
                    .take(COLLAPSE_USER_LINES)
                    .collect::<Vec<_>>()
                    .join("\n");
                head.chars().take(2400).collect::<String>()
            } else {
                text.clone()
            };
            v_flex()
                .group("message")
                .w_full()
                .items_end()
                .gap_0p5()
                .child(
                    v_flex()
                        .max_w(relative(0.85))
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .bg(cx.theme().secondary)
                        .text_color(cx.theme().secondary_foreground)
                        .child(shown)
                        .when(long, |this| {
                            this.child(
                                div()
                                    .id(SharedString::from(format!("more-{id}")))
                                    .pt_1()
                                    .text_xs()
                                    .cursor_pointer()
                                    .text_color(cx.theme().link)
                                    .child(if open { "Show less" } else { "Show more" })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_expanded(id, cx)
                                    })),
                            )
                        }),
                )
                .child(hover_actions(
                    id,
                    text.clone(),
                    true,
                    checkpoint.clone(),
                    session.thread.pinned_items.contains(&id),
                    cx,
                ))
                .into_any_element()
        }
        ItemContent::Assistant { text, .. } => v_flex()
            .group("message")
            .w_full()
            .gap_0p5()
            .child(
                TextView::markdown(SharedString::from(format!("md-{id}")), text.clone())
                    .selectable(true),
            )
            .child(hover_actions(
                id,
                text.clone(),
                false,
                None,
                session.thread.pinned_items.contains(&id),
                cx,
            ))
            .into_any_element(),
        ItemContent::Thinking { text } => {
            let open = expanded.contains(&id);
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .id(SharedString::from(format!("think-{id}")))
                        .gap_1()
                        .cursor_pointer()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(Icon::new(IconName::Brain).xsmall())
                        .child("Thinking")
                        .child(
                            Icon::new(if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .xsmall(),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_expanded(id, cx))),
                )
                .when(open, |this| {
                    this.child(
                        div()
                            .pl_4()
                            .text_sm()
                            .italic()
                            .text_color(cx.theme().muted_foreground)
                            .child(text.clone()),
                    )
                })
                .into_any_element()
        }
        ItemContent::ToolUse {
            tool_use_id,
            name,
            input,
            ..
        } => {
            if name == "TodoWrite" {
                return Some(todo_card(
                    id,
                    input,
                    lookup.index.latest_todo == Some(id),
                    cx,
                ));
            }
            if is_subagent(name) {
                return Some(subagent_card(
                    item,
                    tool_use_id,
                    input,
                    session,
                    lookup,
                    expanded,
                    cx,
                ));
            }
            tool_row(id, tool_use_id, name, input, session, lookup, expanded, cx)
        }
        ItemContent::ToolResult { .. } => return None,
        ItemContent::Approval {
            request_id,
            tool_name,
            description,
            input,
            decision,
        } => {
            if tool_name == "ExitPlanMode" {
                let examining = session.items.iter().any(|item| {
                    matches!(&item.content, ItemContent::CrossExamination { request_id: r, .. } if r == request_id)
                });
                return Some(plan_card(
                    id,
                    request_id,
                    input,
                    *decision,
                    session.thread.provider,
                    examining || session.thread.parent_id.is_some(),
                    cx,
                ));
            }
            approval_card(id, request_id, tool_name, description, input, *decision, cx)
        }
        ItemContent::Question { request, answer } => question_card(
            id,
            request,
            answer.as_ref(),
            forms.get(&request.request_id),
            cx,
        ),
        ItemContent::TurnSummary {
            duration_ms,
            cost_usd,
            is_error,
            ..
        } => {
            let mut parts = Vec::new();
            parts.push(if *is_error {
                "Turn ended with an error".to_string()
            } else {
                "Done".to_string()
            });
            if let Some(ms) = duration_ms {
                parts.push(format!("{:.1}s", *ms as f64 / 1000.0));
            }
            if let Some(cost) = cost_usd {
                parts.push(format!("${cost:.3}"));
            }
            h_flex()
                .w_full()
                .gap_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(div().h(px(1.)).flex_1().bg(cx.theme().border))
                .child(parts.join(" · "))
                .child(div().h(px(1.)).flex_1().bg(cx.theme().border))
                .into_any_element()
        }
        ItemContent::AutomationProposal {
            automation,
            outcome,
        } => proposal_card(id, automation, *outcome, session, cx),
        ItemContent::PrUpdate {
            number,
            title,
            url,
            commits,
            requested,
            handled,
            ..
        } => {
            let pending = !handled;
            let open = url.clone();
            v_flex()
                .w_full()
                .p_3()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(if pending {
                    cx.theme().warning
                } else {
                    cx.theme().border
                })
                .when(pending, |this| this.bg(cx.theme().warning.opacity(0.06)))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Icon::new(IconName::GitPullRequest)
                                .small()
                                .text_color(cx.theme().warning),
                        )
                        .child(
                            div().font_weight(FontWeight::SEMIBOLD).child(
                                crate::review_follow::summary(*number, *commits, *requested),
                            ),
                        ),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(title.clone()),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .when(pending, |this| {
                            this.child(
                                Button::new(SharedString::from(format!("pr-review-{id}")))
                                    .primary()
                                    .small()
                                    .label("Review the changes")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.review_pr_changes(id, cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new(SharedString::from(format!("pr-open-{id}")))
                                .small()
                                .label("Open on GitHub")
                                .on_click(move |_, _, cx| cx.open_url(&open)),
                        )
                        .when(pending, |this| {
                            this.child(
                                Button::new(SharedString::from(format!("pr-stop-{id}")))
                                    .ghost()
                                    .small()
                                    .label("Stop following")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.stop_following_pr(id, cx)
                                    })),
                            )
                        }),
                )
                .into_any_element()
        }
        ItemContent::RuleProposal {
            rule,
            reason,
            paths,
            outcome,
        } => {
            use elyra_core::RuleOutcome;
            let pending = outcome.is_none();
            let dir = session.working_dir();
            let file = crate::rules::destination(&dir, paths.as_deref());
            let file_name = file
                .strip_prefix(&dir)
                .unwrap_or(&file)
                .display()
                .to_string();
            let title = match outcome {
                None => "Proposed rule".to_string(),
                Some(RuleOutcome::File) => format!("Rule added to {file_name}"),
                Some(RuleOutcome::Instructions) => {
                    "Rule added to the project's instructions".into()
                }
                Some(RuleOutcome::Dismissed) => "Rule dismissed".into(),
            };
            let mut card = v_flex()
                .w_full()
                .p_3()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(if pending {
                    cx.theme().info
                } else {
                    cx.theme().border
                })
                .when(pending, |this| this.bg(cx.theme().info.opacity(0.06)))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Icon::new(IconName::Lightbulb)
                                .small()
                                .text_color(cx.theme().info),
                        )
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(title)),
                )
                .child(div().text_sm().whitespace_normal().child(rule.clone()))
                .when(!reason.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .whitespace_normal()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Learned from: {reason}")),
                    )
                })
                .when_some(paths.clone(), |this, paths| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(format!("Applies to {paths}")),
                    )
                });
            if pending {
                card = card.child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(SharedString::from(format!("rule-file-{id}")))
                                .primary()
                                .small()
                                .label(format!("Add to {file_name}"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.keep_rule(id, true, window, cx)
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("rule-instructions-{id}")))
                                .small()
                                .label("Add to project instructions")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.keep_rule(id, false, window, cx)
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("rule-check-{id}")))
                                .small()
                                .icon(IconName::ShieldCheck)
                                .label("Save as check")
                                .tooltip("Ask the agent to turn it into a check Elyra runs after every turn (it proposes it here)")
                                .disabled(session.running)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.session
                                        .update(cx, |session, cx| session.rule_as_check(id, cx))
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("rule-dismiss-{id}")))
                                .ghost()
                                .small()
                                .label("Dismiss")
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.dismiss_rule(id, cx)),
                                ),
                        ),
                );
            }
            card.into_any_element()
        }
        // Held, with checks after them: said on the checks' row instead.
        ItemContent::Invariants { broken, .. }
            if broken.is_empty() && checks_follow(&session.items, id) =>
        {
            return None;
        }
        ItemContent::Invariants { checked, broken } => invariants_card(checked, broken, cx),
        ItemContent::InvariantProposal {
            invariant,
            holds,
            outcome,
        } => invariant_proposal_card(id, invariant, holds.as_deref(), *outcome, cx),
        ItemContent::Journey {
            name,
            passed,
            steps_run,
            total_steps,
            failure,
            duration_ms,
            layout,
            layout_saved,
            layout_accepted,
            queries,
        } => {
            // Still works, but looks or queries differently.
            let layout_changed = !layout.is_empty() || !queries.is_empty();
            let (icon, color, label) = if *passed {
                (IconName::Route, cx.theme().success, "Journey passed")
            } else if layout_changed && *layout_accepted {
                (
                    IconName::Route,
                    cx.theme().muted_foreground,
                    "Journey change accepted",
                )
            } else if !queries.is_empty() {
                (
                    IconName::Route,
                    cx.theme().warning,
                    "Journey queries jumped",
                )
            } else if layout_changed {
                (
                    IconName::Route,
                    cx.theme().warning,
                    "Journey layout changed",
                )
            } else {
                (IconName::Route, cx.theme().danger, "Journey failed")
            };
            let summary = if *passed || layout_changed {
                format!(
                    "{name} · {total_steps} steps · {:.1}s",
                    *duration_ms as f64 / 1000.0
                )
            } else {
                format!("{name} · step {} of {total_steps}", steps_run + 1)
            };
            v_flex()
                .w_full()
                .rounded_md()
                .border_1()
                .border_color(if *passed { cx.theme().border } else { color })
                .child(
                    h_flex()
                        .w_full()
                        .px_2()
                        .py_1()
                        .gap_2()
                        .text_sm()
                        .child(Icon::new(icon).small().text_color(color))
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::MEDIUM)
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(summary),
                        ),
                )
                .when_some(
                    failure.clone().filter(|_| !layout_changed),
                    |this, failure| {
                        this.child(
                            div()
                                .px_2()
                                .pb_1()
                                .text_xs()
                                .whitespace_normal()
                                .text_color(cx.theme().muted_foreground)
                                .child(failure),
                        )
                    },
                )
                .when(layout_changed, |this| {
                    this.child(
                        v_flex()
                            .px_2()
                            .pb_1()
                            .gap_1()
                            .text_xs()
                            .children(queries.iter().map(|jump| {
                                v_flex()
                                    .gap_0p5()
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .text_color(cx.theme().warning)
                                            .child(Icon::new(IconName::Database).xsmall())
                                            .child(div().flex_1().min_w_0().whitespace_normal().child(
                                                format!(
                                                    "after \u{201c}{}\u{201d}, {} database queries instead of {}",
                                                    jump.step_text, jump.after, jump.before
                                                ),
                                            )),
                                    )
                                    .when_some(jump.repeated.clone(), |this, repeated| {
                                        this.child(
                                            h_flex()
                                                .pl_4()
                                                .gap_2()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .overflow_hidden()
                                                        .whitespace_nowrap()
                                                        .text_ellipsis()
                                                        .font_family(cx.theme().mono_font_family.clone())
                                                        .child(repeated),
                                                )
                                                .child(div().flex_none().child(format!("{}×", jump.times))),
                                        )
                                    })
                            }))
                            .children(layout.iter().take(8).map(|problem| {
                                div()
                                    .whitespace_normal()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::journeys::describe_problem(problem))
                            }))
                            .when(!*layout_accepted, |this| {
                                this.child(
                                    h_flex().child(
                                        Button::new(SharedString::from(format!(
                                            "layout-accept-{id}"
                                        )))
                                        .xsmall()
                                        .outline()
                                        .label(match (layout.is_empty(), queries.is_empty()) {
                                            (false, false) => "Accept as it is now",
                                            (true, false) => "Accept these query counts",
                                            _ => "Accept this layout",
                                        })
                                        .tooltip("The change is intended: don't report these again")
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.session.update(cx, |session, cx| {
                                                    session.accept_layout(id, cx)
                                                })
                                            }),
                                        ),
                                    ),
                                )
                            }),
                    )
                })
                .when_some(*layout_saved, |this, count| {
                    this.child(
                        div()
                            .px_2()
                            .pb_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "Its pages' layout was saved{}; later replays report what changes.",
                                match count {
                                    0 => String::new(),
                                    1 => " (with 1 problem already there)".into(),
                                    n => format!(" (with {n} problems already there)"),
                                }
                            )),
                    )
                })
                .into_any_element()
        }
        ItemContent::Check {
            command,
            passed,
            exit_code,
            duration_ms,
            output,
        } => check_row(
            id,
            Check {
                command,
                passed: *passed,
                exit_code: *exit_code,
                duration_ms: *duration_ms,
                output,
                invariants: invariants_held_before(&session.items, id),
            },
            expanded.contains(&id),
            !*passed && !session.running,
            cx,
        ),
        ItemContent::Bisect { command, verdict } => bisect_card(command, verdict, cx),
        ItemContent::MutationGate {
            command,
            mutants,
            state,
            asked,
        } => mutation_card(id, command, mutants, state, *asked, session.running, cx),
        ItemContent::CrossExamination {
            request_id,
            examiner,
            chat,
            objections,
            state,
        } => cross_examination_card(
            id, request_id, *examiner, *chat, objections, state, session, cx,
        ),
        ItemContent::Stuck {
            reason,
            detail,
            outcome,
        } => stuck_card(id, reason, detail, *outcome, session, cx),
        ItemContent::Undo {
            instruction,
            reverted,
            kept,
            put_back,
            ..
        } => undo_card(
            id,
            instruction,
            reverted,
            kept,
            *put_back,
            session.running,
            cx,
        ),
        ItemContent::Rewrote { instructions, .. } => rewrote_card(id, instructions, cx),
        ItemContent::ErrorFixed { errors, test, .. } => {
            error_fixed_card(id, errors, test.as_ref(), session.running, cx)
        }
        ItemContent::PageSnapshot { url, before, after } => {
            let picture = |label: &'static str, path: &str| {
                let path = std::path::PathBuf::from(path);
                let open = path.clone();
                v_flex()
                    .id(SharedString::from(format!("snapshot-{label}-{id}")))
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| cx.open_with_system(&open))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .child(
                        img(path)
                            .w_full()
                            .max_h(px(220.))
                            .object_fit(ObjectFit::Contain)
                            .rounded_md()
                            .border_1()
                            .border_color(cx.theme().border),
                    )
            };
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(Icon::new(IconName::Globe).xsmall())
                        .child(format!("The page after this turn · {url}")),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .children(before.as_deref().map(|before| picture("Before", before)))
                        .child(picture("After", after)),
                )
                .into_any_element()
        }
        ItemContent::Notice { text, is_error } => h_flex()
            .w_full()
            .items_start()
            .gap_2()
            .text_sm()
            .text_color(if *is_error {
                cx.theme().danger
            } else {
                cx.theme().muted_foreground
            })
            .child(
                Icon::new(if *is_error {
                    IconName::CircleAlert
                } else {
                    IconName::Info
                })
                .small(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal()
                    .child(text.clone()),
            )
            .into_any_element(),
    })
}

fn working_row(label: String, cx: &App) -> AnyElement {
    h_flex()
        .gap_2()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(Spinner::new().small())
        .child(label)
        .into_any_element()
}

/// Plain text of an item, for find-in-thread.
pub fn searchable_text(item: &TranscriptItem) -> Option<String> {
    match &item.content {
        ItemContent::User { text, .. } | ItemContent::Assistant { text, .. } => Some(text.clone()),
        ItemContent::ToolUse { name, input, .. } => {
            Some(format!("{name} {}", tool_summary(name, input)))
        }
        ItemContent::Notice { text, .. } => Some(text.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{RowChange, RowIndex, tool_summary};
    use elyra_core::{ItemContent, TranscriptItem};
    use serde_json::json;

    fn item(content: ItemContent) -> TranscriptItem {
        TranscriptItem {
            id: elyra_core::new_id(),
            thread_id: elyra_core::new_id(),
            seq: 0,
            content,
            created_at: chrono::Utc::now(),
        }
    }

    fn call(id: &str, parent: Option<&str>) -> TranscriptItem {
        item(ItemContent::ToolUse {
            tool_use_id: id.into(),
            name: "Bash".into(),
            input: json!({ "command": "ls" }),
            parent_tool_use_id: parent.map(str::to_string),
        })
    }

    fn result(id: &str) -> TranscriptItem {
        item(ItemContent::ToolResult {
            tool_use_id: id.into(),
            content: "ok".into(),
            is_error: false,
        })
    }

    #[test]
    fn keeps_rows_in_step_with_the_transcript() {
        let mut items = vec![
            item(ItemContent::User {
                text: "Fix it".into(),
                checkpoint: None,
                attachments: Vec::new(),
            }),
            call("task", None),
            call("inner", Some("task")),
            result("inner"),
            result("task"),
        ];
        let mut index = RowIndex::default();
        assert_eq!(index.update(&items), RowChange::Rebuilt);
        assert_eq!(index.rows, [0, 1], "the message and the subagent call");
        assert_eq!(index.update(&items), RowChange::Unchanged);
        // A result and a subagent's step show in their call's row.
        assert_eq!(index.row_of(&items, items[4].id), Some(1));
        assert_eq!(index.row_of(&items, items[2].id), Some(1));
        assert_eq!(index.row_of(&items, items[0].id), Some(0));

        items.push(call("t2", None));
        items.push(result("t2"));
        items.push(item(ItemContent::Assistant {
            text: "Done".into(),
            parent_tool_use_id: None,
        }));
        assert_eq!(
            index.update(&items),
            RowChange::Appended { from: 2, count: 2 }
        );
        assert_eq!(index.rows, [0, 1, 5, 7]);

        // Anything but appending starts over.
        items.remove(0);
        assert_eq!(index.update(&items), RowChange::Rebuilt);
        assert_eq!(index.rows, [0, 4, 6]);
    }

    #[test]
    fn summarizes_tool_inputs() {
        assert_eq!(
            tool_summary("Bash", &json!({"command": "cargo test"})),
            "cargo test"
        );
        assert_eq!(tool_summary("bash", &json!({"command": "ls"})), "ls");
        assert_eq!(
            tool_summary("Read", &json!({"file_path": "/a/b.rs"})),
            "/a/b.rs"
        );
        assert_eq!(
            tool_summary("read", &json!({"path": "src/x.ts"})),
            "src/x.ts"
        );
        assert_eq!(
            tool_summary("Grep", &json!({"pattern": "fn", "path": "src"})),
            "fn  in src"
        );
        assert_eq!(tool_summary("Bash", &json!({"command": "a\nb"})), "a …");
        assert_eq!(
            tool_summary("Custom", &json!({"x": 1, "y": "hello"})),
            "hello"
        );
    }
}
