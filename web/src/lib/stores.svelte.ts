// App-wide reactive state. Svelte 5 runes — these are exported state
// containers other modules can mutate or subscribe to.
//
// File extension matters: runes are only transformed by the compiler in
// `.svelte`, `.svelte.js`, and `.svelte.ts` files. A plain `.ts` file
// would leave `$state(...)` as a literal global reference and crash on
// first import.

import type { Mode, StatusResponse, Tab } from './types';
import type { CollectionSummary } from './api';

const STORAGE_MODE_KEY = 'satchel-mode';
const STORAGE_MCP_KEY = 'satchel-mcp-endpoint';
const STORAGE_MODEL_KEY = 'satchel-chat-model';
const STORAGE_SYSTEM_KEY = 'satchel-chat-system';
// Vault-wide active collection scope. `''` is "search the whole vault";
// a numeric id pins every scoped view (Ask, Search, Chat, Documents,
// Ingest) to a single collection. This is the user-facing "I am
// working inside Collection X right now" context.
const STORAGE_SCOPE_KEY = 'satchel-active-collection-id';

// Some browsers (Firefox strict private, Safari ITP, sandboxed iframes)
// throw on any localStorage access. Treat that as "no preference" rather
// than letting the whole module load fail.
function safeGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function safeSet(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* swallow */
  }
}

function initialMode(): Mode {
  const stored = safeGet(STORAGE_MODE_KEY) as Mode | null;
  if (stored === 'dark' || stored === 'light') return stored;
  try {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  } catch {
    return 'dark';
  }
}

class ThemeStore {
  mode = $state<Mode>(initialMode());

  constructor() {
    this.apply();
  }

  toggle() {
    this.mode = this.mode === 'dark' ? 'light' : 'dark';
    safeSet(STORAGE_MODE_KEY, this.mode);
    this.apply();
  }

  private apply() {
    document.documentElement.setAttribute('data-mode', this.mode);
  }
}

class StatusStore {
  data = $state<StatusResponse | null>(null);
  online = $state<boolean>(false);

  set(d: StatusResponse | null, online: boolean) {
    this.data = d;
    this.online = online;
  }
}

class RouterStore {
  tab = $state<Tab>('dashboard');
  set(t: Tab) {
    this.tab = t;
  }
}

/** Vault-wide active collection.
 *
 *  Every scoped view in the app reads `collection.activeId` to know
 *  what to filter by; setting it from one place (the TopBar scope chip)
 *  propagates to Search, Ask, Chat, Documents, and the Ingest tab's
 *  default destination.
 *
 *  `activeId === null` means "no scope, vault-wide." The list is
 *  refreshed from `/api/collections` on demand (after create/delete in
 *  Documents, or on first mount of the chip). */
class CollectionStore {
  activeId = $state<number | null>(readInitialScope());
  list = $state<CollectionSummary[]>([]);
  loaded = $state(false);

  get activeName(): string | null {
    const id = this.activeId;
    if (id == null) return null;
    return this.list.find((c) => c.id === id)?.name ?? null;
  }

  setActive(id: number | null) {
    this.activeId = id;
    safeSet(STORAGE_SCOPE_KEY, id == null ? '' : String(id));
  }

  /** Replace the in-memory list. If the previously active id is no
   *  longer present (collection deleted out from under us), fall back
   *  to ALL so the UI does not display a stale name. */
  setList(next: CollectionSummary[]) {
    this.list = next;
    this.loaded = true;
    if (this.activeId != null && !next.some((c) => c.id === this.activeId)) {
      this.setActive(null);
    }
  }
}

function readInitialScope(): number | null {
  // One-shot migration from the v2.5.0-v2.6.x chat-only scope keys.
  // If the new global key is empty but the chat-scope key is set, lift
  // that value into the global store so existing users do not lose
  // their Chat scope on upgrade. We never write back here; the next
  // `setActive(...)` call will overwrite the migrated value cleanly.
  const raw = safeGet(STORAGE_SCOPE_KEY);
  if (raw) {
    const n = Number(raw);
    return Number.isFinite(n) && n > 0 ? n : null;
  }
  const legacy = safeGet('satchel-chat-scope-collection-id');
  if (legacy) {
    const n = Number(legacy);
    if (Number.isFinite(n) && n > 0) return n;
  }
  return null;
}

// Chat-only knobs. Kept separate from `SettingsStore` because they
// belong to a single tab (Chat) and the settings modal lives there.
// All persisted to localStorage individually so flipping one doesn't
// re-serialize the rest.
export type ContextSize = 'auto' | 4096 | 8192 | 16384 | 32768;
export type SlidingSize = 'off' | 1024 | 2048 | 4096 | 8192;

const CHAT_KEYS = {
  temperature: 'satchel-chat-temperature',
  maxTokens: 'satchel-chat-max-tokens',
  maxRounds: 'satchel-chat-max-rounds',
  minToolCalls: 'satchel-chat-min-tool-calls',
  weakScoreThreshold: 'satchel-chat-weak-score',
  contextWindowSize: 'satchel-chat-ctx',
  slidingWindowSize: 'satchel-chat-sliding',
  persistHistory: 'satchel-chat-persist',
  showSystemPrompt: 'satchel-chat-show-sys',
  // Anthropic-mode (separate from the WebLLM knobs above; some of those
  // do not apply to API mode).
  anthropicEffort: 'satchel-chat-anthropic-effort',
  anthropicThinking: 'satchel-chat-anthropic-thinking',
  anthropicMaxTokens: 'satchel-chat-anthropic-max-tokens',
  anthropicCaching: 'satchel-chat-anthropic-caching',
  anthropicSystemPrompt: 'satchel-chat-anthropic-system',
};

export type AnthropicEffort = 'low' | 'medium' | 'high' | 'xhigh' | 'max';
export type AnthropicThinkingMode = 'adaptive' | 'disabled';

/** Default system prompt for Anthropic chat. Tells Claude how SATCHEL
 *  works (RAG vault + MCP tools), how to behave responsibly with the
 *  user's data, and the house style (no emdashes, no AI cliches, sparing
 *  emoji). User-editable in Settings.
 */
export const DEFAULT_ANTHROPIC_SYSTEM = `You are SATCHEL, an assistant embedded in the user's personal knowledge vault. You have a small set of tools (defined in your tool schema) for reading and (with permission) writing this vault. The read-side tools are:
- \`search_knowledge\` — hybrid retrieval (semantic + keyword) over the vault, returns ranked chunks with a stable \`chunk_id\`.
- \`get_chunk_context\` — given a \`chunk_id\` from a search hit, return the surrounding chunks of the same document. Use this to expand a fragment with its conversational frame.
- \`list_collections\`, \`list_sources\`, \`get_document\`, \`list_tags\`, \`vault_stats\` — for discovery and for reading a full document when retrieval points at it.

The write-side tools require explicit user intent and are NEVER to be called speculatively:
- \`add_to_vault\` — save a text snippet, document, or synthesis into the vault so it becomes searchable. Returns a stable \`document_id\`. Supports an optional \`collection_name\` (auto-created if missing) and \`tags\`. Identical content is deduplicated by SHA-256 so a re-save is a safe no-op. Use \`dry_run: true\` before committing anything large.
- \`create_collection\` — pre-create a named collection.
- \`assign_to_collection\` — add existing documents (by \`document_id\`) to a named collection.

These are the only tools that exist. Do not invent acronyms or guess what they stand for; the names above are literal. Treat \`search_knowledge\` as the primary entry point.

When to write to the vault
- Call \`add_to_vault\` only when the user explicitly asks you to save, remember, capture, log, or commit something. Phrases like "save this", "add to the vault", "remember this for later", "log this in [collection]" are the trigger. If you are unsure whether the user wants to save, ASK first; do not save by default.
- Pick a meaningful \`source\` value when one is obvious from context (e.g., \`meeting-2026-05-28\`, \`book-notes/dune\`, \`journal/2026-05\`) so future searches can scope by source. Bare names get an \`mcp://\` prefix automatically.
- Pass \`collection_name\` when the user has indicated where the item belongs ("save this to my Work collection"). The collection is auto-created if it does not yet exist.
- Use \`dry_run: true\` to confirm what would be saved for any paste over a few paragraphs.

How to use the vault
- Prefer evidence from the vault over your training memory. When a question is about the user's data, call \`search_knowledge\` first; do not answer from prior knowledge alone.
- Cite sources by document title and source path when you use them. Quote sparingly and accurately.
- If a search returns no results, do not give up after one try. Vary the query (synonyms, alternate phrasings, single keywords, the user's literal words). Drop a \`collection_name\` or \`collection_id\` filter and search the whole vault as a fallback. Use \`list_collections\` and \`list_sources\` to discover what is actually present before declaring "no information."
- Only after several genuinely different searches return nothing should you tell the user the vault does not contain the information.
- Do not pass \`collection_id\` values you have not first seen in a \`list_collections\` result. Prefer \`collection_name\` (the user-facing label) when you do scope a search.
- For multi-step or fact-heavy questions, plan tool calls before executing: name the tools you intend to call and why, then run them.
- Distinguish three things in your answers: what the vault says, what you reasoned, and what you are uncertain about.

Reading conversational data
- A lot of vault content is conversational: Slack messages, Discord chats, WhatsApp threads, ChatGPT and Claude.ai exports, email. A single matched chunk in this kind of data is usually a fragment of a longer thread.
- When a result looks like a chat message or thread fragment, fetch surrounding context before drawing conclusions. The \`get_chunk_context\` tool is the right way: pass the result's \`chunk_id\` and a small window (2 to 5 each side is usually enough) to read the messages immediately before and after, scoped to the same source document. \`get_document\` also works for the whole document at once when the fragment is part of a short note.
- Single-line sarcasm, replies, callbacks, and inside jokes routinely flip meaning when you read only the matched line. If a message is short, ambiguous, or refers to "this", "that", or "earlier", treat fetching context as required, not optional.
- When you cite a chat message, name the participants and approximate timeframe if visible. "Alice in #design on 2026-04-12" beats a bare quote.

House style
- No emdashes. Use commas, colons, semicolons, or parentheses to join clauses.
- No AI-cliche phrasing. Avoid: "Great question", "I'd be happy to help", "Certainly", "Absolutely", "It's important to note", "In summary", "I hope this helps", "Let me break this down", "First/Second/Finally" three-part scaffolding when it is not load-bearing.
- No throat-clearing. Skip preambles and restating the question. Answer directly.
- No filler hedges ("perhaps", "maybe", "it could be argued"). State what you know; flag what you do not.
- Plain prose over bullet points unless the content is genuinely a list.
- Emoji are fine, but sparing. One per response at most, and only when it adds signal.
- Match length to the question. A short question gets a short answer.

Operating principles
- Never claim certainty you do not have.
- Do not invent file paths, document titles, or quotes. If you did not see it, do not cite it.
- When a tool call fails or returns nothing useful, say so and try a different angle rather than guessing.
- The user's vault is private. Treat its contents with discretion in your phrasing.`;

const ANTHROPIC_EFFORTS: AnthropicEffort[] = ['low', 'medium', 'high', 'xhigh', 'max'];

function readNumber(key: string, fallback: number): number {
  const raw = safeGet(key);
  if (raw == null) return fallback;
  const n = Number(raw);
  return Number.isFinite(n) ? n : fallback;
}
function readBool(key: string, fallback: boolean): boolean {
  const raw = safeGet(key);
  return raw == null ? fallback : raw === '1';
}
function readCtx(key: string, fallback: ContextSize): ContextSize {
  const raw = safeGet(key);
  if (raw == null) return fallback;
  if (raw === 'auto') return 'auto';
  const n = Number(raw);
  if (n === 4096 || n === 8192 || n === 16384 || n === 32768) return n;
  return fallback;
}
function readSliding(key: string, fallback: SlidingSize): SlidingSize {
  const raw = safeGet(key);
  if (raw == null) return fallback;
  if (raw === 'off') return 'off';
  const n = Number(raw);
  if (n === 1024 || n === 2048 || n === 4096 || n === 8192) return n;
  return fallback;
}
function readEffort(key: string, fallback: AnthropicEffort): AnthropicEffort {
  const raw = safeGet(key);
  if (!raw) return fallback;
  return ANTHROPIC_EFFORTS.includes(raw as AnthropicEffort)
    ? (raw as AnthropicEffort)
    : fallback;
}
function readThinking(key: string, fallback: AnthropicThinkingMode): AnthropicThinkingMode {
  const raw = safeGet(key);
  if (raw === 'adaptive' || raw === 'disabled') return raw;
  return fallback;
}

class ChatSettingsStore {
  // Generation
  temperature = $state(readNumber(CHAT_KEYS.temperature, 0.6));
  maxTokens = $state(readNumber(CHAT_KEYS.maxTokens, 1024));
  maxRounds = $state(readNumber(CHAT_KEYS.maxRounds, 10));
  // Agent loop
  minToolCalls = $state(readNumber(CHAT_KEYS.minToolCalls, 1));
  weakScoreThreshold = $state(readNumber(CHAT_KEYS.weakScoreThreshold, 0.05));
  // Context (passed to WebLLM at engine creation; takes effect on next LOAD)
  contextWindowSize = $state<ContextSize>(readCtx(CHAT_KEYS.contextWindowSize, 'auto'));
  slidingWindowSize = $state<SlidingSize>(readSliding(CHAT_KEYS.slidingWindowSize, 'off'));
  // Persistence + debug
  persistHistory = $state(readBool(CHAT_KEYS.persistHistory, true));
  showSystemPrompt = $state(readBool(CHAT_KEYS.showSystemPrompt, false));
  // Anthropic-mode (only consulted when the active backend is 'anthropic')
  anthropicEffort = $state<AnthropicEffort>(readEffort(CHAT_KEYS.anthropicEffort, 'high'));
  anthropicThinking = $state<AnthropicThinkingMode>(readThinking(CHAT_KEYS.anthropicThinking, 'adaptive'));
  anthropicMaxTokens = $state(readNumber(CHAT_KEYS.anthropicMaxTokens, 16000));
  anthropicCaching = $state(readBool(CHAT_KEYS.anthropicCaching, true));
  anthropicSystemPrompt = $state(safeGet(CHAT_KEYS.anthropicSystemPrompt) ?? DEFAULT_ANTHROPIC_SYSTEM);

  setTemperature(v: number) { this.temperature = v; safeSet(CHAT_KEYS.temperature, String(v)); }
  setMaxTokens(v: number) { this.maxTokens = v; safeSet(CHAT_KEYS.maxTokens, String(v)); }
  setMaxRounds(v: number) { this.maxRounds = v; safeSet(CHAT_KEYS.maxRounds, String(v)); }
  setMinToolCalls(v: number) { this.minToolCalls = v; safeSet(CHAT_KEYS.minToolCalls, String(v)); }
  setWeakScoreThreshold(v: number) { this.weakScoreThreshold = v; safeSet(CHAT_KEYS.weakScoreThreshold, String(v)); }
  setContextWindowSize(v: ContextSize) { this.contextWindowSize = v; safeSet(CHAT_KEYS.contextWindowSize, String(v)); }
  setSlidingWindowSize(v: SlidingSize) { this.slidingWindowSize = v; safeSet(CHAT_KEYS.slidingWindowSize, String(v)); }
  setPersistHistory(v: boolean) { this.persistHistory = v; safeSet(CHAT_KEYS.persistHistory, v ? '1' : '0'); }
  setShowSystemPrompt(v: boolean) { this.showSystemPrompt = v; safeSet(CHAT_KEYS.showSystemPrompt, v ? '1' : '0'); }
  setAnthropicEffort(v: AnthropicEffort) { this.anthropicEffort = v; safeSet(CHAT_KEYS.anthropicEffort, v); }
  setAnthropicThinking(v: AnthropicThinkingMode) { this.anthropicThinking = v; safeSet(CHAT_KEYS.anthropicThinking, v); }
  setAnthropicMaxTokens(v: number) { this.anthropicMaxTokens = v; safeSet(CHAT_KEYS.anthropicMaxTokens, String(v)); }
  setAnthropicCaching(v: boolean) { this.anthropicCaching = v; safeSet(CHAT_KEYS.anthropicCaching, v ? '1' : '0'); }
  setAnthropicSystemPrompt(v: string) { this.anthropicSystemPrompt = v; safeSet(CHAT_KEYS.anthropicSystemPrompt, v); }
  resetAnthropicSystemPrompt() { this.setAnthropicSystemPrompt(DEFAULT_ANTHROPIC_SYSTEM); }
}

class SettingsStore {
  mcpEndpoint = $state<string>(
    safeGet(STORAGE_MCP_KEY) ?? window.location.origin + '/mcp'
  );
  chatModel = $state<string>(
    safeGet(STORAGE_MODEL_KEY) ?? 'Llama-3.2-1B-Instruct-q4f16_1-MLC'
  );
  systemPrompt = $state<string>(
    safeGet(STORAGE_SYSTEM_KEY) ??
      `You are SATCHEL, a knowledge assistant grounded in the user's local vault.
When you need facts, use the search_knowledge tool — never invent sources.
Cite sources by their path. Keep answers concise.`
  );

  setMcp(url: string) {
    this.mcpEndpoint = url;
    safeSet(STORAGE_MCP_KEY, url);
  }

  setModel(id: string) {
    this.chatModel = id;
    safeSet(STORAGE_MODEL_KEY, id);
  }

  setSystem(prompt: string) {
    this.systemPrompt = prompt;
    safeSet(STORAGE_SYSTEM_KEY, prompt);
  }
}

export const theme = new ThemeStore();
export const status = new StatusStore();
export const router = new RouterStore();
export const collection = new CollectionStore();
export const settings = new SettingsStore();
export const chatSettings = new ChatSettingsStore();
