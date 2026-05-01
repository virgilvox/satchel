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
use crate::jobs::{JobRegistry, JobStatus};
use crate::mcp;
use crate::rag::Database;

// The web UI is built from `web/` (Svelte 5 + Vite) into a single
// self-contained HTML file via `vite-plugin-singlefile`. The build artifact
// is committed so `cargo build` works without a JS toolchain. Run
// `bun run build` (or `npm run build`) inside `web/` after editing the UI.
const UI_HTML: &str = include_str!("../web/dist/index.html");

pub struct AppState {
    pub db: Arc<Database>,
    pub embedder: Arc<Embedder>,
    pub jobs: Arc<JobRegistry>,
}

pub fn build_router(db: Database, embedder: Embedder) -> Router {
    let state = Arc::new(AppState {
        db: Arc::new(db),
        embedder: Arc::new(embedder),
        jobs: Arc::new(JobRegistry::new()),
    });

    Router::new()
        .route("/", get(ui_handler))
        .route("/mcp", post(mcp_handler))
        .route("/api/status", get(api_status))
        .route("/api/sources", get(api_sources).delete(api_delete_sources))
        .route("/api/search", post(api_search))
        .route("/api/document", get(api_document))
        .route("/api/tags", get(api_tags))
        .route("/api/clear", post(api_clear))
        .route("/api/config/:client", get(api_config))
        .route("/api/browse", get(api_browse))
        .route("/api/ingest", post(api_ingest))
        .route("/api/jobs", get(api_jobs))
        .route("/api/jobs/:id", get(api_job_get))
        .route("/api/conversation", get(api_conversation))
        .route("/api/types", get(api_types))
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .with_state(state)
}

pub async fn serve(
    db: Database,
    embedder: Embedder,
    port: u16,
    open_in_browser: bool,
) -> anyhow::Result<()> {
    let app = build_router(db, embedder);

    let addr = format!("127.0.0.1:{port}");
    eprintln!("[satchel] Web UI:       http://{addr}");
    eprintln!("[satchel] MCP endpoint: http://{addr}/mcp");
    eprintln!("[satchel] REST API:     http://{addr}/api/");

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    if open_in_browser {
        let url = format!("http://{addr}");
        tokio::spawn(async move {
            // Tiny delay so axum is actually accepting before the browser hits.
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            if let Err(e) = open_url(&url) {
                eprintln!("[satchel] Could not open browser: {e}");
            }
        });
    }

    axum::serve(listener, app).await?;
    Ok(())
}

/// Best-effort cross-platform browser launcher. Returns the spawn error if
/// the helper executable wasn't found; we don't wait on the child.
fn open_url(url: &str) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(target_os = "windows") {
        // The empty "" is a placeholder for the title arg `start` consumes
        // when its first argument is quoted — guards against URLs with spaces.
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        return c
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ());
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
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
    /// Substring match on source_path; SQL wildcards in `q` are escaped.
    q: Option<String>,
    /// "name" (default) | "date" | "chunks" | "records"
    sort_by: Option<String>,
    /// Page size (default 50, max 1000).
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn api_sources(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SourcesQuery>,
) -> Json<Value> {
    let limit = q.limit.unwrap_or(50).min(1000).max(1);
    let offset = q.offset.unwrap_or(0);
    match state.db.list_sources(
        q.filter_type.as_deref(),
        q.q.as_deref(),
        q.sort_by.as_deref().unwrap_or("name"),
        limit,
        offset,
    ) {
        Ok(page) => Json(json!({
            "sources": page.sources,
            "total": page.total,
            "offset": page.offset,
            "limit": page.limit,
        })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct SearchRequest {
    query: String,
    /// Page size (default 20, capped at 100).
    top_k: Option<usize>,
    /// Pagination offset (default 0).
    offset: Option<usize>,
    filter_source: Option<String>,
}

async fn api_search(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SearchRequest>,
) -> Json<Value> {
    let limit = req.top_k.unwrap_or(20).min(100);
    let offset = req.offset.unwrap_or(0);

    let query_embedding = match state.embedder.embed(&req.query) {
        Ok(emb) => emb,
        Err(e) => return Json(json!({ "error": e.to_string() })),
    };

    match state.db.search(
        &query_embedding,
        &req.query,
        limit,
        offset,
        req.filter_source.as_deref(),
        None,
    ) {
        Ok(page) => Json(json!({
            "results": page.results,
            "total": page.total,
            "offset": page.offset,
            "limit": page.limit,
        })),
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

/// Resolve the user's home directory across platforms. Prefers `$HOME`
/// (set on macOS/Linux and on Windows when running under msys/cygwin),
/// falls back to `%USERPROFILE%` on native Windows.
fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

async fn api_browse(Query(q): Query<BrowseQuery>) -> Json<Value> {
    use std::path::PathBuf;
    let start: PathBuf = match q.path.as_deref() {
        Some(p) if !p.is_empty() => {
            if let Some(stripped) = p.strip_prefix("~/") {
                match home_dir() {
                    Some(h) => h.join(stripped),
                    None => return Json(json!({ "error": "could not resolve home directory" })),
                }
            } else if p == "~" {
                match home_dir() {
                    Some(h) => h,
                    None => return Json(json!({ "error": "could not resolve home directory" })),
                }
            } else {
                PathBuf::from(p)
            }
        }
        _ => home_dir().unwrap_or_else(|| PathBuf::from("/")),
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
        kb.cmp(ka)
            .then_with(|| na.to_lowercase().cmp(&nb.to_lowercase()))
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
        match home_dir() {
            Some(h) => h.join(stripped),
            None => return Json(json!({ "error": "could not resolve home directory" })),
        }
    } else if raw == "~" {
        match home_dir() {
            Some(h) => h,
            None => return Json(json!({ "error": "could not resolve home directory" })),
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

    // Fail fast if there's no embedder — every record would fail otherwise
    // and the user would see "all N records failed" only after a long wait.
    if !state.embedder.is_available() {
        return Json(json!({
            "error": "embedding model unavailable; ingest would fail. Run \
                      ./scripts/download-model.sh or rebuild with --features embed-model"
        }));
    }

    // Register the job and return its id immediately. The actual ingest
    // runs in the background; the UI polls /api/jobs for live counters.
    let job_id = state.jobs.create(path.to_string_lossy().to_string());

    let db = state.db.clone();
    let embedder = state.embedder.clone();
    let jobs = state.jobs.clone();
    let job_for_progress = job_id.clone();
    let job_for_finish = job_id.clone();

    tokio::task::spawn_blocking(move || {
        jobs.update(&job_for_progress, |j| j.status = JobStatus::Running);
        let progress_jobs = jobs.clone();
        let progress_id = job_for_progress.clone();
        let progress = crate::ingest::Progress::callback(move |evt| {
            use crate::ingest::ProgressEvent;
            progress_jobs.update(&progress_id, |j| match evt {
                ProgressEvent::ArchiveDetected(name) => {
                    j.archive_kind = Some(name);
                }
                ProgressEvent::FileStarted(p) => {
                    j.files_seen += 1;
                    j.current_file = Some(p.to_string_lossy().to_string());
                }
                ProgressEvent::RecordAdded => {
                    j.records_added += 1;
                }
                ProgressEvent::RecordSkipped => {
                    j.records_skipped += 1;
                }
                ProgressEvent::RecordFailed => {
                    j.records_failed += 1;
                }
            });
        });

        // catch_unwind so a panic inside the ingest pipeline (mutex
        // poisoning, allocation failure, malformed data we didn't
        // anticipate) leaves the job marked Failed instead of stuck on
        // Running forever. AssertUnwindSafe is OK here: the values we
        // borrow into the closure are only mutated through Mutexes, and
        // we don't reuse them after a panic.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::ingest::ingest_path(&path, &db, &embedder, &config, &progress)
        }));

        jobs.update(&job_for_finish, |j| {
            j.current_file = None;
            j.finished_at = Some(chrono::Utc::now().to_rfc3339());
            match result {
                Ok(Ok(_)) => {
                    // Pipeline reports Ok but every record failed — usually a
                    // missing embedder. Treat as failure for the user's sake.
                    if j.records_added == 0 && j.records_failed > 0 {
                        j.status = JobStatus::Failed;
                        j.error = Some(format!(
                            "all {} records failed (check embedding model availability)",
                            j.records_failed
                        ));
                    } else {
                        j.status = JobStatus::Completed;
                    }
                }
                Ok(Err(e)) => {
                    j.status = JobStatus::Failed;
                    j.error = Some(e.to_string());
                }
                Err(panic) => {
                    j.status = JobStatus::Failed;
                    let msg = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "unknown panic".to_string());
                    j.error = Some(format!("ingest task panicked: {msg}"));
                }
            }
        });
    });

    Json(json!({ "job_id": job_id, "status": "pending" }))
}

async fn api_jobs(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "jobs": state.jobs.list() }))
}

#[derive(Deserialize)]
struct ConversationQuery {
    source: String,
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn api_conversation(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ConversationQuery>,
) -> Json<Value> {
    let limit = q.limit.unwrap_or(2000).min(10_000);
    let offset = q.offset.unwrap_or(0);
    match state.db.list_records_by_source(&q.source, limit, offset) {
        Ok((records, total)) => Json(json!({
            "source": q.source,
            "records": records,
            "total": total,
            "offset": offset,
            "limit": limit,
        })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

async fn api_types(State(state): State<Arc<AppState>>) -> Json<Value> {
    match state.db.list_file_types() {
        Ok(types) => Json(json!({
            "types": types
                .into_iter()
                .map(|(t, n)| json!({ "file_type": t, "source_count": n }))
                .collect::<Vec<_>>()
        })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

async fn api_job_get(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<Value> {
    match state.jobs.get(&id) {
        Some(j) => Json(json!({ "job": j })),
        None => Json(json!({ "error": format!("job not found: {id}") })),
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
