//! Orchestration model: automations and their schedules, run history, the
//! task board, external MCP clients and the gateway audit log.

use crate::model::{PermissionMode, ProjectId, ProviderKind, ThreadId};
use chrono::{DateTime, Datelike, Duration, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

pub type AutomationId = Uuid;
pub type TaskId = Uuid;

/// The machine's IANA time zone name, falling back to UTC.
pub fn local_tz_name() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into())
}

fn parse_tz(name: &str) -> Tz {
    Tz::from_str(name).unwrap_or(Tz::UTC)
}

fn parse_time(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text.trim(), "%H:%M").ok()
}

/// When an automation runs. Wall-clock times are in `tz` (IANA name).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    Once {
        at: DateTime<Utc>,
    },
    Interval {
        minutes: u32,
    },
    Daily {
        time: String,
        tz: String,
    },
    /// Monday to Friday.
    Weekdays {
        time: String,
        tz: String,
    },
    /// `weekday` 0 = Monday … 6 = Sunday.
    Weekly {
        weekday: u8,
        time: String,
        tz: String,
    },
    /// Standard five-field cron (`min hour dom month dow`).
    Cron {
        expr: String,
        tz: String,
    },
}

impl Schedule {
    /// The first run strictly after `after`, or `None` when it never runs
    /// again (or the schedule is invalid).
    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Schedule::Once { at } => (*at > after).then_some(*at),
            Schedule::Interval { minutes } => {
                (*minutes > 0).then(|| after + Duration::minutes(*minutes as i64))
            }
            Schedule::Daily { time, tz } => next_matching(after, time, tz, |_| true),
            Schedule::Weekdays { time, tz } => next_matching(after, time, tz, |day| {
                !matches!(day, Weekday::Sat | Weekday::Sun)
            }),
            Schedule::Weekly { weekday, time, tz } => next_matching(after, time, tz, |day| {
                day.num_days_from_monday() == *weekday as u32
            }),
            Schedule::Cron { expr, tz } => {
                let schedule = cron_schedule(expr).ok()?;
                let tz = parse_tz(tz);
                schedule
                    .find_next_occurrence(&after.with_timezone(&tz), false)
                    .ok()
                    .map(|t| t.with_timezone(&Utc))
            }
        }
    }

    pub fn describe(&self) -> String {
        const DAYS: [&str; 7] = [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ];
        match self {
            Schedule::Once { at } => format!(
                "Once at {}",
                at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
            ),
            Schedule::Interval { minutes } if minutes % 60 == 0 => {
                format!("Every {} h", minutes / 60)
            }
            Schedule::Interval { minutes } => format!("Every {minutes} min"),
            Schedule::Daily { time, .. } => format!("Daily at {time}"),
            Schedule::Weekdays { time, .. } => format!("Weekdays at {time}"),
            Schedule::Weekly { weekday, time, .. } => {
                format!(
                    "{}s at {time}",
                    DAYS.get(*weekday as usize).unwrap_or(&"Monday")
                )
            }
            Schedule::Cron { expr, tz } => format!("cron {expr} ({tz})"),
        }
    }

    /// Check a schedule before saving it.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Schedule::Daily { time, .. }
            | Schedule::Weekdays { time, .. }
            | Schedule::Weekly { time, .. }
                if parse_time(time).is_none() =>
            {
                Err(format!("“{time}” is not a time like 09:30"))
            }
            Schedule::Interval { minutes: 0 } => {
                Err("The interval must be at least one minute".into())
            }
            Schedule::Cron { expr, .. } => cron_schedule(expr).map(|_| ()),
            _ => Ok(()),
        }
    }
}

fn cron_schedule(expr: &str) -> Result<croner::Cron, String> {
    if expr.split_whitespace().count() != 5 {
        return Err("Use five cron fields: minute hour day month weekday".into());
    }
    croner::Cron::new(expr)
        .parse()
        .map_err(|err| format!("Invalid cron expression: {err}"))
}

fn next_matching(
    after: DateTime<Utc>,
    time: &str,
    tz: &str,
    day_ok: impl Fn(Weekday) -> bool,
) -> Option<DateTime<Utc>> {
    let time = parse_time(time)?;
    let tz = parse_tz(tz);
    let local = after.with_timezone(&tz).date_naive();
    (0..8).find_map(|offset| {
        let date = local + Duration::days(offset);
        if !day_ok(date.weekday()) {
            return None;
        }
        let candidate = tz.from_local_datetime(&date.and_time(time)).earliest()?;
        let candidate = candidate.with_timezone(&Utc);
        (candidate > after).then_some(candidate)
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// Every run starts a fresh thread.
    #[default]
    NewThread,
    /// Every run continues the same thread.
    SameThread,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    /// Pause the automation after a failed run.
    #[default]
    Pause,
    /// Keep the schedule going.
    Continue,
    /// Try once more right away, then pause if it fails again.
    RetryOnce,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Automation {
    pub id: AutomationId,
    pub name: String,
    pub project_id: ProjectId,
    pub provider: ProviderKind,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    pub prompt: String,
    pub schedule: Schedule,
    #[serde(default)]
    pub run_mode: RunMode,
    /// The thread `SameThread` runs continue.
    #[serde(default)]
    pub thread_id: Option<ThreadId>,
    /// Stop (disable) once the agent's reply contains this text.
    #[serde(default)]
    pub stop_phrase: Option<String>,
    /// Stop after this many runs.
    #[serde(default)]
    pub max_runs: Option<u32>,
    #[serde(default)]
    pub failure_policy: FailurePolicy,
    pub enabled: bool,
    #[serde(default)]
    pub next_run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub runs: u32,
    pub created_at: DateTime<Utc>,
}

impl Automation {
    pub fn new(
        name: String,
        project_id: ProjectId,
        provider: ProviderKind,
        prompt: String,
        schedule: Schedule,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: crate::model::new_id(),
            name,
            project_id,
            provider,
            model: None,
            permission_mode: PermissionMode::AcceptEdits,
            prompt,
            next_run_at: schedule.next_after(now),
            schedule,
            run_mode: RunMode::NewThread,
            thread_id: None,
            stop_phrase: None,
            max_runs: None,
            failure_policy: FailurePolicy::Pause,
            enabled: true,
            runs: 0,
            created_at: now,
        }
    }

    /// Whether the run limit has been reached.
    pub fn exhausted(&self) -> bool {
        self.max_runs.is_some_and(|max| self.runs >= max)
    }
}

/// One finished turn, for usage statistics.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnStat {
    /// `ProviderKind::as_str` of the thread.
    pub provider: String,
    pub at: DateTime<Utc>,
    pub duration_ms: Option<u64>,
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    /// The stop phrase appeared; the automation was switched off.
    Stopped,
    /// Not started (e.g. the previous run was still going).
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomationRun {
    pub id: Uuid,
    pub automation_id: AutomationId,
    pub thread_id: Option<ThreadId>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: RunStatus,
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Draft,
    InProgress,
    Done,
}

impl TaskStatus {
    pub const ALL: [TaskStatus; 3] = [TaskStatus::Draft, TaskStatus::InProgress, TaskStatus::Done];

    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::Draft => "Draft",
            TaskStatus::InProgress => "In progress",
            TaskStatus::Done => "Done",
        }
    }
}

/// A card on the task board; handed to an agent it gets a thread whose
/// progress drives the card's status.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub thread_id: Option<ThreadId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    pub fn new(project_id: ProjectId, title: String) -> Self {
        let now = Utc::now();
        Self {
            id: crate::model::new_id(),
            project_id,
            title,
            notes: String::new(),
            status: TaskStatus::Draft,
            thread_id: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientScope {
    /// List and read threads.
    ReadOnly,
    /// Also create threads, send messages, interrupt, rename and archive.
    Full,
}

/// An external MCP client (Claude Desktop, Codex, …) paired with Elyra.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpClient {
    pub id: Uuid,
    pub name: String,
    pub token: String,
    pub scope: ClientScope,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub at: DateTime<Utc>,
    /// Client name, or "agent: <thread title>" for the in-app gateway.
    pub client: String,
    pub tool: String,
    pub detail: String,
    pub ok: bool,
}

/// Progress toward a thread goal that auto-continues across turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Paused,
    Achieved,
    /// Stopped after reaching the turn budget.
    Exhausted,
}

impl GoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            GoalStatus::Active => "active",
            GoalStatus::Paused => "paused",
            GoalStatus::Achieved => "achieved",
            GoalStatus::Exhausted => "exhausted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [
            GoalStatus::Active,
            GoalStatus::Paused,
            GoalStatus::Achieved,
            GoalStatus::Exhausted,
        ]
        .into_iter()
        .find(|status| status.as_str() == value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn computes_next_runs() {
        // 2026-10-02 is a Friday.
        let now = utc("2026-10-02T10:00:00Z");
        let daily = Schedule::Daily {
            time: "09:30".into(),
            tz: "Europe/Oslo".into(),
        };
        // 09:30 in Oslo (UTC+2 in October) is 07:30 UTC, already past today.
        assert_eq!(daily.next_after(now), Some(utc("2026-10-03T07:30:00Z")));
        let weekdays = Schedule::Weekdays {
            time: "09:30".into(),
            tz: "Europe/Oslo".into(),
        };
        assert_eq!(
            weekdays.next_after(now),
            Some(utc("2026-10-05T07:30:00Z")),
            "skips the weekend"
        );
        let weekly = Schedule::Weekly {
            weekday: 2,
            time: "18:00".into(),
            tz: "UTC".into(),
        };
        assert_eq!(weekly.next_after(now), Some(utc("2026-10-07T18:00:00Z")));
        let interval = Schedule::Interval { minutes: 90 };
        assert_eq!(interval.next_after(now), Some(utc("2026-10-02T11:30:00Z")));
        let once = Schedule::Once {
            at: utc("2026-10-02T09:00:00Z"),
        };
        assert_eq!(once.next_after(now), None, "a past one-off never runs");
        let cron = Schedule::Cron {
            expr: "15 */6 * * *".into(),
            tz: "UTC".into(),
        };
        assert_eq!(cron.next_after(now), Some(utc("2026-10-02T12:15:00Z")));
        // Standard numbering: 1-5 is Monday to Friday.
        let workdays = Schedule::Cron {
            expr: "0 9 * * 1-5".into(),
            tz: "UTC".into(),
        };
        assert_eq!(workdays.next_after(now), Some(utc("2026-10-05T09:00:00Z")));
        // Daylight saving ends 2026-10-25 in Oslo: 09:30 becomes 08:30 UTC.
        let after_dst = utc("2026-10-25T12:00:00Z");
        assert_eq!(
            daily.next_after(after_dst),
            Some(utc("2026-10-26T08:30:00Z"))
        );
    }

    #[test]
    fn validates_schedules() {
        assert!(
            Schedule::Daily {
                time: "25:00".into(),
                tz: "UTC".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Schedule::Cron {
                expr: "* * *".into(),
                tz: "UTC".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Schedule::Cron {
                expr: "0 9 * * 1-5".into(),
                tz: "UTC".into()
            }
            .validate()
            .is_ok()
        );
        assert!(Schedule::Interval { minutes: 0 }.validate().is_err());
    }
}
