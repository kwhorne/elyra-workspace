//! The intent guard: when a turn removes or rewrites lines that the line
//! record says were written for another instruction, Elyra knows why those
//! lines were there. The thread shows which instructions they were for, the
//! agent hears of it with its next message, and the author of each can be
//! asked (in a side chat picked up where it wrote them) whether the new code
//! still does what it was asked to do.

use crate::app_state::AppState;
use elyra_core::{ItemId, ProjectId, RewrittenIntent, ThreadId};
use std::collections::BTreeMap;
use std::path::Path;

/// How many instructions and lines a card lists.
const MAX_INSTRUCTIONS: usize = 5;
const MAX_LINES: usize = 6;

/// A removed line: (path, text, hash).
pub type Removed = (String, String, i64);

/// The lines a turn removed from `before` to `after` that aren't in the file
/// any more (moved lines stay). Blocking.
pub fn removed(root: &Path, before: &str, after: &str) -> Vec<Removed> {
    let lines = elyra_git::checkpoint::removed_between(root, before, after).unwrap_or_default();
    let mut files: BTreeMap<String, Option<String>> = BTreeMap::new();
    lines
        .into_iter()
        .filter_map(|(path, text)| {
            let hash = elyra_core::line_hash(&text)?;
            let now = files
                .entry(path.clone())
                .or_insert_with(|| std::fs::read_to_string(root.join(&path)).ok());
            let moved = now
                .as_deref()
                .is_some_and(|now| now.lines().any(|l| l.trim() == text.trim()));
            (!moved).then_some((path, text, hash))
        })
        .collect()
}

/// Which other instructions the removed lines were written for, most lines
/// first. `turn` is the instruction whose turn removed them.
pub fn find(
    app: &AppState,
    project: ProjectId,
    turn: (ThreadId, ItemId),
    removed: &[Removed],
) -> Vec<RewrittenIntent> {
    let mut groups: BTreeMap<(ThreadId, ItemId), RewrittenIntent> = BTreeMap::new();
    for (path, text, hash) in removed {
        let Some(origin) = app
            .store
            .line_origins(project, path, *hash)
            .unwrap_or_default()
            .into_iter()
            .find(|o| (o.thread, o.item) != turn)
        else {
            continue;
        };
        let group = groups
            .entry((origin.thread, origin.item))
            .or_insert_with(|| {
                let written = crate::provenance::turn(app, &origin);
                let instruction: String = written
                    .prompt
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("")
                    .chars()
                    .take(200)
                    .collect();
                RewrittenIntent {
                    thread: origin.thread,
                    item: origin.item,
                    instruction,
                    title: written.title,
                    agent: written.agent,
                    files: Vec::new(),
                    lines: Vec::new(),
                    count: 0,
                    asked: None,
                }
            });
        if !group.files.contains(path) {
            group.files.push(path.clone());
        }
        if group.lines.len() < MAX_LINES {
            group.lines.push(text.trim_end().to_string());
        }
        group.count += 1;
    }
    let mut found: Vec<RewrittenIntent> = groups.into_values().collect();
    found.sort_by_key(|g| std::cmp::Reverse(g.count));
    found.truncate(MAX_INSTRUCTIONS);
    found
}

fn lines(count: usize) -> String {
    format!("{count} line{}", if count == 1 { "" } else { "s" })
}

/// For the agent's next message.
pub fn note(intents: &[RewrittenIntent], same_thread: ThreadId) -> String {
    let mut out = String::from(
        "<rewritten-intent>\nYour last turn removed or rewrote lines that were written for other instructions:\n",
    );
    for intent in intents {
        let place = if intent.thread == same_thread {
            "earlier in this thread".to_string()
        } else {
            format!("in the thread “{}”, by {}", intent.title, intent.agent)
        };
        out.push_str(&format!(
            "- “{}” ({place}): {} in {}, e.g. `{}`\n",
            intent.instruction,
            lines(intent.count),
            intent.files.join(", "),
            intent.lines.first().map(|l| l.trim()).unwrap_or("")
        ));
    }
    out.push_str("Check that the new code still does what those instructions asked for. If it doesn't, restore that behaviour, or tell the user plainly what is gone and why.\n</rewritten-intent>");
    out
}

/// What the author of the lines is asked.
pub fn question(intent: &RewrittenIntent) -> String {
    format!(
        "A later turn removed or rewrote {} you wrote for this instruction ({}). Given what you knew and were asked when you wrote them: does the new code still do what you were asked to do? Name exactly what is lost or now wrong, with the case that shows it, or say plainly that it still holds.",
        lines(intent.count),
        intent.files.join(", ")
    )
}

/// The card's summary of one instruction.
pub fn summary(intent: &RewrittenIntent) -> String {
    format!(
        "{} in {} · “{}” · {}",
        lines(intent.count),
        intent.files.join(", "),
        intent.title,
        intent.agent
    )
}

#[cfg(test)]
mod tests {
    use super::{note, question, removed};
    use elyra_core::RewrittenIntent;

    fn intent(thread: elyra_core::ThreadId) -> RewrittenIntent {
        RewrittenIntent {
            thread,
            item: elyra_core::new_id(),
            instruction: "Fix VAT rounding for foreign customers".into(),
            title: "VAT".into(),
            agent: "Claude Code".into(),
            files: vec!["app/Tax.php".into()],
            lines: vec!["    if ($customer->isForeign()) {".into()],
            count: 3,
            asked: None,
        }
    }

    #[test]
    fn tells_the_agent_and_asks_the_author() {
        let (other, here) = (elyra_core::new_id(), elyra_core::new_id());
        let text = note(&[intent(other), intent(here)], here);
        assert!(text.starts_with("<rewritten-intent>\n"));
        assert!(text.contains(
            "- “Fix VAT rounding for foreign customers” (in the thread “VAT”, by Claude Code): 3 lines in app/Tax.php, e.g. `if ($customer->isForeign()) {`"
        ));
        assert!(text.contains("(earlier in this thread)"));
        assert!(text.ends_with("</rewritten-intent>"));
        let asked = question(&intent(other));
        assert!(asked.starts_with("A later turn removed or rewrote 3 lines you wrote"));
    }

    #[test]
    fn moved_lines_are_not_removed() {
        let repo = std::env::temp_dir().join(format!("elyra-intent-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        std::fs::write(
            repo.join("tax.php"),
            "function vat($c) {\n    if ($c->isForeign()) { return 0; }\n    return round($c->total * 0.25);\n}\n",
        )
        .unwrap();
        let before = elyra_git::checkpoint::create(&repo, "refs/elyra/t/before", "b").unwrap();
        std::fs::write(
            repo.join("tax.php"),
            "function vat($c) {\n    return round($c->total * 0.25);\n}\n",
        )
        .unwrap();
        let after = elyra_git::checkpoint::create(&repo, "refs/elyra/t/after", "a").unwrap();
        let gone = removed(&repo, &before, &after);
        assert_eq!(gone.len(), 1, "{gone:?}");
        assert_eq!(gone[0].1.trim(), "if ($c->isForeign()) { return 0; }");
        std::fs::remove_dir_all(&repo).unwrap();
    }
}
