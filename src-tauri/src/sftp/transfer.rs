use std::path::Path;
use thiserror::Error;
use serde::{Deserialize, Serialize};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use crate::ssh::client::{SharedHandle, SshError, exec_on_connection};
use super::backend::{SftpBackend, SftpProtocol};
use base64::Engine;

/// Max concurrent in-flight SFTP requests during a transfer. Sized so
/// in-flight bytes (~depth × 255 KiB) roughly fill a default 2 MiB SSH
/// channel window; deeper pipelining only adds memory pressure.
const PIPELINE_DEPTH: usize = 8;

#[derive(Debug, Error)]
pub enum TransferError {
    #[error("Not connected")]
    NotConnected,
    #[error("SSH error: {0}")]
    SshError(#[from] SshError),
    #[error("File not found: {0}")]
    FileNotFound(String),
    #[error("IO error: {0}")]
    IoError(String),
    #[error("SFTP error: {0}")]
    Sftp(String),
    #[error("Transfer cancelled")]
    Cancelled,
}

/// Progress information for an active file transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub id: String,
    pub filename: String,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub percent: f64,
}

fn remote_filename(remote_path: &str) -> String {
    Path::new(remote_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| remote_path.to_string())
}

fn local_filename(local_path: &str) -> String {
    Path::new(local_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| local_path.to_string())
}

fn progress(
    transfer_id: &str,
    filename: &str,
    bytes_transferred: u64,
    total_bytes: u64,
) -> TransferProgress {
    TransferProgress {
        id: transfer_id.to_string(),
        filename: filename.to_string(),
        bytes_transferred,
        total_bytes,
        percent: (bytes_transferred as f64 / total_bytes as f64 * 100.0).min(100.0),
    }
}

/// Map an russh-sftp error onto the transfer error type.
fn map_sftp_error(remote_path: &str, e: SftpError) -> TransferError {
    match e {
        SftpError::Status(ref s) if s.status_code == StatusCode::NoSuchFile => {
            TransferError::FileNotFound(remote_path.to_string())
        }
        other => TransferError::Sftp(other.to_string()),
    }
}

/// Download a file from the remote host. Progress is reported through the
/// `on_progress` callback; completion is signalled by the `Ok` return.
pub(crate) async fn download_file<F: Fn(TransferProgress)>(
    backend: &SftpBackend,
    handle: &SharedHandle,
    remote_path: &str,
    local_path: &str,
    transfer_id: &str,
    on_progress: F,
) -> Result<(), TransferError> {
    match backend.protocol() {
        Some(proto) => {
            download_file_sftp(proto, remote_path, local_path, transfer_id, &on_progress).await
        }
        None => download_file_exec(handle, remote_path, local_path, transfer_id, &on_progress).await,
    }
}

/// Read a whole remote file through pipelined SFTP read requests
/// (PIPELINE_DEPTH requests in flight, explicit offsets).
///
/// `on_chunk(offset, data)` is called as chunks complete — possibly out of
/// order, so the sink must seek by offset. Returns total bytes read.
pub(crate) async fn read_remote_pipelined<C, P>(
    proto: &SftpProtocol,
    remote_path: &str,
    total_bytes: u64,
    mut on_chunk: C,
    mut on_progress: P,
) -> Result<u64, TransferError>
where
    C: FnMut(u64, Vec<u8>) -> Result<(), TransferError>,
    P: FnMut(u64),
{
    use tokio::task::JoinSet;

    let handle = proto
        .session
        .open(remote_path, OpenFlags::READ, FileAttributes::empty())
        .await
        .map_err(|e| map_sftp_error(remote_path, e))?
        .handle;

    let chunk = proto.read_len as u64;
    let mut tasks: JoinSet<Result<(u64, Vec<u8>), TransferError>> = JoinSet::new();
    let mut next_offset = 0u64;
    let mut done = 0u64;
    let mut result: Result<(), TransferError> = Ok(());

    while result.is_ok() && (next_offset < total_bytes || !tasks.is_empty()) {
        while next_offset < total_bytes && tasks.len() < PIPELINE_DEPTH {
            let offset = next_offset;
            let len = chunk.min(total_bytes - offset) as u32;
            let session = proto.session.clone();
            let handle = handle.clone();
            tasks.spawn(async move {
                match session.read(handle, offset, len).await {
                    Ok(d) => Ok((offset, d.data)),
                    // File truncated on the server mid-read: stop cleanly.
                    Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => {
                        Ok((offset, Vec::new()))
                    }
                    Err(e) => Err(TransferError::Sftp(e.to_string())),
                }
            });
            next_offset += len as u64;
        }

        match tasks.join_next().await {
            Some(Ok(Ok((offset, data)))) => {
                done += data.len() as u64;
                if let Err(e) = on_chunk(offset, data) {
                    result = Err(e);
                }
                on_progress(done);
            }
            Some(Ok(Err(e))) => result = Err(e),
            Some(Err(e)) => result = Err(TransferError::IoError(format!("read task: {}", e))),
            None => break,
        }
    }

    tasks.abort_all();
    let _ = proto.session.close(handle).await;
    result.map(|_| done)
}

/// Write a whole remote file through pipelined SFTP write requests.
///
/// `next_chunk(max_len)` produces the next chunk in file order (None = EOF);
/// chunks are dispatched with explicit offsets, so in-flight completion
/// order does not matter. Returns total bytes written.
pub(crate) async fn write_remote_pipelined<N, P>(
    proto: &SftpProtocol,
    remote_path: &str,
    mut next_chunk: N,
    mut on_progress: P,
) -> Result<u64, TransferError>
where
    N: FnMut(usize) -> Result<Option<Vec<u8>>, TransferError>,
    P: FnMut(u64),
{
    use tokio::task::JoinSet;

    let handle = proto
        .session
        .open(
            remote_path,
            OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
            FileAttributes::empty(),
        )
        .await
        .map_err(|e| map_sftp_error(remote_path, e))?
        .handle;

    let max_len = proto.write_len as usize;
    let mut tasks: JoinSet<Result<u64, TransferError>> = JoinSet::new();
    let mut offset = 0u64;
    let mut done = 0u64;
    let mut eof = false;
    let mut result: Result<(), TransferError> = Ok(());

    while result.is_ok() && (!eof || !tasks.is_empty()) {
        while !eof && tasks.len() < PIPELINE_DEPTH {
            match next_chunk(max_len) {
                Ok(Some(data)) => {
                    let len = data.len() as u64;
                    let session = proto.session.clone();
                    let h = handle.clone();
                    let off = offset;
                    tasks.spawn(async move {
                        session
                            .write(h, off, data)
                            .await
                            .map(|_| len)
                            .map_err(|e| TransferError::Sftp(e.to_string()))
                    });
                    offset += len;
                }
                Ok(None) => eof = true,
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }

        if result.is_err() || tasks.is_empty() {
            break;
        }

        match tasks.join_next().await {
            Some(Ok(Ok(len))) => {
                done += len;
                on_progress(done);
            }
            Some(Ok(Err(e))) => result = Err(e),
            Some(Err(e)) => result = Err(TransferError::IoError(format!("write task: {}", e))),
            None => break,
        }
    }

    tasks.abort_all();
    let _ = proto.session.close(handle).await;
    result.map(|_| done)
}

/// Download a file over the SFTP protocol: pipelined reads streamed into
/// the local file at explicit offsets — no base64 overhead, no exec channel.
async fn download_file_sftp<F: Fn(TransferProgress)>(
    proto: &SftpProtocol,
    remote_path: &str,
    local_path: &str,
    transfer_id: &str,
    on_progress: &F,
) -> Result<(), TransferError> {
    use std::io::{Seek, SeekFrom, Write};

    tracing::info!("Downloading {} to {} (sftp)", remote_path, local_path);

    let filename = remote_filename(remote_path);

    let meta = proto
        .session
        .stat(remote_path)
        .await
        .map_err(|e| map_sftp_error(remote_path, e))?;
    let total_bytes = meta.attrs.size.unwrap_or(0);

    if total_bytes == 0 {
        std::fs::write(local_path, b"")
            .map_err(|e| TransferError::IoError(format!("Failed to write local file: {}", e)))?;
        tracing::info!("Download complete: {} (empty file)", remote_path);
        return Ok(());
    }

    let mut file = std::fs::File::create(local_path)
        .map_err(|e| TransferError::IoError(format!("Failed to create local file: {}", e)))?;

    on_progress(progress(transfer_id, &filename, 0, total_bytes));

    let done = read_remote_pipelined(
        proto,
        remote_path,
        total_bytes,
        |offset, data| {
            file.seek(SeekFrom::Start(offset))
                .and_then(|_| file.write_all(&data))
                .map_err(|e| TransferError::IoError(format!("Write error: {}", e)))
        },
        |done| on_progress(progress(transfer_id, &filename, done, total_bytes)),
    )
    .await?;

    file.flush()
        .map_err(|e| TransferError::IoError(format!("Flush error: {}", e)))?;

    tracing::info!("Download complete: {} ({} bytes)", remote_path, done);
    Ok(())
}

/// Download a file from the remote host using streaming base64 over a single SSH exec.
///
/// Runs `base64 <file>` once, streams the channel output, decodes line-by-line
/// and writes to the local file incrementally. Progress events are emitted as
/// data arrives — no per-chunk SSH roundtrips.
async fn download_file_exec<F: Fn(TransferProgress)>(
    handle: &SharedHandle,
    remote_path: &str,
    local_path: &str,
    transfer_id: &str,
    on_progress: &F,
) -> Result<(), TransferError> {
    use russh::ChannelMsg;
    use std::io::Write;

    tracing::info!("Downloading {} to {} (exec)", remote_path, local_path);

    let filename = remote_filename(remote_path);

    // Get file size first (try GNU stat, then BSD stat)
    let size_output = exec_on_connection(
        handle,
        &format!(
            "stat -c%s {} 2>/dev/null || stat -f%z {} 2>/dev/null",
            shell_escape(remote_path),
            shell_escape(remote_path)
        ),
    )
    .await?;
    let total_bytes: u64 = size_output.trim().parse().unwrap_or(0);

    if total_bytes == 0 {
        // Check if the file exists but is empty, or doesn't exist
        let exists_check = exec_on_connection(
            handle,
            &format!("test -f {} && echo EXISTS", shell_escape(remote_path)),
        )
        .await?;
        if !exists_check.trim().contains("EXISTS") {
            return Err(TransferError::FileNotFound(remote_path.to_string()));
        }
        // File exists but is empty -- write an empty file
        std::fs::write(local_path, b"")
            .map_err(|e| TransferError::IoError(format!("Failed to write local file: {}", e)))?;
        tracing::info!("Download complete: {} (empty file)", remote_path);
        return Ok(());
    }

    // Open a dedicated channel for streaming the base64 output
    let mut channel = {
        let guard = handle.lock().await;
        guard.channel_open_session().await
            .map_err(|e| SshError::ChannelError(format!("{}", e)))?
    };
    channel.exec(true, format!("base64 {}", shell_escape(remote_path))).await
        .map_err(|e| SshError::ChannelError(format!("{}", e)))?;

    // Create/truncate the local file
    let mut file = std::fs::File::create(local_path)
        .map_err(|e| TransferError::IoError(format!("Failed to create local file: {}", e)))?;

    let mut b64_buffer = String::new();
    let mut bytes_written: u64 = 0;
    let mut last_progress_bytes: u64 = 0;
    let mut got_eof = false;
    let mut got_exit = false;

    // Emit initial progress
    on_progress(progress(transfer_id, &filename, 0, total_bytes));

    loop {
        let msg = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            channel.wait(),
        ).await;

        match msg {
            Ok(Some(ChannelMsg::Data { ref data })) => {
                b64_buffer.push_str(&String::from_utf8_lossy(data));

                // Process complete base64 lines (76 chars each = 57 raw bytes)
                while let Some(newline_pos) = b64_buffer.find('\n') {
                    let line: String = b64_buffer[..newline_pos]
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect();
                    b64_buffer = b64_buffer[newline_pos + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(&line)
                        .map_err(|e| TransferError::IoError(format!("Base64 decode error: {}", e)))?;

                    file.write_all(&decoded)
                        .map_err(|e| TransferError::IoError(format!("Write error: {}", e)))?;

                    bytes_written += decoded.len() as u64;
                }

                // Emit progress every ~64KB of decoded data
                if bytes_written - last_progress_bytes >= 65536 {
                    last_progress_bytes = bytes_written;
                    on_progress(progress(transfer_id, &filename, bytes_written, total_bytes));
                }
            }
            Ok(Some(ChannelMsg::ExtendedData { .. })) => {
                // stderr — ignore
            }
            Ok(Some(ChannelMsg::Eof)) => {
                got_eof = true;
                if got_exit { break; }
            }
            Ok(Some(ChannelMsg::ExitStatus { .. })) => {
                got_exit = true;
                if got_eof { break; }
            }
            Ok(None) | Err(_) => break,
            _ => {}
        }
    }

    // Decode any remaining data in the buffer (last partial line)
    let remaining: String = b64_buffer.chars().filter(|c| !c.is_whitespace()).collect();
    if !remaining.is_empty() {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&remaining)
            .map_err(|e| TransferError::IoError(format!("Base64 decode error (tail): {}", e)))?;
        file.write_all(&decoded)
            .map_err(|e| TransferError::IoError(format!("Write error: {}", e)))?;
        bytes_written += decoded.len() as u64;
    }

    file.flush()
        .map_err(|e| TransferError::IoError(format!("Flush error: {}", e)))?;

    // Final progress
    on_progress(progress(transfer_id, &filename, bytes_written, total_bytes));

    tracing::info!("Download complete: {} ({} bytes)", remote_path, bytes_written);
    Ok(())
}

/// Upload a file to the remote host. Progress is reported through the
/// `on_progress` callback; completion is signalled by the `Ok` return.
pub(crate) async fn upload_file<F: Fn(TransferProgress)>(
    backend: &SftpBackend,
    handle: &SharedHandle,
    local_path: &str,
    remote_path: &str,
    transfer_id: &str,
    on_progress: F,
) -> Result<(), TransferError> {
    match backend.protocol() {
        Some(proto) => {
            upload_file_sftp(proto, local_path, remote_path, transfer_id, &on_progress).await
        }
        None => upload_file_exec(handle, local_path, remote_path, transfer_id, &on_progress).await,
    }
}

/// Upload a file over the SFTP protocol: pipelined writes with explicit
/// offsets. Unlike the exec path this never holds the whole file in memory.
async fn upload_file_sftp<F: Fn(TransferProgress)>(
    proto: &SftpProtocol,
    local_path: &str,
    remote_path: &str,
    transfer_id: &str,
    on_progress: &F,
) -> Result<(), TransferError> {
    use std::io::Read;

    tracing::info!("Uploading {} to {} (sftp)", local_path, remote_path);

    let filename = local_filename(local_path);

    let mut local = std::fs::File::open(local_path)
        .map_err(|e| TransferError::IoError(format!("Failed to read local file: {}", e)))?;
    let total_bytes = local
        .metadata()
        .map_err(|e| TransferError::IoError(format!("Failed to stat local file: {}", e)))?
        .len();

    // Emit initial progress
    on_progress(progress(transfer_id, &filename, 0, total_bytes));

    let producer = |max_len: usize| -> Result<Option<Vec<u8>>, TransferError> {
        let mut buf = vec![0u8; max_len];
        let mut filled = 0;
        while filled < max_len {
            match local.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    return Err(TransferError::IoError(format!("Local read error: {}", e)))
                }
            }
        }
        if filled == 0 {
            return Ok(None);
        }
        buf.truncate(filled);
        Ok(Some(buf))
    };

    let written = write_remote_pipelined(
        proto,
        remote_path,
        producer,
        |done| on_progress(progress(transfer_id, &filename, done, total_bytes)),
    )
    .await?;

    tracing::info!("Upload complete: {} ({} bytes)", remote_path, written);
    Ok(())
}

/// Upload a file to the remote host using streaming base64 over a single SSH exec.
///
/// Opens one channel running `base64 -d > <file>`, streams base64-encoded data
/// into stdin, then closes the channel. No per-chunk SSH roundtrips — mirrors
/// the download approach for maximum throughput.
async fn upload_file_exec<F: Fn(TransferProgress)>(
    handle: &SharedHandle,
    local_path: &str,
    remote_path: &str,
    transfer_id: &str,
    on_progress: &F,
) -> Result<(), TransferError> {
    use russh::ChannelMsg;

    tracing::info!("Uploading {} to {} (exec)", local_path, remote_path);

    let filename = local_filename(local_path);

    // Read local file
    let data = std::fs::read(local_path)
        .map_err(|e| TransferError::IoError(format!("Failed to read local file: {}", e)))?;

    let total_bytes = data.len() as u64;

    // Handle empty files
    if total_bytes == 0 {
        let _ = exec_on_connection(
            handle,
            &format!(": > {}", shell_escape(remote_path)),
        ).await?;
        tracing::info!("Upload complete: {} (empty file)", remote_path);
        return Ok(());
    }

    // Emit initial progress
    on_progress(progress(transfer_id, &filename, 0, total_bytes));

    // Open a single channel: pipe base64 stdin into decoder, write to file
    let mut channel = {
        let guard = handle.lock().await;
        guard.channel_open_session().await
            .map_err(|e| SshError::ChannelError(format!("{}", e)))?
    };
    channel.exec(true, format!("base64 -d > {}", shell_escape(remote_path))).await
        .map_err(|e| SshError::ChannelError(format!("{}", e)))?;

    // Stream base64-encoded data in chunks through the channel's stdin.
    // 48KB raw → 64KB base64 (multiple of 3 avoids padding mid-stream).
    let chunk_size: usize = 48 * 1024;
    let mut bytes_sent: u64 = 0;
    let mut last_progress_bytes: u64 = 0;

    for chunk in data.chunks(chunk_size) {
        let mut b64 = base64::engine::general_purpose::STANDARD.encode(chunk);
        b64.push('\n');

        channel.data(b64.as_bytes()).await
            .map_err(|e| TransferError::IoError(format!("Channel write error: {}", e)))?;

        bytes_sent += chunk.len() as u64;

        // Emit progress every ~64KB of raw data
        if bytes_sent - last_progress_bytes >= 65536 || bytes_sent == total_bytes {
            last_progress_bytes = bytes_sent;
            on_progress(progress(transfer_id, &filename, bytes_sent, total_bytes));
        }
    }

    // Close stdin to signal EOF to base64 -d
    channel.eof().await
        .map_err(|e| TransferError::IoError(format!("EOF signal error: {}", e)))?;

    // Wait for the remote command to finish
    let mut got_eof = false;
    let mut got_exit = false;
    let mut exit_code: Option<u32> = None;
    let mut stderr_buf = String::new();

    loop {
        let msg = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            channel.wait(),
        ).await;

        match msg {
            Ok(Some(ChannelMsg::ExtendedData { ref data, .. })) => {
                stderr_buf.push_str(&String::from_utf8_lossy(data));
            }
            Ok(Some(ChannelMsg::Eof)) => {
                got_eof = true;
                if got_exit { break; }
            }
            Ok(Some(ChannelMsg::ExitStatus { exit_status })) => {
                exit_code = Some(exit_status);
                got_exit = true;
                if got_eof { break; }
            }
            Ok(None) | Err(_) => break,
            _ => {}
        }
    }

    if let Some(code) = exit_code {
        if code != 0 {
            let msg = stderr_buf.trim();
            return Err(TransferError::IoError(
                if msg.is_empty() { format!("Upload failed with exit code {}", code) } else { msg.to_string() }
            ));
        }
    }

    tracing::info!("Upload complete: {} ({} bytes)", remote_path, bytes_sent);
    Ok(())
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
