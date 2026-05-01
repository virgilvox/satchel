<script lang="ts">
  import ViewHead from '../components/ViewHead.svelte';
  import Composer from '../components/Composer.svelte';
  import MessageBubble from '../components/MessageBubble.svelte';
  import Mark from '../components/Mark.svelte';
  import Pill from '../components/Pill.svelte';
  import Dot from '../components/Dot.svelte';
  import StatusLine from '../components/StatusLine.svelte';
  import {
    MODELS,
    checkSupport,
    createEngine,
    mcpToolsToWebLlmTools,
    transcriptToWebLlmMessages,
    type EngineHandle,
    type InitProgress,
  } from '../lib/webllm';
  import { McpClient } from '../lib/mcp';
  import { settings } from '../lib/stores.svelte';
  import type { ChatMessage, McpTool, ToolCallResult } from '../lib/types';

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
  let abort: AbortController | null = null;
  let stream: HTMLDivElement;

  $effect(() => {
    checkSupport().then((s) => (support = s));
  });

  async function loadModel() {
    if (loading || !support?.supported) return;
    loading = true;
    loadError = undefined;
    progress = { text: 'starting...', progress: 0, timeElapsed: 0 };
    try {
      engine = await createEngine(settings.chatModel, (p) => (progress = p));
    } catch (e) {
      loadError = (e as Error).message;
      engine = null;
    } finally {
      loading = false;
    }
  }

  async function unloadModel() {
    if (!engine) return;
    try { await engine.unload(); } catch {}
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

  $effect(() => {
    // Auto-connect on first mount.
    if (mcpStatus === 'idle') connectMcp();
  });

  function scrollToBottom() {
    setTimeout(() => stream?.scrollTo({ top: stream.scrollHeight, behavior: 'smooth' }));
  }

  // Hard cap on tool-call rounds per user turn so a misbehaving model can't
  // pin the UI in an endless dispatch loop.
  const MAX_TOOL_ROUNDS = 6;

  async function send(text: string) {
    if (busy || !engine) return;
    busy = true;
    abort = new AbortController();

    transcript = [...transcript, { id: crypto.randomUUID(), role: 'user', content: text }];
    scrollToBottom();

    try {
      await runTurn(0);
    } catch (e) {
      transcript = [
        ...transcript,
        { id: crypto.randomUUID(), role: 'error', content: (e as Error).message },
      ];
    } finally {
      busy = false;
      abort = null;
    }
  }

  function cancel() {
    // Signal abort + interrupt the engine; let send()'s finally clear `busy`
    // after runTurn unwinds. Clearing busy here would race a fast follow-up
    // send() against the still-resolving previous turn — two parallel
    // runTurns would both manipulate transcript.
    abort?.abort();
    engine?.interrupt();
  }

  async function runTurn(round: number): Promise<void> {
    if (!engine) return;
    if (round >= MAX_TOOL_ROUNDS) {
      transcript = [
        ...transcript,
        {
          id: crypto.randomUUID(),
          role: 'error',
          content: `tool call loop exceeded ${MAX_TOOL_ROUNDS} rounds; stopping.`,
        },
      ];
      return;
    }
    const aId = crypto.randomUUID();
    transcript = [
      ...transcript,
      { id: aId, role: 'assistant', content: '', streaming: true },
    ];
    scrollToBottom();

    const messages = transcriptToWebLlmMessages(settings.systemPrompt, transcript.slice(0, -1));
    const toolSpecs = tools.length ? mcpToolsToWebLlmTools(tools) : undefined;

    let streamed = '';
    const result = await engine.chat(messages, {
      tools: toolSpecs,
      signal: abort?.signal,
      onDelta: (delta) => {
        streamed += delta;
        // Render reasoning + content live. Tolerate leading whitespace
        // before <think> — DeepSeek-distill etc. emit a newline first.
        const m = /^\s*<think>([\s\S]*?)<\/think>([\s\S]*)$/.exec(streamed);
        const partial = m
          ? { reasoning: m[1].trim(), content: m[2].trimStart() }
          : /^\s*<think>/.test(streamed)
            ? {
                reasoning: streamed.replace(/^\s*<think>/, ''),
                content: '',
              }
            : { reasoning: undefined, content: streamed };
        transcript = transcript.map((m2) =>
          m2.id !== aId ? m2 : { ...m2, ...partial }
        );
      },
    });

    transcript = transcript.map((m) =>
      m.id !== aId
        ? m
        : {
            ...m,
            streaming: false,
            content: result.content,
            reasoning: result.reasoning,
            toolCalls: result.toolCalls,
          }
    );

    // If the model emitted tool calls, dispatch them to MCP and continue.
    if (result.toolCalls && result.toolCalls.length > 0) {
      for (const call of result.toolCalls) {
        try {
          const out = await mcp.callTool(call.name, call.args);
          updateToolCall(aId, call.id, { pending: false, result: out });
        } catch (e) {
          updateToolCall(aId, call.id, { pending: false, error: (e as Error).message });
        }
      }
      scrollToBottom();
      // Loop back: feed tool results to the model and let it continue.
      await runTurn(round + 1);
    }
  }

  function updateToolCall(messageId: string, callId: string, patch: Partial<ToolCallResult>) {
    transcript = transcript.map((m) => {
      if (m.id !== messageId || !m.toolCalls) return m;
      const calls = m.toolCalls.map((c) => (c.id === callId ? { ...c, ...patch } : c));
      return { ...m, toolCalls: calls };
    });
  }

  function clearChat() {
    transcript = [];
  }

  let progressPct = $derived(Math.round((progress.progress || 0) * 100));
</script>

<ViewHead num="08" title={`CHAT <span class="slash">/</span> BROWSER LLM + MCP`}
  desc="A small LLM runs entirely in this browser via WebGPU. Tool calls dispatch to the local MCP server. Nothing leaves your machine." />

<div class="layout">
  <div class="rail">
    <div class="section-label">MODEL</div>
    <select class="select" bind:value={settings.chatModel} disabled={loading || !!engine}
      onchange={() => settings.setModel(settings.chatModel)}>
      {#each MODELS as m (m.id)}
        <option value={m.id}>{m.label} · {m.size}</option>
      {/each}
    </select>
    <div class="model-notes">
      {#each MODELS as m (m.id)}
        {#if m.id === settings.chatModel}
          {#if m.toolCalling}<Pill tone="teal"><Dot tone="teal" />tool calling</Pill>{/if}
          {#if m.reasoning}<Pill tone="amber"><Dot tone="amber" />reasoning</Pill>{/if}
          <p class="note">{m.notes ?? ''}</p>
        {/if}
      {/each}
    </div>
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

    <div class="section-label">SESSION</div>
    <div class="btn-row">
      <button class="btn btn-secondary btn-sm" onclick={clearChat}>CLEAR CHAT</button>
    </div>
  </div>

  <div class="main">
    <div class="stream" bind:this={stream}>
      {#if transcript.length === 0}
        <div class="welcome">
          <Mark size={80} strong />
          <h3>BROWSER LLM · LOCAL MCP</h3>
          <p>Self-hosted chat. Pick a model on the left, hit LOAD — it streams from HuggingFace and lives in this browser's IndexedDB cache. Nothing leaves your machine.</p>
          {#if !engine && !loading}
            <p class="hint"><kbd>1.</kbd> load model · <kbd>2.</kbd> ensure MCP shows <em>connected</em> · <kbd>3.</kbd> ask</p>
          {/if}
        </div>
      {:else}
        {#each transcript as m (m.id)}
          <MessageBubble message={m} />
        {/each}
      {/if}
    </div>
    <Composer onSend={send} onCancel={cancel} {busy}
      placeholder={engine ? 'message... (enter to send, shift+enter newline)' : 'load a model first...'}
      disabled={!engine} />
  </div>
</div>

<style>
  .layout {
    display: grid;
    grid-template-columns: 280px 1fr;
    gap: 24px;
    align-items: start;
  }
  @media (max-width: 880px) {
    .layout { grid-template-columns: 1fr; }
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
  }
  .rail .section-label { margin: 12px 0 6px; }
  .rail .section-label:first-child { margin-top: 0; }
  .model-notes { display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
  .note { font-size: 11px; color: var(--text-dim); line-height: 1.55; }
  .prog .bar {
    height: 3px; background: var(--border); overflow: hidden;
  }
  .prog .fill {
    height: 100%; background: var(--amber); transition: width 220ms ease;
  }
  .prog-label, .ok {
    font-size: 10px; letter-spacing: 1.5px; color: var(--text-dim);
    margin-top: 6px;
    display: flex; align-items: center; gap: 8px;
  }
  .ok { color: var(--teal); }

  .tool-list { display: flex; flex-direction: column; gap: 6px; }
  .tool {
    background: var(--bg);
    border: 1px solid var(--border);
    padding: 8px 10px;
    cursor: default;
  }
  .tname { color: var(--teal); font-weight: 700; font-size: 11px; }
  .tdesc { color: var(--text-dim); font-size: 10px; line-height: 1.5; }

  .main {
    display: flex; flex-direction: column;
    min-height: 60vh;
  }
  .stream {
    flex: 1;
    overflow-y: auto;
    max-height: calc(100vh - 240px);
    padding-right: 6px;
  }
  .welcome {
    text-align: center;
    padding: 60px 24px;
    color: var(--text-dim);
    border: 1px dashed var(--border);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 18px;
  }
  .welcome h3 {
    font-size: 14px; letter-spacing: 4px;
    text-transform: uppercase; color: var(--amber); font-weight: 700;
  }
  .welcome p { font-size: 12px; line-height: 1.7; max-width: 460px; }
  .hint { color: var(--text); font-size: 11px; }
  .hint kbd { margin: 0 4px; }
  em { color: var(--teal); font-style: normal; }
</style>
