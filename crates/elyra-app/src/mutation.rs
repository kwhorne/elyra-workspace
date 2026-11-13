//! The mutation gate: green checks after a turn say the code passes the
//! tests, not that the tests would notice if it were wrong. So once they pass,
//! Elyra makes small changes to the lines the instruction added (a flipped
//! comparison, `&&` for `||`, a dropped `!`; see `elyra_index::mutate`), one at
//! a time in a temporary checkout of the folder as the turn left it, and runs
//! the checks again. A change they still pass with is a line no test guards;
//! the agent can be asked for tests that catch it, and the gate runs again
//! after them. No model is asked.
//!
//! Only for checks that are quick enough to run a few more times, and not on
//! test files.

use elyra_core::{GateState, ItemContent, MutantResult, TranscriptItem};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Checks that took longer than this aren't run again for the gate.
pub const MAX_CHECK_MS: u64 = 90_000;
/// How the message starts that asks the agent for the missing tests.
pub const ASK_START: &str = "Elyra tested the tests";

/// A change to try.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub result: MutantResult,
    /// The file as the turn left it, and with the change.
    pub original: String,
    pub mutated: String,
}

/// Pick up to `max` changes on the lines added between `before` and `after`
/// (commits of the repository at `root`): first the most telling change on
/// each line, taking turns between files, then more on the same lines.
/// Blocking.
pub fn choose(
    root: &Path,
    before: &str,
    after: &str,
    max: usize,
) -> anyhow::Result<Vec<Candidate>> {
    let added = elyra_git::checkpoint::added_between(root, before, after)?;
    let paths: Vec<String> = added
        .into_iter()
        .map(|(path, _)| path)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        // Changing a test doesn't test it.
        .filter(|path| !crate::keep_test::is_test(path))
        .collect();
    let mut per_file: BTreeMap<String, (String, Vec<elyra_index::mutate::Mutant>)> =
        BTreeMap::new();
    for path in paths {
        let Some(language) = elyra_index::Language::from_path(Path::new(&path)) else {
            continue;
        };
        let Some(source) = elyra_git::undo::content_at(root, after, &path)
            .and_then(|bytes| String::from_utf8(bytes).ok())
        else {
            continue;
        };
        let diff = elyra_git::diff_text_of(root, before, after, &path)?;
        let lines: HashSet<usize> = elyra_git::parse_unified_diff(&path, &diff)
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| line.kind == elyra_git::DiffLineKind::Added)
            .filter_map(|line| line.new_line.map(|n| n as usize))
            .collect();
        let mut mutants = elyra_index::mutate::mutants(language, &source, &lines);
        if mutants.is_empty() {
            continue;
        }
        mutants.sort_by_key(|m| (m.kind, m.line, m.start));
        per_file.insert(path, (source, mutants));
    }
    // First the best change on each line, then the rest.
    let mut rounds: Vec<Vec<(String, usize)>> = vec![Vec::new(), Vec::new()];
    for (path, (_, mutants)) in &per_file {
        let mut seen = HashSet::new();
        let (mut first, mut rest) = (Vec::new(), Vec::new());
        for (index, mutant) in mutants.iter().enumerate() {
            if seen.insert(mutant.line) {
                first.push((path.clone(), index));
            } else {
                rest.push((path.clone(), index));
            }
        }
        rounds[0].extend(first);
        rounds[1].extend(rest);
    }
    let mut picked = Vec::new();
    for round in rounds {
        for (path, index) in take_turns(round) {
            if picked.len() == max {
                break;
            }
            let (source, mutants) = &per_file[&path];
            let mutant = &mutants[index];
            let code = source
                .lines()
                .nth(mutant.line - 1)
                .unwrap_or("")
                .trim()
                .chars()
                .take(120)
                .collect();
            picked.push(Candidate {
                result: MutantResult {
                    path: path.clone(),
                    line: mutant.line,
                    code,
                    change: mutant.describe(),
                    caught: None,
                },
                original: source.clone(),
                mutated: mutant.apply(source),
            });
        }
    }
    picked.sort_by(|a, b| (&a.result.path, a.result.line).cmp(&(&b.result.path, b.result.line)));
    Ok(picked)
}

/// Reorder so that files take turns: a1 b1 a2 b2 …
fn take_turns(items: Vec<(String, usize)>) -> Vec<(String, usize)> {
    let mut by_file: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (path, index) in items {
        by_file.entry(path).or_default().push(index);
    }
    let longest = by_file.values().map(Vec::len).max().unwrap_or(0);
    (0..longest)
        .flat_map(|n| {
            by_file
                .iter()
                .filter_map(move |(path, indexes)| indexes.get(n).map(|i| (path.clone(), *i)))
        })
        .collect()
}

/// Run `command` in a temporary checkout of `after` with each change in turn
/// (`dir`: the thread's folder in the repository at `root`): whether the
/// checks caught it. A run longer than `limit` counts as caught (the change
/// made something hang). Shorter than `candidates` when stopped. Blocking.
pub fn run(
    root: &Path,
    dir: &Path,
    after: &str,
    command: &str,
    candidates: &[Candidate],
    limit: Duration,
    cancel: &Arc<AtomicBool>,
) -> anyhow::Result<Vec<bool>> {
    let scratch = elyra_git::scratch::ScratchCheckout::new(root)?;
    scratch.checkout(after)?;
    let folder = scratch
        .dir()
        .join(dir.strip_prefix(root).unwrap_or(Path::new("")));
    let mut caught = Vec::new();
    for candidate in candidates {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let file = scratch.dir().join(&candidate.result.path);
        std::fs::write(&file, &candidate.mutated)?;
        let outcome = run_limited(&folder, command, limit, cancel);
        std::fs::write(&file, &candidate.original)?;
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        caught.push(!outcome.passed);
    }
    Ok(caught)
}

/// `checks::run`, stopped after `limit` or when `cancel` is set.
fn run_limited(
    dir: &Path,
    command: &str,
    limit: Duration,
    cancel: &Arc<AtomicBool>,
) -> crate::checks::Outcome {
    let stop = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    {
        let (stop, done, cancel) = (stop.clone(), done.clone(), cancel.clone());
        std::thread::spawn(move || {
            let started = Instant::now();
            while !done.load(Ordering::Relaxed) {
                if started.elapsed() >= limit || cancel.load(Ordering::Relaxed) {
                    stop.store(true, Ordering::Relaxed);
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
    }
    let outcome = crate::checks::run(dir, command, &stop);
    done.store(true, Ordering::Relaxed);
    outcome
}

/// How long one run may take: a few times the check that passed.
pub fn limit_for(check_ms: u64) -> Duration {
    Duration::from_millis((check_ms * 3).max(30_000))
}

/// A line no test guards: a change the checks still passed with, and the
/// command they were.
#[derive(Clone, Debug, PartialEq)]
pub struct Unguarded {
    pub result: MutantResult,
    pub command: String,
}

/// The changes the tests missed in a thread, as the latest gate that tried
/// each found them (a later gate that caught it clears it), by path.
pub fn unguarded(items: &[TranscriptItem]) -> HashMap<String, Vec<Unguarded>> {
    let mut latest: BTreeMap<(String, String, String), Unguarded> = BTreeMap::new();
    for item in items {
        let ItemContent::MutationGate {
            command,
            mutants,
            state: GateState::Done | GateState::Stopped,
            ..
        } = &item.content
        else {
            continue;
        };
        for mutant in mutants.iter().filter(|m| m.caught.is_some()) {
            latest.insert(
                (
                    mutant.path.clone(),
                    mutant.code.clone(),
                    mutant.change.clone(),
                ),
                Unguarded {
                    result: mutant.clone(),
                    command: command.clone(),
                },
            );
        }
    }
    let mut by_path: HashMap<String, Vec<Unguarded>> = HashMap::new();
    for unguarded in latest.into_values() {
        if unguarded.result.caught == Some(false) {
            by_path
                .entry(unguarded.result.path.clone())
                .or_default()
                .push(unguarded);
        }
    }
    by_path
}

/// Whether a line of code is the one a mutant was made on (`code`: trimmed,
/// as the gate keeps it).
pub fn is_line(code: &str, text: &str) -> bool {
    text.trim().chars().take(120).eq(code.chars())
}

/// The message asking the agent for tests that catch what got through.
pub fn ask_prompt(command: &str, mutants: &[MutantResult]) -> Option<String> {
    let missed: Vec<&MutantResult> = mutants.iter().filter(|m| m.caught == Some(false)).collect();
    if missed.is_empty() {
        return None;
    }
    let mut text = format!(
        "{ASK_START}: after your changes passed `{command}`, Elyra made small changes to the \
         lines this instruction added and ran the checks again. They still passed with these, \
         so no test guards those lines:\n"
    );
    for m in missed {
        text.push_str(&format!(
            "\n- {}:{} `{}`: {}",
            m.path, m.line, m.code, m.change
        ));
    }
    text.push_str(
        "\n\nAdd tests that fail with each of these changes and pass with the code as it is now. \
         Don't change the code itself, only add tests.",
    );
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::{ASK_START, ask_prompt, choose, take_turns};
    use elyra_core::MutantResult;
    use std::process::Command;

    #[test]
    fn files_take_turns() {
        let items = vec![
            ("a".into(), 0),
            ("a".into(), 1),
            ("a".into(), 2),
            ("b".into(), 0),
        ];
        let order: Vec<String> = take_turns(items)
            .into_iter()
            .map(|(p, i)| format!("{p}{i}"))
            .collect();
        assert_eq!(order, ["a0", "b0", "a1", "a2"]);
    }

    #[test]
    fn picks_changes_on_the_added_lines() {
        let dir = std::env::temp_dir().join(format!("elyra-mutation-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tests")).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .current_dir(&dir)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let commit = |message: &str| {
            git(&["add", "-A"]);
            git(&[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-qm",
                message,
            ]);
            git(&["rev-parse", "HEAD"])
        };
        git(&["init", "-q"]);
        std::fs::write(dir.join("rules.py"), "def old(n):\n    return n > 1\n").unwrap();
        let before = commit("before");
        std::fs::write(
            dir.join("rules.py"),
            "def old(n):\n    return n > 1\n\ndef free(total):\n    return total > 500 and total != 999\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("tests/test_rules.py"),
            "assert free(600) == True\n",
        )
        .unwrap();
        let after = commit("after");
        let picked = choose(&dir, &before, &after, 4).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let changes: Vec<(usize, &str)> = picked
            .iter()
            .map(|c| (c.result.line, c.result.change.as_str()))
            .collect();
        // Line 2 is older, and the test file isn't changed.
        assert_eq!(
            changes,
            [(5, "`>` → `<=`"), (5, "`!=` → `==`"), (5, "`and` → `or`")]
        );
        assert_eq!(picked[0].result.code, "return total > 500 and total != 999");
        assert!(
            picked[0]
                .mutated
                .contains("return total <= 500 and total != 999")
        );
        assert_eq!(choose(&std::env::temp_dir(), "x", "y", 4).ok(), None);
    }

    #[test]
    fn a_later_gate_that_catches_it_clears_it() {
        use super::{is_line, unguarded};
        use elyra_core::{GateState, ItemContent, TranscriptItem};
        let mutant = |line, change: &str, caught| MutantResult {
            path: "rules.py".into(),
            line,
            code: "return total > 500 and total != 999".into(),
            change: change.into(),
            caught: Some(caught),
        };
        let gate = |mutants, state| TranscriptItem {
            id: elyra_core::new_id(),
            thread_id: elyra_core::new_id(),
            seq: 0,
            content: ItemContent::MutationGate {
                command: "pytest".into(),
                mutants,
                state,
                asked: false,
            },
            created_at: chrono::Utc::now(),
        };
        let first = gate(
            vec![
                mutant(5, "`and` → `or`", false),
                mutant(5, "`!=` → `==`", false),
            ],
            GateState::Done,
        );
        let again = gate(vec![mutant(6, "`and` → `or`", true)], GateState::Done);
        let running = gate(vec![mutant(6, "`>` → `<=`", false)], GateState::Running);
        let found = unguarded(&[first.clone(), again, running]);
        let changes: Vec<&str> = found["rules.py"]
            .iter()
            .map(|u| u.result.change.as_str())
            .collect();
        assert_eq!(
            changes,
            ["`!=` → `==`"],
            "caught later, and not run yet, don't count"
        );
        assert_eq!(found["rules.py"][0].command, "pytest");
        assert_eq!(unguarded(&[first])["rules.py"].len(), 2);
        assert!(is_line(
            "return total > 500 and total != 999",
            "    return total > 500 and total != 999  "
        ));
        assert!(!is_line(
            "return total > 500",
            "return total > 500 and total != 999"
        ));
    }

    #[test]
    fn asks_for_tests_that_catch_what_got_through() {
        let result = |line, caught| MutantResult {
            path: "rules.py".into(),
            line,
            code: "return total > 500 and total != 999".into(),
            change: "`and` → `or`".into(),
            caught,
        };
        assert_eq!(ask_prompt("pytest", &[result(5, Some(true))]), None);
        let text = ask_prompt("pytest", &[result(5, Some(true)), result(6, Some(false))]).unwrap();
        assert!(text.starts_with(ASK_START));
        assert!(
            text.contains("\n- rules.py:6 `return total > 500 and total != 999`: `and` → `or`")
        );
        assert!(!text.contains("rules.py:5"));
    }
}
