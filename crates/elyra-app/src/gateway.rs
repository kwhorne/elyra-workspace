//! The agent gateway: an MCP server (see `elyra-mcp`) that lets agents in
//! Elyra threads — and paired external clients such as Claude Desktop —
//! list, read, create and steer threads. Every call is written to the audit
//! log.
//!
//! The MCP server runs calls on its own threads; they are forwarded over a
//! channel to the GPUI foreground, which owns all app state.

use crate::app_state::{AppState, transcript_text};
use elyra_core::{
    AuditEntry, ClientScope, ItemContent, McpClient, ProviderKind, ThreadId, ThreadStatus,
};
use elyra_mcp::{Handler, Output, Principal, Server, Tool};
use elyra_provider::{McpServer, Prompt};
use gpui_kit::{App, AsyncApp, Entity, Global};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// How long `wait_for_thread` may block.
const MAX_WAIT: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, PartialEq)]
enum Caller {
    /// An agent working in this thread.
    Thread(ThreadId),
    /// A paired external client.
    Client { id: uuid::Uuid, scope: ClientScope },
}

impl Caller {
    fn principal(&self) -> Principal {
        match self {
            Caller::Thread(id) => format!("thread:{id}"),
            Caller::Client { id, scope } => format!(
                "client:{id}:{}",
                if *scope == ClientScope::Full {
                    "full"
                } else {
                    "read"
                }
            ),
        }
    }

    fn parse(principal: &str) -> Option<Self> {
        if let Some(id) = principal.strip_prefix("thread:") {
            return Some(Caller::Thread(id.parse().ok()?));
        }
        let rest = principal.strip_prefix("client:")?;
        let (id, scope) = rest.rsplit_once(':')?;
        Some(Caller::Client {
            id: id.parse().ok()?,
            scope: if scope == "full" {
                ClientScope::Full
            } else {
                ClientScope::ReadOnly
            },
        })
    }

    fn can_write(&self) -> bool {
        !matches!(
            self,
            Caller::Client {
                scope: ClientScope::ReadOnly,
                ..
            }
        )
    }
}

struct Call {
    caller: Caller,
    tool: String,
    args: Value,
    reply: async_channel::Sender<Result<Output, String>>,
}

struct GatewayHandler {
    tokens: Arc<RwLock<HashMap<String, Principal>>>,
    calls: async_channel::Sender<Call>,
}

const READ_TOOLS: &[&str] = &[
    "symbols",
    "why",
    "error_origin",
    "history",
    "intent_board",
    "changes_by_instruction",
    "conflict_context",
    "restack_preview",
    "who_broke_it",
    "search_threads",
    "list_projects",
    "list_threads",
    "read_thread",
    "wait_for_thread",
];

fn is_read_tool(name: &str) -> bool {
    READ_TOOLS.contains(&name) || crate::browser_tools::READ_TOOLS.contains(&name)
}

fn tools() -> Vec<Tool> {
    let tool = |name: &str, description: &str, schema: Value| Tool {
        name: name.into(),
        description: description.into(),
        input_schema: schema,
    };
    let id = json!({ "type": "string", "description": "Thread id" });
    vec![
        tool(
            "symbols",
            "Look up code symbols in a tree-sitter index of the project: where a function, method, class, interface, type or module is defined, and where it is referenced (calls, implementations, type uses). Rust, TypeScript, TSX, JavaScript, Python, Go, PHP and C. Use it before grep or read when you need a definition or the callers of something: it returns file:line locations with signatures, so you can read only the relevant range. It works in your thread's folder; other callers name a project or thread.",
            json!({ "type": "object", "properties": {
                "name": { "type": "string", "description": "Symbol name (function, method, class, type, module); case-insensitive" },
                "mode": { "type": "string", "enum": ["definitions", "references", "all"], "description": "What to return (default: all)" },
                "exact": { "type": "boolean", "description": "Whole name only (default: true); false also returns names containing it" },
                "path": { "type": "string", "description": "Only files under this directory, relative to the project" },
                "limit": { "type": "integer", "description": "Most rows per section (default: 50)" },
                "project": { "type": "string", "description": "Project id, name or path (for callers outside a thread)" },
                "thread_id": id
            }, "required": ["name"] }),
        ),
        tool(
            "why",
            "Why is this line here? Traces a line back to the thread and the message that had an agent write it (any agent in Elyra Workspace, also when it committed itself), with when and what the agent said afterwards, plus git blame. Use it before changing code you don't understand, instead of guessing at the intent.",
            json!({ "type": "object", "properties": {
                "path": { "type": "string", "description": "File, relative to your folder" },
                "line": { "type": "integer", "description": "1-based line number" },
                "project": { "type": "string", "description": "Project id, name or path (for callers outside a thread)" },
                "thread_id": id
            }, "required": ["path", "line"] }),
        ),
        tool(
            "error_origin",
            "Where the Laravel app's latest errors come from: the errors in storage/logs (newest last), each with the line in the app's own code it failed on and the instruction (thread, agent, message) that had an agent write that line. Use it when something fails at runtime, before guessing at a fix; then ask_author on that line if the reason isn't clear.",
            json!({ "type": "object", "properties": {
                "limit": { "type": "integer", "description": "How many of the latest errors (default 3, at most 10)" },
                "project": { "type": "string", "description": "Project id, name or path (for callers outside a thread)" },
                "thread_id": id
            } }),
        ),
        tool(
            "ask_author",
            "Ask the agent that wrote a line why, before you change code whose intent you don't know. Elyra starts a read-only chat with that agent (same provider and model) picked up where the turn that wrote the line ended: it has that conversation, what the turn changed in the file and the code around the line now, and answers from what it knew and decided then. Waits for the answer and returns it. Use `why` first for the quick facts; use this when the reason isn't in them.",
            json!({ "type": "object", "properties": {
                "path": { "type": "string", "description": "File, relative to your folder" },
                "line": { "type": "integer", "description": "1-based line number" },
                "question": { "type": "string", "description": "What you want to know (default: why the line is the way it is)" },
                "timeout_seconds": { "type": "integer", "description": "How long to wait for the answer (default 300, at most 3600)" },
                "project": { "type": "string", "description": "Project id, name or path (for callers outside a thread)" }
            }, "required": ["path", "line"] }),
        ),
        tool(
            "history",
            "Which earlier turns wrote a file or a symbol (function, class, method): thread, message, agent and date, newest first, plus the file's git log. Use it to learn what was tried before and why.",
            json!({ "type": "object", "properties": {
                "path": { "type": "string", "description": "File, relative to your folder" },
                "symbol": { "type": "string", "description": "A function, method, class or type name instead of a path" },
                "project": { "type": "string", "description": "Project id, name or path (for callers outside a thread)" },
                "thread_id": id
            } }),
        ),
        tool(
            "intent_board",
            "What other agents in Elyra Workspace changed in this repository during the last hour: thread, agent, whether they work in your folder or on another branch, file and the functions or classes they changed. Check it before changing shared code, so parallel agents stay out of each other's way. Your own thread, its Best of N siblings and side chats are left out.",
            json!({ "type": "object", "properties": {
                "project": { "type": "string", "description": "Project id, name or path (for callers outside a thread)" },
                "thread_id": id
            } }),
        ),
        tool(
            "changes_by_instruction",
            "What the current branch does, by the instructions behind it: the lines it adds compared with main (commits and uncommitted changes), grouped by the user message that had an agent write them — any agent in Elyra Workspace, across threads — with the thread, agent, date, whether the checks passed afterwards and the files. Lines no agent wrote form their own group. Use it to write a pull request description that says what was asked for, or to see what a branch contains before reviewing it.",
            json!({ "type": "object", "properties": {
                "project": { "type": "string", "description": "Project id, name or path (for callers outside a thread)" },
                "thread_id": id
            } }),
        ),
        tool(
            "conflict_context",
            "When a merge, rebase or pull left conflicts: each conflict in the folder with both sides, and for each side the instruction (user message, thread, agent) that had an agent in Elyra Workspace write those lines. Call it before resolving, so the result keeps what both sides were asked to do instead of picking one block of text.",
            json!({ "type": "object", "properties": {
                "project": { "type": "string", "description": "Project id, name or path (for callers outside a thread)" },
                "thread_id": id
            } }),
        ),
        tool(
            "restack_preview",
            "How the current branch would read as one commit per instruction: the commits it would get on top of main (each instruction's message as the title, in the order they were asked, and the files it changes), which commits it would replace, and whether those are pushed. Read-only; the user restacks from the PR tab.",
            json!({ "type": "object", "properties": {
                "project": { "type": "string", "description": "Project id, name or path (for callers outside a thread)" },
                "thread_id": id
            } }),
        ),
        tool(
            "undo_instruction",
            "Take back one earlier instruction in a thread: what its turn (and the automatic fixes after it) changed in the folder is undone, and what came after it stays. Files changed again since in ways that don't merge are left as they are and listed, for you to finish by hand. `instruction` counts the user's messages in the thread from 1 (the last one by default).",
            json!({ "type": "object", "properties": {
                "thread_id": id,
                "instruction": { "type": "integer", "description": "Which message of the thread, from 1 (default: the last)" }
            } }),
        ),
        tool(
            "who_broke_it",
            "When the project's checks fail: find the earlier instruction whose turn made them fail. Runs the check command against the state of the folder before each earlier turn (any thread or agent working in the folder) in a temporary checkout — the working tree is not touched — halving the range until the turn where they went from passing to failing, and names that message, its thread and the failing output. Takes a while: each step runs the checks. Use it instead of guessing when something broke earlier than your own changes.",
            json!({ "type": "object", "properties": {
                "command": { "type": "string", "description": "The command that fails (default: the project's check command)" },
                "project": { "type": "string", "description": "Project id, name or path (for callers outside a thread)" },
                "thread_id": id
            } }),
        ),
        tool(
            "search_threads",
            "Full-text search over every thread in Elyra Workspace: what was asked, what the agents replied, their tool calls and the start of tool output, best match first, grouped by thread with the thread id (read more with read_thread). Use it to find how something was solved or decided before, or an earlier error and its fix. Words match as prefixes and must all occur; put words in double quotes for an exact phrase.",
            json!({ "type": "object", "properties": {
                "query": { "type": "string", "description": "Words to find, or \"an exact phrase\"" },
                "project": { "type": "string", "description": "Only this project (id, name or path)" },
                "messages_only": { "type": "boolean", "description": "Leave tool calls and output out (default: false)" },
                "archived": { "type": "boolean", "description": "Include archived threads (default: false)" },
                "limit": { "type": "integer", "description": "Most matches (default: 20, at most 100)" }
            }, "required": ["query"] }),
        ),
        tool(
            "list_projects",
            "List the projects (folders) in Elyra Workspace.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "list_threads",
            "List threads with their status (idle, running, needs_approval, failed, interrupted).",
            json!({ "type": "object", "properties": {
                "project": { "type": "string", "description": "Project id, name or path to filter by" },
                "include_archived": { "type": "boolean" }
            }}),
        ),
        tool(
            "read_thread",
            "Read a thread's conversation (most recent part).",
            json!({ "type": "object", "properties": {
                "thread_id": id,
                "max_chars": { "type": "integer", "description": "Default 12000" }
            }, "required": ["thread_id"] }),
        ),
        tool(
            "create_thread",
            "Start a new agent thread in a project and send it a first message. Returns the thread id. Use wait_for_thread to get the result.",
            json!({ "type": "object", "properties": {
                "project": { "type": "string", "description": "Project id, name or path" },
                "prompt": { "type": "string" },
                "title": { "type": "string" },
                "provider": { "type": "string", "enum": ProviderKind::ALL.map(|k| k.as_str()) },
                "model": { "type": "string" },
                "worktree": { "type": "boolean", "description": "Work in a new Git worktree" }
            }, "required": ["project", "prompt"] }),
        ),
        tool(
            "send_message",
            "Send a message to a thread (queued if it is busy).",
            json!({ "type": "object", "properties": { "thread_id": id, "text": { "type": "string" } },
                    "required": ["thread_id", "text"] }),
        ),
        tool(
            "wait_for_thread",
            "Wait until a thread finishes its turn or needs input, then return its status and last reply.",
            json!({ "type": "object", "properties": {
                "thread_id": id,
                "timeout_seconds": { "type": "integer", "description": "Default 600, max 3600" }
            }, "required": ["thread_id"] }),
        ),
        tool(
            "interrupt_thread",
            "Stop a thread's running turn.",
            json!({ "type": "object", "properties": { "thread_id": id }, "required": ["thread_id"] }),
        ),
        tool(
            "set_thread_title",
            "Rename a thread.",
            json!({ "type": "object", "properties": { "thread_id": id, "title": { "type": "string" } },
                    "required": ["thread_id", "title"] }),
        ),
        tool(
            "propose_automation",
            "Propose an automation (a prompt an agent runs on a schedule, e.g. nightly dependency checks) to the user. It appears as a card in this thread with Create, Edit and Dismiss; nothing is scheduled unless the user accepts. Only agents working in a thread can propose.",
            json!({ "type": "object", "properties": {
                "name": { "type": "string" },
                "prompt": { "type": "string", "description": "What the agent does on each run" },
                "schedule": {
                    "type": "object",
                    "description": "One of: {\"kind\":\"daily\",\"time\":\"02:00\"}, {\"kind\":\"weekdays\",\"time\":\"08:30\"}, {\"kind\":\"weekly\",\"weekday\":0,\"time\":\"09:00\"} (0 = Monday), {\"kind\":\"interval\",\"minutes\":60}, {\"kind\":\"cron\",\"expr\":\"0 3 * * 1-5\"}, {\"kind\":\"once\",\"at\":\"2026-01-31T09:00:00Z\"}. Times are local unless \"tz\" (IANA) is given."
                },
                "project": { "type": "string", "description": "Project id, name or path; default: this thread's" },
                "provider": { "type": "string", "enum": ProviderKind::ALL.map(|k| k.as_str()) },
                "model": { "type": "string" }
            }, "required": ["name", "prompt", "schedule"] }),
        ),
        tool(
            "propose_rule",
            "When the user corrects how you work in this project (a convention, a command to use, something to avoid) and it will matter in later tasks too, propose it as a rule: one short imperative sentence. It appears as a card in this thread; the user adds it to the project's rules (AGENTS.md, or Laravel Boost's .ai/rules for the files it applies to) or the project's instructions, or dismisses it. Use this instead of Boost's record-rule tool: the user decides. Only agents working in a thread can propose.",
            json!({ "type": "object", "properties": {
                "rule": { "type": "string", "description": "e.g. Validate requests with Form Requests, not in controllers." },
                "reason": { "type": "string", "description": "What the user said that taught it" },
                "paths": { "type": "string", "description": "The files it applies to, as a glob relative to the project (e.g. app/Http/Controllers/**); leave out for the whole project" }
            }, "required": ["rule"] }),
        ),
        tool(
            "archive_thread",
            "Archive a finished thread.",
            json!({ "type": "object", "properties": { "thread_id": id }, "required": ["thread_id"] }),
        ),
    ]
}

impl Handler for GatewayHandler {
    fn authorize(&self, token: &str) -> Option<Principal> {
        if token.is_empty() {
            return None;
        }
        self.tokens.read().ok()?.get(token).cloned()
    }

    fn tools(&self, principal: &Principal) -> Vec<Tool> {
        let writable = Caller::parse(principal).is_some_and(|c| c.can_write());
        tools()
            .into_iter()
            .chain(crate::browser_tools::tools())
            .filter(|tool| writable || is_read_tool(&tool.name))
            .collect()
    }

    fn call(&self, principal: &Principal, name: &str, args: Value) -> Result<Output, String> {
        let caller = Caller::parse(principal).ok_or("unknown caller")?;
        let (reply, result) = async_channel::bounded(1);
        self.calls
            .send_blocking(Call {
                caller,
                tool: name.to_string(),
                args,
                reply,
            })
            .map_err(|_| "Elyra Workspace is shutting down".to_string())?;
        result
            .recv_blocking()
            .map_err(|_| "the call was dropped".to_string())?
    }
}

pub struct Gateway {
    server: Option<Server>,
    tokens: Arc<RwLock<HashMap<String, Principal>>>,
}

impl Global for Gateway {}

/// Start the MCP server and the foreground loop that serves its calls.
pub fn init(app: Entity<AppState>, cx: &mut App) {
    let tokens: Arc<RwLock<HashMap<String, Principal>>> = Arc::default();
    let (calls_tx, calls_rx) = async_channel::unbounded::<Call>();
    let port: u16 = app
        .read(cx)
        .store
        .setting("mcp_port")
        .ok()
        .flatten()
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    let handler = Arc::new(GatewayHandler {
        tokens: tokens.clone(),
        calls: calls_tx,
    });
    let server = match Server::start(port, handler) {
        Ok(server) => {
            // Keep the port stable so pasted client configs keep working.
            app.update(cx, |app, _| {
                app.set_setting("mcp_port", &server.port().to_string())
            });
            Some(server)
        }
        Err(err) => {
            log::error!("agent gateway: {err:#}");
            None
        }
    };
    cx.set_global(Gateway { server, tokens });
    refresh_clients(&app, cx);
    cx.spawn(async move |cx| {
        while let Ok(call) = calls_rx.recv().await {
            let app = app.clone();
            cx.spawn(async move |cx| serve_call(app, call, cx).await)
                .detach();
        }
    })
    .detach();
}

/// Re-read paired external clients into the token table.
pub fn refresh_clients(app: &Entity<AppState>, cx: &mut App) {
    let clients = app.read(cx).store.mcp_clients().unwrap_or_default();
    let Some(gateway) = cx.try_global::<Gateway>() else {
        return;
    };
    if let Ok(mut tokens) = gateway.tokens.write() {
        tokens.retain(|_, principal| !principal.starts_with("client:"));
        for client in clients {
            let caller = Caller::Client {
                id: client.id,
                scope: client.scope,
            };
            tokens.insert(client.token, caller.principal());
        }
    }
}

pub fn url(cx: &App) -> Option<String> {
    cx.try_global::<Gateway>()?.server.as_ref().map(|s| s.url())
}

/// Pair a new external client and return it.
pub fn pair_client(app: &Entity<AppState>, scope: ClientScope, cx: &mut App) -> Option<McpClient> {
    let count = app
        .read(cx)
        .store
        .mcp_clients()
        .map(|c| c.len())
        .unwrap_or(0);
    let client = McpClient {
        id: elyra_core::new_id(),
        name: format!("Client {}", count + 1),
        token: format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
        scope,
        created_at: chrono::Utc::now(),
        last_used_at: None,
    };
    app.read(cx).store.save_mcp_client(&client).ok()?;
    refresh_clients(app, cx);
    Some(client)
}

pub fn revoke_client(app: &Entity<AppState>, id: uuid::Uuid, cx: &mut App) {
    let _ = app.read(cx).store.delete_mcp_client(id);
    refresh_clients(app, cx);
}

/// Ready-to-paste configuration for a paired client.
pub fn client_config(kind: &str, url: &str, token: &str) -> String {
    let (exe, args) = bridge_command(url, token);
    let exe = exe.display().to_string();
    match kind {
        "codex" => format!("[mcp_servers.elyra-workspace]\ncommand = {exe:?}\nargs = {args:?}\n"),
        "claude-code" => format!(
            "claude mcp add --transport http elyra-workspace {url} --header \"Authorization: Bearer {token}\""
        ),
        _ => serde_json::to_string_pretty(&json!({
            "mcpServers": { "elyra-workspace": { "command": exe, "args": args } }
        }))
        .unwrap_or_default(),
    }
}

/// The command external clients run to reach the gateway over stdio.
pub fn bridge_command(url: &str, token: &str) -> (std::path::PathBuf, Vec<String>) {
    let exe = std::env::current_exe().unwrap_or_else(|_| "elyra".into());
    (exe, vec!["mcp-bridge".into(), url.into(), token.into()])
}

/// The MCP server entry for an agent running in `thread`, when the gateway
/// is switched on.
pub fn server_for_thread(thread: ThreadId, cx: &App) -> Option<McpServer> {
    if !crate::preferences::Preferences::global(cx).agent_gateway {
        return None;
    }
    let gateway = cx.try_global::<Gateway>()?;
    let url = gateway.server.as_ref()?.url();
    let token = uuid::Uuid::new_v4().simple().to_string();
    gateway
        .tokens
        .write()
        .ok()?
        .insert(token.clone(), Caller::Thread(thread).principal());
    Some(McpServer {
        name: "elyra".into(),
        bridge: Some(bridge_command(&url, &token)),
        url,
        token,
        ..McpServer::default()
    })
}

async fn serve_call(app: Entity<AppState>, call: Call, cx: &mut AsyncApp) {
    let started = Instant::now();
    let result = if !call.caller.can_write() && !is_read_tool(&call.tool) {
        Err("This client has read-only access.".to_string())
    } else if let Err(denied) = allowed_to_change(&app, &call, cx).await {
        Err(denied)
    } else if call.tool == "symbols" {
        symbols(&app, &call, cx).await.map(Output::Text)
    } else if call.tool == "why" {
        why(&app, &call, cx).await.map(Output::Text)
    } else if call.tool == "error_origin" {
        error_origin(&app, &call, cx).await.map(Output::Text)
    } else if call.tool == "ask_author" {
        ask_author(&app, &call, cx).await.map(Output::Text)
    } else if call.tool == "history" {
        history(&app, &call, cx).await.map(Output::Text)
    } else if call.tool == "conflict_context" {
        conflict_context(&app, &call, cx).await.map(Output::Text)
    } else if call.tool == "restack_preview" {
        restack_preview(&app, &call, cx).await.map(Output::Text)
    } else if call.tool == "undo_instruction" {
        undo_instruction(&app, &call, cx).await.map(Output::Text)
    } else if call.tool == "who_broke_it" {
        who_broke_it(&app, &call, cx).await.map(Output::Text)
    } else if call.tool == "changes_by_instruction" {
        changes_by_instruction(&app, &call, cx)
            .await
            .map(Output::Text)
    } else if call.tool == "search_threads" {
        search_threads(&app, &call, cx).await.map(Output::Text)
    } else if call.tool == "intent_board" {
        intent_board(&app, &call, cx).await.map(Output::Text)
    } else if call.tool == "wait_for_thread" {
        wait_for_thread(&app, &call, cx).await.map(Output::Text)
    } else if crate::browser_tools::is_browser_tool(&call.tool) {
        let thread = match call.caller {
            Caller::Thread(id) => Some(id),
            Caller::Client { .. } => None,
        };
        crate::browser_tools::run(&app, thread, &call.tool, &call.args, cx).await
    } else {
        cx.update(|cx| run_tool(&app, &call, cx)).map(Output::Text)
    };
    let detail = summarize(&call.args);
    cx.update(|cx| {
        let client = caller_label(&app, &call.caller, cx);
        let entry = AuditEntry {
            at: chrono::Utc::now(),
            client,
            tool: call.tool.clone(),
            detail: format!("{detail} ({} ms)", started.elapsed().as_millis()),
            ok: result.is_ok(),
        };
        let state = app.read(cx);
        if let Err(err) = state.store.audit(&entry) {
            log::warn!("audit: {err:#}");
        }
        if let Caller::Client { id, .. } = call.caller
            && let Ok(clients) = state.store.mcp_clients()
            && let Some(mut client) = clients.into_iter().find(|c| c.id == id)
        {
            client.last_used_at = Some(chrono::Utc::now());
            let _ = state.store.save_mcp_client(&client);
        }
    });
    let _ = call.reply.send(result).await;
}

/// An agent changing another thread needs the user's say-so once per pair.
async fn allowed_to_change(
    app: &Entity<AppState>,
    call: &Call,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    use crate::thread_access::{self, Answer};
    let Caller::Thread(from) = call.caller else {
        return Ok(());
    };
    if !thread_access::WRITES.contains(&call.tool.as_str()) {
        return Ok(());
    }
    // An unknown or own thread is reported by the tool itself.
    let Ok(to) = cx.update(|cx| thread_arg(&call.args, app, cx)) else {
        return Ok(());
    };
    if to == from || cx.update(|cx| thread_access::is_approved(app.read(cx), from, to)) {
        return Ok(());
    }
    let (tx, rx) = async_channel::bounded(1);
    let asked = cx.update(|cx| thread_access::ask(app, from, to, &call.tool, &call.args, tx, cx));
    if !asked {
        return Err("Elyra Workspace has no window to ask the user whether this agent may change that thread.".into());
    }
    match rx.recv().await {
        Ok(Answer::Always) => {
            cx.update(|cx| thread_access::remember(app.read(cx), from, to));
            Ok(())
        }
        Ok(Answer::Once) => Ok(()),
        _ => Err("The user did not allow this agent to change that thread.".into()),
    }
}

fn caller_label(app: &Entity<AppState>, caller: &Caller, cx: &App) -> String {
    let state = app.read(cx);
    match caller {
        Caller::Thread(id) => format!(
            "agent: {}",
            state
                .thread(*id)
                .map(|t| t.title.as_str())
                .unwrap_or("thread")
        ),
        Caller::Client { id, .. } => state
            .store
            .mcp_clients()
            .ok()
            .and_then(|clients| clients.into_iter().find(|c| c.id == *id))
            .map(|c| c.name)
            .unwrap_or_else(|| "client".into()),
    }
}

fn summarize(args: &Value) -> String {
    let text = args.to_string();
    if text.chars().count() > 160 {
        format!("{}…", text.chars().take(160).collect::<String>())
    } else {
        text
    }
}

/// A schedule as the agent wrote it; wall-clock times default to local time.
fn parse_schedule(value: &Value) -> Result<elyra_core::Schedule, String> {
    let mut value = value.clone();
    let Some(fields) = value.as_object_mut() else {
        return Err(
            "`schedule` must be an object such as {\"kind\":\"daily\",\"time\":\"02:00\"}".into(),
        );
    };
    let kind = fields.get("kind").and_then(Value::as_str).unwrap_or("");
    if matches!(kind, "daily" | "weekdays" | "weekly" | "cron") && !fields.contains_key("tz") {
        fields.insert("tz".into(), json!(elyra_core::local_tz_name()));
    }
    serde_json::from_value(value).map_err(|err| format!("`schedule`: {err}"))
}

fn propose_automation(app: &Entity<AppState>, call: &Call, cx: &mut App) -> Result<String, String> {
    let Caller::Thread(thread_id) = call.caller else {
        return Err("Only an agent working in a thread can propose an automation; the user creates them in the Automations panel.".into());
    };
    let args = &call.args;
    let name = arg(args, "name")?.trim().to_string();
    let prompt = arg(args, "prompt")?.trim().to_string();
    let schedule = parse_schedule(&args["schedule"])?;
    let state = app.read(cx);
    let thread = state.thread(thread_id).cloned().ok_or("no such thread")?;
    let project = match args["project"].as_str().map(str::to_lowercase) {
        Some(query) => state
            .projects
            .iter()
            .find(|p| {
                p.id.to_string() == query
                    || p.name.to_lowercase() == query
                    || p.path.display().to_string().to_lowercase() == query
            })
            .map(|p| p.id)
            .ok_or_else(|| format!("no project matches “{query}” (see list_projects)"))?,
        None => thread.project_id,
    };
    let provider = args["provider"]
        .as_str()
        .and_then(ProviderKind::parse)
        .unwrap_or(thread.provider);
    let mut automation = elyra_core::Automation::new(name, project, provider, prompt, schedule);
    automation.model = match args["model"].as_str() {
        Some(model) => Some(model.to_string()),
        None if provider == thread.provider => thread.model.clone(),
        None => None,
    };
    if automation.next_run_at.is_none() {
        return Err("That schedule never runs (is the time in the past?).".into());
    }
    let summary = format!(
        "Proposed “{}” ({}) to the user as a card in this thread; it is scheduled only if they accept it. Don't create it some other way.",
        automation.name,
        automation.schedule.describe()
    );
    let session = app
        .update(cx, |app, cx| app.session(thread_id, cx))
        .ok_or("could not open the thread")?;
    session.update(cx, |session, cx| session.propose_automation(automation, cx));
    Ok(summary)
}

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args[key]
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("missing `{key}`"))
}

/// One index at a time: lookups are quick, but two first-time builds of the
/// same project would race for its database.
pub(crate) static SYMBOL_LOOKUPS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The folder to look symbols up in: a named thread's or project's, else the
/// calling agent's own thread's.
fn symbols_dir(
    app: &Entity<AppState>,
    call: &Call,
    cx: &App,
) -> Result<std::path::PathBuf, String> {
    let state = app.read(cx);
    let args = &call.args;
    if args["thread_id"].as_str().is_some() {
        let id = thread_arg(args, app, cx)?;
        let thread = state.thread(id).ok_or("no such thread")?;
        let project = state.project(thread.project_id).ok_or("no such project")?;
        return Ok(thread.working_dir(project));
    }
    if let Some(query) = args["project"].as_str().map(str::to_lowercase) {
        return state
            .projects
            .iter()
            .find(|p| {
                p.id.to_string() == query
                    || p.name.to_lowercase() == query
                    || p.path.display().to_string().to_lowercase() == query
            })
            .map(|p| p.path.clone())
            .ok_or_else(|| format!("no project matches “{query}” (see list_projects)"));
    }
    match call.caller {
        Caller::Thread(id) => {
            let thread = state.thread(id).ok_or("no such thread")?;
            let project = state.project(thread.project_id).ok_or("no such project")?;
            Ok(thread.working_dir(project))
        }
        Caller::Client { .. } => {
            Err("Name a `project` (or `thread_id`) to look symbols up in.".into())
        }
    }
}

async fn symbols(app: &Entity<AppState>, call: &Call, cx: &mut AsyncApp) -> Result<String, String> {
    let args = call.args.clone();
    let name = arg(&args, "name")?.trim().to_string();
    let mode = elyra_index::Mode::parse(args["mode"].as_str())?;
    let dir = cx.update(|cx| symbols_dir(app, call, cx))?;
    let db = elyra_index::default_db_path(&elyra_core::paths::data_dir().join("index"), &dir);
    cx.background_executor()
        .spawn(async move {
            let _one_at_a_time = SYMBOL_LOOKUPS.lock().unwrap_or_else(|e| e.into_inner());
            elyra_index::lookup(
                &dir,
                &db,
                &name,
                mode,
                args["exact"].as_bool().unwrap_or(true),
                args["path"].as_str(),
                args["limit"].as_u64().unwrap_or(0) as usize,
            )
            .map_err(|err| format!("symbol index: {err}"))
        })
        .await
}

/// The folder and project a `why` or `history` call is about.
fn provenance_scope(
    app: &Entity<AppState>,
    call: &Call,
    cx: &App,
) -> Result<(std::path::PathBuf, elyra_core::ProjectId), String> {
    let dir = symbols_dir(app, call, cx)?;
    let project = crate::provenance::project_of(app.read(cx), &dir)
        .ok_or("That folder isn't in a project of Elyra Workspace.")?;
    Ok((dir, project))
}

async fn why(app: &Entity<AppState>, call: &Call, cx: &mut AsyncApp) -> Result<String, String> {
    let path = arg(&call.args, "path")?.to_string();
    let line = call.args["line"]
        .as_u64()
        .filter(|l| *l > 0)
        .ok_or("Give the 1-based `line`.")? as u32;
    let (dir, project) = cx.update(|cx| provenance_scope(app, call, cx))?;
    let (relative, text, blame) = cx
        .background_executor()
        .spawn(async move {
            let (root, relative) = crate::provenance::repo_relative(&dir, &path)
                .ok_or_else(|| format!("{path} isn't in a Git repository."))?;
            let content = std::fs::read_to_string(root.join(&relative))
                .map_err(|err| format!("{relative}: {err}"))?;
            let text = content
                .lines()
                .nth(line as usize - 1)
                .ok_or_else(|| format!("{relative} has fewer than {line} lines."))?
                .to_string();
            let blame = elyra_git::blame_line(&root, &relative, line)
                .ok()
                .flatten()
                .map(|c| {
                    let date = chrono::DateTime::from_timestamp(c.time, 0)
                        .map(|t| t.format("%Y-%m-%d").to_string())
                        .unwrap_or_default();
                    format!("{} · {} · {date} · {}", c.sha, c.author, c.summary)
                });
            Ok::<_, String>((relative, text, blame))
        })
        .await?;
    Ok(cx.update(|cx| {
        let state = app.read(cx);
        let hash = elyra_core::line_hash(&text);
        let turns: Vec<_> = hash
            .and_then(|hash| state.store.line_origins(project, &relative, hash).ok())
            .unwrap_or_default()
            .iter()
            .map(|origin| crate::provenance::turn(state, origin))
            .collect();
        let read = hash.is_some_and(|hash| {
            state
                .store
                .unreviewed_lines(project, &relative)
                .is_ok_and(|unread| !unread.contains(&hash))
        });
        crate::provenance::why_text(&format!("{relative}:{line}"), &text, &turns, read, blame)
    }))
}

async fn conflict_context(
    app: &Entity<AppState>,
    call: &Call,
    cx: &mut AsyncApp,
) -> Result<String, String> {
    let (dir, project) = cx.update(|cx| provenance_scope(app, call, cx))?;
    let explained = cx
        .background_executor()
        .spawn(async move {
            let root = elyra_git::repo_root(&dir)?;
            crate::conflicts::explain(&root, project, &elyra_core::paths::database_path())
        })
        .await
        .map_err(|err| format!("{err:#}"))?;
    Ok(explained.unwrap_or_else(|| "There are no merge conflicts in this folder.".into()))
}

async fn restack_preview(
    app: &Entity<AppState>,
    call: &Call,
    cx: &mut AsyncApp,
) -> Result<String, String> {
    let (dir, project) = cx.update(|cx| provenance_scope(app, call, cx))?;
    let conventional =
        cx.update(|cx| crate::preferences::Preferences::global(cx).conventional_titles);
    let plan = cx
        .background_executor()
        .spawn(async move {
            let root = elyra_git::repo_root(&dir)?;
            crate::restack::plan(
                &root,
                project,
                &elyra_core::paths::database_path(),
                conventional,
            )
        })
        .await
        .map_err(|err| format!("Can't restack: {err:#}"))?;
    Ok(crate::restack::describe(&plan))
}

async fn undo_instruction(
    app: &Entity<AppState>,
    call: &Call,
    cx: &mut AsyncApp,
) -> Result<String, String> {
    let id = match call.caller {
        Caller::Thread(own) if call.args["thread_id"].as_str().is_none() => own,
        _ => cx.update(|cx| thread_arg(&call.args, app, cx))?,
    };
    let session = cx
        .update(|cx| app.update(cx, |app, cx| app.session(id, cx)))
        .ok_or("no such thread")?;
    // The user's messages (automatic fixes are part of the one before).
    let (item, before) = cx.update(|cx| {
        let session = session.read(cx);
        let messages: Vec<elyra_core::ItemId> = session
            .items
            .iter()
            .filter_map(|i| match &i.content {
                ItemContent::User { text, .. }
                    if crate::checks::fix_prompt_label(text).is_none() =>
                {
                    Some(i.id)
                }
                _ => None,
            })
            .collect();
        let which = call.args["instruction"].as_u64().map(|n| n as usize);
        let item = match which {
            Some(n) if n >= 1 => messages.get(n - 1).copied(),
            _ => messages.last().copied(),
        };
        (item, session.items.len())
    });
    let item = item.ok_or("That thread has no such message.")?;
    cx.update(|cx| session.update(cx, |s, cx| s.undo_instruction(item, cx)));
    // It runs in the background; wait for its entry (or a notice).
    let deadline = Instant::now() + std::time::Duration::from_secs(120);
    loop {
        cx.background_executor()
            .timer(std::time::Duration::from_millis(100))
            .await;
        let result = cx.update(|cx| {
            session.read(cx).items[before..]
                .iter()
                .find_map(|i| match &i.content {
                    ItemContent::Undo {
                        instruction,
                        reverted,
                        kept,
                        ..
                    } => Some(Ok(format!(
                        "Undid “{}”.\nBack as before it: {}\nChanged again since, left as they are: {}",
                        instruction.lines().next().unwrap_or(""),
                        if reverted.is_empty() { "none".into() } else { reverted.join(", ") },
                        if kept.is_empty() { "none".into() } else { kept.join(", ") },
                    ))),
                    ItemContent::Notice { text, .. } => Some(Err(text.clone())),
                    _ => None,
                })
        });
        if let Some(result) = result {
            return result;
        }
        if Instant::now() > deadline {
            return Err("The undo is taking long; see the thread.".into());
        }
    }
}

async fn who_broke_it(
    app: &Entity<AppState>,
    call: &Call,
    cx: &mut AsyncApp,
) -> Result<String, String> {
    let (dir, project) = cx.update(|cx| provenance_scope(app, call, cx))?;
    let (command, threads) = cx.update(|cx| {
        let state = app.read(cx);
        let command = call.args["command"]
            .as_str()
            .map(str::to_string)
            .or_else(|| state.project(project).and_then(|p| p.check_command.clone()))
            .filter(|c| !c.trim().is_empty());
        (command, crate::bisect::threads_in(state, &dir))
    });
    let command = command.ok_or("Give a `command`, or set the project's check command.")?;
    let run_command = command.clone();
    let verdict = cx
        .background_executor()
        .spawn(async move {
            let found = (|| {
                let store = elyra_core::Store::open_read_only(&elyra_core::paths::database_path())?;
                let steps = crate::bisect::timeline(&store, &threads)?;
                let root = elyra_git::repo_root(&dir)?;
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                anyhow::Ok(crate::bisect::find(
                    &root,
                    &dir,
                    &run_command,
                    &steps,
                    &cancel,
                    |_| {},
                ))
            })();
            found.unwrap_or_else(|err| elyra_core::BisectVerdict::Failed {
                error: format!("{err:#}"),
            })
        })
        .await;
    Ok(crate::bisect::describe(&command, &verdict))
}

async fn changes_by_instruction(
    app: &Entity<AppState>,
    call: &Call,
    cx: &mut AsyncApp,
) -> Result<String, String> {
    let (dir, project) = cx.update(|cx| provenance_scope(app, call, cx))?;
    let overview = cx
        .background_executor()
        .spawn(async move {
            let root = elyra_git::repo_root(&dir)?;
            crate::instructions::load(&root, project, &elyra_core::paths::database_path())
        })
        .await
        .map_err(|err| format!("{err:#}"))?;
    if overview.groups.is_empty() {
        return Ok(format!(
            "The branch adds no lines compared with {}.",
            overview.base
        ));
    }
    Ok(format!(
        "Compared with {}, the branch adds lines for these instructions (oldest first):\n\n{}",
        overview.base,
        crate::instructions::markdown(&overview)
    ))
}

async fn intent_board(
    app: &Entity<AppState>,
    call: &Call,
    cx: &mut AsyncApp,
) -> Result<String, String> {
    let dir = cx.update(|cx| symbols_dir(app, call, cx))?;
    let (repo, root) = cx
        .background_executor()
        .spawn(async move {
            let repo = elyra_git::common_dir(&dir).map_err(|err| format!("{err:#}"))?;
            let root = elyra_git::repo_root(&dir).map_err(|err| format!("{err:#}"))?;
            Ok::<_, String>((repo, root.canonicalize().unwrap_or(root)))
        })
        .await?;
    Ok(cx.update(|cx| {
        let me = match call.caller {
            Caller::Thread(id) => Some(id),
            Caller::Client { .. } => thread_arg(&call.args, app, cx).ok(),
        };
        let rows =
            crate::intent_board::rows_for(app.read(cx), &repo, &root, me, chrono::Utc::now());
        if rows.is_empty() {
            "No other agent has changed files in this repository during the last hour.".into()
        } else {
            format!(
                "Changed by other agents during the last hour, newest first:\n{}",
                rows.iter()
                    .map(|row| format!("- {row}\n"))
                    .collect::<String>()
            )
        }
    }))
}

async fn history(app: &Entity<AppState>, call: &Call, cx: &mut AsyncApp) -> Result<String, String> {
    let path = call.args["path"].as_str().map(str::to_string);
    let symbol = call.args["symbol"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if path.is_none() && symbol.is_none() {
        return Err("Give a `path` or a `symbol`.".into());
    }
    let (dir, project) = cx.update(|cx| provenance_scope(app, call, cx))?;
    // (subject, [(repo-relative path, line hashes)], git log)
    let (subject, files, log) = cx
        .background_executor()
        .spawn(async move {
            if let Some(path) = path {
                let (root, relative) = crate::provenance::repo_relative(&dir, &path)
                    .ok_or_else(|| format!("{path} isn't in a Git repository."))?;
                let log = elyra_git::file_log(&root, &relative, 10).unwrap_or_default();
                return Ok::<_, String>((relative.clone(), vec![(relative, None)], log));
            }
            let symbol = symbol.unwrap_or_default();
            let db =
                elyra_index::default_db_path(&elyra_core::paths::data_dir().join("index"), &dir);
            let hits = {
                let _one_at_a_time = SYMBOL_LOOKUPS.lock().unwrap_or_else(|e| e.into_inner());
                let mut index =
                    elyra_index::SymbolIndex::open(&dir, &db).map_err(|e| e.to_string())?;
                index
                    .refresh_if_stale(elyra_index::REFRESH_MAX_AGE)
                    .map_err(|e| e.to_string())?;
                index
                    .definitions(&symbol, true, None, 10)
                    .map_err(|e| e.to_string())?
            };
            if hits.is_empty() {
                return Err(format!(
                    "No definition of \u{201c}{symbol}\u{201d} in the symbol index."
                ));
            }
            let mut files = Vec::new();
            let mut log = Vec::new();
            for hit in &hits {
                let Some((root, relative)) = crate::provenance::repo_relative(&dir, &hit.file)
                else {
                    continue;
                };
                let content = std::fs::read_to_string(root.join(&relative)).unwrap_or_default();
                let hashes: Vec<i64> = content
                    .lines()
                    .skip(hit.line.saturating_sub(1) as usize)
                    .take((hit.end_line.saturating_sub(hit.line) + 1) as usize)
                    .filter_map(elyra_core::line_hash)
                    .collect();
                if log.is_empty() {
                    log = elyra_git::file_log(&root, &relative, 10).unwrap_or_default();
                }
                files.push((relative, Some(hashes)));
            }
            let subject = format!(
                "{symbol}: {}",
                hits.iter()
                    .map(|h| format!("{}:{}", h.file, h.line))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            Ok((subject, files, log))
        })
        .await?;
    Ok(cx.update(|cx| {
        let state = app.read(cx);
        let mut counts: Vec<(elyra_core::LineOrigin, usize)> = Vec::new();
        for (relative, hashes) in &files {
            let origins: Vec<elyra_core::LineOrigin> = match hashes {
                None => state
                    .store
                    .file_origins(project, relative, 20)
                    .unwrap_or_default(),
                Some(hashes) => hashes
                    .iter()
                    .flat_map(|hash| {
                        state
                            .store
                            .line_origins(project, relative, *hash)
                            .unwrap_or_default()
                    })
                    .map(|origin| elyra_core::LineOrigin { lines: 1, ..origin })
                    .collect(),
            };
            for origin in origins {
                match counts.iter_mut().find(|(o, _)| o.item == origin.item) {
                    Some((_, count)) => *count += origin.lines,
                    None => {
                        let lines = origin.lines;
                        counts.push((origin, lines));
                    }
                }
            }
        }
        counts.sort_by_key(|(origin, _)| std::cmp::Reverse(origin.at));
        let turns: Vec<_> = counts
            .iter()
            .take(20)
            .map(|(origin, lines)| (crate::provenance::turn(state, origin), *lines))
            .collect();
        crate::provenance::history_text(&subject, &turns, &log)
    }))
}

fn thread_arg(args: &Value, app: &Entity<AppState>, cx: &App) -> Result<ThreadId, String> {
    let id: ThreadId = arg(args, "thread_id")?
        .parse()
        .map_err(|_| "`thread_id` is not a thread id".to_string())?;
    app.read(cx)
        .thread(id)
        .map(|t| t.id)
        .ok_or_else(|| format!("no thread {id}"))
}

fn not_self(caller: &Caller, id: ThreadId, what: &str) -> Result<(), String> {
    match caller {
        Caller::Thread(own) if *own == id => Err(format!("A thread cannot {what} itself.")),
        _ => Ok(()),
    }
}

fn status_name(status: ThreadStatus) -> &'static str {
    match status {
        ThreadStatus::Idle => "idle",
        ThreadStatus::Running => "running",
        ThreadStatus::NeedsApproval => "needs_approval",
        ThreadStatus::Failed => "failed",
        ThreadStatus::Interrupted => "interrupted",
    }
}

/// `search_threads`: matches grouped by thread, best thread first. The
/// search itself runs off the main thread.
async fn search_threads(
    app: &Entity<AppState>,
    call: &Call,
    cx: &mut AsyncApp,
) -> Result<String, String> {
    let args = &call.args;
    let query = arg(args, "query")?.to_string();
    let filter = cx.update(|cx| search_filter(app.read(cx), args))?;
    let limit = args["limit"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
    let hits = cx
        .update(|cx| crate::thread_search::search_in_background(query.clone(), filter, limit, cx))
        .await
        .map_err(|err| format!("{err:#}"))?;
    Ok(cx.update(|cx| format_hits(app.read(cx), &query, &hits)))
}

fn search_filter(state: &AppState, args: &Value) -> Result<elyra_core::SearchFilter, String> {
    let project = match args["project"].as_str().map(str::to_lowercase) {
        Some(wanted) => Some(
            state
                .projects
                .iter()
                .find(|p| {
                    p.id.to_string() == wanted
                        || p.name.to_lowercase() == wanted
                        || p.path.display().to_string().to_lowercase() == wanted
                })
                .map(|p| p.id)
                .ok_or_else(|| format!("no project matches “{wanted}” (see list_projects)"))?,
        ),
        None => None,
    };
    Ok(elyra_core::SearchFilter {
        project,
        archived: args["archived"].as_bool().unwrap_or(false),
        messages_only: args["messages_only"].as_bool().unwrap_or(false),
    })
}

fn format_hits(state: &AppState, query: &str, hits: &[elyra_core::SearchHit]) -> String {
    if hits.is_empty() {
        return format!("Nothing in any thread matches “{query}”.");
    }
    let archived = state.archived_threads();
    let mut groups: Vec<(ThreadId, Vec<&elyra_core::SearchHit>)> = Vec::new();
    for hit in hits {
        match groups.iter_mut().find(|(thread, _)| *thread == hit.thread) {
            Some((_, list)) => list.push(hit),
            None => groups.push((hit.thread, vec![hit])),
        }
    }
    let mut out = format!(
        "{} match{} in {} thread{}, best first:\n",
        hits.len(),
        if hits.len() == 1 { "" } else { "es" },
        groups.len(),
        if groups.len() == 1 { "" } else { "s" }
    );
    for (id, list) in groups {
        let thread = state
            .thread(id)
            .or_else(|| archived.iter().find(|t| t.id == id));
        let title = thread.map(|t| t.title.as_str()).unwrap_or("Thread");
        let project = thread
            .and_then(|t| state.project(t.project_id))
            .map(|p| p.name.as_str())
            .unwrap_or("");
        out.push_str(&format!("\n“{title}” · {project} · thread_id {id}\n"));
        for hit in list {
            let who = match hit.kind.as_str() {
                "user" => "User",
                "assistant" => "Agent",
                "tool_use" => "Tool call",
                _ => "Tool output",
            };
            out.push_str(&format!(
                "  {who}, {}: {}\n",
                hit.at.format("%Y-%m-%d"),
                hit.plain_snippet()
            ));
        }
    }
    out
}

fn run_tool(app: &Entity<AppState>, call: &Call, cx: &mut App) -> Result<String, String> {
    let args = &call.args;
    match call.tool.as_str() {
        "list_projects" => {
            let projects: Vec<Value> = app
                .read(cx)
                .projects
                .iter()
                .map(|p| json!({ "id": p.id.to_string(), "name": p.name, "path": p.path }))
                .collect();
            Ok(serde_json::to_string_pretty(&projects).unwrap_or_default())
        }
        "list_threads" => {
            let state = app.read(cx);
            let filter = args["project"].as_str().map(str::to_lowercase);
            let include_archived = args["include_archived"].as_bool().unwrap_or(false);
            let mut threads = state.threads.clone();
            if include_archived {
                threads.extend(state.archived_threads());
            }
            let list: Vec<Value> = threads
                .iter()
                .filter_map(|t| {
                    let project = state.project(t.project_id)?;
                    if let Some(filter) = &filter
                        && project.id.to_string() != *filter
                        && project.name.to_lowercase() != *filter
                        && project.path.display().to_string().to_lowercase() != *filter
                    {
                        return None;
                    }
                    Some(json!({
                        "id": t.id.to_string(),
                        "title": t.title,
                        "project": project.name,
                        "provider": t.provider.as_str(),
                        "status": status_name(t.status),
                        "archived": t.archived,
                        "updated_at": t.updated_at.to_rfc3339(),
                    }))
                })
                .collect();
            Ok(serde_json::to_string_pretty(&list).unwrap_or_default())
        }
        "read_thread" => {
            let id = thread_arg(args, app, cx)?;
            let budget = args["max_chars"]
                .as_u64()
                .unwrap_or(12_000)
                .clamp(500, 100_000) as usize;
            let state = app.read(cx);
            let thread = state.thread(id).cloned().ok_or("no such thread")?;
            let items = state.store.transcript(id).map_err(|e| e.to_string())?;
            Ok(format!(
                "Thread: {}\nStatus: {}\n\n{}",
                thread.title,
                status_name(thread.status),
                transcript_text(&items, budget)
            ))
        }
        "propose_automation" => propose_automation(app, call, cx),
        "propose_rule" => {
            let Caller::Thread(thread_id) = call.caller else {
                return Err("Only an agent working in a thread can propose a rule.".into());
            };
            let rule = arg(args, "rule")?.trim().to_string();
            if rule.chars().count() > 400 {
                return Err("A rule is one short sentence; this is too long.".into());
            }
            let reason = args["reason"].as_str().unwrap_or("").trim().to_string();
            let paths = args["paths"]
                .as_str()
                .map(|p| p.trim().trim_start_matches("./").to_string())
                .filter(|p| !p.is_empty() && p != "**" && p != "**/*");
            if paths
                .as_deref()
                .is_some_and(|p| p.starts_with('/') || p.split('/').any(|part| part == ".."))
            {
                return Err("`paths` is a glob inside the project, like app/Models/**.".into());
            }
            let session = app
                .update(cx, |app, cx| app.session(thread_id, cx))
                .ok_or("could not open the thread")?;
            session.update(cx, |session, cx| {
                session.propose_rule(rule.clone(), reason, paths, cx)
            });
            Ok(format!(
                "Proposed the rule \u{201c}{rule}\u{201d} as a card in this thread; it is kept only if the user accepts it. Follow it for the rest of this task anyway."
            ))
        }
        "create_thread" => {
            let project_query = arg(args, "project")?.to_lowercase();
            let prompt = arg(args, "prompt")?.to_string();
            let project = app
                .read(cx)
                .projects
                .iter()
                .find(|p| {
                    p.id.to_string() == project_query
                        || p.name.to_lowercase() == project_query
                        || p.path.display().to_string().to_lowercase() == project_query
                })
                .cloned()
                .ok_or_else(|| {
                    format!("no project matches “{project_query}” (see list_projects)")
                })?;
            let thread = app
                .update(cx, |app, cx| app.create_thread(project.id, cx))
                .map_err(|e| format!("{e:#}"))?;
            let session = app
                .update(cx, |app, cx| app.session(thread.id, cx))
                .ok_or("could not open the thread")?;
            session.update(cx, |session, cx| {
                if let Some(kind) = args["provider"].as_str().and_then(ProviderKind::parse) {
                    session.set_provider(kind, cx);
                }
                if let Some(model) = args["model"].as_str() {
                    session.set_model(Some(model.to_string()), cx);
                }
                if let Some(title) = args["title"].as_str() {
                    session.rename(title.to_string(), cx);
                }
                if args["worktree"].as_bool() == Some(true) {
                    session.use_worktree = true;
                }
                session.submit(Prompt::text(prompt), cx);
            });
            // An agent may steer the threads it starts without asking again.
            if let Caller::Thread(from) = call.caller {
                crate::thread_access::remember(app.read(cx), from, thread.id);
            }
            Ok(json!({ "thread_id": thread.id.to_string() }).to_string())
        }
        "send_message" => {
            let id = thread_arg(args, app, cx)?;
            let text = arg(args, "text")?.to_string();
            let session = app
                .update(cx, |app, cx| app.session(id, cx))
                .ok_or("could not open the thread")?;
            let queued = session.update(cx, |session, cx| {
                let busy = session.running || session.preparing.is_some();
                session.submit(Prompt::text(text), cx);
                busy
            });
            Ok(if queued {
                "queued behind the running turn"
            } else {
                "sent"
            }
            .into())
        }
        "interrupt_thread" => {
            let id = thread_arg(args, app, cx)?;
            not_self(&call.caller, id, "interrupt")?;
            if let Some(session) = app.read(cx).existing_session(id) {
                session.update(cx, |session, cx| session.interrupt(cx));
            }
            Ok("interrupted".into())
        }
        "set_thread_title" => {
            let id = thread_arg(args, app, cx)?;
            let title = arg(args, "title")?.to_string();
            app.update(cx, |app, cx| {
                app.update_thread(id, |t| t.title = title.clone(), cx)
            });
            Ok("renamed".into())
        }
        "archive_thread" => {
            let id = thread_arg(args, app, cx)?;
            not_self(&call.caller, id, "archive")?;
            app.update(cx, |app, cx| app.archive_thread(id, cx));
            Ok("archived".into())
        }
        other => Err(format!("unknown tool {other}")),
    }
}

/// The thread's status and the text of its last reply.
fn thread_report(app: &Entity<AppState>, id: ThreadId, cx: &App) -> String {
    let state = app.read(cx);
    let Some(thread) = state.thread(id) else {
        return "The thread no longer exists.".into();
    };
    let last = state.existing_session(id).and_then(|session| {
        session
            .read(cx)
            .items
            .iter()
            .rev()
            .find_map(|item| match &item.content {
                ItemContent::Assistant {
                    text,
                    parent_tool_use_id: None,
                } => Some(text.clone()),
                _ => None,
            })
    });
    format!(
        "Status: {}\n\nLast reply:\n{}",
        status_name(thread.status),
        last.unwrap_or_else(|| "(none yet)".into())
    )
}

async fn wait_for_thread(
    app: &Entity<AppState>,
    call: &Call,
    cx: &mut AsyncApp,
) -> Result<String, String> {
    let id = cx.update(|cx| thread_arg(&call.args, app, cx))?;
    not_self(&call.caller, id, "wait for")?;
    let timeout =
        Duration::from_secs(call.args["timeout_seconds"].as_u64().unwrap_or(600)).min(MAX_WAIT);
    Ok(if settle(app, id, timeout, cx).await {
        cx.update(|cx| thread_report(app, id, cx))
    } else {
        format!(
            "Still running after {} s.\n\n{}",
            timeout.as_secs(),
            cx.update(|cx| thread_report(app, id, cx))
        )
    })
}

/// Wait until the thread's turn is over (or it waits for approval); false
/// when it is still running after `timeout`.
async fn settle(
    app: &Entity<AppState>,
    id: ThreadId,
    timeout: Duration,
    cx: &mut AsyncApp,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let settled = cx.update(|cx| {
            let state = app.read(cx);
            let running = state.existing_session(id).is_some_and(|s| {
                let s = s.read(cx);
                s.running || s.preparing.is_some() || !s.queued.is_empty()
            });
            let status = state.thread(id).map(|t| t.status);
            status.is_none()
                || (!running && status != Some(ThreadStatus::Running))
                || status == Some(ThreadStatus::NeedsApproval)
        });
        if settled {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        cx.background_executor()
            .timer(Duration::from_millis(500))
            .await;
    }
}

/// The app's latest logged errors, traced to the instructions behind them.
async fn error_origin(
    app: &Entity<AppState>,
    call: &Call,
    cx: &mut AsyncApp,
) -> Result<String, String> {
    let limit = call.args["limit"].as_u64().unwrap_or(3).clamp(1, 10) as usize;
    let (dir, _) = cx.update(|cx| provenance_scope(app, call, cx))?;
    let (root, found) = cx
        .background_executor()
        .spawn(async move {
            let root = elyra_git::repo_root(&dir).unwrap_or_else(|_| dir.clone());
            let found: Vec<_> = crate::app_errors::latest(&dir, limit)
                .into_iter()
                .map(|error| {
                    let culprit = crate::app_errors::culprit(&root, &error);
                    (error, culprit)
                })
                .collect();
            (root, found)
        })
        .await;
    if found.is_empty() {
        return Ok("No errors in the app's log (storage/logs/laravel*.log).".into());
    }
    Ok(cx.update(|cx| {
        let state = app.read(cx);
        found
            .into_iter()
            .map(|(error, culprit)| {
                crate::app_errors::describe(&crate::app_errors::trace(state, &root, error, culprit))
            })
            .collect::<Vec<_>>()
            .join("\n---\n\n")
    }))
}

/// Ask the agent that wrote a line (see `ask_line`) and wait for its answer.
async fn ask_author(
    app: &Entity<AppState>,
    call: &Call,
    cx: &mut AsyncApp,
) -> Result<String, String> {
    let path = arg(&call.args, "path")?.to_string();
    let line = call.args["line"]
        .as_u64()
        .filter(|l| *l > 0)
        .ok_or("Give the 1-based `line`.")? as u32;
    let question = call.args["question"]
        .as_str()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .unwrap_or(crate::ask_line::DEFAULT_QUESTION)
        .to_string();
    let timeout =
        Duration::from_secs(call.args["timeout_seconds"].as_u64().unwrap_or(300)).min(MAX_WAIT);
    let (dir, _) = cx.update(|cx| provenance_scope(app, call, cx))?;
    let (root, relative, text) = cx
        .background_executor()
        .spawn(async move {
            let (root, relative) = crate::provenance::repo_relative(&dir, &path)
                .ok_or_else(|| format!("{path} isn't in a Git repository."))?;
            let content = std::fs::read_to_string(root.join(&relative))
                .map_err(|err| format!("{relative}: {err}"))?;
            let text = content
                .lines()
                .nth(line as usize - 1)
                .ok_or_else(|| format!("{relative} has fewer than {line} lines."))?
                .to_string();
            Ok::<_, String>((root, relative, text))
        })
        .await?;
    let request = crate::ask_line::Request {
        parent: match call.caller {
            Caller::Thread(id) => Some(id),
            Caller::Client { .. } => None,
        },
        root,
        path: relative.clone(),
        line: Some(line),
        text,
        question,
    };
    let chat = crate::ask_line::ask(app, request, cx)
        .await
        .map_err(|err| format!("{relative}:{line}: {err:#}"))?;
    let id = cx.update(|cx| chat.read(cx).thread.id);
    let done = settle(app, id, timeout, cx).await;
    let reply = cx.update(|cx| chat.read(cx).last_reply());
    Ok(format!(
        "{}The agent that wrote {relative}:{line} answered (thread {id}):\n\n{}",
        if done {
            String::new()
        } else {
            format!(
                "Still answering after {} s; this is its reply so far. ",
                timeout.as_secs()
            )
        },
        if reply.trim().is_empty() {
            "(no answer)".to_string()
        } else {
            reply.trim().to_string()
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::{Caller, GatewayHandler, is_read_tool, tools};
    use elyra_core::ClientScope;
    use elyra_mcp::Handler as _;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};

    #[test]
    fn callers_round_trip_through_principals() {
        let thread = Caller::Thread(elyra_core::new_id());
        assert_eq!(Caller::parse(&thread.principal()), Some(thread.clone()));
        let client = Caller::Client {
            id: elyra_core::new_id(),
            scope: ClientScope::ReadOnly,
        };
        assert_eq!(Caller::parse(&client.principal()), Some(client.clone()));
        assert!(thread.can_write() && !client.can_write());
    }

    fn handler(
        tokens: &[(&str, &Caller)],
    ) -> (GatewayHandler, async_channel::Receiver<super::Call>) {
        let (calls, received) = async_channel::unbounded();
        let tokens = tokens
            .iter()
            .map(|(token, caller)| (token.to_string(), caller.principal()))
            .collect::<HashMap<_, _>>();
        let handler = GatewayHandler {
            tokens: Arc::new(RwLock::new(tokens)),
            calls,
        };
        (handler, received)
    }

    #[test]
    fn unknown_and_empty_tokens_are_refused() {
        let thread = Caller::Thread(elyra_core::new_id());
        let (handler, _) = handler(&[("", &thread), ("good", &thread)]);
        assert_eq!(handler.authorize(""), None, "even if one was stored");
        assert_eq!(handler.authorize("bad"), None);
        assert_eq!(handler.authorize("good"), Some(thread.principal()));
    }

    #[test]
    fn read_only_clients_cannot_change_anything() {
        let reader = Caller::Client {
            id: elyra_core::new_id(),
            scope: ClientScope::ReadOnly,
        };
        let full = Caller::Client {
            id: elyra_core::new_id(),
            scope: ClientScope::Full,
        };
        let (handler, received) = handler(&[("r", &reader), ("f", &full)]);
        let reader = handler.authorize("r").unwrap();
        let full = handler.authorize("f").unwrap();

        let visible: Vec<String> = handler.tools(&reader).into_iter().map(|t| t.name).collect();
        assert!(visible.iter().all(|name| is_read_tool(name)), "{visible:?}");
        assert!(handler.tools(&full).len() > visible.len());

        for tool in handler.tools(&full) {
            if is_read_tool(&tool.name) {
                continue;
            }
            let call = json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name": tool.name, "arguments": {"thread_id": "x", "project": "x"}}});
            let reply = elyra_mcp::handle(&handler, &reader, &call).unwrap();
            assert_eq!(reply["error"]["code"], -32602, "{} was allowed", tool.name);
        }
        assert!(received.try_recv().is_err(), "no call reached the app");
    }

    #[test]
    fn tools_that_change_a_thread_ask_first() {
        // An agent changing another thread is asked about (thread_access);
        // a new tool that takes a thread id must be a read or on that list.
        for tool in tools().into_iter().chain(crate::browser_tools::tools()) {
            if tool.input_schema["properties"].get("thread_id").is_some() {
                assert!(
                    is_read_tool(&tool.name)
                        || crate::thread_access::WRITES.contains(&tool.name.as_str()),
                    "{} takes a thread id but is neither a read nor in thread_access::WRITES",
                    tool.name
                );
            }
        }
        for name in super::READ_TOOLS
            .iter()
            .chain(crate::browser_tools::READ_TOOLS)
        {
            assert!(
                tools()
                    .into_iter()
                    .chain(crate::browser_tools::tools())
                    .any(|tool| tool.name == *name),
                "read tool {name} does not exist"
            );
        }
    }
}
