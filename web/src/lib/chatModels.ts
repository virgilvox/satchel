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

export interface ChatModelPricing {
  /** Dollars per 1,000,000 input tokens. Cache writes are 1.25× this
   *  (5-min TTL); cache reads are 0.1× this. */
  inputPerMillion: number;
  /** Dollars per 1,000,000 output tokens. */
  outputPerMillion: number;
}

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
  /** Public per-token pricing for cost-meter display. Anthropic-only;
   *  WebLLM is always free at runtime. */
  pricing?: ChatModelPricing;
  /** Anthropic models only: true when the model supports the extended
   *  thinking API surface (`thinking: { type: "adaptive" }` and
   *  `output_config.effort`). False on Haiku 4.5 and any other model
   *  that does not have extended thinking; those params would return
   *  400 from the API and must be omitted from the request body. */
  supportsExtendedThinking?: boolean;
}

const ANTHROPIC_MODELS: ChatModel[] = [
  {
    id: 'claude-opus-4-7',
    label: 'Claude Opus 4.7',
    group: 'Anthropic API',
    size: 'API',
    backend: 'anthropic',
    notes: 'Most capable. Anthropic API key required (Settings → Anthropic API).',
    pricing: { inputPerMillion: 5, outputPerMillion: 25 },
    supportsExtendedThinking: true,
  },
  {
    id: 'claude-sonnet-4-6',
    label: 'Claude Sonnet 4.6',
    group: 'Anthropic API',
    size: 'API',
    backend: 'anthropic',
    notes: 'Balanced. Strong tool use; faster than Opus.',
    pricing: { inputPerMillion: 3, outputPerMillion: 15 },
    supportsExtendedThinking: true,
  },
  {
    id: 'claude-haiku-4-5',
    label: 'Claude Haiku 4.5',
    group: 'Anthropic API',
    size: 'API',
    backend: 'anthropic',
    notes: 'Fastest, cheapest. No extended thinking; effort/thinking knobs do not apply.',
    pricing: { inputPerMillion: 1, outputPerMillion: 5 },
    supportsExtendedThinking: false,
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
