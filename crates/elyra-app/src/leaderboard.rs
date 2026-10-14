//! How each agent does on your own code: per instruction (a message from
//! you, with the automatic fixes after it), whether the project's checks were
//! green on the first try and in the end, how many fixes it took and what it
//! cost. From the transcripts, so it reflects your projects and tasks rather
//! than a benchmark.

use elyra_core::{ItemContent, ProjectId, Store, ThreadId, TranscriptItem};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How one instruction went.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outcome {
    /// The first checks after it passed (None: no checks ran).
    pub first_try: Option<bool>,
    /// The last checks passed.
    pub green: Option<bool>,
    /// Automatic fixes sent.
    pub fixes: usize,
    pub cost: f64,
}

/// One agent and model across instructions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub agent: String,
    /// None: the agent's default model.
    pub model: Option<String>,
    pub instructions: usize,
    /// Instructions the checks ran after.
    pub checked: usize,
    pub first_try: usize,
    pub green: usize,
    pub fixes: usize,
    pub cost: f64,
}

impl Row {
    pub fn label(&self) -> String {
        match &self.model {
            Some(model) => format!("{} · {model}", self.agent),
            None => self.agent.clone(),
        }
    }

    pub fn first_try_rate(&self) -> Option<f64> {
        (self.checked > 0).then(|| self.first_try as f64 / self.checked as f64)
    }

    pub fn cost_each(&self) -> f64 {
        self.cost / self.instructions.max(1) as f64
    }
}

/// Fewer checked instructions than this, and an agent isn't compared.
pub const MIN_CHECKED: usize = 3;

/// Each instruction in a transcript and how it went.
pub fn outcomes(items: &[TranscriptItem]) -> Vec<Outcome> {
    let mut outcomes: Vec<Outcome> = Vec::new();
    for item in items {
        match &item.content {
            ItemContent::User { text, .. } => {
                if crate::checks::fix_prompt_label(text).is_some() {
                    if let Some(last) = outcomes.last_mut() {
                        last.fixes += 1;
                    }
                } else {
                    outcomes.push(Outcome::default());
                }
            }
            ItemContent::Check { passed, .. } => {
                if let Some(last) = outcomes.last_mut() {
                    last.first_try.get_or_insert(*passed);
                    last.green = Some(*passed);
                }
            }
            ItemContent::TurnSummary {
                cost_usd: Some(cost),
                ..
            } => {
                if let Some(last) = outcomes.last_mut() {
                    last.cost += cost;
                }
            }
            _ => {}
        }
    }
    outcomes
}

/// Rows per agent and model, the most first-try green first (agents with too
/// few checked instructions last).
pub fn rows(outcomes: impl IntoIterator<Item = ((String, Option<String>), Outcome)>) -> Vec<Row> {
    let mut rows: HashMap<(String, Option<String>), Row> = HashMap::new();
    for ((agent, model), outcome) in outcomes {
        let row = rows
            .entry((agent.clone(), model.clone()))
            .or_insert_with(|| Row {
                agent,
                model,
                ..Default::default()
            });
        row.instructions += 1;
        row.fixes += outcome.fixes;
        row.cost += outcome.cost;
        if let Some(first_try) = outcome.first_try {
            row.checked += 1;
            row.first_try += usize::from(first_try);
        }
        if outcome.green == Some(true) {
            row.green += 1;
        }
    }
    let mut rows: Vec<Row> = rows.into_values().collect();
    rows.sort_by(|a, b| {
        let rank = |row: &Row| {
            (row.checked >= MIN_CHECKED)
                .then(|| row.first_try_rate())
                .flatten()
                .unwrap_or(-1.0)
        };
        rank(b)
            .total_cmp(&rank(a))
            .then_with(|| b.instructions.cmp(&a.instructions))
    });
    rows
}

/// The threads to count: (thread, agent, model), side chats left out.
pub type Threads = Vec<(ThreadId, String, Option<String>)>;

/// The rows for `threads`, read on a connection of its own. Blocking.
pub fn load(db: &Path, threads: Threads) -> anyhow::Result<Vec<Row>> {
    let store = Store::open_read_only(db)?;
    let mut all = Vec::new();
    for (thread, agent, model) in threads {
        let items = store.transcript(thread)?;
        all.extend(
            outcomes(&items)
                .into_iter()
                .map(|outcome| ((agent.clone(), model.clone()), outcome)),
        );
    }
    Ok(rows(all))
}

/// The threads of `project` (or every project) to count.
pub fn threads(app: &crate::app_state::AppState, project: Option<ProjectId>) -> Threads {
    app.threads
        .iter()
        .chain(app.archived_threads().iter())
        .filter(|t| t.parent_id.is_none())
        .filter(|t| project.is_none_or(|p| t.project_id == p))
        .map(|t| (t.id, t.provider.label().to_string(), t.model.clone()))
        .collect()
}

/// A line for a new thread: how the agents did in this project, when there
/// is enough to go on.
pub fn hint(rows: &[Row], project: &str) -> Option<String> {
    let compared: Vec<&Row> = rows.iter().filter(|r| r.checked >= MIN_CHECKED).collect();
    if compared.len() < 2 {
        return None;
    }
    let parts: Vec<String> = compared
        .iter()
        .take(3)
        .map(|row| {
            format!(
                "{} {} of {} ({} each)",
                row.label(),
                row.first_try,
                row.checked,
                crate::thread_session::format_usd(row.cost_each())
            )
        })
        .collect();
    Some(format!(
        "Checks green on the first try in {project}: {}.",
        parts.join(", ")
    ))
}

/// Rows per project, kept for a few minutes: a new thread asks for them, and
/// counting reads every transcript of the project.
type Cache = HashMap<ProjectId, (Instant, Vec<Row>)>;
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
const CACHE_FOR: Duration = Duration::from_secs(300);

pub fn cached(project: ProjectId) -> Option<Vec<Row>> {
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let (at, rows) = cache.as_ref()?.get(&project)?;
    (at.elapsed() < CACHE_FOR).then(|| rows.clone())
}

pub fn remember(project: ProjectId, rows: Vec<Row>) {
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(project, (Instant::now(), rows));
}

#[cfg(test)]
mod tests {
    use super::{Outcome, hint, outcomes, rows};
    use elyra_core::{ItemContent, TranscriptItem};

    fn item(content: ItemContent) -> TranscriptItem {
        TranscriptItem {
            id: elyra_core::new_id(),
            thread_id: elyra_core::new_id(),
            seq: 0,
            content,
            created_at: chrono::Utc::now(),
        }
    }

    fn user(text: &str) -> TranscriptItem {
        item(ItemContent::User {
            text: text.into(),
            checkpoint: None,
        })
    }

    fn check(passed: bool) -> TranscriptItem {
        item(ItemContent::Check {
            command: "cargo test".into(),
            passed,
            exit_code: Some(i32::from(!passed)),
            duration_ms: 1,
            output: String::new(),
        })
    }

    fn turn(cost: f64) -> TranscriptItem {
        item(ItemContent::TurnSummary {
            duration_ms: None,
            cost_usd: Some(cost),
            is_error: false,
            context_tokens: None,
            context_window: None,
            after: None,
        })
    }

    #[test]
    fn counts_how_each_instruction_went() {
        let fix = format!(
            "{}`cargo test` exited with 1. Fix the cause (automatic fix 1 of 2).",
            crate::checks::FIX_PROMPT_START
        );
        let items = vec![
            user("Round the totals"),
            turn(0.10),
            check(false),
            user(&fix),
            turn(0.05),
            check(true),
            user("Rename total()"),
            turn(0.02),
            check(true),
            user("Explain the cart"),
            turn(0.01),
        ];
        let found = outcomes(&items);
        assert_eq!(found.len(), 3, "a fix belongs to its instruction");
        assert_eq!(
            found[0],
            Outcome {
                first_try: Some(false),
                green: Some(true),
                fixes: 1,
                cost: 0.15000000000000002,
            }
        );
        assert_eq!((found[1].first_try, found[1].fixes), (Some(true), 0));
        assert_eq!(found[2].first_try, None, "no checks ran");

        let key = |agent: &str| (agent.to_string(), None);
        let green = Outcome {
            first_try: Some(true),
            green: Some(true),
            fixes: 0,
            cost: 0.1,
        };
        let red = Outcome {
            first_try: Some(false),
            green: Some(true),
            fixes: 1,
            cost: 0.4,
        };
        let rows = rows(vec![
            (key("Claude Code"), green),
            (key("Claude Code"), red),
            (key("Claude Code"), red),
            (key("Codex"), green),
            (key("Codex"), green),
            (key("Codex"), red),
            (key("Pi"), green),
        ]);
        assert_eq!(rows[0].agent, "Codex", "2 of 3 first try");
        assert_eq!(
            (rows[0].first_try, rows[0].checked, rows[0].fixes),
            (2, 3, 1)
        );
        assert_eq!(rows[1].agent, "Claude Code");
        assert_eq!(rows[2].agent, "Pi", "too few to compare: last");
        assert_eq!(
            hint(&rows, "shop").as_deref(),
            Some(
                "Checks green on the first try in shop: Codex 2 of 3 ($0.20 each), Claude Code 1 of 3 ($0.30 each)."
            )
        );
        assert_eq!(hint(&rows[..1], "shop"), None, "nothing to compare with");
    }
}
