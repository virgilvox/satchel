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
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

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
    /// Public quick-tunnel URL (`https://*.trycloudflare.com`). Populated
    /// once cloudflared has printed it on stderr.
    pub url: Option<String>,
    /// Local URL the tunnel is forwarding to.
    pub forwarding: Option<String>,
    /// RFC3339 timestamp of the most recent `start`.
    pub started_at: Option<String>,
    /// Last error from `start`/`stop` or the child exiting unexpectedly.
    pub error: Option<String>,
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
                url: None,
                forwarding: Some(forwarding),
                started_at: Some(chrono::Utc::now().to_rfc3339()),
                error: None,
            };
            g.child = Some(child);
        }

        // Background task: drain stderr, scrape the URL banner, watch for
        // the child exiting. Holds an Arc<Mutex<Inner>> so it can update
        // state without taking ownership of the manager.
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let url_re = regex::Regex::new(r"https://[a-z0-9-]+\.trycloudflare\.com").unwrap();
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(m) = url_re.find(&line) {
                    let mut g = inner.lock().unwrap();
                    if g.state.url.is_none() {
                        g.state.url = Some(m.as_str().to_string());
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
                    "cloudflared exited before reporting a public URL — check the local logs"
                        .to_string(),
                );
            }
        });

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
}
