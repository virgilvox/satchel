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
