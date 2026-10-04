//! Automations: prompts that run on a schedule, each run in a new thread or
//! continuing the same one, with stop conditions, run limits, a failure
//! policy and run history. Runs while the app is open.

use crate::app_state::AppState;
use crate::thread_session::SessionEvent;
use crate::workspace::Workspace;
use chrono::Utc;
use elyra_core::{
    Automation, AutomationId, AutomationRun, FailurePolicy, ItemContent, PermissionMode, ProjectId,
    ProviderKind, RunMode, RunStatus, Schedule, ThreadStatus,
};
use elyra_provider::Prompt;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::HashMap;
use std::time::Duration;

const TICK: Duration = Duration::from_secs(20);

struct ActiveRun {
    run: AutomationRun,
    retry: bool,
    _subscription: Subscription,
}

#[derive(Default)]
struct Scheduler {
    active: HashMap<AutomationId, ActiveRun>,
}

impl Global for Scheduler {}

/// Start the scheduler loop.
pub fn init(app: Entity<AppState>, cx: &mut App) {
    cx.set_global(Scheduler::default());
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            cx.update(|cx| tick(&app, cx));
            cx.background_executor().timer(TICK).await;
        }
    })
    .detach();
}

fn tick(app: &Entity<AppState>, cx: &mut App) {
    let now = Utc::now();
    let due: Vec<Automation> = app
        .read(cx)
        .store
        .automations()
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.enabled && a.next_run_at.is_some_and(|t| t <= now))
        .collect();
    for automation in due {
        start_run(app, automation, false, false, cx);
    }
}

fn is_active(id: AutomationId, cx: &App) -> bool {
    cx.try_global::<Scheduler>()
        .is_some_and(|s| s.active.contains_key(&id))
}

/// Run an automation now, outside its schedule.
pub fn run_now(app: &Entity<AppState>, id: AutomationId, cx: &mut App) {
    let automation = app
        .read(cx)
        .store
        .automations()
        .unwrap_or_default()
        .into_iter()
        .find(|a| a.id == id);
    if let Some(automation) = automation {
        start_run(app, automation, true, false, cx);
    }
}

fn save(app: &Entity<AppState>, automation: &Automation, cx: &mut App) {
    if let Err(err) = app.read(cx).store.save_automation(automation) {
        log::error!("saving automation: {err:#}");
    }
    app.update(cx, |_, cx| cx.notify());
}

fn save_run(app: &Entity<AppState>, run: &AutomationRun, cx: &mut App) {
    if let Err(err) = app.read(cx).store.save_run(run) {
        log::error!("saving automation run: {err:#}");
    }
    app.update(cx, |_, cx| cx.notify());
}

fn start_run(
    app: &Entity<AppState>,
    mut automation: Automation,
    manual: bool,
    retry: bool,
    cx: &mut App,
) {
    let now = Utc::now();
    if !manual && !retry {
        automation.next_run_at = automation.schedule.next_after(now);
        if automation.next_run_at.is_none() {
            automation.enabled = false;
        }
    }
    let mut run = AutomationRun {
        id: elyra_core::new_id(),
        automation_id: automation.id,
        thread_id: None,
        started_at: now,
        finished_at: None,
        status: RunStatus::Running,
        message: None,
    };
    if is_active(automation.id, cx) {
        run.status = RunStatus::Skipped;
        run.finished_at = Some(now);
        run.message = Some("The previous run was still going.".into());
        save_run(app, &run, cx);
        save(app, &automation, cx);
        return;
    }
    let session = match prepare_thread(app, &mut automation, cx) {
        Ok(session) => session,
        Err(err) => {
            run.status = RunStatus::Failed;
            run.finished_at = Some(now);
            run.message = Some(err);
            save_run(app, &run, cx);
            if automation.failure_policy != FailurePolicy::Continue {
                automation.enabled = false;
            }
            save(app, &automation, cx);
            return;
        }
    };
    let (thread_id, busy) = {
        let session = session.read(cx);
        (
            session.thread.id,
            session.running || session.preparing.is_some(),
        )
    };
    run.thread_id = Some(thread_id);
    if busy {
        run.status = RunStatus::Skipped;
        run.finished_at = Some(now);
        run.message = Some("The thread was busy.".into());
        save_run(app, &run, cx);
        save(app, &automation, cx);
        return;
    }
    if !retry {
        automation.runs += 1;
    }
    save(app, &automation, cx);
    save_run(app, &run, cx);
    let id = automation.id;
    let watch_app = app.clone();
    let subscription = cx.subscribe(&session, move |session, event: &SessionEvent, cx| {
        on_session_event(&watch_app, id, &session, event, cx)
    });
    cx.update_global::<Scheduler, _>(|scheduler, _| {
        scheduler.active.insert(
            id,
            ActiveRun {
                run,
                retry,
                _subscription: subscription,
            },
        );
    });
    let prompt = automation.prompt.clone();
    session.update(cx, |session, cx| session.submit(Prompt::text(prompt), cx));
}

/// The session for this run: a new thread, or the automation's own one.
fn prepare_thread(
    app: &Entity<AppState>,
    automation: &mut Automation,
    cx: &mut App,
) -> Result<Entity<crate::thread_session::ThreadSession>, String> {
    if automation.run_mode == RunMode::SameThread
        && let Some(id) = automation.thread_id
        && app.read(cx).thread(id).is_some()
    {
        return app
            .update(cx, |app, cx| app.session(id, cx))
            .ok_or_else(|| "could not open the thread".to_string());
    }
    if app.read(cx).project(automation.project_id).is_none() {
        return Err("The automation's project was removed.".into());
    }
    let thread = app
        .update(cx, |app, cx| app.create_thread(automation.project_id, cx))
        .map_err(|e| format!("{e:#}"))?;
    let title = match automation.run_mode {
        RunMode::SameThread => automation.name.clone(),
        RunMode::NewThread => format!(
            "{} · {}",
            automation.name,
            chrono::Local::now().format("%d.%m %H:%M")
        ),
    };
    let (provider, model, mode) = (
        automation.provider,
        automation.model.clone(),
        automation.permission_mode,
    );
    app.update(cx, |app, cx| {
        app.update_thread(
            thread.id,
            |t| {
                t.provider = provider;
                t.model = model.clone();
                t.permission_mode = mode;
                t.title = title.clone();
            },
            cx,
        )
    });
    if automation.run_mode == RunMode::SameThread {
        automation.thread_id = Some(thread.id);
    }
    app.update(cx, |app, cx| app.session(thread.id, cx))
        .ok_or_else(|| "could not open the thread".to_string())
}

fn on_session_event(
    app: &Entity<AppState>,
    id: AutomationId,
    session: &Entity<crate::thread_session::ThreadSession>,
    event: &SessionEvent,
    cx: &mut App,
) {
    match event {
        SessionEvent::NeedsAttention => {
            let run = cx.update_global::<Scheduler, _>(|s, _| {
                s.active.get_mut(&id).map(|active| {
                    active.run.message = Some("Waiting for your input in the thread.".into());
                    active.run.clone()
                })
            });
            if let Some(run) = run {
                save_run(app, &run, cx);
            }
        }
        SessionEvent::TurnCompleted => {
            let session = session.read(cx);
            if session.running || !session.queued.is_empty() {
                return;
            }
            let failed = session.thread.status == ThreadStatus::Failed;
            let reply = last_reply(&session.items);
            finish(app, id, failed, reply, cx);
        }
        SessionEvent::TurnStarted | SessionEvent::SiteReady(_) => {}
    }
}

fn last_reply(items: &[elyra_core::TranscriptItem]) -> String {
    items
        .iter()
        .rev()
        .take_while(|item| !matches!(item.content, ItemContent::User { .. }))
        .find_map(|item| match &item.content {
            ItemContent::Assistant {
                text,
                parent_tool_use_id: None,
            } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn finish(app: &Entity<AppState>, id: AutomationId, failed: bool, reply: String, cx: &mut App) {
    let Some(active) = cx.update_global::<Scheduler, _>(|s, _| s.active.remove(&id)) else {
        return;
    };
    let Some(mut automation) = app
        .read(cx)
        .store
        .automations()
        .unwrap_or_default()
        .into_iter()
        .find(|a| a.id == id)
    else {
        return;
    };
    let mut run = active.run;
    run.finished_at = Some(Utc::now());
    let stop = automation
        .stop_phrase
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .is_some_and(|phrase| reply.to_lowercase().contains(&phrase.to_lowercase()));
    let mut retry = false;
    if failed {
        run.status = RunStatus::Failed;
        run.message = Some("The turn ended with an error.".into());
        match automation.failure_policy {
            FailurePolicy::Pause => automation.enabled = false,
            FailurePolicy::Continue => {}
            FailurePolicy::RetryOnce if !active.retry => retry = true,
            FailurePolicy::RetryOnce => automation.enabled = false,
        }
    } else if stop {
        run.status = RunStatus::Stopped;
        run.message = Some("The stop phrase appeared; the automation was switched off.".into());
        automation.enabled = false;
    } else {
        run.status = RunStatus::Succeeded;
        run.message = None;
    }
    if automation.exhausted() {
        automation.enabled = false;
    }
    save_run(app, &run, cx);
    save(app, &automation, cx);
    if retry {
        start_run(app, automation, true, true, cx);
    }
}

// ---- view ---------------------------------------------------------------

pub struct AutomationsView {
    app: Entity<AppState>,
    workspace: WeakEntity<Workspace>,
    selected: Option<AutomationId>,
    _subscriptions: Vec<Subscription>,
}

impl AutomationsView {
    pub fn new(
        app: Entity<AppState>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![cx.observe(&app, |_, _, cx| cx.notify())];
        Self {
            app,
            workspace,
            selected: None,
            _subscriptions: subscriptions,
        }
    }

    fn row(&self, automation: &Automation, cx: &Context<Self>) -> AnyElement {
        let id = automation.id;
        let selected = self.selected == Some(id);
        let running = is_active(id, cx);
        let state = self.app.read(cx);
        let project = state
            .project(automation.project_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "missing project".into());
        let next = match (automation.enabled, automation.next_run_at) {
            (false, _) => "Off".to_string(),
            (true, Some(at)) => format!(
                "Next {}",
                at.with_timezone(&chrono::Local).format("%a %d.%m %H:%M")
            ),
            (true, None) => "No further runs".to_string(),
        };
        let last = state
            .store
            .runs(id, 1)
            .ok()
            .and_then(|runs| runs.into_iter().next());
        let (status_icon, status_color) = match last.as_ref().map(|r| r.status) {
            _ if running => (IconName::LoaderCircle, cx.theme().info),
            Some(RunStatus::Succeeded) => (IconName::CircleCheck, cx.theme().success),
            Some(RunStatus::Failed) => (IconName::CircleX, cx.theme().danger),
            Some(RunStatus::Stopped) => (IconName::CircleStop, cx.theme().warning),
            Some(RunStatus::Skipped) => (IconName::CircleSlash, cx.theme().muted_foreground),
            _ => (IconName::CircleDashed, cx.theme().muted_foreground),
        };
        let enabled = automation.enabled;
        let toggle_app = self.app.clone();
        h_flex()
            .id(SharedString::from(format!("automation-{id}")))
            .gap_3()
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .when(selected, |this| this.bg(cx.theme().accent))
            .hover(|this| this.bg(cx.theme().accent.opacity(0.5)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(id);
                cx.notify();
            }))
            .child(Icon::new(status_icon).small().text_color(status_color))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(automation.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} · {} · {} · {} runs",
                                automation.schedule.describe(),
                                project,
                                automation.provider.label(),
                                automation.runs
                            )),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(next),
            )
            .child(
                Switch::new(SharedString::from(format!("enabled-{id}")))
                    .checked(enabled)
                    .on_click(move |checked, _, cx| {
                        let checked = *checked;
                        let automation = toggle_app
                            .read(cx)
                            .store
                            .automations()
                            .unwrap_or_default()
                            .into_iter()
                            .find(|a| a.id == id);
                        if let Some(mut automation) = automation {
                            automation.enabled = checked;
                            if checked {
                                automation.next_run_at = automation.schedule.next_after(Utc::now());
                            }
                            save(&toggle_app, &automation, cx);
                        }
                    }),
            )
            .into_any_element()
    }

    fn detail(&self, automation: &Automation, cx: &Context<Self>) -> AnyElement {
        let id = automation.id;
        let runs = self.app.read(cx).store.runs(id, 50).unwrap_or_default();
        let no_runs = runs.is_empty();
        let running = is_active(id, cx);
        let (run_app, edit_app, delete_app) =
            (self.app.clone(), self.app.clone(), self.app.clone());
        let edit_automation = automation.clone();
        let workspace = self.workspace.clone();
        let history = runs.into_iter().map(|run| {
            let label = match run.status {
                RunStatus::Running => "Running",
                RunStatus::Succeeded => "Succeeded",
                RunStatus::Failed => "Failed",
                RunStatus::Stopped => "Stopped",
                RunStatus::Skipped => "Skipped",
            };
            let duration = run
                .finished_at
                .map(|end| format!(" · {} s", (end - run.started_at).num_seconds().max(0)))
                .unwrap_or_default();
            let thread = run.thread_id;
            let workspace = workspace.clone();
            h_flex()
                .id(SharedString::from(format!("run-{}", run.id)))
                .gap_2()
                .py_1()
                .border_b_1()
                .border_color(cx.theme().border)
                .text_sm()
                .child(
                    div()
                        .w(px(130.))
                        .flex_none()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            run.started_at
                                .with_timezone(&chrono::Local)
                                .format("%d.%m %H:%M")
                                .to_string(),
                        ),
                )
                .child(
                    div()
                        .w(px(150.))
                        .flex_none()
                        .child(format!("{label}{duration}")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(run.message.clone().unwrap_or_default()),
                )
                .when_some(thread, |this, thread| {
                    this.child(
                        Button::new(SharedString::from(format!("open-run-{}", run.id)))
                            .xsmall()
                            .ghost()
                            .label("Open thread")
                            .on_click(move |_, window, cx| {
                                let _ = workspace.update(cx, |w, cx| {
                                    w.open_thread_from_panel(thread, window, cx)
                                });
                            }),
                    )
                })
        });
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(automation.name.clone()),
                    )
                    .child(
                        Button::new("automation-run-now")
                            .small()
                            .icon(IconName::Play)
                            .label("Run now")
                            .disabled(running)
                            .on_click(move |_, _, cx| run_now(&run_app, id, cx)),
                    )
                    .child(
                        Button::new("automation-edit")
                            .small()
                            .icon(IconName::Pencil)
                            .label("Edit")
                            .on_click(move |_, window, cx| {
                                edit(edit_app.clone(), Some(edit_automation.clone()), window, cx)
                            }),
                    )
                    .child(
                        Button::new("automation-delete")
                            .small()
                            .ghost()
                            .icon(IconName::Trash)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = delete_app.read(cx).store.delete_automation(id);
                                this.selected = None;
                                delete_app.update(cx, |_, cx| cx.notify());
                            })),
                    ),
            )
            .child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .text_sm()
                    .child(automation.prompt.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(describe_policy(automation)),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Run history"),
            )
            .child(
                div()
                    .id("automation-runs")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(history)
                    .when(no_runs, |this| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No runs yet."),
                        )
                    }),
            )
            .into_any_element()
    }
}

fn describe_policy(automation: &Automation) -> String {
    let mut parts = vec![match automation.run_mode {
        RunMode::NewThread => "Each run starts a new thread".to_string(),
        RunMode::SameThread => "Runs continue the same thread".to_string(),
    }];
    if let Some(phrase) = automation.stop_phrase.as_deref().filter(|p| !p.is_empty()) {
        parts.push(format!("stops when the reply contains “{phrase}”"));
    }
    if let Some(max) = automation.max_runs {
        parts.push(format!("at most {max} runs"));
    }
    parts.push(
        match automation.failure_policy {
            FailurePolicy::Pause => "pauses after a failure",
            FailurePolicy::Continue => "keeps going after failures",
            FailurePolicy::RetryOnce => "retries a failed run once",
        }
        .into(),
    );
    parts.push(format!(
        "permissions: {}",
        automation.permission_mode.label()
    ));
    parts.join(" · ")
}

impl Render for AutomationsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let automations = self.app.read(cx).store.automations().unwrap_or_default();
        if self.selected.is_none() {
            self.selected = automations.first().map(|a| a.id);
        }
        let selected = automations
            .iter()
            .find(|a| Some(a.id) == self.selected)
            .cloned();
        let rows: Vec<AnyElement> = automations.iter().map(|a| self.row(a, cx)).collect();
        let new_app = self.app.clone();
        let empty = automations.is_empty();
        h_flex()
            .size_full()
            .child(
                v_flex()
                    .w(px(460.))
                    .flex_none()
                    .h_full()
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .px_3()
                            .py_2()
                            .gap_2()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(Icon::new(IconName::CalendarClock).small())
                            .child(div().flex_1().text_sm().font_weight(FontWeight::SEMIBOLD).child("Automations"))
                            .child(
                                Button::new("new-automation")
                                    .small()
                                    .primary()
                                    .icon(IconName::Plus)
                                    .label("New")
                                    .on_click(move |_, window, cx| edit(new_app.clone(), None, window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("automation-list")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p_1()
                            .children(rows)
                            .when(empty, |this| {
                                this.child(
                                    div()
                                        .p_4()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Run a prompt on a schedule — nightly test runs, dependency checks, weekly summaries. Automations run while Elyra Workspace is open."),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .p_4()
                    .child(match selected {
                        Some(automation) => self.detail(&automation, cx),
                        None => div().into_any_element(),
                    }),
            )
    }
}

// ---- editor -------------------------------------------------------------

const SCHEDULE_KINDS: &[(&str, &str)] = &[
    ("daily", "Daily"),
    ("weekdays", "Weekdays"),
    ("weekly", "Weekly"),
    ("interval", "Every N minutes"),
    ("once", "Once"),
    ("cron", "Cron"),
];
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

struct AutomationForm {
    existing: Option<Automation>,
    name: Entity<InputState>,
    prompt: Entity<TextareaState>,
    time: Entity<InputState>,
    minutes: Entity<InputState>,
    once: Entity<InputState>,
    cron: Entity<InputState>,
    stop_phrase: Entity<InputState>,
    max_runs: Entity<InputState>,
    project: Option<ProjectId>,
    provider: ProviderKind,
    kind: &'static str,
    weekday: u8,
    run_mode: RunMode,
    failure: FailurePolicy,
    mode: PermissionMode,
    error: Option<String>,
}

impl AutomationForm {
    fn new(
        app: &Entity<AppState>,
        existing: Option<Automation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input =
            |placeholder: &str, value: String, window: &mut Window, cx: &mut Context<Self>| {
                let placeholder = placeholder.to_string();
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(placeholder)
                        .default_value(value)
                })
            };
        let a = existing.as_ref();
        let (kind, time, minutes, once, cron, weekday) = match a.map(|a| &a.schedule) {
            Some(Schedule::Daily { time, .. }) => (
                "daily",
                time.clone(),
                String::new(),
                String::new(),
                String::new(),
                0,
            ),
            Some(Schedule::Weekdays { time, .. }) => (
                "weekdays",
                time.clone(),
                String::new(),
                String::new(),
                String::new(),
                0,
            ),
            Some(Schedule::Weekly { weekday, time, .. }) => (
                "weekly",
                time.clone(),
                String::new(),
                String::new(),
                String::new(),
                *weekday,
            ),
            Some(Schedule::Interval { minutes }) => (
                "interval",
                String::new(),
                minutes.to_string(),
                String::new(),
                String::new(),
                0,
            ),
            Some(Schedule::Once { at }) => (
                "once",
                String::new(),
                String::new(),
                at.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string(),
                String::new(),
                0,
            ),
            Some(Schedule::Cron { expr, .. }) => (
                "cron",
                String::new(),
                String::new(),
                String::new(),
                expr.clone(),
                0,
            ),
            None => (
                "daily",
                "09:00".into(),
                "60".into(),
                String::new(),
                "0 9 * * 1-5".into(),
                0,
            ),
        };
        let prompt_text = a.map(|a| a.prompt.clone()).unwrap_or_default();
        let prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(4, 12)
                .placeholder("What should the agent do each time?")
                .default_value(prompt_text)
        });
        let first_project = app.read(cx).projects.first().map(|p| p.id);
        Self {
            name: input(
                "Name",
                a.map(|a| a.name.clone()).unwrap_or_default(),
                window,
                cx,
            ),
            prompt,
            time: input("09:00", time, window, cx),
            minutes: input("60", minutes, window, cx),
            once: input("2026-12-24 08:00", once, window, cx),
            cron: input("0 9 * * 1-5", cron, window, cx),
            stop_phrase: input(
                "e.g. ALL CLEAR",
                a.and_then(|a| a.stop_phrase.clone()).unwrap_or_default(),
                window,
                cx,
            ),
            max_runs: input(
                "unlimited",
                a.and_then(|a| a.max_runs)
                    .map(|m| m.to_string())
                    .unwrap_or_default(),
                window,
                cx,
            ),
            project: a.map(|a| a.project_id).or(first_project),
            provider: a.map(|a| a.provider).unwrap_or(ProviderKind::Claude),
            kind,
            weekday,
            run_mode: a.map(|a| a.run_mode).unwrap_or_default(),
            failure: a.map(|a| a.failure_policy).unwrap_or_default(),
            mode: a
                .map(|a| a.permission_mode)
                .unwrap_or(PermissionMode::AcceptEdits),
            existing,
            error: None,
        }
    }

    fn schedule(&self, cx: &App) -> Result<Schedule, String> {
        let tz = elyra_core::local_tz_name();
        let time = self.time.read(cx).value().trim().to_string();
        let schedule = match self.kind {
            "weekdays" => Schedule::Weekdays { time, tz },
            "weekly" => Schedule::Weekly {
                weekday: self.weekday,
                time,
                tz,
            },
            "interval" => Schedule::Interval {
                minutes: self
                    .minutes
                    .read(cx)
                    .value()
                    .trim()
                    .parse()
                    .map_err(|_| "Enter the interval in whole minutes".to_string())?,
            },
            "once" => {
                let text = self.once.read(cx).value().trim().to_string();
                let naive = chrono::NaiveDateTime::parse_from_str(&text, "%Y-%m-%d %H:%M")
                    .map_err(|_| "Enter the time as YYYY-MM-DD HH:MM".to_string())?;
                let local = naive
                    .and_local_timezone(chrono::Local)
                    .earliest()
                    .ok_or("That time doesn't exist in your time zone")?;
                Schedule::Once {
                    at: local.with_timezone(&Utc),
                }
            }
            "cron" => Schedule::Cron {
                expr: self.cron.read(cx).value().trim().to_string(),
                tz,
            },
            _ => Schedule::Daily { time, tz },
        };
        schedule.validate()?;
        Ok(schedule)
    }

    fn build(&self, cx: &App) -> Result<Automation, String> {
        let name = self.name.read(cx).value().trim().to_string();
        let prompt = self.prompt.read(cx).value().trim().to_string();
        if name.is_empty() {
            return Err("Give the automation a name".into());
        }
        if prompt.is_empty() {
            return Err("Write the prompt to run".into());
        }
        let project = self.project.ok_or("Pick a project")?;
        let schedule = self.schedule(cx)?;
        let max_runs = match self.max_runs.read(cx).value().trim() {
            "" => None,
            text => Some(
                text.parse::<u32>()
                    .map_err(|_| "Max runs must be a number".to_string())?,
            ),
        };
        let mut automation = match &self.existing {
            Some(existing) => existing.clone(),
            None => Automation::new(
                name.clone(),
                project,
                self.provider,
                prompt.clone(),
                schedule.clone(),
            ),
        };
        if automation.schedule != schedule || self.existing.is_none() {
            automation.next_run_at = schedule.next_after(Utc::now());
        }
        if automation.project_id != project || automation.run_mode != self.run_mode {
            automation.thread_id = None;
        }
        automation.name = name;
        automation.prompt = prompt;
        automation.project_id = project;
        automation.provider = self.provider;
        automation.schedule = schedule;
        automation.run_mode = self.run_mode;
        automation.failure_policy = self.failure;
        automation.permission_mode = self.mode;
        automation.max_runs = max_runs;
        automation.stop_phrase =
            Some(self.stop_phrase.read(cx).value().trim().to_string()).filter(|p| !p.is_empty());
        Ok(automation)
    }
}

fn picker<T: Clone + PartialEq + 'static>(
    id: &'static str,
    current: T,
    options: Vec<(T, String)>,
    form: WeakEntity<AutomationForm>,
    set: fn(&mut AutomationForm, T),
) -> impl IntoElement {
    let label = options
        .iter()
        .find(|(value, _)| *value == current)
        .map(|(_, label)| label.clone())
        .unwrap_or_default();
    Button::new(id)
        .small()
        .outline()
        .label(label)
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            for (value, label) in options.clone() {
                let form = form.clone();
                let checked = value == current;
                menu = menu.item(PopupMenuItem::new(label).checked(checked).on_click(
                    move |_, _, cx| {
                        let value = value.clone();
                        let _ = form.update(cx, |form, cx| {
                            set(form, value);
                            cx.notify();
                        });
                    },
                ));
            }
            menu
        })
}

fn field(label: &'static str, child: impl IntoElement) -> Div {
    v_flex()
        .gap_1()
        .child(div().text_sm().child(label))
        .child(child)
}

impl Render for AutomationForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        let state = crate::preferences::app_state(cx);
        let projects: Vec<(Option<ProjectId>, String)> = state
            .map(|app| {
                app.read(cx)
                    .projects
                    .iter()
                    .map(|p| (Some(p.id), p.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let providers: Vec<(ProviderKind, String)> = crate::preferences::Preferences::global(cx)
            .enabled_providers()
            .into_iter()
            .map(|k| (k, k.label().to_string()))
            .collect();
        let schedule_input: AnyElement = match self.kind {
            "interval" => field("Minutes", Input::new(&self.minutes).small()).into_any_element(),
            "once" => field("Date and time", Input::new(&self.once).small()).into_any_element(),
            "cron" => field(
                "Cron (minute hour day month weekday)",
                Input::new(&self.cron).small(),
            )
            .into_any_element(),
            "weekly" => h_flex()
                .gap_2()
                .child(field(
                    "Day",
                    picker(
                        "automation-weekday",
                        self.weekday,
                        (0..7)
                            .map(|d| (d, WEEKDAYS[d as usize].to_string()))
                            .collect(),
                        weak.clone(),
                        |f, v| f.weekday = v,
                    ),
                ))
                .child(field("Time", Input::new(&self.time).small().w(px(100.))))
                .into_any_element(),
            _ => field("Time", Input::new(&self.time).small().w(px(100.))).into_any_element(),
        };
        v_flex()
            .gap_3()
            .child(field("Name", Input::new(&self.name).small()))
            .child(field("Prompt", Textarea::new(&self.prompt)))
            .child(
                h_flex()
                    .gap_2()
                    .child(field(
                        "Project",
                        picker(
                            "automation-project",
                            self.project,
                            projects,
                            weak.clone(),
                            |f, v| f.project = v,
                        ),
                    ))
                    .child(field(
                        "Agent",
                        picker(
                            "automation-provider",
                            self.provider,
                            providers,
                            weak.clone(),
                            |f, v| f.provider = v,
                        ),
                    ))
                    .child(field(
                        "Permissions",
                        picker(
                            "automation-mode",
                            self.mode,
                            PermissionMode::ALL
                                .iter()
                                .map(|m| (*m, m.label().to_string()))
                                .collect(),
                            weak.clone(),
                            |f, v| f.mode = v,
                        ),
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(field(
                        "Schedule",
                        picker(
                            "automation-kind",
                            self.kind,
                            SCHEDULE_KINDS
                                .iter()
                                .map(|(k, l)| (*k, l.to_string()))
                                .collect(),
                            weak.clone(),
                            |f, v| f.kind = v,
                        ),
                    ))
                    .child(schedule_input),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(field(
                        "Each run",
                        picker(
                            "automation-run-mode",
                            self.run_mode,
                            vec![
                                (RunMode::NewThread, "New thread".to_string()),
                                (RunMode::SameThread, "Same thread".to_string()),
                            ],
                            weak.clone(),
                            |f, v| f.run_mode = v,
                        ),
                    ))
                    .child(field(
                        "If a run fails",
                        picker(
                            "automation-failure",
                            self.failure,
                            vec![
                                (FailurePolicy::Pause, "Pause".to_string()),
                                (FailurePolicy::Continue, "Keep going".to_string()),
                                (FailurePolicy::RetryOnce, "Retry once".to_string()),
                            ],
                            weak.clone(),
                            |f, v| f.failure = v,
                        ),
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        field(
                            "Stop when the reply contains",
                            Input::new(&self.stop_phrase).small(),
                        )
                        .flex_1(),
                    )
                    .child(field(
                        "Max runs",
                        Input::new(&self.max_runs).small().w(px(110.)),
                    )),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
    }
}

/// Open the automation editor (new when `existing` is None).
pub fn edit(
    app: Entity<AppState>,
    existing: Option<Automation>,
    window: &mut Window,
    cx: &mut App,
) {
    let title = if existing.is_some() {
        "Edit automation"
    } else {
        "New automation"
    };
    let form = cx.new(|cx| AutomationForm::new(&app, existing, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        let (form, app) = (form.clone(), app.clone());
        dialog.title(title).w(px(620.)).child(form.clone()).footer(
            h_flex().justify_end().child(
                Button::new("automation-save")
                    .primary()
                    .small()
                    .label("Save")
                    .on_click(move |_, window, cx| match form.read(cx).build(cx) {
                        Ok(automation) => {
                            save(&app, &automation, cx);
                            window.close_dialog(cx);
                            window.push_notification(
                                Notification::success(format!(
                                    "“{}” saved{}",
                                    automation.name,
                                    automation
                                        .next_run_at
                                        .map(|t| format!(
                                            " — next run {}",
                                            t.with_timezone(&chrono::Local)
                                                .format("%a %d.%m %H:%M")
                                        ))
                                        .unwrap_or_default()
                                )),
                                cx,
                            );
                        }
                        Err(error) => form.update(cx, |form, cx| {
                            form.error = Some(error);
                            cx.notify();
                        }),
                    }),
            ),
        )
    });
}
