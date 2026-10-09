//! Following up a review. When a thread reviewed a pull request (the agent
//! posted a review or comments with `gh`), Elyra follows that PR: every few
//! minutes it asks GitHub whether the author pushed commits since the review,
//! or asked for a review again. Then a card in the thread says so, with
//! "Review the changes": the agent gets only what changed since the reviewed
//! commit, with the earlier comments and the replies to them, and checks
//! point by point what was addressed. A PR that is merged or closed is no
//! longer followed.

use crate::app_state::AppState;
use elyra_core::{ItemContent, ThreadId, TranscriptItem};
use gpui_kit::{App, AsyncApp, Entity, Global};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// How often followed pull requests are looked at (`ELYRA_REVIEW_POLL_SECS`
/// shortens it for the scenarios).
fn poll_every() -> Duration {
    let seconds = std::env::var("ELYRA_REVIEW_POLL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5 * 60);
    Duration::from_secs(seconds)
}
const KEY: &str = "pr_follows";

/// A pull request a thread reviewed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Follow {
    pub thread: ThreadId,
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    /// The commit the review was of.
    pub reviewed: String,
    /// The head the thread was last told about.
    pub notified: String,
    /// It was told that a review is requested again.
    #[serde(default)]
    pub told_requested: bool,
}

struct Following {
    /// Who `gh` is signed in as (looked up once).
    me: Option<String>,
}

impl Global for Following {}

fn load(app: &AppState) -> Vec<Follow> {
    app.store
        .setting(KEY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn save(app: &AppState, follows: &[Follow]) {
    if let Ok(json) = serde_json::to_string(follows)
        && let Err(err) = app.store.set_setting(KEY, &json)
    {
        log::warn!("saving followed pull requests: {err:#}");
    }
}

/// Pull requests (`owner/name` lowercased, number) that changed since a
/// thread reviewed them.
pub fn changed_since_review(app: &AppState) -> std::collections::HashSet<(String, u64)> {
    load(app)
        .into_iter()
        .filter(|f| f.notified != f.reviewed)
        .map(|f| (f.repo.to_lowercase(), f.number))
        .collect()
}

pub fn stop(app: &AppState, thread: ThreadId) {
    let follows: Vec<Follow> = load(app)
        .into_iter()
        .filter(|f| f.thread != thread)
        .collect();
    save(app, &follows);
}

/// The pull request a transcript reviewed: the agent posted a review or
/// review comments. `repo` is the project's GitHub repository, for commands
/// that don't name one.
pub fn find_review(items: &[TranscriptItem], repo: Option<&str>) -> Option<(String, u64)> {
    let mut texts: Vec<String> = Vec::new();
    for item in items {
        match &item.content {
            ItemContent::ToolUse { input, .. } => {
                if let Some(command) = input["command"].as_str() {
                    texts.push(command.to_string());
                }
            }
            ItemContent::ToolResult { content, .. } => texts.push(content.clone()),
            ItemContent::Assistant { text, .. } => texts.push(text.clone()),
            _ => {}
        }
    }
    let reviewed = texts.iter().any(|t| {
        t.contains("gh pr review")
            || t.contains("gh pr comment")
            || t.contains("pullrequestreview-")
            || (t.contains("gh api")
                && t.contains("/pulls/")
                && (t.contains("/reviews") || t.contains("/comments")))
    });
    if !reviewed {
        return None;
    }
    // The last pull request mentioned: by address, or a gh command's number.
    texts
        .iter()
        .rev()
        .find_map(|t| pr_url(t).or_else(|| gh_pr_number(t, repo)))
}

/// `github.com/<owner>/<name>/pull/<n>` in a text (the last one).
fn pr_url(text: &str) -> Option<(String, u64)> {
    text.rmatch_indices("github.com/").find_map(|(at, _)| {
        let rest = &text[at + "github.com/".len()..];
        let mut parts = rest.splitn(4, '/');
        let (owner, name, kind, tail) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        let number: String = tail.chars().take_while(char::is_ascii_digit).collect();
        (kind == "pull" && !owner.is_empty() && !name.is_empty())
            .then(|| Some((format!("{owner}/{name}"), number.parse().ok()?)))
            .flatten()
    })
}

/// `gh pr review|view|diff|comment <n> [--repo owner/name]`.
fn gh_pr_number(text: &str, repo: Option<&str>) -> Option<(String, u64)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    words.windows(4).rev().find_map(|w| {
        if w[0] != "gh"
            || w[1] != "pr"
            || !matches!(w[2], "review" | "view" | "diff" | "comment" | "checkout")
        {
            return None;
        }
        let number: u64 = w[3]
            .trim_matches(|c: char| !c.is_ascii_digit())
            .parse()
            .ok()?;
        let named = words
            .windows(2)
            .find(|p| p[0] == "--repo" || p[0] == "-R")
            .map(|p| p[1].to_string());
        Some((named.or_else(|| repo.map(str::to_string))?, number))
    })
}

pub fn init(app: Entity<AppState>, cx: &mut App) {
    cx.set_global(Following { me: None });
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            cx.background_executor().timer(poll_every()).await;
            poll(&app, cx).await;
        }
    })
    .detach();
}

/// Who `gh` is signed in as, asked once.
async fn me(cx: &mut AsyncApp) -> Option<String> {
    if let Some(me) = cx.update(|cx| cx.global::<Following>().me.clone()) {
        return Some(me);
    }
    let me = cx
        .background_executor()
        .spawn(async { elyra_git::github::signed_in_as().ok() })
        .await?;
    cx.update(|cx| cx.global_mut::<Following>().me = Some(me.clone()));
    Some(me)
}

/// After a turn: start following the pull request the thread reviewed.
pub fn after_turn(app: &Entity<AppState>, thread: ThreadId, cx: &mut App) {
    let Some(session) = app.read(cx).existing_session(thread) else {
        return;
    };
    let (items, dir) = {
        let session = session.read(cx);
        (session.items.clone(), session.working_dir())
    };
    let app = app.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let repo = cx
            .background_executor()
            .spawn(
                async move { elyra_git::remote_url(&dir).and_then(|u| elyra_git::github_slug(&u)) },
            )
            .await;
        let Some((repo, number)) = find_review(&items, repo.as_deref()) else {
            return;
        };
        let known = cx.update(|cx| {
            load(app.read(cx))
                .iter()
                .any(|f| f.thread == thread && f.repo == repo && f.number == number)
        });
        if known {
            return;
        }
        let Some(me) = me(cx).await else {
            return;
        };
        let (job_repo, job_me) = (repo.clone(), me.clone());
        let progress = cx
            .background_executor()
            .spawn(async move { elyra_git::github::pr_progress(&job_repo, number, &job_me) })
            .await;
        let Ok(progress) = progress else {
            return;
        };
        if progress.state != "OPEN" {
            return;
        }
        let reviewed = progress
            .my_review
            .clone()
            .unwrap_or_else(|| progress.head.clone());
        cx.update(|cx| {
            let state = app.read(cx);
            let mut follows: Vec<Follow> = load(state)
                .into_iter()
                .filter(|f| f.thread != thread)
                .collect();
            follows.push(Follow {
                thread,
                repo,
                number,
                reviewed: reviewed.clone(),
                notified: progress.head.clone(),
                told_requested: progress.requested,
            });
            save(state, &follows);
        });
    })
    .detach();
}

/// Look at every followed pull request now.
pub async fn poll(app: &Entity<AppState>, cx: &mut AsyncApp) {
    let follows = cx.update(|cx| load(app.read(cx)));
    if follows.is_empty() {
        return;
    }
    let Some(me) = me(cx).await else {
        return;
    };
    for follow in follows {
        let (repo, me_job) = (follow.repo.clone(), me.clone());
        let number = follow.number;
        let progress = cx
            .background_executor()
            .spawn(async move { elyra_git::github::pr_progress(&repo, number, &me_job) })
            .await;
        let Ok(progress) = progress else {
            continue;
        };
        cx.update(|cx| update(app, &follow, &progress, cx));
    }
}

fn update(
    app: &Entity<AppState>,
    follow: &Follow,
    progress: &elyra_git::github::PrProgress,
    cx: &mut App,
) {
    let session = app.update(cx, |app, cx| app.session(follow.thread, cx));
    let mut next = follow.clone();
    if progress.state != "OPEN" {
        let state = app.read(cx);
        stop(state, follow.thread);
        if let Some(session) = session {
            let what = if progress.state == "MERGED" {
                "merged"
            } else {
                "closed"
            };
            session.update(cx, |session, cx| {
                session.notice(
                    format!("PR #{} was {what}; no longer following it.", follow.number),
                    false,
                    cx,
                )
            });
        }
        return;
    }
    // Reviewed again since (by you or the agent): that is the new baseline.
    if let Some(mine) = &progress.my_review
        && progress.commits.iter().position(|c| c == mine)
            > progress.commits.iter().position(|c| *c == follow.reviewed)
    {
        next.reviewed = mine.clone();
        next.notified = progress.head.clone();
        next.told_requested = progress.requested;
    }
    let new_commits = progress.head != next.reviewed && progress.head != next.notified;
    let asked_again = progress.requested && !next.told_requested;
    if (new_commits || asked_again)
        && let Some(session) = session
    {
        let commits = progress.commits_after(&next.reviewed);
        let card = ItemContent::PrUpdate {
            repo: follow.repo.clone(),
            number: follow.number,
            title: progress.title.clone(),
            url: progress.url.clone(),
            commits,
            requested: progress.requested,
            from: next.reviewed.clone(),
            to: progress.head.clone(),
            handled: false,
        };
        session.update(cx, |session, cx| session.add_pr_update(card, cx));
        let message = summary(follow.number, commits, progress.requested);
        notify(app, follow.thread, &message, cx);
        next.notified = progress.head.clone();
        next.told_requested = progress.requested;
    }
    if next != *follow {
        let state = app.read(cx);
        let follows: Vec<Follow> = load(state)
            .into_iter()
            .map(|f| {
                if f.thread == follow.thread {
                    next.clone()
                } else {
                    f
                }
            })
            .collect();
        save(state, &follows);
    }
}

/// "PR #463: 2 new commits since your review, review requested again".
pub fn summary(number: u64, commits: usize, requested: bool) -> String {
    let mut parts = Vec::new();
    if commits > 0 {
        parts.push(format!(
            "{commits} new commit{} since your review",
            if commits == 1 { "" } else { "s" }
        ));
    }
    if requested {
        parts.push("review requested again".into());
    }
    format!("PR #{number}: {}", parts.join(", "))
}

/// In the app (and the system, when it isn't active) and on the phone.
fn notify(app: &Entity<AppState>, thread: ThreadId, message: &str, cx: &mut App) {
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::component::notification::Notification;
    let title = app
        .read(cx)
        .thread(thread)
        .map(|t| t.title.clone())
        .unwrap_or_default();
    if let Some(hub) = cx.try_global::<crate::browser_tools::BrowserHub>() {
        let (window, workspace) = (hub.window, hub.workspace.clone());
        let message = message.to_string();
        let _ = window.update(cx, move |_, window, cx| {
            let note = Notification::info(message)
                .title(title)
                .autohide(false)
                .on_click(move |_, window, cx| {
                    let _ = workspace.update(cx, |this, cx| this.activate(thread, window, cx));
                    cx.activate(true);
                });
            let note = if window.is_window_active() {
                note
            } else {
                note.in_app_and_system()
            };
            window.push_notification(note, cx);
        });
    }
    crate::phone::notify_message(app, thread, message, cx);
}

/// What the agent gets for "Review the changes": the new commits only, with
/// the earlier comments and the replies to them.
pub fn review_prompt(
    repo: &str,
    number: u64,
    from: &str,
    to: &str,
    commits: usize,
    points: &[elyra_git::github::ReviewPoint],
) -> String {
    let short = |sha: &str| sha.chars().take(9).collect::<String>();
    let mut out = format!(
        "The author updated PR #{number} ({repo}) since your review of {}: {} new commit{}, now at {}. \
         Review only what changed since then; you don't need to read the whole PR again:\n\n\
         gh api repos/{repo}/compare/{from}...{to}   (or: git fetch origin pull/{number}/head && git diff {from}..{to})\n",
        short(from),
        commits,
        if commits == 1 { "" } else { "s" },
        short(to),
    );
    if points.is_empty() {
        out.push_str("\nYour earlier review left no comments on GitHub; use what you wrote in this thread.\n");
    } else {
        out.push_str("\nYour earlier comments, with the replies to them:\n");
        for (index, point) in points.iter().take(40).enumerate() {
            let place = match (&point.path, point.line) {
                (Some(path), Some(line)) => format!("{path}:{line} — "),
                (Some(path), None) => format!("{path} — "),
                _ => String::new(),
            };
            let body: String = point.body.chars().take(500).collect();
            out.push_str(&format!(
                "{}. {place}{}\n",
                index + 1,
                body.replace('\n', " ")
            ));
            for (author, reply) in &point.replies {
                let reply: String = reply.chars().take(300).collect();
                out.push_str(&format!("   ↳ {author}: {}\n", reply.replace('\n', " ")));
            }
        }
    }
    out.push_str(
        "\nFor each earlier point, say whether it is addressed, partly addressed or not, with where. \
         Then list anything new that is wrong or risky in the changed code. \
         Answer in the language of this conversation, and ask me before posting a follow-up review or approving.",
    );
    out
}

/// "Review the changes" on a card: fetch the earlier comments, then send.
pub fn review_changes(
    app: &Entity<AppState>,
    thread: ThreadId,
    item: elyra_core::ItemId,
    cx: &mut App,
) {
    let Some(session) = app.update(cx, |app, cx| app.session(thread, cx)) else {
        return;
    };
    let Some(ItemContent::PrUpdate {
        repo,
        number,
        commits,
        from,
        to,
        ..
    }) = session
        .read(cx)
        .items
        .iter()
        .find(|i| i.id == item)
        .map(|i| i.content.clone())
    else {
        return;
    };
    session.update(cx, |session, cx| session.set_pr_update_handled(item, cx));
    cx.spawn(async move |cx: &mut AsyncApp| {
        let me = me(cx).await.unwrap_or_default();
        let (job_repo, job_me) = (repo.clone(), me.clone());
        let points = cx
            .background_executor()
            .spawn(async move {
                elyra_git::github::review_points(&job_repo, number, &job_me).unwrap_or_default()
            })
            .await;
        let prompt = review_prompt(&repo, number, &from, &to, commits, &points);
        cx.update(|cx| {
            session.update(cx, |session, cx| {
                session.submit(elyra_provider::Prompt::text(prompt), cx)
            })
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::{find_review, review_prompt, summary};
    use elyra_core::{ItemContent, TranscriptItem};
    use serde_json::json;

    fn tool(command: &str) -> TranscriptItem {
        TranscriptItem {
            id: elyra_core::new_id(),
            thread_id: elyra_core::new_id(),
            seq: 0,
            content: ItemContent::ToolUse {
                tool_use_id: "t".into(),
                name: "Bash".into(),
                input: json!({ "command": command }),
                parent_tool_use_id: None,
            },
            created_at: chrono::Utc::now(),
        }
    }

    fn said(text: &str) -> TranscriptItem {
        TranscriptItem {
            content: ItemContent::Assistant {
                text: text.into(),
                parent_tool_use_id: None,
            },
            ..tool("")
        }
    }

    #[test]
    fn finds_the_pull_request_a_thread_reviewed() {
        // Only looking is no review.
        assert_eq!(
            find_review(
                &[tool("gh pr view 463 --json headRefOid")],
                Some("getsno/outside")
            ),
            None
        );
        let posted = [
            tool("gh pr view 463 --json headRefOid"),
            tool("gh api repos/getsno/outside/pulls/463/reviews --input review.json"),
            said(
                "The review is on PR #463: https://github.com/getsno/outside/pull/463#pullrequestreview-5425845726",
            ),
        ];
        assert_eq!(
            find_review(&posted, None),
            Some(("getsno/outside".into(), 463))
        );
        // A gh command with the project's repository.
        assert_eq!(
            find_review(
                &[tool("gh pr review 12 --request-changes -b 'see comments'")],
                Some("o/r")
            ),
            Some(("o/r".into(), 12))
        );
        assert_eq!(
            find_review(
                &[tool("gh pr comment 7 --repo other/repo -b hi")],
                Some("o/r")
            ),
            Some(("other/repo".into(), 7))
        );
    }

    #[test]
    fn tells_the_agent_what_changed() {
        let points = [elyra_git::github::ReviewPoint {
            path: Some("app/Order.php".into()),
            line: Some(42),
            body: "Null check on address?".into(),
            replies: vec![("dev".into(), "Added in c2.".into())],
        }];
        let prompt = review_prompt("o/r", 7, "aaaaaaaaaaaa", "bbbbbbbbbbbb", 2, &points);
        assert!(prompt.contains("2 new commits, now at bbbbbbbbb"));
        assert!(prompt.contains("compare/aaaaaaaaaaaa...bbbbbbbbbbbb"));
        assert!(
            prompt.contains("1. app/Order.php:42 — Null check on address?\n   ↳ dev: Added in c2.")
        );
        assert_eq!(
            summary(7, 1, true),
            "PR #7: 1 new commit since your review, review requested again"
        );
        assert_eq!(
            summary(7, 3, false),
            "PR #7: 3 new commits since your review"
        );
    }
}
