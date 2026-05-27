//! mDNS / Bonjour responder so the running HTTP server can be reached
//! at `http://satchel.local:7428` on the local network without the user
//! looking up a numeric IP. Pure-Rust via `mdns-sd`; no system daemon
//! dependency. macOS resolves `.local` natively via mDNSResponder;
//! Windows 10+ resolves it via the built-in DNS client; Linux needs
//! `nss-mdns` (preinstalled on Ubuntu desktop, available on most
//! distros). When resolution is unavailable the LAN IP shown in the
//! Connect tab still works as a fallback.
//!
//! Behavior is gated by a persisted toggle at `<vault>/mdns.toml` so
//! users on networks where they would rather not broadcast can flip it
//! off without restarting. The toggle defaults to enabled per the
//! design conversation.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

/// On-disk state for the mDNS toggle. Tiny TOML so users can hand-edit
/// it if they want; we only ever read `enabled`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MdnsConfig {
    pub enabled: bool,
}

impl Default for MdnsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl MdnsConfig {
    fn config_path(vault_path: &Path) -> std::path::PathBuf {
        vault_path.join("mdns.toml")
    }

    pub fn load(vault_path: &Path) -> Result<MdnsConfig> {
        let p = Self::config_path(vault_path);
        if !p.exists() {
            return Ok(MdnsConfig::default());
        }
        let raw = std::fs::read_to_string(&p)?;
        let cfg: MdnsConfig = toml::from_str(&raw).unwrap_or_default();
        Ok(cfg)
    }

    pub fn save(&self, vault_path: &Path) -> Result<()> {
        let p = Self::config_path(vault_path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let s = toml::to_string(self)?;
        std::fs::write(&p, s)?;
        Ok(())
    }
}

/// Service type for HTTP under Multicast DNS.
const SERVICE_TYPE: &str = "_http._tcp.local.";
/// Stable instance name shown to mDNS browsers. The hostname users
/// reach is `<hostname>.local` and is set separately below.
const INSTANCE_NAME: &str = "SATCHEL";
/// Hostname users actually type. `mdns-sd` appends the `.local.` zone.
const HOSTNAME: &str = "satchel";

/// Live mDNS responder. `Drop` shuts the daemon down cleanly so a
/// caller that drops the handle (e.g. on toggle-off) stops broadcasting
/// immediately instead of waiting for the next TTL expiry.
pub struct Responder {
    daemon: Mutex<Option<mdns_sd::ServiceDaemon>>,
    fullname: String,
}

impl Responder {
    /// Spin up an mDNS daemon and advertise the HTTP server on `port`.
    /// Returns an error if the daemon cannot bind its multicast
    /// socket; callers should treat this as non-fatal (the HTTP server
    /// itself does not depend on mDNS being up).
    pub fn start(port: u16) -> Result<Self> {
        let daemon = mdns_sd::ServiceDaemon::new()?;
        let host_with_zone = format!("{HOSTNAME}.local.");
        // Empty IP list + enable_addr_auto means the daemon advertises
        // on every interface it discovers. That covers wired + Wi-Fi
        // without us having to enumerate `getifaddrs` ourselves.
        let mut service = mdns_sd::ServiceInfo::new(
            SERVICE_TYPE,
            INSTANCE_NAME,
            &host_with_zone,
            "",
            port,
            None,
        )?;
        service = service.enable_addr_auto();
        let fullname = service.get_fullname().to_string();
        daemon.register(service)?;
        Ok(Self {
            daemon: Mutex::new(Some(daemon)),
            fullname,
        })
    }

    /// The full service instance name (e.g. `SATCHEL._http._tcp.local.`).
    /// Useful for diagnostics and the Connect tab status line.
    pub fn fullname(&self) -> &str {
        &self.fullname
    }

    /// The hostname users actually type into a browser. Stable across
    /// runs because we control it (rather than relying on the system's
    /// computer name, which varies and would surprise users on a
    /// shared machine).
    pub fn hostname() -> &'static str {
        HOSTNAME
    }

    /// Stop broadcasting. Idempotent; safe to call from `Drop`.
    pub fn shutdown(&self) {
        let mut guard = match self.daemon.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if let Some(d) = guard.take() {
            // Unregister + shutdown are best-effort; if the daemon
            // already died, the consumer of this call does not need to
            // know about it.
            let _ = d.unregister(&self.fullname);
            let _ = d.shutdown();
        }
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn config_default_is_enabled() {
        assert!(MdnsConfig::default().enabled);
    }

    #[test]
    fn config_roundtrips_to_disk() {
        let tmp = TempDir::new().unwrap();
        let cfg = MdnsConfig { enabled: false };
        cfg.save(tmp.path()).unwrap();
        let loaded = MdnsConfig::load(tmp.path()).unwrap();
        assert!(!loaded.enabled);
    }

    #[test]
    fn config_load_missing_returns_default() {
        let tmp = TempDir::new().unwrap();
        let loaded = MdnsConfig::load(tmp.path()).unwrap();
        assert!(loaded.enabled);
    }

    #[test]
    fn hostname_is_stable() {
        assert_eq!(Responder::hostname(), "satchel");
    }
}
