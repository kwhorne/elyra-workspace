use crate::thread_session::ThreadSession;
use anyhow::Result;
use chrono::Utc;
use elyra_core::{
    Environment, ItemContent, PermissionMode, Project, ProjectId, ProviderKind, Store, Thread,
    ThreadId, ThreadStatus,
};
use gpui_kit::{AppContext as _, Context, Entity};
use std::collections::HashMap;
use std::path::Path;

/// Application-wide state: the durable store plus the in-memory project and
/// thread lists the sidebar renders. Live provider sessions are kept here so
/// threads keep running while their tab is not visible.
pub struct AppState {
    pub store: Store,
    pub projects: Vec<Project>,
    pub threads: Vec<Thread>,
    sessions: HashMap<ThreadId, Entity<ThreadSession>>,
}

impl AppState {
    pub fn load() -> Result<Self> {
        let store = Store::open(&elyra_core::paths::database_path())?;
        let projects = store.projects()?;
        let mut threads = store.threads(false)?;
        // Side chats expire: unused ones, and ones whose parent is gone or
        // that have been idle for a week.
        let week_ago = Utc::now() - chrono::Duration::days(7);
        let expired: Vec<ThreadId> = threads
            .iter()
            .filter(|t| {
                t.parent_id.is_some_and(|parent| {
                    !threads.iter().any(|p| p.id == parent)
                        || t.updated_at < week_ago
                        || store
                            .transcript(t.id)
                            .map(|items| items.is_empty())
                            .unwrap_or(false)
                })
            })
            .map(|t| t.id)
            .collect();
        for id in &expired {
            store.delete_thread(*id)?;
        }
        threads.retain(|t| !expired.contains(&t.id));
        // No provider process survives a restart. A turn that was running
        // when the app quit can be resumed from the thread.
        for thread in &mut threads {
            let status = match thread.status {
                ThreadStatus::Running => ThreadStatus::Interrupted,
                ThreadStatus::NeedsApproval => ThreadStatus::Idle,
                other => other,
            };
            if status != thread.status {
                thread.status = status;
                store.update_thread(thread)?;
            }
        }
        Ok(Self {
            store,
            projects,
            threads,
            sessions: HashMap::new(),
        })
    }

    pub fn project(&self, id: ProjectId) -> Option<&Project> {
        self.projects.iter().find(|project| project.id == id)
    }

    pub fn thread(&self, id: ThreadId) -> Option<&Thread> {
        self.threads.iter().find(|thread| thread.id == id)
    }

    pub fn project_threads(&self, id: ProjectId) -> impl Iterator<Item = &Thread> {
        self.threads
            .iter()
            .filter(move |thread| thread.project_id == id)
    }

    pub fn add_project(&mut self, path: &Path, cx: &mut Context<Self>) -> Result<Project> {
        let project = self.store.add_project(path)?;
        if self.project(project.id).is_none() {
            self.projects.push(project.clone());
            self.projects
                .sort_by_key(|project| project.name.to_lowercase());
        }
        cx.notify();
        Ok(project)
    }

    /// Move a project to another folder. Refused while one of its threads is
    /// running, since the provider process works in the old directory.
    pub fn relocate_project(
        &mut self,
        id: ProjectId,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Result<Project> {
        let busy = self.project_threads(id).any(|thread| {
            self.sessions
                .get(&thread.id)
                .is_some_and(|session| session.read(cx).running)
        });
        anyhow::ensure!(
            !busy,
            "Stop the running threads in this project before changing its folder"
        );
        let project = self.store.relocate_project(id, path)?;
        if let Some(existing) = self.projects.iter_mut().find(|p| p.id == id) {
            *existing = project.clone();
        }
        self.projects
            .sort_by_key(|project| project.name.to_lowercase());
        // Idle sessions start their next provider process in the new folder.
        for thread in self.threads.iter().filter(|thread| thread.project_id == id) {
            if let Some(session) = self.sessions.get(&thread.id) {
                let project = project.clone();
                session.update(cx, |session, cx| session.set_project(project, cx));
            }
        }
        cx.notify();
        Ok(project)
    }

    pub fn remove_project(&mut self, id: ProjectId, cx: &mut Context<Self>) -> Result<()> {
        self.store.remove_project(id)?;
        self.projects.retain(|project| project.id != id);
        let removed: Vec<_> = self
            .threads
            .iter()
            .filter(|thread| thread.project_id == id)
            .map(|thread| thread.id)
            .collect();
        self.threads.retain(|thread| thread.project_id != id);
        for thread_id in removed {
            self.sessions.remove(&thread_id);
        }
        cx.notify();
        Ok(())
    }

    pub fn create_thread(
        &mut self,
        project_id: ProjectId,
        cx: &mut Context<Self>,
    ) -> Result<Thread> {
        let provider = self
            .store
            .setting("default_provider")?
            .and_then(|p| ProviderKind::parse(&p))
            .unwrap_or(ProviderKind::Claude);
        // A provider switched off in Settings falls back to the first one on.
        let prefs = crate::preferences::Preferences::global(cx);
        let provider = if prefs.provider_enabled(provider) {
            provider
        } else {
            prefs
                .enabled_providers()
                .into_iter()
                .next()
                .unwrap_or(ProviderKind::Claude)
        };
        let setting = |key: &str| -> Result<Option<String>> {
            Ok(self
                .store
                .setting(&format!("{key}.{}", provider.as_str()))?
                .filter(|v| !v.is_empty()))
        };
        let model = setting("default_model")?;
        let effort = setting("default_effort")?;
        let mode = self
            .store
            .setting("default_permission_mode")?
            .map(|mode| PermissionMode::parse(&mode))
            .unwrap_or_default();
        let mut thread =
            self.store
                .create_thread(project_id, provider, model, mode, Environment::Local)?;
        if effort.is_some() {
            thread.effort = effort;
            self.store.update_thread(&thread)?;
        }
        self.threads.insert(0, thread.clone());
        cx.notify();
        Ok(thread)
    }

    /// A side chat: a child thread that branches from the parent's agent
    /// session (same provider, model and working directory) so questions
    /// can be asked without derailing the main thread.
    pub fn create_side_chat(
        &mut self,
        parent_id: ThreadId,
        cx: &mut Context<Self>,
    ) -> Result<Entity<ThreadSession>> {
        let parent = self
            .thread(parent_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        let mut thread = self.store.create_thread(
            parent.project_id,
            parent.provider,
            parent.model.clone(),
            parent.permission_mode,
            parent.environment.clone(),
        )?;
        thread.title = format!("Side chat · {}", parent.title);
        thread.effort = parent.effort.clone();
        thread.parent_id = Some(parent_id);
        thread.provider_session_id = parent.provider_session_id.clone();
        self.store.update_thread(&thread)?;
        let fork = thread.provider_session_id.is_some();
        let id = thread.id;
        self.threads.insert(0, thread);
        let session = self
            .session(id, cx)
            .ok_or_else(|| anyhow::anyhow!("could not start the side chat"))?;
        session.update(cx, |session, _| session.fork_pending = fork);
        cx.notify();
        Ok(session)
    }

    /// A second opinion: a read-only side chat with another provider in the
    /// same folder, which reviews what the thread's agent changed.
    pub fn create_review_chat(
        &mut self,
        parent_id: ThreadId,
        provider: ProviderKind,
        cx: &mut Context<Self>,
    ) -> Result<Entity<ThreadSession>> {
        let parent = self
            .thread(parent_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        let mut thread = self.store.create_thread(
            parent.project_id,
            provider,
            None,
            elyra_core::PermissionMode::Plan,
            parent.environment.clone(),
        )?;
        thread.title = format!("Second opinion · {}", provider.label());
        thread.parent_id = Some(parent_id);
        self.store.update_thread(&thread)?;
        let id = thread.id;
        self.threads.insert(0, thread);
        let session = self
            .session(id, cx)
            .ok_or_else(|| anyhow::anyhow!("could not start the review"))?;
        cx.notify();
        Ok(session)
    }

    /// Copy a thread into a new one that continues on its own. Providers that
    /// can branch a session do so natively; others get the conversation so
    /// far as context with the first message.
    pub fn fork_thread(&mut self, id: ThreadId, cx: &mut Context<Self>) -> Result<ThreadId> {
        let source = self
            .thread(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        let items = self.store.transcript(id)?;
        let mut thread = self.store.create_thread(
            source.project_id,
            source.provider,
            source.model.clone(),
            source.permission_mode,
            source.environment.clone(),
        )?;
        thread.title = format!("{} (fork)", source.title);
        thread.effort = source.effort.clone();
        thread.account = source.account.clone();
        thread.notes = source.notes.clone();
        thread.recap = source.recap.clone();
        let native =
            elyra_provider::supports_fork(source.provider) && source.provider_session_id.is_some();
        if native {
            thread.provider_session_id = source.provider_session_id.clone();
        } else if !items.is_empty() {
            thread.fork_context = Some(transcript_text(&items, 24_000));
        }
        self.store.update_thread(&thread)?;
        self.store.append_items(
            thread.id,
            items
                .into_iter()
                .filter(|item| {
                    !matches!(
                        item.content,
                        ItemContent::Approval { .. } | ItemContent::Question { .. }
                    )
                })
                .map(|item| (item.content, Some(item.created_at))),
        )?;
        let new_id = thread.id;
        self.threads.insert(0, thread);
        cx.notify();
        Ok(new_id)
    }

    /// Start a thread with another provider in the same project and folder,
    /// with a recap of this one to send as its first message.
    pub fn handoff_thread(
        &mut self,
        id: ThreadId,
        provider: ProviderKind,
        cx: &mut Context<Self>,
    ) -> Result<(ThreadId, String)> {
        let source = self
            .thread(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        let items = self.store.transcript(id)?;
        let mut thread = self.store.create_thread(
            source.project_id,
            provider,
            None,
            source.permission_mode,
            source.environment.clone(),
        )?;
        thread.title = format!("{} → {}", source.title, provider.label());
        thread.notes = source.notes.clone();
        self.store.update_thread(&thread)?;
        let context = match source.recap.as_deref().filter(|r| !r.trim().is_empty()) {
            Some(recap) => format!(
                "{recap}\n\nLatest messages:\n{}",
                transcript_text(&items, 6_000)
            ),
            None => transcript_text(&items, 10_000),
        };
        let draft = format!(
            "I'm continuing a task that was started with {}. Here is where it stands:\n\n<handoff>\n{context}\n</handoff>\n\nPlease pick it up from here: ",
            source.provider.label()
        );
        let new_id = thread.id;
        self.threads.insert(0, thread);
        cx.notify();
        Ok((new_id, draft))
    }

    /// Add an imported provider session as a thread that resumes it.
    /// Returns the existing thread when it was imported before.
    pub fn import_session(
        &mut self,
        provider: ProviderKind,
        session: elyra_provider::claude_history::ImportedSession,
        cx: &mut Context<Self>,
    ) -> Result<ThreadId> {
        let summary = session.summary;
        if let Some(existing) = self.threads.iter().find(|t| {
            t.provider == provider
                && t.provider_session_id.as_deref() == Some(summary.session_id.as_str())
        }) {
            return Ok(existing.id);
        }
        let project = match summary.cwd.as_deref().filter(|cwd| cwd.is_dir()) {
            Some(cwd) => self.add_project(cwd, cx)?,
            None => self.scratch_project(cx)?,
        };
        let mut thread = self.store.create_thread(
            project.id,
            provider,
            None,
            PermissionMode::default(),
            Environment::Local,
        )?;
        thread.title = summary.title.clone();
        thread.provider_session_id = Some(summary.session_id.clone());
        if let Some(updated) = summary.updated {
            thread.updated_at = updated;
            thread.created_at = updated;
        }
        self.store.update_thread(&thread)?;
        self.store.append_items(thread.id, session.items)?;
        let id = thread.id;
        self.threads.push(thread);
        self.threads
            .sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        cx.notify();
        Ok(id)
    }

    /// Persist a changed thread and keep the sidebar list most-recent first.
    pub fn save_thread(&mut self, thread: &Thread, cx: &mut Context<Self>) {
        if let Err(err) = self.store.update_thread(thread) {
            log::error!("saving thread {}: {err:#}", thread.id);
        }
        if let Some(existing) = self.threads.iter_mut().find(|t| t.id == thread.id) {
            *existing = thread.clone();
        }
        self.threads
            .sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        cx.notify();
    }

    pub fn archive_thread(&mut self, id: ThreadId, cx: &mut Context<Self>) {
        let Some(mut thread) = self.thread(id).cloned() else {
            return;
        };
        thread.archived = true;
        thread.updated_at = Utc::now();
        if let Err(err) = self.store.update_thread(&thread) {
            log::error!("archiving thread {id}: {err:#}");
        }
        self.threads.retain(|t| t.id != id);
        self.sessions.remove(&id);
        cx.notify();
    }

    /// Change a thread in the sidebar list, its live session and the store.
    pub fn update_thread(
        &mut self,
        id: ThreadId,
        edit: impl Fn(&mut Thread),
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) else {
            return;
        };
        edit(thread);
        let thread = thread.clone();
        if let Err(err) = self.store.update_thread(&thread) {
            log::error!("saving thread {id}: {err:#}");
        }
        if let Some(session) = self.sessions.get(&id) {
            session.update(cx, |session, cx| {
                edit(&mut session.thread);
                cx.notify();
            });
        }
        cx.notify();
    }

    pub fn mark_read(&mut self, id: ThreadId, cx: &mut Context<Self>) {
        if self.thread(id).is_some_and(Thread::is_unread) {
            self.update_thread(id, |t| t.read_at = Some(Utc::now()), cx);
        }
    }

    pub fn archived_threads(&self) -> Vec<Thread> {
        self.store.archived_threads().unwrap_or_else(|err| {
            log::error!("loading archived threads: {err:#}");
            Vec::new()
        })
    }

    pub fn restore_thread(&mut self, id: ThreadId, cx: &mut Context<Self>) {
        let Some(mut thread) = self.archived_threads().into_iter().find(|t| t.id == id) else {
            return;
        };
        thread.archived = false;
        thread.updated_at = Utc::now();
        if let Err(err) = self.store.update_thread(&thread) {
            log::error!("restoring thread {id}: {err:#}");
            return;
        }
        self.threads.insert(0, thread);
        cx.notify();
    }

    /// Delete a thread and its transcript; optionally remove its managed
    /// worktree. Returns the worktree removal error, if any.
    pub fn delete_thread(
        &mut self,
        id: ThreadId,
        remove_worktree: bool,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let thread = self
            .thread(id)
            .cloned()
            .or_else(|| self.archived_threads().into_iter().find(|t| t.id == id));
        self.sessions.remove(&id);
        crate::thread_session::drop_db_snapshots(&self.store, id);
        self.store.delete_thread(id)?;
        let _ = std::fs::remove_dir_all(elyra_core::paths::snapshots_dir().join(id.to_string()));
        if let Some(thread) = &thread
            && let Some(project) = self.project(thread.project_id)
        {
            let dir = thread.working_dir(project);
            let prefix = format!("refs/elyra/{}", id.simple());
            if let Ok(root) = elyra_git::repo_root(&dir) {
                let _ = elyra_git::checkpoint::delete_refs(&root, &prefix);
            }
        }
        self.threads.retain(|t| t.id != id);
        let side_chats: Vec<ThreadId> = self
            .threads
            .iter()
            .filter(|t| t.parent_id == Some(id))
            .map(|t| t.id)
            .collect();
        for side in side_chats {
            self.sessions.remove(&side);
            self.store.delete_thread(side)?;
            self.threads.retain(|t| t.id != side);
        }
        cx.notify();
        if remove_worktree
            && let Some(thread) = thread
            && let Environment::Worktree { path, .. } = &thread.environment
            && let Some(project) = self.project(thread.project_id)
        {
            // A worktree Grove runs goes with its site and database copy.
            match crate::grove::try_at(path) {
                Some(record) => crate::grove::end_try(&record)?,
                None => elyra_git::remove_worktree(&project.path, path)?,
            }
        }
        Ok(())
    }

    pub fn update_project(&mut self, project: Project, cx: &mut Context<Self>) {
        if let Err(err) = self.store.update_project(&project) {
            log::error!("saving project {}: {err:#}", project.id);
        }
        for session in self.sessions.values() {
            session.update(cx, |session, _| {
                if session.project.id == project.id {
                    // The system prompt is fixed when the agent starts.
                    if session.project.instructions != project.instructions {
                        session.needs_restart = true;
                    }
                    session.project = project.clone();
                }
            });
        }
        if let Some(existing) = self.projects.iter_mut().find(|p| p.id == project.id) {
            *existing = project;
        }
        self.projects
            .sort_by_key(|project| project.name.to_lowercase());
        cx.notify();
    }

    /// The hidden-folder project that holds chats not tied to a project.
    pub fn scratch_project(&mut self, cx: &mut Context<Self>) -> Result<Project> {
        let dir = elyra_core::paths::data_dir().join("chats");
        std::fs::create_dir_all(&dir)?;
        let mut project = self.add_project(&dir, cx)?;
        if project.name != "Chats" || project.icon.is_none() {
            project.name = "Chats".into();
            project.icon = Some("💬".into());
            self.update_project(project.clone(), cx);
        }
        Ok(project)
    }

    /// Threads that need the user: waiting for input or with unseen output.
    pub fn attention_count(&self) -> usize {
        self.threads
            .iter()
            .filter(|t| t.parent_id.is_none())
            .filter(|t| t.status == ThreadStatus::NeedsApproval || t.is_unread())
            .count()
    }

    /// Stop every provider. Threads that were mid-turn are marked
    /// interrupted so they can be resumed after the next launch.
    pub fn prepare_quit(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<ThreadId> = self.sessions.keys().copied().collect();
        for id in ids {
            let Some(session) = self.sessions.get(&id).cloned() else {
                continue;
            };
            let interrupted = session.update(cx, |session, _| session.stop_for_quit());
            if interrupted && let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
                thread.status = ThreadStatus::Interrupted;
                if let Err(err) = self.store.update_thread(thread) {
                    log::error!("saving thread {id}: {err:#}");
                }
            }
        }
    }

    pub fn running_threads(&self, cx: &gpui_kit::App) -> Vec<Thread> {
        self.threads
            .iter()
            .filter(|thread| {
                self.sessions
                    .get(&thread.id)
                    .is_some_and(|s| s.read(cx).running)
            })
            .cloned()
            .collect()
    }

    pub fn set_setting(&mut self, key: &str, value: &str) {
        if let Err(err) = self.store.set_setting(key, value) {
            log::error!("saving setting {key}: {err:#}");
        }
    }

    pub fn existing_session(&self, id: ThreadId) -> Option<Entity<ThreadSession>> {
        self.sessions.get(&id).cloned()
    }

    /// The live session for a thread, created on first access.
    pub fn session(
        &mut self,
        id: ThreadId,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ThreadSession>> {
        if let Some(session) = self.sessions.get(&id) {
            return Some(session.clone());
        }
        let thread = self.thread(id)?.clone();
        let project = self.project(thread.project_id)?.clone();
        let items = self.store.transcript(id).unwrap_or_else(|err| {
            log::error!("loading transcript {id}: {err:#}");
            Vec::new()
        });
        let app = cx.weak_entity();
        let models = crate::thread_session::cached_models(&self.store, thread.provider);
        // A provider session shared with another thread (fork, side chat)
        // must branch on first launch rather than continue the original.
        let shared = thread.provider_session_id.as_ref().is_some_and(|sid| {
            self.threads
                .iter()
                .any(|t| t.id != thread.id && t.provider_session_id.as_ref() == Some(sid))
        });
        let session = cx.new(|_| {
            let mut session = ThreadSession::new(app, thread, project, items, models);
            session.fork_pending = shared;
            session
        });
        self.sessions.insert(id, session.clone());
        Some(session)
    }
}

/// A plain-text rendering of a transcript for another agent: messages and
/// one line per tool call, keeping the most recent part within `budget`.
pub fn transcript_text(items: &[elyra_core::TranscriptItem], budget: usize) -> String {
    let mut parts: Vec<String> = items
        .iter()
        .filter_map(|item| match &item.content {
            ItemContent::User { text, .. } => Some(format!("User: {}", text.trim())),
            ItemContent::Assistant {
                text,
                parent_tool_use_id: None,
            } => Some(format!("Assistant: {}", text.trim())),
            ItemContent::ToolUse {
                name,
                input,
                parent_tool_use_id: None,
                ..
            } => Some(format!(
                "[tool] {name}: {}",
                crate::transcript::tool_summary(name, input)
            )),
            ItemContent::Check {
                command, passed, ..
            } => Some(format!(
                "[checks] {command}: {}",
                if *passed { "passed" } else { "failed" }
            )),
            _ => None,
        })
        .collect();
    let mut total = 0;
    let keep = parts
        .iter()
        .rev()
        .take_while(|part| {
            total += part.len() + 2;
            total <= budget
        })
        .count()
        .max(1)
        .min(parts.len());
    let dropped = parts.len() - keep;
    let mut text = parts.split_off(dropped).join("\n\n");
    if dropped > 0 {
        text = format!("[{dropped} earlier messages omitted]\n\n{text}");
    }
    // A single oversized message is cut, not dropped.
    if text.len() > budget + 200 {
        let cut: String = text
            .chars()
            .rev()
            .take(budget)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        text = format!("…{cut}");
    }
    text
}
