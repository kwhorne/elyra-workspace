//! Staging, diff sources, branches, sync and blame.

use crate::{ChangeKind, FileChange, FileDiff, parse_unified_diff, run, run_ok};
use anyhow::{Result, bail};
use std::path::Path;

/// Which side of the index a change lives on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Area {
    Staged,
    Unstaged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AreaChange {
    pub change: FileChange,
    pub area: Area,
}

fn kind_of(code: u8) -> ChangeKind {
    match code {
        b'A' => ChangeKind::Added,
        b'D' => ChangeKind::Deleted,
        b'R' | b'C' => ChangeKind::Renamed,
        b'U' => ChangeKind::Conflicted,
        b'?' => ChangeKind::Untracked,
        _ => ChangeKind::Modified,
    }
}

/// Changes split into staged and unstaged entries (a file can be in both).
pub fn status_by_area(repo: &Path) -> Result<Vec<AreaChange>> {
    let output = run_ok(
        repo,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    Ok(parse_status_by_area(&output))
}

pub(crate) fn parse_status_by_area(output: &str) -> Vec<AreaChange> {
    let mut changes = Vec::new();
    let mut entries = output.split('\0').filter(|entry| !entry.is_empty());
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, file) = entry.split_at(3);
        let bytes = code.as_bytes();
        let (index, worktree) = (bytes[0], bytes[1]);
        let renamed = index == b'R' || index == b'C';
        let original_path = renamed
            .then(|| entries.next().map(str::to_string))
            .flatten();
        let conflicted = index == b'U'
            || worktree == b'U'
            || (index == b'A' && worktree == b'A')
            || (index == b'D' && worktree == b'D');
        let make = |kind: ChangeKind, area: Area| AreaChange {
            change: FileChange {
                path: file.to_string(),
                original_path: original_path.clone(),
                kind,
                staged: area == Area::Staged,
            },
            area,
        };
        if conflicted {
            changes.push(make(ChangeKind::Conflicted, Area::Unstaged));
            continue;
        }
        if index == b'?' {
            changes.push(make(ChangeKind::Untracked, Area::Unstaged));
            continue;
        }
        if index != b' ' {
            changes.push(make(kind_of(index), Area::Staged));
        }
        if worktree != b' ' {
            changes.push(make(kind_of(worktree), Area::Unstaged));
        }
    }
    changes
}

pub fn stage(repo: &Path, paths: &[&str]) -> Result<()> {
    let mut args = vec!["add", "--all", "--"];
    args.extend(paths);
    run_ok(repo, &args)?;
    Ok(())
}

pub fn unstage(repo: &Path, paths: &[&str]) -> Result<()> {
    let has_head = run(repo, &["rev-parse", "--verify", "HEAD"])?
        .status
        .success();
    let mut args = if has_head {
        vec!["restore", "--staged", "--"]
    } else {
        vec!["rm", "--cached", "--quiet", "-r", "--"]
    };
    args.extend(paths);
    run_ok(repo, &args)?;
    Ok(())
}

pub fn stage_all(repo: &Path) -> Result<()> {
    run_ok(repo, &["add", "--all"])?;
    Ok(())
}

pub fn unstage_all(repo: &Path) -> Result<()> {
    if run(repo, &["rev-parse", "--verify", "HEAD"])?
        .status
        .success()
    {
        run_ok(repo, &["reset", "--quiet"])?;
    } else {
        run_ok(repo, &["rm", "--cached", "--quiet", "-r", "."])?;
    }
    Ok(())
}

pub fn has_staged(repo: &Path) -> Result<bool> {
    Ok(!run(repo, &["diff", "--cached", "--quiet"])?
        .status
        .success())
}

/// Commit what is staged. Returns the new short SHA.
pub fn commit_staged(repo: &Path, message: &str) -> Result<String> {
    if message.trim().is_empty() {
        bail!("commit message is empty");
    }
    if !has_staged(repo)? {
        bail!("nothing is staged");
    }
    run_ok(repo, &["commit", "--message", message])?;
    Ok(run_ok(repo, &["rev-parse", "--short", "HEAD"])?
        .trim()
        .to_string())
}

/// What a diff compares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffSource {
    /// HEAD against the working tree (staged and unstaged together).
    WorkingTree,
    /// Index against the working tree.
    Unstaged,
    /// HEAD against the index.
    Staged,
    /// A commit, branch or tag against the working tree.
    Ref(String),
    /// Between two commits.
    Range(String, String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffOptions {
    pub ignore_whitespace: bool,
    pub context_lines: Option<u32>,
}

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

fn head_or_empty(repo: &Path) -> Result<String> {
    Ok(
        if run(repo, &["rev-parse", "--verify", "HEAD"])?
            .status
            .success()
        {
            "HEAD".into()
        } else {
            EMPTY_TREE.into()
        },
    )
}

fn source_args(repo: &Path, source: &DiffSource) -> Result<Vec<String>> {
    Ok(match source {
        DiffSource::WorkingTree => vec![head_or_empty(repo)?],
        DiffSource::Unstaged => vec![],
        DiffSource::Staged => vec!["--cached".into(), head_or_empty(repo)?],
        DiffSource::Ref(reference) => vec![reference.clone()],
        DiffSource::Range(from, to) => vec![from.clone(), to.clone()],
    })
}

/// Diff of one path for the given source.
pub fn diff_path(
    repo: &Path,
    path: &str,
    untracked: bool,
    source: &DiffSource,
    options: DiffOptions,
) -> Result<FileDiff> {
    let mut args: Vec<String> = vec!["diff".into(), "--no-color".into(), "--no-ext-diff".into()];
    if options.ignore_whitespace {
        args.push("-w".into());
    }
    if let Some(context) = options.context_lines {
        args.push(format!("-U{context}"));
    }
    let text = if untracked && matches!(source, DiffSource::WorkingTree | DiffSource::Unstaged) {
        args.extend([
            "--no-index".into(),
            "--".into(),
            "/dev/null".into(),
            path.into(),
        ]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        String::from_utf8_lossy(&run(repo, &refs)?.stdout).into_owned()
    } else {
        args.extend(source_args(repo, source)?);
        args.extend(["--".into(), path.into()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        run_ok(repo, &refs)?
    };
    Ok(parse_unified_diff(path, &text))
}

/// Files changed between two commits (or a commit and the working tree).
pub fn changed_files(repo: &Path, source: &DiffSource) -> Result<Vec<FileChange>> {
    let mut args: Vec<String> = vec![
        "diff".into(),
        "--name-status".into(),
        "-z".into(),
        "-M".into(),
    ];
    args.extend(source_args(repo, source)?);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = run_ok(repo, &refs)?;
    let mut changes = Vec::new();
    let mut parts = output.split('\0').filter(|p| !p.is_empty());
    while let Some(status) = parts.next() {
        let code = status.as_bytes().first().copied().unwrap_or(b'M');
        let (original_path, path) = if code == b'R' || code == b'C' {
            (parts.next().map(str::to_string), parts.next())
        } else {
            (None, parts.next())
        };
        let Some(path) = path else { break };
        changes.push(FileChange {
            path: path.to_string(),
            original_path,
            kind: kind_of(code),
            staged: false,
        });
    }
    Ok(changes)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub upstream: Option<String>,
    pub current: bool,
    /// Unix seconds of the last commit.
    pub updated: i64,
}

pub fn branches(repo: &Path) -> Result<Vec<Branch>> {
    let output = run_ok(
        repo,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(HEAD)%09%(refname:short)%09%(upstream:short)%09%(committerdate:unix)",
            "refs/heads",
        ],
    )?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let head = parts.next()?;
            let name = parts.next()?.to_string();
            let upstream = parts.next().filter(|u| !u.is_empty()).map(str::to_string);
            let updated = parts.next().and_then(|t| t.parse().ok()).unwrap_or(0);
            Some(Branch {
                name,
                upstream,
                current: head == "*",
                updated,
            })
        })
        .collect())
}

pub fn valid_branch_name(repo: &Path, name: &str) -> bool {
    !name.trim().is_empty()
        && run(repo, &["check-ref-format", "--branch", name])
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Create a branch at HEAD and switch to it (uncommitted changes come along).
pub fn create_branch(repo: &Path, name: &str) -> Result<()> {
    if !valid_branch_name(repo, name) {
        bail!("\"{name}\" is not a valid branch name");
    }
    run_ok(repo, &["switch", "--create", name])?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitchOutcome {
    Switched,
    /// Local changes would conflict; they were stashed under this message.
    Stashed(String),
}

/// Switch branches. Uncommitted changes are carried when Git allows it;
/// otherwise they are stashed first (and kept in the stash).
pub fn switch_branch(repo: &Path, name: &str) -> Result<SwitchOutcome> {
    let first = run(repo, &["switch", name])?;
    if first.status.success() {
        return Ok(SwitchOutcome::Switched);
    }
    let stderr = String::from_utf8_lossy(&first.stderr);
    if !stderr.contains("would be overwritten") && !stderr.contains("commit your changes") {
        bail!("git switch {name} failed: {}", stderr.trim());
    }
    let message = format!("elyra: changes before switching to {name}");
    run_ok(
        repo,
        &[
            "stash",
            "push",
            "--include-untracked",
            "--message",
            &message,
        ],
    )?;
    run_ok(repo, &["switch", name])?;
    Ok(SwitchOutcome::Stashed(message))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncState {
    pub ahead: usize,
    pub behind: usize,
    pub has_upstream: bool,
}

pub fn sync_state(repo: &Path) -> SyncState {
    let Ok(output) = run_ok(
        repo,
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
    ) else {
        return SyncState::default();
    };
    let mut parts = output.split_whitespace().filter_map(|n| n.parse().ok());
    SyncState {
        ahead: parts.next().unwrap_or(0),
        behind: parts.next().unwrap_or(0),
        has_upstream: true,
    }
}

pub fn fetch(repo: &Path) -> Result<()> {
    run_ok(repo, &["fetch", "--quiet", "--prune"])?;
    Ok(())
}

/// Fast-forward from the upstream.
pub fn pull(repo: &Path) -> Result<String> {
    let output = run(repo, &["pull", "--ff-only"])?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() {
        bail!("git pull failed: {}", text.trim());
    }
    Ok(text.trim().to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub summary: String,
    pub author: String,
    pub time: i64,
}

pub fn recent_commits(repo: &Path, limit: usize) -> Result<Vec<Commit>> {
    let output = run_ok(
        repo,
        &[
            "log",
            &format!("-{limit}"),
            "--format=%h%x09%an%x09%at%x09%s",
        ],
    )?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\t');
            Some(Commit {
                sha: parts.next()?.to_string(),
                author: parts.next()?.to_string(),
                time: parts.next()?.parse().unwrap_or(0),
                summary: parts.next().unwrap_or("").to_string(),
            })
        })
        .collect())
}

/// Who last changed a line (1-based), or `None` for uncommitted lines.
pub fn blame_line(repo: &Path, path: &str, line: u32) -> Result<Option<Commit>> {
    let range = format!("{line},{line}");
    let output = run_ok(repo, &["blame", "--porcelain", "-L", &range, "--", path])?;
    let mut lines = output.lines();
    let sha = lines
        .next()
        .and_then(|l| l.split_whitespace().next())
        .unwrap_or("");
    if sha.chars().all(|c| c == '0') {
        return Ok(None);
    }
    let mut commit = Commit {
        sha: sha.chars().take(8).collect(),
        summary: String::new(),
        author: String::new(),
        time: 0,
    };
    for line in lines {
        if let Some(author) = line.strip_prefix("author ") {
            commit.author = author.to_string();
        } else if let Some(time) = line.strip_prefix("author-time ") {
            commit.time = time.parse().unwrap_or(0);
        } else if let Some(summary) = line.strip_prefix("summary ") {
            commit.summary = summary.to_string();
        }
    }
    Ok(Some(commit))
}

/// The `origin` (or first) remote URL.
pub fn remote_url(repo: &Path) -> Option<String> {
    run_ok(repo, &["remote", "get-url", "origin"])
        .ok()
        .or_else(|| {
            let remote = run_ok(repo, &["remote"]).ok()?;
            let first = remote.lines().next()?.to_string();
            run_ok(repo, &["remote", "get-url", &first]).ok()
        })
        .map(|url| url.trim().to_string())
}

/// `owner/name` for a GitHub remote URL.
pub fn github_slug(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.split_once("github.com/").map(|(_, rest)| rest))?;
    let slug = rest.trim_end_matches('/').trim_end_matches(".git");
    (slug.split('/').count() == 2).then(|| slug.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::temp_repo;

    #[test]
    fn splits_staged_and_unstaged() {
        let changes =
            parse_status_by_area("MM a.rs\0A  b.rs\0?? c.rs\0R  new.rs\0old.rs\0UU d.rs\0");
        let summary: Vec<_> = changes
            .iter()
            .map(|c| (c.change.path.as_str(), c.area, c.change.kind))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("a.rs", Area::Staged, ChangeKind::Modified),
                ("a.rs", Area::Unstaged, ChangeKind::Modified),
                ("b.rs", Area::Staged, ChangeKind::Added),
                ("c.rs", Area::Unstaged, ChangeKind::Untracked),
                ("new.rs", Area::Staged, ChangeKind::Renamed),
                ("d.rs", Area::Unstaged, ChangeKind::Conflicted),
            ]
        );
        assert_eq!(changes[4].change.original_path.as_deref(), Some("old.rs"));
    }

    #[test]
    fn parses_github_slugs() {
        assert_eq!(
            github_slug("git@github.com:kwhorne/elyra.git").as_deref(),
            Some("kwhorne/elyra")
        );
        assert_eq!(
            github_slug("https://github.com/kwhorne/elyra-workspace").as_deref(),
            Some("kwhorne/elyra-workspace")
        );
        assert_eq!(github_slug("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn stage_commit_branch_and_blame() {
        let repo = temp_repo("ops");
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        stage(&repo, &["a.txt"]).unwrap();
        assert!(has_staged(&repo).unwrap());
        commit_staged(&repo, "first").unwrap();

        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(repo.join("b.txt"), "new\n").unwrap();
        stage(&repo, &["a.txt"]).unwrap();
        let areas = status_by_area(&repo).unwrap();
        assert!(
            areas
                .iter()
                .any(|c| c.change.path == "a.txt" && c.area == Area::Staged)
        );
        assert!(
            areas
                .iter()
                .any(|c| c.change.path == "b.txt" && c.area == Area::Unstaged)
        );
        let staged = diff_path(
            &repo,
            "a.txt",
            false,
            &DiffSource::Staged,
            DiffOptions::default(),
        )
        .unwrap();
        assert_eq!(staged.additions(), 1);
        unstage(&repo, &["a.txt"]).unwrap();
        assert!(!has_staged(&repo).unwrap());
        assert!(commit_staged(&repo, "nothing").is_err());

        create_branch(&repo, "feature/x").unwrap();
        stage_all(&repo).unwrap();
        commit_staged(&repo, "on feature").unwrap();
        let names: Vec<_> = branches(&repo)
            .unwrap()
            .into_iter()
            .map(|b| (b.name, b.current))
            .collect();
        assert!(names.contains(&("feature/x".to_string(), true)));
        assert!(!valid_branch_name(&repo, "bad..name"));

        let files =
            changed_files(&repo, &DiffSource::Range("main".into(), "feature/x".into())).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(
            switch_branch(&repo, "main").unwrap(),
            SwitchOutcome::Switched
        );
        let blame = blame_line(&repo, "a.txt", 1).unwrap().unwrap();
        assert_eq!(blame.summary, "first");
        assert_eq!(recent_commits(&repo, 5).unwrap()[0].summary, "first");
        std::fs::remove_dir_all(&repo).unwrap();
    }

    #[test]
    fn switching_with_conflicting_changes_stashes_them() {
        let repo = temp_repo("switch");
        std::fs::write(repo.join("f.txt"), "base\n").unwrap();
        stage_all(&repo).unwrap();
        commit_staged(&repo, "base").unwrap();
        create_branch(&repo, "other").unwrap();
        std::fs::write(repo.join("f.txt"), "other\n").unwrap();
        stage_all(&repo).unwrap();
        commit_staged(&repo, "other").unwrap();
        switch_branch(&repo, "main").unwrap();
        std::fs::write(repo.join("f.txt"), "local edit\n").unwrap();
        let outcome = switch_branch(&repo, "other").unwrap();
        assert!(matches!(outcome, SwitchOutcome::Stashed(_)));
        assert_eq!(
            std::fs::read_to_string(repo.join("f.txt")).unwrap(),
            "other\n"
        );
        std::fs::remove_dir_all(&repo).unwrap();
    }
}
