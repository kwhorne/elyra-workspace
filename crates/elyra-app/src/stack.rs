//! What a project is built with, as far as agents care: a Laravel app with
//! or without Laravel Boost (its MCP server, guidelines, skills and
//! `.ai/rules`), and an Elyra Framework app with `rata`'s tools for agents.
//! The Context tab says what is set up and what to run; rules go where the
//! project keeps them; reviews mention the tools agents have.

use elyra_core::{ProjectId, ProviderKind, Store};
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stack {
    /// `artisan` and `laravel/framework` in composer.json.
    pub laravel: bool,
    /// `laravel/boost` in composer.json.
    pub boost: bool,
    /// Boost's MCP server is in `.mcp.json`.
    pub boost_mcp: bool,
    /// Boost keeps project rules in `.ai/rules` (on unless the app's `.env`
    /// says `BOOST_RULES_ENABLED=false`).
    pub boost_rules: bool,
    /// An Elyra Framework app (`elyra.toml`).
    pub elyra_app: bool,
    /// `rata mcp dev` is in `.mcp.json`.
    pub elyra_mcp: bool,
}

impl Stack {
    /// The tools a reviewing agent can check changes with, in a sentence.
    pub fn review_hint(&self) -> Option<String> {
        let mut tools = Vec::new();
        if self.boost_mcp {
            tools.push(
                "Laravel Boost's MCP tools (database-schema, last-error, read-log-entries, search-docs)",
            );
        }
        if self.elyra_mcp {
            tools.push("the Elyra Framework tools from `rata mcp dev` (app_info, database_schema, last_error, search_docs)");
        }
        (!tools.is_empty()).then(|| {
            format!(
                "This project has {}: use them to check the changes against the app's real schema, its recent errors and the framework's documentation instead of guessing.",
                tools.join(" and ")
            )
        })
    }
}

/// The tools hint for a reviewing agent with `provider` in the project:
/// only for tools it gets (Claude Code reads `.mcp.json` itself; the others
/// get its servers once the user allowed them).
pub fn tools_hint(
    store: &Store,
    project: ProjectId,
    dir: &Path,
    provider: ProviderKind,
) -> Option<String> {
    let mut stack = detect(dir);
    if provider != ProviderKind::Claude {
        let given: Vec<String> = crate::shared_setup::servers_for(store, project, dir)
            .into_iter()
            .map(|server| server.name)
            .collect();
        let named = |needle: &dyn Fn(&str) -> bool| {
            crate::shared_setup::mcp_json(dir)
                .and_then(|file| file.servers.ok())
                .unwrap_or_default()
                .into_iter()
                .any(|s| needle(&s.summary) && given.contains(&s.name))
        };
        stack.boost_mcp = named(&|s| s.contains("boost:mcp"));
        stack.elyra_mcp = named(&|s| s.contains("rata") && s.contains("mcp dev"));
    }
    stack.review_hint()
}

/// The framework guidelines in the instructions the agent reads, with
/// their size in tokens: (what, tokens). Blocking.
pub fn guidelines(dir: &Path, provider: ProviderKind) -> Vec<(String, u64)> {
    let file = if provider == ProviderKind::Claude {
        "CLAUDE.md"
    } else {
        "AGENTS.md"
    };
    let Ok(text) = std::fs::read_to_string(dir.join(file)) else {
        return Vec::new();
    };
    [
        ("laravel-boost-guidelines", "Laravel Boost's guidelines"),
        ("elyra-framework-guidelines", "Elyra Framework's guidelines"),
    ]
    .into_iter()
    .filter_map(|(tag, what)| {
        let start = text.find(&format!("<{tag}>"))?;
        let end = text[start..]
            .find(&format!("</{tag}>"))
            .map_or(text.len(), |end| start + end);
        Some((
            format!("{what} in {file}"),
            crate::context_report::tokens(&text[start..end]),
        ))
    })
    .collect()
}

/// What the project in `dir` is built with. Blocking (reads a few files).
pub fn detect(dir: &Path) -> Stack {
    let composer: serde_json::Value = std::fs::read_to_string(dir.join("composer.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let requires = |package: &str| {
        ["require", "require-dev"]
            .iter()
            .any(|key| composer[key].get(package).is_some())
    };
    let laravel = dir.join("artisan").is_file() && requires("laravel/framework");
    let boost = requires("laravel/boost");
    let servers: Vec<String> = crate::shared_setup::mcp_json(dir)
        .and_then(|file| file.servers.ok())
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.summary)
        .collect();
    let rules_off = std::fs::read_to_string(dir.join(".env"))
        .ok()
        .and_then(|text| {
            crate::grove::parse_env(&text)
                .get("BOOST_RULES_ENABLED")
                .cloned()
        })
        .is_some_and(|value| matches!(value.to_lowercase().as_str(), "false" | "0" | "off"));
    Stack {
        laravel,
        boost,
        boost_mcp: servers.iter().any(|s| s.contains("boost:mcp")),
        boost_rules: (boost && !rules_off) || dir.join(".ai").join("rules").is_dir(),
        elyra_app: dir.join("elyra.toml").is_file(),
        elyra_mcp: servers
            .iter()
            .any(|s| s.contains("rata") && s.contains("mcp dev")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Stack, detect};

    #[test]
    fn tells_laravel_boost_and_elyra_apart() {
        let dir = std::env::temp_dir().join(format!("elyra-stack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(detect(&dir), Stack::default());
        std::fs::write(dir.join("artisan"), "").unwrap();
        std::fs::write(
            dir.join("composer.json"),
            r#"{"require": {"laravel/framework": "^13.0"}, "require-dev": {"laravel/boost": "^2.0"}}"#,
        )
        .unwrap();
        let stack = detect(&dir);
        assert!(stack.laravel && stack.boost && stack.boost_rules && !stack.boost_mcp);
        assert_eq!(stack.review_hint(), None);
        std::fs::write(
            dir.join(".mcp.json"),
            r#"{"mcpServers": {"laravel-boost": {"command": "php", "args": ["artisan", "boost:mcp"]}}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join(".env"),
            "APP_NAME=Shop\nBOOST_RULES_ENABLED=false\n",
        )
        .unwrap();
        let stack = detect(&dir);
        assert!(stack.boost_mcp && !stack.boost_rules);
        assert!(stack.review_hint().unwrap().contains("database-schema"));
        std::fs::write(
            dir.join("AGENTS.md"),
            format!(
                "# Shop\n\n<laravel-boost-guidelines>\n{}\n</laravel-boost-guidelines>\n",
                "x".repeat(4_000)
            ),
        )
        .unwrap();
        let found = super::guidelines(&dir, elyra_core::ProviderKind::Codex);
        assert_eq!(found.len(), 1);
        assert!(found[0].0 == "Laravel Boost's guidelines in AGENTS.md" && found[0].1 > 900);
        assert!(super::guidelines(&dir, elyra_core::ProviderKind::Claude).is_empty());
        std::fs::write(dir.join("elyra.toml"), "[app]\n").unwrap();
        std::fs::write(
            dir.join(".mcp.json"),
            r#"{"mcpServers": {"elyra-framework": {"command": "rata", "args": ["mcp", "dev"]}}}"#,
        )
        .unwrap();
        let stack = detect(&dir);
        assert!(stack.elyra_app && stack.elyra_mcp && !stack.boost_mcp);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
