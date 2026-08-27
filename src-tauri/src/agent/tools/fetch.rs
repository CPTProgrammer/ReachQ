//! `fetch` tool — port of Zed's fetch_tool.rs (design 03 §6). Runs on the
//! local machine: reqwest GET, 10 MiB download cap, Content-Type dispatch,
//! char-based content windowing.

use std::cell::RefCell;
use std::rc::Rc;

use async_trait::async_trait;
use futures::StreamExt;
use html_to_markdown::markdown;
use html_to_markdown::{TagHandler, convert_html_to_markdown};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::super::providers::ToolSchema;
use super::{AgentTool, ToolContext, err_text, ok_text};
use crate::agent::types::ToolResult;

const DESCRIPTION: &str = "Fetches a URL and returns the content as Markdown. The fetch runs on the user's LOCAL machine, not the remote host.
- Large responses are windowed rather than returned in full: by default only the first 16384 characters of the converted content are returned, along with a note reporting the total length. To read further, call this tool again with the same URL and a `start_char` offset.
- Each call re-downloads the content, so prefer a few large windows over many small ones.";

const DEFAULT_MAX_CHARS: u32 = 16384;
const MAX_CHARS_LIMIT: u32 = 65536;
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024; // 10 MiB
const LINE_SNAP_BACKOFF_CHARS: u32 = 200;

#[derive(Debug, Deserialize, JsonSchema)]
struct FetchToolInput {
    /// The URL to fetch.
    url: String,
    /// Optional. 0-based character offset into the converted content to start
    /// reading from. When omitted, reading starts from the beginning.
    #[serde(default)]
    start_char: Option<u32>,
    /// Optional. Maximum number of characters to return, counted over the
    /// converted content. Defaults to 16384 and is capped at 65536.
    #[serde(default)]
    max_chars: Option<u32>,
}

pub struct FetchTool;

#[async_trait]
impl AgentTool for FetchTool {
    fn name(&self) -> &'static str {
        "fetch"
    }

    fn default_requires_approval(&self) -> bool {
        true
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: DESCRIPTION.to_string(),
            parameters: serde_json::to_value(schemars::schema_for!(FetchToolInput))
                .unwrap_or(Value::Null),
        }
    }

    fn title(&self, args: &Value) -> String {
        args.get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    async fn run(
        &self,
        args: Value,
        _ctx: &ToolContext,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolResult> {
        let input: FetchToolInput =
            serde_json::from_value(args).map_err(|e| err_text(format!("invalid arguments: {e}")))?;

        if cancel.is_cancelled() {
            return Err(err_text("Fetch cancelled by user"));
        }

        let mut url = input.url.trim().to_string();
        if !url.starts_with("https://") && !url.starts_with("http://") {
            url = format!("https://{url}");
        }

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| err_text(format!("failed to fetch {url}: {e}")))?;

        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        // Stream the body with a 10 MiB hard cap (read past the limit by one
        // byte to detect overflow), then trim to a UTF-8 boundary.
        let mut body: Vec<u8> = Vec::new();
        let mut download_truncated = false;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            if cancel.is_cancelled() {
                return Err(err_text("Fetch cancelled by user"));
            }
            let chunk = chunk
                .map_err(|e| err_text(format!("failed to read response body: {e}")))?;
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                let remaining = MAX_RESPONSE_BYTES - body.len();
                body.extend_from_slice(&chunk[..remaining]);
                download_truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }

        let text = match String::from_utf8(body) {
            Ok(text) => text,
            Err(e) if download_truncated => {
                let valid_up_to = e.utf8_error().valid_up_to();
                let bytes = e.into_bytes();
                // The tail holds an incomplete UTF-8 sequence; drop it.
                String::from_utf8_lossy(&bytes[..valid_up_to]).into_owned()
            }
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        };

        // Content-Type dispatch.
        let Some(content_type) = content_type else {
            return Err(err_text("missing Content-Type header"));
        };
        let text = if content_type.starts_with("text/plain") {
            text
        } else if content_type.starts_with("application/json") {
            match serde_json::from_str::<Value>(&text) {
                Ok(json) => match serde_json::to_string_pretty(&json) {
                    Ok(pretty) => format!("```json\n{pretty}\n```"),
                    Err(e) => return Err(err_text(format!("failed to format JSON: {e}"))),
                },
                Err(_) if download_truncated => {
                    return Err(err_text(
                        "failed to parse JSON: the response exceeded the 10MB download limit and is incomplete",
                    ));
                }
                Err(e) => {
                    return Err(err_text(format!("failed to parse JSON response body: {e}")));
                }
            }
        } else {
            match markdown_from_html(&text) {
                Ok(markdown) => markdown,
                Err(e) => return Err(err_text(e)),
            }
        };

        if status.is_client_error() {
            if text.trim().is_empty() {
                return Err(err_text(format!("status error {status}")));
            }
            let windowed =
                apply_char_window(&text, input.start_char, input.max_chars, download_truncated);
            return Err(err_text(format!(
                "status error {status}, response:\n\n{windowed}"
            )));
        }

        if text.trim().is_empty() {
            return Err(err_text("no textual content found"));
        }

        Ok(ok_text(apply_char_window(
            &text,
            input.start_char,
            input.max_chars,
            download_truncated,
        )))
    }
}

fn markdown_from_html(html: &str) -> Result<String, String> {
    let mut handlers: Vec<TagHandler> = vec![
        Rc::new(RefCell::new(markdown::WebpageChromeRemover)),
        Rc::new(RefCell::new(markdown::ParagraphHandler)),
        Rc::new(RefCell::new(markdown::HeadingHandler)),
        Rc::new(RefCell::new(markdown::ListHandler)),
        Rc::new(RefCell::new(markdown::TableHandler::new())),
        Rc::new(RefCell::new(markdown::StyledTextHandler)),
        Rc::new(RefCell::new(markdown::CodeHandler)),
    ];
    convert_html_to_markdown(html.as_bytes(), &mut handlers)
        .map_err(|e| format!("failed to convert HTML to Markdown: {e}"))
}

/// Window `text` by characters (not bytes) per Zed's fetch tool: snap back to
/// a line end within the window when the dropped tail is small enough, and
/// append a continuation note when the window is not the full content.
fn apply_char_window(
    text: &str,
    start_char: Option<u32>,
    max_chars: Option<u32>,
    download_truncated: bool,
) -> String {
    let max_chars = max_chars.unwrap_or(DEFAULT_MAX_CHARS).clamp(1, MAX_CHARS_LIMIT) as usize;
    let total = text.chars().count();
    let start = (start_char.unwrap_or(0) as usize).min(total);

    if start >= total {
        return format!(
            "[start_char {start} is at or beyond the end of the content ({total} chars). Nothing to return.]"
        );
    }

    let byte_start = text
        .char_indices()
        .nth(start)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let remaining = &text[byte_start..];
    let remaining_chars = total - start;

    let mut take_chars = remaining_chars.min(max_chars);
    if take_chars < remaining_chars {
        // Window doesn't reach the end: snap back to the last line break if
        // the discarded tail is at most LINE_SNAP_BACKOFF_CHARS chars.
        let byte_end = remaining
            .char_indices()
            .nth(take_chars)
            .map(|(i, _)| i)
            .unwrap_or(remaining.len());
        let window = &remaining[..byte_end];
        if let Some(newline_pos) = window.rfind('\n') {
            let dropped = window[newline_pos + 1..].chars().count();
            if dropped <= LINE_SNAP_BACKOFF_CHARS as usize {
                take_chars = window[..newline_pos].chars().count();
            }
        }
    }

    let byte_end = remaining
        .char_indices()
        .nth(take_chars)
        .map(|(i, _)| i)
        .unwrap_or(remaining.len());
    let windowed = &remaining[..byte_end];
    let end = start + take_chars;

    let mut result = String::new();
    if download_truncated {
        result.push_str(
            "[Note: the response body exceeded the 10MB download limit; only its beginning is reflected below.]\n\n",
        );
    }
    result.push_str(windowed);

    if start > 0 || end < total {
        if end < total {
            result.push_str(&format!(
                "\n\n[Showing chars {}-{end} of {total}. Use start_char: {end} to continue reading.]",
                start + 1
            ));
        } else {
            result.push_str(&format!(
                "\n\n[Showing chars {}-{end} of {total}.]",
                start + 1
            ));
        }
    }

    result
}
