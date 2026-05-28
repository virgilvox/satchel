<script lang="ts">
  import { onMount } from 'svelte';
  import Modal from './Modal.svelte';
  import Dot from './Dot.svelte';
  import {
    chatSettings,
    settings,
    type ContextSize,
    type SlidingSize,
    type AnthropicEffort,
    type AnthropicThinkingMode,
  } from '../lib/stores.svelte';
  import { CHAT_MODELS, findChatModel } from '../lib/chatModels';
  import type { McpTool } from '../lib/types';
  import {
    getAnthropicConfigured,
    setAnthropicKey,
    clearAnthropicKey,
    testAnthropicKey,
  } from '../lib/anthropic';
  import {
    listMcpServers,
    upsertMcpServer,
    deleteMcpServer,
    type McpServerSummary,
  } from '../lib/mcpServers';

  type McpStatus = 'idle' | 'connecting' | 'connected' | 'error';
  type Section = 'cloud' | 'local' | 'mcp' | 'persistence';

  interface Props {
    open: boolean;
    onClose: () => void;
    /** True when an engine is currently loaded. Context/sliding-window
     *  changes need an UNLOAD + LOAD round-trip to take effect. We grey
     *  those rows out when the engine is hot. */
    engineLoaded: boolean;
    /** Optional deep-link target. When set, the modal opens on this
     *  section. Otherwise we default to the section that matches the
     *  active chat backend. */
    initialSection?: Section;
    /** The chat's currently-selected backend, used to pick the default
     *  tab when no explicit `initialSection` was passed. */
    activeBackend?: 'webllm' | 'anthropic';
    /** Live MCP wiring passed from the chat — this modal owns the UI for
     *  endpoint editing + connection, but the connection lifecycle stays
     *  with the chat so the running session keeps its tool list. */
    mcpEndpoint: string;
    mcpStatus: McpStatus;
    mcpError?: string;
    tools: McpTool[];
    onMcpEndpointChange: (url: string) => void;
    onMcpConnect: () => void;
  }
  let {
    open,
    onClose,
    engineLoaded,
    initialSection,
    activeBackend,
    mcpEndpoint,
    mcpStatus,
    mcpError,
    tools,
    onMcpEndpointChange,
    onMcpConnect,
  }: Props = $props();

  const ctxOptions: ContextSize[] = ['auto', 4096, 8192, 16384, 32768];
  const slidingOptions: SlidingSize[] = ['off', 1024, 2048, 4096, 8192];

  // Active Anthropic model + capability flags. Anthropic exposes
  // extended thinking (`thinking: {type: "adaptive"}`) and the
  // companion `output_config.effort` field only on Opus and Sonnet
  // tiers. Haiku 4.5 returns 400 if either is in the request body,
  // so the runner already omits them; here we just dim the matching
  // controls and add a one-line note so the user understands why
  // their adjustments are inert when Haiku is the active model.
  let currentChatModel = $derived(findChatModel(settings.chatModel) ?? CHAT_MODELS[0]);
  let thinkingSupported = $derived(
    !!(currentChatModel && currentChatModel.supportsExtendedThinking),
  );

  // Active tab. Recomputed every time the modal opens so deep-links and
  // backend defaults take effect on each open.
  let section = $state<Section>('cloud');
  $effect(() => {
    if (!open) return;
    if (initialSection) section = initialSection;
    else section = activeBackend === 'webllm' ? 'local' : 'cloud';
  });

  // WebLLM rejects when both context_window_size and sliding_window_size
  // are positive. Picking one here forces the other back to its sentinel
  // (auto / off) so the engine load can't fail with the dual-positive
  // error.
  function setCtx(v: string) {
    if (v === 'auto') return chatSettings.setContextWindowSize('auto');
    chatSettings.setContextWindowSize(Number(v) as ContextSize);
    if (chatSettings.slidingWindowSize !== 'off') {
      chatSettings.setSlidingWindowSize('off');
    }
  }
  function setSliding(v: string) {
    if (v === 'off') return chatSettings.setSlidingWindowSize('off');
    chatSettings.setSlidingWindowSize(Number(v) as SlidingSize);
    if (chatSettings.contextWindowSize !== 'auto') {
      chatSettings.setContextWindowSize('auto');
    }
  }

  function fmt(v: number, digits = 2): string {
    return v.toFixed(digits).replace(/\.?0+$/, '');
  }

  // ─── Anthropic API key ───────────────────────────────────────────────
  let anthropicConfigured = $state(false);
  let anthropicKey = $state(''); // form field; never round-tripped from server
  let anthropicSaving = $state(false);
  let anthropicError: string | undefined = $state(undefined);
  // Validation result for the "Test" button. `null` means no test has
  // been run yet this session; `{ok: true}` shows the green chip;
  // `{ok: false, error}` shows the red chip with the message.
  let anthropicTestResult = $state<null | { ok: boolean; error?: string }>(null);
  let anthropicTesting = $state(false);

  async function refreshAnthropic() {
    anthropicConfigured = await getAnthropicConfigured();
  }
  async function testAnthropic() {
    if (anthropicTesting) return;
    anthropicTesting = true;
    anthropicTestResult = null;
    anthropicTestResult = await testAnthropicKey();
    anthropicTesting = false;
  }
  async function saveAnthropic() {
    if (anthropicSaving || !anthropicKey.trim()) return;
    anthropicSaving = true;
    anthropicError = undefined;
    const r = await setAnthropicKey(anthropicKey.trim());
    if (r.error) anthropicError = r.error;
    if (r.ok) anthropicKey = '';
    await refreshAnthropic();
    anthropicSaving = false;
  }
  async function clearAnthropic() {
    if (anthropicSaving) return;
    anthropicSaving = true;
    await clearAnthropicKey();
    anthropicKey = '';
    await refreshAnthropic();
    anthropicSaving = false;
  }

  const EFFORTS: AnthropicEffort[] = ['low', 'medium', 'high', 'xhigh', 'max'];
  const THINKING_MODES: AnthropicThinkingMode[] = ['adaptive', 'disabled'];

  // ─── MCP servers (external) ──────────────────────────────────────────
  let mcpServers = $state<McpServerSummary[]>([]);
  let mcpEditing = $state(false);
  let mcpForm = $state({
    id: '',
    name: '',
    url: '',
    authHeader: 'Authorization',
    authValue: '',
  });
  let mcpServerError: string | undefined = $state(undefined);

  async function refreshMcpServers() {
    try {
      mcpServers = await listMcpServers();
    } catch (e) {
      mcpServerError = (e as Error).message;
    }
  }
  function startAddMcp() {
    mcpForm = { id: '', name: '', url: '', authHeader: 'Authorization', authValue: '' };
    mcpEditing = true;
    mcpServerError = undefined;
  }
  async function saveMcp() {
    mcpServerError = undefined;
    try {
      const headers: Record<string, string> = {};
      if (mcpForm.authHeader.trim() && mcpForm.authValue.trim()) {
        headers[mcpForm.authHeader.trim()] = mcpForm.authValue.trim();
      }
      await upsertMcpServer({
        id: mcpForm.id.trim(),
        name: mcpForm.name.trim() || mcpForm.id.trim(),
        url: mcpForm.url.trim(),
        headers: Object.keys(headers).length ? headers : undefined,
      });
      mcpEditing = false;
      await refreshMcpServers();
    } catch (e) {
      mcpServerError = (e as Error).message;
    }
  }
  async function removeMcp(id: string) {
    if (!confirm(`Remove MCP server "${id}"?`)) return;
    try {
      await deleteMcpServer(id);
      await refreshMcpServers();
    } catch (e) {
      mcpServerError = (e as Error).message;
    }
  }

  // Refresh both panels each time the modal opens. Otherwise stale data
  // sits on screen between sessions.
  $effect(() => {
    if (open) {
      refreshAnthropic();
      refreshMcpServers();
    }
  });
  onMount(() => {
    refreshAnthropic();
    refreshMcpServers();
  });
</script>

<Modal {open} title="CHAT SETTINGS" {onClose}>
  <div class="tabs" role="tablist">
    <button class="tab" class:active={section === 'cloud'} role="tab" type="button"
      aria-selected={section === 'cloud'}
      onclick={() => (section = 'cloud')}>CLOUD · CLAUDE</button>
    <button class="tab" class:active={section === 'local'} role="tab" type="button"
      aria-selected={section === 'local'}
      onclick={() => (section = 'local')}>LOCAL · WEBLLM</button>
    <button class="tab" class:active={section === 'mcp'} role="tab" type="button"
      aria-selected={section === 'mcp'}
      onclick={() => (section = 'mcp')}>MCP</button>
    <button class="tab" class:active={section === 'persistence'} role="tab" type="button"
      aria-selected={section === 'persistence'}
      onclick={() => (section = 'persistence')}>PERSISTENCE</button>
  </div>

  <div class="body">
    {#if section === 'cloud'}
      <!-- ============ Anthropic API key ============ -->
      <div class="section-label">API KEY</div>
      <p class="desc">
        Use Claude (Opus / Sonnet / Haiku) in the Chat tab. The API key is
        stored at <code>&lt;vault&gt;/anthropic.toml</code> with 0600 permissions; it
        never leaves the server side after save. Chat traffic is proxied
        through <code>/api/anthropic/messages</code> so the key stays out of the
        browser.
      </p>
      <div class="row anthropic-status">
        <Dot tone={anthropicConfigured ? 'teal' : 'dim'} />
        <span class="name">{anthropicConfigured ? 'API key saved' : 'No API key configured'}</span>
        {#if anthropicConfigured}
          <button class="btn btn-secondary btn-sm" type="button"
            onclick={clearAnthropic} disabled={anthropicSaving}>CLEAR</button>
        {/if}
      </div>
      <input type="password" class="select" placeholder="sk-ant-…" autocomplete="off"
        bind:value={anthropicKey} />
      <div class="btn-row">
        <button class="btn btn-primary btn-sm" type="button"
          onclick={saveAnthropic}
          disabled={anthropicSaving || !anthropicKey.trim()}>
          {anthropicSaving ? 'SAVING…' : (anthropicConfigured ? 'REPLACE KEY' : 'SAVE KEY')}
        </button>
        {#if anthropicConfigured}
          <button class="btn btn-secondary btn-sm" type="button"
            onclick={testAnthropic}
            disabled={anthropicTesting}>
            {anthropicTesting ? 'TESTING…' : 'TEST'}
          </button>
        {/if}
        <a class="link" href="https://console.anthropic.com/settings/keys" target="_blank" rel="noreferrer">get a key →</a>
      </div>
      {#if anthropicError}<p class="err">{anthropicError}</p>{/if}
      {#if anthropicTestResult}
        {#if anthropicTestResult.ok}
          <p class="ok">✓ Key works · claude-haiku-4-5 reachable</p>
        {:else}
          <p class="err">✗ {anthropicTestResult.error ?? 'unknown error'}</p>
        {/if}
      {/if}

      <!-- ============ Generation (API mode) ============ -->
      <div class="section-label">GENERATION</div>

      {#if !thinkingSupported}
        <p class="model-note">
          <strong>{currentChatModel?.label ?? 'This model'}</strong> does not support extended thinking.
          The <code>effort</code> and <code>thinking</code> knobs below are stored but omitted from the API request for this model.
          Pick Claude Opus 4.7 or Sonnet 4.6 if you want them to take effect.
        </p>
      {/if}

      <div class="row" class:row-inactive={!thinkingSupported}>
        <div class="row-head">
          <span class="name">effort</span>
          <span class="val">{chatSettings.anthropicEffort}</span>
        </div>
        <div class="seg-group">
          {#each EFFORTS as e (e)}
            <button class="seg" class:active={chatSettings.anthropicEffort === e}
              type="button" onclick={() => chatSettings.setAnthropicEffort(e)}>{e}</button>
          {/each}
        </div>
        <p class="desc">
          Controls how much Claude thinks and acts per turn. <code>xhigh</code> is the recommended setting for tool-using research; <code>high</code> is the safe default. <code>max</code> is Opus-tier only and best when correctness matters more than cost. <code>low</code> is for quick lookups.
        </p>
      </div>

      <div class="row" class:row-inactive={!thinkingSupported}>
        <div class="row-head">
          <span class="name">thinking</span>
          <span class="val">{chatSettings.anthropicThinking}</span>
        </div>
        <div class="seg-group">
          {#each THINKING_MODES as t (t)}
            <button class="seg" class:active={chatSettings.anthropicThinking === t}
              type="button" onclick={() => chatSettings.setAnthropicThinking(t)}>{t}</button>
          {/each}
        </div>
        <p class="desc">
          Adaptive thinking lets Claude decide when and how much to reason before answering. Recommended for RAG and tool-using flows. Disable for latency-sensitive lookups.
        </p>
      </div>

      <div class="row">
        <div class="row-head">
          <span class="name">max_tokens</span>
          <span class="val">{chatSettings.anthropicMaxTokens.toLocaleString()}</span>
        </div>
        <input type="range" min="2048" max="64000" step="2048"
          value={chatSettings.anthropicMaxTokens}
          oninput={(e) => chatSettings.setAnthropicMaxTokens(Number((e.target as HTMLInputElement).value))}
        />
        <p class="desc">
          Maximum output tokens per turn. 16k is fine for chat; raise to 32k or 64k for long syntheses or code generation. Streaming is always on, so high values do not risk request timeouts.
        </p>
      </div>

      <div class="row">
        <div class="row-head">
          <span class="name">max_rounds</span>
          <span class="val">{chatSettings.maxRounds}</span>
        </div>
        <input type="range" min="1" max="20" step="1"
          value={chatSettings.maxRounds}
          oninput={(e) => chatSettings.setMaxRounds(Number((e.target as HTMLInputElement).value))}
        />
        <p class="desc">Hard cap on tool-call rounds per user turn.</p>
      </div>

      <label class="check">
        <input type="checkbox"
          checked={chatSettings.anthropicCaching}
          onchange={(e) => chatSettings.setAnthropicCaching((e.target as HTMLInputElement).checked)}
        />
        <span>
          <span class="name">prompt_caching</span>
          <p class="desc">
            Attach <code>cache_control</code> to the system prompt so the tools + system prefix is reused across turns (1-hour TTL). Cache reads are roughly 10% of the standard input price; cache writes carry a one-time premium that pays back after a couple of follow-up turns. Recommended on for any multi-turn conversation.
          </p>
        </span>
      </label>

      <!-- ============ System prompt ============ -->
      <div class="section-label">SYSTEM PROMPT</div>
      <p class="desc">
        Sent on every Anthropic request, cached server-side. Edit to change Claude's behavior across all chats. The default tells Claude to ground answers in the vault, fetch surrounding context for chat-shaped data, cite sources honestly, and avoid the typical AI cliches and emdashes.
      </p>
      <textarea class="prompt-area"
        rows="14"
        spellcheck="false"
        value={chatSettings.anthropicSystemPrompt}
        oninput={(e) => chatSettings.setAnthropicSystemPrompt((e.target as HTMLTextAreaElement).value)}
      ></textarea>
      <div class="btn-row">
        <button class="btn btn-secondary btn-sm" type="button"
          onclick={() => chatSettings.resetAnthropicSystemPrompt()}>RESET TO DEFAULT</button>
      </div>
    {/if}

    {#if section === 'local'}
      <!-- ============ WebLLM-only generation ============ -->
      <div class="section-label">GENERATION</div>

      <div class="row">
        <div class="row-head">
          <span class="name">temperature</span>
          <span class="val">{fmt(chatSettings.temperature, 2)}</span>
        </div>
        <input type="range" min="0" max="2" step="0.05"
          value={chatSettings.temperature}
          oninput={(e) => chatSettings.setTemperature(Number((e.target as HTMLInputElement).value))}
        />
        <p class="desc">Sampling temperature for WebLLM models. Higher is more creative; lower is more deterministic. 0.6 is a good default for tool-using agents. Not used in Anthropic mode (sampling parameters are not supported on Opus 4.7).</p>
      </div>

      <div class="row">
        <div class="row-head">
          <span class="name">max_tokens</span>
          <span class="val">{chatSettings.maxTokens}</span>
        </div>
        <input type="range" min="256" max="4096" step="128"
          value={chatSettings.maxTokens}
          oninput={(e) => chatSettings.setMaxTokens(Number((e.target as HTMLInputElement).value))}
        />
        <p class="desc">Maximum tokens the WebLLM model can emit per turn.</p>
      </div>

      <div class="row">
        <div class="row-head">
          <span class="name">max_rounds</span>
          <span class="val">{chatSettings.maxRounds}</span>
        </div>
        <input type="range" min="1" max="20" step="1"
          value={chatSettings.maxRounds}
          oninput={(e) => chatSettings.setMaxRounds(Number((e.target as HTMLInputElement).value))}
        />
        <p class="desc">Hard cap on tool-call rounds per user turn. Shared with Anthropic mode.</p>
      </div>

      <!-- ============ Agent loop ============ -->
      <div class="section-label">AGENT</div>

      <div class="row">
        <div class="row-head">
          <span class="name">min_tool_calls</span>
          <span class="val">{chatSettings.minToolCalls}</span>
        </div>
        <input type="range" min="0" max="5" step="1"
          value={chatSettings.minToolCalls}
          oninput={(e) => chatSettings.setMinToolCalls(Number((e.target as HTMLInputElement).value))}
        />
        <p class="desc">Persistence backstop. Model can't <code>respond_to_user</code> until it has called real tools at least this many times. 0 disables.</p>
      </div>

      <div class="row">
        <div class="row-head">
          <span class="name">weak_score_threshold</span>
          <span class="val">{fmt(chatSettings.weakScoreThreshold, 3)}</span>
        </div>
        <input type="range" min="0" max="0.5" step="0.01"
          value={chatSettings.weakScoreThreshold}
          oninput={(e) => chatSettings.setWeakScoreThreshold(Number((e.target as HTMLInputElement).value))}
        />
        <p class="desc">RRF scores below this are noise; the system prompt tells the model not to base its final answer on results scoring below this line.</p>
      </div>

      <!-- ============ Context (engine-load) ============ -->
      <div class="section-label">
        CONTEXT
        {#if engineLoaded}<span class="hint">· UNLOAD + RELOAD to apply</span>{/if}
      </div>
      <p class="desc">
        Pick one strategy. The two are mutually exclusive and WebLLM rejects a load that sets both. Setting one resets the other automatically.
      </p>

      <div class="row" class:disabled={engineLoaded}>
        <div class="row-head">
          <span class="name">context_window_size</span>
          <span class="val">{chatSettings.contextWindowSize}</span>
        </div>
        <select class="select"
          disabled={engineLoaded}
          value={String(chatSettings.contextWindowSize)}
          onchange={(e) => setCtx((e.target as HTMLSelectElement).value)}
        >
          {#each ctxOptions as opt (opt)}
            <option value={String(opt)}>{opt}</option>
          {/each}
        </select>
        <p class="desc">Override the model's compiled context window. Going past the model's compile-time max throws an error at LOAD.</p>
      </div>

      <div class="row" class:disabled={engineLoaded}>
        <div class="row-head">
          <span class="name">sliding_window_size</span>
          <span class="val">{chatSettings.slidingWindowSize}</span>
        </div>
        <select class="select"
          disabled={engineLoaded}
          value={String(chatSettings.slidingWindowSize)}
          onchange={(e) => setSliding((e.target as HTMLSelectElement).value)}
        >
          {#each slidingOptions as opt (opt)}
            <option value={String(opt)}>{opt}</option>
          {/each}
        </select>
        <p class="desc">Last-resort fallback when the transcript outgrows context: keep only the last N tokens in attention, drop older. Less ideal for tool-calling (the model loses earlier observations) but prevents hard failures.</p>
      </div>
    {/if}

    {#if section === 'mcp'}
      <!-- ============ MCP endpoint ============ -->
      <div class="section-label">MCP ENDPOINT</div>
      <p class="desc">
        The local satchel MCP. Defaults to this server's <code>/mcp</code> route; change only if you're proxying through another endpoint.
      </p>
      <input
        type="text"
        class="select"
        value={mcpEndpoint}
        onchange={(e) => onMcpEndpointChange((e.target as HTMLInputElement).value)}
      />
      <div class="btn-row">
        <button class="btn btn-secondary btn-sm" type="button" onclick={onMcpConnect}>RECONNECT</button>
        <span class="mcp-status">
          {#if mcpStatus === 'connected'}
            <Dot tone="teal" /> connected · {tools.length} tool{tools.length === 1 ? '' : 's'}
          {:else if mcpStatus === 'connecting'}
            <Dot tone="amber" pulse /> connecting…
          {:else if mcpStatus === 'error'}
            <Dot tone="danger" /> {mcpError ?? 'error'}
          {:else}
            <Dot tone="dim" /> idle
          {/if}
        </span>
      </div>

      {#if tools.length > 0}
        <div class="section-label">TOOLS <span class="count">{tools.length}</span></div>
        <div class="tool-list">
          {#each tools as t (t.name)}
            <div class="tool">
              <div class="tname">{t.name}</div>
              {#if t.description}<div class="tdesc">{t.description}</div>{/if}
            </div>
          {/each}
        </div>
      {/if}

      <!-- ============ External MCP servers ============ -->
      <div class="section-label">MCP SERVERS</div>
      <p class="desc">
        Wire up MCP servers other than satchel's own (GitHub MCP, filesystem MCP, anything that speaks the protocol). Auth headers are stored at
        <code>&lt;vault&gt;/mcp.toml</code> (0600); the browser never sees them. Traffic forwards through <code>/api/mcp/proxy/&lt;id&gt;</code>.
      </p>
      <div class="row builtin-mcp">
        <Dot tone="teal" />
        <span class="name">satchel</span>
        <span class="desc-inline">always-on, your local vault. Cannot be removed.</span>
      </div>
      {#each mcpServers as s (s.id)}
        <div class="row">
          <Dot tone={s.has_auth ? 'amber' : 'dim'} />
          <span class="name">{s.name}</span>
          <span class="desc-inline">{s.url}</span>
          <button class="btn btn-secondary btn-sm" type="button" onclick={() => removeMcp(s.id)}>REMOVE</button>
        </div>
      {/each}
      {#if mcpEditing}
        <div class="form">
          <label class="field">
            <span class="field-label">id (URL-safe slug)</span>
            <input type="text" class="select" placeholder="github" bind:value={mcpForm.id}
              autocomplete="off" spellcheck="false" />
          </label>
          <label class="field">
            <span class="field-label">label</span>
            <input type="text" class="select" placeholder="GitHub MCP" bind:value={mcpForm.name}
              autocomplete="off" />
          </label>
          <label class="field">
            <span class="field-label">URL</span>
            <input type="text" class="select" placeholder="https://example.com/mcp"
              bind:value={mcpForm.url} autocomplete="off" spellcheck="false" />
          </label>
          <div class="auth-row">
            <label class="field grow">
              <span class="field-label">auth header (optional)</span>
              <input type="text" class="select" placeholder="Authorization"
                bind:value={mcpForm.authHeader} autocomplete="off" spellcheck="false" />
            </label>
            <label class="field grow">
              <span class="field-label">value</span>
              <input type="password" class="select" placeholder="Bearer …"
                bind:value={mcpForm.authValue} autocomplete="off" />
            </label>
          </div>
          <div class="btn-row">
            <button class="btn btn-primary btn-sm" type="button" onclick={saveMcp}
              disabled={!mcpForm.id.trim() || !mcpForm.url.trim()}>SAVE</button>
            <button class="btn btn-secondary btn-sm" type="button"
              onclick={() => (mcpEditing = false)}>CANCEL</button>
          </div>
        </div>
      {:else}
        <div class="btn-row">
          <button class="btn btn-secondary btn-sm" type="button" onclick={startAddMcp}>+ ADD MCP SERVER</button>
        </div>
      {/if}
      {#if mcpServerError}<p class="err">{mcpServerError}</p>{/if}
    {/if}

    {#if section === 'persistence'}
      <div class="section-label">PERSISTENCE</div>

      <label class="check">
        <input type="checkbox"
          checked={chatSettings.persistHistory}
          onchange={(e) => chatSettings.setPersistHistory((e.target as HTMLInputElement).checked)}
        />
        <span>
          <span class="name">persist_history</span>
          <p class="desc">Keep the chat transcript in localStorage so a refresh does not nuke it.</p>
        </span>
      </label>

      <label class="check">
        <input type="checkbox"
          checked={chatSettings.showSystemPrompt}
          onchange={(e) => chatSettings.setShowSystemPrompt((e.target as HTMLInputElement).checked)}
        />
        <span>
          <span class="name">show_system_prompt</span>
          <p class="desc">Reveal the system prompt being sent to the model (useful when debugging unexpected behavior).</p>
        </span>
      </label>
    {/if}
  </div>

  {#snippet footer()}
    <button class="btn btn-primary btn-sm" type="button" onclick={onClose}>DONE</button>
  {/snippet}
</Modal>

<style>
  .tabs {
    display: flex;
    gap: 4px;
    padding: 12px 18px 0;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .tab {
    background: transparent;
    border: 1px solid transparent;
    border-bottom: none;
    color: var(--text-dim);
    padding: 8px 14px;
    font-size: 10px;
    letter-spacing: 1.5px;
    cursor: pointer;
    margin-bottom: -1px;
  }
  .tab:hover { color: var(--text); }
  .tab.active {
    color: var(--text-bright);
    background: var(--bg);
    border-color: var(--border);
    border-bottom-color: var(--bg);
  }

  .body {
    overflow-y: auto;
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .section-label {
    font-size: 10px;
    letter-spacing: 2.5px;
    color: var(--text-dim);
    margin: 14px 0 0;
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .section-label::before { content: '■'; color: var(--amber); font-size: 10px; }
  .section-label::after { content: ''; flex: 1; border-bottom: 1px solid var(--border); }
  .section-label .hint {
    font-size: 9px;
    letter-spacing: 1.5px;
    color: var(--amber);
    text-transform: none;
    flex: none;
  }
  .section-label .count {
    color: var(--teal);
    font-weight: 700;
    flex: none;
  }
  .desc code {
    color: var(--teal);
    background: var(--teal-soft);
    padding: 1px 5px;
    font-size: 11px;
  }
  .btn-row {
    display: flex;
    gap: 10px;
    align-items: center;
    flex-wrap: wrap;
    margin-top: 4px;
  }
  .mcp-status {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    color: var(--text-dim);
    letter-spacing: 1px;
  }
  .tool-list { display: flex; flex-direction: column; gap: 6px; }
  .tool {
    background: var(--bg-deep);
    border: 1px solid var(--border);
    padding: 8px 10px;
  }
  .tname { color: var(--teal); font-weight: 700; font-size: 11px; }
  .tdesc { color: var(--text-dim); font-size: 10px; line-height: 1.5; margin-top: 3px; }

  /* Anthropic + MCP-server rows live in the same modal sections; share styling. */
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    background: var(--bg-deep);
    border: 1px solid var(--border);
    padding: 8px 10px;
  }
  .row .name { font-size: 11px; color: var(--text); font-weight: 700; }
  .row .desc-inline {
    color: var(--text-dim);
    font-size: 10px;
    flex: 1;
    min-width: 0;
    word-break: break-all;
  }
  .row.builtin-mcp { border-color: var(--teal); }
  .row.row-inactive {
    opacity: 0.55;
  }
  .row.row-inactive .seg {
    cursor: not-allowed;
  }
  .anthropic-status .name { font-weight: 500; flex: 1; }

  .model-note {
    margin: 0 0 10px;
    padding: 10px 12px;
    background: var(--amber-soft);
    border: 1px solid var(--amber-line);
    color: var(--text);
    font-size: 11px;
    line-height: 1.6;
  }
  .model-note code {
    color: var(--amber);
    background: transparent;
    padding: 0;
    font-weight: 700;
  }
  .model-note strong {
    color: var(--amber);
    font-weight: 700;
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    background: var(--bg-deep);
    border: 1px solid var(--border);
    padding: 12px;
  }
  .auth-row {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
  }
  .auth-row .grow { flex: 1; min-width: 180px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .field-label {
    font-size: 9px;
    letter-spacing: 2px;
    text-transform: uppercase;
    color: var(--text-dim);
  }

  .err { color: var(--danger); font-size: 11px; margin: 0; }
  .ok { color: var(--teal); font-size: 11px; margin: 0; }
  .link {
    color: var(--teal);
    font-size: 10px;
    letter-spacing: 1px;
    text-transform: uppercase;
    text-decoration: none;
    border-bottom: 1px dotted var(--teal-soft);
  }
  .link:hover { border-bottom-style: solid; }

  .row { display: flex; flex-direction: column; gap: 6px; }
  .row.disabled { opacity: 0.5; }
  .row-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 11px;
    letter-spacing: 1.5px;
  }
  .name { color: var(--text-bright); font-weight: 700; }
  .val { color: var(--amber); font-weight: 700; }
  .desc {
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.55;
    margin: 0;
  }

  .check {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    cursor: pointer;
    padding: 6px 0;
  }
  .check input {
    flex-shrink: 0;
    margin-top: 2px;
    accent-color: var(--amber);
    width: 14px;
    height: 14px;
  }
  .check span { display: flex; flex-direction: column; gap: 4px; }

  /* Style the range slider track + thumb to match the brand. */
  input[type='range'] {
    -webkit-appearance: none;
    appearance: none;
    width: 100%;
    height: 4px;
    background: var(--border);
    outline: none;
  }
  input[type='range']::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    background: var(--amber);
    cursor: pointer;
  }
  input[type='range']::-moz-range-thumb {
    width: 14px;
    height: 14px;
    background: var(--amber);
    border: 0;
    cursor: pointer;
  }

  /* Segmented control for effort + thinking toggles. */
  .seg-group {
    display: flex;
    gap: 0;
    border: 1px solid var(--border);
    overflow: hidden;
  }
  .seg {
    flex: 1;
    background: transparent;
    border: 0;
    border-right: 1px solid var(--border);
    color: var(--text-dim);
    padding: 6px 10px;
    font-size: 11px;
    letter-spacing: 1px;
    text-transform: uppercase;
    cursor: pointer;
  }
  .seg:last-child { border-right: 0; }
  .seg:hover { color: var(--text); }
  .seg.active {
    background: var(--amber);
    color: var(--bg-deep);
    font-weight: 700;
  }

  .prompt-area {
    width: 100%;
    background: var(--bg-deep);
    border: 1px solid var(--border);
    color: var(--text);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11.5px;
    line-height: 1.55;
    padding: 10px;
    resize: vertical;
    min-height: 240px;
  }
  .prompt-area:focus {
    outline: 1px solid var(--amber);
    outline-offset: -1px;
  }
</style>
