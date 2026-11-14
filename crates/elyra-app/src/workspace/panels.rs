//! The sidebar, the tools panel and its tabs, the panels over the
//! conversation (tasks, automations, stats, review, search), About and the theme.

use super::*;

impl Workspace {
    /// Make sure the right panel has views for the active thread.
    pub(crate) fn sync_right_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(id), Some(cwd)) = (self.active, self.active_cwd(cx)) else {
            return;
        };
        if !self.right_open {
            return;
        }
        match self.right_tab {
            RightTab::Changes => {
                if let Some(view) = self.changes_views.get(&id) {
                    view.update(cx, |view, cx| view.set_cwd(cwd, cx));
                } else {
                    let session = self.app.read(cx).existing_session(id);
                    let view = cx.new(|cx| ChangesView::new(cwd, session, window, cx));
                    self._subscriptions.push(cx.subscribe_in(
                        &view,
                        window,
                        move |this, _, event: &crate::changes_view::ChangesEvent, window, cx| {
                            match event {
                                crate::changes_view::ChangesEvent::Comment(text) => {
                                    if let Some(thread) = this.thread_views.get(&id) {
                                        thread.update(cx, |thread, cx| {
                                            thread.append_to_composer(text, window, cx)
                                        });
                                    }
                                }
                                crate::changes_view::ChangesEvent::Asked(chat) => {
                                    this.show_side_chat(*chat, window, cx);
                                }
                            }
                        },
                    ));
                    self.changes_views.insert(id, view);
                }
            }
            RightTab::Terminal => {
                // A terminal keeps the directory it started in; a thread that
                // moved into a worktree gets a fresh shell there.
                self.terminal_panel(id, cwd, window, cx);
            }
            RightTab::Files => {
                if let Some(view) = self.files_views.get(&id) {
                    view.update(cx, |view, cx| view.set_root(cwd, cx));
                } else {
                    let view = cx.new(|cx| FilesView::new(cwd, window, cx));
                    self._subscriptions.push(cx.subscribe_in(
                        &view,
                        window,
                        move |this, _, event: &FilesEvent, window, cx| match event {
                            FilesEvent::Mention(path) => {
                                if let Some(thread) = this.thread_views.get(&id) {
                                    thread.update(cx, |thread, cx| {
                                        thread.append_to_composer(&format!("@{path} "), window, cx)
                                    });
                                }
                            }
                            FilesEvent::Expand => this.open_files_sheet(window, cx),
                        },
                    ));
                    self.files_views.insert(id, view);
                }
            }
            RightTab::Browser => {
                self.browser_view(id, cwd, window, cx);
            }
            RightTab::Context => {
                if !self.context_views.contains_key(&id)
                    && let Some(session) = self.app.read(cx).existing_session(id)
                {
                    let app = self.app.clone();
                    let view = cx
                        .new(|cx| crate::context_view::ContextView::new(app, session, window, cx));
                    self._subscriptions.push(cx.subscribe_in(
                        &view,
                        window,
                        move |this, _, event: &crate::context_view::ContextEvent, window, cx| {
                            match event {
                                crate::context_view::ContextEvent::OpenUrl(url) => {
                                    this.open_in_browser(id, url, window, cx);
                                }
                                crate::context_view::ContextEvent::ShowItem(item) => {
                                    if let Some(view) = this.thread_views.get(&id) {
                                        view.update(cx, |view, cx| view.reveal(*item, cx));
                                    }
                                }
                            }
                        },
                    ));
                    self.context_views.insert(id, view);
                } else if let Some(view) = self.context_views.get(&id) {
                    view.update(cx, |view, cx| view.scan_servers_if_stale(cx));
                }
            }
            RightTab::SideChat => match self.side_chat_of(id, cx) {
                Some(child) => {
                    self.thread_view(child, window, cx);
                }
                None => {
                    self.right_tab = RightTab::Changes;
                    self.sync_right_panel(window, cx);
                }
            },
            RightTab::PullRequest => {
                if let Some(view) = self.pr_views.get(&id) {
                    view.update(cx, |view, cx| view.set_cwd(cwd, cx));
                } else {
                    let session = self.app.read(cx).existing_session(id);
                    let view = cx.new(|cx| crate::pr_view::PrView::new(cwd, session, window, cx));
                    self._subscriptions.push(cx.subscribe_in(
                        &view,
                        window,
                        move |this, _, event: &crate::pr_view::PrEvent, window, cx| match event {
                            crate::pr_view::PrEvent::Prompt(text) => {
                                if let Some(thread) = this.thread_views.get(&id) {
                                    thread.update(cx, |thread, cx| {
                                        thread.append_to_composer(text, window, cx)
                                    });
                                }
                            }
                            crate::pr_view::PrEvent::BranchChanged => {
                                if let Some(changes) = this.changes_views.get(&id) {
                                    changes.update(cx, |changes, cx| changes.refresh(cx));
                                }
                            }
                        },
                    ));
                    self.pr_views.insert(id, view);
                }
            }
        }
    }

    /// The thread's terminal panel, created on first use.
    pub(super) fn terminal_panel(
        &mut self,
        id: ThreadId,
        cwd: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TerminalPanel> {
        if let Some(panel) = self.terminal_views.get(&id) {
            return panel.clone();
        }
        let panel = cx.new(|cx| TerminalPanel::new(cwd, window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &panel,
            window,
            move |this, _, event: &TerminalPanelEvent, window, cx| {
                let TerminalPanelEvent::AddToChat(text) = event;
                if let Some(thread) = this.thread_views.get(&id) {
                    let quoted = format!("```\n{}\n```\n", text.trim_end());
                    thread.update(cx, |thread, cx| {
                        thread.append_to_composer(&quoted, window, cx);
                        thread.focus_composer(window, cx);
                    });
                }
            },
        ));
        self.terminal_views.insert(id, panel.clone());
        panel
    }

    pub(crate) fn toggle_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_panel(Panel::Review, window, cx);
    }

    /// Let go of panels that aren't shown: they observe the app or poll (the
    /// race comparison every few seconds, Félagi issues every 90 seconds) and
    /// are quick to make again.
    pub(super) fn release_hidden_panels(&mut self) {
        if self.panel != Some(Panel::Race) {
            self.race = None;
        }
        if self.panel != Some(Panel::Tasks) {
            self.tasks = None;
        }
        if self.panel != Some(Panel::Automations) {
            self.automations = None;
        }
        if self.panel != Some(Panel::Stats) {
            self.stats = None;
        }
        if self.panel != Some(Panel::Walkthrough) {
            self.walkthrough = None;
        }
    }

    /// Show (or hide) a full-width panel instead of the chat.
    pub(crate) fn toggle_panel(
        &mut self,
        panel: Panel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.panel == Some(panel) {
            self.panel = None;
            self.release_hidden_panels();
            cx.notify();
            return;
        }
        self.panel = Some(panel);
        self.release_hidden_panels();
        let (app, workspace) = (self.app.clone(), cx.weak_entity());
        match panel {
            Panel::Review => match &self.review {
                Some(review) => review.update(cx, |review, cx| review.refresh(cx)),
                None => {
                    self.review = Some(cx.new(|cx| {
                        crate::review_inbox::ReviewInbox::new(app, workspace, window, cx)
                    }))
                }
            },
            Panel::Automations => {
                if self.automations.is_none() {
                    self.automations =
                        Some(cx.new(|cx| {
                            crate::automations::AutomationsView::new(app, workspace, cx)
                        }));
                }
            }
            Panel::Tasks => {
                if self.tasks.is_none() {
                    let project = self.current_project;
                    self.tasks = Some(cx.new(|cx| {
                        crate::tasks::TasksView::new(app, workspace, project, window, cx)
                    }));
                }
            }
            Panel::Stats => match &self.stats {
                Some(stats) => stats.update(cx, |stats, cx| stats.refresh(cx)),
                None => self.stats = Some(cx.new(|cx| crate::stats::StatsView::new(app, cx))),
            },
            Panel::Race => match &self.race {
                Some(race) => race.update(cx, |race, cx| race.refresh(cx)),
                None => {
                    self.race = Some(cx.new(|cx| crate::race::RaceView::new(app, workspace, cx)))
                }
            },
            Panel::Walkthrough => {
                // The active thread's folder, read afresh each time.
                let dir = self.active_cwd(cx);
                let project = dir
                    .as_ref()
                    .and_then(|dir| crate::provenance::project_of(self.app.read(cx), dir));
                self.walkthrough = match (dir, project) {
                    (Some(dir), Some(project)) => Some(cx.new(|cx| {
                        crate::walkthrough::Walkthrough::new(app, workspace, dir, project, cx)
                    })),
                    _ => None,
                };
            }
            Panel::Search => match &self.search {
                // Threads may have changed since: search again.
                Some(search) => search.update(cx, |search, cx| {
                    search.focus(window, cx);
                    search.search(cx);
                }),
                None => {
                    self.search = Some(cx.new(|cx| {
                        crate::thread_search::ThreadSearch::new(app, workspace, window, cx)
                    }))
                }
            },
        }
        cx.notify();
    }

    /// Leave a panel and show a thread.
    pub(crate) fn open_thread_from_panel(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.panel = None;
        self.release_hidden_panels();
        self.activate(id, window, cx);
    }

    pub(super) fn on_show_automations(
        &mut self,
        _: &ShowAutomations,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_panel(Panel::Automations, window, cx);
    }

    pub(super) fn on_show_tasks(
        &mut self,
        _: &ShowTasks,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_panel(Panel::Tasks, window, cx);
    }

    pub(super) fn on_show_stats(
        &mut self,
        _: &ShowStats,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_panel(Panel::Stats, window, cx);
    }

    pub(super) fn on_review_branch(
        &mut self,
        _: &ReviewBranch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.panel == Some(Panel::Walkthrough) {
            self.panel = None;
        }
        self.toggle_panel(Panel::Walkthrough, window, cx);
    }

    pub(super) fn on_search_threads(
        &mut self,
        _: &SearchThreads,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_panel(Panel::Search, window, cx);
    }

    /// Leave a panel and show a thread scrolled to one of its entries.
    pub(crate) fn open_thread_at(
        &mut self,
        id: ThreadId,
        item: elyra_core::ItemId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_thread_from_panel(id, window, cx);
        // After the thread's view has laid out its transcript.
        cx.defer_in(window, move |this, _, cx| {
            if let Some(view) = this.thread_views.get(&id) {
                view.update(cx, |view, cx| view.reveal(item, cx));
            }
        });
    }

    pub(super) fn on_show_review(
        &mut self,
        _: &ShowCodeReview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_review(window, cx);
    }

    pub(super) fn on_manage_worktrees(
        &mut self,
        _: &ManageWorktrees,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::dialogs::worktrees(self.app.clone(), window, cx);
    }

    pub(super) fn on_toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_open = !self.sidebar_open;
        self.save_layout(cx);
        cx.notify();
    }

    pub(super) fn on_toggle_right(
        &mut self,
        _: &ToggleRightPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.right_open = !self.right_open;
        self.save_layout(cx);
        self.sync_right_panel(window, cx);
        cx.notify();
    }

    pub(crate) fn show_right_tab(
        &mut self,
        tab: RightTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.right_open = true;
        self.right_tab = tab;
        self.save_layout(cx);
        self.sync_right_panel(window, cx);
        if tab == RightTab::Terminal
            && let Some(terminal) = self.active.and_then(|id| self.terminal_views.get(&id))
        {
            terminal.update(cx, |terminal, cx| terminal.focus_active(window, cx));
        }
        cx.notify();
    }

    pub(super) fn on_show_terminal(
        &mut self,
        _: &ShowTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.right_open && self.right_tab == RightTab::Terminal {
            self.right_open = false;
            if let Some(view) = self.active.and_then(|id| self.thread_views.get(&id)) {
                view.update(cx, |view, cx| view.focus_composer(window, cx));
            }
            cx.notify();
        } else {
            self.show_right_tab(RightTab::Terminal, window, cx);
        }
    }

    pub(super) fn on_show_changes(
        &mut self,
        _: &ShowChanges,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_right_tab(RightTab::Changes, window, cx);
    }

    pub(super) fn on_focus_composer(
        &mut self,
        _: &FocusComposer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.active.and_then(|id| self.thread_views.get(&id)) {
            view.update(cx, |view, cx| view.focus_composer(window, cx));
        }
    }

    pub(super) fn on_about(&mut self, _: &About, window: &mut Window, cx: &mut Context<Self>) {
        self.about_open = true;
        window.focus(&self.about_focus, cx);
        cx.notify();
    }

    pub fn close_about(&mut self, cx: &mut Context<Self>) {
        self.about_open = false;
        cx.notify();
    }

    pub(super) fn on_toggle_theme(
        &mut self,
        _: &ToggleTheme,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::preferences::toggle_light_dark(cx);
    }

    // ---- navigation -----------------------------------------------------

    pub(crate) fn set_space(&mut self, space: Option<String>, cx: &mut Context<Self>) {
        self.active_space = space;
        let value = self.active_space.clone().unwrap_or_default();
        self.app
            .update(cx, |app, _| app.set_setting("space", &value));
        cx.notify();
    }

    pub(super) fn on_show_context(
        &mut self,
        _: &ShowContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_right_tab(RightTab::Context, window, cx);
    }

    pub(super) fn on_show_shortcuts(
        &mut self,
        _: &ShowShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::shortcuts::show(window, cx);
    }

    pub(super) fn on_toggle_terminal_workspace(
        &mut self,
        _: &ToggleTerminalWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(id), Some(cwd)) = (self.active, self.active_cwd(cx)) else {
            return;
        };
        self.terminal_full = !self.terminal_full;
        if self.terminal_full {
            let terminal = self.terminal_panel(id, cwd, window, cx);
            terminal.update(cx, |terminal, cx| terminal.focus_active(window, cx));
        } else {
            self.on_focus_composer(&FocusComposer, window, cx);
        }
        cx.notify();
    }
}
