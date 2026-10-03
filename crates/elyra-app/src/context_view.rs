//! The Context tab (⇧⌘I): thread notes, pinned messages, a generated recap,
//! the project's agent instructions and dev servers running in the project.

use crate::app_state::AppState;
use crate::thread_session::{ThreadSession, format_usd};
use elyra_core::{GoalStatus, ItemContent, ItemId};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SAVE_DELAY: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub port: u16,
    pub pid: u32,
    pub command: String,
}

/// Parse `lsof -F pcn` output into (pid, command, names).
fn parse_lsof(output: &str) -> Vec<(u32, String, Vec<String>)> {
    let mut processes: Vec<(u32, String, Vec<String>)> = Vec::new();
    for line in output.lines() {
        let (tag, value) = line.split_at(line.len().min(1));
        match tag {
            "p" => {
                if let Ok(pid) = value.parse() {
                    processes.push((pid, String::new(), Vec::new()));
                }
            }
            "c" => {
                if let Some(last) = processes.last_mut() {
                    last.1 = value.to_string();
                }
            }
            "n" => {
                if let Some(last) = processes.last_mut() {
                    last.2.push(value.to_string());
                }
            }
            _ => {}
        }
    }
    processes
}

/// TCP servers listening on this machine whose working directory is inside
/// `root`.
pub fn local_servers(root: &Path) -> Vec<Server> {
    let run = |args: &[&str]| {
        std::process::Command::new("lsof")
            .args(args)
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let Some(listening) = run(&["-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pcn"]) else {
        return Vec::new();
    };
    let listening = parse_lsof(&listening);
    if listening.is_empty() {
        return Vec::new();
    }
    let pids = listening
        .iter()
        .map(|(pid, ..)| pid.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let cwds = run(&["-a", "-d", "cwd", "-p", &pids, "-F", "pn"]).unwrap_or_default();
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let inside: Vec<u32> = parse_lsof(&cwds)
        .into_iter()
        .filter(|(_, _, names)| {
            names
                .first()
                .is_some_and(|cwd| PathBuf::from(cwd).starts_with(&root))
        })
        .map(|(pid, ..)| pid)
        .collect();
    let mut servers: Vec<Server> = listening
        .into_iter()
        .filter(|(pid, ..)| inside.contains(pid))
        .flat_map(|(pid, command, names)| {
            names
                .into_iter()
                .filter_map(|name| name.rsplit(':').next()?.parse::<u16>().ok())
                .map(move |port| Server {
                    port,
                    pid,
                    command: command.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    servers.sort_by_key(|s| s.port);
    servers.dedup_by_key(|s| s.port);
    servers
}

pub enum ContextEvent {
    /// Show a local server in the thread's browser.
    OpenUrl(String),
}

impl EventEmitter<ContextEvent> for ContextView {}

pub struct ContextView {
    app: Entity<AppState>,
    session: Entity<ThreadSession>,
    notes: Entity<TextareaState>,
    instructions: Entity<TextareaState>,
    goal: Entity<TextareaState>,
    budget: Entity<InputState>,
    servers: Vec<Server>,
    scanning: bool,
    _save_notes: Option<Task<()>>,
    _save_instructions: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ContextView {
    pub fn new(
        app: Entity<AppState>,
        session: Entity<ThreadSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (notes_text, instructions_text) = {
            let session = session.read(cx);
            (
                session.thread.notes.clone().unwrap_or_default(),
                session.project.instructions.clone().unwrap_or_default(),
            )
        };
        let notes = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(4, 16)
                .placeholder("Notes for yourself about this thread…")
                .default_value(notes_text)
        });
        let instructions = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 14)
                .placeholder("Extra instructions every agent in this project gets…")
                .default_value(instructions_text)
        });
        let goal = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 8)
                .placeholder("e.g. All tests pass and the checkout flow handles discounts")
        });
        let budget = cx.new(|cx| InputState::new(window, cx).placeholder("Budget in US$, e.g. 5"));
        let subscriptions = vec![
            cx.subscribe_in(
                &budget,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.apply_budget(window, cx);
                    }
                },
            ),
            cx.subscribe(&notes, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this._save_notes = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(SAVE_DELAY).await;
                        let _ = this.update(cx, |this, cx| this.save_notes(cx));
                    }));
                }
            }),
            cx.subscribe(&instructions, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this._save_instructions = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(SAVE_DELAY).await;
                        let _ = this.update(cx, |this, cx| this.save_instructions(cx));
                    }));
                }
            }),
            cx.observe(&session, |_, _, cx| cx.notify()),
        ];
        let mut this = Self {
            app,
            session,
            notes,
            instructions,
            goal,
            budget,
            servers: Vec::new(),
            scanning: false,
            _save_notes: None,
            _save_instructions: None,
            _subscriptions: subscriptions,
        };
        this.scan_servers(cx);
        this
    }

    fn save_notes(&mut self, cx: &mut Context<Self>) {
        let text = self.notes.read(cx).value().to_string();
        self.session
            .update(cx, |session, cx| session.set_notes(text, cx));
    }

    fn save_instructions(&mut self, cx: &mut Context<Self>) {
        let text = self.instructions.read(cx).value().trim().to_string();
        let instructions = Some(text).filter(|t| !t.is_empty());
        let mut project = self.session.read(cx).project.clone();
        if project.instructions == instructions {
            return;
        }
        project.instructions = instructions;
        self.app
            .update(cx, |app, cx| app.update_project(project, cx));
    }

    pub fn scan_servers(&mut self, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        let root = self.session.read(cx).working_dir();
        let job = cx
            .background_executor()
            .spawn(async move { local_servers(&root) });
        cx.spawn(async move |this, cx| {
            let servers = job.await;
            let _ = this.update(cx, |this, cx| {
                this.servers = servers;
                this.scanning = false;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_budget(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .budget
            .read(cx)
            .value()
            .trim()
            .trim_start_matches('$')
            .replace(',', ".");
        let Ok(amount) = text.parse::<f64>() else {
            return;
        };
        self.budget
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.session
            .update(cx, |session, cx| session.set_budget(Some(amount), cx));
    }

    fn render_budget(&self, cx: &Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let spent = session.spent_usd();
        let budget = session.thread.budget_usd;
        let reached = budget.is_some_and(|b| spent >= b);
        let summary = match budget {
            Some(budget) => format!("{} of {} spent", format_usd(spent), format_usd(budget)),
            None => format!("{} spent, no limit", format_usd(spent)),
        };
        let fraction = budget.map(|b| (spent / b).clamp(0., 1.) as f32);
        let bar_color = if reached {
            cx.theme().danger
        } else if fraction.is_some_and(|f| f >= 0.8) {
            cx.theme().warning
        } else {
            cx.theme().info
        };
        v_flex()
            .gap_2()
            .child(Self::section("Budget", cx))
            .child(
                div()
                    .text_sm()
                    .when(reached, |this| this.text_color(cx.theme().danger))
                    .child(summary),
            )
            .children(fraction.map(|fraction| {
                div()
                    .h(px(4.))
                    .w_full()
                    .rounded_full()
                    .bg(cx.theme().muted)
                    .child(
                        div()
                            .h_full()
                            .w(relative(fraction))
                            .rounded_full()
                            .bg(bar_color),
                    )
            }))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("At the limit, goals stop and ask you; your own messages still go. Costs are as reported by the agent, and not every agent reports them."),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(div().flex_1().child(Input::new(&self.budget).small()))
                    .child(
                        Button::new("set-budget")
                            .small()
                            .label(if budget.is_some() { "Change" } else { "Set" })
                            .on_click(cx.listener(|this, _, window, cx| this.apply_budget(window, cx))),
                    )
                    .when(budget.is_some(), |this| {
                        this.child(
                            Button::new("clear-budget")
                                .small()
                                .ghost()
                                .label("Remove")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session
                                        .update(cx, |session, cx| session.set_budget(None, cx));
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_goal(&self, cx: &Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let budget = crate::preferences::Preferences::global(cx).goal_max_turns;
        let section = Self::section("Goal", cx);
        let Some(goal) = session.thread.goal.clone() else {
            return v_flex()
                .gap_2()
                .child(section)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "The agent keeps working toward it turn after turn (up to {budget} automatic turns) until it reports the goal achieved or needs you."
                        )),
                )
                .child(Textarea::new(&self.goal))
                .child(
                    h_flex().justify_end().child(
                        Button::new("start-goal")
                            .small()
                            .primary()
                            .icon(IconName::Target)
                            .label("Start goal")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let text = this.goal.read(cx).value().to_string();
                                this.goal.update(cx, |goal, cx| goal.set_value("", window, cx));
                                this.session
                                    .update(cx, |session, cx| session.set_goal(Some(text), cx));
                            })),
                    ),
                )
                .into_any_element();
        };
        let status = session.thread.goal_status.unwrap_or(GoalStatus::Paused);
        let runs = session.thread.goal_runs;
        let (label, color) = match status {
            GoalStatus::Active => (format!("Working · {runs}/{budget} turns"), cx.theme().info),
            GoalStatus::Paused => ("Paused".to_string(), cx.theme().warning),
            GoalStatus::Achieved => ("Achieved".to_string(), cx.theme().success),
            GoalStatus::Exhausted => (format!("Stopped after {budget} turns"), cx.theme().warning),
        };
        let active = status == GoalStatus::Active;
        v_flex()
            .gap_2()
            .child(section)
            .child(
                v_flex()
                    .p_2()
                    .gap_1()
                    .rounded_md()
                    .border_1()
                    .border_color(color.opacity(0.5))
                    .child(div().text_sm().child(goal))
                    .child(div().text_xs().text_color(color).child(label)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .justify_end()
                    .child(
                        Button::new("goal-toggle")
                            .small()
                            .icon(if active {
                                IconName::Pause
                            } else {
                                IconName::Play
                            })
                            .label(if active { "Pause" } else { "Resume" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.session
                                    .update(cx, |session, cx| session.pause_goal(active, cx));
                            })),
                    )
                    .child(
                        Button::new("goal-clear")
                            .small()
                            .ghost()
                            .label("Clear")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session
                                    .update(cx, |session, cx| session.set_goal(None, cx));
                            })),
                    ),
            )
            .into_any_element()
    }

    fn section(title: &'static str, cx: &Context<Self>) -> Div {
        h_flex().gap_2().child(
            div()
                .flex_1()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child(title.to_uppercase()),
        )
    }

    fn pinned_rows(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let session = self.session.read(cx);
        session
            .thread
            .pinned_items
            .iter()
            .filter_map(|id| {
                let item = session.items.iter().find(|item| item.id == *id)?;
                let (who, text) = match &item.content {
                    ItemContent::User { text, .. } => ("You", text.clone()),
                    ItemContent::Assistant { text, .. } => ("Agent", text.clone()),
                    _ => return None,
                };
                Some((*id, who, text))
            })
            .map(|(id, who, text): (ItemId, &str, String)| {
                let preview: String = text.chars().take(600).collect();
                v_flex()
                    .p_2()
                    .gap_1()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(who),
                            )
                            .child(
                                Button::new(SharedString::from(format!("unpin-{id}")))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::PinOff)
                                    .tooltip("Unpin")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.session
                                            .update(cx, |session, cx| session.toggle_pin(id, cx))
                                    })),
                            ),
                    )
                    .child(div().text_sm().child(TextView::markdown(
                        SharedString::from(format!("pinned-{id}")),
                        preview,
                    )))
                    .into_any_element()
            })
            .collect()
    }
}

impl Render for ContextView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let recap = session.thread.recap.clone();
        let recapping = session.recapping;
        let project_name = session.project.name.clone();
        let restart_note = session.needs_restart;
        let pinned = self.pinned_rows(cx);
        let servers = self.servers.iter().map(|server| {
            let url = format!("http://localhost:{}", server.port);
            let open_url = url.clone();
            h_flex()
                .gap_2()
                .text_sm()
                .child(
                    Icon::new(IconName::Globe)
                        .xsmall()
                        .text_color(cx.theme().success),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("server-{}", server.port)))
                        .text_color(cx.theme().link)
                        .cursor_pointer()
                        .child(url)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(ContextEvent::OpenUrl(open_url.clone()))
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{} · pid {}", server.command, server.pid)),
                )
        });
        div()
            .id("context-view")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .p_3()
                    .gap_4()
                    .child(self.render_goal(cx))
                    .child(self.render_budget(cx))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(Self::section("Notes", cx))
                            .child(Textarea::new(&self.notes)),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                Self::section("Recap", cx).child(
                                    Button::new("generate-recap")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Sparkles)
                                        .loading(recapping)
                                        .label(if recap.is_some() { "Refresh" } else { "Generate" })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.session
                                                .update(cx, |session, cx| session.generate_recap(cx))
                                        })),
                                ),
                            )
                            .child(match recap {
                                Some(recap) => div()
                                    .text_sm()
                                    .child(TextView::markdown("thread-recap", recap))
                                    .into_any_element(),
                                None => div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("A short summary of the goal, progress and open items.")
                                    .into_any_element(),
                            }),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(Self::section("Pinned messages", cx))
                            .when(pinned.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Hover a message and press the pin to keep it here."),
                                )
                            })
                            .children(pinned),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(Self::section("Project instructions", cx))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "Appended to the system prompt of every agent in {project_name}."
                                    )),
                            )
                            .child(Textarea::new(&self.instructions))
                            .when(restart_note, |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Takes effect from the next message."),
                                )
                            }),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                Self::section("Local servers", cx).child(
                                    Button::new("scan-servers")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::RefreshCw)
                                        .loading(self.scanning)
                                        .tooltip("Scan again")
                                        .on_click(cx.listener(|this, _, _, cx| this.scan_servers(cx))),
                                ),
                            )
                            .when(self.servers.is_empty() && !self.scanning, |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("No dev servers are running in this folder."),
                                )
                            })
                            .children(servers),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::parse_lsof;

    #[test]
    fn parses_lsof_field_output() {
        let out = "p123\ncnode\nn*:5173\nn[::1]:5173\np456\ncpython3\nn127.0.0.1:8000\n";
        let parsed = parse_lsof(out);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, 123);
        assert_eq!(parsed[0].1, "node");
        assert_eq!(parsed[0].2, vec!["*:5173", "[::1]:5173"]);
        assert_eq!(parsed[1].2, vec!["127.0.0.1:8000"]);
    }
}
