//! Asking before one agent changes another thread through the gateway.
//!
//! Reading other threads is open. The first time an agent's thread sends a
//! message to, interrupts, renames or archives another thread, or opens,
//! reloads or clicks and types on a page in its browser, the user is asked; "Always" remembers that pair (in the settings table) so it isn't
//! asked again. Threads an agent starts with `create_thread` are remembered
//! for it right away. Paired external clients are not asked: pairing them was
//! the permission.

use crate::app_state::AppState;
use crate::browser_tools::BrowserHub;
use elyra_core::ThreadId;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use std::collections::BTreeSet;

/// Gateway tools that change the thread they name.
pub const WRITES: &[&str] = &[
    "send_message",
    "interrupt_thread",
    "set_thread_title",
    "archive_thread",
    "browser_open",
    "browser_reload",
    "browser_click",
    "browser_fill",
    "browser_press",
    "browser_save_journey",
    "browser_run_journey",
];

const KEY: &str = "gateway_thread_pairs";

/// What the user answered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Answer {
    Deny,
    Once,
    Always,
}

fn pair_key(from: ThreadId, to: ThreadId) -> String {
    format!("{from}>{to}")
}

/// Thread pairs (`from>to`) allowed for good.
pub fn approved(app: &AppState) -> BTreeSet<String> {
    app.store
        .setting(KEY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn is_approved(app: &AppState, from: ThreadId, to: ThreadId) -> bool {
    approved(app).contains(&pair_key(from, to))
}

pub fn remember(app: &AppState, from: ThreadId, to: ThreadId) {
    let mut pairs = approved(app);
    pairs.insert(pair_key(from, to));
    save(app, &pairs);
}

/// Forget every remembered pair (Settings → Agents & MCP).
pub fn forget_all(app: &AppState) {
    save(app, &BTreeSet::new());
}

fn save(app: &AppState, pairs: &BTreeSet<String>) {
    if let Ok(json) = serde_json::to_string(pairs)
        && let Err(err) = app.store.set_setting(KEY, &json)
    {
        log::warn!("saving thread pairs: {err:#}");
    }
}

/// What the request would do, for the question.
fn describe(tool: &str, args: &serde_json::Value) -> String {
    match tool {
        "send_message" => {
            let text: String = args["text"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(400)
                .collect();
            format!("send it this message:\n\n{text}")
        }
        "interrupt_thread" => "stop its running turn".into(),
        "set_thread_title" => format!(
            "rename it to \u{201c}{}\u{201d}",
            args["title"].as_str().unwrap_or("")
        ),
        "archive_thread" => "archive it".into(),
        "browser_open" => format!(
            "open {} in its browser",
            args["url"].as_str().unwrap_or("a page")
        ),
        "browser_reload" => "reload the page in its browser".into(),
        "browser_save_journey" => format!(
            "save a journey \u{201c}{}\u{201d} in its project",
            args["name"].as_str().unwrap_or("")
        ),
        "browser_run_journey" => "replay a journey in its browser".into(),
        "browser_click" | "browser_fill" | "browser_press" => format!(
            "{} in its browser",
            crate::browser_tools::describe_action(tool, args)
        ),
        other => format!("use {other} on it"),
    }
}

/// Ask in the workspace window; the answer arrives on `answer`. Returns
/// false when there is no window to ask in.
pub fn ask(
    app: &Entity<AppState>,
    from: ThreadId,
    to: ThreadId,
    tool: &str,
    args: &serde_json::Value,
    answer: async_channel::Sender<Answer>,
    cx: &mut App,
) -> bool {
    let Some(window) = cx.try_global::<BrowserHub>().map(|hub| hub.window) else {
        return false;
    };
    let title = |id: ThreadId| {
        app.read(cx)
            .thread(id)
            .map(|t| t.title.clone())
            .unwrap_or_else(|| "a thread".into())
    };
    let (from_title, to_title) = (title(from), title(to));
    let action = describe(tool, args);
    window
        .update(cx, move |_, window, cx| {
            window.open_dialog(cx, move |dialog, _, cx| {
                let reply = |value: Answer| {
                    let answer = answer.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        answer.try_send(value).ok();
                        window.close_dialog(cx);
                    }
                };
                dialog
                    .title("Let one agent change another thread?")
                    .w(px(520.))
                    .close_button(false)
                    .overlay_closable(false)
                    .keyboard(false)
                    .child(
                        v_flex()
                            .gap_2()
                            .text_sm()
                            .child(format!(
                                "The agent in \u{201c}{from_title}\u{201d} wants to change \u{201c}{to_title}\u{201d}:"
                            ))
                            .child(
                                div()
                                    .p_2()
                                    .rounded_md()
                                    .bg(cx.theme().muted)
                                    .text_xs()
                                    .whitespace_normal()
                                    .child(action.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Always allows this agent to change that thread from now on. Settings → Agents & MCP forgets remembered pairs."),
                            ),
                    )
                    .footer(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("pair-deny")
                                    .small()
                                    .label("Don't allow")
                                    .on_click(reply(Answer::Deny)),
                            )
                            .child(
                                Button::new("pair-once")
                                    .small()
                                    .label("Allow once")
                                    .on_click(reply(Answer::Once)),
                            )
                            .child(
                                Button::new("pair-always")
                                    .small()
                                    .primary()
                                    .label("Always allow")
                                    .on_click(reply(Answer::Always)),
                            ),
                    )
            });
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::describe;
    use serde_json::json;

    #[test]
    fn describes_what_the_agent_wants() {
        assert_eq!(
            describe("send_message", &json!({ "text": "run the tests" })),
            "send it this message:\n\nrun the tests"
        );
        assert_eq!(
            describe("interrupt_thread", &json!({})),
            "stop its running turn"
        );
        assert_eq!(
            describe("set_thread_title", &json!({ "title": "Done" })),
            "rename it to \u{201c}Done\u{201d}"
        );
        assert_eq!(
            describe("browser_open", &json!({ "url": "http://shop.test" })),
            "open http://shop.test in its browser"
        );
    }
}
