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
