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
    })
}

async fn serve_call(app: Entity<AppState>, call: Call, cx: &mut AsyncApp) {
    let started = Instant::now();
    let result = if !call.caller.can_write() && !is_read_tool(&call.tool) {
        Err("This client has read-only access.".to_string())
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

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args[key]
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("missing `{key}`"))
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
            return Ok(cx.update(|cx| thread_report(app, id, cx)));
        }
        if Instant::now() >= deadline {
            return Ok(format!(
                "Still running after {} s.\n\n{}",
                timeout.as_secs(),
                cx.update(|cx| thread_report(app, id, cx))
            ));
        }
        cx.background_executor()
            .timer(Duration::from_millis(500))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::Caller;
    use elyra_core::ClientScope;

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
}
