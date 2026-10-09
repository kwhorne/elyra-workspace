//! "While you were away": back at the window after a while, one view of what
//! the agents did meanwhile — what needs you, what failed, what finished and
//! what is still going, with the cost — instead of opening every thread.

use chrono::{DateTime, Utc};
use elyra_core::{ItemContent, ThreadId, TranscriptItem};

/// Away at least this long before the summary is shown.
pub const AWAY: chrono::Duration = chrono::Duration::minutes(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    /// Waiting for an approval, an answer or a decision on a card.
    NeedsYou,
    Failed,
    Finished,
    Working,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub thread: ThreadId,
    pub title: String,
    pub project: String,
    pub state: State,
    /// What happened, in a line.
    pub line: String,
    /// What its turns cost meanwhile.
    pub cost: f64,
}

/// What a thread did since `since`, or None when nothing happened.
pub fn summarize(
    thread: &elyra_core::Thread,
    project: &str,
    items: &[TranscriptItem],
    since: DateTime<Utc>,
    running: bool,
) -> Option<Entry> {
    let recent: Vec<&TranscriptItem> = items.iter().filter(|i| i.created_at >= since).collect();
    if recent.is_empty() && !running {
        return None;
    }
    let mut turns = 0;
    let mut cost = 0.0;
    let mut failed: Option<String> = None;
    let mut waiting: Option<String> = None;
    for item in &recent {
        match &item.content {
            ItemContent::TurnSummary {
                is_error, cost_usd, ..
            } => {
                turns += 1;
                cost += cost_usd.unwrap_or(0.0);
                if *is_error {
                    failed = Some("The turn ended with an error.".into());
                }
            }
            ItemContent::Check {
                passed: false,
                command,
                ..
            } => {
                failed = Some(format!("Checks failed: {command}"));
            }
            ItemContent::Check { passed: true, .. } => failed = None,
            ItemContent::Journey {
                passed: false,
                name,
                ..
            } => failed = Some(format!("Journey failed: {name}")),
            _ => {}
        }
    }
    // Waiting for the user: anything undecided, wherever it is.
    for item in items {
        match &item.content {
            ItemContent::Approval {
                tool_name,
                decision: None,
                ..
            } => waiting = Some(format!("Wants to use {tool_name}")),
            ItemContent::Question {
                answer: None,
                request,
            } => {
                let question = request
                    .questions
                    .first()
                    .map(|q| q.question.clone())
                    .unwrap_or_else(|| "Has a question".into());
                waiting = Some(question);
            }
            ItemContent::AutomationProposal {
                automation,
                outcome: None,
            } => waiting = Some(format!("Proposes an automation: {}", automation.name)),
            ItemContent::RuleProposal {
                rule,
                outcome: None,
                ..
            } => waiting = Some(format!("Proposes a rule: {rule}")),
            ItemContent::PrUpdate {
                number,
                commits,
                requested,
                handled: false,
                ..
            } => waiting = Some(crate::review_follow::summary(*number, *commits, *requested)),
            _ => {}
        }
    }
    let reply = recent.iter().rev().find_map(|i| match &i.content {
        ItemContent::Assistant {
            text,
            parent_tool_use_id: None,
        } if !text.trim().is_empty() => Some(first_line(text)),
        _ => None,
    });
    let (state, line) = if let Some(waiting) = waiting {
        (State::NeedsYou, waiting)
    } else if let Some(failed) = failed {
        (State::Failed, failed)
    } else if running {
        (
            State::Working,
            reply.unwrap_or_else(|| "Still working.".into()),
        )
    } else if turns > 0 {
        (
            State::Finished,
            reply.unwrap_or_else(|| {
                format!(
                    "{turns} turn{} finished.",
                    if turns == 1 { "" } else { "s" }
                )
            }),
        )
    } else {
        return None;
    };
    Some(Entry {
        thread: thread.id,
        title: thread.title.clone(),
        project: project.to_string(),
        state,
        line: first_line(&line),
        cost,
    })
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_start_matches('#')
        .trim();
    if line.chars().count() > 140 {
        format!("{}…", line.chars().take(139).collect::<String>())
    } else {
        line.to_string()
    }
}

/// "1 h 20 min", "25 min".
pub fn duration(away: chrono::Duration) -> String {
    let minutes = away.num_minutes().max(1);
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::{State, duration, summarize};
    use chrono::{Duration, Utc};
    use elyra_core::{
        Environment, ItemContent, PermissionMode, ProviderKind, Store, TranscriptItem,
    };

    fn item(content: ItemContent, minutes_ago: i64) -> TranscriptItem {
        TranscriptItem {
            id: elyra_core::new_id(),
            thread_id: elyra_core::new_id(),
            seq: 0,
            content,
            created_at: Utc::now() - Duration::minutes(minutes_ago),
        }
    }

    #[test]
    fn says_what_each_thread_did() {
        let store = Store::open_in_memory().unwrap();
        let project = store.add_project(&std::env::temp_dir()).unwrap();
        let thread = store
            .create_thread(
                project.id,
                ProviderKind::Claude,
                None,
                PermissionMode::Ask,
                Environment::Local,
            )
            .unwrap();
        let since = Utc::now() - Duration::minutes(30);
        let done = |is_error: bool, minutes_ago| {
            item(
                ItemContent::TurnSummary {
                    duration_ms: None,
                    cost_usd: Some(0.25),
                    is_error,
                    context_tokens: None,
                    context_window: None,
                },
                minutes_ago,
            )
        };
        let reply = item(
            ItemContent::Assistant {
                text: "## Fixed the cart total\n\nDetails…".into(),
                parent_tool_use_id: None,
            },
            5,
        );
        let entry = summarize(
            &thread,
            "shop",
            &[reply.clone(), done(false, 4)],
            since,
            false,
        )
        .unwrap();
        assert_eq!(
            (entry.state, entry.line.as_str(), entry.cost),
            (State::Finished, "Fixed the cart total", 0.25)
        );

        let checks = item(
            ItemContent::Check {
                command: "php artisan test".into(),
                passed: false,
                exit_code: Some(1),
                duration_ms: 1,
                output: String::new(),
            },
            3,
        );
        let entry = summarize(
            &thread,
            "shop",
            &[reply.clone(), done(false, 4), checks],
            since,
            false,
        )
        .unwrap();
        assert_eq!(entry.state, State::Failed);
        assert_eq!(entry.line, "Checks failed: php artisan test");

        let rule = item(
            ItemContent::RuleProposal {
                rule: "Use Pest.".into(),
                reason: String::new(),
                outcome: None,
            },
            2,
        );
        let entry = summarize(
            &thread,
            "shop",
            &[reply, done(false, 4), rule],
            since,
            false,
        )
        .unwrap();
        assert_eq!(
            (entry.state, entry.line.as_str()),
            (State::NeedsYou, "Proposes a rule: Use Pest.")
        );

        // Nothing since: nothing to say.
        assert!(summarize(&thread, "shop", &[done(false, 90)], since, false).is_none());
        assert_eq!(
            summarize(&thread, "shop", &[], since, true).unwrap().state,
            State::Working
        );
    }

    #[test]
    fn says_how_long() {
        assert_eq!(duration(Duration::minutes(25)), "25 min");
        assert_eq!(duration(Duration::minutes(120)), "2 h");
        assert_eq!(duration(Duration::minutes(80)), "1 h 20 min");
    }
}
