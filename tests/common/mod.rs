use satchel_rag::embed::Embedder;
use satchel_rag::rag::Database;

pub fn test_db() -> Database {
    Database::open_memory().expect("in-memory DB should open")
}

pub fn test_embedder() -> Embedder {
    Embedder::fixed(384)
}

pub fn test_router() -> axum::Router {
    // Port 0 is a valid sentinel for "no real bind" — the tunnel UI surfaces
    // it but tests never actually start a tunnel. Tunnel config lookups go to
    // a temp vault path so we don't pollute the real one.
    let tmp = std::env::temp_dir().join(format!("satchel-test-vault-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    satchel_rag::server::build_router(test_db(), test_embedder(), 0, tmp)
}

/// Seed the database with documents and chunks for search testing.
/// Each document gets a unique embedding with a 1.0 in a different dimension.
pub fn seed_data(db: &Database, doc_count: usize) {
    for i in 0..doc_count {
        let doc_id = format!("doc-{i}");
        let source = format!("/test/file{i}.md");
        db.insert_document(
            &doc_id,
            &source,
            "md",
            Some(&format!("File {i}")),
            &format!("Content of document {i}"),
            &format!("hash{i}"),
        )
        .unwrap();

        let mut embedding = vec![0.0f32; 384];
        embedding[i % 384] = 1.0;
        db.insert_chunk(
            &format!("{doc_id}:0"),
            &doc_id,
            0,
            &format!("Chunk text for doc {i}"),
            10,
            0,
            20,
            &embedding,
        )
        .unwrap();
    }
}
