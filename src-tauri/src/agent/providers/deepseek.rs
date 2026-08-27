//! DeepSeek preset (design 05 §2.2).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    drive_sse, error_message, http_client, ChatMessage, ChatRequest, ModelMeta, ProviderError,
    ProviderPreset, ToolSchema,
};
use crate::agent::config::ProviderInstance;

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com/v1";

pub struct DeepSeekPreset;

/// Hardcoded flagship models (Zed approach, design 05 §2.2): thinking
/// capable, 1M context, efforts limited to high/max.
fn hardcoded_models() -> Vec<ModelMeta> {
    ["deepseek-v4-flash", "deepseek-v4-pro"]
        .into_iter()
        .map(|id| ModelMeta {
            id: id.to_string(),
            display_name: id
                .split('-')
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
            context_length: 1_000_000,
            max_output_tokens: Some(384_000),
            supports_thinking: true,
            thinking_mandatory: false,
            thinking_efforts: vec!["high".to_string(), "max".to_string()],
            default_effort: Some("high".to_string()),
            supports_tools: true,
            pricing: None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    /// Dropped entirely when thinking is enabled (API rejects it).
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Thinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ToolDef<'a>>,
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
    System { content: &'a str },
    User { content: &'a str },
    Assistant {
        content: Option<&'a str>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<AssistantToolCallWire<'a>>,
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
                reasoning_content: a.reasoning_content.as_deref(),
            },
            ChatMessage::ToolResult(t) => Message::Tool {
                content: &t.content,
                tool_call_id: &t.tool_call_id,
            },
        })
        .collect()
}

/// Map a unified effort to DeepSeek's high/max, nearest wins (design 05 §3:
/// "其他档位就近取").
fn map_effort(effort: Option<&str>) -> &'static str {
    match effort {
        Some("max") => "max",
        _ => "high",
    }
}

// ---------------------------------------------------------------------------
// Models endpoint (best-effort; hardcoded fallback)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

#[async_trait]
impl ProviderPreset for DeepSeekPreset {
    fn preset_id(&self) -> &'static str {
        "deepseek"
    }

    fn display_name(&self) -> &'static str {
        "DeepSeek"
    }

    fn default_base_url(&self) -> &'static str {
        DEFAULT_BASE_URL
    }

    async fn list_models(
        &self,
        instance: &ProviderInstance,
        api_key: &str,
    ) -> Result<Vec<ModelMeta>, ProviderError> {
        let mut models = hardcoded_models();

        // Best-effort: merge any extra ids the account can see.
        if let Ok(client) = http_client() {
            let url = format!("{}/models", instance.resolved_base_url());
            if let Ok(res) = client
                .get(&url)
                .header("Authorization", format!("Bearer {}", api_key.trim()))
                .send()
                .await
            {
                if res.status().is_success() {
                    if let Ok(parsed) = res.json::<ModelsResponse>().await {
                        for entry in parsed.data {
                            if !models.iter().any(|m| m.id == entry.id) {
                                models.push(ModelMeta {
                                    id: entry.id.clone(),
                                    display_name: entry.id,
                                    context_length: 0,
                                    max_output_tokens: None,
                                    supports_thinking: false,
                                    thinking_mandatory: false,
                                    thinking_efforts: vec![],
                                    default_effort: None,
                                    supports_tools: true,
                                    pricing: None,
                                });
                            }
                        }
                    }
                }
            }
        }
        Ok(models)
    }

    async fn chat_stream(
        &self,
        instance: &ProviderInstance,
        api_key: &str,
        req: ChatRequest,
        event_tx: mpsc::Sender<super::ChatEvent>,
        cancel: CancellationToken,
    ) -> Result<(), ProviderError> {
        let supports_thinking = req
            .model_meta
            .as_ref()
            .map(|m| m.supports_thinking)
            .unwrap_or(false);
        let thinking_enabled = supports_thinking
            && req.thinking.as_ref().map(|t| t.enabled).unwrap_or(false);

        let body = Request {
            model: &req.model,
            messages: build_messages(&req),
            stream: true,
            max_tokens: req.model_meta.as_ref().and_then(|m| m.max_output_tokens),
            temperature: if thinking_enabled { None } else { Some(0.4) },
            thinking: if supports_thinking {
                Some(Thinking {
                    kind: if thinking_enabled { "enabled" } else { "disabled" },
                })
            } else {
                None
            },
            reasoning_effort: if thinking_enabled {
                Some(map_effort(
                    req.thinking.as_ref().and_then(|t| t.effort.as_deref()),
                ))
            } else {
                None
            },
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
