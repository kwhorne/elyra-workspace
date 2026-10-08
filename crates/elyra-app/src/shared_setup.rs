//! One setup for every agent. The MCP servers a project lists in `.mcp.json`
//! (Claude Code's format) and the skills in `.claude/skills` and
//! `.agents/skills` (and the user's own under `~/.claude/skills` and
//! `~/.agents/skills`) reach the other agents too: Codex, Elyra, Pi and ACP
//! agents. Claude Code reads both itself.
//!
//! `.mcp.json` comes with the repository and starts commands, so its servers
//! are only passed on once the user allowed them for the project (Context
//! tab); when the file changes, it has to be allowed again. Skills are only
//! listed (name, description, path) for the agent to read when they apply.

use elyra_core::{ProjectId, Store};
use elyra_provider::McpServer;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A server from `.mcp.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectServer {
    pub name: String,
    /// The command line or address, for the user to judge.
    pub summary: String,
    pub server: McpServer,
}

/// A project's `.mcp.json`: its fingerprint and servers, or why it can't be read.
#[derive(Clone, Debug, PartialEq)]
pub struct McpJson {
    pub fingerprint: String,
    pub servers: Result<Vec<ProjectServer>, String>,
}

pub fn mcp_json(project_dir: &Path) -> Option<McpJson> {
    let text = std::fs::read_to_string(project_dir.join(".mcp.json")).ok()?;
    Some(McpJson {
        fingerprint: fingerprint(&text),
        servers: parse_mcp_json(&text),
    })
}

/// FNV-1a: stays the same across Rust versions (it is stored).
fn fingerprint(text: &str) -> String {
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

fn parse_mcp_json(text: &str) -> Result<Vec<ProjectServer>, String> {
    let value: Value = serde_json::from_str(text).map_err(|err| format!(".mcp.json: {err}"))?;
    let Some(servers) = value["mcpServers"].as_object() else {
        return Err(".mcp.json has no \"mcpServers\"".into());
    };
    let strings = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .map(|items| items.iter().filter_map(Value::as_str).map(expand).collect())
            .unwrap_or_default()
    };
    let pairs = |value: &Value| -> Vec<(String, String)> {
        value
            .as_object()
            .map(|map| {
                map.iter()
                    .filter_map(|(key, value)| Some((key.clone(), expand(value.as_str()?))))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for (name, spec) in servers {
        if spec["disabled"].as_bool() == Some(true) {
            continue;
        }
        if let Some(command) = spec["command"].as_str() {
            let command = expand(command);
            let args = strings(&spec["args"]);
            let mut server = McpServer::stdio(name, PathBuf::from(&command), args.clone());
            server.env = pairs(&spec["env"]);
            out.push(ProjectServer {
                name: name.clone(),
                summary: std::iter::once(command)
                    .chain(args)
                    .collect::<Vec<_>>()
                    .join(" "),
                server,
            });
        } else if let Some(url) = spec["url"].as_str() {
            let url = expand(url);
            out.push(ProjectServer {
                name: name.clone(),
                summary: url.clone(),
                server: McpServer::http(name, url, pairs(&spec["headers"])),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// `${VAR}` and `${VAR:-default}` from the environment, as Claude Code does.
fn expand(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &rest[start + 2..start + end];
        let (name, default) = match inner.split_once(":-") {
            Some((name, default)) => (name, Some(default)),
            None => (inner, None),
        };
        match std::env::var(name).ok().filter(|v| !v.is_empty()) {
            Some(value) => out.push_str(&value),
            None => out.push_str(default.unwrap_or("")),
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

// ---- allowing .mcp.json -------------------------------------------------------

fn allowed_key(project: ProjectId) -> String {
    format!("mcp_json_allowed:{project}")
}

/// The fingerprint of the `.mcp.json` the user allowed, if any.
pub fn allowed(store: &Store, project: ProjectId) -> Option<String> {
    store.setting(&allowed_key(project)).ok().flatten()
}

pub fn allow(store: &Store, project: ProjectId, fingerprint: &str) {
    if let Err(err) = store.set_setting(&allowed_key(project), fingerprint) {
        log::warn!("allowing .mcp.json: {err:#}");
    }
}

pub fn revoke(store: &Store, project: ProjectId) {
    if let Err(err) = store.set_setting(&allowed_key(project), "") {
        log::warn!("revoking .mcp.json: {err:#}");
    }
}

/// The `.mcp.json` servers to give agents other than Claude Code: only when
/// the file is the one the user allowed.
pub fn servers_for(store: &Store, project: ProjectId, project_dir: &Path) -> Vec<McpServer> {
    let Some(file) = mcp_json(project_dir) else {
        return Vec::new();
    };
    if allowed(store, project).as_deref() != Some(file.fingerprint.as_str()) {
        return Vec::new();
    }
    file.servers
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.server)
        .collect()
}

// ---- skills -------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

/// The project's skills, then the user's own (a project skill wins a name).
pub fn skills(project_dir: &Path) -> Vec<Skill> {
    let mut roots = vec![
        project_dir.join(".claude/skills"),
        project_dir.join(".agents/skills"),
    ];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".claude/skills"));
        roots.push(home.join(".agents/skills"));
    }
    let mut found: Vec<Skill> = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path().join("SKILL.md"))
            .filter(|p| p.is_file())
            .collect();
        paths.sort();
        for path in paths {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let folder = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (name, description) = frontmatter(&text);
            let name = name.unwrap_or(folder);
            if found.iter().any(|s| s.name == name) {
                continue;
            }
            found.push(Skill {
                name,
                description: description.unwrap_or_default(),
                path,
            });
        }
    }
    found
}

/// `name` and `description` from a SKILL.md's YAML frontmatter.
fn frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let Some(rest) = text.strip_prefix("---") else {
        return (None, None);
    };
    let Some(end) = rest.find("\n---") else {
        return (None, None);
    };
    let field = |key: &str| {
        rest[..end].lines().find_map(|line| {
            let value = line.strip_prefix(key)?.strip_prefix(':')?.trim();
            let value = value.trim_matches(|c| c == '"' || c == '\'');
            (!value.is_empty()).then(|| value.to_string())
        })
    };
    (field("name"), field("description"))
}

/// Skills listed for the agent's system prompt.
pub fn skills_prompt(skills: &[Skill]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut out = String::from(
        "Skills are available: instructions for particular kinds of work. When a task matches a skill's description, read its SKILL.md (with your file tools) before you start, and follow it.\n",
    );
    for skill in skills.iter().take(60) {
        let description: String = skill.description.chars().take(300).collect();
        out.push_str(&format!(
            "- {}: {} ({})\n",
            skill.name,
            description,
            skill.path.display()
        ));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{expand, fingerprint, frontmatter, parse_mcp_json, skills, skills_prompt};

    #[test]
    fn reads_mcp_json_like_claude_code() {
        // SAFETY: tests in this module set only this variable.
        unsafe { std::env::set_var("ELYRA_SHARED_SETUP_TEST", "secret") };
        let servers = parse_mcp_json(
            r#"{"mcpServers": {
                "laravel-boost": {"command": "docker", "args": ["exec", "-i", "app", "php", "artisan", "boost:mcp"]},
                "docs": {"type": "http", "url": "https://docs.example/mcp", "headers": {"X-Key": "${ELYRA_SHARED_SETUP_TEST}"}},
                "local": {"command": "${HOME_NOT_SET_FOR_TEST:-/usr/bin/env}", "env": {"MODE": "${MISSING_FOR_TEST:-dev}"}},
                "off": {"command": "x", "disabled": true}
            }}"#,
        )
        .unwrap();
        let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["docs", "laravel-boost", "local"]);
        assert_eq!(
            servers[1].summary,
            "docker exec -i app php artisan boost:mcp"
        );
        assert_eq!(
            servers[0].server.headers,
            [("X-Key".to_string(), "secret".to_string())]
        );
        assert_eq!(
            servers[2].server.env,
            [("MODE".to_string(), "dev".to_string())]
        );
        assert_eq!(
            servers[2]
                .server
                .stdio
                .as_ref()
                .unwrap()
                .0
                .display()
                .to_string(),
            "/usr/bin/env"
        );
        assert!(parse_mcp_json("{}").is_err());
        assert!(parse_mcp_json("not json").is_err());
        assert_eq!(expand("a ${UNSET_FOR_TEST} b ${"), "a  b ${");
        assert_eq!(fingerprint("x"), fingerprint("x"));
        assert_ne!(fingerprint("x"), fingerprint("y"));
    }

    #[test]
    fn lists_skills_with_their_descriptions() {
        let dir = std::env::temp_dir().join(format!("elyra-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (root, name, text) in [
            (
                ".claude/skills",
                "flux",
                "---\nname: fluxui-development\ndescription: \"Flux UI in Livewire\"\n---\nBody",
            ),
            (".agents/skills", "plain", "No frontmatter"),
            (
                ".agents/skills",
                "dupe",
                "---\nname: fluxui-development\n---\n",
            ),
        ] {
            let folder = dir.join(root).join(name);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("SKILL.md"), text).unwrap();
        }
        let found: Vec<_> = skills(&dir)
            .into_iter()
            .filter(|s| s.path.starts_with(&dir))
            .collect();
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].name, "fluxui-development");
        assert_eq!(found[0].description, "Flux UI in Livewire");
        assert_eq!(found[1].name, "plain", "the folder names it");
        let prompt = skills_prompt(&found).unwrap();
        assert!(prompt.contains("- fluxui-development: Flux UI in Livewire ("));
        assert_eq!(frontmatter("no"), (None, None));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
