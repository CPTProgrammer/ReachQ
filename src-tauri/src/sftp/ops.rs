//! High-level remote filesystem facade shared by the IPC commands and
//! future AI-agent tools. Resolves the SSH handle and the negotiated
//! SFTP backend for a connection, then delegates to browser/transfer.

use std::sync::Arc;

use crate::ssh::client::{SharedHandle, SshManager};
use super::backend::{SftpBackend, SftpBackendManager};
use super::browser::{self, RemoteEntry, SftpBrowserError};
use super::transfer::{self, TransferError, TransferProgress};

/// Remote filesystem bound to one SSH connection.
///
/// Cheap to create per use: the expensive part (the SFTP handshake) is
/// cached in `AppState::sftp_backend_manager`, so `connect()` on an
/// already-probed connection is just two map lookups.
pub struct RemoteFs {
    backend: Arc<SftpBackend>,
    handle: SharedHandle,
}

impl RemoteFs {
    /// Resolve the SSH handle for `connection_id` and negotiate (or fetch
    /// the cached) SFTP backend for it.
    ///
    /// Takes the two managers rather than `&AppState` so non-Tauri
    /// consumers (plugin host API, future agent tools) can use it too.
    pub async fn connect(
        ssh_manager: &Arc<tokio::sync::Mutex<SshManager>>,
        backend_manager: &Arc<tokio::sync::Mutex<SftpBackendManager>>,
        connection_id: &str,
    ) -> Result<Self, SftpBrowserError> {
        let handle = {
            let manager = ssh_manager.lock().await;
            manager
                .get_handle(connection_id)
                .map_err(|_| SftpBrowserError::NotConnected)?
        };
        let backend = {
            let mut backends = backend_manager.lock().await;
            backends.get(connection_id, &handle).await
        };
        Ok(Self { backend, handle })
    }

    // TODO(agent): entry point for AI-agent file tools, e.g.
    // `for_host(state, host_key)`. Agents are keyed by
    // `username@host:port#via=jumphost`, not by connection_id, so this
    // should ask SshManager for a live connection matching the host key
    // (preferring the active tab) and delegate to `connect`. On
    // connection-level errors (NotConnected / dead channel) it should
    // re-resolve and retry once, so the agent survives the user closing
    // the tab whose connection it rode on. Requires storing the jump
    // chain (or a precomputed host key) on ConnectionInfo.

    /// List the contents of a remote directory.
    pub async fn list_dir(&self, path: &str) -> Result<Vec<RemoteEntry>, SftpBrowserError> {
        browser::list_directory(&self.backend, &self.handle, path).await
    }

    /// Create a directory (non-recursive on the SFTP backend).
    pub async fn mkdir(&self, path: &str) -> Result<(), SftpBrowserError> {
        browser::make_directory(&self.backend, &self.handle, path).await
    }

    /// Create an empty file (keeps content if it already exists).
    pub async fn touch(&self, path: &str) -> Result<(), SftpBrowserError> {
        browser::touch_file(&self.backend, &self.handle, path).await
    }

    /// Rename/move. Fails if the destination already exists.
    pub async fn rename(&self, old_path: &str, new_path: &str) -> Result<(), SftpBrowserError> {
        browser::rename_entry(&self.backend, &self.handle, old_path, new_path).await
    }

    /// Delete a file or directory. Directory deletion currently always
    /// goes through the shell (`rm -rf`) even on the SFTP backend.
    pub async fn delete(&self, path: &str) -> Result<(), SftpBrowserError> {
        browser::delete_entry(&self.backend, &self.handle, path).await
    }

    /// Read a UTF-8 text file (size-capped, see MAX_EDIT_SIZE).
    pub async fn read_text(&self, path: &str) -> Result<String, SftpBrowserError> {
        browser::read_text_file(&self.backend, &self.handle, path).await
    }

    /// Write/truncate a text file.
    pub async fn write_text(&self, path: &str, content: &str) -> Result<(), SftpBrowserError> {
        browser::write_text_file(&self.backend, &self.handle, path, content).await
    }

    /// Stat a remote path (mtime+size+permissions). Added for the agent's
    /// fingerprint logic (design 02 §3).
    pub async fn stat(&self, path: &str) -> Result<browser::RemoteStat, SftpBrowserError> {
        browser::stat_entry(&self.backend, &self.handle, path).await
    }

    /// Read raw bytes of a remote file (no UTF-8 validation, caller-chosen
    /// size cap). Used by the agent's encoding pipeline.
    pub async fn read_bytes(&self, path: &str, max_size: u64) -> Result<Vec<u8>, SftpBrowserError> {
        browser::read_bytes(&self.backend, &self.handle, path, max_size).await
    }

    /// Write raw bytes to a remote file (truncate + write).
    pub async fn write_bytes(&self, path: &str, data: &[u8]) -> Result<(), SftpBrowserError> {
        browser::write_bytes(&self.backend, &self.handle, path, data).await
    }

    /// Upload a local file, reporting progress through `on_progress`.
    pub async fn upload<F: Fn(TransferProgress)>(
        &self,
        local_path: &str,
        remote_path: &str,
        transfer_id: &str,
        on_progress: F,
    ) -> Result<(), TransferError> {
        transfer::upload_file(
            &self.backend,
            &self.handle,
            local_path,
            remote_path,
            transfer_id,
            on_progress,
        )
        .await
    }

    /// Download a remote file, reporting progress through `on_progress`.
    pub async fn download<F: Fn(TransferProgress)>(
        &self,
        remote_path: &str,
        local_path: &str,
        transfer_id: &str,
        on_progress: F,
    ) -> Result<(), TransferError> {
        transfer::download_file(
            &self.backend,
            &self.handle,
            remote_path,
            local_path,
            transfer_id,
            on_progress,
        )
        .await
    }
}
