use crate::state::AppState;
use crate::ssh::client::{AuthParams, ConnectionInfo, HostKeyDecision, JumpHostParams, exec_on_connection};
use crate::plugin::hooks;

/// Parameters for a jump host received from the frontend.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JumpHostConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub password: Option<String>,
    pub key_path: Option<String>,
    pub key_passphrase: Option<String>,
}

fn build_auth(
    auth_method: &str,
    password: Option<String>,
    key_path: Option<String>,
    key_passphrase: Option<String>,
) -> Result<AuthParams, String> {
    // The frontend currently picks one primary method, but the backend
    // cascades through key → agent → password regardless. Populating optional
    // fields here lets a session that was saved with a key still fall back to
    // an entered password (and vice versa) without any UI gymnastics.
    let mut auth = AuthParams { allow_agent: true, ..Default::default() };
    match auth_method {
        "password" => {
            auth.password = Some(password.ok_or("Password required for password auth")?);
        }
        "key" => {
            auth.key = Some(crate::ssh::client::KeyAuth {
                path: key_path.ok_or("Key path required for key auth")?,
                passphrase: key_passphrase,
            });
            // Allow callers to also pass a password as a fallback.
            auth.password = password.filter(|p| !p.is_empty());
        }
        "agent" => {
            // Just use ssh-agent (allow_agent is already true).
        }
        _ => return Err(format!("Unknown auth method: {}", auth_method)),
    }
    Ok(auth)
}

#[tauri::command]
pub async fn ssh_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
    host: String,
    port: u16,
    username: String,
    auth_method: String,
    password: Option<String>,
    key_path: Option<String>,
    key_passphrase: Option<String>,
    cols: u16,
    rows: u16,
    jump_chain: Option<Vec<JumpHostConnectParams>>,
    proxy: Option<crate::state::ProxyConfig>,
    color_init: Option<bool>,
) -> Result<String, String> {
    tracing::info!(
        "ssh_connect IPC: id={}, host={}, port={}, user={}, auth_method='{}', has_key_path={}, has_password={}, has_passphrase={}, has_proxy={}, has_jump={}, color_init={}",
        id, host, port, username, auth_method,
        key_path.as_deref().map(|p| !p.is_empty()).unwrap_or(false),
        password.as_deref().map(|p| !p.is_empty()).unwrap_or(false),
        key_passphrase.as_deref().map(|p| !p.is_empty()).unwrap_or(false),
        proxy.is_some(),
        jump_chain.as_ref().map(|c| c.len()).unwrap_or(0),
        color_init.unwrap_or(true),
    );
    let auth = build_auth(&auth_method, password, key_path, key_passphrase)?;

    let color_init = color_init.unwrap_or(true);

    let mut manager = state.ssh_manager.lock().await;
    let pending_host_keys = state.pending_host_keys.clone();
    let known_hosts = state.known_hosts.clone();

    let info = if let Some(chain) = jump_chain {
        if chain.is_empty() {
            // No jump hosts, connect directly
            manager
                .connect(&id, &host, port, &username, auth, cols, rows, color_init, app.clone(), proxy, pending_host_keys, known_hosts)
                .await
                .map_err(|e| e.to_string())?
        } else {
            // Build jump host params
            let jump_params: Result<Vec<JumpHostParams>, String> = chain
                .into_iter()
                .map(|j| {
                    let jauth = build_auth(
                        &j.auth_method,
                        j.password,
                        j.key_path,
                        j.key_passphrase,
                    )?;
                    Ok(JumpHostParams {
                        host: j.host,
                        port: j.port,
                        username: j.username,
                        auth: jauth,
                    })
                })
                .collect();

            manager
                .connect_via_jump(
                    &id,
                    &host,
                    port,
                    &username,
                    auth,
                    jump_params?,
                    cols,
                    rows,
                    color_init,
                    app.clone(),
                        pending_host_keys,
                        known_hosts,
                    )
                .await
                .map_err(|e| e.to_string())?
        }
    } else {
        manager
            .connect(&id, &host, port, &username, auth, cols, rows, color_init, app.clone(), proxy, pending_host_keys, known_hosts)
            .await
            .map_err(|e| e.to_string())?
    };

    let connection_id = info.id.clone();

    // Drop ssh_manager lock BEFORE dispatching plugin hooks
    // (plugins may call reach.ssh.exec which needs the lock)
    drop(manager);

    // Fire-and-forget: a slow or hung plugin hook must not block the IPC
    // return. dispatch_hook applies a per-hook timeout internally.
    let hook = hooks::session_connected(&connection_id, &host, &username);
    let plugin_mgr = state.plugin_manager.clone();
    let app_for_hook = app.clone();
    tokio::spawn(async move {
        let mut mgr = plugin_mgr.lock().await;
        mgr.dispatch_hook(&hook, Some(&app_for_hook)).await;
    });

    Ok(connection_id)
}

#[tauri::command]
pub async fn ssh_send(
    state: tauri::State<'_, AppState>,
    connection_id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    let manager = state.ssh_manager.lock().await;
    manager.send_data(&connection_id, &data).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ssh_resize(
    state: tauri::State<'_, AppState>,
    connection_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let manager = state.ssh_manager.lock().await;
    manager.resize(&connection_id, cols, rows).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ssh_disconnect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    connection_id: String,
) -> Result<(), String> {
    let mut manager = state.ssh_manager.lock().await;
    manager.disconnect(&connection_id).map_err(|e| e.to_string())?;
    drop(manager);

    // Fire-and-forget hook dispatch (see ssh_connect for rationale).
    let hook = hooks::session_disconnected(&connection_id);
    let plugin_mgr = state.plugin_manager.clone();
    let app_for_hook = app.clone();
    tokio::spawn(async move {
        let mut mgr = plugin_mgr.lock().await;
        mgr.dispatch_hook(&hook, Some(&app_for_hook)).await;
    });

    Ok(())
}

#[tauri::command]
pub async fn ssh_list_connections(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ConnectionInfo>, String> {
    let manager = state.ssh_manager.lock().await;
    Ok(manager.list_connections())
}

/// Signal the session task that the frontend event listener is registered.
/// Must be called after the Terminal component sets up its `ssh-data-*`
/// listener so that buffered MOTD / system-info output can be delivered.
#[tauri::command]
pub async fn ssh_mark_ready(
    state: tauri::State<'_, AppState>,
    connection_id: String,
) -> Result<(), String> {
    let manager = state.ssh_manager.lock().await;
    manager.mark_ready(&connection_id).map_err(|e| e.to_string())
}

/// Detect the remote operating system by parsing /etc/os-release.
/// Returns a lowercase distro ID (e.g. "debian", "ubuntu", "alpine")
/// or a fallback from uname -s (e.g. "darwin", "freebsd").
#[tauri::command]
pub async fn ssh_detect_os(
    state: tauri::State<'_, AppState>,
    connection_id: String,
) -> Result<String, String> {
    let handle = {
        let manager = state.ssh_manager.lock().await;
        manager.get_handle(&connection_id).map_err(|e| e.to_string())?
    };

    // Try /etc/os-release first (standard on modern Linux)
    if let Ok(output) = exec_on_connection(&handle, "cat /etc/os-release 2>/dev/null").await {
        for line in output.lines() {
            // Match the ID= line (not ID_LIKE=)
            if let Some(rest) = line.strip_prefix("ID=") {
                let id = rest.trim().trim_matches('"').to_lowercase();
                if !id.is_empty() {
                    return Ok(id);
                }
            }
        }
    }

    // Fallback: uname -s for non-Linux systems
    if let Ok(output) = exec_on_connection(&handle, "uname -s 2>/dev/null").await {
        let os = output.trim().to_lowercase();
        if !os.is_empty() {
            return Ok(os);
        }
    }

    Ok("linux".to_string())
}

/// Called by the frontend after the user makes a decision on a host key
/// verification dialog. Delivers the decision to the waiting oneshot channel
/// inside SshClientHandler::check_server_key.
#[tauri::command]
pub async fn ssh_confirm_host_key(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    decision: String,
) -> Result<(), String> {
    let host_id = format!("{}:{}", host, port);

    let decision = match decision.as_str() {
        "accept" => HostKeyDecision::Accept,
        "accept-once" => HostKeyDecision::AcceptOnce,
        "reject" => HostKeyDecision::Reject,
        _ => return Err(format!("Invalid decision: {}", decision)),
    };

    let mut pending_map = state.pending_host_keys.lock().await;
    if let Some(senders) = pending_map.remove(&host_id) {
        for tx in senders {
            let _ = tx.send(decision.clone());
        }
        Ok(())
    } else {
        Err("No pending host key verification for this host".to_string())
    }
}
