//! The Context tab (⇧⌘I): thread notes, pinned messages, a generated recap,
//! the project's agent instructions and dev servers running in the project.

use crate::app_state::AppState;
use crate::thread_session::{ThreadSession, format_usd};
use elyra_core::{GoalStatus, ItemContent, ItemId};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
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
    /// The project's check command.
    check: Entity<InputState>,
    /// A check command guessed from the project's files.
    check_suggestion: Option<String>,
    /// The project's .mcp.json and skills, shared with every agent.
    mcp_json: Option<crate::shared_setup::McpJson>,
    skills: Vec<crate::shared_setup::Skill>,
    /// The project's saved browser journeys.
    journeys: Vec<(std::path::PathBuf, Result<crate::journeys::Journey, String>)>,
    goal: Entity<TextareaState>,
    budget: Entity<InputState>,
    servers: Vec<Server>,
    scanning: bool,
    /// The project's app in Grove, with its dev processes and caught mail.
    grove: Option<GroveInfo>,
    _save_notes: Option<Task<()>>,
    _save_instructions: Option<Task<()>>,
    _save_check: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ContextView {
    pub fn new(
        app: Entity<AppState>,
        session: Entity<ThreadSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (notes_text, instructions_text, check_text, check_suggestion) = {
            let session = session.read(cx);
            (
                session.thread.notes.clone().unwrap_or_default(),
                session.project.instructions.clone().unwrap_or_default(),
                session.project.check_command.clone().unwrap_or_default(),
                crate::checks::suggest(&session.working_dir()),
            )
        };
        let check = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(match &check_suggestion {
                    Some(suggestion) => format!("e.g. {suggestion}"),
                    None => "e.g. cargo test, php artisan test, npm test".to_string(),
                })
                .default_value(check_text)
        });
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
            cx.subscribe(&check, |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this._save_check = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(SAVE_DELAY).await;
                        let _ = this.update(cx, |this, cx| this.save_check(cx));
                    }));
                }
                InputEvent::PressEnter { .. } | InputEvent::Blur => this.save_check(cx),
                _ => {}
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
            check,
            check_suggestion,
            mcp_json: None,
            skills: Vec::new(),
            journeys: Vec::new(),
            goal,
            budget,
            servers: Vec::new(),
            scanning: false,
            grove: None,
            _save_notes: None,
            _save_instructions: None,
            _save_check: None,
            _subscriptions: subscriptions,
        };
        this.scan_servers(cx);
        this.load_shared(cx);
        this
    }

    fn load_shared(&mut self, cx: &mut Context<Self>) {
        let path = self.session.read(cx).project.path.clone();
        self.mcp_json = crate::shared_setup::mcp_json(&path);
        self.skills = crate::shared_setup::skills(&path);
        self.journeys = crate::journeys::load_all(&self.session.read(cx).working_dir());
        cx.notify();
    }

    fn render_journeys(&self, cx: &Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let busy = session.running || session.activity.is_some();
        let project = session.project.id;
        let auto = crate::journeys::replay_after_turn(&self.app.read(cx).store, project);
        let muted = |text: String| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .whitespace_normal()
                .child(text)
        };
        let mut section = v_flex()
            .gap_2()
            .child(
                Self::section("Browser journeys", cx)
                    .child(
                        Button::new("journeys-refresh")
                            .ghost()
                            .xsmall()
                            .icon(IconName::RefreshCw)
                            .tooltip("Read again")
                            .on_click(cx.listener(|this, _, _, cx| this.load_shared(cx))),
                    )
                    .when(!self.journeys.is_empty(), |this| {
                        this.child(
                            Button::new("journeys-run-all")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Play)
                                .label("Run all")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session
                                        .update(cx, |session, cx| session.run_journeys_now(None, cx))
                                })),
                        )
                    }),
            )
            .child(muted(
                "Flows an agent walked through in the browser and saved in .elyra/journeys. Replay them here, or after every turn that changed files: a broken flow goes back to the agent like a failing check.".into(),
            ));
        if self.journeys.is_empty() {
            return section
                .child(muted(
                    "None saved yet. Ask an agent to try a flow in the browser (with the dev server running) and save it as a journey.".into(),
                ))
                .into_any_element();
        }
        section = section.child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(
                    gpui_kit::component::switch::Switch::new("journeys-after-turn")
                        .checked(auto)
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            let on = *checked;
                            this.app.update(cx, |app, cx| {
                                crate::journeys::set_replay_after_turn(&app.store, project, on);
                                cx.notify();
                            });
                            cx.notify();
                        })),
                )
                .child("Replay after each turn"),
        );
        for (index, (path, journey)) in self.journeys.iter().enumerate() {
            section = section.child(match journey {
                Ok(journey) => {
                    let name = journey.name.clone();
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .font_weight(FontWeight::MEDIUM)
                                .child(journey.name.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} steps", journey.steps.len())),
                        )
                        .child(
                            Button::new(SharedString::from(format!("journey-run-{index}")))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Play)
                                .tooltip("Replay in the thread's browser")
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let name = name.clone();
                                    this.session.update(cx, |session, cx| {
                                        session.run_journeys_now(Some(name), cx)
                                    })
                                })),
                        )
                        .into_any_element()
                }
                Err(error) => muted(format!(
                    "{}: {error}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ))
                .into_any_element(),
            });
        }
        section.into_any_element()
    }

    /// Give the project's .mcp.json servers to every agent, or stop.
    fn share_mcp_json(&mut self, share: bool, cx: &mut Context<Self>) {
        let project = self.session.read(cx).project.id;
        let fingerprint = self.mcp_json.as_ref().map(|f| f.fingerprint.clone());
        self.app.update(cx, |app, cx| {
            match (share, fingerprint) {
                (true, Some(fingerprint)) => {
                    crate::shared_setup::allow(&app.store, project, &fingerprint)
                }
                _ => crate::shared_setup::revoke(&app.store, project),
            }
            app.restart_agents_in(project, cx);
        });
        self.load_shared(cx);
    }

    fn render_shared(&self, cx: &Context<Self>) -> AnyElement {
        let muted = |text: String| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .whitespace_normal()
                .child(text)
        };
        let project = self.session.read(cx).project.id;
        let allowed = crate::shared_setup::allowed(&self.app.read(cx).store, project)
            .filter(|f| !f.is_empty());
        let mut section = v_flex()
            .gap_2()
            .child(
                Self::section("Shared with every agent", cx).child(
                    Button::new("shared-refresh")
                        .ghost()
                        .xsmall()
                        .icon(IconName::RefreshCw)
                        .tooltip("Read again")
                        .on_click(cx.listener(|this, _, _, cx| this.load_shared(cx))),
                ),
            )
            .child(muted(
                "Claude Code reads the project's .mcp.json and skills itself. Elyra gives them to Codex, Elyra, Pi and ACP agents too.".into(),
            ));
        match &self.mcp_json {
            None => {
                section = section.child(muted("No .mcp.json in the project.".into()));
            }
            Some(file) => match &file.servers {
                Err(error) => section = section.child(muted(error.clone())),
                Ok(servers) => {
                    let shared = allowed.as_deref() == Some(file.fingerprint.as_str());
                    let changed = allowed.is_some() && !shared;
                    section = section.child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("MCP servers in .mcp.json ({})", servers.len())),
                    );
                    for server in servers {
                        section = section.child(
                            h_flex()
                                .gap_2()
                                .text_xs()
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(server.name.clone()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_color(cx.theme().muted_foreground)
                                        .child(server.summary.clone()),
                                ),
                        );
                    }
                    section = section.child(if shared {
                        h_flex()
                            .gap_2()
                            .child(muted("Given to every agent.".into()))
                            .child(
                                Button::new("mcp-json-stop")
                                    .ghost()
                                    .xsmall()
                                    .label("Stop sharing")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.share_mcp_json(false, cx)
                                    })),
                            )
                            .into_any_element()
                    } else {
                        v_flex()
                            .gap_1()
                            .child(muted(if changed {
                                "The file changed since you allowed it. Allow it again to keep sharing it.".into()
                            } else {
                                "These commands come with the repository and run on this Mac. Allow them only if you trust it.".into()
                            }))
                            .child(
                                h_flex().child(
                                    Button::new("mcp-json-allow")
                                        .small()
                                        .label("Allow for every agent")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.share_mcp_json(true, cx)
                                        })),
                                ),
                            )
                            .into_any_element()
                    });
                }
            },
        }
        let names: Vec<String> = self.skills.iter().map(|s| s.name.clone()).collect();
        section = section.child(muted(if names.is_empty() {
            "No skills in .claude/skills or .agents/skills (project or home).".into()
        } else {
            format!(
                "Skills listed for the other agents ({}): {}",
                names.len(),
                names.join(", ")
            )
        }));
        section.into_any_element()
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

    fn save_check(&mut self, cx: &mut Context<Self>) {
        self._save_check = None;
        let text = self.check.read(cx).value().trim().to_string();
        let command = Some(text).filter(|t| !t.is_empty());
        let mut project = self.session.read(cx).project.clone();
        if project.check_command == command {
            return;
        }
        project.check_command = command;
        self.app
            .update(cx, |app, cx| app.update_project(project, cx));
    }

    fn render_checks(&self, project_name: &str, cx: &Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let empty = self.check.read(cx).value().trim().is_empty();
        let running = session.running || session.activity.is_some();
        let suggestion = self.check_suggestion.clone().filter(|_| empty);
        v_flex()
            .gap_2()
            .child(
                Self::section("Checks after each turn", cx).when(!empty, |this| {
                    this.child(
                        Button::new("run-checks")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Play)
                            .label("Run now")
                            .disabled(running)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_check(cx);
                                this.session
                                    .update(cx, |session, cx| session.run_checks_now(cx));
                            })),
                    )
                }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "Runs in {project_name} after every turn that changed files. If it fails, the output goes back to the agent; the thread is done when it passes. Empty: off."
                    )),
            )
            .child(
                Input::new(&self.check)
                    .small()
                    .font_family(cx.theme().mono_font_family.clone()),
            )
            .when_some(suggestion, |this, suggestion| {
                this.child(
                    Button::new("use-check-suggestion")
                        .ghost()
                        .xsmall()
                        .label(format!("Use {suggestion}"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let suggestion = suggestion.clone();
                            this.check.update(cx, |input, cx| {
                                input.set_value(suggestion, window, cx)
                            });
                            this.save_check(cx);
                        })),
                )
            })
            .into_any_element()
    }

    pub fn scan_servers(&mut self, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        let root = self.session.read(cx).working_dir();
        let job = cx.background_executor().spawn(async move {
            // A worktree Grove runs has its own site.
            let grove = crate::grove::app_for(&root).map(|site| GroveInfo {
                dev: crate::grove::dev_running(&site.name),
                mail: crate::grove::mail_count(),
                site,
            });
            (local_servers(&root), grove)
        });
        cx.spawn(async move |this, cx| {
            let (servers, grove) = job.await;
            let _ = this.update(cx, |this, cx| {
                this.servers = servers;
                this.grove = grove;
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

    fn toggle_grove_dev(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.grove.clone() else {
            return;
        };
        self.scanning = true;
        cx.notify();
        let job = cx
            .background_executor()
            .spawn(async move { crate::grove::set_dev(&info.site.name, !info.dev) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(err) = result {
                    log::warn!("grove dev: {err:#}");
                }
                this.scanning = false;
                this.scan_servers(cx);
            });
        })
        .detach();
    }

    fn render_grove(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let info = self.grove.as_ref()?;
        let url = info.site.url();
        let open = url.clone();
        Some(
            v_flex()
                .gap_1()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .gap_2()
                        .text_sm()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Grove"))
                        .child(
                            div()
                                .id("grove-url")
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_color(cx.theme().link)
                                .cursor_pointer()
                                .child(url.trim_end_matches('/').to_string())
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(ContextEvent::OpenUrl(open.clone()))
                                })),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(info.site.driver.clone()),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(div().flex_1().child(if info.dev {
                            "Dev processes running (Vite, queue…)"
                        } else {
                            "Dev processes stopped"
                        }))
                        .child(
                            Button::new("grove-dev")
                                .xsmall()
                                .outline()
                                .label(if info.dev { "Stop" } else { "Start" })
                                .disabled(self.scanning)
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_grove_dev(cx))),
                        ),
                )
                .when(info.mail > 0, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} mail{} caught by Grove (grove mail)",
                                info.mail,
                                if info.mail == 1 { "" } else { "s" }
                            )),
                    )
                })
                .into_any_element(),
        )
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
                    .child(self.render_checks(&project_name, cx))
                    .child(self.render_journeys(cx))
                    .child(self.render_shared(cx))
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
                            .children(self.render_grove(cx))
                            .when(self.servers.is_empty() && self.grove.is_none() && !self.scanning, |this| {
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

/// The project's app in Grove.
#[derive(Clone)]
struct GroveInfo {
    site: crate::grove::Site,
    dev: bool,
    mail: usize,
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
