//! The Git panel of a thread: branch and sync, staged and unstaged changes,
//! diffs of the working tree, of individual agent turns or against another
//! branch, blame and line comments for the agent, and committing. Lines an
//! agent wrote that nobody has marked as read carry a dot until someone does.

use crate::app_state::AppState;
use crate::thread_session::ThreadSession;
use elyra_core::{ProjectId, ProviderKind};
use elyra_git::{
    Area, AreaChange, Branch, ChangeKind, DiffLine, DiffLineKind, DiffOptions, DiffSource,
    FileChange, FileDiff, SyncState,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

const MAX_DIFF_LINES: usize = 4000;
const MAX_GENERATION_DIFF: usize = 14_000;

pub enum ChangesEvent {
    /// A review comment for the agent, ready to drop into the composer.
    Comment(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    WorkingTree,
    /// Changes made during one turn: (label, checkpoint before, checkpoint after or None = now).
    Turn(String, String, Option<String>),
    Compare(String),
}

#[derive(Clone, Debug, PartialEq)]
struct Selected {
    area: Option<Area>,
    change: FileChange,
}

#[derive(Clone, Debug, PartialEq)]
struct SelectedLine {
    hunk: usize,
    line: usize,
    number: Option<u32>,
    text: String,
}

pub struct ChangesView {
    cwd: PathBuf,
    root: Option<PathBuf>,
    session: Option<WeakEntity<ThreadSession>>,
    is_repo: bool,
    branch: Option<String>,
    branches: Vec<Branch>,
    sync: SyncState,
    areas: Vec<AreaChange>,
    files: Vec<FileChange>,
    scope: Scope,
    selected: Option<Selected>,
    diff: Option<FileDiff>,
    split: bool,
    wrap: bool,
    ignore_whitespace: bool,
    loading: bool,
    busy: Option<&'static str>,
    message: Option<(String, bool)>,
    commit_input: Entity<InputState>,
    comment_input: Entity<InputState>,
    selected_line: Option<SelectedLine>,
    blame: Option<String>,
    /// The turn that wrote the selected line (when an agent did): a line
    /// about it and its thread.
    origin: Option<(String, elyra_core::ThreadId)>,
    /// Per changed file: the lines agents wrote there that nobody has marked
    /// as read ([`elyra_core::line_hash`]), and how many of them the file has now.
    unread: HashMap<String, (HashSet<i64>, usize)>,
    /// The session's `lines_recorded` when `unread` was counted.
    lines_recorded: usize,
    diff_scroll: ScrollHandle,
    change_rows: Vec<usize>,
    change_cursor: usize,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ChangesEvent> for ChangesView {}

fn kind_color(kind: ChangeKind, cx: &App) -> Hsla {
    match kind {
        ChangeKind::Added | ChangeKind::Untracked => cx.theme().success,
        ChangeKind::Deleted | ChangeKind::Conflicted => cx.theme().danger,
        ChangeKind::Renamed => cx.theme().info,
        ChangeKind::Modified => cx.theme().warning,
    }
}

/// The mark of a line an agent wrote that nobody has read.
fn unread_dot(cx: &App) -> Div {
    div()
        .size(px(6.))
        .flex_none()
        .rounded_full()
        .bg(cx.theme().warning)
}

/// How many lines of `content` are among `lines` ([`elyra_core::line_hash`]).
fn unread_in(content: &str, lines: &HashSet<i64>) -> usize {
    content
        .lines()
        .filter_map(elyra_core::line_hash)
        .filter(|hash| lines.contains(hash))
        .count()
}

impl ChangesView {
    pub fn new(
        cwd: PathBuf,
        session: Option<Entity<ThreadSession>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let commit_input = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message"));
        let comment_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Comment for the agent on this line")
        });
        let mut subscriptions = vec![
            cx.subscribe_in(
                &commit_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.commit(false, window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &comment_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.send_comment(window, cx);
                    }
                },
            ),
        ];
        if let Some(session) = &session {
            subscriptions.push(cx.observe(session, |this, session, cx| {
                // A finished turn's lines are recorded after it ends.
                let recorded = session.read(cx).lines_recorded;
                if recorded != this.lines_recorded {
                    this.lines_recorded = recorded;
                    this.count_unread(cx);
                }
                cx.notify();
            }));
        }
        let mut this = Self {
            cwd,
            root: None,
            session: session.map(|s| s.downgrade()),
            is_repo: true,
            branch: None,
            branches: Vec::new(),
            sync: SyncState::default(),
            areas: Vec::new(),
            files: Vec::new(),
            scope: Scope::WorkingTree,
            selected: None,
            diff: None,
            split: false,
            wrap: false,
            ignore_whitespace: false,
            loading: false,
            busy: None,
            message: None,
            commit_input,
            comment_input,
            selected_line: None,
            blame: None,
            origin: None,
            unread: HashMap::new(),
            lines_recorded: 0,
            diff_scroll: ScrollHandle::new(),
            change_rows: Vec::new(),
            change_cursor: 0,
            _subscriptions: subscriptions,
        };
        this.refresh(cx);
        this
    }

    pub fn set_cwd(&mut self, cwd: PathBuf, cx: &mut Context<Self>) {
        if self.cwd != cwd {
            self.cwd = cwd;
            self.selected = None;
            self.diff = None;
            self.scope = Scope::WorkingTree;
            self.refresh(cx);
        }
    }

    fn provider(&self, cx: &App) -> ProviderKind {
        self.session
            .as_ref()
            .and_then(|s| s.upgrade())
            .map(|s| s.read(cx).thread.provider)
            .unwrap_or(ProviderKind::Claude)
    }

    fn turns(&self, cx: &App) -> Vec<(String, String)> {
        self.session
            .as_ref()
            .and_then(|s| s.upgrade())
            .map(|s| s.read(cx).turn_checkpoints())
            .unwrap_or_default()
    }

    // ---- loading ------------------------------------------------------------

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let cwd = self.cwd.clone();
        let scope = self.scope.clone();
        self.loading = true;
        cx.notify();
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&cwd).ok()?;
            let branch = elyra_git::current_branch(&root);
            let branches = elyra_git::branches(&root).unwrap_or_default();
            let sync = elyra_git::sync_state(&root);
            let areas = elyra_git::status_by_area(&root);
            let files = match &scope {
                Scope::WorkingTree => Ok(Vec::new()),
                Scope::Turn(_, from, to) => {
                    let to = match to {
                        Some(to) => Ok(to.clone()),
                        None => elyra_git::checkpoint::snapshot(&root, "elyra: now"),
                    };
                    to.and_then(|to| {
                        elyra_git::changed_files(&root, &DiffSource::Range(from.clone(), to))
                    })
                }
                Scope::Compare(reference) => {
                    elyra_git::changed_files(&root, &DiffSource::Ref(reference.clone()))
                }
            };
            Some((root, branch, branches, sync, areas, files))
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                let Some((root, branch, branches, sync, areas, files)) = result else {
                    this.is_repo = false;
                    cx.notify();
                    return;
                };
                this.is_repo = true;
                this.root = Some(root);
                this.branch = branch;
                this.branches = branches;
                this.sync = sync;
                match areas {
                    Ok(areas) => this.areas = areas,
                    Err(err) => this.message = Some((format!("{err:#}"), true)),
                }
                match files {
                    Ok(files) => this.files = files,
                    Err(err) => this.message = Some((format!("{err:#}"), true)),
                }
                this.keep_selection(cx);
                this.count_unread(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The app and the project this view's repository belongs to, for the
    /// record of which lines agents wrote.
    fn provenance(&self, cx: &App) -> Option<(Entity<AppState>, ProjectId)> {
        let app = self.session.as_ref()?.upgrade()?.read(cx).app_entity()?;
        let project = crate::provenance::project_of(app.read(cx), self.root.as_ref()?)?;
        Some((app, project))
    }

    /// How many lines agents wrote in each changed file that nobody has read.
    fn count_unread(&mut self, cx: &mut Context<Self>) {
        let (Some((app, project)), Some(root)) = (self.provenance(cx), self.root.clone()) else {
            self.unread.clear();
            return;
        };
        let mut paths: Vec<String> = self
            .visible_changes()
            .into_iter()
            .map(|selected| selected.change.path)
            .collect();
        paths.sort();
        paths.dedup();
        let store = &app.read(cx).store;
        let pending: Vec<(String, HashSet<i64>)> = paths
            .into_iter()
            .filter_map(|path| {
                let lines = store.unreviewed_lines(project, &path).ok()?;
                (!lines.is_empty()).then_some((path, lines))
            })
            .collect();
        let job = cx.background_executor().spawn(async move {
            pending
                .into_iter()
                .map(|(path, lines)| {
                    let count = std::fs::read_to_string(root.join(&path))
                        .map(|content| unread_in(&content, &lines))
                        .unwrap_or(0);
                    (path, (lines, count))
                })
                .collect::<HashMap<_, _>>()
        });
        cx.spawn(async move |this, cx| {
            let unread = job.await;
            let _ = this.update(cx, |this, cx| {
                this.unread = unread;
                cx.notify();
            });
        })
        .detach();
    }

    /// Mark what agents wrote in `path` as read.
    fn mark_read(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some((app, project)) = self.provenance(cx) else {
            return;
        };
        let Some((lines, _)) = self.unread.remove(path) else {
            return;
        };
        let lines: Vec<i64> = lines.into_iter().collect();
        if let Err(err) =
            app.read(cx)
                .store
                .mark_lines_reviewed(project, path, &lines, chrono::Utc::now())
        {
            self.message = Some((format!("{err:#}"), true));
        }
        cx.notify();
    }

    /// Unread agent lines in what a commit would take now.
    fn unread_to_commit(&self) -> usize {
        let staged = self.areas.iter().any(|a| a.area == Area::Staged);
        let mut paths: Vec<&str> = self
            .areas
            .iter()
            .filter(|a| !staged || a.area == Area::Staged)
            .map(|a| a.change.path.as_str())
            .collect();
        paths.sort();
        paths.dedup();
        paths
            .into_iter()
            .filter_map(|path| self.unread.get(path))
            .map(|(_, count)| count)
            .sum()
    }

    /// Whether this diff line is one an agent wrote that nobody has read.
    fn is_unread(&self, line: &DiffLine) -> bool {
        line.kind != DiffLineKind::Removed
            && self
                .selected
                .as_ref()
                .and_then(|s| self.unread.get(&s.change.path))
                .zip(elyra_core::line_hash(&line.text))
                .is_some_and(|((lines, _), hash)| lines.contains(&hash))
    }

    fn visible_changes(&self) -> Vec<Selected> {
        match self.scope {
            Scope::WorkingTree => self
                .areas
                .iter()
                .map(|a| Selected {
                    area: Some(a.area),
                    change: a.change.clone(),
                })
                .collect(),
            _ => self
                .files
                .iter()
                .map(|change| Selected {
                    area: None,
                    change: change.clone(),
                })
                .collect(),
        }
    }

    fn keep_selection(&mut self, cx: &mut Context<Self>) {
        let visible = self.visible_changes();
        let still = self.selected.as_ref().and_then(|selected| {
            visible
                .iter()
                .find(|v| v.change.path == selected.change.path && v.area == selected.area)
                .cloned()
        });
        self.selected = still.or_else(|| visible.first().cloned());
        self.load_diff(cx);
    }

    fn select(&mut self, selected: Selected, cx: &mut Context<Self>) {
        self.selected = Some(selected);
        self.selected_line = None;
        self.blame = None;
        self.origin = None;
        self.load_diff(cx);
    }

    fn diff_source(&self, area: Option<Area>, cx: &App) -> DiffSource {
        match &self.scope {
            Scope::WorkingTree => match area {
                Some(Area::Staged) => DiffSource::Staged,
                _ => DiffSource::Unstaged,
            },
            Scope::Turn(_, from, to) => DiffSource::Range(
                from.clone(),
                to.clone().unwrap_or_else(|| {
                    let _ = cx;
                    String::new()
                }),
            ),
            Scope::Compare(reference) => DiffSource::Ref(reference.clone()),
        }
    }

    fn load_diff(&mut self, cx: &mut Context<Self>) {
        let (Some(selected), Some(root)) = (self.selected.clone(), self.root.clone()) else {
            self.diff = None;
            cx.notify();
            return;
        };
        let source = self.diff_source(selected.area, cx);
        let options = DiffOptions {
            ignore_whitespace: self.ignore_whitespace,
            context_lines: None,
        };
        let job = cx.background_executor().spawn(async move {
            // A turn that is still the latest compares against "now".
            let source = match source {
                DiffSource::Range(from, to) if to.is_empty() => {
                    DiffSource::Range(from, elyra_git::checkpoint::snapshot(&root, "elyra: now")?)
                }
                other => other,
            };
            let untracked = selected.change.kind == ChangeKind::Untracked;
            elyra_git::diff_path(&root, &selected.change.path, untracked, &source, options)
        });
        cx.spawn(async move |this, cx| {
            let diff = job.await;
            let _ = this.update(cx, |this, cx| {
                match diff {
                    Ok(diff) => this.diff = Some(diff),
                    Err(err) => {
                        this.diff = None;
                        this.message = Some((format!("{err:#}"), true));
                    }
                }
                this.change_cursor = 0;
                cx.notify();
            });
        })
        .detach();
    }

    fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.scope = scope;
        self.selected = None;
        self.diff = None;
        self.refresh(cx);
    }

    // ---- git actions ----------------------------------------------------------

    fn run_git(
        &mut self,
        label: &'static str,
        job: impl FnOnce(PathBuf) -> anyhow::Result<String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(label);
        self.message = None;
        cx.notify();
        let task = cx.background_executor().spawn(async move { job(root) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                this.message = Some(match result {
                    Ok(text) => (text, false),
                    Err(err) => (format!("{err:#}"), true),
                });
                this.refresh(cx);
            });
        })
        .detach();
    }

    fn stage(&mut self, selected: &Selected, cx: &mut Context<Self>) {
        let path = selected.change.path.clone();
        let staged = selected.area == Some(Area::Staged);
        self.run_git(
            if staged { "Unstaging…" } else { "Staging…" },
            move |root| {
                if staged {
                    elyra_git::unstage(&root, &[&path])?;
                } else {
                    elyra_git::stage(&root, &[&path])?;
                }
                Ok(String::new())
            },
            cx,
        );
    }

    fn stage_all(&mut self, stage: bool, cx: &mut Context<Self>) {
        self.run_git(
            if stage { "Staging…" } else { "Unstaging…" },
            move |root| {
                if stage {
                    elyra_git::stage_all(&root)?;
                } else {
                    elyra_git::unstage_all(&root)?;
                }
                Ok(String::new())
            },
            cx,
        );
    }

    fn discard(&mut self, selected: &Selected, window: &mut Window, cx: &mut Context<Self>) {
        let change = selected.change.clone();
        let view = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (change, view) = (change.clone(), view.clone());
            alert
                .title(format!("Discard changes to {}?", change.path))
                .description("This cannot be undone.")
                .confirm()
                .ok_text("Discard")
                .on_ok(move |_, _, cx| {
                    let change = change.clone();
                    let _ = view.update(cx, |this, cx| {
                        this.run_git(
                            "Discarding…",
                            move |root| {
                                elyra_git::discard_file(&root, &change)
                                    .map(|()| format!("Discarded {}", change.path))
                            },
                            cx,
                        )
                    });
                    true
                })
        });
    }

    fn commit(&mut self, push: bool, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.commit_input.read(cx).value().trim().to_string();
        if message.is_empty() {
            self.message = Some((
                "Write a commit message first (or generate one).".into(),
                true,
            ));
            cx.notify();
            return;
        }
        if self.areas.is_empty() {
            return;
        }
        if crate::preferences::Preferences::global(cx).conventional_titles
            && let Some(problem) =
                crate::conventional::problem(message.lines().next().unwrap_or(""))
        {
            self.message = Some((format!("First line: {problem}."), true));
            cx.notify();
            return;
        }
        let only_staged = self.areas.iter().any(|a| a.area == Area::Staged);
        self.commit_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.run_git(
            if push {
                "Committing and pushing…"
            } else {
                "Committing…"
            },
            move |root| {
                let sha = if only_staged {
                    elyra_git::commit_staged(&root, &message)?
                } else {
                    elyra_git::commit_all(&root, &message)?
                };
                if push {
                    elyra_git::push(&root)?;
                    Ok(format!("Committed {sha} and pushed"))
                } else {
                    Ok(format!("Committed {sha}"))
                }
            },
            cx,
        );
    }

    pub fn commit_and_push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(true, window, cx);
    }

    fn generate_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("Writing message…");
        cx.notify();
        let provider = self.provider(cx);
        let conventional = crate::preferences::Preferences::global(cx).conventional_titles;
        let job = cx.background_executor().spawn(async move {
            let staged = elyra_git::has_staged(&root)?;
            let diff = std::process::Command::new("git")
                .current_dir(&root)
                .args(if staged {
                    vec!["diff", "--cached", "--stat", "--patch"]
                } else {
                    vec!["diff", "HEAD", "--stat", "--patch"]
                })
                .output()?;
            let diff: String = String::from_utf8_lossy(&diff.stdout).chars().take(MAX_GENERATION_DIFF).collect();
            anyhow::ensure!(!diff.trim().is_empty(), "there are no tracked changes to describe");
            let first_line = if conventional {
                crate::conventional::RULES.to_string()
            } else {
                "First line: imperative summary, at most 72 characters.".to_string()
            };
            let prompt = format!(
                "Write a Git commit message for this diff. {first_line} \
                 Then a blank line and a short body only if it adds information. Reply with the message only, no code fences.\n\n{diff}"
            );
            elyra_provider::generate_text(provider, &root, &prompt)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                match result {
                    Ok(text) => {
                        let text =
                            if crate::preferences::Preferences::global(cx).conventional_titles {
                                crate::conventional::normalize_message(&text)
                            } else {
                                text.trim().trim_matches('`').trim().to_string()
                            };
                        this.commit_input
                            .update(cx, |input, cx| input.set_value(text, window, cx));
                    }
                    Err(err) => this.message = Some((format!("Could not generate: {err:#}"), true)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn push(&mut self, cx: &mut Context<Self>) {
        let publish = !self.sync.has_upstream;
        self.run_git(
            if publish {
                "Publishing…"
            } else {
                "Pushing…"
            },
            move |root| {
                elyra_git::push(&root)?;
                Ok(if publish {
                    "Published branch".into()
                } else {
                    "Pushed".into()
                })
            },
            cx,
        );
    }

    fn pull(&mut self, cx: &mut Context<Self>) {
        self.run_git("Pulling…", |root| elyra_git::pull(&root), cx);
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        self.run_git(
            "Fetching…",
            |root| elyra_git::fetch(&root).map(|()| "Fetched".into()),
            cx,
        );
    }

    fn switch(&mut self, name: String, cx: &mut Context<Self>) {
        self.run_git(
            "Switching…",
            move |root| match elyra_git::switch_branch(&root, &name)? {
                elyra_git::SwitchOutcome::Switched => Ok(format!("Switched to {name}")),
                elyra_git::SwitchOutcome::Stashed(_) => Ok(format!(
                    "Switched to {name}. Your uncommitted changes conflicted and were stashed (git stash list)."
                )),
            },
            cx,
        );
    }

    fn new_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("feature/my-change"));
        input.update(cx, |input, cx| input.focus(window, cx));
        let view = cx.weak_entity();
        let create = {
            let (input, view) = (input.clone(), view.clone());
            move |window: &mut Window, cx: &mut App| {
                let name = input.read(cx).value().trim().to_string();
                if name.is_empty() {
                    return;
                }
                window.close_dialog(cx);
                let _ = view.update(cx, |this, cx| {
                    this.run_git(
                        "Creating branch…",
                        move |root| {
                            elyra_git::create_branch(&root, &name)
                                .map(|()| format!("Created and switched to {name}"))
                        },
                        cx,
                    )
                });
            }
        };
        let on_enter = create.clone();
        window
            .subscribe(&input, cx, move |_, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    on_enter(window, cx);
                }
            })
            .detach();
        window.open_dialog(cx, move |dialog, _, _| {
            let create = create.clone();
            dialog
                .title("New branch")
                .w(px(420.))
                .child(
                    v_flex().gap_2().child(Input::new(&input)).child(
                        div().text_xs().child(
                            "Created from the current commit; uncommitted changes come along.",
                        ),
                    ),
                )
                .footer(
                    h_flex().justify_end().child(
                        Button::new("create-branch")
                            .small()
                            .primary()
                            .label("Create branch")
                            .on_click(move |_, window, cx| create(window, cx)),
                    ),
                )
        });
    }

    fn compare_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("main, origin/main, a tag or a commit")
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let view = cx.weak_entity();
        let compare = {
            let (input, view) = (input.clone(), view.clone());
            move |window: &mut Window, cx: &mut App| {
                let reference = input.read(cx).value().trim().to_string();
                if reference.is_empty() {
                    return;
                }
                window.close_dialog(cx);
                let _ = view.update(cx, |this, cx| this.set_scope(Scope::Compare(reference), cx));
            }
        };
        let on_enter = compare.clone();
        window
            .subscribe(&input, cx, move |_, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    on_enter(window, cx);
                }
            })
            .detach();
        window.open_dialog(cx, move |dialog, _, _| {
            let compare = compare.clone();
            dialog
                .title("Compare with…")
                .w(px(420.))
                .child(Input::new(&input))
                .footer(
                    h_flex().justify_end().child(
                        Button::new("compare-go")
                            .small()
                            .primary()
                            .label("Compare")
                            .on_click(move |_, window, cx| compare(window, cx)),
                    ),
                )
        });
    }

    // ---- line comments and blame ----------------------------------------------

    /// The turn that wrote a line with this content in the selected file.
    fn line_origin(&self, text: &str, cx: &App) -> Option<(String, elyra_core::ThreadId)> {
        let hash = elyra_core::line_hash(text)?;
        let app = self.session.as_ref()?.upgrade()?.read(cx).app_entity()?;
        let state = app.read(cx);
        let root = self.root.as_ref()?;
        let project = crate::provenance::project_of(state, root)?;
        let path = &self.selected.as_ref()?.change.path;
        let origin = state
            .store
            .line_origins(project, path, hash)
            .ok()?
            .into_iter()
            .next()?;
        let turn = crate::provenance::turn(state, &origin);
        Some((crate::provenance::short(&turn), turn.thread))
    }

    fn select_line(&mut self, hunk: usize, line: usize, cx: &mut Context<Self>) {
        let Some(diff) = &self.diff else {
            return;
        };
        let Some(diff_line) = diff.hunks.get(hunk).and_then(|h| h.lines.get(line)) else {
            return;
        };
        let selection = SelectedLine {
            hunk,
            line,
            number: diff_line.new_line.or(diff_line.old_line),
            text: diff_line.text.clone(),
        };
        if self.selected_line.as_ref() == Some(&selection) {
            self.selected_line = None;
            self.origin = None;
            cx.notify();
            return;
        }
        self.blame = None;
        self.origin = self.line_origin(&diff_line.text, cx);
        // Blame only lines that exist in the working tree.
        if let (Some(root), Some(number), Some(selected)) =
            (self.root.clone(), diff_line.new_line, &self.selected)
            && diff_line.kind != DiffLineKind::Removed
        {
            let path = selected.change.path.clone();
            let job = cx
                .background_executor()
                .spawn(async move { elyra_git::blame_line(&root, &path, number) });
            cx.spawn(async move |this, cx| {
                let blame = job.await;
                let _ = this.update(cx, |this, cx| {
                    this.blame = Some(match blame {
                        Ok(Some(commit)) => {
                            let when = chrono::DateTime::from_timestamp(commit.time, 0)
                                .map(|t| {
                                    t.with_timezone(&chrono::Local)
                                        .format("%Y-%m-%d")
                                        .to_string()
                                })
                                .unwrap_or_default();
                            format!(
                                "{} · {} · {when} · {}",
                                commit.sha, commit.author, commit.summary
                            )
                        }
                        Ok(None) => "Not committed yet".into(),
                        Err(_) => "No blame for this line".into(),
                    });
                    cx.notify();
                });
            })
            .detach();
        }
        self.selected_line = Some(selection);
        cx.notify();
    }

    fn send_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.comment_input.read(cx).value().trim().to_string();
        let (Some(line), Some(selected)) = (&self.selected_line, &self.selected) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let location = match line.number {
            Some(number) => format!("{}:{number}", selected.change.path),
            None => selected.change.path.clone(),
        };
        let comment = format!("In `{location}` (`{}`): {text}", line.text.trim());
        cx.emit(ChangesEvent::Comment(comment));
        self.comment_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.selected_line = None;
        self.message = Some(("Comment added to the composer.".into(), false));
        cx.notify();
    }

    fn jump_change(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.change_rows.is_empty() {
            return;
        }
        let len = self.change_rows.len() as isize;
        self.change_cursor = (self.change_cursor as isize + delta).rem_euclid(len) as usize;
        self.diff_scroll
            .scroll_to_item(self.change_rows[self.change_cursor]);
        cx.notify();
    }

    // ---- rendering ------------------------------------------------------------

    fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        let branch = self.branch.clone().unwrap_or_else(|| "—".into());
        let branches = self.branches.clone();
        let view = cx.weak_entity();
        let (switch_view, new_view) = (view.clone(), view.clone());
        let sync = self.sync;
        h_flex()
            .px_2()
            .py_1()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("branch")
                    .ghost()
                    .xsmall()
                    .icon(IconName::GitBranch)
                    .label(branch)
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        menu = menu.scrollable(true).max_h(px(360.));
                        for b in &branches {
                            let (view, name) = (switch_view.clone(), b.name.clone());
                            menu = menu.item(
                                PopupMenuItem::new(b.name.clone())
                                    .checked(b.current)
                                    .on_click(move |_, _, cx| {
                                        let name = name.clone();
                                        let _ = view.update(cx, |this, cx| this.switch(name, cx));
                                    }),
                            );
                        }
                        let view = new_view.clone();
                        menu.separator().item(
                            PopupMenuItem::new("New branch…")
                                .icon(IconName::Plus)
                                .on_click(move |_, window, cx| {
                                    let _ = view.update(cx, |this, cx| this.new_branch(window, cx));
                                }),
                        )
                    }),
            )
            .when(
                sync.has_upstream && (sync.ahead > 0 || sync.behind > 0),
                |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("↓{} ↑{}", sync.behind, sync.ahead)),
                    )
                },
            )
            .child(div().flex_1())
            .when(self.loading || self.busy.is_some(), |this| {
                this.child(Spinner::new().xsmall())
            })
            .child(
                Button::new("fetch")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RefreshCw)
                    .tooltip("Fetch and refresh")
                    .on_click(cx.listener(|this, _, _, cx| this.fetch(cx))),
            )
            .when(sync.behind > 0, |this| {
                this.child(
                    Button::new("pull")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Download)
                        .tooltip("Pull (fast-forward)")
                        .on_click(cx.listener(|this, _, _, cx| this.pull(cx))),
                )
            })
            .child(
                Button::new("push")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Upload)
                    .tooltip(if sync.has_upstream {
                        "Push"
                    } else {
                        "Publish branch"
                    })
                    .disabled(self.busy.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.push(cx))),
            )
            .into_any_element()
    }

    fn render_scope(&self, cx: &Context<Self>) -> AnyElement {
        let label: SharedString = match &self.scope {
            Scope::WorkingTree => "Working tree".into(),
            Scope::Turn(label, ..) => label.clone().into(),
            Scope::Compare(reference) => format!("Compared with {reference}").into(),
        };
        let turns = self.turns(cx);
        let view = cx.weak_entity();
        let current = self.scope.clone();
        let toggle = |id: &'static str, icon: IconName, tip: &'static str, on: bool| {
            Button::new(id)
                .xsmall()
                .icon(icon)
                .tooltip(tip)
                .when(on, |b| b.primary())
                .when(!on, |b| b.ghost())
        };
        h_flex()
            .px_2()
            .py_1()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("scope")
                    .ghost()
                    .xsmall()
                    .icon(IconName::GitCompareArrows)
                    .label(label)
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        let wt = view.clone();
                        menu = menu.scrollable(true).max_h(px(380.)).item(
                            PopupMenuItem::new("Working tree")
                                .checked(current == Scope::WorkingTree)
                                .on_click(move |_, _, cx| {
                                    let _ = wt.update(cx, |this, cx| {
                                        this.set_scope(Scope::WorkingTree, cx)
                                    });
                                }),
                        );
                        if !turns.is_empty() {
                            menu = menu.separator().label("Agent turns");
                        }
                        for (index, (label, sha)) in turns.iter().enumerate().rev() {
                            let next = turns.get(index + 1).map(|(_, sha)| sha.clone());
                            let scope = Scope::Turn(label.clone(), sha.clone(), next);
                            let checked =
                                matches!(&current, Scope::Turn(_, from, _) if from == sha);
                            let view = view.clone();
                            menu = menu.item(
                                PopupMenuItem::new(label.clone()).checked(checked).on_click(
                                    move |_, _, cx| {
                                        let scope = scope.clone();
                                        let _ =
                                            view.update(cx, |this, cx| this.set_scope(scope, cx));
                                    },
                                ),
                            );
                        }
                        let compare = view.clone();
                        menu.separator().item(
                            PopupMenuItem::new("Compare with branch or commit…").on_click(
                                move |_, window, cx| {
                                    let _ = compare
                                        .update(cx, |this, cx| this.compare_dialog(window, cx));
                                },
                            ),
                        )
                    }),
            )
            .child(div().flex_1())
            .child(
                toggle("split", IconName::Columns2, "Side by side", self.split).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.split = !this.split;
                        cx.notify();
                    }),
                ),
            )
            .child(
                toggle("wrap", IconName::TextWrap, "Wrap lines", self.wrap).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.wrap = !this.wrap;
                        cx.notify();
                    },
                )),
            )
            .child(
                toggle(
                    "whitespace",
                    IconName::Space,
                    "Ignore whitespace",
                    self.ignore_whitespace,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ignore_whitespace = !this.ignore_whitespace;
                    this.load_diff(cx);
                })),
            )
            .into_any_element()
    }

    fn file_row(&self, index: usize, selected: Selected, cx: &Context<Self>) -> AnyElement {
        let is_selected = self.selected.as_ref() == Some(&selected);
        let change = &selected.change;
        let (dir, name) = match change.path.rsplit_once('/') {
            Some((dir, name)) => (format!("{dir}/"), name.to_string()),
            None => (String::new(), change.path.clone()),
        };
        let in_tree = self.scope == Scope::WorkingTree;
        let staged = selected.area == Some(Area::Staged);
        let (select, stage, discard) = (selected.clone(), selected.clone(), selected.clone());
        h_flex()
            .id(("file", index))
            .group("file-row")
            .w_full()
            .px_2()
            .py_0p5()
            .gap_2()
            .rounded_sm()
            .cursor_pointer()
            .text_sm()
            .when(is_selected, |this| this.bg(cx.theme().accent))
            .hover(|this| this.bg(cx.theme().muted))
            .child(
                div()
                    .w(px(14.))
                    .flex_none()
                    .font_weight(FontWeight::BOLD)
                    .text_xs()
                    .text_color(kind_color(change.kind, cx))
                    .child(change.kind.letter()),
            )
            .child(div().flex_none().child(name))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(dir),
            )
            .when_some(
                self.unread
                    .get(&change.path)
                    .map(|(_, count)| *count)
                    .filter(|count| *count > 0),
                |this, count| {
                    this.child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .text_xs()
                            .text_color(cx.theme().warning)
                            .child(unread_dot(cx))
                            .child(format!("{count} unread")),
                    )
                },
            )
            .when(in_tree, |this| {
                this.child(
                    h_flex()
                        .invisible()
                        .group_hover("file-row", |s| s.visible())
                        .when(!staged, |this| {
                            this.child(
                                Button::new(("discard", index))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Undo2)
                                    .tooltip("Discard changes")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.discard(&discard, window, cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new(("stage", index))
                                .ghost()
                                .xsmall()
                                .icon(if staged {
                                    IconName::Minus
                                } else {
                                    IconName::Plus
                                })
                                .tooltip(if staged { "Unstage" } else { "Stage" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.stage(&stage, cx)
                                })),
                        ),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| this.select(select.clone(), cx)))
            .into_any_element()
    }

    fn render_files(&self, cx: &Context<Self>) -> AnyElement {
        let visible = self.visible_changes();
        let mut list = v_flex().p_1();
        if self.scope == Scope::WorkingTree {
            let section = |title: &'static str, count: usize, stage: bool| {
                h_flex()
                    .px_2()
                    .pt_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child(div().flex_1().child(format!("{title} ({count})")))
                    .child(
                        Button::new(if stage { "stage-all" } else { "unstage-all" })
                            .ghost()
                            .xsmall()
                            .label(if stage { "Stage all" } else { "Unstage all" })
                            .on_click(cx.listener(move |this, _, _, cx| this.stage_all(stage, cx))),
                    )
            };
            let staged: Vec<_> = visible
                .iter()
                .enumerate()
                .filter(|(_, s)| s.area == Some(Area::Staged))
                .collect();
            let unstaged: Vec<_> = visible
                .iter()
                .enumerate()
                .filter(|(_, s)| s.area != Some(Area::Staged))
                .collect();
            if !staged.is_empty() {
                list = list.child(section("STAGED", staged.len(), false)).children(
                    staged
                        .into_iter()
                        .map(|(i, s)| self.file_row(i, s.clone(), cx)),
                );
            }
            if !unstaged.is_empty() {
                list = list
                    .child(section("CHANGES", unstaged.len(), true))
                    .children(
                        unstaged
                            .into_iter()
                            .map(|(i, s)| self.file_row(i, s.clone(), cx)),
                    );
            }
        } else {
            list = list.children(
                visible
                    .into_iter()
                    .enumerate()
                    .map(|(i, s)| self.file_row(i, s, cx)),
            );
        }
        div()
            .id("files")
            .max_h(relative(0.35))
            .flex_none()
            .overflow_y_scroll()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(list)
            .into_any_element()
    }

    fn line_cells(&self, line: &DiffLine, cx: &App) -> (Option<Hsla>, &'static str) {
        match line.kind {
            DiffLineKind::Added => (Some(cx.theme().success.opacity(0.12)), "+"),
            DiffLineKind::Removed => (Some(cx.theme().danger.opacity(0.12)), "-"),
            DiffLineKind::Context => (None, " "),
        }
    }

    fn code(&self, text: &str) -> Div {
        let text = if text.is_empty() {
            " ".to_string()
        } else {
            text.replace('\t', "    ")
        };
        div()
            .flex_1()
            .min_w_0()
            .when(self.wrap, |this| this.whitespace_normal())
            .when(!self.wrap, |this| this.whitespace_nowrap())
            .child(text)
    }

    /// Rows of the diff, plus the row index where each change block starts.
    fn diff_rows(&self, cx: &Context<Self>) -> (Vec<AnyElement>, Vec<usize>, bool) {
        let Some(diff) = &self.diff else {
            return (Vec::new(), Vec::new(), false);
        };
        let gutter = cx.theme().muted_foreground.opacity(0.7);
        let number = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut change_rows = Vec::new();
        let mut emitted = 0usize;
        let mut truncated = false;
        let selected_line = self.selected_line.clone();
        for (h, hunk) in diff.hunks.iter().enumerate() {
            rows.push(
                div()
                    .id(("hunk", h))
                    .px_2()
                    .py_0p5()
                    .bg(cx.theme().muted)
                    .text_color(cx.theme().muted_foreground)
                    .child(hunk.header.clone())
                    .into_any_element(),
            );
            let mut previous_changed = false;
            let mut i = 0;
            while i < hunk.lines.len() {
                if emitted >= MAX_DIFF_LINES {
                    truncated = true;
                    break;
                }
                let changed = hunk.lines[i].kind != DiffLineKind::Context;
                if changed && !previous_changed {
                    change_rows.push(rows.len());
                }
                previous_changed = changed;
                if self.split && changed {
                    // Pair a block of removals with the following additions.
                    let start = i;
                    while i < hunk.lines.len() && hunk.lines[i].kind == DiffLineKind::Removed {
                        i += 1;
                    }
                    let removed = &hunk.lines[start..i];
                    let added_start = i;
                    while i < hunk.lines.len() && hunk.lines[i].kind == DiffLineKind::Added {
                        i += 1;
                    }
                    let added = &hunk.lines[added_start..i];
                    for pair in 0..removed.len().max(added.len()) {
                        let side = |line: Option<&DiffLine>, offset: usize| {
                            let (bg, _) =
                                line.map(|l| self.line_cells(l, cx)).unwrap_or((None, " "));
                            let unread = line.is_some_and(|l| self.is_unread(l));
                            let index = offset + pair;
                            h_flex()
                                .id(SharedString::from(format!(
                                    "split-{h}-{index}-{}",
                                    line.is_some()
                                )))
                                .flex_1()
                                .min_w_0()
                                .when_some(bg, |this, bg| this.bg(bg))
                                .child(
                                    div()
                                        .w(px(40.))
                                        .flex_none()
                                        .pr_1()
                                        .text_right()
                                        .text_color(gutter)
                                        .child(
                                            line.map(|l| number(l.new_line.or(l.old_line)))
                                                .unwrap_or_default(),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .w(px(10.))
                                        .flex_none()
                                        .when(unread, |this| this.child(unread_dot(cx))),
                                )
                                .child(self.code(line.map(|l| l.text.as_str()).unwrap_or("")))
                                .when(line.is_some(), |this| {
                                    this.cursor_pointer().on_click(cx.listener(
                                        move |this, _, _, cx| this.select_line(h, index, cx),
                                    ))
                                })
                        };
                        rows.push(
                            h_flex()
                                .w_full()
                                .child(side(removed.get(pair), start))
                                .child(div().w(px(1.)).h_full().bg(cx.theme().border))
                                .child(side(added.get(pair), added_start))
                                .into_any_element(),
                        );
                        emitted += 1;
                    }
                    continue;
                }
                let line = &hunk.lines[i];
                let (bg, sign) = self.line_cells(line, cx);
                let unread = self.is_unread(line);
                let index = i;
                let is_selected = selected_line
                    .as_ref()
                    .is_some_and(|s| s.hunk == h && s.line == index);
                rows.push(
                    h_flex()
                        .id(SharedString::from(format!("line-{h}-{i}")))
                        .w_full()
                        .cursor_pointer()
                        .when_some(bg, |this, bg| this.bg(bg))
                        .when(is_selected, |this| {
                            this.bg(cx.theme().selection.opacity(0.35))
                        })
                        .hover(|this| this.bg(cx.theme().muted.opacity(0.6)))
                        .child(
                            div()
                                .w(px(40.))
                                .flex_none()
                                .pr_1()
                                .text_right()
                                .text_color(gutter)
                                .child(number(line.old_line)),
                        )
                        .child(
                            div()
                                .w(px(40.))
                                .flex_none()
                                .pr_1()
                                .text_right()
                                .text_color(gutter)
                                .child(number(line.new_line)),
                        )
                        .child(
                            h_flex()
                                .w(px(14.))
                                .flex_none()
                                .when(unread, |this| this.child(unread_dot(cx)))
                                .when(!unread, |this| this.child(sign)),
                        )
                        .child(self.code(&line.text))
                        .on_click(cx.listener(move |this, _, _, cx| this.select_line(h, index, cx)))
                        .into_any_element(),
                );
                emitted += 1;
                if is_selected {
                    rows.push(self.render_line_tools(cx));
                }
                i += 1;
            }
            if truncated {
                break;
            }
        }
        (rows, change_rows, truncated)
    }

    fn render_line_tools(&self, cx: &Context<Self>) -> AnyElement {
        v_flex()
            .mx_2()
            .my_1()
            .p_2()
            .gap_1()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .font_family(cx.theme().font_family.clone())
            .when_some(self.blame.clone(), |this, blame| {
                this.child(
                    h_flex()
                        .gap_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(Icon::new(IconName::GitCommitHorizontal).xsmall())
                        .child(blame),
                )
            })
            .when_some(self.origin.clone(), |this, (origin, thread)| {
                this.child(
                    h_flex()
                        .id("line-origin")
                        .gap_1()
                        .text_xs()
                        .text_color(cx.theme().link)
                        .cursor_pointer()
                        .child(Icon::new(IconName::MessageSquare).xsmall())
                        .child(format!("Written by the agent in {origin}"))
                        .on_click(move |_, window, cx| {
                            if let Some(hub) = cx.try_global::<crate::browser_tools::BrowserHub>() {
                                let workspace = hub.workspace.clone();
                                let _ = workspace
                                    .update(cx, |this, cx| this.activate(thread, window, cx));
                            }
                        }),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.comment_input).small()),
                    )
                    .child(
                        Button::new("send-comment")
                            .small()
                            .primary()
                            .label("Add to chat")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.send_comment(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_diff(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(diff) = self.diff.clone() else {
            let empty = if self.visible_changes().is_empty() {
                "No changes"
            } else {
                "Select a file"
            };
            return div()
                .p_4()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(empty)
                .into_any_element();
        };
        if diff.binary {
            return div()
                .p_4()
                .text_sm()
                .child("Binary file")
                .into_any_element();
        }
        let (rows, change_rows, truncated) = self.diff_rows(cx);
        self.change_rows = change_rows;
        let changes = self.change_rows.len();
        let path = diff.path.clone();
        let unread = self
            .selected
            .as_ref()
            .and_then(|s| self.unread.get(&s.change.path))
            .map(|(_, count)| *count)
            .filter(|count| *count > 0)
            .zip(self.selected.as_ref().map(|s| s.change.path.clone()));
        let copy_text = diff
            .hunks
            .iter()
            .flat_map(|h| {
                std::iter::once(h.header.clone()).chain(h.lines.iter().map(|l| {
                    let sign = match l.kind {
                        DiffLineKind::Added => "+",
                        DiffLineKind::Removed => "-",
                        DiffLineKind::Context => " ",
                    };
                    format!("{sign}{}", l.text)
                }))
            })
            .collect::<Vec<_>>()
            .join("\n");
        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .text_xs()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(path),
                    )
                    .when_some(unread, |this, (count, path)| {
                        this.child(
                            Button::new("mark-read")
                                .ghost()
                                .xsmall()
                                .text_color(cx.theme().warning)
                                .icon(IconName::Eye)
                                .label(format!("{count} unread · Mark as read"))
                                .tooltip(
                                    "The dotted lines were written by an agent and nobody has \
                                     marked them as read. Mark everything agents wrote in \
                                     this file as read.",
                                )
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.mark_read(&path, cx)),
                                ),
                        )
                    })
                    .child(
                        div()
                            .text_color(cx.theme().success)
                            .child(format!("+{}", diff.additions())),
                    )
                    .child(
                        div()
                            .text_color(cx.theme().danger)
                            .child(format!("−{}", diff.deletions())),
                    )
                    .child(
                        Button::new("prev-change")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ChevronUp)
                            .tooltip("Previous change (⌥↑)")
                            .disabled(changes == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.jump_change(-1, cx))),
                    )
                    .child(
                        Button::new("next-change")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ChevronDown)
                            .tooltip("Next change (⌥↓)")
                            .disabled(changes == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.jump_change(1, cx))),
                    )
                    .child(
                        Button::new("copy-diff")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Copy)
                            .tooltip("Copy diff")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()))
                            }),
                    ),
            )
            .when(truncated, |this| {
                this.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(cx.theme().warning)
                        .child(format!(
                            "Large diff: showing the first {MAX_DIFF_LINES} lines."
                        )),
                )
            })
            .child(
                div()
                    .id("diff")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .track_scroll(&self.diff_scroll)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .children(rows),
            )
            .into_any_element()
    }

    fn render_commit(&self, cx: &Context<Self>) -> AnyElement {
        let has_changes = !self.areas.is_empty();
        let staged = self.areas.iter().any(|a| a.area == Area::Staged);
        let unread = self.unread_to_commit();
        v_flex()
            .p_2()
            .gap_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .when_some(self.message.clone(), |this, (message, is_error)| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(if is_error {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(message),
                )
            })
            .when(self.scope == Scope::WorkingTree && unread > 0, |this| {
                this.child(
                    h_flex()
                        .gap_1()
                        .text_xs()
                        .text_color(cx.theme().warning)
                        .child(unread_dot(cx))
                        .child(if unread == 1 {
                            "1 line an agent wrote in this commit hasn't been read.".to_string()
                        } else {
                            format!("{unread} lines agents wrote in this commit haven't been read.")
                        }),
                )
            })
            .when(self.scope == Scope::WorkingTree, |this| {
                this.child(
                    h_flex()
                        .gap_1()
                        .child(div().flex_1().child(Input::new(&self.commit_input).small()))
                        .child(
                            Button::new("generate-message")
                                .ghost()
                                .small()
                                .icon(IconName::Sparkles)
                                .tooltip("Write a commit message with the agent")
                                .disabled(!has_changes || self.busy.is_some())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.generate_message(window, cx)
                                })),
                        )
                        .child(
                            Button::new("commit")
                                .small()
                                .primary()
                                .icon(IconName::GitCommitHorizontal)
                                .label(self.busy.unwrap_or(if staged {
                                    "Commit staged"
                                } else {
                                    "Commit all"
                                }))
                                .loading(self.busy.is_some())
                                .disabled(!has_changes || self.busy.is_some())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.commit(false, window, cx)
                                })),
                        )
                        .child(
                            Button::new("commit-push")
                                .small()
                                .icon(IconName::Upload)
                                .tooltip("Commit and push (⌃⌘P)")
                                .disabled(!has_changes || self.busy.is_some())
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.commit(true, window, cx)
                                    }),
                                ),
                        ),
                )
            })
            .into_any_element()
    }
}

impl Render for ChangesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.is_repo {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::GitBranch).large())
                .child("This folder is not a Git repository")
                .into_any_element();
        }
        let header = self.render_header(cx);
        let scope = self.render_scope(cx);
        let files = self.render_files(cx);
        let diff = self.render_diff(cx);
        let commit = self.render_commit(cx);
        v_flex()
            .size_full()
            .key_context("Changes")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let keystroke = &event.keystroke;
                if keystroke.modifiers.alt && !keystroke.modifiers.platform {
                    match keystroke.key.as_str() {
                        "down" => this.jump_change(1, cx),
                        "up" => this.jump_change(-1, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                }
            }))
            .child(header)
            .child(scope)
            .child(files)
            .child(diff)
            .child(commit)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::unread_in;
    use std::collections::HashSet;

    #[test]
    fn counts_the_unread_lines_a_file_still_has() {
        let hash = |line| elyra_core::line_hash(line).unwrap();
        let unread = HashSet::from([hash("let total = sum(items);"), hash("gone();")]);
        let content = "fn total() {\n    let total = sum(items);\n    total\n}\n";
        assert_eq!(
            unread_in(content, &unread),
            1,
            "only lines still in the file"
        );
        assert_eq!(unread_in(content, &HashSet::new()), 0);
    }
}
