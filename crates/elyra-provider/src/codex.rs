//! Codex adapter: drives `codex app-server` (JSON-RPC over newline-delimited
//! stdio, protocol v2).
//!
//! client → server: `initialize` (+ `initialized`), `thread/start`,
//!                  `thread/resume`, `thread/fork`, `turn/start`,
//!                  `turn/steer`, `turn/interrupt`, `thread/compact/start`,
//!                  `model/list`
//! server → client: notifications (`item/*`, `turn/*`, `thread/*`) and the
//!                  requests `item/commandExecution/requestApproval`,
//!                  `item/fileChange/requestApproval`,
//!                  `item/tool/requestUserInput`

use crate::process::JsonlProcess;
use crate::{
    AgentSession, Capabilities, ModelOption, PermissionRequest, PermissionResponse, Prompt,
    ProviderEvent, Question, QuestionAnswer, QuestionKind, QuestionOption, QuestionRequest,
    SessionConfig, Usage,
};
use anyhow::{Result, anyhow};
use elyra_core::PermissionMode;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const CAPABILITIES: Capabilities = Capabilities {
    permission_modes: true,
    efforts: &[
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

/// Environment variable carrying the agent-gateway token to Codex.
const GATEWAY_TOKEN_ENV: &str = "ELYRA_MCP_TOKEN";

pub fn find_executable() -> Option<PathBuf> {
    crate::process::find_executable("codex")
}

/// Codex approval policy and sandbox for a permission mode.
fn policy(mode: PermissionMode) -> (&'static str, &'static str) {
    match mode {
        PermissionMode::Ask => ("untrusted", "read-only"),
        PermissionMode::AcceptEdits => ("on-request", "workspace-write"),
        PermissionMode::Plan => ("never", "read-only"),
        PermissionMode::FullAccess => ("never", "danger-full-access"),
    }
}

fn sandbox_policy(mode: PermissionMode) -> Value {
    match policy(mode).1 {
        "danger-full-access" => json!({ "type": "dangerFullAccess" }),
        "workspace-write" => json!({
            "type": "workspaceWrite",
            "writableRoots": [],
            "networkAccess": false,
            "excludeTmpdirEnvVar": false,
            "excludeSlashTmp": false
        }),
        _ => json!({ "type": "readOnly", "networkAccess": false }),
    }
}

enum Pending {
    Initialize,
    Thread,
    Turn,
    Models,
    Ignore,
}

/// A request from Codex waiting for the user.
enum Waiting {
    Approval {
        id: Value,
    },
    Input {
        id: Value,
        question_ids: Vec<String>,
    },
}

struct ToolState {
    name: String,
    input: Value,
    output: String,
}

struct State {
    next_id: u64,
    pending: HashMap<u64, Pending>,
    thread_id: Option<String>,
    turn_id: Option<String>,
    turn_started: Option<Instant>,
    queued: Vec<Value>,
    waiting: HashMap<String, Waiting>,
    mode: PermissionMode,
    model: Option<String>,
    effort: Option<String>,
    cwd: PathBuf,
    resume: Option<String>,
    fork: bool,
    instructions: Option<String>,
    text: HashMap<String, String>,
    thinking: HashMap<String, String>,
    tools: HashMap<String, ToolState>,
    plan: Option<Value>,
    context_window: Option<u64>,
}

type Writer = Arc<Mutex<Option<Arc<JsonlProcess>>>>;

pub struct CodexSession {
    process: Arc<JsonlProcess>,
    state: Arc<Mutex<State>>,
}

fn request_frame(state: &mut State, method: &str, params: Value, pending: Pending) -> Value {
    let id = state.next_id;
    state.next_id += 1;
    state.pending.insert(id, pending);
    json!({ "id": id, "method": method, "params": params })
}

fn user_input(prompt: &Prompt) -> Value {
    let mut input = vec![json!({ "type": "text", "text": prompt.text, "text_elements": [] })];
    for image in &prompt.images {
        input.push(json!({
            "type": "image",
            "url": format!("data:{};base64,{}", image.media_type, image.data)
        }));
    }
    Value::Array(input)
}

/// `turn/start` for a prompt, carrying the current model, effort and policy.
fn turn_frame(state: &mut State, input: Value) -> Option<Value> {
    let thread_id = state.thread_id.clone()?;
    let (approval, _) = policy(state.mode);
    let mut params = json!({
        "threadId": thread_id,
        "input": input,
        "approvalPolicy": approval,
        "sandboxPolicy": sandbox_policy(state.mode),
    });
    if let Some(model) = state.model.clone() {
        params["model"] = Value::String(model);
    }
    if let Some(effort) = state.effort.clone() {
        params["effort"] = Value::String(effort);
    }
    state.turn_started = Some(Instant::now());
    Some(request_frame(state, "turn/start", params, Pending::Turn))
}

impl CodexSession {
    pub fn start(config: SessionConfig) -> Result<(Self, async_channel::Receiver<ProviderEvent>)> {
        let executable = config
            .executable
            .clone()
            .or_else(find_executable)
            .ok_or_else(|| {
                anyhow!("Codex (`codex`) was not found on PATH — install with `npm install -g @openai/codex`")
            })?;
        let mut args: Vec<String> = vec!["app-server".into()];
        args.extend(config.args.iter().cloned());
        let mut env = config.env.clone();
        // MCP servers (the agent gateway) go in as config overrides; the
        // token travels in the environment, not on the command line.
        for server in &config.mcp_servers {
            let name = &server.name;
            if let Some((command, command_args)) = &server.stdio {
                args.extend([
                    "-c".into(),
                    format!(
                        "mcp_servers.{name}.command={:?}",
                        command.display().to_string()
                    ),
                    "-c".into(),
                    format!(
                        "mcp_servers.{name}.args={}",
                        serde_json::to_string(command_args).unwrap_or_else(|_| "[]".into())
                    ),
                ]);
                continue;
            }
            // Only the gateway is an HTTP server, so one token variable will do.
            args.extend([
                "-c".into(),
                format!("mcp_servers.{name}.url={:?}", server.url),
                "-c".into(),
                format!("mcp_servers.{name}.bearer_token_env_var={GATEWAY_TOKEN_ENV:?}"),
            ]);
            env.push((GATEWAY_TOKEN_ENV.into(), server.token.clone()));
        }
        let state = Arc::new(Mutex::new(State {
            next_id: 1,
            pending: HashMap::new(),
            thread_id: None,
            turn_id: None,
            turn_started: None,
            queued: Vec::new(),
            waiting: HashMap::new(),
            mode: config.permission_mode,
            model: config.model.clone().filter(|m| !m.is_empty()),
            effort: config.effort.clone().filter(|e| !e.is_empty()),
            cwd: config.cwd.clone(),
            resume: config.resume_session_id.clone(),
            fork: config.fork,
            instructions: config
                .append_system_prompt
                .clone()
                .filter(|i| !i.trim().is_empty()),
            text: HashMap::new(),
            thinking: HashMap::new(),
            tools: HashMap::new(),
            plan: None,
            context_window: None,
        }));
        let writer: Writer = Arc::new(Mutex::new(None));
        let (reader_state, reader_writer) = (state.clone(), writer.clone());
        let envs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let (process, rx) = JsonlProcess::spawn(
            &executable,
            &args,
            &config.cwd,
            &envs,
            move |message| handle_message(message, &reader_state, &reader_writer),
            |_| {},
        )?;
        *writer.lock().unwrap() = Some(process.clone());
        let session = Self { process, state };
        let frame = {
            let mut state = session.state.lock().unwrap();
            request_frame(
                &mut state,
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "elyra_workspace",
                        "title": "Elyra Workspace",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": { "experimentalApi": true }
                }),
                Pending::Initialize,
            )
        };
        session.process.write(&frame)?;
        Ok((session, rx))
    }

    fn write_request(
        &self,
        method: &str,
        params: impl FnOnce(&State) -> Option<Value>,
    ) -> Result<()> {
        let frame = {
            let mut state = self.state.lock().unwrap();
            let Some(params) = params(&state) else {
                return Ok(());
            };
            request_frame(&mut state, method, params, Pending::Ignore)
        };
        self.process.write(&frame)
    }
}

impl AgentSession for CodexSession {
    fn send(&self, prompt: &Prompt) -> Result<()> {
        let frame = {
            let mut state = self.state.lock().unwrap();
            match turn_frame(&mut state, user_input(prompt)) {
                Some(frame) => frame,
                None => {
                    state.queued.push(user_input(prompt));
                    return Ok(());
                }
            }
        };
        self.process.write(&frame)
    }

    fn steer(&self, prompt: &Prompt) -> Result<()> {
        let can_steer = {
            let state = self.state.lock().unwrap();
            state.thread_id.is_some() && state.turn_id.is_some()
        };
        if !can_steer {
            return self.send(prompt);
        }
        self.write_request("turn/steer", |state| {
            Some(json!({
                "threadId": state.thread_id.clone()?,
                "input": user_input(prompt),
                "expectedTurnId": state.turn_id.clone()?,
            }))
        })
    }

    fn respond_permission(&self, request_id: &str, response: PermissionResponse) -> Result<()> {
        let Some(Waiting::Approval { id }) = self.state.lock().unwrap().waiting.remove(request_id)
        else {
            return Ok(());
        };
        let decision = match response {
            PermissionResponse::Allow => "accept",
            PermissionResponse::AllowForSession => "acceptForSession",
            PermissionResponse::Deny { .. } => "decline",
        };
        self.process
            .write(&json!({ "id": id, "result": { "decision": decision } }))
    }

    fn answer_question(&self, request_id: &str, answer: QuestionAnswer) -> Result<()> {
        let Some(Waiting::Input { id, question_ids }) =
            self.state.lock().unwrap().waiting.remove(request_id)
        else {
            return Ok(());
        };
        let answers: serde_json::Map<String, Value> = match answer {
            QuestionAnswer::Answers { answers } => question_ids
                .into_iter()
                .zip(answers)
                .map(|(qid, text)| {
                    let parts: Vec<String> = text
                        .split(", ")
                        .map(str::to_string)
                        .filter(|s| !s.is_empty())
                        .collect();
                    (qid, json!({ "answers": parts }))
                })
                .collect(),
            QuestionAnswer::Confirmed { confirmed } => question_ids
                .into_iter()
                .map(|qid| {
                    (
                        qid,
                        json!({ "answers": [if confirmed { "Yes" } else { "No" }] }),
                    )
                })
                .collect(),
            QuestionAnswer::Cancelled => serde_json::Map::new(),
        };
        self.process
            .write(&json!({ "id": id, "result": { "answers": answers } }))
    }

    fn interrupt(&self) -> Result<()> {
        // Requests still waiting for the user are cancelled with the turn.
        let waiting: Vec<Waiting> = {
            let mut state = self.state.lock().unwrap();
            state.waiting.drain().map(|(_, w)| w).collect()
        };
        for waiting in waiting {
            match waiting {
                Waiting::Approval { id } => self
                    .process
                    .write(&json!({ "id": id, "result": { "decision": "cancel" } }))?,
                Waiting::Input { id, .. } => self
                    .process
                    .write(&json!({ "id": id, "result": { "answers": {} } }))?,
            }
        }
        self.write_request("turn/interrupt", |state| {
            Some(json!({ "threadId": state.thread_id.clone()?, "turnId": state.turn_id.clone()? }))
        })
    }

    fn set_permission_mode(&self, mode: PermissionMode) -> Result<()> {
        // Applied with the next turn.
        self.state.lock().unwrap().mode = mode;
        Ok(())
    }

    fn set_model(&self, model: Option<&str>) -> Result<()> {
        self.state.lock().unwrap().model = model.filter(|m| !m.is_empty()).map(str::to_string);
        Ok(())
    }

    fn set_effort(&self, effort: Option<&str>) -> Result<bool> {
        self.state.lock().unwrap().effort = effort.filter(|e| !e.is_empty()).map(str::to_string);
        Ok(true)
    }

    fn compact(&self) -> Result<()> {
        self.write_request("thread/compact/start", |state| {
            Some(json!({ "threadId": state.thread_id.clone()? }))
        })
    }

    fn shutdown(&self) {
        self.process.shutdown();
    }
}

impl Drop for CodexSession {
    fn drop(&mut self) {
        self.process.shutdown();
    }
}

// ---- reader thread ------------------------------------------------------

fn handle_message(
    message: &Value,
    state: &Arc<Mutex<State>>,
    writer: &Writer,
) -> Vec<ProviderEvent> {
    let mut events = Vec::new();
    let mut outgoing = Vec::new();
    {
        let mut state = state.lock().unwrap();
        let method = message["method"].as_str();
        let id = message.get("id").filter(|id| !id.is_null());
        match (method, id) {
            (None, Some(id)) => {
                if let Some(pending) = id.as_u64().and_then(|id| state.pending.remove(&id)) {
                    handle_response(&mut state, pending, message, &mut events, &mut outgoing);
                }
            }
            (Some(method), None) => {
                handle_notification(&mut state, method, &message["params"], &mut events)
            }
            (Some(method), Some(id)) => handle_request(
                &mut state,
                method,
                id,
                &message["params"],
                &mut events,
                &mut outgoing,
            ),
            _ => {}
        }
    }
    if let Some(process) = writer.lock().unwrap().as_ref() {
        for frame in outgoing {
            if let Err(err) = process.write(&frame) {
                log::warn!("codex write: {err:#}");
            }
        }
    }
    events
}

fn rpc_error(message: &Value) -> Option<String> {
    let error = message.get("error")?;
    Some(
        error["message"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string()),
    )
}

fn handle_response(
    state: &mut State,
    pending: Pending,
    message: &Value,
    events: &mut Vec<ProviderEvent>,
    outgoing: &mut Vec<Value>,
) {
    let result = &message["result"];
    match pending {
        Pending::Initialize => {
            if let Some(error) = rpc_error(message) {
                events.push(ProviderEvent::Exited {
                    error: Some(format!("Codex refused to start: {error}")),
                });
                return;
            }
            outgoing.push(json!({ "method": "initialized" }));
            let (approval, sandbox) = policy(state.mode);
            let mut params = json!({
                "cwd": state.cwd.display().to_string(),
                "approvalPolicy": approval,
                "sandbox": sandbox,
            });
            if let Some(model) = state.model.clone() {
                params["model"] = Value::String(model);
            }
            if let Some(instructions) = state.instructions.clone() {
                params["developerInstructions"] = Value::String(instructions);
            }
            let method = match state.resume.clone() {
                Some(thread_id) => {
                    params["threadId"] = Value::String(thread_id);
                    if state.fork {
                        "thread/fork"
                    } else {
                        "thread/resume"
                    }
                }
                None => "thread/start",
            };
            if method == "thread/resume" {
                params["excludeTurns"] = Value::Bool(true);
            }
            outgoing.push(request_frame(state, method, params, Pending::Thread));
            outgoing.push(request_frame(
                state,
                "model/list",
                json!({}),
                Pending::Models,
            ));
        }
        Pending::Thread => {
            let thread_id = result["thread"]["id"].as_str().map(str::to_string);
            let Some(thread_id) = thread_id else {
                let error = rpc_error(message).unwrap_or_else(|| "no thread id".into());
                if state.resume.take().is_some() {
                    // The saved session is gone; start a fresh one.
                    state.fork = false;
                    let (approval, sandbox) = policy(state.mode);
                    let params = json!({
                        "cwd": state.cwd.display().to_string(),
                        "approvalPolicy": approval,
                        "sandbox": sandbox,
                    });
                    outgoing.push(request_frame(
                        state,
                        "thread/start",
                        params,
                        Pending::Thread,
                    ));
                    events.push(ProviderEvent::Warning(format!(
                        "Could not resume the Codex session ({error}); started a new one."
                    )));
                } else {
                    events.push(ProviderEvent::Exited {
                        error: Some(format!("Could not start a Codex thread: {error}")),
                    });
                }
                return;
            };
            state.thread_id = Some(thread_id.clone());
            events.push(ProviderEvent::SessionStarted {
                session_id: thread_id,
                model: result["model"].as_str().map(str::to_string),
            });
            for input in std::mem::take(&mut state.queued) {
                if let Some(frame) = turn_frame(state, input) {
                    outgoing.push(frame);
                }
            }
        }
        Pending::Turn => {
            if let Some(error) = rpc_error(message) {
                events.push(ProviderEvent::TurnCompleted {
                    is_error: true,
                    duration_ms: None,
                    cost_usd: None,
                    message: Some(error),
                });
            } else if let Some(turn_id) = result["turn"]["id"].as_str() {
                state.turn_id = Some(turn_id.to_string());
            }
        }
        Pending::Models => {
            let models: Vec<ModelOption> = result["data"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|m| !m["hidden"].as_bool().unwrap_or(false))
                .filter_map(|m| {
                    let id = m["model"]
                        .as_str()
                        .or_else(|| m["id"].as_str())?
                        .to_string();
                    let label = m["displayName"].as_str().unwrap_or(&id).to_string();
                    Some(ModelOption { id, label })
                })
                .collect();
            if !models.is_empty() {
                let mut options = vec![ModelOption {
                    id: String::new(),
                    label: "Default".into(),
                }];
                options.extend(models);
                events.push(ProviderEvent::Models(options));
            }
        }
        Pending::Ignore => {
            if let Some(error) = rpc_error(message) {
                events.push(ProviderEvent::Warning(error));
            }
        }
    }
}

/// Turn a unified diff into old/new text for the edit preview.
fn split_diff(diff: &str) -> (String, String) {
    let (mut old, mut new) = (Vec::new(), Vec::new());
    for line in diff.lines() {
        if line.starts_with("---") || line.starts_with("+++") || line.starts_with("@@") {
            continue;
        }
        match line.split_at(line.len().min(1)) {
            ("-", rest) => old.push(rest),
            ("+", rest) => new.push(rest),
            (" ", rest) => {
                old.push(rest);
                new.push(rest);
            }
            _ => {}
        }
    }
    (old.join("\n"), new.join("\n"))
}

/// Map a Codex item onto the tool names the transcript renders.
pub(crate) fn tool_for_item(item: &Value) -> Option<(String, Value)> {
    match item["type"].as_str()? {
        "commandExecution" => Some((
            "Bash".into(),
            json!({ "command": item["command"], "cwd": item["cwd"] }),
        )),
        "fileChange" => {
            let changes = item["changes"].as_array()?;
            let first = changes.first()?;
            let path = first["path"].as_str().unwrap_or("").to_string();
            let diff = first["diff"].as_str().unwrap_or("");
            let added = first["kind"]["type"].as_str() == Some("add");
            let (old, new) = split_diff(diff);
            let mut input = json!({ "file_path": path });
            if changes.len() > 1 {
                input["description"] = Value::String(format!("{} files", changes.len()));
            }
            if added {
                input["content"] = Value::String(new);
                Some(("Write".into(), input))
            } else {
                input["old_string"] = Value::String(old);
                input["new_string"] = Value::String(new);
                Some(("Edit".into(), input))
            }
        }
        "mcpToolCall" => Some((
            format!(
                "mcp__{}__{}",
                item["server"].as_str().unwrap_or("mcp"),
                item["tool"].as_str().unwrap_or("tool")
            ),
            item["arguments"].clone(),
        )),
        "dynamicToolCall" => Some((
            item["tool"].as_str().unwrap_or("tool").to_string(),
            item["arguments"].clone(),
        )),
        "webSearch" => Some(("WebSearch".into(), json!({ "query": item["query"] }))),
        "collabAgentToolCall" => Some((
            "Task".into(),
            json!({ "description": item["prompt"].as_str().unwrap_or("Subagent") }),
        )),
        _ => None,
    }
}

pub(crate) fn mcp_result_text(result: &Value) -> String {
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn handle_notification(
    state: &mut State,
    method: &str,
    params: &Value,
    events: &mut Vec<ProviderEvent>,
) {
    let item_id = params["itemId"].as_str().unwrap_or("").to_string();
    match method {
        "turn/started" => {
            if let Some(turn_id) = params["turn"]["id"].as_str() {
                state.turn_id = Some(turn_id.to_string());
            }
        }
        "item/agentMessage/delta" => {
            let delta = params["delta"].as_str().unwrap_or("");
            state.text.entry(item_id).or_default().push_str(delta);
            events.push(ProviderEvent::TextDelta(delta.to_string()));
        }
        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
            let delta = params["delta"].as_str().unwrap_or("");
            state.thinking.entry(item_id).or_default().push_str(delta);
            events.push(ProviderEvent::ThinkingDelta(delta.to_string()));
        }
        "item/commandExecution/outputDelta" => {
            if let Some(tool) = state.tools.get_mut(&item_id) {
                tool.output.push_str(params["delta"].as_str().unwrap_or(""));
                events.push(ProviderEvent::ToolProgress {
                    tool_use_id: item_id,
                    output: tool.output.clone(),
                });
            }
        }
        "item/started" => {
            let item = &params["item"];
            let id = item["id"].as_str().unwrap_or("").to_string();
            if let Some((name, input)) = tool_for_item(item) {
                state.tools.insert(
                    id.clone(),
                    ToolState {
                        name: name.clone(),
                        input: input.clone(),
                        output: String::new(),
                    },
                );
                events.push(ProviderEvent::ToolUse {
                    tool_use_id: id,
                    name,
                    input,
                    parent_tool_use_id: None,
                });
            } else if item["type"] == "contextCompaction" {
                events.push(ProviderEvent::Compacting(true));
            }
        }
        "item/completed" => {
            let item = &params["item"];
            let id = item["id"].as_str().unwrap_or("").to_string();
            match item["type"].as_str().unwrap_or("") {
                "agentMessage" => {
                    state.text.remove(&id);
                    let text = item["text"].as_str().unwrap_or("").to_string();
                    if !text.trim().is_empty() {
                        events.push(ProviderEvent::AssistantText {
                            text,
                            parent_tool_use_id: None,
                        });
                    }
                }
                "plan" => {
                    let text = item["text"].as_str().unwrap_or("").to_string();
                    if !text.trim().is_empty() {
                        events.push(ProviderEvent::AssistantText {
                            text,
                            parent_tool_use_id: None,
                        });
                    }
                }
                "reasoning" => {
                    let streamed = state.thinking.remove(&id).unwrap_or_default();
                    let summary = item["summary"]
                        .as_array()
                        .map(|parts| {
                            parts
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("\n\n")
                        })
                        .unwrap_or_default();
                    let text = if summary.trim().is_empty() {
                        streamed
                    } else {
                        summary
                    };
                    events.push(ProviderEvent::Thinking { text });
                }
                "contextCompaction" => events.push(ProviderEvent::Compacting(false)),
                _ => {
                    let Some(tool) = state.tools.remove(&id) else {
                        return;
                    };
                    let status = item["status"].as_str().unwrap_or("completed");
                    let failed = matches!(status, "failed" | "declined")
                        || item["exitCode"].as_i64().is_some_and(|code| code != 0)
                        || item["success"].as_bool() == Some(false)
                        || !item["error"].is_null() && item.get("error").is_some();
                    let content = match item["type"].as_str() {
                        Some("commandExecution") => item["aggregatedOutput"]
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or(tool.output),
                        Some("fileChange") => match status {
                            "declined" => "Declined".to_string(),
                            "failed" => "Failed to apply the change".to_string(),
                            _ => format!(
                                "Updated {}",
                                tool.input["file_path"].as_str().unwrap_or("file")
                            ),
                        },
                        Some("mcpToolCall") => {
                            if item["error"].is_object() {
                                item["error"]["message"]
                                    .as_str()
                                    .unwrap_or("Error")
                                    .to_string()
                            } else {
                                mcp_result_text(&item["result"])
                            }
                        }
                        _ => String::new(),
                    };
                    let _ = tool.name;
                    events.push(ProviderEvent::ToolResult {
                        tool_use_id: id,
                        content,
                        is_error: failed,
                    });
                }
            }
        }
        "turn/plan/updated" => {
            let todos: Vec<Value> = params["plan"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|step| {
                    let text = step["step"].as_str().unwrap_or("");
                    let status = match step["status"].as_str() {
                        Some("inProgress") => "in_progress",
                        Some("completed") => "completed",
                        _ => "pending",
                    };
                    json!({ "content": text, "activeForm": text, "status": status })
                })
                .collect();
            let input = json!({ "todos": todos });
            if state.plan.as_ref() != Some(&input) {
                state.plan = Some(input.clone());
                events.push(ProviderEvent::ToolUse {
                    tool_use_id: format!("plan-{}", elyra_core::new_id().simple()),
                    name: "TodoWrite".into(),
                    input,
                    parent_tool_use_id: None,
                });
            }
        }
        "thread/tokenUsage/updated" => {
            let usage = &params["tokenUsage"];
            let last = &usage["last"];
            let n = |key: &str| last[key].as_u64().unwrap_or(0);
            if let Some(window) = usage["modelContextWindow"].as_u64() {
                state.context_window = Some(window);
            }
            events.push(ProviderEvent::Usage(Usage {
                input_tokens: n("inputTokens"),
                output_tokens: n("outputTokens"),
                cache_read_tokens: n("cachedInputTokens"),
                cache_write_tokens: n("cacheWriteInputTokens"),
                cost_usd: None,
                context_tokens: last["totalTokens"].as_u64(),
                context_window: state.context_window,
            }));
        }
        "turn/completed" => {
            let turn = &params["turn"];
            let status = turn["status"].as_str().unwrap_or("completed");
            state.turn_id = None;
            state.text.clear();
            state.thinking.clear();
            let duration = turn["durationMs"]
                .as_u64()
                .or_else(|| state.turn_started.map(|t| t.elapsed().as_millis() as u64));
            events.push(ProviderEvent::TurnCompleted {
                is_error: status == "failed",
                duration_ms: duration,
                cost_usd: None,
                message: turn["error"]["message"].as_str().map(str::to_string),
            });
        }
        "error" => {
            let message = params["error"]["message"]
                .as_str()
                .unwrap_or("Codex error")
                .to_string();
            if params["willRetry"].as_bool() == Some(true) {
                // Codex reports progress as "Reconnecting... 2/5".
                let (attempt, max_attempts) = message
                    .rsplit(' ')
                    .next()
                    .and_then(|tail| tail.split_once('/'))
                    .and_then(|(a, m)| Some((a.parse().ok()?, m.parse().ok()?)))
                    .unwrap_or((1, 1));
                events.push(ProviderEvent::Retrying {
                    attempt,
                    max_attempts,
                    error: message,
                });
            } else {
                events.push(ProviderEvent::Notice {
                    text: message,
                    is_error: true,
                });
            }
        }
        "thread/compacted" => events.push(ProviderEvent::Compacting(false)),
        "warning" | "configWarning" | "deprecationNotice" => {
            if let Some(text) = params["message"]
                .as_str()
                .or_else(|| params["summary"].as_str())
            {
                events.push(ProviderEvent::Warning(text.to_string()));
            }
        }
        _ => {}
    }
}

fn handle_request(
    state: &mut State,
    method: &str,
    id: &Value,
    params: &Value,
    events: &mut Vec<ProviderEvent>,
    outgoing: &mut Vec<Value>,
) {
    let request_id = format!("codex-{}", id.to_string().trim_matches('"'));
    let item_id = params["itemId"].as_str().unwrap_or("").to_string();
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            let (tool_name, mut input) = match state.tools.get(&item_id) {
                Some(tool) => (tool.name.clone(), tool.input.clone()),
                None if method.contains("commandExecution") => (
                    "Bash".to_string(),
                    json!({ "command": params["command"], "cwd": params["cwd"] }),
                ),
                None => ("Edit".to_string(), json!({})),
            };
            if let Some(command) = params["command"].as_str() {
                input["command"] = Value::String(command.to_string());
            }
            state.waiting.insert(request_id.clone(), Waiting::Approval { id: id.clone() });
            events.push(ProviderEvent::PermissionRequest(PermissionRequest {
                request_id,
                tool_name,
                tool_use_id: Some(item_id),
                description: params["reason"].as_str().map(str::to_string),
                input,
                suggestions: Value::Null,
            }));
        }
        "item/tool/requestUserInput" => {
            let raw = params["questions"].as_array().cloned().unwrap_or_default();
            let question_ids: Vec<String> = raw
                .iter()
                .map(|q| q["id"].as_str().unwrap_or("").to_string())
                .collect();
            let questions: Vec<Question> = raw
                .iter()
                .map(|q| Question {
                    question: q["question"].as_str().unwrap_or("").to_string(),
                    header: q["header"].as_str().unwrap_or("").to_string(),
                    options: q["options"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|o| QuestionOption {
                            label: o["label"].as_str().unwrap_or("").to_string(),
                            description: o["description"].as_str().unwrap_or("").to_string(),
                        })
                        .collect(),
                    multi_select: false,
                    prefill: String::new(),
                    multiline: false,
                })
                .collect();
            let kind = if questions.iter().all(|q| q.options.is_empty()) {
                QuestionKind::Text
            } else {
                QuestionKind::Choice
            };
            state.waiting.insert(
                request_id.clone(),
                Waiting::Input {
                    id: id.clone(),
                    question_ids,
                },
            );
            events.push(ProviderEvent::Question(QuestionRequest {
                request_id,
                kind,
                title: String::new(),
                questions,
                input: params.clone(),
            }));
        }
        other => outgoing.push(json!({
            "id": id,
            "error": { "code": -32601, "message": format!("not supported by Elyra Workspace: {other}") }
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const MOCK: &str = include_str!("../tests/mock_codex_app_server.py");

    fn wait_for<T>(
        rx: &async_channel::Receiver<ProviderEvent>,
        seen: &mut Vec<ProviderEvent>,
        mut pick: impl FnMut(&ProviderEvent) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match rx.try_recv() {
                Ok(event) => {
                    let picked = pick(&event);
                    seen.push(event);
                    if let Some(value) = picked {
                        return value;
                    }
                }
                Err(_) => {
                    assert!(Instant::now() < deadline, "timed out; seen: {seen:#?}");
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    #[test]
    fn splits_unified_diffs() {
        let (old, new) = split_diff("--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n keep\n-old\n+new\n");
        assert_eq!((old.as_str(), new.as_str()), ("keep\nold", "keep\nnew"));
    }

    #[test]
    fn runs_turns_against_a_mock_app_server() {
        let dir = std::env::temp_dir().join(format!("elyra-codex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("mock_codex.py");
        std::fs::write(&script, MOCK).unwrap();
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // The script stands in for the `codex` binary.
        let config = SessionConfig {
            cwd: dir.clone(),
            model: Some("gpt-test".into()),
            effort: Some("high".into()),
            permission_mode: PermissionMode::Ask,
            resume_session_id: None,
            executable: Some(script.clone()),
            args: vec![],
            env: vec![],
            fork: false,
            append_system_prompt: Some("Be brief.".into()),
            mcp_servers: vec![crate::McpServer {
                stdio: None,
                name: "elyra".into(),
                url: "http://127.0.0.1:1/mcp".into(),
                token: "secret".into(),
                bridge: None,
            }],
        };
        let (session, rx) = CodexSession::start(config).unwrap();
        session.send(&Prompt::text("run ls")).unwrap();
        let mut seen = Vec::new();
        let thread = wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::SessionStarted { session_id, .. } => Some(session_id.clone()),
            _ => None,
        });
        assert_eq!(thread, "thr_1");
        let request = wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::PermissionRequest(r) => Some(r.clone()),
            _ => None,
        });
        assert_eq!(request.tool_name, "Bash");
        assert_eq!(request.input["command"], "ls -la");
        session
            .respond_permission(&request.request_id, PermissionResponse::Allow)
            .unwrap();
        let failed = wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::TurnCompleted { is_error, .. } => Some(*is_error),
            _ => None,
        });
        assert!(!failed);
        let has = |pred: &dyn Fn(&ProviderEvent) -> bool| seen.iter().any(pred);
        assert!(has(
            &|e| matches!(e, ProviderEvent::Models(m) if m.iter().any(|m| m.id == "gpt-test"))
        ));
        assert!(has(
            &|e| matches!(e, ProviderEvent::Thinking { text } if text == "Listing files")
        ));
        assert!(has(
            &|e| matches!(e, ProviderEvent::ToolProgress { output, .. } if output == "a.txt\n")
        ));
        assert!(has(
            &|e| matches!(e, ProviderEvent::ToolResult { content, is_error: false, .. } if content == "a.txt\nb.txt\n")
        ));
        assert!(has(
            &|e| matches!(e, ProviderEvent::ToolUse { name, .. } if name == "TodoWrite")
        ));
        assert!(has(
            &|e| matches!(e, ProviderEvent::Usage(u) if u.context_window == Some(200000))
        ));
        // The mock echoes what it received: model, effort, policy,
        // instructions, the gateway config and its token.
        assert!(has(
            &|e| matches!(e, ProviderEvent::AssistantText { text, .. }
            if text == "model=gpt-test effort=high approval=untrusted sandbox=read-only dev=Be brief. mcp=http://127.0.0.1:1/mcp token=secret")
        ));

        // An edit with a question in between; the answer reaches the server.
        session.send(&Prompt::text("edit file")).unwrap();
        let question = wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::Question(q) => Some(q.clone()),
            _ => None,
        });
        assert_eq!(question.questions[0].options.len(), 2);
        session
            .answer_question(
                &question.request_id,
                QuestionAnswer::Answers {
                    answers: vec!["Tabs".into()],
                },
            )
            .unwrap();
        let edit = wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::ToolUse { name, input, .. } if name == "Edit" => Some(input.clone()),
            _ => None,
        });
        assert_eq!(edit["old_string"], "a\nold");
        assert_eq!(edit["new_string"], "a\nnew");
        wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::AssistantText { text, .. } if text == "answered Tabs" => Some(()),
            _ => None,
        });

        // Interrupt cancels a pending approval and ends the turn.
        session.send(&Prompt::text("run ls")).unwrap();
        wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::PermissionRequest(_) => Some(()),
            _ => None,
        });
        session.interrupt().unwrap();
        let message = wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::TurnCompleted {
                is_error, message, ..
            } => Some((*is_error, message.clone())),
            _ => None,
        });
        assert_eq!(message, (false, None));
        session.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
