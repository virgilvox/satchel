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

/// Strip request fields that the requested model rejects.
///
/// Two model-specific quirks the chat UI already avoids; we mirror
/// them server-side as defense in depth so third-party clients (curl,
/// scripts, alternative chat frontends) hitting this proxy do not
/// trip a 400 from Anthropic.
///
/// 1. Opus 4.7 returns 400 if `temperature`, `top_p`, or `top_k` are
///    present (sampling controls were dropped on the 4.7 line).
/// 2. Haiku 4.5 does not expose the extended-thinking surface, so
///    `thinking` and `output_config.effort` return 400 (Haiku has no
///    extended-thinking budget to spend). The Chat UI gates these
///    behind a `supportsExtendedThinking` flag on each ChatModel and
///    omits them client-side; this strip is for everyone else.
fn strip_unsupported_params(body: &mut serde_json::Map<String, serde_json::Value>) {
    // Take an owned copy so the borrow released and we can mutate
    // `body` freely below. The string is short so the allocation is
    // negligible; the alternative (split-borrow tricks) is uglier.
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if model.starts_with("claude-opus-4-7") {
        body.remove("temperature");
        body.remove("top_p");
        body.remove("top_k");
    }
    if model.starts_with("claude-haiku") {
        body.remove("thinking");
        body.remove("output_config");
    }
}

/// Stream a Messages API call. The caller passes the raw JSON body the
/// browser built. We add the auth + version headers, force `stream:
/// true` if the caller forgot, and strip sampling parameters that the
/// requested model would reject.
pub async fn proxy_messages(api_key: &str, body: serde_json::Value) -> Result<reqwest::Response> {
    if api_key.trim().is_empty() {
        bail!("anthropic api key not configured — POST to /api/anthropic/config first");
    }

    // Guarantee streaming. Anthropic accepts non-stream too, but we
    // always want SSE; that's the path the chat UI knows.
    let mut body = body;
    if let Some(map) = body.as_object_mut() {
        map.entry("stream").or_insert(serde_json::Value::Bool(true));
        strip_unsupported_params(map);
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

    #[test]
    fn strips_sampling_params_on_opus_4_7() {
        let mut body: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
            r#"{"model":"claude-opus-4-7","temperature":0.7,"top_p":0.9,"top_k":40,"max_tokens":1024}"#,
        )
        .unwrap();
        strip_unsupported_params(&mut body);
        assert!(!body.contains_key("temperature"));
        assert!(!body.contains_key("top_p"));
        assert!(!body.contains_key("top_k"));
        assert_eq!(body.get("max_tokens").unwrap().as_i64().unwrap(), 1024);
    }

    #[test]
    fn keeps_sampling_params_on_sonnet() {
        let mut body: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
            r#"{"model":"claude-sonnet-4-6","temperature":0.7,"max_tokens":1024}"#,
        )
        .unwrap();
        strip_unsupported_params(&mut body);
        assert!(body.contains_key("temperature"));
    }

    #[test]
    fn missing_model_is_a_noop() {
        let mut body: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(r#"{"temperature":0.7,"max_tokens":1024}"#).unwrap();
        strip_unsupported_params(&mut body);
        assert!(body.contains_key("temperature"));
    }

    #[test]
    fn strips_thinking_and_output_config_on_haiku() {
        // Haiku 4.5 returns 400 if `thinking` or `output_config` are
        // present (no extended-thinking surface). Defense in depth
        // mirrors the Chat UI's capability gate so third-party clients
        // hitting the proxy do not blow up on a typo.
        let mut body: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
            r#"{"model":"claude-haiku-4-5","thinking":{"type":"adaptive"},"output_config":{"effort":"high"},"max_tokens":2048}"#,
        )
        .unwrap();
        strip_unsupported_params(&mut body);
        assert!(!body.contains_key("thinking"));
        assert!(!body.contains_key("output_config"));
        assert_eq!(body.get("max_tokens").unwrap().as_i64().unwrap(), 2048);
    }

    #[test]
    fn keeps_thinking_on_opus_and_sonnet() {
        // Sanity check the strip is model-specific and not over-broad.
        for model in ["claude-opus-4-7", "claude-sonnet-4-6"] {
            let mut body: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
                &format!(
                    r#"{{"model":"{model}","thinking":{{"type":"adaptive"}},"output_config":{{"effort":"high"}}}}"#
                ),
            )
            .unwrap();
            strip_unsupported_params(&mut body);
            assert!(body.contains_key("thinking"), "{model} dropped thinking");
            assert!(
                body.contains_key("output_config"),
                "{model} dropped output_config"
            );
        }
    }
}
