mod common;

use satchel_rag::ingest::{self, IngestConfig, Progress};
use tempfile::TempDir;

fn config() -> IngestConfig {
    IngestConfig {
        chunk_size: 512,
        chunk_overlap: 64,
    }
}

#[tokio::test]
async fn test_ingest_single_markdown_file() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    let file_path = dir.path().join("test.md");
    std::fs::write(&file_path, "# Hello\n\nThis is a test document.").unwrap();

    ingest::ingest_path(&file_path, &db, &embedder, &config(), &Progress::noop()).unwrap();

    let stats = db.stats().unwrap();
    assert_eq!(stats.document_count, 1);
    assert!(stats.chunk_count >= 1);
}

#[tokio::test]
async fn test_ingest_directory_multiple_files() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    std::fs::write(dir.path().join("a.md"), "Document A content.").unwrap();
    std::fs::write(dir.path().join("b.txt"), "Document B content.").unwrap();
    std::fs::write(dir.path().join("c.md"), "Document C content.").unwrap();

    ingest::ingest_path(dir.path(), &db, &embedder, &config(), &Progress::noop()).unwrap();

    let stats = db.stats().unwrap();
    assert_eq!(stats.document_count, 3);
}

#[tokio::test]
async fn test_ingest_skips_unsupported_extensions() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    std::fs::write(dir.path().join("image.png"), "not a real png").unwrap();
    std::fs::write(dir.path().join("binary.exe"), "not a real exe").unwrap();

    ingest::ingest_path(dir.path(), &db, &embedder, &config(), &Progress::noop()).unwrap();

    assert_eq!(db.stats().unwrap().document_count, 0);
}

#[tokio::test]
async fn test_ingest_dedup_by_hash() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    let content = "Duplicate content here.";
    std::fs::write(dir.path().join("first.md"), content).unwrap();

    ingest::ingest_path(
        &dir.path().join("first.md"),
        &db,
        &embedder,
        &config(),
        &Progress::noop(),
    )
    .unwrap();
    assert_eq!(db.stats().unwrap().document_count, 1);

    // Ingest the same content again from a different file name
    std::fs::write(dir.path().join("second.md"), content).unwrap();
    ingest::ingest_path(
        &dir.path().join("second.md"),
        &db,
        &embedder,
        &config(),
        &Progress::noop(),
    )
    .unwrap();

    // Still 1 document because the hash matches
    assert_eq!(db.stats().unwrap().document_count, 1);
}

#[tokio::test]
async fn test_ingest_json_file() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    let file_path = dir.path().join("data.json");
    std::fs::write(&file_path, r#"{"key": "value", "num": 42}"#).unwrap();

    ingest::ingest_path(&file_path, &db, &embedder, &config(), &Progress::noop()).unwrap();

    let text = db.get_full_document(file_path.to_str().unwrap()).unwrap();
    // JSON is stored pretty-printed
    assert!(text.contains("\"key\": \"value\""));
}

#[tokio::test]
async fn test_ingest_empty_file_skipped() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    let file_path = dir.path().join("empty.txt");
    std::fs::File::create(&file_path).unwrap();

    ingest::ingest_path(&file_path, &db, &embedder, &config(), &Progress::noop()).unwrap();

    assert_eq!(db.stats().unwrap().document_count, 0);
}

#[tokio::test]
async fn test_ingest_html_strips_tags() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    let file_path = dir.path().join("page.html");
    std::fs::write(
        &file_path,
        "<html><body><h1>Title</h1><p>Content here.</p></body></html>",
    )
    .unwrap();

    ingest::ingest_path(&file_path, &db, &embedder, &config(), &Progress::noop()).unwrap();

    let text = db.get_full_document(file_path.to_str().unwrap()).unwrap();
    assert!(text.contains("Title"));
    assert!(text.contains("Content here."));
    assert!(!text.contains("<html>"));
    assert!(!text.contains("<p>"));
}
