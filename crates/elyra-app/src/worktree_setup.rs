//! What a new worktree needs from the project folder that Git doesn't bring:
//! files most projects keep out of the repository. The app's `.env`, the
//! agents' setup that Laravel Boost and others write (`.mcp.json`, the
//! guideline files, skills) and `vendor/`, so `php artisan` — and with it
//! Boost's MCP server — works there. Only what the worktree lacks is copied;
//! `vendor/` only as an APFS clone (cheap, no extra space until changed).

use std::path::Path;
use std::process::Command;

/// Copied when the project has them and the worktree doesn't.
const LOCAL: &[&str] = &[
    ".env",
    ".mcp.json",
    "boost.json",
    "AGENTS.md",
    "CLAUDE.md",
    "GEMINI.md",
    ".ai",
    ".claude/skills",
    ".agents/skills",
    ".gemini/skills",
    ".cursor/rules",
    ".junie",
];

/// Copy what the worktree at `worktree` lacks from `project`; returns what
/// was copied, and a note when `vendor/` couldn't be. Blocking.
pub fn prepare(project: &Path, worktree: &Path) -> (Vec<String>, Option<String>) {
    let mut copied = Vec::new();
    for name in LOCAL {
        let (from, to) = (project.join(name), worktree.join(name));
        if !from.exists() || to.exists() {
            continue;
        }
        if let Some(parent) = to.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            continue;
        }
        if copy(&from, &to, false) {
            copied.push((*name).to_string());
        }
    }
    let mut note = None;
    let (vendor, into) = (project.join("vendor"), worktree.join("vendor"));
    if vendor.is_dir() && !into.exists() && project.join("composer.json").is_file() {
        if copy(&vendor, &into, true) {
            copied.push("vendor".into());
        } else {
            note = Some(
                "vendor/ wasn't copied (cloning needs APFS): run composer install in the worktree."
                    .into(),
            );
        }
    }
    (copied, note)
}

/// `cp -R`, as a clone where the file system can (`clone_only`: or not at
/// all).
fn copy(from: &Path, to: &Path, clone_only: bool) -> bool {
    let clone = Command::new("cp")
        .arg(if cfg!(target_os = "macos") {
            "-cRp"
        } else {
            "-Rp"
        })
        .args(if cfg!(target_os = "macos") {
            &[][..]
        } else {
            &["--reflink=always"][..]
        })
        .arg(from)
        .arg(to)
        .status()
        .is_ok_and(|s| s.success());
    if clone || clone_only {
        if !clone {
            let _ = std::fs::remove_dir_all(to);
        }
        return clone;
    }
    Command::new("cp")
        .arg("-Rp")
        .arg(from)
        .arg(to)
        .status()
        .is_ok_and(|s| s.success())
}

/// In words, for the thread.
pub fn describe(copied: &[String]) -> Option<String> {
    (!copied.is_empty()).then(|| {
        format!(
            "Copied into the worktree from the project folder (not in Git): {}.",
            copied.join(", ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{describe, prepare};

    #[test]
    fn brings_what_git_leaves_out() {
        let root =
            std::env::temp_dir().join(format!("elyra-worktree-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (project, worktree) = (root.join("project"), root.join("worktree"));
        std::fs::create_dir_all(project.join(".claude/skills/pest-testing")).unwrap();
        std::fs::create_dir_all(project.join("vendor/laravel")).unwrap();
        std::fs::create_dir_all(worktree.join(".claude")).unwrap();
        std::fs::write(project.join(".env"), "APP_KEY=x\n").unwrap();
        std::fs::write(project.join("composer.json"), "{}").unwrap();
        std::fs::write(
            project.join(".claude/skills/pest-testing/SKILL.md"),
            "---\n",
        )
        .unwrap();
        std::fs::write(project.join("vendor/autoload.php"), "<?php\n").unwrap();
        std::fs::write(project.join("AGENTS.md"), "project's\n").unwrap();
        std::fs::write(worktree.join("AGENTS.md"), "the branch's own\n").unwrap();

        let (copied, _) = prepare(&project, &worktree);
        assert!(copied.contains(&".env".to_string()));
        assert!(copied.contains(&".claude/skills".to_string()));
        assert!(
            !copied.contains(&"AGENTS.md".to_string()),
            "tracked: Git brought it"
        );
        assert_eq!(
            std::fs::read_to_string(worktree.join("AGENTS.md")).unwrap(),
            "the branch's own\n"
        );
        assert!(
            worktree
                .join(".claude/skills/pest-testing/SKILL.md")
                .is_file()
        );
        if copied.contains(&"vendor".to_string()) {
            assert!(worktree.join("vendor/autoload.php").is_file());
        }
        assert!(describe(&copied).unwrap().contains(".env"));
        assert_eq!(prepare(&project, &worktree).0, Vec::<String>::new(), "once");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
