//! Anthropic Messages API proxy.
//!
//! The browser side of satchel can't talk to api.anthropic.com directly:
//! Anthropic doesn't permit cross-origin calls from arbitrary web pages,
//! and putting the user's API key in the browser would leak it through
//! any extension or devtools peek. So satchel runs a thin server-side
//! proxy that holds the key and streams responses through to the chat UI
//! over SSE.
//!
//! This module owns:
//!   * `AnthropicConfig` — persisted at `<vault>/anthropic.toml`, chmod 0600
//!     on Unix. The key never leaves the server side after save.
//!   * `proxy_messages` — opens a streaming POST against
//!     `https://api.anthropic.com/v1/messages` and pipes the raw SSE bytes
//!     back to the caller. We don't try to re-frame the Anthropic SSE
//!     protocol — the client already understands it.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1/messages";
/// API version pinned per Anthropic docs. Bump deliberately when their
/// schema evolves; the messages API surface is stable but the version
/// header is required.
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AnthropicConfig {
    pub api_key: String,
}

impl AnthropicConfig {
    fn path(vault_path: &Path) -> PathBuf {
        vault_path.join("anthropic.toml")
    }

    pub fn load(vault_path: &Path) -> Result<Option<Self>> {
        let p = Self::path(vault_path);
        if !p.is_file() {
            return Ok(None);
        }
        let body = std::fs::read_to_string(&p)
            .with_context(|| format!("failed to read {}", p.display()))?;
        let cfg: AnthropicConfig =
            toml::from_str(&body).with_context(|| format!("failed to parse {}", p.display()))?;
        if cfg.api_key.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(cfg))
    }

    pub fn save(&self, vault_path: &Path) -> Result<()> {
        std::fs::create_dir_all(vault_path).with_context(|| {
            format!("failed to create vault directory {}", vault_path.display())
        })?;
        let p = Self::path(vault_path);
        let body = toml::to_string(self).context("failed to serialize anthropic config")?;
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

    pub fn clear(vault_path: &Path) -> Result<()> {
        let p = Self::path(vault_path);
        if p.is_file() {
            std::fs::remove_file(&p)
                .with_context(|| format!("failed to remove {}", p.display()))?;
        }
        Ok(())
    }
}

/// Stream a Messages API call. The caller passes the raw JSON body the
/// browser built (we don't re-validate it — Anthropic's error response
/// is more informative than anything we'd write here) and gets back
/// reqwest's response. We add the auth + version headers and force
/// `stream: true` if the caller forgot.
pub async fn proxy_messages(api_key: &str, body: serde_json::Value) -> Result<reqwest::Response> {
    if api_key.trim().is_empty() {
        bail!("anthropic api key not configured — POST to /api/anthropic/config first");
    }

    // Guarantee streaming. Anthropic accepts non-stream too, but we
    // always want SSE — that's the path the chat UI knows.
    let mut body = body;
    if let Some(map) = body.as_object_mut() {
        map.entry("stream").or_insert(serde_json::Value::Bool(true));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .context("failed to build reqwest client")?;
    let res = client
        .post(ANTHROPIC_BASE_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_API_VERSION)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .context("failed to reach api.anthropic.com")?;
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        assert!(AnthropicConfig::load(dir.path()).unwrap().is_none());
        let cfg = AnthropicConfig {
            api_key: "sk-ant-test".to_string(),
        };
        cfg.save(dir.path()).unwrap();
        let back = AnthropicConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(back.api_key, "sk-ant-test");
        AnthropicConfig::clear(dir.path()).unwrap();
        assert!(AnthropicConfig::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn blank_key_treated_as_unset() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = AnthropicConfig {
            api_key: "   ".to_string(),
        };
        cfg.save(dir.path()).unwrap();
        assert!(AnthropicConfig::load(dir.path()).unwrap().is_none());
    }
}
