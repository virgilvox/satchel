mod common;

use satchel_rag::embed::Embedder;
use satchel_rag::mcp::{self, JsonRpcRequest};
use serde_json::{json, Value};

fn make_request(method: &str, id: Option<Value>, params: Option<Value>) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id,
        method: method.to_string(),
        params,
    }
}

#[tokio::test]
async fn test_mcp_initialize_flow() {
    let db = common::test_db();
    let embedder = common::test_embedder();

    let req = make_request("initialize", Some(json!(1)), Some(json!({})));
    let result = mcp::handle_request(&req, &db, &embedder).await;
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert_eq!(result["serverInfo"]["name"], "satchel");

    let req = make_request("notifications/initialized", None, Some(json!({})));
    let result = mcp::handle_request(&req, &db, &embedder).await;
    assert_eq!(result, Value::Null);

    let req = make_request("tools/list", Some(json!(2)), Some(json!({})));
    let result = mcp::handle_request(&req, &db, &embedder).await;
    assert_eq!(result["tools"].as_array().unwrap().len(), 5);
}

#[tokio::test]
async fn test_mcp_search_with_seeded_data() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    common::seed_data(&db, 3);

    let req = make_request(
        "tools/call",
        Some(json!(1)),
        Some(json!({
            "name": "search_knowledge",
            "arguments": { "query": "test", "top_k": 5 }
        })),
    );
    let result = mcp::handle_request(&req, &db, &embedder).await;
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Result 1"));
    assert!(!result
        .get("isError")
        .is_some_and(|v| v.as_bool() == Some(true)));
}

#[tokio::test]
async fn test_mcp_list_sources_with_data() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    common::seed_data(&db, 3);

    let req = make_request(
        "tools/call",
        Some(json!(1)),
        Some(json!({
            "name": "list_sources",
            "arguments": {}
        })),
    );
    let result = mcp::handle_request(&req, &db, &embedder).await;
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("file0.md"));
    assert!(text.contains("file1.md"));
    assert!(text.contains("file2.md"));
}

#[tokio::test]
async fn test_mcp_get_document_roundtrip() {
    let db = common::test_db();
    let embedder = common::test_embedder();

    db.insert_document(
        "d1",
        "/notes/hello.md",
        "md",
        Some("Hello"),
        "Hello world content",
        "h1",
    )
    .unwrap();

    let req = make_request(
        "tools/call",
        Some(json!(1)),
        Some(json!({
            "name": "get_document",
            "arguments": { "source": "/notes/hello.md" }
        })),
    );
    let result = mcp::handle_request(&req, &db, &embedder).await;
    let text = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(text, "Hello world content");
}

#[tokio::test]
async fn test_mcp_tags_roundtrip() {
    let db = common::test_db();
    let embedder = common::test_embedder();

    db.insert_document("d1", "/a.md", "md", None, "a", "h1")
        .unwrap();
    db.add_tag("d1", "work").unwrap();
    db.add_tag("d1", "notes").unwrap();

    let req = make_request(
        "tools/call",
        Some(json!(1)),
        Some(json!({
            "name": "list_tags",
            "arguments": {}
        })),
    );
    let result = mcp::handle_request(&req, &db, &embedder).await;
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("notes"));
    assert!(text.contains("work"));
}
