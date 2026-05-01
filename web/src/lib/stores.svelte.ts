// App-wide reactive state. Svelte 5 runes — these are exported state
// containers other modules can mutate or subscribe to.
//
// File extension matters: runes are only transformed by the compiler in
// `.svelte`, `.svelte.js`, and `.svelte.ts` files. A plain `.ts` file
// would leave `$state(...)` as a literal global reference and crash on
// first import.

import type { Mode, StatusResponse, Tab } from './types';

const STORAGE_MODE_KEY = 'satchel-mode';
const STORAGE_MCP_KEY = 'satchel-mcp-endpoint';
const STORAGE_MODEL_KEY = 'satchel-chat-model';
const STORAGE_SYSTEM_KEY = 'satchel-chat-system';

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
};

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

  setTemperature(v: number) { this.temperature = v; safeSet(CHAT_KEYS.temperature, String(v)); }
  setMaxTokens(v: number) { this.maxTokens = v; safeSet(CHAT_KEYS.maxTokens, String(v)); }
  setMaxRounds(v: number) { this.maxRounds = v; safeSet(CHAT_KEYS.maxRounds, String(v)); }
  setMinToolCalls(v: number) { this.minToolCalls = v; safeSet(CHAT_KEYS.minToolCalls, String(v)); }
  setWeakScoreThreshold(v: number) { this.weakScoreThreshold = v; safeSet(CHAT_KEYS.weakScoreThreshold, String(v)); }
  setContextWindowSize(v: ContextSize) { this.contextWindowSize = v; safeSet(CHAT_KEYS.contextWindowSize, String(v)); }
  setSlidingWindowSize(v: SlidingSize) { this.slidingWindowSize = v; safeSet(CHAT_KEYS.slidingWindowSize, String(v)); }
  setPersistHistory(v: boolean) { this.persistHistory = v; safeSet(CHAT_KEYS.persistHistory, v ? '1' : '0'); }
  setShowSystemPrompt(v: boolean) { this.showSystemPrompt = v; safeSet(CHAT_KEYS.showSystemPrompt, v ? '1' : '0'); }
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
export const settings = new SettingsStore();
export const chatSettings = new ChatSettingsStore();
