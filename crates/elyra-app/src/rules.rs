//! Rules an agent learned from the user's corrections ("no, we always use
//! Form Requests"): proposed as a card in the thread, and on the user's word
//! added to the repository's agent instructions (AGENTS.md, or CLAUDE.md when
//! that is what the project uses) or to the project's instructions in Elyra.

use std::path::{Path, PathBuf};

/// The section learned rules go under.
const SECTION: &str = "## Learned rules";

/// The instructions file to add rules to: AGENTS.md, else CLAUDE.md, else a
/// new AGENTS.md.
pub fn instructions_file(dir: &Path) -> PathBuf {
    for name in ["AGENTS.md", "CLAUDE.md"] {
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

/// Add a rule to the project's instructions file; returns the file.
pub fn add_to_file(dir: &Path, rule: &str) -> Result<PathBuf, String> {
    let path = instructions_file(dir);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, with_rule(&text, rule)).map_err(|err| err.to_string())?;
    Ok(path)
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
    use super::{instructions_file, with_instruction, with_rule};

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
        std::fs::write(dir.join("CLAUDE.md"), "").unwrap();
        assert!(instructions_file(&dir).ends_with("CLAUDE.md"));
        std::fs::write(dir.join("AGENTS.md"), "").unwrap();
        assert!(instructions_file(&dir).ends_with("AGENTS.md"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(with_instruction(None, "A."), "- A.");
        assert_eq!(with_instruction(Some("Be brief."), "A."), "Be brief.\n- A.");
        assert_eq!(with_instruction(Some("- A."), "A."), "- A.");
    }
}
