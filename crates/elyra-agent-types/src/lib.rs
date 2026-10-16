//! Types shared by everything that talks to coding agents: which agent, how
//! freely it may act, and the questions and approvals it raises. They live
//! apart from `elyra-core` so `elyra-provider` stands alone (Elyra Workspace
//! and Huskarl both use it); `elyra-core` re-exports them.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Claude,
    Elyra,
    /// Google Gemini CLI over ACP.
    Gemini,
    /// Cursor Agent CLI over ACP.
    Cursor,
    /// OpenCode over ACP.
    OpenCode,
    /// The Pi coding agent (same RPC protocol as Elyra).
    Pi,
    /// A user-configured Agent Client Protocol command.
    CustomAcp,
    /// OpenAI Codex (`codex app-server`).
    Codex,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 8] = [
        ProviderKind::Claude,
        ProviderKind::Codex,
        ProviderKind::Elyra,
        ProviderKind::Gemini,
        ProviderKind::Cursor,
        ProviderKind::OpenCode,
        ProviderKind::Pi,
        ProviderKind::CustomAcp,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Claude => "claude",
            ProviderKind::Elyra => "elyra",
            ProviderKind::Gemini => "gemini",
            ProviderKind::Cursor => "cursor",
            ProviderKind::OpenCode => "opencode",
            ProviderKind::Pi => "pi",
            ProviderKind::CustomAcp => "acp",
            ProviderKind::Codex => "codex",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Claude => "Claude Code",
            ProviderKind::Elyra => "Elyra",
            ProviderKind::Gemini => "Gemini CLI",
            ProviderKind::Cursor => "Cursor Agent",
            ProviderKind::OpenCode => "OpenCode",
            ProviderKind::Pi => "Pi",
            ProviderKind::CustomAcp => "Custom agent (ACP)",
            ProviderKind::Codex => "Codex",
        }
    }
}

/// How the provider is allowed to act without asking. Mirrors the Claude CLI
/// permission modes Elyra exposes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Ask for approval before edits and commands.
    #[default]
    Ask,
    /// Auto-approve file edits, ask for everything else.
    AcceptEdits,
    /// Read-only planning.
    Plan,
    /// Full access, nothing is asked.
    FullAccess,
}

impl PermissionMode {
    pub const ALL: [PermissionMode; 4] = [
        PermissionMode::Ask,
        PermissionMode::AcceptEdits,
        PermissionMode::Plan,
        PermissionMode::FullAccess,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PermissionMode::Ask => "ask",
            PermissionMode::AcceptEdits => "accept_edits",
            PermissionMode::Plan => "plan",
            PermissionMode::FullAccess => "full_access",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "accept_edits" => PermissionMode::AcceptEdits,
            "plan" => PermissionMode::Plan,
            "full_access" => PermissionMode::FullAccess,
            _ => PermissionMode::Ask,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PermissionMode::Ask => "Ask for approval",
            PermissionMode::AcceptEdits => "Accept edits",
            PermissionMode::Plan => "Plan only",
            PermissionMode::FullAccess => "Full access",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Allowed,
    AllowedForSession,
    Denied,
}

/// A structured question the agent asks the user.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuestionRequest {
    pub request_id: String,
    pub kind: QuestionKind,
    /// Dialog title (provider dialogs) — empty for agent questions.
    #[serde(default)]
    pub title: String,
    pub questions: Vec<Question>,
    /// Original request payload, echoed back when answering.
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    /// One or more multiple-choice questions.
    Choice,
    /// Yes/no confirmation.
    Confirm,
    /// Free-form text (single or multi-line).
    Text,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multi_select: bool,
    /// Prefilled text for text questions.
    #[serde(default)]
    pub prefill: String,
    #[serde(default)]
    pub multiline: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuestionAnswer {
    /// One answer per question, in order (multi-select joined with ", ").
    Answers {
        answers: Vec<String>,
    },
    Confirmed {
        confirmed: bool,
    },
    Cancelled,
}
