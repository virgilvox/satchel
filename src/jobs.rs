//! In-memory job registry for tracking long-running ingest operations.
//!
//! When the user submits a `POST /api/ingest`, we create a `Job`, spawn the
//! actual work in a background task, and return the `job_id` immediately.
//! The task updates its job's counters via a `Progress` callback as files
//! are processed; the UI polls `GET /api/jobs` to render live status.
//!
//! State lives in the running process — restarts forget all jobs. That's
//! fine: jobs are inherently ephemeral and the underlying SQLite vault is
//! the durable record of what got ingested.

use chrono::Utc;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Serialize)]
pub struct Job {
    pub id: String,
    pub path: String,
    pub status: JobStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub error: Option<String>,
    pub current_file: Option<String>,
    pub files_seen: usize,
    pub records_added: usize,
    pub records_skipped: usize,
    pub records_failed: usize,
    pub archive_kind: Option<String>,
}

impl Job {
    fn new(path: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            path,
            status: JobStatus::Pending,
            started_at: Utc::now().to_rfc3339(),
            finished_at: None,
            error: None,
            current_file: None,
            files_seen: 0,
            records_added: 0,
            records_skipped: 0,
            records_failed: 0,
            archive_kind: None,
        }
    }
}

pub struct JobRegistry {
    jobs: Mutex<HashMap<String, Job>>,
    /// Insertion order; newest first when read in reverse.
    order: Mutex<Vec<String>>,
    /// Bound on retained finished jobs. Older completed/failed jobs roll off
    /// when this is exceeded so a long-running server doesn't grow unbounded.
    retain: usize,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
            retain: 100,
        }
    }

    /// Create a new job in `Pending` state and return its id.
    pub fn create(&self, path: String) -> String {
        let job = Job::new(path);
        let id = job.id.clone();
        {
            let mut jobs = self.jobs.lock().unwrap();
            let mut order = self.order.lock().unwrap();
            jobs.insert(id.clone(), job);
            order.push(id.clone());
        }
        self.evict_old_finished();
        id
    }

    pub fn update<F: FnOnce(&mut Job)>(&self, id: &str, f: F) {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.get_mut(id) {
            f(job);
        }
    }

    pub fn list(&self) -> Vec<Job> {
        let jobs = self.jobs.lock().unwrap();
        let order = self.order.lock().unwrap();
        // Newest first.
        order
            .iter()
            .rev()
            .filter_map(|id| jobs.get(id).cloned())
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<Job> {
        self.jobs.lock().unwrap().get(id).cloned()
    }

    /// Drop oldest finished jobs once we exceed the retention bound. Active
    /// jobs (Pending/Running) are never evicted.
    fn evict_old_finished(&self) {
        let mut jobs = self.jobs.lock().unwrap();
        let mut order = self.order.lock().unwrap();
        if order.len() <= self.retain {
            return;
        }
        let mut to_remove: Vec<String> = Vec::new();
        for id in order.iter() {
            if to_remove.len() + self.retain >= order.len() {
                break;
            }
            if let Some(j) = jobs.get(id) {
                if matches!(j.status, JobStatus::Completed | JobStatus::Failed) {
                    to_remove.push(id.clone());
                }
            }
        }
        for id in &to_remove {
            jobs.remove(id);
        }
        order.retain(|id| !to_remove.contains(id));
    }
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_returns_distinct_ids() {
        let r = JobRegistry::new();
        let a = r.create("/a".into());
        let b = r.create("/b".into());
        assert_ne!(a, b);
    }

    #[test]
    fn list_orders_newest_first() {
        let r = JobRegistry::new();
        let a = r.create("/a".into());
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = r.create("/b".into());
        let listed = r.list();
        assert_eq!(listed[0].id, b);
        assert_eq!(listed[1].id, a);
    }

    #[test]
    fn update_mutates_in_place() {
        let r = JobRegistry::new();
        let id = r.create("/a".into());
        r.update(&id, |j| {
            j.records_added = 42;
            j.status = JobStatus::Running;
        });
        let j = r.get(&id).unwrap();
        assert_eq!(j.records_added, 42);
        assert_eq!(j.status, JobStatus::Running);
    }

    #[test]
    fn update_missing_id_is_noop() {
        let r = JobRegistry::new();
        r.update("nonexistent", |j| j.records_added = 1);
        assert!(r.get("nonexistent").is_none());
    }
}
