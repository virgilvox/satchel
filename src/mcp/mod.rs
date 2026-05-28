pub mod stdio;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

use crate::embed::Embedder;
use crate::rag::Database;

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

pub fn tool_definitions() -> Value {
    json!({
        "tools": [
            {
                "name": "search_knowledge",
                "description": "Semantic search across the knowledge vault. Returns the most relevant chunks with source attribution. Pass `collection_name` (or `collection_id`) to scope the search to a single named collection (e.g. \"Work notes\").",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Natural language search query"
                        },
                        "top_k": {
                            "type": "integer",
                            "description": "Number of results (default: 5, max: 20)",
                            "default": 5
                        },
                        "filter_source": {
                            "type": "string",
                            "description": "Substring match on source_path. Use to scope to a folder or filename, e.g. \"slack/\" or \"meeting-notes\"."
                        },
                        "filter_tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Filter results to chunks with any of these tags"
                        },
                        "filter_file_type": {
                            "type": "string",
                            "description": "Restrict results to documents of this exact file type (e.g. 'md', 'pdf', 'csv', 'slack', 'mbox', 'tsv', 'json'). Use list_sources to discover the file types in this vault."
                        },
                        "collection_name": {
                            "type": "string",
                            "description": "Restrict results to documents in this named collection. Case-sensitive exact match."
                        },
                        "collection_id": {
                            "type": "integer",
                            "description": "Numeric collection id. Prefer collection_name unless you already know the id."
                        }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "list_sources",
                "description": "List ingested sources grouped by source_path. Paginated; capped at 100 results per call.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "filter_type": {
                            "type": "string",
                            "description": "Filter by file_type (e.g., 'md', 'pdf', 'slack', 'mbox')"
                        },
                        "q": {
                            "type": "string",
                            "description": "Substring match on source_path"
                        },
                        "sort_by": {
                            "type": "string",
                            "enum": ["name", "date", "chunks", "records"],
                            "default": "name"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Page size (default 100, max 500)"
                        },
                        "offset": {
                            "type": "integer",
                            "description": "Pagination offset"
                        }
                    }
                }
            },
            {
                "name": "get_document",
                "description": "Retrieve the full text of a specific document by source path or ID.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "source": {
                            "type": "string",
                            "description": "Source file path or document ID"
                        }
                    },
                    "required": ["source"]
                }
            },
            {
                "name": "list_tags",
                "description": "List all tags/categories with document counts.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "vault_stats",
                "description": "Get vault statistics: documents, chunks, storage size, model info.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "list_collections",
                "description": "List the named collections (subsets of the vault). Returns each collection's id, name, and document_count. Pair with `search_knowledge`'s `collection_name` argument to scope a query.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "get_chunk_context",
                "description": "Return chunks immediately surrounding a search hit, scoped to the same source document. Use this when a `search_knowledge` result looks like a fragment (a single chat message, a quoted sentence, an isolated reply): pass the result's `chunk_id` and a small window to read the conversation or paragraph around it before drawing a conclusion. Particularly important for chat archives (Slack threads, Discord channels, email replies, ChatGPT/Claude.ai conversations) where one matched line is often a callback, sarcasm, or referent that flips meaning without surrounding context. Center chunk is always included; window clamps cleanly at document start/end.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "chunk_id": {
                            "type": "string",
                            "description": "The chunk_id of the hit you want to expand around. `search_knowledge` results expose this as `chunk_id` per result."
                        },
                        "before": {
                            "type": "integer",
                            "description": "How many chunks immediately preceding the center chunk to include (default 2, max 20).",
                            "default": 2
                        },
                        "after": {
                            "type": "integer",
                            "description": "How many chunks immediately following the center chunk to include (default 2, max 20).",
                            "default": 2
                        }
                    },
                    "required": ["chunk_id"]
                }
            },
            {
                "name": "add_to_vault",
                "description": "Save a text snippet, markdown document, or any other textual content into the vault so it becomes searchable. Use this when the user asks to remember a quote, paste a document, capture a synthesis of the current conversation, or commit a note for later retrieval. Content is chunked, embedded, and indexed alongside everything else. Returns a stable document_id you can pass to `assign_to_collection` or `get_document` later. Identical content is deduplicated by SHA-256, so calling twice is a safe no-op (the second call still honors collection_name and tags so re-ingesting into a new collection actually works). Do not call this without an explicit user intent to save; treat each invocation as a privileged write that the user has authorized for this turn. Cap is 50 MB; large pastes (>10 MB) take a few minutes synchronously while every chunk is embedded, so warn the user before committing a big one.",
                "inputSchema": {
                    "type": "object",
                    "required": ["content"],
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The text to save. Plain text, markdown, JSON, HTML, or code. Required; must be non-empty after trimming; capped at 50 MB. For anything bigger, save the file on disk and use `satchel ingest <path>` so the work runs as a tracked background job instead of blocking a single tool call."
                        },
                        "title": {
                            "type": "string",
                            "description": "Short human-readable label shown in list_sources and search results. If omitted, the first line of content (truncated to 80 chars) is used."
                        },
                        "source": {
                            "type": "string",
                            "description": "Logical source identifier. Use a stable meaningful name like 'meeting-2026-05-28' or 'design-notes/v3' so future searches can scope to it via filter_source. If omitted, a fresh mcp://note/<uuid> is generated. Names without a scheme are auto-prefixed with mcp:// so the source is visually distinct from real filesystem paths in list_sources."
                        },
                        "file_type": {
                            "type": "string",
                            "enum": ["md", "markdown", "txt", "json", "html", "note", "code"],
                            "default": "md",
                            "description": "Format hint, exposed via filter_file_type on later searches."
                        },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Free-form tags applied to the document. Short slug-like values read cleanly (e.g., 'design', 'meeting-notes'). Cap of 32 tags per call."
                        },
                        "collection_name": {
                            "type": "string",
                            "description": "Add this document to the named collection. Auto-creates the collection if it does not yet exist."
                        },
                        "dry_run": {
                            "type": "boolean",
                            "default": false,
                            "description": "Validate inputs and report what would happen without writing. Useful before committing a large paste."
                        }
                    }
                }
            },
            {
                "name": "create_collection",
                "description": "Create a named collection (a subset of the vault you can filter searches to). Returns the collection's id and name. Idempotent: re-creating an existing name (case-insensitive) returns the existing id rather than failing. Useful when you plan to add a batch of related items and want a stable container ready first.",
                "inputSchema": {
                    "type": "object",
                    "required": ["name"],
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Human-readable collection label (e.g., 'Work', 'Research', '2026-Q2-planning'). 1 to 64 characters after trimming."
                        }
                    }
                }
            },
            {
                "name": "assign_to_collection",
                "description": "Add existing documents (by document_id) to a named collection. Pair with search_knowledge or list_sources to organize material the user wants to group together. The collection is auto-created if missing. Idempotent: re-assigning an already-member document is a no-op. Cap of 200 document_ids per call.",
                "inputSchema": {
                    "type": "object",
                    "required": ["collection_name", "document_ids"],
                    "properties": {
                        "collection_name": {
                            "type": "string",
                            "description": "Target collection (auto-created if it does not exist)."
                        },
                        "document_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of document_ids returned by search_knowledge or list_sources. Unknown ids are silently dropped; the response reports the actual count added."
                        }
                    }
                }
            }
        ]
    })
}

pub async fn handle_request(request: &JsonRpcRequest, db: &Database, embedder: &Embedder) -> Value {
    match request.method.as_str() {
        "initialize" => json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "satchel",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),

        "notifications/initialized" => Value::Null,

        "tools/list" => tool_definitions(),

        "tools/call" => {
            let params = match request.params.as_ref() {
                Some(p) => p,
                None => return tool_error("Missing params"),
            };
            let tool_name = params["name"].as_str().unwrap_or("");
            let args = &params["arguments"];

            match tool_name {
                "search_knowledge" => handle_search(args, db, embedder).await,
                "list_sources" => handle_list_sources(args, db),
                "get_document" => handle_get_document(args, db),
                "list_tags" => handle_list_tags(db),
                "vault_stats" => handle_vault_stats(db),
                "list_collections" => handle_list_collections(db),
                "get_chunk_context" => handle_get_chunk_context(args, db),
                "add_to_vault" => handle_add_to_vault(args, db, embedder),
                "create_collection" => handle_create_collection(args, db),
                "assign_to_collection" => handle_assign_to_collection(args, db),
                _ => tool_error(&format!("Unknown tool: {tool_name}")),
            }
        }

        _ => json!({
            "error": {
                "code": -32601,
                "message": format!("Method not found: {}", request.method)
            }
        }),
    }
}

fn tool_error(msg: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": msg }],
        "isError": true
    })
}

fn tool_text(text: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }]
    })
}

async fn handle_search(args: &Value, db: &Database, embedder: &Embedder) -> Value {
    let query = args["query"].as_str().unwrap_or("");
    let top_k = args["top_k"].as_u64().unwrap_or(5).min(20) as usize;
    let filter_source = args["filter_source"].as_str();
    let filter_tags: Option<Vec<&str>> = args["filter_tags"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect());

    // Accept either an explicit numeric id or a friendly name. Resolving by
    // name here keeps the AI-facing surface ergonomic — agents almost always
    // know the collection name from list_sources / vault_stats output, not
    // its database id.
    let filter_collection: Option<i64> = match args["collection_id"].as_i64() {
        Some(id) => Some(id),
        None => match args["collection_name"].as_str() {
            Some(name) if !name.is_empty() => match db.list_collections() {
                Ok(cs) => match cs.iter().find(|c| c.name == name) {
                    Some(c) => Some(c.id),
                    None => return tool_error(&format!("No collection named {name:?}")),
                },
                Err(e) => return tool_error(&format!("Collections lookup error: {e}")),
            },
            _ => None,
        },
    };

    let query_embedding = match embedder.embed(query) {
        Ok(emb) => emb,
        Err(e) => return tool_error(&format!("Embedding error: {e}")),
    };

    let filter_file_type = args["filter_file_type"].as_str();
    let tag_refs: Option<Vec<&str>> = filter_tags.as_ref().map(|v| v.to_vec());
    let page = match db.search(
        &query_embedding,
        query,
        top_k,
        0,
        crate::rag::SearchOptions {
            filter_source,
            filter_tags: tag_refs.as_deref(),
            filter_collection,
            filter_file_type,
        },
    ) {
        Ok(r) => r,
        Err(e) => return tool_error(&format!("Search error: {e}")),
    };

    if page.results.is_empty() {
        return tool_text("No results found.");
    }

    let mut text = String::new();
    if page.total > page.results.len() {
        text.push_str(&format!(
            "Showing top {} of {} matches.\n\n",
            page.results.len(),
            page.total
        ));
    }
    for (i, result) in page.results.iter().enumerate() {
        text.push_str(&format!(
            "--- Result {} (score: {:.3}) ---\nSource: {}\nchunk_id: {}\n{}\n\n",
            i + 1,
            result.score,
            result.source,
            result.chunk_id,
            result.text
        ));
    }

    tool_text(&text)
}

fn handle_get_chunk_context(args: &Value, db: &Database) -> Value {
    let chunk_id = args["chunk_id"].as_str().unwrap_or("");
    if chunk_id.is_empty() {
        return tool_error("`chunk_id` is required");
    }
    // Cap window sizes to keep responses bounded. 20 each direction is
    // already 41 chunks of context; more than enough for any chat
    // thread or paragraph fetch and well under typical context limits.
    let before = args["before"].as_u64().unwrap_or(2).min(20) as usize;
    let after = args["after"].as_u64().unwrap_or(2).min(20) as usize;

    match db.get_chunk_context(chunk_id, before, after) {
        Ok(chunks) if chunks.is_empty() => {
            tool_text(&format!("No chunks found for chunk_id={chunk_id:?}"))
        }
        Ok(chunks) => {
            let first = chunks.first().expect("non-empty checked above");
            let last = chunks.last().expect("non-empty checked above");
            let mut out = format!(
                "Source: {}\nReturned {} chunks (chunk_index {}..={}):\n\n",
                first.source_path,
                chunks.len(),
                first.chunk_index,
                last.chunk_index,
            );
            for c in &chunks {
                let marker = if c.chunk_id == chunk_id {
                    " (center)"
                } else {
                    ""
                };
                out.push_str(&format!(
                    "--- chunk_index {}{} (chunk_id: {}) ---\n{}\n\n",
                    c.chunk_index, marker, c.chunk_id, c.text
                ));
            }
            tool_text(&out)
        }
        Err(e) => tool_error(&format!("Error: {e}")),
    }
}

fn handle_list_sources(args: &Value, db: &Database) -> Value {
    let filter_type = args["filter_type"].as_str();
    let filter_path = args["q"].as_str();
    let sort_by = args["sort_by"].as_str().unwrap_or("name");
    // Cap at 100 — beyond that the response gets unwieldy for an AI client.
    let limit = args["limit"].as_u64().unwrap_or(100).min(500) as usize;
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;

    match db.list_sources(filter_type, filter_path, sort_by, limit, offset, None) {
        Ok(page) if page.sources.is_empty() => {
            tool_text("No documents ingested yet. Use `satchel ingest <path>` to add files.")
        }
        Ok(page) => {
            let body = page
                .sources
                .iter()
                .map(|s| {
                    if s.record_count > 1 {
                        format!(
                            "{} ({} records, {} chunks, .{})",
                            s.path, s.record_count, s.chunk_count, s.file_type
                        )
                    } else {
                        format!("{} ({} chunks, .{})", s.path, s.chunk_count, s.file_type)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            let header = if page.total > page.sources.len() {
                format!(
                    "Showing {} of {} sources (offset={}). Pass {{\"offset\": {}}} for more.\n\n",
                    page.sources.len(),
                    page.total,
                    page.offset,
                    page.offset + page.sources.len()
                )
            } else {
                String::new()
            };
            tool_text(&format!("{header}{body}"))
        }
        Err(e) => tool_error(&format!("Error: {e}")),
    }
}

fn handle_get_document(args: &Value, db: &Database) -> Value {
    let source = args["source"].as_str().unwrap_or("");
    match db.get_full_document(source) {
        Ok(text) => tool_text(&text),
        Err(e) => tool_error(&format!("Error: {e}")),
    }
}

fn handle_list_tags(db: &Database) -> Value {
    match db.list_tags() {
        Ok(tags) if tags.is_empty() => tool_text("No tags defined."),
        Ok(tags) => {
            let text = tags
                .iter()
                .map(|(tag, count)| format!("{tag} ({count} docs)"))
                .collect::<Vec<_>>()
                .join("\n");
            tool_text(&text)
        }
        Err(e) => tool_error(&format!("Error: {e}")),
    }
}

fn handle_vault_stats(db: &Database) -> Value {
    match db.stats() {
        Ok(stats) => tool_text(&format!(
            "Vault Statistics:\n  Documents: {}\n  Chunks: {}\n  Embedding dims: {}\n  DB size: {}",
            stats.document_count, stats.chunk_count, stats.embedding_dims, stats.db_size_human
        )),
        Err(e) => tool_error(&format!("Error: {e}")),
    }
}

fn handle_list_collections(db: &Database) -> Value {
    match db.list_collections() {
        Ok(cs) if cs.is_empty() => tool_text(
            "No collections defined. Create one in the web UI's Documents tab, then pass `collection_name` to `search_knowledge` to scope queries.",
        ),
        Ok(cs) => {
            let text = cs
                .iter()
                .map(|c| format!("{} (id={}, {} docs)", c.name, c.id, c.document_count))
                .collect::<Vec<_>>()
                .join("\n");
            tool_text(&text)
        }
        Err(e) => tool_error(&format!("Error: {e}")),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Write tools (v2.8.0): add_to_vault, create_collection, assign_to_collection.
//
// All writes go through the same chunk + embed + insert pipeline as
// file-based ingest, so MCP-added notes behave identically in search,
// list_sources, get_chunk_context, and collections. Hardened against
// the obvious failure modes:
//
//   - embedder-gone: refuse up-front (same as REST /api/ingest and CLI).
//   - empty content: 400-equivalent error before any DB write.
//   - oversized content: 5 MB cap. Bigger payloads belong on disk +
//     `satchel ingest <path>`.
//   - dedup: SHA-256 content hash; a re-add returns the existing
//     document_id and still honors collection_name and tags so
//     re-adding into a different collection actually populates the
//     new collection (same semantic as ingest_file's dedup-skip path).
//   - dry_run: validate only, never write.
//   - source: defaults to `mcp://note/<uuid>`; bare names get an
//     `mcp://` prefix so MCP-added sources stand out from real
//     filesystem paths in list_sources.
//
// Audit: every successful write emits a tracing::info! with doc_id,
// size, and source so the user can grep the server's stderr to see
// what their agent has been saving.
// ─────────────────────────────────────────────────────────────────────────

/// Hard cap on add_to_vault content size. 50 MB covers genuinely
/// large pastes (a whole book, a thick PDF's extracted text, a
/// multi-day chat export's text fields) while staying safely under
/// the HTTP body limit applied at the router layer (64 MB) and
/// commodity-machine memory. Bigger payloads should land on disk and
/// use `satchel ingest <path>`, where the work runs in a tracked
/// background job rather than synchronously blocking a single MCP
/// tool call. Note: embedding cost scales linearly, so a full 50 MB
/// paste will tie up the tool call for a few minutes on a typical
/// laptop CPU; the model should warn the user before committing one.
const ADD_TO_VAULT_MAX_BYTES: usize = 50 * 1024 * 1024;

/// Cap on tag count per add_to_vault call. Beyond this, the agent is
/// almost certainly hallucinating a taxonomy; reject so the user does
/// not end up with thousands of one-document tags.
const ADD_TO_VAULT_MAX_TAGS: usize = 32;

/// Cap on document_ids per assign_to_collection call. Keeps a single
/// bulk-organize call within sane bounds.
const ASSIGN_TO_COLLECTION_MAX_IDS: usize = 200;

/// Supported file_type values for add_to_vault. Keep in sync with the
/// tool's inputSchema `enum`. Reject anything else so unsupported file
/// types (pdf, docx, csv as raw blob) cannot sneak in this way; users
/// who want those should ingest via the file path.
fn is_supported_mcp_file_type(t: &str) -> bool {
    matches!(
        t,
        "md" | "markdown" | "txt" | "json" | "html" | "note" | "code"
    )
}

/// Resolve a user-supplied `source` to the canonical form we store. A
/// bare relative name like `journal/2026` gets an `mcp://` prefix so
/// it is visually distinct from on-disk paths in list_sources. A
/// fully-qualified name with a scheme (`mcp://...`, `file:///...`,
/// `note://...`) is taken as-is. Empty/missing input yields a fresh
/// `mcp://note/<short-uuid>` so every doc has a stable identifier.
fn canonicalize_mcp_source(raw: Option<&str>) -> String {
    let trimmed = raw.map(str::trim).unwrap_or_default();
    if trimmed.is_empty() {
        let id = uuid::Uuid::new_v4().to_string();
        let short = &id[..8];
        return format!("mcp://note/{short}");
    }
    if trimmed.contains("://") {
        return trimmed.to_string();
    }
    format!("mcp://{trimmed}")
}

/// Derive a title when the caller did not supply one. First non-empty
/// line of content, trimmed to 80 chars with an ellipsis on truncation.
/// Falls back to "(untitled)" for pathological all-whitespace inputs
/// (which add_to_vault would reject earlier anyway; defensive only).
fn derive_title_from_content(content: &str) -> String {
    let first_line = content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("(untitled)");
    if first_line.chars().count() <= 80 {
        first_line.to_string()
    } else {
        let mut out: String = first_line.chars().take(77).collect();
        out.push_str("...");
        out
    }
}

fn handle_add_to_vault(args: &Value, db: &Database, embedder: &Embedder) -> Value {
    // --- Validate up-front, before touching the DB or the embedder. ---
    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return tool_error("`content` is required and must be a string"),
    };
    if content.trim().is_empty() {
        return tool_error("`content` must not be empty");
    }
    if content.len() > ADD_TO_VAULT_MAX_BYTES {
        return tool_error(&format!(
            "`content` is {} bytes; max is {} bytes ({} MB). For larger payloads, save the file and use `satchel ingest <path>`.",
            content.len(),
            ADD_TO_VAULT_MAX_BYTES,
            ADD_TO_VAULT_MAX_BYTES / (1024 * 1024),
        ));
    }

    let file_type = args
        .get("file_type")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("md");
    if !is_supported_mcp_file_type(file_type) {
        return tool_error(&format!(
            "`file_type` {file_type:?} is not supported via MCP. Use one of: md, markdown, txt, json, html, note, code."
        ));
    }

    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    if tags.len() > ADD_TO_VAULT_MAX_TAGS {
        return tool_error(&format!(
            "{} tags supplied; max is {}.",
            tags.len(),
            ADD_TO_VAULT_MAX_TAGS
        ));
    }

    let source_path = canonicalize_mcp_source(args.get("source").and_then(|v| v.as_str()));
    let title_arg = args
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let title = title_arg.unwrap_or_else(|| derive_title_from_content(content));

    let collection_name = args
        .get("collection_name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let dry_run = args
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Chunk preview (same chunker as file ingest, so chunk counts match
    // what the user would see ingesting an equivalent file).
    let chunks_preview = crate::ingest::chunk_text(content, 512, 64);

    if dry_run {
        let coll_line = match &collection_name {
            Some(n) => format!("  collection:    {n} (would be created if missing)\n"),
            None => String::new(),
        };
        let tag_line = if tags.is_empty() {
            String::new()
        } else {
            format!("  tags:          {}\n", tags.join(", "))
        };
        return tool_text(&format!(
            "Would save (dry_run=true; no writes performed):\n  source:        {source}\n  title:         {title}\n  file_type:     {file_type}\n  size:          {bytes} bytes\n  chunks:        {chunks}\n{coll}{tags}",
            source = source_path,
            bytes = content.len(),
            chunks = chunks_preview.len(),
            coll = coll_line,
            tags = tag_line,
        ));
    }

    // --- Embedder gate. Same ordering as REST /api/ingest and CLI: ---
    // refuse BEFORE collection_name auto-create so a no-model run does
    // not leave an orphan empty collection.
    if !embedder.is_available() {
        return tool_error(
            "embedding model unavailable; cannot save. Run ./scripts/download-model.sh or rebuild satchel with --features embed-model.",
        );
    }

    // Hash + dedup check. We hash on (source_path, content) so two
    // distinct sources with identical bodies are NOT collapsed — that
    // matches user intent ("save this twice under different labels")
    // while a true repeat-add at the same source is correctly dedup'd.
    use sha2::{Digest, Sha256};
    let sha256 = {
        let mut h = Sha256::new();
        h.update(source_path.as_bytes());
        h.update(b"\0");
        h.update(content.as_bytes());
        format!("{:x}", h.finalize())
    };

    let existing = match db.document_id_by_hash(&sha256) {
        Ok(opt) => opt,
        Err(e) => return tool_error(&format!("hash lookup error: {e}")),
    };

    // Resolve (or create) the collection AFTER the embedder gate but
    // BEFORE we commit the doc, so a single create failure does not
    // strand a half-inserted document.
    let collection_id = match collection_name.as_deref() {
        Some(name) => match resolve_or_create_collection(db, name) {
            Ok(id) => Some(id),
            Err(e) => return tool_error(&format!("collection: {e}")),
        },
        None => None,
    };

    if let Some(existing_id) = existing {
        // Dedup hit. Still apply the requested collection assignment +
        // tags so the user can re-issue add_to_vault to attach a known
        // doc to a new collection or add tags to it.
        if let Some(cid) = collection_id {
            if let Err(e) = db.collection_add_documents(cid, std::slice::from_ref(&existing_id)) {
                return tool_error(&format!("collection_add_documents: {e}"));
            }
        }
        for tag in &tags {
            if let Err(e) = db.add_tag(&existing_id, tag) {
                return tool_error(&format!("add_tag: {e}"));
            }
        }
        tracing::info!(
            doc_id = %existing_id,
            source = %source_path,
            bytes = content.len(),
            dedup = true,
            "mcp add_to_vault: dedup hit"
        );
        return tool_text(&format_add_result(
            &existing_id,
            &source_path,
            chunks_preview.len(),
            collection_name.as_deref(),
            &tags,
            true,
        ));
    }

    // Fresh insert.
    let doc_id = uuid::Uuid::new_v4().to_string();
    if let Err(e) = db.insert_document(
        &doc_id,
        &source_path,
        file_type,
        Some(&title),
        content,
        &sha256,
    ) {
        return tool_error(&format!("insert_document: {e}"));
    }

    if let Some(cid) = collection_id {
        if let Err(e) = db.collection_add_documents(cid, std::slice::from_ref(&doc_id)) {
            return tool_error(&format!("collection_add_documents: {e}"));
        }
    }
    for tag in &tags {
        if let Err(e) = db.add_tag(&doc_id, tag) {
            return tool_error(&format!("add_tag: {e}"));
        }
    }

    // Chunk + embed + persist. Each chunk failure rolls back nothing
    // (we are not in a transaction; chunks are independent rows), but
    // we surface the first failure and stop so the caller knows the
    // doc is partial. This matches existing ingest_file behavior.
    let mut chunks_written = 0usize;
    for (i, chunk) in chunks_preview.iter().enumerate() {
        let embedding = match embedder.embed_with_info(&chunk.text) {
            Ok(e) => e,
            Err(e) => {
                return tool_error(&format!(
                    "embed chunk {i}: {e} (document {doc_id} created with {chunks_written}/{total} chunks; remaining chunks are not indexed)",
                    total = chunks_preview.len()
                ));
            }
        };
        if let Err(e) = db.insert_chunk(
            &format!("{doc_id}:{i}"),
            &doc_id,
            i,
            &chunk.text,
            embedding.token_count,
            chunk.char_start,
            chunk.char_end,
            &embedding.vector,
        ) {
            return tool_error(&format!("insert_chunk {i}: {e}"));
        }
        chunks_written += 1;
    }

    tracing::info!(
        doc_id = %doc_id,
        source = %source_path,
        bytes = content.len(),
        chunks = chunks_written,
        "mcp add_to_vault: wrote document"
    );

    tool_text(&format_add_result(
        &doc_id,
        &source_path,
        chunks_written,
        collection_name.as_deref(),
        &tags,
        false,
    ))
}

fn format_add_result(
    doc_id: &str,
    source: &str,
    chunks: usize,
    collection: Option<&str>,
    tags: &[String],
    dedup: bool,
) -> String {
    let mut out = if dedup {
        String::from("Already in the vault (deduplicated by content hash).\n")
    } else {
        String::from("Saved.\n")
    };
    out.push_str(&format!("  document_id:   {doc_id}\n"));
    out.push_str(&format!("  source:        {source}\n"));
    out.push_str(&format!("  chunks:        {chunks}\n"));
    if let Some(c) = collection {
        let label = if dedup {
            "collection (added)"
        } else {
            "collection"
        };
        out.push_str(&format!("  {label}:    {c}\n"));
    }
    if !tags.is_empty() {
        let label = if dedup { "tags (added)" } else { "tags" };
        out.push_str(&format!("  {label}:           {}\n", tags.join(", ")));
    }
    out
}

/// Idempotent create-or-resolve helper used by add_to_vault,
/// create_collection, and assign_to_collection. Mirrors the
/// `resolve_or_create_collection` helpers on the CLI and the HTTP
/// server: case-insensitive name match against existing collections,
/// fall back to create when no match.
fn resolve_or_create_collection(db: &Database, name: &str) -> anyhow::Result<i64> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        anyhow::bail!("collection name must not be empty");
    }
    for c in db.list_collections()? {
        if c.name.eq_ignore_ascii_case(trimmed) {
            return Ok(c.id);
        }
    }
    db.create_collection(trimmed)
}

fn handle_create_collection(args: &Value, db: &Database) -> Value {
    let name = match args.get("name").and_then(|v| v.as_str()) {
        Some(s) => s.trim(),
        None => return tool_error("`name` is required and must be a string"),
    };
    if name.is_empty() {
        return tool_error("`name` must not be empty");
    }
    if name.chars().count() > 64 {
        return tool_error("`name` must be 64 characters or fewer");
    }
    match resolve_or_create_collection(db, name) {
        Ok(id) => tool_text(&format!(
            "Collection ready.\n  id:    {id}\n  name:  {name}\n"
        )),
        Err(e) => tool_error(&format!("create_collection: {e}")),
    }
}

fn handle_assign_to_collection(args: &Value, db: &Database) -> Value {
    let name = match args.get("collection_name").and_then(|v| v.as_str()) {
        Some(s) => s.trim(),
        None => return tool_error("`collection_name` is required and must be a string"),
    };
    if name.is_empty() {
        return tool_error("`collection_name` must not be empty");
    }

    let ids: Vec<String> = match args.get("document_ids").and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect(),
        None => {
            return tool_error(
                "`document_ids` is required and must be a non-empty array of strings",
            )
        }
    };
    if ids.is_empty() {
        return tool_error("`document_ids` must contain at least one non-empty string");
    }
    if ids.len() > ASSIGN_TO_COLLECTION_MAX_IDS {
        return tool_error(&format!(
            "{} document_ids supplied; max is {} per call.",
            ids.len(),
            ASSIGN_TO_COLLECTION_MAX_IDS
        ));
    }

    let collection_id = match resolve_or_create_collection(db, name) {
        Ok(id) => id,
        Err(e) => return tool_error(&format!("collection: {e}")),
    };

    // Pre-filter to known document_ids. collection_add_documents is
    // FK-bound and would error on the first unknown id; the MCP
    // contract says unknown ids are silently dropped + reported in
    // the response so agents can pass a "best-effort" list of
    // candidates without having to verify each id first.
    let known_ids = match db.filter_existing_document_ids(&ids) {
        Ok(v) => v,
        Err(e) => return tool_error(&format!("filter_existing_document_ids: {e}")),
    };
    let unknown_count = ids.len() - known_ids.len();

    match db.collection_add_documents(collection_id, &known_ids) {
        Ok(added) => {
            // Known but not added => already a member (INSERT OR IGNORE
            // saw a conflict).
            let already_member = known_ids.len() - added;
            tracing::info!(
                collection_id,
                collection = name,
                requested = ids.len(),
                added,
                already_member,
                unknown = unknown_count,
                "mcp assign_to_collection"
            );
            tool_text(&format!(
                "Assigned to collection.\n  collection:    {name} (id={collection_id})\n  requested:     {req}\n  added:         {added}\n  already there: {already_member}\n  unknown id:    {unknown_count}\n",
                req = ids.len(),
            ))
        }
        Err(e) => tool_error(&format!("collection_add_documents: {e}")),
    }
}

pub fn print_client_config(client: &str, _vault_path: &Path) -> Result<()> {
    let bin = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/path/to/satchel".to_string());

    match client {
        "claude-desktop" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "mcpServers": {
                        "satchel": {
                            "command": &bin,
                            "args": ["serve"]
                        }
                    }
                }))?
            );
        }
        "claude-code" => {
            println!("claude mcp add satchel -- {bin} serve");
        }
        "cursor" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "mcpServers": {
                        "satchel": {
                            "command": &bin,
                            "args": ["serve"]
                        }
                    }
                }))?
            );
        }
        "browser" => {
            println!("Run satchel with no arguments to start the web UI:");
            println!("  {bin}");
            println!();
            println!("Endpoints:");
            println!("  Web UI:       http://localhost:7428");
            println!("  MCP endpoint: http://localhost:7428/mcp");
            println!("  REST API:     http://localhost:7428/api/search");
        }
        _ => {
            println!("# SATCHEL MCP Server");
            println!("# Web UI + HTTP: {bin}");
            println!("# Stdio (for MCP clients): {bin} serve");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::Embedder;
    use crate::rag::Database;

    fn req(method: &str, id: Option<Value>, params: Option<Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        }
    }

    fn db() -> Database {
        Database::open_memory().unwrap()
    }

    #[test]
    fn test_tool_definitions_valid_json() {
        let defs = tool_definitions();
        assert!(defs["tools"].is_array());
    }

    #[test]
    fn test_tool_definitions_has_expected_count() {
        let defs = tool_definitions();
        assert_eq!(defs["tools"].as_array().unwrap().len(), 10);
    }

    #[test]
    fn test_tool_definitions_names() {
        let defs = tool_definitions();
        let names: Vec<&str> = defs["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        // Read tools.
        assert!(names.contains(&"search_knowledge"));
        assert!(names.contains(&"list_sources"));
        assert!(names.contains(&"get_document"));
        assert!(names.contains(&"list_tags"));
        assert!(names.contains(&"vault_stats"));
        assert!(names.contains(&"list_collections"));
        assert!(names.contains(&"get_chunk_context"));
        // Write tools (v2.8.0).
        assert!(names.contains(&"add_to_vault"));
        assert!(names.contains(&"create_collection"));
        assert!(names.contains(&"assign_to_collection"));
    }

    #[test]
    fn test_tool_definitions_required_fields() {
        let defs = tool_definitions();
        for tool in defs["tools"].as_array().unwrap() {
            assert!(tool["name"].is_string());
            assert!(tool["description"].is_string());
            assert!(tool["inputSchema"].is_object());
        }
    }

    #[tokio::test]
    async fn test_handle_initialize() {
        let result = handle_request(
            &req("initialize", Some(json!(1)), Some(json!({}))),
            &db(),
            &Embedder::fixed(384),
        )
        .await;
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "satchel");
    }

    #[tokio::test]
    async fn test_handle_notifications_initialized() {
        let result = handle_request(
            &req("notifications/initialized", None, None),
            &db(),
            &Embedder::fixed(384),
        )
        .await;
        assert_eq!(result, Value::Null);
    }

    #[tokio::test]
    async fn test_handle_tools_list() {
        let result = handle_request(
            &req("tools/list", Some(json!(1)), None),
            &db(),
            &Embedder::fixed(384),
        )
        .await;
        assert_eq!(result["tools"].as_array().unwrap().len(), 10);
    }

    #[tokio::test]
    async fn test_handle_unknown_method() {
        let result = handle_request(
            &req("bogus/method", Some(json!(1)), None),
            &db(),
            &Embedder::fixed(384),
        )
        .await;
        assert_eq!(result["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn test_handle_tools_call_vault_stats() {
        let result = handle_request(
            &req(
                "tools/call",
                Some(json!(1)),
                Some(json!({"name": "vault_stats", "arguments": {}})),
            ),
            &db(),
            &Embedder::fixed(384),
        )
        .await;
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Documents: 0"));
    }

    #[tokio::test]
    async fn test_handle_tools_call_list_sources_empty() {
        let result = handle_request(
            &req(
                "tools/call",
                Some(json!(1)),
                Some(json!({"name": "list_sources", "arguments": {}})),
            ),
            &db(),
            &Embedder::fixed(384),
        )
        .await;
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("No documents"));
    }

    #[tokio::test]
    async fn test_handle_tools_call_list_tags_empty() {
        let result = handle_request(
            &req(
                "tools/call",
                Some(json!(1)),
                Some(json!({"name": "list_tags", "arguments": {}})),
            ),
            &db(),
            &Embedder::fixed(384),
        )
        .await;
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("No tags"));
    }

    #[tokio::test]
    async fn test_handle_tools_call_get_document_missing() {
        let result = handle_request(
            &req(
                "tools/call",
                Some(json!(1)),
                Some(json!({"name": "get_document", "arguments": {"source": "nope"}})),
            ),
            &db(),
            &Embedder::fixed(384),
        )
        .await;
        assert_eq!(result["isError"], true);
    }

    #[tokio::test]
    async fn test_handle_tools_call_search_no_embedder() {
        let result = handle_request(
            &req(
                "tools/call",
                Some(json!(1)),
                Some(json!({"name": "search_knowledge", "arguments": {"query": "test"}})),
            ),
            &db(),
            &Embedder::unavailable(),
        )
        .await;
        assert_eq!(result["isError"], true);
    }

    #[tokio::test]
    async fn test_handle_tools_call_unknown_tool() {
        let result = handle_request(
            &req(
                "tools/call",
                Some(json!(1)),
                Some(json!({"name": "nonexistent_tool", "arguments": {}})),
            ),
            &db(),
            &Embedder::fixed(384),
        )
        .await;
        assert_eq!(result["isError"], true);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn test_handle_tools_call_missing_params() {
        let result = handle_request(
            &req("tools/call", Some(json!(1)), None),
            &db(),
            &Embedder::fixed(384),
        )
        .await;
        assert_eq!(result["isError"], true);
    }
}
