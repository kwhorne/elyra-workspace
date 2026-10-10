//! Where code came from: each turn records the lines it added (see
//! `ThreadSession::record_line_origins`), so a line can be traced back to the
//! thread and the message that produced it — for every agent, also when it
//! committed itself. Agents ask through the gateway's `why` and `history`
//! tools; the blame in the Changes tab shows it too.

use crate::app_state::AppState;
use elyra_core::{ItemContent, LineOrigin, ProjectId, ThreadId};
use std::path::{Path, PathBuf};

/// A path relative to the repository a folder is in (agents name paths
/// relative to their folder, which may be below the repository's root).
pub fn repo_relative(dir: &Path, path: &str) -> Option<(PathBuf, String)> {
    let root = elyra_git::repo_root(dir).ok()?;
    let absolute = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        dir.join(path.trim_start_matches("./"))
    };
    let canonical = absolute.canonicalize().unwrap_or(absolute);
    let root = root.canonicalize().unwrap_or(root);
    let relative = canonical
        .strip_prefix(&root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    Some((root, relative))
}

/// The turn behind an origin, in words: thread, project, agent, when, the
/// message and the start of the agent's answer.
pub struct Turn {
    pub thread: ThreadId,
    pub title: String,
    pub project: String,
    pub agent: String,
    pub at: chrono::DateTime<chrono::Utc>,
    pub prompt: String,
    pub reply: String,
}

pub fn turn(app: &AppState, origin: &LineOrigin) -> Turn {
    let thread = app.thread(origin.thread).cloned().or_else(|| {
        app.archived_threads()
            .into_iter()
            .find(|t| t.id == origin.thread)
    });
    let items = app.store.transcript(origin.thread).unwrap_or_default();
    let start = items.iter().position(|i| i.id == origin.item);
    let prompt = start
        .and_then(|at| match &items[at].content {
            ItemContent::User { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let reply = start
        .map(|at| {
            items[at + 1..]
                .iter()
                .take_while(|i| !matches!(i.content, ItemContent::User { .. }))
                .filter_map(|i| match &i.content {
                    ItemContent::Assistant {
                        text,
                        parent_tool_use_id: None,
                    } if !text.trim().is_empty() => Some(text.clone()),
                    _ => None,
                })
                .last()
                .unwrap_or_default()
        })
        .unwrap_or_default();
    Turn {
        thread: origin.thread,
        title: thread
            .as_ref()
            .map(|t| t.title.clone())
            .unwrap_or_else(|| "a deleted thread".into()),
        project: thread
            .as_ref()
            .and_then(|t| app.project(t.project_id))
            .map(|p| p.name.clone())
            .unwrap_or_default(),
        agent: thread
            .map(|t| t.provider.label().to_string())
            .unwrap_or_default(),
        at: origin.at,
        prompt,
        reply,
    }
}

fn clip(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max - 1).collect::<String>())
    } else {
        text
    }
}

fn when(at: chrono::DateTime<chrono::Utc>) -> String {
    at.with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// One line about a turn, for blame: “Fix totals” (shop) · Claude Code · 2026-10-09 14:02.
pub fn short(turn: &Turn) -> String {
    format!(
        "\u{201c}{}\u{201d} · {} · {}",
        turn.title,
        turn.agent,
        when(turn.at)
    )
}

/// What `why` answers once the line, its origins and git's blame are known.
pub fn why_text(location: &str, line: &str, turns: &[Turn], blame: Option<String>) -> String {
    let mut out = format!("{location}: `{}`\n\n", clip(line, 200));
    match turns.first() {
        Some(turn) => {
            out.push_str(&format!(
                "Written in the thread \u{201c}{}\u{201d}{} by {} on {}, for the message:\n  \u{201c}{}\u{201d}\n",
                turn.title,
                if turn.project.is_empty() { String::new() } else { format!(" ({})", turn.project) },
                turn.agent,
                when(turn.at),
                clip(&turn.prompt, 600)
            ));
            if !turn.reply.is_empty() {
                out.push_str(&format!("The agent said afterwards:\n  \u{201c}{}\u{201d}\n", clip(&turn.reply, 500)));
            }
            if turns.len() > 1 {
                out.push_str(&format!(
                    "The same line was also written in {} earlier turn{} (newest first): {}\n",
                    turns.len() - 1,
                    if turns.len() == 2 { "" } else { "s" },
                    turns[1..].iter().map(short).collect::<Vec<_>>().join("; ")
                ));
            }
        }
        None => out.push_str(
            "No agent in Elyra Workspace is recorded as writing this line (it predates the record, was written by hand, or changed since).\n",
        ),
    }
    out.push_str(&format!(
        "Git: {}\n",
        blame.unwrap_or_else(|| "not committed yet".into())
    ));
    out
}

/// What `history` answers: the turns that wrote lines of the file or symbol,
/// and git's log.
pub fn history_text(subject: &str, turns: &[(Turn, usize)], log: &[String]) -> String {
    let mut out = format!("{subject}\n\n");
    if turns.is_empty() {
        out.push_str("No turn in Elyra Workspace is recorded as writing here.\n");
    } else {
        out.push_str("Turns that wrote lines here (newest first):\n");
        for (turn, lines) in turns {
            out.push_str(&format!(
                "- {} · {} line{} · \u{201c}{}\u{201d}\n",
                short(turn),
                lines,
                if *lines == 1 { "" } else { "s" },
                clip(&turn.prompt, 200)
            ));
        }
    }
    if !log.is_empty() {
        out.push_str("\nGit log:\n");
        for line in log {
            out.push_str(&format!("- {line}\n"));
        }
    }
    out
}

/// The project a folder belongs to.
pub fn project_of(app: &AppState, dir: &Path) -> Option<ProjectId> {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    app.projects
        .iter()
        .find(|p| dir.starts_with(p.path.canonicalize().unwrap_or_else(|_| p.path.clone())))
        .map(|p| p.id)
        .or_else(|| {
            // A thread's worktree lives outside the project folder.
            app.threads.iter().find_map(|t| match &t.environment {
                elyra_core::Environment::Worktree { path, .. } if dir.starts_with(path) => {
                    Some(t.project_id)
                }
                _ => None,
            })
        })
}

#[cfg(test)]
mod tests {
    use super::{Turn, history_text, why_text};

    fn turn(title: &str) -> Turn {
        Turn {
            thread: elyra_core::new_id(),
            title: title.into(),
            project: "shop".into(),
            agent: "Claude Code".into(),
            at: chrono::Utc::now(),
            prompt: "Why does /api/orders return 500?\nFix it.".into(),
            reply: "Fixed: a null check on the address.".into(),
        }
    }

    #[test]
    fn says_where_a_line_came_from() {
        let text = why_text(
            "app/Order.php:42",
            "  return $a?->street;",
            &[turn("Fix 500")],
            Some("abc1234 · Ada · fix".into()),
        );
        assert!(text.starts_with("app/Order.php:42: `return $a?->street;`"));
        assert!(text.contains("Written in the thread “Fix 500” (shop) by Claude Code on "));
        assert!(
            text.contains("“Why does /api/orders return 500? Fix it.”"),
            "{text}"
        );
        assert!(
            text.contains("The agent said afterwards:\n  “Fixed: a null check on the address.”")
        );
        assert!(text.ends_with("Git: abc1234 · Ada · fix\n"));
        let unknown = why_text("a.rs:1", "fn x()", &[], None);
        assert!(unknown.contains("No agent in Elyra Workspace is recorded"));
        let history = history_text(
            "app/Order.php",
            &[(turn("Fix 500"), 3)],
            &["abc1234 2026-10-09 Ada: fix".into()],
        );
        assert!(history.contains("· 3 lines · “Why does /api/orders return 500? Fix it.”"));
        assert!(history.ends_with("- abc1234 2026-10-09 Ada: fix\n"));
    }
}
