//! The agent main loop (design 02 §2): stream provider events, execute
//! tool calls with approvals, feed results back, inject queued messages at
//! round boundaries. One active run per thread.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot, Notify};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::agent::events::{AgentEvent, ApprovalRequest, ApprovalWarning, ToolCallView};
use crate::agent::permissions::{self, ApprovalDecision};
use crate::agent::preview::StreamPreviews;
use crate::agent::providers::{
    preset_by_id, AssistantMessage, AssistantToolCall, ChatEvent, ChatMessage, ChatRequest,
    ModelMeta, ThinkingConfig, ToolSchema,
};
use crate::agent::thread_store::ThreadStore;
use crate::agent::tools::{self, ToolContext};
use crate::agent::types::{
    ContentBlock, MessageMetadata, ModelSelection, StoredMessage, ToolCallStatus, ToolResult,
};
use crate::agent::AgentDeps;
use crate::agent::RunHandle;

/// Options carried by agent_send_message (request-scoped, design 05 §5).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendOpts {
    /// "{instanceId}/{modelId}"
    pub model: String,
    #[serde(default)]
    pub thinking: bool,
    #[serde(default)]
    pub effort: Option<String>,
    /// Prefer this connection when resolving the identity (active tab).
    #[serde(default)]
    pub connection_hint: Option<String>,
}

struct ResolvedModel {
    instance: crate::agent::config::ProviderInstance,
    api_key: String,
    preset: Arc<dyn crate::agent::providers::ProviderPreset>,
    model_id: String,
    model_meta: Option<ModelMeta>,
}

/// Resolve "{instanceId}/{modelId}" against vault config.
async fn resolve_model(deps: &AgentDeps, model: &str) -> Result<ResolvedModel, String> {
    let (instance_id, model_id) = model
        .split_once('/')
        .ok_or_else(|| format!("Invalid model selection: {model}"))?;
    let (instances, api_key) = {
        let vault = deps.vault_manager.lock().await;
        let instances = crate::agent::config::read_providers(&vault).await;
        let api_key = crate::agent::config::read_api_key(&vault, instance_id).await;
        (instances, api_key)
    };
    let instance = instances
        .into_iter()
        .find(|i| i.id == instance_id)
        .ok_or_else(|| "The selected provider instance was deleted. Pick another model.".to_string())?;
    let api_key = api_key
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| format!("No API key configured for \"{}\". Add one in Settings -> AI.", instance.name))?;
    let preset = preset_by_id(&instance.preset)
        .ok_or_else(|| format!("Unknown provider preset: {}", instance.preset))?;

    // Model metadata from the cache / fresh list (best-effort).
    let model_meta = {
        let mut cache = deps.agent.models_cache.lock().await;
        let fresh = cache
            .get(instance_id)
            .filter(|(at, _)| at.elapsed().as_secs() < 600)
            .map(|(_, m)| m.clone());
        match fresh {
            Some(models) => models.into_iter().find(|m| m.id == model_id),
            None => {
                let fetched = preset.list_models(&instance, &api_key).await.ok();
                if let Some(models) = fetched {
                    let meta = models.iter().find(|m| m.id == model_id).cloned();
                    cache.insert(instance_id.to_string(), (Instant::now(), models));
                    meta
                } else {
                    None
                }
            }
        }
    };

    Ok(ResolvedModel {
        instance,
        api_key,
        preset,
        model_id: model_id.to_string(),
        model_meta,
    })
}

fn build_system_prompt(identity: &str, detected_os: Option<&str>) -> String {
    // identity = "user@host:port[#via=hash]"
    let base = identity.split('#').next().unwrap_or(identity);
    let (user, host) = base
        .split_once('@')
        .map(|(u, h)| (u, h))
        .unwrap_or(("unknown", base));
    let os = detected_os.unwrap_or("unknown");
    format!(
        "You are an AI agent embedded in Reach, an SSH remote management tool. You operate on a REMOTE server over SSH, so all file and terminal tools affect the remote host, not the user's local machine.\n\
        \n\
        Host: {host}  User: {user}  OS: {os}\n\
        \n\
        ## Tool Use\n\
        \n\
        - Follow the available tool schemas exactly and provide every required argument.\n\
        - Use only the tools that are currently available; the user may have disabled some.\n\
        - Prefer the most direct tool for the job: read_file/list_directory for inspecting files, the terminal tool for running commands.\n\
        - You can call multiple tools in a single response. Make all independent tool calls in parallel; only serialize when one call depends on another's result.\n\
        \n\
        ## Communication\n\
        \n\
        - Be concise, direct, friendly, and technical. Communicate efficiently and prioritize actionable guidance over verbose narration.\n\
        - Be transparent about uncertainty: say what you inferred versus what you verified.\n\
        - Prioritize technical correctness over affirming the user's assumptions. If something looks risky (destructive commands, production data), say so and explain.\n\
        \n\
        ## Doing the work\n\
        \n\
        - Keep going until the task is resolved; stop early only when you genuinely need information or a decision from the user.\n\
        - Address root causes, not symptoms. When debugging, gather evidence (logs, status, config) before changing anything.\n\
        - Verify with commands before claiming something is fixed. Do not claim success you did not observe.\n\
        \n\
        ## Final message\n\
        \n\
        - Summarize what changed and on which paths/services, and state what verification you ran (or why you did not run any)."
    )
}

/// Convert the active path into provider-neutral API messages.
fn to_api_messages(path: &[StoredMessage]) -> Vec<ChatMessage> {
    let mut out = Vec::new();
    for msg in path {
        match msg.role.as_str() {
            "user" => {
                let text: String = msg
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                out.push(ChatMessage::User(text));
            }
            "assistant" => {
                let mut content = String::new();
                let mut thinking: Option<String> = None;
                let mut details = None;
                let mut calls = Vec::new();
                let mut results: Vec<ChatMessage> = Vec::new();
                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => {
                            content.push_str(text);
                        }
                        ContentBlock::Thinking {
                            text,
                            provider_payload,
                        } => {
                            thinking.get_or_insert_with(String::new).push_str(text);
                            if details.is_none() {
                                details = provider_payload.clone();
                            }
                        }
                        ContentBlock::ToolCall {
                            id,
                            name,
                            args,
                            result,
                            ..
                        } => {
                            calls.push(AssistantToolCall {
                                id: id.clone(),
                                name: name.clone(),
                                arguments_json: serde_json::to_string(args).unwrap_or_default(),
                            });
                            let result_text = result
                                .as_ref()
                                .map(|r| r.llm_text.clone())
                                .unwrap_or_else(|| {
                                    "Tool call was interrupted.".to_string()
                                });
                            let is_error = result.as_ref().map(|r| r.is_error).unwrap_or(true);
                            results.push(ChatMessage::ToolResult(
                                crate::agent::providers::ToolResultMessage {
                                    tool_call_id: id.clone(),
                                    content: result_text,
                                    is_error,
                                },
                            ));
                        }
                    }
                }
                out.push(ChatMessage::Assistant(AssistantMessage {
                    content,
                    reasoning_content: thinking,
                    reasoning_details: details,
                    tool_calls: calls,
                }));
                out.extend(results);
            }
            _ => {}
        }
    }
    out
}

/// Per-round accumulation of streamed content.
#[derive(Default)]
struct RoundAcc {
    text: String,
    thinking: String,
    thinking_details: Option<serde_json::Value>,
    tool_calls: BTreeMap<usize, PendingToolCall>,
    last_usage: Option<crate::agent::types::Usage>,
    finish_reason: Option<String>,
    error: Option<String>,
}

#[derive(Default)]
struct PendingToolCall {
    id: Option<String>,
    name: Option<String>,
    args_json: String,
}

pub(crate) fn emit(app: &AppHandle, identity: &str, event: AgentEvent) {
    if let Err(e) = app.emit(&AgentEvent::channel(identity), &event) {
        tracing::warn!("agent: emit failed: {}", e);
    }
}

/// Fused "queue empty + unregister" check for normal run exits. Holding
/// both locks makes it atomic with agent_send_message's queue-or-start
/// decision (same lock order): a message queued before this point keeps
/// the run alive (injected at the top of the loop), while one sent after
/// finds no registered run and starts a fresh one — nothing can strand in
/// `queued` with no run left to consume it. RunEnd is emitted while still
/// holding the locks, so the events of any successor run are guaranteed to
/// come after it.
async fn try_finish_run(
    app: &AppHandle,
    agent: &crate::agent::AgentState,
    identity: &str,
    thread_id: &str,
    handle: &Arc<RunHandle>,
) -> bool {
    let mut runs = agent.runs.lock().await;
    let queued = agent.queued.lock().await;
    if queued.contains_key(thread_id) {
        return false;
    }
    // Identity check: an agent_send_now successor may already occupy the
    // slot; a dying run must never unregister it.
    if matches!(runs.get(thread_id), Some(h) if Arc::ptr_eq(h, handle)) {
        runs.remove(thread_id);
    }
    emit(
        app,
        identity,
        AgentEvent::RunEnd {
            thread_id: thread_id.to_string(),
        },
    );
    true
}

/// Entry point: run the agent loop for a thread until the model stops
/// calling tools and the queue is empty. `text` is None when the caller
/// already appended the user message (fork path).
pub async fn run_agent(
    app: AppHandle,
    deps: AgentDeps,
    identity: String,
    thread_id: String,
    text: Option<String>,
    opts: SendOpts,
) {
    let agent = &deps.agent;
    let store = match agent.thread_store().await {
        Ok(s) => s,
        Err(e) => {
            emit(&app, &identity, AgentEvent::Error {
                thread_id: thread_id.clone(),
                message: format!("Failed to open the thread store: {e}"),
            });
            return;
        }
    };

    // Append the incoming user message.
    if let Some(text) = text {
        let parent = active_leaf(&store, &thread_id).await;
        match store
            .append_message(
                &thread_id,
                parent.as_deref(),
                "user",
                vec![ContentBlock::Text { text }],
                None,
                None,
                None,
            )
            .await
        {
            Ok(msg) => emit(&app, &identity, AgentEvent::UserMessage {
                thread_id: thread_id.clone(),
                message: msg,
            }),
            Err(e) => {
                emit(&app, &identity, AgentEvent::Error {
                    thread_id: thread_id.clone(),
                    message: e,
                });
                return;
            }
        }
    }

    let _ = agent.ensure_read_paths(&thread_id).await;

    let resolved = match resolve_model(&deps, &opts.model).await {
        Ok(r) => r,
        Err(e) => {
            emit(&app, &identity, AgentEvent::Error {
                thread_id: thread_id.clone(),
                message: e,
            });
            return;
        }
    };

    // Model snapshot on the thread (design 05 §5).
    let _ = store
        .update_model_snapshot(
            &thread_id,
            &ModelSelection {
                model: opts.model.clone(),
                thinking: opts.thinking,
                effort: opts.effort.clone(),
            },
        )
        .await;

    // Best-effort OS detection, once per identity.
    let detected_os = detect_os(&app, &deps, &identity, opts.connection_hint.as_deref()).await;

    let handle = Arc::new(RunHandle {
        token: CancellationToken::new(),
        done: Arc::new(Notify::new()),
    });
    {
        let mut runs = agent.runs.lock().await;
        runs.insert(thread_id.clone(), handle.clone());
    }
    let result = run_loop(
        &app,
        &deps,
        &store,
        &identity,
        &thread_id,
        &opts,
        &resolved,
        detected_os.as_deref(),
        &handle.token,
        &handle,
    )
    .await;

    // A run that dies mid-stream (cancel/error) never reaches the
    // execute-path cleanup; drop its leftover streaming previews here.
    agent
        .previews
        .lock()
        .await
        .retain(|_, entry| entry.thread_id != thread_id);

    // Emit the terminal event before unregistering/waking agent_send_now
    // waiters, so the frontend settles running=false before the successor
    // run starts. (The Ok path's terminal event — RunEnd — is emitted by
    // run_loop itself at its fused exit point, try_finish_run.)
    let failed = result.is_err();
    match result {
        Ok(()) => {}
        Err(LoopExit::Cancelled) => {
            emit(&app, &identity, AgentEvent::Cancelled {
                thread_id: thread_id.clone(),
            });
        }
        Err(LoopExit::Error(e)) => {
            emit(&app, &identity, AgentEvent::Error {
                thread_id: thread_id.clone(),
                message: e,
            });
        }
    }

    // Abnormal exit: the queued message dies with the run. Without this it
    // would linger in the map and be injected into the *next* run as a
    // ghost message (the normal Ok exit implies an empty queue).
    if failed {
        agent.queued.lock().await.remove(&thread_id);
    }

    {
        let mut runs = agent.runs.lock().await;
        // Identity check: an agent_send_now successor may already occupy the
        // slot; a dying run must never unregister it. (On the Ok path the
        // handle was already removed by try_finish_run — this is a no-op.)
        if matches!(runs.get(&thread_id), Some(h) if Arc::ptr_eq(h, &handle)) {
            runs.remove(&thread_id);
        }
    }
    handle.done.notify_waiters();

    // Title generation after the first exchange (design 01 §2.1).
    maybe_generate_title(&app, &deps, &store, &identity, &thread_id).await;
}

async fn active_leaf(store: &ThreadStore, thread_id: &str) -> Option<String> {
    store
        .snapshot(thread_id)
        .await
        .ok()
        .and_then(|s| s.messages.last().map(|m| m.message.id.clone()))
}

enum LoopExit {
    Cancelled,
    Error(String),
}

/// Best-effort `uname` detection, cached per identity for the app session.
async fn detect_os(
    _app: &AppHandle,
    deps: &AgentDeps,
    identity: &str,
    hint: Option<&str>,
) -> Option<String> {
    {
        let cache = deps.agent.detected_os.lock().await;
        if let Some(os) = cache.get(identity) {
            return Some(os.clone());
        }
    }
    let handle = {
        let manager = deps.ssh_manager.lock().await;
        let id = manager.find_by_identity(identity, hint)?;
        manager.get_handle(&id).ok()?
    };
    let out = crate::ssh::client::exec_on_connection(&handle, "uname -s").await.ok()?;
    let os = out.trim().to_string();
    if os.is_empty() {
        return None;
    }
    deps
        .agent
        .detected_os
        .lock()
        .await
        .insert(identity.to_string(), os.clone());
    Some(os)
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    app: &AppHandle,
    deps: &AgentDeps,
    store: &Arc<ThreadStore>,
    identity: &str,
    thread_id: &str,
    opts: &SendOpts,
    resolved: &ResolvedModel,
    detected_os: Option<&str>,
    cancel: &CancellationToken,
    handle: &Arc<RunHandle>,
) -> Result<(), LoopExit> {
    let agent = &deps.agent;
    let system_prompt = build_system_prompt(identity, detected_os);

    loop {
        // 0. Queue injection at the round boundary (design 01 §3.5).
        let queued = agent.queued.lock().await.remove(thread_id);
        if let Some(q) = queued {
            let parent = active_leaf(store, thread_id).await;
            match store
                .append_message(
                    thread_id,
                    parent.as_deref(),
                    "user",
                    vec![ContentBlock::Text { text: q }],
                    None,
                    None,
                    None,
                )
                .await
            {
                Ok(msg) => emit(app, identity, AgentEvent::UserMessage {
                    thread_id: thread_id.to_string(),
                    message: msg,
                }),
                Err(e) => return Err(LoopExit::Error(e)),
            }
        }

        if cancel.is_cancelled() {
            return Err(LoopExit::Cancelled);
        }

        // 1. Assemble the request (system prompt is stable across the run).
        let snapshot = store
            .snapshot(thread_id)
            .await
            .map_err(LoopExit::Error)?;
        let mut messages = vec![ChatMessage::System(system_prompt.clone())];
        messages.extend(to_api_messages(
            &snapshot
                .messages
                .iter()
                .map(|p| p.message.clone())
                .collect::<Vec<_>>(),
        ));

        let tool_list = tools::all_tools();
        let mut tool_schemas: Vec<ToolSchema> = Vec::new();
        {
            let vault = deps.vault_manager.lock().await;
            for tool in &tool_list {
                let cfg = agent.resolved_tool_config(&vault, tool).await;
                if cfg.enabled {
                    tool_schemas.push(tool.schema());
                }
            }
        }

        let request = ChatRequest {
            model: resolved.model_id.clone(),
            model_meta: resolved.model_meta.clone(),
            messages,
            tools: tool_schemas,
            thinking: Some(ThinkingConfig {
                enabled: opts.thinking,
                effort: opts.effort.clone(),
            }),
            prompt_cache_key: Some(thread_id.to_string()),
        };

        // 2. Stream one round.
        let (event_tx, mut event_rx) = mpsc::channel::<ChatEvent>(256);
        let provider_cancel = cancel.clone();
        let provider = resolved.preset.clone();
        let instance = resolved.instance.clone();
        let api_key = resolved.api_key.clone();
        let provider_task = tokio::spawn(async move {
            provider
                .chat_stream(&instance, &api_key, request, event_tx, provider_cancel)
                .await
        });

        let message_id = Uuid::new_v4().to_string();
        // Announce the round up front: the frontend builds its streaming
        // placeholder from this event, so tool-call-only rounds (no text
        // deltas) render too. The same id is passed to append_message below,
        // so every event of this round — streaming, tool execution,
        // message_done — carries one stable message id.
        emit(app, identity, AgentEvent::MessageStart {
            thread_id: thread_id.to_string(),
            message_id: message_id.clone(),
        });
        let round_start = Instant::now();
        // First content delta (text / thinking / tool-call args) => TTFT.
        let mut first_delta_at: Option<Duration> = None;
        let mut acc = RoundAcc::default();
        let mut announced_tools: Vec<String> = Vec::new(); // tool_call ids announced as streaming
        let mut preview_tracker = StreamPreviews::new();
        let mut preview_tick = tokio::time::interval(Duration::from_millis(150));
        // One catch-up tick is enough after a slow preview (fs read + diff).
        preview_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            let event = tokio::select! {
                event = event_rx.recv() => event,
                _ = preview_tick.tick() => {
                    let dirty = preview_tracker.take_dirty();
                    if !dirty.is_empty() {
                        // Snapshot the dirty calls' (id, name, args) out of
                        // the accumulator so the tick owns no borrow of it.
                        let calls: Vec<(String, String, String)> = dirty
                            .iter()
                            .filter_map(|id| {
                                acc.tool_calls
                                    .values()
                                    .find(|p| p.id.as_deref() == Some(id.as_str()))
                                    .and_then(|p| {
                                        Some((id.clone(), p.name.clone()?, p.args_json.clone()))
                                    })
                            })
                            .collect();
                        preview_tracker.tick(app, deps, identity, thread_id, calls).await;
                    }
                    continue;
                }
            };
            let Some(event) = event else { break };
            if first_delta_at.is_none()
                && matches!(
                    event,
                    ChatEvent::TextDelta(_) | ChatEvent::ThinkingDelta(_) | ChatEvent::ToolCallDelta { .. }
                )
            {
                first_delta_at = Some(round_start.elapsed());
            }
            match event {
                ChatEvent::TextDelta(delta) => {
                    acc.text.push_str(&delta);
                    emit(app, identity, AgentEvent::TextDelta {
                        thread_id: thread_id.to_string(),
                        message_id: message_id.clone(),
                        delta,
                    });
                }
                ChatEvent::ThinkingDelta(delta) => {
                    acc.thinking.push_str(&delta);
                    emit(app, identity, AgentEvent::ThinkingDelta {
                        thread_id: thread_id.to_string(),
                        message_id: message_id.clone(),
                        delta,
                    });
                }
                ChatEvent::ReasoningDetails(details) => {
                    if acc.thinking_details.is_none() {
                        acc.thinking_details = Some(details);
                    }
                }
                ChatEvent::ToolCallDelta {
                    index,
                    id,
                    name,
                    args_json_delta,
                } => {
                    let entry = acc.tool_calls.entry(index).or_default();
                    if let Some(id) = id {
                        entry.id = Some(id);
                    }
                    if let Some(name) = name {
                        entry.name = Some(name);
                    }
                    entry.args_json.push_str(&args_json_delta);
                    if let (Some(tc_id), Some(_)) = (&entry.id, &entry.name) {
                        if !announced_tools.contains(tc_id) {
                            announced_tools.push(tc_id.clone());
                            emit(app, identity, AgentEvent::ToolCall {
                                thread_id: thread_id.to_string(),
                                tool_call: ToolCallView {
                                    id: tc_id.clone(),
                                    message_id: message_id.clone(),
                                    name: entry.name.clone().unwrap_or_default(),
                                    args_json: entry.args_json.clone(),
                                    status: ToolCallStatus::Streaming,
                                    result: None,
                                    warnings: None,
                                },
                            });
                        }
                        if !args_json_delta.is_empty() {
                            emit(app, identity, AgentEvent::ToolCallArgsDelta {
                                thread_id: thread_id.to_string(),
                                tool_call_id: tc_id.clone(),
                                args_json_delta,
                            });
                            // Args repair is cheap: emit per delta (deduped),
                            // so the final delta always yields the complete
                            // args. Only the diff preview stays throttled.
                            if let Some(patched) =
                                preview_tracker.patched_args(tc_id, &entry.args_json)
                            {
                                emit(app, identity, AgentEvent::ToolCallArgsPatched {
                                    thread_id: thread_id.to_string(),
                                    tool_call_id: tc_id.clone(),
                                    args_json: patched,
                                });
                            }
                            preview_tracker.mark_dirty(tc_id);
                        }
                    }
                }
                ChatEvent::Usage(usage) => {
                    acc.last_usage = Some(usage.clone());
                    emit(app, identity, AgentEvent::Usage {
                        thread_id: thread_id.to_string(),
                        usage,
                    });
                }
                ChatEvent::Done { finish_reason } => {
                    acc.finish_reason = Some(finish_reason);
                }
                ChatEvent::Error(e) => {
                    acc.error = Some(e);
                }
            }
        }

        let provider_result = provider_task.await;
        if cancel.is_cancelled() {
            return Err(LoopExit::Cancelled);
        }
        match provider_result {
            Err(e) => return Err(LoopExit::Error(format!("Provider task failed: {e}"))),
            Ok(Err(crate::agent::providers::ProviderError::Cancelled)) => {
                return Err(LoopExit::Cancelled);
            }
            Ok(Err(e)) => return Err(LoopExit::Error(e.to_string())),
            Ok(Ok(())) => {}
        }
        if let Some(e) = acc.error.take() {
            return Err(LoopExit::Error(e));
        }

        if let Some(usage) = &acc.last_usage {
            let _ = store.record_usage(thread_id, usage).await;
        }

        // 3. Persist the assistant message for this round.
        let mut content: Vec<ContentBlock> = Vec::new();
        if !acc.thinking.is_empty() || acc.thinking_details.is_some() {
            content.push(ContentBlock::Thinking {
                text: acc.thinking.clone(),
                provider_payload: acc.thinking_details.clone(),
            });
        }
        if !acc.text.is_empty() {
            content.push(ContentBlock::Text {
                text: acc.text.clone(),
            });
        }

        // Parse tool calls.
        let mut parsed_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
        for (_index, pending) in acc.tool_calls {
            let (id, name) = match (pending.id, pending.name) {
                (Some(id), Some(name)) => (id, name),
                _ => continue, // incomplete stream fragment; drop
            };
            let args_raw = if pending.args_json.trim().is_empty() {
                "{}".to_string()
            } else {
                pending.args_json.clone()
            };
            let args = serde_json::from_str::<serde_json::Value>(&args_raw)
                .unwrap_or_else(|_| serde_json::json!({ "__parse_error": args_raw }));
            parsed_calls.push((id, name, args));
        }

        let has_tool_calls = !parsed_calls.is_empty();
        let metadata = MessageMetadata {
            provider_instance: Some(resolved.instance.name.clone()),
            model: Some(resolved.model_id.clone()),
            usage: acc.last_usage.clone(),
            duration_ms: Some(round_start.elapsed().as_millis() as u64),
            ttft_ms: first_delta_at.map(|d| d.as_millis() as u64),
            tool_call_count: Some(parsed_calls.len() as u32),
        };

        if content.is_empty() && !has_tool_calls {
            // Nothing came back (e.g. empty finish). Don't persist noise.
            emit(app, identity, AgentEvent::MessageDone {
                thread_id: thread_id.to_string(),
                message_id: message_id.clone(),
                metadata: Some(metadata),
            });
            if try_finish_run(app, agent, identity, thread_id, handle).await {
                return Ok(());
            }
            continue;
        }

        for (id, name, args) in &parsed_calls {
            content.push(ContentBlock::ToolCall {
                id: id.clone(),
                name: name.clone(),
                args: args.clone(),
                status: ToolCallStatus::Streaming,
                result: None,
                warnings: None,
            });
        }

        let parent = active_leaf(store, thread_id).await;
        let assistant_msg = store
            .append_message(
                thread_id,
                parent.as_deref(),
                "assistant",
                content,
                acc.last_usage.as_ref(),
                Some(&metadata),
                Some(&message_id),
            )
            .await
            .map_err(LoopExit::Error)?;

        if !has_tool_calls {
            emit(app, identity, AgentEvent::MessageDone {
                thread_id: thread_id.to_string(),
                message_id: message_id.clone(),
                metadata: Some(metadata),
            });
            // Run ends when the model produced no tool calls and the queue
            // is empty; otherwise the top of the loop injects the queued
            // message and we continue.
            if try_finish_run(app, agent, identity, thread_id, handle).await {
                return Ok(());
            }
            continue;
        }

        // 4. Execute tool calls (parallel approvals, execute-on-approve).
        let final_content = execute_tool_calls(
            app,
            &deps,
            identity,
            thread_id,
            &assistant_msg,
            parsed_calls,
            opts,
            cancel,
        )
        .await?;

        // 5. Persist final tool call states on the assistant message.
        store
            .update_message(
                &assistant_msg.id,
                &final_content,
                acc.last_usage.as_ref(),
                Some(&metadata),
            )
            .await
            .map_err(LoopExit::Error)?;

        emit(app, identity, AgentEvent::MessageDone {
            thread_id: thread_id.to_string(),
            message_id: assistant_msg.id.clone(),
            metadata: Some(metadata),
        });

        if cancel.is_cancelled() {
            return Err(LoopExit::Cancelled);
        }
        // Back to the top: queue injection, then the next provider round.
    }
}

/// Execute one round of tool calls with approvals (design 02 §2 step 4,
/// 04 §2). Returns the assistant message's final content blocks.
async fn execute_tool_calls(
    app: &AppHandle,
    deps: &AgentDeps,
    identity: &str,
    thread_id: &str,
    assistant_msg: &StoredMessage,
    parsed_calls: Vec<(String, String, serde_json::Value)>,
    opts: &SendOpts,
    cancel: &CancellationToken,
) -> Result<Vec<ContentBlock>, LoopExit> {
    let agent = &deps.agent;
    // Streaming is over for this round's calls: drop their live previews
    // (the approval card / result payload carries the authoritative diff).
    {
        let mut previews = agent.previews.lock().await;
        for (id, _, _) in &parsed_calls {
            previews.remove(id);
        }
    }
    let read_file_options = {
        let vault = deps.vault_manager.lock().await;
        let tool = tools::tool_by_name("read_file");
        match tool {
            Some(t) => agent.resolved_tool_config(&vault, &t).await.options,
            None => serde_json::Map::new(),
        }
    };

    let mut results: Vec<(String, ContentBlock)> = Vec::new();

    // Sequential dispatch, but each approval waits independently; per Zed
    // semantics a call executes as soon as its own approval lands. We model
    // this with one spawned task per call and join them all.
    let mut set: tokio::task::JoinSet<(String, ContentBlock)> = tokio::task::JoinSet::new();

    for (id, name, args) in parsed_calls {
        let app = app.clone();
        let deps = deps.clone();
        let identity = identity.to_string();
        let thread_id = thread_id.to_string();
        let message_id = assistant_msg.id.clone();
        let cancel = cancel.clone();
        let opts = opts.clone();
        let read_file_options = read_file_options.clone();

        set.spawn(async move {
            execute_one_tool(
                app, deps, identity, thread_id, message_id, id, name, args, opts,
                read_file_options, cancel,
            )
            .await
        });
    }

    while let Some(joined) = set.join_next().await {
        if let Ok(pair) = joined {
            results.push(pair);
        }
    }

    // Preserve the model's call order in the persisted content.
    let order: Vec<String> = assistant_msg
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolCall { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();

    let mut final_content: Vec<ContentBlock> = assistant_msg
        .content
        .iter()
        .filter(|b| !matches!(b, ContentBlock::ToolCall { .. }))
        .cloned()
        .collect();

    let mut by_id: std::collections::HashMap<String, ContentBlock> = results.into_iter().collect();
    for id in order {
        if let Some(block) = by_id.remove(&id) {
            final_content.push(block);
        }
    }
    // Any leftover (shouldn't happen) appended in arbitrary order.
    final_content.extend(by_id.into_values());
    Ok(final_content)
}

#[allow(clippy::too_many_arguments)]
async fn execute_one_tool(
    app: AppHandle,
    deps: AgentDeps,
    identity: String,
    thread_id: String,
    message_id: String,
    tool_call_id: String,
    name: String,
    args: serde_json::Value,
    opts: SendOpts,
    read_file_options: serde_json::Map<String, serde_json::Value>,
    cancel: CancellationToken,
) -> (String, ContentBlock) {
    let agent = &deps.agent;
    let emit_view = |status: ToolCallStatus, result: Option<ToolResult>, warnings: Option<Vec<ApprovalWarning>>| {
        emit(&app, &identity, AgentEvent::ToolCall {
            thread_id: thread_id.clone(),
            tool_call: ToolCallView {
                id: tool_call_id.clone(),
                message_id: message_id.clone(),
                name: name.clone(),
                args_json: serde_json::to_string(&args).unwrap_or_default(),
                status,
                result: result.clone(),
                warnings,
            },
        });
    };

    let finish = |status: ToolCallStatus, result: ToolResult, warnings: Option<Vec<ApprovalWarning>>| {
        emit_view(status, Some(result.clone()), warnings);
        (
            tool_call_id.clone(),
            ContentBlock::ToolCall {
                id: tool_call_id.clone(),
                name: name.clone(),
                args: args.clone(),
                status,
                result: Some(result),
                warnings: None,
            },
        )
    };

    // Parse error short-circuit (defensive; providers stream partial JSON).
    if let Some(raw) = args.get("__parse_error").and_then(|v| v.as_str()) {
        return finish(
            ToolCallStatus::Failed,
            super::tools::err_text(format!(
                "Failed to parse tool arguments as JSON: {}",
                raw.chars().take(200).collect::<String>()
            )),
            None,
        );
    }

    let Some(tool) = tools::tool_by_name(&name) else {
        return finish(
            ToolCallStatus::Failed,
            super::tools::err_text(format!(
                "Unknown tool: {}. Use only the tools provided in this conversation.",
                name
            )),
            None,
        );
    };

    let config = {
        let vault = deps.vault_manager.lock().await;
        agent.resolved_tool_config(&vault, &tool).await
    };

    if !config.enabled {
        return finish(
            ToolCallStatus::Failed,
            super::tools::err_text(format!(
                "The {} tool is disabled in settings. Do not retry with this tool; tell the user it can be enabled in Settings -> AI.",
                name
            )),
            None,
        );
    }

    match permissions::decide(&name, &args, &config, &read_file_options) {
        ApprovalDecision::Deny { reason } => {
            return finish(ToolCallStatus::Failed, super::tools::err_text(reason), None);
        }
        ApprovalDecision::Allow => {}
        ApprovalDecision::RequireApproval { warnings } => {
            let ctx = build_context(
                &app, &deps, &identity, &thread_id, &tool_call_id, &config, &opts,
            );
            let payload = tool.approval_payload(&args, &ctx).await;
            // The payload computation may itself fail (e.g. gate violations);
            // write/edit tools return None then and the error surfaces at run.
            let mut all_warnings = warnings;
            all_warnings.extend(tool.approval_warnings(&args, &ctx));

            emit_view(ToolCallStatus::PendingApproval, None, Some(all_warnings.clone()));
            let approval = ApprovalRequest {
                tool_call_id: tool_call_id.clone(),
                tool: name.clone(),
                title: tool.title(&args),
                payload,
                warnings: all_warnings,
            };
            emit(&app, &identity, AgentEvent::ApprovalNeeded {
                thread_id: thread_id.clone(),
                approval: approval.clone(),
            });

            let (tx, rx) = oneshot::channel::<bool>();
            agent.approvals.lock().await.insert(
                tool_call_id.clone(),
                crate::agent::PendingApproval { request: approval, tx },
            );

            let approved = tokio::select! {
                _ = cancel.cancelled() => {
                    agent.approvals.lock().await.remove(&tool_call_id);
                    return finish(
                        ToolCallStatus::Cancelled,
                        super::tools::err_text(
                            "The user cancelled this operation while it was waiting for approval. Ask them what they would like to do next rather than automatically retrying."
                        ),
                        None,
                    );
                }
                decision = rx => {
                    decision.unwrap_or(false)
                }
            };

            if !approved {
                let suffix = match name.as_str() {
                    "write_file" => "\nNo changes were made.",
                    "edit_file" => "\nNo edits were made.",
                    _ => "",
                };
                // Drop any stashed prepared write.
                deps.agent
                    .pending_writes
                    .lock()
                    .unwrap()
                    .remove(&tool_call_id);
                return finish(
                    ToolCallStatus::Rejected,
                    super::tools::err_text(format!(
                        "Permission to run tool denied by user{}",
                        suffix
                    )),
                    None,
                );
            }
        }
    }

    if cancel.is_cancelled() {
        return finish(
            ToolCallStatus::Cancelled,
            super::tools::err_text(
                "The user cancelled this operation. Ask them what they would like to do next rather than automatically retrying.",
            ),
            None,
        );
    }

    emit_view(ToolCallStatus::Running, None, None);

    let ctx = build_context(
        &app, &deps, &identity, &thread_id, &tool_call_id, &config, &opts,
    );
    let result = tool.run(args.clone(), &ctx, cancel.clone()).await;
    let result = match result {
        Ok(r) => r,
        Err(r) => r,
    };
    let status = if cancel.is_cancelled() {
        ToolCallStatus::Cancelled
    } else if result.is_error {
        ToolCallStatus::Failed
    } else {
        ToolCallStatus::Success
    };
    finish(status, result, None)
}

fn build_context(
    app: &AppHandle,
    deps: &AgentDeps,
    identity: &str,
    thread_id: &str,
    tool_call_id: &str,
    config: &crate::agent::config::ToolConfig,
    _opts: &SendOpts,
) -> ToolContext {
    let agent = &deps.agent;
    let read_paths = {
        // Sync bridge: read_paths_for is async; the map is cheap to probe.
        // The run guarantees ensure_read_paths ran before any tool call.
        agent
            .read_paths
            .try_lock()
            .ok()
            .and_then(|m| m.get(thread_id).cloned())
            .unwrap_or_else(|| Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())))
    };
    ToolContext {
        app: app.clone(),
        identity: identity.to_string(),
        thread_id: thread_id.to_string(),
        tool_call_id: tool_call_id.to_string(),
        ssh_manager: deps.ssh_manager.clone(),
        sftp_backends: deps.sftp_backends.clone(),
        options: config.options.clone(),
        read_cache: agent.read_cache.clone(),
        read_paths,
        pending_writes: agent.pending_writes.clone(),
        terminals: agent.terminals.clone(),
    }
}

// ---------------------------------------------------------------------------
// Title generation (design 01 §2.1)
// ---------------------------------------------------------------------------

const TITLE_PROMPT: &str = "\
Generate a concise 3-7 word title for this conversation, omitting punctuation.
Go straight to the title, without any preamble and prefix like `Here's a concise suggestion:...` or `Title:`.
If the conversation is about a specific subject, include it in the title.
Generate the title in the same language as the conversation content.
Be descriptive. DO NOT speak in the first person.";

async fn maybe_generate_title(
    app: &AppHandle,
    deps: &AgentDeps,
    store: &Arc<ThreadStore>,
    identity: &str,
    thread_id: &str,
) {
    let Ok(Some(thread)) = store.get_thread(thread_id).await else {
        return;
    };
    if !thread.title.is_empty() {
        return;
    }

    let title_model = {
        let vault = deps.vault_manager.lock().await;
        crate::agent::config::read_title_model(&vault).await
    };

    let mut title: Option<String> = None;

    if let Some(selection) = title_model {
        if let Ok(resolved) = resolve_model(deps, &selection).await {
            let snapshot = store.snapshot(thread_id).await.ok();
            if let Some(snapshot) = snapshot {
                let mut messages = to_api_messages(
                    &snapshot
                        .messages
                        .iter()
                        .map(|p| p.message.clone())
                        .collect::<Vec<_>>(),
                );
                messages.push(ChatMessage::User(TITLE_PROMPT.to_string()));
                let (tx, mut rx) = mpsc::channel::<ChatEvent>(64);
                let req = ChatRequest {
                    model: resolved.model_id.clone(),
                    model_meta: resolved.model_meta.clone(),
                    messages,
                    tools: vec![],
                    thinking: None,
                    prompt_cache_key: None,
                };
                let instance = resolved.instance.clone();
                let key = resolved.api_key.clone();
                let cancel = CancellationToken::new();
                let handle = tokio::spawn(async move {
                    resolved
                        .preset
                        .chat_stream(&instance, &key, req, tx, cancel)
                        .await
                });
                let mut text = String::new();
                while let Some(ev) = rx.recv().await {
                    if let ChatEvent::TextDelta(d) = ev {
                        text.push_str(&d);
                        // Truncate at the first newline (design 01 §2.1).
                        if text.contains('\n') {
                            break;
                        }
                    }
                }
                handle.abort();
                let first_line = text.split('\n').next().unwrap_or("").trim().to_string();
                if !first_line.is_empty() {
                    title = Some(first_line);
                }
            }
        }
    }

    // Fallback: first user message, 30 chars.
    if title.is_none() {
        if let Ok(snapshot) = store.snapshot(thread_id).await {
            title = snapshot
                .messages
                .iter()
                .find(|m| m.message.role == "user")
                .and_then(|m| {
                    m.message.content.iter().find_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                })
                .map(|t| t.trim().chars().take(30).collect::<String>());
        }
    }

    if let Some(title) = title.filter(|t| !t.is_empty()) {
        if store.rename_thread(thread_id, &title).await.is_ok() {
            emit(app, identity, AgentEvent::TitleUpdated {
                thread_id: thread_id.to_string(),
                title,
            });
        }
    }
}
