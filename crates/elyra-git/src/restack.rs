//! Rewriting a branch as a new series of commits, given the tree each one
//! should have: the working tree ends up exactly as it was, the old commits
//! are kept under a ref, and the switch can be undone.

use crate::run_ok;
use anyhow::{Result, bail};
use std::path::Path;
use std::process::Command;

/// One commit of the new series.
#[derive(Clone, Debug, PartialEq)]
pub struct NewCommit {
    pub tree: String,
    pub message: String,
    /// Git's date format (e.g. `@1760000000 +0000`); None: now.
    pub author_date: Option<String>,
}

/// The tree of a commit.
pub fn tree_of(repo: &Path, commit: &str) -> Result<String> {
    Ok(run_ok(repo, &["rev-parse", &format!("{commit}^{{tree}}")])?
        .trim()
        .to_string())
}

/// Whether `ancestor` is `descendant` or comes before it.
pub fn is_ancestor(repo: &Path, ancestor: &str, descendant: &str) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .status()
        .is_ok_and(|s| s.success())
}

/// The branch HEAD is on, as a ref (`refs/heads/…`); None when detached.
pub fn branch_ref(repo: &Path) -> Option<String> {
    run_ok(repo, &["symbolic-ref", "-q", "HEAD"])
        .ok()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
}

/// A merge, rebase or cherry-pick that isn't finished.
pub fn operation_in_progress(repo: &Path) -> Option<&'static str> {
    let path = |name: &str| {
        run_ok(repo, &["rev-parse", "--git-path", name])
            .map(|p| repo.join(p.trim()))
            .ok()
    };
    [
        ("MERGE_HEAD", "a merge"),
        ("rebase-merge", "a rebase"),
        ("rebase-apply", "a rebase"),
        ("CHERRY_PICK_HEAD", "a cherry-pick"),
        ("REVERT_HEAD", "a revert"),
    ]
    .into_iter()
    .find(|(name, _)| path(name).is_some_and(|p| p.exists()))
    .map(|(_, what)| what)
}

/// Whether `commit` is on a remote branch already (rewriting it means a
/// force push).
pub fn is_pushed(repo: &Path, commit: &str) -> bool {
    run_ok(repo, &["branch", "-r", "--contains", commit]).is_ok_and(|out| !out.trim().is_empty())
}

/// The commits from `base` to `head`, oldest first, as `<short sha> <subject>`.
pub fn commits_between(repo: &Path, base: &str, head: &str) -> Result<Vec<String>> {
    Ok(run_ok(
        repo,
        &[
            "log",
            "--reverse",
            "--format=%h %s",
            &format!("{base}..{head}"),
        ],
    )?
    .lines()
    .map(str::to_string)
    .collect())
}

/// Make the series on top of `base` with the user's own identity; returns
/// the new head. Nothing is moved yet.
pub fn build(repo: &Path, base: &str, commits: &[NewCommit]) -> Result<String> {
    let mut parent = base.to_string();
    for commit in commits {
        let mut command = Command::new("git");
        command.current_dir(repo).args([
            "commit-tree",
            &commit.tree,
            "-p",
            &parent,
            "-m",
            &commit.message,
        ]);
        if let Some(date) = &commit.author_date {
            command.env("GIT_AUTHOR_DATE", date);
        }
        let output = command.output()?;
        if !output.status.success() {
            bail!(
                "git commit-tree failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        parent = String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
    Ok(parent)
}

/// Move `branch` from `old_head` to `new_head` (only if it is still at
/// `old_head`) and set the index to match; the working tree isn't touched.
/// The old head is kept as `backup`.
pub fn switch(
    repo: &Path,
    branch: &str,
    old_head: &str,
    new_head: &str,
    backup: &str,
) -> Result<()> {
    run_ok(repo, &["update-ref", backup, old_head])?;
    run_ok(
        repo,
        &[
            "update-ref",
            "-m",
            "elyra: restack",
            branch,
            new_head,
            old_head,
        ],
    )?;
    run_ok(repo, &["reset", "--quiet", "--mixed", new_head])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{NewCommit, branch_ref, build, commits_between, is_ancestor, switch, tree_of};
    use crate::checkpoint::snapshot;
    use crate::tests::temp_repo;
    use crate::{commit_staged, stage_all, status};

    #[test]
    fn rewrites_a_branch_as_new_commits_and_back() {
        let repo = temp_repo("restack");
        let write = |name: &str, text: &str| std::fs::write(repo.join(name), text).unwrap();
        write("a.txt", "a\n");
        stage_all(&repo).unwrap();
        let base = commit_staged(&repo, "base").unwrap();
        write("b.txt", "b\n");
        let first = snapshot(&repo, "after one").unwrap();
        write("c.txt", "c\n");
        stage_all(&repo).unwrap();
        commit_staged(&repo, "mixed work").unwrap();
        write("d.txt", "d (not committed)\n");
        let now = snapshot(&repo, "now").unwrap();
        let old_head = super::super::run_ok(&repo, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();

        let series = vec![
            NewCommit {
                tree: tree_of(&repo, &first).unwrap(),
                message: "Add b\n\nThread: one".into(),
                author_date: Some("@1760000000 +0000".into()),
            },
            NewCommit {
                tree: tree_of(&repo, &now).unwrap(),
                message: "Add c and d".into(),
                author_date: None,
            },
        ];
        let new_head = build(&repo, &base, &series).unwrap();
        assert!(is_ancestor(&repo, &base, &new_head));
        let branch = branch_ref(&repo).unwrap();
        switch(
            &repo,
            &branch,
            &old_head,
            &new_head,
            "refs/elyra/restack/test",
        )
        .unwrap();

        let log = commits_between(&repo, &base, "HEAD").unwrap();
        assert_eq!(log.len(), 2);
        assert!(
            log[0].ends_with(" Add b") && log[1].ends_with(" Add c and d"),
            "{log:?}"
        );
        assert!(
            status(&repo).unwrap().is_empty(),
            "the working tree is all committed"
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("d.txt")).unwrap(),
            "d (not committed)\n"
        );
        let date = super::super::run_ok(&repo, &["log", "-1", "--format=%at", "HEAD~1"]).unwrap();
        assert_eq!(date.trim(), "1760000000", "the instruction's time");

        // And back: the old commits, with d.txt uncommitted again.
        switch(
            &repo,
            &branch,
            &new_head,
            &old_head,
            "refs/elyra/restack/undo",
        )
        .unwrap();
        assert_eq!(commits_between(&repo, &base, "HEAD").unwrap().len(), 1);
        assert_eq!(
            status(&repo).unwrap().len(),
            1,
            "d.txt is uncommitted again"
        );
        std::fs::remove_dir_all(&repo).unwrap();
    }
}
