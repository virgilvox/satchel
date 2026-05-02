//! GitHub release-update probe. Hits the public
//! `/repos/{owner}/{repo}/releases/latest` endpoint, compares the latest
//! tag against `CARGO_PKG_VERSION`, and returns a struct the web UI can
//! render as a small "update available" hint.
//!
//! The probe is opt-out via the `SATCHEL_DISABLE_UPDATE_CHECK` env var
//! and aggressively cached (1 hour) so even a hot-reloading dev session
//! makes one network call per hour. GitHub allows 60 unauthenticated
//! requests per hour per IP — well above what one running satchel can
//! plausibly burn.

use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReleaseInfo {
    /// Version string from `CARGO_PKG_VERSION`. Always present.
    pub current: String,
    /// Latest tag stripped of a leading `v`. None when the probe fails
    /// or update checks are disabled.
    pub latest: Option<String>,
    /// True only when both `latest` and `current` parse as semver triples
    /// AND the latest is strictly greater. False on parse failure or any
    /// network error so the UI never shows a misleading hint.
    pub update_available: bool,
    pub release_url: Option<String>,
    pub published_at: Option<String>,
    /// ISO8601 timestamp of the most recent successful (or attempted)
    /// fetch. The UI surfaces this so users know when the data is from.
    pub checked_at: String,
    /// Non-fatal probe errors — network down, rate-limit, parse failure.
    /// Surfaced in the UI as a tooltip rather than bothering the user.
    pub error: Option<String>,
    /// Mirrors `SATCHEL_DISABLE_UPDATE_CHECK`. UI hides the entire
    /// update strip when true.
    pub disabled: bool,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    published_at: Option<String>,
}

/// Owner/repo pair extracted from `CARGO_PKG_REPOSITORY` at compile time.
/// Falls back to virgilvox/satchel if the manifest URL isn't a recognizable
/// GitHub URL — keeps the probe useful for source builds.
fn owner_repo() -> (String, String) {
    let repo = env!("CARGO_PKG_REPOSITORY");
    if let Some(rest) = repo
        .strip_prefix("https://github.com/")
        .or_else(|| repo.strip_prefix("http://github.com/"))
    {
        let trimmed = rest.trim_end_matches('/').trim_end_matches(".git");
        let mut parts = trimmed.splitn(2, '/');
        if let (Some(owner), Some(repo)) = (parts.next(), parts.next()) {
            return (owner.to_string(), repo.to_string());
        }
    }
    ("virgilvox".to_string(), "satchel".to_string())
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

/// Compare two `MAJOR.MINOR.PATCH` strings. Returns true iff `latest`
/// is strictly greater. Versions that don't parse return false rather
/// than panicking — that lets us safely accept any tag shape and only
/// raise the flag when we're confident.
fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(s: &str) -> Option<(u64, u64, u64)> {
        let mut it = s.split('.').map(|p| {
            // Strip pre-release / build suffixes (-rc.1, +sha) so a tag
            // like "2.1.1-beta" still compares cleanly against 2.1.0.
            p.split(['-', '+']).next().unwrap_or("").parse::<u64>().ok()
        });
        Some((
            it.next()??,
            it.next().unwrap_or(Some(0))?,
            it.next().unwrap_or(Some(0))?,
        ))
    }
    match (parse(latest), parse(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

async fn fetch_once() -> ReleaseInfo {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let checked_at = now_iso();

    if std::env::var_os("SATCHEL_DISABLE_UPDATE_CHECK").is_some() {
        return ReleaseInfo {
            current,
            latest: None,
            update_available: false,
            release_url: None,
            published_at: None,
            checked_at,
            error: None,
            disabled: true,
        };
    }

    let (owner, repo) = owner_repo();
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");

    let client = match reqwest::Client::builder()
        .user_agent(format!("satchel/{current}"))
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return ReleaseInfo {
                current,
                latest: None,
                update_available: false,
                release_url: None,
                published_at: None,
                checked_at,
                error: Some(format!("client: {e}")),
                disabled: false,
            };
        }
    };

    let res = match client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return ReleaseInfo {
                current,
                latest: None,
                update_available: false,
                release_url: None,
                published_at: None,
                checked_at,
                error: Some(format!("network: {e}")),
                disabled: false,
            };
        }
    };

    if !res.status().is_success() {
        let status = res.status();
        return ReleaseInfo {
            current,
            latest: None,
            update_available: false,
            release_url: None,
            published_at: None,
            checked_at,
            error: Some(format!("github: {status}")),
            disabled: false,
        };
    }

    let rel: GhRelease = match res.json().await {
        Ok(r) => r,
        Err(e) => {
            return ReleaseInfo {
                current,
                latest: None,
                update_available: false,
                release_url: None,
                published_at: None,
                checked_at,
                error: Some(format!("parse: {e}")),
                disabled: false,
            };
        }
    };

    let latest_clean = rel.tag_name.trim_start_matches('v').to_string();
    let update = is_newer(&latest_clean, &current);

    ReleaseInfo {
        current,
        latest: Some(latest_clean),
        update_available: update,
        release_url: Some(rel.html_url),
        published_at: rel.published_at,
        checked_at,
        error: None,
        disabled: false,
    }
}

/// Process-wide cache for the release probe. Keeps GitHub round-trips
/// to one per hour even with many open browser tabs hitting `/api/release`.
#[derive(Clone)]
pub struct ReleaseCache {
    inner: Arc<Mutex<Option<(Instant, ReleaseInfo)>>>,
    ttl: Duration,
}

impl ReleaseCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            ttl,
        }
    }

    pub async fn get_or_fetch(&self, force: bool) -> ReleaseInfo {
        if !force {
            let guard = self.inner.lock().await;
            if let Some((at, info)) = guard.as_ref() {
                if at.elapsed() < self.ttl {
                    return info.clone();
                }
            }
        }
        let fresh = fetch_once().await;
        let mut guard = self.inner.lock().await;
        *guard = Some((Instant::now(), fresh.clone()));
        fresh
    }
}

impl Default for ReleaseCache {
    fn default() -> Self {
        Self::new(Duration::from_secs(60 * 60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_basic() {
        assert!(is_newer("2.1.2", "2.1.1"));
        assert!(is_newer("2.2.0", "2.1.99"));
        assert!(is_newer("3.0.0", "2.99.99"));
        assert!(!is_newer("2.1.1", "2.1.1"));
        assert!(!is_newer("2.1.0", "2.1.1"));
        assert!(!is_newer("1.99.99", "2.0.0"));
    }

    #[test]
    fn is_newer_handles_short_versions() {
        // "2.1" reads as 2.1.0
        assert!(is_newer("2.2", "2.1.99"));
        assert!(!is_newer("2.1", "2.1.0"));
    }

    #[test]
    fn is_newer_handles_pre_release_suffix() {
        // The tag stripping leaves us comparing the numeric parts only;
        // a -beta tag at the same triple is treated as equal, not newer.
        assert!(!is_newer("2.1.1-beta", "2.1.1"));
        assert!(is_newer("2.1.2-rc1", "2.1.1"));
    }

    #[test]
    fn is_newer_unparseable_returns_false() {
        // Defensive: never raise the flag on garbage tags.
        assert!(!is_newer("garbage", "2.1.1"));
        assert!(!is_newer("2.1.1", "garbage"));
        assert!(!is_newer("", "2.1.1"));
    }

    #[test]
    fn owner_repo_from_known_url() {
        let (o, r) = owner_repo();
        assert_eq!(o, "virgilvox");
        assert_eq!(r, "satchel");
    }
}
