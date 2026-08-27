//! Settings commands for storing API keys and app configuration.
//!
//! All settings are stored encrypted in SQLite using XChaCha20-Poly1305.
//! Lookups are O(1) using setting_key as the primary key.

use crate::state::AppState;
use crate::vault::types::SecretCategory;
use secrecy::SecretBox;
use serde::{Deserialize, Serialize};
use tauri::State;

const SETTINGS_VAULT_NAME: &str = "__settings__";

/// App settings structure returned to frontend.
///
/// The legacy fixed AI fields (`openrouter_api_key` etc.) were removed
/// (design 04 §5): agent configuration now lives in the vault's generic kv
/// via `agent_commands`. The struct is kept (empty) so `settings_get_all` /
/// `settings_save_all` stay wire-compatible with existing callers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {}

/// Get all app settings. O(1) per setting.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn settings_get_all(state: State<'_, AppState>) -> Result<AppSettings, String> {
    // No fixed fields remain; generic kv access goes through `settings_get`.
    let _ = &state;
    Ok(AppSettings::default())
}

/// Get a single setting by key. O(1) lookup.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn settings_get(
    state: State<'_, AppState>,
    key: String,
) -> Result<Option<String>, String> {
    let manager = state.vault_manager.lock().await;

    if manager.is_locked() {
        return Ok(None);
    }

    let vault_id = match get_settings_vault_id_if_exists(&manager) {
        Some(id) => id,
        None => return Ok(None),
    };

    Ok(read_setting(&manager, &vault_id, &key).await)
}

/// Set a setting value. O(1) upsert.
#[tauri::command]
#[tracing::instrument(skip(value, state))]
pub async fn settings_set(
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    let mut manager = state.vault_manager.lock().await;

    if manager.is_locked() {
        return Err("Vault is locked. Set a master password first.".to_string());
    }

    let vault_id = ensure_settings_vault(&mut manager).await?;
    let value_bytes = value.into_bytes();

    // O(1) check then upsert
    if manager.secret_exists(&vault_id, &key).await {
        let plaintext = SecretBox::new(Box::new(value_bytes));
        manager
            .update_secret(&vault_id, &key, plaintext)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        let plaintext = SecretBox::new(Box::new(value_bytes));
        manager
            .create_secret_with_id(&vault_id, &key, &key, SecretCategory::ApiToken, plaintext)
            .await
            .map_err(|e| e.to_string())?;
    }

    tracing::info!("Saved setting: {}", key);
    Ok(())
}

/// Delete a setting. O(1) delete.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn settings_delete(state: State<'_, AppState>, key: String) -> Result<(), String> {
    let manager = state.vault_manager.lock().await;

    if manager.is_locked() {
        return Err("Vault is locked".to_string());
    }

    let vault_id = match get_settings_vault_id_if_exists(&manager) {
        Some(id) => id,
        None => return Ok(()),
    };

    let _ = manager.delete_secret(&vault_id, &key).await;
    tracing::info!("Deleted setting: {}", key);
    Ok(())
}

/// Save all app settings at once. No-op: no fixed fields remain (the agent
/// config moved to the vault's generic kv). Kept for wire compatibility.
#[tauri::command]
#[tracing::instrument(skip(settings, state))]
pub async fn settings_save_all(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<(), String> {
    let _ = &state;
    let _ = &settings;
    Ok(())
}

// --- Helper functions (all O(1)) ---

/// Read a single setting. O(1).
async fn read_setting(
    manager: &crate::vault::VaultManager,
    vault_id: &str,
    key: &str,
) -> Option<String> {
    match manager.read_secret(vault_id, key).await {
        Ok(plaintext) => {
            use secrecy::ExposeSecret;
            String::from_utf8(plaintext.expose_secret().clone()).ok()
        }
        Err(_) => None,
    }
}

/// Ensure the settings vault exists. O(1).
async fn ensure_settings_vault(
    manager: &mut crate::vault::VaultManager,
) -> Result<String, String> {
    if let Some(vault_id) = manager.get_vault_id_by_name(SETTINGS_VAULT_NAME) {
        let _ = manager.open_vault(&vault_id, None, None).await;
        manager
            .unlock_vault(&vault_id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(vault_id)
    } else {
        let vault = manager
            .create_vault(SETTINGS_VAULT_NAME, crate::vault::types::VaultType::Private, None, None)
            .await
            .map_err(|e| e.to_string())?;
        Ok(vault.id)
    }
}

/// Get settings vault ID. O(1).
fn get_settings_vault_id_if_exists(manager: &crate::vault::VaultManager) -> Option<String> {
    manager.get_vault_id_by_name(SETTINGS_VAULT_NAME)
}

/// Enumerate system fonts.
///
/// Uses the fontdb crate to scan system font directories and return
/// a deduplicated, sorted list of font family names.
#[tauri::command]
#[tracing::instrument]
pub fn list_system_fonts() -> Result<Vec<String>, String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    let mut families: Vec<String> = db
        .faces()
        .map(|face| {
            face.families
                .first()
                .map(|(name, _lang)| name.to_string())
                .unwrap_or_default()
        })
        .filter(|name| !name.is_empty())
        .collect();

    families.sort_unstable();
    families.dedup();

    tracing::info!("Found {} system font families", families.len());
    Ok(families)
}
