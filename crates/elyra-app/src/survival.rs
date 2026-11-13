//! Code survival: of the lines an agent's turns wrote, how many were still in
//! the code once the work had settled, not rewritten in a follow-up, by hand
//! or by another instruction, and not undone. Checks green on the first try
//! say whether the code worked; this says whether you kept it.
//!
//! Each turn is measured once, [`SETTLE_DAYS`] after it, and the number is
//! kept (later changes don't move it); turns older than [`MAX_AGE_DAYS`] aren't
//! measured. A line counts as kept when the same line is in the same file in
//! the project's folder, the thread's worktree, or the remote's default branch
//! (work merged on the server). Best of N candidates are left out: all but one
//! are thrown away by design.

use crate::app_state::AppState;
use chrono::{DateTime, Utc};
use elyra_core::{ItemId, ProjectId, Store, ThreadId};
use gpui_kit::{App, Entity};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A turn is measured this many days after it.
pub const SETTLE_DAYS: i64 = 3;
/// Turns older than this aren't measured (from before measuring started).
pub const MAX_AGE_DAYS: i64 = 30;
/// Fewer measured lines than this, and the share kept isn't compared.
pub const MIN_LINES: usize = 30;

/// How long after launch, and then how often, turns are measured
/// (`ELYRA_SURVIVAL_POLL_SECS` sets both, for the scenarios).
fn delays() -> (Duration, Duration) {
    match std::env::var("ELYRA_SURVIVAL_POLL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        Some(secs) => (Duration::from_secs(secs), Duration::from_secs(secs)),
        None => (Duration::from_secs(90), Duration::from_secs(6 * 3600)),
    }
}

/// Measured turns: (thread, user message) → (lines written, lines kept).
pub type Survival = HashMap<(ThreadId, ItemId), (usize, usize)>;

/// Measure settled turns a while after launch, and every few hours.
pub fn init(app: Entity<AppState>, cx: &mut App) {
    let (first, every) = delays();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(first).await;
        loop {
            let jobs = cx.update(|cx| (!crate::quitting::draining(cx)).then(|| jobs(app.read(cx))));
            if let Some(jobs) = jobs {
                let now = Utc::now();
                let measured = cx
                    .background_executor()
                    .spawn(async move { measure(&elyra_core::paths::database_path(), jobs, now) })
                    .await;
                match measured {
                    Ok(rows) if !rows.is_empty() => {
                        cx.update(|cx| {
                            let store = &app.read(cx).store;
                            for (thread, item, lines, kept) in &rows {
                                if let Err(err) =
                                    store.record_line_survival(*thread, *item, *lines, *kept, now)
                                {
                                    log::warn!("recording code survival: {err:#}");
                                }
                            }
                        });
                        log::info!("code survival: measured {} turns", rows.len());
                    }
                    Ok(_) => {}
                    Err(err) => log::warn!("measuring code survival: {err:#}"),
                }
            }
            cx.background_executor().timer(every).await;
        }
    })
    .detach();
}

/// One project to measure.
pub struct Job {
    pub project: ProjectId,
    pub dir: PathBuf,
    /// Threads that work in a worktree of their own.
    pub worktrees: HashMap<ThreadId, PathBuf>,
    /// Threads not to measure (Best of N candidates).
    pub skip: HashSet<ThreadId>,
}

fn jobs(app: &AppState) -> Vec<Job> {
    let threads: Vec<elyra_core::Thread> = app
        .threads
        .iter()
        .cloned()
        .chain(app.archived_threads())
        .collect();
    app.projects
        .iter()
        .map(|project| {
            let mine = threads.iter().filter(|t| t.project_id == project.id);
            Job {
                project: project.id,
                dir: project.path.clone(),
                worktrees: mine
                    .clone()
                    .filter_map(|t| match &t.environment {
                        elyra_core::Environment::Worktree { path, .. } => {
                            Some((t.id, path.clone()))
                        }
                        elyra_core::Environment::Local => None,
                    })
                    .collect(),
                skip: mine.filter(|t| t.race_id.is_some()).map(|t| t.id).collect(),
            }
        })
        .collect()
}

/// Measure the turns that have settled since the last time: (thread, user
/// message, lines, kept). Blocking.
pub fn measure(
    db: &Path,
    jobs: Vec<Job>,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<(ThreadId, ItemId, usize, usize)>> {
    let store = Store::open_read_only(db)?;
    let mut out = Vec::new();
    for job in jobs {
        let pending = store.lines_to_measure(
            job.project,
            now - chrono::Duration::days(MAX_AGE_DAYS),
            now - chrono::Duration::days(SETTLE_DAYS),
        )?;
        if pending.is_empty() {
            continue;
        }
        let Ok(root) = elyra_git::repo_root(&job.dir) else {
            continue;
        };
        let mut files = Files::new(root);
        for ((thread, item), lines) in pending {
            if job.skip.contains(&thread) {
                continue;
            }
            let worktree = job
                .worktrees
                .get(&thread)
                .filter(|path| path.is_dir())
                .and_then(|path| elyra_git::repo_root(path).ok());
            let kept = kept(&lines, |path| files.lines(worktree.as_deref(), path));
            out.push((thread, item, lines.len(), kept));
        }
    }
    Ok(out)
}

/// How many of `lines` (path, hash) are there: a line written twice needs
/// to be there twice.
pub fn kept(
    lines: &[(String, i64)],
    mut present: impl FnMut(&str) -> HashMap<i64, usize>,
) -> usize {
    let mut wanted: HashMap<&str, HashMap<i64, usize>> = HashMap::new();
    for (path, hash) in lines {
        *wanted.entry(path).or_default().entry(*hash).or_default() += 1;
    }
    wanted
        .into_iter()
        .map(|(path, hashes)| {
            let have = present(path);
            hashes
                .into_iter()
                .map(|(hash, count)| count.min(have.get(&hash).copied().unwrap_or(0)))
                .sum::<usize>()
        })
        .sum()
}

/// How often each line (by [`elyra_core::line_hash`]) is in a text.
pub fn line_counts(text: &str) -> HashMap<i64, usize> {
    let mut counts = HashMap::new();
    for hash in text.lines().filter_map(elyra_core::line_hash) {
        *counts.entry(hash).or_default() += 1;
    }
    counts
}

/// A project's files where they may be: its folder, a worktree, the remote's
/// default branch. Read once each.
struct Files {
    root: PathBuf,
    remote: Option<String>,
    cache: HashMap<(Option<PathBuf>, String), HashMap<i64, usize>>,
}

impl Files {
    fn new(root: PathBuf) -> Self {
        let remote = elyra_git::remote_default(&root);
        Self {
            root,
            remote,
            cache: HashMap::new(),
        }
    }

    /// The lines of `path` wherever it is most complete.
    fn lines(&mut self, worktree: Option<&Path>, path: &str) -> HashMap<i64, usize> {
        let mut all = self.source(None, path);
        if let Some(worktree) = worktree.filter(|w| *w != self.root) {
            for (hash, count) in self.source(Some(worktree.to_path_buf()), path) {
                let slot = all.entry(hash).or_default();
                *slot = (*slot).max(count);
            }
        }
        all
    }

    /// The folder (None: the project's, with the remote's default branch).
    fn source(&mut self, folder: Option<PathBuf>, path: &str) -> HashMap<i64, usize> {
        let key = (folder.clone(), path.to_string());
        if let Some(counts) = self.cache.get(&key) {
            return counts.clone();
        }
        let read = |dir: &Path| {
            std::fs::read(dir.join(path))
                .map(|bytes| line_counts(&String::from_utf8_lossy(&bytes)))
                .unwrap_or_default()
        };
        let counts = match &folder {
            Some(dir) => read(dir),
            None => {
                let mut counts = read(&self.root);
                if let Some(remote) = &self.remote
                    && let Some(bytes) = elyra_git::undo::content_at(&self.root, remote, path)
                {
                    for (hash, count) in line_counts(&String::from_utf8_lossy(&bytes)) {
                        let slot = counts.entry(hash).or_default();
                        *slot = (*slot).max(count);
                    }
                }
                counts
            }
        };
        self.cache.insert(key, counts.clone());
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::{Job, kept, line_counts, measure};
    use elyra_core::{Store, line_hash, new_id};
    use std::collections::{HashMap, HashSet};
    use std::process::Command;

    fn line(path: &str, text: &str) -> (String, i64) {
        (path.to_string(), line_hash(text).unwrap())
    }

    #[test]
    fn counts_the_lines_still_there() {
        let file = "fn total() -> u32 {\n    price + tax\n}\n    price + tax\n";
        let lines = [
            line("src/lib.rs", "fn total() -> u32 {"),
            line("src/lib.rs", "    price + tax"),
            line("src/lib.rs", "price + tax"),
            line("src/lib.rs", "price + tax"),
            line("src/lib.rs", "    let rounded = total.round();"),
            line("src/gone.rs", "fn total() -> u32 {"),
        ];
        let kept = kept(&lines, |path| {
            if path == "src/lib.rs" {
                line_counts(file)
            } else {
                HashMap::new()
            }
        });
        // The function line, and "price + tax" as often as the file has it.
        assert_eq!(kept, 3);
    }

    #[test]
    fn measures_settled_turns_against_the_folder() {
        let dir = std::env::temp_dir().join(format!("elyra-survival-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = dir.as_path();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .current_dir(repo)
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success()
            )
        };
        git(&["init", "-q"]);
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(
            repo.join("src/lib.rs"),
            "fn total() -> u32 {\n    price + tax\n}\n",
        )
        .unwrap();
        let db = repo.join("state.db");
        let store = Store::open(&db).unwrap();
        let (project, thread, raced) = (new_id(), new_id(), new_id());
        let (settled, fresh, race) = (new_id(), new_id(), new_id());
        let now = chrono::Utc::now();
        let written = [
            line("src/lib.rs", "fn total() -> u32 {"),
            line("src/lib.rs", "    price + tax"),
            line("src/lib.rs", "    price * 2"),
            line("src/lib.rs", "    let unused = 1;"),
        ];
        for (thread, item, days) in [(thread, settled, 4), (thread, fresh, 1), (raced, race, 4)] {
            store
                .record_line_origins(
                    project,
                    thread,
                    item,
                    now - chrono::Duration::days(days),
                    &written,
                )
                .unwrap();
        }
        let job = Job {
            project,
            dir: repo.to_path_buf(),
            worktrees: HashMap::new(),
            skip: HashSet::from([raced]),
        };
        let measured = measure(&db, vec![job], now).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(measured, vec![(thread, settled, 4, 2)]);
    }
}
