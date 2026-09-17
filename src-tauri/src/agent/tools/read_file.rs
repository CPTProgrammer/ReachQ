//! read_file tool (design 03 §1): full-read transport, paged output,
//! output budget, per-line guard, read cache, read-path registration.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::agent::remote_fs::{self, Fingerprint, ReadCacheEntry};
use crate::agent::tools::{
    deserialize_maybe_stringified, err_text, AgentTool, ToolContext, ToolOptionKind,
    ToolOptionSpec,
};
use crate::agent::providers::ToolSchema;
use crate::agent::types::ToolResult;

const DESCRIPTION: &str = "\
Reads the content of a file on the remote host.
- `start_line`/`end_line` are 1-based and inclusive; omit both to get all lines. Use them to read specific sections of large files.
- Output is prefixed with line numbers (`{line:>6}\\t` + content). When quoting content (e.g. for edit_file), strip this prefix and keep the original text exactly.
- Binary files and files larger than 1 MB cannot be read with this tool; use the `terminal` tool instead (e.g. `tail`/`grep`) when guided by the error message.";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadFileInput {
    /// Absolute path of the file on the remote host.
    path: String,
    /// Optional line number to start reading on (1-based index).
    #[serde(default)]
    start_line: Option<u32>,
    /// Optional line number to end reading on (1-based index, inclusive).
    #[serde(default)]
    end_line: Option<u32>,
}

pub struct ReadFileTool;

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 16 * 1024;
pub const DEFAULT_MAX_OUTPUT_LINES: usize = 2000;
pub const DEFAULT_CACHE_TTL_SECONDS: u64 = 600;
/// Per-line guard against minified single lines exploding the context.
const MAX_LINE_CHARS: usize = 2000;

pub const DEFAULT_SENSITIVE_PATTERNS: &[&str] = &[
    "**/.ssh/id_*",
    "**/.ssh/*.pem",
    "**/.aws/credentials",
    "**/.aws/config",
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*.key",
    "**/.gnupg/**",
    "**/shadow",
];

fn opt_usize(ctx: &ToolContext, key: &str, default: usize) -> usize {
    ctx.options
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(default)
}

/// Sensitive glob patterns from the tool's options (shared by the
/// permissions layer for read upgrade and write/edit always-approve).
pub fn sensitive_patterns(options: &serde_json::Map<String, Value>) -> Vec<String> {
    options
        .get("sensitive_patterns")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| DEFAULT_SENSITIVE_PATTERNS.iter().map(|s| s.to_string()).collect())
}

#[async_trait]
impl AgentTool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn default_requires_approval(&self) -> bool {
        false
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: DESCRIPTION.to_string(),
            parameters: serde_json::to_value(schemars::schema_for!(ReadFileInput))
                .unwrap_or_default(),
        }
    }

    fn settings_schema(&self) -> Vec<ToolOptionSpec> {
        vec![
            ToolOptionSpec {
                key: "sensitive_patterns".into(),
                description: "gitignore-style glob list; matching files require approval to read and are always approved for writes".into(),
                kind: ToolOptionKind::StringList,
                default: serde_json::json!(DEFAULT_SENSITIVE_PATTERNS),
            },
            ToolOptionSpec {
                key: "cache_ttl_seconds".into(),
                description: "Read cache TTL in seconds (fingerprint still re-validated every read); 0 disables the cache".into(),
                kind: ToolOptionKind::Number { min: Some(0.0), max: None },
                default: serde_json::json!(DEFAULT_CACHE_TTL_SECONDS),
            },
            ToolOptionSpec {
                key: "max_output_bytes".into(),
                description: "Reject output larger than this many bytes; read in sections instead".into(),
                kind: ToolOptionKind::Number { min: Some(1024.0), max: None },
                default: serde_json::json!(DEFAULT_MAX_OUTPUT_BYTES),
            },
            ToolOptionSpec {
                key: "max_output_lines".into(),
                description: "Reject output longer than this many lines; read in sections instead".into(),
                kind: ToolOptionKind::Number { min: Some(1.0), max: None },
                default: serde_json::json!(DEFAULT_MAX_OUTPUT_LINES),
            },
        ]
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
        let input: ReadFileInput = deserialize_maybe_stringified(args)
            .map_err(|e| err_text(format!("Invalid arguments: {e}")))?;
        let path = input.path.trim().to_string();
        if path.is_empty() {
            return Err(err_text("Invalid arguments: path is empty"));
        }

        let fs = ctx.remote_fs(None).await?;

        // Stat first: not found / directory / size cap.
        let stat = fs.stat(&path).await.map_err(|e| remote_fs::map_stat_error(&path, &e))?;
        if stat.is_dir {
            return Err(err_text(format!(
                "{} is a directory, not a file. Use the list_directory tool to explore directory contents.",
                path
            )));
        }
        if stat.size > remote_fs::AGENT_READ_MAX {
            return Err(err_text(format!(
                "File is too large to read directly ({} bytes). Use the terminal tool to inspect it, e.g. `grep`, or read specific line ranges if you know them.",
                stat.size
            )));
        }
        let fingerprint = Fingerprint::from_stat(&stat);

        let max_bytes = opt_usize(ctx, "max_output_bytes", DEFAULT_MAX_OUTPUT_BYTES);
        let max_lines = opt_usize(ctx, "max_output_lines", DEFAULT_MAX_OUTPUT_LINES);
        let ttl_secs = opt_usize(ctx, "cache_ttl_seconds", DEFAULT_CACHE_TTL_SECONDS as usize) as u64;

        // Cache hit: fingerprint equal and TTL fresh (design 03 §1.7).
        let cached = {
            let cache = ctx.read_cache.lock().unwrap();
            cache.get(&(ctx.scope.clone(), path.clone())).and_then(|e| {
                if ttl_secs > 0
                    && e.fingerprint == fingerprint
                    && e.fetched_at.elapsed().as_secs() < ttl_secs
                {
                    Some((e.text.clone(), e.encoding_label.clone(), e.line_ending))
                } else {
                    None
                }
            })
        };

        let (text, encoding_label, line_ending) = match cached {
            Some(hit) => hit,
            None => {
                let decoded = remote_fs::read_pipeline(&fs, &path).await?;
                let label = decoded.encoding.name().to_string();
                let entry = ReadCacheEntry {
                    text: decoded.text.clone(),
                    encoding_label: label.clone(),
                    line_ending: decoded.line_ending,
                    fingerprint: decoded.fingerprint,
                    fetched_at: std::time::Instant::now(),
                };
                ctx.read_cache
                    .lock()
                    .unwrap()
                    .insert((ctx.scope.clone(), path.clone()), entry);
                (decoded.text, label, decoded.line_ending)
            }
        };

        // Range normalization (Zed defensive: 0 clamps to 1, end < start
        // still reads one line).
        let lines: Vec<&str> = text.split('\n').collect();
        let total_lines = lines.len();
        let total_bytes = text.len();
        let start = input.start_line.unwrap_or(1).max(1);
        let end = input.end_line.unwrap_or(u32::MAX).max(start);
        let ranged = input.start_line.is_some() || input.end_line.is_some();

        let sel_start = (start - 1) as usize;
        let sel_end = (end as usize).min(total_lines);
        let selected: Vec<&str> = if sel_start >= total_lines {
            Vec::new()
        } else {
            lines[sel_start..sel_end.max(sel_start)].to_vec()
        };
        let sel_bytes: usize = selected.iter().map(|l| l.len() + 1).sum();

        // Output budget: reject, never silently truncate (design 03 §1.0).
        if !ranged {
            let over_lines = total_lines > max_lines;
            let over_bytes = total_bytes > max_bytes;
            if over_lines || over_bytes {
                let (kind, value) = if over_bytes { ("byte", max_bytes) } else { ("line", max_lines) };
                return Err(err_text(format!(
                    "File has {} lines and {} bytes, exceeding the {} limit ({}). Read it in sections with start_line/end_line.",
                    total_lines, total_bytes, kind, value
                )));
            }
        } else if selected.len() > max_lines || sel_bytes > max_bytes {
            let (kind, value) = if sel_bytes > max_bytes {
                ("byte", max_bytes)
            } else {
                ("line", max_lines)
            };
            return Err(err_text(format!(
                "The requested range has {} lines and {} bytes, exceeding the {} limit ({}). Read a smaller range with start_line/end_line.",
                selected.len(), sel_bytes, kind, value
            )));
        }

        // Format with line numbers + per-line guard.
        let mut out = String::new();
        for (i, line) in selected.iter().enumerate() {
            let lineno = sel_start + i + 1;
            out.push_str(&format!("{:>6}\t", lineno));
            if line.chars().count() > MAX_LINE_CHARS {
                let truncated: String = line.chars().take(MAX_LINE_CHARS).collect();
                out.push_str(&truncated);
                out.push_str(&format!(" [... line truncated, {} bytes total]", line.len()));
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }

        let payload = serde_json::json!({
            "path": path,
            "content": out,
            "encoding": encoding_label,
            "lineEnding": match line_ending { remote_fs::LineEnding::Lf => "lf", remote_fs::LineEnding::CrLf => "crlf" },
        });
        Ok(ToolResult {
            llm_text: out,
            is_error: false,
            ui_payload: Some(payload),
        })
    }
}
