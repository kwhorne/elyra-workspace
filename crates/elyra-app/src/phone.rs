//! Answering from the phone, through ntfy (https://ntfy.sh, or your own
//! server). While Elyra Workspace isn't the active app, a thread that needs
//! you (an approval, a question) or finishes sends a push to a private topic;
//! the ntfy app shows it with buttons — Allow, Deny, an answer's options —
//! that post back to `<topic>-reply`, which Elyra listens to. Anything else
//! posted there answers the last question, or goes to the last thread as a
//! message. Both ways go through the ntfy server, so the Mac needs no
//! incoming connection. Off until turned on in Settings.

use crate::app_state::AppState;
use elyra_core::{ApprovalDecision, ItemContent, QuestionAnswer, QuestionKind, ThreadId};
use gpui_kit::{App, AsyncApp, Entity, Global};
use serde_json::{Value, json};
use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the settings are looked at (to start or stop listening).
const POLL: Duration = Duration::from_secs(3);

struct Phone {
    /// The server and topic being listened to, with the curl doing it.
    listening: Option<(String, String)>,
    child: Arc<Mutex<Option<Child>>>,
    /// The thread that last sent a push, for replies without a button.
    last_thread: Option<ThreadId>,
}

impl Global for Phone {}

/// What a reply posted to `<topic>-reply` asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Approve {
        thread: ThreadId,
        request: String,
        decision: ApprovalDecision,
    },
    Choose {
        thread: ThreadId,
        request: String,
        choice: String,
    },
    Text(String),
}

/// `allow|always|deny <thread> <request>`, `choose <thread> <request> <label>`,
/// or free text.
pub fn parse_reply(message: &str) -> Option<Reply> {
    let message = message.trim();
    if message.is_empty() {
        return None;
    }
    let mut words = message.splitn(4, ' ');
    let verb = words.next().unwrap_or("");
    let thread = words.next().and_then(|t| t.parse::<ThreadId>().ok());
    let request = words.next().map(str::to_string);
    let rest = words.next().map(str::to_string);
    let decision = match verb {
        "allow" => Some(ApprovalDecision::Allowed),
        "always" => Some(ApprovalDecision::AllowedForSession),
        "deny" => Some(ApprovalDecision::Denied),
        _ => None,
    };
    match (decision, verb, thread, request, rest) {
        (Some(decision), _, Some(thread), Some(request), None) => Some(Reply::Approve {
            thread,
            request,
            decision,
        }),
        (None, "choose", Some(thread), Some(request), Some(choice)) => Some(Reply::Choose {
            thread,
            request,
            choice,
        }),
        _ => Some(Reply::Text(message.to_string())),
    }
}

pub fn init(app: Entity<AppState>, cx: &mut App) {
    cx.set_global(Phone {
        listening: None,
        child: Arc::new(Mutex::new(None)),
        last_thread: None,
    });
    let (tx, rx) = async_channel::unbounded::<String>();
    // Apply replies on the foreground.
    let replies_app = app.clone();
    cx.spawn(async move |cx| {
        while let Ok(message) = rx.recv().await {
            cx.update(|cx| apply(&replies_app, &message, cx));
        }
    })
    .detach();
    // Follow the settings: listen while it's on.
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            cx.update(|cx| sync_listener(&tx, cx));
            cx.background_executor().timer(POLL).await;
        }
    })
    .detach();
}

/// The ntfy server and topic, when sending to the phone is on.
fn target(cx: &App) -> Option<(String, String)> {
    let prefs = crate::preferences::Preferences::global(cx);
    let topic = prefs.phone_topic.trim();
    (prefs.phone_notifications && !topic.is_empty()).then(|| {
        let server = prefs.phone_server.trim().trim_end_matches('/');
        let server = if server.is_empty() {
            "https://ntfy.sh"
        } else {
            server
        };
        (server.to_string(), topic.to_string())
    })
}

fn sync_listener(tx: &async_channel::Sender<String>, cx: &mut App) {
    let wanted = target(cx);
    let phone = cx.global_mut::<Phone>();
    if phone.listening == wanted {
        return;
    }
    if let Some(mut child) = phone.child.lock().ok().and_then(|mut c| c.take()) {
        let _ = child.kill();
    }
    phone.listening = wanted.clone();
    let Some((server, topic)) = wanted else {
        return;
    };
    let slot = phone.child.clone();
    let tx = tx.clone();
    std::thread::spawn(move || listen(&server, &topic, &slot, &tx));
}

/// Stream `<topic>-reply` and hand each message over; reconnect when the
/// stream ends, until the listener is replaced.
fn listen(
    server: &str,
    topic: &str,
    slot: &Arc<Mutex<Option<Child>>>,
    tx: &async_channel::Sender<String>,
) {
    let url = format!("{server}/{topic}-reply/json");
    loop {
        let child = Command::new("curl")
            .args(["-sN", "--max-time", "0", &url])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            return;
        };
        let stdout = child.stdout.take();
        let id = child.id();
        if let Ok(mut current) = slot.lock() {
            *current = Some(child);
        }
        if let Some(stdout) = stdout {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(event) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if event["event"] == "message"
                    && let Some(message) = event["message"].as_str()
                {
                    let _ = tx.send_blocking(message.to_string());
                }
            }
        }
        // Replaced or turned off: the slot no longer holds this curl.
        let still_mine = slot
            .lock()
            .ok()
            .and_then(|current| current.as_ref().map(|c| c.id() == id))
            .unwrap_or(false);
        if !still_mine || tx.is_closed() {
            return;
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

/// Act on a reply from the phone.
fn apply(app: &Entity<AppState>, message: &str, cx: &mut App) {
    let Some(reply) = parse_reply(message) else {
        return;
    };
    let session_of =
        |thread: ThreadId, cx: &mut App| app.update(cx, |app, cx| app.session(thread, cx));
    match reply {
        Reply::Approve {
            thread,
            request,
            decision,
        } => {
            if let Some(session) = session_of(thread, cx) {
                session.update(cx, |session, cx| session.respond(&request, decision, cx));
            }
        }
        Reply::Choose {
            thread,
            request,
            choice,
        } => {
            if let Some(session) = session_of(thread, cx) {
                session.update(cx, |session, cx| {
                    let confirm = session.items.iter().any(|item| {
                        matches!(&item.content, ItemContent::Question { request: r, .. }
                            if r.request_id == request && r.kind == QuestionKind::Confirm)
                    });
                    let answer = if confirm {
                        QuestionAnswer::Confirmed {
                            confirmed: choice == "yes",
                        }
                    } else {
                        QuestionAnswer::Answers {
                            answers: vec![choice],
                        }
                    };
                    session.answer(&request, answer, cx);
                });
            }
        }
        Reply::Text(text) => {
            let Some(thread) = cx.global::<Phone>().last_thread else {
                return;
            };
            let Some(session) = session_of(thread, cx) else {
                return;
            };
            session.update(cx, |session, cx| {
                // An open one-question form takes it as its answer; else a message.
                let open = session
                    .items
                    .iter()
                    .rev()
                    .find_map(|item| match &item.content {
                        ItemContent::Question {
                            request,
                            answer: None,
                        } if request.questions.len() == 1
                            && request.kind != QuestionKind::Confirm =>
                        {
                            Some(request.request_id.clone())
                        }
                        _ => None,
                    });
                match open {
                    Some(request) => session.answer(
                        &request,
                        QuestionAnswer::Answers {
                            answers: vec![text],
                        },
                        cx,
                    ),
                    None => session.submit(elyra_provider::Prompt::text(text), cx),
                }
            });
        }
    }
}

/// Stop listening (the app quits).
pub fn shutdown(cx: &mut App) {
    if let Some(phone) = cx.try_global::<Phone>()
        && let Some(mut child) = phone.child.lock().ok().and_then(|mut c| c.take())
    {
        let _ = child.kill();
    }
}

/// A private topic name: hard to guess, so only you read and answer it.
pub fn new_topic() -> String {
    format!("elyra-{}", &uuid::Uuid::new_v4().simple().to_string()[..24])
}

/// What happened in a thread, for the push.
pub enum Event {
    NeedsYou,
    Finished,
    Failed,
}

/// A thread needs the user or ended its turn: push it when Elyra Workspace
/// isn't the active app.
pub fn on_session_event(app: &Entity<AppState>, thread: ThreadId, needs_you: bool, cx: &mut App) {
    if cx.active_window().is_some() {
        return;
    }
    let event = if needs_you {
        Event::NeedsYou
    } else {
        let failed = app
            .read(cx)
            .existing_session(thread)
            .is_some_and(|session| {
                let session = session.read(cx);
                session.checks_failed() || session.thread.status == elyra_core::ThreadStatus::Failed
            });
        if failed {
            Event::Failed
        } else {
            Event::Finished
        }
    };
    notify(app, thread, event, cx);
}

/// Send a push about `thread` (when sending to the phone is on).
pub fn notify(app: &Entity<AppState>, thread: ThreadId, event: Event, cx: &mut App) {
    let Some((server, topic)) = target(cx) else {
        return;
    };
    let Some(payload) = payload(app, thread, &event, &server, &topic, cx) else {
        return;
    };
    cx.global_mut::<Phone>().last_thread = Some(thread);
    publish(server, payload);
}

/// Send a push with a message of its own about `thread` (when sending to
/// the phone is on and Elyra Workspace isn't the active app).
pub fn notify_message(app: &Entity<AppState>, thread: ThreadId, message: &str, cx: &mut App) {
    if cx.active_window().is_some() {
        return;
    }
    let Some((server, topic)) = target(cx) else {
        return;
    };
    let Some(title) = app.read(cx).thread(thread).map(|t| {
        let project = app
            .read(cx)
            .project(t.project_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        format!("{project}: {}", t.title)
    }) else {
        return;
    };
    cx.global_mut::<Phone>().last_thread = Some(thread);
    publish(
        server,
        json!({ "topic": topic, "title": title, "message": message, "tags": ["eyes"], "priority": 4 }),
    );
}

/// Send a test push.
pub fn send_test(cx: &mut App) -> Result<(), String> {
    let (server, topic) = target(cx).ok_or("Turn it on and set a topic first.")?;
    publish(
        server,
        json!({
            "topic": topic,
            "title": "Elyra Workspace",
            "message": "Pushes from your threads arrive here. Buttons answer them; to reply in your own words, publish to this topic followed by -reply.",
            "tags": ["robot"],
        }),
    );
    Ok(())
}

fn publish(server: String, payload: Value) {
    std::thread::spawn(move || {
        let child = Command::new("curl")
            .args([
                "-s",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "--data-binary",
                "@-",
                &server,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(payload.to_string().as_bytes());
            }
            let _ = child.wait();
        }
    });
}

fn payload(
    app: &Entity<AppState>,
    thread_id: ThreadId,
    event: &Event,
    server: &str,
    topic: &str,
    cx: &App,
) -> Option<Value> {
    let state = app.read(cx);
    let thread = state.thread(thread_id)?;
    let project = state
        .project(thread.project_id)
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let session = state.existing_session(thread_id)?;
    let items = &session.read(cx).items;
    let reply_url = format!("{server}/{topic}-reply");
    let button = |label: &str, body: String| json!({ "action": "http", "label": label, "url": reply_url, "method": "POST", "body": body, "clear": true });
    let mut actions: Vec<Value> = Vec::new();
    let (message, tags, priority) = match event {
        Event::NeedsYou => {
            let pending = items.iter().rev().find_map(|item| match &item.content {
                ItemContent::Approval {
                    request_id,
                    tool_name,
                    description,
                    decision: None,
                    ..
                } => Some((
                    request_id.clone(),
                    format!(
                        "Wants to use {tool_name}{}",
                        description
                            .as_deref()
                            .map(|d| format!(": {d}"))
                            .unwrap_or_default()
                    ),
                    None,
                )),
                ItemContent::Question {
                    request,
                    answer: None,
                } => {
                    let question = request.questions.first();
                    let text = question
                        .map(|q| q.question.clone())
                        .unwrap_or_else(|| request.title.clone());
                    Some((request.request_id.clone(), text, Some(request.clone())))
                }
                _ => None,
            });
            match pending {
                Some((request, text, None)) => {
                    actions.push(button("Allow", format!("allow {thread_id} {request}")));
                    actions.push(button("Always", format!("always {thread_id} {request}")));
                    actions.push(button("Deny", format!("deny {thread_id} {request}")));
                    (text, vec!["warning"], 4)
                }
                Some((request, text, Some(form))) => {
                    if form.kind == QuestionKind::Confirm {
                        actions.push(button("Yes", format!("choose {thread_id} {request} yes")));
                        actions.push(button("No", format!("choose {thread_id} {request} no")));
                    } else if let [question] = form.questions.as_slice()
                        && !question.multi_select
                        && (1..=3).contains(&question.options.len())
                    {
                        for option in &question.options {
                            actions.push(button(
                                &option.label,
                                format!("choose {thread_id} {request} {}", option.label),
                            ));
                        }
                    }
                    let hint = if actions.is_empty() {
                        "\n\nAnswer by publishing to the topic with -reply."
                    } else {
                        ""
                    };
                    (format!("{text}{hint}"), vec!["question"], 4)
                }
                None => ("Needs your input.".to_string(), vec!["warning"], 4),
            }
        }
        Event::Finished | Event::Failed => {
            let reply = items.iter().rev().find_map(|item| match &item.content {
                ItemContent::Assistant {
                    text,
                    parent_tool_use_id: None,
                } if !text.trim().is_empty() => Some(
                    text.lines()
                        .map(str::trim)
                        .find(|l| !l.is_empty())
                        .unwrap_or("")
                        .to_string(),
                ),
                _ => None,
            });
            let failed = matches!(event, Event::Failed);
            let text = match (failed, reply) {
                (true, _) => "Stopped: the checks or the turn failed.".to_string(),
                (false, Some(reply)) => reply.chars().take(300).collect(),
                (false, None) => "Finished.".to_string(),
            };
            (
                text,
                vec![if failed { "x" } else { "white_check_mark" }],
                if failed { 4 } else { 3 },
            )
        }
    };
    Some(json!({
        "topic": topic,
        "title": format!("{project}: {}", thread.title),
        "message": message,
        "tags": tags,
        "priority": priority,
        "actions": actions,
    }))
}

#[cfg(test)]
mod tests {
    use super::{Reply, parse_reply};
    use elyra_core::ApprovalDecision;

    #[test]
    fn reads_replies_from_the_phone() {
        let thread = elyra_core::new_id();
        assert_eq!(
            parse_reply(&format!("allow {thread} req-1")),
            Some(Reply::Approve {
                thread,
                request: "req-1".into(),
                decision: ApprovalDecision::Allowed
            })
        );
        assert_eq!(
            parse_reply(&format!("deny {thread} req-1")),
            Some(Reply::Approve {
                thread,
                request: "req-1".into(),
                decision: ApprovalDecision::Denied
            })
        );
        assert_eq!(
            parse_reply(&format!("choose {thread} q-2 Built-in tests")),
            Some(Reply::Choose {
                thread,
                request: "q-2".into(),
                choice: "Built-in tests".into()
            })
        );
        assert_eq!(
            parse_reply("use rstest please"),
            Some(Reply::Text("use rstest please".into()))
        );
        assert_eq!(
            parse_reply("allow everything"),
            Some(Reply::Text("allow everything".into())),
            "not a thread id"
        );
        assert_eq!(parse_reply("   "), None);
    }
}
