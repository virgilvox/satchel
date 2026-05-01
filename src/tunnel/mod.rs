//! Cloudflare quick-tunnel manager.
//!
//! There is no native Rust client for the Cloudflare Tunnel protocol — the
//! wire format is reverse-engineered Cloudflare-internal traffic and the
//! reference implementation lives in their Go daemon `cloudflared`. We drive
//! it as a child process instead.
//!
//! The release CI bundles a per-platform `cloudflared` (~35 MB) next to the
//! satchel binary so the user gets one-click public tunnels with no extra
//! install. Cargo-from-source builds and `cargo install` users still get the
//! feature as long as `cloudflared` is somewhere on `$PATH` — we look in two
//! places, in order:
//!
//!   1. `<satchel-binary-dir>/cloudflared` (or `cloudflared.exe` on Windows)
//!      — the release-CI-bundled copy. Inside `Satchel.app` this resolves to
//!      `Satchel.app/Contents/MacOS/cloudflared`.
//!   2. `$PATH` — fallback for source builds.
//!
//! Quick tunnels are anonymous (no Cloudflare account needed) and ephemeral —
//! perfect for one-click "make this satchel reachable from claude.ai web for
//! the next hour" workflows.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// Tunnel mode the user chose for the most recent (or current) run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelMode {
    /// Anonymous one-shot tunnel — random `*.trycloudflare.com` URL,
    /// no Cloudflare account needed, dies with the process.
    #[default]
    Quick,
    /// Persistent named tunnel from the user's Cloudflare Zero Trust
    /// dashboard. The connector token + the public hostname they
    /// configured live in `<vault_path>/tunnel.toml`. The hostname maps
    /// to a stable URL on their domain (or `*.cfargotunnel.com`).
    Named,
}

/// Locate the `cloudflared` binary. Prefers the bundled copy next to the
/// running satchel executable; falls back to `$PATH` (returns the bare name
/// `"cloudflared"` so `Command::new` performs the lookup).
fn locate_cloudflared() -> PathBuf {
    let bare = if cfg!(windows) {
        "cloudflared.exe"
    } else {
        "cloudflared"
    };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let cand = parent.join(bare);
            if cand.is_file() {
                return cand;
            }
        }
    }
    PathBuf::from(bare)
}

/// Public snapshot of tunnel state. Cheap to clone; serialized straight to
/// the JSON returned by `/api/tunnel`.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct TunnelState {
    /// Whether `cloudflared` was found on `$PATH` the last time we checked.
    pub installed: bool,
    /// True between a successful `start` and the next `stop` / child exit.
    pub running: bool,
    /// Mode of the current/last run. Drives UI labelling.
    pub mode: TunnelMode,
    /// Public URL. For `Quick` this is parsed out of cloudflared's stderr
    /// banner once available; for `Named` it's `https://<hostname>` set at
    /// spawn time but only displayed after we see a "Registered tunnel
    /// connection" line so the user knows the edge accepted us.
    pub url: Option<String>,
    /// Local URL the tunnel is forwarding to.
    pub forwarding: Option<String>,
    /// RFC3339 timestamp of the most recent `start`.
    pub started_at: Option<String>,
    /// Last error from `start`/`stop` or the child exiting unexpectedly.
    pub error: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────
// TunnelConfig — persisted named-tunnel credentials.
//
// Stored at `<vault_path>/tunnel.toml`. Schema is intentionally tiny:
//
//     token = "eyJh..."          # connector token from Zero Trust dash
//     hostname = "vault.example.com"  # the public hostname the tunnel
//                                     # routes to satchel
//
// On Unix we chmod the file to 0600 after write — the connector token can
// be used to start a tunnel as the user's account, so we treat it like a
// password and minimise blast radius.
// ─────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TunnelConfig {
    pub token: String,
    pub hostname: String,
}

impl TunnelConfig {
    fn path(vault_path: &Path) -> PathBuf {
        vault_path.join("tunnel.toml")
    }

    pub fn load(vault_path: &Path) -> Result<Option<Self>> {
        let p = Self::path(vault_path);
        if !p.is_file() {
            return Ok(None);
        }
        let body = std::fs::read_to_string(&p)
            .with_context(|| format!("failed to read {}", p.display()))?;
        let cfg: TunnelConfig =
            toml::from_str(&body).with_context(|| format!("failed to parse {}", p.display()))?;
        if cfg.token.trim().is_empty() || cfg.hostname.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(cfg))
    }

    pub fn save(&self, vault_path: &Path) -> Result<()> {
        std::fs::create_dir_all(vault_path).with_context(|| {
            format!("failed to create vault directory {}", vault_path.display())
        })?;
        let p = Self::path(vault_path);
        let body = toml::to_string(self).context("failed to serialize tunnel config")?;
        std::fs::write(&p, body).with_context(|| format!("failed to write {}", p.display()))?;

        // Lock the file down on Unix — connector tokens are credentials.
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

struct Inner {
    state: TunnelState,
    /// Live child handle. Keeps `kill_on_drop(true)` so the subprocess dies
    /// with satchel — but stop() also gracefully kills it on demand.
    child: Option<Child>,
}

/// Thread-safe handle to the singleton tunnel for the whole satchel
/// process. Cheap to clone (it's an `Arc` under the hood); pass it into
/// the axum router via `AppState`.
#[derive(Clone)]
pub struct TunnelManager {
    inner: Arc<Mutex<Inner>>,
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TunnelManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                state: TunnelState::default(),
                child: None,
            })),
        }
    }

    /// Probe the host for `cloudflared --version`. We don't cache the
    /// result — installs/uninstalls during a satchel session are rare but
    /// possible (e.g. user `brew install`s in another terminal and refreshes
    /// the tab) and the probe is cheap (a fork + 50ms).
    pub async fn check_installed(&self) -> bool {
        let path = locate_cloudflared();
        let installed = Command::new(&path)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);
        let mut g = self.inner.lock().unwrap();
        g.state.installed = installed;
        installed
    }

    /// Start a quick tunnel pointing at `http://localhost:{port}`.
    /// Returns immediately after the child is spawned; the public URL is
    /// scraped from cloudflared's stderr in a background task and lands in
    /// the next `snapshot()` once available (typically <2 s).
    pub async fn start_quick(&self, port: u16) -> Result<()> {
        // Refuse if already running — caller should `stop()` first.
        {
            let g = self.inner.lock().unwrap();
            if g.state.running || g.child.is_some() {
                bail!("a tunnel is already running; stop it first");
            }
        }

        let forwarding = format!("http://localhost:{port}");
        let path = locate_cloudflared();
        let mut cmd = Command::new(&path);
        cmd.args([
            "tunnel",
            "--url",
            &forwarding,
            "--no-autoupdate",
            // `--protocol auto` is the default, but pinning it makes the
            // command line deterministic across cloudflared versions.
            "--protocol",
            "auto",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

        let mut child = cmd.spawn().with_context(|| {
            format!(
                "failed to spawn {} (is cloudflared bundled or installed?)",
                path.display()
            )
        })?;

        // Take stderr before we stash the child — the borrow checker won't
        // let us pull it out after the move.
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("cloudflared stderr pipe missing"))?;

        // Reset state and stash the child handle.
        {
            let mut g = self.inner.lock().unwrap();
            g.state = TunnelState {
                installed: true,
                running: true,
                mode: TunnelMode::Quick,
                url: None,
                forwarding: Some(forwarding),
                started_at: Some(chrono::Utc::now().to_rfc3339()),
                error: None,
            };
            g.child = Some(child);
        }

        // Background task: drain stderr, scrape the trycloudflare URL,
        // watch for the child exiting. Holds an Arc<Mutex<Inner>> so it
        // can update state without taking ownership of the manager.
        spawn_stderr_reader(
            stderr,
            Arc::clone(&self.inner),
            // For quick tunnels the URL is published in cloudflared's
            // stderr banner — match it.
            UrlSource::ScrapeStderr,
        );

        Ok(())
    }

    /// Start a named tunnel using a connector token from the user's
    /// Cloudflare Zero Trust dashboard. The dashboard's tunnel config
    /// dictates the local service URL — we don't pass `--url` here.
    pub async fn start_named(&self, token: &str, hostname: &str, port: u16) -> Result<()> {
        if token.trim().is_empty() {
            bail!("tunnel token is empty — paste the connector token from the Cloudflare Zero Trust dashboard");
        }
        if hostname.trim().is_empty() {
            bail!("tunnel hostname is empty — set the public hostname you configured in the Cloudflare dashboard");
        }
        // Refuse if already running.
        {
            let g = self.inner.lock().unwrap();
            if g.state.running || g.child.is_some() {
                bail!("a tunnel is already running; stop it first");
            }
        }

        let path = locate_cloudflared();
        // `tunnel run --token <T>` is the documented form; cloudflared also
        // accepts `tunnel --token <T>` without `run` in modern versions but
        // we use the explicit `run` form for clarity.
        let mut cmd = Command::new(&path);
        cmd.args(["tunnel", "run", "--no-autoupdate", "--token", token])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().with_context(|| {
            format!(
                "failed to spawn {} (is cloudflared bundled or installed?)",
                path.display()
            )
        })?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("cloudflared stderr pipe missing"))?;

        // For named tunnels the public URL is `https://<hostname>` —
        // known up-front. We hold off on populating `url` until the edge
        // confirms the connection so the UI's "starting" → "live" arc
        // stays meaningful.
        let public_url = format!(
            "https://{}",
            hostname
                .trim_start_matches("https://")
                .trim_end_matches('/')
        );
        let forwarding = format!("http://localhost:{port}");

        {
            let mut g = self.inner.lock().unwrap();
            g.state = TunnelState {
                installed: true,
                running: true,
                mode: TunnelMode::Named,
                url: None,
                forwarding: Some(forwarding),
                started_at: Some(chrono::Utc::now().to_rfc3339()),
                error: None,
            };
            g.child = Some(child);
        }

        spawn_stderr_reader(
            stderr,
            Arc::clone(&self.inner),
            UrlSource::OnConnectRegistered(public_url),
        );

        Ok(())
    }

    /// Kill the running tunnel. No-op if there isn't one.
    pub async fn stop(&self) -> Result<()> {
        let mut maybe_child = {
            let mut g = self.inner.lock().unwrap();
            g.state.running = false;
            g.state.url = None;
            g.state.forwarding = None;
            g.state.started_at = None;
            g.child.take()
        };
        if let Some(child) = maybe_child.as_mut() {
            // Best effort: kill the process, then reap so we don't leave a
            // zombie. cloudflared honors SIGTERM cleanly.
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        Ok(())
    }

    pub fn snapshot(&self) -> TunnelState {
        self.inner.lock().unwrap().state.clone()
    }
}

/// How the background reader should populate `state.url`. Quick tunnels
/// publish a `*.trycloudflare.com` URL on stderr that we scrape verbatim;
/// named tunnels know their URL up-front and only need a "connection
/// registered" log line as a signal that the edge accepted the tunnel.
enum UrlSource {
    ScrapeStderr,
    OnConnectRegistered(String),
}

/// Spawn a background task that drains cloudflared's stderr, populates
/// `state.url` per `UrlSource`, and reflects child exit in `state`.
fn spawn_stderr_reader(
    stderr: tokio::process::ChildStderr,
    inner: Arc<Mutex<Inner>>,
    source: UrlSource,
) {
    tokio::spawn(async move {
        let url_re = regex::Regex::new(r"https://[a-z0-9-]+\.trycloudflare\.com").unwrap();
        // cloudflared logs "Registered tunnel connection" once the edge
        // accepts the named tunnel. Some versions print "Connection
        // registered" instead — match either.
        let connected_re =
            regex::Regex::new(r"(?i)Registered tunnel connection|Connection \S+ registered")
                .unwrap();

        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            match &source {
                UrlSource::ScrapeStderr => {
                    if let Some(m) = url_re.find(&line) {
                        let mut g = inner.lock().unwrap();
                        if g.state.url.is_none() {
                            g.state.url = Some(m.as_str().to_string());
                        }
                    }
                }
                UrlSource::OnConnectRegistered(public_url) => {
                    if connected_re.is_match(&line) {
                        let mut g = inner.lock().unwrap();
                        if g.state.url.is_none() {
                            g.state.url = Some(public_url.clone());
                        }
                    }
                }
            }
        }
        // stderr EOF means cloudflared exited. Two cases:
        //   1. The user called stop(): `running` is already false
        //      (stop() flipped it before killing the child). No error.
        //   2. cloudflared crashed/exited on its own: `running` is
        //      still true. Mark it down and surface a hint.
        let mut g = inner.lock().unwrap();
        let was_running = g.state.running;
        g.state.running = false;
        g.child = None;
        if was_running && g.state.url.is_none() {
            g.state.error = Some(
                "cloudflared exited before establishing a tunnel — check the host's cloudflared logs"
                    .to_string(),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn snapshot_starts_empty() {
        let m = TunnelManager::new();
        let s = m.snapshot();
        assert!(!s.running);
        assert!(s.url.is_none());
        assert!(s.error.is_none());
    }

    #[tokio::test]
    async fn stop_is_idempotent() {
        let m = TunnelManager::new();
        m.stop().await.unwrap();
        m.stop().await.unwrap();
        assert!(!m.snapshot().running);
    }

    #[tokio::test]
    async fn check_installed_does_not_panic() {
        // Whether or not cloudflared is on PATH, the probe must not panic.
        let m = TunnelManager::new();
        let _ = m.check_installed().await;
    }

    #[test]
    fn config_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        assert!(TunnelConfig::load(dir.path()).unwrap().is_none());
        let cfg = TunnelConfig {
            token: "eyJtest".to_string(),
            hostname: "vault.example.com".to_string(),
        };
        cfg.save(dir.path()).unwrap();
        let back = TunnelConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(back.token, "eyJtest");
        assert_eq!(back.hostname, "vault.example.com");
        TunnelConfig::clear(dir.path()).unwrap();
        assert!(TunnelConfig::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn config_treats_blank_as_unset() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = TunnelConfig {
            token: "".to_string(),
            hostname: "".to_string(),
        };
        cfg.save(dir.path()).unwrap();
        // Empty fields should round-trip back to None — never start a
        // named tunnel with no token.
        assert!(TunnelConfig::load(dir.path()).unwrap().is_none());
    }

    #[tokio::test]
    async fn start_named_rejects_empty_inputs() {
        let m = TunnelManager::new();
        assert!(m.start_named("", "vault.example.com", 7428).await.is_err());
        assert!(m.start_named("eyJtest", "", 7428).await.is_err());
    }
}
