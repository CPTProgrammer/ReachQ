//! SSH identity normalization, used to name quick-connect link scopes
//! (`link:<identity>`). Saved-session scopes (`session:<uuid>`) do not use
//! this module. The same link always produces the same identity, so link
//! scopes stay stable across reconnects and credential choices.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::state::ProxyConfig;

/// Owning scope for a connection: `session:<id>` when started from a saved
/// session, else `link:<identity>`.
pub fn agent_scope(session_id: Option<&str>, identity: String) -> String {
    match session_id {
        Some(id) => format!("session:{id}"),
        None => format!("link:{identity}"),
    }
}

/// One hop of a ProxyJump chain, reduced to the fields that identify the
/// link (credentials deliberately excluded from the fingerprint).
#[derive(Debug, Clone, Serialize)]
pub struct ChainHop {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_kind: AuthKind,
}

/// Auth category labels for the chain fingerprint. The serialized form is a
/// stability contract: renaming variants changes link identities, orphaning
/// `link:`-scoped thread history. All conversions funnel through this enum
/// so the label set has a single source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Key,
    Password,
    Agent,
}

/// Runtime connect params can hold several methods at once (the auth cascade
/// tries key → agent → password); classify by that precedence.
impl From<&crate::ssh::client::AuthParams> for AuthKind {
    fn from(auth: &crate::ssh::client::AuthParams) -> Self {
        if auth.key.is_some() {
            Self::Key
        } else if auth.password.is_some() {
            Self::Password
        } else {
            Self::Agent
        }
    }
}

impl From<&crate::state::AuthMethod> for AuthKind {
    fn from(m: &crate::state::AuthMethod) -> Self {
        match m {
            crate::state::AuthMethod::Key { .. } => Self::Key,
            crate::state::AuthMethod::Password { .. } => Self::Password,
            crate::state::AuthMethod::Agent => Self::Agent,
        }
    }
}

impl From<&crate::ssh::client::JumpHostParams> for ChainHop {
    fn from(p: &crate::ssh::client::JumpHostParams) -> Self {
        Self {
            host: p.host.trim().to_lowercase(),
            port: p.port,
            username: p.username.trim().to_string(),
            auth_kind: AuthKind::from(&p.auth),
        }
    }
}

impl From<&crate::state::JumpHostConfig> for ChainHop {
    fn from(p: &crate::state::JumpHostConfig) -> Self {
        Self {
            host: p.host.trim().to_lowercase(),
            port: p.port,
            username: p.username.trim().to_string(),
            auth_kind: AuthKind::from(&p.auth_method),
        }
    }
}

/// Proxy config reduced to identity-relevant fields (no password).
#[derive(Debug, Clone, Serialize)]
pub struct ChainProxy {
    pub proxy_type: String,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
}

impl From<&ProxyConfig> for ChainProxy {
    fn from(p: &ProxyConfig) -> Self {
        Self {
            proxy_type: p.proxy_type.clone(),
            host: p.host.trim().to_lowercase(),
            port: p.port,
            username: p.username.clone(),
        }
    }
}

/// Canonical serialization of the connection chain for hashing. Field
/// order is declaration order (serde struct), which is stable.
#[derive(Serialize)]
struct ChainFingerprint<'a> {
    jump_chain: Option<&'a [ChainHop]>,
    proxy: Option<ChainProxy>,
}

fn normalize_host(host: &str) -> String {
    let h = host.trim().to_lowercase();
    // IPv6 literals get bracketed so "addr:port" stays unambiguous.
    if h.contains(':') && !h.starts_with('[') {
        format!("[{}]", h.trim_start_matches('[').trim_end_matches(']'))
    } else {
        h
    }
}

fn chain_hash(jump_chain: Option<&[ChainHop]>, proxy: Option<&ProxyConfig>) -> Option<String> {
    let has_chain = jump_chain.map(|c| !c.is_empty()).unwrap_or(false);
    if !has_chain && proxy.is_none() {
        return None;
    }
    let fp = ChainFingerprint {
        jump_chain,
        proxy: proxy.map(ChainProxy::from),
    };
    let json = serde_json::to_string(&fp).ok()?;
    let digest = Sha256::digest(json.as_bytes());
    Some(hex_encode(&digest)[..8].to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Compute the normalized identity for a connection.
pub fn compute_identity(
    username: &str,
    host: &str,
    port: u16,
    jump_chain: Option<&[ChainHop]>,
    proxy: Option<&ProxyConfig>,
) -> String {
    let host = normalize_host(host);
    let base = format!("{}@{}:{}", username.trim(), host, port);
    match chain_hash(jump_chain, proxy) {
        Some(hash) => format!("{}#via={}", base, hash),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_connection() {
        let id = compute_identity("root", "Example.COM ", 22, None, None);
        assert_eq!(id, "root@example.com:22");
    }

    #[test]
    fn ipv6_bracketed() {
        let id = compute_identity("root", "::1", 22, None, None);
        assert_eq!(id, "root@[::1]:22");
    }

    #[test]
    fn via_suffix_when_proxied() {
        let proxy = ProxyConfig {
            proxy_type: "socks5".into(),
            host: "127.0.0.1".into(),
            port: 9050,
            username: None,
            password: None,
        };
        let id = compute_identity("root", "10.0.0.8", 22, None, Some(&proxy));
        assert!(id.starts_with("root@10.0.0.8:22#via="));
        assert_eq!(id.len(), "root@10.0.0.8:22#via=".len() + 8);
    }

    #[test]
    fn via_suffix_stable_for_same_chain() {
        let proxy = ProxyConfig {
            proxy_type: "socks5".into(),
            host: "127.0.0.1".into(),
            port: 9050,
            username: None,
            password: Some("secret".into()),
        };
        // Password must not affect the fingerprint.
        let a = compute_identity("root", "10.0.0.8", 22, None, Some(&proxy));
        let proxy2 = ProxyConfig { password: None, ..proxy };
        let b = compute_identity("root", "10.0.0.8", 22, None, Some(&proxy2));
        assert_eq!(a, b);
    }

    #[test]
    fn auth_kind_labels_are_stable() {
        // The labels feed the chain hash: renaming a variant changes link
        // identities and orphans `link:`-scoped thread history.
        let hop = |kind: AuthKind| ChainHop {
            host: "h".into(),
            port: 22,
            username: "u".into(),
            auth_kind: kind,
        };
        let json = serde_json::to_string(&hop(AuthKind::Key)).unwrap();
        assert!(json.contains(r#""auth_kind":"key""#));
        let json = serde_json::to_string(&hop(AuthKind::Password)).unwrap();
        assert!(json.contains(r#""auth_kind":"password""#));
        let json = serde_json::to_string(&hop(AuthKind::Agent)).unwrap();
        assert!(json.contains(r#""auth_kind":"agent""#));
    }
}
