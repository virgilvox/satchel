//! Anthropic Claude.ai personal data export handler.
//!
//! Detection: directory containing `conversations.json` AND `users.json` AND
//! `projects.json`. Critical disambiguator vs ChatGPT: presence of
//! `projects.json` and absence of `mapping` (Claude conversations are flat).

use anyhow::Result;
use serde_json::Value;
use std::path::Path;

use super::{persist_record, ArchiveStats, Record};
use crate::embed::Embedder;
use crate::ingest::progress::Progress;
use crate::rag::Database;

pub fn detect(path: &Path) -> bool {
    path.is_dir()
        && path.join("conversations.json").is_file()
        && path.join("users.json").is_file()
        && path.join("projects.json").is_file()
}

pub fn ingest(
    path: &Path,
    db: &Database,
    embedder: &Embedder,
    progress: &Progress,
) -> Result<ArchiveStats> {
    let convo_path = path.join("conversations.json");
    let bytes = std::fs::read(&convo_path)?;
    let convos: Vec<Value> = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        // JSONL fallback: one conversation per line.
        Err(_) => std::str::from_utf8(&bytes)?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str::<Value>)
            .collect::<Result<Vec<_>, _>>()?,
    };
    eprintln!("[satchel] Claude.ai export: {} conversations", convos.len());

    let mut stats = ArchiveStats::default();
    let source = convo_path.to_string_lossy().to_string();

    for c in &convos {
        let title = c.get("name").and_then(|v| v.as_str()).unwrap_or("Untitled");
        let uuid = c.get("uuid").and_then(|v| v.as_str()).unwrap_or("");
        let created = c.get("created_at").and_then(|v| v.as_str()).unwrap_or("");

        let messages = match c.get("chat_messages").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => {
                stats.records_skipped += 1;
                continue;
            }
        };

        let mut body = format!("[claude.ai conversation: \"{title}\" started: {created}]\n\n");
        let mut wrote_any = false;
        for m in messages {
            let sender = m.get("sender").and_then(|v| v.as_str()).unwrap_or("");
            let role = match sender {
                "human" => "@human",
                "assistant" => "@assistant",
                other => other,
            };
            // Prefer flattened text; fall back to walking content blocks.
            let text = m
                .get("text")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| extract_content_blocks(m.get("content")));
            if text.trim().is_empty() {
                continue;
            }
            body.push_str(&format!("{role}: {text}\n\n"));
            wrote_any = true;

            // Inline attachment-extracted text.
            if let Some(atts) = m.get("attachments").and_then(|v| v.as_array()) {
                for a in atts {
                    let name = a.get("file_name").and_then(|v| v.as_str()).unwrap_or("");
                    let extracted = a
                        .get("extracted_content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !extracted.is_empty() {
                        body.push_str(&format!("[attachment: {name}]\n{extracted}\n\n"));
                    }
                }
            }
        }

        if !wrote_any {
            stats.records_skipped += 1;
            continue;
        }

        let record = Record {
            source_path: source.clone(),
            record_id: format!("claude.ai:{uuid}"),
            title: format!("Claude.ai: {title}"),
            body,
        };
        if persist_record(&record, "claude.ai", db, embedder, progress)? {
            stats.records_added += 1;
        } else {
            stats.records_skipped += 1;
        }
    }

    eprintln!(
        "[satchel] Claude.ai: {} added, {} skipped",
        stats.records_added, stats.records_skipped
    );
    Ok(stats)
}

fn extract_content_blocks(content: Option<&Value>) -> String {
    let arr = match content.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return String::new(),
    };
    let mut out = String::new();
    for b in arr {
        match b.get("type").and_then(|v| v.as_str()) {
            Some("text") => {
                if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                    out.push_str(t);
                    out.push('\n');
                }
            }
            Some("tool_use") => {
                if let Some(name) = b.get("name").and_then(|v| v.as_str()) {
                    out.push_str(&format!("[tool_use: {name}]\n"));
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detect_requires_all_three_signatures() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("conversations.json"), b"[]").unwrap();
        assert!(!detect(dir.path()));
        std::fs::write(dir.path().join("users.json"), b"[]").unwrap();
        assert!(!detect(dir.path()));
        std::fs::write(dir.path().join("projects.json"), b"[]").unwrap();
        assert!(detect(dir.path()));
    }

    #[test]
    fn extract_text_block() {
        let c = json!([
            {"type":"text","text":"hello world"},
            {"type":"tool_use","name":"web_search"}
        ]);
        let s = extract_content_blocks(Some(&c));
        assert!(s.contains("hello world"));
        assert!(s.contains("[tool_use: web_search]"));
    }
}
