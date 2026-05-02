// Client for the satchel-side Anthropic Messages proxy.
//
// The browser POSTs to `/api/anthropic/messages`; satchel adds the
// `x-api-key` + `anthropic-version` headers from the saved config and
// pipes Anthropic's SSE stream straight back to us. This module decodes
// that SSE stream into typed events so the chat agent loop never has to
// touch the wire format.

const ENDPOINT = '/api/anthropic/messages';

export interface AnthropicTool {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
}

export interface AnthropicTextBlock {
  type: 'text';
  text: string;
}
export interface AnthropicToolUseBlock {
  type: 'tool_use';
  id: string;
  name: string;
  input: Record<string, unknown>;
}
export type AnthropicContentBlock = AnthropicTextBlock | AnthropicToolUseBlock;

export interface AnthropicMessage {
  role: 'user' | 'assistant';
  content: string | AnthropicContentBlock[];
}

export type AnthropicEffortLevel = 'low' | 'medium' | 'high' | 'xhigh' | 'max';

export interface AnthropicStreamRequest {
  model: string;
  messages: AnthropicMessage[];
  tools?: AnthropicTool[];
  /** Plain string is convenient for callers; we promote it to a single
   *  `text` block server-side when caching is enabled. */
  system?: string;
  /** Defaults to 16000. Streaming tolerates up to 64000 on Opus 4.7 /
   *  Sonnet 4.6, but most chat turns finish well under 16k and the
   *  smaller default keeps bills predictable. */
  max_tokens?: number;
  /** When true, attaches `cache_control: {type: "ephemeral", ttl: "1h"}`
   *  to the system prompt so the tools + system prefix is reused across
   *  turns. Render order is tools then system, so a single breakpoint on
   *  the last system block caches both. */
  cache?: boolean;
  /** Adaptive thinking is the recommended mode on Opus 4.7 / 4.6 /
   *  Sonnet 4.6. `disabled` skips it for latency-sensitive cases. */
  thinking?: 'adaptive' | 'disabled';
  /** Effort lives inside `output_config`, not at the top level. We
   *  accept it flat here and rewrap. */
  effort?: AnthropicEffortLevel;
}

export interface AnthropicUsage {
  input_tokens?: number;
  output_tokens?: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
}

export interface AnthropicTurnResult {
  /** Concatenated text-block content. */
  text: string;
  /** Any tool_use blocks the model emitted this turn. */
  toolUses: AnthropicToolUseBlock[];
  /** "end_turn" | "tool_use" | "stop_sequence" | "max_tokens" | "refusal". */
  stopReason: string | null;
  /** Token accounting from the upstream `message_start` + `message_delta`
   *  events. Cache fields are populated when prompt caching is in use. */
  usage: AnthropicUsage;
  /** Set on Anthropic API error (4xx / 5xx parsed out of the SSE stream). */
  error?: string;
}

/**
 * Run one Anthropic Messages turn through the satchel proxy. Returns when
 * the SSE stream ends. The caller drives the agent loop: if the result
 * contains tool_uses, dispatch them, push tool_result blocks, and call
 * this again.
 *
 * `onTextDelta` fires for each `content_block_delta` of type `text_delta`
 * so the UI can stream the assistant's prose into a bubble live.
 */
export async function streamAnthropicTurn(
  req: AnthropicStreamRequest,
  onTextDelta: (delta: string) => void,
  signal?: AbortSignal,
): Promise<AnthropicTurnResult> {
  // Build the wire-format body with caching, thinking, and effort applied.
  // System prompt becomes a single-block array when caching is enabled so
  // we can attach `cache_control` to it; otherwise we leave it as a bare
  // string (smaller payload, no behavioral difference).
  const wire: Record<string, unknown> = {
    model: req.model,
    messages: req.messages,
    stream: true,
    max_tokens: req.max_tokens ?? 16000,
  };
  if (req.tools?.length) wire.tools = req.tools;
  if (req.system) {
    if (req.cache) {
      wire.system = [
        {
          type: 'text',
          text: req.system,
          cache_control: { type: 'ephemeral', ttl: '1h' },
        },
      ];
    } else {
      wire.system = req.system;
    }
  }
  if (req.thinking === 'adaptive') {
    wire.thinking = { type: 'adaptive' };
  }
  if (req.effort) {
    wire.output_config = { effort: req.effort };
  }

  const res = await fetch(ENDPOINT, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(wire),
    signal,
  });
  if (!res.ok) {
    let detail = '';
    try {
      const j = await res.json();
      detail = (j as { error?: string }).error ?? JSON.stringify(j);
    } catch {
      detail = await res.text().catch(() => '');
    }
    return {
      text: '',
      toolUses: [],
      stopReason: null,
      usage: {},
      error: `anthropic proxy ${res.status}: ${detail || res.statusText}`,
    };
  }
  if (!res.body) {
    return {
      text: '',
      toolUses: [],
      stopReason: null,
      usage: {},
      error: 'empty response body',
    };
  }

  // Accumulators. Anthropic's stream emits message_start, content_block_start
  // (per block), content_block_delta (one per chunk), content_block_stop,
  // message_delta (with stop_reason), message_stop, plus error/ping events.
  let text = '';
  const blocks: Record<number, { type: 'text' | 'tool_use'; partial: string; meta?: { id?: string; name?: string } }> =
    {};
  let stopReason: string | null = null;
  let streamError: string | undefined;
  const usage: AnthropicUsage = {};

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    // SSE events are separated by blank lines.
    let sep = buf.indexOf('\n\n');
    while (sep !== -1) {
      const event = buf.slice(0, sep);
      buf = buf.slice(sep + 2);
      sep = buf.indexOf('\n\n');
      processEvent(event);
    }
  }

  function mergeUsage(u: Record<string, unknown> | undefined) {
    if (!u || typeof u !== 'object') return;
    if (typeof u.input_tokens === 'number') usage.input_tokens = u.input_tokens;
    if (typeof u.output_tokens === 'number') usage.output_tokens = u.output_tokens;
    if (typeof u.cache_creation_input_tokens === 'number')
      usage.cache_creation_input_tokens = u.cache_creation_input_tokens;
    if (typeof u.cache_read_input_tokens === 'number')
      usage.cache_read_input_tokens = u.cache_read_input_tokens;
  }

  function processEvent(raw: string) {
    let eventType: string | null = null;
    const dataLines: string[] = [];
    for (const line of raw.split('\n')) {
      if (line.startsWith('event:')) eventType = line.slice(6).trim();
      else if (line.startsWith('data:')) dataLines.push(line.slice(5).trim());
    }
    if (!dataLines.length) return;
    const data = dataLines.join('\n');
    let payload: any;
    try {
      payload = JSON.parse(data);
    } catch {
      return;
    }
    switch (eventType ?? payload?.type) {
      case 'message_start': {
        // Initial usage: input_tokens + cache_* fields.
        mergeUsage(payload?.message?.usage);
        break;
      }
      case 'content_block_start': {
        const idx = payload.index as number;
        const t = payload.content_block?.type;
        if (t === 'text') {
          blocks[idx] = { type: 'text', partial: '' };
        } else if (t === 'tool_use') {
          blocks[idx] = {
            type: 'tool_use',
            partial: '',
            meta: { id: payload.content_block.id, name: payload.content_block.name },
          };
        }
        break;
      }
      case 'content_block_delta': {
        const idx = payload.index as number;
        const block = blocks[idx];
        if (!block) return;
        const delta = payload.delta;
        if (delta?.type === 'text_delta' && typeof delta.text === 'string') {
          block.partial += delta.text;
          text += delta.text;
          onTextDelta(delta.text);
        } else if (delta?.type === 'input_json_delta' && typeof delta.partial_json === 'string') {
          block.partial += delta.partial_json;
        }
        break;
      }
      case 'message_delta': {
        if (payload.delta?.stop_reason) stopReason = payload.delta.stop_reason;
        // Final usage update: output_tokens lands here.
        mergeUsage(payload?.usage);
        break;
      }
      case 'error': {
        streamError = payload.error?.message ?? JSON.stringify(payload);
        break;
      }
    }
  }

  // Materialize tool_use blocks from buffered partial JSON.
  const toolUses: AnthropicToolUseBlock[] = [];
  for (const idx of Object.keys(blocks).map(Number).sort((a, b) => a - b)) {
    const b = blocks[idx];
    if (b.type !== 'tool_use') continue;
    let input: Record<string, unknown> = {};
    if (b.partial) {
      try {
        input = JSON.parse(b.partial);
      } catch {
        input = { _raw: b.partial };
      }
    }
    toolUses.push({
      type: 'tool_use',
      id: b.meta?.id ?? '',
      name: b.meta?.name ?? '',
      input,
    });
  }

  return { text, toolUses, stopReason, usage, error: streamError };
}

export async function getAnthropicConfigured(): Promise<boolean> {
  try {
    const r = await fetch('/api/anthropic/config');
    const j = (await r.json()) as { configured?: boolean };
    return !!j.configured;
  } catch {
    return false;
  }
}

export async function setAnthropicKey(key: string): Promise<{ ok: boolean; error?: string }> {
  const r = await fetch('/api/anthropic/config', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ api_key: key }),
  });
  const j = (await r.json()) as { configured?: boolean; error?: string };
  return { ok: !!j.configured && !j.error, error: j.error };
}

export async function clearAnthropicKey(): Promise<void> {
  await fetch('/api/anthropic/config', { method: 'DELETE' });
}
