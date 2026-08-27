//! SSH identity normalization (design 01 §1.1).
//!
//! Identity = "{username}@{host}:{port}[#via={chainhash8}]". The same link
//! (saved session or quick connect) always produces the same identity, so
//! threads and the agent panel are shared per link rather than per session.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::state::ProxyConfig;

/// One hop of a ProxyJump chain, reduced to the fields that identify the
/// link (credentials deliberately excluded from the fingerprint).
#[derive(Debug, Clone, Serialize)]
pub struct ChainHop {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_kind: String,
}

fn auth_kind(auth: &crate::ssh::client::AuthParams) -> &'static str {
    if auth.key.is_some() {
        "key"
    } else if auth.password.is_some() {
        "password"
    } else {
        "agent"
    }
}

impl From<&crate::ssh::client::JumpHostParams> for ChainHop {
    fn from(p: &crate::ssh::client::JumpHostParams) -> Self {
        Self {
            host: p.host.trim().to_lowercase(),
            port: p.port,
            username: p.username.trim().to_string(),
            auth_kind: auth_kind(&p.auth).to_string(),
        }
    }
}

impl From<&crate::state::JumpHostConfig> for ChainHop {
    fn from(p: &crate::state::JumpHostConfig) -> Self {
        let kind = match &p.auth_method {
            crate::state::AuthMethod::Key { .. } => "key",
            crate::state::AuthMethod::Password { .. } => "password",
            crate::state::AuthMethod::Agent => "agent",
        };
        Self {
            host: p.host.trim().to_lowercase(),
            port: p.port,
            username: p.username.trim().to_string(),
            auth_kind: kind.to_string(),
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
}
