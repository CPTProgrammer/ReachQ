//! Per-connection negotiation between the real SFTP protocol and the
//! shell-exec fallback for hosts without an SFTP subsystem.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use russh::ChannelMsg;
use russh_sftp::client::rawsession::{Limits, RawSftpSession};
use russh_sftp::extensions;
use tracing::{info, warn};

use crate::ssh::client::SharedHandle;

/// Timeout waiting for the server's reply to the subsystem request.
const SUBSYSTEM_REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Timeout for the SFTP protocol version handshake once the subsystem
/// has been accepted.
const INIT_TIMEOUT: Duration = Duration::from_secs(10);
/// Per-request size ceiling when the server does not advertise
/// `limits@openssh.com`. Matches russh-sftp's own default (255 KiB).
const DEFAULT_PACKET_LEN: u32 = 261120;

/// A live SFTP protocol session on one SSH connection.
///
/// Wraps the raw (request/response) session rather than the high-level
/// `SftpSession`: the high-level `File` API is stop-and-wait (one request
/// in flight), which caps throughput at ~chunk/RTT. The raw session
/// multiplexes requests by id, so transfers pipeline multiple requests
/// and saturate the SSH channel window instead.
pub struct SftpProtocol {
    pub session: Arc<RawSftpSession>,
    /// Max bytes per read request (server limit or DEFAULT_PACKET_LEN).
    pub read_len: u32,
    /// Max bytes per write request (server limit or DEFAULT_PACKET_LEN).
    pub write_len: u32,
}

/// How remote file operations are carried out for a connection.
pub enum SftpBackend {
    /// SFTP subsystem via russh-sftp. The session multiplexes concurrent
    /// requests (matched by request id), so one session is shared by the
    /// file browser, transfers and any future consumers.
    Protocol(Arc<SftpProtocol>),
    /// Shell commands over SSH exec channels (`ls`, `stat`, `base64`, ...).
    Exec,
}

impl SftpBackend {
    pub fn protocol(&self) -> Option<&Arc<SftpProtocol>> {
        match self {
            Self::Protocol(session) => Some(session),
            Self::Exec => None,
        }
    }
}

/// Caches the negotiated backend per SSH connection so the SFTP handshake
/// happens at most once per connection.
pub struct SftpBackendManager {
    backends: HashMap<String, Arc<SftpBackend>>,
}

impl SftpBackendManager {
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
        }
    }

    /// Return the cached backend, or probe the connection: try the SFTP
    /// subsystem, fall back to shell exec when unavailable.
    ///
    /// Only a working SFTP session is cached. The exec fallback is NOT
    /// cached, so the next operation re-probes (a cheap ~1 RTT handshake)
    /// and a transient failure cannot pin the connection to exec forever.
    pub async fn get(&mut self, connection_id: &str, handle: &SharedHandle) -> Arc<SftpBackend> {
        if let Some(backend) = self.backends.get(connection_id) {
            return backend.clone();
        }

        match probe_sftp(handle).await {
            Ok(protocol) => {
                info!("connection {}: SFTP subsystem available", connection_id);
                let backend = Arc::new(SftpBackend::Protocol(Arc::new(protocol)));
                self.backends.insert(connection_id.to_string(), backend.clone());
                backend
            }
            Err(reason) => {
                warn!(
                    "connection {}: SFTP unavailable ({}), falling back to shell exec",
                    connection_id, reason
                );
                Arc::new(SftpBackend::Exec)
            }
        }
    }

    /// Drop the cached backend (on disconnect/reconnect, or when an
    /// operation discovers the session died) so the next operation
    /// re-probes the connection.
    pub fn invalidate(&mut self, connection_id: &str) {
        if let Some(backend) = self.backends.remove(connection_id) {
            if let SftpBackend::Protocol(protocol) = &*backend {
                // Signals the session's writer task to shut down the stream.
                let _ = protocol.session.close_session();
            }
        }
    }
}

/// Open a session channel, request the `sftp` subsystem and initialize the
/// SFTP protocol over it.
async fn probe_sftp(handle: &SharedHandle) -> Result<SftpProtocol, String> {
    let mut channel = {
        let guard = handle.lock().await;
        guard
            .channel_open_session()
            .await
            .map_err(|e| format!("channel open failed: {}", e))?
    };

    // russh's request_subsystem() is fire-and-forget even with
    // want_reply=true (it never awaits the reply), so explicitly wait for
    // CHANNEL_SUCCESS / CHANNEL_FAILURE here. OpenSSH keeps the channel
    // open when rejecting a subsystem, so without this wait a missing
    // subsystem is indistinguishable from a slow server until timeout.
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| format!("subsystem request failed: {}", e))?;

    let accepted = tokio::time::timeout(SUBSYSTEM_REPLY_TIMEOUT, async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => break true,
                // Failure = rejected; None = channel closed by the server
                Some(ChannelMsg::Failure) | None => break false,
                // WindowAdjust and friends: keep waiting
                Some(_) => {}
            }
        }
    })
    .await
    .map_err(|_| "subsystem reply timed out".to_string())?;

    if !accepted {
        return Err("server rejected the sftp subsystem".to_string());
    }

    let mut session = RawSftpSession::new(channel.into_stream());

    // Generous per-request timeout (default is 10s): pipelined requests may
    // legitimately sit queued behind a full SSH channel window on slow links.
    session.set_timeout(60).await;

    let version = match tokio::time::timeout(INIT_TIMEOUT, session.init()).await {
        Ok(Ok(version)) => version,
        Ok(Err(e)) => {
            let _ = session.close_session();
            return Err(format!("sftp init failed: {}", e));
        }
        Err(_) => {
            let _ = session.close_session();
            return Err("sftp init timed out".to_string());
        }
    };

    // Negotiate request size limits if the server advertises them.
    let mut read_len = DEFAULT_PACKET_LEN;
    let mut write_len = DEFAULT_PACKET_LEN;
    if version.extensions.get(extensions::LIMITS).is_some_and(|e| e == "1") {
        if let Ok(ext) = session.limits().await {
            let limits = Limits::from(ext);
            if let Some(r) = limits.read_len {
                read_len = read_len.min(r.min(u32::MAX as u64) as u32);
            }
            if let Some(w) = limits.write_len {
                write_len = write_len.min(w.min(u32::MAX as u64) as u32);
            }
            session.set_limits(Arc::new(limits));
        }
    }

    Ok(SftpProtocol {
        session: Arc::new(session),
        read_len,
        write_len,
    })
}
