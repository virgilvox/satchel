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
  server.rs       HTTP server, REST API, embeds web/dist/index.html
  vault/          Multi-vault management
assets/
  brand/          Logo SVG + PNG renderings
  screenshots/    Screenshots used in README
web/              Svelte 5 + Vite + TypeScript front-end
  src/
    lib/          API/MCP/WebLLM clients, stores, design tokens
    components/   Reusable UI primitives (Mark, Pill, Modal, ...)
    routes/       Tab views (Dashboard, Ask, Chat, Search, ...)
    App.svelte    Root, theme, routing
    main.ts       Mount entry
  dist/           Built single-file bundle (committed; embedded by Rust)
scripts/
  build-all.sh    Cross-platform build script
  download-model.sh  Model download for offline use
```

## Web UI Development

The web UI is a Svelte 5 + Vite + TypeScript single-page app, built into
one self-contained HTML file (`web/dist/index.html`) via
`vite-plugin-singlefile`. The Rust binary embeds it with `include_str!`,
so `cargo build` works without a JS toolchain — but you need `bun` (or
`npm`) installed locally to make changes.

```bash
cd web
bun install
bun run dev      # dev server at http://localhost:5173, proxies /api + /mcp
bun run build    # type-check + bundle into dist/index.html
bun run check    # type-check only
```

After editing UI code, run `bun run build`, commit `web/dist/index.html`
alongside your source changes, and `cargo build` will pick up the new bundle.

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
MIT license. See [LICENSE](LICENSE) for details.
