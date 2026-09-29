use std::collections::HashMap;

use russh::ChannelMsg;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};

use crate::ssh::client::SharedHandle;
use crate::state::{TunnelConfig, TunnelType};

#[derive(Debug, Error)]
pub enum TunnelError {
    #[error("Tunnel creation failed: {0}")]
    CreationFailed(String),
    #[error("Tunnel not found: {0}")]
    NotFound(String),
    #[error("Port already in use: {0}")]
    PortInUse(u16),
    #[error("Connection not found: {0}")]
    ConnectionNotFound(String),
    #[error("IO error: {0}")]
    IoError(String),
    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

/// Grace period for in-flight relays to close cleanly after a tunnel stop
/// before they are force-aborted.
const RELAY_SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// An active tunnel task with a shutdown channel.
struct ActiveTunnel {
    /// The SSH connection this tunnel rides — when it is torn down, the
    /// tunnel is stopped too (tunnels follow tabs/connections).
    connection_id: String,
    shutdown_tx: mpsc::Sender<()>,
    task: JoinHandle<()>,
}

/// Manages active SSH tunnel forwarding tasks.
pub struct TunnelManager {
    active: HashMap<String, ActiveTunnel>,
}

impl TunnelManager {
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
        }
    }

    /// Create a new tunnel configuration (does not start it yet).
    ///
    /// `session_id` makes the tunnel persistent (stored in the vault);
    /// `connection_id` binds it to a live connection for this run only.
    pub fn create_tunnel(
        tunnel_type: TunnelType,
        local_port: u16,
        remote_host: &str,
        remote_port: u16,
        session_id: Option<String>,
        connection_id: Option<String>,
    ) -> TunnelConfig {
        TunnelConfig {
            id: uuid::Uuid::new_v4().to_string(),
            tunnel_type,
            local_port,
            remote_host: remote_host.to_string(),
            remote_port,
            session_id,
            connection_id,
            active: false,
        }
    }

    /// Start a tunnel. For Local tunnels, binds a TCP listener and relays
    /// each connection through an SSH direct-tcpip channel.
    pub async fn start_tunnel(
        &mut self,
        tunnel: &TunnelConfig,
        connection_id: &str,
        handle: &SharedHandle,
    ) -> Result<(), TunnelError> {
        if self.active.contains_key(&tunnel.id) {
            return Err(TunnelError::CreationFailed(format!(
                "Tunnel {} is already running",
                tunnel.id
            )));
        }

        match tunnel.tunnel_type {
            TunnelType::Local => {
                self.start_local_forward(tunnel, connection_id, handle).await?;
            }
            TunnelType::Remote => {
                tracing::warn!("Remote port forwarding not yet implemented");
                return Err(TunnelError::NotImplemented(
                    "Remote port forwarding".to_string(),
                ));
            }
            TunnelType::Dynamic => {
                tracing::warn!("Dynamic (SOCKS) forwarding not yet implemented");
                return Err(TunnelError::NotImplemented(
                    "Dynamic (SOCKS) forwarding".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Start local port forwarding: bind locally and forward through SSH.
    async fn start_local_forward(
        &mut self,
        tunnel: &TunnelConfig,
        connection_id: &str,
        handle: &SharedHandle,
    ) -> Result<(), TunnelError> {
        let bind_addr = format!("127.0.0.1:{}", tunnel.local_port);
        let listener = TcpListener::bind(&bind_addr).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                TunnelError::PortInUse(tunnel.local_port)
            } else {
                TunnelError::IoError(format!("Failed to bind {}: {}", bind_addr, e))
            }
        })?;

        tracing::info!(
            "Tunnel {}: listening on {} -> {}:{}",
            tunnel.id,
            bind_addr,
            tunnel.remote_host,
            tunnel.remote_port
        );

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        // Broadcast to in-flight relay tasks: the tunnel is stopping.
        let (relay_shutdown_tx, relay_shutdown_rx) = watch::channel(false);
        let tunnel_id = tunnel.id.clone();
        let remote_host = tunnel.remote_host.clone();
        let remote_port = tunnel.remote_port;
        let ssh_handle = handle.clone();

        let task = tokio::spawn(async move {
            // In-flight relays are tracked so tunnel stop can shut them down
            // deterministically instead of relying on SSH channel teardown
            // timing (a relay parked in `channel.wait()` would otherwise leak
            // its TCP socket in CLOSE_WAIT).
            let mut relays = JoinSet::new();
            loop {
                tokio::select! {
                    accept_result = listener.accept() => {
                        match accept_result {
                            Ok((tcp_stream, peer_addr)) => {
                                tracing::info!(
                                    "Tunnel {}: new connection from {}",
                                    tunnel_id, peer_addr
                                );
                                let handle_clone = ssh_handle.clone();
                                let rhost = remote_host.clone();
                                let tid = tunnel_id.clone();
                                let shutdown = relay_shutdown_rx.clone();
                                relays.spawn(async move {
                                    if let Err(e) = relay_connection(
                                        tcp_stream,
                                        &handle_clone,
                                        &rhost,
                                        remote_port,
                                        shutdown,
                                    )
                                    .await
                                    {
                                        tracing::error!(
                                            "Tunnel {}: relay error: {}",
                                            tid, e
                                        );
                                    }
                                });
                                // Reap finished relays so a long-lived
                                // listener doesn't accumulate handles.
                                while relays.try_join_next().is_some() {}
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Tunnel {}: accept error: {}",
                                    tunnel_id, e
                                );
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::info!("Tunnel {}: shutdown signal received", tunnel_id);
                        // Cascade to relays: graceful close first...
                        let _ = relay_shutdown_tx.send(true);
                        // ...with a bounded grace period, then force-abort
                        // whatever is still stuck.
                        if tokio::time::timeout(RELAY_SHUTDOWN_GRACE, async {
                            while relays.join_next().await.is_some() {}
                        })
                        .await
                        .is_err()
                        {
                            tracing::warn!(
                                "Tunnel {}: aborting {} stuck relay(s)",
                                tunnel_id,
                                relays.len()
                            );
                            relays.abort_all();
                        }
                        break;
                    }
                }
            }
            tracing::info!("Tunnel {}: listener task exiting", tunnel_id);
        });

        self.active.insert(
            tunnel.id.clone(),
            ActiveTunnel {
                connection_id: connection_id.to_string(),
                shutdown_tx,
                task,
            },
        );

        Ok(())
    }

    /// Stop a running tunnel by sending a shutdown signal.
    /// Idempotent: stopping a non-running tunnel is a no-op.
    pub async fn stop_tunnel(&mut self, tunnel_id: &str) -> Result<(), TunnelError> {
        if let Some(active) = self.active.remove(tunnel_id) {
            tracing::info!("Tunnel {}: stopping", tunnel_id);
            let _ = active.shutdown_tx.send(()).await;
            // Give the task a moment to shut down gracefully
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                active.task,
            )
            .await;
        } else {
            tracing::warn!("Tunnel {}: not running", tunnel_id);
        }
        Ok(())
    }

    /// Stop all tunnels riding the given connection. Called during
    /// connection teardown; returns the stopped tunnel ids.
    pub async fn stop_tunnels_for_connection(&mut self, connection_id: &str) -> Vec<String> {
        let ids: Vec<String> = self
            .active
            .iter()
            .filter(|(_, t)| t.connection_id == connection_id)
            .map(|(id, _)| id.clone())
            .collect();
        let mut stopped = Vec::new();
        for id in ids {
            let _ = self.stop_tunnel(&id).await;
            stopped.push(id);
        }
        stopped
    }

    /// Check if a tunnel is actively running.
    pub fn is_active(&self, tunnel_id: &str) -> bool {
        self.active.contains_key(tunnel_id)
    }
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Relay data bidirectionally between a local TCP stream and an SSH direct-tcpip channel.
///
/// Uses a single task with `tokio::select!` since russh::Channel is not Clone.
/// The TCP stream is split into read/write halves; the SSH channel is driven
/// from a single owner using select over both directions.
///
/// `shutdown` fires when the owning tunnel stops: the relay closes the SSH
/// channel and the TCP write side (so a client that half-closed first does
/// not leave the socket parked in CLOSE_WAIT) and returns.
async fn relay_connection(
    tcp_stream: tokio::net::TcpStream,
    handle: &SharedHandle,
    remote_host: &str,
    remote_port: u16,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), TunnelError> {
    // Open a direct-tcpip channel through the SSH connection
    let mut channel = {
        let guard = handle.lock().await;
        guard
            .channel_open_direct_tcpip(
                remote_host,
                remote_port as u32,
                "127.0.0.1",
                0,
            )
            .await
            .map_err(|e| TunnelError::IoError(format!("Failed to open direct-tcpip channel: {}", e)))?
    };

    let (mut tcp_read, mut tcp_write) = tcp_stream.into_split();

    let mut buf = vec![0u8; 8192];
    let mut tcp_done = false;
    let mut ssh_done = false;

    // Relay loop: use select to handle both directions in a single task
    while !tcp_done || !ssh_done {
        tokio::select! {
            // Local TCP -> SSH channel (only if TCP is still readable)
            result = tcp_read.read(&mut buf), if !tcp_done => {
                match result {
                    Ok(0) => {
                        tcp_done = true;
                        let _ = channel.eof().await;
                    }
                    Ok(n) => {
                        if let Err(e) = channel.data(&buf[..n]).await {
                            tracing::debug!("Tunnel relay: write to SSH failed: {}", e);
                            tcp_done = true;
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Tunnel relay: TCP read error: {}", e);
                        tcp_done = true;
                        let _ = channel.eof().await;
                    }
                }
            }
            // SSH channel -> local TCP (only if SSH channel is still open)
            msg = channel.wait(), if !ssh_done => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        if let Err(e) = tcp_write.write_all(data).await {
                            tracing::debug!("Tunnel relay: TCP write error: {}", e);
                            ssh_done = true;
                        }
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        ssh_done = true;
                        let _ = tcp_write.shutdown().await;
                    }
                    _ => {}
                }
            }
            // Tunnel is stopping: close both sides and leave promptly.
            // `changed()` completes immediately if the signal was already
            // sent before this relay started.
            _ = shutdown.changed() => {
                tracing::debug!("Tunnel relay: shutdown requested");
                let _ = channel.close().await;
                let _ = tcp_write.shutdown().await;
                return Ok(());
            }
        }
    }

    tracing::debug!("Tunnel relay: connection closed");
    Ok(())
}
