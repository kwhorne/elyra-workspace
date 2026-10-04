//! A thread working on a Félagi issue: a banner above the composer with the
//! issue, its status and the running timer, and the report that writes back
//! what was done, the new status and the hours.

use crate::felagi::{self, Client, Connection, Issue, Timer};
use crate::thread_session::ThreadSession;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::Duration;

/// How often the banner asks Félagi again.
const REFRESH: Duration = Duration::from_secs(120);

/// The issue a thread works on, as Félagi last said.
pub struct FelagiLink {
    session: Entity<ThreadSession>,
    id: String,
    connection: Option<Connection>,
    issue: Option<Issue>,
    timer: Option<Timer>,
    error: Option<String>,
    _poll: Task<()>,
}

impl FelagiLink {
    pub fn new(session: Entity<ThreadSession>, id: String, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(REFRESH).await;
            }
        });
        Self {
            session,
            id,
            connection: None,
            issue: None,
            timer: None,
            error: None,
            _poll: poll,
        }
    }

    fn client(&mut self, cx: &App) -> Option<Client> {
        let app = crate::preferences::app_state(cx)?;
        self.connection = felagi::connection(&app.read(cx).store);
        Client::for_connection(self.connection.as_ref()?)
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client(cx) else {
            self.error = Some("Félagi isn't connected (Settings → Félagi).".into());
            cx.notify();
            return;
        };
        let id = self.id.clone();
        let job = cx
            .background_executor()
            .spawn(async move { anyhow::Ok((client.issue(&id)?, client.timer()?)) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok((issue, timer)) => {
                        this.issue = Some(issue);
                        this.timer = timer;
                        this.error = None;
                    }
                    Err(err) => this.error = Some(format!("{err:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The timer, when it runs on this issue.
    fn own_timer(&self) -> Option<&Timer> {
        self.timer.as_ref().filter(|t| t.issue == self.id)
    }

    fn open_report(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.connection.clone() else {
            return;
        };
        let current = self
            .issue
            .as_ref()
            .map(|issue| issue.status.clone())
            .unwrap_or_else(|| "in_progress".into());
        let suggested_status = match current.as_str() {
            "backlog" | "todo" | "in_progress" => "in_review".to_string(),
            other => other.to_string(),
        };
        let minutes = self.own_timer().map(Timer::minutes);
        let link = cx.entity().downgrade();
        let (session, id) = (self.session.clone(), self.id.clone());
        let title = self
            .issue
            .as_ref()
            .map(|i| i.title.clone())
            .unwrap_or_default();
        let timer_running = self.own_timer().is_some();
        let report = cx.new(|cx| {
            Report::new(
                session,
                link,
                id,
                title,
                connection,
                current,
                suggested_status,
                minutes,
                timer_running,
                window,
                cx,
            )
        });
        report.update(cx, |report, cx| report.draft(window, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            let send = report.clone();
            dialog
                .title("Report to Félagi")
                .w(px(600.))
                .child(report.clone())
                .footer(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("felagi-report-cancel")
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("felagi-report-send")
                                .small()
                                .primary()
                                .label("Send to Félagi")
                                .on_click(move |_, window, cx| {
                                    send.update(cx, |report, cx| report.send(window, cx))
                                }),
                        ),
                )
        });
    }
}

impl Render for FelagiLink {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self
            .issue
            .as_ref()
            .map(|issue| felagi::status_label(&issue.status).to_string());
        let timer = self
            .own_timer()
            .map(|t| format!("⏱ {}", felagi::format_minutes(t.minutes())));
        let url = self
            .connection
            .as_ref()
            .map(|c| format!("{}/app/issues/{}", c.url, self.id))
            .unwrap_or_default();
        let can_write = self.connection.as_ref().is_some_and(|c| c.can_write);
        h_flex()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .text_xs()
            .child(Icon::new(IconName::SquareKanban).xsmall())
            .child(
                div()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(self.id.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_color(cx.theme().muted_foreground)
                    .child(match (&self.error, &self.issue) {
                        (Some(error), _) => error.clone(),
                        (None, Some(issue)) => issue.title.clone(),
                        (None, None) => "Asking Félagi…".into(),
                    }),
            )
            .children(status.map(|status| div().child(status)))
            .children(timer.map(|timer| div().text_color(cx.theme().info).child(timer)))
            .child(
                Button::new("felagi-issue-open")
                    .xsmall()
                    .ghost()
                    .icon(IconName::ExternalLink)
                    .tooltip("Open in Félagi")
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            )
            .child(
                Button::new("felagi-report")
                    .xsmall()
                    .outline()
                    .label("Report…")
                    .disabled(!can_write)
                    .tooltip(if can_write {
                        "Write back what was done, the status and the hours"
                    } else {
                        "The Félagi token is read-only"
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.open_report(window, cx))),
            )
    }
}

/// The report dialog's content.
pub struct Report {
    session: Entity<ThreadSession>,
    link: WeakEntity<FelagiLink>,
    id: String,
    title: String,
    connection: Connection,
    current_status: String,
    status: String,
    timer_running: bool,
    comment: Entity<TextareaState>,
    time: Entity<InputState>,
    /// The latest pictures of the page from around a turn (before, after).
    pictures: Vec<std::path::PathBuf>,
    attach_pictures: bool,
    drafting: bool,
    sending: bool,
    error: Option<String>,
}

impl Report {
    #[allow(clippy::too_many_arguments)]
    fn new(
        session: Entity<ThreadSession>,
        link: WeakEntity<FelagiLink>,
        id: String,
        title: String,
        connection: Connection,
        current_status: String,
        status: String,
        minutes: Option<u64>,
        timer_running: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let comment = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(6, 16)
                .placeholder("What was done, how it was checked, what is left")
        });
        let time = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("e.g. 1h 30m");
            if let Some(minutes) = minutes {
                state.set_value(felagi::format_minutes(minutes), window, cx);
            }
            state
        });
        let pictures = latest_pictures(&session.read(cx).items);
        Self {
            attach_pictures: !pictures.is_empty(),
            pictures,
            session,
            link,
            id,
            title,
            connection,
            current_status,
            status,
            timer_running,
            comment,
            time,
            drafting: false,
            sending: false,
            error: None,
        }
    }

    /// Let the thread's agent write a first version of the report.
    fn draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let provider = session.thread.provider;
        let cwd = session.working_dir();
        let conversation = crate::app_state::transcript_text(&session.items, 12_000);
        let prompt = format!(
            "Write a short report for the issue {} \u{201c}{}\u{201d} in our task tracker, about the work done in this conversation: what was changed, how it was checked, and anything left open. Plain text: two or three sentences, then a short list with lines starting with \u{201c}- \u{201d}. No headings, no greeting, no code fences.\n\nThe conversation:\n{conversation}",
            self.id, self.title
        );
        self.drafting = true;
        cx.notify();
        let job = cx.background_executor().spawn(async move {
            let stat = std::process::Command::new("git")
                .current_dir(&cwd)
                .args(["diff", "--stat", "HEAD"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            // The branch's pull request, for the report to link to.
            let pr = std::process::Command::new("gh")
                .current_dir(&cwd)
                .args(["pr", "view", "--json", "url", "--jq", ".url"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|url| url.starts_with("https://"));
            let mut prompt = format!("{prompt}\n\nUncommitted changes:\n{stat}");
            if let Some(pr) = &pr {
                prompt.push_str(&format!(
                    "\n\nThe work is in the pull request {pr}; end the report with the line \u{201c}Pull request: {pr}\u{201d}."
                ));
            }
            elyra_provider::generate_text(provider, &cwd, &prompt)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.drafting = false;
                match result {
                    // Don't overwrite what the user started writing meanwhile.
                    Ok(text) if this.comment.read(cx).value().trim().is_empty() => {
                        let text = text.trim().trim_matches('`').trim().to_string();
                        this.comment
                            .update(cx, |comment, cx| comment.set_value(text, window, cx));
                    }
                    Ok(_) => {}
                    Err(err) => this.error = Some(format!("The agent couldn't draft it: {err:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sending {
            return;
        }
        let text = self.comment.read(cx).value().trim().to_string();
        let time_text = self.time.read(cx).value().trim().to_string();
        let minutes = if time_text.is_empty() {
            None
        } else {
            match felagi::parse_duration(&time_text) {
                Some(minutes) => Some(minutes),
                None => {
                    self.error = Some(format!(
                        "\u{201c}{time_text}\u{201d} isn't a duration; write it like 1h 30m."
                    ));
                    cx.notify();
                    return;
                }
            }
        };
        let Some(client) = Client::for_connection(&self.connection) else {
            self.error = Some("The Félagi token isn't in the Keychain.".into());
            cx.notify();
            return;
        };
        let (id, status, current) = (
            self.id.clone(),
            self.status.clone(),
            self.current_status.clone(),
        );
        let timer_running = self.timer_running;
        let pictures = if self.attach_pictures {
            self.pictures.clone()
        } else {
            Vec::new()
        };
        let note = format!("Elyra Workspace: {}", self.session.read(cx).thread.title);
        self.sending = true;
        self.error = None;
        cx.notify();
        let job = cx.background_executor().spawn(async move {
            let mut done = Vec::new();
            // The hours are the ones in the dialog, so the clock is dropped
            // rather than stopped (stopping would log its own count).
            if timer_running {
                client.discard_timer()?;
            }
            if let Some(minutes) = minutes.filter(|m| *m > 0) {
                client.log_time(&id, minutes, &note)?;
                done.push(format!("{} logged", felagi::format_minutes(minutes)));
            }
            if !text.is_empty() || !pictures.is_empty() {
                let body = if text.is_empty() {
                    "<p>Pictures of the page before and after the change.</p>".to_string()
                } else {
                    felagi::text_to_html(&text)
                };
                let comment = client.comment(&id, &body)?;
                done.push("comment posted".into());
                let mut attached = 0;
                for picture in pictures.iter().filter(|p| p.exists()) {
                    client.attach_to_comment(&id, comment, picture)?;
                    attached += 1;
                }
                if attached > 0 {
                    done.push(format!(
                        "{attached} picture{} attached",
                        if attached == 1 { "" } else { "s" }
                    ));
                }
            }
            if status != current {
                client.set_status(&id, &status)?;
                done.push(format!("status {}", felagi::status_label(&status)));
            }
            anyhow::Ok(done)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.sending = false;
                match result {
                    Ok(done) => {
                        let summary = if done.is_empty() {
                            "nothing to send".to_string()
                        } else {
                            done.join(", ")
                        };
                        let id = this.id.clone();
                        this.session.update(cx, |session, cx| {
                            session.notice(
                                format!("Reported to Félagi on {id}: {summary}."),
                                false,
                                cx,
                            )
                        });
                        let _ = this.link.update(cx, |link, cx| link.refresh(cx));
                        window.close_dialog(cx);
                    }
                    Err(err) => this.error = Some(format!("{err:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for Report {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.weak_entity();
        let status_label: SharedString = felagi::status_label(&self.status).to_string().into();
        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().text_sm().child("What was done"))
                            .when(self.drafting, |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("The agent is writing a draft…"),
                                )
                            }),
                    )
                    .child(Textarea::new(&self.comment)),
            )
            .child(
                h_flex()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child("Status"))
                            .child(
                                Button::new("felagi-report-status")
                                    .small()
                                    .outline()
                                    .label(status_label)
                                    .dropdown_caret(true)
                                    .dropdown_menu(move |mut menu, _, _| {
                                        for (key, label) in felagi::STATUSES {
                                            let view = view.clone();
                                            menu = menu.item(PopupMenuItem::new(*label).on_click(
                                                move |_, _, cx| {
                                                    let _ = view.update(cx, |r, cx| {
                                                        r.status = key.to_string();
                                                        cx.notify();
                                                    });
                                                },
                                            ));
                                        }
                                        menu
                                    }),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child("Time to log"))
                            .child(Input::new(&self.time).small().w(px(160.))),
                    ),
            )
            .when(!self.pictures.is_empty(), |this| {
                this.child(
                    Checkbox::new("felagi-report-pictures")
                        .label(if self.pictures.len() > 1 {
                            "Attach the pictures of the page from before and after the last change"
                        } else {
                            "Attach the picture of the page after the last change"
                        })
                        .checked(self.attach_pictures)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.attach_pictures = *checked;
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.timer_running {
                        "Félagi's timer for this issue is replaced by the time above. Leave it empty to log nothing."
                    } else {
                        "Leave the time empty to log nothing."
                    }),
            )
            .children(self.error.clone().map(|error| {
                div().text_sm().text_color(cx.theme().danger).child(error)
            }))
            .when(self.sending, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Sending…"),
                )
            })
    }
}

/// The newest before/after pictures of the page in a thread (see
/// `ItemContent::PageSnapshot`), before first.
fn latest_pictures(items: &[elyra_core::TranscriptItem]) -> Vec<std::path::PathBuf> {
    items
        .iter()
        .rev()
        .find_map(|item| match &item.content {
            elyra_core::ItemContent::PageSnapshot { before, after, .. } => Some(
                before
                    .iter()
                    .chain(std::iter::once(after))
                    .map(std::path::PathBuf::from)
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}
