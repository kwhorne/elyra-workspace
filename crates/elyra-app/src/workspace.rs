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
    Browser,
    Context,
    SideChat,
}

impl RightTab {
    const ALL: [RightTab; 7] = [
        RightTab::Changes,
        RightTab::PullRequest,
        RightTab::Files,
        RightTab::Browser,
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
            RightTab::Browser => "browser",
            RightTab::Context => "context",
            RightTab::SideChat => "side",
        }
    }
}

/// Full-width views that replace the chat area.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Panel {
    Review,
    Automations,
    Tasks,
    Stats,
    Race,
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
    pub(crate) browser_views: HashMap<ThreadId, Entity<crate::browser_view::BrowserView>>,
    /// Picture of the browser page taken when a thread's turn started: (url, file).
    page_before: HashMap<ThreadId, (String, String)>,
    race: Option<Entity<crate::race::RaceView>>,
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
    /// The thread whose Files view is open in the large editor sheet.
    files_sheet: Option<ThreadId>,
    /// Project space shown in the sidebar; None shows all.
    pub(crate) active_space: Option<String>,
    review: Option<Entity<crate::review_inbox::ReviewInbox>>,
    pub(crate) panel: Option<Panel>,
    automations: Option<Entity<crate::automations::AutomationsView>>,
    tasks: Option<Entity<crate::tasks::TasksView>>,
    stats: Option<Entity<crate::stats::StatsView>>,
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
        cx.set_global(crate::browser_tools::BrowserHub {
            workspace: cx.entity().downgrade(),
            window: window.window_handle(),
        });
        let subscriptions = vec![
            cx.observe_in(&app, window, |this, _, window, cx| {
                this.app_changed(window, cx);
                cx.notify();
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                this.mark_active_read(window, cx);
                if window.is_window_active() {
                    crate::quitting::came_back(window, cx);
                }
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
            browser_views: HashMap::new(),
            page_before: HashMap::new(),
            race: None,
            context_views: HashMap::new(),
            palette: None,
            split: None,
            recent: Vec::new(),
            back: Vec::new(),
            forward: Vec::new(),
            terminal_full: false,
            files_sheet: None,
            active_space,
            review: None,
            panel: None,
            automations: None,
            tasks: None,
            stats: None,
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
        // Development: open a panel at launch (screenshots without input).
        let panel = match std::env::var("ELYRA_OPEN_PANEL").as_deref() {
            Ok("tasks") => Some(Panel::Tasks),
            Ok("automations") => Some(Panel::Automations),
            Ok("stats") => Some(Panel::Stats),
            Ok("review") => Some(Panel::Review),
            _ => None,
        };
        if let Some(panel) = panel {
            self.toggle_panel(panel, window, cx);
        }
        // `files` or `files:<path>`: the large file editor.
        if let Ok(spec) = std::env::var("ELYRA_OPEN_PANEL")
            && let Some(rest) = spec.strip_prefix("files")
        {
            if let Some(path) = rest.strip_prefix(':') {
                self.open_file(path, None, window, cx);
            }
            self.open_files_sheet(window, cx);
        }
        // Development: open an address in the active thread's browser.
        if let (Ok(url), Some(id)) = (std::env::var("ELYRA_BROWSER_URL"), self.active) {
            self.open_in_browser(id, &url, window, cx);
        }
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
                match event {
                    SessionEvent::TurnStarted => this.picture_page_before(id, cx),
                    SessionEvent::TurnCompleted => this.picture_page_after(id, cx),
                    SessionEvent::SiteReady(url) => this.open_site(id, url, window, cx),
                    SessionEvent::NeedsAttention => {}
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
        self.panel = None;
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
                            let crate::context_view::ContextEvent::OpenUrl(url) = event;
                            this.open_in_browser(id, url, window, cx);
                        },
                    ));
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
        self.browser_views.remove(&id);
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
        self.toggle_panel(Panel::Review, window, cx);
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
            cx.notify();
            return;
        }
        self.panel = Some(panel);
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
        self.activate(id, window, cx);
    }

    fn on_show_automations(
        &mut self,
        _: &ShowAutomations,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_panel(Panel::Automations, window, cx);
    }

    fn on_show_tasks(&mut self, _: &ShowTasks, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_panel(Panel::Tasks, window, cx);
    }

    fn on_show_stats(&mut self, _: &ShowStats, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_panel(Panel::Stats, window, cx);
    }

    /// Save a thread as a ZIP chosen by the user.
    pub(crate) fn export_thread(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(title) = self.app.read(cx).thread(id).map(|t| t.title.clone()) else {
            return;
        };
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let path = cx.prompt_for_new_path(&dir, Some(&crate::export::file_name(&title)));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                match crate::export::export_thread(this.app.read(cx), id, &path) {
                    Ok(()) => window.push_notification(
                        Notification::success(format!("Exported to {}", path.display()))
                            .on_click(move |_, _, cx| cx.reveal_path(&path)),
                        cx,
                    ),
                    Err(err) => window.push_notification(
                        Notification::error(format!("Export failed: {err:#}")),
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    fn on_export_thread(&mut self, _: &ExportThread, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active {
            self.export_thread(id, window, cx);
        }
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
            SessionEvent::TurnStarted | SessionEvent::SiteReady(_) => return,
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

    fn on_open_files_editor(
        &mut self,
        _: &OpenFilesEditor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.files_sheet.is_some() && window.has_active_sheet(cx) {
            window.close_sheet(cx);
        } else {
            self.open_files_sheet(window, cx);
        }
    }

    /// Show the active thread's Files view in a sheet over half the window,
    /// where the editor has room; the side panel is too narrow for it.
    pub(crate) fn open_files_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.active else {
            return;
        };
        if !self.files_views.contains_key(&id) {
            self.show_right_tab(RightTab::Files, window, cx);
        }
        let Some(view) = self.files_views.get(&id).cloned() else {
            return;
        };
        view.update(cx, |view, cx| view.set_in_sheet(true, cx));
        self.files_sheet = Some(id);
        let title = self
            .front_thread(cx)
            .map(|(project, _)| project)
            .unwrap_or_default();
        window.open_sheet(cx, move |sheet, _, _| {
            sheet
                .size(relative(0.5))
                .p_0()
                .title(title.clone())
                .child(div().size_full().child(view.clone()))
        });
        cx.notify();
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

    // ---- browser -----------------------------------------------------------

    /// The thread's browser, created on first use.
    pub(crate) fn browser_view(
        &mut self,
        id: ThreadId,
        cwd: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<crate::browser_view::BrowserView> {
        if let Some(view) = self.browser_views.get(&id) {
            view.update(cx, |view, cx| view.set_cwd(cwd, cx));
            return view.clone();
        }
        let view = cx.new(|cx| crate::browser_view::BrowserView::new(cwd, window, cx));
        if let Some(thread) = self.thread_views.get(&id) {
            let browser = view.clone();
            thread.update(cx, |thread, cx| thread.set_browser(browser, window, cx));
        }
        self.browser_views.insert(id, view.clone());
        view
    }

    /// Open `url` in a thread's browser, and show it when that thread is in
    /// front.
    pub(crate) fn open_in_browser(
        &mut self,
        id: ThreadId,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<crate::browser_view::BrowserView>> {
        let cwd = {
            let state = self.app.read(cx);
            let thread = state.thread(id)?;
            thread.working_dir(state.project(thread.project_id)?)
        };
        let view = self.browser_view(id, cwd, window, cx);
        if self.active == Some(id) {
            self.show_right_tab(RightTab::Browser, window, cx);
        }
        view.update(cx, |view, cx| view.open(url, window, cx));
        Some(view)
    }

    /// Tell every browser whether it's on screen and uncovered.
    fn sync_browsers(&mut self, panel_open: bool, window: &mut Window, cx: &mut Context<Self>) {
        let covered = self.palette.is_some() || self.about_open;
        let visible_tab = self.right_open
            && self.right_tab == RightTab::Browser
            && !panel_open
            && !self.terminal_full;
        for (id, view) in self.browser_views.clone() {
            let shown = visible_tab && self.active == Some(id);
            view.update(cx, |view, cx| view.set_shown(shown, covered, window, cx));
        }
    }

    fn on_show_browser(&mut self, _: &ShowBrowser, window: &mut Window, cx: &mut Context<Self>) {
        self.show_right_tab(RightTab::Browser, window, cx);
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

    /// Load a thread's own Grove site in its browser, without switching tabs.
    fn open_site(&mut self, id: ThreadId, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cwd) = self.app.read(cx).thread(id).and_then(|thread| {
            let project = self.app.read(cx).project(thread.project_id)?;
            Some(thread.working_dir(project))
        }) else {
            return;
        };
        let browser = self.browser_view(id, cwd, window, cx);
        browser.update(cx, |browser, cx| browser.open(url, window, cx));
    }

    // ---- page pictures around turns ------------------------------------------

    /// Ask the thread's browser for a picture of its page, when it shows a
    /// local page on screen (WebKit only pictures what is on screen).
    fn picture_page(
        &self,
        id: ThreadId,
        cx: &App,
    ) -> Option<(String, async_channel::Receiver<Option<Vec<u8>>>)> {
        let browser = self.browser_views.get(&id)?.read(cx);
        let url = browser.page_state().url.clone();
        if !browser.is_on_screen() || !crate::webview::is_local_address(&url) {
            return None;
        }
        let page = browser.web_view()?;
        let (tx, rx) = async_channel::bounded(1);
        page.agent_snapshot(move |jpeg| {
            tx.try_send(jpeg).ok();
        });
        Some((url, rx))
    }

    fn picture_page_before(&mut self, id: ThreadId, cx: &mut Context<Self>) {
        self.page_before.remove(&id);
        let Some((url, rx)) = self.picture_page(id, cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let Some(path) = save_page_picture(id, rx, cx).await else {
                return;
            };
            let _ = this.update(cx, |this, _| this.page_before.insert(id, (url, path)));
        })
        .detach();
    }

    /// After a turn: reload the page, give the dev server a moment, and keep
    /// a picture next to the one from before the turn.
    fn picture_page_after(&mut self, id: ThreadId, cx: &mut Context<Self>) {
        let on_screen = self.browser_views.get(&id).is_some_and(|browser| {
            let browser = browser.read(cx);
            browser.is_on_screen() && crate::webview::is_local_address(&browser.page_state().url)
        });
        if !on_screen {
            self.page_before.remove(&id);
            return;
        }
        if let Some(page) = self.browser_views[&id].read(cx).web_view() {
            page.reload();
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAGE_SETTLE).await;
            let Ok(Some((url, rx))) = this.update(cx, |this, cx| this.picture_page(id, cx)) else {
                return;
            };
            let Some(after) = save_page_picture(id, rx, cx).await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let before = this
                    .page_before
                    .remove(&id)
                    .filter(|(before_url, _)| *before_url == url)
                    .map(|(_, path)| path);
                if let Some(session) = this.app.update(cx, |app, cx| app.session(id, cx)) {
                    session.update(cx, |session, cx| {
                        session.add_page_snapshot(url, before, after, cx)
                    });
                }
            });
        })
        .detach();
    }

    fn on_todays_work(&mut self, _: &TodaysWork, window: &mut Window, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let summary = cx.new(|cx| crate::day_summary::DaySummary::new(app, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Today's work")
                .w(px(640.))
                .child(summary.clone())
        });
    }

    // ---- best of N --------------------------------------------------------

    fn on_best_of_n(&mut self, _: &BestOfN, window: &mut Window, cx: &mut Context<Self>) {
        let project = self
            .active
            .and_then(|id| self.app.read(cx).thread(id).map(|t| t.project_id))
            .or(self.current_project);
        match project {
            Some(project) => crate::race::start_dialog(cx.weak_entity(), project, window, cx),
            None => window.push_notification(
                Notification::info("Open a project first: best of N runs in a project's folder."),
                cx,
            ),
        }
    }

    /// Give `prompt` to each provider in a new thread with its own worktree.
    pub(crate) fn start_race(
        &mut self,
        project: ProjectId,
        prompt: String,
        providers: Vec<elyra_core::ProviderKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let race = uuid::Uuid::new_v4();
        let short: String = prompt
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(40)
            .collect();
        for provider in providers {
            let thread = match self
                .app
                .update(cx, |app, cx| app.create_thread(project, cx))
            {
                Ok(thread) => thread,
                Err(err) => {
                    window.push_notification(Notification::error(format!("{err:#}")), cx);
                    continue;
                }
            };
            let Some(session) = self.app.update(cx, |app, cx| app.session(thread.id, cx)) else {
                continue;
            };
            let prompt = prompt.clone();
            session.update(cx, |session, cx| {
                session.set_provider(provider, cx);
                session.thread.race_id = Some(race);
                session.use_worktree = true;
                session.rename(format!("{short} · {}", provider.label()), cx);
                session.submit(elyra_provider::Prompt::text(prompt), cx);
            });
        }
        self.show_race(race, window, cx);
    }

    /// Open the comparison for a best-of-N run.
    pub(crate) fn show_race(
        &mut self,
        race: uuid::Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = match &self.race {
            Some(view) => view.clone(),
            None => {
                let (app, workspace) = (self.app.clone(), cx.weak_entity());
                let view = cx.new(|cx| crate::race::RaceView::new(app, workspace, cx));
                self.race = Some(view.clone());
                view
            }
        };
        view.update(cx, |view, cx| view.set_race(race, cx));
        if self.panel != Some(Panel::Race) {
            self.toggle_panel(Panel::Race, window, cx);
        }
    }

    // ---- second opinion --------------------------------------------------

    /// Have another provider review what the thread's last turn changed, in a
    /// read-only side chat.
    pub(crate) fn second_opinion(
        &mut self,
        id: ThreadId,
        provider: elyra_core::ProviderKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.app.update(cx, |app, cx| app.session(id, cx)) else {
            return;
        };
        let (root, from, request, author) = {
            let session = session.read(cx);
            (
                session.working_dir(),
                session
                    .turn_checkpoints()
                    .last()
                    .map(|(_, sha)| sha.clone()),
                session.last_request().unwrap_or_default(),
                session.thread.provider,
            )
        };
        let Some(from) = from else {
            window.push_notification(
                Notification::info(
                    "Nothing to review yet: the thread has no turn with a checkpoint.",
                ),
                cx,
            );
            return;
        };
        let job = cx.background_executor().spawn(async move {
            let to = elyra_git::checkpoint::snapshot(&root, "elyra: second opinion")?;
            elyra_git::diff_text(&root, &from, &to)
        });
        cx.spawn_in(window, async move |this, cx| {
            let diff = job.await;
            let _ = this.update_in(cx, |this, window, cx| match diff {
                Ok(diff) if diff.trim().is_empty() => window.push_notification(
                    Notification::info(
                        "The last turn changed no files, so there is nothing to review.",
                    ),
                    cx,
                ),
                Ok(diff) => this.start_review(id, provider, author, &request, &diff, window, cx),
                Err(err) => window.push_notification(
                    Notification::error(format!("Could not read the changes: {err:#}")),
                    cx,
                ),
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn start_review(
        &mut self,
        id: ThreadId,
        provider: elyra_core::ProviderKind,
        author: elyra_core::ProviderKind,
        request: &str,
        diff: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = match self
            .app
            .update(cx, |app, cx| app.create_review_chat(id, provider, cx))
        {
            Ok(session) => session,
            Err(err) => {
                window.push_notification(Notification::error(format!("{err:#}")), cx);
                return;
            }
        };
        let prompt = review_prompt(author.label(), request, diff);
        session.update(cx, |session, cx| {
            session.submit(elyra_provider::Prompt::text(prompt), cx)
        });
        let review = session.read(cx).thread.id;
        if self.thread_view(review, window, cx).is_some() {
            self.show_right_tab(RightTab::SideChat, window, cx);
        }
    }

    /// Put a side chat's last reply in its parent thread's composer.
    fn send_side_reply(&mut self, side: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.app.update(cx, |app, cx| app.session(side, cx)) else {
            return;
        };
        let (reply, parent, title) = {
            let session = session.read(cx);
            (
                session.last_reply(),
                session.thread.parent_id,
                session.thread.title.clone(),
            )
        };
        let (Some(parent), false) = (parent, reply.trim().is_empty()) else {
            return;
        };
        let text = if title.starts_with("Second opinion") {
            format!(
                "Another agent reviewed your last changes. Fix what you agree with and say why you disagree with the rest:\n\n{}",
                reply.trim()
            )
        } else {
            reply.trim().to_string()
        };
        if let Some(view) = self.thread_view(parent, window, cx) {
            view.update(cx, |view, cx| {
                view.append_to_composer(&text, window, cx);
                view.focus_composer(window, cx);
            });
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

    /// The project and title of the thread in front, for the window title.
    fn front_thread(&self, cx: &App) -> Option<(String, String)> {
        let state = self.app.read(cx);
        let thread = state.thread(self.active?)?;
        let project = state.project(thread.project_id).map(|p| p.name.clone())?;
        Some((project, thread.title.clone()))
    }

    fn render_title_bar(&self, cx: &Context<Self>) -> AnyElement {
        let front = self.front_thread(cx);
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
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .justify_center()
                            .gap_2()
                            .text_sm()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .map(|this| match front {
                                Some((project, title)) => this
                                    .child(
                                        div()
                                            .flex_none()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(project),
                                    )
                                    .child(div().text_color(cx.theme().muted_foreground).child("—"))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(title),
                                    ),
                                None => this
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Elyra Workspace"),
                            }),
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
            .filter_map(|id| {
                let thread = state.thread(*id)?;
                Some((
                    *id,
                    thread.title.clone(),
                    state.project(thread.project_id).cloned(),
                ))
            })
            .map(|(id, title, project)| {
                let short: String = title.chars().take(28).collect();
                // Which project the thread is in, with its colour: tabs from
                // several projects otherwise look alike.
                let prefix = project.map(|project| {
                    let color = project
                        .color
                        .as_deref()
                        .and_then(crate::sidebar::parse_color)
                        .unwrap_or(cx.theme().muted_foreground);
                    let name: String = project.name.chars().take(18).collect();
                    h_flex()
                        .gap_1()
                        .mr_1()
                        .child(match project.icon.as_deref().filter(|i| !i.is_empty()) {
                            Some(icon) => {
                                div().text_xs().child(icon.to_string()).into_any_element()
                            }
                            None => div()
                                .size(px(7.))
                                .rounded_full()
                                .bg(color)
                                .into_any_element(),
                        })
                        .child(div().text_xs().text_color(color).child(name))
                });
                Tab::new()
                    .label(short)
                    .when_some(prefix, |tab, prefix| tab.prefix(prefix))
                    .suffix(
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
            (RightTab::Files, Some(id)) if self.files_sheet == Some(id) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Open in the large editor.")
                .child(
                    Button::new("files-sheet-back")
                        .small()
                        .label("Show it here")
                        .on_click(|_, window, cx| window.close_sheet(cx)),
                )
                .into_any_element(),
            (RightTab::Files, Some(id)) => self
                .files_views
                .get(&id)
                .map(|v| v.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            (RightTab::Browser, Some(id)) => self
                .browser_views
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
                RightTab::Browser => ("Browser".into(), IconName::Globe),
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
                        Button::new("side-send")
                            .ghost()
                            .xsmall()
                            .icon(IconName::CornerDownLeft)
                            .tooltip("Put the last reply in the thread's composer")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.send_side_reply(side, window, cx)
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
        // The window's own title, for Mission Control and ⌘`.
        let window_title = match self.front_thread(cx) {
            Some((project, title)) => format!("{project} — {title}"),
            None => "Elyra Workspace".to_string(),
        };
        window.set_window_title(&window_title);
        // The large editor sheet was closed: hand the Files view back.
        if let Some(id) = self.files_sheet
            && !window.has_active_sheet(cx)
        {
            self.files_sheet = None;
            if let Some(view) = self.files_views.get(&id) {
                view.update(cx, |view, cx| view.set_in_sheet(false, cx));
            }
        }
        let full_terminal = self
            .terminal_full
            .then(|| self.active.and_then(|id| self.terminal_views.get(&id)))
            .flatten()
            .cloned();
        if full_terminal.is_none() {
            self.terminal_full = false;
        }
        let panel: Option<AnyElement> = match self.panel {
            Some(Panel::Review) => self.review.clone().map(|v| v.into_any_element()),
            Some(Panel::Automations) => self.automations.clone().map(|v| v.into_any_element()),
            Some(Panel::Tasks) => self.tasks.clone().map(|v| v.into_any_element()),
            Some(Panel::Stats) => self.stats.clone().map(|v| v.into_any_element()),
            Some(Panel::Race) => self.race.clone().map(|v| v.into_any_element()),
            None => None,
        };
        let panel_open = panel.is_some();
        self.sync_browsers(panel_open, window, cx);
        let center = match (panel, full_terminal) {
            (Some(panel), _) => panel,
            (None, Some(terminal)) => terminal.into_any_element(),
            _ => self.render_center(window, cx),
        };
        let title_bar = self.render_title_bar(cx);
        let sidebar = self.sidebar_open.then(|| self.render_sidebar(cx));
        let right =
            (self.right_open && self.active.is_some() && !panel_open && !self.terminal_full)
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
            .on_action(cx.listener(Self::on_open_files_editor))
            .on_action(cx.listener(Self::on_open_in_editor))
            .on_action(cx.listener(Self::on_new_side_chat))
            .on_action(cx.listener(Self::on_import_thread))
            .on_action(cx.listener(Self::on_show_automations))
            .on_action(cx.listener(Self::on_show_tasks))
            .on_action(cx.listener(Self::on_show_stats))
            .on_action(cx.listener(Self::on_show_browser))
            .on_action(cx.listener(Self::on_best_of_n))
            .on_action(cx.listener(Self::on_todays_work))
            .on_action(cx.listener(Self::on_export_thread))
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

/// How long a reloaded page gets to settle before its picture is taken.
const PAGE_SETTLE: std::time::Duration = std::time::Duration::from_millis(2500);

/// Wait for a page picture and keep it under the thread's snapshot folder.
async fn save_page_picture(
    id: ThreadId,
    rx: async_channel::Receiver<Option<Vec<u8>>>,
    cx: &mut AsyncApp,
) -> Option<String> {
    let jpeg = rx.recv().await.ok().flatten()?;
    cx.background_executor()
        .spawn(async move {
            let dir = elyra_core::paths::snapshots_dir().join(id.to_string());
            std::fs::create_dir_all(&dir).ok()?;
            let path = dir.join(format!("{}.jpg", uuid::Uuid::new_v4().simple()));
            std::fs::write(&path, jpeg).ok()?;
            Some(path.to_string_lossy().into_owned())
        })
        .await
}

/// The request to a reviewing agent: what was asked, the diff, and how to
/// report findings.
fn review_prompt(author: &str, request: &str, diff: &str) -> String {
    const MAX_DIFF: usize = 80_000;
    let request: String = request.chars().take(1_500).collect();
    let mut diff_text: String = diff.chars().take(MAX_DIFF).collect();
    if diff_text.len() < diff.len() {
        diff_text.push_str("\n… (diff cut; read the files for the rest)");
    }
    format!(
        "Give a second opinion on changes another coding agent ({author}) just made in this repository.\n\nWhat it was asked:\n\n> {}\n\nThe changes:\n\n```diff\n{}\n```\n\nRead the surrounding code as needed, but do not change any files. Look for bugs, missed cases, security problems and anything that does not do what was asked. List each finding as `path:line` — what is wrong — how to fix it, most important first. If nothing is worth changing, say so in one line.",
        request.trim().replace('\n', "\n> "),
        diff_text.trim_end()
    )
}

#[cfg(test)]
mod tests {
    use super::{format_age, review_prompt};

    #[test]
    fn formats_relative_age() {
        assert_eq!(format_age(chrono::Duration::seconds(10)), "now");
        assert_eq!(format_age(chrono::Duration::minutes(5)), "5m");
        assert_eq!(format_age(chrono::Duration::hours(3)), "3h");
        assert_eq!(format_age(chrono::Duration::days(2)), "2d");
    }

    #[test]
    fn review_prompt_quotes_the_request_and_includes_the_diff() {
        let prompt = review_prompt("Claude Code", "Fix totals\nand tax", "+a\n-b\n");
        assert!(prompt.contains("(Claude Code)"));
        assert!(prompt.contains("> Fix totals\n> and tax"));
        assert!(prompt.contains("```diff\n+a\n-b\n```"));
        assert!(prompt.contains("do not change any files"));
        assert!(!prompt.contains("diff cut"));

        let long = "x".repeat(90_000);
        assert!(review_prompt("Codex", "r", &long).contains("diff cut"));
    }
}
