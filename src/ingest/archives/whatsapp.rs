//! WhatsApp chat export (`_chat.txt` / `WhatsApp Chat with *.txt`).
//!
//! Date format is locale-dependent (12h/24h, M/D vs D/M). We auto-detect by
//! parsing the first ~50 messages with each candidate format and picking the
//! one that yields strictly monotonic timestamps.

use anyhow::Result;
use chrono::NaiveDateTime;
use std::path::Path;

use super::{persist_record, ArchiveStats, Record};
use crate::embed::Embedder;
use crate::rag::Database;

pub fn detect(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !(name == "_chat.txt"
        || (name.starts_with("whatsapp chat with ") && name.ends_with(".txt")))
    {
        return false;
    }
    // Sniff the first non-blank line for the WhatsApp date prefix shape.
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let head = std::str::from_utf8(&bytes[..bytes.len().min(2048)]).unwrap_or("");
    head.lines().any(|l| {
        let l = l.trim_start_matches('\u{feff}').trim_start_matches('\u{200e}');
        // iOS:    [31/12/22, 15:02:13] Bob: ...
        // Android: 31/12/2022, 15:02 - Bob: ...
        l.starts_with('[') || l.split_once(" - ").is_some()
    })
}

const FMTS: &[&str] = &[
    // Android (no brackets, " - " separator)
    "%d/%m/%Y, %H:%M",
    "%m/%d/%Y, %H:%M",
    "%d/%m/%y, %H:%M",
    "%m/%d/%y, %H:%M",
    // iOS (square brackets)
    "%d/%m/%y, %H:%M:%S",
    "%m/%d/%y, %H:%M:%S",
    "%d/%m/%Y, %H:%M:%S",
    "%m/%d/%Y, %H:%M:%S",
];

pub fn ingest(path: &Path, db: &Database, embedder: &Embedder) -> Result<ArchiveStats> {
    let bytes = std::fs::read(path)?;
    // Strip BOM and bidi marks.
    let text = std::str::from_utf8(&bytes)?.trim_start_matches('\u{feff}');

    let chat_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("whatsapp")
        .trim_start_matches("WhatsApp Chat with ")
        .to_string();

    let fmt = pick_date_format(text).unwrap_or("%d/%m/%y, %H:%M");
    let messages = parse_messages(text, fmt);
    eprintln!("[satchel] WhatsApp '{chat_name}': {} messages", messages.len());

    let source = path.to_string_lossy().to_string();
    let mut stats = ArchiveStats::default();
    for (i, m) in messages.iter().enumerate() {
        if m.is_system {
            continue;
        }
        let date = m.timestamp.format("%Y-%m-%d %H:%M").to_string();
        let body = format!(
            "[{date} chat:{chat_name} @{}]: {}",
            m.sender.replace(' ', "_"),
            m.body
        );
        let record = Record {
            source_path: source.clone(),
            record_id: format!(
                "whatsapp:{chat_name}:{i}:{}",
                m.timestamp.and_utc().timestamp()
            ),
            title: format!("@{} in {chat_name} on {date}", m.sender),
            body,
        };
        if persist_record(&record, "whatsapp", db, embedder)? {
            stats.records_added += 1;
        } else {
            stats.records_skipped += 1;
        }
    }
    Ok(stats)
}

struct Msg {
    timestamp: NaiveDateTime,
    sender: String,
    body: String,
    is_system: bool,
}

fn pick_date_format(text: &str) -> Option<&'static str> {
    let mut best: Option<(&'static str, usize)> = None;
    for fmt in FMTS {
        let mut count = 0usize;
        let mut ok = true;
        let mut last: Option<NaiveDateTime> = None;
        for line in text.lines().take(200) {
            if let Some(ts) = parse_line_ts(line, fmt) {
                count += 1;
                if let Some(prev) = last {
                    if ts < prev {
                        ok = false;
                        break;
                    }
                }
                last = Some(ts);
                if count >= 50 {
                    break;
                }
            }
        }
        if ok && count >= 2 {
            match best {
                Some((_, c)) if c >= count => {}
                _ => best = Some((fmt, count)),
            }
        }
    }
    best.map(|(f, _)| f)
}

fn parse_line_ts(line: &str, fmt: &str) -> Option<NaiveDateTime> {
    let line = line.trim_start_matches('\u{200e}');
    if let Some(stripped) = line.strip_prefix('[') {
        if let Some(end) = stripped.find(']') {
            let date_part = &stripped[..end];
            return NaiveDateTime::parse_from_str(date_part, fmt).ok();
        }
    }
    if let Some((date_part, _rest)) = line.split_once(" - ") {
        return NaiveDateTime::parse_from_str(date_part, fmt).ok();
    }
    None
}

fn parse_messages(text: &str, fmt: &str) -> Vec<Msg> {
    let mut out: Vec<Msg> = Vec::new();
    for line in text.lines() {
        let stripped = line.trim_start_matches('\u{200e}');
        // iOS: [date] sender: body
        if let Some(rest) = stripped.strip_prefix('[') {
            if let Some(end) = rest.find(']') {
                let date_str = &rest[..end];
                if let Ok(ts) = NaiveDateTime::parse_from_str(date_str, fmt) {
                    let after = rest[end + 1..].trim_start();
                    let (sender, body, is_system) = split_sender(after);
                    out.push(Msg {
                        timestamp: ts,
                        sender,
                        body,
                        is_system,
                    });
                    continue;
                }
            }
        }
        // Android: date - sender: body
        if let Some((date_part, after)) = stripped.split_once(" - ") {
            if let Ok(ts) = NaiveDateTime::parse_from_str(date_part, fmt) {
                let (sender, body, is_system) = split_sender(after);
                out.push(Msg {
                    timestamp: ts,
                    sender,
                    body,
                    is_system,
                });
                continue;
            }
        }
        // Continuation of previous message.
        if let Some(prev) = out.last_mut() {
            prev.body.push('\n');
            prev.body.push_str(line);
        }
    }
    out
}

fn split_sender(text: &str) -> (String, String, bool) {
    if let Some((sender, body)) = text.split_once(": ") {
        // System messages have no sender; they look like "Messages and calls are
        // end-to-end encrypted." Heuristic: if sender contains spaces or special
        // chars beyond what a name would, but the most reliable signal is the
        // absence of the colon-space separator entirely — handled below.
        (sender.to_string(), body.to_string(), false)
    } else {
        (String::new(), text.to_string(), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_only_whatsapp_filenames() {
        let dir = tempfile::tempdir().unwrap();
        let chat_path = dir.path().join("_chat.txt");
        std::fs::write(&chat_path, b"[31/12/22, 15:02:13] Bob: hi\n").unwrap();
        assert!(detect(&chat_path));

        let other = dir.path().join("notes.txt");
        std::fs::write(&other, b"hello").unwrap();
        assert!(!detect(&other));
    }

    #[test]
    fn parses_ios_format() {
        let text = "[31/12/22, 15:02:13] Bob: Hello world\n[31/12/22, 15:03:00] Alice: Hi";
        let msgs = parse_messages(text, "%d/%m/%y, %H:%M:%S");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].sender, "Bob");
        assert_eq!(msgs[0].body, "Hello world");
        assert_eq!(msgs[1].sender, "Alice");
    }

    #[test]
    fn parses_android_format() {
        let text = "31/12/2022, 15:02 - Bob: Hello\n31/12/2022, 15:03 - Alice: Hi";
        let msgs = parse_messages(text, "%d/%m/%Y, %H:%M");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].sender, "Bob");
    }

    #[test]
    fn picks_format_by_monotonic_check() {
        // 13/01 has no valid %m/%d interpretation (no 13th month); the picker
        // must reject %m/%d formats and accept some %d/%m form.
        let text = "13/01/22, 10:00 - Bob: a\n14/01/22, 10:00 - Bob: b\n";
        let fmt = pick_date_format(text).expect("should pick a format");
        assert!(
            fmt.starts_with("%d/%m"),
            "should choose day-first format, got: {fmt}"
        );
        let msgs = parse_messages(text, fmt);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].sender, "Bob");
    }
}
