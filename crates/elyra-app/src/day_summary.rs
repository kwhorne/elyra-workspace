//! Today's work: the threads you worked in, the Félagi issues they were for
//! with the agents' time against the hours already logged (and a click to log
//! the rest), and today's commits. Copyable for a standup.

use crate::app_state::AppState;
use crate::felagi::{self, Client, Connection};
use chrono::{Local, NaiveDate};
use elyra_core::{ItemContent, ThreadId};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
struct ThreadDay {
    id: ThreadId,
    title: String,
    project: String,
    issue: Option<String>,
    /// Agent time today, from the turns' durations.
    minutes: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IssueDay {
    pub issue: String,
    pub agent_minutes: u64,
    pub logged: u64,
    pub threads: Vec<String>,
}

impl IssueDay {
    /// What is still to log: the agents' time, rounded up to five minutes,
    /// less what is logged already.
    pub fn suggested(&self) -> u64 {
        let rounded = self.agent_minutes.div_ceil(5) * 5;
        rounded.saturating_sub(self.logged)
    }
}

/// Issues worked on today, from the day's threads and what Félagi has logged.
pub fn issues_of(
    threads: &[(Option<String>, String, u64)],
    logged: &BTreeMap<String, u64>,
) -> Vec<IssueDay> {
    let mut by_issue: BTreeMap<String, IssueDay> = BTreeMap::new();
    for (issue, title, minutes) in threads {
        let Some(issue) = issue else { continue };
        let entry = by_issue.entry(issue.clone()).or_insert_with(|| IssueDay {
            issue: issue.clone(),
            agent_minutes: 0,
            logged: logged.get(issue).copied().unwrap_or(0),
            threads: Vec::new(),
        });
        entry.agent_minutes += minutes;
        entry.threads.push(title.clone());
    }
    // Issues logged today without a thread here still count.
    for (issue, minutes) in logged {
        by_issue.entry(issue.clone()).or_insert_with(|| IssueDay {
            issue: issue.clone(),
            agent_minutes: 0,
            logged: *minutes,
            threads: Vec::new(),
        });
    }
    by_issue.into_values().collect()
}

pub struct DaySummary {
    app: Entity<AppState>,
    date: NaiveDate,
    threads: Vec<ThreadDay>,
    issues: Vec<IssueDay>,
    commits: Vec<String>,
    connection: Option<Connection>,
    loading: bool,
    logging: Option<String>,
    error: Option<String>,
}

impl DaySummary {
    pub fn new(app: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            connection: felagi::connection(&app.read(cx).store),
            app,
            date: Local::now().date_naive(),
            threads: Vec::new(),
            issues: Vec::new(),
            commits: Vec::new(),
            loading: false,
            logging: None,
            error: None,
        };
        this.refresh(cx);
        this
    }

    fn today_threads(&self, cx: &App) -> (Vec<ThreadDay>, Vec<PathBuf>) {
        let state = self.app.read(cx);
        let is_today =
            |at: chrono::DateTime<chrono::Utc>| at.with_timezone(&Local).date_naive() == self.date;
        let mut threads = Vec::new();
        let mut repos = Vec::new();
        for thread in &state.threads {
            if !thread.last_activity_at.is_some_and(is_today) {
                continue;
            }
            let minutes: u64 = state
                .store
                .transcript(thread.id)
                .unwrap_or_default()
                .iter()
                .filter(|item| is_today(item.created_at))
                .filter_map(|item| match item.content {
                    ItemContent::TurnSummary { duration_ms, .. } => duration_ms,
                    _ => None,
                })
                .sum::<u64>()
                / 60_000;
            let project = state.project(thread.project_id);
            if let Some(project) = project
                && !repos.contains(&project.path)
            {
                repos.push(project.path.clone());
            }
            threads.push(ThreadDay {
                id: thread.id,
                title: thread.title.clone(),
                project: project.map(|p| p.name.clone()).unwrap_or_default(),
                issue: thread.felagi_issue.clone(),
                minutes,
            });
        }
        (threads, repos)
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let (threads, repos) = self.today_threads(cx);
        self.threads = threads;
        let client = self.connection.as_ref().and_then(Client::for_connection);
        let date = self.date.format("%Y-%m-%d").to_string();
        let pairs: Vec<(Option<String>, String, u64)> = self
            .threads
            .iter()
            .map(|t| (t.issue.clone(), t.title.clone(), t.minutes))
            .collect();
        self.loading = true;
        cx.notify();
        let job = cx.background_executor().spawn(async move {
            let logged = match &client {
                Some(client) => client
                    .my_minutes_on(&date)
                    .map_err(|err| format!("{err:#}")),
                None => Ok(BTreeMap::new()),
            };
            let mut commits = Vec::new();
            for repo in repos {
                let email = std::process::Command::new("git")
                    .current_dir(&repo)
                    .args(["config", "user.email"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                if email.is_empty() {
                    continue;
                }
                let name = repo
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Ok(output) = std::process::Command::new("git")
                    .current_dir(&repo)
                    .args([
                        "log",
                        "--all",
                        "--since=midnight",
                        "--author",
                        &email,
                        "--format=%h %s",
                    ])
                    .output()
                {
                    for line in String::from_utf8_lossy(&output.stdout).lines() {
                        commits.push(format!("{name}: {line}"));
                    }
                }
            }
            (logged, commits)
        });
        cx.spawn(async move |this, cx| {
            let (logged, commits) = job.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.commits = commits;
                match logged {
                    Ok(logged) => {
                        this.issues = issues_of(&pairs, &logged);
                        this.error = None;
                    }
                    Err(err) => {
                        this.issues = issues_of(&pairs, &BTreeMap::new());
                        this.error = Some(err);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn log(&mut self, issue: IssueDay, cx: &mut Context<Self>) {
        let Some(client) = self.connection.as_ref().and_then(Client::for_connection) else {
            return;
        };
        let minutes = issue.suggested();
        if minutes == 0 {
            return;
        }
        let note = if issue.threads.is_empty() {
            "Elyra Workspace".to_string()
        } else {
            format!("Elyra Workspace: {}", issue.threads.join("; "))
        };
        self.logging = Some(issue.issue.clone());
        cx.notify();
        let id = issue.issue.clone();
        let job = cx
            .background_executor()
            .spawn(async move { client.log_time(&id, minutes, &note) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.logging = None;
                if let Err(err) = result {
                    this.error = Some(format!("{err:#}"));
                }
                this.refresh(cx);
            });
        })
        .detach();
    }

    /// The day as text, for a standup or a chat.
    fn summary_text(&self) -> String {
        let mut out = format!("Work on {}\n", self.date.format("%A %e %B %Y"));
        if !self.issues.is_empty() {
            out.push_str("\nIssues:\n");
            for issue in &self.issues {
                out.push_str(&format!(
                    "- {}: {} logged{}\n",
                    issue.issue,
                    felagi::format_minutes(issue.logged),
                    if issue.threads.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", issue.threads.join("; "))
                    }
                ));
            }
        }
        let other: Vec<&ThreadDay> = self.threads.iter().filter(|t| t.issue.is_none()).collect();
        if !other.is_empty() {
            out.push_str("\nOther threads:\n");
            for thread in other {
                out.push_str(&format!("- {} ({})\n", thread.title, thread.project));
            }
        }
        if !self.commits.is_empty() {
            out.push_str("\nCommits:\n");
            for commit in &self.commits {
                out.push_str(&format!("- {commit}\n"));
            }
        }
        out
    }
}

impl Render for DaySummary {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let section = |title: &str| {
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(muted)
                .child(title.to_uppercase())
        };
        let can_write = self.connection.as_ref().is_some_and(|c| c.can_write);
        let issue_rows = self.issues.iter().map(|issue| {
            let suggested = issue.suggested();
            let logging = self.logging.as_deref() == Some(issue.issue.as_str());
            let row = issue.clone();
            h_flex()
                .gap_3()
                .text_sm()
                .child(
                    div()
                        .w(px(90.))
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(issue.issue.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_color(muted)
                        .child(issue.threads.join("; ")),
                )
                .child(div().text_xs().child(format!(
                    "agents {} · logged {}",
                    felagi::format_minutes(issue.agent_minutes),
                    felagi::format_minutes(issue.logged)
                )))
                .child(
                    Button::new(SharedString::from(format!("day-log-{}", issue.issue)))
                        .xsmall()
                        .outline()
                        .label(if suggested > 0 {
                            format!("Log {}", felagi::format_minutes(suggested))
                        } else {
                            "Logged".into()
                        })
                        .loading(logging)
                        .disabled(suggested == 0 || !can_write)
                        .on_click(cx.listener(move |this, _, _, cx| this.log(row.clone(), cx))),
                )
        });
        let other_threads = self
            .threads
            .iter()
            .filter(|t| t.issue.is_none())
            .map(|thread| {
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(div().flex_1().child(thread.title.clone()))
                    .child(div().text_xs().text_color(muted).child(format!(
                        "{} · agents {}",
                        thread.project,
                        felagi::format_minutes(thread.minutes)
                    )))
            });
        let text = self.summary_text();
        v_flex()
            .gap_3()
            .max_h(px(560.))
            .child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .text_color(muted)
                    .child(
                        div()
                            .flex_1()
                            .child(self.date.format("%A %e %B").to_string()),
                    )
                    .when(self.loading, |this| this.child("Asking Félagi…"))
                    .child(
                        Button::new("day-copy")
                            .xsmall()
                            .ghost()
                            .label("Copy summary")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                            }),
                    ),
            )
            .children(
                self.error
                    .clone()
                    .map(|error| div().text_xs().text_color(cx.theme().danger).child(error)),
            )
            .when(self.connection.is_none(), |this| {
                this.child(
                    div().text_xs().text_color(muted).child(
                        "Connect Félagi in Settings → Félagi to see and log hours per issue.",
                    ),
                )
            })
            .when(!self.issues.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .child(section("Félagi issues"))
                        .children(issue_rows),
                )
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(section("Other threads"))
                    .children(other_threads)
                    .when(self.threads.iter().all(|t| t.issue.is_some()), |this| {
                        this.child(div().text_sm().text_color(muted).child("None today."))
                    }),
            )
            .child(
                v_flex()
                    .id("day-commits")
                    .gap_1()
                    .overflow_y_scroll()
                    .child(section("Commits"))
                    .children(self.commits.iter().map(|c| {
                        div()
                            .text_xs()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(c.clone())
                    }))
                    .when(self.commits.is_empty(), |this| {
                        this.child(div().text_sm().text_color(muted).child("None today."))
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{IssueDay, issues_of};
    use std::collections::BTreeMap;

    #[test]
    fn suggests_what_is_left_to_log() {
        let threads = vec![
            (Some("ACM-1".to_string()), "Fix login".to_string(), 47),
            (Some("ACM-1".to_string()), "Login tests".to_string(), 10),
            (None, "Spike".to_string(), 30),
        ];
        let logged = BTreeMap::from([("ACM-1".to_string(), 30), ("ACM-9".to_string(), 60)]);
        let issues = issues_of(&threads, &logged);
        assert_eq!(issues.len(), 2);
        let first = &issues[0];
        assert_eq!(
            (first.issue.as_str(), first.agent_minutes, first.logged),
            ("ACM-1", 57, 30)
        );
        assert_eq!(first.threads, ["Fix login", "Login tests"]);
        // 57 rounds up to 60, less the 30 logged.
        assert_eq!(first.suggested(), 30);
        let other = IssueDay {
            issue: "ACM-9".into(),
            agent_minutes: 0,
            logged: 60,
            threads: Vec::new(),
        };
        assert_eq!(issues[1], other);
        assert_eq!(other.suggested(), 0);
    }
}
