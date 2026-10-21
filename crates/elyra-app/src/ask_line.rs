//! Asking a line why: a read-only chat with the agent that wrote it, picked
//! up where that turn ended. It gets the conversation as it was then (not
//! what came after), what the turn changed in the file and the code around
//! the line now, so it answers from what it knew and decided at the time.
//! From the Changes tab's line tools, and agents ask through the gateway's
//! `ask_author`.

use crate::app_state::AppState;
use crate::thread_session::ThreadSession;
use elyra_core::{ItemContent, ItemId, ThreadId, TranscriptItem};
use gpui_kit::*;
use std::path::{Path, PathBuf};

/// What is asked about which line.
#[derive(Clone, Debug, Default)]
pub struct Request {
    /// The thread the chat belongs to (a side chat of it); None: on its own.
    pub parent: Option<ThreadId>,
    /// The repository the line is in, and its path there.
    pub root: PathBuf,
    pub path: String,
    /// 1-based, in the file as it is now; None for a removed line.
    pub line: Option<u32>,
    pub text: String,
    pub question: String,
    /// The message that wrote it, when known (otherwise the line record's
    /// newest for the line).
    pub author: Option<(ThreadId, ItemId)>,
    /// How a later turn changed it, as a diff.
    pub since: Option<String>,
    /// The chat's title (default: Why · <location>).
    pub title: Option<String>,
}

pub const DEFAULT_QUESTION: &str = "Why is this line the way it is?";

/// The conversation up to the end of the turns of the message `item`
/// (automatic fixes included), leaving out what came after.
pub fn until_turn_end(items: &[TranscriptItem], item: ItemId) -> &[TranscriptItem] {
    let Some(start) = items.iter().position(|i| i.id == item) else {
        return items;
    };
    let end = items[start + 1..]
        .iter()
        .position(|i| match &i.content {
            ItemContent::User { text, .. } => crate::checks::fix_prompt_label(text).is_none(),
            _ => false,
        })
        .map_or(items.len(), |at| start + 1 + at);
    &items[..end]
}

/// The lines around `line` (1-based), numbered.
fn around(content: &str, line: u32, reach: usize) -> String {
    let at = line as usize - 1;
    content
        .lines()
        .enumerate()
        .skip(at.saturating_sub(reach))
        .take(reach * 2 + 1)
        .map(|(index, text)| {
            format!(
                "{}{:>5}  {text}",
                if index == at { ">" } else { " " },
                index + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the turn changed in the file, and the code around the line now.
/// Blocking.
fn gather(
    root: &Path,
    path: &str,
    line: Option<u32>,
    turn: Option<(String, String)>,
) -> (Option<String>, Option<String>) {
    let diff = turn
        .and_then(|(before, after)| elyra_git::diff_text_of(root, &before, &after, path).ok())
        .filter(|diff| !diff.trim().is_empty())
        .map(|diff| {
            if diff.chars().count() > 8_000 {
                format!("{}\n… (cut)", diff.chars().take(8_000).collect::<String>())
            } else {
                diff
            }
        });
    let now = line.and_then(|line| {
        let content = std::fs::read_to_string(root.join(path)).ok()?;
        (content.lines().count() >= line as usize).then(|| around(&content, line, 8))
    });
    (diff, now)
}

/// The first message of the chat.
pub fn prompt(
    location: &str,
    text: &str,
    diff: Option<&str>,
    now: Option<&str>,
    since: Option<&str>,
    question: &str,
) -> String {
    let mut out = format!(
        "You wrote the line below in the conversation above, which ends where that turn ended. The code may have changed since. Someone is asking you about it.\n\n`{location}`: `{}`\n",
        text.trim()
    );
    if let Some(diff) = diff {
        out.push_str(&format!(
            "\nWhat that turn changed in the file:\n```diff\n{}\n```\n",
            diff.trim_end()
        ));
    }
    match now {
        Some(now) => out.push_str(&format!(
            "\nThe code around it now (the line is marked >):\n```\n{now}\n```\n"
        )),
        None => out.push_str("\nThe line is no longer in the file.\n"),
    }
    if let Some(since) = since {
        out.push_str(&format!(
            "\nSince then, a later turn changed it like this:\n```diff\n{}\n```\n",
            since.trim_end()
        ));
    }
    out.push_str(&format!(
        "\nAnswer from what you knew and decided then: the reason, what you weighed it against, and what it depends on. If you would do it differently now that you see the code as it is, say so and why. This is a question: don't change any files.\n\nQuestion: {}",
        question.trim()
    ));
    out
}

/// Start the chat and send the question; returns its session.
pub async fn ask(
    app: &Entity<AppState>,
    request: Request,
    cx: &mut AsyncApp,
) -> anyhow::Result<Entity<ThreadSession>> {
    let found = cx.update(|cx| -> anyhow::Result<_> {
        let state = app.read(cx);
        let (thread, item) = match request.author {
            Some(author) => author,
            None => {
                let project = crate::provenance::project_of(state, &request.root)
                    .ok_or_else(|| anyhow::anyhow!("that folder isn't in a project"))?;
                let hash = elyra_core::line_hash(&request.text)
                    .ok_or_else(|| anyhow::anyhow!("that line is blank"))?;
                let origin = state
                    .store
                    .line_origins(project, &request.path, hash)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "no agent in Elyra Workspace is recorded as writing that line"
                        )
                    })?;
                (origin.thread, origin.item)
            }
        };
        let items = state.store.transcript(thread)?;
        anyhow::ensure!(!items.is_empty(), "the conversation that wrote it is gone");
        let then = until_turn_end(&items, item);
        // Where the turn ended; for turns from before that was kept, where
        // the next message began.
        let turn = crate::undo::span(&items, item).and_then(|span| {
            let end = span.after.or_else(|| {
                items.get(then.len()).and_then(|next| match &next.content {
                    ItemContent::User {
                        checkpoint: Some(checkpoint),
                        ..
                    } => Some(checkpoint.clone()),
                    _ => None,
                })
            })?;
            Some((span.before, end))
        });
        Ok((
            thread,
            crate::app_state::transcript_text(then, 40_000),
            turn,
        ))
    })?;
    let (author, context, turn) = found;
    let (root, path, line) = (request.root.clone(), request.path.clone(), request.line);
    let (diff, now) = cx
        .background_executor()
        .spawn(async move { gather(&root, &path, line, turn) })
        .await;
    let location = match request.line {
        Some(line) => format!("{}:{line}", request.path),
        None => request.path.clone(),
    };
    let first = prompt(
        &location,
        &request.text,
        diff.as_deref(),
        now.as_deref(),
        request.since.as_deref(),
        &request.question,
    );
    let title = request
        .title
        .clone()
        .unwrap_or_else(|| format!("Why · {location}"));
    cx.update(|cx| {
        let session = app.update(cx, |app, cx| {
            app.create_line_chat(request.parent, author, title, context, cx)
        })?;
        session.update(cx, |session, cx| {
            session.submit(elyra_provider::Prompt::text(first), cx)
        });
        Ok(session)
    })
}

#[cfg(test)]
mod tests {
    use super::{around, prompt, until_turn_end};
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
            attachments: Vec::new(),
        })
    }

    fn reply(text: &str) -> TranscriptItem {
        item(ItemContent::Assistant {
            text: text.into(),
            parent_tool_use_id: None,
        })
    }

    #[test]
    fn keeps_the_conversation_until_that_turn_ended() {
        let items = vec![
            user("Add the discount"),
            reply("Added."),
            user("Round the totals"),
            reply("Rounded."),
            user("Ship it"),
        ];
        let then = until_turn_end(&items, items[2].id);
        assert_eq!(then.len(), 4, "up to the answer to “Round the totals”");
        assert_eq!(until_turn_end(&items, items[4].id).len(), 5);
    }

    #[test]
    fn asks_with_the_line_then_and_now() {
        assert_eq!(
            around("a\nb\nc\nd\ne", 3, 1),
            "     2  b\n>    3  c\n     4  d"
        );
        let text = prompt(
            "lib.rs:3",
            "  let total = round(sum - discount);",
            Some("@@ -1 +1 @@\n-a\n+b\n"),
            Some(">    3  let total"),
            None,
            "Why round after the discount?",
        );
        assert!(text.contains("`lib.rs:3`: `let total = round(sum - discount);`"));
        assert!(text.contains("```diff\n@@ -1 +1 @@\n-a\n+b\n```"));
        assert!(text.contains("(the line is marked >)"));
        assert!(text.ends_with("Question: Why round after the discount?"));
        let gone = prompt("lib.rs", "x", None, None, Some("-x\n+y"), "Why?");
        assert!(gone.contains("The line is no longer in the file."));
        assert!(
            gone.contains("Since then, a later turn changed it like this:\n```diff\n-x\n+y\n```")
        );
    }
}
