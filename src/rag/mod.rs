use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub text: String,
    pub source: String,
    pub score: f32,
    pub chunk_index: usize,
    pub tags: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct SourceInfo {
    pub path: String,
    pub file_type: String,
    pub chunk_count: usize,
    pub ingested_at: String,
}

#[derive(Debug, serde::Serialize)]
pub struct VaultStats {
    pub document_count: usize,
    pub chunk_count: usize,
    pub embedding_dims: usize,
    pub db_size_human: String,
}

const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS documents (
        id          TEXT PRIMARY KEY,
        source_path TEXT NOT NULL,
        file_type   TEXT NOT NULL,
        title       TEXT,
        raw_text    TEXT,
        ingested_at TEXT DEFAULT (datetime('now')),
        sha256      TEXT
    );

    CREATE TABLE IF NOT EXISTS chunks (
        id              TEXT PRIMARY KEY,
        document_id     TEXT NOT NULL REFERENCES documents(id),
        chunk_index     INTEGER NOT NULL,
        text            TEXT NOT NULL,
        token_count     INTEGER,
        char_start      INTEGER,
        char_end        INTEGER,
        embedding       BLOB
    );

    CREATE TABLE IF NOT EXISTS tags (
        document_id TEXT NOT NULL REFERENCES documents(id),
        tag         TEXT NOT NULL,
        PRIMARY KEY (document_id, tag)
    );

    CREATE TABLE IF NOT EXISTS metadata (
        key   TEXT PRIMARY KEY,
        value TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_chunks_doc ON chunks(document_id);
    CREATE INDEX IF NOT EXISTS idx_docs_source ON documents(source_path);
    CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag);
"#;

impl Database {
    /// Open an in-memory database for testing. No files created.
    #[cfg(feature = "test-support")]
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(SCHEMA)?;
        conn.execute(
            "INSERT OR IGNORE INTO metadata (key, value) VALUES ('embedding_dims', '384')",
            [],
        )?;
        Ok(Database {
            conn: Mutex::new(conn),
        })
    }

    pub fn open(vault_path: &Path) -> Result<Self> {
        std::fs::create_dir_all(vault_path)?;
        let db_path = vault_path.join("satchel.db");
        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(SCHEMA)?;
        conn.execute(
            "INSERT OR IGNORE INTO metadata (key, value) VALUES ('embedding_dims', '384')",
            [],
        )?;
        Ok(Database {
            conn: Mutex::new(conn),
        })
    }

    pub fn insert_document(
        &self,
        id: &str,
        source_path: &str,
        file_type: &str,
        title: Option<&str>,
        raw_text: &str,
        sha256: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO documents (id, source_path, file_type, title, raw_text, sha256)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, source_path, file_type, title, raw_text, sha256],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_chunk(
        &self,
        chunk_id: &str,
        document_id: &str,
        chunk_index: usize,
        text: &str,
        token_count: usize,
        char_start: usize,
        char_end: usize,
        embedding: &[f32],
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let embedding_blob = embedding_to_blob(embedding);
        conn.execute(
            "INSERT OR REPLACE INTO chunks (id, document_id, chunk_index, text, token_count, char_start, char_end, embedding)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                chunk_id,
                document_id,
                chunk_index as i64,
                text,
                token_count as i64,
                char_start as i64,
                char_end as i64,
                embedding_blob
            ],
        )?;
        Ok(())
    }

    pub fn search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        filter_source: Option<&str>,
        filter_tags: Option<&[&str]>,
    ) -> Result<Vec<SearchResult>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT c.text, d.source_path, c.chunk_index, c.embedding,
                    COALESCE(GROUP_CONCAT(DISTINCT t.tag), '') AS tags
             FROM chunks c
             JOIN documents d ON d.id = c.document_id
             LEFT JOIN tags t ON t.document_id = d.id
             WHERE c.embedding IS NOT NULL
             GROUP BY c.id",
        )?;

        let mut results: Vec<SearchResult> = stmt
            .query_map([], |row| {
                let embedding_blob: Vec<u8> = row.get(3)?;
                let embedding = blob_to_embedding(&embedding_blob);
                let score = cosine_similarity(query_embedding, &embedding);
                Ok(SearchResult {
                    text: row.get(0)?,
                    source: row.get(1)?,
                    score,
                    chunk_index: row.get::<_, i64>(2)? as usize,
                    tags: row
                        .get::<_, String>(4)?
                        .split(',')
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect(),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        if let Some(source) = filter_source {
            results.retain(|r| r.source.contains(source));
        }
        if let Some(tags) = filter_tags {
            results.retain(|r| tags.iter().any(|t| r.tags.iter().any(|rt| rt == t)));
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(top_k);

        Ok(results)
    }

    pub fn list_sources(
        &self,
        filter_type: Option<&str>,
        sort_by: &str,
    ) -> Result<Vec<SourceInfo>> {
        let conn = self.conn.lock().unwrap();
        let order = match sort_by {
            "date" => "d.ingested_at DESC",
            "chunks" => "chunk_count DESC",
            _ => "d.source_path ASC",
        };

        let sql = format!(
            "SELECT d.source_path, d.file_type, COUNT(c.id) as chunk_count, d.ingested_at
             FROM documents d
             LEFT JOIN chunks c ON c.document_id = d.id
             {} GROUP BY d.id ORDER BY {}",
            if filter_type.is_some() {
                "WHERE d.file_type = ?1"
            } else {
                ""
            },
            order
        );

        let mut stmt = conn.prepare(&sql)?;

        fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SourceInfo> {
            Ok(SourceInfo {
                path: row.get(0)?,
                file_type: row.get(1)?,
                chunk_count: row.get::<_, i64>(2)? as usize,
                ingested_at: row.get(3)?,
            })
        }

        let results: Vec<SourceInfo> = if let Some(ft) = filter_type {
            stmt.query_map(params![ft], map_row)?
                .filter_map(|r| r.ok())
                .collect()
        } else {
            stmt.query_map([], map_row)?
                .filter_map(|r| r.ok())
                .collect()
        };

        Ok(results)
    }

    pub fn get_full_document(&self, source: &str) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        let text: String = conn.query_row(
            "SELECT raw_text FROM documents WHERE source_path = ?1 OR id = ?1",
            params![source],
            |row| row.get(0),
        )?;
        Ok(text)
    }

    pub fn list_tags(&self) -> Result<Vec<(String, usize)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT tag, COUNT(DISTINCT document_id) FROM tags GROUP BY tag ORDER BY tag",
        )?;
        let results = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        Ok(results.filter_map(|r| r.ok()).collect())
    }

    pub fn stats(&self) -> Result<VaultStats> {
        let conn = self.conn.lock().unwrap();
        let doc_count: i64 = conn.query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))?;
        let chunk_count: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
        let dims: String = conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'embedding_dims'",
                [],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "384".to_string());
        let page_count: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
        let page_size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
        let size_bytes = page_count * page_size;

        Ok(VaultStats {
            document_count: doc_count as usize,
            chunk_count: chunk_count as usize,
            embedding_dims: dims.parse().unwrap_or(384),
            db_size_human: humanize_bytes(size_bytes as u64),
        })
    }

    pub fn document_exists_by_hash(&self, sha256: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM documents WHERE sha256 = ?1",
            params![sha256],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn delete_document(&self, source: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let doc_id: String = conn.query_row(
            "SELECT id FROM documents WHERE source_path = ?1 OR id = ?1",
            params![source],
            |row| row.get(0),
        )?;
        conn.execute(
            "DELETE FROM chunks WHERE document_id = ?1",
            params![&doc_id],
        )?;
        conn.execute("DELETE FROM tags WHERE document_id = ?1", params![&doc_id])?;
        conn.execute("DELETE FROM documents WHERE id = ?1", params![&doc_id])?;
        Ok(())
    }

    pub fn add_tag(&self, document_id: &str, tag: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO tags (document_id, tag) VALUES (?1, ?2)",
            params![document_id, tag],
        )?;
        Ok(())
    }
}

pub fn print_stats(db: &Database) -> Result<()> {
    let stats = db.stats()?;
    println!("SATCHEL Vault Statistics");
    println!("========================");
    println!("  Documents:  {}", stats.document_count);
    println!("  Chunks:     {}", stats.chunk_count);
    println!("  Dimensions: {}", stats.embedding_dims);
    println!("  DB Size:    {}", stats.db_size_human);
    Ok(())
}

fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

fn humanize_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    for unit in UNITS {
        if size < 1024.0 {
            return format!("{:.1} {unit}", size);
        }
        size /= 1024.0;
    }
    format!("{:.1} TB", size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (Database, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("satchel-test-{}", uuid::Uuid::new_v4()));
        let db = Database::open(&dir).unwrap();
        (db, dir)
    }

    fn cleanup(dir: &std::path::Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_embedding_blob_roundtrip() {
        let original = vec![0.1, 0.2, -0.3, 1.0, 0.0];
        let blob = embedding_to_blob(&original);
        let restored = blob_to_embedding(&blob);
        assert_eq!(original, restored);
    }

    #[test]
    fn test_embedding_blob_empty() {
        let original: Vec<f32> = vec![];
        let blob = embedding_to_blob(&original);
        let restored = blob_to_embedding(&blob);
        assert!(restored.is_empty());
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_cosine_similarity_mismatched_lengths() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 2.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_humanize_bytes() {
        assert_eq!(humanize_bytes(0), "0.0 B");
        assert_eq!(humanize_bytes(500), "500.0 B");
        assert_eq!(humanize_bytes(1024), "1.0 KB");
        assert_eq!(humanize_bytes(1536), "1.5 KB");
        assert_eq!(humanize_bytes(1048576), "1.0 MB");
        assert_eq!(humanize_bytes(1073741824), "1.0 GB");
    }

    #[test]
    fn test_database_open_and_stats() {
        let (db, dir) = temp_db();
        let stats = db.stats().unwrap();
        assert_eq!(stats.document_count, 0);
        assert_eq!(stats.chunk_count, 0);
        assert_eq!(stats.embedding_dims, 384);
        cleanup(&dir);
    }

    #[test]
    fn test_database_insert_and_retrieve() {
        let (db, dir) = temp_db();

        db.insert_document(
            "doc1",
            "/test/file.md",
            "md",
            Some("Test"),
            "hello world",
            "abc123",
        )
        .unwrap();

        let text = db.get_full_document("/test/file.md").unwrap();
        assert_eq!(text, "hello world");

        let stats = db.stats().unwrap();
        assert_eq!(stats.document_count, 1);

        cleanup(&dir);
    }

    #[test]
    fn test_database_document_exists_by_hash() {
        let (db, dir) = temp_db();

        assert!(!db.document_exists_by_hash("abc123").unwrap());
        db.insert_document("doc1", "/test.md", "md", None, "text", "abc123")
            .unwrap();
        assert!(db.document_exists_by_hash("abc123").unwrap());
        assert!(!db.document_exists_by_hash("xyz789").unwrap());

        cleanup(&dir);
    }

    #[test]
    fn test_database_insert_chunk_and_search() {
        let (db, dir) = temp_db();

        db.insert_document("doc1", "/test.md", "md", None, "text", "hash1")
            .unwrap();

        let embedding = vec![1.0, 0.0, 0.0];
        db.insert_chunk("c1", "doc1", 0, "chunk text", 10, 0, 10, &embedding)
            .unwrap();

        let query = vec![1.0, 0.0, 0.0];
        let results = db.search(&query, 5, None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "chunk text");
        assert!((results[0].score - 1.0).abs() < 1e-6);

        cleanup(&dir);
    }

    #[test]
    fn test_database_delete_document() {
        let (db, dir) = temp_db();

        db.insert_document("doc1", "/test.md", "md", None, "text", "hash1")
            .unwrap();
        db.insert_chunk("c1", "doc1", 0, "chunk", 5, 0, 5, &[0.1, 0.2])
            .unwrap();

        assert_eq!(db.stats().unwrap().document_count, 1);
        db.delete_document("/test.md").unwrap();
        assert_eq!(db.stats().unwrap().document_count, 0);
        assert_eq!(db.stats().unwrap().chunk_count, 0);

        cleanup(&dir);
    }

    #[test]
    fn test_database_list_sources() {
        let (db, dir) = temp_db();

        db.insert_document("d1", "/a.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_document("d2", "/b.txt", "txt", None, "b", "h2")
            .unwrap();

        let all = db.list_sources(None, "name").unwrap();
        assert_eq!(all.len(), 2);

        let md_only = db.list_sources(Some("md"), "name").unwrap();
        assert_eq!(md_only.len(), 1);
        assert_eq!(md_only[0].file_type, "md");

        cleanup(&dir);
    }

    #[test]
    fn test_database_tags() {
        let (db, dir) = temp_db();

        db.insert_document("d1", "/a.md", "md", None, "a", "h1")
            .unwrap();
        db.add_tag("d1", "notes").unwrap();
        db.add_tag("d1", "work").unwrap();

        let tags = db.list_tags().unwrap();
        assert_eq!(tags.len(), 2);

        cleanup(&dir);
    }

    #[test]
    fn test_search_with_source_filter() {
        let (db, dir) = temp_db();

        db.insert_document("d1", "/notes/a.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_document("d2", "/work/b.md", "md", None, "b", "h2")
            .unwrap();

        let emb = vec![1.0, 0.0];
        db.insert_chunk("c1", "d1", 0, "notes chunk", 5, 0, 5, &emb)
            .unwrap();
        db.insert_chunk("c2", "d2", 0, "work chunk", 5, 0, 5, &emb)
            .unwrap();

        let results = db.search(&emb, 10, Some("/notes/"), None).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].source.contains("/notes/"));

        cleanup(&dir);
    }

    #[test]
    fn test_search_empty_database() {
        let db = Database::open_memory().unwrap();
        let query = vec![1.0, 0.0, 0.0];
        let results = db.search(&query, 5, None, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_with_tag_filter() {
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/a.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_document("d2", "/b.md", "md", None, "b", "h2")
            .unwrap();

        let emb = vec![1.0, 0.0];
        db.insert_chunk("c1", "d1", 0, "chunk a", 5, 0, 5, &emb)
            .unwrap();
        db.insert_chunk("c2", "d2", 0, "chunk b", 5, 0, 5, &emb)
            .unwrap();
        db.add_tag("d1", "important").unwrap();

        let results = db.search(&emb, 10, None, Some(&["important"])).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].source.contains("a.md"));
    }

    #[test]
    fn test_get_full_document_by_id() {
        let db = Database::open_memory().unwrap();
        db.insert_document("my-id", "/file.md", "md", None, "content here", "h1")
            .unwrap();
        let text = db.get_full_document("my-id").unwrap();
        assert_eq!(text, "content here");
    }

    #[test]
    fn test_get_full_document_missing() {
        let db = Database::open_memory().unwrap();
        assert!(db.get_full_document("nonexistent").is_err());
    }

    #[test]
    fn test_add_tag_idempotent() {
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/a.md", "md", None, "a", "h1")
            .unwrap();
        db.add_tag("d1", "test").unwrap();
        db.add_tag("d1", "test").unwrap(); // should not error
        let tags = db.list_tags().unwrap();
        assert_eq!(tags.len(), 1);
    }

    #[test]
    fn test_delete_document_cascades_tags() {
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/a.md", "md", None, "a", "h1")
            .unwrap();
        db.add_tag("d1", "tag1").unwrap();
        db.add_tag("d1", "tag2").unwrap();
        assert_eq!(db.list_tags().unwrap().len(), 2);

        db.delete_document("/a.md").unwrap();
        assert_eq!(db.list_tags().unwrap().len(), 0);
    }

    #[test]
    fn test_list_sources_sort_by_date() {
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/old.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_document("d2", "/new.md", "md", None, "b", "h2")
            .unwrap();
        let sources = db.list_sources(None, "date").unwrap();
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn test_list_sources_sort_by_chunks() {
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/few.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_document("d2", "/many.md", "md", None, "b", "h2")
            .unwrap();
        db.insert_chunk("c1", "d2", 0, "x", 5, 0, 1, &[0.1])
            .unwrap();
        db.insert_chunk("c2", "d2", 1, "y", 5, 1, 2, &[0.2])
            .unwrap();

        let sources = db.list_sources(None, "chunks").unwrap();
        assert_eq!(sources[0].path, "/many.md");
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn cosine_self_similarity_is_one(v in proptest::collection::vec(-1.0f32..1.0, 1..100usize)) {
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 1e-10 {
                let sim = cosine_similarity(&v, &v);
                prop_assert!((sim - 1.0).abs() < 1e-4, "Got {sim} for self-similarity");
            }
        }

        #[test]
        fn cosine_similarity_is_symmetric(
            a in proptest::collection::vec(-1.0f32..1.0, 1..50usize),
        ) {
            let b: Vec<f32> = a.iter().rev().cloned().collect();
            let ab = cosine_similarity(&a, &b);
            let ba = cosine_similarity(&b, &a);
            prop_assert!((ab - ba).abs() < 1e-6);
        }

        #[test]
        fn cosine_similarity_in_range(
            a in proptest::collection::vec(-1.0f32..1.0, 1..50usize),
        ) {
            let b: Vec<f32> = a.iter().map(|x| x + 0.1).collect();
            let sim = cosine_similarity(&a, &b);
            prop_assert!(sim >= -1.0 - 1e-5 && sim <= 1.0 + 1e-5, "Got {sim}");
        }

        #[test]
        fn embedding_blob_roundtrip(v in proptest::collection::vec(-1e6f32..1e6, 0..500usize)) {
            let blob = embedding_to_blob(&v);
            let restored = blob_to_embedding(&blob);
            prop_assert_eq!(v, restored);
        }
    }
}
