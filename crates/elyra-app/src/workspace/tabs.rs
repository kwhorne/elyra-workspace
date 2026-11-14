//! Tabs and navigation: which threads are open and in what order, opening,
//! closing and moving between them, and the split view.

use super::*;

impl Workspace {
    pub(super) fn persist_tabs(&self, cx: &mut Context<Self>) {
        let mut tabs: Vec<ThreadId> = self
            .open_tabs
            .iter()
            .copied()
            .filter(|id| Some(*id) != self.active)
            .collect();
        tabs.extend(self.active);
        let value = tabs
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        self.app
            .update(cx, |app, _| app.set_setting("open_tabs", &value));
    }

    pub(super) fn thread_view(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ThreadView>> {
        if let Some(view) = self.thread_views.get(&id) {
            return Some(view.clone());
        }
        let session = self.app.update(cx, |app, cx| app.session(id, cx))?;
        self._subscriptions.push(cx.subscribe_in(
            &session,
            window,
            move |this, _, event: &SessionEvent, window, cx| {
                match event {
                    // The thread's view redraws itself; nothing here changes.
                    SessionEvent::Streamed => return,
                    SessionEvent::FilesChanged => {
                        if let Some(changes) = this.changes_views.get(&id) {
                            changes.update(cx, |changes, cx| changes.refresh(cx));
                        }
                        if let Some(files) = this.files_views.get(&id) {
                            files.update(cx, |files, cx| files.refresh(window, cx));
                        }
                        return;
                    }
                    SessionEvent::TurnStarted => this.picture_page_before(id, cx),
                    SessionEvent::TurnCompleted => this.picture_page_after(id, cx),
                    SessionEvent::SiteReady(url) => this.open_site(id, url, window, cx),
                    SessionEvent::NeedsAttention | SessionEvent::TestsTested => {}
                }
                if let SessionEvent::TurnCompleted = event {
                    if let Some(changes) = this.changes_views.get(&id) {
                        changes.update(cx, |changes, cx| changes.refresh(cx));
                    }
                    if let Some(pr) = this.pr_views.get(&id) {
                        pr.update(cx, |pr, cx| pr.refresh(cx));
                    }
                    if let Some(files) = this.files_views.get(&id) {
                        files.update(cx, |files, cx| files.refresh(window, cx));
                    }
                    // A side chat's edits show up in its parent's panels.
                    let parent = this.app.read(cx).thread(id).and_then(|t| t.parent_id);
                    if let Some(changes) = parent.and_then(|p| this.changes_views.get(&p)) {
                        changes.update(cx, |changes, cx| changes.refresh(cx));
                    }
                }
                this.notify_session(id, event, window, cx);
                cx.notify();
            },
        ));
        let view = cx.new(|cx| ThreadView::new(session, window, cx));
        if let Some(browser) = self.browser_views.get(&id).cloned() {
            view.update(cx, |view, cx| view.set_browser(browser, window, cx));
        }
        self.thread_views.insert(id, view.clone());
        Some(view)
    }

    pub(crate) fn activate(&mut self, id: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.active
            && previous != id
        {
            self.back.push(previous);
            self.back.truncate(100);
            self.forward.clear();
        }
        self.show_thread(id, window, cx);
    }

    /// Show a thread without touching the back/forward history.
    pub(super) fn show_thread(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Side chats live in the tools panel, not in tabs.
        if let Some(parent) = self.app.read(cx).thread(id).and_then(|t| t.parent_id) {
            if self.app.read(cx).thread(parent).is_some() {
                self.show_thread(parent, window, cx);
                self.show_right_tab(RightTab::SideChat, window, cx);
            }
            return;
        }
        let Some(view) = self.thread_view(id, window, cx) else {
            return;
        };
        self.recent.retain(|t| *t != id);
        self.recent.insert(0, id);
        if self.split == Some(id) {
            self.split = self.active;
        }
        if !self.open_tabs.contains(&id) {
            self.open_tabs.push(id);
        }
        self.active = Some(id);
        self.panel = None;
        self.release_hidden_panels();
        self.current_project = self.app.read(cx).thread(id).map(|t| t.project_id);
        self.app.update(cx, |app, cx| app.mark_read(id, cx));
        view.update(cx, |view, cx| view.focus_composer(window, cx));
        self.sync_right_panel(window, cx);
        self.persist_tabs(cx);
        cx.notify();
    }

    pub(super) fn active_cwd(&self, cx: &App) -> Option<std::path::PathBuf> {
        let id = self.active?;
        let session = self.app.read(cx).existing_session(id)?;
        Some(session.read(cx).working_dir())
    }

    pub(crate) fn close_tab(&mut self, id: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.open_tabs.iter().position(|t| *t == id) else {
            return;
        };
        self.open_tabs.remove(index);
        self.thread_views.remove(&id);
        self.browser_views.remove(&id);
        // Its panels observe the session; they're made again when it opens.
        // (A terminal stays: its shell may be running something.)
        self.changes_views.remove(&id);
        self.context_views.remove(&id);
        self.pr_views.remove(&id);
        self.recent.retain(|t| *t != id);
        self.back.retain(|t| *t != id);
        self.forward.retain(|t| *t != id);
        if self.split == Some(id) {
            self.split = None;
        }
        if self.active == Some(id) {
            self.active = None;
            let next = self
                .open_tabs
                .get(index.min(self.open_tabs.len().saturating_sub(1)))
                .copied();
            if let Some(next) = next {
                self.activate(next, window, cx);
            }
        }
        self.persist_tabs(cx);
        cx.notify();
    }

    pub(crate) fn new_thread_in(
        &mut self,
        project_id: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self
            .app
            .update(cx, |app, cx| app.create_thread(project_id, cx))
        {
            Ok(thread) => self.activate(thread.id, window, cx),
            Err(err) => window.push_notification(Notification::error(format!("{err:#}")), cx),
        }
    }

    pub(crate) fn new_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_new_thread(&NewThread, window, cx);
    }

    pub(crate) fn add_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_add_project(&AddProject, window, cx);
    }

    pub(super) fn mark_active_read(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active
            && window.is_window_active()
        {
            self.app.update(cx, |app, cx| app.mark_read(id, cx));
        }
    }

    pub(super) fn on_new_thread(
        &mut self,
        _: &NewThread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self
            .current_project
            .or_else(|| self.app.read(cx).projects.first().map(|p| p.id))
        {
            Some(project) => self.new_thread_in(project, window, cx),
            None => self.on_add_project(&AddProject, window, cx),
        }
    }

    pub(super) fn on_add_project(
        &mut self,
        _: &AddProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add project".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                match this.app.update(cx, |app, cx| app.add_project(&path, cx)) {
                    Ok(project) => {
                        this.current_project = Some(project.id);
                        this.new_thread_in(project.id, window, cx);
                    }
                    Err(err) => {
                        window.push_notification(Notification::error(format!("{err:#}")), cx)
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn on_close_tab(
        &mut self,
        _: &CloseTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(active) = self.active {
            self.close_tab(active, window, cx);
        }
    }

    pub(super) fn select_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // ⌘9 is always the last tab, as in browsers.
        let index = if index == 8 {
            self.open_tabs.len().saturating_sub(1)
        } else {
            index
        };
        if let Some(id) = self.open_tabs.get(index).copied() {
            self.activate(id, window, cx);
        }
    }

    pub(super) fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.open_tabs.len() as isize;
        if len == 0 {
            return;
        }
        let current = self
            .active
            .and_then(|id| self.open_tabs.iter().position(|t| *t == id))
            .unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.activate(self.open_tabs[next], window, cx);
    }

    pub(super) fn on_next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(1, window, cx);
    }

    pub(super) fn on_previous_tab(
        &mut self,
        _: &PreviousTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_tab(-1, window, cx);
    }

    pub(super) fn on_recent_thread(
        &mut self,
        _: &RecentThread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.app.read(cx);
        let target = self
            .recent
            .iter()
            .copied()
            .find(|id| Some(*id) != self.active && state.thread(*id).is_some());
        if let Some(id) = target {
            self.activate(id, window, cx);
        }
    }

    pub(super) fn on_go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(id) = self.back.pop() {
            if self.app.read(cx).thread(id).is_some() {
                self.forward.extend(self.active);
                self.show_thread(id, window, cx);
                return;
            }
        }
    }

    pub(super) fn on_go_forward(
        &mut self,
        _: &GoForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        while let Some(id) = self.forward.pop() {
            if self.app.read(cx).thread(id).is_some() {
                self.back.extend(self.active);
                self.show_thread(id, window, cx);
                return;
            }
        }
    }

    pub(super) fn on_toggle_split(
        &mut self,
        _: &ToggleSplit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.split.take().is_some() {
            cx.notify();
            return;
        }
        let state = self.app.read(cx);
        let other = self
            .recent
            .iter()
            .chain(self.open_tabs.iter())
            .copied()
            .find(|id| Some(*id) != self.active && state.thread(*id).is_some());
        match other {
            Some(id) => {
                self.thread_view(id, window, cx);
                self.split = Some(id);
            }
            None => window.push_notification(
                Notification::info("Open a second thread to show side by side."),
                cx,
            ),
        }
        cx.notify();
    }

    pub(super) fn swap_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(split), Some(active)) = (self.split, self.active) {
            self.split = Some(active);
            self.activate(split, window, cx);
        }
    }

    // ---- palette --------------------------------------------------------

    /// The project and title of the thread in front, for the window title.
    pub(super) fn front_thread(&self, cx: &App) -> Option<(String, String)> {
        let state = self.app.read(cx);
        let thread = state.thread(self.active?)?;
        let project = state.project(thread.project_id).map(|p| p.name.clone())?;
        Some((project, thread.title.clone()))
    }
}
