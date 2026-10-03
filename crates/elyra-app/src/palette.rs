//! Command palette (⌘K), file finder (⌘P) and content search (⇧⌘F).

use crate::actions;
use crate::app_state::AppState;
use crate::composer::fuzzy_score;
use crate::themes;
use elyra_core::{ProjectId, ThreadId};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::PathBuf;
use std::sync::Arc;

const MAX_ROWS: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteMode {
    Commands,
    Files,
    Search,
}

/// What picking an entry does.
#[derive(Clone)]
pub enum Target {
    Action(Arc<dyn Fn() -> Box<dyn Action>>),
    Thread(ThreadId),
    Project(ProjectId),
    Theme(String),
    File { path: String, line: Option<usize> },
}

#[derive(Clone)]
struct Entry {
    icon: IconName,
    label: String,
    detail: String,
    hint: String,
    target: Target,
    score: i64,
}

pub enum PaletteEvent {
    Pick(Target),
    Dismiss,
}

pub struct Palette {
    mode: PaletteMode,
    input: Entity<InputState>,
    entries: Vec<Entry>,
    selected: usize,
    /// Keeps the selected row in view while moving with the keyboard.
    scroll: ScrollHandle,
    app: Entity<AppState>,
    root: Option<PathBuf>,
    files: Option<Arc<Vec<String>>>,
    loading: bool,
    generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PaletteEvent> for Palette {}

type ActionFactory = Arc<dyn Fn() -> Box<dyn Action>>;
type Command = (&'static str, &'static str, IconName, ActionFactory);

macro_rules! command {
    ($label:expr, $hint:expr, $icon:expr, $action:expr) => {
        (
            $label,
            $hint,
            $icon,
            Arc::new(|| Box::new($action) as Box<dyn Action>) as Arc<dyn Fn() -> Box<dyn Action>>,
        )
    };
}

fn commands() -> Vec<Command> {
    vec![
        command!("New thread", "⌘N", IconName::SquarePen, actions::NewThread),
        command!(
            "New chat without a project",
            "⌥⌘N",
            IconName::MessageSquarePlus,
            actions::NewChat
        ),
        command!(
            "Add project…",
            "⇧⌘O",
            IconName::FolderPlus,
            actions::AddProject
        ),
        command!(
            "Open in external editor",
            "⌘O",
            IconName::ExternalLink,
            actions::OpenInEditor
        ),
        command!("Find file…", "⌘P", IconName::File, actions::FindFile),
        command!(
            "Search in files…",
            "⇧⌘F",
            IconName::Search,
            actions::SearchInFiles
        ),
        command!(
            "Find in thread",
            "⌘F",
            IconName::TextSearch,
            actions::FindInThread
        ),
        command!(
            "Code review",
            "⇧⌘R",
            IconName::GitPullRequest,
            actions::ShowCodeReview
        ),
        command!(
            "Show changes",
            "⇧⌘G",
            IconName::GitCompareArrows,
            actions::ShowChanges
        ),
        command!(
            "Show files",
            "⇧⌘E",
            IconName::FolderTree,
            actions::ShowFiles
        ),
        command!("Show browser", "⇧⌘B", IconName::Globe, actions::ShowBrowser),
        command!(
            "Best of N… (one task, several agents)",
            "",
            IconName::Trophy,
            actions::BestOfN
        ),
        command!(
            "Toggle terminal",
            "⌘J",
            IconName::SquareTerminal,
            actions::ShowTerminal
        ),
        command!(
            "Full-width terminal",
            "⇧⌘J",
            IconName::SquareTerminal,
            actions::ToggleTerminalWorkspace
        ),
        command!(
            "Split chat",
            "⌘\\",
            IconName::Columns2,
            actions::ToggleSplit
        ),
        command!(
            "New side chat",
            "⌥⌘S",
            IconName::MessagesSquare,
            actions::NewSideChat
        ),
        command!("Fork thread", "⇧⌘K", IconName::GitFork, actions::ForkThread),
        command!(
            "Import sessions from Claude Code or Codex…",
            "⌘I",
            IconName::Import,
            actions::ImportThreads
        ),
        command!(
            "Export thread…",
            "",
            IconName::Download,
            actions::ExportThread
        ),
        command!(
            "Task board",
            "⌥⌘T",
            IconName::SquareKanban,
            actions::ShowTasks
        ),
        command!(
            "Automations",
            "⌥⌘A",
            IconName::CalendarClock,
            actions::ShowAutomations
        ),
        command!(
            "Usage statistics",
            "",
            IconName::ChartColumn,
            actions::ShowStats
        ),
        command!(
            "Show context and notes",
            "⇧⌘I",
            IconName::NotebookPen,
            actions::ShowContext
        ),
        command!(
            "Commit and push",
            "⌃⌘P",
            IconName::Upload,
            actions::CommitAndPush
        ),
        command!(
            "Toggle sidebar",
            "⌘B",
            IconName::PanelLeft,
            actions::ToggleSidebar
        ),
        command!(
            "Toggle tools panel",
            "⌥⌘B",
            IconName::PanelRight,
            actions::ToggleRightPanel
        ),
        command!(
            "Toggle light/dark",
            "⇧⌘T",
            IconName::SunMoon,
            actions::ToggleTheme
        ),
        command!("Settings", "⌘,", IconName::Settings, actions::OpenSettings),
        command!(
            "Keyboard shortcuts",
            "⌘/",
            IconName::Keyboard,
            actions::ShowShortcuts
        ),
        command!(
            "Documentation",
            "",
            IconName::BookOpen,
            actions::OpenDocumentation
        ),
        command!(
            "Manage worktrees…",
            "",
            IconName::GitBranch,
            actions::ManageWorktrees
        ),
        command!(
            "Check for updates…",
            "",
            IconName::Download,
            actions::CheckForUpdates
        ),
        command!("About Elyra Workspace", "", IconName::Info, actions::About),
    ]
}

impl Palette {
    pub fn new(
        mode: PaletteMode,
        app: Entity<AppState>,
        root: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = match mode {
            PaletteMode::Commands => "Search threads, projects and commands…",
            PaletteMode::Files => "Find a file by name…",
            PaletteMode::Search => "Search in files…",
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        input.update(cx, |input, cx| input.focus(window, cx));
        let weak = cx.weak_entity();
        let subscriptions = vec![
            cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.update_entries(cx);
                }
            }),
            cx.intercept_keystrokes(move |event, window, cx| {
                let _ = weak.update(cx, |this, cx| this.intercept(event, window, cx));
            }),
        ];
        let mut this = Self {
            mode,
            input,
            entries: Vec::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            app,
            root,
            files: None,
            loading: false,
            generation: 0,
            _subscriptions: subscriptions,
        };
        if mode == PaletteMode::Files {
            this.load_files(cx);
        }
        this.update_entries(cx);
        this
    }

    fn intercept(&mut self, event: &KeystrokeEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let mods = keystroke.modifiers;
        if mods.platform || mods.alt {
            return;
        }
        let handled = match keystroke.key.as_str() {
            "up" => {
                self.move_selection(-1, cx);
                true
            }
            "down" => {
                self.move_selection(1, cx);
                true
            }
            "n" if mods.control => {
                self.move_selection(1, cx);
                true
            }
            "p" if mods.control => {
                self.move_selection(-1, cx);
                true
            }
            "enter" => {
                self.pick(self.selected, cx);
                true
            }
            "escape" => {
                cx.emit(PaletteEvent::Dismiss);
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.entries.len() as isize;
        if len > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
            self.scroll.scroll_to_item(self.selected);
        }
        cx.notify();
    }

    fn pick(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self.entries.get(index) {
            cx.emit(PaletteEvent::Pick(entry.target.clone()));
        }
    }

    fn load_files(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        self.loading = true;
        let job = cx
            .background_executor()
            .spawn(async move { crate::composer::index_files(&root) });
        cx.spawn(async move |this, cx| {
            let files = job.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.files = Some(Arc::new(
                    files.into_iter().filter(|f| !f.ends_with('/')).collect(),
                ));
                this.update_entries(cx);
            });
        })
        .detach();
    }

    fn update_entries(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().trim().to_string();
        self.selected = 0;
        // New results start at the top (works before the list has been laid out).
        self.scroll.set_offset(point(px(0.), px(0.)));
        match self.mode {
            PaletteMode::Commands => self.entries = self.command_entries(&query, cx),
            PaletteMode::Files => {
                self.entries = self
                    .files
                    .as_ref()
                    .map(|files| file_entries(files, &query))
                    .unwrap_or_default();
            }
            PaletteMode::Search => self.run_search(query, cx),
        }
        cx.notify();
    }

    fn command_entries(&self, query: &str, cx: &App) -> Vec<Entry> {
        let state = self.app.read(cx);
        let mut entries = Vec::new();
        let score = |text: &str| fuzzy_score(text, query);
        for thread in state.threads.iter().filter(|t| t.parent_id.is_none()) {
            if let Some(score) = score(&thread.title) {
                let project = state
                    .project(thread.project_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                entries.push(Entry {
                    icon: IconName::MessageSquare,
                    label: thread.title.clone(),
                    detail: project,
                    hint: String::new(),
                    target: Target::Thread(thread.id),
                    // Recent threads first when there is no query.
                    score: if query.is_empty() {
                        thread.updated_at.timestamp()
                    } else {
                        score + 50
                    },
                });
            }
        }
        if query.is_empty() {
            entries.sort_by_key(|e| std::cmp::Reverse(e.score));
            entries.truncate(8);
        }
        for (label, hint, icon, action) in commands() {
            if let Some(score) = score(label) {
                entries.push(Entry {
                    icon,
                    label: label.to_string(),
                    detail: String::new(),
                    hint: hint.to_string(),
                    target: Target::Action(action),
                    score: if query.is_empty() { 0 } else { score },
                });
            }
        }
        for project in &state.projects {
            let label = format!("New thread in {}", project.name);
            if !query.is_empty()
                && let Some(score) = score(&label)
            {
                entries.push(Entry {
                    icon: IconName::Folder,
                    label,
                    detail: project.path.display().to_string(),
                    hint: String::new(),
                    target: Target::Project(project.id),
                    score,
                });
            }
        }
        for name in themes::all_names(cx) {
            let label = format!("Theme: {name}");
            if !query.is_empty()
                && let Some(score) = score(&label)
            {
                entries.push(Entry {
                    icon: IconName::Palette,
                    label,
                    detail: String::new(),
                    hint: String::new(),
                    target: Target::Theme(name.to_string()),
                    score,
                });
            }
        }
        if query.chars().count() >= 3 {
            for (thread_id, snippet) in state.store.search_messages(query, 8).unwrap_or_default() {
                if let Some(thread) = state.thread(thread_id) {
                    entries.push(Entry {
                        icon: IconName::TextSearch,
                        label: thread.title.clone(),
                        detail: snippet,
                        hint: "message".into(),
                        target: Target::Thread(thread_id),
                        score: -1000,
                    });
                }
            }
        }
        if !query.is_empty() {
            entries.sort_by_key(|e| std::cmp::Reverse(e.score));
        }
        entries.truncate(MAX_ROWS);
        entries
    }

    fn run_search(&mut self, query: String, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let Some(root) = self.root.clone() else {
            return;
        };
        if query.chars().count() < 2 {
            self.entries.clear();
            self.loading = false;
            return;
        }
        self.loading = true;
        let job = cx
            .background_executor()
            .spawn(async move { crate::search::search(&root, &query, 300) });
        cx.spawn(async move |this, cx| {
            let matches = job.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return; // a newer query is running
                }
                this.loading = false;
                this.entries = matches
                    .into_iter()
                    .map(|m| Entry {
                        icon: IconName::FileText,
                        label: m.text,
                        detail: format!("{}:{}", m.path, m.line),
                        hint: String::new(),
                        target: Target::File {
                            path: m.path,
                            line: Some(m.line),
                        },
                        score: 0,
                    })
                    .collect();
                cx.notify();
            });
        })
        .detach();
    }
}

fn file_entries(files: &[String], query: &str) -> Vec<Entry> {
    let mut scored: Vec<(i64, &String)> = files
        .iter()
        .filter_map(|path| Some((fuzzy_score(path, query)?, path)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(MAX_ROWS)
        .map(|(score, path)| {
            let (dir, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
            Entry {
                icon: IconName::File,
                label: name.to_string(),
                detail: dir.to_string(),
                hint: String::new(),
                target: Target::File {
                    path: path.clone(),
                    line: None,
                },
                score,
            }
        })
        .collect()
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.entries.iter().enumerate().map(|(index, entry)| {
            let selected = index == self.selected;
            h_flex()
                .id(("palette-row", index))
                .w_full()
                .px_3()
                .py_1p5()
                .gap_2()
                .rounded_md()
                .cursor_pointer()
                .when(selected, |this| this.bg(cx.theme().accent))
                .hover(|this| this.bg(cx.theme().accent.opacity(0.6)))
                .child(
                    Icon::new(entry.icon)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .flex_none()
                        .max_w(relative(0.6))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_sm()
                        .when(self.mode == PaletteMode::Search, |this| {
                            this.font_family(cx.theme().mono_font_family.clone())
                        })
                        .child(entry.label.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(entry.detail.clone()),
                )
                .when(!entry.hint.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(entry.hint.clone()),
                    )
                })
                .on_click(cx.listener(move |this, _, _, cx| this.pick(index, cx)))
        });
        let empty = self.entries.is_empty() && !self.loading;
        v_flex()
            .w(px(680.))
            .max_h(px(520.))
            .rounded_xl()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_2xl()
            .overflow_hidden()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Icon::new(IconName::Search)
                            .small()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.input).appearance(false)),
                    )
                    .when(self.loading, |this| this.child(Spinner::new().small())),
            )
            .child(
                div()
                    .id("palette-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p_1()
                    .children(rows)
                    .when(empty, |this| {
                        this.child(
                            div()
                                .p_4()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(match self.mode {
                                    PaletteMode::Search => "Type at least two characters",
                                    _ => "No matches",
                                }),
                        )
                    }),
            )
    }
}
