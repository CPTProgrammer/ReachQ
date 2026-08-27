//! AI agent subsystem (design 02 §1). Per-SSH-identity agent with standard
//! tool calling, approvals, streaming, and branched thread history.

pub mod agent_loop;
pub mod config;
pub mod events;
pub mod identity;
pub mod permissions;
pub mod providers;
pub mod remote_fs;
pub mod thread_store;
pub mod tools;
pub mod types;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{Mutex, Notify, OnceCell, RwLock};
use tokio_util::sync::CancellationToken;

use remote_fs::{PendingWrites, ReadCache, ReadPaths};
use thread_store::ThreadStore;
pub use tools::terminal::SharedTerminals;

/// The AppState pieces the agent loop needs, cloneable and 'static so the
/// loop can run in a spawned task (Tauri `State` never yields Arc<AppState>).
#[derive(Clone)]
pub struct AgentDeps {
    pub agent: Arc<AgentState>,
    pub vault_manager: Arc<tokio::sync::Mutex<crate::vault::VaultManager>>,
    pub ssh_manager: Arc<tokio::sync::Mutex<crate::ssh::client::SshManager>>,
    pub sftp_backends: Arc<tokio::sync::Mutex<crate::sftp::backend::SftpBackendManager>>,
}

impl AgentDeps {
    pub fn from_state(state: &crate::state::AppState) -> Self {
        Self {
            agent: state.agent.clone(),
            vault_manager: state.vault_manager.clone(),
            ssh_manager: state.ssh_manager.clone(),
            sftp_backends: state.sftp_backend_manager.clone(),
        }
    }
}

/// A live agent run: cancellation signal plus a completion notification
/// (agent_send_now waits on `done` before starting the successor run).
pub struct RunHandle {
    /// Observed by the SSE stream, approvals, and tools.
    pub token: CancellationToken,
    /// Fired once the run has fully exited and been unregistered.
    pub done: Arc<Notify>,
}

/// Shared agent state, owned by AppState.
pub struct AgentState {
    app_dir: PathBuf,
    /// agent.db, opened lazily (AppState::new is sync).
    pub store: OnceCell<Arc<ThreadStore>>,
    /// thread_id -> active run handle (one active run per thread).
    pub runs: Mutex<HashMap<String, Arc<RunHandle>>>,
    /// thread_id -> queued message (at most 1; injected at round boundary).
    pub queued: Mutex<HashMap<String, String>>,
    /// tool_call_id -> approval decision channel.
    pub approvals: Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    pub read_cache: ReadCache,
    /// thread_id -> set of paths read (read-before-write gate).
    pub read_paths: Mutex<HashMap<String, ReadPaths>>,
    pub pending_writes: PendingWrites,
    pub terminals: SharedTerminals,
    /// Tool configs cached from the vault; invalidated on settings writes.
    pub tool_configs: RwLock<Option<HashMap<String, config::ToolConfig>>>,
    /// instance_id -> (fetched_at, models). 10 min TTL (design 05 §4).
    pub models_cache: Mutex<HashMap<String, (Instant, Vec<providers::ModelMeta>)>>,
    /// identity -> detected OS string (best-effort, once per session).
    pub detected_os: Mutex<HashMap<String, String>>,
}

impl AgentState {
    pub fn new(app_dir: PathBuf) -> Self {
        Self {
            app_dir,
            store: OnceCell::new(),
            runs: Mutex::new(HashMap::new()),
            queued: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            read_cache: remote_fs::new_read_cache(),
            read_paths: Mutex::new(HashMap::new()),
            pending_writes: remote_fs::new_pending_writes(),
            terminals: tools::terminal::new_shared_terminals(),
            tool_configs: RwLock::new(None),
            models_cache: Mutex::new(HashMap::new()),
            detected_os: Mutex::new(HashMap::new()),
        }
    }

    /// Open (or get) the thread store.
    pub async fn thread_store(&self) -> Result<Arc<ThreadStore>, String> {
        self.store
            .get_or_try_init(|| async {
                ThreadStore::open(&self.app_dir).await.map(Arc::new)
            })
            .await
            .map(|s| s.clone())
    }

    /// Read the read-paths set for a thread (creating it empty).
    pub async fn read_paths_for(&self, thread_id: &str) -> ReadPaths {
        let mut map = self.read_paths.lock().await;
        map.entry(thread_id.to_string())
            .or_insert_with(|| Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())))
            .clone()
    }

    /// Rebuild a thread's read-path set from history (recovery after
    /// restart; design 03 §2 behavior 0).
    pub async fn ensure_read_paths(&self, thread_id: &str) -> Result<(), String> {
        let exists = self.read_paths.lock().await.contains_key(thread_id);
        if exists {
            return Ok(());
        }
        let store = self.thread_store().await?;
        let paths = store.collect_read_paths(thread_id).await?;
        let set = self.read_paths_for(thread_id).await;
        let mut guard = set.lock().unwrap();
        for p in paths {
            guard.insert(p);
        }
        Ok(())
    }

    /// Tool configs: vault-backed with an in-memory cache (design 04 §4.3:
    /// the loop reads the latest value before each tool call; the cache is
    /// invalidated on settings writes).
    pub async fn tool_configs(
        &self,
        vault_manager: &crate::vault::VaultManager,
    ) -> HashMap<String, config::ToolConfig> {
        {
            let cache = self.tool_configs.read().await;
            if let Some(c) = cache.as_ref() {
                return c.clone();
            }
        }
        let configs = config::read_tool_configs(vault_manager).await;
        let mut cache = self.tool_configs.write().await;
        *cache = Some(configs.clone());
        configs
    }

    pub async fn invalidate_tool_configs(&self) {
        let mut cache = self.tool_configs.write().await;
        *cache = None;
    }

    /// Resolved config for one tool: defaults merged under user settings.
    pub async fn resolved_tool_config(
        &self,
        vault_manager: &crate::vault::VaultManager,
        tool: &Arc<dyn tools::AgentTool>,
    ) -> config::ToolConfig {
        let configs = self.tool_configs(vault_manager).await;
        let mut defaults: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
        for opt in tool.settings_schema() {
            defaults.insert(opt.key.clone(), opt.default.clone());
        }
        match configs.get(tool.name()) {
            Some(user) => {
                let mut options = defaults;
                for (k, v) in &user.options {
                    options.insert(k.clone(), v.clone());
                }
                config::ToolConfig {
                    enabled: user.enabled,
                    require_approval: user.require_approval,
                    options,
                }
            }
            None => config::ToolConfig {
                enabled: true,
                require_approval: tool.default_requires_approval(),
                options: defaults,
            },
        }
    }
}
