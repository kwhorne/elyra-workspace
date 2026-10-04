//! The task board: Draft → In progress → Done cards. Starting a card hands
//! it to an agent in a new thread; from then on the card follows the chat
//! (done or archived threads finish the card).

use crate::app_state::AppState;
use crate::workspace::Workspace;
use elyra_core::{ProjectId, Task, TaskId, TaskStatus, ThreadStatus};
use elyra_provider::Prompt;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

#[derive(Clone)]
struct DraggedTask {
    id: TaskId,
    title: String,
}

struct DragPreview(String);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_lg()
            .text_sm()
            .child(self.0.clone())
    }
}

pub struct TasksView {
    app: Entity<AppState>,
    workspace: WeakEntity<Workspace>,
    project: Option<ProjectId>,
    new_title: Entity<InputState>,
    /// Showing the Félagi tab (issues from Elyra Félagi) instead of local tasks.
    show_felagi: bool,
    felagi: Option<Entity<crate::felagi_board::FelagiBoard>>,
    _subscriptions: Vec<Subscription>,
}

impl TasksView {
    pub fn new(
        app: Entity<AppState>,
        workspace: WeakEntity<Workspace>,
        project: Option<ProjectId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let new_title = cx.new(|cx| InputState::new(window, cx).placeholder("Add a task…"));
        let subscriptions = vec![
            cx.observe(&app, |this, _, cx| {
                this.sync_with_threads(cx);
                cx.notify();
            }),
            cx.subscribe_in(
                &new_title,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.add_task(window, cx);
                    }
                },
            ),
        ];
        let mut this = Self {
            app,
            workspace,
            project,
            new_title,
            show_felagi: false,
            felagi: None,
            _subscriptions: subscriptions,
        };
        this.sync_with_threads(cx);
        this
    }

    fn set_project(&mut self, project: Option<ProjectId>, cx: &mut Context<Self>) {
        self.project = project;
        if let Some(board) = &self.felagi {
            board.update(cx, |board, cx| board.set_project(project, cx));
        }
        cx.notify();
    }

    fn show_felagi(&mut self, on: bool, cx: &mut Context<Self>) {
        self.show_felagi = on;
        if on && self.felagi.is_none() {
            let (app, workspace, project) =
                (self.app.clone(), self.workspace.clone(), self.project);
            self.felagi = Some(
                cx.new(|cx| crate::felagi_board::FelagiBoard::new(app, workspace, project, cx)),
            );
        } else if on && let Some(board) = &self.felagi {
            board.update(cx, |board, cx| board.refresh(cx));
        }
        cx.notify();
    }

    fn tasks(&self, cx: &App) -> Vec<Task> {
        self.app
            .read(cx)
            .store
            .tasks()
            .unwrap_or_default()
            .into_iter()
            .filter(|t| self.project.is_none() || Some(t.project_id) == self.project)
            .collect()
    }

    fn save(&self, task: &mut Task, cx: &mut Context<Self>) {
        task.updated_at = chrono::Utc::now();
        if let Err(err) = self.app.read(cx).store.save_task(task) {
            log::error!("saving task: {err:#}");
        }
        cx.notify();
    }

    /// Cards follow their threads: a thread marked done or archived
    /// finishes the card.
    fn sync_with_threads(&mut self, cx: &mut Context<Self>) {
        let state = self.app.read(cx);
        let archived: Vec<_> = state.archived_threads().into_iter().map(|t| t.id).collect();
        let mut changed = Vec::new();
        for mut task in state.store.tasks().unwrap_or_default() {
            let Some(thread_id) = task.thread_id else {
                continue;
            };
            let finished =
                archived.contains(&thread_id) || state.thread(thread_id).is_some_and(|t| t.done);
            if finished && task.status != TaskStatus::Done {
                task.status = TaskStatus::Done;
                changed.push(task);
            }
        }
        for mut task in changed {
            self.save(&mut task, cx);
        }
    }

    fn add_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.new_title.read(cx).value().trim().to_string();
        if title.is_empty() {
            return;
        }
        let Some(project) = self
            .project
            .or_else(|| self.app.read(cx).projects.first().map(|p| p.id))
        else {
            window.push_notification("Add a project first.", cx);
            return;
        };
        let mut task = Task::new(project, title);
        self.save(&mut task, cx);
        self.new_title
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn set_status(
        &mut self,
        id: TaskId,
        status: TaskStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut task) = self.tasks(cx).into_iter().find(|t| t.id == id) else {
            return;
        };
        if task.status == status {
            return;
        }
        // Moving an unstarted card to In progress hands it to an agent.
        if status == TaskStatus::InProgress && task.thread_id.is_none() {
            self.start(id, window, cx);
            return;
        }
        task.status = status;
        self.save(&mut task, cx);
    }

    /// Hand a card to an agent: a new thread whose first message is the task.
    fn start(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut task) = self.tasks(cx).into_iter().find(|t| t.id == id) else {
            return;
        };
        let thread = match self
            .app
            .update(cx, |app, cx| app.create_thread(task.project_id, cx))
        {
            Ok(thread) => thread,
            Err(err) => {
                window.push_notification(format!("{err:#}"), cx);
                return;
            }
        };
        let Some(session) = self.app.update(cx, |app, cx| app.session(thread.id, cx)) else {
            return;
        };
        let prompt = if task.notes.trim().is_empty() {
            task.title.clone()
        } else {
            format!("{}\n\n{}", task.title, task.notes.trim())
        };
        let title = task.title.clone();
        session.update(cx, |session, cx| {
            session.rename(title, cx);
            session.submit(Prompt::text(prompt), cx);
        });
        task.thread_id = Some(thread.id);
        task.status = TaskStatus::InProgress;
        self.save(&mut task, cx);
    }

    fn delete(&mut self, id: TaskId, cx: &mut Context<Self>) {
        let _ = self.app.read(cx).store.delete_task(id);
        cx.notify();
    }

    fn edit(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.tasks(cx).into_iter().find(|t| t.id == id) else {
            return;
        };
        let title = cx.new(|cx| InputState::new(window, cx).default_value(task.title.clone()));
        let notes = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(4, 14)
                .placeholder("Details, acceptance criteria, links…")
                .default_value(task.notes.clone())
        });
        let view = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let (title, notes, view, task) =
                (title.clone(), notes.clone(), view.clone(), task.clone());
            dialog
                .title("Task")
                .w(px(520.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(Input::new(&title))
                        .child(Textarea::new(&notes)),
                )
                .footer(
                    h_flex().justify_end().child(
                        Button::new("task-save")
                            .primary()
                            .small()
                            .label("Save")
                            .on_click(move |_, window, cx| {
                                let mut task = task.clone();
                                let new_title = title.read(cx).value().trim().to_string();
                                if !new_title.is_empty() {
                                    task.title = new_title;
                                }
                                task.notes = notes.read(cx).value().to_string();
                                let _ = view.update(cx, |view, cx| view.save(&mut task, cx));
                                window.close_dialog(cx);
                            }),
                    ),
                )
        });
    }

    fn card(&self, task: &Task, cx: &Context<Self>) -> AnyElement {
        let id = task.id;
        let state = self.app.read(cx);
        let thread = task.thread_id.and_then(|t| state.thread(t));
        let project = state
            .project(task.project_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let chip = thread.map(|t| match t.status {
            ThreadStatus::Running => ("Working", cx.theme().info),
            ThreadStatus::NeedsApproval => ("Needs you", cx.theme().warning),
            ThreadStatus::Failed => ("Failed", cx.theme().danger),
            ThreadStatus::Interrupted => ("Interrupted", cx.theme().warning),
            ThreadStatus::Idle if t.is_unread() => ("Replied", cx.theme().success),
            ThreadStatus::Idle => ("Idle", cx.theme().muted_foreground),
        });
        let thread_id = task.thread_id;
        let status = task.status;
        let workspace = self.workspace.clone();
        let dragged = DraggedTask {
            id,
            title: task.title.clone(),
        };
        v_flex()
            .id(SharedString::from(format!("task-{id}")))
            .p_2()
            .gap_1()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .cursor_grab()
            .on_drag(dragged, |dragged, _, _, cx| {
                cx.new(|_| DragPreview(dragged.title.clone()))
            })
            .child(
                h_flex()
                    .gap_1()
                    .child(div().flex_1().text_sm().child(task.title.clone()))
                    .child(
                        Button::new(SharedString::from(format!("task-menu-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Ellipsis)
                            .dropdown_menu({
                                let view = cx.weak_entity();
                                move |menu, _, _| {
                                    let (edit, del) = (view.clone(), view.clone());
                                    let mut menu = menu.item(PopupMenuItem::new("Edit…").on_click(
                                        move |_, window, cx| {
                                            let _ = edit.update(cx, |v, cx| v.edit(id, window, cx));
                                        },
                                    ));
                                    for target in TaskStatus::ALL {
                                        if target == status {
                                            continue;
                                        }
                                        let view = view.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(format!(
                                                "Move to {}",
                                                target.label()
                                            ))
                                            .on_click(move |_, window, cx| {
                                                let _ = view.update(cx, |v, cx| {
                                                    v.set_status(id, target, window, cx)
                                                });
                                            }),
                                        );
                                    }
                                    menu.separator().item(PopupMenuItem::new("Delete").on_click(
                                        move |_, _, cx| {
                                            let _ = del.update(cx, |v, cx| v.delete(id, cx));
                                        },
                                    ))
                                }
                            }),
                    ),
            )
            .when(!task.notes.trim().is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            task.notes
                                .lines()
                                .next()
                                .unwrap_or("")
                                .chars()
                                .take(120)
                                .collect::<String>(),
                        ),
                )
            })
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(project),
                    )
                    .when_some(chip, |this, (label, color)| {
                        this.child(div().text_xs().text_color(color).child(label))
                    })
                    .when(thread_id.is_none() && status != TaskStatus::Done, |this| {
                        this.child(
                            Button::new(SharedString::from(format!("task-start-{id}")))
                                .xsmall()
                                .primary()
                                .icon(IconName::Play)
                                .label("Start")
                                .tooltip("Hand this task to an agent")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.start(id, window, cx)
                                })),
                        )
                    })
                    .when_some(thread_id, |this, thread| {
                        this.child(
                            Button::new(SharedString::from(format!("task-open-{id}")))
                                .xsmall()
                                .ghost()
                                .label("Open")
                                .on_click(move |_, window, cx| {
                                    let _ = workspace.update(cx, |w, cx| {
                                        w.open_thread_from_panel(thread, window, cx)
                                    });
                                }),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Render for TasksView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tasks = self.tasks(cx);
        let state = self.app.read(cx);
        let filter_label: SharedString = self
            .project
            .and_then(|id| state.project(id))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "All projects".into())
            .into();
        let projects: Vec<(Option<ProjectId>, String)> =
            std::iter::once((None, "All projects".to_string()))
                .chain(state.projects.iter().map(|p| (Some(p.id), p.name.clone())))
                .collect();
        let view = cx.weak_entity();
        let columns = TaskStatus::ALL.map(|status| {
            let cards: Vec<AnyElement> = tasks
                .iter()
                .filter(|t| t.status == status)
                .map(|t| self.card(t, cx))
                .collect();
            let count = cards.len();
            v_flex()
                .id(SharedString::from(format!("column-{}", status.label())))
                .flex_1()
                .min_w_0()
                .h_full()
                .p_2()
                .gap_2()
                .rounded_lg()
                .bg(cx.theme().muted.opacity(0.5))
                .drag_over::<DraggedTask>(|style, _, _, cx| {
                    style.bg(cx.theme().accent.opacity(0.4))
                })
                .on_drop(cx.listener(move |this, dragged: &DraggedTask, window, cx| {
                    this.set_status(dragged.id, status, window, cx)
                }))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(status.label()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(count.to_string()),
                        ),
                )
                .when(status == TaskStatus::Draft, |this| {
                    this.child(Input::new(&self.new_title).small())
                })
                .child(
                    v_flex()
                        .id(SharedString::from(format!("cards-{}", status.label())))
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .gap_2()
                        .children(cards),
                )
        });
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Icon::new(IconName::SquareKanban).small())
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Tasks"),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                Button::new("tasks-local")
                                    .xsmall()
                                    .when(!self.show_felagi, |b| b.primary())
                                    .when(self.show_felagi, |b| b.ghost())
                                    .label("Local")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.show_felagi(false, cx)),
                                    ),
                            )
                            .child(
                                Button::new("tasks-felagi")
                                    .xsmall()
                                    .when(self.show_felagi, |b| b.primary())
                                    .when(!self.show_felagi, |b| b.ghost())
                                    .label("Félagi")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.show_felagi(true, cx)),
                                    ),
                            ),
                    )
                    .child(
                        Button::new("tasks-project")
                            .small()
                            .ghost()
                            .label(filter_label)
                            .dropdown_caret(true)
                            .dropdown_menu(move |mut menu, _, _| {
                                for (id, name) in projects.clone() {
                                    let view = view.clone();
                                    menu = menu.item(PopupMenuItem::new(name).on_click(
                                        move |_, _, cx| {
                                            let _ = view.update(cx, |v, cx| v.set_project(id, cx));
                                        },
                                    ));
                                }
                                menu
                            }),
                    ),
            )
            .child(match (&self.felagi, self.show_felagi) {
                (Some(board), true) => div()
                    .flex_1()
                    .min_h_0()
                    .child(board.clone())
                    .into_any_element(),
                _ => h_flex()
                    .flex_1()
                    .min_h_0()
                    .p_3()
                    .gap_3()
                    .children(columns)
                    .into_any_element(),
            })
    }
}
