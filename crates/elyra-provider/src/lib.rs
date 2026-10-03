//! Coding-agent provider adapters. Each adapter owns its provider's wire
//! protocol and translates it into provider-neutral [`ProviderEvent`]s behind
//! the [`AgentSession`] trait.

pub mod acp;
pub mod claude;
pub mod claude_history;
pub mod elyra;
mod events;
mod process;

pub use events::*;

use anyhow::Result;
use elyra_core::{PermissionMode, ProviderKind};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub cwd: PathBuf,
    /// Model id; `None` uses the provider default.
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: PermissionMode,
    /// Provider-specific handle from [`ProviderEvent::SessionStarted`].
    pub resume_session_id: Option<String>,
    /// Override the provider executable.
    pub executable: Option<PathBuf>,
    /// Override the launch arguments (ACP agents; empty uses the defaults).
    pub args: Vec<String>,
    /// Extra environment variables (provider settings and accounts).
    pub env: Vec<(String, String)>,
    /// Start a new session that branches from `resume_session_id`.
    pub fork: bool,
    /// Extra instructions appended to the provider's system prompt.
    pub append_system_prompt: Option<String>,
    /// MCP servers to give the agent (providers that support MCP).
    pub mcp_servers: Vec<McpServer>,
}

/// An MCP server reachable over HTTP, with a stdio bridge command for
/// agents that only launch local servers.
#[derive(Clone, Debug, PartialEq)]
pub struct McpServer {
    pub name: String,
    pub url: String,
    pub token: String,
    /// Command that relays stdio to `url` (e.g. `elyra mcp-bridge …`).
    pub bridge: Option<(PathBuf, Vec<String>)>,
}

/// What a provider supports, so the UI only offers what works.
#[derive(Clone, Debug)]
pub struct Capabilities {
    /// Tool approvals and permission modes.
    pub permission_modes: bool,
    /// Effort/thinking levels, in display order (empty = not supported).
    pub efforts: &'static [(&'static str, &'static str)],
    /// Built-in model suggestions; runtime discovery may add more.
    pub models: &'static [(&'static str, &'static str)],
    /// Inject a message into a running turn.
    pub steer: bool,
    pub compact: bool,
    /// Image attachments.
    pub images: bool,
}

pub fn capabilities(kind: ProviderKind) -> Capabilities {
    match kind {
        ProviderKind::Claude => claude::CAPABILITIES,
        ProviderKind::Elyra | ProviderKind::Pi => elyra::CAPABILITIES,
        ProviderKind::Gemini
        | ProviderKind::Cursor
        | ProviderKind::OpenCode
        | ProviderKind::CustomAcp => acp::CAPABILITIES,
    }
}

/// Whether a new session can branch from an existing one natively.
pub fn supports_fork(kind: ProviderKind) -> bool {
    matches!(
        kind,
        ProviderKind::Claude | ProviderKind::Elyra | ProviderKind::Pi
    )
}

/// The provider's CLI binary name and an install command.
pub fn install_info(kind: ProviderKind) -> Option<(&'static str, &'static str)> {
    match kind {
        ProviderKind::Claude => Some(("claude", "npm install -g @anthropic-ai/claude-code")),
        ProviderKind::Elyra => Some(("elyra", "npm install -g @elyracode/coding-agent")),
        ProviderKind::Pi => Some(("pi", "npm install -g @mariozechner/pi-coding-agent")),
        ProviderKind::CustomAcp => None,
        other => acp::agent(other).map(|a| (a.binary, a.install_hint)),
    }
}

/// The interactive command that signs in to the provider.
pub fn login_command(kind: ProviderKind) -> Option<&'static str> {
    match kind {
        ProviderKind::Claude => Some("claude /login"),
        ProviderKind::Elyra => Some("elyra"),
        ProviderKind::Pi => Some("pi"),
        ProviderKind::Gemini => Some("gemini"),
        ProviderKind::Cursor => Some("cursor-agent login"),
        ProviderKind::OpenCode => Some("opencode auth login"),
        ProviderKind::CustomAcp => None,
    }
}

/// Locate the provider's CLI.
pub fn find_executable(kind: ProviderKind) -> Option<PathBuf> {
    match kind {
        ProviderKind::Claude => claude::find_executable(),
        ProviderKind::Elyra => elyra::find_executable(),
        ProviderKind::Pi => elyra::find_pi(),
        ProviderKind::CustomAcp => None,
        other => acp::find_executable(other),
    }
}

/// `<cli> --version`, first line.
pub fn cli_version(executable: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new(executable)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.lines().next().unwrap_or("").trim().to_string()).filter(|v| !v.is_empty())
}

/// A live provider process for one thread. Dropping it terminates the process.
pub trait AgentSession: Send {
    /// Start a new turn.
    fn send(&self, prompt: &Prompt) -> Result<()>;
    /// Deliver a message into the running turn (when supported).
    fn steer(&self, prompt: &Prompt) -> Result<()>;
    fn respond_permission(&self, request_id: &str, response: PermissionResponse) -> Result<()>;
    fn answer_question(&self, request_id: &str, answer: QuestionAnswer) -> Result<()>;
    fn interrupt(&self) -> Result<()>;
    fn set_permission_mode(&self, mode: PermissionMode) -> Result<()>;
    fn set_model(&self, model: Option<&str>) -> Result<()>;
    /// Returns `false` when the change needs a session restart to apply.
    fn set_effort(&self, effort: Option<&str>) -> Result<bool>;
    fn compact(&self) -> Result<()>;
    /// Ask the provider to report fresh usage (if it reports on demand).
    fn refresh_usage(&self) -> Result<()> {
        Ok(())
    }
    fn shutdown(&self);
}

pub fn start_session(
    kind: ProviderKind,
    config: SessionConfig,
) -> Result<(
    Box<dyn AgentSession>,
    async_channel::Receiver<ProviderEvent>,
)> {
    match kind {
        ProviderKind::Claude => {
            let (session, events) = claude::ClaudeSession::start(config)?;
            Ok((Box::new(session), events))
        }
        ProviderKind::Elyra => {
            let (session, events) = elyra::ElyraSession::start(config)?;
            Ok((Box::new(session), events))
        }
        ProviderKind::Pi => {
            let (session, events) = elyra::ElyraSession::start_pi(config)?;
            Ok((Box::new(session), events))
        }
        ProviderKind::Gemini
        | ProviderKind::Cursor
        | ProviderKind::OpenCode
        | ProviderKind::CustomAcp => {
            let (session, events) = acp::AcpSession::start(kind, config)?;
            Ok((Box::new(session), events))
        }
    }
}

/// Whether the provider's CLI is installed.
pub fn is_installed(kind: ProviderKind) -> bool {
    find_executable(kind).is_some()
}

/// One-shot text generation with the provider's CLI in print mode (titles,
/// commit messages). Blocking; run it on a background thread. Providers
/// without a print mode fall back to Claude Code, then Elyra.
pub fn generate_text(kind: ProviderKind, cwd: &std::path::Path, prompt: &str) -> Result<String> {
    let mut errors = Vec::new();
    for kind in [kind, ProviderKind::Claude, ProviderKind::Elyra] {
        match generate_with(kind, cwd, prompt) {
            Ok(Some(text)) => return Ok(text),
            Ok(None) => {}
            Err(err) => errors.push(format!("{}: {err:#}", kind.label())),
        }
    }
    match errors.into_iter().next() {
        Some(error) => Err(anyhow::anyhow!(error)),
        None => Err(anyhow::anyhow!("no installed agent can generate text")),
    }
}

/// `Ok(None)` when the provider is missing or has no print mode.
fn generate_with(
    kind: ProviderKind,
    cwd: &std::path::Path,
    prompt: &str,
) -> Result<Option<String>> {
    let Some(executable) = find_executable(kind) else {
        return Ok(None);
    };
    let args: Vec<&str> = match kind {
        ProviderKind::Claude => vec!["-p", "--output-format", "text", "--model", "haiku", prompt],
        ProviderKind::Elyra | ProviderKind::Pi => vec!["-p", "--no-session", "--no-tools", prompt],
        ProviderKind::Gemini => vec!["-p", prompt],
        ProviderKind::Cursor => vec!["-p", "--output-format", "text", prompt],
        ProviderKind::OpenCode => vec!["run", prompt],
        ProviderKind::CustomAcp => return Ok(None),
    };
    let output = std::process::Command::new(executable)
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "generation failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

/// Clean a generated title: first line, no quotes or trailing period.
pub fn clean_title(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let line = line
        .trim_start_matches(['#', '*', ' '])
        .trim_matches(['"', '\'', '`', '*'])
        .trim_end_matches('.');
    line.chars().take(80).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn cleans_generated_titles() {
        assert_eq!(
            super::clean_title("\n\"Fix login redirect.\"\n"),
            "Fix login redirect"
        );
        assert_eq!(
            super::clean_title("## **Refactor parser**"),
            "Refactor parser"
        );
    }
}
