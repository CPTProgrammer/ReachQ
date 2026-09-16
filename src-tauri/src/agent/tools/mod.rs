//! Tool trait, registry, execution context, and the reverse-registered
//! settings schema (design 03 §9, 04 §4.3).

// TODO(agent-tools): uncomment as each tool module lands.
pub mod edit_file;
pub mod edit_match;
pub mod fetch;
pub mod list_directory;
pub mod read_file;
pub mod terminal;
pub mod write_file;

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::events::ToolCallView;
use super::providers::ToolSchema;
use super::remote_fs::{PendingWrites, ReadCache};
use super::types::{SshIdentity, ToolResult};
use crate::sftp::backend::SftpBackendManager;
use crate::ssh::client::SshManager;

/// Everything a tool needs at execution time.
pub struct ToolContext {
    pub app: tauri::AppHandle,
    pub identity: SshIdentity,
    pub thread_id: String,
    pub tool_call_id: String,
    pub ssh_manager: Arc<tokio::sync::Mutex<SshManager>>,
    pub sftp_backends: Arc<tokio::sync::Mutex<SftpBackendManager>>,
    /// Resolved tool options (defaults merged with user config).
    pub options: serde_json::Map<String, Value>,
    /// In-memory read cache shared across the identity (design 03 §1.7).
    pub read_cache: ReadCache,
    /// Prepared writes stashed between approval and execution, keyed by
    /// tool_call_id (the approved diff is exactly what gets written).
    pub pending_writes: PendingWrites,
    /// Live terminal sessions for resize/stop (terminal tool).
    pub terminals: super::SharedTerminals,
}

impl ToolContext {
    /// Resolve a live connection for this identity (preferring `hint`),
    /// acquire a tool-call lease, and build a RemoteFs. On connection-level
    /// failure, re-resolves once against another connection of the same
    /// identity (design 02 §3.1).
    pub async fn remote_fs(
        &self,
        connection_hint: Option<&str>,
    ) -> Result<super::remote_fs::AgentRemoteFs, ToolResult> {
        super::remote_fs::AgentRemoteFs::for_identity(
            &self.ssh_manager,
            &self.sftp_backends,
            self.app.clone(),
            &self.identity,
            connection_hint,
        )
        .await
        .map_err(|e| super::remote_fs::connection_lost_result(&self.identity, &e))
    }
}

/// One configurable tool option, reverse-registered into the settings page
/// (design 04 §4.3).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOptionSpec {
    pub key: String,
    pub description: String,
    #[serde(flatten)]
    pub kind: ToolOptionKind,
    pub default: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolOptionKind {
    String,
    Number { min: Option<f64>, max: Option<f64> },
    Boolean,
    StringList,
    Enum { values: Vec<String> },
}

/// Tool metadata + settings schema sent to the frontend settings page
/// (design 04 §4.3 `agent_tool_schemas`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub default_requires_approval: bool,
    pub options: Vec<ToolOptionSpec>,
}

/// Outcome of pre-approval preparation: either show the approval card
/// (with or without a detail payload), or finish the tool call immediately
/// without consuming a user approval (validation errors, no-op edits).
pub enum ApprovalPrep {
    Proceed(Option<Value>),
    ShortCircuit(ToolResult),
}

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &'static str;
    fn default_requires_approval(&self) -> bool;

    /// JSON schema + description sent to the model.
    fn schema(&self) -> ToolSchema;

    /// Reverse-registered options (design 04 §4.3).
    fn settings_schema(&self) -> Vec<ToolOptionSpec> {
        vec![]
    }

    /// Short human-facing title content for the card header (path, command,
    /// URL). Used by approval requests and the tool_call view.
    fn title(&self, args: &Value) -> String;

    /// Pre-approval preparation: compute the approval-card detail payload
    /// (diff text, etc), or short-circuit the call without asking the user
    /// (validation errors, no-op edits). Default: proceed without a payload.
    async fn approval_payload(
        &self,
        _args: &Value,
        _ctx: &ToolContext,
    ) -> ApprovalPrep {
        ApprovalPrep::Proceed(None)
    }

    /// Approval warning lines (dangerous command keywords, sensitive
    /// pattern hits). Default: none.
    fn approval_warnings(
        &self,
        _args: &Value,
        _ctx: &ToolContext,
    ) -> Vec<super::events::ApprovalWarning> {
        vec![]
    }

    /// Execute. Errors are content: `Err(ToolResult)` still goes back to
    /// the model with `is_error: true`.
    async fn run(
        &self,
        args: Value,
        ctx: &ToolContext,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolResult>;
}

pub fn all_tools() -> Vec<Arc<dyn AgentTool>> {
    vec![
        Arc::new(read_file::ReadFileTool),
        Arc::new(write_file::WriteFileTool),
        Arc::new(edit_file::EditFileTool),
        Arc::new(list_directory::ListDirectoryTool),
        Arc::new(terminal::TerminalTool),
        Arc::new(fetch::FetchTool),
    ]
}

pub fn tool_by_name(name: &str) -> Option<Arc<dyn AgentTool>> {
    all_tools().into_iter().find(|t| t.name() == name)
}

pub fn tool_descriptors() -> Vec<ToolDescriptor> {
    all_tools()
        .into_iter()
        .map(|t| ToolDescriptor {
            name: t.name().to_string(),
            description: t.schema().description,
            default_requires_approval: t.default_requires_approval(),
            options: t.settings_schema(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Small shared helpers
// ---------------------------------------------------------------------------

pub fn ok_text(text: impl Into<String>) -> ToolResult {
    ToolResult {
        llm_text: text.into(),
        is_error: false,
        ui_payload: None,
    }
}

pub fn err_text(text: impl Into<String>) -> ToolResult {
    ToolResult {
        llm_text: text.into(),
        is_error: true,
        ui_payload: None,
    }
}

/// Zed-style tolerant deserialization: accept both a structured value and a
/// JSON-stringified one (design 03 §3 edge cases).
pub fn deserialize_maybe_stringified<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: serde::de::DeserializeOwned,
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;

    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum ValueOrJsonString<T> {
        Value(T),
        String(String),
    }

    match ValueOrJsonString::<T>::deserialize(deserializer)? {
        ValueOrJsonString::Value(value) => Ok(value),
        ValueOrJsonString::String(string) => serde_json::from_str::<T>(&string).map_err(|e| {
            serde::de::Error::custom(format!("failed to parse stringified value: {e}"))
        }),
    }
}

/// Build the streaming-state card view for a tool call.
pub fn streaming_view(message_id: &str, id: &str, name: &str, args_json: &str) -> ToolCallView {
    ToolCallView {
        id: id.to_string(),
        message_id: message_id.to_string(),
        name: name.to_string(),
        args_json: args_json.to_string(),
        status: super::types::ToolCallStatus::Streaming,
        result: None,
        warnings: None,
    }
}
