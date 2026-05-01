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
    MODELS,
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
  } from '../lib/agent';
  import { McpClient } from '../lib/mcp';
  import { settings, chatSettings } from '../lib/stores.svelte';
  import type { ChatMessage, McpTool, ToolCallResult } from '../lib/types';

  const TRANSCRIPT_KEY = 'satchel-chat-transcript';

  // ---- Model state ----
  let support = $state<{ supported: boolean; reason?: string } | null>(null);
  let engine = $state<EngineHandle | null>(null);
  let loading = $state(false);
  let progress = $state<InitProgress>({ text: '', progress: 0, timeElapsed: 0 });
  let loadError = $state<string | undefined>();

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

  // ---- Mobile drawer + settings modal ----
  let railOpen = $state(false);
  let settingsOpen = $state(false);

  $effect(() => {
    checkSupport().then((s) => (support = s));
  });

  $effect(() => {
    if (mcpStatus === 'idle') connectMcp();
  });

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
    if (loading || !support?.supported) return;
    loading = true;
    loadError = undefined;
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
      engine = await createEngine(
        settings.chatModel,
        (p) => (progress = p),
        { contextWindowSize: ctx, slidingWindowSize: sliding }
      );
      // Capture the live context-window size for the in-chat indicator.
      // We use the override if the user picked one, else fall back to the
      // model's compiled default which we don't know precisely without
      // poking WebLLM internals — 4096 is the safe lower bound used by
      // most q4f16_1 builds.
      lastUsage = { prompt: 0, total: 0, window: ctx ?? 4096 };
      contextFull = false;
    } catch (e) {
      loadError = (e as Error).message;
      engine = null;
    } finally {
      loading = false;
    }
  }

  async function unloadModel() {
    if (!engine) return;
    try {
      await engine.unload();
    } catch {}
    engine = null;
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
      mcpError = (e as Error).message;
    }
  }

  function scrollToBottom() {
    setTimeout(() => stream?.scrollTo({ top: stream.scrollHeight, behavior: 'smooth' }));
  }

  async function send(text: string) {
    if (busy || !engine) return;
    busy = true;
    abortFlag = false;
    useLooseSchema = false;
    round = 0;

    transcript = [...transcript, { id: crypto.randomUUID(), role: 'user', content: text }];
    scrollToBottom();
    railOpen = false; // collapse the drawer on mobile when the user sends

    try {
      await runLoop();
    } catch (e) {
      transcript = [
        ...transcript,
        { id: crypto.randomUUID(), role: 'error', content: (e as Error).message },
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
    const sys = constrainedSystemPrompt({
      tools,
      minToolCalls: chatSettings.minToolCalls,
      weakScoreThreshold: chatSettings.weakScoreThreshold,
    });

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
        const msg = (e as Error).message || String(e);
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
                    'Context full — the conversation grew past the model\'s window.\n\nFix one of: (1) open ⚙ Settings → Context, bump context_window_size to 8192, then UNLOAD + LOAD; (2) enable sliding_window_size to keep the most recent N tokens; (3) Clear chat to start fresh.\n\nRaw: ' +
                    msg,
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

      try {
        const out = await mcp.callTool(call.name, call.args);
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
        const errMsg = (e as Error).message;
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
            const body = tc.error ? `error: ${tc.error}` : tc.result ?? '';
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

  let progressPct = $derived(Math.round((progress.progress || 0) * 100));
  let modelInfo = $derived(MODELS.find((m) => m.id === settings.chatModel));
  let canSend = $derived(!!engine && !busy);

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

<!-- Status / actions strip — visible above the chat in both layouts. -->
<div class="strip">
  <div class="strip-left">
    <button class="rail-toggle" type="button" onclick={() => (railOpen = !railOpen)}
      aria-expanded={railOpen} aria-label="Toggle model + MCP panel">
      ☰ MODEL · MCP
    </button>
    {#if engine}
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
    {#if engine && lastUsage.total > 0}
      <Pill tone={ctxTone}>
        <Dot tone={ctxTone} />
        <span class="pill-text">ctx {ctxPct}% · {lastUsage.total}t</span>
      </Pill>
    {/if}
  </div>
  <div class="strip-right">
    <button class="icon-btn" type="button" title="Chat settings" aria-label="Chat settings"
      onclick={() => (settingsOpen = true)}>⚙</button>
    {#if transcript.length > 0}
      <button class="btn btn-secondary btn-sm" type="button" onclick={clearChat}>CLEAR</button>
    {/if}
  </div>
</div>

{#if contextFull}
  <div class="ctx-banner">
    <strong>Context full.</strong>
    Open <button class="link" type="button" onclick={() => (settingsOpen = true)}>⚙ Settings → Context</button>
    and bump <code>context_window_size</code> to 8192 (then UNLOAD + LOAD), or
    <button class="link" type="button" onclick={clearChat}>clear chat</button>
    to start fresh.
  </div>
{/if}

<div class="layout">
  <!-- Rail: collapsed off-canvas on mobile, sticky sidebar on desktop -->
  <aside class="rail" class:open={railOpen}>
    <div class="rail-head mobile-only">
      <span class="rail-title">SETTINGS</span>
      <button class="close-x" type="button" onclick={() => (railOpen = false)} aria-label="Close panel">×</button>
    </div>

    <div class="section-label">MODEL</div>
    <select class="select" bind:value={settings.chatModel} disabled={loading || !!engine}
      onchange={() => settings.setModel(settings.chatModel)}>
      {#each MODELS as m (m.id)}
        <option value={m.id}>{m.label} · {m.size}</option>
      {/each}
    </select>
    {#if modelInfo?.notes}
      <p class="note">{modelInfo.notes}</p>
    {/if}
    <div class="btn-row">
      {#if !engine}
        <button class="btn btn-primary btn-sm" onclick={loadModel}
          disabled={loading || !support?.supported}>LOAD</button>
      {:else}
        <button class="btn btn-secondary btn-sm" onclick={unloadModel}>UNLOAD</button>
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
    {:else if engine}
      <div class="prog-label ok">READY · {settings.chatModel}</div>
    {:else if loadError}
      <StatusLine text={'LOAD FAILED · ' + loadError} tone="danger" />
    {/if}

    <div class="section-label">MCP</div>
    <input type="text" class="input" bind:value={settings.mcpEndpoint}
      onchange={() => settings.setMcp(settings.mcpEndpoint)} />
    <div class="btn-row">
      <button class="btn btn-secondary btn-sm" onclick={connectMcp}>CONNECT</button>
    </div>
    <div class="prog-label">
      {#if mcpStatus === 'connected'}
        <Dot tone="teal" /> connected · {tools.length} tool{tools.length === 1 ? '' : 's'}
      {:else if mcpStatus === 'connecting'}
        <Dot tone="amber" pulse /> connecting...
      {:else if mcpStatus === 'error'}
        <Dot tone="danger" /> {mcpError}
      {:else}
        <Dot tone="dim" /> idle
      {/if}
    </div>

    {#if tools.length > 0}
      <div class="section-label">TOOLS <span class="count">{tools.length}</span></div>
      <div class="tool-list">
        {#each tools as t (t.name)}
          <div class="tool">
            <div class="tname">{t.name}</div>
            <div class="tdesc">{t.description}</div>
          </div>
        {/each}
      </div>
    {/if}
  </aside>

  <!-- Backdrop for mobile drawer -->
  {#if railOpen}
    <button class="backdrop" type="button" aria-label="Close panel" onclick={() => (railOpen = false)}></button>
  {/if}

  <div class="main">
    <div class="stream" bind:this={stream}>
      {#if transcript.length === 0}
        <div class="welcome">
          <Mark size={72} strong />
          <h3>BROWSER LLM · LOCAL MCP</h3>
          <p>A small model runs in this browser via WebGPU. Tool calls go straight to the local MCP server. Nothing leaves your machine.</p>
          {#if !engine && !loading}
            <p class="hint">
              <kbd>1.</kbd> open <strong>MODEL · MCP</strong> ·
              <kbd>2.</kbd> pick a model + LOAD ·
              <kbd>3.</kbd> ask
            </p>
          {/if}
          <p class="hint mode">
            <span class="glyph">⚒</span>
            <span>Output is constrained by an XGrammar logit mask — the model literally cannot emit invalid JSON or hallucinate tool names. Works with every model in the list, not just the Hermes whitelist.</span>
          </p>
        </div>
      {:else}
        {#each transcript as m (m.id)}
          <MessageBubble message={m} />
        {/each}
      {/if}
    </div>
    <Composer onSend={send} onCancel={cancel} {busy}
      placeholder={engine ? 'message... (enter to send, shift+enter newline)' : 'load a model first...'}
      disabled={!canSend && !busy} />
  </div>
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

<SettingsModal open={settingsOpen} onClose={() => (settingsOpen = false)} engineLoaded={!!engine} />

<style>
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

  .rail-toggle {
    font-family: inherit;
    font-size: 10px;
    letter-spacing: 2px;
    font-weight: 700;
    text-transform: uppercase;
    padding: 6px 12px;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    cursor: pointer;
    display: none;
    transition: 120ms ease;
  }
  .rail-toggle:hover { color: var(--amber); border-color: var(--amber-line); }

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

  .layout {
    display: grid;
    grid-template-columns: 280px 1fr;
    gap: 24px;
    align-items: start;
    position: relative;
  }
  .rail {
    border: 1px solid var(--border);
    background: var(--surface);
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    position: sticky;
    top: 80px;
    max-height: calc(100vh - 100px);
    overflow-y: auto;
  }
  .rail .section-label { margin: 12px 0 6px; }
  .rail .section-label:first-of-type { margin-top: 0; }
  .rail-head { display: none; }
  .mobile-only { display: none; }
  .backdrop { display: none; }

  .note { font-size: 11px; color: var(--text-dim); line-height: 1.55; }
  .prog .bar { height: 3px; background: var(--border); overflow: hidden; }
  .prog .fill { height: 100%; background: var(--amber); transition: width 220ms ease; }
  .prog-label, .ok {
    font-size: 10px;
    letter-spacing: 1.5px;
    color: var(--text-dim);
    margin-top: 6px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ok { color: var(--teal); }

  .tool-list { display: flex; flex-direction: column; gap: 6px; }
  .tool {
    background: var(--bg);
    border: 1px solid var(--border);
    padding: 8px 10px;
  }
  .tname { color: var(--teal); font-weight: 700; font-size: 11px; word-break: break-word; }
  .tdesc { color: var(--text-dim); font-size: 10px; line-height: 1.5; }

  .main {
    display: flex;
    flex-direction: column;
    min-height: 60vh;
    min-width: 0; /* let children shrink in the grid */
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

  /* ============================================================
     Mobile-first responsive: single column, rail becomes a drawer.
  ============================================================ */
  @media (max-width: 880px) {
    .strip { padding: 6px 0 10px; }
    .rail-toggle { display: inline-flex; align-items: center; gap: 8px; }
    .layout { grid-template-columns: 1fr; gap: 0; }
    .rail {
      position: fixed;
      top: 0;
      bottom: 0;
      left: -340px;
      width: 320px;
      max-width: 86vw;
      max-height: 100vh;
      z-index: 80;
      transition: left 220ms ease;
      box-shadow: var(--shadow-frame);
    }
    .rail.open { left: 0; }
    .rail-head {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding-bottom: 12px;
      border-bottom: 1px solid var(--border);
      margin-bottom: 10px;
    }
    .rail-title {
      font-size: 11px;
      letter-spacing: 2.5px;
      text-transform: uppercase;
      color: var(--amber);
      font-weight: 700;
    }
    .close-x {
      background: transparent;
      border: none;
      color: var(--text-dim);
      font-size: 22px;
      line-height: 1;
      cursor: pointer;
      padding: 0 6px;
    }
    .close-x:hover { color: var(--text-bright); }
    .mobile-only { display: flex; }
    .backdrop {
      display: block;
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.55);
      z-index: 70;
      border: none;
      cursor: pointer;
    }
    .stream { max-height: calc(100vh - 240px); }
  }
</style>
