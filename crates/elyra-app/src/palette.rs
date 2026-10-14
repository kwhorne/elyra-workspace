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
    /// Definitions from the project's symbol index.
    Symbols,
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
    symbols: Option<Arc<Vec<elyra_index::SymbolHit>>>,
    /// Why there are no symbols (no index, no supported files).
    symbols_note: Option<String>,
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
            "Go to symbol…",
            "⌘T",
            IconName::SquareFunction,
            actions::GoToSymbol
        ),
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
            "Today's work (threads, Félagi hours, commits)",
            "",
            IconName::CalendarDays,
            actions::TodaysWork
        ),
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
            "Search threads",
            "⌥⌘F",
            IconName::TextSearch,
            actions::SearchThreads
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
        command!(
            "While you were away…",
            "",
            IconName::Coffee,
            actions::WhileAway
        ),
        command!(
            "Go back to the previous version…",
            "",
            IconName::Undo2,
            actions::GoBackVersion
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
            PaletteMode::Symbols => "Go to a function, class or type…",
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
            symbols: None,
            symbols_note: None,
            loading: false,
            generation: 0,
            _subscriptions: subscriptions,
        };
        if mode == PaletteMode::Files {
            this.load_files(cx);
        }
        if mode == PaletteMode::Symbols {
            this.load_symbols(cx);
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

    /// Every definition in the folder, from its symbol index (refreshed when
    /// stale, built the first time).
    /// Threads whose conversation matches, added below the other entries
    /// once the search (in the background) is done.
    fn search_messages(&mut self, query: String, cx: &mut Context<Self>) {
        self.generation += 1;
        if query.chars().count() < 3 {
            return;
        }
        let generation = self.generation;
        let job = crate::thread_search::search_in_background(
            query,
            elyra_core::SearchFilter::default(),
            MESSAGE_HITS * 4,
            cx,
        );
        cx.spawn(async move |this, cx| {
            let Ok(hits) = job.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return; // the query changed meanwhile
                }
                let state = this.app.read(cx);
                let mut seen = Vec::new();
                let mut found = Vec::new();
                for hit in hits {
                    if seen.contains(&hit.thread) || found.len() == MESSAGE_HITS {
                        continue;
                    }
                    seen.push(hit.thread);
                    if let Some(thread) = state.thread(hit.thread) {
                        found.push(Entry {
                            icon: IconName::TextSearch,
                            label: thread.title.clone(),
                            detail: hit.plain_snippet(),
                            hint: "message".into(),
                            target: Target::Thread(hit.thread),
                            score: -1000,
                        });
                    }
                }
                let room = MAX_ROWS.saturating_sub(this.entries.len());
                this.entries.extend(found.into_iter().take(room));
                cx.notify();
            });
        })
        .detach();
    }

    fn load_symbols(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            self.symbols_note = Some("Open a thread or project first.".into());
            return;
        };
        self.loading = true;
        let job = cx.background_executor().spawn(async move {
            let _one_at_a_time = crate::gateway::SYMBOL_LOOKUPS
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let db =
                elyra_index::default_db_path(&elyra_core::paths::data_dir().join("index"), &root);
            let mut index = elyra_index::SymbolIndex::open(&root, &db)?;
            index.refresh_if_stale(elyra_index::REFRESH_MAX_AGE)?;
            index.all_definitions(MAX_SYMBOLS)
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(symbols) if symbols.is_empty() => {
                        this.symbols_note = Some(
                            "No symbols: the folder has no Rust, TypeScript, JavaScript, Python, Go, PHP or C files."
                                .into(),
                        )
                    }
                    Ok(symbols) => this.symbols = Some(Arc::new(symbols)),
                    Err(err) => this.symbols_note = Some(format!("The symbol index failed: {err}")),
                }
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
            PaletteMode::Commands => {
                self.entries = self.command_entries(&query, cx);
                self.search_messages(query, cx);
            }
            PaletteMode::Files => {
                self.entries = self
                    .files
                    .as_ref()
                    .map(|files| file_entries(files, &query))
                    .unwrap_or_default();
            }
            PaletteMode::Search => self.run_search(query, cx),
            PaletteMode::Symbols => {
                self.entries = self
                    .symbols
                    .as_ref()
                    .map(|symbols| symbol_entries(symbols, &query))
                    .unwrap_or_default();
            }
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

/// Threads found by their conversation, shown below the other entries.
const MESSAGE_HITS: usize = 8;

/// The most definitions read for the symbol list.
const MAX_SYMBOLS: usize = 100_000;

/// Definitions whose name matches `query`, best first: an exact name, then a
/// name starting with it, then fuzzy matches; shorter names first.
fn symbol_entries(symbols: &[elyra_index::SymbolHit], query: &str) -> Vec<Entry> {
    if query.is_empty() {
        return Vec::new();
    }
    let lower = query.to_lowercase();
    let mut scored: Vec<(i64, &elyra_index::SymbolHit)> = symbols
        .iter()
        .filter_map(|symbol| {
            let mut score = fuzzy_score(&symbol.name, query)?;
            if symbol.name.to_lowercase() == lower {
                score += 1_000_000;
            }
            Some((score, symbol))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.file.cmp(&b.1.file))
            .then_with(|| a.1.line.cmp(&b.1.line))
    });
    scored
        .into_iter()
        .take(MAX_ROWS)
        .map(|(score, symbol)| {
            let place = format!("{}:{}", symbol.file, symbol.line);
            Entry {
                icon: match symbol.kind.as_str() {
                    "function" | "method" => IconName::SquareFunction,
                    "class" | "interface" | "type" | "struct" | "enum" | "trait" => IconName::Box,
                    "module" => IconName::Boxes,
                    "constant" | "variable" => IconName::Variable,
                    _ => IconName::Braces,
                },
                label: symbol.name.clone(),
                detail: match &symbol.container {
                    Some(container) => format!("{container} · {place}"),
                    None => place,
                },
                hint: symbol.kind.clone(),
                target: Target::File {
                    path: symbol.file.clone(),
                    line: Some(symbol.line as usize),
                },
                score,
            }
        })
        .collect()
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
                                    PaletteMode::Search => "Type at least two characters".into(),
                                    PaletteMode::Symbols => match &self.symbols_note {
                                        Some(note) => note.clone(),
                                        None if self.input.read(cx).value().trim().is_empty() => {
                                            "Type the name of a function, class or type".into()
                                        }
                                        None => "No matches".into(),
                                    },
                                    _ => "No matches".to_string(),
                                }),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::symbol_entries;

    fn symbol(name: &str, kind: &str, file: &str, line: u32) -> elyra_index::SymbolHit {
        elyra_index::SymbolHit {
            file: file.into(),
            language: "rust".into(),
            name: name.into(),
            kind: kind.into(),
            line,
            col: 0,
            end_line: line,
            container: (kind == "method").then(|| "Cart".to_string()),
            signature: String::new(),
        }
    }

    #[test]
    fn ranks_symbols_by_name() {
        let symbols = vec![
            symbol("CartTotals", "class", "src/totals.rs", 3),
            symbol("update_totals", "function", "src/cart.rs", 40),
            symbol("Cart", "class", "src/cart.rs", 1),
            symbol("total", "method", "src/cart.rs", 12),
        ];
        let names = |query| {
            symbol_entries(&symbols, query)
                .into_iter()
                .map(|e| e.label)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names("cart")[..2],
            ["Cart", "CartTotals"],
            "the exact name first"
        );
        assert_eq!(names("updtot"), ["update_totals"], "fuzzy");
        assert!(names("").is_empty(), "nothing until a name is typed");
        let total = &symbol_entries(&symbols, "total")[0];
        assert_eq!(
            (
                total.label.as_str(),
                total.detail.as_str(),
                total.hint.as_str()
            ),
            ("total", "Cart · src/cart.rs:12", "method")
        );
    }
}
