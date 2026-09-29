use crate::plugin::hooks;
use crate::state::{AppState, TunnelConfig, TunnelType};
use crate::tunnel::manager::TunnelManager;
use crate::vault::types::SecretCategory;
use crate::vault::TUNNELS_VAULT;
use secrecy::SecretBox;

/// Ensure the internal tunnels vault exists and is unlocked. O(1).
async fn ensure_tunnels_vault(manager: &mut crate::vault::VaultManager) -> Result<String, String> {
    if let Some(vault_id) = manager.get_vault_id_by_name(TUNNELS_VAULT) {
        let _ = manager.open_vault(&vault_id, None, None).await;
        manager
            .unlock_vault(&vault_id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(vault_id)
    } else {
        let vault = manager
            .create_vault(TUNNELS_VAULT, crate::vault::types::VaultType::Private, None, None)
            .await
            .map_err(|e| e.to_string())?;
        Ok(vault.id)
    }
}

fn get_tunnels_vault_id_if_exists(manager: &crate::vault::VaultManager) -> Option<String> {
    manager.get_vault_id_by_name(TUNNELS_VAULT)
}

/// Load all persisted tunnels from the vault.
/// Empty when the vault is locked or no tunnels vault exists yet.
async fn load_persistent_tunnels(manager: &crate::vault::VaultManager) -> Vec<TunnelConfig> {
    if manager.is_locked() {
        return Vec::new();
    }
    let Some(vault_id) = get_tunnels_vault_id_if_exists(manager) else {
        return Vec::new();
    };
    let Ok(secrets) = manager.list_secrets(&vault_id).await else {
        return Vec::new();
    };

    let mut tunnels = Vec::new();
    for secret in secrets {
        if secret.category != "tunnel" {
            continue;
        }
        if let Ok(plaintext) = manager.read_secret(&vault_id, &secret.id).await {
            use secrecy::ExposeSecret;
            if let Ok(json) = String::from_utf8(plaintext.expose_secret().clone()) {
                if let Ok(tunnel) = serde_json::from_str::<TunnelConfig>(&json) {
                    tunnels.push(tunnel);
                }
            }
        }
    }
    tunnels
}

/// Find a persisted tunnel by id, returning its vault id and config.
async fn find_persistent_tunnel(
    manager: &crate::vault::VaultManager,
    tunnel_id: &str,
) -> Option<(String, TunnelConfig)> {
    if manager.is_locked() {
        return None;
    }
    let vault_id = get_tunnels_vault_id_if_exists(manager)?;
    if !manager.secret_exists(&vault_id, tunnel_id).await {
        return None;
    }
    let plaintext = manager.read_secret(&vault_id, tunnel_id).await.ok()?;
    use secrecy::ExposeSecret;
    let json = String::from_utf8(plaintext.expose_secret().clone()).ok()?;
    serde_json::from_str::<TunnelConfig>(&json)
        .ok()
        .map(|t| (vault_id, t))
}

/// Human-readable summary stored as the secret name (debugging/sync listings).
fn tunnel_display_name(tunnel: &TunnelConfig) -> String {
    let kind = match tunnel.tunnel_type {
        TunnelType::Local => "local",
        TunnelType::Remote => "remote",
        TunnelType::Dynamic => "dynamic",
    };
    format!(
        "{} :{} -> {}:{}",
        kind, tunnel.local_port, tunnel.remote_host, tunnel.remote_port
    )
}

/// Create a new tunnel configuration (does not start it yet).
///
/// With `session_id` the tunnel is persisted encrypted in the vault and
/// survives restarts. Without it, the tunnel is ephemeral: bound to the
/// runtime `connection_id` (quick connect) and lost when the app exits.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn tunnel_create(
    state: tauri::State<'_, AppState>,
    tunnel_type: TunnelType,
    local_port: u16,
    remote_host: String,
    remote_port: u16,
    connection_id: Option<String>,
    session_id: Option<String>,
) -> Result<TunnelConfig, String> {
    if session_id.is_none() && connection_id.is_none() {
        return Err("Tunnel requires a session or an active connection".to_string());
    }

    let tunnel = TunnelManager::create_tunnel(
        tunnel_type,
        local_port,
        &remote_host,
        remote_port,
        session_id,
        connection_id,
    );

    if tunnel.session_id.is_some() {
        let mut manager = state.vault_manager.lock().await;
        if manager.is_locked() {
            return Err("Vault is locked. Set a master password first.".to_string());
        }
        let vault_id = ensure_tunnels_vault(&mut manager).await?;
        let json = serde_json::to_string(&tunnel).map_err(|e| e.to_string())?;
        manager
            .create_secret_with_id(
                &vault_id,
                &tunnel.id,
                &tunnel_display_name(&tunnel),
                SecretCategory::Tunnel,
                SecretBox::new(Box::new(json.into_bytes())),
            )
            .await
            .map_err(|e| e.to_string())?;
        tracing::info!("Created persistent tunnel: {}", tunnel.id);
    } else {
        let mut tunnels = state.tunnels.write().await;
        tunnels.insert(tunnel.id.clone(), tunnel.clone());
        tracing::info!("Created ephemeral tunnel: {}", tunnel.id);
    }

    Ok(tunnel)
}

/// Start an existing tunnel, establishing the actual port forwarding.
///
/// Session-bound tunnels resolve their SSH connection by agent scope
/// (`session:<id>`), so any live connection of that session works —
/// including reconnects that minted a new connection id.
#[tauri::command]
#[tracing::instrument(skip(app, state))]
pub async fn tunnel_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    tunnel_id: String,
    prefer_connection_id: Option<String>,
) -> Result<(), String> {
    // Locate the config: ephemeral in-memory first, then the vault.
    let ephemeral = {
        let tunnels = state.tunnels.read().await;
        tunnels.get(&tunnel_id).cloned()
    };
    let tunnel_config = match ephemeral {
        Some(t) => t,
        None => {
            let manager = state.vault_manager.lock().await;
            find_persistent_tunnel(&manager, &tunnel_id)
                .await
                .map(|(_, t)| t)
                .ok_or_else(|| format!("Tunnel not found: {}", tunnel_id))?
        }
    };

    // Resolve the SSH connection serving this tunnel. Session-bound
    // tunnels prefer the caller's current connection (the tab the user is
    // looking at) so the tunnel dies predictably with that tab.
    let (handle, connection_id) = {
        let ssh_manager = state.ssh_manager.lock().await;
        let connection_id = match &tunnel_config.session_id {
            Some(session_id) => {
                let scope = format!("session:{}", session_id);
                ssh_manager.find_by_scope(&scope, prefer_connection_id.as_deref())
            }
            None => tunnel_config.connection_id.clone(),
        };
        let connection_id = connection_id.ok_or_else(|| {
            "Session is not connected. Connect it before starting the tunnel.".to_string()
        })?;
        let handle = ssh_manager
            .get_handle(&connection_id)
            .map_err(|e| e.to_string())?;
        (handle, connection_id)
    };

    {
        let mut tunnel_manager = state.tunnel_manager.lock().await;
        tunnel_manager
            .start_tunnel(&tunnel_config, &connection_id, &handle)
            .await
            .map_err(|e| e.to_string())?;
    }

    // Fire-and-forget hook dispatch — tunnel start completes for the user
    // immediately; plugin reactions run in the background with a per-hook
    // timeout enforced by PluginManager::dispatch_hook.
    let hook = hooks::tunnel_started(&tunnel_id, tunnel_config.local_port);
    let plugin_mgr = state.plugin_manager.clone();
    let app_for_hook = app.clone();
    tokio::spawn(async move {
        let mut mgr = plugin_mgr.lock().await;
        mgr.dispatch_hook(&hook, Some(&app_for_hook)).await;
    });

    Ok(())
}

/// Stop an active tunnel. Idempotent.
#[tauri::command]
#[tracing::instrument(skip(app, state))]
pub async fn tunnel_stop(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    tunnel_id: String,
) -> Result<(), String> {
    {
        let mut tunnel_manager = state.tunnel_manager.lock().await;
        tunnel_manager
            .stop_tunnel(&tunnel_id)
            .await
            .map_err(|e| e.to_string())?;
    }

    // Fire-and-forget hook dispatch (see tunnel_start for rationale).
    let hook = hooks::tunnel_stopped(&tunnel_id);
    let plugin_mgr = state.plugin_manager.clone();
    let app_for_hook = app.clone();
    tokio::spawn(async move {
        let mut mgr = plugin_mgr.lock().await;
        mgr.dispatch_hook(&hook, Some(&app_for_hook)).await;
    });

    Ok(())
}

/// Delete a tunnel: stops the forwarder if running, then removes the
/// persisted record (session-bound) or the in-memory entry (ephemeral).
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn tunnel_delete(state: tauri::State<'_, AppState>, tunnel_id: String) -> Result<(), String> {
    {
        let mut tunnel_manager = state.tunnel_manager.lock().await;
        tunnel_manager
            .stop_tunnel(&tunnel_id)
            .await
            .map_err(|e| e.to_string())?;
    }

    {
        let mut tunnels = state.tunnels.write().await;
        tunnels.remove(&tunnel_id);
    }

    let manager = state.vault_manager.lock().await;
    if !manager.is_locked() {
        if let Some(vault_id) = get_tunnels_vault_id_if_exists(&manager) {
            // NotFound simply means the tunnel was ephemeral — ignore.
            let _ = manager.delete_secret(&vault_id, &tunnel_id).await;
        }
    }

    tracing::info!("Deleted tunnel: {}", tunnel_id);
    Ok(())
}

/// List all tunnels: persisted (session-bound) plus ephemeral
/// (connection-bound), with runtime `active` state merged in.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn tunnel_list(state: tauri::State<'_, AppState>) -> Result<Vec<TunnelConfig>, String> {
    let mut tunnels = {
        let manager = state.vault_manager.lock().await;
        load_persistent_tunnels(&manager).await
    };

    {
        let ephemeral = state.tunnels.read().await;
        tunnels.extend(ephemeral.values().cloned());
    }

    // `active` is runtime-only; the forwarder task registry is the source of truth.
    let tunnel_manager = state.tunnel_manager.lock().await;
    for tunnel in &mut tunnels {
        tunnel.active = tunnel_manager.is_active(&tunnel.id);
    }

    Ok(tunnels)
}
