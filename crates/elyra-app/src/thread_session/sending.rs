//! Sending a message: the checkpoint before it, the worktree, the provider
//! and what it is told (the system prompt, a warm start), and the queue.

use super::*;

impl ThreadSession {
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

    pub(super) fn send(&mut self, mut prompt: Prompt, cx: &mut Context<Self>) {
        prompt.text = prompt.text.trim().to_string();
        // Testing the tests runs the checks in a temporary checkout: not
        // beside this turn's own (a shared test database would make them
        // flaky). It runs again once this turn's checks pass.
        if let Some(cancel) = &self.mutation_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        if self.items.is_empty() && self.thread.title == "New thread" && !prompt.text.is_empty() {
            self.thread.title = title_from_prompt(&prompt.text);
        }
        self.append(
            ItemContent::User {
                text: user_text(&prompt),
                checkpoint: None,
                attachments: Vec::new(),
            },
            cx,
        );
        let item_id = self.items.last().map(|item| item.id);
        self.set_running(true, cx);
        self.stopped = false;
        self.thrash.reset();
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
    pub(super) fn checkpoint_then_deliver(
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
                    attachments: Vec::new(),
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

    pub(super) fn prepare_worktree_then_send(
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
                let (path, record, note) = 'made: {
                    if let Some(site) = grove_site {
                        match crate::grove::start_try(&site.name, &branch) {
                            Ok(record) => break 'made (record.path.clone(), Some(record), None),
                            Err(err) => {
                                let note = format!(
                                    "Grove couldn't run this branch ({err:#}), so it has a plain worktree without its own site or database."
                                );
                                elyra_git::create_worktree(&repo, &dest, &branch)?;
                                break 'made (dest, None, Some(note));
                            }
                        }
                    }
                    elyra_git::create_worktree(&repo, &dest, &branch)?;
                    (dest, None, None)
                };
                // What Git leaves out: .env, the agents' setup, vendor/.
                let (copied, missing) = crate::worktree_setup::prepare(&repo, &path);
                let notes: Vec<String> = note
                    .into_iter()
                    .chain(crate::worktree_setup::describe(&copied))
                    .chain(missing)
                    .collect();
                anyhow::Ok((path, record, notes))
            })
        };
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<_> = job.await;
            let _ = this.update(cx, |this, cx| {
                this.preparing = None;
                match result {
                    Ok((path, record, notes)) => {
                        this.thread.environment = Environment::Worktree { path, branch };
                        this.save_thread(cx);
                        for note in notes {
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
                        this.set_running(false, cx);
                        this.notice(format!("Could not create worktree: {err:#}"), true, cx);
                        this.set_status(ThreadStatus::Failed, cx);
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn ensure_provider(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
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
            ..SessionConfig::default()
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
    pub(super) fn grove_mcp(&self, cx: &App) -> Option<elyra_provider::McpServer> {
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

    pub(super) fn deliver(&mut self, mut prompt: Prompt, cx: &mut Context<Self>) {
        if self.wants_warm_start(cx) {
            self.warm_start(prompt, cx);
            return;
        }
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
        // What the last turn took out of other instructions, once.
        if let Some(note) = self.rewrite_note.take() {
            prompt.text = format!("{note}\n\n{}", prompt.text);
        }
        let result = self
            .ensure_provider(cx)
            .and_then(|()| self.provider.as_ref().unwrap().send(&prompt));
        if let Err(err) = result {
            self.set_running(false, cx);
            self.stop_provider();
            self.notice(format!("{err:#}"), true, cx);
            self.set_status(ThreadStatus::Failed, cx);
        }
        cx.notify();
    }

    /// A new thread's first message, with the warm start experiment on.
    pub(super) fn wants_warm_start(&self, cx: &App) -> bool {
        !self.warm_started
            && crate::preferences::Preferences::global(cx).warm_start
            && self.thread.parent_id.is_none()
            && self.thread.fork_context.is_none()
            && self
                .items
                .iter()
                .filter(|i| matches!(i.content, ItemContent::User { .. }))
                .count()
                == 1
    }

    /// Look the first message up, then deliver it: with what was found for
    /// half the threads, without for the others (to compare). Threads where
    /// nothing was found aren't part of the experiment.
    pub(super) fn warm_start(&mut self, mut prompt: Prompt, cx: &mut Context<Self>) {
        self.warm_started = true;
        let (dir, project, thread) = (self.project.path.clone(), self.project.id, self.thread.id);
        let message = prompt.text.clone();
        let job = cx
            .background_executor()
            .spawn(async move { crate::warm_start::build(&dir, project, thread, &message) });
        cx.spawn(async move |this, cx| {
            let pack = job.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(pack) = pack {
                    if crate::warm_start::gets_it(thread) {
                        this.notice(pack.notice(), false, cx);
                        prompt.text = format!("{}\n\n{}", pack.text, prompt.text);
                    } else {
                        this.notice(crate::warm_start::WITHOUT, false, cx);
                    }
                }
                this.deliver(prompt, cx);
            });
        })
        .detach();
    }

    pub(super) fn system_prompt(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(instructions) = self.project.instructions.as_deref() {
            parts.push(instructions.to_string());
        }
        // Gemini CLI reads GEMINI.md only; give it the project's AGENTS.md.
        if self.thread.provider == ProviderKind::Gemini
            && let Some(context) = crate::shared_setup::gemini_context(
                &self.working_dir(),
                dirs::home_dir()
                    .map(|home| home.join(".gemini/settings.json"))
                    .as_deref(),
            )
        {
            parts.push(context);
        }
        if self.thread.debug_mode {
            parts.push(DEBUG_INSTRUCTIONS.to_string());
        }
        // The project's invariants, so agents keep them from the start.
        if let Some(invariants) = crate::invariants::prompt(&self.working_dir()) {
            parts.push(invariants);
        }
        // Skills the agent doesn't find itself.
        let skills = crate::shared_setup::skills_for(
            self.thread.provider,
            crate::shared_setup::skills(&self.project.path),
        );
        if let Some(skills) = crate::shared_setup::skills_prompt(&skills) {
            parts.push(skills);
        }
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }
}
