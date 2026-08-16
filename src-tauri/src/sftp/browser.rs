use serde::{Deserialize, Serialize};
use thiserror::Error;

use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, FileType, OpenFlags, StatusCode};

use crate::ssh::client::{SharedHandle, exec_on_connection, SshError};
use super::backend::{SftpBackend, SftpProtocol};
use super::transfer::{read_remote_pipelined, write_remote_pipelined};

#[derive(Debug, Error)]
pub enum SftpBrowserError {
    #[error("Not connected")]
    NotConnected,
    #[error("SSH error: {0}")]
    SshError(#[from] SshError),
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("Path not found: {0}")]
    PathNotFound(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Target already exists: {0}")]
    AlreadyExists(String),
    #[error("SFTP error: {0}")]
    Protocol(String),
}

/// Metadata for a remote file or directory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "isDirectory")]
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub permissions: String,
}

/// Map an russh-sftp error onto the browser error type, preserving the
/// variants the UI cares about.
fn map_sftp_error(path: &str, e: SftpError) -> SftpBrowserError {
    match e {
        SftpError::Status(status) => match status.status_code {
            StatusCode::NoSuchFile => SftpBrowserError::PathNotFound(path.to_string()),
            StatusCode::PermissionDenied => SftpBrowserError::PermissionDenied(path.to_string()),
            _ => SftpBrowserError::Protocol(SftpError::Status(status).to_string()),
        },
        other => SftpBrowserError::Protocol(other.to_string()),
    }
}

/// Render ls-style permission characters (e.g. `drwxr-xr-x`) from an SFTP
/// file type + mode so the UI looks the same on both backends.
fn format_permissions(file_type: &FileType, mode: Option<u32>) -> String {
    let mut s = String::with_capacity(10);
    s.push(match file_type {
        FileType::Dir => 'd',
        FileType::Symlink => 'l',
        _ => '-',
    });
    let mode = mode.unwrap_or(0);
    const BITS: [(u32, char); 9] = [
        (0o400, 'r'), (0o200, 'w'), (0o100, 'x'),
        (0o040, 'r'), (0o020, 'w'), (0o010, 'x'),
        (0o004, 'r'), (0o002, 'w'), (0o001, 'x'),
    ];
    for (bit, ch) in BITS {
        s.push(if mode & bit != 0 { ch } else { '-' });
    }
    s
}

fn sort_entries(entries: &mut [RemoteEntry]) {
    // Directories first, then by name
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

fn with_trailing_slash(path: &str) -> String {
    if path.ends_with('/') {
        path.to_string()
    } else {
        format!("{}/", path)
    }
}

/// List the contents of a remote directory.
pub(crate) async fn list_directory(
    backend: &SftpBackend,
    handle: &SharedHandle,
    path: &str,
) -> Result<Vec<RemoteEntry>, SftpBrowserError> {
    match backend.protocol() {
        Some(proto) => list_directory_sftp(proto, path).await,
        None => list_directory_exec(handle, path).await,
    }
}

async fn list_directory_sftp(
    proto: &SftpProtocol,
    path: &str,
) -> Result<Vec<RemoteEntry>, SftpBrowserError> {
    let handle = proto
        .session
        .opendir(path)
        .await
        .map_err(|e| map_sftp_error(path, e))?
        .handle;

    let base = with_trailing_slash(path);
    let mut entries = Vec::new();

    loop {
        match proto.session.readdir(handle.clone()).await {
            Ok(name) => {
                for f in name.files {
                    if f.filename == "." || f.filename == ".." {
                        continue;
                    }
                    let file_type = f.attrs.file_type();
                    entries.push(RemoteEntry {
                        path: format!("{}{}", base, f.filename),
                        name: f.filename,
                        is_dir: file_type.is_dir(),
                        size: f.attrs.size.unwrap_or(0),
                        modified: f.attrs.mtime.map(u64::from).unwrap_or(0),
                        permissions: format_permissions(&file_type, f.attrs.permissions),
                    });
                }
            }
            Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => break,
            Err(e) => {
                let _ = proto.session.close(handle).await;
                return Err(map_sftp_error(path, e));
            }
        }
    }

    proto
        .session
        .close(handle)
        .await
        .map_err(|e| map_sftp_error(path, e))?;

    sort_entries(&mut entries);
    Ok(entries)
}

/// List the contents of a remote directory via SSH exec.
async fn list_directory_exec(
    handle: &SharedHandle,
    path: &str,
) -> Result<Vec<RemoteEntry>, SftpBrowserError> {
    // Use ls -lA --time-style=+%s to get machine-parseable output with timestamps
    let command = format!(
        "ls -lA --time-style=+%s {} 2>/dev/null || ls -lA {}",
        shell_escape(path),
        shell_escape(path)
    );
    let output = exec_on_connection(handle, &command).await?;
    parse_ls_output(&output, path)
}

/// Create a directory on the remote host.
///
/// Backend difference: SFTP `mkdir` requires the parent to exist and
/// fails if the target already exists, while the exec path uses `mkdir -p`.
/// Acceptable for the explorer's "new folder" flow, which always creates a
/// fresh name inside an existing directory.
pub(crate) async fn make_directory(
    backend: &SftpBackend,
    handle: &SharedHandle,
    path: &str,
) -> Result<(), SftpBrowserError> {
    match backend.protocol() {
        Some(proto) => proto
            .session
            .mkdir(path, FileAttributes::empty())
            .await
            .map(|_| ())
            .map_err(|e| map_sftp_error(path, e)),
        None => {
            let command = format!("mkdir -p {}", shell_escape(path));
            exec_on_connection(handle, &command).await?;
            Ok(())
        }
    }
}

/// Delete a file or directory on the remote host.
pub(crate) async fn delete_entry(
    backend: &SftpBackend,
    handle: &SharedHandle,
    path: &str,
) -> Result<(), SftpBrowserError> {
    match backend.protocol() {
        Some(proto) => {
            // lstat semantics: a symlink to a directory is deleted as a file.
            let meta = proto
                .session
                .lstat(path)
                .await
                .map_err(|e| map_sftp_error(path, e))?;
            if meta.attrs.file_type().is_dir() {
                // TODO: native recursive delete over SFTP (client-side
                // traversal with bounded-concurrency removals). Until then
                // directories go through the shell, exactly like the exec
                // backend — this breaks only on SFTP-only accounts
                // (ForceCommand internal-sftp), where exec is rejected.
                delete_entry_exec(handle, path).await
            } else {
                proto
                    .session
                    .remove(path)
                    .await
                    .map(|_| ())
                    .map_err(|e| map_sftp_error(path, e))
            }
        }
        None => delete_entry_exec(handle, path).await,
    }
}

async fn delete_entry_exec(
    handle: &SharedHandle,
    path: &str,
) -> Result<(), SftpBrowserError> {
    let command = format!("rm -rf {}", shell_escape(path));
    exec_on_connection(handle, &command).await?;
    Ok(())
}

/// Create an empty file on the remote host.
///
/// Both backends preserve the content of an existing file (SFTP opens with
/// CREATE but without TRUNCATE); only the mtime bump of `touch` is lost.
pub(crate) async fn touch_file(
    backend: &SftpBackend,
    handle: &SharedHandle,
    path: &str,
) -> Result<(), SftpBrowserError> {
    match backend.protocol() {
        Some(proto) => {
            let handle = proto
                .session
                .open(path, OpenFlags::CREATE | OpenFlags::WRITE, FileAttributes::empty())
                .await
                .map_err(|e| map_sftp_error(path, e))?
                .handle;
            proto
                .session
                .close(handle)
                .await
                .map(|_| ())
                .map_err(|e| map_sftp_error(path, e))
        }
        None => {
            let command = format!("touch {}", shell_escape(path));
            exec_on_connection(handle, &command).await?;
            Ok(())
        }
    }
}

/// Rename or move a remote file or directory.
///
/// Unified semantics on both backends: fails when the destination already
/// exists (native SFTP rename behaviour; the exec path checks first).
pub(crate) async fn rename_entry(
    backend: &SftpBackend,
    handle: &SharedHandle,
    old_path: &str,
    new_path: &str,
) -> Result<(), SftpBrowserError> {
    match backend.protocol() {
        Some(proto) => {
            let exists = match proto.session.stat(new_path).await {
                Ok(_) => true,
                Err(SftpError::Status(s)) if s.status_code == StatusCode::NoSuchFile => false,
                Err(e) => return Err(map_sftp_error(new_path, e)),
            };
            if exists {
                return Err(SftpBrowserError::AlreadyExists(new_path.to_string()));
            }
            proto
                .session
                .rename(old_path, new_path)
                .await
                .map(|_| ())
                .map_err(|e| map_sftp_error(old_path, e))
        }
        None => {
            let check = exec_on_connection(
                handle,
                &format!("test -e {} && echo EXISTS", shell_escape(new_path)),
            )
            .await?;
            if check.trim().contains("EXISTS") {
                return Err(SftpBrowserError::AlreadyExists(new_path.to_string()));
            }
            let command = format!("mv {} {}", shell_escape(old_path), shell_escape(new_path));
            exec_on_connection(handle, &command).await?;
            Ok(())
        }
    }
}

/// Maximum file size for text editing (5 MB).
const MAX_EDIT_SIZE: u64 = 5 * 1024 * 1024;

fn too_large_error(size: u64) -> SftpBrowserError {
    SftpBrowserError::ParseError(format!(
        "File too large to edit ({:.1} MB, max {:.0} MB)",
        size as f64 / (1024.0 * 1024.0),
        MAX_EDIT_SIZE as f64 / (1024.0 * 1024.0)
    ))
}

/// Read a text file's content from the remote host.
pub(crate) async fn read_text_file(
    backend: &SftpBackend,
    handle: &SharedHandle,
    path: &str,
) -> Result<String, SftpBrowserError> {
    match backend.protocol() {
        Some(proto) => read_text_file_sftp(proto, path).await,
        None => read_text_file_exec(handle, path).await,
    }
}

async fn read_text_file_sftp(
    proto: &SftpProtocol,
    path: &str,
) -> Result<String, SftpBrowserError> {
    let meta = proto
        .session
        .stat(path)
        .await
        .map_err(|e| map_sftp_error(path, e))?;

    let size = meta.attrs.size.unwrap_or(0);
    if size > MAX_EDIT_SIZE {
        return Err(too_large_error(size));
    }

    if size == 0 {
        return Ok(String::new());
    }

    let mut buf = vec![0u8; size as usize];
    let read = read_remote_pipelined(
        proto,
        path,
        size,
        |offset, data| {
            let start = offset as usize;
            let end = start + data.len();
            if end > buf.len() {
                return Err(super::transfer::TransferError::IoError(
                    "Remote returned more data than expected".to_string(),
                ));
            }
            buf[start..end].copy_from_slice(&data);
            Ok(())
        },
        |_| {},
    )
    .await
    .map_err(|e| SftpBrowserError::Protocol(e.to_string()))?;

    // The file may have shrunk on the server mid-read.
    buf.truncate(read as usize);

    String::from_utf8(buf)
        .map_err(|_| SftpBrowserError::ParseError("File is not valid UTF-8 text".to_string()))
}

/// Read a text file's content from the remote host via base64 encoding.
async fn read_text_file_exec(
    handle: &SharedHandle,
    path: &str,
) -> Result<String, SftpBrowserError> {
    // Check file size first
    let stat_cmd = format!("stat -c %s {} 2>/dev/null || stat -f %z {}", shell_escape(path), shell_escape(path));
    let size_output = exec_on_connection(handle, &stat_cmd).await?;
    let size: u64 = size_output
        .trim()
        .parse()
        .map_err(|_| SftpBrowserError::ParseError(format!("Cannot determine file size: {}", path)))?;

    if size > MAX_EDIT_SIZE {
        return Err(too_large_error(size));
    }

    // Read file via base64 to handle binary-safe transport
    let cmd = format!("base64 {}", shell_escape(path));
    let b64_output = exec_on_connection(handle, &cmd).await?;

    // Remove all whitespace from base64 output (line breaks etc.)
    let b64_clean: String = b64_output.chars().filter(|c| !c.is_whitespace()).collect();

    if b64_clean.is_empty() {
        return Ok(String::new());
    }

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&b64_clean)
        .map_err(|e| SftpBrowserError::ParseError(format!("Base64 decode failed: {}", e)))?;

    String::from_utf8(bytes).map_err(|_| {
        SftpBrowserError::ParseError("File is not valid UTF-8 text".to_string())
    })
}

/// Write text content to a remote file.
pub(crate) async fn write_text_file(
    backend: &SftpBackend,
    handle: &SharedHandle,
    path: &str,
    content: &str,
) -> Result<(), SftpBrowserError> {
    match backend.protocol() {
        Some(proto) => {
            let bytes = content.as_bytes();
            let mut pos = 0usize;
            let producer = |max_len: usize| {
                if pos >= bytes.len() {
                    return Ok(None);
                }
                let end = (pos + max_len).min(bytes.len());
                let chunk = bytes[pos..end].to_vec();
                pos = end;
                Ok(Some(chunk))
            };
            write_remote_pipelined(proto, path, producer, |_| {})
                .await
                .map(|_| ())
                .map_err(|e| SftpBrowserError::Protocol(e.to_string()))
        }
        None => write_text_file_exec(handle, path, content).await,
    }
}

/// Write text content to a remote file via streaming base64 over a single SSH channel.
async fn write_text_file_exec(
    handle: &SharedHandle,
    path: &str,
    content: &str,
) -> Result<(), SftpBrowserError> {
    use base64::Engine;
    use russh::ChannelMsg;

    let data = content.as_bytes();

    // Empty file: simple truncate
    if data.is_empty() {
        let cmd = format!(": > {}", shell_escape(path));
        exec_on_connection(handle, &cmd).await?;
        return Ok(());
    }

    // Open a single channel: pipe base64 stdin into decoder, write to file
    let mut channel = {
        let guard = handle.lock().await;
        guard.channel_open_session().await
            .map_err(|e| SshError::ChannelError(format!("{}", e)))?
    };
    channel.exec(true, format!("base64 -d > {}", shell_escape(path))).await
        .map_err(|e| SshError::ChannelError(format!("{}", e)))?;

    // Stream base64 in 48KB raw chunks (multiple of 3 → clean base64, no mid-stream padding)
    let chunk_size: usize = 48 * 1024;

    for chunk in data.chunks(chunk_size) {
        let mut b64 = base64::engine::general_purpose::STANDARD.encode(chunk);
        b64.push('\n');

        channel.data(b64.as_bytes()).await
            .map_err(|e| SftpBrowserError::ParseError(format!("Channel write error: {}", e)))?;
    }

    // Close stdin to signal EOF
    channel.eof().await
        .map_err(|e| SftpBrowserError::ParseError(format!("EOF signal error: {}", e)))?;

    // Wait for exit
    let mut got_eof = false;
    let mut got_exit = false;
    let mut exit_code: Option<u32> = None;
    let mut stderr_buf = String::new();

    loop {
        let msg = tokio::time::timeout(
            std::time::Duration::from_secs(10),
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
            if msg.to_lowercase().contains("permission denied") {
                return Err(SftpBrowserError::PermissionDenied(path.to_string()));
            }
            return Err(SftpBrowserError::ParseError(
                if msg.is_empty() { format!("Write failed with exit code {}", code) } else { msg.to_string() }
            ));
        }
    }

    Ok(())
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn parse_ls_output(output: &str, base_path: &str) -> Result<Vec<RemoteEntry>, SftpBrowserError> {
    let mut entries = Vec::new();
    let base = with_trailing_slash(base_path);

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("total") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 7 {
            continue;
        }

        let permissions = parts[0].to_string();
        // Skip non-entry lines (permissions must start with d, -, l, c, b, p, s)
        if !permissions.starts_with(|c: char| "d-lcbps".contains(c)) {
            continue;
        }
        let is_dir = permissions.starts_with('d');
        let size: u64 = parts[4].parse().unwrap_or(0);

        // Detect format: --time-style=+%s gives epoch in parts[5], standard ls gives month
        let (modified, name) = if parts[5].parse::<u64>().is_ok() {
            // Epoch format: perms links owner group size epoch name...
            // parts: [0]=perms [1]=links [2]=owner [3]=group [4]=size [5]=epoch [6..]=name
            let ts = parts[5].parse::<u64>().unwrap_or(0);
            let name = parts[6..].join(" ");
            (ts, name)
        } else {
            // Standard format: perms links owner group size month day time/year name...
            // parts: [0]=perms [1]=links [2]=owner [3]=group [4]=size [5]=month [6]=day [7]=time [8..]=name
            if parts.len() < 9 {
                // Might be a short format, take last field as name
                let name = parts[parts.len() - 1].to_string();
                (0u64, name)
            } else {
                let name = parts[8..].join(" ");
                (0u64, name)
            }
        };

        if name == "." || name == ".." || name.is_empty() {
            continue;
        }

        // Handle symlinks: name -> target
        let clean_name = if let Some(idx) = name.find(" -> ") {
            name[..idx].to_string()
        } else {
            name
        };

        entries.push(RemoteEntry {
            path: format!("{}{}", base, clean_name),
            name: clean_name,
            is_dir,
            size,
            modified,
            permissions,
        });
    }

    sort_entries(&mut entries);
    Ok(entries)
}
