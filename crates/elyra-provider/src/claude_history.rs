//! Read Claude Code's own session history (`~/.claude/projects/*/*.jsonl`)
//! so past sessions can be imported as threads and resumed.

use crate::process::text_of;
use chrono::{DateTime, Utc};
use elyra_core::ItemContent;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct SessionSummary {
    pub session_id: String,
    pub path: PathBuf,
    pub cwd: Option<PathBuf>,
    pub title: String,
    pub messages: usize,
    pub updated: Option<DateTime<Utc>>,
}

pub struct ImportedSession {
    pub summary: SessionSummary,
    pub items: Vec<(ItemContent, Option<DateTime<Utc>>)>,
}

pub fn projects_dir() -> Option<PathBuf> {
    let base = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))?;
    Some(base.join("projects"))
}

/// The most recently modified sessions, newest first.
pub fn list_sessions(root: &Path, limit: usize) -> Vec<SessionSummary> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .flat_map(|dir| {
            std::fs::read_dir(dir.path())
                .into_iter()
                .flatten()
                .flatten()
        })
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .filter_map(|path| Some((std::fs::metadata(&path).ok()?.modified().ok()?, path)))
        .collect();
    files.sort_by_key(|file| std::cmp::Reverse(file.0));
    files
        .into_iter()
        .filter_map(|(_, path)| read(&path, false).map(|s| s.summary))
        .filter(|summary| summary.messages > 0)
        .take(limit)
        .collect()
}

/// Load one session with its transcript.
pub fn load_session(path: &Path) -> Option<ImportedSession> {
    read(path, true)
}

fn timestamp(record: &Value) -> Option<DateTime<Utc>> {
    record["timestamp"]
        .as_str()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// Prompts Claude Code injects itself (slash command echoes, caveats).
fn is_internal(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with("<command-")
        || text.starts_with("<local-command")
        || text.starts_with("Caveat: ")
        || text.starts_with("<system-reminder>")
}

fn read(path: &Path, with_items: bool) -> Option<ImportedSession> {
    let file = std::fs::File::open(path).ok()?;
    let mut summary = SessionSummary {
        session_id: path.file_stem()?.to_string_lossy().into_owned(),
        path: path.to_path_buf(),
        cwd: None,
        title: String::new(),
        messages: 0,
        updated: None,
    };
    let mut first_prompt = None;
    let mut items = Vec::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match record["type"].as_str() {
            Some("ai-title") => {
                if let Some(title) = record["aiTitle"].as_str() {
                    summary.title = title.to_string();
                }
                continue;
            }
            Some("summary") => {
                if summary.title.is_empty()
                    && let Some(title) = record["summary"].as_str()
                {
                    summary.title = title.to_string();
                }
                continue;
            }
            Some("user" | "assistant") => {}
            _ => continue,
        }
        // Subagent transcripts and injected meta messages are not part of
        // the visible conversation.
        if record["isSidechain"].as_bool() == Some(true) || record["isMeta"].as_bool() == Some(true)
        {
            continue;
        }
        if summary.cwd.is_none() {
            summary.cwd = record["cwd"].as_str().map(PathBuf::from);
        }
        if let Some(sid) = record["sessionId"].as_str() {
            summary.session_id = sid.to_string();
        }
        let when = timestamp(&record);
        if when.is_some() {
            summary.updated = when;
        }
        let message = &record["message"];
        let content = &message["content"];
        if record["type"] == "user" {
            if let Value::String(text) = content {
                if is_internal(text) {
                    continue;
                }
                summary.messages += 1;
                first_prompt.get_or_insert_with(|| text.clone());
                if with_items {
                    items.push((
                        ItemContent::User {
                            text: text.clone(),
                            checkpoint: None,
                        },
                        when,
                    ));
                }
                continue;
            }
            for block in content.as_array().into_iter().flatten() {
                match block["type"].as_str() {
                    Some("text") => {
                        let text = block["text"].as_str().unwrap_or("");
                        if is_internal(text) {
                            continue;
                        }
                        summary.messages += 1;
                        first_prompt.get_or_insert_with(|| text.to_string());
                        if with_items {
                            items.push((
                                ItemContent::User {
                                    text: text.into(),
                                    checkpoint: None,
                                },
                                when,
                            ));
                        }
                    }
                    Some("tool_result") if with_items => items.push((
                        ItemContent::ToolResult {
                            tool_use_id: block["tool_use_id"].as_str().unwrap_or("").into(),
                            content: text_of(&block["content"]),
                            is_error: block["is_error"].as_bool().unwrap_or(false),
                        },
                        when,
                    )),
                    _ => {}
                }
            }
            continue;
        }
        for block in content.as_array().into_iter().flatten() {
            match block["type"].as_str() {
                Some("text") => {
                    summary.messages += 1;
                    if with_items {
                        items.push((
                            ItemContent::Assistant {
                                text: block["text"].as_str().unwrap_or("").into(),
                                parent_tool_use_id: None,
                            },
                            when,
                        ));
                    }
                }
                Some("thinking") if with_items => {
                    let text = block["thinking"].as_str().unwrap_or("");
                    if !text.trim().is_empty() {
                        items.push((ItemContent::Thinking { text: text.into() }, when));
                    }
                }
                Some("tool_use") if with_items => items.push((
                    ItemContent::ToolUse {
                        tool_use_id: block["id"].as_str().unwrap_or("").into(),
                        name: block["name"].as_str().unwrap_or("tool").into(),
                        input: block["input"].clone(),
                        parent_tool_use_id: None,
                    },
                    when,
                )),
                _ => {}
            }
        }
    }
    if summary.title.is_empty() {
        summary.title = crate::clean_title(first_prompt.as_deref().unwrap_or("Imported session"));
    }
    Some(ImportedSession { summary, items })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_claude_code_sessions() {
        let root = std::env::temp_dir().join(format!("elyra-history-{}", std::process::id()));
        let dir = root.join("-tmp-demo");
        std::fs::create_dir_all(&dir).unwrap();
        let lines = [
            r#"{"type":"mode","sessionId":"s1"}"#,
            r#"{"type":"user","sessionId":"s1","cwd":"/tmp/demo","timestamp":"2026-09-01T10:00:00Z","message":{"role":"user","content":"Fix the build"}}"#,
            r#"{"type":"user","isMeta":true,"sessionId":"s1","message":{"role":"user","content":"meta"}}"#,
            r#"{"type":"user","sessionId":"s1","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
            r#"{"type":"assistant","sessionId":"s1","message":{"content":[{"type":"text","text":"Looking."}]}}"#,
            r#"{"type":"assistant","sessionId":"s1","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo build"}}]}}"#,
            r#"{"type":"user","sessionId":"s1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"sessionId":"s1","message":{"content":[{"type":"text","text":"sub"}]}}"#,
            r#"{"type":"ai-title","aiTitle":"Fix the build","sessionId":"s1"}"#,
        ];
        std::fs::write(dir.join("s1.jsonl"), lines.join("\n")).unwrap();
        std::fs::write(dir.join("empty.jsonl"), r#"{"type":"mode"}"#).unwrap();

        let sessions = list_sessions(&root, 10);
        assert_eq!(sessions.len(), 1, "sessions without messages are skipped");
        let summary = &sessions[0];
        assert_eq!(
            (
                summary.session_id.as_str(),
                summary.title.as_str(),
                summary.messages
            ),
            ("s1", "Fix the build", 2)
        );
        assert_eq!(summary.cwd.as_deref(), Some(Path::new("/tmp/demo")));

        let session = load_session(&summary.path).unwrap();
        let kinds: Vec<&str> = session
            .items
            .iter()
            .map(|(item, _)| match item {
                ItemContent::User { .. } => "user",
                ItemContent::Assistant { .. } => "assistant",
                ItemContent::ToolUse { .. } => "tool",
                ItemContent::ToolResult { .. } => "result",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["user", "assistant", "tool", "result"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
