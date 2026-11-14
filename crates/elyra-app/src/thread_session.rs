//! A thread's session: its agent, its transcript and what happens around a
//! turn. This module has the state and keeping it; beside it are `sending`
//! (a message to the agent), `events` (what the agent sends, the end of a
//! turn), `decisions` (approvals, answers, interrupting), `checkpoints`
//! (restore, undo, who broke it), `settings`, and what runs after a turn:
//! `checks`, `journeys`, `kept_tests`, `mutation_gate`, `stuck`, `goals`,
//! `lines`, `proposals` and `cross_examination`.

use crate::app_state::AppState;
use chrono::Utc;
use elyra_core::{
    ApprovalDecision, Environment, ExamState, GoalStatus, ItemContent, ItemId, PermissionMode,
    Project, ProviderKind, QuestionAnswer, Thread, ThreadId, ThreadStatus, TranscriptItem,
};
use elyra_provider::{
    AgentSession, ModelOption, PermissionResponse, Prompt, ProviderEvent, SessionConfig,
    SlashCommand, Usage,
};
use gpui_kit::{App, Context, EventEmitter, Task, WeakEntity};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod checkpoints;
mod checks;
mod cross_examination;
mod decisions;
mod events;
mod goals;
mod journeys;
mod kept_tests;
mod lines;
mod mutation_gate;
mod proposals;
mod sending;
mod settings;
mod stuck;

pub enum SessionEvent {
    /// A message went to the agent and a turn began.
    TurnStarted,
    /// Grove runs the thread's worktree as its own site, at this address.
    SiteReady(String),
    /// A turn finished; the working tree may have changed.
    TurnCompleted,
    /// The agent needs the user (approval or question).
    NeedsAttention,
    /// The folder's files changed outside a turn (an instruction undone).
    FilesChanged,
    /// More of the reply, thinking or a tool's output streamed in. Only the
    /// thread's own view redraws for it; observers of the session aren't
    /// notified (it happens many times a second).
    Streamed,
    /// The mutation gate finished testing the tests (a duel hands what got
    /// through to Red).
    TestsTested,
}

/// One thread's live state: transcript, in-flight streaming text, queued
/// messages and the provider process. Survives tab switches; owned by
/// [`AppState`].
pub struct ThreadSession {
    app: WeakEntity<AppState>,
    pub thread: Thread,
    pub project: Project,
    pub items: Vec<TranscriptItem>,
    pub streaming_text: String,
    pub streaming_thinking: String,
    /// How many turns have had their lines recorded, so views that show
    /// what agents wrote know when to look again.
    pub lines_recorded: usize,
    /// How many times the mutation gate finished, for the same reason.
    pub gates_done: usize,
    /// What the last turn took out of other instructions, for the agent's
    /// next message.
    rewrite_note: Option<String>,
    /// Errors the message being worked on carried: (one line, what the agent
    /// was given), for offering to keep them as a test.
    pub errors_in_turn: Vec<(String, String)>,
    /// The card whose test the running turn writes.
    kept_test: Option<ItemId>,
    /// The warm start was looked up for this thread's first message.
    warm_started: bool,
    /// Don't run the checks after turns (Red in a duel writes failing tests).
    pub skip_checks: bool,
    /// The working folder, its repository's shared git directory and its
    /// root, for the intent board (looked up once per folder).
    intent_repo: Option<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf)>,
    pub running: bool,
    /// Status line shown while the environment is being prepared.
    pub preparing: Option<String>,
    /// Transient provider status: compacting, retrying.
    pub activity: Option<String>,
    /// Requested before the first message: run in an isolated worktree.
    pub use_worktree: bool,
    /// Messages waiting for the current turn to finish.
    pub queued: Vec<Prompt>,
    /// Messages the provider has accepted but not yet delivered (steering).
    pub provider_queue: Vec<String>,
    /// Latest output of running tools, by tool use id.
    pub tool_progress: HashMap<String, String>,
    /// Latest known token/context usage and session cost.
    pub usage: Usage,
    pub commands: Vec<SlashCommand>,
    /// Subagents offered as `@agent-<name>` mentions.
    pub agents: Vec<SlashCommand>,
    pub models: Vec<ModelOption>,
    /// Unsent composer text, kept across tab switches.
    pub draft: String,
    /// Failed requests (Grove id, `METHOD /path → status`) the last message
    /// carried, replayed when the turn ends.
    pub replay_after_turn: Vec<(u64, String)>,
    provider: Option<Box<dyn AgentSession>>,
    /// Settings changed that only apply when the provider restarts.
    pub(crate) needs_restart: bool,
    /// The next launch forks `provider_session_id` (side chats).
    pub fork_pending: bool,
    pub recapping: bool,
    /// Stops the project's checks while they run.
    check_cancel: Option<Arc<AtomicBool>>,
    /// Automatic fixes sent for failed checks since the last message.
    check_attempts: u32,
    /// The user stopped this turn: don't check it.
    stopped: bool,
    /// Model and effort to go back to after an escalated fix.
    escalated_from: Option<(Option<String>, Option<String>)>,
    /// Watches the running turn's tool calls for an agent going in circles.
    thrash: crate::thrash_guard::Guard,
    /// Why the running turn is being stopped to start over, once it ends.
    start_over: Option<String>,
    /// Cross-examinations of plans under way: the card, and the side chat
    /// being followed.
    examinations: HashMap<ItemId, gpui_kit::Subscription>,
    /// Stops the mutation gate while it runs.
    mutation_cancel: Option<Arc<AtomicBool>>,
    /// The thread's folder has kept invariants (`.elyra/invariants`).
    invariants_seen: bool,
    _events: Option<Task<()>>,
}

impl EventEmitter<SessionEvent> for ThreadSession {}

impl ThreadSession {
    pub fn new(
        app: WeakEntity<AppState>,
        thread: Thread,
        project: Project,
        items: Vec<TranscriptItem>,
        models: Vec<ModelOption>,
    ) -> Self {
        let usage = items
            .iter()
            .rev()
            .find_map(|item| match item.content {
                ItemContent::TurnSummary {
                    context_tokens,
                    context_window,
                    ..
                } if context_tokens.is_some() => Some(Usage {
                    context_tokens,
                    context_window,
                    ..Usage::default()
                }),
                _ => None,
            })
            .unwrap_or_default();
        Self {
            app,
            thread,
            project,
            items,
            streaming_text: String::new(),
            streaming_thinking: String::new(),
            lines_recorded: 0,
            gates_done: 0,
            rewrite_note: None,
            errors_in_turn: Vec::new(),
            kept_test: None,
            warm_started: false,
            skip_checks: false,
            intent_repo: None,
            running: false,
            preparing: None,
            activity: None,
            use_worktree: false,
            queued: Vec::new(),
            provider_queue: Vec::new(),
            tool_progress: HashMap::new(),
            usage,
            commands: Vec::new(),
            agents: Vec::new(),
            models,
            draft: String::new(),
            replay_after_turn: Vec::new(),
            provider: None,
            needs_restart: false,
            fork_pending: false,
            recapping: false,
            check_cancel: None,
            check_attempts: 0,
            stopped: false,
            escalated_from: None,
            thrash: Default::default(),
            start_over: None,
            examinations: HashMap::new(),
            mutation_cancel: None,
            invariants_seen: false,
            _events: None,
        }
    }

    pub fn capabilities(&self) -> elyra_provider::Capabilities {
        elyra_provider::capabilities(self.thread.provider)
    }

    /// Adopt a relocated project. Any idle provider process is stopped so the
    /// next message starts in the new folder (resuming the same conversation).
    pub fn set_project(&mut self, project: Project, cx: &mut Context<Self>) {
        self.project = project;
        if !self.running {
            self.stop_provider();
        }
        cx.notify();
    }

    pub fn working_dir(&self) -> std::path::PathBuf {
        self.thread.working_dir(&self.project)
    }

    pub fn has_started(&self) -> bool {
        !self.items.is_empty() || self.running || self.preparing.is_some()
    }

    pub fn pending_approval(&self) -> bool {
        self.items.iter().any(|item| {
            matches!(
                item.content,
                ItemContent::Approval { decision: None, .. }
                    | ItemContent::Question { answer: None, .. }
            )
        })
    }

    fn stop_provider(&mut self) {
        if let Some(provider) = self.provider.take() {
            provider.shutdown();
        }
        self._events = None;
        self.needs_restart = false;
    }

    fn save_thread(&mut self, cx: &mut Context<Self>) {
        self.thread.updated_at = Utc::now();
        let thread = self.thread.clone();
        let _ = self.app.update(cx, |app, cx| app.save_thread(&thread, cx));
    }

    /// Whether a turn is running, also for the app (the sidebar reads it there
    /// rather than from the session, so streaming doesn't redraw the window).
    fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        self.running = running;
        let id = self.thread.id;
        let _ = self.app.update(cx, |app, cx| {
            let changed = if running {
                app.running.insert(id)
            } else {
                app.running.remove(&id)
            };
            if changed {
                cx.notify();
            }
        });
    }

    fn set_status(&mut self, status: ThreadStatus, cx: &mut Context<Self>) {
        if self.thread.status != status {
            self.thread.status = status;
            self.save_thread(cx);
        }
    }

    fn set_setting(&self, key: &str, value: &str, cx: &mut Context<Self>) {
        let _ = self.app.update(cx, |app, _| app.set_setting(key, value));
    }

    fn append(&mut self, content: ItemContent, cx: &mut Context<Self>) {
        let thread_id = self.thread.id;
        let stored = self.app.read_with(cx, |app, _| {
            app.store.append_item(thread_id, content.clone())
        });
        let item = match stored {
            Ok(Ok(item)) => item,
            other => {
                if let Ok(Err(err)) = other {
                    log::error!("persisting transcript item: {err:#}");
                }
                TranscriptItem {
                    id: elyra_core::new_id(),
                    thread_id,
                    seq: self.items.last().map_or(1, |item| item.seq + 1),
                    content,
                    created_at: Utc::now(),
                }
            }
        };
        self.items.push(item);
        cx.notify();
    }

    fn persist_item(&self, index: usize, cx: &mut Context<Self>) {
        let item = self.items[index].clone();
        let _ = self.app.read_with(cx, |app, _| {
            if let Err(err) = app.store.update_item(&item) {
                log::error!("updating transcript item: {err:#}");
            }
        });
    }

    pub fn notice(&mut self, text: impl Into<String>, is_error: bool, cx: &mut Context<Self>) {
        self.append(
            ItemContent::Notice {
                text: text.into(),
                is_error,
            },
            cx,
        );
    }

    // ---- sending ----------------------------------------------------------

    pub fn app_entity(&self) -> Option<gpui_kit::Entity<AppState>> {
        self.app.upgrade()
    }

    /// Whether the thread's last checks failed (and nothing came after them).
    pub fn checks_failed(&self) -> bool {
        self.items
            .iter()
            .rev()
            .find(|item| {
                !matches!(
                    item.content,
                    ItemContent::TurnSummary { .. } | ItemContent::Notice { .. }
                )
            })
            .is_some_and(|item| {
                matches!(
                    item.content,
                    ItemContent::Check { passed: false, .. }
                        | ItemContent::Journey { passed: false, .. }
                )
            })
    }

    /// The agent's last reply in the latest turn (empty when it wrote none).
    pub fn last_reply(&self) -> String {
        self.items
            .iter()
            .rev()
            .take_while(|item| !matches!(item.content, ItemContent::User { .. }))
            .find_map(|item| match &item.content {
                ItemContent::Assistant {
                    text,
                    parent_tool_use_id: None,
                } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// What the user last asked for.
    pub fn last_request(&self) -> Option<String> {
        self.items
            .iter()
            .rev()
            .find_map(|item| match &item.content {
                ItemContent::User { text, .. } => Some(text.clone()),
                _ => None,
            })
    }

    /// Record pictures of the browser page from before and after a turn.
    pub fn add_page_snapshot(
        &mut self,
        url: String,
        before: Option<String>,
        after: String,
        cx: &mut Context<Self>,
    ) {
        self.append(ItemContent::PageSnapshot { url, before, after }, cx);
    }

    /// What the thread's turns have cost, as reported by its providers.
    /// After a turn: replay the failed requests the message carried.
    fn replay_requests(&mut self, cx: &mut Context<Self>) {
        let requests = std::mem::take(&mut self.replay_after_turn);
        if requests.is_empty() {
            return;
        }
        let job = cx.background_executor().spawn(async move {
            requests
                .into_iter()
                .map(|(id, line)| {
                    let status = crate::grove::replay(id).map_err(|err| format!("{err:#}"));
                    crate::grove::replay_note(&line, status)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let notes = job.await;
            let _ = this.update(cx, |this, cx| {
                for (note, failed) in notes {
                    this.notice(note, failed, cx);
                }
            });
        })
        .detach();
    }
}

/// Database snapshots kept per thread (they can be large).
const MAX_DB_SNAPSHOTS: usize = 10;

fn db_snapshots_key(thread: elyra_core::ThreadId) -> String {
    format!("db_snapshots:{thread}")
}

/// Where a thread's SQLite snapshots are kept.
fn db_snapshot_dir(thread: elyra_core::ThreadId) -> std::path::PathBuf {
    elyra_core::paths::snapshots_dir()
        .join(thread.to_string())
        .join("db")
}

/// A thread's database snapshots: (checkpoint commit, snapshot reference).
pub fn db_snapshots(
    store: &elyra_core::Store,
    thread: elyra_core::ThreadId,
) -> Vec<(String, String)> {
    store
        .setting(&db_snapshots_key(thread))
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// Forget a thread's database snapshots and delete them.
pub fn drop_db_snapshots(store: &elyra_core::Store, thread: elyra_core::ThreadId) {
    for (_, reference) in db_snapshots(store, thread) {
        crate::grove::drop_snapshot(&reference);
    }
    let _ = store.set_setting(&db_snapshots_key(thread), "[]");
}

/// `$1.23`, or `$0.004` for amounts under a cent.
pub fn format_usd(amount: f64) -> String {
    if amount > 0. && amount < 0.01 {
        format!("${amount:.3}")
    } else {
        format!("${amount:.2}")
    }
}

const GOAL_DONE: &str = "GOAL ACHIEVED";
const GOAL_BLOCKED: &str = "GOAL BLOCKED";

const DEBUG_INSTRUCTIONS: &str = "Debug mode: reproduce before you fix. First write a failing test or a minimal reproduction that shows the bug and run it. Then find the root cause (explain it in one or two sentences), make the smallest fix, and show the reproduction passing. Do not change unrelated code.";

/// Models last reported by the provider, or its built-in suggestions.
pub fn cached_models(store: &elyra_core::Store, provider: ProviderKind) -> Vec<ModelOption> {
    let key = format!("models.{}", provider.as_str());
    store
        .setting(&key)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<Vec<ModelOption>>(&json).ok())
        .unwrap_or_else(|| {
            elyra_provider::capabilities(provider)
                .models
                .iter()
                .map(|(id, label)| ModelOption {
                    id: id.to_string(),
                    label: label.to_string(),
                })
                .collect()
        })
}

/// How a prompt is shown in the transcript.
fn user_text(prompt: &Prompt) -> String {
    match prompt.images.len() {
        0 => prompt.text.clone(),
        1 => format!("{}\n\n📎 1 image", prompt.text),
        n => format!("{}\n\n📎 {n} images", prompt.text),
    }
    .trim()
    .to_string()
}

fn title_from_prompt(text: &str) -> String {
    let line = text.lines().next().unwrap_or(text).trim();
    let mut title: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        title.push('…');
    }
    title
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::format_usd;

    #[test]
    fn formats_amounts() {
        assert_eq!(format_usd(5.0), "$5.00");
        assert_eq!(format_usd(1.234), "$1.23");
        assert_eq!(format_usd(0.004), "$0.004");
        assert_eq!(format_usd(0.0), "$0.00");
    }
}
