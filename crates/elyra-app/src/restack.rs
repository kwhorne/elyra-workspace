//! Restacking a branch by instruction: its commits since it left main (and
//! what isn't committed yet) become one commit per instruction behind it, in
//! the order they were asked, each with the folder as that instruction's
//! turns left it, and a last one for what no agent wrote. The working tree
//! stays exactly as it is; the old commits are kept to put back.

use elyra_core::{ProjectId, Store};
use elyra_git::restack::NewCommit;
use std::path::Path;

/// A commit of the plan, and the files it changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub commit: NewCommit,
    pub title: String,
    pub files: usize,
}

/// What restacking would do.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub base: String,
    pub base_sha: String,
    pub branch: String,
    pub old_head: String,
    /// The commits it replaces (`<sha> <subject>`).
    pub replaces: Vec<String>,
    pub steps: Vec<Step>,
    /// The old commits are on a remote: replacing them needs a force push.
    pub pushed: bool,
}

/// Restacked: what to put back.
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub branch: String,
    pub old_head: String,
    pub new_head: String,
}

/// A commit title from an instruction: its first line, at most 72
/// characters; with Conventional Commits on, one that doesn't follow them
/// becomes a `chore:`.
pub fn title(instruction: &str, conventional: bool) -> String {
    let line = instruction
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Changes")
        .trim_end_matches('.');
    let line = if conventional && crate::conventional::problem(line).is_some() {
        let mut chars = line.chars();
        let first = chars
            .next()
            .map(|c| c.to_lowercase().to_string())
            .unwrap_or_default();
        format!("chore: {first}{}", chars.as_str())
    } else {
        line.to_string()
    };
    if line.chars().count() > 72 {
        format!("{}…", line.chars().take(71).collect::<String>())
    } else {
        line
    }
}

/// Work out the restack of the branch in `root`. Blocking.
pub fn plan(
    root: &Path,
    project: ProjectId,
    db: &Path,
    conventional: bool,
) -> anyhow::Result<Plan> {
    use elyra_git::restack;
    let branch =
        restack::branch_ref(root).ok_or_else(|| anyhow::anyhow!("HEAD isn't on a branch"))?;
    if let Some(what) = restack::operation_in_progress(root) {
        anyhow::bail!("finish or abort {what} first");
    }
    if !elyra_git::conflicted_files(root)?.is_empty() {
        anyhow::bail!("resolve the merge conflicts first");
    }
    let (base, base_sha) = elyra_git::branch_base(root)
        .ok_or_else(|| anyhow::anyhow!("no main or master branch to compare with"))?;
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()?;
    anyhow::ensure!(output.status.success(), "the branch has no commits");
    let old_head = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let replaces = restack::commits_between(root, &base_sha, &old_head)?;
    let short = branch.trim_start_matches("refs/heads/");
    if !replaces.is_empty() && matches!(short, "main" | "master") {
        anyhow::bail!(
            "{short} has commits of its own since {base}; restack a feature branch instead"
        );
    }

    // The instructions behind the branch, in the order they were asked.
    let overview = crate::instructions::load(root, project, db)?;
    let store = Store::open_read_only(db)?;
    let mut spans: Vec<(crate::undo::Span, String, String)> = Vec::new();
    for group in &overview.groups {
        let Some((thread, item)) = group.turn else {
            continue;
        };
        let items = store.transcript(thread)?;
        let Some(span) = crate::undo::span(&items, item) else {
            continue;
        };
        // Only turns made on this branch: the folder they started from
        // descends from the base and leads to HEAD.
        let parent = format!("{}^", span.before);
        if !restack::is_ancestor(root, &base_sha, &parent)
            || !restack::is_ancestor(root, &parent, &old_head)
        {
            continue;
        }
        spans.push((span, group.thread.clone(), group.agent.clone()));
    }
    spans.sort_by_key(|(span, _, _)| span.at);

    let mut steps: Vec<Step> = Vec::new();
    let mut previous = restack::tree_of(root, &base_sha)?;
    for index in 0..spans.len() {
        let (span, thread, agent) = &spans[index];
        // Where its turns ended; for turns from before that was kept, where
        // the next instruction began.
        let end = match &span.after {
            Some(after) => after.clone(),
            None => match spans.get(index + 1) {
                Some((next, _, _)) => next.before.clone(),
                None => continue,
            },
        };
        let tree = restack::tree_of(root, &end)?;
        if tree == previous {
            continue;
        }
        let title = title(&span.instruction, conventional);
        let mut message = format!("{title}\n\n");
        if span.instruction.lines().count() > 1 || span.instruction.chars().count() > 72 {
            let body: String = span.instruction.chars().take(1500).collect();
            message.push_str(&format!("{}\n\n", body.trim()));
        }
        message.push_str(&format!("Instruction in “{thread}” ({agent})."));
        steps.push(Step {
            files: changed_files(root, &previous, &tree),
            commit: NewCommit {
                tree: tree.clone(),
                message,
                author_date: Some(format!("@{} +0000", span.at.timestamp())),
            },
            title,
        });
        previous = tree;
    }
    // What came after the last instruction, or no agent wrote.
    let now = restack::tree_of(
        root,
        &elyra_git::checkpoint::snapshot(root, "elyra: restack")?,
    )?;
    if now != previous {
        let title = if conventional {
            "chore: other changes".to_string()
        } else {
            "Other changes".to_string()
        };
        steps.push(Step {
            files: changed_files(root, &previous, &now),
            commit: NewCommit {
                tree: now,
                message: format!("{title}\n\nNot written for an instruction in Elyra Workspace."),
                author_date: None,
            },
            title,
        });
    }
    anyhow::ensure!(!steps.is_empty(), "the branch has nothing to restack");
    Ok(Plan {
        pushed: !replaces.is_empty() && restack::is_pushed(root, &old_head),
        base,
        base_sha,
        branch,
        old_head,
        replaces,
        steps,
    })
}

fn changed_files(root: &Path, from: &str, to: &str) -> usize {
    elyra_git::undo::changed_paths(root, from, to)
        .map(|p| p.len())
        .unwrap_or(0)
}

/// Do the restack. Blocking.
pub fn apply(root: &Path, plan: &Plan) -> anyhow::Result<Done> {
    use elyra_git::restack;
    let commits: Vec<NewCommit> = plan.steps.iter().map(|s| s.commit.clone()).collect();
    let new_head = restack::build(root, &plan.base_sha, &commits)?;
    let backup = format!(
        "refs/elyra/restack/{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    restack::switch(root, &plan.branch, &plan.old_head, &new_head, &backup)?;
    Ok(Done {
        branch: plan.branch.clone(),
        old_head: plan.old_head.clone(),
        new_head,
    })
}

/// Put the old commits back (and what wasn't committed, uncommitted again).
pub fn undo(root: &Path, done: &Done) -> anyhow::Result<()> {
    elyra_git::restack::switch(
        root,
        &done.branch,
        &done.new_head,
        &done.old_head,
        "refs/elyra/restack/undone",
    )
}

/// The plan in words, for the confirmation and for agents.
pub fn describe(plan: &Plan) -> String {
    let mut text = format!(
        "{} commit{} on top of {}, one per instruction:\n",
        plan.steps.len(),
        if plan.steps.len() == 1 { "" } else { "s" },
        plan.base
    );
    for step in &plan.steps {
        text.push_str(&format!(
            "- {} ({} file{})\n",
            step.title,
            step.files,
            if step.files == 1 { "" } else { "s" }
        ));
    }
    if plan.replaces.is_empty() {
        text.push_str("\nNothing is committed on the branch yet; nothing is replaced.");
    } else {
        text.push_str(&format!(
            "\nIt replaces the branch's {} commit{} since {} (kept, to put back).",
            plan.replaces.len(),
            if plan.replaces.len() == 1 { "" } else { "s" },
            plan.base
        ));
    }
    if plan.pushed {
        text.push_str(" Those are pushed already: pushing the new ones needs a force push.");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::title;

    #[test]
    fn makes_titles_from_instructions() {
        assert_eq!(
            title("Round the totals after the discount.\nAnd test it.", false),
            "Round the totals after the discount"
        );
        assert_eq!(title("Round the totals", true), "chore: round the totals");
        assert_eq!(
            title("fix(cart): round totals", true),
            "fix(cart): round totals"
        );
        assert_eq!(title(&"x".repeat(100), false).chars().count(), 72);
    }
}
