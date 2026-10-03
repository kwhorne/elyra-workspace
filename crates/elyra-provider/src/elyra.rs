//! Elyra coding agent adapter: drives `elyra --mode rpc` (JSON lines over
//! stdio; see elyra/packages/coding-agent/docs/rpc.md).
//!
//! stdin:  commands (`prompt`, `steer`, `abort`, `set_model`, …) and
//!         `extension_ui_response`
//! stdout: `response` (to commands), agent events and `extension_ui_request`

use crate::process::{JsonlProcess, text_of};
use crate::{
    AgentSession, Capabilities, ModelOption, PermissionResponse, Prompt, ProviderEvent, Question,
    QuestionAnswer, QuestionKind, QuestionOption, QuestionRequest, SessionConfig, SlashCommand,
    Usage,
};
use anyhow::{Result, anyhow};
use elyra_core::PermissionMode;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub const CAPABILITIES: Capabilities = Capabilities {
    // Elyra runs tools without per-call approval; extensions ask through
    // dialogs, which arrive as questions.
    permission_modes: false,
    efforts: &[
        ("off", "Off"),
        ("minimal", "Minimal"),
        ("low", "Low"),
        ("medium", "Medium"),
        ("high", "High"),
        ("xhigh", "Extra high"),
    ],
    models: &[("", "Default")],
    steer: true,
    compact: true,
    images: true,
};

pub fn find_executable() -> Option<PathBuf> {
    crate::process::find_executable("elyra")
}

/// The Pi coding agent speaks the same RPC protocol.
pub fn find_pi() -> Option<PathBuf> {
    crate::process::find_executable("pi")
}

/// Environment variable through which Elyra's MCP client takes servers for
/// one process (`.mcp.json` format); versions without it ignore it.
const MCP_SERVERS_ENV: &str = "ELYRA_MCP_SERVERS";

/// The gateway servers as `ELYRA_MCP_SERVERS` JSON, or `None` without any.
fn mcp_servers_env(servers: &[crate::McpServer]) -> Option<String> {
    if servers.is_empty() {
        return None;
    }
    let servers: serde_json::Map<String, Value> = servers
        .iter()
        .map(|server| {
            let entry = json!({
                "type": "http",
                "url": server.url,
                "headers": { "Authorization": format!("Bearer {}", server.token) },
                // Its own tools (`mcp__elyra__…`), not only through mcp_search.
                "directTools": true,
                // `wait_for_thread` may block for up to an hour.
                "timeout": 61 * 60 * 1000,
            });
            (server.name.clone(), entry)
        })
        .collect();
    Some(json!({ "mcpServers": servers }).to_string())
}

pub struct ElyraSession {
    process: Arc<JsonlProcess>,
    next_id: AtomicU64,
}

impl ElyraSession {
    pub fn start(config: SessionConfig) -> Result<(Self, async_channel::Receiver<ProviderEvent>)> {
        let executable = config
            .executable
            .clone()
            .or_else(find_executable)
            .ok_or_else(|| anyhow!("Elyra (`elyra`) was not found on PATH — install with `npm install -g @elyracode/coding-agent`"))?;
        let mut config = config;
        if let Some(servers) = mcp_servers_env(&config.mcp_servers) {
            config.env.push((MCP_SERVERS_ENV.into(), servers));
        }
        Self::start_with(executable, config)
    }

    /// Start Pi (`pi --mode rpc`).
    pub fn start_pi(
        config: SessionConfig,
    ) -> Result<(Self, async_channel::Receiver<ProviderEvent>)> {
        let executable = config
            .executable
            .clone()
            .or_else(find_pi)
            .ok_or_else(|| anyhow!("Pi (`pi`) was not found on PATH — install with `npm install -g @mariozechner/pi-coding-agent`"))?;
        Self::start_with(executable, config)
    }

    fn start_with(
        executable: PathBuf,
        config: SessionConfig,
    ) -> Result<(Self, async_channel::Receiver<ProviderEvent>)> {
        let mut args: Vec<String> = vec!["--mode".into(), "rpc".into()];
        if let Some(model) = config.model.as_deref().filter(|m| !m.is_empty()) {
            args.extend(["--model".into(), model.into()]);
        }
        if let Some(effort) = config.effort.as_deref().filter(|e| !e.is_empty()) {
            args.extend(["--thinking".into(), effort.into()]);
        }
        if let Some(session) = config.resume_session_id.as_deref() {
            let flag = if config.fork { "--fork" } else { "--session" };
            args.extend([flag.into(), session.into()]);
        }
        if let Some(prompt) = config
            .append_system_prompt
            .as_deref()
            .filter(|p| !p.trim().is_empty())
        {
            args.extend(["--append-system-prompt".into(), prompt.into()]);
        }
        let mut parser = RpcParser::default();
        let (process, rx) = JsonlProcess::spawn(
            &executable,
            &args,
            &config.cwd,
            &config
                .env
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect::<Vec<_>>(),
            move |value| parser.handle(value),
            |_| {},
        )?;
        let session = Self {
            process,
            next_id: AtomicU64::new(1),
        };
        session.command(json!({ "type": "get_state" }), "state")?;
        session.command(json!({ "type": "get_commands" }), "commands")?;
        session.command(json!({ "type": "get_available_models" }), "models")?;
        Ok((session, rx))
    }

    fn command(&self, mut command: Value, prefix: &str) -> Result<()> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        command["id"] = Value::String(format!("{prefix}_{id}"));
        self.process.write(&command)
    }

    fn images(prompt: &Prompt) -> Value {
        Value::Array(
            prompt
                .images
                .iter()
                .map(|image| json!({ "type": "image", "data": image.data, "mimeType": image.media_type }))
                .collect(),
        )
    }
}

impl AgentSession for ElyraSession {
    fn send(&self, prompt: &Prompt) -> Result<()> {
        self.command(
            json!({ "type": "prompt", "message": prompt.text, "images": Self::images(prompt) }),
            "prompt",
        )
    }

    fn steer(&self, prompt: &Prompt) -> Result<()> {
        self.command(
            json!({ "type": "steer", "message": prompt.text, "images": Self::images(prompt) }),
            "steer",
        )
    }

    fn respond_permission(&self, _request_id: &str, _response: PermissionResponse) -> Result<()> {
        Ok(())
    }

    fn answer_question(&self, request_id: &str, answer: QuestionAnswer) -> Result<()> {
        let mut response = json!({ "type": "extension_ui_response", "id": request_id });
        match answer {
            QuestionAnswer::Answers { answers } => {
                response["value"] = Value::String(answers.into_iter().next().unwrap_or_default());
            }
            QuestionAnswer::Confirmed { confirmed } => {
                response["confirmed"] = Value::Bool(confirmed)
            }
            QuestionAnswer::Cancelled => response["cancelled"] = Value::Bool(true),
        }
        self.process.write(&response)
    }

    fn interrupt(&self) -> Result<()> {
        self.command(json!({ "type": "abort" }), "abort")
    }

    fn set_permission_mode(&self, _mode: PermissionMode) -> Result<()> {
        Ok(())
    }

    fn set_model(&self, model: Option<&str>) -> Result<()> {
        let Some((provider, model_id)) = model.and_then(|m| m.split_once('/')) else {
            return Ok(());
        };
        self.command(
            json!({ "type": "set_model", "provider": provider, "modelId": model_id }),
            "set_model",
        )
    }

    fn set_effort(&self, effort: Option<&str>) -> Result<bool> {
        if let Some(level) = effort.filter(|e| !e.is_empty()) {
            self.command(
                json!({ "type": "set_thinking_level", "level": level }),
                "thinking",
            )?;
        }
        Ok(true)
    }

    fn compact(&self) -> Result<()> {
        self.command(json!({ "type": "compact" }), "compact")
    }

    fn refresh_usage(&self) -> Result<()> {
        self.command(json!({ "type": "get_session_stats" }), "stats")
    }

    fn shutdown(&self) {
        self.process.shutdown();
    }
}

impl Drop for ElyraSession {
    fn drop(&mut self) {
        self.process.shutdown();
    }
}

#[derive(Default)]
pub(crate) struct RpcParser {
    turn_started: Option<Instant>,
    turn_cost: f64,
    context_window: Option<u64>,
}

fn model_id(model: &Value) -> Option<String> {
    Some(format!(
        "{}/{}",
        model["provider"].as_str()?,
        model["id"].as_str()?
    ))
}

impl RpcParser {
    pub(crate) fn handle(&mut self, message: &Value) -> Vec<ProviderEvent> {
        let mut events = Vec::new();
        match message["type"].as_str().unwrap_or("") {
            "response" => self.handle_response(message, &mut events),
            "agent_start" => {
                self.turn_started = Some(Instant::now());
                self.turn_cost = 0.;
            }
            "message_update" => {
                let delta = &message["assistantMessageEvent"];
                match delta["type"].as_str() {
                    Some("text_delta") => events.push(ProviderEvent::TextDelta(
                        delta["delta"].as_str().unwrap_or("").into(),
                    )),
                    Some("thinking_delta") => events.push(ProviderEvent::ThinkingDelta(
                        delta["delta"].as_str().unwrap_or("").into(),
                    )),
                    _ => {}
                }
            }
            "message_end" => {
                let msg = &message["message"];
                if msg["role"].as_str() != Some("assistant") {
                    return events;
                }
                for block in msg["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => {
                            let text = block["text"].as_str().unwrap_or("");
                            if !text.is_empty() {
                                events.push(ProviderEvent::AssistantText {
                                    text: text.into(),
                                    parent_tool_use_id: None,
                                });
                            }
                        }
                        Some("thinking") => events.push(ProviderEvent::Thinking {
                            text: block["thinking"].as_str().unwrap_or("").into(),
                        }),
                        Some("toolCall") => events.push(ProviderEvent::ToolUse {
                            tool_use_id: block["id"].as_str().unwrap_or("").into(),
                            name: block["name"].as_str().unwrap_or("tool").into(),
                            input: block.get("arguments").cloned().unwrap_or(Value::Null),
                            parent_tool_use_id: None,
                        }),
                        _ => {}
                    }
                }
                let usage = &msg["usage"];
                if usage.is_object() {
                    let n = |key: &str| usage[key].as_u64().unwrap_or(0);
                    self.turn_cost += usage["cost"]["total"].as_f64().unwrap_or(0.);
                    events.push(ProviderEvent::Usage(Usage {
                        input_tokens: n("input"),
                        output_tokens: n("output"),
                        cache_read_tokens: n("cacheRead"),
                        cache_write_tokens: n("cacheWrite"),
                        cost_usd: None,
                        context_tokens: Some(n("input") + n("cacheRead") + n("cacheWrite")),
                        context_window: self.context_window,
                    }));
                }
                if msg["stopReason"].as_str() == Some("error")
                    && let Some(error) = msg["errorMessage"].as_str()
                {
                    events.push(ProviderEvent::Notice {
                        text: error.into(),
                        is_error: true,
                    });
                }
            }
            "tool_execution_update" => {
                let output = text_of(&message["partialResult"]["content"]);
                if !output.is_empty() {
                    events.push(ProviderEvent::ToolProgress {
                        tool_use_id: message["toolCallId"].as_str().unwrap_or("").into(),
                        output,
                    });
                }
            }
            "tool_execution_end" => events.push(ProviderEvent::ToolResult {
                tool_use_id: message["toolCallId"].as_str().unwrap_or("").into(),
                content: text_of(&message["result"]["content"]),
                is_error: message["isError"].as_bool().unwrap_or(false),
            }),
            "agent_end" => {
                let last = message["messages"].as_array().and_then(|m| m.last());
                let stop = last
                    .and_then(|m| m["stopReason"].as_str())
                    .unwrap_or("stop");
                let is_error = matches!(stop, "error" | "aborted");
                events.push(ProviderEvent::TurnCompleted {
                    is_error,
                    duration_ms: self
                        .turn_started
                        .take()
                        .map(|t| t.elapsed().as_millis() as u64),
                    cost_usd: (self.turn_cost > 0.).then_some(self.turn_cost),
                    message: is_error.then(|| {
                        last.and_then(|m| m["errorMessage"].as_str())
                            .unwrap_or(if stop == "aborted" {
                                "Interrupted"
                            } else {
                                "The agent stopped with an error"
                            })
                            .to_string()
                    }),
                });
            }
            "queue_update" => {
                let mut queued: Vec<String> = Vec::new();
                for key in ["steering", "followUp"] {
                    queued.extend(
                        message[key]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|m| m.as_str().map(str::to_string)),
                    );
                }
                events.push(ProviderEvent::QueueChanged(queued));
            }
            "compaction_start" => events.push(ProviderEvent::Compacting(true)),
            "compaction_end" => {
                events.push(ProviderEvent::Compacting(false));
                if let Some(error) = message["errorMessage"].as_str() {
                    events.push(ProviderEvent::Notice {
                        text: format!("Compaction failed: {error}"),
                        is_error: true,
                    });
                }
            }
            "auto_retry_start" => events.push(ProviderEvent::Retrying {
                attempt: message["attempt"].as_u64().unwrap_or(1) as u32,
                max_attempts: message["maxAttempts"].as_u64().unwrap_or(1) as u32,
                error: message["errorMessage"].as_str().unwrap_or("").into(),
            }),
            "auto_retry_end" => {
                if message["success"].as_bool() == Some(false) {
                    events.push(ProviderEvent::Notice {
                        text: message["finalError"]
                            .as_str()
                            .unwrap_or("Retry failed")
                            .into(),
                        is_error: true,
                    });
                }
            }
            "extension_error" => events.push(ProviderEvent::Warning(format!(
                "Extension error: {}",
                message["error"].as_str().unwrap_or("unknown")
            ))),
            "extension_ui_request" => self.handle_ui_request(message, &mut events),
            _ => {}
        }
        events
    }

    fn handle_response(&mut self, message: &Value, events: &mut Vec<ProviderEvent>) {
        let data = &message["data"];
        let command = message["command"].as_str().unwrap_or("");
        if message["success"].as_bool() == Some(false) {
            let error = message["error"]
                .as_str()
                .unwrap_or("command failed")
                .to_string();
            if command == "prompt" {
                events.push(ProviderEvent::TurnCompleted {
                    is_error: true,
                    duration_ms: None,
                    cost_usd: None,
                    message: Some(error),
                });
            } else {
                events.push(ProviderEvent::Warning(format!("{command}: {error}")));
            }
            return;
        }
        match command {
            "get_state" => {
                self.context_window = data["model"]["contextWindow"].as_u64();
                if let Some(session) = data["sessionFile"].as_str() {
                    events.push(ProviderEvent::SessionStarted {
                        session_id: session.into(),
                        model: model_id(&data["model"]),
                    });
                }
            }
            "set_model" => {
                self.context_window = data["contextWindow"].as_u64().or(self.context_window)
            }
            "get_commands" => events.push(ProviderEvent::Commands(
                data["commands"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| {
                        Some(SlashCommand {
                            name: c["name"].as_str()?.into(),
                            description: c["description"].as_str().unwrap_or("").into(),
                            argument_hint: String::new(),
                        })
                    })
                    .collect(),
            )),
            "get_available_models" => {
                let mut options = vec![ModelOption {
                    id: String::new(),
                    label: "Default".into(),
                }];
                options.extend(
                    data["models"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|m| {
                            Some(ModelOption {
                                id: model_id(m)?,
                                label: m["name"].as_str().or(m["id"].as_str())?.into(),
                            })
                        }),
                );
                events.push(ProviderEvent::Models(options));
            }
            "get_session_stats" => {
                let context = &data["contextUsage"];
                events.push(ProviderEvent::Usage(Usage {
                    cost_usd: data["cost"].as_f64(),
                    context_tokens: context["tokens"].as_u64(),
                    context_window: context["contextWindow"].as_u64().or(self.context_window),
                    ..Usage::default()
                }));
            }
            "compact" => {
                if let Some(before) = data["tokensBefore"].as_u64() {
                    events.push(ProviderEvent::Notice {
                        text: format!("Compacted the conversation ({before} tokens before)."),
                        is_error: false,
                    });
                }
            }
            _ => {}
        }
    }

    fn handle_ui_request(&mut self, message: &Value, events: &mut Vec<ProviderEvent>) {
        let id = message["id"].as_str().unwrap_or("").to_string();
        let title = message["title"].as_str().unwrap_or("").to_string();
        let question = |options: Vec<QuestionOption>, prefill: &str, multiline: bool| Question {
            question: title.clone(),
            header: String::new(),
            options,
            multi_select: false,
            prefill: prefill.into(),
            multiline,
        };
        let request = match message["method"].as_str().unwrap_or("") {
            "select" => Some((
                QuestionKind::Choice,
                question(
                    message["options"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|o| o.as_str())
                        .map(|label| QuestionOption {
                            label: label.into(),
                            description: String::new(),
                        })
                        .collect(),
                    "",
                    false,
                ),
            )),
            "confirm" => Some((
                QuestionKind::Confirm,
                Question {
                    header: message["message"].as_str().unwrap_or("").into(),
                    ..question(Vec::new(), "", false)
                },
            )),
            "input" => Some((
                QuestionKind::Text,
                question(
                    Vec::new(),
                    message["placeholder"].as_str().unwrap_or(""),
                    false,
                ),
            )),
            "editor" => Some((
                QuestionKind::Text,
                question(Vec::new(), message["prefill"].as_str().unwrap_or(""), true),
            )),
            "notify" => {
                events.push(ProviderEvent::Notice {
                    text: message["message"].as_str().unwrap_or("").into(),
                    is_error: matches!(message["notifyType"].as_str(), Some("error")),
                });
                None
            }
            // setStatus, setWidget, setTitle, set_editor_text: TUI chrome.
            _ => None,
        };
        if let Some((kind, question)) = request {
            events.push(ProviderEvent::Question(QuestionRequest {
                request_id: id,
                kind,
                title,
                questions: vec![question],
                input: message.clone(),
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_gateway_servers_as_json() {
        assert_eq!(mcp_servers_env(&[]), None);
        let json = mcp_servers_env(&[crate::McpServer {
            name: "elyra".into(),
            url: "http://127.0.0.1:1/mcp".into(),
            token: "secret".into(),
            bridge: None,
        }])
        .unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        let server = &value["mcpServers"]["elyra"];
        assert_eq!(server["url"], "http://127.0.0.1:1/mcp");
        assert_eq!(server["headers"]["Authorization"], "Bearer secret");
        assert_eq!(server["directTools"], true);
    }

    fn feed(parser: &mut RpcParser, line: &str) -> Vec<ProviderEvent> {
        parser.handle(&serde_json::from_str(line).unwrap())
    }

    #[test]
    fn parses_recorded_rpc_session() {
        let mut parser = RpcParser::default();
        let state = feed(
            &mut parser,
            r#"{"id":"state_1","type":"response","command":"get_state","success":true,"data":{"model":{"id":"claude-sonnet-5-5","provider":"anthropic","contextWindow":1000000},"sessionFile":"/s/a.jsonl","sessionId":"x"}}"#,
        );
        assert_eq!(
            state,
            vec![ProviderEvent::SessionStarted {
                session_id: "/s/a.jsonl".into(),
                model: Some("anthropic/claude-sonnet-5-5".into())
            }]
        );
        assert!(feed(&mut parser, r#"{"type":"agent_start"}"#).is_empty());
        assert_eq!(
            feed(
                &mut parser,
                r#"{"type":"message_update","message":{},"assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Do"}}"#
            ),
            vec![ProviderEvent::TextDelta("Do".into())]
        );
        let end = feed(
            &mut parser,
            r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"toolCall","id":"t1","name":"bash","arguments":{"command":"ls"}}],"usage":{"input":4,"output":53,"cacheRead":0,"cacheWrite":19523,"cost":{"total":0.05}},"stopReason":"toolUse"}}"#,
        );
        assert_eq!(
            end[0],
            ProviderEvent::ToolUse {
                tool_use_id: "t1".into(),
                name: "bash".into(),
                input: json!({"command":"ls"}),
                parent_tool_use_id: None
            }
        );
        let ProviderEvent::Usage(usage) = &end[1] else {
            panic!("{end:?}")
        };
        assert_eq!(
            (usage.context_tokens, usage.context_window),
            (Some(19527), Some(1_000_000))
        );
        assert_eq!(
            feed(
                &mut parser,
                r#"{"type":"tool_execution_update","toolCallId":"t1","toolName":"bash","partialResult":{"content":[{"type":"text","text":"out\n"}]}}"#
            ),
            vec![ProviderEvent::ToolProgress {
                tool_use_id: "t1".into(),
                output: "out\n".into()
            }]
        );
        assert_eq!(
            feed(
                &mut parser,
                r#"{"type":"tool_execution_end","toolCallId":"t1","toolName":"bash","result":{"content":[{"type":"text","text":"out\n"}]},"isError":false}"#
            ),
            vec![ProviderEvent::ToolResult {
                tool_use_id: "t1".into(),
                content: "out\n".into(),
                is_error: false
            }]
        );
        let done = feed(
            &mut parser,
            r#"{"type":"agent_end","messages":[{"role":"assistant","content":[],"stopReason":"stop"}]}"#,
        );
        let [
            ProviderEvent::TurnCompleted {
                is_error: false,
                cost_usd: Some(cost),
                ..
            },
        ] = done.as_slice()
        else {
            panic!("{done:?}")
        };
        assert!((cost - 0.05).abs() < 1e-9);
    }

    #[test]
    fn extension_dialogs_become_questions() {
        let mut parser = RpcParser::default();
        let events = feed(
            &mut parser,
            r#"{"type":"extension_ui_request","id":"u1","method":"select","title":"Allow dangerous command?","options":["Allow","Block"],"timeout":10000}"#,
        );
        let [ProviderEvent::Question(q)] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert_eq!(
            (q.kind, q.questions[0].options.len()),
            (QuestionKind::Choice, 2)
        );
        let events = feed(
            &mut parser,
            r#"{"type":"extension_ui_request","id":"u2","method":"confirm","title":"Clear?","message":"All lost."}"#,
        );
        let [ProviderEvent::Question(q)] = events.as_slice() else {
            panic!()
        };
        assert_eq!(
            (q.kind, q.questions[0].header.as_str()),
            (QuestionKind::Confirm, "All lost.")
        );
        let failed = feed(
            &mut parser,
            r#"{"id":"prompt_4","type":"response","command":"prompt","success":false,"error":"busy"}"#,
        );
        assert!(matches!(
            failed.as_slice(),
            [ProviderEvent::TurnCompleted { is_error: true, .. }]
        ));
    }
}
