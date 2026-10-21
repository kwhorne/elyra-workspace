//! Taking back one change (from one snapshot to another) while keeping what
//! came after it: each file the change touched is moved back where it still
//! matches, and merged three ways where it was changed since; files that
//! can't be merged cleanly are left as they are and reported.

use crate::run_ok;
use anyhow::{Context as _, Result};
use std::path::Path;

/// What applying a change did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    /// Files now as the target has them (written, merged or removed).
    pub changed: Vec<String>,
    /// Files changed since in ways that don't merge: left as they are.
    pub kept: Vec<String>,
}

/// The files that differ between two snapshots (paths from the repository).
pub fn changed_paths(repo: &Path, from: &str, to: &str) -> Result<Vec<String>> {
    let out = run_ok(
        repo,
        &["diff", "--name-only", "--no-renames", "-z", from, to],
    )?;
    Ok(out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect())
}

/// A file's content in a snapshot; None when it isn't there.
pub fn content_at(repo: &Path, commit: &str, path: &str) -> Option<Vec<u8>> {
    let output = std::process::Command::new("git")
        .current_dir(repo)
        .args(["show", &format!("{commit}:{path}")])
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// Move `paths` of the working tree from how `base` has them toward how
/// `target` has them, keeping changes made since `base`. Undoing a turn is
/// `apply(repo, after_turn, before_turn, …)`.
pub fn apply(repo: &Path, base: &str, target: &str, paths: &[String]) -> Result<Applied> {
    let mut applied = Applied::default();
    for path in paths {
        let file = repo.join(path);
        let current = std::fs::read(&file).ok();
        let (was, wanted) = (content_at(repo, base, path), content_at(repo, target, path));
        if current == wanted {
            continue;
        }
        let result = if current == was {
            // Untouched since: take the target as it is.
            Some(wanted)
        } else {
            match (&current, &was, &wanted) {
                (Some(current), Some(was), Some(wanted)) => merge(current, was, wanted)?.map(Some),
                // Created or removed on one side and changed on the other.
                _ => None,
            }
        };
        match result {
            Some(Some(content)) => {
                if let Some(parent) = file.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&file, content).with_context(|| format!("writing {path}"))?;
                applied.changed.push(path.clone());
            }
            Some(None) => {
                std::fs::remove_file(&file).with_context(|| format!("removing {path}"))?;
                applied.changed.push(path.clone());
            }
            None => applied.kept.push(path.clone()),
        }
    }
    Ok(applied)
}

/// A three-way merge of text: `ours` and `theirs` both changed from `base`.
/// None when they conflict or a side isn't text.
fn merge(ours: &[u8], base: &[u8], theirs: &[u8]) -> Result<Option<Vec<u8>>> {
    if [ours, base, theirs].iter().any(|bytes| bytes.contains(&0)) {
        return Ok(None);
    }
    let dir = tempfile_dir()?;
    let write = |name: &str, bytes: &[u8]| -> Result<String> {
        let path = dir.join(name);
        std::fs::write(&path, bytes)?;
        Ok(path.display().to_string())
    };
    let (o, b, t) = (
        write("ours", ours)?,
        write("base", base)?,
        write("theirs", theirs)?,
    );
    let output = std::process::Command::new("git")
        .args(["merge-file", "-p", "--quiet", &o, &b, &t])
        .output()?;
    let _ = std::fs::remove_dir_all(&dir);
    // Exit 0: merged cleanly; above 0: that many conflicts.
    Ok((output.status.code() == Some(0)).then_some(output.stdout))
}

fn tempfile_dir() -> Result<std::path::PathBuf> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "elyra-merge-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::{apply, changed_paths};
    use crate::checkpoint::snapshot;
    use crate::tests::temp_repo;
    use crate::{commit_staged, stage_all};

    #[test]
    fn takes_back_one_change_and_keeps_the_rest() {
        let repo = temp_repo("undo");
        let write = |name: &str, text: &str| std::fs::write(repo.join(name), text).unwrap();
        let read = |name: &str| std::fs::read_to_string(repo.join(name)).ok();
        let lines: String = (1..=12).map(|n| format!("line {n}\n")).collect();
        write("lib.rs", &lines);
        write("keep.rs", "fn keep() {}\n");
        stage_all(&repo).unwrap();
        commit_staged(&repo, "base").unwrap();

        let before = snapshot(&repo, "before").unwrap();
        // The change to take back: an edit at the top of lib.rs, a new file,
        // a deleted file, and an edit to keep.rs.
        write("lib.rs", &lines.replacen("line 1\n", "line 1 changed\n", 1));
        write("new.rs", "fn new() {}\n");
        std::fs::remove_file(repo.join("keep.rs")).unwrap();
        let after = snapshot(&repo, "after").unwrap();
        // Later work: lib.rs changed far below, and keep.rs back with other text.
        write(
            "lib.rs",
            &read("lib.rs")
                .unwrap()
                .replacen("line 12\n", "line 12 later\n", 1),
        );
        write("keep.rs", "fn other() {}\n");

        let mut paths = changed_paths(&repo, &before, &after).unwrap();
        paths.sort();
        assert_eq!(paths, ["keep.rs", "lib.rs", "new.rs"]);
        let applied = apply(&repo, &after, &before, &paths).unwrap();
        assert_eq!(applied.changed, ["lib.rs", "new.rs"]);
        assert_eq!(
            applied.kept,
            ["keep.rs"],
            "removed, then made again: left as it is"
        );
        let lib = read("lib.rs").unwrap();
        assert!(lib.starts_with("line 1\n"), "the change is gone: {lib}");
        assert!(
            lib.ends_with("line 12 later\n"),
            "the later work stays: {lib}"
        );
        assert_eq!(read("new.rs"), None, "the file it made is gone");
        assert_eq!(read("keep.rs").as_deref(), Some("fn other() {}\n"));
        std::fs::remove_dir_all(&repo).unwrap();
    }
}
