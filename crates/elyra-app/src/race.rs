//! Best of N: one task given to several agents at once, each in its own
//! worktree, compared side by side. The change you pick is brought into the
//! project folder (squashed and staged, not committed) for you to review.

use crate::app_state::AppState;
use crate::thread_session::format_usd;
use crate::workspace::Workspace;
use elyra_core::{Environment, ProjectId, ProviderKind, ThreadId, ThreadStatus};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

/// How often the comparison refreshes while it is open.
const REFRESH: Duration = Duration::from_secs(3);

/// The task and the agents to give it to.
pub struct RaceForm {
    prompt: Entity<TextareaState>,
    available: Vec<ProviderKind>,
    chosen: Vec<ProviderKind>,
}

impl RaceForm {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(4, 12)
                .placeholder("The task, as you would write it in a thread")
        });
        let available = crate::preferences::Preferences::global(cx).enabled_providers();
        let chosen = available.iter().copied().take(3).collect();
        Self {
            prompt,
            available,
            chosen,
        }
    }
}

impl Render for RaceForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chips = self.available.iter().map(|&kind| {
            let on = self.chosen.contains(&kind);
            Button::new(SharedString::from(format!("race-{}", kind.as_str())))
                .small()
                .when(on, |this| this.primary())
                .when(!on, |this| this.outline())
                .label(kind.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(index) = this.chosen.iter().position(|k| *k == kind) {
                        this.chosen.remove(index);
                    } else {
                        this.chosen.push(kind);
                    }
                    cx.notify();
                }))
        });
        v_flex()
            .gap_3()
            .child(Textarea::new(&self.prompt))
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Agents"))
                    .child(h_flex().flex_wrap().gap_1().children(chips))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Each agent works in its own Git worktree, so they don't get in each other's way. Pick at least two."),
                    ),
            )
    }
}

/// Ask for the task and the agents, then start them.
pub fn start_dialog(
    workspace: WeakEntity<Workspace>,
    project: ProjectId,
    window: &mut Window,
    cx: &mut App,
) {
    let form = cx.new(|cx| RaceForm::new(window, cx));
    let prompt = form.read(cx).prompt.clone();
    prompt.update(cx, |prompt, cx| prompt.focus(window, cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        let ready = {
            let form = form.read(cx);
            form.chosen.len() >= 2 && !form.prompt.read(cx).value().trim().is_empty()
        };
        let (form_start, workspace) = (form.clone(), workspace.clone());
        dialog
            .title("Best of N")
            .w(px(560.))
            .child(form.clone())
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("race-cancel")
                            .small()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("race-start")
                            .small()
                            .primary()
                            .label("Start")
                            .disabled(!ready)
                            .on_click(move |_, window, cx| {
                                let (prompt, chosen) = {
                                    let form = form_start.read(cx);
                                    (
                                        form.prompt.read(cx).value().trim().to_string(),
                                        form.chosen.clone(),
                                    )
                                };
                                window.close_dialog(cx);
                                let _ = workspace.update(cx, |this, cx| {
                                    this.start_race(project, prompt, chosen, window, cx)
                                });
                            }),
                    ),
            )
    });
}

/// What the comparison shows for one candidate.
#[derive(Clone, Default)]
struct Candidate {
    cost: f64,
    reply: String,
    files: usize,
    insertions: usize,
    deletions: usize,
}

/// The comparison panel for one best-of-N run.
pub struct RaceView {
    app: Entity<AppState>,
    workspace: WeakEntity<Workspace>,
    race: Option<Uuid>,
    /// What the candidates were asked.
    task: Option<String>,
    candidates: HashMap<ThreadId, Candidate>,
    /// The candidate whose changes were brought into the project.
    picked: Option<ThreadId>,
    picking: bool,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl RaceView {
    pub fn new(
        app: Entity<AppState>,
        workspace: WeakEntity<Workspace>,
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
        let subscriptions = vec![cx.observe(&app, |_, _, cx| cx.notify())];
        Self {
            app,
            workspace,
            race: None,
            task: None,
            candidates: HashMap::new(),
            picked: None,
            picking: false,
            _tasks: vec![poll],
            _subscriptions: subscriptions,
        }
    }

    pub fn set_race(&mut self, race: Uuid, cx: &mut Context<Self>) {
        if self.race != Some(race) {
            self.race = Some(race);
            self.task = None;
            self.candidates.clear();
            self.picked = None;
        }
        self.refresh(cx);
    }

    fn threads(&self, cx: &App) -> Vec<elyra_core::Thread> {
        let Some(race) = self.race else {
            return Vec::new();
        };
        let mut threads: Vec<_> = self
            .app
            .read(cx)
            .threads
            .iter()
            .filter(|t| t.race_id == Some(race))
            .cloned()
            .collect();
        threads.sort_by_key(|t| t.created_at);
        threads
    }

    /// Costs and replies from the sessions, and diff sizes from the worktrees.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        for thread in self.threads(cx) {
            let Some(session) = self.app.update(cx, |app, cx| app.session(thread.id, cx)) else {
                continue;
            };
            let (cost, reply) = {
                let session = session.read(cx);
                if self.task.is_none() {
                    self.task = session.items.iter().find_map(|item| match &item.content {
                        elyra_core::ItemContent::User { text, .. } => Some(text.clone()),
                        _ => None,
                    });
                }
                (session.spent_usd(), session.last_reply())
            };
            let entry = self.candidates.entry(thread.id).or_default();
            entry.cost = cost;
            entry.reply = reply;
            if let Environment::Worktree { path, .. } = thread.environment {
                let id = thread.id;
                let job = cx
                    .background_executor()
                    .spawn(async move { elyra_git::diff_stat(&path) });
                cx.spawn(async move |this, cx| {
                    if let Ok(stat) = job.await {
                        let _ = this.update(cx, |this, cx| {
                            let entry = this.candidates.entry(id).or_default();
                            entry.files = stat.files;
                            entry.insertions = stat.insertions;
                            entry.deletions = stat.deletions;
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
        }
        cx.notify();
    }

    fn confirm_pick(&mut self, id: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.app.read(cx).thread(id).cloned() else {
            return;
        };
        let project = self
            .app
            .read(cx)
            .project(thread.project_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let view = cx.entity().downgrade();
        let provider = thread.provider.label();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog
                .title(format!("Use {provider}'s changes?"))
                .w(px(460.))
                .child(div().text_sm().child(format!(
                    "Its changes are brought into {project} and staged, not committed, so you can review them in Changes first. Uncommitted changes in {project} that touch the same files may conflict."
                )))
                .footer(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("pick-cancel")
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("pick-confirm")
                                .small()
                                .primary()
                                .label("Use these changes")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = view.update(cx, |this, cx| this.pick(id, window, cx));
                                }),
                        ),
                )
        });
    }

    /// Commit the candidate's worktree and squash it into the project.
    fn pick(&mut self, id: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.app.read(cx);
        let Some(thread) = state.thread(id).cloned() else {
            return;
        };
        let Some(project) = state.project(thread.project_id).cloned() else {
            return;
        };
        let Environment::Worktree { path, branch } = thread.environment.clone() else {
            window.push_notification(
                Notification::error("This candidate has no worktree to take changes from."),
                cx,
            );
            return;
        };
        let label = thread.provider.label();
        self.picking = true;
        cx.notify();
        let job = cx.background_executor().spawn(async move {
            if elyra_git::diff_stat(&path).is_ok_and(|stat| stat.files > 0) {
                elyra_git::commit_all(&path, &format!("Best of N: {label}"))?;
            }
            let root = elyra_git::repo_root(&project.path)?;
            elyra_git::squash_merge(&root, &branch)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.picking = false;
                match result {
                    Ok(()) => {
                        this.picked = Some(id);
                        window.push_notification(
                            Notification::success(format!(
                                "{label}'s changes are staged in the project. Review and commit them in Changes."
                            )),
                            cx,
                        );
                    }
                    Err(err) => window.push_notification(
                        Notification::error(format!("Could not bring the changes in: {err:#}")),
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Delete the candidates that weren't picked, with their worktrees.
    fn discard_others(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keep = self.picked;
        for thread in self.threads(cx) {
            if Some(thread.id) == keep {
                continue;
            }
            let _ = self
                .workspace
                .update(cx, |workspace, cx| workspace.forget_thread(thread.id, cx));
            if let Err(err) = self
                .app
                .update(cx, |app, cx| app.delete_thread(thread.id, true, cx))
            {
                window.push_notification(Notification::error(format!("{err:#}")), cx);
            }
        }
        cx.notify();
    }

    fn card(&self, thread: &elyra_core::Thread, cx: &Context<Self>) -> AnyElement {
        let id = thread.id;
        let info = self.candidates.get(&id).cloned().unwrap_or_default();
        let (status, color) = match thread.status {
            ThreadStatus::Running => ("Working", cx.theme().info),
            ThreadStatus::NeedsApproval => ("Needs you", cx.theme().warning),
            ThreadStatus::Failed => ("Failed", cx.theme().danger),
            ThreadStatus::Interrupted => ("Interrupted", cx.theme().warning),
            ThreadStatus::Idle if info.reply.is_empty() => {
                ("Starting", cx.theme().muted_foreground)
            }
            ThreadStatus::Idle => ("Done", cx.theme().success),
        };
        let picked = self.picked == Some(id);
        let done = thread.status == ThreadStatus::Idle && !info.reply.is_empty();
        let reply: String = info.reply.chars().take(600).collect();
        let workspace = self.workspace.clone();
        v_flex()
            .flex_1()
            .min_w(px(240.))
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if picked {
                cx.theme().success
            } else {
                cx.theme().border
            })
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(thread.provider.label()),
                    )
                    .child(div().text_xs().text_color(color).child(status)),
            )
            .child(
                h_flex()
                    .gap_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{} files", info.files))
                    .child(
                        div()
                            .text_color(cx.theme().success)
                            .child(format!("+{}", info.insertions)),
                    )
                    .child(
                        div()
                            .text_color(cx.theme().danger)
                            .child(format!("−{}", info.deletions)),
                    )
                    .child(format_usd(info.cost)),
            )
            .child(
                div()
                    .id(SharedString::from(format!("race-reply-{id}")))
                    .flex_1()
                    .max_h(px(260.))
                    .overflow_y_scroll()
                    .text_xs()
                    .whitespace_normal()
                    .child(if reply.is_empty() {
                        "No reply yet.".to_string()
                    } else {
                        reply
                    }),
            )
            .child(
                h_flex()
                    .gap_1()
                    .justify_end()
                    .child(
                        Button::new(SharedString::from(format!("race-open-{id}")))
                            .small()
                            .ghost()
                            .icon(IconName::SquareArrowOutUpRight)
                            .label("Open")
                            .on_click(move |_, window, cx| {
                                let _ =
                                    workspace.update(cx, |this, cx| this.activate(id, window, cx));
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("race-pick-{id}")))
                            .small()
                            .primary()
                            .icon(IconName::Check)
                            .label(if picked { "Picked" } else { "Use this one" })
                            .disabled(!done || self.picking || self.picked.is_some())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_pick(id, window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for RaceView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let threads = self.threads(cx);
        let task = self.task.clone();
        let cards: Vec<AnyElement> = threads.iter().map(|t| self.card(t, cx)).collect();
        let others = threads.len().saturating_sub(1);
        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(IconName::Trophy).text_color(cx.theme().muted_foreground))
                    .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Best of N"))
                    .child(div().flex_1())
                    .when(self.picked.is_some() && others > 0, |this| {
                        this.child(
                            Button::new("race-discard")
                                .small()
                                .danger()
                                .icon(IconName::Trash)
                                .label(format!(
                                    "Delete the other {others} and their worktrees"
                                ))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.discard_others(window, cx)
                                })),
                        )
                    }),
            )
            .children(task.map(|task| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(task.chars().take(400).collect::<String>())
            }))
            .child(if cards.is_empty() {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No best-of-N run here yet. Start one with \u{201c}Best of N\u{2026}\u{201d} in the command palette.")
                    .into_any_element()
            } else {
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .gap_3()
                    .children(cards)
                    .into_any_element()
            })
    }
}
