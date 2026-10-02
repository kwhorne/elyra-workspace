use crate::actions::*;
use crate::app_state::AppState;
use crate::changes_view::ChangesView;
use crate::terminal_view::TerminalPanel;
use crate::thread_session::SessionEvent;
use crate::thread_view::ThreadView;
use elyra_core::{ProjectId, ThreadId, ThreadStatus};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
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
        let right_tab = if layout.next() == Some("terminal") {
            RightTab::Terminal
        } else {
            RightTab::Changes
        };
        let mut this = Self {
            app,
            focus: cx.focus_handle(),
            open_tabs,
            active: None,
            current_project,
            thread_views: HashMap::new(),
            changes_views: HashMap::new(),
            terminal_views: HashMap::new(),
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
            if self.right_tab == RightTab::Terminal {
                "terminal"
            } else {
                "changes"
            }
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
                if let SessionEvent::TurnCompleted = event
                    && let Some(changes) = this.changes_views.get(&id)
                {
                    changes.update(cx, |changes, cx| changes.refresh(cx));
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
        let Some(view) = self.thread_view(id, window, cx) else {
            return;
        };
        if !self.open_tabs.contains(&id) {
            self.open_tabs.push(id);
        }
        self.active = Some(id);
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
                    let view = cx.new(|cx| ChangesView::new(cwd, window, cx));
                    self.changes_views.insert(id, view);
                }
            }
            RightTab::Terminal => {
                // A terminal keeps the directory it started in; a thread that
                // moved into a worktree gets a fresh shell there.
                self.terminal_views
                    .entry(id)
                    .or_insert_with(|| cx.new(|cx| TerminalPanel::new(cwd, window, cx)));
            }
        }
    }

    pub(crate) fn close_tab(&mut self, id: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.open_tabs.iter().position(|t| *t == id) else {
            return;
        };
        self.open_tabs.remove(index);
        self.thread_views.remove(&id);
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
                        .label("Add project  ⌘O")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.on_add_project(&AddProject, window, cx)
                        }))
                })
                .into_any_element();
        };
        v_flex()
            .size_full()
            .child(self.render_tabs(cx))
            .child(div().flex_1().min_h_0().child(view))
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
            _ => div()
                .p_4()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Open a thread to see its changes and terminal.")
                .into_any_element(),
        };
        let index = match self.right_tab {
            RightTab::Changes => 0,
            RightTab::Terminal => 1,
        };
        v_flex()
            .size_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                TabBar::new("right-tabs")
                    .underline()
                    .small()
                    .selected_index(index)
                    .child(
                        Tab::new()
                            .label("Changes")
                            .prefix(Icon::new(IconName::GitCompareArrows).xsmall()),
                    )
                    .child(
                        Tab::new()
                            .label("Terminal")
                            .prefix(Icon::new(IconName::SquareTerminal).xsmall()),
                    )
                    .on_click(cx.listener(|this, index: &usize, window, cx| {
                        let tab = if *index == 0 {
                            RightTab::Changes
                        } else {
                            RightTab::Terminal
                        };
                        this.show_right_tab(tab, window, cx);
                    })),
            )
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
        let center = self.render_center(window, cx);
        let title_bar = self.render_title_bar(cx);
        let sidebar = self.sidebar_open.then(|| self.render_sidebar(cx));
        let right = (self.right_open && self.active.is_some()).then(|| self.render_right(cx));

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
            .on_action(cx.listener(Self::on_focus_composer))
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
