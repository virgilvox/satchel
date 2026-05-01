//! OpenAI ChatGPT data export handler.
//!
//! Detection: directory containing `conversations.json` AND `user.json` AND
//! `message_feedback.json` (the unique trio in OpenAI's export ZIP).
//!
//! `conversations.json` is an array of conversations; each is a tree of
//! mapping nodes (parent/children). We linearize the **active branch** by
//! walking from `current_node` up to root, then reversing.

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
        && path.join("user.json").is_file()
        && path.join("message_feedback.json").is_file()
}

pub fn ingest(
    path: &Path,
    db: &Database,
    embedder: &Embedder,
    progress: &Progress,
) -> Result<ArchiveStats> {
    let convo_path = path.join("conversations.json");
    let bytes = std::fs::read(&convo_path)?;
    let convos: Vec<Value> = serde_json::from_slice(&bytes)?;
    eprintln!("[satchel] ChatGPT export: {} conversations", convos.len());

    let mut stats = ArchiveStats::default();
    let source = convo_path.to_string_lossy().to_string();

    for c in &convos {
        let title = c.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        let id = c.get("conversation_id").and_then(|v| v.as_str()).unwrap_or("");
        let create_time = c.get("create_time").and_then(|v| v.as_f64()).unwrap_or(0.0);

        let body = match render_conversation(c) {
            Some(b) if !b.trim().is_empty() => b,
            _ => {
                stats.records_skipped += 1;
                continue;
            }
        };

        let header = format!(
            "[chatgpt conversation: \"{}\" started: {}]\n\n",
            title,
            format_unix_secs(create_time as i64),
        );
        let body = format!("{header}{body}");

        let record = Record {
            source_path: source.clone(),
            record_id: format!("chatgpt:{id}"),
            title: format!("ChatGPT: {title}"),
            body,
        };
        if persist_record(&record, "chatgpt", db, embedder, progress)? {
            stats.records_added += 1;
        } else {
            stats.records_skipped += 1;
        }
    }

    eprintln!(
        "[satchel] ChatGPT: {} added, {} skipped",
        stats.records_added, stats.records_skipped
    );
    Ok(stats)
}

fn render_conversation(c: &Value) -> Option<String> {
    let mapping = c.get("mapping")?.as_object()?;
    let current = c.get("current_node").and_then(|v| v.as_str())?;

    // Walk parent-chain from current_node up to root.
    let mut chain: Vec<&str> = Vec::new();
    let mut cur = Some(current);
    while let Some(id) = cur {
        chain.push(id);
        cur = mapping
            .get(id)
            .and_then(|n| n.get("parent"))
            .and_then(|v| v.as_str());
    }
    chain.reverse();

    let mut out = String::new();
    for id in chain {
        let node = match mapping.get(id) {
            Some(n) => n,
            None => continue,
        };
        let msg = match node.get("message") {
            Some(m) if !m.is_null() => m,
            _ => continue,
        };
        // Skip hidden/system messages.
        let hidden = msg
            .get("metadata")
            .and_then(|m| m.get("is_visually_hidden_from_conversation"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if hidden {
            continue;
        }
        let role = msg
            .get("author")
            .and_then(|a| a.get("role"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if role == "system" || role == "tool" {
            continue;
        }
        let content = msg.get("content")?;
        let text = extract_content_text(content);
        if text.trim().is_empty() {
            continue;
        }
        out.push_str(&format!("@{role}: {text}\n\n"));
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn extract_content_text(content: &Value) -> String {
    let ctype = content
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match ctype {
        "text" => content
            .get("parts")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default(),
        "code" => {
            let lang = content
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let body = content
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("```{lang}\n{body}\n```")
        }
        "multimodal_text" => content
            .get("parts")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|p| {
                        if let Some(s) = p.as_str() {
                            s.to_string()
                        } else if p.get("content_type").and_then(|v| v.as_str())
                            == Some("image_asset_pointer")
                        {
                            "[image]".to_string()
                        } else {
                            String::new()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default(),
        _ => content
            .get("parts")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

fn format_unix_secs(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_conversation() -> Value {
        json!({
            "title": "Test Chat",
            "conversation_id": "abc-123",
            "create_time": 1700000000.0,
            "current_node": "n3",
            "mapping": {
                "root": {"id":"root","parent":null,"children":["n1"],"message":null},
                "n1": {
                    "id":"n1","parent":"root","children":["n2"],
                    "message":{
                        "author":{"role":"user"},
                        "content":{"content_type":"text","parts":["hello"]}
                    }
                },
                "n2": {
                    "id":"n2","parent":"n1","children":["n3"],
                    "message":{
                        "author":{"role":"assistant"},
                        "content":{"content_type":"text","parts":["hi back"]}
                    }
                },
                "n3": {
                    "id":"n3","parent":"n2","children":[],
                    "message":{
                        "author":{"role":"user"},
                        "content":{"content_type":"text","parts":["bye"]}
                    }
                }
            }
        })
    }

    #[test]
    fn linearizes_active_branch_in_order() {
        let body = render_conversation(&make_conversation()).unwrap();
        let h = body.find("hello").unwrap();
        let b = body.find("hi back").unwrap();
        let by = body.find("bye").unwrap();
        assert!(h < b && b < by, "messages out of order: {body}");
    }

    #[test]
    fn skips_system_messages() {
        let c = json!({
            "title":"x","conversation_id":"x","create_time":0.0,"current_node":"a",
            "mapping": {
                "a": {"id":"a","parent":null,"children":[],
                      "message":{"author":{"role":"system"},
                                 "content":{"content_type":"text","parts":["secret prompt"]}}}
            }
        });
        let body = render_conversation(&c).unwrap_or_default();
        assert!(!body.contains("secret prompt"));
    }

    #[test]
    fn detect_requires_all_signatures() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("conversations.json"), b"[]").unwrap();
        assert!(!detect(dir.path()));
        std::fs::write(dir.path().join("user.json"), b"{}").unwrap();
        assert!(!detect(dir.path()));
        std::fs::write(dir.path().join("message_feedback.json"), b"[]").unwrap();
        assert!(detect(dir.path()));
    }
}
