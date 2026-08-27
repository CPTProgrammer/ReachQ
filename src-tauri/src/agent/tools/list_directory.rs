//! list_directory tool (design 03 §4): direct children only, grouped
//! Folders/Files output, per-group entry cap.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::agent::remote_fs;
use crate::agent::tools::{err_text, ok_text, AgentTool, ToolContext, ToolOptionKind, ToolOptionSpec};
use crate::agent::providers::ToolSchema;
use crate::agent::types::ToolResult;

const DESCRIPTION: &str = "\
Lists files and directories in a given path on the remote host. Not recursive. Prefer the terminal tool (find/grep) when searching for files.";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListDirectoryInput {
    /// Absolute path of the directory on the remote host.
    path: String,
}

pub struct ListDirectoryTool;

pub const DEFAULT_MAX_ENTRIES: usize = 500;

#[async_trait]
impl AgentTool for ListDirectoryTool {
    fn name(&self) -> &'static str {
        "list_directory"
    }

    fn default_requires_approval(&self) -> bool {
        false
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: DESCRIPTION.to_string(),
            parameters: serde_json::to_value(schemars::schema_for!(ListDirectoryInput))
                .unwrap_or_default(),
        }
    }

    fn settings_schema(&self) -> Vec<ToolOptionSpec> {
        vec![ToolOptionSpec {
            key: "max_entries".into(),
            description: "Maximum entries per group (Folders/Files) before the output is truncated".into(),
            kind: ToolOptionKind::Number { min: Some(10.0), max: None },
            default: serde_json::json!(DEFAULT_MAX_ENTRIES),
        }]
    }

    fn title(&self, args: &Value) -> String {
        args.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string()
    }

    async fn run(
        &self,
        args: Value,
        ctx: &ToolContext,
        _cancel: CancellationToken,
    ) -> Result<ToolResult, ToolResult> {
        let input: ListDirectoryInput = serde_json::from_value(args)
            .map_err(|e| err_text(format!("Invalid arguments: {e}")))?;
        let path = input.path.trim().to_string();
        if path.is_empty() {
            return Err(err_text("Invalid arguments: path is empty"));
        }

        let max_entries = ctx
            .options
            .get("max_entries")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_MAX_ENTRIES);

        let fs = ctx.remote_fs(None).await?;

        // Distinguish "not a directory" from a plain listing failure.
        match fs.stat(&path).await {
            Ok(stat) if !stat.is_dir => {
                return Err(err_text(format!("{} is not a directory.", path)))
            }
            Ok(_) => {}
            Err(crate::sftp::browser::SftpBrowserError::PathNotFound(_)) => {
                return Err(err_text(format!("Path not found: {}", path)))
            }
            Err(e) => return Err(remote_fs::map_stat_error(&path, &e)),
        }

        let entries = fs
            .list_dir(&path)
            .await
            .map_err(|e| remote_fs::map_stat_error(&path, &e))?;

        if entries.is_empty() {
            return Ok(ok_text(format!("{} is empty.", path)));
        }

        let folders: Vec<_> = entries.iter().filter(|e| e.is_dir).collect();
        let files: Vec<_> = entries.iter().filter(|e| !e.is_dir).collect();

        let mut out = String::new();
        let mut overflow = 0usize;

        if !folders.is_empty() {
            out.push_str("# Folders:\n");
            for e in folders.iter().take(max_entries) {
                out.push_str(&e.path);
                out.push('\n');
            }
            overflow += folders.len().saturating_sub(max_entries);
        }
        if !files.is_empty() {
            out.push_str("\n# Files:\n");
            for e in files.iter().take(max_entries) {
                out.push_str(&e.path);
                out.push('\n');
            }
            overflow += files.len().saturating_sub(max_entries);
        }

        if overflow > 0 {
            out.push_str(&format!(
                "\n... and {} more entries. Narrow down with the terminal tool (find/ls | grep).\n",
                overflow
            ));
        }

        Ok(ToolResult {
            llm_text: out.clone(),
            is_error: false,
            ui_payload: Some(serde_json::json!({ "path": path, "content": out })),
        })
    }
}
