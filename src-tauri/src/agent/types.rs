//! Shared types for the agent subsystem: messages stored in the thread
//! tree, view objects sent to the frontend, usage, and tool call state.

use serde::{Deserialize, Serialize};

/// Normalized SSH identity: "user@host:port[#via=chainhash]".
/// See design 01 §1.1 for normalization rules.
pub type SshIdentity = String;

/// Token usage reported by a provider at the end of a stream round.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Provider-reported cache hits (DeepSeek prompt_cache_hit_tokens, etc).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
}

/// Per-assistant-message metadata shown in the hover card (design 01 §2.2).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_instance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_count: Option<u32>,
}

/// Runtime/final state of a tool call (design 01 §2.3 state machine).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    /// Arguments still streaming in.
    Streaming,
    /// Waiting for the user's Accept/Reject decision.
    PendingApproval,
    Running,
    Success,
    Failed,
    Rejected,
    Cancelled,
}

impl ToolCallStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Success | Self::Failed | Self::Rejected | Self::Cancelled
        )
    }
}

/// The result of executing a tool. `llm_text` goes back to the model;
/// `ui_payload` (diff text, full terminal output, ...) is UI-only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub llm_text: String,
    pub is_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_payload: Option<serde_json::Value>,
}

/// A block inside a stored message's `content` JSON array.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        text: String,
        /// Provider-opaque reasoning payload needed for history round-trip
        /// (OpenRouter reasoning_details). Serialized verbatim back to the API.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_payload: Option<serde_json::Value>,
    },
    ToolCall {
        id: String,
        name: String,
        /// Full arguments JSON (accumulated from streaming deltas).
        args: serde_json::Value,
        status: ToolCallStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<ToolResult>,
        /// Approval warnings, filled only by snapshot enrichment for live
        /// pending approvals (AgentState::enrich_snapshot); never persisted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        warnings: Option<Vec<String>>,
    },
}

/// A message node in the thread tree (design 02 §4).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredMessage {
    pub id: String,
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub branch_index: i64,
    pub seq: i64,
    pub role: String, // "user" | "assistant"
    pub content: Vec<ContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MessageMetadata>,
    pub created_at: i64,
}

/// Branch navigation info for one node on the active path (`< 2 / 4 >`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    /// 1-based position among siblings (for display: "2 / 4").
    pub index: i64,
    pub count: i64,
}

/// A message on the active path, annotated with its sibling branch info.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathMessage {
    #[serde(flatten)]
    pub message: StoredMessage,
    pub branch: BranchInfo,
}

/// Summary row for the threads list (design 01 §2.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    pub identity: SshIdentity,
    pub title: String,
    pub archived: bool,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_usage: Option<Usage>,
    /// JSON triple snapshot { model, thinking, effort } (design 05 §5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<serde_json::Value>,
}

/// Full thread snapshot: active path + branch info (design 02 §4).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSnapshot {
    pub thread: ThreadSummary,
    pub messages: Vec<PathMessage>,
}

/// Model selection triple snapshot (composer state, design 05 §5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSelection {
    /// "{instanceId}/{modelId}"
    pub model: String,
    #[serde(default)]
    pub thinking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}
