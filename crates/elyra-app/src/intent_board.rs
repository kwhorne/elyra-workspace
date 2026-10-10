//! The intent board: what agents in other threads have been changing in the
//! same repository during the last hour, so parallel agents can stay out of
//! each other's way.
//!
//! Every agent's tool calls pass through Elyra Workspace, so the board is
//! kept here, in memory. A successful edit is recorded with the file and the
//! symbols it changed; the files a turn changed are recorded when it ends
//! (which also catches edits made from the shell). When two unrelated
//! threads touch the same file, both are told once; every prompt carries what
//! the others are changing, and agents can ask with the gateway's
//! `intent_board`. Best of N candidates and side chats are related to each
//! other and meant to overlap, so they don't count.

use chrono::{DateTime, Duration, Utc};
use elyra_core::ThreadId;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// How long an edit stays on the board.
pub fn recent() -> Duration {
    Duration::hours(1)
}

/// The most rows put in front of an agent.
const MAX_ROWS: usize = 12;

#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    pub name: String,
    /// First line of the definition, trimmed.
    pub signature: String,
}

#[derive(Clone, Debug)]
pub struct Intent {
    /// The repository's shared git directory: the same for all its worktrees.
    pub repo: PathBuf,
    /// The working tree the change was made in.
    pub root: PathBuf,
    pub branch: Option<String>,
    pub thread: ThreadId,
    /// Relative to `root`.
    pub path: String,
    pub symbols: Vec<Symbol>,
    pub at: DateTime<Utc>,
}

/// Another thread's change to the file just changed.
#[derive(Clone, Debug)]
pub struct Overlap {
    pub other: Intent,
    /// Symbols both changed.
    pub shared: Vec<String>,
    /// Symbols both changed whose first lines now differ: what a merge can't
    /// reconcile.
    pub differing: Vec<String>,
}

#[derive(Default)]
pub struct IntentBoard {
    intents: Vec<Intent>,
    /// (thread, thread, path) pairs already told about each other.
    told: HashSet<(ThreadId, ThreadId, String)>,
}

impl IntentBoard {
    /// Record a change. Returns the other threads' recent changes to the same
    /// file that the two haven't been told about yet; `related` says which
    /// threads are meant to overlap with this one.
    pub fn publish(&mut self, intent: Intent, related: impl Fn(ThreadId) -> bool) -> Vec<Overlap> {
        self.prune(intent.at);
        let mut overlaps = Vec::new();
        for other in &self.intents {
            if other.repo != intent.repo
                || other.thread == intent.thread
                || other.path != intent.path
                || related(other.thread)
            {
                continue;
            }
            let pair = if intent.thread < other.thread {
                (intent.thread, other.thread)
            } else {
                (other.thread, intent.thread)
            };
            if !self.told.insert((pair.0, pair.1, intent.path.clone())) {
                continue;
            }
            let shared: Vec<&Symbol> = intent
                .symbols
                .iter()
                .filter(|s| other.symbols.iter().any(|o| o.name == s.name))
                .collect();
            let differing = shared
                .iter()
                .filter(|s| {
                    other
                        .symbols
                        .iter()
                        .any(|o| o.name == s.name && o.signature != s.signature)
                })
                .map(|s| s.name.clone())
                .collect();
            overlaps.push(Overlap {
                other: other.clone(),
                shared: shared.into_iter().map(|s| s.name.clone()).collect(),
                differing,
            });
        }
        match self
            .intents
            .iter_mut()
            .find(|o| o.repo == intent.repo && o.thread == intent.thread && o.path == intent.path)
        {
            Some(existing) => {
                for symbol in intent.symbols {
                    match existing.symbols.iter_mut().find(|s| s.name == symbol.name) {
                        Some(known) => known.signature = symbol.signature,
                        None => existing.symbols.push(symbol),
                    }
                }
                existing.root = intent.root;
                existing.branch = intent.branch;
                existing.at = intent.at;
            }
            None => self.intents.push(intent),
        }
        overlaps
    }

    /// What threads other than `thread` changed in `repo` lately, newest first.
    pub fn others(
        &self,
        repo: &Path,
        thread: Option<ThreadId>,
        now: DateTime<Utc>,
        related: impl Fn(ThreadId) -> bool,
    ) -> Vec<&Intent> {
        let mut rows: Vec<&Intent> = self
            .intents
            .iter()
            .filter(|i| i.repo == repo && Some(i.thread) != thread && !related(i.thread))
            .filter(|i| now - i.at < recent())
            .collect();
        rows.sort_by_key(|i| std::cmp::Reverse(i.at));
        rows
    }

    /// Take a thread off the board (archived or deleted).
    pub fn forget(&mut self, thread: ThreadId) {
        self.intents.retain(|i| i.thread != thread);
    }

    fn prune(&mut self, now: DateTime<Utc>) {
        self.intents.retain(|i| now - i.at < recent());
    }
}

/// Who changed what, as a row for an agent or a person: `title` and `agent`
/// name the other thread; `viewer` is the working tree of whoever reads it.
pub fn describe(
    intent: &Intent,
    title: &str,
    agent: &str,
    viewer: Option<&Path>,
    now: DateTime<Utc>,
) -> String {
    let symbols = if intent.symbols.is_empty() {
        String::new()
    } else {
        format!(
            ": {}",
            intent
                .symbols
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!(
        "\u{201c}{title}\u{201d} ({agent}, {}) · {}{symbols} · {}",
        place(intent, viewer),
        intent.path,
        ago(now - intent.at)
    )
}

/// The notice for a thread whose change overlaps `overlap.other`'s.
pub fn overlap_notice(
    overlap: &Overlap,
    title: &str,
    agent: &str,
    viewer: &Path,
    now: DateTime<Utc>,
) -> String {
    let other = &overlap.other;
    let mut text = format!(
        "\u{201c}{title}\u{201d} ({agent}) also changed {} {}",
        other.path,
        ago(now - other.at)
    );
    if !overlap.shared.is_empty() {
        text.push_str(&format!(", in {}", overlap.shared.join(", ")));
    }
    text.push_str(if same_folder(other, viewer) {
        ". Both work in the same folder, on the same copy of the file."
    } else {
        ". It works in another worktree, so the changes meet when the branches are merged."
    });
    if !overlap.differing.is_empty() {
        text.push_str(&format!(
            " Both changed how {} is declared, differently.",
            overlap.differing.join(", ")
        ));
    }
    text
}

/// The block put in front of a prompt: what other agents changed lately.
pub fn context(rows: &[String]) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let mut text = String::from(
        "<other-agents>\nOther agents in Elyra Workspace changed these files in this repository during the last hour:\n",
    );
    for row in rows.iter().take(MAX_ROWS) {
        text.push_str(&format!("- {row}\n"));
    }
    if rows.len() > MAX_ROWS {
        text.push_str(&format!("- and {} more\n", rows.len() - MAX_ROWS));
    }
    text.push_str(
        "Stay out of that code unless the task needs it; if it does, say so in your reply.\n</other-agents>",
    );
    Some(text)
}

fn same_folder(intent: &Intent, viewer: &Path) -> bool {
    intent.root == viewer
}

fn place(intent: &Intent, viewer: Option<&Path>) -> String {
    if viewer.is_some_and(|viewer| same_folder(intent, viewer)) {
        "same folder".into()
    } else {
        match &intent.branch {
            Some(branch) => format!("branch {branch}"),
            None => "another worktree".into(),
        }
    }
}

fn ago(elapsed: Duration) -> String {
    match elapsed.num_minutes() {
        ..1 => "just now".into(),
        minutes => format!("{minutes} min ago"),
    }
}

/// The file an edit tool call changes and the text it put there, for the
/// tools agents use (Claude Code's, Codex's and ACP's as the providers map
/// them, and Elyra's own). None for anything else.
pub fn edited_file(name: &str, input: &serde_json::Value) -> Option<(String, Vec<String>)> {
    let path = ["file_path", "path", "notebook_path"]
        .iter()
        .find_map(|key| input[key].as_str())?
        .to_string();
    let new_text = |value: &serde_json::Value| {
        value["new_string"]
            .as_str()
            .or(value["newText"].as_str())
            .map(str::to_string)
    };
    let snippets = match name.to_ascii_lowercase().as_str() {
        "edit" | "multiedit" => new_text(input)
            .into_iter()
            .chain(
                input["edits"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(new_text),
            )
            .collect(),
        // The whole file: no symbol stands out.
        "write" | "notebookedit" => Vec::new(),
        _ => return None,
    };
    Some((path, snippets))
}

/// Threads meant to overlap with `thread`: its Best of N siblings, its side
/// chats and the thread it is a side chat of.
pub fn related_threads(app: &crate::app_state::AppState, thread: ThreadId) -> HashSet<ThreadId> {
    let Some(me) = app.thread(thread) else {
        return HashSet::new();
    };
    app.threads
        .iter()
        .filter(|t| {
            (me.race_id.is_some() && t.race_id == me.race_id)
                || t.parent_id == Some(thread)
                || me.parent_id == Some(t.id)
        })
        .map(|t| t.id)
        .collect()
}

/// A thread's title and agent, for rows and notices.
pub fn label(app: &crate::app_state::AppState, thread: ThreadId) -> (String, String) {
    app.thread(thread)
        .map(|t| (t.title.clone(), t.provider.label().to_string()))
        .unwrap_or_else(|| ("a thread".into(), "agent".into()))
}

/// What others changed lately in `repo`, as rows for `me` (working in `root`).
pub fn rows_for(
    app: &crate::app_state::AppState,
    repo: &Path,
    root: &Path,
    me: Option<ThreadId>,
    now: DateTime<Utc>,
) -> Vec<String> {
    let related = me.map(|me| related_threads(app, me)).unwrap_or_default();
    app.intents
        .others(repo, me, now, |t| related.contains(&t))
        .into_iter()
        .map(|intent| {
            let (title, agent) = label(app, intent.thread);
            describe(intent, &title, &agent, Some(root), now)
        })
        .collect()
}

/// The definitions in `source` that the text `snippets` sit in (the
/// innermost one around each) or that start inside them. At most four.
pub fn symbols_in(path: &str, source: &str, snippets: &[String]) -> Vec<Symbol> {
    let Some(language) = elyra_index::Language::from_path(Path::new(path)) else {
        return Vec::new();
    };
    let Ok(extraction) = elyra_index::extract(language, source) else {
        return Vec::new();
    };
    let mut found: Vec<Symbol> = Vec::new();
    for snippet in snippets.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let Some(offset) = source.find(snippet) else {
            continue;
        };
        let first = source[..offset].matches('\n').count() as u32 + 1;
        let last = first + snippet.lines().count().saturating_sub(1) as u32;
        let around = extraction
            .definitions
            .iter()
            .filter(|d| d.line <= first && d.end_line >= last)
            .min_by_key(|d| d.end_line - d.line);
        let inside = extraction
            .definitions
            .iter()
            .filter(|d| d.line >= first && d.line <= last);
        for definition in around.into_iter().chain(inside) {
            if found.len() < 4 && !found.iter().any(|s| s.name == definition.name) {
                found.push(Symbol {
                    name: definition.name.clone(),
                    signature: definition.signature.clone(),
                });
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::{
        Intent, IntentBoard, Symbol, context, describe, edited_file, overlap_notice, symbols_in,
    };
    use chrono::{Duration, Utc};
    use serde_json::json;
    use std::path::{Path, PathBuf};

    fn intent(thread: elyra_core::ThreadId, root: &str, symbols: &[(&str, &str)]) -> Intent {
        Intent {
            repo: PathBuf::from("/shop/.git"),
            root: PathBuf::from(root),
            branch: Some("main".into()),
            thread,
            path: "src/cart.rs".into(),
            symbols: symbols
                .iter()
                .map(|(name, signature)| Symbol {
                    name: name.to_string(),
                    signature: signature.to_string(),
                })
                .collect(),
            at: Utc::now(),
        }
    }

    #[test]
    fn tells_both_threads_once_when_they_change_the_same_file() {
        let (a, b, c) = (
            elyra_core::new_id(),
            elyra_core::new_id(),
            elyra_core::new_id(),
        );
        let mut board = IntentBoard::default();
        assert!(
            board
                .publish(
                    intent(a, "/shop", &[("total", "fn total(&self) -> u32")]),
                    |_| false
                )
                .is_empty()
        );
        let overlaps = board.publish(
            intent(b, "/shop", &[("total", "fn total(&self) -> u64")]),
            |_| false,
        );
        assert_eq!(overlaps.len(), 1);
        assert_eq!(overlaps[0].other.thread, a);
        assert_eq!(overlaps[0].shared, ["total"]);
        assert_eq!(overlaps[0].differing, ["total"], "declared differently");
        let notice = overlap_notice(
            &overlaps[0],
            "Fix totals",
            "Codex",
            Path::new("/shop"),
            Utc::now(),
        );
        assert_eq!(
            notice,
            "“Fix totals” (Codex) also changed src/cart.rs just now, in total. Both work in the same folder, on the same copy of the file. Both changed how total is declared, differently."
        );
        // Told once per pair and file.
        assert!(board.publish(intent(b, "/shop", &[]), |_| false).is_empty());
        assert!(board.publish(intent(a, "/shop", &[]), |_| false).is_empty());
        // A Best of N sibling is meant to overlap.
        assert!(
            board
                .publish(intent(c, "/wt/c", &[]), |t| t == a || t == b)
                .is_empty()
        );

        let now = Utc::now();
        let others = board.others(Path::new("/shop/.git"), Some(b), now, |_| false);
        assert_eq!(
            others.iter().map(|i| i.thread).collect::<Vec<_>>(),
            [c, a],
            "newest first, not itself"
        );
        assert_eq!(
            describe(
                others[1],
                "Fix totals",
                "Codex",
                Some(Path::new("/shop")),
                now
            ),
            "“Fix totals” (Codex, same folder) · src/cart.rs: total · just now"
        );
        assert_eq!(
            describe(
                others[0],
                "Try 2",
                "Claude Code",
                Some(Path::new("/shop")),
                now
            ),
            "“Try 2” (Claude Code, branch main) · src/cart.rs · just now"
        );
        let block = context(&["a row".into()]).unwrap();
        assert!(block.starts_with("<other-agents>\n") && block.contains("- a row\n"));
        assert!(context(&[]).is_none());

        // An hour later it's gone.
        let later = now + Duration::minutes(61);
        assert!(
            board
                .others(Path::new("/shop/.git"), None, later, |_| false)
                .is_empty()
        );
        board.forget(a);
        assert_eq!(
            board
                .others(Path::new("/shop/.git"), None, now, |_| false)
                .len(),
            2
        );
    }

    #[test]
    fn reads_edits_from_every_agent() {
        assert_eq!(
            edited_file("Edit", &json!({"file_path": "/a.rs", "new_string": "x"})),
            Some(("/a.rs".into(), vec!["x".into()]))
        );
        assert_eq!(
            edited_file(
                "edit",
                &json!({"path": "a.rs", "edits": [{"newText": "y"}]})
            ),
            Some(("a.rs".into(), vec!["y".into()]))
        );
        assert_eq!(
            edited_file("Write", &json!({"file_path": "/b.rs", "content": "z"})),
            Some(("/b.rs".into(), vec![]))
        );
        assert_eq!(edited_file("Read", &json!({"file_path": "/a.rs"})), None);
        assert_eq!(edited_file("Edit", &json!({})), None);
    }

    #[test]
    fn finds_the_symbols_an_edit_sits_in() {
        let source = "pub struct Cart { total: u32 }\n\nimpl Cart {\n    pub fn total(&self) -> u32 {\n        discount(self.total)\n    }\n}\n\nfn discount(amount: u32) -> u32 {\n    amount * 9 / 10\n}\n";
        let names = |snippets: &[&str]| {
            symbols_in(
                "src/cart.rs",
                source,
                &snippets.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            )
            .into_iter()
            .map(|s| s.name)
            .collect::<Vec<_>>()
        };
        assert_eq!(names(&["amount * 9 / 10"]), ["discount"]);
        assert_eq!(names(&["discount(self.total)"]), ["total"]);
        assert_eq!(
            names(&["fn discount(amount: u32) -> u32 {\n    amount * 9 / 10\n}"]),
            ["discount"]
        );
        assert!(names(&["not in the file"]).is_empty());
        assert!(symbols_in("notes.txt", "hello", &["hello".into()]).is_empty());
    }
}
