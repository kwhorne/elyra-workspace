use crate::actions::*;
use crate::app_state::AppState;
use crate::changes_view::ChangesView;
use crate::files_view::{FilesEvent, FilesView};
use crate::palette::{Palette, PaletteEvent, PaletteMode, Target};
use crate::terminal_view::{TerminalPanel, TerminalPanelEvent};
use crate::thread_session::SessionEvent;
use crate::thread_view::ThreadView;
use elyra_core::{ProjectId, ThreadId, ThreadStatus};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, TitleBar, WindowExt as _, h_flex, h_resizable,
    resizable_panel, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RightTab {
    Changes,
    Terminal,
    PullRequest,
    Files,
    Context,
    SideChat,
}

impl RightTab {
    const ALL: [RightTab; 6] = [
        RightTab::Changes,
        RightTab::PullRequest,
        RightTab::Files,
        RightTab::Context,
        RightTab::Terminal,
        RightTab::SideChat,
    ];

    fn key(self) -> &'static str {
        match self {
            RightTab::Changes => "changes",
            RightTab::Terminal => "terminal",
            RightTab::PullRequest => "pr",
            RightTab::Files => "files",
            RightTab::Context => "context",
            RightTab::SideChat => "side",
        }
    }
}

pub struct Workspace {
    pub(crate) app: Entity<AppState>,
    focus: FocusHandle,
    pub(crate) open_tabs: Vec<ThreadId>,
    pub(crate) active: Option<ThreadId>,
    /// Project used for ⌘N when no thread is active.
    pub(crate) current_project: Option<ProjectId>,
    pub(crate) thread_views: HashMap<ThreadId, Entity<ThreadView>>,
    pub(crate) changes_views: HashMap<ThreadId, Entity<ChangesView>>,
    pub(crate) terminal_views: HashMap<ThreadId, Entity<TerminalPanel>>,
    pub(crate) pr_views: HashMap<ThreadId, Entity<crate::pr_view::PrView>>,
    pub(crate) files_views: HashMap<ThreadId, Entity<FilesView>>,
    pub(crate) context_views: HashMap<ThreadId, Entity<crate::context_view::ContextView>>,
    palette: Option<Entity<Palette>>,
    /// Second thread shown beside the active one (⌘\).
    split: Option<ThreadId>,
    /// Most recently used threads, newest first.
    recent: Vec<ThreadId>,
    back: Vec<ThreadId>,
    forward: Vec<ThreadId>,
    /// The terminal takes the whole content area (⇧⌘J).
    terminal_full: bool,
    /// Project space shown in the sidebar; None shows all.
    pub(crate) active_space: Option<String>,
    review: Option<Entity<crate::review_inbox::ReviewInbox>>,
    pub(crate) review_open: bool,
    pub(crate) sidebar_open: bool,
    pub(crate) right_open: bool,
    pub(crate) collapsed: HashSet<ProjectId>,
    pub(crate) show_done: HashSet<ProjectId>,
    last_badge: usize,
    right_tab: RightTab,
    about_open: bool,
    about_focus: FocusHandle,
    about_logo: std::sync::Arc<Image>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(app: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe_in(&app, window, |this, _, window, cx| {
                this.app_changed(window, cx);
                cx.notify();
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                this.mark_active_read(window, cx);
            }),
            cx.observe_window_appearance(window, |_, _, cx| {
                crate::preferences::sync_with_system(cx)
            }),
        ];
        let collapsed: HashSet<ProjectId> = app
            .read(cx)
            .store
            .setting("collapsed_projects")
            .ok()
            .flatten()
            .map(|value| value.split(',').filter_map(|id| id.parse().ok()).collect())
            .unwrap_or_default();
        let (open_tabs, active, current_project) = {
            let state = app.read(cx);
            let restore: Vec<ThreadId> = state
                .store
                .setting("open_tabs")
                .ok()
                .flatten()
                .map(|value| {
                    value
                        .split(',')
                        .filter_map(|id| id.parse().ok())
                        .filter(|id| state.thread(*id).is_some())
                        .collect()
                })
                .unwrap_or_default();
            let active = restore.last().copied();
            let project = active
                .and_then(|id| state.thread(id))
                .map(|t| t.project_id)
                .or_else(|| state.projects.first().map(|p| p.id));
            (restore, active, project)
        };
        let layout = app
            .read(cx)
            .store
            .setting("layout")
            .ok()
            .flatten()
            .unwrap_or_default();
        let mut layout = layout.split(',');
        let sidebar_open = layout.next() != Some("0");
        let right_open = layout.next() != Some("0");
        let right_tab = layout
            .next()
            .and_then(|key| RightTab::ALL.into_iter().find(|tab| tab.key() == key))
            .unwrap_or(RightTab::Changes);
        let active_space = app
            .read(cx)
            .store
            .setting("space")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty());
        let mut this = Self {
            app,
            focus: cx.focus_handle(),
            open_tabs,
            active: None,
            current_project,
            thread_views: HashMap::new(),
            changes_views: HashMap::new(),
            terminal_views: HashMap::new(),
            pr_views: HashMap::new(),
            files_views: HashMap::new(),
            context_views: HashMap::new(),
            palette: None,
            split: None,
            recent: Vec::new(),
            back: Vec::new(),
            forward: Vec::new(),
            terminal_full: false,
            active_space,
            review: None,
            review_open: false,
            sidebar_open,
            right_open,
            right_tab,
            collapsed,
            show_done: HashSet::new(),
            last_badge: 0,
            about_open: false,
            about_focus: cx.focus_handle(),
            about_logo: crate::about::logo(),
            _subscriptions: subscriptions,
        };
        if let Some(active) = active {
            this.activate(active, window, cx);
        }
        window.focus(&this.focus, cx);
        this._subscriptions.push(
            cx.observe_window_bounds(window, |this, window, cx| this.save_bounds(window, cx)),
        );
        cx.defer_in(window, |this, window, cx| this.startup(window, cx));
        this
    }

    fn save_bounds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let WindowBounds::Windowed(bounds) = window.window_bounds() {
            let value = format!(
                "{},{},{},{}",
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height)
            );
            self.app
                .update(cx, |app, _| app.set_setting("window_bounds", &value));
        }
    }

    pub(crate) fn save_layout(&self, cx: &mut Context<Self>) {
        let value = format!(
            "{},{},{}",
            self.sidebar_open as u8,
            self.right_open as u8,
            self.right_tab.key()
        );
        self.app
            .update(cx, |app, _| app.set_setting("layout", &value));
    }

    /// First-run welcome and a note about a crash in the previous session.
    fn startup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if crate::onboarding::should_show(&self.app, cx) {
            crate::onboarding::show(cx.weak_entity(), self.app.clone(), window, cx);
        }
        if let Some(crashed) = crate::lifecycle::last_crash_time() {
            let seen: u64 = self
                .app
                .read(cx)
                .store
                .setting("crash_seen")
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if crashed > seen {
                self.app.update(cx, |app, _| {
                    app.set_setting("crash_seen", &crashed.to_string())
                });
                let log = crate::lifecycle::crash_log_path();
                window.push_notification(
                    Notification::warning(format!(
                        "Elyra Workspace quit unexpectedly last time. Details are in {}",
                        log.display()
                    ))
                    .autohide(false)
                    .on_click(move |_, _, cx| cx.reveal_path(&log)),
                    cx,
                );
            }
        }
    }

    fn persist_tabs(&self, cx: &mut Context<Self>) {
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

    fn thread_view(
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
    fn show_thread(&mut self, id: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
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
        self.review_open = false;
        self.current_project = self.app.read(cx).thread(id).map(|t| t.project_id);
        self.app.update(cx, |app, cx| app.mark_read(id, cx));
        view.update(cx, |view, cx| view.focus_composer(window, cx));
        self.sync_right_panel(window, cx);
        self.persist_tabs(cx);
        cx.notify();
    }

    fn active_cwd(&self, cx: &App) -> Option<std::path::PathBuf> {
        let id = self.active?;
        let session = self.app.read(cx).existing_session(id)?;
        Some(session.read(cx).working_dir())
    }

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
                            let crate::changes_view::ChangesEvent::Comment(text) = event;
                            if let Some(thread) = this.thread_views.get(&id) {
                                thread.update(cx, |thread, cx| {
                                    thread.append_to_composer(text, window, cx)
                                });
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
                        move |this, _, event: &FilesEvent, window, cx| {
                            let FilesEvent::Mention(path) = event;
                            if let Some(thread) = this.thread_views.get(&id) {
                                thread.update(cx, |thread, cx| {
                                    thread.append_to_composer(&format!("@{path} "), window, cx)
                                });
                            }
                        },
                    ));
                    self.files_views.insert(id, view);
                }
            }
            RightTab::Context => {
                if !self.context_views.contains_key(&id)
                    && let Some(session) = self.app.read(cx).existing_session(id)
                {
                    let app = self.app.clone();
                    let view = cx
                        .new(|cx| crate::context_view::ContextView::new(app, session, window, cx));
                    self.context_views.insert(id, view);
                } else if let Some(view) = self.context_views.get(&id) {
                    view.update(cx, |view, cx| view.scan_servers(cx));
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
                        move |this, _, event: &crate::pr_view::PrEvent, window, cx| {
                            let crate::pr_view::PrEvent::Prompt(text) = event;
                            if let Some(thread) = this.thread_views.get(&id) {
                                thread.update(cx, |thread, cx| {
                                    thread.append_to_composer(text, window, cx)
                                });
                            }
                        },
                    ));
                    self.pr_views.insert(id, view);
                }
            }
        }
    }

    /// The thread's terminal panel, created on first use.
    fn terminal_panel(
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

    pub(crate) fn close_tab(&mut self, id: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.open_tabs.iter().position(|t| *t == id) else {
            return;
        };
        self.open_tabs.remove(index);
        self.thread_views.remove(&id);
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

    pub(crate) fn toggle_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.review_open = !self.review_open;
        if self.review_open {
            match &self.review {
                Some(review) => review.update(cx, |review, cx| review.refresh(cx)),
                None => {
                    let (app, workspace) = (self.app.clone(), cx.weak_entity());
                    self.review = Some(cx.new(|cx| {
                        crate::review_inbox::ReviewInbox::new(app, workspace, window, cx)
                    }));
                }
            }
        }
        cx.notify();
    }

    fn on_show_review(&mut self, _: &ShowCodeReview, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_review(window, cx);
    }

    fn on_manage_worktrees(
        &mut self,
        _: &ManageWorktrees,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::dialogs::worktrees(self.app.clone(), window, cx);
    }

    /// New thread in a project with a prepared (unsent) message.
    pub(crate) fn start_thread_with_draft(
        &mut self,
        project_id: ProjectId,
        draft: String,
        use_worktree: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_thread_in(project_id, window, cx);
        let Some(id) = self.active else {
            return;
        };
        if let Some(session) = self.app.read(cx).existing_session(id) {
            session.update(cx, |session, cx| {
                session.use_worktree = use_worktree;
                cx.notify();
            });
        }
        if let Some(view) = self.thread_views.get(&id) {
            view.update(cx, |view, cx| view.append_to_composer(&draft, window, cx));
        }
    }

    fn on_commit_and_push(
        &mut self,
        _: &CommitAndPush,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.active else {
            return;
        };
        self.show_right_tab(RightTab::Changes, window, cx);
        if let Some(changes) = self.changes_views.get(&id) {
            changes.update(cx, |changes, cx| changes.commit_and_push(window, cx));
        }
    }

    fn on_new_chat(&mut self, _: &NewChat, window: &mut Window, cx: &mut Context<Self>) {
        self.new_chat(window, cx);
    }

    fn mark_active_read(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active
            && window.is_window_active()
        {
            self.app.update(cx, |app, cx| app.mark_read(id, cx));
        }
    }

    /// Keep the active thread read and the Dock badge current.
    fn app_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mark_active_read(window, cx);
        let count = self.app.read(cx).attention_count();
        if count != self.last_badge {
            self.last_badge = count;
            crate::app_icon::set_badge(count);
        }
    }

    /// Tell the user when a thread they are not looking at finishes or
    /// needs input.
    fn notify_session(
        &mut self,
        id: ThreadId,
        event: &SessionEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let visible = window.is_window_active() && self.active == Some(id);
        if visible {
            return;
        }
        let Some(thread) = self.app.read(cx).thread(id).cloned() else {
            return;
        };
        let (message, kind) = match event {
            SessionEvent::TurnCompleted if thread.status == ThreadStatus::Failed => {
                ("Stopped with an error", NotificationType::Error)
            }
            SessionEvent::TurnCompleted => ("Finished", NotificationType::Success),
            SessionEvent::NeedsAttention => ("Needs your input", NotificationType::Warning),
        };
        let workspace = cx.weak_entity();
        let note = Notification::new()
            .with_type(kind)
            .title(thread.title.clone())
            .message(message)
            .on_click(move |_, window, cx| {
                let _ = workspace.update(cx, |this, cx| this.activate(id, window, cx));
                cx.activate(true);
            });
        let note = if window.is_window_active() {
            note
        } else {
            note.in_app_and_system()
        };
        window.push_notification(note, cx);
    }

    fn on_new_thread(&mut self, _: &NewThread, window: &mut Window, cx: &mut Context<Self>) {
        match self
            .current_project
            .or_else(|| self.app.read(cx).projects.first().map(|p| p.id))
        {
            Some(project) => self.new_thread_in(project, window, cx),
            None => self.on_add_project(&AddProject, window, cx),
        }
    }

    fn on_add_project(&mut self, _: &AddProject, window: &mut Window, cx: &mut Context<Self>) {
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

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(active) = self.active {
            self.close_tab(active, window, cx);
        }
    }

    fn on_toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        self.save_layout(cx);
        cx.notify();
    }

    fn on_toggle_right(
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

    fn on_show_terminal(&mut self, _: &ShowTerminal, window: &mut Window, cx: &mut Context<Self>) {
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

    fn on_show_changes(&mut self, _: &ShowChanges, window: &mut Window, cx: &mut Context<Self>) {
        self.show_right_tab(RightTab::Changes, window, cx);
    }

    fn on_focus_composer(
        &mut self,
        _: &FocusComposer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.active.and_then(|id| self.thread_views.get(&id)) {
            view.update(cx, |view, cx| view.focus_composer(window, cx));
        }
    }

    fn on_about(&mut self, _: &About, window: &mut Window, cx: &mut Context<Self>) {
        self.about_open = true;
        window.focus(&self.about_focus, cx);
        cx.notify();
    }

    pub fn close_about(&mut self, cx: &mut Context<Self>) {
        self.about_open = false;
        cx.notify();
    }

    fn on_toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
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

    fn select_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
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

    fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
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

    fn on_next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(1, window, cx);
    }

    fn on_previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(-1, window, cx);
    }

    fn on_recent_thread(&mut self, _: &RecentThread, window: &mut Window, cx: &mut Context<Self>) {
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

    fn on_go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(id) = self.back.pop() {
            if self.app.read(cx).thread(id).is_some() {
                self.forward.extend(self.active);
                self.show_thread(id, window, cx);
                return;
            }
        }
    }

    fn on_go_forward(&mut self, _: &GoForward, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(id) = self.forward.pop() {
            if self.app.read(cx).thread(id).is_some() {
                self.back.extend(self.active);
                self.show_thread(id, window, cx);
                return;
            }
        }
    }

    fn on_toggle_split(&mut self, _: &ToggleSplit, window: &mut Window, cx: &mut Context<Self>) {
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

    fn swap_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(split), Some(active)) = (self.split, self.active) {
            self.split = Some(active);
            self.activate(split, window, cx);
        }
    }

    // ---- palette --------------------------------------------------------

    fn open_palette(&mut self, mode: PaletteMode, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.active_cwd(cx).or_else(|| {
            let state = self.app.read(cx);
            self.current_project
                .and_then(|id| state.project(id))
                .map(|p| p.path.clone())
        });
        let app = self.app.clone();
        let palette = cx.new(|cx| Palette::new(mode, app, root, window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &palette,
            window,
            |this, _, event: &PaletteEvent, window, cx| {
                this.palette = None;
                if let PaletteEvent::Pick(target) = event {
                    this.pick(target.clone(), window, cx);
                } else {
                    this.on_focus_composer(&FocusComposer, window, cx);
                }
                cx.notify();
            },
        ));
        self.palette = Some(palette);
        cx.notify();
    }

    fn pick(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            Target::Action(action) => {
                window.focus(&self.focus, cx);
                window.dispatch_action(action(), cx);
            }
            Target::Thread(id) => self.activate(id, window, cx),
            Target::Project(id) => self.new_thread_in(id, window, cx),
            Target::Theme(name) => crate::preferences::update(cx, |p| p.theme = name),
            Target::File { path, line } => self.open_file(&path, line, window, cx),
        }
    }

    /// Show a file of the active thread's working directory in the Files tab.
    pub(crate) fn open_file(
        &mut self,
        path: &str,
        line: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_right_tab(RightTab::Files, window, cx);
        if let Some(files) = self.active.and_then(|id| self.files_views.get(&id)) {
            files.update(cx, |files, cx| files.open_file(path, line, window, cx));
        }
    }

    fn on_command_palette(
        &mut self,
        _: &CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteMode::Commands, window, cx);
    }

    fn on_find_file(&mut self, _: &FindFile, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(PaletteMode::Files, window, cx);
    }

    fn on_search_in_files(
        &mut self,
        _: &SearchInFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteMode::Search, window, cx);
    }

    // ---- panels ---------------------------------------------------------

    fn on_show_files(&mut self, _: &ShowFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.show_right_tab(RightTab::Files, window, cx);
    }

    fn on_show_context(&mut self, _: &ShowContext, window: &mut Window, cx: &mut Context<Self>) {
        self.show_right_tab(RightTab::Context, window, cx);
    }

    fn on_show_shortcuts(
        &mut self,
        _: &ShowShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::shortcuts::show(window, cx);
    }

    fn on_toggle_terminal_workspace(
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

    /// Open the working directory, or the file open in the Files tab, in the
    /// preferred external editor.
    pub(crate) fn open_in_editor(
        &mut self,
        editor: Option<crate::editors::EditorApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cwd) = self.active_cwd(cx).or_else(|| {
            let state = self.app.read(cx);
            self.current_project
                .and_then(|id| state.project(id))
                .map(|p| p.path.clone())
        }) else {
            return;
        };
        let editor = editor.or_else(|| crate::preferences::Preferences::global(cx).editor());
        let Some(editor) = editor else {
            window.push_notification(
                Notification::warning("No external editor found. Pick one in Settings → General."),
                cx,
            );
            return;
        };
        let file = (self.right_open && self.right_tab == RightTab::Files)
            .then(|| self.active.and_then(|id| self.files_views.get(&id)))
            .flatten()
            .and_then(|files| files.read(cx).open_path());
        let target = file.unwrap_or(cwd);
        if let Err(err) = crate::editors::open(editor, &target, None) {
            window.push_notification(
                Notification::error(format!("Couldn't open {}: {err:#}", editor.name)),
                cx,
            );
        }
    }

    fn on_open_in_editor(&mut self, _: &OpenInEditor, window: &mut Window, cx: &mut Context<Self>) {
        self.open_in_editor(None, window, cx);
    }

    // ---- fork, handoff, import -------------------------------------------

    pub(crate) fn fork_thread(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.app.update(cx, |app, cx| app.fork_thread(id, cx)) {
            Ok(fork) => self.activate(fork, window, cx),
            Err(err) => window.push_notification(Notification::error(format!("{err:#}")), cx),
        }
    }

    /// Continue a thread with another provider: a new thread with a recap
    /// ready to send.
    pub(crate) fn handoff_thread(
        &mut self,
        id: ThreadId,
        provider: elyra_core::ProviderKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self
            .app
            .update(cx, |app, cx| app.handoff_thread(id, provider, cx))
        {
            Ok((new_id, draft)) => {
                self.activate(new_id, window, cx);
                if let Some(view) = self.thread_views.get(&new_id) {
                    view.update(cx, |view, cx| view.append_to_composer(&draft, window, cx));
                }
            }
            Err(err) => window.push_notification(Notification::error(format!("{err:#}")), cx),
        }
    }

    fn on_import_thread(&mut self, _: &ImportThreads, window: &mut Window, cx: &mut Context<Self>) {
        crate::dialogs::import_claude_sessions(cx.weak_entity(), self.app.clone(), window, cx);
    }

    fn on_fork_thread(&mut self, _: &ForkThread, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active {
            self.fork_thread(id, window, cx);
        }
    }

    // ---- side chats -----------------------------------------------------

    pub(crate) fn side_chat_of(&self, parent: ThreadId, cx: &App) -> Option<ThreadId> {
        self.app
            .read(cx)
            .threads
            .iter()
            .filter(|t| t.parent_id == Some(parent))
            .max_by_key(|t| t.created_at)
            .map(|t| t.id)
    }

    fn on_new_side_chat(&mut self, _: &NewSideChat, window: &mut Window, cx: &mut Context<Self>) {
        let Some(parent) = self.active else {
            return;
        };
        match self
            .app
            .update(cx, |app, cx| app.create_side_chat(parent, cx))
        {
            Ok(session) => {
                let id = session.read(cx).thread.id;
                if let Some(view) = self.thread_view(id, window, cx) {
                    self.show_right_tab(RightTab::SideChat, window, cx);
                    view.update(cx, |view, cx| view.focus_composer(window, cx));
                }
            }
            Err(err) => window.push_notification(Notification::error(format!("{err:#}")), cx),
        }
    }

    /// Turn a side chat into a regular thread with its own tab.
    fn promote_side_chat(&mut self, id: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| {
            app.update_thread(id, |t| t.parent_id = None, cx)
        });
        self.activate(id, window, cx);
    }

    fn close_side_chat(&mut self, id: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        self.thread_views.remove(&id);
        if let Err(err) = self
            .app
            .update(cx, |app, cx| app.delete_thread(id, false, cx))
        {
            window.push_notification(Notification::error(format!("{err:#}")), cx);
        }
        cx.notify();
    }

    // ---- rendering ------------------------------------------------------

    fn render_title_bar(&self, cx: &Context<Self>) -> AnyElement {
        let title: SharedString = self
            .active
            .and_then(|id| self.app.read(cx).thread(id))
            .map(|thread| thread.title.clone().into())
            .unwrap_or_else(|| "Elyra Workspace".into());
        TitleBar::new()
            .child(
                h_flex()
                    .w_full()
                    .pr_2()
                    .gap_1()
                    .child(
                        Button::new("toggle-sidebar")
                            .ghost()
                            .xsmall()
                            .icon(IconName::PanelLeft)
                            .tooltip("Toggle sidebar (⌘B)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_toggle_sidebar(&ToggleSidebar, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_center()
                            .text_color(cx.theme().muted_foreground)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(title),
                    )
                    .when(
                        self.active.is_some() || self.current_project.is_some(),
                        |this| this.child(self.render_open_in(cx)),
                    )
                    .child(
                        Button::new("open-settings")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Settings)
                            .tooltip("Settings (⌘,)")
                            .on_click(|_, _, cx| crate::settings_window::open(cx)),
                    )
                    .child(
                        Button::new("toggle-theme")
                            .ghost()
                            .xsmall()
                            .icon(if cx.theme().is_dark() {
                                IconName::Sun
                            } else {
                                IconName::Moon
                            })
                            .tooltip("Toggle theme (⇧⌘T)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_toggle_theme(&ToggleTheme, window, cx)
                            })),
                    )
                    .child(
                        Button::new("toggle-right")
                            .ghost()
                            .xsmall()
                            .icon(IconName::PanelRight)
                            .tooltip("Toggle tools panel (⌥⌘B)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_toggle_right(&ToggleRightPanel, window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_open_in(&self, cx: &Context<Self>) -> AnyElement {
        let preferred = crate::preferences::Preferences::global(cx).editor();
        let label = preferred.map(|e| e.name).unwrap_or("Open");
        let workspace = cx.weak_entity();
        Button::new("open-in-editor")
            .ghost()
            .xsmall()
            .icon(IconName::ExternalLink)
            .label(label)
            .tooltip("Open in external editor (⌘O)")
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu;
                for editor in crate::editors::installed() {
                    let workspace = workspace.clone();
                    menu = menu.item(PopupMenuItem::new(editor.name).on_click(
                        move |_, window, cx| {
                            let _ = workspace.update(cx, |this, cx| {
                                this.open_in_editor(Some(editor), window, cx)
                            });
                        },
                    ));
                }
                menu
            })
            .into_any_element()
    }

    fn render_tabs(&self, cx: &Context<Self>) -> AnyElement {
        let state = self.app.read(cx);
        let selected = self
            .active
            .and_then(|id| self.open_tabs.iter().position(|t| *t == id))
            .unwrap_or(0);
        let tabs: Vec<Tab> = self
            .open_tabs
            .iter()
            .filter_map(|id| state.thread(*id).map(|t| (*id, t.title.clone())))
            .map(|(id, title)| {
                let short: String = title.chars().take(28).collect();
                Tab::new().label(short).suffix(
                    Button::new(SharedString::from(format!("close-{id}")))
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_tab(id, window, cx)
                        })),
                )
            })
            .collect();
        let ids = self.open_tabs.clone();
        TabBar::new("thread-tabs")
            .underline()
            .small()
            .selected_index(selected)
            .children(tabs)
            .on_click(cx.listener(move |this, index: &usize, window, cx| {
                if let Some(id) = ids.get(*index).copied() {
                    this.activate(id, window, cx);
                }
            }))
            .into_any_element()
    }

    fn render_center(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let view = self.active.and_then(|id| self.thread_view(id, window, cx));
        let Some(view) = view else {
            let has_projects = !self.app.read(cx).projects.is_empty();
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    Icon::new(IconName::Sparkles)
                        .large()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Elyra Workspace"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("A focused workspace for coding agents."),
                )
                .child(if has_projects {
                    Button::new("empty-new-thread")
                        .primary()
                        .icon(IconName::SquarePen)
                        .label("New thread  ⌘N")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.on_new_thread(&NewThread, window, cx)
                        }))
                } else {
                    Button::new("empty-add-project")
                        .primary()
                        .icon(IconName::FolderPlus)
                        .label("Add project  ⇧⌘O")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.on_add_project(&AddProject, window, cx)
                        }))
                })
                .into_any_element();
        };
        let main = v_flex()
            .size_full()
            .child(self.render_tabs(cx))
            .child(div().flex_1().min_h_0().child(view));
        let split = self
            .split
            .filter(|id| Some(*id) != self.active)
            .and_then(|id| Some((id, self.thread_view(id, window, cx)?)));
        let Some((split_id, split_view)) = split else {
            return main.into_any_element();
        };
        let title = self
            .app
            .read(cx)
            .thread(split_id)
            .map(|t| t.title.clone())
            .unwrap_or_default();
        let pane = v_flex()
            .size_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .h(px(33.))
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_sm()
                            .child(title),
                    )
                    .child(
                        Button::new("split-swap")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ArrowLeftRight)
                            .tooltip("Make this the main thread")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.swap_split(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("split-close")
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .tooltip("Close split (⌘\\)")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.split = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().flex_1().min_h_0().child(split_view));
        h_resizable("chat-split")
            .child(resizable_panel().child(main))
            .child(resizable_panel().child(pane))
            .into_any_element()
    }

    fn render_right(&self, cx: &Context<Self>) -> AnyElement {
        let id = self.active;
        let content: AnyElement = match (self.right_tab, id) {
            (RightTab::Changes, Some(id)) => self
                .changes_views
                .get(&id)
                .map(|v| v.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            (RightTab::Terminal, Some(id)) => self
                .terminal_views
                .get(&id)
                .map(|v| v.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            (RightTab::PullRequest, Some(id)) => self
                .pr_views
                .get(&id)
                .map(|v| v.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            (RightTab::Files, Some(id)) => self
                .files_views
                .get(&id)
                .map(|v| v.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            (RightTab::Context, Some(id)) => self
                .context_views
                .get(&id)
                .map(|v| v.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            (RightTab::SideChat, Some(id)) => self
                .side_chat_of(id, cx)
                .and_then(|side| self.thread_views.get(&side))
                .map(|v| v.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            _ => div()
                .p_4()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Open a thread to see its changes and terminal.")
                .into_any_element(),
        };
        let side_chat = id.and_then(|id| self.side_chat_of(id, cx));
        let tabs: Vec<RightTab> = RightTab::ALL
            .into_iter()
            .filter(|tab| *tab != RightTab::SideChat || side_chat.is_some())
            .collect();
        let index = tabs
            .iter()
            .position(|tab| *tab == self.right_tab)
            .unwrap_or(0);
        let pr_label: SharedString = id
            .and_then(|id| self.pr_views.get(&id))
            .and_then(|view| view.read(cx).pr_number())
            .map(|number| format!("PR #{number}"))
            .unwrap_or_else(|| "PR".into())
            .into();
        let tab_items = tabs.iter().map(|tab| {
            let (label, icon): (SharedString, IconName) = match tab {
                RightTab::Changes => ("Changes".into(), IconName::GitCompareArrows),
                RightTab::PullRequest => (pr_label.clone(), IconName::GitPullRequest),
                RightTab::Files => ("Files".into(), IconName::FolderOpen),
                RightTab::Context => ("Context".into(), IconName::NotebookPen),
                RightTab::Terminal => ("Terminal".into(), IconName::SquareTerminal),
                RightTab::SideChat => ("Side chat".into(), IconName::MessagesSquare),
            };
            Tab::new().label(label).prefix(Icon::new(icon).xsmall())
        });
        let side_bar = (self.right_tab == RightTab::SideChat)
            .then_some(side_chat)
            .flatten()
            .map(|side| {
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Branches from this thread. Unused side chats expire."),
                    )
                    .child(
                        Button::new("side-new")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .tooltip("New side chat (⌥⌘S)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_new_side_chat(&NewSideChat, window, cx)
                            })),
                    )
                    .child(
                        Button::new("side-promote")
                            .ghost()
                            .xsmall()
                            .icon(IconName::SquareArrowOutUpRight)
                            .tooltip("Open as a thread")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.promote_side_chat(side, window, cx)
                            })),
                    )
                    .child(
                        Button::new("side-close")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Trash)
                            .tooltip("Discard side chat")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_side_chat(side, window, cx)
                            })),
                    )
            });
        v_flex()
            .size_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                TabBar::new("right-tabs")
                    .underline()
                    .small()
                    .selected_index(index)
                    .children(tab_items)
                    .on_click(cx.listener(move |this, index: &usize, window, cx| {
                        let tab = tabs.get(*index).copied().unwrap_or(RightTab::Changes);
                        this.show_right_tab(tab, window, cx);
                    })),
            )
            .children(side_bar)
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

pub(crate) fn format_age(age: chrono::Duration) -> String {
    let minutes = age.num_minutes();
    if minutes < 1 {
        "now".into()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else if minutes < 60 * 24 {
        format!("{}h", minutes / 60)
    } else {
        format!("{}d", minutes / (60 * 24))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let full_terminal = self
            .terminal_full
            .then(|| self.active.and_then(|id| self.terminal_views.get(&id)))
            .flatten()
            .cloned();
        if full_terminal.is_none() {
            self.terminal_full = false;
        }
        let center = match (&self.review, self.review_open, full_terminal) {
            (Some(review), true, _) => review.clone().into_any_element(),
            (_, _, Some(terminal)) => terminal.into_any_element(),
            _ => self.render_center(window, cx),
        };
        let title_bar = self.render_title_bar(cx);
        let sidebar = self.sidebar_open.then(|| self.render_sidebar(cx));
        let right =
            (self.right_open && self.active.is_some() && !self.review_open && !self.terminal_full)
                .then(|| self.render_right(cx));

        let main: AnyElement = match right {
            Some(right) => h_resizable("workspace-split")
                .child(resizable_panel().child(center))
                .child(
                    resizable_panel()
                        .size(px(520.))
                        .size_range(px(320.)..px(1100.))
                        .child(right),
                )
                .into_any_element(),
            None => center,
        };

        v_flex()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_new_thread))
            .on_action(cx.listener(Self::on_add_project))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_toggle_right))
            .on_action(cx.listener(Self::on_show_terminal))
            .on_action(cx.listener(Self::on_show_changes))
            .on_action(cx.listener(Self::on_toggle_theme))
            .on_action(cx.listener(Self::on_about))
            .on_action(cx.listener(Self::on_new_chat))
            .on_action(cx.listener(Self::on_commit_and_push))
            .on_action(cx.listener(Self::on_show_review))
            .on_action(cx.listener(Self::on_manage_worktrees))
            .on_action(cx.listener(Self::on_focus_composer))
            .on_action(cx.listener(Self::on_command_palette))
            .on_action(cx.listener(Self::on_find_file))
            .on_action(cx.listener(Self::on_search_in_files))
            .on_action(cx.listener(Self::on_show_files))
            .on_action(cx.listener(Self::on_show_context))
            .on_action(cx.listener(Self::on_show_shortcuts))
            .on_action(cx.listener(Self::on_toggle_terminal_workspace))
            .on_action(cx.listener(Self::on_open_in_editor))
            .on_action(cx.listener(Self::on_new_side_chat))
            .on_action(cx.listener(Self::on_import_thread))
            .on_action(cx.listener(Self::on_fork_thread))
            .on_action(cx.listener(Self::on_toggle_split))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_previous_tab))
            .on_action(cx.listener(Self::on_recent_thread))
            .on_action(cx.listener(Self::on_go_back))
            .on_action(cx.listener(Self::on_go_forward))
            .on_action(cx.listener(|this, _: &SelectTab1, w, cx| this.select_tab(0, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab2, w, cx| this.select_tab(1, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab3, w, cx| this.select_tab(2, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab4, w, cx| this.select_tab(3, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab5, w, cx| this.select_tab(4, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab6, w, cx| this.select_tab(5, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab7, w, cx| this.select_tab(6, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab8, w, cx| this.select_tab(7, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab9, w, cx| this.select_tab(8, w, cx)))
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(title_bar)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar)
                    .child(div().flex_1().min_w_0().h_full().child(main)),
            )
            .when_some(self.palette.clone(), |this, palette| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .justify_center()
                        .pt(px(72.))
                        .bg(gpui_kit::black().opacity(0.2))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.palette = None;
                                this.on_focus_composer(&FocusComposer, window, cx);
                                cx.notify();
                            }),
                        )
                        .child(
                            div()
                                .occlude()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(palette),
                        ),
                )
            })
            .when(self.about_open, |this| {
                this.child(crate::about::render(
                    self.about_logo.clone(),
                    &self.about_focus,
                    cx,
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::format_age;

    #[test]
    fn formats_relative_age() {
        assert_eq!(format_age(chrono::Duration::seconds(10)), "now");
        assert_eq!(format_age(chrono::Duration::minutes(5)), "5m");
        assert_eq!(format_age(chrono::Duration::hours(3)), "3h");
        assert_eq!(format_age(chrono::Duration::days(2)), "2d");
    }
}
