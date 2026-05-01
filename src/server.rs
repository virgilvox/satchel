use axum::{
    extract::{Json, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

use crate::embed::Embedder;
use crate::mcp;
use crate::rag::Database;

const UI_HTML: &str = include_str!("../assets/ui.html");

pub struct AppState {
    pub db: Arc<Database>,
    pub embedder: Arc<Embedder>,
}

pub fn build_router(db: Database, embedder: Embedder) -> Router {
    let state = Arc::new(AppState {
        db: Arc::new(db),
        embedder: Arc::new(embedder),
    });

    Router::new()
        .route("/", get(ui_handler))
        .route("/mcp", post(mcp_handler))
        .route("/api/status", get(api_status))
        .route(
            "/api/sources",
            get(api_sources).delete(api_delete_sources),
        )
        .route("/api/search", post(api_search))
        .route("/api/document", get(api_document))
        .route("/api/tags", get(api_tags))
        .route("/api/clear", post(api_clear))
        .route("/api/config/:client", get(api_config))
        .route("/api/browse", get(api_browse))
        .route("/api/ingest", post(api_ingest))
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .with_state(state)
}

pub async fn serve(db: Database, embedder: Embedder, port: u16) -> anyhow::Result<()> {
    let app = build_router(db, embedder);

    let addr = format!("127.0.0.1:{port}");
    eprintln!("[satchel] Web UI:       http://{addr}");
    eprintln!("[satchel] MCP endpoint: http://{addr}/mcp");
    eprintln!("[satchel] REST API:     http://{addr}/api/");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ui_handler() -> Html<&'static str> {
    Html(UI_HTML)
}

async fn mcp_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<mcp::JsonRpcRequest>,
) -> impl IntoResponse {
    let result = mcp::handle_request(&request, &state.db, &state.embedder).await;

    if request.id.is_none() {
        return (StatusCode::ACCEPTED, HeaderMap::new(), "".to_string());
    }

    let response = mcp::JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: request.id,
        result: Some(result),
        error: None,
    };

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "application/json".parse().unwrap());

    if let Some(session_id) = headers.get("mcp-session-id") {
        resp_headers.insert("mcp-session-id", session_id.clone());
    }

    let body = serde_json::to_string(&response).unwrap_or_default();
    (StatusCode::OK, resp_headers, body)
}

async fn api_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    let stats = state.db.stats().ok();
    Json(json!({
        "status": "running",
        "version": env!("CARGO_PKG_VERSION"),
        "embedding_model": state.embedder.model_name(),
        "embedding_available": state.embedder.is_available(),
        "stats": stats.map(|s| json!({
            "documents": s.document_count,
            "chunks": s.chunk_count,
            "dimensions": s.embedding_dims,
            "db_size": s.db_size_human,
        }))
    }))
}

#[derive(Deserialize)]
struct SourcesQuery {
    filter_type: Option<String>,
    sort_by: Option<String>,
}

async fn api_sources(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SourcesQuery>,
) -> Json<Value> {
    match state.db.list_sources(
        q.filter_type.as_deref(),
        q.sort_by.as_deref().unwrap_or("name"),
    ) {
        Ok(sources) => Json(json!({ "sources": sources })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct SearchRequest {
    query: String,
    top_k: Option<usize>,
    filter_source: Option<String>,
}

async fn api_search(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SearchRequest>,
) -> Json<Value> {
    let top_k = req.top_k.unwrap_or(5).min(20);

    let query_embedding = match state.embedder.embed(&req.query) {
        Ok(emb) => emb,
        Err(e) => return Json(json!({ "error": e.to_string() })),
    };

    match state.db.search(
        &query_embedding,
        &req.query,
        top_k,
        req.filter_source.as_deref(),
        None,
    ) {
        Ok(results) => Json(json!({ "results": results })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct DocumentQuery {
    source: String,
}

async fn api_document(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DocumentQuery>,
) -> Json<Value> {
    match state.db.get_full_document(&q.source) {
        Ok(text) => Json(json!({ "text": text, "source": q.source })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

async fn api_tags(State(state): State<Arc<AppState>>) -> Json<Value> {
    match state.db.list_tags() {
        Ok(tags) => Json(json!({
            "tags": tags.iter().map(|(tag, count)| json!({
                "tag": tag,
                "count": count,
            })).collect::<Vec<_>>()
        })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct DeleteRequest {
    path: Option<String>,
    prefix: Option<String>,
    file_type: Option<String>,
    #[serde(default)]
    dry_run: bool,
}

async fn api_delete_sources(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DeleteRequest>,
) -> Json<Value> {
    let result = match (req.path, req.prefix, req.file_type) {
        (Some(p), None, None) => state.db.delete_by_path_exact(&p, req.dry_run),
        (None, Some(pre), None) => state.db.delete_by_path_prefix(&pre, req.dry_run),
        (None, None, Some(t)) => state.db.delete_by_file_type(&t, req.dry_run),
        _ => {
            return Json(json!({
                "error": "specify exactly one of: path, prefix, file_type"
            }))
        }
    };
    match result {
        Ok((docs, chunks)) => Json(json!({
            "deleted_documents": docs,
            "deleted_chunks": chunks,
            "dry_run": req.dry_run,
        })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct ClearRequest {
    #[serde(default)]
    confirm: bool,
    #[serde(default)]
    dry_run: bool,
}

async fn api_clear(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClearRequest>,
) -> Json<Value> {
    if !req.dry_run && !req.confirm {
        return Json(json!({
            "error": "destructive operation: pass {\"confirm\": true} or {\"dry_run\": true}"
        }));
    }
    match state.db.clear_all(req.dry_run) {
        Ok((docs, chunks)) => Json(json!({
            "deleted_documents": docs,
            "deleted_chunks": chunks,
            "dry_run": req.dry_run,
        })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct BrowseQuery {
    path: Option<String>,
}

async fn api_browse(Query(q): Query<BrowseQuery>) -> Json<Value> {
    use std::path::PathBuf;
    let start: PathBuf = match q.path.as_deref() {
        Some(p) if !p.is_empty() => {
            // Expand leading "~/" to home dir; otherwise treat as absolute.
            if let Some(stripped) = p.strip_prefix("~/") {
                match std::env::var_os("HOME") {
                    Some(h) => PathBuf::from(h).join(stripped),
                    None => return Json(json!({ "error": "HOME not set" })),
                }
            } else if p == "~" {
                match std::env::var_os("HOME") {
                    Some(h) => PathBuf::from(h),
                    None => return Json(json!({ "error": "HOME not set" })),
                }
            } else {
                PathBuf::from(p)
            }
        }
        _ => match std::env::var_os("HOME") {
            Some(h) => PathBuf::from(h),
            None => PathBuf::from("/"),
        },
    };

    if !start.exists() {
        return Json(json!({ "error": format!("not found: {}", start.display()) }));
    }
    let canonical = start.canonicalize().unwrap_or(start.clone());
    let read = match std::fs::read_dir(&canonical) {
        Ok(r) => r,
        Err(e) => return Json(json!({ "error": format!("read_dir: {e}") })),
    };
    let mut entries: Vec<Value> = Vec::new();
    for e in read.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        // Hide dotfiles by default — clutter for the user's home.
        if name.starts_with('.') {
            continue;
        }
        let kind = if p.is_dir() {
            "dir"
        } else if p.is_file() {
            "file"
        } else {
            continue;
        };
        entries.push(json!({
            "name": name,
            "kind": kind,
            "path": p.to_string_lossy(),
        }));
    }
    entries.sort_by(|a, b| {
        let ka = a["kind"].as_str().unwrap_or("");
        let kb = b["kind"].as_str().unwrap_or("");
        let na = a["name"].as_str().unwrap_or("");
        let nb = b["name"].as_str().unwrap_or("");
        // Directories first, then alphabetical.
        kb.cmp(ka).then_with(|| na.to_lowercase().cmp(&nb.to_lowercase()))
    });

    let parent = canonical.parent().map(|p| p.to_string_lossy().to_string());

    Json(json!({
        "path": canonical.to_string_lossy(),
        "parent": parent,
        "entries": entries,
    }))
}

#[derive(Deserialize)]
struct IngestRequest {
    path: String,
    #[serde(default)]
    chunk_size: Option<usize>,
    #[serde(default)]
    chunk_overlap: Option<usize>,
}

async fn api_ingest(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IngestRequest>,
) -> Json<Value> {
    use std::path::PathBuf;
    let raw = req.path.trim();
    let path: PathBuf = if let Some(stripped) = raw.strip_prefix("~/") {
        match std::env::var_os("HOME") {
            Some(h) => PathBuf::from(h).join(stripped),
            None => return Json(json!({ "error": "HOME not set" })),
        }
    } else {
        PathBuf::from(raw)
    };

    if !path.exists() {
        return Json(json!({ "error": format!("not found: {}", path.display()) }));
    }

    let config = crate::ingest::IngestConfig {
        chunk_size: req.chunk_size.unwrap_or(512),
        chunk_overlap: req.chunk_overlap.unwrap_or(64),
    };

    // Run on a blocking thread — ingestion is sync I/O + CPU-bound embedding.
    let db = state.db.clone();
    let embedder = state.embedder.clone();
    let result = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::try_current().ok();
        if let Some(handle) = rt {
            handle.block_on(crate::ingest::ingest_path(&path, &db, &embedder, &config))
        } else {
            // Fallback: build a small runtime.
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(crate::ingest::ingest_path(&path, &db, &embedder, &config))
        }
    })
    .await;

    match result {
        Ok(Ok(())) => match state.db.stats() {
            Ok(s) => Json(json!({
                "status": "ok",
                "documents": s.document_count,
                "chunks": s.chunk_count,
            })),
            Err(e) => Json(json!({ "error": e.to_string() })),
        },
        Ok(Err(e)) => Json(json!({ "error": e.to_string() })),
        Err(e) => Json(json!({ "error": format!("join error: {e}") })),
    }
}

async fn api_config(axum::extract::Path(client): axum::extract::Path<String>) -> Json<Value> {
    let config = match client.as_str() {
        "claude-desktop" => json!({
            "client": "Claude Desktop",
            "config": {
                "mcpServers": {
                    "satchel": {
                        "command": "/path/to/satchel",
                        "args": ["serve"]
                    }
                }
            },
            "instructions": "Add this to your Claude Desktop config. Replace /path/to/satchel with the actual path to the binary."
        }),
        "claude-code" => json!({
            "client": "Claude Code",
            "command": "claude mcp add satchel -- /path/to/satchel serve",
            "instructions": "Run this command in your terminal. Replace /path/to/satchel with the actual path."
        }),
        "cursor" => json!({
            "client": "Cursor",
            "config": {
                "mcpServers": {
                    "satchel": {
                        "command": "/path/to/satchel",
                        "args": ["serve"]
                    }
                }
            },
            "instructions": "Add this to your Cursor MCP config. Replace /path/to/satchel with the actual path."
        }),
        "browser" => json!({
            "client": "Browser AI (Claude.ai, ChatGPT)",
            "endpoint": "http://localhost:7428/mcp",
            "rest_api": "http://localhost:7428/api/search",
            "instructions": "SATCHEL is already running. Use the claude-mcp browser extension to bridge to claude.ai, or use the REST API directly."
        }),
        _ => json!({
            "client": "Generic",
            "stdio": "/path/to/satchel serve",
            "http": "/path/to/satchel  (runs HTTP server by default)"
        }),
    };

    Json(config)
}
