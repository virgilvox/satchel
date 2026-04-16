# SATCHEL

**Portable RAG on a stick.**

Download one file. Feed it your documents. Point your AI at it. Your entire knowledge base is now available as context in Claude, ChatGPT, Cursor, or any MCP-compatible client. No cloud. No API keys. No installation. Everything runs locally.

## Get Started

**1. Download** the binary for your platform from the [latest release](https://github.com/virgilvox/satchel/releases/latest). The embedding model is included. Nothing else to install.

**2. Ingest** your documents:

```bash
chmod +x satchel-*
./satchel-* init personal
./satchel-* ingest ~/Documents/notes/
```

**3. Connect** your AI client (see below).

That's it.

## Connect to Claude Desktop

Add this to your Claude Desktop config (Settings > Developer > Edit Config):

```json
{
  "mcpServers": {
    "satchel": {
      "command": "/full/path/to/satchel",
      "args": ["serve"]
    }
  }
}
```

Restart Claude. Your documents are now searchable in every conversation.

## Connect to Claude Code

```bash
claude mcp add satchel -- /full/path/to/satchel serve
```

## Connect to Cursor

Add to your Cursor MCP config:

```json
{
  "mcpServers": {
    "satchel": {
      "command": "/full/path/to/satchel",
      "args": ["serve"]
    }
  }
}
```

## Connect via Browser (Claude.ai, ChatGPT)

Start the HTTP server:

```bash
satchel serve --transport http
```

Open `http://localhost:7428` for the web UI. The MCP endpoint is at `http://localhost:7428/mcp`. Browser extensions like [claude-mcp](https://github.com/nicepkg/claude-mcp) can bridge this to claude.ai.

## Supported File Types

Markdown, plain text, PDF, DOCX, HTML, CSV, TSV, JSON.

## Multiple Vaults

Keep separate knowledge bases for different contexts:

```bash
satchel vault create work
satchel vault create personal
satchel vault list
satchel vault use work
```

## Alternative Install Methods

### Cargo (builds from source, does not include the model)

```bash
cargo install satchel-rag
./scripts/download-model.sh
```

### Build from source with embedded model

```bash
git clone https://github.com/virgilvox/satchel.git
cd satchel
./scripts/download-model.sh
cargo build --release --features embed-model
```

## How It Works

SATCHEL splits your documents into chunks and generates 384-dimensional vector embeddings using [all-MiniLM-L6-v2](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2), running locally via [candle](https://github.com/huggingface/candle) (pure Rust, zero C dependencies). Chunks and embeddings are stored in a SQLite database. When your AI client asks a question, SATCHEL embeds the query, finds the most relevant chunks by cosine similarity, and returns them as context.

No data leaves your machine. No internet needed after download.

## MCP Tools

When connected, your AI client can use these tools:

| Tool | What it does |
|------|-------------|
| `search_knowledge` | Semantic search across all documents |
| `list_sources` | List ingested documents with metadata |
| `get_document` | Retrieve the full text of a document |
| `list_tags` | List tags and categories |
| `vault_stats` | Storage stats, document counts, model info |

## REST API

Available when running with `--transport http`:

```
GET  /api/status              Server status and vault stats
GET  /api/sources             List ingested documents
POST /api/search              Semantic search  {"query": "...", "top_k": 5}
GET  /api/document?source=... Retrieve full document text
GET  /api/tags                List all tags
POST /mcp                     MCP Streamable HTTP endpoint
```

All endpoints return JSON. CORS is enabled.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[AGPL-3.0-only](LICENSE). For commercial licensing, contact Moheeb Zara at hackbuildvideo@gmail.com.
