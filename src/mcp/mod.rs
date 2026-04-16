pub mod stdio;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use anyhow::Result;

use crate::rag::Database;
use crate::embed::Embedder;

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
                "description": "Semantic search across the entire knowledge vault. Returns the most relevant chunks with source attribution.",
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
                        }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "list_sources",
                "description": "List all ingested documents with metadata (file type, chunk count, ingestion date).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "filter_type": {
                            "type": "string",
                            "description": "Filter by file extension (e.g., 'md', 'pdf')"
                        },
                        "sort_by": {
                            "type": "string",
                            "enum": ["name", "date", "chunks"],
                            "default": "name"
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
            }
        ]
    })
}

pub async fn handle_request(
    request: &JsonRpcRequest,
    db: &Database,
    embedder: &Embedder,
) -> Value {
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

    let query_embedding = match embedder.embed(query) {
        Ok(emb) => emb,
        Err(e) => return tool_error(&format!("Embedding error: {e}")),
    };

    let results = match db.search(
        &query_embedding,
        top_k,
        filter_source,
        filter_tags.as_deref(),
    ) {
        Ok(r) => r,
        Err(e) => return tool_error(&format!("Search error: {e}")),
    };

    if results.is_empty() {
        return tool_text("No results found.");
    }

    let mut text = String::new();
    for (i, result) in results.iter().enumerate() {
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
    let sort_by = args["sort_by"].as_str().unwrap_or("name");

    match db.list_sources(filter_type, sort_by) {
        Ok(sources) if sources.is_empty() => {
            tool_text("No documents ingested yet. Use `satchel ingest <path>` to add files.")
        }
        Ok(sources) => {
            let text = sources
                .iter()
                .map(|s| format!("{} ({} chunks, .{})", s.path, s.chunk_count, s.file_type))
                .collect::<Vec<_>>()
                .join("\n");
            tool_text(&text)
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

pub fn print_client_config(client: &str, vault_path: &Path) -> Result<()> {
    let vault_abs = std::fs::canonicalize(vault_path).unwrap_or_else(|_| vault_path.to_path_buf());
    let vault_str = vault_abs.display();

    match client {
        "claude-desktop" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "mcpServers": {
                        "satchel": {
                            "command": format!("{}/satchel", vault_str),
                            "args": ["serve", "--transport", "stdio", "--vault", &vault_str.to_string()]
                        }
                    }
                }))?
            );
        }
        "claude-code" => {
            println!(
                "claude mcp add satchel -- {vault_str}/satchel serve --transport stdio --vault {vault_str}"
            );
        }
        "cursor" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "mcpServers": {
                        "satchel": {
                            "command": format!("{}/satchel", vault_str),
                            "args": ["serve", "--transport", "stdio", "--vault", &vault_str.to_string()]
                        }
                    }
                }))?
            );
        }
        "browser" => {
            println!("Start SATCHEL with HTTP transport:");
            println!("  satchel serve --transport http --vault {vault_str}");
            println!();
            println!("Then connect browser AI clients:");
            println!("  MCP endpoint: http://localhost:7428/mcp");
            println!("  REST API:     http://localhost:7428/api/search");
            println!("  Web UI:       http://localhost:7428");
        }
        _ => {
            println!("# SATCHEL MCP Server");
            println!("# Stdio:  satchel serve --transport stdio --vault {vault_str}");
            println!("# HTTP:   satchel serve --transport http --port 7428 --vault {vault_str}");
        }
    }

    Ok(())
}
