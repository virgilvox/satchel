//! External MCP-server proxy.
//!
//! v1.5.0 lets users wire up MCP servers other than satchel's own —
//! e.g. a GitHub MCP, a filesystem MCP, anything that speaks the
//! protocol. The browser doesn't talk to those servers directly:
//!
//!   1. Auth headers (Bearer tokens, etc.) need to stay server-side —
//!      same trust model as the Anthropic API key. The browser only
//!      sees `{ id, name, url, has_auth }`.
//!   2. CORS would otherwise block most external MCP endpoints.
//!
//! So satchel proxies. The browser POSTs JSON-RPC to
//! `/api/mcp/proxy/<server-id>` and we forward to the configured URL
//! with the stored headers attached.
//!
//! satchel's *own* MCP at `/mcp` is implicit (always available, never
//! configured) — this module only manages the user-added entries.

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct McpServerEntry {
    /// Stable identifier used in the proxy URL path. Lowercase letters,
    /// digits, dashes, underscores only — we enforce this on save.
    pub id: String,
    /// Human-readable label shown in the UI.
    pub name: String,
    /// Target URL the proxy forwards to. Should be the JSON-RPC
    /// endpoint of an MCP server (typically `https://host/mcp`).
    pub url: String,
    /// Optional auth headers (anything: `Authorization`, `X-API-Key`, …).
    /// Stored on disk, never returned to the browser.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct McpServersConfig {
    #[serde(default, rename = "servers")]
    pub servers: Vec<McpServerEntry>,
}

impl McpServersConfig {
    fn path(vault_path: &Path) -> PathBuf {
        vault_path.join("mcp.toml")
    }

    pub fn load(vault_path: &Path) -> Result<Self> {
        let p = Self::path(vault_path);
        if !p.is_file() {
            return Ok(Self::default());
        }
        let body = std::fs::read_to_string(&p)
            .with_context(|| format!("failed to read {}", p.display()))?;
        let cfg: Self =
            toml::from_str(&body).with_context(|| format!("failed to parse {}", p.display()))?;
        Ok(cfg)
    }

    pub fn save(&self, vault_path: &Path) -> Result<()> {
        std::fs::create_dir_all(vault_path).with_context(|| {
            format!("failed to create vault directory {}", vault_path.display())
        })?;
        let p = Self::path(vault_path);
        let body = toml::to_string(self).context("failed to serialize mcp servers")?;
        std::fs::write(&p, body).with_context(|| format!("failed to write {}", p.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&p)?.permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(&p, perms)?;
        }
        Ok(())
    }

    pub fn upsert(&mut self, entry: McpServerEntry) -> Result<()> {
        validate_id(&entry.id)?;
        if entry.url.trim().is_empty() {
            bail!("mcp server url is empty");
        }
        if let Some(existing) = self.servers.iter_mut().find(|s| s.id == entry.id) {
            *existing = entry;
        } else {
            self.servers.push(entry);
        }
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.servers.len();
        self.servers.retain(|s| s.id != id);
        before != self.servers.len()
    }

    pub fn find(&self, id: &str) -> Option<&McpServerEntry> {
        self.servers.iter().find(|s| s.id == id)
    }
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        bail!("mcp server id is empty");
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!(
            "mcp server id '{id}' must be ASCII letters / digits / dash / underscore (it shows up in a URL path)"
        );
    }
    Ok(())
}

/// Forward a raw JSON-RPC body to the configured external MCP. Any auth
/// headers stored alongside the entry are attached server-side; the
/// caller's body passes through untouched.
pub async fn proxy_call(
    entry: &McpServerEntry,
    body: serde_json::Value,
    extra_headers: &reqwest::header::HeaderMap,
) -> Result<reqwest::Response> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .context("failed to build reqwest client")?;
    let mut req = client
        .post(&entry.url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    for (k, v) in &entry.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    // Forward MCP session headers from the browser (some servers stash
    // session state across requests).
    if let Some(sid) = extra_headers
        .get("mcp-session-id")
        .or_else(|| extra_headers.get("Mcp-Session-Id"))
    {
        if let Ok(s) = sid.to_str() {
            req = req.header("mcp-session-id", s);
        }
    }
    let res = req
        .json(&body)
        .send()
        .await
        .with_context(|| format!("failed to reach {}", entry.url))?;
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = McpServersConfig::load(dir.path()).unwrap();
        assert!(cfg.servers.is_empty());

        let mut cfg = McpServersConfig::default();
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".to_string(), "Bearer xxx".to_string());
        cfg.upsert(McpServerEntry {
            id: "github".to_string(),
            name: "GitHub MCP".to_string(),
            url: "https://example.com/mcp".to_string(),
            headers,
        })
        .unwrap();
        cfg.save(dir.path()).unwrap();

        let back = McpServersConfig::load(dir.path()).unwrap();
        assert_eq!(back.servers.len(), 1);
        assert_eq!(back.servers[0].id, "github");
        assert_eq!(
            back.servers[0].headers.get("Authorization").unwrap(),
            "Bearer xxx"
        );
    }

    #[test]
    fn upsert_replaces_by_id() {
        let mut cfg = McpServersConfig::default();
        cfg.upsert(McpServerEntry {
            id: "x".to_string(),
            name: "v1".to_string(),
            url: "https://a/".to_string(),
            headers: BTreeMap::new(),
        })
        .unwrap();
        cfg.upsert(McpServerEntry {
            id: "x".to_string(),
            name: "v2".to_string(),
            url: "https://b/".to_string(),
            headers: BTreeMap::new(),
        })
        .unwrap();
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.servers[0].name, "v2");
    }

    #[test]
    fn id_validation() {
        let mut cfg = McpServersConfig::default();
        assert!(cfg
            .upsert(McpServerEntry {
                id: "has space".to_string(),
                name: "x".to_string(),
                url: "https://a/".to_string(),
                headers: BTreeMap::new(),
            })
            .is_err());
        assert!(cfg
            .upsert(McpServerEntry {
                id: "ok-id_1".to_string(),
                name: "x".to_string(),
                url: "https://a/".to_string(),
                headers: BTreeMap::new(),
            })
            .is_ok());
    }

    #[test]
    fn remove_works() {
        let mut cfg = McpServersConfig::default();
        cfg.upsert(McpServerEntry {
            id: "x".to_string(),
            name: "x".to_string(),
            url: "https://a/".to_string(),
            headers: BTreeMap::new(),
        })
        .unwrap();
        assert!(cfg.remove("x"));
        assert!(!cfg.remove("x"));
    }
}
