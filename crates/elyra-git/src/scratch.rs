//! A temporary checkout next to a repository, for running a command against
//! earlier states (a turn's checkpoint) without touching the working tree.
//! What git ignores at the top of the repository (dependencies such as
//! `vendor` and `node_modules`, `.env`) is linked in from the real folder, so
//! the project's checks can run there. Some are cloned instead (APFS, no
//! extra space until changed), because through a link the checks would run
//! the real folder's code rather than the checkout's:
//!
//! - Composer's `vendor`: PHP resolves symlinks, so a linked autoloader loads
//!   the app's classes from the real folder.
//! - `node_modules` in a JavaScript workspace (npm, Yarn or pnpm workspaces,
//!   Lerna): it links the workspace's own packages (`../../packages/ui`),
//!   which through a linked `node_modules` resolve to the real folder's.
//!
//! A Python virtualenv stays linked (a clone wouldn't help: an editable
//! install's `.pth` names the real folder); [`ScratchCheckout::env`] puts the
//! checkout's own code first on `PYTHONPATH` instead. Removed when dropped.

use crate::run_ok;
use anyhow::{Context as _, Result};
use std::path::{Path, PathBuf};

pub struct ScratchCheckout {
    repo: PathBuf,
    dir: PathBuf,
    /// The ignored entries linked in, kept through `git clean` (a link isn't
    /// a directory, so a `node_modules/` pattern doesn't keep it).
    linked: Vec<String>,
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
        let mut scratch = Self {
            repo: repo.to_path_buf(),
            dir,
            linked: Vec::new(),
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
        let mut clean = vec!["clean".to_string(), "--quiet".into(), "-fd".into()];
        for entry in &self.linked {
            clean.push("-e".into());
            clean.push(format!("/{entry}"));
        }
        let clean: Vec<&str> = clean.iter().map(String::as_str).collect();
        run_ok(&self.dir, &clean)?;
        Ok(())
    }

    /// Link what git ignores at the top of the repository into the checkout.
    fn link_ignored(&mut self) -> Result<()> {
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
            let own_code = match entry.as_str() {
                "vendor" => self.repo.join("composer.json").is_file(),
                "node_modules" => js_workspace(&self.repo),
                _ => false,
            };
            if !(own_code && clone(&from, &to)) {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&from, &to)
                    .with_context(|| format!("linking {entry} into the temporary checkout"))?;
            }
            self.linked.push(entry);
        }
        Ok(())
    }
}

impl ScratchCheckout {
    /// Environment for running the project's checks in the checkout: in a
    /// Python project, `PYTHONPATH` with the checkout's `src` (when it has one)
    /// and its top, so its code is imported rather than what an editable
    /// install in the linked virtualenv points to.
    pub fn env(&self) -> Vec<(String, String)> {
        let python = ["pyproject.toml", "setup.py", "setup.cfg"]
            .iter()
            .any(|file| self.dir.join(file).is_file());
        if !python {
            return Vec::new();
        }
        let mut paths = Vec::new();
        if self.dir.join("src").is_dir() {
            paths.push(self.dir.join("src").display().to_string());
        }
        paths.push(self.dir.display().to_string());
        if let Ok(existing) = std::env::var("PYTHONPATH")
            && !existing.is_empty()
        {
            paths.push(existing);
        }
        vec![("PYTHONPATH".into(), paths.join(":"))]
    }
}

/// Whether the repository is a JavaScript workspace, whose `node_modules`
/// links its own packages.
pub fn js_workspace(repo: &Path) -> bool {
    repo.join("pnpm-workspace.yaml").is_file()
        || repo.join("lerna.json").is_file()
        || std::fs::read_to_string(repo.join("package.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|package| package.get("workspaces").is_some())
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
        std::fs::write(repo.join(".gitignore"), "vendor/\n.env\n.venv/\n").unwrap();
        std::fs::write(repo.join("app.txt"), "one\n").unwrap();
        stage_all(&repo).unwrap();
        let first = commit_staged(&repo, "first").unwrap();
        std::fs::write(repo.join("app.txt"), "two\n").unwrap();
        stage_all(&repo).unwrap();
        commit_staged(&repo, "second").unwrap();
        std::fs::create_dir_all(repo.join("vendor/lib")).unwrap();
        std::fs::write(repo.join(".env"), "KEY=1\n").unwrap();
        std::fs::write(repo.join("composer.json"), "{}\n").unwrap();
        std::fs::create_dir_all(repo.join(".venv/bin")).unwrap();

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
            assert!(scratch.dir().join(".venv").is_symlink());
            scratch.checkout(&first).unwrap();
            assert_eq!(read("app.txt"), "one\n");
            assert_eq!(read(".env"), "KEY=1\n", "and stay");
            assert!(
                scratch.dir().join(".venv/bin").is_dir(),
                "a linked folder stays too, though `.venv/` only ignores folders"
            );
            scratch.dir().to_path_buf()
        };
        assert!(!dir.exists(), "removed when dropped");

        // A JavaScript workspace's node_modules is cloned (it links the
        // workspace's packages); a Python project gets its code first.
        std::fs::write(repo.join(".gitignore"), "vendor/\n.env\nnode_modules/\n").unwrap();
        std::fs::create_dir_all(repo.join("node_modules/@shop")).unwrap();
        std::fs::write(
            repo.join("package.json"),
            r#"{"workspaces": ["packages/*"]}"#,
        )
        .unwrap();
        std::fs::write(repo.join("pyproject.toml"), "[project]\nname = \"shop\"\n").unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/shop.py"), "\n").unwrap();
        stage_all(&repo).unwrap();
        commit_staged(&repo, "third").unwrap();
        {
            let scratch = ScratchCheckout::new(&repo).unwrap();
            assert!(scratch.dir().join("node_modules/@shop").is_dir());
            assert!(
                cfg!(not(target_os = "macos")) || !scratch.dir().join("node_modules").is_symlink(),
                "a workspace's node_modules is a clone"
            );
            let env = scratch.env();
            assert_eq!(env[0].0, "PYTHONPATH");
            assert!(
                env[0].1.starts_with(&format!(
                    "{}:{}",
                    scratch.dir().join("src").display(),
                    scratch.dir().display()
                )),
                "{}",
                env[0].1
            );
        }
        std::fs::write(repo.join("package.json"), "{}").unwrap();
        assert!(!super::js_workspace(&repo));
        std::fs::write(repo.join("pnpm-workspace.yaml"), "packages: []\n").unwrap();
        assert!(super::js_workspace(&repo));
        assert_eq!(
            std::fs::read_to_string(repo.join("app.txt")).unwrap(),
            "two\n",
            "the working tree is untouched"
        );
        std::fs::remove_dir_all(&repo).unwrap();
    }
}
