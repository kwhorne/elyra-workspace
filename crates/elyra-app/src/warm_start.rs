//! Warm start, an experiment: before a new thread's first message reaches the
//! agent, Elyra looks the names in it up in the project's symbol index and
//! its earlier threads, and puts what it found first (where those symbols are
//! defined, which instructions last changed their files, earlier threads
//! about the same thing), so the agent needn't spend its first tool calls
//! finding them. Whether that helps is measured: half the new threads (by
//! their id) start without it, and the Stats panel compares the first turns
//! of both (tool calls before the first edit, time to it, cost). Off unless
//! switched on in Settings.

use elyra_core::{ItemContent, ProjectId, SearchFilter, Store, ThreadId, TranscriptItem};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// The notices that put a thread in the experiment.
pub const WITH: &str = "Warm start: the agent got";
pub const WITHOUT: &str = "Warm start experiment: this thread starts without the orientation";

const MAX_SYMBOLS: usize = 8;
const MAX_FILES: usize = 4;
const MAX_THREADS: usize = 2;

/// Whether a thread gets the orientation (half of them, by id).
pub fn gets_it(thread: ThreadId) -> bool {
    thread.as_u128().is_multiple_of(2)
}

/// The words in a message worth looking up, code-shaped ones first.
pub fn words(message: &str) -> Vec<String> {
    let mut code = Vec::new();
    let mut plain = Vec::new();
    let mut seen = HashSet::new();
    for raw in message.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':')) {
        let qualified = raw.contains("::");
        for word in raw.split("::").map(|w| w.trim_matches(':')) {
            if word.chars().count() < 4
                || !word
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
                || !seen.insert(word.to_lowercase())
            {
                continue;
            }
            let code_shaped =
                qualified || word.contains('_') || word.chars().skip(1).any(char::is_uppercase);
            if code_shaped {
                code.push(word.to_string());
            } else if word.chars().count() >= 5 {
                plain.push(word.to_lowercase());
            }
        }
    }
    code.truncate(10);
    plain.truncate(10);
    code.into_iter().chain(plain).collect()
}

/// What was found, and the text for the agent.
#[derive(Clone, Debug, PartialEq)]
pub struct Pack {
    pub text: String,
    pub symbols: usize,
    pub files: usize,
    pub threads: usize,
}

impl Pack {
    pub fn notice(&self) -> String {
        format!(
            "{WITH} {} symbol{}, the recent history of {} file{} and {} earlier thread{} with your message.",
            self.symbols,
            plural(self.symbols),
            self.files,
            plural(self.files),
            self.threads,
            plural(self.threads)
        )
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Look the message up: the symbol index as it is (not refreshed: the first
/// message mustn't wait), the line record and earlier threads. Blocking.
pub fn build(dir: &Path, project: ProjectId, thread: ThreadId, message: &str) -> Option<Pack> {
    let words = words(message);
    if words.is_empty() {
        return None;
    }
    let index_db = elyra_index::default_db_path(&elyra_core::paths::data_dir().join("index"), dir);
    let mut hits = Vec::new();
    if index_db.exists() {
        let _one_at_a_time = crate::gateway::SYMBOL_LOOKUPS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Ok(index) = elyra_index::SymbolIndex::open(dir, &index_db) {
            let mut seen = HashSet::new();
            for word in &words {
                for hit in index.definitions(word, true, None, 3).unwrap_or_default() {
                    if hits.len() < MAX_SYMBOLS && seen.insert((hit.file.clone(), hit.line)) {
                        hits.push(hit);
                    }
                }
            }
        }
    }
    let store = Store::open_read_only(&elyra_core::paths::database_path()).ok()?;
    let mut files: Vec<String> = Vec::new();
    for hit in &hits {
        if files.len() < MAX_FILES && !files.contains(&hit.file) {
            files.push(hit.file.clone());
        }
    }
    let mut history: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in &files {
        let relative = crate::provenance::repo_relative(dir, file)
            .map(|(_, relative)| relative)
            .unwrap_or_else(|| file.clone());
        for origin in store
            .file_origins(project, &relative, 2)
            .unwrap_or_default()
        {
            if origin.thread == thread {
                continue;
            }
            let Some(written) = store.thread(origin.thread).ok().flatten() else {
                continue;
            };
            let instruction = store
                .transcript(origin.thread)
                .unwrap_or_default()
                .into_iter()
                .find(|i| i.id == origin.item)
                .and_then(|i| match i.content {
                    ItemContent::User { text, .. } => Some(text),
                    _ => None,
                })
                .unwrap_or_default();
            history.entry(file.clone()).or_default().push(format!(
                "“{}” (thread “{}”, {}, {})",
                headline(&instruction, 100),
                written.title,
                written.provider.label(),
                origin.at.format("%Y-%m-%d")
            ));
        }
    }
    // Earlier threads about the same: the most specific words, together.
    let query: Vec<&str> = words.iter().take(2).map(String::as_str).collect();
    let filter = SearchFilter {
        project: Some(project),
        archived: true,
        messages_only: true,
    };
    let mut earlier: Vec<String> = Vec::new();
    let mut threads_seen = HashSet::from([thread]);
    for hit in store
        .search_transcripts(&query.join(" "), &filter, 10)
        .unwrap_or_default()
    {
        if earlier.len() >= MAX_THREADS || !threads_seen.insert(hit.thread) {
            continue;
        }
        let Some(found) = store.thread(hit.thread).ok().flatten() else {
            continue;
        };
        let snippet = hit.snippet.replace(['\u{2}', '\u{3}'], "");
        earlier.push(format!(
            "“{}” ({}): “{}”",
            found.title,
            hit.at.format("%Y-%m-%d"),
            headline(&snippet, 160)
        ));
    }
    if hits.is_empty() && earlier.is_empty() {
        return None;
    }
    let mut text = String::from(
        "<orientation>\nElyra Workspace looked the names in this message up in its index of the project and in earlier threads, to save you the first searches. It may be incomplete or out of date: read the code before relying on it.\n",
    );
    if !hits.is_empty() {
        text.push_str("\nDefined here:\n");
        for hit in &hits {
            let owner = hit
                .container
                .as_deref()
                .map(|c| format!("{c}::"))
                .unwrap_or_default();
            text.push_str(&format!(
                "- `{owner}{}` ({}) {}:{} — `{}`\n",
                hit.name,
                hit.kind,
                hit.file,
                hit.line,
                headline(&hit.signature, 120)
            ));
        }
    }
    if !history.is_empty() {
        text.push_str("\nLast changed by:\n");
        for (file, turns) in &history {
            text.push_str(&format!("- {file}: {}\n", turns.join("; ")));
        }
    }
    if !earlier.is_empty() {
        text.push_str("\nEarlier threads about this:\n");
        for thread in &earlier {
            text.push_str(&format!("- {thread}\n"));
        }
    }
    text.push_str("</orientation>");
    Some(Pack {
        text,
        symbols: hits.len(),
        files: history.len(),
        threads: earlier.len(),
    })
}

fn headline(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > max {
        format!("{}…", line.chars().take(max - 1).collect::<String>())
    } else {
        line
    }
}

// ---- measuring --------------------------------------------------------------------

/// How a thread's first turn went.
#[derive(Clone, Debug, PartialEq)]
pub struct FirstTurn {
    pub with: bool,
    /// Tool calls before the first edit (all of the turn's when it made none).
    pub calls_before_edit: usize,
    pub edited: bool,
    /// From the message to the first edit.
    pub seconds_to_edit: Option<f64>,
    pub cost: Option<f64>,
}

fn is_edit(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        "edit",
        "write",
        "multiedit",
        "notebookedit",
        "apply_patch",
        "filechange",
    ]
    .iter()
    .any(|tool| lower == *tool)
        || lower.starts_with("edit ")
        || lower.starts_with("write ")
}

/// The first turn of a thread in the experiment.
pub fn first_turn(items: &[TranscriptItem]) -> Option<FirstTurn> {
    let with = items.iter().find_map(|item| match &item.content {
        ItemContent::Notice { text, .. } if text.starts_with(WITH) => Some(true),
        ItemContent::Notice { text, .. } if text.starts_with(WITHOUT) => Some(false),
        _ => None,
    })?;
    let start = items
        .iter()
        .position(|i| matches!(i.content, ItemContent::User { .. }))?;
    let asked = items[start].created_at;
    let mut turn = FirstTurn {
        with,
        calls_before_edit: 0,
        edited: false,
        seconds_to_edit: None,
        cost: None,
    };
    for item in &items[start + 1..] {
        match &item.content {
            ItemContent::ToolUse {
                name,
                parent_tool_use_id: None,
                ..
            } if !turn.edited => {
                if is_edit(name) {
                    turn.edited = true;
                    turn.seconds_to_edit =
                        Some((item.created_at - asked).num_milliseconds() as f64 / 1000.);
                } else {
                    turn.calls_before_edit += 1;
                }
            }
            ItemContent::TurnSummary { cost_usd, .. } => {
                turn.cost = *cost_usd;
                return Some(turn);
            }
            ItemContent::User { .. } => break,
            _ => {}
        }
    }
    // Still running: not counted yet.
    None
}

/// One side of the comparison.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Side {
    pub threads: usize,
    pub median_calls: Option<f64>,
    pub median_seconds: Option<f64>,
    pub median_cost: Option<f64>,
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.
    } else {
        values[mid]
    })
}

/// With and without, from the first turns.
pub fn compare(turns: &[FirstTurn]) -> (Side, Side) {
    let side = |with: bool| {
        let turns: Vec<&FirstTurn> = turns.iter().filter(|t| t.with == with).collect();
        Side {
            threads: turns.len(),
            median_calls: median(turns.iter().map(|t| t.calls_before_edit as f64).collect()),
            median_seconds: median(turns.iter().filter_map(|t| t.seconds_to_edit).collect()),
            median_cost: median(turns.iter().filter_map(|t| t.cost).collect()),
        }
    };
    (side(true), side(false))
}

/// The experiment's first turns, read from the threads. Blocking.
pub fn load(db: &Path, threads: &[ThreadId]) -> anyhow::Result<Vec<FirstTurn>> {
    let store = Store::open_read_only(db)?;
    Ok(threads
        .iter()
        .filter_map(|id| first_turn(&store.transcript(*id).ok()?))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{FirstTurn, WITH, WITHOUT, compare, first_turn, words};
    use chrono::{Duration, Utc};
    use elyra_core::{ItemContent, TranscriptItem};

    #[test]
    fn looks_up_code_shaped_words_first() {
        assert_eq!(
            words(
                "Why does CartController::total round before the discount in apply_discount? Check the total."
            ),
            [
                "CartController",
                "total",
                "apply_discount",
                "round",
                "before",
                "discount",
                "check"
            ]
        );
        assert!(words("fix it").is_empty());
    }

    fn item(content: ItemContent, seconds: i64) -> TranscriptItem {
        TranscriptItem {
            id: elyra_core::new_id(),
            thread_id: elyra_core::new_id(),
            seq: 0,
            content,
            created_at: Utc::now() + Duration::seconds(seconds),
        }
    }

    fn tool(name: &str, seconds: i64) -> TranscriptItem {
        item(
            ItemContent::ToolUse {
                tool_use_id: String::new(),
                name: name.into(),
                input: serde_json::Value::Null,
                parent_tool_use_id: None,
            },
            seconds,
        )
    }

    #[test]
    fn measures_the_first_turn() {
        let notice = |text: &str| {
            item(
                ItemContent::Notice {
                    text: text.into(),
                    is_error: false,
                },
                0,
            )
        };
        let summary = item(
            ItemContent::TurnSummary {
                duration_ms: Some(30_000),
                cost_usd: Some(0.12),
                is_error: false,
                context_tokens: None,
                context_window: None,
                after: None,
            },
            40,
        );
        let user = item(
            ItemContent::User {
                text: "Round the totals".into(),
                checkpoint: None,
                attachments: Vec::new(),
            },
            0,
        );
        let items = vec![
            notice(&format!("{WITH} 3 symbols")),
            user.clone(),
            tool("Grep", 5),
            tool("Read", 8),
            tool("Edit", 20),
            tool("Read", 25),
            summary.clone(),
        ];
        let turn = first_turn(&items).unwrap();
        assert!(turn.with && turn.edited);
        assert_eq!(turn.calls_before_edit, 2);
        assert!((turn.seconds_to_edit.unwrap() - 20.).abs() < 1.);
        assert_eq!(turn.cost, Some(0.12));
        let without = vec![notice(WITHOUT), user, tool("Grep", 3), summary];
        let turn = first_turn(&without).unwrap();
        assert!(!turn.with && !turn.edited && turn.calls_before_edit == 1);
        let (with, without) = compare(&[
            FirstTurn {
                with: true,
                calls_before_edit: 2,
                edited: true,
                seconds_to_edit: Some(20.),
                cost: Some(0.1),
            },
            FirstTurn {
                with: false,
                calls_before_edit: 6,
                edited: true,
                seconds_to_edit: Some(40.),
                cost: Some(0.2),
            },
            FirstTurn {
                with: false,
                calls_before_edit: 4,
                edited: false,
                seconds_to_edit: None,
                cost: None,
            },
        ]);
        assert_eq!((with.threads, with.median_calls), (1, Some(2.)));
        assert_eq!(
            (
                without.threads,
                without.median_calls,
                without.median_seconds
            ),
            (2, Some(5.), Some(40.))
        );
    }
}
