//! Tool calls with their results, the task list and subagents.

use super::*;

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
pub(super) fn tool_row(
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

pub(super) fn todo_card(
    id: ItemId,
    input: &Value,
    latest: bool,
    cx: &Context<ThreadView>,
) -> AnyElement {
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

pub(super) fn subagent_card(
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
