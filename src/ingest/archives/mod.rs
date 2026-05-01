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

/// Dispatch to the appropriate handler.
pub fn ingest(
    kind: ArchiveKind,
    path: &Path,
    db: &Database,
    embedder: &Embedder,
    progress: &Progress,
) -> Result<ArchiveStats> {
    match kind {
        ArchiveKind::Slack => slack::ingest(path, db, embedder, progress),
        ArchiveKind::ChatGpt => chatgpt::ingest(path, db, embedder, progress),
        ArchiveKind::ClaudeAi => claude_ai::ingest(path, db, embedder, progress),
        ArchiveKind::Discord => discord::ingest(path, db, embedder, progress),
        ArchiveKind::WhatsApp => whatsapp::ingest(path, db, embedder, progress),
        ArchiveKind::Mbox => mbox::ingest(path, db, embedder, progress),
    }
}

/// Persist a single record: insert document, embed body, insert single chunk.
/// Skips if a document with the same source_path already exists with same
/// record_id-derived hash. Emits a `RecordAdded` or `RecordSkipped` event.
pub(crate) fn persist_record(
    record: &Record,
    file_type: &str,
    db: &Database,
    embedder: &Embedder,
    progress: &Progress,
) -> Result<bool> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(record.record_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(record.body.as_bytes());
    let sha256 = format!("{:x}", hasher.finalize());

    if db.document_exists_by_hash(&sha256)? {
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

    let embedding = embedder.embed_with_info(&record.body)?;
    db.insert_chunk(
        &format!("{doc_id}:0"),
        &doc_id,
        0,
        &record.body,
        embedding.token_count,
        0,
        record.body.len(),
        &embedding.vector,
    )?;
    progress.emit(ProgressEvent::RecordAdded);
    Ok(true)
}
