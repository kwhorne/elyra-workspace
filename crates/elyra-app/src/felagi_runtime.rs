//! Elyra Workspace as one of Elyra Félagi's runtimes: work Félagi assigns to
//! an agent is claimed by this Mac and run as a thread, where you can watch
//! and step in, and what the agent does is reported back as it happens.
//!
//! Speaks Félagi's daemon protocol (see `felagi::Daemon`): register the
//! machine with the agents it can run, heartbeat every 30 seconds, ask for
//! work every few seconds, then for a task: start, stream the transcript as
//! messages (renewing the lease), report the session, and complete or fail.
//! Off unless Settings → Félagi → Run Félagi agents on this Mac is on.

use crate::app_state::AppState;
use crate::felagi::{self, Daemon, DaemonTask, Heartbeat, TaskAnswer};
use crate::thread_session::{SessionEvent, ThreadSession};
use elyra_core::{Environment, ItemContent, ProjectId, ThreadStatus};
use elyra_provider::Prompt;
use gpui_kit::*;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// How often the runtime looks for work and moves its runs along.
const TICK: Duration = Duration::from_secs(3);
/// How often Félagi is told the machine is still here.
const HEARTBEAT: Duration = Duration::from_secs(30);
/// A run silent for this long sends a line anyway, to keep its lease.
const KEEPALIVE: Duration = Duration::from_secs(90);
/// Runs at a time on this Mac.
const MAX_RUNS: usize = 1;
/// Longest text sent in one message.
const MAX_TEXT: usize = 8_000;

/// The settings key holding this machine's daemon id.
const DAEMON_ID_KEY: &str = "felagi_daemon_id";

/// One task this Mac is running.
struct Run {
    task: DaemonTask,
    session: Entity<ThreadSession>,
    /// Transcript items already sent to Félagi.
    sent: usize,
    seq: u64,
    last_sent: Instant,
    session_reported: bool,
    /// A network call about this run is under way.
    busy: bool,
    /// The agent's turn ended and the result is still to be sent.
    ended: bool,
    finishing: bool,
    _subscription: Subscription,
}

pub struct Runtime {
    app: Entity<AppState>,
    runs: Vec<Run>,
    registered: bool,
    last_heartbeat: Option<Instant>,
    /// A loop call (register, heartbeat, claim) is under way.
    busy: bool,
    status: String,
    /// Félagi's own daemon runs on this Mac too: both would compete for work.
    other_daemon: bool,
    _loop: Task<()>,
}

struct RuntimeHandle(Entity<Runtime>);

impl Global for RuntimeHandle {}

/// Start the runtime (it idles while switched off).
pub fn init(app: Entity<AppState>, cx: &mut App) {
    let runtime = cx.new(|cx| Runtime::new(app, cx));
    cx.set_global(RuntimeHandle(runtime));
}

/// What the runtime is doing, for Settings.
pub fn status(cx: &App) -> String {
    cx.try_global::<RuntimeHandle>()
        .map(|handle| {
            let runtime = handle.0.read(cx);
            if runtime.other_daemon {
                format!(
                    "{} · Félagi's own daemon runs on this Mac too; stop one of them, or they compete for the same work.",
                    runtime.status
                )
            } else {
                runtime.status.clone()
            }
        })
        .unwrap_or_default()
}

/// On quit: hand running tasks back so Félagi requeues them at once.
pub fn hand_back(cx: &mut App) {
    let Some(handle) = cx.try_global::<RuntimeHandle>() else {
        return;
    };
    let runtime = handle.0.clone();
    let Some(daemon) = runtime.read(cx).daemon(cx) else {
        return;
    };
    for run in &runtime.read(cx).runs {
        let _ = daemon.fail(
            &run.task,
            "Elyra Workspace quit while the agent was working.",
            true,
        );
    }
}

fn enabled(cx: &App) -> bool {
    crate::preferences::Preferences::global(cx).felagi_runtime
}

/// The name this machine registers under.
pub fn machine_name(cx: &App) -> String {
    let name = crate::preferences::Preferences::global(cx)
        .felagi_runtime_name
        .trim()
        .to_string();
    if !name.is_empty() {
        return name;
    }
    std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Elyra Workspace".into())
}

/// One output line for Félagi.
#[derive(Debug, PartialEq)]
struct Message {
    kind: &'static str,
    content: Option<String>,
    tool: Option<String>,
    payload: Option<Value>,
}

impl Message {
    fn new(kind: &'static str) -> Self {
        Self {
            kind,
            content: None,
            tool: None,
            payload: None,
        }
    }
}

/// The agent message for one transcript item, if it is worth sending.
fn message_for(content: &ItemContent) -> Option<Message> {
    let clip = |text: &str| Some(text.chars().take(MAX_TEXT).collect::<String>());
    Some(match content {
        ItemContent::Assistant { text, .. } if !text.trim().is_empty() => Message {
            content: clip(text),
            ..Message::new("assistant")
        },
        ItemContent::User { text, .. } => Message {
            content: clip(text),
            ..Message::new("user")
        },
        ItemContent::ToolUse { name, input, .. } => Message {
            tool: Some(name.clone()),
            payload: Some(if input.is_object() {
                input.clone()
            } else {
                json!({ "input": input })
            }),
            ..Message::new("tool_use")
        },
        ItemContent::ToolResult {
            content, is_error, ..
        } => Message {
            content: clip(content),
            payload: Some(json!({ "is_error": is_error })),
            ..Message::new("tool_result")
        },
        ItemContent::Notice { text, .. } => Message {
            content: clip(text),
            ..Message::new("system")
        },
        ItemContent::Approval { tool_name, .. } => Message {
            content: Some(format!(
                "Waiting for approval in Elyra Workspace: {tool_name}"
            )),
            ..Message::new("system")
        },
        _ => return None,
    })
}

/// The run's transcript items not yet sent, as numbered messages.
fn collect_messages(run: &mut Run, cx: &App) -> Vec<Value> {
    let session = run.session.read(cx);
    let mut messages = Vec::new();
    for item in &session.items[run.sent.min(session.items.len())..] {
        if let Some(Message {
            kind,
            content,
            tool,
            payload,
        }) = message_for(&item.content)
        {
            run.seq += 1;
            let mut message = json!({ "seq": run.seq, "type": kind });
            if let Some(content) = content {
                message["content"] = content.into();
            }
            if let Some(tool) = tool {
                message["tool"] = tool.into();
            }
            if let Some(payload) = payload {
                message["payload"] = payload;
            }
            messages.push(message);
        }
    }
    run.sent = session.items.len();
    messages
}

impl Runtime {
    fn new(app: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            app,
            runs: Vec::new(),
            registered: false,
            last_heartbeat: None,
            busy: false,
            status: "Off".into(),
            other_daemon: false,
            _loop: tick,
        }
    }

    /// The daemon client, when Félagi is connected and a daemon token is set.
    fn daemon(&self, cx: &App) -> Option<Daemon> {
        let store = &self.app.read(cx).store;
        let connection = felagi::connection(store)?;
        let token = felagi::daemon_token(&connection.url)?;
        let id = match store
            .setting(DAEMON_ID_KEY)
            .ok()
            .flatten()
            .filter(|id| !id.is_empty())
        {
            Some(id) => id,
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                let _ = store.set_setting(DAEMON_ID_KEY, &id);
                id
            }
        };
        Some(Daemon::new(&connection.url, &token, &id))
    }

    fn set_status(&mut self, status: impl Into<String>, cx: &mut Context<Self>) {
        let status = status.into();
        if self.status != status {
            self.status = status;
            cx.notify();
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        // Runs carry on even when switched off, so work in hand is finished.
        let ended: Vec<String> = self
            .runs
            .iter()
            .filter(|r| r.ended && !r.finishing && !r.busy)
            .map(|r| r.task.id.clone())
            .collect();
        for ulid in ended {
            self.finish(&ulid, cx);
        }
        let indices: Vec<usize> = (0..self.runs.len()).collect();
        for index in indices {
            self.advance(index, cx);
        }
        if !enabled(cx) {
            if self.runs.is_empty() {
                self.registered = false;
                self.set_status("Off", cx);
            }
            return;
        }
        let Some(daemon) = self.daemon(cx) else {
            self.set_status("Needs Félagi connected and a daemon token", cx);
            return;
        };
        if self.busy {
            return;
        }
        let providers: Vec<&'static str> = crate::preferences::Preferences::global(cx)
            .enabled_providers()
            .into_iter()
            .filter_map(felagi::provider_name)
            .collect();
        let name = machine_name(cx);
        let register = !self.registered;
        let heartbeat = self
            .last_heartbeat
            .is_none_or(|at| at.elapsed() >= HEARTBEAT);
        // No new work while waiting to quit or restart.
        let claim = self.runs.len() < MAX_RUNS && !crate::quitting::draining(cx);
        self.busy = true;
        let job = cx.background_executor().spawn(async move {
            let other_daemon = (register || heartbeat).then(|| {
                std::process::Command::new("pgrep")
                    .args(["-x", "felagi-daemon"])
                    .output()
                    .is_ok_and(|o| o.status.success())
            });
            // Register when new here, or when Félagi has forgotten the machine.
            if register || (heartbeat && matches!(daemon.heartbeat()?, Heartbeat::Gone)) {
                daemon.register(&name, &providers)?;
            }
            let task = if claim { daemon.claim()? } else { None };
            anyhow::Ok((task, other_daemon))
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((task, other_daemon)) => {
                        if let Some(other) = other_daemon {
                            this.other_daemon = other;
                        }
                        this.registered = true;
                        if register || heartbeat {
                            this.last_heartbeat = Some(Instant::now());
                        }
                        if let Some(task) = task {
                            this.take(task, cx);
                        } else if this.runs.is_empty() {
                            this.set_status(
                                format!(
                                    "Ready as \u{201c}{}\u{201d}, waiting for work",
                                    machine_name(cx)
                                ),
                                cx,
                            );
                        }
                    }
                    Err(err) => {
                        this.registered = false;
                        this.set_status(format!("{err:#}"), cx);
                    }
                }
            });
        })
        .detach();
    }

    /// A task was claimed: find its project and run it in a new thread.
    fn take(&mut self, task: DaemonTask, cx: &mut Context<Self>) {
        let Some(daemon) = self.daemon(cx) else {
            return;
        };
        let projects: Vec<(ProjectId, std::path::PathBuf)> = self
            .app
            .read(cx)
            .projects
            .iter()
            .map(|p| (p.id, p.path.clone()))
            .collect();
        let wanted: Vec<String> = task
            .workspace
            .repositories
            .iter()
            .map(|r| felagi::repository_key(r))
            .collect();
        let job = cx.background_executor().spawn(async move {
            projects.into_iter().find_map(|(id, path)| {
                let output = std::process::Command::new("git")
                    .current_dir(&path)
                    .args(["remote", "get-url", "origin"])
                    .output()
                    .ok()?;
                let key = felagi::repository_key(&String::from_utf8_lossy(&output.stdout));
                wanted.contains(&key).then_some(id)
            })
        });
        cx.spawn(async move |this, cx| {
            let project = job.await;
            let _ = this.update(cx, |this, cx| this.run(task, project, daemon, cx));
        })
        .detach();
    }

    fn run(
        &mut self,
        task: DaemonTask,
        project: Option<ProjectId>,
        daemon: Daemon,
        cx: &mut Context<Self>,
    ) {
        let refuse = {
            let daemon = daemon.clone();
            move |task: DaemonTask, why: String| {
                std::thread::spawn(move || {
                    let _ = daemon.fail(&task, &why, false);
                });
            }
        };
        let Some(kind) = felagi::provider_kind(&task.agent.provider) else {
            let why = format!("Elyra Workspace can't run {} agents.", task.agent.provider);
            self.set_status(why.clone(), cx);
            return refuse(task, why);
        };
        let Some(project) = project else {
            let why = format!(
                "No project in Elyra Workspace on {} has {}.",
                machine_name(cx),
                if task.workspace.repositories.is_empty() {
                    "the repository (the task names none)".to_string()
                } else {
                    task.workspace.repositories.join(" or ")
                }
            );
            self.set_status(why.clone(), cx);
            return refuse(task, why);
        };
        let thread = match self
            .app
            .update(cx, |app, cx| app.create_thread(project, cx))
        {
            Ok(thread) => thread,
            Err(err) => return refuse(task, format!("{err:#}")),
        };
        let Some(session) = self.app.update(cx, |app, cx| app.session(thread.id, cx)) else {
            return refuse(task, "Elyra Workspace couldn't open the thread.".into());
        };
        let prompt = task.prompt();
        let (agent, identifier, title, workspace) = (
            task.agent.name.clone(),
            task.issue.identifier.clone(),
            task.issue.title.clone(),
            task.workspace.slug.clone(),
        );
        let model = task.agent.model.clone().filter(|m| !m.is_empty());
        session.update(cx, |session, cx| {
            session.set_provider(kind, cx);
            if model.is_some() {
                session.set_model(model, cx);
            }
            session.use_worktree = true;
            session.link_felagi(identifier.clone(), cx);
            session.rename(format!("{agent} · {identifier} {title}"), cx);
            session.notice(
                format!(
                    "Félagi's agent {agent} runs {identifier} here for {workspace}. What it does is sent to Félagi as it happens, and the result when its turn ends. Step in at any time."
                ),
                false,
                cx,
            );
            session.submit(Prompt::text(prompt), cx);
        });
        let sent = session.read(cx).items.len();
        let ulid = task.id.clone();
        let subscription = cx.subscribe(&session, move |this, _, event: &SessionEvent, cx| {
            if let SessionEvent::TurnCompleted = event {
                this.finish(&ulid, cx);
            }
        });
        self.set_status(format!("Running {identifier} for {agent}"), cx);
        self.runs.push(Run {
            task: task.clone(),
            session,
            sent,
            seq: 0,
            last_sent: Instant::now(),
            session_reported: false,
            busy: true,
            ended: false,
            finishing: false,
            _subscription: subscription,
        });
        let job = cx
            .background_executor()
            .spawn(async move { daemon.start(&task) });
        let ulid = self
            .runs
            .last()
            .map(|r| r.task.id.clone())
            .unwrap_or_default();
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| this.answered(&ulid, result, cx));
        })
        .detach();
    }

    /// Send a run's new output (or a keepalive), and its session once known.
    fn advance(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(daemon) = self.daemon(cx) else {
            return;
        };
        let run = &mut self.runs[index];
        if run.busy || run.finishing {
            return;
        }
        let mut messages = collect_messages(run, cx);
        let session = run.session.read(cx);
        if messages.is_empty() && run.last_sent.elapsed() >= KEEPALIVE {
            run.seq += 1;
            messages.push(json!({
                "seq": run.seq,
                "type": "system",
                "content": format!("Still running — no output for {}s", run.last_sent.elapsed().as_secs()),
            }));
        }
        let new_session = (!run.session_reported)
            .then(|| session.thread.provider_session_id.clone())
            .flatten()
            .map(|id| (id, session.working_dir().display().to_string()));
        if messages.is_empty() && new_session.is_none() {
            return;
        }
        run.busy = true;
        if !messages.is_empty() {
            run.last_sent = Instant::now();
        }
        if new_session.is_some() {
            run.session_reported = true;
        }
        let task = run.task.clone();
        let ulid = task.id.clone();
        let job = cx.background_executor().spawn(async move {
            if let Some((id, dir)) = new_session
                && let TaskAnswer::Lost(why) = daemon.session(&task, &id, &dir)?
            {
                return Ok(TaskAnswer::Lost(why));
            }
            // Félagi takes up to 200 lines a request.
            let mut answer = TaskAnswer::Ok { cancel: false };
            for chunk in messages.chunks(200) {
                answer = daemon.messages(&task, chunk.to_vec())?;
                if !matches!(answer, TaskAnswer::Ok { cancel: false }) {
                    break;
                }
            }
            Ok(answer)
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| this.answered(&ulid, result, cx));
        })
        .detach();
    }

    /// Act on what Félagi said about a run: carry on, stop it, or let it go.
    fn answered(&mut self, ulid: &str, result: anyhow::Result<TaskAnswer>, cx: &mut Context<Self>) {
        let Some(index) = self.runs.iter().position(|r| r.task.id == ulid) else {
            return;
        };
        self.runs[index].busy = false;
        match result {
            Ok(TaskAnswer::Ok { cancel: false }) => {}
            Ok(TaskAnswer::Ok { cancel: true }) => {
                let run = self.runs.remove(index);
                run.session.update(cx, |session, cx| {
                    session.interrupt(cx);
                    session.notice(
                        "Félagi asked to stop this run, so it was stopped.",
                        false,
                        cx,
                    );
                });
                if let Some(daemon) = self.daemon(cx) {
                    std::thread::spawn(move || {
                        let _ = daemon.cancel_ack(&run.task);
                    });
                }
                self.set_status("Stopped a run Félagi cancelled", cx);
            }
            Ok(TaskAnswer::Lost(why)) => {
                let run = self.runs.remove(index);
                run.session.update(cx, |session, cx| {
                    session.notice(
                        format!("This run is no longer Félagi's on this Mac: {why}"),
                        true,
                        cx,
                    )
                });
            }
            // A network hiccup: the next tick tries again.
            Err(err) => self.set_status(format!("{err:#}"), cx),
        }
    }

    /// The agent's turn ended: send the rest of its output, then the result.
    fn finish(&mut self, ulid: &str, cx: &mut Context<Self>) {
        let Some(index) = self.runs.iter().position(|r| r.task.id == ulid) else {
            return;
        };
        let session = self.runs[index].session.read(cx);
        // Wait for queued messages and goals: the run ends when the thread rests.
        if session.running || !session.queued.is_empty() || session.preparing.is_some() {
            return;
        }
        let Some(daemon) = self.daemon(cx) else {
            return;
        };
        let run = &mut self.runs[index];
        run.ended = true;
        if run.busy {
            // Output is being sent; the next tick finishes it.
            return;
        }
        run.finishing = true;
        // What the turn wrote last goes out before the result.
        let messages = collect_messages(run, cx);
        let session = run.session.read(cx);
        let failed = session.thread.status == ThreadStatus::Failed;
        let reply = session.last_reply();
        let cwd = session.working_dir();
        let session_id = session.thread.provider_session_id.clone();
        let branch = match &session.thread.environment {
            Environment::Worktree { branch, .. } => Some(branch.clone()),
            Environment::Local => None,
        };
        let cost_micros = (session.spent_usd() * 1_000_000.).round() as u64;
        let task = run.task.clone();
        let ulid = ulid.to_string();
        let job = cx.background_executor().spawn(async move {
            for chunk in messages.chunks(200) {
                daemon.messages(&task, chunk.to_vec())?;
            }
            if failed {
                let error: String = if reply.trim().is_empty() {
                    "The agent's turn failed in Elyra Workspace.".to_string()
                } else {
                    reply.chars().take(2_000).collect()
                };
                daemon.fail(&task, &error, false)?;
                return anyhow::Ok(false);
            }
            let mut artifacts = Vec::new();
            if let Some(pr) = std::process::Command::new("gh")
                .current_dir(&cwd)
                .args(["pr", "view", "--json", "url,title,state", "--jq", "[.url,.title,.state]|@tsv"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|line| line.starts_with("https://"))
            {
                let mut parts = pr.split('\t');
                artifacts.push(json!({
                    "type": "pull_request",
                    "url": parts.next().unwrap_or_default(),
                    "title": parts.next().unwrap_or_default(),
                    "state": parts.next().unwrap_or("open").to_lowercase(),
                }));
            }
            if let Some(branch) = branch {
                artifacts.push(json!({ "type": "branch", "reference": branch }));
            }
            let mut result = json!({
                "summary": if reply.trim().is_empty() { "Done.".to_string() } else { reply.chars().take(20_000).collect() },
                "artifacts": artifacts,
            });
            if cost_micros > 0 {
                result["usage"] = json!({ "cost_micros": cost_micros });
            }
            let mut body = json!({ "result": result, "work_dir": cwd.display().to_string() });
            if let Some(id) = session_id {
                body["session_id"] = id.into();
            }
            daemon.complete(&task, body)?;
            anyhow::Ok(true)
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                let Some(index) = this.runs.iter().position(|r| r.task.id == ulid) else {
                    return;
                };
                let run = this.runs.remove(index);
                let note = match &result {
                    Ok(true) => "The result was reported to Félagi.".to_string(),
                    Ok(false) => "The failure was reported to Félagi.".to_string(),
                    Err(err) => format!("Could not report to Félagi: {err:#}"),
                };
                run.session
                    .update(cx, |session, cx| session.notice(note, result.is_err(), cx));
                this.set_status(
                    format!(
                        "Finished {} ({})",
                        run.task.issue.identifier,
                        if matches!(result, Ok(true)) {
                            "done"
                        } else {
                            "not done"
                        }
                    ),
                    cx,
                );
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::message_for;
    use elyra_core::ItemContent;
    use serde_json::json;

    #[test]
    fn turns_transcript_items_into_messages() {
        let message = message_for(&ItemContent::Assistant {
            text: "Reading the test.".into(),
            parent_tool_use_id: None,
        })
        .unwrap();
        assert_eq!(message.kind, "assistant");
        assert_eq!(message.content.as_deref(), Some("Reading the test."));
        let message = message_for(&ItemContent::ToolUse {
            tool_use_id: "1".into(),
            name: "Bash".into(),
            input: json!({ "command": "php artisan test" }),
            parent_tool_use_id: None,
        })
        .unwrap();
        assert_eq!(message.kind, "tool_use");
        assert_eq!(message.tool.as_deref(), Some("Bash"));
        assert_eq!(message.payload.unwrap()["command"], "php artisan test");
        assert!(message_for(&ItemContent::Thinking { text: "hmm".into() }).is_none());
    }
}
