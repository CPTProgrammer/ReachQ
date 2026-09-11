//! write_file tool (design 03 §2): whole-file overwrite with the
//! read-before-write gate, fingerprint race protection, and a prepared
//! write stashed between approval and execution.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::agent::tools::edit_match::{format_hunks, structured_diff, DiffHunk, DiffPreview};
use crate::agent::remote_fs::{self, Fingerprint, LineEnding, PreparedWrite, ReadCacheEntry};
use crate::agent::tools::{err_text, AgentTool, ToolContext};
use crate::agent::providers::ToolSchema;
use crate::agent::types::ToolResult;

const DESCRIPTION: &str = "\
Creates a new file or completely overwrites an existing file on the remote host.
- To make granular edits to an existing file, prefer the `edit_file` tool instead.
- Before creating a new file, verify the parent directory exists with list_directory. This tool will NOT create missing directories.
- You must read a file with `read_file` before overwriting it.";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WriteFileInput {
    /// Absolute path of the file on the remote host.
    path: String,
    /// The complete new content of the file. Overwrites everything.
    content: String,
}

pub struct WriteFileTool;

/// Shared preparation for approval_payload() and run(): validates the path,
/// reads the current content (existing files), computes the diff, and
/// stashes a PreparedWrite keyed by tool_call_id.
pub(crate) async fn prepare_write(
    ctx: &ToolContext,
    path: &str,
    new_content: &str,
) -> Result<serde_json::Value, ToolResult> {
    if path.is_empty() || path.contains('\0') {
        return Err(err_text("Can't create file: invalid filename"));
    }

    let fs = ctx.remote_fs(None).await?;
    let stat = fs.stat(path).await;

    let (old_text, fingerprint, encoding_label, line_ending) = match stat {
        Ok(stat) => {
            if stat.is_dir {
                return Err(err_text("Can't write to file: path is a directory"));
            }
            // Read-before-write gate (design 03 §2 behavior 0).
            if !ctx.read_paths.lock().unwrap().contains(path) {
                return Err(err_text(
                    "You must read the file with read_file before overwriting it.",
                ));
            }
            let decoded = remote_fs::read_pipeline(&fs, path).await?;
            (
                decoded.text,
                decoded.fingerprint,
                decoded.encoding.name().to_string(),
                decoded.line_ending,
            )
        }
        Err(crate::sftp::browser::SftpBrowserError::PathNotFound(_)) => {
            // New file: verify the parent directory exists.
            let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
            let parent = if parent.is_empty() { "/" } else { parent };
            match fs.stat(parent).await {
                Ok(pstat) if pstat.is_dir => {}
                Ok(_) => {
                    return Err(err_text("Can't create file: parent directory doesn't exist"))
                }
                Err(_) => {
                    return Err(err_text("Can't create file: parent directory doesn't exist"))
                }
            }
            (
                String::new(),
                Fingerprint::Absent,
                "UTF-8".to_string(),
                LineEnding::Lf,
            )
        }
        Err(e) => return Err(remote_fs::map_stat_error(path, &e)),
    };

    let is_new_file = matches!(fingerprint, Fingerprint::Absent);
    let hunks = structured_diff(&old_text, new_content);
    let diff_text = format_hunks(&hunks);
    ctx.pending_writes.lock().unwrap().insert(
        ctx.tool_call_id.clone(),
        PreparedWrite {
            path: path.to_string(),
            new_text: new_content.to_string(),
            diff: diff_text,
            hunks: hunks.clone(),
            expected: fingerprint,
            encoding_label,
            line_ending,
        },
    );

    let preview = DiffPreview {
        path: path.to_string(),
        is_new_file,
        hunks,
    };
    Ok(serde_json::json!({
        "diff": preview,
        "path": path,
        "isNewFile": is_new_file,
    }))
}

/// Streaming-preview entry (agent::preview): structured hunks of the current
/// content vs the (possibly still partial) new content. Pure computation.
pub(crate) fn preview_write(old_text: &str, new_content: &str) -> Vec<DiffHunk> {
    structured_diff(old_text, new_content)
}

/// Execute a stashed (or freshly prepared) write. Shared by write_file and
/// edit_file.
pub(crate) async fn execute_prepared_write(
    ctx: &ToolContext,
    success_text: impl FnOnce(&str, &str, usize) -> String,
) -> Result<ToolResult, ToolResult> {
    let plan = ctx
        .pending_writes
        .lock()
        .unwrap()
        .remove(&ctx.tool_call_id)
        .ok_or_else(|| err_text("Internal error: prepared write missing; please retry."))?;

    let fs = ctx.remote_fs(None).await?;
    let encoding = encoding_rs::Encoding::for_label(plan.encoding_label.as_bytes())
        .unwrap_or(encoding_rs::UTF_8);
    let (new_fingerprint, bytes_written) = remote_fs::write_pipeline(
        &fs,
        &plan.path,
        &plan.new_text,
        plan.expected,
        encoding,
        plan.line_ending,
    )
    .await?;

    // Keep the read cache hot with what we just wrote (design 03 §1.7).
    ctx.read_cache.lock().unwrap().insert(
        (ctx.identity.clone(), plan.path.clone()),
        ReadCacheEntry {
            text: plan.new_text.clone(),
            encoding_label: plan.encoding_label.clone(),
            line_ending: plan.line_ending,
            fingerprint: new_fingerprint,
            fetched_at: std::time::Instant::now(),
        },
    );

    let diff = plan.diff.clone();
    let is_new_file = matches!(plan.expected, Fingerprint::Absent);
    let preview = DiffPreview {
        path: plan.path.clone(),
        is_new_file,
        hunks: plan.hunks.clone(),
    };
    Ok(ToolResult {
        llm_text: success_text(&plan.path, &diff, bytes_written),
        is_error: false,
        ui_payload: Some(serde_json::json!({
            "diff": preview,
            "path": plan.path,
            "isNewFile": is_new_file,
        })),
    })
}

#[async_trait]
impl AgentTool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn default_requires_approval(&self) -> bool {
        true
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: DESCRIPTION.to_string(),
            parameters: serde_json::to_value(schemars::schema_for!(WriteFileInput))
                .unwrap_or_default(),
        }
    }

    fn title(&self, args: &Value) -> String {
        args.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string()
    }

    async fn approval_payload(&self, args: &Value, ctx: &ToolContext) -> Option<Value> {
        let input: WriteFileInput = serde_json::from_value(args.clone()).ok()?;
        match prepare_write(ctx, input.path.trim(), &input.content).await {
            Ok(payload) => Some(payload),
            // Path/gate errors are surfaced at run time as tool results;
            // approval cards only exist when there is something to approve.
            Err(_) => None,
        }
    }

    async fn run(
        &self,
        args: Value,
        ctx: &ToolContext,
        _cancel: CancellationToken,
    ) -> Result<ToolResult, ToolResult> {
        let input: WriteFileInput = serde_json::from_value(args)
            .map_err(|e| err_text(format!("Invalid arguments: {e}")))?;
        let path = input.path.trim().to_string();

        // No approval happened (tool is pre-approved): nothing stashed.
        if !ctx
            .pending_writes
            .lock()
            .unwrap()
            .contains_key(&ctx.tool_call_id)
        {
            prepare_write(ctx, &path, &input.content).await?;
        }

        execute_prepared_write(ctx, |p, _diff, bytes| {
            format!("Wrote {} ({} bytes).", p, bytes)
        })
        .await
    }
}
