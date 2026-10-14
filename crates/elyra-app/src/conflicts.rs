//! Merge conflicts that know why: for each side of a conflict, the
//! instruction that had an agent write those lines (from the line record),
//! so the agent resolving it keeps what both sides were asked to do instead
//! of picking one block of text.

use elyra_core::{ItemContent, ProjectId, Store, ThreadId};
use std::collections::HashMap;
use std::path::Path;

/// One side of a conflict: the branch git names and its lines.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Side {
    pub label: String,
    pub lines: Vec<String>,
}

/// A conflict in a file, from its markers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hunk {
    /// 1-based line of the `<<<<<<<` marker.
    pub line: usize,
    pub ours: Side,
    pub theirs: Side,
}

/// The conflicts in a file's text (`<<<<<<<`, an optional `|||||||` base,
/// `=======`, `>>>>>>>`).
pub fn parse(text: &str) -> Vec<Hunk> {
    enum At {
        Outside,
        Ours,
        Base,
        Theirs,
    }
    let mut hunks = Vec::new();
    let mut at = At::Outside;
    let mut hunk = Hunk::default();
    for (index, line) in text.lines().enumerate() {
        match at {
            At::Outside => {
                if let Some(label) = line.strip_prefix("<<<<<<<") {
                    hunk = Hunk {
                        line: index + 1,
                        ours: Side {
                            label: label.trim().to_string(),
                            lines: Vec::new(),
                        },
                        ..Default::default()
                    };
                    at = At::Ours;
                }
            }
            At::Ours if line.starts_with("|||||||") => at = At::Base,
            At::Ours | At::Base if line.starts_with("=======") => at = At::Theirs,
            At::Ours => hunk.ours.lines.push(line.to_string()),
            At::Base => {}
            At::Theirs => {
                if let Some(label) = line.strip_prefix(">>>>>>>") {
                    hunk.theirs.label = label.trim().to_string();
                    hunks.push(std::mem::take(&mut hunk));
                    at = At::Outside;
                } else {
                    hunk.theirs.lines.push(line.to_string());
                }
            }
        }
    }
    hunks
}

/// Who wrote a side's lines: per instruction (message, thread title, agent),
/// with how many of its lines, most first; lines no agent is recorded as
/// writing are counted apart.
fn authors(
    store: &Store,
    project: ProjectId,
    path: &str,
    lines: &[String],
) -> anyhow::Result<(Vec<(String, usize)>, usize)> {
    let mut counts: HashMap<(ThreadId, elyra_core::ItemId), usize> = HashMap::new();
    let mut unknown = 0;
    for line in lines {
        let Some(hash) = elyra_core::line_hash(line) else {
            continue;
        };
        match store.line_origins(project, path, hash)?.first() {
            Some(origin) => *counts.entry((origin.thread, origin.item)).or_default() += 1,
            None => unknown += 1,
        }
    }
    let mut named = Vec::new();
    for ((thread, item), count) in counts {
        let info = store.thread(thread)?;
        let message = store
            .transcript(thread)?
            .into_iter()
            .find(|i| i.id == item)
            .and_then(|i| match i.content {
                ItemContent::User { text, .. } => Some(text),
                _ => None,
            })
            .unwrap_or_default();
        let headline = message
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        let who = match info {
            Some(thread) => format!(
                "“{headline}” (thread “{}”, {})",
                thread.title,
                thread.provider.label()
            ),
            None => format!("“{headline}”"),
        };
        named.push((who, count));
    }
    named.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    Ok((named, unknown))
}

/// A message for the agent about the conflicts in the repository at
/// `root`: each conflict with its two sides and the instructions behind
/// them. None when there are no conflicts. Blocking.
pub fn explain(root: &Path, project: ProjectId, db: &Path) -> anyhow::Result<Option<String>> {
    let files = elyra_git::conflicted_files(root)?;
    if files.is_empty() {
        return Ok(None);
    }
    let store = Store::open_read_only(db)?;
    let mut text = String::from(
        "Resolve the merge conflicts below so that what both sides were asked to do still holds. Each side says which instruction had an agent write it. Where the two really contradict, keep the one that fits the code as it is now and say so in your reply. Remove the conflict markers, then run the checks.\n",
    );
    for path in &files {
        let Ok(content) = std::fs::read_to_string(root.join(path)) else {
            text.push_str(&format!(
                "\n### `{path}`\n\nNot text, or unreadable: resolve it by hand.\n"
            ));
            continue;
        };
        let hunks = parse(&content);
        if hunks.is_empty() {
            text.push_str(&format!(
                "\n### `{path}`\n\nConflicted, without markers (deleted on one side, or binary).\n"
            ));
            continue;
        }
        for hunk in hunks {
            text.push_str(&format!("\n### `{path}`, line {}\n", hunk.line));
            for (name, side) in [("Ours", &hunk.ours), ("Theirs", &hunk.theirs)] {
                let label = if side.label.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", side.label)
                };
                text.push_str(&format!(
                    "\n{name}{label}:\n```\n{}\n```\n",
                    side.lines.join("\n")
                ));
                let (named, unknown) = authors(&store, project, path, &side.lines)?;
                if named.is_empty() {
                    text.push_str("Not written by an agent in Elyra Workspace (by hand, or before the record).\n");
                } else {
                    for (who, lines) in &named {
                        text.push_str(&format!(
                            "Written for {who} — {lines} line{}.\n",
                            if *lines == 1 { "" } else { "s" }
                        ));
                    }
                    if unknown > 0 {
                        text.push_str(&format!(
                            "{unknown} more not by an agent in Elyra Workspace.\n"
                        ));
                    }
                }
            }
        }
    }
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn reads_both_sides_of_each_conflict() {
        let text = "fn a() {}\n<<<<<<< HEAD\nlet total = round(sum - discount);\n=======\nlet total = round(sum) - discount;\nlet note = 1;\n>>>>>>> member-discount\nfn b() {}\n<<<<<<< ours\nx\n||||||| base\nw\n=======\ny\n>>>>>>> theirs\n";
        let hunks = parse(text);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].line, 2);
        assert_eq!(
            (hunks[0].ours.label.as_str(), hunks[0].ours.lines.clone()),
            (
                "HEAD",
                vec!["let total = round(sum - discount);".to_string()]
            )
        );
        assert_eq!(hunks[0].theirs.label, "member-discount");
        assert_eq!(hunks[0].theirs.lines.len(), 2);
        assert_eq!(
            (hunks[1].ours.lines.clone(), hunks[1].theirs.lines.clone()),
            (vec!["x".to_string()], vec!["y".to_string()]),
            "the base section of a diff3 conflict is left out"
        );
        assert!(parse("no conflicts here\n").is_empty());
    }
}
