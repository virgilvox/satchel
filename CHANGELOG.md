# Changelog

## v2.9.1 (2026-05-28)

### v2.9.0 audit fix pack

The post-ship audit caught three real gaps:

1. **Settings UI gap.** The SMART AGENT MODE block lived only in the
   `cloud` tab. WebLLM users land on `local` and could not see the
   toggle. v2.9.1 lifts the block into a `{#snippet smartMode()}`
   at component-template scope and renders it inside both tab
   bodies via `{@render smartMode()}`. The exact users the feature
   targets can now reach the toggle.

2. **Anthropic loop parity.** `runLoop` (WebLLM) got stall + context-
   full detection in v2.9.0 but `runAnthropicLoop` did not.
   Anthropic's bigger context makes stall less catastrophic, but
   loop-stuck Sonnet/Opus burns API dollars and produces a worse
   answer too. v2.9.1 wires the same hashing + `detectStallPattern`
   in the Anthropic loop, fingerprinting the whole tool_uses set per
   turn (Anthropic can emit multiple in one turn).

3. **`auto_compact` was a stub.** The toggle was stored but inert in
   v2.9.0; the CHANGELOG was honest about it but the UI hid the
   row. v2.9.1 ships the real implementation in both
   `buildMessagesForLLM` and `buildAnthropicMessages`:
   - Counts completed tool-pair messages in the transcript.
   - When `smart_mode + auto_compact + context >= 65%` are all true
     AND there are more than two pairs, the oldest are emitted as
     `[compacted earlier search_knowledge call: <summary> (orig
     1240 chars)]` instead of the full tool body.
   - Summary prefers the NEXT round's reasoning (which usually
     describes what the model learned), falling back to this
     round's pre-call thought, then a generic placeholder.
   - The most recent two tool calls and the system prompt are
     always preserved verbatim so fresh evidence is intact.
   - On Anthropic the trigger almost never fires (200K window) but
     parity matters and the heaviest sessions still benefit.

   The `auto_compact` Settings row is restored, alongside
   `smart_mode` and `tool_result_max_tokens`.

### README

Brief mention of Smart Mode in the Chat tab tour, including the
v2.9.1 auto-compact addition.

### Tests

26 Vitest TS tests cover the new helpers; the loop integration
itself is exercised manually. No regressions; 261 Rust + 26 TS = 287
total still passing.

## v2.9.0 (2026-05-28)

### Smart agent mode for local LLMs (and Anthropic too)

Browser WebLLM has small context, slow inference, and no prompt
cache. The default chat loop's verbose tool results + repeating tool
schemas + fixed `max_rounds=10` combined into a "ran out of context
before answering" failure mode that hit on almost every multi-step
question. v2.9.0 adds Smart agent mode (on by default), shipping
five fixes:

1. **Backend-aware tool-result truncation** (`truncateToolResult` in
   `agent.ts`). WebLLM caps each tool result at 800 tokens (slider in
   Settings; range 256-4000). The truncation marker tells the model
   how much was dropped and which follow-up call would fetch the
   rest, e.g. `[truncated 1240 more tokens of output. Use
   get_chunk_context with a chunk_id from the head of this result to
   fetch the rest.]`. Anthropic gets a 10x looser cap (default 8000)
   because its context is bigger; cost-conscious users on long
   sessions still benefit.

2. **Stall detection** (`hashToolCall` + `detectStallPattern`). Every
   emitted tool_use is fingerprinted; an identical hash twice in a
   row injects a synthetic tool_result that nudges the model to
   either vary its approach or call `respond_to_user`. This is the
   single biggest win for small local models that otherwise loop the
   same search until the window blows.

3. **Context-pressure early stopping**. When context use crosses 75%
   of the model's window, the loop forces a final-answer round
   instead of running another tool call. `max_rounds` becomes a
   fallback ceiling, not the primary stopper.

4. **Compact WebLLM system prompt** (`compactSystemPrompt`). ~500
   tokens vs the 3K-token Anthropic-shaped prompt. Concrete tool
   one-liners, two worked examples, no style guidance (small models
   do not follow "no AI cliches" anyway and the tokens are better
   spent on tool-use mechanics).

5. **Anthropic prompt caching extended to tools block**. The cache
   breakpoint now attaches to the LAST tool in `tools[]` instead of
   just the system prompt, so the cached prefix covers the full
   `tools + system` instead of system alone. Multi-turn chats reuse
   the entire schema block; cache_read_input_tokens climbs
   proportionally on long sessions.

### Settings exposure

Settings -> Cloud -> SMART AGENT MODE section:

- **smart_mode** toggle, default on.
- **tool_result_max_tokens** slider (256-4000, default 800).

`auto_compact` is in the store but the in-loop transcript-compaction
implementation slipped to v2.9.1; truncation + stall handle the
common cases already.

### Tests

Vitest infrastructure added to `web/`. 26 new TS unit tests in
`web/src/lib/agent.test.ts` covering: token approximation,
truncation byte-boundary safety, hash determinism + order
independence, stall detection edge cases (no history, duplicate,
context full, zero window), transcript-compaction structural
guarantees, nudge strings, compact-prompt size. Hook: `npm test`
from `web/`.

287 tests passing total (261 Rust + 26 TS).

### Migration notes for existing chats

- Default behavior changes for WebLLM users: shorter system prompt,
  tool results capped, loop exits earlier on stall. If you were
  relying on the verbose prompt or full-fat tool results, toggle
  off Smart mode in Settings.
- Anthropic users: behaviorally the same answers, just a smaller
  prompt-cache miss on each new tool definition load.

## v2.8.3 (2026-05-28)

### Claude Haiku no longer triggers 400s from extended-thinking knobs

Haiku 4.5 has no extended-thinking surface, so sending `thinking` or
`output_config.effort` in the request body returns
`invalid_request_error`. The chat UI was sending them on every model.

Fixed at two layers:

- **Client side:** `ChatModel` grows a `supportsExtendedThinking`
  flag; Opus 4.7 and Sonnet 4.6 are `true`, Haiku 4.5 is `false`.
  The Anthropic runner in `Chat.svelte` reads the flag and omits
  `thinking` and `effort` from the API call when the active model
  does not support them. The Settings modal dims those two controls
  and shows an amber note when Haiku is selected: "Claude Haiku 4.5
  does not support extended thinking. The effort and thinking knobs
  are stored but omitted from the API request for this model."
- **Server side defense in depth:** `strip_unsupported_params` in
  `src/anthropic/mod.rs` now strips `thinking` and `output_config`
  for any model whose id starts with `claude-haiku`, mirroring the
  existing strip of `temperature`/`top_p`/`top_k` for Opus 4.7. Two
  new unit tests pin the behavior (and confirm Opus + Sonnet still
  pass those fields through).

261 tests passing.

## v2.8.2 (2026-05-28)

### Server now binds 0.0.0.0 by default so satchel.local reaches the LAN

Up through v2.8.1 the server bound `127.0.0.1`, which meant the
`satchel.local` hostname advertised by mDNS only worked from the
same machine (and only through routing accidents). Other devices on
the network would see the hostname resolve to the LAN IP and then
get connection-refused. v2.8.2 binds `0.0.0.0` by default so
satchel.local is genuinely reachable from a phone, tablet, or other
laptop on the same Wi-Fi.

New `satchel serve --bind <addr>` flag overrides the default. Pass
`--bind 127.0.0.1` for the prior loopback-only behavior; useful on
public networks where you do not want to broadcast.

Auto-open still uses `http://localhost:{port}` (NOT the LAN IP or
satchel.local), because the browser only treats `localhost` and
`127.0.0.1` as secure-context origins, and WebGPU requires a secure
context. Opening to satchel.local on the same machine would silently
disable the local-LLM Chat tab; opening to localhost keeps it
working while LAN access still works for everyone else.

### WebGPU detection error message names the secure-context cause

`checkSupport()` now distinguishes "browser does not have WebGPU"
from "this origin is not secure." When a user opens SATCHEL via
satchel.local or a LAN IP, `navigator.gpu` is undefined and the new
message reads "Insecure origin: satchel.local:7428. WebGPU is only
exposed on localhost / 127.0.0.1 or HTTPS. Open SATCHEL at
http://localhost:7428 on this machine to enable the local LLM." So
the failure mode is actionable instead of mysterious.

### Connect page reorganized: 3 tabs, tunnel up top

Connect was getting busy. v2.8.2 splits it into three top tabs:

  1. **PUBLIC TUNNEL** (default) is the Cloudflare panel for the
     "expose this vault on the internet" workflow. Promoted to the
     top per user request.
  2. **LOCAL ADDRESSES** has two clean blocks: "This machine"
     (localhost Web UI + MCP, the secure-context origin) and "Other
     devices on this network" (satchel.local Web UI + MCP, LAN IP
     Web UI + MCP, mDNS toggle). A secure-context warning surfaces
     automatically when the user is browsing via a non-localhost
     origin.
  3. **CLIENT SETUP** has the Claude Desktop / Code / Cursor / web
     snippets with the real binary path auto-filled.

Bug fix: every COPY button now copies exactly the URL shown next to
it. Previously some rows labeled "WEB UI" copied an MCP-suffixed
URL, which was confusing.

### Welcome modal on first launch

New users see a three-step popup the first time the web UI loads:
01 Ingest, 02 Connect, 03 Ask. Each step has a one-click action to
jump to the relevant tab. Dismiss persists in localStorage under
`satchel-welcome-dismissed=1`; clearing that key brings the modal
back next reload.

## v2.8.1 (2026-05-28)

### Larger documents via add_to_vault

Raise the `add_to_vault` MCP tool's content cap from 5 MB to 50 MB so
a real book, a thick PDF's extracted text, or a multi-day chat
export's text fields can land in the vault through one tool call.
The chunker, embedder, and SQLite storage handle the higher cap
without changes; embedding cost scales linearly so a full 50 MB
paste still ties up the tool call for a few minutes. The tool
description was updated to warn the model to flag long pastes
before committing.

### HTTP body limit lifted to 64 MB

Axum's default `Json<T>` body limit (2 MB) was the hidden ceiling
for both `POST /mcp` and `POST /api/ingest`; a 4 MB MCP tool call
would have 413'd before reaching the handler, well below the
`add_to_vault` cap. The router now applies
`DefaultBodyLimit::max(64 MB)` globally. New
`test_post_api_search_accepts_large_body` regression guard.

## v2.8.0 (2026-05-28)

### MCP write surface: add_to_vault, create_collection, assign_to_collection

The MCP tool list grows from 7 read-only tools to 10. Agents can now
save text into the vault, pre-create collections, and organize
existing documents. The new tools:

- `add_to_vault` accepts a `content` string (plain text, markdown,
  JSON, HTML, or code; capped at 5 MB), optional `title`, `source`,
  `file_type`, `tags`, `collection_name`, and `dry_run`. Goes through
  the same chunk + embed + index pipeline as file ingest, so
  MCP-added notes behave identically in `search_knowledge`,
  `get_chunk_context`, `list_sources`, and the Documents UI. The
  source defaults to `mcp://note/<short-uuid>`; bare names get an
  `mcp://` prefix auto-applied so MCP-added documents stand out from
  real filesystem paths in `list_sources`.
- `create_collection` is the explicit "set this up first" companion
  for a batch of adds. Idempotent on case-insensitive name match.
- `assign_to_collection` adds existing documents (by `document_id`)
  to a named collection. Cap of 200 ids per call. Unknown ids are
  silently dropped and reported in the response so an agent can pass
  a best-effort list without having to verify each id first.

The default Anthropic system prompt was updated to teach the model
when to invoke these write tools (only on explicit user intent like
"save this") and when to use `dry_run: true`.

### Hardening

- Embedder availability is checked BEFORE collection resolution so
  a no-model dev launch never leaves an orphan empty collection.
  Mirrors the same fix on CLI and REST `/api/ingest`.
- 5 MB cap on `add_to_vault` content; bigger payloads should land on
  disk and use `satchel ingest <path>`.
- 32-tag cap per `add_to_vault` call; 200-id cap per
  `assign_to_collection` call.
- File type restricted to text formats via inputSchema enum
  (md, markdown, txt, json, html, note, code); pdf/docx/etc. cannot
  sneak in this way.
- SHA-256 dedup on `(source_path, content)` means a re-issued
  `add_to_vault` returns the existing `document_id` instead of
  duplicating the row. The dedup-skip path still honors `tags` and
  `collection_name` so re-issuing into a new collection actually
  populates the new collection (same semantic as file ingest's
  dedup-skip path).
- `dry_run: true` validates and reports the would-be result without
  any DB writes.
- Every successful write emits a `tracing::info!` line with
  `doc_id`, `source`, and byte size so users can grep server stderr
  to audit what their agent has been saving.

### Database

- `Database::collection_add_documents` now returns the number of rows
  *actually inserted* (excludes already-members), not the size of
  the input. The web UI's "added" count in the bulk-move modal now
  reports honest numbers.
- New `Database::filter_existing_document_ids` helper. Used by
  `assign_to_collection` to pre-filter unknown ids so the FK-bound
  `collection_add_documents` does not abort the whole batch on the
  first bad id.

### Tests

258 passing, up from 239. New coverage:

- `add_to_vault`: basic, dedup, dedup-still-joins-new-collection,
  collection auto-create, tag persistence, dry_run, empty rejection,
  oversized rejection, bad file_type, too-many-tags, source
  canonicalization, end-to-end search-then-assign round-trip.
- `create_collection`: idempotent (case-insensitive), empty rejection.
- `assign_to_collection`: known ids, silent drop on unknown ids,
  empty list rejection, too-many-ids rejection, idempotent on repeat.

## v2.7.1 (2026-05-27)

### npm packaging fix for macOS

`npm install satchel-rag@2.7.0` ran cleanly on Linux and Windows but the
macOS binary was immediately killed with SIGKILL on first invocation.
The release script extracts the inner `Satchel.app/Contents/MacOS/satchel`
out of the signed `.app` bundle and ships it bare; macOS still sees the
embedded Developer ID signature but the bundle's resource manifest is
gone, so code-signature verification rejects the binary and the kernel
kills it before it can print anything.

v2.7.1 adds a tiny `postinstall.js` to the darwin platform packages
(`satchel-rag-darwin-arm64`, `satchel-rag-darwin-x64`) that runs
`codesign -s - --force` on install. The binary is re-stamped with an
ad-hoc signature, which is exactly what `cargo install` produces
locally and what macOS accepts for non-Gatekeeper invocation paths.
No-op on Linux/Windows.

The 2.7.0 npm packages have been deprecated on the registry pointing
to 2.7.1.

No Rust changes; the Cargo.toml bump to 2.7.1 keeps the
binary's reported `--version` aligned across all distribution
channels (cargo, npm, GitHub releases).

## v2.7.0 (2026-05-27)

### Collections become the app's primary scope

The active collection is now a vault-wide context driven by a single
chip in the top bar. Picking a collection there propagates to Search,
Ask, Chat, Documents, and the Ingest tab's default destination, so
"work inside the Work collection for the next hour" is one click
instead of three separate per-page pickers. ALL still means whole
vault. Per-page mini-pickers were removed; each scoped view shows a
small "scoped to X; change in the top bar" readout when a scope is
active, so the chip is always discoverable. Collection management
(create, delete, bulk-move) stays on the Documents tab; the chip's
dropdown links to it.

A one-shot migration lifts the v2.5.0 through v2.6.x chat-scope
setting into the new global key on first load, so the user's
previously chosen chat scope is preserved on upgrade.

### Ingest into a collection, in one step

The Ingest tab grows a destination block above the path input with
three options: follow the top-bar scope (default), ingest with no
collection at all, or pin this job to a specific collection. There is
also a free-text "create a new collection for this job" field that
auto-creates the named collection server-side if it does not yet
exist. The status line shows the resolved destination on queue
("JOB QUEUED · INTO WORK").

CLI gets the same: `satchel ingest -c <name> <path>` ingests with
collection assignment and auto-creates the named collection if
missing. Re-ingesting an already-known document into a new collection
now joins the existing document to the requested collection rather
than silently dropping the assignment under hash dedup.

Server side: `POST /api/ingest` accepts `collection_id` or
`collection_name`; id wins when both are present. The embedder
availability check moved ahead of collection resolution so a failed
ingest from a no-model dev build does not leave an orphan empty
collection. `IngestConfig.collection_id` threads through every code
path including the seven archive handlers (Slack, ChatGPT, Claude.ai,
Discord, WhatsApp, mbox, CSV/TSV).

### satchel.local on the LAN, via mDNS

The HTTP server now advertises itself over Multicast DNS so any
device on the same network can reach it at
`http://satchel.local:7428` without looking up an IP. Pure-Rust via
the `mdns-sd` crate, no native deps, on by default. Persisted toggle
at `<vault>/mdns.toml`; flip it from the Connect tab to stop
broadcasting.

macOS resolves `satchel.local` natively via mDNSResponder; Windows 10
and newer resolve through the built-in DNS client; Linux desktops
that ship `nss-mdns` or `avahi-daemon` resolve out of the box.
Smoke-tested on macOS: `dscacheutil -q host -a name satchel.local`
returns the host's IPv6 link-local addresses and the LAN IP.

### Connect page reframe

Top of the page now leads with three live addresses in priority
order: loopback MCP URL, satchel.local MCP URL (when mDNS is on), and
LAN IP MCP URL. Each has a one-click COPY button. The per-client
setup snippets (Claude Desktop, Claude Code, Cursor, browser) now
have the real binary path auto-filled from a new
`GET /api/connect-info` endpoint instead of asking users to manually
replace `/path/to/satchel`. The tunnel panel moves to a "remote
access" section at the bottom. Plain prose throughout: no emdashes,
no emoji icons.

### Test growth

238 tests passing (up from 216). New coverage:

- `test_ingest_assigns_to_collection_when_configured`
- `test_ingest_dedup_skip_still_joins_new_collection`
- `test_ingest_archive_csv_assigns_to_collection`
- `test_document_id_by_hash_lookup`
- `test_get_api_connect_info`
- `test_get_api_mdns_returns_state`
- `test_post_api_mdns_toggle_off_persists`
- `test_post_api_ingest_creates_collection_by_name`
- `test_post_api_ingest_rejects_missing_path_without_creating_collection`
- Four mDNS-config unit tests

### Dependencies

`mdns-sd = "0.20"` added with default features off (`logging` only;
the `async` flag dropped to keep the binary lean). Pure Rust, no C
deps, keeps the static-musl Linux story intact.

## v2.6.1 — 2026-05-04

### Determinate ingest progress for directory walks

Directory ingest now does a one-shot pre-walk to count every supported
file before processing starts, then reports the total back to the UI as
`files_total`. The Ingest view uses it to render a real, filling
progress bar with `seen / total` counts and a percentage, instead of
the indeterminate amber pulse.

The pre-walk is cheap (filesystem metadata + extension match) and stays
in memory only as a `Vec<PathBuf>` of supported files, so cost scales
with what would already be processed, not the directory size. Archive
ingest and single-file ingest still fall back to the indeterminate bar
since their record totals aren't knowable without parse work.

The per-job meta line also picks up a live `rate: 12.4/s` chip showing
records-added-per-second, derived client-side from `records_added` and
elapsed wall-clock — useful for eyeballing whether a long-running job is
making meaningful progress and for guessing ETA.

New `ProgressEvent::FilesPlanned(usize)` variant on the ingest progress
bus and `Job.files_total: Option<usize>` on the job registry. No schema
migration; jobs are in-memory only.

## v2.6.0 — 2026-05-04

### Anthropic API key validation button

The Settings → Cloud tab adds a TEST button next to SAVE KEY. Click
it to issue a minimal 5-output-token request against
`claude-haiku-4-5` through the same `/api/anthropic` proxy the chat
uses. A green check confirms the key + proxy round-trip works; a red
chip shows Anthropic's actual error message inline (`401 invalid x-api-key`,
`404 not_found_error`, etc.) so typos and revoked keys surface
immediately on save instead of on first chat use.

New endpoint `POST /api/anthropic/test`. Stateless, single-shot, no
streaming, doesn't pollute the chat session.

### Cost meter pill in chat header

Anthropic chat sessions now track cumulative dollar cost across turns
and surface it as a pill in the chat strip next to the cache-hit pill.
Calculation matches Anthropic's published rates:

- regular input tokens × `inputPerMillion`
- cache-write tokens × `inputPerMillion` × 1.25 (5-min TTL)
- cache-read tokens × `inputPerMillion` × 0.1
- output tokens × `outputPerMillion`

Pricing per model (as of 2026-05-04):

| Model | Input $/M | Output $/M |
|---|---|---|
| Opus 4.7 | $5 | $25 |
| Sonnet 4.6 | $3 | $15 |
| Haiku 4.5 | $1 | $5 |

Pill formats `$0.0042` for sub-cent costs and `$1.42` once you cross
a cent. Resets on `Clear Chat`. WebLLM sessions don't show the pill
(local inference, no per-token cost).

Pricing data lives in `web/src/lib/chatModels.ts` next to the model
list, so adding a new model means filling in one struct.

### Tests + verification

- All 225 tests pass (no new tests; both features are UI + a one-shot
  endpoint).
- `cargo fmt --check`, `cargo clippy -D warnings`, release build all
  clean.
- Web bundle rebuilt.

## v2.5.1 — 2026-05-04

### Token-bounded sub-chunking for oversize archive records

Until now, every archive record (Slack thread, mbox email, ChatGPT
conversation, CSV row, Discord export message, ...) was inserted as
exactly one document with one chunk. Records longer than the embedder's
context window (`bge-small-en-v1.5` = 512 tokens) had their tail
silently truncated by the tokenizer when the embedding was computed.
A 50-message Slack thread, a long email, or a dense AI conversation
ended up with most of its content unsearchable.

v2.5.1 adds `chunk_archive_body` to `archives::persist_record`. When a
record's body exceeds 480 tokens (leaving headroom for header
repetition), it is split into multiple chunks while:

- Preserving the **header line** (the normalized `csv: file.csv row 7` /
  `slack: @alice in #design 2026-04-12` prefix the handlers already
  produce) at the top of every chunk so each chunk is self-describing
  for both dense embeddings and BM25.
- **Splitting at natural boundaries**: paragraph (`\n\n`) → line (`\n`)
  → hard char wrap with UTF-8 boundary safety, in that order.
- Keeping the **`documents` row identical**: full body still stored
  there. Only the `chunks` rows multiply. So `get_document` returns
  the whole record unchanged; `search_knowledge` returns the most
  relevant slice; `get_chunk_context` (v2.4.0) walks neighbors of a
  hit within the same record.

Records under the threshold are unaffected (single chunk, exact
behavior preserved). Existing pre-v2.5.1 archive records in your DB
are still single oversize chunks; re-ingest if you want the new
treatment for already-stored data.

### Tests + verification

- 6 new unit tests for `chunk_archive_body`: under threshold returns
  single chunk; empty body handled; oversize paragraph-split with
  header repeated; no-newline body falls back to char-wrap; header
  token budget respected; chat-style messages split at message
  boundaries.
- 225 total tests pass (was 219).
- `cargo fmt --check`, `cargo clippy -D warnings`, release build all
  clean.

## v2.5.0 — 2026-05-04

### Chat-side collection scope picker

The Chat tab gains a sticky scope chip row above the message input,
the same `CollectionScope` UI the Search and Ask tabs already use. Pick
"ALL" (whole vault) or any named collection. The selection persists to
localStorage, survives reloads, and **auto-injects `collection_name`
into every `search_knowledge` call the LLM makes** unless the LLM
explicitly passes its own `collection_name` / `collection_id` (then
the LLM's choice wins; this lets a model still cross-collection search
when the user asks).

Why: smaller models routinely forget to pass `collection_name` even
when the user has clearly framed the question as scoped. Up to
v2.4.x the user had to either trust the model to remember or repeat
the collection name in every prompt. Now scope is a UI affordance the
user sets once and forgets.

### `filter_file_type` on `search_knowledge` + REST search

The `search_knowledge` MCP tool gains a `filter_file_type` argument:
restrict results to documents of a specific type (`md`, `pdf`, `csv`,
`slack`, `mbox`, `tsv`, `json`, etc.). The same field is on
`POST /api/search`. `SearchResult` now carries `file_type` per
result so callers can render type badges without a second query.

### Internal: `SearchOptions` refactor

`Database::search` had a 7-positional-arg signature flagged with
`#[allow(clippy::too_many_arguments)]`. Replaced with a single
`SearchOptions<'_>` struct that defaults to "search the whole vault"
and exposes named fields: `filter_source`, `filter_tags`,
`filter_collection`, `filter_file_type`. All 16 call sites updated
across production code and tests; future filter knobs (date ranges,
reranker toggle, contextual-retrieval mode) slot in here without
breaking the call signature.

### Tests + verification

- 3 new unit tests covering `filter_file_type` (single-type
  restriction, unknown-type returns empty, `SearchResult.file_type`
  populated).
- 219 tests pass (was 216).
- `cargo fmt --check`, `cargo clippy -D warnings`, release build all
  clean. Web bundle rebuilt.

## v2.4.3 — 2026-05-04

### See through macOS App Translocation; fresh deploys now actually get fresh vaults

v2.4.1 fixed the breadcrumb-falls-through-to-stale-vault problem for
non-translocated launches. v2.4.2's notarization helped on subsequent
launches. Neither fixed the case the user actually hits most often:
**first launch of a freshly downloaded .app**. macOS quarantines any
browser download (Chrome's `com.apple.quarantine` xattr) and runs the
app from a randomized App Translocation sandbox the first time, even
when the .app is fully Developer-ID-signed and notarized. From inside
the sandbox `current_exe()` cannot see the user's filesystem, so the
sibling-vault probe fails and the breadcrumb fires, dragging in the
wrong vault.

v2.4.3 fixes this by resolving the sandbox path back to the real
install path via Apple's public API
`SecTranslocateCreateOriginalPathForURL` in `Security.framework`. On a
translocated launch SATCHEL now logs:

```
[satchel] App Translocation detected; resolved real .app path:
  /Users/you/Projects/satchel/q2/Satchel.app/Contents/MacOS/satchel
```

…and the rest of `default_vault_path` operates on that real path:
sibling vault detection works, `preferred_sibling_vault_target`
auto-creates a fresh vault next to the .app, and the breadcrumb is
correctly bypassed. Net effect: drop a v2.4.3 .app into a new folder
on a fresh USB stick and the first double-click creates a vault on
that stick, not at your last-known location.

If `SecTranslocate` fails for any reason (offline disk, exotic OS
patch level), the v2.4.1 breadcrumb fallback still kicks in as a
safety net so you don't get a hard error.

### Implementation

- `src/macos_translocation.rs` (new). Raw FFI to two CoreFoundation
  symbols (`CFURLCreateFromFileSystemRepresentation`,
  `CFURLCopyFileSystemPath`, `CFStringGetLength`,
  `CFStringGetMaximumSizeForEncoding`, `CFStringGetCString`,
  `CFRelease`) and one Security symbol
  (`SecTranslocateCreateOriginalPathForURL`). No new crate dep; we
  link the system frameworks directly. On non-macOS targets the
  resolver is a stub that returns `None`.
- `default_vault_path` in `src/main.rs` calls the resolver before any
  sibling probing. The rest of the resolution branches operate on the
  resolved real path; everything works transparently.

### Tests + verification

- 3 new unit tests in `macos_translocation.rs`: empty path returns
  None; nonexistent path returns None safely (no crash); the FFI
  links and runs against a real non-translocated path
  (`/usr/bin/cat`) without panicking.
- 216 tests pass total (was 213).
- Live smoke test against an actual translocated process:
  SecTranslocate resolved
  `/private/var/folders/.../AppTranslocation/<UUID>/d/Satchel.app/...`
  back to
  `/Users/you/Projects/satchel/q2/Satchel.app/...` exactly as
  expected.

## v2.4.2 — 2026-05-04

### CSV / TSV: row-aware ingest, not "whole file as one chunk"

A user reported that searching for a value inside an ingested CSV
returned the entire file as a single result. The cause: csv/tsv files
were going through the plain-text path (`extract_text` returned raw
bytes, then `chunk_text(512, 64)` ran over them). Small CSVs collapsed
to one chunk; larger ones lost the header on every chunk past the
first. Either way the embedding had no idea what the columns meant.

v2.4.2 adds a dedicated CSV/TSV archive handler that emits one
`Record` per data row. Each record's body starts with a normalized
header line (`csv: <filename> row N`) and then carries the column
name / value pairs of that row, one per line:

```
csv: contacts.csv row 7

name: Alice
email: alice@example.com
role: founder
```

That makes every chunk self-describing for both legs of hybrid
retrieval: dense embeddings see "name: Alice" / "role: founder" as
semantic anchors, and BM25 hits queries like "founder Alice" with
the field name and value as adjacent tokens.

Implementation:
- `csv = "1"` added to `Cargo.toml` (small, well-maintained, brings
  no new transitive deps beyond `serde` which is already in tree).
- `src/ingest/archives/csv.rs` (new) with `detect` (case-insensitive
  `.csv` / `.tsv`) and `ingest` (one record per row up to 10k rows;
  groups of 50 rows beyond that to keep embedding cost bounded).
- `archives::detect` and `archives::ingest` (`src/ingest/archives/mod.rs`)
  pick up the new `ArchiveKind::Csv` variant for both top-level
  ingest_path entries AND directory-walk hits.
- `src/ingest/mod.rs::ingest_file` now calls `archives::detect`
  before its plain-text path, so a directory of CSVs gets the same
  row-aware treatment as a direct `satchel ingest data.csv`.

Tests (12 new; 213 passing):
- 9 unit tests in `csv.rs`: basic parse, quoted fields with embedded
  commas, TSV tab delimiter, skips blank rows, empty file, header-only
  file, flexible row widths, extension detection, case-insensitive
  detection.
- 3 integration tests in `tests/ingest_pipeline_test.rs`: one record
  per row with header repeated and row-separation preserved, TSV uses
  tab delimiter, directory walk picks up CSVs through the file-level
  intercept.

If you have CSVs already ingested under v2.4.1 or earlier, they are
still in the database as the old single-blob chunks. To re-ingest
under the new handler:

```
satchel delete --type csv      # also --type tsv if applicable
satchel ingest <path>
```

Existing slack / mbox / discord / whatsapp / chatgpt / claude.ai
archives are unaffected; this change only touches the CSV/TSV path.

## v2.4.1 — 2026-05-04

### Fresh deployment at a new path now creates a sibling vault (not the breadcrumb)

v2.3.2 added a "last-known vault" breadcrumb to recover from macOS App
Translocation. The fallback was too aggressive: it also fired when the
.app was running from a new real path with no sibling vault yet (a
second USB stick, a fresh extract on another disk), pulling in the
old vault from a previous run instead of treating the new spot as a
fresh deployment.

v2.4.1 differentiates by translocation state:

1. Sibling vault next to the binary or .app -> use it.
2. Translocated (sandbox path with `AppTranslocation` in it) -> consult
   the breadcrumb. The sandbox cannot see a real sibling and the user
   genuinely wants their previous vault back.
3. Not translocated and no sibling -> create a new sibling vault at the
   deployment dir (next to the .app on macOS, next to the binary on
   Linux/Windows). System install paths (`/Applications`, `/usr`,
   `/opt`, `/System`, `/Library`, `~/.cargo/bin`, `~/.local/bin`,
   `C:\Program Files`, `C:\Windows`) are skipped here; those defer to
   the platform data dir as before so we do not auto-clutter shared
   locations with `vault/`.
4. Platform data dir is the final fallback.

Net effect: plug a fresh USB stick into a Mac with v2.4.1 on it and
double-clicking the .app creates a brand-new vault on the stick. The
old breadcrumb still kicks in for the only scenario it was designed
for (translocated launches that have no other path to the user's
data).

### System prompt scrub: avoid the "MCP" acronym, stronger retry guidance

A user using a smaller model (Sonnet at low effort) saw the chat
hallucinate "Multilingual Content Processor (MCP)" while reasoning
about which tool to call, and give up after a single empty
`search_knowledge {collection_id: 16}`. Two prompt fixes:

- Replaced "MCP tools" with the literal tool names and a one-line
  description per tool. The system prompt now lists `search_knowledge`,
  `get_chunk_context`, `list_collections`, `list_sources`,
  `get_document`, `list_tags`, `vault_stats` and tells the model these
  are the only tools that exist; do not invent acronyms or guess what
  they stand for. Smaller models will not riff on "MCP" anymore.
- Added explicit retry guidance: vary the query, drop the collection
  filter, call `list_collections` / `list_sources` to discover what is
  actually present before declaring "no information." Specifically
  warns against passing `collection_id` values that were not first
  seen in a `list_collections` result; prefer `collection_name`.

### Tests + verification

- 5 new unit tests cover the new vault-resolution branch:
  preferred sibling target for an .app on a non-system path, raw
  binary, /Applications skip, /usr/local/bin skip, and a broad pass
  over the system-install-path predicate.
- Total 201 passing (was 196).
- `cargo fmt --check`, `cargo clippy -D warnings`, `cargo build
  --release --features embed-model` all clean. Web bundle rebuilt.

### Deferred to v2.5.0

The chat tab still has no scope picker for collections; users rely on
Claude to remember and pass `collection_name` correctly. v2.5.0 will
add a sticky scope chip in the chat header that auto-injects the
selected collection into every search call (and lets the user toggle
between "whole vault" and a named collection).

## v2.4.0 — 2026-05-03

### `get_chunk_context` MCP tool — expand a hit by N neighboring chunks

Adds a new MCP tool, `get_chunk_context(chunk_id, before, after)`, that
returns the chunk identified by `chunk_id` plus up to `before` chunks
preceding and `after` chunks following it, all from the same document
and ordered by `chunk_index`. Window clamps cleanly at document
boundaries; unknown `chunk_id` returns an empty list (not an error).

Why: a lot of vault content is conversational (Slack threads, Discord,
WhatsApp, ChatGPT/Claude.ai exports, email reply chains). A single
matched chunk in this kind of data is often a fragment whose meaning
flips with one message of context. The Anthropic chat path's existing
SATCHEL system prompt already tells Claude to fetch surrounding
context for chat-shaped fragments; v2.4.0 gives it the right tool for
the job, scoped to one document so the LLM does not have to do a
second search to find neighbors.

`search_knowledge` results now include `chunk_id` per hit so an LLM
can call `get_chunk_context` directly on a result. Existing fields
(score, source, text, tags, chunk_index) are unchanged.

Implementation:
- `Database::get_chunk_context` (`src/rag/mod.rs`) — single SQL
  `BETWEEN` over `chunks` joined to `documents`. Saturating
  arithmetic on the bounds, with explicit `usize -> i64` clamping
  via u64 to avoid wrap-to-negative on values larger than `i64::MAX`.
- New `ChunkContext` struct returned to callers (`chunk_id`,
  `document_id`, `source_path`, `chunk_index`, `text`).
- `chunk_id: String` added to `SearchResult` and `FusedRow`.
- MCP tool definition + `handle_get_chunk_context` handler in
  `src/mcp/mod.rs`. Window sizes capped at 20 each direction (41
  chunks total, comfortably under any context-window concern).
- Search MCP output now includes `chunk_id: <id>` per result so the
  LLM can call `get_chunk_context` directly without a second lookup.
- SATCHEL system prompt updated to direct Claude to use the new tool
  for chat-shaped fragments.

Tests added (9 new; 196 total passing):
- DB: returns the requested neighborhood; clamps at start; clamps at
  end (including `usize::MAX` window); unknown id returns empty;
  query never crosses document boundaries.
- MCP: search output surfaces `chunk_id` (regression guard); tool
  call returns the expected neighborhood with center marker; unknown
  chunk_id returns helpful text not an error envelope; missing
  `chunk_id` arg returns a tool error.

Documentation: README MCP-tools table rewritten to list all seven
tools (was five, missing `list_collections` and the new
`get_chunk_context`) with accurate descriptions.

## v2.3.4 — 2026-05-02

### release.yml workflow validation fix

v2.3.3's release workflow had `secrets.X != ''` predicates inside step
`if:` conditions to gate the new code-signing path. GitHub Actions
disallows `secrets.*` in `if:` (parse-time rejection, no jobs
created), so the v2.3.3 tag pushed but no Release artifacts were ever
produced.

The gate is moved into the script itself: each macOS-signing step
unconditionally runs, then `set -e`-bails early when the relevant
secrets are absent. `env.SIGNING_IDENTITY != ''` (which IS allowed in
`if:`, populated via `$GITHUB_ENV` from the import step) handles the
notarize step's gate.

Functionally identical to the v2.3.3 intent, but the workflow now
parses. v2.3.4 is the first tag that actually publishes a notarized
build.

## v2.3.3 — 2026-05-02

### macOS releases are now Developer ID signed and notarized

The release workflow now imports an Apple Developer ID Application
certificate into a temporary keychain on the macOS runner, signs the
.app and the bundled cloudflared with `--options runtime --timestamp`
(hardened runtime, required for notarization), submits the zip to
Apple's notary service, waits for the ticket, and staples the result
back onto the bundle before re-zipping.

End-user impact: a fresh download is no longer quarantined into the
App Translocation sandbox. Double-click works on first launch with no
"can't be opened" dialog and no `xattr -dr` dance.

The signing path is gated on five secrets in the `prod` environment
(`APPLE_CERT_P12_BASE64`, `APPLE_CERT_PASSWORD`, `APPLE_ID`,
`APPLE_TEAM_ID`, `APPLE_APP_SPECIFIC_PASSWORD`). When these are not
present (e.g. on forks), the workflow falls back to the previous
ad-hoc signing path so unprivileged builds still produce a launchable
bundle. The temp keychain is torn down at the end of the macOS jobs
regardless of success or failure.

### CI fmt fix

`recall_vault_path()` in src/main.rs used a single-line
`if cond { Some(p) } else { None }` shape that the local rustfmt
toolchain accepts but stable rustfmt on `dtolnay/rust-toolchain@stable`
rewrites to a four-line block. CI failed on the diff; v2.3.2's release
workflow shipped fine, but the CI badge went red. Rewrote as
`Some(PathBuf::from(trimmed)).filter(|p| p.exists())`, which both
formatters accept identically.

## v2.3.2 — 2026-05-02

### Recover the right vault after macOS App Translocation

When a freshly downloaded `Satchel.app` is double-clicked while still
quarantined, macOS copies it to a randomized read-only sandbox
(`/private/var/folders/.../AppTranslocation/<UUID>/d/Satchel.app/`)
and runs from there. `current_exe()` then points into the sandbox, the
sibling-vault probe finds nothing, and SATCHEL silently falls through
to `~/Library/Application Support/satchel/` and creates an empty
default vault there. The user's real vault, sitting next to the
original `.app`, sees no traffic.

v2.3.2 fixes the recovery path:

1. The binary writes a breadcrumb (`<data-dir>/last-vault.txt`) every
   time it successfully resolves a sibling-of-the-binary vault. Future
   launches that cannot find a sibling consult this file before
   falling back to the data dir, so the second launch after a fresh
   download lands on the right vault even if the .app is still
   quarantined and translocated.
2. When `current_exe()` is detected to be inside an `AppTranslocation`
   path, the startup banner now prints a clear stderr warning naming
   the cause and the fix (`xattr -dr com.apple.quarantine
   /path/to/Satchel.app`).
3. Added two unit tests covering the translocation-path detector
   (positive path with a realistic sandbox path; negative paths for
   `/Applications/...` and `/usr/local/bin/satchel`).

The first launch after a fresh download still hits the empty
data-dir vault (the binary cannot recover the original `.app` path
from inside the sandbox; that is a macOS API gap, not something
SATCHEL can solve in code). Subsequent launches recover, and the
permanent fix remains: strip the quarantine xattr, or notarize.

## v2.3.1 — 2026-05-02

### Anthropic chat 400 fix + visible upstream errors

v2.3.0's Anthropic mode sent `cache_control: {type: "ephemeral", ttl:
"1h"}` on the system prompt. The 1-hour TTL is gated behind the
`extended-cache-ttl-2025-04-11` beta header on raw HTTP, and the proxy
was not sending it; Anthropic returned 400 on every chat turn. The
default 5-minute TTL (no beta header required) is plenty for a chat
session, so v2.3.1 drops the explicit `ttl` field. Caching still works.

The 400s were also rendering in the chat as
`anthropic proxy 400: [object Object]` because the response is
`{type: "error", error: {type, message}}` and the inline extractor was
treating the inner object as a string. Routes through the existing
`errMessage` helper now and shows the actual upstream message.

The proxy now also buffers + logs upstream non-success responses
instead of streaming them blind. Anthropic's error JSON shows up in
the satchel terminal as a `tracing::warn` with status + body, so the
next time the API rejects something, the cause is one log line away.

## v2.3.0 — 2026-05-02

### Settings modal: tabs, deep-linking, mode-aware controls

The chat settings modal now opens with four tabs (CLOUD · CLAUDE / LOCAL ·
WEBLLM / MCP / PERSISTENCE) instead of one long scroll. The tab is picked
based on the active backend the first time the modal opens, and the
"SET API KEY" button on the chat empty-state now deep-links straight to
the Cloud tab. The Anthropic API key, model controls, and system prompt
editor are all on the first tab now instead of buried at line 320 of the
old modal.

### Anthropic mode: prompt caching, adaptive thinking, effort, no temperature

Anthropic chat traffic now sends an Opus-4.7-correct payload:

- `cache_control: {type: "ephemeral", ttl: "1h"}` on the system prompt
  block, so the tools + system prefix is reused across turns. The chat
  surfaces a small `cache Nt` pill once cache reads start landing.
- `thinking: {type: "adaptive"}` by default. Claude decides when and
  how much to reason; can be disabled per-chat for latency-sensitive
  flows.
- `output_config.effort` (low / medium / high / xhigh / max). Default
  is high; xhigh is recommended for tool-heavy research.
- `temperature` is no longer sent. Sampling parameters are removed on
  Opus 4.7 and would 400; the proxy strips them server-side as well so
  curl callers cannot trip the same wire.
- `max_tokens` default raised from 1024 to 16000, configurable up to
  64000.

### SATCHEL system prompt for Anthropic chat

A user-editable default system prompt is now sent on every Anthropic
turn. It tells Claude how the vault works, when to use MCP tools, how
to cite sources honestly, and how to read conversational data from
Slack / Discord / WhatsApp / chat exports (fetch surrounding context
before drawing conclusions on a single matched line).

The prompt also enforces a house style: no emdashes, no AI-cliche
phrasing ("Great question", "I'd be happy to help", "It's important to
note", etc.), no throat-clearing preambles, no filler hedges, prose
over bullets unless the content is genuinely a list, and emoji used
sparingly. Editable in Settings → Cloud → System Prompt with a "reset
to default" button.

### Errors no longer render as "[object Object]"

`(e as Error).message` rendering across the chat has been replaced with
a shared `errMessage(unknown)` helper that handles plain strings,
Error instances, `{message}` / `{error}` / nested Anthropic API error
shapes, and falls back to `JSON.stringify` before ever returning
"[object Object]". Every catch block in Chat.svelte routes through it.

### Reasoning blocks no longer collapse mid-read

`<ReasoningBlock>` now seeds an internal `bind:open` state from its
prop and lets the user's expand/collapse persist across parent
re-renders. Previously, autoscroll on a streaming turn would re-render
the message bubble and reset `<details {open}>` back to closed, which
felt especially bad for the bottom-most reasoning block.

### Internal

`src/anthropic/mod.rs` now strips `temperature` / `top_p` / `top_k`
from any request whose model starts with `claude-opus-4-7` (added 3
unit tests). Server-side belt-and-suspenders for the client-side
strip, so curl / script callers cannot trip a 400.

## v2.2.2 — 2026-05-02

### Reverted LSMultipleInstancesProhibited; single-instance now Rust-side

v2.1.3 added `LSMultipleInstancesProhibited` to the macOS Info.plist to
fix error -47 (file busy) when re-launching while a previous SATCHEL
was still running. That fix was wrong: it hands the launch decision to
LaunchServices, which on Macs that have downloaded multiple SATCHEL
versions ends up with stale registrations for `com.satchel.app` at
paths that no longer exist. LaunchServices errors out with "file not
found" before Gatekeeper ever shows the standard "cannot be opened
because Apple cannot check it for malicious software / Open Anyway"
dialog, leaving users with no recovery path from the Finder side.

The plist key is gone in v2.2.2. Single-instance enforcement now lives
in `server::serve`: if `tokio::net::TcpListener::bind("127.0.0.1:7428")`
returns `AddrInUse`, the second copy prints a one-liner and opens the
browser to the running instance's UI, then exits cleanly. Same UX as
LaunchServices "activate existing instance" without the LaunchServices
side effects, and Gatekeeper sees a normal launch so the "Open Anyway"
path works again on quarantined fresh downloads.

If your Mac has accumulated stale registrations from earlier 2.x
releases, one-shot cleanup:

```
/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister -f /path/to/Satchel.app
```

re-registers the current bundle as the canonical `com.satchel.app`.

## v2.2.1 — 2026-05-02

### CI fmt re-flow

`cargo fmt --check` tripped on v2.2.0 main. Removing the closure from
the version-suffix split chain in `src/release.rs` shortened the chain
enough that rustfmt now prefers a single line over a five-line dotted
form. Pure whitespace, no behavior change; v2.2.0 binaries are
byte-equivalent. v2.2.1 ships so the published source matches the
release tag and CI is green going forward.

## v2.2.0 — 2026-05-02

### Relicensed under MIT

SATCHEL is now MIT-licensed. The previous AGPL-3.0 + commercial-licensing
exception is gone. `LICENSE`, `Cargo.toml`, `README.md`, `CONTRIBUTING.md`,
and the npm package metadata all carry the MIT terms now. Existing users
keep everything they had under AGPL plus the looser MIT permissions; no
contributor agreement was in place, the copyright remains with the author.

### macOS Satchel.app finds the vault next to the bundle

USB-stick deployment on macOS was effectively broken since the .app bundle
landed: `default_vault_path()` probed `<binary-dir>/vault/`, which inside a
.app bundle resolves to `Satchel.app/Contents/MacOS/vault/`, never the
`vault/` folder sitting next to `Satchel.app` on the drive. SATCHEL would
silently fall through to `~/Library/Application Support/satchel/` and
create an empty default vault there; users plugging in their stick saw an
empty Dashboard while their records sat unused on disk.

The resolver now also probes the directory that holds the .app bundle when
the binary lives at `<dir>/Satchel.app/Contents/MacOS/satchel`. Linux and
Windows behavior is unchanged. Four unit tests cover raw-binary, .app
sibling, no-vault-found, and inner-vault-precedence cases.

### Build and lint cleanup

`scripts/build-all.sh` now passes `--features embed-model` (matching CI)
and bails up front when `vault/models/bge-small-en-v1.5/*` are missing, so
local cross-builds cannot silently produce binaries that fall back to the
`Unavailable` embedder. Three clippy warnings cleared (`sort_by_key`,
`RangeInclusive::contains`, `[','`/`'+']` slice pattern) plus a handful
of unused-import / dead-code warnings in the integration-test scaffolding.
A stale `bin/lib/libonnxruntime.dylib` (left over from before the candle
migration) has been deleted from the working tree.

## v2.1.3 — 2026-05-02

### macOS error -47 on launch

Double-clicking `Satchel.app` while an older instance was still running
threw the Finder dialog "The application 'SATCHEL' can't be opened. -47"
(macOS `fBsyErr`, "file busy"): the previous process held the binary
inside `Contents/MacOS/`, so `LaunchServices` refused to launch a
second copy that would overwrite it.

Fixed in the bundle: `Info.plist` now sets

```xml
<key>LSMultipleInstancesProhibited</key>
<true/>
```

so a second click focuses the running SATCHEL instead of trying to spawn
a duplicate. SATCHEL binds port 7428 and a second copy would have failed
on `bind()` anyway — this just makes the right thing happen instead of
the confusing thing.

README's macOS section gained a troubleshooting note covering the older-
release recovery path (`pkill -f satchel`, `lsof -i :7428`, re-extract,
`xattr -dr com.apple.quarantine`) for users still on v2.1.2 or earlier.

## v2.1.2 — 2026-05-02

### Update checker

The Dashboard now probes GitHub's `releases/latest` endpoint and shows
a clickable banner when a newer version is available. Comparing
`CARGO_PKG_VERSION` against the tag (with leading `v` stripped and
pre-release / build suffixes ignored), the banner reads
"UPDATE AVAILABLE · v2.1.2 → v2.2.0 · view release notes ↗" and
links to the GitHub release page so the user can grab the new zip.

A small status chip on the active-vault strip says `UP TO DATE` /
`v2.2.0 READY` / `CHECK FOR UPDATES`. Clicking it forces a refresh
(bypasses the cache) so users can verify after a known release.

Server-side caching: one hour, process-wide. GitHub allows 60
unauthenticated requests/hour per IP — well above what one running
satchel can plausibly burn even with several open browser tabs. No
auth token needed.

Privacy: hitting `api.github.com` discloses to GitHub that someone
is running SATCHEL. Set `SATCHEL_DISABLE_UPDATE_CHECK=1` in the env
to opt out. The chip then reads `UPDATES OFF` and no network call
fires.

### `GET /api/release`

```json
{
  "current": "2.1.2",
  "latest": "2.2.0",
  "update_available": true,
  "release_url": "https://github.com/virgilvox/satchel/releases/tag/v2.2.0",
  "published_at": "2026-05-09T...",
  "checked_at": "2026-05-02T...",
  "error": null,
  "disabled": false
}
```

`?refresh=1` bypasses the cache. `error` carries the network /
parse / rate-limit reason on failure (in which case
`update_available` stays false — never raise the flag on uncertain
data).

153 lib tests passing (5 new in `release::tests` covering the version
comparator: basics, short versions, pre-release suffixes, garbage tags,
and owner/repo extraction). 25 integration tests, all green.

## v2.1.1 — 2026-05-02

### Surface "you may be looking at the wrong vault" in the UI

A user upgrading from v1.x reported the Dashboard count showing only
their most recent ingest (a single mbox archive) and not the much
larger pile of records they'd ingested previously. The records weren't
lost — SATCHEL was opening a different vault than the one their old
data lived in. v1.x stored vaults in `./vault`, `~/vault`, or the
platform data dir (`~/Library/Application Support/satchel`,
`%APPDATA%/satchel`, `$XDG_DATA_HOME/satchel`) depending on how it
was launched, and nothing in the web UI told you which one you'd
landed on.

Fixed at the data layer:

- `/api/status` now returns a `vault` block: `{ name, path, base_path,
  siblings: [...], legacy_bases: [...] }`.
- `siblings` enumerates every vault under the active base — the user's
  other named vaults that aren't currently selected.
- `legacy_bases` checks `./vault` and `~/vault` for SATCHEL-shaped
  layouts (a `satchel.toml` or `vaults/` subdir) that aren't the
  chosen base. If `~/vault` has 1.2 GB of `satchel.db` and the active
  vault has 8 KB, this is the smoking gun.

Surfaced in the Dashboard:

- A small `ACTIVE VAULT · <name> · <full-path>` strip sits at the top
  so you always know which vault produced the count below.
- An amber banner appears when other SATCHEL data is detected nearby,
  with the exact CLI to switch — `satchel vault use <name>` for
  siblings or `--vault <path>` for legacy bases — and the byte
  totals so the size mismatch is unmissable.

### `vault::list_vaults_info` + `vault::legacy_bases`

New library API for the same diagnostics. Returns
`Vec<VaultListEntry>` with `{name, path, size_bytes, size_human,
active}`. `get_active_vault` is now `pub` so the server can resolve
the active name without going through stdout.

148 lib + 25 integration tests, all green.

## v2.1.0 — 2026-05-02

### Documents tab — BY RECORD view

The Documents tab grouped by `source_path` so an archive ingest of, say,
50,000 Slack messages collapsed into ~365 rows (one per daily JSON
file) — and you couldn't see or pick the individual messages from the
UI. The records were *in* the vault; the table was just summarizing
above them.

A new BY RECORD / BY SOURCE toggle sits at the right of the filter
row. BY RECORD (the new default) shows one row per `documents` row —
the actual ingested unit (each Slack message, each PDF, each
conversation) — with title, source path, chunk count, ingested time,
and the collections each record currently belongs to. BY SOURCE keeps
the v1.6 path-grouped summary for archive-heavy vaults.

Multi-select and bulk *move to collection* / *remove from collection*
work in both modes. BY RECORD assigns at the document level via two
new endpoints:

- `POST   /api/collections/:id/documents { document_ids: [...] }` → `{added}`
- `DELETE /api/collections/:id/documents { document_ids: [...] }` → `{removed}`

BY SOURCE keeps using the existing source-paths endpoints. Collection
membership is honest about which axis you're picking on.

The path-substring filter in BY RECORD also matches against `title`
so you can find a specific Slack message or conversation by name.

### `GET /api/documents`

New REST endpoint backing BY RECORD. Same query shape as `/api/sources`
(`q`, `filter_type`, `sort_by`, `limit`, `offset`, `collection_id`)
but returns `{documents: DocumentRow[], total, ...}` where each
`DocumentRow` is `{id, source_path, title?, file_type, chunk_count,
ingested_at, collection_ids: [...]}`. The `collection_ids` field is
populated server-side so the UI doesn't need a second round-trip per
row.

### WebLLM context/sliding-window load fix

Picking both `context_window_size` and `sliding_window_size` in the
chat Settings modal made WebLLM refuse to load the engine —

> LOAD FAILED · Only one of context_window_size and sliding_window_size
> can be positive. Got: context_window_size: 8192, sliding_window_size:
> 8192. Consider modifying ModelRecord.overrides to set one of them to -1.

Three-part fix:

1. `createEngine` now sends one positive override and explicitly sets
   the opposite to `-1`, overriding any default the model card might
   carry. Mutually exclusive at the boundary so a stale config can't
   reproduce the collision.
2. The Settings modal makes the choice explicit: picking a context
   strategy resets the other to its sentinel (`auto` / `off`). A short
   description above the two pickers explains the trade-off.
3. The "context full" runtime hint now leads with `sliding_window_size`
   as the most reliable answer (keeps the most recent N tokens, drops
   older — works regardless of the model's compile-time max). Calling
   out the v2.1+ mutual-exclusivity behavior so the message stays
   accurate as the user opens Settings.

### Tests

148 lib tests passing (added `test_list_documents_ungrouped` covering
the per-record list, title-match filter, collection-id surfacing on
each row, and collection-scoped totals).

## v2.0.0 — 2026-05-01

A second stability declaration. v1.0 declared the retrieval surface
stable; v2.0 declares the *full* surface stable — chat (local + Claude),
public tunnels (quick + named), external MCP servers, and collections.
The entire v1.x feature wave is now considered the supported shape.

### What v2.0 actually means

No breaking REST or MCP changes. Existing v1.x vaults open without
migration; the v1.6 schema additions (`collections`,
`document_collections`, `PRAGMA foreign_keys = ON`) are idempotent and
purely additive. v1.x integrations keep working.

What changed:

### Search & Ask UI scoped to collections

The Documents tab gained a tab strip in v1.6.0; the rest of the UI
caught up here. Search and Ask both render a `CollectionScope` chip
row right under the input — `ALL · Work · Personal · Research` — and
re-run the active query when scope changes. Pulled into a shared
`<CollectionScope>` component so future routes get the same control
in a one-line import.

### Server-side guard on `/api/sources` DELETE

`/api/clear` already required `confirm: true` ∥ `dry_run: true`. The
sibling `/api/sources` DELETE didn't. A tunneled vault was therefore
one curl away from a wipe. Now `DELETE /api/sources` rejects without
either flag with the same error contract as `/api/clear`. The web UI
auto-supplies `confirm` on writes (it already gates writes through a
dry-run preview), so the change is invisible to UI users; external API
consumers must add `"confirm": true` to write calls.

### Lib clippy fully clean

Three pre-existing warnings cleared: `manual_clamp`,
unnecessary `as usize`, and an `#[allow(clippy::too_many_arguments)]`
on `db.search` (eight args after the v1.6.1 collection scope). Lib +
binaries pass `cargo clippy` with zero warnings; tests still carry a
handful of unused-import warnings that don't affect production.

### Tests + docs

- 147 lib + 25 integration tests, all green
- README refreshed for v2.0 (feature index across v1.x, security
  model, public-tunnel risk note)
- CLAUDE.md updated to reflect the v2.0 surface

## v1.6.2 — 2026-05-01

Audit-driven follow-up to v1.6.1.

### `list_collections` MCP tool

`search_knowledge`'s `collection_name` argument was unreachable for AI
agents in v1.6.1 — the agent had no way to discover what names exist.
Added a sixth MCP tool, `list_collections`, that returns each
collection's name, id, and document_count. Empty surface returns a
hint pointing at the Documents tab. Pair with `search_knowledge` to
let an agent scope a query to "the work notes" without coordination.

### Documents tab — ALL count is the vault count

The ALL tab's count was tied to the filtered `total`, so clicking
"Work" made the ALL tab read out the work count too. Now `vaultTotal`
is tracked separately and the ALL tab always reflects the unfiltered
size. One extra `/api/sources?limit=1` call when filtered, free
otherwise.

### Tests + docs

- Two new MCP integration tests (mcp_handler_test went 5 → 7):
  `test_mcp_search_with_collection_name` exercises the resolve-by-name
  path and the unknown-name error case;
  `test_mcp_list_collections` covers empty and populated surfaces.
- Tool-count assertions bumped (5 → 6) in both unit and integration.
- README's Collections section now lists the v1.6.1 search-by-collection
  REST + MCP surface (was still flagged as a "v1.6.x follow-up").
- `mockups/` added to `.gitignore` so design-reference HTML stops
  showing up in `git status`.

## v1.6.1 — 2026-05-01

### Search scoped to a collection (REST + MCP)

`db.search()`, `POST /api/search`, and the `search_knowledge` MCP tool
now accept a collection scope. Documents-tab filtering already worked
in v1.6.0; this lifts the same scope into the retrieval surface so AI
agents and external API consumers can ask "the work notes" instead of
"the totality."

- `POST /api/search   { ..., collection_id: number }`
- MCP `search_knowledge` arguments: `collection_name` (preferred — it
  is the user-visible label) or `collection_id` (numeric id). An
  unknown name returns a clear error so the agent can correct.
- `web/src/lib/api.ts` `api.search()` accepts an optional
  `collection_id` argument; existing call sites continue to scope to
  the entire vault.

Filter is applied post-fusion (same code path as `filter_source` /
`filter_tags`), so RRF still ranks across the full vault before
restricting to membership — quality unchanged, just narrower.

Test added: `test_search_filter_by_collection` (147 lib tests passing,
up from 146 in v1.6.0). It covers the scoped-results case, an empty
collection (returns zero, not an error), and the unfiltered baseline.

### Format fix

`cargo fmt --all` cleanup that should have ridden along with v1.6.0.
The Format CI step on main turned red on the v1.6.0 push; this brings
it green again.

## v1.6.0 — 2026-05-01

### Collections — named subsets of your vault

Sometimes you don't want to ask the totality. You want to ask "the
research papers" or "the work notes" or "the cooking PDFs." Collections
are named, opt-in groupings of documents that already live in the
vault — no re-ingest, no second copy, just a named view.

Schema (SQLite, additive — no migration required for existing vaults):

- `collections (id, name UNIQUE, created_at)` — the named set
- `document_collections (document_id, collection_id, PRIMARY KEY both)`
  — many-to-many membership with `ON DELETE CASCADE` on both sides, so
  deleting a document or a collection cleans up the join automatically

Connections set `PRAGMA foreign_keys = ON` so the cascades actually
fire (SQLite leaves them off by default).

REST surface (server-side; the browser drives all of this through
`web/src/lib/api.ts`):

- `GET    /api/collections`                    → `[{id, name, created_at, document_count}]`
- `POST   /api/collections           {name}`   → `{id, name}`
- `DELETE /api/collections/:id`                → `{ok: true}`
- `POST   /api/collections/:id/sources         {source_paths: [...]}`  → `{added}`
- `DELETE /api/collections/:id/sources         {source_paths: [...]}`  → `{removed}`
- `GET    /api/sources?collection_id=:id&...`  → existing browse, scoped to membership

The browse endpoint takes `collection_id` alongside the existing
`q` / `filter_type` / `sort_by` filters; they compose, so you can
search inside a collection or sort it however you like.

### Documents tab — multi-select and bulk move

The Documents page got rebuilt around collections:

- **Tab strip** at the top: ALL · Work · Personal · Research · …
  with the count next to each tab name, an inline "+ new collection"
  form, and a small × on each tab to delete that collection (the
  documents stay; only the membership is removed).
- **Multi-select rows**: every row gets a checkbox, the header has a
  toggle-all, and a bulk-action bar appears in the summary line once
  anything is selected — *Move to collection…* / *Remove from
  collection…* / *Clear selection*.
- **Move-to-collection modal** lets you pick the target collection
  and choose add or remove. The list refreshes when it closes.

### Tests

- `test_collections_full_lifecycle` — exercises create, assign, list,
  unassign, scope filtering, cascade-on-delete (146 tests passing).

### Coming in v1.6.x

- Filter `search_knowledge` (and the MCP surface) by collection so
  Claude / WebLLM agents can be pointed at "just my work notes" the
  same way the Documents tab is. The schema and REST already support
  it; the chat agent loop just doesn't pass `collection_id` through
  yet.

## v1.5.0 — 2026-05-01

### Chat: Anthropic Claude alongside the local WebLLM models

The chat picker now has two groups:

- **Anthropic API** — Claude Opus 4.7, Sonnet 4.6, Haiku 4.5. Streamed
  through a server-side proxy at `/api/anthropic/messages`. The API key
  lives at `<vault>/anthropic.toml` (chmod 0600 on Unix); satchel
  attaches the `x-api-key` + `anthropic-version` headers and pipes the
  SSE response straight to the browser. The browser never sees the key.
- **Local · WebLLM** — same curated list as before (Llama 3.2 1B/3B,
  Hermes 3, Qwen 2.5 3B, Phi 3.5, Gemma 2, DeepSeek R1, …).

The runLoop branches on backend. Claude turns use Anthropic's native
tool-use protocol (`tools` + `tool_use` blocks) instead of XGrammar
constrained decoding; tool calls dispatch to the same MCP, results come
back as `tool_result` content blocks.

UI cues for "you can configure Claude":
- Engine-bar shows a `SET API KEY` button instead of LOAD when an
  Anthropic model is selected with no key saved.
- Selected-model note carries an `Anthropic API` badge.
- Settings modal has a new **ANTHROPIC API** section: paste key, save
  / replace / clear, plus a `console.anthropic.com → Settings → Keys`
  link.

### External MCP servers — settings UI + server-side proxy

Beyond satchel's own built-in MCP at `/mcp`, users can now wire up any
JSON-RPC MCP server (GitHub MCP, filesystem MCP, anything that speaks
the protocol). Configuration UI lives in the same gear modal — a new
**MCP SERVERS** section with a list + ADD form.

Auth headers (Bearer tokens, X-API-Keys, etc.) are stored at
`<vault>/mcp.toml` (chmod 0600) and never leave the server side. Browser
traffic forwards through `/api/mcp/proxy/<id>` so the headers attach
server-side.

The built-in satchel MCP renders at the top of the list, locked, marked
"can't be removed" — exactly as you'd expect.

Endpoints:
- `GET /api/mcp/servers` → `[{id, name, url, has_auth}]` (no headers)
- `POST /api/mcp/servers` → upsert
- `DELETE /api/mcp/servers/:id` → remove
- `POST /api/mcp/proxy/:id` → forwards JSON-RPC body to the configured
  upstream with stored headers; pipes response (including SSE) back

(Aggregating tools across multiple connected MCPs in the chat agent
loop is a v1.5.x follow-up — the management surface ships now so the
UX bar lands; the cross-server tool aggregation comes next.)

### Library bits

- `src/anthropic/mod.rs` — `AnthropicConfig` (load/save/clear at
  `<vault>/anthropic.toml`), `proxy_messages()` opening a streaming
  POST against `https://api.anthropic.com/v1/messages` with the right
  headers. Uses `reqwest` (rustls-tls, stream, json) + `futures-util`
  for the byte-stream forwarder.
- `src/mcp_proxy/mod.rs` — `McpServersConfig` with `upsert` / `remove`
  / `find`, id validation (URL-safe slugs only), `proxy_call()`.
- `src/server.rs` — the four new endpoint families plus a generic
  `forward_streaming_response()` helper that copies status, the
  whitelist of pass-through headers (`content-type`, `cache-control`,
  `anthropic-request-id`, `mcp-session-id`), and streams the body
  through with `Body::from_stream`.
- 7 new tests covering both configs (roundtrip + blank-treated-as-unset
  + MCP id validation + remove + upsert-by-id-replaces).

145 cargo tests passing. Verified end-to-end: save fake Anthropic key →
status flips to `API key saved`; add MCP server → appears in list with
the built-in `satchel` entry above it; underlying files written with
0600 permissions.

## v1.4.0 — 2026-05-01

### Chat layout: engine bar on top, no rail

Chat tab was a fixed 280 px sidebar (model picker + MCP config + tools
list) plus the chat. Way too much chrome for what's mostly a chat
window. Restructured:

- **Status strip** above the chat (model · MCP · tools · round · ctx
  pills + ⚙ + CLEAR) — same role as before, unchanged.
- **Engine bar** below the strip: model picker + LOAD button when no
  engine is loaded, shrinks to a single-line "ready · UNLOAD" once a
  model is hot. Inline with the chat instead of in a sidebar.
- **MCP endpoint** + **tools list** moved into the Settings modal
  (gear button) — they're configured rarely, so they don't earn
  permanent screen real estate.
- The rail is gone entirely — no desktop sidebar, no mobile drawer,
  no toggle. The chat fills the whole width on every viewport.

### Compact tool cards

Tool calls used to render as fat teal boxes with full args + result
visible always. New `ToolCallCard` is collapsed by default — a
one-liner `→ search_knowledge {"query":"…"} OK`. Click anywhere on
the row to expand for full args + result. Pending calls auto-expand
(so the user can see what the model is asking for in flight) and
collapse once the result lands.

Same data, ~5× less vertical space in a multi-tool transcript.

### Smoke

Playwright at 1280×900 + 390×844: rail removed (locator count 0),
engine bar visible on both viewports, Settings modal grows the
expected sections (`GENERATION`, `AGENT`, `CONTEXT`, `MCP ENDPOINT`,
`TOOLS 5`, `PERSISTENCE`), no overflow, no console errors.

## v1.3.0 — 2026-05-01

### Tunnel: named-mode (stable URL on your own domain)

v1.2.0 only did anonymous quick tunnels — random `*.trycloudflare.com`
URLs that die with the satchel process. v1.3.0 adds **named tunnels**
for users who already have (or want to set up) a Cloudflare Zero Trust
tunnel pointed at their machine. The Connect-tab tunnel widget grew a
two-tab toggle:

- **Quick** (default, unchanged) — one-click anonymous tunnel.
- **Named** — paste the connector token + the public hostname you
  configured in the Cloudflare Zero Trust dashboard. Hit SAVE, then
  START NAMED TUNNEL. cloudflared runs as
  `cloudflared tunnel run --token <TOKEN>`, and the dashboard's
  ingress config decides what local port to forward to. The hostname
  becomes a stable URL across restarts.

### Token storage

Connector tokens are credentials — anyone with one can run a tunnel as
your account — so they live server-side, never in `localStorage`:

- Persisted to `<vault>/tunnel.toml` (same dir that holds `satchel.toml`).
- Unix permissions chmod'd to **0600** after write.
- The GET endpoint (`/api/tunnel/config`) only returns
  `{ configured, hostname }`, never the token. The token only leaves
  the file when satchel shells it into `cloudflared --token` at start
  time.

### Endpoints

- `GET /api/tunnel` — now includes `mode: "quick" | "named"` and
  `named: { configured, hostname }` so the UI can render its full
  state from one fetch.
- `POST /api/tunnel/start` — accepts `{ mode }` (default `"quick"`).
  Named requires a saved config; rejects with a clear error otherwise.
- `GET /api/tunnel/config` — `{ configured, hostname }`.
- `POST /api/tunnel/config` — `{ token, hostname }`.
- `DELETE /api/tunnel/config` — clears the file.

### Library bits

- `TunnelManager::start_named(token, hostname, port)` alongside the
  existing `start_quick(port)`.
- `TunnelConfig` struct with `load` / `save` / `clear` (TOML, 0600).
- Stderr reader factored to a single `spawn_stderr_reader` with a
  `UrlSource` enum: `ScrapeStderr` for quick tunnels (regex on the
  banner), `OnConnectRegistered(public_url)` for named (waits for
  `Registered tunnel connection` in stderr before flipping `state.url`
  to the user-supplied hostname so the "starting → live" UI arc
  stays meaningful).
- Three new tests: config-roundtrip, blank-token-treated-as-unset,
  start_named-rejects-empty-inputs.

Verified end-to-end: save fake config → start named → cloudflared
spawns with `--token` → stop reaps cleanly → DELETE config wipes the
file. 139 cargo tests passing.

## v1.2.0 — 2026-05-01

### One-click public tunnels via bundled cloudflared

A button in the Connect tab → a public `https://*.trycloudflare.com`
URL pointing at your vault. Anonymous quick tunnels, no Cloudflare
account needed. Works as a Custom Connector URL in claude.ai (web)
or as the `/mcp` HTTP transport endpoint in any client outside your
LAN.

There is no native Rust client for the Cloudflare Tunnel protocol
— the wire format is reverse-engineered Cloudflare-internal traffic
and the reference implementation is their Go daemon `cloudflared`.
We drive it as a child process. Two install paths:

- **Release downloads bundle the per-platform `cloudflared` binary
  next to satchel** (~35 MB extra in the zip): `Satchel.app/Contents/MacOS/cloudflared`,
  `cloudflared` next to the Linux binary, `cloudflared.exe` next to
  the Windows .exe. The CI workflow (`.github/workflows/release.yml`)
  fetches each from `https://github.com/cloudflare/cloudflared/releases/latest/download/`
  per matrix target. macOS releases come as `.tgz`; Linux/Windows are
  raw single-file downloads. The bundled cloudflared inherits the
  ad-hoc `codesign --deep` signature on macOS .app bundles.
- **Source builds (`cargo install`, etc.) use whatever `cloudflared`
  is on `$PATH`** — `brew install cloudflared` /
  `winget install Cloudflare.cloudflared` / `apt install cloudflared`
  / Cloudflare's deb/rpm. The UI shows install hints for each OS
  when `cloudflared --version` fails.

The tunnel manager (`src/tunnel/mod.rs`) probes
`<satchel-binary-dir>/cloudflared` first, falls back to `$PATH`,
spawns `cloudflared tunnel --url http://localhost:PORT --no-autoupdate
--protocol auto` with `kill_on_drop(true)` so the subprocess dies
with satchel, parses the trycloudflare.com URL out of stderr in a
background tokio task, and exposes start/stop/snapshot through a
thread-safe `Arc<Mutex<…>>`.

REST surface:
- `GET /api/tunnel` — current state + `cloudflared --version` probe
- `POST /api/tunnel/start` — spawn child, returns immediately
- `POST /api/tunnel/stop` — SIGTERM the child, await reap

UI: a panel at the top of the Connect tab with three states —
*not installed* (per-OS install hints), *idle* with a one-click
START button + a clear "the URL is public" warning, and *live*
with the public URL, the `<url>/mcp` derived endpoint, copy
buttons, and a STOP button.

Verified end-to-end on macOS: start → URL appears in ~5 s
(`https://breaking-meyer-attractive-qualify.trycloudflare.com`),
stop → child reaped cleanly, no zombie, no spurious "exited
unexpectedly" error after intentional stop.

### Includes everything in v1.1.1

(Ask 600-char truncate fix, Ingest copy, README hero screenshots —
see below.)

## v1.1.1 — 2026-05-01

### Ask: drop the 600-char hard truncate

`MessageBubble` was passing `truncate={600}` into `ResultRow` for the
Ask-tab retrieval rows, which clipped every passage at 600 characters
in the *data* (not just the view). The user noticed long mbox emails
came back chopped. `ResultRow` already has a `max-height: 220px` /
`overflow-y: auto` scroll-clip in CSS — same one Search uses — so
removing the hard truncate gets the full chunk text in the data with
visual scroll-clip in the row. Same fix landed as `b56717e` on `main`;
this release just tags it.

### Ingest: full file-type list visible on the page

The Ingest tab's description only listed archive types. Added a
single-paragraph instructions block that names every supported format:

- **Single files:** `md` · `txt` · `pdf` · `docx` · `html` · `csv` ·
  `tsv` · `json`
- **Format-aware archives** (auto-detected, parsed structurally — no
  raw JSON in your vault): Slack exports · ChatGPT exports · Claude.ai
  exports · Discord (DiscordChatExporter) · WhatsApp chat exports ·
  `.mbox` mail.

### README: hero screenshots + captions

Three full-width screenshots above the fold (Dashboard, Chat, Ingest)
showcase the headline features; the rest live in a 2-up grid below.
Every screenshot has an italic caption explaining what it shows and
why it's there. Chat and Ingest re-shot against the v1.1.x build so
the new strip (gear button, ctx pill) and the new Ingest description
are visible.

## v1.1.0 — 2026-05-01

### Chat: settings modal + context-fill recovery

A 4-turn agent transcript on Hermes 3 3B blew past the model's compiled
4096-token context window — WebLLM threw "Prompt tokens exceed context
window size" and the user was left staring at a raw error with no
recourse. Lifted the mockup's settings pattern into a proper modal and
fixed the underlying gap.

- **`web/src/components/SettingsModal.svelte`** — gear button (⚙) in the
  chat strip opens a modal with four sections: Generation, Agent,
  Context, Persistence. Each setting has a name, a live value readout,
  a control (range / select / checkbox), and a one-line description
  explaining what it does and when to change it. Lifted from the mockup's
  settings pattern at `mockups/satchel-chat (15).html`.
- **`context_window_size` and `sliding_window_size` are now wired all
  the way through.** `lib/webllm.ts:createEngine` accepts an
  `EngineChatOpts`; `Chat.svelte:loadModel` reads from `chatSettings`
  and forwards them to `CreateMLCEngine` as the third argument
  (snake_case, matching WebLLM's TS types). Picking 8192 / 16384 /
  32768 takes effect on the next UNLOAD + LOAD; the modal flags this
  with `· UNLOAD + RELOAD to apply` while the engine is hot.
- **Context-fill indicator** in the chat strip — once the engine
  reports `usage.total_tokens`, a `ctx 47% · 1923t` pill appears.
  Turns amber > 80%, red > 95%.
- **Friendly recovery from `Prompt tokens exceed context window size`.**
  The error catch detects this case specifically and writes a banner
  with the actionable fixes (open ⚙ Settings → Context, bump
  `context_window_size`; or enable `sliding_window_size`; or clear
  chat) instead of dumping the raw stack at the user.
- **Transcript persists to localStorage.** Refresh-survival, on by
  default; toggle in Persistence. Tool-call results larger than 8 KB
  are head+tail truncated so a long research session doesn't blow
  past the localStorage budget.
- **All hard-coded knobs are now settings-driven.** `MAX_ROUNDS`,
  `min_tool_calls`, `weak_score_threshold`, `temperature`, `max_tokens`
  all read from `chatSettings` instead of constants. Defaults match
  what was hard-coded so existing behavior is preserved.
- **Show-system-prompt toggle** under Persistence reveals the actual
  prompt being sent to the model in a collapsible panel below the
  composer — useful when debugging unexpected behavior.

Headless smoke (Playwright, desktop + 390 px mobile): modal opens on
both, all four sections render, the temperature slider writes to
localStorage on input, ctx + sliding-window selects expose the right
options, the gear is tappable on mobile, zero overflow, zero console
errors.

No backend changes — MCP wire format, REST API, SQLite schema, embedding
dimension, and CLI flags are all unchanged from `v1.0.0`.

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
