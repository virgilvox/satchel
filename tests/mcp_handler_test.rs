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
    assert_eq!(result["tools"].as_array().unwrap().len(), 6);
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
async fn test_mcp_search_with_collection_name() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    common::seed_data(&db, 3);

    // Build a "Research" collection that contains only file0.md.
    let cid = db.create_collection("Research").unwrap();
    db.collection_add_source_paths(cid, &["/test/file0.md".into()])
        .unwrap();

    // Scoped search: should hit file0 and miss file1/file2.
    let req = make_request(
        "tools/call",
        Some(json!(1)),
        Some(json!({
            "name": "search_knowledge",
            "arguments": { "query": "Content", "top_k": 5, "collection_name": "Research" }
        })),
    );
    let result = mcp::handle_request(&req, &db, &embedder).await;
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("file0.md"), "expected file0 in scoped result");
    assert!(!text.contains("file1.md"), "file1 leaked past scope");
    assert!(!text.contains("file2.md"), "file2 leaked past scope");

    // Unknown name returns an actionable tool error, not a panic.
    let req = make_request(
        "tools/call",
        Some(json!(2)),
        Some(json!({
            "name": "search_knowledge",
            "arguments": { "query": "x", "collection_name": "DoesNotExist" }
        })),
    );
    let result = mcp::handle_request(&req, &db, &embedder).await;
    assert_eq!(result["isError"], json!(true));
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("DoesNotExist"));
}

#[tokio::test]
async fn test_mcp_list_collections() {
    let db = common::test_db();
    let embedder = common::test_embedder();

    // Empty surface returns a hint, not an error.
    let req = make_request(
        "tools/call",
        Some(json!(1)),
        Some(json!({ "name": "list_collections", "arguments": {} })),
    );
    let empty = mcp::handle_request(&req, &db, &embedder).await;
    assert!(empty["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("No collections"));

    // After creating two, the tool returns both with their doc counts.
    db.create_collection("Work").unwrap();
    db.create_collection("Personal").unwrap();
    let result = mcp::handle_request(&req, &db, &embedder).await;
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Work"));
    assert!(text.contains("Personal"));
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
