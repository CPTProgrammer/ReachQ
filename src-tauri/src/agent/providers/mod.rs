//! Provider preset trait, unified message/event model, model metadata,
//! and the shared SSE streaming driver (design 02 §5, 05 §1).
//!
//! All three presets speak OpenAI Chat Completions wire protocol with
//! vendor-specific extensions (thinking / reasoning_effort /
//! reasoning_content / cached tokens). Each preset keeps its own
//! request/response types; they converge on `ChatEvent` going out.

pub mod deepseek;
pub mod kimi;
pub mod openrouter;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::config::ProviderInstance;
use super::types::Usage;

// ---------------------------------------------------------------------------
// Unified request model (design 02 §5)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// Model id (without the "{instanceId}/" prefix).
    pub model: String,
    /// Capabilities of the selected model (from the instance's model list),
    /// needed to build provider-specific thinking fields.
    pub model_meta: Option<ModelMeta>,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSchema>,
    pub thinking: Option<ThinkingConfig>,
    /// Kimi: thread id, improves prompt cache hits.
    pub prompt_cache_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
    pub enabled: bool,
    #[serde(default)]
    pub effort: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Provider-neutral conversation model. Serializes to each vendor's wire
/// format in their preset module.
#[derive(Debug, Clone)]
pub enum ChatMessage {
    System(String),
    User(String),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
}

#[derive(Debug, Clone, Default)]
pub struct AssistantMessage {
    pub content: String,
    /// Accumulated reasoning text, sent back as `reasoning_content`
    /// (DeepSeek/Kimi preserved-thinking round trip).
    pub reasoning_content: Option<String>,
    /// Opaque reasoning_details array, round-tripped verbatim (OpenRouter;
    /// dropped for anthropic/* models which treat it as output-only).
    pub reasoning_details: Option<serde_json::Value>,
    pub tool_calls: Vec<AssistantToolCall>,
}

#[derive(Debug, Clone)]
pub struct AssistantToolCall {
    pub id: String,
    pub name: String,
    pub arguments_json: String,
}

#[derive(Debug, Clone)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub content: String,
    /// UI-only flag; OpenAI-compatible wire format has no is_error field.
    #[allow(dead_code)]
    pub is_error: bool,
}

// ---------------------------------------------------------------------------
// Unified stream events (design 02 §5)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ChatEvent {
    TextDelta(String),
    ThinkingDelta(String),
    /// OpenRouter reasoning_details array (accumulated, round-tripped).
    ReasoningDetails(serde_json::Value),
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        args_json_delta: String,
    },
    Usage(Usage),
    Done {
        finish_reason: String,
    },
    Error(String),
}

// ---------------------------------------------------------------------------
// Model metadata (design 05 §1)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMeta {
    pub id: String,
    pub display_name: String,
    pub context_length: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub supports_thinking: bool,
    /// true = thinking is forced on (frontend locks the toggle on).
    #[serde(default)]
    pub thinking_mandatory: bool,
    /// Supported reasoning effort levels (empty = on/off only).
    #[serde(default)]
    pub thinking_efforts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
    #[serde(default = "default_true")]
    pub supports_tools: bool,
    /// $/1M tokens (prompt, completion); only OpenRouter provides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<(f64, f64)>,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Preset trait & registry
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error {status}: {message}")]
    Api { status: u16, message: String },
    #[error("Stream error: {0}")]
    Stream(String),
    #[error("Cancelled")]
    Cancelled,
}

#[async_trait]
pub trait ProviderPreset: Send + Sync {
    fn preset_id(&self) -> &'static str;
    /// Preset display name; basis for default instance naming.
    fn display_name(&self) -> &'static str;
    fn default_base_url(&self) -> &'static str;

    async fn list_models(
        &self,
        instance: &ProviderInstance,
        api_key: &str,
    ) -> Result<Vec<ModelMeta>, ProviderError>;

    async fn chat_stream(
        &self,
        instance: &ProviderInstance,
        api_key: &str,
        req: ChatRequest,
        event_tx: mpsc::Sender<ChatEvent>,
        cancel: CancellationToken,
    ) -> Result<(), ProviderError>;
}

/// The single registration site: add a preset here and `preset_by_id` /
/// `known_presets` / `preset_default_base_url` / `preset_display_name` all
/// pick it up.
fn presets() -> Vec<std::sync::Arc<dyn ProviderPreset>> {
    vec![
        std::sync::Arc::new(openrouter::OpenRouterPreset),
        std::sync::Arc::new(deepseek::DeepSeekPreset),
        std::sync::Arc::new(kimi::KimiPreset),
    ]
}

pub fn preset_by_id(id: &str) -> Option<std::sync::Arc<dyn ProviderPreset>> {
    presets().into_iter().find(|p| p.preset_id() == id)
}

pub fn preset_default_base_url(id: &str) -> Option<&'static str> {
    preset_by_id(id).map(|p| p.default_base_url())
}

pub fn preset_display_name(id: &str) -> Option<&'static str> {
    preset_by_id(id).map(|p| p.display_name())
}

pub fn known_presets() -> Vec<(&'static str, &'static str)> {
    presets()
        .into_iter()
        .map(|p| (p.preset_id(), p.display_name()))
        .collect()
}

// ---------------------------------------------------------------------------
// Shared SSE machinery
// ---------------------------------------------------------------------------

/// Lenient OpenAI-style stream chunk: every field optional because the
/// three vendors differ in what they send (esp. the trailing usage chunk).
#[derive(Debug, Deserialize, Default)]
pub struct StreamChunk {
    #[serde(default)]
    pub choices: Vec<StreamChoice>,
    #[serde(default)]
    pub usage: Option<StreamUsage>,
}

#[derive(Debug, Deserialize, Default)]
pub struct StreamChoice {
    #[serde(default)]
    pub delta: StreamDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct StreamDelta {
    #[serde(default)]
    pub content: Option<String>,
    /// OpenRouter thinking text delta.
    #[serde(default)]
    pub reasoning: Option<String>,
    /// DeepSeek/Kimi thinking text delta.
    #[serde(default)]
    pub reasoning_content: Option<String>,
    /// OpenRouter opaque reasoning_details array (round-tripped).
    #[serde(default)]
    pub reasoning_details: Option<serde_json::Value>,
    #[serde(default)]
    pub tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
pub struct StreamToolCallDelta {
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<StreamToolFunctionDelta>,
}

#[derive(Debug, Deserialize, Default)]
pub struct StreamToolFunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct StreamUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    /// DeepSeek: prompt_cache_hit_tokens.
    #[serde(default)]
    pub prompt_cache_hit_tokens: Option<u64>,
    /// Kimi: top-level Moonshot-style cached_tokens.
    #[serde(default)]
    pub cached_tokens: Option<u64>,
    /// Kimi/OpenAI-style nested cached tokens.
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
}

impl StreamUsage {
    pub fn into_usage(self) -> Usage {
        let cached = self
            .prompt_cache_hit_tokens
            .or(self.cached_tokens)
            .or(self
                .prompt_tokens_details
                .and_then(|d| d.cached_tokens));
        Usage {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            cached_tokens: cached,
        }
    }
}

/// Map one lenient chunk to unified events. Order: usage first (usage-only
/// chunks have empty choices), then reasoning, text, tool calls, done.
/// Returns events; `Done` is produced from finish_reason.
pub fn map_chunk(chunk: StreamChunk) -> Vec<ChatEvent> {
    let mut out = Vec::new();
    if let Some(usage) = chunk.usage {
        out.push(ChatEvent::Usage(usage.into_usage()));
    }
    for choice in chunk.choices {
        let d = choice.delta;
        if let Some(details) = d.reasoning_details {
            if !details.as_array().map(|a| a.is_empty()).unwrap_or(false) {
                out.push(ChatEvent::ReasoningDetails(details));
            }
        }
        if let Some(r) = d.reasoning {
            if !r.is_empty() {
                out.push(ChatEvent::ThinkingDelta(r));
            }
        }
        if let Some(r) = d.reasoning_content {
            // Empty strings still matter for Kimi preserved-thinking on the
            // wire, but as a UI delta they are noise.
            if !r.is_empty() {
                out.push(ChatEvent::ThinkingDelta(r));
            }
        }
        if let Some(c) = d.content {
            if !c.is_empty() {
                out.push(ChatEvent::TextDelta(c));
            }
        }
        if let Some(calls) = d.tool_calls {
            for tc in calls {
                let (name, args) = match tc.function {
                    Some(f) => (f.name, f.arguments),
                    None => (None, None),
                };
                out.push(ChatEvent::ToolCallDelta {
                    index: tc.index,
                    id: tc.id,
                    name,
                    args_json_delta: args.unwrap_or_default(),
                });
            }
        }
        if let Some(reason) = choice.finish_reason {
            if !reason.is_empty() {
                out.push(ChatEvent::Done {
                    finish_reason: reason,
                });
            }
        }
    }
    out
}

/// Drive an SSE response to completion, mapping chunks with `map_chunk`.
/// Emits events via `event_tx`; honors cancellation.
pub async fn drive_sse(
    response: reqwest::Response,
    event_tx: &mpsc::Sender<ChatEvent>,
    cancel: &CancellationToken,
) -> Result<(), ProviderError> {
    use eventsource_stream::Eventsource;
    use futures::StreamExt;

    let byte_stream = response.bytes_stream();
    let mut events = byte_stream.eventsource();

    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
            item = events.next() => item,
        };
        let Some(item) = next else { break };
        match item {
            Ok(event) => {
                let data = event.data;
                if data.trim() == "[DONE]" {
                    break;
                }
                match serde_json::from_str::<StreamChunk>(&data) {
                    Ok(chunk) => {
                        for ev in map_chunk(chunk) {
                            if event_tx.send(ev).await.is_err() {
                                return Ok(()); // receiver gone (run dropped)
                            }
                        }
                    }
                    Err(e) => {
                        // Tolerate blank payloads; surface real drift.
                        if !data.trim().is_empty() {
                            tracing::warn!("agent: failed to parse SSE chunk: {} — {:.200}", e, data);
                        }
                    }
                }
            }
            Err(e) => {
                return Err(ProviderError::Stream(format!("SSE stream error: {e}")));
            }
        }
    }
    Ok(())
}

/// Build an HTTP client for provider calls (no global timeout: streams can
/// run long; the loop enforces cancellation).
pub fn http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()
}

/// Extract a human-readable message from an error response body, trying
/// the structured `{error: {message}}` shape first.
pub async fn error_message(response: reqwest::Response) -> (u16, String) {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    #[derive(Deserialize)]
    struct ErrEnvelope {
        error: Option<ErrBody>,
    }
    #[derive(Deserialize)]
    struct ErrBody {
        message: Option<String>,
    }
    if let Ok(env) = serde_json::from_str::<ErrEnvelope>(&body) {
        if let Some(msg) = env.error.and_then(|e| e.message) {
            return (status, msg);
        }
    }
    (status, body.chars().take(500).collect())
}
