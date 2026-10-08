//! "Done means green": the project's check command (tests, lint) runs after
//! a turn that changed files, and failures go back to the agent.
//!
//! This module is UI-agnostic: suggesting a command, telling whether a turn
//! changed the working tree, running the command and writing the follow-up
//! message. `ThreadSession` drives it.

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Longest a check may run before it is stopped and counted as failed.
const TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// Output kept from a check (its end).
const KEEP_OUTPUT: usize = 16 * 1024;
/// Output sent back to the agent (its end).
const PROMPT_OUTPUT: usize = 12 * 1024;

/// What a check run found.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub passed: bool,
    /// None when it was stopped (timeout, cancel) or could not start.
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    /// The end of the combined output, without colour codes.
    pub output: String,
}

/// A check command for the project in `dir`, from the files it has.
pub fn suggest(dir: &Path) -> Option<String> {
    let has = |name: &str| dir.join(name).exists();
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
    if has("Cargo.toml") {
        return Some("cargo test".into());
    }
    if has("artisan") && has("composer.json") {
        return Some("php artisan test".into());
    }
    if let Some(composer) = read("composer.json")
        && json_script(&composer, "test")
    {
        return Some("composer test".into());
    }
    if has("vendor/bin/pest") {
        return Some("vendor/bin/pest".into());
    }
    if has("phpunit.xml") || has("phpunit.xml.dist") {
        return Some("vendor/bin/phpunit".into());
    }
    if let Some(package) = read("package.json")
        && json_script(&package, "test")
    {
        let runner = if has("pnpm-lock.yaml") {
            "pnpm"
        } else if has("yarn.lock") {
            "yarn"
        } else if has("bun.lockb") || has("bun.lock") {
            "bun run"
        } else {
            "npm"
        };
        return Some(format!("{runner} test"));
    }
    if has("go.mod") {
        return Some("go test ./...".into());
    }
    if has("pyproject.toml") || has("pytest.ini") || has("setup.py") {
        return Some("pytest".into());
    }
    if read("Makefile").is_some_and(|make| make.lines().any(|line| line.starts_with("test:"))) {
        return Some("make test".into());
    }
    None
}

/// Whether a package.json / composer.json defines a real `name` script (not
/// npm's placeholder that only fails).
fn json_script(json: &str, name: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return false;
    };
    match &value["scripts"][name] {
        serde_json::Value::String(script) => !script.contains("no test specified"),
        serde_json::Value::Array(steps) => !steps.is_empty(),
        _ => false,
    }
}

/// Whether the working tree changed since the turn's checkpoint (taken
/// before the message reached the agent). Without one, assume it did.
pub fn changed_since(dir: &Path, checkpoint: Option<&str>) -> bool {
    let (Some(checkpoint), Ok(root)) = (checkpoint, elyra_git::repo_root(dir)) else {
        return true;
    };
    elyra_git::checkpoint::changed_since(&root, checkpoint).unwrap_or(true)
}

/// Run `command` in `dir` with the user's login shell, until it ends, the
/// timeout passes or `cancel` is set.
pub fn run(dir: &Path, command: &str, cancel: &Arc<AtomicBool>) -> Outcome {
    let started = Instant::now();
    // A POSIX login shell (fish doesn't take `exec 2>&1`).
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.ends_with("fish"))
        .unwrap_or_else(|| "/bin/sh".into());
    let child = Command::new(&shell)
        .arg("-lc")
        // Merge stderr into stdout so the output keeps its order.
        .arg(format!("exec 2>&1\n{command}"))
        .current_dir(dir)
        // Test runners stay non-interactive (no watch mode, no prompts).
        .env("CI", "1")
        .env("ELYRA_CHECK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(err) => {
            return Outcome {
                passed: false,
                exit_code: None,
                duration_ms: started.elapsed().as_millis() as u64,
                output: format!("Couldn't start {shell}: {err}"),
            };
        }
    };
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut kept: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            kept.extend_from_slice(&buf[..n]);
            if kept.len() > KEEP_OUTPUT * 2 {
                kept.drain(..kept.len() - KEEP_OUTPUT);
            }
        }
        kept
    });
    let mut stopped = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        if cancel.load(Ordering::Relaxed) {
            stopped = Some("Stopped.");
        } else if started.elapsed() > TIMEOUT {
            stopped = Some("Stopped after 20 minutes.");
        }
        if stopped.is_some() {
            kill_tree(&mut child);
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let raw = reader.join().unwrap_or_default();
    let mut output = tail(&strip_ansi(&String::from_utf8_lossy(&raw)), KEEP_OUTPUT);
    if let Some(note) = stopped {
        output.push_str(&format!("\n{note}"));
    }
    let exit_code = status.and_then(|s| s.code());
    Outcome {
        passed: status.is_some_and(|s| s.success()),
        exit_code,
        duration_ms: started.elapsed().as_millis() as u64,
        output: output.trim_end().to_string(),
    }
}

/// Stop the shell and what it started (test runners spawn workers).
fn kill_tree(child: &mut std::process::Child) {
    let _ = Command::new("pkill")
        .args(["-TERM", "-P", &child.id().to_string()])
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

/// How every follow-up message for a failed check begins.
const FIX_PROMPT_START: &str = "The project's checks failed after your changes:";

/// "automatic fix 1 of 2" when `text` is a follow-up message for a failed
/// check (shown folded in the transcript).
pub fn fix_prompt_label(text: &str) -> Option<String> {
    if !text.starts_with(FIX_PROMPT_START) && !text.starts_with("The browser journey ") {
        return None;
    }
    let start = text.find("(automatic fix ")? + 1;
    let end = start + text[start..].find(')')?;
    Some(text[start..end].to_string())
}

/// The message that sends a failed check back to the agent.
pub fn fix_prompt(command: &str, outcome: &Outcome, attempt: u32, attempts: u32) -> String {
    let how = match outcome.exit_code {
        Some(code) => format!("exited with {code}"),
        None => "did not finish".to_string(),
    };
    format!(
        "{FIX_PROMPT_START} `{command}` {how}. \
         Fix the cause (not the checks), then finish your turn; the checks run again \
         (automatic fix {attempt} of {attempts}).\n\nEnd of the output:\n```\n{}\n```",
        tail(&outcome.output, PROMPT_OUTPUT)
    )
}

/// The last `max` bytes of `text`, starting at a line.
pub fn tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let cut = &text[start..];
    match cut.find('\n') {
        Some(newline) => format!("…\n{}", &cut[newline + 1..]),
        None => format!("…{cut}"),
    }
}

/// Remove terminal colour and cursor codes.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // Parameters, then one final byte in @..~.
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
        } else if c == '\r' {
            // Progress lines rewrite themselves; keep the last version.
            if chars.peek() != Some(&'\n') {
                let line_start = out.rfind('\n').map_or(0, |i| i + 1);
                out.truncate(line_start);
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{fix_prompt, json_script, run, strip_ansi, suggest, tail};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("elyra-checks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn suggests_a_command_from_the_project_files() {
        let dir = temp("suggest");
        assert_eq!(suggest(&dir), None);
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
        )
        .unwrap();
        assert_eq!(suggest(&dir), None, "npm's placeholder is not a test");
        std::fs::write(dir.join("package.json"), r#"{"scripts":{"test":"vitest"}}"#).unwrap();
        assert_eq!(suggest(&dir).as_deref(), Some("npm test"));
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(suggest(&dir).as_deref(), Some("pnpm test"));
        std::fs::write(dir.join("composer.json"), "{}").unwrap();
        std::fs::write(dir.join("artisan"), "").unwrap();
        assert_eq!(suggest(&dir).as_deref(), Some("php artisan test"));
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        assert_eq!(suggest(&dir).as_deref(), Some("cargo test"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(json_script(r#"{"scripts":{"test":["phpunit"]}}"#, "test"));
    }

    #[test]
    fn runs_a_command_and_keeps_its_output() {
        let dir = temp("run");
        let cancel = Arc::new(AtomicBool::new(false));
        let ok = run(&dir, "echo one; echo two >&2", &cancel);
        assert!(ok.passed);
        assert_eq!(ok.exit_code, Some(0));
        assert_eq!(ok.output, "one\ntwo", "stderr is kept, in order");
        let failed = run(&dir, "printf '\\033[31mred\\033[0m\\n'; exit 3", &cancel);
        assert!(!failed.passed);
        assert_eq!(failed.exit_code, Some(3));
        assert_eq!(failed.output, "red", "colour codes are removed");
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let stopped = run(&dir, "sleep 30", &cancel);
        assert!(!stopped.passed && stopped.exit_code.is_none());
        assert!(stopped.duration_ms < 5000);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writes_the_follow_up_message() {
        let outcome = super::Outcome {
            passed: false,
            exit_code: Some(101),
            duration_ms: 10,
            output: "test parse ... FAILED".into(),
        };
        let prompt = fix_prompt("cargo test", &outcome, 1, 2);
        assert!(prompt.contains("`cargo test` exited with 101"));
        assert!(prompt.contains("fix 1 of 2"));
        assert!(prompt.ends_with("test parse ... FAILED\n```"));
        assert_eq!(
            super::fix_prompt_label(&prompt).as_deref(),
            Some("automatic fix 1 of 2")
        );
        assert_eq!(super::fix_prompt_label("Please fix the tests"), None);
    }

    #[test]
    fn trims_text_for_people_and_agents() {
        assert_eq!(tail("a\nb\nc", 100), "a\nb\nc");
        assert_eq!(tail("first line\nsecond\nthird", 12), "…\nthird");
        assert_eq!(strip_ansi("10%\r50%\r100%\ndone"), "100%\ndone");
        assert_eq!(strip_ansi("\u{1b}[1;32mok\u{1b}[0m"), "ok");
    }
}
