// Wrapper around @mlc-ai/web-llm for the satchel chat. The lib is
// dynamically imported on first use so the rest of the UI doesn't pay
// the parse cost when the user never opens the chat tab.
//
// The chat client never uses WebLLM's native `tools=` API path because
// that's gated to ~5 Hermes models per WebLLM's whitelist (and is "WIP"
// per their README). All function calling goes through constrained mode:
// `response_format: { type: 'json_object', schema: ... }` with an agent
// schema built from the live MCP tool list. See lib/agent.ts.

export interface InitProgress {
  text: string;
  progress: number; // 0..1
  timeElapsed: number;
}

/** Per-engine overrides forwarded to WebLLM at create time. The context
 *  knobs are the load-bearing ones — Hermes 3 3B compiles to a 4096-token
 *  context by default and a 4-turn agent transcript blows past that.
 *  Setting `contextWindowSize` to 8192 (when the model supports it) or
 *  enabling `slidingWindowSize` gets you out of the jam. */
export interface EngineChatOpts {
  contextWindowSize?: number;
  slidingWindowSize?: number;
}

export interface EngineHandle {
  modelId: string;
  unload: () => Promise<void>;
  /** Best-effort cancel of an in-flight chat. */
  interrupt: () => void;
  /** Stream a chat completion. Returns the final raw content + usage. */
  stream: (req: ChatRequest, onDelta: (delta: string) => void) => Promise<ChatResult>;
}

export interface ChatRequest {
  messages: Array<{ role: 'system' | 'user' | 'assistant' | 'tool'; content: string }>;
  /** Stringified JSON schema. Forwarded as response_format.json_schema. */
  schema?: string;
  temperature?: number;
  max_tokens?: number;
}

export interface ChatResult {
  content: string;
  usage?: { prompt_tokens?: number; completion_tokens?: number; total_tokens?: number };
}

export interface ModelOption {
  id: string;
  label: string;
  size: string;
  /** Param-bucket: small (<=2B), medium (2–4B), large (4–9B), xl (9B+). */
  category: 'small' | 'medium' | 'large' | 'xl';
  notes?: string;
}

// Curated list of WebLLM-cataloged models that work well with
// constrained-mode tool calling. Lifted from the mockup's KEEP list.
// Each id MUST exist in @mlc-ai/web-llm's prebuiltAppConfig.model_list —
// we re-validate at engine-create time, and the engine throws clearly
// if WebLLM doesn't recognize the id.
//
// All of these can drive tool calls via constrained decoding even when
// they're not on WebLLM's native FC whitelist — XGrammar enforces the
// schema at the logit level regardless of model family.
export const MODELS: ModelOption[] = [
  {
    id: 'Llama-3.2-1B-Instruct-q4f16_1-MLC',
    label: 'Llama 3.2 · 1B',
    size: '~0.9 GB',
    category: 'small',
    notes: 'Fastest. 1B is borderline reliable for multi-step research; OK for quick lookups.',
  },
  {
    id: 'Llama-3.2-3B-Instruct-q4f16_1-MLC',
    label: 'Llama 3.2 · 3B',
    size: '~2.3 GB',
    category: 'medium',
    notes: 'Balanced default. Good Pre-Act behavior with our schema.',
  },
  {
    id: 'Hermes-3-Llama-3.2-3B-q4f16_1-MLC',
    label: 'Hermes 3 · 3B (FC-tuned)',
    size: '~2.3 GB',
    category: 'medium',
    notes: 'Nous Hermes 3, fine-tuned on tool calling. Strongest 3B for MCP workflows.',
  },
  {
    id: 'Qwen2.5-3B-Instruct-q4f16_1-MLC',
    label: 'Qwen 2.5 · 3B',
    size: '~2.0 GB',
    category: 'medium',
    notes: 'Native Hermes-style tool format in its chat template.',
  },
  {
    id: 'Phi-3.5-mini-instruct-q4f16_1-MLC',
    label: 'Phi 3.5 · 3.8B',
    size: '~2.4 GB',
    category: 'medium',
    notes: 'Microsoft Phi 3.5. Compact, very capable.',
  },
  {
    id: 'gemma-2-2b-it-q4f16_1-MLC',
    label: 'Gemma 2 · 2B',
    size: '~1.8 GB',
    category: 'small',
    notes: 'Google Gemma 2. No system role — we fold instructions into the first user turn.',
  },
  {
    id: 'Llama-3.1-8B-Instruct-q4f32_1-MLC',
    label: 'Llama 3.1 · 8B',
    size: '~4.7 GB',
    category: 'large',
    notes: 'Stronger reasoning at the cost of VRAM. Needs ≥6 GB.',
  },
  {
    id: 'Hermes-3-Llama-3.1-8B-q4f16_1-MLC',
    label: 'Hermes 3 · 8B (FC-tuned)',
    size: '~4.7 GB',
    category: 'large',
    notes: 'Nous Hermes 3 8B. The most reliable open tool-caller in this size class.',
  },
  {
    id: 'Hermes-2-Pro-Llama-3-8B-q4f16_1-MLC',
    label: 'Hermes 2 Pro · 8B',
    size: '~4.7 GB',
    category: 'large',
    notes: 'Earlier Hermes; still very strong on FC. WebLLM-whitelisted for tools=.',
  },
  {
    id: 'Hermes-2-Pro-Mistral-7B-q4f16_1-MLC',
    label: 'Hermes 2 Pro · Mistral 7B',
    size: '~4.4 GB',
    category: 'large',
    notes: 'Mistral-base Hermes. Comparable quality to the Llama variant.',
  },
  {
    id: 'DeepSeek-R1-Distill-Qwen-7B-q4f16_1-MLC',
    label: 'DeepSeek R1 · 7B (reasoning)',
    size: '~5.2 GB',
    category: 'large',
    notes: 'Emits <think>…</think> reasoning. The chat collapses it into a panel.',
  },
];

let cachedLib: typeof import('@mlc-ai/web-llm') | null = null;
async function loadLib() {
  if (!cachedLib) {
    cachedLib = await import('@mlc-ai/web-llm');
  }
  return cachedLib;
}

export async function checkSupport(): Promise<{ supported: boolean; reason?: string }> {
  // WebGPU only exists on `navigator.gpu` in a secure context. The browser
  // treats `http://localhost` and `http://127.0.0.1` as secure by spec
  // (the "potentially trustworthy origin" allow-list), but it does NOT
  // extend that exemption to arbitrary `.local` hostnames or LAN IPs.
  // So opening SATCHEL via `http://satchel.local:7428` from the same
  // machine silently disables WebGPU even though the laptop has it.
  // Make the failure mode actionable: if we are clearly in an insecure
  // origin, name it so the user knows to switch to localhost.
  if (!window.isSecureContext) {
    const here = window.location.host;
    return {
      supported: false,
      reason:
        'Insecure origin: ' +
        here +
        '. WebGPU is only exposed on localhost / 127.0.0.1 or HTTPS. Open SATCHEL at http://localhost:' +
        window.location.port +
        ' on this machine to enable the local LLM.',
    };
  }
  if (!('gpu' in navigator)) {
    return { supported: false, reason: 'WebGPU is not available in this browser.' };
  }
  try {
    const adapter = await (
      navigator as Navigator & { gpu: { requestAdapter: () => Promise<unknown> } }
    ).gpu.requestAdapter();
    if (!adapter) {
      return {
        supported: false,
        reason: 'No WebGPU adapter; try Chrome 113+ on macOS/Windows, or enable hardware acceleration.',
      };
    }
  } catch (e) {
    return { supported: false, reason: 'WebGPU adapter request failed: ' + (e as Error).message };
  }
  return { supported: true };
}

export async function createEngine(
  modelId: string,
  onProgress: (p: InitProgress) => void,
  chatOpts?: EngineChatOpts
): Promise<EngineHandle> {
  const webllm = await loadLib();
  // WebLLM's third-arg `chatOpts` accepts `context_window_size` and
  // `sliding_window_size` (snake_case in the underlying TS types).
  // Only ONE of them can be positive — the other must be -1 (or omitted)
  // or WebLLM rejects with "Only one of context_window_size and
  // sliding_window_size can be positive." Pass through the user's chosen
  // override and explicitly set the opposite to -1 so a stale model-card
  // default can't reintroduce the collision.
  const ctx = chatOpts?.contextWindowSize;
  const sliding = chatOpts?.slidingWindowSize;
  let llmChatOpts: Record<string, number> | undefined;
  if (ctx && ctx > 0) {
    llmChatOpts = { context_window_size: ctx, sliding_window_size: -1 };
  } else if (sliding && sliding > 0) {
    llmChatOpts = { context_window_size: -1, sliding_window_size: sliding };
  }
  const opts = llmChatOpts;

  const engine = await webllm.CreateMLCEngine(
    modelId,
    { initProgressCallback: (p: InitProgress) => onProgress(p) },
    opts as any
  );

  return {
    modelId,
    unload: async () => {
      try {
        await engine.unload();
      } catch {
        /* engine already unloaded */
      }
    },
    interrupt: () => {
      try {
        engine.interruptGenerate();
      } catch {
        /* no in-flight generation */
      }
    },
    stream: async (req, onDelta) => {
      // WebLLM's response_format expects a stringified JSON schema. The
      // mockup's example link:
      //   https://github.com/mlc-ai/web-llm/blob/main/examples/json-schema/src/json_schema.ts
      const llmReq: Record<string, unknown> = {
        messages: req.messages,
        temperature: req.temperature ?? 0.6,
        max_tokens: req.max_tokens ?? 1024,
        stream: true,
        stream_options: { include_usage: true },
      };
      if (req.schema) {
        llmReq.response_format = { type: 'json_object', schema: req.schema };
      }
      const stream = await engine.chat.completions.create(llmReq as any);
      let content = '';
      let usage: ChatResult['usage'] | undefined;
      for await (const chunk of stream as any) {
        if (chunk.usage) usage = chunk.usage;
        const choice = chunk.choices?.[0];
        if (!choice) continue;
        const delta = choice.delta?.content;
        if (delta) {
          content += delta;
          onDelta(delta);
        }
      }
      return { content, usage };
    },
  };
}
