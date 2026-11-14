//! The main window: the sidebar, the thread tabs, the conversation and the
//! tools panel, and the panels and sheets over them. This module has the
//! state, startup and what follows the threads' sessions; beside it are
//! `tabs` (open threads and moving between them), `panels` (the tools panel,
//! the sidebar and the panels over the conversation), `files` (the command
//! palette and files), `away`, `browser`, `threads` (forks, races, duels,
//! reviews, side chats) and `render`.

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

mod away;
mod browser;
mod files;
mod panels;
mod render;
mod tabs;
mod threads;

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
    Search,
    /// Reviewing the active thread's branch one instruction at a time.
    Walkthrough,
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
    /// When the window last lost focus (for "While you were away").
    away_since: Option<chrono::DateTime<chrono::Utc>>,
    /// The last time away that was summarized: (from, to).
    last_away: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    /// Scrolls the thread tabs when they don't fit.
    tab_scroll: ScrollHandle,
    /// The tab last scrolled into view.
    tab_revealed: std::cell::Cell<Option<ThreadId>>,
    /// Project space shown in the sidebar; None shows all.
    pub(crate) active_space: Option<String>,
    review: Option<Entity<crate::review_inbox::ReviewInbox>>,
    pub(crate) panel: Option<Panel>,
    automations: Option<Entity<crate::automations::AutomationsView>>,
    tasks: Option<Entity<crate::tasks::TasksView>>,
    stats: Option<Entity<crate::stats::StatsView>>,
    /// The title last given to the window (set only when it changes).
    window_title: String,
    search: Option<Entity<crate::thread_search::ThreadSearch>>,
    walkthrough: Option<Entity<crate::walkthrough::Walkthrough>>,
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
                    this.came_back(window, cx);
                } else {
                    this.away_since = Some(chrono::Utc::now());
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
            away_since: None,
            last_away: None,
            tab_scroll: ScrollHandle::new(),
            tab_revealed: std::cell::Cell::new(None),
            active_space,
            review: None,
            panel: None,
            automations: None,
            tasks: None,
            stats: None,
            window_title: String::new(),
            search: None,
            walkthrough: None,
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
            Ok("search") => Some(Panel::Search),
            Ok("walkthrough") => Some(Panel::Walkthrough),
            Ok("review") => Some(Panel::Review),
            _ => None,
        };
        if let Some(panel) = panel {
            self.toggle_panel(panel, window, cx);
        }
        // `away`: what the threads did in the last day.
        if std::env::var("ELYRA_OPEN_PANEL").as_deref() == Ok("away") {
            let now = chrono::Utc::now();
            self.show_away(now - chrono::Duration::hours(24), now, window, cx);
        }
        // `race`: the best-of-N comparison of the active thread's race.
        if std::env::var("ELYRA_OPEN_PANEL").as_deref() == Ok("race")
            && let Some(race) = self
                .active
                .and_then(|id| self.app.read(cx).thread(id))
                .and_then(|t| t.race_id)
        {
            self.show_race(race, window, cx);
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
        // `settings` or `settings:<page>`: the Settings window.
        if std::env::var("ELYRA_OPEN_PANEL").is_ok_and(|spec| spec.starts_with("settings")) {
            crate::settings_window::open(cx);
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
        let checks_failed = self
            .app
            .read(cx)
            .existing_session(id)
            .is_some_and(|session| session.read(cx).checks_failed());
        let (message, kind) = match event {
            SessionEvent::TurnCompleted if checks_failed => {
                ("The checks failed", NotificationType::Error)
            }
            SessionEvent::TurnCompleted if thread.status == ThreadStatus::Failed => {
                ("Stopped with an error", NotificationType::Error)
            }
            SessionEvent::TurnCompleted => ("Finished", NotificationType::Success),
            SessionEvent::NeedsAttention => ("Needs your input", NotificationType::Warning),
            SessionEvent::TurnStarted
            | SessionEvent::SiteReady(_)
            | SessionEvent::Streamed
            | SessionEvent::FilesChanged
            | SessionEvent::TestsTested => {
                return;
            }
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
