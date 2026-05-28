mod common;

use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app() -> axum::Router {
    common::test_router()
}

async fn get(uri: &str) -> (u16, serde_json::Value) {
    let resp = app()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&body)
        .unwrap_or(serde_json::json!({"_raw": String::from_utf8_lossy(&body).to_string()}));
    (status, json)
}

async fn post_json(uri: &str, body: &str) -> (u16, serde_json::Value) {
    let resp = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}))
    };
    (status, json)
}

/// Send a GET through a specific router. Use this when a test makes
/// multiple requests that need to share state; the bare `get(..)`
/// helper builds a fresh app each call and would silently lose
/// in-memory DB state between requests. `Router` is `Clone` and
/// `oneshot` consumes its service, so we clone per call here.
async fn get_on(app: &axum::Router, uri: &str) -> (u16, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&body)
        .unwrap_or(serde_json::json!({"_raw": String::from_utf8_lossy(&body).to_string()}));
    (status, json)
}

async fn post_on(app: &axum::Router, uri: &str, body: &str) -> (u16, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}))
    };
    (status, json)
}

#[tokio::test]
async fn test_get_root_returns_html() {
    let resp = app()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("<!DOCTYPE html>"));
}

#[tokio::test]
async fn test_get_api_status() {
    let (status, json) = get("/api/status").await;
    assert_eq!(status, 200);
    assert_eq!(json["status"], "running");
    assert!(json["stats"].is_object());
}

#[tokio::test]
async fn test_get_api_sources_empty() {
    let (status, json) = get("/api/sources").await;
    assert_eq!(status, 200);
    assert!(json["sources"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_post_api_search() {
    let (status, json) = post_json("/api/search", r#"{"query":"test","top_k":5}"#).await;
    assert_eq!(status, 200);
    assert!(json["results"].is_array());
}

#[tokio::test]
async fn test_get_api_tags_empty() {
    let (status, json) = get("/api/tags").await;
    assert_eq!(status, 200);
    assert!(json["tags"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_get_api_document_missing() {
    let (status, json) = get("/api/document?source=nonexistent").await;
    assert_eq!(status, 200);
    assert!(json["error"].is_string());
}

#[tokio::test]
async fn test_get_api_config_claude_desktop() {
    let (status, json) = get("/api/config/claude-desktop").await;
    assert_eq!(status, 200);
    assert_eq!(json["client"], "Claude Desktop");
}

#[tokio::test]
async fn test_get_api_config_unknown() {
    let (status, json) = get("/api/config/unknown-client").await;
    assert_eq!(status, 200);
    assert_eq!(json["client"], "Generic");
}

#[tokio::test]
async fn test_post_mcp_initialize() {
    let (status, json) = post_json(
        "/mcp",
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(json["jsonrpc"], "2.0");
    assert!(json["result"]["protocolVersion"].is_string());
}

#[tokio::test]
async fn test_post_mcp_notification_returns_accepted() {
    let (status, _) = post_json(
        "/mcp",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#,
    )
    .await;
    assert_eq!(status, 202);
}

#[tokio::test]
async fn test_get_api_connect_info() {
    // Connect tab consumes this on mount. The shape is contractual:
    // the UI assumes binary_path, port, mcp_url and mdns.{enabled,
    // hostname} all exist.
    let (status, json) = get("/api/connect-info").await;
    assert_eq!(status, 200);
    assert!(json["binary_path"].is_string());
    assert_eq!(json["port"], 0); // test router port sentinel
    assert!(json["mcp_url"].as_str().unwrap().contains("/mcp"));
    assert_eq!(json["mdns"]["hostname"], "satchel.local");
    // mDNS is not started in tests (port == 0 skips startup), so
    // running is false but enabled may be either depending on the
    // test-vault config. The structural keys must exist either way.
    assert!(json["mdns"]["enabled"].is_boolean());
}

#[tokio::test]
async fn test_get_api_mdns_returns_state() {
    // The Connect tab's toggle is driven by this endpoint's shape;
    // pin enabled + running + hostname so a future refactor that
    // renames or drops any of them is caught.
    let (status, json) = get("/api/mdns").await;
    assert_eq!(status, 200);
    assert!(json["enabled"].is_boolean());
    assert!(json["running"].is_boolean());
    assert_eq!(json["hostname"], "satchel.local");
}

#[tokio::test]
async fn test_post_api_mdns_toggle_off_persists() {
    // POST {enabled:false} must (a) not error out (b) reflect the
    // change in the same router's GET response. Port-0 test router
    // never started a daemon, so this exercises the persist-toggle
    // path even though there is nothing to shut down.
    //
    // We reuse one router across both calls because the toggle is
    // persisted to disk under the test vault; the in-process Mutex
    // state would otherwise reset between fresh routers.
    let app = app();
    let (s1, j1) = post_on(&app, "/api/mdns", r#"{"enabled":false}"#).await;
    assert_eq!(s1, 200);
    assert_eq!(j1["enabled"], false);
    let (s2, j2) = get_on(&app, "/api/mdns").await;
    assert_eq!(s2, 200);
    assert_eq!(j2["enabled"], false);
    // Restore the default so a subsequent test on the same vault dir
    // does not inherit the disabled state. The test vault path is
    // shared per-pid in test_router(), so a polluted state would
    // affect the next run of `cargo test`.
    let _ = post_on(&app, "/api/mdns", r#"{"enabled":true}"#).await;
}

fn collection_names(j: &serde_json::Value) -> Vec<String> {
    j["collections"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|c| c["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn test_post_api_ingest_creates_collection_by_name() {
    // The Ingest tab's "create a new collection for this job" field
    // POSTs `collection_name`. The server must (a) accept it,
    // (b) auto-create the collection, (c) queue the job. We re-use
    // one router for the round-trip so the in-memory DB persists
    // between the ingest call and the collections-list verification.
    let app = app();

    let (_, before) = get_on(&app, "/api/collections").await;
    assert!(!collection_names(&before)
        .iter()
        .any(|n| n == "fresh-collection"));

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"smoke test contents").unwrap();
    let body = format!(
        r#"{{"path":{:?},"collection_name":"fresh-collection"}}"#,
        tmp.path().to_string_lossy()
    );
    let (status, j) = post_on(&app, "/api/ingest", &body).await;
    assert_eq!(status, 200);
    // The fixed test embedder reports is_available()==true, so the
    // ingest proceeds and a job_id is returned; the worker runs in
    // the background and the collection is created up-front before
    // the worker spawns. We do not wait for the worker to finish;
    // collection creation is synchronous in the request handler.
    assert!(
        j.get("job_id").is_some() || j.get("error").is_some(),
        "unexpected ingest response shape: {j}"
    );

    let (_, after) = get_on(&app, "/api/collections").await;
    assert!(
        collection_names(&after)
            .iter()
            .any(|n| n == "fresh-collection"),
        "collection 'fresh-collection' should have been auto-created, got {:?}",
        collection_names(&after)
    );
}

#[tokio::test]
async fn test_post_api_ingest_rejects_missing_path_without_creating_collection() {
    // The path-not-found branch errors out BEFORE the embedder check
    // and before collection resolution, so a missing path with a
    // brand-new collection_name must NOT create the collection.
    // Critical: tests the ordering of validation in api_ingest.
    let app = app();

    let (_, before) = get_on(&app, "/api/collections").await;
    assert!(!collection_names(&before)
        .iter()
        .any(|n| n == "phantom-collection"));

    let (status, j) = post_on(
        &app,
        "/api/ingest",
        r#"{"path":"/definitely/does/not/exist/anywhere","collection_name":"phantom-collection"}"#,
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        j["error"]
            .as_str()
            .unwrap_or_default()
            .contains("not found"),
        "expected 'not found' error, got {j}"
    );

    let (_, after) = get_on(&app, "/api/collections").await;
    assert!(
        !collection_names(&after)
            .iter()
            .any(|n| n == "phantom-collection"),
        "missing-path ingest must NOT leave a stray collection behind, got {:?}",
        collection_names(&after)
    );
}

#[tokio::test]
async fn test_cors_headers_present() {
    let resp = app()
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.headers().get("access-control-allow-origin").is_some());
}

#[tokio::test]
async fn test_post_api_search_accepts_large_body() {
    // Regression guard for the v2.8.1 body-limit bump. Axum's
    // default Json<T> body limit is 2 MB, which would have capped MCP
    // and ingest payloads below the tool's own 50 MB cap. The router
    // now sets DefaultBodyLimit::max(64 MB) so this 4 MB POST must
    // pass through without a 413.
    let big_query: String = "x".repeat(4 * 1024 * 1024);
    let body = format!(r#"{{"query":{:?},"top_k":1}}"#, big_query);
    let (status, json) = post_json("/api/search", &body).await;
    assert_eq!(
        status, 200,
        "expected 200 (router body limit must accept 4 MB), got {status} body={json}"
    );
    // The search itself returns 0 results on an empty test DB; the
    // important assertion is that the transport did not 413.
    assert!(json["results"].is_array());
}
