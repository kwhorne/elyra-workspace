//! Rules that went stale: the rules in AGENTS.md, CLAUDE.md, GEMINI.md and
//! Laravel Boost's `.ai/rules` name files, folders and symbols (in
//! backticks). When what a rule names is gone from the repository (a path no
//! tracked file has, a symbol no file mentions any more, a glob that matches
//! nothing), the rule probably is too: the Context tab lists it, with what is
//! missing, to fix or remove it. Generated framework guidelines (Boost's and
//! Elyra Framework's blocks) and code blocks are left alone.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

/// A rule as written: its file (relative to the project), its line (1-based)
/// and its text.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub file: String,
    pub line: usize,
    pub text: String,
}

/// A rule naming something that isn't there any more.
#[derive(Clone, Debug, PartialEq)]
pub struct Stale {
    pub rule: Rule,
    pub missing: Vec<String>,
    /// Boost rule files: the globs in the frontmatter (removing the line
    /// doesn't apply).
    pub frontmatter: bool,
}

const FILES: &[&str] = &["AGENTS.md", "CLAUDE.md", "GEMINI.md"];
const GENERATED: &[&str] = &["laravel-boost-guidelines", "elyra-framework-guidelines"];

/// The bullet rules in a Markdown file, outside code and generated blocks.
fn bullets(file: &str, text: &str) -> Vec<Rule> {
    let mut out = Vec::new();
    let (mut fence, mut generated) = (false, false);
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            fence = !fence;
            continue;
        }
        if GENERATED
            .iter()
            .any(|tag| trimmed.starts_with(&format!("<{tag}>")))
        {
            generated = true;
        }
        if GENERATED
            .iter()
            .any(|tag| trimmed.starts_with(&format!("</{tag}>")))
        {
            generated = false;
            continue;
        }
        if fence || generated {
            continue;
        }
        if let Some(rule) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            out.push(Rule {
                file: file.to_string(),
                line: index + 1,
                text: rule.to_string(),
            });
        }
    }
    out
}

/// What a rule names in backticks: (paths, symbols).
fn names(rule: &str) -> (Vec<String>, Vec<String>) {
    let (mut paths, mut symbols) = (Vec::new(), Vec::new());
    for (index, part) in rule.split('`').enumerate() {
        if index % 2 == 0 {
            continue;
        }
        let part = part.trim();
        if part.is_empty()
            || part.contains(char::is_whitespace)
            || part.contains(['=', '"', '\'', '<', '>', '$', '@'])
            || part.starts_with('-')
            // Outside the repository: the home folder, absolute paths.
            || part.starts_with(['~', '/'])
            || part.contains("://")
        {
            continue;
        }
        let looks_like_path = part.contains('/')
            || part
                .rsplit_once('.')
                .is_some_and(|(stem, ext)| !stem.is_empty() && is_extension(ext));
        if looks_like_path {
            paths.push(
                part.trim_start_matches("./")
                    .trim_end_matches('/')
                    .to_string(),
            );
            continue;
        }
        // The last segment of `Foo::bar`, `$x->bar()`, `obj.bar`.
        let symbol = part
            .trim_end_matches("()")
            .rsplit([':', '.', '>', '\\'])
            .next()
            .unwrap_or("");
        let identifier = symbol.len() >= 4
            && symbol.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
            && symbol.chars().all(|c| c.is_alphanumeric() || c == '_')
            // A word, not a name: has an uppercase letter after the first, a
            // `_` or comes from a qualified name.
            && (symbol.chars().skip(1).any(char::is_uppercase)
                || symbol.contains('_')
                || symbol.len() < part.len());
        if identifier && !matches!(symbol, "true" | "false" | "null" | "None" | "Some") {
            symbols.push(symbol.to_string());
        }
    }
    (paths, symbols)
}

fn is_extension(ext: &str) -> bool {
    matches!(
        ext,
        "rs" | "php"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "vue"
            | "svelte"
            | "py"
            | "go"
            | "rb"
            | "md"
            | "json"
            | "toml"
            | "yml"
            | "yaml"
            | "sql"
            | "sh"
            | "css"
            | "html"
            | "lock"
            | "env"
            | "xml"
            | "c"
            | "h"
            | "swift"
            | "kt"
            | "java"
            | "cs"
    )
}

/// `glob` (with `*`, `**`, `?`) against a path.
fn glob_matches(glob: &str, path: &str) -> bool {
    fn go(g: &[u8], p: &[u8]) -> bool {
        match (g.first(), p.first()) {
            (None, None) => true,
            (Some(b'*'), _) if g.get(1) == Some(&b'*') => {
                let rest = g[2..].strip_prefix(b"/").unwrap_or(&g[2..]);
                (0..=p.len()).any(|i| go(rest, &p[i..]))
            }
            (Some(b'*'), _) => (0..=p.len())
                .take_while(|&i| i == 0 || p[i - 1] != b'/')
                .any(|i| go(&g[1..], &p[i..])),
            (Some(b'?'), Some(c)) if *c != b'/' => go(&g[1..], &p[1..]),
            (Some(a), Some(b)) if a == b => go(&g[1..], &p[1..]),
            _ => false,
        }
    }
    go(glob.as_bytes(), path.as_bytes())
}

/// Whether a named path is in the repository: a tracked file, a folder of
/// tracked files, a glob matching one, or a bare file name anywhere.
fn path_exists(name: &str, tracked: &[String], dir: &Path) -> bool {
    if name.contains(['*', '?']) {
        return tracked.iter().any(|t| glob_matches(name, t));
    }
    dir.join(name).exists()
        || tracked.iter().any(|t| {
            t == name || t.starts_with(&format!("{name}/")) || t.ends_with(&format!("/{name}"))
        })
}

/// The project's rules that name something gone. Blocking (reads the rule
/// files and asks git).
pub fn check(dir: &Path) -> Vec<Stale> {
    let tracked: Vec<String> = Command::new("git")
        .current_dir(dir)
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if tracked.is_empty() {
        return Vec::new();
    }
    let mut rules: Vec<(Rule, bool)> = Vec::new();
    for file in FILES {
        if let Ok(text) = std::fs::read_to_string(dir.join(file)) {
            rules.extend(bullets(file, &text).into_iter().map(|r| (r, false)));
        }
    }
    let boost = dir.join(".ai").join("rules");
    if let Ok(entries) = std::fs::read_dir(&boost) {
        let mut files: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        files.sort();
        for path in files {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if !name.ends_with(".md") || name == "index.md" {
                continue;
            }
            let file = format!(".ai/rules/{name}");
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let globs = crate::rules::globs_of(&text);
            if !globs.is_empty() {
                rules.push((
                    Rule {
                        file: file.clone(),
                        line: 1,
                        text: globs
                            .iter()
                            .map(|g| format!("`{g}`"))
                            .collect::<Vec<_>>()
                            .join(", "),
                    },
                    true,
                ));
            }
            rules.extend(bullets(&file, &text).into_iter().map(|r| (r, false)));
        }
    }
    let mut seen_symbols: std::collections::HashMap<String, bool> = Default::default();
    let mut out = Vec::new();
    for (rule, frontmatter) in rules {
        let (paths, symbols) = names(&rule.text);
        let mut missing: Vec<String> = paths
            .into_iter()
            .filter(|p| !path_exists(p, &tracked, dir))
            .collect();
        let unique: HashSet<String> = symbols.into_iter().collect();
        let mut unique: Vec<String> = unique.into_iter().collect();
        unique.sort();
        for symbol in unique {
            let found = *seen_symbols
                .entry(symbol.clone())
                .or_insert_with(|| mentioned(dir, &symbol, &rule.file));
            if !found {
                missing.push(symbol);
            }
        }
        if !missing.is_empty() {
            out.push(Stale {
                rule,
                missing,
                frontmatter,
            });
        }
    }
    out
}

/// Whether any file other than the rules mentions `symbol` as a word.
fn mentioned(dir: &Path, symbol: &str, rules_file: &str) -> bool {
    let mut args = vec![
        "grep".to_string(),
        "-q".into(),
        "-w".into(),
        "-F".into(),
        "--untracked".into(),
        symbol.into(),
        "--".into(),
        ".".into(),
    ];
    for file in FILES.iter().copied().chain([rules_file]) {
        args.push(format!(":(exclude){file}"));
    }
    args.push(":(exclude).ai/rules".into());
    Command::new("git")
        .current_dir(dir)
        .args(&args)
        .status()
        .map(|s| s.success())
        // Can't tell: not stale.
        .unwrap_or(true)
}

/// Take a rule's line out of its file.
pub fn remove(dir: &Path, rule: &Rule) -> std::io::Result<()> {
    let path = dir.join(&rule.file);
    let text = std::fs::read_to_string(&path)?;
    let mut lines: Vec<&str> = text.lines().collect();
    let index = rule.line - 1;
    // Only if it's still that line.
    if lines
        .get(index)
        .is_some_and(|l| l.contains(rule.text.as_str()))
    {
        lines.remove(index);
        let mut out = lines.join("\n");
        if text.ends_with('\n') {
            out.push('\n');
        }
        std::fs::write(path, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{bullets, check, glob_matches, names, remove};

    #[test]
    fn reads_what_a_rule_names() {
        let (paths, symbols) = names(
            "Validate in `app/Http/Requests` with `StoreOrderRequest`, see `AGENTS.md`; run `cargo test --workspace`, never `true`; use `Cart::total()` and `round`, not `~/.elyra` or `/tmp/x`.",
        );
        assert_eq!(paths, ["app/Http/Requests", "AGENTS.md"]);
        assert_eq!(symbols, ["StoreOrderRequest", "total"]);
        assert!(glob_matches(
            "app/Http/Controllers/**",
            "app/Http/Controllers/Api/CartController.php"
        ));
        assert!(glob_matches("app/Models/*.php", "app/Models/Order.php"));
        assert!(!glob_matches(
            "app/Models/*.php",
            "app/Models/Sub/Order.php"
        ));
        let text = "# A\n\n- One `x/y.rs`.\n```\n- not a rule\n```\n<laravel-boost-guidelines>\n- generated\n</laravel-boost-guidelines>\n* Two.\n";
        let rules = bullets("AGENTS.md", text);
        assert_eq!(rules.len(), 2);
        assert_eq!((rules[0].line, rules[1].text.as_str()), (3, "Two."));
    }

    #[test]
    fn finds_rules_that_name_what_is_gone() {
        let dir = std::env::temp_dir().join(format!("elyra-rules-health-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app/Services")).unwrap();
        std::fs::create_dir_all(dir.join(".ai/rules")).unwrap();
        std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(&dir)
            .output()
            .unwrap();
        std::fs::write(
            dir.join("app/Services/TaxService.php"),
            "<?php class TaxService {}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("AGENTS.md"),
            "# Shop\n\n## Learned rules\n\n- Put VAT in `TaxService`.\n- Money goes through `MoneyService` in `app/Services/MoneyService.php`.\n- Be brief.\n",
        )
        .unwrap();
        std::fs::write(
            dir.join(".ai/rules/controllers.md"),
            "---\npaths:\n  - app/Http/Controllers/**\n---\n\n# Controllers\n\n- Use Form Requests.\n",
        )
        .unwrap();
        let stale = check(&dir);
        assert_eq!(stale.len(), 2, "{stale:#?}");
        assert_eq!(stale[0].rule.line, 6);
        assert_eq!(
            stale[0].missing,
            ["app/Services/MoneyService.php", "MoneyService"]
        );
        assert!(stale[1].frontmatter && stale[1].missing == ["app/Http/Controllers/**"]);
        remove(&dir, &stale[0].rule).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("AGENTS.md")).unwrap(),
            "# Shop\n\n## Learned rules\n\n- Put VAT in `TaxService`.\n- Be brief.\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Against a real project: `ELYRA_RULES_DIR=<dir> cargo test -p elyra-app live_rules -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_rules() {
        let dir = std::env::var("ELYRA_RULES_DIR").expect("ELYRA_RULES_DIR");
        for stale in check(std::path::Path::new(&dir)) {
            println!(
                "{}:{} {:?} — {}",
                stale.rule.file, stale.rule.line, stale.missing, stale.rule.text
            );
        }
    }
}
