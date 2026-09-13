//! Agent IPC commands (design 02 §7). Registered in both handler blocks in
//! lib.rs (desktop + mobile).

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, State};

use crate::agent::agent_loop::{self, SendOpts};
use crate::agent::config::{self, ProviderInstance, ToolConfig};
use crate::agent::events::AgentEvent;
use crate::agent::identity::{compute_identity, ChainHop};
use crate::agent::providers::{self, ModelMeta};
use crate::agent::thread_store::ThreadStore;
use crate::agent::tools;
use crate::agent::tools::edit_match::DiffPreview;
use crate::agent::types::{ThreadSnapshot, ThreadSummary};
use crate::agent::AgentDeps;
use crate::state::AppState;

async fn store(state: &AppState) -> Result<Arc<ThreadStore>, String> {
    state.agent.thread_store().await
}

// ---------------------------------------------------------------------------
// Messaging
// ---------------------------------------------------------------------------

/// Send a user message and start the agent loop. When a run is already
/// active the message is queued (max 1) and injected at the next round
/// boundary (design 01 §3.5). Returns "started" | "queued".
///
/// The queue-or-start decision holds both locks (order: runs -> queued,
/// same as run_loop's try_finish_run) so it is atomic with the run's
/// fused exit: a message can never land in `queued` after the last round
/// decided the queue was empty, which would strand it with no run left
/// to consume it.
#[tauri::command]
pub async fn agent_send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    identity: String,
    thread_id: String,
    text: String,
    opts: SendOpts,
) -> Result<String, String> {
    {
        let runs = state.agent.runs.lock().await;
        let mut queued = state.agent.queued.lock().await;
        if runs.contains_key(&thread_id) {
            if queued.contains_key(&thread_id) {
                return Ok("queued_full".to_string());
            }
            queued.insert(thread_id.clone(), text.clone());
            return Ok("queued".to_string());
        }
    }

    let deps = AgentDeps::from_state(state.inner());
    tauri::async_runtime::spawn(async move {
        agent_loop::run_agent(app, deps, identity, thread_id, Some(text), opts).await;
    });
    Ok("started".to_string())
}

/// Cancel the active run of a thread (SSE, approvals, and running tools all
/// observe the token).
#[tauri::command]
pub async fn agent_cancel(state: State<'_, AppState>, thread_id: String) -> Result<(), String> {
    let handle = state.agent.runs.lock().await.get(&thread_id).cloned();
    if let Some(handle) = handle {
        handle.token.cancel();
    }
    Ok(())
}

/// Remove the queued message of a thread (design 01 §3.5: the queued bar's
/// delete/edit actions). Returns true when a message was dequeued.
#[tauri::command]
pub async fn agent_dequeue(state: State<'_, AppState>, thread_id: String) -> Result<bool, String> {
    Ok(state.agent.queued.lock().await.remove(&thread_id).is_some())
}

/// Upper bound for waiting on a run to observe cancellation. Only tools
/// that ignore the cancel token (SFTP file ops) can delay the exit.
const SEND_NOW_STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// Force-send the queued message (design 01 §3.5 "send now"): supersede the
/// queue, cancel the active run, wait for it to fully exit, then start a
/// fresh run — atomically, so no racing message can slip into the dying
/// run's queue slot.
#[tauri::command]
pub async fn agent_send_now(
    app: AppHandle,
    state: State<'_, AppState>,
    identity: String,
    thread_id: String,
    text: String,
    opts: SendOpts,
) -> Result<(), String> {
    // Supersede the queue. If the dying run already consumed the message
    // (injected at a round boundary), start the successor with None so it
    // answers the existing message instead of duplicating it.
    let dequeued = state.agent.queued.lock().await.remove(&thread_id);
    let mut initial = dequeued.map(|_| text);

    let handle = state.agent.runs.lock().await.get(&thread_id).cloned();
    if let Some(handle) = handle {
        handle.token.cancel();
        if tokio::time::timeout(SEND_NOW_STOP_TIMEOUT, handle.done.notified())
            .await
            .is_err()
        {
            // The run is still alive: roll back into the queue so the
            // message is consumed at its next round boundary, not lost.
            if let Some(t) = initial.take() {
                state.agent.queued.lock().await.insert(thread_id, t);
            }
            return Err(
                "The current run did not stop in time. Your message was kept in the queue."
                    .to_string(),
            );
        }
    }

    let deps = AgentDeps::from_state(state.inner());
    tauri::async_runtime::spawn(async move {
        agent_loop::run_agent(app, deps, identity, thread_id, initial, opts).await;
    });
    Ok(())
}

/// Resolve a pending approval.
#[tauri::command]
pub async fn agent_approve(
    state: State<'_, AppState>,
    tool_call_id: String,
    approved: bool,
) -> Result<(), String> {
    let pending = state
        .agent
        .approvals
        .lock()
        .await
        .remove(&tool_call_id);
    match pending {
        Some(p) => {
            let _ = p.tx.send(approved);
            Ok(())
        }
        None => Err("No pending approval for this tool call".to_string()),
    }
}

/// Resize the PTY of a running terminal tool call (card expanded view).
#[tauri::command]
pub async fn agent_terminal_resize(
    state: State<'_, AppState>,
    tool_call_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    let handle = state
        .agent
        .terminals
        .lock()
        .unwrap()
        .get(&tool_call_id)
        .map(|h| h.resize_tx.clone());
    match handle {
        Some(tx) => {
            let _ = tx.send((cols, rows));
            Ok(())
        }
        None => Ok(()), // finished already; not an error
    }
}

/// Stop a running terminal tool call (card stop button).
#[tauri::command]
pub async fn agent_terminal_stop(
    state: State<'_, AppState>,
    tool_call_id: String,
) -> Result<(), String> {
    let sender = state
        .agent
        .terminals
        .lock()
        .unwrap()
        .get(&tool_call_id)
        .and_then(|h| h.stop_tx.lock().unwrap().take());
    if let Some(tx) = sender {
        let _ = tx.send(());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Threads
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn agent_threads_list(
    state: State<'_, AppState>,
    identity: String,
) -> Result<Vec<ThreadSummary>, String> {
    store(&state).await?.list_threads(&identity).await
}

#[tauri::command]
pub async fn agent_threads_list_all(
    state: State<'_, AppState>,
) -> Result<Vec<ThreadSummary>, String> {
    store(&state).await?.list_all_threads().await
}

#[tauri::command]
pub async fn agent_thread_create(
    state: State<'_, AppState>,
    identity: String,
) -> Result<ThreadSummary, String> {
    store(&state).await?.create_thread(&identity).await
}

#[tauri::command]
pub async fn agent_thread_rename(
    state: State<'_, AppState>,
    thread_id: String,
    title: String,
) -> Result<(), String> {
    store(&state).await?.rename_thread(&thread_id, &title).await
}

#[tauri::command]
pub async fn agent_thread_archive(
    state: State<'_, AppState>,
    thread_id: String,
    archived: bool,
) -> Result<(), String> {
    store(&state).await?.set_archived(&thread_id, archived).await
}

#[tauri::command]
pub async fn agent_thread_delete(
    state: State<'_, AppState>,
    thread_id: String,
) -> Result<(), String> {
    // Cancel any active run first.
    let handle = state.agent.runs.lock().await.get(&thread_id).cloned();
    if let Some(handle) = handle {
        handle.token.cancel();
    }
    store(&state).await?.delete_thread(&thread_id).await
}

#[tauri::command]
pub async fn agent_thread_set_draft(
    state: State<'_, AppState>,
    thread_id: String,
    draft: String,
) -> Result<(), String> {
    store(&state).await?.set_draft(&thread_id, &draft).await
}

#[tauri::command]
pub async fn agent_thread_messages(
    state: State<'_, AppState>,
    thread_id: String,
) -> Result<ThreadSnapshot, String> {
    let mut snapshot = store(&state).await?.snapshot(&thread_id).await?;
    let running = state.agent.runs.lock().await.contains_key(&thread_id);
    state.agent.enrich_snapshot(&mut snapshot, running).await;
    Ok(snapshot)
}

/// Snapshot plus whether a run is active (panel reopen path, 01 §5).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadState {
    #[serde(flatten)]
    pub snapshot: ThreadSnapshot,
    pub running: bool,
    /// Live diff previews of tool calls still streaming, keyed by tool_call
    /// id; only present while a run is active and previews exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previews: Option<std::collections::HashMap<String, DiffPreview>>,
}

#[tauri::command]
pub async fn agent_get_thread_state(
    state: State<'_, AppState>,
    thread_id: String,
) -> Result<ThreadState, String> {
    let mut snapshot = store(&state).await?.snapshot(&thread_id).await?;
    let running = state.agent.runs.lock().await.contains_key(&thread_id);
    let previews = state.agent.enrich_snapshot(&mut snapshot, running).await;
    Ok(ThreadState {
        snapshot,
        running,
        previews: if previews.is_empty() { None } else { Some(previews) },
    })
}

/// Edit a user message and fork a new branch from it (design 01 §2.4).
/// Returns the thread snapshot with the fork as the active path, so the
/// frontend can switch the visible branch immediately.
#[tauri::command]
pub async fn agent_edit_message(
    app: AppHandle,
    state: State<'_, AppState>,
    identity: String,
    thread_id: String,
    message_id: String,
    new_content: String,
    opts: SendOpts,
) -> Result<ThreadSnapshot, String> {
    let store = store(&state).await?;
    let msg = store
        .get_message(&message_id)
        .await?
        .ok_or_else(|| "Message not found".to_string())?;
    if msg.role != "user" {
        return Err("Only user messages can be edited".to_string());
    }
    if state.agent.runs.lock().await.contains_key(&thread_id) {
        return Err("A run is active; cancel it before editing history".to_string());
    }

    store
        .append_message(
            &thread_id,
            msg.parent_id.as_deref(),
            "user",
            vec![crate::agent::types::ContentBlock::Text { text: new_content }],
            None,
            None,
            None,
        )
        .await?;

    // Snapshot before spawning the run: the response is then at least as
    // fresh as the fork, and any run events that overtake it can only add
    // to it (the frontend lazily re-creates its streaming placeholder).
    let snapshot = store.snapshot(&thread_id).await?;

    let deps = AgentDeps::from_state(state.inner());
    tauri::async_runtime::spawn(async move {
        agent_loop::run_agent(app, deps, identity, thread_id, None, opts).await;
    });
    Ok(snapshot)
}

#[tauri::command]
pub async fn agent_set_active_branch(
    state: State<'_, AppState>,
    thread_id: String,
    at_message_id: String,
    direction: String,
) -> Result<ThreadSnapshot, String> {
    let mut snapshot = store(&state)
        .await?
        .switch_branch(&thread_id, &at_message_id, &direction)
        .await?;
    let running = state.agent.runs.lock().await.contains_key(&thread_id);
    state.agent.enrich_snapshot(&mut snapshot, running).await;
    Ok(snapshot)
}

// ---------------------------------------------------------------------------
// Providers & models
// ---------------------------------------------------------------------------

/// Provider instances without API keys.
#[tauri::command]
pub async fn agent_providers_list(
    state: State<'_, AppState>,
) -> Result<Vec<ProviderInstance>, String> {
    let vault = state.vault_manager.lock().await;
    Ok(config::read_providers(&vault).await)
}

/// Preset catalog for the "Add provider" flow: (id, display name, default base URL).
#[tauri::command]
pub async fn agent_provider_presets() -> Result<Vec<(String, String, String)>, String> {
    Ok(providers::known_presets()
        .into_iter()
        .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
        .collect())
}

/// Add an instance. Default name: preset name, then "Name 2", "Name 3"...
#[tauri::command]
pub async fn agent_provider_add(
    state: State<'_, AppState>,
    preset: String,
    name: Option<String>,
    base_url: Option<String>,
    api_key: String,
) -> Result<ProviderInstance, String> {
    let preset_name = providers::preset_display_name(&preset)
        .ok_or_else(|| format!("Unknown preset: {preset}"))?;
    let mut vault = state.vault_manager.lock().await;
    let mut instances = config::read_providers(&vault).await;
    let count = instances.iter().filter(|i| i.preset == preset).count();
    let name = name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            if count == 0 {
                preset_name.to_string()
            } else {
                format!("{} {}", preset_name, count + 1)
            }
        });
    let instance = ProviderInstance {
        id: uuid::Uuid::new_v4().to_string(),
        preset,
        name,
        base_url: base_url.filter(|u| !u.trim().is_empty()),
    };
    config::write_api_key(&mut vault, &instance.id, api_key.trim()).await?;
    instances.push(instance.clone());
    config::write_providers(&mut vault, &instances).await?;
    Ok(instance)
}

#[tauri::command]
pub async fn agent_provider_update(
    state: State<'_, AppState>,
    instance_id: String,
    name: Option<String>,
    base_url: Option<String>,
) -> Result<(), String> {
    let mut vault = state.vault_manager.lock().await;
    let mut instances = config::read_providers(&vault).await;
    let Some(inst) = instances.iter_mut().find(|i| i.id == instance_id) else {
        return Err("Provider instance not found".to_string());
    };
    if let Some(name) = name {
        if !name.trim().is_empty() {
            inst.name = name;
        }
    }
    if let Some(url) = base_url {
        inst.base_url = if url.trim().is_empty() {
            None
        } else {
            Some(url.trim().to_string())
        };
    }
    config::write_providers(&mut vault, &instances).await
}

#[tauri::command]
pub async fn agent_provider_delete(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<(), String> {
    let mut vault = state.vault_manager.lock().await;
    let mut instances = config::read_providers(&vault).await;
    instances.retain(|i| i.id != instance_id);
    config::write_providers(&mut vault, &instances).await?;
    config::delete_api_key(&mut vault, &instance_id).await?;
    state
        .agent
        .models_cache
        .lock()
        .await
        .remove(&instance_id);
    Ok(())
}

#[tauri::command]
pub async fn agent_provider_set_api_key(
    state: State<'_, AppState>,
    instance_id: String,
    api_key: String,
) -> Result<(), String> {
    let mut vault = state.vault_manager.lock().await;
    config::write_api_key(&mut vault, &instance_id, api_key.trim()).await?;
    // Key rotation invalidates the model cache.
    state
        .agent
        .models_cache
        .lock()
        .await
        .remove(&instance_id);
    Ok(())
}

#[tauri::command]
pub async fn agent_provider_get_api_key(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<String, String> {
    let vault = state.vault_manager.lock().await;
    Ok(config::read_api_key(&vault, &instance_id)
        .await
        .unwrap_or_default())
}

/// Validate an instance by fetching its model list.
#[tauri::command]
pub async fn agent_provider_validate(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<Vec<ModelMeta>, String> {
    let (instance, key) = {
        let vault = state.vault_manager.lock().await;
        let instance = config::read_providers(&vault)
            .await
            .into_iter()
            .find(|i| i.id == instance_id);
        let key = config::read_api_key(&vault, &instance_id).await;
        (instance, key)
    };
    let instance = instance.ok_or_else(|| "Provider instance not found".to_string())?;
    let key = key
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| "No API key configured".to_string())?;
    let preset = providers::preset_by_id(&instance.preset)
        .ok_or_else(|| format!("Unknown preset: {}", instance.preset))?;
    let models = preset
        .list_models(&instance, &key)
        .await
        .map_err(|e| e.to_string())?;
    state
        .agent
        .models_cache
        .lock()
        .await
        .insert(instance_id, (std::time::Instant::now(), models.clone()));
    Ok(models)
}

/// Aggregated models of all configured instances (grouped by instance,
/// 10-minute cache per instance; design 05 §4).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceModels {
    pub instance: ProviderInstance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ModelMeta>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[tauri::command]
pub async fn agent_models_list(
    state: State<'_, AppState>,
    force_refresh: Option<bool>,
) -> Result<Vec<InstanceModels>, String> {
    let instances = {
        let vault = state.vault_manager.lock().await;
        config::read_providers(&vault).await
    };
    let force = force_refresh.unwrap_or(false);
    let mut out = Vec::new();
    for instance in instances {
        let cached = {
            let cache = state.agent.models_cache.lock().await;
            cache
                .get(&instance.id)
                .filter(|(at, _)| !force && at.elapsed().as_secs() < 600)
                .map(|(_, m)| m.clone())
        };
        if let Some(models) = cached {
            out.push(InstanceModels {
                instance,
                models: Some(models),
                error: None,
            });
            continue;
        }
        let key = {
            let vault = state.vault_manager.lock().await;
            config::read_api_key(&vault, &instance.id).await
        };
        let Some(key) = key.filter(|k| !k.trim().is_empty()) else {
            out.push(InstanceModels {
                instance,
                models: None,
                error: Some("No API key configured".to_string()),
            });
            continue;
        };
        let preset = match providers::preset_by_id(&instance.preset) {
            Some(p) => p,
            None => continue,
        };
        match preset.list_models(&instance, &key).await {
            Ok(models) => {
                state
                    .agent
                    .models_cache
                    .lock()
                    .await
                    .insert(instance.id.clone(), (std::time::Instant::now(), models.clone()));
                out.push(InstanceModels {
                    instance,
                    models: Some(models),
                    error: None,
                });
            }
            Err(e) => out.push(InstanceModels {
                instance,
                models: None,
                error: Some(e.to_string()),
            }),
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tool settings (design 04 §4.3)
// ---------------------------------------------------------------------------

/// Tool descriptors + current user config, for the settings page.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSettingsEntry {
    #[serde(flatten)]
    pub descriptor: tools::ToolDescriptor,
    pub enabled: bool,
    pub require_approval: bool,
    /// Current option values (defaults merged under user config).
    pub values: serde_json::Map<String, serde_json::Value>,
}

#[tauri::command]
pub async fn agent_tool_schemas(
    state: State<'_, AppState>,
) -> Result<Vec<ToolSettingsEntry>, String> {
    let vault = state.vault_manager.lock().await;
    let mut out = Vec::new();
    for tool in tools::all_tools() {
        let cfg = state.agent.resolved_tool_config(&vault, &tool).await;
        out.push(ToolSettingsEntry {
            descriptor: tools::ToolDescriptor {
                name: tool.name().to_string(),
                description: tool.schema().description,
                default_requires_approval: tool.default_requires_approval(),
                options: tool.settings_schema(),
            },
            enabled: cfg.enabled,
            require_approval: cfg.require_approval,
            values: cfg.options,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn agent_set_tool_config(
    state: State<'_, AppState>,
    tool: String,
    config_value: ToolConfig,
) -> Result<(), String> {
    let mut vault = state.vault_manager.lock().await;
    let mut configs = config::read_tool_configs(&vault).await;
    configs.insert(tool, config_value);
    config::write_tool_configs(&mut vault, &configs).await?;
    state.agent.invalidate_tool_configs().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Title model
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn agent_title_model_get(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let vault = state.vault_manager.lock().await;
    Ok(config::read_title_model(&vault).await)
}

#[tauri::command]
pub async fn agent_title_model_set(
    state: State<'_, AppState>,
    value: Option<String>,
) -> Result<(), String> {
    let mut vault = state.vault_manager.lock().await;
    match value {
        Some(v) if !v.trim().is_empty() => config::write_title_model(&mut vault, v.trim()).await,
        _ => config::kv_delete(&mut vault, config::KEY_TITLE_MODEL).await,
    }
}

// ---------------------------------------------------------------------------
// Migration (design 04 §5)
// ---------------------------------------------------------------------------

/// One-time migration: create the first OpenRouter instance from the old
/// settings keys, then delete them.
#[tauri::command]
pub async fn agent_migrate_legacy_settings(state: State<'_, AppState>) -> Result<bool, String> {
    let mut vault = state.vault_manager.lock().await;
    if vault.is_locked() {
        return Ok(false);
    }
    let legacy_key = config::kv_read(&vault, "openrouter_api_key").await;
    let Some(api_key) = legacy_key.filter(|k| !k.trim().is_empty()) else {
        return Ok(false); // nothing to migrate
    };
    let base_url = config::kv_read(&vault, "openrouter_url").await;
    let old_default_model = config::kv_read(&vault, "default_ai_model").await;

    let mut instances = config::read_providers(&vault).await;
    if instances.iter().any(|i| i.preset == "openrouter") {
        return Ok(false); // already migrated
    }

    let instance = ProviderInstance {
        id: uuid::Uuid::new_v4().to_string(),
        preset: "openrouter".to_string(),
        name: "OpenRouter".to_string(),
        base_url: base_url.filter(|u| !u.trim().is_empty()),
    };
    config::write_api_key(&mut vault, &instance.id, api_key.trim()).await?;
    instances.push(instance.clone());
    config::write_providers(&mut vault, &instances).await?;

    // Keep the old default model usable as the title model when set.
    if let Some(model) = old_default_model.filter(|m| !m.trim().is_empty()) {
        let _ = config::write_title_model(&mut vault, &format!("{}/{}", instance.id, model)).await;
    }

    let _ = config::kv_delete(&mut vault, "openrouter_api_key").await;
    let _ = config::kv_delete(&mut vault, "openrouter_url").await;
    let _ = config::kv_delete(&mut vault, "default_ai_model").await;
    tracing::info!("agent: migrated legacy OpenRouter settings to instance {}", instance.id);
    Ok(true)
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Emit helper for the frontend's initial subscription sanity check.
/// (Unused in production; the loop emits on agent-event-{base64url(identity)}.)
#[tauri::command]
pub async fn agent_ping() -> Result<String, String> {
    let _ = AgentEvent::channel("ping");
    Ok("pong".to_string())
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Compute the agent identity a saved-session link would connect with — the
/// exact normalization `ssh_connect` applies (design 01 §1.1): a non-empty
/// jump chain fingerprints the hops only (mirrors `connect_via_jump`), a
/// direct link fingerprints the proxy (mirrors `connect`). Lets the
/// frontend match sessions to identities without reimplementing the chain
/// hash.
#[tauri::command]
pub fn agent_compute_identity(
    username: String,
    host: String,
    port: u16,
    jump_chain: Option<Vec<crate::state::JumpHostConfig>>,
    proxy: Option<crate::state::ProxyConfig>,
) -> String {
    match jump_chain.as_ref().filter(|c| !c.is_empty()) {
        Some(chain) => {
            let hops: Vec<ChainHop> = chain.iter().map(ChainHop::from).collect();
            compute_identity(&username, &host, port, Some(&hops), None)
        }
        None => compute_identity(&username, &host, port, None, proxy.as_ref()),
    }
}
