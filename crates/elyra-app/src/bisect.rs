//! "Who broke it?": the instruction whose turn turned the checks from green to
//! red. Every turn has a checkpoint of the folder taken before it, also across
//! threads and agents working in the same folder; with the state now, that's
//! a line of states. The checks run against them in a temporary checkout (the
//! working tree isn't touched): the latest and the oldest first, then halving
//! the range until the step from passing to failing is found.

use crate::checks;
use elyra_core::{BisectVerdict, ItemContent, ItemId, Store, ThreadId};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// At most this many turns back.
const MAX_STEPS: usize = 40;

/// The state before a turn.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub checkpoint: String,
    pub thread: ThreadId,
    pub item: ItemId,
    pub instruction: String,
    pub thread_title: String,
    pub agent: String,
    pub at: chrono::DateTime<chrono::Utc>,
}

/// The turns in a folder, oldest first: `threads` are the threads working
/// there (id, title, agent). Instructions only; automatic fixes are part of
/// the turn they fix.
pub fn timeline(
    store: &Store,
    threads: &[(ThreadId, String, String)],
) -> anyhow::Result<Vec<Step>> {
    let mut steps = Vec::new();
    for (thread, title, agent) in threads {
        for item in store.transcript(*thread)? {
            if let ItemContent::User {
                text,
                checkpoint: Some(checkpoint),
                ..
            } = &item.content
                && checks::fix_prompt_label(text).is_none()
            {
                steps.push(Step {
                    checkpoint: checkpoint.clone(),
                    thread: *thread,
                    item: item.id,
                    instruction: text.trim().to_string(),
                    thread_title: title.clone(),
                    agent: agent.clone(),
                    at: item.created_at,
                });
            }
        }
    }
    steps.sort_by_key(|step| step.at);
    let keep = steps.len().saturating_sub(MAX_STEPS);
    steps.drain(..keep);
    Ok(steps)
}

/// The first state that fails, given a check that passes at `good` and fails
/// at `bad` (indexes into the states, `good < bad`), testing as few as it can.
pub fn first_failing(
    mut good: usize,
    mut bad: usize,
    mut passes: impl FnMut(usize) -> Option<bool>,
) -> Option<usize> {
    while bad - good > 1 {
        let mid = good + (bad - good) / 2;
        if passes(mid)? {
            good = mid;
        } else {
            bad = mid;
        }
    }
    Some(bad)
}

/// Run `command` (in `dir`, a folder of the repository at `root`) against
/// the states before each of `steps` and now, and say which turn broke it.
/// `progress` hears what is being tried. Blocking, and long: run it in the
/// background; `cancel` stops it.
pub fn find(
    root: &Path,
    dir: &Path,
    command: &str,
    steps: &[Step],
    cancel: &Arc<AtomicBool>,
    progress: impl Fn(String),
) -> BisectVerdict {
    match search(root, dir, command, steps, cancel, &progress) {
        Ok(verdict) => verdict,
        Err(err) => BisectVerdict::Failed {
            error: format!("{err:#}"),
        },
    }
}

fn search(
    root: &Path,
    dir: &Path,
    command: &str,
    steps: &[Step],
    cancel: &Arc<AtomicBool>,
    progress: &impl Fn(String),
) -> anyhow::Result<BisectVerdict> {
    if steps.is_empty() {
        return Ok(BisectVerdict::NoHistory);
    }
    let now = elyra_git::checkpoint::snapshot(root, "elyra: who broke it")?;
    // States: before each turn, then now.
    let states: Vec<&str> = steps
        .iter()
        .map(|s| s.checkpoint.as_str())
        .chain(std::iter::once(now.as_str()))
        .collect();
    let scratch = elyra_git::scratch::ScratchCheckout::new(root)?;
    let folder = scratch
        .dir()
        .join(dir.strip_prefix(root).unwrap_or(Path::new("")));
    let mut runner = Runner {
        scratch: &scratch,
        folder: &folder,
        command,
        states: &states,
        cancel,
        progress,
        runs: 0,
        outputs: vec![None; states.len()],
    };
    let last = states.len() - 1;
    let Some(now_passes) = runner.run(last, "on the current state".into())? else {
        anyhow::bail!("stopped");
    };
    if now_passes {
        return Ok(BisectVerdict::Passes { runs: runner.runs });
    }
    let Some(oldest_passes) = runner.run(0, format!("before {}", headline(&steps[0])))? else {
        anyhow::bail!("stopped");
    };
    if !oldest_passes {
        return Ok(BisectVerdict::AlreadyFailing {
            since: steps[0].at,
            runs: runner.runs,
        });
    }
    let mut failed = None;
    let first_bad = first_failing(0, last, |index| {
        match runner.run(index, format!("before {}", headline(&steps[index]))) {
            Ok(result) => result,
            Err(err) => {
                failed = Some(err);
                None
            }
        }
    });
    if let Some(err) = failed {
        return Err(err);
    }
    let Some(first_bad) = first_bad else {
        anyhow::bail!("stopped");
    };
    // The turn between the last passing state and the first failing one.
    let culprit = &steps[first_bad - 1];
    Ok(BisectVerdict::Found {
        thread: culprit.thread,
        item: culprit.item,
        instruction: culprit.instruction.clone(),
        thread_title: culprit.thread_title.clone(),
        agent: culprit.agent.clone(),
        at: culprit.at,
        output: runner.outputs[first_bad].clone().unwrap_or_default(),
        runs: runner.runs,
    })
}

/// Runs the checks against one state at a time in the temporary checkout,
/// keeping count and each run's output.
struct Runner<'a, P: Fn(String)> {
    scratch: &'a elyra_git::scratch::ScratchCheckout,
    folder: &'a Path,
    command: &'a str,
    states: &'a [&'a str],
    cancel: &'a Arc<AtomicBool>,
    progress: &'a P,
    runs: usize,
    outputs: Vec<Option<String>>,
}

impl<P: Fn(String)> Runner<'_, P> {
    /// Whether the checks pass at state `index`; None when stopped.
    fn run(&mut self, index: usize, what: String) -> anyhow::Result<Option<bool>> {
        let stopped = || self.cancel.load(std::sync::atomic::Ordering::Relaxed);
        if stopped() {
            return Ok(None);
        }
        self.runs += 1;
        (self.progress)(format!("Running the checks {what} (run {})", self.runs));
        self.scratch.checkout(self.states[index])?;
        let outcome = checks::run(self.folder, self.command, self.cancel);
        if stopped() {
            return Ok(None);
        }
        self.outputs[index] = Some(outcome.output);
        Ok(Some(outcome.passed))
    }
}

/// An instruction's first line, quoted and short.
fn headline(step: &Step) -> String {
    let line = step.instruction.lines().next().unwrap_or("").trim();
    let short: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        format!("“{short}…”")
    } else {
        format!("“{short}”")
    }
}

/// The threads working in `dir` (id, title, agent), archived ones too.
pub fn threads_in(app: &crate::app_state::AppState, dir: &Path) -> Vec<(ThreadId, String, String)> {
    app.threads
        .iter()
        .chain(app.archived_threads().iter())
        .filter_map(|thread| {
            let project = app.project(thread.project_id)?;
            (thread.working_dir(project) == dir).then(|| {
                (
                    thread.id,
                    thread.title.clone(),
                    thread.provider.label().to_string(),
                )
            })
        })
        .collect()
}

/// A verdict in words, for an agent (the gateway) or a notice.
pub fn describe(command: &str, verdict: &BisectVerdict) -> String {
    match verdict {
        BisectVerdict::Found {
            thread,
            instruction,
            thread_title,
            agent,
            at,
            output,
            runs,
            ..
        } => format!(
            "`{command}` started failing with the turn for this message in “{thread_title}” ({agent}, {}, thread_id {thread}):\n  “{}”\nIt passed before that turn and failed after it ({runs} check runs). The failing output ends with:\n{}",
            at.format("%Y-%m-%d %H:%M"),
            instruction.lines().next().unwrap_or(""),
            output
                .lines()
                .rev()
                .take(30)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
        ),
        BisectVerdict::AlreadyFailing { since, runs } => format!(
            "`{command}` already failed before the oldest turn looked at ({}), so none of the turns since broke it ({runs} check runs).",
            since.format("%Y-%m-%d %H:%M")
        ),
        BisectVerdict::Passes { runs } => format!(
            "`{command}` passes in a clean copy of the current state ({runs} check run): the failure depends on something outside the repository's files, such as a database, a service or an ignored file that differs."
        ),
        BisectVerdict::NoHistory => {
            "No turns with checkpoints in this folder to compare with.".to_string()
        }
        BisectVerdict::Failed { error } => format!("Could not find out: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::first_failing;

    #[test]
    fn halves_its_way_to_the_first_failing_state() {
        // States 0..=10; the checks fail from 7 on.
        let mut tried = Vec::new();
        let first = first_failing(0, 10, |index| {
            tried.push(index);
            Some(index < 7)
        });
        assert_eq!(first, Some(7));
        assert!(tried.len() <= 4, "about log2(10) runs: {tried:?}");
        assert_eq!(first_failing(3, 4, |_| unreachable!()), Some(4), "adjacent");
        assert_eq!(first_failing(0, 10, |_| None), None, "stopped");
    }
}
