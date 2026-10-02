use crate::app_state::AppState;
use chrono::Utc;
use elyra_core::{
    ApprovalDecision, Environment, ItemContent, ItemId, PermissionMode, Project, ProviderKind,
    QuestionAnswer, Thread, ThreadStatus, TranscriptItem,
};
use elyra_provider::{
    AgentSession, ModelOption, PermissionResponse, Prompt, ProviderEvent, SessionConfig,
    SlashCommand, Usage,
};
use gpui_kit::{Context, EventEmitter, Task, WeakEntity};
use std::collections::HashMap;

pub enum SessionEvent {
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
    pub models: Vec<ModelOption>,
    /// Unsent composer text, kept across tab switches.
    pub draft: String,
    provider: Option<Box<dyn AgentSession>>,
    /// Settings changed that only apply when the provider restarts.
    pub(crate) needs_restart: bool,
    /// The next launch forks `provider_session_id` (side chats).
    pub fork_pending: bool,
    pub recapping: bool,
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
            running: false,
            preparing: None,
            activity: None,
            use_worktree: false,
            queued: Vec::new(),
            provider_queue: Vec::new(),
            tool_progress: HashMap::new(),
            usage,
            commands: Vec::new(),
            models,
            draft: String::new(),
            provider: None,
            needs_restart: false,
            fork_pending: false,
            recapping: false,
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
        self.set_status(ThreadStatus::Running, cx);

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
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&cwd).ok()?;
            elyra_git::checkpoint::create(&root, &refname, "elyra: before turn")
                .map_err(|err| log::warn!("checkpoint failed: {err:#}"))
                .ok()
        });
        cx.spawn(async move |this, cx| {
            let checkpoint = job.await;
            let _ = this.update(cx, |this, cx| {
                if let (Some(sha), Some(id)) = (checkpoint, item_id)
                    && let Some(index) = this.items.iter().position(|item| item.id == id)
                {
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
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&cwd)?;
            elyra_git::checkpoint::restore(&root, &sha)
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.notice(
                        "Files restored to how they were before that message. The conversation is unchanged.",
                        false,
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
        self.preparing = Some("Preparing worktree…".into());
        cx.notify();

        let job = {
            let (dest, branch) = (dest.clone(), branch.clone());
            cx.background_executor()
                .spawn(async move { elyra_git::create_worktree(&repo, &dest, &branch) })
        };
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.preparing = None;
                match result {
                    Ok(()) => {
                        this.thread.environment = Environment::Worktree { path: dest, branch };
                        this.save_thread(cx);
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
        let config = SessionConfig {
            cwd: self.working_dir(),
            model: self.thread.model.clone(),
            effort: self.thread.effort.clone(),
            permission_mode: self.thread.permission_mode,
            resume_session_id: self.thread.provider_session_id.clone(),
            executable: None,
            // A side chat's first launch branches from its parent's session.
            fork: self.fork_pending,
            append_system_prompt: self.project.instructions.clone(),
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

    fn deliver(&mut self, prompt: Prompt, cx: &mut Context<Self>) {
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
        self.commands.clear();
        self.models = self
            .app
            .read_with(cx, |app, _| cached_models(&app.store, provider))
            .unwrap_or_default();
        self.set_setting("default_provider", provider.as_str(), cx);
        self.save_thread(cx);
        cx.notify();
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
                "Your previous turn was interrupted because the app quit. Continue where you left off.",
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
        self.running = false;
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
        cx.emit(SessionEvent::TurnCompleted);
        // Deliver the next queued message.
        if !self.queued.is_empty() {
            let prompt = self.queued.remove(0);
            self.send(prompt, cx);
        }
    }
}

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
