//! A Laravel app's errors, traced to the instruction behind them: new
//! entries in `storage/logs/laravel*.log` are read as they come, the first
//! stack frame in the app's own code is the line that failed, and the line
//! record says which instruction had an agent write it. The thread shows it
//! with Ask why; agents ask with the gateway's `error_origin`.

use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

/// Levels worth telling about.
const LEVELS: &[&str] = &["ERROR", "CRITICAL", "ALERT", "EMERGENCY"];
/// At most this much of the log is read at a time (its end).
const MAX_READ: u64 = 512 * 1024;

/// A logged error.
#[derive(Clone, Debug, PartialEq)]
pub struct AppError {
    pub at: String,
    pub level: String,
    pub message: String,
    /// Where it was thrown, then the stack: (path as logged, line).
    pub frames: Vec<(String, u32)>,
    /// The entry as logged, cut.
    pub text: String,
}

/// The line in the app's code that failed: repository-relative path, line,
/// its text now.
#[derive(Clone, Debug, PartialEq)]
pub struct Culprit {
    pub path: String,
    pub line: u32,
    pub text: String,
}

/// The errors in a piece of Laravel's log (`[time] env.LEVEL: message …`).
pub fn parse(text: &str) -> Vec<AppError> {
    let mut entries: Vec<(String, String, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if let Some((at, level, first)) = header(line) {
            entries.push((at, level, vec![first.to_string()]));
        } else if let Some((_, _, lines)) = entries.last_mut() {
            lines.push(line.to_string());
        }
    }
    entries
        .into_iter()
        .filter(|(_, level, _)| LEVELS.contains(&level.as_str()))
        .map(|(at, level, lines)| {
            let first = &lines[0];
            // The message ends where the context's JSON begins.
            let message = first
                .split_once(" {\"")
                .map_or(first.as_str(), |(message, _)| message)
                .trim()
                .to_string();
            let mut frames = Vec::new();
            for line in &lines {
                if let Some(frame) = thrown_at(line) {
                    frames.push(frame);
                }
                if let Some(frame) = stack_frame(line) {
                    frames.push(frame);
                }
            }
            let text: String = lines.join("\n").chars().take(4_000).collect();
            AppError {
                at,
                level,
                message: message.chars().take(300).collect(),
                frames,
                text,
            }
        })
        .collect()
}

/// `[2026-10-15 12:00:00] local.ERROR: message` → (time, level, message).
fn header(line: &str) -> Option<(String, String, &str)> {
    let rest = line.strip_prefix('[')?;
    let (at, rest) = rest.split_once("] ")?;
    if !at.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let (channel, message) = rest.split_once(": ")?;
    let (_, level) = channel.rsplit_once('.')?;
    level
        .chars()
        .all(|c| c.is_ascii_uppercase())
        .then(|| (at.to_string(), level.to_string(), message))
}

/// `… at /path/File.php:42)` in an exception's summary.
fn thrown_at(line: &str) -> Option<(String, u32)> {
    let at = line.rfind(" at ")?;
    let rest = &line[at + 4..];
    let rest = rest.split(')').next()?;
    let (path, number) = rest.rsplit_once(':')?;
    Some((path.to_string(), number.trim().parse().ok()?))
}

/// `#3 /path/File.php(42): …` in a stack trace.
fn stack_frame(line: &str) -> Option<(String, u32)> {
    let rest = line.strip_prefix('#')?;
    let (_, rest) = rest.split_once(' ')?;
    let (path, rest) = rest.split_once('(')?;
    let (number, _) = rest.split_once("):")?;
    Some((path.to_string(), number.parse().ok()?))
}

/// A logged path as a path in the repository at `root`: as it is, or (from a
/// container or another machine) by the longest end of it that exists here.
fn in_repository(root: &Path, logged: &str) -> Option<String> {
    let path = Path::new(logged);
    if let Ok(relative) = path.strip_prefix(root) {
        return Some(relative.to_string_lossy().replace('\\', "/"));
    }
    let parts: Vec<&str> = logged.split('/').filter(|p| !p.is_empty()).collect();
    (0..parts.len()).find_map(|start| {
        let relative = parts[start..].join("/");
        root.join(&relative).is_file().then_some(relative)
    })
}

/// The first frame in the app's own code (not `vendor/`, compiled views or
/// caches), with its line as it is now. Blocking.
pub fn culprit(root: &Path, error: &AppError) -> Option<Culprit> {
    error.frames.iter().find_map(|(logged, line)| {
        let path = in_repository(root, logged)?;
        if ["vendor/", "storage/", "bootstrap/cache/", "node_modules/"]
            .iter()
            .any(|skip| path.starts_with(skip))
        {
            return None;
        }
        let content = std::fs::read_to_string(root.join(&path)).ok()?;
        let text = content.lines().nth((*line as usize).checked_sub(1)?)?;
        Some(Culprit {
            path,
            line: *line,
            text: text.to_string(),
        })
    })
}

/// The newest Laravel log in the app at `dir`.
fn log_file(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir.join("storage").join("logs"))
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("laravel") && name.ends_with(".log")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max()
        .map(|(_, path)| path)
}

/// Where reading the log got to.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    file: PathBuf,
    offset: u64,
}

/// The errors logged since `seen`; the first call only notes where the log
/// ends. Blocking.
pub fn new_errors(dir: &Path, seen: Option<Seen>) -> (Option<Seen>, Vec<AppError>) {
    let Some(file) = log_file(dir) else {
        return (seen, Vec::new());
    };
    let Ok(len) = std::fs::metadata(&file).map(|m| m.len()) else {
        return (seen, Vec::new());
    };
    let from = match &seen {
        None => return (Some(Seen { file, offset: len }), Vec::new()),
        // Rotated to a new day's file, or cleared: from its start.
        Some(seen) if seen.file != file || len < seen.offset => 0,
        Some(seen) => seen.offset,
    };
    let errors = if len > from {
        read_from(&file, from.max(len.saturating_sub(MAX_READ)))
            .map(|text| parse(&text))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    (Some(Seen { file, offset: len }), errors)
}

/// The latest `limit` errors in the log, newest last. Blocking.
pub fn latest(dir: &Path, limit: usize) -> Vec<AppError> {
    let Some(file) = log_file(dir) else {
        return Vec::new();
    };
    let len = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
    let errors = read_from(&file, len.saturating_sub(MAX_READ))
        .map(|text| parse(&text))
        .unwrap_or_default();
    let skip = errors.len().saturating_sub(limit);
    errors.into_iter().skip(skip).collect()
}

fn read_from(file: &Path, offset: u64) -> std::io::Result<String> {
    let mut handle = std::fs::File::open(file)?;
    handle.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    handle.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// An error with the line that failed and the instruction that wrote it.
#[derive(Clone, Debug)]
pub struct Traced {
    pub error: AppError,
    pub culprit: Option<Culprit>,
    /// "“Thread” · Agent · date", and the instruction.
    pub written_for: Option<(String, String)>,
}

/// Look the failing line up in the line record.
pub fn trace(
    app: &crate::app_state::AppState,
    root: &Path,
    error: AppError,
    culprit: Option<Culprit>,
) -> Traced {
    let written_for = culprit.as_ref().and_then(|culprit| {
        let project = crate::provenance::project_of(app, root)?;
        let hash = elyra_core::line_hash(&culprit.text)?;
        let origin = app
            .store
            .line_origins(project, &culprit.path, hash)
            .ok()?
            .into_iter()
            .next()?;
        let turn = crate::provenance::turn(app, &origin);
        Some((crate::provenance::short(&turn), turn.prompt))
    });
    Traced {
        error,
        culprit,
        written_for,
    }
}

/// One line for the error chip.
pub fn summary(traced: &Traced) -> String {
    let mut text = traced.error.message.clone();
    if let Some(culprit) = &traced.culprit {
        text.push_str(&format!(" — {}:{}", culprit.path, culprit.line));
    }
    if let Some((turn, _)) = &traced.written_for {
        text.push_str(&format!(", written in {turn}"));
    }
    text
}

/// The error for an agent: what was logged, the line, who wrote it.
pub fn describe(traced: &Traced) -> String {
    let mut out = format!(
        "{} {}: {}\n",
        traced.error.at, traced.error.level, traced.error.message
    );
    match &traced.culprit {
        Some(culprit) => {
            out.push_str(&format!(
                "Failing line in the app's code: {}:{}: `{}`\n",
                culprit.path,
                culprit.line,
                culprit.text.trim()
            ));
            match &traced.written_for {
                Some((turn, prompt)) => {
                    let prompt: String = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
                    let prompt: String = prompt.chars().take(400).collect();
                    out.push_str(&format!(
                        "Written by the agent in {turn}, for the message: “{prompt}”. Ask that agent why with ask_author (path {}, line {}).\n",
                        culprit.path, culprit.line
                    ));
                }
                None => {
                    out.push_str("No agent in Elyra Workspace is recorded as writing that line.\n")
                }
            }
        }
        None => out.push_str(
            "No frame in the app's own code (the error is in a package or the framework).\n",
        ),
    }
    let log: String = traced.error.text.chars().take(1_500).collect();
    out.push_str(&format!("\nLogged:\n```\n{log}\n```\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::{culprit, new_errors, parse};

    const LOG: &str = r#"[2026-10-15 12:00:01] local.INFO: Cart viewed {"id":1}
[2026-10-15 12:00:02] local.ERROR: Undefined array key "price" {"userId":1,"exception":"[object] (ErrorException(code: 0): Undefined array key \"price\" at /var/www/html/app/Http/Controllers/CartController.php:3)
[stacktrace]
#0 /var/www/html/vendor/laravel/framework/src/Illuminate/Foundation/Bootstrap/HandleExceptions.php(256): handleError(2, 'Undefined array...')
#1 /var/www/html/app/Http/Controllers/CartController.php(3): Illuminate\\Foundation\\Bootstrap\\HandleExceptions->Illuminate\\Foundation\\Bootstrap\\{closure}()
#2 {main}
"}
[2026-10-15 12:00:03] production.CRITICAL: Queue down
"#;

    #[test]
    fn reads_errors_and_their_frames() {
        let errors = parse(LOG);
        assert_eq!(errors.len(), 2, "INFO is left out");
        assert_eq!(errors[0].message, "Undefined array key \"price\"");
        assert_eq!(errors[0].level, "ERROR");
        assert_eq!(
            errors[0].frames,
            [
                ("/var/www/html/app/Http/Controllers/CartController.php".to_string(), 3),
                (
                    "/var/www/html/vendor/laravel/framework/src/Illuminate/Foundation/Bootstrap/HandleExceptions.php".to_string(),
                    256
                ),
                ("/var/www/html/app/Http/Controllers/CartController.php".to_string(), 3),
            ]
        );
        assert_eq!(errors[1].message, "Queue down");
    }

    #[test]
    fn finds_the_line_in_the_apps_own_code() {
        let root = std::env::temp_dir().join(format!("elyra-app-errors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("app/Http/Controllers")).unwrap();
        std::fs::create_dir_all(root.join("storage/logs")).unwrap();
        std::fs::write(
            root.join("app/Http/Controllers/CartController.php"),
            "<?php\n\n    return $item['price'];\n",
        )
        .unwrap();
        let error = parse(LOG).remove(0);
        let found = culprit(&root, &error).unwrap();
        assert_eq!(
            (found.path.as_str(), found.line, found.text.trim()),
            (
                "app/Http/Controllers/CartController.php",
                3,
                "return $item['price'];"
            ),
            "found from the container's path"
        );

        let log = root.join("storage/logs/laravel.log");
        std::fs::write(&log, "[2026-10-15 11:00:00] local.ERROR: Old one\n").unwrap();
        let (seen, errors) = new_errors(&root, None);
        assert!(errors.is_empty(), "what was there before isn't new");
        std::fs::write(
            &log,
            format!("[2026-10-15 11:00:00] local.ERROR: Old one\n{LOG}"),
        )
        .unwrap();
        let (seen, errors) = new_errors(&root, seen);
        assert_eq!(errors.len(), 2);
        let (_, errors) = new_errors(&root, seen);
        assert!(errors.is_empty(), "read once");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
