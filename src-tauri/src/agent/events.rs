//! Streaming events emitted to the frontend over Tauri events.
//! Channel name: `agent-event-{identity}` (design 01 §5).

use serde::{Deserialize, Serialize};

use super::types::{ContentBlock, ToolCallStatus, Usage};

/// A tool call as rendered by the frontend ToolCallCard.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallView {
    pub id: String,
    pub message_id: String,
    pub name: String,
    /// Accumulated arguments JSON so far (text; may be partial while streaming).
    pub args_json: String,
    pub status: ToolCallStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<super::types::ToolResult>,
    /// Approval warning info (e.g. dangerous keyword hits for terminal,
    /// matched sensitive pattern for read_file). Only set when relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
}

/// Approval request payload (design 04 §2). Rendered as the bottom bar of
/// the ToolCallCard; carries everything the card needs to render the
/// pending state (diff, command, path, ...).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub tool_call_id: String,
    pub tool: String,
    /// Human-facing title content (path / command / URL).
    pub title: String,
    /// Tool-specific payload for the detail area (diff text, etc).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    #[serde(rename_all = "camelCase")]
    TextDelta {
        thread_id: String,
        message_id: String,
        delta: String,
    },
    #[serde(rename_all = "camelCase")]
    ThinkingDelta {
        thread_id: String,
        message_id: String,
        delta: String,
    },
    /// A tool call was added or migrated state (design 01 §5).
    #[serde(rename_all = "camelCase")]
    ToolCall { thread_id: String, tool_call: ToolCallView },
    /// Streaming arguments increment for a tool call still being generated.
    #[serde(rename_all = "camelCase")]
    ToolCallArgsDelta {
        thread_id: String,
        tool_call_id: String,
        args_json_delta: String,
    },
    #[serde(rename_all = "camelCase")]
    ApprovalNeeded {
        thread_id: String,
        approval: ApprovalRequest,
    },
    /// A user message was appended (initial send and queued-message
    /// injection); the frontend appends the bubble from this event.
    #[serde(rename_all = "camelCase")]
    UserMessage {
        thread_id: String,
        message: super::types::StoredMessage,
    },
    #[serde(rename_all = "camelCase")]
    Usage { thread_id: String, usage: Usage },
    #[serde(rename_all = "camelCase")]
    MessageDone { thread_id: String, message_id: String },
    #[serde(rename_all = "camelCase")]
    Error { thread_id: String, message: String },
    #[serde(rename_all = "camelCase")]
    Cancelled { thread_id: String },
    /// Raw PTY byte stream of a running terminal tool call, for the card's
    /// embedded xterm view (design 01 §2.3). Base64-encoded bytes.
    #[serde(rename_all = "camelCase")]
    TerminalOutput { tool_call_id: String, data_b64: String },
    /// Auto-generated thread title is ready (design 01 §2.1).
    #[serde(rename_all = "camelCase")]
    TitleUpdated { thread_id: String, title: String },
    /// Thread metadata changed (model snapshot, archived, etc.) so lists
    /// can refresh.
    #[serde(rename_all = "camelCase")]
    ThreadUpdated { thread_id: String },
}

impl AgentEvent {
    /// The event channel name for one identity.
    pub fn channel(identity: &str) -> String {
        format!("agent-event-{}", identity)
    }
}

/// A single content block update pushed when a run is interrupted
/// mid-stream and the partial message is persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialMessage {
    pub message_id: String,
    pub content: Vec<ContentBlock>,
}
