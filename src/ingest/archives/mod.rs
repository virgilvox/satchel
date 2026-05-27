//! Format-aware ingest for structured personal-data archives.
//!
//! Each handler detects its archive layout (e.g. Slack export = `users.json` +
//! `channels.json` + per-channel daily JSONs at the root) and emits one
//! [`Record`] per logical item — a message, conversation turn, email, etc.
//!
//! A `Record` becomes one row in `documents` and one chunk per record (no
//! sub-chunking). The chunk text starts with a normalized header line so
//! BM25 can rank against names, dates, and channels — see [`Record::format`].
//!
//! Detection is intentionally cheap and conservative: a handler returns true
//! only if the directory layout matches its archive's distinctive signature.
//! The dispatcher tries each in declared order; first match wins.

use anyhow::Result;
use std::path::Path;

use crate::embed::Embedder;
use crate::ingest::progress::{Progress, ProgressEvent};
use crate::rag::Database;

pub mod chatgpt;
pub mod claude_ai;
pub mod csv;
pub mod discord;
pub mod mbox;
pub mod slack;
pub mod whatsapp;

/// What kind of archive a directory contains, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Slack,
    ChatGpt,
    ClaudeAi,
    Discord,
    WhatsApp,
    Mbox,
    Csv,
}

impl ArchiveKind {
    pub fn name(self) -> &'static str {
        match self {
            ArchiveKind::Slack => "slack",
            ArchiveKind::ChatGpt => "chatgpt",
            ArchiveKind::ClaudeAi => "claude.ai",
            ArchiveKind::Discord => "discord",
            ArchiveKind::WhatsApp => "whatsapp",
            ArchiveKind::Mbox => "mbox",
            ArchiveKind::Csv => "csv",
        }
    }
}

/// A normalized record ready to be chunked + embedded.
pub struct Record {
    /// The source file path (used for `documents.source_path`).
    pub source_path: String,
    /// Stable per-record key (used for `documents.id` and dedup hash basis).
    pub record_id: String,
    /// One-line title for documents.title (e.g., "@alice in #general 2024-08-12").
    pub title: String,
    /// Full chunk text. Should already contain a normalized header line so
    /// keyword search can match dates, names, and channels.
    pub body: String,
}

/// Detect which archive (if any) a directory contains.
///
/// Returns `None` for plain directories — the caller should fall back to
/// per-file text extraction. Order matters: more-specific signatures are
/// checked first.
pub fn detect(path: &Path) -> Option<ArchiveKind> {
    if !path.is_dir() {
        // Single-file archives (mbox, ChatGPT zip post-extract is a dir, but
        // a raw .mbox file is one file).
        if mbox::detect(path) {
            return Some(ArchiveKind::Mbox);
        }
        if whatsapp::detect(path) {
            return Some(ArchiveKind::WhatsApp);
        }
        if discord::detect(path) {
            return Some(ArchiveKind::Discord);
        }
        if csv::detect(path) {
            return Some(ArchiveKind::Csv);
        }
        return None;
    }
    if slack::detect(path) {
        return Some(ArchiveKind::Slack);
    }
    if chatgpt::detect(path) {
        return Some(ArchiveKind::ChatGpt);
    }
    if claude_ai::detect(path) {
        return Some(ArchiveKind::ClaudeAi);
    }
    if whatsapp::detect(path) {
        return Some(ArchiveKind::WhatsApp);
    }
    if mbox::detect(path) {
        return Some(ArchiveKind::Mbox);
    }
    None
}

/// Stats reported by an archive handler back to the user.
#[derive(Debug, Default)]
pub struct ArchiveStats {
    pub records_added: usize,
    pub records_skipped: usize,
    pub files_failed: usize,
}

/// Dispatch to the appropriate handler. `collection_id`, when set, is
/// forwarded into every `persist_record` call so each emitted document
/// joins the named collection at insert time. This is what makes
/// "ingest a Slack export into the Work collection" a one-step
/// operation instead of an ingest-then-assign two-step.
pub fn ingest(
    kind: ArchiveKind,
    path: &Path,
    db: &Database,
    embedder: &Embedder,
    progress: &Progress,
    collection_id: Option<i64>,
) -> Result<ArchiveStats> {
    match kind {
        ArchiveKind::Slack => slack::ingest(path, db, embedder, progress, collection_id),
        ArchiveKind::ChatGpt => chatgpt::ingest(path, db, embedder, progress, collection_id),
        ArchiveKind::ClaudeAi => claude_ai::ingest(path, db, embedder, progress, collection_id),
        ArchiveKind::Discord => discord::ingest(path, db, embedder, progress, collection_id),
        ArchiveKind::WhatsApp => whatsapp::ingest(path, db, embedder, progress, collection_id),
        ArchiveKind::Mbox => mbox::ingest(path, db, embedder, progress, collection_id),
        ArchiveKind::Csv => csv::ingest(path, db, embedder, progress, collection_id),
    }
}

/// Soft cap for the per-chunk text size when sub-chunking oversize
/// archive records. The default embedder (`bge-small-en-v1.5`) has a
/// 512-token context; we leave headroom for the repeated header line
/// and a few special tokens. Anything larger gets silently truncated
/// by the embedder, which is exactly the bug v2.5.1 fixes.
const ARCHIVE_CHUNK_MAX_TOKENS: usize = 480;

/// Persist a single record. The full body is stored as one row in
/// `documents`; the body may be split into multiple `chunks` rows when
/// it exceeds the embedder's effective context. Each sub-chunk
/// preserves the record's normalized header line so search hits in
/// the middle of a long thread / email / conversation still surface
/// who/when/where without a second query.
///
/// Skips entirely if a document with the same hash already exists
/// (idempotent on re-ingest). Emits a `RecordAdded` or `RecordSkipped`
/// event.
pub(crate) fn persist_record(
    record: &Record,
    file_type: &str,
    db: &Database,
    embedder: &Embedder,
    progress: &Progress,
    collection_id: Option<i64>,
) -> Result<bool> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(record.record_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(record.body.as_bytes());
    let sha256 = format!("{:x}", hasher.finalize());

    // Dedup-skip path: the content is already in the vault. We still
    // honor the caller's collection_id so a "re-ingest this Slack
    // export into the new Work collection" run actually populates Work
    // with the existing records, instead of silently doing nothing.
    if db.document_exists_by_hash(&sha256)? {
        if let Some(cid) = collection_id {
            if let Some(existing_id) = db.document_id_by_hash(&sha256)? {
                db.collection_add_documents(cid, std::slice::from_ref(&existing_id))?;
            }
        }
        progress.emit(ProgressEvent::RecordSkipped);
        return Ok(false);
    }

    let doc_id = uuid::Uuid::new_v4().to_string();
    db.insert_document(
        &doc_id,
        &record.source_path,
        file_type,
        Some(&record.title),
        &record.body,
        &sha256,
    )?;

    if let Some(cid) = collection_id {
        db.collection_add_documents(cid, std::slice::from_ref(&doc_id))?;
    }

    // Sub-chunk if the body is too long for the embedder's context.
    // Short bodies still produce exactly one chunk (current behavior
    // preserved for everything that already fits).
    let chunks = chunk_archive_body(&record.body, ARCHIVE_CHUNK_MAX_TOKENS);
    for (i, chunk_text) in chunks.iter().enumerate() {
        let embedding = embedder.embed_with_info(chunk_text)?;
        db.insert_chunk(
            &format!("{doc_id}:{i}"),
            &doc_id,
            i,
            chunk_text,
            embedding.token_count,
            0,
            chunk_text.len(),
            &embedding.vector,
        )?;
    }
    progress.emit(ProgressEvent::RecordAdded);
    Ok(true)
}

/// Split an archive record's body into one or more chunks each under
/// `max_tokens`. The first non-empty line ending in `\n` is treated
/// as the record's header (e.g. `csv: file.csv row 7`,
/// `slack: @alice in #design 2026-04-12`) and is repeated at the top
/// of every emitted chunk so each chunk is self-describing.
///
/// Splitting strategy: paragraph (`\n\n`) → line (`\n`) → hard char
/// wrap. Always returns at least one chunk; for bodies under the
/// threshold the result is `vec![body.to_string()]` (no header
/// repetition needed since there's only one piece).
pub(crate) fn chunk_archive_body(body: &str, max_tokens: usize) -> Vec<String> {
    if body.is_empty() {
        return vec![String::new()];
    }
    if crate::ingest::approximate_tokens(body) <= max_tokens {
        return vec![body.to_string()];
    }

    // Identify the header line: first '\n'. Everything before (and
    // including) the newline is the header; everything after is body
    // content. If the body has no newline at all, treat all of it as
    // content with no header.
    let (header, content) = match body.find('\n') {
        Some(i) => (&body[..=i], &body[i + 1..]),
        None => ("", body),
    };

    let header_tokens = crate::ingest::approximate_tokens(header);
    let slice_budget = max_tokens.saturating_sub(header_tokens).max(1);

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    let push_current = |chunks: &mut Vec<String>, current: &mut String| {
        if !current.is_empty() {
            let mut chunk = String::with_capacity(header.len() + current.len());
            chunk.push_str(header);
            chunk.push_str(current.trim_end());
            chunks.push(chunk);
            current.clear();
        }
    };

    for paragraph in content.split("\n\n") {
        if paragraph.trim().is_empty() {
            continue;
        }
        let p_tokens = crate::ingest::approximate_tokens(paragraph);
        let cur_tokens = crate::ingest::approximate_tokens(&current);

        if cur_tokens + p_tokens + 1 > slice_budget {
            push_current(&mut chunks, &mut current);
            if p_tokens > slice_budget {
                // The single paragraph itself overflows; split it
                // line-by-line, falling back to char-wrap inside lines.
                for sub in split_oversized(paragraph, slice_budget) {
                    let mut chunk = String::with_capacity(header.len() + sub.len());
                    chunk.push_str(header);
                    chunk.push_str(&sub);
                    chunks.push(chunk);
                }
                continue;
            }
        }

        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(paragraph);
    }
    push_current(&mut chunks, &mut current);

    if chunks.is_empty() {
        chunks.push(body.to_string());
    }
    chunks
}

fn split_oversized(text: &str, budget_tokens: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        let cur_tokens = crate::ingest::approximate_tokens(&current);
        let line_tokens = crate::ingest::approximate_tokens(line);

        if cur_tokens + line_tokens + 1 > budget_tokens {
            if !current.is_empty() {
                out.push(current.trim_end().to_string());
                current.clear();
            }
            if line_tokens > budget_tokens {
                // Hard char-wrap; respect UTF-8 boundaries so we
                // never produce invalid strings.
                let chars_per_chunk = (budget_tokens.saturating_mul(4)).max(1);
                let mut start = 0;
                while start < line.len() {
                    let target = (start + chars_per_chunk).min(line.len());
                    let safe_end = (start..=target)
                        .rev()
                        .find(|&i| line.is_char_boundary(i))
                        .unwrap_or(target);
                    if safe_end <= start {
                        break;
                    }
                    out.push(line[start..safe_end].to_string());
                    start = safe_end;
                }
                continue;
            }
        }

        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        out.push(current.trim_end().to_string());
    }
    if out.is_empty() {
        out.push(text.to_string());
    }
    out
}

#[cfg(test)]
mod chunking_tests {
    use super::*;

    #[test]
    fn under_threshold_returns_single_chunk_unchanged() {
        let body = "csv: file.csv row 1\n\nname: Alice\nemail: a@b.com";
        let chunks = chunk_archive_body(body, 480);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], body);
    }

    #[test]
    fn empty_body_returns_one_empty_chunk() {
        let chunks = chunk_archive_body("", 480);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "");
    }

    #[test]
    fn oversize_body_splits_on_paragraph_boundaries_with_header_repeated() {
        let header = "slack: @alice in #design 2026-04-12\n";
        // ~100 tokens per paragraph (400 chars / 4), 4 paragraphs.
        // With max_tokens=120 we should get multiple chunks.
        let para = "x".repeat(400);
        let body = format!(
            "{header}\n{para}\n\n{para}\n\n{para}\n\n{para}",
            header = header,
            para = para
        );
        let chunks = chunk_archive_body(&body, 120);
        let n = chunks.len();
        assert!(n > 1, "expected multiple chunks, got {n}");
        for c in &chunks {
            assert!(
                c.starts_with("slack: @alice in #design 2026-04-12\n"),
                "every sub-chunk must start with the header line; got:\n{c}"
            );
        }
    }

    #[test]
    fn body_with_no_newline_chunks_anyway() {
        // Long single line, no header. Hard char-wrap fallback.
        let body = "a".repeat(5000);
        let chunks = chunk_archive_body(&body, 50);
        assert!(chunks.len() > 1);
        // No header to repeat (body has no '\n'); each chunk is content only.
        let total_chars: usize = chunks.iter().map(|c| c.len()).sum();
        let body_len = body.len();
        assert!(
            total_chars >= body_len - 1,
            "char count drift: {total_chars} vs {body_len}"
        );
    }

    #[test]
    fn header_token_budget_respected() {
        // Big header + medium body. Each chunk's slice budget should
        // be (max - header_tokens), so even a body that fits in
        // `max_tokens` raw could need splitting after header overhead
        // is accounted for.
        let header = format!("{}\n", "h".repeat(800)); // ~200 tokens of header
        let body = format!("{}{}", header, "x".repeat(2000));
        let chunks = chunk_archive_body(&body, 250);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.starts_with(&header));
        }
    }

    #[test]
    fn splits_at_message_boundaries_in_chat_data() {
        // Slack-style body with multiple messages separated by blanks.
        let body = "\
slack: #general 2026-04-12\n\n\
[12:01] alice: hey have we shipped the v2 api yet\n\n\
[12:03] bob: nope, still blocked on auth\n\n\
[12:04] alice: lol perfect timing then\n\n\
[12:05] bob: you have no idea\n\n\
[12:10] alice: ok i'll move it to next sprint\n\n\
[12:11] bob: appreciate it";
        // Force splitting with a small budget.
        let chunks = chunk_archive_body(body, 30);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.starts_with("slack: #general 2026-04-12\n"));
        }
    }
}
