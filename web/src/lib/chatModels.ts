// Unified chat-model registry. Two backends share one picker:
//
//   "webllm"    — local browser LLM via @mlc-ai/web-llm
//   "anthropic" — Anthropic Messages API, proxied through satchel
//
// Each entry tells the chat which `runLoop` branch to take when the user
// hits LOAD/SEND. Adding a new backend = add a new `ChatModel.backend`
// variant + a runLoop branch in Chat.svelte.

import { MODELS as WEBLLM_MODELS, type ModelOption } from './webllm';

export type ChatModelBackend = 'webllm' | 'anthropic';

export interface ChatModel {
  /** Stable id used in the picker + persisted to localStorage. For
   *  WebLLM models this matches @mlc-ai/web-llm's prebuilt id; for
   *  Anthropic models it's the API model name. */
  id: string;
  /** Display label in the dropdown. */
  label: string;
  /** Group label for `<optgroup>`. */
  group: string;
  /** Footprint shown in the dropdown ("~2.3 GB" for local, "API" for cloud). */
  size: string;
  backend: ChatModelBackend;
  notes?: string;
}

const ANTHROPIC_MODELS: ChatModel[] = [
  {
    id: 'claude-opus-4-7',
    label: 'Claude Opus 4.7',
    group: 'Anthropic API',
    size: 'API',
    backend: 'anthropic',
    notes: 'Most capable. Anthropic API key required (Settings → Anthropic API).',
  },
  {
    id: 'claude-sonnet-4-6',
    label: 'Claude Sonnet 4.6',
    group: 'Anthropic API',
    size: 'API',
    backend: 'anthropic',
    notes: 'Balanced. Strong tool use; faster than Opus.',
  },
  {
    id: 'claude-haiku-4-5',
    label: 'Claude Haiku 4.5',
    group: 'Anthropic API',
    size: 'API',
    backend: 'anthropic',
    notes: 'Fastest, cheapest. Good for quick lookups.',
  },
];

const WEBLLM_AS_CHAT: ChatModel[] = WEBLLM_MODELS.map((m: ModelOption) => ({
  id: m.id,
  label: m.label,
  group: 'Local · WebLLM',
  size: m.size,
  backend: 'webllm' as const,
  notes: m.notes,
}));

/** Single picker list — Anthropic group rendered first (so the Claude
 *  models are above the local ones, matching the user's "make it clear
 *  Claude is an option" ask). */
export const CHAT_MODELS: ChatModel[] = [...ANTHROPIC_MODELS, ...WEBLLM_AS_CHAT];

export function findChatModel(id: string): ChatModel | undefined {
  return CHAT_MODELS.find((m) => m.id === id);
}

/** Group entries by their `group` field while preserving the order of
 *  first appearance — feeds straight into a Svelte `<optgroup>` loop. */
export function groupedChatModels(): Array<{ name: string; items: ChatModel[] }> {
  const out: Array<{ name: string; items: ChatModel[] }> = [];
  for (const m of CHAT_MODELS) {
    const last = out[out.length - 1];
    if (!last || last.name !== m.group) out.push({ name: m.group, items: [m] });
    else last.items.push(m);
  }
  return out;
}
