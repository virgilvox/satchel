//! Lightweight progress reporting for ingest pipelines.
//!
//! `Progress` wraps an optional `Fn(ProgressEvent)` callback. The CLI passes
//! `Progress::noop()`; the HTTP server passes a callback that updates a
//! `JobRegistry` row so the UI can show live counters. Archive handlers and
//! the per-file walker emit events as work happens — no caller is required
//! to do anything but pass the value through.

use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// An archive layout was matched and the specialized handler is starting.
    ArchiveDetected(String),
    /// We're about to process this path (one event per file in walks; one per
    /// daily JSON in Slack; one per email in mbox; etc.). Cosmetic; safe to ignore.
    FileStarted(PathBuf),
    /// A logical record was persisted (a file, a message, a conversation).
    RecordAdded,
    /// Record was a duplicate or content-empty; not persisted.
    RecordSkipped,
    /// A record failed to process. Counters increment but ingest continues.
    RecordFailed,
}

#[derive(Clone)]
pub struct Progress {
    cb: Option<Arc<dyn Fn(ProgressEvent) + Send + Sync>>,
}

impl Progress {
    pub fn noop() -> Self {
        Self { cb: None }
    }

    pub fn callback<F>(f: F) -> Self
    where
        F: Fn(ProgressEvent) + Send + Sync + 'static,
    {
        Self {
            cb: Some(Arc::new(f)),
        }
    }

    pub fn emit(&self, evt: ProgressEvent) {
        if let Some(cb) = &self.cb {
            cb(evt);
        }
    }
}

impl Default for Progress {
    fn default() -> Self {
        Self::noop()
    }
}
