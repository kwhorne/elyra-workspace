//! Rendering of a thread transcript: messages, reasoning, tool calls with
//! their results, subagents, plans, task lists, questions, approvals and
//! turn summaries.

use crate::thread_session::ThreadSession;
use crate::thread_view::{QuestionForm, ThreadView};
use elyra_core::{
    ApprovalDecision, ItemContent, ItemId, PermissionMode, QuestionAnswer, QuestionKind,
    QuestionRequest, TranscriptItem,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

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
                        .child(format!("Sent the failures to the agent · {label}"))
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
                return Some(plan_card(id, request_id, input, *decision, cx));
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
        ItemContent::Journey {
            name,
            passed,
            steps_run,
            total_steps,
            failure,
            duration_ms,
        } => {
            let (icon, color, label) = if *passed {
                (IconName::Route, cx.theme().success, "Journey passed")
            } else {
                (IconName::Route, cx.theme().danger, "Journey failed")
            };
            let summary = if *passed {
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
                .when_some(failure.clone(), |this, failure| {
                    this.child(
                        div()
                            .px_2()
                            .pb_1()
                            .text_xs()
                            .whitespace_normal()
                            .text_color(cx.theme().muted_foreground)
                            .child(failure),
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
            },
            expanded.contains(&id),
            !*passed && !session.running,
            cx,
        ),
        ItemContent::Bisect { command, verdict } => bisect_card(command, verdict, cx),
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

fn status_icon(result: Option<(&str, bool)>, running: bool, cx: &App) -> AnyElement {
    match result {
        None if running => Spinner::new().xsmall().into_any_element(),
        None => Icon::new(IconName::CircleDashed)
            .xsmall()
            .text_color(cx.theme().muted_foreground)
            .into_any_element(),
        Some((_, true)) => Icon::new(IconName::CircleX)
            .xsmall()
            .text_color(cx.theme().danger)
            .into_any_element(),
        Some((_, false)) => Icon::new(IconName::CircleCheck)
            .xsmall()
            .text_color(cx.theme().success)
            .into_any_element(),
    }
}

/// An automation the agent proposed: what, when and where, with Create,
/// Edit and Dismiss until the user decides.
fn proposal_card(
    id: ItemId,
    automation: &elyra_core::Automation,
    outcome: Option<elyra_core::ProposalOutcome>,
    session: &ThreadSession,
    cx: &Context<ThreadView>,
) -> AnyElement {
    use elyra_core::ProposalOutcome;
    let pending = outcome.is_none();
    let title = match outcome {
        None => "Proposed automation",
        Some(ProposalOutcome::Created) => "Automation created",
        Some(ProposalOutcome::Dismissed) => "Automation proposal dismissed",
    };
    let next = automation
        .next_run_at
        .map(|t| {
            format!(
                " · next {}",
                t.with_timezone(&chrono::Local).format("%a %d.%m %H:%M")
            )
        })
        .unwrap_or_default();
    let project = if automation.project_id == session.project.id {
        session.project.name.clone()
    } else {
        "another project".to_string()
    };
    let mut runs = format!(
        "{} in {project} · {}",
        automation.provider.label(),
        automation.permission_mode.label()
    );
    if let Some(model) = &automation.model {
        runs.push_str(&format!(" · {model}"));
    }
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
                    Icon::new(IconName::CalendarClock)
                        .small()
                        .text_color(cx.theme().info),
                )
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title)),
        )
        .child(
            v_flex()
                .gap_1()
                .text_sm()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(automation.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{}{next}", automation.schedule.describe())),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(runs),
                ),
        )
        .child(mono_block(truncate(&automation.prompt, 1200), cx));
    if pending {
        card = card.child(
            h_flex()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("proposal-create-{id}")))
                        .primary()
                        .small()
                        .label("Create")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.create_proposal(id, window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("proposal-edit-{id}")))
                        .small()
                        .label("Edit…")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_proposal(id, window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("proposal-dismiss-{id}")))
                        .ghost()
                        .small()
                        .label("Dismiss")
                        .on_click(cx.listener(move |this, _, _, cx| this.dismiss_proposal(id, cx))),
                ),
        );
    }
    card.into_any_element()
}

/// The project's checks after a turn: passed or failed, with the output
/// (the last line shows while folded, when they failed).
#[allow(clippy::too_many_arguments)]
/// An instruction taken back.
/// The turn took out lines written for other instructions.
fn rewrote_card(
    id: ItemId,
    instructions: &[elyra_core::RewrittenIntent],
    cx: &Context<ThreadView>,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let mut card = v_flex()
        .w_full()
        .p_2()
        .gap_2()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().warning.opacity(0.5))
        .bg(cx.theme().warning.opacity(0.05))
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .small()
                        .text_color(cx.theme().warning),
                )
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child("This turn took out lines written for other instructions"),
                ),
        );
    for (index, intent) in instructions.iter().enumerate() {
        let (thread, asked) = (intent.thread, intent.asked);
        card = card.child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .whitespace_normal()
                        .child(format!("“{}”", intent.instruction)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(crate::intent_guard::summary(intent)),
                )
                .child(
                    v_flex()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(cx.theme().danger.opacity(0.06))
                        .text_xs()
                        .font_family(cx.theme().mono_font_family.clone())
                        .children(intent.lines.iter().take(3).map(|line| {
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(format!("- {line}"))
                        })),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(match asked {
                            None => Button::new(SharedString::from(format!("rewrite-ask-{id}-{index}")))
                                .xsmall()
                                .outline()
                                .icon(IconName::MessagesSquare)
                                .label("Ask the author")
                                .tooltip("Ask the agent that wrote these lines, as it was then, whether the new code still does what it was asked to do")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.ask_rewrite_author(id, index, window, cx)
                                })),
                            Some(chat) => Button::new(SharedString::from(format!("rewrite-answer-{id}-{index}")))
                                .xsmall()
                                .outline()
                                .icon(IconName::MessagesSquare)
                                .label("Open the answer")
                                .on_click(move |_, window, cx| {
                                    if let Some(hub) = cx.try_global::<crate::browser_tools::BrowserHub>() {
                                        let workspace = hub.workspace.clone();
                                        let _ = workspace.update(cx, |this, cx| {
                                            this.show_side_chat(chat, window, cx)
                                        });
                                    }
                                }),
                        })
                        .child(
                            Button::new(SharedString::from(format!("rewrite-open-{id}-{index}")))
                                .xsmall()
                                .ghost()
                                .label("Open its thread")
                                .on_click(move |_, window, cx| {
                                    if let Some(hub) = cx.try_global::<crate::browser_tools::BrowserHub>() {
                                        let workspace = hub.workspace.clone();
                                        let _ = workspace
                                            .update(cx, |this, cx| this.activate(thread, window, cx));
                                    }
                                }),
                        ),
                ),
        );
    }
    card.child(
        div()
            .text_xs()
            .text_color(muted)
            .child("The agent hears of it with your next message."),
    )
    .into_any_element()
}

fn undo_card(
    id: ItemId,
    instruction: &str,
    reverted: &[String],
    kept: &[String],
    put_back: bool,
    running: bool,
    cx: &Context<ThreadView>,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let headline = instruction
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string();
    let list = |paths: &[String]| {
        paths
            .iter()
            .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let prompt = crate::undo::finish_prompt(instruction, kept, reverted);
    v_flex()
        .w_full()
        .p_2()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(Icon::new(IconName::Undo2).small().text_color(muted))
                .child(div().font_weight(FontWeight::MEDIUM).child(if put_back {
                    "Undone, then put back"
                } else {
                    "Undone"
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(format!("“{headline}”")),
                ),
        )
        .when(!reverted.is_empty(), |this| {
            this.child(div().text_xs().text_color(muted).child(format!(
                "Back as before it: {}",
                list(reverted)
            )))
        })
        .when(!kept.is_empty(), |this| {
            this.child(div().text_xs().text_color(cx.theme().warning).child(format!(
                "Changed again since, left as they are: {}",
                list(kept)
            )))
        })
        .when(!put_back, |this| {
            this.child(
                h_flex()
                    .gap_2()
                    .when(!kept.is_empty(), |this| {
                        this.child(
                            Button::new(SharedString::from(format!("undo-finish-{id}")))
                                .xsmall()
                                .primary()
                                .label("Ask the agent to finish")
                                .tooltip("Have the agent take the instruction's part out of the files left as they are")
                                .disabled(running)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let prompt = elyra_provider::Prompt::text(prompt.clone());
                                    this.session
                                        .update(cx, |session, cx| session.submit(prompt, cx));
                                })),
                        )
                    })
                    .child(
                        Button::new(SharedString::from(format!("undo-put-back-{id}")))
                            .xsmall()
                            .ghost()
                            .label("Put it back")
                            .tooltip("Bring back what the undo took away; what changed since stays")
                            .disabled(running)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.session
                                    .update(cx, |session, cx| session.put_back_undo(id, cx));
                            })),
                    ),
            )
        })
        .into_any_element()
}

/// What "Who broke it?" found.
fn bisect_card(
    command: &str,
    verdict: &elyra_core::BisectVerdict,
    cx: &Context<ThreadView>,
) -> AnyElement {
    use elyra_core::BisectVerdict;
    let muted = cx.theme().muted_foreground;
    let BisectVerdict::Found {
        thread,
        instruction,
        thread_title,
        agent,
        at,
        output,
        runs,
        ..
    } = verdict
    else {
        return h_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .text_color(muted)
            .child(Icon::new(IconName::Search).small())
            .child(crate::bisect::describe(command, verdict))
            .into_any_element();
    };
    let thread = *thread;
    let headline = instruction
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("(an empty message)")
        .to_string();
    let tail: Vec<&str> = output
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(6)
        .collect();
    let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    let prompt = format!(
        "The project's checks (`{command}`) started failing with the changes made for this message:\n“{headline}”\n\nThey passed before that turn and fail after it. The failing output ends with:\n```\n{}\n```\nFind and fix the cause.",
        output
            .lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );
    let open = move |window: &mut Window, cx: &mut App| {
        if let Some(hub) = cx.try_global::<crate::browser_tools::BrowserHub>() {
            let workspace = hub.workspace.clone();
            let _ = workspace.update(cx, |this, cx| this.activate(thread, window, cx));
        }
    };
    v_flex()
        .w_full()
        .p_2()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().warning)
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(Icon::new(IconName::Search).small().text_color(cx.theme().warning))
                .child(div().font_weight(FontWeight::MEDIUM).child("Broken by"))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(format!("“{headline}”")),
                ),
        )
        .child(div().text_xs().text_color(muted).child(format!(
            "{thread_title} · {agent} · {} · `{command}` passed before that turn and failed after it ({runs} runs)",
            at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
        )))
        .when(!tail.is_empty(), |this| {
            this.child(
                div()
                    .p_1()
                    .rounded_sm()
                    .bg(cx.theme().muted)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .child(tail),
            )
        })
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("bisect-open-{thread}")))
                        .xsmall()
                        .ghost()
                        .label("Open thread")
                        .on_click(move |_, window, cx| open(window, cx)),
                )
                .child(
                    Button::new(SharedString::from(format!("bisect-send-{thread}")))
                        .xsmall()
                        .primary()
                        .label("Send the failure there")
                        .tooltip("Ask the thread that broke it to fix it, with the failing output")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let Some(app) = this.session.read(cx).app_entity() else {
                                return;
                            };
                            if let Some(session) = app.update(cx, |app, cx| app.session(thread, cx)) {
                                let prompt = elyra_provider::Prompt::text(prompt.clone());
                                session.update(cx, |session, cx| session.submit(prompt, cx));
                            }
                            open(window, cx);
                        })),
                ),
        )
        .into_any_element()
}

/// A check entry's fields, for its row.
struct Check<'a> {
    command: &'a str,
    passed: bool,
    exit_code: Option<i32>,
    duration_ms: u64,
    output: &'a str,
}

fn check_row(
    id: ItemId,
    check: Check,
    open: bool,
    can_bisect: bool,
    cx: &Context<ThreadView>,
) -> AnyElement {
    let Check {
        command,
        passed,
        exit_code,
        duration_ms,
        output,
    } = check;
    let bisect_command = command.to_string();
    let mut summary = vec![
        command.to_string(),
        format!("{:.1}s", duration_ms as f64 / 1000.0),
    ];
    if !passed && let Some(code) = exit_code {
        summary.push(format!("exit {code}"));
    }
    let last_line = (!passed && !open)
        .then(|| output.lines().rev().find(|l| !l.trim().is_empty()))
        .flatten()
        .map(|line| line.chars().take(160).collect::<String>());
    let (icon, color, label) = if passed {
        (IconName::CircleCheck, cx.theme().success, "Checks passed")
    } else {
        (IconName::CircleX, cx.theme().danger, "Checks failed")
    };
    v_flex()
        .w_full()
        .rounded_md()
        .border_1()
        .border_color(if passed { cx.theme().border } else { color })
        .child(
            h_flex()
                .id(SharedString::from(format!("check-{id}")))
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .cursor_pointer()
                .hover(|this| this.bg(cx.theme().muted))
                .text_sm()
                .child(Icon::new(icon).small().text_color(color))
                .child(div().font_weight(FontWeight::MEDIUM).child(label))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(summary.join(" · ")),
                )
                .when(can_bisect, |this| {
                    this.child(
                        Button::new(SharedString::from(format!("bisect-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Search)
                            .label("Who broke it?")
                            .tooltip("Run the checks against the states before earlier turns in this folder to find the one that made them fail")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                let command = bisect_command.clone();
                                this.session.update(cx, |session, cx| {
                                    session.find_breaking_change(Some(command), cx)
                                });
                            })),
                    )
                })
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_expanded(id, cx))),
        )
        .when_some(last_line, |this, line| {
            this.child(
                div()
                    .px_2()
                    .pb_1()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(line),
            )
        })
        .when(open && !output.is_empty(), |this| {
            this.child(
                div()
                    .p_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(mono_block(truncate(output, MAX_RESULT_CHARS), cx)),
            )
        })
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn tool_row(
    id: ItemId,
    tool_use_id: &str,
    name: &str,
    input: &Value,
    session: &ThreadSession,
    lookup: &Lookup,
    expanded: &HashSet<ItemId>,
    cx: &Context<ThreadView>,
) -> AnyElement {
    let open = expanded.contains(&id);
    let result = lookup.result(tool_use_id);
    let progress = session.tool_progress.get(tool_use_id);
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    let details: Option<AnyElement> = open.then(|| {
        let mut body = v_flex()
            .p_2()
            .gap_2()
            .border_t_1()
            .border_color(cx.theme().border);
        match name {
            "Edit" | "edit" => {
                body = body.child(edit_preview(
                    field("old_string").or(field("oldText")).unwrap_or(""),
                    field("new_string").or(field("newText")).unwrap_or(""),
                    cx,
                ));
            }
            "MultiEdit" => {
                for edit in input["edits"].as_array().into_iter().flatten().take(10) {
                    body = body.child(edit_preview(
                        edit["old_string"].as_str().unwrap_or(""),
                        edit["new_string"].as_str().unwrap_or(""),
                        cx,
                    ));
                }
            }
            "Write" | "write" => {
                body = body.child(edit_preview(
                    "",
                    &truncate(field("content").unwrap_or(""), 4000),
                    cx,
                ));
            }
            "Bash" | "bash" => {
                body = body.child(mono_block(
                    format!("$ {}", field("command").unwrap_or("")),
                    cx,
                ));
            }
            _ => {
                let input_text = serde_json::to_string_pretty(input).unwrap_or_default();
                body = body.child(mono_block(truncate(&input_text, MAX_RESULT_CHARS), cx));
            }
        }
        let output = result
            .map(|(content, is_error)| (content.to_string(), is_error))
            .or_else(|| progress.map(|p| (p.clone(), false)));
        if let Some((content, is_error)) = output.filter(|(c, _)| !c.is_empty()) {
            body = body.child(
                mono_block(truncate(&content, MAX_RESULT_CHARS), cx)
                    .when(is_error, |this| this.text_color(cx.theme().danger)),
            );
        }
        body.into_any_element()
    });
    // Live output tail while running and collapsed.
    let live_tail = (!open && result.is_none())
        .then(|| progress.and_then(|p| p.lines().rev().find(|l| !l.trim().is_empty())))
        .flatten()
        .map(|line| line.chars().take(160).collect::<String>());

    v_flex()
        .w_full()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .id(SharedString::from(format!("tool-{id}")))
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .cursor_pointer()
                .hover(|this| this.bg(cx.theme().muted))
                .text_sm()
                .child(
                    Icon::new(tool_icon(name))
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(name.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tool_summary(name, input)),
                )
                .child(status_icon(result, session.running, cx))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_expanded(id, cx))),
        )
        .when_some(live_tail, |this, tail| {
            this.child(
                div()
                    .px_2()
                    .pb_1()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(tail),
            )
        })
        .children(details)
        .into_any_element()
}

fn todo_card(id: ItemId, input: &Value, latest: bool, cx: &Context<ThreadView>) -> AnyElement {
    let todos: Vec<(String, String)> = input["todos"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let status = t["status"].as_str().unwrap_or("pending").to_string();
            let text = if status == "in_progress" {
                t["activeForm"].as_str().or(t["content"].as_str())
            } else {
                t["content"].as_str()
            }
            .unwrap_or("")
            .to_string();
            (status, text)
        })
        .collect();
    let done = todos.iter().filter(|(s, _)| s == "completed").count();
    let header = h_flex()
        .id(SharedString::from(format!("todo-{id}")))
        .gap_2()
        .text_sm()
        .child(
            Icon::new(IconName::ListTodo)
                .small()
                .text_color(cx.theme().muted_foreground),
        )
        .child(div().font_weight(FontWeight::MEDIUM).child("Tasks"))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{done}/{} done", todos.len())),
        );
    if !latest {
        return header.into_any_element();
    }
    let rows = todos.into_iter().map(|(status, text)| {
        let (icon, color, strike) = match status.as_str() {
            "completed" => (IconName::CircleCheck, cx.theme().success, true),
            "in_progress" => (IconName::LoaderCircle, cx.theme().info, false),
            _ => (IconName::Circle, cx.theme().muted_foreground, false),
        };
        h_flex()
            .gap_2()
            .text_sm()
            .child(Icon::new(icon).xsmall().text_color(color))
            .child(
                div()
                    .when(strike, |this| {
                        this.line_through().text_color(cx.theme().muted_foreground)
                    })
                    .when(status == "in_progress", |this| {
                        this.font_weight(FontWeight::MEDIUM)
                    })
                    .child(text),
            )
    });
    v_flex()
        .w_full()
        .p_3()
        .gap_1p5()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .child(header)
        .children(rows)
        .into_any_element()
}

fn subagent_card(
    item: &TranscriptItem,
    tool_use_id: &str,
    input: &Value,
    session: &ThreadSession,
    lookup: &Lookup,
    expanded: &HashSet<ItemId>,
    cx: &Context<ThreadView>,
) -> AnyElement {
    let id = item.id;
    let open = expanded.contains(&id);
    let result = lookup.result(tool_use_id);
    let children = lookup.children(tool_use_id);
    let tool_calls = children
        .iter()
        .filter(|c| matches!(c.content, ItemContent::ToolUse { .. }))
        .count();
    let kind = input["subagent_type"]
        .as_str()
        .unwrap_or("agent")
        .to_string();
    let latest = children.iter().rev().find_map(|c| match &c.content {
        ItemContent::ToolUse { name, input, .. } => {
            Some(format!("{name} {}", tool_summary(name, input)))
        }
        _ => None,
    });
    let body = open.then(|| {
        let mut body = v_flex()
            .p_2()
            .gap_1()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(mono_block(
                truncate(input["prompt"].as_str().unwrap_or(""), 2000),
                cx,
            ));
        for child in &children {
            match &child.content {
                ItemContent::ToolUse {
                    tool_use_id,
                    name,
                    input,
                    ..
                } => {
                    body = body.child(tool_row(
                        child.id,
                        tool_use_id,
                        name,
                        input,
                        session,
                        lookup,
                        expanded,
                        cx,
                    ));
                }
                ItemContent::Assistant { text, .. } => {
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(truncate(text, 1200)),
                    );
                }
                _ => {}
            }
        }
        if let Some((content, _)) = result {
            body = body.child(
                TextView::markdown(
                    SharedString::from(format!("agent-result-{id}")),
                    truncate(content, MAX_RESULT_CHARS),
                )
                .selectable(true),
            );
        }
        body
    });
    let summary = match (&result, latest) {
        (None, Some(latest)) => latest,
        _ => input["description"].as_str().unwrap_or("").to_string(),
    };
    v_flex()
        .w_full()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .id(SharedString::from(format!("agent-{id}")))
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .cursor_pointer()
                .hover(|this| this.bg(cx.theme().muted))
                .text_sm()
                .child(
                    Icon::new(IconName::Bot)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!("Subagent · {kind}")),
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
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{tool_calls} tools")),
                )
                .child(status_icon(result, session.running, cx))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_expanded(id, cx))),
        )
        .children(body)
        .into_any_element()
}

fn decided_line(label: &str, color: Hsla, tool_name: &str, input: &Value, cx: &App) -> AnyElement {
    h_flex()
        .gap_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::ShieldCheck).xsmall().text_color(color))
        .child(format!("{label}: {tool_name}"))
        .child(
            div()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(tool_summary(tool_name, input)),
        )
        .into_any_element()
}

fn attention_card(cx: &App) -> Div {
    v_flex()
        .w_full()
        .p_3()
        .gap_2()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().warning)
        .bg(cx.theme().warning.opacity(0.08))
}

fn approval_card(
    id: ItemId,
    request_id: &str,
    tool_name: &str,
    description: &Option<String>,
    input: &Value,
    decision: Option<ApprovalDecision>,
    cx: &Context<ThreadView>,
) -> AnyElement {
    if let Some(decision) = decision {
        let (label, color) = match decision {
            ApprovalDecision::Allowed => ("Allowed", cx.theme().success),
            ApprovalDecision::AllowedForSession => ("Allowed for session", cx.theme().success),
            ApprovalDecision::Denied => ("Denied", cx.theme().danger),
        };
        return decided_line(label, color, tool_name, input, cx);
    }
    let summary = tool_summary(tool_name, input);
    let (allow, session_allow, deny) = (
        request_id.to_string(),
        request_id.to_string(),
        request_id.to_string(),
    );
    attention_card(cx)
        .child(
            h_flex()
                .gap_2()
                .child(
                    Icon::new(IconName::ShieldAlert)
                        .small()
                        .text_color(cx.theme().warning),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("{tool_name} wants permission")),
                ),
        )
        .when_some(description.clone(), |this, d| {
            this.child(div().text_sm().child(d))
        })
        .when(!summary.is_empty(), |this| {
            this.child(mono_block(summary, cx))
        })
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("allow-{id}")))
                        .primary()
                        .small()
                        .label("Allow")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.respond(&allow, ApprovalDecision::Allowed, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("allow-session-{id}")))
                        .small()
                        .label("Allow for session")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.respond(&session_allow, ApprovalDecision::AllowedForSession, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("deny-{id}")))
                        .ghost()
                        .small()
                        .label("Deny")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.respond(&deny, ApprovalDecision::Denied, cx)
                        })),
                ),
        )
        .into_any_element()
}

fn plan_card(
    id: ItemId,
    request_id: &str,
    input: &Value,
    decision: Option<ApprovalDecision>,
    cx: &Context<ThreadView>,
) -> AnyElement {
    let plan = input["plan"].as_str().unwrap_or("").to_string();
    let pending = decision.is_none();
    let title = match decision {
        None => "Plan ready for review",
        Some(ApprovalDecision::Denied) => "Plan sent back for changes",
        Some(_) => "Plan approved",
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
                    Icon::new(IconName::NotebookPen)
                        .small()
                        .text_color(cx.theme().info),
                )
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title)),
        )
        .when(!plan.is_empty(), |this| {
            this.child(
                TextView::markdown(SharedString::from(format!("plan-{id}")), plan.clone())
                    .selectable(true),
            )
        });
    if pending {
        let (auto, ask, keep) = (
            request_id.to_string(),
            request_id.to_string(),
            request_id.to_string(),
        );
        card = card.child(
            h_flex()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("plan-auto-{id}")))
                        .primary()
                        .small()
                        .label("Approve, accept edits")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.approve_plan(&auto, Some(PermissionMode::AcceptEdits), cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("plan-ask-{id}")))
                        .small()
                        .label("Approve, ask for edits")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.approve_plan(&ask, Some(PermissionMode::Ask), cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("plan-keep-{id}")))
                        .ghost()
                        .small()
                        .label("Keep planning")
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.approve_plan(&keep, None, cx)),
                        ),
                ),
        );
    }
    card.into_any_element()
}

fn answer_summary(request: &QuestionRequest, answer: &QuestionAnswer) -> String {
    match answer {
        QuestionAnswer::Answers { answers } => request
            .questions
            .iter()
            .zip(answers)
            .map(|(q, a)| {
                let label = if q.header.is_empty() {
                    q.question.as_str()
                } else {
                    q.header.as_str()
                };
                if label.is_empty() {
                    a.clone()
                } else {
                    format!("{label}: {a}")
                }
            })
            .collect::<Vec<_>>()
            .join(" · "),
        QuestionAnswer::Confirmed { confirmed } => {
            format!(
                "{}: {}",
                request.title,
                if *confirmed { "Yes" } else { "No" }
            )
        }
        QuestionAnswer::Cancelled => "Question dismissed".to_string(),
    }
}

fn question_card(
    id: ItemId,
    request: &QuestionRequest,
    answer: Option<&QuestionAnswer>,
    form: Option<&QuestionForm>,
    cx: &Context<ThreadView>,
) -> AnyElement {
    if let Some(answer) = answer {
        return h_flex()
            .gap_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(IconName::MessageCircleQuestionMark).xsmall())
            .child(div().flex_1().child(answer_summary(request, answer)))
            .into_any_element();
    }
    let request_id = request.request_id.clone();
    let title = if request.title.is_empty() {
        "The agent has a question".to_string()
    } else {
        request.title.clone()
    };
    let mut card = attention_card(cx).child(
        h_flex()
            .gap_2()
            .child(
                Icon::new(IconName::MessageCircleQuestionMark)
                    .small()
                    .text_color(cx.theme().warning),
            )
            .child(div().font_weight(FontWeight::SEMIBOLD).child(title)),
    );
    if request.kind == QuestionKind::Confirm {
        let header = request
            .questions
            .first()
            .map(|q| q.header.clone())
            .unwrap_or_default();
        let (yes, no) = (request_id.clone(), request_id.clone());
        return card
            .when(!header.is_empty(), |this| {
                this.child(div().text_sm().child(header))
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("q-yes-{id}")))
                            .primary()
                            .small()
                            .label("Yes")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.answer(&yes, QuestionAnswer::Confirmed { confirmed: true }, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("q-no-{id}")))
                            .small()
                            .label("No")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.answer(&no, QuestionAnswer::Confirmed { confirmed: false }, cx)
                            })),
                    ),
            )
            .into_any_element();
    }
    for (qi, question) in request.questions.iter().enumerate() {
        let mut block = v_flex().gap_1();
        let show_question = request.kind == QuestionKind::Choice && !question.question.is_empty();
        if show_question || !question.header.is_empty() {
            block = block.child(
                h_flex()
                    .gap_2()
                    .when(!question.header.is_empty(), |this| {
                        this.child(
                            div()
                                .px_1p5()
                                .rounded_sm()
                                .bg(cx.theme().muted)
                                .text_xs()
                                .child(question.header.clone()),
                        )
                    })
                    .when(show_question, |this| {
                        this.child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(question.question.clone()),
                        )
                    }),
            );
        }
        for (oi, option) in question.options.iter().enumerate() {
            let selected =
                form.is_some_and(|f| f.selected.get(qi).is_some_and(|s| s.contains(&oi)));
            let rid = request_id.clone();
            let multi = question.multi_select;
            let icon = match (multi, selected) {
                (true, true) => IconName::SquareCheck,
                (true, false) => IconName::Square,
                (false, true) => IconName::CircleDot,
                (false, false) => IconName::Circle,
            };
            block = block.child(
                h_flex()
                    .id(SharedString::from(format!("q-{id}-{qi}-{oi}")))
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(if selected {
                        cx.theme().ring
                    } else {
                        cx.theme().border
                    })
                    .when(selected, |this| this.bg(cx.theme().accent))
                    .cursor_pointer()
                    .child(Icon::new(icon).xsmall())
                    .child(
                        v_flex()
                            .child(div().text_sm().child(option.label.clone()))
                            .when(!option.description.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(option.description.clone()),
                                )
                            }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_option(&rid, qi, oi, multi, cx)
                    })),
            );
        }
        if let Some(input) = form.and_then(|f| f.texts.get(qi)) {
            block = block.child(Input::new(input).small());
        }
        card = card.child(block);
    }
    let (submit, dismiss) = (request_id.clone(), request_id);
    card.child(
        h_flex()
            .gap_2()
            .child(
                Button::new(SharedString::from(format!("q-submit-{id}")))
                    .primary()
                    .small()
                    .label("Submit")
                    .on_click(cx.listener(move |this, _, _, cx| this.submit_question(&submit, cx))),
            )
            .child(
                Button::new(SharedString::from(format!("q-dismiss-{id}")))
                    .ghost()
                    .small()
                    .label("Dismiss")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.answer(&dismiss, QuestionAnswer::Cancelled, cx)
                    })),
            ),
    )
    .into_any_element()
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
