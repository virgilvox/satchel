# Contributing to SATCHEL

Contributions are welcome. This document covers the basics of how to get involved.

## Getting Started

1. Fork the repository and clone your fork.
2. Install the Rust toolchain: https://rustup.rs
3. Build and run tests:

```bash
cargo build
cargo test
cargo clippy
```

4. Download the embedding model for integration testing:

```bash
./scripts/download-model.sh
```

## Development Workflow

- Create a branch from `main` for your work.
- Write tests for new functionality.
- Run `cargo fmt` before committing.
- Run `cargo clippy` and fix any warnings.
- Run `cargo test` and ensure all tests pass.
- Open a pull request against `main`.

## Code Style

- Follow standard Rust conventions.
- Use `anyhow::Result` at public API boundaries.
- Avoid `unwrap()` in production code paths.
- Keep dependencies minimal. Every new crate needs justification.
- Prefer clear code over clever code.

## Project Structure

```
src/
  main.rs         CLI entry point
  embed/          Embedding model inference (candle)
  ingest/         Document parsing and chunking
  mcp/            MCP protocol and stdio transport
  rag/            SQLite database and vector search
  server.rs       HTTP server, REST API, web UI
  vault/          Multi-vault management
assets/
  ui.html         Embedded web interface
scripts/
  build-all.sh    Cross-platform build script
  download-model.sh  Model download for offline use
```

## Supported File Types

When adding support for a new file format:

1. Add the extension to `is_supported()` in `src/ingest/mod.rs`.
2. Add an extraction branch in `extract_text()`.
3. Write tests for the extractor.
4. Document the format in the README.

## Testing

Unit tests live in `#[cfg(test)]` modules within each source file. Run them with:

```bash
cargo test
```

Tests that require the embedding model are integration tests and should be
marked accordingly.

## License

By contributing, you agree that your contributions will be licensed under the
AGPL-3.0 license. See [LICENSE](LICENSE) for details.
