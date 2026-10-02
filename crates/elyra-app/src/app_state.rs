use crate::thread_session::ThreadSession;
use anyhow::Result;
use chrono::Utc;
use elyra_core::{
    Environment, PermissionMode, Project, ProjectId, ProviderKind, Store, Thread, ThreadId,
    ThreadStatus,
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
        self.store.delete_thread(id)?;
        self.threads.retain(|t| t.id != id);
        cx.notify();
        if remove_worktree
            && let Some(thread) = thread
            && let Environment::Worktree { path, .. } = &thread.environment
            && let Some(project) = self.project(thread.project_id)
        {
            elyra_git::remove_worktree(&project.path, path)?;
        }
        Ok(())
    }

    pub fn update_project(&mut self, project: Project, cx: &mut Context<Self>) {
        if let Err(err) = self.store.update_project(&project) {
            log::error!("saving project {}: {err:#}", project.id);
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
        let session = cx.new(|_| ThreadSession::new(app, thread, project, items, models));
        self.sessions.insert(id, session.clone());
        Some(session)
    }
}
