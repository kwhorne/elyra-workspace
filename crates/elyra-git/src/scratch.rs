//! A temporary checkout next to a repository, for running a command against
//! earlier states (a turn's checkpoint) without touching the working tree.
//! What git ignores at the top of the repository (dependencies such as
//! `vendor` and `node_modules`, `.env`) is linked in from the real folder, so
//! the project's checks can run there. Composer's `vendor` is cloned instead
//! (APFS, no extra space): PHP resolves symlinks, so a linked autoloader would
//! load the app's classes from the real folder, not the checkout. Removed
//! when dropped.

use crate::run_ok;
use anyhow::{Context as _, Result};
use std::path::{Path, PathBuf};

pub struct ScratchCheckout {
    repo: PathBuf,
    dir: PathBuf,
}

impl ScratchCheckout {
    pub fn new(repo: &Path) -> Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "elyra-scratch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let path = dir.display().to_string();
        run_ok(
            repo,
            &["worktree", "add", "--detach", "--quiet", &path, "HEAD"],
        )
        .context("making a temporary checkout")?;
        let scratch = Self {
            repo: repo.to_path_buf(),
            dir,
        };
        scratch.link_ignored()?;
        Ok(scratch)
    }

    /// The checkout's folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Show `commit` in the checkout. The linked files stay: git ignores them.
    pub fn checkout(&self, commit: &str) -> Result<()> {
        run_ok(
            &self.dir,
            &["checkout", "--quiet", "--force", "--detach", commit],
        )?;
        run_ok(&self.dir, &["clean", "--quiet", "-fd"])?;
        Ok(())
    }

    /// Link what git ignores at the top of the repository into the checkout.
    fn link_ignored(&self) -> Result<()> {
        let ignored = run_ok(
            &self.repo,
            &[
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "--directory",
            ],
        )?;
        for entry in top_level(&ignored) {
            let (from, to) = (self.repo.join(&entry), self.dir.join(&entry));
            if to.exists() || entry == ".git" {
                continue;
            }
            if entry == "vendor" && self.repo.join("composer.json").is_file() && clone(&from, &to) {
                continue;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&from, &to)
                .with_context(|| format!("linking {entry} into the temporary checkout"))?;
        }
        Ok(())
    }
}

impl Drop for ScratchCheckout {
    fn drop(&mut self) {
        let path = self.dir.display().to_string();
        if run_ok(&self.repo, &["worktree", "remove", "--force", &path]).is_err() {
            let _ = std::fs::remove_dir_all(&self.dir);
            let _ = run_ok(&self.repo, &["worktree", "prune"]);
        }
    }
}

/// A copy-on-write clone of a folder (APFS); false where that isn't possible.
fn clone(from: &Path, to: &Path) -> bool {
    cfg!(target_os = "macos")
        && std::process::Command::new("cp")
            .arg("-cRp")
            .arg(from)
            .arg(to)
            .status()
            .is_ok_and(|s| s.success())
}

/// The ignored entries at the top of the repository, from `git ls-files
/// --others --ignored --directory` (nested ones stay out).
fn top_level(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(|line| line.trim_end_matches('/'))
        .filter(|entry| !entry.is_empty() && !entry.contains('/'))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ScratchCheckout, top_level};
    use crate::tests::temp_repo;
    use crate::{commit_staged, stage_all};

    #[test]
    fn checks_out_earlier_states_beside_the_repository() {
        assert_eq!(
            top_level("vendor/\n.env\nstorage/logs/\nnode_modules/\n"),
            ["vendor", ".env", "node_modules"]
        );
        let repo = temp_repo("scratch");
        std::fs::write(repo.join(".gitignore"), "vendor/\n.env\n").unwrap();
        std::fs::write(repo.join("app.txt"), "one\n").unwrap();
        stage_all(&repo).unwrap();
        let first = commit_staged(&repo, "first").unwrap();
        std::fs::write(repo.join("app.txt"), "two\n").unwrap();
        stage_all(&repo).unwrap();
        commit_staged(&repo, "second").unwrap();
        std::fs::create_dir_all(repo.join("vendor/lib")).unwrap();
        std::fs::write(repo.join(".env"), "KEY=1\n").unwrap();
        std::fs::write(repo.join("composer.json"), "{}\n").unwrap();

        let dir = {
            let scratch = ScratchCheckout::new(&repo).unwrap();
            let read = |name: &str| std::fs::read_to_string(scratch.dir().join(name)).unwrap();
            assert_eq!(read("app.txt"), "two\n");
            assert_eq!(read(".env"), "KEY=1\n", "ignored files are linked in");
            assert!(scratch.dir().join("vendor/lib").is_dir());
            assert!(
                cfg!(not(target_os = "macos")) || !scratch.dir().join("vendor").is_symlink(),
                "Composer's vendor is a clone, so PHP loads the checkout's classes"
            );
            assert!(scratch.dir().join(".env").is_symlink());
            scratch.checkout(&first).unwrap();
            assert_eq!(read("app.txt"), "one\n");
            assert_eq!(read(".env"), "KEY=1\n", "and stay");
            scratch.dir().to_path_buf()
        };
        assert!(!dir.exists(), "removed when dropped");
        assert_eq!(
            std::fs::read_to_string(repo.join("app.txt")).unwrap(),
            "two\n",
            "the working tree is untouched"
        );
        std::fs::remove_dir_all(&repo).unwrap();
    }
}
