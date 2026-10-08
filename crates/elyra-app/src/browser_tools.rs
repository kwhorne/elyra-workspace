//! Agent gateway tools for the in-app browser: an agent can open the app it
//! is building in its thread's browser, look at the page — its DOM, elements
//! and their styles, the console, network calls and a screenshot — and use
//! it: click, fill in fields, press keys, wait for something to appear.
//!
//! Adapted from Litr's agent tools (© Wirelabs AS), used under the MIT
//! licence with its owner's permission. As there, agents only ever see pages
//! served from this Mac (localhost, 127.0.0.1, *.test, *.local). Before an
//! agent first clicks or types in its thread's browser the user is asked
//! (not in Full access); "Take over" in the browser withdraws it.

use crate::app_state::AppState;
use crate::browser_view::BrowserView;
use crate::webview;
use crate::workspace::Workspace;
use base64::Engine as _;
use elyra_core::ThreadId;
use elyra_mcp::{Output, Tool};
use gpui_kit::{AnyWindowHandle, App, AsyncApp, Entity, Global, WeakEntity};
use serde_json::{Value, json};
use std::time::Duration;

/// How long to wait for a page to answer.
const PAGE_TIMEOUT: Duration = Duration::from_secs(15);

/// Where agent tools find the browsers: the workspace and its window.
pub struct BrowserHub {
    pub workspace: WeakEntity<Workspace>,
    pub window: AnyWindowHandle,
}

impl Global for BrowserHub {}

/// Tools that only read (offered to read-only clients too).
pub const READ_TOOLS: &[&str] = &[
    "browser_snapshot",
    "browser_query",
    "browser_console",
    "browser_network",
    "browser_screenshot",
    "browser_wait",
];

/// Tools that act on the page like a user.
pub const ACT_TOOLS: &[&str] = &["browser_click", "browser_fill", "browser_press"];

/// Longest `browser_wait` may wait.
const MAX_WAIT: Duration = Duration::from_secs(60);

pub fn tools() -> Vec<Tool> {
    let tool = |name: &str, description: &str, schema: Value| Tool {
        name: name.into(),
        description: description.into(),
        input_schema: schema,
    };
    let thread =
        json!({ "type": "string", "description": "Thread id; defaults to your own thread" });
    let limit = |default: u32| json!({ "type": "integer", "description": format!("Most entries to return (default {default})") });
    vec![
        tool(
            "browser_open",
            "Open a local development page (localhost, 127.0.0.1, *.test, *.local) in the thread's browser in Elyra Workspace, for example the app you are building.",
            json!({ "type": "object", "properties": { "url": { "type": "string" }, "thread_id": thread }, "required": ["url"] }),
        ),
        tool(
            "browser_snapshot",
            "The page in the thread's browser as an outline of its DOM: elements with id, classes and key attributes, and their text. Scripts and styles are left out; long pages are cut.",
            json!({ "type": "object", "properties": { "thread_id": thread } }),
        ),
        tool(
            "browser_query",
            "The elements matching a CSS selector on the page: their HTML (shortened), text, position and size, and the computed styles that usually explain layout bugs.",
            json!({ "type": "object", "properties": { "selector": { "type": "string" }, "limit": limit(5), "thread_id": thread }, "required": ["selector"] }),
        ),
        tool(
            "browser_console",
            "What the page wrote to the console (log, info, warn, error, debug) since it loaded, newest last.",
            json!({ "type": "object", "properties": { "limit": limit(100), "thread_id": thread } }),
        ),
        tool(
            "browser_network",
            "The page's fetch and XMLHttpRequest calls since it loaded (method, address, status, time, start of text or JSON bodies), and every other file it loaded with timing and size.",
            json!({ "type": "object", "properties": { "limit": limit(100), "thread_id": thread } }),
        ),
        tool(
            "browser_screenshot",
            "A picture of the page as it looks now. The Browser tab has to be on screen.",
            json!({ "type": "object", "properties": { "thread_id": thread } }),
        ),
        tool(
            "browser_click",
            "Click an element on the page, like the user would: by CSS selector, by its visible text, or both (the first visible match). Returns what was clicked; look again (browser_snapshot, browser_wait) to see the result. The user is asked the first time.",
            json!({ "type": "object", "properties": {
                "selector": { "type": "string", "description": "CSS selector" },
                "text": { "type": "string", "description": "Visible text the element contains, e.g. a button label" },
                "thread_id": thread
            } }),
        ),
        tool(
            "browser_fill",
            "Fill in a field (input, textarea, select, checkbox, radio, contenteditable) so the page's framework sees it as typed. Find it by CSS selector, or by text: its label, placeholder or current text. For a select, value is an option's value or label; for a checkbox, \"true\" or \"false\".",
            json!({ "type": "object", "properties": {
                "selector": { "type": "string" },
                "text": { "type": "string", "description": "Label, placeholder or text of the field" },
                "value": { "type": "string" },
                "thread_id": thread
            }, "required": ["value"] }),
        ),
        tool(
            "browser_press",
            "Press a key (Enter, Tab, Escape, ArrowDown, a letter…) in the focused element, or in the element given by selector or text. Enter in a form field submits the form.",
            json!({ "type": "object", "properties": {
                "key": { "type": "string" },
                "selector": { "type": "string" },
                "text": { "type": "string" },
                "thread_id": thread
            }, "required": ["key"] }),
        ),
        tool(
            "browser_wait",
            "Wait until an element (CSS selector) or a text appears on the page, e.g. after a click or a form submit.",
            json!({ "type": "object", "properties": {
                "selector": { "type": "string" },
                "text": { "type": "string" },
                "timeout_seconds": { "type": "integer", "description": "Default 10, max 60" },
                "thread_id": thread
            } }),
        ),
        tool(
            "browser_reload",
            "Reload the page, for instance after changing its code. Console and network logs start over.",
            json!({ "type": "object", "properties": { "thread_id": thread } }),
        ),
    ]
}

pub fn is_browser_tool(name: &str) -> bool {
    name.starts_with("browser_")
}

fn thread_for(caller: Option<ThreadId>, args: &Value) -> Result<ThreadId, String> {
    match args["thread_id"].as_str() {
        Some(id) => id
            .parse()
            .map_err(|_| "`thread_id` is not a thread id".to_string()),
        None => {
            caller.ok_or_else(|| "Give `thread_id`: which thread's browser to use.".to_string())
        }
    }
}

fn browser_of(thread: ThreadId, cx: &App) -> Option<Entity<BrowserView>> {
    let hub = cx.try_global::<BrowserHub>()?;
    hub.workspace
        .upgrade()?
        .read(cx)
        .browser_views
        .get(&thread)
        .cloned()
}

/// Run a browser tool for `caller` (the agent's own thread, if it is one).
pub async fn run(
    app: &Entity<AppState>,
    caller: Option<ThreadId>,
    tool: &str,
    args: &Value,
    cx: &mut AsyncApp,
) -> Result<Output, String> {
    let thread = thread_for(caller, args)?;
    let exists = cx.update(|cx| app.read(cx).thread(thread).is_some());
    if !exists {
        return Err(format!("No thread {thread}."));
    }
    if tool == "browser_open" {
        let url = args["url"].as_str().unwrap_or("").trim().to_string();
        let url = crate::browser_view::normalize_address(&url).unwrap_or(url);
        if !webview::is_local_address(&url) {
            return Err(format!(
                "{url} isn't a local development page. Agents can open pages served from this Mac only (localhost, 127.0.0.1, *.test, *.local)."
            ));
        }
        let opened = cx.update(|cx| {
            let hub = cx.try_global::<BrowserHub>()?;
            let (workspace, window) = (hub.workspace.clone(), hub.window);
            window
                .update(cx, |_, window, cx| {
                    workspace
                        .update(cx, |workspace, cx| {
                            workspace.open_in_browser(thread, &url, window, cx)
                        })
                        .ok()
                        .flatten()
                })
                .ok()
                .flatten()
        });
        return match opened {
            Some(_) => Ok(format!("Opening {url} in the thread's browser.").into()),
            None => Err("Elyra Workspace has no window to show the browser in.".into()),
        };
    }

    let Some(browser) = cx.update(|cx| browser_of(thread, cx)) else {
        return Err("The thread's browser has no page. Open one with browser_open.".into());
    };
    let (page, state, on_screen) = cx.update(|cx| {
        let browser = browser.read(cx);
        (
            browser.web_view(),
            browser.page_state().clone(),
            browser.is_on_screen(),
        )
    });
    let Some(page) = page else {
        return Err("The thread's browser has no page. Open one with browser_open.".into());
    };
    if !webview::is_local_address(&state.url) {
        return Err(format!(
            "The thread's browser shows {}, not a local development page; agents can only look at pages served from this Mac.",
            if state.url.is_empty() {
                "nothing yet"
            } else {
                &state.url
            }
        ));
    }
    let label = format!(
        "{} ({})",
        if state.title.is_empty() {
            "Untitled"
        } else {
            &state.title
        },
        state.url
    );
    if ACT_TOOLS.contains(&tool) && caller == Some(thread) {
        allowed_to_act(app, &browser, thread, tool, args, &state.url, cx).await?;
    }
    let target = |args: &Value| {
        (
            serde_json::to_string(args["selector"].as_str().unwrap_or("").trim())
                .unwrap_or_default(),
            serde_json::to_string(args["text"].as_str().unwrap_or("").trim()).unwrap_or_default(),
        )
    };
    let limit = |default: usize| {
        args["limit"]
            .as_u64()
            .map(|n| (n as usize).clamp(1, 500))
            .unwrap_or(default)
    };

    // The page answers through WebKit callbacks on the main thread.
    let (tx, rx) = async_channel::bounded::<Option<String>>(1);
    match tool {
        "browser_reload" => {
            page.reload();
            return Ok(format!("Reloading {label}.").into());
        }
        "browser_screenshot" => {
            if !on_screen {
                return Err(format!(
                    "{label} isn't on screen; ask the user to show the Browser tab in Elyra Workspace."
                ));
            }
            let (image_tx, image_rx) = async_channel::bounded::<Option<Vec<u8>>>(1);
            page.agent_snapshot(move |jpeg| {
                image_tx.try_send(jpeg).ok();
            });
            let jpeg = wait(image_rx, cx).await.flatten();
            return match jpeg {
                Some(jpeg) => Ok(Output::Image {
                    base64: base64::engine::general_purpose::STANDARD.encode(jpeg),
                    mime_type: "image/jpeg".into(),
                    caption: label,
                }),
                None => Err(format!("{label} couldn't be captured; is it on screen?")),
            };
        }
        "browser_snapshot" => page.agent_look("snapshot()", move |json| {
            tx.try_send(json).ok();
        }),
        "browser_query" => {
            let selector = args["selector"].as_str().unwrap_or("").trim().to_string();
            if selector.is_empty() {
                return Err("Give a CSS selector.".into());
            }
            let call = format!(
                "query({}, {})",
                serde_json::to_string(&selector).unwrap_or_default(),
                limit(5)
            );
            page.agent_look(&call, move |json| {
                tx.try_send(json).ok();
            });
        }
        "browser_console" | "browser_network" => page.agent_captured(move |json| {
            tx.try_send(json).ok();
        }),
        "browser_click" | "browser_fill" | "browser_press" => {
            let (selector, text) = target(args);
            let call = match tool {
                "browser_click" => {
                    if args["selector"].as_str().unwrap_or("").trim().is_empty()
                        && args["text"].as_str().unwrap_or("").trim().is_empty()
                    {
                        return Err("Give a selector, a text, or both.".into());
                    }
                    format!("click({selector}, {text})")
                }
                "browser_fill" => {
                    let value = args["value"].as_str().unwrap_or("");
                    let value = serde_json::to_string(value).unwrap_or_default();
                    format!("fill({selector}, {text}, {value})")
                }
                _ => {
                    let key = args["key"].as_str().unwrap_or("").trim();
                    if key.is_empty() {
                        return Err("Give a key, e.g. Enter.".into());
                    }
                    let key = serde_json::to_string(key).unwrap_or_default();
                    format!("press({key}, {selector}, {text})")
                }
            };
            page.agent_act(&call, move |json| {
                tx.try_send(json).ok();
            });
        }
        "browser_wait" => {
            let (selector, text) = target(args);
            if selector == "\"\"" && text == "\"\"" {
                return Err("Give a selector or a text to wait for.".into());
            }
            let timeout =
                Duration::from_secs(args["timeout_seconds"].as_u64().unwrap_or(10)).min(MAX_WAIT);
            let started = std::time::Instant::now();
            let call = format!("present({selector}, {text})");
            loop {
                let (found_tx, found_rx) = async_channel::bounded::<Option<String>>(1);
                page.agent_act(&call, move |json| {
                    found_tx.try_send(json).ok();
                });
                let found = wait(found_rx, cx).await.flatten().unwrap_or_default();
                if found.contains("\"present\":true") {
                    return Ok(format!(
                        "{label}\nFound after {:.1} s: {found}",
                        started.elapsed().as_secs_f64()
                    )
                    .into());
                }
                if started.elapsed() >= timeout {
                    return Err(format!(
                        "{label}\nStill not on the page after {} s.",
                        timeout.as_secs()
                    ));
                }
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
            }
        }
        other => return Err(format!("No tool called {other}.")),
    }
    let answer = wait(rx, cx).await.flatten();
    match tool {
        "browser_console" | "browser_network" => {
            let Some(captured) = answer.and_then(|json| serde_json::from_str::<Value>(&json).ok())
            else {
                return Ok(format!(
                    "Nothing captured on {label} yet: the console and network are watched from a page's load on. Reload it (browser_reload) and try again."
                )
                .into());
            };
            if tool == "browser_console" {
                Ok(captured_list(&captured, "console", limit(100), &label).into())
            } else {
                Ok(format!(
                    "{}\n\nFiles the page loaded:\n{}",
                    captured_list(&captured, "network", limit(100), &label),
                    captured.get("resources").cloned().unwrap_or(json!([]))
                )
                .into())
            }
        }
        _ => match answer {
            Some(json) if json.starts_with("{\"error\"") => Err(format!("{label}\n{json}")),
            Some(json) => Ok(format!("{label}\n{json}").into()),
            None => Err(format!("{label} didn't answer; it may still be loading.")),
        },
    }
}

/// Before an agent clicks or types in its own thread's browser: allowed for
/// the thread already, Full access, or the user says so now.
async fn allowed_to_act(
    app: &Entity<AppState>,
    browser: &Entity<BrowserView>,
    thread: ThreadId,
    tool: &str,
    args: &Value,
    url: &str,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    let (granted, full_access, title) = cx.update(|cx| {
        let thread = app.read(cx).thread(thread);
        (
            browser.read(cx).agent_control(),
            thread.is_some_and(|t| t.permission_mode == elyra_core::PermissionMode::FullAccess),
            thread.map(|t| t.title.clone()).unwrap_or_default(),
        )
    });
    if granted || full_access {
        return Ok(());
    }
    let (tx, rx) = async_channel::bounded(1);
    let action = describe_action(tool, args);
    let url = url.to_string();
    let asked = cx.update(|cx| ask_to_act(&title, &action, &url, tx, cx));
    if !asked {
        return Err(
            "Elyra Workspace has no window to ask the user whether the agent may use the browser."
                .into(),
        );
    }
    match rx.recv().await {
        Ok(crate::thread_access::Answer::Always) => {
            cx.update(|cx| browser.update(cx, |browser, cx| browser.set_agent_control(true, cx)));
            Ok(())
        }
        Ok(crate::thread_access::Answer::Once) => Ok(()),
        _ => Err("The user didn't let the agent click or type in the browser. Describe what to try instead, or ask them to do it.".into()),
    }
}

/// What a browser action would do, for the question.
pub fn describe_action(tool: &str, args: &Value) -> String {
    let field = |key: &str| args[key].as_str().map(str::trim).filter(|v| !v.is_empty());
    let target = match (field("text"), field("selector")) {
        (Some(text), _) => format!("\u{201c}{text}\u{201d}"),
        (None, Some(selector)) => format!("`{selector}`"),
        (None, None) => "the focused element".to_string(),
    };
    match tool {
        "browser_click" => format!("click {target}"),
        "browser_fill" => format!(
            "fill in {target} with \u{201c}{}\u{201d}",
            field("value")
                .unwrap_or("")
                .chars()
                .take(80)
                .collect::<String>()
        ),
        "browser_press" => format!("press {} in {target}", field("key").unwrap_or("a key")),
        other => format!("use {other}"),
    }
}

/// Ask in the workspace window; the answer arrives on `answer`.
fn ask_to_act(
    title: &str,
    action: &str,
    url: &str,
    answer: async_channel::Sender<crate::thread_access::Answer>,
    cx: &mut App,
) -> bool {
    use crate::thread_access::Answer;
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
    use gpui_kit::{ClickEvent, ParentElement as _, Styled as _, Window, div, px};

    let Some(window) = cx.try_global::<BrowserHub>().map(|hub| hub.window) else {
        return false;
    };
    let text = format!("The agent in \u{201c}{title}\u{201d} wants to {action} on {url}.");
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
                    .title("Let the agent use the browser?")
                    .w(px(500.))
                    .close_button(false)
                    .overlay_closable(false)
                    .keyboard(false)
                    .child(
                        v_flex()
                            .gap_2()
                            .text_sm()
                            .child(div().whitespace_normal().child(text.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Allow for this thread lets it click and type on local pages in this thread's browser until you press Take over there. In Full access it isn't asked."),
                            ),
                    )
                    .footer(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("act-deny")
                                    .small()
                                    .label("Don't allow")
                                    .on_click(reply(Answer::Deny)),
                            )
                            .child(
                                Button::new("act-once")
                                    .small()
                                    .label("Allow once")
                                    .on_click(reply(Answer::Once)),
                            )
                            .child(
                                Button::new("act-thread")
                                    .small()
                                    .primary()
                                    .label("Allow for this thread")
                                    .on_click(reply(Answer::Always)),
                            ),
                    )
            });
        })
        .is_ok()
}

async fn wait<T>(rx: async_channel::Receiver<T>, cx: &mut AsyncApp) -> Option<T> {
    let timeout = cx.background_executor().timer(PAGE_TIMEOUT);
    futures_or(rx.recv(), timeout).await
}

/// The first of `value` and `timeout` to finish.
async fn futures_or<T>(
    value: impl std::future::Future<Output = Result<T, async_channel::RecvError>>,
    timeout: impl std::future::Future<Output = ()>,
) -> Option<T> {
    use std::pin::pin;
    use std::task::Poll;
    let mut value = pin!(value);
    let mut timeout = pin!(timeout);
    std::future::poll_fn(move |task| {
        if let Poll::Ready(result) = value.as_mut().poll(task) {
            return Poll::Ready(result.ok());
        }
        if timeout.as_mut().poll(task).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await
}

/// The newest `limit` entries of the captured `key` list, as JSON text.
fn captured_list(captured: &Value, key: &str, limit: usize, label: &str) -> String {
    let items = captured[key].as_array().cloned().unwrap_or_default();
    let newest = &items[items.len().saturating_sub(limit)..];
    format!(
        "{label}: {} of {} {key} entries since the page loaded, newest last\n{}",
        newest.len(),
        items.len(),
        Value::Array(newest.to_vec())
    )
}

#[cfg(test)]
mod tests {
    use super::captured_list;
    use serde_json::json;

    #[test]
    fn keeps_the_newest_entries() {
        let captured = json!({ "console": [{"n": 1}, {"n": 2}, {"n": 3}] });
        let text = captured_list(&captured, "console", 2, "Page");
        assert!(text.starts_with("Page: 2 of 3 console entries"));
        assert!(text.ends_with(r#"[{"n":2},{"n":3}]"#));
    }
}
