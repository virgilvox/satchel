//! Discord chat export (DiscordChatExporter / Tyrrrz JSON format).
//!
//! Detection: a `.json` file whose top level is an object with the unique
//! key trio `{guild, channel, dateRange, messages, messageCount}`. We detect
//! at file granularity since one export = one channel = one file.

use anyhow::Result;
use serde_json::Value;
use std::path::Path;

use super::{persist_record, ArchiveStats, Record};
use crate::embed::Embedder;
use crate::ingest::progress::Progress;
use crate::rag::Database;

pub fn detect(path: &Path) -> bool {
    if !path.is_file()
        || path
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| !x.eq_ignore_ascii_case("json"))
            .unwrap_or(true)
    {
        return false;
    }
    // Read first 4 KB and probe for the signature keys without parsing the
    // whole file (some exports are large).
    let mut buf = [0u8; 4096];
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let n = f.read(&mut buf).unwrap_or(0);
    let s = std::str::from_utf8(&buf[..n]).unwrap_or("");
    s.contains("\"guild\"") && s.contains("\"channel\"") && s.contains("\"messages\"")
}

pub fn ingest(
    path: &Path,
    db: &Database,
    embedder: &Embedder,
    progress: &Progress,
) -> Result<ArchiveStats> {
    let bytes = std::fs::read(path)?;
    let v: Value = serde_json::from_slice(&bytes)?;

    let guild_name = v
        .get("guild")
        .and_then(|g| g.get("name"))
        .and_then(|x| x.as_str())
        .unwrap_or("server");
    let channel_name = v
        .get("channel")
        .and_then(|c| c.get("name"))
        .and_then(|x| x.as_str())
        .unwrap_or("channel");

    let messages = match v.get("messages").and_then(|m| m.as_array()) {
        Some(arr) => arr,
        None => return Ok(ArchiveStats::default()),
    };
    eprintln!(
        "[satchel] Discord {guild_name}/#{channel_name}: {} messages",
        messages.len()
    );

    let source = path.to_string_lossy().to_string();
    let mut stats = ArchiveStats::default();

    for m in messages {
        // Skip non-default chatter.
        let mtype = m.get("type").and_then(|v| v.as_str()).unwrap_or("Default");
        if matches!(
            mtype,
            "ChannelPinnedMessage"
                | "RecipientAdd"
                | "RecipientRemove"
                | "Call"
                | "ChannelNameChange"
                | "ChannelIconChange"
                | "GuildBoost"
                | "ThreadCreated"
        ) {
            continue;
        }
        let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let ts = m.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
        let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let author = m
            .get("author")
            .and_then(|a| a.get("nickname").and_then(|v| v.as_str()).filter(|s| !s.is_empty()))
            .or_else(|| {
                m.get("author")
                    .and_then(|a| a.get("name"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("");

        let date = ts.get(..16).unwrap_or(ts).replace('T', " ");
        let mut body = format!(
            "[{date} {guild_name}/#{channel_name} @{author}]: {content}"
        );

        // Embeds often carry the actual high-value text (link previews, quoted
        // articles). Attach as nested context.
        if let Some(embeds) = m.get("embeds").and_then(|v| v.as_array()) {
            for e in embeds {
                if let Some(d) = e.get("description").and_then(|v| v.as_str()) {
                    if !d.is_empty() {
                        body.push_str("\n  embed: ");
                        body.push_str(d);
                    }
                }
            }
        }

        if content.trim().is_empty() && !body.contains("embed:") {
            continue;
        }

        let title = format!("@{author} in {guild_name}/#{channel_name} on {date}");
        let record = Record {
            source_path: source.clone(),
            record_id: format!("discord:{id}"),
            title,
            body,
        };
        if persist_record(&record, "discord", db, embedder, progress)? {
            stats.records_added += 1;
        } else {
            stats.records_skipped += 1;
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_signature() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("export.json");
        std::fs::write(
            &p,
            br#"{"guild":{"name":"g"},"channel":{"name":"c"},"messages":[]}"#,
        )
        .unwrap();
        assert!(detect(&p));

        let q = dir.path().join("not_discord.json");
        std::fs::write(&q, br#"{"foo":"bar"}"#).unwrap();
        assert!(!detect(&q));
    }
}
