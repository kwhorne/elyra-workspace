//! Modal dialogs: rename thread, project appearance, delete thread and the
//! archived-threads list.

use crate::app_state::AppState;
use crate::workspace::Workspace;
use elyra_core::{Environment, ProjectId, ThreadId};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Project accent colours offered in the appearance dialog.
pub const PROJECT_COLORS: &[&str] = &[
    "#ef4444", "#f97316", "#eab308", "#22c55e", "#14b8a6", "#3b82f6", "#8b5cf6", "#ec4899",
];

fn primary_button(id: &'static str, label: &'static str) -> Button {
    Button::new(id).primary().small().label(label)
}

pub fn rename_thread(app: Entity<AppState>, id: ThreadId, window: &mut Window, cx: &mut App) {
    let Some(title) = app.read(cx).thread(id).map(|t| t.title.clone()) else {
        return;
    };
    let input = cx.new(|cx| {
        let mut state = InputState::new(window, cx).placeholder("Thread title");
        state.set_value(title, window, cx);
        state
    });
    let save = {
        let (app, input) = (app.clone(), input.clone());
        move |window: &mut Window, cx: &mut App| {
            let title = input.read(cx).value().trim().to_string();
            if !title.is_empty() {
                app.update(cx, |app, cx| {
                    app.update_thread(id, |t| t.title = title.clone(), cx)
                });
            }
            window.close_dialog(cx);
        }
    };
    let save_on_enter = save.clone();
    window
        .subscribe(&input, cx, move |_, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                save_on_enter(window, cx);
            }
        })
        .detach();
    input.update(cx, |input, cx| input.focus(window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        let save = save.clone();
        dialog
            .title("Rename thread")
            .w(px(420.))
            .child(Input::new(&input))
            .footer(
                h_flex().justify_end().gap_2().child(
                    primary_button("rename-save", "Save")
                        .on_click(move |_, window, cx| save(window, cx)),
                ),
            )
    });
}

/// Name, emoji and accent colour of a project.
pub struct ProjectForm {
    name: Entity<InputState>,
    icon: Entity<InputState>,
    space: Entity<InputState>,
    color: Option<String>,
}

impl Render for ProjectForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let swatches = PROJECT_COLORS.iter().map(|color| {
            let selected = self.color.as_deref() == Some(*color);
            let value = color.to_string();
            div()
                .id(SharedString::from(format!("swatch-{color}")))
                .size(px(22.))
                .rounded_full()
                .cursor_pointer()
                .bg(rgb(u32::from_str_radix(&color[1..], 16).unwrap_or(0)))
                .border_2()
                .border_color(if selected {
                    cx.theme().foreground
                } else {
                    transparent_black()
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.color = if this.color.as_deref() == Some(value.as_str()) {
                        None
                    } else {
                        Some(value.clone())
                    };
                    cx.notify();
                }))
        });
        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Name"))
                    .child(Input::new(&self.name)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Emoji"))
                    .child(Input::new(&self.icon).w(px(120.))),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Space"))
                    .child(Input::new(&self.space))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Group projects (e.g. Work, Personal) and filter the sidebar by space."),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Colour"))
                    .child(h_flex().gap_2().children(swatches)),
            )
    }
}

pub fn edit_project(app: Entity<AppState>, id: ProjectId, window: &mut Window, cx: &mut App) {
    let Some(project) = app.read(cx).project(id).cloned() else {
        return;
    };
    let form = cx.new(|cx| ProjectForm {
        name: cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Project name");
            state.set_value(project.name.clone(), window, cx);
            state
        }),
        icon: cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("📁");
            if let Some(icon) = &project.icon {
                state.set_value(icon.clone(), window, cx);
            }
            state
        }),
        space: cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("No space");
            if let Some(space) = &project.space {
                state.set_value(space.clone(), window, cx);
            }
            state
        }),
        color: project.color.clone(),
    });
    window.open_dialog(cx, move |dialog, _, _| {
        let (app, form, project) = (app.clone(), form.clone(), project.clone());
        dialog
            .title("Project")
            .w(px(420.))
            .child(form.clone())
            .footer(h_flex().justify_end().gap_2().child(
                primary_button("project-save", "Save").on_click(move |_, window, cx| {
                    let form = form.read(cx);
                    let name = form.name.read(cx).value().trim().to_string();
                    let icon: String = form.icon.read(cx).value().trim().chars().take(4).collect();
                    let mut updated = project.clone();
                    if !name.is_empty() {
                        updated.name = name;
                    }
                    updated.icon = Some(icon).filter(|i| !i.is_empty());
                    updated.color = form.color.clone();
                    let space = form.space.read(cx).value().trim().to_string();
                    updated.space = Some(space).filter(|s| !s.is_empty());
                    app.update(cx, |app, cx| app.update_project(updated, cx));
                    window.close_dialog(cx);
                }),
            ))
    });
}

pub fn delete_thread(
    workspace: WeakEntity<Workspace>,
    app: Entity<AppState>,
    id: ThreadId,
    window: &mut Window,
    cx: &mut App,
) {
    let thread = app.read(cx).thread(id).cloned().or_else(|| {
        app.read(cx)
            .archived_threads()
            .into_iter()
            .find(|t| t.id == id)
    });
    let Some(thread) = thread else {
        return;
    };
    let worktree = match &thread.environment {
        Environment::Worktree { path, .. } => Some(path.display().to_string()),
        Environment::Local => None,
    };
    let title = thread.title.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let run = {
            let (workspace, app) = (workspace.clone(), app.clone());
            move |remove_worktree: bool, window: &mut Window, cx: &mut App| {
                window.close_dialog(cx);
                let _ = workspace.update(cx, |this, cx| this.forget_thread(id, cx));
                if let Err(err) =
                    app.update(cx, |app, cx| app.delete_thread(id, remove_worktree, cx))
                {
                    window.push_notification(Notification::error(format!("{err:#}")), cx);
                }
            }
        };
        let (delete, delete_all) = (run.clone(), run);
        dialog
            .title("Delete thread?")
            .w(px(460.))
            .child(
                v_flex()
                    .gap_2()
                    .text_sm()
                    .child(format!(
                        "\u{201c}{title}\u{201d} and its transcript will be deleted permanently."
                    ))
                    .when_some(worktree.clone(), |this, path| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("Its worktree is at {path}")),
                        )
                    }),
            )
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("delete-cancel")
                            .small()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .when(worktree.is_some(), |this| {
                        this.child(
                            Button::new("delete-with-worktree")
                                .small()
                                .danger()
                                .label("Delete with worktree")
                                .on_click(move |_, window, cx| delete_all(true, window, cx)),
                        )
                    })
                    .child(
                        Button::new("delete-confirm")
                            .small()
                            .danger()
                            .label("Delete")
                            .on_click(move |_, window, cx| delete(false, window, cx)),
                    ),
            )
    });
}

/// Archived threads with restore and delete.
pub struct ArchivedList {
    app: Entity<AppState>,
    workspace: WeakEntity<Workspace>,
}

impl Render for ArchivedList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let threads = self.app.read(cx).archived_threads();
        if threads.is_empty() {
            return div()
                .py_6()
                .text_sm()
                .text_center()
                .text_color(cx.theme().muted_foreground)
                .child("No archived threads")
                .into_any_element();
        }
        let rows = threads.into_iter().map(|thread| {
            let id = thread.id;
            let project = self
                .app
                .read(cx)
                .project(thread.project_id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            h_flex()
                .gap_2()
                .py_1()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    Icon::new(IconName::Archive)
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(thread.title.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(project),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!("restore-{id}")))
                        .xsmall()
                        .label("Restore")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.app.update(cx, |app, cx| app.restore_thread(id, cx));
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("delete-archived-{id}")))
                        .xsmall()
                        .ghost()
                        .icon(IconName::Trash)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let (workspace, app) = (this.workspace.clone(), this.app.clone());
                            window.close_dialog(cx);
                            delete_thread(workspace, app, id, window, cx);
                        })),
                )
        });
        v_flex()
            .id("archived-list")
            .max_h(px(420.))
            .overflow_y_scroll()
            .children(rows)
            .into_any_element()
    }
}

pub fn archived_threads(
    workspace: WeakEntity<Workspace>,
    app: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) {
    let list = cx.new(|cx| {
        cx.observe(&app, |_, _, cx| cx.notify()).detach();
        ArchivedList { app, workspace }
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Archived threads")
            .w(px(520.))
            .child(list.clone())
    });
}

/// One managed worktree on disk and the thread that uses it, if any.
struct WorktreeEntry {
    path: std::path::PathBuf,
    repo: Option<std::path::PathBuf>,
    branch: Option<String>,
    thread: Option<String>,
}

pub struct WorktreeList {
    app: Entity<AppState>,
    entries: Vec<WorktreeEntry>,
    message: Option<String>,
}

impl WorktreeList {
    fn load(&mut self, cx: &mut Context<Self>) {
        let state = self.app.read(cx);
        let threads: Vec<_> = state
            .threads
            .iter()
            .cloned()
            .chain(state.archived_threads())
            .collect();
        let root = elyra_core::paths::worktrees_dir();
        let mut entries = Vec::new();
        for project_dir in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            for dir in std::fs::read_dir(project_dir.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let path = dir.path();
                if !path.is_dir() {
                    continue;
                }
                let thread = threads.iter().find(|t| {
                    matches!(&t.environment, Environment::Worktree { path: p, .. } if *p == path)
                });
                let repo = thread
                    .and_then(|t| state.project(t.project_id))
                    .map(|p| p.path.clone())
                    .or_else(|| {
                        // `<repo>/.git/worktrees/<name>` → `<repo>`
                        let common = std::process::Command::new("git")
                            .current_dir(&path)
                            .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
                            .output()
                            .ok()?;
                        let common = std::path::PathBuf::from(
                            String::from_utf8_lossy(&common.stdout).trim(),
                        );
                        common.parent().map(|p| p.to_path_buf())
                    });
                entries.push(WorktreeEntry {
                    branch: elyra_git::current_branch(&path),
                    thread: thread.map(|t| {
                        if t.archived {
                            format!("{} (archived)", t.title)
                        } else {
                            t.title.clone()
                        }
                    }),
                    path,
                    repo,
                });
            }
        }
        self.entries = entries;
        cx.notify();
    }
}

impl Render for WorktreeList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.entries.iter().enumerate().map(|(index, entry)| {
            let (reveal, remove_path, repo) =
                (entry.path.clone(), entry.path.clone(), entry.repo.clone());
            h_flex()
                .gap_2()
                .py_1()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    Icon::new(IconName::GitBranch)
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .child(entry.branch.clone().unwrap_or_else(|| "(detached)".into())),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(entry.thread.clone().unwrap_or_else(|| "No thread".into())),
                        ),
                )
                .child(
                    Button::new(("reveal-worktree", index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::FolderOpen)
                        .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
                )
                .child(
                    Button::new(("remove-worktree", index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Trash)
                        .tooltip("Remove worktree (its branch is kept)")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            // A worktree Grove runs goes with its site and database copy.
                            let result = match (crate::grove::try_at(&remove_path), &repo) {
                                (Some(record), _) => crate::grove::end_try(&record),
                                (None, Some(repo)) => {
                                    elyra_git::remove_worktree(repo, &remove_path)
                                }
                                (None, None) => {
                                    std::fs::remove_dir_all(&remove_path).map_err(Into::into)
                                }
                            };
                            this.message = Some(match result {
                                Ok(()) => format!("Removed {}", remove_path.display()),
                                Err(err) => format!("{err:#}"),
                            });
                            this.load(cx);
                        })),
                )
        });
        v_flex()
            .gap_2()
            .when_some(self.message.clone(), |this, message| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(message),
                )
            })
            .child(
                v_flex()
                    .id("worktree-list")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .children(rows)
                    .when(self.entries.is_empty(), |this| {
                        this.child(
                            div()
                                .py_6()
                                .text_sm()
                                .text_center()
                                .text_color(cx.theme().muted_foreground)
                                .child("No managed worktrees"),
                        )
                    }),
            )
    }
}

pub fn worktrees(app: Entity<AppState>, window: &mut Window, cx: &mut App) {
    let list = cx.new(|cx| {
        let mut list = WorktreeList {
            app,
            entries: Vec::new(),
            message: None,
        };
        list.load(cx);
        list
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Managed worktrees")
            .w(px(560.))
            .child(list.clone())
    });
}

/// Pick Claude Code sessions to import as threads.
pub struct ImportList {
    app: Entity<AppState>,
    workspace: WeakEntity<Workspace>,
    /// Claude Code or Codex.
    source: elyra_core::ProviderKind,
    error: Option<String>,
    sessions: Option<Vec<elyra_provider::claude_history::SessionSummary>>,
    selected: std::collections::HashSet<String>,
    filter: Entity<InputState>,
    importing: bool,
}

impl ImportList {
    /// The Codex executable from provider settings or PATH.
    fn codex(cx: &App) -> Option<std::path::PathBuf> {
        crate::preferences::Preferences::global(cx)
            .launch(elyra_core::ProviderKind::Codex, None)
            .executable
            .or_else(elyra_provider::codex::find_executable)
    }

    /// (Re)load the session list for the current source.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.sessions = None;
        self.error = None;
        self.selected.clear();
        let source = self.source;
        let codex = Self::codex(cx);
        let job = cx.background_executor().spawn(async move {
            match source {
                elyra_core::ProviderKind::Codex => match codex {
                    Some(codex) => elyra_provider::codex_history::list_sessions(&codex, 200)
                        .map_err(|e| format!("{e:#}")),
                    None => Err("Codex (`codex`) is not installed.".to_string()),
                },
                _ => Ok(elyra_provider::claude_history::projects_dir()
                    .map(|root| elyra_provider::claude_history::list_sessions(&root, 300))
                    .unwrap_or_default()),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this: &mut ImportList, cx| {
                if this.source != source {
                    return;
                }
                match result {
                    Ok(sessions) => this.sessions = Some(sessions),
                    Err(error) => {
                        this.sessions = Some(Vec::new());
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn set_source(&mut self, source: elyra_core::ProviderKind, cx: &mut Context<Self>) {
        if self.source != source {
            self.source = source;
            self.load(cx);
        }
    }

    fn imported(&self, session_id: &str, cx: &App) -> bool {
        self.app.read(cx).threads.iter().any(|t| {
            t.provider == self.source && t.provider_session_id.as_deref() == Some(session_id)
        })
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked: Vec<(String, std::path::PathBuf)> = self
            .sessions
            .iter()
            .flatten()
            .filter(|s| self.selected.contains(&s.session_id))
            .map(|s| (s.session_id.clone(), s.path.clone()))
            .collect();
        if picked.is_empty() {
            return;
        }
        self.importing = true;
        cx.notify();
        // Reading sessions can take a moment (Codex starts its app server).
        let source = self.source;
        let codex = Self::codex(cx);
        let job = cx.background_executor().spawn(async move {
            picked
                .into_iter()
                .map(|(id, path)| match (source, &codex) {
                    (elyra_core::ProviderKind::Codex, Some(codex)) => {
                        elyra_provider::codex_history::load_session(codex, &id)
                            .map_err(|e| format!("{e:#}"))
                    }
                    (elyra_core::ProviderKind::Codex, None) => {
                        Err("Codex (`codex`) is not installed.".to_string())
                    }
                    _ => elyra_provider::claude_history::load_session(&path)
                        .ok_or_else(|| format!("could not read {}", path.display())),
                })
                .collect::<Vec<_>>()
        });
        cx.spawn_in(window, async move |this, cx| {
            let loaded = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let mut last = None;
                let mut errors = Vec::new();
                for session in loaded {
                    let result = session.and_then(|session| {
                        this.app
                            .update(cx, |app, cx| app.import_session(source, session, cx))
                            .map_err(|e| format!("{e:#}"))
                    });
                    match result {
                        Ok(id) => last = Some(id),
                        Err(error) => errors.push(error),
                    }
                }
                this.importing = false;
                this.selected.clear();
                if let Some(error) = errors.first() {
                    window.push_notification(Notification::error(error.clone()), cx);
                }
                if let Some(id) = last {
                    window.close_dialog(cx);
                    let _ = this
                        .workspace
                        .update(cx, |workspace, cx| workspace.activate(id, window, cx));
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for ImportList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(sessions) = &self.sessions else {
            return div()
                .py_6()
                .text_sm()
                .text_center()
                .text_color(cx.theme().muted_foreground)
                .child(match self.source {
                    elyra_core::ProviderKind::Codex => "Reading Codex history…",
                    _ => "Reading Claude Code history…",
                })
                .into_any_element();
        };
        let query = self.filter.read(cx).value().to_lowercase();
        let rows: Vec<AnyElement> = sessions
            .iter()
            .filter(|s| {
                query.is_empty()
                    || s.title.to_lowercase().contains(&query)
                    || s.cwd
                        .as_ref()
                        .is_some_and(|c| c.display().to_string().to_lowercase().contains(&query))
            })
            .map(|session| {
                let sid = session.session_id.clone();
                let imported = self.imported(&sid, cx);
                let checked = self.selected.contains(&sid);
                let folder = session
                    .cwd
                    .as_ref()
                    .and_then(|c| c.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "—".into());
                let when = session
                    .updated
                    .map(|t| crate::workspace::format_age(chrono::Utc::now() - t))
                    .unwrap_or_default();
                h_flex()
                    .id(SharedString::from(format!("import-{sid}")))
                    .gap_2()
                    .px_1()
                    .py_1()
                    .rounded_md()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .when(!imported, |this| {
                        this.cursor_pointer()
                            .hover(|this| this.bg(cx.theme().accent.opacity(0.5)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.selected.remove(&sid) {
                                    this.selected.insert(sid.clone());
                                }
                                cx.notify();
                            }))
                    })
                    .child(
                        Icon::new(if imported {
                            IconName::CircleCheck
                        } else if checked {
                            IconName::SquareCheck
                        } else {
                            IconName::Square
                        })
                        .small()
                        .text_color(if checked {
                            cx.theme().primary
                        } else {
                            cx.theme().muted_foreground
                        }),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(session.title.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{folder} · {} messages{}",
                                        session.messages,
                                        if imported { " · imported" } else { "" }
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(when),
                    )
                    .into_any_element()
            })
            .collect();
        let count = self.selected.len();
        let source = self.source;
        let tab = |id: &'static str, label: &'static str, kind: elyra_core::ProviderKind| {
            let selected = source == kind;
            Button::new(id)
                .small()
                .when(selected, |b| b.primary())
                .when(!selected, |b| b.ghost())
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_source(kind, cx)))
        };
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_1()
                    .child(tab(
                        "import-claude",
                        "Claude Code",
                        elyra_core::ProviderKind::Claude,
                    ))
                    .child(tab(
                        "import-codex",
                        "Codex",
                        elyra_core::ProviderKind::Codex,
                    )),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(Input::new(&self.filter).small())
            .child(
                v_flex()
                    .id("import-list")
                    .h(px(380.))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(match source {
                                elyra_core::ProviderKind::Codex => {
                                    "Imported threads resume the same Codex session."
                                }
                                _ => "Imported threads resume the same Claude Code session.",
                            }),
                    )
                    .child(
                        Button::new("import-selected")
                            .primary()
                            .small()
                            .loading(self.importing)
                            .disabled(count == 0)
                            .label(if count > 1 {
                                format!("Import {count} sessions")
                            } else {
                                "Import".to_string()
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.import(window, cx))),
                    ),
            )
            .into_any_element()
    }
}

pub fn import_claude_sessions(
    workspace: WeakEntity<Workspace>,
    app: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) {
    let list = cx.new(|cx| {
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter by title or folder"));
        cx.subscribe(&filter, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();
        cx.observe(&app, |_, _, cx| cx.notify()).detach();
        let mut list = ImportList {
            app,
            workspace,
            source: elyra_core::ProviderKind::Claude,
            error: None,
            sessions: None,
            selected: Default::default(),
            filter,
            importing: false,
        };
        list.load(cx);
        list
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Import sessions")
            .w(px(620.))
            .child(list.clone())
    });
}
