//! Agent configuration storage (design 04 §1).
//!
//! All backend-consumed config lives in the encrypted vault's generic kv
//! (the `__settings__` vault, key == secret id, same pattern as
//! `settings_commands.rs`):
//!
//! - `agent:tools`                         → JSON Record<ToolName, ToolConfig>
//! - `agent:providers`                     → JSON Vec<ProviderInstance> (no keys)
//! - `agent:provider:{instanceId}:api_key` → per-instance API key
//! - `agent:title_model`                   → "{instanceId}/{modelId}"

use serde::{Deserialize, Serialize};

use crate::vault::types::SecretCategory;
use crate::vault::VaultManager;

const SETTINGS_VAULT_NAME: &str = "__settings__";

pub const KEY_TOOLS: &str = "agent:tools";
pub const KEY_PROVIDERS: &str = "agent:providers";
pub const KEY_TITLE_MODEL: &str = "agent:title_model";

pub fn api_key_key(instance_id: &str) -> String {
    format!("agent:provider:{}:api_key", instance_id)
}

/// Per-tool user configuration (design 04 §1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
    pub enabled: bool,
    #[serde(default)]
    pub require_approval: bool,
    #[serde(default)]
    pub options: serde_json::Map<String, serde_json::Value>,
}

/// A provider instance created from a preset (design 05 §0).
/// The API key is stored separately under `agent:provider:{id}:api_key`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInstance {
    pub id: String,
    pub preset: String, // "openrouter" | "deepseek" | "kimi"
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

impl ProviderInstance {
    /// Effective base URL: instance override or the preset default.
    pub fn resolved_base_url(&self) -> String {
        if let Some(url) = &self.base_url {
            let trimmed = url.trim();
            if !trimmed.is_empty() {
                return trimmed.trim_end_matches('/').to_string();
            }
        }
        crate::agent::providers::preset_default_base_url(&self.preset)
            .unwrap_or_default()
            .to_string()
    }
}

// ---------------------------------------------------------------------------
// Raw kv helpers
// ---------------------------------------------------------------------------

/// Read a raw string value. Returns None when the vault is locked, the
/// vault doesn't exist yet, or the key is missing.
pub async fn kv_read(manager: &VaultManager, key: &str) -> Option<String> {
    if manager.is_locked() {
        return None;
    }
    let vault_id = manager.get_vault_id_by_name(SETTINGS_VAULT_NAME)?;
    match manager.read_secret(&vault_id, key).await {
        Ok(plaintext) => {
            use secrecy::ExposeSecret;
            String::from_utf8(plaintext.expose_secret().clone()).ok()
        }
        Err(_) => None,
    }
}

/// Upsert a raw string value. Errors when the vault is locked.
pub async fn kv_write(manager: &mut VaultManager, key: &str, value: &str) -> Result<(), String> {
    if manager.is_locked() {
        return Err("Vault is locked. Unlock it to change AI settings.".to_string());
    }
    let vault_id = ensure_settings_vault(manager).await?;
    let plaintext = secrecy::SecretBox::new(Box::new(value.as_bytes().to_vec()));
    if manager.secret_exists(&vault_id, key).await {
        manager
            .update_secret(&vault_id, key, plaintext)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        manager
            .create_secret_with_id(&vault_id, key, key, SecretCategory::ApiToken, plaintext)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub async fn kv_delete(manager: &mut VaultManager, key: &str) -> Result<(), String> {
    if manager.is_locked() {
        return Err("Vault is locked".to_string());
    }
    if let Some(vault_id) = manager.get_vault_id_by_name(SETTINGS_VAULT_NAME) {
        let _ = manager.delete_secret(&vault_id, key).await;
    }
    Ok(())
}

async fn ensure_settings_vault(manager: &mut VaultManager) -> Result<String, String> {
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

// ---------------------------------------------------------------------------
// Typed accessors
// ---------------------------------------------------------------------------

pub async fn read_tool_configs(
    manager: &VaultManager,
) -> std::collections::HashMap<String, ToolConfig> {
    kv_read(manager, KEY_TOOLS)
        .await
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub async fn write_tool_configs(
    manager: &mut VaultManager,
    configs: &std::collections::HashMap<String, ToolConfig>,
) -> Result<(), String> {
    let json = serde_json::to_string(configs).map_err(|e| e.to_string())?;
    kv_write(manager, KEY_TOOLS, &json).await
}

pub async fn read_providers(manager: &VaultManager) -> Vec<ProviderInstance> {
    kv_read(manager, KEY_PROVIDERS)
        .await
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub async fn write_providers(
    manager: &mut VaultManager,
    providers: &[ProviderInstance],
) -> Result<(), String> {
    let json = serde_json::to_string(providers).map_err(|e| e.to_string())?;
    kv_write(manager, KEY_PROVIDERS, &json).await
}

pub async fn read_api_key(manager: &VaultManager, instance_id: &str) -> Option<String> {
    kv_read(manager, &api_key_key(instance_id)).await
}

pub async fn write_api_key(
    manager: &mut VaultManager,
    instance_id: &str,
    key: &str,
) -> Result<(), String> {
    kv_write(manager, &api_key_key(instance_id), key).await
}

pub async fn delete_api_key(manager: &mut VaultManager, instance_id: &str) -> Result<(), String> {
    kv_delete(manager, &api_key_key(instance_id)).await
}

pub async fn read_title_model(manager: &VaultManager) -> Option<String> {
    kv_read(manager, KEY_TITLE_MODEL).await
}

pub async fn write_title_model(manager: &mut VaultManager, value: &str) -> Result<(), String> {
    kv_write(manager, KEY_TITLE_MODEL, value).await
}
