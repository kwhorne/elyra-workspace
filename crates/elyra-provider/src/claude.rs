//! Claude Code adapter: drives `claude -p` over its bidirectional
//! stream-json protocol (the same protocol the Claude Agent SDK uses).
//!
//! stdin:  `user` messages and `control_request`/`control_response` frames
//! stdout: `system`, `assistant`, `user`, `stream_event`, `result`,
//!         `control_request` (e.g. `can_use_tool`) and `control_response`

use crate::process::{JsonlProcess, text_of};
use crate::{
    AgentSession, Capabilities, ModelOption, PermissionRequest, PermissionResponse, Prompt,
    ProviderEvent, Question, QuestionAnswer, QuestionKind, QuestionOption, QuestionRequest,
    SessionConfig, SlashCommand, Usage,
};
use anyhow::{Result, anyhow};
use elyra_core::PermissionMode;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const CAPABILITIES: Capabilities = Capabilities {
    permission_modes: true,
    efforts: &[
        ("low", "Low"),
        ("medium", "Medium"),
        ("high", "High"),
        ("xhigh", "Extra high"),
        ("max", "Max"),
    ],
    models: MODELS,
    steer: true,
    compact: true,
    images: true,
};

/// Suggested models; any model id or alias accepted by the CLI also works.
pub const MODELS: &[(&str, &str)] = &[
    ("", "Default"),
    ("opus", "Opus"),
    ("sonnet", "Sonnet"),
    ("haiku", "Haiku"),
    ("fable", "Fable"),
];

/// Locate the `claude` executable on PATH or in common install locations.
pub fn find_executable() -> Option<PathBuf> {
    crate::process::find_executable("claude")
}

fn cli_permission_mode(mode: PermissionMode) -> Option<&'static str> {
    match mode {
        // The CLI's own default asks before edits and commands.
        PermissionMode::Ask => None,
        PermissionMode::AcceptEdits => Some("acceptEdits"),
        PermissionMode::Plan => Some("plan"),
        PermissionMode::FullAccess => Some("bypassPermissions"),
    }
}

#[derive(Clone)]
enum Pending {
    Permission(PermissionRequest),
    Question(QuestionRequest),
}

/// A live Claude Code process for one thread.
pub struct ClaudeSession {
    process: Arc<JsonlProcess>,
    next_request: AtomicU64,
    /// Requests awaiting a decision, keyed by request id, so the response can
    /// echo the original input and suggestions.
    pending: Arc<Mutex<HashMap<String, Pending>>>,
}

impl ClaudeSession {
    pub fn start(config: SessionConfig) -> Result<(Self, async_channel::Receiver<ProviderEvent>)> {
        let executable = config
            .executable
            .clone()
            .or_else(find_executable)
            .ok_or_else(|| anyhow!("Claude Code CLI (`claude`) was not found on PATH"))?;

        let mut args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
            // Makes Full access selectable later without enabling it now.
            "--allow-dangerously-skip-permissions",
        ]
        .map(str::to_string)
        .into();
        if let Some(model) = config.model.as_deref().filter(|m| !m.is_empty()) {
            args.extend(["--model".into(), model.into()]);
        }
        if let Some(effort) = config.effort.as_deref().filter(|e| !e.is_empty()) {
            args.extend(["--effort".into(), effort.into()]);
        }
        if let Some(mode) = cli_permission_mode(config.permission_mode) {
            args.extend(["--permission-mode".into(), mode.into()]);
        }
        if let Some(session) = config.resume_session_id.as_deref() {
            args.extend(["--resume".into(), session.into()]);
            if config.fork {
                args.push("--fork-session".into());
            }
        }
        if !config.mcp_servers.is_empty() {
            let servers: serde_json::Map<String, Value> = config
                .mcp_servers
                .iter()
                .map(|server| {
                    (
                        server.name.clone(),
                        json!({
                            "type": "http",
                            "url": server.url,
                            "headers": { "Authorization": format!("Bearer {}", server.token) }
                        }),
                    )
                })
                .collect();
            args.extend([
                "--mcp-config".into(),
                json!({ "mcpServers": servers }).to_string(),
            ]);
        }
        if let Some(prompt) = config
            .append_system_prompt
            .as_deref()
            .filter(|p| !p.trim().is_empty())
        {
            args.extend(["--append-system-prompt".into(), prompt.into()]);
        }

        let pending = Arc::new(Mutex::new(HashMap::new()));
        let pending_for_events = pending.clone();
        let mut parser = StreamParser::default();
        let mut envs: Vec<(&str, &str)> = vec![("CLAUDE_CODE_ENTRYPOINT", "sdk-rust-elyra")];
        envs.extend(config.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let (process, rx) = JsonlProcess::spawn(
            &executable,
            &args,
            &config.cwd,
            &envs,
            move |value| parser.handle(value),
            move |event| match event {
                ProviderEvent::PermissionRequest(request) => {
                    pending_for_events.lock().unwrap().insert(
                        request.request_id.clone(),
                        Pending::Permission(request.clone()),
                    );
                }
                ProviderEvent::Question(request) => {
                    pending_for_events.lock().unwrap().insert(
                        request.request_id.clone(),
                        Pending::Question(request.clone()),
                    );
                }
                _ => {}
            },
        )?;
        let session = Self {
            process,
            next_request: AtomicU64::new(1),
            pending,
        };
        session.send_control(json!({ "subtype": "initialize", "hooks": null }), "init")?;
        Ok((session, rx))
    }

    fn send_control(&self, request: Value, prefix: &str) -> Result<()> {
        let id = self.next_request.fetch_add(1, Ordering::Relaxed);
        self.process.write(&json!({
            "type": "control_request",
            "request_id": format!("{prefix}_{id}"),
            "request": request,
        }))
    }

    fn write_user_message(&self, prompt: &Prompt) -> Result<()> {
        let mut content: Vec<Value> = prompt
            .images
            .iter()
            .map(|image| {
                json!({
                    "type": "image",
                    "source": { "type": "base64", "media_type": image.media_type, "data": image.data },
                })
            })
            .collect();
        content.push(json!({ "type": "text", "text": prompt.text }));
        self.process.write(&json!({
            "type": "user",
            "message": { "role": "user", "content": content },
            "parent_tool_use_id": null,
            "session_id": "",
        }))
    }

    fn respond(&self, request_id: &str, body: Value) -> Result<()> {
        self.process.write(&json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": request_id, "response": body },
        }))
    }
}

impl AgentSession for ClaudeSession {
    fn send(&self, prompt: &Prompt) -> Result<()> {
        self.write_user_message(prompt)
    }

    /// The CLI accepts user messages mid-turn and folds them into the run.
    fn steer(&self, prompt: &Prompt) -> Result<()> {
        self.write_user_message(prompt)
    }

    fn respond_permission(&self, request_id: &str, response: PermissionResponse) -> Result<()> {
        let request = match self.pending.lock().unwrap().remove(request_id) {
            Some(Pending::Permission(request)) => Some(request),
            _ => None,
        };
        let input = request
            .as_ref()
            .map(|r| r.input.clone())
            .unwrap_or(json!({}));
        let body = match response {
            PermissionResponse::Allow => json!({ "behavior": "allow", "updatedInput": input }),
            PermissionResponse::AllowForSession => {
                let suggestions = request
                    .map(|r| r.suggestions)
                    .filter(|s| s.as_array().is_some_and(|a| !a.is_empty()));
                match suggestions {
                    Some(suggestions) => json!({
                        "behavior": "allow",
                        "updatedInput": input,
                        "updatedPermissions": suggestions,
                    }),
                    None => json!({ "behavior": "allow", "updatedInput": input }),
                }
            }
            PermissionResponse::Deny { message } => {
                json!({ "behavior": "deny", "message": message })
            }
        };
        self.respond(request_id, body)
    }

    fn answer_question(&self, request_id: &str, answer: QuestionAnswer) -> Result<()> {
        let request = match self.pending.lock().unwrap().remove(request_id) {
            Some(Pending::Question(request)) => request,
            _ => return Err(anyhow!("question {request_id} is no longer pending")),
        };
        let body = match answer {
            QuestionAnswer::Answers { answers } => {
                let mut input = request.input.clone();
                let map: serde_json::Map<String, Value> = request
                    .questions
                    .iter()
                    .zip(answers)
                    .map(|(question, answer)| (question.question.clone(), Value::String(answer)))
                    .collect();
                if let Some(object) = input.as_object_mut() {
                    object.insert("answers".into(), Value::Object(map));
                }
                json!({ "behavior": "allow", "updatedInput": input })
            }
            QuestionAnswer::Confirmed { confirmed: true } => {
                json!({ "behavior": "allow", "updatedInput": request.input })
            }
            QuestionAnswer::Confirmed { confirmed: false } | QuestionAnswer::Cancelled => json!({
                "behavior": "deny",
                "message": "The user dismissed the question without answering.",
            }),
        };
        self.respond(request_id, body)
    }

    fn interrupt(&self) -> Result<()> {
        self.send_control(json!({ "subtype": "interrupt" }), "interrupt")
    }

    fn set_permission_mode(&self, mode: PermissionMode) -> Result<()> {
        let mode = cli_permission_mode(mode).unwrap_or("default");
        self.send_control(
            json!({ "subtype": "set_permission_mode", "mode": mode }),
            "mode",
        )
    }

    fn set_model(&self, model: Option<&str>) -> Result<()> {
        self.send_control(
            json!({ "subtype": "set_model", "model": model.filter(|m| !m.is_empty()) }),
            "model",
        )
    }

    fn set_effort(&self, _effort: Option<&str>) -> Result<bool> {
        // Effort is a launch flag; the next turn restarts with it.
        Ok(false)
    }

    fn compact(&self) -> Result<()> {
        self.write_user_message(&Prompt::text("/compact"))
    }

    fn shutdown(&self) {
        self.process.shutdown();
    }
}

impl Drop for ClaudeSession {
    fn drop(&mut self) {
        self.process.shutdown();
    }
}

/// Translation of Claude stream-json into provider events.
#[derive(Default)]
pub(crate) struct StreamParser {
    compacting: bool,
}

fn question_request(request_id: &str, input: &Value) -> QuestionRequest {
    let questions = input["questions"]
        .as_array()
        .map(|questions| {
            questions
                .iter()
                .map(|q| Question {
                    question: q["question"].as_str().unwrap_or("").to_string(),
                    header: q["header"].as_str().unwrap_or("").to_string(),
                    options: q["options"]
                        .as_array()
                        .map(|options| {
                            options
                                .iter()
                                .map(|o| QuestionOption {
                                    label: o["label"].as_str().unwrap_or("").to_string(),
                                    description: o["description"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    multi_select: q["multiSelect"].as_bool().unwrap_or(false),
                    prefill: String::new(),
                    multiline: false,
                })
                .collect()
        })
        .unwrap_or_default();
    QuestionRequest {
        request_id: request_id.to_string(),
        kind: QuestionKind::Choice,
        title: String::new(),
        questions,
        input: input.clone(),
    }
}

fn usage_of(usage: &Value) -> Usage {
    let n = |key: &str| usage[key].as_u64().unwrap_or(0);
    let input = n("input_tokens");
    let cache_read = n("cache_read_input_tokens");
    let cache_write = n("cache_creation_input_tokens");
    Usage {
        input_tokens: input,
        output_tokens: n("output_tokens"),
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        cost_usd: None,
        context_tokens: Some(input + cache_read + cache_write),
        context_window: None,
    }
}

impl StreamParser {
    pub(crate) fn handle(&mut self, message: &Value) -> Vec<ProviderEvent> {
        let kind = message.get("type").and_then(Value::as_str).unwrap_or("");
        let parent = message
            .get("parent_tool_use_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut events = Vec::new();
        match kind {
            "system" => match message.get("subtype").and_then(Value::as_str) {
                Some("init") => {
                    if let Some(session_id) = message.get("session_id").and_then(Value::as_str) {
                        events.push(ProviderEvent::SessionStarted {
                            session_id: session_id.to_string(),
                            model: message
                                .get("model")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        });
                    }
                }
                Some("status") => {
                    let compacting = message["status"].as_str() == Some("compacting");
                    if compacting != self.compacting {
                        self.compacting = compacting;
                        events.push(ProviderEvent::Compacting(compacting));
                    }
                }
                Some("compact_boundary") if self.compacting => {
                    self.compacting = false;
                    events.push(ProviderEvent::Compacting(false));
                }
                _ => {}
            },
            "stream_event" => {
                // Only top-level deltas stream into the live message; subagent
                // output arrives as completed blocks.
                if parent.is_some() {
                    return events;
                }
                let event = &message["event"];
                if event.get("type").and_then(Value::as_str) == Some("content_block_delta") {
                    let delta = &event["delta"];
                    match delta.get("type").and_then(Value::as_str) {
                        Some("text_delta") => {
                            if let Some(text) = delta.get("text").and_then(Value::as_str) {
                                events.push(ProviderEvent::TextDelta(text.to_string()));
                            }
                        }
                        Some("thinking_delta") => {
                            if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                                events.push(ProviderEvent::ThinkingDelta(text.to_string()));
                            }
                        }
                        _ => {}
                    }
                }
            }
            "assistant" => {
                let blocks = message["message"]["content"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let text = block["text"].as_str().unwrap_or("").to_string();
                            if !text.is_empty() {
                                events.push(ProviderEvent::AssistantText {
                                    text,
                                    parent_tool_use_id: parent.clone(),
                                });
                            }
                        }
                        Some("thinking") => {
                            let text = block["thinking"].as_str().unwrap_or("").to_string();
                            if parent.is_none() {
                                events.push(ProviderEvent::Thinking { text });
                            }
                        }
                        Some("tool_use") | Some("server_tool_use") => {
                            events.push(ProviderEvent::ToolUse {
                                tool_use_id: block["id"].as_str().unwrap_or("").to_string(),
                                name: block["name"].as_str().unwrap_or("tool").to_string(),
                                input: block.get("input").cloned().unwrap_or(Value::Null),
                                parent_tool_use_id: parent.clone(),
                            });
                        }
                        _ => {}
                    }
                }
                if parent.is_none()
                    && let Some(usage) = message["message"].get("usage").filter(|u| u.is_object())
                {
                    events.push(ProviderEvent::Usage(usage_of(usage)));
                }
            }
            "user" => {
                let content = &message["message"]["content"];
                if let Some(blocks) = content.as_array() {
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                            events.push(ProviderEvent::ToolResult {
                                tool_use_id: block["tool_use_id"]
                                    .as_str()
                                    .unwrap_or("")
                                    .to_string(),
                                content: text_of(&block["content"]),
                                is_error: block["is_error"].as_bool().unwrap_or(false),
                            });
                        }
                    }
                }
            }
            "result" => {
                let is_error = message["is_error"].as_bool().unwrap_or(false)
                    || message["subtype"].as_str().is_some_and(|s| s != "success");
                let detail = if is_error {
                    message
                        .get("result")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| message["subtype"].as_str().map(str::to_string))
                } else {
                    None
                };
                let cost = message["total_cost_usd"].as_f64();
                let window = message["modelUsage"].as_object().and_then(|models| {
                    models
                        .values()
                        .filter_map(|m| m["contextWindow"].as_u64())
                        .max()
                });
                if cost.is_some() || window.is_some() {
                    events.push(ProviderEvent::Usage(Usage {
                        cost_usd: cost,
                        context_window: window,
                        ..Usage::default()
                    }));
                }
                events.push(ProviderEvent::TurnCompleted {
                    is_error,
                    duration_ms: message["duration_ms"].as_u64(),
                    cost_usd: cost,
                    message: detail,
                });
            }
            "rate_limit_event" => {
                let info = &message["rate_limit_info"];
                if info["status"].as_str() == Some("rejected")
                    && info["overageStatus"].as_str() != Some("allowed")
                {
                    events.push(ProviderEvent::RateLimited {
                        resets_at: info["resetsAt"].as_i64(),
                    });
                }
            }
            "control_request" => {
                let request = &message["request"];
                if request.get("subtype").and_then(Value::as_str) == Some("can_use_tool") {
                    let request_id = message["request_id"].as_str().unwrap_or("").to_string();
                    let tool_name = request["tool_name"].as_str().unwrap_or("tool").to_string();
                    let input = request.get("input").cloned().unwrap_or(Value::Null);
                    if tool_name == "AskUserQuestion" {
                        events.push(ProviderEvent::Question(question_request(
                            &request_id,
                            &input,
                        )));
                    } else {
                        events.push(ProviderEvent::PermissionRequest(PermissionRequest {
                            request_id,
                            tool_name,
                            tool_use_id: request["tool_use_id"].as_str().map(str::to_string),
                            description: request["description"].as_str().map(str::to_string),
                            input,
                            suggestions: request
                                .get("permission_suggestions")
                                .cloned()
                                .unwrap_or(Value::Null),
                        }));
                    }
                }
            }
            "control_response" => {
                let response = &message["response"];
                match response.get("subtype").and_then(Value::as_str) {
                    Some("error") => events.push(ProviderEvent::Warning(
                        response["error"]
                            .as_str()
                            .unwrap_or("control request failed")
                            .to_string(),
                    )),
                    Some("success") => {
                        let body = &response["response"];
                        if let Some(commands) = body["commands"].as_array() {
                            events.push(ProviderEvent::Commands(
                                commands
                                    .iter()
                                    .filter_map(|c| {
                                        Some(SlashCommand {
                                            name: c["name"].as_str()?.to_string(),
                                            description: c["description"]
                                                .as_str()
                                                .unwrap_or("")
                                                .to_string(),
                                            argument_hint: c["argumentHint"]
                                                .as_str()
                                                .unwrap_or("")
                                                .to_string(),
                                        })
                                    })
                                    .collect(),
                            ));
                        }
                        if let Some(agents) = body["agents"].as_array() {
                            events.push(ProviderEvent::Agents(
                                agents
                                    .iter()
                                    .filter_map(|a| {
                                        Some(SlashCommand {
                                            name: a["name"].as_str()?.to_string(),
                                            description: a["description"]
                                                .as_str()
                                                .unwrap_or("")
                                                .to_string(),
                                            argument_hint: String::new(),
                                        })
                                    })
                                    .collect(),
                            ));
                        }
                        if let Some(models) = body["models"].as_array() {
                            let mut options = vec![ModelOption {
                                id: String::new(),
                                label: "Default".into(),
                            }];
                            options.extend(models.iter().filter_map(|m| {
                                let id = m["value"].as_str()?.to_string();
                                let label = m["displayName"].as_str().unwrap_or(&id).to_string();
                                Some(ModelOption { id, label })
                            }));
                            events.push(ProviderEvent::Models(options));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Vec<ProviderEvent> {
        StreamParser::default().handle(&serde_json::from_str(line).unwrap())
    }

    #[test]
    fn parses_recorded_protocol_frames() {
        assert_eq!(
            parse(
                r#"{"type":"system","subtype":"init","session_id":"s1","model":"claude-opus-5-5"}"#
            ),
            vec![ProviderEvent::SessionStarted {
                session_id: "s1".into(),
                model: Some("claude-opus-5-5".into())
            }]
        );
        assert_eq!(
            parse(
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"He"}},"parent_tool_use_id":null}"#
            ),
            vec![ProviderEvent::TextDelta("He".into())]
        );
        let events = parse(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":2,"cache_read_input_tokens":100,"cache_creation_input_tokens":8,"output_tokens":5}},"parent_tool_use_id":null}"#,
        );
        assert_eq!(
            events[0],
            ProviderEvent::ToolUse {
                tool_use_id: "t1".into(),
                name: "Bash".into(),
                input: serde_json::json!({"command":"ls"}),
                parent_tool_use_id: None
            }
        );
        let ProviderEvent::Usage(usage) = &events[1] else {
            panic!("{events:?}")
        };
        assert_eq!((usage.context_tokens, usage.output_tokens), (Some(110), 5));
        assert_eq!(
            parse(
                r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t1","type":"tool_result","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}],"is_error":false}]}}"#
            ),
            vec![ProviderEvent::ToolResult {
                tool_use_id: "t1".into(),
                content: "a\nb".into(),
                is_error: false
            }]
        );
        let permission = parse(
            r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"/x"},"description":"x","permission_suggestions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}],"tool_use_id":"t2"}}"#,
        );
        let [ProviderEvent::PermissionRequest(request)] = permission.as_slice() else {
            panic!("{permission:?}")
        };
        assert_eq!(
            (request.request_id.as_str(), request.tool_name.as_str()),
            ("r1", "Write")
        );
        let result = parse(
            r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":1200,"total_cost_usd":0.05,"modelUsage":{"m":{"contextWindow":1000000}}}"#,
        );
        assert_eq!(
            result.last().unwrap(),
            &ProviderEvent::TurnCompleted {
                is_error: false,
                duration_ms: Some(1200),
                cost_usd: Some(0.05),
                message: None
            }
        );
        let ProviderEvent::Usage(usage) = &result[0] else {
            panic!()
        };
        assert_eq!(usage.context_window, Some(1_000_000));
    }

    #[test]
    fn ask_user_question_becomes_a_question() {
        let events = parse(
            r#"{"type":"control_request","request_id":"q1","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Which DB?","header":"DB","multiSelect":false,"options":[{"label":"SQLite","description":"local"},{"label":"Postgres","description":"server"}]}]}}}"#,
        );
        let [ProviderEvent::Question(question)] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert_eq!(question.kind, QuestionKind::Choice);
        assert_eq!(question.questions[0].options[1].label, "Postgres");
    }

    #[test]
    fn initialize_response_lists_commands() {
        let events = parse(
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"init_1","response":{"commands":[{"name":"review","description":"Review code","argumentHint":"[pr]"}]}}}"#,
        );
        assert_eq!(
            events,
            vec![ProviderEvent::Commands(vec![SlashCommand {
                name: "review".into(),
                description: "Review code".into(),
                argument_hint: "[pr]".into()
            }])]
        );
    }
}
