//! Kimi Code preset (design 05 §2.3). API key only, no OAuth.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    drive_sse, error_message, http_client, ChatMessage, ChatRequest, ModelMeta, ProviderError,
    ProviderPreset, ToolSchema,
};
use crate::agent::config::ProviderInstance;

const DEFAULT_BASE_URL: &str = "https://api.kimi.com/coding/v1";

pub struct KimiPreset;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    stream: bool,
    /// Required, or Kimi sends no usage chunk at the end of the stream.
    stream_options: StreamOptions,
    /// Note: max_completion_tokens, not max_tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u64>,
    /// Dropped entirely when thinking is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    /// Thread id; improves prompt cache hits.
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Thinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ToolDef<'a>>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct Thinking {
    #[serde(rename = "type")]
    kind: &'static str, // "enabled" | "disabled"
}

#[derive(Serialize)]
struct ToolDef<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: ToolFunction<'a>,
}

#[derive(Serialize)]
struct ToolFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
enum Message<'a> {
    System {
        content: &'a str,
    },
    User {
        content: &'a str,
    },
    Assistant {
        /// Kimi's coding endpoint rejects whitespace-only text parts on
        /// tool-call messages; send null instead.
        content: Option<&'a str>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<AssistantToolCallWire<'a>>,
        /// Preserved thinking: kept even as an empty string.
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<&'a str>,
    },
    Tool {
        content: &'a str,
        tool_call_id: &'a str,
    },
}

#[derive(Serialize)]
struct AssistantToolCallWire<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: FunctionCallWire<'a>,
}

#[derive(Serialize)]
struct FunctionCallWire<'a> {
    name: &'a str,
    arguments: &'a str,
}

fn build_messages<'a>(req: &'a ChatRequest) -> Vec<Message<'a>> {
    req.messages
        .iter()
        .map(|msg| match msg {
            ChatMessage::System(text) => Message::System { content: text },
            ChatMessage::User(text) => Message::User { content: text },
            ChatMessage::Assistant(a) => Message::Assistant {
                content: if a.content.trim().is_empty() {
                    None
                } else {
                    Some(a.content.as_str())
                },
                tool_calls: a
                    .tool_calls
                    .iter()
                    .map(|tc| AssistantToolCallWire {
                        id: &tc.id,
                        kind: "function",
                        function: FunctionCallWire {
                            name: &tc.name,
                            arguments: &tc.arguments_json,
                        },
                    })
                    .collect(),
                reasoning_content: a.reasoning_content.as_deref(),
            },
            ChatMessage::ToolResult(t) => Message::Tool {
                content: if t.content.is_empty() {
                    "<Tool returned an empty string>"
                } else {
                    &t.content
                },
                tool_call_id: &t.tool_call_id,
            },
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Models endpoint
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    supports_reasoning: Option<bool>,
    #[serde(default)]
    think_efforts: Option<ThinkEfforts>,
}

#[derive(Deserialize, Default)]
struct ThinkEfforts {
    #[serde(default)]
    support: bool,
    #[serde(default)]
    valid_efforts: Vec<String>,
    #[serde(default)]
    default_effort: Option<String>,
}

#[async_trait]
impl ProviderPreset for KimiPreset {
    fn preset_id(&self) -> &'static str {
        "kimi"
    }

    fn display_name(&self) -> &'static str {
        "Kimi Code"
    }

    fn default_base_url(&self) -> &'static str {
        DEFAULT_BASE_URL
    }

    async fn list_models(
        &self,
        instance: &ProviderInstance,
        api_key: &str,
    ) -> Result<Vec<ModelMeta>, ProviderError> {
        let client = http_client()?;
        let url = format!("{}/models", instance.resolved_base_url());
        let res = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key.trim()))
            .header("Accept", "application/json")
            .send()
            .await?;
        if !res.status().is_success() {
            let (status, message) = error_message(res).await;
            return Err(ProviderError::Api { status, message });
        }
        let parsed: ModelsResponse = res
            .json()
            .await
            .map_err(|e| ProviderError::Stream(format!("parse models: {e}")))?;

        Ok(parsed
            .data
            .into_iter()
            .map(|m| {
                let think = m.think_efforts.unwrap_or_default();
                let (efforts, default_effort) = if think.support {
                    (think.valid_efforts, think.default_effort)
                } else {
                    (vec![], None)
                };
                // Name heuristic (design 05 §2.3): ids containing
                // "thinking"/"reason" are always-thinking models.
                let id_lower = m.id.to_lowercase();
                let thinking_mandatory =
                    id_lower.contains("thinking") || id_lower.contains("reason");
                let supports_thinking =
                    m.supports_reasoning.unwrap_or(false) || thinking_mandatory;
                ModelMeta {
                    display_name: m
                        .display_name
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| m.id.clone()),
                    id: m.id,
                    context_length: m.context_length.unwrap_or(0),
                    max_output_tokens: None,
                    supports_thinking,
                    thinking_mandatory,
                    thinking_efforts: efforts,
                    default_effort,
                    supports_tools: true,
                    pricing: None,
                }
            })
            .collect())
    }

    async fn chat_stream(
        &self,
        instance: &ProviderInstance,
        api_key: &str,
        req: ChatRequest,
        event_tx: mpsc::Sender<super::ChatEvent>,
        cancel: CancellationToken,
    ) -> Result<(), ProviderError> {
        let meta = req.model_meta.as_ref();
        let supports_thinking = meta.map(|m| m.supports_thinking).unwrap_or(false);
        let thinking_mandatory = meta.map(|m| m.thinking_mandatory).unwrap_or(false);
        let thinking_enabled = supports_thinking
            && (thinking_mandatory || req.thinking.as_ref().map(|t| t.enabled).unwrap_or(false));

        // Effort validated against the model's valid_efforts (design 05 §3:
        // invalid levels are an error surfaced to the UI).
        let reasoning_effort: Option<&str> = if thinking_enabled {
            let requested = req.thinking.as_ref().and_then(|t| t.effort.as_deref());
            match (meta, requested) {
                (Some(m), Some(e)) => {
                    if m.thinking_efforts.is_empty() || m.thinking_efforts.iter().any(|v| v == e)
                    {
                        Some(e)
                    } else {
                        return Err(ProviderError::Stream(format!(
                            "Reasoning effort \"{}\" is not supported by {} (valid: {}). Pick a valid level in the composer.",
                            e,
                            m.id,
                            m.thinking_efforts.join(", ")
                        )));
                    }
                }
                (Some(m), None) => m.default_effort.as_deref(),
                (None, e) => e,
            }
        } else {
            None
        };

        let body = Request {
            model: &req.model,
            messages: build_messages(&req),
            stream: true,
            stream_options: StreamOptions { include_usage: true },
            max_completion_tokens: meta.and_then(|m| m.max_output_tokens),
            temperature: if thinking_enabled { None } else { Some(0.4) },
            prompt_cache_key: req.prompt_cache_key.as_deref(),
            thinking: if supports_thinking {
                Some(Thinking {
                    kind: if thinking_enabled { "enabled" } else { "disabled" },
                })
            } else {
                None
            },
            reasoning_effort,
            tool_choice: if req.tools.is_empty() {
                None
            } else {
                Some("auto")
            },
            tools: req
                .tools
                .iter()
                .map(|t: &ToolSchema| ToolDef {
                    kind: "function",
                    function: ToolFunction {
                        name: &t.name,
                        description: &t.description,
                        parameters: &t.parameters,
                    },
                })
                .collect(),
        };

        let client = http_client()?;
        let url = format!("{}/chat/completions", instance.resolved_base_url());
        let res = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key.trim()))
            .json(&body)
            .send()
            .await?;
        if !res.status().is_success() {
            let (status, message) = error_message(res).await;
            return Err(ProviderError::Api { status, message });
        }
        drive_sse(res, &event_tx, &cancel).await
    }
}
