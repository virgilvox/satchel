// Wrapper around @mlc-ai/web-llm for the satchel chat. The lib is
// dynamically imported on first use so the rest of the UI doesn't pay
// the parse cost when the user never opens the chat tab.

import type { ChatMessage, McpTool, ToolCallResult } from './types';

export interface ModelOption {
  id: string;
  label: string;
  // Approximate VRAM / weight footprint (rounded to the nearest 100 MB).
  size: string;
  // Whether the model is known to support OpenAI-style function/tool calling.
  toolCalling: boolean;
  // Whether the model emits <think>...</think> reasoning blocks.
  reasoning: boolean;
  notes?: string;
}

// Curated list. These ids are recognized by WebLLM's prebuilt config —
// adding new ones requires either matching the catalog or supplying a
// custom appConfig at engine init time.
export const MODELS: ModelOption[] = [
  {
    id: 'Llama-3.2-1B-Instruct-q4f16_1-MLC',
    label: 'Llama 3.2 · 1B',
    size: '~0.9 GB',
    toolCalling: true,
    reasoning: false,
    notes: 'Fastest. Good for quick lookups; weaker on multi-step reasoning.',
  },
  {
    id: 'Llama-3.2-3B-Instruct-q4f16_1-MLC',
    label: 'Llama 3.2 · 3B',
    size: '~2.3 GB',
    toolCalling: true,
    reasoning: false,
    notes: 'Balanced. Stronger reasoning than 1B at modest cost.',
  },
  {
    id: 'Hermes-3-Llama-3.2-3B-q4f16_1-MLC',
    label: 'Hermes 3 · 3B',
    size: '~2.3 GB',
    toolCalling: true,
    reasoning: false,
    notes: 'Tool-calling tuned. Best choice for MCP workflows.',
  },
  {
    id: 'Qwen2.5-3B-Instruct-q4f16_1-MLC',
    label: 'Qwen 2.5 · 3B',
    size: '~2.0 GB',
    toolCalling: true,
    reasoning: false,
    notes: 'Strong general assistant; reliable tool calls.',
  },
  {
    id: 'DeepSeek-R1-Distill-Qwen-7B-q4f16_1-MLC',
    label: 'DeepSeek R1 · 7B (reasoning)',
    size: '~5.2 GB',
    toolCalling: false,
    reasoning: true,
    notes: 'Emits <think> reasoning. Larger; needs ≥8 GB GPU memory.',
  },
  {
    id: 'Phi-3.5-mini-instruct-q4f16_1-MLC',
    label: 'Phi 3.5 · 3.8B',
    size: '~2.4 GB',
    toolCalling: true,
    reasoning: false,
    notes: 'Microsoft Phi 3.5. Compact, very capable.',
  },
];

export interface InitProgress {
  text: string;
  progress: number; // 0..1
  timeElapsed: number;
}

export interface EngineHandle {
  modelId: string;
  unload: () => Promise<void>;
  /** Best-effort cancel of an in-flight chat. Stops new tokens from arriving;
   *  the for-await loop also bails out on the first AbortSignal tick. */
  interrupt: () => void;
  chat: (
    messages: WebLLMMessage[],
    opts: ChatOpts
  ) => Promise<{ content: string; reasoning?: string; toolCalls?: ToolCallResult[] }>;
}

interface WebLLMMessage {
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
  // For tool messages: the id of the tool call this is the result of.
  tool_call_id?: string;
  // For assistant messages: the tool calls the model emitted.
  tool_calls?: Array<{
    id: string;
    type: 'function';
    function: { name: string; arguments: string };
  }>;
  name?: string;
}

interface ChatOpts {
  tools?: ChatToolSpec[];
  temperature?: number;
  max_tokens?: number;
  signal?: AbortSignal;
  onDelta?: (delta: string) => void;
}

export interface ChatToolSpec {
  type: 'function';
  function: {
    name: string;
    description: string;
    parameters: Record<string, unknown>;
  };
}

let cachedLib: typeof import('@mlc-ai/web-llm') | null = null;
async function loadLib() {
  if (!cachedLib) {
    cachedLib = await import('@mlc-ai/web-llm');
  }
  return cachedLib;
}

export async function checkSupport(): Promise<{ supported: boolean; reason?: string }> {
  if (!('gpu' in navigator)) {
    return { supported: false, reason: 'WebGPU is not available in this browser.' };
  }
  try {
    const adapter = await (navigator as Navigator & { gpu: { requestAdapter: () => Promise<unknown> } }).gpu.requestAdapter();
    if (!adapter) return { supported: false, reason: 'No WebGPU adapter — try a different browser or enable hardware acceleration.' };
  } catch (e) {
    return { supported: false, reason: 'WebGPU adapter request failed: ' + (e as Error).message };
  }
  return { supported: true };
}

export async function createEngine(
  modelId: string,
  onProgress: (p: InitProgress) => void
): Promise<EngineHandle> {
  const webllm = await loadLib();
  const engine = await webllm.CreateMLCEngine(modelId, {
    initProgressCallback: (p: InitProgress) => onProgress(p),
  });

  const handle: EngineHandle = {
    modelId,
    unload: async () => {
      await engine.unload();
    },
    interrupt: () => {
      try { engine.interruptGenerate(); } catch { /* engine may not be generating */ }
    },
    chat: async (messages, opts) => {
      const stream = await engine.chat.completions.create({
        messages: messages as any,
        stream: true,
        tools: opts.tools as any,
        temperature: opts.temperature ?? 0.6,
        max_tokens: opts.max_tokens ?? 1024,
      });

      let content = '';
      const toolCallBuf: Record<number, { id: string; name: string; args: string }> = {};

      for await (const chunk of stream as any) {
        if (opts.signal?.aborted) {
          break;
        }
        const choice = chunk.choices?.[0];
        if (!choice) continue;
        const delta = choice.delta;
        if (delta?.content) {
          content += delta.content;
          opts.onDelta?.(delta.content);
        }
        if (delta?.tool_calls) {
          for (const tc of delta.tool_calls) {
            const idx = tc.index ?? 0;
            const buf = (toolCallBuf[idx] ??= { id: '', name: '', args: '' });
            if (tc.id) buf.id = tc.id;
            if (tc.function?.name) buf.name = tc.function.name;
            if (tc.function?.arguments) buf.args += tc.function.arguments;
          }
        }
      }

      const toolCalls: ToolCallResult[] = Object.values(toolCallBuf).map((tc) => {
        let args: Record<string, unknown> = {};
        try {
          args = tc.args ? JSON.parse(tc.args) : {};
        } catch {
          args = { _raw: tc.args };
        }
        return {
          id: tc.id || crypto.randomUUID(),
          name: tc.name,
          args,
          pending: true,
        };
      });

      // Pull <think>...</think> reasoning out of the assistant content.
      // Tolerate leading whitespace before <think> — some models emit a
      // newline first.
      let reasoning: string | undefined;
      const m = /^\s*<think>([\s\S]*?)<\/think>\s*([\s\S]*)$/.exec(content);
      let userContent = content;
      if (m) {
        reasoning = m[1].trim();
        userContent = m[2].trim();
      }

      return {
        content: userContent,
        reasoning,
        toolCalls: toolCalls.length ? toolCalls : undefined,
      };
    },
  };

  return handle;
}

/** Convert MCP tool descriptors to WebLLM's OpenAI-style tool specs. */
export function mcpToolsToWebLlmTools(tools: McpTool[]): ChatToolSpec[] {
  return tools.map((t) => ({
    type: 'function',
    function: {
      name: t.name,
      description: t.description,
      parameters: (t.inputSchema as Record<string, unknown>) ?? {
        type: 'object',
        properties: {},
      },
    },
  }));
}

/** Convert ChatMessage transcript to WebLLM message history. */
export function transcriptToWebLlmMessages(
  systemPrompt: string,
  transcript: ChatMessage[]
): WebLLMMessage[] {
  const out: WebLLMMessage[] = [{ role: 'system', content: systemPrompt }];
  for (const m of transcript) {
    if (m.role === 'user') {
      out.push({ role: 'user', content: m.content });
    } else if (m.role === 'assistant') {
      const calls = m.toolCalls?.length
        ? m.toolCalls.map((tc) => ({
            id: tc.id,
            type: 'function' as const,
            function: { name: tc.name, arguments: JSON.stringify(tc.args) },
          }))
        : undefined;
      out.push({
        role: 'assistant',
        content: m.content,
        tool_calls: calls,
      });
      if (m.toolCalls) {
        for (const tc of m.toolCalls) {
          out.push({
            role: 'tool',
            tool_call_id: tc.id,
            name: tc.name,
            content: tc.error ? `error: ${tc.error}` : tc.result ?? '',
          });
        }
      }
    } else if (m.role === 'tool') {
      out.push({ role: 'tool', content: m.content });
    } else if (m.role === 'system') {
      // System turns from the user override the boot prompt; later wins.
      out.push({ role: 'system', content: m.content });
    }
    // 'error' role is UI-only.
  }
  return out;
}
