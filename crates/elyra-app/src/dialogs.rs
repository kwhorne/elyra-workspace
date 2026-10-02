//! Modal dialogs: rename thread, project appearance, delete thread and the
//! archived-threads list.

use crate::app_state::AppState;
use crate::workspace::Workspace;
use elyra_core::{Environment, ProjectId, ThreadId};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
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
