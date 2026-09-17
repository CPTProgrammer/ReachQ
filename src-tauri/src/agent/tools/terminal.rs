//! terminal tool (design 03 §5): SSH exec channel with a PTY, dual output
//! projection (raw bytes streamed to the frontend xterm card; headless
//! alacritty grid extraction for the model).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Point};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;
use async_trait::async_trait;
use base64::Engine;
use russh::ChannelMsg;
use serde::Deserialize;
use serde_json::Value;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::super::events::AgentEvent;
use super::super::remote_fs::{self, ConnectionLease};
use super::super::types::ToolResult;
use super::{err_text, AgentTool, ToolContext};
use crate::agent::providers::ToolSchema;

/// Default PTY size (Zed's headless terminal defaults: 100 cols x 6 rows).
const DEFAULT_COLS: u32 = 100;
const DEFAULT_ROWS: u32 = 6;
const DEFAULT_TIMEOUT_MS: u64 = 300_000;
const OUTPUT_LIMIT: usize = 16 * 1024;
/// Scrollback lines kept in the headless grid (Zed task terminal).
const SCROLL_HISTORY: usize = 100_000;

const DESCRIPTION: &str = "\
Executes a shell one-liner on the REMOTE host and returns its output with the exit code.
- Each invocation spawns a new shell: no state (cwd, env, shell vars) carries over.
- Commands that open a pager, editor, or any interactive prompt will hang until timeout. Use non-interactive forms instead: `git --no-pager ...`, `apt-get -y ...`. Interactive password prompts cannot be answered. For sudo, only passwordless (NOPASSWD) accounts work. Use `sudo -n ...`.
- Do NOT pipe output to `head`/`tail`/`grep -m`; use the `head_lines`/`tail_lines` parameters instead. When both are given, the first N lines are returned, then a blank line, then the last N lines.
- Do not use this for commands that run indefinitely (servers, watchers, `top`).
- Prefer `timeout_ms` for potentially slow commands; the command is killed on timeout.
- Prefer read_file/list_directory over `cat`/`ls` when inspecting files.";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TerminalInput {
    /// The one-liner command to execute on the remote host.
    command: String,
    /// Optional maximum runtime in milliseconds (default 300000, no upper
    /// limit). The command is killed on timeout.
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Optional. Return only the first N lines of the output.
    #[serde(default)]
    head_lines: Option<usize>,
    /// Optional. Return only the last N lines of the output.
    #[serde(default)]
    tail_lines: Option<usize>,
}

/// Handle for the UI to drive a running terminal session (resize + stop).
pub struct TerminalHandle {
    pub resize_tx: mpsc::UnboundedSender<(u32, u32)>,
    pub stop_tx: Mutex<Option<oneshot::Sender<()>>>,
}

/// tool_call_id -> live terminal session.
pub type SharedTerminals = Arc<Mutex<HashMap<String, TerminalHandle>>>;

pub fn new_shared_terminals() -> SharedTerminals {
    Arc::new(Mutex::new(HashMap::new()))
}

pub struct TerminalTool;

/// Fixed-size Dimensions for the headless grid.
#[derive(Clone, Copy)]
struct TermSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Wrap the model's command per design 03 §5:
/// `env 'PAGER=' 'GIT_PAGER=cat' '/bin/sh' '-i' '-c' '(<cmd>\n) </dev/null'`
pub fn wrap_command(command: &str) -> Result<String, ToolResult> {
    let inner = format!("({}\n) </dev/null", command);
    shlex::try_join([
        "env",
        "PAGER=",
        "GIT_PAGER=cat",
        "/bin/sh",
        "-i",
        "-c",
        inner.as_str(),
    ])
    .map_err(|e| err_text(format!("Could not shell-quote the command: {e}")))
}

/// Extract the grid's full text (scrollback + screen), Zed's
/// `content_text` equivalent.
fn extract_term_text(term: &Term<VoidListener>) -> String {
    let start = Point::new(term.topmost_line(), Column(0));
    let end = Point::new(term.bottommost_line(), term.last_column());
    term.bounds_to_string(start, end)
}

/// head_lines/tail_lines windowing (Zed semantics; bypasses the byte cap).
fn select_lines(output: &str, head: Option<usize>, tail: Option<usize>) -> String {
    let lines: Vec<&str> = output.lines().collect();
    match (head, tail) {
        (Some(h), Some(t)) => {
            let head_part = lines.iter().take(h).copied().collect::<Vec<_>>().join("\n");
            let tail_part = lines[lines.len().saturating_sub(t)..].join("\n");
            format!("{}\n\n{}", head_part, tail_part)
        }
        (Some(h), None) => lines.iter().take(h).copied().collect::<Vec<_>>().join("\n"),
        (None, Some(t)) => lines[lines.len().saturating_sub(t)..].join("\n"),
        (None, None) => output.to_string(),
    }
}

/// Byte cap with char-boundary alignment and line backoff (Zed semantics).
fn truncate_to_limit(content: &mut String, limit: usize) -> bool {
    let original_len = content.len();
    if content.len() > limit {
        let mut end = limit.min(content.len());
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        end = content[..end].rfind('\n').unwrap_or(end);
        content.truncate(end);
    }
    content.len() < original_len
}

struct CommandOutcome {
    exit_code: Option<i32>,
    timed_out: bool,
    user_stopped: bool,
}

fn format_result(command: &str, output: &str, truncated: bool, outcome: &CommandOutcome) -> String {
    let trimmed = output.trim();
    let content = if trimmed.is_empty() {
        String::new()
    } else {
        format!("```\n{}\n```", trimmed)
    };
    let content = if truncated && !content.is_empty() {
        format!("Command output too long. The first {} bytes:\n\n{}", trimmed.len(), content)
    } else {
        content
    };

    if outcome.user_stopped {
        if content.is_empty() {
            return "The user stopped this command. No output was captured before stopping.\n\nSince the user intentionally interrupted this command, ask them what they would like to do next rather than automatically retrying or assuming something went wrong.".to_string();
        }
        return format!(
            "The user stopped this command. Output captured before stopping:\n\n{}\n\nSince the user intentionally interrupted this command, ask them what they would like to do next rather than automatically retrying or assuming something went wrong.",
            content
        );
    }
    if outcome.timed_out {
        if content.is_empty() {
            return format!("Command \"{}\" timed out. No output was captured.", command);
        }
        return format!(
            "Command \"{}\" timed out. Output captured before timeout:\n\n{}",
            command, content
        );
    }
    match outcome.exit_code {
        Some(0) => {
            if content.is_empty() {
                "Command executed successfully.".to_string()
            } else {
                content
            }
        }
        Some(code) => {
            if content.is_empty() {
                format!("Command \"{}\" failed with exit code {}.", command, code)
            } else {
                format!(
                    "Command \"{}\" failed with exit code {}.\n\n{}",
                    command, code, content
                )
            }
        }
        None => {
            if content.is_empty() {
                "Command terminated unexpectedly. No output was captured.".to_string()
            } else {
                format!("Command terminated unexpectedly. Output captured:\n\n{}", content)
            }
        }
    }
}

#[async_trait]
impl AgentTool for TerminalTool {
    fn name(&self) -> &'static str {
        "terminal"
    }

    fn default_requires_approval(&self) -> bool {
        true
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: DESCRIPTION.to_string(),
            parameters: serde_json::to_value(schemars::schema_for!(TerminalInput))
                .unwrap_or_default(),
        }
    }

    fn title(&self, args: &Value) -> String {
        args.get("command")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string()
    }

    async fn run(
        &self,
        args: Value,
        ctx: &ToolContext,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolResult> {
        let input: TerminalInput = serde_json::from_value(args)
            .map_err(|e| err_text(format!("Invalid arguments: {e}")))?;
        let command = input.command.trim().to_string();
        if command.is_empty() {
            return Err(err_text("Invalid arguments: command is empty"));
        }
        let timeout_ms = input.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);

        // Leased connection (design 02 §3.1: covers the whole tool call).
        let lease = ConnectionLease::acquire(
            &ctx.ssh_manager,
            &ctx.sftp_backends,
            ctx.app.clone(),
            &ctx.scope,
            None,
        )
        .await
        .map_err(|e| remote_fs::connection_lost_result(&ctx.scope, &e))?;

        let wrapped = wrap_command(&command)?;

        let mut channel = {
            let guard = lease.handle.lock().await;
            guard.channel_open_session().await.map_err(|e| {
                err_text(format!("Failed to open a channel on the SSH connection: {e}"))
            })?
        };
        channel
            .request_pty(true, "xterm-256color", DEFAULT_COLS, DEFAULT_ROWS, 0, 0, &[])
            .await
            .map_err(|e| err_text(format!("PTY request failed: {e}")))?;
        channel
            .exec(true, wrapped.as_str())
            .await
            .map_err(|e| err_text(format!("Failed to execute the command: {e}")))?;

        // Headless grid for the model-facing text projection.
        let mut term = Term::new(
            Config {
                scrolling_history: SCROLL_HISTORY,
                ..Config::default()
            },
            &TermSize {
                cols: DEFAULT_COLS as usize,
                rows: DEFAULT_ROWS as usize,
            },
            VoidListener,
        );
        let mut processor: Processor = Processor::new();

        // Registry entry for UI resize/stop.
        let (resize_tx, mut resize_rx) = mpsc::unbounded_channel::<(u32, u32)>();
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        ctx.terminals.lock().unwrap().insert(
            ctx.tool_call_id.clone(),
            TerminalHandle {
                resize_tx,
                stop_tx: Mutex::new(Some(stop_tx)),
            },
        );
        let registry_cleanup = |ctx: &ToolContext| {
            ctx.terminals.lock().unwrap().remove(&ctx.tool_call_id);
        };

        let event_channel = AgentEvent::channel(&ctx.scope);
        let mut exit_code: Option<i32> = None;
        let mut got_eof = false;
        let mut got_exit = false;
        let mut timed_out = false;
        let mut user_stopped = false;
        let timeout = tokio::time::sleep(std::time::Duration::from_millis(timeout_ms));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                msg = channel.wait() => {
                    match msg {
                        Some(ChannelMsg::Data { data }) => {
                            processor.advance(&mut term, &data);
                            let _ = ctx.app.emit(&event_channel, AgentEvent::TerminalOutput {
                                tool_call_id: ctx.tool_call_id.clone(),
                                data_b64: base64::engine::general_purpose::STANDARD.encode(&data),
                            });
                        }
                        Some(ChannelMsg::ExtendedData { data, .. }) => {
                            processor.advance(&mut term, &data);
                        }
                        Some(ChannelMsg::ExitStatus { exit_status }) => {
                            exit_code = Some(exit_status as i32);
                            got_exit = true;
                            if got_eof { break; }
                        }
                        Some(ChannelMsg::Eof) => {
                            got_eof = true;
                            if got_exit { break; }
                        }
                        None => break,
                        _ => {}
                    }
                }
                Some((cols, rows)) = resize_rx.recv() => {
                    let _ = channel.window_change(cols, rows, 0, 0).await;
                    term.resize(TermSize { cols: cols as usize, rows: rows as usize });
                }
                _ = &mut stop_rx => {
                    user_stopped = true;
                    let _ = channel.close().await;
                    break;
                }
                _ = &mut timeout => {
                    timed_out = true;
                    let _ = channel.close().await;
                    break;
                }
                _ = cancel.cancelled() => {
                    user_stopped = true;
                    let _ = channel.close().await;
                    break;
                }
            }
        }

        registry_cleanup(ctx);

        let raw = extract_term_text(&term);
        let trimmed = raw.trim().to_string();

        // head/tail windowing bypasses the byte cap (design 03 §5).
        let (selected, truncated) = if input.head_lines.is_some() || input.tail_lines.is_some() {
            (select_lines(&trimmed, input.head_lines, input.tail_lines), false)
        } else {
            let mut s = trimmed;
            let t = truncate_to_limit(&mut s, OUTPUT_LIMIT);
            (s, t)
        };

        let outcome = CommandOutcome {
            exit_code,
            timed_out,
            user_stopped,
        };
        let llm_text = format_result(&command, &selected, truncated, &outcome);
        let is_error = timed_out || user_stopped || !matches!(exit_code, Some(0));

        Ok(ToolResult {
            llm_text,
            is_error,
            ui_payload: Some(serde_json::json!({
                "command": command,
                "output": selected,
                "exitCode": exit_code,
                "timedOut": timed_out,
                "userStopped": user_stopped,
                "truncated": truncated,
            })),
        })
    }
}
