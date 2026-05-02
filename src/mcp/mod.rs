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
                            "description": "Filter results to a specific source file"
                        },
                        "filter_tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Filter results to chunks with any of these tags"
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

    let page = match db.search(
        &query_embedding,
        query,
        top_k,
        0,
        filter_source,
        filter_tags.as_deref(),
        filter_collection,
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
            "--- Result {} (score: {:.3}) ---\nSource: {}\n{}\n\n",
            i + 1,
            result.score,
            result.source,
            result.text
        ));
    }

    tool_text(&text)
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
    fn test_tool_definitions_has_six_tools() {
        let defs = tool_definitions();
        assert_eq!(defs["tools"].as_array().unwrap().len(), 6);
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
        assert!(names.contains(&"search_knowledge"));
        assert!(names.contains(&"list_sources"));
        assert!(names.contains(&"get_document"));
        assert!(names.contains(&"list_tags"));
        assert!(names.contains(&"vault_stats"));
        assert!(names.contains(&"list_collections"));
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
        assert_eq!(result["tools"].as_array().unwrap().len(), 6);
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
