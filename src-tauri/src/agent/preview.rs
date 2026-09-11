//! Live preview while tool-call arguments stream (design: the
//! `tool_call_args_patched` / `tool_call_preview` events). A throttle tick in
//! the agent loop re-parses the accumulated args JSON prefix with a tolerant
//! parser (jiter, trailing partial strings included), emits the repaired
//! JSON for every tool, and for write_file/edit_file computes a structured
//! diff preview with the exact same fuzzy-match + diff code as the execution
//! path.
//!
//! Previews are advisory: every failure is silent, and the final arguments
//! parse at execution time stays authoritative.

use std::collections::{HashMap, HashSet};

use tauri::AppHandle;

use super::agent_loop::emit;
use super::events::AgentEvent;
use super::remote_fs::{self, AgentRemoteFs};
use super::tools::edit_match::DiffPreview;
use super::tools::{edit_file, write_file};
use super::AgentDeps;
use crate::sftp::browser::SftpBrowserError;

/// Value of the shared previews map (`AgentState::previews`).
#[derive(Debug, Clone)]
pub struct PreviewEntry {
    /// Owning thread, for run-exit cleanup.
    pub thread_id: String,
    pub preview: DiffPreview,
}

/// What a diff preview is computed against; resolved once per tool call.
enum SourceText {
    /// Current file content (LF-normalized).
    Existing(String),
    /// write_file target does not exist on disk.
    NewFile,
    /// Resolution failed (connection/read error, edit of a missing file).
    /// Never retried, so a broken target doesn't cost an SSH read per tick.
    Failed,
}

/// Per-tool-call streaming state.
struct CallState {
    /// Last patched-args JSON emitted (dedup).
    last_patched: Option<String>,
    /// Source text resolution, attempted at most once per call.
    source: Option<SourceText>,
    /// Last preview emitted, serialized (dedup).
    last_preview: Option<String>,
}

/// Streaming-preview tracker for one provider round: a dirty set fed by
/// args deltas, plus per-call state reused across ticks.
#[derive(Default)]
pub struct StreamPreviews {
    dirty: HashSet<String>,
    calls: HashMap<String, CallState>,
}

impl StreamPreviews {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a tool call's accumulated args as changed since the last tick.
    pub fn mark_dirty(&mut self, tool_call_id: &str) {
        self.dirty.insert(tool_call_id.to_string());
    }

    /// Drain the dirty set (the loop calls this right before `tick`).
    pub fn take_dirty(&mut self) -> Vec<String> {
        std::mem::take(&mut self.dirty).into_iter().collect()
    }

    /// One throttle tick. `calls` snapshots the dirty calls as
    /// (tool_call_id, tool name, accumulated args JSON).
    pub async fn tick(
        &mut self,
        app: &AppHandle,
        deps: &AgentDeps,
        identity: &str,
        thread_id: &str,
        calls: Vec<(String, String, String)>,
    ) {
        for (tool_call_id, name, args_json) in calls {
            self.tick_call(app, deps, identity, thread_id, &tool_call_id, &name, &args_json)
                .await;
        }
    }

    async fn tick_call(
        &mut self,
        app: &AppHandle,
        deps: &AgentDeps,
        identity: &str,
        thread_id: &str,
        tool_call_id: &str,
        name: &str,
        args_json: &str,
    ) {
        let Some(args) = parse_partial_args(args_json) else {
            return;
        };
        let Ok(patched) = serde_json::to_string(&args) else {
            return;
        };

        let state = self
            .calls
            .entry(tool_call_id.to_string())
            .or_insert_with(|| CallState {
                last_patched: None,
                source: None,
                last_preview: None,
            });

        if state.last_patched.as_deref() != Some(patched.as_str()) {
            state.last_patched = Some(patched.clone());
            emit(app, identity, AgentEvent::ToolCallArgsPatched {
                thread_id: thread_id.to_string(),
                tool_call_id: tool_call_id.to_string(),
                args_json: patched,
            });
        }

        let is_write = match name {
            "write_file" => true,
            "edit_file" => false,
            _ => return,
        };
        let Some(obj) = args.as_object() else {
            return;
        };
        // A value is only known complete once a LATER key has started: the
        // last value in a partial parse may still be a growing string.
        let follows_path = if is_write { "content" } else { "edits" };
        if !obj.contains_key(follows_path) {
            return;
        }
        let Some(path) = obj
            .get("path")
            .and_then(|p| p.as_str())
            .map(str::trim)
            .filter(|p| !p.is_empty())
        else {
            return;
        };

        if state.source.is_none() {
            state.source = Some(resolve_source(app, deps, identity, path, is_write).await);
        }
        let (old_text, is_new_file) = match state.source.as_ref() {
            Some(SourceText::Existing(text)) => (text.clone(), false),
            Some(SourceText::NewFile) => (String::new(), true),
            _ => return,
        };

        enum Compute {
            Write(String),
            Edit(Vec<edit_file::Edit>),
        }
        let compute = if is_write {
            // content itself may be a partial string; diff against the prefix.
            let Some(content) = obj.get("content").and_then(|c| c.as_str()).map(str::to_string)
            else {
                return;
            };
            Compute::Write(content)
        } else {
            let Some(edits) = obj.get("edits").and_then(|e| e.as_array()) else {
                return;
            };
            let mut parsed: Vec<edit_file::Edit> = Vec::new();
            for item in edits {
                let Some(edit) = item.as_object() else {
                    continue;
                };
                // An edit applies once new_text exists (old_text is then
                // complete); the last edit's new_text may be partial.
                let (Some(old_text), Some(new_text)) = (
                    edit.get("old_text").and_then(|v| v.as_str()),
                    edit.get("new_text").and_then(|v| v.as_str()),
                ) else {
                    continue;
                };
                parsed.push(edit_file::Edit {
                    old_text: old_text.to_string(),
                    new_text: new_text.to_string(),
                });
            }
            if parsed.is_empty() {
                return;
            }
            Compute::Edit(parsed)
        };

        // Fuzzy matching + diffing are pure CPU; keep them off the runtime.
        let hunks = match tokio::task::spawn_blocking(move || match compute {
            Compute::Write(content) => Ok(write_file::preview_write(&old_text, &content)),
            Compute::Edit(edits) => edit_file::preview_edit(&old_text, &edits),
        })
        .await
        {
            Ok(Ok(hunks)) => hunks,
            _ => return,
        };

        let preview = DiffPreview {
            path: path.to_string(),
            is_new_file,
            hunks,
        };
        let Ok(serialized) = serde_json::to_string(&preview) else {
            return;
        };
        if state.last_preview.as_deref() == Some(serialized.as_str()) {
            return;
        }
        state.last_preview = Some(serialized);
        deps.agent.previews.lock().await.insert(
            tool_call_id.to_string(),
            PreviewEntry {
                thread_id: thread_id.to_string(),
                preview: preview.clone(),
            },
        );
        emit(app, identity, AgentEvent::ToolCallPreview {
            thread_id: thread_id.to_string(),
            tool_call_id: tool_call_id.to_string(),
            preview,
        });
    }
}

/// Tolerant parse of an args JSON prefix: partial containers are closed and
/// a trailing partial string is kept as-is. None when the prefix is not yet
/// parseable at all (e.g. empty).
///
/// `parse_with_config` (not `parse_owned`): in partial mode it skips the
/// trailing-content check, which would otherwise reject a prefix whose
/// truncation point is a half-written object KEY (the parser then stands
/// mid-input, and `finish()` reports TrailingCharacters).
fn parse_partial_args(args_json: &str) -> Option<serde_json::Value> {
    let value = jiter::JsonValue::parse_with_config(
        args_json.as_bytes(),
        false,
        jiter::PartialMode::TrailingStrings,
    )
    .ok()?;
    Some(jiter_to_serde(&value))
}

/// jiter's JsonValue has no serde impl; convert by hand. Out-of-i64-range
/// integers degrade to strings and non-finite floats to null — fine for a
/// preview; the authoritative parse happens at execution.
fn jiter_to_serde(value: &jiter::JsonValue) -> serde_json::Value {
    use serde_json::Value;
    match value {
        jiter::JsonValue::Null => Value::Null,
        jiter::JsonValue::Bool(b) => Value::Bool(*b),
        jiter::JsonValue::Int(i) => Value::Number((*i).into()),
        jiter::JsonValue::BigInt(b) => Value::String(b.to_string()),
        jiter::JsonValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        jiter::JsonValue::Str(s) => Value::String(s.to_string()),
        jiter::JsonValue::Array(items) => Value::Array(items.iter().map(jiter_to_serde).collect()),
        jiter::JsonValue::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(k, v)| (k.to_string(), jiter_to_serde(v)))
                .collect(),
        ),
    }
}

/// Current file content for the diff's old side: a read-cache hit is used
/// directly; otherwise the file is read through the normal pipeline WITHOUT
/// populating the read cache or the read-before-write gate — the preview is
/// a stateless reader; both belong to the model-visible read_file path.
async fn resolve_source(
    app: &AppHandle,
    deps: &AgentDeps,
    identity: &str,
    path: &str,
    is_write: bool,
) -> SourceText {
    let cached = deps
        .agent
        .read_cache
        .lock()
        .unwrap()
        .get(&(identity.to_string(), path.to_string()))
        .map(|e| e.text.clone());
    if let Some(text) = cached {
        return SourceText::Existing(text);
    }

    let fs = match AgentRemoteFs::for_identity(
        &deps.ssh_manager,
        &deps.sftp_backends,
        app.clone(),
        identity,
        None,
    )
    .await
    {
        Ok(fs) => fs,
        Err(_) => return SourceText::Failed,
    };
    match fs.stat(path).await {
        Ok(stat) if !stat.is_dir => match remote_fs::read_pipeline(&fs, path).await {
            Ok(decoded) => SourceText::Existing(decoded.text),
            Err(_) => SourceText::Failed,
        },
        Err(SftpBrowserError::PathNotFound(_)) if is_write => SourceText::NewFile,
        _ => SourceText::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_args_keep_trailing_string() {
        let v = parse_partial_args(r#"{"path": "/etc/app.conf", "content": "hello wor"#)
            .expect("parseable prefix");
        assert_eq!(v["path"], "/etc/app.conf");
        // The trailing partial string is included as-is.
        assert_eq!(v["content"], "hello wor");
    }

    #[test]
    fn partial_args_drop_incomplete_member() {
        // The "content" key has not completed: the member is dropped, which
        // the preview uses as the path-completeness signal.
        let v = parse_partial_args(r#"{"path": "/etc/app.conf", "cont"#).unwrap();
        assert_eq!(v, serde_json::json!({ "path": "/etc/app.conf" }));
    }

    #[test]
    fn partial_args_partial_edit_array() {
        let v = parse_partial_args(
            r#"{"path": "/a", "edits": [{"old_text": "foo", "new_text": "ba"#,
        )
        .unwrap();
        assert_eq!(v["edits"][0]["old_text"], "foo");
        assert_eq!(v["edits"][0]["new_text"], "ba");
    }

    #[test]
    fn partial_args_complete_roundtrip() {
        let v = parse_partial_args(r#"{"path": "/a", "content": "x", "n": 3}"#).unwrap();
        assert_eq!(v, serde_json::json!({ "path": "/a", "content": "x", "n": 3 }));
    }

    #[test]
    fn partial_args_empty_is_none() {
        assert!(parse_partial_args("").is_none());
    }
}
