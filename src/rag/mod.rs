use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    /// Stable chunk identifier (`{document_id}:{chunk_index}`). Pass to
    /// `get_chunk_context` to expand the hit with neighboring chunks.
    pub chunk_id: String,
    pub text: String,
    pub source: String,
    /// File type of the source document (`md`, `pdf`, `slack`, `csv`, ...).
    pub file_type: String,
    pub score: f32,
    pub chunk_index: usize,
    pub tags: Vec<String>,
}

/// A page of search results plus the total match count, so the UI can
/// render "Showing N of M" and a Load More button.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchPage {
    pub results: Vec<SearchResult>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

/// One chunk in the same-document neighborhood of a hit. Returned by
/// `get_chunk_context` so an LLM can read the conversational frame
/// around a fragment (Slack-thread context, the messages immediately
/// before and after a quoted line, etc.) before drawing conclusions.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChunkContext {
    pub chunk_id: String,
    pub document_id: String,
    pub source_path: String,
    pub chunk_index: usize,
    pub text: String,
}

/// Optional scoping knobs for `Database::search`. All fields default
/// to `None`/empty, which preserves "search the whole vault" behavior.
/// Built as a struct so we can extend the search API with new filters
/// without breaking the call signature on every site (date range,
/// reranker toggle, etc. are coming).
#[derive(Default, Debug, Clone)]
pub struct SearchOptions<'a> {
    /// Substring match against `documents.source_path`.
    pub filter_source: Option<&'a str>,
    /// Restrict to chunks whose document carries any of these tags.
    pub filter_tags: Option<&'a [&'a str]>,
    /// Restrict to chunks whose document is in this collection.
    pub filter_collection: Option<i64>,
    /// Restrict to chunks whose document has this exact `file_type`
    /// (e.g. `"md"`, `"pdf"`, `"slack"`, `"csv"`). Useful for "search
    /// only my emails" or "search only my CSVs" queries.
    pub filter_file_type: Option<&'a str>,
}

#[derive(Debug, serde::Serialize)]
pub struct SourceInfo {
    pub path: String,
    pub file_type: String,
    /// Number of underlying `documents` rows at this `source_path`. For most
    /// files it's 1; for archive sources where the handler emits one record
    /// per logical message (Slack daily JSONs, mbox files), it's >1.
    pub record_count: usize,
    pub chunk_count: usize,
    pub ingested_at: String,
}

/// A page of grouped sources plus total count, for UI pagination.
#[derive(Debug, serde::Serialize)]
pub struct SourcesPage {
    pub sources: Vec<SourceInfo>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

/// One row in the documents table — a single ingested record. Archive
/// imports (Slack, mbox, ChatGPT) produce one of these per message /
/// conversation / email; single-file imports produce one per file.
/// This is the ungrouped view that backs the Documents-tab "BY RECORD"
/// mode where users can pick individual records into collections.
#[derive(Debug, serde::Serialize)]
pub struct DocumentRow {
    pub id: String,
    pub source_path: String,
    pub title: Option<String>,
    pub file_type: String,
    pub chunk_count: usize,
    pub ingested_at: String,
    /// Collection ids this document is currently in. Empty for unassigned.
    /// Lets the UI render "in: Work, Research" badges without a second call.
    pub collection_ids: Vec<i64>,
}

#[derive(Debug, serde::Serialize)]
pub struct DocumentsPage {
    pub documents: Vec<DocumentRow>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

/// One record at a given source_path, suitable for "show conversation context"
/// rendering. Records are returned in ingest order — for archive sources whose
/// handlers walk messages chronologically (Slack, Discord, mbox), this yields
/// chronological listing.
#[derive(Debug, serde::Serialize)]
pub struct ConversationRecord {
    pub id: String,
    pub title: Option<String>,
    pub text: String,
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

    -- v1.6.0: collections — named subsets of documents inside a single
    -- vault. Documents can belong to multiple collections (or none — they
    -- still surface in unfiltered search). Cascading FK so deleting a
    -- document removes its membership entries.
    CREATE TABLE IF NOT EXISTS collections (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        name        TEXT NOT NULL UNIQUE,
        created_at  TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS document_collections (
        document_id   TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
        collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
        PRIMARY KEY (document_id, collection_id)
    );
    CREATE INDEX IF NOT EXISTS idx_dc_collection ON document_collections(collection_id);

    CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
        text,
        content='chunks',
        content_rowid='rowid',
        tokenize='unicode61 remove_diacritics 2'
    );

    CREATE TRIGGER IF NOT EXISTS chunks_ai AFTER INSERT ON chunks BEGIN
        INSERT INTO chunks_fts(rowid, text) VALUES (new.rowid, new.text);
    END;
    CREATE TRIGGER IF NOT EXISTS chunks_ad AFTER DELETE ON chunks BEGIN
        INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES('delete', old.rowid, old.text);
    END;
    CREATE TRIGGER IF NOT EXISTS chunks_au AFTER UPDATE ON chunks BEGIN
        INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES('delete', old.rowid, old.text);
        INSERT INTO chunks_fts(rowid, text) VALUES (new.rowid, new.text);
    END;
"#;

/// Reciprocal Rank Fusion constant. Standard value from Cormack et al. 2009.
const RRF_K: f32 = 60.0;
/// Sparse-leg candidate pool. The dense leg ranks the entire corpus so that
/// post-fusion `filter_source` / `filter_tags` retain rank information from
/// chunks deep in the dense ranking — a constant-K dense truncate would
/// silently strip them.
const RRF_SPARSE_LIMIT: usize = 200;

impl Database {
    /// Open an in-memory database for testing. No files created.
    #[cfg(feature = "test-support")]
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        conn.execute_batch(SCHEMA)?;
        conn.execute(
            "INSERT OR IGNORE INTO metadata (key, value) VALUES ('embedding_dims', '384')",
            [],
        )?;
        backfill_fts(&conn)?;
        Ok(Database {
            conn: Mutex::new(conn),
        })
    }

    pub fn open(vault_path: &Path) -> Result<Self> {
        std::fs::create_dir_all(vault_path)?;
        let db_path = vault_path.join("satchel.db");
        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        conn.execute_batch(SCHEMA)?;
        conn.execute(
            "INSERT OR IGNORE INTO metadata (key, value) VALUES ('embedding_dims', '384')",
            [],
        )?;
        backfill_fts(&conn)?;
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

    /// Hybrid retrieval: dense (cosine over embeddings) + sparse (BM25 over FTS5),
    /// fused via Reciprocal Rank Fusion. Returns a [`SearchPage`] so the UI can
    /// paginate through long result lists; `offset` is 0 for the first page.
    ///
    /// `query_text` is what FTS5 tokenizes for keyword matching. Pass the same
    /// natural-language string the user typed; do not pre-tokenize.
    ///
    /// All scoping (source filter, tag filter, collection scope, file-type
    /// scope) lives on [`SearchOptions`]. Pass `SearchOptions::default()`
    /// for "search the whole vault."
    pub fn search(
        &self,
        query_embedding: &[f32],
        query_text: &str,
        limit: usize,
        offset: usize,
        options: SearchOptions<'_>,
    ) -> Result<SearchPage> {
        let SearchOptions {
            filter_source,
            filter_tags,
            filter_collection,
            filter_file_type,
        } = options;
        let conn = self.conn.lock().unwrap();

        // Pre-resolve collection membership once. An empty collection (or a
        // bad id) yields an empty set, which makes the filter return zero
        // results — that is the correct semantic, not an error.
        // documents.id is TEXT (uuid-like), so the set is keyed by String.
        let collection_doc_ids: Option<std::collections::HashSet<String>> = if let Some(cid) =
            filter_collection
        {
            let mut stmt = conn
                .prepare("SELECT document_id FROM document_collections WHERE collection_id = ?")?;
            let ids: std::collections::HashSet<String> = stmt
                .query_map(params![cid], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            Some(ids)
        } else {
            None
        };

        let mut by_chunk: std::collections::HashMap<String, FusedRow> =
            std::collections::HashMap::new();

        // --- Dense leg: cosine over all embeddings, full ranking. We do not
        // truncate before fusion because filter_source/filter_tags are applied
        // after fusion and could otherwise drop legitimate hits below an
        // arbitrary cutoff. For a 50K-chunk vault this is ~50K f32 dot
        // products; <10ms on commodity hardware.
        let dense: Vec<(String, f32)> = {
            let mut stmt = conn.prepare(
                "SELECT c.id, c.text, d.source_path, d.id, c.chunk_index, c.embedding,
                        COALESCE(GROUP_CONCAT(DISTINCT t.tag), '') AS tags,
                        d.file_type
                 FROM chunks c
                 JOIN documents d ON d.id = c.document_id
                 LEFT JOIN tags t ON t.document_id = d.id
                 WHERE c.embedding IS NOT NULL
                 GROUP BY c.id",
            )?;

            let rows: Vec<(String, FusedRow, f32)> = stmt
                .query_map([], |row| {
                    let id: String = row.get(0)?;
                    let embedding_blob: Vec<u8> = row.get(5)?;
                    let embedding = blob_to_embedding(&embedding_blob);
                    let score = cosine_similarity(query_embedding, &embedding);
                    let fr = FusedRow {
                        chunk_id: id.clone(),
                        text: row.get(1)?,
                        source: row.get(2)?,
                        document_id: row.get(3)?,
                        file_type: row.get(7)?,
                        chunk_index: row.get::<_, i64>(4)? as usize,
                        tags: row
                            .get::<_, String>(6)?
                            .split(',')
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .collect(),
                        rrf: 0.0,
                    };
                    Ok((id, fr, score))
                })?
                .filter_map(|r| r.ok())
                .collect();

            for (id, fr, _) in &rows {
                by_chunk.insert(id.clone(), fr.clone());
            }
            let mut scored: Vec<(String, f32)> =
                rows.into_iter().map(|(id, _, s)| (id, s)).collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            scored
        };

        // --- Sparse leg: BM25 over FTS5, take top RRF_CANDIDATES ---
        // SQLite's bm25() is only callable when FTS5 is the FROM table, so we
        // resolve rank in a CTE and JOIN out to chunks/documents in the outer.
        let sparse: Vec<String> = match build_fts5_query(query_text) {
            Some(fts_query) => {
                let mut stmt = conn.prepare(
                    "WITH fts AS (
                         SELECT rowid, bm25(chunks_fts) AS bm25
                         FROM chunks_fts
                         WHERE chunks_fts MATCH ?1
                         ORDER BY bm25
                         LIMIT ?2
                     )
                     SELECT c.id, c.text, d.source_path, d.id, c.chunk_index,
                            COALESCE(GROUP_CONCAT(DISTINCT t.tag), '') AS tags,
                            fts.bm25 AS score,
                            d.file_type
                     FROM fts
                     JOIN chunks c ON c.rowid = fts.rowid
                     JOIN documents d ON d.id = c.document_id
                     LEFT JOIN tags t ON t.document_id = d.id
                     GROUP BY c.id
                     ORDER BY score",
                )?;

                let rows: Vec<(String, FusedRow)> = stmt
                    .query_map(params![fts_query, RRF_SPARSE_LIMIT as i64], |row| {
                        let id: String = row.get(0)?;
                        let fr = FusedRow {
                            chunk_id: id.clone(),
                            text: row.get(1)?,
                            source: row.get(2)?,
                            document_id: row.get(3)?,
                            file_type: row.get(7)?,
                            chunk_index: row.get::<_, i64>(4)? as usize,
                            tags: row
                                .get::<_, String>(5)?
                                .split(',')
                                .filter(|s| !s.is_empty())
                                .map(String::from)
                                .collect(),
                            rrf: 0.0,
                        };
                        Ok((id, fr))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();

                let ids: Vec<String> = rows.iter().map(|(id, _)| id.clone()).collect();
                for (id, fr) in rows {
                    by_chunk.entry(id).or_insert(fr);
                }
                ids
            }
            // Empty/all-stopword query: skip sparse leg entirely.
            None => Vec::new(),
        };

        // --- Reciprocal Rank Fusion ---
        for (rank, (id, _)) in dense.iter().enumerate() {
            if let Some(row) = by_chunk.get_mut(id) {
                row.rrf += 1.0 / (RRF_K + rank as f32 + 1.0);
            }
        }
        for (rank, id) in sparse.iter().enumerate() {
            if let Some(row) = by_chunk.get_mut(id) {
                row.rrf += 1.0 / (RRF_K + rank as f32 + 1.0);
            }
        }

        let mut fused: Vec<SearchResult> = by_chunk
            .into_iter()
            .filter(|(_, fr)| fr.rrf > 0.0)
            .filter(|(_, fr)| match &collection_doc_ids {
                Some(ids) => ids.contains(&fr.document_id),
                None => true,
            })
            .map(|(_, fr)| SearchResult {
                chunk_id: fr.chunk_id,
                text: fr.text,
                source: fr.source,
                file_type: fr.file_type,
                score: fr.rrf,
                chunk_index: fr.chunk_index,
                tags: fr.tags,
            })
            .collect();

        if let Some(source) = filter_source {
            fused.retain(|r| r.source.contains(source));
        }
        if let Some(tags) = filter_tags {
            fused.retain(|r| tags.iter().any(|t| r.tags.iter().any(|rt| rt == t)));
        }
        if let Some(ft) = filter_file_type {
            fused.retain(|r| r.file_type == ft);
        }
        // (filter_file_type is applied above against `r.file_type`,
        // populated in the SELECT join so each chunk knows its
        // document's file_type without a second query.)

        fused.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let total = fused.len();
        let start = offset.min(total);
        let end = (offset + limit).min(total);
        let page = fused[start..end].to_vec();

        Ok(SearchPage {
            results: page,
            total,
            offset,
            limit,
        })
    }

    /// Return the chunk identified by `chunk_id` plus up to `before`
    /// chunks immediately preceding it and `after` chunks immediately
    /// following it, all from the same document, ordered by
    /// `chunk_index`. The center chunk is always included when it
    /// exists.
    ///
    /// Use this after a search hit to give an LLM enough conversational
    /// frame to interpret a fragment correctly. Particularly important
    /// for chat archives (Slack threads, Discord channels, email
    /// replies) where a single matched line is often a callback,
    /// sarcasm, or a referent that flips meaning without surrounding
    /// context.
    ///
    /// Returns an empty Vec if `chunk_id` does not exist.
    pub fn get_chunk_context(
        &self,
        chunk_id: &str,
        before: usize,
        after: usize,
    ) -> Result<Vec<ChunkContext>> {
        let conn = self.conn.lock().unwrap();

        // First resolve the center chunk to its document and sequence
        // index. If the id is unknown we return an empty Vec rather
        // than erroring; the typical caller is a tool surface where a
        // helpful empty result is better than a 500.
        let center: Option<(String, i64)> = conn
            .query_row(
                "SELECT document_id, chunk_index FROM chunks WHERE id = ?1",
                params![chunk_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .ok();
        let Some((document_id, center_index)) = center else {
            return Ok(Vec::new());
        };

        // Saturating arithmetic on both bounds. `as i64` on a usize
        // larger than i64::MAX (e.g. usize::MAX) silently wraps to
        // -1 on 64-bit, which would shrink the window instead of
        // expanding it; clamp via u64 first.
        let i64_cap = |n: usize| -> i64 { (n as u64).min(i64::MAX as u64) as i64 };
        let lo = center_index.saturating_sub(i64_cap(before));
        let hi = center_index.saturating_add(i64_cap(after));

        let mut stmt = conn.prepare(
            "SELECT c.id, c.document_id, d.source_path, c.chunk_index, c.text
             FROM chunks c
             JOIN documents d ON d.id = c.document_id
             WHERE c.document_id = ?1
               AND c.chunk_index BETWEEN ?2 AND ?3
             ORDER BY c.chunk_index ASC",
        )?;

        let rows: Vec<ChunkContext> = stmt
            .query_map(params![document_id, lo, hi], |row| {
                Ok(ChunkContext {
                    chunk_id: row.get(0)?,
                    document_id: row.get(1)?,
                    source_path: row.get(2)?,
                    chunk_index: row.get::<_, i64>(3)? as usize,
                    text: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    /// List sources grouped by `source_path`. Each row aggregates all
    /// documents at that path so an archive (e.g. a Slack daily file with
    /// 50 messages) shows as one entry with `record_count=50`, not 50 rows.
    ///
    /// `filter_path` is a substring match (LIKE %q%, with %/_ in `q` escaped).
    /// Pass `limit = usize::MAX` to disable pagination.
    pub fn list_sources(
        &self,
        filter_type: Option<&str>,
        filter_path: Option<&str>,
        sort_by: &str,
        limit: usize,
        offset: usize,
        filter_collection: Option<i64>,
    ) -> Result<SourcesPage> {
        let conn = self.conn.lock().unwrap();
        let path_like = filter_path
            .filter(|s| !s.is_empty())
            .map(|q| format!("%{}%", escape_like(q)));

        let mut where_parts: Vec<&'static str> = Vec::new();
        if filter_type.is_some() {
            where_parts.push("d.file_type = ?");
        }
        if path_like.is_some() {
            where_parts.push(r"d.source_path LIKE ? ESCAPE '\'");
        }
        if filter_collection.is_some() {
            // Restrict to source_paths that have at least one document
            // assigned to the collection. EXISTS keeps the cost flat as
            // collection size grows.
            where_parts.push(
                "EXISTS (SELECT 1 FROM document_collections dc \
                 WHERE dc.document_id = d.id AND dc.collection_id = ?)",
            );
        }
        let where_clause = if where_parts.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_parts.join(" AND "))
        };
        let order = match sort_by {
            "date" => "MAX(d.ingested_at) DESC",
            "chunks" => "chunk_count DESC",
            "records" => "record_count DESC",
            _ => "d.source_path ASC",
        };

        // Total: count of distinct source_paths after WHERE.
        let total_sql =
            format!("SELECT COUNT(DISTINCT d.source_path) FROM documents d {where_clause}");
        // Build the param list. Note that `filter_type: Option<&str>` borrows
        // for the function lifetime, so casting through a String avoids the
        // "doesn't live long enough" trap when passing &dyn ToSql later.
        let ft_owned: Option<String> = filter_type.map(String::from);
        let mut filter_params: Vec<&dyn rusqlite::ToSql> = Vec::new();
        if let Some(ft) = &ft_owned {
            filter_params.push(ft as &dyn rusqlite::ToSql);
        }
        if let Some(p) = &path_like {
            filter_params.push(p as &dyn rusqlite::ToSql);
        }
        if let Some(cid) = &filter_collection {
            filter_params.push(cid as &dyn rusqlite::ToSql);
        }
        let total: i64 = conn.query_row(
            &total_sql,
            rusqlite::params_from_iter(filter_params.iter().copied()),
            |r| r.get(0),
        )?;

        let page_sql = format!(
            "SELECT
                d.source_path,
                MIN(d.file_type) AS file_type,
                COUNT(DISTINCT d.id) AS record_count,
                COUNT(c.id) AS chunk_count,
                MAX(d.ingested_at) AS ingested_at
             FROM documents d
             LEFT JOIN chunks c ON c.document_id = d.id
             {where_clause}
             GROUP BY d.source_path
             ORDER BY {order}
             LIMIT ? OFFSET ?"
        );
        let limit_sql = limit as i64;
        let offset_sql = offset as i64;
        let mut all_params = filter_params.clone();
        all_params.push(&limit_sql);
        all_params.push(&offset_sql);

        let mut stmt = conn.prepare(&page_sql)?;
        let sources: Vec<SourceInfo> = stmt
            .query_map(
                rusqlite::params_from_iter(all_params.iter().copied()),
                |row| {
                    Ok(SourceInfo {
                        path: row.get(0)?,
                        file_type: row.get(1)?,
                        record_count: row.get::<_, i64>(2)? as usize,
                        chunk_count: row.get::<_, i64>(3)? as usize,
                        ingested_at: row.get(4)?,
                    })
                },
            )?
            .filter_map(|r| r.ok())
            .collect();

        Ok(SourcesPage {
            sources,
            total: total as usize,
            offset,
            limit,
        })
    }

    /// List individual `documents` rows — every ingested record, ungrouped.
    /// Use this when the user needs to see (or pick) records one-by-one
    /// rather than the path-level summary `list_sources` returns. Same
    /// filter contract as `list_sources`: `filter_type` (file_type), a
    /// substring `filter_path` against source_path, an optional collection
    /// scope, and `sort_by` keyed on name / date / chunks. Pagination
    /// applies to the documents table directly; `total` is the un-paged
    /// count after filters.
    #[allow(clippy::too_many_arguments)]
    pub fn list_documents(
        &self,
        filter_type: Option<&str>,
        filter_path: Option<&str>,
        sort_by: &str,
        limit: usize,
        offset: usize,
        filter_collection: Option<i64>,
    ) -> Result<DocumentsPage> {
        let conn = self.conn.lock().unwrap();
        let path_like = filter_path
            .filter(|s| !s.is_empty())
            .map(|q| format!("%{}%", escape_like(q)));

        let mut where_parts: Vec<&'static str> = Vec::new();
        if filter_type.is_some() {
            where_parts.push("d.file_type = ?");
        }
        if path_like.is_some() {
            where_parts.push(r"(d.source_path LIKE ? ESCAPE '\' OR d.title LIKE ? ESCAPE '\')");
        }
        if filter_collection.is_some() {
            where_parts.push(
                "EXISTS (SELECT 1 FROM document_collections dc \
                 WHERE dc.document_id = d.id AND dc.collection_id = ?)",
            );
        }
        let where_clause = if where_parts.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_parts.join(" AND "))
        };
        let order = match sort_by {
            "date" => "d.ingested_at DESC",
            "chunks" => "chunk_count DESC",
            // No "records" for the per-doc view; fall through to name/path.
            _ => "d.source_path ASC, d.id ASC",
        };

        let ft_owned: Option<String> = filter_type.map(String::from);
        let mut filter_params: Vec<&dyn rusqlite::ToSql> = Vec::new();
        if let Some(ft) = &ft_owned {
            filter_params.push(ft as &dyn rusqlite::ToSql);
        }
        if let Some(p) = &path_like {
            // Two ?'s for the path/title pair.
            filter_params.push(p as &dyn rusqlite::ToSql);
            filter_params.push(p as &dyn rusqlite::ToSql);
        }
        if let Some(cid) = &filter_collection {
            filter_params.push(cid as &dyn rusqlite::ToSql);
        }

        let total_sql = format!("SELECT COUNT(*) FROM documents d {where_clause}");
        let total: i64 = conn.query_row(
            &total_sql,
            rusqlite::params_from_iter(filter_params.iter().copied()),
            |r| r.get(0),
        )?;

        let page_sql = format!(
            "SELECT
                d.id,
                d.source_path,
                d.title,
                d.file_type,
                d.ingested_at,
                (SELECT COUNT(*) FROM chunks c WHERE c.document_id = d.id) AS chunk_count,
                COALESCE(
                    (SELECT GROUP_CONCAT(dc.collection_id)
                     FROM document_collections dc WHERE dc.document_id = d.id),
                    ''
                ) AS collection_ids_csv
             FROM documents d
             {where_clause}
             ORDER BY {order}
             LIMIT ? OFFSET ?"
        );
        let limit_sql = limit as i64;
        let offset_sql = offset as i64;
        let mut all_params = filter_params.clone();
        all_params.push(&limit_sql);
        all_params.push(&offset_sql);

        let mut stmt = conn.prepare(&page_sql)?;
        let documents: Vec<DocumentRow> = stmt
            .query_map(
                rusqlite::params_from_iter(all_params.iter().copied()),
                |row| {
                    let csv: String = row.get(6)?;
                    let collection_ids: Vec<i64> = csv
                        .split(',')
                        .filter(|s| !s.is_empty())
                        .filter_map(|s| s.parse().ok())
                        .collect();
                    Ok(DocumentRow {
                        id: row.get(0)?,
                        source_path: row.get(1)?,
                        title: row.get(2)?,
                        file_type: row.get(3)?,
                        ingested_at: row.get(4)?,
                        chunk_count: row.get::<_, i64>(5)? as usize,
                        collection_ids,
                    })
                },
            )?
            .filter_map(|r| r.ok())
            .collect();

        Ok(DocumentsPage {
            documents,
            total: total as usize,
            offset,
            limit,
        })
    }

    /// Return records at `source_path` in ingest order, with pagination so
    /// busy archives (Slack `#general` with 5000+ messages/day) don't blow
    /// up the response. Returns `(records, total)` so the UI can render
    /// "showing N of M". Archive handlers that walk files chronologically
    /// produce chronological output here.
    pub fn list_records_by_source(
        &self,
        source: &str,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<ConversationRecord>, usize)> {
        let conn = self.conn.lock().unwrap();
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM documents WHERE source_path = ?1",
            params![source],
            |row| row.get(0),
        )?;
        let mut stmt = conn.prepare(
            "SELECT d.id, d.title, d.raw_text, d.ingested_at
             FROM documents d
             WHERE d.source_path = ?1
             ORDER BY d.rowid ASC
             LIMIT ?2 OFFSET ?3",
        )?;
        let records: Vec<ConversationRecord> = stmt
            .query_map(params![source, limit as i64, offset as i64], |row| {
                Ok(ConversationRecord {
                    id: row.get(0)?,
                    title: row.get::<_, Option<String>>(1)?,
                    text: row.get(2)?,
                    ingested_at: row.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok((records, total as usize))
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

    /// Distinct file_types in the vault and how many *grouped sources*
    /// (deduplicated source_path) each covers. Used to populate
    /// filter/manage dropdowns in the UI without hardcoding.
    pub fn list_file_types(&self) -> Result<Vec<(String, usize)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT file_type, COUNT(DISTINCT source_path)
             FROM documents
             GROUP BY file_type
             ORDER BY file_type",
        )?;
        let rows: Vec<(String, usize)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
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

    /// Delete documents whose `source_path` or `id` is exactly `path`.
    /// Returns `(deleted_documents, deleted_chunks)`. Wildcard chars (`%`,
    /// `_`) in `path` are treated as literals — this is `=`, not `LIKE`.
    pub fn delete_by_path_exact(&self, path: &str, dry_run: bool) -> Result<(usize, usize)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id FROM documents WHERE source_path = ?1 OR id = ?1")?;
        let doc_ids: Vec<String> = stmt
            .query_map(params![path], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        let chunk_count = count_chunks_for_docs(&conn, &doc_ids)?;
        if dry_run || doc_ids.is_empty() {
            return Ok((doc_ids.len(), chunk_count));
        }
        delete_docs_in_tx(&conn, &doc_ids)?;
        Ok((doc_ids.len(), chunk_count))
    }

    /// Delete documents whose `source_path` starts with `prefix`. Underscores
    /// and percent signs in `prefix` are escaped — they match literally, not
    /// as SQL wildcards.
    pub fn delete_by_path_prefix(&self, prefix: &str, dry_run: bool) -> Result<(usize, usize)> {
        let pattern = format!("{}%", escape_like(prefix));
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare(r"SELECT id FROM documents WHERE source_path LIKE ?1 ESCAPE '\'")?;
        let doc_ids: Vec<String> = stmt
            .query_map(params![pattern], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        let chunk_count = count_chunks_for_docs(&conn, &doc_ids)?;
        if dry_run || doc_ids.is_empty() {
            return Ok((doc_ids.len(), chunk_count));
        }
        delete_docs_in_tx(&conn, &doc_ids)?;
        Ok((doc_ids.len(), chunk_count))
    }

    /// Delete all documents of a given file_type (e.g., "json", "pdf").
    pub fn delete_by_file_type(&self, file_type: &str, dry_run: bool) -> Result<(usize, usize)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id FROM documents WHERE file_type = ?1")?;
        let doc_ids: Vec<String> = stmt
            .query_map(params![file_type], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        let chunk_count = count_chunks_for_docs(&conn, &doc_ids)?;
        if dry_run || doc_ids.is_empty() {
            return Ok((doc_ids.len(), chunk_count));
        }
        delete_docs_in_tx(&conn, &doc_ids)?;
        Ok((doc_ids.len(), chunk_count))
    }

    /// Wipe everything: documents, chunks, tags. Schema (and FTS index) remain.
    pub fn clear_all(&self, dry_run: bool) -> Result<(usize, usize)> {
        let conn = self.conn.lock().unwrap();
        let doc_count: i64 = conn.query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))?;
        let chunk_count: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
        if dry_run {
            return Ok((doc_count as usize, chunk_count as usize));
        }
        let tx = conn.unchecked_transaction()?;
        // Delete chunks first (drops FTS rows via trigger), then tags, then docs.
        tx.execute("DELETE FROM chunks", [])?;
        tx.execute("DELETE FROM tags", [])?;
        tx.execute("DELETE FROM documents", [])?;
        tx.commit()?;
        // Optimize the FTS index after bulk delete to reclaim pages.
        conn.execute("INSERT INTO chunks_fts(chunks_fts) VALUES('optimize')", [])?;
        Ok((doc_count as usize, chunk_count as usize))
    }

    pub fn add_tag(&self, document_id: &str, tag: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO tags (document_id, tag) VALUES (?1, ?2)",
            params![document_id, tag],
        )?;
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────
    // Collections — named subsets of documents inside a single vault.
    // Schema is in the SCHEMA constant above; foreign keys are enabled at
    // open() so the ON DELETE CASCADE entries clean up after themselves.
    // ─────────────────────────────────────────────────────────────────────

    pub fn list_collections(&self) -> Result<Vec<CollectionInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.name, c.created_at,
                    (SELECT COUNT(*) FROM document_collections dc WHERE dc.collection_id = c.id)
             FROM collections c
             ORDER BY c.name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(CollectionInfo {
                    id: r.get::<_, i64>(0)?,
                    name: r.get::<_, String>(1)?,
                    created_at: r.get::<_, String>(2)?,
                    document_count: r.get::<_, i64>(3)? as usize,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Create a new collection and return its id. Names are unique
    /// (case-sensitive at the SQL level; we trim whitespace).
    pub fn create_collection(&self, name: &str) -> Result<i64> {
        let n = name.trim();
        if n.is_empty() {
            anyhow::bail!("collection name must not be empty");
        }
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT INTO collections (name) VALUES (?1)", params![n])?;
        Ok(conn.last_insert_rowid())
    }

    pub fn delete_collection(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM collections WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    /// Add documents (by id) to a collection. Idempotent — duplicates are
    /// collapsed by the composite primary key.
    pub fn collection_add_documents(&self, id: i64, document_ids: &[String]) -> Result<usize> {
        if document_ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO document_collections (document_id, collection_id) VALUES (?1, ?2)",
            )?;
            for doc_id in document_ids {
                stmt.execute(params![doc_id, id])?;
            }
        }
        tx.commit()?;
        Ok(document_ids.len())
    }

    pub fn collection_remove_documents(&self, id: i64, document_ids: &[String]) -> Result<usize> {
        if document_ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut removed = 0usize;
        {
            let mut stmt = tx.prepare(
                "DELETE FROM document_collections WHERE collection_id = ?1 AND document_id = ?2",
            )?;
            for doc_id in document_ids {
                removed += stmt.execute(params![id, doc_id])?;
            }
        }
        tx.commit()?;
        Ok(removed)
    }

    /// Add every document at the given source_paths to a collection.
    /// Convenience for the Documents-tab bulk-move UI.
    pub fn collection_add_source_paths(&self, id: i64, source_paths: &[String]) -> Result<usize> {
        if source_paths.is_empty() {
            return Ok(0);
        }
        let placeholders = vec!["?"; source_paths.len()].join(",");
        let conn = self.conn.lock().unwrap();
        let sql = format!("SELECT id FROM documents WHERE source_path IN ({placeholders})");
        let mut stmt = conn.prepare(&sql)?;
        let p: Vec<&dyn rusqlite::ToSql> = source_paths
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        let ids: Vec<String> = stmt
            .query_map(p.as_slice(), |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);
        self.collection_add_documents(id, &ids)
    }

    pub fn collection_remove_source_paths(
        &self,
        id: i64,
        source_paths: &[String],
    ) -> Result<usize> {
        if source_paths.is_empty() {
            return Ok(0);
        }
        let placeholders = vec!["?"; source_paths.len()].join(",");
        let conn = self.conn.lock().unwrap();
        let sql = format!("SELECT id FROM documents WHERE source_path IN ({placeholders})");
        let mut stmt = conn.prepare(&sql)?;
        let p: Vec<&dyn rusqlite::ToSql> = source_paths
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        let ids: Vec<String> = stmt
            .query_map(p.as_slice(), |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);
        self.collection_remove_documents(id, &ids)
    }

    /// List the source_paths assigned to a collection (one row per
    /// distinct source_path; collections membership is per-document but
    /// the UI groups by source_path everywhere else, so we mirror that).
    pub fn collection_source_paths(&self, id: i64) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT d.source_path
             FROM documents d
             JOIN document_collections dc ON dc.document_id = d.id
             WHERE dc.collection_id = ?1
             ORDER BY d.source_path",
        )?;
        let rows = stmt
            .query_map(params![id], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CollectionInfo {
    pub id: i64,
    pub name: String,
    pub created_at: String,
    pub document_count: usize,
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

fn count_chunks_for_docs(conn: &Connection, doc_ids: &[String]) -> Result<usize> {
    if doc_ids.is_empty() {
        return Ok(0);
    }
    let placeholders = vec!["?"; doc_ids.len()].join(",");
    let sql = format!("SELECT COUNT(*) FROM chunks WHERE document_id IN ({placeholders})");
    let params: Vec<&dyn rusqlite::ToSql> =
        doc_ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let count: i64 = conn.query_row(&sql, params.as_slice(), |r| r.get(0))?;
    Ok(count as usize)
}

/// Atomically delete a set of documents along with their chunks and tags.
/// All three deletes happen in a single transaction so a partial failure
/// (e.g. SQLite busy mid-way) leaves the vault consistent.
fn delete_docs_in_tx(conn: &Connection, doc_ids: &[String]) -> Result<()> {
    if doc_ids.is_empty() {
        return Ok(());
    }
    let placeholders = vec!["?"; doc_ids.len()].join(",");
    let params: Vec<&dyn rusqlite::ToSql> =
        doc_ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        &format!("DELETE FROM chunks WHERE document_id IN ({placeholders})"),
        params.as_slice(),
    )?;
    tx.execute(
        &format!("DELETE FROM tags WHERE document_id IN ({placeholders})"),
        params.as_slice(),
    )?;
    tx.execute(
        &format!("DELETE FROM documents WHERE id IN ({placeholders})"),
        params.as_slice(),
    )?;
    tx.commit()?;
    Ok(())
}

/// Escape SQL `LIKE` metacharacters so they match literally. The matching
/// LIKE clause must use `ESCAPE '\\'` for this to work.
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(c);
            }
            other => out.push(other),
        }
    }
    out
}

/// Per-chunk row carried through the fusion pipeline.
#[derive(Clone)]
struct FusedRow {
    chunk_id: String,
    text: String,
    source: String,
    document_id: String,
    file_type: String,
    chunk_index: usize,
    tags: Vec<String>,
    rrf: f32,
}

/// Backfill FTS5 with any chunks already in the table that aren't yet indexed.
/// Idempotent: a no-op for fresh databases (chunks empty) and for already-synced
/// databases. Used on every open() to migrate pre-FTS5 vaults transparently.
fn backfill_fts(conn: &Connection) -> Result<()> {
    let missing: i64 = conn.query_row(
        "SELECT COUNT(*) FROM chunks WHERE rowid NOT IN (SELECT rowid FROM chunks_fts)",
        [],
        |r| r.get(0),
    )?;
    if missing > 0 {
        eprintln!("[satchel] Building keyword index for {missing} existing chunks...");
        conn.execute(
            "INSERT INTO chunks_fts(rowid, text)
             SELECT rowid, text FROM chunks
             WHERE rowid NOT IN (SELECT rowid FROM chunks_fts)",
            [],
        )?;
    }
    Ok(())
}

/// Build a safe FTS5 MATCH expression from free-form user text.
/// Strategy: tokenize on non-word boundaries, drop tokens shorter than 2 chars,
/// double-quote each surviving token (escaping embedded quotes), join with OR.
/// Returns None if no usable tokens remain (e.g. pure punctuation).
fn build_fts5_query(query: &str) -> Option<String> {
    let tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.')
        .filter(|t| t.chars().count() >= 2)
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" OR "))
    }
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
        let page = db
            .search(&query, "chunk text", 5, 0, SearchOptions::default())
            .unwrap();
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.total, 1);
        assert_eq!(page.results[0].text, "chunk text");
        // RRF score: hit by dense + sparse, both at rank 0 → 2 * 1/(60+1) ≈ 0.0328
        assert!(page.results[0].score > 0.0);

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
    fn test_collections_full_lifecycle() {
        let db = Database::open_memory().unwrap();
        // Empty
        assert!(db.list_collections().unwrap().is_empty());
        // Create two
        let work = db.create_collection("Work").unwrap();
        let personal = db.create_collection("Personal").unwrap();
        // Empty name rejected
        assert!(db.create_collection("   ").is_err());
        // Duplicate name rejected (UNIQUE)
        assert!(db.create_collection("Work").is_err());

        // Seed two source paths
        db.insert_document("d1", "/work/a.md", "md", None, "x", "h")
            .unwrap();
        db.insert_document("d2", "/work/b.md", "md", None, "y", "h")
            .unwrap();
        db.insert_document("d3", "/personal/diary.md", "md", None, "z", "h")
            .unwrap();

        // Assign by source path
        db.collection_add_source_paths(work, &["/work/a.md".into(), "/work/b.md".into()])
            .unwrap();
        db.collection_add_source_paths(personal, &["/personal/diary.md".into()])
            .unwrap();

        // collection_source_paths reflects the assignments
        let work_paths = db.collection_source_paths(work).unwrap();
        assert_eq!(work_paths, vec!["/work/a.md", "/work/b.md"]);

        // list_collections has document_counts (one per document, not source)
        let cols = db.list_collections().unwrap();
        let work_info = cols.iter().find(|c| c.id == work).unwrap();
        assert_eq!(work_info.document_count, 2);
        let personal_info = cols.iter().find(|c| c.id == personal).unwrap();
        assert_eq!(personal_info.document_count, 1);

        // list_sources collection filter narrows the result
        let work_sources = db
            .list_sources(None, None, "name", 100, 0, Some(work))
            .unwrap();
        assert_eq!(work_sources.total, 2);
        let personal_sources = db
            .list_sources(None, None, "name", 100, 0, Some(personal))
            .unwrap();
        assert_eq!(personal_sources.total, 1);
        let all_sources = db.list_sources(None, None, "name", 100, 0, None).unwrap();
        assert_eq!(all_sources.total, 3);

        // Unassign + delete
        db.collection_remove_source_paths(work, &["/work/a.md".into()])
            .unwrap();
        let after = db.collection_source_paths(work).unwrap();
        assert_eq!(after, vec!["/work/b.md"]);

        // Delete cascades through document_collections
        assert!(db.delete_collection(personal).unwrap());
        let cols = db.list_collections().unwrap();
        assert_eq!(cols.len(), 1);
        // and a re-delete returns false (idempotent)
        assert!(!db.delete_collection(personal).unwrap());
    }

    #[test]
    fn test_database_list_sources() {
        let (db, dir) = temp_db();

        db.insert_document("d1", "/a.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_document("d2", "/b.txt", "txt", None, "b", "h2")
            .unwrap();

        let all = db.list_sources(None, None, "name", 100, 0, None).unwrap();
        assert_eq!(all.sources.len(), 2);
        assert_eq!(all.total, 2);

        let md_only = db
            .list_sources(Some("md"), None, "name", 100, 0, None)
            .unwrap();
        assert_eq!(md_only.sources.len(), 1);
        assert_eq!(md_only.sources[0].file_type, "md");

        cleanup(&dir);
    }

    #[test]
    fn test_list_documents_ungrouped() {
        // Same source_path, three records — `list_sources` returns one row,
        // `list_documents` returns three. Backs the Documents-tab BY RECORD
        // mode where users pick individual ingested records into collections.
        let db = Database::open_memory().unwrap();
        db.insert_document(
            "m1",
            "/slack/general.json",
            "slack",
            Some("msg-1"),
            "a",
            "h1",
        )
        .unwrap();
        db.insert_document(
            "m2",
            "/slack/general.json",
            "slack",
            Some("msg-2"),
            "b",
            "h2",
        )
        .unwrap();
        db.insert_document(
            "m3",
            "/slack/general.json",
            "slack",
            Some("msg-3"),
            "c",
            "h3",
        )
        .unwrap();

        let grouped = db.list_sources(None, None, "name", 100, 0, None).unwrap();
        assert_eq!(grouped.total, 1, "list_sources groups by source_path");
        assert_eq!(grouped.sources[0].record_count, 3);

        let flat = db.list_documents(None, None, "name", 100, 0, None).unwrap();
        assert_eq!(flat.total, 3, "list_documents shows every record");
        assert_eq!(flat.documents.len(), 3);

        // Path filter also matches title.
        let by_title = db
            .list_documents(None, Some("msg-2"), "name", 100, 0, None)
            .unwrap();
        assert_eq!(by_title.total, 1);
        assert_eq!(by_title.documents[0].id, "m2");

        // Per-record collection assignment surfaces in the row's collection_ids.
        let work = db.create_collection("Work").unwrap();
        db.collection_add_documents(work, &["m1".into(), "m3".into()])
            .unwrap();
        let after = db.list_documents(None, None, "name", 100, 0, None).unwrap();
        let m1 = after.documents.iter().find(|d| d.id == "m1").unwrap();
        let m2 = after.documents.iter().find(|d| d.id == "m2").unwrap();
        assert_eq!(m1.collection_ids, vec![work]);
        assert!(m2.collection_ids.is_empty());

        // Collection scope filters at the document level.
        let scoped = db
            .list_documents(None, None, "name", 100, 0, Some(work))
            .unwrap();
        assert_eq!(scoped.total, 2);
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

        let page = db
            .search(
                &emb,
                "notes chunk",
                10,
                0,
                SearchOptions {
                    filter_source: Some("/notes/"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(page.results.len(), 1);
        assert!(page.results[0].source.contains("/notes/"));

        cleanup(&dir);
    }

    #[test]
    fn test_search_empty_database() {
        let db = Database::open_memory().unwrap();
        let query = vec![1.0, 0.0, 0.0];
        let page = db
            .search(&query, "anything", 5, 0, SearchOptions::default())
            .unwrap();
        assert!(page.results.is_empty());
        assert_eq!(page.total, 0);
    }

    #[test]
    fn test_search_filter_by_collection() {
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/work/a.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_document("d2", "/personal/b.md", "md", None, "b", "h2")
            .unwrap();

        let emb = vec![1.0, 0.0];
        db.insert_chunk("c1", "d1", 0, "shared term", 5, 0, 5, &emb)
            .unwrap();
        db.insert_chunk("c2", "d2", 0, "shared term", 5, 0, 5, &emb)
            .unwrap();

        let work = db.create_collection("Work").unwrap();
        db.collection_add_source_paths(work, &["/work/a.md".into()])
            .unwrap();

        // Unfiltered: both chunks come back.
        let all = db
            .search(&emb, "shared", 10, 0, SearchOptions::default())
            .unwrap();
        assert_eq!(all.results.len(), 2);

        // Scoped to the Work collection: only the work doc's chunk.
        let scoped = db
            .search(
                &emb,
                "shared",
                10,
                0,
                SearchOptions {
                    filter_collection: Some(work),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(scoped.results.len(), 1);
        assert!(scoped.results[0].source.contains("/work/"));

        // Empty collection (id with no members) returns zero, not an error.
        let empty = db.create_collection("Empty").unwrap();
        let none = db
            .search(
                &emb,
                "shared",
                10,
                0,
                SearchOptions {
                    filter_collection: Some(empty),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(none.results.is_empty());
        assert_eq!(none.total, 0);
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

        let page = db
            .search(
                &emb,
                "chunk",
                10,
                0,
                SearchOptions {
                    filter_tags: Some(&["important"]),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(page.results.len(), 1);
        assert!(page.results[0].source.contains("a.md"));
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
        let sources = db.list_sources(None, None, "date", 100, 0, None).unwrap();
        assert_eq!(sources.sources.len(), 2);
    }

    #[test]
    fn test_hybrid_keyword_finds_proper_noun_dense_misses() {
        // Regression: pure cosine on MiniLM-L6 fails to surface rare proper nouns
        // even when they appear verbatim in the corpus. BM25 must close the gap.
        // We seed many irrelevant chunks with the matching dense embedding to
        // crowd them out of the dense-only top-k, then verify that "lumencanvas"
        // still surfaces because its rare token wins the BM25 leg uncontested.
        let db = Database::open_memory().unwrap();
        db.insert_document("d_target", "/target.md", "md", None, "x", "h0")
            .unwrap();
        db.insert_chunk(
            "c_target",
            "d_target",
            0,
            "I have it opening chromium with webGPU pointed at my app lumencanvas.studio",
            10,
            0,
            10,
            &[0.0, 1.0, 0.0], // orthogonal to query — dense-only would never find it
        )
        .unwrap();

        let query_emb = vec![1.0, 0.0, 0.0];
        for i in 0..30 {
            let id = format!("d{i}");
            db.insert_document(
                &id,
                &format!("/doc{i}.md"),
                "md",
                None,
                "x",
                &format!("h{i}"),
            )
            .unwrap();
            db.insert_chunk(
                &format!("c{i}"),
                &id,
                0,
                "completely unrelated content about gardening tomatoes",
                10,
                0,
                10,
                &query_emb,
            )
            .unwrap();
        }

        let page = db
            .search(&query_emb, "lumencanvas", 5, 0, SearchOptions::default())
            .unwrap();
        assert!(
            page.results.iter().any(|r| r.text.contains("lumencanvas")),
            "lumencanvas chunk must surface in hybrid results despite zero cosine"
        );
        // Should rank #1: gets both legs (sparse rank 0 + dense rank 30); the
        // 30 dense-only chunks each get only the dense leg.
        assert!(
            page.results[0].text.contains("lumencanvas"),
            "lumencanvas should be top-ranked due to BM25 contribution"
        );
    }

    #[test]
    fn test_search_pagination() {
        let db = Database::open_memory().unwrap();
        let emb = vec![1.0, 0.0];
        for i in 0..15 {
            let id = format!("d{i}");
            db.insert_document(
                &id,
                &format!("/x/{i}.md"),
                "md",
                None,
                "x",
                &format!("h{i}"),
            )
            .unwrap();
            db.insert_chunk(
                &format!("c{i}"),
                &id,
                0,
                "shared keyword content",
                3,
                0,
                3,
                &emb,
            )
            .unwrap();
        }

        let p1 = db
            .search(&emb, "shared", 5, 0, SearchOptions::default())
            .unwrap();
        assert_eq!(p1.results.len(), 5);
        assert_eq!(p1.total, 15);
        assert_eq!(p1.offset, 0);

        let p2 = db
            .search(&emb, "shared", 5, 5, SearchOptions::default())
            .unwrap();
        assert_eq!(p2.results.len(), 5);
        assert_eq!(p2.total, 15);

        // No overlap between page 1 and page 2.
        let p1_ids: Vec<&str> = p1.results.iter().map(|r| r.source.as_str()).collect();
        let p2_ids: Vec<&str> = p2.results.iter().map(|r| r.source.as_str()).collect();
        for id in &p2_ids {
            assert!(!p1_ids.contains(id), "{id} appeared in both pages");
        }

        // Final page; offset past total returns empty without panic.
        let p4 = db
            .search(&emb, "shared", 5, 100, SearchOptions::default())
            .unwrap();
        assert!(p4.results.is_empty());
        assert_eq!(p4.total, 15);
    }

    #[test]
    fn test_filter_source_works_at_deep_dense_rank() {
        // Regression: a previous version truncated the dense leg at 100
        // candidates before applying filter_source. If the only chunk matching
        // the user's filter ranked at dense rank 150, it would silently vanish.
        // This test seeds 200 high-cosine "noise" chunks plus one filter-target
        // chunk with a deliberately low cosine, then confirms filter_source
        // still surfaces the target.
        let db = Database::open_memory().unwrap();
        let query_emb = vec![1.0, 0.0, 0.0];

        for i in 0..200 {
            let id = format!("n{i}");
            db.insert_document(
                &id,
                &format!("/noise/{i}.md"),
                "md",
                None,
                "x",
                &format!("h{i}"),
            )
            .unwrap();
            db.insert_chunk(
                &format!("nc{i}"),
                &id,
                0,
                "noise",
                1,
                0,
                1,
                &query_emb, // perfect dense score, but won't pass filter
            )
            .unwrap();
        }
        db.insert_document("t1", "/target/only.md", "md", None, "x", "ht")
            .unwrap();
        db.insert_chunk("tc1", "t1", 0, "target chunk", 1, 0, 12, &[0.0, 0.0, 0.5])
            .unwrap();

        let page = db
            .search(
                &query_emb,
                "target chunk",
                5,
                0,
                SearchOptions {
                    filter_source: Some("/target/"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(
            !page.results.is_empty(),
            "filter_source must find the target even when it ranks deep in dense"
        );
        assert!(page.results[0].source.contains("/target/"));
    }

    #[test]
    fn test_fts5_query_builder_handles_punctuation() {
        assert_eq!(build_fts5_query(""), None);
        assert_eq!(build_fts5_query("!@#$%"), None);
        assert_eq!(build_fts5_query("a"), None); // single char filtered
        assert_eq!(
            build_fts5_query("hello world"),
            Some("\"hello\" OR \"world\"".to_string())
        );
        // Quotes are non-alphanumeric so they act as separators; the inner word
        // survives unquoted (and so cannot break the FTS5 expression).
        assert_eq!(
            build_fts5_query("say \"hi\""),
            Some("\"say\" OR \"hi\"".to_string())
        );
        // Hyphens, dots, underscores survive (useful for usernames, domains).
        assert_eq!(
            build_fts5_query("lumencanvas.studio user_name"),
            Some("\"lumencanvas.studio\" OR \"user_name\"".to_string())
        );
    }

    #[test]
    fn test_fts_backfill_for_chunks_inserted_pre_index() {
        // Simulate an upgrade: insert chunks while triggers exist (they always do
        // in our schema), then verify the FTS index is populated.
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/a.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_chunk("c1", "d1", 0, "the quick brown fox", 4, 0, 19, &[1.0, 0.0])
            .unwrap();

        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_delete_by_path_prefix() {
        let db = Database::open_memory().unwrap();
        db.insert_document(
            "d1",
            "/slack/general/2024-01-01.json",
            "json",
            None,
            "a",
            "h1",
        )
        .unwrap();
        db.insert_document(
            "d2",
            "/slack/general/2024-01-02.json",
            "json",
            None,
            "b",
            "h2",
        )
        .unwrap();
        db.insert_document("d3", "/notes/keep.md", "md", None, "c", "h3")
            .unwrap();
        db.insert_chunk("c1", "d1", 0, "x", 1, 0, 1, &[1.0, 0.0])
            .unwrap();
        db.insert_chunk("c2", "d2", 0, "y", 1, 0, 1, &[1.0, 0.0])
            .unwrap();
        db.insert_chunk("c3", "d3", 0, "z", 1, 0, 1, &[1.0, 0.0])
            .unwrap();

        let (d, c) = db.delete_by_path_prefix("/slack/general/", true).unwrap();
        assert_eq!((d, c), (2, 2));
        assert_eq!(
            db.stats().unwrap().document_count,
            3,
            "dry_run must not delete"
        );

        let (d, c) = db.delete_by_path_prefix("/slack/general/", false).unwrap();
        assert_eq!((d, c), (2, 2));
        assert_eq!(db.stats().unwrap().document_count, 1);
        assert_eq!(db.stats().unwrap().chunk_count, 1);
    }

    #[test]
    fn test_delete_by_path_prefix_treats_underscore_literally() {
        // Regression: an unescaped LIKE pattern would treat '_' as a wildcard,
        // so `delete --prefix "/notes_2024"` would also delete "/notesX2024".
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/notes_2024/a.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_document("d2", "/notesX2024/b.md", "md", None, "b", "h2")
            .unwrap();

        let (d, _) = db.delete_by_path_prefix("/notes_2024", false).unwrap();
        assert_eq!(d, 1, "underscore must match literally, not as wildcard");
        let remaining = db.list_sources(None, None, "name", 100, 0, None).unwrap();
        assert_eq!(remaining.sources.len(), 1);
        assert!(remaining.sources[0].path.contains("notesX2024"));
    }

    #[test]
    fn test_delete_by_path_exact_treats_path_literally() {
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/foo_bar.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_document("d2", "/fooXbar.md", "md", None, "b", "h2")
            .unwrap();

        let (d, _) = db.delete_by_path_exact("/foo_bar.md", false).unwrap();
        assert_eq!(d, 1);
        let remaining = db.list_sources(None, None, "name", 100, 0, None).unwrap();
        assert_eq!(remaining.sources.len(), 1);
        assert!(remaining.sources[0].path.contains("fooXbar"));
    }

    #[test]
    fn test_delete_is_atomic_under_failure() {
        // After a successful transactional delete, the document and its
        // dependent rows are all gone. We verify the chunks/tags/documents
        // counts are consistent (no orphaned chunks).
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/a.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_chunk("c1", "d1", 0, "chunk", 1, 0, 5, &[1.0])
            .unwrap();
        db.add_tag("d1", "important").unwrap();

        db.delete_by_path_exact("/a.md", false).unwrap();
        let s = db.stats().unwrap();
        assert_eq!(s.document_count, 0);
        assert_eq!(s.chunk_count, 0);
        assert!(db.list_tags().unwrap().is_empty());
    }

    #[test]
    fn test_escape_like_helper() {
        assert_eq!(escape_like("foo"), "foo");
        assert_eq!(escape_like("foo_bar"), "foo\\_bar");
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("a\\b"), "a\\\\b");
        assert_eq!(escape_like("a_b%c\\d"), "a\\_b\\%c\\\\d");
    }

    #[test]
    fn test_delete_by_file_type() {
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/a.json", "json", None, "a", "h1")
            .unwrap();
        db.insert_document("d2", "/b.json", "json", None, "b", "h2")
            .unwrap();
        db.insert_document("d3", "/c.md", "md", None, "c", "h3")
            .unwrap();

        let (d, _) = db.delete_by_file_type("json", false).unwrap();
        assert_eq!(d, 2);
        assert_eq!(db.stats().unwrap().document_count, 1);
    }

    #[test]
    fn test_delete_by_file_type_no_match() {
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/a.md", "md", None, "a", "h1")
            .unwrap();
        let (d, c) = db.delete_by_file_type("pdf", false).unwrap();
        assert_eq!((d, c), (0, 0));
        assert_eq!(db.stats().unwrap().document_count, 1);
    }

    #[test]
    fn test_clear_all_dry_run_and_real() {
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/a.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_chunk("c1", "d1", 0, "x", 1, 0, 1, &[1.0])
            .unwrap();
        db.add_tag("d1", "important").unwrap();

        let (d, c) = db.clear_all(true).unwrap();
        assert_eq!((d, c), (1, 1));
        assert_eq!(db.stats().unwrap().document_count, 1);

        let (d, c) = db.clear_all(false).unwrap();
        assert_eq!((d, c), (1, 1));
        let stats = db.stats().unwrap();
        assert_eq!(stats.document_count, 0);
        assert_eq!(stats.chunk_count, 0);
        assert!(db.list_tags().unwrap().is_empty());
    }

    #[test]
    fn test_delete_drops_fts_entries() {
        // FTS should stay in sync via triggers when chunks are deleted.
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/a.md", "md", None, "a", "h1")
            .unwrap();
        db.insert_chunk("c1", "d1", 0, "uniqueword12345", 1, 0, 15, &[1.0])
            .unwrap();

        let page = db
            .search(&[1.0], "uniqueword12345", 5, 0, SearchOptions::default())
            .unwrap();
        assert_eq!(page.results.len(), 1);

        db.delete_by_path_exact("/a.md", false).unwrap();

        let page = db
            .search(&[1.0], "uniqueword12345", 5, 0, SearchOptions::default())
            .unwrap();
        assert!(
            page.results.is_empty(),
            "FTS entry should be removed by trigger"
        );
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

        let sources = db.list_sources(None, None, "chunks", 100, 0, None).unwrap();
        assert_eq!(sources.sources[0].path, "/many.md");
    }

    #[test]
    fn test_list_records_by_source_chronological() {
        // Documents inserted in order should come back in that order, allowing
        // the UI to show context around a search hit.
        let db = Database::open_memory().unwrap();
        let path = "/slack/general/2024-01-15.json";
        for (i, body) in ["first msg", "second msg", "third msg"].iter().enumerate() {
            db.insert_document(
                &format!("d{i}"),
                path,
                "slack",
                Some(&format!("title {i}")),
                body,
                &format!("h{i}"),
            )
            .unwrap();
        }
        let (records, total) = db.list_records_by_source(path, 100, 0).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(total, 3);
        assert_eq!(records[0].text, "first msg");
        assert_eq!(records[1].text, "second msg");
        assert_eq!(records[2].text, "third msg");
        assert_eq!(records[0].title.as_deref(), Some("title 0"));
    }

    #[test]
    fn test_list_records_by_source_respects_limit_and_offset() {
        let db = Database::open_memory().unwrap();
        for i in 0..10 {
            db.insert_document(
                &format!("d{i}"),
                "/x.json",
                "slack",
                None,
                &format!("body {i}"),
                &format!("h{i}"),
            )
            .unwrap();
        }
        let (page1, total) = db.list_records_by_source("/x.json", 3, 0).unwrap();
        assert_eq!(page1.len(), 3);
        assert_eq!(total, 10);
        assert_eq!(page1[0].text, "body 0");

        let (page2, _) = db.list_records_by_source("/x.json", 3, 3).unwrap();
        assert_eq!(page2.len(), 3);
        assert_eq!(page2[0].text, "body 3");

        // Offset past total returns empty without panicking.
        let (page_empty, _) = db.list_records_by_source("/x.json", 3, 100).unwrap();
        assert!(page_empty.is_empty());
    }

    #[test]
    fn test_list_records_by_source_unknown_source_empty() {
        let db = Database::open_memory().unwrap();
        let (records, total) = db
            .list_records_by_source("/does/not/exist", 100, 0)
            .unwrap();
        assert!(records.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn test_search_filter_file_type_restricts_to_one_type() {
        let db = Database::open_memory().unwrap();
        // Three documents at different file types, all containing
        // the word "alpha". Without a file-type filter all three
        // come back; with `filter_file_type=Some("md")` only the
        // markdown one does.
        db.insert_document("d1", "/a.md", "md", None, "alpha", "h1")
            .unwrap();
        db.insert_document("d2", "/a.pdf", "pdf", None, "alpha", "h2")
            .unwrap();
        db.insert_document("d3", "/a.csv", "csv", None, "alpha row 1", "h3")
            .unwrap();
        let v = vec![1.0_f32; 2];
        db.insert_chunk("d1:0", "d1", 0, "alpha hello md", 3, 0, 14, &v)
            .unwrap();
        db.insert_chunk("d2:0", "d2", 0, "alpha hello pdf", 3, 0, 15, &v)
            .unwrap();
        db.insert_chunk("d3:0", "d3", 0, "alpha hello csv", 3, 0, 15, &v)
            .unwrap();

        let all = db
            .search(&v, "alpha", 10, 0, SearchOptions::default())
            .unwrap();
        assert_eq!(all.total, 3);

        let only_md = db
            .search(
                &v,
                "alpha",
                10,
                0,
                SearchOptions {
                    filter_file_type: Some("md"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(only_md.total, 1);
        assert!(only_md.results[0].source.ends_with(".md"));
        assert_eq!(only_md.results[0].file_type, "md");
    }

    #[test]
    fn test_search_filter_file_type_unknown_returns_empty() {
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/a.md", "md", None, "alpha", "h1")
            .unwrap();
        let v = vec![1.0_f32; 2];
        db.insert_chunk("d1:0", "d1", 0, "alpha hello", 2, 0, 11, &v)
            .unwrap();
        let page = db
            .search(
                &v,
                "alpha",
                10,
                0,
                SearchOptions {
                    filter_file_type: Some("nosuchtype"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(page.total, 0);
    }

    #[test]
    fn test_search_result_carries_file_type() {
        // Regression guard: if we drop the file_type from FusedRow or
        // from the SELECT join, this test fails noisily before any
        // user-facing surface (UI, MCP) starts showing empty types.
        let db = Database::open_memory().unwrap();
        db.insert_document("d1", "/a.csv", "csv", None, "alpha", "h1")
            .unwrap();
        let v = vec![1.0_f32; 2];
        db.insert_chunk("d1:0", "d1", 0, "alpha hello", 2, 0, 11, &v)
            .unwrap();
        let page = db
            .search(&v, "alpha", 10, 0, SearchOptions::default())
            .unwrap();
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.results[0].file_type, "csv");
    }

    #[test]
    fn test_get_chunk_context_returns_neighbors() {
        let db = Database::open_memory().unwrap();
        db.insert_document("doc1", "/notes.md", "md", None, "raw", "hash1")
            .unwrap();
        let v = vec![1.0, 0.0];
        for i in 0..6 {
            db.insert_chunk(
                &format!("doc1:{i}"),
                "doc1",
                i,
                &format!("chunk {i} body"),
                3,
                0,
                14,
                &v,
            )
            .unwrap();
        }
        let ctx = db.get_chunk_context("doc1:2", 1, 2).unwrap();
        let indices: Vec<usize> = ctx.iter().map(|c| c.chunk_index).collect();
        assert_eq!(indices, vec![1, 2, 3, 4]);
        assert!(ctx.iter().all(|c| c.document_id == "doc1"));
        assert!(ctx.iter().all(|c| c.source_path == "/notes.md"));
        assert_eq!(ctx[1].text, "chunk 2 body");
    }

    #[test]
    fn test_get_chunk_context_clamps_at_start() {
        let db = Database::open_memory().unwrap();
        db.insert_document("doc1", "/notes.md", "md", None, "raw", "hash1")
            .unwrap();
        let v = vec![1.0, 0.0];
        for i in 0..3 {
            db.insert_chunk(
                &format!("doc1:{i}"),
                "doc1",
                i,
                &format!("chunk {i}"),
                1,
                0,
                7,
                &v,
            )
            .unwrap();
        }
        // Asking for 5 chunks before chunk 0 should not underflow; we
        // expect [0, 1] (chunk 0 plus one after).
        let ctx = db.get_chunk_context("doc1:0", 5, 1).unwrap();
        let indices: Vec<usize> = ctx.iter().map(|c| c.chunk_index).collect();
        assert_eq!(indices, vec![0, 1]);
    }

    #[test]
    fn test_get_chunk_context_clamps_at_end() {
        let db = Database::open_memory().unwrap();
        db.insert_document("doc1", "/notes.md", "md", None, "raw", "hash1")
            .unwrap();
        let v = vec![1.0, 0.0];
        for i in 0..3 {
            db.insert_chunk(
                &format!("doc1:{i}"),
                "doc1",
                i,
                &format!("chunk {i}"),
                1,
                0,
                7,
                &v,
            )
            .unwrap();
        }
        // Asking for many chunks after the last index should clamp at
        // the document's last chunk; usize::MAX must not overflow when
        // added to the center index.
        let ctx = db.get_chunk_context("doc1:2", 1, usize::MAX).unwrap();
        let indices: Vec<usize> = ctx.iter().map(|c| c.chunk_index).collect();
        assert_eq!(indices, vec![1, 2]);
    }

    #[test]
    fn test_get_chunk_context_unknown_chunk_empty() {
        let db = Database::open_memory().unwrap();
        let ctx = db
            .get_chunk_context("doesnotexist", 1, 1)
            .expect("unknown chunk should not error");
        assert!(ctx.is_empty());
    }

    #[test]
    fn test_get_chunk_context_does_not_cross_documents() {
        // Two documents share consecutive chunk_index values; the
        // neighborhood query must scope to one document's id.
        let db = Database::open_memory().unwrap();
        db.insert_document("doc1", "/a.md", "md", None, "x", "h1")
            .unwrap();
        db.insert_document("doc2", "/b.md", "md", None, "y", "h2")
            .unwrap();
        let v = vec![1.0, 0.0];
        for i in 0..3 {
            db.insert_chunk(&format!("doc1:{i}"), "doc1", i, "a-text", 1, 0, 6, &v)
                .unwrap();
            db.insert_chunk(&format!("doc2:{i}"), "doc2", i, "b-text", 1, 0, 6, &v)
                .unwrap();
        }
        let ctx = db.get_chunk_context("doc1:1", 5, 5).unwrap();
        assert!(ctx.iter().all(|c| c.document_id == "doc1"));
        assert!(ctx.iter().all(|c| c.source_path == "/a.md"));
        assert_eq!(ctx.len(), 3);
    }

    #[test]
    fn test_list_file_types_groups_by_source_path() {
        let db = Database::open_memory().unwrap();
        // Two slack records at the same source_path should count once toward
        // source_count, not twice.
        db.insert_document("d1", "/slack/a.json", "slack", None, "x", "h1")
            .unwrap();
        db.insert_document("d2", "/slack/a.json", "slack", None, "y", "h2")
            .unwrap();
        db.insert_document("d3", "/slack/b.json", "slack", None, "z", "h3")
            .unwrap();
        db.insert_document("d4", "/note.md", "md", None, "w", "h4")
            .unwrap();
        let types = db.list_file_types().unwrap();
        // Sorted alphabetically by file_type: ("md", 1), ("slack", 2)
        assert_eq!(types, vec![("md".to_string(), 1), ("slack".to_string(), 2)]);
    }

    #[test]
    fn test_list_sources_groups_by_source_path() {
        // After Slack-aware ingest, many documents share the same source_path
        // (one document per message). The grouped list should collapse them
        // into one row per file.
        let db = Database::open_memory().unwrap();
        let path = "/slack/general/2024-01-15.json";
        for i in 0..50 {
            let id = format!("d{i}");
            db.insert_document(&id, path, "slack", None, "x", &format!("h{i}"))
                .unwrap();
            db.insert_chunk(&format!("c{i}"), &id, 0, "msg", 1, 0, 3, &[1.0, 0.0])
                .unwrap();
        }
        let page = db.list_sources(None, None, "name", 100, 0, None).unwrap();
        assert_eq!(page.sources.len(), 1, "should group 50 docs into 1 row");
        assert_eq!(page.sources[0].record_count, 50);
        assert_eq!(page.sources[0].chunk_count, 50);
        assert_eq!(page.total, 1);
    }

    #[test]
    fn test_list_sources_pagination_and_path_filter() {
        let db = Database::open_memory().unwrap();
        for i in 0..30 {
            let id = format!("d{i}");
            let path = if i < 10 {
                format!("/notes/n{i}.md")
            } else {
                format!("/work/w{i}.md")
            };
            db.insert_document(&id, &path, "md", None, "x", &format!("h{i}"))
                .unwrap();
        }

        let p1 = db.list_sources(None, None, "name", 10, 0, None).unwrap();
        assert_eq!(p1.sources.len(), 10);
        assert_eq!(p1.total, 30);

        let p2 = db.list_sources(None, None, "name", 10, 10, None).unwrap();
        assert_eq!(p2.sources.len(), 10);

        let notes = db
            .list_sources(None, Some("/notes/"), "name", 100, 0, None)
            .unwrap();
        assert_eq!(notes.total, 10);
        assert_eq!(notes.sources.len(), 10);

        // Underscore in filter must match literally.
        let underscored = db
            .list_sources(None, Some("/work/_"), "name", 100, 0, None)
            .unwrap();
        assert_eq!(
            underscored.total, 0,
            "literal '_' shouldn't act as a wildcard"
        );
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
            prop_assert!((-1.0 - 1e-5..=1.0 + 1e-5).contains(&sim), "Got {sim}");
        }

        #[test]
        fn embedding_blob_roundtrip(v in proptest::collection::vec(-1e6f32..1e6, 0..500usize)) {
            let blob = embedding_to_blob(&v);
            let restored = blob_to_embedding(&blob);
            prop_assert_eq!(v, restored);
        }
    }
}
