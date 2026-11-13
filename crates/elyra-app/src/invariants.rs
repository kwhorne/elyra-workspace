//! Invariants: rules the code must keep, checked by Elyra after every turn
//! that changed files, in milliseconds and without asking a model. Rules in
//! AGENTS.md get forgotten; an invariant gets checked. The hardest mistakes
//! are omissions in new files (a new API route without the tenant scope
//! asked for last week), which no edit to old lines would show.
//!
//! Each is a JSON file in the repository's `.elyra/invariants`, so the team
//! and other tools share them: a glob of the files it applies to and any of
//!
//! - `require`: a regular expression each of those files must match;
//! - `forbid`: one no line a turn adds to them may match;
//! - `command`: a shell command that must succeed, given the files in
//!   `ELYRA_FILES` (one per line).
//!
//! Only the files a turn changed are looked at. A broken invariant goes back
//! to the agent like failed checks, before the project's slower check
//! command runs. Agents propose them (`propose_invariant`, or asked to from a
//! proposed rule); the user saves them.

use elyra_core::{BrokenInvariant, Invariant};
use std::collections::HashSet;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const DIR: &str = ".elyra/invariants";
/// How the message starts that sends broken invariants back.
pub const FIX_START: &str = "The project's invariants broke with your changes:";
/// How long a `command` may take.
const COMMAND_LIMIT: Duration = Duration::from_secs(60);

/// Whether the repository at or above `dir` keeps invariants. Cheap.
pub fn exist(dir: &Path) -> bool {
    elyra_git::repo_root(dir).is_ok_and(|root| root.join(DIR).is_dir())
}

/// The invariants in the repository at `root`, and what couldn't be read.
pub fn load(root: &Path) -> (Vec<Invariant>, Vec<String>) {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    let Ok(entries) = std::fs::read_dir(root.join(DIR)) else {
        return (found, problems);
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        match std::fs::read_to_string(&path)
            .map_err(|err| err.to_string())
            .and_then(|text| serde_json::from_str::<Invariant>(&text).map_err(|e| e.to_string()))
            .and_then(|invariant| validate(&invariant).map(|_| invariant))
        {
            Ok(invariant) => found.push(invariant),
            Err(err) => problems.push(format!("{DIR}/{name}: {err}")),
        }
    }
    (found, problems)
}

/// Whether an invariant can be checked: a name, something to check, valid
/// expressions and a glob inside the repository.
pub fn validate(invariant: &Invariant) -> Result<(), String> {
    if invariant.name.trim().is_empty() {
        return Err("it needs a name".into());
    }
    if invariant.require.is_none() && invariant.forbid.is_none() && invariant.command.is_none() {
        return Err("it needs `require`, `forbid` or `command`".into());
    }
    for pattern in [&invariant.require, &invariant.forbid]
        .into_iter()
        .flatten()
    {
        regex::Regex::new(pattern).map_err(|err| format!("`{pattern}`: {err}"))?;
    }
    let paths = invariant.paths.trim();
    if paths.starts_with('/') || paths.split('/').any(|part| part == "..") {
        return Err("`paths` is a glob inside the project, like app/Http/**/*.php".into());
    }
    glob(paths).map(|_| ())
}

fn glob(paths: &str) -> Result<Option<regex::Regex>, String> {
    if paths.is_empty() || paths == "**" || paths == "**/*" {
        return Ok(None);
    }
    // `**/` any folders, `*` within a name, `?` one character, `{a,b}`.
    let mut pattern = String::from("^");
    let mut chars = paths.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    pattern.push_str("(?:.*/)?");
                } else {
                    pattern.push_str(".*");
                }
            }
            '*' => pattern.push_str("[^/]*"),
            '?' => pattern.push_str("[^/]"),
            '{' => pattern.push_str("(?:"),
            '}' => pattern.push(')'),
            ',' => pattern.push('|'),
            c => pattern.push_str(&regex::escape(&c.to_string())),
        }
    }
    // A folder covers what is in it.
    pattern.push_str("(?:/.*)?$");
    regex::Regex::new(&pattern)
        .map(Some)
        .map_err(|err| format!("`{paths}`: {err}"))
}

/// A file a turn changed: its path in the repository and the lines it added
/// (line number, text).
pub type Changed = (String, Vec<(usize, String)>);

/// What changed in the repository at `root` since `before` (a checkpoint):
/// files still there, with their added lines. Blocking.
pub fn changes(root: &Path, before: &str) -> anyhow::Result<Vec<Changed>> {
    let now = elyra_git::checkpoint::snapshot(root, "elyra: invariants")?;
    let mut changed = Vec::new();
    for path in elyra_git::undo::changed_paths(root, before, &now)? {
        if !root.join(&path).is_file() {
            continue;
        }
        let diff = elyra_git::diff_text_of(root, before, &now, &path).unwrap_or_default();
        let added = elyra_git::parse_unified_diff(&path, &diff)
            .hunks
            .into_iter()
            .flat_map(|hunk| hunk.lines)
            .filter(|line| line.kind == elyra_git::DiffLineKind::Added)
            .filter_map(|line| Some((line.new_line? as usize, line.text)))
            .collect();
        changed.push((path, added));
    }
    Ok(changed)
}

/// Check `invariants` against `changed` (repository at `root`): the names of
/// those that applied, and what broke. Blocking.
pub fn check(
    root: &Path,
    invariants: &[Invariant],
    changed: &[Changed],
) -> (Vec<String>, Vec<BrokenInvariant>) {
    let mut checked = Vec::new();
    let mut broken = Vec::new();
    for invariant in invariants {
        let Ok(glob) = glob(invariant.paths.trim()) else {
            continue;
        };
        let files: Vec<&Changed> = changed
            .iter()
            .filter(|(path, _)| glob.as_ref().is_none_or(|g| g.is_match(path)))
            .collect();
        if files.is_empty() {
            continue;
        }
        checked.push(invariant.name.clone());
        broken.extend(check_one(root, invariant, &files, true));
    }
    (checked, broken)
}

/// Where `invariant` doesn't hold in `files`; `added_only`: `forbid` looks at
/// the added lines only (else at whole files, for a new invariant).
fn check_one(
    root: &Path,
    invariant: &Invariant,
    files: &[&Changed],
    added_only: bool,
) -> Vec<BrokenInvariant> {
    let mut broken = Vec::new();
    let found = |path: Option<&str>, line: Option<usize>, detail: String| BrokenInvariant {
        name: invariant.name.clone(),
        path: path.map(str::to_string),
        line,
        detail,
        why: invariant.why.clone(),
    };
    if let Some(require) = invariant
        .require
        .as_deref()
        .and_then(|p| regex::Regex::new(p).ok())
    {
        for (path, _) in files {
            let content = std::fs::read_to_string(root.join(path)).unwrap_or_default();
            if !require.is_match(&content) {
                broken.push(found(
                    Some(path),
                    None,
                    format!("doesn't have `{}`", require.as_str()),
                ));
            }
        }
    }
    if let Some(forbid) = invariant
        .forbid
        .as_deref()
        .and_then(|p| regex::Regex::new(p).ok())
    {
        for (path, added) in files {
            let lines: Vec<(usize, String)> = if added_only {
                added.clone()
            } else {
                std::fs::read_to_string(root.join(path))
                    .unwrap_or_default()
                    .lines()
                    .enumerate()
                    .map(|(n, line)| (n + 1, line.to_string()))
                    .collect()
            };
            for (line, text) in lines.iter().filter(|(_, text)| forbid.is_match(text)) {
                broken.push(found(
                    Some(path),
                    Some(*line),
                    format!(
                        "has `{}`",
                        text.trim().chars().take(120).collect::<String>()
                    ),
                ));
            }
        }
    }
    if let Some(command) = &invariant.command {
        let list: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
        if let Err(output) = run(root, command, &list.join("\n")) {
            broken.push(found(None, None, output));
        }
    }
    broken
}

/// Run an invariant's command in `root` with `ELYRA_FILES`: Err with the end
/// of its output when it fails or takes too long.
fn run(root: &Path, command: &str, files: &str) -> Result<(), String> {
    let child = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("exec 2>&1\n{command}"))
        .current_dir(root)
        .env("ELYRA_FILES", files)
        .env("CI", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = child.map_err(|err| format!("`{command}` couldn't start: {err}"))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.read_to_end(&mut output);
        output
    });
    let started = Instant::now();
    let status = loop {
        if let Ok(Some(status)) = child.try_wait() {
            break Some(status);
        }
        if started.elapsed() > COMMAND_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let output = String::from_utf8_lossy(&reader.join().unwrap_or_default()).to_string();
    let tail = crate::checks::tail(output.trim(), 1_500);
    match status {
        Some(status) if status.success() => Ok(()),
        Some(status) => Err(format!(
            "`{command}` exited with {}{}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "a signal".into()),
            if tail.is_empty() {
                String::new()
            } else {
                format!(":\n{tail}")
            }
        )),
        None => Err(format!(
            "`{command}` took longer than {} s",
            COMMAND_LIMIT.as_secs()
        )),
    }
}

/// Where a proposed invariant doesn't hold in the code as it is: every
/// matching file in the repository at `root`. Blocking.
pub fn try_on(root: &Path, invariant: &Invariant) -> Vec<BrokenInvariant> {
    let Ok(glob) = glob(invariant.paths.trim()) else {
        return Vec::new();
    };
    let mut files: Vec<Changed> = Vec::new();
    let walk = ignore::WalkBuilder::new(root).hidden(false).build();
    for entry in walk.filter_map(Result::ok) {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let path = relative.to_string_lossy().replace('\\', "/");
        if path.starts_with(".git/") || !glob.as_ref().is_none_or(|g| g.is_match(&path)) {
            continue;
        }
        files.push((path, Vec::new()));
        if files.len() >= 5_000 {
            break;
        }
    }
    let files: Vec<&Changed> = files.iter().collect();
    if files.is_empty() {
        return Vec::new();
    }
    check_one(root, invariant, &files, false)
}

/// A file name for an invariant: its name in lower case, words joined by `-`.
pub fn file_name(invariant: &Invariant) -> String {
    let slug: String = invariant
        .name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("-");
    format!("{}.json", if slug.is_empty() { "invariant" } else { &slug })
}

/// Save an invariant in the repository at `root` (a new file; an existing
/// one with the same name gets a number). The file's path.
pub fn save(root: &Path, invariant: &Invariant) -> anyhow::Result<PathBuf> {
    validate(invariant).map_err(anyhow::Error::msg)?;
    let dir = root.join(DIR);
    std::fs::create_dir_all(&dir)?;
    let name = file_name(invariant);
    let mut path = dir.join(&name);
    let mut n = 2;
    while path.exists() {
        path = dir.join(name.replace(".json", &format!("-{n}.json")));
        n += 1;
    }
    std::fs::write(&path, serde_json::to_string_pretty(invariant)? + "\n")?;
    Ok(path)
}

/// In words, one line each: `app/Http/Api/Orders.php:12: has \`dd(\`` …
pub fn describe(broken: &[BrokenInvariant]) -> Vec<String> {
    broken
        .iter()
        .map(|b| {
            let place = match (&b.path, b.line) {
                (Some(path), Some(line)) => format!("{path}:{line}: "),
                (Some(path), None) => format!("{path}: "),
                _ => String::new(),
            };
            format!("{place}{}", b.detail)
        })
        .collect()
}

/// The message sending broken invariants back to the agent.
pub fn fix_prompt(broken: &[BrokenInvariant], attempt: u32, attempts: u32) -> String {
    let mut text = format!(
        "{FIX_START} fix the code so they hold (automatic fix {attempt} of {attempts}). They are \
         the project's own rules, checked on the files you changed; don't change them or the \
         files in {DIR}.\n"
    );
    let mut names = HashSet::new();
    for invariant in broken {
        if names.insert(&invariant.name) {
            text.push_str(&format!("\n{}", invariant.name));
            if !invariant.why.is_empty() {
                text.push_str(&format!(" ({})", invariant.why));
            }
            text.push(':');
            for line in describe(
                &broken
                    .iter()
                    .filter(|b| b.name == invariant.name)
                    .cloned()
                    .collect::<Vec<_>>(),
            ) {
                text.push_str(&format!("\n- {line}"));
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{check, file_name, fix_prompt, glob, load, save, try_on, validate};
    use elyra_core::Invariant;

    fn invariant(paths: &str) -> Invariant {
        Invariant {
            name: "API controllers scope queries to the tenant".into(),
            paths: paths.into(),
            require: Some("tenant_id".into()),
            forbid: Some(r"\bdd\(".into()),
            command: None,
            why: "customers must never see another tenant's orders".into(),
        }
    }

    #[test]
    fn globs_match_paths_in_the_repository() {
        let g = glob("app/Http/Controllers/Api/**/*.php").unwrap().unwrap();
        assert!(g.is_match("app/Http/Controllers/Api/OrderController.php"));
        assert!(g.is_match("app/Http/Controllers/Api/V2/OrderController.php"));
        assert!(!g.is_match("app/Http/Controllers/OrderController.php"));
        assert!(!g.is_match("app/Http/Controllers/Api/routes.js"));
        let g = glob("src/**/*.{ts,tsx}").unwrap().unwrap();
        assert!(g.is_match("src/a/b.tsx") && g.is_match("src/c.ts") && !g.is_match("src/c.js"));
        assert!(
            glob("app/Models")
                .unwrap()
                .unwrap()
                .is_match("app/Models/Order.php"),
            "a folder"
        );
        assert!(glob("").unwrap().is_none() && glob("**").unwrap().is_none());
    }

    #[test]
    fn says_what_is_wrong_with_one() {
        assert!(validate(&invariant("app/**")).is_ok());
        let mut nothing = invariant("app/**");
        (nothing.require, nothing.forbid) = (None, None);
        assert!(validate(&nothing).unwrap_err().contains("require"));
        let mut bad = invariant("app/**");
        bad.forbid = Some("dd(".into());
        assert!(validate(&bad).unwrap_err().contains("dd("));
        assert!(validate(&invariant("../secrets/**")).is_err());
        assert_eq!(
            file_name(&invariant("")),
            "api-controllers-scope-queries-to-the-tenant.json"
        );
    }

    #[test]
    fn checks_the_files_a_turn_changed() {
        let root = std::env::temp_dir().join(format!("elyra-invariants-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("app/Api")).unwrap();
        std::fs::write(
            root.join("app/Api/Orders.php"),
            "<?php\n$orders = Order::where('tenant_id', $tenant)->get();\n",
        )
        .unwrap();
        std::fs::write(
            root.join("app/Api/Invoices.php"),
            "<?php\n$all = Invoice::all();\ndd($all);\n",
        )
        .unwrap();
        std::fs::write(root.join("app/Old.php"), "<?php\ndd($x);\n").unwrap();
        let saved = save(&root, &invariant("app/Api/**/*.php")).unwrap();
        assert!(
            saved.ends_with(".elyra/invariants/api-controllers-scope-queries-to-the-tenant.json")
        );
        let mut command = invariant("app/**");
        command.name = "PHP files parse".into();
        (command.require, command.forbid) = (None, None);
        command.command = Some("for f in $ELYRA_FILES; do grep -q '<?php' \"$f\" || { echo \"$f: no <?php\"; exit 1; }; done".into());
        save(&root, &command).unwrap();
        std::fs::write(root.join(".elyra/invariants/broken.json"), "{ not json").unwrap();

        let (invariants, problems) = load(&root);
        assert_eq!(invariants.len(), 2);
        assert!(
            problems[0].starts_with(".elyra/invariants/broken.json"),
            "{problems:?}"
        );
        let changed = vec![
            (
                "app/Api/Orders.php".to_string(),
                vec![(2, "$orders = …".to_string())],
            ),
            (
                "app/Api/Invoices.php".to_string(),
                vec![
                    (2, "$all = Invoice::all();".into()),
                    (3, "dd($all);".into()),
                ],
            ),
            ("app/Old.php".to_string(), vec![]),
        ];
        let (checked, broken) = check(&root, &invariants, &changed);
        assert_eq!(checked.len(), 2);
        let found: Vec<(Option<&str>, Option<usize>, &str)> = broken
            .iter()
            .map(|b| (b.path.as_deref(), b.line, b.detail.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (
                    Some("app/Api/Invoices.php"),
                    None,
                    "doesn't have `tenant_id`"
                ),
                (Some("app/Api/Invoices.php"), Some(3), "has `dd($all);`"),
            ],
            "app/Old.php isn't in app/Api, and its dd( wasn't added"
        );
        let text = fix_prompt(&broken, 1, 2);
        assert!(text.starts_with(super::FIX_START));
        assert!(text.contains("(automatic fix 1 of 2)"));
        assert!(text.contains("API controllers scope queries to the tenant (customers must never see another tenant's orders):\n- app/Api/Invoices.php: doesn't have `tenant_id`\n- app/Api/Invoices.php:3: has `dd($all);`"));

        // The command gets the files, and its output when it fails.
        std::fs::write(root.join("app/Old.php"), "dd($x);\n").unwrap();
        let parse: Vec<_> = invariants
            .iter()
            .filter(|i| i.name == "PHP files parse")
            .cloned()
            .collect();
        let (_, broken) = check(&root, &parse, &[("app/Old.php".into(), vec![])]);
        assert!(
            broken[0]
                .detail
                .contains("exited with 1:\napp/Old.php: no <?php"),
            "{broken:?}"
        );

        // Proposed: tried on every matching file as it is.
        let now = try_on(&root, &invariant("app/**/*.php"));
        let places: Vec<String> = super::describe(&now);
        assert!(
            places.contains(&"app/Old.php:1: has `dd($x);`".to_string()),
            "{places:?}"
        );
        assert!(
            places.contains(&"app/Api/Invoices.php: doesn't have `tenant_id`".to_string()),
            "{places:?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
