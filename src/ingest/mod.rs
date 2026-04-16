use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::embed::Embedder;
use crate::rag::Database;

pub struct IngestConfig {
    pub chunk_size: usize,
    pub chunk_overlap: usize,
}

impl Default for IngestConfig {
    fn default() -> Self {
        IngestConfig {
            chunk_size: 512,
            chunk_overlap: 64,
        }
    }
}

pub async fn ingest_path(
    path: &Path,
    db: &Database,
    embedder: &Embedder,
    config: &IngestConfig,
) -> Result<()> {
    if path.is_file() {
        ingest_file(path, db, embedder, config)?;
    } else if path.is_dir() {
        let mut success = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;

        for entry in WalkDir::new(path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| is_supported(&ext.to_lowercase()))
                    .unwrap_or(false)
            })
        {
            match ingest_file(entry.path(), db, embedder, config) {
                Ok(true) => success += 1,
                Ok(false) => skipped += 1,
                Err(e) => {
                    eprintln!("[satchel] Failed: {} - {e}", entry.path().display());
                    failed += 1;
                }
            }
        }

        eprintln!(
            "[satchel] Ingestion complete: {success} added, {skipped} unchanged, {failed} failed"
        );
    } else {
        anyhow::bail!("Path does not exist: {}", path.display());
    }
    Ok(())
}

pub async fn watch_and_ingest(
    path: &Path,
    db: &Database,
    embedder: &Embedder,
    config: &IngestConfig,
) -> Result<()> {
    use notify::{Event, EventKind, RecursiveMode, Watcher};

    eprintln!("[satchel] Watching {} for changes...", path.display());

    ingest_path(path, db, embedder, config).await?;

    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(tx)?;
    watcher.watch(path, RecursiveMode::Recursive)?;

    for event in rx {
        match event {
            Ok(event) => {
                if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                    for path in &event.paths {
                        if path.is_file() {
                            eprintln!("[satchel] Change detected: {}", path.display());
                            if let Err(e) = ingest_file(path, db, embedder, config) {
                                eprintln!("[satchel] Ingest error: {e}");
                            }
                        }
                    }
                }
            }
            Err(e) => eprintln!("[satchel] Watch error: {e}"),
        }
    }

    Ok(())
}

fn ingest_file(
    path: &Path,
    db: &Database,
    embedder: &Embedder,
    config: &IngestConfig,
) -> Result<bool> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if !is_supported(&extension) {
        return Ok(false);
    }

    let raw_bytes = std::fs::read(path)?;
    let sha256 = hex_digest(&raw_bytes);

    if db.document_exists_by_hash(&sha256)? {
        return Ok(false);
    }

    let text = extract_text(path, &raw_bytes, &extension)
        .with_context(|| format!("Failed to extract text from {}", path.display()))?;

    if text.trim().is_empty() {
        return Ok(false);
    }

    let doc_id = Uuid::new_v4().to_string();
    let source_path = path.to_string_lossy().to_string();
    let title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled");

    db.insert_document(&doc_id, &source_path, &extension, Some(title), &text, &sha256)?;

    let chunks = chunk_text(&text, config.chunk_size, config.chunk_overlap);

    eprintln!(
        "[satchel] Ingesting: {} ({} chunks)",
        path.display(),
        chunks.len()
    );

    for (i, chunk) in chunks.iter().enumerate() {
        let chunk_id = format!("{doc_id}:{i}");
        let embedding = embedder.embed_with_info(&chunk.text)?;

        db.insert_chunk(
            &chunk_id,
            &doc_id,
            i,
            &chunk.text,
            embedding.token_count,
            chunk.char_start,
            chunk.char_end,
            &embedding.vector,
        )?;
    }

    eprintln!("[satchel] Done: {}", path.display());
    Ok(true)
}

fn extract_text(path: &Path, raw_bytes: &[u8], extension: &str) -> Result<String> {
    match extension {
        "txt" | "md" | "markdown" => Ok(String::from_utf8_lossy(raw_bytes).to_string()),

        "json" => {
            let val: serde_json::Value = serde_json::from_slice(raw_bytes)?;
            Ok(serde_json::to_string_pretty(&val)?)
        }

        "csv" | "tsv" => Ok(String::from_utf8_lossy(raw_bytes).to_string()),

        "html" | "htm" => {
            let html = String::from_utf8_lossy(raw_bytes);
            Ok(strip_html_tags(&html))
        }

        "pdf" => extract_pdf_text(path),

        "docx" => extract_docx_text(path),

        _ => anyhow::bail!("No extractor for .{extension}"),
    }
}

fn extract_pdf_text(path: &Path) -> Result<String> {
    let doc = lopdf::Document::load(path)
        .with_context(|| format!("Failed to open PDF: {}", path.display()))?;

    let mut text = String::new();
    let pages = doc.get_pages();

    for (page_num, _) in pages.iter() {
        match doc.extract_text(&[*page_num]) {
            Ok(page_text) => {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(&page_text);
            }
            Err(e) => {
                tracing::debug!("PDF page {page_num} extraction error: {e}");
            }
        }
    }

    Ok(text)
}

fn extract_docx_text(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("Not a valid DOCX (ZIP): {}", path.display()))?;

    let mut text = String::new();

    if let Ok(mut document_xml) = archive.by_name("word/document.xml") {
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut document_xml, &mut xml)?;
        text = extract_text_from_docx_xml(&xml);
    }

    Ok(text)
}

fn extract_text_from_docx_xml(xml: &str) -> String {
    let mut result = String::new();
    let mut in_text = false;
    let mut in_paragraph = false;
    let mut chars = xml.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut tag = String::new();
            for c in chars.by_ref() {
                if c == '>' {
                    break;
                }
                tag.push(c);
            }

            if tag.starts_with("w:p ") || tag == "w:p" {
                if in_paragraph && !result.ends_with('\n') {
                    result.push('\n');
                }
                in_paragraph = true;
            } else if tag == "/w:p" {
                in_paragraph = false;
            } else if tag.starts_with("w:t") && !tag.starts_with("w:tab") {
                in_text = true;
            } else if tag == "/w:t" {
                in_text = false;
            } else if tag.starts_with("w:tab") || tag.starts_with("w:br") {
                result.push('\t');
            }
        } else if in_text {
            result.push(ch);
        }
    }

    result
}

fn is_supported(extension: &str) -> bool {
    matches!(
        extension,
        "txt" | "md" | "markdown" | "json" | "csv" | "tsv" | "html" | "htm" | "pdf" | "docx"
    )
}

fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut tag_buf = String::new();

    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
            tag_buf.clear();
        } else if ch == '>' {
            in_tag = false;
            let tag_lower = tag_buf.to_lowercase();
            if tag_lower.starts_with("script") {
                in_script = true;
            } else if tag_lower.starts_with("/script") {
                in_script = false;
            } else if tag_lower.starts_with("style") {
                in_style = true;
            } else if tag_lower.starts_with("/style") {
                in_style = false;
            }
        } else if in_tag {
            tag_buf.push(ch);
        } else if !in_script && !in_style {
            result.push(ch);
        }
    }

    collapse_whitespace(&result)
}

fn collapse_whitespace(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    let mut last_was_newline = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !last_was_newline {
                collapsed.push('\n');
                last_was_newline = true;
            }
        } else {
            collapsed.push_str(trimmed);
            collapsed.push('\n');
            last_was_newline = false;
        }
    }
    collapsed
}

struct Chunk {
    text: String,
    char_start: usize,
    char_end: usize,
}

fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let paragraphs: Vec<&str> = text.split("\n\n").collect();

    let mut current_text = String::new();
    let mut current_start = 0;
    let mut char_offset = 0;

    for para in paragraphs {
        let para_tokens = approximate_tokens(para);

        if approximate_tokens(&current_text) + para_tokens > chunk_size && !current_text.is_empty()
        {
            let char_end = char_offset;
            chunks.push(Chunk {
                text: current_text.trim().to_string(),
                char_start: current_start,
                char_end,
            });

            let overlap_text = get_tail_tokens(&current_text, overlap);
            current_text = overlap_text;
            current_start = char_end.saturating_sub(current_text.len());
        }

        if !current_text.is_empty() {
            current_text.push_str("\n\n");
        }
        current_text.push_str(para);
        char_offset += para.len() + 2;
    }

    if !current_text.trim().is_empty() {
        chunks.push(Chunk {
            text: current_text.trim().to_string(),
            char_start: current_start,
            char_end: text.len(),
        });
    }

    chunks
}

fn approximate_tokens(text: &str) -> usize {
    text.len() / 4
}

/// Return the last N approximate tokens of text, respecting UTF-8 char boundaries.
fn get_tail_tokens(text: &str, token_count: usize) -> String {
    let char_count = token_count * 4;
    if text.len() <= char_count {
        return text.to_string();
    }
    // Walk backwards to find a safe char boundary
    let target = text.len() - char_count;
    let start = text.ceil_char_boundary(target);
    text[start..].to_string()
}

fn hex_digest(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_supported() {
        assert!(is_supported("md"));
        assert!(is_supported("txt"));
        assert!(is_supported("pdf"));
        assert!(is_supported("docx"));
        assert!(is_supported("html"));
        assert!(is_supported("json"));
        assert!(is_supported("csv"));
        assert!(!is_supported("exe"));
        assert!(!is_supported("png"));
        assert!(!is_supported(""));
    }

    #[test]
    fn test_hex_digest() {
        let hash = hex_digest(b"hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_hex_digest_empty() {
        let hash = hex_digest(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_approximate_tokens() {
        assert_eq!(approximate_tokens(""), 0);
        assert_eq!(approximate_tokens("abcd"), 1);
        assert_eq!(approximate_tokens("hello world, this is a test"), 6);
    }

    #[test]
    fn test_chunk_text_single_short() {
        let text = "Hello world.";
        let chunks = chunk_text(text, 512, 64);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "Hello world.");
    }

    #[test]
    fn test_chunk_text_respects_paragraphs() {
        let para1 = "A".repeat(300);
        let para2 = "B".repeat(300);
        let text = format!("{}\n\n{}", para1, para2);
        // Each paragraph is ~75 tokens, total ~150, should fit in one 512-token chunk
        let chunks = chunk_text(&text, 512, 64);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_chunk_text_splits_large() {
        let para1 = "A".repeat(2048);
        let para2 = "B".repeat(2048);
        let text = format!("{}\n\n{}", para1, para2);
        let chunks = chunk_text(&text, 512, 64);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn test_strip_html_tags_basic() {
        assert_eq!(
            strip_html_tags("<p>Hello</p>").trim(),
            "Hello"
        );
    }

    #[test]
    fn test_strip_html_tags_nested() {
        assert_eq!(
            strip_html_tags("<div><p>Hello <b>world</b></p></div>").trim(),
            "Hello world"
        );
    }

    #[test]
    fn test_strip_html_tags_script() {
        let html = "<p>Before</p><script>var x = 1;</script><p>After</p>";
        let result = strip_html_tags(html);
        assert!(result.contains("Before"));
        assert!(result.contains("After"));
        assert!(!result.contains("var x"));
    }

    #[test]
    fn test_strip_html_tags_style() {
        let html = "<style>.x{color:red}</style><p>Content</p>";
        let result = strip_html_tags(html);
        assert!(result.contains("Content"));
        assert!(!result.contains("color:red"));
    }

    #[test]
    fn test_extract_docx_xml() {
        let xml = r#"<w:document><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:p><w:r><w:t>World</w:t></w:r></w:p></w:body></w:document>"#;
        let text = extract_text_from_docx_xml(xml);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
    }

    #[test]
    fn test_get_tail_tokens_short_text() {
        let text = "short";
        assert_eq!(get_tail_tokens(text, 100), "short");
    }

    #[test]
    fn test_get_tail_tokens_long_text() {
        let text = "a".repeat(1000);
        let tail = get_tail_tokens(&text, 10); // 10 tokens = 40 chars
        assert_eq!(tail.len(), 40);
    }

    #[test]
    fn test_get_tail_tokens_unicode_safe() {
        // Each emoji is 4 bytes in UTF-8. Ensure we don't split mid-character.
        let text = "prefix_text_here_\u{1F600}\u{1F600}\u{1F600}";
        let tail = get_tail_tokens(text, 2); // 2 tokens = 8 chars target
        // Should not panic, and result should be valid UTF-8
        assert!(!tail.is_empty());
        assert!(tail.is_ascii() || !tail.is_empty());
    }
}
