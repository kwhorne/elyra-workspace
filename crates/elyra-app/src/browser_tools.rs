//! Agent gateway tools for the in-app browser: an agent can open the app it
//! is building in its thread's browser and look at the page — its DOM,
//! elements and their styles, the console, network calls and a screenshot.
//!
//! Adapted from Litr's agent tools (© Wirelabs AS), used under the MIT
//! licence with its owner's permission. As there, agents only ever see pages
//! served from this Mac (localhost, 127.0.0.1, *.test, *.local), and the
//! tools only read, apart from opening and reloading a page.

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
];

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
