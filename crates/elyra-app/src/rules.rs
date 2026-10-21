//! Rules an agent learned from the user's corrections ("no, we always use
//! Form Requests"): proposed as a card in the thread, and on the user's word
//! added to the repository's agent instructions (AGENTS.md, or CLAUDE.md or
//! GEMINI.md when that is what the project uses) or to the project's
//! instructions in Elyra. A project with Laravel Boost's project rules keeps
//! them in `.ai/rules` instead, one file per area with the globs it applies
//! to, and the index Boost tells agents to read — so there is one rule book,
//! shared with the team, whichever tool wrote the rule.

use std::path::{Path, PathBuf};

/// The section learned rules go under.
const SECTION: &str = "## Learned rules";

/// The instructions file to add rules to: AGENTS.md, else CLAUDE.md, else
/// GEMINI.md, else a new AGENTS.md.
pub fn instructions_file(dir: &Path) -> PathBuf {
    for name in ["AGENTS.md", "CLAUDE.md", "GEMINI.md"] {
        let path = dir.join(name);
        if path.is_file() {
            return path;
        }
    }
    dir.join("AGENTS.md")
}

/// `text` with `rule` added as a bullet at the end of the learned-rules
/// section (made at the end when missing). An identical rule isn't repeated.
pub fn with_rule(text: &str, rule: &str) -> String {
    let rule = rule.trim().trim_start_matches("- ").trim();
    let bullet = format!("- {rule}");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let Some(start) = lines.iter().position(|l| l.trim() == SECTION) else {
        let mut out = text.trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&format!("{SECTION}\n\n{bullet}\n"));
        return out;
    };
    // The section ends at the next heading of the same or a higher level.
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("# ") || l.starts_with("## "))
        .map_or(lines.len(), |offset| start + 1 + offset);
    if lines[start..end].iter().any(|l| l.trim() == bullet) {
        return text.to_string();
    }
    // After the section's last non-empty line.
    let last = (start..end)
        .rev()
        .find(|&i| !lines[i].trim().is_empty())
        .unwrap_or(start);
    let at = if last == start { start + 1 } else { last + 1 };
    if last == start {
        lines.insert(at, String::new());
        lines.insert(at + 1, bullet);
    } else {
        lines.insert(at, bullet);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Where a rule goes: Boost's `.ai/rules/<area>.md` in a project that keeps
/// rules there, otherwise the instructions file.
pub fn destination(dir: &Path, paths: Option<&str>) -> PathBuf {
    if crate::stack::detect(dir).boost_rules {
        dir.join(BOOST_RULES).join(format!("{}.md", area(paths)))
    } else {
        instructions_file(dir)
    }
}

/// Add a rule to the project's rules; returns the file.
pub fn add_to_file(dir: &Path, rule: &str, paths: Option<&str>) -> Result<PathBuf, String> {
    let path = destination(dir, paths);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if path.starts_with(dir.join(BOOST_RULES)) {
        std::fs::create_dir_all(dir.join(BOOST_RULES)).map_err(|err| err.to_string())?;
        let title = title_of(&area(paths));
        std::fs::write(&path, with_boost_rule(&text, &title, paths, rule))
            .map_err(|err| err.to_string())?;
        write_index(&dir.join(BOOST_RULES)).map_err(|err| err.to_string())?;
    } else {
        std::fs::write(&path, with_rule(&text, rule)).map_err(|err| err.to_string())?;
    }
    Ok(path)
}

// ---- Boost's .ai/rules ---------------------------------------------------------

const BOOST_RULES: &str = ".ai/rules";
const ALL: &str = "**";

/// The file name for rules about `paths`: its last plain folder or file
/// name (`app/Http/Controllers/**` → `controllers`), or `general`.
fn area(paths: Option<&str>) -> String {
    let name = paths
        .and_then(|glob| {
            glob.split('/')
                .rfind(|part| !part.is_empty() && !part.contains(['*', '{', '[', '?']))
        })
        .map(|part| part.split('.').next().unwrap_or(part))
        .unwrap_or("");
    let mut slug = String::new();
    let mut previous_lower = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && previous_lower {
                slug.push('-');
            }
            previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
            slug.push(c.to_ascii_lowercase());
        } else {
            if !slug.ends_with('-') {
                slug.push('-');
            }
            previous_lower = false;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() || slug == "index" {
        "general".into()
    } else {
        slug
    }
}

fn title_of(area: &str) -> String {
    let words = area.replace('-', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

/// The globs in a Boost rule file's frontmatter.
pub fn globs_of(text: &str) -> Vec<String> {
    globs(text)
}

/// The globs in a rule file's frontmatter (`paths:` list).
fn globs(text: &str) -> Vec<String> {
    let Some(rest) = text.strip_prefix("---\n") else {
        return Vec::new();
    };
    let front = rest.split("\n---").next().unwrap_or("");
    let mut in_paths = false;
    let mut out = Vec::new();
    for line in front.lines() {
        if line.trim_end() == "paths:" {
            in_paths = true;
        } else if in_paths && let Some(glob) = line.trim().strip_prefix("- ") {
            out.push(glob.trim().trim_matches(['"', '\'']).to_string());
        } else if !line.starts_with(' ') {
            in_paths = false;
        }
    }
    out
}

/// A Boost rule file with `rule` added: made with its frontmatter and title
/// when new, its globs extended, an identical rule not repeated.
fn with_boost_rule(text: &str, title: &str, paths: Option<&str>, rule: &str) -> String {
    let rule = rule.trim().trim_start_matches("- ").trim();
    let bullet = format!("- {rule}");
    let glob = paths.unwrap_or(ALL);
    if text.trim().is_empty() {
        return format!("---\npaths:\n  - {glob}\n---\n\n# {title}\n\n{bullet}\n");
    }
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if !globs(text).iter().any(|g| g == glob) {
        let at = lines
            .iter()
            .position(|l| l.trim_end() == "paths:")
            .map(|start| {
                start
                    + 1
                    + lines[start + 1..]
                        .iter()
                        .take_while(|l| l.trim().starts_with("- "))
                        .count()
            });
        match at {
            Some(at) => lines.insert(at, format!("  - {glob}")),
            None if text.starts_with("---\n") => {
                lines.insert(1, "paths:".into());
                lines.insert(2, format!("  - {glob}"));
            }
            None => {
                lines.splice(
                    0..0,
                    [
                        "---".into(),
                        "paths:".into(),
                        format!("  - {glob}"),
                        "---".into(),
                        String::new(),
                    ],
                );
            }
        }
    }
    if lines.iter().any(|l| l.trim() == bullet) {
        let mut out = lines.join("\n");
        out.push('\n');
        return out;
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    if lines
        .last()
        .is_some_and(|l| !l.trim_start().starts_with("- "))
    {
        lines.push(String::new());
    }
    lines.push(bullet);
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Boost's `.ai/rules/index.md`, from the rule files' globs.
fn index(files: &[(String, Vec<String>)]) -> String {
    let mut out = String::from(
        "# Project Rules Index\n\nBefore planning or editing, find the row whose globs match the file's path and read that rule file.\n\n| Applies to | Rule file |\n| --- | --- |\n",
    );
    for (name, globs) in files {
        let applies = if globs.is_empty() {
            ALL.to_string()
        } else {
            globs.join(", ")
        };
        out.push_str(&format!("| {applies} | {BOOST_RULES}/{name} |\n"));
    }
    out
}

fn write_index(rules: &Path) -> std::io::Result<()> {
    let mut files: Vec<(String, Vec<String>)> = std::fs::read_dir(rules)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            (name.ends_with(".md") && name != "index.md").then(|| {
                let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
                (name, globs(&text))
            })
        })
        .collect();
    files.sort();
    std::fs::write(rules.join("index.md"), index(&files))
}

/// Elyra's project instructions with the rule added.
pub fn with_instruction(instructions: Option<&str>, rule: &str) -> String {
    let rule = rule.trim().trim_start_matches("- ").trim();
    match instructions.map(str::trim).filter(|i| !i.is_empty()) {
        Some(current) if current.lines().any(|l| l.trim() == format!("- {rule}")) => {
            current.to_string()
        }
        Some(current) => format!("{current}\n- {rule}"),
        None => format!("- {rule}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        area, globs, index, instructions_file, with_boost_rule, with_instruction, with_rule,
    };

    #[test]
    fn adds_rules_under_their_own_section() {
        assert_eq!(
            with_rule("", "Use Form Requests for validation."),
            "## Learned rules\n\n- Use Form Requests for validation.\n"
        );
        let doc = "# Agents\n\nBe brief.\n";
        let once = with_rule(doc, "Use Pest, not PHPUnit.");
        assert_eq!(
            once,
            "# Agents\n\nBe brief.\n\n## Learned rules\n\n- Use Pest, not PHPUnit.\n"
        );
        let twice = with_rule(&once, "- Never edit migrations that ran.");
        assert_eq!(
            twice,
            "# Agents\n\nBe brief.\n\n## Learned rules\n\n- Use Pest, not PHPUnit.\n- Never edit migrations that ran.\n"
        );
        assert_eq!(
            with_rule(&twice, "Use Pest, not PHPUnit."),
            twice,
            "no duplicates"
        );
        // A section in the middle keeps what follows it.
        let middle = "## Learned rules\n\n- One.\n\n## Verify\n\nRun tests.\n";
        assert_eq!(
            with_rule(middle, "Two."),
            "## Learned rules\n\n- One.\n- Two.\n\n## Verify\n\nRun tests.\n"
        );
    }

    #[test]
    fn picks_the_file_the_project_uses() {
        let dir = std::env::temp_dir().join(format!("elyra-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(instructions_file(&dir).ends_with("AGENTS.md"));
        std::fs::write(dir.join("GEMINI.md"), "").unwrap();
        assert!(instructions_file(&dir).ends_with("GEMINI.md"));
        std::fs::write(dir.join("CLAUDE.md"), "").unwrap();
        assert!(instructions_file(&dir).ends_with("CLAUDE.md"));
        std::fs::write(dir.join("AGENTS.md"), "").unwrap();
        assert!(instructions_file(&dir).ends_with("AGENTS.md"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(with_instruction(None, "A."), "- A.");
        assert_eq!(with_instruction(Some("Be brief."), "A."), "Be brief.\n- A.");
        assert_eq!(with_instruction(Some("- A."), "A."), "- A.");
    }

    #[test]
    fn keeps_rules_in_boosts_rule_files() {
        assert_eq!(area(Some("app/Http/Controllers/**")), "controllers");
        assert_eq!(area(Some("app/Models/*.php")), "models");
        assert_eq!(area(Some("resources/views/**/*.blade.php")), "views");
        assert_eq!(area(Some("app/HttpKernel.php")), "http-kernel");
        assert_eq!(area(None), "general");
        assert_eq!(area(Some("**/*.php")), "general");

        let new = with_boost_rule(
            "",
            "Controllers",
            Some("app/Http/Controllers/**"),
            "Validate with Form Requests.",
        );
        assert_eq!(
            new,
            "---\npaths:\n  - app/Http/Controllers/**\n---\n\n# Controllers\n\n- Validate with Form Requests.\n"
        );
        assert_eq!(globs(&new), ["app/Http/Controllers/**"]);
        let more = with_boost_rule(&new, "Controllers", Some("routes/**"), "Name every route.");
        assert_eq!(globs(&more), ["app/Http/Controllers/**", "routes/**"]);
        assert!(more.ends_with("- Validate with Form Requests.\n- Name every route.\n"));
        assert_eq!(
            with_boost_rule(&more, "Controllers", Some("routes/**"), "Name every route."),
            more,
            "no duplicates"
        );
        // A file Boost wrote, with a heading and a note.
        let boost = "---\npaths:\n  - app/Models/**\n---\n\n# Models\n\n## Money in cents\n\nStore money as integer cents.\n";
        assert!(
            with_boost_rule(boost, "Models", Some("app/Models/**"), "Use casts.")
                .ends_with("Store money as integer cents.\n\n- Use casts.\n")
        );
        assert_eq!(
            index(&[
                (
                    "controllers.md".into(),
                    vec!["app/Http/Controllers/**".into(), "routes/**".into()]
                ),
                ("general.md".into(), vec![]),
            ]),
            "# Project Rules Index\n\nBefore planning or editing, find the row whose globs match the file's path and read that rule file.\n\n| Applies to | Rule file |\n| --- | --- |\n| app/Http/Controllers/**, routes/** | .ai/rules/controllers.md |\n| ** | .ai/rules/general.md |\n"
        );
    }

    #[test]
    fn writes_rules_where_the_project_keeps_them() {
        let dir = std::env::temp_dir().join(format!("elyra-boost-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ai/rules")).unwrap();
        let path = super::add_to_file(&dir, "Use Pest.", Some("tests/**")).unwrap();
        assert!(path.ends_with(".ai/rules/tests.md"));
        let index = std::fs::read_to_string(dir.join(".ai/rules/index.md")).unwrap();
        assert!(
            index.contains("| tests/** | .ai/rules/tests.md |"),
            "{index}"
        );
        assert!(!dir.join("AGENTS.md").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
