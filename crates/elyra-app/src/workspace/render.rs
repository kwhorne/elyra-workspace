//! Drawing the workspace: the title bar, the tabs, the conversation and the
//! tools panel.

use super::*;

/// A view drawn from the previous frame's cache unless it notified since.
/// Spinners and streaming redraw the window many times a second; the views
/// that didn't change are then not built again.
pub(super) fn cached<V: Render>(view: &Entity<V>) -> AnyElement {
    view.clone()
        .cached(StyleRefinement::default().size_full())
        .into_any_element()
}

/// The narrowest the conversation gets while the tools panel can still shrink.
pub(super) const MIN_CONVERSATION: Pixels = px(400.);

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The window's own title, for Mission Control and ⌘`.
        let window_title = match self.front_thread(cx) {
            Some((project, title)) => format!("{project} — {title}"),
            None => "Elyra Workspace".to_string(),
        };
        if self.window_title != window_title {
            window.set_window_title(&window_title);
            self.window_title = window_title;
        }
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
            Some(Panel::Review) => self.review.as_ref().map(cached),
            Some(Panel::Automations) => self.automations.as_ref().map(cached),
            Some(Panel::Tasks) => self.tasks.as_ref().map(cached),
            Some(Panel::Stats) => self.stats.as_ref().map(cached),
            Some(Panel::Race) => self.race.as_ref().map(cached),
            Some(Panel::Search) => self.search.as_ref().map(cached),
            Some(Panel::Walkthrough) => self.walkthrough.as_ref().map(cached),
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

        // The conversation keeps room to read: in a narrow window the tools
        // panel gives way (down to its own minimum) before it does.
        let room = window.viewport_size().width - if self.sidebar_open { px(272.) } else { px(0.) };
        let right_max = (room - MIN_CONVERSATION).clamp(px(320.), px(1100.));
        let main: AnyElement = match right {
            Some(right) => h_resizable("workspace-split")
                .child(
                    resizable_panel()
                        .size_range(
                            MIN_CONVERSATION.min(room - px(320.)).max(px(200.))..Pixels::MAX,
                        )
                        .child(center),
                )
                .child(
                    resizable_panel()
                        .size(px(520.).min(right_max))
                        .size_range(px(320.)..right_max)
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
            .on_action(cx.listener(Self::on_go_to_symbol))
            .on_action(cx.listener(Self::on_search_in_files))
            .on_action(cx.listener(Self::on_show_files))
            .on_action(cx.listener(Self::on_show_context))
            .on_action(cx.listener(Self::on_show_shortcuts))
            .on_action(cx.listener(Self::on_toggle_terminal_workspace))
            .on_action(cx.listener(Self::on_open_files_editor))
            .on_action(cx.listener(|this, _: &WhileAway, window, cx| {
                let (from, to) = this.last_away.unwrap_or_else(|| {
                    let now = chrono::Utc::now();
                    (now - chrono::Duration::hours(8), now)
                });
                if !this.show_away(from, to, window, cx) {
                    window.push_notification(
                        Notification::info("Nothing happened in the threads while you were away."),
                        cx,
                    );
                }
            }))
            .on_action(cx.listener(Self::on_open_in_editor))
            .on_action(cx.listener(Self::on_new_side_chat))
            .on_action(cx.listener(Self::on_import_thread))
            .on_action(cx.listener(Self::on_show_automations))
            .on_action(cx.listener(Self::on_show_tasks))
            .on_action(cx.listener(Self::on_show_stats))
            .on_action(cx.listener(Self::on_search_threads))
            .on_action(cx.listener(Self::on_review_branch))
            .on_action(cx.listener(Self::on_show_browser))
            .on_action(cx.listener(Self::on_best_of_n))
            .on_action(cx.listener(Self::on_new_duel))
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

impl Workspace {
    pub(super) fn render_title_bar(&self, cx: &Context<Self>) -> AnyElement {
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

    pub(super) fn render_open_in(&self, cx: &Context<Self>) -> AnyElement {
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

    pub(super) fn render_tabs(&self, cx: &Context<Self>) -> AnyElement {
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
                let short: String = if title.chars().count() > 28 {
                    format!("{}…", title.chars().take(27).collect::<String>().trim_end())
                } else {
                    title
                };
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
        // Keep the active tab in view when there are more than fit.
        if self.tab_revealed.get() != self.active {
            self.tab_revealed.set(self.active);
            self.tab_scroll.scroll_to_item(selected);
        }
        let ids = self.open_tabs.clone();
        div()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .child(
                TabBar::new("thread-tabs")
                    .underline()
                    .small()
                    .w_full()
                    .min_w_0()
                    .track_scroll(&self.tab_scroll)
                    // A list of every tab, for when they don't fit.
                    .menu(ids.len() > 1)
                    .selected_index(selected)
                    .children(tabs)
                    .on_click(cx.listener(move |this, index: &usize, window, cx| {
                        if let Some(id) = ids.get(*index).copied() {
                            this.activate(id, window, cx);
                        }
                    })),
            )
            .into_any_element()
    }

    pub(super) fn render_center(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
            .min_w_0()
            .overflow_hidden()
            .child(self.render_tabs(cx))
            .child(div().flex_1().min_h_0().child(cached(&view)));
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
            .child(div().flex_1().min_h_0().child(cached(&split_view)));
        h_resizable("chat-split")
            .child(resizable_panel().child(main))
            .child(resizable_panel().child(pane))
            .into_any_element()
    }

    pub(super) fn render_right(&self, cx: &Context<Self>) -> AnyElement {
        let id = self.active;
        let content: AnyElement = match (self.right_tab, id) {
            (RightTab::Changes, Some(id)) => self
                .changes_views
                .get(&id)
                .map(cached)
                .unwrap_or_else(|| div().into_any_element()),
            (RightTab::Terminal, Some(id)) => self
                .terminal_views
                .get(&id)
                .map(cached)
                .unwrap_or_else(|| div().into_any_element()),
            (RightTab::PullRequest, Some(id)) => self
                .pr_views
                .get(&id)
                .map(cached)
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
                .map(cached)
                .unwrap_or_else(|| div().into_any_element()),
            (RightTab::Browser, Some(id)) => self
                .browser_views
                .get(&id)
                .map(|v| v.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            (RightTab::Context, Some(id)) => self
                .context_views
                .get(&id)
                .map(cached)
                .unwrap_or_else(|| div().into_any_element()),
            (RightTab::SideChat, Some(id)) => self
                .side_chat_of(id, cx)
                .and_then(|side| self.thread_views.get(&side))
                .map(cached)
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
                // A narrow panel scrolls its tabs and lists them all in ▾.
                div().w_full().min_w_0().overflow_hidden().child(
                    TabBar::new("right-tabs")
                        .underline()
                        .small()
                        .w_full()
                        .min_w_0()
                        .menu(true)
                        .selected_index(index)
                        .children(tab_items)
                        .on_click(cx.listener(move |this, index: &usize, window, cx| {
                            let tab = tabs.get(*index).copied().unwrap_or(RightTab::Changes);
                            this.show_right_tab(tab, window, cx);
                        })),
                ),
            )
            .children(side_bar)
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}
