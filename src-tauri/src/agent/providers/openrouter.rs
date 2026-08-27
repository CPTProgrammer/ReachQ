//! OpenRouter preset (design 05 §2.1).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    drive_sse, error_message, http_client, ChatMessage, ChatRequest, ModelMeta, ProviderError,
    ProviderPreset, ThinkingConfig, ToolSchema,
};
use crate::agent::config::ProviderInstance;

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";

pub struct OpenRouterPreset;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    stream: bool,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ToolDef<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<serde_json::Value>,
    usage: RequestUsage,
}

#[derive(Serialize)]
struct RequestUsage {
    include: bool,
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
    System { content: &'a str },
    User { content: &'a str },
    Assistant {
        content: Option<&'a str>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<AssistantToolCallWire<'a>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_details: Option<&'a serde_json::Value>,
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
    let drop_reasoning = req.model.starts_with("anthropic/");
    let mut out = Vec::new();
    for msg in &req.messages {
        match msg {
            ChatMessage::System(text) => out.push(Message::System { content: text }),
            ChatMessage::User(text) => out.push(Message::User { content: text }),
            ChatMessage::Assistant(a) => out.push(Message::Assistant {
                content: if a.content.is_empty() && !a.tool_calls.is_empty() {
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
                reasoning_details: if drop_reasoning {
                    None
                } else {
                    a.reasoning_details.as_ref()
                },
            }),
            ChatMessage::ToolResult(t) => out.push(Message::Tool {
                content: &t.content,
                tool_call_id: &t.tool_call_id,
            }),
        }
    }
    out
}

/// Design 05 §2.1: effort passes straight through; mandatory models always
/// carry reasoning; disabled thinking sends {"enabled": false}.
fn resolve_reasoning(
    thinking: Option<&ThinkingConfig>,
    meta: Option<&ModelMeta>,
) -> Option<serde_json::Value> {
    let mandatory = meta.map(|m| m.thinking_mandatory).unwrap_or(false);
    let supports = meta.map(|m| m.supports_thinking).unwrap_or(false);
    if !mandatory && !supports {
        return None;
    }
    let enabled = mandatory || thinking.map(|t| t.enabled).unwrap_or(false);
    if !enabled {
        return Some(json!({ "enabled": false }));
    }
    let effort = thinking
        .and_then(|t| t.effort.clone())
        .or_else(|| meta.and_then(|m| m.default_effort.clone()));
    match effort {
        Some(e) => Some(json!({ "effort": e })),
        None => Some(json!({ "enabled": true })),
    }
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
    name: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    max_completion_tokens: Option<u64>,
    #[serde(default)]
    supported_parameters: Option<Vec<String>>,
    #[serde(default)]
    pricing: Option<PricingEntry>,
    /// Undocumented but observed: {mandatory, default_enabled,
    /// supported_efforts, default_effort}.
    #[serde(default)]
    reasoning: Option<ReasoningEntry>,
}

#[derive(Deserialize)]
struct PricingEntry {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

#[derive(Deserialize, Default)]
struct ReasoningEntry {
    #[serde(default)]
    mandatory: Option<bool>,
    #[serde(default)]
    supported_efforts: Option<Vec<String>>,
    #[serde(default)]
    default_effort: Option<String>,
}

fn parse_price(s: Option<&str>) -> Option<f64> {
    s.and_then(|v| v.parse::<f64>().ok()).map(|v| v * 1_000_000.0)
}

#[async_trait]
impl ProviderPreset for OpenRouterPreset {
    fn preset_id(&self) -> &'static str {
        "openrouter"
    }

    fn display_name(&self) -> &'static str {
        "OpenRouter"
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
            .header("Authorization", format!("Bearer {}", api_key))
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
                let params = m.supported_parameters.unwrap_or_default();
                let reasoning = m.reasoning.unwrap_or_default();
                let efforts = reasoning.supported_efforts.unwrap_or_default();
                let supports_thinking = reasoning.mandatory.unwrap_or(false)
                    || !efforts.is_empty()
                    || params.iter().any(|p| p == "reasoning");
                let display = m
                    .name
                    .as_deref()
                    .and_then(|n| n.split(':').next_back())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| m.id.clone());
                ModelMeta {
                    id: m.id,
                    display_name: display,
                    context_length: m.context_length.unwrap_or(0),
                    max_output_tokens: m.max_completion_tokens,
                    supports_thinking,
                    thinking_mandatory: reasoning.mandatory.unwrap_or(false),
                    thinking_efforts: efforts,
                    default_effort: reasoning.default_effort,
                    supports_tools: params.iter().any(|p| p == "tools"),
                    pricing: match &m.pricing {
                        Some(p) => match (
                            parse_price(p.prompt.as_deref()),
                            parse_price(p.completion.as_deref()),
                        ) {
                            (Some(prompt), Some(completion)) => Some((prompt, completion)),
                            _ => None,
                        },
                        None => None,
                    },
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
        let messages = build_messages(&req);
        let reasoning = resolve_reasoning(req.thinking.as_ref(), req.model_meta.as_ref());
        let body = Request {
            model: &req.model,
            messages,
            stream: true,
            temperature: 0.4,
            max_tokens: req.model_meta.as_ref().and_then(|m| m.max_output_tokens),
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
            reasoning,
            usage: RequestUsage { include: true },
        };

        let client = http_client()?;
        let url = format!("{}/chat/completions", instance.resolved_base_url());
        let res = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
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
