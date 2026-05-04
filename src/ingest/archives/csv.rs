//! CSV / TSV ingest handler.
//!
//! Detection: file extension `.csv` or `.tsv` (case-insensitive).
//!
//! Emission: one `Record` per data row. Each record's body starts with
//! a normalized header line (`csv: <filename> row <n>`) followed by the
//! column-name / value pairs of that row, one per line. This makes
//! every chunk self-describing for both embeddings (the column names
//! act as semantic anchors) and BM25 (queries like "rows where status
//! is paid" hit the field name and value as adjacent tokens).
//!
//! The first row is always treated as the header. Files without a true
//! header row will lose that row to header treatment; document accordingly.
//!
//! Plain-text fallback in `ingest::extract_text` previously chunked
//! whole CSVs through `chunk_text(512, 64)`. Small CSVs ended up as
//! one undifferentiated chunk; large ones lost the header on every
//! chunk past the first. This handler intercepts before that path.
//!
//! For files larger than `MAX_RECORDS_PER_CSV` we fall back to grouping
//! `ROW_GROUP_SIZE` rows per record so a 100k-row log does not produce
//! 100k embedding calls. Each grouped record still carries the header
//! line at the top and the contiguous row range.

use anyhow::Result;
use std::path::Path;

use super::{persist_record, ArchiveStats, Record};
use crate::embed::Embedder;
use crate::ingest::progress::Progress;
use crate::rag::Database;

/// Above this many data rows, switch from one-record-per-row to
/// grouped records so embedding cost stays bounded.
const MAX_RECORDS_PER_CSV: usize = 10_000;

/// When grouping kicks in, how many rows per emitted record.
const ROW_GROUP_SIZE: usize = 50;

pub fn detect(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("csv") | Some("tsv")
    )
}

pub fn ingest(
    path: &Path,
    db: &Database,
    embedder: &Embedder,
    progress: &Progress,
) -> Result<ArchiveStats> {
    let bytes = std::fs::read(path)?;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let delimiter: u8 = if extension == "tsv" { b'\t' } else { b',' };
    let file_type = if extension == "tsv" { "tsv" } else { "csv" };

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let source_path = path.to_string_lossy().to_string();

    // First pass: read header + count + materialize rows. CSV parsing
    // is stateful enough that reading twice is cleaner than trying to
    // decide grouping mid-stream.
    let (headers, rows) = parse_csv(&bytes, delimiter)?;

    let mut stats = ArchiveStats::default();

    if rows.is_empty() {
        // Header-only or empty file. Nothing to embed.
        return Ok(stats);
    }

    let group_size = if rows.len() > MAX_RECORDS_PER_CSV {
        ROW_GROUP_SIZE
    } else {
        1
    };

    for (group_idx, chunk) in rows.chunks(group_size).enumerate() {
        let row_range_start = group_idx * group_size + 1;
        let row_range_end = row_range_start + chunk.len() - 1;

        let title = if group_size == 1 {
            format!("{filename} row {row_range_start}")
        } else {
            format!("{filename} rows {row_range_start}..{row_range_end}")
        };

        let mut body = format!("{file_type}: {title}\n");
        for (i, row) in chunk.iter().enumerate() {
            let absolute_row = row_range_start + i;
            if group_size > 1 {
                body.push_str(&format!("\n-- row {absolute_row} --\n"));
            } else {
                body.push('\n');
            }
            for (j, value) in row.iter().enumerate() {
                let column_name = headers
                    .get(j)
                    .cloned()
                    .unwrap_or_else(|| format!("col_{}", j + 1));
                body.push_str(&column_name);
                body.push_str(": ");
                body.push_str(value);
                body.push('\n');
            }
        }

        let record_id = if group_size == 1 {
            format!("{source_path}#row{row_range_start}")
        } else {
            format!("{source_path}#rows{row_range_start}-{row_range_end}")
        };

        let record = Record {
            source_path: source_path.clone(),
            record_id,
            title,
            body,
        };

        if persist_record(&record, file_type, db, embedder, progress)? {
            stats.records_added += 1;
        } else {
            stats.records_skipped += 1;
        }
    }

    Ok(stats)
}

fn parse_csv(bytes: &[u8], delimiter: u8) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(true)
        // Tolerant: accept rows with fewer or more fields than the
        // header. Pad / truncate when emitting records.
        .flexible(true)
        .from_reader(bytes);

    let headers: Vec<String> = reader
        .headers()?
        .iter()
        .map(|s| s.trim().to_string())
        .collect();

    let mut rows: Vec<Vec<String>> = Vec::new();
    for record in reader.records() {
        // Don't fail the whole file on one bad row; csv crate sometimes
        // chokes on an embedded null byte or a stray quote near EOF.
        let Ok(record) = record else {
            continue;
        };
        let row: Vec<String> = record.iter().map(|s| s.to_string()).collect();
        // Skip fully-empty rows (trailing newlines, blank separators).
        if row.iter().all(|f| f.trim().is_empty()) {
            continue;
        }
        rows.push(row);
    }
    Ok((headers, rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_csv_with_header() {
        let csv = b"name,age,city\nAlice,30,Phoenix\nBob,25,Tucson\n";
        let (headers, rows) = parse_csv(csv, b',').unwrap();
        assert_eq!(headers, vec!["name", "age", "city"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["Alice", "30", "Phoenix"]);
        assert_eq!(rows[1], vec!["Bob", "25", "Tucson"]);
    }

    #[test]
    fn parse_handles_quoted_fields_with_commas() {
        // Embedded commas inside quoted fields must not split the row.
        let csv = b"name,note\nAlice,\"hello, world\"\nBob,plain\n";
        let (headers, rows) = parse_csv(csv, b',').unwrap();
        assert_eq!(headers, vec!["name", "note"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["Alice", "hello, world"]);
    }

    #[test]
    fn parse_tsv_uses_tab_delimiter() {
        let tsv = b"name\tage\nAlice\t30\nBob\t25\n";
        let (headers, rows) = parse_csv(tsv, b'\t').unwrap();
        assert_eq!(headers, vec!["name", "age"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["Alice", "30"]);
    }

    #[test]
    fn parse_skips_blank_rows() {
        let csv = b"a,b\n1,2\n\n,\n3,4\n";
        let (_, rows) = parse_csv(csv, b',').unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["1", "2"]);
        assert_eq!(rows[1], vec!["3", "4"]);
    }

    #[test]
    fn parse_empty_file_yields_empty() {
        let (headers, rows) = parse_csv(b"", b',').unwrap();
        assert!(headers.is_empty());
        assert!(rows.is_empty());
    }

    #[test]
    fn parse_header_only_yields_no_rows() {
        let (headers, rows) = parse_csv(b"a,b,c\n", b',').unwrap();
        assert_eq!(headers, vec!["a", "b", "c"]);
        assert!(rows.is_empty());
    }

    #[test]
    fn parse_flexible_row_widths() {
        // Row 2 has fewer fields than the header; the parser keeps it
        // and our emit code pads / truncates against the header.
        let csv = b"a,b,c\n1,2,3\n4,5\n";
        let (_, rows) = parse_csv(csv, b',').unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["1", "2", "3"]);
        assert_eq!(rows[1], vec!["4", "5"]);
    }

    #[test]
    fn detect_recognizes_csv_and_tsv_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        let csv_path = tmp.path().join("data.csv");
        std::fs::write(&csv_path, b"a,b\n1,2\n").unwrap();
        let tsv_path = tmp.path().join("data.tsv");
        std::fs::write(&tsv_path, b"a\tb\n1\t2\n").unwrap();
        let other_path = tmp.path().join("data.txt");
        std::fs::write(&other_path, b"a,b\n").unwrap();

        assert!(detect(&csv_path));
        assert!(detect(&tsv_path));
        assert!(!detect(&other_path));
        assert!(!detect(tmp.path())); // a directory, not a file
    }

    #[test]
    fn detect_is_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("DATA.CSV");
        std::fs::write(&path, b"a,b\n1,2\n").unwrap();
        assert!(detect(&path));
    }
}
