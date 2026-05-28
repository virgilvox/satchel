// Agent loop that works with ANY WebLLM-supported model — including ones
// not on WebLLM's tools= whitelist. Strategy lifted from the mockup at
// mockups/satchel-chat (15).html, with comments describing why each
// piece exists.
//
// The core trick: instead of WebLLM's native FC API (which is "WIP" per
// their docs and only whitelists ~5 Hermes models), we drive the model
// with `response_format: { type: 'json_object', schema: ... }`. WebLLM
// hands the schema to XGrammar, which compiles it into a logit mask —
// the model literally cannot emit a token sequence that violates the
// schema. Output is always valid JSON, names are always real tools,
// arguments always match the tool's parameter shape.

import type { McpTool, ToolCallResult } from './types';

// XGrammar (the engine WebLLM uses for response_format) doesn't support
// every JSON Schema feature. Strip the risky bits to avoid silent
// grammar-compile failures while preserving the parts that matter.
const STRIP_KEYS = new Set([
  'minItems',
  'maxItems',
  'multipleOf',
  '$schema',
  '$id',
  'examples',
  'default',
  'title',
  'minLength',
  'maxLength',
  'pattern',
]);

function walk(node: unknown): unknown {
  if (Array.isArray(node)) return node.map(walk);
  if (!node || typeof node !== 'object') return node;
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(node as Record<string, unknown>)) {
    if (STRIP_KEYS.has(k)) continue;
    out[k] = walk(v);
  }
  return out;
}

export function sanitizeSchemaForXGrammar(schema: unknown): Record<string, unknown> {
  if (!schema || typeof schema !== 'object') return { type: 'object' };
  const cleaned = walk(schema) as Record<string, unknown>;
  // XGrammar wants `type: 'object'` at the root of an arguments schema.
  if (!cleaned.type && (cleaned.properties || cleaned.required)) {
    cleaned.type = 'object';
  }
  return cleaned;
}

/**
 * Build the JSON schema that constrains the model's output. The model
 * MUST emit one object per response: `{thought, tool_call: {name, arguments}}`.
 * The tool_call is constrained via `anyOf` to one of:
 *  - per-tool variants where `name` is locked to a const string AND
 *    `arguments` follows that tool's actual parameter schema (so the
 *    model can't hallucinate argument names or omit required fields)
 *  - the special `respond_to_user` pseudo-tool with `{answer: string}`,
 *    which the model uses to signal "I'm done; here is the answer".
 */
export function buildAgentSchema(tools: McpTool[]): Record<string, unknown> {
  const variants: Record<string, unknown>[] = [];
  for (const t of tools) {
    variants.push({
      type: 'object',
      properties: {
        thought: {
          type: 'string',
          description: '1–2 sentences of reasoning about what to do next.',
        },
        tool_call: {
          type: 'object',
          properties: {
            name: { type: 'string', enum: [t.name] },
            arguments: sanitizeSchemaForXGrammar(t.inputSchema ?? { type: 'object' }),
          },
          required: ['name', 'arguments'],
        },
      },
      required: ['thought', 'tool_call'],
    });
  }
  variants.push({
    type: 'object',
    properties: {
      thought: {
        type: 'string',
        description: '1–2 sentences explaining why you have enough evidence to answer.',
      },
      tool_call: {
        type: 'object',
        properties: {
          name: { type: 'string', enum: ['respond_to_user'] },
          arguments: {
            type: 'object',
            properties: {
              answer: {
                type: 'string',
                description:
                  'Final answer to the user, citing specific facts/sources from earlier tool results.',
              },
            },
            required: ['answer'],
          },
        },
        required: ['name', 'arguments'],
      },
    },
    required: ['thought', 'tool_call'],
  });
  return { anyOf: variants };
}

/**
 * Loose fallback schema: structurally valid envelope but doesn't constrain
 * argument shape per-tool. Used when the strict per-tool anyOf fails to
 * compile (some MCP tool schemas have features XGrammar still rejects
 * even after sanitization).
 */
export function buildLooseAgentSchema(tools: McpTool[]): Record<string, unknown> {
  const allNames = [...tools.map((t) => t.name), 'respond_to_user'];
  return {
    type: 'object',
    properties: {
      thought: { type: 'string' },
      tool_call: {
        type: 'object',
        properties: {
          name: { type: 'string', enum: allNames },
          arguments: { type: 'object' },
        },
        required: ['name', 'arguments'],
      },
    },
    required: ['thought', 'tool_call'],
  };
}

export interface ConstrainedParse {
  thought: string;
  toolCalls: ToolCallResult[];
  /** Set when the model selected `respond_to_user`. */
  answer: string | null;
  errors: string[];
}

/**
 * Repair JSON cut off mid-stream. XGrammar guarantees validity for
 * complete output, but stream cancellation can leave us with a truncated
 * tail — try to recover.
 */
export function repairJSON(s: string): string {
  s = s.trim().replace(/^```(?:json)?\s*|```\s*$/g, '').trim();
  s = s.replace(/,\s*([}\]])/g, '$1');
  let depth = 0;
  let bracket = 0;
  let inStr = false;
  let esc = false;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (esc) {
      esc = false;
      continue;
    }
    if (c === '\\') {
      esc = true;
      continue;
    }
    if (c === '"') {
      inStr = !inStr;
      continue;
    }
    if (inStr) continue;
    if (c === '{') depth++;
    else if (c === '}') depth--;
    else if (c === '[') bracket++;
    else if (c === ']') bracket--;
  }
  let out = s;
  if (inStr) out += '"';
  while (bracket-- > 0) out += ']';
  while (depth-- > 0) out += '}';
  return out;
}

export function parseConstrainedOutput(text: string): ConstrainedParse {
  const trimmed = (text || '').trim();
  if (!trimmed) {
    return { thought: '', toolCalls: [], answer: null, errors: ['empty output'] };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    try {
      parsed = JSON.parse(repairJSON(trimmed));
    } catch (e) {
      return {
        thought: '',
        toolCalls: [],
        answer: null,
        errors: [(e as Error).message],
      };
    }
  }
  const obj = parsed as { thought?: string; tool_call?: { name?: string; arguments?: unknown } };
  const thought = obj.thought ?? '';
  const tc = obj.tool_call;
  if (!tc || typeof tc !== 'object') {
    return { thought, toolCalls: [], answer: null, errors: ['missing tool_call object'] };
  }
  let args = tc.arguments;
  if (typeof args === 'string') {
    try {
      args = JSON.parse(args);
    } catch {
      args = {};
    }
  }
  if (!args || typeof args !== 'object') args = {};
  if (tc.name === 'respond_to_user') {
    const answer = (args as { answer?: string }).answer ?? '';
    return { thought, toolCalls: [], answer, errors: [] };
  }
  if (!tc.name) {
    return { thought, toolCalls: [], answer: null, errors: ['tool_call missing name'] };
  }
  return {
    thought,
    answer: null,
    toolCalls: [
      {
        id: 'call_' + Math.random().toString(36).slice(2, 10),
        name: tc.name,
        args: args as Record<string, unknown>,
        pending: true,
      },
    ],
    errors: [],
  };
}

/**
 * System prompt for constrained mode. The schema enforces structure;
 * this prompt supplies semantics — what each tool does, when to use
 * `respond_to_user`, persistence rules.
 */
export function constrainedSystemPrompt(opts: {
  tools: McpTool[];
  minToolCalls: number;
  weakScoreThreshold: number;
}): string {
  const { tools, minToolCalls, weakScoreThreshold } = opts;
  const toolDescriptions = tools
    .map((t) => {
      const schema = t.inputSchema ? JSON.stringify(t.inputSchema) : '{}';
      return `- ${t.name}: ${t.description || '(no description)'}\n  arguments schema: ${schema}`;
    })
    .join('\n');
  return `You are SATCHEL — a research agent for the user's local knowledge base.

Your training data is OUTDATED and INCOMPLETE compared to the user's local knowledge base. The user is asking about THEIR private data — facts that exist ONLY in the tools below, NOT in your training. You CANNOT answer without searching.

YOUR OUTPUT IS SCHEMA-CONSTRAINED. You will emit ONE JSON object per response with exactly this shape:
{
  "thought": "1-2 sentences of reasoning about what to do next",
  "tool_call": {
    "name": "<one of the available tool names, OR 'respond_to_user'>",
    "arguments": <object>
  }
}

AVAILABLE TOOLS (the COMPLETE list — nothing else exists):
${toolDescriptions}

PSEUDO-TOOL: "respond_to_user"
- Use this ONLY when you have enough evidence from tool results to answer the user.
- arguments: {"answer": "your synthesized answer to the user, citing specific facts/sources/scores from earlier tool results"}

PERSISTENCE RULES:
- MINIMUM ${minToolCalls} real tool calls (not respond_to_user) before you may use respond_to_user.
- If search scores are below ${weakScoreThreshold}, results are NOISE — do not answer from them. Try different keywords, list_sources, or get_document.
- "I don't have enough information" is a last resort, not a first move.
- When you finally answer via respond_to_user, cite specific sources, quotes, file paths, and scores.

WORKFLOW EXAMPLE:
User: "what is heatsync labs"
Round 1: {"thought": "Need to search the vault for heatsync labs.", "tool_call": {"name": "search_knowledge", "arguments": {"query": "heatsync labs"}}}
Round 2: (after seeing results) {"thought": "Found relevant chunks with high scores. Let me get a fuller document.", "tool_call": {"name": "get_document", "arguments": {"source": "..."}}}
Round 3+: more searches as needed
Final: {"thought": "I have enough evidence to answer.", "tool_call": {"name": "respond_to_user", "arguments": {"answer": "HeatSync Labs is... (cite sources)"}}}`;
}

// ─────────────────────────────────────────────────────────────────────────
// Smart-mode helpers (v2.9.0). Backend-aware tool-result handling,
// stall detection, and transcript compaction for small local LLMs.
//
// Why: local browser LLMs have small context (typically 4-8K usable),
// no prompt cache, and slow inference. The default chat loop's verbose
// tool results, repeating tool schemas, and fixed max_rounds combine
// into a "ran out of context before answering" failure mode that
// plagues every multi-round agent. These helpers fix the worst of it
// without changing the protocol the model sees.
// ─────────────────────────────────────────────────────────────────────────

/** Rough byte-to-token conversion. Matches the server-side estimator
 *  in src/ingest/mod.rs (`approximate_tokens`); good enough for budget
 *  gating without paying a real tokenizer roundtrip. */
export function approximateTokens(text: string): number {
  return Math.ceil((text?.length ?? 0) / 4);
}

export interface TruncateOpts {
  /** Hard cap on output token count. Caller passes the per-backend
   *  default (800 for WebLLM, undefined / very large for Anthropic). */
  maxTokens: number;
  /** Optional pointer the agent can use to recover the dropped tail.
   *  When set, the truncation marker mentions it explicitly. */
  recoverHint?: string;
}

/**
 * Cap a tool result at `maxTokens` while preserving the head (most
 * search engines and list tools put the most useful content there).
 * Appended marker tells the model how much was dropped and how to get
 * the rest, so the next round can do a targeted follow-up tool call
 * instead of re-issuing the same expensive search.
 *
 * When the input already fits under the budget, returned verbatim
 * (no copy in the hot path).
 */
export function truncateToolResult(text: string, opts: TruncateOpts): string {
  if (!text) return '';
  const tokens = approximateTokens(text);
  if (tokens <= opts.maxTokens) return text;
  // Convert token budget back to chars (4:1). Leave headroom for the
  // marker so the FINAL emitted result still fits the budget cleanly.
  const markerHint = opts.recoverHint
    ? ` Use \`${opts.recoverHint}\` to fetch the rest.`
    : '';
  const marker = `\n\n... [truncated ${tokens - opts.maxTokens} more tokens of output.${markerHint}]`;
  const markerTokens = approximateTokens(marker);
  const slackTokens = Math.max(opts.maxTokens - markerTokens, 1);
  const charBudget = slackTokens * 4;
  // Walk back to a UTF-8 char boundary to avoid splitting a multibyte
  // char or emoji mid-stream.
  let cut = Math.min(charBudget, text.length);
  while (cut > 0 && cut < text.length) {
    const code = text.charCodeAt(cut);
    // High surrogate — back up one.
    if (code >= 0xd800 && code <= 0xdbff) {
      cut -= 1;
      continue;
    }
    break;
  }
  return text.slice(0, cut) + marker;
}

/**
 * Stable fingerprint for a tool call. Used by the agent loop to detect
 * "the model just emitted the same tool call as last round," which is
 * the most common local-LLM failure mode (loops the same search with
 * tiny argument variations). Keys are sorted so {a: 1, b: 2} and
 * {b: 2, a: 1} hash to the same string.
 */
export function hashToolCall(name: string, args: unknown): string {
  return name + '|' + canonicalJsonStringify(args ?? {});
}

function canonicalJsonStringify(v: unknown): string {
  if (v === null || typeof v !== 'object') return JSON.stringify(v ?? null);
  if (Array.isArray(v)) return '[' + v.map(canonicalJsonStringify).join(',') + ']';
  const obj = v as Record<string, unknown>;
  const keys = Object.keys(obj).sort();
  return (
    '{' +
    keys.map((k) => JSON.stringify(k) + ':' + canonicalJsonStringify(obj[k])).join(',') +
    '}'
  );
}

/** What the agent loop concluded about the last tool call attempt. */
export type StallDecision =
  | { kind: 'ok' }
  | { kind: 'duplicate'; previousHash: string }
  | { kind: 'context-full'; usedFraction: number };

/**
 * Decide whether to abort the round and force a final-answer attempt.
 *
 *  - `duplicate` fires when the most recent tool call hash equals the
 *    one before it. The loop should inject a synthetic tool_result
 *    asking the model to either pick a different angle or call
 *    respond_to_user.
 *  - `context-full` fires at the configurable fraction (default 0.75)
 *    so the model has room for ONE more turn to wrap up before the
 *    window blows. Without this guard the model usually overruns and
 *    WebLLM throws "Prompt tokens exceed context window size".
 */
export function detectStallPattern(opts: {
  toolHashHistory: string[];
  contextUsedTokens: number;
  contextWindowTokens: number;
  contextFullFraction?: number;
}): StallDecision {
  const fullFrac = opts.contextFullFraction ?? 0.75;
  const used =
    opts.contextWindowTokens > 0
      ? opts.contextUsedTokens / opts.contextWindowTokens
      : 0;
  if (opts.contextWindowTokens > 0 && used >= fullFrac) {
    return { kind: 'context-full', usedFraction: used };
  }
  const h = opts.toolHashHistory;
  if (h.length >= 2 && h[h.length - 1] === h[h.length - 2]) {
    return { kind: 'duplicate', previousHash: h[h.length - 1] };
  }
  return { kind: 'ok' };
}

/**
 * Compact a transcript to fit a token budget by replacing the oldest
 * tool_use + tool_result pairs with one-line summaries lifted from
 * the model's own `thought` field. The system prompt, every user
 * message, and the most recent K tool exchanges are preserved
 * verbatim so the model still has fresh evidence to work with.
 *
 *  - input transcript: `{role, tokens, text, isToolPair?, summary?}`
 *  - returns the same transcript with oldest tool pairs replaced
 *  - idempotent: if everything already fits, returns the input
 *
 * The shape is deliberately storage-agnostic so this can run against
 * either the WebLLM constrained-mode transcript or the Anthropic
 * messages array.
 */
export interface CompactItem {
  role: 'system' | 'user' | 'assistant' | 'tool';
  text: string;
  tokens: number;
  /** True when this item is part of an assistant→tool→result triplet
   *  that compaction is allowed to collapse. Plain user/assistant turns
   *  are never collapsed. */
  isToolPair?: boolean;
  /** Pre-computed compaction summary the agent's `thought` field
   *  produced. Used as the replacement text when this pair is dropped. */
  summary?: string;
}

export function compactTranscript(
  items: CompactItem[],
  opts: { budgetTokens: number; keepRecentToolPairs?: number },
): CompactItem[] {
  const keep = opts.keepRecentToolPairs ?? 2;
  const total = items.reduce((sum, it) => sum + it.tokens, 0);
  if (total <= opts.budgetTokens) return items;

  // Identify tool-pair items by index, oldest first.
  const pairIdx: number[] = items
    .map((it, i) => (it.isToolPair ? i : -1))
    .filter((i) => i >= 0);
  if (pairIdx.length <= keep) return items;

  // Drop oldest pairs (keep the most recent `keep`) one at a time
  // until we fit the budget or run out of compactable items.
  const dropCount = pairIdx.length - keep;
  const out = items.slice();
  let dropped = 0;
  for (let i = 0; i < dropCount; i++) {
    const idx = pairIdx[i];
    const item = out[idx];
    const summaryText = item.summary
      ? `[compacted: ${item.summary}]`
      : `[compacted: prior ${item.role} turn dropped to free context]`;
    out[idx] = {
      role: item.role,
      text: summaryText,
      tokens: approximateTokens(summaryText),
      isToolPair: item.isToolPair,
      summary: item.summary,
    };
    dropped += item.tokens - out[idx].tokens;
    if (total - dropped <= opts.budgetTokens) break;
  }
  return out;
}

/**
 * Synthetic tool result the loop injects when stall detection fires
 * with `duplicate`. The model sees this on its next round and has a
 * clear nudge to either change angle or finalize.
 */
export function stallNudgeToolResult(): string {
  return (
    'You just issued an identical tool call to the previous round. ' +
    'Either pick a different search query / source / chunk_id, or call ' +
    '`respond_to_user` with the best answer you can give from the ' +
    'evidence you already have.'
  );
}

/**
 * Synthetic tool result for `context-full`. Tells the model it must
 * wrap up THIS round.
 */
export function contextFullNudgeToolResult(usedFraction: number): string {
  const pct = Math.round(usedFraction * 100);
  return (
    `Context is at ${pct}% of the model window. No more tool calls; ` +
    'call `respond_to_user` now with the best synthesis you can give from ' +
    'the evidence you already have. Cite sources from the tool results above.'
  );
}

/**
 * Compact alternative to `constrainedSystemPrompt` for small local
 * models. ~500 tokens vs. the verbose Anthropic-shaped prompt. Strict
 * operational shape, concrete one-liner per tool, two worked
 * examples. No style guidance ("no AI cliches" etc.) because small
 * models don't follow it anyway and the tokens are better spent on
 * tool-use mechanics.
 */
export function compactSystemPrompt(opts: {
  tools: McpTool[];
  minToolCalls: number;
}): string {
  const { tools, minToolCalls } = opts;
  const lines = tools.map((t) => `- ${t.name}: ${t.description?.split(/\.|\n/)[0] ?? '(no description)'}`);
  return `You are SATCHEL, an agent over the user's local vault. Emit ONE JSON object per turn:
{"thought":"...","tool_call":{"name":"...","arguments":{...}}}

Tools (only these exist; "respond_to_user" is the pseudo-tool to finalize):
${lines.join('\n')}
- respond_to_user: {"answer": "<final answer with citations>"}

Rules:
- Call at least ${minToolCalls} real tool(s) before respond_to_user.
- search_knowledge is almost always the first call. Use a single-sentence query.
- If a result is too long, it ends with "[truncated ... call get_chunk_context for the rest]" and the chunk_id you need.
- Don't repeat an identical tool call; vary your query or pick a different tool.

Example:
User: what is heatsync labs
Round 1: {"thought":"Need to search the vault.","tool_call":{"name":"search_knowledge","arguments":{"query":"heatsync labs"}}}
Round 2: {"thought":"Got 3 chunks describing HeatSync Labs as a makerspace.","tool_call":{"name":"respond_to_user","arguments":{"answer":"HeatSync Labs is a community makerspace in Mesa, AZ ..."}}}`;
}
