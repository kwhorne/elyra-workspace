//! The task board's Félagi tab: the issues assigned to you in Elyra Félagi,
//! by status. Starting one opens a thread on it (linked to the issue, which
//! moves to In progress, with Félagi's timer running); a thread on an issue
//! reports back from its banner (see `felagi_report`).

use crate::app_state::AppState;
use crate::felagi::{self, Client, Connection, Issue};
use crate::workspace::Workspace;
use elyra_core::ProjectId;
use elyra_provider::Prompt;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::Duration;

/// How often the board asks Félagi again while it exists.
const REFRESH: Duration = Duration::from_secs(90);

/// Columns: a title and the statuses they hold.
const COLUMNS: &[(&str, &[&str])] = &[
    ("To do", &["backlog", "todo"]),
    ("In progress", &["in_progress"]),
    ("In review", &["in_review", "on_hold"]),
    ("Done", &["done"]),
];

pub struct FelagiBoard {
    app: Entity<AppState>,
    workspace: WeakEntity<Workspace>,
    /// The Elyra project the board is for (the task board's filter).
    project: Option<ProjectId>,
    connection: Option<Connection>,
    issues: Vec<Issue>,
    projects: Vec<(u64, String)>,
    loading: bool,
    error: Option<String>,
    _poll: Task<()>,
}

impl FelagiBoard {
    pub fn new(
        app: Entity<AppState>,
        workspace: WeakEntity<Workspace>,
        project: Option<ProjectId>,
        cx: &mut Context<Self>,
    ) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFRESH).await;
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        });
        let mut this = Self {
            connection: felagi::connection(&app.read(cx).store),
            app,
            workspace,
            project,
            issues: Vec::new(),
            projects: Vec::new(),
            loading: false,
            error: None,
            _poll: poll,
        };
        this.refresh(cx);
        this
    }

    pub fn set_project(&mut self, project: Option<ProjectId>, cx: &mut Context<Self>) {
        if self.project != project {
            self.project = project;
            self.issues.clear();
            self.refresh(cx);
        }
    }

    fn linked_project(&self, cx: &App) -> Option<u64> {
        felagi::linked_project(&self.app.read(cx).store, self.project?)
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.connection = felagi::connection(&self.app.read(cx).store);
        let Some(connection) = self.connection.clone() else {
            cx.notify();
            return;
        };
        if self.loading {
            return;
        }
        let Some(client) = Client::for_connection(&connection) else {
            self.error = Some(
                "The Félagi token isn't in the Keychain; connect again in Settings → Félagi."
                    .into(),
            );
            cx.notify();
            return;
        };
        self.loading = true;
        cx.notify();
        let project = self.linked_project(cx);
        let job = cx.background_executor().spawn(async move {
            let issues = client.issues(&connection.actor, project, 300)?;
            let projects = client.projects().unwrap_or_default();
            anyhow::Ok((issues, projects))
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((issues, projects)) => {
                        this.issues = issues;
                        this.projects = projects;
                        this.error = None;
                    }
                    Err(err) => this.error = Some(format!("{err:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn link(&mut self, felagi_project: Option<u64>, cx: &mut Context<Self>) {
        if let Some(project) = self.project {
            felagi::link_project(&self.app.read(cx).store, project, felagi_project);
            self.issues.clear();
            self.refresh(cx);
        }
    }

    /// The thread already working on `issue` in this project, if any.
    fn thread_for(&self, issue: &str, cx: &App) -> Option<elyra_core::ThreadId> {
        self.app
            .read(cx)
            .threads
            .iter()
            .find(|t| {
                t.felagi_issue.as_deref() == Some(issue)
                    && (self.project.is_none() || Some(t.project_id) == self.project)
            })
            .map(|t| t.id)
    }

    /// Open a thread on the issue: its text as the first message, the issue
    /// In progress and Félagi's timer running.
    fn start(&mut self, issue: Issue, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.project.or_else(|| {
            self.workspace
                .upgrade()
                .and_then(|w| w.read(cx).current_project)
        });
        let Some(project) = project else {
            window.push_notification(
                "Choose the project to work in with the filter above first.",
                cx,
            );
            return;
        };
        let thread = match self
            .app
            .update(cx, |app, cx| app.create_thread(project, cx))
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
        let (id, title, prompt) = (issue.id.clone(), issue.title.clone(), issue.as_prompt());
        session.update(cx, |session, cx| {
            session.thread.felagi_issue = Some(id.clone());
            session.rename(format!("{id} {title}"), cx);
            session.submit(Prompt::text(prompt), cx);
        });
        if let Some(connection) = self.connection.clone().filter(|c| c.can_write)
            && let Some(client) = Client::for_connection(&connection)
        {
            let move_on = matches!(issue.status.as_str(), "backlog" | "todo");
            let job = cx.background_executor().spawn(async move {
                if move_on {
                    client.set_status(&id, "in_progress")?;
                }
                client.start_timer(&id)
            });
            cx.spawn(async move |this, cx| {
                let result = job.await;
                let _ = this.update(cx, |this, cx| {
                    if let Err(err) = result {
                        this.error = Some(format!("{err:#}"));
                    }
                    this.refresh(cx);
                });
            })
            .detach();
        }
        let _ = self.workspace.update(cx, |workspace, cx| {
            workspace.open_thread_from_panel(thread.id, window, cx)
        });
    }

    fn card(&self, issue: &Issue, cx: &Context<Self>) -> AnyElement {
        let thread = self.thread_for(&issue.id, cx);
        let url = self
            .connection
            .as_ref()
            .map(|c| format!("{}/app/issues/{}", c.url, issue.id))
            .unwrap_or_default();
        let time = match (issue.spent_minutes.unwrap_or(0), issue.estimate_minutes) {
            (0, None) => None,
            (spent, None) => Some(felagi::format_minutes(spent)),
            (spent, Some(estimate)) => Some(format!(
                "{} / {}",
                felagi::format_minutes(spent),
                felagi::format_minutes(estimate)
            )),
        };
        let priority = issue.priority.clone().unwrap_or_default();
        let priority_color = match priority.as_str() {
            "urgent" => cx.theme().danger,
            "high" => cx.theme().warning,
            _ => cx.theme().muted_foreground,
        };
        let open_issue = issue.clone();
        let workspace = self.workspace.clone();
        v_flex()
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_color(cx.theme().muted_foreground)
                            .child(issue.id.clone()),
                    )
                    .when(!priority.is_empty(), |this| {
                        this.child(div().text_color(priority_color).child(priority.clone()))
                    })
                    .child(div().flex_1())
                    .children(
                        time.map(|time| div().text_color(cx.theme().muted_foreground).child(time)),
                    ),
            )
            .child(div().text_sm().child(issue.title.clone()))
            .child(
                h_flex()
                    .gap_1()
                    .justify_end()
                    .child(
                        Button::new(SharedString::from(format!("felagi-open-{}", issue.id)))
                            .xsmall()
                            .ghost()
                            .icon(IconName::ExternalLink)
                            .tooltip("Open in Félagi")
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                    .child(match thread {
                        Some(thread) => {
                            Button::new(SharedString::from(format!("felagi-thread-{}", issue.id)))
                                .xsmall()
                                .outline()
                                .label("Open thread")
                                .on_click(move |_, window, cx| {
                                    let _ = workspace.update(cx, |w, cx| {
                                        w.open_thread_from_panel(thread, window, cx)
                                    });
                                })
                                .into_any_element()
                        }
                        None => {
                            Button::new(SharedString::from(format!("felagi-start-{}", issue.id)))
                                .xsmall()
                                .primary()
                                .icon(IconName::Play)
                                .label("Start")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.start(open_issue.clone(), window, cx)
                                }))
                                .into_any_element()
                        }
                    }),
            )
            .into_any_element()
    }
}

impl Render for FelagiBoard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(connection) = self.connection.clone() else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Connect Elyra Félagi to see the issues assigned to you here."),
                )
                .child(
                    Button::new("felagi-settings")
                        .small()
                        .primary()
                        .label("Open Settings → Félagi")
                        .on_click(|_, _, cx| crate::settings_window::open(cx)),
                )
                .into_any_element();
        };
        let linked = self.linked_project(cx);
        let linked_name: SharedString = linked
            .and_then(|id| self.projects.iter().find(|(p, _)| *p == id))
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| "All my issues".into())
            .into();
        let projects = self.projects.clone();
        let view = cx.weak_entity();
        let columns = COLUMNS.iter().map(|(title, statuses)| {
            let cards: Vec<AnyElement> = self
                .issues
                .iter()
                .filter(|issue| statuses.contains(&issue.status.as_str()))
                .map(|issue| self.card(issue, cx))
                .collect();
            let count = cards.len();
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .p_2()
                .gap_2()
                .rounded_lg()
                .bg(cx.theme().muted.opacity(0.5))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(*title),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(count.to_string()),
                        ),
                )
                .child(
                    v_flex()
                        .id(SharedString::from(format!("felagi-cards-{title}")))
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
                    .py_1()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{} as {}", connection.workspace, connection.user))
                    .when(!connection.can_write, |this| {
                        this.child(
                            div()
                                .text_color(cx.theme().warning)
                                .child("read-only token"),
                        )
                    })
                    .child(div().flex_1())
                    .when(self.project.is_some(), |this| {
                        this.child(
                            Button::new("felagi-link")
                                .xsmall()
                                .ghost()
                                .label(linked_name)
                                .dropdown_caret(true)
                                .tooltip("The Félagi project this Elyra project's work belongs to")
                                .dropdown_menu(move |mut menu, _, _| {
                                    let choices =
                                        std::iter::once((None, "All my issues".to_string())).chain(
                                            projects
                                                .iter()
                                                .map(|(id, name)| (Some(*id), name.clone())),
                                        );
                                    for (id, name) in choices {
                                        let view = view.clone();
                                        menu = menu.item(PopupMenuItem::new(name).on_click(
                                            move |_, _, cx| {
                                                let _ = view.update(cx, |v, cx| v.link(id, cx));
                                            },
                                        ));
                                    }
                                    menu
                                }),
                        )
                    })
                    .child(
                        Button::new("felagi-refresh")
                            .xsmall()
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .loading(self.loading)
                            .tooltip("Ask Félagi again")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .px_3()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(error)
            }))
            .child(h_flex().flex_1().min_h_0().p_3().gap_3().children(columns))
            .into_any_element()
    }
}
