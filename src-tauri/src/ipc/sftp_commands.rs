use tracing::info;
use tauri::Emitter;
use crate::state::AppState;
use crate::sftp::browser::RemoteEntry;
use crate::sftp::ops::RemoteFs;
use crate::sftp::transfer::TransferProgress;
use crate::plugin::hooks;

/// List the contents of a remote directory.
#[tauri::command]
pub async fn sftp_list_dir(
    state: tauri::State<'_, AppState>,
    connection_id: String,
    path: String,
) -> Result<Vec<RemoteEntry>, String> {
    info!("sftp_list_dir called: conn={}, path={}", connection_id, path);
    let fs = RemoteFs::connect(&state.ssh_manager, &state.sftp_backend_manager, &connection_id).await.map_err(|e| {
        info!("sftp_list_dir connect error: {}", e);
        e.to_string()
    })?;
    let result = fs.list_dir(&path).await.map_err(|e| {
        info!("sftp_list_dir browse error: {}", e);
        e.to_string()
    })?;
    info!("sftp_list_dir returning {} entries for {}", result.len(), path);
    Ok(result)
}

/// Upload a local file to the remote host. Returns the transfer_id immediately
/// and runs the upload in a background task, emitting progress events.
#[tauri::command]
pub async fn sftp_upload(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    connection_id: String,
    local_path: String,
    remote_path: String,
) -> Result<String, String> {
    let fs = RemoteFs::connect(&state.ssh_manager, &state.sftp_backend_manager, &connection_id).await.map_err(|e| e.to_string())?;
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let tid = transfer_id.clone();
    let plugin_mgr = state.plugin_manager.clone();
    let conn_id = connection_id.clone();
    let rpath = remote_path.clone();

    tokio::spawn(async move {
        let app_progress = app.clone();
        let tid_progress = tid.clone();
        let result = fs
            .upload(&local_path, &remote_path, &tid, move |p: TransferProgress| {
                let _ = app_progress.emit(&format!("transfer-progress-{}", tid_progress), &p);
            })
            .await;

        match result {
            Ok(()) => {
                let _ = app.emit(&format!("transfer-complete-{}", tid), ());
                let hook = hooks::sftp_upload_complete(&conn_id, &rpath);
                let mut mgr = plugin_mgr.lock().await;
                mgr.dispatch_hook(&hook, Some(&app)).await;
            }
            Err(e) => {
                tracing::error!("Upload failed for {}: {}", tid, e);
                let _ = app.emit(&format!("transfer-error-{}", tid), e.to_string());
            }
        }
    });

    Ok(transfer_id)
}

/// Download a file from the remote host. Returns the transfer_id immediately
/// and runs the download in a background task, emitting progress events.
#[tauri::command]
pub async fn sftp_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    connection_id: String,
    remote_path: String,
    local_path: String,
) -> Result<String, String> {
    let fs = RemoteFs::connect(&state.ssh_manager, &state.sftp_backend_manager, &connection_id).await.map_err(|e| e.to_string())?;
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let tid = transfer_id.clone();
    let plugin_mgr = state.plugin_manager.clone();
    let conn_id = connection_id.clone();
    let rpath = remote_path.clone();
    let lpath = local_path.clone();

    tokio::spawn(async move {
        let app_progress = app.clone();
        let tid_progress = tid.clone();
        let result = fs
            .download(&remote_path, &local_path, &tid, move |p: TransferProgress| {
                let _ = app_progress.emit(&format!("transfer-progress-{}", tid_progress), &p);
            })
            .await;

        match result {
            Ok(()) => {
                let _ = app.emit(&format!("transfer-complete-{}", tid), ());
                let hook = hooks::sftp_download_complete(&conn_id, &rpath, &lpath);
                let mut mgr = plugin_mgr.lock().await;
                mgr.dispatch_hook(&hook, Some(&app)).await;
            }
            Err(e) => {
                tracing::error!("Download failed for {}: {}", tid, e);
                let _ = app.emit(&format!("transfer-error-{}", tid), e.to_string());
            }
        }
    });

    Ok(transfer_id)
}

/// Delete a file or directory on the remote host.
#[tauri::command]
pub async fn sftp_delete(
    state: tauri::State<'_, AppState>,
    connection_id: String,
    path: String,
) -> Result<(), String> {
    let fs = RemoteFs::connect(&state.ssh_manager, &state.sftp_backend_manager, &connection_id).await.map_err(|e| e.to_string())?;
    fs.delete(&path).await.map_err(|e| e.to_string())
}

/// Rename or move a file on the remote host.
#[tauri::command]
pub async fn sftp_rename(
    state: tauri::State<'_, AppState>,
    connection_id: String,
    old_path: String,
    new_path: String,
) -> Result<(), String> {
    let fs = RemoteFs::connect(&state.ssh_manager, &state.sftp_backend_manager, &connection_id).await.map_err(|e| e.to_string())?;
    fs.rename(&old_path, &new_path).await.map_err(|e| e.to_string())
}

/// Create an empty file on the remote host.
#[tauri::command]
pub async fn sftp_touch(
    state: tauri::State<'_, AppState>,
    connection_id: String,
    path: String,
) -> Result<(), String> {
    let fs = RemoteFs::connect(&state.ssh_manager, &state.sftp_backend_manager, &connection_id).await.map_err(|e| e.to_string())?;
    fs.touch(&path).await.map_err(|e| e.to_string())
}

/// Read a text file's content from the remote host.
#[tauri::command]
pub async fn sftp_read_file(
    state: tauri::State<'_, AppState>,
    connection_id: String,
    path: String,
) -> Result<String, String> {
    info!("sftp_read_file called: conn={}, path={}", connection_id, path);
    let fs = RemoteFs::connect(&state.ssh_manager, &state.sftp_backend_manager, &connection_id).await.map_err(|e| e.to_string())?;
    fs.read_text(&path).await.map_err(|e| e.to_string())
}

/// Write text content to a remote file.
#[tauri::command]
pub async fn sftp_write_file(
    state: tauri::State<'_, AppState>,
    connection_id: String,
    path: String,
    content: String,
) -> Result<(), String> {
    info!("sftp_write_file called: conn={}, path={}", connection_id, path);
    let fs = RemoteFs::connect(&state.ssh_manager, &state.sftp_backend_manager, &connection_id).await.map_err(|e| e.to_string())?;
    fs.write_text(&path, &content).await.map_err(|e| e.to_string())
}

/// Create a directory on the remote host.
#[tauri::command]
pub async fn sftp_mkdir(
    state: tauri::State<'_, AppState>,
    connection_id: String,
    path: String,
) -> Result<(), String> {
    let fs = RemoteFs::connect(&state.ssh_manager, &state.sftp_backend_manager, &connection_id).await.map_err(|e| e.to_string())?;
    fs.mkdir(&path).await.map_err(|e| e.to_string())
}
