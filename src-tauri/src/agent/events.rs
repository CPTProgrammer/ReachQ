//! Streaming events emitted to the frontend over Tauri events.
//! Channel name: `agent-event-{base64url(identity)}` (design 01 §5).
//! Tauri event names only allow alphanumeric, '-', '/', ':', '_', so the
//! identity ("user@host:port[#via=hash]") is encoded as base64url without
//! padding (alphabet A-Za-z0-9-_). The frontend mirrors this encoding in
//! `onAgentEvent` (src/lib/ipc/agent.ts).

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use super::types::{ContentBlock, MessageMetadata, ToolCallStatus, Usage};

/// Structured approval warning, rendered as the yellow strip on the
/// ToolCallCard. The frontend maps `kind` to an i18n string; parameters
/// stay structured so translations control wording and order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalWarning {
    /// The target path hit the user's sensitive glob list.
    SensitivePattern { pattern: String },
    /// The terminal command contains dangerous keywords.
    DangerousKeywords { keywords: Vec<String> },
}

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
    pub warnings: Option<Vec<ApprovalWarning>>,
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
    pub warnings: Vec<ApprovalWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A new assistant round started; carries the round's message id, which
    /// the loop also uses when persisting the message. The frontend creates
    /// the streaming placeholder from this event regardless of the round's
    /// content shape (text / thinking / tool calls / empty).
    #[serde(rename_all = "camelCase")]
    MessageStart { thread_id: String, message_id: String },
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
    /// Tolerant re-parse of the in-flight arguments (trailing partial
    /// strings included); always valid JSON, safe to parse directly.
    /// Emitted per args delta while the arguments stream (deduped).
    #[serde(rename_all = "camelCase")]
    ToolCallArgsPatched {
        thread_id: String,
        tool_call_id: String,
        args_json: String,
    },
    /// Structured diff preview of a streaming write_file/edit_file call,
    /// computed with the same fuzzy-match + diff code as execution.
    #[serde(rename_all = "camelCase")]
    ToolCallPreview {
        thread_id: String,
        tool_call_id: String,
        preview: super::tools::edit_match::DiffPreview,
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
    MessageDone {
        thread_id: String,
        message_id: String,
        /// Final metadata of the finished assistant message; the frontend
        /// attaches it to the live message so the meta anchor renders
        /// without waiting for a snapshot reload.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<MessageMetadata>,
    },
    /// The run finished normally: no more rounds and the queue is empty.
    /// Terminal event of the Ok path (abnormal exits use Cancelled/Error);
    /// the frontend settles running=false on any of the three.
    #[serde(rename_all = "camelCase")]
    RunEnd { thread_id: String },
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
        format!("agent-event-{}", URL_SAFE_NO_PAD.encode(identity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_tauri_event_name(name: &str) {
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '/' | ':' | '_')),
            "invalid Tauri event name: {name}"
        );
    }

    #[test]
    fn channel_is_valid_for_nasty_identities() {
        for identity in [
            "root@131.143.215.36:35589",
            "user@example.com:22",
            "admin@host:2222#via=a1b2c3d4",
            "user@[::1]:22",
            "user_name@a_b.domain:22",
            "用户@主机:22",
        ] {
            let channel = AgentEvent::channel(identity);
            assert!(channel.starts_with("agent-event-"));
            assert_tauri_event_name(&channel);
            // Base64 padding must never leak into the name.
            assert!(!channel.contains('='));
        }
    }

    #[test]
    fn channel_does_not_collide() {
        assert_ne!(
            AgentEvent::channel("user@a.b:22"),
            AgentEvent::channel("user@a_b:22")
        );
    }

    #[test]
    fn channel_matches_frontend_encoding() {
        // Locked against the base64url helper in src/lib/ipc/agent.ts; if this
        // breaks, the two sides stopped agreeing on the channel name.
        assert_eq!(
            AgentEvent::channel("root@131.143.215.36:35589"),
            "agent-event-cm9vdEAxMzEuMTQzLjIxNS4zNjozNTU4OQ"
        );
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
