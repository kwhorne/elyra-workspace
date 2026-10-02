//! Coding-agent provider adapters. Each adapter owns its provider's wire
//! protocol and translates it into provider-neutral [`ProviderEvent`]s behind
//! the [`AgentSession`] trait.

pub mod claude;
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
    /// Start a new session that branches from `resume_session_id`.
    pub fork: bool,
    /// Extra instructions appended to the provider's system prompt.
    pub append_system_prompt: Option<String>,
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
        ProviderKind::Elyra => elyra::CAPABILITIES,
    }
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
    }
}

/// Whether the provider's CLI is installed.
pub fn is_installed(kind: ProviderKind) -> bool {
    match kind {
        ProviderKind::Claude => claude::find_executable().is_some(),
        ProviderKind::Elyra => elyra::find_executable().is_some(),
    }
}

/// One-shot text generation with the provider's CLI in print mode (titles,
/// commit messages). Blocking; run it on a background thread.
pub fn generate_text(kind: ProviderKind, cwd: &std::path::Path, prompt: &str) -> Result<String> {
    let (executable, args): (PathBuf, Vec<&str>) = match kind {
        ProviderKind::Claude => (
            claude::find_executable().ok_or_else(|| anyhow::anyhow!("`claude` not found"))?,
            vec!["-p", "--output-format", "text", "--model", "haiku", prompt],
        ),
        ProviderKind::Elyra => (
            elyra::find_executable().ok_or_else(|| anyhow::anyhow!("`elyra` not found"))?,
            vec!["-p", "--no-session", "--no-tools", prompt],
        ),
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
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
