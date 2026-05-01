// App-wide reactive state. Svelte 5 runes — these are exported state
// containers other modules can mutate or subscribe to.

import type { Mode, StatusResponse, Tab } from './types';

const STORAGE_MODE_KEY = 'satchel-mode';
const STORAGE_MCP_KEY = 'satchel-mcp-endpoint';
const STORAGE_MODEL_KEY = 'satchel-chat-model';
const STORAGE_SYSTEM_KEY = 'satchel-chat-system';

function initialMode(): Mode {
  const stored = localStorage.getItem(STORAGE_MODE_KEY) as Mode | null;
  if (stored === 'dark' || stored === 'light') return stored;
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

class ThemeStore {
  mode = $state<Mode>(initialMode());

  constructor() {
    this.apply();
  }

  toggle() {
    this.mode = this.mode === 'dark' ? 'light' : 'dark';
    localStorage.setItem(STORAGE_MODE_KEY, this.mode);
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

class SettingsStore {
  mcpEndpoint = $state<string>(
    localStorage.getItem(STORAGE_MCP_KEY) ?? window.location.origin + '/mcp'
  );
  chatModel = $state<string>(
    localStorage.getItem(STORAGE_MODEL_KEY) ??
      'Llama-3.2-1B-Instruct-q4f16_1-MLC'
  );
  systemPrompt = $state<string>(
    localStorage.getItem(STORAGE_SYSTEM_KEY) ??
      `You are SATCHEL, a knowledge assistant grounded in the user's local vault.
When you need facts, use the search_knowledge tool — never invent sources.
Cite sources by their path. Keep answers concise.`
  );

  setMcp(url: string) {
    this.mcpEndpoint = url;
    localStorage.setItem(STORAGE_MCP_KEY, url);
  }

  setModel(id: string) {
    this.chatModel = id;
    localStorage.setItem(STORAGE_MODEL_KEY, id);
  }

  setSystem(prompt: string) {
    this.systemPrompt = prompt;
    localStorage.setItem(STORAGE_SYSTEM_KEY, prompt);
  }
}

export const theme = new ThemeStore();
export const status = new StatusStore();
export const router = new RouterStore();
export const settings = new SettingsStore();
