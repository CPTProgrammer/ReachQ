//! edit_file tool (design 03 §3): atomic old_text/new_text edits with
//! line-level fuzzy matching, loud indent-mismatch rejection, and
//! all-or-nothing write after approval.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::edit_match::{describe_indent, first_line_indent_mismatch, fuzzy_find, unified_diff};
use crate::agent::remote_fs::{self, PreparedWrite};
use crate::agent::tools::{
    deserialize_maybe_stringified, err_text, ok_text, AgentTool, ToolContext,
};
use crate::agent::providers::ToolSchema;
use crate::agent::types::ToolResult;
use crate::agent::tools::write_file::execute_prepared_write;

const DESCRIPTION: &str = "\
Applies edits to an existing file on the remote host.
- Before using this tool, use read_file to understand the file's current content.
- To create a file or fully overwrite one, use write_file instead.
- Each edit finds `old_text` in the file and replaces it with `new_text`. Edits apply sequentially, each against the result of the previous one.
- Matching tolerates minor whitespace differences, but you should still copy text exactly from read_file output, stripping the line-number prefix.
- Be minimal: for unique lines include only those lines; for repeated lines include enough surrounding context to uniquely identify the location.";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct Edit {
    /// Exact text to find (copied from read_file output, line numbers stripped).
    old_text: String,
    /// Replacement text.
    new_text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EditFileInput {
    /// Absolute path of the file on the remote host.
    path: String,
    /// Edits applied sequentially, each against the result of the previous.
    #[serde(deserialize_with = "deserialize_maybe_stringified")]
    #[schemars(with = "Vec<Edit>")]
    edits: Vec<Edit>,
}

pub struct EditFileTool;

/// Apply all edits in memory; shared by approval_payload and run.
/// Returns the stash payload (diff etc) or the tool error to feed back.
async fn prepare_edit(ctx: &ToolContext, path: &str, edits: &[Edit]) -> Result<Value, ToolResult> {
    if path.is_empty() || path.contains('\0') {
        return Err(err_text("Can't edit file: invalid path"));
    }

    let fs = ctx.remote_fs(None).await?;
    let stat = fs.stat(path).await.map_err(|e| match e {
        crate::sftp::browser::SftpBrowserError::PathNotFound(_) => {
            err_text("Can't edit file: path not found")
        }
        other => remote_fs::map_stat_error(path, &other),
    })?;
    if stat.is_dir {
        return Err(err_text("Can't edit file: path is a directory"));
    }

    // Read-before-edit gate (design 03 §2 behavior 0).
    if !ctx.read_paths.lock().unwrap().contains(path) {
        return Err(err_text(
            "You must read the file with read_file before editing it.",
        ));
    }

    // Fingerprint at the last read (for the changed-since-read hint).
    let last_read_fingerprint = ctx
        .read_cache
        .lock()
        .unwrap()
        .get(&(ctx.identity.clone(), path.to_string()))
        .map(|e| e.fingerprint);

    let decoded = remote_fs::read_pipeline(&fs, path).await?;
    let file_changed_since_read = matches!(
        last_read_fingerprint,
        Some(fp) if fp != decoded.fingerprint
    );

    let mut buffer = decoded.text.clone();
    for (i, edit) in edits.iter().enumerate() {
        let ranges = fuzzy_find(&buffer, &edit.old_text);
        match ranges.len() {
            0 => {
                let hint = if file_changed_since_read {
                    " The file has changed on disk since you last read it."
                } else {
                    ""
                };
                return Err(err_text(format!(
                    "Could not find matching text for edit at index {}. The old_text did not match any content in the file.{} Please read the file again to get the current content.",
                    i, hint
                )));
            }
            1 => {
                let range = &ranges[0];
                if let Some(m) = first_line_indent_mismatch(&buffer, range, &edit.old_text) {
                    return Err(err_text(format!(
                        "Edit {} was rejected: the matched text at line {} is indented differently than your old_text (the file uses {}, your old_text uses {}). Re-read the file and copy the text exactly, including its indentation. No edits were made.",
                        i,
                        m.line,
                        describe_indent(&m.file_indent),
                        describe_indent(&m.old_text_indent),
                    )));
                }
                buffer.replace_range(range.clone(), &edit.new_text);
            }
            _ => {
                let lines: Vec<String> = ranges
                    .iter()
                    .map(|r| {
                        (buffer[..r.start].bytes().filter(|b| *b == b'\n').count() + 1)
                            .to_string()
                    })
                    .collect();
                return Err(err_text(format!(
                    "Edit {} matched multiple locations in the file at lines: {}. Please provide more context in old_text to uniquely identify the location.",
                    i,
                    lines.join(", ")
                )));
            }
        }
    }

    if buffer == decoded.text {
        return Ok(Value::Null); // "No edits were made."
    }

    let diff = unified_diff(&decoded.text, &buffer);
    ctx.pending_writes.lock().unwrap().insert(
        ctx.tool_call_id.clone(),
        PreparedWrite {
            path: path.to_string(),
            new_text: buffer,
            diff: diff.clone(),
            expected: decoded.fingerprint,
            encoding_label: decoded.encoding.name().to_string(),
            line_ending: decoded.line_ending,
        },
    );
    Ok(serde_json::json!({ "diff": diff, "path": path }))
}

#[async_trait]
impl AgentTool for EditFileTool {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn default_requires_approval(&self) -> bool {
        true
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: DESCRIPTION.to_string(),
            parameters: serde_json::to_value(schemars::schema_for!(EditFileInput))
                .unwrap_or_default(),
        }
    }

    fn title(&self, args: &Value) -> String {
        args.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string()
    }

    async fn approval_payload(&self, args: &Value, ctx: &ToolContext) -> Option<Value> {
        let input: EditFileInput = serde_json::from_value(args.clone()).ok()?;
        prepare_edit(ctx, input.path.trim(), &input.edits).await.ok()
    }

    async fn run(
        &self,
        args: Value,
        ctx: &ToolContext,
        _cancel: CancellationToken,
    ) -> Result<ToolResult, ToolResult> {
        let input: EditFileInput = serde_json::from_value(args)
            .map_err(|e| err_text(format!("Invalid arguments: {e}")))?;
        let path = input.path.trim().to_string();
        if input.edits.is_empty() {
            return Ok(ok_text("No edits were made."));
        }

        // No approval happened (tool pre-approved): nothing stashed.
        if !ctx
            .pending_writes
            .lock()
            .unwrap()
            .contains_key(&ctx.tool_call_id)
        {
            match prepare_edit(ctx, &path, &input.edits).await? {
                Value::Null => return Ok(ok_text("No edits were made.")),
                _ => {}
            }
        } else if ctx
            .pending_writes
            .lock()
            .unwrap()
            .get(&ctx.tool_call_id)
            .map(|p| p.diff.is_empty())
            .unwrap_or(false)
        {
            return Ok(ok_text("No edits were made."));
        }

        execute_prepared_write(ctx, |p, diff, _bytes| {
            format!("Edited {}:\n\n```diff\n{}\n```", p, diff)
        })
        .await
    }
}
