use axum::{
    extract::{DefaultBodyLimit, Json, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

use crate::anthropic::{self, AnthropicConfig};
use crate::embed::Embedder;
use crate::jobs::{JobRegistry, JobStatus};
use crate::mcp;
use crate::mcp_proxy::{self, McpServerEntry, McpServersConfig};
use crate::mdns::{self, MdnsConfig, Responder as MdnsResponder};
use crate::rag::{self, Database};
use crate::release::ReleaseCache;
use crate::tunnel::{TunnelConfig, TunnelManager, TunnelMode};
use axum::body::Body;
use axum::extract::Path as AxPath;
use axum::http::HeaderName;
use axum::response::Response;
use futures_util::StreamExt;
use std::path::PathBuf;

// The web UI is built from `web/` (Svelte 5 + Vite) into a single
// self-contained HTML file via `vite-plugin-singlefile`. The build artifact
// is committed so `cargo build` works without a JS toolchain. Run
// `bun run build` (or `npm run build`) inside `web/` after editing the UI.
const UI_HTML: &str = include_str!("../web/dist/index.html");

pub struct AppState {
    pub db: Arc<Database>,
    pub embedder: Arc<Embedder>,
    pub jobs: Arc<JobRegistry>,
    pub tunnel: TunnelManager,
    /// Port the HTTP server is bound to. The tunnel UI uses it as the
    /// default forwarding target so the user never has to type the port
    /// twice.
    pub port: u16,
    /// Vault directory that holds `tunnel.toml`. Different from the
    /// active vault path used for the DB; this is the parent that all
    /// vaults live under.
    pub vault_path: PathBuf,
    /// Cached GitHub-release probe. One hour TTL — keeps GitHub
    /// round-trips minimal even with several browser tabs open.
    pub release_cache: ReleaseCache,
    /// Active mDNS responder, when enabled. Wrapped in `Mutex<Option>`
    /// so the `/api/mdns` POST handler can swap it in/out at runtime
    /// without restarting the HTTP server.
    pub mdns: std::sync::Mutex<Option<MdnsResponder>>,
}

pub fn build_router(db: Database, embedder: Embedder, port: u16, vault_path: PathBuf) -> Router {
    // Start mDNS only when the persisted toggle is enabled (default
    // on) AND we have a real port to advertise. `port == 0` is the
    // test-router sentinel for "no real bind" — registering mDNS at
    // port 0 would advertise an unreachable service and the test suite
    // would also pay for one daemon-spawn per test. A failure here is
    // non-fatal: the HTTP server still binds and the Connect tab still
    // works via the LAN IP. We log the error so users on locked-down
    // networks see why satchel.local is not resolving.
    let mdns_cfg = MdnsConfig::load(&vault_path).unwrap_or_default();
    let mdns_initial = if mdns_cfg.enabled && port != 0 {
        match MdnsResponder::start(port) {
            Ok(r) => {
                eprintln!(
                    "[satchel] mDNS responder up; LAN clients can use http://{}.local:{port}",
                    mdns::Responder::hostname()
                );
                Some(r)
            }
            Err(e) => {
                eprintln!("[satchel] mDNS responder failed to start: {e}");
                None
            }
        }
    } else {
        None
    };

    let state = Arc::new(AppState {
        db: Arc::new(db),
        embedder: Arc::new(embedder),
        jobs: Arc::new(JobRegistry::new()),
        tunnel: TunnelManager::new(),
        port,
        vault_path,
        release_cache: ReleaseCache::default(),
        mdns: std::sync::Mutex::new(mdns_initial),
    });

    Router::new()
        .route("/", get(ui_handler))
        .route("/mcp", post(mcp_handler))
        .route("/api/status", get(api_status))
        .route("/api/release", get(api_release))
        .route("/api/sources", get(api_sources).delete(api_delete_sources))
        .route("/api/documents", get(api_documents))
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
        .route("/api/tunnel", get(api_tunnel_status))
        .route("/api/tunnel/start", post(api_tunnel_start))
        .route("/api/tunnel/stop", post(api_tunnel_stop))
        .route(
            "/api/tunnel/config",
            get(api_tunnel_config_get)
                .post(api_tunnel_config_set)
                .delete(api_tunnel_config_clear),
        )
        .route(
            "/api/anthropic/config",
            get(api_anthropic_config_get)
                .post(api_anthropic_config_set)
                .delete(api_anthropic_config_clear),
        )
        .route("/api/anthropic/messages", post(api_anthropic_messages))
        .route("/api/anthropic/test", post(api_anthropic_test))
        .route(
            "/api/mcp/servers",
            get(api_mcp_servers_list).post(api_mcp_servers_upsert),
        )
        .route(
            "/api/mcp/servers/:id",
            axum::routing::delete(api_mcp_servers_delete),
        )
        .route("/api/mcp/proxy/:id", post(api_mcp_proxy))
        .route(
            "/api/collections",
            get(api_collections_list).post(api_collections_create),
        )
        .route(
            "/api/collections/:id",
            axum::routing::delete(api_collections_delete),
        )
        .route(
            "/api/collections/:id/sources",
            post(api_collection_assign).delete(api_collection_unassign),
        )
        .route(
            "/api/collections/:id/documents",
            post(api_collection_assign_docs).delete(api_collection_unassign_docs),
        )
        .route("/api/connect-info", get(api_connect_info))
        .route("/api/mdns", get(api_mdns_get).post(api_mdns_set))
        // Lift axum's per-`Json<T>` body limit (default 2 MB) to 64 MB
        // so the transport does not cap us below the MCP `add_to_vault`
        // 50 MB content limit. 64 MB leaves ~14 MB headroom for JSON
        // wrapping overhead (escapes, key names, base64 boundaries).
        // Applies to every route on this router; the MCP and ingest
        // paths are the only ones that practically push it, but a
        // uniform setting beats a route-by-route override.
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
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
    vault_path: PathBuf,
) -> anyhow::Result<()> {
    let app = build_router(db, embedder, port, vault_path);

    let addr = format!("127.0.0.1:{port}");
    let url = format!("http://{addr}");

    // Single-instance enforcement at the Rust level (was previously
    // `LSMultipleInstancesProhibited` in the macOS Info.plist, but that
    // hands the launch decision to LaunchServices and breaks the
    // standard Gatekeeper "Open Anyway" flow when the bundle has stale
    // registrations). Bind first; if the port is already taken, it's
    // because another SATCHEL is already running. Open the browser to
    // that running instance's UI and exit cleanly so a second click on
    // the .app behaves like "bring it forward."
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            eprintln!("[satchel] Another SATCHEL is already running on {addr}.");
            eprintln!("[satchel] Opening the existing instance's web UI: {url}");
            if open_in_browser {
                let _ = open_url(&url);
            }
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    eprintln!("[satchel] Web UI:       {url}");
    eprintln!("[satchel] MCP endpoint: {url}/mcp");
    eprintln!("[satchel] REST API:     {url}/api/");
    // mDNS broadcasting (when enabled) is logged from `build_router`
    // alongside the satchel.local hostname so users see both lines
    // before the browser opens.

    if open_in_browser {
        let url = url.clone();
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
    let active_name = crate::vault::get_active_vault(&state.vault_path);
    let active_dir = crate::vault::active_vault_path(&state.vault_path)
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    let other_vaults = crate::vault::list_vaults_info(&state.vault_path)
        .ok()
        .unwrap_or_default()
        .into_iter()
        .filter(|v| !v.active)
        .collect::<Vec<_>>();
    let legacy = crate::vault::legacy_bases(&state.vault_path);
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
        })),
        "vault": {
            "name": active_name,
            "path": active_dir,
            "base_path": state.vault_path.to_string_lossy().to_string(),
            // Sibling vaults under the same base.
            "siblings": other_vaults,
            // SATCHEL bases discovered elsewhere on disk that aren't the
            // chosen one — most commonly an old `~/vault` from a v1.x
            // launch. If non-empty, the UI can prompt the user with
            // "this much data is sitting at <path>; restart with
            // --vault <path> to use it."
            "legacy_bases": legacy,
        },
    }))
}

#[derive(Deserialize, Default)]
struct ReleaseQuery {
    /// `?refresh=1` bypasses the in-memory cache. Used by the
    /// "check now" button in the Dashboard so the user can see a fresh
    /// answer without waiting for the TTL to expire.
    #[serde(default)]
    refresh: bool,
}

async fn api_release(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ReleaseQuery>,
) -> Json<Value> {
    let info = state.release_cache.get_or_fetch(q.refresh).await;
    Json(serde_json::to_value(info).unwrap_or_else(|_| json!({})))
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
    /// Restrict to source paths assigned to a single collection (v1.6.0+).
    collection_id: Option<i64>,
}

async fn api_sources(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SourcesQuery>,
) -> Json<Value> {
    let limit = q.limit.unwrap_or(50).clamp(1, 1000);
    let offset = q.offset.unwrap_or(0);
    match state.db.list_sources(
        q.filter_type.as_deref(),
        q.q.as_deref(),
        q.sort_by.as_deref().unwrap_or("name"),
        limit,
        offset,
        q.collection_id,
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

async fn api_documents(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SourcesQuery>,
) -> Json<Value> {
    let limit = q.limit.unwrap_or(50).clamp(1, 1000);
    let offset = q.offset.unwrap_or(0);
    match state.db.list_documents(
        q.filter_type.as_deref(),
        q.q.as_deref(),
        q.sort_by.as_deref().unwrap_or("name"),
        limit,
        offset,
        q.collection_id,
    ) {
        Ok(page) => Json(json!({
            "documents": page.documents,
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
    /// Restrict results to chunks whose document is in this collection.
    collection_id: Option<i64>,
    /// Restrict to a single file type (e.g. `"md"`, `"csv"`, `"slack"`).
    filter_file_type: Option<String>,
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
        rag::SearchOptions {
            filter_source: req.filter_source.as_deref(),
            filter_collection: req.collection_id,
            filter_file_type: req.filter_file_type.as_deref(),
            ..Default::default()
        },
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
    /// Mirrors `/api/clear`: a write-mode call must explicitly opt in.
    /// Defaults false so a stray DELETE never wipes documents from a
    /// tunneled vault.
    #[serde(default)]
    confirm: bool,
}

async fn api_delete_sources(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DeleteRequest>,
) -> Json<Value> {
    if !req.dry_run && !req.confirm {
        return Json(json!({
            "error": "destructive operation: pass {\"confirm\": true} or {\"dry_run\": true}"
        }));
    }
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
    /// Either `collection_id` (resolved numeric id) or
    /// `collection_name` (user-facing label, auto-created if it does
    /// not yet exist). When both are present, the id wins. When neither
    /// is present, the documents are ingested vault-wide with no
    /// collection assignment.
    #[serde(default)]
    collection_id: Option<i64>,
    #[serde(default)]
    collection_name: Option<String>,
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

    // Fail fast if there's no embedder — every record would fail otherwise
    // and the user would see "all N records failed" only after a long wait.
    // Done BEFORE collection resolution so a no-model retry loop does not
    // leave a trail of empty auto-created collections behind.
    if !state.embedder.is_available() {
        return Json(json!({
            "error": "embedding model unavailable; ingest would fail. Run \
                      ./scripts/download-model.sh or rebuild with --features embed-model"
        }));
    }

    // Resolve the optional collection target. id wins if both are
    // supplied; a name that doesn't match an existing collection is
    // created on the fly so the UI's "+ NEW" affordance in the Ingest
    // form is a single round trip.
    let collection_id = match (req.collection_id, req.collection_name.as_deref()) {
        (Some(id), _) => Some(id),
        (None, Some(name)) if !name.trim().is_empty() => {
            match resolve_or_create_collection(&state.db, name.trim()) {
                Ok(id) => Some(id),
                Err(e) => return Json(json!({ "error": format!("collection: {e}") })),
            }
        }
        _ => None,
    };

    let config = crate::ingest::IngestConfig {
        chunk_size: req.chunk_size.unwrap_or(512),
        chunk_overlap: req.chunk_overlap.unwrap_or(64),
        collection_id,
    };

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
                ProgressEvent::FilesPlanned(n) => {
                    j.files_total = Some(n);
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

// ─────────────────────────────────────────────────────────────────────────
// Tunnel endpoints. Drive the bundled-or-PATH `cloudflared` so a one-click
// public quick-tunnel (https://*.trycloudflare.com → http://localhost:7428)
// is available straight from the Connect tab.
// ─────────────────────────────────────────────────────────────────────────

async fn api_tunnel_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    // Refresh `installed` on every poll — cheap (~1 fork) and lets the UI
    // recover when the user installs cloudflared after first load.
    let _ = state.tunnel.check_installed().await;
    let snap = state.tunnel.snapshot();
    // Surface "is named-tunnel config saved?" + the configured hostname
    // alongside the live tunnel state so the UI can render a single
    // coherent view from one fetch.
    let cfg = TunnelConfig::load(&state.vault_path).ok().flatten();
    Json(json!({
        "installed": snap.installed,
        "running": snap.running,
        "mode": snap.mode,
        "url": snap.url,
        "forwarding": snap.forwarding,
        "started_at": snap.started_at,
        "error": snap.error,
        "named": {
            "configured": cfg.is_some(),
            "hostname": cfg.as_ref().map(|c| c.hostname.clone()),
        },
    }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct StartTunnelRequest {
    /// "quick" (default) or "named". Named requires a saved
    /// `<vault>/tunnel.toml`.
    mode: Option<String>,
}

async fn api_tunnel_start(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartTunnelRequest>,
) -> Json<Value> {
    let mode = req.mode.as_deref().unwrap_or("quick").to_lowercase();
    let result = match mode.as_str() {
        "quick" => state.tunnel.start_quick(state.port).await,
        "named" => match TunnelConfig::load(&state.vault_path) {
            Ok(Some(cfg)) => {
                state
                    .tunnel
                    .start_named(&cfg.token, &cfg.hostname, state.port)
                    .await
            }
            Ok(None) => Err(anyhow::anyhow!(
                "no named-tunnel config saved — POST a token + hostname to /api/tunnel/config first"
            )),
            Err(e) => Err(e),
        },
        other => Err(anyhow::anyhow!(
            "unknown tunnel mode: {other}. Use 'quick' or 'named'."
        )),
    };
    match result {
        Ok(_) => Json(json!(state.tunnel.snapshot())),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

async fn api_tunnel_stop(State(state): State<Arc<AppState>>) -> Json<Value> {
    match state.tunnel.stop().await {
        Ok(_) => Json(json!(state.tunnel.snapshot())),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

// ─── Named-tunnel config (token + hostname) — persisted to vault ───────

async fn api_tunnel_config_get(State(state): State<Arc<AppState>>) -> Json<Value> {
    // NEVER return the saved token over the wire. Only report whether
    // we have one and what hostname it points at — that's all the UI
    // needs to render its "configured: vault.example.com" pill.
    match TunnelConfig::load(&state.vault_path) {
        Ok(Some(cfg)) => Json(json!({
            "configured": true,
            "hostname": cfg.hostname,
        })),
        Ok(None) => Json(json!({ "configured": false, "hostname": null })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct SetTunnelConfigRequest {
    token: String,
    hostname: String,
}

async fn api_tunnel_config_set(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SetTunnelConfigRequest>,
) -> Json<Value> {
    let token = req.token.trim().to_string();
    let hostname = req
        .hostname
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    if token.is_empty() {
        return Json(json!({ "error": "token must not be empty" }));
    }
    if hostname.is_empty() {
        return Json(json!({ "error": "hostname must not be empty" }));
    }
    let cfg = TunnelConfig {
        token,
        hostname: hostname.clone(),
    };
    match cfg.save(&state.vault_path) {
        Ok(_) => Json(json!({ "configured": true, "hostname": hostname })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

async fn api_tunnel_config_clear(State(state): State<Arc<AppState>>) -> Json<Value> {
    match TunnelConfig::clear(&state.vault_path) {
        Ok(_) => Json(json!({ "configured": false, "hostname": null })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

// Reference TunnelMode so the import isn't dead — even though we serialize
// it via the snapshot, the import is needed for `cfg!`-style use elsewhere.
#[allow(dead_code)]
fn _tunnel_mode_referenced(_m: TunnelMode) {}

// ─────────────────────────────────────────────────────────────────────────
// Anthropic — server-side proxy so the API key stays server-side and so we
// can dodge the browser CORS gates around api.anthropic.com.
// ─────────────────────────────────────────────────────────────────────────

async fn api_anthropic_config_get(State(state): State<Arc<AppState>>) -> Json<Value> {
    match AnthropicConfig::load(&state.vault_path) {
        Ok(Some(_)) => Json(json!({ "configured": true })),
        Ok(None) => Json(json!({ "configured": false })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct SetAnthropicRequest {
    api_key: String,
}

async fn api_anthropic_config_set(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SetAnthropicRequest>,
) -> Json<Value> {
    let key = req.api_key.trim().to_string();
    if key.is_empty() {
        return Json(json!({ "error": "api_key must not be empty" }));
    }
    let cfg = AnthropicConfig { api_key: key };
    match cfg.save(&state.vault_path) {
        Ok(_) => Json(json!({ "configured": true })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

async fn api_anthropic_config_clear(State(state): State<Arc<AppState>>) -> Json<Value> {
    match AnthropicConfig::clear(&state.vault_path) {
        Ok(_) => Json(json!({ "configured": false })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

/// Validate the saved Anthropic API key by issuing a minimal,
/// 5-output-token request to claude-haiku-4-5. Returns
/// `{ok: true, model: "..."}` on success or
/// `{ok: false, status: <code>, error: "..."}` on any failure.
/// Distinct from `/api/anthropic/messages` (which streams a full chat
/// turn and is gated on the key already being good); this is a
/// single-shot health check the Settings UI can use to surface key
/// errors immediately on save instead of on first chat use.
async fn api_anthropic_test(State(state): State<Arc<AppState>>) -> Json<Value> {
    let cfg = match AnthropicConfig::load(&state.vault_path) {
        Ok(Some(c)) => c,
        Ok(None) => {
            return Json(json!({
                "ok": false,
                "error": "No API key configured. Save a key first."
            }));
        }
        Err(e) => {
            return Json(json!({"ok": false, "error": format!("config load: {e}")}));
        }
    };
    let probe_body = json!({
        "model": "claude-haiku-4-5",
        "max_tokens": 5,
        "stream": false,
        "messages": [{"role": "user", "content": "ok"}]
    });
    match anthropic::proxy_messages(&cfg.api_key, probe_body).await {
        Ok(res) => {
            let status = res.status();
            if status.is_success() {
                Json(json!({"ok": true, "model": "claude-haiku-4-5"}))
            } else {
                let body = res.text().await.unwrap_or_default();
                let detail = match serde_json::from_str::<Value>(&body) {
                    Ok(v) => v
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or(&body)
                        .to_string(),
                    Err(_) => body,
                };
                Json(json!({
                    "ok": false,
                    "status": status.as_u16(),
                    "error": detail
                }))
            }
        }
        Err(e) => Json(json!({"ok": false, "error": e.to_string()})),
    }
}

async fn api_anthropic_messages(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let cfg = match AnthropicConfig::load(&state.vault_path) {
        Ok(Some(c)) => c,
        Ok(None) => {
            return error_response(
                StatusCode::PRECONDITION_REQUIRED,
                "anthropic api key not configured — Settings → Anthropic API to add one",
            );
        }
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }
    };
    let upstream = match anthropic::proxy_messages(&cfg.api_key, body).await {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, &e.to_string()),
    };

    // Errors from Anthropic come back as a single JSON body, not an SSE
    // stream. Buffer + log them so they show up in the satchel terminal
    // instead of disappearing into the streaming pipe; pass the same
    // body through to the browser so the chat can render the message.
    let status = upstream.status();
    if !status.is_success() {
        let headers = upstream.headers().clone();
        let body_bytes = match upstream.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("anthropic upstream read failed: {e}"),
                );
            }
        };
        let body_str = String::from_utf8_lossy(&body_bytes);
        tracing::warn!(
            status = %status,
            body = %body_str,
            "anthropic upstream returned non-success status",
        );
        let mut builder = Response::builder().status(status);
        if let Some(ct) = headers.get("content-type") {
            builder = builder.header("content-type", ct);
        }
        return builder
            .body(Body::from(body_bytes))
            .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "build failed"));
    }

    forward_streaming_response(upstream)
}

// ─────────────────────────────────────────────────────────────────────────
// External MCP servers — store the auth headers server-side; expose the
// safe shape ({ id, name, url, has_auth }) to the browser; proxy raw
// JSON-RPC through `/api/mcp/proxy/:id`.
// ─────────────────────────────────────────────────────────────────────────

async fn api_mcp_servers_list(State(state): State<Arc<AppState>>) -> Json<Value> {
    match McpServersConfig::load(&state.vault_path) {
        Ok(cfg) => Json(json!({
            "servers": cfg.servers.iter().map(|s| json!({
                "id": s.id,
                "name": s.name,
                "url": s.url,
                "has_auth": !s.headers.is_empty(),
            })).collect::<Vec<_>>(),
        })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct UpsertMcpRequest {
    id: String,
    name: String,
    url: String,
    /// Optional: `{ "Authorization": "Bearer xxx", … }`
    #[serde(default)]
    headers: std::collections::BTreeMap<String, String>,
}

async fn api_mcp_servers_upsert(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UpsertMcpRequest>,
) -> Json<Value> {
    let mut cfg = match McpServersConfig::load(&state.vault_path) {
        Ok(c) => c,
        Err(e) => return Json(json!({ "error": e.to_string() })),
    };
    let entry = McpServerEntry {
        id: req.id.trim().to_string(),
        name: req.name.trim().to_string(),
        url: req.url.trim().to_string(),
        headers: req
            .headers
            .into_iter()
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .filter(|(k, v)| !k.is_empty() && !v.is_empty())
            .collect(),
    };
    if let Err(e) = cfg.upsert(entry) {
        return Json(json!({ "error": e.to_string() }));
    }
    if let Err(e) = cfg.save(&state.vault_path) {
        return Json(json!({ "error": e.to_string() }));
    }
    Json(json!({ "ok": true }))
}

async fn api_mcp_servers_delete(
    State(state): State<Arc<AppState>>,
    AxPath(id): AxPath<String>,
) -> Json<Value> {
    let mut cfg = match McpServersConfig::load(&state.vault_path) {
        Ok(c) => c,
        Err(e) => return Json(json!({ "error": e.to_string() })),
    };
    if !cfg.remove(&id) {
        return Json(json!({ "error": format!("no mcp server with id '{id}'") }));
    }
    if let Err(e) = cfg.save(&state.vault_path) {
        return Json(json!({ "error": e.to_string() }));
    }
    Json(json!({ "ok": true }))
}

async fn api_mcp_proxy(
    State(state): State<Arc<AppState>>,
    AxPath(id): AxPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let cfg = match McpServersConfig::load(&state.vault_path) {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let entry = match cfg.find(&id) {
        Some(e) => e.clone(),
        None => {
            return error_response(
                StatusCode::NOT_FOUND,
                &format!("no mcp server with id '{id}'"),
            );
        }
    };
    // Translate axum HeaderMap -> reqwest HeaderMap (just the bits we
    // forward — currently only the MCP session id).
    let mut fwd = reqwest::header::HeaderMap::new();
    if let Some(v) = headers.get("mcp-session-id") {
        if let Ok(name) = HeaderName::try_from("mcp-session-id") {
            if let Ok(val) = reqwest::header::HeaderValue::from_bytes(v.as_bytes()) {
                fwd.insert(
                    name.as_str()
                        .parse::<reqwest::header::HeaderName>()
                        .unwrap(),
                    val,
                );
            }
        }
    }
    match mcp_proxy::proxy_call(&entry, body, &fwd).await {
        Ok(res) => forward_streaming_response(res),
        Err(e) => error_response(StatusCode::BAD_GATEWAY, &e.to_string()),
    }
}

// ─── shared helpers ───

fn error_response(code: StatusCode, message: &str) -> Response {
    let body = serde_json::to_string(&json!({ "error": message })).unwrap_or_default();
    Response::builder()
        .status(code)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::from("")))
}

/// Forward an upstream reqwest response (status + headers + body) to the
/// axum caller. Streams the body byte-for-byte so SSE works without
/// re-framing.
fn forward_streaming_response(upstream: reqwest::Response) -> Response {
    let status = upstream.status();
    let mut builder = Response::builder().status(status);
    let pass_through = [
        "content-type",
        "cache-control",
        "anthropic-request-id",
        "mcp-session-id",
    ];
    for &name in &pass_through {
        if let Some(v) = upstream.headers().get(name) {
            builder = builder.header(name, v);
        }
    }
    let stream = upstream
        .bytes_stream()
        .map(|chunk| chunk.map_err(|e| std::io::Error::other(e.to_string())));
    builder.body(Body::from_stream(stream)).unwrap_or_else(|_| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "response build failed")
    })
}

// ─────────────────────────────────────────────────────────────────────────
// Collections — named subsets of documents inside a single vault. Schema
// + Database methods live in src/rag/mod.rs; this section just exposes
// them over REST.
// ─────────────────────────────────────────────────────────────────────────

/// Look up a collection by name (case-insensitive), creating one when
/// no match exists. Mirrors the CLI's `--collection foo` semantic so
/// `POST /api/ingest { collection_name: "foo" }` is a one-shot
/// operation from the Ingest tab even on a brand new vault.
fn resolve_or_create_collection(db: &Database, name: &str) -> anyhow::Result<i64> {
    for c in db.list_collections()? {
        if c.name.eq_ignore_ascii_case(name) {
            return Ok(c.id);
        }
    }
    db.create_collection(name)
}

async fn api_collections_list(State(state): State<Arc<AppState>>) -> Json<Value> {
    match state.db.list_collections() {
        Ok(rows) => Json(json!({ "collections": rows })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct CreateCollectionRequest {
    name: String,
}

async fn api_collections_create(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateCollectionRequest>,
) -> Json<Value> {
    match state.db.create_collection(&req.name) {
        Ok(id) => Json(json!({ "id": id, "name": req.name.trim() })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

async fn api_collections_delete(
    State(state): State<Arc<AppState>>,
    AxPath(id): AxPath<i64>,
) -> Json<Value> {
    match state.db.delete_collection(id) {
        Ok(true) => Json(json!({ "ok": true })),
        Ok(false) => Json(json!({ "error": format!("no collection with id {id}") })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct CollectionSourcesRequest {
    /// Source paths to assign / unassign. We resolve these to all matching
    /// document_ids server-side so the UI can stay in source-path land.
    source_paths: Vec<String>,
}

async fn api_collection_assign(
    State(state): State<Arc<AppState>>,
    AxPath(id): AxPath<i64>,
    Json(req): Json<CollectionSourcesRequest>,
) -> Json<Value> {
    match state.db.collection_add_source_paths(id, &req.source_paths) {
        Ok(n) => Json(json!({ "added": n })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

async fn api_collection_unassign(
    State(state): State<Arc<AppState>>,
    AxPath(id): AxPath<i64>,
    Json(req): Json<CollectionSourcesRequest>,
) -> Json<Value> {
    match state
        .db
        .collection_remove_source_paths(id, &req.source_paths)
    {
        Ok(n) => Json(json!({ "removed": n })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct CollectionDocumentsRequest {
    /// Document ids (from `documents.id`) to assign / unassign. Use this
    /// when the UI is in BY RECORD mode — picking individual ingested rows
    /// rather than every record at a source_path.
    document_ids: Vec<String>,
}

async fn api_collection_assign_docs(
    State(state): State<Arc<AppState>>,
    AxPath(id): AxPath<i64>,
    Json(req): Json<CollectionDocumentsRequest>,
) -> Json<Value> {
    match state.db.collection_add_documents(id, &req.document_ids) {
        Ok(n) => Json(json!({ "added": n })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

async fn api_collection_unassign_docs(
    State(state): State<Arc<AppState>>,
    AxPath(id): AxPath<i64>,
    Json(req): Json<CollectionDocumentsRequest>,
) -> Json<Value> {
    match state.db.collection_remove_documents(id, &req.document_ids) {
        Ok(n) => Json(json!({ "removed": n })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Connect / mDNS — surface the local addresses the running server can be
// reached at, and let the user toggle mDNS broadcasting on or off.
// ─────────────────────────────────────────────────────────────────────────

/// Best-effort enumeration of non-loopback IPv4 addresses bound to local
/// interfaces. Returned to the Connect tab so the user can pick the IP
/// their phone / other laptop should hit when `.local` resolution is
/// blocked. We rely on `local_ipv4s` via a small helper rather than
/// pulling in a third-party crate: read `/proc/net/fib_trie` is too
/// Linux-specific, and `gethostbyname` rules out musl. Instead we
/// connect a UDP socket to a public IP and ask the OS which local
/// address it would have used; that is the canonical "outbound" IP and
/// is enough for the Connect tab. Multi-interface hosts can still see
/// satchel.local via mDNS.
fn detect_lan_ipv4() -> Option<std::net::IpAddr> {
    use std::net::UdpSocket;
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    // 192.0.2.1 is in TEST-NET-1; no actual packet is sent for `connect`.
    socket.connect("192.0.2.1:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

async fn api_connect_info(State(state): State<Arc<AppState>>) -> Json<Value> {
    // The binary path printed in client-setup snippets. `current_exe`
    // resolves to the actual on-disk path even when the process was
    // launched from a .app bundle, which is exactly what users need to
    // paste into Claude Desktop / Cursor configs.
    let binary_path = std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "satchel".to_string());

    let port = state.port;
    let local_url = format!("http://127.0.0.1:{port}");
    let mcp_url = format!("{local_url}/mcp");

    let mdns_enabled = state.mdns.lock().map(|g| g.is_some()).unwrap_or(false);
    let mdns_host = mdns::Responder::hostname();
    let mdns_url = if mdns_enabled {
        Some(format!("http://{mdns_host}.local:{port}"))
    } else {
        None
    };
    let mdns_mcp_url = mdns_url.as_ref().map(|u| format!("{u}/mcp"));

    let lan_ip = detect_lan_ipv4().map(|ip| ip.to_string());
    let lan_url = lan_ip.as_ref().map(|ip| format!("http://{ip}:{port}"));
    let lan_mcp_url = lan_url.as_ref().map(|u| format!("{u}/mcp"));

    Json(json!({
        "binary_path": binary_path,
        "port": port,
        "local_url": local_url,
        "mcp_url": mcp_url,
        "lan_ip": lan_ip,
        "lan_url": lan_url,
        "lan_mcp_url": lan_mcp_url,
        "mdns": {
            "enabled": mdns_enabled,
            "hostname": format!("{mdns_host}.local"),
            "url": mdns_url,
            "mcp_url": mdns_mcp_url,
        },
    }))
}

async fn api_mdns_get(State(state): State<Arc<AppState>>) -> Json<Value> {
    let cfg = MdnsConfig::load(&state.vault_path).unwrap_or_default();
    let running = state.mdns.lock().map(|g| g.is_some()).unwrap_or(false);
    Json(json!({
        "enabled": cfg.enabled,
        "running": running,
        "hostname": format!("{}.local", mdns::Responder::hostname()),
    }))
}

#[derive(Deserialize)]
struct SetMdnsRequest {
    enabled: bool,
}

async fn api_mdns_set(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SetMdnsRequest>,
) -> Json<Value> {
    // Persist the toggle first so a crash mid-flip still reflects the
    // user's intent on next launch.
    let cfg = MdnsConfig {
        enabled: req.enabled,
    };
    if let Err(e) = cfg.save(&state.vault_path) {
        return Json(json!({ "error": format!("save mdns config: {e}") }));
    }

    let mut guard = match state.mdns.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if req.enabled {
        if guard.is_none() {
            match MdnsResponder::start(state.port) {
                Ok(r) => *guard = Some(r),
                Err(e) => return Json(json!({ "error": format!("start mdns: {e}") })),
            }
        }
    } else if let Some(r) = guard.take() {
        r.shutdown();
    }

    Json(json!({
        "enabled": req.enabled,
        "running": guard.is_some(),
        "hostname": format!("{}.local", mdns::Responder::hostname()),
    }))
}
