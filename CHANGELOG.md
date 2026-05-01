# Changelog

## v0.2.1 — 2026-04-30

Quality-of-life pass on top of v0.2.0.

### Default vault location

`./vault` is no longer the implicit default. New resolution order:

1. `--vault PATH` if explicit
2. `<binary-dir>/vault/` if it exists — preserves USB-stick deployments where
   the binary and its vault travel together
3. Platform data directory:
   - macOS: `~/Library/Application Support/satchel`
   - Linux/BSD: `$XDG_DATA_HOME/satchel` or `~/.local/share/satchel`
   - Windows: `%APPDATA%\satchel`

The chosen path prints on first stderr line at every launch (`[satchel] Vault: ...`).
If a probable legacy vault exists at `./vault` or `~/vault` and isn't the chosen
one, a hint prints with the exact `--vault` flag to keep using it. Eliminates
the foot-gun where `./vault` resolved against the shell's PWD (typically `~`)
not the binary's directory, leaving users hunting for their data.

### Slack handler — burst-bundling

Consecutive messages from the same user within 90 seconds collapse into one
chunk with continuation-line formatting (`[hh:mm]: extra text` after the
header). Captures stream-of-consciousness sequences ("hey", "actually nvm",
"and one more thing") as the single thought they actually are. Thread parents
are not bundled with adjacent messages — they own their replies via the
thread mechanism. `BURST_GAP_SECS = 90.0`.

### Conversation context viewer

Each search result now has a "Show context" link that opens a modal with the
full sequence of records at that source. The matched message is highlighted
and scrolled into view. Backed by a new endpoint:

```
GET /api/conversation?source=<path>&limit=<n>
→ {source, records: [{id, title, text, ingested_at}], limit}
```

Records come back in ingest order, which for chronological archive handlers
(Slack, Discord, mbox) is chronological message order. Capped at 2000
records per response by default, 10000 max — generous enough that a single
day's archive viewer sees everything, bounded enough to not OOM the modal.

### Documents tab — grouping and pagination

After v0.2.0's Slack handler started emitting one document per message, the
Documents page was a 50,000-row table that locked up the UI. The fix:

- `list_sources` now groups by `source_path` in SQL — a Slack daily file with
  50 messages renders as one row with `record_count = 50`, not 50 rows.
- `GET /api/sources` accepts `?q=<substring>&filter_type=&sort_by=&limit=&offset=`.
- The Documents tab has a path-substring filter (debounced), file-type
  dropdown, sort selector, and Load More pagination at 50 rows per page.
- New `Records` column shows N when a single file produced multiple records.
- Underscores and percents in the path filter are SQL-escaped (regression
  test added).

### Search pagination

`POST /api/search` now accepts `offset` and returns `{results, total, offset, limit}`.
The web UI defaults to 20 results per page and shows a Load More button until
all matches are exhausted; "Showing N of M" stays visible. Default `top_k`
bumped from 5 to 20; max raised from 20 to 100. MCP `search_knowledge` tool
prepends "Showing top N of M matches" when there's a long tail.

### Slack handler — no more header-only chunks

Messages with no `blocks` and empty `text` fields (file shares, link unfurls,
sticker-only messages) used to ingest as `[date #channel @user]: ` with nothing
after the colon, drowning real content in BM25 rankings (BM25 favors short
documents — those header-only chunks ranked first for any username query).
Now `extract_text` falls through to legacy `attachments[]` (title/text/fallback),
then `files[]` (title/name/mimetype), and only persists if something is found.

**Cleaning up existing data**: empty records ingested before this fix remain
in the vault. To clean: `satchel delete --prefix "<your slack export path>"`
then re-ingest. The dedup hash makes re-ingest fast for chunks that didn't
change.

### Auto-open the web UI on launch

Running `./satchel` (with no args, or `serve --transport http`) now opens the
default browser to `http://localhost:7428` ~250 ms after the listener binds.
Suppressed automatically when stderr is not a TTY (so `nohup`, CI, headless
servers, and SSH-without-DISPLAY don't try to launch anything), and
suppressed by `--no-browser` when you want to keep it quiet anyway.

Cross-platform shell-out: `open` on macOS, `xdg-open` on Linux,
`cmd /C start "" <url>` on Windows. Best-effort — failure to launch is
logged but never fatal.

### Live progress tracking for ingests

Submitting an ingest used to block the HTTP request until the entire
archive was processed. Now `POST /api/ingest` returns a `job_id` immediately
and the UI's new **Jobs** panel shows live counters: files seen, records
added/skipped/failed, current file, archive kind detected, elapsed time,
and final outcome. You can queue several folders and watch them all at
once.

Internally:
- New `Progress` callback type threaded through `ingest_path`, the archive
  dispatcher, and every format handler. Default callers pass `Progress::noop()`.
- New `JobRegistry` (in-memory, per-process) holds up to 100 recent jobs.
  Older completed/failed entries roll off; active jobs are never evicted.
- Progress events: `ArchiveDetected`, `FileStarted`, `RecordAdded`,
  `RecordSkipped`, `RecordFailed`. The HTTP layer turns these into atomic
  counter updates on the active `Job`.

### Smaller audit fixes

- `/api/browse` and `/api/ingest` now resolve `~/` against `HOME` first then
  `USERPROFILE` so Windows users without an `HOME` env var work.
- Jobs whose every record failed (typically: embedding model unavailable) are
  now marked `failed` with a useful error message instead of `completed` with
  a silent zero added.
- The Browse modal lets you click files, not just folders — useful for
  single-file archives (mbox, `_chat.txt`, Discord JSON exports).

### New REST endpoints

```
GET    /api/jobs                 List recent ingest jobs (newest first)
GET    /api/jobs/:id             One job's full state
POST   /api/ingest               Now returns {job_id, status} immediately
POST   /api/search               Now accepts {offset?} and returns {total, offset, limit}
GET    /api/sources              Now accepts ?q=&filter_type=&sort_by=&limit=&offset=,
                                 returns {sources, total, offset, limit}; sources
                                 are now grouped by source_path with record_count
GET    /api/conversation         Returns ?source=<path> records in ingest order
```

### Migration

No database migration. The new endpoints are additive. The shape of the
`POST /api/ingest` response changed from `{status, documents, chunks}` to
`{job_id, status: "pending"}`; clients should now poll `/api/jobs/:id` for
final counts. The CLI `satchel ingest` is unchanged.

---

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
