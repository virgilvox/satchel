<script lang="ts">
  import { onMount } from 'svelte';
  import ViewHead from '../components/ViewHead.svelte';
  import Composer from '../components/Composer.svelte';
  import MessageBubble from '../components/MessageBubble.svelte';
  import Mark from '../components/Mark.svelte';
  import Pill from '../components/Pill.svelte';
  import Dot from '../components/Dot.svelte';
  import StatusLine from '../components/StatusLine.svelte';
  import SettingsModal from '../components/SettingsModal.svelte';
  import {
    checkSupport,
    createEngine,
    type EngineHandle,
    type InitProgress,
  } from '../lib/webllm';
  import {
    buildAgentSchema,
    buildLooseAgentSchema,
    constrainedSystemPrompt,
    parseConstrainedOutput,
    compactSystemPrompt,
    truncateToolResult,
    hashToolCall,
    detectStallPattern,
    stallNudgeToolResult,
    contextFullNudgeToolResult,
  } from '../lib/agent';
  import { McpClient } from '../lib/mcp';
  import { api } from '../lib/api';
  import { collection, settings, chatSettings } from '../lib/stores.svelte';
  import type { ChatMessage, McpTool, ToolCallResult } from '../lib/types';
  import {
    CHAT_MODELS,
    findChatModel,
    groupedChatModels,
    type ChatModel,
  } from '../lib/chatModels';
  import {
    streamAnthropicTurn,
    getAnthropicConfigured,
    type AnthropicMessage,
    type AnthropicTool,
    type AnthropicContentBlock,
    type AnthropicUsage,
  } from '../lib/anthropic';
  import { errMessage } from '../lib/errors';

  const TRANSCRIPT_KEY = 'satchel-chat-transcript';

  // ---- Model state ----
  let support = $state<{ supported: boolean; reason?: string } | null>(null);
  let engine = $state<EngineHandle | null>(null);
  let loading = $state(false);
  let progress = $state<InitProgress>({ text: '', progress: 0, timeElapsed: 0 });
  let loadError = $state<string | undefined>();
  // Tracks which backend is "live". Set by loadModel(); cleared by unloadModel().
  // For 'webllm' we additionally hold an EngineHandle in `engine`; for
  // 'anthropic' we don't have an engine, just an attestation that the API
  // key is configured.
  let liveBackend = $state<'webllm' | 'anthropic' | null>(null);
  let anthropicConfigured = $state(false);

  // ---- MCP state ----
  let mcp = new McpClient(settings.mcpEndpoint);
  let tools = $state<McpTool[]>([]);
  let mcpStatus = $state<'idle' | 'connecting' | 'connected' | 'error'>('idle');
  let mcpError = $state<string | undefined>();

  // ---- Chat state ----
  let transcript = $state<ChatMessage[]>([]);
  let busy = $state(false);
  let abortFlag = $state(false);
  let stream: HTMLDivElement;
  let useLooseSchema = $state(false);
  let round = $state(0);
  // Token bookkeeping for the context indicator. Filled in from
  // chunk.usage on each successful turn; cleared on clear-chat.
  let lastUsage = $state<{ prompt: number; total: number; window: number }>({
    prompt: 0,
    total: 0,
    window: 0,
  });
  let contextFull = $state(false);

  // Auto-inject the user's chosen collection scope into search_knowledge
  // tool calls. Reads from the global collection store (driven by the
  // TopBar scope chip), so chat results stay consistent with what the
  // user sees on Search and Ask. The LLM is allowed to override: if it
  // explicitly passes collection_name or collection_id, we keep its
  // choice. For "whole vault" (no active scope) we pass through
  // unchanged.
  function applyScopeToToolArgs(
    name: string,
    args: Record<string, unknown>
  ): Record<string, unknown> {
    if (name !== 'search_knowledge') return args;
    const scopeName = collection.activeName;
    if (!scopeName) return args;
    if (typeof args.collection_name === 'string' && args.collection_name) return args;
    if (typeof args.collection_id === 'number') return args;
    return { ...args, collection_name: scopeName };
  }

  // Cumulative Anthropic prompt-cache hit count for the current chat.
  // Incremented after each successful turn from
  // `result.usage.cache_read_input_tokens`. Displayed as a pill so the
  // user can see caching is paying off.
  let cacheReadTokens = $state(0);
  // Estimated cumulative dollar cost for the current chat session.
  // Updated after each successful Anthropic turn from `result.usage`
  // and the active model's published per-token pricing. Reset by
  // clearChat. WebLLM turns contribute nothing.
  let sessionCost = $state(0);

  /** Add a turn's `result.usage` to the running session-cost meter. */
  function accumulateCost(modelId: string, usage: AnthropicUsage) {
    const model = findChatModel(modelId);
    if (!model?.pricing) return;
    const { inputPerMillion: pIn, outputPerMillion: pOut } = model.pricing;
    const turn =
      ((usage.input_tokens ?? 0) * pIn +
        (usage.cache_creation_input_tokens ?? 0) * pIn * 1.25 +
        (usage.cache_read_input_tokens ?? 0) * pIn * 0.1 +
        (usage.output_tokens ?? 0) * pOut) /
      1_000_000;
    sessionCost += turn;
  }

  // ---- Settings modal (rail removed in v1.4.0) ----
  let settingsOpen = $state(false);
  // Optional deep-link target. Set before opening to land on a specific
  // tab (e.g. "cloud" when the user clicks SET API KEY).
  let settingsInitialSection = $state<'local' | 'cloud' | 'mcp' | 'persistence' | undefined>(undefined);

  function openSettings(section?: 'local' | 'cloud' | 'mcp' | 'persistence') {
    settingsInitialSection = section;
    settingsOpen = true;
  }

  $effect(() => {
    checkSupport().then((s) => (support = s));
  });

  $effect(() => {
    if (mcpStatus === 'idle') connectMcp();
  });

  // Poll the Anthropic-config status so the UI can disable LOAD on Claude
  // models when the user hasn't saved an API key yet, and recover when
  // they save one without re-loading the page.
  $effect(() => {
    let cancelled = false;
    const tick = async () => {
      const ok = await getAnthropicConfigured();
      if (!cancelled) anthropicConfigured = ok;
    };
    tick();
    const t = window.setInterval(tick, 5000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  });

  let selectedModel = $derived(findChatModel(settings.chatModel) ?? CHAT_MODELS[0]);
  let selectedBackend = $derived(selectedModel?.backend ?? 'webllm');

  // Restore the transcript from localStorage on mount (one-shot).
  // Only happens when persist_history is on.
  onMount(() => {
    if (!chatSettings.persistHistory) return;
    try {
      const raw = localStorage.getItem(TRANSCRIPT_KEY);
      if (!raw) return;
      const parsed = JSON.parse(raw) as ChatMessage[];
      if (Array.isArray(parsed) && parsed.length) {
        // Drop streaming flags from a previous session — anything with
        // streaming:true was an in-flight turn that never completed.
        transcript = parsed.map((m) => ({ ...m, streaming: false }));
      }
    } catch {
      /* corrupt JSON — ignore */
    }
  });

  // Persist transcript to localStorage on every change (when enabled).
  // Tool call results can be large; cap each one at 8 KB so we don't
  // blow past the 5 MB localStorage budget on a long research session.
  $effect(() => {
    if (!chatSettings.persistHistory) return;
    try {
      const compact = transcript.map((m) => {
        if (!m.toolCalls) return m;
        return {
          ...m,
          toolCalls: m.toolCalls.map((tc) => ({
            ...tc,
            result:
              tc.result && tc.result.length > 8000
                ? tc.result.slice(0, 6000) +
                  '\n…[truncated ' +
                  (tc.result.length - 8000) +
                  ' chars]…\n' +
                  tc.result.slice(-2000)
                : tc.result,
          })),
        };
      });
      localStorage.setItem(TRANSCRIPT_KEY, JSON.stringify(compact));
    } catch {
      /* quota exceeded / private mode — best effort */
    }
  });

  async function loadModel() {
    if (loading) return;
    loadError = undefined;
    const m = selectedModel;
    if (!m) return;

    if (m.backend === 'anthropic') {
      // No engine to spin up — just attest that the API key is saved.
      // The chat path streams through `/api/anthropic/messages` per turn.
      const ok = await getAnthropicConfigured();
      anthropicConfigured = ok;
      if (!ok) {
        loadError =
          "Anthropic API key not configured. Open ⚙ Settings → Anthropic API to add one.";
        return;
      }
      liveBackend = 'anthropic';
      // Fake a synthetic context window for the indicator. Anthropic's
      // models don't expose live token counts via the SSE stream's usage
      // event reliably; the indicator stays at 0% during use.
      lastUsage = { prompt: 0, total: 0, window: 200000 };
      contextFull = false;
      return;
    }

    // WebLLM path (unchanged from v1.4.x).
    if (!support?.supported) return;
    loading = true;
    progress = { text: 'starting...', progress: 0, timeElapsed: 0 };
    try {
      const ctx =
        chatSettings.contextWindowSize !== 'auto'
          ? Number(chatSettings.contextWindowSize)
          : undefined;
      const sliding =
        chatSettings.slidingWindowSize !== 'off'
          ? Number(chatSettings.slidingWindowSize)
          : undefined;
      engine = await createEngine(m.id, (p) => (progress = p), {
        contextWindowSize: ctx,
        slidingWindowSize: sliding,
      });
      liveBackend = 'webllm';
      lastUsage = { prompt: 0, total: 0, window: ctx ?? 4096 };
      contextFull = false;
    } catch (e) {
      loadError = errMessage(e);
      engine = null;
      liveBackend = null;
    } finally {
      loading = false;
    }
  }

  async function unloadModel() {
    if (engine) {
      try {
        await engine.unload();
      } catch {}
      engine = null;
    }
    liveBackend = null;
    progress = { text: '', progress: 0, timeElapsed: 0 };
  }

  async function connectMcp() {
    mcpStatus = 'connecting';
    mcpError = undefined;
    mcp.setEndpoint(settings.mcpEndpoint);
    try {
      await mcp.initialize();
      tools = await mcp.listTools();
      mcpStatus = 'connected';
    } catch (e) {
      mcpStatus = 'error';
      mcpError = errMessage(e);
    }
  }

  function scrollToBottom() {
    setTimeout(() => stream?.scrollTo({ top: stream.scrollHeight, behavior: 'smooth' }));
  }

  async function send(text: string) {
    if (busy || !liveBackend) return;
    busy = true;
    abortFlag = false;
    useLooseSchema = false;
    round = 0;

    transcript = [...transcript, { id: crypto.randomUUID(), role: 'user', content: text }];
    scrollToBottom();

    try {
      if (liveBackend === 'anthropic') {
        await runAnthropicLoop();
      } else {
        await runLoop();
      }
    } catch (e) {
      transcript = [
        ...transcript,
        { id: crypto.randomUUID(), role: 'error', content: errMessage(e) },
      ];
    } finally {
      busy = false;
      round = 0;
    }
  }

  function cancel() {
    abortFlag = true;
    engine?.interrupt();
  }

  function clearChat() {
    if (busy) cancel();
    transcript = [];
    lastUsage = { prompt: 0, total: 0, window: lastUsage.window };
    contextFull = false;
    cacheReadTokens = 0;
    sessionCost = 0;
    try {
      localStorage.removeItem(TRANSCRIPT_KEY);
    } catch {}
  }

  // The main agent loop. Each round:
  //   1. Build messages (system prompt + transcript)
  //   2. Stream a constrained response from the model (XGrammar enforces
  //      the agent schema — output is always valid JSON)
  //   3. Parse the JSON: either a tool_call (dispatch via MCP) or a
  //      `respond_to_user` (terminal — write the answer and stop)
  //   4. Loop until terminal, max-rounds, or abort
  async function runLoop(): Promise<void> {
    if (!engine) return;
    // Smart mode picks a compact ~500-token system prompt designed
    // for small local LLMs (concrete tool one-liners, no style
    // guidance). Off-mode keeps the verbose persistence-oriented
    // prompt for users who want maximum hand-holding.
    const sys = chatSettings.smartMode
      ? compactSystemPrompt({
          tools,
          minToolCalls: chatSettings.minToolCalls,
        })
      : constrainedSystemPrompt({
          tools,
          minToolCalls: chatSettings.minToolCalls,
          weakScoreThreshold: chatSettings.weakScoreThreshold,
        });

    // Stall detection: hash every emitted tool_use. When the same
    // hash fires twice in a row, the loop injects a synthetic
    // tool_result nudging the model to vary its approach or finalize.
    const toolHashHistory: string[] = [];
    let pendingNudge: string | null = null;

    while (round < chatSettings.maxRounds && !abortFlag) {
      round += 1;

      const aId = crypto.randomUUID();
      transcript = [
        ...transcript,
        { id: aId, role: 'assistant', content: '', streaming: true },
      ];
      scrollToBottom();

      const messages = buildMessagesForLLM(sys);
      const schemaObj = useLooseSchema ? buildLooseAgentSchema(tools) : buildAgentSchema(tools);

      let streamed = '';
      let result;
      try {
        result = await engine.stream(
          {
            messages,
            schema: JSON.stringify(schemaObj),
            temperature: chatSettings.temperature,
            max_tokens: chatSettings.maxTokens,
          },
          (delta) => {
            streamed += delta;
            // Live render: pull `thought` out of the partial JSON so the
            // user sees the model "thinking" instead of a placeholder.
            const partial = previewThought(streamed);
            transcript = transcript.map((m) =>
              m.id !== aId ? m : { ...m, reasoning: partial, content: '' }
            );
          }
        );
      } catch (e) {
        const msg = errMessage(e);
        // XGrammar can reject some MCP tool argument schemas even after
        // sanitization. Fall back to the loose schema for this turn and
        // retry once.
        const looksLikeSchemaErr = /grammar|xgrammar|unsupported/i.test(msg);
        // WebLLM throws "Prompt tokens exceed context window size" once
        // the agent transcript outgrows the model's compiled window.
        // Surface a friendly hint with the actionable fixes — bump
        // context_window_size in settings (then UNLOAD + LOAD), enable
        // sliding window, or clear the chat — instead of the raw error.
        const looksLikeCtxErr = /exceed context window|context window size|sliding_window/i.test(msg);
        if (looksLikeSchemaErr && !useLooseSchema) {
          useLooseSchema = true;
          transcript = transcript.filter((m) => m.id !== aId);
          transcript = [
            ...transcript,
            {
              id: crypto.randomUUID(),
              role: 'tool',
              content:
                'XGrammar rejected the strict per-tool schema. Falling back to a loose schema for the rest of this turn — argument shapes won\'t be enforced at the token level, but the prompt still describes them.',
            },
          ];
          continue;
        }
        if (looksLikeCtxErr) {
          contextFull = true;
          transcript = transcript.map((m) =>
            m.id !== aId
              ? m
              : {
                  ...m,
                  role: 'error' as const,
                  streaming: false,
                  content:
                    'Context full — the conversation outgrew the model\'s window.\n\n' +
                    'Most reliable fix: Settings → Context → set sliding_window_size (e.g. 4096 or 8192). ' +
                    'It keeps the most recent N tokens in attention and drops older ones, which works regardless of the model\'s compile-time max. ' +
                    'UNLOAD + LOAD to apply.\n\n' +
                    'Alternatives: (1) bump context_window_size if the model was compiled with a larger window (most small models top out at 4096–8192); ' +
                    '(2) Clear chat to start fresh.\n\n' +
                    'Note: context_window_size and sliding_window_size are mutually exclusive — picking one in Settings clears the other automatically (v2.1+).\n\n' +
                    'Raw: ' + msg,
                },
          );
          return;
        }
        transcript = transcript.map((m) =>
          m.id !== aId
            ? m
            : { ...m, role: 'error' as const, streaming: false, content: msg }
        );
        return;
      }

      // Update the context indicator from the WebLLM usage payload.
      if (result?.usage?.total_tokens) {
        lastUsage = {
          prompt: result.usage.prompt_tokens ?? lastUsage.prompt,
          total: result.usage.total_tokens,
          window: lastUsage.window,
        };
        contextFull = lastUsage.window > 0 && lastUsage.total >= lastUsage.window;
      }

      const parsed = parseConstrainedOutput(result?.content ?? streamed);

      if (parsed.errors.length && parsed.toolCalls.length === 0 && !parsed.answer) {
        transcript = transcript.map((m) =>
          m.id !== aId
            ? m
            : {
                ...m,
                role: 'error' as const,
                streaming: false,
                content:
                  'Model output failed to parse: ' +
                  parsed.errors.join('; ') +
                  '\n\nRaw:\n' +
                  (result?.content ?? streamed).slice(0, 800),
              }
        );
        return;
      }

      // Terminal: respond_to_user
      if (parsed.answer !== null) {
        transcript = transcript.map((m) =>
          m.id !== aId
            ? m
            : {
                ...m,
                streaming: false,
                content: parsed.answer ?? '',
                reasoning: parsed.thought || undefined,
                toolCalls: undefined,
              }
        );
        return;
      }

      // Tool call: render it pending, dispatch via MCP, then loop.
      const call = parsed.toolCalls[0];

      // Smart-mode stall detection. Hash this tool call and compare
      // to the prior round's. If duplicate, OR context is above 75%,
      // we skip the real dispatch and inject a synthetic tool_result
      // that nudges the model toward respond_to_user. This is the
      // biggest single win for small local LLMs that otherwise loop
      // the same search query until the window blows.
      if (chatSettings.smartMode) {
        const hash = hashToolCall(call.name, call.args);
        toolHashHistory.push(hash);
        const decision = detectStallPattern({
          toolHashHistory,
          contextUsedTokens: lastUsage.total,
          contextWindowTokens: lastUsage.window,
        });
        if (decision.kind === 'duplicate') {
          pendingNudge = stallNudgeToolResult();
        } else if (decision.kind === 'context-full') {
          pendingNudge = contextFullNudgeToolResult(decision.usedFraction);
        }
      }

      transcript = transcript.map((m) =>
        m.id !== aId
          ? m
          : {
              ...m,
              streaming: false,
              content: '',
              reasoning: parsed.thought || undefined,
              toolCalls: [{ ...call, pending: true }],
            }
      );
      scrollToBottom();

      // If a nudge fired, skip the real MCP dispatch this round and
      // surface the nudge AS the tool result. The model sees the nudge
      // text next round and is strongly pushed toward respond_to_user.
      if (pendingNudge) {
        const nudge = pendingNudge;
        pendingNudge = null;
        transcript = transcript.map((m) =>
          m.id !== aId || !m.toolCalls
            ? m
            : {
                ...m,
                toolCalls: m.toolCalls.map((c) =>
                  c.id === call.id ? { ...c, pending: false, result: nudge } : c
                ),
              }
        );
        scrollToBottom();
        continue;
      }

      try {
        const out = await mcp.callTool(call.name, applyScopeToToolArgs(call.name, call.args));
        transcript = transcript.map((m) =>
          m.id !== aId || !m.toolCalls
            ? m
            : {
                ...m,
                toolCalls: m.toolCalls.map((c) =>
                  c.id === call.id ? { ...c, pending: false, result: out } : c
                ),
              }
        );
      } catch (e) {
        const errMsg = errMessage(e);
        transcript = transcript.map((m) =>
          m.id !== aId || !m.toolCalls
            ? m
            : {
                ...m,
                toolCalls: m.toolCalls.map((c) =>
                  c.id === call.id ? { ...c, pending: false, error: errMsg } : c
                ),
              }
        );
      }
      scrollToBottom();
      // Loop back: feed the tool result into the next round.
    }

    if (round >= chatSettings.maxRounds) {
      transcript = [
        ...transcript,
        {
          id: crypto.randomUUID(),
          role: 'error',
          content: `Hit max_rounds (${chatSettings.maxRounds}). The model didn't reach a respond_to_user — try a stronger model, rephrase, or bump max_rounds in ⚙ Settings.`,
        },
      ];
    }
  }

  // Pull the "thought" string out of partial JSON during streaming so the
  // user sees something other than a static placeholder. Tolerates an
  // unclosed quote (still streaming).
  function previewThought(streamed: string): string {
    let m = streamed.match(/"thought"\s*:\s*"((?:[^"\\]|\\.)*)"/);
    if (!m) m = streamed.match(/"thought"\s*:\s*"((?:[^"\\]|\\.)*)$/);
    if (!m) return '';
    return m[1].replace(/\\n/g, '\n').replace(/\\"/g, '"');
  }

  // Compose the LLM message history. The transcript already encodes
  // role + content + tool calls + reasoning, but we have to flatten
  // tool calls + their results into messages the model expects.
  function buildMessagesForLLM(
    systemPrompt: string
  ): Array<{ role: 'system' | 'user' | 'assistant' | 'tool'; content: string }> {
    const out: Array<{ role: 'system' | 'user' | 'assistant' | 'tool'; content: string }> = [];
    out.push({ role: 'system', content: systemPrompt });

    for (const m of transcript) {
      if (m.role === 'user') {
        out.push({ role: 'user', content: m.content });
        continue;
      }
      if (m.role === 'assistant') {
        // Re-emit the constrained-mode envelope the model produced. This
        // keeps the in-context examples consistent across rounds.
        if (m.toolCalls && m.toolCalls.length) {
          const tc = m.toolCalls[0];
          out.push({
            role: 'assistant',
            content: JSON.stringify({
              thought: m.reasoning || '',
              tool_call: { name: tc.name, arguments: tc.args },
            }),
          });
          if (!tc.pending) {
            let body = tc.error ? `error: ${tc.error}` : tc.result ?? '';
            // Smart-mode caps tool results so a single verbose
            // search hit cannot dominate the local LLM's context.
            // The truncation marker tells the model how much was
            // dropped and (when applicable) which follow-up call
            // would fetch the rest.
            if (chatSettings.smartMode && body && !tc.error) {
              body = truncateToolResult(body, {
                maxTokens: chatSettings.toolResultMaxTokens,
                recoverHint:
                  tc.name === 'search_knowledge'
                    ? 'get_chunk_context with a chunk_id from the head of this result'
                    : tc.name === 'list_sources'
                      ? 'list_sources with a refined filter_type / q'
                      : undefined,
              });
            }
            out.push({
              role: 'tool',
              content: `<tool_response>\n${body}\n</tool_response>`,
            });
          }
        } else if (m.content) {
          // Final answer — represent as the respond_to_user envelope.
          out.push({
            role: 'assistant',
            content: JSON.stringify({
              thought: m.reasoning || 'Sufficient evidence to answer.',
              tool_call: { name: 'respond_to_user', arguments: { answer: m.content } },
            }),
          });
        }
      }
      // 'tool' (free-form notice) and 'error' messages are UI-only; don't
      // feed them back into the model as turns.
    }
    return out;
  }

  // ───────────────────────────────────────────────────────────────────
  // Anthropic agent loop. Mirrors runLoop() but uses Claude's native
  // tool-use protocol (no constrained-decoding schema needed — Anthropic
  // models are reliably trained on `tools=` + `tool_use` content blocks).
  //
  //   1. Build Anthropic-format messages from transcript
  //   2. Stream a turn through `/api/anthropic/messages`
  //   3. If the response has tool_use blocks: dispatch each via MCP,
  //      append a tool_result content block, loop
  //   4. Otherwise: render the final text answer, exit
  // ───────────────────────────────────────────────────────────────────
  async function runAnthropicLoop(): Promise<void> {
    const m = selectedModel;
    if (!m || m.backend !== 'anthropic') return;

    const systemPrompt = buildAnthropicSystemPrompt();
    const anthropicTools: AnthropicTool[] = tools.map((t) => ({
      name: t.name,
      description: t.description ?? '',
      input_schema: (t.inputSchema as Record<string, unknown>) ?? {
        type: 'object',
        properties: {},
      },
    }));

    while (round < chatSettings.maxRounds && !abortFlag) {
      round += 1;
      const aId = crypto.randomUUID();
      transcript = [
        ...transcript,
        { id: aId, role: 'assistant', content: '', streaming: true },
      ];
      scrollToBottom();

      const messages = buildAnthropicMessages();
      // Extended-thinking surface (thinking + output_config.effort) is
      // only valid on models that opt in. Haiku 4.5 returns 400 from
      // the API if those fields are present. Gate on the model's
      // capability flag so we omit them cleanly. Server-side
      // strip_unsupported_params is the defense-in-depth backstop for
      // anything that still leaks through (e.g. third-party clients
      // hitting the proxy directly).
      const supportsThinking = m.supportsExtendedThinking ?? false;
      let result;
      try {
        result = await streamAnthropicTurn(
          {
            model: m.id,
            messages,
            tools: anthropicTools.length ? anthropicTools : undefined,
            system: systemPrompt,
            max_tokens: chatSettings.anthropicMaxTokens,
            cache: chatSettings.anthropicCaching,
            thinking: supportsThinking ? chatSettings.anthropicThinking : undefined,
            effort: supportsThinking ? chatSettings.anthropicEffort : undefined,
          },
          (delta) => {
            transcript = transcript.map((m2) =>
              m2.id !== aId ? m2 : { ...m2, content: (m2.content ?? '') + delta }
            );
          },
        );
      } catch (e) {
        const msg = errMessage(e);
        transcript = transcript.map((m2) =>
          m2.id !== aId
            ? m2
            : { ...m2, role: 'error' as const, streaming: false, content: msg }
        );
        return;
      }

      if (result.error) {
        transcript = transcript.map((m2) =>
          m2.id !== aId
            ? m2
            : { ...m2, role: 'error' as const, streaming: false, content: result.error! }
        );
        return;
      }

      // Track cumulative cache hits AND dollar cost across the chat
      // session. accumulateCost is a no-op for models without published
      // pricing (e.g. WebLLM models, future models we haven't priced).
      if (result.usage.cache_read_input_tokens) {
        cacheReadTokens += result.usage.cache_read_input_tokens;
      }
      accumulateCost(m.id, result.usage);

      // No tool calls? Final answer; finalize and exit.
      if (result.toolUses.length === 0) {
        transcript = transcript.map((m2) =>
          m2.id !== aId
            ? m2
            : { ...m2, streaming: false, content: result.text || m2.content || '' }
        );
        return;
      }

      // Tool calls: render them on the assistant turn, dispatch, loop.
      const calls: ToolCallResult[] = result.toolUses.map((tu) => ({
        id: tu.id,
        name: tu.name,
        args: tu.input,
        pending: true,
      }));
      transcript = transcript.map((m2) =>
        m2.id !== aId
          ? m2
          : {
              ...m2,
              streaming: false,
              content: result.text,
              toolCalls: calls,
            }
      );
      scrollToBottom();

      for (const tu of result.toolUses) {
        try {
          const out = await mcp.callTool(tu.name, applyScopeToToolArgs(tu.name, tu.input));
          transcript = transcript.map((m2) =>
            m2.id !== aId || !m2.toolCalls
              ? m2
              : {
                  ...m2,
                  toolCalls: m2.toolCalls.map((c) =>
                    c.id === tu.id ? { ...c, pending: false, result: out } : c
                  ),
                }
          );
        } catch (e) {
          const errMsg = errMessage(e);
          transcript = transcript.map((m2) =>
            m2.id !== aId || !m2.toolCalls
              ? m2
              : {
                  ...m2,
                  toolCalls: m2.toolCalls.map((c) =>
                    c.id === tu.id ? { ...c, pending: false, error: errMsg } : c
                  ),
                }
          );
        }
      }
      scrollToBottom();
      // Loop body continues, builds next message list including the
      // tool_result blocks.
    }

    if (round >= chatSettings.maxRounds) {
      transcript = [
        ...transcript,
        {
          id: crypto.randomUUID(),
          role: 'error',
          content: `Hit max_rounds (${chatSettings.maxRounds}). The model didn't reach a final answer.`,
        },
      ];
    }
  }

  function buildAnthropicSystemPrompt(): string {
    // User-editable, persisted in chatSettings.anthropicSystemPrompt.
    // Default lives in stores.svelte.ts (DEFAULT_ANTHROPIC_SYSTEM).
    return chatSettings.anthropicSystemPrompt;
  }

  /** Translate the satchel transcript to Anthropic's `messages` shape.
   *  - User turns → `{role: 'user', content: text}`
   *  - Assistant turns with tool calls → assistant message containing
   *    text + tool_use blocks; followed by a user message with
   *    tool_result blocks (one per call).
   *  - Plain assistant final-answer turns → assistant message with text. */
  function buildAnthropicMessages(): AnthropicMessage[] {
    const out: AnthropicMessage[] = [];
    for (const m of transcript) {
      if (m.streaming) continue; // skip in-flight placeholder
      if (m.role === 'user') {
        out.push({ role: 'user', content: m.content });
        continue;
      }
      if (m.role === 'assistant') {
        const content: AnthropicContentBlock[] = [];
        if (m.content) content.push({ type: 'text', text: m.content });
        if (m.toolCalls?.length) {
          for (const tc of m.toolCalls) {
            content.push({
              type: 'tool_use',
              id: tc.id,
              name: tc.name,
              input: tc.args,
            });
          }
        }
        if (content.length) {
          out.push({ role: 'assistant', content });
        }
        if (m.toolCalls?.length) {
          // tool_result blocks live on a USER message in Anthropic's
          // protocol, immediately following the assistant's tool_use.
          const results: AnthropicContentBlock[] = m.toolCalls
            .filter((tc) => !tc.pending)
            .map((tc) => {
              let body = tc.error ? `Error: ${tc.error}` : tc.result ?? '';
              // Smart-mode caps results on the Anthropic side too.
              // Anthropic's larger context makes this a smaller win,
              // but cost-conscious users on long sessions still benefit
              // (fewer input tokens per turn). Default budget is 8000
              // for Anthropic so the cap is much looser than WebLLM.
              if (chatSettings.smartMode && body && !tc.error) {
                body = truncateToolResult(body, {
                  maxTokens: Math.max(chatSettings.toolResultMaxTokens * 10, 8000),
                  recoverHint:
                    tc.name === 'search_knowledge'
                      ? 'get_chunk_context with a chunk_id from the head of this result'
                      : undefined,
                });
              }
              return {
                // tool_result is a valid Anthropic content-block type;
                // our narrow union doesn't include it. Cast through
                // unknown because TS won't accept a one-step cast.
                type: 'tool_result',
                tool_use_id: tc.id,
                content: body,
                is_error: !!tc.error,
              } as unknown as AnthropicContentBlock;
            });
          if (results.length) out.push({ role: 'user', content: results });
        }
      }
    }
    return out;
  }

  let progressPct = $derived(Math.round((progress.progress || 0) * 100));
  let modelInfo = $derived(selectedModel);
  let canSend = $derived(!!liveBackend && !busy);

  // Context-fill indicator: turns amber > 80%, danger > 95%.
  let ctxPct = $derived.by(() => {
    if (!lastUsage.window || !lastUsage.total) return 0;
    return Math.min(100, Math.round((lastUsage.total / lastUsage.window) * 100));
  });
  let ctxTone: 'teal' | 'amber' | 'danger' = $derived.by(() => {
    if (ctxPct >= 95) return 'danger';
    if (ctxPct >= 80) return 'amber';
    return 'teal';
  });
</script>

<ViewHead num="08" title={`CHAT <span class="slash">/</span> BROWSER LLM + MCP`}
  desc="A small LLM runs entirely in this browser via WebGPU. Tool calls dispatch to the local MCP server. Nothing leaves your machine." />

<!-- ───────── Status strip · always above the chat ─────────
     Compact pills for live state. The full controls (settings, MCP
     endpoint, tools list) live in the gear-button modal so the chat
     window stays as roomy as possible. -->
<div class="strip">
  <div class="strip-left">
    {#if liveBackend}
      <Pill tone="teal"><Dot tone="teal" /><span class="pill-text">{modelInfo?.label ?? 'model'}</span></Pill>
    {:else if loading}
      <Pill tone="amber"><Dot tone="amber" pulse /><span class="pill-text">loading {progressPct}%</span></Pill>
    {:else}
      <Pill tone="neutral"><Dot tone="dim" /><span class="pill-text">no model</span></Pill>
    {/if}
    {#if mcpStatus === 'connected'}
      <Pill tone="teal"><Dot tone="teal" /><span class="pill-text">{tools.length} tool{tools.length === 1 ? '' : 's'}</span></Pill>
    {:else if mcpStatus === 'connecting'}
      <Pill tone="amber"><Dot tone="amber" pulse /><span class="pill-text">connecting</span></Pill>
    {:else if mcpStatus === 'error'}
      <Pill tone="danger"><Dot tone="danger" /><span class="pill-text">mcp error</span></Pill>
    {/if}
    {#if busy}
      <Pill tone="amber"><Dot tone="amber" pulse /><span class="pill-text">round {round}/{chatSettings.maxRounds}</span></Pill>
    {/if}
    {#if liveBackend && lastUsage.total > 0}
      <Pill tone={ctxTone}>
        <Dot tone={ctxTone} />
        <span class="pill-text">ctx {ctxPct}% · {lastUsage.total}t</span>
      </Pill>
    {/if}
    {#if liveBackend === 'anthropic' && cacheReadTokens > 0}
      <Pill tone="teal">
        <Dot tone="teal" />
        <span class="pill-text" title="Tokens served from prompt cache this chat">cache {cacheReadTokens.toLocaleString()}t</span>
      </Pill>
    {/if}
    {#if liveBackend === 'anthropic' && sessionCost > 0}
      <Pill tone="neutral">
        <Dot tone="dim" />
        <span class="pill-text" title="Estimated cost this chat (input + output + cache, at the active model's published rate)">${sessionCost < 0.01 ? sessionCost.toFixed(4) : sessionCost.toFixed(2)}</span>
      </Pill>
    {/if}
  </div>
  <div class="strip-right">
    <button class="icon-btn" type="button" title="Chat settings" aria-label="Chat settings"
      onclick={() => openSettings()}>⚙</button>
    {#if transcript.length > 0}
      <button class="btn btn-secondary btn-sm" type="button" onclick={clearChat}>CLEAR</button>
    {/if}
  </div>
</div>

{#if collection.activeName}
  <p class="scope-readout">
    search_knowledge calls scoped to <strong>{collection.activeName}</strong>; change in the top bar
  </p>
{/if}

{#if contextFull}
  <div class="ctx-banner">
    <strong>Context full.</strong>
    Open <button class="link" type="button" onclick={() => openSettings()}>⚙ Settings → Context</button>
    and pick <code>sliding_window_size</code> (the most reliable fix — keeps the recent N tokens, drops older), then UNLOAD + LOAD, or
    <button class="link" type="button" onclick={clearChat}>clear chat</button>
    to start fresh.
  </div>
{/if}

<!-- ───────── Engine bar · model picker + LOAD/UNLOAD ─────────
     Inline above the chat instead of off in a sidebar. When no engine is
     "live" (WebLLM or Anthropic) the picker + LOAD button are visible.
     Once a model is hot the bar shrinks to a one-line "ready"
     affordance with UNLOAD. -->
<div class="engine-bar" class:hot={!!liveBackend}>
  {#if !liveBackend}
    <select class="select model-select" bind:value={settings.chatModel}
      disabled={loading} onchange={() => settings.setModel(settings.chatModel)}>
      {#each groupedChatModels() as g (g.name)}
        <optgroup label={g.name}>
          {#each g.items as m (m.id)}
            <option value={m.id}>{m.label} · {m.size}</option>
          {/each}
        </optgroup>
      {/each}
    </select>
    {#if selectedBackend === 'anthropic' && !anthropicConfigured}
      <button class="btn btn-secondary btn-sm" type="button"
        onclick={() => openSettings('cloud')}>SET API KEY</button>
    {:else}
      <button class="btn btn-primary btn-sm" type="button" onclick={loadModel}
        disabled={loading || (selectedBackend === 'webllm' && !support?.supported)}>
        {loading ? `LOADING ${progressPct}%` : 'LOAD'}
      </button>
    {/if}
    {#if selectedModel?.notes}
      <p class="model-note">
        {#if selectedBackend === 'anthropic'}
          <span class="badge-anthropic">Anthropic API</span>
        {/if}
        {selectedModel.notes}
      </p>
    {/if}
  {:else}
    <div class="engine-ready">
      <Dot tone="teal" />
      <span class="ready-text">
        {selectedModel?.label ?? settings.chatModel}
        {#if liveBackend === 'anthropic'}<span class="ready-suffix">· Anthropic API</span>{/if}
      </span>
    </div>
    <button class="btn btn-secondary btn-sm" type="button" onclick={unloadModel}>UNLOAD</button>
  {/if}
</div>

{#if support && !support.supported}
  <StatusLine text={'WEBGPU UNAVAILABLE · ' + (support.reason ?? '')} tone="danger" />
{/if}
{#if loading}
  <div class="prog">
    <div class="bar"><div class="fill" style="width:{progressPct}%"></div></div>
    <div class="prog-label">{progress.text} · {progressPct}%</div>
  </div>
{:else if loadError}
  <StatusLine text={'LOAD FAILED · ' + loadError} tone="danger" />
{/if}

<div class="main">
  <div class="stream" bind:this={stream}>
    {#if transcript.length === 0}
      <div class="welcome">
        <Mark size={72} strong />
        <h3>BROWSER LLM · LOCAL MCP</h3>
        <p>A small model runs in this browser via WebGPU. Tool calls go straight to the local MCP server. Nothing leaves your machine.</p>
        {#if !liveBackend && !loading}
          <p class="hint">
            <kbd>1.</kbd> pick a model above + LOAD ·
            <kbd>2.</kbd> wait for MCP to show <strong>connected</strong> ·
            <kbd>3.</kbd> ask
          </p>
        {/if}
        <p class="hint mode">
          <span class="glyph">⚒</span>
          <span>
            Two backends in one picker. <strong>Local · WebLLM</strong> models run
            entirely in your browser via WebGPU; output is locked to a per-tool JSON
            schema by an XGrammar logit mask. <strong>Anthropic API</strong> models
            (Claude Opus / Sonnet / Haiku) stream through this satchel's server-side
            proxy using your saved API key. Tool calls go to the same MCP either way.
          </span>
        </p>
      </div>
    {:else}
      {#each transcript as m (m.id)}
        <MessageBubble message={m} />
      {/each}
    {/if}
  </div>
  <Composer onSend={send} onCancel={cancel} {busy}
    placeholder={liveBackend ? 'message... (enter to send, shift+enter newline)' : 'load a model first...'}
    disabled={!canSend && !busy} />
</div>

{#if chatSettings.showSystemPrompt && tools.length}
  <details class="sys-debug">
    <summary>SYSTEM PROMPT (debug)</summary>
    <pre>{constrainedSystemPrompt({
      tools,
      minToolCalls: chatSettings.minToolCalls,
      weakScoreThreshold: chatSettings.weakScoreThreshold,
    })}</pre>
  </details>
{/if}

<SettingsModal
  open={settingsOpen}
  onClose={() => (settingsOpen = false)}
  engineLoaded={!!liveBackend}
  initialSection={settingsInitialSection}
  activeBackend={selectedBackend}
  mcpEndpoint={settings.mcpEndpoint}
  {mcpStatus}
  {mcpError}
  {tools}
  onMcpEndpointChange={(url) => settings.setMcp(url)}
  onMcpConnect={connectMcp}
/>

<style>
  .scope-readout {
    margin: 6px 0 14px;
    font-size: 10px;
    letter-spacing: 1.5px;
    color: var(--text-dim);
    text-transform: uppercase;
  }
  .scope-readout strong {
    color: var(--amber);
    font-weight: 700;
  }
  .strip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
    padding: 10px 0 14px;
    border-bottom: 1px dashed var(--border);
    margin-bottom: 18px;
  }
  .strip-left, .strip-right {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .pill-text { white-space: nowrap; }

  .icon-btn {
    font-family: inherit;
    font-size: 16px;
    line-height: 1;
    padding: 6px 10px;
    background: var(--surface);
    color: var(--text-dim);
    border: 1px solid var(--border);
    cursor: pointer;
    transition: 120ms ease;
  }
  .icon-btn:hover { color: var(--amber); border-color: var(--amber-line); }

  .ctx-banner {
    border: 1px solid var(--amber-line);
    background: var(--amber-soft);
    color: var(--text);
    padding: 10px 14px;
    margin-bottom: 14px;
    font-size: 12px;
    line-height: 1.6;
  }
  .ctx-banner strong { color: var(--amber); margin-right: 6px; }
  .ctx-banner code {
    color: var(--teal);
    background: var(--teal-soft);
    padding: 1px 6px;
    font-size: 11px;
  }
  .ctx-banner .link {
    color: var(--teal);
    background: transparent;
    border: none;
    padding: 0;
    font-family: inherit;
    font-size: inherit;
    cursor: pointer;
    text-decoration: underline;
  }
  .ctx-banner .link:hover { color: var(--teal-deep); }

  .sys-debug {
    margin-top: 18px;
    border: 1px dashed var(--border-strong);
    background: var(--surface);
  }
  .sys-debug summary {
    cursor: pointer;
    list-style: none;
    padding: 8px 12px;
    font-size: 10px;
    letter-spacing: 2px;
    text-transform: uppercase;
    color: var(--text-dim);
    user-select: none;
  }
  .sys-debug summary::before {
    content: '▸';
    color: var(--text-dim);
    margin-right: 8px;
    display: inline-block;
    transition: transform 120ms ease;
  }
  .sys-debug[open] summary::before { transform: rotate(90deg); }
  .sys-debug pre {
    padding: 12px 14px;
    border-top: 1px dashed var(--border);
    font-size: 11px;
    line-height: 1.5;
    color: var(--text-dim);
    white-space: pre-wrap;
    overflow-x: auto;
    max-height: 360px;
    overflow-y: auto;
  }

  /* ───── Engine bar — model picker + LOAD/UNLOAD inline above chat ───── */
  .engine-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    padding: 10px 14px;
    border: 1px solid var(--border);
    background: var(--surface);
    margin-bottom: 14px;
  }
  .engine-bar.hot { padding: 8px 14px; }
  .model-select {
    flex: 1;
    min-width: 220px;
    width: auto;
  }
  .model-note {
    flex-basis: 100%;
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.55;
    margin: 0;
    padding-top: 4px;
    border-top: 1px dashed var(--border);
  }
  .engine-ready {
    flex: 1;
    min-width: 220px;
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--teal);
    font-size: 11px;
    letter-spacing: 1px;
    font-weight: 700;
  }
  .ready-text { color: var(--text); font-weight: 500; word-break: break-all; }
  .ready-suffix {
    color: var(--text-dim);
    font-weight: 400;
    margin-left: 8px;
    letter-spacing: 1.5px;
    font-size: 9px;
    text-transform: uppercase;
  }
  .badge-anthropic {
    display: inline-block;
    color: var(--amber);
    background: var(--amber-soft);
    border: 1px solid var(--amber-line);
    padding: 1px 7px;
    margin-right: 8px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 1.5px;
    text-transform: uppercase;
  }

  .prog { margin-bottom: 14px; }
  .prog .bar { height: 3px; background: var(--border); overflow: hidden; }
  .prog .fill { height: 100%; background: var(--amber); transition: width 220ms ease; }
  .prog-label {
    font-size: 10px;
    letter-spacing: 1.5px;
    color: var(--text-dim);
    margin-top: 6px;
  }

  .main {
    display: flex;
    flex-direction: column;
    min-height: 60vh;
    min-width: 0;
  }
  .stream {
    flex: 1;
    overflow-y: auto;
    max-height: calc(100vh - 280px);
    padding-right: 6px;
  }
  .welcome {
    text-align: center;
    padding: 48px 18px;
    color: var(--text-dim);
    border: 1px dashed var(--border);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
  }
  .welcome h3 {
    font-size: 14px;
    letter-spacing: 4px;
    text-transform: uppercase;
    color: var(--amber);
    font-weight: 700;
  }
  .welcome p { font-size: 12px; line-height: 1.7; max-width: 460px; }
  .hint { color: var(--text); font-size: 11px; }
  .hint kbd { margin: 0 4px; }
  .hint.mode {
    color: var(--text-dim);
    font-size: 11px;
    border-top: 1px dashed var(--border);
    padding-top: 14px;
    margin-top: 10px;
    display: flex;
    align-items: flex-start;
    gap: 10px;
    text-align: left;
  }
  .hint.mode .glyph {
    color: var(--amber);
    font-weight: 700;
    flex-shrink: 0;
    margin-top: 1px;
  }
  strong { color: var(--text-bright); font-weight: 700; }

  /* ───── Mobile responsive — single-column already; just tighten spacing ───── */
  @media (max-width: 880px) {
    .strip { padding: 6px 0 10px; }
    .engine-bar { padding: 10px; }
    .stream { max-height: calc(100vh - 280px); }
  }
</style>
