mod common;

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
    assert_eq!(result["tools"].as_array().unwrap().len(), 10);
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
    assert!(result
        .get("isError")
        .is_none_or(|v| v.as_bool() != Some(true)));
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
async fn test_mcp_search_surfaces_chunk_id() {
    // The search_knowledge response must include `chunk_id:` per result so
    // an LLM can pass it back to `get_chunk_context`. Regression guard:
    // dropping this line would silently break the expansion path.
    let db = common::test_db();
    let embedder = common::test_embedder();
    common::seed_data(&db, 2);

    let req = make_request(
        "tools/call",
        Some(json!(1)),
        Some(json!({
            "name": "search_knowledge",
            "arguments": { "query": "Content", "top_k": 5 }
        })),
    );
    let result = mcp::handle_request(&req, &db, &embedder).await;
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("chunk_id: doc-0:"),
        "search output is missing chunk_id; got:\n{text}"
    );
}

#[tokio::test]
async fn test_mcp_get_chunk_context_returns_neighborhood() {
    let db = common::test_db();
    let embedder = common::test_embedder();

    // Lay out one document with five sequential chunks. We'll ask for
    // the neighborhood around chunk 2 with before=1, after=2.
    db.insert_document("doc1", "/notes.md", "md", None, "raw", "hash1")
        .unwrap();
    let v = vec![1.0_f32; 384];
    for i in 0..5 {
        db.insert_chunk(
            &format!("doc1:{i}"),
            "doc1",
            i,
            &format!("body of chunk {i}"),
            4,
            0,
            17,
            &v,
        )
        .unwrap();
    }

    let req = make_request(
        "tools/call",
        Some(json!(1)),
        Some(json!({
            "name": "get_chunk_context",
            "arguments": { "chunk_id": "doc1:2", "before": 1, "after": 2 }
        })),
    );
    let result = mcp::handle_request(&req, &db, &embedder).await;
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("/notes.md"));
    assert!(text.contains("body of chunk 1"));
    assert!(text.contains("body of chunk 2"));
    assert!(text.contains("body of chunk 3"));
    assert!(text.contains("body of chunk 4"));
    assert!(
        !text.contains("body of chunk 0"),
        "before=1 must not pull chunk 0"
    );
    assert!(
        text.contains("(center)"),
        "center marker missing on the requested chunk"
    );
}

#[tokio::test]
async fn test_mcp_get_chunk_context_unknown_chunk() {
    let db = common::test_db();
    let embedder = common::test_embedder();

    let req = make_request(
        "tools/call",
        Some(json!(1)),
        Some(json!({
            "name": "get_chunk_context",
            "arguments": { "chunk_id": "doesnotexist:99", "before": 2, "after": 2 }
        })),
    );
    let result = mcp::handle_request(&req, &db, &embedder).await;
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("No chunks found"));
    // Soft failure (helpful text), not an error envelope.
    assert!(result
        .get("isError")
        .is_none_or(|v| v.as_bool() != Some(true)));
}

#[tokio::test]
async fn test_mcp_get_chunk_context_missing_chunk_id() {
    let db = common::test_db();
    let embedder = common::test_embedder();

    let req = make_request(
        "tools/call",
        Some(json!(1)),
        Some(json!({
            "name": "get_chunk_context",
            "arguments": { "before": 2, "after": 2 }
        })),
    );
    let result = mcp::handle_request(&req, &db, &embedder).await;
    assert_eq!(result["isError"], json!(true));
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("chunk_id"));
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

// ─────────────────────────────────────────────────────────────────────────
// v2.8.0 write tools: add_to_vault, create_collection, assign_to_collection.
// ─────────────────────────────────────────────────────────────────────────

fn call_tool(name: &str, arguments: Value) -> JsonRpcRequest {
    make_request(
        "tools/call",
        Some(json!(1)),
        Some(json!({ "name": name, "arguments": arguments })),
    )
}

fn assert_ok(result: &Value) {
    assert!(
        result
            .get("isError")
            .is_none_or(|v| v.as_bool() != Some(true)),
        "expected success, got error: {result}"
    );
}

fn assert_err_contains(result: &Value, fragment: &str) {
    assert_eq!(
        result["isError"],
        json!(true),
        "expected error, got: {result}"
    );
    let text = result["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains(fragment),
        "error text {text:?} did not contain {fragment:?}"
    );
}

#[tokio::test]
async fn test_mcp_add_to_vault_basic_persists_searchable_doc() {
    // Happy path: a small markdown note via MCP is chunked, embedded,
    // and shows up in list_sources + a follow-up search.
    let db = common::test_db();
    let embedder = common::test_embedder();

    let req = call_tool(
        "add_to_vault",
        json!({ "content": "Recap of the design discussion: choose Postgres over MySQL." }),
    );
    let result = mcp::handle_request(&req, &db, &embedder).await;
    assert_ok(&result);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("Saved."), "got: {text}");
    assert!(
        text.contains("mcp://note/"),
        "default source should be mcp://note/<uuid>"
    );
    assert!(text.contains("chunks:"));

    // It should now exist on the documents table.
    let stats = db.stats().unwrap();
    assert_eq!(stats.document_count, 1);
    assert!(stats.chunk_count >= 1);
}

#[tokio::test]
async fn test_mcp_add_to_vault_dedup_returns_existing() {
    // Same source + same content = SHA-256 hit. Second call returns
    // the existing document_id and does NOT double up.
    let db = common::test_db();
    let embedder = common::test_embedder();

    let payload = json!({
        "content": "shared body",
        "source": "journal/2026-05-28",
    });
    let r1 = mcp::handle_request(&call_tool("add_to_vault", payload.clone()), &db, &embedder).await;
    assert_ok(&r1);
    let r2 = mcp::handle_request(&call_tool("add_to_vault", payload), &db, &embedder).await;
    assert_ok(&r2);

    let t2 = r2["content"][0]["text"].as_str().unwrap();
    assert!(
        t2.starts_with("Already in the vault"),
        "second call should report dedup, got: {t2}"
    );
    assert_eq!(db.stats().unwrap().document_count, 1, "no duplicate row");
}

#[tokio::test]
async fn test_mcp_add_to_vault_dedup_still_joins_new_collection() {
    // Re-issuing add_to_vault with the same content but a new
    // collection_name should attach the existing document to that
    // collection. Mirrors the file-ingest dedup-skip-still-assigns
    // behavior so the MCP path is not a regression vector.
    let db = common::test_db();
    let embedder = common::test_embedder();

    let r1 = mcp::handle_request(
        &call_tool(
            "add_to_vault",
            json!({
                "content": "policy doc body",
                "source": "policies/v1",
                "collection_name": "work",
            }),
        ),
        &db,
        &embedder,
    )
    .await;
    assert_ok(&r1);
    let r2 = mcp::handle_request(
        &call_tool(
            "add_to_vault",
            json!({
                "content": "policy doc body",
                "source": "policies/v1",
                "collection_name": "personal",
            }),
        ),
        &db,
        &embedder,
    )
    .await;
    assert_ok(&r2);
    assert_eq!(db.stats().unwrap().document_count, 1);
    let counts: std::collections::HashMap<String, usize> = db
        .list_collections()
        .unwrap()
        .into_iter()
        .map(|c| (c.name, c.document_count))
        .collect();
    assert_eq!(counts.get("work"), Some(&1), "work still has the doc");
    assert_eq!(
        counts.get("personal"),
        Some(&1),
        "personal joined via dedup-skip path"
    );
}

#[tokio::test]
async fn test_mcp_add_to_vault_with_collection_auto_creates() {
    // collection_name that does not exist should be auto-created.
    let db = common::test_db();
    let embedder = common::test_embedder();
    assert!(db.list_collections().unwrap().is_empty());

    let result = mcp::handle_request(
        &call_tool(
            "add_to_vault",
            json!({
                "content": "an item",
                "collection_name": "Fresh Collection",
            }),
        ),
        &db,
        &embedder,
    )
    .await;
    assert_ok(&result);
    let cols = db.list_collections().unwrap();
    assert_eq!(cols.len(), 1);
    assert_eq!(cols[0].name, "Fresh Collection");
    assert_eq!(cols[0].document_count, 1);
}

#[tokio::test]
async fn test_mcp_add_to_vault_with_tags_persists_them() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let result = mcp::handle_request(
        &call_tool(
            "add_to_vault",
            json!({
                "content": "tagged content",
                "tags": ["design", "draft", ""],  // empty trimmed away
            }),
        ),
        &db,
        &embedder,
    )
    .await;
    assert_ok(&result);
    let tags: std::collections::HashMap<String, usize> =
        db.list_tags().unwrap().into_iter().collect();
    assert_eq!(tags.get("design"), Some(&1));
    assert_eq!(tags.get("draft"), Some(&1));
    assert!(!tags.contains_key(""), "empty tags must be dropped");
}

#[tokio::test]
async fn test_mcp_add_to_vault_dry_run_does_not_write() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let result = mcp::handle_request(
        &call_tool(
            "add_to_vault",
            json!({
                "content": "would-be content",
                "collection_name": "phantom",
                "dry_run": true,
            }),
        ),
        &db,
        &embedder,
    )
    .await;
    assert_ok(&result);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("Would save"), "got: {text}");
    assert_eq!(db.stats().unwrap().document_count, 0, "no doc written");
    assert!(
        db.list_collections().unwrap().is_empty(),
        "dry_run must not auto-create the collection"
    );
}

#[tokio::test]
async fn test_mcp_add_to_vault_rejects_empty_content() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let r = mcp::handle_request(
        &call_tool("add_to_vault", json!({ "content": "   \n\t  " })),
        &db,
        &embedder,
    )
    .await;
    assert_err_contains(&r, "must not be empty");
}

#[tokio::test]
async fn test_mcp_add_to_vault_rejects_oversized_content() {
    // 51 MB > 50 MB cap. The repeat is fast and `assert_err_contains`
    // short-circuits before we ever try to chunk or embed the buffer.
    let db = common::test_db();
    let embedder = common::test_embedder();
    let payload: String = "x".repeat(51 * 1024 * 1024);
    let r = mcp::handle_request(
        &call_tool("add_to_vault", json!({ "content": payload })),
        &db,
        &embedder,
    )
    .await;
    assert_err_contains(&r, "max is");
}

#[tokio::test]
async fn test_mcp_add_to_vault_rejects_bad_file_type() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let r = mcp::handle_request(
        &call_tool(
            "add_to_vault",
            json!({ "content": "x", "file_type": "pdf" }),
        ),
        &db,
        &embedder,
    )
    .await;
    assert_err_contains(&r, "not supported");
}

#[tokio::test]
async fn test_mcp_add_to_vault_rejects_too_many_tags() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let many_tags: Vec<String> = (0..33).map(|i| format!("t{i}")).collect();
    let r = mcp::handle_request(
        &call_tool("add_to_vault", json!({ "content": "x", "tags": many_tags })),
        &db,
        &embedder,
    )
    .await;
    assert_err_contains(&r, "max is");
}

#[tokio::test]
async fn test_mcp_add_to_vault_canonicalizes_source() {
    // Bare name (no scheme) -> auto-prefixed with mcp:// so the source
    // is visually distinct from a real filesystem path.
    // A fully-qualified scheme is left untouched.
    let db = common::test_db();
    let embedder = common::test_embedder();
    mcp::handle_request(
        &call_tool(
            "add_to_vault",
            json!({ "content": "a", "source": "journal/2026" }),
        ),
        &db,
        &embedder,
    )
    .await;
    mcp::handle_request(
        &call_tool(
            "add_to_vault",
            json!({ "content": "b", "source": "file:///tmp/explicit.md" }),
        ),
        &db,
        &embedder,
    )
    .await;
    let sources: Vec<String> = db
        .list_sources(None, None, "name", 100, 0, None)
        .unwrap()
        .sources
        .into_iter()
        .map(|s| s.path)
        .collect();
    assert!(
        sources.contains(&"mcp://journal/2026".to_string()),
        "bare name should be prefixed; got {sources:?}"
    );
    assert!(
        sources.contains(&"file:///tmp/explicit.md".to_string()),
        "fully-qualified scheme should be preserved; got {sources:?}"
    );
}

#[tokio::test]
async fn test_mcp_create_collection_idempotent() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let r1 = mcp::handle_request(
        &call_tool("create_collection", json!({ "name": "Work" })),
        &db,
        &embedder,
    )
    .await;
    assert_ok(&r1);
    let r2 = mcp::handle_request(
        &call_tool("create_collection", json!({ "name": "work" })), // case-insensitive
        &db,
        &embedder,
    )
    .await;
    assert_ok(&r2);
    assert_eq!(db.list_collections().unwrap().len(), 1);
}

#[tokio::test]
async fn test_mcp_create_collection_rejects_empty() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let r = mcp::handle_request(
        &call_tool("create_collection", json!({ "name": "   " })),
        &db,
        &embedder,
    )
    .await;
    assert_err_contains(&r, "must not be empty");
}

#[tokio::test]
async fn test_mcp_assign_to_collection_assigns_known_docs() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    db.insert_document("d1", "/a.md", "md", None, "a", "h1")
        .unwrap();
    db.insert_document("d2", "/b.md", "md", None, "b", "h2")
        .unwrap();

    let r = mcp::handle_request(
        &call_tool(
            "assign_to_collection",
            json!({ "collection_name": "Inbox", "document_ids": ["d1", "d2"] }),
        ),
        &db,
        &embedder,
    )
    .await;
    assert_ok(&r);
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("added:         2"), "got: {text}");
    let cols = db.list_collections().unwrap();
    assert_eq!(cols[0].document_count, 2);
}

#[tokio::test]
async fn test_mcp_assign_to_collection_silently_drops_unknown_ids() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    db.insert_document("real", "/a.md", "md", None, "a", "h1")
        .unwrap();

    let r = mcp::handle_request(
        &call_tool(
            "assign_to_collection",
            json!({
                "collection_name": "Mixed",
                "document_ids": ["real", "nonexistent"],
            }),
        ),
        &db,
        &embedder,
    )
    .await;
    assert_ok(&r);
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("added:         1"), "got: {text}");
    assert!(text.contains("unknown id:    1"), "got: {text}");
}

#[tokio::test]
async fn test_mcp_assign_to_collection_rejects_empty_id_list() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let r = mcp::handle_request(
        &call_tool(
            "assign_to_collection",
            json!({ "collection_name": "x", "document_ids": [] }),
        ),
        &db,
        &embedder,
    )
    .await;
    assert_err_contains(&r, "at least one");
}

#[tokio::test]
async fn test_mcp_assign_to_collection_rejects_too_many_ids() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let many: Vec<String> = (0..201).map(|i| format!("d{i}")).collect();
    let r = mcp::handle_request(
        &call_tool(
            "assign_to_collection",
            json!({ "collection_name": "x", "document_ids": many }),
        ),
        &db,
        &embedder,
    )
    .await;
    assert_err_contains(&r, "max is 200");
}

#[tokio::test]
async fn test_mcp_assign_to_collection_idempotent_on_repeated_call() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    db.insert_document("d1", "/a.md", "md", None, "a", "h1")
        .unwrap();

    let req = call_tool(
        "assign_to_collection",
        json!({ "collection_name": "Once", "document_ids": ["d1"] }),
    );
    let _ = mcp::handle_request(&req, &db, &embedder).await;
    let r = mcp::handle_request(&req, &db, &embedder).await;
    assert_ok(&r);
    let text = r["content"][0]["text"].as_str().unwrap();
    // Second call: already a member, so added=0 and already_there=1.
    assert!(text.contains("added:         0"), "got: {text}");
    assert!(text.contains("already there: 1"), "got: {text}");
    let cols = db.list_collections().unwrap();
    assert_eq!(cols[0].document_count, 1, "no duplicate membership");
}

#[tokio::test]
async fn test_mcp_add_to_vault_search_then_assign_roundtrip() {
    // End-to-end: agent adds a doc, searches for it, takes the
    // returned source/id, and assigns it to a new collection via
    // assign_to_collection. The full pipeline must remain coherent.
    let db = common::test_db();
    let embedder = common::test_embedder();

    let add = mcp::handle_request(
        &call_tool(
            "add_to_vault",
            json!({
                "content": "a roundtrip body",
                "source": "rt/1",
            }),
        ),
        &db,
        &embedder,
    )
    .await;
    assert_ok(&add);
    let add_text = add["content"][0]["text"].as_str().unwrap().to_string();

    // Pull the document_id out of the Saved.\n  document_id:   <id> line.
    let doc_id = add_text
        .lines()
        .find_map(|l| l.trim().strip_prefix("document_id:   "))
        .expect("document_id line missing from add_to_vault output")
        .to_string();

    let assigned = mcp::handle_request(
        &call_tool(
            "assign_to_collection",
            json!({
                "collection_name": "RoundTrip",
                "document_ids": [doc_id],
            }),
        ),
        &db,
        &embedder,
    )
    .await;
    assert_ok(&assigned);
    assert!(
        assigned["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("added:         1"),
        "expected added: 1, got: {assigned}"
    );
}
