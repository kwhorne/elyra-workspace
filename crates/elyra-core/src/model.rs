use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

pub type ProjectId = Uuid;
pub type ThreadId = Uuid;
pub type ItemId = Uuid;

pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub pinned: bool,
    /// Emoji shown instead of the folder icon.
    #[serde(default)]
    pub icon: Option<String>,
    /// Accent colour as `#rrggbb`.
    #[serde(default)]
    pub color: Option<String>,
    /// Named group the project belongs to (Spaces).
    #[serde(default)]
    pub space: Option<String>,
    /// Extra instructions given to agents in this project.
    #[serde(default)]
    pub instructions: Option<String>,
    /// Shell command run after a turn that changed files (tests, lint);
    /// failures go back to the agent.
    #[serde(default)]
    pub check_command: Option<String>,
}

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    #[default]
    Idle,
    Running,
    NeedsApproval,
    Failed,
    /// The app quit while a turn was running.
    Interrupted,
}

impl ThreadStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadStatus::Idle => "idle",
            ThreadStatus::Running => "running",
            ThreadStatus::NeedsApproval => "needs_approval",
            ThreadStatus::Failed => "failed",
            ThreadStatus::Interrupted => "interrupted",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "running" => ThreadStatus::Running,
            "needs_approval" => ThreadStatus::NeedsApproval,
            "failed" => ThreadStatus::Failed,
            "interrupted" => ThreadStatus::Interrupted,
            _ => ThreadStatus::Idle,
        }
    }
}

/// Where a thread does its work: the project checkout, or an isolated
/// managed Git worktree on its own branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Environment {
    Local,
    Worktree { path: PathBuf, branch: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub id: ThreadId,
    pub project_id: ProjectId,
    pub title: String,
    pub provider: ProviderKind,
    pub model: Option<String>,
    /// Effort/thinking level understood by the provider.
    pub effort: Option<String>,
    pub permission_mode: PermissionMode,
    /// Provider-native session id used to resume the conversation.
    pub provider_session_id: Option<String>,
    pub environment: Environment,
    pub status: ThreadStatus,
    pub archived: bool,
    pub pinned: bool,
    /// Marked finished by the user; hidden from the active list.
    pub done: bool,
    /// When the user last looked at the thread.
    pub read_at: Option<DateTime<Utc>>,
    /// When the agent last finished a turn or asked for input.
    pub last_activity_at: Option<DateTime<Utc>>,
    /// Set for side chats: the thread they branched from.
    pub parent_id: Option<ThreadId>,
    /// Autosaved scratchpad.
    pub notes: Option<String>,
    /// Cached AI summary of the thread.
    pub recap: Option<String>,
    /// Transcript items the user pinned.
    pub pinned_items: Vec<ItemId>,
    /// Provider account (named environment from provider settings).
    pub account: Option<String>,
    /// Conversation context to send ahead of the first prompt, for forks
    /// that cannot branch the provider's own session.
    pub fork_context: Option<String>,
    /// Objective the thread keeps working toward across turns.
    pub goal: Option<String>,
    pub goal_status: Option<crate::orchestration::GoalStatus>,
    /// Automatic continuations spent on the goal.
    pub goal_runs: u32,
    /// Reproduce-first debugging mode.
    pub debug_mode: bool,
    /// Spending limit in US dollars: at it, automatic turns (goals) stop.
    pub budget_usd: Option<f64>,
    /// Set on the candidates of a best-of-N run: the same task given to
    /// several agents, each in its own worktree.
    pub race_id: Option<Uuid>,
    /// The Félagi issue the thread works on (`ACM-231`).
    pub felagi_issue: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Thread {
    /// The agent produced something the user has not seen yet.
    pub fn is_unread(&self) -> bool {
        self.last_activity_at
            .is_some_and(|activity| self.read_at.is_none_or(|read| activity > read))
    }

    /// The directory the provider, terminal and Git views operate in.
    pub fn working_dir(&self, project: &Project) -> PathBuf {
        match &self.environment {
            Environment::Local => project.path.clone(),
            Environment::Worktree { path, .. } => path.clone(),
        }
    }
}

/// What became of a proposal in the transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalOutcome {
    Created,
    Dismissed,
}

/// Where a proposed rule went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleOutcome {
    /// Added to the repository's AGENTS.md (or CLAUDE.md).
    File,
    /// Added to the project's instructions in Elyra.
    Instructions,
    Dismissed,
}

/// One durable entry in a thread transcript.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemContent {
    User {
        text: String,
        /// Working-tree snapshot taken before this message was sent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint: Option<String>,
    },
    Assistant {
        text: String,
        /// Set when a subagent produced the text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    Thinking {
        text: String,
    },
    ToolUse {
        tool_use_id: String,
        name: String,
        input: serde_json::Value,
        /// Set when the call was made by a subagent (Task/Agent tool).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    Approval {
        request_id: String,
        tool_name: String,
        description: Option<String>,
        input: serde_json::Value,
        decision: Option<ApprovalDecision>,
    },
    Question {
        request: QuestionRequest,
        answer: Option<QuestionAnswer>,
    },
    TurnSummary {
        duration_ms: Option<u64>,
        cost_usd: Option<f64>,
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_window: Option<u64>,
    },
    Notice {
        text: String,
        is_error: bool,
    },
    /// The page in the thread's browser before and after a turn (JPEG files).
    PageSnapshot {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        after: String,
    },
    /// An automation the agent proposed; the user creates it (perhaps after
    /// editing) or dismisses it.
    AutomationProposal {
        automation: Box<crate::orchestration::Automation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<ProposalOutcome>,
    },
    /// A rule the agent learned from the user's correction, for the user to
    /// keep (in AGENTS.md or the project's instructions) or dismiss.
    RuleProposal {
        rule: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<RuleOutcome>,
    },
    /// A pull request the thread reviewed changed: new commits since the
    /// review, or a review asked for again.
    PrUpdate {
        /// `owner/name`.
        repo: String,
        number: u64,
        title: String,
        url: String,
        /// Commits pushed since the reviewed one.
        commits: usize,
        /// A review is asked for again.
        requested: bool,
        /// The reviewed commit and the new head.
        from: String,
        to: String,
        /// The user acted on it (reviewed the changes or stopped following).
        #[serde(default)]
        handled: bool,
    },
    /// A saved browser journey, replayed (after a turn, or on request).
    Journey {
        name: String,
        passed: bool,
        /// Steps that ran before it stopped (all of them when it passed).
        steps_run: usize,
        total_steps: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<String>,
        duration_ms: u64,
    },
    /// The project's checks, run after a turn that changed files.
    Check {
        command: String,
        passed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        duration_ms: u64,
        /// The end of the output.
        output: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Allowed,
    AllowedForSession,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranscriptItem {
    pub id: ItemId,
    pub thread_id: ThreadId,
    pub seq: i64,
    pub content: ItemContent,
    pub created_at: DateTime<Utc>,
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

/// A turn that wrote lines: the thread and its user message, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct LineOrigin {
    pub thread: ThreadId,
    pub item: ItemId,
    pub at: DateTime<Utc>,
    /// How many of the matched lines it wrote.
    pub lines: usize,
}

/// A line's content, for finding where it came from: whitespace around it
/// doesn't count, and lines with fewer than four visible characters (braces,
/// blank lines) aren't tracked, as they would match everywhere.
pub fn line_hash(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.chars().filter(|c| !c.is_whitespace()).count() < 4 {
        return None;
    }
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    Some(hash as i64)
}
