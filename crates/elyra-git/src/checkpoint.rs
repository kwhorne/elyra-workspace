//! Turn checkpoints: snapshots of the whole working tree (tracked and
//! untracked, respecting .gitignore) stored as commits under a hidden ref,
//! without touching the user's index, stash or branches.

use crate::{git, run, run_ok};
use anyhow::{Context as _, Result, bail};
use std::path::{Path, PathBuf};

const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "Elyra Workspace"),
    ("GIT_AUTHOR_EMAIL", "checkpoint@elyra.local"),
    ("GIT_COMMITTER_NAME", "Elyra Workspace"),
    ("GIT_COMMITTER_EMAIL", "checkpoint@elyra.local"),
];

fn git_path(repo: &Path, name: &str) -> Result<PathBuf> {
    let path = PathBuf::from(run_ok(repo, &["rev-parse", "--git-path", name])?.trim());
    Ok(if path.is_absolute() {
        path
    } else {
        repo.join(path)
    })
}

/// Commit the current working tree to a dangling commit and return its SHA.
pub fn snapshot(repo: &Path, message: &str) -> Result<String> {
    with_working_tree(repo, |with_index, tree| {
        let has_head = run(repo, &["rev-parse", "--verify", "HEAD"])?
            .status
            .success();
        let mut args = vec!["commit-tree", tree, "-m", message];
        if has_head {
            args.extend(["-p", "HEAD"]);
        }
        with_index(&args)
    })
}

/// Whether the working tree differs from a snapshot (`commit`): files
/// changed, added or removed since.
pub fn changed_since(repo: &Path, commit: &str) -> Result<bool> {
    let now = with_working_tree(repo, |_, tree| Ok(tree.to_string()))?;
    let then = run_ok(repo, &["rev-parse", &format!("{commit}^{{tree}}")])?;
    Ok(now != then.trim())
}

/// Write the working tree (tracked and untracked, respecting .gitignore) as a
/// tree object through a temporary index, and hand its SHA to `finish`.
fn with_working_tree(
    repo: &Path,
    finish: impl FnOnce(&dyn Fn(&[&str]) -> Result<String>, &str) -> Result<String>,
) -> Result<String> {
    let index = git_path(repo, "index")?;
    let temp = std::env::temp_dir().join(format!(
        "elyra-checkpoint-{}-{}.index",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    // Start from the real index so unchanged files are not re-hashed.
    if index.exists() {
        std::fs::copy(&index, &temp).context("copying the index")?;
    }
    let with_index = |args: &[&str]| -> Result<String> {
        let output = git(repo)
            .args(args)
            .env("GIT_INDEX_FILE", &temp)
            .envs(IDENTITY)
            .output()?;
        if !output.status.success() {
            bail!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    let result = (|| {
        with_index(&["add", "--all"])?;
        let tree = with_index(&["write-tree"])?;
        finish(&with_index, &tree)
    })();
    let _ = std::fs::remove_file(&temp);
    result
}

/// Snapshot the working tree and keep it under `refname`.
pub fn create(repo: &Path, refname: &str, message: &str) -> Result<String> {
    let commit = snapshot(repo, message)?;
    run_ok(repo, &["update-ref", refname, &commit])?;
    Ok(commit)
}

/// Restore the working tree to a snapshot: modified and deleted files come
/// back, files created since are removed. The index is reset to HEAD.
pub fn restore(repo: &Path, commit: &str) -> Result<()> {
    let current = snapshot(repo, "elyra: before restore")?;
    let added = run_ok(
        repo,
        &[
            "diff",
            "--name-only",
            "-z",
            "--diff-filter=A",
            commit,
            &current,
        ],
    )?;
    for path in added.split('\0').filter(|p| !p.is_empty()) {
        let file = repo.join(path);
        if file.is_file() {
            std::fs::remove_file(&file).with_context(|| format!("removing {path}"))?;
        }
    }
    run_ok(
        repo,
        &["restore", "--source", commit, "--worktree", "--", "."],
    )?;
    if run(repo, &["rev-parse", "--verify", "HEAD"])?
        .status
        .success()
    {
        run_ok(repo, &["reset", "--quiet"])?;
    }
    Ok(())
}

/// Remove every ref under a prefix (e.g. a deleted thread's checkpoints).
pub fn delete_refs(repo: &Path, prefix: &str) -> Result<()> {
    let refs = run_ok(repo, &["for-each-ref", "--format=%(refname)", prefix])?;
    for name in refs.lines().filter(|l| !l.is_empty()) {
        run_ok(repo, &["update-ref", "-d", name])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::temp_repo;
    use crate::{DiffSource, changed_files, commit_staged, stage_all};

    #[test]
    fn checkpoint_and_restore_working_tree() {
        let repo = temp_repo("checkpoint");
        std::fs::write(repo.join("keep.txt"), "v1\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "ignored.log\n").unwrap();
        std::fs::write(repo.join("gone.txt"), "tracked\n").unwrap();
        stage_all(&repo).unwrap();
        commit_staged(&repo, "base").unwrap();
        std::fs::write(repo.join("keep.txt"), "v2 uncommitted\n").unwrap();
        std::fs::write(repo.join("ignored.log"), "noise").unwrap();

        let before = create(&repo, "refs/elyra/test/1", "turn 1").unwrap();
        let status_before = run_ok(&repo, &["status", "--porcelain"]).unwrap();
        assert!(!changed_since(&repo, &before).unwrap());
        std::fs::write(repo.join("ignored.log"), "more noise").unwrap();
        assert!(
            !changed_since(&repo, &before).unwrap(),
            "ignored files don't count"
        );

        // The "agent" edits, deletes and creates files.
        std::fs::write(repo.join("keep.txt"), "agent edit\n").unwrap();
        std::fs::write(repo.join("new.rs"), "fn main() {}\n").unwrap();
        std::fs::remove_file(repo.join("gone.txt")).unwrap();

        assert!(changed_since(&repo, &before).unwrap());
        let after = snapshot(&repo, "now").unwrap();
        let files: Vec<_> = changed_files(&repo, &DiffSource::Range(before.clone(), after))
            .unwrap()
            .into_iter()
            .map(|c| c.path)
            .collect();
        assert_eq!(files.len(), 3, "{files:?}");

        restore(&repo, &before).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("keep.txt")).unwrap(),
            "v2 uncommitted\n"
        );
        assert!(!repo.join("new.rs").exists());
        assert!(repo.join("gone.txt").exists());
        assert!(
            repo.join("ignored.log").exists(),
            "ignored files are left alone"
        );
        assert_eq!(
            run_ok(&repo, &["status", "--porcelain"]).unwrap(),
            status_before
        );

        delete_refs(&repo, "refs/elyra/test").unwrap();
        assert!(
            run_ok(&repo, &["for-each-ref", "refs/elyra"])
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(&repo).unwrap();
    }
}
