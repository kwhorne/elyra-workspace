//! Conventional Commits titles for commits and pull requests:
//! `type(scope): summary`, e.g. `fix(nightwatch-exceptions): employee service create`.

/// The types a title may start with.
pub const TYPES: &[&str] = &[
    "feat", "fix", "refactor", "perf", "test", "docs", "build", "ci", "chore", "style", "revert",
];

/// What an agent is told about the format.
pub const RULES: &str = "The title (for a commit message, its first line) must follow Conventional Commits: `type(scope): summary`. \
     type is one of feat, fix, refactor, perf, test, docs, build, ci, chore, style, revert \
     (feat for new behaviour, fix for a bug fix). scope names the area that changed, in lowercase \
     kebab-case, such as a module, package, service or feature (nightwatch-exceptions, billing, \
     auth-api). summary is a short lowercase phrase without a trailing period. The whole title \
     is at most 72 characters. Example: fix(nightwatch-exceptions): employee service create";

/// The example shown to the user.
pub const EXAMPLE: &str = "fix(nightwatch-exceptions): employee service create";

/// Tidy a title an agent wrote: drop a `TITLE:` label, quotes and a trailing
/// period, lowercase the type, and make the scope kebab-case.
pub fn normalize(raw: &str) -> String {
    let mut title = raw.trim();
    for label in ["TITLE:", "Title:", "title:"] {
        title = title.strip_prefix(label).unwrap_or(title).trim();
    }
    let title = title
        .trim_matches(|c| c == '`' || c == '"' || c == '\'')
        .trim()
        .trim_end_matches('.')
        .trim();
    let Some((head, summary)) = title.split_once(':') else {
        return title.to_string();
    };
    let head = head.trim();
    // `feat!:` or `feat(api)!:` marks a breaking change.
    let breaking = head.ends_with('!');
    let head = head.trim_end_matches('!');
    let (kind, scope) = match head.split_once('(') {
        Some((kind, rest)) => (kind, Some(rest.trim_end_matches(')'))),
        None => (head, None),
    };
    let kind = kind.trim().to_lowercase();
    let scope = scope.map(|scope| {
        scope
            .trim()
            .to_lowercase()
            .split(|c: char| c.is_whitespace() || c == '_')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    });
    let mut summary = summary.trim().to_string();
    // Lowercase the first word unless it looks like a name (NightwatchService, API).
    if let Some(first) = summary.split_whitespace().next()
        && first.chars().skip(1).all(|c| !c.is_uppercase())
    {
        let mut chars = summary.chars();
        if let Some(c) = chars.next() {
            summary = c.to_lowercase().chain(chars).collect();
        }
    }
    let bang = if breaking { "!" } else { "" };
    match scope.filter(|s| !s.is_empty()) {
        Some(scope) => format!("{kind}({scope}){bang}: {summary}"),
        None => format!("{kind}{bang}: {summary}"),
    }
}

/// `normalize` for the first line of a commit message; the body is kept.
pub fn normalize_message(message: &str) -> String {
    let message = message.trim().trim_matches('`').trim();
    match message.split_once('\n') {
        Some((subject, body)) => format!("{}\n{}", normalize(subject), body.trim_end()),
        None => normalize(message),
    }
}

/// Why a title doesn't follow the format, or `None` when it does.
pub fn problem(title: &str) -> Option<String> {
    let title = title.trim();
    let Some((head, summary)) = title.split_once(": ") else {
        return Some(format!("Write it as type(scope): summary, e.g. {EXAMPLE}"));
    };
    let head = head.trim_end_matches('!');
    let Some((kind, scope)) = head.split_once('(') else {
        return Some(format!("Add a scope in parentheses, e.g. {EXAMPLE}"));
    };
    if !TYPES.contains(&kind) {
        return Some(format!(
            "\u{201c}{kind}\u{201d} isn't a known type; use one of {}",
            TYPES.join(", ")
        ));
    }
    let scope = scope.strip_suffix(')').unwrap_or("");
    let kebab = !scope.is_empty()
        && scope
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '/');
    if !kebab {
        return Some("The scope should be lowercase kebab-case, e.g. nightwatch-exceptions".into());
    }
    if summary.trim().is_empty() {
        return Some("Add a short summary after the colon".into());
    }
    if title.chars().count() > 72 {
        return Some(format!(
            "Keep it at most 72 characters (this one has {})",
            title.chars().count()
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{normalize, problem};

    #[test]
    fn tidies_what_an_agent_writes() {
        assert_eq!(
            normalize("TITLE: Fix(Nightwatch Exceptions): Employee service create."),
            "fix(nightwatch-exceptions): employee service create"
        );
        assert_eq!(
            normalize("`feat(billing_api): Add invoice export`"),
            "feat(billing-api): add invoice export"
        );
        // Names keep their capitals.
        assert_eq!(
            normalize("fix(auth): OAuth refresh loops"),
            "fix(auth): OAuth refresh loops"
        );
        assert_eq!(normalize("feat(api)!: drop v1"), "feat(api)!: drop v1");
    }

    #[test]
    fn keeps_a_commit_body() {
        assert_eq!(
            super::normalize_message(
                "```\nFix(Nightwatch): Employee service create.\n\nThe service threw on empty input.\n```"
            ),
            "fix(nightwatch): employee service create\n\nThe service threw on empty input."
        );
    }

    #[test]
    fn checks_the_format() {
        assert_eq!(
            problem("fix(nightwatch-exceptions): employee service create"),
            None
        );
        assert_eq!(problem("feat(api)!: drop v1"), None);
        assert!(problem("Employee service create").is_some());
        assert!(problem("fix: employee service create").is_some());
        assert!(
            problem("bugfix(api): x")
                .unwrap()
                .contains("isn't a known type")
        );
        assert!(
            problem("fix(Nightwatch): x")
                .unwrap()
                .contains("kebab-case")
        );
        assert!(
            problem(&format!("fix(a): {}", "x".repeat(80)))
                .unwrap()
                .contains("72")
        );
    }
}
