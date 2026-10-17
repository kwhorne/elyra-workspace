//! Taking back one instruction: what its turn (and the automatic fixes that
//! followed) changed in the folder is undone, and what came after it stays.
//! Files changed since in ways that don't merge are left for the agent.

use chrono::{DateTime, Utc};
use elyra_core::{ItemContent, ItemId, TranscriptItem};

/// Where an instruction's changes start and end.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub instruction: String,
    pub at: DateTime<Utc>,
    /// The folder before it (the message's checkpoint).
    pub before: String,
    /// The folder as its last turn left it, when that was kept.
    pub after: Option<String>,
}

/// The instruction `item` in a transcript: from its checkpoint to the end
/// of its last turn, automatic fixes included.
pub fn span(items: &[TranscriptItem], item: ItemId) -> Option<Span> {
    let start = items.iter().position(|i| i.id == item)?;
    let ItemContent::User {
        text,
        checkpoint: Some(before),
        ..
    } = &items[start].content
    else {
        return None;
    };
    let mut after = None;
    for later in &items[start + 1..] {
        match &later.content {
            ItemContent::User { text, .. } if crate::checks::fix_prompt_label(text).is_none() => {
                break;
            }
            ItemContent::TurnSummary {
                after: Some(snapshot),
                ..
            } => after = Some(snapshot.clone()),
            _ => {}
        }
    }
    Some(Span {
        instruction: text.trim().to_string(),
        at: items[start].created_at,
        before: before.clone(),
        after,
    })
}

/// What undoing did, done in the background: (files put back, files left,
/// snapshots before and after the undo).
pub struct Done {
    pub reverted: Vec<String>,
    pub kept: Vec<String>,
    pub before_undo: String,
    pub after_undo: String,
}

/// Undo `span` in the repository at `root`. `next` is the state before the
/// folder's next instruction, for turns from before their end was kept;
/// without either, the folder as it is now. `refs` names the two snapshots
/// kept for putting it back. Blocking.
pub fn undo(
    root: &std::path::Path,
    span: &Span,
    next: Option<String>,
    refs: (&str, &str),
) -> anyhow::Result<Done> {
    let after = match span.after.clone().or(next) {
        Some(after) => after,
        None => elyra_git::checkpoint::snapshot(root, "elyra: before undo")?,
    };
    let paths = elyra_git::undo::changed_paths(root, &span.before, &after)?;
    anyhow::ensure!(
        !paths.is_empty(),
        "that instruction didn't change any files"
    );
    let before_undo = elyra_git::checkpoint::create(root, refs.0, "elyra: before undo")?;
    let applied = elyra_git::undo::apply(root, &after, &span.before, &paths)?;
    let after_undo = elyra_git::checkpoint::create(root, refs.1, "elyra: after undo")?;
    Ok(Done {
        reverted: applied.changed,
        kept: applied.kept,
        before_undo,
        after_undo,
    })
}

/// Put an undo back: what it changed returns, what changed since stays.
pub fn put_back(
    root: &std::path::Path,
    before_undo: &str,
    after_undo: &str,
) -> anyhow::Result<elyra_git::undo::Applied> {
    let paths = elyra_git::undo::changed_paths(root, after_undo, before_undo)?;
    elyra_git::undo::apply(root, after_undo, before_undo, &paths)
}

/// The message for the agent about the files that couldn't be undone.
pub fn finish_prompt(instruction: &str, kept: &[String], reverted: &[String]) -> String {
    let headline = instruction.lines().next().unwrap_or("").trim();
    let mut text = format!(
        "Undo what was done for this earlier message, keeping everything that came after it:\n“{headline}”\n\n"
    );
    if !reverted.is_empty() {
        text.push_str(&format!(
            "These files are already back as they were: {}.\n",
            reverted
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    text.push_str(&format!(
        "These changed again later, so take that message's part out of them by hand: {}. Run the checks afterwards.",
        kept.iter()
            .map(|p| format!("`{p}`"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::{finish_prompt, span};
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

    fn summary(after: Option<&str>) -> TranscriptItem {
        item(ItemContent::TurnSummary {
            duration_ms: None,
            cost_usd: None,
            is_error: false,
            context_tokens: None,
            context_window: None,
            after: after.map(str::to_string),
        })
    }

    #[test]
    fn an_instruction_runs_to_the_end_of_its_last_fix() {
        let fix = format!(
            "{}`cargo test` exited with 1 (automatic fix 1 of 2).",
            crate::checks::FIX_PROMPT_START
        );
        let message = |text: &str, checkpoint: &str| {
            item(ItemContent::User {
                text: text.into(),
                checkpoint: Some(checkpoint.into()),
                attachments: Vec::new(),
            })
        };
        let items = vec![
            message("Round the totals", "c1"),
            summary(Some("a1")),
            message(&fix, "c2"),
            summary(Some("a2")),
            message("Rename total()", "c3"),
            summary(None),
        ];
        let first = span(&items, items[0].id).unwrap();
        assert_eq!(
            (
                first.instruction.as_str(),
                first.before.as_str(),
                first.after.as_deref()
            ),
            ("Round the totals", "c1", Some("a2")),
            "through the automatic fix"
        );
        let second = span(&items, items[4].id).unwrap();
        assert_eq!(second.after, None, "a turn from before its end was kept");
        assert!(span(&items, items[1].id).is_none(), "not a message");

        let prompt = finish_prompt("Round the totals", &["lib.rs".into()], &["new.rs".into()]);
        assert!(prompt.contains("“Round the totals”"));
        assert!(prompt.contains("already back as they were: `new.rs`."));
        assert!(prompt.contains("take that message's part out of them by hand: `lib.rs`."));
    }
}
