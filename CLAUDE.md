# SATCHEL Development Rules

## Git Policy
- NEVER add Claude, AI assistants, or any Co-Authored-By lines to git commits
- NEVER add AI-related attribution in commit messages
- Commit messages should be written as if authored solely by the developer

## Architecture
- Rust single-binary, statically linked, no runtime dependencies
- All data lives on the portable drive; never write to the host filesystem
- MCP protocol over stdio (primary) and Streamable HTTP (browser bridge)
- SQLite for storage, embeddings as BLOBs with Rust-side cosine similarity
- candle (pure Rust) for local embeddings; no C deps, no dynamic libraries

## Code Style
- Minimal dependencies; every crate must justify its inclusion
- Error handling via anyhow at boundaries
- No unwrap() in production paths; only in tests or truly unreachable cases
- Modules: mcp/, rag/, embed/, ingest/, vault/

## Testing
- `cargo test` must pass before any commit
- `cargo clippy` must be clean

## Build
- `cargo build --release` for current platform
- `./scripts/build-all.sh` for cross-platform binaries
- Target: static musl binaries on Linux, universal on macOS
