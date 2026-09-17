//! Threads/messages persistence in a standalone plaintext SQLite database
//! (`data_dir/agent.db`), independent of the encrypted vault (design 02 §4).
//!
//! Threads are owned by a scope (`session:<uuid>` for saved sessions,
//! `link:<identity>` for quick-connect links) and listed strictly by that
//! key, so saved-session history survives connection-detail edits.
//!
//! Messages form a tree: each message points at its parent; siblings under
//! the same parent are branches. A thread records the active leaf; the
//! visible conversation is the path from the leaf up to the root.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use libsql::{params, Connection};
use uuid::Uuid;

use super::types::{
    BranchInfo, ModelSelection, PathMessage, StoredMessage, ThreadSnapshot, ThreadSummary, Usage,
};

pub struct ThreadStore {
    conn: Connection,
    /// Global monotonic sequence for messages (seeded from MAX(seq) at open).
    next_seq: AtomicI64,
    /// In-memory branch memory: (thread_id, parent_id) -> preferred child.
    /// Lets `< prev / next >` navigation return to the exact sub-branch the
    /// user came from instead of always falling to the latest fork.
    /// Persisted indirectly: every switch updates `active_leaf_id`.
    active_children: Mutex<HashMap<(String, Option<String>), String>>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS agent_threads (
    id TEXT PRIMARY KEY,
    owner_key TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    archived INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    active_leaf_id TEXT,
    last_usage TEXT,
    model TEXT
);
CREATE INDEX IF NOT EXISTS idx_threads_owner ON agent_threads(owner_key, updated_at DESC);

CREATE TABLE IF NOT EXISTS agent_messages (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES agent_threads(id) ON DELETE CASCADE,
    parent_id TEXT,
    branch_index INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    usage TEXT,
    metadata TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_thread ON agent_messages(thread_id, seq);
CREATE INDEX IF NOT EXISTS idx_messages_parent ON agent_messages(parent_id);

-- Unsent composer drafts, one per thread. Separate table (not a column on
-- agent_threads) so existing databases need no ALTER TABLE; the FK cascade
-- makes drafts die with their thread.
CREATE TABLE IF NOT EXISTS agent_drafts (
    thread_id TEXT PRIMARY KEY REFERENCES agent_threads(id) ON DELETE CASCADE,
    text TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
";

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Thread list columns: base fields + live message count + persisted draft.
const THREAD_SELECT: &str = "\
SELECT t.id, t.owner_key, t.title, t.archived, t.created_at, t.updated_at, t.last_usage, t.model, \
       (SELECT COUNT(*) FROM agent_messages m WHERE m.thread_id = t.id), \
       COALESCE((SELECT d.text FROM agent_drafts d WHERE d.thread_id = t.id), '') \
FROM agent_threads t";

impl ThreadStore {
    pub async fn open(app_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(app_dir).map_err(|e| format!("create data dir: {e}"))?;
        let db_path = app_dir.join("agent.db");
        let db = libsql::Builder::new_local(&db_path)
            .build()
            .await
            .map_err(|e| format!("open agent.db: {e}"))?;
        let conn = db.connect().map_err(|e| format!("connect agent.db: {e}"))?;
        Self::init(conn).await
    }

    /// Shared setup: pragmas, schema, message sequence seed.
    async fn init(conn: Connection) -> Result<Self, String> {
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
            .await
            .map_err(|e| format!("agent.db pragmas: {e}"))?;
        conn.execute_batch(SCHEMA)
            .await
            .map_err(|e| format!("agent.db schema: {e}"))?;

        let mut rows = conn
            .query("SELECT COALESCE(MAX(seq), 0) FROM agent_messages", ())
            .await
            .map_err(|e| format!("agent.db seq init: {e}"))?;
        let max_seq: i64 = match rows.next().await {
            Ok(Some(row)) => row.get(0).unwrap_or(0),
            _ => 0,
        };

        Ok(Self {
            conn,
            next_seq: AtomicI64::new(max_seq + 1),
            active_children: Mutex::new(HashMap::new()),
        })
    }

    // ------------------------------------------------------------------
    // Threads CRUD
    // ------------------------------------------------------------------

    pub async fn create_thread(&self, owner_key: &str) -> Result<ThreadSummary, String> {
        let id = Uuid::new_v4().to_string();
        let now = now_millis();
        self.conn
            .execute(
                "INSERT INTO agent_threads (id, owner_key, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                params![id.clone(), owner_key.to_string(), now],
            )
            .await
            .map_err(|e| format!("create thread: {e}"))?;
        Ok(ThreadSummary {
            id,
            owner_key: owner_key.to_string(),
            title: String::new(),
            archived: false,
            created_at: now,
            updated_at: now,
            last_usage: None,
            model: None,
            draft: String::new(),
            message_count: 0,
            preview: None,
        })
    }

    /// Threads of one owner scope, excluding archived, most recent first.
    pub async fn list_threads(&self, owner_key: &str) -> Result<Vec<ThreadSummary>, String> {
        let mut rows = self
            .conn
            .query(
                &format!("{THREAD_SELECT} WHERE t.owner_key = ?1 AND t.archived = 0 ORDER BY t.updated_at DESC"),
                params![owner_key.to_string()],
            )
            .await
            .map_err(|e| format!("list threads: {e}"))?;
        let mut threads = collect_thread_rows(&mut rows).await?;
        self.fill_previews(&mut threads).await?;
        Ok(threads)
    }

    /// Re-anchor a thread to a different owner scope (settings "all
    /// threads" reassign action).
    pub async fn reassign_thread(&self, id: &str, owner_key: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE agent_threads SET owner_key = ?2 WHERE id = ?1",
                params![id.to_string(), owner_key.to_string()],
            )
            .await
            .map_err(|e| format!("reassign thread: {e}"))?;
        Ok(())
    }

    /// All threads across scopes, including archived (settings page
    /// "View all threads", design 04 §4.5).
    pub async fn list_all_threads(&self) -> Result<Vec<ThreadSummary>, String> {
        let mut rows = self
            .conn
            .query(
                &format!("{THREAD_SELECT} ORDER BY t.updated_at DESC"),
                (),
            )
            .await
            .map_err(|e| format!("list all threads: {e}"))?;
        let mut threads = collect_thread_rows(&mut rows).await?;
        self.fill_previews(&mut threads).await?;
        Ok(threads)
    }

    pub async fn rename_thread(&self, id: &str, title: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE agent_threads SET title = ?2 WHERE id = ?1",
                params![id.to_string(), title.to_string()],
            )
            .await
            .map_err(|e| format!("rename thread: {e}"))?;
        Ok(())
    }

    pub async fn set_archived(&self, id: &str, archived: bool) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE agent_threads SET archived = ?2 WHERE id = ?1",
                params![id.to_string(), archived as i64],
            )
            .await
            .map_err(|e| format!("archive thread: {e}"))?;
        Ok(())
    }

    pub async fn delete_thread(&self, id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM agent_threads WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| format!("delete thread: {e}"))?;
        Ok(())
    }

    pub async fn get_thread(&self, id: &str) -> Result<Option<ThreadSummary>, String> {
        let mut rows = self
            .conn
            .query(
                &format!("{THREAD_SELECT} WHERE t.id = ?1"),
                params![id.to_string()],
            )
            .await
            .map_err(|e| format!("get thread: {e}"))?;
        let mut threads = collect_thread_rows(&mut rows).await?;
        self.fill_previews(&mut threads).await?;
        Ok(threads.pop())
    }

    /// Persist (or clear, on empty text) the unsent composer draft.
    pub async fn set_draft(&self, id: &str, text: &str) -> Result<(), String> {
        if text.is_empty() {
            self.conn
                .execute(
                    "DELETE FROM agent_drafts WHERE thread_id = ?1",
                    params![id.to_string()],
                )
                .await
                .map_err(|e| format!("clear draft: {e}"))?;
        } else {
            self.conn
                .execute(
                    "INSERT INTO agent_drafts (thread_id, text, updated_at) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(thread_id) DO UPDATE \
                     SET text = excluded.text, updated_at = excluded.updated_at",
                    params![id.to_string(), text.to_string(), now_millis()],
                )
                .await
                .map_err(|e| format!("set draft: {e}"))?;
        }
        Ok(())
    }

    /// Fill `preview` (first user message, 30 chars) for empty-title threads
    /// that already have messages. One batched query for the whole list.
    async fn fill_previews(&self, threads: &mut [ThreadSummary]) -> Result<(), String> {
        let ids: Vec<String> = threads
            .iter()
            .filter(|t| t.title.is_empty() && t.message_count > 0)
            .map(|t| t.id.clone())
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let placeholders = (1..=ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT m.thread_id, m.content FROM agent_messages m \
             JOIN (SELECT thread_id, MIN(seq) AS min_seq FROM agent_messages \
                   WHERE role = 'user' AND thread_id IN ({placeholders}) GROUP BY thread_id) f \
             ON m.thread_id = f.thread_id AND m.seq = f.min_seq"
        );
        let params = libsql::params::Params::Positional(
            ids.iter()
                .map(|s| libsql::Value::from(s.clone()))
                .collect(),
        );
        let mut rows = self
            .conn
            .query(&sql, params)
            .await
            .map_err(|e| format!("preview query: {e}"))?;
        let mut contents: HashMap<String, String> = HashMap::new();
        while let Ok(Some(row)) = rows.next().await {
            let thread_id: String = row.get(0).map_err(|e| e.to_string())?;
            let content: String = row.get(1).map_err(|e| e.to_string())?;
            contents.insert(thread_id, content);
        }
        for t in threads.iter_mut() {
            let Some(content) = contents.get(&t.id) else { continue };
            let Ok(blocks) = serde_json::from_str::<Vec<super::types::ContentBlock>>(content)
            else {
                continue;
            };
            let text = blocks.iter().find_map(|b| match b {
                super::types::ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            });
            if let Some(text) = text.map(str::trim).filter(|s| !s.is_empty()) {
                t.preview = Some(text.chars().take(30).collect());
            }
        }
        Ok(())
    }

    pub async fn update_model_snapshot(
        &self,
        id: &str,
        selection: &ModelSelection,
    ) -> Result<(), String> {
        let json = serde_json::to_string(selection).map_err(|e| e.to_string())?;
        self.conn
            .execute(
                "UPDATE agent_threads SET model = ?2 WHERE id = ?1",
                params![id.to_string(), json],
            )
            .await
            .map_err(|e| format!("update model snapshot: {e}"))?;
        Ok(())
    }

    pub async fn record_usage(&self, id: &str, usage: &Usage) -> Result<(), String> {
        let json = serde_json::to_string(usage).map_err(|e| e.to_string())?;
        self.conn
            .execute(
                "UPDATE agent_threads SET last_usage = ?2, updated_at = ?3 WHERE id = ?1",
                params![id.to_string(), json, now_millis()],
            )
            .await
            .map_err(|e| format!("record usage: {e}"))?;
        Ok(())
    }

    pub async fn touch_thread(&self, id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE agent_threads SET updated_at = ?2 WHERE id = ?1",
                params![id.to_string(), now_millis()],
            )
            .await
            .map_err(|e| format!("touch thread: {e}"))?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Messages
    // ------------------------------------------------------------------

    /// Append a message as the latest child of `parent_id` (None = root)
    /// and make it the thread's active leaf.
    ///
    /// `id` pins the message id: the agent loop persists each round's
    /// assistant message under the id its streaming events already used, so
    /// the frontend sees one stable id across the whole round. None
    /// generates a fresh one.
    pub async fn append_message(
        &self,
        thread_id: &str,
        parent_id: Option<&str>,
        role: &str,
        content: Vec<super::types::ContentBlock>,
        usage: Option<&Usage>,
        metadata: Option<&super::types::MessageMetadata>,
        id: Option<&str>,
    ) -> Result<StoredMessage, String> {
        let id = id
            .map(str::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);

        let mut rows = match parent_id {
            Some(pid) => self
                .conn
                .query(
                    "SELECT COALESCE(MAX(branch_index) + 1, 0) FROM agent_messages WHERE parent_id = ?1",
                    params![pid.to_string()],
                )
                .await,
            None => self
                .conn
                .query(
                    "SELECT COALESCE(MAX(branch_index) + 1, 0) FROM agent_messages WHERE thread_id = ?1 AND parent_id IS NULL",
                    params![thread_id.to_string()],
                )
                .await,
        }
        .map_err(|e| format!("branch index: {e}"))?;
        let branch_index: i64 = match rows.next().await {
            Ok(Some(row)) => row.get(0).unwrap_or(0),
            _ => 0,
        };

        let content_json = serde_json::to_string(&content).map_err(|e| e.to_string())?;
        let usage_json = usage
            .map(|u| serde_json::to_string(u).map_err(|e| e.to_string()))
            .transpose()?;
        let metadata_json = metadata
            .map(|m| serde_json::to_string(m).map_err(|e| e.to_string()))
            .transpose()?;
        let now = now_millis();

        self.conn
            .execute(
                "INSERT INTO agent_messages \
                 (id, thread_id, parent_id, branch_index, seq, role, content, usage, metadata, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id.clone(),
                    thread_id.to_string(),
                    parent_id.map(|s| s.to_string()),
                    branch_index,
                    seq,
                    role.to_string(),
                    content_json,
                    usage_json,
                    metadata_json,
                    now
                ],
            )
            .await
            .map_err(|e| format!("insert message: {e}"))?;

        self.set_active_leaf(thread_id, &id).await?;
        self.remember_active_child(thread_id, parent_id, &id);

        Ok(StoredMessage {
            id,
            thread_id: thread_id.to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            branch_index,
            seq,
            role: role.to_string(),
            content,
            usage: usage.cloned(),
            metadata: metadata.cloned(),
            created_at: now,
        })
    }

    pub async fn get_message(&self, id: &str) -> Result<Option<StoredMessage>, String> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, thread_id, parent_id, branch_index, seq, role, content, usage, metadata, created_at \
                 FROM agent_messages WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| format!("get message: {e}"))?;
        let mut msgs = collect_message_rows(&mut rows).await?;
        Ok(msgs.pop())
    }

    /// Replace a message's content/usage/metadata (persist final state of a
    /// streamed assistant message when its run round completes).
    pub async fn update_message(
        &self,
        id: &str,
        content: &[super::types::ContentBlock],
        usage: Option<&Usage>,
        metadata: Option<&super::types::MessageMetadata>,
    ) -> Result<(), String> {
        let content_json = serde_json::to_string(content).map_err(|e| e.to_string())?;
        let usage_json = usage
            .map(|u| serde_json::to_string(u).map_err(|e| e.to_string()))
            .transpose()?;
        let metadata_json = metadata
            .map(|m| serde_json::to_string(m).map_err(|e| e.to_string()))
            .transpose()?;
        self.conn
            .execute(
                "UPDATE agent_messages SET content = ?2, usage = ?3, metadata = ?4 WHERE id = ?1",
                params![id.to_string(), content_json, usage_json, metadata_json],
            )
            .await
            .map_err(|e| format!("update message: {e}"))?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Active path & branch navigation
    // ------------------------------------------------------------------

    async fn active_leaf(&self, thread_id: &str) -> Result<Option<String>, String> {
        let mut rows = self
            .conn
            .query(
                "SELECT active_leaf_id FROM agent_threads WHERE id = ?1",
                params![thread_id.to_string()],
            )
            .await
            .map_err(|e| format!("active leaf: {e}"))?;
        match rows.next().await {
            Ok(Some(row)) => Ok(row.get::<Option<String>>(0).ok().flatten()),
            Ok(None) => Ok(None),
            Err(e) => Err(format!("active leaf: {e}")),
        }
    }

    async fn set_active_leaf(&self, thread_id: &str, leaf_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE agent_threads SET active_leaf_id = ?2, updated_at = ?3 WHERE id = ?1",
                params![thread_id.to_string(), leaf_id.to_string(), now_millis()],
            )
            .await
            .map_err(|e| format!("set active leaf: {e}"))?;
        Ok(())
    }

    fn remember_active_child(&self, thread_id: &str, parent_id: Option<&str>, child_id: &str) {
        let mut map = self.active_children.lock().unwrap();
        map.insert(
            (thread_id.to_string(), parent_id.map(|s| s.to_string())),
            child_id.to_string(),
        );
    }

    fn recalled_active_child(&self, thread_id: &str, parent_id: Option<&str>) -> Option<String> {
        let map = self.active_children.lock().unwrap();
        map.get(&(thread_id.to_string(), parent_id.map(|s| s.to_string())))
            .cloned()
    }

    /// Children of a parent, ordered by branch_index.
    async fn children_of(
        &self,
        thread_id: &str,
        parent_id: Option<&str>,
    ) -> Result<Vec<StoredMessage>, String> {
        let mut rows = match parent_id {
            Some(pid) => self
                .conn
                .query(
                    "SELECT id, thread_id, parent_id, branch_index, seq, role, content, usage, metadata, created_at \
                     FROM agent_messages WHERE parent_id = ?1 ORDER BY branch_index",
                    params![pid.to_string()],
                )
                .await,
            None => self
                .conn
                .query(
                    "SELECT id, thread_id, parent_id, branch_index, seq, role, content, usage, metadata, created_at \
                     FROM agent_messages WHERE thread_id = ?1 AND parent_id IS NULL ORDER BY branch_index",
                    params![thread_id.to_string()],
                )
                .await,
        }
        .map_err(|e| format!("children query: {e}"))?;
        collect_message_rows(&mut rows).await
    }

    /// Walk from the active leaf up to the root; return the path in display
    /// order (root first), annotated with sibling branch info.
    pub async fn snapshot(&self, thread_id: &str) -> Result<ThreadSnapshot, String> {
        let thread = self
            .get_thread(thread_id)
            .await?
            .ok_or_else(|| format!("thread not found: {thread_id}"))?;

        let leaf_id = match self.active_leaf(thread_id).await? {
            Some(l) => Some(l),
            None => {
                // No active leaf yet (fresh thread, or pre-branching data):
                // fall back to the latest message so history still shows.
                let mut rows = self
                    .conn
                    .query(
                        "SELECT id FROM agent_messages WHERE thread_id = ?1 ORDER BY seq DESC LIMIT 1",
                        params![thread_id.to_string()],
                    )
                    .await
                    .map_err(|e| format!("latest message: {e}"))?;
                match rows.next().await {
                    Ok(Some(row)) => row.get::<String>(0).ok(),
                    _ => None,
                }
            }
        };

        let mut path: Vec<StoredMessage> = Vec::new();
        let mut cursor = leaf_id;
        while let Some(id) = cursor {
            match self.get_message(&id).await? {
                Some(msg) => {
                    cursor = msg.parent_id.clone();
                    path.push(msg);
                }
                None => break,
            }
        }
        path.reverse();

        // Seed branch memory from the active path so a fresh session can
        // navigate back to where the user was.
        for msg in &path {
            self.remember_active_child(thread_id, msg.parent_id.as_deref(), &msg.id);
        }

        let mut messages = Vec::with_capacity(path.len());
        for msg in path {
            let siblings = self
                .children_of(thread_id, msg.parent_id.as_deref())
                .await?;
            let branch = BranchInfo {
                index: msg.branch_index + 1,
                count: siblings.len() as i64,
            };
            messages.push(PathMessage {
                message: msg,
                branch,
            });
        }

        Ok(ThreadSnapshot { thread, messages })
    }

    /// Switch the displayed branch at `at_message_id` one sibling left/right
    /// and persist the new active leaf (design 01 §2.4). Returns the new
    /// snapshot.
    pub async fn switch_branch(
        &self,
        thread_id: &str,
        at_message_id: &str,
        direction: &str,
    ) -> Result<ThreadSnapshot, String> {
        let node = self
            .get_message(at_message_id)
            .await?
            .ok_or_else(|| format!("message not found: {at_message_id}"))?;
        let siblings = self.children_of(thread_id, node.parent_id.as_deref()).await?;
        if siblings.len() < 2 {
            return self.snapshot(thread_id).await;
        }
        let pos = siblings
            .iter()
            .position(|m| m.id == at_message_id)
            .ok_or_else(|| "message not among its siblings".to_string())?;
        let new_pos = match direction {
            "prev" => pos.saturating_sub(1),
            "next" => (pos + 1).min(siblings.len() - 1),
            other => return Err(format!("invalid direction: {other}")),
        };
        if new_pos == pos {
            return self.snapshot(thread_id).await;
        }
        let mut cursor = siblings[new_pos].clone();
        self.remember_active_child(thread_id, node.parent_id.as_deref(), &cursor.id);

        // Descend to a leaf: prefer the branch the user last viewed under
        // each node, otherwise the newest fork (highest branch_index).
        loop {
            let children = self.children_of(thread_id, Some(&cursor.id)).await?;
            if children.is_empty() {
                break;
            }
            let next = match self.recalled_active_child(thread_id, Some(&cursor.id)) {
                Some(id) => children
                    .iter()
                    .find(|c| c.id == id)
                    .cloned()
                    .unwrap_or_else(|| children.last().unwrap().clone()),
                None => children.last().unwrap().clone(),
            };
            self.remember_active_child(thread_id, Some(&cursor.id), &next.id);
            cursor = next;
        }

        self.set_active_leaf(thread_id, &cursor.id).await?;
        self.snapshot(thread_id).await
    }
}

async fn collect_thread_rows(rows: &mut libsql::Rows) -> Result<Vec<ThreadSummary>, String> {
    let mut out = Vec::new();
    while let Ok(Some(row)) = rows.next().await {
        let last_usage: Option<String> = row.get(6).ok().flatten();
        let model: Option<String> = row.get(7).ok().flatten();
        out.push(ThreadSummary {
            id: row.get(0).map_err(|e| e.to_string())?,
            owner_key: row.get(1).map_err(|e| e.to_string())?,
            title: row.get(2).map_err(|e| e.to_string())?,
            archived: row.get::<i64>(3).map_err(|e| e.to_string())? != 0,
            created_at: row.get(4).map_err(|e| e.to_string())?,
            updated_at: row.get(5).map_err(|e| e.to_string())?,
            last_usage: last_usage.and_then(|s| serde_json::from_str(&s).ok()),
            model: model.and_then(|s| serde_json::from_str(&s).ok()),
            message_count: row.get(8).map_err(|e| e.to_string())?,
            draft: row.get(9).map_err(|e| e.to_string())?,
            preview: None,
        });
    }
    Ok(out)
}

async fn collect_message_rows(rows: &mut libsql::Rows) -> Result<Vec<StoredMessage>, String> {
    let mut out = Vec::new();
    while let Ok(Some(row)) = rows.next().await {
        let content: String = row.get(6).map_err(|e| e.to_string())?;
        let usage: Option<String> = row.get(7).ok().flatten();
        let metadata: Option<String> = row.get(8).ok().flatten();
        out.push(StoredMessage {
            id: row.get(0).map_err(|e| e.to_string())?,
            thread_id: row.get(1).map_err(|e| e.to_string())?,
            parent_id: row.get(2).ok().flatten(),
            branch_index: row.get(3).map_err(|e| e.to_string())?,
            seq: row.get(4).map_err(|e| e.to_string())?,
            role: row.get(5).map_err(|e| e.to_string())?,
            content: serde_json::from_str(&content).unwrap_or_default(),
            usage: usage.and_then(|s| serde_json::from_str(&s).ok()),
            metadata: metadata.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: row.get(9).map_err(|e| e.to_string())?,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory store: tests must not touch the filesystem.
    async fn mem_store() -> ThreadStore {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        ThreadStore::init(conn).await.unwrap()
    }

    #[tokio::test]
    async fn create_list_reassign_by_owner() {
        let store = mem_store().await;

        let t1 = store.create_thread("session:s1").await.unwrap();
        let _t2 = store
            .create_thread("link:root@example.com:22")
            .await
            .unwrap();

        // Strict owner matching: same identity, different scopes stay separate.
        assert_eq!(store.list_threads("session:s1").await.unwrap().len(), 1);
        assert_eq!(
            store
                .list_threads("link:root@example.com:22")
                .await
                .unwrap()
                .len(),
            1
        );

        store.reassign_thread(&t1.id, "session:s2").await.unwrap();
        assert_eq!(store.list_threads("session:s1").await.unwrap().len(), 0);
        let moved = store.list_threads("session:s2").await.unwrap();
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].owner_key, "session:s2");
    }
}
