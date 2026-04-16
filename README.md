# SATCHEL

**Self-contained Augmented Text Corpus for Host-free Embedded Lookup**

SATCHEL is a portable RAG (Retrieval-Augmented Generation) system that runs entirely from a USB drive. It turns a folder of documents into a searchable vector database and exposes it to AI clients via the [Model Context Protocol](https://modelcontextprotocol.io) (MCP). One binary, no installation, no cloud, no dependencies on the host machine.

## Install

### Pre-built binaries (recommended)

Download from the [latest release](https://github.com/virgilvox/satchel/releases/latest) for your platform. The embedding model is included in the binary. No additional downloads needed.

```bash
chmod +x satchel-*              # make executable (macOS/Linux)
./satchel-* init personal       # create a vault
./satchel-* ingest ~/notes/     # ingest your documents
./satchel-* serve               # start the MCP server
```

### Cargo

Building from source does not include the embedded model. Download it after install:

```bash
cargo install satchel-rag
./scripts/download-model.sh
```

Or with [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) for a pre-built binary (model included):

```bash
cargo binstall satchel-rag
```

### From source with embedded model

To bake the model into the binary yourself:

```bash
git clone https://github.com/virgilvox/satchel.git
cd satchel
./scripts/download-model.sh
cargo build --release --features embed-model
```

The binary is at `target/release/satchel` (~100 MB with embedded model).

## Quick Start

```bash
# Create a vault
satchel init personal

# Ingest documents
satchel ingest ~/notes/

# Get the config snippet for your AI client
satchel config --client claude-desktop

# Or launch the web UI and HTTP server
satchel serve --transport http
# Open http://localhost:7428
```

## How It Works

SATCHEL reads your documents, splits them into chunks, and generates vector embeddings using [all-MiniLM-L6-v2](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2) (~90 MB). Embeddings are computed locally with [candle](https://github.com/huggingface/candle) (pure Rust, no C dependencies). Chunks and their embeddings are stored in a SQLite database on the drive.

When an AI client sends a query, SATCHEL embeds the query, computes cosine similarity against all stored chunks, and returns the most relevant results. The AI client uses these results as context for its response.

No data leaves the drive. No API keys are needed. No internet connection is required after the initial model download.

## Connecting AI Clients

SATCHEL supports two transports: stdio (for desktop MCP clients) and HTTP (for browser-based AI tools and the web UI).

### Claude Desktop / Cursor

Add this to your MCP configuration:

```json
{
  "mcpServers": {
    "satchel": {
      "command": "/path/to/start.sh",
      "args": ["serve", "--transport", "stdio"]
    }
  }
}
```

### Claude Code

```bash
claude mcp add satchel -- /path/to/start.sh serve --transport stdio
```

### Browser AI Clients

Start the HTTP server:

```bash
satchel serve --transport http --port 7428
```

This starts:

- **Web UI** at `http://localhost:7428`
- **MCP endpoint** at `http://localhost:7428/mcp` (Streamable HTTP)
- **REST API** at `http://localhost:7428/api/search`

Browser extensions such as [claude-mcp](https://github.com/nicepkg/claude-mcp) can bridge local MCP servers to claude.ai. For ChatGPT, point a Custom GPT action at the REST API.

## MCP Tools

When connected, SATCHEL provides these tools to the AI client:

| Tool | Description |
|------|-------------|
| `search_knowledge` | Semantic search across all documents. Supports filtering by source and tags. |
| `list_sources` | List all ingested documents with metadata. |
| `get_document` | Retrieve the full text of a specific document. |
| `list_tags` | List all tags and categories with document counts. |
| `vault_stats` | Show storage statistics, document and chunk counts, and model information. |

## REST API

When running with `--transport http`, the following endpoints are available:

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/status` | Server status and vault statistics |
| GET | `/api/sources` | List ingested documents |
| POST | `/api/search` | Semantic search (body: `{"query": "...", "top_k": 5}`) |
| GET | `/api/document?source=...` | Retrieve full document text |
| GET | `/api/tags` | List all tags |
| POST | `/mcp` | MCP Streamable HTTP endpoint |

All endpoints return JSON. CORS is enabled for browser access.

## Supported File Types

Markdown, plain text, PDF, DOCX, HTML, CSV, TSV, JSON.

## Multiple Vaults

Organize your knowledge into separate vaults:

```bash
satchel vault create work
satchel vault create personal
satchel vault list
satchel vault use work
```

The active vault determines which knowledge base is exposed to AI clients.

## Drive Layout

```
USB Drive/
  start.sh                         Platform launcher script
  bin/
    satchel-linux-x86_64
    satchel-linux-aarch64
    satchel-macos-x86_64
    satchel-macos-aarch64
    satchel-windows-x86_64.exe
  vault/
    satchel.toml                   Drive configuration (active vault)
    models/
      all-MiniLM-L6-v2/           Embedding model (~90 MB)
        model.safetensors
        tokenizer.json
        config.json
    vaults/
      personal/
        satchel.db                 SQLite database with embeddings
        inbox/                     Drop files here for ingestion
      work/
        satchel.db
        inbox/
```

## Architecture

```
  AI Client (Claude, ChatGPT, Cursor, etc.)
       |
       | MCP (stdio or HTTP)
       v
  SATCHEL Binary (~11 MB, zero runtime dependencies)
    +------------------+  +----------------------+
    | MCP Server       |  | Embedder             |
    | + Web UI         |  | candle (pure Rust)   |
    | + REST API       |  | all-MiniLM-L6-v2     |
    +--------+---------+  +----------+-----------+
             |                       |
    +--------v-----------------------v-----------+
    | SQLite Database                             |
    | documents > chunks > embeddings (384-dim)   |
    +---------------------------------------------+
             ^
             | reads from portable drive
             |
    USB Drive / External Storage
```

## Cross-Platform Builds

Build for all supported platforms:

```bash
./scripts/build-all.sh
```

Targets: Linux (x86_64, aarch64), macOS (x86_64, aarch64), Windows (x86_64).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, code style, and testing guidelines.

## License

SATCHEL is licensed under the [GNU Affero General Public License v3.0](LICENSE) (AGPL-3.0-only).

For commercial licensing under different terms, contact Moheeb Zara at hackbuildvideo@gmail.com.
