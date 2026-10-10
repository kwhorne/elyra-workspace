//! Git operations for Elyra, implemented on top of the `git` CLI so behavior
//! matches the user's own Git configuration (hooks, signing, credentials).

pub mod checkpoint;
pub mod diff;
pub mod github;
pub mod ops;

use anyhow::{Context as _, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub use diff::{DiffHunk, DiffLine, DiffLineKind, FileDiff, parse_unified_diff};
pub use ops::*;

pub(crate) fn git(cwd: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0");
    command
}

pub(crate) fn run(cwd: &Path, args: &[&str]) -> Result<Output> {
    let output = git(cwd)
        .args(args)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    Ok(output)
}

pub(crate) fn run_ok(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = run(cwd, args)?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn is_repo(path: &Path) -> bool {
    run(path, &["rev-parse", "--is-inside-work-tree"])
        .map(|output| output.status.success())
        .unwrap_or(false)
}

pub fn repo_root(path: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(
        run_ok(path, &["rev-parse", "--show-toplevel"])?.trim(),
    ))
}

/// The repository's shared git directory: the same for all its worktrees.
pub fn common_dir(path: &Path) -> Result<PathBuf> {
    let dir = PathBuf::from(
        run_ok(
            path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?
        .trim(),
    );
    Ok(dir.canonicalize().unwrap_or(dir))
}

pub fn current_branch(path: &Path) -> Option<String> {
    let branch = run_ok(path, &["branch", "--show-current"]).ok()?;
    let branch = branch.trim();
    if branch.is_empty() {
        // Detached HEAD: show the short commit instead.
        run_ok(path, &["rev-parse", "--short", "HEAD"])
            .ok()
            .map(|sha| format!("({})", sha.trim()))
    } else {
        Some(branch.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl ChangeKind {
    pub fn letter(self) -> &'static str {
        match self {
            ChangeKind::Added => "A",
            ChangeKind::Modified => "M",
            ChangeKind::Deleted => "D",
            ChangeKind::Renamed => "R",
            ChangeKind::Untracked => "U",
            ChangeKind::Conflicted => "!",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub original_path: Option<String>,
    pub kind: ChangeKind,
    pub staged: bool,
}

/// Working-tree changes relative to HEAD, one entry per path.
pub fn status(path: &Path) -> Result<Vec<FileChange>> {
    let output = run_ok(
        path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    Ok(parse_status(&output))
}

pub(crate) fn parse_status(output: &str) -> Vec<FileChange> {
    let mut changes = Vec::new();
    let mut entries = output.split('\0').filter(|entry| !entry.is_empty());
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, file) = entry.split_at(3);
        let bytes = code.as_bytes();
        let (index, worktree) = (bytes[0], bytes[1]);
        let kind = match (index, worktree) {
            (b'?', b'?') => ChangeKind::Untracked,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => ChangeKind::Conflicted,
            (b'R', _) | (_, b'R') => ChangeKind::Renamed,
            (b'A', _) => ChangeKind::Added,
            (b'D', _) | (_, b'D') => ChangeKind::Deleted,
            _ => ChangeKind::Modified,
        };
        let original_path = if kind == ChangeKind::Renamed {
            entries.next().map(str::to_string)
        } else {
            None
        };
        changes.push(FileChange {
            path: file.to_string(),
            original_path,
            kind,
            staged: index != b' ' && index != b'?',
        });
    }
    changes
}

/// Unified diff of one file against HEAD, including untracked files.
pub fn diff_file(repo: &Path, change: &FileChange) -> Result<FileDiff> {
    let text = if change.kind == ChangeKind::Untracked {
        // `--no-index` exits 1 when files differ, which is the expected case.
        let output = run(
            repo,
            &[
                "diff",
                "--no-color",
                "--no-index",
                "--",
                "/dev/null",
                &change.path,
            ],
        )?;
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        let has_head = run(repo, &["rev-parse", "--verify", "HEAD"])?
            .status
            .success();
        let base = if has_head {
            "HEAD"
        } else {
            // Empty tree: lets a repository without commits show its files.
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
        };
        run_ok(repo, &["diff", "--no-color", base, "--", &change.path])?
    };
    Ok(parse_unified_diff(&change.path, &text))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffStat {
    pub files: usize,
    pub insertions: usize,
    pub deletions: usize,
}

pub fn diff_stat(repo: &Path) -> Result<DiffStat> {
    let changes = status(repo)?;
    let mut stat = DiffStat {
        files: changes.len(),
        ..Default::default()
    };
    if let Ok(numstat) = run_ok(repo, &["diff", "--numstat", "HEAD"]) {
        for line in numstat.lines() {
            let mut parts = line.split('\t');
            stat.insertions += parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            stat.deletions += parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        }
    }
    Ok(stat)
}

/// Bring a branch's changes into the working tree and index without
/// committing (`git merge --squash`), for the user to review and commit.
pub fn squash_merge(repo: &Path, branch: &str) -> Result<()> {
    run_ok(repo, &["merge", "--squash", branch]).map(|_| ())
}

/// The changes between two commits as a unified diff, for an agent to read.
pub fn diff_text(repo: &Path, from: &str, to: &str) -> Result<String> {
    run_ok(repo, &["diff", "--no-color", "--find-renames", from, to])
}

/// Stage everything and commit. Returns the new commit's short SHA.
pub fn commit_all(repo: &Path, message: &str) -> Result<String> {
    if message.trim().is_empty() {
        bail!("commit message is empty");
    }
    run_ok(repo, &["add", "--all"])?;
    run_ok(repo, &["commit", "--message", message])?;
    Ok(run_ok(repo, &["rev-parse", "--short", "HEAD"])?
        .trim()
        .to_string())
}

/// Discard all changes to one file, restoring it from HEAD (or deleting it
/// when untracked).
pub fn discard_file(repo: &Path, change: &FileChange) -> Result<()> {
    match change.kind {
        ChangeKind::Untracked => {
            std::fs::remove_file(repo.join(&change.path))
                .with_context(|| format!("removing {}", change.path))?;
        }
        ChangeKind::Added => {
            run_ok(repo, &["rm", "--force", "--quiet", "--", &change.path])?;
        }
        _ => {
            run_ok(
                repo,
                &[
                    "restore",
                    "--staged",
                    "--worktree",
                    "--source=HEAD",
                    "--",
                    &change.path,
                ],
            )?;
        }
    }
    Ok(())
}

pub fn push(repo: &Path) -> Result<String> {
    let output = run(repo, &["push", "--set-upstream", "origin", "HEAD"])?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() {
        bail!("git push failed: {}", text.trim());
    }
    Ok(text)
}

/// Create a managed worktree at `dest` on a new branch from the repository's
/// current HEAD.
pub fn create_worktree(repo: &Path, dest: &Path, branch: &str) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let dest = dest.to_string_lossy();
    run_ok(repo, &["worktree", "add", "-b", branch, &dest, "HEAD"])?;
    Ok(())
}

pub fn remove_worktree(repo: &Path, dest: &Path) -> Result<()> {
    run_ok(
        repo,
        &["worktree", "remove", "--force", &dest.to_string_lossy()],
    )?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fresh repository on branch `main` with a test identity.
    pub(crate) fn temp_repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "elyra-git-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        run_ok(&dir, &["init", "--quiet", "--initial-branch=main"]).unwrap();
        run_ok(&dir, &["config", "user.email", "test@example.com"]).unwrap();
        run_ok(&dir, &["config", "user.name", "Test"]).unwrap();
        run_ok(&dir, &["config", "commit.gpgsign", "false"]).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn squash_merges_a_worktree_branch_and_diffs_commits() {
        let repo = temp_repo("squash");
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        commit_all(&repo, "base").unwrap();
        let base = run_ok(&repo, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        let wt = repo.with_extension("wt");
        create_worktree(&repo, &wt, "elyra/candidate").unwrap();
        std::fs::write(wt.join("a.txt"), "two\n").unwrap();
        std::fs::write(wt.join("b.txt"), "new\n").unwrap();
        commit_all(&wt, "candidate").unwrap();
        let tip = run_ok(&wt, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();

        let diff = diff_text(&repo, &base, &tip).unwrap();
        assert!(diff.contains("-one") && diff.contains("+two") && diff.contains("b.txt"));

        squash_merge(&repo, "elyra/candidate").unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "two\n"
        );
        let staged = run_ok(&repo, &["diff", "--cached", "--name-only"]).unwrap();
        assert!(staged.contains("a.txt") && staged.contains("b.txt"));
        // Nothing is committed for the user.
        assert_eq!(run_ok(&repo, &["rev-parse", "HEAD"]).unwrap().trim(), base);

        remove_worktree(&repo, &wt).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn parses_porcelain_status() {
        let output = " M src/a.rs\0?? new.txt\0R  b.rs\0old_b.rs\0A  c.rs\0 D d.rs\0UU e.rs\0";
        let changes = parse_status(output);
        let kinds: Vec<_> = changes.iter().map(|c| (c.path.as_str(), c.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                ("src/a.rs", ChangeKind::Modified),
                ("new.txt", ChangeKind::Untracked),
                ("b.rs", ChangeKind::Renamed),
                ("c.rs", ChangeKind::Added),
                ("d.rs", ChangeKind::Deleted),
                ("e.rs", ChangeKind::Conflicted),
            ]
        );
        assert_eq!(changes[2].original_path.as_deref(), Some("old_b.rs"));
        assert!(!changes[0].staged);
        assert!(changes[3].staged);
    }

    #[test]
    fn status_diff_and_commit_in_temp_repo() {
        let dir = std::env::temp_dir().join(format!("elyra-git-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        run_ok(&dir, &["init", "--quiet", "--initial-branch=main"]).unwrap();
        run_ok(&dir, &["config", "user.email", "test@example.com"]).unwrap();
        run_ok(&dir, &["config", "user.name", "Test"]).unwrap();
        run_ok(&dir, &["config", "commit.gpgsign", "false"]).unwrap();
        assert!(is_repo(&dir));

        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        let changes = status(&dir).unwrap();
        assert_eq!(changes.len(), 1);
        let diff = diff_file(&dir, &changes[0]).unwrap();
        assert_eq!(diff.additions(), 2);

        commit_all(&dir, "initial").unwrap();
        assert!(status(&dir).unwrap().is_empty());
        assert_eq!(current_branch(&dir).as_deref(), Some("main"));

        std::fs::write(dir.join("a.txt"), "one\nthree\n").unwrap();
        let changes = status(&dir).unwrap();
        let diff = diff_file(&dir, &changes[0]).unwrap();
        assert_eq!((diff.additions(), diff.deletions()), (1, 1));

        discard_file(&dir, &changes[0]).unwrap();
        assert!(status(&dir).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
