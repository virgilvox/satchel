# Changelog

## v1.0.0 — 2026-05-01

Stability declaration. Same code as `v0.3.5` (plus the post-tag fmt
fix on `main`), just declared stable. Everything that was broken in the
0.3.x rapid-iteration cycle is fixed:

- Web UI is a real Svelte 5 + Vite SPA with seven tabs (Dashboard,
  Ask, Chat, Search, Documents, Ingest, Manage, Connect), bundled to
  a single self-contained HTML and embedded into the Rust binary.
- Default embedder is `BAAI/bge-small-en-v1.5` (384-d, MTEB ~62).
  Loader still recognizes `all-MiniLM-L6-v2` for backward compat.
  Long inputs (mbox emails, big PDFs) truncate cleanly to 512 tokens
  instead of crashing the position-embedding lookup.
- In-browser Chat runs WebLLM models entirely in WebGPU and drives
  tool calling via constrained decoding (XGrammar logit mask on a
  per-tool agent JSON-schema). Works with any model in the curated
  list, not just WebLLM's Hermes whitelist.
- Executable branding works the way each OS allows: Windows embeds
  the icon directly in the `.exe`, macOS ships a `Satchel.app`
  bundle (codesigned, packed with `ditto`), Linux ships a `.desktop`
  file plus a hicolor PNG.
- Theme tokens, the punk/displaced notched-pin mark, JetBrains Mono,
  and dark/light mode persist through `localStorage` with
  `prefers-color-scheme` fallback.
- Mobile-first responsive layout — Playwright walk at 390 px viewport
  reports zero overflow and zero console errors on every tab.

No breaking changes from `v0.3.5`. The MCP wire format, REST API,
SQLite schema, embedding dimension, and CLI flags are all preserved.

## v0.3.5 — 2026-05-01

### Icon actually shows on the executable (per-OS, the only way each OS allows)

v0.3.3 shipped `.icns` and `.png` files alongside the binary in the
release zip. That was useless: macOS still drew the generic Unix-exec
icon on `satchel-macos-aarch64`, and Linux still drew its generic ELF
chevron, because **a raw single-file binary cannot host an icon
resource on either OS**. v0.3.5 ships each platform the way that
platform actually wants:

- **macOS** — the zip now contains `Satchel.app`, a real bundle:
  `Contents/MacOS/satchel` is the binary, `Contents/Resources/satchel.icns`
  is the icon, `Contents/Info.plist` declares them, and the bundle is
  ad-hoc `codesign`'d so Gatekeeper accepts the signature (full
  notarization still requires a paid Developer ID — README documents
  the right-click → Open workaround). Packed with `ditto -c -k
  --keepParent --sequesterRsrc` to preserve bundle metadata. Finder
  shows the SATCHEL mark; double-click launches the server and opens
  the browser. Terminal users call the inner binary directly:
  `Satchel.app/Contents/MacOS/satchel`.
- **Windows** — already correct since v0.3.3 (icon embedded in the
  `.exe` via `winresource` + `build.rs`). No change.
- **Linux** — ELF binaries cannot host an icon resource period; the
  Linux convention is a `.desktop` entry referencing a PNG from the
  hicolor theme. The release zip now ships `satchel.desktop` and
  `satchel.png` alongside the binary, with copy-paste install steps in
  the README that put them in `~/.local/share/{applications,icons}/`.

### Bundle-aware auto-open

Double-clicking `Satchel.app` from Finder runs with no controlling TTY,
so the existing "open browser when stderr is a terminal" gate would
have left the user staring at a blank screen. `main.rs` now also opens
the browser when `__CFBundleIdentifier` is set (Finder launch) or the
binary path contains `.app/Contents/MacOS` (older macOS fallback).
Headless / CI / piped-stderr launches continue to skip auto-open.

### README + screenshots

- New ingest screenshot (running mbox job, post-truncation-fix)
  captured by the user on a real vault — added to the gallery.
- "Get Started" rewritten with per-OS install paths.
- Release notes follow the same per-OS structure.

## v0.3.4 — 2026-05-01

### Chat: constrained-mode tool calling that works with any model

v0.3.x shipped a chat that broke instantly on every model except the
five Hermes IDs WebLLM whitelists for its native `tools=` API
("ChatCompletionRequest.tools is not supported for Llama-3.2-1B-…").
That whole path is now dead. The new flow is lifted from
`mockups/satchel-chat (15).html`, which has been mostly-functional all
along — I just hadn't read it carefully enough.

Output is constrained at the logit level via WebLLM's `response_format`
+ XGrammar:

- `lib/agent.ts` — builds an agent JSON-schema that locks the model's
  output to `{thought, tool_call: {name, arguments}}`. The `tool_call`
  is an `anyOf` over per-tool variants where `name` is a const string
  and `arguments` matches that tool's actual parameter shape (with
  XGrammar-incompatible bits like `minItems`/`maxItems`/`pattern`
  stripped). A `respond_to_user` pseudo-tool with `{answer}` lets the
  model signal "I'm done."
- Loose-schema fallback when XGrammar rejects an MCP tool's input
  schema even after sanitization.
- The system prompt enumerates tools, anti-narration rules, and
  persistence rules — verbatim adapted from the mockup's
  `constrainedSystemPrompt()`.
- The chat loop streams `thought` live so the user sees the model
  reasoning instead of a static placeholder, then either dispatches
  the parsed tool call to MCP and loops, or terminates on
  `respond_to_user`.

Curated model list bumped to 11 entries (Llama 3.2 1B/3B, Hermes 3
3B/8B, Hermes 2 Pro Llama/Mistral 8B, Qwen 2.5 3B, Phi 3.5 mini,
Gemma 2 2B, Llama 3.1 8B, DeepSeek R1 Distill 7B). Every one drives
tool calls correctly under constrained mode, not just the FC-tuned
Hermes variants.

### Chat: layout fixes the user kept asking for

- **Clear chat is no longer hidden in a sidebar section.** It lives in
  a strip directly above the chat — alongside live status pills
  (model, MCP, round counter) — and only appears once there's a
  transcript to clear.
- **Mobile-first.** The 280 px settings rail collapses off-canvas at
  ≤880 px and slides in from a `☰ MODEL · MCP` toggle in the strip.
  Backdrop closes it; the close-x button does too. All other tabs
  pass a 390 px-viewport audit (Playwright walks every tab; zero
  elements overflow the viewport, zero console errors, zero page
  errors).

### Embed: 512-token truncation (was crashing on long mbox emails)

`index-select invalid index 512 with dim size 512` — BERT models in
the registry have `max_position_embeddings: 512`, but the tokenizer
happily produced multi-thousand-token sequences for long emails. The
position-embedding lookup blew up. Fixed by truncating
`input_ids`/`attention_mask`/`token_type_ids` to 512 in
`run_inference` and replacing the final WordPiece with `[SEP]` (token
102, same in BGE/MiniLM) so the model still sees a sentence end.
Verified end-to-end with a 67 kB synthetic input — produces a clean
unit-norm 384-d vector instead of crashing.

### README screenshots refreshed

New gallery: dashboard / ask / chat / search / documents / connect.
Captured against the v0.3.4 build via Playwright at 1440×900 @ 2x.

## v0.3.3 — 2026-05-01

### Executable icons

- Brand the binary itself: a new `assets/brand/icon.svg` (solid amber
  notched-pin silhouette with mint data lines, designed to read at OS-icon
  sizes) is rendered at 16/32/48/64/128/256/512/1024 px and packed into:
  - `assets/brand/satchel.ico` — multi-resolution Windows icon, embedded
    into the `.exe` via a new `build.rs` + the `winresource` build-dep
    (target-gated so it only runs on Windows targets and degrades cleanly
    on non-MSVC hosts).
  - `assets/brand/satchel.icns` — macOS icon resource, built with
    `iconutil` and shipped alongside the binary in the release zip
    (a raw CLI binary cannot host an icon without a `.app` bundle).
  - `assets/brand/icon-*.png` — multi-size PNGs for Linux `.desktop`
    files; the 256 px variant ships as `satchel-icon.png` in the linux
    release zips.
- Release workflow (`.github/workflows/release.yml`) now copies the right
  icon flavor into the per-target zip alongside the binary.

## v0.3.2 — 2026-05-01

### Audit fixes (caught by a Playwright walk-through of the rebuilt bundle)

- **Chat: cancel/busy race.** `cancel()` set `busy = false` synchronously,
  which left a window where the user could press Send before the
  in-flight `runTurn` finished unwinding — two concurrent turns would
  both manipulate the transcript. Now `cancel()` only signals abort and
  calls `engine.interruptGenerate()`; `send()`'s `finally` block remains
  the single source of truth for `busy`.
- **Ingest: timer race after async unmount.** The poll could resolve and
  schedule a new `setInterval` *after* the cleanup ran on tab switch,
  leaking a 1.5s tick. Tracked an `alive` flag and check it on every
  await-resume.
- **Stores: localStorage in private/sandboxed contexts.** Wrapped
  `localStorage.{get,set}Item` in try/catch and gave `prefers-color-scheme`
  the same treatment so the whole module load can't crash on a hostile
  storage API.
- **Reasoning regex.** Tolerate leading whitespace/newline before the
  opening `<think>` tag — DeepSeek-distill emits one — both at stream
  time (live render) and post-stream (final extract).

## v0.3.1 — 2026-05-01

### Hotfix

- **Web UI was blank with `Uncaught ReferenceError: $state is not defined` in v0.3.0.** Svelte 5 runes only run through the compiler when they appear in `.svelte`, `.svelte.js`, or `.svelte.ts` files; the shared store module was named `lib/stores.ts`, so its `$state(...)` calls were left as literal references and the app crashed on first import. Renamed to `lib/stores.svelte.ts` and updated the five importers. svelte-check still passes (it only validates types, not the rune transform site).

## v0.3.0 — 2026-05-01

### Web UI · Svelte 5 + Vite framework

- The single `assets/ui.html` is replaced by a proper front-end project at
  `web/`: Svelte 5 (runes) + TypeScript + Vite. `vite-plugin-singlefile`
  emits one self-contained HTML at `web/dist/index.html` with all CSS + JS
  inlined, so the Rust binary still embeds a single file via `include_str!`
  and ships as one statically-linked binary.
- Strong separation of concerns: design tokens live in `web/src/lib/tokens.css`,
  API/MCP/WebLLM clients in `web/src/lib/`, reusable primitives in
  `web/src/components/` (Mark, Pill, Dot, Modal, MessageBubble, ToolCallCard,
  ReasoningBlock, Composer, …), per-tab views in `web/src/routes/`.
- New **Chat** tab: full in-browser LLM with tool calling against the local
  MCP server. The chat client picks a small WebLLM model (Llama 3.2 1B/3B,
  Hermes 3 3B, Qwen 2.5 3B, Phi 3.5 mini, DeepSeek R1 distill 7B), loads it
  via WebGPU into the browser's IndexedDB cache, and chats over the vault
  using the MCP tools advertised by the satchel server. Reasoning blocks
  emitted as `<think>...</think>` are rendered in a collapsible panel; tool
  calls render inline as teal-bordered cards with the request, the result,
  and a status footer.
- Brand assets at `assets/brand/`: dark + light SVG and PNG variants of the
  horizontal lockup (mark + SATCHEL wordmark + tagline) and the standalone
  punk/displaced mark. README's hero now uses a `<picture>` element that
  picks the right variant from `prefers-color-scheme`.

### Web UI redesign

- New design system applied across the web UI: dark + light themes via CSS
  custom properties (toggled with the topbar button, persisted in
  `localStorage`, falls back to `prefers-color-scheme` on first load).
  JetBrains Mono throughout. Punk/displaced notched-pin mark in the topbar
  and Ask view, with an `feTurbulence` + `feDisplacementMap` filter so the
  geometry stays hand-built.
- New **Ask** tab: conversational entry point that wraps `search_knowledge`
  in a chat-style transcript with tool-call cards, source attribution, and
  truncated previews. No external LLM — pure retrieval.
- All sections (Dashboard, Search, Documents, Ingest, Manage, Connect)
  rebuilt around the design-system components: stat cards, tool cards,
  pill statuses, message blocks, dashed section labels.
- Active-job badge on the Ingest nav item.

### Embedding model upgrade

- Default embedding model is now [BAAI/bge-small-en-v1.5](https://huggingface.co/BAAI/bge-small-en-v1.5)
  (33M params, 384-d, MTEB ~62 vs MiniLM-L6-v2's ~57). Same architecture and
  dimension as the legacy model, so the SQLite schema is unchanged.
- The loader still recognizes `all-MiniLM-L6-v2` and prefers whichever
  single model is on disk. If both are present, BGE is preferred and a
  warning is logged so users notice if their existing index was built under
  the older model and would benefit from re-ingest.
- Pooling strategy is selected per model: BGE/Snowflake/E5 use `[CLS]`
  pooling; MiniLM-family models keep mean pooling.
- `scripts/download-model.sh` now fetches BGE by default. Pass `legacy` to
  fetch MiniLM instead.

## v0.2.2 — 2026-04-30

Audit pass on v0.2.1.

### Reliability

- **Ingest jobs now survive panics.** The background task wraps the pipeline
  in `catch_unwind`. Previously, a panic inside `ingest_path` (mutex
  poisoning, allocation failure, malformed data we didn't anticipate) left
  the job stuck on `running` forever and held a slot in the bounded retention
  list. Now the job is marked `failed` with the panic message in `error`.
- **`POST /api/ingest` fails fast when the embedding model is unavailable.**
  Previously the job would queue, run for a long time, and end with "all N
  records failed". Now the request returns a clear error before queueing.

### `/api/types` endpoint

```
GET /api/types  →  {types: [{file_type, source_count}, ...]}
```

The Manage tab's "delete by file type" dropdown and the Documents tab's
type filter are now populated dynamically from this endpoint instead of a
hardcoded HTML list. New formats added to the codebase show up
automatically; an empty vault doesn't show options that aren't there. The
list is cached client-side and invalidated after each ingest/delete.

### `/api/conversation` pagination

The endpoint now accepts `?offset=` and returns `{records, total, offset, limit}`
so a Slack `#general` with thousands of messages in a single day is
paginatable instead of getting clipped at the 2000 cap silently. Default
limit unchanged (2000), max raised to 10000.

### Score display

Search result cards now show the RRF score multiplied by 100, formatted
to two decimals (e.g. `2.74` instead of `0.027`). Title attribute
explains "Reciprocal Rank Fusion score (×100). Higher is better." Same
ranking, less mistakable for noise.

### Tests

132 unit + 23 integration tests pass. New: `test_list_records_by_source_respects_limit_and_offset`,
`test_list_file_types_groups_by_source_path`.

### Known issues flagged but not fixed in this release

- O(n²) thread-reply scan in the Slack handler. Negligible for typical day
  files (<200 messages); would matter for >5000 msgs/day. Optimization for
  v0.2.x or v0.3.
- `/api/jobs` has no `status=` filter. The UI fetches all 100 retained jobs
  even when only polling for active ones. Minor.

---

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
