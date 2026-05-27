mod common;

use satchel_rag::ingest::{self, IngestConfig, Progress};
use tempfile::TempDir;

fn config() -> IngestConfig {
    IngestConfig {
        chunk_size: 512,
        chunk_overlap: 64,
        collection_id: None,
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
async fn test_ingest_csv_emits_one_record_per_row() {
    // Regression guard for the v2.4.2 CSV fix: the old plain-text path
    // returned a small CSV as a single chunk, so search hits returned
    // the entire file. The new archive handler must emit one record
    // (== one chunk) per data row, with the column headers repeated in
    // every record so each chunk is self-describing.
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    let file_path = dir.path().join("contacts.csv");
    std::fs::write(
        &file_path,
        "name,email,role\nAlice,alice@example.com,founder\nBob,bob@example.com,engineer\nCarol,carol@example.com,designer\n",
    )
    .unwrap();

    ingest::ingest_path(&file_path, &db, &embedder, &config(), &Progress::noop()).unwrap();

    // 3 documents (one per row), 3 chunks total.
    let stats = db.stats().unwrap();
    assert_eq!(stats.document_count, 3, "expected one document per row");
    assert_eq!(stats.chunk_count, 3, "expected one chunk per row");

    // Each row's body should carry the headers AND the row's values
    // adjacent, so a search for "alice founder" hits the right row.
    let (records, total) = db
        .list_records_by_source(file_path.to_str().unwrap(), 100, 0)
        .unwrap();
    assert_eq!(total, 3);
    let alice = records
        .iter()
        .find(|r| r.text.contains("Alice"))
        .expect("Alice row missing");
    // Use let-bindings so each `assert!` is short enough that every
    // rustfmt version we ship through (local nightly, CI stable) keeps
    // it on one line. Past releases tripped the fmt CI gate when one
    // form was just barely over the wrap threshold for stable.
    let has_name = alice.text.contains("name: Alice");
    let has_email = alice.text.contains("email: alice@example.com");
    let has_role = alice.text.contains("role: founder");
    assert!(has_name, "header repeated: name");
    assert!(has_email, "header repeated: email");
    assert!(has_role, "header repeated: role");
    // Crucially, Alice's body must NOT contain Bob's data: each row is
    // its own document/chunk.
    assert!(!alice.text.contains("Bob"), "row separation broken");
    assert!(
        !alice.text.contains("bob@example.com"),
        "row separation broken"
    );
}

#[tokio::test]
async fn test_ingest_tsv_uses_tab_delimiter() {
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    let file_path = dir.path().join("data.tsv");
    std::fs::write(&file_path, "name\tage\nAlice\t30\nBob\t25\n").unwrap();

    ingest::ingest_path(&file_path, &db, &embedder, &config(), &Progress::noop()).unwrap();

    assert_eq!(
        db.stats().unwrap().document_count,
        2,
        "TSV must split on tabs, not commas"
    );
}

#[tokio::test]
async fn test_ingest_csv_directory_walk() {
    // Directory walks dispatch via ingest_file; the file-level archive
    // intercept must fire there too, not just at top-level ingest_path.
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    std::fs::write(dir.path().join("a.csv"), "x,y\n1,2\n3,4\n").unwrap();
    std::fs::write(dir.path().join("b.csv"), "p,q\n10,20\n").unwrap();

    ingest::ingest_path(dir.path(), &db, &embedder, &config(), &Progress::noop()).unwrap();

    // a.csv => 2 records, b.csv => 1 record. Total 3.
    assert_eq!(db.stats().unwrap().document_count, 3);
}

#[tokio::test]
async fn test_ingest_assigns_to_collection_when_configured() {
    // When IngestConfig::collection_id is set, every persisted document
    // joins that collection in a single pass. This is what backs the
    // Ingest tab's collection picker and `satchel ingest --collection`.
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    let cid = db.create_collection("work").unwrap();
    std::fs::write(dir.path().join("a.md"), "Document A.").unwrap();
    std::fs::write(dir.path().join("b.md"), "Document B.").unwrap();

    let cfg = IngestConfig {
        chunk_size: 512,
        chunk_overlap: 64,
        collection_id: Some(cid),
    };
    ingest::ingest_path(dir.path(), &db, &embedder, &cfg, &Progress::noop()).unwrap();

    let cols = db.list_collections().unwrap();
    let work = cols.iter().find(|c| c.id == cid).unwrap();
    assert_eq!(work.document_count, 2);

    // A second ingest into the same paths is a no-op (hash dedup); the
    // count should stay at 2, not double up via re-assignment.
    ingest::ingest_path(dir.path(), &db, &embedder, &cfg, &Progress::noop()).unwrap();
    let work2 = db.list_collections().unwrap();
    assert_eq!(
        work2.iter().find(|c| c.id == cid).unwrap().document_count,
        2
    );
}

#[tokio::test]
async fn test_ingest_dedup_skip_still_joins_new_collection() {
    // Regression guard for the dedup-skip semantic: if the user
    // re-ingests an already-known document into a NEW collection, the
    // existing doc should join the new collection. Without this fix,
    // a re-ingest into "personal" after the same content was first
    // ingested into "work" would silently leave "personal" empty.
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    let work = db.create_collection("work").unwrap();
    let personal = db.create_collection("personal").unwrap();

    let path = dir.path().join("note.md");
    std::fs::write(&path, "shared note body").unwrap();

    let cfg_work = IngestConfig {
        chunk_size: 512,
        chunk_overlap: 64,
        collection_id: Some(work),
    };
    ingest::ingest_path(&path, &db, &embedder, &cfg_work, &Progress::noop()).unwrap();
    assert_eq!(
        db.list_collections()
            .unwrap()
            .iter()
            .find(|c| c.id == work)
            .unwrap()
            .document_count,
        1
    );

    let cfg_personal = IngestConfig {
        chunk_size: 512,
        chunk_overlap: 64,
        collection_id: Some(personal),
    };
    ingest::ingest_path(&path, &db, &embedder, &cfg_personal, &Progress::noop()).unwrap();

    // Document is still single (hash dedup), but it now belongs to BOTH collections.
    assert_eq!(db.stats().unwrap().document_count, 1);
    let cols = db.list_collections().unwrap();
    let work_count = cols.iter().find(|c| c.id == work).unwrap().document_count;
    let personal_count = cols
        .iter()
        .find(|c| c.id == personal)
        .unwrap()
        .document_count;
    assert_eq!(work_count, 1);
    assert_eq!(personal_count, 1);
}

#[tokio::test]
async fn test_ingest_archive_csv_assigns_to_collection() {
    // The archive-handler path (csv, slack, mbox, etc.) goes through
    // persist_record, not ingest_file. Cover the CSV archive
    // explicitly so a future refactor that drops the collection_id
    // forwarding from any archive handler trips this test.
    let db = common::test_db();
    let embedder = common::test_embedder();
    let dir = TempDir::new().unwrap();

    let cid = db.create_collection("contacts").unwrap();
    let path = dir.path().join("rows.csv");
    std::fs::write(&path, "name,role\nAlice,founder\nBob,engineer\n").unwrap();

    let cfg = IngestConfig {
        chunk_size: 512,
        chunk_overlap: 64,
        collection_id: Some(cid),
    };
    ingest::ingest_path(&path, &db, &embedder, &cfg, &Progress::noop()).unwrap();

    let stats = db.stats().unwrap();
    assert_eq!(stats.document_count, 2, "one document per CSV row");

    let count = db
        .list_collections()
        .unwrap()
        .iter()
        .find(|c| c.id == cid)
        .unwrap()
        .document_count;
    assert_eq!(
        count, 2,
        "every CSV row should join the requested collection"
    );
}

#[tokio::test]
async fn test_document_id_by_hash_lookup() {
    // The dedup-skip-still-joins-new-collection behavior relies on
    // document_id_by_hash. Pin the contract: returns Some(id) for a
    // known hash, None for an unknown one.
    let db = common::test_db();
    db.insert_document("d1", "/x.md", "md", None, "body", "hash-known")
        .unwrap();
    let known = db.document_id_by_hash("hash-known").unwrap();
    let missing = db.document_id_by_hash("hash-missing").unwrap();
    assert_eq!(known, Some("d1".to_string()));
    assert_eq!(missing, None);
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
