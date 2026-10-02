use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use elyra_core::{Question, QuestionAnswer, QuestionKind, QuestionOption, QuestionRequest};

/// Provider-neutral events emitted by a running session. Adapters translate
/// their wire protocol into these; the app never sees provider JSON.
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderEvent {
    /// The provider created or resumed a session. `session_id` is whatever
    /// the provider needs to resume later (a session id or a session file).
    SessionStarted {
        session_id: String,
        model: Option<String>,
    },
    /// Incremental assistant text for the in-progress message.
    TextDelta(String),
    /// Incremental reasoning text for the in-progress message.
    ThinkingDelta(String),
    /// A completed assistant text block.
    AssistantText {
        text: String,
        parent_tool_use_id: Option<String>,
    },
    /// A completed reasoning block (may be empty when the provider redacts it).
    Thinking {
        text: String,
    },
    ToolUse {
        tool_use_id: String,
        name: String,
        input: Value,
        parent_tool_use_id: Option<String>,
    },
    /// Accumulated output of a still-running tool (replaces earlier progress).
    ToolProgress {
        tool_use_id: String,
        output: String,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    PermissionRequest(PermissionRequest),
    Question(QuestionRequest),
    /// Token usage and context window after an assistant message or turn.
    Usage(Usage),
    TurnCompleted {
        is_error: bool,
        duration_ms: Option<u64>,
        cost_usd: Option<f64>,
        message: Option<String>,
    },
    /// Slash commands the provider accepts (skills, prompts, extensions).
    Commands(Vec<SlashCommand>),
    /// Models the provider offers, discovered at runtime.
    Models(Vec<ModelOption>),
    /// Messages the provider has queued but not yet delivered.
    QueueChanged(Vec<String>),
    Compacting(bool),
    /// The provider is retrying after a transient error.
    Retrying {
        attempt: u32,
        max_attempts: u32,
        error: String,
    },
    RateLimited {
        resets_at: Option<i64>,
    },
    /// Informational message from the provider or one of its extensions.
    Notice {
        text: String,
        is_error: bool,
    },
    /// Non-fatal diagnostic from the provider process.
    Warning(String),
    /// The provider process ended. `error` carries the reason when unexpected.
    Exited {
        error: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PermissionRequest {
    pub request_id: String,
    pub tool_name: String,
    pub tool_use_id: Option<String>,
    pub description: Option<String>,
    pub input: Value,
    /// Raw provider suggestions to apply for "always allow this session".
    pub suggestions: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PermissionResponse {
    Allow,
    AllowForSession,
    Deny { message: String },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Cumulative session cost when the provider reports it.
    pub cost_usd: Option<f64>,
    /// Tokens currently occupying the context window.
    pub context_tokens: Option<u64>,
    pub context_window: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlashCommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub argument_hint: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelOption {
    /// Value passed back to the provider ("" = provider default).
    pub id: String,
    pub label: String,
}

/// An image or file sent with a prompt.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageAttachment {
    pub media_type: String,
    /// Base64-encoded bytes.
    pub data: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prompt {
    pub text: String,
    pub images: Vec<ImageAttachment>,
}

impl Prompt {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            images: Vec::new(),
        }
    }
}
