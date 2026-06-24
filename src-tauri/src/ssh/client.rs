use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use russh::{Channel, ChannelMsg};
use tauri::Emitter;
use tauri::Manager;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use crate::state::ProxyConfig;

/// Expand `~` and `~/` to the user's home directory. Cross-platform: works
/// on Windows (resolves to %USERPROFILE%), macOS, and Linux. Leaves absolute
/// paths and paths without leading `~` unchanged.
fn expand_tilde(path: &str) -> PathBuf {
    let trimmed = path.trim();
    if trimmed == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(trimmed));
    }
    if let Some(rest) = trimmed.strip_prefix("~/").or_else(|| trimmed.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

async fn init_color(channel: &Channel<russh::client::Msg>) -> Result<(), SshError> {
    // Inject color initialization for remote shells that may lack color config
    // (e.g. root on Debian/Ubuntu ships with a minimal .bashrc without colors).
    // Echo is disabled at the PTY level (ECHO=0 in pty_modes) so the commands
    // below are invisible without needing `stty -echo`. We re-enable echo via
    // `stty echo` at the end so normal interactive use works afterward.
    let color_init = concat!(
        r#" export COLORTERM=truecolor; "#,
        r#"[ -z "$LS_COLORS" ] && eval "$(dircolors -b 2>/dev/null)"; "#,

        r#"if ls --color=auto -d . >/dev/null 2>&1; then "#,
        r#"alias ls='ls --color=auto' 2>/dev/null; "#,
        r#"fi; "#,

        r#"if echo | grep --color=auto "" >/dev/null 2>&1; then "#,
        r#"alias grep='grep --color=auto' 2>/dev/null; "#,
        r#"fi; "#,

        r#"if diff --color=auto /dev/null /dev/null >/dev/null 2>&1; then "#,
        r#"alias diff='diff --color=auto' 2>/dev/null; "#,
        r#"fi; "#,

        r#"if [ -n "$BASH" ]; then "#,

        r#"_n=$(($(printf '%s' "$PS1" | grep -o '\\n' 2>/dev/null | wc -l) + $(printf '%s\n' "$PS1" | wc -l))); "#,

        r#"case "$PS1" in *033*|*\\e\[*) ;; *) "#,
        r#"_c=32; [ "${EUID:-$(id -u)}" = "0" ] && _c=31; "#,
        r#"PS1="\\[\\033[01;${_c}m\\]\\u@\\h\\[\\033[00m\\]:\\[\\033[01;34m\\]\\w\\[\\033[00m\\]\\$ "; "#,
        r#"unset _c; "#,
        r#"esac; "#,

        r#"while [ $_n -gt 0 ]; do printf '\r\033[2K\033[1A'; _n=$((_n-1)); done; "#,
        r#"printf '\r\033[2K'; unset _n; "#,

        r#"fi; stty echo"#,
        "\n"
    );
    channel.data(color_init.as_bytes()).await
        .map_err(|e| SshError::ChannelError(format!("Color init failed: {}", e)))?;
    Ok(())
}

/// Attempt to authenticate via the local SSH agent (OpenSSH agent or Pageant
/// on Windows; SSH_AUTH_SOCK on Unix). Tries every identity the agent offers
/// and returns Ok(true) on the first one the server accepts. Returns Ok(false)
/// if no agent identity is accepted, or Err if the agent is unreachable.
/// Cascade through the available auth methods in OpenSSH order: configured
/// public key → ssh-agent identities → password. Returns Ok(true) when the
/// server accepts a method, Ok(false) when every available method is rejected.
/// Returns Err only on hard transport-level errors; all "auth was tried but
/// rejected" outcomes resolve to Ok(false) so the caller can decide what to
/// do (e.g. surface a password fallback prompt to the user).
async fn cascade_authenticate(
    handle: &mut russh::client::Handle<SshClientHandler>,
    username: &str,
    auth: &AuthParams,
) -> Result<bool, SshError> {
    // 1. Configured private key (file).
    if let Some(key_auth) = &auth.key {
        let expanded = expand_tilde(&key_auth.path);
        tracing::info!(
            "SSH key auth: loading key from '{}' (raw input: '{}')",
            expanded.display(),
            key_auth.path
        );
        let key = russh_keys::load_secret_key(&expanded, key_auth.passphrase.as_deref())
            .map_err(|e| {
                tracing::error!("SSH key load failed for '{}': {}", expanded.display(), e);
                SshError::ConnectionFailed(format!(
                    "Key load error: {} (path: {})",
                    e,
                    expanded.display()
                ))
            })?;
        tracing::info!(
            "SSH key loaded successfully, attempting publickey auth as '{}'",
            username
        );
        let accepted = handle
            .authenticate_publickey(username, Arc::new(key))
            .await
            .map_err(|e| {
                tracing::error!("SSH publickey auth error: {}", e);
                SshError::ConnectionFailed(format!("Auth error: {}", e))
            })?;
        tracing::info!("SSH publickey auth result: {}", accepted);
        if accepted {
            return Ok(true);
        }
    }

    // 2. ssh-agent identities (auto-detected: OpenSSH agent / Pageant / SSH_AUTH_SOCK).
    if auth.allow_agent {
        match try_agent_auth(handle, username.to_string()).await {
            Ok(true) => return Ok(true),
            Ok(false) => tracing::info!("SSH agent: no identity accepted"),
            Err(e) => tracing::info!("SSH agent fallback skipped: {}", e),
        }
    }

    // 3. Password.
    if let Some(password) = &auth.password {
        tracing::info!(
            "SSH password auth: attempting as '{}' (password length: {})",
            username,
            password.len()
        );
        let accepted = handle
            .authenticate_password(username, password)
            .await
            .map_err(|e| {
                tracing::error!("SSH password auth error: {}", e);
                SshError::ConnectionFailed(format!("Auth error: {}", e))
            })?;
        tracing::info!("SSH password auth result: {}", accepted);
        if accepted {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Try every identity from the local SSH agent. Returns Ok(true) on the first
/// identity accepted by the server, Ok(false) if none are accepted, or Err if
/// the agent is unreachable or holds no keys. Cross-platform: uses OpenSSH's
/// Windows named pipe / Pageant on Windows; SSH_AUTH_SOCK on Unix.
async fn try_agent_auth(
    handle: &mut russh::client::Handle<SshClientHandler>,
    username: String,
) -> Result<bool, String> {
    #[cfg(unix)]
    {
        let agent = russh_keys::agent::client::AgentClient::connect_env()
            .await
            .map_err(|e| format!("ssh-agent unavailable (SSH_AUTH_SOCK): {}", e))?;
        try_agent_auth_inner(handle, username, agent).await
    }
    #[cfg(windows)]
    {
        // Try OpenSSH for Windows agent named pipe first (most common on Win10+).
        match russh_keys::agent::client::AgentClient::connect_named_pipe(
            r"\\.\pipe\openssh-ssh-agent",
        )
        .await
        {
            Ok(agent) => try_agent_auth_inner(handle, username, agent).await,
            Err(e) => {
                tracing::debug!("OpenSSH Windows agent named pipe unavailable: {}", e);
                let pageant = russh_keys::agent::client::AgentClient::connect_pageant().await;
                try_agent_auth_inner(handle, username, pageant).await
            }
        }
    }
}

async fn try_agent_auth_inner<S>(
    handle: &mut russh::client::Handle<SshClientHandler>,
    username: String,
    mut agent: russh_keys::agent::client::AgentClient<S>,
) -> Result<bool, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| format!("agent request_identities failed: {}", e))?;
    if identities.is_empty() {
        return Err("ssh agent is reachable but holds no identities".into());
    }
    tracing::info!(
        "SSH agent offers {} identit{}",
        identities.len(),
        if identities.len() == 1 { "y" } else { "ies" }
    );
    let mut current_agent = agent;
    for (idx, key) in identities.into_iter().enumerate() {
        tracing::info!(
            "SSH agent: trying identity #{} (type: {})",
            idx + 1,
            key.name()
        );
        let (returned, result) = handle
            .authenticate_future(username.clone(), key, current_agent)
            .await;
        current_agent = returned;
        match result {
            Ok(true) => {
                tracing::info!("SSH agent: identity #{} accepted by server", idx + 1);
                return Ok(true);
            }
            Ok(false) => {
                tracing::info!("SSH agent: identity #{} rejected by server", idx + 1);
            }
            Err(e) => {
                tracing::warn!("SSH agent: identity #{} signing error: {:?}", idx + 1, e);
            }
        }
    }
    Ok(false)
}

/// A shared, clonable wrapper around the russh Handle.
/// Handle is not Clone, so we wrap it in Arc<Mutex<>> for reuse.
pub type SharedHandle = Arc<tokio::sync::Mutex<russh::client::Handle<SshClientHandler>>>;

#[derive(Debug, Error)]
pub enum SshError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Authentication rejected — server did not accept the public key (not in authorized_keys?) or password is wrong")]
    AuthFailed,
    #[error("Channel error: {0}")]
    ChannelError(String),
    #[error("Connection not found: {0}")]
    NotFound(String),
    #[error("Send error: {0}")]
    SendError(String),
}

enum SessionCommand {
    Data(Vec<u8>),
    Resize { cols: u32, rows: u32 },
    Close,
    /// Frontend signals that the event listener is registered and buffered
    /// data can be flushed. Before this, all channel output is held in a
    /// Vec to avoid losing MOTD / system info emitted before setup completes.
    Ready,
}

/// Cascading SSH auth parameters. Each field is optional and tried in order:
/// configured key → ssh-agent identities → password. The first method the
/// server accepts wins. This mirrors OpenSSH's progressive auth — `ssh root@h`
/// without an `IdentitiesOnly yes` will try every loaded identity, then prompt
/// for a password if all fail.
#[derive(Debug, Clone, Default)]
pub struct AuthParams {
    pub key: Option<KeyAuth>,
    pub password: Option<String>,
    pub allow_agent: bool,
}

#[derive(Debug, Clone)]
pub struct KeyAuth {
    pub path: String,
    pub passphrase: Option<String>,
}

impl AuthParams {
    pub fn from_password(password: String) -> Self {
        Self { password: Some(password), allow_agent: true, ..Default::default() }
    }

    pub fn from_key(path: String, passphrase: Option<String>) -> Self {
        Self { key: Some(KeyAuth { path, passphrase }), allow_agent: true, ..Default::default() }
    }

    pub fn from_agent() -> Self {
        Self { allow_agent: true, ..Default::default() }
    }
}

/// Parameters for a single jump host in a proxy chain.
#[derive(Debug, Clone)]
pub struct JumpHostParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthParams,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConnectionInfo {
    pub id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
}

struct ActiveConnection {
    cmd_tx: mpsc::UnboundedSender<SessionCommand>,
    info: ConnectionInfo,
    handle: SharedHandle,
    /// Keep intermediate jump host sessions alive for the lifetime of this connection.
    /// These are intentionally stored but never directly read — dropping them closes the tunnels.
    #[allow(dead_code)]
    jump_handles: Vec<SharedHandle>,
}

/// Decision returned by the frontend after the user confirms a host key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostKeyDecision {
    /// Accept and save the fingerprint to known_hosts.
    Accept,
    /// Accept for this session only, do not persist.
    AcceptOnce,
    /// Reject the connection.
    Reject,
}

/// Payload emitted to the frontend when a host key needs verification.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyVerifyEvent {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub key_type: String,
    /// True for a brand-new host, false when the key has changed.
    pub is_new: bool,
    /// The previously saved fingerprint (only set when is_new is false).
    pub old_fingerprint: Option<String>,
    /// Unix timestamp in milliseconds when the SSH connection will time out.
    /// The frontend uses this to show a countdown in the host-key dialog.
    pub deadline_ms: u64,
}

pub struct SshManager {
    connections: HashMap<String, ActiveConnection>,
}

impl SshManager {
    pub fn new() -> Self {
        Self {
            connections: HashMap::new(),
        }
    }

    pub async fn connect(
        &mut self,
        id: &str,
        host: &str,
        port: u16,
        username: &str,
        auth: AuthParams,
        cols: u16,
        rows: u16,
        color_init: bool,
        app_handle: tauri::AppHandle,
        proxy: Option<ProxyConfig>,
        pending_host_keys: Arc<tokio::sync::Mutex<HashMap<String, Vec<oneshot::Sender<HostKeyDecision>>>>>,
        known_hosts: Arc<tokio::sync::RwLock<KnownHosts>>,
    ) -> Result<ConnectionInfo, SshError> {
        tracing::info!("SSH connecting to {}@{}:{}", username, host, port);

        let timeout_duration = std::time::Duration::from_secs(15);
        let deadline_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
            + timeout_duration.as_millis() as u64;
        let connect_future = async {
            let config = Arc::new(russh::client::Config::default());
            let handler = SshClientHandler::new(
                host,
                port,
                app_handle.clone(),
                pending_host_keys.clone(),
                known_hosts.clone(),
                deadline_ms,
            );

            let mut handle = if let Some(ref proxy) = proxy {
                tracing::info!("SSH connecting via {} proxy {}:{}", proxy.proxy_type, proxy.host, proxy.port);
                let stream = Self::connect_via_proxy(proxy, host, port).await?;
                russh::client::connect_stream(config, stream, handler)
                    .await
                    .map_err(|e| SshError::ConnectionFailed(format!("Proxy SSH handshake failed: {}", e)))?
            } else {
                russh::client::connect(config, (host, port), handler)
                    .await
                    .map_err(|e| SshError::ConnectionFailed(format!("{}", e)))?
            };

            // Authenticate using a cascading strategy: configured key → agent → password.
            // The first method the server accepts wins. Mirrors OpenSSH's progressive auth.
            Self::authenticate_handle(&mut handle, username, &auth).await?;

            tracing::info!("SSH authenticated for {}@{}:{}", username, host, port);

            Ok(handle)
        };

        let handle = tokio::time::timeout(timeout_duration, connect_future)
            .await
            .map_err(|_| SshError::ConnectionFailed("Connection timed out".into()))??;

        self.open_session_and_register(id, host, port, username, handle, cols, rows, color_init, app_handle, Vec::new()).await
    }

    /// Connect to a target host through one or more jump hosts (ProxyJump).
    /// `jump_chain` is ordered outermost-first: connect to first hop, then tunnel through.
    /// Connect to a target host through a SOCKS5/SOCKS4/HTTP proxy.
    async fn connect_via_proxy(
        proxy: &ProxyConfig,
        target_host: &str,
        target_port: u16,
    ) -> Result<tokio::net::TcpStream, SshError> {
        let proxy_addr = format!("{}:{}", proxy.host, proxy.port);
        let target_addr = (target_host, target_port);

        match proxy.proxy_type.to_lowercase().as_str() {
            "socks5" => {
                let stream = if let (Some(user), Some(pass)) = (&proxy.username, &proxy.password) {
                    tokio_socks::tcp::Socks5Stream::connect_with_password(
                        proxy_addr.as_str(),
                        target_addr,
                        user.as_str(),
                        pass.as_str(),
                    )
                    .await
                    .map_err(|e| SshError::ConnectionFailed(format!("SOCKS5 proxy error: {}", e)))?
                } else {
                    tokio_socks::tcp::Socks5Stream::connect(
                        proxy_addr.as_str(),
                        target_addr,
                    )
                    .await
                    .map_err(|e| SshError::ConnectionFailed(format!("SOCKS5 proxy error: {}", e)))?
                };
                Ok(stream.into_inner())
            }
            "socks4" => {
                let stream = tokio_socks::tcp::Socks4Stream::connect(
                    proxy_addr.as_str(),
                    target_addr,
                )
                .await
                .map_err(|e| SshError::ConnectionFailed(format!("SOCKS4 proxy error: {}", e)))?;
                Ok(stream.into_inner())
            }
            "http" => {
                // HTTP CONNECT proxy
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut stream = tokio::net::TcpStream::connect(&proxy_addr)
                    .await
                    .map_err(|e| SshError::ConnectionFailed(format!("HTTP proxy connect error: {}", e)))?;

                let connect_req = if let (Some(user), Some(pass)) = (&proxy.username, &proxy.password) {
                    let creds = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, format!("{}:{}", user, pass));
                    format!(
                        "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\nProxy-Authorization: Basic {}\r\n\r\n",
                        target_host, target_port, target_host, target_port, creds
                    )
                } else {
                    format!(
                        "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n\r\n",
                        target_host, target_port, target_host, target_port
                    )
                };

                stream.write_all(connect_req.as_bytes()).await
                    .map_err(|e| SshError::ConnectionFailed(format!("HTTP proxy write error: {}", e)))?;

                let mut buf = [0u8; 1024];
                let n = stream.read(&mut buf).await
                    .map_err(|e| SshError::ConnectionFailed(format!("HTTP proxy read error: {}", e)))?;
                let response = String::from_utf8_lossy(&buf[..n]);

                if !response.contains("200") {
                    return Err(SshError::ConnectionFailed(format!("HTTP proxy rejected: {}", response.lines().next().unwrap_or(""))));
                }

                Ok(stream)
            }
            _ => Err(SshError::ConnectionFailed(format!("Unsupported proxy type: {}", proxy.proxy_type))),
        }
    }

    pub async fn connect_via_jump(
        &mut self,
        id: &str,
        target_host: &str,
        target_port: u16,
        target_username: &str,
        target_auth: AuthParams,
        jump_chain: Vec<JumpHostParams>,
        cols: u16,
        rows: u16,
        color_init: bool,
        app_handle: tauri::AppHandle,
        pending_host_keys: Arc<tokio::sync::Mutex<HashMap<String, Vec<oneshot::Sender<HostKeyDecision>>>>>,
        known_hosts: Arc<tokio::sync::RwLock<KnownHosts>>,
    ) -> Result<ConnectionInfo, SshError> {
        tracing::info!(
            "SSH connecting to {}@{}:{} via {} jump host(s)",
            target_username, target_host, target_port, jump_chain.len()
        );

        let timeout_duration = std::time::Duration::from_secs(30);
        let deadline_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
            + timeout_duration.as_millis() as u64;
        let connect_future = async {
            let mut jump_handles: Vec<SharedHandle> = Vec::new();

            // Step 1: Connect to the first jump host directly
            let first_jump = &jump_chain[0];
            let config = Arc::new(russh::client::Config::default());
            let handler = SshClientHandler::new(
                first_jump.host.as_str(),
                first_jump.port,
                app_handle.clone(),
                pending_host_keys.clone(),
                known_hosts.clone(),
                deadline_ms,
            );

            let mut current_handle = russh::client::connect(
                config,
                (first_jump.host.as_str(), first_jump.port),
                handler,
            )
            .await
            .map_err(|e| {
                SshError::ConnectionFailed(format!(
                    "Jump host {} connection failed: {}",
                    first_jump.host, e
                ))
            })?;

            // Authenticate on first jump host
            Self::authenticate_handle(&mut current_handle, &first_jump.username, &first_jump.auth)
                .await?;

            tracing::info!("Authenticated on jump host {}", first_jump.host);

            // Step 2: Chain through remaining jump hosts or tunnel to target
            if jump_chain.len() > 1 {
                let shared = Arc::new(tokio::sync::Mutex::new(current_handle));
                jump_handles.push(shared.clone());

                let mut prev_shared = shared;

                for i in 1..jump_chain.len() {
                    let next_jump = &jump_chain[i];

                    // Open direct-tcpip channel to next hop through current handle
                    let channel = {
                        let guard = prev_shared.lock().await;
                        guard
                            .channel_open_direct_tcpip(
                                &next_jump.host,
                                next_jump.port as u32,
                                "127.0.0.1",
                                0,
                            )
                            .await
                            .map_err(|e| {
                                SshError::ConnectionFailed(format!(
                                    "Failed to open tunnel to {}: {}",
                                    next_jump.host, e
                                ))
                            })?
                    };

                    let stream = channel.into_stream();
                    let config = Arc::new(russh::client::Config::default());
                    let handler = SshClientHandler::new(
                        next_jump.host.as_str(),
                        next_jump.port,
                        app_handle.clone(),
                        pending_host_keys.clone(),
                        known_hosts.clone(),
                        deadline_ms,
                    );

                    let mut next_handle =
                        russh::client::connect_stream(config, stream, handler)
                            .await
                            .map_err(|e| {
                                SshError::ConnectionFailed(format!(
                                    "SSH over tunnel to {} failed: {}",
                                    next_jump.host, e
                                ))
                            })?;

                    Self::authenticate_handle(
                        &mut next_handle,
                        &next_jump.username,
                        &next_jump.auth,
                    )
                    .await?;

                    tracing::info!("Authenticated on jump host {}", next_jump.host);

                    let next_shared = Arc::new(tokio::sync::Mutex::new(next_handle));
                    jump_handles.push(next_shared.clone());
                    prev_shared = next_shared;
                }

                // Now open a tunnel from the last jump host to the target
                let channel = {
                    let guard = prev_shared.lock().await;
                    guard
                        .channel_open_direct_tcpip(
                            target_host,
                            target_port as u32,
                            "127.0.0.1",
                            0,
                        )
                        .await
                        .map_err(|e| {
                            SshError::ConnectionFailed(format!(
                                "Failed to open tunnel to target {}:{}: {}",
                                target_host, target_port, e
                            ))
                        })?
                };

                let stream = channel.into_stream();
                let config = Arc::new(russh::client::Config::default());
                let handler = SshClientHandler::new(
                    target_host,
                    target_port,
                    app_handle.clone(),
                    pending_host_keys.clone(),
                    known_hosts.clone(),
                    deadline_ms,
                );

                let mut target_handle =
                    russh::client::connect_stream(config, stream, handler)
                        .await
                        .map_err(|e| {
                            SshError::ConnectionFailed(format!(
                                "SSH to target {}:{} via jump failed: {}",
                                target_host, target_port, e
                            ))
                        })?;

                Self::authenticate_handle(
                    &mut target_handle,
                    target_username,
                    &target_auth,
                )
                .await?;

                Ok((target_handle, jump_handles))
            } else {
                // Single jump host: tunnel directly to target
                let shared = Arc::new(tokio::sync::Mutex::new(current_handle));
                jump_handles.push(shared.clone());

                let channel = {
                    let guard = shared.lock().await;
                    guard
                        .channel_open_direct_tcpip(
                            target_host,
                            target_port as u32,
                            "127.0.0.1",
                            0,
                        )
                        .await
                        .map_err(|e| {
                            SshError::ConnectionFailed(format!(
                                "Failed to open tunnel to target {}:{}: {}",
                                target_host, target_port, e
                            ))
                        })?
                };

                let stream = channel.into_stream();
                let config = Arc::new(russh::client::Config::default());
                let handler = SshClientHandler::new(
                    target_host,
                    target_port,
                    app_handle.clone(),
                    pending_host_keys.clone(),
                    known_hosts.clone(),
                    deadline_ms,
                );

                let mut target_handle =
                    russh::client::connect_stream(config, stream, handler)
                        .await
                        .map_err(|e| {
                            SshError::ConnectionFailed(format!(
                                "SSH to target {}:{} via jump failed: {}",
                                target_host, target_port, e
                            ))
                        })?;

                Self::authenticate_handle(
                    &mut target_handle,
                    target_username,
                    &target_auth,
                )
                .await?;

                Ok((target_handle, jump_handles))
            }
        };

        let (target_handle, jump_handles) =
            tokio::time::timeout(timeout_duration, connect_future)
                .await
                .map_err(|_| SshError::ConnectionFailed("Connection via jump timed out".into()))??;

        tracing::info!(
            "SSH authenticated for {}@{}:{} (via jump)",
            target_username, target_host, target_port
        );

        self.open_session_and_register(
            id, target_host, target_port, target_username,
            target_handle, cols, rows, color_init, app_handle, jump_handles,
        ).await
    }

    /// Authenticate on a russh handle by cascading through the configured
    /// methods. Thin wrapper around [`cascade_authenticate`] that returns
    /// [`SshError::AuthFailed`] when no method succeeds.
    async fn authenticate_handle(
        handle: &mut russh::client::Handle<SshClientHandler>,
        username: &str,
        auth: &AuthParams,
    ) -> Result<(), SshError> {
        if !cascade_authenticate(handle, username, auth).await? {
            return Err(SshError::AuthFailed);
        }
        Ok(())
    }

    /// Open session, request PTY and shell, then register the connection for I/O.
    /// This is the common tail shared by [`connect`] and [`connect_via_jump`].
    async fn open_session_and_register(
        &mut self,
        id: &str,
        host: &str,
        port: u16,
        username: &str,
        handle: russh::client::Handle<SshClientHandler>,
        cols: u16,
        rows: u16,
        color_init: bool,
        app_handle: tauri::AppHandle,
        jump_handles: Vec<SharedHandle>,
    ) -> Result<ConnectionInfo, SshError> {
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::ChannelError(format!("Failed to open session: {}", e)))?;

        // Disable echo in PTY modes so the color-init commands sent below
        // are invisible to the user; we re-enable echo via `stty echo` at
        // the end of the init script. When color_init is disabled there are
        // no commands to hide, so we skip the ECHO=0 mode.
        let pty_modes: &[(russh::Pty, u32)] = if color_init {
            &[(russh::Pty::ECHO, 0)]
        } else {
            &[]
        };
        channel
            .request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, pty_modes)
            .await
            .map_err(|e| SshError::ChannelError(format!("PTY request failed: {}", e)))?;

        channel
            .request_shell(false)
            .await
            .map_err(|e| SshError::ChannelError(format!("Shell request failed: {}", e)))?;

        tracing::info!("SSH shell opened for {}@{}:{}", username, host, port);

        if color_init {
            init_color(&channel).await?;
        }

        let info = ConnectionInfo {
            id: id.to_string(),
            host: host.to_string(),
            port,
            username: username.to_string(),
        };

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

        let task_id = id.to_string();
        let task_handle = app_handle.clone();
        tokio::spawn(async move {
            ssh_session_task(channel, cmd_rx, task_id, task_handle).await;
        });

        let shared_handle = Arc::new(tokio::sync::Mutex::new(handle));

        self.connections.insert(
            id.to_string(),
            ActiveConnection {
                cmd_tx,
                info: info.clone(),
                handle: shared_handle,
                jump_handles,
            },
        );

        Ok(info)
    }

    pub fn send_data(&self, id: &str, data: &[u8]) -> Result<(), SshError> {
        let conn = self.connections.get(id)
            .ok_or_else(|| SshError::NotFound(id.to_string()))?;
        conn.cmd_tx.send(SessionCommand::Data(data.to_vec()))
            .map_err(|e| SshError::SendError(format!("{}", e)))
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), SshError> {
        let conn = self.connections.get(id)
            .ok_or_else(|| SshError::NotFound(id.to_string()))?;
        conn.cmd_tx.send(SessionCommand::Resize { cols: cols as u32, rows: rows as u32 })
            .map_err(|e| SshError::SendError(format!("{}", e)))
    }

    pub fn disconnect(&mut self, id: &str) -> Result<(), SshError> {
        let conn = self.connections.remove(id)
            .ok_or_else(|| SshError::NotFound(id.to_string()))?;
        let _ = conn.cmd_tx.send(SessionCommand::Close);
        tracing::info!("SSH disconnected: {}", id);
        Ok(())
    }

    pub fn list_connections(&self) -> Vec<ConnectionInfo> {
        self.connections.values().map(|c| c.info.clone()).collect()
    }

    pub fn is_connected(&self, id: &str) -> bool {
        self.connections.contains_key(id)
    }

    pub fn get_handle(&self, id: &str) -> Result<SharedHandle, SshError> {
        self.connections.get(id)
            .map(|c| c.handle.clone())
            .ok_or_else(|| SshError::NotFound(id.to_string()))
    }

    /// Signal the session task that the frontend is ready to receive data.
    /// Causes the task to flush its buffered channel output and switch to
    /// direct forwarding mode.
    pub fn mark_ready(&self, id: &str) -> Result<(), SshError> {
        let conn = self.connections.get(id)
            .ok_or_else(|| SshError::NotFound(id.to_string()))?;
        conn.cmd_tx.send(SessionCommand::Ready)
            .map_err(|e| SshError::SendError(format!("{}", e)))
    }
}

impl Default for SshManager {
    fn default() -> Self { Self::new() }
}

pub async fn exec_on_connection(
    handle: &SharedHandle,
    command: &str,
) -> Result<String, SshError> {
    let mut channel = {
        let guard = handle.lock().await;
        guard.channel_open_session().await
            .map_err(|e| SshError::ChannelError(format!("{}", e)))?
    };
    channel.exec(true, command).await
        .map_err(|e| SshError::ChannelError(format!("{}", e)))?;
    let mut output = String::new();
    let mut got_eof = false;
    let mut got_exit = false;
    loop {
        // Timeout to avoid hanging forever
        let msg = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            channel.wait(),
        ).await;

        match msg {
            Ok(Some(ChannelMsg::Data { ref data })) => {
                output.push_str(&String::from_utf8_lossy(data));
            }
            Ok(Some(ChannelMsg::ExtendedData { .. })) => {
                // stderr — skip
            }
            Ok(Some(ChannelMsg::Eof)) => {
                got_eof = true;
                if got_exit { break; }
            }
            Ok(Some(ChannelMsg::ExitStatus { .. })) => {
                got_exit = true;
                if got_eof { break; }
            }
            Ok(None) | Err(_) => break, // channel closed or timeout
            _ => {
                // WindowAdjusted, etc.
            }
        }
    }
    Ok(output)
}

/// Execute a command on an existing SSH connection and return (stdout, stderr, exit_code).
/// Unlike `exec_on_connection`, this captures stderr separately and returns the exit code.
pub async fn exec_on_connection_with_exit_code(
    handle: &SharedHandle,
    command: &str,
) -> Result<(String, String, i32), SshError> {
    let mut channel = {
        let guard = handle.lock().await;
        guard.channel_open_session().await
            .map_err(|e| SshError::ChannelError(format!("{}", e)))?
    };
    channel.exec(true, command).await
        .map_err(|e| SshError::ChannelError(format!("{}", e)))?;

    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut exit_code: i32 = -1;
    let mut got_eof = false;
    let mut got_exit = false;

    loop {
        let msg = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            channel.wait(),
        ).await;

        match msg {
            Ok(Some(ChannelMsg::Data { ref data })) => {
                stdout.push_str(&String::from_utf8_lossy(data));
            }
            Ok(Some(ChannelMsg::ExtendedData { ref data, .. })) => {
                stderr.push_str(&String::from_utf8_lossy(data));
            }
            Ok(Some(ChannelMsg::Eof)) => {
                got_eof = true;
                if got_exit { break; }
            }
            Ok(Some(ChannelMsg::ExitStatus { exit_status })) => {
                exit_code = exit_status as i32;
                got_exit = true;
                if got_eof { break; }
            }
            Ok(None) | Err(_) => break,
            _ => {}
        }
    }

    Ok((stdout, stderr, exit_code))
}

/// Generic streaming output event used by all remote streaming commands.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StreamingOutputEvent {
    pub run_id: String,
    pub stream: String,
    pub data: String,
}

/// Streaming variant of `exec_on_connection`.
/// Emits each chunk as a `{event_prefix}-{run_id}` Tauri event.
/// Returns the exit code (defaults to -1 if not received).
pub async fn exec_on_connection_streaming(
    handle: &SharedHandle,
    command: &str,
    run_id: &str,
    event_prefix: &str,
    app_handle: &tauri::AppHandle,
) -> Result<i32, SshError> {
    let mut channel = {
        let guard = handle.lock().await;
        guard.channel_open_session().await
            .map_err(|e| SshError::ChannelError(format!("{}", e)))?
    };
    channel.exec(true, command).await
        .map_err(|e| SshError::ChannelError(format!("{}", e)))?;

    let output_event = format!("{}-{}", event_prefix, run_id);
    let mut exit_code: i32 = -1;
    let mut got_eof = false;
    let mut got_exit = false;

    loop {
        let msg = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            channel.wait(),
        ).await;

        match msg {
            Ok(Some(ChannelMsg::Data { ref data })) => {
                let text = String::from_utf8_lossy(data).to_string();
                let _ = app_handle.emit(
                    &output_event,
                    StreamingOutputEvent {
                        run_id: run_id.to_string(),
                        stream: "stdout".to_string(),
                        data: text,
                    },
                );
            }
            Ok(Some(ChannelMsg::ExtendedData { ref data, .. })) => {
                let text = String::from_utf8_lossy(data).to_string();
                let _ = app_handle.emit(
                    &output_event,
                    StreamingOutputEvent {
                        run_id: run_id.to_string(),
                        stream: "stderr".to_string(),
                        data: text,
                    },
                );
            }
            Ok(Some(ChannelMsg::Eof)) => {
                got_eof = true;
                if got_exit { break; }
            }
            Ok(Some(ChannelMsg::ExitStatus { exit_status })) => {
                exit_code = exit_status as i32;
                got_exit = true;
                if got_eof { break; }
            }
            Ok(None) | Err(_) => break,
            _ => {}
        }
    }

    Ok(exit_code)
}

#[derive(Debug, Clone)]
pub struct SshClientHandler {
    host: String,
    port: u16,
    app_handle: tauri::AppHandle,
    pending_verifications: Arc<tokio::sync::Mutex<HashMap<String, Vec<oneshot::Sender<HostKeyDecision>>>>>,
    known_hosts: Arc<tokio::sync::RwLock<KnownHosts>>,
    deadline_ms: u64,
}

impl SshClientHandler {
    pub fn new(
        host: &str,
        port: u16,
        app_handle: tauri::AppHandle,
        pending_verifications: Arc<
            tokio::sync::Mutex<HashMap<String, Vec<oneshot::Sender<HostKeyDecision>>>>,
        >,
        known_hosts: Arc<tokio::sync::RwLock<KnownHosts>>,
        deadline_ms: u64,
    ) -> Self {
        Self { host: host.into(), port, app_handle, pending_verifications, known_hosts, deadline_ms }
    }
}

/// In-memory representation of ~/.reach/ssh/known_hosts.json.
/// All SSH connections share a single instance via Arc<RwLock<>> in AppState
/// so concurrent Accept & Save operations don't overwrite each other.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownHosts {
    pub entries: HashMap<String, String>,
}

impl Default for KnownHosts {
    fn default() -> Self {
        Self { entries: HashMap::new() }
    }
}

/// Return the path to the known_hosts JSON file in the app data directory.
pub fn known_hosts_path() -> std::path::PathBuf {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.reach.app");
    data_dir.join("ssh").join("known_hosts.json")
}

#[async_trait]
impl russh::client::Handler for SshClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let host_id = format!("{}:{}", self.host, self.port);
        let fingerprint = server_public_key.fingerprint();
        let key_type = server_public_key.name();

        // Ensure parent directory exists so async persist won't fail later.
        let path = known_hosts_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // Read shared state under a read lock.
        let is_new;
        let old_fingerprint;
        {
            let known = self.known_hosts.read().await;
            if let Some(existing) = known.entries.get(&host_id) {
                if existing == &fingerprint {
                    return Ok(true); // Scenario 2: match → accept silently.
                }
            }
            is_new = !known.entries.contains_key(&host_id);
            old_fingerprint = known.entries.get(&host_id).cloned();
        }

        // Scenario 1 (new host) or 3 (fingerprint changed) → prompt the user.
        // Create a oneshot channel so we can wait for the frontend response.
        let (tx, rx) = oneshot::channel();
        let is_first = {
            let mut pending = self.pending_verifications.lock().await;
            let entry = pending.entry(host_id.clone()).or_default();
            // Clean up dead senders from previously cancelled/timeout connections
            // whose receivers were dropped, so they don't block subsequent
            // connection attempts from emitting the host-key-verify event.
            entry.retain(|s| !s.is_closed());
            let first = entry.is_empty();
            entry.push(tx);
            first
        };

        let event = HostKeyVerifyEvent {
            host: self.host.clone(),
            port: self.port,
            fingerprint: fingerprint.clone(),
            key_type: key_type.to_string(),
            is_new,
            old_fingerprint,
            deadline_ms: self.deadline_ms,
        };

        // Only emit the event once per host:port; subsequent concurrent
        // connections share the decision from this same dialog.
        if is_first {
            let _ = self.app_handle.emit("host-key-verify", &event);
        }

        // Block until the frontend sends back a decision via ssh_confirm_host_key.
        let decision = match rx.await {
            Ok(d) => d,
            Err(_) => {
                // The channel was closed (e.g. app quit). Reject.
                return Ok(false);
            }
        };

        match decision {
            HostKeyDecision::Accept => {
                // Persist the new/updated fingerprint in shared state.
                {
                    let mut known = self.known_hosts.write().await;
                    known.entries.insert(host_id, fingerprint);
                    // Persist to disk asynchronously so we don't block
                    // the async runtime on filesystem I/O.
                    if let Ok(raw) = serde_json::to_string_pretty(&*known) {
                        let path = known_hosts_path();
                        let _ = tokio::fs::write(&path, &raw).await;
                    }
                }
                tracing::info!("SSH host key {} for {}", if is_new { "saved" } else { "updated" }, self.host);
                Ok(true)
            }
            HostKeyDecision::AcceptOnce => {
                // Allow this session without saving.
                tracing::info!("SSH host key accepted (once) for {}", self.host);
                Ok(true)
            }
            HostKeyDecision::Reject => {
                tracing::info!("SSH host key rejected by user for {}", self.host);
                Ok(false)
            }
        }
    }
}

async fn ssh_session_task(
    mut channel: russh::Channel<russh::client::Msg>,
    mut cmd_rx: mpsc::UnboundedReceiver<SessionCommand>,
    connection_id: String,
    app_handle: tauri::AppHandle,
) {
    let data_event = format!("ssh-data-{}", connection_id);
    let exit_event = format!("ssh-exit-{}", connection_id);

    // Buffer channel output until the frontend sends Ready.
    // This prevents losing MOTD / system-info that the remote shell
    // emits immediately after request_shell(), before the frontend
    // has had time to create the terminal component and register
    // its Tauri event listener.
    let mut buffer: Vec<String> = Vec::new();
    let mut ready = false;

    loop {
        tokio::select! {
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        let payload = String::from_utf8_lossy(data).to_string();
                        if ready {
                            if let Err(e) = app_handle.emit(&data_event, &payload) {
                                tracing::error!("Failed to emit '{}': {}", data_event, e);
                                break;
                            }
                        } else {
                            buffer.push(payload);
                        }
                    }
                    Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                        let payload = String::from_utf8_lossy(data).to_string();
                        if ready {
                            if let Err(e) = app_handle.emit(&data_event, &payload) {
                                tracing::error!("Failed to emit '{}': {}", data_event, e);
                                break;
                            }
                        } else {
                            buffer.push(payload);
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        tracing::info!("SSH '{}' exited with status {}", connection_id, exit_status);
                        let _ = app_handle.emit(&exit_event, exit_status);
                        cleanup_dead_connection(&app_handle, &connection_id).await;
                        break;
                    }
                    Some(ChannelMsg::Eof) => {
                        tracing::info!("SSH '{}' received EOF", connection_id);
                        cleanup_dead_connection(&app_handle, &connection_id).await;
                        break;
                    }
                    None => {
                        tracing::info!("SSH '{}' channel closed", connection_id);
                        cleanup_dead_connection(&app_handle, &connection_id).await;
                        break;
                    }
                    _ => {}
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(SessionCommand::Data(data)) => {
                        if let Err(e) = channel.data(&data[..]).await {
                            tracing::error!("SSH '{}' write error: {}", connection_id, e);
                            break;
                        }
                    }
                    Some(SessionCommand::Resize { cols, rows }) => {
                        if let Err(e) = channel.window_change(cols, rows, 0, 0).await {
                            tracing::error!("SSH '{}' resize error: {}", connection_id, e);
                        }
                    }
                    Some(SessionCommand::Ready) => {
                        // Drain the entire buffer in one burst, then switch
                        // to direct forwarding for all subsequent output.
                        for payload in buffer.drain(..) {
                            if let Err(e) = app_handle.emit(&data_event, &payload) {
                                tracing::error!("Failed to emit '{}': {}", data_event, e);
                                break;
                            }
                        }
                        ready = true;
                    }
                    Some(SessionCommand::Close) | None => {
                        tracing::info!("SSH '{}' closing", connection_id);
                        let _ = channel.close().await;
                        break;
                    }
                }
            }
        }
    }

    if let Err(e) = app_handle.emit(&exit_event, ()) {
        tracing::error!("Failed to emit '{}': {}", exit_event, e);
    }
    tracing::info!("SSH '{}' session task exiting", connection_id);
}

/// Clean up a connection that died unexpectedly (network drop, server
/// timeout, etc.). Stops monitoring and removes the connection from the
/// manager so the monitoring loop doesn't keep retrying on a dead handle.
async fn cleanup_dead_connection(app_handle: &tauri::AppHandle, connection_id: &str) {
    let state = app_handle.state::<crate::state::AppState>();

    // Stop the monitoring task first — it depends on the SSH handle.
    {
        let mut collector = state.monitoring_collector.lock().await;
        collector.stop(connection_id);
    }
    {
        let mut monitoring = state.monitoring.write().await;
        monitoring.remove(connection_id);
    }

    // Remove the dead connection from SshManager.
    // `disconnect()` is safe to call even if the connection was already
    // removed — it just returns NotFound, which we ignore.
    let mut manager = state.ssh_manager.lock().await;
    let _ = manager.disconnect(connection_id);
}
