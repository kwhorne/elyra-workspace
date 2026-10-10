use crate::app_state::AppState;
use chrono::Utc;
use elyra_core::{
    ApprovalDecision, Environment, GoalStatus, ItemContent, ItemId, PermissionMode, Project,
    ProviderKind, QuestionAnswer, Thread, ThreadStatus, TranscriptItem,
};
use elyra_provider::{
    AgentSession, ModelOption, PermissionResponse, Prompt, ProviderEvent, SessionConfig,
    SlashCommand, Usage,
};
use gpui_kit::{App, Context, EventEmitter, Task, WeakEntity};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub enum SessionEvent {
    /// A message went to the agent and a turn began.
    TurnStarted,
    /// Grove runs the thread's worktree as its own site, at this address.
    SiteReady(String),
    /// A turn finished; the working tree may have changed.
    TurnCompleted,
    /// The agent needs the user (approval or question).
    NeedsAttention,
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

    /// Send now, or queue behind the running turn.
    pub fn submit(&mut self, prompt: Prompt, cx: &mut Context<Self>) {
        if prompt.text.trim().is_empty() && prompt.images.is_empty() {
            return;
        }
        self.check_attempts = 0;
        if self.running || self.preparing.is_some() {
            self.queued.push(prompt);
            cx.notify();
        } else {
            self.send(prompt, cx);
        }
    }

    fn send(&mut self, mut prompt: Prompt, cx: &mut Context<Self>) {
        prompt.text = prompt.text.trim().to_string();
        if self.items.is_empty() && self.thread.title == "New thread" && !prompt.text.is_empty() {
            self.thread.title = title_from_prompt(&prompt.text);
        }
        self.append(
            ItemContent::User {
                text: user_text(&prompt),
                checkpoint: None,
            },
            cx,
        );
        let item_id = self.items.last().map(|item| item.id);
        self.running = true;
        self.stopped = false;
        self.set_status(ThreadStatus::Running, cx);
        cx.emit(SessionEvent::TurnStarted);

        let needs_worktree = self.use_worktree
            && matches!(self.thread.environment, Environment::Local)
            && self.thread.provider_session_id.is_none();
        if needs_worktree {
            self.stop_provider();
            self.prepare_worktree_then_send(prompt, item_id, cx);
        } else {
            self.checkpoint_then_deliver(prompt, item_id, cx);
        }
    }

    /// Snapshot the working tree (so the turn can be reviewed and reverted),
    /// then hand the prompt to the provider.
    fn checkpoint_then_deliver(
        &mut self,
        prompt: Prompt,
        item_id: Option<elyra_core::ItemId>,
        cx: &mut Context<Self>,
    ) {
        let cwd = self.working_dir();
        let refname = format!(
            "refs/elyra/{}/{}",
            self.thread.id.simple(),
            self.items.len()
        );
        // In an app Grove runs, the database goes into the checkpoint too.
        let with_database = crate::preferences::Preferences::global(cx).grove_db_checkpoints;
        let snapshots = db_snapshot_dir(self.thread.id);
        let note = format!(
            "Elyra Workspace: before a turn in \u{201c}{}\u{201d}",
            self.thread.title
        );
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&cwd).ok()?;
            let sha = elyra_git::checkpoint::create(&root, &refname, "elyra: before turn")
                .map_err(|err| log::warn!("checkpoint failed: {err:#}"))
                .ok()?;
            let database = (with_database && crate::grove::app_for(&cwd).is_some())
                .then(|| crate::grove::database_of(&cwd))
                .flatten()
                .and_then(|database| {
                    crate::grove::snapshot_database(&database, &snapshots, &note)
                        .map_err(|err| log::warn!("database snapshot failed: {err:#}"))
                        .ok()
                });
            Some((sha, database))
        });
        cx.spawn(async move |this, cx| {
            let checkpoint = job.await;
            let _ = this.update(cx, |this, cx| {
                if let (Some((sha, database)), Some(id)) = (checkpoint, item_id)
                    && let Some(index) = this.items.iter().position(|item| item.id == id)
                {
                    if let Some(database) = database {
                        this.remember_db_snapshot(&sha, database, cx);
                    }
                    if let ItemContent::User { checkpoint, .. } = &mut this.items[index].content {
                        *checkpoint = Some(sha);
                    }
                    this.persist_item(index, cx);
                }
                this.deliver(prompt, cx);
            });
        })
        .detach();
    }

    /// Database snapshots taken with this thread's checkpoints, oldest first:
    /// (checkpoint commit, snapshot reference).
    fn db_snapshots(&self, cx: &App) -> Vec<(String, String)> {
        let Some(app) = self.app.upgrade() else {
            return Vec::new();
        };
        db_snapshots(&app.read(cx).store, self.thread.id)
    }

    /// Keep a checkpoint's database snapshot; drop the oldest beyond the limit.
    fn remember_db_snapshot(&mut self, sha: &str, reference: String, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let mut snapshots = self.db_snapshots(cx);
        snapshots.push((sha.to_string(), reference));
        let excess = snapshots.len().saturating_sub(MAX_DB_SNAPSHOTS);
        let dropped: Vec<String> = snapshots.drain(..excess).map(|(_, r)| r).collect();
        let key = db_snapshots_key(self.thread.id);
        if let Ok(json) = serde_json::to_string(&snapshots)
            && let Err(err) = app.read(cx).store.set_setting(&key, &json)
        {
            log::warn!("saving database snapshots: {err:#}");
        }
        if !dropped.is_empty() {
            cx.background_executor()
                .spawn(async move {
                    for reference in dropped {
                        crate::grove::drop_snapshot(&reference);
                    }
                })
                .detach();
        }
    }

    /// Checkpoints of this thread's turns, oldest first: (label, commit).
    pub fn turn_checkpoints(&self) -> Vec<(String, String)> {
        self.items
            .iter()
            .filter_map(|item| match &item.content {
                ItemContent::User {
                    text,
                    checkpoint: Some(sha),
                } => Some((
                    text.lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(60)
                        .collect::<String>(),
                    sha.clone(),
                )),
                _ => None,
            })
            .enumerate()
            .map(|(index, (text, sha))| (format!("Turn {} · {text}", index + 1), sha))
            .collect()
    }

    /// Put the working tree back to how it was before a message was sent.
    pub fn restore_checkpoint(&mut self, sha: String, cx: &mut Context<Self>) {
        if self.running {
            self.notice("Stop the agent before restoring files.", true, cx);
            return;
        }
        let cwd = self.working_dir();
        let snapshot = self
            .db_snapshots(cx)
            .into_iter()
            .find(|(s, _)| *s == sha)
            .map(|(_, r)| r);
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&cwd)?;
            elyra_git::checkpoint::restore(&root, &sha)?;
            // The database too, when the checkpoint has it.
            let database = snapshot.and_then(|reference| {
                let database = crate::grove::database_of(&cwd)?;
                Some(crate::grove::restore_database(&database, &reference))
            });
            anyhow::Ok(database)
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(None) => this.notice(
                        "Files restored to how they were before that message. The conversation is unchanged.",
                        false,
                        cx,
                    ),
                    Ok(Some(Ok(()))) => this.notice(
                        "Files and database restored to how they were before that message. The conversation is unchanged.",
                        false,
                        cx,
                    ),
                    Ok(Some(Err(err))) => this.notice(
                        format!("Files restored, but not the database: {err:#}"),
                        true,
                        cx,
                    ),
                    Err(err) => this.notice(format!("Could not restore files: {err:#}"), true, cx),
                }
                cx.emit(SessionEvent::TurnCompleted);
            });
        })
        .detach();
    }

    /// Deliver a queued message into the running turn now.
    pub fn steer_queued(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.queued.len() {
            return;
        }
        if !self.running {
            let prompt = self.queued.remove(index);
            self.send(prompt, cx);
            return;
        }
        let Some(provider) = self.provider.as_ref() else {
            return;
        };
        let prompt = self.queued.remove(index);
        match provider.steer(&prompt) {
            Ok(()) => self.append(
                ItemContent::User {
                    text: user_text(&prompt),
                    checkpoint: None,
                },
                cx,
            ),
            Err(err) => {
                self.queued.insert(index, prompt);
                self.notice(format!("Could not steer: {err:#}"), true, cx);
            }
        }
        cx.notify();
    }

    pub fn remove_queued(&mut self, index: usize, cx: &mut Context<Self>) -> Option<Prompt> {
        let prompt = (index < self.queued.len()).then(|| self.queued.remove(index));
        cx.notify();
        prompt
    }

    fn prepare_worktree_then_send(
        &mut self,
        prompt: Prompt,
        item_id: Option<elyra_core::ItemId>,
        cx: &mut Context<Self>,
    ) {
        let short = self.thread.id.simple().to_string()[..8].to_string();
        let branch = format!("elyra/{short}");
        let dest = elyra_core::paths::worktrees_dir()
            .join(sanitize(&self.project.name))
            .join(&short);
        let repo = self.project.path.clone();
        // In an app Grove runs, Grove makes the worktree: with its own copy
        // of the database, migrated, and its own site.
        let grove_site = crate::preferences::Preferences::global(cx)
            .grove_worktrees
            .then(|| crate::grove::site_for(&crate::grove::cached_sites(), &repo))
            .flatten()
            .filter(crate::grove::Site::is_app);
        self.preparing = Some(if grove_site.is_some() {
            "Preparing worktree, database copy and site with Grove…".into()
        } else {
            "Preparing worktree…".into()
        });
        cx.notify();

        let job = {
            let (dest, branch) = (dest.clone(), branch.clone());
            cx.background_executor().spawn(async move {
                if let Some(site) = grove_site {
                    match crate::grove::start_try(&site.name, &branch) {
                        Ok(record) => return Ok((record.path.clone(), Some(record), None)),
                        Err(err) => {
                            let note = format!(
                                "Grove couldn't run this branch ({err:#}), so it has a plain worktree without its own site or database."
                            );
                            elyra_git::create_worktree(&repo, &dest, &branch)?;
                            return Ok((dest, None, Some(note)));
                        }
                    }
                }
                elyra_git::create_worktree(&repo, &dest, &branch).map(|()| (dest, None, None))
            })
        };
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<_> = job.await;
            let _ = this.update(cx, |this, cx| {
                this.preparing = None;
                match result {
                    Ok((path, record, note)) => {
                        this.thread.environment = Environment::Worktree { path, branch };
                        this.save_thread(cx);
                        if let Some(note) = note {
                            this.notice(note, false, cx);
                        }
                        if let Some(record) = record {
                            this.notice(
                                format!(
                                    "Grove runs this worktree at {} with its own copy of the database.",
                                    record.url
                                ),
                                false,
                                cx,
                            );
                            cx.emit(SessionEvent::SiteReady(record.url));
                        }
                        this.checkpoint_then_deliver(prompt, item_id, cx);
                    }
                    Err(err) => {
                        this.running = false;
                        this.notice(format!("Could not create worktree: {err:#}"), true, cx);
                        this.set_status(ThreadStatus::Failed, cx);
                    }
                }
            });
        })
        .detach();
    }

    fn ensure_provider(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        // Only called before a new turn, so a restart never cuts one short.
        if self.needs_restart {
            self.stop_provider();
        }
        if self.provider.is_some() {
            return Ok(());
        }
        let launch = crate::preferences::Preferences::global(cx)
            .launch(self.thread.provider, self.thread.account.as_deref());
        let config = SessionConfig {
            cwd: self.working_dir(),
            model: self.thread.model.clone(),
            effort: self.thread.effort.clone(),
            permission_mode: self.thread.permission_mode,
            resume_session_id: self.thread.provider_session_id.clone(),
            executable: launch.executable,
            args: launch.args,
            env: launch.env,
            // A side chat's first launch branches from its parent's session.
            fork: self.fork_pending,
            append_system_prompt: self.system_prompt(),
            mcp_servers: {
                let mut servers: Vec<elyra_provider::McpServer> =
                    crate::gateway::server_for_thread(self.thread.id, cx)
                        .into_iter()
                        .chain(self.grove_mcp(cx))
                        .collect();
                // The project's .mcp.json, for agents that don't read it
                // themselves (once the user allowed it).
                if self.thread.provider != ProviderKind::Claude
                    && let Some(app) = self.app.upgrade()
                {
                    for server in crate::shared_setup::servers_for(
                        &app.read(cx).store,
                        self.project.id,
                        &self.project.path,
                    ) {
                        if !servers.iter().any(|s| s.name == server.name) {
                            servers.push(server);
                        }
                    }
                }
                servers
            },
        };
        let (session, events) = elyra_provider::start_session(self.thread.provider, config)?;
        self.provider = Some(session);
        self._events = Some(cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |this, cx| this.handle_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        Ok(())
    }

    /// Grove's own MCP server (read-only) for projects Grove runs as an app.
    fn grove_mcp(&self, cx: &App) -> Option<elyra_provider::McpServer> {
        if !crate::preferences::Preferences::global(cx).grove_mcp {
            return None;
        }
        crate::grove::site_for(&crate::grove::cached_sites(), &self.project.path)
            .filter(crate::grove::Site::is_app)?;
        let grove = crate::grove::executable()?;
        Some(elyra_provider::McpServer::stdio(
            "grove",
            grove,
            vec!["mcp".into()],
        ))
    }

    /// Start the provider early so commands and models are known before the
    /// first message. Skipped when a new worktree will change the directory.
    pub fn warm_up(&mut self, cx: &mut Context<Self>) {
        if self.provider.is_some() || self.use_worktree && self.thread.provider_session_id.is_none()
        {
            return;
        }
        if let Err(err) = self.ensure_provider(cx) {
            log::debug!("warm-up failed: {err:#}");
        }
    }

    fn deliver(&mut self, mut prompt: Prompt, cx: &mut Context<Self>) {
        // A fork that couldn't branch the provider session carries the
        // earlier conversation into its first message.
        if let Some(context) = self.thread.fork_context.take() {
            prompt.text = format!(
                "<earlier-conversation>\n{context}\n</earlier-conversation>\n\nContinue from the conversation above.\n\n{}",
                prompt.text
            );
            self.save_thread(cx);
        }
        if let Some(board) = self.other_agents(cx) {
            prompt.text = format!("{board}\n\n{}", prompt.text);
        }
        let result = self
            .ensure_provider(cx)
            .and_then(|()| self.provider.as_ref().unwrap().send(&prompt));
        if let Err(err) = result {
            self.running = false;
            self.stop_provider();
            self.notice(format!("{err:#}"), true, cx);
            self.set_status(ThreadStatus::Failed, cx);
        }
        cx.notify();
    }

    pub fn interrupt(&mut self, cx: &mut Context<Self>) {
        self.stopped = true;
        if let Some(cancel) = &self.check_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some(provider) = &self.provider
            && let Err(err) = provider.interrupt()
        {
            log::warn!("interrupt failed: {err:#}");
        }
        cx.notify();
    }

    pub fn compact(&mut self, cx: &mut Context<Self>) {
        if self.running {
            self.notice(
                "Wait for the current turn to finish before compacting.",
                false,
                cx,
            );
            return;
        }
        let result = self
            .ensure_provider(cx)
            .and_then(|()| self.provider.as_ref().unwrap().compact());
        match result {
            Ok(()) => {
                if self.thread.provider == ProviderKind::Claude {
                    // Claude runs /compact as a turn.
                    self.running = true;
                }
                self.activity = Some("Compacting conversation…".into());
            }
            Err(err) => self.notice(format!("Could not compact: {err:#}"), true, cx),
        }
        cx.notify();
    }

    /// Forget the provider conversation and start fresh in this thread.
    pub fn clear_context(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        self.stop_provider();
        self.thread.provider_session_id = None;
        self.usage = Usage::default();
        self.save_thread(cx);
        self.notice(
            "Context cleared — the next message starts a new conversation.",
            false,
            cx,
        );
    }

    // ---- decisions --------------------------------------------------------

    pub fn respond(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.items.iter().position(|item| {
            matches!(&item.content, ItemContent::Approval { request_id: id, decision: None, .. } if id == request_id)
        }) else {
            return;
        };
        let response = match decision {
            ApprovalDecision::Allowed => PermissionResponse::Allow,
            ApprovalDecision::AllowedForSession => PermissionResponse::AllowForSession,
            ApprovalDecision::Denied => PermissionResponse::Deny {
                message: "The user denied this action.".into(),
            },
        };
        self.respond_with(index, response, decision, cx);
    }

    /// Answer a plan approval (ExitPlanMode) and pick the mode to continue in.
    pub fn approve_plan(
        &mut self,
        request_id: &str,
        continue_in: Option<PermissionMode>,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.items.iter().position(|item| {
            matches!(&item.content, ItemContent::Approval { request_id: id, decision: None, .. } if id == request_id)
        }) else {
            return;
        };
        match continue_in {
            Some(mode) => {
                self.respond_with(
                    index,
                    PermissionResponse::Allow,
                    ApprovalDecision::Allowed,
                    cx,
                );
                self.set_permission_mode(mode, cx);
            }
            None => self.respond_with(
                index,
                PermissionResponse::Deny {
                    message: "The user wants to keep planning. Refine the plan.".into(),
                },
                ApprovalDecision::Denied,
                cx,
            ),
        }
    }

    fn respond_with(
        &mut self,
        index: usize,
        response: PermissionResponse,
        decision: ApprovalDecision,
        cx: &mut Context<Self>,
    ) {
        let ItemContent::Approval { request_id, .. } = &self.items[index].content else {
            return;
        };
        let request_id = request_id.clone();
        match self.provider.as_ref() {
            Some(provider) => {
                if let Err(err) = provider.respond_permission(&request_id, response) {
                    self.notice(format!("Could not send approval: {err:#}"), true, cx);
                    return;
                }
            }
            None => {
                self.notice(
                    "The provider session has ended; this request can no longer be answered.",
                    true,
                    cx,
                );
            }
        }
        if let ItemContent::Approval { decision: slot, .. } = &mut self.items[index].content {
            *slot = Some(decision);
        }
        self.persist_item(index, cx);
        self.after_decision(cx);
    }

    pub fn answer(&mut self, request_id: &str, answer: QuestionAnswer, cx: &mut Context<Self>) {
        let Some(index) = self.items.iter().position(|item| {
            matches!(&item.content, ItemContent::Question { request, answer: None } if request.request_id == request_id)
        }) else {
            return;
        };
        match self.provider.as_ref() {
            Some(provider) => {
                if let Err(err) = provider.answer_question(request_id, answer.clone()) {
                    self.notice(format!("Could not send answer: {err:#}"), true, cx);
                    return;
                }
            }
            None => self.notice(
                "The provider session has ended; this question can no longer be answered.",
                true,
                cx,
            ),
        }
        if let ItemContent::Question { answer: slot, .. } = &mut self.items[index].content {
            *slot = Some(answer);
        }
        self.persist_item(index, cx);
        self.after_decision(cx);
    }

    fn after_decision(&mut self, cx: &mut Context<Self>) {
        if !self.pending_approval() && self.running {
            self.set_status(ThreadStatus::Running, cx);
        }
        cx.notify();
    }

    // ---- options ----------------------------------------------------------

    /// Change provider; only before the conversation has started.
    pub fn set_provider(&mut self, provider: ProviderKind, cx: &mut Context<Self>) {
        if self.has_started() || self.thread.provider == provider {
            return;
        }
        self.stop_provider();
        self.thread.provider = provider;
        self.thread.model = None;
        self.thread.effort = None;
        self.thread.account = None;
        self.commands.clear();
        self.models = self
            .app
            .read_with(cx, |app, _| cached_models(&app.store, provider))
            .unwrap_or_default();
        self.set_setting("default_provider", provider.as_str(), cx);
        self.save_thread(cx);
        cx.notify();
    }

    /// Pick a provider account; only before the first message, since a
    /// session belongs to the account that started it.
    pub fn set_account(&mut self, account: Option<String>, cx: &mut Context<Self>) {
        if self.has_started() || self.thread.account == account {
            return;
        }
        self.stop_provider();
        self.thread.account = account;
        self.save_thread(cx);
        cx.notify();
    }

    /// Apply a starred provider/model/effort preset.
    pub fn apply_preset(
        &mut self,
        preset: &crate::preferences::ModelPreset,
        cx: &mut Context<Self>,
    ) {
        if let Some(kind) = ProviderKind::parse(&preset.provider) {
            if kind != self.thread.provider {
                if self.has_started() {
                    return;
                }
                self.set_provider(kind, cx);
            }
            self.set_model(Some(preset.model.clone()), cx);
            if preset.effort.is_some() {
                self.set_effort(preset.effort.clone(), cx);
            }
        }
    }

    pub fn set_permission_mode(&mut self, mode: PermissionMode, cx: &mut Context<Self>) {
        self.thread.permission_mode = mode;
        if let Some(provider) = &self.provider
            && let Err(err) = provider.set_permission_mode(mode)
        {
            log::warn!("set_permission_mode failed: {err:#}");
        }
        self.set_setting("default_permission_mode", mode.as_str(), cx);
        self.save_thread(cx);
        cx.notify();
    }

    pub fn set_model(&mut self, model: Option<String>, cx: &mut Context<Self>) {
        self.thread.model = model.filter(|m| !m.is_empty());
        if let Some(provider) = &self.provider
            && let Err(err) = provider.set_model(self.thread.model.as_deref())
        {
            log::warn!("set_model failed: {err:#}");
        }
        let key = format!("default_model.{}", self.thread.provider.as_str());
        let value = self.thread.model.clone().unwrap_or_default();
        self.set_setting(&key, &value, cx);
        self.save_thread(cx);
        cx.notify();
    }

    pub fn set_effort(&mut self, effort: Option<String>, cx: &mut Context<Self>) {
        self.thread.effort = effort.filter(|e| !e.is_empty());
        if let Some(provider) = &self.provider {
            match provider.set_effort(self.thread.effort.as_deref()) {
                Ok(true) => {}
                Ok(false) => self.needs_restart = true,
                Err(err) => log::warn!("set_effort failed: {err:#}"),
            }
        }
        let key = format!("default_effort.{}", self.thread.provider.as_str());
        let value = self.thread.effort.clone().unwrap_or_default();
        self.set_setting(&key, &value, cx);
        self.save_thread(cx);
        cx.notify();
    }

    /// Stop the provider before the app quits. Returns whether a turn was
    /// cut off (the caller persists the interrupted status).
    pub fn stop_for_quit(&mut self) -> bool {
        let was_running = self.running;
        if let Some(cancel) = &self.check_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        self.stop_provider();
        if was_running {
            self.running = false;
            self.thread.status = ThreadStatus::Interrupted;
        }
        was_running
    }

    /// Continue a turn that was cut off when the app quit.
    pub fn resume_interrupted(&mut self, cx: &mut Context<Self>) {
        if self.thread.status != ThreadStatus::Interrupted || self.running {
            return;
        }
        self.set_status(ThreadStatus::Idle, cx);
        self.submit(
            Prompt::text(
                "Your previous turn was interrupted because the app quit. First check what was already done (files changed, commands that ran) so nothing is done twice, then continue where you left off.",
            ),
            cx,
        );
    }

    pub fn dismiss_interrupted(&mut self, cx: &mut Context<Self>) {
        if self.thread.status == ThreadStatus::Interrupted {
            self.set_status(ThreadStatus::Idle, cx);
            cx.notify();
        }
    }

    /// Ask the provider for a short title based on the conversation.
    pub fn generate_title(&mut self, cx: &mut Context<Self>) {
        let excerpt: String = self
            .items
            .iter()
            .filter_map(|item| match &item.content {
                ItemContent::User { text, .. } => Some(format!("User: {text}")),
                ItemContent::Assistant {
                    text,
                    parent_tool_use_id: None,
                } => Some(format!("Assistant: {text}")),
                _ => None,
            })
            .take(6)
            .collect::<Vec<_>>()
            .join("\n\n")
            .chars()
            .take(4000)
            .collect();
        if excerpt.is_empty() {
            return;
        }
        let prompt = format!(
            "Write a short title (3-7 words, no quotes, no trailing period) for this coding task conversation. Reply with the title only.\n\n{excerpt}"
        );
        let (kind, cwd) = (self.thread.provider, self.working_dir());
        let job = cx
            .background_executor()
            .spawn(async move { elyra_provider::generate_text(kind, &cwd, &prompt) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(text) => {
                    let title = elyra_provider::clean_title(&text);
                    if !title.is_empty() {
                        this.rename(title, cx);
                    }
                }
                Err(err) => this.notice(format!("Could not generate a title: {err:#}"), true, cx),
            });
        })
        .detach();
    }

    pub fn toggle_pin(&mut self, id: ItemId, cx: &mut Context<Self>) {
        let pinned = &mut self.thread.pinned_items;
        match pinned.iter().position(|item| *item == id) {
            Some(index) => {
                pinned.remove(index);
            }
            None => pinned.push(id),
        }
        self.save_thread(cx);
        cx.notify();
    }

    /// Summarise the conversation so far into `thread.recap`.
    pub fn generate_recap(&mut self, cx: &mut Context<Self>) {
        let mut excerpt: Vec<String> = self
            .items
            .iter()
            .filter_map(|item| match &item.content {
                ItemContent::User { text, .. } => Some(format!("User: {text}")),
                ItemContent::Assistant {
                    text,
                    parent_tool_use_id: None,
                } => Some(format!("Assistant: {text}")),
                _ => None,
            })
            .collect();
        // Keep the most recent part of long conversations.
        let mut total = 0;
        let keep = excerpt
            .iter()
            .rev()
            .take_while(|text| {
                total += text.len();
                total < 16_000
            })
            .count();
        let excerpt = excerpt.split_off(excerpt.len() - keep).join("\n\n");
        if excerpt.is_empty() {
            return;
        }
        self.recapping = true;
        cx.notify();
        let prompt = format!(
            "Summarise this coding conversation for someone picking it up later: the goal, what has been done, decisions made, and what is left. Use at most 8 short bullet points. Reply with the bullets only.\n\n{excerpt}"
        );
        let (kind, cwd) = (self.thread.provider, self.working_dir());
        let job = cx
            .background_executor()
            .spawn(async move { elyra_provider::generate_text(kind, &cwd, &prompt) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.recapping = false;
                match result {
                    Ok(text) => {
                        this.thread.recap = Some(text.trim().to_string());
                        this.save_thread(cx);
                    }
                    Err(err) => this.notice(format!("Could not write a recap: {err:#}"), true, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn set_notes(&mut self, notes: String, cx: &mut Context<Self>) {
        let notes = Some(notes).filter(|n| !n.trim().is_empty());
        if self.thread.notes != notes {
            self.thread.notes = notes;
            self.save_thread(cx);
        }
    }

    /// Work on a Félagi issue from this thread.
    pub fn link_felagi(&mut self, issue: String, cx: &mut Context<Self>) {
        self.thread.felagi_issue = Some(issue);
        self.save_thread(cx);
        cx.notify();
    }

    pub fn rename(&mut self, title: String, cx: &mut Context<Self>) {
        let title = title.trim();
        if title.is_empty() {
            return;
        }
        self.thread.title = title.chars().take(120).collect();
        self.save_thread(cx);
        cx.notify();
    }

    // ---- provider events ----------------------------------------------------

    fn handle_event(&mut self, event: ProviderEvent, cx: &mut Context<Self>) {
        match event {
            ProviderEvent::SessionStarted { session_id, .. } => {
                self.fork_pending = false;
                if self.thread.provider_session_id.as_deref() != Some(session_id.as_str()) {
                    self.thread.provider_session_id = Some(session_id);
                    self.save_thread(cx);
                }
            }
            ProviderEvent::TextDelta(text) => {
                self.activity = None;
                self.streaming_text.push_str(&text);
                cx.notify();
            }
            ProviderEvent::ThinkingDelta(text) => {
                self.streaming_thinking.push_str(&text);
                cx.notify();
            }
            ProviderEvent::AssistantText {
                text,
                parent_tool_use_id,
            } => {
                if parent_tool_use_id.is_none() {
                    self.streaming_text.clear();
                }
                self.append(
                    ItemContent::Assistant {
                        text,
                        parent_tool_use_id,
                    },
                    cx,
                );
            }
            ProviderEvent::Thinking { text } => {
                let text = if text.is_empty() {
                    std::mem::take(&mut self.streaming_thinking)
                } else {
                    self.streaming_thinking.clear();
                    text
                };
                if !text.trim().is_empty() {
                    self.append(ItemContent::Thinking { text }, cx);
                }
            }
            ProviderEvent::ToolUse {
                tool_use_id,
                name,
                input,
                parent_tool_use_id,
            } => {
                if parent_tool_use_id.is_none() {
                    self.streaming_text.clear();
                }
                self.append(
                    ItemContent::ToolUse {
                        tool_use_id,
                        name,
                        input,
                        parent_tool_use_id,
                    },
                    cx,
                );
            }
            ProviderEvent::ToolProgress {
                tool_use_id,
                output,
            } => {
                self.tool_progress.insert(tool_use_id, output);
                cx.notify();
            }
            ProviderEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                self.tool_progress.remove(&tool_use_id);
                if !is_error
                    && let Some(edit) =
                        self.items
                            .iter()
                            .rev()
                            .find_map(|item| match &item.content {
                                ItemContent::ToolUse {
                                    tool_use_id: id,
                                    name,
                                    input,
                                    ..
                                } if *id == tool_use_id => {
                                    crate::intent_board::edited_file(name, input)
                                }
                                _ => None,
                            })
                {
                    self.publish_changes(vec![edit], cx);
                }
                self.append(
                    ItemContent::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    },
                    cx,
                );
            }
            ProviderEvent::PermissionRequest(request) => {
                self.append(
                    ItemContent::Approval {
                        request_id: request.request_id,
                        tool_name: request.tool_name,
                        description: request.description,
                        input: request.input,
                        decision: None,
                    },
                    cx,
                );
                self.thread.last_activity_at = Some(Utc::now());
                self.set_status(ThreadStatus::NeedsApproval, cx);
                self.save_thread(cx);
                cx.emit(SessionEvent::NeedsAttention);
            }
            ProviderEvent::Question(request) => {
                self.append(
                    ItemContent::Question {
                        request,
                        answer: None,
                    },
                    cx,
                );
                self.thread.last_activity_at = Some(Utc::now());
                self.set_status(ThreadStatus::NeedsApproval, cx);
                self.save_thread(cx);
                cx.emit(SessionEvent::NeedsAttention);
            }
            ProviderEvent::Usage(usage) => {
                self.merge_usage(usage);
                cx.notify();
            }
            ProviderEvent::TurnCompleted {
                is_error,
                duration_ms,
                cost_usd,
                message,
            } => self.complete_turn(is_error, duration_ms, cost_usd, message, cx),
            ProviderEvent::Commands(commands) => {
                self.commands = commands;
                cx.notify();
            }
            ProviderEvent::Agents(agents) => {
                self.agents = agents;
                cx.notify();
            }
            ProviderEvent::Models(models) => {
                if let Ok(json) = serde_json::to_string(&models) {
                    let key = format!("models.{}", self.thread.provider.as_str());
                    self.set_setting(&key, &json, cx);
                }
                self.models = models;
                cx.notify();
            }
            ProviderEvent::QueueChanged(queue) => {
                self.provider_queue = queue;
                cx.notify();
            }
            ProviderEvent::Compacting(active) => {
                self.activity = active.then(|| "Compacting conversation…".to_string());
                cx.notify();
            }
            ProviderEvent::Retrying {
                attempt,
                max_attempts,
                error,
            } => {
                let reason = error
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(120)
                    .collect::<String>();
                self.activity = Some(format!("Retrying ({attempt}/{max_attempts}) — {reason}"));
                cx.notify();
            }
            ProviderEvent::RateLimited { resets_at } => {
                let when = resets_at
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                    .map(|t| {
                        t.with_timezone(&chrono::Local)
                            .format(" until %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                self.notice(format!("Usage limit reached{when}."), true, cx);
            }
            ProviderEvent::Notice { text, is_error } => self.notice(text, is_error, cx),
            ProviderEvent::Warning(text) => self.notice(text, false, cx),
            ProviderEvent::Exited { error } => {
                self.provider = None;
                self._events = None;
                self.needs_restart = false;
                let was_running = self.running;
                self.running = false;
                self.activity = None;
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.tool_progress.clear();
                if let Some(error) = error {
                    self.notice(error, true, cx);
                    self.set_status(ThreadStatus::Failed, cx);
                } else if was_running {
                    self.set_status(ThreadStatus::Idle, cx);
                }
                if was_running {
                    cx.emit(SessionEvent::TurnCompleted);
                }
                cx.notify();
            }
        }
    }

    fn merge_usage(&mut self, usage: Usage) {
        if usage.context_tokens.is_some() {
            self.usage.context_tokens = usage.context_tokens;
        }
        if usage.context_window.is_some() {
            self.usage.context_window = usage.context_window;
        }
        if usage.cost_usd.is_some() {
            self.usage.cost_usd = usage.cost_usd;
        }
        if usage.input_tokens + usage.output_tokens > 0 {
            self.usage.input_tokens = usage.input_tokens;
            self.usage.output_tokens = usage.output_tokens;
            self.usage.cache_read_tokens = usage.cache_read_tokens;
            self.usage.cache_write_tokens = usage.cache_write_tokens;
        }
    }

    fn complete_turn(
        &mut self,
        is_error: bool,
        duration_ms: Option<u64>,
        cost_usd: Option<f64>,
        message: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let spent_before = self.spent_usd();
        self.activity = None;
        self.streaming_text.clear();
        self.streaming_thinking.clear();
        self.tool_progress.clear();
        if let Some(message) = message.filter(|_| is_error) {
            self.notice(message, true, cx);
        }
        self.append(
            ItemContent::TurnSummary {
                duration_ms,
                cost_usd,
                is_error,
                context_tokens: self.usage.context_tokens,
                context_window: self.usage.context_window,
            },
            cx,
        );
        self.thread.last_activity_at = Some(Utc::now());
        self.record_line_origins(cx);
        // Done means green: the turn ends when the project's checks and
        // journeys pass.
        let command = self.check_command();
        if !is_error && !self.stopped && (command.is_some() || self.journeys_after_turn(cx)) {
            self.run_checks(command, spent_before, cx);
            return;
        }
        self.finish_turn(is_error, spent_before, cx);
    }

    /// The end of a turn (after its checks): idle, notify, next message.
    fn finish_turn(&mut self, is_error: bool, spent_before: f64, cx: &mut Context<Self>) {
        if let Some((model, effort)) = self.escalated_from.take() {
            self.apply_model_effort(model, effort, cx);
        }
        self.running = false;
        self.activity = None;
        self.set_status(
            if is_error {
                ThreadStatus::Failed
            } else {
                ThreadStatus::Idle
            },
            cx,
        );
        self.save_thread(cx);
        if let Some(provider) = &self.provider {
            let _ = provider.refresh_usage();
        }
        self.check_budget(spent_before, cx);
        // A failed turn keeps them for the next one.
        if !is_error {
            self.replay_requests(cx);
        }
        cx.emit(SessionEvent::TurnCompleted);
        // Deliver the next queued message.
        if !self.queued.is_empty() {
            let prompt = self.queued.remove(0);
            self.send(prompt, cx);
            return;
        }
        self.continue_goal(is_error, cx);
    }

    // ---- checks -----------------------------------------------------------

    /// The project's check command, if it has one.
    pub fn check_command(&self) -> Option<String> {
        self.project
            .check_command
            .as_deref()
            .map(str::trim)
            .filter(|command| !command.is_empty())
            .map(str::to_string)
    }

    /// Run the checks if the turn changed files; the turn stays running
    /// until they are done.
    fn run_checks(&mut self, command: Option<String>, spent_before: f64, cx: &mut Context<Self>) {
        let dir = self.working_dir();
        // The snapshot taken before this turn's message reached the agent.
        let checkpoint = self
            .items
            .iter()
            .rev()
            .find_map(|item| match &item.content {
                ItemContent::User { checkpoint, .. } => Some(checkpoint.clone()),
                _ => None,
            });
        let cancel = Arc::new(AtomicBool::new(false));
        self.check_cancel = Some(cancel.clone());
        if let Some(command) = &command {
            self.activity = Some(format!("Running checks: {command}"));
        }
        cx.notify();
        let run_command = command.clone();
        // None: the turn changed no files; Some(None): no check command.
        let job = cx.background_executor().spawn(async move {
            if !crate::checks::changed_since(&dir, checkpoint.flatten().as_deref()) {
                return None;
            }
            Some(run_command.map(|command| crate::checks::run(&dir, &command, &cancel)))
        });
        cx.spawn(async move |this, cx| {
            let outcome = job.await;
            let _ = this.update(cx, |this, cx| {
                this.checks_done(command, outcome, spent_before, cx)
            });
        })
        .detach();
    }

    fn checks_done(
        &mut self,
        command: Option<String>,
        outcome: Option<Option<crate::checks::Outcome>>,
        spent_before: f64,
        cx: &mut Context<Self>,
    ) {
        self.check_cancel = None;
        // The turn changed no files.
        let Some(outcome) = outcome else {
            self.finish_turn(false, spent_before, cx);
            return;
        };
        // No check command: on to the journeys.
        let (Some(command), Some(outcome)) = (command, outcome) else {
            self.replay_journeys(spent_before, cx);
            return;
        };
        let passed = outcome.passed;
        self.append(
            ItemContent::Check {
                command: command.clone(),
                passed,
                exit_code: outcome.exit_code,
                duration_ms: outcome.duration_ms,
                output: outcome.output.clone(),
            },
            cx,
        );
        if self.stopped {
            self.finish_turn(false, spent_before, cx);
            return;
        }
        if passed {
            self.replay_journeys(spent_before, cx);
            return;
        }
        self.fix_or_hand_over(
            "The checks",
            |attempt, attempts| crate::checks::fix_prompt(&command, &outcome, attempt, attempts),
            spent_before,
            cx,
        );
    }

    /// Something failed after the turn: send it back to the agent while
    /// attempts are left, else hand the thread to the user.
    fn fix_or_hand_over(
        &mut self,
        what: &str,
        prompt: impl FnOnce(u32, u32) -> String,
        spent_before: f64,
        cx: &mut Context<Self>,
    ) {
        let attempts = crate::preferences::Preferences::global(cx).check_fix_attempts;
        let can_fix = self.check_attempts < attempts
            && self.queued.is_empty()
            && self.budget_reached().is_none();
        if can_fix {
            self.check_attempts += 1;
            // A fix already failed: the next one gets a stronger model.
            if self.check_attempts >= 2 {
                self.escalate(cx);
            }
            let prompt = prompt(self.check_attempts, attempts);
            self.activity = None;
            self.send(Prompt::text(prompt), cx);
            return;
        }
        if attempts > 0 && self.check_attempts >= attempts {
            self.notice(
                format!("{what} still fail after {attempts} automatic fixes; over to you."),
                true,
                cx,
            );
        }
        self.finish_turn(true, spent_before, cx);
    }

    /// Remember which lines this turn added, so `why` and `history` (and the
    /// blame in Changes) can trace them back to its message.
    fn record_line_origins(&mut self, cx: &mut Context<Self>) {
        let Some((item, Some(checkpoint))) =
            self.items
                .iter()
                .rev()
                .find_map(|item| match &item.content {
                    ItemContent::User { checkpoint, .. } => Some((item.id, checkpoint.clone())),
                    _ => None,
                })
        else {
            return;
        };
        let (dir, project, thread) = (self.working_dir(), self.project.id, self.thread.id);
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&dir).ok()?;
            let added = elyra_git::checkpoint::added_lines(&root, &checkpoint).ok()?;
            // Every file the turn added to, also from the shell, for the board.
            let mut files: Vec<String> = added
                .iter()
                .map(|(path, _)| root.join(path).display().to_string())
                .collect();
            files.dedup();
            let lines = added
                .into_iter()
                .filter_map(|(path, text)| Some((path, elyra_core::line_hash(&text)?)))
                .collect::<Vec<_>>();
            Some((files, lines))
        });
        cx.spawn(async move |this, cx| {
            let Some((files, lines)) = job.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.publish_changes(
                    files.into_iter().map(|file| (file, Vec::new())).collect(),
                    cx,
                );
            });
            if lines.is_empty() {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if let Some(app) = this.app.upgrade()
                    && let Err(err) = app.read(cx).store.record_line_origins(
                        project,
                        thread,
                        item,
                        Utc::now(),
                        &lines,
                    )
                {
                    log::warn!("recording line origins: {err:#}");
                }
                this.lines_recorded += 1;
                cx.notify();
            });
        })
        .detach();
    }

    /// Put files this thread's agent changed on the intent board — (path as
    /// the tool named it, the text it wrote) — and tell this thread and any
    /// other that changed the same file lately.
    fn publish_changes(&mut self, files: Vec<(String, Vec<String>)>, cx: &mut Context<Self>) {
        let dir = self.working_dir();
        let thread = self.thread.id;
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&dir).ok()?;
            let root = root.canonicalize().unwrap_or(root);
            let repo = elyra_git::common_dir(&dir).ok()?;
            let branch = elyra_git::current_branch(&root);
            let now = Utc::now();
            let intents: Vec<crate::intent_board::Intent> = files
                .into_iter()
                .filter_map(|(path, snippets)| {
                    let file = dir.join(&path);
                    let file = file.canonicalize().unwrap_or(file);
                    let path = file.strip_prefix(&root).ok()?.display().to_string();
                    let symbols = if snippets.is_empty() {
                        Vec::new()
                    } else {
                        let source = std::fs::read_to_string(&file).unwrap_or_default();
                        crate::intent_board::symbols_in(&path, &source, &snippets)
                    };
                    Some(crate::intent_board::Intent {
                        repo: repo.clone(),
                        root: root.clone(),
                        branch: branch.clone(),
                        thread,
                        path,
                        symbols,
                        at: now,
                    })
                })
                .collect();
            Some(intents)
        });
        cx.spawn(async move |this, cx| {
            let Some(intents) = job.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.on_changes(intents, cx));
        })
        .detach();
    }

    fn on_changes(&mut self, intents: Vec<crate::intent_board::Intent>, cx: &mut Context<Self>) {
        use crate::intent_board::{Overlap, label, overlap_notice, related_threads};
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let related = related_threads(app.read(cx), self.thread.id);
        let overlaps: Vec<(crate::intent_board::Intent, Overlap)> = app.update(cx, |app, _| {
            intents
                .into_iter()
                .flat_map(|intent| {
                    app.intents
                        .publish(intent.clone(), |t| related.contains(&t))
                        .into_iter()
                        .map(move |overlap| (intent.clone(), overlap))
                })
                .collect()
        });
        let now = Utc::now();
        for (mine, overlap) in overlaps {
            let (title, agent) = label(app.read(cx), overlap.other.thread);
            let text = overlap_notice(&overlap, &title, &agent, &mine.root, now);
            self.notice(text, false, cx);
            let Some(other) = app.read(cx).existing_session(overlap.other.thread) else {
                continue;
            };
            let viewer = overlap.other.root.clone();
            let mirrored = Overlap {
                other: mine,
                shared: overlap.shared,
                differing: overlap.differing,
            };
            let text = overlap_notice(
                &mirrored,
                &self.thread.title,
                self.thread.provider.label(),
                &viewer,
                now,
            );
            other.update(cx, |other, cx| other.notice(text, false, cx));
        }
    }

    /// What other agents changed in this repository lately, for the prompt.
    fn other_agents(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let dir = self.working_dir();
        if self
            .intent_repo
            .as_ref()
            .is_none_or(|(known, _, _)| *known != dir)
        {
            let repo = elyra_git::common_dir(&dir).ok()?;
            let root = elyra_git::repo_root(&dir).ok()?;
            let root = root.canonicalize().unwrap_or(root);
            self.intent_repo = Some((dir, repo, root));
        }
        let (_, repo, root) = self.intent_repo.as_ref()?;
        let app = self.app.upgrade()?;
        let rows = crate::intent_board::rows_for(
            app.read(cx),
            repo,
            root,
            Some(self.thread.id),
            Utc::now(),
        );
        crate::intent_board::context(&rows)
    }

    /// Switch to the escalation model and the highest effort for the next
    /// fix, remembering what to go back to.
    fn escalate(&mut self, cx: &mut Context<Self>) {
        if self.escalated_from.is_some() {
            return;
        }
        let configured = crate::preferences::Preferences::global(cx)
            .provider(self.thread.provider)
            .escalation_model
            .trim()
            .to_string();
        let model = Some(configured)
            .filter(|m| !m.is_empty())
            .or_else(|| self.thread.model.clone());
        let effort = self
            .capabilities()
            .efforts
            .last()
            .map(|(id, _)| id.to_string())
            .or_else(|| self.thread.effort.clone());
        if model == self.thread.model && effort == self.thread.effort {
            return;
        }
        self.escalated_from = Some((self.thread.model.clone(), self.thread.effort.clone()));
        let mut parts = Vec::new();
        if model != self.thread.model {
            parts.push(format!("model {}", model.as_deref().unwrap_or("default")));
        }
        if effort != self.thread.effort {
            parts.push(format!("effort {}", effort.as_deref().unwrap_or("default")));
        }
        self.notice(
            format!(
                "The first fix didn't do it; the next one gets {} (back afterwards).",
                parts.join(" and ")
            ),
            false,
            cx,
        );
        self.apply_model_effort(model, effort, cx);
    }

    /// Change the thread's model and effort without making them the
    /// provider's defaults.
    fn apply_model_effort(
        &mut self,
        model: Option<String>,
        effort: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.thread.model != model {
            self.thread.model = model;
            if let Some(provider) = &self.provider
                && let Err(err) = provider.set_model(self.thread.model.as_deref())
            {
                log::warn!("set_model failed: {err:#}");
                self.needs_restart = true;
            }
            // Going back to the provider's default needs a fresh start.
            if self.thread.model.is_none() {
                self.needs_restart = true;
            }
        }
        if self.thread.effort != effort {
            self.thread.effort = effort;
            if let Some(provider) = &self.provider {
                match provider.set_effort(self.thread.effort.as_deref()) {
                    Ok(true) => {}
                    Ok(false) => self.needs_restart = true,
                    Err(err) => log::warn!("set_effort failed: {err:#}"),
                }
            }
        }
        self.save_thread(cx);
        cx.notify();
    }

    /// Whether the project replays its journeys after each turn (and has any).
    fn journeys_after_turn(&self, cx: &App) -> bool {
        let Some(app) = self.app.upgrade() else {
            return false;
        };
        crate::journeys::replay_after_turn(&app.read(cx).store, self.project.id)
            && !crate::journeys::load_all(&self.working_dir()).is_empty()
    }

    /// After the checks passed: replay the project's journeys (when it does
    /// that after each turn), then end the turn.
    fn replay_journeys(&mut self, spent_before: f64, cx: &mut Context<Self>) {
        if !self.journeys_after_turn(cx) || !cx.has_global::<crate::browser_tools::BrowserHub>() {
            self.finish_turn(false, spent_before, cx);
            return;
        }
        let journeys: Vec<crate::journeys::Journey> =
            crate::journeys::load_all(&self.working_dir())
                .into_iter()
                .filter_map(|(_, journey)| journey.ok())
                .collect();
        let thread = self.thread.id;
        self.activity = Some("Replaying journeys".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            for journey in journeys {
                let stopped = this.update(cx, |this, _| this.stopped).unwrap_or(true);
                if stopped {
                    break;
                }
                let outcome = crate::browser_tools::replay(thread, &journey, cx).await;
                let failed = this
                    .update(cx, |this, cx| {
                        this.append_journey(&journey, &outcome, cx);
                        if outcome.passed {
                            return false;
                        }
                        let name = journey.name.clone();
                        this.fix_or_hand_over(
                            "The journeys",
                            |attempt, attempts| {
                                crate::journeys::fix_prompt(&name, &outcome, attempt, attempts)
                            },
                            spent_before,
                            cx,
                        );
                        true
                    })
                    .unwrap_or(true);
                if failed {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| this.finish_turn(false, spent_before, cx));
        })
        .detach();
    }

    fn append_journey(
        &mut self,
        journey: &crate::journeys::Journey,
        outcome: &crate::journeys::Outcome,
        cx: &mut Context<Self>,
    ) {
        self.append(
            ItemContent::Journey {
                name: journey.name.clone(),
                passed: outcome.passed,
                steps_run: outcome.steps_run,
                total_steps: journey.steps.len(),
                failure: outcome.failure.clone(),
                duration_ms: outcome.duration_ms,
            },
            cx,
        );
    }

    /// Replay journeys now (from the Context tab): shown, not sent to the agent.
    pub fn run_journeys_now(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        if self.running || self.activity.is_some() {
            return;
        }
        let journeys: Vec<crate::journeys::Journey> =
            crate::journeys::load_all(&self.working_dir())
                .into_iter()
                .filter_map(|(_, journey)| journey.ok())
                .filter(|j| name.as_deref().is_none_or(|name| j.name == name))
                .collect();
        let thread = self.thread.id;
        self.activity = Some("Replaying journeys".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            for journey in journeys {
                let outcome = crate::browser_tools::replay(thread, &journey, cx).await;
                let _ = this.update(cx, |this, cx| this.append_journey(&journey, &outcome, cx));
            }
            let _ = this.update(cx, |this, cx| {
                this.activity = None;
                cx.notify();
            });
        })
        .detach();
    }

    /// Run the checks now, outside a turn (to try the command): the result is
    /// shown but not sent to the agent.
    pub fn run_checks_now(&mut self, cx: &mut Context<Self>) {
        if self.running || self.check_cancel.is_some() {
            return;
        }
        let Some(command) = self.check_command() else {
            return;
        };
        let dir = self.working_dir();
        let cancel = Arc::new(AtomicBool::new(false));
        self.check_cancel = Some(cancel.clone());
        self.activity = Some(format!("Running checks: {command}"));
        cx.notify();
        let run_command = command.clone();
        let mine = cancel.clone();
        let job = cx
            .background_executor()
            .spawn(async move { crate::checks::run(&dir, &run_command, &cancel) });
        cx.spawn(async move |this, cx| {
            let outcome = job.await;
            let _ = this.update(cx, |this, cx| {
                // A turn may have started meanwhile, with checks of its own.
                if this
                    .check_cancel
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &mine))
                {
                    this.check_cancel = None;
                    this.activity = None;
                }
                this.append(
                    ItemContent::Check {
                        command,
                        passed: outcome.passed,
                        exit_code: outcome.exit_code,
                        duration_ms: outcome.duration_ms,
                        output: outcome.output,
                    },
                    cx,
                );
            });
        })
        .detach();
    }

    // ---- proposals -----------------------------------------------------------

    /// Show an automation the agent proposed, for the user to decide on.
    pub fn propose_automation(
        &mut self,
        automation: elyra_core::Automation,
        cx: &mut Context<Self>,
    ) {
        self.append(
            ItemContent::AutomationProposal {
                automation: Box::new(automation),
                outcome: None,
            },
            cx,
        );
        cx.notify();
    }

    /// A pull request the thread reviewed changed (see `review_follow`).
    pub fn add_pr_update(&mut self, card: ItemContent, cx: &mut Context<Self>) {
        self.append(card, cx);
        cx.notify();
    }

    pub fn set_pr_update_handled(&mut self, item: ItemId, cx: &mut Context<Self>) {
        let Some(index) = self.items.iter().position(|i| i.id == item) else {
            return;
        };
        if let ItemContent::PrUpdate { handled, .. } = &mut self.items[index].content {
            *handled = true;
            self.persist_item(index, cx);
            cx.notify();
        }
    }

    /// Show a rule the agent learned, for the user to keep or dismiss.
    pub fn propose_rule(&mut self, rule: String, reason: String, cx: &mut Context<Self>) {
        self.append(
            ItemContent::RuleProposal {
                rule,
                reason,
                outcome: None,
            },
            cx,
        );
        cx.notify();
    }

    /// The rule a proposal item holds.
    pub fn proposed_rule(&self, item: ItemId) -> Option<String> {
        self.items
            .iter()
            .find(|i| i.id == item)
            .and_then(|i| match &i.content {
                ItemContent::RuleProposal { rule, .. } => Some(rule.clone()),
                _ => None,
            })
    }

    pub fn set_rule_outcome(
        &mut self,
        item: ItemId,
        outcome: elyra_core::RuleOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.items.iter().position(|i| i.id == item) else {
            return;
        };
        if let ItemContent::RuleProposal {
            outcome: current, ..
        } = &mut self.items[index].content
        {
            *current = Some(outcome);
            self.persist_item(index, cx);
            cx.notify();
        }
    }

    /// The automation a proposal item holds.
    pub fn proposal(&self, item: ItemId) -> Option<elyra_core::Automation> {
        self.items
            .iter()
            .find(|i| i.id == item)
            .and_then(|i| match &i.content {
                ItemContent::AutomationProposal { automation, .. } => Some((**automation).clone()),
                _ => None,
            })
    }

    pub fn set_proposal_outcome(
        &mut self,
        item: ItemId,
        outcome: elyra_core::ProposalOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.items.iter().position(|i| i.id == item) else {
            return;
        };
        if let ItemContent::AutomationProposal {
            outcome: current, ..
        } = &mut self.items[index].content
        {
            *current = Some(outcome);
            self.persist_item(index, cx);
            cx.notify();
        }
    }

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

    // ---- goals ------------------------------------------------------------

    /// Set (or clear) the thread's goal and start working on it.
    pub fn set_goal(&mut self, goal: Option<String>, cx: &mut Context<Self>) {
        let goal = goal.map(|g| g.trim().to_string()).filter(|g| !g.is_empty());
        self.thread.goal_status = goal.as_ref().map(|_| GoalStatus::Active);
        self.thread.goal = goal;
        self.thread.goal_runs = 0;
        if self.thread.goal.is_some() && self.refuse_at_budget(cx) {
            self.thread.goal_status = Some(GoalStatus::Paused);
        }
        self.save_thread(cx);
        if self.thread.goal_status == Some(GoalStatus::Active)
            && !self.running
            && self.preparing.is_none()
        {
            let prompt = self.goal_prompt(true);
            self.submit(Prompt::text(prompt), cx);
        }
        cx.notify();
    }

    pub fn pause_goal(&mut self, paused: bool, cx: &mut Context<Self>) {
        if self.thread.goal.is_none() || (!paused && self.refuse_at_budget(cx)) {
            return;
        }
        self.thread.goal_status = Some(if paused {
            GoalStatus::Paused
        } else {
            GoalStatus::Active
        });
        if !paused {
            self.thread.goal_runs = 0;
        }
        self.save_thread(cx);
        if !paused && !self.running && self.preparing.is_none() {
            let prompt = self.goal_prompt(false);
            self.submit(Prompt::text(prompt), cx);
        }
        cx.notify();
    }

    fn goal_prompt(&self, first: bool) -> String {
        let goal = self.thread.goal.as_deref().unwrap_or("");
        let lead = if first {
            "Work toward this goal across as many turns as it takes"
        } else {
            "Keep working toward the goal"
        };
        format!(
            "{lead}:\n\n{goal}\n\nTake the next concrete step and verify it. When the goal is fully achieved and verified, end your reply with the line {GOAL_DONE}. If you are blocked and need me, end with {GOAL_BLOCKED} and say what you need."
        )
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

    /// After a turn: continue, finish or stop the goal.
    fn continue_goal(&mut self, is_error: bool, cx: &mut Context<Self>) {
        if self.thread.goal_status != Some(GoalStatus::Active) {
            return;
        }
        let reply = self.last_reply();
        let budget = crate::preferences::Preferences::global(cx)
            .goal_max_turns
            .max(1);
        let (status, note) = if reply.contains(GOAL_DONE) {
            (GoalStatus::Achieved, "Goal achieved.".to_string())
        } else if let Some(budget) = self.budget_reached() {
            (
                GoalStatus::Paused,
                format!(
                    "Goal paused: the thread reached its {} budget.",
                    format_usd(budget)
                ),
            )
        } else if reply.contains(GOAL_BLOCKED) || is_error {
            (
                GoalStatus::Paused,
                "Goal paused: the agent needs you.".to_string(),
            )
        } else if self.thread.goal_runs >= budget {
            (
                GoalStatus::Exhausted,
                format!("Goal paused after {budget} automatic turns."),
            )
        } else {
            self.thread.goal_runs += 1;
            self.save_thread(cx);
            let prompt = self.goal_prompt(false);
            self.send(Prompt::text(prompt), cx);
            return;
        };
        self.thread.goal_status = Some(status);
        self.save_thread(cx);
        self.notice(note, false, cx);
        cx.emit(SessionEvent::NeedsAttention);
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

    // ---- budget -----------------------------------------------------------

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

    pub fn spent_usd(&self) -> f64 {
        self.items
            .iter()
            .filter_map(|item| match item.content {
                ItemContent::TurnSummary { cost_usd, .. } => cost_usd,
                _ => None,
            })
            // An empty f64 sum is -0.0, shown as "$-0.00".
            .fold(0.0, |total, cost| total + cost)
    }

    /// The budget, when the thread has spent all of it.
    pub fn budget_reached(&self) -> Option<f64> {
        self.thread
            .budget_usd
            .filter(|budget| self.spent_usd() >= *budget)
    }

    pub fn set_budget(&mut self, budget: Option<f64>, cx: &mut Context<Self>) {
        self.thread.budget_usd = budget.filter(|b| b.is_finite() && *b > 0.);
        self.save_thread(cx);
        cx.notify();
    }

    /// Say so, and return true, when the budget is used up.
    fn refuse_at_budget(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(budget) = self.budget_reached() else {
            return false;
        };
        self.notice(
            format!(
                "This thread has spent {} of its {} budget. Raise the budget to let the goal continue.",
                format_usd(self.spent_usd()),
                format_usd(budget)
            ),
            false,
            cx,
        );
        true
    }

    /// After a turn: say when the thread passed 80% of its budget, or all of it.
    fn check_budget(&mut self, spent_before: f64, cx: &mut Context<Self>) {
        let Some(budget) = self.thread.budget_usd else {
            return;
        };
        let spent = self.spent_usd();
        let note = if spent_before < budget && spent >= budget {
            format!(
                "Budget reached: {} of {} spent. Goals stop here; your own messages still go.",
                format_usd(spent),
                format_usd(budget)
            )
        } else if spent_before < budget * 0.8 && spent >= budget * 0.8 && spent < budget {
            format!(
                "{} of this thread's {} budget spent.",
                format_usd(spent),
                format_usd(budget)
            )
        } else {
            return;
        };
        self.notice(note, false, cx);
        cx.emit(SessionEvent::NeedsAttention);
    }

    /// Reproduce-first debugging: changes the agent's instructions, so it
    /// takes effect from the next message.
    pub fn set_debug_mode(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.thread.debug_mode == on {
            return;
        }
        self.thread.debug_mode = on;
        if self.provider.is_some() {
            self.needs_restart = true;
        }
        self.save_thread(cx);
        cx.notify();
    }

    fn system_prompt(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(instructions) = self.project.instructions.as_deref() {
            parts.push(instructions.to_string());
        }
        if self.thread.debug_mode {
            parts.push(DEBUG_INSTRUCTIONS.to_string());
        }
        // Claude Code finds skills itself; the others get a list.
        if self.thread.provider != ProviderKind::Claude
            && let Some(skills) =
                crate::shared_setup::skills_prompt(&crate::shared_setup::skills(&self.project.path))
        {
            parts.push(skills);
        }
        (!parts.is_empty()).then(|| parts.join("\n\n"))
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
