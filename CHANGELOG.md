# Changelog

## v0.2.0 — 2026-04-30

The "search actually works" release. Diagnosed against a 9318-chunk Slack
archive where a search for the user's own username returned zero results.

### Hybrid retrieval

Search now runs **dense (cosine) + sparse (BM25)** retrieval and fuses
results via Reciprocal Rank Fusion. Pure dense retrieval with a small
embedding model fails on proper nouns, project names, identifiers — exactly
the queries you reach for first ("did I post about lumencanvas?", "what did
@alice say about deploys?"). Hybrid fixes that without giving up semantic
matching.

- New SQLite FTS5 virtual table mirrors `chunks.text`, populated transparently
  on first open after upgrade. No re-ingest needed for the search win.
- Both legs run in parallel; ranks fused with RRF (k=60, the standard).
- The dense leg ranks the entire corpus before fusion so `filter_source` /
  `filter_tags` keep working at any depth.

### Format-aware archive ingest

Folders matching a known archive layout are detected and parsed structurally
instead of being ingested as raw JSON/text blobs:

| Archive | Detection | What's emitted |
|---|---|---|
| Slack workspace export | `users.json` + `channels.json` | One chunk per message; `@username (Display Name)`, `#channel`, dates resolved; threads glued to parent. |
| ChatGPT data export | `conversations.json` + `user.json` + `message_feedback.json` | Active branch only; system messages skipped. |
| Claude.ai data export | `conversations.json` + `users.json` + `projects.json` | Flat conversation; attachment-extracted text inlined. |
| Discord (DiscordChatExporter) | `{guild, channel, messages}` JSON | One chunk per message; embeds attached. |
| WhatsApp chat export | `_chat.txt` | Date format auto-detected (12h/24h, M/D vs D/M). |
| Email mbox | `.mbox` or `From ` separator | Per-email with From/To/Subject/Date/Labels header. |

Archive detection runs before per-file walking; unknown directories fall back
to the previous text/PDF/DOCX/HTML/CSV/JSON pipeline.

### Data management

- New CLI: `satchel delete <path>`, `satchel delete --prefix <p>`,
  `satchel delete --type <ext>`, `satchel clear`. All support `--dry-run`
  and `--yes`; bulk operations prompt by default.
- New REST: `DELETE /api/sources`, `POST /api/clear` (requires explicit
  `confirm: true`).
- New Web UI: Manage tab with prefix/type delete, plus a Danger Zone
  with typed confirmation for vault wipe.
- Deletes are transactional: chunks, tags, and documents drop in one
  SQLite transaction so partial failure can't leave orphan rows.
- LIKE patterns from user input are escaped — `--prefix "/notes_2024"`
  matches that exact prefix, not `/notesX2024` (regression test included).
- Delete is deliberately **not** exposed via MCP. An AI client should not
  be able to remove your data.

### Web UI

- New **Ingest** tab — paste a path or use the Browse modal to navigate
  the local filesystem server-side and pick a folder. Format detection
  runs automatically.
- New **Manage** tab — see "Data management" above.
- Connect tab: replaced the unverified "claude-mcp browser extension"
  reference with honest tunnel + Custom Connector instructions for
  claude.ai web (Cloudflare Tunnel or ngrok), and a clearer pointer to
  Claude Desktop as the simpler local-MCP path.

### Release packaging

- Binaries now ship as `.zip` on all three platforms. Zip preserves the
  Unix exec bit (the OLD raw-binary downloads stripped it because GitHub
  serves release assets over plain HTTP). One format, native
  double-click extract everywhere.
- README install steps updated; `chmod +x` no longer needed.

### Migration notes

If you're upgrading from v0.1.0 with an existing vault:

1. Replace your binary with the new one and start it. The first launch
   prints `[satchel] Building keyword index for N existing chunks...` —
   this is the FTS5 backfill running once.
2. Hybrid search works immediately on existing chunks. If those chunks
   are raw JSON (e.g. ingested from a Slack export under v0.1.0), search
   results will improve but text will still look like JSON.
3. To get full Slack-aware records, wipe and re-ingest:
   `satchel delete --prefix "<your slack export path prefix>"`
   then `satchel ingest <same path>`. The new handler will detect the
   export structure and emit one record per message.

### New REST endpoints

```
DELETE /api/sources    {path? | prefix? | file_type?, dry_run?}
POST   /api/clear      {confirm? | dry_run?}
GET    /api/browse     ?path=...      Server-side directory listing
POST   /api/ingest     {path}         Run ingestion on a path
```

### Tests

114 unit + 23 integration tests pass. Adds regression coverage for:
hybrid search surfacing rare tokens, FTS5 trigger cleanup on delete,
LIKE-pattern wildcard escaping, deep-rank filter correctness, and
the format-detection signatures of every archive handler.

---

## v0.1.0

Initial release. Single-binary local RAG, embeddings via candle, MCP over
stdio + Streamable HTTP, web UI for search and ingestion overview.
