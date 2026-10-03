//! The project and thread sidebar: pinned threads, collapsible projects with
//! emoji and colour, unread and status indicators, done threads, the archive
//! and context menus for threads and projects.

use crate::app_state::AppState;
use crate::dialogs;
use crate::workspace::{Panel, RightTab, Workspace, format_age};
use chrono::Utc;
use elyra_core::{Project, ProjectId, Thread, ThreadId, ThreadStatus};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

fn parse_color(hex: &str) -> Option<Hsla> {
    let value = u32::from_str_radix(hex.strip_prefix('#')?, 16).ok()?;
    Some(rgb(value).into())
}

/// Build the thread context menu (shared by right-click and the ⋯ button).
fn thread_menu(
    menu: PopupMenu,
    workspace: WeakEntity<Workspace>,
    app: Entity<AppState>,
    thread: Thread,
    path: std::path::PathBuf,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let id = thread.id;
    let current_provider = thread.provider;
    let handoff_workspace = workspace.clone();
    let on = |workspace: &WeakEntity<Workspace>,
              f: fn(&mut Workspace, ThreadId, &mut Window, &mut Context<Workspace>)| {
        let workspace = workspace.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            let _ = workspace.update(cx, |this, cx| f(this, id, window, cx));
        }
    };
    let set = |app: &Entity<AppState>, edit: fn(&mut Thread)| {
        let app = app.clone();
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            app.update(cx, |app, cx| app.update_thread(id, edit, cx));
        }
    };
    let rename_app = app.clone();
    let copy_id = id.to_string();
    let copy_path = path.display().to_string();
    menu.item(
        PopupMenuItem::new("Rename…")
            .icon(IconName::Pencil)
            .on_click(move |_, window, cx| {
                dialogs::rename_thread(rename_app.clone(), id, window, cx)
            }),
    )
    .item(
        PopupMenuItem::new("Generate title")
            .icon(IconName::Sparkles)
            .on_click(on(&workspace, |this, id, _, cx| {
                if let Some(session) = this.app.update(cx, |app, cx| app.session(id, cx)) {
                    session.update(cx, |session, cx| session.generate_title(cx));
                }
            })),
    )
    .separator()
    .item(if thread.pinned {
        PopupMenuItem::new("Unpin")
            .icon(IconName::PinOff)
            .on_click(set(&app, |t| t.pinned = false))
    } else {
        PopupMenuItem::new("Pin")
            .icon(IconName::Pin)
            .on_click(set(&app, |t| t.pinned = true))
    })
    .item(if thread.done {
        PopupMenuItem::new("Mark as not done")
            .icon(IconName::CircleDashed)
            .on_click(set(&app, |t| t.done = false))
    } else {
        PopupMenuItem::new("Mark as done")
            .icon(IconName::CircleCheck)
            .on_click(on(&workspace, |this, id, window, cx| {
                this.mark_done(id, window, cx)
            }))
    })
    .item(
        PopupMenuItem::new("Mark as unread")
            .icon(IconName::Dot)
            .on_click(set(&app, |t| {
                t.read_at = None;
                t.last_activity_at = Some(Utc::now());
            })),
    )
    .separator()
    .item(
        PopupMenuItem::new("Fork")
            .icon(IconName::GitFork)
            .on_click(on(&workspace, |this, id, window, cx| {
                this.fork_thread(id, window, cx)
            })),
    )
    .submenu_with_icon(
        Some(Icon::new(IconName::ArrowRightLeft)),
        "Continue with",
        window,
        cx,
        move |mut menu, _, cx| {
            for kind in crate::preferences::Preferences::global(cx).enabled_providers() {
                if kind == current_provider {
                    continue;
                }
                let workspace = handoff_workspace.clone();
                menu = menu.item(PopupMenuItem::new(kind.label()).on_click(
                    move |_, window, cx| {
                        let _ = workspace
                            .update(cx, |this, cx| this.handoff_thread(id, kind, window, cx));
                    },
                ));
            }
            menu
        },
    )
    .item(
        PopupMenuItem::new("Export…")
            .icon(IconName::Download)
            .on_click(on(&workspace, |this, id, window, cx| {
                this.export_thread(id, window, cx)
            })),
    )
    .separator()
    .item(
        PopupMenuItem::new("Open terminal here")
            .icon(IconName::SquareTerminal)
            .on_click(on(&workspace, |this, id, window, cx| {
                this.activate(id, window, cx);
                this.show_right_tab(RightTab::Terminal, window, cx);
            })),
    )
    .item(
        PopupMenuItem::new("Copy path")
            .icon(IconName::Copy)
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_path.clone()))
            }),
    )
    .item(
        PopupMenuItem::new("Copy thread ID")
            .icon(IconName::Hash)
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_id.clone()))
            }),
    )
    .separator()
    .item(
        PopupMenuItem::new("Archive")
            .icon(IconName::Archive)
            .on_click(on(&workspace, |this, id, window, cx| {
                this.archive_thread(id, window, cx)
            })),
    )
    .item(
        PopupMenuItem::new("Delete…")
            .icon(IconName::Trash)
            .on_click({
                let workspace = workspace.clone();
                move |_, window, cx| {
                    dialogs::delete_thread(workspace.clone(), app.clone(), id, window, cx)
                }
            }),
    )
}

fn project_menu(
    menu: PopupMenu,
    workspace: WeakEntity<Workspace>,
    app: Entity<AppState>,
    project: Project,
) -> PopupMenu {
    let id = project.id;
    let on = |f: fn(&mut Workspace, ProjectId, &mut Window, &mut Context<Workspace>)| {
        let workspace = workspace.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            let _ = workspace.update(cx, |this, cx| f(this, id, window, cx));
        }
    };
    let (reveal, copy) = (project.path.clone(), project.path.display().to_string());
    let (edit_app, pin_app) = (app.clone(), app.clone());
    let pinned = project.pinned;
    menu.item(
        PopupMenuItem::new("New thread")
            .icon(IconName::SquarePen)
            .on_click(on(|this, id, window, cx| {
                this.new_thread_in(id, window, cx)
            })),
    )
    .item(
        PopupMenuItem::new("Rename and appearance…")
            .icon(IconName::Palette)
            .on_click(move |_, window, cx| dialogs::edit_project(edit_app.clone(), id, window, cx)),
    )
    .item(
        PopupMenuItem::new(if pinned {
            "Unpin project"
        } else {
            "Pin project"
        })
        .icon(if pinned {
            IconName::PinOff
        } else {
            IconName::Pin
        })
        .on_click(move |_, _, cx| {
            pin_app.update(cx, |app, cx| {
                if let Some(mut project) = app.project(id).cloned() {
                    project.pinned = !project.pinned;
                    app.update_project(project, cx);
                }
            })
        }),
    )
    .separator()
    .item(
        PopupMenuItem::new("Reveal in Finder")
            .icon(IconName::FolderOpen)
            .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
    )
    .item(
        PopupMenuItem::new("Copy path")
            .icon(IconName::Copy)
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
            }),
    )
    .item(
        PopupMenuItem::new("Change folder…")
            .icon(IconName::FolderInput)
            .on_click(on(|this, id, window, cx| {
                this.change_project_folder(id, window, cx)
            })),
    )
    .separator()
    .item(
        PopupMenuItem::new("Remove project")
            .icon(IconName::Trash)
            .on_click(on(|this, id, window, cx| {
                this.remove_project(id, window, cx)
            })),
    )
}

impl Workspace {
    fn status_indicator(thread: &Thread, running: bool, cx: &App) -> AnyElement {
        match (thread.status, running) {
            (ThreadStatus::NeedsApproval, _) => Icon::new(IconName::CircleAlert)
                .xsmall()
                .text_color(cx.theme().warning)
                .into_any_element(),
            (_, true) | (ThreadStatus::Running, _) => Spinner::new().xsmall().into_any_element(),
            (ThreadStatus::Failed, _) => Icon::new(IconName::CircleX)
                .xsmall()
                .text_color(cx.theme().danger)
                .into_any_element(),
            (ThreadStatus::Interrupted, _) => Icon::new(IconName::CirclePause)
                .xsmall()
                .text_color(cx.theme().warning)
                .into_any_element(),
            _ if thread.done => Icon::new(IconName::CircleCheck)
                .xsmall()
                .text_color(cx.theme().muted_foreground)
                .into_any_element(),
            _ => Icon::new(IconName::MessageSquare)
                .xsmall()
                .text_color(cx.theme().muted_foreground)
                .into_any_element(),
        }
    }

    fn thread_row(&self, thread: &Thread, indent: bool, cx: &Context<Self>) -> AnyElement {
        let state = self.app.read(cx);
        let id = thread.id;
        let selected = self.active == Some(id);
        let running = state
            .existing_session(id)
            .is_some_and(|s| s.read(cx).running);
        let unread = thread.is_unread() && !selected;
        let path = state
            .project(thread.project_id)
            .map(|p| thread.working_dir(p))
            .unwrap_or_default();
        let age = format_age(Utc::now() - thread.updated_at);
        let (menu_workspace, menu_app, menu_thread, menu_path) = (
            cx.weak_entity(),
            self.app.clone(),
            thread.clone(),
            path.clone(),
        );
        let (button_workspace, button_app, button_thread) =
            (cx.weak_entity(), self.app.clone(), thread.clone());
        h_flex()
            .id(SharedString::from(format!("thread-{id}")))
            .group("thread-row")
            .w_full()
            .when(indent, |this| this.pl_5())
            .when(!indent, |this| this.pl_2())
            .pr_1()
            .py_1()
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .text_sm()
            .when(selected, |this| {
                this.bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
            })
            .when(thread.done && !selected, |this| {
                this.text_color(cx.theme().muted_foreground)
            })
            .hover(|this| this.bg(cx.theme().sidebar_accent.opacity(0.6)))
            .child(Self::status_indicator(thread, running, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .when(unread, |this| this.font_weight(FontWeight::SEMIBOLD))
                    .child(thread.title.clone()),
            )
            .when(unread, |this| {
                this.child(div().size(px(7.)).rounded_full().bg(cx.theme().primary))
            })
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .group_hover("thread-row", |style| style.invisible())
                    .child(age),
            )
            .child(
                div()
                    .invisible()
                    .group_hover("thread-row", |style| style.visible())
                    .child(
                        Button::new(SharedString::from(format!("thread-menu-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Ellipsis)
                            .dropdown_menu(move |menu, window, cx| {
                                thread_menu(
                                    menu,
                                    button_workspace.clone(),
                                    button_app.clone(),
                                    button_thread.clone(),
                                    path.clone(),
                                    window,
                                    cx,
                                )
                            }),
                    ),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.activate(id, window, cx)))
            .context_menu(move |menu, window, cx| {
                thread_menu(
                    menu,
                    menu_workspace.clone(),
                    menu_app.clone(),
                    menu_thread.clone(),
                    menu_path.clone(),
                    window,
                    cx,
                )
            })
            .into_any_element()
    }

    fn project_section(
        &self,
        project: &Project,
        threads: &[&Thread],
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = project.id;
        let collapsed = self.collapsed.contains(&id);
        let show_done = self.show_done.contains(&id);
        let active: Vec<&&Thread> = threads.iter().filter(|t| !t.done).collect();
        let done: Vec<&&Thread> = threads.iter().filter(|t| t.done).collect();
        let unread = threads.iter().filter(|t| t.is_unread()).count();
        let color = project.color.as_deref().and_then(parse_color);
        let icon: AnyElement = match &project.icon {
            Some(emoji) => div().text_sm().child(emoji.clone()).into_any_element(),
            None => Icon::new(IconName::Folder)
                .small()
                .text_color(color.unwrap_or(cx.theme().muted_foreground))
                .into_any_element(),
        };
        let (menu_workspace, menu_app, menu_project) =
            (cx.weak_entity(), self.app.clone(), project.clone());
        let (button_workspace, button_app, button_project) =
            (cx.weak_entity(), self.app.clone(), project.clone());
        let header = h_flex()
            .id(SharedString::from(format!("project-{id}")))
            .group("project-row")
            .w_full()
            .pl_1()
            .pr_1()
            .py_1()
            .gap_1p5()
            .rounded_md()
            .cursor_pointer()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .hover(|this| this.bg(cx.theme().sidebar_accent.opacity(0.4)))
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall()
                .text_color(cx.theme().muted_foreground),
            )
            .child(icon)
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .when_some(color, |this, color| this.text_color(color))
                    .child(project.name.clone()),
            )
            .when(collapsed && unread > 0, |this| {
                this.child(
                    div()
                        .px_1p5()
                        .rounded_full()
                        .bg(cx.theme().primary)
                        .text_color(cx.theme().primary_foreground)
                        .text_xs()
                        .child(unread.to_string()),
                )
            })
            .when(project.pinned, |this| {
                this.child(
                    Icon::new(IconName::Pin)
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
            })
            .child(
                Button::new(SharedString::from(format!("new-in-{id}")))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip("New thread")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.new_thread_in(id, window, cx)
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("project-menu-{id}")))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Ellipsis)
                    .dropdown_menu(move |menu, _, _| {
                        project_menu(
                            menu,
                            button_workspace.clone(),
                            button_app.clone(),
                            button_project.clone(),
                        )
                    }),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_collapsed(id, cx)))
            .context_menu(move |menu, _, _| {
                project_menu(
                    menu,
                    menu_workspace.clone(),
                    menu_app.clone(),
                    menu_project.clone(),
                )
            });

        let mut section = v_flex().gap_0p5().child(header);
        if !collapsed {
            section = section.children(active.iter().map(|t| self.thread_row(t, true, cx)));
            if !done.is_empty() {
                section = section.child(
                    div()
                        .id(SharedString::from(format!("done-{id}")))
                        .pl_5()
                        .py_0p5()
                        .text_xs()
                        .cursor_pointer()
                        .text_color(cx.theme().muted_foreground)
                        .hover(|this| this.text_color(cx.theme().foreground))
                        .child(if show_done {
                            format!("Hide {} done", done.len())
                        } else {
                            format!("Show {} done", done.len())
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.show_done.remove(&id) {
                                this.show_done.insert(id);
                            }
                            cx.notify();
                        })),
                );
                if show_done {
                    section = section.children(done.iter().map(|t| self.thread_row(t, true, cx)));
                }
            }
        }
        section.into_any_element()
    }

    pub(crate) fn render_sidebar(&self, cx: &Context<Self>) -> AnyElement {
        let state = self.app.read(cx);
        // Side chats live under their parent thread, not in the sidebar.
        let pinned: Vec<&Thread> = state
            .threads
            .iter()
            .filter(|t| t.pinned && t.parent_id.is_none())
            .collect();
        let mut spaces: Vec<String> = state
            .projects
            .iter()
            .filter_map(|p| p.space.clone())
            .collect();
        spaces.sort_by_key(|s| s.to_lowercase());
        spaces.dedup();
        let space = self
            .active_space
            .clone()
            .filter(|space| spaces.contains(space));
        let mut projects: Vec<&Project> = state
            .projects
            .iter()
            .filter(|p| space.is_none() || p.space == space)
            .collect();
        projects.sort_by_key(|p| (!p.pinned, p.name.to_lowercase()));
        let sections: Vec<AnyElement> = projects
            .iter()
            .map(|project| {
                let threads: Vec<&Thread> = state
                    .project_threads(project.id)
                    .filter(|t| !t.pinned && t.parent_id.is_none())
                    .collect();
                self.project_section(project, &threads, cx)
            })
            .collect();
        let archived = state.store.archived_threads().map(|t| t.len()).unwrap_or(0);
        let label = |text: &'static str| {
            div()
                .px_2()
                .pt_1()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child(text)
        };

        v_flex()
            .h_full()
            .w(px(272.))
            .flex_none()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                h_flex()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child("PROJECTS"),
                    )
                    .child(
                        Button::new("tasks-board")
                            .ghost()
                            .xsmall()
                            .icon(IconName::SquareKanban)
                            .when(self.panel == Some(Panel::Tasks), |b| b.primary())
                            .tooltip("Tasks (⌥⌘T)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_panel(Panel::Tasks, window, cx)
                            })),
                    )
                    .child(
                        Button::new("automations")
                            .ghost()
                            .xsmall()
                            .icon(IconName::CalendarClock)
                            .when(self.panel == Some(Panel::Automations), |b| b.primary())
                            .tooltip("Automations (⌥⌘A)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_panel(Panel::Automations, window, cx)
                            })),
                    )
                    .child(
                        Button::new("code-review")
                            .ghost()
                            .xsmall()
                            .icon(IconName::GitPullRequest)
                            .when(self.panel == Some(Panel::Review), |b| b.primary())
                            .tooltip("Code review (⇧⌘R)")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_review(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("new-chat")
                            .ghost()
                            .xsmall()
                            .icon(IconName::MessageSquarePlus)
                            .tooltip("New chat without a project (⌥⌘N)")
                            .on_click(cx.listener(|this, _, window, cx| this.new_chat(window, cx))),
                    )
                    .child(
                        Button::new("add-project")
                            .ghost()
                            .xsmall()
                            .icon(IconName::FolderPlus)
                            .tooltip("Add project (⇧⌘O)")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_project(window, cx)),
                            ),
                    ),
            )
            .when(!spaces.is_empty(), |this| {
                let chip = |id: SharedString,
                            label: String,
                            value: Option<String>,
                            cx: &Context<Self>| {
                    let selected = space == value;
                    div()
                        .id(id)
                        .px_2()
                        .py_0p5()
                        .rounded_md()
                        .text_xs()
                        .cursor_pointer()
                        .when(selected, |this| {
                            this.bg(cx.theme().sidebar_accent)
                                .text_color(cx.theme().sidebar_accent_foreground)
                        })
                        .when(!selected, |this| {
                            this.text_color(cx.theme().muted_foreground)
                        })
                        .hover(|this| this.bg(cx.theme().sidebar_accent.opacity(0.6)))
                        .child(label)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.set_space(value.clone(), cx)),
                        )
                };
                this.child(
                    h_flex()
                        .px_2()
                        .pb_1()
                        .gap_0p5()
                        .flex_wrap()
                        .child(chip("space-all".into(), "All".into(), None, cx))
                        .children(spaces.iter().map(|name| {
                            chip(
                                SharedString::from(format!("space-{name}")),
                                name.clone(),
                                Some(name.clone()),
                                cx,
                            )
                        })),
                )
            })
            .child(
                div()
                    .id("sidebar-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .when(!pinned.is_empty(), |this| {
                        this.child(
                            v_flex()
                                .pb_2()
                                .gap_0p5()
                                .child(label("PINNED"))
                                .children(pinned.iter().map(|t| self.thread_row(t, false, cx))),
                        )
                    })
                    .child(v_flex().gap_2().children(sections))
                    .when(state.projects.is_empty(), |this| {
                        this.child(
                            div()
                                .p_2()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Add a project folder to start a thread."),
                        )
                    }),
            )
            .child(
                v_flex()
                    .p_2()
                    .gap_1()
                    .border_t_1()
                    .border_color(cx.theme().sidebar_border)
                    .when(archived > 0, |this| {
                        this.child(
                            Button::new("archived")
                                .ghost()
                                .xsmall()
                                .w_full()
                                .icon(IconName::Archive)
                                .label(format!("Archived ({archived})"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    dialogs::archived_threads(
                                        cx.weak_entity(),
                                        this.app.clone(),
                                        window,
                                        cx,
                                    )
                                })),
                        )
                    })
                    .child(
                        Button::new("new-thread")
                            .w_full()
                            .small()
                            .primary()
                            .icon(IconName::SquarePen)
                            .label("New thread")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_thread(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn toggle_collapsed(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&id) {
            self.collapsed.insert(id);
        }
        let value = self
            .collapsed
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        self.app
            .update(cx, |app, _| app.set_setting("collapsed_projects", &value));
        cx.notify();
    }

    pub(crate) fn remove_project(
        &mut self,
        project_id: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tabs: Vec<ThreadId> = self
            .open_tabs
            .iter()
            .copied()
            .filter(|id| {
                self.app
                    .read(cx)
                    .thread(*id)
                    .is_some_and(|t| t.project_id == project_id)
            })
            .collect();
        for id in tabs {
            self.close_tab(id, window, cx);
        }
        if let Err(err) = self
            .app
            .update(cx, |app, cx| app.remove_project(project_id, cx))
        {
            window.push_notification(Notification::error(format!("{err:#}")), cx);
        }
        if self.current_project == Some(project_id) {
            self.current_project = None;
        }
    }

    pub(crate) fn change_project_folder(
        &mut self,
        project_id: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Use folder".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                match this
                    .app
                    .update(cx, |app, cx| app.relocate_project(project_id, &path, cx))
                {
                    Ok(_) => {
                        // Shells keep their start directory; reopen them in the new folder.
                        let moved: Vec<ThreadId> = this
                            .app
                            .read(cx)
                            .project_threads(project_id)
                            .map(|thread| thread.id)
                            .collect();
                        for id in moved {
                            this.terminal_views.remove(&id);
                        }
                        this.sync_right_panel(window, cx);
                        cx.notify();
                    }
                    Err(err) => {
                        window.push_notification(Notification::error(format!("{err:#}")), cx)
                    }
                }
            });
        })
        .detach();
    }

    /// Drop all views of a thread that is going away.
    pub(crate) fn forget_thread(&mut self, id: ThreadId, cx: &mut Context<Self>) {
        self.open_tabs.retain(|t| *t != id);
        self.thread_views.remove(&id);
        self.changes_views.remove(&id);
        self.terminal_views.remove(&id);
        if self.active == Some(id) {
            self.active = None;
        }
        cx.notify();
    }

    /// After a thread leaves the active list, show the most recently used
    /// unfinished thread instead.
    fn move_on_from(&mut self, id: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        if self.active != Some(id) && self.active.is_some() {
            return;
        }
        let next = self
            .app
            .read(cx)
            .threads
            .iter()
            .filter(|t| t.id != id && !t.done && !t.archived)
            .max_by_key(|t| t.updated_at)
            .map(|t| t.id);
        match next {
            Some(next) => self.activate(next, window, cx),
            None => {
                self.active = None;
                cx.notify();
            }
        }
    }

    pub(crate) fn archive_thread(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_active = self.active == Some(id);
        self.forget_thread(id, cx);
        self.app.update(cx, |app, cx| app.archive_thread(id, cx));
        if was_active {
            self.move_on_from(id, window, cx);
        }
    }

    pub(crate) fn mark_done(&mut self, id: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        let was_active = self.active == Some(id);
        self.app
            .update(cx, |app, cx| app.update_thread(id, |t| t.done = true, cx));
        if was_active {
            self.close_tab(id, window, cx);
            self.move_on_from(id, window, cx);
        }
    }

    pub(crate) fn new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.app.update(cx, |app, cx| app.scratch_project(cx)) {
            Ok(project) => self.new_thread_in(project.id, window, cx),
            Err(err) => window.push_notification(Notification::error(format!("{err:#}")), cx),
        }
    }
}
