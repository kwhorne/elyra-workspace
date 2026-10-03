//! Read Codex's saved threads through a short-lived `codex app-server`
//! (`thread/list`, `thread/read`) so they can be imported and resumed.

use crate::claude_history::{ImportedSession, SessionSummary};
use anyhow::{Context as _, Result, anyhow};
use chrono::{DateTime, TimeZone as _, Utc};
use elyra_core::ItemContent;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// A blocking JSON-RPC client for one `codex app-server` process.
struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Client {
    fn start(executable: &Path) -> Result<Self> {
        let mut child = Command::new(executable)
            .arg("app-server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("starting {}", executable.display()))?;
        let stdin = child.stdin.take().context("stdin")?;
        let stdout = BufReader::new(child.stdout.take().context("stdout")?);
        let mut client = Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        client.call(
            "initialize",
            json!({
                "clientInfo": { "name": "elyra_workspace", "title": "Elyra Workspace", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": true }
            }),
        )?;
        writeln!(client.stdin, "{}", json!({ "method": "initialized" }))?;
        Ok(client)
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({ "id": id, "method": method, "params": params })
        )?;
        self.stdin.flush()?;
        let mut line = String::new();
        loop {
            line.clear();
            if self.stdout.read_line(&mut line)? == 0 {
                return Err(anyhow!("codex app-server closed the connection"));
            }
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if message["id"].as_u64() == Some(id) && message.get("method").is_none() {
                if let Some(error) = message.get("error") {
                    return Err(anyhow!(
                        "{method}: {}",
                        error["message"].as_str().unwrap_or("error")
                    ));
                }
                return Ok(message["result"].clone());
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Codex timestamps are Unix seconds (or milliseconds in newer fields).
fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    let n = value.as_i64()?;
    let (secs, millis) = if n > 100_000_000_000 {
        (n / 1000, n % 1000)
    } else {
        (n, 0)
    };
    Utc.timestamp_opt(secs, (millis * 1_000_000) as u32)
        .single()
}

fn summary(thread: &Value) -> Option<SessionSummary> {
    let id = thread["id"].as_str()?.to_string();
    let title = thread["name"]
        .as_str()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| thread["preview"].as_str())
        .map(crate::clean_title)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Codex session".into());
    Some(SessionSummary {
        session_id: id.clone(),
        path: PathBuf::from(thread["path"].as_str().unwrap_or(&id)),
        cwd: thread["cwd"].as_str().map(PathBuf::from),
        title,
        messages: thread["turns"].as_array().map(|t| t.len()).unwrap_or(0),
        updated: timestamp(&thread["updatedAt"]),
    })
}

/// Recent Codex threads, newest first.
pub fn list_sessions(executable: &Path, limit: usize) -> Result<Vec<SessionSummary>> {
    let mut client = Client::start(executable)?;
    let result = client.call(
        "thread/list",
        json!({ "limit": limit, "sortKey": "updated_at", "archived": false }),
    )?;
    Ok(result["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["ephemeral"].as_bool() != Some(true))
        .filter_map(summary)
        .collect())
}

fn user_text(item: &Value) -> String {
    item["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| match c["type"].as_str() {
            Some("text") => c["text"].as_str().map(str::to_string),
            Some("image" | "localImage") => Some("[image]".into()),
            Some("mention" | "skill") => c["name"].as_str().map(|n| format!("@{n}")),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Transcript items for one Codex thread item.
fn items_for(item: &Value) -> Vec<ItemContent> {
    match item["type"].as_str().unwrap_or("") {
        "userMessage" => {
            let text = user_text(item);
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![ItemContent::User {
                    text,
                    checkpoint: None,
                }]
            }
        }
        "agentMessage" | "plan" => {
            let text = item["text"].as_str().unwrap_or("").to_string();
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![ItemContent::Assistant {
                    text,
                    parent_tool_use_id: None,
                }]
            }
        }
        "reasoning" => {
            let text = item["summary"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n\n");
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![ItemContent::Thinking { text }]
            }
        }
        _ => {
            let Some((name, input)) = crate::codex::tool_for_item(item) else {
                return Vec::new();
            };
            let id = item["id"].as_str().unwrap_or("").to_string();
            let status = item["status"].as_str().unwrap_or("completed");
            let content = match item["type"].as_str() {
                Some("commandExecution") => {
                    item["aggregatedOutput"].as_str().unwrap_or("").to_string()
                }
                Some("mcpToolCall") => crate::codex::mcp_result_text(&item["result"]),
                _ => String::new(),
            };
            vec![
                ItemContent::ToolUse {
                    tool_use_id: id.clone(),
                    name,
                    input,
                    parent_tool_use_id: None,
                },
                ItemContent::ToolResult {
                    tool_use_id: id,
                    content,
                    is_error: matches!(status, "failed" | "declined")
                        || item["exitCode"].as_i64().is_some_and(|c| c != 0),
                },
            ]
        }
    }
}

/// Load one thread with its transcript.
pub fn load_session(executable: &Path, thread_id: &str) -> Result<ImportedSession> {
    let mut client = Client::start(executable)?;
    let result = client.call(
        "thread/read",
        json!({ "threadId": thread_id, "includeTurns": true }),
    )?;
    let thread = &result["thread"];
    let mut summary = summary(thread).ok_or_else(|| anyhow!("Codex returned no thread"))?;
    let mut items = Vec::new();
    for turn in thread["turns"].as_array().into_iter().flatten() {
        let when = timestamp(&turn["startedAt"]).or(summary.updated);
        for item in turn["items"].as_array().into_iter().flatten() {
            items.extend(items_for(item).into_iter().map(|content| (content, when)));
        }
    }
    summary.messages = items
        .iter()
        .filter(|(c, _)| matches!(c, ItemContent::User { .. } | ItemContent::Assistant { .. }))
        .count();
    Ok(ImportedSession { summary, items })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_codex_items_to_transcript_items() {
        let user = json!({ "type": "userMessage", "id": "u", "content": [{ "type": "text", "text": "Fix it", "text_elements": [] }] });
        assert!(
            matches!(&items_for(&user)[..], [ItemContent::User { text, .. }] if text == "Fix it")
        );
        let command = json!({ "type": "commandExecution", "id": "c", "command": "cargo test", "cwd": "/x",
                              "status": "failed", "aggregatedOutput": "1 failed", "exitCode": 101 });
        match &items_for(&command)[..] {
            [
                ItemContent::ToolUse { name, .. },
                ItemContent::ToolResult {
                    content, is_error, ..
                },
            ] => {
                assert_eq!(
                    (name.as_str(), content.as_str(), *is_error),
                    ("Bash", "1 failed", true)
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            timestamp(&json!(1_700_000_000)).unwrap().timestamp(),
            1_700_000_000
        );
        assert_eq!(
            timestamp(&json!(1_700_000_000_123i64)).unwrap().timestamp(),
            1_700_000_000
        );
        let thread = json!({ "id": "t1", "name": null, "preview": "Add a login page.", "cwd": "/repo", "updatedAt": 1_700_000_000 });
        let summary = summary(&thread).unwrap();
        assert_eq!(
            (summary.title.as_str(), summary.cwd.unwrap()),
            ("Add a login page", PathBuf::from("/repo"))
        );
    }
}
