//! Thrash guard: notice, while a turn runs, that the agent is going round in
//! circles, so it can be stopped and started over (files back to the turn's
//! checkpoint, the escalation model on the new attempt) before it burns the
//! context and the budget. Two signs, from the turn's own tool calls (no
//! model is asked):
//!
//! - the same call failing the same way [`REPEATS`] times: whatever the agent
//!   changed in between made no difference;
//! - [`STREAK`] calls in a row failing (per agent: the main one and each
//!   subagent), with nothing working in between.

use serde_json::Value;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

/// The same call failing the same way this many times is going in circles.
pub const REPEATS: usize = 3;
/// This many failed calls in a row is going nowhere.
pub const STREAK: usize = 6;

/// What a finished tool call looked like.
pub struct Call<'a> {
    pub name: &'a str,
    pub input: &'a Value,
    /// The subagent that made it, if any.
    pub parent: Option<&'a str>,
    pub is_error: bool,
    pub output: &'a str,
}

/// Why the guard thinks the agent is stuck.
#[derive(Debug, PartialEq)]
pub struct Stuck {
    /// One line, for people and for the agent.
    pub reason: String,
    /// The end of the last failure's output.
    pub detail: String,
}

/// The tool calls of one turn so far.
#[derive(Default)]
pub struct Guard {
    streaks: HashMap<Option<String>, usize>,
    repeats: HashMap<u64, usize>,
    tripped: bool,
}

impl Guard {
    /// A new turn.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Take in a finished call; says so (once a turn) when the agent is stuck.
    pub fn observe(&mut self, call: &Call) -> Option<Stuck> {
        if self.tripped {
            return None;
        }
        let streak = self
            .streaks
            .entry(call.parent.map(str::to_string))
            .or_default();
        // A call the user turned down isn't the agent's failure.
        if !call.is_error || declined(call.output) {
            *streak = 0;
            return None;
        }
        *streak += 1;
        let streak = *streak;
        let mut key = DefaultHasher::new();
        call.name.hash(&mut key);
        call.input.to_string().hash(&mut key);
        normalize(call.output).hash(&mut key);
        let repeats = self.repeats.entry(key.finish()).or_default();
        *repeats += 1;
        let reason = if *repeats >= REPEATS {
            let what = crate::transcript::tool_summary(call.name, call.input);
            format!(
                "{} failed the same way {} times",
                clip(what.trim(), 80),
                *repeats
            )
        } else if streak >= STREAK {
            format!("{streak} tool calls failed in a row")
        } else {
            return None;
        };
        self.tripped = true;
        Some(Stuck {
            reason,
            detail: tail(call.output, 600),
        })
    }
}

/// The output without what changes from run to run (numbers: times, ids,
/// line numbers) and spacing, the end of it (where errors are).
fn normalize(output: &str) -> String {
    let text: String = output
        .chars()
        .filter(|c| !c.is_ascii_digit())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let skip = text.chars().count().saturating_sub(1500);
    text.chars().skip(skip).collect()
}

fn declined(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    [
        "user rejected",
        "user denied",
        "was rejected by the user",
        // Claude Code: "The user doesn't want to proceed with this tool use."
        "doesn't want to proceed",
        "permission denied by user",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn clip(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("");
    if line.chars().count() <= max {
        line.to_string()
    } else {
        format!("{}…", line.chars().take(max).collect::<String>())
    }
}

/// The last `max` characters of the output, trimmed.
fn tail(output: &str, max: usize) -> String {
    let output = output.trim();
    let skip = output.chars().count().saturating_sub(max);
    let tail: String = output.chars().skip(skip).collect();
    if skip > 0 { format!("…{tail}") } else { tail }
}

#[cfg(test)]
mod tests {
    use super::{Call, Guard, REPEATS, STREAK};
    use serde_json::json;

    fn call<'a>(input: &'a serde_json::Value, is_error: bool, output: &'a str) -> Call<'a> {
        Call {
            name: "Bash",
            input,
            parent: None,
            is_error,
            output,
        }
    }

    #[test]
    fn the_same_failure_again_and_again_is_stuck() {
        let mut guard = Guard::default();
        let test = json!({"command": "cargo test"});
        let edit = json!({"file_path": "src/lib.rs"});
        for run in 1..=REPEATS {
            let output =
                format!("error[E0308]: mismatched types at src/lib.rs:{run}\n(took {run}.2s)");
            let stuck = guard.observe(&call(&test, true, &output));
            // Edits in between don't make it progress.
            guard.observe(&Call {
                name: "Edit",
                input: &edit,
                parent: None,
                is_error: false,
                output: "ok",
            });
            if run < REPEATS {
                assert_eq!(stuck, None);
            } else {
                let stuck = stuck.expect("stuck");
                assert!(stuck.reason.contains("cargo test"), "{}", stuck.reason);
                assert!(stuck.reason.contains("3 times"), "{}", stuck.reason);
                assert!(stuck.detail.contains("mismatched types"));
            }
        }
        // Once a turn.
        assert_eq!(
            guard.observe(&call(&test, true, "error[E0308]: mismatched types")),
            None
        );
    }

    #[test]
    fn a_different_error_each_time_is_progress() {
        let mut guard = Guard::default();
        let test = json!({"command": "cargo test"});
        for (run, error) in ["missing import", "wrong type", "unused variable"]
            .iter()
            .enumerate()
        {
            let output = format!("error: {error}");
            assert_eq!(
                guard.observe(&call(&test, true, &output)),
                None,
                "run {run}"
            );
            guard.observe(&call(&json!({"command": "ls"}), false, "src"));
        }
    }

    #[test]
    fn failure_after_failure_is_stuck() {
        let mut guard = Guard::default();
        let inputs: Vec<_> = (0..STREAK)
            .map(|n| json!({"command": format!("try {n}")}))
            .collect();
        for (n, input) in inputs.iter().enumerate() {
            let output = format!("command {n} not found: x{}", "y".repeat(n));
            let stuck = guard.observe(&call(input, true, &output));
            assert_eq!(stuck.is_some(), n + 1 == STREAK, "call {n}");
        }
    }

    #[test]
    fn a_call_that_works_ends_the_streak() {
        let mut guard = Guard::default();
        // Three failures, one that works, and again.
        for n in 0..STREAK * 3 {
            let input = json!({"command": format!("try {n}")});
            let output = format!("failure {}", "z".repeat(n));
            assert_eq!(
                guard.observe(&call(&input, n % 4 != 3, &output)),
                None,
                "call {n}"
            );
        }
    }

    #[test]
    fn subagents_keep_their_own_streak() {
        let mut guard = Guard::default();
        for n in 0..STREAK - 1 {
            let input = json!({"command": format!("try {n}")});
            let output = format!("nope {}", "a".repeat(n));
            for parent in [None, Some("task-1")] {
                let stuck = guard.observe(&Call {
                    name: "Bash",
                    input: &input,
                    parent,
                    is_error: true,
                    output: &output,
                });
                assert_eq!(stuck, None);
            }
        }
    }

    #[test]
    fn calls_the_user_turned_down_dont_count() {
        let mut guard = Guard::default();
        let input = json!({"command": "rm -rf build"});
        for _ in 0..STREAK {
            assert_eq!(
                guard.observe(&call(&input, true, "The user rejected this tool use.")),
                None
            );
        }
        // Claude Code's words for it.
        let claude = "The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file).";
        for _ in 0..STREAK {
            assert_eq!(guard.observe(&call(&input, true, claude)), None);
        }
    }

    #[test]
    fn a_new_turn_starts_over() {
        let mut guard = Guard::default();
        let input = json!({"command": "make"});
        for _ in 0..REPEATS - 1 {
            guard.observe(&call(&input, true, "make: *** No rule"));
        }
        guard.reset();
        assert_eq!(
            guard.observe(&call(&input, true, "make: *** No rule")),
            None
        );
    }
}
