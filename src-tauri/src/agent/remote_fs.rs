//! Agent-side remote filesystem facade (design 02 §3, 03 §8).
//!
//! On top of `sftp::ops::RemoteFs` this adds:
//! - identity-based connection resolution + tool-call leases (02 §3.1)
//! - the read pipeline: stat -> cache -> raw bytes -> binary/encoding
//!   detection -> LF normalization -> fingerprint
//! - the write pipeline: re-stat fingerprint check -> line-ending restore ->
//!   encode -> round-trip verification -> write -> new fingerprint
//! - the read cache (03 §1.7) and read-before-write path set (03 §2)

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tauri::Emitter;

use crate::sftp::browser::{RemoteStat, SftpBrowserError};
use crate::sftp::ops::RemoteFs;
use crate::ssh::client::SshManager;
use crate::sftp::backend::SftpBackendManager;

use super::tools::edit_match::DiffHunk;
use super::tools::{err_text, ok_text};
use super::types::ToolResult;

/// Agent read cap (design 03 §1 behavior 2): files larger than this are
/// rejected outright and the model is guided to the terminal tool.
pub const AGENT_READ_MAX: u64 = 1024 * 1024;

// ---------------------------------------------------------------------------
// Shared caches (owned by AgentState, cloned into ToolContext)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ReadCacheEntry {
    pub text: String,
    /// encoding_rs label (e.g. "UTF-8", "GBK").
    pub encoding_label: String,
    pub line_ending: LineEnding,
    pub fingerprint: Fingerprint,
    pub fetched_at: Instant,
}

/// (identity, path) -> decoded file content.
pub type ReadCache = Arc<Mutex<HashMap<(String, String), ReadCacheEntry>>>;

/// Per-thread set of paths successfully read (read-before-write gate).
pub type ReadPaths = Arc<Mutex<HashSet<String>>>;

/// A prepared file write, stashed between the approval request (which
/// computes the diff) and the actual write after Accept. Keyed by
/// tool_call_id so the approved diff is exactly what gets written.
pub struct PreparedWrite {
    pub path: String,
    /// New full content, LF-normalized.
    pub new_text: String,
    /// Unified-diff text (LLM-facing), formatted from `hunks`.
    pub diff: String,
    /// Structured hunks of the same diff (UI payload / previews).
    pub hunks: Vec<DiffHunk>,
    /// Fingerprint at prepare time (design 03 §2 behavior 3: F1).
    pub expected: Fingerprint,
    pub encoding_label: String,
    pub line_ending: LineEnding,
}

pub type PendingWrites = Arc<Mutex<HashMap<String, PreparedWrite>>>;

pub fn new_read_cache() -> ReadCache {
    Arc::new(Mutex::new(HashMap::new()))
}

pub fn new_pending_writes() -> PendingWrites {
    Arc::new(Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Fingerprint & encoding types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fingerprint {
    Present { mtime: u64, size: u64 },
    Absent,
}

impl Fingerprint {
    pub fn from_stat(stat: &RemoteStat) -> Self {
        Self::Present {
            mtime: stat.mtime,
            size: stat.size,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

/// A remote file after the read pipeline: decoded, LF-normalized text plus
/// everything needed to write it back losslessly.
#[derive(Debug, Clone)]
pub struct DecodedFile {
    pub text: String,
    pub encoding: &'static encoding_rs::Encoding,
    pub line_ending: LineEnding,
    pub fingerprint: Fingerprint,
}

// ---------------------------------------------------------------------------
// Connection resolution with lease (design 02 §3.1)
// ---------------------------------------------------------------------------

/// A leased SSH connection. The lease releases on drop (spawned task); if
/// the connection was pending-close, the real disconnect happens then and
/// the frontend is notified so the tab can close.
pub struct ConnectionLease {
    pub connection_id: String,
    pub handle: crate::ssh::client::SharedHandle,
    ssh_manager: Arc<tokio::sync::Mutex<SshManager>>,
    sftp_backends: Arc<tokio::sync::Mutex<SftpBackendManager>>,
    app: tauri::AppHandle,
}

impl ConnectionLease {
    /// Resolve a live connection for `identity` (preferring `hint`),
    /// retrying once on a different connection of the same identity when the
    /// first candidate vanished (design 02 §3.1).
    pub async fn acquire(
        ssh_manager: &Arc<tokio::sync::Mutex<SshManager>>,
        sftp_backends: &Arc<tokio::sync::Mutex<SftpBackendManager>>,
        app: tauri::AppHandle,
        identity: &str,
        hint: Option<&str>,
    ) -> Result<Self, SftpBrowserError> {
        let mut excluded: Option<String> = None;
        for attempt in 0..2 {
            let connection_id = {
                let manager = ssh_manager.lock().await;
                let preferred = if attempt == 0 { hint } else { None };
                manager
                    .find_by_identity(identity, preferred)
                    .filter(|id| Some(id) != excluded.as_ref())
            };
            let Some(connection_id) = connection_id else {
                return Err(SftpBrowserError::NotConnected);
            };
            let leased = {
                let mut manager = ssh_manager.lock().await;
                if manager.acquire_lease(&connection_id).is_err() {
                    false // vanished between resolve and lease; retry
                } else {
                    true
                }
            };
            if !leased {
                excluded = Some(connection_id);
                continue;
            }
            match {
                let manager = ssh_manager.lock().await;
                manager.get_handle(&connection_id)
            } {
                Ok(handle) => {
                    return Ok(Self {
                        connection_id,
                        handle,
                        ssh_manager: ssh_manager.clone(),
                        sftp_backends: sftp_backends.clone(),
                        app,
                    })
                }
                Err(_) => {
                    let mut manager = ssh_manager.lock().await;
                    manager.release_lease(&connection_id);
                    if attempt == 0 {
                        excluded = Some(connection_id);
                        continue;
                    }
                    return Err(SftpBrowserError::NotConnected);
                }
            }
        }
        Err(SftpBrowserError::NotConnected)
    }
}

impl Drop for ConnectionLease {
    fn drop(&mut self) {
        let ssh_manager = self.ssh_manager.clone();
        let sftp_backends = self.sftp_backends.clone();
        let app = self.app.clone();
        let connection_id = self.connection_id.clone();
        tokio::spawn(async move {
            let closed = {
                let mut manager = ssh_manager.lock().await;
                manager.release_lease(&connection_id)
            };
            if closed {
                {
                    let mut backends = sftp_backends.lock().await;
                    backends.invalidate(&connection_id);
                }
                // Frontend closes the pending-close tab on this event.
                let _ = app.emit(
                    &format!("ssh-pending-close-done-{}", connection_id),
                    connection_id.clone(),
                );
            }
        });
    }
}

/// A RemoteFs riding on a ConnectionLease.
pub struct AgentRemoteFs {
    fs: RemoteFs,
    /// Held for its Drop (lease release); never read directly.
    #[allow(dead_code)]
    lease: ConnectionLease,
}

impl AgentRemoteFs {
    pub async fn for_identity(
        ssh_manager: &Arc<tokio::sync::Mutex<SshManager>>,
        sftp_backends: &Arc<tokio::sync::Mutex<SftpBackendManager>>,
        app: tauri::AppHandle,
        identity: &str,
        hint: Option<&str>,
    ) -> Result<Self, SftpBrowserError> {
        let lease =
            ConnectionLease::acquire(ssh_manager, sftp_backends, app, identity, hint).await?;
        let fs = RemoteFs::connect(ssh_manager, sftp_backends, &lease.connection_id)
            .await
            .map_err(|e| {
                // Drop of `lease` releases it; surface the original error.
                e
            })?;
        Ok(Self { fs, lease })
    }

    pub fn inner(&self) -> &RemoteFs {
        &self.fs
    }

    pub async fn stat(&self, path: &str) -> Result<RemoteStat, SftpBrowserError> {
        self.fs.stat(path).await
    }

    pub async fn read_bytes(&self, path: &str) -> Result<Vec<u8>, SftpBrowserError> {
        self.fs.read_bytes(path, AGENT_READ_MAX).await
    }

    pub async fn write_bytes(&self, path: &str, data: &[u8]) -> Result<(), SftpBrowserError> {
        self.fs.write_bytes(path, data).await
    }

    pub async fn list_dir(
        &self,
        path: &str,
    ) -> Result<Vec<crate::sftp::browser::RemoteEntry>, SftpBrowserError> {
        self.fs.list_dir(path).await
    }
}

/// Structured "connection lost" tool result (design 02 §3.1): fed back to
/// the model, the loop itself does not crash.
pub fn connection_lost_result(identity: &str, err: &SftpBrowserError) -> ToolResult {
    let _ = err;
    err_text(format!(
        "SSH connection to {} was lost. The user may have closed it. Ask the user to reconnect before retrying.",
        identity
    ))
}

// ---------------------------------------------------------------------------
// Read pipeline (design 03 §8)
// ---------------------------------------------------------------------------

/// Detect line endings: first newline type wins; mixed input resolves by
/// majority.
pub fn detect_line_ending(text: &str) -> LineEnding {
    let mut crlf = 0usize;
    let mut bare_lf = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            if i > 0 && bytes[i - 1] == b'\r' {
                crlf += 1;
            } else {
                bare_lf += 1;
            }
        }
        i += 1;
    }
    if crlf > bare_lf {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

/// Decode raw bytes into text: binary detection, BOM handling, encoding
/// guess for legacy encodings, LF normalization.
pub fn decode_bytes(path: &str, bytes: &[u8]) -> Result<(String, &'static encoding_rs::Encoding), ToolResult> {
    use content_inspector::{inspect, ContentType};

    match inspect(bytes) {
        ContentType::BINARY => Err(err_text(format!(
            "File appears to be binary ({}); it cannot be displayed as text.",
            path
        ))),
        ContentType::UTF_8 | ContentType::UTF_8_BOM => {
            let text = String::from_utf8_lossy(bytes);
            Ok((text.into_owned(), encoding_rs::UTF_8))
        }
        _ => {
            // UTF-16 variants and 8-bit encodings: guess with chardetng.
            let mut detector = chardetng::EncodingDetector::new();
            detector.feed(bytes, true);
            let encoding = detector.guess(None, true);
            let (cow, _, _) = encoding.decode(bytes);
            Ok((cow.into_owned(), encoding))
        }
    }
}

/// Full read: stat (fingerprint) -> read bytes -> decode -> LF normalize.
pub async fn read_pipeline(fs: &AgentRemoteFs, path: &str) -> Result<DecodedFile, ToolResult> {
    let stat = fs.stat(path).await.map_err(|e| map_stat_error(path, &e))?;
    if stat.is_dir {
        return Err(err_text(format!(
            "{} is a directory, not a file. Use the list_directory tool to explore directory contents.",
            path
        )));
    }
    if stat.size > AGENT_READ_MAX {
        return Err(err_text(format!(
            "File is too large to read directly ({} bytes). Use the terminal tool to inspect it, e.g. `grep`, or read specific line ranges if you know them.",
            stat.size
        )));
    }
    let bytes = fs
        .read_bytes(path)
        .await
        .map_err(|e| map_read_error(path, &e))?;
    let (text, encoding) = decode_bytes(path, &bytes)?;
    let line_ending = detect_line_ending(&text);
    let text = if line_ending == LineEnding::CrLf {
        text.replace("\r\n", "\n")
    } else {
        text
    };
    Ok(DecodedFile {
        text,
        encoding,
        line_ending,
        fingerprint: Fingerprint::from_stat(&stat),
    })
}

/// Write with fingerprint race protection and lossless encoding
/// verification (design 03 §2 behavior 3-4, §8).
pub async fn write_pipeline(
    fs: &AgentRemoteFs,
    path: &str,
    new_text_lf: &str,
    expected: Fingerprint,
    encoding: &'static encoding_rs::Encoding,
    line_ending: LineEnding,
) -> Result<(Fingerprint, usize), ToolResult> {
    // 1. Re-stat: abort when the file changed between preview and write.
    let current = match fs.stat(path).await {
        Ok(stat) => Fingerprint::from_stat(&stat),
        Err(SftpBrowserError::PathNotFound(_)) => Fingerprint::Absent,
        Err(e) => return Err(err_text(format!("Could not stat {} before writing: {}", path, e))),
    };
    if current != expected {
        return Err(err_text(
            "The file changed on disk between preview and write (mtime or size differs). Aborted. Please re-read the file and retry.",
        ));
    }

    // 2. Restore original line endings.
    let restored = if line_ending == LineEnding::CrLf {
        new_text_lf.replace('\n', "\r\n")
    } else {
        new_text_lf.to_string()
    };

    // 3. Encode in the original encoding; reject unrepresentable chars.
    let (bytes, _, had_errors) = encoding.encode(&restored);
    if had_errors {
        return Err(err_text(format!(
            "The new content contains characters that cannot be represented in the file's original encoding ({}). No changes were made.",
            encoding.name()
        )));
    }

    // 4. Round-trip verification: decode the encoded bytes back and compare.
    let (decoded_back, _, _) = encoding.decode(&bytes);
    if decoded_back != restored {
        return Err(err_text(format!(
            "Encoding round-trip verification failed for {} ({}). No changes were made.",
            path,
            encoding.name()
        )));
    }

    // 5. Write, then record the new fingerprint.
    fs.write_bytes(path, &bytes)
        .await
        .map_err(|e| err_text(format!("Failed to write {}: {}", path, e)))?;
    let new_stat = fs
        .stat(path)
        .await
        .map(|stat| Fingerprint::from_stat(&stat))
        .unwrap_or(expected);
    Ok((new_stat, bytes.len()))
}

pub fn map_stat_error(path: &str, e: &SftpBrowserError) -> ToolResult {
    match e {
        SftpBrowserError::NotConnected => err_text("Not connected to the remote host."),
        SftpBrowserError::PathNotFound(_) => err_text(format!("File not found: {}", path)),
        SftpBrowserError::PermissionDenied(_) => err_text(format!(
            "Permission denied reading {}. You may need sudo; use the terminal tool if elevated access is required.",
            path
        )),
        other => err_text(format!("Failed to stat {}: {}", path, other)),
    }
}

pub fn map_read_error(path: &str, e: &SftpBrowserError) -> ToolResult {
    match e {
        SftpBrowserError::PermissionDenied(_) => err_text(format!(
            "Permission denied reading {}. You may need sudo; use the terminal tool if elevated access is required.",
            path
        )),
        other => map_stat_error(path, other),
    }
}

/// Success text for write_file (design 03 §2 behavior 6).
pub fn wrote_result(path: &str, bytes: usize) -> ToolResult {
    ok_text(format!("Wrote {} ({} bytes).", path, bytes))
}
