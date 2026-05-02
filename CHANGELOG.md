# Changelog

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
