//! Agent Client Protocol adapter (https://agentclientprotocol.com): JSON-RPC
//! 2.0 over newline-delimited stdio, spoken by Gemini CLI, Cursor Agent,
//! OpenCode and others.
//!
//! client → agent: `initialize`, `session/new`, `session/load`,
//!                 `session/prompt`, `session/set_mode`, `session/set_model`,
//!                 `session/cancel` (notification)
//! agent → client: `session/update` notifications, and the requests
//!                 `session/request_permission`, `fs/read_text_file`,
//!                 `fs/write_text_file`

use crate::process::JsonlProcess;
use crate::{
    AgentSession, Capabilities, ModelOption, PermissionRequest, PermissionResponse, Prompt,
    ProviderEvent, QuestionAnswer, SessionConfig, SlashCommand, Usage,
};
use anyhow::{Result, anyhow};
use elyra_core::{PermissionMode, ProviderKind};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const PROTOCOL_VERSION: u64 = 1;

pub const CAPABILITIES: Capabilities = Capabilities {
    permission_modes: true,
    efforts: &[],
    models: &[("", "Default")],
    steer: false,
    compact: false,
    images: true,
};

/// How to launch a known ACP agent.
#[derive(Clone, Copy, Debug)]
pub struct AcpAgent {
    pub binary: &'static str,
    pub args: &'static [&'static str],
    pub install_hint: &'static str,
}

pub fn agent(kind: ProviderKind) -> Option<AcpAgent> {
    match kind {
        ProviderKind::Gemini => Some(AcpAgent {
            binary: "gemini",
            args: &["--experimental-acp"],
            install_hint: "npm install -g @google/gemini-cli",
        }),
        ProviderKind::Cursor => Some(AcpAgent {
            binary: "cursor-agent",
            args: &["acp"],
            install_hint: "curl https://cursor.com/install -fsS | bash",
        }),
        ProviderKind::OpenCode => Some(AcpAgent {
            binary: "opencode",
            args: &["acp"],
            install_hint: "npm install -g opencode-ai",
        }),
        _ => None,
    }
}

pub fn is_acp(kind: ProviderKind) -> bool {
    agent(kind).is_some() || kind == ProviderKind::CustomAcp
}

pub fn find_executable(kind: ProviderKind) -> Option<PathBuf> {
    crate::process::find_executable(agent(kind)?.binary)
}

/// What the reader thread is waiting for, by JSON-RPC id.
enum Pending {
    Initialize,
    NewSession,
    LoadSession(String),
    Prompt(Instant),
    Ignore,
}

struct State {
    next_id: u64,
    pending: HashMap<u64, Pending>,
    session_id: Option<String>,
    /// Prompts sent before the session was ready.
    queued: Vec<Value>,
    /// Permission requests awaiting the user: our id → (rpc id, options).
    permissions: HashMap<String, (Value, Vec<Value>)>,
    modes: Vec<String>,
    permission_mode: PermissionMode,
    model: Option<String>,
    cwd: PathBuf,
    resume: Option<String>,
    load_session: bool,
    /// Replayed history from `session/load` is already in the transcript.
    loading: bool,
    text: String,
    thinking: String,
    /// Tool calls already reported, so updates don't duplicate them.
    tools: HashMap<String, ToolState>,
    plan: Option<Value>,
    mcp_servers: Vec<crate::McpServer>,
    /// The agent accepts HTTP MCP servers (else stdio bridges are used).
    mcp_http: bool,
}

impl State {
    fn mcp_servers(&self) -> Value {
        Value::Array(
            self.mcp_servers
                .iter()
                .filter_map(|server| {
                    if let Some((command, args)) = &server.stdio {
                        return Some(json!({
                            "name": server.name,
                            "command": command.display().to_string(),
                            "args": args,
                            "env": []
                        }));
                    }
                    if self.mcp_http {
                        return Some(json!({
                            "type": "http",
                            "name": server.name,
                            "url": server.url,
                            "headers": [{ "name": "Authorization", "value": format!("Bearer {}", server.token) }]
                        }));
                    }
                    let (command, args) = server.bridge.as_ref()?;
                    Some(json!({
                        "name": server.name,
                        "command": command.display().to_string(),
                        "args": args,
                        "env": []
                    }))
                })
                .collect(),
        )
    }
}

/// A tool call as reported so far. Agents often announce a call before its
/// arguments, so the transcript row is emitted once the details arrive.
#[derive(Default)]
struct ToolState {
    call: Value,
    name: String,
    input: Value,
    emitted: bool,
    output: String,
    done: bool,
}

impl ToolState {
    fn merge(&mut self, update: &Value) {
        if !self.call.is_object() {
            self.call = json!({});
        }
        for key in [
            "toolCallId",
            "title",
            "kind",
            "rawInput",
            "locations",
            "status",
        ] {
            if let Some(value) = update.get(key).filter(|v| !v.is_null()) {
                self.call[key] = value.clone();
            }
        }
        // Keep diffs for the Edit preview; plain output content is progress.
        let has_diff = update["content"]
            .as_array()
            .is_some_and(|items| items.iter().any(|c| c["type"].as_str() == Some("diff")));
        if has_diff {
            self.call["content"] = update["content"].clone();
        }
    }

    fn ready(&self) -> bool {
        let call = &self.call;
        let started = matches!(
            call["status"].as_str(),
            Some("in_progress" | "completed" | "failed")
        );
        // Edits are worth waiting for: the diff makes the preview.
        if matches!(call["kind"].as_str(), Some("edit" | "delete" | "move")) {
            return started || call["content"].is_array();
        }
        call["rawInput"].as_object().is_some_and(|o| !o.is_empty())
            || call["locations"].as_array().is_some_and(|l| !l.is_empty())
            || call["content"].is_array()
            || matches!(
                call["status"].as_str(),
                Some("in_progress" | "completed" | "failed")
            )
    }

    fn emit(&mut self, id: &str, events: &mut Vec<ProviderEvent>) {
        if self.emitted {
            return;
        }
        self.emitted = true;
        let (name, input) = tool_use(&self.call);
        self.name = name.clone();
        self.input = input.clone();
        events.push(ProviderEvent::ToolUse {
            tool_use_id: id.to_string(),
            name,
            input,
            parent_tool_use_id: None,
        });
    }
}

type Writer = Arc<Mutex<Option<Arc<JsonlProcess>>>>;

pub struct AcpSession {
    process: Arc<JsonlProcess>,
    state: Arc<Mutex<State>>,
}

impl AcpSession {
    pub fn start(
        kind: ProviderKind,
        config: SessionConfig,
    ) -> Result<(Self, async_channel::Receiver<ProviderEvent>)> {
        let known = agent(kind);
        let executable = config
            .executable
            .clone()
            .or_else(|| find_executable(kind))
            .ok_or_else(|| match known {
                Some(agent) => anyhow!(
                    "{} (`{}`) was not found on PATH — install with `{}`",
                    kind.label(),
                    agent.binary,
                    agent.install_hint
                ),
                None => anyhow!("Set the agent command in Settings → Providers"),
            })?;
        let args: Vec<String> = if config.args.is_empty() {
            known
                .map(|a| a.args.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default()
        } else {
            config.args.clone()
        };
        let state = Arc::new(Mutex::new(State {
            next_id: 1,
            pending: HashMap::new(),
            session_id: None,
            queued: Vec::new(),
            permissions: HashMap::new(),
            modes: Vec::new(),
            permission_mode: config.permission_mode,
            model: config.model.clone().filter(|m| !m.is_empty()),
            cwd: config.cwd.clone(),
            resume: config.resume_session_id.clone(),
            load_session: false,
            loading: false,
            text: String::new(),
            thinking: String::new(),
            tools: HashMap::new(),
            plan: None,
            mcp_servers: config.mcp_servers.clone(),
            mcp_http: false,
        }));
        let writer: Writer = Arc::new(Mutex::new(None));
        let (reader_state, reader_writer) = (state.clone(), writer.clone());
        let envs: Vec<(&str, &str)> = config
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
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
        session.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": { "readTextFile": true, "writeTextFile": true },
                    "terminal": false
                },
                "clientInfo": { "name": "elyra-workspace", "version": env!("CARGO_PKG_VERSION") }
            }),
            Pending::Initialize,
        )?;
        Ok((session, rx))
    }

    fn request(&self, method: &str, params: Value, pending: Pending) -> Result<()> {
        let frame = {
            let mut state = self.state.lock().unwrap();
            request_frame(&mut state, method, params, pending)
        };
        self.process.write(&frame)
    }

    fn session_id(&self) -> Option<String> {
        self.state.lock().unwrap().session_id.clone()
    }
}

fn request_frame(state: &mut State, method: &str, params: Value, pending: Pending) -> Value {
    let id = state.next_id;
    state.next_id += 1;
    state.pending.insert(id, pending);
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn prompt_blocks(prompt: &Prompt) -> Value {
    let mut blocks = vec![json!({ "type": "text", "text": prompt.text })];
    for image in &prompt.images {
        blocks.push(json!({ "type": "image", "data": image.data, "mimeType": image.media_type }));
    }
    Value::Array(blocks)
}

/// The agent's mode id that best matches a permission mode.
fn mode_for(modes: &[String], mode: PermissionMode) -> Option<String> {
    let candidates: &[&str] = match mode {
        PermissionMode::Ask => &["default", "ask", "normal", "build"],
        PermissionMode::AcceptEdits => &[
            "acceptEdits",
            "accept_edits",
            "auto_edit",
            "autoEdit",
            "edit",
        ],
        PermissionMode::Plan => &["plan"],
        PermissionMode::FullAccess => &["bypassPermissions", "yolo", "full", "auto", "full-access"],
    };
    candidates.iter().find_map(|candidate| {
        modes
            .iter()
            .find(|m| m.eq_ignore_ascii_case(candidate))
            .cloned()
    })
}

impl AgentSession for AcpSession {
    fn send(&self, prompt: &Prompt) -> Result<()> {
        let frame = {
            let mut state = self.state.lock().unwrap();
            let Some(session_id) = state.session_id.clone() else {
                state.queued.push(prompt_blocks(prompt));
                return Ok(());
            };
            request_frame(
                &mut state,
                "session/prompt",
                json!({ "sessionId": session_id, "prompt": prompt_blocks(prompt) }),
                Pending::Prompt(Instant::now()),
            )
        };
        self.process.write(&frame)
    }

    fn steer(&self, prompt: &Prompt) -> Result<()> {
        self.send(prompt)
    }

    fn respond_permission(&self, request_id: &str, response: PermissionResponse) -> Result<()> {
        let Some((id, options)) = self.state.lock().unwrap().permissions.remove(request_id) else {
            return Ok(());
        };
        let wanted: &[&str] = match response {
            PermissionResponse::Allow => &["allow_once", "allow_always"],
            PermissionResponse::AllowForSession => &["allow_always", "allow_once"],
            PermissionResponse::Deny { .. } => &["reject_once", "reject_always"],
        };
        let option = wanted.iter().find_map(|kind| {
            options
                .iter()
                .find(|o| o["kind"].as_str() == Some(kind))
                .and_then(|o| o["optionId"].as_str())
        });
        let outcome = match option {
            Some(option) => json!({ "outcome": "selected", "optionId": option }),
            None => json!({ "outcome": "cancelled" }),
        };
        self.process.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "outcome": outcome }
        }))
    }

    fn answer_question(&self, _request_id: &str, _answer: QuestionAnswer) -> Result<()> {
        Ok(())
    }

    fn interrupt(&self) -> Result<()> {
        let pending: Vec<Value> = {
            let mut state = self.state.lock().unwrap();
            state.permissions.drain().map(|(_, (id, _))| id).collect()
        };
        // Outstanding permission requests must be answered as cancelled.
        for id in pending {
            self.process.write(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "outcome": { "outcome": "cancelled" } }
            }))?;
        }
        if let Some(session_id) = self.session_id() {
            self.process.write(&json!({
                "jsonrpc": "2.0",
                "method": "session/cancel",
                "params": { "sessionId": session_id }
            }))?;
        }
        Ok(())
    }

    fn set_permission_mode(&self, mode: PermissionMode) -> Result<()> {
        let (session_id, mode_id) = {
            let mut state = self.state.lock().unwrap();
            state.permission_mode = mode;
            (state.session_id.clone(), mode_for(&state.modes, mode))
        };
        if let (Some(session_id), Some(mode_id)) = (session_id, mode_id) {
            self.request(
                "session/set_mode",
                json!({ "sessionId": session_id, "modeId": mode_id }),
                Pending::Ignore,
            )?;
        }
        Ok(())
    }

    fn set_model(&self, model: Option<&str>) -> Result<()> {
        let Some(model) = model.filter(|m| !m.is_empty()) else {
            return Ok(());
        };
        self.state.lock().unwrap().model = Some(model.to_string());
        if let Some(session_id) = self.session_id() {
            self.request(
                "session/set_model",
                json!({ "sessionId": session_id, "modelId": model }),
                Pending::Ignore,
            )?;
        }
        Ok(())
    }

    fn set_effort(&self, _effort: Option<&str>) -> Result<bool> {
        Ok(true)
    }

    fn compact(&self) -> Result<()> {
        self.send(&Prompt::text("/compact"))
    }

    fn shutdown(&self) {
        self.process.shutdown();
    }
}

impl Drop for AcpSession {
    fn drop(&mut self) {
        self.process.shutdown();
    }
}

// ---- reader thread ------------------------------------------------------

fn write(writer: &Writer, frame: &Value) {
    if let Some(process) = writer.lock().unwrap().as_ref()
        && let Err(err) = process.write(frame)
    {
        log::warn!("acp write: {err:#}");
    }
}

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
            // A response to one of our requests.
            (None, Some(id)) => {
                if let Some(pending) = id.as_u64().and_then(|id| state.pending.remove(&id)) {
                    handle_response(&mut state, pending, message, &mut events, &mut outgoing);
                }
            }
            (Some("session/update"), None) => {
                handle_update(&mut state, &message["params"]["update"], &mut events)
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
    for frame in outgoing {
        write(writer, &frame);
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
                    error: Some(format!("The agent refused to start: {error}")),
                });
                return;
            }
            state.load_session = result["agentCapabilities"]["loadSession"]
                .as_bool()
                .unwrap_or(false);
            state.mcp_http = result["agentCapabilities"]["mcpCapabilities"]["http"]
                .as_bool()
                .unwrap_or(false);
            let cwd = state.cwd.display().to_string();
            let frame = match state.resume.clone() {
                Some(session_id) if state.load_session => {
                    state.loading = true;
                    request_frame(
                        state,
                        "session/load",
                        json!({ "sessionId": session_id, "cwd": cwd, "mcpServers": state.mcp_servers() }),
                        Pending::LoadSession(session_id),
                    )
                }
                _ => request_frame(
                    state,
                    "session/new",
                    json!({ "cwd": cwd, "mcpServers": state.mcp_servers() }),
                    Pending::NewSession,
                ),
            };
            outgoing.push(frame);
        }
        Pending::NewSession | Pending::LoadSession(_) => {
            state.loading = false;
            if rpc_error(message).is_some() && matches!(pending, Pending::LoadSession(_)) {
                // The saved session is gone; start fresh.
                state.resume = None;
                let cwd = state.cwd.display().to_string();
                outgoing.push(request_frame(
                    state,
                    "session/new",
                    json!({ "cwd": cwd, "mcpServers": state.mcp_servers() }),
                    Pending::NewSession,
                ));
                return;
            }
            let session_id = match (&pending, result["sessionId"].as_str()) {
                (_, Some(id)) => id.to_string(),
                (Pending::LoadSession(id), None) => id.clone(),
                _ => {
                    let error = rpc_error(message).unwrap_or_else(|| "no session id".into());
                    events.push(ProviderEvent::Exited {
                        error: Some(format!("Could not start an agent session: {error}")),
                    });
                    return;
                }
            };
            state.session_id = Some(session_id.clone());
            state.modes = result["modes"]["availableModes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m["id"].as_str().map(str::to_string))
                .collect();
            let models: Vec<ModelOption> = result["models"]["availableModels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| {
                    let id = m["modelId"].as_str()?.to_string();
                    let label = m["name"].as_str().unwrap_or(&id).to_string();
                    Some(ModelOption { id, label })
                })
                .collect();
            let current_model = result["models"]["currentModelId"]
                .as_str()
                .map(str::to_string);
            events.push(ProviderEvent::SessionStarted {
                session_id: session_id.clone(),
                model: current_model.clone(),
            });
            if !models.is_empty() {
                let mut options = vec![ModelOption {
                    id: String::new(),
                    label: "Default".into(),
                }];
                options.extend(models);
                events.push(ProviderEvent::Models(options));
            }
            if let Some(mode) = mode_for(&state.modes, state.permission_mode) {
                let current = result["modes"]["currentModeId"].as_str();
                if current != Some(mode.as_str()) {
                    outgoing.push(request_frame(
                        state,
                        "session/set_mode",
                        json!({ "sessionId": session_id, "modeId": mode }),
                        Pending::Ignore,
                    ));
                }
            }
            if let Some(model) = state.model.clone()
                && current_model.as_deref() != Some(model.as_str())
            {
                outgoing.push(request_frame(
                    state,
                    "session/set_model",
                    json!({ "sessionId": session_id, "modelId": model }),
                    Pending::Ignore,
                ));
            }
            for prompt in std::mem::take(&mut state.queued) {
                outgoing.push(request_frame(
                    state,
                    "session/prompt",
                    json!({ "sessionId": session_id, "prompt": prompt }),
                    Pending::Prompt(Instant::now()),
                ));
            }
        }
        Pending::Prompt(started) => {
            flush_text(state, events);
            let error = rpc_error(message);
            let stop = result["stopReason"].as_str().unwrap_or("end_turn");
            let message = match (&error, stop) {
                (Some(error), _) => Some(error.clone()),
                (None, "refusal") => Some("The agent refused to continue.".into()),
                (None, "max_tokens") => Some("Stopped: the output token limit was reached.".into()),
                (None, "max_turn_requests") => {
                    Some("Stopped: too many requests in one turn.".into())
                }
                _ => None,
            };
            if let Some(usage) = usage_from(&result["usage"]) {
                events.push(ProviderEvent::Usage(usage));
            }
            events.push(ProviderEvent::TurnCompleted {
                is_error: error.is_some() || stop == "refusal",
                duration_ms: Some(started.elapsed().as_millis() as u64),
                cost_usd: None,
                message,
            });
        }
        Pending::Ignore => {
            if let Some(error) = rpc_error(message) {
                events.push(ProviderEvent::Warning(error));
            }
        }
    }
}

fn usage_from(usage: &Value) -> Option<Usage> {
    let n = |key: &str| usage[key].as_u64();
    let input = n("inputTokens").or_else(|| n("input_tokens"))?;
    Some(Usage {
        input_tokens: input,
        output_tokens: n("outputTokens")
            .or_else(|| n("output_tokens"))
            .unwrap_or(0),
        cache_read_tokens: n("cachedReadTokens").unwrap_or(0),
        cache_write_tokens: n("cachedWriteTokens").unwrap_or(0),
        ..Default::default()
    })
}

/// Turn accumulated message chunks into finished transcript items.
fn flush_text(state: &mut State, events: &mut Vec<ProviderEvent>) {
    let thinking = std::mem::take(&mut state.thinking);
    if !thinking.trim().is_empty() {
        events.push(ProviderEvent::Thinking { text: thinking });
    }
    let text = std::mem::take(&mut state.text);
    if !text.trim().is_empty() {
        events.push(ProviderEvent::AssistantText {
            text,
            parent_tool_use_id: None,
        });
    }
}

fn content_text(content: &Value) -> Option<String> {
    match content["type"].as_str()? {
        "text" => content["text"].as_str().map(str::to_string),
        "resource_link" => content["uri"].as_str().map(str::to_string),
        "resource" => content["resource"]["text"].as_str().map(str::to_string),
        _ => None,
    }
}

/// Text of a tool call's `content` list (text blocks and diffs).
fn tool_content_text(content: &Value) -> String {
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| match item["type"].as_str() {
            Some("content") => content_text(&item["content"]),
            Some("diff") => Some(format!("Edited {}", item["path"].as_str().unwrap_or(""))),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Map an ACP tool call onto the tool names the transcript knows how to
/// render (Bash, Read, Edit, …).
fn tool_use(update: &Value) -> (String, Value) {
    let title = update["title"].as_str().unwrap_or("Tool").to_string();
    let raw = update.get("rawInput").cloned().unwrap_or(Value::Null);
    let path = update["locations"][0]["path"]
        .as_str()
        .or_else(|| raw["path"].as_str())
        .or_else(|| raw["file_path"].as_str())
        .map(str::to_string);
    let diff = update["content"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| c["type"].as_str() == Some("diff"))
        .cloned();
    let mut input = match &raw {
        Value::Object(_) => raw.clone(),
        _ => json!({}),
    };
    input["description"] = Value::String(title.clone());
    let name = match update["kind"].as_str().unwrap_or("other") {
        "execute" => {
            if input["command"].as_str().is_none() {
                input["command"] = Value::String(title.clone());
            }
            "Bash"
        }
        "read" => {
            if let Some(path) = &path {
                input["file_path"] = Value::String(path.clone());
            }
            "Read"
        }
        "edit" | "delete" | "move" => {
            if let Some(path) = diff
                .as_ref()
                .and_then(|d| d["path"].as_str().map(str::to_string))
                .or(path)
            {
                input["file_path"] = Value::String(path);
            }
            if let Some(diff) = &diff {
                input["old_string"] = diff["oldText"].clone();
                input["new_string"] = diff["newText"].clone();
                if diff["oldText"].is_null() {
                    input["content"] = diff["newText"].clone();
                    return ("Write".into(), input);
                }
            }
            "Edit"
        }
        "search" => {
            if input["pattern"].as_str().is_none() {
                input["pattern"] = Value::String(title.clone());
            }
            "Grep"
        }
        "fetch" => {
            if input["url"].as_str().is_none() {
                input["url"] = Value::String(title.clone());
            }
            "WebFetch"
        }
        _ => return (title, input),
    };
    (name.into(), input)
}

fn handle_update(state: &mut State, update: &Value, events: &mut Vec<ProviderEvent>) {
    let kind = update["sessionUpdate"].as_str().unwrap_or("");
    if state.loading && kind != "available_commands_update" {
        return;
    }
    match kind {
        "agent_message_chunk" => {
            if let Some(text) = content_text(&update["content"]) {
                state.text.push_str(&text);
                events.push(ProviderEvent::TextDelta(text));
            }
        }
        "agent_thought_chunk" => {
            if let Some(text) = content_text(&update["content"]) {
                state.thinking.push_str(&text);
                events.push(ProviderEvent::ThinkingDelta(text));
            }
        }
        "tool_call" | "tool_call_update" => {
            let id = update["toolCallId"].as_str().unwrap_or("").to_string();
            if kind == "tool_call" && !state.tools.contains_key(&id) {
                flush_text(state, events);
                state.tools.insert(id, ToolState::default());
            }
            tool_update(state, update, events);
        }
        "plan" => {
            let todos: Vec<Value> = update["entries"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|entry| {
                    let content = entry["content"].as_str().unwrap_or("");
                    json!({
                        "content": content,
                        "activeForm": content,
                        "status": entry["status"].as_str().unwrap_or("pending"),
                    })
                })
                .collect();
            let input = json!({ "todos": todos });
            if state.plan.as_ref() != Some(&input) {
                flush_text(state, events);
                state.plan = Some(input.clone());
                events.push(ProviderEvent::ToolUse {
                    tool_use_id: format!("plan-{}", elyra_core::new_id().simple()),
                    name: "TodoWrite".into(),
                    input,
                    parent_tool_use_id: None,
                });
            }
        }
        "available_commands_update" => {
            let commands = update["availableCommands"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| {
                    Some(SlashCommand {
                        name: c["name"].as_str()?.trim_start_matches('/').to_string(),
                        description: c["description"].as_str().unwrap_or("").to_string(),
                        argument_hint: c["input"]["hint"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect();
            events.push(ProviderEvent::Commands(commands));
        }
        "usage_update" => {
            let used = update["used"].as_u64();
            let size = update["size"].as_u64();
            if used.is_some() || size.is_some() {
                events.push(ProviderEvent::Usage(Usage {
                    context_tokens: used,
                    context_window: size,
                    cost_usd: update["cost"]["amount"].as_f64(),
                    ..Default::default()
                }));
            }
        }
        _ => {}
    }
}

fn tool_update(state: &mut State, update: &Value, events: &mut Vec<ProviderEvent>) {
    let id = update["toolCallId"].as_str().unwrap_or("").to_string();
    if !state.tools.contains_key(&id) {
        return;
    }
    // Text streamed before the tool row belongs above it.
    if state.tools.get(&id).is_some_and(|t| !t.emitted) {
        let mut probe = ToolState::default();
        probe.merge(update);
        if probe.ready() || state.tools[&id].ready() {
            flush_text(state, events);
        }
    }
    let Some(tool) = state.tools.get_mut(&id) else {
        return;
    };
    if tool.done {
        return;
    }
    tool.merge(update);
    if !tool.emitted {
        if !tool.ready() {
            return;
        }
        tool.emit(&id, events);
    }
    let text = tool_content_text(&update["content"]);
    if !text.is_empty() {
        tool.output = text;
        events.push(ProviderEvent::ToolProgress {
            tool_use_id: id.clone(),
            output: tool.output.clone(),
        });
    }
    let status = update["status"].as_str().unwrap_or("");
    if matches!(status, "completed" | "failed") {
        tool.done = true;
        let content = if tool.output.is_empty() {
            match &update["rawOutput"] {
                Value::Null => String::new(),
                Value::String(s) => s.clone(),
                other => other.to_string(),
            }
        } else {
            tool.output.clone()
        };
        events.push(ProviderEvent::ToolResult {
            tool_use_id: id,
            content,
            is_error: status == "failed",
        });
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
    let reply = |result: Value| json!({ "jsonrpc": "2.0", "id": id, "result": result });
    let fail = |code: i64, message: String| json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } });
    match method {
        "session/request_permission" => {
            let options: Vec<Value> = params["options"].as_array().cloned().unwrap_or_default();
            if state.permission_mode == PermissionMode::FullAccess {
                let option = ["allow_always", "allow_once"].iter().find_map(|kind| {
                    options
                        .iter()
                        .find(|o| o["kind"].as_str() == Some(kind))
                        .and_then(|o| o["optionId"].as_str())
                });
                if let Some(option) = option {
                    outgoing.push(reply(
                        json!({ "outcome": { "outcome": "selected", "optionId": option } }),
                    ));
                    return;
                }
            }
            flush_text(state, events);
            let tool = &params["toolCall"];
            let request_id = format!("acp-{}", id.to_string().trim_matches('"'));
            let title = tool["title"].as_str().unwrap_or("Tool").to_string();
            // Prefer the details of the tool call this request is about,
            // showing its row first if it wasn't shown yet.
            let (name, input) = match tool["toolCallId"].as_str() {
                Some(call_id) => {
                    let known = state.tools.entry(call_id.to_string()).or_default();
                    known.merge(tool);
                    known.emit(call_id, events);
                    (known.name.clone(), known.input.clone())
                }
                None => tool_use(tool),
            };
            state
                .permissions
                .insert(request_id.clone(), (id.clone(), options.clone()));
            events.push(ProviderEvent::PermissionRequest(PermissionRequest {
                request_id,
                tool_name: name,
                tool_use_id: tool["toolCallId"].as_str().map(str::to_string),
                description: Some(title),
                input,
                suggestions: Value::Array(options),
            }));
        }
        "fs/read_text_file" => {
            let path = params["path"].as_str().unwrap_or("");
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    let start = params["line"].as_u64().unwrap_or(1).max(1) as usize - 1;
                    let limit = params["limit"].as_u64().map(|l| l as usize);
                    let content = if start == 0 && limit.is_none() {
                        text
                    } else {
                        let lines = text.lines().skip(start);
                        match limit {
                            Some(limit) => lines.take(limit).collect::<Vec<_>>().join("\n"),
                            None => lines.collect::<Vec<_>>().join("\n"),
                        }
                    };
                    outgoing.push(reply(json!({ "content": content })));
                }
                Err(err) => outgoing.push(fail(-32002, format!("{path}: {err}"))),
            }
        }
        "fs/write_text_file" => {
            let path = params["path"].as_str().unwrap_or("");
            let content = params["content"].as_str().unwrap_or("");
            let result = std::path::Path::new(path)
                .parent()
                .map(std::fs::create_dir_all)
                .transpose()
                .and_then(|_| std::fs::write(path, content));
            match result {
                Ok(()) => outgoing.push(reply(Value::Null)),
                Err(err) => outgoing.push(fail(-32002, format!("{path}: {err}"))),
            }
        }
        other => outgoing.push(fail(-32601, format!("method not supported: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const MOCK_AGENT: &str = include_str!("../tests/mock_acp_agent.py");

    fn recv(rx: &async_channel::Receiver<ProviderEvent>) -> ProviderEvent {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(event) = rx.try_recv() {
                return event;
            }
            assert!(Instant::now() < deadline, "timed out waiting for an event");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for<T>(
        rx: &async_channel::Receiver<ProviderEvent>,
        seen: &mut Vec<ProviderEvent>,
        mut pick: impl FnMut(&ProviderEvent) -> Option<T>,
    ) -> T {
        loop {
            let event = recv(rx);
            let picked = pick(&event);
            seen.push(event);
            if let Some(value) = picked {
                return value;
            }
        }
    }

    #[test]
    fn maps_permission_modes_to_agent_modes() {
        let modes: Vec<String> = ["default", "acceptEdits", "plan", "bypassPermissions"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            mode_for(&modes, PermissionMode::Plan).as_deref(),
            Some("plan")
        );
        assert_eq!(
            mode_for(&modes, PermissionMode::FullAccess).as_deref(),
            Some("bypassPermissions")
        );
        assert_eq!(mode_for(&["code".to_string()], PermissionMode::Plan), None);
    }

    #[test]
    fn runs_a_turn_against_a_mock_agent() {
        let dir = std::env::temp_dir().join(format!("elyra-acp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("mock_agent.py");
        std::fs::write(&script, MOCK_AGENT).unwrap();
        std::fs::write(dir.join("notes.txt"), "line one\nline two\n").unwrap();
        let config = SessionConfig {
            cwd: dir.clone(),
            model: Some("mock-large".into()),
            effort: None,
            permission_mode: PermissionMode::Ask,
            resume_session_id: None,
            executable: Some("python3".into()),
            args: vec![script.display().to_string()],
            env: vec![("MOCK_GREETING".into(), "hej".into())],
            fork: false,
            append_system_prompt: None,
            mcp_servers: Vec::new(),
        };
        let (session, rx) = AcpSession::start(ProviderKind::CustomAcp, config).unwrap();
        // Prompts sent before the session exists are queued.
        session.send(&Prompt::text("read notes")).unwrap();
        let mut seen = Vec::new();
        let session_id = wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::SessionStarted { session_id, .. } => Some(session_id.clone()),
            _ => None,
        });
        assert_eq!(session_id, "mock-session-1");
        let request = wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::PermissionRequest(r) => Some(r.clone()),
            _ => None,
        });
        assert_eq!(request.tool_name, "Read");
        assert!(
            request.input["file_path"]
                .as_str()
                .unwrap()
                .ends_with("notes.txt")
        );
        session
            .respond_permission(&request.request_id, PermissionResponse::Allow)
            .unwrap();
        let completed = wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::TurnCompleted { is_error, .. } => Some(*is_error),
            _ => None,
        });
        assert!(!completed);

        let has = |pred: &dyn Fn(&ProviderEvent) -> bool| seen.iter().any(pred);
        assert!(has(
            &|e| matches!(e, ProviderEvent::Models(m) if m.iter().any(|m| m.id == "mock-large"))
        ));
        assert!(has(
            &|e| matches!(e, ProviderEvent::Commands(c) if c.iter().any(|c| c.name == "review"))
        ));
        assert!(has(
            &|e| matches!(e, ProviderEvent::Thinking { text } if text == "Planning…")
        ));
        assert!(has(
            &|e| matches!(e, ProviderEvent::ToolUse { name, .. } if name == "TodoWrite")
        ));
        // The agent read the file through fs/read_text_file (line 2 only)
        // and the env var reached it.
        assert!(has(&|e| matches!(e,
            ProviderEvent::AssistantText { text, .. } if text == "hej: line two | mode=default model=mock-large")));
        assert!(has(&|e| matches!(e,
            ProviderEvent::ToolResult { content, is_error: false, .. } if content == "read 1 line")));

        // A denied tool call fails the tool; cancel ends the turn.
        session.send(&Prompt::text("write file")).unwrap();
        let request = wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::PermissionRequest(r) => Some(r.clone()),
            _ => None,
        });
        assert_eq!(request.tool_name, "Write");
        session.interrupt().unwrap();
        let message = wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::TurnCompleted { message, .. } => Some(message.clone()),
            _ => None,
        });
        assert_eq!(message, None, "cancelled turns end cleanly");
        assert!(!dir.join("out.txt").exists());

        // Writes go through fs/write_text_file when allowed.
        session
            .set_permission_mode(PermissionMode::FullAccess)
            .unwrap();
        session.send(&Prompt::text("write file")).unwrap();
        wait_for(&rx, &mut seen, |e| match e {
            ProviderEvent::TurnCompleted { .. } => Some(()),
            _ => None,
        });
        assert_eq!(
            std::fs::read_to_string(dir.join("out.txt")).unwrap(),
            "written by agent"
        );
        session.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
