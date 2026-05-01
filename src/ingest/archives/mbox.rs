//! Email mbox handler (Gmail Takeout / generic mbox files).
//!
//! Detection: file extension `.mbox` OR a file whose first non-blank line
//! starts with `From ` (the mbox-O message separator).
//!
//! Each message becomes one record. Threads are not stitched; mbox provides
//! `In-Reply-To`/`References` if needed but Gmail Takeout's `X-GM-THRID`
//! header is more reliable when present.

use anyhow::Result;
use mail_parser::MessageParser;
use std::path::Path;

use super::{persist_record, ArchiveStats, Record};
use crate::embed::Embedder;
use crate::rag::Database;

pub fn detect(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    if path
        .extension()
        .and_then(|x| x.to_str())
        .map(|x| x.eq_ignore_ascii_case("mbox"))
        .unwrap_or(false)
    {
        return true;
    }
    // Probe first 64 bytes: mbox-O messages begin with "From " (not "From:").
    let mut buf = [0u8; 64];
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let n = f.read(&mut buf).unwrap_or(0);
    let s = std::str::from_utf8(&buf[..n]).unwrap_or("");
    s.starts_with("From ")
}

pub fn ingest(path: &Path, db: &Database, embedder: &Embedder) -> Result<ArchiveStats> {
    let bytes = std::fs::read(path)?;
    let raw = std::str::from_utf8(&bytes).unwrap_or("");
    let messages = split_mbox(raw);
    eprintln!("[satchel] mbox: {} messages", messages.len());

    let source = path.to_string_lossy().to_string();
    let mut stats = ArchiveStats::default();

    for (i, raw) in messages.iter().enumerate() {
        let parsed = match MessageParser::default().parse(raw.as_bytes()) {
            Some(m) => m,
            None => {
                stats.records_skipped += 1;
                continue;
            }
        };
        let from = parsed
            .from()
            .and_then(|a| a.first())
            .map(|addr| {
                let name = addr.name().unwrap_or("");
                let email = addr.address().unwrap_or("");
                if name.is_empty() {
                    email.to_string()
                } else {
                    format!("{name} <{email}>")
                }
            })
            .unwrap_or_default();
        let to: Vec<String> = parsed
            .to()
            .map(|addrs| {
                addrs
                    .iter()
                    .map(|a| a.address().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default();
        let subject = parsed.subject().unwrap_or("(no subject)");
        let date = parsed
            .date()
            .map(|d| d.to_rfc3339().chars().take(16).collect::<String>())
            .unwrap_or_default();
        let mut text = parsed.body_text(0).map(|s| s.into_owned()).unwrap_or_default();
        if text.trim().is_empty() {
            // Fall back to HTML body stripped of tags.
            if let Some(html) = parsed.body_html(0) {
                text = strip_tags(&html);
            }
        }
        let labels = parsed
            .header("X-Gmail-Labels")
            .and_then(|h| h.as_text())
            .unwrap_or("")
            .to_string();
        let id = parsed.message_id().unwrap_or(&format!("mbox-{i}")).to_string();

        let header = format!(
            "[{date}] From: {from}{}\nSubject: {subject}{}\n",
            if to.is_empty() {
                String::new()
            } else {
                format!(" To: {}", to.join(", "))
            },
            if labels.is_empty() {
                String::new()
            } else {
                format!("\nLabels: {labels}")
            },
        );
        let body = format!("{header}\n{text}");
        let title = format!("{subject} — {from}");

        let record = Record {
            source_path: source.clone(),
            record_id: format!("mbox:{id}"),
            title,
            body,
        };
        if persist_record(&record, "mbox", db, embedder)? {
            stats.records_added += 1;
        } else {
            stats.records_skipped += 1;
        }
    }
    Ok(stats)
}

/// Split mbox-O on lines starting with "From " (with trailing space).
/// We keep this simple — the standard `>From ` body-escape preservation
/// is handled by the per-message parser since the leading `>` stays attached.
fn split_mbox(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut started = false;
    for line in s.lines() {
        if line.starts_with("From ") && (line.len() == 5 || line.as_bytes()[5] != b':') {
            if started && !current.trim().is_empty() {
                out.push(std::mem::take(&mut current));
            }
            started = true;
            // Drop the From_ separator itself; keep only RFC 5322 headers/body.
            continue;
        }
        if started {
            current.push_str(line);
            current.push('\n');
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_mbox_extension() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("mail.mbox");
        std::fs::write(&p, b"From a@b.com Mon Jan 1 00:00:00 2024\nFrom: a\nSubject: t\n\nbody")
            .unwrap();
        assert!(detect(&p));
    }

    #[test]
    fn splits_messages_on_from_separator() {
        let mbox = "From a Mon\nSubject: One\n\nbody1\n\nFrom b Tue\nSubject: Two\n\nbody2\n";
        let parts = split_mbox(mbox);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].contains("One"));
        assert!(parts[1].contains("Two"));
    }
}
