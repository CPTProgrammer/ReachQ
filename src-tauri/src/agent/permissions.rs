//! Approval decisions (design 04): tool enablement, per-tool approval
//! config, sensitive-path escalation, and the hardcoded terminal security
//! rules (ported from Zed's tool_permissions.rs; not configurable).

use regex::Regex;

use super::config::ToolConfig;
use super::tools::read_file;

/// Outcome of the permission check for one tool call.
pub enum ApprovalDecision {
    /// Run without asking.
    Allow,
    /// Ask the user; `warnings` render as the yellow strip on the card.
    RequireApproval { warnings: Vec<String> },
    /// Hard deny (terminal security rules); the reason goes back to the model.
    Deny { reason: String },
}

// ---------------------------------------------------------------------------
// Sensitive file globs (design 03 §1 behavior 6, 04 §1)
// ---------------------------------------------------------------------------

/// Returns the matched pattern when `path` hits the sensitive list.
pub fn sensitive_match(path: &str, patterns: &[String]) -> Option<String> {
    let mut builder = globset::GlobSetBuilder::new();
    let mut valid: Vec<&String> = Vec::new();
    for p in patterns {
        match globset::Glob::new(p) {
            Ok(g) => {
                builder.add(g);
                valid.push(p);
            }
            Err(e) => {
                tracing::warn!("agent: ignoring invalid sensitive pattern {:?}: {}", p, e);
            }
        }
    }
    let set = builder.build().ok()?;
    // globset works on component-wise paths; strip the leading slash so
    // `**/.env` matches `/root/.env`.
    let normalized = path.trim_start_matches('/');
    set.matches(normalized)
        .first()
        .map(|i| valid[*i].clone())
}

// ---------------------------------------------------------------------------
// Terminal hardcoded security rules (Zed port; cannot be disabled)
// ---------------------------------------------------------------------------

const DENY_TEXT: &str =
    "Blocked by built-in security rule. This operation is considered too harmful.";

fn hardcoded_rules() -> &'static [Regex] {
    static RULES: std::sync::OnceLock<Vec<Regex>> = std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        // Flags between `rm` and the target path (zero or more).
        const F: &str = r"(?:--[a-zA-Z0-9][-a-zA-Z0-9_]*(?:=[^\s]*)?\s+|-[a-zA-Z]+\s+)*";
        // GNU-style trailing flags after the path.
        const T: &str = r"(?:\s+--[a-zA-Z0-9][-a-zA-Z0-9_]*(?:=[^\s]*)?|\s+-[a-zA-Z]+)*\s*$";
        [
            format!(r"\brm\s+{F}(?:--\s+)?/\*?{T}"),                 // /
            format!(r"\brm\s+{F}(?:--\s+)?~/?\*?{T}"),               // ~
            format!(r"\brm\s+{F}(?:--\s+)?(?:\$HOME|\$\{{HOME\}})/?(?:\*)?{T}"), // $HOME
            format!(r"\brm\s+{F}(?:--\s+)?\./?\*?{T}"),              // .
            format!(r"\brm\s+{F}(?:--\s+)?\.\./?\*?{T}"),            // ..
        ]
        .into_iter()
        .filter_map(|p| {
            regex::RegexBuilder::new(&p)
                .case_insensitive(true)
                .build()
                .ok()
        })
        .collect()
    })
}

/// Lexical path normalization (no disk access): drops `.`, pops `..`
/// (clamped at the root for absolute paths).
fn normalize_path(raw: &str) -> String {
    let absolute = raw.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for comp in raw.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                if parts.last() == Some(&"..") || parts.is_empty() {
                    if !absolute {
                        parts.push("..");
                    }
                } else {
                    parts.pop();
                }
            }
            c => parts.push(c),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{}", joined)
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// Split an `rm` command with multiple paths into one command per path, so
/// `rm -rf /tmp /` is checked as `rm -rf /tmp` and `rm -rf /`.
fn expand_rm_to_single_path_commands(cmd: &str) -> Vec<String> {
    let words: Vec<&str> = cmd.split_whitespace().collect();
    if words
        .first()
        .map(|w| !w.eq_ignore_ascii_case("rm"))
        .unwrap_or(true)
    {
        return vec![cmd.to_string()];
    }
    let mut flags: Vec<&str> = Vec::new();
    let mut paths: Vec<&str> = Vec::new();
    let mut seen_dashdash = false;
    for w in &words[1..] {
        if !seen_dashdash && (*w == "--" || w.starts_with('-')) {
            if *w == "--" {
                seen_dashdash = true;
            }
            flags.push(w);
        } else {
            paths.push(w);
        }
    }
    if paths.is_empty() {
        return vec![cmd.to_string()];
    }
    paths
        .iter()
        .map(|p| {
            let expanded;
            let path = if p.starts_with("$HOME") || p.starts_with("${HOME}") {
                expanded = p.to_string();
                expanded.as_str()
            } else {
                p
            };
            let normalized = if path.starts_with('$') || path.starts_with('~') {
                path.to_string()
            } else {
                normalize_path(path)
            };
            format!("rm {} {}", flags.join(" "), normalized)
        })
        .collect()
}

fn matches_hardcoded(cmd: &str) -> bool {
    let rules = hardcoded_rules();
    if rules.iter().any(|r| r.is_match(cmd)) {
        return true;
    }
    expand_rm_to_single_path_commands(cmd)
        .iter()
        .any(|c| rules.iter().any(|r| r.is_match(c)))
}

/// Naive chain splitting (`&&`, `||`, `;`, `|`) so `ls && rm -rf /` is
/// caught. Not a full shell parser (documented limitation vs Zed's
/// brush-parser approach).
pub fn check_terminal_security(command: &str) -> Option<String> {
    for fragment in command
        .split(|c| c == ';' || c == '|')
        .flat_map(|s| s.split("&&"))
    {
        if matches_hardcoded(fragment.trim()) {
            return Some(DENY_TEXT.to_string());
        }
    }
    None
}

/// Dangerous keywords that render a yellow warning strip on the terminal
/// approval card (design 04 §3).
pub fn terminal_warnings(command: &str) -> Vec<String> {
    const KEYWORDS: &[&str] = &[
        "sudo", "rm", "kill", "killall", "pkill", "mkfs", "dd", "shutdown", "reboot",
        "poweroff", "halt", "umount", "fdisk", "parted",
    ];
    let mut hits = Vec::new();
    for word in command.split(|c: char| c.is_whitespace() || c == ';' || c == '|' || c == '&') {
        let w = word.trim();
        if KEYWORDS.contains(&w) && !hits.iter().any(|h: &String| h == w) {
            hits.push(w.to_string());
        }
    }
    if hits.is_empty() {
        return vec![];
    }
    vec![format!(
        "Command contains potentially dangerous keyword{}: {}",
        if hits.len() > 1 { "s" } else { "" },
        hits.join(", ")
    )]
}

// ---------------------------------------------------------------------------
// Decision entry point (design 04 §2)
// ---------------------------------------------------------------------------

/// Decide whether a tool call may run. `config` is the resolved tool config
/// (defaults merged); `read_file_options` carries the sensitive glob list.
pub fn decide(
    tool_name: &str,
    args: &serde_json::Value,
    config: &ToolConfig,
    read_file_options: &serde_json::Map<String, serde_json::Value>,
) -> ApprovalDecision {
    // Hardcoded terminal rules first: unconditional.
    if tool_name == "terminal" {
        let command = args.get("command").and_then(|c| c.as_str()).unwrap_or("");
        if let Some(reason) = check_terminal_security(command) {
            return ApprovalDecision::Deny { reason };
        }
    }

    // Sensitive path escalation.
    let path = args.get("path").and_then(|p| p.as_str());
    if let Some(path) = path {
        if matches!(tool_name, "read_file" | "write_file" | "edit_file") {
            let patterns = read_file::sensitive_patterns(read_file_options);
            if let Some(pattern) = sensitive_match(path, &patterns) {
                let warning = format!("matches sensitive pattern `{}`", pattern);
                return ApprovalDecision::RequireApproval {
                    warnings: vec![warning],
                };
            }
        }
    }

    let warnings = if tool_name == "terminal" {
        terminal_warnings(args.get("command").and_then(|c| c.as_str()).unwrap_or(""))
    } else {
        vec![]
    };

    if config.require_approval {
        ApprovalDecision::RequireApproval { warnings }
    } else {
        ApprovalDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn denied(cmd: &str) -> bool {
        check_terminal_security(cmd).is_some()
    }

    #[test]
    fn blocks_rm_on_root() {
        assert!(denied("rm -rf /"));
        assert!(denied("rm -rf /tmp/../"));
        assert!(denied("rm -rf /tmp /"));
        assert!(denied("rm -rf ~"));
        assert!(denied("rm -rf ~/"));
        assert!(denied("rm -rf $HOME"));
        assert!(denied("rm -rf ${HOME}"));
        assert!(denied("rm -rf ."));
        assert!(denied("rm -rf .."));
        assert!(denied("sudo rm -rf /"));
        assert!(denied("ls -la && rm -rf /"));
        assert!(denied("rm / -rf"));
        assert!(denied("rm --no-preserve-root /"));
    }

    #[test]
    fn allows_safe_rm() {
        assert!(!denied("rm -rf ./build"));
        assert!(!denied("rm -rf /tmp/test"));
        assert!(!denied("rm -rf ~/Documents/old"));
        assert!(!denied("rm -rf ../some_dir"));
        assert!(!denied("rm -rf .hidden_dir"));
        assert!(!denied("rm file.txt"));
        assert!(!denied("systemctl restart nginx"));
    }

    #[test]
    fn sensitive_globs() {
        let patterns: Vec<String> = read_file::DEFAULT_SENSITIVE_PATTERNS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(sensitive_match("/root/.ssh/id_rsa", &patterns).is_some());
        assert!(sensitive_match("/home/u/.env", &patterns).is_some());
        assert!(sensitive_match("/etc/shadow", &patterns).is_some());
        assert!(sensitive_match("/srv/app/cert.pem", &patterns).is_some());
        assert!(sensitive_match("/etc/nginx/nginx.conf", &patterns).is_none());
    }
}
