//! Slack workspace export handler.
//!
//! Detection: directory containing `users.json` AND `channels.json` at the
//! root. Per-channel subdirectories hold `YYYY-MM-DD.json` daily message logs.
//!
//! Resolution: user IDs in messages (e.g. `"user": "UAA8VMDP1"`) are opaque.
//! We pre-load `users.json` and `channels.json` into lookup maps so every
//! emitted record contains `@username (Display Name)` and `#channel`,
//! making BM25 razor-sharp on identity queries.
//!
//! Each message becomes one record. Threads are emitted as parent + replies
//! glued into a single chunk so a question and its answers ride together
//! through retrieval.

use anyhow::{Context, Result};
use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

use super::{persist_record, ArchiveStats, Record};
use crate::embed::Embedder;
use crate::rag::Database;

pub fn detect(path: &Path) -> bool {
    path.is_dir()
        && path.join("users.json").is_file()
        && path.join("channels.json").is_file()
}

#[derive(Debug, Deserialize)]
struct SlackUser {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    real_name: Option<String>,
    #[serde(default)]
    profile: Option<SlackUserProfile>,
}

#[derive(Debug, Deserialize)]
struct SlackUserProfile {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    real_name_normalized: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackChannel {
    id: String,
    #[serde(default)]
    name: Option<String>,
}

struct UserInfo {
    handle: String,        // @name (preferred Slack username)
    display: Option<String>, // "Real Name" if non-empty
}

pub fn ingest(path: &Path, db: &Database, embedder: &Embedder) -> Result<ArchiveStats> {
    let users = load_users(&path.join("users.json"))?;
    let channels = load_channels(&path.join("channels.json"))?;

    eprintln!(
        "[satchel] Slack export: {} users, {} channels",
        users.len(),
        channels.len()
    );

    let mut stats = ArchiveStats::default();

    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("json"))
                .unwrap_or(false)
        })
    {
        let p = entry.path();
        // Skip the top-level metadata files; they're already loaded.
        let parent_eq_root = p.parent().map(|q| q == path).unwrap_or(false);
        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if parent_eq_root
            && (fname == "users.json"
                || fname == "channels.json"
                || fname == "groups.json"
                || fname == "mpims.json"
                || fname == "dms.json"
                || fname == "integration_logs.json"
                || fname == "canvases.json"
                || fname == "content_flags.json"
                || fname == "file_conversations.json")
        {
            continue;
        }

        // Channel name = parent directory name relative to the export root.
        let channel = p
            .parent()
            .and_then(|d| d.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        match process_day(p, &channel, &users, &channels, db, embedder) {
            Ok((added, skipped)) => {
                stats.records_added += added;
                stats.records_skipped += skipped;
            }
            Err(e) => {
                eprintln!("[satchel] Slack failed: {} - {e}", p.display());
                stats.files_failed += 1;
            }
        }
    }

    eprintln!(
        "[satchel] Slack: {} messages added, {} skipped, {} files failed",
        stats.records_added, stats.records_skipped, stats.files_failed
    );
    Ok(stats)
}

fn load_users(path: &Path) -> Result<HashMap<String, UserInfo>> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let users: Vec<SlackUser> = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {} as Slack users.json", path.display()))?;
    let mut map = HashMap::with_capacity(users.len());
    for u in users {
        let handle = u
            .profile
            .as_ref()
            .and_then(|p| p.display_name.clone())
            .filter(|s| !s.is_empty())
            .or(u.name.clone())
            .unwrap_or_else(|| u.id.clone());
        let display = u
            .real_name
            .or_else(|| u.profile.as_ref().and_then(|p| p.real_name_normalized.clone()))
            .filter(|s| !s.is_empty() && Some(s) != Some(&handle));
        map.insert(u.id.clone(), UserInfo { handle, display });
    }
    Ok(map)
}

fn load_channels(path: &Path) -> Result<HashMap<String, String>> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let chans: Vec<SlackChannel> = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {} as Slack channels.json", path.display()))?;
    Ok(chans
        .into_iter()
        .map(|c| (c.id, c.name.unwrap_or_default()))
        .collect())
}

fn process_day(
    path: &Path,
    channel: &str,
    users: &HashMap<String, UserInfo>,
    channels: &HashMap<String, String>,
    db: &Database,
    embedder: &Embedder,
) -> Result<(usize, usize)> {
    let bytes = std::fs::read(path)?;
    let messages: Vec<Value> = match serde_json::from_slice(&bytes) {
        Ok(m) => m,
        // Some daily files are empty or missing; skip silently.
        Err(_) => return Ok((0, 0)),
    };

    // First pass: index messages by ts so we can attach replies to their parent.
    let mut by_ts: HashMap<String, &Value> = HashMap::new();
    for m in &messages {
        if let Some(ts) = m.get("ts").and_then(|v| v.as_str()) {
            by_ts.insert(ts.to_string(), m);
        }
    }

    let source = path.to_string_lossy().to_string();
    let mut added = 0;
    let mut skipped = 0;

    for m in &messages {
        if !is_renderable(m) {
            continue;
        }
        // Replies are emitted as part of their parent's record. Skip non-parents.
        let ts = m.get("ts").and_then(|v| v.as_str()).unwrap_or("");
        let thread_ts = m.get("thread_ts").and_then(|v| v.as_str());
        if let Some(parent_ts) = thread_ts {
            if parent_ts != ts {
                // It's a reply; will be picked up by the parent.
                continue;
            }
        }

        let header_and_body = render_message(m, channel, users, channels);
        let title = make_title(m, channel, users);
        let mut body = header_and_body;

        // If this is a thread parent, glue replies into the same record.
        if let Some(parent_ts) = thread_ts {
            if parent_ts == ts {
                let mut replies: Vec<&Value> = messages
                    .iter()
                    .filter(|r| {
                        r.get("thread_ts").and_then(|v| v.as_str()) == Some(parent_ts)
                            && r.get("ts").and_then(|v| v.as_str()) != Some(parent_ts)
                    })
                    .collect();
                replies.sort_by_key(|r| r.get("ts").and_then(|v| v.as_str()).unwrap_or("").to_string());
                for r in replies {
                    if !is_renderable(r) {
                        continue;
                    }
                    body.push_str("\n  ↳ ");
                    body.push_str(&render_message(r, channel, users, channels));
                }
            }
        }

        let record = Record {
            source_path: source.clone(),
            record_id: format!("slack:{channel}:{ts}"),
            title,
            body,
        };

        if persist_record(&record, "slack", db, embedder)? {
            added += 1;
        } else {
            skipped += 1;
        }
    }

    Ok((added, skipped))
}

fn is_renderable(m: &Value) -> bool {
    if m.get("type").and_then(|v| v.as_str()) != Some("message") {
        return false;
    }
    let subtype = m.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
    !matches!(
        subtype,
        "channel_join"
            | "channel_leave"
            | "channel_archive"
            | "channel_unarchive"
            | "channel_name"
            | "channel_topic"
            | "channel_purpose"
            | "pinned_item"
            | "unpinned_item"
            | "bot_add"
            | "bot_remove"
            | "reminder_add"
            | "thread_broadcast" // dupe of the reply
    )
}

fn render_message(
    m: &Value,
    channel: &str,
    users: &HashMap<String, UserInfo>,
    channels: &HashMap<String, String>,
) -> String {
    let ts = m.get("ts").and_then(|v| v.as_str()).unwrap_or("0");
    let date = format_slack_ts(ts);

    let user_id = m
        .get("user")
        .and_then(|v| v.as_str())
        .or_else(|| m.get("bot_id").and_then(|v| v.as_str()))
        .unwrap_or("");

    let (handle, display) = match users.get(user_id) {
        Some(u) => (
            format!("@{}", u.handle),
            u.display.clone().map(|d| format!(" ({d})")).unwrap_or_default(),
        ),
        None => {
            // Fall back to "username" field used by some bot messages.
            let fallback = m
                .get("username")
                .and_then(|v| v.as_str())
                .unwrap_or(user_id);
            (format!("@{fallback}"), String::new())
        }
    };

    let text = extract_text(m);
    let resolved = resolve_mentions(&text, users, channels);

    format!("[{date} #{channel} {handle}{display}]: {resolved}")
}

fn make_title(m: &Value, channel: &str, users: &HashMap<String, UserInfo>) -> String {
    let user_id = m.get("user").and_then(|v| v.as_str()).unwrap_or("");
    let handle = users
        .get(user_id)
        .map(|u| format!("@{}", u.handle))
        .unwrap_or_else(|| format!("@{user_id}"));
    let date = m
        .get("ts")
        .and_then(|v| v.as_str())
        .map(format_slack_ts)
        .unwrap_or_default();
    format!("{handle} in #{channel} on {date}")
}

/// Convert Slack ts (e.g. "1482960137.003543") to "YYYY-MM-DD HH:MM".
fn format_slack_ts(ts: &str) -> String {
    let secs: i64 = ts
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    DateTime::from_timestamp(secs, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}

/// Pull text from a message, preferring `blocks` (newer rich content) over
/// the legacy `text` field. Returns the legacy text if blocks are absent.
fn extract_text(m: &Value) -> String {
    if let Some(blocks) = m.get("blocks").and_then(|v| v.as_array()) {
        let mut out = String::new();
        walk_block_text(blocks, &mut out);
        if !out.trim().is_empty() {
            return out;
        }
    }
    m.get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn walk_block_text(arr: &[Value], out: &mut String) {
    for el in arr {
        if let Some(t) = el.get("text").and_then(|v| v.as_str()) {
            out.push_str(t);
        }
        if let Some(children) = el.get("elements").and_then(|v| v.as_array()) {
            walk_block_text(children, out);
        }
    }
}

/// Replace `<@U123>` with `@username` and `<#C123|name>` with `#name`.
/// Strips angle-bracket URL wrappers (`<https://x|label>` → `label`).
fn resolve_mentions(
    text: &str,
    users: &HashMap<String, UserInfo>,
    channels: &HashMap<String, String>,
) -> String {
    use regex::Regex;
    // Lazy-once would be cleaner but compiling per-message is fine — these
    // regexes are tiny and Slack messages are short.
    let user_re = Regex::new(r"<@([A-Z0-9]+)>").unwrap();
    let chan_re = Regex::new(r"<#([A-Z0-9]+)(?:\|([^>]*))?>").unwrap();
    let url_re = Regex::new(r"<(https?://[^|>]+)(?:\|([^>]+))?>").unwrap();

    let mut s = text.to_string();
    s = user_re
        .replace_all(&s, |caps: &regex::Captures| {
            let id = &caps[1];
            users
                .get(id)
                .map(|u| format!("@{}", u.handle))
                .unwrap_or_else(|| format!("@{id}"))
        })
        .into_owned();
    s = chan_re
        .replace_all(&s, |caps: &regex::Captures| {
            let id = &caps[1];
            let name = caps
                .get(2)
                .map(|m| m.as_str().to_string())
                .or_else(|| channels.get(id).cloned())
                .unwrap_or_else(|| id.to_string());
            format!("#{name}")
        })
        .into_owned();
    s = url_re
        .replace_all(&s, |caps: &regex::Captures| {
            caps.get(2)
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| caps[1].to_string())
        })
        .into_owned();
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_users() -> HashMap<String, UserInfo> {
        let mut m = HashMap::new();
        m.insert(
            "UAA8VMDP1".to_string(),
            UserInfo {
                handle: "virgilvox".to_string(),
                display: Some("Moheeb".to_string()),
            },
        );
        m.insert(
            "U999".to_string(),
            UserInfo {
                handle: "alice".to_string(),
                display: None,
            },
        );
        m
    }

    fn sample_channels() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("C123".to_string(), "general".to_string());
        m
    }

    #[test]
    fn renders_message_with_username_not_id() {
        let m = json!({
            "type": "message",
            "user": "UAA8VMDP1",
            "ts": "1700000000.000100",
            "text": "check out my app lumencanvas.studio"
        });
        let out = render_message(&m, "general", &sample_users(), &sample_channels());
        assert!(out.contains("@virgilvox"), "got: {out}");
        assert!(out.contains("(Moheeb)"), "got: {out}");
        assert!(out.contains("#general"), "got: {out}");
        assert!(out.contains("lumencanvas.studio"), "got: {out}");
        // Must NOT leak the opaque user ID.
        assert!(!out.contains("UAA8VMDP1"), "user ID leaked: {out}");
    }

    #[test]
    fn resolves_user_and_channel_mentions() {
        let resolved = resolve_mentions(
            "hi <@UAA8VMDP1> see <#C123|general> at <https://x.com|x.com>",
            &sample_users(),
            &sample_channels(),
        );
        assert_eq!(resolved, "hi @virgilvox see #general at x.com");
    }

    #[test]
    fn skips_join_leave_subtypes() {
        let join = json!({"type":"message","subtype":"channel_join","user":"U999","ts":"1.0"});
        assert!(!is_renderable(&join));
        let regular = json!({"type":"message","user":"U999","text":"hi","ts":"1.0"});
        assert!(is_renderable(&regular));
    }

    #[test]
    fn extracts_text_from_blocks_when_present() {
        let m = json!({
            "type": "message",
            "user": "U999",
            "ts": "1.0",
            "text": "fallback",
            "blocks": [{
                "type": "rich_text",
                "elements": [{
                    "type": "rich_text_section",
                    "elements": [{"type":"text","text":"from blocks"}]
                }]
            }]
        });
        assert_eq!(extract_text(&m), "from blocks");
    }

    #[test]
    fn detect_requires_both_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("users.json"), b"[]").unwrap();
        assert!(!detect(dir.path()));
        std::fs::write(dir.path().join("channels.json"), b"[]").unwrap();
        assert!(detect(dir.path()));
    }
}
