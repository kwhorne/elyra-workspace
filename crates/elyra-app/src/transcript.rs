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
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
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

struct Lookup<'a> {
    results: HashMap<&'a str, (&'a str, bool)>,
    children: HashMap<&'a str, Vec<&'a TranscriptItem>>,
    latest_todo: Option<ItemId>,
}

impl<'a> Lookup<'a> {
    fn new(items: &'a [TranscriptItem]) -> Self {
        let mut results = HashMap::new();
        let mut children: HashMap<&str, Vec<&TranscriptItem>> = HashMap::new();
        let mut latest_todo = None;
        for item in items {
            match &item.content {
                ItemContent::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    results.insert(tool_use_id.as_str(), (content.as_str(), *is_error));
                }
                ItemContent::ToolUse {
                    name,
                    parent_tool_use_id,
                    ..
                } => {
                    if name == "TodoWrite" {
                        latest_todo = Some(item.id);
                    }
                    if let Some(parent) = parent_tool_use_id {
                        children.entry(parent.as_str()).or_default().push(item);
                    }
                }
                ItemContent::Assistant {
                    parent_tool_use_id: Some(parent),
                    ..
                } => children.entry(parent.as_str()).or_default().push(item),
                _ => {}
            }
        }
        Self {
            results,
            children,
            latest_todo,
        }
    }
}

/// Returns the transcript rows, each tagged with the item it renders (used
/// by find-in-thread to scroll to a match).
pub fn render(
    session: &ThreadSession,
    expanded: &HashSet<ItemId>,
    forms: &HashMap<String, QuestionForm>,
    highlight: Option<ItemId>,
    cx: &Context<ThreadView>,
) -> Vec<(Option<ItemId>, AnyElement)> {
    let lookup = Lookup::new(&session.items);
    let mut rows = Vec::new();
    for item in &session.items {
        let is_child = matches!(
            &item.content,
            ItemContent::ToolUse {
                parent_tool_use_id: Some(_),
                ..
            } | ItemContent::Assistant {
                parent_tool_use_id: Some(_),
                ..
            }
        );
        if is_child {
            continue;
        }
        if let Some(element) = render_item(item, session, &lookup, expanded, forms, cx) {
            let element = if highlight == Some(item.id) {
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().ring)
                    .p_1()
                    .child(element)
                    .into_any_element()
            } else {
                element
            };
            rows.push((Some(item.id), element));
        }
    }

    if !session.streaming_thinking.is_empty() && session.streaming_text.is_empty() {
        let tail: String = {
            let chars: Vec<char> = session.streaming_thinking.chars().collect();
            chars[chars.len().saturating_sub(400)..].iter().collect()
        };
        rows.push((
            None,
            div()
                .text_sm()
                .italic()
                .text_color(cx.theme().muted_foreground)
                .child(tail)
                .into_any_element(),
        ));
    }
    if !session.streaming_text.is_empty() {
        rows.push((
            None,
            TextView::markdown("md-streaming", session.streaming_text.clone())
                .selectable(true)
                .into_any_element(),
        ));
    }
    if let Some(preparing) = &session.preparing {
        rows.push((None, working_row(preparing.clone(), cx)));
    } else if let Some(activity) = &session.activity {
        rows.push((None, working_row(activity.clone(), cx)));
    } else if session.running && session.streaming_text.is_empty() && !session.pending_approval() {
        rows.push((None, working_row("Working…".into(), cx)));
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
        ItemContent::User { text, checkpoint } => {
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
                return Some(todo_card(id, input, lookup.latest_todo == Some(id), cx));
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
            .child(div().flex_1().child(text.clone()))
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
    let result = lookup.results.get(tool_use_id).copied();
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
    let result = lookup.results.get(tool_use_id).copied();
    let children = lookup
        .children
        .get(tool_use_id)
        .cloned()
        .unwrap_or_default();
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
    use super::tool_summary;
    use serde_json::json;

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
